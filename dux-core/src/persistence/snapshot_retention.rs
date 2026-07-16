//! Append-only logical unavailability for immutable snapshot history.
//!
//! Absence of a tombstone means the immutable file may still be opened. A
//! matching tombstone means retention committed before any later unlink. This
//! writer only freezes and inserts an exact tombstone. The
//! repository remains responsible for proving latest-two, active-review pin,
//! and total-cap eligibility while holding the database-before-snapshot lock
//! boundary before it calls this layer.

use std::path::Path;
use std::time::SystemTime;

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use super::codec::encode_host_path;
use super::history::{
    HistoryError, HistoryErrorKind, ScanStatus, load_scan_record_within_budget,
    map_query_sql_error, map_write_sql_error, run_bounded_query, system_time_to_unix_ms,
    unix_ms_to_system_time,
};
use super::snapshot::SnapshotReference;

const RECORD_FORMAT_VERSION: i64 = 1;
const MAX_ID_BYTES: i64 = 128;
const MAX_STATUS_BYTES: i64 = 16;
const MAX_PATH_BYTES: i64 = 65_536;

const TOMBSTONE_SELECT_BY_SCAN_ID: &str = "SELECT
    typeof(tombstone.scan_id),
    length(CAST(tombstone.scan_id AS BLOB)), tombstone.scan_id,
    typeof(tombstone.record_format_version),
    tombstone.record_format_version,
    typeof(tombstone.scan_status),
    length(CAST(tombstone.scan_status AS BLOB)), tombstone.scan_status,
    typeof(tombstone.completed_at_unix_ms),
    tombstone.completed_at_unix_ms,
    typeof(tombstone.snapshot_version), tombstone.snapshot_version,
    typeof(tombstone.snapshot_relative_path),
    length(tombstone.snapshot_relative_path),
    tombstone.snapshot_relative_path,
    typeof(tombstone.snapshot_relative_path_encoding),
    tombstone.snapshot_relative_path_encoding,
    typeof(tombstone.snapshot_checksum_sha256),
    length(tombstone.snapshot_checksum_sha256),
    tombstone.snapshot_checksum_sha256,
    typeof(tombstone.committed_at_unix_ms),
    tombstone.committed_at_unix_ms,
    typeof(scan.status), length(CAST(scan.status AS BLOB)), scan.status,
    typeof(scan.completed_at_unix_ms), scan.completed_at_unix_ms,
    typeof(scan.snapshot_version), scan.snapshot_version,
    typeof(scan.snapshot_relative_path), length(scan.snapshot_relative_path),
    scan.snapshot_relative_path,
    typeof(scan.snapshot_relative_path_encoding),
    scan.snapshot_relative_path_encoding,
    typeof(scan.snapshot_checksum_sha256), length(scan.snapshot_checksum_sha256),
    scan.snapshot_checksum_sha256
 FROM snapshot_retention_tombstones AS tombstone
 LEFT JOIN scans AS scan ON scan.scan_id = tombstone.scan_id
 WHERE tombstone.scan_id = ?1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapshotRetentionState {
    Available,
    Tombstoned { committed_at: SystemTime },
}

/// Exact immutable values selected by retention while it still holds the
/// database-before-snapshot exclusion boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PreparedSnapshotRetentionTombstone {
    reference: SnapshotReference,
    scan_status: &'static str,
    completed_at_unix_ms: i64,
    snapshot_relative_path: Vec<u8>,
    snapshot_relative_path_encoding: i64,
    committed_at_unix_ms: i64,
}

impl PreparedSnapshotRetentionTombstone {
    pub(super) fn prepare(
        reference: &SnapshotReference,
        completed_at: SystemTime,
        committed_at: SystemTime,
    ) -> Result<Self, HistoryError> {
        let completed_at_unix_ms =
            system_time_to_unix_ms(completed_at, HistoryErrorKind::InvalidInput)?;
        let committed_at_unix_ms =
            system_time_to_unix_ms(committed_at, HistoryErrorKind::InvalidInput)?;
        if committed_at_unix_ms < completed_at_unix_ms {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let path = encode_host_path(Path::new(reference.file_name().as_str()))
            .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        Ok(Self {
            reference: reference.clone(),
            scan_status: "succeeded",
            completed_at_unix_ms,
            snapshot_relative_path: path.bytes,
            snapshot_relative_path_encoding: path.encoding as i64,
            committed_at_unix_ms,
        })
    }
}

/// Whether this transaction created the immutable row or adopted an exact row
/// created by an earlier idempotent attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapshotRetentionTombstoneInsertOutcome {
    Inserted,
    AlreadyExact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SnapshotRetentionTombstoneState {
    Missing,
    Exact,
    Conflicting,
}

struct RawTombstone {
    scan_id: String,
    record_format_version: i64,
    scan_status: String,
    completed_at_unix_ms: i64,
    snapshot_version: i64,
    snapshot_relative_path: Vec<u8>,
    snapshot_relative_path_encoding: i64,
    snapshot_checksum_sha256: Vec<u8>,
    committed_at_unix_ms: i64,
    parent_scan_status: String,
    parent_completed_at_unix_ms: i64,
    parent_snapshot_version: i64,
    parent_snapshot_relative_path: Vec<u8>,
    parent_snapshot_relative_path_encoding: i64,
    parent_snapshot_checksum_sha256: Vec<u8>,
}

impl RawTombstone {
    fn exactly_matches(&self, expected: &PreparedSnapshotRetentionTombstone) -> bool {
        self.scan_id == expected.reference.scan_id().as_str()
            && self.record_format_version == RECORD_FORMAT_VERSION
            && self.scan_status == expected.scan_status
            && self.completed_at_unix_ms == expected.completed_at_unix_ms
            && self.snapshot_version == i64::from(expected.reference.version())
            && self.snapshot_relative_path == expected.snapshot_relative_path
            && self.snapshot_relative_path_encoding == expected.snapshot_relative_path_encoding
            && self.snapshot_checksum_sha256.as_slice() == expected.reference.digest().bytes()
            && self.committed_at_unix_ms == expected.committed_at_unix_ms
            && self.parent_scan_status == expected.scan_status
            && self.parent_completed_at_unix_ms == expected.completed_at_unix_ms
            && self.parent_snapshot_version == i64::from(expected.reference.version())
            && self.parent_snapshot_relative_path == expected.snapshot_relative_path
            && self.parent_snapshot_relative_path_encoding
                == expected.snapshot_relative_path_encoding
            && self.parent_snapshot_checksum_sha256.as_slice()
                == expected.reference.digest().bytes()
    }
}

/// Insert one append-only tombstone in the caller's current-schema,
/// writer-leased transaction. This function does not select a victim and does
/// not grant unlink authority.
pub(super) fn insert_snapshot_retention_tombstone(
    transaction: &Transaction<'_>,
    expected: &PreparedSnapshotRetentionTombstone,
) -> Result<SnapshotRetentionTombstoneInsertOutcome, HistoryError> {
    run_bounded_query(transaction, || {
        match snapshot_retention_tombstone_state_within_budget(transaction, expected)? {
            SnapshotRetentionTombstoneState::Exact => {
                return Ok(SnapshotRetentionTombstoneInsertOutcome::AlreadyExact);
            }
            SnapshotRetentionTombstoneState::Conflicting => return Err(corrupt()),
            SnapshotRetentionTombstoneState::Missing => {}
        }
        validate_parent_within_budget(transaction, expected)?;
        let changed = transaction
            .execute(
                "INSERT INTO snapshot_retention_tombstones (
                    scan_id, record_format_version, scan_status,
                    completed_at_unix_ms, snapshot_version,
                    snapshot_relative_path, snapshot_relative_path_encoding,
                    snapshot_checksum_sha256, committed_at_unix_ms
                 ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(scan_id) DO NOTHING",
                params![
                    expected.reference.scan_id().as_str(),
                    expected.scan_status,
                    expected.completed_at_unix_ms,
                    i64::from(expected.reference.version()),
                    expected.snapshot_relative_path,
                    expected.snapshot_relative_path_encoding,
                    expected.reference.digest().bytes(),
                    expected.committed_at_unix_ms,
                ],
            )
            .map_err(map_write_sql_error)?;
        match changed {
            1 => Ok(SnapshotRetentionTombstoneInsertOutcome::Inserted),
            0 => match snapshot_retention_tombstone_state_within_budget(transaction, expected)? {
                SnapshotRetentionTombstoneState::Exact => {
                    Ok(SnapshotRetentionTombstoneInsertOutcome::AlreadyExact)
                }
                SnapshotRetentionTombstoneState::Missing => {
                    Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown))
                }
                SnapshotRetentionTombstoneState::Conflicting => Err(corrupt()),
            },
            _ => Err(HistoryError::new(HistoryErrorKind::InternalState)),
        }
    })
}

/// Reconcile only an exact durable tombstone after a commit-adjacent failure.
/// Absence preserves the original failure, distinguishing a rolled-back
/// transaction from an exact post-commit result. A conflicting immutable row
/// is never adopted.
pub(super) fn reconcile_snapshot_retention_tombstone_insert(
    connection: &Connection,
    expected: &PreparedSnapshotRetentionTombstone,
    failure: HistoryError,
) -> Result<(), HistoryError> {
    match run_bounded_query(connection, || {
        snapshot_retention_tombstone_state_within_budget(connection, expected)
    }) {
        Ok(SnapshotRetentionTombstoneState::Exact) => Ok(()),
        Ok(SnapshotRetentionTombstoneState::Missing) => Err(failure),
        Ok(SnapshotRetentionTombstoneState::Conflicting) => Err(corrupt()),
        Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

pub(super) fn load_snapshot_retention_state(
    connection: &Connection,
    reference: &SnapshotReference,
) -> Result<SnapshotRetentionState, HistoryError> {
    run_bounded_query(connection, || {
        let raw = load_raw_tombstone_within_budget(connection, reference.scan_id().as_str())?;
        raw.map(|row| decode_tombstone(row, reference))
            .transpose()
            .map(|tombstone| tombstone.unwrap_or(SnapshotRetentionState::Available))
    })
}

fn snapshot_retention_tombstone_state_within_budget(
    connection: &Connection,
    expected: &PreparedSnapshotRetentionTombstone,
) -> Result<SnapshotRetentionTombstoneState, HistoryError> {
    let Some(raw) =
        load_raw_tombstone_within_budget(connection, expected.reference.scan_id().as_str())?
    else {
        return Ok(SnapshotRetentionTombstoneState::Missing);
    };
    validate_raw_tombstone(&raw, &expected.reference)?;
    validate_parent_within_budget(connection, expected)?;
    Ok(if raw.exactly_matches(expected) {
        SnapshotRetentionTombstoneState::Exact
    } else {
        SnapshotRetentionTombstoneState::Conflicting
    })
}

fn validate_parent_within_budget(
    connection: &Connection,
    expected: &PreparedSnapshotRetentionTombstone,
) -> Result<(), HistoryError> {
    let parent = load_scan_record_within_budget(connection, expected.reference.scan_id())?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    let completed_at = unix_ms_to_system_time(expected.completed_at_unix_ms)?;
    if parent.status() != ScanStatus::Succeeded
        || parent.completed_at() != Some(completed_at)
        || parent.snapshot() != Some(&expected.reference)
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok(())
}

fn load_raw_tombstone_within_budget(
    connection: &Connection,
    scan_id: &str,
) -> Result<Option<RawTombstone>, HistoryError> {
    connection
        .query_row(TOMBSTONE_SELECT_BY_SCAN_ID, [scan_id], raw_tombstone)
        .optional()
        .map_err(map_query_sql_error)
}

fn raw_tombstone(row: &Row<'_>) -> rusqlite::Result<RawTombstone> {
    require_type_length(row, 0, 1, "text", 1, MAX_ID_BYTES)?;
    require_type(row, 3, "integer")?;
    require_type_length(row, 5, 6, "text", 1, MAX_STATUS_BYTES)?;
    require_type(row, 8, "integer")?;
    require_type(row, 10, "integer")?;
    require_type_length(row, 12, 13, "blob", 1, MAX_PATH_BYTES)?;
    require_type(row, 15, "integer")?;
    require_type_length(row, 17, 18, "blob", 32, 32)?;
    require_type(row, 20, "integer")?;
    require_type_length(row, 22, 23, "text", 1, MAX_STATUS_BYTES)?;
    require_type(row, 25, "integer")?;
    require_type(row, 27, "integer")?;
    require_type_length(row, 29, 30, "blob", 1, MAX_PATH_BYTES)?;
    require_type(row, 32, "integer")?;
    require_type_length(row, 34, 35, "blob", 32, 32)?;
    Ok(RawTombstone {
        scan_id: row.get(2)?,
        record_format_version: row.get(4)?,
        scan_status: row.get(7)?,
        completed_at_unix_ms: row.get(9)?,
        snapshot_version: row.get(11)?,
        snapshot_relative_path: row.get(14)?,
        snapshot_relative_path_encoding: row.get(16)?,
        snapshot_checksum_sha256: row.get(19)?,
        committed_at_unix_ms: row.get(21)?,
        parent_scan_status: row.get(24)?,
        parent_completed_at_unix_ms: row.get(26)?,
        parent_snapshot_version: row.get(28)?,
        parent_snapshot_relative_path: row.get(31)?,
        parent_snapshot_relative_path_encoding: row.get(33)?,
        parent_snapshot_checksum_sha256: row.get(36)?,
    })
}

fn decode_tombstone(
    raw: RawTombstone,
    reference: &SnapshotReference,
) -> Result<SnapshotRetentionState, HistoryError> {
    let committed_at = validate_raw_tombstone(&raw, reference)?;
    Ok(SnapshotRetentionState::Tombstoned { committed_at })
}

fn validate_raw_tombstone(
    raw: &RawTombstone,
    reference: &SnapshotReference,
) -> Result<SystemTime, HistoryError> {
    let expected_path =
        encode_host_path(Path::new(reference.file_name().as_str())).map_err(|_| corrupt())?;
    let completed_at = unix_ms_to_system_time(raw.completed_at_unix_ms)?;
    let committed_at = unix_ms_to_system_time(raw.committed_at_unix_ms)?;
    if raw.scan_id != reference.scan_id().as_str()
        || raw.record_format_version != RECORD_FORMAT_VERSION
        || raw.scan_status != "succeeded"
        || raw.parent_scan_status != raw.scan_status
        || raw.parent_completed_at_unix_ms != raw.completed_at_unix_ms
        || raw.parent_snapshot_version != raw.snapshot_version
        || raw.parent_snapshot_relative_path != raw.snapshot_relative_path
        || raw.parent_snapshot_relative_path_encoding != raw.snapshot_relative_path_encoding
        || raw.parent_snapshot_checksum_sha256 != raw.snapshot_checksum_sha256
        || raw.snapshot_version != i64::from(reference.version())
        || raw.snapshot_relative_path != expected_path.bytes
        || raw.snapshot_relative_path_encoding != expected_path.encoding as i64
        || raw.snapshot_checksum_sha256.as_slice() != reference.digest().bytes()
        || committed_at < completed_at
    {
        return Err(corrupt());
    }
    Ok(committed_at)
}

fn require_type(row: &Row<'_>, column: usize, expected: &str) -> rusqlite::Result<()> {
    let stored: String = row.get(column)?;
    if stored == expected {
        Ok(())
    } else {
        Err(rusqlite::Error::InvalidQuery)
    }
}

fn require_type_length(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
    expected: &str,
    minimum: i64,
    maximum: i64,
) -> rusqlite::Result<()> {
    require_type(row, type_column, expected)?;
    let length: i64 = row.get(length_column)?;
    if (minimum..=maximum).contains(&length) {
        Ok(())
    } else {
        Err(rusqlite::Error::InvalidQuery)
    }
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    use rusqlite::TransactionBehavior;
    use tempfile::TempDir;

    use super::*;
    use crate::ScanCoverage;
    use crate::domain::ScanId;
    use crate::persistence::history::{NewScanRecord, ScanCompletionRecord, ScanCounts};
    use crate::persistence::snapshot::SnapshotFileName;
    use crate::persistence::store::StoreCoordinator;

    const BASE_MILLIS: u64 = 1_750_000_000_000;

    fn reference(scan_id: &ScanId, digest_byte: u8) -> SnapshotReference {
        let name = SnapshotFileName::from_scan_id(scan_id.as_str().as_bytes());
        SnapshotReference::from_stored(scan_id, 1, name.as_str(), [digest_byte; 32]).unwrap()
    }

    fn succeeded_parent(
        scan: &str,
    ) -> (
        TempDir,
        Arc<StoreCoordinator>,
        SnapshotReference,
        SystemTime,
    ) {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let scan_id = ScanId::new(scan).unwrap();
        let started_at = UNIX_EPOCH + Duration::from_millis(BASE_MILLIS);
        let completed_at = started_at + Duration::from_secs(2);
        let reference = reference(&scan_id, 7);
        store
            .record_scan_started(
                &NewScanRecord::try_new(scan_id.clone(), temp.path().join("root"), started_at)
                    .unwrap(),
            )
            .unwrap();
        let coverage = ScanCoverage::try_from_terminal(None, Vec::new()).unwrap();
        store
            .record_scan_finished_reconciled(
                &ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
                    scan_id,
                    completed_at,
                    ScanCounts::default(),
                    coverage,
                    reference.clone(),
                )
                .unwrap(),
            )
            .unwrap();
        (temp, store, reference, completed_at)
    }

    #[test]
    fn prepare_rejects_invalid_clock_and_time_order() {
        let scan_id = ScanId::new("scan:tombstone-prepare").unwrap();
        let reference = reference(&scan_id, 1);
        let completed_at = UNIX_EPOCH + Duration::from_millis(BASE_MILLIS);
        assert_eq!(
            PreparedSnapshotRetentionTombstone::prepare(
                &reference,
                completed_at,
                completed_at - Duration::from_millis(1),
            ),
            Err(HistoryError::new(HistoryErrorKind::InvalidInput))
        );
        assert_eq!(
            PreparedSnapshotRetentionTombstone::prepare(
                &reference,
                UNIX_EPOCH - Duration::from_millis(1),
                completed_at,
            ),
            Err(HistoryError::new(HistoryErrorKind::InvalidInput))
        );
    }

    #[test]
    fn insert_is_append_only_exact_and_idempotent() {
        let (_temp, store, reference, completed_at) = succeeded_parent("scan:tombstone-exact");
        let committed_at = completed_at + Duration::from_secs(1);
        let expected =
            PreparedSnapshotRetentionTombstone::prepare(&reference, completed_at, committed_at)
                .unwrap();

        let mut guard = store.lock_current_history_connection().unwrap();
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            insert_snapshot_retention_tombstone(&transaction, &expected).unwrap(),
            SnapshotRetentionTombstoneInsertOutcome::Inserted
        );
        transaction.commit().unwrap();
        assert_eq!(
            load_snapshot_retention_state(&guard.connection, &reference).unwrap(),
            SnapshotRetentionState::Tombstoned { committed_at }
        );

        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            insert_snapshot_retention_tombstone(&transaction, &expected).unwrap(),
            SnapshotRetentionTombstoneInsertOutcome::AlreadyExact
        );
        transaction.commit().unwrap();

        let conflicting = PreparedSnapshotRetentionTombstone::prepare(
            &reference,
            completed_at,
            committed_at + Duration::from_millis(1),
        )
        .unwrap();
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            insert_snapshot_retention_tombstone(&transaction, &conflicting),
            Err(HistoryError::new(HistoryErrorKind::CorruptData))
        );
    }

    #[test]
    fn reconciliation_distinguishes_rollback_exact_commit_and_conflict() {
        let (_temp, store, reference, completed_at) = succeeded_parent("scan:tombstone-reconcile");
        let committed_at = completed_at + Duration::from_secs(1);
        let expected =
            PreparedSnapshotRetentionTombstone::prepare(&reference, completed_at, committed_at)
                .unwrap();
        let failure = HistoryError::new(HistoryErrorKind::DatabaseUnavailable);
        let mut guard = store.lock_current_history_connection().unwrap();

        {
            let transaction = guard
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            assert_eq!(
                insert_snapshot_retention_tombstone(&transaction, &expected).unwrap(),
                SnapshotRetentionTombstoneInsertOutcome::Inserted
            );
        }
        assert_eq!(
            reconcile_snapshot_retention_tombstone_insert(&guard.connection, &expected, failure,),
            Err(failure)
        );

        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        insert_snapshot_retention_tombstone(&transaction, &expected).unwrap();
        transaction.commit().unwrap();
        assert_eq!(
            reconcile_snapshot_retention_tombstone_insert(&guard.connection, &expected, failure,),
            Ok(())
        );

        let conflicting = PreparedSnapshotRetentionTombstone::prepare(
            &reference,
            completed_at,
            committed_at + Duration::from_millis(1),
        )
        .unwrap();
        assert_eq!(
            reconcile_snapshot_retention_tombstone_insert(&guard.connection, &conflicting, failure,),
            Err(HistoryError::new(HistoryErrorKind::CorruptData))
        );
    }

    #[test]
    fn insert_requires_the_exact_succeeded_parent_reference() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let scan_id = ScanId::new("scan:tombstone-parent").unwrap();
        let started_at = UNIX_EPOCH + Duration::from_millis(BASE_MILLIS);
        let completed_at = started_at + Duration::from_secs(2);
        let reference = reference(&scan_id, 3);
        let expected = PreparedSnapshotRetentionTombstone::prepare(
            &reference,
            completed_at,
            completed_at + Duration::from_secs(1),
        )
        .unwrap();

        let mut guard = store.lock_current_history_connection().unwrap();
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            insert_snapshot_retention_tombstone(&transaction, &expected),
            Err(HistoryError::new(HistoryErrorKind::NotFound))
        );
        drop(transaction);
        drop(guard);

        store
            .record_scan_started(
                &NewScanRecord::try_new(scan_id, temp.path().join("root"), started_at).unwrap(),
            )
            .unwrap();
        let mut guard = store.lock_current_history_connection().unwrap();
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            insert_snapshot_retention_tombstone(&transaction, &expected),
            Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
        );
        drop(transaction);
        let count: i64 = guard
            .connection
            .query_row(
                "SELECT count(*) FROM snapshot_retention_tombstones",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);

        let (_other_temp, other_store, other_reference, other_completed_at) =
            succeeded_parent("scan:tombstone-parent-mismatch");
        let mismatch = PreparedSnapshotRetentionTombstone::prepare(
            &other_reference,
            other_completed_at + Duration::from_millis(1),
            other_completed_at + Duration::from_secs(1),
        )
        .unwrap();
        let mut other_guard = other_store.lock_current_history_connection().unwrap();
        let transaction = other_guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            insert_snapshot_retention_tombstone(&transaction, &mismatch),
            Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
        );
    }
}

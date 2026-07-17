//! Durable identity rows for in-progress snapshot temporary files.
//!
//! A row is coordination metadata, never proof that its process or temporary
//! file is live. The snapshot staging layer retains an independent kernel
//! lease as the sole live-writer proof. `scan_status = 'running'` records the
//! state at lease creation, but the foreign key intentionally binds only the
//! stable scan ID: a panic must still let the durable scan guard record
//! `Interrupted` while a residual lease remains inspectable. Normal completion
//! deletes the exact row before its terminal compare-and-set in the same
//! transaction.

use std::time::SystemTime;

use rusqlite::{Connection, Row, Transaction, params};

use crate::domain::ScanId;

use super::history::{
    HistoryError, HistoryErrorKind, ScanStatus, load_scan_record_within_budget,
    map_query_sql_error, map_write_sql_error, run_bounded_snapshot_pin_query,
    system_time_to_unix_ms,
};
use super::process_liveness::{ProcessIdentityError, ProcessInstanceId};
use super::snapshot::SnapshotFileName;

const RECORD_FORMAT_VERSION: i64 = 1;
pub(super) const MAX_SNAPSHOT_TEMP_LEASES: usize = 64;
const MAX_ID_BYTES: i64 = 32;
const MAX_SCAN_ID_BYTES: i64 = 128;
const MAX_STATUS_BYTES: i64 = 16;
const FINAL_NAME_BYTES: i64 = 85;
const MIN_TEMP_NAME_BYTES: i64 = 113;
const MAX_TEMP_NAME_BYTES: i64 = 122;
const MAX_OWNER_BYTES: i64 = 128;
const FINAL_PREFIX: &str = "snapshot-";
const FINAL_DIGEST_BYTES: usize = 64;
const TEMP_PREFIX: &str = ".snapshot-";
const TEMP_SUFFIX: &str = ".tmp";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SnapshotTempLeaseId(String);

impl SnapshotTempLeaseId {
    pub(super) fn random() -> Result<Self, HistoryError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?;
        let mut encoded = String::with_capacity(32);
        push_lower_hex(&mut encoded, &random);
        Ok(Self(encoded))
    }

    pub(super) fn from_stored(value: String) -> Result<Self, HistoryError> {
        if value.len() != 32 || !value.bytes().all(is_lower_hex) {
            return Err(corrupt());
        }
        Ok(Self(value))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SnapshotTempName(String);

impl SnapshotTempName {
    fn parse(
        value: String,
        final_name: &SnapshotFileName,
        owner: &ProcessInstanceId,
        error_kind: HistoryErrorKind,
    ) -> Result<Self, HistoryError> {
        if !value.is_ascii() || !value.starts_with(TEMP_PREFIX) || !value.ends_with(TEMP_SUFFIX) {
            return Err(HistoryError::new(error_kind));
        }
        let without_prefix = value
            .strip_prefix(TEMP_PREFIX)
            .ok_or_else(|| HistoryError::new(error_kind))?;
        let body = without_prefix
            .strip_suffix(TEMP_SUFFIX)
            .ok_or_else(|| HistoryError::new(error_kind))?;
        let mut fields = body.split('.');
        let (Some(digest), Some(pid), Some(random), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(HistoryError::new(error_kind));
        };
        if digest.len() != FINAL_DIGEST_BYTES
            || !digest.bytes().all(is_lower_hex)
            || random.len() != 32
            || !random.bytes().all(is_lower_hex)
            || pid.is_empty()
            || pid.len() > 10
            || !pid.bytes().all(|byte| byte.is_ascii_digit())
            || (pid.len() > 1 && pid.starts_with('0'))
        {
            return Err(HistoryError::new(error_kind));
        }
        let pid = pid
            .parse::<u32>()
            .ok()
            .filter(|pid| *pid > 0)
            .ok_or_else(|| HistoryError::new(error_kind))?;
        let final_digest_start = FINAL_PREFIX.len();
        let final_digest_end = final_digest_start + FINAL_DIGEST_BYTES;
        let final_value = final_name.as_str();
        if final_value.len() != FINAL_NAME_BYTES as usize
            || digest != &final_value[final_digest_start..final_digest_end]
            || pid != owner.pid()
        {
            return Err(HistoryError::new(error_kind));
        }
        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

/// Frozen values written before a recognized snapshot temporary file exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PreparedSnapshotTempLease {
    id: SnapshotTempLeaseId,
    scan_id: ScanId,
    scan_status: &'static str,
    final_name: SnapshotFileName,
    temp_name: SnapshotTempName,
    owner: ProcessInstanceId,
    created_at_unix_ms: i64,
}

impl PreparedSnapshotTempLease {
    pub(super) fn prepare(
        id: SnapshotTempLeaseId,
        scan_id: ScanId,
        final_name: SnapshotFileName,
        temp_name: String,
        owner: ProcessInstanceId,
        created_at: SystemTime,
    ) -> Result<Self, HistoryError> {
        if final_name != SnapshotFileName::from_scan_id(scan_id.as_str().as_bytes()) {
            return Err(invalid());
        }
        let temp_name = SnapshotTempName::parse(
            temp_name,
            &final_name,
            &owner,
            HistoryErrorKind::InvalidInput,
        )?;
        let created_at_unix_ms =
            system_time_to_unix_ms(created_at, HistoryErrorKind::InvalidInput)?;
        Ok(Self {
            id,
            scan_id,
            scan_status: "running",
            final_name,
            temp_name,
            owner,
            created_at_unix_ms,
        })
    }

    #[cfg(test)]
    pub(super) fn id(&self) -> &SnapshotTempLeaseId {
        &self.id
    }

    pub(super) fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    #[cfg(test)]
    pub(super) fn final_name(&self) -> &SnapshotFileName {
        &self.final_name
    }

    #[cfg(test)]
    pub(super) fn temp_name(&self) -> &str {
        self.temp_name.as_str()
    }

    #[cfg(test)]
    pub(super) fn owner(&self) -> &ProcessInstanceId {
        &self.owner
    }

    #[cfg(test)]
    pub(super) const fn created_at_unix_ms(&self) -> i64 {
        self.created_at_unix_ms
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StoredSnapshotTempLease {
    id: SnapshotTempLeaseId,
    record_format_version: i64,
    scan_id: ScanId,
    scan_status: String,
    final_name: SnapshotFileName,
    temp_name: SnapshotTempName,
    owner: ProcessInstanceId,
    created_at_unix_ms: i64,
    parent_status: SnapshotTempLeaseParentStatus,
}

impl StoredSnapshotTempLease {
    fn exactly_matches(&self, expected: &PreparedSnapshotTempLease) -> bool {
        self.id == expected.id
            && self.record_format_version == RECORD_FORMAT_VERSION
            && self.scan_id == expected.scan_id
            && self.scan_status == expected.scan_status
            && self.final_name == expected.final_name
            && self.temp_name == expected.temp_name
            && self.owner == expected.owner
            && self.created_at_unix_ms == expected.created_at_unix_ms
            && matches!(
                self.parent_status,
                SnapshotTempLeaseParentStatus::Running
                    | SnapshotTempLeaseParentStatus::Failed
                    | SnapshotTempLeaseParentStatus::Cancelled
                    | SnapshotTempLeaseParentStatus::Interrupted
            )
    }

    fn conflicts_with(&self, expected: &PreparedSnapshotTempLease) -> bool {
        self.id == expected.id
            || self.scan_id == expected.scan_id
            || self.final_name == expected.final_name
            || self.temp_name == expected.temp_name
    }

    pub(super) fn id(&self) -> &SnapshotTempLeaseId {
        &self.id
    }

    pub(super) fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    #[cfg(test)]
    pub(super) fn final_name(&self) -> &SnapshotFileName {
        &self.final_name
    }

    pub(super) fn temp_name(&self) -> &str {
        self.temp_name.as_str()
    }

    #[cfg(test)]
    pub(super) fn owner(&self) -> &ProcessInstanceId {
        &self.owner
    }

    pub(super) const fn created_at_unix_ms(&self) -> i64 {
        self.created_at_unix_ms
    }

    pub(super) const fn parent_status(&self) -> SnapshotTempLeaseParentStatus {
        self.parent_status
    }

    /// Recover the exact immutable write tuple for guarded residual retry.
    ///
    /// This does not carry parent-state or liveness authority. Callers still
    /// need the current database/snapshot/kernel proofs for any file action.
    pub(super) fn as_prepared(&self) -> PreparedSnapshotTempLease {
        PreparedSnapshotTempLease {
            id: self.id.clone(),
            scan_id: self.scan_id.clone(),
            scan_status: "running",
            final_name: self.final_name.clone(),
            temp_name: self.temp_name.clone(),
            owner: self.owner.clone(),
            created_at_unix_ms: self.created_at_unix_ms,
        }
    }
}

/// Current parent state observed with the immutable lease row.
///
/// A terminal value is expected cleanup debt after panic or crash recovery;
/// it is not corruption and does not prove that the temporary file is stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapshotTempLeaseParentStatus {
    Running,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SnapshotTempLeasePopulation {
    rows: Vec<StoredSnapshotTempLease>,
}

impl SnapshotTempLeasePopulation {
    pub(super) fn rows(&self) -> &[StoredSnapshotTempLease] {
        &self.rows
    }
}

/// Insert one immutable row after completely validating the bounded table.
///
/// Production callers must hold a current-schema `HistoryConnectionGuard` and
/// start this transaction from that guard. This low-level function exists so
/// the later terminal scan transaction can delete the row and terminalize the
/// parent atomically without introducing a second lock or transaction.
pub(super) fn insert_snapshot_temp_lease(
    transaction: &Transaction<'_>,
    expected: &PreparedSnapshotTempLease,
) -> Result<(), HistoryError> {
    run_bounded_snapshot_pin_query(transaction, || {
        let population = inspect_population_within_budget(transaction)?;
        if population.rows.len() >= MAX_SNAPSHOT_TEMP_LEASES {
            return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
        }
        if population
            .rows
            .iter()
            .any(|row| row.conflicts_with(expected))
        {
            return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
        }
        let parent = load_scan_record_within_budget(transaction, expected.scan_id())?
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
        if parent.status() != ScanStatus::Running || parent.snapshot().is_some() {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let changed = transaction
            .execute(
                "INSERT INTO snapshot_temp_leases (
                    lease_id, record_format_version, scan_id, scan_status,
                    final_relative_name, temp_relative_name,
                    owner_process_instance, created_at_unix_ms
                 ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    expected.id.as_str(),
                    expected.scan_id.as_str(),
                    expected.scan_status,
                    expected.final_name.as_str(),
                    expected.temp_name.as_str(),
                    expected.owner.as_str(),
                    expected.created_at_unix_ms,
                ],
            )
            .map_err(map_write_sql_error)?;
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::InternalState));
        }
        Ok(())
    })
}

/// Delete only the complete frozen row. The caller must perform this before a
/// normal terminal scan CAS in the same transaction. A panic path may instead
/// terminalize first and leave this row as explicit, inspectable cleanup debt.
pub(super) fn delete_snapshot_temp_lease(
    transaction: &Transaction<'_>,
    expected: &PreparedSnapshotTempLease,
) -> Result<(), HistoryError> {
    run_bounded_snapshot_pin_query(transaction, || {
        match snapshot_temp_lease_state_within_budget(transaction, expected)? {
            SnapshotTempLeaseState::Missing => {
                return Err(HistoryError::new(HistoryErrorKind::NotFound));
            }
            SnapshotTempLeaseState::Conflicting => return Err(corrupt()),
            SnapshotTempLeaseState::Exact => {}
        }
        let changed = transaction
            .execute(
                "DELETE FROM snapshot_temp_leases
                 WHERE lease_id = ?1
                   AND record_format_version = 1
                   AND scan_id = ?2
                   AND scan_status = ?3
                   AND final_relative_name = ?4
                   AND temp_relative_name = ?5
                   AND owner_process_instance = ?6
                   AND created_at_unix_ms = ?7",
                params![
                    expected.id.as_str(),
                    expected.scan_id.as_str(),
                    expected.scan_status,
                    expected.final_name.as_str(),
                    expected.temp_name.as_str(),
                    expected.owner.as_str(),
                    expected.created_at_unix_ms,
                ],
            )
            .map_err(map_write_sql_error)?;
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        Ok(())
    })
}

pub(super) fn inspect_snapshot_temp_leases(
    connection: &Connection,
) -> Result<SnapshotTempLeasePopulation, HistoryError> {
    run_bounded_snapshot_pin_query(connection, || inspect_population_within_budget(connection))
}

#[cfg(test)]
pub(super) fn load_snapshot_temp_lease(
    connection: &Connection,
    id: &SnapshotTempLeaseId,
) -> Result<Option<StoredSnapshotTempLease>, HistoryError> {
    inspect_snapshot_temp_leases(connection)
        .map(|population| population.rows.into_iter().find(|row| row.id == *id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapshotTempLeaseState {
    Missing,
    Exact,
    Conflicting,
}

pub(super) fn snapshot_temp_lease_state(
    connection: &Connection,
    expected: &PreparedSnapshotTempLease,
) -> Result<SnapshotTempLeaseState, HistoryError> {
    run_bounded_snapshot_pin_query(connection, || {
        snapshot_temp_lease_state_within_budget(connection, expected)
    })
}

/// Reconcile only an exact durable insert after a commit-adjacent failure.
pub(super) fn reconcile_snapshot_temp_lease_insert(
    connection: &Connection,
    expected: &PreparedSnapshotTempLease,
    failure: HistoryError,
) -> Result<(), HistoryError> {
    let state = run_bounded_snapshot_pin_query(connection, || {
        let population = inspect_population_within_budget(connection)?;
        if let Some(exact) = population
            .rows
            .iter()
            .find(|row| row.exactly_matches(expected))
        {
            return Ok(Some(exact.parent_status));
        }
        if population
            .rows
            .iter()
            .any(|row| row.conflicts_with(expected))
        {
            return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
        }
        Ok(None)
    });
    match state {
        Ok(Some(SnapshotTempLeaseParentStatus::Running)) => Ok(()),
        Ok(Some(
            SnapshotTempLeaseParentStatus::Failed
            | SnapshotTempLeaseParentStatus::Cancelled
            | SnapshotTempLeaseParentStatus::Interrupted,
        )) => Err(HistoryError::new(HistoryErrorKind::InvalidTransition)),
        Ok(None) => Err(failure),
        Err(error) if error.kind == HistoryErrorKind::AlreadyExists => Err(error),
        Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

/// Reconcile only complete absence after a commit-adjacent delete failure.
pub(super) fn reconcile_snapshot_temp_lease_delete(
    connection: &Connection,
    expected: &PreparedSnapshotTempLease,
    failure: HistoryError,
) -> Result<(), HistoryError> {
    match snapshot_temp_lease_state(connection, expected) {
        Ok(SnapshotTempLeaseState::Missing) => Ok(()),
        Ok(SnapshotTempLeaseState::Exact) => Err(failure),
        Ok(SnapshotTempLeaseState::Conflicting) => Err(corrupt()),
        Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn snapshot_temp_lease_state_within_budget(
    connection: &Connection,
    expected: &PreparedSnapshotTempLease,
) -> Result<SnapshotTempLeaseState, HistoryError> {
    let population = inspect_population_within_budget(connection)?;
    if population
        .rows
        .iter()
        .any(|row| row.exactly_matches(expected))
    {
        return Ok(SnapshotTempLeaseState::Exact);
    }
    if population
        .rows
        .iter()
        .any(|row| row.conflicts_with(expected))
    {
        return Ok(SnapshotTempLeaseState::Conflicting);
    }
    Ok(SnapshotTempLeaseState::Missing)
}

fn inspect_population_within_budget(
    connection: &Connection,
) -> Result<SnapshotTempLeasePopulation, HistoryError> {
    let mut statement = connection
        .prepare(&format!(
            "{LEASE_SELECT_PREFIX}
             ORDER BY lease.lease_id ASC
             LIMIT {}",
            MAX_SNAPSHOT_TEMP_LEASES + 1
        ))
        .map_err(map_query_sql_error)?;
    let rows = statement
        .query_map([], raw_snapshot_temp_lease)
        .map_err(map_query_sql_error)?;
    let mut decoded = Vec::with_capacity(MAX_SNAPSHOT_TEMP_LEASES);
    for row in rows {
        if decoded.len() >= MAX_SNAPSHOT_TEMP_LEASES {
            return Err(corrupt());
        }
        decoded.push(decode_snapshot_temp_lease(
            row.map_err(map_query_sql_error)?,
        )?);
    }
    Ok(SnapshotTempLeasePopulation { rows: decoded })
}

const LEASE_SELECT_PREFIX: &str =
    "SELECT typeof(lease.lease_id), length(CAST(lease.lease_id AS BLOB)), lease.lease_id,
            typeof(lease.record_format_version), lease.record_format_version,
            typeof(lease.scan_id), length(CAST(lease.scan_id AS BLOB)), lease.scan_id,
            typeof(lease.scan_status), length(CAST(lease.scan_status AS BLOB)), lease.scan_status,
            typeof(lease.final_relative_name),
            length(CAST(lease.final_relative_name AS BLOB)), lease.final_relative_name,
            typeof(lease.temp_relative_name),
            length(CAST(lease.temp_relative_name AS BLOB)), lease.temp_relative_name,
            typeof(lease.owner_process_instance),
            length(CAST(lease.owner_process_instance AS BLOB)), lease.owner_process_instance,
            typeof(lease.created_at_unix_ms), lease.created_at_unix_ms,
            typeof(parent.status), length(CAST(parent.status AS BLOB)), parent.status
     FROM snapshot_temp_leases AS lease
     LEFT JOIN scans AS parent
       ON parent.scan_id = lease.scan_id";

struct RawSnapshotTempLease {
    id: String,
    record_format_version: i64,
    scan_id: String,
    scan_status: String,
    final_name: String,
    temp_name: String,
    owner: String,
    created_at_unix_ms: i64,
    parent_status: String,
}

fn raw_snapshot_temp_lease(row: &Row<'_>) -> rusqlite::Result<RawSnapshotTempLease> {
    validate_text(row, 0, 1, MAX_ID_BYTES, MAX_ID_BYTES)?;
    validate_integer(row, 3)?;
    validate_text(row, 5, 6, 1, MAX_SCAN_ID_BYTES)?;
    validate_text(row, 8, 9, 1, MAX_STATUS_BYTES)?;
    validate_text(row, 11, 12, FINAL_NAME_BYTES, FINAL_NAME_BYTES)?;
    validate_text(row, 14, 15, MIN_TEMP_NAME_BYTES, MAX_TEMP_NAME_BYTES)?;
    validate_text(row, 17, 18, 1, MAX_OWNER_BYTES)?;
    validate_integer(row, 20)?;
    validate_text(row, 22, 23, 1, MAX_STATUS_BYTES)?;
    Ok(RawSnapshotTempLease {
        id: row.get(2)?,
        record_format_version: row.get(4)?,
        scan_id: row.get(7)?,
        scan_status: row.get(10)?,
        final_name: row.get(13)?,
        temp_name: row.get(16)?,
        owner: row.get(19)?,
        created_at_unix_ms: row.get(21)?,
        parent_status: row.get(24)?,
    })
}

fn validate_text(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
    minimum_length: i64,
    maximum_length: i64,
) -> rusqlite::Result<()> {
    let storage_type: String = row.get(type_column)?;
    let length: i64 = row.get(length_column)?;
    if storage_type != "text" || !(minimum_length..=maximum_length).contains(&length) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn validate_integer(row: &Row<'_>, type_column: usize) -> rusqlite::Result<()> {
    let storage_type: String = row.get(type_column)?;
    if storage_type != "integer" {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn decode_snapshot_temp_lease(
    raw: RawSnapshotTempLease,
) -> Result<StoredSnapshotTempLease, HistoryError> {
    let id = SnapshotTempLeaseId::from_stored(raw.id)?;
    let scan_id = ScanId::new(raw.scan_id).map_err(|_| corrupt())?;
    if raw.record_format_version != RECORD_FORMAT_VERSION
        || raw.scan_status != "running"
        || raw.created_at_unix_ms < 0
    {
        return Err(corrupt());
    }
    let parent_status = match raw.parent_status.as_str() {
        "running" => SnapshotTempLeaseParentStatus::Running,
        "failed" => SnapshotTempLeaseParentStatus::Failed,
        "cancelled" => SnapshotTempLeaseParentStatus::Cancelled,
        "interrupted" => SnapshotTempLeaseParentStatus::Interrupted,
        _ => return Err(corrupt()),
    };
    let final_name = SnapshotFileName::parse(&raw.final_name).map_err(|_| corrupt())?;
    if final_name != SnapshotFileName::from_scan_id(scan_id.as_str().as_bytes()) {
        return Err(corrupt());
    }
    let owner = ProcessInstanceId::from_stored(&raw.owner).map_err(map_process_identity_error)?;
    let temp_name = SnapshotTempName::parse(
        raw.temp_name,
        &final_name,
        &owner,
        HistoryErrorKind::CorruptData,
    )?;
    Ok(StoredSnapshotTempLease {
        id,
        record_format_version: raw.record_format_version,
        scan_id,
        scan_status: raw.scan_status,
        final_name,
        temp_name,
        owner,
        created_at_unix_ms: raw.created_at_unix_ms,
        parent_status,
    })
}

fn map_process_identity_error(_: ProcessIdentityError) -> HistoryError {
    corrupt()
}

fn push_lower_hex(output: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

const fn invalid() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidInput)
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use rusqlite::config::DbConfig;
    use rusqlite::{Connection, TransactionBehavior, params};

    use super::*;
    use crate::persistence::migrations::test_migrations;

    fn fresh_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        for migration in test_migrations() {
            connection.execute_batch(migration.sql).unwrap();
        }
        connection
    }

    fn owner(pid: u32) -> ProcessInstanceId {
        ProcessInstanceId::from_stored(&format!(
            "1:l:{pid:x}:1:{}:{}",
            "11".repeat(32),
            "22".repeat(16)
        ))
        .unwrap()
    }

    fn insert_running_scan(connection: &Connection, scan_id: &ScanId, started_at: i64) {
        connection
            .execute(
                "INSERT INTO scans (
                    scan_id, root_path, root_path_encoding, started_at_unix_ms,
                    status, coverage_status
                 ) VALUES (?1, ?2, 1, ?3, 'running', 'unknown')",
                params![
                    scan_id.as_str(),
                    format!("/fixture/{}", scan_id.as_str()).into_bytes(),
                    started_at
                ],
            )
            .unwrap();
    }

    fn prepared(index: u64, pid: u32) -> PreparedSnapshotTempLease {
        let scan_id = ScanId::new(format!("scan:temp:{index}")).unwrap();
        let final_name = SnapshotFileName::from_scan_id(scan_id.as_str().as_bytes());
        let digest = &final_name.as_str()[FINAL_PREFIX.len()..FINAL_PREFIX.len() + 64];
        let temp_name = format!(
            ".snapshot-{digest}.{pid}.{}.tmp",
            format_args!("{index:032x}")
        );
        PreparedSnapshotTempLease::prepare(
            SnapshotTempLeaseId::from_stored(format!("{:032x}", index + 1)).unwrap(),
            scan_id,
            final_name,
            temp_name,
            owner(pid),
            UNIX_EPOCH + Duration::from_millis(index + 1),
        )
        .unwrap()
    }

    fn insert(connection: &mut Connection, lease: &PreparedSnapshotTempLease) {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        insert_snapshot_temp_lease(&transaction, lease).unwrap();
        transaction.commit().unwrap();
    }

    #[test]
    fn strict_prepare_binds_scan_final_temp_pid_and_canonical_ascii() {
        let lease = prepared(7, 42);
        assert_eq!(lease.scan_id().as_str(), "scan:temp:7");
        assert_eq!(lease.owner().pid(), 42);
        assert_eq!(lease.created_at_unix_ms(), 8);

        let wrong_scan = ScanId::new("scan:other".to_owned()).unwrap();
        let error = PreparedSnapshotTempLease::prepare(
            lease.id.clone(),
            wrong_scan,
            lease.final_name.clone(),
            lease.temp_name.as_str().to_owned(),
            lease.owner.clone(),
            UNIX_EPOCH,
        )
        .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::InvalidInput);

        for malformed in [
            lease.temp_name.as_str().replace(".42.", ".042."),
            lease.temp_name.as_str().replace(".42.", ".43."),
            lease.temp_name.as_str().replace('a', "A"),
            format!("{}/child", lease.temp_name.as_str()),
        ] {
            let error = PreparedSnapshotTempLease::prepare(
                lease.id.clone(),
                lease.scan_id.clone(),
                lease.final_name.clone(),
                malformed,
                lease.owner.clone(),
                UNIX_EPOCH,
            )
            .unwrap_err();
            assert_eq!(error.kind, HistoryErrorKind::InvalidInput);
        }
    }

    #[test]
    fn insert_load_delete_and_exact_reconciliation_are_typed() {
        let mut connection = fresh_connection();
        let lease = prepared(1, 42);
        insert_running_scan(&connection, lease.scan_id(), 0);

        let failure = HistoryError::new(HistoryErrorKind::DatabaseUnavailable);
        assert_eq!(
            snapshot_temp_lease_state(&connection, &lease).unwrap(),
            SnapshotTempLeaseState::Missing
        );
        assert_eq!(
            reconcile_snapshot_temp_lease_insert(&connection, &lease, failure),
            Err(failure)
        );
        insert(&mut connection, &lease);
        let loaded = load_snapshot_temp_lease(&connection, lease.id())
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id(), lease.id());
        assert_eq!(loaded.scan_id(), lease.scan_id());
        assert_eq!(loaded.final_name(), lease.final_name());
        assert_eq!(loaded.temp_name(), lease.temp_name());
        assert_eq!(loaded.owner(), lease.owner());
        assert_eq!(loaded.created_at_unix_ms(), lease.created_at_unix_ms());
        assert_eq!(loaded.as_prepared(), lease);
        assert_eq!(
            reconcile_snapshot_temp_lease_insert(&connection, &lease, failure),
            Ok(())
        );

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        delete_snapshot_temp_lease(&transaction, &lease).unwrap();
        transaction.commit().unwrap();
        assert_eq!(
            reconcile_snapshot_temp_lease_delete(&connection, &lease, failure),
            Ok(())
        );
    }

    #[test]
    fn conflicting_identity_is_never_adopted_during_reconciliation() {
        let mut connection = fresh_connection();
        let lease = prepared(2, 42);
        insert_running_scan(&connection, lease.scan_id(), 0);
        insert(&mut connection, &lease);

        let mut conflicting = lease.clone();
        conflicting.id = SnapshotTempLeaseId::from_stored("f".repeat(32)).unwrap();
        let failure = HistoryError::new(HistoryErrorKind::DatabaseUnavailable);
        assert_eq!(
            snapshot_temp_lease_state(&connection, &conflicting).unwrap(),
            SnapshotTempLeaseState::Conflicting
        );
        assert_eq!(
            reconcile_snapshot_temp_lease_insert(&connection, &conflicting, failure),
            Err(HistoryError::new(HistoryErrorKind::AlreadyExists))
        );
        assert_eq!(
            reconcile_snapshot_temp_lease_delete(&connection, &conflicting, failure),
            Err(HistoryError::new(HistoryErrorKind::CorruptData))
        );
    }

    #[test]
    fn terminal_scan_remains_available_for_panic_debt_and_normal_delete_is_atomic() {
        let mut connection = fresh_connection();
        let lease = prepared(3, 42);
        insert_running_scan(&connection, lease.scan_id(), 0);
        insert(&mut connection, &lease);

        // A panic guard may terminalize while the immutable row remains.
        assert_eq!(
            connection
                .execute(
                    "UPDATE scans SET status = 'interrupted', completed_at_unix_ms = 10
                     WHERE scan_id = ?1",
                    [lease.scan_id().as_str()],
                )
                .unwrap(),
            1
        );
        let residual = load_snapshot_temp_lease(&connection, lease.id())
            .unwrap()
            .unwrap();
        assert_eq!(
            residual.parent_status(),
            SnapshotTempLeaseParentStatus::Interrupted
        );
        let terminal_transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        delete_snapshot_temp_lease(&terminal_transaction, &lease).unwrap();
        terminal_transaction.commit().unwrap();

        // Normal completion removes the exact row before its CAS in one
        // transaction. A rollback preserves both retry inputs.
        let lease = prepared(4, 42);
        insert_running_scan(&connection, lease.scan_id(), 20);
        insert(&mut connection, &lease);
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        delete_snapshot_temp_lease(&transaction, &lease).unwrap();
        assert_eq!(
            transaction
                .execute(
                    "UPDATE scans SET status = 'failed', completed_at_unix_ms = 30
                     WHERE scan_id = ?1 AND status = 'running'",
                    [lease.scan_id().as_str()],
                )
                .unwrap(),
            1
        );
        transaction.rollback().unwrap();
        assert_eq!(
            snapshot_temp_lease_state(&connection, &lease).unwrap(),
            SnapshotTempLeaseState::Exact
        );

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        delete_snapshot_temp_lease(&transaction, &lease).unwrap();
        assert_eq!(
            transaction
                .execute(
                    "UPDATE scans SET status = 'failed', completed_at_unix_ms = 30
                     WHERE scan_id = ?1 AND status = 'running'",
                    [lease.scan_id().as_str()],
                )
                .unwrap(),
            1
        );
        transaction.commit().unwrap();
        assert_eq!(
            snapshot_temp_lease_state(&connection, &lease).unwrap(),
            SnapshotTempLeaseState::Missing
        );
    }

    #[test]
    fn insert_requires_current_running_parent_but_missing_file_metadata_is_valid() {
        let mut connection = fresh_connection();
        let terminal = prepared(5, 42);
        insert_running_scan(&connection, terminal.scan_id(), 0);
        connection
            .execute(
                "UPDATE scans SET status = 'failed', completed_at_unix_ms = 10
                 WHERE scan_id = ?1",
                [terminal.scan_id().as_str()],
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    "INSERT INTO snapshot_temp_leases VALUES
                     (?1, 1, ?2, 'running', ?3, ?4, ?5, ?6)",
                    params![
                        terminal.id.as_str(),
                        terminal.scan_id.as_str(),
                        terminal.final_name.as_str(),
                        terminal.temp_name.as_str(),
                        terminal.owner.as_str(),
                        terminal.created_at_unix_ms,
                    ],
                )
                .is_err()
        );
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            insert_snapshot_temp_lease(&transaction, &terminal)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        drop(transaction);

        // This layer never consults the filesystem. A missing-file row is
        // classifiable residual metadata, not liveness or removal authority.
        let missing_file = prepared(6, 42);
        insert_running_scan(&connection, missing_file.scan_id(), 20);
        insert(&mut connection, &missing_file);
        assert_eq!(
            snapshot_temp_lease_state(&connection, &missing_file).unwrap(),
            SnapshotTempLeaseState::Exact
        );
    }

    #[test]
    fn terminal_residual_is_deletable_but_never_adopted_as_an_insert() {
        for (index, status, expected_status) in [
            (80, "failed", SnapshotTempLeaseParentStatus::Failed),
            (81, "cancelled", SnapshotTempLeaseParentStatus::Cancelled),
            (
                82,
                "interrupted",
                SnapshotTempLeaseParentStatus::Interrupted,
            ),
        ] {
            let mut connection = fresh_connection();
            let lease = prepared(index, 42);
            insert_running_scan(&connection, lease.scan_id(), 0);
            insert(&mut connection, &lease);
            connection
                .execute(
                    "UPDATE scans SET status = ?2, completed_at_unix_ms = 100
                     WHERE scan_id = ?1",
                    params![lease.scan_id().as_str(), status],
                )
                .unwrap();

            let stored = load_snapshot_temp_lease(&connection, lease.id())
                .unwrap()
                .unwrap();
            assert_eq!(stored.parent_status(), expected_status);
            let failure = HistoryError::new(HistoryErrorKind::DatabaseUnavailable);
            assert_eq!(
                reconcile_snapshot_temp_lease_insert(&connection, &lease, failure),
                Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
            );

            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            delete_snapshot_temp_lease(&transaction, &lease).unwrap();
            transaction.commit().unwrap();
        }
    }

    #[test]
    fn succeeded_transition_requires_exact_lease_deletion_in_same_transaction() {
        let mut connection = fresh_connection();
        let lease = prepared(7, 42);
        insert_running_scan(&connection, lease.scan_id(), 0);
        insert(&mut connection, &lease);

        assert!(
            connection
                .execute(
                    "UPDATE scans SET status = 'succeeded', completed_at_unix_ms = 10
                     WHERE scan_id = ?1 AND status = 'running'",
                    [lease.scan_id().as_str()],
                )
                .is_err()
        );
        assert_eq!(
            snapshot_temp_lease_state(&connection, &lease).unwrap(),
            SnapshotTempLeaseState::Exact
        );

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        delete_snapshot_temp_lease(&transaction, &lease).unwrap();
        assert_eq!(
            transaction
                .execute(
                    "UPDATE scans SET status = 'succeeded', completed_at_unix_ms = 10
                     WHERE scan_id = ?1 AND status = 'running'",
                    [lease.scan_id().as_str()],
                )
                .unwrap(),
            1
        );
        transaction.commit().unwrap();
    }

    #[test]
    fn population_is_complete_bounded_and_rows_are_immutable() {
        let mut connection = fresh_connection();
        for index in 0..MAX_SNAPSHOT_TEMP_LEASES as u64 {
            let lease = prepared(index + 10, 42);
            insert_running_scan(&connection, lease.scan_id(), index as i64);
            insert(&mut connection, &lease);
        }
        assert_eq!(
            inspect_snapshot_temp_leases(&connection)
                .unwrap()
                .rows()
                .len(),
            MAX_SNAPSHOT_TEMP_LEASES
        );

        let overflow = prepared(1_000, 42);
        insert_running_scan(&connection, overflow.scan_id(), 1_000);
        assert!(
            connection
                .execute(
                    "INSERT INTO snapshot_temp_leases VALUES
                     (?1, 1, ?2, 'running', ?3, ?4, ?5, ?6)",
                    params![
                        overflow.id.as_str(),
                        overflow.scan_id.as_str(),
                        overflow.final_name.as_str(),
                        overflow.temp_name.as_str(),
                        overflow.owner.as_str(),
                        overflow.created_at_unix_ms,
                    ],
                )
                .is_err()
        );
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let error = insert_snapshot_temp_lease(&transaction, &overflow).unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::QueryLimitExceeded);
        drop(transaction);

        assert!(
            connection
                .execute(
                    "UPDATE snapshot_temp_leases SET created_at_unix_ms = 999",
                    [],
                )
                .is_err()
        );
    }

    #[test]
    fn hostile_missing_parent_malformed_owner_and_oversized_population_fail_closed() {
        let connection = fresh_connection();
        let missing = prepared(2_000, 42);
        connection
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        let enabled = connection
            .set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        assert!(!enabled);
        connection
            .execute(
                "INSERT INTO snapshot_temp_leases VALUES (?1, 1, ?2, 'running', ?3, ?4, ?5, ?6)",
                params![
                    missing.id.as_str(),
                    missing.scan_id.as_str(),
                    missing.final_name.as_str(),
                    missing.temp_name.as_str(),
                    missing.owner.as_str(),
                    missing.created_at_unix_ms,
                ],
            )
            .unwrap();
        assert_eq!(
            inspect_snapshot_temp_leases(&connection).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );

        let connection = fresh_connection();
        let succeeded = prepared(2_500, 42);
        insert_running_scan(&connection, succeeded.scan_id(), 0);
        connection
            .execute(
                "UPDATE scans SET status = 'succeeded', completed_at_unix_ms = 10
                 WHERE scan_id = ?1",
                [succeeded.scan_id().as_str()],
            )
            .unwrap();
        let enabled = connection
            .set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        assert!(!enabled);
        connection
            .execute(
                "INSERT INTO snapshot_temp_leases VALUES (?1, 1, ?2, 'running', ?3, ?4, ?5, ?6)",
                params![
                    succeeded.id.as_str(),
                    succeeded.scan_id.as_str(),
                    succeeded.final_name.as_str(),
                    succeeded.temp_name.as_str(),
                    succeeded.owner.as_str(),
                    succeeded.created_at_unix_ms,
                ],
            )
            .unwrap();
        assert_eq!(
            inspect_snapshot_temp_leases(&connection).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );

        let connection = fresh_connection();
        let malformed = prepared(3_000, 42);
        insert_running_scan(&connection, malformed.scan_id(), 0);
        connection
            .execute(
                "INSERT INTO snapshot_temp_leases VALUES (?1, 1, ?2, 'running', ?3, ?4, 'bad', ?5)",
                params![
                    malformed.id.as_str(),
                    malformed.scan_id.as_str(),
                    malformed.final_name.as_str(),
                    malformed.temp_name.as_str(),
                    malformed.created_at_unix_ms,
                ],
            )
            .unwrap();
        assert_eq!(
            inspect_snapshot_temp_leases(&connection).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );

        let connection = fresh_connection();
        let enabled = connection
            .set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        assert!(!enabled);
        for index in 0..=MAX_SNAPSHOT_TEMP_LEASES as u64 {
            let lease = prepared(index + 4_000, 42);
            insert_running_scan(&connection, lease.scan_id(), index as i64);
            connection
                .execute(
                    "INSERT INTO snapshot_temp_leases VALUES (?1, 1, ?2, 'running', ?3, ?4, ?5, ?6)",
                    params![
                        lease.id.as_str(),
                        lease.scan_id.as_str(),
                        lease.final_name.as_str(),
                        lease.temp_name.as_str(),
                        lease.owner.as_str(),
                        lease.created_at_unix_ms,
                    ],
                )
                .unwrap();
        }
        assert_eq!(
            inspect_snapshot_temp_leases(&connection).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }
}

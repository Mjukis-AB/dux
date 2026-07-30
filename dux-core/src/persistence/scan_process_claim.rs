//! Bounded process ownership for durable running scans.
//!
//! Claims are liveness evidence only. They grant no scan, snapshot, planner,
//! or cleanup authority. Recovery probes process state without SQLite locks and
//! exact-CASes only one claim whose owner is definitely gone.

use std::time::SystemTime;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::domain::ScanId;

use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::process_liveness::{ProcessInstanceId, ProcessLiveness};

pub(super) const MAX_SCAN_PROCESS_CLAIMS: usize = 64;

pub(super) struct ScanProcessClaimPage {
    pub(super) claims: Vec<ScanProcessClaim>,
    pub(super) has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanRecoveryBatchOutcome {
    NoClaim,
    DeferredUnproven,
    Interrupted,
    ChangedConcurrently,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanRecoveryBatchResult {
    pub(crate) observed_at: SystemTime,
    pub(crate) outcome: ScanRecoveryBatchOutcome,
    pub(crate) claimed_count_before: u32,
    pub(crate) claimed_count_after: u32,
    pub(crate) alive_count: u32,
    pub(crate) unknown_count: u32,
    pub(crate) recoverable_count: u32,
    pub(crate) has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ScanProcessClaim {
    scan_id: ScanId,
    owner: ProcessInstanceId,
    recovery_scope: Option<String>,
    claimed_at_unix_ms: i64,
}

impl ScanProcessClaim {
    pub(super) fn owner(&self) -> &ProcessInstanceId {
        &self.owner
    }

    pub(super) fn cursor(&self) -> (i64, &str) {
        (self.claimed_at_unix_ms, self.scan_id.as_str())
    }
}

pub(super) fn insert_scan_process_claim(
    transaction: &Transaction<'_>,
    scan_id: &ScanId,
    started_at: SystemTime,
    owner: &ProcessInstanceId,
) -> Result<(), HistoryError> {
    let claimed_at_unix_ms = system_time_to_unix_ms(started_at, HistoryErrorKind::InvalidInput)?;
    let changed = transaction
        .execute(
            "INSERT INTO scan_process_claims (
                 scan_id, record_format_version, owner_process_instance,
                 recovery_scope, claimed_at_unix_ms
             ) VALUES (?1, 1, ?2, ?3, ?4)",
            params![
                scan_id.as_str(),
                owner.as_str(),
                owner.recovery_scope_key(),
                claimed_at_unix_ms
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
    }
}

/// Consume only this coordinator's exact immutable claim. An unclaimed row is
/// legacy v8 debt and remains completable by the existing typed low-level test
/// paths, but a foreign claim can never be consumed by scan ID alone.
pub(super) fn consume_owned_scan_process_claim(
    transaction: &Transaction<'_>,
    scan_id: &ScanId,
    owner: &ProcessInstanceId,
) -> Result<(), HistoryError> {
    let stored_claim = transaction
        .query_row(
            "SELECT typeof(owner_process_instance),
                    length(CAST(owner_process_instance AS BLOB)),
                    owner_process_instance, record_format_version,
                    typeof(recovery_scope),
                    length(CAST(recovery_scope AS BLOB)), recovery_scope,
                    claimed_at_unix_ms,
                    (SELECT started_at_unix_ms FROM scans AS parent
                     WHERE parent.scan_id = scan_process_claims.scan_id)
             FROM scan_process_claims WHERE scan_id = ?1",
            [scan_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                ))
            },
        )
        .optional()
        .map_err(map_query_sql_error)?;
    let Some((
        owner_storage,
        owner_length,
        stored_owner,
        format,
        scope_storage,
        scope_length,
        stored_scope,
        claimed_at_unix_ms,
        parent_started_at_unix_ms,
    )) = stored_claim
    else {
        return Ok(());
    };
    if owner_storage != "text"
        || !(1..=128).contains(&owner_length)
        || stored_owner.len() != owner_length as usize
        || format != 1
        || claimed_at_unix_ms < 0
        || parent_started_at_unix_ms != Some(claimed_at_unix_ms)
    {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    let decoded = ProcessInstanceId::from_stored(&stored_owner)
        .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
    let expected_scope = decoded.recovery_scope_key();
    let scope_is_well_formed = match &stored_scope {
        None => scope_storage == "null" && scope_length.is_none(),
        Some(scope) => {
            scope_storage == "text"
                && scope_length == i64::try_from(scope.len()).ok()
                && scope.len() == 66
        }
    };
    if !scope_is_well_formed || expected_scope != stored_scope {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    if &decoded != owner {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    let changed = transaction
        .execute(
            "DELETE FROM scan_process_claims
             WHERE scan_id = ?1 AND owner_process_instance = ?2
               AND recovery_scope IS ?3 AND claimed_at_unix_ms = ?4
               AND record_format_version = 1",
            params![
                scan_id.as_str(),
                owner.as_str(),
                expected_scope,
                claimed_at_unix_ms
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
    }
}

pub(super) fn exact_scan_process_claim_matches(
    connection: &Connection,
    scan_id: &ScanId,
    started_at: SystemTime,
    owner: &ProcessInstanceId,
) -> Result<bool, HistoryError> {
    let claimed_at = system_time_to_unix_ms(started_at, HistoryErrorKind::InvalidInput)?;
    let recovery_scope = owner.recovery_scope_key();
    let matched: i64 = connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM scan_process_claims
                 WHERE scan_id = ?1 AND owner_process_instance = ?2
                   AND claimed_at_unix_ms = ?3 AND recovery_scope IS ?4
                   AND record_format_version = 1
             )",
            params![scan_id.as_str(), owner.as_str(), claimed_at, recovery_scope],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    Ok(matched == 1)
}

pub(super) fn scan_process_claim_is_missing(
    connection: &Connection,
    scan_id: &ScanId,
) -> Result<bool, HistoryError> {
    let missing: i64 = connection
        .query_row(
            "SELECT NOT EXISTS(
                 SELECT 1 FROM scan_process_claims WHERE scan_id = ?1
             )",
            [scan_id.as_str()],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    Ok(missing == 1)
}

fn scan_process_claim_page_query(has_scope: bool) -> String {
    // Keep the equality-bound scope first and the cursor as one row-value
    // range so SQLite can seek and stream directly through
    // scan_process_claims_by_time. Nullable/optional OR predicates here
    // turn legal same-scope history into an unbounded scan plus temp sort.
    let scope_predicate = if has_scope {
        "claim.recovery_scope = ?1
         AND (claim.claimed_at_unix_ms, claim.scan_id) > (?2, ?3)"
    } else {
        "claim.recovery_scope IS NULL
         AND (claim.claimed_at_unix_ms, claim.scan_id) > (?1, ?2)"
    };
    format!(
        "SELECT
             typeof(claim.scan_id), length(CAST(claim.scan_id AS BLOB)),
             claim.scan_id,
             claim.record_format_version,
             typeof(claim.owner_process_instance),
             length(CAST(claim.owner_process_instance AS BLOB)),
             claim.owner_process_instance,
             typeof(claim.recovery_scope),
             length(CAST(claim.recovery_scope AS BLOB)),
             claim.recovery_scope,
             claim.claimed_at_unix_ms,
             CASE WHEN EXISTS (
                 SELECT 1 FROM scans AS parent
                 WHERE parent.scan_id = claim.scan_id
                   AND parent.status = 'running'
                   AND parent.completed_at_unix_ms IS NULL
                   AND parent.directory_count = 0 AND parent.file_count = 0
                   AND parent.logical_bytes = 0 AND parent.allocated_bytes IS NULL
                   AND parent.snapshot_version IS NULL
                   AND parent.snapshot_relative_path IS NULL
                   AND parent.snapshot_relative_path_encoding IS NULL
                   AND parent.snapshot_checksum_sha256 IS NULL
                   AND parent.coverage_status = 'unknown'
                   AND parent.coverage_permille IS NULL
                   AND parent.issue_count = 0
                   AND parent.started_at_unix_ms = claim.claimed_at_unix_ms
                   AND NOT EXISTS (
                       SELECT 1 FROM scan_issues
                       WHERE scan_issues.scan_id = parent.scan_id
                   )
             ) THEN 1 ELSE 0 END
         FROM scan_process_claims AS claim
         WHERE {scope_predicate}
         ORDER BY claim.claimed_at_unix_ms, claim.scan_id
         LIMIT 65"
    )
}

pub(super) fn load_scan_process_claim_page(
    connection: &Connection,
    recovery_scope: Option<&str>,
    after: Option<(i64, &str)>,
) -> Result<ScanProcessClaimPage, HistoryError> {
    run_bounded_query(connection, || {
        let query = scan_process_claim_page_query(recovery_scope.is_some());
        let mut statement = connection.prepare(&query).map_err(map_query_sql_error)?;
        let (after_time, after_scan_id) = after.unwrap_or((-1, ""));
        let mut rows = if let Some(scope) = recovery_scope {
            statement.query(params![scope, after_time, after_scan_id])
        } else {
            statement.query(params![after_time, after_scan_id])
        }
        .map_err(map_query_sql_error)?;
        let mut claims = Vec::new();
        while let Some(row) = rows.next().map_err(map_query_sql_error)? {
            let id_storage: String = row.get(0).map_err(map_query_sql_error)?;
            let id_length: i64 = row.get(1).map_err(map_query_sql_error)?;
            let id: String = row.get(2).map_err(map_query_sql_error)?;
            let format: i64 = row.get(3).map_err(map_query_sql_error)?;
            let owner_storage: String = row.get(4).map_err(map_query_sql_error)?;
            let owner_length: i64 = row.get(5).map_err(map_query_sql_error)?;
            let owner: String = row.get(6).map_err(map_query_sql_error)?;
            let scope_storage: String = row.get(7).map_err(map_query_sql_error)?;
            let scope_length: Option<i64> = row.get(8).map_err(map_query_sql_error)?;
            let stored_scope: Option<String> = row.get(9).map_err(map_query_sql_error)?;
            let claimed_at_unix_ms: i64 = row.get(10).map_err(map_query_sql_error)?;
            let parent_is_pristine: i64 = row.get(11).map_err(map_query_sql_error)?;
            if id_storage != "text"
                || !(1..=128).contains(&id_length)
                || id.len() != id_length as usize
                || format != 1
                || owner_storage != "text"
                || !(1..=128).contains(&owner_length)
                || owner.len() != owner_length as usize
                || claimed_at_unix_ms < 0
                || parent_is_pristine != 1
            {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            let owner = ProcessInstanceId::from_stored(&owner)
                .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
            if (stored_scope.is_none() && (scope_storage != "null" || scope_length.is_some()))
                || stored_scope.as_ref().is_some_and(|scope| {
                    scope_storage != "text"
                        || scope_length != i64::try_from(scope.len()).ok()
                        || scope.len() != 66
                })
                || owner.recovery_scope_key() != stored_scope
                || stored_scope.as_deref() != recovery_scope
            {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            claims.push(ScanProcessClaim {
                scan_id: ScanId::new(id)
                    .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?,
                owner,
                recovery_scope: stored_scope,
                claimed_at_unix_ms,
            });
        }
        let has_more = claims.len() > MAX_SCAN_PROCESS_CLAIMS;
        claims.truncate(MAX_SCAN_PROCESS_CLAIMS);
        Ok(ScanProcessClaimPage { claims, has_more })
    })
}

pub(super) fn count_remaining_scan_process_claims(
    connection: &Connection,
    claims: &[ScanProcessClaim],
) -> Result<u32, HistoryError> {
    run_bounded_query(connection, || {
        let mut remaining = 0_u32;
        for claim in claims {
            let exists: i64 = connection
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM scan_process_claims
                         WHERE scan_id = ?1 AND owner_process_instance = ?2
                           AND recovery_scope IS ?3 AND claimed_at_unix_ms = ?4
                     )",
                    params![
                        claim.scan_id.as_str(),
                        claim.owner.as_str(),
                        claim.recovery_scope.as_deref(),
                        claim.claimed_at_unix_ms,
                    ],
                    |row| row.get(0),
                )
                .map_err(map_query_sql_error)?;
            if exists == 1 {
                remaining = remaining
                    .checked_add(1)
                    .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
            }
        }
        Ok(remaining)
    })
}

#[cfg(test)]
pub(super) fn load_scan_process_claims(
    connection: &Connection,
) -> Result<Vec<ScanProcessClaim>, HistoryError> {
    let scope = super::process_liveness::current_process_instance()
        .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?
        .recovery_scope_key();
    Ok(load_scan_process_claim_page(connection, scope.as_deref(), None)?.claims)
}

pub(super) fn interrupt_scan_process_claim(
    transaction: &Transaction<'_>,
    claim: &ScanProcessClaim,
    completed_at_unix_ms: i64,
) -> Result<bool, HistoryError> {
    if completed_at_unix_ms < claim.claimed_at_unix_ms {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    let deleted = transaction
        .execute(
            "DELETE FROM scan_process_claims
             WHERE scan_id = ?1 AND owner_process_instance = ?2
               AND recovery_scope IS ?3 AND claimed_at_unix_ms = ?4
               AND record_format_version = 1
               AND EXISTS (
                   SELECT 1 FROM scans AS parent
                   WHERE parent.scan_id = scan_process_claims.scan_id
                     AND parent.status = 'running'
                     AND parent.completed_at_unix_ms IS NULL
                     AND parent.directory_count = 0 AND parent.file_count = 0
                     AND parent.logical_bytes = 0 AND parent.allocated_bytes IS NULL
                     AND parent.snapshot_version IS NULL
                     AND parent.snapshot_relative_path IS NULL
                     AND parent.snapshot_relative_path_encoding IS NULL
                     AND parent.snapshot_checksum_sha256 IS NULL
                     AND parent.coverage_status = 'unknown'
                     AND parent.coverage_permille IS NULL
                     AND parent.issue_count = 0
                     AND parent.started_at_unix_ms = scan_process_claims.claimed_at_unix_ms
                     AND NOT EXISTS (
                         SELECT 1 FROM scan_issues
                         WHERE scan_issues.scan_id = parent.scan_id
                     )
               )",
            params![
                claim.scan_id.as_str(),
                claim.owner.as_str(),
                claim.recovery_scope.as_deref(),
                claim.claimed_at_unix_ms,
            ],
        )
        .map_err(map_write_sql_error)?;
    if deleted == 0 {
        return Ok(false);
    }
    let updated = transaction
        .execute(
            "UPDATE scans
             SET completed_at_unix_ms = ?2, status = 'interrupted'
             WHERE scan_id = ?1 AND status = 'running'
               AND completed_at_unix_ms IS NULL AND started_at_unix_ms <= ?2
               AND directory_count = 0 AND file_count = 0 AND logical_bytes = 0
               AND allocated_bytes IS NULL AND snapshot_version IS NULL
               AND snapshot_relative_path IS NULL
               AND snapshot_relative_path_encoding IS NULL
               AND snapshot_checksum_sha256 IS NULL
               AND coverage_status = 'unknown' AND coverage_permille IS NULL
               AND issue_count = 0
               AND NOT EXISTS (
                   SELECT 1 FROM scan_issues WHERE scan_issues.scan_id = scans.scan_id
               )",
            params![claim.scan_id.as_str(), completed_at_unix_ms],
        )
        .map_err(map_write_sql_error)?;
    if updated != 1 {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    Ok(true)
}

pub(super) fn exact_recovered_scan_matches(
    connection: &Connection,
    claim: &ScanProcessClaim,
    completed_at_unix_ms: i64,
) -> Result<bool, HistoryError> {
    let matched: i64 = connection
        .query_row(
            "SELECT
                 NOT EXISTS (
                     SELECT 1 FROM scan_process_claims WHERE scan_id = ?1
                 ) AND EXISTS (
                     SELECT 1 FROM scans
                     WHERE scan_id = ?1 AND status = 'interrupted'
                       AND completed_at_unix_ms = ?2
                       AND started_at_unix_ms = ?3
                       AND directory_count = 0 AND file_count = 0
                       AND logical_bytes = 0 AND allocated_bytes IS NULL
                       AND snapshot_version IS NULL AND snapshot_relative_path IS NULL
                       AND snapshot_relative_path_encoding IS NULL
                       AND snapshot_checksum_sha256 IS NULL
                       AND coverage_status = 'unknown' AND coverage_permille IS NULL
                       AND issue_count = 0
                       AND NOT EXISTS (
                           SELECT 1 FROM scan_issues
                           WHERE scan_issues.scan_id = scans.scan_id
                       )
                 )",
            params![
                claim.scan_id.as_str(),
                completed_at_unix_ms,
                claim.claimed_at_unix_ms,
            ],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    Ok(matched == 1)
}

pub(super) fn canonical_recovery_time(
    value: SystemTime,
) -> Result<(SystemTime, i64), HistoryError> {
    let millis = system_time_to_unix_ms(value, HistoryErrorKind::InvalidInput)?;
    Ok((unix_ms_to_system_time(millis)?, millis))
}

pub(super) fn classify_claims(
    claims: &[ScanProcessClaim],
    mut probe: impl FnMut(&ProcessInstanceId) -> ProcessLiveness,
) -> (Vec<ProcessLiveness>, u32, u32, u32) {
    let liveness = claims
        .iter()
        .map(|claim| probe(claim.owner()))
        .collect::<Vec<_>>();
    let alive = liveness
        .iter()
        .filter(|state| **state == ProcessLiveness::Alive)
        .count() as u32;
    let unknown = liveness
        .iter()
        .filter(|state| **state == ProcessLiveness::Unknown)
        .count() as u32;
    let recoverable = liveness
        .iter()
        .filter(|state| **state == ProcessLiveness::DefinitelyGone)
        .count() as u32;
    (liveness, alive, unknown, recoverable)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::ScanId;
    use crate::persistence::{
        NewScanRecord, ScanCompletionRecord, ScanCounts, ScanStatus, StoreCoordinator,
        TerminalScanStatus,
    };

    fn fixture() -> (
        TempDir,
        std::sync::Arc<StoreCoordinator>,
        std::path::PathBuf,
    ) {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("scan-root");
        std::fs::create_dir(&root).unwrap();
        let store = StoreCoordinator::open(&temp.path().join("data/dux.sqlite3")).unwrap();
        (temp, store, root)
    }

    fn start(
        store: &StoreCoordinator,
        root: &std::path::Path,
        id: &str,
        offset_ms: u64,
    ) -> NewScanRecord {
        let record = NewScanRecord::try_new_without_root_identity(
            ScanId::new(id).unwrap(),
            root.to_path_buf(),
            UNIX_EPOCH + Duration::from_millis(offset_ms),
        )
        .unwrap();
        store.record_scan_started_reconciled(&record).unwrap();
        record
    }

    fn finish(store: &StoreCoordinator, scan: &NewScanRecord, offset_ms: u64) {
        store
            .record_scan_finished_reconciled(
                &ScanCompletionRecord::try_new(
                    scan.id().clone(),
                    UNIX_EPOCH + Duration::from_millis(offset_ms),
                    TerminalScanStatus::Failed,
                    ScanCounts::default(),
                )
                .unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn store_starts_atomically_claim_and_normal_completion_consumes_exact_claim() {
        let (_temp, store, root) = fixture();
        let first = start(&store, &root, "scan:claim-first", 10);
        let second = start(&store, &root, "scan:claim-second", 11);
        let owners = store.with_connection(|connection| {
            connection
                .prepare("SELECT owner_process_instance FROM scan_process_claims ORDER BY scan_id")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        });
        assert_eq!(owners.len(), 2);
        assert_eq!(owners[0], owners[1]);

        finish(&store, &first, 20);
        let claims =
            store.with_connection(|connection| load_scan_process_claims(connection).unwrap());
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].scan_id, second.id().clone());
    }

    #[test]
    fn normal_completion_refuses_a_scope_mismatched_claim_without_erasing_it() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:claim-scope-mismatch", 10);
        store.with_connection(|connection| {
            let (owner, scope): (String, Option<String>) = connection
                .query_row(
                    "SELECT owner_process_instance, recovery_scope
                     FROM scan_process_claims WHERE scan_id = ?1",
                    [scan.id().as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let mismatched_scope = if scope.is_some() {
                None
            } else {
                Some(format!("l:{}", "0".repeat(64)))
            };
            connection
                .execute(
                    "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                    [scan.id().as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO scan_process_claims (
                         scan_id, record_format_version, owner_process_instance,
                         recovery_scope, claimed_at_unix_ms
                     ) VALUES (?1, 1, ?2, ?3, 10)",
                    params![scan.id().as_str(), owner, mismatched_scope],
                )
                .unwrap();
        });

        let completion = ScanCompletionRecord::try_new(
            scan.id().clone(),
            UNIX_EPOCH + Duration::from_millis(20),
            TerminalScanStatus::Failed,
            ScanCounts::default(),
        )
        .unwrap();
        assert_eq!(
            store
                .record_scan_finished_reconciled(&completion)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Running
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM scan_process_claims WHERE scan_id = ?1",
                        [scan.id().as_str()],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1
            );
        });
    }

    #[test]
    fn ambiguous_start_reconciliation_rejects_a_scope_mismatched_claim() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:start-reconcile-scope-mismatch", 10);
        let owner = crate::persistence::process_liveness::current_process_instance().unwrap();
        let expected_scope = owner.recovery_scope_key();
        let mismatched_scope = if expected_scope.is_some() {
            None
        } else {
            Some(format!("l:{}", "0".repeat(64)))
        };
        store.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                    [scan.id().as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO scan_process_claims (
                         scan_id, record_format_version, owner_process_instance,
                         recovery_scope, claimed_at_unix_ms
                     ) VALUES (?1, 1, ?2, ?3, 10)",
                    params![scan.id().as_str(), owner.as_str(), mismatched_scope],
                )
                .unwrap();
            assert!(!exact_scan_process_claim_matches(
                connection,
                scan.id(),
                scan.started_at(),
                &owner,
            )
            .unwrap());
        });
    }

    #[test]
    fn alive_and_unknown_claims_are_exact_no_ops() {
        for state in [ProcessLiveness::Alive, ProcessLiveness::Unknown] {
            let (_temp, store, root) = fixture();
            let scan = start(&store, &root, "scan:unproven", 10);
            let before = store.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT status, completed_at_unix_ms,
                                (SELECT count(*) FROM scan_process_claims)
                         FROM scans WHERE scan_id = ?1",
                        [scan.id().as_str()],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, Option<i64>>(1)?,
                                row.get::<_, i64>(2)?,
                            ))
                        },
                    )
                    .unwrap()
            });
            let result = store
                .run_scan_recovery_batch_with_hooks_for_test(
                    UNIX_EPOCH + Duration::from_millis(20),
                    |_| state,
                    || Ok(()),
                    || Ok(()),
                )
                .unwrap();
            assert_eq!(result.outcome, ScanRecoveryBatchOutcome::DeferredUnproven);
            assert_eq!(result.claimed_count_before, 1);
            assert_eq!(result.claimed_count_after, 1);
            assert_eq!(
                result.alive_count,
                u32::from(state == ProcessLiveness::Alive)
            );
            assert_eq!(
                result.unknown_count,
                u32::from(state == ProcessLiveness::Unknown)
            );
            let after = store.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT status, completed_at_unix_ms,
                                (SELECT count(*) FROM scan_process_claims)
                         FROM scans WHERE scan_id = ?1",
                        [scan.id().as_str()],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, Option<i64>>(1)?,
                                row.get::<_, i64>(2)?,
                            ))
                        },
                    )
                    .unwrap()
            });
            assert_eq!(after, before);
        }
    }

    #[test]
    fn complete_inventory_recovers_past_an_unknown_owner_one_at_a_time() {
        let (_temp, store, root) = fixture();
        let first = start(&store, &root, "scan:recovery-first", 10);
        let second = start(&store, &root, "scan:recovery-second", 11);
        let mut probes = 0;
        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(20),
                |_| {
                    probes += 1;
                    if probes == 1 {
                        ProcessLiveness::Unknown
                    } else {
                        ProcessLiveness::DefinitelyGone
                    }
                },
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(probes, 2);
        assert_eq!(result.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(result.unknown_count, 1);
        assert_eq!(result.recoverable_count, 1);
        assert_eq!(result.claimed_count_before, 2);
        assert_eq!(result.claimed_count_after, 1);
        assert!(!result.has_more);
        assert_eq!(
            store.load_scan(first.id()).unwrap().unwrap().status(),
            ScanStatus::Running
        );
        assert_eq!(
            store.load_scan(second.id()).unwrap().unwrap().status(),
            ScanStatus::Interrupted
        );
    }

    #[test]
    fn terminal_write_winning_after_probe_is_reported_without_overwrite() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:recovery-race", 10);
        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(30),
                |_| ProcessLiveness::DefinitelyGone,
                || {
                    finish(&store, &scan, 20);
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            result.outcome,
            ScanRecoveryBatchOutcome::ChangedConcurrently
        );
        assert_eq!(result.claimed_count_after, 0);
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Failed
        );
    }

    #[test]
    fn recovery_cas_rejects_a_scope_replacement_after_liveness_probe() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:recovery-scope-race", 10);
        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(30),
                |_| ProcessLiveness::DefinitelyGone,
                || {
                    store.with_connection(|connection| {
                        let (owner, scope): (String, Option<String>) = connection
                            .query_row(
                                "SELECT owner_process_instance, recovery_scope
                                 FROM scan_process_claims WHERE scan_id = ?1",
                                [scan.id().as_str()],
                                |row| Ok((row.get(0)?, row.get(1)?)),
                            )
                            .unwrap();
                        let mismatched_scope = if scope.is_some() {
                            None
                        } else {
                            Some(format!("l:{}", "0".repeat(64)))
                        };
                        connection
                            .execute(
                                "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                                [scan.id().as_str()],
                            )
                            .unwrap();
                        connection
                            .execute(
                                "INSERT INTO scan_process_claims (
                                     scan_id, record_format_version,
                                     owner_process_instance, recovery_scope,
                                     claimed_at_unix_ms
                                 ) VALUES (?1, 1, ?2, ?3, 10)",
                                params![scan.id().as_str(), owner, mismatched_scope],
                            )
                            .unwrap();
                    });
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            result.outcome,
            ScanRecoveryBatchOutcome::ChangedConcurrently
        );
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Running
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM scan_process_claims WHERE scan_id = ?1",
                        [scan.id().as_str()],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1
            );
        });
    }

    #[test]
    fn successful_recovery_reloads_honest_after_count_across_concurrent_completion() {
        let (_temp, store, root) = fixture();
        let recovered = start(&store, &root, "scan:recovery-count-target", 10);
        let concurrent = start(&store, &root, "scan:recovery-count-concurrent", 11);
        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(30),
                |_| ProcessLiveness::DefinitelyGone,
                || {
                    finish(&store, &concurrent, 20);
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(result.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(result.claimed_count_before, 2);
        assert_eq!(result.claimed_count_after, 0);
        assert_eq!(result.recoverable_count, 2);
        assert!(result.has_more);
        assert_eq!(
            store.load_scan(recovered.id()).unwrap().unwrap().status(),
            ScanStatus::Interrupted
        );
        assert_eq!(
            store.load_scan(concurrent.id()).unwrap().unwrap().status(),
            ScanStatus::Failed
        );
    }

    #[test]
    fn exact_post_commit_reconciliation_returns_interrupted() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:recovery-ambiguous", 10);
        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(20),
                |_| ProcessLiveness::DefinitelyGone,
                || Ok(()),
                || Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable)),
            )
            .unwrap();
        assert_eq!(result.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(result.claimed_count_after, 0);
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Interrupted
        );
    }

    #[test]
    fn malformed_owner_fails_closed_without_mutating_scan_or_claim() {
        let (_temp, store, root) = fixture();
        let scan = NewScanRecord::try_new_without_root_identity(
            ScanId::new("scan:malformed-owner").unwrap(),
            root.clone(),
            UNIX_EPOCH + Duration::from_millis(10),
        )
        .unwrap();
        let observed = crate::persistence::observe_host_path(&root).unwrap();
        let encoding = match observed.encoding() {
            crate::persistence::HostPathObservationEncoding::Utf8 => 1_i64,
            crate::persistence::HostPathObservationEncoding::Utf16LittleEndian => 2_i64,
        };
        let scope = crate::persistence::process_liveness::current_process_instance()
            .unwrap()
            .recovery_scope_key();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO scans (
                         scan_id, root_path, root_path_encoding, started_at_unix_ms,
                         status, coverage_status
                     ) VALUES (?1, ?2, ?3, 10, 'running', 'unknown')",
                    params![scan.id().as_str(), observed.bytes(), encoding],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO scan_process_claims (
                         scan_id, record_format_version, owner_process_instance,
                         recovery_scope, claimed_at_unix_ms
                     ) VALUES (?1, 1, 'malformed', ?2, 10)",
                    params![scan.id().as_str(), scope],
                )
                .unwrap();
        });
        let before = store.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT status, completed_at_unix_ms, owner_process_instance
                     FROM scans JOIN scan_process_claims USING (scan_id)
                     WHERE scan_id = ?1",
                    [scan.id().as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .unwrap()
        });
        assert_eq!(
            store
                .run_scan_recovery_batch_with_hooks_for_test(
                    UNIX_EPOCH + Duration::from_millis(20),
                    |_| ProcessLiveness::DefinitelyGone,
                    || Ok(()),
                    || Ok(()),
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
        let after = store.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT status, completed_at_unix_ms, owner_process_instance
                     FROM scans JOIN scan_process_claims USING (scan_id)
                     WHERE scan_id = ?1",
                    [scan.id().as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .unwrap()
        });
        assert_eq!(after, before);
    }

    #[test]
    fn competing_recoverer_wins_exactly_once() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:two-recoverers", 10);
        let winner = std::cell::RefCell::new(None);
        let outer = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(20),
                |_| ProcessLiveness::DefinitelyGone,
                || {
                    *winner.borrow_mut() = Some(
                        store
                            .run_scan_recovery_batch_with_hooks_for_test(
                                UNIX_EPOCH + Duration::from_millis(20),
                                |_| ProcessLiveness::DefinitelyGone,
                                || Ok(()),
                                || Ok(()),
                            )
                            .unwrap(),
                    );
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            winner.borrow().as_ref().unwrap().outcome,
            ScanRecoveryBatchOutcome::Interrupted
        );
        assert_eq!(outer.outcome, ScanRecoveryBatchOutcome::ChangedConcurrently);
        assert_eq!(outer.claimed_count_after, 0);
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Interrupted
        );
    }

    #[test]
    fn schema_upgrade_between_probe_and_cas_preserves_running_claim() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:recovery-schema-race", 10);
        let result = store.run_scan_recovery_batch_with_hooks_for_test(
            UNIX_EPOCH + Duration::from_millis(20),
            |_| ProcessLiveness::DefinitelyGone,
            || {
                store.with_connection(|connection| {
                    let future = i64::from(crate::DATABASE_SCHEMA_VERSION + 1);
                    connection
                        .execute(
                            "INSERT INTO schema_migrations (
                                 version, name, checksum_sha256, applied_at_unix_ms
                             ) VALUES (?1, 'future-scan-recovery', zeroblob(32), 20)",
                            [future],
                        )
                        .unwrap();
                    connection
                        .pragma_update(None, "user_version", future)
                        .unwrap();
                });
                Ok(())
            },
            || Ok(()),
        );
        assert_eq!(
            result.unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
        store.with_connection(|connection| {
            let row: (String, Option<i64>, i64) = connection
                .query_row(
                    "SELECT status, completed_at_unix_ms,
                            (SELECT count(*) FROM scan_process_claims WHERE scan_id = ?1)
                     FROM scans WHERE scan_id = ?1",
                    [scan.id().as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(row, ("running".to_owned(), None, 1));
        });
    }

    #[test]
    fn claim_relation_is_immutable_and_hard_bounded() {
        let (_temp, store, root) = fixture();
        for index in 0..MAX_SCAN_PROCESS_CLAIMS {
            start(
                &store,
                &root,
                &format!("scan:bounded-{index:02}"),
                10 + index as u64,
            );
        }
        let overflow = NewScanRecord::try_new_without_root_identity(
            ScanId::new("scan:bounded-overflow").unwrap(),
            root,
            UNIX_EPOCH + Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(
            store
                .record_scan_started_reconciled(&overflow)
                .unwrap_err()
                .kind,
            HistoryErrorKind::DatabaseUnavailable
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM scan_process_claims", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap(),
                MAX_SCAN_PROCESS_CLAIMS as i64
            );
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM scans", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                MAX_SCAN_PROCESS_CLAIMS as i64
            );
            assert!(
                connection
                    .execute(
                        "UPDATE scan_process_claims SET claimed_at_unix_ms = 999",
                        [],
                    )
                    .is_err()
            );
        });
    }

    #[test]
    fn foreign_owner_debt_does_not_lock_admission_and_keyset_page_reaches_later_work() {
        let (_temp, store, root) = fixture();
        let observed = crate::persistence::observe_host_path(&root).unwrap();
        let encoding = match observed.encoding() {
            crate::persistence::HostPathObservationEncoding::Utf8 => 1_i64,
            crate::persistence::HostPathObservationEncoding::Utf16LittleEndian => 2_i64,
        };
        store.with_connection(|connection| {
            for index in 0..=MAX_SCAN_PROCESS_CLAIMS {
                let owner =
                    crate::persistence::process_liveness::current_process_instance().unwrap();
                connection
                    .execute(
                        "INSERT INTO scans (
                             scan_id, root_path, root_path_encoding, started_at_unix_ms,
                             status, coverage_status
                         ) VALUES (?1, ?2, ?3, ?4, 'running', 'unknown')",
                        params![
                            format!("scan:foreign-debt-{index:03}"),
                            observed.bytes(),
                            encoding,
                            100_i64 + index as i64,
                        ],
                    )
                    .unwrap();
                connection
                    .execute(
                        "INSERT INTO scan_process_claims (
                             scan_id, record_format_version, owner_process_instance,
                             recovery_scope, claimed_at_unix_ms
                         ) VALUES (?1, 1, ?2, ?3, ?4)",
                        params![
                            format!("scan:foreign-debt-{index:03}"),
                            owner.as_str(),
                            owner.recovery_scope_key(),
                            100_i64 + index as i64,
                        ],
                    )
                    .unwrap();
            }
        });

        let own = start(&store, &root, "scan:own-after-foreign-debt", 1_000);
        let first = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(2_000),
                |_| ProcessLiveness::Unknown,
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(first.outcome, ScanRecoveryBatchOutcome::DeferredUnproven);
        assert_eq!(first.claimed_count_before, MAX_SCAN_PROCESS_CLAIMS as u32);
        assert!(first.has_more);

        let mut index = 0_u32;
        let second = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(2_001),
                |_| {
                    index += 1;
                    if index == 1 {
                        ProcessLiveness::DefinitelyGone
                    } else {
                        ProcessLiveness::Alive
                    }
                },
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(second.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(second.claimed_count_before, 2);
        assert_eq!(second.claimed_count_after, 1);
        assert!(!second.has_more);
        assert_eq!(
            store.load_scan(own.id()).unwrap().unwrap().status(),
            ScanStatus::Running
        );
    }

    #[test]
    fn claim_page_query_is_an_index_ordered_range_without_a_temp_sort() {
        let (_temp, store, _root) = fixture();
        store.with_connection(|connection| {
            for has_scope in [false, true] {
                let query = format!(
                    "EXPLAIN QUERY PLAN {}",
                    scan_process_claim_page_query(has_scope)
                );
                let mut statement = connection.prepare(&query).unwrap();
                let mut rows = if has_scope {
                    statement.query(params![format!("l:{}", "0".repeat(64)), -1_i64, ""])
                } else {
                    statement.query(params![-1_i64, ""])
                }
                .unwrap();
                let mut details = Vec::new();
                while let Some(row) = rows.next().unwrap() {
                    details.push(row.get::<_, String>(3).unwrap());
                }
                assert!(
                    details.iter().any(|detail| {
                        detail.contains("SEARCH claim USING INDEX scan_process_claims_by_time")
                    }),
                    "query did not seek through the scope/cursor index: {details:?}"
                );
                assert!(
                    details
                        .iter()
                        .all(|detail| !detail.contains("USE TEMP B-TREE")),
                    "query introduced an unbounded temp sort: {details:?}"
                );
            }
        });
    }
}

//! Explicit history-only dismissal of pristine legacy unclaimed running scans.
//!
//! Missing ownership is never interpreted as process death. Preparation
//! selects only exact pristine rows, and commit can change only their durable
//! history status. Snapshot-temp leases are deliberately outside this module.

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, Transaction, params};

use crate::domain::ScanId;

use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
};

pub(crate) const MAX_LEGACY_RUNNING_SCAN_DISMISSALS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
struct LegacyRunningScanWitness {
    scan_id: String,
    volume_id: Option<String>,
    root_path: Vec<u8>,
    root_path_encoding: i64,
    started_at_unix_ms: i64,
    root_identity_v1_sha256: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedLegacyRunningScanDismissal {
    rows: Vec<LegacyRunningScanWitness>,
    completed_at_unix_ms: i64,
    has_more: bool,
}

impl PreparedLegacyRunningScanDismissal {
    pub(crate) fn eligible_count(&self) -> u16 {
        u16::try_from(self.rows.len()).expect("legacy dismissal page is bounded to 64 rows")
    }

    pub(crate) const fn has_more(&self) -> bool {
        self.has_more
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegacyRunningScanDismissalStoreError {
    NothingEligible,
    ChangedSincePreview,
    History(HistoryError),
}

impl From<HistoryError> for LegacyRunningScanDismissalStoreError {
    fn from(value: HistoryError) -> Self {
        Self::History(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegacyRunningScanDismissalReconciliation {
    Applied,
    NotApplied,
    Ambiguous,
}

pub(super) fn prepare_legacy_running_scan_dismissal(
    connection: &Connection,
    completed_at_unix_ms: i64,
) -> Result<PreparedLegacyRunningScanDismissal, LegacyRunningScanDismissalStoreError> {
    if completed_at_unix_ms < 0 {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput).into());
    }
    let mut rows = run_bounded_query(connection, || {
        let mut statement = connection
            .prepare(eligible_rows_query())
            .map_err(map_query_sql_error)?;
        let mut selected = statement
            .query([completed_at_unix_ms])
            .map_err(map_query_sql_error)?;
        let mut rows = Vec::with_capacity(MAX_LEGACY_RUNNING_SCAN_DISMISSALS + 1);
        while let Some(row) = selected.next().map_err(map_query_sql_error)? {
            let scan_id: String = row.get(0).map_err(map_query_sql_error)?;
            let volume_id: Option<String> = row.get(1).map_err(map_query_sql_error)?;
            let root_path: Vec<u8> = row.get(2).map_err(map_query_sql_error)?;
            let root_path_encoding: i64 = row.get(3).map_err(map_query_sql_error)?;
            let started_at_unix_ms: i64 = row.get(4).map_err(map_query_sql_error)?;
            let root_identity_v1_sha256: Option<Vec<u8>> =
                row.get(5).map_err(map_query_sql_error)?;
            validate_witness(
                &scan_id,
                volume_id.as_deref(),
                &root_path,
                root_path_encoding,
                started_at_unix_ms,
                root_identity_v1_sha256.as_deref(),
                completed_at_unix_ms,
            )?;
            rows.push(LegacyRunningScanWitness {
                scan_id,
                volume_id,
                root_path,
                root_path_encoding,
                started_at_unix_ms,
                root_identity_v1_sha256,
            });
        }
        Ok(rows)
    })?;
    let has_more = rows.len() > MAX_LEGACY_RUNNING_SCAN_DISMISSALS;
    if has_more {
        rows.truncate(MAX_LEGACY_RUNNING_SCAN_DISMISSALS);
    }
    if rows.is_empty() {
        return Err(LegacyRunningScanDismissalStoreError::NothingEligible);
    }
    Ok(PreparedLegacyRunningScanDismissal {
        rows,
        completed_at_unix_ms,
        has_more,
    })
}

pub(super) fn apply_legacy_running_scan_dismissal(
    transaction: &Transaction<'_>,
    prepared: &PreparedLegacyRunningScanDismissal,
) -> Result<u16, LegacyRunningScanDismissalStoreError> {
    if prepared.rows.is_empty()
        || prepared.rows.len() > MAX_LEGACY_RUNNING_SCAN_DISMISSALS
        || prepared.completed_at_unix_ms < 0
    {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData).into());
    }
    let mut authorizer = LegacyDismissalAuthorizerGuard::install(transaction)?;
    for witness in &prepared.rows {
        let changed = update_exact_row(transaction, witness, prepared.completed_at_unix_ms)?;
        if changed != 1 {
            authorizer.remove()?;
            return Err(LegacyRunningScanDismissalStoreError::ChangedSincePreview);
        }
    }
    authorizer.remove()?;
    Ok(prepared.eligible_count())
}

pub(super) fn reconcile_legacy_running_scan_dismissal(
    connection: &Connection,
    prepared: &PreparedLegacyRunningScanDismissal,
) -> Result<LegacyRunningScanDismissalReconciliation, HistoryError> {
    run_bounded_query(connection, || {
        let mut applied = 0_usize;
        let mut not_applied = 0_usize;
        for witness in &prepared.rows {
            if exact_post_state(connection, witness, prepared.completed_at_unix_ms)? {
                applied = applied.checked_add(1).ok_or_else(corrupt)?;
            } else if exact_eligible_state(connection, witness, prepared.completed_at_unix_ms)? {
                not_applied = not_applied.checked_add(1).ok_or_else(corrupt)?;
            } else {
                return Ok(LegacyRunningScanDismissalReconciliation::Ambiguous);
            }
        }
        if applied == prepared.rows.len() {
            Ok(LegacyRunningScanDismissalReconciliation::Applied)
        } else if not_applied == prepared.rows.len() {
            Ok(LegacyRunningScanDismissalReconciliation::NotApplied)
        } else {
            Ok(LegacyRunningScanDismissalReconciliation::Ambiguous)
        }
    })
}

pub(super) const fn eligible_rows_query() -> &'static str {
    "SELECT scan.scan_id, scan.volume_id, scan.root_path,
            scan.root_path_encoding, scan.started_at_unix_ms,
            scan.root_identity_v1_sha256
       FROM scans AS scan INDEXED BY scans_running_by_started
      WHERE scan.status = 'running'
        AND scan.completed_at_unix_ms IS NULL
        AND scan.directory_count = 0 AND scan.file_count = 0
        AND scan.logical_bytes = 0 AND scan.allocated_bytes IS NULL
        AND scan.snapshot_version IS NULL
        AND scan.snapshot_relative_path IS NULL
        AND scan.snapshot_relative_path_encoding IS NULL
        AND scan.snapshot_checksum_sha256 IS NULL
        AND scan.coverage_status = 'unknown'
        AND scan.coverage_permille IS NULL AND scan.issue_count = 0
        AND typeof(scan.scan_id) = 'text'
        AND length(CAST(scan.scan_id AS BLOB)) BETWEEN 1 AND 128
        AND (
            scan.volume_id IS NULL OR (
                typeof(scan.volume_id) = 'text'
                AND length(CAST(scan.volume_id AS BLOB)) BETWEEN 1 AND 128
                AND EXISTS (
                    SELECT 1 FROM volumes AS volume
                    WHERE volume.volume_id = scan.volume_id
                )
            )
        )
        AND typeof(scan.root_path) = 'blob'
        AND (
            (scan.root_path_encoding = 1 AND length(scan.root_path) BETWEEN 1 AND 32768)
            OR (
                scan.root_path_encoding = 2
                AND length(scan.root_path) BETWEEN 2 AND 65536
                AND length(scan.root_path) % 2 = 0
            )
        )
        AND typeof(scan.started_at_unix_ms) = 'integer'
        AND scan.started_at_unix_ms BETWEEN 0 AND ?1
        AND (
            scan.root_identity_v1_sha256 IS NULL OR (
                typeof(scan.root_identity_v1_sha256) = 'blob'
                AND length(scan.root_identity_v1_sha256) = 32
            )
        )
        AND NOT EXISTS (
            SELECT 1 FROM scan_process_claims AS claim
            WHERE claim.scan_id = scan.scan_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM scan_issues AS issue WHERE issue.scan_id = scan.scan_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM scan_aggregates AS aggregate WHERE aggregate.scan_id = scan.scan_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM candidates AS candidate WHERE candidate.scan_id = scan.scan_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM candidate_evaluations AS evaluation
            WHERE evaluation.scan_id = scan.scan_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM snapshot_retention_tombstones AS tombstone
            WHERE tombstone.scan_id = scan.scan_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM snapshot_review_pins AS pin WHERE pin.scan_id = scan.scan_id
        )
      ORDER BY scan.started_at_unix_ms, scan.scan_id
      LIMIT 65"
}

fn update_exact_row(
    transaction: &Transaction<'_>,
    witness: &LegacyRunningScanWitness,
    completed_at_unix_ms: i64,
) -> Result<usize, LegacyRunningScanDismissalStoreError> {
    transaction
        .execute(
            &format!(
                "UPDATE scans
                    SET completed_at_unix_ms = ?7, status = 'interrupted'
                  WHERE {}",
                exact_eligible_predicate()
            ),
            params![
                witness.scan_id,
                witness.volume_id,
                witness.root_path,
                witness.root_path_encoding,
                witness.started_at_unix_ms,
                witness.root_identity_v1_sha256,
                completed_at_unix_ms,
            ],
        )
        .map_err(map_write_sql_error)
        .map_err(Into::into)
}

fn exact_eligible_state(
    connection: &Connection,
    witness: &LegacyRunningScanWitness,
    completed_at_unix_ms: i64,
) -> Result<bool, HistoryError> {
    let count: i64 = connection
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM scans WHERE {}",
                exact_eligible_predicate()
            ),
            params![
                witness.scan_id,
                witness.volume_id,
                witness.root_path,
                witness.root_path_encoding,
                witness.started_at_unix_ms,
                witness.root_identity_v1_sha256,
                completed_at_unix_ms,
            ],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    Ok(count == 1)
}

fn exact_post_state(
    connection: &Connection,
    witness: &LegacyRunningScanWitness,
    completed_at_unix_ms: i64,
) -> Result<bool, HistoryError> {
    let count: i64 = connection
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM scans
                  WHERE scan_id = ?1 AND volume_id IS ?2 AND root_path = ?3
                    AND root_path_encoding = ?4 AND started_at_unix_ms = ?5
                    AND root_identity_v1_sha256 IS ?6
                    AND completed_at_unix_ms = ?7 AND status = 'interrupted'
                    AND directory_count = 0 AND file_count = 0
                    AND logical_bytes = 0 AND allocated_bytes IS NULL
                    AND snapshot_version IS NULL
                    AND snapshot_relative_path IS NULL
                    AND snapshot_relative_path_encoding IS NULL
                    AND snapshot_checksum_sha256 IS NULL
                    AND coverage_status = 'unknown'
                    AND coverage_permille IS NULL AND issue_count = 0
                    AND {}",
                no_related_work_predicate()
            ),
            params![
                witness.scan_id,
                witness.volume_id,
                witness.root_path,
                witness.root_path_encoding,
                witness.started_at_unix_ms,
                witness.root_identity_v1_sha256,
                completed_at_unix_ms,
            ],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    Ok(count == 1)
}

fn exact_eligible_predicate() -> &'static str {
    "scan_id = ?1 AND volume_id IS ?2 AND root_path = ?3
     AND root_path_encoding = ?4 AND started_at_unix_ms = ?5
     AND root_identity_v1_sha256 IS ?6
     AND status = 'running' AND completed_at_unix_ms IS NULL
     AND started_at_unix_ms <= ?7
     AND directory_count = 0 AND file_count = 0 AND logical_bytes = 0
     AND allocated_bytes IS NULL AND snapshot_version IS NULL
     AND snapshot_relative_path IS NULL
     AND snapshot_relative_path_encoding IS NULL
     AND snapshot_checksum_sha256 IS NULL
     AND coverage_status = 'unknown' AND coverage_permille IS NULL
     AND issue_count = 0
     AND NOT EXISTS (
         SELECT 1 FROM scan_process_claims AS claim
         WHERE claim.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM scan_issues AS issue WHERE issue.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM scan_aggregates AS aggregate
         WHERE aggregate.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM candidates AS candidate WHERE candidate.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM candidate_evaluations AS evaluation
         WHERE evaluation.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM snapshot_retention_tombstones AS tombstone
         WHERE tombstone.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM snapshot_review_pins AS pin WHERE pin.scan_id = scans.scan_id
     )"
}

fn no_related_work_predicate() -> &'static str {
    "NOT EXISTS (
         SELECT 1 FROM scan_process_claims AS claim
         WHERE claim.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM scan_issues AS issue WHERE issue.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM scan_aggregates AS aggregate
         WHERE aggregate.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM candidates AS candidate WHERE candidate.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM candidate_evaluations AS evaluation
         WHERE evaluation.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM snapshot_retention_tombstones AS tombstone
         WHERE tombstone.scan_id = scans.scan_id
     )
     AND NOT EXISTS (
         SELECT 1 FROM snapshot_review_pins AS pin WHERE pin.scan_id = scans.scan_id
     )"
}

fn validate_witness(
    scan_id: &str,
    volume_id: Option<&str>,
    root_path: &[u8],
    root_path_encoding: i64,
    started_at_unix_ms: i64,
    root_identity_v1_sha256: Option<&[u8]>,
    completed_at_unix_ms: i64,
) -> Result<(), HistoryError> {
    let valid_root = match root_path_encoding {
        1 => (1..=32_768).contains(&root_path.len()),
        2 => (2..=65_536).contains(&root_path.len()) && root_path.len().is_multiple_of(2),
        _ => false,
    };
    if ScanId::new(scan_id.to_owned()).is_err()
        || volume_id.is_some_and(|value| value.is_empty() || value.len() > 128)
        || !valid_root
        || started_at_unix_ms < 0
        || started_at_unix_ms > completed_at_unix_ms
        || root_identity_v1_sha256.is_some_and(|digest| digest.len() != 32)
    {
        return Err(corrupt());
    }
    Ok(())
}

struct LegacyDismissalAuthorizerGuard<'a> {
    connection: &'a Connection,
    installed: bool,
}

impl LegacyDismissalAuthorizerGuard<'_> {
    fn install(
        connection: &Connection,
    ) -> Result<LegacyDismissalAuthorizerGuard<'_>, LegacyRunningScanDismissalStoreError> {
        connection
            .authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Read { .. } | AuthAction::Select | AuthAction::Function { .. } => {
                    Authorization::Allow
                }
                AuthAction::Update {
                    table_name: "scans",
                    column_name: "status" | "completed_at_unix_ms",
                } => Authorization::Allow,
                _ => Authorization::Deny,
            }))
            .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
        Ok(LegacyDismissalAuthorizerGuard {
            connection,
            installed: true,
        })
    }

    fn remove(&mut self) -> Result<(), LegacyRunningScanDismissalStoreError> {
        self.connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for LegacyDismissalAuthorizerGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self
                .connection
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}

fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

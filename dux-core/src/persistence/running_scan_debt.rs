//! Bounded, read-only census of legacy running scans without process claims.
//!
//! The result is deliberately path- and identity-free. It is diagnostic
//! evidence only and grants no recovery, cleanup, or mutation authority.

use rusqlite::Connection;

use crate::domain::ScanId;

use super::history::{HistoryError, HistoryErrorKind, map_query_sql_error, run_bounded_query};

pub(crate) const MAX_RUNNING_SCAN_DEBT_ROWS: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RunningScanDebtCensus {
    pub(crate) inspected_unclaimed_count: u16,
    pub(crate) pristine_unclaimed_count: u16,
    pub(crate) unexplained_unclaimed_count: u16,
    pub(crate) has_more: bool,
}

pub(super) const fn running_scan_debt_census_query() -> &'static str {
    "SELECT
         typeof(scan.scan_id),
         length(CAST(scan.scan_id AS BLOB)),
         scan.scan_id,
         typeof(scan.started_at_unix_ms),
         scan.started_at_unix_ms,
         CASE WHEN
             typeof(scan.status) = 'text' AND scan.status = 'running'
             AND typeof(scan.completed_at_unix_ms) = 'null'
             AND typeof(scan.directory_count) = 'integer' AND scan.directory_count >= 0
             AND typeof(scan.file_count) = 'integer' AND scan.file_count >= 0
             AND typeof(scan.logical_bytes) = 'integer' AND scan.logical_bytes >= 0
             AND (
                 typeof(scan.allocated_bytes) = 'null'
                 OR (typeof(scan.allocated_bytes) = 'integer' AND scan.allocated_bytes >= 0)
             )
             AND (
                 (
                     typeof(scan.snapshot_version) = 'null'
                     AND typeof(scan.snapshot_relative_path) = 'null'
                     AND typeof(scan.snapshot_relative_path_encoding) = 'null'
                     AND typeof(scan.snapshot_checksum_sha256) = 'null'
                 )
                 OR (
                     typeof(scan.snapshot_version) = 'integer'
                     AND scan.snapshot_version > 0
                     AND typeof(scan.snapshot_relative_path) = 'blob'
                     AND typeof(scan.snapshot_relative_path_encoding) = 'integer'
                     AND (
                         (
                             scan.snapshot_relative_path_encoding = 1
                             AND length(scan.snapshot_relative_path) BETWEEN 1 AND 32768
                         )
                         OR (
                             scan.snapshot_relative_path_encoding = 2
                             AND length(scan.snapshot_relative_path) BETWEEN 2 AND 65536
                             AND length(scan.snapshot_relative_path) % 2 = 0
                         )
                     )
                     AND typeof(scan.snapshot_checksum_sha256) = 'blob'
                     AND length(scan.snapshot_checksum_sha256) = 32
                 )
             )
             AND typeof(scan.coverage_status) = 'text'
             AND scan.coverage_status IN ('unknown', 'complete', 'limited_access', 'partial')
             AND (
                 (scan.coverage_status = 'unknown' AND typeof(scan.coverage_permille) = 'null')
                 OR (
                     scan.coverage_status = 'complete'
                     AND typeof(scan.coverage_permille) = 'integer'
                     AND scan.coverage_permille = 1000
                 )
                 OR (
                     scan.coverage_status IN ('limited_access', 'partial')
                     AND (
                         typeof(scan.coverage_permille) = 'null'
                         OR (
                             typeof(scan.coverage_permille) = 'integer'
                             AND scan.coverage_permille BETWEEN 0 AND 999
                         )
                     )
                 )
             )
             AND typeof(scan.issue_count) = 'integer' AND scan.issue_count >= 0
         THEN 1 ELSE 0 END,
         CASE WHEN
             scan.completed_at_unix_ms IS NULL
             AND scan.directory_count = 0
             AND scan.file_count = 0
             AND scan.logical_bytes = 0
             AND scan.allocated_bytes IS NULL
             AND scan.snapshot_version IS NULL
             AND scan.snapshot_relative_path IS NULL
             AND scan.snapshot_relative_path_encoding IS NULL
             AND scan.snapshot_checksum_sha256 IS NULL
             AND scan.coverage_status = 'unknown'
             AND scan.coverage_permille IS NULL
             AND scan.issue_count = 0
             AND NOT EXISTS (
                 SELECT 1 FROM scan_issues AS issue
                 WHERE issue.scan_id = scan.scan_id
             )
             AND NOT EXISTS (
                 SELECT 1 FROM scan_aggregates AS aggregate
                 WHERE aggregate.scan_id = scan.scan_id
             )
             AND NOT EXISTS (
                 SELECT 1 FROM candidates AS candidate
                 WHERE candidate.scan_id = scan.scan_id
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
                 SELECT 1 FROM snapshot_review_pins AS pin
                 WHERE pin.scan_id = scan.scan_id
             )
         THEN 1 ELSE 0 END
     FROM scans AS scan INDEXED BY scans_running_by_started
     WHERE scan.status = 'running'
       AND NOT EXISTS (
           SELECT 1 FROM scan_process_claims AS claim
           WHERE claim.scan_id = scan.scan_id
       )
     ORDER BY scan.started_at_unix_ms, scan.scan_id
     LIMIT 65"
}

pub(super) fn load_running_scan_debt_census(
    connection: &Connection,
) -> Result<RunningScanDebtCensus, HistoryError> {
    run_bounded_query(connection, || {
        let mut statement = connection
            .prepare(running_scan_debt_census_query())
            .map_err(map_query_sql_error)?;
        let mut rows = statement.query([]).map_err(map_query_sql_error)?;
        let mut pristine = 0_u16;
        let mut unexplained = 0_u16;
        let mut seen = 0_usize;

        while let Some(row) = rows.next().map_err(map_query_sql_error)? {
            let id_storage: String = row.get(0).map_err(map_query_sql_error)?;
            let id_length: i64 = row.get(1).map_err(map_query_sql_error)?;
            let id: String = row.get(2).map_err(map_query_sql_error)?;
            let started_storage: String = row.get(3).map_err(map_query_sql_error)?;
            let started_at_unix_ms: i64 = row.get(4).map_err(map_query_sql_error)?;
            let valid_shape: i64 = row.get(5).map_err(map_query_sql_error)?;
            let is_pristine: i64 = row.get(6).map_err(map_query_sql_error)?;
            if id_storage != "text"
                || !(1..=128).contains(&id_length)
                || id.len() != id_length as usize
                || ScanId::new(id).is_err()
                || started_storage != "integer"
                || started_at_unix_ms < 0
                || valid_shape != 1
                || !matches!(is_pristine, 0 | 1)
            {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            seen = seen
                .checked_add(1)
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
            if seen <= MAX_RUNNING_SCAN_DEBT_ROWS {
                if is_pristine == 1 {
                    pristine = pristine
                        .checked_add(1)
                        .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
                } else {
                    unexplained = unexplained
                        .checked_add(1)
                        .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
                }
            }
        }

        let has_more = seen > MAX_RUNNING_SCAN_DEBT_ROWS;
        let inspected = pristine
            .checked_add(unexplained)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
        if usize::from(inspected) > MAX_RUNNING_SCAN_DEBT_ROWS
            || (has_more && usize::from(inspected) != MAX_RUNNING_SCAN_DEBT_ROWS)
        {
            return Err(HistoryError::new(HistoryErrorKind::CorruptData));
        }
        Ok(RunningScanDebtCensus {
            inspected_unclaimed_count: inspected,
            pristine_unclaimed_count: pristine,
            unexplained_unclaimed_count: unexplained,
            has_more,
        })
    })
}

//! Typed schema-v2 scan coverage and issue observations.
//!
//! Stored paths are shortened relative to the scan root. Decoded paths remain
//! historical observations and never become cleanup or filesystem authority.

use std::path::{Component, Path};

use rusqlite::{Connection, Row, Transaction, params};

use crate::domain::{
    CoveragePermille, MAX_SCAN_ISSUES, ScanCoverage, ScanCoverageStatus, ScanIssue, ScanIssueKind,
};

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::history::{HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error};

const MAX_STORED_PATH_BYTES: i64 = 65_536;
const MAX_STORED_KIND_BYTES: i64 = 128;
const MAX_STORED_MESSAGE_KEY_BYTES: i64 = 128;

#[derive(Clone)]
pub(super) struct PreparedScanCoverage {
    coverage: ScanCoverage,
    issue_count: i64,
}

impl PreparedScanCoverage {
    pub(super) fn prepare(coverage: &ScanCoverage) -> Result<Self, HistoryError> {
        let issue_count = coverage.issues().iter().try_fold(0_i64, |total, issue| {
            total
                .checked_add(i64::from(issue.occurrence_count()))
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))
        })?;
        Ok(Self {
            coverage: coverage.clone(),
            issue_count,
        })
    }

    pub(super) fn status_name(&self) -> &'static str {
        coverage_status_name(self.coverage.status())
    }

    pub(super) fn measured_permille(&self) -> Option<i64> {
        self.coverage
            .measured_permille()
            .map(|value| i64::from(value.get()))
    }

    pub(super) const fn issue_count(&self) -> i64 {
        self.issue_count
    }
}

pub(super) fn insert_scan_issues(
    transaction: &Transaction<'_>,
    scan_id: &str,
    scan_root: &Path,
    coverage: &PreparedScanCoverage,
) -> Result<(), HistoryError> {
    for issue in coverage.coverage.issues() {
        let shortened = issue
            .path()
            .map(|path| shorten_issue_path(scan_root, path))
            .transpose()?;
        transaction
            .execute(
                "INSERT INTO scan_issues (
                    scan_id, shortened_path, shortened_path_encoding,
                    issue_kind, occurrence_count, message_key
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    scan_id,
                    shortened.as_ref().map(|path| path.bytes.as_slice()),
                    shortened.as_ref().map(|path| path.encoding as i64),
                    issue_kind_name(issue.kind()),
                    i64::from(issue.occurrence_count()),
                    issue.message_key().as_str(),
                ],
            )
            .map_err(map_write_sql_error)?;
    }
    Ok(())
}

fn shorten_issue_path(scan_root: &Path, path: &Path) -> Result<EncodedBytes, HistoryError> {
    let relative = path
        .strip_prefix(scan_root)
        .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    if relative.as_os_str().is_empty() {
        return encode_host_path(Path::new("."))
            .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    encode_host_path(relative).map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))
}

pub(super) fn load_scan_coverage_within_budget(
    connection: &Connection,
    scan_id: &str,
    scan_root: &Path,
    is_terminal: bool,
    stored_status: &str,
    stored_permille: Option<i64>,
    stored_issue_count: i64,
) -> Result<ScanCoverage, HistoryError> {
    if stored_issue_count < 0 {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    let mut statement = connection
        .prepare(
            "SELECT typeof(issue_id), issue_id,
                    typeof(shortened_path), length(shortened_path), shortened_path,
                    typeof(shortened_path_encoding), shortened_path_encoding,
                    typeof(issue_kind), length(CAST(issue_kind AS BLOB)), issue_kind,
                    typeof(occurrence_count), occurrence_count,
                    typeof(message_key), length(CAST(message_key AS BLOB)), message_key
             FROM scan_issues
             WHERE scan_id = ?1
             ORDER BY issue_id
             LIMIT ?2",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![
            scan_id,
            i64::try_from(MAX_SCAN_ISSUES + 1).unwrap()
        ])
        .map_err(map_query_sql_error)?;
    let mut issues = Vec::new();
    let mut issue_count = 0_i64;
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if issues.len() == MAX_SCAN_ISSUES {
            return Err(HistoryError::new(HistoryErrorKind::CorruptData));
        }
        let raw = raw_scan_issue(row).map_err(map_query_sql_error)?;
        let issue = decode_scan_issue(scan_root, raw)?;
        issue_count = issue_count
            .checked_add(i64::from(issue.occurrence_count()))
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
        issues.push(issue);
    }
    if issue_count != stored_issue_count {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    let status = coverage_status_from_name(stored_status)?;
    if !is_terminal
        && (status != ScanCoverageStatus::Unknown || !issues.is_empty() || stored_issue_count != 0)
    {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    let measured_permille = stored_permille
        .map(|value| {
            u16::try_from(value)
                .ok()
                .and_then(|value| CoveragePermille::new(value).ok())
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))
        })
        .transpose()?;
    ScanCoverage::try_new(status, measured_permille, issues)
        .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))
}

struct RawScanIssue {
    issue_id: i64,
    shortened_path: Option<Vec<u8>>,
    shortened_path_encoding: Option<i64>,
    kind: String,
    occurrence_count: i64,
    message_key: String,
}

fn raw_scan_issue(row: &Row<'_>) -> rusqlite::Result<RawScanIssue> {
    let issue_id_type: String = row.get(0)?;
    let issue_id: i64 = row.get(1)?;
    if issue_id_type != "integer" || issue_id <= 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let path_type: String = row.get(2)?;
    let path_encoding_type: String = row.get(5)?;
    let (shortened_path, shortened_path_encoding) =
        if path_type == "null" && path_encoding_type == "null" {
            (None, None)
        } else if path_type == "blob" && path_encoding_type == "integer" {
            let length: i64 = row.get(3)?;
            if !(1..=MAX_STORED_PATH_BYTES).contains(&length) {
                return Err(rusqlite::Error::InvalidQuery);
            }
            (Some(row.get(4)?), Some(row.get(6)?))
        } else {
            return Err(rusqlite::Error::InvalidQuery);
        };
    validate_text(row, 7, 8, MAX_STORED_KIND_BYTES)?;
    let occurrence_type: String = row.get(10)?;
    if occurrence_type != "integer" {
        return Err(rusqlite::Error::InvalidQuery);
    }
    validate_text(row, 12, 13, MAX_STORED_MESSAGE_KEY_BYTES)?;
    Ok(RawScanIssue {
        issue_id,
        shortened_path,
        shortened_path_encoding,
        kind: row.get(9)?,
        occurrence_count: row.get(11)?,
        message_key: row.get(14)?,
    })
}

fn validate_text(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
    maximum_length: i64,
) -> rusqlite::Result<()> {
    let storage_type: String = row.get(type_column)?;
    let length: i64 = row.get(length_column)?;
    if storage_type != "text" || !(1..=maximum_length).contains(&length) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn decode_scan_issue(scan_root: &Path, raw: RawScanIssue) -> Result<ScanIssue, HistoryError> {
    let _issue_id = raw.issue_id;
    let path = match (raw.shortened_path, raw.shortened_path_encoding) {
        (None, None) => None,
        (Some(bytes), Some(encoding)) => {
            let relative = decode_host_path(&EncodedBytes {
                bytes,
                encoding: StoredEncoding::host_path_from_stored(encoding)
                    .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?,
            })
            .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
            let observed = if relative == Path::new(".") {
                scan_root.to_path_buf()
            } else {
                if relative.is_absolute()
                    || !relative
                        .components()
                        .all(|component| matches!(component, Component::Normal(_)))
                {
                    return Err(HistoryError::new(HistoryErrorKind::CorruptData));
                }
                scan_root.join(relative)
            };
            Some(observed)
        }
        _ => return Err(HistoryError::new(HistoryErrorKind::CorruptData)),
    };
    let occurrence_count = u32::try_from(raw.occurrence_count)
        .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
    let issue = ScanIssue::try_new(issue_kind_from_name(&raw.kind)?, path, occurrence_count)
        .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
    if issue.message_key().as_str() != raw.message_key {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    Ok(issue)
}

pub(super) const fn coverage_status_name(status: ScanCoverageStatus) -> &'static str {
    match status {
        ScanCoverageStatus::Unknown => "unknown",
        ScanCoverageStatus::Complete => "complete",
        ScanCoverageStatus::LimitedAccess => "limited_access",
        ScanCoverageStatus::Partial => "partial",
    }
}

fn coverage_status_from_name(value: &str) -> Result<ScanCoverageStatus, HistoryError> {
    match value {
        "unknown" => Ok(ScanCoverageStatus::Unknown),
        "complete" => Ok(ScanCoverageStatus::Complete),
        "limited_access" => Ok(ScanCoverageStatus::LimitedAccess),
        "partial" => Ok(ScanCoverageStatus::Partial),
        _ => Err(HistoryError::new(HistoryErrorKind::CorruptData)),
    }
}

const fn issue_kind_name(kind: ScanIssueKind) -> &'static str {
    match kind {
        ScanIssueKind::PermissionDenied => "permission_denied",
        ScanIssueKind::TimedOut => "timed_out",
        ScanIssueKind::DifferentFilesystem => "different_filesystem",
        ScanIssueKind::NetworkOrVirtualFilesystem => "network_or_virtual_filesystem",
        ScanIssueKind::SymlinkSkipped => "symlink_skipped",
        ScanIssueKind::FileChangedDuringScan => "file_changed_during_scan",
        ScanIssueKind::MetadataError => "metadata_error",
        ScanIssueKind::Cancelled => "cancelled",
        ScanIssueKind::PolicyExcluded => "policy_excluded",
        ScanIssueKind::DepthLimited => "depth_limited",
        ScanIssueKind::ProbePoolExhausted => "probe_pool_exhausted",
        ScanIssueKind::FilesystemBoundaryUnknown => "filesystem_boundary_unknown",
        ScanIssueKind::IssueLimitReached => "issue_limit_reached",
    }
}

fn issue_kind_from_name(value: &str) -> Result<ScanIssueKind, HistoryError> {
    match value {
        "permission_denied" => Ok(ScanIssueKind::PermissionDenied),
        "timed_out" => Ok(ScanIssueKind::TimedOut),
        "different_filesystem" => Ok(ScanIssueKind::DifferentFilesystem),
        "network_or_virtual_filesystem" => Ok(ScanIssueKind::NetworkOrVirtualFilesystem),
        "symlink_skipped" => Ok(ScanIssueKind::SymlinkSkipped),
        "file_changed_during_scan" => Ok(ScanIssueKind::FileChangedDuringScan),
        "metadata_error" => Ok(ScanIssueKind::MetadataError),
        "cancelled" => Ok(ScanIssueKind::Cancelled),
        "policy_excluded" => Ok(ScanIssueKind::PolicyExcluded),
        "depth_limited" => Ok(ScanIssueKind::DepthLimited),
        "probe_pool_exhausted" => Ok(ScanIssueKind::ProbePoolExhausted),
        "filesystem_boundary_unknown" => Ok(ScanIssueKind::FilesystemBoundaryUnknown),
        "issue_limit_reached" => Ok(ScanIssueKind::IssueLimitReached),
        _ => Err(HistoryError::new(HistoryErrorKind::CorruptData)),
    }
}

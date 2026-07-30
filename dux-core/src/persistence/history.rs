//! Typed scan-history records stored in the v1 SQLite schema.
//!
//! These values are historical observations for presentation and comparison.
//! A decoded path is never a path-validation witness or cleanup authority.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use sha2::{Digest, Sha256};

use crate::domain::{ScanCoverage, ScanCoverageStatus, ScanId, ScanIssueKind};
use crate::path_validation::FilesystemIdentity;

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::scan_coverage_history::{
    PreparedScanCoverage, insert_scan_issues, load_scan_coverage_within_budget,
};
use super::snapshot::SnapshotReference;

const QUERY_PROGRESS_INTERVAL: i32 = 100;
const QUERY_MAX_CALLBACKS: u64 = 1_000;
const QUERY_MAX_ELAPSED: Duration = Duration::from_millis(250);
const RECENT_QUERY_BASE_CALLBACKS: u64 = 1_000;
const RECENT_QUERY_CALLBACKS_PER_SCAN: u64 = 400;
const RECENT_QUERY_MAX_ELAPSED: Duration = Duration::from_secs(5);
const MAX_STORED_ID_BYTES: i64 = 128;
const MAX_STORED_PATH_BYTES: i64 = 65_536;
const MAX_STORED_STATUS_BYTES: i64 = 16;
pub(crate) const MAX_RECENT_SCAN_HISTORY_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

impl ScanStatus {
    fn as_stored(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    fn from_stored(value: &str) -> Result<Self, HistoryError> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(HistoryError::new(HistoryErrorKind::CorruptData)),
        }
    }

    const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalScanStatus {
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

impl From<TerminalScanStatus> for ScanStatus {
    fn from(value: TerminalScanStatus) -> Self {
        match value {
            TerminalScanStatus::Succeeded => Self::Succeeded,
            TerminalScanStatus::Failed => Self::Failed,
            TerminalScanStatus::Cancelled => Self::Cancelled,
            TerminalScanStatus::Interrupted => Self::Interrupted,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScanCounts {
    pub(crate) directory_count: u64,
    pub(crate) file_count: u64,
    pub(crate) logical_bytes: u64,
    pub(crate) allocated_bytes: Option<u64>,
}

impl ScanCounts {
    /// Validate the frozen counts against SQLite's signed integer domain before
    /// any snapshot bytes are staged or published.
    pub(crate) fn validate_for_storage(self) -> Result<(), HistoryError> {
        validate_counts(self, HistoryErrorKind::InvalidInput)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NewScanRecord {
    id: ScanId,
    root: PathBuf,
    started_at: SystemTime,
    root_identity_v1_sha256: Option<[u8; 32]>,
}

impl NewScanRecord {
    pub(crate) fn try_new_with_root_identity(
        id: ScanId,
        root: PathBuf,
        started_at: SystemTime,
        root_identity: FilesystemIdentity,
    ) -> Result<Self, HistoryError> {
        Self::try_new_inner(
            id,
            root,
            started_at,
            Some(root_identity_v1_sha256(root_identity)),
        )
    }

    #[cfg(test)]
    pub(crate) fn try_new_without_root_identity(
        id: ScanId,
        root: PathBuf,
        started_at: SystemTime,
    ) -> Result<Self, HistoryError> {
        Self::try_new_inner(id, root, started_at, None)
    }

    fn try_new_inner(
        id: ScanId,
        root: PathBuf,
        started_at: SystemTime,
        root_identity_v1_sha256: Option<[u8; 32]>,
    ) -> Result<Self, HistoryError> {
        if !root.is_absolute() {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        encode_host_path(&root).map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        let started_at = unix_ms_to_system_time(system_time_to_unix_ms(
            started_at,
            HistoryErrorKind::InvalidInput,
        )?)?;
        Ok(Self {
            id,
            root,
            started_at,
            root_identity_v1_sha256,
        })
    }

    pub(crate) fn id(&self) -> &ScanId {
        &self.id
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn started_at(&self) -> SystemTime {
        self.started_at
    }

    #[cfg(test)]
    pub(crate) const fn root_identity_v1_sha256(&self) -> Option<[u8; 32]> {
        self.root_identity_v1_sha256
    }
}

fn root_identity_v1_sha256(identity: FilesystemIdentity) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"dux.scan.root-identity.v1\0");
    digest.update(identity.volume().to_le_bytes());
    digest.update(identity.object().to_le_bytes());
    digest.finalize().into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanCompletionRecord {
    id: ScanId,
    completed_at: SystemTime,
    status: TerminalScanStatus,
    counts: ScanCounts,
    coverage: ScanCoverage,
    snapshot: Option<SnapshotReference>,
}

impl ScanCompletionRecord {
    pub(crate) fn try_new(
        id: ScanId,
        completed_at: SystemTime,
        status: TerminalScanStatus,
        counts: ScanCounts,
    ) -> Result<Self, HistoryError> {
        let completed_at = unix_ms_to_system_time(system_time_to_unix_ms(
            completed_at,
            HistoryErrorKind::InvalidInput,
        )?)?;
        counts.validate_for_storage()?;
        Ok(Self {
            id,
            completed_at,
            status,
            counts,
            coverage: ScanCoverage::unknown(),
            snapshot: None,
        })
    }

    pub(crate) fn try_new_with_coverage(
        id: ScanId,
        completed_at: SystemTime,
        status: TerminalScanStatus,
        counts: ScanCounts,
        coverage: ScanCoverage,
    ) -> Result<Self, HistoryError> {
        let mut record = Self::try_new(id, completed_at, status, counts)?;
        record.coverage = coverage;
        Ok(record)
    }

    pub(crate) fn try_succeeded_with_snapshot(
        id: ScanId,
        completed_at: SystemTime,
        counts: ScanCounts,
        snapshot: SnapshotReference,
    ) -> Result<Self, HistoryError> {
        if snapshot.scan_id() != &id {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let completed_at = unix_ms_to_system_time(system_time_to_unix_ms(
            completed_at,
            HistoryErrorKind::InvalidInput,
        )?)?;
        counts.validate_for_storage()?;
        Ok(Self {
            id,
            completed_at,
            status: TerminalScanStatus::Succeeded,
            counts,
            coverage: ScanCoverage::unknown(),
            snapshot: Some(snapshot),
        })
    }

    pub(crate) fn try_succeeded_with_snapshot_and_coverage(
        id: ScanId,
        completed_at: SystemTime,
        counts: ScanCounts,
        coverage: ScanCoverage,
        snapshot: SnapshotReference,
    ) -> Result<Self, HistoryError> {
        let mut record = Self::try_succeeded_with_snapshot(id, completed_at, counts, snapshot)?;
        record.coverage = coverage;
        Ok(record)
    }

    pub(crate) fn id(&self) -> &ScanId {
        &self.id
    }

    #[cfg(test)]
    pub(crate) fn completed_at(&self) -> SystemTime {
        self.completed_at
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> ScanCounts {
        self.counts
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanRecord {
    id: ScanId,
    root: PathBuf,
    started_at: SystemTime,
    completed_at: Option<SystemTime>,
    status: ScanStatus,
    counts: ScanCounts,
    coverage: ScanCoverage,
    snapshot: Option<SnapshotReference>,
    root_identity_v1_sha256: Option<[u8; 32]>,
}

impl ScanRecord {
    pub(crate) fn id(&self) -> &ScanId {
        &self.id
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn started_at(&self) -> SystemTime {
        self.started_at
    }

    pub(crate) fn completed_at(&self) -> Option<SystemTime> {
        self.completed_at
    }

    pub(crate) fn status(&self) -> ScanStatus {
        self.status
    }

    pub(crate) fn counts(&self) -> ScanCounts {
        self.counts
    }

    pub(crate) fn coverage(&self) -> &ScanCoverage {
        &self.coverage
    }

    pub(crate) fn snapshot(&self) -> Option<&SnapshotReference> {
        self.snapshot.as_ref()
    }

    #[allow(
        dead_code,
        reason = "the persisted identity is consumed by a later exact-root outcome query"
    )]
    pub(crate) const fn root_identity_v1_sha256(&self) -> Option<[u8; 32]> {
        self.root_identity_v1_sha256
    }

    pub(crate) fn exactly_matches_start(&self, start: &NewScanRecord) -> bool {
        self.id == start.id
            && self.root == start.root
            && self.started_at == start.started_at
            && self.completed_at.is_none()
            && self.status == ScanStatus::Running
            && self.counts == ScanCounts::default()
            && self.coverage.status() == ScanCoverageStatus::Unknown
            && self.snapshot.is_none()
            && self.root_identity_v1_sha256 == start.root_identity_v1_sha256
    }

    pub(crate) fn exactly_matches_completion(&self, completion: &ScanCompletionRecord) -> bool {
        self.id == completion.id
            && self.completed_at == Some(completion.completed_at)
            && self.status == ScanStatus::from(completion.status)
            && self.counts == completion.counts
            && self.coverage == completion.coverage
            && self.snapshot == completion.snapshot
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecentScanRecords {
    records: Vec<ScanRecord>,
    has_more: bool,
}

impl RecentScanRecords {
    pub(crate) fn records(&self) -> &[ScanRecord] {
        &self.records
    }

    pub(crate) const fn has_more(&self) -> bool {
        self.has_more
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryErrorKind {
    InvalidInput,
    AlreadyExists,
    NotFound,
    InvalidTransition,
    IncompatibleSchema,
    QueryLimitExceeded,
    Busy,
    UnsafeStorage,
    CorruptData,
    DatabaseUnavailable,
    InternalState,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("history operation failed: {kind:?}")]
pub(crate) struct HistoryError {
    pub(crate) kind: HistoryErrorKind,
}

impl HistoryError {
    pub(crate) const fn new(kind: HistoryErrorKind) -> Self {
        Self { kind }
    }
}

pub(super) struct PreparedNewScan {
    id: String,
    root: EncodedBytes,
    started_at_unix_ms: i64,
    root_identity_v1_sha256: Option<[u8; 32]>,
}

impl PreparedNewScan {
    pub(super) fn prepare(scan: &NewScanRecord) -> Result<Self, HistoryError> {
        Ok(Self {
            id: scan.id.as_str().to_owned(),
            root: encode_host_path(&scan.root)
                .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?,
            started_at_unix_ms: system_time_to_unix_ms(
                scan.started_at,
                HistoryErrorKind::InvalidInput,
            )?,
            root_identity_v1_sha256: scan.root_identity_v1_sha256,
        })
    }
}

pub(super) struct PreparedScanCompletion {
    id: String,
    completed_at_unix_ms: i64,
    status: ScanStatus,
    counts: ScanCounts,
    coverage: PreparedScanCoverage,
    snapshot: Option<PreparedSnapshotReference>,
}

struct PreparedSnapshotReference {
    version: i64,
    relative_path: EncodedBytes,
    digest: [u8; 32],
}

impl PreparedScanCompletion {
    pub(super) fn prepare(completion: &ScanCompletionRecord) -> Result<Self, HistoryError> {
        validate_counts(completion.counts, HistoryErrorKind::InvalidInput)?;
        validate_terminal_coverage(
            completion.status,
            &completion.coverage,
            HistoryErrorKind::InvalidInput,
        )?;
        if completion.snapshot.is_some() && completion.status != TerminalScanStatus::Succeeded {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let snapshot = completion
            .snapshot
            .as_ref()
            .map(|reference| {
                if reference.scan_id() != &completion.id {
                    return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
                }
                Ok(PreparedSnapshotReference {
                    version: i64::from(reference.version()),
                    relative_path: encode_host_path(Path::new(reference.file_name().as_str()))
                        .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?,
                    digest: reference.digest().bytes(),
                })
            })
            .transpose()?;
        Ok(Self {
            id: completion.id.as_str().to_owned(),
            completed_at_unix_ms: system_time_to_unix_ms(
                completion.completed_at,
                HistoryErrorKind::InvalidInput,
            )?,
            status: completion.status.into(),
            counts: completion.counts,
            coverage: PreparedScanCoverage::prepare(&completion.coverage)?,
            snapshot,
        })
    }
}

pub(super) fn insert_scan_started(
    transaction: &Transaction<'_>,
    scan: &PreparedNewScan,
) -> Result<(), HistoryError> {
    let changed = transaction
        .execute(
            "INSERT INTO scans (
                scan_id, root_path, root_path_encoding, started_at_unix_ms,
                status, coverage_status, root_identity_v1_sha256
             ) VALUES (?1, ?2, ?3, ?4, 'running', 'unknown', ?5)
             ON CONFLICT(scan_id) DO NOTHING",
            params![
                scan.id,
                scan.root.bytes,
                scan.root.encoding as i64,
                scan.started_at_unix_ms,
                scan.root_identity_v1_sha256
                    .as_ref()
                    .map(|digest| digest.as_slice()),
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
    }
    Ok(())
}

pub(super) fn update_scan_finished(
    transaction: &Transaction<'_>,
    completion: &PreparedScanCompletion,
) -> Result<(), HistoryError> {
    let changed = transaction
        .execute(
            "UPDATE scans
             SET completed_at_unix_ms = ?2, status = ?3, directory_count = ?4,
                 file_count = ?5, logical_bytes = ?6, allocated_bytes = ?7,
                 snapshot_version = ?8, snapshot_relative_path = ?9,
                 snapshot_relative_path_encoding = ?10,
                 snapshot_checksum_sha256 = ?11, coverage_status = ?12,
                 coverage_permille = ?13, issue_count = ?14
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
            params![
                completion.id,
                completion.completed_at_unix_ms,
                completion.status.as_stored(),
                to_i64(
                    completion.counts.directory_count,
                    HistoryErrorKind::InvalidInput
                )?,
                to_i64(completion.counts.file_count, HistoryErrorKind::InvalidInput)?,
                to_i64(
                    completion.counts.logical_bytes,
                    HistoryErrorKind::InvalidInput
                )?,
                completion
                    .counts
                    .allocated_bytes
                    .map(|value| to_i64(value, HistoryErrorKind::InvalidInput))
                    .transpose()?,
                completion
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.version),
                completion
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.relative_path.bytes.as_slice()),
                completion
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.relative_path.encoding as i64),
                completion
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.digest.as_slice()),
                completion.coverage.status_name(),
                completion.coverage.measured_permille(),
                completion.coverage.issue_count(),
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed == 1 {
        let root = load_scan_root_for_write(transaction, &completion.id)?;
        insert_scan_issues(transaction, &completion.id, &root, &completion.coverage)?;
        return Ok(());
    }

    let current = transaction
        .query_row(
            "SELECT started_at_unix_ms FROM scans WHERE scan_id = ?1",
            [&completion.id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    match current {
        None => Err(HistoryError::new(HistoryErrorKind::NotFound)),
        Some(started_at) if completion.completed_at_unix_ms < started_at => {
            Err(HistoryError::new(HistoryErrorKind::InvalidInput))
        }
        Some(_) => Err(HistoryError::new(HistoryErrorKind::InvalidTransition)),
    }
}

fn load_scan_root_for_write(
    transaction: &Transaction<'_>,
    scan_id: &str,
) -> Result<PathBuf, HistoryError> {
    let raw = transaction
        .query_row(
            "SELECT typeof(root_path), length(root_path), root_path, root_path_encoding
             FROM scans WHERE scan_id = ?1",
            [scan_id],
            |row| {
                validate_stored_value(row, 0, 1, "blob", 1, MAX_STORED_PATH_BYTES)?;
                Ok((row.get::<_, Vec<u8>>(2)?, row.get::<_, i64>(3)?))
            },
        )
        .map_err(map_query_sql_error)?;
    let root = decode_host_path(&EncodedBytes {
        bytes: raw.0,
        encoding: stored_host_encoding(raw.1)?,
    })
    .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
    if !root.is_absolute() {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    Ok(root)
}

pub(super) fn load_scan_record(
    connection: &Connection,
    id: &ScanId,
) -> Result<Option<ScanRecord>, HistoryError> {
    run_bounded_query(connection, || {
        load_scan_record_within_budget(connection, id)
    })
}

pub(super) fn load_scan_record_within_budget(
    connection: &Connection,
    id: &ScanId,
) -> Result<Option<ScanRecord>, HistoryError> {
    let raw = connection
        .query_row(
            "SELECT typeof(scan_id), length(CAST(scan_id AS BLOB)), scan_id,
                    typeof(root_path), length(root_path), root_path,
                    root_path_encoding, started_at_unix_ms, completed_at_unix_ms,
                    typeof(status), length(CAST(status AS BLOB)), status,
                    directory_count, file_count, logical_bytes, allocated_bytes,
                    typeof(coverage_status), length(CAST(coverage_status AS BLOB)),
                    coverage_status, typeof(coverage_permille), coverage_permille,
                    typeof(issue_count), issue_count,
                    typeof(snapshot_version), snapshot_version,
                    typeof(snapshot_relative_path), length(snapshot_relative_path),
                    snapshot_relative_path,
                    typeof(snapshot_relative_path_encoding), snapshot_relative_path_encoding,
                    typeof(snapshot_checksum_sha256), length(snapshot_checksum_sha256),
                    snapshot_checksum_sha256,
                    typeof(root_identity_v1_sha256), length(root_identity_v1_sha256),
                    root_identity_v1_sha256
             FROM scans WHERE scan_id = ?1",
            [id.as_str()],
            raw_scan_row,
        )
        .optional()
        .map_err(map_query_sql_error)?;
    raw.map(|raw| decode_scan_row(connection, raw)).transpose()
}

/// Load the newest fully validated succeeded snapshot scan for one exact
/// host-path byte sequence at or after a canonical millisecond boundary.
/// Failed, running, or snapshotless newer rows cannot mask an older qualifying
/// observation. Selection is bounded to one stable ID; the complete parent and
/// coverage graph is then decoded under the same fixed query budget.
pub(super) fn load_latest_scan_record_for_exact_root_since(
    connection: &Connection,
    root: &Path,
    started_at_or_after: SystemTime,
) -> Result<Option<ScanRecord>, HistoryError> {
    if !root.is_absolute() {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    let encoded_root =
        encode_host_path(root).map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    let started_at_unix_ms =
        system_time_to_unix_ms(started_at_or_after, HistoryErrorKind::InvalidInput)?;
    run_bounded_query(connection, || {
        let raw_id = connection
            .query_row(
                "SELECT typeof(scan_id), length(CAST(scan_id AS BLOB)), scan_id
                 FROM scans
                 WHERE root_path = ?1
                   AND root_path_encoding = ?2
                   AND started_at_unix_ms >= ?3
                   AND scan_id GLOB 'scan:targeted:*'
                   AND status = 'succeeded'
                   AND snapshot_version IS NOT NULL
                   AND snapshot_relative_path IS NOT NULL
                   AND snapshot_relative_path_encoding IS NOT NULL
                   AND snapshot_checksum_sha256 IS NOT NULL
                   AND EXISTS (
                     SELECT 1 FROM candidate_evaluations AS evaluation
                     WHERE evaluation.scan_id = scans.scan_id
                       AND evaluation.status IN ('succeeded', 'failed')
                   )
                 ORDER BY started_at_unix_ms DESC, scan_id ASC
                 LIMIT 1",
                params![
                    encoded_root.bytes,
                    encoded_root.encoding as i64,
                    started_at_unix_ms
                ],
                |row| {
                    validate_stored_value(row, 0, 1, "text", 1, MAX_STORED_ID_BYTES)?;
                    row.get::<_, String>(2)
                },
            )
            .optional()
            .map_err(map_query_sql_error)?;
        let Some(raw_id) = raw_id else {
            return Ok(None);
        };
        let id =
            ScanId::new(raw_id).map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
        let record = load_scan_record_within_budget(connection, &id)?
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
        if record.root() != root
            || record.started_at() < started_at_or_after
            || record.status() != ScanStatus::Succeeded
            || record.snapshot().is_none()
        {
            return Err(HistoryError::new(HistoryErrorKind::CorruptData));
        }
        Ok(Some(record))
    })
}

/// Load one bounded recent-history page. The parent rows are selected in one
/// query, then each selected raw row is passed through the same full decoder as
/// an exact-ID read. Coverage child queries therefore share one VM/time budget.
/// The dedicated budget is sized from the declared page bound and the legal
/// 256-issue maximum per scan; the composition can decode no more than 200
/// parents and 51,200 children rather than exposing a generic unbounded N+1
/// surface.
pub(super) fn load_recent_scan_records(
    connection: &Connection,
    limit: usize,
) -> Result<RecentScanRecords, HistoryError> {
    if !(1..=MAX_RECENT_SCAN_HISTORY_LIMIT).contains(&limit) {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    let row_limit =
        i64::try_from(limit + 1).map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    run_bounded_recent_query(connection, limit, || {
        let mut statement = connection
            .prepare(
                "SELECT typeof(scan_id), length(CAST(scan_id AS BLOB)), scan_id,
                        typeof(root_path), length(root_path), root_path,
                        root_path_encoding, started_at_unix_ms, completed_at_unix_ms,
                        typeof(status), length(CAST(status AS BLOB)), status,
                        directory_count, file_count, logical_bytes, allocated_bytes,
                        typeof(coverage_status), length(CAST(coverage_status AS BLOB)),
                        coverage_status, typeof(coverage_permille), coverage_permille,
                        typeof(issue_count), issue_count,
                        typeof(snapshot_version), snapshot_version,
                        typeof(snapshot_relative_path), length(snapshot_relative_path),
                        snapshot_relative_path,
                        typeof(snapshot_relative_path_encoding), snapshot_relative_path_encoding,
                        typeof(snapshot_checksum_sha256), length(snapshot_checksum_sha256),
                        snapshot_checksum_sha256,
                        typeof(root_identity_v1_sha256), length(root_identity_v1_sha256),
                        root_identity_v1_sha256
                 FROM scans
                 WHERE scan_id NOT GLOB 'scan:targeted:*'
                 ORDER BY started_at_unix_ms DESC, scan_id ASC
                 LIMIT ?1",
            )
            .map_err(map_query_sql_error)?;
        let mut rows = statement.query([row_limit]).map_err(map_query_sql_error)?;
        let mut raw_records = Vec::with_capacity(limit + 1);
        while let Some(row) = rows.next().map_err(map_query_sql_error)? {
            if raw_records.len() > limit {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            raw_records.push(raw_scan_row(row).map_err(map_query_sql_error)?);
        }
        drop(rows);
        drop(statement);

        let has_more = raw_records.len() > limit;
        raw_records.truncate(limit);
        let records = raw_records
            .into_iter()
            .map(|raw| decode_scan_row(connection, raw))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RecentScanRecords { records, has_more })
    })
}

/// Load the newest succeeded scan whose snapshot has not been tombstoned.
/// Selection is bounded to one stable ID; the selected row still passes the
/// complete scan decoder, and repository lease acquisition remains the final
/// snapshot availability authority.
pub(super) fn load_latest_available_snapshot_scan_record(
    connection: &Connection,
) -> Result<Option<ScanRecord>, HistoryError> {
    run_bounded_query(connection, || {
        let raw_id = connection
            .query_row(
                "SELECT typeof(scan.scan_id),
                        length(CAST(scan.scan_id AS BLOB)), scan.scan_id
                 FROM scans AS scan
                 LEFT JOIN snapshot_retention_tombstones AS tombstone
                   ON tombstone.scan_id = scan.scan_id
                 WHERE scan.status = 'succeeded'
                   AND scan.scan_id NOT GLOB 'scan:targeted:*'
                   AND tombstone.scan_id IS NULL
                   AND (
                     scan.snapshot_version IS NOT NULL OR
                     scan.snapshot_relative_path IS NOT NULL OR
                     scan.snapshot_relative_path_encoding IS NOT NULL OR
                     scan.snapshot_checksum_sha256 IS NOT NULL
                   )
                 ORDER BY scan.completed_at_unix_ms DESC,
                          scan.started_at_unix_ms DESC,
                          scan.scan_id ASC
                 LIMIT 1",
                [],
                |row| {
                    validate_stored_value(row, 0, 1, "text", 1, MAX_STORED_ID_BYTES)?;
                    row.get::<_, String>(2)
                },
            )
            .optional()
            .map_err(map_query_sql_error)?;
        let Some(raw_id) = raw_id else {
            return Ok(None);
        };
        let id =
            ScanId::new(raw_id).map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
        let record = load_scan_record_within_budget(connection, &id)?
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
        if record.status() != ScanStatus::Succeeded || record.snapshot().is_none() {
            return Err(HistoryError::new(HistoryErrorKind::CorruptData));
        }
        Ok(Some(record))
    })
}

/// Load the immediately preceding available snapshot for the current scan's
/// exact lossless root and non-legacy root identity.
///
/// Ordering deliberately matches snapshot retention's per-root latest-two
/// policy: completion descending, start descending, then scan ID ascending.
/// The returned row is selection evidence only; repository lease acquisition
/// repeats the complete durable and physical snapshot validation.
pub(super) fn load_previous_comparable_snapshot_scan_record(
    connection: &Connection,
    current: &ScanRecord,
) -> Result<Option<ScanRecord>, HistoryError> {
    if current.status() != ScanStatus::Succeeded || current.snapshot().is_none() {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    let current_completed_at = current
        .completed_at()
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
    let current_identity = current
        .root_identity_v1_sha256()
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    let encoded_root = encode_host_path(current.root())
        .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
    let current_completed_at_unix_ms =
        system_time_to_unix_ms(current_completed_at, HistoryErrorKind::CorruptData)?;
    let current_started_at_unix_ms =
        system_time_to_unix_ms(current.started_at(), HistoryErrorKind::CorruptData)?;

    run_bounded_query(connection, || {
        let raw_id = connection
            .query_row(
                "SELECT typeof(scan.scan_id),
                        length(CAST(scan.scan_id AS BLOB)), scan.scan_id
                 FROM scans AS scan
                 LEFT JOIN snapshot_retention_tombstones AS tombstone
                   ON tombstone.scan_id = scan.scan_id
                 WHERE scan.status = 'succeeded'
                   AND scan.scan_id NOT GLOB 'scan:targeted:*'
                   AND tombstone.scan_id IS NULL
                   AND scan.root_path_encoding = ?1
                   AND scan.root_path = ?2
                   AND scan.root_identity_v1_sha256 = ?3
                   AND scan.completed_at_unix_ms IS NOT NULL
                   AND (
                     scan.completed_at_unix_ms < ?4 OR
                     (
                       scan.completed_at_unix_ms = ?4 AND
                       scan.started_at_unix_ms < ?5
                     ) OR
                     (
                       scan.completed_at_unix_ms = ?4 AND
                       scan.started_at_unix_ms = ?5 AND
                       scan.scan_id > ?6
                     )
                   )
                   AND (
                     scan.snapshot_version IS NOT NULL OR
                     scan.snapshot_relative_path IS NOT NULL OR
                     scan.snapshot_relative_path_encoding IS NOT NULL OR
                     scan.snapshot_checksum_sha256 IS NOT NULL
                   )
                 ORDER BY scan.completed_at_unix_ms DESC,
                          scan.started_at_unix_ms DESC,
                          scan.scan_id ASC
                 LIMIT 1",
                params![
                    encoded_root.encoding as i64,
                    encoded_root.bytes,
                    current_identity.as_slice(),
                    current_completed_at_unix_ms,
                    current_started_at_unix_ms,
                    current.id().as_str(),
                ],
                |row| {
                    validate_stored_value(row, 0, 1, "text", 1, MAX_STORED_ID_BYTES)?;
                    row.get::<_, String>(2)
                },
            )
            .optional()
            .map_err(map_query_sql_error)?;
        let Some(raw_id) = raw_id else {
            return Ok(None);
        };
        let id =
            ScanId::new(raw_id).map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
        let record = load_scan_record_within_budget(connection, &id)?
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
        if record.status() != ScanStatus::Succeeded
            || record.snapshot().is_none()
            || record.completed_at().is_none()
            || record.root() != current.root()
            || record.root_identity_v1_sha256() != Some(current_identity)
        {
            return Err(HistoryError::new(HistoryErrorKind::CorruptData));
        }
        Ok(Some(record))
    })
}

struct RawScanRow {
    id: String,
    root: Vec<u8>,
    root_encoding: i64,
    started_at_unix_ms: i64,
    completed_at_unix_ms: Option<i64>,
    status: String,
    directory_count: i64,
    file_count: i64,
    logical_bytes: i64,
    allocated_bytes: Option<i64>,
    coverage_status: String,
    coverage_permille: Option<i64>,
    issue_count: i64,
    snapshot: Option<RawSnapshotReference>,
    root_identity_v1_sha256: Option<Vec<u8>>,
}

struct RawSnapshotReference {
    version: i64,
    relative_path: Vec<u8>,
    relative_path_encoding: i64,
    digest: Vec<u8>,
}

fn raw_scan_row(row: &Row<'_>) -> rusqlite::Result<RawScanRow> {
    validate_stored_value(row, 0, 1, "text", 1, MAX_STORED_ID_BYTES)?;
    validate_stored_value(row, 3, 4, "blob", 1, MAX_STORED_PATH_BYTES)?;
    validate_stored_value(row, 9, 10, "text", 1, MAX_STORED_STATUS_BYTES)?;
    validate_stored_value(row, 16, 17, "text", 1, MAX_STORED_STATUS_BYTES)?;
    let coverage_permille_type: String = row.get(19)?;
    if !matches!(coverage_permille_type.as_str(), "null" | "integer") {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let issue_count_type: String = row.get(21)?;
    if issue_count_type != "integer" {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let snapshot_types = [
        row.get::<_, String>(23)?,
        row.get::<_, String>(25)?,
        row.get::<_, String>(28)?,
        row.get::<_, String>(30)?,
    ];
    let snapshot = if snapshot_types.iter().all(|value| value == "null") {
        None
    } else if snapshot_types == ["integer", "blob", "integer", "blob"] {
        let relative_path_length: i64 = row.get(26)?;
        let digest_length: i64 = row.get(31)?;
        if !(1..=MAX_STORED_PATH_BYTES).contains(&relative_path_length) || digest_length != 32 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Some(RawSnapshotReference {
            version: row.get(24)?,
            relative_path: row.get(27)?,
            relative_path_encoding: row.get(29)?,
            digest: row.get(32)?,
        })
    } else {
        return Err(rusqlite::Error::InvalidQuery);
    };
    let root_identity_type: String = row.get(33)?;
    let root_identity_length: Option<i64> = row.get(34)?;
    let root_identity_v1_sha256 = match (root_identity_type.as_str(), root_identity_length) {
        ("null", None) => None,
        ("blob", Some(32)) => Some(row.get(35)?),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(RawScanRow {
        id: row.get(2)?,
        root: row.get(5)?,
        root_encoding: row.get(6)?,
        started_at_unix_ms: row.get(7)?,
        completed_at_unix_ms: row.get(8)?,
        status: row.get(11)?,
        directory_count: row.get(12)?,
        file_count: row.get(13)?,
        logical_bytes: row.get(14)?,
        allocated_bytes: row.get(15)?,
        coverage_status: row.get(18)?,
        coverage_permille: row.get(20)?,
        issue_count: row.get(22)?,
        snapshot,
        root_identity_v1_sha256,
    })
}

fn validate_stored_value(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
    expected_type: &str,
    minimum_length: i64,
    maximum_length: i64,
) -> rusqlite::Result<()> {
    let storage_type: String = row.get(type_column)?;
    let length: i64 = row.get(length_column)?;
    if storage_type != expected_type || !(minimum_length..=maximum_length).contains(&length) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn decode_scan_row(connection: &Connection, raw: RawScanRow) -> Result<ScanRecord, HistoryError> {
    let id = ScanId::new(raw.id).map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
    let root = decode_host_path(&EncodedBytes {
        bytes: raw.root,
        encoding: stored_host_encoding(raw.root_encoding)?,
    })
    .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
    if !root.is_absolute() {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    let started_at = unix_ms_to_system_time(raw.started_at_unix_ms)?;
    let completed_at = raw
        .completed_at_unix_ms
        .map(unix_ms_to_system_time)
        .transpose()?;
    let status = ScanStatus::from_stored(&raw.status)?;
    if status.is_terminal() != completed_at.is_some()
        || completed_at.is_some_and(|completed| completed < started_at)
    {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    let counts = ScanCounts {
        directory_count: from_i64(raw.directory_count)?,
        file_count: from_i64(raw.file_count)?,
        logical_bytes: from_i64(raw.logical_bytes)?,
        allocated_bytes: raw.allocated_bytes.map(from_i64).transpose()?,
    };
    let snapshot = raw
        .snapshot
        .map(|snapshot| decode_snapshot_reference(&id, snapshot))
        .transpose()?;
    let root_identity_v1_sha256 = raw
        .root_identity_v1_sha256
        .map(|digest| {
            digest
                .try_into()
                .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))
        })
        .transpose()?;
    if snapshot.is_some() && status != ScanStatus::Succeeded {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    let coverage = load_scan_coverage_within_budget(
        connection,
        id.as_str(),
        &root,
        status.is_terminal(),
        &raw.coverage_status,
        raw.coverage_permille,
        raw.issue_count,
    )?;
    if let Some(terminal_status) = terminal_status(status) {
        validate_terminal_coverage(terminal_status, &coverage, HistoryErrorKind::CorruptData)?;
    }
    Ok(ScanRecord {
        id,
        root,
        started_at,
        completed_at,
        status,
        counts,
        coverage,
        snapshot,
        root_identity_v1_sha256,
    })
}

fn terminal_status(status: ScanStatus) -> Option<TerminalScanStatus> {
    match status {
        ScanStatus::Succeeded => Some(TerminalScanStatus::Succeeded),
        ScanStatus::Failed => Some(TerminalScanStatus::Failed),
        ScanStatus::Cancelled => Some(TerminalScanStatus::Cancelled),
        ScanStatus::Interrupted => Some(TerminalScanStatus::Interrupted),
        ScanStatus::Queued | ScanStatus::Running => None,
    }
}

fn validate_terminal_coverage(
    status: TerminalScanStatus,
    coverage: &ScanCoverage,
    error_kind: HistoryErrorKind,
) -> Result<(), HistoryError> {
    if coverage.status() == ScanCoverageStatus::Unknown {
        return Ok(());
    }
    let has_cancelled = coverage
        .issues()
        .iter()
        .any(|issue| issue.kind() == ScanIssueKind::Cancelled);
    let compatible = match status {
        TerminalScanStatus::Succeeded => !has_cancelled,
        TerminalScanStatus::Cancelled => has_cancelled,
        TerminalScanStatus::Failed | TerminalScanStatus::Interrupted => {
            coverage.status() != ScanCoverageStatus::Complete
        }
    };
    if compatible {
        Ok(())
    } else {
        Err(HistoryError::new(error_kind))
    }
}

fn decode_snapshot_reference(
    scan_id: &ScanId,
    raw: RawSnapshotReference,
) -> Result<SnapshotReference, HistoryError> {
    let version = u32::try_from(raw.version)
        .ok()
        .filter(|version| *version > 0)
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
    let relative_path = decode_host_path(&EncodedBytes {
        bytes: raw.relative_path,
        encoding: stored_host_encoding(raw.relative_path_encoding)?,
    })
    .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
    let mut components = relative_path.components();
    let Some(std::path::Component::Normal(file_name)) = components.next() else {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    };
    if components.next().is_some() {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    let file_name = file_name
        .to_str()
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
    let digest: [u8; 32] = raw
        .digest
        .try_into()
        .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
    SnapshotReference::from_stored(scan_id, version, file_name, digest)
        .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))
}

fn validate_counts(counts: ScanCounts, kind: HistoryErrorKind) -> Result<(), HistoryError> {
    for value in [
        counts.directory_count,
        counts.file_count,
        counts.logical_bytes,
    ] {
        to_i64(value, kind)?;
    }
    if let Some(value) = counts.allocated_bytes {
        to_i64(value, kind)?;
    }
    Ok(())
}

fn stored_host_encoding(value: i64) -> Result<StoredEncoding, HistoryError> {
    match value {
        1 => Ok(StoredEncoding::Utf8HostPath),
        2 => Ok(StoredEncoding::Utf16LeHostPath),
        _ => Err(HistoryError::new(HistoryErrorKind::CorruptData)),
    }
}

fn to_i64(value: u64, kind: HistoryErrorKind) -> Result<i64, HistoryError> {
    i64::try_from(value).map_err(|_| HistoryError::new(kind))
}

fn from_i64(value: i64) -> Result<u64, HistoryError> {
    u64::try_from(value).map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))
}

pub(super) fn system_time_to_unix_ms(
    value: SystemTime,
    error_kind: HistoryErrorKind,
) -> Result<i64, HistoryError> {
    let milliseconds = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HistoryError::new(error_kind))?
        .as_millis();
    i64::try_from(milliseconds).map_err(|_| HistoryError::new(error_kind))
}

pub(super) fn unix_ms_to_system_time(value: i64) -> Result<SystemTime, HistoryError> {
    let milliseconds =
        u64::try_from(value).map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
    UNIX_EPOCH
        .checked_add(Duration::from_millis(milliseconds))
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))
}

pub(super) fn run_bounded_query<T>(
    connection: &Connection,
    query: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    run_bounded_query_with_limits(connection, QUERY_MAX_CALLBACKS, QUERY_MAX_ELAPSED, query)
}

/// A larger fixed budget for bounded cross-process snapshot-pin population
/// inspection. The legal table has up to 1,024 exact rows, so the smaller
/// single-record history budget is insufficient even though this remains
/// independently time and VM bounded.
pub(super) fn run_bounded_snapshot_pin_query<T>(
    connection: &Connection,
    query: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    run_bounded_query_with_limits(connection, 50_000, Duration::from_secs(1), query)
}

/// A fixed but item-scaled budget for indexed reconciliation of the bounded
/// physical snapshot-store inventory. This does not make historical scan
/// enumeration bounded; callers must issue at most one indexed lookup for each
/// already-bounded physical entry.
pub(super) fn run_bounded_snapshot_retention_inventory_query<T>(
    connection: &Connection,
    physical_final_count: usize,
    query: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    if physical_final_count > 2_048 {
        return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
    }
    let physical_final_count = u64::try_from(physical_final_count)
        .map_err(|_| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
    let maximum_callbacks = 10_000_u64.saturating_add(250 * physical_final_count);
    run_bounded_query_with_limits(connection, maximum_callbacks, Duration::from_secs(5), query)
}

fn run_bounded_recent_query<T>(
    connection: &Connection,
    limit: usize,
    query: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    let limit =
        u64::try_from(limit).map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    let maximum_callbacks = RECENT_QUERY_BASE_CALLBACKS
        .saturating_add(RECENT_QUERY_CALLBACKS_PER_SCAN.saturating_mul(limit));
    run_bounded_query_with_limits(
        connection,
        maximum_callbacks,
        RECENT_QUERY_MAX_ELAPSED,
        query,
    )
}

fn run_bounded_query_with_limits<T>(
    connection: &Connection,
    maximum_callbacks: u64,
    maximum_elapsed: Duration,
    query: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    let started_at = Instant::now();
    let mut callbacks = 0_u64;
    connection
        .progress_handler(
            QUERY_PROGRESS_INTERVAL,
            Some(move || {
                callbacks = callbacks.saturating_add(1);
                callbacks >= maximum_callbacks || started_at.elapsed() >= maximum_elapsed
            }),
        )
        .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
    let mut guard = QueryProgressGuard {
        connection,
        installed: true,
    };
    let result = query();
    guard.remove()?;
    let value = result?;
    if started_at.elapsed() >= maximum_elapsed {
        return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
    }
    Ok(value)
}

/// Dedicated fixed budget for the path-free cleanup-history pager. The pager
/// validates bounded scalar structure and lifecycle state without reading path
/// or evidence payload values.
pub(super) fn run_bounded_cleanup_history_page_query<T>(
    connection: &Connection,
    query: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    run_bounded_query_with_limits(connection, 250_000, Duration::from_secs(3), query)
}

/// Dedicated fixed budget for one complete cleanup-history observation. This
/// is intentionally separate from the journal's mutation-time reads so callers
/// never nest SQLite progress handlers.
pub(super) fn run_bounded_cleanup_history_observation_query<T>(
    connection: &Connection,
    query: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    run_bounded_query_with_limits(connection, 300_000, Duration::from_secs(3), query)
}

struct QueryProgressGuard<'connection> {
    connection: &'connection Connection,
    installed: bool,
}

impl QueryProgressGuard<'_> {
    fn remove(&mut self) -> Result<(), HistoryError> {
        self.connection
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for QueryProgressGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self.connection.progress_handler(0, None::<fn() -> bool>);
        }
    }
}

pub(super) fn map_write_sql_error(error: rusqlite::Error) -> HistoryError {
    use rusqlite::ErrorCode;
    let kind = match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => HistoryErrorKind::Busy,
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => HistoryErrorKind::CorruptData,
        _ => HistoryErrorKind::DatabaseUnavailable,
    };
    HistoryError::new(kind)
}

pub(super) fn map_query_sql_error(error: rusqlite::Error) -> HistoryError {
    use rusqlite::ErrorCode;
    let kind = match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => HistoryErrorKind::Busy,
        Some(ErrorCode::OperationInterrupted) => HistoryErrorKind::QueryLimitExceeded,
        _ => HistoryErrorKind::CorruptData,
    };
    HistoryError::new(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::MAX_SCAN_ISSUES;
    use crate::persistence::StoreCoordinator;
    use crate::persistence::snapshot::{SnapshotFileName, SnapshotReference};
    use crate::{
        CoveragePermille, DATABASE_SCHEMA_VERSION, ScanCoverageStatus, ScanIssue, ScanIssueKind,
    };
    use tempfile::TempDir;

    fn started(id: &str, root: PathBuf, offset_ms: u64) -> NewScanRecord {
        NewScanRecord::try_new_without_root_identity(
            ScanId::new(id).unwrap(),
            root,
            UNIX_EPOCH + Duration::from_millis(1_750_000_000_000 + offset_ms),
        )
        .unwrap()
    }

    fn completion(id: &str, offset_ms: u64) -> ScanCompletionRecord {
        ScanCompletionRecord::try_new(
            ScanId::new(id).unwrap(),
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_000 + offset_ms),
            TerminalScanStatus::Succeeded,
            ScanCounts {
                directory_count: 3,
                file_count: 7,
                logical_bytes: 11,
                allocated_bytes: Some(13),
            },
        )
        .unwrap()
    }

    fn snapshot_reference(id: &ScanId) -> SnapshotReference {
        let name = SnapshotFileName::from_scan_id(id.as_str().as_bytes());
        SnapshotReference::from_stored(id, 1, name.as_str(), [0x5a; 32]).unwrap()
    }

    fn partial_coverage(root: &Path) -> ScanCoverage {
        ScanCoverage::try_from_terminal(
            Some(CoveragePermille::new(750).unwrap()),
            vec![
                ScanIssue::try_new(
                    ScanIssueKind::MetadataError,
                    Some(root.join("metadata-å")),
                    3,
                )
                .unwrap(),
                ScanIssue::try_new(
                    ScanIssueKind::PermissionDenied,
                    Some(root.join("denied")),
                    2,
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn start_finish_and_load_survive_process_style_reopen() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let start = started("scan:one", temp.path().join("scan-root-å"), 0);
        let finish = completion("scan:one", 0);

        {
            let store = StoreCoordinator::open(&database).unwrap();
            store.record_scan_started(&start).unwrap();
            let running = store.load_scan(start.id()).unwrap().unwrap();
            assert_eq!(running.id(), start.id());
            assert_eq!(running.status(), ScanStatus::Running);
            assert_eq!(running.root(), start.root());
            assert_eq!(running.started_at(), start.started_at());
            assert_eq!(running.completed_at(), None);
            assert_eq!(running.counts(), ScanCounts::default());
            assert_eq!(running.coverage(), &ScanCoverage::unknown());
            store.record_scan_finished(&finish).unwrap();
        }

        let reopened = StoreCoordinator::open(&database).unwrap();
        let completed = reopened.load_scan(start.id()).unwrap().unwrap();
        assert_eq!(completed.status(), ScanStatus::Succeeded);
        assert_eq!(completed.completed_at(), Some(finish.completed_at()));
        assert_eq!(completed.counts(), finish.counts());
    }

    #[test]
    fn scan_start_persists_domain_separated_root_identity_digest() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let start = NewScanRecord::try_new_with_root_identity(
            ScanId::new("scan:root-identity").unwrap(),
            temp.path().join("root"),
            UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            FilesystemIdentity::new(7, 11),
        )
        .unwrap();
        let expected = [
            0x7e, 0x88, 0x76, 0x62, 0xf3, 0x76, 0x7f, 0xc2, 0x2f, 0x21, 0x11, 0x0b, 0x46, 0x0c,
            0x81, 0xf7, 0x0f, 0x47, 0x05, 0x0a, 0x3a, 0xa2, 0x7e, 0x11, 0xf2, 0x89, 0xbe, 0x25,
            0x8d, 0x92, 0x52, 0x45,
        ];
        assert_eq!(start.root_identity_v1_sha256(), Some(expected));

        store.record_scan_started(&start).unwrap();

        let loaded = store.load_scan(start.id()).unwrap().unwrap();
        assert_eq!(loaded.root_identity_v1_sha256(), Some(expected));
        store.with_connection(|connection| {
            let stored: Vec<u8> = connection
                .query_row(
                    "SELECT root_identity_v1_sha256 FROM scans WHERE scan_id = ?1",
                    [start.id().as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(stored, expected);
        });
    }

    #[test]
    fn exact_root_since_query_is_bounded_newest_and_byte_exact() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let selected = temp.path().join("selected");
        let other = temp.path().join("other");
        let older = started("scan:targeted:exact-root-older", selected.clone(), 10);
        let newer = started("scan:targeted:exact-root-newer", selected.clone(), 30);
        let failed_newest = started("scan:exact-root-failed", selected.clone(), 40);
        let unrelated = started("scan:exact-root-other", other, 40);
        for scan in [&older, &newer, &failed_newest, &unrelated] {
            store.record_scan_started(scan).unwrap();
        }
        for scan in [&older, &newer] {
            store
                .record_scan_finished(
                    &ScanCompletionRecord::try_succeeded_with_snapshot(
                        scan.id().clone(),
                        scan.started_at() + Duration::from_millis(1),
                        ScanCounts::default(),
                        snapshot_reference(scan.id()),
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        store.with_connection(|connection| {
            for scan in [&older, &newer] {
                let scheduled =
                    system_time_to_unix_ms(scan.started_at(), HistoryErrorKind::InvalidInput)
                        .unwrap();
                connection
                    .execute(
                        "INSERT INTO candidate_evaluations (
                           scan_id, record_format_version, evaluator_revision,
                           rule_catalog_schema_version, rule_catalog_sha256,
                           context_format_version, context_sha256,
                           snapshot_version, snapshot_sha256,
                           scheduled_at_unix_ms, completed_at_unix_ms,
                           status, candidate_count, failure_kind
                         ) VALUES (
                           ?1, 1, 1, 1, zeroblob(32), 1, zeroblob(32),
                           1, zeroblob(32), ?2, ?3, 'failed', NULL, 'cancelled'
                         )",
                        params![scan.id().as_str(), scheduled, scheduled + 1],
                    )
                    .unwrap();
            }
        });
        store
            .record_scan_finished(
                &ScanCompletionRecord::try_new(
                    failed_newest.id().clone(),
                    failed_newest.started_at() + Duration::from_millis(1),
                    TerminalScanStatus::Failed,
                    ScanCounts::default(),
                )
                .unwrap(),
            )
            .unwrap();

        assert_eq!(
            store
                .load_latest_scan_for_exact_root_since(
                    &selected,
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_020),
                )
                .unwrap()
                .unwrap()
                .id(),
            newer.id()
        );
        assert!(
            store
                .load_latest_scan_for_exact_root_since(
                    &selected,
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_031),
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn committed_scan_start_reconciles_an_injected_post_commit_failure() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let start = started("scan:ambiguous-start", temp.path().join("root"), 0);

        store
            .record_scan_started_reconciled_after_commit_failure_for_test(&start)
            .unwrap();
        store.record_scan_started_reconciled(&start).unwrap();
        assert!(
            store
                .load_scan(start.id())
                .unwrap()
                .unwrap()
                .exactly_matches_start(&start)
        );
    }

    #[test]
    fn terminal_coverage_and_issues_are_atomic_shortened_and_survive_reopen() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let start = started("scan:coverage", root.clone(), 0);
        let coverage = partial_coverage(&root);
        let finish = ScanCompletionRecord::try_new_with_coverage(
            start.id().clone(),
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
            TerminalScanStatus::Succeeded,
            ScanCounts {
                directory_count: 2,
                file_count: 4,
                logical_bytes: 8,
                allocated_bytes: Some(12),
            },
            coverage.clone(),
        )
        .unwrap();

        {
            let store = StoreCoordinator::open(&database).unwrap();
            store.record_scan_started(&start).unwrap();
            store.record_scan_finished_reconciled(&finish).unwrap();
            let stored = store.load_scan(start.id()).unwrap().unwrap();
            assert_eq!(stored.coverage(), &coverage);
            assert!(stored.exactly_matches_completion(&finish));
            store.with_connection(|connection| {
                let issue_count: i64 = connection
                    .query_row(
                        "SELECT issue_count FROM scans WHERE scan_id = ?1",
                        [start.id().as_str()],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(issue_count, 5);
                let shortened = connection
                    .prepare(
                        "SELECT shortened_path, shortened_path_encoding
                         FROM scan_issues WHERE scan_id = ?1 ORDER BY issue_id",
                    )
                    .unwrap()
                    .query_map([start.id().as_str()], |row| {
                        Ok(EncodedBytes {
                            bytes: row.get(0)?,
                            encoding: stored_host_encoding(row.get(1)?).unwrap(),
                        })
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                assert_eq!(
                    shortened
                        .into_iter()
                        .map(|path| decode_host_path(&path).unwrap())
                        .collect::<Vec<_>>(),
                    [PathBuf::from("denied"), PathBuf::from("metadata-å")]
                );
            });
        }

        let reopened = StoreCoordinator::open(&database).unwrap();
        assert_eq!(
            reopened.load_scan(start.id()).unwrap().unwrap().coverage(),
            &coverage
        );
    }

    #[test]
    fn terminal_status_and_known_coverage_must_agree() {
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let cancelled = ScanCoverage::try_from_terminal(
            None,
            vec![ScanIssue::try_new(ScanIssueKind::Cancelled, None, 1).unwrap()],
        )
        .unwrap();
        let partial = ScanCoverage::try_from_terminal(
            None,
            vec![
                ScanIssue::try_new(
                    ScanIssueKind::MetadataError,
                    Some(if cfg!(windows) {
                        PathBuf::from(r"C:\scan\root\error")
                    } else {
                        PathBuf::from("/scan/root/error")
                    }),
                    1,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let complete = ScanCoverage::try_from_terminal(None, Vec::new()).unwrap();

        let cases = [
            (TerminalScanStatus::Succeeded, cancelled.clone()),
            (TerminalScanStatus::Cancelled, partial.clone()),
            (TerminalScanStatus::Failed, complete.clone()),
            (TerminalScanStatus::Interrupted, complete),
        ];
        for (index, (status, coverage)) in cases.into_iter().enumerate() {
            let completion = ScanCompletionRecord::try_new_with_coverage(
                ScanId::new(format!("scan:coverage-status-{index}")).unwrap(),
                completed_at,
                status,
                ScanCounts::default(),
                coverage,
            )
            .unwrap();
            assert_eq!(
                PreparedScanCompletion::prepare(&completion)
                    .err()
                    .unwrap()
                    .kind,
                HistoryErrorKind::InvalidInput
            );
        }

        let valid_cancelled = ScanCompletionRecord::try_new_with_coverage(
            ScanId::new("scan:coverage-status-valid").unwrap(),
            completed_at,
            TerminalScanStatus::Cancelled,
            ScanCounts::default(),
            cancelled,
        )
        .unwrap();
        PreparedScanCompletion::prepare(&valid_cancelled).unwrap();
    }

    #[test]
    fn out_of_root_issue_rolls_back_parent_and_every_child() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("scan-root");
        let start = started("scan:issue-rollback", root, 0);
        store.record_scan_started(&start).unwrap();
        let coverage = ScanCoverage::try_from_terminal(
            None,
            vec![
                ScanIssue::try_new(
                    ScanIssueKind::MetadataError,
                    Some(temp.path().join("outside")),
                    1,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let finish = ScanCompletionRecord::try_new_with_coverage(
            start.id().clone(),
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
            TerminalScanStatus::Succeeded,
            ScanCounts::default(),
            coverage,
        )
        .unwrap();

        assert_eq!(
            store.record_scan_finished(&finish).unwrap_err().kind,
            HistoryErrorKind::InvalidInput
        );
        let running = store.load_scan(start.id()).unwrap().unwrap();
        assert_eq!(running.status(), ScanStatus::Running);
        assert_eq!(running.coverage().status(), ScanCoverageStatus::Unknown);
        store.with_connection(|connection| {
            let children: i64 = connection
                .query_row(
                    "SELECT count(*) FROM scan_issues WHERE scan_id = ?1",
                    [start.id().as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(children, 0);
        });
    }

    #[test]
    fn scan_ids_are_immutable_and_finish_is_compare_and_set() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store").join("dux.sqlite3")).unwrap();
        let start = started("scan:one", temp.path().join("one"), 0);
        let finish = completion("scan:one", 0);
        store.record_scan_started(&start).unwrap();
        assert_eq!(
            store.record_scan_started(&start).unwrap_err().kind,
            HistoryErrorKind::AlreadyExists
        );
        store.record_scan_finished(&finish).unwrap();
        assert_eq!(
            store.record_scan_finished(&finish).unwrap_err().kind,
            HistoryErrorKind::InvalidTransition
        );
        assert_eq!(
            store
                .record_scan_finished(&completion("scan:missing", 0))
                .unwrap_err()
                .kind,
            HistoryErrorKind::NotFound
        );
    }

    #[test]
    fn exact_retry_adopts_only_the_same_canonical_coverage_facts() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        let start = started("scan:coverage-collision", root.clone(), 0);
        store.record_scan_started(&start).unwrap();
        let first = ScanCompletionRecord::try_new_with_coverage(
            start.id().clone(),
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
            TerminalScanStatus::Succeeded,
            ScanCounts::default(),
            partial_coverage(&root),
        )
        .unwrap();
        let different = ScanCompletionRecord::try_new_with_coverage(
            start.id().clone(),
            first.completed_at(),
            TerminalScanStatus::Succeeded,
            first.counts(),
            ScanCoverage::try_from_terminal(
                None,
                vec![
                    ScanIssue::try_new(ScanIssueKind::TimedOut, Some(root.join("different")), 1)
                        .unwrap(),
                ],
            )
            .unwrap(),
        )
        .unwrap();

        store.record_scan_finished_reconciled(&first).unwrap();
        store.record_scan_finished_reconciled(&first).unwrap();
        assert_eq!(
            store
                .record_scan_finished_reconciled(&different)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        assert!(
            store
                .load_scan(start.id())
                .unwrap()
                .unwrap()
                .exactly_matches_completion(&first)
        );
    }

    #[test]
    fn snapshot_tuple_is_atomic_typed_and_exactly_reconciled() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let start = started("scan:with-snapshot", temp.path().join("root"), 0);
        let reference = snapshot_reference(start.id());
        let finish = ScanCompletionRecord::try_succeeded_with_snapshot(
            start.id().clone(),
            UNIX_EPOCH + Duration::from_nanos(1_750_000_002_000_999_999),
            ScanCounts {
                directory_count: 1,
                file_count: 2,
                logical_bytes: 3,
                allocated_bytes: Some(4),
            },
            reference.clone(),
        )
        .unwrap();

        {
            let store = StoreCoordinator::open(&database).unwrap();
            store.record_scan_started(&start).unwrap();
            store.record_scan_finished_reconciled(&finish).unwrap();
            store.record_scan_finished_reconciled(&finish).unwrap();
        }

        let reopened = StoreCoordinator::open(&database).unwrap();
        let stored = reopened.load_scan(start.id()).unwrap().unwrap();
        assert_eq!(stored.snapshot(), Some(&reference));
        assert!(stored.exactly_matches_completion(&finish));
        assert_eq!(
            finish.completed_at(),
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_000)
        );
    }

    #[test]
    fn committed_scan_completion_reconciles_an_injected_post_commit_failure() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let start = started("scan:ambiguous-commit", temp.path().join("root"), 0);
        let finish = ScanCompletionRecord::try_succeeded_with_snapshot(
            start.id().clone(),
            UNIX_EPOCH + Duration::from_nanos(1_750_000_002_000_999_999),
            ScanCounts {
                directory_count: 1,
                file_count: 1,
                logical_bytes: 2,
                allocated_bytes: Some(3),
            },
            snapshot_reference(start.id()),
        )
        .unwrap();
        store.record_scan_started(&start).unwrap();

        store
            .record_scan_finished_reconciled_after_commit_failure_for_test(&finish)
            .unwrap();

        assert!(
            store
                .load_scan(start.id())
                .unwrap()
                .unwrap()
                .exactly_matches_completion(&finish)
        );
    }

    #[cfg(unix)]
    #[test]
    fn exact_post_commit_match_cannot_mask_unsafe_retained_storage() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let marker = database.with_extension("sqlite3.writer.lock");
        let store = StoreCoordinator::open(&database).unwrap();
        let root = temp.path().join("root");
        let start = started("scan:unsafe-post-commit", root.clone(), 0);
        let finish = ScanCompletionRecord::try_new_with_coverage(
            start.id().clone(),
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
            TerminalScanStatus::Succeeded,
            ScanCounts::default(),
            partial_coverage(&root),
        )
        .unwrap();
        store.record_scan_started(&start).unwrap();

        let error = store
            .record_scan_finished_reconciled_with_after_commit_hook_for_test(&finish, || {
                std::fs::write(&marker, b"NOT-A-DUX-MARKER")
                    .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))
            })
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::OutcomeUnknown);
        store.with_connection(|connection| {
            let durable: (String, i64) = connection
                .query_row(
                    "SELECT status, issue_count FROM scans WHERE scan_id = ?1",
                    [start.id().as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(durable, ("succeeded".to_owned(), 5));
        });
    }

    #[test]
    fn scan_start_time_is_canonical_before_persistence_and_ordering() {
        let temp = TempDir::new().unwrap();
        let input = UNIX_EPOCH + Duration::from_nanos(1_750_000_000_000_999_999);
        let start = NewScanRecord::try_new_without_root_identity(
            ScanId::new("scan:canonical-start").unwrap(),
            temp.path().join("root"),
            input,
        )
        .unwrap();
        assert_eq!(
            start.started_at(),
            UNIX_EPOCH + Duration::from_millis(1_750_000_000_000)
        );

        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        store.record_scan_started(&start).unwrap();
        assert_eq!(
            store.load_scan(start.id()).unwrap().unwrap().started_at(),
            start.started_at()
        );
    }

    #[test]
    fn malformed_or_non_success_snapshot_tuples_fail_as_corrupt() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();

        let partial = started("scan:snapshot-partial", temp.path().join("partial"), 0);
        store.record_scan_started(&partial).unwrap();
        store.with_connection(|connection| {
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "UPDATE scans SET snapshot_version = 1 WHERE scan_id = ?1",
                    [partial.id().as_str()],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            store.load_scan(partial.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );

        let failed = started("scan:snapshot-failed", temp.path().join("failed"), 1);
        store.record_scan_started(&failed).unwrap();
        let reference = snapshot_reference(failed.id());
        let finish = ScanCompletionRecord::try_succeeded_with_snapshot(
            failed.id().clone(),
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_001),
            ScanCounts::default(),
            reference,
        )
        .unwrap();
        store.record_scan_finished(&finish).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE scans SET status = 'failed' WHERE scan_id = ?1",
                    [failed.id().as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_scan(failed.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn validation_rejects_relative_paths_pre_epoch_times_and_large_counts() {
        let temp = TempDir::new().unwrap();
        let id = ScanId::new("scan:invalid").unwrap();
        assert_eq!(
            NewScanRecord::try_new_without_root_identity(
                id.clone(),
                PathBuf::from("relative"),
                UNIX_EPOCH
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );
        assert_eq!(
            NewScanRecord::try_new_without_root_identity(
                id.clone(),
                temp.path().to_path_buf(),
                UNIX_EPOCH - Duration::from_millis(1),
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );
        assert_eq!(
            ScanCompletionRecord::try_new(
                id,
                UNIX_EPOCH,
                TerminalScanStatus::Failed,
                ScanCounts {
                    logical_bytes: u64::MAX,
                    ..ScanCounts::default()
                },
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );
    }

    #[test]
    fn completion_before_start_rolls_back_without_rewriting_running_record() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store").join("dux.sqlite3")).unwrap();
        let start = started("scan:one", temp.path().join("one"), 10);
        store.record_scan_started(&start).unwrap();
        let too_early = ScanCompletionRecord::try_new(
            start.id.clone(),
            UNIX_EPOCH + Duration::from_millis(1_750_000_000_009),
            TerminalScanStatus::Cancelled,
            ScanCounts::default(),
        )
        .unwrap();
        assert_eq!(
            store.record_scan_finished(&too_early).unwrap_err().kind,
            HistoryErrorKind::InvalidInput
        );
        assert_eq!(
            store.load_scan(start.id()).unwrap().unwrap().status(),
            ScanStatus::Running
        );
    }

    #[test]
    fn every_terminal_status_is_persisted_without_rewrite_authority() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store").join("dux.sqlite3")).unwrap();
        for (index, terminal) in [
            TerminalScanStatus::Succeeded,
            TerminalScanStatus::Failed,
            TerminalScanStatus::Cancelled,
            TerminalScanStatus::Interrupted,
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("scan:terminal:{index}");
            let start = started(&id, temp.path().join(format!("root-{index}")), index as u64);
            store.record_scan_started(&start).unwrap();
            let finish = ScanCompletionRecord::try_new(
                start.id.clone(),
                UNIX_EPOCH + Duration::from_millis(1_750_000_002_000 + index as u64),
                terminal,
                ScanCounts::default(),
            )
            .unwrap();
            store.record_scan_finished(&finish).unwrap();
            assert!(
                store
                    .load_scan(start.id())
                    .unwrap()
                    .unwrap()
                    .status()
                    .is_terminal()
            );
        }
    }

    #[test]
    fn malformed_lifecycle_row_fails_as_corrupt_observation() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store").join("dux.sqlite3")).unwrap();
        let start = started("scan:malformed", temp.path().join("root"), 0);
        store.record_scan_started(&start).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                    [start.id().as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE scans SET status = 'succeeded' WHERE scan_id = ?1",
                    [start.id().as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_scan(start.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
        assert_eq!(
            store
                .load_scan(&ScanId::new("scan:unrelated").unwrap())
                .unwrap(),
            None,
            "the progress handler must be removed after a decode failure"
        );
    }

    #[test]
    fn nonterminal_scan_cannot_claim_measured_coverage() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let start = started("scan:running-coverage", temp.path().join("root"), 0);
        store.record_scan_started(&start).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE scans
                     SET coverage_status = 'complete', coverage_permille = 1000
                     WHERE scan_id = ?1",
                    [start.id().as_str()],
                )
                .unwrap();
        });

        assert_eq!(
            store.load_scan(start.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn hostile_issue_kind_count_and_shortened_path_fail_closed() {
        for corruption in ["kind", "count", "path", "message"] {
            let temp = TempDir::new().unwrap();
            let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
            let root = temp.path().join("root");
            let start = started(&format!("scan:hostile:{corruption}"), root.clone(), 0);
            let finish = ScanCompletionRecord::try_new_with_coverage(
                start.id().clone(),
                UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
                TerminalScanStatus::Succeeded,
                ScanCounts::default(),
                partial_coverage(&root),
            )
            .unwrap();
            store.record_scan_started(&start).unwrap();
            store.record_scan_finished(&finish).unwrap();
            store.with_connection(|connection| match corruption {
                "kind" => {
                    connection
                        .execute(
                            "UPDATE scan_issues SET issue_kind = 'future_issue'
                             WHERE issue_id = (
                                 SELECT min(issue_id) FROM scan_issues WHERE scan_id = ?1
                             )",
                            [start.id().as_str()],
                        )
                        .unwrap();
                }
                "count" => {
                    connection
                        .execute(
                            "UPDATE scans SET issue_count = issue_count + 1 WHERE scan_id = ?1",
                            [start.id().as_str()],
                        )
                        .unwrap();
                }
                "path" => {
                    let encoded = encode_host_path(&root).unwrap();
                    connection
                        .execute(
                            "UPDATE scan_issues
                             SET shortened_path = ?2, shortened_path_encoding = ?3
                             WHERE issue_id = (
                                 SELECT min(issue_id) FROM scan_issues WHERE scan_id = ?1
                             )",
                            params![start.id().as_str(), encoded.bytes, encoded.encoding as i64],
                        )
                        .unwrap();
                }
                "message" => {
                    connection
                        .execute(
                            "UPDATE scan_issues SET message_key = 'scan.issue.wrong'
                             WHERE issue_id = (
                                 SELECT min(issue_id) FROM scan_issues WHERE scan_id = ?1
                             )",
                            [start.id().as_str()],
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            });
            assert_eq!(
                store.load_scan(start.id()).unwrap_err().kind,
                HistoryErrorKind::CorruptData,
                "accepted {corruption} corruption"
            );
            assert_eq!(
                store
                    .load_scan(&ScanId::new(format!("scan:clean:{corruption}")).unwrap())
                    .unwrap(),
                None,
                "query progress handler leaked after {corruption} decode"
            );
        }
    }

    #[test]
    fn hostile_issue_rows_over_the_domain_bound_are_rejected_before_materialization() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        let start = started("scan:too-many-issues", root, 0);
        store.record_scan_started(&start).unwrap();
        store
            .record_scan_finished(&completion(start.id().as_str(), 0))
            .unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE scans
                     SET coverage_status = 'partial', coverage_permille = NULL,
                         issue_count = ?2
                     WHERE scan_id = ?1",
                    params![
                        start.id().as_str(),
                        i64::try_from(MAX_SCAN_ISSUES + 1).unwrap()
                    ],
                )
                .unwrap();
            let mut statement = connection
                .prepare(
                    "INSERT INTO scan_issues (
                        scan_id, issue_kind, occurrence_count, message_key
                     ) VALUES (?1, 'cancelled', 1, 'scan.issue.cancelled')",
                )
                .unwrap();
            for _ in 0..=MAX_SCAN_ISSUES {
                statement.execute([start.id().as_str()]).unwrap();
            }
        });
        assert_eq!(
            store.load_scan(start.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn oversized_blob_is_rejected_before_path_materialization() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store").join("dux.sqlite3")).unwrap();
        let start = started("scan:oversized", temp.path().join("root"), 0);
        store.record_scan_started(&start).unwrap();
        store.with_connection(|connection| {
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "UPDATE scans SET root_path = zeroblob(65537) WHERE scan_id = ?1",
                    [start.id().as_str()],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            store.load_scan(start.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn external_schema_upgrade_blocks_history_writes_before_transaction() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let store = StoreCoordinator::open(&database).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO schema_migrations
                     (version, name, checksum_sha256, applied_at_unix_ms)
                     VALUES (?1, 'future-schema', zeroblob(32), 2)",
                    [i64::from(DATABASE_SCHEMA_VERSION + 1)],
                )
                .unwrap();
            connection
                .pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION + 1)
                .unwrap();
        });

        let scan = started("scan:blocked", temp.path().join("root"), 0);
        let error = store.record_scan_started(&scan).unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::IncompatibleSchema);
        assert!(!error.to_string().contains(database.to_str().unwrap()));
        store.with_connection(|connection| {
            let count: i64 = connection
                .query_row(
                    "SELECT count(*) FROM scans WHERE scan_id = ?1",
                    [scan.id().as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        });
    }
}

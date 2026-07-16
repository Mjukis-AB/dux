//! Typed scan-history records stored in the v1 SQLite schema.
//!
//! These values are historical observations for presentation and comparison.
//! A decoded path is never a path-validation witness or cleanup authority.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::domain::ScanId;

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::snapshot::SnapshotReference;

const QUERY_PROGRESS_INTERVAL: i32 = 100;
const QUERY_MAX_CALLBACKS: u64 = 1_000;
const QUERY_MAX_ELAPSED: Duration = Duration::from_millis(250);
const MAX_STORED_ID_BYTES: i64 = 128;
const MAX_STORED_PATH_BYTES: i64 = 65_536;
const MAX_STORED_STATUS_BYTES: i64 = 16;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NewScanRecord {
    id: ScanId,
    root: PathBuf,
    started_at: SystemTime,
}

impl NewScanRecord {
    pub(crate) fn try_new(
        id: ScanId,
        root: PathBuf,
        started_at: SystemTime,
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
        })
    }

    pub(crate) fn id(&self) -> &ScanId {
        &self.id
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn started_at(&self) -> SystemTime {
        self.started_at
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanCompletionRecord {
    id: ScanId,
    completed_at: SystemTime,
    status: TerminalScanStatus,
    counts: ScanCounts,
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
        validate_counts(counts, HistoryErrorKind::InvalidInput)?;
        Ok(Self {
            id,
            completed_at,
            status,
            counts,
            snapshot: None,
        })
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
        validate_counts(counts, HistoryErrorKind::InvalidInput)?;
        Ok(Self {
            id,
            completed_at,
            status: TerminalScanStatus::Succeeded,
            counts,
            snapshot: Some(snapshot),
        })
    }

    pub(crate) fn id(&self) -> &ScanId {
        &self.id
    }

    pub(crate) fn completed_at(&self) -> SystemTime {
        self.completed_at
    }

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
    snapshot: Option<SnapshotReference>,
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

    pub(crate) fn snapshot(&self) -> Option<&SnapshotReference> {
        self.snapshot.as_ref()
    }

    pub(crate) fn exactly_matches_completion(&self, completion: &ScanCompletionRecord) -> bool {
        self.id == completion.id
            && self.completed_at == Some(completion.completed_at)
            && self.status == ScanStatus::from(completion.status)
            && self.counts == completion.counts
            && self.snapshot == completion.snapshot
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
        })
    }
}

pub(super) struct PreparedScanCompletion {
    id: String,
    completed_at_unix_ms: i64,
    status: ScanStatus,
    counts: ScanCounts,
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
                status, coverage_status
             ) VALUES (?1, ?2, ?3, ?4, 'running', 'unknown')
             ON CONFLICT(scan_id) DO NOTHING",
            params![
                scan.id,
                scan.root.bytes,
                scan.root.encoding as i64,
                scan.started_at_unix_ms,
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
                 snapshot_checksum_sha256 = ?11
             WHERE scan_id = ?1 AND status = 'running'
               AND completed_at_unix_ms IS NULL AND started_at_unix_ms <= ?2",
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
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed == 1 {
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

pub(super) fn load_scan_record(
    connection: &Connection,
    id: &ScanId,
) -> Result<Option<ScanRecord>, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(
                "SELECT typeof(scan_id), length(CAST(scan_id AS BLOB)), scan_id,
                        typeof(root_path), length(root_path), root_path,
                        root_path_encoding, started_at_unix_ms, completed_at_unix_ms,
                        typeof(status), length(CAST(status AS BLOB)), status,
                        directory_count, file_count, logical_bytes, allocated_bytes,
                        typeof(snapshot_version), snapshot_version,
                        typeof(snapshot_relative_path), length(snapshot_relative_path),
                        snapshot_relative_path,
                        typeof(snapshot_relative_path_encoding), snapshot_relative_path_encoding,
                        typeof(snapshot_checksum_sha256), length(snapshot_checksum_sha256),
                        snapshot_checksum_sha256
                 FROM scans WHERE scan_id = ?1",
                [id.as_str()],
                raw_scan_row,
            )
            .optional()
            .map_err(map_query_sql_error)?;
        raw.map(decode_scan_row).transpose()
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
    snapshot: Option<RawSnapshotReference>,
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
    let snapshot_types = [
        row.get::<_, String>(16)?,
        row.get::<_, String>(18)?,
        row.get::<_, String>(21)?,
        row.get::<_, String>(23)?,
    ];
    let snapshot = if snapshot_types.iter().all(|value| value == "null") {
        None
    } else if snapshot_types == ["integer", "blob", "integer", "blob"] {
        let relative_path_length: i64 = row.get(19)?;
        let digest_length: i64 = row.get(24)?;
        if !(1..=MAX_STORED_PATH_BYTES).contains(&relative_path_length) || digest_length != 32 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Some(RawSnapshotReference {
            version: row.get(17)?,
            relative_path: row.get(20)?,
            relative_path_encoding: row.get(22)?,
            digest: row.get(25)?,
        })
    } else {
        return Err(rusqlite::Error::InvalidQuery);
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
        snapshot,
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

fn decode_scan_row(raw: RawScanRow) -> Result<ScanRecord, HistoryError> {
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
    if snapshot.is_some() && status != ScanStatus::Succeeded {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    Ok(ScanRecord {
        id,
        root,
        started_at,
        completed_at,
        status,
        counts,
        snapshot,
    })
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
    let started_at = Instant::now();
    let mut callbacks = 0_u64;
    connection
        .progress_handler(
            QUERY_PROGRESS_INTERVAL,
            Some(move || {
                callbacks = callbacks.saturating_add(1);
                callbacks >= QUERY_MAX_CALLBACKS || started_at.elapsed() >= QUERY_MAX_ELAPSED
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
    if started_at.elapsed() >= QUERY_MAX_ELAPSED {
        return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
    }
    Ok(value)
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
    use crate::DATABASE_SCHEMA_VERSION;
    use crate::persistence::StoreCoordinator;
    use crate::persistence::snapshot::{SnapshotFileName, SnapshotReference};
    use tempfile::TempDir;

    fn started(id: &str, root: PathBuf, offset_ms: u64) -> NewScanRecord {
        NewScanRecord::try_new(
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
            store.record_scan_finished(&finish).unwrap();
        }

        let reopened = StoreCoordinator::open(&database).unwrap();
        let completed = reopened.load_scan(start.id()).unwrap().unwrap();
        assert_eq!(completed.status(), ScanStatus::Succeeded);
        assert_eq!(completed.completed_at(), Some(finish.completed_at()));
        assert_eq!(completed.counts(), finish.counts());
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

    #[test]
    fn scan_start_time_is_canonical_before_persistence_and_ordering() {
        let temp = TempDir::new().unwrap();
        let input = UNIX_EPOCH + Duration::from_nanos(1_750_000_000_000_999_999);
        let start = NewScanRecord::try_new(
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
            NewScanRecord::try_new(id.clone(), PathBuf::from("relative"), UNIX_EPOCH)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidInput
        );
        assert_eq!(
            NewScanRecord::try_new(
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

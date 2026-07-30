use std::fmt::Write as _;
use std::io::{self, Write as _};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dux_core::{
    DATABASE_SCHEMA_VERSION, DatabaseAccess, DatabaseOpenErrorKind, DatabaseStatus,
    DurableScanCounts, DurableScanCoverage, DurableScanStatus, DurableScanSummary, EngineConfig,
    EngineHandle, EngineOpenError, RecentScanHistory, ScanCoverageStatus, ScanHistoryError,
    SnapshotOpenErrorKind,
};
use serde::Serialize;

pub(crate) const JSON_SCHEMA_VERSION: u32 = 1;
const DEFAULT_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn run_status(json: bool) -> ExitCode {
    emit("status", json, build_status(json))
}

pub(crate) fn run_history(json: bool, limit: usize) -> ExitCode {
    emit("history", json, build_history(json, limit))
}

fn build_status(json: bool) -> Result<String, CommandError> {
    with_engine(|engine| {
        let database = engine
            .database_status()
            .map_err(CommandError::from_database_open)?;
        let latest_scan = match database.access {
            DatabaseAccess::ReadWriteCurrent => engine
                .recent_scan_history(1)
                .map_err(CommandError::from_history)?
                .scans
                .into_iter()
                .next(),
            DatabaseAccess::ReadOnlyNewer { .. } => None,
        };
        if json {
            serialize_json(&StatusDocument::try_from_engine(database, latest_scan)?)
        } else {
            render_status_human(database, latest_scan.as_ref())
        }
    })
}

fn build_history(json: bool, limit: usize) -> Result<String, CommandError> {
    with_engine(|engine| {
        let database = engine
            .database_status()
            .map_err(CommandError::from_database_open)?;
        if matches!(database.access, DatabaseAccess::ReadOnlyNewer { .. }) {
            return Err(CommandError::new(
                ErrorCode::IncompatibleDatabaseSchema,
                "Scan history is unavailable because the database was created by a newer DUX version.",
            ));
        }
        let history = engine
            .recent_scan_history(limit)
            .map_err(CommandError::from_history)?;
        if json {
            serialize_json(&HistoryDocument::try_from_engine(database, limit, history)?)
        } else {
            render_history_human(database, &history, limit)
        }
    })
}

pub(crate) fn with_engine<T>(
    operation: impl FnOnce(&EngineHandle) -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    let config = default_engine_config()?;
    let engine = EngineHandle::open(config).map_err(CommandError::from_engine_open)?;
    let result = operation(&engine);
    engine.close();
    if !engine.wait_until_closed(DEFAULT_CLOSE_TIMEOUT) && result.is_ok() {
        return Err(CommandError::new(
            ErrorCode::InternalError,
            "The DUX engine did not stop cleanly.",
        ));
    }
    result
}

pub(crate) fn default_engine_config() -> Result<EngineConfig, CommandError> {
    let data_parent = dirs::data_dir().ok_or_else(|| {
        CommandError::new(
            ErrorCode::StorageUnavailable,
            "The platform application-data directory is unavailable.",
        )
    })?;
    let cache_parent = dirs::cache_dir().ok_or_else(|| {
        CommandError::new(
            ErrorCode::StorageUnavailable,
            "The platform cache directory is unavailable.",
        )
    })?;
    prepare_platform_parent(&data_parent)?;
    prepare_platform_parent(&cache_parent)?;
    engine_config_for_roots(data_parent.join("Dux"), cache_parent.join("Dux"))
}

fn prepare_platform_parent(path: &std::path::Path) -> Result<(), CommandError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;

        // These directories are adapter-owned platform containers. The core
        // independently validates the exact publication parent before it
        // stages or publishes any private DUX storage beneath it.
        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| {
        CommandError::new(
            ErrorCode::StorageUnavailable,
            "The platform storage directory could not be prepared.",
        )
    })
}

fn engine_config_for_roots(
    data_root: std::path::PathBuf,
    cache_root: std::path::PathBuf,
) -> Result<EngineConfig, CommandError> {
    EngineConfig::new(
        data_root.join("dux.sqlite3"),
        data_root.join("snapshots"),
        cache_root,
    )
    .map_err(|_| {
        CommandError::new(
            ErrorCode::StorageUnavailable,
            "The DUX storage configuration is invalid.",
        )
    })
}

pub(crate) fn emit(
    command: &'static str,
    json: bool,
    result: Result<String, CommandError>,
) -> ExitCode {
    match result {
        Ok(output) => match write_stdout(&output) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
            Err(_) => {
                let error = CommandError::new(
                    ErrorCode::InternalError,
                    "DUX could not write command output.",
                );
                write_error(command, json, &error);
                error.exit_code()
            }
        },
        Err(error) => {
            write_error(command, json, &error);
            error.exit_code()
        }
    }
}

fn write_stdout(output: &str) -> io::Result<()> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(output.as_bytes())?;
    lock.write_all(b"\n")?;
    lock.flush()
}

fn write_error(command: &'static str, json: bool, error: &CommandError) {
    let stderr = io::stderr();
    let mut lock = stderr.lock();
    if json {
        let document = ErrorDocument {
            schema_version: JSON_SCHEMA_VERSION,
            command,
            error: ErrorBody {
                schema_version: JSON_SCHEMA_VERSION,
                code: error.code.as_str(),
                message: error.message,
                retryable: error.code.retryable(),
            },
        };
        if serde_json::to_writer(&mut lock, &document).is_ok() {
            let _ = lock.write_all(b"\n");
        }
    } else {
        let _ = writeln!(lock, "Error: {}", error.message);
    }
}

pub(crate) fn serialize_json<T: Serialize>(value: &T) -> Result<String, CommandError> {
    serde_json::to_string_pretty(value).map_err(|_| {
        CommandError::new(
            ErrorCode::InternalError,
            "DUX could not serialize command output.",
        )
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CommandError {
    code: ErrorCode,
    message: &'static str,
}

impl CommandError {
    pub(crate) const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }

    #[cfg(test)]
    pub(crate) const fn code(self) -> ErrorCode {
        self.code
    }

    fn exit_code(self) -> ExitCode {
        ExitCode::from(self.code.exit_code())
    }

    fn from_engine_open(error: EngineOpenError) -> Self {
        match error {
            EngineOpenError::Database(kind) => Self::from_database_open(kind),
            EngineOpenError::Snapshot(SnapshotOpenErrorKind::Busy) => Self::busy(),
            EngineOpenError::Snapshot(
                SnapshotOpenErrorKind::UnsafeRoot
                | SnapshotOpenErrorKind::UnsafeObject
                | SnapshotOpenErrorKind::UnrecognizedStore,
            ) => Self::unsafe_storage(),
            EngineOpenError::Snapshot(
                SnapshotOpenErrorKind::InvalidConfiguration | SnapshotOpenErrorKind::Unavailable,
            ) => Self::storage_unavailable(),
            EngineOpenError::CandidateCatalogInvalid
            | EngineOpenError::Snapshot(SnapshotOpenErrorKind::InternalState)
            | EngineOpenError::WorkerUnavailable => Self::internal(),
        }
    }

    fn from_database_open(kind: DatabaseOpenErrorKind) -> Self {
        match kind {
            DatabaseOpenErrorKind::Busy => Self::busy(),
            DatabaseOpenErrorKind::UnsafeStorageRoot
            | DatabaseOpenErrorKind::UnsafeStorageObject
            | DatabaseOpenErrorKind::OwnershipMismatch
            | DatabaseOpenErrorKind::UnsafePermissions => Self::unsafe_storage(),
            DatabaseOpenErrorKind::UnrecognizedDatabase
            | DatabaseOpenErrorKind::CorruptDatabase => Self::new(
                ErrorCode::CorruptDatabase,
                "The DUX database is corrupt or invalid.",
            ),
            DatabaseOpenErrorKind::InspectionLimitExceeded => Self::new(
                ErrorCode::QueryLimitExceeded,
                "Database inspection exceeded its safety limit.",
            ),
            DatabaseOpenErrorKind::StorageRootUnavailable
            | DatabaseOpenErrorKind::DatabaseUnavailable
            | DatabaseOpenErrorKind::MigrationFailed => Self::storage_unavailable(),
            DatabaseOpenErrorKind::InternalState => Self::internal(),
        }
    }

    fn from_history(error: ScanHistoryError) -> Self {
        match error {
            ScanHistoryError::InvalidLimit { .. } => Self::new(
                ErrorCode::InternalError,
                "DUX encountered an internal history-limit error.",
            ),
            ScanHistoryError::IncompatibleSchema => Self::new(
                ErrorCode::IncompatibleDatabaseSchema,
                "Scan history is unavailable because the database schema is incompatible.",
            ),
            ScanHistoryError::QueryLimitExceeded => Self::new(
                ErrorCode::QueryLimitExceeded,
                "The history query exceeded its safety limit.",
            ),
            ScanHistoryError::Busy => Self::busy(),
            ScanHistoryError::UnsafeStorage => Self::unsafe_storage(),
            ScanHistoryError::CorruptData => Self::new(
                ErrorCode::CorruptDatabase,
                "The stored scan history is corrupt or invalid.",
            ),
            ScanHistoryError::Unavailable => Self::storage_unavailable(),
            ScanHistoryError::Closed | ScanHistoryError::InternalState => Self::internal(),
            _ => Self::internal(),
        }
    }

    pub(crate) const fn busy() -> Self {
        Self::new(
            ErrorCode::StorageBusy,
            "DUX storage is busy; try again shortly.",
        )
    }

    pub(crate) const fn unsafe_storage() -> Self {
        Self::new(
            ErrorCode::UnsafeStorage,
            "DUX refused to use storage that did not meet its safety requirements.",
        )
    }

    pub(crate) const fn storage_unavailable() -> Self {
        Self::new(ErrorCode::StorageUnavailable, "DUX storage is unavailable.")
    }

    pub(crate) const fn internal() -> Self {
        Self::new(
            ErrorCode::InternalError,
            "DUX encountered an internal error.",
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ErrorCode {
    StorageBusy,
    StorageUnavailable,
    UnsafeStorage,
    IncompatibleDatabaseSchema,
    CorruptDatabase,
    QueryLimitExceeded,
    ScanNotFound,
    CandidateNotFound,
    CandidateNotReviewable,
    CleanupSessionNotFound,
    CursorOutOfRange,
    OutcomeUnknown,
    InternalError,
}

impl ErrorCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::StorageBusy => "storage_busy",
            Self::StorageUnavailable => "storage_unavailable",
            Self::UnsafeStorage => "unsafe_storage",
            Self::IncompatibleDatabaseSchema => "incompatible_database_schema",
            Self::CorruptDatabase => "corrupt_database",
            Self::QueryLimitExceeded => "query_limit_exceeded",
            Self::ScanNotFound => "scan_not_found",
            Self::CandidateNotFound => "candidate_not_found",
            Self::CandidateNotReviewable => "candidate_not_reviewable",
            Self::CleanupSessionNotFound => "cleanup_session_not_found",
            Self::CursorOutOfRange => "cursor_out_of_range",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::InternalError => "internal_error",
        }
    }

    pub(crate) const fn retryable(self) -> bool {
        matches!(self, Self::StorageBusy)
    }

    const fn exit_code(self) -> u8 {
        match self {
            Self::StorageBusy => 4,
            Self::StorageUnavailable
            | Self::UnsafeStorage
            | Self::IncompatibleDatabaseSchema
            | Self::CorruptDatabase
            | Self::QueryLimitExceeded
            | Self::ScanNotFound
            | Self::CandidateNotFound
            | Self::CandidateNotReviewable
            | Self::CleanupSessionNotFound
            | Self::CursorOutOfRange
            | Self::OutcomeUnknown => 3,
            Self::InternalError => 70,
        }
    }
}

#[derive(Serialize)]
struct StatusDocument {
    schema_version: u32,
    command: &'static str,
    dux_version: &'static str,
    path_disclosure: &'static str,
    database: DatabaseDocument,
    capabilities: CapabilitiesDocument,
    latest_scan: Option<ScanDocument>,
}

impl StatusDocument {
    fn try_from_engine(
        database: DatabaseStatus,
        latest_scan: Option<DurableScanSummary>,
    ) -> Result<Self, CommandError> {
        let scan_history = match database.access {
            DatabaseAccess::ReadWriteCurrent => "available",
            DatabaseAccess::ReadOnlyNewer { .. } => "unavailable_newer_schema",
        };
        Ok(Self {
            schema_version: JSON_SCHEMA_VERSION,
            command: "status",
            dux_version: env!("CARGO_PKG_VERSION"),
            path_disclosure: "none",
            database: DatabaseDocument::from(database),
            capabilities: CapabilitiesDocument {
                schema_version: JSON_SCHEMA_VERSION,
                scan_history,
            },
            latest_scan: latest_scan.map(ScanDocument::try_from).transpose()?,
        })
    }
}

#[derive(Serialize)]
struct CapabilitiesDocument {
    schema_version: u32,
    scan_history: &'static str,
}

#[derive(Serialize)]
struct HistoryDocument {
    schema_version: u32,
    command: &'static str,
    history_kind: &'static str,
    path_disclosure: &'static str,
    order: &'static str,
    database: DatabaseDocument,
    requested_limit: usize,
    has_more: bool,
    items: Vec<ScanDocument>,
}

impl HistoryDocument {
    fn try_from_engine(
        database: DatabaseStatus,
        requested_limit: usize,
        history: RecentScanHistory,
    ) -> Result<Self, CommandError> {
        Ok(Self {
            schema_version: JSON_SCHEMA_VERSION,
            command: "history",
            history_kind: "scan",
            path_disclosure: "none",
            order: "started_at_desc_scan_id_asc",
            database: DatabaseDocument::from(database),
            requested_limit,
            has_more: history.has_more,
            items: history
                .scans
                .into_iter()
                .map(ScanDocument::try_from)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Serialize)]
struct DatabaseDocument {
    schema_version: u32,
    database_schema_version: u32,
    supported_database_schema_version: u32,
    access: &'static str,
}

impl From<DatabaseStatus> for DatabaseDocument {
    fn from(value: DatabaseStatus) -> Self {
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            database_schema_version: value.schema_version,
            supported_database_schema_version: DATABASE_SCHEMA_VERSION,
            access: database_access(value.access),
        }
    }
}

#[derive(Serialize)]
struct ScanDocument {
    schema_version: u32,
    scan_id: String,
    started_at_unix_ms: i64,
    completed_at_unix_ms: Option<i64>,
    status: &'static str,
    counts: Option<CountsDocument>,
    coverage: CoverageDocument,
    snapshot_recorded: bool,
}

impl TryFrom<DurableScanSummary> for ScanDocument {
    type Error = CommandError;

    fn try_from(value: DurableScanSummary) -> Result<Self, Self::Error> {
        Ok(Self {
            schema_version: JSON_SCHEMA_VERSION,
            scan_id: value.scan_id.as_str().to_owned(),
            started_at_unix_ms: unix_ms(value.started_at)?,
            completed_at_unix_ms: value.completed_at.map(unix_ms).transpose()?,
            status: durable_status(value.status),
            counts: value.counts.map(CountsDocument::from),
            coverage: CoverageDocument::from(value.coverage),
            snapshot_recorded: value.snapshot_recorded,
        })
    }
}

#[derive(Serialize)]
struct CountsDocument {
    schema_version: u32,
    directory_count: u64,
    file_count: u64,
    logical_bytes: u64,
    allocated_bytes: Option<u64>,
}

impl From<DurableScanCounts> for CountsDocument {
    fn from(value: DurableScanCounts) -> Self {
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            directory_count: value.directory_count,
            file_count: value.file_count,
            logical_bytes: value.logical_bytes,
            allocated_bytes: value.allocated_bytes,
        }
    }
}

#[derive(Serialize)]
struct CoverageDocument {
    schema_version: u32,
    status: &'static str,
    measured_permille: Option<u16>,
    issue_record_count: usize,
    issue_occurrence_count: u64,
}

impl From<DurableScanCoverage> for CoverageDocument {
    fn from(value: DurableScanCoverage) -> Self {
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            status: coverage_status(value.status),
            measured_permille: value.measured_permille.map(|value| value.get()),
            issue_record_count: value.issue_record_count,
            issue_occurrence_count: value.issue_occurrence_count,
        }
    }
}

#[derive(Serialize)]
struct ErrorDocument {
    schema_version: u32,
    command: &'static str,
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    schema_version: u32,
    code: &'static str,
    message: &'static str,
    retryable: bool,
}

fn database_access(value: DatabaseAccess) -> &'static str {
    match value {
        DatabaseAccess::ReadWriteCurrent => "read_write_current",
        DatabaseAccess::ReadOnlyNewer { .. } => "read_only_newer",
    }
}

pub(crate) fn durable_status(value: DurableScanStatus) -> &'static str {
    match value {
        DurableScanStatus::Queued => "queued",
        DurableScanStatus::Running => "running",
        DurableScanStatus::Succeeded => "succeeded",
        DurableScanStatus::Failed => "failed",
        DurableScanStatus::Cancelled => "cancelled",
        DurableScanStatus::Interrupted => "interrupted",
        _ => "unknown",
    }
}

pub(crate) fn coverage_status(value: ScanCoverageStatus) -> &'static str {
    match value {
        ScanCoverageStatus::Unknown => "unknown",
        ScanCoverageStatus::Complete => "complete",
        ScanCoverageStatus::LimitedAccess => "limited_access",
        ScanCoverageStatus::Partial => "partial",
    }
}

pub(crate) fn unix_ms(value: SystemTime) -> Result<i64, CommandError> {
    let milliseconds = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CommandError::internal())?
        .as_millis();
    i64::try_from(milliseconds).map_err(|_| CommandError::internal())
}

fn render_status_human(
    database: DatabaseStatus,
    latest: Option<&DurableScanSummary>,
) -> Result<String, CommandError> {
    let mut output = String::new();
    let _ = writeln!(output, "DUX {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        output,
        "Database: {} (schema {})",
        human_database_access(database.access),
        database.schema_version
    );
    match (database.access, latest) {
        (DatabaseAccess::ReadOnlyNewer { .. }, _) => {
            output.push_str("Latest scan: unavailable with newer database schema\n");
        }
        (_, None) => output.push_str("Latest scan: none\n"),
        (_, Some(scan)) => {
            let _ = writeln!(
                output,
                "Latest scan: {} ({})",
                scan.scan_id.as_str(),
                durable_status(scan.status)
            );
            let _ = writeln!(
                output,
                "Started: {} ms since Unix epoch",
                unix_ms(scan.started_at)?
            );
            if let Some(counts) = scan.counts {
                let _ = writeln!(
                    output,
                    "Observed: {} files, {} directories, {} logical",
                    counts.file_count,
                    counts.directory_count,
                    dux_core::format_size(counts.logical_bytes)
                );
            }
            let _ = writeln!(
                output,
                "Coverage: {}",
                coverage_status(scan.coverage.status)
            );
        }
    }
    output.pop();
    Ok(output)
}

fn render_history_human(
    database: DatabaseStatus,
    history: &RecentScanHistory,
    requested_limit: usize,
) -> Result<String, CommandError> {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Scan history — database schema {} ({})",
        database.schema_version,
        human_database_access(database.access)
    );
    if history.scans.is_empty() {
        output.push_str("No durable scans recorded.\n");
    } else {
        for scan in &history.scans {
            let observed = scan
                .counts
                .map(|counts| dux_core::format_size(counts.logical_bytes))
                .unwrap_or_else(|| "not measured".to_owned());
            let _ = writeln!(
                output,
                "{}  {:11}  {:>12}  started {}",
                scan.scan_id.as_str(),
                durable_status(scan.status),
                observed,
                unix_ms(scan.started_at)?
            );
        }
        if history.has_more {
            if requested_limit < 200 {
                output.push_str("More scans exist; increase --limit to show them.\n");
            } else {
                output.push_str(
                    "Older scans exist but are not available through history schema v1.\n",
                );
            }
        }
    }
    output.pop();
    Ok(output)
}

fn human_database_access(value: DatabaseAccess) -> &'static str {
    match value {
        DatabaseAccess::ReadWriteCurrent => "read/write",
        DatabaseAccess::ReadOnlyNewer { .. } => "read-only newer schema",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unknown_coverage() -> DurableScanCoverage {
        DurableScanCoverage {
            status: ScanCoverageStatus::Unknown,
            measured_permille: None,
            issue_record_count: 0,
            issue_occurrence_count: 0,
        }
    }

    #[test]
    fn explicit_roots_build_the_normative_layout() {
        let temp = tempfile::TempDir::new().unwrap();
        let data = temp.path().join("Application Support").join("Dux");
        let cache = temp.path().join("Caches").join("Dux");

        let config = engine_config_for_roots(data.clone(), cache.clone()).unwrap();

        assert_eq!(config.database_path(), data.join("dux.sqlite3"));
        assert_eq!(config.snapshots_directory(), data.join("snapshots"));
        assert_eq!(config.cache_directory(), cache);
    }

    #[test]
    fn empty_history_json_is_versioned_at_every_object() {
        let document = HistoryDocument::try_from_engine(
            DatabaseStatus {
                schema_version: DATABASE_SCHEMA_VERSION,
                access: DatabaseAccess::ReadWriteCurrent,
            },
            20,
            RecentScanHistory {
                scans: Vec::new(),
                has_more: false,
            },
        )
        .unwrap();

        let json = serialize_json(&document).unwrap();
        assert_eq!(
            json,
            format!(
                "{{\n  \"schema_version\": 1,\n  \"command\": \"history\",\n  \"history_kind\": \"scan\",\n  \"path_disclosure\": \"none\",\n  \"order\": \"started_at_desc_scan_id_asc\",\n  \"database\": {{\n    \"schema_version\": 1,\n    \"database_schema_version\": {0},\n    \"supported_database_schema_version\": {0},\n    \"access\": \"read_write_current\"\n  }},\n  \"requested_limit\": 20,\n  \"has_more\": false,\n  \"items\": []\n}}",
                DATABASE_SCHEMA_VERSION
            )
        );
    }

    #[test]
    fn empty_status_json_distinguishes_available_history_from_no_scans() {
        let document = StatusDocument::try_from_engine(
            DatabaseStatus {
                schema_version: DATABASE_SCHEMA_VERSION,
                access: DatabaseAccess::ReadWriteCurrent,
            },
            None,
        )
        .unwrap();

        let json = serialize_json(&document).unwrap();
        assert_eq!(
            json,
            format!(
                "{{\n  \"schema_version\": 1,\n  \"command\": \"status\",\n  \"dux_version\": \"{}\",\n  \"path_disclosure\": \"none\",\n  \"database\": {{\n    \"schema_version\": 1,\n    \"database_schema_version\": {1},\n    \"supported_database_schema_version\": {1},\n    \"access\": \"read_write_current\"\n  }},\n  \"capabilities\": {{\n    \"schema_version\": 1,\n    \"scan_history\": \"available\"\n  }},\n  \"latest_scan\": null\n}}",
                env!("CARGO_PKG_VERSION"),
                DATABASE_SCHEMA_VERSION
            )
        );
    }

    #[test]
    fn populated_history_json_preserves_null_and_measured_semantics() {
        let started = UNIX_EPOCH + Duration::from_millis(1_750_000_000_001);
        let completed = started + Duration::from_millis(9);
        let history = RecentScanHistory {
            scans: vec![
                DurableScanSummary {
                    scan_id: dux_core::ScanId::new("scan:running").unwrap(),
                    started_at: started,
                    completed_at: None,
                    status: DurableScanStatus::Running,
                    counts: None,
                    coverage: unknown_coverage(),
                    snapshot_recorded: false,
                },
                DurableScanSummary {
                    scan_id: dux_core::ScanId::new("scan:succeeded").unwrap(),
                    started_at: started - Duration::from_millis(1),
                    completed_at: Some(completed),
                    status: DurableScanStatus::Succeeded,
                    counts: Some(DurableScanCounts {
                        directory_count: 2,
                        file_count: 3,
                        logical_bytes: 4,
                        allocated_bytes: None,
                    }),
                    coverage: unknown_coverage(),
                    snapshot_recorded: true,
                },
            ],
            has_more: true,
        };
        let document = HistoryDocument::try_from_engine(
            DatabaseStatus {
                schema_version: DATABASE_SCHEMA_VERSION,
                access: DatabaseAccess::ReadWriteCurrent,
            },
            2,
            history,
        )
        .unwrap();

        let json = serialize_json(&document).unwrap();
        assert_eq!(
            json,
            format!(
                "{{\n  \"schema_version\": 1,\n  \"command\": \"history\",\n  \"history_kind\": \"scan\",\n  \"path_disclosure\": \"none\",\n  \"order\": \"started_at_desc_scan_id_asc\",\n  \"database\": {{\n    \"schema_version\": 1,\n    \"database_schema_version\": {0},\n    \"supported_database_schema_version\": {0},\n    \"access\": \"read_write_current\"\n  }},\n  \"requested_limit\": 2,\n  \"has_more\": true,\n  \"items\": [\n    {{\n      \"schema_version\": 1,\n      \"scan_id\": \"scan:running\",\n      \"started_at_unix_ms\": 1750000000001,\n      \"completed_at_unix_ms\": null,\n      \"status\": \"running\",\n      \"counts\": null,\n      \"coverage\": {{\n        \"schema_version\": 1,\n        \"status\": \"unknown\",\n        \"measured_permille\": null,\n        \"issue_record_count\": 0,\n        \"issue_occurrence_count\": 0\n      }},\n      \"snapshot_recorded\": false\n    }},\n    {{\n      \"schema_version\": 1,\n      \"scan_id\": \"scan:succeeded\",\n      \"started_at_unix_ms\": 1750000000000,\n      \"completed_at_unix_ms\": 1750000000010,\n      \"status\": \"succeeded\",\n      \"counts\": {{\n        \"schema_version\": 1,\n        \"directory_count\": 2,\n        \"file_count\": 3,\n        \"logical_bytes\": 4,\n        \"allocated_bytes\": null\n      }},\n      \"coverage\": {{\n        \"schema_version\": 1,\n        \"status\": \"unknown\",\n        \"measured_permille\": null,\n        \"issue_record_count\": 0,\n        \"issue_occurrence_count\": 0\n      }},\n      \"snapshot_recorded\": true\n    }}\n  ]\n}}",
                DATABASE_SCHEMA_VERSION
            )
        );
    }

    #[test]
    fn newer_database_status_reports_history_as_unavailable() {
        let document = StatusDocument::try_from_engine(
            DatabaseStatus {
                schema_version: DATABASE_SCHEMA_VERSION + 1,
                access: DatabaseAccess::ReadOnlyNewer {
                    found: DATABASE_SCHEMA_VERSION + 1,
                    supported: DATABASE_SCHEMA_VERSION,
                },
            },
            None,
        )
        .unwrap();

        let json = serialize_json(&document).unwrap();
        assert_eq!(
            json,
            format!(
                "{{\n  \"schema_version\": 1,\n  \"command\": \"status\",\n  \"dux_version\": \"{}\",\n  \"path_disclosure\": \"none\",\n  \"database\": {{\n    \"schema_version\": 1,\n    \"database_schema_version\": {},\n    \"supported_database_schema_version\": {},\n    \"access\": \"read_only_newer\"\n  }},\n  \"capabilities\": {{\n    \"schema_version\": 1,\n    \"scan_history\": \"unavailable_newer_schema\"\n  }},\n  \"latest_scan\": null\n}}",
                env!("CARGO_PKG_VERSION"),
                DATABASE_SCHEMA_VERSION + 1,
                DATABASE_SCHEMA_VERSION
            )
        );
    }

    #[test]
    fn invalid_public_timestamp_fails_instead_of_panicking_or_clamping() {
        let before_epoch = UNIX_EPOCH.checked_sub(Duration::from_millis(1)).unwrap();
        let scan = DurableScanSummary {
            scan_id: dux_core::ScanId::new("scan:invalid-time").unwrap(),
            started_at: before_epoch,
            completed_at: None,
            status: DurableScanStatus::Running,
            counts: None,
            coverage: unknown_coverage(),
            snapshot_recorded: false,
        };

        let error = ScanDocument::try_from(scan).err().unwrap();
        assert_eq!(error.code.as_str(), "internal_error");
    }

    #[test]
    fn structured_errors_are_path_free_and_versioned() {
        let error = CommandError::unsafe_storage();
        let document = ErrorDocument {
            schema_version: JSON_SCHEMA_VERSION,
            command: "history",
            error: ErrorBody {
                schema_version: JSON_SCHEMA_VERSION,
                code: error.code.as_str(),
                message: error.message,
                retryable: error.code.retryable(),
            },
        };

        let json = serde_json::to_string(&document).unwrap();
        assert_eq!(
            json,
            "{\"schema_version\":1,\"command\":\"history\",\"error\":{\"schema_version\":1,\"code\":\"unsafe_storage\",\"message\":\"DUX refused to use storage that did not meet its safety requirements.\",\"retryable\":false}}"
        );
        assert_eq!(error.code.exit_code(), 3);
    }
}

//! Owned, versioned UniFFI boundary for the DUX macOS application.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dux_core::engine::{
    CancelOutcome as CoreCancelOutcome,
    CandidateEvaluationTaskFailureKind as CoreCandidateEvaluationFailure,
    CandidateEvaluationTaskStatus as CoreCandidateEvaluationStatus,
    CapacityHistoryDisposition as CoreHistoryDisposition, DiskPressurePolicy as CorePressurePolicy,
    DiskPressurePolicyError as CorePressurePolicyError,
    DiskPressurePolicySource as CorePressurePolicySource,
    DiskPressurePolicyUpdate as CorePressurePolicyUpdate,
    DurableScanIssueKind as CoreDurableScanIssueKind, DurableScanStatus as CoreDurableScanStatus,
    EngineConfig, EngineHandle, EngineOpenError, HistoryMaintenanceStartOutcome,
    ScanCoverageDetailsError as CoreScanCoverageDetailsError,
    ScanHistoryError as CoreScanHistoryError,
    ScanRecoveryMaintenanceOutcome as CoreScanRecoveryOutcome, ScanRecoveryMaintenanceStartOutcome,
    ScanRootErrorKind, ScanTaskResult as CoreScanTaskResult, ScanTaskStatus as CoreScanTaskStatus,
    SnapshotOrphanMaintenanceOutcome as CoreOrphanOutcome, SnapshotOrphanMaintenanceStartOutcome,
    SnapshotProvisioningStageMaintenanceOutcome as CoreStageOutcome,
    SnapshotProvisioningStageMaintenanceStartOutcome,
    SnapshotRetentionOutcome as CoreRetentionOutcome, SnapshotRetentionStartOutcome,
    SnapshotReviewCategory as CoreReviewCategory, SnapshotReviewError as CoreReviewError,
    SnapshotReviewLargeFile as CoreReviewLargeFile,
    SnapshotReviewLargeFilePage as CoreReviewLargeFilePage,
    SnapshotReviewLiveTarget as CoreReviewLiveTarget,
    SnapshotReviewLiveTargetKind as CoreReviewLiveTargetKind,
    SnapshotReviewLiveTargetPurpose as CoreReviewLiveTargetPurpose,
    SnapshotReviewNameEncoding as CoreReviewNameEncoding, SnapshotReviewNode as CoreReviewNode,
    SnapshotReviewNodeKind as CoreReviewNodeKind, SnapshotReviewNodePage as CoreReviewNodePage,
    SnapshotReviewNodeSort as CoreReviewNodeSort,
    SnapshotReviewReleaseOutcome as CoreReviewReleaseOutcome,
    SnapshotReviewSession as CoreReviewSession, SnapshotReviewTimestamp as CoreReviewTimestamp,
    SnapshotReviewTreemap as CoreReviewTreemap, SnapshotReviewTreemapCell as CoreReviewTreemapCell,
    SnapshotTerminalTempMaintenanceOutcome as CoreTerminalTempOutcome,
    SnapshotTerminalTempMaintenanceStartOutcome,
    SnapshotUnleasedTempMaintenanceOutcome as CoreUnleasedTempOutcome,
    SnapshotUnleasedTempMaintenanceStartOutcome, StartSubtreeScanError, StartTaskError,
    TaskAccessError, TaskEventBatch as CoreTaskEventBatch, TaskEventKind as CoreTaskEventKind,
    TaskFailureKind, TaskId, TaskKind as CoreTaskKind, TaskPhase as CoreTaskPhase,
    VolumeCapacityObservation as CoreVolumeObservation,
    VolumeCapacityStatusError as CoreVolumeStatusError,
};
use dux_core::{
    AvailableCapacitySource as CoreCapacitySource, DatabaseOpenErrorKind,
    DiskPressure as CoreDiskPressure, DiskPressureConfig, DiskPressureConfigError,
    DiskPressureRecoveryMargin, DiskPressureThreshold, ScanCoverageStatus as CoreCoverageStatus,
    ScanId, SnapshotOpenErrorKind, VolumeCapacity, VolumeId,
};

const FFI_CONTRACT_VERSION: u32 = 16;
const FFI_RECORD_VERSION: u32 = 1;
const SNAPSHOT_NODE_RECORD_VERSION: u32 = 2;
const SCAN_EVENT_PAGE_LIMIT: u16 = 64;
const RECENT_SCAN_HISTORY_PAGE_LIMIT: u16 = 200;
const SCAN_COVERAGE_DETAIL_PAGE_LIMIT: u16 = 64;
const MAX_SCAN_ROOT_UTF8_BYTES: usize = 32 * 1_024;
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
static LIVE_ENGINE_INSTANCE_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LibraryVersion {
    pub library_version: String,
    pub ffi_contract_version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FormattedSize {
    pub bytes: u64,
    pub display: String,
}

/// Foundation-derived startup-volume facts. The mount path is intentionally
/// fixed inside this adapter and never crosses the FFI boundary.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StartupVolumeObservation {
    pub record_version: u32,
    pub stable_volume_id: Option<String>,
    pub display_name: Option<String>,
    pub filesystem: Option<String>,
    pub is_internal: Option<bool>,
    pub is_removable: Option<bool>,
    pub sampled_at_unix_ms: i64,
    pub total_bytes: u64,
    pub ordinary_available_bytes: Option<u64>,
    pub important_available_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum VolumePressure {
    Healthy,
    Warning,
    Critical,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum VolumeCapacitySource {
    ImportantUsage,
    Ordinary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum VolumeHistoryDisposition {
    Stored,
    ExistingExact,
    SuppressedByHourlyCadence,
    NotStoredMissingOrdinaryAvailability,
    NotStoredMissingStableIdentity,
    NotStoredIncompleteMetadata,
}

/// Canonical path-free result. It is presentation telemetry and carries no
/// cleanup target or authority.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StartupVolumeStatus {
    pub record_version: u32,
    pub stable_volume_id: Option<String>,
    pub sampled_at_unix_ms: i64,
    pub total_bytes: u64,
    pub ordinary_available_bytes: Option<u64>,
    pub important_available_bytes: Option<u64>,
    pub headline_available_bytes: u64,
    pub headline_source: VolumeCapacitySource,
    pub pressure: VolumePressure,
    pub previous_durable_pressure: Option<VolumePressure>,
    pub critical_boundary_bytes: u64,
    pub warning_boundary_bytes: u64,
    pub history_disposition: VolumeHistoryDisposition,
}

/// Exact integer policy input. Basis points retain two decimal percentage
/// places without floating-point conversion.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PressurePolicyInput {
    pub record_version: u32,
    pub critical_available_bytes: u64,
    pub critical_available_basis_points: u16,
    pub warning_available_bytes: u64,
    pub warning_available_basis_points: u16,
    pub recovery_bytes: u64,
    pub recovery_basis_points: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PressurePolicySource {
    Default,
    Stored,
}

/// Versioned, path-free effective pressure policy.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PressurePolicyStatus {
    pub record_version: u32,
    pub source: PressurePolicySource,
    pub revision: u64,
    pub critical_available_bytes: u64,
    pub critical_available_basis_points: u16,
    pub warning_available_bytes: u64,
    pub warning_available_basis_points: u16,
    pub recovery_bytes: u64,
    pub recovery_basis_points: u16,
    pub updated_at_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PressurePolicyUpdate {
    pub record_version: u32,
    pub policy: PressurePolicyStatus,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum PressurePolicyError {
    #[error("engine session is closed")]
    Closed,
    #[error("pressure policy record version is unsupported")]
    InvalidRecordVersion,
    #[error("a pressure threshold byte value must be positive")]
    ThresholdBytesZero,
    #[error("pressure threshold basis points are outside 1 through 10000")]
    ThresholdBasisPointsOutOfRange,
    #[error("the warning byte threshold is below the critical threshold")]
    WarningBytesBelowCritical,
    #[error("the warning percentage threshold is below the critical threshold")]
    WarningBasisPointsBelowCritical,
    #[error("warning and critical thresholds are identical")]
    WarningThresholdMatchesCritical,
    #[error("the pressure recovery byte margin must be positive")]
    RecoveryBytesZero,
    #[error("pressure recovery basis points are outside 1 through 10000")]
    RecoveryBasisPointsOutOfRange,
    #[error("the pressure policy revision cannot advance")]
    RevisionExhausted,
    #[error("the system clock cannot be represented")]
    InvalidClock,
    #[error("the durable schema is incompatible")]
    IncompatibleSchema,
    #[error("the durable store is temporarily busy")]
    Busy,
    #[error("storage failed its safety checks")]
    UnsafeStorage,
    #[error("the bounded settings query exceeded its budget")]
    BudgetExceeded,
    #[error("disk-pressure settings are corrupt")]
    CorruptData,
    #[error("disk-pressure settings are unavailable")]
    Unavailable,
    #[error("the settings write outcome is unknown")]
    OutcomeUnknown,
    #[error("internal pressure-policy state is invalid")]
    InternalState,
}

/// Input-only adapter storage roots. No path is returned by this contract.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EngineStorageRoots {
    pub data_root: String,
    pub cache_root: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum EngineError {
    #[error("engine session is closed")]
    Closed,
    #[error("engine storage configuration is invalid")]
    InvalidStorage,
    #[error("engine storage is unavailable")]
    StorageUnavailable,
    #[error("engine task registry is unavailable")]
    RegistryUnavailable,
    #[error("scan identity is invalid")]
    InvalidScanId,
    #[error("volume capacity observation is invalid")]
    InvalidCapacityObservation,
    #[error("a different capacity observation already exists at this time")]
    ConflictingCapacityObservation,
    #[error("a newer capacity observation already exists")]
    SupersededCapacityObservation,
    #[error("scan does not exist")]
    ScanNotFound,
    #[error("scan snapshot is unavailable")]
    SnapshotUnavailable,
    #[error("snapshot review lease expired")]
    ReviewExpired,
    #[error("snapshot node does not exist")]
    SnapshotNodeNotFound,
    #[error("snapshot node is not a directory")]
    SnapshotNodeNotDirectory,
    #[error("snapshot node page is invalid")]
    InvalidSnapshotNodePage,
    #[error("snapshot treemap cell budget is invalid")]
    InvalidSnapshotTreemapBudget,
    #[error("snapshot large-file request is invalid")]
    InvalidSnapshotLargeFileRequest,
    #[error("snapshot live-target request is invalid")]
    InvalidSnapshotLiveTargetRequest,
    #[error("the requested platform action is unsupported for this snapshot item")]
    SnapshotLiveTargetUnsupported,
    #[error("the snapshot item has no safely usable current path")]
    SnapshotLivePathUnavailable,
    #[error("the snapshot item no longer exists at its observed location")]
    SnapshotLivePathMissing,
    #[error("the snapshot item or one of its ancestors is now a symbolic link")]
    SnapshotLivePathSymlink,
    #[error("the snapshot item now crosses a filesystem boundary")]
    SnapshotLivePathCrossVolume,
    #[error("the current filesystem item no longer matches the snapshot")]
    SnapshotLivePathChanged,
    #[error("the current filesystem item cannot be inspected")]
    SnapshotLivePathAccessDenied,
    #[error("scan coverage detail request is invalid")]
    InvalidScanCoverageDetailsRequest,
    #[error("durable store is read-only")]
    ReadOnlyStore,
    #[error("durable schema is incompatible")]
    IncompatibleSchema,
    #[error("operation is temporarily busy")]
    Busy,
    #[error("storage failed its safety checks")]
    UnsafeStorage,
    #[error("bounded operation exceeded its budget")]
    BudgetExceeded,
    #[error("durable state is corrupt")]
    CorruptData,
    #[error("snapshot format is incompatible")]
    IncompatibleSnapshot,
    #[error("operation outcome is unknown")]
    OutcomeUnknown,
    #[error("internal engine state is invalid")]
    InternalState,
}

/// Versioned read-only discovery request. `root` is input scope only: it is
/// validated by the core and is never returned as cleanup authority.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScanRequest {
    pub record_version: u32,
    pub root: String,
}

/// Versioned, path-free request to rescan one directory selected from an exact
/// retained review. Rust resolves and revalidates the node; names and paths are
/// never accepted from Swift.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SubtreeScanRequest {
    pub record_version: u32,
    pub node_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScanStartDisposition {
    Started,
    AlreadyActive,
}

#[derive(Clone, uniffi::Record)]
pub struct ScanStart {
    pub record_version: u32,
    pub disposition: ScanStartDisposition,
    pub task: Arc<ScanTask>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScanStage {
    Queued,
    Scanning,
    Finalizing,
    Evaluating,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScanTerminalStatus {
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScanCoverageStatus {
    Unknown,
    Complete,
    LimitedAccess,
    Partial,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScanCoverageSummary {
    pub record_version: u32,
    pub status: ScanCoverageStatus,
    pub measured_permille: Option<u16>,
    pub issue_record_count: u64,
    pub issue_occurrence_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum HistoricalScanStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HistoricalScanCounts {
    pub directory_count: u64,
    pub file_count: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
}

/// Path-free durable scan metadata used to choose an Explorer review target.
/// `snapshot_recorded` is only discovery evidence; acquiring the review lease
/// repeats snapshot availability and safety validation.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HistoricalScanSummary {
    pub record_version: u32,
    pub scan_id: String,
    pub started_at_unix_ms: i64,
    pub completed_at_unix_ms: Option<i64>,
    pub status: HistoricalScanStatus,
    pub counts: Option<HistoricalScanCounts>,
    pub coverage: ScanCoverageSummary,
    pub snapshot_recorded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RecentScanHistoryPage {
    pub record_version: u32,
    pub scans: Vec<HistoricalScanSummary>,
    pub has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum HistoricalScanIssueKind {
    PermissionDenied,
    TimedOut,
    DifferentFilesystem,
    NetworkOrVirtualFilesystem,
    SymlinkSkipped,
    FileChangedDuringScan,
    MetadataError,
    Cancelled,
    PolicyExcluded,
    DepthLimited,
    ProbePoolExhausted,
    FilesystemBoundaryUnknown,
    IssueLimitReached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum HistoricalScanIssueLocationScope {
    Global,
    ScanRoot,
    Descendant,
}

/// One historical coverage observation. Location components are bounded,
/// root-relative display context and never a live path or cleanup capability.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HistoricalScanIssue {
    pub record_version: u32,
    pub ordinal: u16,
    pub kind: HistoricalScanIssueKind,
    pub occurrence_count: u32,
    pub location_scope: HistoricalScanIssueLocationScope,
    pub location_components: Vec<String>,
    pub location_truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScanCoverageDetailsPage {
    pub record_version: u32,
    pub scan_id: String,
    pub coverage: ScanCoverageSummary,
    pub offset: u16,
    pub total_issue_records: u16,
    pub total_issue_occurrences: u64,
    pub has_more: bool,
    pub issues: Vec<HistoricalScanIssue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScanCoverageDetailsRequest {
    pub record_version: u32,
    pub offset: u16,
    pub limit: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScanCandidateEvaluationStatus {
    NotRun,
    Succeeded,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScanCandidateEvaluationFailure {
    Cancelled,
    CatalogInvalid,
    ContextInvalid,
    EvaluationFailed,
    CandidateInvalid,
    LimitExceeded,
    InternalState,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScanCandidateEvaluationSummary {
    pub record_version: u32,
    pub status: ScanCandidateEvaluationStatus,
    pub candidate_count: u32,
    pub failure: Option<ScanCandidateEvaluationFailure>,
}

/// Frozen, path-free terminal summary. It is discovery history only and does
/// not carry a target, plan, approval, or cleanup capability.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScanTaskResult {
    pub record_version: u32,
    pub scan_id: String,
    pub started_at_unix_ms: i64,
    pub completed_at_unix_ms: i64,
    pub status: ScanTerminalStatus,
    pub directory_count: u64,
    pub file_count: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub snapshot_available: bool,
    pub coverage: ScanCoverageSummary,
    pub candidate_evaluation: ScanCandidateEvaluationSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScanTaskFailure {
    RootChanged,
    ScanFailed,
    SnapshotRejected,
    PersistenceUnavailable,
    PersistenceOutcomeUnknown,
    InternalState,
}

/// Latest measured scanner heartbeat. Its absence means that no aggregate
/// progress event has been observed; it must not be presented as measured zero.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScanProgress {
    pub record_version: u32,
    pub files_scanned: u64,
    pub directories_scanned: u64,
    pub known_allocated_bytes: u64,
    pub error_count: u64,
}

/// One path-free aggregate observation. The task retains its event cursor
/// privately; `events_truncated` records whether any aggregate events were
/// already gone from the bounded core ring before this object observed them.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScanPoll {
    pub record_version: u32,
    pub phase: TaskPhase,
    pub stage: ScanStage,
    pub cancellation_requested: bool,
    pub revision: u64,
    pub progress: Option<ScanProgress>,
    pub events_truncated: bool,
    pub failure: Option<ScanTaskFailure>,
    pub result: Option<ScanTaskResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScanCancelOutcome {
    CancelledBeforeStart,
    Requested,
    AlreadyRequested,
    AlreadyTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ScanError {
    #[error("engine session is closed")]
    Closed,
    #[error("scan request record version is unsupported")]
    InvalidRecordVersion,
    #[error("snapshot review belongs to a different engine session")]
    ForeignReview,
    #[error("snapshot review lease is released or expired")]
    ReviewExpired,
    #[error("snapshot review is unavailable")]
    ReviewUnavailable,
    #[error("snapshot node does not exist")]
    SnapshotNodeNotFound,
    #[error("snapshot node is not a directory")]
    SnapshotNodeNotDirectory,
    #[error("scan root must be an absolute discovery scope")]
    InvalidRoot,
    #[error("scan root does not exist")]
    RootMissing,
    #[error("scan root is not accessible")]
    RootAccessDenied,
    #[error("scan root is not a directory")]
    RootNotDirectory,
    #[error("scan root is a symbolic link")]
    RootSymlink,
    #[error("scan root changed while it was validated")]
    RootChanged,
    #[error("scan root identity is unavailable")]
    RootIdentityUnavailable,
    #[error("scan root is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("scan root is temporarily unavailable")]
    RootUnavailable,
    #[error("engine task queue is full")]
    QueueFull,
    #[error("an overlapping scan scope is already active")]
    Busy,
    #[error("scan request exceeds the engine input bound")]
    InputTooLarge,
    #[error("durable store is read-only")]
    ReadOnlyStore,
    #[error("durable scan storage is unavailable")]
    StorageUnavailable,
    #[error("engine task registry is unavailable")]
    RegistryUnavailable,
    #[error("scan task is unknown or no longer retained")]
    TaskUnavailable,
    #[error("scan event history is internally inconsistent")]
    EventHistoryUnavailable,
    #[error("opaque task does not refer to a scan")]
    WrongTaskKind,
    #[error("internal scan bridge state is invalid")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MaintenanceKind {
    ScanRecovery,
    History,
    SnapshotRetention,
    SnapshotOrphan,
    SnapshotProvisioningStage,
    SnapshotTerminalTemp,
    SnapshotUnleasedTemp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MaintenanceStartDisposition {
    Started,
    AlreadyActive,
    DeferredBusy,
}

#[derive(Clone, uniffi::Record)]
pub struct MaintenanceStart {
    pub record_version: u32,
    pub disposition: MaintenanceStartDisposition,
    pub task: Option<Arc<MaintenanceTask>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TaskPhase {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MaintenanceFailure {
    InvalidClock,
    IncompatibleSchema,
    Busy,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    IncompatibleSnapshot,
    Unavailable,
    OutcomeUnknown,
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MaintenanceOutcome {
    ScanRecoveryNone,
    ScanRecoveryDeferredUnproven,
    ScanRecoveryInterrupted,
    ScanRecoveryChangedConcurrently,
    HistoryApplied,
    RetentionUnderCap,
    RetentionDeferredUnstable,
    RetentionDeferredNoEligibleSnapshot,
    RetentionRemovedTombstonedResidual,
    RetentionTombstonedAndRemoved,
    OrphanNone,
    OrphanRemoved,
    StageNone,
    StageDeferredUnproven,
    StageRemovedMarkerOnly,
    StageRemovedMarkerComplete,
    TerminalTempNone,
    TerminalTempDeferredActive,
    TerminalTempReconciledRowOnly,
    TerminalTempRemoved,
    UnleasedTempNone,
    UnleasedTempDeferredActive,
    UnleasedTempRemoved,
}

/// One path-free terminal observation. Fields not used by a kind are zero.
/// Their meanings are fixed by `kind` and `outcome`; no field carries cleanup
/// authority. History uses the four `*_count_after` fields for created daily
/// rollups, pruned raw samples, pruned daily rollups, and pruned AI insights.
/// Scan recovery uses primary before/after for the inspected claimed-running
/// page, then secondary/tertiary/quaternary before for alive, unknown, and
/// definitely gone owners; process identities never cross this boundary.
/// Snapshot residual kinds use count pairs for their documented inventories;
/// byte fields always contain bytes and never row counts.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MaintenanceResult {
    pub record_version: u32,
    pub kind: MaintenanceKind,
    pub observed_at_unix_ms: i64,
    pub outcome: MaintenanceOutcome,
    pub primary_count_before: u64,
    pub primary_count_after: u64,
    pub secondary_count_before: u64,
    pub secondary_count_after: u64,
    pub tertiary_count_before: u64,
    pub tertiary_count_after: u64,
    pub quaternary_count_before: u64,
    pub quaternary_count_after: u64,
    pub charged_bytes_before: u64,
    pub charged_bytes_after: u64,
    pub removed_bytes: u64,
    pub cap_bytes: u64,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MaintenancePoll {
    pub record_version: u32,
    pub kind: MaintenanceKind,
    pub phase: TaskPhase,
    pub cancellation_requested: bool,
    pub revision: u64,
    pub failure: Option<MaintenanceFailure>,
    pub result: Option<MaintenanceResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MaintenanceCancelOutcome {
    CancelledBeforeStart,
    Requested,
    AlreadyRequested,
    AlreadyTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ReviewReleaseOutcome {
    Released,
    AlreadyReleased,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotReviewInfo {
    pub record_version: u32,
    pub scan_id: String,
    pub expires_at_unix_ms: i64,
    pub released: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SnapshotNodeSort {
    NameAscending,
    LogicalBytesDescending,
    AllocatedBytesDescending,
    ModifiedNewest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SnapshotNodeKind {
    Directory,
    File,
    Symlink,
    Other,
    Error,
}

/// Historical display classification only. This value carries no candidate,
/// safety, action, reclaimability, planning, AI, or cleanup authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SnapshotStorageCategory {
    Unclassified,
    DeveloperArtifact,
    ApplicationCache,
    BrowserCache,
    LogAndDiagnostic,
    InstallerAndDownload,
    DeviceAndSimulatorData,
    CloudFile,
    LargeReviewItem,
    ProtectedSystemData,
    UnknownStorage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SnapshotNameEncoding {
    UnixBytes,
    WindowsUtf16LittleEndian,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotNodeName {
    pub encoding: SnapshotNameEncoding,
    pub encoded_bytes: Vec<u8>,
    pub display: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotNodeTimestamp {
    pub seconds_since_unix_epoch: u64,
    pub nanoseconds: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotNodeScanFlags {
    pub inaccessible: bool,
    pub timed_out: bool,
    pub hard_link_duplicate: bool,
    pub mount_boundary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotNode {
    pub record_version: u32,
    pub id: u64,
    pub parent_id: Option<u64>,
    pub depth: u32,
    pub kind: SnapshotNodeKind,
    pub category: SnapshotStorageCategory,
    pub name: SnapshotNodeName,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub file_count: u64,
    pub child_count: u64,
    pub modified_at: Option<SnapshotNodeTimestamp>,
    pub accessed_at: Option<SnapshotNodeTimestamp>,
    pub scan_flags: SnapshotNodeScanFlags,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotNodePage {
    pub record_version: u32,
    pub parent_id: u64,
    pub offset: u64,
    pub total_children: u64,
    pub has_more: bool,
    pub nodes: Vec<SnapshotNode>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotTreemapCell {
    pub record_version: u32,
    pub node: SnapshotNode,
    pub logical_rank: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotTreemap {
    pub record_version: u32,
    pub parent_id: u64,
    pub total_children: u64,
    pub total_child_logical_bytes: u64,
    pub other_child_count: u64,
    pub other_logical_bytes: u64,
    pub zero_logical_child_count: u64,
    pub cells: Vec<SnapshotTreemapCell>,
}

/// Versioned, bounded historical large-file discovery input. It grants no
/// cleanup authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotLargeFileRequest {
    pub record_version: u32,
    pub minimum_logical_bytes: u64,
    pub modified_before: Option<SnapshotNodeTimestamp>,
    pub max_results: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotLargeFile {
    pub record_version: u32,
    pub node: SnapshotNode,
    /// Root-to-parent historical name components, excluding the scan root.
    pub parent_context: Vec<SnapshotNodeName>,
    pub context_truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotLargeFilePage {
    pub record_version: u32,
    pub total_matching_files: u64,
    pub total_matching_logical_bytes: u64,
    pub has_more: bool,
    pub files: Vec<SnapshotLargeFile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SnapshotLiveTargetPurpose {
    Reveal,
    CopyPath,
    QuickLook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SnapshotLiveTargetKind {
    Directory,
    File,
}

/// Versioned request for one user-initiated, read-only macOS platform action.
/// The snapshot-local node ID is not durable filesystem identity and no path
/// may be supplied by Swift.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotLiveTargetRequest {
    pub record_version: u32,
    pub node_id: u64,
    pub purpose: SnapshotLiveTargetPurpose,
}

/// A freshly validated current path for one immediate presentation action.
/// This value can become stale immediately, grants no cleanup authority, and
/// must not be persisted, logged, sent to AI, or reconstructed from node names.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotLiveTarget {
    pub record_version: u32,
    pub node_id: u64,
    pub purpose: SnapshotLiveTargetPurpose,
    pub kind: SnapshotLiveTargetKind,
    pub path_encoding: SnapshotNameEncoding,
    pub absolute_path_bytes: Vec<u8>,
    pub display_path: String,
    pub exact_text_path: Option<String>,
}

#[derive(uniffi::Object)]
pub struct SnapshotReviewSession {
    inner: Mutex<CoreReviewSession>,
    engine_closed: Arc<AtomicBool>,
}

#[uniffi::export]
impl SnapshotReviewSession {
    pub fn info(&self) -> Result<SnapshotReviewInfo, EngineError> {
        let session = self.inner.lock().map_err(|_| EngineError::InternalState)?;
        let released = session.is_released();
        Ok(SnapshotReviewInfo {
            record_version: FFI_RECORD_VERSION,
            scan_id: session.scan_id().as_str().to_owned(),
            expires_at_unix_ms: if released {
                0
            } else {
                session.expires_at_unix_ms().map_err(map_review_error)?
            },
            released,
        })
    }

    pub fn renew(&self) -> Result<SnapshotReviewInfo, EngineError> {
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(EngineError::Closed);
        }
        let mut session = self.inner.lock().map_err(|_| EngineError::InternalState)?;
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(EngineError::Closed);
        }
        session.renew_unix_ms().map_err(map_review_error)?;
        Ok(SnapshotReviewInfo {
            record_version: FFI_RECORD_VERSION,
            scan_id: session.scan_id().as_str().to_owned(),
            expires_at_unix_ms: session.expires_at_unix_ms().map_err(map_review_error)?,
            released: false,
        })
    }

    pub fn release(&self) -> Result<ReviewReleaseOutcome, EngineError> {
        self.release_inner()
    }

    pub fn root_node(&self) -> Result<SnapshotNode, EngineError> {
        self.with_open_session(|session| {
            session
                .root_node()
                .map(project_snapshot_node)
                .map_err(map_review_error)
        })
    }

    pub fn child_nodes(
        &self,
        parent_id: u64,
        sort: SnapshotNodeSort,
        offset: u64,
        limit: u16,
    ) -> Result<SnapshotNodePage, EngineError> {
        self.with_open_session(|session| {
            session
                .child_nodes(parent_id, map_snapshot_node_sort(sort), offset, limit)
                .map(project_snapshot_node_page)
                .map_err(map_review_error)
        })
    }

    pub fn treemap(&self, parent_id: u64, max_cells: u16) -> Result<SnapshotTreemap, EngineError> {
        self.with_open_session(|session| {
            session
                .treemap(parent_id, max_cells)
                .map(project_snapshot_treemap)
                .map_err(map_review_error)
        })
    }

    pub fn large_files(
        &self,
        request: SnapshotLargeFileRequest,
    ) -> Result<SnapshotLargeFilePage, EngineError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(EngineError::InvalidSnapshotLargeFileRequest);
        }
        self.with_open_session(|session| {
            session
                .large_files(
                    request.minimum_logical_bytes,
                    request
                        .modified_before
                        .map(|timestamp| CoreReviewTimestamp {
                            seconds_since_unix_epoch: timestamp.seconds_since_unix_epoch,
                            nanoseconds: timestamp.nanoseconds,
                        }),
                    request.max_results,
                )
                .map(project_snapshot_large_file_page)
                .map_err(map_review_error)
        })
    }

    pub fn resolve_live_target(
        &self,
        request: SnapshotLiveTargetRequest,
    ) -> Result<SnapshotLiveTarget, EngineError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(EngineError::InvalidSnapshotLiveTargetRequest);
        }
        self.with_open_session(|session| {
            let target = session
                .live_target(
                    request.node_id,
                    map_snapshot_live_target_purpose(request.purpose),
                )
                .map_err(map_review_error)?;
            project_snapshot_live_target(target)
        })
    }
}

impl SnapshotReviewSession {
    fn with_open_session<T>(
        &self,
        operation: impl FnOnce(&mut CoreReviewSession) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(EngineError::Closed);
        }
        let mut session = self.inner.lock().map_err(|_| EngineError::InternalState)?;
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(EngineError::Closed);
        }
        operation(&mut session)
    }

    fn release_inner(&self) -> Result<ReviewReleaseOutcome, EngineError> {
        let mut session = self.inner.lock().map_err(|_| EngineError::InternalState)?;
        match session.release().map_err(map_review_error)? {
            CoreReviewReleaseOutcome::Released => Ok(ReviewReleaseOutcome::Released),
            CoreReviewReleaseOutcome::AlreadyReleased => Ok(ReviewReleaseOutcome::AlreadyReleased),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ScanProgressState {
    event_cursor: u64,
    stage: ScanStage,
    has_progress: bool,
    files_scanned: u64,
    directories_scanned: u64,
    known_allocated_bytes: u64,
    error_count: u64,
    events_truncated: bool,
}

impl Default for ScanProgressState {
    fn default() -> Self {
        Self {
            event_cursor: 0,
            stage: ScanStage::Queued,
            has_progress: false,
            files_scanned: 0,
            directories_scanned: 0,
            known_allocated_bytes: 0,
            error_count: 0,
            events_truncated: false,
        }
    }
}

impl ScanProgressState {
    fn apply(&mut self, batch: CoreTaskEventBatch) {
        self.events_truncated |= batch.truncated;
        for event in batch.events {
            match event.kind {
                CoreTaskEventKind::Queued => self.stage = ScanStage::Queued,
                CoreTaskEventKind::Started => self.stage = ScanStage::Scanning,
                CoreTaskEventKind::ScanProgress {
                    files,
                    directories,
                    known_allocated_bytes,
                    errors,
                } => {
                    self.stage = ScanStage::Scanning;
                    self.has_progress = true;
                    self.files_scanned = files;
                    self.directories_scanned = directories;
                    self.known_allocated_bytes = known_allocated_bytes;
                    self.error_count = errors;
                }
                CoreTaskEventKind::ScanFinalizing => self.stage = ScanStage::Finalizing,
                CoreTaskEventKind::CandidateEvaluationStarted
                | CoreTaskEventKind::CandidateEvaluationFinished { .. } => {
                    self.stage = ScanStage::Evaluating;
                }
                CoreTaskEventKind::Terminal { .. } => self.stage = ScanStage::Terminal,
                CoreTaskEventKind::Progress { .. }
                | CoreTaskEventKind::CancellationRequested
                | CoreTaskEventKind::ScanRecoveryMaintenanceBatchApplying
                | CoreTaskEventKind::ScanRecoveryMaintenanceBatchFinished { .. }
                | CoreTaskEventKind::HistoryMaintenanceBatchApplying
                | CoreTaskEventKind::HistoryMaintenanceBatchFinished { .. }
                | CoreTaskEventKind::SnapshotRetentionBatchApplying
                | CoreTaskEventKind::SnapshotRetentionBatchFinished { .. }
                | CoreTaskEventKind::SnapshotOrphanMaintenanceBatchApplying
                | CoreTaskEventKind::SnapshotOrphanMaintenanceBatchFinished { .. }
                | CoreTaskEventKind::SnapshotProvisioningStageMaintenanceBatchApplying
                | CoreTaskEventKind::SnapshotProvisioningStageMaintenanceBatchFinished { .. }
                | CoreTaskEventKind::SnapshotTerminalTempMaintenanceBatchApplying
                | CoreTaskEventKind::SnapshotTerminalTempMaintenanceBatchFinished { .. }
                | CoreTaskEventKind::SnapshotUnleasedTempMaintenanceBatchApplying
                | CoreTaskEventKind::SnapshotUnleasedTempMaintenanceBatchFinished { .. } => {}
                _ => {}
            }
        }
        self.event_cursor = batch.next_sequence;
    }
}

#[derive(uniffi::Object)]
pub struct ScanTask {
    engine: EngineHandle,
    id: TaskId,
    progress: Mutex<ScanProgressState>,
}

#[uniffi::export]
impl ScanTask {
    pub fn poll(&self) -> Result<ScanPoll, ScanError> {
        let mut progress = self.progress.lock().map_err(|_| ScanError::InternalState)?;
        let events = self
            .engine
            .task_events(self.id, progress.event_cursor, SCAN_EVENT_PAGE_LIMIT)
            .map_err(map_scan_access_error)?;
        progress.apply(events);

        let snapshot = self
            .engine
            .task_snapshot(self.id)
            .map_err(map_scan_access_error)?;
        if snapshot.kind != CoreTaskKind::Scan {
            return Err(ScanError::WrongTaskKind);
        }
        if snapshot.phase == CoreTaskPhase::Running && progress.stage == ScanStage::Queued {
            progress.stage = ScanStage::Scanning;
        }
        if snapshot.phase.is_terminal() {
            progress.stage = ScanStage::Terminal;
        }

        let result = if snapshot.result_available {
            self.engine
                .scan_result(self.id)
                .map_err(map_scan_access_error)?
                .map(|result| scan_task_result(&result))
                .transpose()?
        } else {
            None
        };
        let failure = snapshot.failure.map(map_scan_task_failure);
        validate_scan_terminal_observation(snapshot.phase, failure, result.as_ref())?;

        Ok(ScanPoll {
            record_version: FFI_RECORD_VERSION,
            phase: map_phase(snapshot.phase),
            stage: progress.stage,
            cancellation_requested: snapshot.cancellation_requested,
            revision: snapshot.revision,
            progress: progress.has_progress.then_some(ScanProgress {
                record_version: FFI_RECORD_VERSION,
                files_scanned: progress.files_scanned,
                directories_scanned: progress.directories_scanned,
                known_allocated_bytes: progress.known_allocated_bytes,
                error_count: progress.error_count,
            }),
            events_truncated: progress.events_truncated,
            failure,
            result,
        })
    }

    pub fn cancel(&self) -> Result<ScanCancelOutcome, ScanError> {
        match self
            .engine
            .cancel_task(self.id)
            .map_err(map_scan_access_error)?
        {
            CoreCancelOutcome::CancelledBeforeStart => Ok(ScanCancelOutcome::CancelledBeforeStart),
            CoreCancelOutcome::Requested => Ok(ScanCancelOutcome::Requested),
            CoreCancelOutcome::AlreadyRequested => Ok(ScanCancelOutcome::AlreadyRequested),
            CoreCancelOutcome::AlreadyTerminal => Ok(ScanCancelOutcome::AlreadyTerminal),
        }
    }
}

#[derive(uniffi::Object)]
pub struct MaintenanceTask {
    engine: EngineHandle,
    id: TaskId,
    kind: MaintenanceKind,
}

#[uniffi::export]
impl MaintenanceTask {
    pub fn poll(&self) -> Result<MaintenancePoll, EngineError> {
        let snapshot = self
            .engine
            .task_snapshot(self.id)
            .map_err(map_task_access_error)?;
        let result = if snapshot.result_available {
            maintenance_result(&self.engine, self.id, self.kind)?
        } else {
            None
        };
        Ok(MaintenancePoll {
            record_version: FFI_RECORD_VERSION,
            kind: self.kind,
            phase: map_phase(snapshot.phase),
            cancellation_requested: snapshot.cancellation_requested,
            revision: snapshot.revision,
            failure: snapshot.failure.map(map_failure),
            result,
        })
    }

    pub fn cancel(&self) -> Result<MaintenanceCancelOutcome, EngineError> {
        match self
            .engine
            .cancel_task(self.id)
            .map_err(map_task_access_error)?
        {
            CoreCancelOutcome::CancelledBeforeStart => {
                Ok(MaintenanceCancelOutcome::CancelledBeforeStart)
            }
            CoreCancelOutcome::Requested => Ok(MaintenanceCancelOutcome::Requested),
            CoreCancelOutcome::AlreadyRequested => Ok(MaintenanceCancelOutcome::AlreadyRequested),
            CoreCancelOutcome::AlreadyTerminal => Ok(MaintenanceCancelOutcome::AlreadyTerminal),
        }
    }
}

enum EngineState {
    Open(EngineHandle),
    Closing,
    Closed { quiesced: bool },
}

#[derive(uniffi::Object)]
pub struct DuxEngine {
    state: Mutex<EngineState>,
    close_completed: Condvar,
    reviews: Mutex<Vec<Weak<SnapshotReviewSession>>>,
    closed: Arc<AtomicBool>,
}

#[uniffi::export]
impl DuxEngine {
    #[uniffi::constructor]
    pub fn new(storage: EngineStorageRoots) -> Result<Self, EngineError> {
        let data_root = PathBuf::from(storage.data_root);
        let cache_root = PathBuf::from(storage.cache_root);
        let config = EngineConfig::new(
            data_root.join("dux.sqlite3"),
            data_root.join("snapshots"),
            cache_root,
        )
        .map_err(|_| EngineError::InvalidStorage)?;
        let engine = EngineHandle::open(config).map_err(map_open_error)?;
        LIVE_ENGINE_INSTANCE_COUNT.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            state: Mutex::new(EngineState::Open(engine)),
            close_completed: Condvar::new(),
            reviews: Mutex::new(Vec::new()),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn library_version(&self) -> Result<LibraryVersion, EngineError> {
        self.with_engine(|_| Ok(library_version()))
    }

    pub fn format_size(&self, bytes: u64) -> Result<FormattedSize, EngineError> {
        self.with_engine(|_| {
            Ok(FormattedSize {
                bytes,
                display: dux_core::format_size(bytes),
            })
        })
    }

    pub fn observe_startup_volume(
        &self,
        observation: StartupVolumeObservation,
    ) -> Result<StartupVolumeStatus, EngineError> {
        if observation.record_version != FFI_RECORD_VERSION {
            return Err(EngineError::InvalidCapacityObservation);
        }
        let stable_volume_id = parse_macos_volume_id(observation.stable_volume_id)?;
        let sampled_at = unix_ms_to_system_time(observation.sampled_at_unix_ms)?;
        let capacity = VolumeCapacity::new(
            observation.total_bytes,
            observation.ordinary_available_bytes,
            observation.important_available_bytes,
        )
        .map_err(|_| EngineError::InvalidCapacityObservation)?;
        let core = CoreVolumeObservation::try_new(
            stable_volume_id,
            PathBuf::from("/"),
            observation.display_name,
            observation.filesystem,
            observation.is_internal,
            observation.is_removable,
            sampled_at,
            capacity,
        )
        .map_err(map_volume_status_error)?;
        self.with_engine(|engine| {
            let status = engine
                .observe_volume_capacity(core)
                .map_err(map_volume_status_error)?;
            Ok(StartupVolumeStatus {
                record_version: FFI_RECORD_VERSION,
                stable_volume_id: status.volume_id().map(ToString::to_string),
                sampled_at_unix_ms: system_time_ms(status.sampled_at())?,
                total_bytes: status.total_bytes(),
                ordinary_available_bytes: status.ordinary_available_bytes(),
                important_available_bytes: status.important_available_bytes(),
                headline_available_bytes: status.headline_available_bytes(),
                headline_source: map_capacity_source(status.headline_source()),
                pressure: map_volume_pressure(status.pressure()),
                previous_durable_pressure: status
                    .previous_durable_pressure()
                    .map(map_volume_pressure),
                critical_boundary_bytes: status.critical_boundary_bytes(),
                warning_boundary_bytes: status.warning_boundary_bytes(),
                history_disposition: map_history_disposition(status.history_disposition()),
            })
        })
    }

    pub fn get_disk_pressure_policy(&self) -> Result<PressurePolicyStatus, PressurePolicyError> {
        self.with_pressure_engine(|engine| {
            engine
                .disk_pressure_policy()
                .map_err(map_pressure_policy_error)
                .and_then(pressure_policy_status)
        })
    }

    pub fn set_disk_pressure_policy(
        &self,
        input: PressurePolicyInput,
    ) -> Result<PressurePolicyUpdate, PressurePolicyError> {
        let config = pressure_policy_config(input)?;
        self.with_pressure_engine(|engine| {
            engine
                .set_disk_pressure_policy(config)
                .map_err(map_pressure_policy_error)
                .and_then(pressure_policy_update)
        })
    }

    pub fn reset_disk_pressure_policy(&self) -> Result<PressurePolicyUpdate, PressurePolicyError> {
        self.with_pressure_engine(|engine| {
            engine
                .reset_disk_pressure_policy()
                .map_err(map_pressure_policy_error)
                .and_then(pressure_policy_update)
        })
    }

    pub fn start_scan(&self, request: ScanRequest) -> Result<ScanStart, ScanError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(ScanError::InvalidRecordVersion);
        }
        if request.root.len() > MAX_SCAN_ROOT_UTF8_BYTES {
            return Err(ScanError::InputTooLarge);
        }
        if request.root.is_empty() || request.root.chars().any(char::is_control) {
            return Err(ScanError::InvalidRoot);
        }
        let root = PathBuf::from(request.root);
        if !root.is_absolute() {
            return Err(ScanError::InvalidRoot);
        }
        self.with_scan_engine(|engine| match engine.start_scan(root) {
            Ok(id) => Ok(scan_start(engine, id, ScanStartDisposition::Started)),
            Err(StartTaskError::ScanAlreadyActive { existing }) => Ok(scan_start(
                engine,
                existing,
                ScanStartDisposition::AlreadyActive,
            )),
            Err(StartTaskError::ScanScopeBusy) => Err(ScanError::Busy),
            Err(error) => Err(map_scan_start_error(error)),
        })
    }

    /// Start a standalone immutable scan rooted at one directory from this
    /// engine's exact Explorer review. The resolved current path remains
    /// sealed inside Rust and the returned task uses the ordinary scan poll and
    /// cancellation contract.
    pub fn start_subtree_scan(
        &self,
        review: Arc<SnapshotReviewSession>,
        request: SubtreeScanRequest,
    ) -> Result<ScanStart, ScanError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(ScanError::InvalidRecordVersion);
        }
        if !Arc::ptr_eq(&review.engine_closed, &self.closed) {
            return Err(ScanError::ForeignReview);
        }
        let state = self.state.lock().map_err(|_| ScanError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(ScanError::Closed);
        };
        if self.closed.load(Ordering::Acquire) {
            return Err(ScanError::Closed);
        }
        let mut core_review = review.inner.lock().map_err(|_| ScanError::InternalState)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(ScanError::Closed);
        }
        match engine.start_subtree_scan(&mut core_review, request.node_id) {
            Ok(id) => Ok(scan_start(engine, id, ScanStartDisposition::Started)),
            // A subtree request is bound to exact historical identity. Never
            // attach it to an arbitrary same-path task.
            Err(StartSubtreeScanError::Task(
                StartTaskError::ScanAlreadyActive { .. } | StartTaskError::ScanScopeBusy,
            )) => Err(ScanError::Busy),
            Err(error) => Err(map_subtree_scan_start_error(error)),
        }
    }

    /// Return a bounded, newest-first page of durable scan metadata for
    /// Explorer selection. Paths and snapshot contents remain sealed; a
    /// selected snapshot must still be opened through a review lease.
    pub fn recent_scan_history(&self, limit: u16) -> Result<RecentScanHistoryPage, EngineError> {
        if !(1..=RECENT_SCAN_HISTORY_PAGE_LIMIT).contains(&limit) {
            return Err(EngineError::BudgetExceeded);
        }
        self.with_engine(|engine| {
            let history = engine
                .recent_scan_history(usize::from(limit))
                .map_err(map_scan_history_error)?;
            let scans = history
                .scans
                .iter()
                .map(historical_scan_summary)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(RecentScanHistoryPage {
                record_version: FFI_RECORD_VERSION,
                scans,
                has_more: history.has_more,
            })
        })
    }

    /// Return one exact, bounded page of durable coverage issues. This reads
    /// history metadata only and remains available without a retained snapshot.
    pub fn scan_coverage_details(
        &self,
        scan_id: String,
        request: ScanCoverageDetailsRequest,
    ) -> Result<ScanCoverageDetailsPage, EngineError> {
        if request.record_version != FFI_RECORD_VERSION
            || !(1..=SCAN_COVERAGE_DETAIL_PAGE_LIMIT).contains(&request.limit)
        {
            return Err(EngineError::InvalidScanCoverageDetailsRequest);
        }
        let scan_id = ScanId::new(scan_id).map_err(|_| EngineError::InvalidScanId)?;
        self.with_engine(|engine| {
            let page = engine
                .scan_coverage_details(&scan_id, request.offset, request.limit)
                .map_err(map_scan_coverage_details_error)?;
            Ok(ScanCoverageDetailsPage {
                record_version: FFI_RECORD_VERSION,
                scan_id: page.scan_id().as_str().to_owned(),
                coverage: ScanCoverageSummary {
                    record_version: FFI_RECORD_VERSION,
                    status: map_scan_coverage_status(page.status()),
                    measured_permille: page.measured_permille().map(|value| value.get()),
                    issue_record_count: u64::from(page.total_issue_records()),
                    issue_occurrence_count: page.total_issue_occurrences(),
                },
                offset: page.offset(),
                total_issue_records: page.total_issue_records(),
                total_issue_occurrences: page.total_issue_occurrences(),
                has_more: page.has_more(),
                issues: page
                    .issues()
                    .iter()
                    .map(historical_scan_issue)
                    .collect::<Result<Vec<_>, _>>()?,
            })
        })
    }

    pub fn acquire_explorer_snapshot_review(
        &self,
        scan_id: String,
    ) -> Result<Arc<SnapshotReviewSession>, EngineError> {
        let scan_id = ScanId::new(scan_id).map_err(|_| EngineError::InvalidScanId)?;
        let state = self.state.lock().map_err(|_| EngineError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(EngineError::Closed);
        };
        let session = engine
            .acquire_explorer_snapshot_review(&scan_id)
            .map_err(map_review_error)?;
        self.register_snapshot_review(session)
    }

    pub fn acquire_latest_explorer_snapshot_review(
        &self,
    ) -> Result<Arc<SnapshotReviewSession>, EngineError> {
        let state = self.state.lock().map_err(|_| EngineError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(EngineError::Closed);
        };
        let session = engine
            .acquire_latest_explorer_snapshot_review()
            .map_err(map_review_error)?;
        self.register_snapshot_review(session)
    }

    pub fn start_maintenance(
        &self,
        kind: MaintenanceKind,
    ) -> Result<MaintenanceStart, EngineError> {
        self.with_engine(|engine| start_maintenance(engine, kind))
    }

    /// Close the engine and wait for at most five seconds for worker quiescence.
    /// Returns whether all workers have quiesced; repeated calls return the
    /// first call's final observation without reopening storage.
    pub fn close(&self) -> bool {
        self.closed.store(true, Ordering::Release);
        let engine = loop {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &*state {
                EngineState::Open(_) => {
                    let EngineState::Open(engine) =
                        std::mem::replace(&mut *state, EngineState::Closing)
                    else {
                        unreachable!("open state was just matched")
                    };
                    break engine;
                }
                EngineState::Closing => {
                    let state = self
                        .close_completed
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let EngineState::Closed { quiesced } = *state {
                        return quiesced;
                    }
                }
                EngineState::Closed { quiesced } => return *quiesced,
            }
        };
        self.release_registered_reviews();
        engine.close();
        let quiesced = engine.wait_until_closed(CLOSE_TIMEOUT);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *state = EngineState::Closed { quiesced };
        self.close_completed.notify_all();
        quiesced
    }
}

impl DuxEngine {
    fn register_snapshot_review(
        &self,
        session: CoreReviewSession,
    ) -> Result<Arc<SnapshotReviewSession>, EngineError> {
        let review = Arc::new(SnapshotReviewSession {
            inner: Mutex::new(session),
            engine_closed: Arc::clone(&self.closed),
        });
        if self.closed.load(Ordering::Acquire) {
            let _ = review.release_inner();
            return Err(EngineError::Closed);
        }
        let mut reviews = match self.reviews.lock() {
            Ok(reviews) => reviews,
            Err(_) => {
                let _ = review.release_inner();
                return Err(EngineError::InternalState);
            }
        };
        reviews.retain(|review| review.strong_count() != 0);
        reviews.push(Arc::downgrade(&review));
        Ok(review)
    }
    fn with_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let state = self.state.lock().map_err(|_| EngineError::InternalState)?;
        match &*state {
            EngineState::Open(engine) => operation(engine),
            EngineState::Closing | EngineState::Closed { .. } => Err(EngineError::Closed),
        }
    }

    fn with_pressure_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, PressurePolicyError>,
    ) -> Result<T, PressurePolicyError> {
        let state = self
            .state
            .lock()
            .map_err(|_| PressurePolicyError::InternalState)?;
        match &*state {
            EngineState::Open(engine) => operation(engine),
            EngineState::Closing | EngineState::Closed { .. } => Err(PressurePolicyError::Closed),
        }
    }

    fn with_scan_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, ScanError>,
    ) -> Result<T, ScanError> {
        let state = self.state.lock().map_err(|_| ScanError::InternalState)?;
        match &*state {
            EngineState::Open(engine) => operation(engine),
            EngineState::Closing | EngineState::Closed { .. } => Err(ScanError::Closed),
        }
    }

    fn release_registered_reviews(&self) {
        let reviews = match self.reviews.lock() {
            Ok(mut reviews) => std::mem::take(&mut *reviews),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };
        for review in reviews.into_iter().filter_map(|review| review.upgrade()) {
            let _ = review.release_inner();
        }
    }
}

impl Drop for DuxEngine {
    fn drop(&mut self) {
        let _ = self.close();
        LIVE_ENGINE_INSTANCE_COUNT.fetch_sub(1, Ordering::Relaxed);
    }
}

#[uniffi::export]
pub fn library_version() -> LibraryVersion {
    LibraryVersion {
        library_version: env!("CARGO_PKG_VERSION").to_owned(),
        ffi_contract_version: FFI_CONTRACT_VERSION,
    }
}

#[uniffi::export]
pub fn live_engine_instance_count() -> u64 {
    LIVE_ENGINE_INSTANCE_COUNT.load(Ordering::Relaxed)
}

fn scan_start(engine: &EngineHandle, id: TaskId, disposition: ScanStartDisposition) -> ScanStart {
    ScanStart {
        record_version: FFI_RECORD_VERSION,
        disposition,
        task: Arc::new(ScanTask {
            engine: engine.clone(),
            id,
            progress: Mutex::new(ScanProgressState::default()),
        }),
    }
}

fn scan_task_result(result: &CoreScanTaskResult) -> Result<ScanTaskResult, ScanError> {
    let counts = result.counts();
    let coverage = result.coverage();
    let issue_record_count =
        u64::try_from(coverage.issues().len()).map_err(|_| ScanError::InternalState)?;
    let issue_occurrence_count = coverage.issues().iter().try_fold(0_u64, |total, issue| {
        total
            .checked_add(u64::from(issue.occurrence_count()))
            .ok_or(ScanError::InternalState)
    })?;
    Ok(ScanTaskResult {
        record_version: FFI_RECORD_VERSION,
        scan_id: result.scan_id().as_str().to_owned(),
        started_at_unix_ms: scan_system_time_ms(result.started_at())?,
        completed_at_unix_ms: scan_system_time_ms(result.completed_at())?,
        status: map_scan_terminal_status(result.status()),
        directory_count: counts.directory_count,
        file_count: counts.file_count,
        logical_bytes: counts.logical_bytes,
        allocated_bytes: counts.allocated_bytes,
        snapshot_available: result.snapshot_available(),
        coverage: ScanCoverageSummary {
            record_version: FFI_RECORD_VERSION,
            status: map_scan_coverage_status(coverage.status()),
            measured_permille: coverage.measured_permille().map(|value| value.get()),
            issue_record_count,
            issue_occurrence_count,
        },
        candidate_evaluation: map_scan_candidate_evaluation(result.candidate_evaluation()),
    })
}

fn historical_scan_summary(
    scan: &dux_core::engine::DurableScanSummary,
) -> Result<HistoricalScanSummary, EngineError> {
    let issue_record_count =
        u64::try_from(scan.coverage.issue_record_count).map_err(|_| EngineError::InternalState)?;
    Ok(HistoricalScanSummary {
        record_version: FFI_RECORD_VERSION,
        scan_id: scan.scan_id.as_str().to_owned(),
        started_at_unix_ms: system_time_ms(scan.started_at)?,
        completed_at_unix_ms: scan.completed_at.map(system_time_ms).transpose()?,
        status: map_historical_scan_status(scan.status),
        counts: scan.counts.map(|counts| HistoricalScanCounts {
            directory_count: counts.directory_count,
            file_count: counts.file_count,
            logical_bytes: counts.logical_bytes,
            allocated_bytes: counts.allocated_bytes,
        }),
        coverage: ScanCoverageSummary {
            record_version: FFI_RECORD_VERSION,
            status: map_scan_coverage_status(scan.coverage.status),
            measured_permille: scan.coverage.measured_permille.map(|value| value.get()),
            issue_record_count,
            issue_occurrence_count: scan.coverage.issue_occurrence_count,
        },
        snapshot_recorded: scan.snapshot_recorded,
    })
}

fn historical_scan_issue(
    issue: &dux_core::engine::DurableScanIssue,
) -> Result<HistoricalScanIssue, EngineError> {
    let kind = map_historical_scan_issue_kind(issue.kind())?;
    let (location_scope, location_components, location_truncated) = match issue.location() {
        None => (HistoricalScanIssueLocationScope::Global, Vec::new(), false),
        Some(location) if location.is_scan_root() => (
            HistoricalScanIssueLocationScope::ScanRoot,
            Vec::new(),
            false,
        ),
        Some(location) => (
            HistoricalScanIssueLocationScope::Descendant,
            location
                .components()
                .iter()
                .map(|component| component.as_ref().to_owned())
                .collect(),
            location.context_truncated(),
        ),
    };
    Ok(HistoricalScanIssue {
        record_version: FFI_RECORD_VERSION,
        ordinal: issue.ordinal(),
        kind,
        occurrence_count: issue.occurrence_count(),
        location_scope,
        location_components,
        location_truncated,
    })
}

fn map_historical_scan_issue_kind(
    kind: CoreDurableScanIssueKind,
) -> Result<HistoricalScanIssueKind, EngineError> {
    Ok(match kind {
        CoreDurableScanIssueKind::PermissionDenied => HistoricalScanIssueKind::PermissionDenied,
        CoreDurableScanIssueKind::TimedOut => HistoricalScanIssueKind::TimedOut,
        CoreDurableScanIssueKind::DifferentFilesystem => {
            HistoricalScanIssueKind::DifferentFilesystem
        }
        CoreDurableScanIssueKind::NetworkOrVirtualFilesystem => {
            HistoricalScanIssueKind::NetworkOrVirtualFilesystem
        }
        CoreDurableScanIssueKind::SymlinkSkipped => HistoricalScanIssueKind::SymlinkSkipped,
        CoreDurableScanIssueKind::FileChangedDuringScan => {
            HistoricalScanIssueKind::FileChangedDuringScan
        }
        CoreDurableScanIssueKind::MetadataError => HistoricalScanIssueKind::MetadataError,
        CoreDurableScanIssueKind::Cancelled => HistoricalScanIssueKind::Cancelled,
        CoreDurableScanIssueKind::PolicyExcluded => HistoricalScanIssueKind::PolicyExcluded,
        CoreDurableScanIssueKind::DepthLimited => HistoricalScanIssueKind::DepthLimited,
        CoreDurableScanIssueKind::ProbePoolExhausted => HistoricalScanIssueKind::ProbePoolExhausted,
        CoreDurableScanIssueKind::FilesystemBoundaryUnknown => {
            HistoricalScanIssueKind::FilesystemBoundaryUnknown
        }
        CoreDurableScanIssueKind::IssueLimitReached => HistoricalScanIssueKind::IssueLimitReached,
        _ => return Err(EngineError::InternalState),
    })
}

fn map_historical_scan_status(status: CoreDurableScanStatus) -> HistoricalScanStatus {
    match status {
        CoreDurableScanStatus::Queued => HistoricalScanStatus::Queued,
        CoreDurableScanStatus::Running => HistoricalScanStatus::Running,
        CoreDurableScanStatus::Succeeded => HistoricalScanStatus::Succeeded,
        CoreDurableScanStatus::Failed => HistoricalScanStatus::Failed,
        CoreDurableScanStatus::Cancelled => HistoricalScanStatus::Cancelled,
        CoreDurableScanStatus::Interrupted => HistoricalScanStatus::Interrupted,
        _ => HistoricalScanStatus::Interrupted,
    }
}

fn map_scan_terminal_status(status: CoreScanTaskStatus) -> ScanTerminalStatus {
    match status {
        CoreScanTaskStatus::Succeeded => ScanTerminalStatus::Succeeded,
        CoreScanTaskStatus::Failed => ScanTerminalStatus::Failed,
        CoreScanTaskStatus::Cancelled => ScanTerminalStatus::Cancelled,
        CoreScanTaskStatus::Interrupted => ScanTerminalStatus::Interrupted,
        _ => ScanTerminalStatus::Interrupted,
    }
}

fn map_scan_coverage_status(status: CoreCoverageStatus) -> ScanCoverageStatus {
    match status {
        CoreCoverageStatus::Unknown => ScanCoverageStatus::Unknown,
        CoreCoverageStatus::Complete => ScanCoverageStatus::Complete,
        CoreCoverageStatus::LimitedAccess => ScanCoverageStatus::LimitedAccess,
        CoreCoverageStatus::Partial => ScanCoverageStatus::Partial,
    }
}

fn map_scan_candidate_evaluation(
    status: CoreCandidateEvaluationStatus,
) -> ScanCandidateEvaluationSummary {
    let (status, candidate_count, failure) = match status {
        CoreCandidateEvaluationStatus::NotRun => (ScanCandidateEvaluationStatus::NotRun, 0, None),
        CoreCandidateEvaluationStatus::Succeeded { candidate_count } => (
            ScanCandidateEvaluationStatus::Succeeded,
            candidate_count,
            None,
        ),
        CoreCandidateEvaluationStatus::Failed { kind } => (
            ScanCandidateEvaluationStatus::Failed,
            0,
            Some(map_scan_candidate_evaluation_failure(kind)),
        ),
        _ => (
            ScanCandidateEvaluationStatus::Failed,
            0,
            Some(ScanCandidateEvaluationFailure::InternalState),
        ),
    };
    ScanCandidateEvaluationSummary {
        record_version: FFI_RECORD_VERSION,
        status,
        candidate_count,
        failure,
    }
}

fn map_scan_candidate_evaluation_failure(
    failure: CoreCandidateEvaluationFailure,
) -> ScanCandidateEvaluationFailure {
    match failure {
        CoreCandidateEvaluationFailure::Cancelled => ScanCandidateEvaluationFailure::Cancelled,
        CoreCandidateEvaluationFailure::CatalogInvalid => {
            ScanCandidateEvaluationFailure::CatalogInvalid
        }
        CoreCandidateEvaluationFailure::ContextInvalid => {
            ScanCandidateEvaluationFailure::ContextInvalid
        }
        CoreCandidateEvaluationFailure::EvaluationFailed => {
            ScanCandidateEvaluationFailure::EvaluationFailed
        }
        CoreCandidateEvaluationFailure::CandidateInvalid => {
            ScanCandidateEvaluationFailure::CandidateInvalid
        }
        CoreCandidateEvaluationFailure::LimitExceeded => {
            ScanCandidateEvaluationFailure::LimitExceeded
        }
        _ => ScanCandidateEvaluationFailure::InternalState,
    }
}

fn map_scan_task_failure(failure: TaskFailureKind) -> ScanTaskFailure {
    match failure {
        TaskFailureKind::ScanRootChanged => ScanTaskFailure::RootChanged,
        TaskFailureKind::ScanFailed => ScanTaskFailure::ScanFailed,
        TaskFailureKind::SnapshotRejected => ScanTaskFailure::SnapshotRejected,
        TaskFailureKind::PersistenceUnavailable => ScanTaskFailure::PersistenceUnavailable,
        TaskFailureKind::PersistenceOutcomeUnknown => ScanTaskFailure::PersistenceOutcomeUnknown,
        TaskFailureKind::InternalFailure => ScanTaskFailure::InternalState,
        TaskFailureKind::ScanRecoveryMaintenance(_)
        | TaskFailureKind::HistoryMaintenance(_)
        | TaskFailureKind::SnapshotRetention(_)
        | TaskFailureKind::SnapshotOrphanMaintenance(_)
        | TaskFailureKind::SnapshotProvisioningStageMaintenance(_)
        | TaskFailureKind::SnapshotTerminalTempMaintenance(_)
        | TaskFailureKind::SnapshotUnleasedTempMaintenance(_) => ScanTaskFailure::InternalState,
        _ => ScanTaskFailure::InternalState,
    }
}

fn validate_scan_terminal_observation(
    phase: CoreTaskPhase,
    failure: Option<ScanTaskFailure>,
    result: Option<&ScanTaskResult>,
) -> Result<(), ScanError> {
    let valid = match phase {
        CoreTaskPhase::Queued | CoreTaskPhase::Running => failure.is_none() && result.is_none(),
        CoreTaskPhase::Succeeded => {
            failure.is_none()
                && result.is_some_and(|result| result.status == ScanTerminalStatus::Succeeded)
        }
        CoreTaskPhase::Failed => {
            failure.is_some()
                && result.is_none_or(|result| {
                    matches!(
                        result.status,
                        ScanTerminalStatus::Failed | ScanTerminalStatus::Interrupted
                    )
                })
        }
        CoreTaskPhase::Cancelled => {
            failure.is_none()
                && result.is_none_or(|result| result.status == ScanTerminalStatus::Cancelled)
        }
    };
    if valid {
        Ok(())
    } else {
        Err(ScanError::InternalState)
    }
}

fn map_scan_start_error(error: StartTaskError) -> ScanError {
    match error {
        StartTaskError::Closed => ScanError::Closed,
        StartTaskError::QueueFull => ScanError::QueueFull,
        StartTaskError::InputTooLarge { .. } => ScanError::InputTooLarge,
        StartTaskError::InvalidScanRoot { reason } => map_scan_root_error(reason),
        StartTaskError::ScanAlreadyActive { .. } => ScanError::InternalState,
        StartTaskError::ScanScopeBusy => ScanError::Busy,
        StartTaskError::ReadOnlyStore => ScanError::ReadOnlyStore,
        StartTaskError::PersistenceUnavailable => ScanError::StorageUnavailable,
        StartTaskError::TaskIdExhausted | StartTaskError::InternalState => {
            ScanError::RegistryUnavailable
        }
        _ => ScanError::InternalState,
    }
}

fn map_subtree_scan_start_error(error: StartSubtreeScanError) -> ScanError {
    match error {
        StartSubtreeScanError::ForeignReview => ScanError::ForeignReview,
        StartSubtreeScanError::Review(error) => match error {
            CoreReviewError::Closed => ScanError::Closed,
            CoreReviewError::LeaseExpired => ScanError::ReviewExpired,
            CoreReviewError::ScanNotFound | CoreReviewError::SnapshotUnavailable => {
                ScanError::ReviewUnavailable
            }
            CoreReviewError::NodeNotFound => ScanError::SnapshotNodeNotFound,
            CoreReviewError::NodeNotDirectory | CoreReviewError::LiveTargetUnsupported => {
                ScanError::SnapshotNodeNotDirectory
            }
            CoreReviewError::LivePathUnavailable => ScanError::RootIdentityUnavailable,
            CoreReviewError::LivePathMissing => ScanError::RootMissing,
            CoreReviewError::LivePathSymlink => ScanError::RootSymlink,
            CoreReviewError::LivePathCrossVolume | CoreReviewError::LivePathChanged => {
                ScanError::RootChanged
            }
            CoreReviewError::LivePathAccessDenied => ScanError::RootAccessDenied,
            CoreReviewError::ReadOnlyStore => ScanError::ReadOnlyStore,
            CoreReviewError::Busy => ScanError::Busy,
            CoreReviewError::IncompatibleSchema
            | CoreReviewError::UnsafeStorage
            | CoreReviewError::BudgetExceeded
            | CoreReviewError::CorruptData
            | CoreReviewError::IncompatibleSnapshot
            | CoreReviewError::Unavailable
            | CoreReviewError::OutcomeUnknown
            | CoreReviewError::InternalState
            | CoreReviewError::InvalidPage
            | CoreReviewError::InvalidTreemapBudget
            | CoreReviewError::InvalidLargeFileRequest => ScanError::StorageUnavailable,
            _ => ScanError::InternalState,
        },
        StartSubtreeScanError::Task(error) => match error {
            StartTaskError::ScanAlreadyActive { .. } | StartTaskError::ScanScopeBusy => {
                ScanError::Busy
            }
            error => map_scan_start_error(error),
        },
        _ => ScanError::InternalState,
    }
}

fn map_scan_root_error(error: ScanRootErrorKind) -> ScanError {
    match error {
        ScanRootErrorKind::InvalidPath => ScanError::InvalidRoot,
        ScanRootErrorKind::Missing => ScanError::RootMissing,
        ScanRootErrorKind::AccessDenied => ScanError::RootAccessDenied,
        ScanRootErrorKind::NotDirectory => ScanError::RootNotDirectory,
        ScanRootErrorKind::Symlink => ScanError::RootSymlink,
        ScanRootErrorKind::ChangedDuringValidation => ScanError::RootChanged,
        ScanRootErrorKind::IdentityUnavailable => ScanError::RootIdentityUnavailable,
        ScanRootErrorKind::UnsupportedPlatform => ScanError::UnsupportedPlatform,
        ScanRootErrorKind::Unavailable => ScanError::RootUnavailable,
        _ => ScanError::InternalState,
    }
}

fn map_scan_access_error(error: TaskAccessError) -> ScanError {
    match error {
        TaskAccessError::Closed => ScanError::Closed,
        TaskAccessError::UnknownTask => ScanError::TaskUnavailable,
        TaskAccessError::InvalidEventLimit { .. } | TaskAccessError::InvalidEventCursor => {
            ScanError::EventHistoryUnavailable
        }
        TaskAccessError::WrongTaskKind => ScanError::WrongTaskKind,
        TaskAccessError::InternalState => ScanError::RegistryUnavailable,
    }
}

fn scan_system_time_ms(value: SystemTime) -> Result<i64, ScanError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ScanError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| ScanError::InternalState)
}

fn start_maintenance(
    engine: &EngineHandle,
    kind: MaintenanceKind,
) -> Result<MaintenanceStart, EngineError> {
    macro_rules! map_start {
        ($outcome:expr, $started:path, $active:path, $busy:path) => {
            match $outcome.map_err(map_start_error)? {
                $started(id) => {
                    maintenance_start(engine, kind, MaintenanceStartDisposition::Started, Some(id))
                }
                $active(id) => maintenance_start(
                    engine,
                    kind,
                    MaintenanceStartDisposition::AlreadyActive,
                    Some(id),
                ),
                $busy => maintenance_start(
                    engine,
                    kind,
                    MaintenanceStartDisposition::DeferredBusy,
                    None,
                ),
            }
        };
    }
    Ok(match kind {
        MaintenanceKind::ScanRecovery => map_start!(
            engine.start_scan_recovery_maintenance(),
            ScanRecoveryMaintenanceStartOutcome::Started,
            ScanRecoveryMaintenanceStartOutcome::AlreadyActive,
            ScanRecoveryMaintenanceStartOutcome::DeferredBusy
        ),
        MaintenanceKind::History => map_start!(
            engine.start_history_maintenance(),
            HistoryMaintenanceStartOutcome::Started,
            HistoryMaintenanceStartOutcome::AlreadyActive,
            HistoryMaintenanceStartOutcome::DeferredBusy
        ),
        MaintenanceKind::SnapshotRetention => map_start!(
            engine.start_snapshot_retention(),
            SnapshotRetentionStartOutcome::Started,
            SnapshotRetentionStartOutcome::AlreadyActive,
            SnapshotRetentionStartOutcome::DeferredBusy
        ),
        MaintenanceKind::SnapshotOrphan => map_start!(
            engine.start_snapshot_orphan_maintenance(),
            SnapshotOrphanMaintenanceStartOutcome::Started,
            SnapshotOrphanMaintenanceStartOutcome::AlreadyActive,
            SnapshotOrphanMaintenanceStartOutcome::DeferredBusy
        ),
        MaintenanceKind::SnapshotProvisioningStage => map_start!(
            engine.start_snapshot_provisioning_stage_maintenance(),
            SnapshotProvisioningStageMaintenanceStartOutcome::Started,
            SnapshotProvisioningStageMaintenanceStartOutcome::AlreadyActive,
            SnapshotProvisioningStageMaintenanceStartOutcome::DeferredBusy
        ),
        MaintenanceKind::SnapshotTerminalTemp => map_start!(
            engine.start_snapshot_terminal_temp_maintenance(),
            SnapshotTerminalTempMaintenanceStartOutcome::Started,
            SnapshotTerminalTempMaintenanceStartOutcome::AlreadyActive,
            SnapshotTerminalTempMaintenanceStartOutcome::DeferredBusy
        ),
        MaintenanceKind::SnapshotUnleasedTemp => map_start!(
            engine.start_snapshot_unleased_temp_maintenance(),
            SnapshotUnleasedTempMaintenanceStartOutcome::Started,
            SnapshotUnleasedTempMaintenanceStartOutcome::AlreadyActive,
            SnapshotUnleasedTempMaintenanceStartOutcome::DeferredBusy
        ),
    })
}

fn maintenance_start(
    engine: &EngineHandle,
    kind: MaintenanceKind,
    disposition: MaintenanceStartDisposition,
    id: Option<TaskId>,
) -> MaintenanceStart {
    MaintenanceStart {
        record_version: FFI_RECORD_VERSION,
        disposition,
        task: id.map(|id| {
            Arc::new(MaintenanceTask {
                engine: engine.clone(),
                id,
                kind,
            })
        }),
    }
}

fn empty_result(
    kind: MaintenanceKind,
    observed_at: SystemTime,
    outcome: MaintenanceOutcome,
    has_more: bool,
) -> Result<MaintenanceResult, EngineError> {
    Ok(MaintenanceResult {
        record_version: FFI_RECORD_VERSION,
        kind,
        observed_at_unix_ms: system_time_ms(observed_at)?,
        outcome,
        primary_count_before: 0,
        primary_count_after: 0,
        secondary_count_before: 0,
        secondary_count_after: 0,
        tertiary_count_before: 0,
        tertiary_count_after: 0,
        quaternary_count_before: 0,
        quaternary_count_after: 0,
        charged_bytes_before: 0,
        charged_bytes_after: 0,
        removed_bytes: 0,
        cap_bytes: 0,
        has_more,
    })
}

fn maintenance_result(
    engine: &EngineHandle,
    id: TaskId,
    kind: MaintenanceKind,
) -> Result<Option<MaintenanceResult>, EngineError> {
    let result = match kind {
        MaintenanceKind::ScanRecovery => engine
            .scan_recovery_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let outcome = match r.outcome() {
                    CoreScanRecoveryOutcome::NoClaim => MaintenanceOutcome::ScanRecoveryNone,
                    CoreScanRecoveryOutcome::DeferredUnproven => {
                        MaintenanceOutcome::ScanRecoveryDeferredUnproven
                    }
                    CoreScanRecoveryOutcome::Interrupted => {
                        MaintenanceOutcome::ScanRecoveryInterrupted
                    }
                    CoreScanRecoveryOutcome::ChangedConcurrently => {
                        MaintenanceOutcome::ScanRecoveryChangedConcurrently
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.primary_count_before = u64::from(r.claimed_count_before());
                out.primary_count_after = u64::from(r.claimed_count_after());
                out.secondary_count_before = u64::from(r.alive_count());
                out.tertiary_count_before = u64::from(r.unknown_count());
                out.quaternary_count_before = u64::from(r.recoverable_count());
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::History => engine
            .history_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let mut out = empty_result(
                    kind,
                    r.observed_at(),
                    MaintenanceOutcome::HistoryApplied,
                    r.has_more(),
                )?;
                out.primary_count_after = u64::from(r.daily_rollups_created());
                out.secondary_count_after = u64::from(r.raw_samples_pruned());
                out.tertiary_count_after = u64::from(r.daily_rollups_pruned());
                out.quaternary_count_after = u64::from(r.ai_insights_pruned());
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::SnapshotRetention => engine
            .snapshot_retention_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let (outcome, removed) = match r.outcome() {
                    CoreRetentionOutcome::UnderCap => (MaintenanceOutcome::RetentionUnderCap, 0),
                    CoreRetentionOutcome::DeferredUnstable => {
                        (MaintenanceOutcome::RetentionDeferredUnstable, 0)
                    }
                    CoreRetentionOutcome::DeferredNoEligibleSnapshot => {
                        (MaintenanceOutcome::RetentionDeferredNoEligibleSnapshot, 0)
                    }
                    CoreRetentionOutcome::RemovedTombstonedResidual { bytes } => (
                        MaintenanceOutcome::RetentionRemovedTombstonedResidual,
                        bytes,
                    ),
                    CoreRetentionOutcome::TombstonedAndRemoved { bytes } => {
                        (MaintenanceOutcome::RetentionTombstonedAndRemoved, bytes)
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.charged_bytes_before = r.charged_bytes_before();
                out.charged_bytes_after = r.charged_bytes_after();
                out.removed_bytes = removed;
                out.cap_bytes = r.cap_bytes();
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::SnapshotOrphan => engine
            .snapshot_orphan_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let (outcome, removed) = match r.outcome() {
                    CoreOrphanOutcome::NoOrphan => (MaintenanceOutcome::OrphanNone, 0),
                    CoreOrphanOutcome::Removed { bytes } => {
                        (MaintenanceOutcome::OrphanRemoved, bytes)
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.primary_count_before = u64::from(r.orphan_count_before());
                out.primary_count_after = u64::from(r.orphan_count_after());
                out.charged_bytes_before = r.orphan_charged_bytes_before();
                out.charged_bytes_after = r.orphan_charged_bytes_after();
                out.removed_bytes = removed;
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::SnapshotProvisioningStage => engine
            .snapshot_provisioning_stage_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let (outcome, removed) = match r.outcome() {
                    CoreStageOutcome::NoStage => (MaintenanceOutcome::StageNone, 0),
                    CoreStageOutcome::DeferredUnproven => {
                        (MaintenanceOutcome::StageDeferredUnproven, 0)
                    }
                    CoreStageOutcome::RemovedMarkerOnly { bytes } => {
                        (MaintenanceOutcome::StageRemovedMarkerOnly, bytes)
                    }
                    CoreStageOutcome::RemovedMarkerComplete { bytes } => {
                        (MaintenanceOutcome::StageRemovedMarkerComplete, bytes)
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.primary_count_before = r.total_stage_count_before();
                out.primary_count_after = r.total_stage_count_after();
                out.secondary_count_before = r.marker_owned_count_before();
                out.secondary_count_after = r.marker_owned_count_after();
                out.tertiary_count_before = r.unproven_count_before();
                out.tertiary_count_after = r.unproven_count_after();
                out.charged_bytes_before = r.control_charged_bytes_before();
                out.charged_bytes_after = r.control_charged_bytes_after();
                out.removed_bytes = removed;
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::SnapshotTerminalTemp => engine
            .snapshot_terminal_temp_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let (outcome, removed) = match r.outcome() {
                    CoreTerminalTempOutcome::NoTerminalResidual => {
                        (MaintenanceOutcome::TerminalTempNone, 0)
                    }
                    CoreTerminalTempOutcome::DeferredActive => {
                        (MaintenanceOutcome::TerminalTempDeferredActive, 0)
                    }
                    CoreTerminalTempOutcome::ReconciledRowOnly => {
                        (MaintenanceOutcome::TerminalTempReconciledRowOnly, 0)
                    }
                    CoreTerminalTempOutcome::RemovedTemp { bytes } => {
                        (MaintenanceOutcome::TerminalTempRemoved, bytes)
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.primary_count_before = u64::from(r.terminal_lease_count_before());
                out.primary_count_after = u64::from(r.terminal_lease_count_after());
                out.secondary_count_before = u64::from(r.active_terminal_lease_count_before());
                out.secondary_count_after = u64::from(r.active_terminal_lease_count_after());
                out.charged_bytes_before = r.terminal_charged_bytes_before();
                out.charged_bytes_after = r.terminal_charged_bytes_after();
                out.removed_bytes = removed;
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::SnapshotUnleasedTemp => engine
            .snapshot_unleased_temp_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let (outcome, removed) = match r.outcome() {
                    CoreUnleasedTempOutcome::NoUnleasedTemp => {
                        (MaintenanceOutcome::UnleasedTempNone, 0)
                    }
                    CoreUnleasedTempOutcome::DeferredActive => {
                        (MaintenanceOutcome::UnleasedTempDeferredActive, 0)
                    }
                    CoreUnleasedTempOutcome::Removed { bytes } => {
                        (MaintenanceOutcome::UnleasedTempRemoved, bytes)
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.primary_count_before = u64::from(r.unleased_temp_count_before());
                out.primary_count_after = u64::from(r.unleased_temp_count_after());
                out.secondary_count_before = u64::from(r.active_unleased_temp_count_before());
                out.secondary_count_after = u64::from(r.active_unleased_temp_count_after());
                out.charged_bytes_before = r.unleased_charged_bytes_before();
                out.charged_bytes_after = r.unleased_charged_bytes_after();
                out.removed_bytes = removed;
                Ok(out)
            })
            .transpose()?,
    };
    Ok(result)
}

fn map_phase(phase: CoreTaskPhase) -> TaskPhase {
    match phase {
        CoreTaskPhase::Queued => TaskPhase::Queued,
        CoreTaskPhase::Running => TaskPhase::Running,
        CoreTaskPhase::Succeeded => TaskPhase::Succeeded,
        CoreTaskPhase::Failed => TaskPhase::Failed,
        CoreTaskPhase::Cancelled => TaskPhase::Cancelled,
    }
}

fn map_failure(failure: TaskFailureKind) -> MaintenanceFailure {
    use dux_core::engine::{
        HistoryMaintenanceFailureKind as H, ScanRecoveryMaintenanceFailureKind as S,
        SnapshotOrphanMaintenanceFailureKind as O,
        SnapshotProvisioningStageMaintenanceFailureKind as P, SnapshotRetentionFailureKind as R,
        SnapshotTerminalTempMaintenanceFailureKind as T,
        SnapshotUnleasedTempMaintenanceFailureKind as U,
    };
    macro_rules! map_typed {
        ($value:expr, $kind:ident) => {
            match $value {
                $kind::InvalidClock => MaintenanceFailure::InvalidClock,
                $kind::IncompatibleSchema => MaintenanceFailure::IncompatibleSchema,
                $kind::Busy => MaintenanceFailure::Busy,
                $kind::UnsafeStorage => MaintenanceFailure::UnsafeStorage,
                $kind::BudgetExceeded => MaintenanceFailure::BudgetExceeded,
                $kind::CorruptData => MaintenanceFailure::CorruptData,
                $kind::Unavailable => MaintenanceFailure::Unavailable,
                $kind::OutcomeUnknown => MaintenanceFailure::OutcomeUnknown,
                $kind::InternalState => MaintenanceFailure::InternalState,
                _ => MaintenanceFailure::InternalState,
            }
        };
    }
    match failure {
        TaskFailureKind::ScanRecoveryMaintenance(value) => map_typed!(value, S),
        TaskFailureKind::HistoryMaintenance(value) => map_typed!(value, H),
        TaskFailureKind::SnapshotProvisioningStageMaintenance(value) => map_typed!(value, P),
        TaskFailureKind::SnapshotTerminalTempMaintenance(value) => map_typed!(value, T),
        TaskFailureKind::SnapshotUnleasedTempMaintenance(value) => map_typed!(value, U),
        TaskFailureKind::SnapshotRetention(value) => match value {
            R::IncompatibleSnapshot => MaintenanceFailure::IncompatibleSnapshot,
            other => map_typed!(other, R),
        },
        TaskFailureKind::SnapshotOrphanMaintenance(value) => match value {
            O::IncompatibleSnapshot => MaintenanceFailure::IncompatibleSnapshot,
            other => map_typed!(other, O),
        },
        _ => MaintenanceFailure::InternalState,
    }
}

fn map_review_error(error: CoreReviewError) -> EngineError {
    match error {
        CoreReviewError::Closed => EngineError::Closed,
        CoreReviewError::ScanNotFound => EngineError::ScanNotFound,
        CoreReviewError::SnapshotUnavailable => EngineError::SnapshotUnavailable,
        CoreReviewError::LeaseExpired => EngineError::ReviewExpired,
        CoreReviewError::NodeNotFound => EngineError::SnapshotNodeNotFound,
        CoreReviewError::NodeNotDirectory => EngineError::SnapshotNodeNotDirectory,
        CoreReviewError::InvalidPage => EngineError::InvalidSnapshotNodePage,
        CoreReviewError::InvalidTreemapBudget => EngineError::InvalidSnapshotTreemapBudget,
        CoreReviewError::InvalidLargeFileRequest => EngineError::InvalidSnapshotLargeFileRequest,
        CoreReviewError::LiveTargetUnsupported => EngineError::SnapshotLiveTargetUnsupported,
        CoreReviewError::LivePathUnavailable => EngineError::SnapshotLivePathUnavailable,
        CoreReviewError::LivePathMissing => EngineError::SnapshotLivePathMissing,
        CoreReviewError::LivePathSymlink => EngineError::SnapshotLivePathSymlink,
        CoreReviewError::LivePathCrossVolume => EngineError::SnapshotLivePathCrossVolume,
        CoreReviewError::LivePathChanged => EngineError::SnapshotLivePathChanged,
        CoreReviewError::LivePathAccessDenied => EngineError::SnapshotLivePathAccessDenied,
        CoreReviewError::ReadOnlyStore => EngineError::ReadOnlyStore,
        CoreReviewError::IncompatibleSchema => EngineError::IncompatibleSchema,
        CoreReviewError::Busy => EngineError::Busy,
        CoreReviewError::UnsafeStorage => EngineError::UnsafeStorage,
        CoreReviewError::BudgetExceeded => EngineError::BudgetExceeded,
        CoreReviewError::CorruptData => EngineError::CorruptData,
        CoreReviewError::IncompatibleSnapshot => EngineError::IncompatibleSnapshot,
        CoreReviewError::Unavailable => EngineError::StorageUnavailable,
        CoreReviewError::OutcomeUnknown => EngineError::OutcomeUnknown,
        CoreReviewError::InternalState => EngineError::InternalState,
        _ => EngineError::InternalState,
    }
}

const fn map_snapshot_live_target_purpose(
    purpose: SnapshotLiveTargetPurpose,
) -> CoreReviewLiveTargetPurpose {
    match purpose {
        SnapshotLiveTargetPurpose::Reveal => CoreReviewLiveTargetPurpose::Reveal,
        SnapshotLiveTargetPurpose::CopyPath => CoreReviewLiveTargetPurpose::CopyPath,
        SnapshotLiveTargetPurpose::QuickLook => CoreReviewLiveTargetPurpose::QuickLook,
    }
}

fn project_snapshot_live_target(
    target: CoreReviewLiveTarget,
) -> Result<SnapshotLiveTarget, EngineError> {
    if !target.path.is_absolute() {
        return Err(EngineError::InternalState);
    }
    let (path_encoding, absolute_path_bytes) = encode_snapshot_live_path(&target.path)?;
    Ok(SnapshotLiveTarget {
        record_version: FFI_RECORD_VERSION,
        node_id: target.node_id,
        purpose: match target.purpose {
            CoreReviewLiveTargetPurpose::Reveal => SnapshotLiveTargetPurpose::Reveal,
            CoreReviewLiveTargetPurpose::CopyPath => SnapshotLiveTargetPurpose::CopyPath,
            CoreReviewLiveTargetPurpose::QuickLook => SnapshotLiveTargetPurpose::QuickLook,
        },
        kind: match target.kind {
            CoreReviewLiveTargetKind::Directory => SnapshotLiveTargetKind::Directory,
            CoreReviewLiveTargetKind::File => SnapshotLiveTargetKind::File,
        },
        display_path: target.path.to_string_lossy().into_owned(),
        exact_text_path: target.path.to_str().map(str::to_owned),
        path_encoding,
        absolute_path_bytes,
    })
}

#[cfg(unix)]
fn encode_snapshot_live_path(path: &Path) -> Result<(SnapshotNameEncoding, Vec<u8>), EngineError> {
    use std::os::unix::ffi::OsStrExt;

    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() || bytes.contains(&0) {
        return Err(EngineError::InternalState);
    }
    Ok((SnapshotNameEncoding::UnixBytes, bytes.to_vec()))
}

#[cfg(windows)]
fn encode_snapshot_live_path(path: &Path) -> Result<(SnapshotNameEncoding, Vec<u8>), EngineError> {
    use std::os::windows::ffi::OsStrExt;

    let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if units.is_empty() || units.contains(&0) {
        return Err(EngineError::InternalState);
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(units.len().saturating_mul(2))
        .map_err(|_| EngineError::InternalState)?;
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    Ok((SnapshotNameEncoding::WindowsUtf16LittleEndian, bytes))
}

const fn map_snapshot_node_sort(sort: SnapshotNodeSort) -> CoreReviewNodeSort {
    match sort {
        SnapshotNodeSort::NameAscending => CoreReviewNodeSort::NameAscending,
        SnapshotNodeSort::LogicalBytesDescending => CoreReviewNodeSort::LogicalBytesDescending,
        SnapshotNodeSort::AllocatedBytesDescending => CoreReviewNodeSort::AllocatedBytesDescending,
        SnapshotNodeSort::ModifiedNewest => CoreReviewNodeSort::ModifiedNewest,
    }
}

fn project_snapshot_node_page(page: CoreReviewNodePage) -> SnapshotNodePage {
    SnapshotNodePage {
        record_version: FFI_RECORD_VERSION,
        parent_id: page.parent_id,
        offset: page.offset,
        total_children: page.total_children,
        has_more: page.has_more,
        nodes: page.nodes.into_iter().map(project_snapshot_node).collect(),
    }
}

fn project_snapshot_treemap(treemap: CoreReviewTreemap) -> SnapshotTreemap {
    SnapshotTreemap {
        record_version: FFI_RECORD_VERSION,
        parent_id: treemap.parent_id,
        total_children: treemap.total_children,
        total_child_logical_bytes: treemap.total_child_logical_bytes,
        other_child_count: treemap.other_child_count,
        other_logical_bytes: treemap.other_logical_bytes,
        zero_logical_child_count: treemap.zero_logical_child_count,
        cells: treemap
            .cells
            .into_iter()
            .map(project_snapshot_treemap_cell)
            .collect(),
    }
}

fn project_snapshot_treemap_cell(cell: CoreReviewTreemapCell) -> SnapshotTreemapCell {
    SnapshotTreemapCell {
        record_version: FFI_RECORD_VERSION,
        node: project_snapshot_node(cell.node),
        logical_rank: cell.logical_rank,
    }
}

fn project_snapshot_large_file_page(page: CoreReviewLargeFilePage) -> SnapshotLargeFilePage {
    SnapshotLargeFilePage {
        record_version: FFI_RECORD_VERSION,
        total_matching_files: page.total_matching_files,
        total_matching_logical_bytes: page.total_matching_logical_bytes,
        has_more: page.has_more,
        files: page
            .files
            .into_iter()
            .map(project_snapshot_large_file)
            .collect(),
    }
}

fn project_snapshot_large_file(file: CoreReviewLargeFile) -> SnapshotLargeFile {
    SnapshotLargeFile {
        record_version: FFI_RECORD_VERSION,
        node: project_snapshot_node(file.node),
        parent_context: file
            .parent_context
            .into_iter()
            .map(project_snapshot_node_name)
            .collect(),
        context_truncated: file.context_truncated,
    }
}

fn project_snapshot_node(node: CoreReviewNode) -> SnapshotNode {
    SnapshotNode {
        record_version: SNAPSHOT_NODE_RECORD_VERSION,
        id: node.id,
        parent_id: node.parent_id,
        depth: node.depth,
        kind: match node.kind {
            CoreReviewNodeKind::Directory => SnapshotNodeKind::Directory,
            CoreReviewNodeKind::File => SnapshotNodeKind::File,
            CoreReviewNodeKind::Symlink => SnapshotNodeKind::Symlink,
            CoreReviewNodeKind::Other => SnapshotNodeKind::Other,
            CoreReviewNodeKind::Error => SnapshotNodeKind::Error,
        },
        category: project_snapshot_category(node.category),
        name: project_snapshot_node_name(node.name),
        logical_bytes: node.logical_bytes,
        allocated_bytes: node.allocated_bytes,
        file_count: node.file_count,
        child_count: node.child_count,
        modified_at: node.modified_at.map(|timestamp| SnapshotNodeTimestamp {
            seconds_since_unix_epoch: timestamp.seconds_since_unix_epoch,
            nanoseconds: timestamp.nanoseconds,
        }),
        accessed_at: node.accessed_at.map(|timestamp| SnapshotNodeTimestamp {
            seconds_since_unix_epoch: timestamp.seconds_since_unix_epoch,
            nanoseconds: timestamp.nanoseconds,
        }),
        scan_flags: SnapshotNodeScanFlags {
            inaccessible: node.scan_flags.inaccessible,
            timed_out: node.scan_flags.timed_out,
            hard_link_duplicate: node.scan_flags.hard_link_duplicate,
            mount_boundary: node.scan_flags.mount_boundary,
        },
    }
}

const fn project_snapshot_category(category: CoreReviewCategory) -> SnapshotStorageCategory {
    match category {
        CoreReviewCategory::Unclassified => SnapshotStorageCategory::Unclassified,
        CoreReviewCategory::DeveloperArtifact => SnapshotStorageCategory::DeveloperArtifact,
        CoreReviewCategory::ApplicationCache => SnapshotStorageCategory::ApplicationCache,
        CoreReviewCategory::BrowserCache => SnapshotStorageCategory::BrowserCache,
        CoreReviewCategory::LogAndDiagnostic => SnapshotStorageCategory::LogAndDiagnostic,
        CoreReviewCategory::InstallerAndDownload => SnapshotStorageCategory::InstallerAndDownload,
        CoreReviewCategory::DeviceAndSimulatorData => {
            SnapshotStorageCategory::DeviceAndSimulatorData
        }
        CoreReviewCategory::CloudFile => SnapshotStorageCategory::CloudFile,
        CoreReviewCategory::LargeReviewItem => SnapshotStorageCategory::LargeReviewItem,
        CoreReviewCategory::ProtectedSystemData => SnapshotStorageCategory::ProtectedSystemData,
        CoreReviewCategory::UnknownStorage => SnapshotStorageCategory::UnknownStorage,
    }
}

fn project_snapshot_node_name(name: dux_core::engine::SnapshotReviewName) -> SnapshotNodeName {
    SnapshotNodeName {
        encoding: match name.encoding {
            CoreReviewNameEncoding::UnixBytes => SnapshotNameEncoding::UnixBytes,
            CoreReviewNameEncoding::WindowsUtf16LittleEndian => {
                SnapshotNameEncoding::WindowsUtf16LittleEndian
            }
        },
        encoded_bytes: name.encoded_bytes.as_ref().to_vec(),
        display: name.display.as_ref().to_owned(),
    }
}

fn map_scan_history_error(error: CoreScanHistoryError) -> EngineError {
    match error {
        CoreScanHistoryError::InvalidLimit { .. } => EngineError::BudgetExceeded,
        CoreScanHistoryError::Closed => EngineError::Closed,
        CoreScanHistoryError::IncompatibleSchema => EngineError::IncompatibleSchema,
        CoreScanHistoryError::Busy => EngineError::Busy,
        CoreScanHistoryError::UnsafeStorage => EngineError::UnsafeStorage,
        CoreScanHistoryError::QueryLimitExceeded => EngineError::BudgetExceeded,
        CoreScanHistoryError::CorruptData => EngineError::CorruptData,
        CoreScanHistoryError::Unavailable => EngineError::StorageUnavailable,
        CoreScanHistoryError::InternalState => EngineError::InternalState,
        _ => EngineError::InternalState,
    }
}

fn map_scan_coverage_details_error(error: CoreScanCoverageDetailsError) -> EngineError {
    match error {
        CoreScanCoverageDetailsError::InvalidLimit { .. }
        | CoreScanCoverageDetailsError::InvalidOffset => {
            EngineError::InvalidScanCoverageDetailsRequest
        }
        CoreScanCoverageDetailsError::Closed => EngineError::Closed,
        CoreScanCoverageDetailsError::ScanNotFound => EngineError::ScanNotFound,
        CoreScanCoverageDetailsError::IncompatibleSchema => EngineError::IncompatibleSchema,
        CoreScanCoverageDetailsError::Busy => EngineError::Busy,
        CoreScanCoverageDetailsError::UnsafeStorage => EngineError::UnsafeStorage,
        CoreScanCoverageDetailsError::QueryLimitExceeded => EngineError::BudgetExceeded,
        CoreScanCoverageDetailsError::CorruptData => EngineError::CorruptData,
        CoreScanCoverageDetailsError::Unavailable => EngineError::StorageUnavailable,
        CoreScanCoverageDetailsError::InternalState => EngineError::InternalState,
        _ => EngineError::InternalState,
    }
}

fn map_volume_status_error(error: CoreVolumeStatusError) -> EngineError {
    match error {
        CoreVolumeStatusError::Closed => EngineError::Closed,
        CoreVolumeStatusError::InvalidObservation => EngineError::InvalidCapacityObservation,
        CoreVolumeStatusError::ConflictingObservation => {
            EngineError::ConflictingCapacityObservation
        }
        CoreVolumeStatusError::SupersededObservation => EngineError::SupersededCapacityObservation,
        CoreVolumeStatusError::ReadOnlyStore => EngineError::ReadOnlyStore,
        CoreVolumeStatusError::IncompatibleSchema => EngineError::IncompatibleSchema,
        CoreVolumeStatusError::Busy => EngineError::Busy,
        CoreVolumeStatusError::UnsafeStorage => EngineError::UnsafeStorage,
        CoreVolumeStatusError::BudgetExceeded => EngineError::BudgetExceeded,
        CoreVolumeStatusError::CorruptData => EngineError::CorruptData,
        CoreVolumeStatusError::Unavailable => EngineError::StorageUnavailable,
        CoreVolumeStatusError::OutcomeUnknown => EngineError::OutcomeUnknown,
        CoreVolumeStatusError::InternalState => EngineError::InternalState,
    }
}

fn pressure_policy_config(
    input: PressurePolicyInput,
) -> Result<DiskPressureConfig, PressurePolicyError> {
    if input.record_version != FFI_RECORD_VERSION {
        return Err(PressurePolicyError::InvalidRecordVersion);
    }
    let critical = DiskPressureThreshold::new(
        input.critical_available_bytes,
        input.critical_available_basis_points,
    )
    .map_err(map_pressure_config_error)?;
    let warning = DiskPressureThreshold::new(
        input.warning_available_bytes,
        input.warning_available_basis_points,
    )
    .map_err(map_pressure_config_error)?;
    let recovery =
        DiskPressureRecoveryMargin::new(input.recovery_bytes, input.recovery_basis_points)
            .map_err(map_pressure_config_error)?;
    DiskPressureConfig::new(critical, warning, recovery).map_err(map_pressure_config_error)
}

fn map_pressure_config_error(error: DiskPressureConfigError) -> PressurePolicyError {
    match error {
        DiskPressureConfigError::ThresholdBytesZero => PressurePolicyError::ThresholdBytesZero,
        DiskPressureConfigError::ThresholdBasisPointsOutOfRange => {
            PressurePolicyError::ThresholdBasisPointsOutOfRange
        }
        DiskPressureConfigError::WarningBytesBelowCritical => {
            PressurePolicyError::WarningBytesBelowCritical
        }
        DiskPressureConfigError::WarningBasisPointsBelowCritical => {
            PressurePolicyError::WarningBasisPointsBelowCritical
        }
        DiskPressureConfigError::WarningThresholdMatchesCritical => {
            PressurePolicyError::WarningThresholdMatchesCritical
        }
        DiskPressureConfigError::RecoveryBytesZero => PressurePolicyError::RecoveryBytesZero,
        DiskPressureConfigError::RecoveryBasisPointsOutOfRange => {
            PressurePolicyError::RecoveryBasisPointsOutOfRange
        }
    }
}

fn map_pressure_policy_error(error: CorePressurePolicyError) -> PressurePolicyError {
    match error {
        CorePressurePolicyError::Closed => PressurePolicyError::Closed,
        CorePressurePolicyError::RevisionExhausted => PressurePolicyError::RevisionExhausted,
        CorePressurePolicyError::InvalidClock => PressurePolicyError::InvalidClock,
        CorePressurePolicyError::IncompatibleSchema => PressurePolicyError::IncompatibleSchema,
        CorePressurePolicyError::Busy => PressurePolicyError::Busy,
        CorePressurePolicyError::UnsafeStorage => PressurePolicyError::UnsafeStorage,
        CorePressurePolicyError::QueryLimitExceeded => PressurePolicyError::BudgetExceeded,
        CorePressurePolicyError::CorruptData => PressurePolicyError::CorruptData,
        CorePressurePolicyError::Unavailable => PressurePolicyError::Unavailable,
        CorePressurePolicyError::OutcomeUnknown => PressurePolicyError::OutcomeUnknown,
        CorePressurePolicyError::InternalState => PressurePolicyError::InternalState,
        _ => PressurePolicyError::InternalState,
    }
}

fn pressure_policy_status(
    policy: CorePressurePolicy,
) -> Result<PressurePolicyStatus, PressurePolicyError> {
    let critical = policy.config.critical_threshold();
    let warning = policy.config.warning_threshold();
    let recovery = policy.config.recovery_margin();
    let updated_at_unix_ms = policy.updated_at.map(pressure_policy_time_ms).transpose()?;
    if (policy.revision == 0) != updated_at_unix_ms.is_none()
        || (policy.revision == 0 && policy.source != CorePressurePolicySource::Default)
    {
        return Err(PressurePolicyError::InternalState);
    }
    Ok(PressurePolicyStatus {
        record_version: FFI_RECORD_VERSION,
        source: match policy.source {
            CorePressurePolicySource::Default => PressurePolicySource::Default,
            CorePressurePolicySource::Stored => PressurePolicySource::Stored,
        },
        revision: policy.revision,
        critical_available_bytes: critical.maximum_available_bytes(),
        critical_available_basis_points: critical.maximum_available_basis_points(),
        warning_available_bytes: warning.maximum_available_bytes(),
        warning_available_basis_points: warning.maximum_available_basis_points(),
        recovery_bytes: recovery.bytes(),
        recovery_basis_points: recovery.basis_points(),
        updated_at_unix_ms,
    })
}

fn pressure_policy_update(
    update: CorePressurePolicyUpdate,
) -> Result<PressurePolicyUpdate, PressurePolicyError> {
    Ok(PressurePolicyUpdate {
        record_version: FFI_RECORD_VERSION,
        policy: pressure_policy_status(update.settings)?,
        changed: update.changed,
    })
}

fn pressure_policy_time_ms(value: SystemTime) -> Result<i64, PressurePolicyError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PressurePolicyError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| PressurePolicyError::InternalState)
}

fn map_volume_pressure(pressure: CoreDiskPressure) -> VolumePressure {
    match pressure {
        CoreDiskPressure::Healthy => VolumePressure::Healthy,
        CoreDiskPressure::Warning => VolumePressure::Warning,
        CoreDiskPressure::Critical => VolumePressure::Critical,
        CoreDiskPressure::Unknown => VolumePressure::Unknown,
    }
}

fn map_capacity_source(source: CoreCapacitySource) -> VolumeCapacitySource {
    match source {
        CoreCapacitySource::ImportantUsage => VolumeCapacitySource::ImportantUsage,
        CoreCapacitySource::Ordinary => VolumeCapacitySource::Ordinary,
    }
}

fn map_history_disposition(disposition: CoreHistoryDisposition) -> VolumeHistoryDisposition {
    match disposition {
        CoreHistoryDisposition::Stored => VolumeHistoryDisposition::Stored,
        CoreHistoryDisposition::ExistingExact => VolumeHistoryDisposition::ExistingExact,
        CoreHistoryDisposition::SuppressedByHourlyCadence => {
            VolumeHistoryDisposition::SuppressedByHourlyCadence
        }
        CoreHistoryDisposition::NotStoredMissingOrdinaryAvailability => {
            VolumeHistoryDisposition::NotStoredMissingOrdinaryAvailability
        }
        CoreHistoryDisposition::NotStoredMissingStableIdentity => {
            VolumeHistoryDisposition::NotStoredMissingStableIdentity
        }
        CoreHistoryDisposition::NotStoredIncompleteMetadata => {
            VolumeHistoryDisposition::NotStoredIncompleteMetadata
        }
    }
}

fn parse_macos_volume_id(value: Option<String>) -> Result<Option<VolumeId>, EngineError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    let bytes = value.as_bytes();
    let canonical_shape = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
    if !canonical_shape {
        return Err(EngineError::InvalidCapacityObservation);
    }
    VolumeId::new(format!("volume:macos:{}", value.to_ascii_lowercase()))
        .map(Some)
        .map_err(|_| EngineError::InvalidCapacityObservation)
}

fn unix_ms_to_system_time(value: i64) -> Result<SystemTime, EngineError> {
    let millis = u64::try_from(value).map_err(|_| EngineError::InvalidCapacityObservation)?;
    UNIX_EPOCH
        .checked_add(Duration::from_millis(millis))
        .ok_or(EngineError::InvalidCapacityObservation)
}

fn map_open_error(error: EngineOpenError) -> EngineError {
    match error {
        EngineOpenError::CandidateCatalogInvalid => EngineError::InternalState,
        EngineOpenError::WorkerUnavailable => EngineError::RegistryUnavailable,
        EngineOpenError::Database(kind) => match kind {
            DatabaseOpenErrorKind::UnsafeStorageRoot
            | DatabaseOpenErrorKind::UnsafeStorageObject
            | DatabaseOpenErrorKind::OwnershipMismatch
            | DatabaseOpenErrorKind::UnsafePermissions
            | DatabaseOpenErrorKind::UnrecognizedDatabase => EngineError::UnsafeStorage,
            DatabaseOpenErrorKind::Busy => EngineError::Busy,
            DatabaseOpenErrorKind::InspectionLimitExceeded => EngineError::BudgetExceeded,
            DatabaseOpenErrorKind::CorruptDatabase => EngineError::CorruptData,
            DatabaseOpenErrorKind::StorageRootUnavailable
            | DatabaseOpenErrorKind::DatabaseUnavailable
            | DatabaseOpenErrorKind::MigrationFailed => EngineError::StorageUnavailable,
            DatabaseOpenErrorKind::InternalState => EngineError::InternalState,
        },
        EngineOpenError::Snapshot(kind) => match kind {
            SnapshotOpenErrorKind::InvalidConfiguration => EngineError::InvalidStorage,
            SnapshotOpenErrorKind::UnsafeRoot
            | SnapshotOpenErrorKind::UnsafeObject
            | SnapshotOpenErrorKind::UnrecognizedStore => EngineError::UnsafeStorage,
            SnapshotOpenErrorKind::Unavailable => EngineError::StorageUnavailable,
            SnapshotOpenErrorKind::Busy => EngineError::Busy,
            SnapshotOpenErrorKind::InternalState => EngineError::InternalState,
        },
    }
}
fn map_start_error(error: StartTaskError) -> EngineError {
    match error {
        StartTaskError::Closed => EngineError::Closed,
        StartTaskError::ReadOnlyStore => EngineError::ReadOnlyStore,
        StartTaskError::PersistenceUnavailable => EngineError::StorageUnavailable,
        StartTaskError::ScanScopeBusy => EngineError::Busy,
        StartTaskError::InternalState | StartTaskError::TaskIdExhausted => {
            EngineError::RegistryUnavailable
        }
        _ => EngineError::InternalState,
    }
}
fn map_task_access_error(error: dux_core::engine::TaskAccessError) -> EngineError {
    match error {
        dux_core::engine::TaskAccessError::Closed => EngineError::Closed,
        dux_core::engine::TaskAccessError::InternalState => EngineError::RegistryUnavailable,
        _ => EngineError::InternalState,
    }
}
fn system_time_ms(value: SystemTime) -> Result<i64, EngineError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| EngineError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| EngineError::InternalState)
}

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    static ENGINE_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn engine() -> (TempDir, DuxEngine) {
        let temp = TempDir::new().unwrap();
        let engine = DuxEngine::new(EngineStorageRoots {
            data_root: temp.path().join("data").to_string_lossy().into_owned(),
            cache_root: temp.path().join("cache").to_string_lossy().into_owned(),
        })
        .unwrap();
        (temp, engine)
    }

    #[test]
    fn reports_contract_sixteen_and_preserves_legacy_formatting() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        assert_eq!(library_version().ffi_contract_version, 16);
        assert_eq!(engine.library_version().unwrap(), library_version());
        assert_eq!(engine.format_size(1536).unwrap().display, "1.5 KB");
        assert!(engine.close());
        assert_eq!(engine.format_size(1), Err(EngineError::Closed));
    }

    fn scan_request(root: &std::path::Path) -> ScanRequest {
        ScanRequest {
            record_version: FFI_RECORD_VERSION,
            root: root.to_string_lossy().into_owned(),
        }
    }

    fn wait_for_scan(task: &ScanTask) -> ScanPoll {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let poll = task.poll().unwrap();
            if matches!(
                poll.phase,
                TaskPhase::Succeeded | TaskPhase::Failed | TaskPhase::Cancelled
            ) {
                return poll;
            }
            assert!(Instant::now() < deadline, "scan did not become terminal");
            std::thread::yield_now();
        }
    }

    #[test]
    fn subtree_scan_is_review_bound_path_free_and_returns_an_ordinary_scan_task() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("ffi-subtree-source");
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("old.bin"), b"old").unwrap();
        std::fs::write(root.join("outside.bin"), b"outside").unwrap();

        let source = engine.start_scan(scan_request(&root)).unwrap();
        let source_terminal = wait_for_scan(&source.task);
        assert_eq!(source_terminal.phase, TaskPhase::Succeeded);
        let source_scan = source_terminal.result.unwrap().scan_id;
        let review = engine
            .acquire_explorer_snapshot_review(source_scan)
            .unwrap();
        let children = review
            .child_nodes(0, SnapshotNodeSort::NameAscending, 0, 10)
            .unwrap();
        let nested_node = children
            .nodes
            .iter()
            .find(|node| node.name.display == "nested")
            .unwrap();
        let file_node = children
            .nodes
            .iter()
            .find(|node| node.kind == SnapshotNodeKind::File)
            .unwrap();

        assert!(matches!(
            engine.start_subtree_scan(
                Arc::clone(&review),
                SubtreeScanRequest {
                    record_version: FFI_RECORD_VERSION + 1,
                    node_id: nested_node.id,
                },
            ),
            Err(ScanError::InvalidRecordVersion)
        ));
        assert!(matches!(
            engine.start_subtree_scan(
                Arc::clone(&review),
                SubtreeScanRequest {
                    record_version: FFI_RECORD_VERSION,
                    node_id: file_node.id,
                },
            ),
            Err(ScanError::SnapshotNodeNotDirectory)
        ));
        assert!(matches!(
            engine.start_subtree_scan(
                Arc::clone(&review),
                SubtreeScanRequest {
                    record_version: FFI_RECORD_VERSION,
                    node_id: u64::MAX,
                },
            ),
            Err(ScanError::SnapshotNodeNotFound)
        ));

        let (_other_temp, other_engine) = self::engine();
        assert!(matches!(
            other_engine.start_subtree_scan(
                Arc::clone(&review),
                SubtreeScanRequest {
                    record_version: FFI_RECORD_VERSION,
                    node_id: nested_node.id,
                },
            ),
            Err(ScanError::ForeignReview)
        ));

        std::fs::write(nested.join("new.bin"), b"new").unwrap();
        let refresh = engine
            .start_subtree_scan(
                Arc::clone(&review),
                SubtreeScanRequest {
                    record_version: FFI_RECORD_VERSION,
                    node_id: nested_node.id,
                },
            )
            .unwrap();
        assert_eq!(refresh.disposition, ScanStartDisposition::Started);
        let terminal = wait_for_scan(&refresh.task);
        assert_eq!(terminal.phase, TaskPhase::Succeeded);
        let result = terminal.result.unwrap();
        assert_eq!(result.file_count, 2);
        assert!(result.snapshot_available);
        let refreshed = engine
            .acquire_explorer_snapshot_review(result.scan_id)
            .unwrap();
        assert_eq!(refreshed.root_node().unwrap().child_count, 2);

        assert_eq!(review.release().unwrap(), ReviewReleaseOutcome::Released);
        assert!(matches!(
            engine.start_subtree_scan(
                review,
                SubtreeScanRequest {
                    record_version: FFI_RECORD_VERSION,
                    node_id: nested_node.id,
                },
            ),
            Err(ScanError::ReviewExpired)
        ));
        assert!(other_engine.close());
        assert!(engine.close());
    }

    #[test]
    fn scan_progress_fold_retains_latest_aggregate_stage_and_truncation() {
        use dux_core::engine::{TaskEvent as CoreTaskEvent, TaskEventKind};

        let mut state = ScanProgressState::default();
        state.apply(CoreTaskEventBatch {
            events: vec![
                CoreTaskEvent {
                    sequence: 4,
                    kind: TaskEventKind::Started,
                },
                CoreTaskEvent {
                    sequence: 5,
                    kind: TaskEventKind::ScanProgress {
                        files: 12,
                        directories: 3,
                        known_allocated_bytes: 4096,
                        errors: 2,
                    },
                },
                CoreTaskEvent {
                    sequence: 6,
                    kind: TaskEventKind::ScanFinalizing,
                },
            ],
            next_sequence: 6,
            oldest_available_sequence: 4,
            truncated: true,
            terminal: false,
        });
        assert_eq!(state.event_cursor, 6);
        assert_eq!(state.stage, ScanStage::Finalizing);
        assert!(state.has_progress);
        assert_eq!(state.files_scanned, 12);
        assert_eq!(state.directories_scanned, 3);
        assert_eq!(state.known_allocated_bytes, 4096);
        assert_eq!(state.error_count, 2);
        assert!(state.events_truncated);

        state.apply(CoreTaskEventBatch {
            events: vec![CoreTaskEvent {
                sequence: 7,
                kind: TaskEventKind::CandidateEvaluationStarted,
            }],
            next_sequence: 7,
            oldest_available_sequence: 4,
            truncated: false,
            terminal: false,
        });
        assert_eq!(state.stage, ScanStage::Evaluating);
        assert!(state.events_truncated);
    }

    #[test]
    fn scan_start_and_access_errors_map_each_core_category() {
        for (input, expected) in [
            (ScanRootErrorKind::InvalidPath, ScanError::InvalidRoot),
            (ScanRootErrorKind::Missing, ScanError::RootMissing),
            (ScanRootErrorKind::AccessDenied, ScanError::RootAccessDenied),
            (ScanRootErrorKind::NotDirectory, ScanError::RootNotDirectory),
            (ScanRootErrorKind::Symlink, ScanError::RootSymlink),
            (
                ScanRootErrorKind::ChangedDuringValidation,
                ScanError::RootChanged,
            ),
            (
                ScanRootErrorKind::IdentityUnavailable,
                ScanError::RootIdentityUnavailable,
            ),
            (
                ScanRootErrorKind::UnsupportedPlatform,
                ScanError::UnsupportedPlatform,
            ),
            (ScanRootErrorKind::Unavailable, ScanError::RootUnavailable),
        ] {
            assert_eq!(map_scan_root_error(input), expected);
        }
        assert_eq!(
            map_scan_start_error(StartTaskError::Closed),
            ScanError::Closed
        );
        assert_eq!(
            map_scan_start_error(StartTaskError::QueueFull),
            ScanError::QueueFull
        );
        assert_eq!(
            map_scan_start_error(StartTaskError::InputTooLarge { limit: 1 }),
            ScanError::InputTooLarge
        );
        assert_eq!(
            map_scan_start_error(StartTaskError::ReadOnlyStore),
            ScanError::ReadOnlyStore
        );
        assert_eq!(
            map_scan_start_error(StartTaskError::PersistenceUnavailable),
            ScanError::StorageUnavailable
        );
        assert_eq!(
            map_scan_start_error(StartTaskError::ScanScopeBusy),
            ScanError::Busy
        );
        assert_eq!(
            map_scan_start_error(StartTaskError::TaskIdExhausted),
            ScanError::RegistryUnavailable
        );
        assert_eq!(
            map_scan_start_error(StartTaskError::InternalState),
            ScanError::RegistryUnavailable
        );

        for (input, expected) in [
            (TaskAccessError::Closed, ScanError::Closed),
            (TaskAccessError::UnknownTask, ScanError::TaskUnavailable),
            (
                TaskAccessError::InvalidEventLimit { max: 64 },
                ScanError::EventHistoryUnavailable,
            ),
            (
                TaskAccessError::InvalidEventCursor,
                ScanError::EventHistoryUnavailable,
            ),
            (TaskAccessError::WrongTaskKind, ScanError::WrongTaskKind),
            (
                TaskAccessError::InternalState,
                ScanError::RegistryUnavailable,
            ),
        ] {
            assert_eq!(map_scan_access_error(input), expected);
        }

        for (input, expected) in [
            (
                TaskFailureKind::ScanRootChanged,
                ScanTaskFailure::RootChanged,
            ),
            (TaskFailureKind::ScanFailed, ScanTaskFailure::ScanFailed),
            (
                TaskFailureKind::SnapshotRejected,
                ScanTaskFailure::SnapshotRejected,
            ),
            (
                TaskFailureKind::PersistenceUnavailable,
                ScanTaskFailure::PersistenceUnavailable,
            ),
            (
                TaskFailureKind::PersistenceOutcomeUnknown,
                ScanTaskFailure::PersistenceOutcomeUnknown,
            ),
            (
                TaskFailureKind::InternalFailure,
                ScanTaskFailure::InternalState,
            ),
        ] {
            assert_eq!(map_scan_task_failure(input), expected);
        }
    }

    #[test]
    fn public_scan_succeeds_with_consistent_path_free_terminal_summary() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("scan-success");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("payload.bin"), b"ffi scan payload").unwrap();

        let start = engine.start_scan(scan_request(&root)).unwrap();
        assert_eq!(start.record_version, FFI_RECORD_VERSION);
        assert_eq!(start.disposition, ScanStartDisposition::Started);
        let terminal = wait_for_scan(&start.task);
        assert_eq!(terminal.record_version, FFI_RECORD_VERSION);
        assert_eq!(terminal.phase, TaskPhase::Succeeded);
        assert_eq!(terminal.stage, ScanStage::Terminal);
        assert!(!terminal.cancellation_requested);
        assert_eq!(terminal.failure, None);
        assert!(!terminal.events_truncated);

        let result = terminal.result.expect("successful scan result");
        assert_eq!(result.record_version, FFI_RECORD_VERSION);
        assert!(result.scan_id.starts_with("scan:"));
        assert!(result.started_at_unix_ms <= result.completed_at_unix_ms);
        assert_eq!(result.status, ScanTerminalStatus::Succeeded);
        assert!(result.file_count >= 1);
        assert!(result.directory_count >= 1);
        assert!(result.logical_bytes >= 16);
        assert!(result.snapshot_available);
        assert_eq!(result.coverage.record_version, FFI_RECORD_VERSION);
        assert_eq!(result.coverage.status, ScanCoverageStatus::Complete);
        assert_eq!(result.coverage.measured_permille, Some(1_000));
        assert_eq!(result.coverage.issue_record_count, 0);
        assert_eq!(result.coverage.issue_occurrence_count, 0);
        assert_eq!(
            result.candidate_evaluation.status,
            ScanCandidateEvaluationStatus::Succeeded
        );
        assert_eq!(result.candidate_evaluation.failure, None);
        assert_eq!(
            start.task.cancel().unwrap(),
            ScanCancelOutcome::AlreadyTerminal
        );

        let history = engine.recent_scan_history(10).unwrap();
        assert_eq!(history.record_version, FFI_RECORD_VERSION);
        assert!(!history.has_more);
        assert_eq!(history.scans.len(), 1);
        let historical = &history.scans[0];
        assert_eq!(historical.record_version, FFI_RECORD_VERSION);
        assert_eq!(historical.scan_id, result.scan_id);
        assert_eq!(historical.started_at_unix_ms, result.started_at_unix_ms);
        assert_eq!(
            historical.completed_at_unix_ms,
            Some(result.completed_at_unix_ms)
        );
        assert_eq!(historical.status, HistoricalScanStatus::Succeeded);
        assert_eq!(
            historical.counts,
            Some(HistoricalScanCounts {
                directory_count: result.directory_count,
                file_count: result.file_count,
                logical_bytes: result.logical_bytes,
                allocated_bytes: result.allocated_bytes,
            })
        );
        assert_eq!(historical.coverage, result.coverage);
        assert!(historical.snapshot_recorded);
        let details = engine
            .scan_coverage_details(
                result.scan_id.clone(),
                ScanCoverageDetailsRequest {
                    record_version: FFI_RECORD_VERSION,
                    offset: 0,
                    limit: SCAN_COVERAGE_DETAIL_PAGE_LIMIT,
                },
            )
            .unwrap();
        assert_eq!(details.record_version, FFI_RECORD_VERSION);
        assert_eq!(details.scan_id, result.scan_id);
        assert_eq!(details.coverage, result.coverage);
        assert_eq!(details.offset, 0);
        assert_eq!(details.total_issue_records, 0);
        assert_eq!(details.total_issue_occurrences, 0);
        assert!(!details.has_more);
        assert!(details.issues.is_empty());
        assert!(engine.close());
        assert_eq!(start.task.poll(), Err(ScanError::Closed));
    }

    #[test]
    fn recent_scan_history_is_bounded_empty_and_closed() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        assert_eq!(
            engine.recent_scan_history(1).unwrap(),
            RecentScanHistoryPage {
                record_version: FFI_RECORD_VERSION,
                scans: Vec::new(),
                has_more: false,
            }
        );
        for invalid in [0, RECENT_SCAN_HISTORY_PAGE_LIMIT + 1] {
            assert_eq!(
                engine.recent_scan_history(invalid),
                Err(EngineError::BudgetExceeded)
            );
        }
        assert!(engine.close());
        assert_eq!(engine.recent_scan_history(1), Err(EngineError::Closed));
    }

    #[test]
    fn scan_coverage_details_validate_request_identity_and_all_issue_mappings() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        for request in [
            ScanCoverageDetailsRequest {
                record_version: FFI_RECORD_VERSION + 1,
                offset: 0,
                limit: 1,
            },
            ScanCoverageDetailsRequest {
                record_version: FFI_RECORD_VERSION,
                offset: 0,
                limit: 0,
            },
            ScanCoverageDetailsRequest {
                record_version: FFI_RECORD_VERSION,
                offset: 0,
                limit: SCAN_COVERAGE_DETAIL_PAGE_LIMIT + 1,
            },
        ] {
            assert_eq!(
                engine.scan_coverage_details("scan:missing".to_owned(), request),
                Err(EngineError::InvalidScanCoverageDetailsRequest)
            );
        }
        assert_eq!(
            engine.scan_coverage_details(
                "bad scan".to_owned(),
                ScanCoverageDetailsRequest {
                    record_version: FFI_RECORD_VERSION,
                    offset: 0,
                    limit: 1,
                },
            ),
            Err(EngineError::InvalidScanId)
        );
        assert_eq!(
            engine.scan_coverage_details(
                "scan:missing".to_owned(),
                ScanCoverageDetailsRequest {
                    record_version: FFI_RECORD_VERSION,
                    offset: 0,
                    limit: 1,
                },
            ),
            Err(EngineError::ScanNotFound)
        );

        let mappings = [
            (
                CoreDurableScanIssueKind::PermissionDenied,
                HistoricalScanIssueKind::PermissionDenied,
            ),
            (
                CoreDurableScanIssueKind::TimedOut,
                HistoricalScanIssueKind::TimedOut,
            ),
            (
                CoreDurableScanIssueKind::DifferentFilesystem,
                HistoricalScanIssueKind::DifferentFilesystem,
            ),
            (
                CoreDurableScanIssueKind::NetworkOrVirtualFilesystem,
                HistoricalScanIssueKind::NetworkOrVirtualFilesystem,
            ),
            (
                CoreDurableScanIssueKind::SymlinkSkipped,
                HistoricalScanIssueKind::SymlinkSkipped,
            ),
            (
                CoreDurableScanIssueKind::FileChangedDuringScan,
                HistoricalScanIssueKind::FileChangedDuringScan,
            ),
            (
                CoreDurableScanIssueKind::MetadataError,
                HistoricalScanIssueKind::MetadataError,
            ),
            (
                CoreDurableScanIssueKind::Cancelled,
                HistoricalScanIssueKind::Cancelled,
            ),
            (
                CoreDurableScanIssueKind::PolicyExcluded,
                HistoricalScanIssueKind::PolicyExcluded,
            ),
            (
                CoreDurableScanIssueKind::DepthLimited,
                HistoricalScanIssueKind::DepthLimited,
            ),
            (
                CoreDurableScanIssueKind::ProbePoolExhausted,
                HistoricalScanIssueKind::ProbePoolExhausted,
            ),
            (
                CoreDurableScanIssueKind::FilesystemBoundaryUnknown,
                HistoricalScanIssueKind::FilesystemBoundaryUnknown,
            ),
            (
                CoreDurableScanIssueKind::IssueLimitReached,
                HistoricalScanIssueKind::IssueLimitReached,
            ),
        ];
        for (core, ffi) in mappings {
            assert_eq!(map_historical_scan_issue_kind(core).unwrap(), ffi);
        }
    }

    #[test]
    fn public_scan_starts_and_maps_cancellation() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("scan-active");
        std::fs::create_dir(&root).unwrap();
        for index in 0..2_000 {
            std::fs::write(root.join(format!("payload-{index}")), b"x").unwrap();
        }

        let start = engine.start_scan(scan_request(&root)).unwrap();
        assert_eq!(start.disposition, ScanStartDisposition::Started);
        let cancellation = start.task.cancel().unwrap();
        assert!(matches!(
            cancellation,
            ScanCancelOutcome::CancelledBeforeStart
                | ScanCancelOutcome::Requested
                | ScanCancelOutcome::AlreadyRequested
                | ScanCancelOutcome::AlreadyTerminal
        ));
        let terminal = wait_for_scan(&start.task);
        assert_eq!(terminal.stage, ScanStage::Terminal);
        assert!(matches!(
            terminal.phase,
            TaskPhase::Cancelled | TaskPhase::Succeeded
        ));
        if terminal.phase == TaskPhase::Cancelled {
            assert_eq!(terminal.failure, None);
            assert!(
                terminal
                    .result
                    .is_none_or(|result| { result.status == ScanTerminalStatus::Cancelled })
            );
        }
        assert!(engine.close());
    }

    #[test]
    fn public_scan_rejects_unbounded_invalid_and_unavailable_roots() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();

        let mut wrong_version = scan_request(temp.path());
        wrong_version.record_version += 1;
        assert!(matches!(
            engine.start_scan(wrong_version),
            Err(ScanError::InvalidRecordVersion)
        ));
        for root in [
            String::new(),
            "relative".to_owned(),
            "/tmp/control\npath".to_owned(),
        ] {
            assert!(matches!(
                engine.start_scan(ScanRequest {
                    record_version: FFI_RECORD_VERSION,
                    root,
                }),
                Err(ScanError::InvalidRoot)
            ));
        }
        assert!(matches!(
            engine.start_scan(ScanRequest {
                record_version: FFI_RECORD_VERSION,
                root: format!("/{}", "x".repeat(MAX_SCAN_ROOT_UTF8_BYTES)),
            }),
            Err(ScanError::InputTooLarge)
        ));
        assert!(matches!(
            engine.start_scan(ScanRequest {
                record_version: FFI_RECORD_VERSION,
                root: format!("/{}", "x".repeat(MAX_SCAN_ROOT_UTF8_BYTES - 1)),
            }),
            Err(ScanError::RootMissing | ScanError::RootUnavailable)
        ));

        let missing = temp.path().join("missing");
        assert!(matches!(
            engine.start_scan(scan_request(&missing)),
            Err(ScanError::RootMissing)
        ));
        let file = temp.path().join("not-a-directory");
        std::fs::write(&file, b"file").unwrap();
        assert!(matches!(
            engine.start_scan(scan_request(&file)),
            Err(ScanError::RootNotDirectory)
        ));

        assert!(engine.close());
        assert!(matches!(
            engine.start_scan(scan_request(temp.path())),
            Err(ScanError::Closed)
        ));
    }

    fn startup_observation(
        sampled_at_unix_ms: i64,
        ordinary_available_bytes: Option<u64>,
        important_available_bytes: Option<u64>,
    ) -> StartupVolumeObservation {
        StartupVolumeObservation {
            record_version: FFI_RECORD_VERSION,
            stable_volume_id: Some("01234567-89AB-CDEF-0123-456789ABCDEF".into()),
            display_name: Some("Macintosh HD".into()),
            filesystem: Some("APFS".into()),
            is_internal: Some(true),
            is_removable: Some(false),
            sampled_at_unix_ms,
            total_bytes: 1_024 * 1_024 * 1_024 * 1_024,
            ordinary_available_bytes,
            important_available_bytes,
        }
    }

    fn default_pressure_policy_input() -> PressurePolicyInput {
        let gib = 1_024 * 1_024 * 1_024;
        PressurePolicyInput {
            record_version: FFI_RECORD_VERSION,
            critical_available_bytes: 10 * gib,
            critical_available_basis_points: 500,
            warning_available_bytes: 30 * gib,
            warning_available_basis_points: 1_000,
            recovery_bytes: 2 * gib,
            recovery_basis_points: 100,
        }
    }

    #[test]
    fn pressure_policy_get_set_reset_is_versioned_typed_and_path_free() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let initial = engine.get_disk_pressure_policy().unwrap();
        assert_eq!(initial.record_version, 1);
        assert_eq!(initial.source, PressurePolicySource::Default);
        assert_eq!(initial.revision, 0);
        assert_eq!(initial.updated_at_unix_ms, None);

        let stored = engine
            .set_disk_pressure_policy(default_pressure_policy_input())
            .unwrap();
        assert!(stored.changed);
        assert_eq!(stored.record_version, 1);
        assert_eq!(stored.policy.source, PressurePolicySource::Stored);
        assert_eq!(stored.policy.revision, 1);
        assert!(stored.policy.updated_at_unix_ms.is_some());
        assert_eq!(engine.get_disk_pressure_policy().unwrap(), stored.policy);

        let exact = engine
            .set_disk_pressure_policy(default_pressure_policy_input())
            .unwrap();
        assert!(!exact.changed);
        assert_eq!(exact.policy, stored.policy);

        let reset = engine.reset_disk_pressure_policy().unwrap();
        assert!(reset.changed);
        assert_eq!(reset.policy.source, PressurePolicySource::Default);
        assert_eq!(reset.policy.revision, 2);
        assert!(reset.policy.updated_at_unix_ms.is_some());
        let exact_reset = engine.reset_disk_pressure_policy().unwrap();
        assert!(!exact_reset.changed);
        assert_eq!(exact_reset.policy, reset.policy);

        assert!(engine.close());
        assert_eq!(
            engine.get_disk_pressure_policy(),
            Err(PressurePolicyError::Closed)
        );
        assert_eq!(
            engine.set_disk_pressure_policy(default_pressure_policy_input()),
            Err(PressurePolicyError::Closed)
        );
        assert_eq!(
            engine.reset_disk_pressure_policy(),
            Err(PressurePolicyError::Closed)
        );
    }

    #[test]
    fn pressure_policy_input_maps_every_validation_boundary() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();

        let mut value = default_pressure_policy_input();
        value.record_version += 1;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::InvalidRecordVersion)
        );
        let mut value = default_pressure_policy_input();
        value.critical_available_bytes = 0;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::ThresholdBytesZero)
        );
        let mut value = default_pressure_policy_input();
        value.critical_available_basis_points = 0;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::ThresholdBasisPointsOutOfRange)
        );
        let mut value = default_pressure_policy_input();
        value.warning_available_bytes = value.critical_available_bytes - 1;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::WarningBytesBelowCritical)
        );
        let mut value = default_pressure_policy_input();
        value.warning_available_basis_points = value.critical_available_basis_points - 1;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::WarningBasisPointsBelowCritical)
        );
        let mut value = default_pressure_policy_input();
        value.warning_available_bytes = value.critical_available_bytes;
        value.warning_available_basis_points = value.critical_available_basis_points;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::WarningThresholdMatchesCritical)
        );
        let mut value = default_pressure_policy_input();
        value.recovery_bytes = 0;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::RecoveryBytesZero)
        );
        let mut value = default_pressure_policy_input();
        value.recovery_basis_points = 0;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::RecoveryBasisPointsOutOfRange)
        );
    }

    #[test]
    fn startup_volume_round_trip_is_path_free_versioned_and_honest() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let gib = 1_024 * 1_024 * 1_024;
        let stored = engine
            .observe_startup_volume(startup_observation(
                3_600_000,
                Some(100 * gib),
                Some(20 * gib),
            ))
            .unwrap();
        assert_eq!(stored.record_version, 1);
        assert_eq!(
            stored.stable_volume_id.as_deref(),
            Some("volume:macos:01234567-89ab-cdef-0123-456789abcdef")
        );
        assert_eq!(stored.pressure, VolumePressure::Warning);
        assert_eq!(stored.headline_source, VolumeCapacitySource::ImportantUsage);
        assert_eq!(stored.history_disposition, VolumeHistoryDisposition::Stored);
        assert_eq!(stored.previous_durable_pressure, None);

        let important_only = engine
            .observe_startup_volume(startup_observation(3_600_001, None, Some(5 * gib)))
            .unwrap();
        assert_eq!(important_only.ordinary_available_bytes, None);
        assert_eq!(important_only.pressure, VolumePressure::Critical);
        assert_eq!(
            important_only.history_disposition,
            VolumeHistoryDisposition::NotStoredMissingOrdinaryAvailability
        );
        assert_eq!(
            important_only.previous_durable_pressure,
            Some(VolumePressure::Warning)
        );
    }

    #[test]
    fn startup_volume_rejects_malformed_input_and_use_after_close() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let gib = 1_024 * 1_024 * 1_024;
        let mut invalid_id = startup_observation(1, Some(gib), Some(gib));
        invalid_id.stable_volume_id = Some("not-a-uuid".into());
        assert_eq!(
            engine.observe_startup_volume(invalid_id),
            Err(EngineError::InvalidCapacityObservation)
        );
        let mut wrong_version = startup_observation(1, Some(gib), Some(gib));
        wrong_version.record_version = FFI_RECORD_VERSION + 1;
        assert_eq!(
            engine.observe_startup_volume(wrong_version),
            Err(EngineError::InvalidCapacityObservation)
        );
        assert_eq!(
            engine.observe_startup_volume(startup_observation(-1, Some(gib), Some(gib))),
            Err(EngineError::InvalidCapacityObservation)
        );
        assert_eq!(
            engine.observe_startup_volume(startup_observation(2, None, None)),
            Err(EngineError::InvalidCapacityObservation)
        );
        assert!(engine.close());
        assert_eq!(
            engine.observe_startup_volume(startup_observation(3, Some(gib), Some(gib))),
            Err(EngineError::Closed)
        );
    }

    #[test]
    fn all_maintenance_kinds_submit_and_poll_path_free_results() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        for kind in [
            MaintenanceKind::ScanRecovery,
            MaintenanceKind::History,
            MaintenanceKind::SnapshotRetention,
            MaintenanceKind::SnapshotOrphan,
            MaintenanceKind::SnapshotProvisioningStage,
            MaintenanceKind::SnapshotTerminalTemp,
            MaintenanceKind::SnapshotUnleasedTemp,
        ] {
            let start = engine.start_maintenance(kind).unwrap();
            assert_eq!(start.record_version, 1);
            let task = start.task.expect("started task");
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let poll = task.poll().unwrap();
                assert_eq!(poll.kind, kind);
                if matches!(
                    poll.phase,
                    TaskPhase::Succeeded | TaskPhase::Failed | TaskPhase::Cancelled
                ) {
                    assert!(poll.phase != TaskPhase::Succeeded || poll.result.is_some());
                    if kind == MaintenanceKind::ScanRecovery && poll.phase == TaskPhase::Succeeded {
                        let result = poll.result.as_ref().unwrap();
                        assert_eq!(result.outcome, MaintenanceOutcome::ScanRecoveryNone);
                        assert_eq!(result.primary_count_before, 0);
                        assert_eq!(result.primary_count_after, 0);
                        assert_eq!(result.secondary_count_before, 0);
                        assert_eq!(result.tertiary_count_before, 0);
                        assert_eq!(result.quaternary_count_before, 0);
                        assert_eq!(result.removed_bytes, 0);
                    }
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
        assert!(engine.close());
    }

    #[test]
    fn invalid_storage_scan_ids_and_missing_scans_are_typed() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        assert!(matches!(
            DuxEngine::new(EngineStorageRoots {
                data_root: "relative".into(),
                cache_root: "relative".into()
            }),
            Err(EngineError::InvalidStorage)
        ));
        let (_temp, engine) = engine();
        assert!(matches!(
            engine.acquire_explorer_snapshot_review("bad scan".into()),
            Err(EngineError::InvalidScanId)
        ));
        assert!(matches!(
            engine.acquire_explorer_snapshot_review("scan:missing".into()),
            Err(EngineError::ScanNotFound)
        ));
        assert!(matches!(
            engine.acquire_latest_explorer_snapshot_review(),
            Err(EngineError::SnapshotUnavailable)
        ));
        assert!(engine.close());
    }

    #[test]
    fn opaque_review_session_renews_and_releases_exact_scan() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("review-root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("payload"), b"ffi review").unwrap();
        std::fs::write(root.join("larger"), b"ffi review paging").unwrap();
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        let task = engine
            .with_engine(|core| core.start_scan(root.clone()).map_err(map_start_error))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let scan_id = loop {
            let observation = engine
                .with_engine(|core| {
                    let snapshot = core.task_snapshot(task).map_err(map_task_access_error)?;
                    if snapshot.phase == CoreTaskPhase::Succeeded {
                        Ok(core
                            .scan_result(task)
                            .map_err(map_task_access_error)?
                            .map(|result| result.scan_id().as_str().to_owned()))
                    } else {
                        Ok(None)
                    }
                })
                .unwrap();
            if let Some(scan_id) = observation {
                break scan_id;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        };

        let review = engine
            .acquire_explorer_snapshot_review(scan_id.clone())
            .unwrap();
        let initial = review.info().unwrap();
        assert_eq!(initial.scan_id, scan_id);
        assert!(!initial.released);
        assert!(initial.expires_at_unix_ms > 0);
        let root_node = review.root_node().unwrap();
        assert_eq!(root_node.record_version, SNAPSHOT_NODE_RECORD_VERSION);
        assert_eq!(root_node.id, 0);
        assert_eq!(root_node.kind, SnapshotNodeKind::Directory);
        assert_eq!(root_node.category, SnapshotStorageCategory::Unclassified);
        assert!(root_node.name.display.ends_with("/review-root"));
        assert_eq!(root_node.child_count, 2);
        let first = review
            .child_nodes(0, SnapshotNodeSort::LogicalBytesDescending, 0, 1)
            .unwrap();
        assert_eq!(first.record_version, 1);
        assert_eq!(first.total_children, 2);
        assert!(first.has_more);
        assert_eq!(first.nodes.len(), 1);
        assert_eq!(first.nodes[0].name.display, "larger");
        assert_eq!(
            first.nodes[0].category,
            SnapshotStorageCategory::Unclassified
        );
        let root_reveal = review
            .resolve_live_target(SnapshotLiveTargetRequest {
                record_version: FFI_RECORD_VERSION,
                node_id: 0,
                purpose: SnapshotLiveTargetPurpose::Reveal,
            })
            .unwrap();
        assert_eq!(root_reveal.record_version, FFI_RECORD_VERSION);
        assert_eq!(root_reveal.node_id, 0);
        assert_eq!(root_reveal.purpose, SnapshotLiveTargetPurpose::Reveal);
        assert_eq!(root_reveal.kind, SnapshotLiveTargetKind::Directory);
        assert_eq!(root_reveal.path_encoding, SnapshotNameEncoding::UnixBytes);
        assert!(!root_reveal.absolute_path_bytes.contains(&0));
        assert_eq!(
            root_reveal.exact_text_path.as_deref(),
            Some(canonical_root.to_str().unwrap())
        );
        assert_eq!(
            review.resolve_live_target(SnapshotLiveTargetRequest {
                record_version: FFI_RECORD_VERSION,
                node_id: 0,
                purpose: SnapshotLiveTargetPurpose::QuickLook,
            }),
            Err(EngineError::SnapshotLiveTargetUnsupported)
        );
        let file_quick_look = review
            .resolve_live_target(SnapshotLiveTargetRequest {
                record_version: FFI_RECORD_VERSION,
                node_id: first.nodes[0].id,
                purpose: SnapshotLiveTargetPurpose::QuickLook,
            })
            .unwrap();
        assert_eq!(file_quick_look.node_id, first.nodes[0].id);
        assert_eq!(
            file_quick_look.purpose,
            SnapshotLiveTargetPurpose::QuickLook
        );
        assert_eq!(file_quick_look.kind, SnapshotLiveTargetKind::File);
        assert!(!file_quick_look.absolute_path_bytes.contains(&0));
        assert_eq!(
            file_quick_look.exact_text_path.as_deref(),
            Some(canonical_root.join("larger").to_str().unwrap())
        );
        assert_eq!(
            review.resolve_live_target(SnapshotLiveTargetRequest {
                record_version: FFI_RECORD_VERSION + 1,
                node_id: first.nodes[0].id,
                purpose: SnapshotLiveTargetPurpose::Reveal,
            }),
            Err(EngineError::InvalidSnapshotLiveTargetRequest)
        );
        let treemap = review.treemap(0, 1).unwrap();
        assert_eq!(treemap.record_version, 1);
        assert_eq!(treemap.parent_id, 0);
        assert_eq!(treemap.total_children, 2);
        assert_eq!(treemap.total_child_logical_bytes, 27);
        assert_eq!(treemap.cells.len(), 1);
        assert_eq!(treemap.cells[0].record_version, 1);
        assert_eq!(treemap.cells[0].logical_rank, 0);
        assert_eq!(treemap.cells[0].node.name.display, "larger");
        assert_eq!(
            treemap.cells[0].node.category,
            SnapshotStorageCategory::Unclassified
        );
        assert_eq!(treemap.other_child_count, 1);
        assert_eq!(treemap.other_logical_bytes, 10);
        assert_eq!(treemap.zero_logical_child_count, 0);
        let large_files = review
            .large_files(SnapshotLargeFileRequest {
                record_version: FFI_RECORD_VERSION,
                minimum_logical_bytes: 1,
                modified_before: None,
                max_results: 1,
            })
            .unwrap();
        assert_eq!(large_files.record_version, FFI_RECORD_VERSION);
        assert_eq!(large_files.total_matching_files, 2);
        assert_eq!(large_files.total_matching_logical_bytes, 27);
        assert!(large_files.has_more);
        assert_eq!(large_files.files.len(), 1);
        assert_eq!(large_files.files[0].record_version, FFI_RECORD_VERSION);
        assert_eq!(large_files.files[0].node.kind, SnapshotNodeKind::File);
        assert_eq!(
            large_files.files[0].node.category,
            SnapshotStorageCategory::Unclassified
        );
        assert_eq!(large_files.files[0].node.name.display, "larger");
        assert!(large_files.files[0].parent_context.is_empty());
        assert!(!large_files.files[0].context_truncated);
        assert_eq!(
            review.large_files(SnapshotLargeFileRequest {
                record_version: FFI_RECORD_VERSION + 1,
                minimum_logical_bytes: 1,
                modified_before: None,
                max_results: 1,
            }),
            Err(EngineError::InvalidSnapshotLargeFileRequest)
        );
        assert_eq!(
            review.large_files(SnapshotLargeFileRequest {
                record_version: FFI_RECORD_VERSION,
                minimum_logical_bytes: 0,
                modified_before: None,
                max_results: 1,
            }),
            Err(EngineError::InvalidSnapshotLargeFileRequest)
        );
        assert_eq!(
            review.child_nodes(0, SnapshotNodeSort::NameAscending, 0, 0),
            Err(EngineError::InvalidSnapshotNodePage)
        );
        assert_eq!(
            review.treemap(0, 0),
            Err(EngineError::InvalidSnapshotTreemapBudget)
        );
        assert_eq!(
            review.treemap(0, 65),
            Err(EngineError::InvalidSnapshotTreemapBudget)
        );
        assert_eq!(
            review.child_nodes(u64::MAX, SnapshotNodeSort::NameAscending, 0, 1),
            Err(EngineError::SnapshotNodeNotFound)
        );
        assert_eq!(
            review.child_nodes(first.nodes[0].id, SnapshotNodeSort::NameAscending, 0, 1),
            Err(EngineError::SnapshotNodeNotDirectory)
        );
        let renewed = review.renew().unwrap();
        assert!(renewed.expires_at_unix_ms >= initial.expires_at_unix_ms);
        let latest = engine.acquire_latest_explorer_snapshot_review().unwrap();
        assert_eq!(latest.info().unwrap().scan_id, scan_id);
        assert_eq!(latest.release().unwrap(), ReviewReleaseOutcome::Released);
        let close_drained = engine
            .acquire_explorer_snapshot_review(scan_id.clone())
            .unwrap();
        assert_eq!(review.release().unwrap(), ReviewReleaseOutcome::Released);
        assert_eq!(
            review.release().unwrap(),
            ReviewReleaseOutcome::AlreadyReleased
        );
        let ended = review.info().unwrap();
        assert!(ended.released);
        assert_eq!(ended.expires_at_unix_ms, 0);
        assert_eq!(review.root_node(), Err(EngineError::ReviewExpired));
        assert_eq!(review.treemap(0, 1), Err(EngineError::ReviewExpired));
        assert_eq!(
            review.resolve_live_target(SnapshotLiveTargetRequest {
                record_version: FFI_RECORD_VERSION,
                node_id: 0,
                purpose: SnapshotLiveTargetPurpose::Reveal,
            }),
            Err(EngineError::ReviewExpired)
        );
        assert_eq!(
            review.large_files(SnapshotLargeFileRequest {
                record_version: FFI_RECORD_VERSION,
                minimum_logical_bytes: 1,
                modified_before: None,
                max_results: 1,
            }),
            Err(EngineError::ReviewExpired)
        );
        assert!(engine.close());
        assert_eq!(close_drained.renew(), Err(EngineError::Closed));
        assert!(close_drained.info().unwrap().released);
        assert_eq!(
            close_drained.release().unwrap(),
            ReviewReleaseOutcome::AlreadyReleased
        );
    }

    #[test]
    fn snapshot_live_path_failures_map_to_specific_path_free_errors() {
        for (core, ffi) in [
            (
                CoreReviewError::LiveTargetUnsupported,
                EngineError::SnapshotLiveTargetUnsupported,
            ),
            (
                CoreReviewError::LivePathUnavailable,
                EngineError::SnapshotLivePathUnavailable,
            ),
            (
                CoreReviewError::LivePathMissing,
                EngineError::SnapshotLivePathMissing,
            ),
            (
                CoreReviewError::LivePathSymlink,
                EngineError::SnapshotLivePathSymlink,
            ),
            (
                CoreReviewError::LivePathCrossVolume,
                EngineError::SnapshotLivePathCrossVolume,
            ),
            (
                CoreReviewError::LivePathChanged,
                EngineError::SnapshotLivePathChanged,
            ),
            (
                CoreReviewError::LivePathAccessDenied,
                EngineError::SnapshotLivePathAccessDenied,
            ),
        ] {
            assert_eq!(map_review_error(core), ffi);
            assert!(!ffi.to_string().contains('/'));
        }
    }

    #[test]
    fn every_core_snapshot_category_maps_to_one_stable_ffi_case() {
        for (core, ffi) in [
            (
                CoreReviewCategory::Unclassified,
                SnapshotStorageCategory::Unclassified,
            ),
            (
                CoreReviewCategory::DeveloperArtifact,
                SnapshotStorageCategory::DeveloperArtifact,
            ),
            (
                CoreReviewCategory::ApplicationCache,
                SnapshotStorageCategory::ApplicationCache,
            ),
            (
                CoreReviewCategory::BrowserCache,
                SnapshotStorageCategory::BrowserCache,
            ),
            (
                CoreReviewCategory::LogAndDiagnostic,
                SnapshotStorageCategory::LogAndDiagnostic,
            ),
            (
                CoreReviewCategory::InstallerAndDownload,
                SnapshotStorageCategory::InstallerAndDownload,
            ),
            (
                CoreReviewCategory::DeviceAndSimulatorData,
                SnapshotStorageCategory::DeviceAndSimulatorData,
            ),
            (
                CoreReviewCategory::CloudFile,
                SnapshotStorageCategory::CloudFile,
            ),
            (
                CoreReviewCategory::LargeReviewItem,
                SnapshotStorageCategory::LargeReviewItem,
            ),
            (
                CoreReviewCategory::ProtectedSystemData,
                SnapshotStorageCategory::ProtectedSystemData,
            ),
            (
                CoreReviewCategory::UnknownStorage,
                SnapshotStorageCategory::UnknownStorage,
            ),
        ] {
            assert_eq!(project_snapshot_category(core), ffi);
        }
    }

    #[test]
    fn concurrent_close_callers_observe_the_same_result() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let engine = Arc::new(engine);
        let barrier = Arc::new(std::sync::Barrier::new(9));
        let callers = (0..8)
            .map(|_| {
                let engine = Arc::clone(&engine);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    engine.close()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        for caller in callers {
            assert!(caller.join().unwrap());
        }
        assert!(engine.close());
    }

    #[test]
    fn close_is_idempotent_and_live_count_tracks_objects() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let baseline = live_engine_instance_count();
        let (_temp, engine) = engine();
        assert_eq!(live_engine_instance_count(), baseline + 1);
        assert!(engine.close());
        assert!(engine.close());
        drop(engine);
        assert_eq!(live_engine_instance_count(), baseline);
    }
}

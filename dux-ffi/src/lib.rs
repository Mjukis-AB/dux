//! Owned, versioned UniFFI boundary for the DUX macOS application.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use dux_core::engine::{
    CancelOutcome as CoreCancelOutcome, CandidateDetailError as CoreCandidateDetailError,
    CandidateEvaluationTaskFailureKind as CoreCandidateEvaluationFailure,
    CandidateEvaluationTaskStatus as CoreCandidateEvaluationStatus,
    CandidateHistoryError as CoreCandidateHistoryError,
    CandidateReviewCommand as CoreCandidateReviewCommand,
    CandidateReviewError as CoreCandidateReviewError,
    CapacityHistoryDisposition as CoreHistoryDisposition, CapacityTrend as CoreCapacityTrend,
    CapacityTrendChange as CoreCapacityTrendChange, CapacityTrendPoint as CoreCapacityTrendPoint,
    CapacityTrendPointSource as CoreCapacityTrendPointSource,
    CleanupExclusionSource as CoreCleanupExclusionSource,
    CleanupExclusions as CoreCleanupExclusions,
    CleanupExclusionsError as CoreCleanupExclusionsError,
    CleanupExclusionsUpdate as CoreCleanupExclusionsUpdate,
    CleanupHistoryCursor as CoreCleanupHistoryCursor,
    CleanupHistoryError as CoreCleanupHistoryError,
    DirectCargoCodeSignature as CoreDirectCargoCodeSignature,
    DirectCargoEnrollmentError as CoreDirectCargoEnrollmentError,
    DirectCargoEnrollmentPreview as CoreDirectCargoEnrollmentPreview,
    DirectCargoEnrollmentState as CoreDirectCargoEnrollmentState,
    DirectCargoEnrollmentStatus as CoreDirectCargoEnrollmentStatus,
    DirectCargoEnrollmentUpdate as CoreDirectCargoEnrollmentUpdate,
    DirectCargoSignatureClass as CoreDirectCargoSignatureClass,
    DiskPressurePolicy as CorePressurePolicy, DiskPressurePolicyError as CorePressurePolicyError,
    DiskPressurePolicySource as CorePressurePolicySource,
    DiskPressurePolicyUpdate as CorePressurePolicyUpdate,
    DurableCandidateEvaluationStatus as CoreDurableCandidateEvaluationStatus,
    DurableCandidateEvidence as CoreCandidateEvidence,
    DurableCandidateEvidencePage as CoreCandidateEvidencePage,
    DurableCandidatePathPage as CoreCandidatePathPage,
    DurableCandidateStatus as CoreCandidateStatus, DurableCandidateSummary as CoreCandidateSummary,
    DurableCleanupHistoryPage as CoreCleanupHistoryPage,
    DurableCleanupItemStatus as CoreCleanupItemStatus,
    DurableCleanupItemSummary as CoreCleanupItemSummary, DurableCleanupMode as CoreCleanupMode,
    DurableCleanupRecordFormat as CoreCleanupRecordFormat,
    DurableCleanupSessionId as CoreCleanupSessionId,
    DurableCleanupSessionObservation as CoreCleanupSessionObservation,
    DurableCleanupSessionStatus as CoreCleanupSessionStatus,
    DurableCleanupStatusCounts as CoreCleanupStatusCounts,
    DurableCleanupTrigger as CoreCleanupTrigger, DurableCleanupWarning as CoreCleanupWarning,
    DurableObservedPath as CoreObservedPath, DurableScanIssueKind as CoreDurableScanIssueKind,
    DurableScanStatus as CoreDurableScanStatus, EngineConfig, EngineHandle, EngineOpenError,
    HistoryMaintenanceStartOutcome, PermanentCleanupPolicy as CorePermanentCleanupPolicy,
    PermanentCleanupPolicyError as CorePermanentCleanupPolicyError,
    PermanentCleanupPolicySource as CorePermanentCleanupPolicySource,
    PermanentCleanupPolicyUpdate as CorePermanentCleanupPolicyUpdate,
    RustTargetPlanReview as CoreRustTargetPlanReview,
    RustTargetPlanReviewError as CoreRustTargetPlanReviewError,
    RustTargetPlanReviewInfo as CoreRustTargetPlanReviewInfo,
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
    AvailableCapacitySource as CoreCapacitySource, BlockReason as CoreBlockReason,
    CandidateAction as CoreCandidateAction, CandidateCategory as CoreCandidateCategory,
    CandidateId, CleanupMode as CorePlanCleanupMode, DatabaseOpenErrorKind,
    DiskPressure as CoreDiskPressure, DiskPressureConfig, DiskPressureConfigError,
    DiskPressureRecoveryMargin, DiskPressureThreshold, EvidenceKind as CoreEvidenceKind,
    PlanWarning as CorePlanWarning, SafetyTier as CoreSafetyTier,
    ScanCoverageStatus as CoreCoverageStatus, ScanId, SnapshotOpenErrorKind,
    TrashEffectTargetKind as CoreTrashEffectTargetKind,
    TrashPlatformResult as CoreTrashPlatformResult, TrashSelectionError as CoreTrashSelectionError,
    VolumeCapacity, VolumeId,
};

const FFI_CONTRACT_VERSION: u32 = 29;
const FFI_RECORD_VERSION: u32 = 1;
const SNAPSHOT_NODE_RECORD_VERSION: u32 = 2;
const SCAN_EVENT_PAGE_LIMIT: u16 = 64;
const RECENT_SCAN_HISTORY_PAGE_LIMIT: u16 = 200;
const SCAN_COVERAGE_DETAIL_PAGE_LIMIT: u16 = 64;
const CANDIDATE_DETAIL_PAGE_LIMIT: u16 = 64;
const MAX_CANDIDATE_ENCODED_PATH_BYTES: usize = 65_536;
const MAX_CANDIDATE_DISPLAY_PATH_BYTES: usize = MAX_CANDIDATE_ENCODED_PATH_BYTES * 4;
const MAX_CANDIDATE_DETAIL_PAGE_PAYLOAD_BYTES: usize = 24 * 1_024 * 1_024;
const MAX_CANDIDATE_IDENTIFIER_BYTES: usize = 4_096;
const MAX_CLEANUP_HISTORY_SESSION_ID_BYTES: usize = 128;
const MAX_CLEANUP_HISTORY_PLAN_ID_BYTES: usize = 128;
const MAX_CLEANUP_HISTORY_RULE_ID_BYTES: usize = 128;
const MAX_CLEANUP_HISTORY_ERROR_CATEGORY_BYTES: usize = 128;
const MAX_CLEANUP_HISTORY_ITEMS: usize = 64;
const MAX_CLEANUP_HISTORY_PATHS: u16 = 256;
const MAX_CLEANUP_HISTORY_EVIDENCE: u16 = 512;
const MAX_CLEANUP_HISTORY_WARNINGS: usize = 5;
const MAX_SCAN_ROOT_UTF8_BYTES: usize = 32 * 1_024;
const MAX_CLEANUP_EXCLUSION_COUNT: usize = 64;
const MAX_CLEANUP_EXCLUSION_PATH_BYTES: usize = 32 * 1_024;
const MAX_DIRECT_CARGO_EXECUTABLE_PATH_BYTES: usize = 32 * 1_024;
const MAX_DIRECT_CARGO_CODE_DIRECTORY_HASHES: usize = 16;
const MIN_DIRECT_CARGO_CODE_DIRECTORY_HASH_BYTES: usize = 20;
const MAX_DIRECT_CARGO_CODE_DIRECTORY_HASH_BYTES: usize = 64;
const MAX_DIRECT_CARGO_SIGNING_IDENTIFIER_BYTES: usize = 512;
const MAX_DIRECT_CARGO_TEAM_IDENTIFIER_BYTES: usize = 128;
const DIRECT_CARGO_SUPPORTED_RELEASE: [u32; 3] = [1, 96, 0];
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

/// Path-free request for bounded capacity changes and UTC-day trend points.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CapacityTrendRequest {
    pub record_version: u32,
    pub stable_volume_id: String,
    pub anchor_at_unix_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CapacityTrendChange {
    pub record_version: u32,
    pub from_unix_ms: i64,
    pub to_unix_ms: i64,
    pub total_bytes: i64,
    pub available_bytes: i64,
    pub important_available_bytes: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CapacityTrendPointSource {
    Raw,
    DailyRollup,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CapacityTrendPoint {
    pub record_version: u32,
    pub sampled_at_unix_ms: i64,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub important_available_bytes: Option<u64>,
    pub pressure: VolumePressure,
    pub source: CapacityTrendPointSource,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CapacityTrendStatus {
    pub record_version: u32,
    pub stable_volume_id: String,
    pub sampled_at_unix_ms: i64,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub important_available_bytes: Option<u64>,
    pub pressure: VolumePressure,
    pub change_24h: Option<CapacityTrendChange>,
    pub change_7d: Option<CapacityTrendChange>,
    pub points: Vec<CapacityTrendPoint>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PermanentCleanupPolicySource {
    Default,
    Stored,
}

/// Versioned, path-free global permanent-cleanup kill switch. It never
/// selects a target or grants execution authority.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PermanentCleanupPolicyStatus {
    pub record_version: u32,
    pub enabled: bool,
    pub source: PermanentCleanupPolicySource,
    pub revision: u64,
    pub updated_at_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PermanentCleanupPolicyUpdate {
    pub record_version: u32,
    pub policy: PermanentCleanupPolicyStatus,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum PermanentCleanupPolicyError {
    #[error("engine session is closed")]
    Closed,
    #[error("permanent-cleanup settings are corrupt")]
    CorruptData,
    #[error("permanent-cleanup settings are unavailable")]
    Unavailable,
    #[error("the settings write outcome could not be proven")]
    OutcomeUnknown,
    #[error("the permanent-cleanup policy revision cannot advance")]
    RevisionExhausted,
    #[error("the system clock cannot be represented by the settings store")]
    InvalidClock,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the settings query exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("engine settings state is unavailable")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupExclusionsSource {
    Default,
    Stored,
}

/// A lossless absolute lexical path prefix from the deny-only cleanup
/// exclusion setting. The bytes are an observation payload only: they do not
/// select a plan, grant access, or authorize a filesystem effect.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupExclusionPath {
    pub encoding: SnapshotNameEncoding,
    pub encoded_bytes: Vec<u8>,
}

/// Versioned input for replacing the bounded user exclusion set. Core owns
/// absolute-path, component, ordering, and duplicate validation.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupExclusionsInput {
    pub record_version: u32,
    pub paths: Vec<CleanupExclusionPath>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupExclusionsStatus {
    pub record_version: u32,
    pub paths: Vec<CleanupExclusionPath>,
    pub source: CleanupExclusionsSource,
    pub revision: u64,
    pub updated_at_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupExclusionsUpdate {
    pub record_version: u32,
    pub exclusions: CleanupExclusionsStatus,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum CleanupExclusionsError {
    #[error("engine session is closed")]
    Closed,
    #[error("cleanup-exclusion record version is unsupported")]
    InvalidRecordVersion,
    #[error("cleanup exclusion path bytes are invalid")]
    InvalidPath,
    #[error("the cleanup exclusion set exceeds its fixed bound")]
    TooManyPaths,
    #[error("the cleanup-exclusion revision cannot advance")]
    RevisionExhausted,
    #[error("the system clock cannot be represented by the settings store")]
    InvalidClock,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the settings query exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("cleanup exclusions are corrupt")]
    CorruptData,
    #[error("cleanup exclusions are unavailable")]
    Unavailable,
    #[error("the settings write outcome could not be proven")]
    OutcomeUnknown,
    #[error("engine settings state is unavailable")]
    InternalState,
}

/// Lossless host path for one explicitly chosen Cargo executable. These bytes
/// identify discovery tooling only; they are never a cleanup target.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DirectCargoExecutablePath {
    pub encoding: SnapshotNameEncoding,
    pub encoded_bytes: Vec<u8>,
}

/// Versioned request to inspect one exact Cargo executable without changing
/// durable settings. The adapter accepts no PATH lookup or command text.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DirectCargoEnrollmentInspectionRequest {
    pub record_version: u32,
    pub path_encoding: SnapshotNameEncoding,
    pub executable_path_bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum DirectCargoSignatureClass {
    AdHoc,
    Cms,
}

/// One bounded Code Directory digest from macOS static-code inspection.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DirectCargoCodeDirectoryHash {
    pub record_version: u32,
    pub bytes: Vec<u8>,
}

/// Exact bounded static-code evidence. This proves local byte identity, not a
/// publisher identity and not cleanup authority.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DirectCargoCodeSignature {
    pub record_version: u32,
    pub class: DirectCargoSignatureClass,
    pub flags: u32,
    pub code_directory_hashes: Vec<DirectCargoCodeDirectoryHash>,
    pub signing_identifier: String,
    pub team_identifier: Option<String>,
    pub designated_requirement_sha256: Option<Vec<u8>>,
}

/// Read-only evidence held by an opaque inspection session until it is either
/// consumed by enrollment or explicitly released.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DirectCargoEnrollmentPreviewInfo {
    pub record_version: u32,
    pub executable_path: DirectCargoExecutablePath,
    pub executable_sha256: Vec<u8>,
    pub code_signature: DirectCargoCodeSignature,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum DirectCargoEnrollmentState {
    NotEnrolled,
    Enrolled,
    Revoked,
}

/// Exact enrolled identity. It grants permission to use this Cargo only for
/// deterministic discovery and cannot name or authorize a cleanup target.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DirectCargoEnrollmentIdentity {
    pub record_version: u32,
    pub executable_path: DirectCargoExecutablePath,
    pub executable_sha256: Vec<u8>,
    pub version_sha256: Vec<u8>,
    pub cargo_major: u32,
    pub cargo_minor: u32,
    pub cargo_patch: u32,
    pub code_signature: DirectCargoCodeSignature,
}

/// Revisioned durable enrollment state. `identity` is present exactly for the
/// Enrolled state.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DirectCargoEnrollmentStatus {
    pub record_version: u32,
    pub revision: u64,
    pub state: DirectCargoEnrollmentState,
    pub identity: Option<DirectCargoEnrollmentIdentity>,
    pub updated_at_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DirectCargoEnrollmentUpdate {
    pub record_version: u32,
    pub status: DirectCargoEnrollmentStatus,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum DirectCargoEnrollmentPreviewReleaseOutcome {
    Released,
    AlreadyUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum DirectCargoEnrollmentError {
    #[error("engine session is closed")]
    Closed,
    #[error("direct Cargo enrollment is supported only on macOS")]
    UnsupportedPlatform,
    #[error("direct Cargo enrollment record version is unsupported")]
    InvalidRecordVersion,
    #[error("the Cargo executable path is invalid or exceeds its fixed bound")]
    InvalidExecutablePath,
    #[error("the Cargo executable is not a regular file")]
    ExecutableNotRegular,
    #[error("the Cargo executable or its environment changed during inspection")]
    ChangedDuringInspection,
    #[error("Cargo inspection could not run to completion")]
    InspectionUnavailable,
    #[error("Cargo inspection exceeded its fixed time or output budget")]
    InspectionLimitExceeded,
    #[error("Cargo resolution directories are not canonical directories")]
    InvalidResolutionEnvironment,
    #[error("Cargo verbose version is invalid or unsupported")]
    InvalidCargoVersion,
    #[error("Cargo does not have valid bounded macOS code-signing evidence")]
    InvalidCodeSignature,
    #[error("the inspection preview belongs to a different engine session")]
    WrongEngine,
    #[error("the inspection preview was already consumed or released")]
    PreviewUnavailable,
    #[error("the enrollment revision cannot advance")]
    RevisionExhausted,
    #[error("the system clock cannot be represented by the settings store")]
    InvalidClock,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the settings query exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("direct Cargo enrollment is corrupt")]
    CorruptData,
    #[error("direct Cargo enrollment is unavailable")]
    Unavailable,
    #[error("the enrollment write outcome could not be proven")]
    OutcomeUnknown,
    #[error("direct Cargo enrollment state is unavailable")]
    InternalState,
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
    #[error("candidate detail request is invalid")]
    InvalidCandidateDetailRequest,
    #[error("the scan has no successful candidate evaluation")]
    CandidateEvaluationNotSucceeded,
    #[error("the requested candidate does not belong to this scan")]
    CandidateNotFound,
    #[error("the candidate detail cursor is outside the immutable observation")]
    CandidateCursorOutOfRange,
    #[error("the requested candidate review command is not valid")]
    CandidateReviewNotReviewable,
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

/// Typed, path-free observations emitted while a scan task runs. Maintenance
/// events are intentionally coalesced into `Maintenance` because they are
/// engine bookkeeping, not user-facing scan progress.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScanEventKind {
    Queued,
    Started,
    Progress {
        completed: u64,
        total: u64,
    },
    ScanProgress {
        files_scanned: u64,
        directories_scanned: u64,
        known_allocated_bytes: u64,
        error_count: u64,
    },
    ScanFinalizing,
    CandidateEvaluationStarted,
    CandidateEvaluationFinished {
        status: ScanCandidateEvaluationStatus,
        candidate_count: u32,
        failure: Option<ScanCandidateEvaluationFailure>,
    },
    CancellationRequested,
    Terminal {
        phase: TaskPhase,
    },
    Maintenance,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScanEvent {
    pub record_version: u32,
    pub sequence: u64,
    pub kind: ScanEventKind,
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

/// Opaque keyset cursor for the path-free cleanup-history feed. It is a
/// position observation only and cannot be used to resume or authorize a
/// cleanup operation.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupHistoryCursor {
    pub record_version: u32,
    pub started_at_unix_ms: i64,
    pub session_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupRecordFormat {
    LegacyIncomplete,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupMode {
    DryRun,
    Trash,
    PermanentSafe,
    EvictLocalCopy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupTrigger {
    Manual,
    LowDisk,
    Scheduled,
    Cli,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupSessionStatus {
    Planned,
    Running,
    Recovering,
    Completed,
    PartiallyCompleted,
    Failed,
    Cancelled,
    Interrupted,
    Rejected,
    DryRun,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupItemStatus {
    Planned,
    Validating,
    DryRun,
    EffectStarted,
    Trashed,
    Removed,
    Evicted,
    Skipped,
    Rejected,
    Failed,
    ChangedSincePlan,
    Interrupted,
    Unavailable,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupWarning {
    EstimatedBytesUnverified,
    DryRunDoesNotMutate,
    TrashDoesNotFreeSpaceImmediately,
    PermanentRemovalCannotBeUndone,
    CloudEvictionRequiresNetworkToRedownload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupStatusCounts {
    pub planned: u16,
    pub validating: u16,
    pub dry_run: u16,
    pub effect_started: u16,
    pub trashed: u16,
    pub removed: u16,
    pub evicted: u16,
    pub skipped: u16,
    pub rejected: u16,
    pub failed: u16,
    pub changed_since_plan: u16,
    pub interrupted: u16,
    pub unavailable: u16,
    pub outcome_unknown: u16,
    pub total: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupSessionSummary {
    pub record_version: u32,
    pub session_id: String,
    pub plan_id: String,
    pub format: CleanupRecordFormat,
    pub source_scan_id: Option<String>,
    pub started_at_unix_ms: i64,
    pub completed_at_unix_ms: Option<i64>,
    pub plan_created_at_unix_ms: Option<i64>,
    pub plan_expires_at_unix_ms: Option<i64>,
    pub mode: CleanupMode,
    pub trigger: CleanupTrigger,
    pub status: CleanupSessionStatus,
    pub estimated_bytes: u64,
    pub verified_capacity_delta_bytes: Option<i64>,
    pub cancellation_requested: Option<bool>,
    pub item_total: u16,
    pub path_total: u16,
    pub evidence_total: u16,
    pub item_status_counts: CleanupStatusCounts,
    pub path_status_counts: CleanupStatusCounts,
}

/// Select one exact path-free history observation by the stable session ID
/// copied from [`CleanupSessionSummary`]. The token grants no recovery,
/// approval, journal, or executor authority.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupSessionHistoryRequest {
    pub record_version: u32,
    pub session_id: String,
}

/// One ordered path-free item observation from a fully validated cleanup
/// session. Path values, evidence payloads, candidate IDs, and execution
/// fences remain sealed inside core.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupItemSummary {
    pub record_version: u32,
    pub ordinal: u16,
    pub rule_id: String,
    pub rule_revision: u32,
    pub category: Option<CandidateCategory>,
    pub safety: Option<CandidateSafety>,
    pub action: Option<CandidateAction>,
    pub rule_schedule_eligible: Option<bool>,
    pub newest_mtime_unix_ms: Option<i64>,
    pub estimated_bytes: u64,
    pub status: CleanupItemStatus,
    pub error_recorded: bool,
    pub error_category: Option<String>,
    pub path_count: u16,
    pub evidence_count: u16,
}

/// Fully validated exact-session cleanup history. This is immutable,
/// path-free presentation data and cannot be supplied to any planner,
/// approval, recovery, journal, retry, or executor operation.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupSessionHistory {
    pub record_version: u32,
    pub summary: CleanupSessionSummary,
    pub items: Vec<CleanupItemSummary>,
    pub warnings: Vec<CleanupWarning>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupHistoryPage {
    pub record_version: u32,
    pub records: Vec<CleanupSessionSummary>,
    pub next_cursor: Option<CleanupHistoryCursor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum CleanupHistoryError {
    #[error("engine session is closed")]
    Closed,
    #[error("the cleanup-history record version is unsupported")]
    InvalidRecordVersion,
    #[error("the cleanup-history session ID is invalid")]
    InvalidSessionId,
    #[error("cleanup history limit is outside its fixed bound")]
    InvalidLimit,
    #[error("cleanup history cursor is invalid")]
    InvalidCursor,
    #[error("the requested cleanup session does not exist")]
    SessionNotFound,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the cleanup history query exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("durable cleanup history is corrupt")]
    CorruptData,
    #[error("durable cleanup history is unavailable")]
    Unavailable,
    #[error("cleanup history state is unavailable")]
    InternalState,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CandidatePathEncoding {
    Utf8,
    Utf16LittleEndian,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CandidateCategory {
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
pub enum CandidateSafety {
    SafeRegenerable,
    SafeEvictable,
    ReviewRequired,
    Informational,
    Protected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CandidateAction {
    RemoveKnownRegenerableContents,
    EvictLocalCopy,
    MoveToTrash,
    RevealOnly,
    NoAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CandidateStatus {
    Discovered,
    Selected,
    Dismissed,
    Stale,
    Planned,
    Completed,
    Failed,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CandidateEvidenceKind {
    MatchedPath,
    RequiredMarker,
    ForbiddenMarkerAbsent,
    BundleIdentifier,
    MinimumAge,
    MinimumSize,
    InactiveProcess,
    CloudUploadComplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CandidateBlockReason {
    MissingOrIncompleteEvidence,
    MissingModificationTime,
    PartialScanCoverage,
    RecentActivity,
    BelowMinimumBytes,
    ActiveUse,
    AccessDenied,
    ProtectedPath,
    ProtectedDescendant,
    SymlinkBoundary,
    VolumeBoundary,
    ChangedSinceScan,
    UnsupportedPlatform,
    CloudUploadUnconfirmed,
}

/// Review intent is a bounded, scan-bound selection observation. It cannot
/// create a plan or authorize an effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CandidateReviewCommand {
    Select,
    ClearSelection,
    Dismiss,
    Restore,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CandidateReviewResult {
    pub record_version: u32,
    pub scan_id: String,
    pub candidate_id: String,
    pub status: CandidateStatus,
}

/// A historical candidate path/evidence observation. It is intentionally
/// scoped to a retained review and can never be passed back as an execution
/// or planning input.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CandidateObservedPath {
    pub encoding: CandidatePathEncoding,
    pub encoded_bytes: Vec<u8>,
    pub display: String,
}

/// Exact current plan path for presentation only. This observation cannot be
/// supplied back to Rust as planner or executor input.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RustTargetPlanReviewPath {
    pub encoding: SnapshotNameEncoding,
    pub encoded_bytes: Vec<u8>,
    pub display: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RustTargetPlanReviewRequest {
    pub record_version: u32,
    pub candidate_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RustTargetPlanReviewInfo {
    pub record_version: u32,
    pub plan_id: String,
    pub source_scan_id: String,
    pub candidate_id: String,
    pub rule_id: String,
    pub rule_revision: u32,
    pub category: CandidateCategory,
    pub mode: CleanupMode,
    pub safety: CandidateSafety,
    pub action: CandidateAction,
    pub estimated_bytes: u64,
    pub warnings: Vec<CleanupWarning>,
    pub created_at_unix_ms: i64,
    pub effective_expires_at_unix_ms: i64,
    pub schedule_eligible: bool,
    pub item_count: u16,
    pub path_count: u16,
    pub path: RustTargetPlanReviewPath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RustTargetPlanReviewReleaseOutcome {
    Released,
    AlreadyUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum RustTargetPlanReviewError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the supplied snapshot review belongs to another engine")]
    WrongEngine,
    #[error("the Rust-target plan-review record version is unsupported")]
    InvalidRecordVersion,
    #[error("the exact parent snapshot review is released or expired")]
    ParentReviewUnavailable,
    #[error("the exact Rust-target plan review expired")]
    ReviewExpired,
    #[error("the exact Rust-target candidate is unavailable for review")]
    CandidateUnavailable,
    #[error("direct Cargo enrollment is required before this plan can be reviewed")]
    CargoNotEnrolled,
    #[error("Cargo or rustc is active")]
    ActiveProcesses,
    #[error("the Rust-target plan evidence changed during review")]
    ChangedDuringReview,
    #[error("Rust-target plan review is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("the bounded plan-review operation exceeded its resource budget")]
    BudgetExceeded,
    #[error("the plan-review store is temporarily busy")]
    Busy,
    #[error("the plan-review store is unsafe")]
    UnsafeStorage,
    #[error("the plan-review store is corrupt")]
    CorruptData,
    #[error("the plan-review store is unavailable")]
    Unavailable,
    #[error("another Rust-target plan review is already active")]
    ReviewBusy,
    #[error("the Rust-target plan review is no longer available")]
    ReviewUnavailable,
    #[error("the plan-review state is internally unavailable")]
    InternalState,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CandidateSummary {
    pub record_version: u32,
    pub candidate_id: String,
    pub rule_id: String,
    pub rule_revision: u32,
    pub category: CandidateCategory,
    pub estimated_bytes: u64,
    pub newest_mtime: Option<SnapshotNodeTimestamp>,
    pub safety: CandidateSafety,
    pub action: CandidateAction,
    pub rule_schedule_eligible: bool,
    pub path_count: u16,
    pub evidence_kinds: Vec<CandidateEvidenceKind>,
    pub blockers: Vec<CandidateBlockReason>,
    pub created_at: SnapshotNodeTimestamp,
    pub status: CandidateStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CandidateSummaryPage {
    pub record_version: u32,
    pub scan_id: String,
    pub cursor: u16,
    pub next_cursor: Option<u16>,
    pub total_candidates: u16,
    pub candidates: Vec<CandidateSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CandidatePathPage {
    pub record_version: u32,
    pub scan_id: String,
    pub candidate: CandidateSummary,
    pub cursor: u16,
    pub next_cursor: Option<u16>,
    pub total_paths: u16,
    pub paths: Vec<CandidateObservedPath>,
}

/// Typed historical evidence. Optional fields are populated only for the
/// corresponding `kind`; Swift must reject contradictory shapes.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CandidateEvidenceRecord {
    pub record_version: u32,
    pub ordinal: u16,
    pub kind: CandidateEvidenceKind,
    pub path: Option<CandidateObservedPath>,
    pub identifier: Option<String>,
    pub newest_mtime: Option<SnapshotNodeTimestamp>,
    pub minimum_age_seconds: Option<u64>,
    pub minimum_age_nanoseconds: Option<u32>,
    pub observed_bytes: Option<u64>,
    pub minimum_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CandidateEvidencePage {
    pub record_version: u32,
    pub scan_id: String,
    pub candidate: CandidateSummary,
    pub cursor: u16,
    pub next_cursor: Option<u16>,
    pub total_evidence: u16,
    pub evidence: Vec<CandidateEvidenceRecord>,
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

/// One path-free aggregate observation plus the next bounded page of typed
/// task events. `next_event_sequence` is the last delivered event sequence
/// (the cursor to pass on the next poll), not a one-past count. The
/// `events_truncated` bit is sticky when the requested cursor predates the
/// bounded core ring.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScanPoll {
    pub record_version: u32,
    pub phase: TaskPhase,
    pub stage: ScanStage,
    pub cancellation_requested: bool,
    pub revision: u64,
    pub progress: Option<ScanProgress>,
    pub events: Vec<ScanEvent>,
    pub next_event_sequence: u64,
    pub oldest_available_event_sequence: u64,
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

/// The only target payload a future core-owned Trash callback may receive.
///
/// There is intentionally no UniFFI constructor. Rust creates this object
/// only after a reviewed-plan admission has revalidated the target and fenced
/// the journal receipt. The request is ephemeral and its path bytes can be
/// consumed once by the synchronous platform adapter; it is not a plan,
/// approval, or reusable filesystem capability.
#[derive(uniffi::Object)]
pub struct TrashEffectRequest {
    state: Mutex<Option<TrashEffectRequestState>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrashEffectRequestState {
    record_version: u32,
    target_kind: TrashEffectTargetKind,
    path_encoding: SnapshotNameEncoding,
    absolute_path_bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TrashEffectTargetKind {
    Directory,
    File,
    Symlink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum TrashEffectRequestError {
    #[error("the Trash request is no longer available")]
    Consumed,
    #[error("the Trash request contains an invalid path")]
    InvalidPath,
    #[error("the Trash request state is unavailable")]
    InternalState,
}

#[uniffi::export]
impl TrashEffectRequest {
    /// Return the stable record version for this one-shot request.
    pub fn record_version(&self) -> Result<u32, TrashEffectRequestError> {
        self.with_state(|state| state.record_version)
    }

    /// Return the no-follow kind captured by core before the callback began.
    pub fn target_kind(&self) -> Result<TrashEffectTargetKind, TrashEffectRequestError> {
        self.with_state(|state| state.target_kind)
    }

    /// Return the encoding of the exact path bytes captured by core.
    pub fn path_encoding(&self) -> Result<SnapshotNameEncoding, TrashEffectRequestError> {
        self.with_state(|state| state.path_encoding)
    }

    /// Consume the exact path bytes once. A callback must not retain or retry
    /// this value after returning to Rust.
    pub fn take_path_bytes(&self) -> Result<Vec<u8>, TrashEffectRequestError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| TrashEffectRequestError::InternalState)?;
        let state = state.take().ok_or(TrashEffectRequestError::Consumed)?;
        if state.absolute_path_bytes.is_empty() {
            return Err(TrashEffectRequestError::InvalidPath);
        }
        Ok(state.absolute_path_bytes)
    }
}

impl TrashEffectRequest {
    fn from_core(
        target_kind: CoreTrashEffectTargetKind,
        absolute_path_bytes: Vec<u8>,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(Some(TrashEffectRequestState {
                record_version: FFI_RECORD_VERSION,
                target_kind: match target_kind {
                    CoreTrashEffectTargetKind::Directory => TrashEffectTargetKind::Directory,
                    CoreTrashEffectTargetKind::File => TrashEffectTargetKind::File,
                    CoreTrashEffectTargetKind::Symlink => TrashEffectTargetKind::Symlink,
                },
                path_encoding: SnapshotNameEncoding::UnixBytes,
                absolute_path_bytes,
            })),
        })
    }

    fn with_state<T>(
        &self,
        operation: impl FnOnce(&TrashEffectRequestState) -> T,
    ) -> Result<T, TrashEffectRequestError> {
        let state = self
            .state
            .lock()
            .map_err(|_| TrashEffectRequestError::InternalState)?;
        let state = state.as_ref().ok_or(TrashEffectRequestError::Consumed)?;
        Ok(operation(state))
    }

    #[cfg(test)]
    fn for_test(
        target_kind: TrashEffectTargetKind,
        path_encoding: SnapshotNameEncoding,
        absolute_path_bytes: Vec<u8>,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(Some(TrashEffectRequestState {
                record_version: FFI_RECORD_VERSION,
                target_kind,
                path_encoding,
                absolute_path_bytes,
            })),
        })
    }
}

/// Synchronous platform callback contract for the future reviewed Trash
/// executor. The callback returns only a bounded outcome; it cannot approve,
/// journal, retry, or choose a path.
#[uniffi::export(callback_interface)]
pub trait TrashPlatformDriver: Send + Sync {
    fn trash(&self, request: Arc<TrashEffectRequest>) -> TrashPlatformResult;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TrashPlatformResult {
    Completed,
    Unsupported,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum TrashExecutionError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the Explorer Trash request is invalid")]
    InvalidRequest,
    #[error("the reviewed Explorer Trash target changed since the plan was created")]
    ChangedSincePlan,
    #[error("the retained Explorer review cannot supply this Trash target")]
    ReviewUnavailable,
    #[error("the cleanup journal is temporarily busy")]
    Busy,
    #[error("the cleanup store is unavailable")]
    StorageUnavailable,
    #[error("the cleanup store is unsafe")]
    UnsafeStorage,
    #[error("the cleanup schema is incompatible")]
    IncompatibleSchema,
    #[error("the cleanup journal is corrupt")]
    CorruptData,
    #[error("the cleanup operation outcome is unknown")]
    OutcomeUnknown,
    #[error("the cleanup engine is internally unavailable")]
    InternalState,
}

enum DirectCargoEnrollmentPreviewState {
    Available(Box<CoreDirectCargoEnrollmentPreview>),
    Consumed,
    Released,
}

enum RustTargetPlanReviewState {
    Available(Box<CoreRustTargetPlanReview>),
    Inspecting,
    ReleasePending,
    Released,
}

#[derive(Default)]
struct RustTargetPlanPreparationTracker {
    state: Mutex<RustTargetPlanOperationState>,
    drained: Condvar,
}

#[derive(Default)]
struct RustTargetPlanOperationState {
    active: usize,
    preparation_active: bool,
}

struct RustTargetPlanPreparationGuard {
    tracker: Arc<RustTargetPlanPreparationTracker>,
    preparation: bool,
}

impl RustTargetPlanPreparationTracker {
    fn enter_operation(
        self: &Arc<Self>,
        closed: &AtomicBool,
    ) -> Result<RustTargetPlanPreparationGuard, RustTargetPlanReviewError> {
        self.enter(closed, false)
    }

    fn enter_preparation(
        self: &Arc<Self>,
        closed: &AtomicBool,
    ) -> Result<RustTargetPlanPreparationGuard, RustTargetPlanReviewError> {
        self.enter(closed, true)
    }

    fn enter_cleanup(
        self: &Arc<Self>,
    ) -> Result<RustTargetPlanPreparationGuard, RustTargetPlanReviewError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RustTargetPlanReviewError::InternalState)?;
        state.active = state
            .active
            .checked_add(1)
            .ok_or(RustTargetPlanReviewError::BudgetExceeded)?;
        Ok(RustTargetPlanPreparationGuard {
            tracker: Arc::clone(self),
            preparation: false,
        })
    }

    fn enter(
        self: &Arc<Self>,
        closed: &AtomicBool,
        preparation: bool,
    ) -> Result<RustTargetPlanPreparationGuard, RustTargetPlanReviewError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RustTargetPlanReviewError::InternalState)?;
        if closed.load(Ordering::Acquire) {
            return Err(RustTargetPlanReviewError::Closed);
        }
        if preparation && state.preparation_active {
            return Err(RustTargetPlanReviewError::ReviewBusy);
        }
        state.active = state
            .active
            .checked_add(1)
            .ok_or(RustTargetPlanReviewError::BudgetExceeded)?;
        state.preparation_active |= preparation;
        Ok(RustTargetPlanPreparationGuard {
            tracker: Arc::clone(self),
            preparation,
        })
    }

    fn wait_until(&self, deadline: Instant) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.active != 0 {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let (next, timed_out) = self
                .drained
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
            if timed_out.timed_out() && state.active != 0 {
                return false;
            }
        }
        true
    }
}

impl Drop for RustTargetPlanPreparationGuard {
    fn drop(&mut self) {
        let mut state = self
            .tracker
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active = state.active.saturating_sub(1);
        if self.preparation {
            state.preparation_active = false;
        }
        if state.active == 0 {
            self.tracker.drained.notify_all();
        }
    }
}

/// Engine-bound, observation-only plan review. The retained core plan cannot
/// be cloned, approved, persisted, or executed through this surface.
#[derive(uniffi::Object)]
pub struct RustTargetPlanReviewSession {
    state: Mutex<RustTargetPlanReviewState>,
    parent_review: Weak<SnapshotReviewSession>,
    operations: Arc<RustTargetPlanPreparationTracker>,
    engine_closed: Arc<AtomicBool>,
}

#[uniffi::export]
impl RustTargetPlanReviewSession {
    pub fn info(&self) -> Result<RustTargetPlanReviewInfo, RustTargetPlanReviewError> {
        let _operation = self.operations.enter_operation(&self.engine_closed)?;
        let Some(parent) = self.parent_review.upgrade() else {
            let _ = self.release_inner();
            return Err(RustTargetPlanReviewError::ParentReviewUnavailable);
        };
        if let Err(error) = self.validate_parent(&parent) {
            if error == RustTargetPlanReviewError::ParentReviewUnavailable {
                let _ = self.release_inner();
            }
            return Err(error);
        }
        let review = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| RustTargetPlanReviewError::InternalState)?;
            let RustTargetPlanReviewState::Available(_) = &*state else {
                return Err(RustTargetPlanReviewError::ReviewUnavailable);
            };
            let RustTargetPlanReviewState::Available(review) =
                std::mem::replace(&mut *state, RustTargetPlanReviewState::Inspecting)
            else {
                unreachable!("available state was just matched")
            };
            review
        };
        let observation = review
            .info()
            .map_err(map_rust_target_plan_review_error)
            .and_then(project_rust_target_plan_review_info);
        let parent_result = self.validate_parent(&parent);
        let closed = self.engine_closed.load(Ordering::Acquire);
        let mut state = self
            .state
            .lock()
            .map_err(|_| RustTargetPlanReviewError::InternalState)?;
        let release_pending = matches!(*state, RustTargetPlanReviewState::ReleasePending);
        let should_release = closed
            || release_pending
            || is_terminal_plan_review_result(&observation)
            || matches!(
                parent_result,
                Err(RustTargetPlanReviewError::ParentReviewUnavailable)
            );
        let mut review = Some(review);
        if should_release {
            *state = RustTargetPlanReviewState::Released;
        } else {
            *state = RustTargetPlanReviewState::Available(
                review
                    .take()
                    .expect("the inspected review must still be locally retained"),
            );
        }
        drop(state);
        if let Some(review) = review {
            review.release();
        }
        if closed {
            return Err(RustTargetPlanReviewError::Closed);
        }
        if release_pending {
            return Err(RustTargetPlanReviewError::ReviewUnavailable);
        }
        parent_result?;
        observation
    }

    pub fn release(&self) -> Result<RustTargetPlanReviewReleaseOutcome, RustTargetPlanReviewError> {
        self.release_inner()
    }
}

impl RustTargetPlanReviewSession {
    fn validate_parent(
        &self,
        parent: &SnapshotReviewSession,
    ) -> Result<(), RustTargetPlanReviewError> {
        let parent_session = parent
            .inner
            .lock()
            .map_err(|_| RustTargetPlanReviewError::InternalState)?;
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(RustTargetPlanReviewError::Closed);
        }
        parent_session
            .validate_for_plan_review()
            .map(|_| ())
            .map_err(map_parent_plan_review_error)
    }

    fn occupies_capacity(&self) -> Result<bool, RustTargetPlanReviewError> {
        let Some(parent) = self.parent_review.upgrade() else {
            let _ = self.release_inner()?;
            return Ok(false);
        };
        if let Err(error) = self.validate_parent(&parent) {
            if error == RustTargetPlanReviewError::ParentReviewUnavailable {
                let _ = self.release_inner()?;
                return Ok(false);
            }
            return Err(error);
        }
        self.occupies_capacity_quick()
    }

    fn occupies_capacity_quick(&self) -> Result<bool, RustTargetPlanReviewError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RustTargetPlanReviewError::InternalState)?;
        match &*state {
            RustTargetPlanReviewState::Available(review) => {
                if review.terminal_error().is_none() {
                    return Ok(true);
                }
                let RustTargetPlanReviewState::Available(review) =
                    std::mem::replace(&mut *state, RustTargetPlanReviewState::Released)
                else {
                    unreachable!("available state was just matched")
                };
                drop(state);
                review.release();
                Ok(false)
            }
            RustTargetPlanReviewState::Inspecting => Ok(true),
            RustTargetPlanReviewState::ReleasePending | RustTargetPlanReviewState::Released => {
                Ok(false)
            }
        }
    }

    fn occupies_capacity_state_only(&self) -> Result<bool, RustTargetPlanReviewError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RustTargetPlanReviewError::InternalState)?;
        Ok(matches!(
            *state,
            RustTargetPlanReviewState::Available(_) | RustTargetPlanReviewState::Inspecting
        ))
    }

    fn release_inner(
        &self,
    ) -> Result<RustTargetPlanReviewReleaseOutcome, RustTargetPlanReviewError> {
        let _operation = self.operations.enter_cleanup()?;
        let (outcome, review) = self.take_for_release()?;
        if let Some(review) = review {
            review.release();
        }
        Ok(outcome)
    }

    fn take_for_release(
        &self,
    ) -> Result<
        (
            RustTargetPlanReviewReleaseOutcome,
            Option<Box<CoreRustTargetPlanReview>>,
        ),
        RustTargetPlanReviewError,
    > {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RustTargetPlanReviewError::InternalState)?;
        match std::mem::replace(&mut *state, RustTargetPlanReviewState::Released) {
            RustTargetPlanReviewState::Available(review) => {
                Ok((RustTargetPlanReviewReleaseOutcome::Released, Some(review)))
            }
            RustTargetPlanReviewState::Inspecting => {
                *state = RustTargetPlanReviewState::ReleasePending;
                Ok((RustTargetPlanReviewReleaseOutcome::Released, None))
            }
            RustTargetPlanReviewState::ReleasePending | RustTargetPlanReviewState::Released => {
                Ok((RustTargetPlanReviewReleaseOutcome::AlreadyUnavailable, None))
            }
        }
    }

    fn release_for_close(&self) {
        let Ok(operation) = self.operations.enter_cleanup() else {
            return;
        };
        let Ok((_, review)) = self.take_for_release() else {
            return;
        };
        let Some(review) = review else {
            return;
        };
        std::thread::spawn(move || {
            review.release();
            drop(operation);
        });
    }
}

/// Engine-bound, consume-once inspection capability. The object carries only
/// Cargo discovery enrollment authority; it cannot create or execute cleanup.
#[derive(uniffi::Object)]
pub struct DirectCargoEnrollmentPreviewSession {
    state: Mutex<DirectCargoEnrollmentPreviewState>,
    info: DirectCargoEnrollmentPreviewInfo,
    engine_closed: Arc<AtomicBool>,
}

#[uniffi::export]
impl DirectCargoEnrollmentPreviewSession {
    /// Return immutable static inspection evidence while this preview remains
    /// available. No selected executable bytes are run by this call.
    pub fn info(&self) -> Result<DirectCargoEnrollmentPreviewInfo, DirectCargoEnrollmentError> {
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(DirectCargoEnrollmentError::Closed);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| DirectCargoEnrollmentError::InternalState)?;
        match &*state {
            DirectCargoEnrollmentPreviewState::Available(_) => Ok(self.info.clone()),
            DirectCargoEnrollmentPreviewState::Consumed
            | DirectCargoEnrollmentPreviewState::Released => {
                Err(DirectCargoEnrollmentError::PreviewUnavailable)
            }
        }
    }

    /// Explicitly discard this preview. Releasing an already consumed or
    /// released preview is an idempotent no-op.
    pub fn release(
        &self,
    ) -> Result<DirectCargoEnrollmentPreviewReleaseOutcome, DirectCargoEnrollmentError> {
        self.release_inner()
    }
}

impl DirectCargoEnrollmentPreviewSession {
    fn is_available(&self) -> Result<bool, DirectCargoEnrollmentError> {
        let state = self
            .state
            .lock()
            .map_err(|_| DirectCargoEnrollmentError::InternalState)?;
        Ok(matches!(
            *state,
            DirectCargoEnrollmentPreviewState::Available(_)
        ))
    }

    fn take_for_commit(
        &self,
    ) -> Result<CoreDirectCargoEnrollmentPreview, DirectCargoEnrollmentError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DirectCargoEnrollmentError::InternalState)?;
        match std::mem::replace(&mut *state, DirectCargoEnrollmentPreviewState::Consumed) {
            DirectCargoEnrollmentPreviewState::Available(preview) => Ok(*preview),
            prior @ (DirectCargoEnrollmentPreviewState::Consumed
            | DirectCargoEnrollmentPreviewState::Released) => {
                *state = prior;
                Err(DirectCargoEnrollmentError::PreviewUnavailable)
            }
        }
    }

    fn release_inner(
        &self,
    ) -> Result<DirectCargoEnrollmentPreviewReleaseOutcome, DirectCargoEnrollmentError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DirectCargoEnrollmentError::InternalState)?;
        match std::mem::replace(&mut *state, DirectCargoEnrollmentPreviewState::Released) {
            DirectCargoEnrollmentPreviewState::Available(_) => {
                Ok(DirectCargoEnrollmentPreviewReleaseOutcome::Released)
            }
            prior @ (DirectCargoEnrollmentPreviewState::Consumed
            | DirectCargoEnrollmentPreviewState::Released) => {
                *state = prior;
                Ok(DirectCargoEnrollmentPreviewReleaseOutcome::AlreadyUnavailable)
            }
        }
    }
}

#[derive(uniffi::Object)]
pub struct SnapshotReviewSession {
    inner: Mutex<CoreReviewSession>,
    engine: EngineHandle,
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

    /// Return one bounded page of historical candidate paths while this exact
    /// snapshot review remains retained. These observations are for display
    /// only and do not carry planning or cleanup authority.
    pub fn candidate_summaries(
        &self,
        cursor: u16,
        limit: u16,
    ) -> Result<CandidateSummaryPage, EngineError> {
        if !(1..=CANDIDATE_DETAIL_PAGE_LIMIT).contains(&limit) {
            return Err(EngineError::InvalidCandidateDetailRequest);
        }
        self.with_open_session(|session| {
            let evaluation = self
                .engine
                .candidate_history_for_scan(session.scan_id())
                .map_err(map_candidate_history_error)?;
            if !matches!(
                evaluation.status(),
                CoreDurableCandidateEvaluationStatus::Succeeded { .. }
            ) {
                return Err(EngineError::CandidateEvaluationNotSucceeded);
            }
            let total_candidates = u16::try_from(evaluation.candidates().len())
                .map_err(|_| EngineError::InternalState)?;
            if cursor > total_candidates {
                return Err(EngineError::CandidateCursorOutOfRange);
            }
            let start = usize::from(cursor);
            let end = start
                .checked_add(usize::from(limit))
                .ok_or(EngineError::InternalState)?
                .min(usize::from(total_candidates));
            let next_cursor = (end < usize::from(total_candidates))
                .then(|| u16::try_from(end).map_err(|_| EngineError::InternalState))
                .transpose()?;
            let candidates = evaluation.candidates()[start..end]
                .iter()
                .map(project_candidate_summary)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CandidateSummaryPage {
                record_version: FFI_RECORD_VERSION,
                scan_id: session.scan_id().as_str().to_owned(),
                cursor,
                next_cursor,
                total_candidates,
                candidates,
            })
        })
    }

    /// Persist one semantic review-intent transition for a candidate belonging
    /// to this retained snapshot review. The result is status-only and cannot
    /// become a plan, approval, journal claim, or filesystem effect.
    pub fn review_candidate(
        &self,
        candidate_id: String,
        command: CandidateReviewCommand,
    ) -> Result<CandidateReviewResult, EngineError> {
        let candidate_id = CandidateId::new(candidate_id)
            .map_err(|_| EngineError::InvalidCandidateDetailRequest)?;
        self.with_open_session(|session| {
            let result = self
                .engine
                .review_candidate(
                    session.scan_id(),
                    &candidate_id,
                    map_candidate_review_command(command),
                )
                .map_err(map_candidate_review_error)?;
            Ok(CandidateReviewResult {
                record_version: FFI_RECORD_VERSION,
                scan_id: result.scan_id().as_str().to_owned(),
                candidate_id: result.candidate_id().as_str().to_owned(),
                status: map_candidate_status(result.status()),
            })
        })
    }

    /// Return one bounded page of historical candidate paths while this exact
    /// snapshot review remains retained. These observations are for display
    /// only and do not carry planning or cleanup authority.
    pub fn candidate_paths(
        &self,
        candidate_id: String,
        cursor: u16,
        limit: u16,
    ) -> Result<CandidatePathPage, EngineError> {
        if !(1..=CANDIDATE_DETAIL_PAGE_LIMIT).contains(&limit) {
            return Err(EngineError::InvalidCandidateDetailRequest);
        }
        let candidate_id = CandidateId::new(candidate_id)
            .map_err(|_| EngineError::InvalidCandidateDetailRequest)?;
        self.with_open_session(|session| {
            self.engine
                .candidate_path_page(session.scan_id(), &candidate_id, cursor, limit)
                .map_err(map_candidate_detail_error)
                .and_then(project_candidate_path_page)
        })
    }

    /// Return one bounded page of typed historical candidate evidence while
    /// this exact snapshot review remains retained.
    pub fn candidate_evidence(
        &self,
        candidate_id: String,
        cursor: u16,
        limit: u16,
    ) -> Result<CandidateEvidencePage, EngineError> {
        if !(1..=CANDIDATE_DETAIL_PAGE_LIMIT).contains(&limit) {
            return Err(EngineError::InvalidCandidateDetailRequest);
        }
        let candidate_id = CandidateId::new(candidate_id)
            .map_err(|_| EngineError::InvalidCandidateDetailRequest)?;
        self.with_open_session(|session| {
            self.engine
                .candidate_evidence_page(session.scan_id(), &candidate_id, cursor, limit)
                .map_err(map_candidate_detail_error)
                .and_then(project_candidate_evidence_page)
        })
    }
}

impl SnapshotReviewSession {
    fn ensure_open_for_plan_review(&self) -> Result<(), RustTargetPlanReviewError> {
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(RustTargetPlanReviewError::Closed);
        }
        let session = self
            .inner
            .lock()
            .map_err(|_| RustTargetPlanReviewError::InternalState)?;
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(RustTargetPlanReviewError::Closed);
        }
        session
            .validate_for_plan_review()
            .map(|_| ())
            .map_err(map_parent_plan_review_error)
    }

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
        // Candidate detail is keyed by the review lease as well as the scan
        // identity. Validate the lease before every projection so expiry can
        // never silently turn historical disclosure into an unbounded store
        // query.
        session.validate_current().map_err(map_review_error)?;
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
    next_event_sequence: u64,
    oldest_available_event_sequence: u64,
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
            next_event_sequence: 0,
            oldest_available_event_sequence: 0,
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
    fn apply(&mut self, batch: CoreTaskEventBatch) -> Vec<ScanEvent> {
        self.events_truncated |= batch.truncated;
        self.next_event_sequence = batch.next_sequence;
        self.oldest_available_event_sequence = batch.oldest_available_sequence;
        let events = batch
            .events
            .iter()
            .map(|event| ScanEvent {
                record_version: FFI_RECORD_VERSION,
                sequence: event.sequence,
                kind: map_scan_event_kind(&event.kind),
            })
            .collect();
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
        events
    }
}

fn map_scan_event_kind(kind: &CoreTaskEventKind) -> ScanEventKind {
    match kind {
        CoreTaskEventKind::Queued => ScanEventKind::Queued,
        CoreTaskEventKind::Started => ScanEventKind::Started,
        CoreTaskEventKind::Progress { completed, total } => ScanEventKind::Progress {
            completed: *completed,
            total: *total,
        },
        CoreTaskEventKind::ScanProgress {
            files,
            directories,
            known_allocated_bytes,
            errors,
        } => ScanEventKind::ScanProgress {
            files_scanned: *files,
            directories_scanned: *directories,
            known_allocated_bytes: *known_allocated_bytes,
            error_count: *errors,
        },
        CoreTaskEventKind::ScanFinalizing => ScanEventKind::ScanFinalizing,
        CoreTaskEventKind::CandidateEvaluationStarted => ScanEventKind::CandidateEvaluationStarted,
        CoreTaskEventKind::CandidateEvaluationFinished { status } => {
            let (status, candidate_count, failure) = match *status {
                CoreCandidateEvaluationStatus::NotRun => {
                    (ScanCandidateEvaluationStatus::NotRun, 0, None)
                }
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
            ScanEventKind::CandidateEvaluationFinished {
                status,
                candidate_count,
                failure,
            }
        }
        CoreTaskEventKind::CancellationRequested => ScanEventKind::CancellationRequested,
        CoreTaskEventKind::Terminal { phase } => ScanEventKind::Terminal {
            phase: map_phase(*phase),
        },
        CoreTaskEventKind::ScanRecoveryMaintenanceBatchApplying
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
        | CoreTaskEventKind::SnapshotUnleasedTempMaintenanceBatchFinished { .. } => {
            ScanEventKind::Maintenance
        }
        _ => ScanEventKind::Maintenance,
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
        let events = progress.apply(events);

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
            events,
            next_event_sequence: progress.next_event_sequence,
            oldest_available_event_sequence: progress.oldest_available_event_sequence,
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
    state: Arc<Mutex<EngineState>>,
    close_completed: Arc<Condvar>,
    reviews: Arc<Mutex<Vec<Weak<SnapshotReviewSession>>>>,
    direct_cargo_previews: Arc<Mutex<Vec<Weak<DirectCargoEnrollmentPreviewSession>>>>,
    rust_target_plan_reviews: Arc<Mutex<Vec<Weak<RustTargetPlanReviewSession>>>>,
    rust_target_plan_preparations: Arc<RustTargetPlanPreparationTracker>,
    background_close_started: Arc<AtomicBool>,
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
            state: Arc::new(Mutex::new(EngineState::Open(engine))),
            close_completed: Arc::new(Condvar::new()),
            reviews: Arc::new(Mutex::new(Vec::new())),
            direct_cargo_previews: Arc::new(Mutex::new(Vec::new())),
            rust_target_plan_reviews: Arc::new(Mutex::new(Vec::new())),
            rust_target_plan_preparations: Arc::new(RustTargetPlanPreparationTracker::default()),
            background_close_started: Arc::new(AtomicBool::new(false)),
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

    /// Return bounded path-free capacity changes and chart points for one
    /// stable volume. Missing historical baselines remain optional; no trend
    /// value grants cleanup or scheduling authority.
    pub fn get_capacity_trend(
        &self,
        request: CapacityTrendRequest,
    ) -> Result<CapacityTrendStatus, EngineError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(EngineError::InvalidCapacityObservation);
        }
        let stable_volume_id = parse_macos_volume_id(Some(request.stable_volume_id))?
            .ok_or(EngineError::InvalidCapacityObservation)?;
        let anchor_at = unix_ms_to_system_time(request.anchor_at_unix_ms)?;
        self.with_engine(|engine| {
            let trend = engine
                .capacity_trend(&stable_volume_id, anchor_at)
                .map_err(map_volume_status_error)?;
            capacity_trend_status(trend)
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

    /// Load the path-free global permanent-cleanup kill switch. This setting
    /// can only deny effects; it cannot create a plan or authorize a target.
    pub fn get_permanent_cleanup_policy(
        &self,
    ) -> Result<PermanentCleanupPolicyStatus, PermanentCleanupPolicyError> {
        self.with_permanent_cleanup_engine(|engine| {
            engine
                .permanent_cleanup_policy()
                .map_err(map_permanent_cleanup_policy_error)
                .and_then(permanent_cleanup_policy_status)
        })
    }

    pub fn set_permanent_cleanup_enabled(
        &self,
        enabled: bool,
    ) -> Result<PermanentCleanupPolicyUpdate, PermanentCleanupPolicyError> {
        self.with_permanent_cleanup_engine(|engine| {
            engine
                .set_permanent_cleanup_enabled(enabled)
                .map_err(map_permanent_cleanup_policy_error)
                .and_then(permanent_cleanup_policy_update)
        })
    }

    pub fn reset_permanent_cleanup(
        &self,
    ) -> Result<PermanentCleanupPolicyUpdate, PermanentCleanupPolicyError> {
        self.with_permanent_cleanup_engine(|engine| {
            engine
                .reset_permanent_cleanup()
                .map_err(map_permanent_cleanup_policy_error)
                .and_then(permanent_cleanup_policy_update)
        })
    }

    /// Load the bounded, losslessly encoded deny-only user exclusion set.
    /// Returned paths are observations for settings presentation and never
    /// become planner or executor authority.
    pub fn get_cleanup_exclusions(
        &self,
    ) -> Result<CleanupExclusionsStatus, CleanupExclusionsError> {
        self.with_cleanup_exclusions_engine(|engine| {
            engine
                .cleanup_exclusions()
                .map_err(map_cleanup_exclusions_error)
                .and_then(cleanup_exclusions_status)
        })
    }

    /// Replace the bounded deny-only user exclusion set. Rust remains the
    /// semantic validator and takes the store-wide cleanup exclusion before
    /// persisting the exact lexical prefixes.
    pub fn set_cleanup_exclusions(
        &self,
        input: CleanupExclusionsInput,
    ) -> Result<CleanupExclusionsUpdate, CleanupExclusionsError> {
        let paths = decode_cleanup_exclusion_input(input)?;
        self.with_cleanup_exclusions_engine(|engine| {
            engine
                .set_cleanup_exclusions(paths)
                .map_err(map_cleanup_exclusions_error)
                .and_then(cleanup_exclusions_update)
        })
    }

    /// Restore the empty, versioned default exclusion set.
    pub fn reset_cleanup_exclusions(
        &self,
    ) -> Result<CleanupExclusionsUpdate, CleanupExclusionsError> {
        self.with_cleanup_exclusions_engine(|engine| {
            engine
                .reset_cleanup_exclusions()
                .map_err(map_cleanup_exclusions_error)
                .and_then(cleanup_exclusions_update)
        })
    }

    /// Statically inspect one exact Cargo file and return an engine-bound,
    /// consume-once preview. Inspection does not run the selected bytes or
    /// change durable enrollment.
    pub fn inspect_direct_cargo_enrollment(
        &self,
        request: DirectCargoEnrollmentInspectionRequest,
    ) -> Result<Arc<DirectCargoEnrollmentPreviewSession>, DirectCargoEnrollmentError> {
        let executable = decode_direct_cargo_executable_path(request)?;
        let state = self
            .state
            .lock()
            .map_err(|_| DirectCargoEnrollmentError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(DirectCargoEnrollmentError::Closed);
        };
        if self.closed.load(Ordering::Acquire) {
            return Err(DirectCargoEnrollmentError::Closed);
        }
        self.ensure_direct_cargo_preview_capacity()?;
        let preview = engine
            .inspect_direct_cargo_enrollment(&executable)
            .map_err(map_direct_cargo_enrollment_error)?;
        let info = direct_cargo_preview_info(&preview)?;
        self.register_direct_cargo_preview(preview, info)
    }

    /// Consume one preview from this exact engine and persist its identity.
    /// Consumption happens before core validation, so a stale or failed
    /// preview cannot be retried through the same foreign object.
    pub fn commit_direct_cargo_enrollment(
        &self,
        preview: Arc<DirectCargoEnrollmentPreviewSession>,
    ) -> Result<DirectCargoEnrollmentUpdate, DirectCargoEnrollmentError> {
        let state = self
            .state
            .lock()
            .map_err(|_| DirectCargoEnrollmentError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(DirectCargoEnrollmentError::Closed);
        };
        if self.closed.load(Ordering::Acquire) {
            return Err(DirectCargoEnrollmentError::Closed);
        }
        if !Arc::ptr_eq(&preview.engine_closed, &self.closed) {
            return Err(DirectCargoEnrollmentError::WrongEngine);
        }
        let core_preview = preview.take_for_commit()?;
        direct_cargo_mutation_result(engine.commit_direct_cargo_enrollment(core_preview))
    }

    /// Return the revisioned direct-Cargo discovery enrollment. This endpoint
    /// exposes observation DTOs only and cannot create cleanup authority.
    pub fn direct_cargo_enrollment_status(
        &self,
    ) -> Result<DirectCargoEnrollmentStatus, DirectCargoEnrollmentError> {
        self.with_direct_cargo_engine(|engine| {
            engine
                .direct_cargo_enrollment_status()
                .map_err(map_direct_cargo_enrollment_error)
                .and_then(direct_cargo_enrollment_status)
        })
    }

    /// Revoke any active discovery enrollment and retain core's revisioned
    /// tombstone. Previously issued previews become stale in core.
    pub fn revoke_direct_cargo_enrollment(
        &self,
    ) -> Result<DirectCargoEnrollmentUpdate, DirectCargoEnrollmentError> {
        self.with_direct_cargo_engine(|engine| {
            direct_cargo_mutation_result(engine.revoke_direct_cargo_enrollment())
        })
    }

    /// Prepare one exact Rust-target plan for presentation through an active
    /// snapshot review. The request supplies only a candidate ID; Rust derives
    /// the scan, path, mode, policy, plan identity, and time.
    pub fn prepare_rust_target_plan_review(
        &self,
        parent_review: Arc<SnapshotReviewSession>,
        request: RustTargetPlanReviewRequest,
    ) -> Result<Arc<RustTargetPlanReviewSession>, RustTargetPlanReviewError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(RustTargetPlanReviewError::InvalidRecordVersion);
        }
        let candidate_id = CandidateId::new(request.candidate_id)
            .map_err(|_| RustTargetPlanReviewError::CandidateUnavailable)?;
        if !Arc::ptr_eq(&parent_review.engine_closed, &self.closed) {
            return Err(RustTargetPlanReviewError::WrongEngine);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| RustTargetPlanReviewError::InternalState)?;
        let engine = match &*state {
            EngineState::Open(engine) => engine.clone(),
            EngineState::Closing | EngineState::Closed { .. } => {
                return Err(RustTargetPlanReviewError::Closed);
            }
        };
        drop(state);
        if self.closed.load(Ordering::Acquire) {
            return Err(RustTargetPlanReviewError::Closed);
        }
        let _preparation = self
            .rust_target_plan_preparations
            .enter_preparation(&self.closed)?;
        self.ensure_rust_target_plan_review_capacity()?;
        let admission = {
            let parent = parent_review
                .inner
                .lock()
                .map_err(|_| RustTargetPlanReviewError::InternalState)?;
            if self.closed.load(Ordering::Acquire) {
                return Err(RustTargetPlanReviewError::Closed);
            }
            engine
                .begin_rust_target_plan_review(&parent, &candidate_id)
                .map_err(map_rust_target_plan_review_error)?
        };
        let pending = engine
            .prepare_admitted_rust_target_plan_review(admission)
            .map_err(map_rust_target_plan_review_error)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(RustTargetPlanReviewError::Closed);
        }
        let validated = {
            let parent = parent_review
                .inner
                .lock()
                .map_err(|_| RustTargetPlanReviewError::InternalState)?;
            if self.closed.load(Ordering::Acquire) {
                return Err(RustTargetPlanReviewError::Closed);
            }
            engine
                .validate_pending_rust_target_plan_review(&parent, pending)
                .map_err(map_rust_target_plan_review_error)?
        };
        let review = engine
            .materialize_rust_target_plan_review(validated)
            .map_err(map_rust_target_plan_review_error)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(RustTargetPlanReviewError::Closed);
        }
        self.register_rust_target_plan_review(review, &parent_review)
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
    ///
    /// Cleanup history is exposed separately from scan history so the app can
    /// present prior outcomes without gaining a plan, approval, path, or
    /// executor capability.
    pub fn recent_cleanup_history(
        &self,
        cursor: Option<CleanupHistoryCursor>,
        limit: u16,
    ) -> Result<CleanupHistoryPage, CleanupHistoryError> {
        if !(1..=64).contains(&limit) {
            return Err(CleanupHistoryError::InvalidLimit);
        }
        let cursor = cursor.map(core_cleanup_history_cursor).transpose()?;
        self.with_cleanup_history_engine(|engine| {
            let page = engine
                .recent_cleanup_history(cursor.as_ref(), limit)
                .map_err(map_cleanup_history_error)?;
            cleanup_history_page(page)
        })
    }

    /// Return one exact, bounded, path-free cleanup-session observation. The
    /// supplied ID must come from summary history and is used only to select
    /// immutable history; it cannot resume, retry, approve, or execute work.
    pub fn cleanup_session_history(
        &self,
        request: CleanupSessionHistoryRequest,
    ) -> Result<CleanupSessionHistory, CleanupHistoryError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(CleanupHistoryError::InvalidRecordVersion);
        }
        let requested_session_id = request.session_id;
        let session_id = CoreCleanupSessionId::from_stable_str(requested_session_id.clone())
            .ok_or(CleanupHistoryError::InvalidSessionId)?;
        self.with_cleanup_history_engine(|engine| {
            let observation = engine
                .cleanup_session_history(&session_id)
                .map_err(map_cleanup_history_error)?;
            let projected = cleanup_session_history(observation)?;
            if projected.summary.session_id != requested_session_id {
                return Err(CleanupHistoryError::CorruptData);
            }
            Ok(projected)
        })
    }

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
        if self.closed.load(Ordering::Acquire) {
            return Err(EngineError::Closed);
        }
        let session = engine
            .acquire_explorer_snapshot_review(&scan_id)
            .map_err(map_review_error)?;
        self.register_snapshot_review(engine, session)
    }

    pub fn acquire_latest_explorer_snapshot_review(
        &self,
    ) -> Result<Arc<SnapshotReviewSession>, EngineError> {
        let state = self.state.lock().map_err(|_| EngineError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(EngineError::Closed);
        };
        if self.closed.load(Ordering::Acquire) {
            return Err(EngineError::Closed);
        }
        let session = engine
            .acquire_latest_explorer_snapshot_review()
            .map_err(map_review_error)?;
        self.register_snapshot_review(engine, session)
    }

    /// Execute one explicit Explorer Trash selection. Rust resolves and
    /// revalidates the retained node, creates the bounded journal row, and
    /// fences the one-shot callback. Swift cannot supply a path or retry a
    /// request; the callback is invoked synchronously while the claim is held.
    pub fn execute_explorer_trash(
        &self,
        review: Arc<SnapshotReviewSession>,
        node_id: u64,
        driver: Box<dyn TrashPlatformDriver>,
    ) -> Result<TrashPlatformResult, TrashExecutionError> {
        if !Arc::ptr_eq(&review.engine_closed, &self.closed) {
            return Err(TrashExecutionError::InvalidRequest);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| TrashExecutionError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(TrashExecutionError::Closed);
        };
        if self.closed.load(Ordering::Acquire) {
            return Err(TrashExecutionError::Closed);
        }
        let mut core_review = review
            .inner
            .lock()
            .map_err(|_| TrashExecutionError::InternalState)?;
        let result = engine
            .execute_explorer_trash_selection(&mut core_review, node_id, move |request| {
                let Ok((target_kind, absolute_path_bytes)) = request.into_parts() else {
                    return CoreTrashPlatformResult::Failed;
                };
                let ffi_request = TrashEffectRequest::from_core(target_kind, absolute_path_bytes);
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    driver.trash(ffi_request)
                }))
                .unwrap_or(TrashPlatformResult::OutcomeUnknown)
                {
                    TrashPlatformResult::Completed => CoreTrashPlatformResult::Completed,
                    TrashPlatformResult::Unsupported => CoreTrashPlatformResult::Unsupported,
                    TrashPlatformResult::Failed => CoreTrashPlatformResult::Failed,
                    TrashPlatformResult::OutcomeUnknown => CoreTrashPlatformResult::OutcomeUnknown,
                }
            })
            .map_err(map_trash_selection_error)?;
        Ok(match result {
            CoreTrashPlatformResult::Completed => TrashPlatformResult::Completed,
            CoreTrashPlatformResult::Unsupported => TrashPlatformResult::Unsupported,
            CoreTrashPlatformResult::Failed => TrashPlatformResult::Failed,
            CoreTrashPlatformResult::OutcomeUnknown => TrashPlatformResult::OutcomeUnknown,
        })
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
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                self.schedule_background_close();
                return false;
            }
            match self.state.try_lock() {
                Ok(mut state) => match &*state {
                    EngineState::Open(_) => {
                        let EngineState::Open(engine) =
                            std::mem::replace(&mut *state, EngineState::Closing)
                        else {
                            unreachable!("open state was just matched")
                        };
                        drop(state);
                        self.release_registered_rust_target_plan_reviews();
                        self.release_registered_reviews();
                        self.release_registered_direct_cargo_previews();
                        return finish_ffi_engine_close(
                            &self.state,
                            &self.close_completed,
                            &self.rust_target_plan_preparations,
                            engine,
                            deadline,
                        );
                    }
                    EngineState::Closing => {
                        let (state, timed_out) = self
                            .close_completed
                            .wait_timeout(state, remaining)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if let EngineState::Closed { quiesced } = *state {
                            return quiesced;
                        }
                        if timed_out.timed_out() {
                            return false;
                        }
                    }
                    EngineState::Closed { quiesced } => return *quiesced,
                },
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                    let mut state = poisoned.into_inner();
                    match &*state {
                        EngineState::Open(_) => {
                            let EngineState::Open(engine) =
                                std::mem::replace(&mut *state, EngineState::Closing)
                            else {
                                unreachable!("open state was just matched")
                            };
                            drop(state);
                            self.release_registered_rust_target_plan_reviews();
                            self.release_registered_reviews();
                            self.release_registered_direct_cargo_previews();
                            return finish_ffi_engine_close(
                                &self.state,
                                &self.close_completed,
                                &self.rust_target_plan_preparations,
                                engine,
                                deadline,
                            );
                        }
                        EngineState::Closing => return false,
                        EngineState::Closed { quiesced } => return *quiesced,
                    }
                }
            }
        }
    }
}

impl DuxEngine {
    fn schedule_background_close(&self) {
        if self.background_close_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let state = Arc::clone(&self.state);
        let close_completed = Arc::clone(&self.close_completed);
        let operations = Arc::clone(&self.rust_target_plan_preparations);
        let reviews = Arc::clone(&self.reviews);
        let plan_reviews = Arc::clone(&self.rust_target_plan_reviews);
        let cargo_previews = Arc::clone(&self.direct_cargo_previews);
        std::thread::spawn(move || {
            let engine = loop {
                let mut state_guard = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match &*state_guard {
                    EngineState::Open(_) => {
                        let EngineState::Open(engine) =
                            std::mem::replace(&mut *state_guard, EngineState::Closing)
                        else {
                            unreachable!("open state was just matched")
                        };
                        break Some(engine);
                    }
                    EngineState::Closing => {
                        let state_guard = close_completed
                            .wait(state_guard)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if matches!(*state_guard, EngineState::Closed { .. }) {
                            break None;
                        }
                    }
                    EngineState::Closed { .. } => break None,
                }
            };
            if let Some(engine) = engine {
                release_plan_review_registry(&plan_reviews);
                release_snapshot_review_registry(&reviews, &operations);
                release_direct_cargo_preview_registry(&cargo_previews, &operations);
                let _ = finish_ffi_engine_close(
                    &state,
                    &close_completed,
                    &operations,
                    engine,
                    Instant::now() + CLOSE_TIMEOUT,
                );
            }
        });
    }

    fn ensure_rust_target_plan_review_capacity(&self) -> Result<(), RustTargetPlanReviewError> {
        let reviews = {
            let mut reviews = self
                .rust_target_plan_reviews
                .lock()
                .map_err(|_| RustTargetPlanReviewError::InternalState)?;
            std::mem::take(&mut *reviews)
        };
        let live_reviews = reviews
            .into_iter()
            .filter_map(|review| review.upgrade())
            .collect::<Vec<_>>();
        let retained = live_reviews.iter().map(Arc::downgrade).collect::<Vec<_>>();
        let mut available = false;
        let mut failure = None;
        for review in &live_reviews {
            match review.occupies_capacity() {
                Ok(true) => available = true,
                Ok(false) => {}
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let mut reviews = self
            .rust_target_plan_reviews
            .lock()
            .map_err(|_| RustTargetPlanReviewError::InternalState)?;
        if self.closed.load(Ordering::Acquire) {
            drop(reviews);
            for review in retained.into_iter().filter_map(|review| review.upgrade()) {
                let _ = review.release_inner();
            }
            return Err(RustTargetPlanReviewError::Closed);
        }
        reviews.extend(retained);
        if let Some(error) = failure {
            return Err(error);
        }
        if available {
            Err(RustTargetPlanReviewError::ReviewBusy)
        } else {
            Ok(())
        }
    }

    fn register_rust_target_plan_review(
        &self,
        review: CoreRustTargetPlanReview,
        parent_review: &Arc<SnapshotReviewSession>,
    ) -> Result<Arc<RustTargetPlanReviewSession>, RustTargetPlanReviewError> {
        parent_review.ensure_open_for_plan_review()?;
        let _ = review
            .info()
            .map_err(map_rust_target_plan_review_error)
            .and_then(project_rust_target_plan_review_info)?;
        parent_review.ensure_open_for_plan_review()?;
        let review = Arc::new(RustTargetPlanReviewSession {
            state: Mutex::new(RustTargetPlanReviewState::Available(Box::new(review))),
            parent_review: Arc::downgrade(parent_review),
            operations: Arc::clone(&self.rust_target_plan_preparations),
            engine_closed: Arc::clone(&self.closed),
        });
        if self.closed.load(Ordering::Acquire) {
            let _ = review.release_inner();
            return Err(RustTargetPlanReviewError::Closed);
        }
        let mut reviews = match self.rust_target_plan_reviews.lock() {
            Ok(reviews) => reviews,
            Err(_) => {
                let _ = review.release_inner();
                return Err(RustTargetPlanReviewError::InternalState);
            }
        };
        let mut retained = Vec::with_capacity(reviews.len().saturating_add(1));
        let mut existing_busy = false;
        for retained_review in reviews.iter().filter_map(Weak::upgrade) {
            if retained_review.occupies_capacity_state_only()? {
                existing_busy = true;
                break;
            }
            retained.push(Arc::downgrade(&retained_review));
        }
        if existing_busy {
            drop(reviews);
            let _ = review.release_inner();
            return Err(RustTargetPlanReviewError::ReviewBusy);
        }
        if self.closed.load(Ordering::Acquire) {
            drop(reviews);
            let _ = review.release_inner();
            return Err(RustTargetPlanReviewError::Closed);
        }
        *reviews = retained;
        reviews.push(Arc::downgrade(&review));
        Ok(review)
    }

    fn ensure_direct_cargo_preview_capacity(&self) -> Result<(), DirectCargoEnrollmentError> {
        let mut previews = self
            .direct_cargo_previews
            .lock()
            .map_err(|_| DirectCargoEnrollmentError::InternalState)?;
        let mut retained = Vec::with_capacity(previews.len());
        let mut available = false;
        for preview in previews.iter().filter_map(Weak::upgrade) {
            if preview.is_available()? {
                available = true;
            }
            retained.push(Arc::downgrade(&preview));
        }
        *previews = retained;
        if available {
            Err(DirectCargoEnrollmentError::Busy)
        } else {
            Ok(())
        }
    }

    fn register_direct_cargo_preview(
        &self,
        preview: CoreDirectCargoEnrollmentPreview,
        info: DirectCargoEnrollmentPreviewInfo,
    ) -> Result<Arc<DirectCargoEnrollmentPreviewSession>, DirectCargoEnrollmentError> {
        let preview = Arc::new(DirectCargoEnrollmentPreviewSession {
            state: Mutex::new(DirectCargoEnrollmentPreviewState::Available(Box::new(
                preview,
            ))),
            info,
            engine_closed: Arc::clone(&self.closed),
        });
        if self.closed.load(Ordering::Acquire) {
            let _ = preview.release_inner();
            return Err(DirectCargoEnrollmentError::Closed);
        }
        let mut previews = match self.direct_cargo_previews.lock() {
            Ok(previews) => previews,
            Err(_) => {
                let _ = preview.release_inner();
                return Err(DirectCargoEnrollmentError::InternalState);
            }
        };
        let mut retained = Vec::with_capacity(previews.len().saturating_add(1));
        let mut existing_busy = false;
        for retained_preview in previews.iter().filter_map(Weak::upgrade) {
            if retained_preview.is_available()? {
                existing_busy = true;
                break;
            }
            retained.push(Arc::downgrade(&retained_preview));
        }
        if existing_busy {
            drop(previews);
            let _ = preview.release_inner();
            return Err(DirectCargoEnrollmentError::Busy);
        }
        if self.closed.load(Ordering::Acquire) {
            drop(previews);
            let _ = preview.release_inner();
            return Err(DirectCargoEnrollmentError::Closed);
        }
        *previews = retained;
        previews.push(Arc::downgrade(&preview));
        Ok(preview)
    }

    fn register_snapshot_review(
        &self,
        engine: &EngineHandle,
        session: CoreReviewSession,
    ) -> Result<Arc<SnapshotReviewSession>, EngineError> {
        let review = Arc::new(SnapshotReviewSession {
            inner: Mutex::new(session),
            engine: engine.clone(),
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
        if self.closed.load(Ordering::Acquire) {
            drop(reviews);
            let _ = review.release_inner();
            return Err(EngineError::Closed);
        }
        reviews.push(Arc::downgrade(&review));
        Ok(review)
    }
    fn with_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let state = self.state.lock().map_err(|_| EngineError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(EngineError::Closed)
            }
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
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(PressurePolicyError::Closed)
            }
        }
    }

    fn with_permanent_cleanup_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, PermanentCleanupPolicyError>,
    ) -> Result<T, PermanentCleanupPolicyError> {
        let state = self
            .state
            .lock()
            .map_err(|_| PermanentCleanupPolicyError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(PermanentCleanupPolicyError::Closed)
            }
        }
    }

    fn with_cleanup_exclusions_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, CleanupExclusionsError>,
    ) -> Result<T, CleanupExclusionsError> {
        let state = self
            .state
            .lock()
            .map_err(|_| CleanupExclusionsError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(CleanupExclusionsError::Closed)
            }
        }
    }

    fn with_cleanup_history_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, CleanupHistoryError>,
    ) -> Result<T, CleanupHistoryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| CleanupHistoryError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(CleanupHistoryError::Closed)
            }
        }
    }

    fn with_direct_cargo_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, DirectCargoEnrollmentError>,
    ) -> Result<T, DirectCargoEnrollmentError> {
        let state = self
            .state
            .lock()
            .map_err(|_| DirectCargoEnrollmentError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(DirectCargoEnrollmentError::Closed)
            }
        }
    }

    fn with_scan_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, ScanError>,
    ) -> Result<T, ScanError> {
        let state = self.state.lock().map_err(|_| ScanError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(ScanError::Closed)
            }
        }
    }

    fn release_registered_reviews(&self) {
        release_snapshot_review_registry(&self.reviews, &self.rust_target_plan_preparations);
    }

    fn release_registered_rust_target_plan_reviews(&self) {
        release_plan_review_registry(&self.rust_target_plan_reviews);
    }

    fn release_registered_direct_cargo_previews(&self) {
        release_direct_cargo_preview_registry(
            &self.direct_cargo_previews,
            &self.rust_target_plan_preparations,
        );
    }
}

fn release_snapshot_review_registry(
    registry: &Mutex<Vec<Weak<SnapshotReviewSession>>>,
    operations: &Arc<RustTargetPlanPreparationTracker>,
) {
    let reviews = match registry.lock() {
        Ok(mut reviews) => std::mem::take(&mut *reviews),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    }
    .into_iter()
    .filter_map(|review| review.upgrade())
    .collect::<Vec<_>>();
    if reviews.is_empty() {
        return;
    }
    let Ok(operation) = operations.enter_cleanup() else {
        return;
    };
    std::thread::spawn(move || {
        for review in reviews {
            let _ = review.release_inner();
        }
        drop(operation);
    });
}

fn release_plan_review_registry(registry: &Mutex<Vec<Weak<RustTargetPlanReviewSession>>>) {
    let reviews = match registry.lock() {
        Ok(mut reviews) => std::mem::take(&mut *reviews),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    };
    for review in reviews.into_iter().filter_map(|review| review.upgrade()) {
        review.release_for_close();
    }
}

fn release_direct_cargo_preview_registry(
    registry: &Mutex<Vec<Weak<DirectCargoEnrollmentPreviewSession>>>,
    operations: &Arc<RustTargetPlanPreparationTracker>,
) {
    let previews = match registry.lock() {
        Ok(mut previews) => std::mem::take(&mut *previews),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    }
    .into_iter()
    .filter_map(|preview| preview.upgrade())
    .collect::<Vec<_>>();
    if previews.is_empty() {
        return;
    }
    let Ok(operation) = operations.enter_cleanup() else {
        return;
    };
    std::thread::spawn(move || {
        for preview in previews {
            let _ = preview.release_inner();
        }
        drop(operation);
    });
}

fn finish_ffi_engine_close(
    state: &Mutex<EngineState>,
    close_completed: &Condvar,
    operations: &RustTargetPlanPreparationTracker,
    engine: EngineHandle,
    deadline: Instant,
) -> bool {
    engine.close();
    let operations_drained = operations.wait_until(deadline);
    let worker_budget = deadline
        .checked_duration_since(Instant::now())
        .unwrap_or(Duration::ZERO);
    let workers_quiesced = engine.wait_until_closed(worker_budget);
    let quiesced = operations_drained && workers_quiesced;
    let mut state = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *state = EngineState::Closed { quiesced };
    close_completed.notify_all();
    quiesced
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

fn map_trash_selection_error(error: CoreTrashSelectionError) -> TrashExecutionError {
    match error {
        CoreTrashSelectionError::Review => TrashExecutionError::ReviewUnavailable,
        CoreTrashSelectionError::InvalidRequest => TrashExecutionError::InvalidRequest,
        CoreTrashSelectionError::ChangedSincePlan => TrashExecutionError::ChangedSincePlan,
        CoreTrashSelectionError::Busy => TrashExecutionError::Busy,
        CoreTrashSelectionError::Unavailable => TrashExecutionError::StorageUnavailable,
        CoreTrashSelectionError::UnsafeStorage => TrashExecutionError::UnsafeStorage,
        CoreTrashSelectionError::IncompatibleSchema => TrashExecutionError::IncompatibleSchema,
        CoreTrashSelectionError::CorruptData => TrashExecutionError::CorruptData,
        CoreTrashSelectionError::OutcomeUnknown => TrashExecutionError::OutcomeUnknown,
        CoreTrashSelectionError::InternalState => TrashExecutionError::InternalState,
    }
}

fn map_candidate_detail_error(error: CoreCandidateDetailError) -> EngineError {
    match error {
        CoreCandidateDetailError::Closed => EngineError::Closed,
        CoreCandidateDetailError::InvalidLimit { .. }
        | CoreCandidateDetailError::CursorOutOfRange => EngineError::InvalidCandidateDetailRequest,
        CoreCandidateDetailError::ScanNotFound => EngineError::ScanNotFound,
        CoreCandidateDetailError::EvaluationNotSucceeded => {
            EngineError::CandidateEvaluationNotSucceeded
        }
        CoreCandidateDetailError::CandidateNotFound => EngineError::CandidateNotFound,
        CoreCandidateDetailError::IncompatibleSchema => EngineError::IncompatibleSchema,
        CoreCandidateDetailError::Busy => EngineError::Busy,
        CoreCandidateDetailError::UnsafeStorage => EngineError::UnsafeStorage,
        CoreCandidateDetailError::QueryLimitExceeded => EngineError::BudgetExceeded,
        CoreCandidateDetailError::CorruptData => EngineError::CorruptData,
        CoreCandidateDetailError::Unavailable => EngineError::StorageUnavailable,
        CoreCandidateDetailError::InternalState => EngineError::InternalState,
        _ => EngineError::InternalState,
    }
}

const fn map_candidate_review_command(
    command: CandidateReviewCommand,
) -> CoreCandidateReviewCommand {
    match command {
        CandidateReviewCommand::Select => CoreCandidateReviewCommand::Select,
        CandidateReviewCommand::ClearSelection => CoreCandidateReviewCommand::ClearSelection,
        CandidateReviewCommand::Dismiss => CoreCandidateReviewCommand::Dismiss,
        CandidateReviewCommand::Restore => CoreCandidateReviewCommand::Restore,
    }
}

fn map_candidate_review_error(error: CoreCandidateReviewError) -> EngineError {
    match error {
        CoreCandidateReviewError::Closed => EngineError::Closed,
        CoreCandidateReviewError::CandidateNotFound => EngineError::CandidateNotFound,
        CoreCandidateReviewError::NotReviewable => EngineError::CandidateReviewNotReviewable,
        CoreCandidateReviewError::IncompatibleSchema => EngineError::IncompatibleSchema,
        CoreCandidateReviewError::Busy => EngineError::Busy,
        CoreCandidateReviewError::UnsafeStorage => EngineError::UnsafeStorage,
        CoreCandidateReviewError::QueryLimitExceeded => EngineError::BudgetExceeded,
        CoreCandidateReviewError::CorruptData => EngineError::CorruptData,
        CoreCandidateReviewError::OutcomeUnknown => EngineError::OutcomeUnknown,
        CoreCandidateReviewError::Unavailable => EngineError::StorageUnavailable,
        CoreCandidateReviewError::InternalState => EngineError::InternalState,
        _ => EngineError::InternalState,
    }
}

fn map_candidate_history_error(error: CoreCandidateHistoryError) -> EngineError {
    match error {
        CoreCandidateHistoryError::Closed => EngineError::Closed,
        CoreCandidateHistoryError::ScanNotFound => EngineError::ScanNotFound,
        CoreCandidateHistoryError::IncompatibleSchema => EngineError::IncompatibleSchema,
        CoreCandidateHistoryError::Busy => EngineError::Busy,
        CoreCandidateHistoryError::UnsafeStorage => EngineError::UnsafeStorage,
        CoreCandidateHistoryError::QueryLimitExceeded => EngineError::BudgetExceeded,
        CoreCandidateHistoryError::CorruptData => EngineError::CorruptData,
        CoreCandidateHistoryError::Unavailable => EngineError::StorageUnavailable,
        CoreCandidateHistoryError::InternalState => EngineError::InternalState,
        _ => EngineError::InternalState,
    }
}

const fn map_rust_target_plan_review_error(
    error: CoreRustTargetPlanReviewError,
) -> RustTargetPlanReviewError {
    match error {
        CoreRustTargetPlanReviewError::Closed => RustTargetPlanReviewError::Closed,
        CoreRustTargetPlanReviewError::WrongEngine => RustTargetPlanReviewError::WrongEngine,
        CoreRustTargetPlanReviewError::ParentReviewUnavailable => {
            RustTargetPlanReviewError::ParentReviewUnavailable
        }
        CoreRustTargetPlanReviewError::ReviewExpired => RustTargetPlanReviewError::ReviewExpired,
        CoreRustTargetPlanReviewError::CandidateUnavailable => {
            RustTargetPlanReviewError::CandidateUnavailable
        }
        CoreRustTargetPlanReviewError::CargoNotEnrolled => {
            RustTargetPlanReviewError::CargoNotEnrolled
        }
        CoreRustTargetPlanReviewError::ActiveProcesses => {
            RustTargetPlanReviewError::ActiveProcesses
        }
        CoreRustTargetPlanReviewError::ChangedDuringReview => {
            RustTargetPlanReviewError::ChangedDuringReview
        }
        CoreRustTargetPlanReviewError::UnsupportedPlatform => {
            RustTargetPlanReviewError::UnsupportedPlatform
        }
        CoreRustTargetPlanReviewError::BudgetExceeded => RustTargetPlanReviewError::BudgetExceeded,
        CoreRustTargetPlanReviewError::Busy => RustTargetPlanReviewError::Busy,
        CoreRustTargetPlanReviewError::UnsafeStorage => RustTargetPlanReviewError::UnsafeStorage,
        CoreRustTargetPlanReviewError::CorruptData => RustTargetPlanReviewError::CorruptData,
        CoreRustTargetPlanReviewError::Unavailable => RustTargetPlanReviewError::Unavailable,
        CoreRustTargetPlanReviewError::InternalState => RustTargetPlanReviewError::InternalState,
    }
}

const fn map_parent_plan_review_error(error: CoreReviewError) -> RustTargetPlanReviewError {
    match error {
        CoreReviewError::Closed => RustTargetPlanReviewError::Closed,
        CoreReviewError::ScanNotFound
        | CoreReviewError::SnapshotUnavailable
        | CoreReviewError::LeaseExpired => RustTargetPlanReviewError::ParentReviewUnavailable,
        CoreReviewError::Busy => RustTargetPlanReviewError::Busy,
        CoreReviewError::UnsafeStorage => RustTargetPlanReviewError::UnsafeStorage,
        CoreReviewError::BudgetExceeded => RustTargetPlanReviewError::BudgetExceeded,
        CoreReviewError::CorruptData
        | CoreReviewError::IncompatibleSchema
        | CoreReviewError::IncompatibleSnapshot => RustTargetPlanReviewError::CorruptData,
        CoreReviewError::ReadOnlyStore
        | CoreReviewError::Unavailable
        | CoreReviewError::OutcomeUnknown => RustTargetPlanReviewError::Unavailable,
        CoreReviewError::InternalState => RustTargetPlanReviewError::InternalState,
        _ => RustTargetPlanReviewError::InternalState,
    }
}

fn is_terminal_plan_review_result(
    result: &Result<RustTargetPlanReviewInfo, RustTargetPlanReviewError>,
) -> bool {
    matches!(
        result,
        Err(RustTargetPlanReviewError::ReviewExpired
            | RustTargetPlanReviewError::ParentReviewUnavailable
            | RustTargetPlanReviewError::ChangedDuringReview
            | RustTargetPlanReviewError::ReviewUnavailable)
    )
}

fn project_rust_target_plan_review_info(
    info: CoreRustTargetPlanReviewInfo,
) -> Result<RustTargetPlanReviewInfo, RustTargetPlanReviewError> {
    if info.item_count != 1
        || info.path_count != 1
        || info.rule_id != "developer.rust.target"
        || info.rule_revision != 2
        || info.category != CoreCandidateCategory::DeveloperArtifact
        || info.mode != CorePlanCleanupMode::PermanentSafe
        || info.safety != CoreSafetyTier::SafeRegenerable
        || info.action != CoreCandidateAction::RemoveKnownRegenerableContents
        || info.schedule_eligible
        || info.warnings
            != [
                CorePlanWarning::EstimatedBytesUnverified,
                CorePlanWarning::PermanentRemovalCannotBeUndone,
            ]
        || !is_bounded_plan_review_identifier(&info.plan_id)
        || !is_bounded_plan_review_identifier(info.source_scan_id.as_str())
        || !is_bounded_plan_review_identifier(info.candidate_id.as_str())
        || !is_bounded_plan_review_identifier(&info.rule_id)
    {
        return Err(RustTargetPlanReviewError::InternalState);
    }
    let created_at_unix_ms = rust_target_plan_review_time_ms(info.created_at)?;
    let effective_expires_at_unix_ms = rust_target_plan_review_time_ms(info.effective_expires_at)?;
    if created_at_unix_ms >= effective_expires_at_unix_ms {
        return Err(RustTargetPlanReviewError::InternalState);
    }
    let path = project_rust_target_plan_review_path(&info.path)?;
    Ok(RustTargetPlanReviewInfo {
        record_version: FFI_RECORD_VERSION,
        plan_id: info.plan_id,
        source_scan_id: info.source_scan_id.as_str().to_owned(),
        candidate_id: info.candidate_id.as_str().to_owned(),
        rule_id: info.rule_id,
        rule_revision: info.rule_revision,
        category: map_candidate_category(info.category),
        mode: CleanupMode::PermanentSafe,
        safety: map_candidate_safety(info.safety),
        action: map_candidate_action(info.action),
        estimated_bytes: info.estimated_bytes,
        warnings: vec![
            CleanupWarning::EstimatedBytesUnverified,
            CleanupWarning::PermanentRemovalCannotBeUndone,
        ],
        created_at_unix_ms,
        effective_expires_at_unix_ms,
        schedule_eligible: info.schedule_eligible,
        item_count: info.item_count,
        path_count: info.path_count,
        path,
    })
}

fn is_bounded_plan_review_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CANDIDATE_IDENTIFIER_BYTES
        && !value.chars().any(char::is_control)
}

fn rust_target_plan_review_time_ms(value: SystemTime) -> Result<i64, RustTargetPlanReviewError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RustTargetPlanReviewError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| RustTargetPlanReviewError::InternalState)
}

fn project_rust_target_plan_review_path(
    path: &Path,
) -> Result<RustTargetPlanReviewPath, RustTargetPlanReviewError> {
    if !path.is_absolute()
        || path.components().count() == 1
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(RustTargetPlanReviewError::InternalState);
    }
    #[cfg(unix)]
    let (encoding, encoded_bytes) = (
        SnapshotNameEncoding::UnixBytes,
        path.as_os_str().as_bytes().to_vec(),
    );
    #[cfg(windows)]
    let (encoding, encoded_bytes) = {
        let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(units.len().saturating_mul(2))
            .map_err(|_| RustTargetPlanReviewError::BudgetExceeded)?;
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        (SnapshotNameEncoding::WindowsUtf16LittleEndian, bytes)
    };
    if encoded_bytes.is_empty() || encoded_bytes.len() > MAX_CANDIDATE_ENCODED_PATH_BYTES {
        return Err(RustTargetPlanReviewError::BudgetExceeded);
    }
    #[cfg(unix)]
    if encoded_bytes.contains(&0) {
        return Err(RustTargetPlanReviewError::InternalState);
    }
    #[cfg(windows)]
    if encoded_bytes
        .chunks_exact(2)
        .any(|bytes| bytes == [0_u8, 0_u8])
    {
        return Err(RustTargetPlanReviewError::InternalState);
    }
    #[cfg(unix)]
    let display = match std::str::from_utf8(&encoded_bytes) {
        Ok(exact) if !exact.chars().any(is_unsafe_plan_review_display_scalar) => exact.to_owned(),
        _ => format!(
            "unix-bytes:{}",
            escaped_plan_review_path_bytes(&encoded_bytes)
        ),
    };
    #[cfg(windows)]
    let display = {
        let units = encoded_bytes
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();
        match String::from_utf16(&units) {
            Ok(exact) if !exact.chars().any(is_unsafe_plan_review_display_scalar) => exact,
            _ => format!(
                "windows-utf16le:{}",
                escaped_plan_review_path_bytes(&encoded_bytes)
            ),
        }
    };
    if display.len() > MAX_CANDIDATE_DISPLAY_PATH_BYTES {
        return Err(RustTargetPlanReviewError::BudgetExceeded);
    }
    Ok(RustTargetPlanReviewPath {
        encoding,
        encoded_bytes,
        display,
    })
}

fn escaped_plan_review_path_bytes(bytes: &[u8]) -> String {
    let mut escaped = String::with_capacity(bytes.len());
    for byte in bytes {
        match byte {
            0x20..=0x7e if *byte != b'\\' => escaped.push(char::from(*byte)),
            b'\\' => escaped.push_str("\\\\"),
            _ => {
                use std::fmt::Write as _;
                let _ = write!(&mut escaped, "\\x{byte:02x}");
            }
        }
    }
    escaped
}

/// Contract-wide plain-path display policy. C0/C1 controls and Unicode
/// 16.0 format/default-ignorable scalars are always byte-escaped so UI and
/// accessibility text cannot reorder, hide, or silently normalize a path.
fn is_unsafe_plan_review_display_scalar(value: char) -> bool {
    value.is_control()
        || matches!(
            value,
            '\u{00ad}'
                | '\u{034f}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061c}'
                | '\u{06dd}'
                | '\u{070f}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08e2}'
                | '\u{115f}'..='\u{1160}'
                | '\u{17b4}'..='\u{17b5}'
                | '\u{180b}'..='\u{180f}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{3164}'
                | '\u{fe00}'..='\u{fe0f}'
                | '\u{feff}'
                | '\u{ffa0}'
                | '\u{fff0}'..='\u{fffb}'
                | '\u{110bd}'
                | '\u{110cd}'
                | '\u{13430}'..='\u{1343f}'
                | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0000}'..='\u{e0fff}'
        )
}

fn project_candidate_path_page(
    page: CoreCandidatePathPage,
) -> Result<CandidatePathPage, EngineError> {
    ensure_candidate_detail_page_payload(
        page.paths()
            .iter()
            .map(|item| candidate_path_payload_bytes(item.path())),
    )?;
    Ok(CandidatePathPage {
        record_version: FFI_RECORD_VERSION,
        scan_id: page.scan_id().as_str().to_owned(),
        candidate: project_candidate_summary(page.candidate())?,
        cursor: page.cursor(),
        next_cursor: page.next_cursor(),
        total_paths: page.total_paths(),
        paths: page
            .paths()
            .iter()
            .map(|item| project_candidate_path(item.path()))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn project_candidate_evidence_page(
    page: CoreCandidateEvidencePage,
) -> Result<CandidateEvidencePage, EngineError> {
    ensure_candidate_detail_page_payload(
        page.evidence()
            .iter()
            .map(|item| candidate_evidence_payload_bytes(item.evidence())),
    )?;
    Ok(CandidateEvidencePage {
        record_version: FFI_RECORD_VERSION,
        scan_id: page.scan_id().as_str().to_owned(),
        candidate: project_candidate_summary(page.candidate())?,
        cursor: page.cursor(),
        next_cursor: page.next_cursor(),
        total_evidence: page.total_evidence(),
        evidence: page
            .evidence()
            .iter()
            .map(|item| project_candidate_evidence(item.ordinal(), item.evidence()))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn project_candidate_path(path: &CoreObservedPath) -> Result<CandidateObservedPath, EngineError> {
    let encoding = match path.encoding() {
        dux_core::engine::DurablePathEncoding::Utf8 => CandidatePathEncoding::Utf8,
        dux_core::engine::DurablePathEncoding::Utf16LittleEndian => {
            CandidatePathEncoding::Utf16LittleEndian
        }
        _ => return Err(EngineError::InternalState),
    };
    candidate_path_payload_bytes(path)?;
    Ok(CandidateObservedPath {
        encoding,
        encoded_bytes: path.encoded_bytes().to_vec(),
        display: path.display().to_owned(),
    })
}

fn candidate_path_payload_bytes(path: &CoreObservedPath) -> Result<usize, EngineError> {
    candidate_path_payload_from_lengths(path.encoded_bytes().len(), path.display().len())
}

fn candidate_path_payload_from_lengths(
    encoded_bytes: usize,
    display_bytes: usize,
) -> Result<usize, EngineError> {
    if encoded_bytes > MAX_CANDIDATE_ENCODED_PATH_BYTES
        || display_bytes > MAX_CANDIDATE_DISPLAY_PATH_BYTES
    {
        return Err(EngineError::BudgetExceeded);
    }
    encoded_bytes
        .checked_add(display_bytes)
        .ok_or(EngineError::BudgetExceeded)
}

fn candidate_evidence_payload_bytes(
    evidence: &CoreCandidateEvidence,
) -> Result<usize, EngineError> {
    let (path, identifier) = match evidence {
        CoreCandidateEvidence::MatchedPath { path }
        | CoreCandidateEvidence::RequiredMarker { path }
        | CoreCandidateEvidence::ForbiddenMarkerAbsent { path }
        | CoreCandidateEvidence::CloudUploadComplete { path } => (Some(path), None),
        CoreCandidateEvidence::BundleIdentifier { path, identifier } => {
            (Some(path), Some(identifier.len()))
        }
        CoreCandidateEvidence::InactiveProcess { identifier } => (None, Some(identifier.len())),
        CoreCandidateEvidence::MinimumAge { .. } | CoreCandidateEvidence::MinimumSize { .. } => {
            (None, None)
        }
        _ => return Err(EngineError::InternalState),
    };
    let path_bytes = path.map_or(Ok(0), candidate_path_payload_bytes)?;
    let identifier_bytes = identifier.unwrap_or(0);
    if identifier_bytes > MAX_CANDIDATE_IDENTIFIER_BYTES {
        return Err(EngineError::BudgetExceeded);
    }
    path_bytes
        .checked_add(identifier_bytes)
        .ok_or(EngineError::BudgetExceeded)
}

fn ensure_candidate_detail_page_payload(
    payloads: impl IntoIterator<Item = Result<usize, EngineError>>,
) -> Result<(), EngineError> {
    let mut total = 0_usize;
    for payload in payloads {
        total = total
            .checked_add(payload?)
            .ok_or(EngineError::BudgetExceeded)?;
        if total > MAX_CANDIDATE_DETAIL_PAGE_PAYLOAD_BYTES {
            return Err(EngineError::BudgetExceeded);
        }
    }
    Ok(())
}

fn project_candidate_summary(
    summary: &CoreCandidateSummary,
) -> Result<CandidateSummary, EngineError> {
    Ok(CandidateSummary {
        record_version: FFI_RECORD_VERSION,
        candidate_id: summary.id().as_str().to_owned(),
        rule_id: summary.rule().id().as_str().to_owned(),
        rule_revision: summary.rule().revision().get(),
        category: map_candidate_category(summary.category()),
        estimated_bytes: summary.estimated_bytes(),
        newest_mtime: summary
            .newest_mtime()
            .map(project_system_time_timestamp)
            .transpose()?,
        safety: map_candidate_safety(summary.safety()),
        action: map_candidate_action(summary.action()),
        rule_schedule_eligible: summary.rule_schedule_eligible(),
        path_count: summary.path_count(),
        evidence_kinds: summary
            .evidence_kinds()
            .iter()
            .copied()
            .map(map_candidate_evidence_kind)
            .collect(),
        blockers: summary
            .blockers()
            .iter()
            .map(|reason| map_candidate_block_reason(reason.clone()))
            .collect(),
        created_at: project_system_time_timestamp(summary.created_at())?,
        status: map_candidate_status(summary.status()),
    })
}

fn project_candidate_evidence(
    ordinal: u16,
    evidence: &CoreCandidateEvidence,
) -> Result<CandidateEvidenceRecord, EngineError> {
    let empty = || CandidateEvidenceRecord {
        record_version: FFI_RECORD_VERSION,
        ordinal,
        kind: CandidateEvidenceKind::MatchedPath,
        path: None,
        identifier: None,
        newest_mtime: None,
        minimum_age_seconds: None,
        minimum_age_nanoseconds: None,
        observed_bytes: None,
        minimum_bytes: None,
    };
    let mut output = empty();
    match evidence {
        CoreCandidateEvidence::MatchedPath { path } => {
            output.kind = CandidateEvidenceKind::MatchedPath;
            output.path = Some(project_candidate_path(path)?);
        }
        CoreCandidateEvidence::RequiredMarker { path } => {
            output.kind = CandidateEvidenceKind::RequiredMarker;
            output.path = Some(project_candidate_path(path)?);
        }
        CoreCandidateEvidence::ForbiddenMarkerAbsent { path } => {
            output.kind = CandidateEvidenceKind::ForbiddenMarkerAbsent;
            output.path = Some(project_candidate_path(path)?);
        }
        CoreCandidateEvidence::BundleIdentifier { path, identifier } => {
            output.kind = CandidateEvidenceKind::BundleIdentifier;
            output.path = Some(project_candidate_path(path)?);
            output.identifier = Some(identifier.to_string());
        }
        CoreCandidateEvidence::MinimumAge {
            newest_mtime,
            minimum_age,
        } => {
            output.kind = CandidateEvidenceKind::MinimumAge;
            output.newest_mtime = Some(project_system_time_timestamp(*newest_mtime)?);
            output.minimum_age_seconds = Some(minimum_age.as_secs());
            output.minimum_age_nanoseconds = Some(minimum_age.subsec_nanos());
        }
        CoreCandidateEvidence::MinimumSize {
            observed_bytes,
            minimum_bytes,
        } => {
            output.kind = CandidateEvidenceKind::MinimumSize;
            output.observed_bytes = Some(*observed_bytes);
            output.minimum_bytes = Some(*minimum_bytes);
        }
        CoreCandidateEvidence::InactiveProcess { identifier } => {
            output.kind = CandidateEvidenceKind::InactiveProcess;
            output.identifier = Some(identifier.to_string());
        }
        CoreCandidateEvidence::CloudUploadComplete { path } => {
            output.kind = CandidateEvidenceKind::CloudUploadComplete;
            output.path = Some(project_candidate_path(path)?);
        }
        _ => return Err(EngineError::InternalState),
    }
    Ok(output)
}

fn project_system_time_timestamp(value: SystemTime) -> Result<SnapshotNodeTimestamp, EngineError> {
    let duration = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| EngineError::InternalState)?;
    Ok(SnapshotNodeTimestamp {
        seconds_since_unix_epoch: duration.as_secs(),
        nanoseconds: duration.subsec_nanos(),
    })
}

const fn map_candidate_category(category: CoreCandidateCategory) -> CandidateCategory {
    match category {
        CoreCandidateCategory::DeveloperArtifact => CandidateCategory::DeveloperArtifact,
        CoreCandidateCategory::ApplicationCache => CandidateCategory::ApplicationCache,
        CoreCandidateCategory::BrowserCache => CandidateCategory::BrowserCache,
        CoreCandidateCategory::LogAndDiagnostic => CandidateCategory::LogAndDiagnostic,
        CoreCandidateCategory::InstallerAndDownload => CandidateCategory::InstallerAndDownload,
        CoreCandidateCategory::DeviceAndSimulatorData => CandidateCategory::DeviceAndSimulatorData,
        CoreCandidateCategory::CloudFile => CandidateCategory::CloudFile,
        CoreCandidateCategory::LargeReviewItem => CandidateCategory::LargeReviewItem,
        CoreCandidateCategory::ProtectedSystemData => CandidateCategory::ProtectedSystemData,
        CoreCandidateCategory::UnknownStorage => CandidateCategory::UnknownStorage,
    }
}

const fn map_candidate_safety(safety: CoreSafetyTier) -> CandidateSafety {
    match safety {
        CoreSafetyTier::SafeRegenerable => CandidateSafety::SafeRegenerable,
        CoreSafetyTier::SafeEvictable => CandidateSafety::SafeEvictable,
        CoreSafetyTier::ReviewRequired => CandidateSafety::ReviewRequired,
        CoreSafetyTier::Informational => CandidateSafety::Informational,
        CoreSafetyTier::Protected => CandidateSafety::Protected,
    }
}

const fn map_candidate_action(action: CoreCandidateAction) -> CandidateAction {
    match action {
        CoreCandidateAction::RemoveKnownRegenerableContents => {
            CandidateAction::RemoveKnownRegenerableContents
        }
        CoreCandidateAction::EvictLocalCopy => CandidateAction::EvictLocalCopy,
        CoreCandidateAction::MoveToTrash => CandidateAction::MoveToTrash,
        CoreCandidateAction::RevealOnly => CandidateAction::RevealOnly,
        CoreCandidateAction::NoAction => CandidateAction::NoAction,
    }
}

const fn map_candidate_evidence_kind(kind: CoreEvidenceKind) -> CandidateEvidenceKind {
    match kind {
        CoreEvidenceKind::MatchedPath => CandidateEvidenceKind::MatchedPath,
        CoreEvidenceKind::RequiredMarker => CandidateEvidenceKind::RequiredMarker,
        CoreEvidenceKind::ForbiddenMarkerAbsent => CandidateEvidenceKind::ForbiddenMarkerAbsent,
        CoreEvidenceKind::BundleIdentifier => CandidateEvidenceKind::BundleIdentifier,
        CoreEvidenceKind::MinimumAge => CandidateEvidenceKind::MinimumAge,
        CoreEvidenceKind::MinimumSize => CandidateEvidenceKind::MinimumSize,
        CoreEvidenceKind::InactiveProcess => CandidateEvidenceKind::InactiveProcess,
        CoreEvidenceKind::CloudUploadComplete => CandidateEvidenceKind::CloudUploadComplete,
    }
}

fn map_candidate_block_reason(reason: CoreBlockReason) -> CandidateBlockReason {
    match reason {
        CoreBlockReason::MissingOrIncompleteEvidence => {
            CandidateBlockReason::MissingOrIncompleteEvidence
        }
        CoreBlockReason::MissingModificationTime => CandidateBlockReason::MissingModificationTime,
        CoreBlockReason::PartialScanCoverage => CandidateBlockReason::PartialScanCoverage,
        CoreBlockReason::RecentActivity => CandidateBlockReason::RecentActivity,
        CoreBlockReason::BelowMinimumBytes => CandidateBlockReason::BelowMinimumBytes,
        CoreBlockReason::ActiveUse => CandidateBlockReason::ActiveUse,
        CoreBlockReason::AccessDenied => CandidateBlockReason::AccessDenied,
        CoreBlockReason::ProtectedPath => CandidateBlockReason::ProtectedPath,
        CoreBlockReason::ProtectedDescendant => CandidateBlockReason::ProtectedDescendant,
        CoreBlockReason::SymlinkBoundary => CandidateBlockReason::SymlinkBoundary,
        CoreBlockReason::VolumeBoundary => CandidateBlockReason::VolumeBoundary,
        CoreBlockReason::ChangedSinceScan => CandidateBlockReason::ChangedSinceScan,
        CoreBlockReason::UnsupportedPlatform => CandidateBlockReason::UnsupportedPlatform,
        CoreBlockReason::CloudUploadUnconfirmed => CandidateBlockReason::CloudUploadUnconfirmed,
    }
}

const fn map_candidate_status(status: CoreCandidateStatus) -> CandidateStatus {
    match status {
        CoreCandidateStatus::Discovered => CandidateStatus::Discovered,
        CoreCandidateStatus::Selected => CandidateStatus::Selected,
        CoreCandidateStatus::Dismissed => CandidateStatus::Dismissed,
        CoreCandidateStatus::Stale => CandidateStatus::Stale,
        CoreCandidateStatus::Planned => CandidateStatus::Planned,
        CoreCandidateStatus::Completed => CandidateStatus::Completed,
        CoreCandidateStatus::Failed => CandidateStatus::Failed,
        CoreCandidateStatus::Unavailable => CandidateStatus::Unavailable,
        _ => CandidateStatus::Unavailable,
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

fn map_cleanup_history_error(error: CoreCleanupHistoryError) -> CleanupHistoryError {
    match error {
        CoreCleanupHistoryError::Closed => CleanupHistoryError::Closed,
        CoreCleanupHistoryError::InvalidLimit { .. } => CleanupHistoryError::InvalidLimit,
        CoreCleanupHistoryError::SessionNotFound => CleanupHistoryError::SessionNotFound,
        CoreCleanupHistoryError::IncompatibleSchema => CleanupHistoryError::IncompatibleSchema,
        CoreCleanupHistoryError::Busy => CleanupHistoryError::Busy,
        CoreCleanupHistoryError::UnsafeStorage => CleanupHistoryError::UnsafeStorage,
        CoreCleanupHistoryError::QueryLimitExceeded => CleanupHistoryError::BudgetExceeded,
        CoreCleanupHistoryError::CorruptData => CleanupHistoryError::CorruptData,
        CoreCleanupHistoryError::Unavailable => CleanupHistoryError::Unavailable,
        CoreCleanupHistoryError::InternalState => CleanupHistoryError::InternalState,
        _ => CleanupHistoryError::InternalState,
    }
}

fn core_cleanup_history_cursor(
    cursor: CleanupHistoryCursor,
) -> Result<CoreCleanupHistoryCursor, CleanupHistoryError> {
    if cursor.record_version != FFI_RECORD_VERSION {
        return Err(CleanupHistoryError::InvalidCursor);
    }
    let started_at = cleanup_history_system_time(cursor.started_at_unix_ms)?;
    let session_id = CoreCleanupSessionId::from_stable_str(cursor.session_id)
        .ok_or(CleanupHistoryError::InvalidCursor)?;
    Ok(CoreCleanupHistoryCursor::new(started_at, session_id))
}

fn cleanup_history_system_time(value: i64) -> Result<SystemTime, CleanupHistoryError> {
    let milliseconds = u64::try_from(value).map_err(|_| CleanupHistoryError::InvalidCursor)?;
    UNIX_EPOCH
        .checked_add(Duration::from_millis(milliseconds))
        .ok_or(CleanupHistoryError::InvalidCursor)
}

fn cleanup_history_time_ms(value: SystemTime) -> Result<i64, CleanupHistoryError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CleanupHistoryError::CorruptData)?
            .as_millis(),
    )
    .map_err(|_| CleanupHistoryError::CorruptData)
}

fn cleanup_history_cursor(
    cursor: &CoreCleanupHistoryCursor,
) -> Result<CleanupHistoryCursor, CleanupHistoryError> {
    Ok(CleanupHistoryCursor {
        record_version: FFI_RECORD_VERSION,
        started_at_unix_ms: cleanup_history_time_ms(cursor.started_at())?,
        session_id: cursor.session_id().as_str().to_owned(),
    })
}

fn cleanup_status_counts(counts: &CoreCleanupStatusCounts) -> CleanupStatusCounts {
    CleanupStatusCounts {
        planned: counts.planned(),
        validating: counts.validating(),
        dry_run: counts.dry_run(),
        effect_started: counts.effect_started(),
        trashed: counts.trashed(),
        removed: counts.removed(),
        evicted: counts.evicted(),
        skipped: counts.skipped(),
        rejected: counts.rejected(),
        failed: counts.failed(),
        changed_since_plan: counts.changed_since_plan(),
        interrupted: counts.interrupted(),
        unavailable: counts.unavailable(),
        outcome_unknown: counts.outcome_unknown(),
        total: counts.total(),
    }
}

fn cleanup_session_summary(
    summary: &dux_core::engine::DurableCleanupSessionSummary,
) -> Result<CleanupSessionSummary, CleanupHistoryError> {
    Ok(CleanupSessionSummary {
        record_version: FFI_RECORD_VERSION,
        session_id: summary.id().as_str().to_owned(),
        plan_id: summary.plan_id().to_owned(),
        format: match summary.format() {
            CoreCleanupRecordFormat::LegacyIncomplete => CleanupRecordFormat::LegacyIncomplete,
            CoreCleanupRecordFormat::Complete => CleanupRecordFormat::Complete,
            _ => return Err(CleanupHistoryError::InternalState),
        },
        source_scan_id: summary.source_scan_id().map(|id| id.as_str().to_owned()),
        started_at_unix_ms: cleanup_history_time_ms(summary.started_at())?,
        completed_at_unix_ms: summary
            .completed_at()
            .map(cleanup_history_time_ms)
            .transpose()?,
        plan_created_at_unix_ms: summary
            .plan_created_at()
            .map(cleanup_history_time_ms)
            .transpose()?,
        plan_expires_at_unix_ms: summary
            .plan_expires_at()
            .map(cleanup_history_time_ms)
            .transpose()?,
        mode: match summary.mode() {
            CoreCleanupMode::DryRun => CleanupMode::DryRun,
            CoreCleanupMode::Trash => CleanupMode::Trash,
            CoreCleanupMode::PermanentSafe => CleanupMode::PermanentSafe,
            CoreCleanupMode::EvictLocalCopy => CleanupMode::EvictLocalCopy,
            _ => return Err(CleanupHistoryError::InternalState),
        },
        trigger: match summary.trigger() {
            CoreCleanupTrigger::Manual => CleanupTrigger::Manual,
            CoreCleanupTrigger::LowDisk => CleanupTrigger::LowDisk,
            CoreCleanupTrigger::Scheduled => CleanupTrigger::Scheduled,
            CoreCleanupTrigger::Cli => CleanupTrigger::Cli,
            _ => return Err(CleanupHistoryError::InternalState),
        },
        status: map_cleanup_session_status(summary.status())?,
        estimated_bytes: summary.estimated_bytes(),
        verified_capacity_delta_bytes: summary.verified_capacity_delta_bytes(),
        cancellation_requested: summary.cancellation_requested(),
        item_total: summary.item_total(),
        path_total: summary.path_total(),
        evidence_total: summary.evidence_total(),
        item_status_counts: cleanup_status_counts(summary.item_status_counts()),
        path_status_counts: cleanup_status_counts(summary.path_status_counts()),
    })
}

fn map_cleanup_session_status(
    status: CoreCleanupSessionStatus,
) -> Result<CleanupSessionStatus, CleanupHistoryError> {
    Ok(match status {
        CoreCleanupSessionStatus::Planned => CleanupSessionStatus::Planned,
        CoreCleanupSessionStatus::Running => CleanupSessionStatus::Running,
        CoreCleanupSessionStatus::Recovering => CleanupSessionStatus::Recovering,
        CoreCleanupSessionStatus::Completed => CleanupSessionStatus::Completed,
        CoreCleanupSessionStatus::PartiallyCompleted => CleanupSessionStatus::PartiallyCompleted,
        CoreCleanupSessionStatus::Failed => CleanupSessionStatus::Failed,
        CoreCleanupSessionStatus::Cancelled => CleanupSessionStatus::Cancelled,
        CoreCleanupSessionStatus::Interrupted => CleanupSessionStatus::Interrupted,
        CoreCleanupSessionStatus::Rejected => CleanupSessionStatus::Rejected,
        CoreCleanupSessionStatus::DryRun => CleanupSessionStatus::DryRun,
        _ => return Err(CleanupHistoryError::InternalState),
    })
}

fn map_cleanup_item_status(
    status: CoreCleanupItemStatus,
) -> Result<CleanupItemStatus, CleanupHistoryError> {
    Ok(match status {
        CoreCleanupItemStatus::Planned => CleanupItemStatus::Planned,
        CoreCleanupItemStatus::Validating => CleanupItemStatus::Validating,
        CoreCleanupItemStatus::DryRun => CleanupItemStatus::DryRun,
        CoreCleanupItemStatus::EffectStarted => CleanupItemStatus::EffectStarted,
        CoreCleanupItemStatus::Trashed => CleanupItemStatus::Trashed,
        CoreCleanupItemStatus::Removed => CleanupItemStatus::Removed,
        CoreCleanupItemStatus::Evicted => CleanupItemStatus::Evicted,
        CoreCleanupItemStatus::Skipped => CleanupItemStatus::Skipped,
        CoreCleanupItemStatus::Rejected => CleanupItemStatus::Rejected,
        CoreCleanupItemStatus::Failed => CleanupItemStatus::Failed,
        CoreCleanupItemStatus::ChangedSincePlan => CleanupItemStatus::ChangedSincePlan,
        CoreCleanupItemStatus::Interrupted => CleanupItemStatus::Interrupted,
        CoreCleanupItemStatus::Unavailable => CleanupItemStatus::Unavailable,
        CoreCleanupItemStatus::OutcomeUnknown => CleanupItemStatus::OutcomeUnknown,
        _ => return Err(CleanupHistoryError::InternalState),
    })
}

fn cleanup_item_summary(
    item: &CoreCleanupItemSummary,
) -> Result<CleanupItemSummary, CleanupHistoryError> {
    Ok(CleanupItemSummary {
        record_version: FFI_RECORD_VERSION,
        ordinal: item.ordinal(),
        rule_id: item.rule().id().as_str().to_owned(),
        rule_revision: item.rule().revision().get(),
        category: item.category().map(map_candidate_category),
        safety: item.safety().map(map_candidate_safety),
        action: item.action().map(map_candidate_action),
        rule_schedule_eligible: item.rule_schedule_eligible(),
        newest_mtime_unix_ms: item
            .newest_mtime()
            .map(cleanup_history_time_ms)
            .transpose()?,
        estimated_bytes: item.estimated_bytes(),
        status: map_cleanup_item_status(item.status())?,
        error_recorded: item.error_recorded(),
        error_category: item
            .error_category()
            .map(|category| category.as_str().to_owned()),
        path_count: item.path_count(),
        evidence_count: item.evidence_count(),
    })
}

const fn cleanup_warning(
    warning: CoreCleanupWarning,
) -> Result<CleanupWarning, CleanupHistoryError> {
    Ok(match warning {
        CoreCleanupWarning::EstimatedBytesUnverified => CleanupWarning::EstimatedBytesUnverified,
        CoreCleanupWarning::DryRunDoesNotMutate => CleanupWarning::DryRunDoesNotMutate,
        CoreCleanupWarning::TrashDoesNotFreeSpaceImmediately => {
            CleanupWarning::TrashDoesNotFreeSpaceImmediately
        }
        CoreCleanupWarning::PermanentRemovalCannotBeUndone => {
            CleanupWarning::PermanentRemovalCannotBeUndone
        }
        CoreCleanupWarning::CloudEvictionRequiresNetworkToRedownload => {
            CleanupWarning::CloudEvictionRequiresNetworkToRedownload
        }
        _ => return Err(CleanupHistoryError::InternalState),
    })
}

fn cleanup_session_history(
    observation: CoreCleanupSessionObservation,
) -> Result<CleanupSessionHistory, CleanupHistoryError> {
    let history = CleanupSessionHistory {
        record_version: FFI_RECORD_VERSION,
        summary: cleanup_session_summary(observation.summary())?,
        items: observation
            .items()
            .iter()
            .map(cleanup_item_summary)
            .collect::<Result<Vec<_>, _>>()?,
        warnings: observation
            .warnings()
            .iter()
            .copied()
            .map(cleanup_warning)
            .collect::<Result<Vec<_>, _>>()?,
    };
    validate_cleanup_session_history(&history)?;
    Ok(history)
}

fn validate_cleanup_session_history(
    history: &CleanupSessionHistory,
) -> Result<(), CleanupHistoryError> {
    if history.record_version != FFI_RECORD_VERSION
        || history.summary.record_version != FFI_RECORD_VERSION
        || history.items.len() > MAX_CLEANUP_HISTORY_ITEMS
        || history.warnings.len() > MAX_CLEANUP_HISTORY_WARNINGS
        || history
            .warnings
            .iter()
            .enumerate()
            .any(|(index, warning)| history.warnings[..index].contains(warning))
    {
        return Err(CleanupHistoryError::CorruptData);
    }
    validate_cleanup_session_summary(&history.summary)?;
    if history.warnings != expected_cleanup_warnings(history) {
        return Err(CleanupHistoryError::CorruptData);
    }

    let mut path_total = 0_u16;
    let mut evidence_total = 0_u16;
    let mut estimated_bytes_total = 0_u64;
    let mut item_counts = CleanupStatusCounts {
        planned: 0,
        validating: 0,
        dry_run: 0,
        effect_started: 0,
        trashed: 0,
        removed: 0,
        evicted: 0,
        skipped: 0,
        rejected: 0,
        failed: 0,
        changed_since_plan: 0,
        interrupted: 0,
        unavailable: 0,
        outcome_unknown: 0,
        total: 0,
    };
    for (expected_ordinal, item) in history.items.iter().enumerate() {
        validate_cleanup_item_summary(item, history.summary.format)?;
        if usize::from(item.ordinal) != expected_ordinal {
            return Err(CleanupHistoryError::CorruptData);
        }
        path_total = path_total
            .checked_add(item.path_count)
            .ok_or(CleanupHistoryError::CorruptData)?;
        evidence_total = evidence_total
            .checked_add(item.evidence_count)
            .ok_or(CleanupHistoryError::CorruptData)?;
        estimated_bytes_total = estimated_bytes_total
            .checked_add(item.estimated_bytes)
            .ok_or(CleanupHistoryError::CorruptData)?;
        add_cleanup_status(&mut item_counts, item.status)?;
    }
    if history.items.len() != usize::from(history.summary.item_total)
        || path_total != history.summary.path_total
        || evidence_total != history.summary.evidence_total
        || item_counts != history.summary.item_status_counts
        || (history.summary.format == CleanupRecordFormat::Complete
            && estimated_bytes_total != history.summary.estimated_bytes)
    {
        return Err(CleanupHistoryError::CorruptData);
    }
    Ok(())
}

fn expected_cleanup_warnings(history: &CleanupSessionHistory) -> Vec<CleanupWarning> {
    if history.summary.format == CleanupRecordFormat::LegacyIncomplete {
        return Vec::new();
    }

    let mut warnings = vec![CleanupWarning::EstimatedBytesUnverified];
    match history.summary.mode {
        CleanupMode::DryRun => warnings.push(CleanupWarning::DryRunDoesNotMutate),
        CleanupMode::Trash => warnings.push(CleanupWarning::TrashDoesNotFreeSpaceImmediately),
        CleanupMode::PermanentSafe => {
            warnings.push(CleanupWarning::PermanentRemovalCannotBeUndone);
        }
        CleanupMode::EvictLocalCopy => {}
    }
    if history
        .items
        .iter()
        .any(|item| item.action == Some(CandidateAction::EvictLocalCopy))
    {
        warnings.push(CleanupWarning::CloudEvictionRequiresNetworkToRedownload);
    }
    warnings
}

fn validate_cleanup_session_summary(
    summary: &CleanupSessionSummary,
) -> Result<(), CleanupHistoryError> {
    if summary.record_version != FFI_RECORD_VERSION
        || !is_bounded_cleanup_history_token(
            &summary.session_id,
            MAX_CLEANUP_HISTORY_SESSION_ID_BYTES,
        )
        || !is_bounded_cleanup_history_token(&summary.plan_id, MAX_CLEANUP_HISTORY_PLAN_ID_BYTES)
        || summary
            .source_scan_id
            .as_deref()
            .is_some_and(|id| !is_bounded_cleanup_history_token(id, 128))
        || summary.started_at_unix_ms < 0
        || summary
            .completed_at_unix_ms
            .is_some_and(|completed| completed < summary.started_at_unix_ms)
        || usize::from(summary.item_total) > MAX_CLEANUP_HISTORY_ITEMS
        || summary.path_total > MAX_CLEANUP_HISTORY_PATHS
        || summary.evidence_total > MAX_CLEANUP_HISTORY_EVIDENCE
    {
        return Err(CleanupHistoryError::CorruptData);
    }
    validate_cleanup_status_counts(&summary.item_status_counts, summary.item_total)?;
    validate_cleanup_status_counts(&summary.path_status_counts, summary.path_total)?;
    let complete_fields = (
        summary.source_scan_id.as_ref(),
        summary.plan_created_at_unix_ms,
        summary.plan_expires_at_unix_ms,
    );
    match summary.format {
        CleanupRecordFormat::LegacyIncomplete => {
            if complete_fields != (None, None, None)
                || summary.cancellation_requested.is_some()
                || summary.evidence_total != 0
                || summary.status == CleanupSessionStatus::Recovering
            {
                return Err(CleanupHistoryError::CorruptData);
            }
        }
        CleanupRecordFormat::Complete => {
            let (Some(_), Some(created), Some(expires)) = complete_fields else {
                return Err(CleanupHistoryError::CorruptData);
            };
            if summary.cancellation_requested.is_none()
                || created < 0
                || expires <= summary.started_at_unix_ms
                || created > summary.started_at_unix_ms
            {
                return Err(CleanupHistoryError::CorruptData);
            }
            let terminal = !matches!(
                summary.status,
                CleanupSessionStatus::Planned
                    | CleanupSessionStatus::Running
                    | CleanupSessionStatus::Recovering
            );
            if terminal != summary.completed_at_unix_ms.is_some()
                || (!terminal && summary.verified_capacity_delta_bytes.is_some())
                || (summary.status == CleanupSessionStatus::Planned
                    && summary.cancellation_requested != Some(false))
            {
                return Err(CleanupHistoryError::CorruptData);
            }
        }
    }
    Ok(())
}

fn validate_cleanup_item_summary(
    item: &CleanupItemSummary,
    format: CleanupRecordFormat,
) -> Result<(), CleanupHistoryError> {
    if item.record_version != FFI_RECORD_VERSION
        || usize::from(item.ordinal) >= MAX_CLEANUP_HISTORY_ITEMS
        || !is_bounded_cleanup_history_token(&item.rule_id, MAX_CLEANUP_HISTORY_RULE_ID_BYTES)
        || item.rule_revision == 0
        || item.newest_mtime_unix_ms.is_some_and(|value| value < 0)
        || item.path_count == 0
        || item.path_count > MAX_CLEANUP_HISTORY_PATHS
        || item.evidence_count > MAX_CLEANUP_HISTORY_EVIDENCE
        || (!item.error_recorded && item.error_category.is_some())
        || item.error_category.as_deref().is_some_and(|category| {
            !is_bounded_cleanup_history_token(category, MAX_CLEANUP_HISTORY_ERROR_CATEGORY_BYTES)
        })
    {
        return Err(CleanupHistoryError::CorruptData);
    }
    let policy_complete = item.category.is_some()
        && item.safety.is_some()
        && item.action.is_some()
        && item.rule_schedule_eligible.is_some();
    match format {
        CleanupRecordFormat::LegacyIncomplete => {
            if policy_complete
                || item.category.is_some()
                || item.safety.is_some()
                || item.action.is_some()
                || item.rule_schedule_eligible.is_some()
                || item.newest_mtime_unix_ms.is_some()
                || item.evidence_count != 0
                || item.error_category.is_some()
            {
                return Err(CleanupHistoryError::CorruptData);
            }
        }
        CleanupRecordFormat::Complete if !policy_complete => {
            return Err(CleanupHistoryError::CorruptData);
        }
        CleanupRecordFormat::Complete => {}
    }
    Ok(())
}

fn validate_cleanup_status_counts(
    counts: &CleanupStatusCounts,
    expected_total: u16,
) -> Result<(), CleanupHistoryError> {
    let total = [
        counts.planned,
        counts.validating,
        counts.dry_run,
        counts.effect_started,
        counts.trashed,
        counts.removed,
        counts.evicted,
        counts.skipped,
        counts.rejected,
        counts.failed,
        counts.changed_since_plan,
        counts.interrupted,
        counts.unavailable,
        counts.outcome_unknown,
    ]
    .into_iter()
    .try_fold(0_u16, u16::checked_add)
    .ok_or(CleanupHistoryError::CorruptData)?;
    if total != counts.total || total != expected_total {
        return Err(CleanupHistoryError::CorruptData);
    }
    Ok(())
}

fn add_cleanup_status(
    counts: &mut CleanupStatusCounts,
    status: CleanupItemStatus,
) -> Result<(), CleanupHistoryError> {
    counts.total = counts
        .total
        .checked_add(1)
        .ok_or(CleanupHistoryError::CorruptData)?;
    let count = match status {
        CleanupItemStatus::Planned => &mut counts.planned,
        CleanupItemStatus::Validating => &mut counts.validating,
        CleanupItemStatus::DryRun => &mut counts.dry_run,
        CleanupItemStatus::EffectStarted => &mut counts.effect_started,
        CleanupItemStatus::Trashed => &mut counts.trashed,
        CleanupItemStatus::Removed => &mut counts.removed,
        CleanupItemStatus::Evicted => &mut counts.evicted,
        CleanupItemStatus::Skipped => &mut counts.skipped,
        CleanupItemStatus::Rejected => &mut counts.rejected,
        CleanupItemStatus::Failed => &mut counts.failed,
        CleanupItemStatus::ChangedSincePlan => &mut counts.changed_since_plan,
        CleanupItemStatus::Interrupted => &mut counts.interrupted,
        CleanupItemStatus::Unavailable => &mut counts.unavailable,
        CleanupItemStatus::OutcomeUnknown => &mut counts.outcome_unknown,
    };
    *count = count
        .checked_add(1)
        .ok_or(CleanupHistoryError::CorruptData)?;
    Ok(())
}

fn is_bounded_cleanup_history_token(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn cleanup_history_page(
    page: CoreCleanupHistoryPage,
) -> Result<CleanupHistoryPage, CleanupHistoryError> {
    Ok(CleanupHistoryPage {
        record_version: FFI_RECORD_VERSION,
        records: page
            .records()
            .iter()
            .map(cleanup_session_summary)
            .collect::<Result<Vec<_>, _>>()?,
        next_cursor: page.next_cursor().map(cleanup_history_cursor).transpose()?,
    })
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

fn capacity_trend_status(trend: CoreCapacityTrend) -> Result<CapacityTrendStatus, EngineError> {
    Ok(CapacityTrendStatus {
        record_version: FFI_RECORD_VERSION,
        stable_volume_id: trend.volume_id().to_string(),
        sampled_at_unix_ms: system_time_ms(trend.sampled_at())?,
        total_bytes: trend.total_bytes(),
        available_bytes: trend.available_bytes(),
        important_available_bytes: trend.important_available_bytes(),
        pressure: map_volume_pressure(trend.pressure()),
        change_24h: trend.change_24h().map(capacity_trend_change).transpose()?,
        change_7d: trend.change_7d().map(capacity_trend_change).transpose()?,
        points: trend
            .points()
            .iter()
            .map(capacity_trend_point)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn capacity_trend_change(
    change: &CoreCapacityTrendChange,
) -> Result<CapacityTrendChange, EngineError> {
    Ok(CapacityTrendChange {
        record_version: FFI_RECORD_VERSION,
        from_unix_ms: system_time_ms(change.from())?,
        to_unix_ms: system_time_ms(change.to())?,
        total_bytes: change.total_bytes(),
        available_bytes: change.available_bytes(),
        important_available_bytes: change.important_available_bytes(),
    })
}

fn capacity_trend_point(point: &CoreCapacityTrendPoint) -> Result<CapacityTrendPoint, EngineError> {
    Ok(CapacityTrendPoint {
        record_version: FFI_RECORD_VERSION,
        sampled_at_unix_ms: system_time_ms(point.sampled_at())?,
        total_bytes: point.total_bytes(),
        available_bytes: point.available_bytes(),
        important_available_bytes: point.important_available_bytes(),
        pressure: map_volume_pressure(point.pressure()),
        source: match point.source() {
            CoreCapacityTrendPointSource::Raw => CapacityTrendPointSource::Raw,
            CoreCapacityTrendPointSource::DailyRollup => CapacityTrendPointSource::DailyRollup,
        },
    })
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

fn map_permanent_cleanup_policy_error(
    error: CorePermanentCleanupPolicyError,
) -> PermanentCleanupPolicyError {
    match error {
        CorePermanentCleanupPolicyError::Closed => PermanentCleanupPolicyError::Closed,
        CorePermanentCleanupPolicyError::RevisionExhausted => {
            PermanentCleanupPolicyError::RevisionExhausted
        }
        CorePermanentCleanupPolicyError::InvalidClock => PermanentCleanupPolicyError::InvalidClock,
        CorePermanentCleanupPolicyError::IncompatibleSchema => {
            PermanentCleanupPolicyError::IncompatibleSchema
        }
        CorePermanentCleanupPolicyError::Busy => PermanentCleanupPolicyError::Busy,
        CorePermanentCleanupPolicyError::UnsafeStorage => {
            PermanentCleanupPolicyError::UnsafeStorage
        }
        CorePermanentCleanupPolicyError::QueryLimitExceeded => {
            PermanentCleanupPolicyError::BudgetExceeded
        }
        CorePermanentCleanupPolicyError::CorruptData => PermanentCleanupPolicyError::CorruptData,
        CorePermanentCleanupPolicyError::Unavailable => PermanentCleanupPolicyError::Unavailable,
        CorePermanentCleanupPolicyError::OutcomeUnknown => {
            PermanentCleanupPolicyError::OutcomeUnknown
        }
        CorePermanentCleanupPolicyError::InternalState => {
            PermanentCleanupPolicyError::InternalState
        }
        _ => PermanentCleanupPolicyError::InternalState,
    }
}

fn map_cleanup_exclusions_error(error: CoreCleanupExclusionsError) -> CleanupExclusionsError {
    match error {
        CoreCleanupExclusionsError::Closed => CleanupExclusionsError::Closed,
        CoreCleanupExclusionsError::InvalidInput => CleanupExclusionsError::InvalidPath,
        CoreCleanupExclusionsError::RevisionExhausted => CleanupExclusionsError::RevisionExhausted,
        CoreCleanupExclusionsError::IncompatibleSchema => {
            CleanupExclusionsError::IncompatibleSchema
        }
        CoreCleanupExclusionsError::Busy => CleanupExclusionsError::Busy,
        CoreCleanupExclusionsError::UnsafeStorage => CleanupExclusionsError::UnsafeStorage,
        CoreCleanupExclusionsError::QueryLimitExceeded => CleanupExclusionsError::BudgetExceeded,
        CoreCleanupExclusionsError::CorruptData => CleanupExclusionsError::CorruptData,
        CoreCleanupExclusionsError::Unavailable => CleanupExclusionsError::Unavailable,
        CoreCleanupExclusionsError::OutcomeUnknown => CleanupExclusionsError::OutcomeUnknown,
        CoreCleanupExclusionsError::InternalState => CleanupExclusionsError::InternalState,
        _ => CleanupExclusionsError::InternalState,
    }
}

fn cleanup_exclusions_status(
    exclusions: CoreCleanupExclusions,
) -> Result<CleanupExclusionsStatus, CleanupExclusionsError> {
    if exclusions.paths.len() > MAX_CLEANUP_EXCLUSION_COUNT {
        return Err(CleanupExclusionsError::InternalState);
    }
    let updated_at_unix_ms = exclusions
        .updated_at
        .map(cleanup_exclusions_time_ms)
        .transpose()?;
    if (exclusions.revision == 0) != updated_at_unix_ms.is_none()
        || (exclusions.revision == 0 && exclusions.source != CoreCleanupExclusionSource::Default)
        || (exclusions.revision == 0 && !exclusions.paths.is_empty())
        || (exclusions.revision > 0 && exclusions.source != CoreCleanupExclusionSource::Stored)
    {
        return Err(CleanupExclusionsError::InternalState);
    }
    let paths = exclusions
        .paths
        .iter()
        .map(|path| cleanup_exclusion_path(path))
        .collect::<Result<Vec<_>, _>>()?;
    if paths.windows(2).any(|pair| {
        cleanup_exclusion_path_order_key(&pair[0]) >= cleanup_exclusion_path_order_key(&pair[1])
    }) {
        return Err(CleanupExclusionsError::InternalState);
    }
    Ok(CleanupExclusionsStatus {
        record_version: FFI_RECORD_VERSION,
        paths,
        source: match exclusions.source {
            CoreCleanupExclusionSource::Default => CleanupExclusionsSource::Default,
            CoreCleanupExclusionSource::Stored => CleanupExclusionsSource::Stored,
        },
        revision: exclusions.revision,
        updated_at_unix_ms,
    })
}

fn cleanup_exclusion_path_order_key(path: &CleanupExclusionPath) -> (u8, &[u8]) {
    let encoding = match path.encoding {
        SnapshotNameEncoding::UnixBytes => 0,
        SnapshotNameEncoding::WindowsUtf16LittleEndian => 1,
    };
    (encoding, path.encoded_bytes.as_slice())
}

fn cleanup_exclusions_update(
    update: CoreCleanupExclusionsUpdate,
) -> Result<CleanupExclusionsUpdate, CleanupExclusionsError> {
    Ok(CleanupExclusionsUpdate {
        record_version: FFI_RECORD_VERSION,
        exclusions: cleanup_exclusions_status(update.exclusions)?,
        changed: update.changed,
    })
}

fn cleanup_exclusions_time_ms(value: SystemTime) -> Result<i64, CleanupExclusionsError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CleanupExclusionsError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| CleanupExclusionsError::InternalState)
}

fn decode_cleanup_exclusion_input(
    input: CleanupExclusionsInput,
) -> Result<Vec<PathBuf>, CleanupExclusionsError> {
    if input.record_version != FFI_RECORD_VERSION {
        return Err(CleanupExclusionsError::InvalidRecordVersion);
    }
    if input.paths.len() > MAX_CLEANUP_EXCLUSION_COUNT {
        return Err(CleanupExclusionsError::TooManyPaths);
    }
    input
        .paths
        .into_iter()
        .map(decode_cleanup_exclusion_path)
        .collect()
}

fn decode_cleanup_exclusion_path(
    encoded: CleanupExclusionPath,
) -> Result<PathBuf, CleanupExclusionsError> {
    if encoded.encoded_bytes.is_empty()
        || encoded.encoded_bytes.len() > MAX_CLEANUP_EXCLUSION_PATH_BYTES
    {
        return Err(CleanupExclusionsError::InvalidPath);
    }
    let path = match encoded.encoding {
        SnapshotNameEncoding::UnixBytes => {
            #[cfg(unix)]
            {
                if encoded.encoded_bytes.contains(&0)
                    || std::str::from_utf8(&encoded.encoded_bytes).is_err()
                {
                    return Err(CleanupExclusionsError::InvalidPath);
                }
                PathBuf::from(std::ffi::OsString::from_vec(encoded.encoded_bytes))
            }
            #[cfg(not(unix))]
            {
                return Err(CleanupExclusionsError::InvalidPath);
            }
        }
        SnapshotNameEncoding::WindowsUtf16LittleEndian => {
            #[cfg(windows)]
            {
                if encoded.encoded_bytes.len() % 2 != 0 {
                    return Err(CleanupExclusionsError::InvalidPath);
                }
                let units = encoded
                    .encoded_bytes
                    .chunks_exact(2)
                    .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                    .collect::<Vec<_>>();
                if units.contains(&0) || String::from_utf16(&units).is_err() {
                    return Err(CleanupExclusionsError::InvalidPath);
                }
                PathBuf::from(std::ffi::OsString::from_wide(&units))
            }
            #[cfg(not(windows))]
            {
                return Err(CleanupExclusionsError::InvalidPath);
            }
        }
    };
    if !is_valid_cleanup_exclusion_shape(&path) {
        return Err(CleanupExclusionsError::InvalidPath);
    }
    Ok(path)
}

fn cleanup_exclusion_path(path: &Path) -> Result<CleanupExclusionPath, CleanupExclusionsError> {
    if !is_valid_cleanup_exclusion_shape(path) {
        return Err(CleanupExclusionsError::InternalState);
    }
    let (encoding, encoded_bytes) = encode_cleanup_exclusion_path(path)?;
    Ok(CleanupExclusionPath {
        encoding,
        encoded_bytes,
    })
}

fn is_valid_cleanup_exclusion_shape(path: &Path) -> bool {
    path.is_absolute()
        && !path.as_os_str().is_empty()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
}

fn encode_cleanup_exclusion_path(
    path: &Path,
) -> Result<(SnapshotNameEncoding, Vec<u8>), CleanupExclusionsError> {
    #[cfg(unix)]
    {
        let bytes = path.as_os_str().as_bytes();
        if bytes.is_empty()
            || bytes.len() > MAX_CLEANUP_EXCLUSION_PATH_BYTES
            || bytes.contains(&0)
            || std::str::from_utf8(bytes).is_err()
        {
            return Err(CleanupExclusionsError::InternalState);
        }
        return Ok((SnapshotNameEncoding::UnixBytes, bytes.to_vec()));
    }
    #[cfg(windows)]
    {
        let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if units.is_empty()
            || units.len().saturating_mul(2) > MAX_CLEANUP_EXCLUSION_PATH_BYTES
            || units.contains(&0)
            || String::from_utf16(&units).is_err()
        {
            return Err(CleanupExclusionsError::InternalState);
        }
        let mut bytes = Vec::with_capacity(units.len().saturating_mul(2));
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        return Ok((SnapshotNameEncoding::WindowsUtf16LittleEndian, bytes));
    }
    #[allow(unreachable_code)]
    Err(CleanupExclusionsError::InternalState)
}

fn decode_direct_cargo_executable_path(
    request: DirectCargoEnrollmentInspectionRequest,
) -> Result<PathBuf, DirectCargoEnrollmentError> {
    if request.record_version != FFI_RECORD_VERSION {
        return Err(DirectCargoEnrollmentError::InvalidRecordVersion);
    }
    if request.executable_path_bytes.is_empty()
        || request.executable_path_bytes.len() > MAX_DIRECT_CARGO_EXECUTABLE_PATH_BYTES
    {
        return Err(DirectCargoEnrollmentError::InvalidExecutablePath);
    }
    let path = match request.path_encoding {
        SnapshotNameEncoding::UnixBytes => {
            #[cfg(unix)]
            {
                if request.executable_path_bytes.contains(&0) {
                    return Err(DirectCargoEnrollmentError::InvalidExecutablePath);
                }
                let text = std::str::from_utf8(&request.executable_path_bytes)
                    .map_err(|_| DirectCargoEnrollmentError::InvalidExecutablePath)?;
                if !is_lexical_direct_cargo_unix_path(text) {
                    return Err(DirectCargoEnrollmentError::InvalidExecutablePath);
                }
                PathBuf::from(std::ffi::OsString::from_vec(request.executable_path_bytes))
            }
            #[cfg(not(unix))]
            {
                return Err(DirectCargoEnrollmentError::InvalidExecutablePath);
            }
        }
        SnapshotNameEncoding::WindowsUtf16LittleEndian => {
            #[cfg(windows)]
            {
                if request.executable_path_bytes.len() % 2 != 0 {
                    return Err(DirectCargoEnrollmentError::InvalidExecutablePath);
                }
                let units = request
                    .executable_path_bytes
                    .chunks_exact(2)
                    .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                    .collect::<Vec<_>>();
                if units.contains(&0) {
                    return Err(DirectCargoEnrollmentError::InvalidExecutablePath);
                }
                PathBuf::from(std::ffi::OsString::from_wide(&units))
            }
            #[cfg(not(windows))]
            {
                return Err(DirectCargoEnrollmentError::InvalidExecutablePath);
            }
        }
    };
    if !path.is_absolute()
        || path.file_name() != Some(std::ffi::OsStr::new("cargo"))
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(DirectCargoEnrollmentError::InvalidExecutablePath);
    }
    Ok(path)
}

fn is_lexical_direct_cargo_unix_path(text: &str) -> bool {
    text.starts_with('/')
        && !text.ends_with('/')
        && !text.contains("//")
        && !text.chars().any(char::is_control)
        && text[1..]
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
        && text.rsplit('/').next() == Some("cargo")
}

fn direct_cargo_executable_path(
    path: &Path,
) -> Result<DirectCargoExecutablePath, DirectCargoEnrollmentError> {
    if !path.is_absolute()
        || path.file_name() != Some(std::ffi::OsStr::new("cargo"))
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(DirectCargoEnrollmentError::InternalState);
    }
    #[cfg(unix)]
    {
        let bytes = path.as_os_str().as_bytes();
        let text =
            std::str::from_utf8(bytes).map_err(|_| DirectCargoEnrollmentError::InternalState)?;
        if bytes.is_empty()
            || bytes.len() > MAX_DIRECT_CARGO_EXECUTABLE_PATH_BYTES
            || bytes.contains(&0)
            || !is_lexical_direct_cargo_unix_path(text)
        {
            return Err(DirectCargoEnrollmentError::InternalState);
        }
        return Ok(DirectCargoExecutablePath {
            encoding: SnapshotNameEncoding::UnixBytes,
            encoded_bytes: bytes.to_vec(),
        });
    }
    #[cfg(windows)]
    {
        let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if units.is_empty()
            || units.len().saturating_mul(2) > MAX_DIRECT_CARGO_EXECUTABLE_PATH_BYTES
            || units.contains(&0)
        {
            return Err(DirectCargoEnrollmentError::InternalState);
        }
        let mut encoded_bytes = Vec::new();
        encoded_bytes
            .try_reserve_exact(units.len().saturating_mul(2))
            .map_err(|_| DirectCargoEnrollmentError::InternalState)?;
        for unit in units {
            encoded_bytes.extend_from_slice(&unit.to_le_bytes());
        }
        return Ok(DirectCargoExecutablePath {
            encoding: SnapshotNameEncoding::WindowsUtf16LittleEndian,
            encoded_bytes,
        });
    }
    #[allow(unreachable_code)]
    Err(DirectCargoEnrollmentError::InternalState)
}

fn direct_cargo_code_signature(
    signature: &CoreDirectCargoCodeSignature,
) -> Result<DirectCargoCodeSignature, DirectCargoEnrollmentError> {
    if !(1..=MAX_DIRECT_CARGO_CODE_DIRECTORY_HASHES)
        .contains(&signature.code_directory_hashes.len())
        || signature.code_directory_hashes.iter().any(|hash| {
            !(MIN_DIRECT_CARGO_CODE_DIRECTORY_HASH_BYTES
                ..=MAX_DIRECT_CARGO_CODE_DIRECTORY_HASH_BYTES)
                .contains(&hash.len())
        })
        || !signature
            .code_directory_hashes
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        || signature.signing_identifier.is_empty()
        || signature.signing_identifier.len() > MAX_DIRECT_CARGO_SIGNING_IDENTIFIER_BYTES
        || signature.signing_identifier.chars().any(char::is_control)
        || signature
            .team_identifier
            .as_ref()
            .is_some_and(|identifier| {
                identifier.is_empty()
                    || identifier.len() > MAX_DIRECT_CARGO_TEAM_IDENTIFIER_BYTES
                    || identifier.chars().any(char::is_control)
            })
    {
        return Err(DirectCargoEnrollmentError::InternalState);
    }
    Ok(DirectCargoCodeSignature {
        record_version: FFI_RECORD_VERSION,
        class: match signature.class {
            CoreDirectCargoSignatureClass::AdHoc => DirectCargoSignatureClass::AdHoc,
            CoreDirectCargoSignatureClass::Cms => DirectCargoSignatureClass::Cms,
        },
        flags: signature.flags,
        code_directory_hashes: signature
            .code_directory_hashes
            .iter()
            .map(|hash| DirectCargoCodeDirectoryHash {
                record_version: FFI_RECORD_VERSION,
                bytes: hash.clone(),
            })
            .collect(),
        signing_identifier: signature.signing_identifier.clone(),
        team_identifier: signature.team_identifier.clone(),
        designated_requirement_sha256: signature
            .designated_requirement_sha256
            .map(|digest| digest.to_vec()),
    })
}

fn direct_cargo_preview_info(
    preview: &CoreDirectCargoEnrollmentPreview,
) -> Result<DirectCargoEnrollmentPreviewInfo, DirectCargoEnrollmentError> {
    Ok(DirectCargoEnrollmentPreviewInfo {
        record_version: FFI_RECORD_VERSION,
        executable_path: direct_cargo_executable_path(preview.path())?,
        executable_sha256: preview.executable_sha256().to_vec(),
        code_signature: direct_cargo_code_signature(preview.code_signature())?,
    })
}

fn direct_cargo_enrollment_status(
    status: CoreDirectCargoEnrollmentStatus,
) -> Result<DirectCargoEnrollmentStatus, DirectCargoEnrollmentError> {
    let updated_at_unix_ms = status
        .updated_at
        .map(direct_cargo_enrollment_time_ms)
        .transpose()?;
    let (state, identity) = match status.state {
        CoreDirectCargoEnrollmentState::NotEnrolled => {
            (DirectCargoEnrollmentState::NotEnrolled, None)
        }
        CoreDirectCargoEnrollmentState::Revoked => (DirectCargoEnrollmentState::Revoked, None),
        CoreDirectCargoEnrollmentState::Enrolled {
            path,
            executable_sha256,
            version_sha256,
            cargo_release,
            code_signature,
        } => {
            if cargo_release != DIRECT_CARGO_SUPPORTED_RELEASE {
                return Err(DirectCargoEnrollmentError::InternalState);
            }
            (
                DirectCargoEnrollmentState::Enrolled,
                Some(DirectCargoEnrollmentIdentity {
                    record_version: FFI_RECORD_VERSION,
                    executable_path: direct_cargo_executable_path(&path)?,
                    executable_sha256: executable_sha256.to_vec(),
                    version_sha256: version_sha256.to_vec(),
                    cargo_major: cargo_release[0],
                    cargo_minor: cargo_release[1],
                    cargo_patch: cargo_release[2],
                    code_signature: direct_cargo_code_signature(&code_signature)?,
                }),
            )
        }
    };
    let valid_shape = match state {
        DirectCargoEnrollmentState::NotEnrolled => {
            status.revision == 0 && identity.is_none() && updated_at_unix_ms.is_none()
        }
        DirectCargoEnrollmentState::Enrolled => {
            status.revision > 0 && identity.is_some() && updated_at_unix_ms.is_some()
        }
        DirectCargoEnrollmentState::Revoked => {
            status.revision > 0 && identity.is_none() && updated_at_unix_ms.is_some()
        }
    };
    if !valid_shape {
        return Err(DirectCargoEnrollmentError::InternalState);
    }
    Ok(DirectCargoEnrollmentStatus {
        record_version: FFI_RECORD_VERSION,
        revision: status.revision,
        state,
        identity,
        updated_at_unix_ms,
    })
}

fn direct_cargo_enrollment_update(
    update: CoreDirectCargoEnrollmentUpdate,
) -> Result<DirectCargoEnrollmentUpdate, DirectCargoEnrollmentError> {
    Ok(DirectCargoEnrollmentUpdate {
        record_version: FFI_RECORD_VERSION,
        status: direct_cargo_enrollment_status(update.status)?,
        changed: update.changed,
    })
}

/// Project a mutating core result without claiming that a successful mutation
/// failed safely. Once core returns an update, its durable outcome may already
/// be visible even if the transport shape cannot be represented.
fn direct_cargo_mutation_result(
    result: Result<CoreDirectCargoEnrollmentUpdate, CoreDirectCargoEnrollmentError>,
) -> Result<DirectCargoEnrollmentUpdate, DirectCargoEnrollmentError> {
    let update = result.map_err(map_direct_cargo_enrollment_error)?;
    direct_cargo_enrollment_update(update).map_err(|_| DirectCargoEnrollmentError::OutcomeUnknown)
}

fn direct_cargo_enrollment_time_ms(value: SystemTime) -> Result<i64, DirectCargoEnrollmentError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| DirectCargoEnrollmentError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| DirectCargoEnrollmentError::InternalState)
}

const fn map_direct_cargo_enrollment_error(
    error: CoreDirectCargoEnrollmentError,
) -> DirectCargoEnrollmentError {
    match error {
        CoreDirectCargoEnrollmentError::Closed => DirectCargoEnrollmentError::Closed,
        CoreDirectCargoEnrollmentError::UnsupportedPlatform => {
            DirectCargoEnrollmentError::UnsupportedPlatform
        }
        CoreDirectCargoEnrollmentError::InvalidExecutableLocator => {
            DirectCargoEnrollmentError::InvalidExecutablePath
        }
        CoreDirectCargoEnrollmentError::ExecutableNotRegular => {
            DirectCargoEnrollmentError::ExecutableNotRegular
        }
        CoreDirectCargoEnrollmentError::ChangedDuringInspection => {
            DirectCargoEnrollmentError::ChangedDuringInspection
        }
        CoreDirectCargoEnrollmentError::InspectionUnavailable => {
            DirectCargoEnrollmentError::InspectionUnavailable
        }
        CoreDirectCargoEnrollmentError::InspectionLimitExceeded => {
            DirectCargoEnrollmentError::InspectionLimitExceeded
        }
        CoreDirectCargoEnrollmentError::InvalidResolutionEnvironment => {
            DirectCargoEnrollmentError::InvalidResolutionEnvironment
        }
        CoreDirectCargoEnrollmentError::InvalidCargoVersion => {
            DirectCargoEnrollmentError::InvalidCargoVersion
        }
        CoreDirectCargoEnrollmentError::InvalidCodeSignature => {
            DirectCargoEnrollmentError::InvalidCodeSignature
        }
        CoreDirectCargoEnrollmentError::WrongEngine => DirectCargoEnrollmentError::WrongEngine,
        CoreDirectCargoEnrollmentError::RevisionExhausted => {
            DirectCargoEnrollmentError::RevisionExhausted
        }
        CoreDirectCargoEnrollmentError::InvalidClock => DirectCargoEnrollmentError::InvalidClock,
        CoreDirectCargoEnrollmentError::IncompatibleSchema => {
            DirectCargoEnrollmentError::IncompatibleSchema
        }
        CoreDirectCargoEnrollmentError::Busy => DirectCargoEnrollmentError::Busy,
        CoreDirectCargoEnrollmentError::UnsafeStorage => DirectCargoEnrollmentError::UnsafeStorage,
        CoreDirectCargoEnrollmentError::QueryLimitExceeded => {
            DirectCargoEnrollmentError::BudgetExceeded
        }
        CoreDirectCargoEnrollmentError::CorruptData => DirectCargoEnrollmentError::CorruptData,
        CoreDirectCargoEnrollmentError::Unavailable => DirectCargoEnrollmentError::Unavailable,
        CoreDirectCargoEnrollmentError::OutcomeUnknown => {
            DirectCargoEnrollmentError::OutcomeUnknown
        }
        CoreDirectCargoEnrollmentError::InternalState => DirectCargoEnrollmentError::InternalState,
        _ => DirectCargoEnrollmentError::InternalState,
    }
}

fn permanent_cleanup_policy_status(
    policy: CorePermanentCleanupPolicy,
) -> Result<PermanentCleanupPolicyStatus, PermanentCleanupPolicyError> {
    let updated_at_unix_ms = policy
        .updated_at
        .map(permanent_cleanup_policy_time_ms)
        .transpose()?;
    if (policy.revision == 0) != updated_at_unix_ms.is_none()
        || (policy.revision == 0 && policy.source != CorePermanentCleanupPolicySource::Default)
    {
        return Err(PermanentCleanupPolicyError::InternalState);
    }
    Ok(PermanentCleanupPolicyStatus {
        record_version: FFI_RECORD_VERSION,
        enabled: policy.enabled,
        source: match policy.source {
            CorePermanentCleanupPolicySource::Default => PermanentCleanupPolicySource::Default,
            CorePermanentCleanupPolicySource::Stored => PermanentCleanupPolicySource::Stored,
        },
        revision: policy.revision,
        updated_at_unix_ms,
    })
}

fn permanent_cleanup_policy_update(
    update: CorePermanentCleanupPolicyUpdate,
) -> Result<PermanentCleanupPolicyUpdate, PermanentCleanupPolicyError> {
    Ok(PermanentCleanupPolicyUpdate {
        record_version: FFI_RECORD_VERSION,
        policy: permanent_cleanup_policy_status(update.policy)?,
        changed: update.changed,
    })
}

fn permanent_cleanup_policy_time_ms(value: SystemTime) -> Result<i64, PermanentCleanupPolicyError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PermanentCleanupPolicyError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| PermanentCleanupPolicyError::InternalState)
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
    use std::sync::{Mutex, TryLockError};
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

    fn wait_until(description: &str, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {description}"
            );
            std::thread::yield_now();
        }
    }

    fn mutex_is_locked<T>(mutex: &Mutex<T>) -> bool {
        match mutex.try_lock() {
            Ok(_) => false,
            Err(TryLockError::WouldBlock) => true,
            Err(TryLockError::Poisoned(_)) => panic!("test mutex was poisoned"),
        }
    }

    #[test]
    fn reports_contract_twenty_nine_and_preserves_legacy_formatting() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        assert_eq!(library_version().ffi_contract_version, 29);
        assert_eq!(engine.library_version().unwrap(), library_version());
        assert_eq!(engine.format_size(1536).unwrap().display, "1.5 KB");
        assert!(engine.close());
        assert_eq!(engine.format_size(1), Err(EngineError::Closed));
    }

    #[cfg(unix)]
    #[test]
    fn rust_target_plan_review_path_preserves_exact_bytes_and_escapes_unsafe_display() {
        let path = PathBuf::from(std::ffi::OsString::from_vec(
            b"/tmp/non-utf8-\xff\\\n".to_vec(),
        ));
        let projected = project_rust_target_plan_review_path(&path).unwrap();
        assert_eq!(projected.encoding, SnapshotNameEncoding::UnixBytes);
        assert_eq!(projected.encoded_bytes, b"/tmp/non-utf8-\xff\\\n");
        assert_eq!(projected.display, "unix-bytes:/tmp/non-utf8-\\xff\\\\\\x0a");
        assert!(!projected.display.chars().any(char::is_control));

        let controlled =
            project_rust_target_plan_review_path(Path::new("/tmp/valid-utf8-\ttarget")).unwrap();
        assert_eq!(controlled.display, "unix-bytes:/tmp/valid-utf8-\\x09target");
        assert!(!controlled.display.chars().any(char::is_control));

        let bidi =
            project_rust_target_plan_review_path(Path::new("/tmp/bidi-\u{202e}target")).unwrap();
        assert_eq!(bidi.display, "unix-bytes:/tmp/bidi-\\xe2\\x80\\xaetarget");
        assert!(!bidi.display.contains('\u{202e}'));

        let arabic_mark =
            project_rust_target_plan_review_path(Path::new("/tmp/mark-\u{061c}target")).unwrap();
        assert_eq!(arabic_mark.display, "unix-bytes:/tmp/mark-\\xd8\\x9ctarget");
        assert!(!arabic_mark.display.contains('\u{061c}'));
        for unsafe_scalar in ['\u{00ad}', '\u{034f}', '\u{180e}', '\u{180f}', '\u{fe0f}'] {
            assert!(is_unsafe_plan_review_display_scalar(unsafe_scalar));
        }
        assert!(!is_unsafe_plan_review_display_scalar('å'));

        let plain = project_rust_target_plan_review_path(Path::new("/tmp/plain target")).unwrap();
        assert_eq!(plain.display, "/tmp/plain target");
    }

    #[test]
    fn rust_target_plan_preparation_tracker_enforces_one_inflight_reservation() {
        let tracker = Arc::new(RustTargetPlanPreparationTracker::default());
        let closed = AtomicBool::new(false);
        let first = tracker.enter_preparation(&closed).unwrap();
        assert!(matches!(
            tracker.enter_preparation(&closed),
            Err(RustTargetPlanReviewError::ReviewBusy)
        ));
        let observation = tracker.enter_operation(&closed).unwrap();
        drop(observation);
        drop(first);
        let second = tracker.enter_preparation(&closed).unwrap();
        drop(second);
    }

    #[test]
    fn rust_target_plan_operation_tracker_keeps_shutdown_wait_bounded() {
        let tracker = Arc::new(RustTargetPlanPreparationTracker::default());
        let closed = AtomicBool::new(false);
        let operation = tracker.enter_operation(&closed).unwrap();
        let started = Instant::now();
        assert!(!tracker.wait_until(started + Duration::from_millis(10)));
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(operation);
        assert!(tracker.wait_until(Instant::now() + Duration::from_secs(1)));
    }

    #[test]
    fn engine_close_returns_within_bound_when_state_admission_is_blocked() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let engine = Arc::new(engine);
        let state_guard = engine.state.lock().unwrap();
        let started = Instant::now();
        let closing = {
            let engine = Arc::clone(&engine);
            std::thread::spawn(move || engine.close())
        };
        wait_until("blocked close admission", || {
            engine.closed.load(Ordering::Acquire)
        });
        assert!(!closing.join().unwrap());
        assert!(started.elapsed() < Duration::from_secs(6));
        drop(state_guard);
        wait_until("background engine close", || {
            matches!(*engine.state.lock().unwrap(), EngineState::Closed { .. })
        });
        assert_eq!(engine.format_size(1), Err(EngineError::Closed));
    }

    #[test]
    fn closed_admission_bit_rejects_every_shared_engine_helper_while_state_is_open() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        engine.closed.store(true, Ordering::Release);
        assert_eq!(engine.with_engine(|_| Ok(())), Err(EngineError::Closed));
        assert_eq!(
            engine.with_pressure_engine(|_| Ok(())),
            Err(PressurePolicyError::Closed)
        );
        assert_eq!(
            engine.with_permanent_cleanup_engine(|_| Ok(())),
            Err(PermanentCleanupPolicyError::Closed)
        );
        assert_eq!(
            engine.with_cleanup_exclusions_engine(|_| Ok(())),
            Err(CleanupExclusionsError::Closed)
        );
        assert_eq!(
            engine.with_cleanup_history_engine(|_| Ok(())),
            Err(CleanupHistoryError::Closed)
        );
        assert_eq!(
            engine.with_direct_cargo_engine(|_| Ok(())),
            Err(DirectCargoEnrollmentError::Closed)
        );
        assert_eq!(engine.with_scan_engine(|_| Ok(())), Err(ScanError::Closed));
    }

    #[test]
    fn rust_target_plan_release_does_not_wait_for_inflight_info() {
        let tracker = Arc::new(RustTargetPlanPreparationTracker::default());
        let session = RustTargetPlanReviewSession {
            state: Mutex::new(RustTargetPlanReviewState::Inspecting),
            parent_review: Weak::new(),
            operations: tracker,
            engine_closed: Arc::new(AtomicBool::new(false)),
        };
        assert_eq!(
            session.release().unwrap(),
            RustTargetPlanReviewReleaseOutcome::Released
        );
        assert!(matches!(
            *session.state.lock().unwrap(),
            RustTargetPlanReviewState::ReleasePending
        ));
        assert_eq!(
            session.release().unwrap(),
            RustTargetPlanReviewReleaseOutcome::AlreadyUnavailable
        );
    }

    #[test]
    fn rust_target_plan_capacity_reaps_a_retained_released_parent() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("review-capacity-root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("payload"), b"review capacity").unwrap();
        let task = engine
            .with_engine(|core| core.start_scan(root).map_err(map_start_error))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let scan_id = loop {
            let result = engine
                .with_engine(|core| {
                    if core
                        .task_snapshot(task)
                        .map_err(map_task_access_error)?
                        .phase
                        == CoreTaskPhase::Succeeded
                    {
                        Ok(core
                            .scan_result(task)
                            .map_err(map_task_access_error)?
                            .map(|result| result.scan_id().as_str().to_owned()))
                    } else {
                        Ok(None)
                    }
                })
                .unwrap();
            if let Some(scan_id) = result {
                break scan_id;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        };
        let parent = engine.acquire_explorer_snapshot_review(scan_id).unwrap();
        let child = Arc::new(RustTargetPlanReviewSession {
            state: Mutex::new(RustTargetPlanReviewState::Inspecting),
            parent_review: Arc::downgrade(&parent),
            operations: Arc::clone(&engine.rust_target_plan_preparations),
            engine_closed: Arc::clone(&engine.closed),
        });
        engine
            .rust_target_plan_reviews
            .lock()
            .unwrap()
            .push(Arc::downgrade(&child));

        assert_eq!(parent.release().unwrap(), ReviewReleaseOutcome::Released);
        assert!(engine.ensure_rust_target_plan_review_capacity().is_ok());
        assert!(matches!(
            *child.state.lock().unwrap(),
            RustTargetPlanReviewState::ReleasePending
        ));
        assert!(engine.close());
    }

    #[test]
    fn parent_review_registry_uses_one_bounded_cleanup_task() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("many-review-root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("payload"), b"many reviews").unwrap();
        let terminal = wait_for_scan(&engine.start_scan(scan_request(&root)).unwrap().task);
        let scan_id = terminal.result.unwrap().scan_id;
        let parents = (0..4)
            .map(|_| {
                engine
                    .acquire_explorer_snapshot_review(scan_id.clone())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let first_parent_guard = parents[0].inner.lock().unwrap();

        engine.release_registered_reviews();
        assert_eq!(
            engine
                .rust_target_plan_preparations
                .state
                .lock()
                .unwrap()
                .active,
            1
        );
        assert!(engine.reviews.lock().unwrap().is_empty());
        drop(first_parent_guard);
        assert!(
            engine
                .rust_target_plan_preparations
                .wait_until(Instant::now() + Duration::from_secs(5))
        );
        assert!(parents.iter().all(|parent| parent.info().unwrap().released));
        assert!(engine.close());
    }

    #[cfg(unix)]
    fn direct_cargo_request(path: &Path) -> DirectCargoEnrollmentInspectionRequest {
        DirectCargoEnrollmentInspectionRequest {
            record_version: FFI_RECORD_VERSION,
            path_encoding: SnapshotNameEncoding::UnixBytes,
            executable_path_bytes: path.as_os_str().as_bytes().to_vec(),
        }
    }

    #[cfg(target_os = "macos")]
    fn direct_toolchain_cargo() -> PathBuf {
        let rustup_home = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".rustup"));
        let toolchain = std::env::var_os("RUSTUP_TOOLCHAIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(format!("stable-{}-apple-darwin", std::env::consts::ARCH))
            });
        let cargo = rustup_home
            .join("toolchains")
            .join(toolchain)
            .join("bin/cargo");
        assert!(
            cargo.is_file(),
            "direct stable Cargo must exist at {cargo:?}"
        );
        cargo
    }

    #[test]
    fn direct_cargo_locator_preserves_valid_bytes_and_rejects_unbounded_or_unsafe_text() {
        let valid = DirectCargoEnrollmentInspectionRequest {
            record_version: FFI_RECORD_VERSION,
            path_encoding: SnapshotNameEncoding::UnixBytes,
            executable_path_bytes: b"/tmp/Cargo \xF0\x9F\xA6\x80/cargo".to_vec(),
        };
        let decoded = decode_direct_cargo_executable_path(valid.clone()).unwrap();
        assert_eq!(direct_cargo_request(&decoded), valid);

        for request in [
            DirectCargoEnrollmentInspectionRequest {
                record_version: FFI_RECORD_VERSION + 1,
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: Vec::new(),
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: vec![b'x'; MAX_DIRECT_CARGO_EXECUTABLE_PATH_BYTES + 1],
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: b"relative/cargo".to_vec(),
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: b"/tmp/cargo\0suffix".to_vec(),
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: b"/tmp/cargo\nsuffix".to_vec(),
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: b"/tmp/\xff/cargo".to_vec(),
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: b"/tmp//cargo".to_vec(),
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: b"/tmp/./cargo".to_vec(),
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: b"/tmp/../cargo".to_vec(),
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: b"/tmp/cargo/".to_vec(),
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: b"/tmp/Cargo".to_vec(),
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                executable_path_bytes: b"/cargo-other".to_vec(),
                ..valid.clone()
            },
            DirectCargoEnrollmentInspectionRequest {
                path_encoding: SnapshotNameEncoding::WindowsUtf16LittleEndian,
                ..valid
            },
        ] {
            let expected = if request.record_version != FFI_RECORD_VERSION {
                DirectCargoEnrollmentError::InvalidRecordVersion
            } else {
                DirectCargoEnrollmentError::InvalidExecutablePath
            };
            assert_eq!(decode_direct_cargo_executable_path(request), Err(expected));
        }
        for path in [
            "/tmp//cargo",
            "/tmp/./cargo",
            "/tmp/../cargo",
            "/tmp/cargo/",
            "/tmp/Cargo",
            "/cargo-other",
        ] {
            assert_eq!(
                direct_cargo_executable_path(Path::new(path)),
                Err(DirectCargoEnrollmentError::InternalState)
            );
        }
    }

    #[test]
    fn direct_cargo_status_projection_rejects_malformed_state_and_release_shapes() {
        assert_eq!(
            direct_cargo_enrollment_status(CoreDirectCargoEnrollmentStatus {
                revision: 1,
                state: CoreDirectCargoEnrollmentState::NotEnrolled,
                updated_at: Some(UNIX_EPOCH),
            }),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        assert_eq!(
            direct_cargo_enrollment_status(CoreDirectCargoEnrollmentStatus {
                revision: 0,
                state: CoreDirectCargoEnrollmentState::Revoked,
                updated_at: None,
            }),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        let signature = CoreDirectCargoCodeSignature {
            class: CoreDirectCargoSignatureClass::AdHoc,
            flags: 2,
            code_directory_hashes: vec![vec![1; 20]],
            signing_identifier: "cargo-test".to_owned(),
            team_identifier: None,
            designated_requirement_sha256: None,
        };
        assert_eq!(
            direct_cargo_enrollment_status(CoreDirectCargoEnrollmentStatus {
                revision: 1,
                state: CoreDirectCargoEnrollmentState::Enrolled {
                    path: PathBuf::from("/tmp/cargo"),
                    executable_sha256: [2; 32],
                    version_sha256: [3; 32],
                    cargo_release: [1, 95, 0],
                    code_signature: Box::new(signature.clone()),
                },
                updated_at: Some(UNIX_EPOCH),
            }),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        let enrolled = direct_cargo_enrollment_status(CoreDirectCargoEnrollmentStatus {
            revision: 1,
            state: CoreDirectCargoEnrollmentState::Enrolled {
                path: PathBuf::from("/tmp/cargo"),
                executable_sha256: [2; 32],
                version_sha256: [3; 32],
                cargo_release: DIRECT_CARGO_SUPPORTED_RELEASE,
                code_signature: Box::new(signature),
            },
            updated_at: Some(UNIX_EPOCH),
        })
        .unwrap();
        let identity = enrolled.identity.unwrap();
        assert_eq!(identity.executable_sha256, vec![2; 32]);
        assert_eq!(identity.version_sha256, vec![3; 32]);
        assert_eq!(
            direct_cargo_enrollment_status(CoreDirectCargoEnrollmentStatus {
                revision: 1,
                state: CoreDirectCargoEnrollmentState::Revoked,
                updated_at: Some(UNIX_EPOCH),
            })
            .unwrap(),
            DirectCargoEnrollmentStatus {
                record_version: FFI_RECORD_VERSION,
                revision: 1,
                state: DirectCargoEnrollmentState::Revoked,
                identity: None,
                updated_at_unix_ms: Some(0),
            }
        );
    }

    #[test]
    fn direct_cargo_commit_projection_failure_after_core_success_is_outcome_unknown() {
        let malformed_success = CoreDirectCargoEnrollmentUpdate {
            status: CoreDirectCargoEnrollmentStatus {
                revision: 1,
                state: CoreDirectCargoEnrollmentState::Enrolled {
                    path: PathBuf::from("/tmp/cargo"),
                    executable_sha256: [2; 32],
                    version_sha256: [3; 32],
                    cargo_release: [1, 95, 0],
                    code_signature: Box::new(CoreDirectCargoCodeSignature {
                        class: CoreDirectCargoSignatureClass::AdHoc,
                        flags: 2,
                        code_directory_hashes: vec![vec![1; 20]],
                        signing_identifier: "cargo-test".to_owned(),
                        team_identifier: None,
                        designated_requirement_sha256: None,
                    }),
                },
                updated_at: Some(UNIX_EPOCH),
            },
            changed: true,
        };
        assert_eq!(
            direct_cargo_enrollment_update(malformed_success.clone()),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        assert_eq!(
            direct_cargo_mutation_result(Ok(malformed_success)),
            Err(DirectCargoEnrollmentError::OutcomeUnknown)
        );
        assert_eq!(
            direct_cargo_mutation_result(Err(CoreDirectCargoEnrollmentError::InternalState)),
            Err(DirectCargoEnrollmentError::InternalState)
        );
    }

    #[test]
    fn direct_cargo_revoke_projection_failure_after_core_success_is_outcome_unknown() {
        let malformed_success = CoreDirectCargoEnrollmentUpdate {
            status: CoreDirectCargoEnrollmentStatus {
                revision: 0,
                state: CoreDirectCargoEnrollmentState::Revoked,
                updated_at: Some(UNIX_EPOCH),
            },
            changed: true,
        };
        assert_eq!(
            direct_cargo_enrollment_update(malformed_success.clone()),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        assert_eq!(
            direct_cargo_mutation_result(Ok(malformed_success)),
            Err(DirectCargoEnrollmentError::OutcomeUnknown)
        );
    }

    #[test]
    fn direct_cargo_signature_projection_enforces_independent_transport_bounds() {
        let signature = CoreDirectCargoCodeSignature {
            class: CoreDirectCargoSignatureClass::AdHoc,
            flags: 2,
            code_directory_hashes: (0_u8..15)
                .map(|value| vec![value; MIN_DIRECT_CARGO_CODE_DIRECTORY_HASH_BYTES])
                .chain(std::iter::once(vec![
                    15;
                    MAX_DIRECT_CARGO_CODE_DIRECTORY_HASH_BYTES
                ]))
                .collect(),
            signing_identifier: "s".repeat(MAX_DIRECT_CARGO_SIGNING_IDENTIFIER_BYTES),
            team_identifier: Some("T".repeat(MAX_DIRECT_CARGO_TEAM_IDENTIFIER_BYTES)),
            designated_requirement_sha256: Some([3; 32]),
        };
        let projected = direct_cargo_code_signature(&signature).unwrap();
        assert_eq!(projected.record_version, FFI_RECORD_VERSION);
        assert_eq!(
            projected.code_directory_hashes.len(),
            MAX_DIRECT_CARGO_CODE_DIRECTORY_HASHES
        );
        assert_eq!(
            projected.code_directory_hashes.first().unwrap().bytes.len(),
            MIN_DIRECT_CARGO_CODE_DIRECTORY_HASH_BYTES
        );
        assert_eq!(
            projected.code_directory_hashes.last().unwrap().bytes.len(),
            MAX_DIRECT_CARGO_CODE_DIRECTORY_HASH_BYTES
        );
        assert_eq!(
            projected.signing_identifier.len(),
            MAX_DIRECT_CARGO_SIGNING_IDENTIFIER_BYTES
        );
        assert_eq!(
            projected.team_identifier.as_ref().unwrap().len(),
            MAX_DIRECT_CARGO_TEAM_IDENTIFIER_BYTES
        );
        assert_eq!(projected.designated_requirement_sha256, Some(vec![3; 32]));

        let mut invalid = signature.clone();
        invalid.code_directory_hashes = vec![vec![0; 19]];
        assert_eq!(
            direct_cargo_code_signature(&invalid),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        let mut invalid = signature.clone();
        invalid
            .code_directory_hashes
            .push(vec![u8::MAX; MIN_DIRECT_CARGO_CODE_DIRECTORY_HASH_BYTES]);
        assert_eq!(
            direct_cargo_code_signature(&invalid),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        let mut invalid = signature.clone();
        invalid.code_directory_hashes =
            vec![vec![0; MAX_DIRECT_CARGO_CODE_DIRECTORY_HASH_BYTES + 1]];
        assert_eq!(
            direct_cargo_code_signature(&invalid),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        let mut invalid = signature.clone();
        invalid.code_directory_hashes = vec![vec![2; 20], vec![1; 20]];
        assert_eq!(
            direct_cargo_code_signature(&invalid),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        let mut invalid = signature.clone();
        invalid.signing_identifier = "s".repeat(MAX_DIRECT_CARGO_SIGNING_IDENTIFIER_BYTES + 1);
        assert_eq!(
            direct_cargo_code_signature(&invalid),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        let mut invalid = signature.clone();
        invalid.signing_identifier = "cargo\nunsafe".to_owned();
        assert_eq!(
            direct_cargo_code_signature(&invalid),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        let mut invalid = signature.clone();
        invalid.team_identifier = Some("T".repeat(MAX_DIRECT_CARGO_TEAM_IDENTIFIER_BYTES + 1));
        assert_eq!(
            direct_cargo_code_signature(&invalid),
            Err(DirectCargoEnrollmentError::InternalState)
        );
        let mut invalid = signature;
        invalid.team_identifier = Some("TEAM\u{7f}".to_owned());
        assert_eq!(
            direct_cargo_code_signature(&invalid),
            Err(DirectCargoEnrollmentError::InternalState)
        );
    }

    #[test]
    fn every_direct_cargo_core_error_maps_to_a_stable_transport_error() {
        for (core, ffi) in [
            (
                CoreDirectCargoEnrollmentError::Closed,
                DirectCargoEnrollmentError::Closed,
            ),
            (
                CoreDirectCargoEnrollmentError::UnsupportedPlatform,
                DirectCargoEnrollmentError::UnsupportedPlatform,
            ),
            (
                CoreDirectCargoEnrollmentError::InvalidExecutableLocator,
                DirectCargoEnrollmentError::InvalidExecutablePath,
            ),
            (
                CoreDirectCargoEnrollmentError::ExecutableNotRegular,
                DirectCargoEnrollmentError::ExecutableNotRegular,
            ),
            (
                CoreDirectCargoEnrollmentError::ChangedDuringInspection,
                DirectCargoEnrollmentError::ChangedDuringInspection,
            ),
            (
                CoreDirectCargoEnrollmentError::InspectionUnavailable,
                DirectCargoEnrollmentError::InspectionUnavailable,
            ),
            (
                CoreDirectCargoEnrollmentError::InspectionLimitExceeded,
                DirectCargoEnrollmentError::InspectionLimitExceeded,
            ),
            (
                CoreDirectCargoEnrollmentError::InvalidResolutionEnvironment,
                DirectCargoEnrollmentError::InvalidResolutionEnvironment,
            ),
            (
                CoreDirectCargoEnrollmentError::InvalidCargoVersion,
                DirectCargoEnrollmentError::InvalidCargoVersion,
            ),
            (
                CoreDirectCargoEnrollmentError::InvalidCodeSignature,
                DirectCargoEnrollmentError::InvalidCodeSignature,
            ),
            (
                CoreDirectCargoEnrollmentError::WrongEngine,
                DirectCargoEnrollmentError::WrongEngine,
            ),
            (
                CoreDirectCargoEnrollmentError::RevisionExhausted,
                DirectCargoEnrollmentError::RevisionExhausted,
            ),
            (
                CoreDirectCargoEnrollmentError::InvalidClock,
                DirectCargoEnrollmentError::InvalidClock,
            ),
            (
                CoreDirectCargoEnrollmentError::IncompatibleSchema,
                DirectCargoEnrollmentError::IncompatibleSchema,
            ),
            (
                CoreDirectCargoEnrollmentError::Busy,
                DirectCargoEnrollmentError::Busy,
            ),
            (
                CoreDirectCargoEnrollmentError::UnsafeStorage,
                DirectCargoEnrollmentError::UnsafeStorage,
            ),
            (
                CoreDirectCargoEnrollmentError::QueryLimitExceeded,
                DirectCargoEnrollmentError::BudgetExceeded,
            ),
            (
                CoreDirectCargoEnrollmentError::CorruptData,
                DirectCargoEnrollmentError::CorruptData,
            ),
            (
                CoreDirectCargoEnrollmentError::Unavailable,
                DirectCargoEnrollmentError::Unavailable,
            ),
            (
                CoreDirectCargoEnrollmentError::OutcomeUnknown,
                DirectCargoEnrollmentError::OutcomeUnknown,
            ),
            (
                CoreDirectCargoEnrollmentError::InternalState,
                DirectCargoEnrollmentError::InternalState,
            ),
        ] {
            assert_eq!(map_direct_cargo_enrollment_error(core), ffi);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn direct_cargo_preview_is_bounded_engine_bound_consume_once_and_close_drained() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, primary) = engine();
        let primary = Arc::new(primary);
        let (_foreign_temp, foreign) = engine();
        let same_store = DuxEngine::new(EngineStorageRoots {
            data_root: temp.path().join("data").to_string_lossy().into_owned(),
            cache_root: temp
                .path()
                .join("cache-peer")
                .to_string_lossy()
                .into_owned(),
        })
        .unwrap();
        let cargo = direct_toolchain_cargo();
        assert_eq!(
            primary.direct_cargo_enrollment_status().unwrap(),
            DirectCargoEnrollmentStatus {
                record_version: FFI_RECORD_VERSION,
                revision: 0,
                state: DirectCargoEnrollmentState::NotEnrolled,
                identity: None,
                updated_at_unix_ms: None,
            }
        );

        let released = primary
            .inspect_direct_cargo_enrollment(direct_cargo_request(&cargo))
            .unwrap();
        let info = released.info().unwrap();
        assert_eq!(info.record_version, FFI_RECORD_VERSION);
        assert_eq!(
            info.executable_path.encoded_bytes,
            cargo.as_os_str().as_bytes()
        );
        assert_eq!(info.executable_sha256.len(), 32);
        assert!(!info.code_signature.code_directory_hashes.is_empty());
        assert!(matches!(
            primary.inspect_direct_cargo_enrollment(direct_cargo_request(&cargo)),
            Err(DirectCargoEnrollmentError::Busy)
        ));
        assert_eq!(
            same_store.commit_direct_cargo_enrollment(Arc::clone(&released)),
            Err(DirectCargoEnrollmentError::WrongEngine)
        );
        assert_eq!(
            foreign.commit_direct_cargo_enrollment(Arc::clone(&released)),
            Err(DirectCargoEnrollmentError::WrongEngine)
        );
        assert_eq!(
            released.release().unwrap(),
            DirectCargoEnrollmentPreviewReleaseOutcome::Released
        );
        assert_eq!(
            released.release().unwrap(),
            DirectCargoEnrollmentPreviewReleaseOutcome::AlreadyUnavailable
        );
        assert_eq!(
            released.info(),
            Err(DirectCargoEnrollmentError::PreviewUnavailable)
        );

        let stale = primary
            .inspect_direct_cargo_enrollment(direct_cargo_request(&cargo))
            .unwrap();
        let peer_preview = same_store
            .inspect_direct_cargo_enrollment(direct_cargo_request(&cargo))
            .unwrap();
        let peer_update = same_store
            .commit_direct_cargo_enrollment(peer_preview)
            .unwrap();
        assert!(peer_update.changed);
        assert_eq!(
            primary.commit_direct_cargo_enrollment(Arc::clone(&stale)),
            Err(DirectCargoEnrollmentError::ChangedDuringInspection)
        );
        assert_eq!(
            primary.commit_direct_cargo_enrollment(stale),
            Err(DirectCargoEnrollmentError::PreviewUnavailable)
        );
        let revoked = primary.revoke_direct_cargo_enrollment().unwrap();
        assert!(revoked.changed);
        assert_eq!(revoked.status.state, DirectCargoEnrollmentState::Revoked);

        let committed_preview = primary
            .inspect_direct_cargo_enrollment(direct_cargo_request(&cargo))
            .unwrap();
        let committed = primary
            .commit_direct_cargo_enrollment(Arc::clone(&committed_preview))
            .unwrap();
        assert!(committed.changed);
        assert_eq!(committed.status.state, DirectCargoEnrollmentState::Enrolled);
        let identity = committed.status.identity.as_ref().unwrap();
        assert_eq!(
            identity.executable_path.encoded_bytes,
            cargo.as_os_str().as_bytes()
        );
        assert_eq!(identity.executable_sha256.len(), 32);
        assert_eq!(identity.version_sha256.len(), 32);
        assert_eq!(
            primary.commit_direct_cargo_enrollment(committed_preview),
            Err(DirectCargoEnrollmentError::PreviewUnavailable)
        );
        assert_eq!(
            primary.direct_cargo_enrollment_status().unwrap(),
            committed.status
        );

        let raced = primary
            .inspect_direct_cargo_enrollment(direct_cargo_request(&cargo))
            .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let commit_thread = {
            let primary = Arc::clone(&primary);
            let raced = Arc::clone(&raced);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                primary.commit_direct_cargo_enrollment(raced)
            })
        };
        let release_thread = {
            let raced = Arc::clone(&raced);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                raced.release()
            })
        };
        barrier.wait();
        let commit_result = commit_thread.join().unwrap();
        let release_result = release_thread.join().unwrap().unwrap();
        match commit_result {
            Ok(_) => assert_eq!(
                release_result,
                DirectCargoEnrollmentPreviewReleaseOutcome::AlreadyUnavailable
            ),
            Err(DirectCargoEnrollmentError::PreviewUnavailable) => assert_eq!(
                release_result,
                DirectCargoEnrollmentPreviewReleaseOutcome::Released
            ),
            other => panic!("unexpected commit/release race result: {other:?}"),
        }
        assert_eq!(
            raced.info(),
            Err(DirectCargoEnrollmentError::PreviewUnavailable)
        );

        let close_drained = primary
            .inspect_direct_cargo_enrollment(direct_cargo_request(&cargo))
            .unwrap();
        assert!(primary.close());
        assert_eq!(
            close_drained.info(),
            Err(DirectCargoEnrollmentError::Closed)
        );
        assert_eq!(
            close_drained.release().unwrap(),
            DirectCargoEnrollmentPreviewReleaseOutcome::AlreadyUnavailable
        );
        assert_eq!(
            primary.commit_direct_cargo_enrollment(close_drained),
            Err(DirectCargoEnrollmentError::Closed)
        );
        assert_eq!(
            primary.direct_cargo_enrollment_status(),
            Err(DirectCargoEnrollmentError::Closed)
        );
        assert!(foreign.close());
        assert!(same_store.close());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn direct_cargo_close_wins_inspection_registration_and_commit_admission() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let cargo = direct_toolchain_cargo();

        let (_inspect_temp, inspect_engine) = engine();
        let inspect_engine = Arc::new(inspect_engine);
        let registry_guard = inspect_engine.direct_cargo_previews.lock().unwrap();
        let inspect_thread = {
            let inspect_engine = Arc::clone(&inspect_engine);
            let cargo = cargo.clone();
            std::thread::spawn(move || {
                inspect_engine.inspect_direct_cargo_enrollment(direct_cargo_request(&cargo))
            })
        };
        wait_until("inspection to acquire the engine state lock", || {
            mutex_is_locked(&inspect_engine.state)
        });
        let inspect_close_thread = {
            let inspect_engine = Arc::clone(&inspect_engine);
            std::thread::spawn(move || inspect_engine.close())
        };
        wait_until("inspection engine close admission", || {
            inspect_engine.closed.load(Ordering::Acquire)
        });
        drop(registry_guard);
        assert!(matches!(
            inspect_thread.join().unwrap(),
            Err(DirectCargoEnrollmentError::Closed)
        ));
        let inspect_close_result = inspect_close_thread.join().unwrap();
        if !inspect_close_result {
            wait_until("timed-out inspection close to finish in background", || {
                matches!(
                    *inspect_engine.state.lock().unwrap(),
                    EngineState::Closed { .. }
                )
            });
        }
        assert!(
            inspect_engine
                .direct_cargo_previews
                .lock()
                .unwrap()
                .iter()
                .all(|preview| preview.upgrade().is_none())
        );

        let (_commit_temp, commit_engine) = engine();
        let commit_engine = Arc::new(commit_engine);
        let preview = commit_engine
            .inspect_direct_cargo_enrollment(direct_cargo_request(&cargo))
            .unwrap();
        let state_guard = commit_engine.state.lock().unwrap();
        let commit_close_thread = {
            let commit_engine = Arc::clone(&commit_engine);
            std::thread::spawn(move || commit_engine.close())
        };
        wait_until("commit engine close admission", || {
            commit_engine.closed.load(Ordering::Acquire)
        });
        let commit_thread = {
            let commit_engine = Arc::clone(&commit_engine);
            let preview = Arc::clone(&preview);
            std::thread::spawn(move || commit_engine.commit_direct_cargo_enrollment(preview))
        };
        drop(state_guard);
        assert_eq!(
            commit_thread.join().unwrap(),
            Err(DirectCargoEnrollmentError::Closed)
        );
        assert!(commit_close_thread.join().unwrap());
        assert_eq!(preview.info(), Err(DirectCargoEnrollmentError::Closed));
        assert_eq!(
            preview.release().unwrap(),
            DirectCargoEnrollmentPreviewReleaseOutcome::AlreadyUnavailable
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn direct_cargo_concurrent_commit_and_revoke_converge_to_revoked() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let engine = Arc::new(engine);
        let cargo = direct_toolchain_cargo();
        let initial = engine
            .inspect_direct_cargo_enrollment(direct_cargo_request(&cargo))
            .and_then(|preview| engine.commit_direct_cargo_enrollment(preview))
            .unwrap();
        assert_eq!(initial.status.state, DirectCargoEnrollmentState::Enrolled);
        let preview = engine
            .inspect_direct_cargo_enrollment(direct_cargo_request(&cargo))
            .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let commit_thread = {
            let engine = Arc::clone(&engine);
            let preview = Arc::clone(&preview);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                engine.commit_direct_cargo_enrollment(preview)
            })
        };
        let revoke_thread = {
            let engine = Arc::clone(&engine);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                engine.revoke_direct_cargo_enrollment()
            })
        };
        barrier.wait();
        let commit_result = commit_thread.join().unwrap();
        let revoke = revoke_thread.join().unwrap().unwrap();
        match commit_result {
            Ok(update) => {
                assert_eq!(update.status.state, DirectCargoEnrollmentState::Enrolled);
            }
            Err(DirectCargoEnrollmentError::ChangedDuringInspection) => {}
            other => panic!("unexpected concurrent commit result: {other:?}"),
        }
        assert!(revoke.changed);
        assert_eq!(revoke.status.state, DirectCargoEnrollmentState::Revoked);
        assert_eq!(
            engine.direct_cargo_enrollment_status().unwrap(),
            revoke.status
        );
        assert_eq!(
            preview.info(),
            Err(DirectCargoEnrollmentError::PreviewUnavailable)
        );
        assert_eq!(
            engine.commit_direct_cargo_enrollment(preview),
            Err(DirectCargoEnrollmentError::PreviewUnavailable)
        );
        assert!(engine.close());
    }

    #[test]
    fn candidate_review_intent_maps_without_effect_authority() {
        assert_eq!(
            map_candidate_review_command(CandidateReviewCommand::Select),
            CoreCandidateReviewCommand::Select
        );
        assert_eq!(
            map_candidate_review_command(CandidateReviewCommand::ClearSelection),
            CoreCandidateReviewCommand::ClearSelection
        );
        assert_eq!(
            map_candidate_review_command(CandidateReviewCommand::Dismiss),
            CoreCandidateReviewCommand::Dismiss
        );
        assert_eq!(
            map_candidate_review_command(CandidateReviewCommand::Restore),
            CoreCandidateReviewCommand::Restore
        );
        assert_eq!(
            map_candidate_review_error(CoreCandidateReviewError::NotReviewable),
            EngineError::CandidateReviewNotReviewable
        );
    }

    #[test]
    fn cleanup_history_summary_feed_is_bounded_path_free_and_cursor_validated() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let page = engine.recent_cleanup_history(None, 64).unwrap();
        assert_eq!(page.record_version, FFI_RECORD_VERSION);
        assert!(page.records.is_empty());
        assert!(page.next_cursor.is_none());
        assert_eq!(
            engine.recent_cleanup_history(None, 0),
            Err(CleanupHistoryError::InvalidLimit)
        );
        assert_eq!(
            engine.recent_cleanup_history(
                Some(CleanupHistoryCursor {
                    record_version: FFI_RECORD_VERSION,
                    started_at_unix_ms: -1,
                    session_id: "session:invalid".to_owned(),
                }),
                1
            ),
            Err(CleanupHistoryError::InvalidCursor)
        );
        assert!(engine.close());
        assert_eq!(
            engine.recent_cleanup_history(None, 1),
            Err(CleanupHistoryError::Closed)
        );
    }

    #[test]
    fn cleanup_session_history_is_exact_bounded_path_free_and_strictly_validated() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("cleanup-history-detail");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("reviewed.bin"), b"reviewed").unwrap();

        let scan = engine.start_scan(scan_request(&root)).unwrap();
        let terminal = wait_for_scan(&scan.task);
        assert_eq!(terminal.phase, TaskPhase::Succeeded);
        let scan_id = terminal.result.unwrap().scan_id;
        let review = engine.acquire_explorer_snapshot_review(scan_id).unwrap();
        let root_node = review.root_node().unwrap();
        let children = review
            .child_nodes(root_node.id, SnapshotNodeSort::NameAscending, 0, 10)
            .unwrap();
        let selected = children
            .nodes
            .iter()
            .find(|node| node.name.display == "reviewed.bin")
            .unwrap();
        let result = engine
            .execute_explorer_trash(
                Arc::clone(&review),
                selected.id,
                Box::new(RecordingTrashDriver {
                    calls: Mutex::new(Vec::new()),
                }),
            )
            .unwrap();
        assert_eq!(result, TrashPlatformResult::Completed);

        let summary_page = engine.recent_cleanup_history(None, 1).unwrap();
        let summary = summary_page.records.first().unwrap().clone();
        let history = engine
            .cleanup_session_history(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                session_id: summary.session_id.clone(),
            })
            .unwrap();
        assert_eq!(history.record_version, FFI_RECORD_VERSION);
        assert_eq!(history.summary, summary);
        assert_eq!(history.items.len(), 1);
        assert_eq!(history.items[0].ordinal, 0);
        assert_eq!(history.items[0].status, CleanupItemStatus::Trashed);
        assert_eq!(history.items[0].path_count, 1);
        assert_eq!(
            history.warnings,
            vec![
                CleanupWarning::EstimatedBytesUnverified,
                CleanupWarning::TrashDoesNotFreeSpaceImmediately,
            ]
        );
        assert_eq!(history.summary.verified_capacity_delta_bytes, None);
        assert!(validate_cleanup_session_history(&history).is_ok());

        let mut wrong_ordinal = history.clone();
        wrong_ordinal.items[0].ordinal = 1;
        assert_eq!(
            validate_cleanup_session_history(&wrong_ordinal),
            Err(CleanupHistoryError::CorruptData)
        );
        let mut leaked_policy_shape = history.clone();
        leaked_policy_shape.items[0].category = None;
        assert_eq!(
            validate_cleanup_session_history(&leaked_policy_shape),
            Err(CleanupHistoryError::CorruptData)
        );
        let mut mismatched_counts = history.clone();
        mismatched_counts.summary.path_total = 2;
        assert_eq!(
            validate_cleanup_session_history(&mismatched_counts),
            Err(CleanupHistoryError::CorruptData)
        );
        let mut mismatched_estimate = history.clone();
        mismatched_estimate.items[0].estimated_bytes = 1;
        assert_eq!(
            validate_cleanup_session_history(&mismatched_estimate),
            Err(CleanupHistoryError::CorruptData)
        );
        let mut overflowing_estimate = history.clone();
        overflowing_estimate.items[0].estimated_bytes = u64::MAX;
        let mut second_item = overflowing_estimate.items[0].clone();
        second_item.ordinal = 1;
        second_item.estimated_bytes = 1;
        overflowing_estimate.items.push(second_item);
        overflowing_estimate.summary.estimated_bytes = u64::MAX;
        overflowing_estimate.summary.item_total = 2;
        overflowing_estimate.summary.path_total = 2;
        overflowing_estimate.summary.evidence_total = 2;
        overflowing_estimate.summary.item_status_counts.trashed = 2;
        overflowing_estimate.summary.item_status_counts.total = 2;
        overflowing_estimate.summary.path_status_counts.trashed = 2;
        overflowing_estimate.summary.path_status_counts.total = 2;
        assert_eq!(
            validate_cleanup_session_history(&overflowing_estimate),
            Err(CleanupHistoryError::CorruptData)
        );
        let mut duplicate_warning = history.clone();
        duplicate_warning
            .warnings
            .push(duplicate_warning.warnings[0]);
        assert_eq!(
            validate_cleanup_session_history(&duplicate_warning),
            Err(CleanupHistoryError::CorruptData)
        );
        let mut reordered_warnings = history.clone();
        reordered_warnings.warnings.reverse();
        assert_eq!(
            validate_cleanup_session_history(&reordered_warnings),
            Err(CleanupHistoryError::CorruptData)
        );
        let mut verified_delta = history.clone();
        verified_delta.summary.status = CleanupSessionStatus::Completed;
        verified_delta.summary.completed_at_unix_ms =
            Some(verified_delta.summary.started_at_unix_ms + 1);
        verified_delta.summary.verified_capacity_delta_bytes = Some(4_096);
        assert_eq!(validate_cleanup_session_history(&verified_delta), Ok(()));
        assert_eq!(
            verified_delta.summary.verified_capacity_delta_bytes,
            Some(4_096)
        );
        verified_delta.summary.verified_capacity_delta_bytes = Some(-4_096);
        assert_eq!(validate_cleanup_session_history(&verified_delta), Ok(()));
        assert_eq!(
            verified_delta.summary.verified_capacity_delta_bytes,
            Some(-4_096)
        );

        assert_eq!(
            engine.cleanup_session_history(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION + 1,
                session_id: summary.session_id.clone(),
            }),
            Err(CleanupHistoryError::InvalidRecordVersion)
        );
        assert_eq!(
            engine.cleanup_session_history(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                session_id: "../not-a-session".to_owned(),
            }),
            Err(CleanupHistoryError::InvalidSessionId)
        );
        assert_eq!(
            engine.cleanup_session_history(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                session_id: "cleanup:missing".to_owned(),
            }),
            Err(CleanupHistoryError::SessionNotFound)
        );
        assert!(engine.close());
        assert_eq!(
            engine.cleanup_session_history(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                session_id: summary.session_id,
            }),
            Err(CleanupHistoryError::Closed)
        );
    }

    #[test]
    fn cleanup_session_history_maps_every_typed_status_warning_and_error() {
        let statuses = [
            (CoreCleanupItemStatus::Planned, CleanupItemStatus::Planned),
            (
                CoreCleanupItemStatus::Validating,
                CleanupItemStatus::Validating,
            ),
            (CoreCleanupItemStatus::DryRun, CleanupItemStatus::DryRun),
            (
                CoreCleanupItemStatus::EffectStarted,
                CleanupItemStatus::EffectStarted,
            ),
            (CoreCleanupItemStatus::Trashed, CleanupItemStatus::Trashed),
            (CoreCleanupItemStatus::Removed, CleanupItemStatus::Removed),
            (CoreCleanupItemStatus::Evicted, CleanupItemStatus::Evicted),
            (CoreCleanupItemStatus::Skipped, CleanupItemStatus::Skipped),
            (CoreCleanupItemStatus::Rejected, CleanupItemStatus::Rejected),
            (CoreCleanupItemStatus::Failed, CleanupItemStatus::Failed),
            (
                CoreCleanupItemStatus::ChangedSincePlan,
                CleanupItemStatus::ChangedSincePlan,
            ),
            (
                CoreCleanupItemStatus::Interrupted,
                CleanupItemStatus::Interrupted,
            ),
            (
                CoreCleanupItemStatus::Unavailable,
                CleanupItemStatus::Unavailable,
            ),
            (
                CoreCleanupItemStatus::OutcomeUnknown,
                CleanupItemStatus::OutcomeUnknown,
            ),
        ];
        for (core, expected) in statuses {
            assert_eq!(map_cleanup_item_status(core).unwrap(), expected);
        }

        let warnings = [
            (
                CoreCleanupWarning::EstimatedBytesUnverified,
                CleanupWarning::EstimatedBytesUnverified,
            ),
            (
                CoreCleanupWarning::DryRunDoesNotMutate,
                CleanupWarning::DryRunDoesNotMutate,
            ),
            (
                CoreCleanupWarning::TrashDoesNotFreeSpaceImmediately,
                CleanupWarning::TrashDoesNotFreeSpaceImmediately,
            ),
            (
                CoreCleanupWarning::PermanentRemovalCannotBeUndone,
                CleanupWarning::PermanentRemovalCannotBeUndone,
            ),
            (
                CoreCleanupWarning::CloudEvictionRequiresNetworkToRedownload,
                CleanupWarning::CloudEvictionRequiresNetworkToRedownload,
            ),
        ];
        for (core, expected) in warnings {
            assert_eq!(cleanup_warning(core).unwrap(), expected);
        }

        let errors = [
            (CoreCleanupHistoryError::Closed, CleanupHistoryError::Closed),
            (
                CoreCleanupHistoryError::InvalidLimit { maximum: 64 },
                CleanupHistoryError::InvalidLimit,
            ),
            (
                CoreCleanupHistoryError::SessionNotFound,
                CleanupHistoryError::SessionNotFound,
            ),
            (
                CoreCleanupHistoryError::IncompatibleSchema,
                CleanupHistoryError::IncompatibleSchema,
            ),
            (CoreCleanupHistoryError::Busy, CleanupHistoryError::Busy),
            (
                CoreCleanupHistoryError::UnsafeStorage,
                CleanupHistoryError::UnsafeStorage,
            ),
            (
                CoreCleanupHistoryError::QueryLimitExceeded,
                CleanupHistoryError::BudgetExceeded,
            ),
            (
                CoreCleanupHistoryError::CorruptData,
                CleanupHistoryError::CorruptData,
            ),
            (
                CoreCleanupHistoryError::Unavailable,
                CleanupHistoryError::Unavailable,
            ),
            (
                CoreCleanupHistoryError::InternalState,
                CleanupHistoryError::InternalState,
            ),
        ];
        for (core, expected) in errors {
            assert_eq!(map_cleanup_history_error(core), expected);
        }
        assert!(is_bounded_cleanup_history_token(
            "permanent_safe_target_changed",
            MAX_CLEANUP_HISTORY_ERROR_CATEGORY_BYTES
        ));
        assert!(!is_bounded_cleanup_history_token(
            "error category with spaces",
            MAX_CLEANUP_HISTORY_ERROR_CATEGORY_BYTES
        ));
    }

    #[test]
    fn capacity_trend_endpoint_preserves_signed_changes_and_bounds_points() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let volume_id = "01234567-89AB-CDEF-0123-456789ABCDEF";
        let day = 86_400_000_i64;
        let base = 1_800_000_000_000_i64;
        for (offset, available) in [(0, 900), (2 * day, 800), (7 * day, 700), (8 * day, 650)] {
            engine
                .observe_startup_volume(StartupVolumeObservation {
                    record_version: FFI_RECORD_VERSION,
                    stable_volume_id: Some(volume_id.to_owned()),
                    display_name: Some("Macintosh HD".to_owned()),
                    filesystem: Some("APFS".to_owned()),
                    is_internal: Some(true),
                    is_removable: Some(false),
                    sampled_at_unix_ms: base + offset,
                    total_bytes: 1_000,
                    ordinary_available_bytes: Some(available),
                    important_available_bytes: Some(available),
                })
                .unwrap();
        }
        let trend = engine
            .get_capacity_trend(CapacityTrendRequest {
                record_version: FFI_RECORD_VERSION,
                stable_volume_id: volume_id.to_owned(),
                anchor_at_unix_ms: base + 8 * day + 1,
            })
            .unwrap();
        assert_eq!(
            trend.stable_volume_id,
            "volume:macos:01234567-89ab-cdef-0123-456789abcdef"
        );
        assert_eq!(trend.available_bytes, 650);
        assert_eq!(trend.change_24h.unwrap().available_bytes, -50);
        assert_eq!(trend.change_7d.unwrap().available_bytes, -250);
        assert_eq!(trend.points.len(), 4);
        assert!(
            trend
                .points
                .windows(2)
                .all(|pair| { pair[0].sampled_at_unix_ms < pair[1].sampled_at_unix_ms })
        );
        assert!(engine.close());
    }

    fn scan_request(root: &std::path::Path) -> ScanRequest {
        ScanRequest {
            record_version: FFI_RECORD_VERSION,
            root: root.to_string_lossy().into_owned(),
        }
    }

    #[test]
    fn trash_effect_request_is_core_issued_and_one_shot() {
        let request = TrashEffectRequest::for_test(
            TrashEffectTargetKind::Symlink,
            SnapshotNameEncoding::UnixBytes,
            b"/private/tmp/dux-reviewed-link".to_vec(),
        );
        assert_eq!(request.record_version().unwrap(), FFI_RECORD_VERSION);
        assert_eq!(
            request.target_kind().unwrap(),
            TrashEffectTargetKind::Symlink
        );
        assert_eq!(
            request.path_encoding().unwrap(),
            SnapshotNameEncoding::UnixBytes
        );
        assert_eq!(
            request.take_path_bytes().unwrap(),
            b"/private/tmp/dux-reviewed-link"
        );
        assert_eq!(
            request.take_path_bytes().unwrap_err(),
            TrashEffectRequestError::Consumed
        );
        assert_eq!(
            request.record_version().unwrap_err(),
            TrashEffectRequestError::Consumed
        );
    }

    #[test]
    fn changed_since_plan_maps_to_a_distinct_trash_error() {
        assert_eq!(
            map_trash_selection_error(CoreTrashSelectionError::ChangedSincePlan),
            TrashExecutionError::ChangedSincePlan
        );
    }

    struct RecordingTrashDriver {
        calls: Mutex<Vec<Vec<u8>>>,
    }

    impl TrashPlatformDriver for RecordingTrashDriver {
        fn trash(&self, request: Arc<TrashEffectRequest>) -> TrashPlatformResult {
            match request.take_path_bytes() {
                Ok(path) => {
                    self.calls.lock().unwrap().push(path);
                    TrashPlatformResult::Completed
                }
                Err(_) => TrashPlatformResult::OutcomeUnknown,
            }
        }
    }

    #[test]
    fn trash_platform_callback_is_synchronous_and_cannot_retry_a_request() {
        let driver = RecordingTrashDriver {
            calls: Mutex::new(Vec::new()),
        };
        let request = TrashEffectRequest::for_test(
            TrashEffectTargetKind::File,
            SnapshotNameEncoding::UnixBytes,
            b"/private/tmp/dux-reviewed-file".to_vec(),
        );
        assert_eq!(
            driver.trash(Arc::clone(&request)),
            TrashPlatformResult::Completed
        );
        assert_eq!(
            driver.trash(request),
            TrashPlatformResult::OutcomeUnknown,
            "a callback retry must not obtain the target bytes"
        );
        assert_eq!(
            driver.calls.lock().unwrap().as_slice(),
            [b"/private/tmp/dux-reviewed-file".to_vec()]
        );
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
    fn scan_poll_exposes_ordered_typed_events_and_stable_cursor() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("scan-root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("one.txt"), b"one").unwrap();

        let start = engine.start_scan(scan_request(&root)).unwrap();
        let mut observed = Vec::new();
        let mut terminal = None;
        let deadline = Instant::now() + Duration::from_secs(10);
        while terminal.is_none() {
            let poll = start.task.poll().unwrap();
            observed.extend(poll.events.iter().map(|event| event.sequence));
            if matches!(
                poll.phase,
                TaskPhase::Succeeded | TaskPhase::Failed | TaskPhase::Cancelled
            ) {
                terminal = Some(poll);
            } else {
                assert!(Instant::now() < deadline, "scan did not become terminal");
                std::thread::yield_now();
            }
        }
        assert!(!observed.is_empty());
        assert!(observed.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            observed.last().copied(),
            Some(terminal.unwrap().next_event_sequence)
        );
        assert!(observed.contains(&1));

        let drained = start.task.poll().unwrap();
        observed.extend(drained.events.iter().map(|event| event.sequence));
        let empty = start.task.poll().unwrap();
        assert!(empty.events.is_empty());
        assert_eq!(empty.next_event_sequence, observed.last().copied().unwrap());
        assert_eq!(empty.oldest_available_event_sequence, 1);
        assert!(engine.close());
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
        assert!(matches!(
            review.candidate_paths("candidate:missing".to_owned(), 0, 0),
            Err(EngineError::InvalidCandidateDetailRequest)
        ));
        assert!(matches!(
            review.candidate_evidence("candidate:missing".to_owned(), 0, 65),
            Err(EngineError::InvalidCandidateDetailRequest)
        ));
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
        let events = state.apply(CoreTaskEventBatch {
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
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![4, 5, 6]
        );
        assert!(matches!(events[0].kind, ScanEventKind::Started));
        assert!(matches!(events[1].kind, ScanEventKind::ScanProgress { .. }));
        assert!(matches!(events[2].kind, ScanEventKind::ScanFinalizing));
        assert_eq!(state.event_cursor, 6);
        assert_eq!(state.next_event_sequence, 6);
        assert_eq!(state.oldest_available_event_sequence, 4);
        assert_eq!(state.stage, ScanStage::Finalizing);
        assert!(state.has_progress);
        assert_eq!(state.files_scanned, 12);
        assert_eq!(state.directories_scanned, 3);
        assert_eq!(state.known_allocated_bytes, 4096);
        assert_eq!(state.error_count, 2);
        assert!(state.events_truncated);

        let events = state.apply(CoreTaskEventBatch {
            events: vec![CoreTaskEvent {
                sequence: 7,
                kind: TaskEventKind::CandidateEvaluationStarted,
            }],
            next_sequence: 7,
            oldest_available_sequence: 4,
            truncated: false,
            terminal: false,
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].kind,
            ScanEventKind::CandidateEvaluationStarted
        ));
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
    fn permanent_cleanup_policy_get_set_reset_is_versioned_typed_and_path_free() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let initial = engine.get_permanent_cleanup_policy().unwrap();
        assert_eq!(initial.record_version, 1);
        assert!(initial.enabled);
        assert_eq!(initial.source, PermanentCleanupPolicySource::Default);
        assert_eq!(initial.revision, 0);
        assert_eq!(initial.updated_at_unix_ms, None);

        let disabled = engine.set_permanent_cleanup_enabled(false).unwrap();
        assert!(disabled.changed);
        assert!(!disabled.policy.enabled);
        assert_eq!(disabled.policy.source, PermanentCleanupPolicySource::Stored);
        assert_eq!(disabled.policy.revision, 1);
        assert!(disabled.policy.updated_at_unix_ms.is_some());
        assert_eq!(
            engine.get_permanent_cleanup_policy().unwrap(),
            disabled.policy
        );

        let exact = engine.set_permanent_cleanup_enabled(false).unwrap();
        assert!(!exact.changed);
        assert_eq!(exact.policy, disabled.policy);

        let reset = engine.reset_permanent_cleanup().unwrap();
        assert!(reset.changed);
        assert!(reset.policy.enabled);
        assert_eq!(reset.policy.source, PermanentCleanupPolicySource::Default);
        assert_eq!(reset.policy.revision, 2);
        assert!(reset.policy.updated_at_unix_ms.is_some());
        let exact_reset = engine.reset_permanent_cleanup().unwrap();
        assert!(!exact_reset.changed);
        assert_eq!(exact_reset.policy, reset.policy);

        assert!(engine.close());
        assert_eq!(
            engine.get_permanent_cleanup_policy(),
            Err(PermanentCleanupPolicyError::Closed)
        );
        assert_eq!(
            engine.set_permanent_cleanup_enabled(false),
            Err(PermanentCleanupPolicyError::Closed)
        );
        assert_eq!(
            engine.reset_permanent_cleanup(),
            Err(PermanentCleanupPolicyError::Closed)
        );
    }

    #[test]
    fn cleanup_exclusions_get_set_reset_round_trip_is_lossless_and_deny_only() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let first = temp.path().join("excluded-first");
        let second = temp.path().join("excluded-second");
        let input_path = |path: &Path| CleanupExclusionPath {
            encoding: SnapshotNameEncoding::UnixBytes,
            encoded_bytes: path.to_str().unwrap().as_bytes().to_vec(),
        };

        let initial = engine.get_cleanup_exclusions().unwrap();
        assert_eq!(initial.record_version, 1);
        assert!(initial.paths.is_empty());
        assert_eq!(initial.source, CleanupExclusionsSource::Default);
        assert_eq!(initial.revision, 0);
        assert_eq!(initial.updated_at_unix_ms, None);

        let stored = engine
            .set_cleanup_exclusions(CleanupExclusionsInput {
                record_version: 1,
                paths: vec![input_path(&second), input_path(&first)],
            })
            .unwrap();
        assert!(stored.changed);
        assert_eq!(stored.record_version, 1);
        assert_eq!(stored.exclusions.source, CleanupExclusionsSource::Stored);
        assert_eq!(stored.exclusions.revision, 1);
        assert!(stored.exclusions.updated_at_unix_ms.is_some());
        assert_eq!(
            stored.exclusions.paths,
            vec![input_path(&first), input_path(&second)]
        );
        assert_eq!(engine.get_cleanup_exclusions().unwrap(), stored.exclusions);

        let exact = engine
            .set_cleanup_exclusions(CleanupExclusionsInput {
                record_version: 1,
                paths: vec![input_path(&first), input_path(&second)],
            })
            .unwrap();
        assert!(!exact.changed);
        assert_eq!(exact.exclusions, stored.exclusions);

        let reset = engine.reset_cleanup_exclusions().unwrap();
        assert!(reset.changed);
        assert_eq!(reset.exclusions.source, CleanupExclusionsSource::Default);
        assert!(reset.exclusions.paths.is_empty());
        assert_eq!(reset.exclusions.revision, 0);
        assert_eq!(reset.exclusions.updated_at_unix_ms, None);
        let exact_reset = engine.reset_cleanup_exclusions().unwrap();
        assert!(!exact_reset.changed);
        assert_eq!(exact_reset.exclusions, reset.exclusions);

        assert!(engine.close());
        assert_eq!(
            engine.get_cleanup_exclusions(),
            Err(CleanupExclusionsError::Closed)
        );
        assert_eq!(
            engine.reset_cleanup_exclusions(),
            Err(CleanupExclusionsError::Closed)
        );
    }

    #[test]
    fn cleanup_exclusions_rejects_malformed_inputs_before_core() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let valid = || CleanupExclusionPath {
            encoding: SnapshotNameEncoding::UnixBytes,
            encoded_bytes: b"/private/tmp/dux-exclusion".to_vec(),
        };

        assert_eq!(
            engine.set_cleanup_exclusions(CleanupExclusionsInput {
                record_version: 2,
                paths: Vec::new(),
            }),
            Err(CleanupExclusionsError::InvalidRecordVersion)
        );
        assert_eq!(
            engine.set_cleanup_exclusions(CleanupExclusionsInput {
                record_version: 1,
                paths: vec![CleanupExclusionPath {
                    encoding: SnapshotNameEncoding::UnixBytes,
                    encoded_bytes: Vec::new(),
                }],
            }),
            Err(CleanupExclusionsError::InvalidPath)
        );
        assert_eq!(
            engine.set_cleanup_exclusions(CleanupExclusionsInput {
                record_version: 1,
                paths: vec![CleanupExclusionPath {
                    encoding: SnapshotNameEncoding::UnixBytes,
                    encoded_bytes: b"relative/path".to_vec(),
                }],
            }),
            Err(CleanupExclusionsError::InvalidPath)
        );
        assert_eq!(
            engine.set_cleanup_exclusions(CleanupExclusionsInput {
                record_version: 1,
                paths: vec![CleanupExclusionPath {
                    encoding: SnapshotNameEncoding::UnixBytes,
                    encoded_bytes: b"/private/tmp/with/../parent".to_vec(),
                }],
            }),
            Err(CleanupExclusionsError::InvalidPath)
        );
        assert_eq!(
            engine.set_cleanup_exclusions(CleanupExclusionsInput {
                record_version: 1,
                paths: vec![CleanupExclusionPath {
                    encoding: SnapshotNameEncoding::WindowsUtf16LittleEndian,
                    encoded_bytes: b"/private/tmp/dux-exclusion".to_vec(),
                }],
            }),
            Err(CleanupExclusionsError::InvalidPath)
        );

        let too_many = (0..=MAX_CLEANUP_EXCLUSION_COUNT)
            .map(|_| valid())
            .collect::<Vec<_>>();
        assert_eq!(
            engine.set_cleanup_exclusions(CleanupExclusionsInput {
                record_version: 1,
                paths: too_many,
            }),
            Err(CleanupExclusionsError::TooManyPaths)
        );

        let too_long = CleanupExclusionPath {
            encoding: SnapshotNameEncoding::UnixBytes,
            encoded_bytes: std::iter::once(b'/')
                .chain(std::iter::repeat_n(b'x', MAX_CLEANUP_EXCLUSION_PATH_BYTES))
                .collect(),
        };
        assert_eq!(
            engine.set_cleanup_exclusions(CleanupExclusionsInput {
                record_version: 1,
                paths: vec![too_long],
            }),
            Err(CleanupExclusionsError::InvalidPath)
        );
    }

    #[test]
    fn cleanup_exclusions_projection_rejects_malformed_core_shapes_and_paths() {
        let malformed_default = CoreCleanupExclusions {
            paths: vec![PathBuf::from("/private/tmp/should-not-be-default")],
            source: CoreCleanupExclusionSource::Default,
            revision: 0,
            updated_at: None,
        };
        assert_eq!(
            cleanup_exclusions_status(malformed_default),
            Err(CleanupExclusionsError::InternalState)
        );

        let malformed_revision = CoreCleanupExclusions {
            paths: Vec::new(),
            source: CoreCleanupExclusionSource::Stored,
            revision: 0,
            updated_at: None,
        };
        assert_eq!(
            cleanup_exclusions_status(malformed_revision),
            Err(CleanupExclusionsError::InternalState)
        );

        let malformed_path = CoreCleanupExclusions {
            paths: vec![PathBuf::from("relative/path")],
            source: CoreCleanupExclusionSource::Stored,
            revision: 1,
            updated_at: Some(UNIX_EPOCH),
        };
        assert_eq!(
            cleanup_exclusions_status(malformed_path),
            Err(CleanupExclusionsError::InternalState)
        );

        let too_many = CoreCleanupExclusions {
            paths: (0..=MAX_CLEANUP_EXCLUSION_COUNT)
                .map(|index| PathBuf::from(format!("/private/tmp/exclusion-{index:02}")))
                .collect(),
            source: CoreCleanupExclusionSource::Stored,
            revision: 1,
            updated_at: Some(UNIX_EPOCH),
        };
        assert_eq!(
            cleanup_exclusions_status(too_many),
            Err(CleanupExclusionsError::InternalState)
        );

        let oversized_path = CoreCleanupExclusions {
            paths: vec![PathBuf::from(format!(
                "/{}",
                "x".repeat(MAX_CLEANUP_EXCLUSION_PATH_BYTES)
            ))],
            source: CoreCleanupExclusionSource::Stored,
            revision: 1,
            updated_at: Some(UNIX_EPOCH),
        };
        assert_eq!(
            cleanup_exclusions_status(oversized_path),
            Err(CleanupExclusionsError::InternalState)
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
        assert_eq!(
            review.candidate_summaries(0, 0),
            Err(EngineError::InvalidCandidateDetailRequest)
        );
        assert_eq!(
            review.candidate_paths("candidate:missing".into(), 0, 0),
            Err(EngineError::InvalidCandidateDetailRequest)
        );
        assert_eq!(
            review.candidate_evidence("candidate:missing".into(), 0, 0),
            Err(EngineError::InvalidCandidateDetailRequest)
        );
        assert_eq!(
            review.candidate_paths("bad candidate".into(), 0, 1),
            Err(EngineError::InvalidCandidateDetailRequest)
        );
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
            review.candidate_summaries(0, 1),
            Err(EngineError::ReviewExpired)
        );
        assert_eq!(
            review.candidate_paths("candidate:missing".into(), 0, 1),
            Err(EngineError::ReviewExpired)
        );
        assert_eq!(
            review.candidate_evidence("candidate:missing".into(), 0, 1),
            Err(EngineError::ReviewExpired)
        );
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
    fn candidate_detail_projection_enforces_item_and_aggregate_payload_budgets() {
        assert_eq!(
            candidate_path_payload_from_lengths(
                MAX_CANDIDATE_ENCODED_PATH_BYTES,
                MAX_CANDIDATE_DISPLAY_PATH_BYTES
            ),
            Ok(MAX_CANDIDATE_ENCODED_PATH_BYTES + MAX_CANDIDATE_DISPLAY_PATH_BYTES)
        );
        assert_eq!(
            candidate_path_payload_from_lengths(MAX_CANDIDATE_ENCODED_PATH_BYTES + 1, 0),
            Err(EngineError::BudgetExceeded)
        );
        assert_eq!(
            candidate_path_payload_from_lengths(0, MAX_CANDIDATE_DISPLAY_PATH_BYTES + 1),
            Err(EngineError::BudgetExceeded)
        );
        assert_eq!(
            ensure_candidate_detail_page_payload([
                Ok(MAX_CANDIDATE_DETAIL_PAGE_PAYLOAD_BYTES),
                Ok(1),
            ]),
            Err(EngineError::BudgetExceeded)
        );
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

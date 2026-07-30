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
    CandidateEvaluationRecoveryMaintenanceOutcome as CoreCandidateEvaluationRecoveryOutcome,
    CandidateEvaluationRecoveryMaintenanceStartOutcome,
    CandidateEvaluationTaskFailureKind as CoreCandidateEvaluationFailure,
    CandidateEvaluationTaskStatus as CoreCandidateEvaluationStatus,
    CandidateHistoryError as CoreCandidateHistoryError,
    CandidateReviewCommand as CoreCandidateReviewCommand,
    CandidateReviewError as CoreCandidateReviewError,
    CapacityHistoryDisposition as CoreHistoryDisposition, CapacityTrend as CoreCapacityTrend,
    CapacityTrendChange as CoreCapacityTrendChange, CapacityTrendPoint as CoreCapacityTrendPoint,
    CapacityTrendPointSource as CoreCapacityTrendPointSource,
    ClaimedRunningScanProvenanceCensus as CoreClaimedRunningScanProvenanceCensus,
    ClaimedRunningScanProvenanceCensusError as CoreClaimedRunningScanProvenanceCensusError,
    CleanupExclusionSource as CoreCleanupExclusionSource,
    CleanupExclusions as CoreCleanupExclusions,
    CleanupExclusionsError as CoreCleanupExclusionsError,
    CleanupExclusionsUpdate as CoreCleanupExclusionsUpdate,
    CleanupHistoryClearError as CoreCleanupHistoryClearError,
    CleanupHistoryClearPreview as CoreCleanupHistoryClearPreview,
    CleanupHistoryClearPreviewInfo as CoreCleanupHistoryClearPreviewInfo,
    CleanupHistoryClearResult as CoreCleanupHistoryClearResult,
    CleanupHistoryCursor as CoreCleanupHistoryCursor,
    CleanupHistoryError as CoreCleanupHistoryError,
    CloudEvictionProbeError as CoreCloudEvictionProbeError,
    CloudEvictionProbePlatformError as CoreCloudEvictionProbePlatformError,
    ConfiguredProjectRoots as CoreConfiguredProjectRoots,
    ConfiguredProjectRootsError as CoreConfiguredProjectRootsError,
    ConfiguredProjectRootsSource as CoreConfiguredProjectRootsSource,
    ConfiguredProjectRootsUpdate as CoreConfiguredProjectRootsUpdate,
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
    DurableObservedPath as CoreObservedPath, DurableRuleOutcomeBatch as CoreRuleOutcomeBatch,
    DurableRuleOutcomeState as CoreRuleOutcomeState,
    DurableScanIssueKind as CoreDurableScanIssueKind, DurableScanStatus as CoreDurableScanStatus,
    DurableStorageThiefRanking as CoreStorageThiefRanking, EMERGENCY_RECOVERY_POLICY_REVISION,
    EmergencyRecoveryError as CoreEmergencyRecoveryError,
    EmergencyRecoveryGroup as CoreEmergencyRecoveryGroup,
    EmergencyRecoveryLane as CoreEmergencyRecoveryLane,
    EmergencyRecoveryOrdering as CoreEmergencyRecoveryOrdering,
    EmergencyRecoverySource as CoreEmergencyRecoverySource, EngineConfig, EngineHandle,
    EngineOpenError, HistoryMaintenanceStartOutcome,
    MAX_CLAIMED_RUNNING_SCAN_PROVENANCE_CENSUS_ROWS,
    PermanentCleanupPolicy as CorePermanentCleanupPolicy,
    PermanentCleanupPolicyError as CorePermanentCleanupPolicyError,
    PermanentCleanupPolicySource as CorePermanentCleanupPolicySource,
    PermanentCleanupPolicyUpdate as CorePermanentCleanupPolicyUpdate,
    PermanentSafeCleanupFailureKind as CorePermanentSafeCleanupFailureKind,
    PressureEpisodeHistory as CorePressureEpisodeHistory,
    PressureEpisodeHistoryError as CorePressureEpisodeHistoryError,
    PressureEpisodeLevel as CorePressureEpisodeLevel, RuleOutcomeError as CoreRuleOutcomeError,
    RuleOutcomeNotEligibleReason as CoreRuleOutcomeNotEligibleReason,
    RunningScanDebtCensus as CoreRunningScanDebtCensus,
    RunningScanDebtCensusError as CoreRunningScanDebtCensusError,
    RustTargetCleanupError as CoreRustTargetCleanupError,
    RustTargetCleanupResult as CoreRustTargetCleanupResult,
    RustTargetDryRunError as CoreRustTargetDryRunError,
    RustTargetDryRunFailureKind as CoreRustTargetDryRunFailureKind,
    RustTargetDryRunResult as CoreRustTargetDryRunResult,
    RustTargetPlanReview as CoreRustTargetPlanReview,
    RustTargetPlanReviewError as CoreRustTargetPlanReviewError,
    RustTargetPlanReviewInfo as CoreRustTargetPlanReviewInfo,
    ScanCoverageDetailsError as CoreScanCoverageDetailsError,
    ScanHistoryError as CoreScanHistoryError,
    ScanRecoveryMaintenanceOutcome as CoreScanRecoveryOutcome, ScanRecoveryMaintenanceStartOutcome,
    ScanRootErrorKind, ScanTaskOrigin as CoreScanTaskOrigin, ScanTaskResult as CoreScanTaskResult,
    ScanTaskStatus as CoreScanTaskStatus, SnapshotDiffChange as CoreSnapshotDiffChange,
    SnapshotDiffDirection as CoreSnapshotDiffDirection, SnapshotDiffInfo as CoreSnapshotDiffInfo,
    SnapshotDiffNode as CoreSnapshotDiffNode, SnapshotDiffNodePage as CoreSnapshotDiffNodePage,
    SnapshotDiffNodeSort as CoreSnapshotDiffNodeSort,
    SnapshotDiffReviewSession as CoreSnapshotDiffReviewSession,
    SnapshotDiffTreemap as CoreSnapshotDiffTreemap,
    SnapshotDiffTreemapCell as CoreSnapshotDiffTreemapCell,
    SnapshotDiffValue as CoreSnapshotDiffValue,
    SnapshotOrphanMaintenanceOutcome as CoreOrphanOutcome, SnapshotOrphanMaintenanceStartOutcome,
    SnapshotProvisioningStageMaintenanceOutcome as CoreStageOutcome,
    SnapshotProvisioningStageMaintenanceStartOutcome,
    SnapshotRetentionOutcome as CoreRetentionOutcome, SnapshotRetentionStartOutcome,
    SnapshotReviewCategory as CoreReviewCategory, SnapshotReviewError as CoreReviewError,
    SnapshotReviewICloudObservationSource as CoreReviewICloudObservationSource,
    SnapshotReviewICloudObservationTarget as CoreReviewICloudObservationTarget,
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
    StorageThiefError as CoreStorageThiefError,
    TargetedProjectScanAdmission as CoreTargetedProjectScanAdmission,
    TargetedProjectScanCheckpoint as CoreTargetedProjectScanCheckpoint,
    TargetedProjectScanCurrent as CoreTargetedProjectScanCurrent,
    TargetedProjectScanDisposition as CoreTargetedProjectScanDisposition,
    TargetedProjectScanError as CoreTargetedProjectScanError,
    TargetedProjectScanPressure as CoreTargetedProjectScanPressure,
    TargetedProjectScanPressureContext as CoreTargetedProjectScanPressureContext,
    TargetedReclaimRootCatalogStamp as CoreTargetedReclaimRootCatalogStamp,
    TargetedReclaimRootKind as CoreTargetedReclaimRootKind, TaskAccessError,
    TaskEventBatch as CoreTaskEventBatch, TaskEventKind as CoreTaskEventKind, TaskFailureKind,
    TaskId, TaskKind as CoreTaskKind, TaskPhase as CoreTaskPhase, TaskPriority as CoreTaskPriority,
    VolumeCapacityObservation as CoreVolumeObservation,
    VolumeCapacityStatusError as CoreVolumeStatusError,
};
use dux_core::{
    AvailableCapacitySource as CoreCapacitySource, BlockReason as CoreBlockReason,
    CandidateAction as CoreCandidateAction, CandidateCategory as CoreCandidateCategory,
    CandidateId, CleanupMode as CorePlanCleanupMode, CloudBooleanState as CoreCloudBooleanState,
    CloudErrorState as CoreCloudErrorState, CloudEvictionAssessment as CoreCloudEvictionAssessment,
    CloudEvictionBlockReason as CoreCloudEvictionBlockReason,
    CloudEvictionIdentityBlockReason as CoreCloudEvictionIdentityBlockReason,
    CloudEvictionIdentityFacts as CoreCloudEvictionIdentityFacts,
    CloudEvictionItemKind as CoreCloudEvictionItemKind,
    CloudEvictionPlatformFacts as CoreCloudEvictionPlatformFacts,
    CloudEvictionProvider as CoreCloudEvictionProvider,
    CloudIdentityFactState as CoreCloudIdentityFactState,
    CloudLocalCopyState as CoreCloudLocalCopyState, DATABASE_SCHEMA_VERSION, DatabaseOpenErrorKind,
    DiskPressure as CoreDiskPressure, DiskPressureConfig, DiskPressureConfigError,
    DiskPressureRecoveryMargin, DiskPressureThreshold, EvidenceKind as CoreEvidenceKind,
    PlanWarning as CorePlanWarning, SNAPSHOT_FORMAT_VERSION, SafetyTier as CoreSafetyTier,
    ScanCoverageStatus as CoreCoverageStatus, ScanId, SnapshotOpenErrorKind,
    TrashEffectTargetKind as CoreTrashEffectTargetKind,
    TrashPlatformResult as CoreTrashPlatformResult, TrashSelectionError as CoreTrashSelectionError,
    VolumeCapacity, VolumeId,
};

const FFI_CONTRACT_VERSION: u32 = 50;
const FFI_RECORD_VERSION: u32 = 1;
const RUST_TARGET_MINIMUM_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const SNAPSHOT_NODE_RECORD_VERSION: u32 = 2;
const SCAN_EVENT_PAGE_LIMIT: u16 = 64;
const RECENT_SCAN_HISTORY_PAGE_LIMIT: u16 = 200;
const SCAN_COVERAGE_DETAIL_PAGE_LIMIT: u16 = 64;
const CANDIDATE_DETAIL_PAGE_LIMIT: u16 = 64;
const MAX_CANDIDATE_ENCODED_PATH_BYTES: usize = 65_536;
const MAX_CANDIDATE_DISPLAY_PATH_BYTES: usize = MAX_CANDIDATE_ENCODED_PATH_BYTES * 4;
const MAX_CANDIDATE_DETAIL_PAGE_PAYLOAD_BYTES: usize = 24 * 1_024 * 1_024;
const MAX_CANDIDATE_IDENTIFIER_BYTES: usize = 4_096;
const MAX_RULE_OUTCOMES: usize = 64;
const MAX_STORAGE_THIEF_GROUPS: usize = 12;
const MAX_STORAGE_THIEF_SOURCE_SESSIONS: u16 = 32;
const MAX_RUNNING_SCAN_DEBT_CENSUS_ROWS: u16 = 64;
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
const MAX_CONFIGURED_PROJECT_ROOT_COUNT: usize = 16;
const MAX_CONFIGURED_PROJECT_ROOT_PATH_BYTES: usize = 32 * 1_024;
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
    pub database_schema_version: u32,
    pub snapshot_format_version: u32,
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

/// Bounded, path-free pressure-history request anchored to one accepted
/// startup-volume sample.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PressureEpisodeHistoryRequest {
    pub record_version: u32,
    pub stable_volume_id: String,
    pub anchor_at_unix_ms: i64,
    pub limit: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PressureEpisodeLevel {
    Warning,
    Critical,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PressureEpisodeRecord {
    pub record_version: u32,
    pub level: PressureEpisodeLevel,
    pub entered_at_unix_ms: i64,
    pub exited_at_unix_ms: Option<i64>,
    pub policy_revision: u64,
}

/// Newest-first pressure intervals as of `anchor_at_unix_ms`. This telemetry
/// contains no paths, candidate identity, plan, approval, or mutation command.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PressureEpisodeHistoryStatus {
    pub record_version: u32,
    pub stable_volume_id: String,
    pub anchor_at_unix_ms: i64,
    pub episodes: Vec<PressureEpisodeRecord>,
    pub has_more: bool,
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

/// Versioned, path-free global permanent-cleanup opt-in gate. It never selects
/// a target or grants execution authority.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ConfiguredProjectRootsSource {
    Default,
    Stored,
}

/// Lossless local path selected only as read-only project discovery scope.
///
/// These bytes cannot become a cleanup target, plan, approval, or effect.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ConfiguredProjectRootPath {
    pub encoding: SnapshotNameEncoding,
    pub encoded_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ConfiguredProjectRootsInput {
    pub record_version: u32,
    pub roots: Vec<ConfiguredProjectRootPath>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ConfiguredProjectRootsStatus {
    pub record_version: u32,
    pub roots: Vec<ConfiguredProjectRootPath>,
    pub source: ConfiguredProjectRootsSource,
    pub revision: u64,
    pub updated_at_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ConfiguredProjectRootsUpdate {
    pub record_version: u32,
    pub roots: ConfiguredProjectRootsStatus,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ConfiguredProjectRootsError {
    #[error("engine session is closed")]
    Closed,
    #[error("configured-project-root record version is unsupported")]
    InvalidRecordVersion,
    #[error("configured project root path bytes are invalid")]
    InvalidPath,
    #[error("the configured project root set exceeds its fixed bound")]
    TooManyPaths,
    #[error("configured project roots overlap")]
    OverlappingPaths,
    #[error("the configured-project-root revision cannot advance")]
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
    #[error("configured project roots are corrupt")]
    CorruptData,
    #[error("configured project roots are unavailable")]
    Unavailable,
    #[error("the settings write outcome could not be proven")]
    OutcomeUnknown,
    #[error("engine settings state is unavailable")]
    InternalState,
}

/// Versioned path-free request for one configured-root pressure scan.
///
/// The opaque stable volume identity selects the accepted macOS capacity
/// observation. The root remains sealed in Rust and is addressed only by its
/// stored ordinal.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TargetedProjectScanRequest {
    pub record_version: u32,
    pub stable_volume_id: String,
    pub capacity_anchor_unix_ms: i64,
    pub selected_root_ordinal: u16,
    pub expected_configured_roots_revision: Option<u64>,
    pub expected_root_catalog_digest_sha256: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TargetedProjectScanPressure {
    Warning,
    Critical,
}

/// Exact durable pressure proof used for every admission in one bounded pass.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TargetedProjectScanPressureContext {
    pub record_version: u32,
    pub stable_volume_id: String,
    pub capacity_anchor_unix_ms: i64,
    pub pressure: TargetedProjectScanPressure,
    pub current_episode_started_at_unix_ms: i64,
    pub pressure_started_at_unix_ms: i64,
    pub policy_revision: u64,
}

/// Lossless stored discovery root plus the per-root node budget enforced by
/// Rust. The returned bytes are display/observation data, not cleanup input.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TargetedProjectScanSelection {
    pub record_version: u32,
    pub ordinal: u16,
    pub kind: TargetedReclaimRootKind,
    pub root: ConfiguredProjectRootPath,
    pub max_nodes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TargetedReclaimRootKind {
    KnownUserLibraryCaches,
    ConfiguredProject,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TargetedReclaimRootCatalog {
    pub record_version: u32,
    pub known_roots_policy_revision: u32,
    pub configured_roots_revision: u64,
    pub known_user_library_caches_included: bool,
    pub root_count: u16,
    pub digest_sha256: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TargetedProjectScanRootUnavailableReason {
    InvalidPath,
    Missing,
    AccessDenied,
    NotDirectory,
    Symlink,
    ChangedDuringValidation,
    IdentityUnavailable,
    VolumeMismatch,
    VolumeUnproven,
    UnsupportedPlatform,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TargetedProjectScanDisposition {
    EmptyRegistry,
    NoPressure,
    RootUnavailable,
    ExistingTask,
    Current,
    Started,
}

/// Strict tagged admission. Optional payloads are populated only for the
/// corresponding disposition; Swift must independently reject contradictory
/// shapes before presenting or retaining a task.
#[derive(Clone, uniffi::Record)]
pub struct TargetedProjectScanAdmission {
    pub record_version: u32,
    pub configured_roots_revision: u64,
    pub root_count: u16,
    pub root_catalog: TargetedReclaimRootCatalog,
    pub selection: Option<TargetedProjectScanSelection>,
    pub pressure: Option<TargetedProjectScanPressureContext>,
    pub disposition: TargetedProjectScanDisposition,
    pub root_unavailable_reason: Option<TargetedProjectScanRootUnavailableReason>,
    pub current_result: Option<ScanTaskResult>,
    pub task: Option<Arc<ScanTask>>,
    pub existing_task_observed_phase: Option<TaskPhase>,
}

/// Path-free end-of-pass request. It repeats only the exact pressure and
/// registry facts returned by admission; it cannot select a filesystem root.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TargetedProjectScanCheckpointRequest {
    pub record_version: u32,
    pub expected_pressure: TargetedProjectScanPressureContext,
    pub expected_root_catalog: TargetedReclaimRootCatalog,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TargetedProjectScanCheckpoint {
    pub record_version: u32,
    pub configured_roots_revision: u64,
    pub root_count: u16,
    pub root_catalog: TargetedReclaimRootCatalog,
    pub pressure: TargetedProjectScanPressureContext,
}

/// Exact path-free proof echoed into atomic Critical recovery finalization.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EmergencyRecoveryRequest {
    pub record_version: u32,
    pub expected_pressure: TargetedProjectScanPressureContext,
    pub expected_root_catalog: TargetedReclaimRootCatalog,
}

/// Fixed §13.3 taxonomy. Rust supplies explicit ranks; discriminants are not
/// policy and clients must not infer priority from declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum EmergencyRecoveryLane {
    EvictableCloud,
    StaleSafeRegenerable,
    TrashInformation,
    ReviewableInstallerArchive,
    LargeFile,
    GuidedExploration,
    PermissionGap,
}

/// One path-free exact-scan navigation source.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EmergencyRecoverySource {
    pub record_version: u32,
    pub root_ordinal: u16,
    pub scan_id: String,
    pub observed_at_unix_ms: i64,
    pub candidate_count: Option<u32>,
    pub blocked_candidate_count: Option<u32>,
    pub permission_issue_count: Option<u64>,
}

/// Bounded display-only recovery group. Optional shapes are lane-specific and
/// validated before crossing the boundary.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EmergencyRecoveryGroup {
    pub record_version: u32,
    pub rank: u16,
    pub lane: EmergencyRecoveryLane,
    pub rule_id: Option<String>,
    pub rule_revision: Option<u32>,
    pub category: Option<CandidateCategory>,
    pub unavailable_root_count: u16,
    pub sources: Vec<EmergencyRecoverySource>,
}

/// Atomic path-free projection tied to the exact returned Critical proof.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EmergencyRecoveryOrdering {
    pub record_version: u32,
    pub policy_revision: u32,
    pub pressure: TargetedProjectScanPressureContext,
    pub root_catalog: TargetedReclaimRootCatalog,
    pub observed_root_count: u16,
    pub candidate_evaluated_root_count: u16,
    pub unavailable_root_count: u16,
    pub groups: Vec<EmergencyRecoveryGroup>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum EmergencyRecoveryError {
    #[error("engine session is closed")]
    Closed,
    #[error("emergency-recovery record version is unsupported")]
    InvalidRecordVersion,
    #[error("the exact pressure proof is malformed")]
    InvalidPressureProof,
    #[error("the targeted-reclaim root catalog proof is malformed")]
    InvalidCatalog,
    #[error("emergency recovery ordering requires Critical pressure")]
    NotCritical,
    #[error("configured project roots changed")]
    RegistryChanged,
    #[error("the targeted-reclaim root catalog changed")]
    CatalogChanged,
    #[error("the exact pressure proof changed")]
    PressureChanged,
    #[error("the durable engine store is read-only")]
    ReadOnlyStore,
    #[error("durable schema is incompatible")]
    IncompatibleSchema,
    #[error("operation is temporarily busy")]
    Busy,
    #[error("storage failed its safety checks")]
    UnsafeStorage,
    #[error("bounded operation exceeded its resource budget")]
    BudgetExceeded,
    #[error("durable emergency-recovery state is corrupt")]
    CorruptData,
    #[error("durable emergency-recovery state is unavailable")]
    Unavailable,
    #[error("operation outcome is unknown")]
    OutcomeUnknown,
    #[error("internal emergency-recovery state is invalid")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum TargetedProjectScanError {
    #[error("engine session is closed")]
    Closed,
    #[error("targeted-project-scan record version is unsupported")]
    InvalidRecordVersion,
    #[error("the stable macOS volume identity is invalid")]
    InvalidVolumeIdentity,
    #[error("the capacity anchor is invalid or is not the latest accepted observation")]
    InvalidAnchor,
    #[error("configured project root ordinal is outside the current registry")]
    InvalidOrdinal,
    #[error("the targeted-reclaim root catalog proof is malformed")]
    InvalidCatalog,
    #[error("configured project roots changed during the targeted scan pass")]
    RegistryChanged,
    #[error("the derived targeted-reclaim root catalog changed during the scan pass")]
    CatalogChanged,
    #[error("disk pressure changed during the targeted scan pass")]
    PressureChanged,
    #[error("the durable engine store is read-only")]
    ReadOnlyStore,
    #[error("the durable schema is incompatible")]
    IncompatibleSchema,
    #[error("operation is temporarily busy")]
    Busy,
    #[error("storage failed its safety checks")]
    UnsafeStorage,
    #[error("bounded operation exceeded its resource budget")]
    BudgetExceeded,
    #[error("durable targeted-scan state is corrupt")]
    CorruptData,
    #[error("durable targeted-scan state is unavailable")]
    Unavailable,
    #[error("operation outcome is unknown")]
    OutcomeUnknown,
    #[error("engine task queue is full")]
    QueueFull,
    #[error("engine task identifiers are exhausted")]
    TaskIdExhausted,
    #[error("internal targeted-scan state is invalid")]
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
    #[error("pressure episode history request is invalid")]
    InvalidPressureEpisodeRequest,
    #[error("a different capacity observation already exists at this time")]
    ConflictingCapacityObservation,
    #[error("a newer capacity observation already exists")]
    SupersededCapacityObservation,
    #[error("scan does not exist")]
    ScanNotFound,
    #[error("scan snapshot is unavailable")]
    SnapshotUnavailable,
    #[error("the snapshot has no preceding comparable retained snapshot")]
    ComparableSnapshotUnavailable,
    #[error("the comparison does not belong to this exact snapshot review")]
    WrongParentReview,
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
    #[error("snapshot iCloud observation-source request is invalid")]
    InvalidSnapshotICloudObservationSourceRequest,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RuleOutcomeNotEligibleReason {
    SourceCleanupIncomplete,
    ItemNotSuccessfulPermanentRegenerable,
    SourceScanNotComparable,
    SourceEvaluationNotComparable,
    SourceEvaluationAfterPlan,
    SourceCandidateMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RuleOutcomeState {
    NotEligible {
        reason: RuleOutcomeNotEligibleReason,
    },
    AwaitingComparableScan {
        cleaned_at_unix_ms: i64,
    },
    Superseded {
        cleaned_at_unix_ms: i64,
        superseded_at_unix_ms: i64,
    },
    LaterSizeObserved {
        cleaned_at_unix_ms: i64,
        observed_at_unix_ms: i64,
        observed_bytes: u64,
    },
    ZeroBaselineObserved {
        cleaned_at_unix_ms: i64,
        observed_at_unix_ms: i64,
    },
    Regrown {
        cleaned_at_unix_ms: i64,
        zero_observed_at_unix_ms: i64,
        observed_at_unix_ms: i64,
        observed_bytes: u64,
    },
}

/// One path-free, read-only observation derived for the matching cleanup item.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RuleOutcome {
    pub record_version: u32,
    pub item_ordinal: u16,
    pub rule_id: String,
    pub rule_revision: u32,
    pub state: RuleOutcomeState,
}

/// Exact-session outcome batch. It contains no path, candidate ID, scan
/// identity, plan, approval, schedule, AI input, or filesystem capability.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RuleOutcomeBatch {
    pub record_version: u32,
    pub session_id: String,
    pub outcomes: Vec<RuleOutcome>,
}

/// One deterministic rule-ID aggregate from a bounded recent history window.
/// It contains no paths, source identities, plan facts, or mutation capability.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StorageThiefGroup {
    pub record_version: u32,
    pub rank: u16,
    pub rule_id: String,
    pub latest_rule_revision: u32,
    pub observed_revision_count: u16,
    pub successful_cleanup_count: u16,
    pub successful_manual_cleanup_count: u16,
    pub observed_regrowth_cycle_count: u16,
    pub manual_regrowth_cycle_count: u16,
    pub total_observed_regrown_bytes: u64,
    pub total_regrowth_duration_seconds: u64,
    pub total_regrowth_duration_nanoseconds: u32,
    pub bytes_regrown_per_day: u64,
    pub rate_capped: bool,
    pub latest_cleanup_at_unix_ms: i64,
    pub latest_regrowth_at_unix_ms: i64,
    pub automation_history_threshold_met: bool,
}

/// Read-only recurring-growth ranking. The source-count and truncation fields
/// are part of the contract so clients cannot present the window as all-time.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StorageThiefRanking {
    pub record_version: u32,
    pub permanent_safe_session_count: u16,
    pub manual_cleanup_session_count: u16,
    pub ranked_rule_count: u16,
    pub has_older_permanent_safe_sessions: bool,
    pub groups: Vec<StorageThiefGroup>,
}

/// Bounded, path-free census of unclaimed running scan rows. This record is
/// diagnostic evidence only and carries no row selector or mutation authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RunningScanDebtCensus {
    pub record_version: u32,
    pub inspected_unclaimed_count: u16,
    pub pristine_unclaimed_count: u16,
    pub unexplained_unclaimed_count: u16,
    pub has_more: bool,
}

/// Bounded, path-free census of provenance relationships for claimed running
/// scan rows. These aggregate counts are diagnostic evidence only and expose
/// no claim selector, provenance digest, liveness fact, or mutation authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ClaimedRunningScanProvenanceCensus {
    pub record_version: u32,
    pub inspected_claimed_count: u16,
    pub same_host_current_boot_count: u16,
    pub same_host_prior_boot_count: u16,
    pub foreign_host_count: u16,
    pub stored_unproven_count: u16,
    pub current_context_unavailable_count: u16,
    pub has_more: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum RuleOutcomeError {
    #[error("engine session is closed")]
    Closed,
    #[error("the rule-outcome record version is unsupported")]
    InvalidRecordVersion,
    #[error("the rule-outcome cleanup session ID is invalid")]
    InvalidSessionId,
    #[error("the requested cleanup session does not exist")]
    SessionNotFound,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the rule-outcome query exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("durable rule-outcome evidence is corrupt")]
    CorruptData,
    #[error("durable rule-outcome evidence is unavailable")]
    Unavailable,
    #[error("rule-outcome state is unavailable")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum StorageThiefError {
    #[error("engine session is closed")]
    Closed,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the storage-thief query exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("durable storage-thief evidence is corrupt")]
    CorruptData,
    #[error("durable storage-thief evidence is unavailable")]
    Unavailable,
    #[error("storage-thief state is unavailable")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum RunningScanDebtCensusError {
    #[error("engine session is closed")]
    Closed,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the running-scan debt query exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("durable running-scan debt evidence is corrupt")]
    CorruptData,
    #[error("durable running-scan debt evidence is unavailable")]
    Unavailable,
    #[error("running-scan debt state is unavailable")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ClaimedRunningScanProvenanceCensusError {
    #[error("engine session is closed")]
    Closed,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the claimed running-scan provenance query exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("durable claimed running-scan provenance evidence is corrupt")]
    CorruptData,
    #[error("durable claimed running-scan provenance evidence is unavailable")]
    Unavailable,
    #[error("claimed running-scan provenance state is unavailable")]
    InternalState,
}

/// Immutable, path-free confirmation facts for clearing the exact current
/// cleanup-history graph. This record carries no row selector or cleanup
/// authority; only its opaque companion session can be consumed.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupHistoryClearPreviewInfo {
    pub record_version: u32,
    pub session_count: u64,
    pub oldest_started_at_unix_ms: i64,
    pub newest_started_at_unix_ms: i64,
    pub prepared_at_unix_ms: i64,
    pub expires_at_unix_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CleanupHistoryClearResult {
    pub record_version: u32,
    pub cleared_session_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupHistoryClearPreviewReleaseOutcome {
    Released,
    AlreadyUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum CleanupHistoryClearError {
    #[error("engine session is closed")]
    Closed,
    #[error("there is no cleanup history to clear")]
    NothingToClear,
    #[error("cleanup history includes unfinished or uncertain work")]
    ActiveCleanup,
    #[error("cleanup history changed after confirmation")]
    ChangedSincePreview,
    #[error("the cleanup-history clear preview expired")]
    PreviewExpired,
    #[error("the cleanup-history clear preview belongs to another engine")]
    WrongEngine,
    #[error("the cleanup-history clear preview was consumed or released")]
    PreviewUnavailable,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("cleanup-history clearing exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("durable cleanup history is corrupt")]
    CorruptData,
    #[error("the result of clearing cleanup history is unknown")]
    OutcomeUnknown,
    #[error("durable cleanup history is unavailable")]
    Unavailable,
    #[error("cleanup-history clearing state is unavailable")]
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
    pub newest_mtime: SnapshotNodeTimestamp,
    pub minimum_age_seconds: u64,
    pub minimum_age_nanoseconds: u32,
    pub warnings: Vec<CleanupWarning>,
    pub created_at: SnapshotNodeTimestamp,
    pub effective_expires_at: SnapshotNodeTimestamp,
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

/// Path-free terminal observation for one exact reviewed Rust-target cleanup.
/// The session identifier is correlation for durable history only and cannot
/// authorize, resume, or retry an effect.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RustTargetCleanupResult {
    pub record_version: u32,
    pub session_id: String,
    pub status: CleanupSessionStatus,
    pub removed_entries: u64,
    pub removed_logical_bytes: u64,
    pub verified_capacity_delta_bytes: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RustTargetCleanupTaskFailure {
    ParentReviewUnavailable,
    ReviewExpired,
    ChangedDuringReview,
    BudgetExceeded,
    Busy,
    UnsafeStorage,
    IncompatibleSchema,
    CorruptData,
    OutcomeUnknown,
    Unavailable,
    InternalState,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RustTargetCleanupPoll {
    pub record_version: u32,
    pub phase: TaskPhase,
    pub cancellation_requested: bool,
    pub revision: u64,
    pub failure: Option<RustTargetCleanupTaskFailure>,
    pub result: Option<RustTargetCleanupResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RustTargetCleanupCancelOutcome {
    CancelledBeforeStart,
    Requested,
    AlreadyRequested,
    AlreadyTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum RustTargetCleanupStartError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the supplied plan review belongs to another engine")]
    WrongEngine,
    #[error("the exact Rust-target plan review is no longer available")]
    ReviewUnavailable,
    #[error("the exact parent snapshot review is released or expired")]
    ParentReviewUnavailable,
    #[error("the exact Rust-target plan review expired")]
    ReviewExpired,
    #[error("the Rust-target plan evidence changed before execution")]
    ChangedDuringReview,
    #[error("cleanup was cancelled before durable execution began")]
    CancelledBeforeStart,
    #[error("the cleanup operation exceeded its bounded resource budget")]
    BudgetExceeded,
    #[error("the engine task queue is full")]
    QueueFull,
    #[error("another cleanup operation is active")]
    Busy,
    #[error("the cleanup store is unsafe")]
    UnsafeStorage,
    #[error("the cleanup schema is incompatible")]
    IncompatibleSchema,
    #[error("the cleanup journal is corrupt")]
    CorruptData,
    #[error("the cleanup operation outcome is unknown")]
    OutcomeUnknown,
    #[error("the cleanup operation is unavailable")]
    Unavailable,
    #[error("the cleanup state is internally unavailable")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum RustTargetCleanupTaskError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the cleanup task is no longer available")]
    TaskUnavailable,
    #[error("the retained task is not a permanent-safe cleanup")]
    WrongTaskKind,
    #[error("the cleanup task state is internally unavailable")]
    InternalState,
}

/// Path-free terminal observation for one exact reviewed Rust-target dry run.
///
/// The session identifier correlates read-only Cleanup History. It cannot
/// authorize, resume, retry, or identify a filesystem target.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RustTargetDryRunResult {
    pub record_version: u32,
    pub session_id: String,
    pub status: CleanupSessionStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RustTargetDryRunTaskFailure {
    ParentReviewUnavailable,
    ReviewExpired,
    ChangedDuringReview,
    BudgetExceeded,
    Busy,
    UnsafeStorage,
    IncompatibleSchema,
    CorruptData,
    HistoryUnresolved,
    Unavailable,
    InternalState,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RustTargetDryRunPoll {
    pub record_version: u32,
    pub phase: TaskPhase,
    pub cancellation_requested: bool,
    pub revision: u64,
    pub failure: Option<RustTargetDryRunTaskFailure>,
    pub result: Option<RustTargetDryRunResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RustTargetDryRunCancelOutcome {
    CancelledBeforeStart,
    Requested,
    AlreadyRequested,
    AlreadyTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum RustTargetDryRunStartError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the supplied plan review belongs to another engine")]
    WrongEngine,
    #[error("the exact Rust-target plan review is no longer available")]
    ReviewUnavailable,
    #[error("the exact parent snapshot review is released or expired")]
    ParentReviewUnavailable,
    #[error("the exact Rust-target plan review expired")]
    ReviewExpired,
    #[error("the Rust-target plan evidence changed before or during validation")]
    ChangedDuringReview,
    #[error("the dry run was cancelled before durable validation began")]
    CancelledBeforeStart,
    #[error("the dry run exceeded its bounded resource budget")]
    BudgetExceeded,
    #[error("the engine task queue is full")]
    QueueFull,
    #[error("another cleanup operation is active")]
    Busy,
    #[error("the dry-run history store is unsafe")]
    UnsafeStorage,
    #[error("the dry-run history schema is incompatible")]
    IncompatibleSchema,
    #[error("the dry-run history journal is corrupt")]
    CorruptData,
    #[error("the dry-run history outcome could not be reconciled")]
    HistoryUnresolved,
    #[error("the dry run is unavailable")]
    Unavailable,
    #[error("the dry-run state is internally unavailable")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum RustTargetDryRunTaskError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the dry-run task is no longer available")]
    TaskUnavailable,
    #[error("the retained task is not a Rust-target dry run")]
    WrongTaskKind,
    #[error("the dry-run task state is internally unavailable")]
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
    CandidateEvaluationRecovery,
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
    CandidateEvaluationRecoveryNone,
    CandidateEvaluationRecoveryRecovered,
    CandidateEvaluationRecoveryIncompatible,
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
/// Candidate-evaluation recovery uses primary after only for the recovered
/// candidate count. Scan and candidate identities never cross this boundary.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SnapshotDiffNodeSort {
    NameAscending,
    MagnitudeDescending,
    CurrentBytesDescending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SnapshotDiffChange {
    Added,
    Removed,
    Grew,
    Shrank,
    Unchanged,
    Replaced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SnapshotDiffDirection {
    Growth,
    Shrinkage,
    Unchanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotDiffValue {
    pub direction: SnapshotDiffDirection,
    pub magnitude_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotDiffInfo {
    pub record_version: u32,
    pub current_scan_id: String,
    pub baseline_scan_id: String,
    pub current_started_at_unix_ms: i64,
    pub current_completed_at_unix_ms: i64,
    pub baseline_started_at_unix_ms: i64,
    pub baseline_completed_at_unix_ms: i64,
    pub current_coverage: ScanCoverageSummary,
    pub baseline_coverage: ScanCoverageSummary,
    pub released: bool,
}

/// One union node from two retained historical snapshots. Its ID is opaque
/// outside the exact comparison session and grants no live-path authority.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotDiffNode {
    pub record_version: u32,
    pub id: u64,
    pub parent_id: Option<u64>,
    pub depth: u32,
    pub name: SnapshotNodeName,
    pub kind: SnapshotNodeKind,
    pub current_kind: Option<SnapshotNodeKind>,
    pub baseline_kind: Option<SnapshotNodeKind>,
    pub category: SnapshotStorageCategory,
    pub change: SnapshotDiffChange,
    pub logical_change: SnapshotDiffValue,
    pub current_logical_bytes: Option<u64>,
    pub baseline_logical_bytes: Option<u64>,
    pub current_allocated_bytes: Option<u64>,
    pub baseline_allocated_bytes: Option<u64>,
    pub allocated_change: Option<SnapshotDiffValue>,
    pub current_file_count: Option<u64>,
    pub baseline_file_count: Option<u64>,
    pub current_child_count: Option<u64>,
    pub baseline_child_count: Option<u64>,
    pub current_scan_flags: Option<SnapshotNodeScanFlags>,
    pub baseline_scan_flags: Option<SnapshotNodeScanFlags>,
    pub can_descend: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotDiffNodePage {
    pub record_version: u32,
    pub parent_id: u64,
    pub offset: u64,
    pub total_children: u64,
    pub has_more: bool,
    pub total_growth_bytes: u64,
    pub total_shrinkage_bytes: u64,
    pub unchanged_child_count: u64,
    pub replaced_child_count: u64,
    pub nodes: Vec<SnapshotDiffNode>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotDiffTreemapCell {
    pub record_version: u32,
    pub node: SnapshotDiffNode,
    pub magnitude_rank: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotDiffTreemap {
    pub record_version: u32,
    pub parent_id: u64,
    pub total_children: u64,
    pub changed_child_count: u64,
    pub total_growth_bytes: u64,
    pub total_shrinkage_bytes: u64,
    pub other_growth_child_count: u64,
    pub other_growth_bytes: u64,
    pub other_shrinkage_child_count: u64,
    pub other_shrinkage_bytes: u64,
    pub unchanged_child_count: u64,
    pub replaced_child_count: u64,
    pub cells: Vec<SnapshotDiffTreemapCell>,
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

/// Versioned request for one bounded, allocation-ranked set of regular files
/// from an exact retained snapshot directory. It grants no cleanup authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotICloudObservationSourceRequest {
    pub record_version: u32,
    pub scope_node_id: u64,
    pub max_results: u16,
}

/// One path-free historical file nominated for an explicit, read-only iCloud
/// metadata observation.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotICloudObservationTarget {
    pub record_version: u32,
    pub rank: u16,
    pub node: SnapshotNode,
    /// Root-to-parent historical name components, excluding the scan root.
    pub parent_context: Vec<SnapshotNodeName>,
    pub context_truncated: bool,
}

/// Exact traversal accounting plus a bounded deterministic projection. The
/// source does not establish provider identity, current allocation, or
/// reclaimable capacity.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotICloudObservationSource {
    pub record_version: u32,
    pub scan_id: String,
    pub scope_node_id: u64,
    pub requested_max_results: u16,
    pub visited_node_count: u64,
    pub total_ranked_files: u64,
    pub has_more: bool,
    pub targets: Vec<SnapshotICloudObservationTarget>,
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

/// Versioned, path-free selection for one read-only iCloud metadata probe.
///
/// The node ID is meaningful only inside the retained review. It is not a
/// filesystem identity, cleanup candidate, approval, or effect capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ICloudLocalCopyProbeSelection {
    pub record_version: u32,
    pub node_id: u64,
}

/// The fixed provider selected by Rust for this first cloud policy revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ICloudLocalCopyProvider {
    ICloudDrive,
}

/// The no-follow item kind revalidated by Rust before metadata is requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ICloudLocalCopyItemKind {
    RegularFile,
}

/// A tri-state Foundation Boolean. Missing values stay unknown and therefore
/// cannot accidentally become favorable evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ICloudBooleanState {
    True,
    False,
    Unknown,
}

/// Whether a requested Foundation error resource value was present.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ICloudErrorState {
    Absent,
    Present,
    Unknown,
}

/// Foundation's bounded local-copy download status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ICloudLocalCopyState {
    Current,
    Stale,
    NotDownloaded,
    Unknown,
}

/// Whether one account/container/item identity prerequisite stayed stable
/// across the platform adapter's bracketed read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ICloudIdentityFactState {
    Stable,
    Unavailable,
    ChangedDuringRead,
    Unsupported,
}

/// Raw facts returned synchronously for the exact path consumed from a
/// core-issued probe request. There is intentionally no path, provider, item
/// kind, allocation, timestamp, eligibility flag, or cleanup authority here.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ICloudLocalCopyRawFacts {
    pub record_version: u32,
    pub ubiquitous: ICloudBooleanState,
    pub uploaded: ICloudBooleanState,
    pub uploading: ICloudBooleanState,
    pub upload_error: ICloudErrorState,
    pub unresolved_conflicts: ICloudBooleanState,
    pub local_copy_state: ICloudLocalCopyState,
    pub download_requested: ICloudBooleanState,
    pub downloading: ICloudBooleanState,
    pub download_error: ICloudErrorState,
    pub excluded_from_sync: ICloudBooleanState,
    pub account_identity: ICloudIdentityFactState,
    pub container_identity: ICloudIdentityFactState,
    pub item_generation: ICloudIdentityFactState,
    pub file_version: ICloudIdentityFactState,
    pub shared: ICloudBooleanState,
    pub sync_paused: ICloudBooleanState,
}

/// Synchronous result from the narrow Foundation metadata adapter.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ICloudLocalCopyMetadataResult {
    Observed { facts: ICloudLocalCopyRawFacts },
    Unsupported,
    Failed,
}

/// Fixed, path-free reasons an observation cannot support discovery of an
/// iCloud local-copy eviction opportunity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ICloudLocalCopyBlockReason {
    UnsupportedItemKind,
    UbiquityUnknown,
    NotUbiquitous,
    UploadStateUnknown,
    UploadIncomplete,
    UploadActivityUnknown,
    UploadInProgress,
    UploadErrorUnknown,
    UploadErrorPresent,
    ConflictStateUnknown,
    UnresolvedConflicts,
    LocalCopyStateUnknown,
    StaleLocalCopy,
    NoLocalCopy,
    DownloadRequestUnknown,
    DownloadRequested,
    DownloadActivityUnknown,
    DownloadInProgress,
    DownloadErrorUnknown,
    DownloadErrorPresent,
    SyncExclusionUnknown,
    ExcludedFromSync,
    AllocationUnknown,
    NoLocalAllocation,
    InvalidObservationTime,
}

/// Fixed, path-free reasons the observation cannot become durable identity
/// evidence for a future candidate-admission boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ICloudIdentityBlockReason {
    AccountIdentityUnavailable,
    AccountIdentityChanged,
    AccountIdentityUnsupported,
    ContainerIdentityUnavailable,
    ContainerIdentityChanged,
    ContainerIdentityUnsupported,
    ItemGenerationUnavailable,
    ItemGenerationChanged,
    ItemGenerationUnsupported,
    FileVersionUnavailable,
    FileVersionChanged,
    FileVersionUnsupported,
    SharedStateUnknown,
    SharedItem,
    SyncPausedStateUnknown,
    SyncPaused,
}

/// Deterministic, path-free projection of one point-in-time observation.
///
/// `is_eligible_observation` is discovery evidence only. It cannot be used as
/// a candidate, cleanup plan, approval, journal claim, or eviction effect.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ICloudLocalCopyAssessment {
    pub record_version: u32,
    pub provider: ICloudLocalCopyProvider,
    pub item_kind: ICloudLocalCopyItemKind,
    pub local_allocated_bytes: u64,
    pub observed_at_unix_ms: i64,
    pub ubiquitous: ICloudBooleanState,
    pub uploaded: ICloudBooleanState,
    pub uploading: ICloudBooleanState,
    pub upload_error: ICloudErrorState,
    pub unresolved_conflicts: ICloudBooleanState,
    pub local_copy_state: ICloudLocalCopyState,
    pub download_requested: ICloudBooleanState,
    pub downloading: ICloudBooleanState,
    pub download_error: ICloudErrorState,
    pub excluded_from_sync: ICloudBooleanState,
    pub account_identity: ICloudIdentityFactState,
    pub container_identity: ICloudIdentityFactState,
    pub item_generation: ICloudIdentityFactState,
    pub file_version: ICloudIdentityFactState,
    pub shared: ICloudBooleanState,
    pub sync_paused: ICloudBooleanState,
    pub is_eligible_observation: bool,
    pub blockers: Vec<ICloudLocalCopyBlockReason>,
    pub is_identity_ready: bool,
    pub identity_blockers: Vec<ICloudIdentityBlockReason>,
}

/// The only path payload a read-only iCloud metadata callback may receive.
///
/// There is intentionally no UniFFI constructor. Rust creates this object
/// only after resolving and revalidating an exact retained Explorer file. Its
/// bytes must be consumed exactly once during the synchronous callback.
#[derive(uniffi::Object)]
pub struct ICloudLocalCopyProbeRequest {
    state: Mutex<Option<ICloudLocalCopyProbeRequestState>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ICloudLocalCopyProbeRequestState {
    record_version: u32,
    path_encoding: SnapshotNameEncoding,
    absolute_path_bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ICloudLocalCopyProbeRequestError {
    #[error("the iCloud metadata request is no longer available")]
    Consumed,
    #[error("the iCloud metadata request contains an invalid path")]
    InvalidPath,
    #[error("the iCloud metadata request state is unavailable")]
    InternalState,
}

#[uniffi::export]
impl ICloudLocalCopyProbeRequest {
    pub fn record_version(&self) -> Result<u32, ICloudLocalCopyProbeRequestError> {
        self.with_state(|state| state.record_version)
    }

    pub fn path_encoding(&self) -> Result<SnapshotNameEncoding, ICloudLocalCopyProbeRequestError> {
        self.with_state(|state| state.path_encoding)
    }

    /// Consume the exact path bytes once. The callback must use this path for
    /// the returned facts and must not retain, log, retry, or return it.
    pub fn take_path_bytes(&self) -> Result<Vec<u8>, ICloudLocalCopyProbeRequestError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ICloudLocalCopyProbeRequestError::InternalState)?;
        let state = state
            .take()
            .ok_or(ICloudLocalCopyProbeRequestError::Consumed)?;
        if state.absolute_path_bytes.is_empty() {
            return Err(ICloudLocalCopyProbeRequestError::InvalidPath);
        }
        Ok(state.absolute_path_bytes)
    }
}

impl ICloudLocalCopyProbeRequest {
    fn from_core(absolute_path_bytes: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(Some(ICloudLocalCopyProbeRequestState {
                record_version: FFI_RECORD_VERSION,
                path_encoding: SnapshotNameEncoding::UnixBytes,
                absolute_path_bytes,
            })),
        })
    }

    fn with_state<T>(
        &self,
        operation: impl FnOnce(&ICloudLocalCopyProbeRequestState) -> T,
    ) -> Result<T, ICloudLocalCopyProbeRequestError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ICloudLocalCopyProbeRequestError::InternalState)?;
        let state = state
            .as_ref()
            .ok_or(ICloudLocalCopyProbeRequestError::Consumed)?;
        Ok(operation(state))
    }

    fn is_consumed(&self) -> Result<bool, ICloudLocalCopyProbeRequestError> {
        self.state
            .lock()
            .map(|state| state.is_none())
            .map_err(|_| ICloudLocalCopyProbeRequestError::InternalState)
    }

    #[cfg(test)]
    fn for_test(
        record_version: u32,
        path_encoding: SnapshotNameEncoding,
        absolute_path_bytes: Vec<u8>,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(Some(ICloudLocalCopyProbeRequestState {
                record_version,
                path_encoding,
                absolute_path_bytes,
            })),
        })
    }
}

/// Read-only platform adapter. It may report only bounded metadata facts for
/// the consumed Rust-selected path and cannot choose a target or perform an
/// eviction.
#[uniffi::export(callback_interface)]
pub trait ICloudLocalCopyMetadataDriver: Send + Sync {
    fn read_metadata(
        &self,
        request: Arc<ICloudLocalCopyProbeRequest>,
    ) -> ICloudLocalCopyMetadataResult;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ICloudLocalCopyProbeError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the iCloud probe record version is unsupported")]
    InvalidRecordVersion,
    #[error("the retained Explorer review belongs to a different engine")]
    WrongReview,
    #[error("the selected Explorer node is not an eligible probe target")]
    InvalidTarget,
    #[error("the selected Explorer node changed since snapshot capture")]
    ChangedSinceSnapshot,
    #[error("the retained Explorer review is unavailable")]
    ReviewUnavailable,
    #[error("the platform does not support the iCloud metadata probe")]
    PlatformUnsupported,
    #[error("the iCloud metadata probe failed closed")]
    PlatformFailed,
    #[error("the iCloud probe state is unavailable")]
    InternalState,
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

enum CleanupHistoryClearPreviewState {
    Available(Box<CoreCleanupHistoryClearPreview>),
    Consumed,
    Released,
}

enum RustTargetPlanReviewState {
    Available(Box<CoreRustTargetPlanReview>),
    Inspecting,
    ReleasePending,
    Consumed,
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

/// Engine-bound, consume-once reviewed-plan capability. Information remains a
/// display observation; only the owning `DuxEngine` can irreversibly consume
/// the retained core plan into its fixed permanent-safe cleanup task.
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
        let release_pending = matches!(
            *state,
            RustTargetPlanReviewState::ReleasePending | RustTargetPlanReviewState::Consumed
        );
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
            RustTargetPlanReviewState::ReleasePending
            | RustTargetPlanReviewState::Consumed
            | RustTargetPlanReviewState::Released => Ok(false),
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

    /// Irreversibly consume this FFI review for one correct-engine cleanup
    /// start attempt. If an information read already owns the core review, the
    /// pending transition makes that read release it instead of restoring it.
    fn take_for_cleanup_start(
        &self,
    ) -> Result<Box<CoreRustTargetPlanReview>, RustTargetCleanupStartError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RustTargetCleanupStartError::InternalState)?;
        match &*state {
            RustTargetPlanReviewState::Available(_) => {
                let RustTargetPlanReviewState::Available(review) =
                    std::mem::replace(&mut *state, RustTargetPlanReviewState::Consumed)
                else {
                    unreachable!("available state was just matched")
                };
                Ok(review)
            }
            RustTargetPlanReviewState::Inspecting => {
                *state = RustTargetPlanReviewState::ReleasePending;
                Err(RustTargetCleanupStartError::ReviewUnavailable)
            }
            RustTargetPlanReviewState::ReleasePending
            | RustTargetPlanReviewState::Consumed
            | RustTargetPlanReviewState::Released => {
                Err(RustTargetCleanupStartError::ReviewUnavailable)
            }
        }
    }

    /// Irreversibly consume this FFI review for one correct-engine dry-run
    /// start attempt. This remains a separate authority edge from permanent
    /// cleanup even though both consume the same opaque reviewed plan.
    fn take_for_dry_run_start(
        &self,
    ) -> Result<Box<CoreRustTargetPlanReview>, RustTargetDryRunStartError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RustTargetDryRunStartError::InternalState)?;
        match &*state {
            RustTargetPlanReviewState::Available(_) => {
                let RustTargetPlanReviewState::Available(review) =
                    std::mem::replace(&mut *state, RustTargetPlanReviewState::Consumed)
                else {
                    unreachable!("available state was just matched")
                };
                Ok(review)
            }
            RustTargetPlanReviewState::Inspecting => {
                *state = RustTargetPlanReviewState::ReleasePending;
                Err(RustTargetDryRunStartError::ReviewUnavailable)
            }
            RustTargetPlanReviewState::ReleasePending
            | RustTargetPlanReviewState::Consumed
            | RustTargetPlanReviewState::Released => {
                Err(RustTargetDryRunStartError::ReviewUnavailable)
            }
        }
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
            RustTargetPlanReviewState::ReleasePending
            | RustTargetPlanReviewState::Consumed
            | RustTargetPlanReviewState::Released => {
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

/// Engine-bound, consume-once confirmation for deleting only DUX's local
/// terminal cleanup-history metadata.
#[derive(uniffi::Object)]
pub struct CleanupHistoryClearPreviewSession {
    state: Mutex<CleanupHistoryClearPreviewState>,
    info: CleanupHistoryClearPreviewInfo,
    engine_closed: Arc<AtomicBool>,
}

#[uniffi::export]
impl CleanupHistoryClearPreviewSession {
    pub fn info(&self) -> Result<CleanupHistoryClearPreviewInfo, CleanupHistoryClearError> {
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(CleanupHistoryClearError::Closed);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| CleanupHistoryClearError::InternalState)?;
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(CleanupHistoryClearError::Closed);
        }
        match &*state {
            CleanupHistoryClearPreviewState::Available(preview) => {
                preview.info().map_err(map_cleanup_history_clear_error)?;
                if self.engine_closed.load(Ordering::Acquire) {
                    return Err(CleanupHistoryClearError::Closed);
                }
                Ok(self.info.clone())
            }
            CleanupHistoryClearPreviewState::Consumed
            | CleanupHistoryClearPreviewState::Released => {
                Err(CleanupHistoryClearError::PreviewUnavailable)
            }
        }
    }

    pub fn release(
        &self,
    ) -> Result<CleanupHistoryClearPreviewReleaseOutcome, CleanupHistoryClearError> {
        self.release_inner()
    }
}

impl CleanupHistoryClearPreviewSession {
    fn is_available(&self) -> Result<bool, CleanupHistoryClearError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| CleanupHistoryClearError::InternalState)?;
        match &*state {
            CleanupHistoryClearPreviewState::Available(preview) => match preview.info() {
                Ok(_) => Ok(true),
                Err(CoreCleanupHistoryClearError::PreviewExpired) => {
                    *state = CleanupHistoryClearPreviewState::Released;
                    Ok(false)
                }
                Err(error) => Err(map_cleanup_history_clear_error(error)),
            },
            CleanupHistoryClearPreviewState::Consumed
            | CleanupHistoryClearPreviewState::Released => Ok(false),
        }
    }

    fn take_for_clear(&self) -> Result<CoreCleanupHistoryClearPreview, CleanupHistoryClearError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| CleanupHistoryClearError::InternalState)?;
        match std::mem::replace(&mut *state, CleanupHistoryClearPreviewState::Consumed) {
            CleanupHistoryClearPreviewState::Available(preview) => Ok(*preview),
            prior @ (CleanupHistoryClearPreviewState::Consumed
            | CleanupHistoryClearPreviewState::Released) => {
                *state = prior;
                Err(CleanupHistoryClearError::PreviewUnavailable)
            }
        }
    }

    fn release_inner(
        &self,
    ) -> Result<CleanupHistoryClearPreviewReleaseOutcome, CleanupHistoryClearError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| CleanupHistoryClearError::InternalState)?;
        match std::mem::replace(&mut *state, CleanupHistoryClearPreviewState::Released) {
            CleanupHistoryClearPreviewState::Available(_) => {
                Ok(CleanupHistoryClearPreviewReleaseOutcome::Released)
            }
            prior @ (CleanupHistoryClearPreviewState::Consumed
            | CleanupHistoryClearPreviewState::Released) => {
                *state = prior;
                Ok(CleanupHistoryClearPreviewReleaseOutcome::AlreadyUnavailable)
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

    pub fn icloud_observation_source(
        &self,
        request: SnapshotICloudObservationSourceRequest,
    ) -> Result<SnapshotICloudObservationSource, EngineError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(EngineError::InvalidSnapshotICloudObservationSourceRequest);
        }
        self.with_open_session(|session| {
            let scan_id = session.scan_id().as_str().to_owned();
            let source = session
                .icloud_observation_source(request.scope_node_id, request.max_results)
                .map_err(map_review_error)?;
            Ok(project_snapshot_icloud_observation_source(scan_id, source))
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

#[derive(uniffi::Object)]
pub struct SnapshotDiffReviewSession {
    inner: Mutex<CoreSnapshotDiffReviewSession>,
    parent: Weak<SnapshotReviewSession>,
    engine_closed: Arc<AtomicBool>,
}

#[uniffi::export]
impl SnapshotDiffReviewSession {
    pub fn info(&self) -> Result<SnapshotDiffInfo, EngineError> {
        {
            let diff = self.inner.lock().map_err(|_| EngineError::InternalState)?;
            if let Some(info) = diff.released_info() {
                return project_snapshot_diff_info(info, true);
            }
        }
        self.with_open_pair(|diff, current| {
            diff.info(current)
                .map_err(map_review_error)
                .and_then(|info| project_snapshot_diff_info(info, false))
        })
    }

    pub fn renew(&self) -> Result<SnapshotDiffInfo, EngineError> {
        self.with_open_pair(|diff, current| {
            diff.renew(current).map_err(map_review_error)?;
            diff.info(current)
                .map_err(map_review_error)
                .and_then(|info| project_snapshot_diff_info(info, false))
        })
    }

    pub fn release(&self) -> Result<ReviewReleaseOutcome, EngineError> {
        self.release_inner()
    }

    pub fn root_node(&self) -> Result<SnapshotDiffNode, EngineError> {
        self.with_open_pair(|diff, current| {
            diff.root_node(current)
                .map(project_snapshot_diff_node)
                .map_err(map_review_error)
        })
    }

    pub fn child_nodes(
        &self,
        parent_id: u64,
        sort: SnapshotDiffNodeSort,
        offset: u64,
        limit: u16,
    ) -> Result<SnapshotDiffNodePage, EngineError> {
        self.with_open_pair(|diff, current| {
            diff.child_nodes(
                current,
                parent_id,
                map_snapshot_diff_node_sort(sort),
                offset,
                limit,
            )
            .map(project_snapshot_diff_node_page)
            .map_err(map_review_error)
        })
    }

    pub fn treemap(
        &self,
        parent_id: u64,
        max_cells: u16,
    ) -> Result<SnapshotDiffTreemap, EngineError> {
        self.with_open_pair(|diff, current| {
            diff.treemap(current, parent_id, max_cells)
                .map(project_snapshot_diff_treemap)
                .map_err(map_review_error)
        })
    }
}

impl SnapshotDiffReviewSession {
    fn with_open_pair<T>(
        &self,
        operation: impl FnOnce(
            &mut CoreSnapshotDiffReviewSession,
            &mut CoreReviewSession,
        ) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(EngineError::Closed);
        }
        let parent = self
            .parent
            .upgrade()
            .ok_or(EngineError::WrongParentReview)?;
        let mut current = parent
            .inner
            .lock()
            .map_err(|_| EngineError::InternalState)?;
        let mut diff = self.inner.lock().map_err(|_| EngineError::InternalState)?;
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(EngineError::Closed);
        }
        operation(&mut diff, &mut current)
    }

    fn release_inner(&self) -> Result<ReviewReleaseOutcome, EngineError> {
        let mut diff = self.inner.lock().map_err(|_| EngineError::InternalState)?;
        match diff.release().map_err(map_review_error)? {
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
                | CoreTaskEventKind::CandidateEvaluationRecoveryMaintenanceApplying
                | CoreTaskEventKind::CandidateEvaluationRecoveryMaintenanceFinished { .. }
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
        | CoreTaskEventKind::CandidateEvaluationRecoveryMaintenanceApplying
        | CoreTaskEventKind::CandidateEvaluationRecoveryMaintenanceFinished { .. }
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

/// Opaque observer for one engine-owned permanent-safe cleanup task. Dropping
/// this object does not cancel the task; explicit cancellation or engine
/// shutdown are the only cancellation routes.
#[derive(uniffi::Object)]
pub struct RustTargetCleanupTask {
    engine: EngineHandle,
    id: TaskId,
}

#[uniffi::export]
impl RustTargetCleanupTask {
    pub fn poll(&self) -> Result<RustTargetCleanupPoll, RustTargetCleanupTaskError> {
        let snapshot = self
            .engine
            .task_snapshot(self.id)
            .map_err(map_rust_target_cleanup_task_access_error)?;
        if snapshot.kind != CoreTaskKind::PermanentSafeCleanup {
            return Err(RustTargetCleanupTaskError::WrongTaskKind);
        }
        let result = if snapshot.result_available {
            self.engine
                .permanent_safe_cleanup_result(self.id)
                .map_err(map_rust_target_cleanup_task_access_error)?
                .map(|result| project_rust_target_cleanup_result(&result))
                .transpose()?
        } else {
            None
        };
        let failure = snapshot
            .failure
            .map(map_rust_target_cleanup_task_failure)
            .transpose()?;
        validate_rust_target_cleanup_poll_shape(snapshot.phase, failure, result.as_ref())?;
        Ok(RustTargetCleanupPoll {
            record_version: FFI_RECORD_VERSION,
            phase: map_phase(snapshot.phase),
            cancellation_requested: snapshot.cancellation_requested,
            revision: snapshot.revision,
            failure,
            result,
        })
    }

    pub fn cancel(&self) -> Result<RustTargetCleanupCancelOutcome, RustTargetCleanupTaskError> {
        let outcome = self
            .engine
            .cancel_task(self.id)
            .map_err(map_rust_target_cleanup_task_access_error)?;
        Ok(map_rust_target_cleanup_cancel_outcome(outcome))
    }
}

/// Opaque observer for one engine-owned, effect-free Rust-target dry-run task.
/// Dropping this object does not cancel the task.
#[derive(uniffi::Object)]
pub struct RustTargetDryRunTask {
    engine: EngineHandle,
    id: TaskId,
}

#[uniffi::export]
impl RustTargetDryRunTask {
    pub fn poll(&self) -> Result<RustTargetDryRunPoll, RustTargetDryRunTaskError> {
        let snapshot = self
            .engine
            .task_snapshot(self.id)
            .map_err(map_rust_target_dry_run_task_access_error)?;
        if snapshot.kind != CoreTaskKind::RustTargetDryRun {
            return Err(RustTargetDryRunTaskError::WrongTaskKind);
        }
        let result = if snapshot.result_available {
            self.engine
                .rust_target_dry_run_result(self.id)
                .map_err(map_rust_target_dry_run_task_access_error)?
                .map(|result| project_rust_target_dry_run_result(&result))
                .transpose()?
        } else {
            None
        };
        let failure = snapshot
            .failure
            .map(map_rust_target_dry_run_task_failure)
            .transpose()?;
        validate_rust_target_dry_run_poll_shape(snapshot.phase, failure, result.as_ref())?;
        Ok(RustTargetDryRunPoll {
            record_version: FFI_RECORD_VERSION,
            phase: map_phase(snapshot.phase),
            cancellation_requested: snapshot.cancellation_requested,
            revision: snapshot.revision,
            failure,
            result,
        })
    }

    pub fn cancel(&self) -> Result<RustTargetDryRunCancelOutcome, RustTargetDryRunTaskError> {
        let outcome = self
            .engine
            .cancel_task(self.id)
            .map_err(map_rust_target_dry_run_task_access_error)?;
        Ok(map_rust_target_dry_run_cancel_outcome(outcome))
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
        if !maintenance_core_kind_matches(self.kind, snapshot.kind) {
            return Err(EngineError::InternalState);
        }
        let result = if snapshot.result_available {
            maintenance_result(&self.engine, self.id, self.kind)?
        } else {
            None
        };
        let phase = map_phase(snapshot.phase);
        let failure = snapshot.failure.map(map_failure);
        validate_maintenance_poll_shape(self.kind, phase, failure, result.as_ref())?;
        Ok(MaintenancePoll {
            record_version: FFI_RECORD_VERSION,
            kind: self.kind,
            phase,
            cancellation_requested: snapshot.cancellation_requested,
            revision: snapshot.revision,
            failure,
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
    diff_reviews: Arc<Mutex<Vec<Weak<SnapshotDiffReviewSession>>>>,
    direct_cargo_previews: Arc<Mutex<Vec<Weak<DirectCargoEnrollmentPreviewSession>>>>,
    cleanup_history_clear_previews: Arc<Mutex<Vec<Weak<CleanupHistoryClearPreviewSession>>>>,
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
            diff_reviews: Arc::new(Mutex::new(Vec::new())),
            direct_cargo_previews: Arc::new(Mutex::new(Vec::new())),
            cleanup_history_clear_previews: Arc::new(Mutex::new(Vec::new())),
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

    /// Return a bounded newest-first pressure history as of one accepted
    /// capacity anchor. The response is observation-only telemetry.
    pub fn get_pressure_episode_history(
        &self,
        request: PressureEpisodeHistoryRequest,
    ) -> Result<PressureEpisodeHistoryStatus, EngineError> {
        let limit = usize::from(request.limit);
        if request.record_version != FFI_RECORD_VERSION
            || !(1..=dux_core::MAX_PRESSURE_EPISODE_HISTORY_LIMIT).contains(&limit)
        {
            return Err(EngineError::InvalidPressureEpisodeRequest);
        }
        let stable_volume_id = parse_macos_volume_id(Some(request.stable_volume_id))
            .map_err(|_| EngineError::InvalidPressureEpisodeRequest)?
            .ok_or(EngineError::InvalidPressureEpisodeRequest)?;
        let anchor_at = unix_ms_to_system_time(request.anchor_at_unix_ms)
            .map_err(|_| EngineError::InvalidPressureEpisodeRequest)?;
        self.with_engine(|engine| {
            let history = engine
                .pressure_episode_history(&stable_volume_id, anchor_at, limit)
                .map_err(map_pressure_episode_history_error)?;
            pressure_episode_history_status(history)
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

    /// Load the path-free global permanent-cleanup opt-in. The disabled default
    /// can only deny effects; this cannot create a plan or authorize a target.
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

    /// Load the bounded project-root registry used only by read-only discovery.
    pub fn get_configured_project_roots(
        &self,
    ) -> Result<ConfiguredProjectRootsStatus, ConfiguredProjectRootsError> {
        self.with_configured_project_roots_engine(|engine| {
            engine
                .configured_project_roots()
                .map_err(map_configured_project_roots_error)
                .and_then(configured_project_roots_status)
        })
    }

    /// Replace the complete configured-project-root registry. The roots grant
    /// discovery scope only; this endpoint starts no scan or cleanup.
    pub fn set_configured_project_roots(
        &self,
        input: ConfiguredProjectRootsInput,
    ) -> Result<ConfiguredProjectRootsUpdate, ConfiguredProjectRootsError> {
        let roots = decode_configured_project_roots_input(input)?;
        self.with_configured_project_roots_engine(|engine| {
            engine
                .set_configured_project_roots(roots)
                .map_err(map_configured_project_roots_error)
                .and_then(configured_project_roots_update)
        })
    }

    /// Restore the empty versioned project-root default without touching any
    /// project or filesystem content.
    pub fn reset_configured_project_roots(
        &self,
    ) -> Result<ConfiguredProjectRootsUpdate, ConfiguredProjectRootsError> {
        self.with_configured_project_roots_engine(|engine| {
            engine
                .reset_configured_project_roots()
                .map_err(map_configured_project_roots_error)
                .and_then(configured_project_roots_update)
        })
    }

    /// Admit one bounded read-only scan selected exclusively from Rust's
    /// configured-root registry at an exact accepted low-space observation.
    pub fn start_targeted_project_scan(
        &self,
        request: TargetedProjectScanRequest,
    ) -> Result<TargetedProjectScanAdmission, TargetedProjectScanError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(TargetedProjectScanError::InvalidRecordVersion);
        }
        let volume_id = parse_targeted_project_scan_volume_id(&request.stable_volume_id)?;
        let capacity_anchor = targeted_project_scan_time(request.capacity_anchor_unix_ms, false)?;
        let expected_catalog_digest = request
            .expected_root_catalog_digest_sha256
            .map(|digest| {
                <[u8; 32]>::try_from(digest).map_err(|_| TargetedProjectScanError::InvalidCatalog)
            })
            .transpose()?;
        if request.selected_root_ordinal > 0 && expected_catalog_digest.is_none() {
            return Err(TargetedProjectScanError::InvalidCatalog);
        }
        self.with_targeted_project_scan_engine(|engine| {
            let admission = engine
                .start_targeted_reclaim_scan(
                    &volume_id,
                    capacity_anchor,
                    request.selected_root_ordinal,
                    request.expected_configured_roots_revision,
                    expected_catalog_digest,
                )
                .map_err(map_targeted_project_scan_error)?;
            targeted_project_scan_admission(
                engine,
                admission,
                &volume_id,
                capacity_anchor,
                request.selected_root_ordinal,
                request.expected_configured_roots_revision,
                expected_catalog_digest,
            )
        })
    }

    /// Revalidate the path-free pressure and registry context after a bounded
    /// configured-root pass. This starts no work and exposes no path.
    pub fn validate_targeted_project_scan_context(
        &self,
        request: TargetedProjectScanCheckpointRequest,
    ) -> Result<TargetedProjectScanCheckpoint, TargetedProjectScanError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(TargetedProjectScanError::InvalidRecordVersion);
        }
        let expected_pressure =
            core_targeted_project_scan_pressure_context(request.expected_pressure)?;
        let expected_catalog = core_targeted_reclaim_root_catalog(request.expected_root_catalog)?;
        self.with_targeted_project_scan_engine(|engine| {
            let checkpoint = engine
                .validate_targeted_reclaim_scan_context(&expected_pressure, &expected_catalog)
                .map_err(map_targeted_project_scan_error)?;
            targeted_project_scan_checkpoint(checkpoint, &expected_pressure, &expected_catalog)
        })
    }

    /// Atomically finalize the exact Critical targeted pass into a bounded,
    /// path-free recovery ordering. This starts no scan and exposes no target,
    /// candidate identifier, plan, approval, action, or effect authority.
    pub fn finalize_emergency_recovery(
        &self,
        request: EmergencyRecoveryRequest,
    ) -> Result<EmergencyRecoveryOrdering, EmergencyRecoveryError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(EmergencyRecoveryError::InvalidRecordVersion);
        }
        let expected_pressure =
            core_targeted_project_scan_pressure_context(request.expected_pressure)
                .map_err(map_targeted_project_scan_request_to_emergency_recovery_error)?;
        let expected_catalog = core_targeted_reclaim_root_catalog(request.expected_root_catalog)
            .map_err(map_targeted_project_scan_request_to_emergency_recovery_error)?;
        self.with_emergency_recovery_engine(|engine| {
            let ordering = engine
                .finalize_emergency_recovery(&expected_pressure, &expected_catalog)
                .map_err(map_emergency_recovery_error)?;
            emergency_recovery_ordering(ordering, &expected_pressure, &expected_catalog)
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

    /// Irreversibly consume one exact, engine-bound reviewed plan and start
    /// core's permanent-safe task. This is the only FFI approval edge: no
    /// caller path, identifier, timestamp, Boolean, callback, or retry token
    /// participates in execution admission.
    pub fn start_permanent_safe_cleanup(
        &self,
        review: Arc<RustTargetPlanReviewSession>,
    ) -> Result<Arc<RustTargetCleanupTask>, RustTargetCleanupStartError> {
        self.start_permanent_safe_cleanup_with(review, |engine, review| {
            engine
                .start_permanent_safe_cleanup(review)
                .map_err(|failure| {
                    let error = failure.error();
                    (error, Box::new(failure.into_review()))
                })
        })
    }

    /// Consume one exact, engine-bound reviewed plan for an effect-free dry
    /// run. No path, effect witness, or permanent-cleanup task is exposed
    /// across this boundary.
    pub fn start_rust_target_dry_run(
        &self,
        review: Arc<RustTargetPlanReviewSession>,
    ) -> Result<Arc<RustTargetDryRunTask>, RustTargetDryRunStartError> {
        self.start_rust_target_dry_run_with(review, |engine, review| {
            engine.start_rust_target_dry_run(review).map_err(|failure| {
                let error = failure.error();
                (error, Box::new(failure.into_review()))
            })
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

    /// Derive a bounded, path-free outcome for every item in one exact cleanup
    /// session. This is historical presentation data only and cannot resume,
    /// approve, schedule, or execute cleanup.
    pub fn rule_outcomes_for_cleanup_session(
        &self,
        request: CleanupSessionHistoryRequest,
    ) -> Result<RuleOutcomeBatch, RuleOutcomeError> {
        if request.record_version != FFI_RECORD_VERSION {
            return Err(RuleOutcomeError::InvalidRecordVersion);
        }
        let requested_session_id = request.session_id;
        let session_id = CoreCleanupSessionId::from_stable_str(requested_session_id.clone())
            .ok_or(RuleOutcomeError::InvalidSessionId)?;
        self.with_rule_outcome_engine(|engine| {
            let batch = engine
                .rule_outcomes_for_cleanup_session(&session_id)
                .map_err(map_rule_outcome_error)?;
            rule_outcome_batch(batch, &requested_session_id)
        })
    }

    /// Return a bounded, read-only ranking of deterministic rule IDs with
    /// confirmed zero-to-nonzero regrowth observations.
    pub fn recurring_storage_thieves(&self) -> Result<StorageThiefRanking, StorageThiefError> {
        self.with_storage_thief_engine(|engine| {
            let ranking = engine
                .recurring_storage_thieves()
                .map_err(map_storage_thief_error)?;
            storage_thief_ranking(ranking)
        })
    }

    /// Return one bounded diagnostic census of running rows without process
    /// claims. This performs no liveness probe and cannot mutate storage.
    pub fn running_scan_debt_census(
        &self,
    ) -> Result<RunningScanDebtCensus, RunningScanDebtCensusError> {
        self.with_running_scan_debt_engine(|engine| {
            let census = engine
                .running_scan_debt_census()
                .map_err(map_running_scan_debt_census_error)?;
            running_scan_debt_census(census)
        })
    }

    /// Return one bounded, path-free census of provenance relationships for
    /// claimed running scan rows. This performs no liveness probe or mutation
    /// and exposes no claim identity, digest, scope, owner, PID, or timestamp.
    pub fn claimed_running_scan_provenance_census(
        &self,
    ) -> Result<ClaimedRunningScanProvenanceCensus, ClaimedRunningScanProvenanceCensusError> {
        self.with_claimed_running_scan_provenance_engine(|engine| {
            let census = engine
                .claimed_running_scan_provenance_census()
                .map_err(map_claimed_running_scan_provenance_census_error)?;
            claimed_running_scan_provenance_census(census)
        })
    }

    /// Prepare one path-free, short-lived confirmation for clearing the exact
    /// current terminal cleanup-history graph.
    pub fn prepare_cleanup_history_clear(
        &self,
    ) -> Result<Arc<CleanupHistoryClearPreviewSession>, CleanupHistoryClearError> {
        let state = self
            .state
            .lock()
            .map_err(|_| CleanupHistoryClearError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(CleanupHistoryClearError::Closed);
        };
        if self.closed.load(Ordering::Acquire) {
            return Err(CleanupHistoryClearError::Closed);
        }
        self.ensure_cleanup_history_clear_preview_capacity()?;
        let preview = engine
            .prepare_cleanup_history_clear()
            .map_err(map_cleanup_history_clear_error)?;
        let info = preview
            .info()
            .map_err(map_cleanup_history_clear_error)
            .and_then(cleanup_history_clear_preview_info)?;
        self.register_cleanup_history_clear_preview(preview, info)
    }

    /// Consume one confirmation from this exact engine. Consumption occurs
    /// before the core mutation is called and is never restored after any
    /// result.
    pub fn clear_cleanup_history(
        &self,
        preview: Arc<CleanupHistoryClearPreviewSession>,
    ) -> Result<CleanupHistoryClearResult, CleanupHistoryClearError> {
        let state = self
            .state
            .lock()
            .map_err(|_| CleanupHistoryClearError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(CleanupHistoryClearError::Closed);
        };
        if self.closed.load(Ordering::Acquire) {
            return Err(CleanupHistoryClearError::Closed);
        }
        if !Arc::ptr_eq(&preview.engine_closed, &self.closed) {
            return Err(CleanupHistoryClearError::WrongEngine);
        }
        let expected_session_count = preview.info.session_count;
        let core_preview = preview.take_for_clear()?;
        let result = engine
            .clear_cleanup_history(core_preview)
            .map_err(map_cleanup_history_clear_error)?;
        cleanup_history_clear_result(result, expected_session_count)
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

    /// Prepare a read-only comparison against the exact review's immediately
    /// preceding comparable retained snapshot. Rust selects and matches both
    /// histories; the child exposes no current path or cleanup capability.
    pub fn prepare_explorer_snapshot_diff_review(
        &self,
        parent: Arc<SnapshotReviewSession>,
    ) -> Result<Arc<SnapshotDiffReviewSession>, EngineError> {
        if !Arc::ptr_eq(&parent.engine_closed, &self.closed) {
            return Err(EngineError::WrongParentReview);
        }
        let state = self.state.lock().map_err(|_| EngineError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(EngineError::Closed);
        };
        if self.closed.load(Ordering::Acquire) {
            return Err(EngineError::Closed);
        }
        let current = parent
            .inner
            .lock()
            .map_err(|_| EngineError::InternalState)?;
        let diff = engine
            .prepare_explorer_snapshot_diff_review(&current)
            .map_err(map_review_error)?;
        drop(current);
        self.register_snapshot_diff_review(diff, &parent)
    }

    /// Read Foundation iCloud metadata for one exact retained Explorer file.
    ///
    /// Rust selects and revalidates the path, owns provider/kind/allocation/
    /// timestamp authority, and performs the deterministic classification.
    /// Swift can only consume the one-shot path and return bounded raw facts.
    pub fn probe_explorer_icloud_local_copy(
        &self,
        review: Arc<SnapshotReviewSession>,
        selection: ICloudLocalCopyProbeSelection,
        driver: Box<dyn ICloudLocalCopyMetadataDriver>,
    ) -> Result<ICloudLocalCopyAssessment, ICloudLocalCopyProbeError> {
        if selection.record_version != FFI_RECORD_VERSION {
            return Err(ICloudLocalCopyProbeError::InvalidRecordVersion);
        }
        if !Arc::ptr_eq(&review.engine_closed, &self.closed) {
            return Err(ICloudLocalCopyProbeError::WrongReview);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| ICloudLocalCopyProbeError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(ICloudLocalCopyProbeError::Closed);
        };
        if self.closed.load(Ordering::Acquire) {
            return Err(ICloudLocalCopyProbeError::Closed);
        }
        let mut core_review = review
            .inner
            .lock()
            .map_err(|_| ICloudLocalCopyProbeError::InternalState)?;
        let assessment = engine
            .probe_explorer_cloud_eviction(&mut core_review, selection.node_id, move |request| {
                let absolute_path_bytes = request
                    .into_path_bytes()
                    .map_err(|_| CoreCloudEvictionProbePlatformError::Failed)?;
                let ffi_request = ICloudLocalCopyProbeRequest::from_core(absolute_path_bytes);
                let callback_result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        driver.read_metadata(Arc::clone(&ffi_request))
                    }))
                    .map_err(|_| CoreCloudEvictionProbePlatformError::Failed)?;
                match callback_result {
                    ICloudLocalCopyMetadataResult::Observed { facts } => {
                        if facts.record_version != FFI_RECORD_VERSION
                            || !ffi_request.is_consumed().unwrap_or(false)
                        {
                            return Err(CoreCloudEvictionProbePlatformError::Failed);
                        }
                        Ok(core_icloud_platform_facts(facts))
                    }
                    ICloudLocalCopyMetadataResult::Unsupported => {
                        Err(CoreCloudEvictionProbePlatformError::Unsupported)
                    }
                    ICloudLocalCopyMetadataResult::Failed => {
                        Err(CoreCloudEvictionProbePlatformError::Failed)
                    }
                }
            })
            .map_err(map_icloud_local_copy_probe_error)?;
        project_icloud_local_copy_assessment(&assessment)
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
                        self.release_registered_cleanup_history_clear_previews();
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
                            self.release_registered_cleanup_history_clear_previews();
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
    fn start_rust_target_dry_run_with(
        &self,
        review: Arc<RustTargetPlanReviewSession>,
        start: impl FnOnce(
            &EngineHandle,
            CoreRustTargetPlanReview,
        )
            -> Result<TaskId, (CoreRustTargetDryRunError, Box<CoreRustTargetPlanReview>)>,
    ) -> Result<Arc<RustTargetDryRunTask>, RustTargetDryRunStartError> {
        if !Arc::ptr_eq(&review.engine_closed, &self.closed) {
            return Err(RustTargetDryRunStartError::WrongEngine);
        }
        let _operation = self
            .rust_target_plan_preparations
            .enter_operation(&self.closed)
            .map_err(map_plan_operation_to_dry_run_start_error)?;
        let engine = {
            let state = self
                .state
                .lock()
                .map_err(|_| RustTargetDryRunStartError::InternalState)?;
            match &*state {
                EngineState::Open(engine) => engine.clone(),
                EngineState::Closing | EngineState::Closed { .. } => {
                    return Err(RustTargetDryRunStartError::Closed);
                }
            }
        };
        let core_review = *review.take_for_dry_run_start()?;
        match start(&engine, core_review) {
            Ok(id) => Ok(Arc::new(RustTargetDryRunTask { engine, id })),
            Err((error, review)) => {
                (*review).release();
                Err(map_rust_target_dry_run_start_error(error))
            }
        }
    }

    fn start_permanent_safe_cleanup_with(
        &self,
        review: Arc<RustTargetPlanReviewSession>,
        start: impl FnOnce(
            &EngineHandle,
            CoreRustTargetPlanReview,
        ) -> Result<
            TaskId,
            (CoreRustTargetCleanupError, Box<CoreRustTargetPlanReview>),
        >,
    ) -> Result<Arc<RustTargetCleanupTask>, RustTargetCleanupStartError> {
        if !Arc::ptr_eq(&review.engine_closed, &self.closed) {
            return Err(RustTargetCleanupStartError::WrongEngine);
        }
        let _operation = self
            .rust_target_plan_preparations
            .enter_operation(&self.closed)
            .map_err(map_plan_operation_to_cleanup_start_error)?;
        let engine = {
            let state = self
                .state
                .lock()
                .map_err(|_| RustTargetCleanupStartError::InternalState)?;
            match &*state {
                EngineState::Open(engine) => engine.clone(),
                EngineState::Closing | EngineState::Closed { .. } => {
                    return Err(RustTargetCleanupStartError::Closed);
                }
            }
        };
        let core_review = *review.take_for_cleanup_start()?;
        match start(&engine, core_review) {
            Ok(id) => Ok(Arc::new(RustTargetCleanupTask { engine, id })),
            Err((error, review)) => {
                (*review).release();
                Err(map_rust_target_cleanup_start_error(error))
            }
        }
    }

    fn schedule_background_close(&self) {
        if self.background_close_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let state = Arc::clone(&self.state);
        let close_completed = Arc::clone(&self.close_completed);
        let operations = Arc::clone(&self.rust_target_plan_preparations);
        let reviews = Arc::clone(&self.reviews);
        let diff_reviews = Arc::clone(&self.diff_reviews);
        let plan_reviews = Arc::clone(&self.rust_target_plan_reviews);
        let cargo_previews = Arc::clone(&self.direct_cargo_previews);
        let cleanup_history_clear_previews = Arc::clone(&self.cleanup_history_clear_previews);
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
                release_snapshot_diff_review_registry(&diff_reviews);
                release_snapshot_review_registry(&reviews, &operations);
                release_direct_cargo_preview_registry(&cargo_previews, &operations);
                release_cleanup_history_clear_preview_registry(&cleanup_history_clear_previews);
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

    fn ensure_cleanup_history_clear_preview_capacity(
        &self,
    ) -> Result<(), CleanupHistoryClearError> {
        let mut previews = self
            .cleanup_history_clear_previews
            .lock()
            .map_err(|_| CleanupHistoryClearError::InternalState)?;
        let mut retained = Vec::with_capacity(previews.len());
        let mut available = false;
        for preview in previews.iter().filter_map(Weak::upgrade) {
            if preview.is_available()? {
                available = true;
                retained.push(Arc::downgrade(&preview));
            }
        }
        *previews = retained;
        if available {
            Err(CleanupHistoryClearError::Busy)
        } else {
            Ok(())
        }
    }

    fn register_cleanup_history_clear_preview(
        &self,
        preview: CoreCleanupHistoryClearPreview,
        info: CleanupHistoryClearPreviewInfo,
    ) -> Result<Arc<CleanupHistoryClearPreviewSession>, CleanupHistoryClearError> {
        let preview = Arc::new(CleanupHistoryClearPreviewSession {
            state: Mutex::new(CleanupHistoryClearPreviewState::Available(Box::new(
                preview,
            ))),
            info,
            engine_closed: Arc::clone(&self.closed),
        });
        if self.closed.load(Ordering::Acquire) {
            let _ = preview.release_inner();
            return Err(CleanupHistoryClearError::Closed);
        }
        let mut previews = match self.cleanup_history_clear_previews.lock() {
            Ok(previews) => previews,
            Err(_) => {
                let _ = preview.release_inner();
                return Err(CleanupHistoryClearError::InternalState);
            }
        };
        let mut retained = Vec::with_capacity(previews.len().saturating_add(1));
        let mut existing_busy = false;
        for retained_preview in previews.iter().filter_map(Weak::upgrade) {
            if retained_preview.is_available()? {
                existing_busy = true;
                retained.push(Arc::downgrade(&retained_preview));
                break;
            }
        }
        if existing_busy {
            drop(previews);
            let _ = preview.release_inner();
            return Err(CleanupHistoryClearError::Busy);
        }
        if self.closed.load(Ordering::Acquire) {
            drop(previews);
            let _ = preview.release_inner();
            return Err(CleanupHistoryClearError::Closed);
        }
        *previews = retained;
        previews.push(Arc::downgrade(&preview));
        Ok(preview)
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

    fn register_snapshot_diff_review(
        &self,
        diff: CoreSnapshotDiffReviewSession,
        parent: &Arc<SnapshotReviewSession>,
    ) -> Result<Arc<SnapshotDiffReviewSession>, EngineError> {
        let review = Arc::new(SnapshotDiffReviewSession {
            inner: Mutex::new(diff),
            parent: Arc::downgrade(parent),
            engine_closed: Arc::clone(&self.closed),
        });
        if self.closed.load(Ordering::Acquire) {
            let _ = review.release_inner();
            return Err(EngineError::Closed);
        }
        let mut reviews = match self.diff_reviews.lock() {
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

    fn with_configured_project_roots_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, ConfiguredProjectRootsError>,
    ) -> Result<T, ConfiguredProjectRootsError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ConfiguredProjectRootsError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(ConfiguredProjectRootsError::Closed)
            }
        }
    }

    fn with_targeted_project_scan_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, TargetedProjectScanError>,
    ) -> Result<T, TargetedProjectScanError> {
        let state = self
            .state
            .lock()
            .map_err(|_| TargetedProjectScanError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(TargetedProjectScanError::Closed)
            }
        }
    }

    fn with_emergency_recovery_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, EmergencyRecoveryError>,
    ) -> Result<T, EmergencyRecoveryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| EmergencyRecoveryError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(EmergencyRecoveryError::Closed)
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

    fn with_rule_outcome_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, RuleOutcomeError>,
    ) -> Result<T, RuleOutcomeError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RuleOutcomeError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(RuleOutcomeError::Closed)
            }
        }
    }

    fn with_storage_thief_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, StorageThiefError>,
    ) -> Result<T, StorageThiefError> {
        let state = self
            .state
            .lock()
            .map_err(|_| StorageThiefError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(StorageThiefError::Closed)
            }
        }
    }

    fn with_running_scan_debt_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, RunningScanDebtCensusError>,
    ) -> Result<T, RunningScanDebtCensusError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RunningScanDebtCensusError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(RunningScanDebtCensusError::Closed)
            }
        }
    }

    fn with_claimed_running_scan_provenance_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, ClaimedRunningScanProvenanceCensusError>,
    ) -> Result<T, ClaimedRunningScanProvenanceCensusError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ClaimedRunningScanProvenanceCensusError::InternalState)?;
        match &*state {
            EngineState::Open(engine) if !self.closed.load(Ordering::Acquire) => operation(engine),
            EngineState::Open(_) | EngineState::Closing | EngineState::Closed { .. } => {
                Err(ClaimedRunningScanProvenanceCensusError::Closed)
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
        release_snapshot_diff_review_registry(&self.diff_reviews);
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

    fn release_registered_cleanup_history_clear_previews(&self) {
        release_cleanup_history_clear_preview_registry(&self.cleanup_history_clear_previews);
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

fn release_snapshot_diff_review_registry(registry: &Mutex<Vec<Weak<SnapshotDiffReviewSession>>>) {
    let reviews = match registry.lock() {
        Ok(mut reviews) => std::mem::take(&mut *reviews),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    };
    for review in reviews.into_iter().filter_map(|review| review.upgrade()) {
        let _ = review.release_inner();
    }
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

fn release_cleanup_history_clear_preview_registry(
    registry: &Mutex<Vec<Weak<CleanupHistoryClearPreviewSession>>>,
) {
    let previews = match registry.lock() {
        Ok(mut previews) => std::mem::take(&mut *previews),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    };
    for preview in previews.into_iter().filter_map(|preview| preview.upgrade()) {
        let _ = preview.release_inner();
    }
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
        database_schema_version: DATABASE_SCHEMA_VERSION,
        snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
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
        | TaskFailureKind::CandidateEvaluationRecoveryMaintenance(_)
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
        MaintenanceKind::CandidateEvaluationRecovery => map_start!(
            engine.start_candidate_evaluation_recovery_maintenance(),
            CandidateEvaluationRecoveryMaintenanceStartOutcome::Started,
            CandidateEvaluationRecoveryMaintenanceStartOutcome::AlreadyActive,
            CandidateEvaluationRecoveryMaintenanceStartOutcome::DeferredBusy
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
        MaintenanceKind::CandidateEvaluationRecovery => engine
            .candidate_evaluation_recovery_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let (outcome, candidate_count) = match r.outcome() {
                    CoreCandidateEvaluationRecoveryOutcome::None => {
                        (MaintenanceOutcome::CandidateEvaluationRecoveryNone, 0)
                    }
                    CoreCandidateEvaluationRecoveryOutcome::Recovered { candidate_count } => (
                        MaintenanceOutcome::CandidateEvaluationRecoveryRecovered,
                        candidate_count,
                    ),
                    CoreCandidateEvaluationRecoveryOutcome::Incompatible => (
                        MaintenanceOutcome::CandidateEvaluationRecoveryIncompatible,
                        0,
                    ),
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.primary_count_after = u64::from(candidate_count);
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

fn maintenance_outcome_matches_kind(kind: MaintenanceKind, outcome: MaintenanceOutcome) -> bool {
    matches!(
        (kind, outcome),
        (
            MaintenanceKind::ScanRecovery,
            MaintenanceOutcome::ScanRecoveryNone
                | MaintenanceOutcome::ScanRecoveryDeferredUnproven
                | MaintenanceOutcome::ScanRecoveryInterrupted
                | MaintenanceOutcome::ScanRecoveryChangedConcurrently
        ) | (
            MaintenanceKind::CandidateEvaluationRecovery,
            MaintenanceOutcome::CandidateEvaluationRecoveryNone
                | MaintenanceOutcome::CandidateEvaluationRecoveryRecovered
                | MaintenanceOutcome::CandidateEvaluationRecoveryIncompatible
        ) | (MaintenanceKind::History, MaintenanceOutcome::HistoryApplied)
            | (
                MaintenanceKind::SnapshotRetention,
                MaintenanceOutcome::RetentionUnderCap
                    | MaintenanceOutcome::RetentionDeferredUnstable
                    | MaintenanceOutcome::RetentionDeferredNoEligibleSnapshot
                    | MaintenanceOutcome::RetentionRemovedTombstonedResidual
                    | MaintenanceOutcome::RetentionTombstonedAndRemoved
            )
            | (
                MaintenanceKind::SnapshotOrphan,
                MaintenanceOutcome::OrphanNone | MaintenanceOutcome::OrphanRemoved
            )
            | (
                MaintenanceKind::SnapshotProvisioningStage,
                MaintenanceOutcome::StageNone
                    | MaintenanceOutcome::StageDeferredUnproven
                    | MaintenanceOutcome::StageRemovedMarkerOnly
                    | MaintenanceOutcome::StageRemovedMarkerComplete
            )
            | (
                MaintenanceKind::SnapshotTerminalTemp,
                MaintenanceOutcome::TerminalTempNone
                    | MaintenanceOutcome::TerminalTempDeferredActive
                    | MaintenanceOutcome::TerminalTempReconciledRowOnly
                    | MaintenanceOutcome::TerminalTempRemoved
            )
            | (
                MaintenanceKind::SnapshotUnleasedTemp,
                MaintenanceOutcome::UnleasedTempNone
                    | MaintenanceOutcome::UnleasedTempDeferredActive
                    | MaintenanceOutcome::UnleasedTempRemoved
            )
    )
}

fn maintenance_core_kind_matches(kind: MaintenanceKind, core_kind: CoreTaskKind) -> bool {
    matches!(
        (kind, core_kind),
        (
            MaintenanceKind::ScanRecovery,
            CoreTaskKind::ScanRecoveryMaintenance
        ) | (
            MaintenanceKind::CandidateEvaluationRecovery,
            CoreTaskKind::CandidateEvaluationRecoveryMaintenance
        ) | (MaintenanceKind::History, CoreTaskKind::HistoryMaintenance)
            | (
                MaintenanceKind::SnapshotRetention,
                CoreTaskKind::SnapshotRetention
            )
            | (
                MaintenanceKind::SnapshotOrphan,
                CoreTaskKind::SnapshotOrphanMaintenance
            )
            | (
                MaintenanceKind::SnapshotProvisioningStage,
                CoreTaskKind::SnapshotProvisioningStageMaintenance
            )
            | (
                MaintenanceKind::SnapshotTerminalTemp,
                CoreTaskKind::SnapshotTerminalTempMaintenance
            )
            | (
                MaintenanceKind::SnapshotUnleasedTemp,
                CoreTaskKind::SnapshotUnleasedTempMaintenance
            )
    )
}

fn validate_maintenance_poll_shape(
    expected_kind: MaintenanceKind,
    phase: TaskPhase,
    failure: Option<MaintenanceFailure>,
    result: Option<&MaintenanceResult>,
) -> Result<(), EngineError> {
    let terminal_shape_is_valid = match phase {
        TaskPhase::Queued | TaskPhase::Running => failure.is_none() && result.is_none(),
        TaskPhase::Succeeded => failure.is_none() && result.is_some(),
        TaskPhase::Failed => failure.is_some() && result.is_none(),
        TaskPhase::Cancelled => failure.is_none() && result.is_none(),
    };
    if !terminal_shape_is_valid {
        return Err(EngineError::InternalState);
    }
    let Some(result) = result else {
        return Ok(());
    };
    if result.record_version != FFI_RECORD_VERSION
        || result.kind != expected_kind
        || !maintenance_outcome_matches_kind(result.kind, result.outcome)
    {
        return Err(EngineError::InternalState);
    }
    if result.kind == MaintenanceKind::CandidateEvaluationRecovery {
        let candidate_count_is_valid = match result.outcome {
            MaintenanceOutcome::CandidateEvaluationRecoveryRecovered => true,
            MaintenanceOutcome::CandidateEvaluationRecoveryNone
            | MaintenanceOutcome::CandidateEvaluationRecoveryIncompatible => {
                result.primary_count_after == 0
            }
            _ => false,
        };
        let unused_fields_are_zero = result.primary_count_before == 0
            && result.secondary_count_before == 0
            && result.secondary_count_after == 0
            && result.tertiary_count_before == 0
            && result.tertiary_count_after == 0
            && result.quaternary_count_before == 0
            && result.quaternary_count_after == 0
            && result.charged_bytes_before == 0
            && result.charged_bytes_after == 0
            && result.removed_bytes == 0
            && result.cap_bytes == 0;
        let none_has_no_more = result.outcome
            != MaintenanceOutcome::CandidateEvaluationRecoveryNone
            || !result.has_more;
        if !candidate_count_is_valid || !unused_fields_are_zero || !none_has_no_more {
            return Err(EngineError::InternalState);
        }
    }
    Ok(())
}

fn map_failure(failure: TaskFailureKind) -> MaintenanceFailure {
    use dux_core::engine::{
        CandidateEvaluationRecoveryMaintenanceFailureKind as C, HistoryMaintenanceFailureKind as H,
        ScanRecoveryMaintenanceFailureKind as S, SnapshotOrphanMaintenanceFailureKind as O,
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
        TaskFailureKind::CandidateEvaluationRecoveryMaintenance(value) => map_typed!(value, C),
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
        CoreReviewError::ComparableSnapshotUnavailable => {
            EngineError::ComparableSnapshotUnavailable
        }
        CoreReviewError::WrongParentReview => EngineError::WrongParentReview,
        CoreReviewError::LeaseExpired => EngineError::ReviewExpired,
        CoreReviewError::NodeNotFound => EngineError::SnapshotNodeNotFound,
        CoreReviewError::NodeNotDirectory => EngineError::SnapshotNodeNotDirectory,
        CoreReviewError::InvalidPage => EngineError::InvalidSnapshotNodePage,
        CoreReviewError::InvalidTreemapBudget => EngineError::InvalidSnapshotTreemapBudget,
        CoreReviewError::InvalidLargeFileRequest => EngineError::InvalidSnapshotLargeFileRequest,
        CoreReviewError::InvalidICloudObservationSourceRequest => {
            EngineError::InvalidSnapshotICloudObservationSourceRequest
        }
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

fn core_icloud_platform_facts(facts: ICloudLocalCopyRawFacts) -> CoreCloudEvictionPlatformFacts {
    CoreCloudEvictionPlatformFacts {
        ubiquitous: core_icloud_boolean(facts.ubiquitous),
        uploaded: core_icloud_boolean(facts.uploaded),
        uploading: core_icloud_boolean(facts.uploading),
        upload_error: core_icloud_error(facts.upload_error),
        unresolved_conflicts: core_icloud_boolean(facts.unresolved_conflicts),
        local_copy_state: core_icloud_local_copy_state(facts.local_copy_state),
        download_requested: core_icloud_boolean(facts.download_requested),
        downloading: core_icloud_boolean(facts.downloading),
        download_error: core_icloud_error(facts.download_error),
        excluded_from_sync: core_icloud_boolean(facts.excluded_from_sync),
        identity: CoreCloudEvictionIdentityFacts {
            account: core_icloud_identity_state(facts.account_identity),
            container: core_icloud_identity_state(facts.container_identity),
            item_generation: core_icloud_identity_state(facts.item_generation),
            file_version: core_icloud_identity_state(facts.file_version),
            shared: core_icloud_boolean(facts.shared),
            sync_paused: core_icloud_boolean(facts.sync_paused),
        },
    }
}

const fn core_icloud_boolean(value: ICloudBooleanState) -> CoreCloudBooleanState {
    match value {
        ICloudBooleanState::True => CoreCloudBooleanState::True,
        ICloudBooleanState::False => CoreCloudBooleanState::False,
        ICloudBooleanState::Unknown => CoreCloudBooleanState::Unknown,
    }
}

const fn project_icloud_boolean(value: CoreCloudBooleanState) -> ICloudBooleanState {
    match value {
        CoreCloudBooleanState::True => ICloudBooleanState::True,
        CoreCloudBooleanState::False => ICloudBooleanState::False,
        CoreCloudBooleanState::Unknown => ICloudBooleanState::Unknown,
    }
}

const fn core_icloud_error(value: ICloudErrorState) -> CoreCloudErrorState {
    match value {
        ICloudErrorState::Absent => CoreCloudErrorState::Absent,
        ICloudErrorState::Present => CoreCloudErrorState::Present,
        ICloudErrorState::Unknown => CoreCloudErrorState::Unknown,
    }
}

const fn project_icloud_error(value: CoreCloudErrorState) -> ICloudErrorState {
    match value {
        CoreCloudErrorState::Absent => ICloudErrorState::Absent,
        CoreCloudErrorState::Present => ICloudErrorState::Present,
        CoreCloudErrorState::Unknown => ICloudErrorState::Unknown,
    }
}

const fn core_icloud_local_copy_state(value: ICloudLocalCopyState) -> CoreCloudLocalCopyState {
    match value {
        ICloudLocalCopyState::Current => CoreCloudLocalCopyState::Current,
        ICloudLocalCopyState::Stale => CoreCloudLocalCopyState::Stale,
        ICloudLocalCopyState::NotDownloaded => CoreCloudLocalCopyState::NotDownloaded,
        ICloudLocalCopyState::Unknown => CoreCloudLocalCopyState::Unknown,
    }
}

const fn project_icloud_local_copy_state(value: CoreCloudLocalCopyState) -> ICloudLocalCopyState {
    match value {
        CoreCloudLocalCopyState::Current => ICloudLocalCopyState::Current,
        CoreCloudLocalCopyState::Stale => ICloudLocalCopyState::Stale,
        CoreCloudLocalCopyState::NotDownloaded => ICloudLocalCopyState::NotDownloaded,
        CoreCloudLocalCopyState::Unknown => ICloudLocalCopyState::Unknown,
    }
}

const fn core_icloud_identity_state(value: ICloudIdentityFactState) -> CoreCloudIdentityFactState {
    match value {
        ICloudIdentityFactState::Stable => CoreCloudIdentityFactState::Stable,
        ICloudIdentityFactState::Unavailable => CoreCloudIdentityFactState::Unavailable,
        ICloudIdentityFactState::ChangedDuringRead => CoreCloudIdentityFactState::ChangedDuringRead,
        ICloudIdentityFactState::Unsupported => CoreCloudIdentityFactState::Unsupported,
    }
}

const fn project_icloud_identity_state(
    value: CoreCloudIdentityFactState,
) -> ICloudIdentityFactState {
    match value {
        CoreCloudIdentityFactState::Stable => ICloudIdentityFactState::Stable,
        CoreCloudIdentityFactState::Unavailable => ICloudIdentityFactState::Unavailable,
        CoreCloudIdentityFactState::ChangedDuringRead => ICloudIdentityFactState::ChangedDuringRead,
        CoreCloudIdentityFactState::Unsupported => ICloudIdentityFactState::Unsupported,
    }
}

fn project_icloud_local_copy_assessment(
    assessment: &CoreCloudEvictionAssessment,
) -> Result<ICloudLocalCopyAssessment, ICloudLocalCopyProbeError> {
    let observation = assessment.observation();
    let provider = match observation.provider() {
        CoreCloudEvictionProvider::ICloudDrive => ICloudLocalCopyProvider::ICloudDrive,
    };
    let item_kind = match observation.item_kind() {
        CoreCloudEvictionItemKind::RegularFile => ICloudLocalCopyItemKind::RegularFile,
        CoreCloudEvictionItemKind::Directory
        | CoreCloudEvictionItemKind::Symlink
        | CoreCloudEvictionItemKind::Other
        | CoreCloudEvictionItemKind::Unknown => {
            return Err(ICloudLocalCopyProbeError::InternalState);
        }
    };
    let local_allocated_bytes = observation
        .local_allocated_bytes()
        .ok_or(ICloudLocalCopyProbeError::InternalState)?;
    let observed_at_unix_ms = i64::try_from(
        observation
            .observed_at()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ICloudLocalCopyProbeError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| ICloudLocalCopyProbeError::InternalState)?;
    Ok(ICloudLocalCopyAssessment {
        record_version: FFI_RECORD_VERSION,
        provider,
        item_kind,
        local_allocated_bytes,
        observed_at_unix_ms,
        ubiquitous: project_icloud_boolean(observation.ubiquitous()),
        uploaded: project_icloud_boolean(observation.uploaded()),
        uploading: project_icloud_boolean(observation.uploading()),
        upload_error: project_icloud_error(observation.upload_error()),
        unresolved_conflicts: project_icloud_boolean(observation.unresolved_conflicts()),
        local_copy_state: project_icloud_local_copy_state(observation.local_copy_state()),
        download_requested: project_icloud_boolean(observation.download_requested()),
        downloading: project_icloud_boolean(observation.downloading()),
        download_error: project_icloud_error(observation.download_error()),
        excluded_from_sync: project_icloud_boolean(observation.excluded_from_sync()),
        account_identity: project_icloud_identity_state(observation.identity().account),
        container_identity: project_icloud_identity_state(observation.identity().container),
        item_generation: project_icloud_identity_state(observation.identity().item_generation),
        file_version: project_icloud_identity_state(observation.identity().file_version),
        shared: project_icloud_boolean(observation.identity().shared),
        sync_paused: project_icloud_boolean(observation.identity().sync_paused),
        is_eligible_observation: assessment.is_eligible_observation(),
        blockers: assessment
            .blockers()
            .iter()
            .copied()
            .map(project_icloud_block_reason)
            .collect(),
        is_identity_ready: assessment.is_identity_ready(),
        identity_blockers: assessment
            .identity_blockers()
            .iter()
            .copied()
            .map(project_icloud_identity_block_reason)
            .collect(),
    })
}

const fn project_icloud_identity_block_reason(
    reason: CoreCloudEvictionIdentityBlockReason,
) -> ICloudIdentityBlockReason {
    match reason {
        CoreCloudEvictionIdentityBlockReason::AccountIdentityUnavailable => {
            ICloudIdentityBlockReason::AccountIdentityUnavailable
        }
        CoreCloudEvictionIdentityBlockReason::AccountIdentityChanged => {
            ICloudIdentityBlockReason::AccountIdentityChanged
        }
        CoreCloudEvictionIdentityBlockReason::AccountIdentityUnsupported => {
            ICloudIdentityBlockReason::AccountIdentityUnsupported
        }
        CoreCloudEvictionIdentityBlockReason::ContainerIdentityUnavailable => {
            ICloudIdentityBlockReason::ContainerIdentityUnavailable
        }
        CoreCloudEvictionIdentityBlockReason::ContainerIdentityChanged => {
            ICloudIdentityBlockReason::ContainerIdentityChanged
        }
        CoreCloudEvictionIdentityBlockReason::ContainerIdentityUnsupported => {
            ICloudIdentityBlockReason::ContainerIdentityUnsupported
        }
        CoreCloudEvictionIdentityBlockReason::ItemGenerationUnavailable => {
            ICloudIdentityBlockReason::ItemGenerationUnavailable
        }
        CoreCloudEvictionIdentityBlockReason::ItemGenerationChanged => {
            ICloudIdentityBlockReason::ItemGenerationChanged
        }
        CoreCloudEvictionIdentityBlockReason::ItemGenerationUnsupported => {
            ICloudIdentityBlockReason::ItemGenerationUnsupported
        }
        CoreCloudEvictionIdentityBlockReason::FileVersionUnavailable => {
            ICloudIdentityBlockReason::FileVersionUnavailable
        }
        CoreCloudEvictionIdentityBlockReason::FileVersionChanged => {
            ICloudIdentityBlockReason::FileVersionChanged
        }
        CoreCloudEvictionIdentityBlockReason::FileVersionUnsupported => {
            ICloudIdentityBlockReason::FileVersionUnsupported
        }
        CoreCloudEvictionIdentityBlockReason::SharedStateUnknown => {
            ICloudIdentityBlockReason::SharedStateUnknown
        }
        CoreCloudEvictionIdentityBlockReason::SharedItem => ICloudIdentityBlockReason::SharedItem,
        CoreCloudEvictionIdentityBlockReason::SyncPausedStateUnknown => {
            ICloudIdentityBlockReason::SyncPausedStateUnknown
        }
        CoreCloudEvictionIdentityBlockReason::SyncPaused => ICloudIdentityBlockReason::SyncPaused,
    }
}

const fn project_icloud_block_reason(
    reason: CoreCloudEvictionBlockReason,
) -> ICloudLocalCopyBlockReason {
    match reason {
        CoreCloudEvictionBlockReason::UnsupportedItemKind => {
            ICloudLocalCopyBlockReason::UnsupportedItemKind
        }
        CoreCloudEvictionBlockReason::UbiquityUnknown => {
            ICloudLocalCopyBlockReason::UbiquityUnknown
        }
        CoreCloudEvictionBlockReason::NotUbiquitous => ICloudLocalCopyBlockReason::NotUbiquitous,
        CoreCloudEvictionBlockReason::UploadStateUnknown => {
            ICloudLocalCopyBlockReason::UploadStateUnknown
        }
        CoreCloudEvictionBlockReason::UploadIncomplete => {
            ICloudLocalCopyBlockReason::UploadIncomplete
        }
        CoreCloudEvictionBlockReason::UploadActivityUnknown => {
            ICloudLocalCopyBlockReason::UploadActivityUnknown
        }
        CoreCloudEvictionBlockReason::UploadInProgress => {
            ICloudLocalCopyBlockReason::UploadInProgress
        }
        CoreCloudEvictionBlockReason::UploadErrorUnknown => {
            ICloudLocalCopyBlockReason::UploadErrorUnknown
        }
        CoreCloudEvictionBlockReason::UploadErrorPresent => {
            ICloudLocalCopyBlockReason::UploadErrorPresent
        }
        CoreCloudEvictionBlockReason::ConflictStateUnknown => {
            ICloudLocalCopyBlockReason::ConflictStateUnknown
        }
        CoreCloudEvictionBlockReason::UnresolvedConflicts => {
            ICloudLocalCopyBlockReason::UnresolvedConflicts
        }
        CoreCloudEvictionBlockReason::LocalCopyStateUnknown => {
            ICloudLocalCopyBlockReason::LocalCopyStateUnknown
        }
        CoreCloudEvictionBlockReason::StaleLocalCopy => ICloudLocalCopyBlockReason::StaleLocalCopy,
        CoreCloudEvictionBlockReason::NoLocalCopy => ICloudLocalCopyBlockReason::NoLocalCopy,
        CoreCloudEvictionBlockReason::DownloadRequestUnknown => {
            ICloudLocalCopyBlockReason::DownloadRequestUnknown
        }
        CoreCloudEvictionBlockReason::DownloadRequested => {
            ICloudLocalCopyBlockReason::DownloadRequested
        }
        CoreCloudEvictionBlockReason::DownloadActivityUnknown => {
            ICloudLocalCopyBlockReason::DownloadActivityUnknown
        }
        CoreCloudEvictionBlockReason::DownloadInProgress => {
            ICloudLocalCopyBlockReason::DownloadInProgress
        }
        CoreCloudEvictionBlockReason::DownloadErrorUnknown => {
            ICloudLocalCopyBlockReason::DownloadErrorUnknown
        }
        CoreCloudEvictionBlockReason::DownloadErrorPresent => {
            ICloudLocalCopyBlockReason::DownloadErrorPresent
        }
        CoreCloudEvictionBlockReason::SyncExclusionUnknown => {
            ICloudLocalCopyBlockReason::SyncExclusionUnknown
        }
        CoreCloudEvictionBlockReason::ExcludedFromSync => {
            ICloudLocalCopyBlockReason::ExcludedFromSync
        }
        CoreCloudEvictionBlockReason::AllocationUnknown => {
            ICloudLocalCopyBlockReason::AllocationUnknown
        }
        CoreCloudEvictionBlockReason::NoLocalAllocation => {
            ICloudLocalCopyBlockReason::NoLocalAllocation
        }
        CoreCloudEvictionBlockReason::InvalidObservationTime => {
            ICloudLocalCopyBlockReason::InvalidObservationTime
        }
    }
}

fn map_icloud_local_copy_probe_error(
    error: CoreCloudEvictionProbeError,
) -> ICloudLocalCopyProbeError {
    match error {
        CoreCloudEvictionProbeError::Closed => ICloudLocalCopyProbeError::Closed,
        CoreCloudEvictionProbeError::WrongReview => ICloudLocalCopyProbeError::WrongReview,
        CoreCloudEvictionProbeError::InvalidTarget => ICloudLocalCopyProbeError::InvalidTarget,
        CoreCloudEvictionProbeError::ChangedSinceSnapshot => {
            ICloudLocalCopyProbeError::ChangedSinceSnapshot
        }
        CoreCloudEvictionProbeError::ReviewUnavailable => {
            ICloudLocalCopyProbeError::ReviewUnavailable
        }
        CoreCloudEvictionProbeError::PlatformUnsupported => {
            ICloudLocalCopyProbeError::PlatformUnsupported
        }
        CoreCloudEvictionProbeError::PlatformFailed => ICloudLocalCopyProbeError::PlatformFailed,
        _ => ICloudLocalCopyProbeError::InternalState,
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

fn project_rust_target_cleanup_result(
    result: &CoreRustTargetCleanupResult,
) -> Result<RustTargetCleanupResult, RustTargetCleanupTaskError> {
    let session_id = result.session_id().as_str();
    if !is_rust_target_cleanup_session_id(session_id) {
        return Err(RustTargetCleanupTaskError::InternalState);
    }
    let status = map_cleanup_session_status(result.status())
        .map_err(|_| RustTargetCleanupTaskError::InternalState)?;
    let projected = RustTargetCleanupResult {
        record_version: FFI_RECORD_VERSION,
        session_id: session_id.to_owned(),
        status,
        removed_entries: result.removed_entries(),
        removed_logical_bytes: result.removed_logical_bytes(),
        verified_capacity_delta_bytes: result.verified_capacity_delta_bytes(),
    };
    validate_rust_target_cleanup_result(&projected)?;
    Ok(projected)
}

fn is_rust_target_cleanup_session_id(value: &str) -> bool {
    const PREFIX: &str = "cleanup:rust-target:";
    value.len() <= MAX_CLEANUP_HISTORY_SESSION_ID_BYTES
        && value.strip_prefix(PREFIX).is_some_and(|suffix| {
            suffix.len() == 32
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn validate_rust_target_cleanup_result(
    result: &RustTargetCleanupResult,
) -> Result<(), RustTargetCleanupTaskError> {
    if result.record_version != FFI_RECORD_VERSION
        || !is_rust_target_cleanup_session_id(&result.session_id)
        || (result.status == CleanupSessionStatus::Recovering
            && (result.removed_entries != 0
                || result.removed_logical_bytes != 0
                || result.verified_capacity_delta_bytes.is_some()))
    {
        return Err(RustTargetCleanupTaskError::InternalState);
    }
    Ok(())
}

fn validate_rust_target_cleanup_poll_shape(
    phase: CoreTaskPhase,
    failure: Option<RustTargetCleanupTaskFailure>,
    result: Option<&RustTargetCleanupResult>,
) -> Result<(), RustTargetCleanupTaskError> {
    if let Some(result) = result {
        validate_rust_target_cleanup_result(result)?;
    }
    let valid = match phase {
        CoreTaskPhase::Queued | CoreTaskPhase::Running => failure.is_none() && result.is_none(),
        CoreTaskPhase::Succeeded => {
            failure.is_none()
                && result.is_some_and(|result| {
                    matches!(
                        result.status,
                        CleanupSessionStatus::Completed
                            | CleanupSessionStatus::PartiallyCompleted
                            | CleanupSessionStatus::Failed
                            | CleanupSessionStatus::Interrupted
                            | CleanupSessionStatus::Rejected
                    )
                })
        }
        CoreTaskPhase::Cancelled => {
            failure.is_none()
                && result.is_none_or(|result| result.status == CleanupSessionStatus::Cancelled)
        }
        CoreTaskPhase::Failed => match failure {
            Some(RustTargetCleanupTaskFailure::OutcomeUnknown) => {
                result.is_none_or(|result| result.status == CleanupSessionStatus::Recovering)
            }
            Some(_) => result.is_none(),
            None => false,
        },
    };
    if valid {
        Ok(())
    } else {
        Err(RustTargetCleanupTaskError::InternalState)
    }
}

fn map_rust_target_cleanup_task_failure(
    failure: TaskFailureKind,
) -> Result<RustTargetCleanupTaskFailure, RustTargetCleanupTaskError> {
    let TaskFailureKind::PermanentSafeCleanup(failure) = failure else {
        return Err(RustTargetCleanupTaskError::InternalState);
    };
    Ok(match failure {
        CorePermanentSafeCleanupFailureKind::ParentReviewUnavailable => {
            RustTargetCleanupTaskFailure::ParentReviewUnavailable
        }
        CorePermanentSafeCleanupFailureKind::ReviewExpired => {
            RustTargetCleanupTaskFailure::ReviewExpired
        }
        CorePermanentSafeCleanupFailureKind::ChangedDuringReview => {
            RustTargetCleanupTaskFailure::ChangedDuringReview
        }
        CorePermanentSafeCleanupFailureKind::BudgetExceeded => {
            RustTargetCleanupTaskFailure::BudgetExceeded
        }
        CorePermanentSafeCleanupFailureKind::Busy => RustTargetCleanupTaskFailure::Busy,
        CorePermanentSafeCleanupFailureKind::UnsafeStorage => {
            RustTargetCleanupTaskFailure::UnsafeStorage
        }
        CorePermanentSafeCleanupFailureKind::IncompatibleSchema => {
            RustTargetCleanupTaskFailure::IncompatibleSchema
        }
        CorePermanentSafeCleanupFailureKind::CorruptData => {
            RustTargetCleanupTaskFailure::CorruptData
        }
        CorePermanentSafeCleanupFailureKind::OutcomeUnknown => {
            RustTargetCleanupTaskFailure::OutcomeUnknown
        }
        CorePermanentSafeCleanupFailureKind::Unavailable => {
            RustTargetCleanupTaskFailure::Unavailable
        }
        CorePermanentSafeCleanupFailureKind::InternalState => {
            RustTargetCleanupTaskFailure::InternalState
        }
        _ => RustTargetCleanupTaskFailure::InternalState,
    })
}

const fn map_rust_target_cleanup_start_error(
    error: CoreRustTargetCleanupError,
) -> RustTargetCleanupStartError {
    match error {
        CoreRustTargetCleanupError::Closed => RustTargetCleanupStartError::Closed,
        CoreRustTargetCleanupError::WrongEngine => RustTargetCleanupStartError::WrongEngine,
        CoreRustTargetCleanupError::ParentReviewUnavailable => {
            RustTargetCleanupStartError::ParentReviewUnavailable
        }
        CoreRustTargetCleanupError::ReviewExpired => RustTargetCleanupStartError::ReviewExpired,
        CoreRustTargetCleanupError::ChangedDuringReview => {
            RustTargetCleanupStartError::ChangedDuringReview
        }
        CoreRustTargetCleanupError::CancelledBeforeStart => {
            RustTargetCleanupStartError::CancelledBeforeStart
        }
        CoreRustTargetCleanupError::BudgetExceeded => RustTargetCleanupStartError::BudgetExceeded,
        CoreRustTargetCleanupError::QueueFull => RustTargetCleanupStartError::QueueFull,
        CoreRustTargetCleanupError::Busy => RustTargetCleanupStartError::Busy,
        CoreRustTargetCleanupError::UnsafeStorage => RustTargetCleanupStartError::UnsafeStorage,
        CoreRustTargetCleanupError::IncompatibleSchema => {
            RustTargetCleanupStartError::IncompatibleSchema
        }
        CoreRustTargetCleanupError::CorruptData => RustTargetCleanupStartError::CorruptData,
        CoreRustTargetCleanupError::OutcomeUnknown => RustTargetCleanupStartError::OutcomeUnknown,
        CoreRustTargetCleanupError::Unavailable => RustTargetCleanupStartError::Unavailable,
        CoreRustTargetCleanupError::InternalState => RustTargetCleanupStartError::InternalState,
    }
}

const fn map_plan_operation_to_cleanup_start_error(
    error: RustTargetPlanReviewError,
) -> RustTargetCleanupStartError {
    match error {
        RustTargetPlanReviewError::Closed => RustTargetCleanupStartError::Closed,
        RustTargetPlanReviewError::WrongEngine => RustTargetCleanupStartError::WrongEngine,
        RustTargetPlanReviewError::ParentReviewUnavailable => {
            RustTargetCleanupStartError::ParentReviewUnavailable
        }
        RustTargetPlanReviewError::ReviewExpired => RustTargetCleanupStartError::ReviewExpired,
        RustTargetPlanReviewError::ChangedDuringReview
        | RustTargetPlanReviewError::CandidateUnavailable
        | RustTargetPlanReviewError::CargoNotEnrolled
        | RustTargetPlanReviewError::ActiveProcesses => {
            RustTargetCleanupStartError::ChangedDuringReview
        }
        RustTargetPlanReviewError::BudgetExceeded => RustTargetCleanupStartError::BudgetExceeded,
        RustTargetPlanReviewError::Busy | RustTargetPlanReviewError::ReviewBusy => {
            RustTargetCleanupStartError::Busy
        }
        RustTargetPlanReviewError::UnsafeStorage => RustTargetCleanupStartError::UnsafeStorage,
        RustTargetPlanReviewError::CorruptData => RustTargetCleanupStartError::CorruptData,
        RustTargetPlanReviewError::UnsupportedPlatform | RustTargetPlanReviewError::Unavailable => {
            RustTargetCleanupStartError::Unavailable
        }
        RustTargetPlanReviewError::ReviewUnavailable => {
            RustTargetCleanupStartError::ReviewUnavailable
        }
        RustTargetPlanReviewError::InvalidRecordVersion
        | RustTargetPlanReviewError::InternalState => RustTargetCleanupStartError::InternalState,
    }
}

fn map_rust_target_cleanup_task_access_error(error: TaskAccessError) -> RustTargetCleanupTaskError {
    match error {
        TaskAccessError::Closed => RustTargetCleanupTaskError::Closed,
        TaskAccessError::UnknownTask => RustTargetCleanupTaskError::TaskUnavailable,
        TaskAccessError::WrongTaskKind => RustTargetCleanupTaskError::WrongTaskKind,
        TaskAccessError::InvalidEventLimit { .. }
        | TaskAccessError::InvalidEventCursor
        | TaskAccessError::InternalState => RustTargetCleanupTaskError::InternalState,
    }
}

const fn map_rust_target_cleanup_cancel_outcome(
    outcome: CoreCancelOutcome,
) -> RustTargetCleanupCancelOutcome {
    match outcome {
        CoreCancelOutcome::CancelledBeforeStart => {
            RustTargetCleanupCancelOutcome::CancelledBeforeStart
        }
        CoreCancelOutcome::Requested => RustTargetCleanupCancelOutcome::Requested,
        CoreCancelOutcome::AlreadyRequested => RustTargetCleanupCancelOutcome::AlreadyRequested,
        CoreCancelOutcome::AlreadyTerminal => RustTargetCleanupCancelOutcome::AlreadyTerminal,
    }
}

fn project_rust_target_dry_run_result(
    result: &CoreRustTargetDryRunResult,
) -> Result<RustTargetDryRunResult, RustTargetDryRunTaskError> {
    let session_id = result.session_id().as_str();
    if !is_rust_target_dry_run_session_id(session_id) {
        return Err(RustTargetDryRunTaskError::InternalState);
    }
    let status = map_cleanup_session_status(result.status())
        .map_err(|_| RustTargetDryRunTaskError::InternalState)?;
    let projected = RustTargetDryRunResult {
        record_version: FFI_RECORD_VERSION,
        session_id: session_id.to_owned(),
        status,
    };
    validate_rust_target_dry_run_result(&projected)?;
    Ok(projected)
}

fn is_rust_target_dry_run_session_id(value: &str) -> bool {
    const PREFIX: &str = "cleanup:rust-target-dry-run:";
    value.len() <= MAX_CLEANUP_HISTORY_SESSION_ID_BYTES
        && value.strip_prefix(PREFIX).is_some_and(|suffix| {
            suffix.len() == 32
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn validate_rust_target_dry_run_result(
    result: &RustTargetDryRunResult,
) -> Result<(), RustTargetDryRunTaskError> {
    if result.record_version != FFI_RECORD_VERSION
        || !is_rust_target_dry_run_session_id(&result.session_id)
        || !matches!(
            result.status,
            CleanupSessionStatus::DryRun
                | CleanupSessionStatus::Rejected
                | CleanupSessionStatus::Failed
                | CleanupSessionStatus::Interrupted
                | CleanupSessionStatus::Cancelled
        )
    {
        return Err(RustTargetDryRunTaskError::InternalState);
    }
    Ok(())
}

fn validate_rust_target_dry_run_poll_shape(
    phase: CoreTaskPhase,
    failure: Option<RustTargetDryRunTaskFailure>,
    result: Option<&RustTargetDryRunResult>,
) -> Result<(), RustTargetDryRunTaskError> {
    if let Some(result) = result {
        validate_rust_target_dry_run_result(result)?;
    }
    let valid = match phase {
        CoreTaskPhase::Queued | CoreTaskPhase::Running => failure.is_none() && result.is_none(),
        CoreTaskPhase::Succeeded => {
            failure.is_none()
                && result.is_some_and(|result| {
                    matches!(
                        result.status,
                        CleanupSessionStatus::DryRun
                            | CleanupSessionStatus::Rejected
                            | CleanupSessionStatus::Failed
                            | CleanupSessionStatus::Interrupted
                    )
                })
        }
        CoreTaskPhase::Cancelled => {
            failure.is_none()
                && result.is_none_or(|result| result.status == CleanupSessionStatus::Cancelled)
        }
        CoreTaskPhase::Failed => failure.is_some() && result.is_none(),
    };
    if valid {
        Ok(())
    } else {
        Err(RustTargetDryRunTaskError::InternalState)
    }
}

fn map_rust_target_dry_run_task_failure(
    failure: TaskFailureKind,
) -> Result<RustTargetDryRunTaskFailure, RustTargetDryRunTaskError> {
    let TaskFailureKind::RustTargetDryRun(failure) = failure else {
        return Err(RustTargetDryRunTaskError::InternalState);
    };
    Ok(match failure {
        CoreRustTargetDryRunFailureKind::ParentReviewUnavailable => {
            RustTargetDryRunTaskFailure::ParentReviewUnavailable
        }
        CoreRustTargetDryRunFailureKind::ReviewExpired => {
            RustTargetDryRunTaskFailure::ReviewExpired
        }
        CoreRustTargetDryRunFailureKind::ChangedDuringReview => {
            RustTargetDryRunTaskFailure::ChangedDuringReview
        }
        CoreRustTargetDryRunFailureKind::BudgetExceeded => {
            RustTargetDryRunTaskFailure::BudgetExceeded
        }
        CoreRustTargetDryRunFailureKind::Busy => RustTargetDryRunTaskFailure::Busy,
        CoreRustTargetDryRunFailureKind::UnsafeStorage => {
            RustTargetDryRunTaskFailure::UnsafeStorage
        }
        CoreRustTargetDryRunFailureKind::IncompatibleSchema => {
            RustTargetDryRunTaskFailure::IncompatibleSchema
        }
        CoreRustTargetDryRunFailureKind::CorruptData => RustTargetDryRunTaskFailure::CorruptData,
        CoreRustTargetDryRunFailureKind::HistoryUnresolved => {
            RustTargetDryRunTaskFailure::HistoryUnresolved
        }
        CoreRustTargetDryRunFailureKind::Unavailable => RustTargetDryRunTaskFailure::Unavailable,
        CoreRustTargetDryRunFailureKind::InternalState => {
            RustTargetDryRunTaskFailure::InternalState
        }
        _ => RustTargetDryRunTaskFailure::InternalState,
    })
}

const fn map_rust_target_dry_run_start_error(
    error: CoreRustTargetDryRunError,
) -> RustTargetDryRunStartError {
    match error {
        CoreRustTargetDryRunError::Closed => RustTargetDryRunStartError::Closed,
        CoreRustTargetDryRunError::WrongEngine => RustTargetDryRunStartError::WrongEngine,
        CoreRustTargetDryRunError::ParentReviewUnavailable => {
            RustTargetDryRunStartError::ParentReviewUnavailable
        }
        CoreRustTargetDryRunError::ReviewExpired => RustTargetDryRunStartError::ReviewExpired,
        CoreRustTargetDryRunError::ChangedDuringReview => {
            RustTargetDryRunStartError::ChangedDuringReview
        }
        CoreRustTargetDryRunError::CancelledBeforeStart => {
            RustTargetDryRunStartError::CancelledBeforeStart
        }
        CoreRustTargetDryRunError::BudgetExceeded => RustTargetDryRunStartError::BudgetExceeded,
        CoreRustTargetDryRunError::QueueFull => RustTargetDryRunStartError::QueueFull,
        CoreRustTargetDryRunError::Busy => RustTargetDryRunStartError::Busy,
        CoreRustTargetDryRunError::UnsafeStorage => RustTargetDryRunStartError::UnsafeStorage,
        CoreRustTargetDryRunError::IncompatibleSchema => {
            RustTargetDryRunStartError::IncompatibleSchema
        }
        CoreRustTargetDryRunError::CorruptData => RustTargetDryRunStartError::CorruptData,
        CoreRustTargetDryRunError::HistoryUnresolved => {
            RustTargetDryRunStartError::HistoryUnresolved
        }
        CoreRustTargetDryRunError::Unavailable => RustTargetDryRunStartError::Unavailable,
        CoreRustTargetDryRunError::InternalState => RustTargetDryRunStartError::InternalState,
    }
}

const fn map_plan_operation_to_dry_run_start_error(
    error: RustTargetPlanReviewError,
) -> RustTargetDryRunStartError {
    match error {
        RustTargetPlanReviewError::Closed => RustTargetDryRunStartError::Closed,
        RustTargetPlanReviewError::WrongEngine => RustTargetDryRunStartError::WrongEngine,
        RustTargetPlanReviewError::ParentReviewUnavailable => {
            RustTargetDryRunStartError::ParentReviewUnavailable
        }
        RustTargetPlanReviewError::ReviewExpired => RustTargetDryRunStartError::ReviewExpired,
        RustTargetPlanReviewError::ChangedDuringReview
        | RustTargetPlanReviewError::CandidateUnavailable
        | RustTargetPlanReviewError::CargoNotEnrolled
        | RustTargetPlanReviewError::ActiveProcesses => {
            RustTargetDryRunStartError::ChangedDuringReview
        }
        RustTargetPlanReviewError::BudgetExceeded => RustTargetDryRunStartError::BudgetExceeded,
        RustTargetPlanReviewError::Busy | RustTargetPlanReviewError::ReviewBusy => {
            RustTargetDryRunStartError::Busy
        }
        RustTargetPlanReviewError::UnsafeStorage => RustTargetDryRunStartError::UnsafeStorage,
        RustTargetPlanReviewError::CorruptData => RustTargetDryRunStartError::CorruptData,
        RustTargetPlanReviewError::UnsupportedPlatform | RustTargetPlanReviewError::Unavailable => {
            RustTargetDryRunStartError::Unavailable
        }
        RustTargetPlanReviewError::ReviewUnavailable => {
            RustTargetDryRunStartError::ReviewUnavailable
        }
        RustTargetPlanReviewError::InvalidRecordVersion
        | RustTargetPlanReviewError::InternalState => RustTargetDryRunStartError::InternalState,
    }
}

fn map_rust_target_dry_run_task_access_error(error: TaskAccessError) -> RustTargetDryRunTaskError {
    match error {
        TaskAccessError::Closed => RustTargetDryRunTaskError::Closed,
        TaskAccessError::UnknownTask => RustTargetDryRunTaskError::TaskUnavailable,
        TaskAccessError::WrongTaskKind => RustTargetDryRunTaskError::WrongTaskKind,
        TaskAccessError::InvalidEventLimit { .. }
        | TaskAccessError::InvalidEventCursor
        | TaskAccessError::InternalState => RustTargetDryRunTaskError::InternalState,
    }
}

const fn map_rust_target_dry_run_cancel_outcome(
    outcome: CoreCancelOutcome,
) -> RustTargetDryRunCancelOutcome {
    match outcome {
        CoreCancelOutcome::CancelledBeforeStart => {
            RustTargetDryRunCancelOutcome::CancelledBeforeStart
        }
        CoreCancelOutcome::Requested => RustTargetDryRunCancelOutcome::Requested,
        CoreCancelOutcome::AlreadyRequested => RustTargetDryRunCancelOutcome::AlreadyRequested,
        CoreCancelOutcome::AlreadyTerminal => RustTargetDryRunCancelOutcome::AlreadyTerminal,
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
        || info.rule_revision != 3
        || info.category != CoreCandidateCategory::DeveloperArtifact
        || info.mode != CorePlanCleanupMode::PermanentSafe
        || info.safety != CoreSafetyTier::SafeRegenerable
        || info.action != CoreCandidateAction::RemoveKnownRegenerableContents
        || info.minimum_age != RUST_TARGET_MINIMUM_AGE
        || info
            .created_at
            .duration_since(info.newest_mtime)
            .map_or(true, |age| age < info.minimum_age)
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
    if info.created_at >= info.effective_expires_at {
        return Err(RustTargetPlanReviewError::InternalState);
    }
    let created_at = project_system_time_timestamp(info.created_at)
        .map_err(|_| RustTargetPlanReviewError::InternalState)?;
    let effective_expires_at = project_system_time_timestamp(info.effective_expires_at)
        .map_err(|_| RustTargetPlanReviewError::InternalState)?;
    let newest_mtime = project_system_time_timestamp(info.newest_mtime)
        .map_err(|_| RustTargetPlanReviewError::InternalState)?;
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
        newest_mtime,
        minimum_age_seconds: info.minimum_age.as_secs(),
        minimum_age_nanoseconds: info.minimum_age.subsec_nanos(),
        warnings: vec![
            CleanupWarning::EstimatedBytesUnverified,
            CleanupWarning::PermanentRemovalCannotBeUndone,
        ],
        created_at,
        effective_expires_at,
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

const fn map_snapshot_diff_node_sort(sort: SnapshotDiffNodeSort) -> CoreSnapshotDiffNodeSort {
    match sort {
        SnapshotDiffNodeSort::NameAscending => CoreSnapshotDiffNodeSort::NameAscending,
        SnapshotDiffNodeSort::MagnitudeDescending => CoreSnapshotDiffNodeSort::MagnitudeDescending,
        SnapshotDiffNodeSort::CurrentBytesDescending => {
            CoreSnapshotDiffNodeSort::CurrentBytesDescending
        }
    }
}

fn project_snapshot_diff_info(
    info: CoreSnapshotDiffInfo,
    released: bool,
) -> Result<SnapshotDiffInfo, EngineError> {
    Ok(SnapshotDiffInfo {
        record_version: FFI_RECORD_VERSION,
        current_scan_id: info.current_scan_id.as_str().to_owned(),
        baseline_scan_id: info.baseline_scan_id.as_str().to_owned(),
        current_started_at_unix_ms: system_time_ms(info.current_started_at)?,
        current_completed_at_unix_ms: system_time_ms(info.current_completed_at)?,
        baseline_started_at_unix_ms: system_time_ms(info.baseline_started_at)?,
        baseline_completed_at_unix_ms: system_time_ms(info.baseline_completed_at)?,
        current_coverage: ScanCoverageSummary {
            record_version: FFI_RECORD_VERSION,
            status: map_scan_coverage_status(info.current_coverage.status),
            measured_permille: info.current_coverage.measured_permille,
            issue_record_count: info.current_coverage.issue_record_count,
            issue_occurrence_count: info.current_coverage.issue_occurrence_count,
        },
        baseline_coverage: ScanCoverageSummary {
            record_version: FFI_RECORD_VERSION,
            status: map_scan_coverage_status(info.baseline_coverage.status),
            measured_permille: info.baseline_coverage.measured_permille,
            issue_record_count: info.baseline_coverage.issue_record_count,
            issue_occurrence_count: info.baseline_coverage.issue_occurrence_count,
        },
        released,
    })
}

fn project_snapshot_diff_node(node: CoreSnapshotDiffNode) -> SnapshotDiffNode {
    SnapshotDiffNode {
        record_version: FFI_RECORD_VERSION,
        id: node.id,
        parent_id: node.parent_id,
        depth: node.depth,
        name: project_snapshot_node_name(node.name),
        kind: project_snapshot_node_kind(node.kind),
        current_kind: node.current_kind.map(project_snapshot_node_kind),
        baseline_kind: node.baseline_kind.map(project_snapshot_node_kind),
        category: project_snapshot_category(node.category),
        change: match node.change {
            CoreSnapshotDiffChange::Added => SnapshotDiffChange::Added,
            CoreSnapshotDiffChange::Removed => SnapshotDiffChange::Removed,
            CoreSnapshotDiffChange::Grew => SnapshotDiffChange::Grew,
            CoreSnapshotDiffChange::Shrank => SnapshotDiffChange::Shrank,
            CoreSnapshotDiffChange::Unchanged => SnapshotDiffChange::Unchanged,
            CoreSnapshotDiffChange::Replaced => SnapshotDiffChange::Replaced,
        },
        logical_change: project_snapshot_diff_value(node.logical_change),
        current_logical_bytes: node.current_logical_bytes,
        baseline_logical_bytes: node.baseline_logical_bytes,
        current_allocated_bytes: node.current_allocated_bytes,
        baseline_allocated_bytes: node.baseline_allocated_bytes,
        allocated_change: node.allocated_change.map(project_snapshot_diff_value),
        current_file_count: node.current_file_count,
        baseline_file_count: node.baseline_file_count,
        current_child_count: node.current_child_count,
        baseline_child_count: node.baseline_child_count,
        current_scan_flags: node.current_scan_flags.map(project_snapshot_scan_flags),
        baseline_scan_flags: node.baseline_scan_flags.map(project_snapshot_scan_flags),
        can_descend: node.can_descend,
    }
}

const fn project_snapshot_diff_value(value: CoreSnapshotDiffValue) -> SnapshotDiffValue {
    SnapshotDiffValue {
        direction: match value.direction {
            CoreSnapshotDiffDirection::Growth => SnapshotDiffDirection::Growth,
            CoreSnapshotDiffDirection::Shrinkage => SnapshotDiffDirection::Shrinkage,
            CoreSnapshotDiffDirection::Unchanged => SnapshotDiffDirection::Unchanged,
        },
        magnitude_bytes: value.magnitude_bytes,
    }
}

fn project_snapshot_diff_node_page(page: CoreSnapshotDiffNodePage) -> SnapshotDiffNodePage {
    SnapshotDiffNodePage {
        record_version: FFI_RECORD_VERSION,
        parent_id: page.parent_id,
        offset: page.offset,
        total_children: page.total_children,
        has_more: page.has_more,
        total_growth_bytes: page.total_growth_bytes,
        total_shrinkage_bytes: page.total_shrinkage_bytes,
        unchanged_child_count: page.unchanged_child_count,
        replaced_child_count: page.replaced_child_count,
        nodes: page
            .nodes
            .into_iter()
            .map(project_snapshot_diff_node)
            .collect(),
    }
}

fn project_snapshot_diff_treemap(treemap: CoreSnapshotDiffTreemap) -> SnapshotDiffTreemap {
    SnapshotDiffTreemap {
        record_version: FFI_RECORD_VERSION,
        parent_id: treemap.parent_id,
        total_children: treemap.total_children,
        changed_child_count: treemap.changed_child_count,
        total_growth_bytes: treemap.total_growth_bytes,
        total_shrinkage_bytes: treemap.total_shrinkage_bytes,
        other_growth_child_count: treemap.other_growth_child_count,
        other_growth_bytes: treemap.other_growth_bytes,
        other_shrinkage_child_count: treemap.other_shrinkage_child_count,
        other_shrinkage_bytes: treemap.other_shrinkage_bytes,
        unchanged_child_count: treemap.unchanged_child_count,
        replaced_child_count: treemap.replaced_child_count,
        cells: treemap
            .cells
            .into_iter()
            .map(project_snapshot_diff_treemap_cell)
            .collect(),
    }
}

fn project_snapshot_diff_treemap_cell(
    cell: CoreSnapshotDiffTreemapCell,
) -> SnapshotDiffTreemapCell {
    SnapshotDiffTreemapCell {
        record_version: FFI_RECORD_VERSION,
        node: project_snapshot_diff_node(cell.node),
        magnitude_rank: cell.magnitude_rank,
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

fn project_snapshot_icloud_observation_source(
    scan_id: String,
    source: CoreReviewICloudObservationSource,
) -> SnapshotICloudObservationSource {
    SnapshotICloudObservationSource {
        record_version: FFI_RECORD_VERSION,
        scan_id,
        scope_node_id: source.scope_node_id,
        requested_max_results: source.requested_max_results,
        visited_node_count: source.visited_node_count,
        total_ranked_files: source.total_ranked_files,
        has_more: source.has_more,
        targets: source
            .targets
            .into_iter()
            .map(project_snapshot_icloud_observation_target)
            .collect(),
    }
}

fn project_snapshot_icloud_observation_target(
    target: CoreReviewICloudObservationTarget,
) -> SnapshotICloudObservationTarget {
    SnapshotICloudObservationTarget {
        record_version: FFI_RECORD_VERSION,
        rank: target.rank,
        node: project_snapshot_node(target.node),
        parent_context: target
            .parent_context
            .into_iter()
            .map(project_snapshot_node_name)
            .collect(),
        context_truncated: target.context_truncated,
    }
}

fn project_snapshot_node(node: CoreReviewNode) -> SnapshotNode {
    SnapshotNode {
        record_version: SNAPSHOT_NODE_RECORD_VERSION,
        id: node.id,
        parent_id: node.parent_id,
        depth: node.depth,
        kind: project_snapshot_node_kind(node.kind),
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
        scan_flags: project_snapshot_scan_flags(node.scan_flags),
    }
}

const fn project_snapshot_node_kind(kind: CoreReviewNodeKind) -> SnapshotNodeKind {
    match kind {
        CoreReviewNodeKind::Directory => SnapshotNodeKind::Directory,
        CoreReviewNodeKind::File => SnapshotNodeKind::File,
        CoreReviewNodeKind::Symlink => SnapshotNodeKind::Symlink,
        CoreReviewNodeKind::Other => SnapshotNodeKind::Other,
        CoreReviewNodeKind::Error => SnapshotNodeKind::Error,
    }
}

const fn project_snapshot_scan_flags(
    flags: dux_core::engine::SnapshotReviewScanFlags,
) -> SnapshotNodeScanFlags {
    SnapshotNodeScanFlags {
        inaccessible: flags.inaccessible,
        timed_out: flags.timed_out,
        hard_link_duplicate: flags.hard_link_duplicate,
        mount_boundary: flags.mount_boundary,
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

fn map_rule_outcome_error(error: CoreRuleOutcomeError) -> RuleOutcomeError {
    match error {
        CoreRuleOutcomeError::Closed => RuleOutcomeError::Closed,
        CoreRuleOutcomeError::SessionNotFound => RuleOutcomeError::SessionNotFound,
        CoreRuleOutcomeError::IncompatibleSchema => RuleOutcomeError::IncompatibleSchema,
        CoreRuleOutcomeError::Busy => RuleOutcomeError::Busy,
        CoreRuleOutcomeError::UnsafeStorage => RuleOutcomeError::UnsafeStorage,
        CoreRuleOutcomeError::QueryLimitExceeded => RuleOutcomeError::BudgetExceeded,
        CoreRuleOutcomeError::CorruptData => RuleOutcomeError::CorruptData,
        CoreRuleOutcomeError::Unavailable => RuleOutcomeError::Unavailable,
        CoreRuleOutcomeError::InternalState => RuleOutcomeError::InternalState,
        _ => RuleOutcomeError::InternalState,
    }
}

fn rule_outcome_time_ms(value: SystemTime) -> Result<i64, RuleOutcomeError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RuleOutcomeError::CorruptData)?
            .as_millis(),
    )
    .map_err(|_| RuleOutcomeError::CorruptData)
}

fn project_rule_outcome_state(
    state: &CoreRuleOutcomeState,
) -> Result<RuleOutcomeState, RuleOutcomeError> {
    match state {
        CoreRuleOutcomeState::NotEligible { reason } => Ok(RuleOutcomeState::NotEligible {
            reason: match reason {
                CoreRuleOutcomeNotEligibleReason::SourceCleanupIncomplete => {
                    RuleOutcomeNotEligibleReason::SourceCleanupIncomplete
                }
                CoreRuleOutcomeNotEligibleReason::ItemNotSuccessfulPermanentRegenerable => {
                    RuleOutcomeNotEligibleReason::ItemNotSuccessfulPermanentRegenerable
                }
                CoreRuleOutcomeNotEligibleReason::SourceScanNotComparable => {
                    RuleOutcomeNotEligibleReason::SourceScanNotComparable
                }
                CoreRuleOutcomeNotEligibleReason::SourceEvaluationNotComparable => {
                    RuleOutcomeNotEligibleReason::SourceEvaluationNotComparable
                }
                CoreRuleOutcomeNotEligibleReason::SourceEvaluationAfterPlan => {
                    RuleOutcomeNotEligibleReason::SourceEvaluationAfterPlan
                }
                CoreRuleOutcomeNotEligibleReason::SourceCandidateMismatch => {
                    RuleOutcomeNotEligibleReason::SourceCandidateMismatch
                }
                _ => return Err(RuleOutcomeError::InternalState),
            },
        }),
        CoreRuleOutcomeState::AwaitingComparableScan { cleaned_at } => {
            Ok(RuleOutcomeState::AwaitingComparableScan {
                cleaned_at_unix_ms: rule_outcome_time_ms(*cleaned_at)?,
            })
        }
        CoreRuleOutcomeState::Superseded {
            cleaned_at,
            superseded_at,
        } => {
            if superseded_at < cleaned_at {
                return Err(RuleOutcomeError::CorruptData);
            }
            let cleaned_at_unix_ms = rule_outcome_time_ms(*cleaned_at)?;
            let superseded_at_unix_ms = rule_outcome_time_ms(*superseded_at)?;
            if superseded_at_unix_ms < cleaned_at_unix_ms {
                return Err(RuleOutcomeError::CorruptData);
            }
            Ok(RuleOutcomeState::Superseded {
                cleaned_at_unix_ms,
                superseded_at_unix_ms,
            })
        }
        CoreRuleOutcomeState::LaterSizeObserved {
            cleaned_at,
            observed_at,
            observed_bytes,
        } => {
            if observed_at <= cleaned_at || *observed_bytes == 0 {
                return Err(RuleOutcomeError::CorruptData);
            }
            let cleaned_at_unix_ms = rule_outcome_time_ms(*cleaned_at)?;
            let observed_at_unix_ms = rule_outcome_time_ms(*observed_at)?;
            if observed_at_unix_ms <= cleaned_at_unix_ms {
                return Err(RuleOutcomeError::CorruptData);
            }
            Ok(RuleOutcomeState::LaterSizeObserved {
                cleaned_at_unix_ms,
                observed_at_unix_ms,
                observed_bytes: *observed_bytes,
            })
        }
        CoreRuleOutcomeState::ZeroBaselineObserved {
            cleaned_at,
            observed_at,
        } => {
            if observed_at <= cleaned_at {
                return Err(RuleOutcomeError::CorruptData);
            }
            let cleaned_at_unix_ms = rule_outcome_time_ms(*cleaned_at)?;
            let observed_at_unix_ms = rule_outcome_time_ms(*observed_at)?;
            if observed_at_unix_ms <= cleaned_at_unix_ms {
                return Err(RuleOutcomeError::CorruptData);
            }
            Ok(RuleOutcomeState::ZeroBaselineObserved {
                cleaned_at_unix_ms,
                observed_at_unix_ms,
            })
        }
        CoreRuleOutcomeState::Regrown {
            cleaned_at,
            zero_observed_at,
            observed_at,
            observed_bytes,
            regrowth_duration,
        } => {
            let exact_duration = observed_at
                .duration_since(*zero_observed_at)
                .map_err(|_| RuleOutcomeError::CorruptData)?;
            if zero_observed_at <= cleaned_at
                || observed_at <= zero_observed_at
                || *observed_bytes == 0
                || exact_duration != *regrowth_duration
            {
                return Err(RuleOutcomeError::CorruptData);
            }
            let cleaned_at_unix_ms = rule_outcome_time_ms(*cleaned_at)?;
            let zero_observed_at_unix_ms = rule_outcome_time_ms(*zero_observed_at)?;
            let observed_at_unix_ms = rule_outcome_time_ms(*observed_at)?;
            if zero_observed_at_unix_ms <= cleaned_at_unix_ms
                || observed_at_unix_ms <= zero_observed_at_unix_ms
            {
                return Err(RuleOutcomeError::CorruptData);
            }
            Ok(RuleOutcomeState::Regrown {
                cleaned_at_unix_ms,
                zero_observed_at_unix_ms,
                observed_at_unix_ms,
                observed_bytes: *observed_bytes,
            })
        }
        _ => Err(RuleOutcomeError::InternalState),
    }
}

fn rule_outcome_batch(
    batch: CoreRuleOutcomeBatch,
    requested_session_id: &str,
) -> Result<RuleOutcomeBatch, RuleOutcomeError> {
    if batch.session_id().as_str() != requested_session_id
        || batch.outcomes().len() > MAX_RULE_OUTCOMES
    {
        return Err(RuleOutcomeError::CorruptData);
    }
    let outcomes = batch
        .outcomes()
        .iter()
        .enumerate()
        .map(|(expected_ordinal, outcome)| {
            if usize::from(outcome.item_ordinal()) != expected_ordinal {
                return Err(RuleOutcomeError::CorruptData);
            }
            let rule_id = outcome.rule().id().as_str().to_owned();
            let rule_revision = outcome.rule().revision().get();
            if !is_bounded_cleanup_history_token(&rule_id, MAX_CLEANUP_HISTORY_RULE_ID_BYTES)
                || rule_revision == 0
            {
                return Err(RuleOutcomeError::CorruptData);
            }
            let state = project_rule_outcome_state(outcome.state())?;
            Ok(RuleOutcome {
                record_version: FFI_RECORD_VERSION,
                item_ordinal: outcome.item_ordinal(),
                rule_id,
                rule_revision,
                state,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RuleOutcomeBatch {
        record_version: FFI_RECORD_VERSION,
        session_id: requested_session_id.to_owned(),
        outcomes,
    })
}

fn map_storage_thief_error(error: CoreStorageThiefError) -> StorageThiefError {
    match error {
        CoreStorageThiefError::Closed => StorageThiefError::Closed,
        CoreStorageThiefError::IncompatibleSchema => StorageThiefError::IncompatibleSchema,
        CoreStorageThiefError::Busy => StorageThiefError::Busy,
        CoreStorageThiefError::UnsafeStorage => StorageThiefError::UnsafeStorage,
        CoreStorageThiefError::QueryLimitExceeded => StorageThiefError::BudgetExceeded,
        CoreStorageThiefError::CorruptData => StorageThiefError::CorruptData,
        CoreStorageThiefError::Unavailable => StorageThiefError::Unavailable,
        CoreStorageThiefError::InternalState => StorageThiefError::InternalState,
        _ => StorageThiefError::InternalState,
    }
}

fn map_running_scan_debt_census_error(
    error: CoreRunningScanDebtCensusError,
) -> RunningScanDebtCensusError {
    match error {
        CoreRunningScanDebtCensusError::Closed => RunningScanDebtCensusError::Closed,
        CoreRunningScanDebtCensusError::IncompatibleSchema => {
            RunningScanDebtCensusError::IncompatibleSchema
        }
        CoreRunningScanDebtCensusError::Busy => RunningScanDebtCensusError::Busy,
        CoreRunningScanDebtCensusError::UnsafeStorage => RunningScanDebtCensusError::UnsafeStorage,
        CoreRunningScanDebtCensusError::QueryLimitExceeded => {
            RunningScanDebtCensusError::BudgetExceeded
        }
        CoreRunningScanDebtCensusError::CorruptData => RunningScanDebtCensusError::CorruptData,
        CoreRunningScanDebtCensusError::Unavailable => RunningScanDebtCensusError::Unavailable,
        CoreRunningScanDebtCensusError::InternalState => RunningScanDebtCensusError::InternalState,
        _ => RunningScanDebtCensusError::InternalState,
    }
}

fn running_scan_debt_census(
    census: CoreRunningScanDebtCensus,
) -> Result<RunningScanDebtCensus, RunningScanDebtCensusError> {
    let inspected = census.inspected_unclaimed_count();
    let pristine = census.pristine_unclaimed_count();
    let unexplained = census.unexplained_unclaimed_count();
    if inspected > MAX_RUNNING_SCAN_DEBT_CENSUS_ROWS
        || pristine.checked_add(unexplained) != Some(inspected)
        || (census.has_more() && inspected != MAX_RUNNING_SCAN_DEBT_CENSUS_ROWS)
    {
        return Err(RunningScanDebtCensusError::CorruptData);
    }
    Ok(RunningScanDebtCensus {
        record_version: FFI_RECORD_VERSION,
        inspected_unclaimed_count: inspected,
        pristine_unclaimed_count: pristine,
        unexplained_unclaimed_count: unexplained,
        has_more: census.has_more(),
    })
}

fn map_claimed_running_scan_provenance_census_error(
    error: CoreClaimedRunningScanProvenanceCensusError,
) -> ClaimedRunningScanProvenanceCensusError {
    match error {
        CoreClaimedRunningScanProvenanceCensusError::Closed => {
            ClaimedRunningScanProvenanceCensusError::Closed
        }
        CoreClaimedRunningScanProvenanceCensusError::IncompatibleSchema => {
            ClaimedRunningScanProvenanceCensusError::IncompatibleSchema
        }
        CoreClaimedRunningScanProvenanceCensusError::Busy => {
            ClaimedRunningScanProvenanceCensusError::Busy
        }
        CoreClaimedRunningScanProvenanceCensusError::UnsafeStorage => {
            ClaimedRunningScanProvenanceCensusError::UnsafeStorage
        }
        CoreClaimedRunningScanProvenanceCensusError::QueryLimitExceeded => {
            ClaimedRunningScanProvenanceCensusError::BudgetExceeded
        }
        CoreClaimedRunningScanProvenanceCensusError::CorruptData => {
            ClaimedRunningScanProvenanceCensusError::CorruptData
        }
        CoreClaimedRunningScanProvenanceCensusError::Unavailable => {
            ClaimedRunningScanProvenanceCensusError::Unavailable
        }
        CoreClaimedRunningScanProvenanceCensusError::InternalState => {
            ClaimedRunningScanProvenanceCensusError::InternalState
        }
        _ => ClaimedRunningScanProvenanceCensusError::InternalState,
    }
}

fn claimed_running_scan_provenance_census(
    census: CoreClaimedRunningScanProvenanceCensus,
) -> Result<ClaimedRunningScanProvenanceCensus, ClaimedRunningScanProvenanceCensusError> {
    project_claimed_running_scan_provenance_census(
        census.inspected_claimed_count(),
        census.same_host_current_boot_count(),
        census.same_host_prior_boot_count(),
        census.foreign_host_count(),
        census.stored_unproven_count(),
        census.current_context_unavailable_count(),
        census.has_more(),
    )
}

fn project_claimed_running_scan_provenance_census(
    inspected_claimed_count: u16,
    same_host_current_boot_count: u16,
    same_host_prior_boot_count: u16,
    foreign_host_count: u16,
    stored_unproven_count: u16,
    current_context_unavailable_count: u16,
    has_more: bool,
) -> Result<ClaimedRunningScanProvenanceCensus, ClaimedRunningScanProvenanceCensusError> {
    let classified_count = same_host_current_boot_count
        .checked_add(same_host_prior_boot_count)
        .and_then(|count| count.checked_add(foreign_host_count))
        .and_then(|count| count.checked_add(stored_unproven_count))
        .and_then(|count| count.checked_add(current_context_unavailable_count));
    if inspected_claimed_count > MAX_CLAIMED_RUNNING_SCAN_PROVENANCE_CENSUS_ROWS
        || classified_count != Some(inspected_claimed_count)
        || (has_more && inspected_claimed_count != MAX_CLAIMED_RUNNING_SCAN_PROVENANCE_CENSUS_ROWS)
        || (current_context_unavailable_count > 0
            && (same_host_current_boot_count > 0
                || same_host_prior_boot_count > 0
                || foreign_host_count > 0))
    {
        return Err(ClaimedRunningScanProvenanceCensusError::CorruptData);
    }
    Ok(ClaimedRunningScanProvenanceCensus {
        record_version: FFI_RECORD_VERSION,
        inspected_claimed_count,
        same_host_current_boot_count,
        same_host_prior_boot_count,
        foreign_host_count,
        stored_unproven_count,
        current_context_unavailable_count,
        has_more,
    })
}

fn storage_thief_time_ms(value: SystemTime) -> Result<i64, StorageThiefError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| StorageThiefError::CorruptData)?
            .as_millis(),
    )
    .map_err(|_| StorageThiefError::CorruptData)
}

fn storage_thief_ranking(
    ranking: CoreStorageThiefRanking,
) -> Result<StorageThiefRanking, StorageThiefError> {
    if ranking.permanent_safe_session_count() > MAX_STORAGE_THIEF_SOURCE_SESSIONS
        || ranking.manual_cleanup_session_count() > ranking.permanent_safe_session_count()
        || ranking.groups().len() > MAX_STORAGE_THIEF_GROUPS
        || ranking.groups().len() > usize::from(ranking.ranked_rule_count())
    {
        return Err(StorageThiefError::CorruptData);
    }
    let mut rule_ids = std::collections::HashSet::new();
    let mut groups = Vec::with_capacity(ranking.groups().len());
    for (index, group) in ranking.groups().iter().enumerate() {
        let rule_id = group.latest_rule().id().as_str().to_owned();
        let duration = group.total_regrowth_duration();
        if !is_bounded_cleanup_history_token(&rule_id, MAX_CLEANUP_HISTORY_RULE_ID_BYTES)
            || !rule_ids.insert(rule_id.clone())
            || group.latest_rule().revision().get() == 0
            || group.observed_revision_count() == 0
            || group.successful_cleanup_count() == 0
            || group.successful_manual_cleanup_count() > group.successful_cleanup_count()
            || group.observed_regrowth_cycle_count() == 0
            || group.manual_regrowth_cycle_count() > group.observed_regrowth_cycle_count()
            || group.total_observed_regrown_bytes() == 0
            || duration.is_zero()
            || (group.automation_history_threshold_met()
                && (group.successful_manual_cleanup_count() < 2
                    || group.manual_regrowth_cycle_count() == 0))
        {
            return Err(StorageThiefError::CorruptData);
        }
        groups.push(StorageThiefGroup {
            record_version: FFI_RECORD_VERSION,
            rank: u16::try_from(index + 1).map_err(|_| StorageThiefError::CorruptData)?,
            rule_id,
            latest_rule_revision: group.latest_rule().revision().get(),
            observed_revision_count: group.observed_revision_count(),
            successful_cleanup_count: group.successful_cleanup_count(),
            successful_manual_cleanup_count: group.successful_manual_cleanup_count(),
            observed_regrowth_cycle_count: group.observed_regrowth_cycle_count(),
            manual_regrowth_cycle_count: group.manual_regrowth_cycle_count(),
            total_observed_regrown_bytes: group.total_observed_regrown_bytes(),
            total_regrowth_duration_seconds: duration.as_secs(),
            total_regrowth_duration_nanoseconds: duration.subsec_nanos(),
            bytes_regrown_per_day: group.bytes_regrown_per_day(),
            rate_capped: group.rate_capped(),
            latest_cleanup_at_unix_ms: storage_thief_time_ms(group.latest_cleanup_at())?,
            latest_regrowth_at_unix_ms: storage_thief_time_ms(group.latest_regrowth_at())?,
            automation_history_threshold_met: group.automation_history_threshold_met(),
        });
    }
    for pair in groups.windows(2) {
        if ffi_storage_thief_group_order(&pair[0], &pair[1]) == std::cmp::Ordering::Greater {
            return Err(StorageThiefError::CorruptData);
        }
    }
    Ok(StorageThiefRanking {
        record_version: FFI_RECORD_VERSION,
        permanent_safe_session_count: ranking.permanent_safe_session_count(),
        manual_cleanup_session_count: ranking.manual_cleanup_session_count(),
        ranked_rule_count: ranking.ranked_rule_count(),
        has_older_permanent_safe_sessions: ranking.has_older_permanent_safe_sessions(),
        groups,
    })
}

fn ffi_storage_thief_group_order(
    left: &StorageThiefGroup,
    right: &StorageThiefGroup,
) -> std::cmp::Ordering {
    compare_ffi_storage_thief_rates(right, left)
        .then_with(|| {
            right
                .successful_cleanup_count
                .cmp(&left.successful_cleanup_count)
        })
        .then_with(|| {
            right
                .observed_regrowth_cycle_count
                .cmp(&left.observed_regrowth_cycle_count)
        })
        .then_with(|| {
            right
                .latest_regrowth_at_unix_ms
                .cmp(&left.latest_regrowth_at_unix_ms)
        })
        .then_with(|| left.rule_id.cmp(&right.rule_id))
}

fn compare_ffi_storage_thief_rates(
    left: &StorageThiefGroup,
    right: &StorageThiefGroup,
) -> std::cmp::Ordering {
    let left_duration = u128::from(left.total_regrowth_duration_seconds) * 1_000_000_000
        + u128::from(left.total_regrowth_duration_nanoseconds);
    let right_duration = u128::from(right.total_regrowth_duration_seconds) * 1_000_000_000
        + u128::from(right.total_regrowth_duration_nanoseconds);
    compare_positive_ffi_fractions(
        u128::from(left.total_observed_regrown_bytes),
        left_duration,
        u128::from(right.total_observed_regrown_bytes),
        right_duration,
    )
}

fn compare_positive_ffi_fractions(
    mut left_numerator: u128,
    mut left_denominator: u128,
    mut right_numerator: u128,
    mut right_denominator: u128,
) -> std::cmp::Ordering {
    debug_assert!(left_denominator > 0 && right_denominator > 0);
    let mut inverted = false;
    loop {
        let left_quotient = left_numerator / left_denominator;
        let right_quotient = right_numerator / right_denominator;
        if left_quotient != right_quotient {
            let ordering = left_quotient.cmp(&right_quotient);
            return if inverted {
                ordering.reverse()
            } else {
                ordering
            };
        }
        let left_remainder = left_numerator % left_denominator;
        let right_remainder = right_numerator % right_denominator;
        match (left_remainder == 0, right_remainder == 0) {
            (true, true) => return std::cmp::Ordering::Equal,
            (true, false) => {
                return if inverted {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Less
                };
            }
            (false, true) => {
                return if inverted {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            (false, false) => {
                left_numerator = left_denominator;
                left_denominator = left_remainder;
                right_numerator = right_denominator;
                right_denominator = right_remainder;
                inverted = !inverted;
            }
        }
    }
}

fn map_cleanup_history_clear_error(
    error: CoreCleanupHistoryClearError,
) -> CleanupHistoryClearError {
    match error {
        CoreCleanupHistoryClearError::Closed => CleanupHistoryClearError::Closed,
        CoreCleanupHistoryClearError::NothingToClear => CleanupHistoryClearError::NothingToClear,
        CoreCleanupHistoryClearError::ActiveCleanup => CleanupHistoryClearError::ActiveCleanup,
        CoreCleanupHistoryClearError::ChangedSincePreview => {
            CleanupHistoryClearError::ChangedSincePreview
        }
        CoreCleanupHistoryClearError::PreviewExpired => CleanupHistoryClearError::PreviewExpired,
        CoreCleanupHistoryClearError::WrongEngine => CleanupHistoryClearError::WrongEngine,
        CoreCleanupHistoryClearError::IncompatibleSchema => {
            CleanupHistoryClearError::IncompatibleSchema
        }
        CoreCleanupHistoryClearError::Busy => CleanupHistoryClearError::Busy,
        CoreCleanupHistoryClearError::UnsafeStorage => CleanupHistoryClearError::UnsafeStorage,
        CoreCleanupHistoryClearError::QueryLimitExceeded => {
            CleanupHistoryClearError::BudgetExceeded
        }
        CoreCleanupHistoryClearError::CorruptData => CleanupHistoryClearError::CorruptData,
        CoreCleanupHistoryClearError::OutcomeUnknown => CleanupHistoryClearError::OutcomeUnknown,
        CoreCleanupHistoryClearError::Unavailable => CleanupHistoryClearError::Unavailable,
        CoreCleanupHistoryClearError::InternalState => CleanupHistoryClearError::InternalState,
        _ => CleanupHistoryClearError::InternalState,
    }
}

fn cleanup_history_clear_time_ms(value: SystemTime) -> Result<i64, CleanupHistoryClearError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CleanupHistoryClearError::CorruptData)?
            .as_millis(),
    )
    .map_err(|_| CleanupHistoryClearError::CorruptData)
}

fn cleanup_history_clear_preview_info(
    info: CoreCleanupHistoryClearPreviewInfo,
) -> Result<CleanupHistoryClearPreviewInfo, CleanupHistoryClearError> {
    cleanup_history_clear_preview_info_values(
        info.session_count(),
        info.oldest_started_at(),
        info.newest_started_at(),
        info.prepared_at(),
        info.expires_at(),
    )
}

fn cleanup_history_clear_preview_info_values(
    session_count: u64,
    oldest_started_at: SystemTime,
    newest_started_at: SystemTime,
    prepared_at: SystemTime,
    expires_at: SystemTime,
) -> Result<CleanupHistoryClearPreviewInfo, CleanupHistoryClearError> {
    let projected = CleanupHistoryClearPreviewInfo {
        record_version: FFI_RECORD_VERSION,
        session_count,
        oldest_started_at_unix_ms: cleanup_history_clear_time_ms(oldest_started_at)?,
        newest_started_at_unix_ms: cleanup_history_clear_time_ms(newest_started_at)?,
        prepared_at_unix_ms: cleanup_history_clear_time_ms(prepared_at)?,
        expires_at_unix_ms: cleanup_history_clear_time_ms(expires_at)?,
    };
    if projected.session_count == 0
        || projected.oldest_started_at_unix_ms > projected.newest_started_at_unix_ms
        || projected.prepared_at_unix_ms >= projected.expires_at_unix_ms
    {
        return Err(CleanupHistoryClearError::CorruptData);
    }
    Ok(projected)
}

fn cleanup_history_clear_result(
    result: CoreCleanupHistoryClearResult,
    expected_session_count: u64,
) -> Result<CleanupHistoryClearResult, CleanupHistoryClearError> {
    cleanup_history_clear_result_count(result.cleared_session_count(), expected_session_count)
}

fn cleanup_history_clear_result_count(
    cleared_session_count: u64,
    expected_session_count: u64,
) -> Result<CleanupHistoryClearResult, CleanupHistoryClearError> {
    if cleared_session_count == 0 || cleared_session_count != expected_session_count {
        return Err(CleanupHistoryClearError::OutcomeUnknown);
    }
    Ok(CleanupHistoryClearResult {
        record_version: FFI_RECORD_VERSION,
        cleared_session_count,
    })
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

fn map_pressure_episode_history_error(error: CorePressureEpisodeHistoryError) -> EngineError {
    match error {
        CorePressureEpisodeHistoryError::Closed => EngineError::Closed,
        CorePressureEpisodeHistoryError::InvalidLimit { .. }
        | CorePressureEpisodeHistoryError::InvalidAnchor => {
            EngineError::InvalidPressureEpisodeRequest
        }
        CorePressureEpisodeHistoryError::IncompatibleSchema => EngineError::IncompatibleSchema,
        CorePressureEpisodeHistoryError::Busy => EngineError::Busy,
        CorePressureEpisodeHistoryError::UnsafeStorage => EngineError::UnsafeStorage,
        CorePressureEpisodeHistoryError::BudgetExceeded => EngineError::BudgetExceeded,
        CorePressureEpisodeHistoryError::CorruptData => EngineError::CorruptData,
        CorePressureEpisodeHistoryError::Unavailable => EngineError::StorageUnavailable,
        CorePressureEpisodeHistoryError::OutcomeUnknown => EngineError::OutcomeUnknown,
        CorePressureEpisodeHistoryError::InternalState => EngineError::InternalState,
    }
}

fn pressure_episode_history_status(
    history: CorePressureEpisodeHistory,
) -> Result<PressureEpisodeHistoryStatus, EngineError> {
    let episodes = history
        .episodes()
        .iter()
        .map(|episode| {
            Ok(PressureEpisodeRecord {
                record_version: FFI_RECORD_VERSION,
                level: match episode.level() {
                    CorePressureEpisodeLevel::Warning => PressureEpisodeLevel::Warning,
                    CorePressureEpisodeLevel::Critical => PressureEpisodeLevel::Critical,
                },
                entered_at_unix_ms: system_time_ms(episode.entered_at())?,
                exited_at_unix_ms: episode.exited_at().map(system_time_ms).transpose()?,
                policy_revision: episode.policy_revision(),
            })
        })
        .collect::<Result<Vec<_>, EngineError>>()?;
    Ok(PressureEpisodeHistoryStatus {
        record_version: FFI_RECORD_VERSION,
        stable_volume_id: history.volume_id().to_string(),
        anchor_at_unix_ms: system_time_ms(history.anchor_at())?,
        episodes,
        has_more: history.has_more(),
    })
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

fn map_configured_project_roots_error(
    error: CoreConfiguredProjectRootsError,
) -> ConfiguredProjectRootsError {
    match error {
        CoreConfiguredProjectRootsError::Closed => ConfiguredProjectRootsError::Closed,
        CoreConfiguredProjectRootsError::InvalidInput => ConfiguredProjectRootsError::InvalidPath,
        CoreConfiguredProjectRootsError::RevisionExhausted => {
            ConfiguredProjectRootsError::RevisionExhausted
        }
        CoreConfiguredProjectRootsError::InvalidClock => ConfiguredProjectRootsError::InvalidClock,
        CoreConfiguredProjectRootsError::IncompatibleSchema => {
            ConfiguredProjectRootsError::IncompatibleSchema
        }
        CoreConfiguredProjectRootsError::Busy => ConfiguredProjectRootsError::Busy,
        CoreConfiguredProjectRootsError::UnsafeStorage => {
            ConfiguredProjectRootsError::UnsafeStorage
        }
        CoreConfiguredProjectRootsError::QueryLimitExceeded => {
            ConfiguredProjectRootsError::BudgetExceeded
        }
        CoreConfiguredProjectRootsError::CorruptData => ConfiguredProjectRootsError::CorruptData,
        CoreConfiguredProjectRootsError::Unavailable => ConfiguredProjectRootsError::Unavailable,
        CoreConfiguredProjectRootsError::OutcomeUnknown => {
            ConfiguredProjectRootsError::OutcomeUnknown
        }
        CoreConfiguredProjectRootsError::InternalState => {
            ConfiguredProjectRootsError::InternalState
        }
        _ => ConfiguredProjectRootsError::InternalState,
    }
}

fn configured_project_roots_status(
    settings: CoreConfiguredProjectRoots,
) -> Result<ConfiguredProjectRootsStatus, ConfiguredProjectRootsError> {
    if settings.roots.len() > MAX_CONFIGURED_PROJECT_ROOT_COUNT {
        return Err(ConfiguredProjectRootsError::InternalState);
    }
    let updated_at_unix_ms = settings
        .updated_at
        .map(configured_project_roots_time_ms)
        .transpose()?;
    if (settings.revision == 0) != updated_at_unix_ms.is_none()
        || (settings.revision == 0
            && (settings.source != CoreConfiguredProjectRootsSource::Default
                || !settings.roots.is_empty()))
        || (settings.revision > 0 && settings.source != CoreConfiguredProjectRootsSource::Stored)
    {
        return Err(ConfiguredProjectRootsError::InternalState);
    }
    let roots = settings
        .roots
        .iter()
        .map(|path| configured_project_root_path(path.as_path()))
        .collect::<Result<Vec<_>, _>>()?;
    if roots.windows(2).any(|pair| {
        configured_project_root_order_key(&pair[0]) >= configured_project_root_order_key(&pair[1])
    }) || settings.roots.iter().enumerate().any(|(index, root)| {
        settings
            .roots
            .iter()
            .skip(index + 1)
            .any(|other| root.starts_with(other) || other.starts_with(root))
    }) {
        return Err(ConfiguredProjectRootsError::InternalState);
    }
    Ok(ConfiguredProjectRootsStatus {
        record_version: FFI_RECORD_VERSION,
        roots,
        source: match settings.source {
            CoreConfiguredProjectRootsSource::Default => ConfiguredProjectRootsSource::Default,
            CoreConfiguredProjectRootsSource::Stored => ConfiguredProjectRootsSource::Stored,
        },
        revision: settings.revision,
        updated_at_unix_ms,
    })
}

fn configured_project_root_order_key(path: &ConfiguredProjectRootPath) -> (u8, &[u8]) {
    let encoding = match path.encoding {
        SnapshotNameEncoding::UnixBytes => 0,
        SnapshotNameEncoding::WindowsUtf16LittleEndian => 1,
    };
    (encoding, path.encoded_bytes.as_slice())
}

fn configured_project_roots_update(
    update: CoreConfiguredProjectRootsUpdate,
) -> Result<ConfiguredProjectRootsUpdate, ConfiguredProjectRootsError> {
    Ok(ConfiguredProjectRootsUpdate {
        record_version: FFI_RECORD_VERSION,
        roots: configured_project_roots_status(update.settings)?,
        changed: update.changed,
    })
}

fn configured_project_roots_time_ms(value: SystemTime) -> Result<i64, ConfiguredProjectRootsError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ConfiguredProjectRootsError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| ConfiguredProjectRootsError::InternalState)
}

fn decode_configured_project_roots_input(
    input: ConfiguredProjectRootsInput,
) -> Result<Vec<PathBuf>, ConfiguredProjectRootsError> {
    if input.record_version != FFI_RECORD_VERSION {
        return Err(ConfiguredProjectRootsError::InvalidRecordVersion);
    }
    if input.roots.len() > MAX_CONFIGURED_PROJECT_ROOT_COUNT {
        return Err(ConfiguredProjectRootsError::TooManyPaths);
    }
    let roots = input
        .roots
        .into_iter()
        .map(decode_configured_project_root_path)
        .collect::<Result<Vec<_>, _>>()?;
    if roots.iter().enumerate().any(|(index, root)| {
        roots
            .iter()
            .skip(index + 1)
            .any(|other| root.starts_with(other) || other.starts_with(root))
    }) {
        return Err(ConfiguredProjectRootsError::OverlappingPaths);
    }
    Ok(roots)
}

fn decode_configured_project_root_path(
    encoded: ConfiguredProjectRootPath,
) -> Result<PathBuf, ConfiguredProjectRootsError> {
    if encoded.encoded_bytes.is_empty()
        || encoded.encoded_bytes.len() > MAX_CONFIGURED_PROJECT_ROOT_PATH_BYTES
    {
        return Err(ConfiguredProjectRootsError::InvalidPath);
    }
    let path = match encoded.encoding {
        SnapshotNameEncoding::UnixBytes => {
            #[cfg(unix)]
            {
                if !is_normalized_absolute_unix_project_root(&encoded.encoded_bytes) {
                    return Err(ConfiguredProjectRootsError::InvalidPath);
                }
                PathBuf::from(std::ffi::OsString::from_vec(encoded.encoded_bytes))
            }
            #[cfg(not(unix))]
            {
                return Err(ConfiguredProjectRootsError::InvalidPath);
            }
        }
        SnapshotNameEncoding::WindowsUtf16LittleEndian => {
            #[cfg(windows)]
            {
                if encoded.encoded_bytes.len() % 2 != 0 {
                    return Err(ConfiguredProjectRootsError::InvalidPath);
                }
                let units = encoded
                    .encoded_bytes
                    .chunks_exact(2)
                    .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                    .collect::<Vec<_>>();
                if units.contains(&0) {
                    return Err(ConfiguredProjectRootsError::InvalidPath);
                }
                let path = PathBuf::from(std::ffi::OsString::from_wide(&units));
                if !is_supported_windows_project_root(&path) {
                    return Err(ConfiguredProjectRootsError::InvalidPath);
                }
                path
            }
            #[cfg(not(windows))]
            {
                return Err(ConfiguredProjectRootsError::InvalidPath);
            }
        }
    };
    let normalized: PathBuf = path.components().collect();
    if !path.is_absolute()
        || path.parent().is_none()
        || normalized.as_os_str() != path.as_os_str()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(ConfiguredProjectRootsError::InvalidPath);
    }
    Ok(path)
}

#[cfg(unix)]
fn is_normalized_absolute_unix_project_root(bytes: &[u8]) -> bool {
    bytes.len() > 1
        && bytes[0] == b'/'
        && bytes.last() != Some(&b'/')
        && !bytes.contains(&0)
        && bytes[1..].split(|byte| *byte == b'/').all(|component| {
            !component.is_empty()
                && component != b"."
                && component != b".."
                && !component.iter().any(|byte| byte.is_ascii_control())
        })
}

#[cfg(windows)]
fn is_supported_windows_project_root(path: &Path) -> bool {
    use std::path::{Component, Prefix};

    !path
        .as_os_str()
        .encode_wide()
        .any(|unit| unit <= 0x1f || unit == 0x7f)
        && matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::UNC(_, _))
        )
}

fn configured_project_root_path(
    path: &Path,
) -> Result<ConfiguredProjectRootPath, ConfiguredProjectRootsError> {
    #[cfg(unix)]
    {
        let bytes = path.as_os_str().as_bytes();
        if bytes.len() > MAX_CONFIGURED_PROJECT_ROOT_PATH_BYTES
            || !is_normalized_absolute_unix_project_root(bytes)
        {
            return Err(ConfiguredProjectRootsError::InternalState);
        }
        return Ok(ConfiguredProjectRootPath {
            encoding: SnapshotNameEncoding::UnixBytes,
            encoded_bytes: bytes.to_vec(),
        });
    }
    #[cfg(windows)]
    {
        let normalized: PathBuf = path.components().collect();
        if !is_supported_windows_project_root(path)
            || !path.is_absolute()
            || path.parent().is_none()
            || normalized.as_os_str() != path.as_os_str()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return Err(ConfiguredProjectRootsError::InternalState);
        }
        let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if units.is_empty()
            || units.len().saturating_mul(2) > MAX_CONFIGURED_PROJECT_ROOT_PATH_BYTES
            || units.contains(&0)
        {
            return Err(ConfiguredProjectRootsError::InternalState);
        }
        let mut bytes = Vec::with_capacity(units.len().saturating_mul(2));
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        return Ok(ConfiguredProjectRootPath {
            encoding: SnapshotNameEncoding::WindowsUtf16LittleEndian,
            encoded_bytes: bytes,
        });
    }
    #[allow(unreachable_code)]
    Err(ConfiguredProjectRootsError::InternalState)
}

fn parse_targeted_project_scan_volume_id(
    value: &str,
) -> Result<VolumeId, TargetedProjectScanError> {
    const PREFIX: &str = "volume:macos:";
    let raw = value.strip_prefix(PREFIX).unwrap_or(value);
    let parsed = parse_macos_volume_id(Some(raw.to_owned()))
        .map_err(|_| TargetedProjectScanError::InvalidVolumeIdentity)?
        .ok_or(TargetedProjectScanError::InvalidVolumeIdentity)?;
    if value.starts_with(PREFIX) && value != parsed.to_string() {
        return Err(TargetedProjectScanError::InvalidVolumeIdentity);
    }
    Ok(parsed)
}

fn targeted_project_scan_time(
    value: i64,
    internal: bool,
) -> Result<SystemTime, TargetedProjectScanError> {
    let millis = u64::try_from(value).map_err(|_| {
        if internal {
            TargetedProjectScanError::InternalState
        } else {
            TargetedProjectScanError::InvalidAnchor
        }
    })?;
    UNIX_EPOCH
        .checked_add(Duration::from_millis(millis))
        .ok_or({
            if internal {
                TargetedProjectScanError::InternalState
            } else {
                TargetedProjectScanError::InvalidAnchor
            }
        })
}

fn targeted_project_scan_time_ms(value: SystemTime) -> Result<i64, TargetedProjectScanError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| TargetedProjectScanError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| TargetedProjectScanError::InternalState)
}

fn core_targeted_project_scan_pressure_context(
    context: TargetedProjectScanPressureContext,
) -> Result<CoreTargetedProjectScanPressureContext, TargetedProjectScanError> {
    if context.record_version != FFI_RECORD_VERSION {
        return Err(TargetedProjectScanError::InvalidRecordVersion);
    }
    let capacity_anchor = targeted_project_scan_time(context.capacity_anchor_unix_ms, false)?;
    let current_episode_started_at =
        targeted_project_scan_time(context.current_episode_started_at_unix_ms, false)?;
    let pressure_started_at =
        targeted_project_scan_time(context.pressure_started_at_unix_ms, false)?;
    if pressure_started_at > current_episode_started_at
        || current_episode_started_at > capacity_anchor
    {
        return Err(TargetedProjectScanError::InvalidAnchor);
    }
    Ok(CoreTargetedProjectScanPressureContext {
        volume_id: parse_targeted_project_scan_volume_id(&context.stable_volume_id)?,
        capacity_anchor,
        pressure: match context.pressure {
            TargetedProjectScanPressure::Warning => CoreTargetedProjectScanPressure::Warning,
            TargetedProjectScanPressure::Critical => CoreTargetedProjectScanPressure::Critical,
        },
        current_episode_started_at,
        pressure_started_at,
        policy_revision: context.policy_revision,
    })
}

fn targeted_project_scan_pressure_context(
    context: &CoreTargetedProjectScanPressureContext,
) -> Result<TargetedProjectScanPressureContext, TargetedProjectScanError> {
    if context.pressure_started_at > context.current_episode_started_at
        || context.current_episode_started_at > context.capacity_anchor
    {
        return Err(TargetedProjectScanError::InternalState);
    }
    Ok(TargetedProjectScanPressureContext {
        record_version: FFI_RECORD_VERSION,
        stable_volume_id: context.volume_id.to_string(),
        capacity_anchor_unix_ms: targeted_project_scan_time_ms(context.capacity_anchor)?,
        pressure: match context.pressure {
            CoreTargetedProjectScanPressure::Warning => TargetedProjectScanPressure::Warning,
            CoreTargetedProjectScanPressure::Critical => TargetedProjectScanPressure::Critical,
        },
        current_episode_started_at_unix_ms: targeted_project_scan_time_ms(
            context.current_episode_started_at,
        )?,
        pressure_started_at_unix_ms: targeted_project_scan_time_ms(context.pressure_started_at)?,
        policy_revision: context.policy_revision,
    })
}

fn targeted_project_scan_selection(
    selection: &dux_core::engine::TargetedProjectScanSelection,
    catalog: &CoreTargetedReclaimRootCatalogStamp,
    expected_ordinal: u16,
) -> Result<TargetedProjectScanSelection, TargetedProjectScanError> {
    let known_count = u16::from(catalog.known_user_library_caches_included);
    let configured_count = catalog
        .root_count
        .checked_sub(known_count)
        .ok_or(TargetedProjectScanError::InternalState)?;
    let layout = dux_core::targeted_reclaim_root_catalog_layout(
        configured_count,
        catalog.known_user_library_caches_included,
    )
    .map_err(|_| TargetedProjectScanError::InternalState)?;
    let slot = layout
        .get(usize::from(expected_ordinal))
        .ok_or(TargetedProjectScanError::InternalState)?;
    if selection.ordinal != expected_ordinal
        || selection.ordinal >= catalog.root_count
        || selection.kind != slot.kind
        || selection.max_nodes != slot.max_nodes
    {
        return Err(TargetedProjectScanError::InternalState);
    }
    let root = configured_project_root_path(&selection.root)
        .map_err(|_| TargetedProjectScanError::InternalState)?;
    Ok(TargetedProjectScanSelection {
        record_version: FFI_RECORD_VERSION,
        ordinal: selection.ordinal,
        kind: match selection.kind {
            CoreTargetedReclaimRootKind::KnownUserLibraryCaches => {
                TargetedReclaimRootKind::KnownUserLibraryCaches
            }
            CoreTargetedReclaimRootKind::ConfiguredProject => {
                TargetedReclaimRootKind::ConfiguredProject
            }
        },
        root,
        max_nodes: selection.max_nodes,
    })
}

fn targeted_reclaim_root_catalog(
    catalog: &CoreTargetedReclaimRootCatalogStamp,
) -> Result<TargetedReclaimRootCatalog, TargetedProjectScanError> {
    if catalog.known_roots_policy_revision != dux_core::TARGETED_RECLAIM_ROOT_POLICY_REVISION
        || catalog.root_count > dux_core::MAX_TARGETED_RECLAIM_ROOTS
        || catalog.root_count < u16::from(catalog.known_user_library_caches_included)
    {
        return Err(TargetedProjectScanError::InternalState);
    }
    Ok(TargetedReclaimRootCatalog {
        record_version: FFI_RECORD_VERSION,
        known_roots_policy_revision: catalog.known_roots_policy_revision,
        configured_roots_revision: catalog.configured_roots_revision,
        known_user_library_caches_included: catalog.known_user_library_caches_included,
        root_count: catalog.root_count,
        digest_sha256: catalog.digest_sha256.to_vec(),
    })
}

fn core_targeted_reclaim_root_catalog(
    catalog: TargetedReclaimRootCatalog,
) -> Result<CoreTargetedReclaimRootCatalogStamp, TargetedProjectScanError> {
    let digest_sha256 = <[u8; 32]>::try_from(catalog.digest_sha256)
        .map_err(|_| TargetedProjectScanError::InvalidCatalog)?;
    if catalog.record_version != FFI_RECORD_VERSION
        || catalog.known_roots_policy_revision != dux_core::TARGETED_RECLAIM_ROOT_POLICY_REVISION
        || catalog.root_count > dux_core::MAX_TARGETED_RECLAIM_ROOTS
        || catalog.root_count < u16::from(catalog.known_user_library_caches_included)
    {
        return Err(TargetedProjectScanError::InvalidCatalog);
    }
    Ok(CoreTargetedReclaimRootCatalogStamp {
        known_roots_policy_revision: catalog.known_roots_policy_revision,
        configured_roots_revision: catalog.configured_roots_revision,
        known_user_library_caches_included: catalog.known_user_library_caches_included,
        root_count: catalog.root_count,
        digest_sha256,
    })
}

fn targeted_project_scan_admission(
    engine: &EngineHandle,
    admission: CoreTargetedProjectScanAdmission,
    expected_volume_id: &VolumeId,
    expected_capacity_anchor: SystemTime,
    expected_ordinal: u16,
    expected_revision: Option<u64>,
    expected_catalog_digest: Option<[u8; 32]>,
) -> Result<TargetedProjectScanAdmission, TargetedProjectScanError> {
    if admission.root_count > dux_core::MAX_TARGETED_RECLAIM_ROOTS
        || expected_revision.is_some_and(|value| value != admission.configured_roots_revision)
        || admission.configured_roots_revision != admission.root_catalog.configured_roots_revision
        || admission.root_count != admission.root_catalog.root_count
        || expected_catalog_digest
            .is_some_and(|expected| expected != admission.root_catalog.digest_sha256)
    {
        return Err(TargetedProjectScanError::InternalState);
    }
    let selection = admission
        .selection
        .as_ref()
        .map(|selection| {
            targeted_project_scan_selection(selection, &admission.root_catalog, expected_ordinal)
        })
        .transpose()?;
    let pressure = admission
        .pressure
        .as_ref()
        .map(targeted_project_scan_pressure_context)
        .transpose()?;
    if admission.pressure.as_ref().is_some_and(|context| {
        context.volume_id != *expected_volume_id
            || context.capacity_anchor != expected_capacity_anchor
    }) {
        return Err(TargetedProjectScanError::InternalState);
    }

    let mut response = TargetedProjectScanAdmission {
        record_version: FFI_RECORD_VERSION,
        configured_roots_revision: admission.configured_roots_revision,
        root_count: admission.root_count,
        root_catalog: targeted_reclaim_root_catalog(&admission.root_catalog)?,
        selection,
        pressure,
        disposition: TargetedProjectScanDisposition::EmptyRegistry,
        root_unavailable_reason: None,
        current_result: None,
        task: None,
        existing_task_observed_phase: None,
    };
    match admission.disposition {
        CoreTargetedProjectScanDisposition::EmptyRegistry => {
            if response.root_count != 0 || response.selection.is_some() {
                return Err(TargetedProjectScanError::InternalState);
            }
        }
        CoreTargetedProjectScanDisposition::NoPressure => {
            if response.root_count == 0
                || response.selection.is_none()
                || response.pressure.is_some()
            {
                return Err(TargetedProjectScanError::InternalState);
            }
            response.disposition = TargetedProjectScanDisposition::NoPressure;
        }
        CoreTargetedProjectScanDisposition::RootUnavailable { reason } => {
            require_targeted_project_scan_selection_and_pressure(&response)?;
            response.disposition = TargetedProjectScanDisposition::RootUnavailable;
            response.root_unavailable_reason = Some(map_targeted_project_scan_root_reason(reason)?);
        }
        CoreTargetedProjectScanDisposition::ExistingTask { task_id, phase } => {
            require_targeted_project_scan_selection_and_pressure(&response)?;
            response.disposition = TargetedProjectScanDisposition::ExistingTask;
            response.task = Some(targeted_project_scan_task(engine, task_id)?);
            response.existing_task_observed_phase = Some(map_phase(phase));
        }
        CoreTargetedProjectScanDisposition::Current(current) => {
            require_targeted_project_scan_selection_and_pressure(&response)?;
            response.disposition = TargetedProjectScanDisposition::Current;
            let kind = response
                .selection
                .as_ref()
                .ok_or(TargetedProjectScanError::InternalState)?
                .kind;
            response.current_result = Some(targeted_project_scan_current_result(&current, kind)?);
        }
        CoreTargetedProjectScanDisposition::Started { task_id } => {
            require_targeted_project_scan_selection_and_pressure(&response)?;
            response.disposition = TargetedProjectScanDisposition::Started;
            response.task = Some(targeted_project_scan_task(engine, task_id)?);
        }
        _ => return Err(TargetedProjectScanError::InternalState),
    }
    Ok(response)
}

fn require_targeted_project_scan_selection_and_pressure(
    response: &TargetedProjectScanAdmission,
) -> Result<(), TargetedProjectScanError> {
    if response.root_count == 0 || response.selection.is_none() || response.pressure.is_none() {
        Err(TargetedProjectScanError::InternalState)
    } else {
        Ok(())
    }
}

fn map_targeted_project_scan_root_reason(
    reason: ScanRootErrorKind,
) -> Result<TargetedProjectScanRootUnavailableReason, TargetedProjectScanError> {
    Ok(match reason {
        ScanRootErrorKind::InvalidPath => TargetedProjectScanRootUnavailableReason::InvalidPath,
        ScanRootErrorKind::Missing => TargetedProjectScanRootUnavailableReason::Missing,
        ScanRootErrorKind::AccessDenied => TargetedProjectScanRootUnavailableReason::AccessDenied,
        ScanRootErrorKind::NotDirectory => TargetedProjectScanRootUnavailableReason::NotDirectory,
        ScanRootErrorKind::Symlink => TargetedProjectScanRootUnavailableReason::Symlink,
        ScanRootErrorKind::ChangedDuringValidation => {
            TargetedProjectScanRootUnavailableReason::ChangedDuringValidation
        }
        ScanRootErrorKind::IdentityUnavailable => {
            TargetedProjectScanRootUnavailableReason::IdentityUnavailable
        }
        ScanRootErrorKind::VolumeMismatch => {
            TargetedProjectScanRootUnavailableReason::VolumeMismatch
        }
        ScanRootErrorKind::VolumeUnproven => {
            TargetedProjectScanRootUnavailableReason::VolumeUnproven
        }
        ScanRootErrorKind::UnsupportedPlatform => {
            TargetedProjectScanRootUnavailableReason::UnsupportedPlatform
        }
        ScanRootErrorKind::Unavailable => TargetedProjectScanRootUnavailableReason::Unavailable,
        _ => return Err(TargetedProjectScanError::InternalState),
    })
}

fn targeted_project_scan_task(
    engine: &EngineHandle,
    task_id: TaskId,
) -> Result<Arc<ScanTask>, TargetedProjectScanError> {
    let snapshot = engine
        .task_snapshot(task_id)
        .map_err(map_targeted_project_scan_task_access_error)?;
    if snapshot.kind != CoreTaskKind::Scan
        || snapshot.priority != CoreTaskPriority::Targeted
        || snapshot.scan_origin != Some(CoreScanTaskOrigin::TargetedRecommendation)
    {
        return Err(TargetedProjectScanError::InternalState);
    }
    Ok(Arc::new(ScanTask {
        engine: engine.clone(),
        id: task_id,
        progress: Mutex::new(ScanProgressState::default()),
    }))
}

fn map_targeted_project_scan_task_access_error(error: TaskAccessError) -> TargetedProjectScanError {
    match error {
        TaskAccessError::Closed => TargetedProjectScanError::Closed,
        TaskAccessError::UnknownTask => TargetedProjectScanError::Unavailable,
        TaskAccessError::InvalidEventLimit { .. }
        | TaskAccessError::InvalidEventCursor
        | TaskAccessError::WrongTaskKind
        | TaskAccessError::InternalState => TargetedProjectScanError::InternalState,
    }
}

fn targeted_project_scan_current_result(
    current: &CoreTargetedProjectScanCurrent,
    expected_kind: TargetedReclaimRootKind,
) -> Result<ScanTaskResult, TargetedProjectScanError> {
    let scan = &current.scan;
    let evaluation = &current.candidate_evaluation;
    let completed_at = scan
        .completed_at
        .ok_or(TargetedProjectScanError::InternalState)?;
    let counts = scan.counts.ok_or(TargetedProjectScanError::InternalState)?;
    let is_known_cache = scan
        .scan_id
        .as_str()
        .starts_with("scan:targeted:known-user-cache:");
    let kind_matches = match expected_kind {
        TargetedReclaimRootKind::KnownUserLibraryCaches => is_known_cache,
        TargetedReclaimRootKind::ConfiguredProject => {
            scan.scan_id.as_str().starts_with("scan:targeted:") && !is_known_cache
        }
    };
    if scan.status != CoreDurableScanStatus::Succeeded
        || !scan.snapshot_recorded
        || !kind_matches
        || completed_at < scan.started_at
        || evaluation.scan_id() != &scan.scan_id
        || evaluation.source_scan_status() != CoreDurableScanStatus::Succeeded
    {
        return Err(TargetedProjectScanError::InternalState);
    }
    let scheduled_at = evaluation
        .scheduled_at()
        .ok_or(TargetedProjectScanError::InternalState)?;
    let evaluation_completed_at = evaluation
        .completed_at()
        .ok_or(TargetedProjectScanError::InternalState)?;
    if scheduled_at < scan.started_at || evaluation_completed_at < scheduled_at {
        return Err(TargetedProjectScanError::InternalState);
    }
    let candidate_evaluation = match evaluation.status() {
        CoreDurableCandidateEvaluationStatus::Succeeded { candidate_count } => {
            if usize::try_from(candidate_count).ok() != Some(evaluation.candidates().len()) {
                return Err(TargetedProjectScanError::InternalState);
            }
            ScanCandidateEvaluationSummary {
                record_version: FFI_RECORD_VERSION,
                status: ScanCandidateEvaluationStatus::Succeeded,
                candidate_count,
                failure: None,
            }
        }
        CoreDurableCandidateEvaluationStatus::Failed { kind } => {
            if !evaluation.candidates().is_empty() {
                return Err(TargetedProjectScanError::InternalState);
            }
            ScanCandidateEvaluationSummary {
                record_version: FFI_RECORD_VERSION,
                status: ScanCandidateEvaluationStatus::Failed,
                candidate_count: 0,
                failure: Some(map_scan_candidate_evaluation_failure(kind)),
            }
        }
        CoreDurableCandidateEvaluationStatus::NotRun
        | CoreDurableCandidateEvaluationStatus::Pending => {
            return Err(TargetedProjectScanError::InternalState);
        }
        _ => return Err(TargetedProjectScanError::InternalState),
    };
    let issue_record_count = u64::try_from(scan.coverage.issue_record_count)
        .map_err(|_| TargetedProjectScanError::InternalState)?;
    Ok(ScanTaskResult {
        record_version: FFI_RECORD_VERSION,
        scan_id: scan.scan_id.as_str().to_owned(),
        started_at_unix_ms: targeted_project_scan_time_ms(scan.started_at)?,
        completed_at_unix_ms: targeted_project_scan_time_ms(completed_at)?,
        status: ScanTerminalStatus::Succeeded,
        directory_count: counts.directory_count,
        file_count: counts.file_count,
        logical_bytes: counts.logical_bytes,
        allocated_bytes: counts.allocated_bytes,
        snapshot_available: true,
        coverage: ScanCoverageSummary {
            record_version: FFI_RECORD_VERSION,
            status: map_scan_coverage_status(scan.coverage.status),
            measured_permille: scan.coverage.measured_permille.map(|value| value.get()),
            issue_record_count,
            issue_occurrence_count: scan.coverage.issue_occurrence_count,
        },
        candidate_evaluation,
    })
}

fn targeted_project_scan_checkpoint(
    checkpoint: CoreTargetedProjectScanCheckpoint,
    expected_pressure: &CoreTargetedProjectScanPressureContext,
    expected_catalog: &CoreTargetedReclaimRootCatalogStamp,
) -> Result<TargetedProjectScanCheckpoint, TargetedProjectScanError> {
    if checkpoint.configured_roots_revision != expected_catalog.configured_roots_revision
        || checkpoint.root_count != expected_catalog.root_count
        || checkpoint.root_catalog != *expected_catalog
        || checkpoint.root_count > dux_core::MAX_TARGETED_RECLAIM_ROOTS
        || checkpoint.pressure != *expected_pressure
    {
        return Err(TargetedProjectScanError::InternalState);
    }
    Ok(TargetedProjectScanCheckpoint {
        record_version: FFI_RECORD_VERSION,
        configured_roots_revision: checkpoint.configured_roots_revision,
        root_count: checkpoint.root_count,
        root_catalog: targeted_reclaim_root_catalog(&checkpoint.root_catalog)?,
        pressure: targeted_project_scan_pressure_context(&checkpoint.pressure)?,
    })
}

fn emergency_recovery_ordering(
    ordering: CoreEmergencyRecoveryOrdering,
    expected_pressure: &CoreTargetedProjectScanPressureContext,
    expected_catalog: &CoreTargetedReclaimRootCatalogStamp,
) -> Result<EmergencyRecoveryOrdering, EmergencyRecoveryError> {
    if ordering.policy_revision != EMERGENCY_RECOVERY_POLICY_REVISION
        || &ordering.pressure != expected_pressure
        || &ordering.root_catalog != expected_catalog
        || ordering.groups.len() > dux_core::engine::MAX_EMERGENCY_RECOVERY_GROUPS
        || ordering.observed_root_count > ordering.root_catalog.root_count
        || ordering.candidate_evaluated_root_count > ordering.observed_root_count
        || ordering
            .observed_root_count
            .checked_add(ordering.unavailable_root_count)
            != Some(ordering.root_catalog.root_count)
    {
        return Err(EmergencyRecoveryError::InternalState);
    }
    let mut previous_lane_priority = None;
    let permission_group_count = ordering
        .groups
        .iter()
        .filter(|group| group.lane == CoreEmergencyRecoveryLane::PermissionGap)
        .count();
    if (ordering.unavailable_root_count > 0 && permission_group_count != 1)
        || permission_group_count > 1
    {
        return Err(EmergencyRecoveryError::InternalState);
    }
    let groups = ordering
        .groups
        .iter()
        .enumerate()
        .map(|(index, group)| {
            let expected_rank =
                u16::try_from(index).map_err(|_| EmergencyRecoveryError::InternalState)?;
            if group.rank != expected_rank {
                return Err(EmergencyRecoveryError::InternalState);
            }
            let lane_priority = group.lane.priority();
            if previous_lane_priority.is_some_and(|previous| previous > lane_priority) {
                return Err(EmergencyRecoveryError::InternalState);
            }
            previous_lane_priority = Some(lane_priority);
            if ordering.groups[..index].iter().any(|previous| {
                previous.lane == group.lane
                    && previous.rule == group.rule
                    && previous.category == group.category
            }) {
                return Err(EmergencyRecoveryError::InternalState);
            }
            emergency_recovery_group(
                group,
                ordering.root_catalog.root_count,
                ordering.unavailable_root_count,
            )
        })
        .collect::<Result<Vec<_>, EmergencyRecoveryError>>()?;
    Ok(EmergencyRecoveryOrdering {
        record_version: FFI_RECORD_VERSION,
        policy_revision: ordering.policy_revision,
        pressure: targeted_project_scan_pressure_context(&ordering.pressure)
            .map_err(map_targeted_project_scan_request_to_emergency_recovery_error)?,
        root_catalog: targeted_reclaim_root_catalog(&ordering.root_catalog)
            .map_err(map_targeted_project_scan_request_to_emergency_recovery_error)?,
        observed_root_count: ordering.observed_root_count,
        candidate_evaluated_root_count: ordering.candidate_evaluated_root_count,
        unavailable_root_count: ordering.unavailable_root_count,
        groups,
    })
}

fn emergency_recovery_group(
    group: &CoreEmergencyRecoveryGroup,
    root_count: u16,
    ordering_unavailable_root_count: u16,
) -> Result<EmergencyRecoveryGroup, EmergencyRecoveryError> {
    if group.sources.len() > usize::from(dux_core::engine::MAX_TARGETED_RECLAIM_ROOTS)
        || group
            .sources
            .windows(2)
            .any(|pair| pair[0].root_ordinal >= pair[1].root_ordinal)
        || group
            .sources
            .iter()
            .any(|source| source.root_ordinal >= root_count)
    {
        return Err(EmergencyRecoveryError::InternalState);
    }
    let stable_rule_shape = group.rule.is_some() && group.category.is_some();
    match group.lane {
        CoreEmergencyRecoveryLane::StaleSafeRegenerable
            if stable_rule_shape
                && group.unavailable_root_count == 0
                && !group.sources.is_empty() => {}
        CoreEmergencyRecoveryLane::GuidedExploration | CoreEmergencyRecoveryLane::PermissionGap
            if group.rule.is_none()
                && group.category.is_none()
                && match group.lane {
                    CoreEmergencyRecoveryLane::GuidedExploration => {
                        group.unavailable_root_count == 0 && !group.sources.is_empty()
                    }
                    CoreEmergencyRecoveryLane::PermissionGap => {
                        group.unavailable_root_count == ordering_unavailable_root_count
                            && (!group.sources.is_empty() || group.unavailable_root_count > 0)
                    }
                    _ => false,
                } => {}
        CoreEmergencyRecoveryLane::EvictableCloud
        | CoreEmergencyRecoveryLane::TrashInformation
        | CoreEmergencyRecoveryLane::ReviewableInstallerArchive
        | CoreEmergencyRecoveryLane::LargeFile
        | CoreEmergencyRecoveryLane::StaleSafeRegenerable
        | CoreEmergencyRecoveryLane::GuidedExploration
        | CoreEmergencyRecoveryLane::PermissionGap => {
            return Err(EmergencyRecoveryError::InternalState);
        }
    }
    let sources = group
        .sources
        .iter()
        .map(|source| emergency_recovery_source(group.lane, source))
        .collect::<Result<Vec<_>, EmergencyRecoveryError>>()?;
    Ok(EmergencyRecoveryGroup {
        record_version: FFI_RECORD_VERSION,
        rank: group.rank,
        lane: map_emergency_recovery_lane(group.lane),
        rule_id: group
            .rule
            .as_ref()
            .map(|rule| rule.id().as_str().to_owned()),
        rule_revision: group.rule.as_ref().map(|rule| rule.revision().get()),
        category: group.category.map(map_candidate_category),
        unavailable_root_count: group.unavailable_root_count,
        sources,
    })
}

fn emergency_recovery_source(
    lane: CoreEmergencyRecoveryLane,
    source: &CoreEmergencyRecoverySource,
) -> Result<EmergencyRecoverySource, EmergencyRecoveryError> {
    let shape_is_valid = match lane {
        CoreEmergencyRecoveryLane::StaleSafeRegenerable => {
            source.candidate_count.is_some_and(|count| count > 0)
                && source.blocked_candidate_count.is_some_and(|blocked| {
                    source.candidate_count.is_some_and(|count| blocked <= count)
                })
                && source.permission_issue_count.is_none()
        }
        CoreEmergencyRecoveryLane::GuidedExploration => {
            source.candidate_count.is_none()
                && source.blocked_candidate_count.is_none()
                && source.permission_issue_count.is_none()
        }
        CoreEmergencyRecoveryLane::PermissionGap => {
            source.candidate_count.is_none()
                && source.blocked_candidate_count.is_none()
                && source.permission_issue_count.is_some_and(|count| count > 0)
        }
        CoreEmergencyRecoveryLane::EvictableCloud
        | CoreEmergencyRecoveryLane::TrashInformation
        | CoreEmergencyRecoveryLane::ReviewableInstallerArchive
        | CoreEmergencyRecoveryLane::LargeFile => false,
    };
    if !shape_is_valid {
        return Err(EmergencyRecoveryError::InternalState);
    }
    Ok(EmergencyRecoverySource {
        record_version: FFI_RECORD_VERSION,
        root_ordinal: source.root_ordinal,
        scan_id: source.scan_id.as_str().to_owned(),
        observed_at_unix_ms: targeted_project_scan_time_ms(source.observed_at)
            .map_err(map_targeted_project_scan_request_to_emergency_recovery_error)?,
        candidate_count: source.candidate_count,
        blocked_candidate_count: source.blocked_candidate_count,
        permission_issue_count: source.permission_issue_count,
    })
}

const fn map_emergency_recovery_lane(lane: CoreEmergencyRecoveryLane) -> EmergencyRecoveryLane {
    match lane {
        CoreEmergencyRecoveryLane::EvictableCloud => EmergencyRecoveryLane::EvictableCloud,
        CoreEmergencyRecoveryLane::StaleSafeRegenerable => {
            EmergencyRecoveryLane::StaleSafeRegenerable
        }
        CoreEmergencyRecoveryLane::TrashInformation => EmergencyRecoveryLane::TrashInformation,
        CoreEmergencyRecoveryLane::ReviewableInstallerArchive => {
            EmergencyRecoveryLane::ReviewableInstallerArchive
        }
        CoreEmergencyRecoveryLane::LargeFile => EmergencyRecoveryLane::LargeFile,
        CoreEmergencyRecoveryLane::GuidedExploration => EmergencyRecoveryLane::GuidedExploration,
        CoreEmergencyRecoveryLane::PermissionGap => EmergencyRecoveryLane::PermissionGap,
    }
}

const fn map_emergency_recovery_error(error: CoreEmergencyRecoveryError) -> EmergencyRecoveryError {
    match error {
        CoreEmergencyRecoveryError::Closed => EmergencyRecoveryError::Closed,
        CoreEmergencyRecoveryError::ReadOnlyStore => EmergencyRecoveryError::ReadOnlyStore,
        CoreEmergencyRecoveryError::NotCritical => EmergencyRecoveryError::NotCritical,
        CoreEmergencyRecoveryError::RegistryChanged => EmergencyRecoveryError::RegistryChanged,
        CoreEmergencyRecoveryError::InvalidCatalog => EmergencyRecoveryError::InvalidCatalog,
        CoreEmergencyRecoveryError::CatalogChanged => EmergencyRecoveryError::CatalogChanged,
        CoreEmergencyRecoveryError::PressureChanged => EmergencyRecoveryError::PressureChanged,
        CoreEmergencyRecoveryError::IncompatibleSchema => {
            EmergencyRecoveryError::IncompatibleSchema
        }
        CoreEmergencyRecoveryError::Busy => EmergencyRecoveryError::Busy,
        CoreEmergencyRecoveryError::UnsafeStorage => EmergencyRecoveryError::UnsafeStorage,
        CoreEmergencyRecoveryError::BudgetExceeded => EmergencyRecoveryError::BudgetExceeded,
        CoreEmergencyRecoveryError::CorruptData => EmergencyRecoveryError::CorruptData,
        CoreEmergencyRecoveryError::Unavailable => EmergencyRecoveryError::Unavailable,
        CoreEmergencyRecoveryError::OutcomeUnknown => EmergencyRecoveryError::OutcomeUnknown,
        CoreEmergencyRecoveryError::InternalState => EmergencyRecoveryError::InternalState,
        _ => EmergencyRecoveryError::InternalState,
    }
}

const fn map_targeted_project_scan_request_to_emergency_recovery_error(
    error: TargetedProjectScanError,
) -> EmergencyRecoveryError {
    match error {
        TargetedProjectScanError::InvalidRecordVersion => {
            EmergencyRecoveryError::InvalidRecordVersion
        }
        TargetedProjectScanError::InvalidVolumeIdentity
        | TargetedProjectScanError::InvalidAnchor
        | TargetedProjectScanError::PressureChanged => EmergencyRecoveryError::InvalidPressureProof,
        TargetedProjectScanError::InvalidCatalog => EmergencyRecoveryError::InvalidCatalog,
        TargetedProjectScanError::RegistryChanged => EmergencyRecoveryError::RegistryChanged,
        TargetedProjectScanError::CatalogChanged => EmergencyRecoveryError::CatalogChanged,
        TargetedProjectScanError::Closed => EmergencyRecoveryError::Closed,
        TargetedProjectScanError::ReadOnlyStore => EmergencyRecoveryError::ReadOnlyStore,
        TargetedProjectScanError::IncompatibleSchema => EmergencyRecoveryError::IncompatibleSchema,
        TargetedProjectScanError::Busy => EmergencyRecoveryError::Busy,
        TargetedProjectScanError::UnsafeStorage => EmergencyRecoveryError::UnsafeStorage,
        TargetedProjectScanError::BudgetExceeded => EmergencyRecoveryError::BudgetExceeded,
        TargetedProjectScanError::CorruptData => EmergencyRecoveryError::CorruptData,
        TargetedProjectScanError::Unavailable => EmergencyRecoveryError::Unavailable,
        TargetedProjectScanError::OutcomeUnknown => EmergencyRecoveryError::OutcomeUnknown,
        TargetedProjectScanError::InvalidOrdinal
        | TargetedProjectScanError::QueueFull
        | TargetedProjectScanError::TaskIdExhausted
        | TargetedProjectScanError::InternalState => EmergencyRecoveryError::InternalState,
    }
}

fn map_targeted_project_scan_error(
    error: CoreTargetedProjectScanError,
) -> TargetedProjectScanError {
    match error {
        CoreTargetedProjectScanError::Closed => TargetedProjectScanError::Closed,
        CoreTargetedProjectScanError::ReadOnlyStore => TargetedProjectScanError::ReadOnlyStore,
        CoreTargetedProjectScanError::RegistryChanged { .. } => {
            TargetedProjectScanError::RegistryChanged
        }
        CoreTargetedProjectScanError::InvalidCatalog => TargetedProjectScanError::InvalidCatalog,
        CoreTargetedProjectScanError::CatalogChanged => TargetedProjectScanError::CatalogChanged,
        CoreTargetedProjectScanError::InvalidOrdinal { .. } => {
            TargetedProjectScanError::InvalidOrdinal
        }
        CoreTargetedProjectScanError::InvalidAnchor => TargetedProjectScanError::InvalidAnchor,
        CoreTargetedProjectScanError::PressureChanged => TargetedProjectScanError::PressureChanged,
        CoreTargetedProjectScanError::IncompatibleSchema => {
            TargetedProjectScanError::IncompatibleSchema
        }
        CoreTargetedProjectScanError::Busy => TargetedProjectScanError::Busy,
        CoreTargetedProjectScanError::UnsafeStorage => TargetedProjectScanError::UnsafeStorage,
        CoreTargetedProjectScanError::BudgetExceeded => TargetedProjectScanError::BudgetExceeded,
        CoreTargetedProjectScanError::CorruptData => TargetedProjectScanError::CorruptData,
        CoreTargetedProjectScanError::Unavailable => TargetedProjectScanError::Unavailable,
        CoreTargetedProjectScanError::OutcomeUnknown => TargetedProjectScanError::OutcomeUnknown,
        CoreTargetedProjectScanError::QueueFull => TargetedProjectScanError::QueueFull,
        CoreTargetedProjectScanError::TaskIdExhausted => TargetedProjectScanError::TaskIdExhausted,
        CoreTargetedProjectScanError::InternalState => TargetedProjectScanError::InternalState,
        _ => TargetedProjectScanError::InternalState,
    }
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
        || (policy.source == CorePermanentCleanupPolicySource::Default && policy.enabled)
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
    use std::time::{Duration, Instant, SystemTime};
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

    fn scan_snapshot(engine: &DuxEngine, root: &Path) -> String {
        let scan = engine.start_scan(scan_request(root)).unwrap();
        let terminal = wait_for_scan(&scan.task);
        assert_eq!(terminal.phase, TaskPhase::Succeeded);
        terminal.result.unwrap().scan_id
    }

    fn seed_terminal_cleanup_history(temp: &TempDir, engine: &DuxEngine, fixture: &str) -> String {
        let root = temp.path().join(fixture);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("reviewed.bin"), b"reviewed").unwrap();
        let scan = engine.start_scan(scan_request(&root)).unwrap();
        let terminal = wait_for_scan(&scan.task);
        assert_eq!(
            terminal.phase,
            TaskPhase::Succeeded,
            "unexpected cleanup terminal state: {terminal:?}"
        );
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
        assert_eq!(
            engine
                .execute_explorer_trash(
                    Arc::clone(&review),
                    selected.id,
                    Box::new(RecordingTrashDriver {
                        calls: Mutex::new(Vec::new()),
                    }),
                )
                .unwrap(),
            TrashPlatformResult::Completed
        );
        let _ = review.release();
        engine
            .recent_cleanup_history(None, 1)
            .unwrap()
            .records
            .first()
            .unwrap()
            .session_id
            .clone()
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
    fn reports_contract_fifty_with_exact_storage_compatibility_and_preserves_formatting() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let expected = LibraryVersion {
            library_version: env!("CARGO_PKG_VERSION").to_owned(),
            ffi_contract_version: 50,
            database_schema_version: DATABASE_SCHEMA_VERSION,
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
        };
        assert_eq!(library_version(), expected);
        assert_eq!(engine.library_version().unwrap(), expected);
        assert_eq!(engine.format_size(1536).unwrap().display, "1.5 KB");
        assert!(engine.close());
        assert_eq!(engine.format_size(1), Err(EngineError::Closed));
    }

    fn core_rust_target_plan_review_info() -> CoreRustTargetPlanReviewInfo {
        CoreRustTargetPlanReviewInfo {
            plan_id: "plan:example".to_owned(),
            source_scan_id: ScanId::new("scan:example").unwrap(),
            candidate_id: CandidateId::new("candidate:example").unwrap(),
            rule_id: "developer.rust.target".to_owned(),
            rule_revision: 3,
            category: CoreCandidateCategory::DeveloperArtifact,
            mode: CorePlanCleanupMode::PermanentSafe,
            safety: CoreSafetyTier::SafeRegenerable,
            action: CoreCandidateAction::RemoveKnownRegenerableContents,
            estimated_bytes: 42,
            newest_mtime: UNIX_EPOCH + Duration::new(1_699_395_200, 123_456_789),
            minimum_age: RUST_TARGET_MINIMUM_AGE,
            warnings: vec![
                CorePlanWarning::EstimatedBytesUnverified,
                CorePlanWarning::PermanentRemovalCannotBeUndone,
            ],
            created_at: UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789),
            effective_expires_at: UNIX_EPOCH + Duration::new(1_700_000_600, 123_456_789),
            schedule_eligible: false,
            item_count: 1,
            path_count: 1,
            path: PathBuf::from("/Users/example/project/target"),
        }
    }

    #[test]
    fn rust_target_plan_review_projects_exact_revision_three_recency() {
        let projected =
            project_rust_target_plan_review_info(core_rust_target_plan_review_info()).unwrap();
        assert_eq!(projected.rule_revision, 3);
        assert_eq!(
            projected.newest_mtime,
            SnapshotNodeTimestamp {
                seconds_since_unix_epoch: 1_699_395_200,
                nanoseconds: 123_456_789,
            }
        );
        assert_eq!(projected.minimum_age_seconds, 604_800);
        assert_eq!(projected.minimum_age_nanoseconds, 0);

        let mut wrong_revision = core_rust_target_plan_review_info();
        wrong_revision.rule_revision = 2;
        assert_eq!(
            project_rust_target_plan_review_info(wrong_revision),
            Err(RustTargetPlanReviewError::InternalState)
        );

        let mut wrong_age = core_rust_target_plan_review_info();
        wrong_age.minimum_age = Duration::from_secs(604_799);
        assert_eq!(
            project_rust_target_plan_review_info(wrong_age),
            Err(RustTargetPlanReviewError::InternalState)
        );

        let mut recent = core_rust_target_plan_review_info();
        recent.newest_mtime = recent.created_at - Duration::from_secs(604_799);
        assert_eq!(
            project_rust_target_plan_review_info(recent),
            Err(RustTargetPlanReviewError::InternalState)
        );
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

    fn cleanup_result_for_test(
        status: CleanupSessionStatus,
        removed_entries: u64,
        removed_logical_bytes: u64,
        verified_capacity_delta_bytes: Option<i64>,
    ) -> RustTargetCleanupResult {
        RustTargetCleanupResult {
            record_version: FFI_RECORD_VERSION,
            session_id: "cleanup:rust-target:0123456789abcdef0123456789abcdef".to_owned(),
            status,
            removed_entries,
            removed_logical_bytes,
            verified_capacity_delta_bytes,
        }
    }

    #[test]
    fn rust_target_cleanup_poll_shapes_and_correlation_are_fail_closed() {
        let completed = cleanup_result_for_test(CleanupSessionStatus::Completed, 2, 42, Some(9));
        let cancelled = cleanup_result_for_test(CleanupSessionStatus::Cancelled, 1, 7, None);
        let recovering = cleanup_result_for_test(CleanupSessionStatus::Recovering, 0, 0, None);
        for (phase, failure, result) in [
            (CoreTaskPhase::Queued, None, None),
            (CoreTaskPhase::Running, None, None),
            (CoreTaskPhase::Succeeded, None, Some(&completed)),
            (CoreTaskPhase::Cancelled, None, None),
            (CoreTaskPhase::Cancelled, None, Some(&cancelled)),
            (
                CoreTaskPhase::Failed,
                Some(RustTargetCleanupTaskFailure::OutcomeUnknown),
                None,
            ),
            (
                CoreTaskPhase::Failed,
                Some(RustTargetCleanupTaskFailure::OutcomeUnknown),
                Some(&recovering),
            ),
            (
                CoreTaskPhase::Failed,
                Some(RustTargetCleanupTaskFailure::Busy),
                None,
            ),
        ] {
            assert!(
                validate_rust_target_cleanup_poll_shape(phase, failure, result).is_ok(),
                "expected valid shape for {phase:?}"
            );
        }

        for (phase, failure, result) in [
            (CoreTaskPhase::Succeeded, None, None),
            (CoreTaskPhase::Succeeded, None, Some(&cancelled)),
            (CoreTaskPhase::Failed, None, None),
            (
                CoreTaskPhase::Failed,
                Some(RustTargetCleanupTaskFailure::Busy),
                Some(&completed),
            ),
            (CoreTaskPhase::Cancelled, None, Some(&completed)),
            (
                CoreTaskPhase::Running,
                Some(RustTargetCleanupTaskFailure::Busy),
                None,
            ),
        ] {
            assert_eq!(
                validate_rust_target_cleanup_poll_shape(phase, failure, result),
                Err(RustTargetCleanupTaskError::InternalState)
            );
        }

        let malformed_recovering =
            cleanup_result_for_test(CleanupSessionStatus::Recovering, 1, 0, None);
        assert_eq!(
            validate_rust_target_cleanup_result(&malformed_recovering),
            Err(RustTargetCleanupTaskError::InternalState)
        );
        for invalid in [
            "cleanup:rust-target:0123456789ABCDEF0123456789abcdef",
            "cleanup:rust-target:0123456789abcdef",
            "cleanup:rust-target:0123456789abcdef0123456789abcdeg",
            "cleanup:other:0123456789abcdef0123456789abcdef",
            "cleanup:rust-target:\u{0}123456789abcdef0123456789abcdef",
        ] {
            assert!(!is_rust_target_cleanup_session_id(invalid));
        }
    }

    #[test]
    fn rust_target_cleanup_failure_and_cancel_taxonomies_are_exact() {
        use CorePermanentSafeCleanupFailureKind as CoreFailure;
        for (core, expected) in [
            (
                CoreFailure::ParentReviewUnavailable,
                RustTargetCleanupTaskFailure::ParentReviewUnavailable,
            ),
            (
                CoreFailure::ReviewExpired,
                RustTargetCleanupTaskFailure::ReviewExpired,
            ),
            (
                CoreFailure::ChangedDuringReview,
                RustTargetCleanupTaskFailure::ChangedDuringReview,
            ),
            (
                CoreFailure::BudgetExceeded,
                RustTargetCleanupTaskFailure::BudgetExceeded,
            ),
            (CoreFailure::Busy, RustTargetCleanupTaskFailure::Busy),
            (
                CoreFailure::UnsafeStorage,
                RustTargetCleanupTaskFailure::UnsafeStorage,
            ),
            (
                CoreFailure::IncompatibleSchema,
                RustTargetCleanupTaskFailure::IncompatibleSchema,
            ),
            (
                CoreFailure::CorruptData,
                RustTargetCleanupTaskFailure::CorruptData,
            ),
            (
                CoreFailure::OutcomeUnknown,
                RustTargetCleanupTaskFailure::OutcomeUnknown,
            ),
            (
                CoreFailure::Unavailable,
                RustTargetCleanupTaskFailure::Unavailable,
            ),
            (
                CoreFailure::InternalState,
                RustTargetCleanupTaskFailure::InternalState,
            ),
        ] {
            assert_eq!(
                map_rust_target_cleanup_task_failure(TaskFailureKind::PermanentSafeCleanup(core))
                    .unwrap(),
                expected
            );
        }
        assert_eq!(
            map_rust_target_cleanup_task_failure(TaskFailureKind::InternalFailure),
            Err(RustTargetCleanupTaskError::InternalState)
        );
        for (core, expected) in [
            (
                CoreCancelOutcome::CancelledBeforeStart,
                RustTargetCleanupCancelOutcome::CancelledBeforeStart,
            ),
            (
                CoreCancelOutcome::Requested,
                RustTargetCleanupCancelOutcome::Requested,
            ),
            (
                CoreCancelOutcome::AlreadyRequested,
                RustTargetCleanupCancelOutcome::AlreadyRequested,
            ),
            (
                CoreCancelOutcome::AlreadyTerminal,
                RustTargetCleanupCancelOutcome::AlreadyTerminal,
            ),
        ] {
            assert_eq!(map_rust_target_cleanup_cancel_outcome(core), expected);
        }
        for (core, expected) in [
            (
                CoreRustTargetCleanupError::Closed,
                RustTargetCleanupStartError::Closed,
            ),
            (
                CoreRustTargetCleanupError::WrongEngine,
                RustTargetCleanupStartError::WrongEngine,
            ),
            (
                CoreRustTargetCleanupError::ParentReviewUnavailable,
                RustTargetCleanupStartError::ParentReviewUnavailable,
            ),
            (
                CoreRustTargetCleanupError::ReviewExpired,
                RustTargetCleanupStartError::ReviewExpired,
            ),
            (
                CoreRustTargetCleanupError::ChangedDuringReview,
                RustTargetCleanupStartError::ChangedDuringReview,
            ),
            (
                CoreRustTargetCleanupError::CancelledBeforeStart,
                RustTargetCleanupStartError::CancelledBeforeStart,
            ),
            (
                CoreRustTargetCleanupError::BudgetExceeded,
                RustTargetCleanupStartError::BudgetExceeded,
            ),
            (
                CoreRustTargetCleanupError::QueueFull,
                RustTargetCleanupStartError::QueueFull,
            ),
            (
                CoreRustTargetCleanupError::Busy,
                RustTargetCleanupStartError::Busy,
            ),
            (
                CoreRustTargetCleanupError::UnsafeStorage,
                RustTargetCleanupStartError::UnsafeStorage,
            ),
            (
                CoreRustTargetCleanupError::IncompatibleSchema,
                RustTargetCleanupStartError::IncompatibleSchema,
            ),
            (
                CoreRustTargetCleanupError::CorruptData,
                RustTargetCleanupStartError::CorruptData,
            ),
            (
                CoreRustTargetCleanupError::OutcomeUnknown,
                RustTargetCleanupStartError::OutcomeUnknown,
            ),
            (
                CoreRustTargetCleanupError::Unavailable,
                RustTargetCleanupStartError::Unavailable,
            ),
            (
                CoreRustTargetCleanupError::InternalState,
                RustTargetCleanupStartError::InternalState,
            ),
        ] {
            assert_eq!(map_rust_target_cleanup_start_error(core), expected);
        }
    }

    #[test]
    fn rust_target_cleanup_start_losing_an_info_race_is_irreversible() {
        let tracker = Arc::new(RustTargetPlanPreparationTracker::default());
        let session = RustTargetPlanReviewSession {
            state: Mutex::new(RustTargetPlanReviewState::Inspecting),
            parent_review: Weak::new(),
            operations: tracker,
            engine_closed: Arc::new(AtomicBool::new(false)),
        };
        assert!(matches!(
            session.take_for_cleanup_start(),
            Err(RustTargetCleanupStartError::ReviewUnavailable)
        ));
        assert!(matches!(
            *session.state.lock().unwrap(),
            RustTargetPlanReviewState::ReleasePending
        ));
        assert_eq!(
            session.release().unwrap(),
            RustTargetPlanReviewReleaseOutcome::AlreadyUnavailable
        );
    }

    fn dry_run_result_for_test(status: CleanupSessionStatus) -> RustTargetDryRunResult {
        RustTargetDryRunResult {
            record_version: FFI_RECORD_VERSION,
            session_id: "cleanup:rust-target-dry-run:0123456789abcdef0123456789abcdef".to_owned(),
            status,
        }
    }

    #[test]
    fn rust_target_dry_run_poll_shapes_and_correlation_are_fail_closed() {
        let dry_run = dry_run_result_for_test(CleanupSessionStatus::DryRun);
        let rejected = dry_run_result_for_test(CleanupSessionStatus::Rejected);
        let failed = dry_run_result_for_test(CleanupSessionStatus::Failed);
        let interrupted = dry_run_result_for_test(CleanupSessionStatus::Interrupted);
        let cancelled = dry_run_result_for_test(CleanupSessionStatus::Cancelled);
        for (phase, failure, result) in [
            (CoreTaskPhase::Queued, None, None),
            (CoreTaskPhase::Running, None, None),
            (CoreTaskPhase::Succeeded, None, Some(&dry_run)),
            (CoreTaskPhase::Succeeded, None, Some(&rejected)),
            (CoreTaskPhase::Succeeded, None, Some(&failed)),
            (CoreTaskPhase::Succeeded, None, Some(&interrupted)),
            (CoreTaskPhase::Cancelled, None, None),
            (CoreTaskPhase::Cancelled, None, Some(&cancelled)),
            (
                CoreTaskPhase::Failed,
                Some(RustTargetDryRunTaskFailure::HistoryUnresolved),
                None,
            ),
            (
                CoreTaskPhase::Failed,
                Some(RustTargetDryRunTaskFailure::Busy),
                None,
            ),
        ] {
            assert!(
                validate_rust_target_dry_run_poll_shape(phase, failure, result).is_ok(),
                "expected valid shape for {phase:?}"
            );
        }

        for (phase, failure, result) in [
            (CoreTaskPhase::Succeeded, None, None),
            (CoreTaskPhase::Succeeded, None, Some(&cancelled)),
            (CoreTaskPhase::Failed, None, None),
            (
                CoreTaskPhase::Failed,
                Some(RustTargetDryRunTaskFailure::Busy),
                Some(&dry_run),
            ),
            (CoreTaskPhase::Cancelled, None, Some(&dry_run)),
            (
                CoreTaskPhase::Running,
                Some(RustTargetDryRunTaskFailure::Busy),
                None,
            ),
        ] {
            assert_eq!(
                validate_rust_target_dry_run_poll_shape(phase, failure, result),
                Err(RustTargetDryRunTaskError::InternalState)
            );
        }

        for status in [
            CleanupSessionStatus::Planned,
            CleanupSessionStatus::Running,
            CleanupSessionStatus::Recovering,
            CleanupSessionStatus::Completed,
            CleanupSessionStatus::PartiallyCompleted,
        ] {
            assert_eq!(
                validate_rust_target_dry_run_result(&dry_run_result_for_test(status)),
                Err(RustTargetDryRunTaskError::InternalState)
            );
        }
        for invalid in [
            "cleanup:rust-target-dry-run:0123456789ABCDEF0123456789abcdef",
            "cleanup:rust-target-dry-run:0123456789abcdef",
            "cleanup:rust-target-dry-run:0123456789abcdef0123456789abcdeg",
            "cleanup:rust-target:0123456789abcdef0123456789abcdef",
            "cleanup:rust-target-dry-run:\u{0}123456789abcdef0123456789abcdef",
        ] {
            assert!(!is_rust_target_dry_run_session_id(invalid));
        }
    }

    #[test]
    fn rust_target_dry_run_failure_start_and_cancel_taxonomies_are_exact() {
        use CoreRustTargetDryRunFailureKind as CoreFailure;
        for (core, expected) in [
            (
                CoreFailure::ParentReviewUnavailable,
                RustTargetDryRunTaskFailure::ParentReviewUnavailable,
            ),
            (
                CoreFailure::ReviewExpired,
                RustTargetDryRunTaskFailure::ReviewExpired,
            ),
            (
                CoreFailure::ChangedDuringReview,
                RustTargetDryRunTaskFailure::ChangedDuringReview,
            ),
            (
                CoreFailure::BudgetExceeded,
                RustTargetDryRunTaskFailure::BudgetExceeded,
            ),
            (CoreFailure::Busy, RustTargetDryRunTaskFailure::Busy),
            (
                CoreFailure::UnsafeStorage,
                RustTargetDryRunTaskFailure::UnsafeStorage,
            ),
            (
                CoreFailure::IncompatibleSchema,
                RustTargetDryRunTaskFailure::IncompatibleSchema,
            ),
            (
                CoreFailure::CorruptData,
                RustTargetDryRunTaskFailure::CorruptData,
            ),
            (
                CoreFailure::HistoryUnresolved,
                RustTargetDryRunTaskFailure::HistoryUnresolved,
            ),
            (
                CoreFailure::Unavailable,
                RustTargetDryRunTaskFailure::Unavailable,
            ),
            (
                CoreFailure::InternalState,
                RustTargetDryRunTaskFailure::InternalState,
            ),
        ] {
            assert_eq!(
                map_rust_target_dry_run_task_failure(TaskFailureKind::RustTargetDryRun(core))
                    .unwrap(),
                expected
            );
        }
        assert_eq!(
            map_rust_target_dry_run_task_failure(TaskFailureKind::InternalFailure),
            Err(RustTargetDryRunTaskError::InternalState)
        );

        for (core, expected) in [
            (
                CoreCancelOutcome::CancelledBeforeStart,
                RustTargetDryRunCancelOutcome::CancelledBeforeStart,
            ),
            (
                CoreCancelOutcome::Requested,
                RustTargetDryRunCancelOutcome::Requested,
            ),
            (
                CoreCancelOutcome::AlreadyRequested,
                RustTargetDryRunCancelOutcome::AlreadyRequested,
            ),
            (
                CoreCancelOutcome::AlreadyTerminal,
                RustTargetDryRunCancelOutcome::AlreadyTerminal,
            ),
        ] {
            assert_eq!(map_rust_target_dry_run_cancel_outcome(core), expected);
        }

        for (core, expected) in [
            (
                CoreRustTargetDryRunError::Closed,
                RustTargetDryRunStartError::Closed,
            ),
            (
                CoreRustTargetDryRunError::WrongEngine,
                RustTargetDryRunStartError::WrongEngine,
            ),
            (
                CoreRustTargetDryRunError::ParentReviewUnavailable,
                RustTargetDryRunStartError::ParentReviewUnavailable,
            ),
            (
                CoreRustTargetDryRunError::ReviewExpired,
                RustTargetDryRunStartError::ReviewExpired,
            ),
            (
                CoreRustTargetDryRunError::ChangedDuringReview,
                RustTargetDryRunStartError::ChangedDuringReview,
            ),
            (
                CoreRustTargetDryRunError::CancelledBeforeStart,
                RustTargetDryRunStartError::CancelledBeforeStart,
            ),
            (
                CoreRustTargetDryRunError::BudgetExceeded,
                RustTargetDryRunStartError::BudgetExceeded,
            ),
            (
                CoreRustTargetDryRunError::QueueFull,
                RustTargetDryRunStartError::QueueFull,
            ),
            (
                CoreRustTargetDryRunError::Busy,
                RustTargetDryRunStartError::Busy,
            ),
            (
                CoreRustTargetDryRunError::UnsafeStorage,
                RustTargetDryRunStartError::UnsafeStorage,
            ),
            (
                CoreRustTargetDryRunError::IncompatibleSchema,
                RustTargetDryRunStartError::IncompatibleSchema,
            ),
            (
                CoreRustTargetDryRunError::CorruptData,
                RustTargetDryRunStartError::CorruptData,
            ),
            (
                CoreRustTargetDryRunError::HistoryUnresolved,
                RustTargetDryRunStartError::HistoryUnresolved,
            ),
            (
                CoreRustTargetDryRunError::Unavailable,
                RustTargetDryRunStartError::Unavailable,
            ),
            (
                CoreRustTargetDryRunError::InternalState,
                RustTargetDryRunStartError::InternalState,
            ),
        ] {
            assert_eq!(map_rust_target_dry_run_start_error(core), expected);
        }
    }

    #[test]
    fn rust_target_dry_run_start_losing_an_info_race_is_irreversible() {
        let tracker = Arc::new(RustTargetPlanPreparationTracker::default());
        let session = RustTargetPlanReviewSession {
            state: Mutex::new(RustTargetPlanReviewState::Inspecting),
            parent_review: Weak::new(),
            operations: tracker,
            engine_closed: Arc::new(AtomicBool::new(false)),
        };
        assert!(matches!(
            session.take_for_dry_run_start(),
            Err(RustTargetDryRunStartError::ReviewUnavailable)
        ));
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
    fn rust_target_dry_run_wrong_engine_rejects_before_consuming_review() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let review = Arc::new(RustTargetPlanReviewSession {
            state: Mutex::new(RustTargetPlanReviewState::Inspecting),
            parent_review: Weak::new(),
            operations: Arc::new(RustTargetPlanPreparationTracker::default()),
            engine_closed: Arc::new(AtomicBool::new(false)),
        });
        assert!(matches!(
            engine.start_rust_target_dry_run(Arc::clone(&review)),
            Err(RustTargetDryRunStartError::WrongEngine)
        ));
        assert!(matches!(
            *review.state.lock().unwrap(),
            RustTargetPlanReviewState::Inspecting
        ));
        assert!(engine.close());
    }

    #[test]
    fn rust_target_dry_run_task_rejects_a_foreign_task_kind() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("dry-run-wrong-kind");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("payload"), b"scan only").unwrap();
        let scan = engine.start_scan(scan_request(&root)).unwrap();
        let dry_run = RustTargetDryRunTask {
            engine: scan.task.engine.clone(),
            id: scan.task.id,
        };
        assert_eq!(
            dry_run.poll(),
            Err(RustTargetDryRunTaskError::WrongTaskKind)
        );
        let _ = wait_for_scan(&scan.task);
        assert!(engine.close());
    }

    #[test]
    fn rust_target_cleanup_task_rejects_a_foreign_task_kind() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("cleanup-wrong-kind");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("payload"), b"scan only").unwrap();
        let scan = engine.start_scan(scan_request(&root)).unwrap();
        let cleanup = RustTargetCleanupTask {
            engine: scan.task.engine.clone(),
            id: scan.task.id,
        };
        assert_eq!(
            cleanup.poll(),
            Err(RustTargetCleanupTaskError::WrongTaskKind)
        );
        let _ = wait_for_scan(&scan.task);
        assert!(engine.close());
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

    #[cfg(target_os = "macos")]
    struct FfiRustTargetFixture {
        _temp: TempDir,
        engine: Arc<DuxEngine>,
        parent: Arc<SnapshotReviewSession>,
        review: Arc<RustTargetPlanReviewSession>,
        candidate_id: String,
        data_root: PathBuf,
        target: PathBuf,
        payload: PathBuf,
        manifest: PathBuf,
        lockfile: PathBuf,
        source: PathBuf,
    }

    #[cfg(target_os = "macos")]
    fn ffi_rust_target_fixture() -> FfiRustTargetFixture {
        const CARGO_CACHE_TAG: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55\n\
            # Cargo-generated cache directory\n";
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let temp = tempfile::tempdir_in(home).unwrap();
        let root = temp.path().join("scan-root");
        let project = root.join("project");
        let target = project.join("target");
        let manifest = project.join("Cargo.toml");
        let lockfile = project.join("Cargo.lock");
        let source = project.join("src/lib.rs");
        let cache_tag = target.join("CACHEDIR.TAG");
        let payload = target.join("object");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            &manifest,
            b"[package]\nname = 'fixture'\nversion = '0.1.0'\nedition = '2021'\n\n[workspace]\n",
        )
        .unwrap();
        std::fs::write(
            &lockfile,
            b"# This file is automatically @generated by Cargo.\n\
              # It is not intended for manual editing.\n\
              version = 4\n\
              \n\
              [[package]]\n\
              name = \"fixture\"\n\
              version = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(&source, b"pub fn fixture() {}\n").unwrap();
        std::fs::write(&cache_tag, CARGO_CACHE_TAG).unwrap();
        std::fs::write(&payload, b"temporary build output").unwrap();
        let stale_mtime = SystemTime::now() - Duration::from_secs(8 * 86_400);
        for path in [&cache_tag, &payload, &target] {
            std::fs::File::open(path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(stale_mtime))
                .unwrap();
        }

        let data_root = temp.path().join("data");
        let engine = Arc::new(
            DuxEngine::new(EngineStorageRoots {
                data_root: data_root.to_string_lossy().into_owned(),
                cache_root: temp.path().join("cache").to_string_lossy().into_owned(),
            })
            .unwrap(),
        );
        engine.set_permanent_cleanup_enabled(true).unwrap();
        let cargo = direct_toolchain_cargo();
        let enrollment = engine
            .inspect_direct_cargo_enrollment(direct_cargo_request(&cargo))
            .unwrap();
        engine.commit_direct_cargo_enrollment(enrollment).unwrap();
        let scan = engine.start_scan(scan_request(&root)).unwrap();
        let terminal = wait_for_scan_with_timeout(&scan.task, Duration::from_secs(60));
        assert_eq!(terminal.phase, TaskPhase::Succeeded);
        let scan_id = terminal.result.unwrap().scan_id;
        let parent = engine.acquire_explorer_snapshot_review(scan_id).unwrap();
        let candidates = parent.candidate_summaries(0, 64).unwrap();
        let candidate_id = candidates
            .candidates
            .iter()
            .find(|candidate| candidate.rule_id == "developer.rust.target")
            .expect("fixture scan should discover one Rust target")
            .candidate_id
            .clone();
        let review = engine
            .prepare_rust_target_plan_review(
                Arc::clone(&parent),
                RustTargetPlanReviewRequest {
                    record_version: FFI_RECORD_VERSION,
                    candidate_id: candidate_id.clone(),
                },
            )
            .unwrap();
        FfiRustTargetFixture {
            _temp: temp,
            engine,
            parent,
            review,
            candidate_id,
            data_root,
            target,
            payload,
            manifest,
            lockfile,
            source,
        }
    }

    fn wait_for_scan_with_timeout(task: &ScanTask, timeout: Duration) -> ScanPoll {
        let deadline = Instant::now() + timeout;
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

    #[cfg(target_os = "macos")]
    fn wait_for_rust_target_cleanup(task: &RustTargetCleanupTask) -> RustTargetCleanupPoll {
        let deadline = Instant::now() + Duration::from_secs(5 * 60);
        loop {
            let poll = task.poll().unwrap();
            if matches!(
                poll.phase,
                TaskPhase::Succeeded | TaskPhase::Failed | TaskPhase::Cancelled
            ) {
                return poll;
            }
            assert!(
                Instant::now() < deadline,
                "Rust-target cleanup did not become terminal"
            );
            std::thread::yield_now();
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires no active cargo/rustc process; run scripts/test_ffi_rust_target_cleanup.sh"]
    fn rust_target_cleanup_is_engine_bound_consume_once_path_free_and_history_correlated() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let fixture = ffi_rust_target_fixture();
        let (_foreign_temp, foreign) = engine();
        let same_store = DuxEngine::new(EngineStorageRoots {
            data_root: fixture.data_root.to_string_lossy().into_owned(),
            cache_root: fixture
                .data_root
                .parent()
                .unwrap()
                .join("peer-cache")
                .to_string_lossy()
                .into_owned(),
        })
        .unwrap();

        assert!(matches!(
            foreign.start_permanent_safe_cleanup(Arc::clone(&fixture.review)),
            Err(RustTargetCleanupStartError::WrongEngine)
        ));
        assert!(matches!(
            same_store.start_permanent_safe_cleanup(Arc::clone(&fixture.review)),
            Err(RustTargetCleanupStartError::WrongEngine)
        ));
        assert_eq!(
            fixture.review.info().unwrap().candidate_id,
            fixture.candidate_id
        );

        let task = fixture
            .engine
            .start_permanent_safe_cleanup(Arc::clone(&fixture.review))
            .unwrap();
        assert!(matches!(
            fixture
                .engine
                .start_permanent_safe_cleanup(Arc::clone(&fixture.review)),
            Err(RustTargetCleanupStartError::ReviewUnavailable)
        ));
        assert_eq!(
            fixture.review.release().unwrap(),
            RustTargetPlanReviewReleaseOutcome::AlreadyUnavailable
        );
        assert_eq!(
            fixture.parent.release().unwrap(),
            ReviewReleaseOutcome::Released
        );

        let terminal = wait_for_rust_target_cleanup(&task);
        assert_eq!(terminal.phase, TaskPhase::Succeeded);
        assert_eq!(terminal.failure, None);
        let result = terminal.result.unwrap();
        assert_eq!(result.status, CleanupSessionStatus::Completed);
        assert!(result.removed_entries >= 1);
        assert!(result.removed_logical_bytes > 0);
        assert!(is_rust_target_cleanup_session_id(&result.session_id));
        assert!(!fixture.payload.exists());
        assert!(fixture.target.join("CACHEDIR.TAG").exists());
        assert!(fixture.manifest.exists());
        assert!(fixture.lockfile.exists());
        assert!(fixture.source.exists());

        let history = fixture
            .engine
            .cleanup_session_history(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                session_id: result.session_id.clone(),
            })
            .unwrap();
        assert_eq!(history.summary.session_id, result.session_id);
        assert_eq!(history.summary.status, result.status);
        assert_eq!(
            history.summary.verified_capacity_delta_bytes,
            result.verified_capacity_delta_bytes
        );
        assert_eq!(
            task.cancel().unwrap(),
            RustTargetCleanupCancelOutcome::AlreadyTerminal
        );
        assert!(same_store.close());
        assert!(foreign.close());
        assert!(fixture.engine.close());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires no active cargo/rustc process; run scripts/test_ffi_rust_target_cleanup.sh"]
    fn rust_target_cleanup_refusal_is_one_shot_and_close_drains_start_operation() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let fixture = ffi_rust_target_fixture();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (continue_tx, continue_rx) = std::sync::mpsc::channel();
        let start_thread = {
            let engine = Arc::clone(&fixture.engine);
            let review = Arc::clone(&fixture.review);
            std::thread::spawn(move || {
                engine.start_permanent_safe_cleanup_with(review, |_engine, review| {
                    started_tx.send(()).unwrap();
                    continue_rx.recv().unwrap();
                    Err((CoreRustTargetCleanupError::Busy, Box::new(review)))
                })
            })
        };
        started_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("cleanup start should consume its review");
        assert_eq!(
            fixture
                .engine
                .rust_target_plan_preparations
                .state
                .lock()
                .unwrap()
                .active,
            1,
            "the admitted cleanup start must enter the shared operation tracker"
        );
        assert!(matches!(
            *fixture.review.state.lock().unwrap(),
            RustTargetPlanReviewState::Consumed
        ));
        continue_tx.send(()).unwrap();
        assert!(matches!(
            start_thread.join().unwrap(),
            Err(RustTargetCleanupStartError::Busy)
        ));
        assert_eq!(
            fixture
                .engine
                .rust_target_plan_preparations
                .state
                .lock()
                .unwrap()
                .active,
            0
        );
        assert!(matches!(
            fixture
                .engine
                .start_permanent_safe_cleanup(Arc::clone(&fixture.review)),
            Err(RustTargetCleanupStartError::ReviewUnavailable)
        ));
        assert_eq!(
            fixture.review.release().unwrap(),
            RustTargetPlanReviewReleaseOutcome::AlreadyUnavailable
        );

        let close_operation = fixture
            .engine
            .rust_target_plan_preparations
            .enter_operation(&fixture.engine.closed)
            .unwrap();
        let close_thread = {
            let engine = Arc::clone(&fixture.engine);
            std::thread::spawn(move || engine.close())
        };
        wait_until("cleanup close admission", || {
            fixture.engine.closed.load(Ordering::Acquire)
        });
        assert!(
            !close_thread.is_finished(),
            "engine close must wait for the admitted cleanup operation"
        );
        drop(close_operation);
        assert!(close_thread.join().unwrap());
        assert_eq!(
            fixture
                .engine
                .rust_target_plan_preparations
                .state
                .lock()
                .unwrap()
                .active,
            0
        );
        assert!(fixture.payload.exists());
        assert!(fixture.target.join("CACHEDIR.TAG").exists());
        assert!(
            fixture.engine.recent_cleanup_history(None, 64).is_err(),
            "closed engines must not fabricate cleanup history"
        );
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
    fn recurring_storage_thief_feed_is_bounded_empty_and_closed() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let ranking = engine.recurring_storage_thieves().unwrap();
        assert_eq!(ranking.record_version, FFI_RECORD_VERSION);
        assert_eq!(ranking.permanent_safe_session_count, 0);
        assert_eq!(ranking.manual_cleanup_session_count, 0);
        assert_eq!(ranking.ranked_rule_count, 0);
        assert!(!ranking.has_older_permanent_safe_sessions);
        assert!(ranking.groups.is_empty());
        assert!(engine.close());
        assert_eq!(
            engine.recurring_storage_thieves(),
            Err(StorageThiefError::Closed)
        );
    }

    #[test]
    fn running_scan_debt_census_is_versioned_empty_and_closed() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let census = engine.running_scan_debt_census().unwrap();
        assert_eq!(
            census,
            RunningScanDebtCensus {
                record_version: FFI_RECORD_VERSION,
                inspected_unclaimed_count: 0,
                pristine_unclaimed_count: 0,
                unexplained_unclaimed_count: 0,
                has_more: false,
            }
        );
        assert!(engine.close());
        assert_eq!(
            engine.running_scan_debt_census(),
            Err(RunningScanDebtCensusError::Closed)
        );
    }

    #[test]
    fn running_scan_debt_error_mapping_is_exact() {
        for (core, projected) in [
            (
                CoreRunningScanDebtCensusError::Closed,
                RunningScanDebtCensusError::Closed,
            ),
            (
                CoreRunningScanDebtCensusError::IncompatibleSchema,
                RunningScanDebtCensusError::IncompatibleSchema,
            ),
            (
                CoreRunningScanDebtCensusError::Busy,
                RunningScanDebtCensusError::Busy,
            ),
            (
                CoreRunningScanDebtCensusError::UnsafeStorage,
                RunningScanDebtCensusError::UnsafeStorage,
            ),
            (
                CoreRunningScanDebtCensusError::QueryLimitExceeded,
                RunningScanDebtCensusError::BudgetExceeded,
            ),
            (
                CoreRunningScanDebtCensusError::CorruptData,
                RunningScanDebtCensusError::CorruptData,
            ),
            (
                CoreRunningScanDebtCensusError::Unavailable,
                RunningScanDebtCensusError::Unavailable,
            ),
            (
                CoreRunningScanDebtCensusError::InternalState,
                RunningScanDebtCensusError::InternalState,
            ),
        ] {
            assert_eq!(map_running_scan_debt_census_error(core), projected);
        }
    }

    #[test]
    fn claimed_running_scan_provenance_census_is_versioned_empty_and_closed() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let census = engine.claimed_running_scan_provenance_census().unwrap();
        assert_eq!(
            census,
            ClaimedRunningScanProvenanceCensus {
                record_version: FFI_RECORD_VERSION,
                inspected_claimed_count: 0,
                same_host_current_boot_count: 0,
                same_host_prior_boot_count: 0,
                foreign_host_count: 0,
                stored_unproven_count: 0,
                current_context_unavailable_count: 0,
                has_more: false,
            }
        );
        assert!(engine.close());
        assert_eq!(
            engine.claimed_running_scan_provenance_census(),
            Err(ClaimedRunningScanProvenanceCensusError::Closed)
        );
    }

    #[test]
    fn claimed_running_scan_provenance_projection_is_strict_and_path_free() {
        let projected =
            project_claimed_running_scan_provenance_census(64, 10, 11, 12, 31, 0, true).unwrap();
        assert_eq!(
            projected,
            ClaimedRunningScanProvenanceCensus {
                record_version: FFI_RECORD_VERSION,
                inspected_claimed_count: 64,
                same_host_current_boot_count: 10,
                same_host_prior_boot_count: 11,
                foreign_host_count: 12,
                stored_unproven_count: 31,
                current_context_unavailable_count: 0,
                has_more: true,
            }
        );
        assert!(project_claimed_running_scan_provenance_census(64, 0, 0, 0, 14, 50, true,).is_ok());

        for malformed in [
            (65, 65, 0, 0, 0, 0, false),
            (4, 1, 1, 1, 0, 0, false),
            (u16::MAX, u16::MAX, 1, 0, 0, 0, false),
            (63, 63, 0, 0, 0, 0, true),
            (2, 1, 0, 0, 0, 1, false),
        ] {
            assert_eq!(
                project_claimed_running_scan_provenance_census(
                    malformed.0,
                    malformed.1,
                    malformed.2,
                    malformed.3,
                    malformed.4,
                    malformed.5,
                    malformed.6,
                ),
                Err(ClaimedRunningScanProvenanceCensusError::CorruptData)
            );
        }
    }

    #[test]
    fn claimed_running_scan_provenance_error_mapping_is_exact() {
        for (core, projected) in [
            (
                CoreClaimedRunningScanProvenanceCensusError::Closed,
                ClaimedRunningScanProvenanceCensusError::Closed,
            ),
            (
                CoreClaimedRunningScanProvenanceCensusError::IncompatibleSchema,
                ClaimedRunningScanProvenanceCensusError::IncompatibleSchema,
            ),
            (
                CoreClaimedRunningScanProvenanceCensusError::Busy,
                ClaimedRunningScanProvenanceCensusError::Busy,
            ),
            (
                CoreClaimedRunningScanProvenanceCensusError::UnsafeStorage,
                ClaimedRunningScanProvenanceCensusError::UnsafeStorage,
            ),
            (
                CoreClaimedRunningScanProvenanceCensusError::QueryLimitExceeded,
                ClaimedRunningScanProvenanceCensusError::BudgetExceeded,
            ),
            (
                CoreClaimedRunningScanProvenanceCensusError::CorruptData,
                ClaimedRunningScanProvenanceCensusError::CorruptData,
            ),
            (
                CoreClaimedRunningScanProvenanceCensusError::Unavailable,
                ClaimedRunningScanProvenanceCensusError::Unavailable,
            ),
            (
                CoreClaimedRunningScanProvenanceCensusError::InternalState,
                ClaimedRunningScanProvenanceCensusError::InternalState,
            ),
        ] {
            assert_eq!(
                map_claimed_running_scan_provenance_census_error(core),
                projected
            );
        }
    }

    #[test]
    fn storage_thief_fraction_order_and_error_mapping_are_exact() {
        let group = |rule_id: &str, bytes: u64, seconds: u64| StorageThiefGroup {
            record_version: FFI_RECORD_VERSION,
            rank: 1,
            rule_id: rule_id.to_owned(),
            latest_rule_revision: 1,
            observed_revision_count: 1,
            successful_cleanup_count: 1,
            successful_manual_cleanup_count: 1,
            observed_regrowth_cycle_count: 1,
            manual_regrowth_cycle_count: 1,
            total_observed_regrown_bytes: bytes,
            total_regrowth_duration_seconds: seconds,
            total_regrowth_duration_nanoseconds: 0,
            bytes_regrown_per_day: 0,
            rate_capped: false,
            latest_cleanup_at_unix_ms: 1,
            latest_regrowth_at_unix_ms: 2,
            automation_history_threshold_met: false,
        };
        assert_eq!(
            compare_ffi_storage_thief_rates(
                &group("rule.a", u64::MAX, u64::MAX - 1),
                &group("rule.b", u64::MAX - 1, u64::MAX)
            ),
            std::cmp::Ordering::Greater
        );
        for (core, projected) in [
            (CoreStorageThiefError::Closed, StorageThiefError::Closed),
            (
                CoreStorageThiefError::IncompatibleSchema,
                StorageThiefError::IncompatibleSchema,
            ),
            (CoreStorageThiefError::Busy, StorageThiefError::Busy),
            (
                CoreStorageThiefError::UnsafeStorage,
                StorageThiefError::UnsafeStorage,
            ),
            (
                CoreStorageThiefError::QueryLimitExceeded,
                StorageThiefError::BudgetExceeded,
            ),
            (
                CoreStorageThiefError::CorruptData,
                StorageThiefError::CorruptData,
            ),
            (
                CoreStorageThiefError::Unavailable,
                StorageThiefError::Unavailable,
            ),
            (
                CoreStorageThiefError::InternalState,
                StorageThiefError::InternalState,
            ),
        ] {
            assert_eq!(map_storage_thief_error(core), projected);
        }
    }

    #[test]
    fn rule_outcome_states_project_exactly_and_fail_closed() {
        let cleaned = UNIX_EPOCH + Duration::from_millis(1_000);
        let zero = UNIX_EPOCH + Duration::from_millis(2_000);
        let observed = UNIX_EPOCH + Duration::from_millis(3_500);
        let reasons = [
            (
                CoreRuleOutcomeNotEligibleReason::SourceCleanupIncomplete,
                RuleOutcomeNotEligibleReason::SourceCleanupIncomplete,
            ),
            (
                CoreRuleOutcomeNotEligibleReason::ItemNotSuccessfulPermanentRegenerable,
                RuleOutcomeNotEligibleReason::ItemNotSuccessfulPermanentRegenerable,
            ),
            (
                CoreRuleOutcomeNotEligibleReason::SourceScanNotComparable,
                RuleOutcomeNotEligibleReason::SourceScanNotComparable,
            ),
            (
                CoreRuleOutcomeNotEligibleReason::SourceEvaluationNotComparable,
                RuleOutcomeNotEligibleReason::SourceEvaluationNotComparable,
            ),
            (
                CoreRuleOutcomeNotEligibleReason::SourceEvaluationAfterPlan,
                RuleOutcomeNotEligibleReason::SourceEvaluationAfterPlan,
            ),
            (
                CoreRuleOutcomeNotEligibleReason::SourceCandidateMismatch,
                RuleOutcomeNotEligibleReason::SourceCandidateMismatch,
            ),
        ];
        for (core, projected) in reasons {
            assert_eq!(
                project_rule_outcome_state(&CoreRuleOutcomeState::NotEligible { reason: core }),
                Ok(RuleOutcomeState::NotEligible { reason: projected })
            );
        }

        assert_eq!(
            project_rule_outcome_state(&CoreRuleOutcomeState::AwaitingComparableScan {
                cleaned_at: cleaned,
            }),
            Ok(RuleOutcomeState::AwaitingComparableScan {
                cleaned_at_unix_ms: 1_000,
            })
        );
        assert_eq!(
            project_rule_outcome_state(&CoreRuleOutcomeState::Superseded {
                cleaned_at: cleaned,
                superseded_at: zero,
            }),
            Ok(RuleOutcomeState::Superseded {
                cleaned_at_unix_ms: 1_000,
                superseded_at_unix_ms: 2_000,
            })
        );
        assert_eq!(
            project_rule_outcome_state(&CoreRuleOutcomeState::LaterSizeObserved {
                cleaned_at: cleaned,
                observed_at: observed,
                observed_bytes: 4_096,
            }),
            Ok(RuleOutcomeState::LaterSizeObserved {
                cleaned_at_unix_ms: 1_000,
                observed_at_unix_ms: 3_500,
                observed_bytes: 4_096,
            })
        );
        assert_eq!(
            project_rule_outcome_state(&CoreRuleOutcomeState::ZeroBaselineObserved {
                cleaned_at: cleaned,
                observed_at: zero,
            }),
            Ok(RuleOutcomeState::ZeroBaselineObserved {
                cleaned_at_unix_ms: 1_000,
                observed_at_unix_ms: 2_000,
            })
        );
        assert_eq!(
            project_rule_outcome_state(&CoreRuleOutcomeState::Regrown {
                cleaned_at: cleaned,
                zero_observed_at: zero,
                observed_at: observed,
                observed_bytes: 8_192,
                regrowth_duration: Duration::from_millis(1_500),
            }),
            Ok(RuleOutcomeState::Regrown {
                cleaned_at_unix_ms: 1_000,
                zero_observed_at_unix_ms: 2_000,
                observed_at_unix_ms: 3_500,
                observed_bytes: 8_192,
            })
        );

        for invalid in [
            CoreRuleOutcomeState::Superseded {
                cleaned_at: zero,
                superseded_at: cleaned,
            },
            CoreRuleOutcomeState::LaterSizeObserved {
                cleaned_at: cleaned,
                observed_at: cleaned,
                observed_bytes: 1,
            },
            CoreRuleOutcomeState::LaterSizeObserved {
                cleaned_at: cleaned,
                observed_at: observed,
                observed_bytes: 0,
            },
            CoreRuleOutcomeState::ZeroBaselineObserved {
                cleaned_at: zero,
                observed_at: cleaned,
            },
            CoreRuleOutcomeState::Regrown {
                cleaned_at: cleaned,
                zero_observed_at: zero,
                observed_at: observed,
                observed_bytes: 1,
                regrowth_duration: Duration::from_millis(1_499),
            },
        ] {
            assert_eq!(
                project_rule_outcome_state(&invalid),
                Err(RuleOutcomeError::CorruptData)
            );
        }
        assert_eq!(
            project_rule_outcome_state(&CoreRuleOutcomeState::AwaitingComparableScan {
                cleaned_at: UNIX_EPOCH - Duration::from_millis(1),
            }),
            Err(RuleOutcomeError::CorruptData)
        );
        let same_projected_millisecond = UNIX_EPOCH + Duration::from_micros(1_001);
        assert_eq!(
            project_rule_outcome_state(&CoreRuleOutcomeState::LaterSizeObserved {
                cleaned_at: UNIX_EPOCH + Duration::from_millis(1),
                observed_at: same_projected_millisecond,
                observed_bytes: 1,
            }),
            Err(RuleOutcomeError::CorruptData)
        );
        assert_eq!(
            project_rule_outcome_state(&CoreRuleOutcomeState::Regrown {
                cleaned_at: UNIX_EPOCH,
                zero_observed_at: UNIX_EPOCH + Duration::from_millis(1),
                observed_at: same_projected_millisecond,
                observed_bytes: 1,
                regrowth_duration: Duration::from_micros(1),
            }),
            Err(RuleOutcomeError::CorruptData)
        );
    }

    #[test]
    fn rule_outcome_endpoint_is_exact_versioned_path_free_and_typed() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let session_id =
            seed_terminal_cleanup_history(&temp, &engine, "rule-outcome-path-sentinel");
        let batch = engine
            .rule_outcomes_for_cleanup_session(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                session_id: session_id.clone(),
            })
            .unwrap();
        assert_eq!(batch.record_version, FFI_RECORD_VERSION);
        assert_eq!(batch.session_id, session_id);
        assert_eq!(batch.outcomes.len(), 1);
        assert_eq!(batch.outcomes[0].record_version, FFI_RECORD_VERSION);
        assert_eq!(batch.outcomes[0].item_ordinal, 0);
        assert!(batch.outcomes[0].rule_revision > 0);
        assert!(matches!(
            batch.outcomes[0].state,
            RuleOutcomeState::NotEligible {
                reason: RuleOutcomeNotEligibleReason::ItemNotSuccessfulPermanentRegenerable
            }
        ));
        let debug = format!("{batch:?}");
        assert!(!debug.contains("reviewed.bin"));
        assert!(!debug.contains(&temp.path().to_string_lossy().into_owned()));

        assert_eq!(
            engine.rule_outcomes_for_cleanup_session(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION + 1,
                session_id: batch.session_id.clone(),
            }),
            Err(RuleOutcomeError::InvalidRecordVersion)
        );
        assert_eq!(
            engine.rule_outcomes_for_cleanup_session(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                session_id: "not/a/session".to_owned(),
            }),
            Err(RuleOutcomeError::InvalidSessionId)
        );
        assert_eq!(
            engine.rule_outcomes_for_cleanup_session(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                session_id: "session:missing".to_owned(),
            }),
            Err(RuleOutcomeError::SessionNotFound)
        );
        assert!(engine.close());
        assert_eq!(
            engine.rule_outcomes_for_cleanup_session(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                session_id: batch.session_id,
            }),
            Err(RuleOutcomeError::Closed)
        );
    }

    #[test]
    fn rule_outcome_errors_map_one_to_one() {
        let cases = [
            (CoreRuleOutcomeError::Closed, RuleOutcomeError::Closed),
            (
                CoreRuleOutcomeError::SessionNotFound,
                RuleOutcomeError::SessionNotFound,
            ),
            (
                CoreRuleOutcomeError::IncompatibleSchema,
                RuleOutcomeError::IncompatibleSchema,
            ),
            (CoreRuleOutcomeError::Busy, RuleOutcomeError::Busy),
            (
                CoreRuleOutcomeError::UnsafeStorage,
                RuleOutcomeError::UnsafeStorage,
            ),
            (
                CoreRuleOutcomeError::QueryLimitExceeded,
                RuleOutcomeError::BudgetExceeded,
            ),
            (
                CoreRuleOutcomeError::CorruptData,
                RuleOutcomeError::CorruptData,
            ),
            (
                CoreRuleOutcomeError::Unavailable,
                RuleOutcomeError::Unavailable,
            ),
            (
                CoreRuleOutcomeError::InternalState,
                RuleOutcomeError::InternalState,
            ),
        ];
        for (core, ffi) in cases {
            assert_eq!(map_rule_outcome_error(core), ffi);
        }
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
    fn cleanup_history_clear_preview_is_engine_bound_consume_once_and_close_drained() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, primary) = engine();
        let primary = Arc::new(primary);
        assert_eq!(
            primary.prepare_cleanup_history_clear().err().unwrap(),
            CleanupHistoryClearError::NothingToClear
        );
        let session_id = seed_terminal_cleanup_history(&temp, &primary, "clear-history-primary");
        let same_store = DuxEngine::new(EngineStorageRoots {
            data_root: temp.path().join("data").to_string_lossy().into_owned(),
            cache_root: temp
                .path()
                .join("cache-peer")
                .to_string_lossy()
                .into_owned(),
        })
        .unwrap();
        let (_foreign_temp, foreign) = engine();

        let released = primary.prepare_cleanup_history_clear().unwrap();
        let info = released.info().unwrap();
        assert_eq!(info.record_version, FFI_RECORD_VERSION);
        assert_eq!(info.session_count, 1);
        assert!(info.oldest_started_at_unix_ms <= info.newest_started_at_unix_ms);
        assert!(info.prepared_at_unix_ms < info.expires_at_unix_ms);
        assert_eq!(
            primary.prepare_cleanup_history_clear().err().unwrap(),
            CleanupHistoryClearError::Busy
        );
        assert_eq!(
            same_store.clear_cleanup_history(Arc::clone(&released)),
            Err(CleanupHistoryClearError::WrongEngine)
        );
        assert_eq!(
            foreign.clear_cleanup_history(Arc::clone(&released)),
            Err(CleanupHistoryClearError::WrongEngine)
        );
        assert_eq!(released.info().unwrap(), info);
        assert_eq!(
            released.release().unwrap(),
            CleanupHistoryClearPreviewReleaseOutcome::Released
        );
        assert_eq!(
            released.release().unwrap(),
            CleanupHistoryClearPreviewReleaseOutcome::AlreadyUnavailable
        );
        assert_eq!(
            released.info(),
            Err(CleanupHistoryClearError::PreviewUnavailable)
        );

        let committed = primary.prepare_cleanup_history_clear().unwrap();
        let result = primary
            .clear_cleanup_history(Arc::clone(&committed))
            .unwrap();
        assert_eq!(
            result,
            CleanupHistoryClearResult {
                record_version: FFI_RECORD_VERSION,
                cleared_session_count: 1,
            }
        );
        assert_eq!(
            primary.clear_cleanup_history(committed),
            Err(CleanupHistoryClearError::PreviewUnavailable)
        );
        assert!(
            primary
                .recent_cleanup_history(None, 64)
                .unwrap()
                .records
                .is_empty()
        );
        assert_eq!(
            primary.cleanup_session_history(CleanupSessionHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                session_id,
            }),
            Err(CleanupHistoryError::SessionNotFound)
        );

        seed_terminal_cleanup_history(&temp, &primary, "clear-history-close");
        let close_drained = primary.prepare_cleanup_history_clear().unwrap();
        assert!(primary.close());
        assert_eq!(close_drained.info(), Err(CleanupHistoryClearError::Closed));
        assert_eq!(
            close_drained.release().unwrap(),
            CleanupHistoryClearPreviewReleaseOutcome::AlreadyUnavailable
        );
        assert_eq!(
            primary.clear_cleanup_history(close_drained),
            Err(CleanupHistoryClearError::Closed)
        );
        assert_eq!(
            primary.prepare_cleanup_history_clear().err().unwrap(),
            CleanupHistoryClearError::Closed
        );
        assert!(foreign.close());
        assert!(same_store.close());
    }

    #[test]
    fn cleanup_history_clear_close_wins_prepare_registration() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let engine = Arc::new(engine);
        seed_terminal_cleanup_history(&temp, &engine, "clear-history-prepare-close");

        let registry_guard = engine.cleanup_history_clear_previews.lock().unwrap();
        let prepare_thread = {
            let engine = Arc::clone(&engine);
            std::thread::spawn(move || engine.prepare_cleanup_history_clear())
        };
        wait_until("clear preview preparation to acquire engine state", || {
            mutex_is_locked(&engine.state)
        });
        let close_thread = {
            let engine = Arc::clone(&engine);
            std::thread::spawn(move || engine.close())
        };
        wait_until("clear preview preparation close admission", || {
            engine.closed.load(Ordering::Acquire)
        });
        drop(registry_guard);

        assert!(matches!(
            prepare_thread.join().unwrap(),
            Err(CleanupHistoryClearError::Closed)
        ));
        let close_result = close_thread.join().unwrap();
        if !close_result {
            wait_until("clear preview preparation background close", || {
                matches!(*engine.state.lock().unwrap(), EngineState::Closed { .. })
            });
        }
        assert!(
            engine
                .cleanup_history_clear_previews
                .lock()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn cleanup_history_clear_close_wins_clear_admission() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let engine = Arc::new(engine);
        seed_terminal_cleanup_history(&temp, &engine, "clear-history-admission-close");
        let preview = engine.prepare_cleanup_history_clear().unwrap();

        let state_guard = engine.state.lock().unwrap();
        let close_thread = {
            let engine = Arc::clone(&engine);
            std::thread::spawn(move || engine.close())
        };
        wait_until("cleanup-history clear close admission", || {
            engine.closed.load(Ordering::Acquire)
        });
        let clear_thread = {
            let engine = Arc::clone(&engine);
            let preview = Arc::clone(&preview);
            std::thread::spawn(move || engine.clear_cleanup_history(preview))
        };
        drop(state_guard);

        assert_eq!(
            clear_thread.join().unwrap(),
            Err(CleanupHistoryClearError::Closed)
        );
        let close_result = close_thread.join().unwrap();
        if !close_result {
            wait_until("cleanup-history clear background close", || {
                matches!(*engine.state.lock().unwrap(), EngineState::Closed { .. })
            });
        }
        assert_eq!(preview.info(), Err(CleanupHistoryClearError::Closed));
        assert_eq!(
            preview.release().unwrap(),
            CleanupHistoryClearPreviewReleaseOutcome::AlreadyUnavailable
        );
    }

    #[test]
    fn cleanup_history_clear_and_release_race_has_one_terminal_winner() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let engine = Arc::new(engine);
        seed_terminal_cleanup_history(&temp, &engine, "clear-history-release-race");
        let preview = engine.prepare_cleanup_history_clear().unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));

        let clear_thread = {
            let engine = Arc::clone(&engine);
            let preview = Arc::clone(&preview);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                engine.clear_cleanup_history(preview)
            })
        };
        let release_thread = {
            let preview = Arc::clone(&preview);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                preview.release()
            })
        };
        barrier.wait();

        let clear_result = clear_thread.join().unwrap();
        let release_result = release_thread.join().unwrap().unwrap();
        let expected_history_count = match clear_result {
            Ok(result) => {
                assert_eq!(result.cleared_session_count, 1);
                assert_eq!(
                    release_result,
                    CleanupHistoryClearPreviewReleaseOutcome::AlreadyUnavailable
                );
                0
            }
            Err(CleanupHistoryClearError::PreviewUnavailable) => {
                assert_eq!(
                    release_result,
                    CleanupHistoryClearPreviewReleaseOutcome::Released
                );
                1
            }
            other => panic!("unexpected cleanup-history clear/release race result: {other:?}"),
        };
        assert_eq!(
            engine
                .recent_cleanup_history(None, 64)
                .unwrap()
                .records
                .len(),
            expected_history_count
        );
        assert_eq!(
            preview.info(),
            Err(CleanupHistoryClearError::PreviewUnavailable)
        );
        assert!(engine.close());
    }

    #[test]
    fn cleanup_history_clear_change_consumes_preview_without_retry() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        seed_terminal_cleanup_history(&temp, &engine, "clear-history-before");
        let preview = engine.prepare_cleanup_history_clear().unwrap();
        seed_terminal_cleanup_history(&temp, &engine, "clear-history-after");
        assert_eq!(
            engine.clear_cleanup_history(Arc::clone(&preview)),
            Err(CleanupHistoryClearError::ChangedSincePreview)
        );
        assert_eq!(
            engine.clear_cleanup_history(preview),
            Err(CleanupHistoryClearError::PreviewUnavailable)
        );
        assert_eq!(
            engine
                .recent_cleanup_history(None, 64)
                .unwrap()
                .records
                .len(),
            2
        );
        assert!(engine.close());
    }

    #[test]
    fn cleanup_history_clear_projection_accepts_future_durable_history_time() {
        let prepared_at = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let expires_at = prepared_at + Duration::from_secs(120);
        let oldest_started_at = prepared_at - Duration::from_secs(60);
        let newest_started_at = prepared_at + Duration::from_secs(1);

        let projected = cleanup_history_clear_preview_info_values(
            2,
            oldest_started_at,
            newest_started_at,
            prepared_at,
            expires_at,
        )
        .unwrap();
        assert!(projected.newest_started_at_unix_ms > projected.prepared_at_unix_ms);
        assert_eq!(projected.session_count, 2);
        assert_eq!(projected.record_version, FFI_RECORD_VERSION);

        assert_eq!(
            cleanup_history_clear_preview_info_values(
                2,
                newest_started_at,
                oldest_started_at,
                prepared_at,
                expires_at,
            ),
            Err(CleanupHistoryClearError::CorruptData)
        );
        assert_eq!(
            cleanup_history_clear_preview_info_values(
                2,
                oldest_started_at,
                newest_started_at,
                prepared_at,
                prepared_at,
            ),
            Err(CleanupHistoryClearError::CorruptData)
        );
    }

    #[test]
    fn cleanup_history_clear_errors_and_post_mutation_shape_are_exhaustive() {
        for (core, expected) in [
            (
                CoreCleanupHistoryClearError::Closed,
                CleanupHistoryClearError::Closed,
            ),
            (
                CoreCleanupHistoryClearError::NothingToClear,
                CleanupHistoryClearError::NothingToClear,
            ),
            (
                CoreCleanupHistoryClearError::ActiveCleanup,
                CleanupHistoryClearError::ActiveCleanup,
            ),
            (
                CoreCleanupHistoryClearError::ChangedSincePreview,
                CleanupHistoryClearError::ChangedSincePreview,
            ),
            (
                CoreCleanupHistoryClearError::PreviewExpired,
                CleanupHistoryClearError::PreviewExpired,
            ),
            (
                CoreCleanupHistoryClearError::WrongEngine,
                CleanupHistoryClearError::WrongEngine,
            ),
            (
                CoreCleanupHistoryClearError::IncompatibleSchema,
                CleanupHistoryClearError::IncompatibleSchema,
            ),
            (
                CoreCleanupHistoryClearError::Busy,
                CleanupHistoryClearError::Busy,
            ),
            (
                CoreCleanupHistoryClearError::UnsafeStorage,
                CleanupHistoryClearError::UnsafeStorage,
            ),
            (
                CoreCleanupHistoryClearError::QueryLimitExceeded,
                CleanupHistoryClearError::BudgetExceeded,
            ),
            (
                CoreCleanupHistoryClearError::CorruptData,
                CleanupHistoryClearError::CorruptData,
            ),
            (
                CoreCleanupHistoryClearError::OutcomeUnknown,
                CleanupHistoryClearError::OutcomeUnknown,
            ),
            (
                CoreCleanupHistoryClearError::Unavailable,
                CleanupHistoryClearError::Unavailable,
            ),
            (
                CoreCleanupHistoryClearError::InternalState,
                CleanupHistoryClearError::InternalState,
            ),
        ] {
            assert_eq!(map_cleanup_history_clear_error(core), expected);
        }
        assert_eq!(
            cleanup_history_clear_result_count(0, 1),
            Err(CleanupHistoryClearError::OutcomeUnknown)
        );
        assert_eq!(
            cleanup_history_clear_result_count(2, 1),
            Err(CleanupHistoryClearError::OutcomeUnknown)
        );
        assert_eq!(
            cleanup_history_clear_result_count(1, 1).unwrap(),
            CleanupHistoryClearResult {
                record_version: FFI_RECORD_VERSION,
                cleared_session_count: 1,
            }
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

    #[test]
    fn pressure_episode_history_round_trip_is_anchored_bounded_and_path_free() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let volume_id = "01234567-89AB-CDEF-0123-456789ABCDEF";
        let base = 1_800_000_000_000_i64;
        engine
            .set_disk_pressure_policy(PressurePolicyInput {
                record_version: FFI_RECORD_VERSION,
                critical_available_bytes: 100,
                critical_available_basis_points: 1_000,
                warning_available_bytes: 300,
                warning_available_basis_points: 3_000,
                recovery_bytes: 20,
                recovery_basis_points: 100,
            })
            .unwrap();
        for (offset, available) in [(0, 250), (60_000, 50), (120_000, 900)] {
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

        let page = engine
            .get_pressure_episode_history(PressureEpisodeHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                stable_volume_id: volume_id.to_owned(),
                anchor_at_unix_ms: base + 120_000,
                limit: 1,
            })
            .unwrap();
        assert_eq!(page.record_version, FFI_RECORD_VERSION);
        assert_eq!(
            page.stable_volume_id,
            "volume:macos:01234567-89ab-cdef-0123-456789abcdef"
        );
        assert_eq!(page.anchor_at_unix_ms, base + 120_000);
        assert!(page.has_more);
        assert_eq!(
            page.episodes,
            vec![PressureEpisodeRecord {
                record_version: FFI_RECORD_VERSION,
                level: PressureEpisodeLevel::Critical,
                entered_at_unix_ms: base + 60_000,
                exited_at_unix_ms: Some(base + 120_000),
                policy_revision: 1,
            }]
        );

        let at_escalation = engine
            .get_pressure_episode_history(PressureEpisodeHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                stable_volume_id: volume_id.to_owned(),
                anchor_at_unix_ms: base + 60_000,
                limit: 2,
            })
            .unwrap();
        assert_eq!(at_escalation.episodes.len(), 2);
        assert_eq!(
            at_escalation.episodes[0].level,
            PressureEpisodeLevel::Critical
        );
        assert_eq!(at_escalation.episodes[0].exited_at_unix_ms, None);
        assert_eq!(
            at_escalation.episodes[1].exited_at_unix_ms,
            Some(base + 60_000)
        );

        for request in [
            PressureEpisodeHistoryRequest {
                record_version: FFI_RECORD_VERSION + 1,
                stable_volume_id: volume_id.to_owned(),
                anchor_at_unix_ms: base,
                limit: 1,
            },
            PressureEpisodeHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                stable_volume_id: volume_id.to_owned(),
                anchor_at_unix_ms: base,
                limit: 0,
            },
            PressureEpisodeHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                stable_volume_id: volume_id.to_owned(),
                anchor_at_unix_ms: -1,
                limit: 1,
            },
            PressureEpisodeHistoryRequest {
                record_version: FFI_RECORD_VERSION,
                stable_volume_id: volume_id.to_owned(),
                anchor_at_unix_ms: base + 180_000,
                limit: 1,
            },
        ] {
            assert_eq!(
                engine.get_pressure_episode_history(request),
                Err(EngineError::InvalidPressureEpisodeRequest)
            );
        }
        assert!(engine.close());
    }

    fn scan_request(root: &std::path::Path) -> ScanRequest {
        ScanRequest {
            record_version: FFI_RECORD_VERSION,
            root: root.to_string_lossy().into_owned(),
        }
    }

    fn eligible_icloud_facts() -> ICloudLocalCopyRawFacts {
        ICloudLocalCopyRawFacts {
            record_version: FFI_RECORD_VERSION,
            ubiquitous: ICloudBooleanState::True,
            uploaded: ICloudBooleanState::True,
            uploading: ICloudBooleanState::False,
            upload_error: ICloudErrorState::Absent,
            unresolved_conflicts: ICloudBooleanState::False,
            local_copy_state: ICloudLocalCopyState::Current,
            download_requested: ICloudBooleanState::False,
            downloading: ICloudBooleanState::False,
            download_error: ICloudErrorState::Absent,
            excluded_from_sync: ICloudBooleanState::False,
            account_identity: ICloudIdentityFactState::Stable,
            container_identity: ICloudIdentityFactState::Unsupported,
            item_generation: ICloudIdentityFactState::Stable,
            file_version: ICloudIdentityFactState::Stable,
            shared: ICloudBooleanState::False,
            sync_paused: ICloudBooleanState::False,
        }
    }

    struct FixedICloudMetadataDriver {
        result: ICloudLocalCopyMetadataResult,
        consume_path: bool,
        panic: bool,
        calls: Arc<AtomicU64>,
        paths: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl FixedICloudMetadataDriver {
        fn new(result: ICloudLocalCopyMetadataResult, consume_path: bool) -> Self {
            Self {
                result,
                consume_path,
                panic: false,
                calls: Arc::new(AtomicU64::new(0)),
                paths: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn panicking() -> Self {
            Self {
                result: ICloudLocalCopyMetadataResult::Failed,
                consume_path: true,
                panic: true,
                calls: Arc::new(AtomicU64::new(0)),
                paths: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl ICloudLocalCopyMetadataDriver for FixedICloudMetadataDriver {
        fn read_metadata(
            &self,
            request: Arc<ICloudLocalCopyProbeRequest>,
        ) -> ICloudLocalCopyMetadataResult {
            self.calls.fetch_add(1, Ordering::AcqRel);
            if self.consume_path {
                let path = request.take_path_bytes().unwrap();
                self.paths.lock().unwrap().push(path);
            }
            assert!(!self.panic, "simulated Foundation metadata panic");
            self.result.clone()
        }
    }

    fn icloud_probe_fixture(
        temp: &TempDir,
        engine: &DuxEngine,
        fixture: &str,
    ) -> (Arc<SnapshotReviewSession>, u64, PathBuf) {
        let root = temp.path().join(fixture);
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("icloud-current.bin");
        std::fs::write(&file, vec![0x5a; 8_192]).unwrap();
        let scan = engine.start_scan(scan_request(&root)).unwrap();
        let terminal = wait_for_scan(&scan.task);
        assert_eq!(terminal.phase, TaskPhase::Succeeded);
        let review = engine
            .acquire_explorer_snapshot_review(terminal.result.unwrap().scan_id)
            .unwrap();
        let root_node = review.root_node().unwrap();
        let node_id = review
            .child_nodes(root_node.id, SnapshotNodeSort::NameAscending, 0, 10)
            .unwrap()
            .nodes
            .into_iter()
            .find(|node| node.name.display == "icloud-current.bin")
            .unwrap()
            .id;
        (review, node_id, file)
    }

    fn icloud_selection(node_id: u64) -> ICloudLocalCopyProbeSelection {
        ICloudLocalCopyProbeSelection {
            record_version: FFI_RECORD_VERSION,
            node_id,
        }
    }

    #[test]
    fn icloud_probe_request_is_core_issued_path_only_and_one_shot() {
        let request = ICloudLocalCopyProbeRequest::for_test(
            FFI_RECORD_VERSION,
            SnapshotNameEncoding::UnixBytes,
            b"/private/tmp/dux-icloud-reviewed-file".to_vec(),
        );
        assert_eq!(request.record_version().unwrap(), FFI_RECORD_VERSION);
        assert_eq!(
            request.path_encoding().unwrap(),
            SnapshotNameEncoding::UnixBytes
        );
        assert_eq!(
            request.take_path_bytes().unwrap(),
            b"/private/tmp/dux-icloud-reviewed-file"
        );
        assert_eq!(
            request.take_path_bytes().unwrap_err(),
            ICloudLocalCopyProbeRequestError::Consumed
        );
        assert_eq!(
            request.record_version().unwrap_err(),
            ICloudLocalCopyProbeRequestError::Consumed
        );

        let empty = ICloudLocalCopyProbeRequest::for_test(
            FFI_RECORD_VERSION,
            SnapshotNameEncoding::UnixBytes,
            Vec::new(),
        );
        assert_eq!(
            empty.take_path_bytes().unwrap_err(),
            ICloudLocalCopyProbeRequestError::InvalidPath
        );
        assert_eq!(
            empty.take_path_bytes().unwrap_err(),
            ICloudLocalCopyProbeRequestError::Consumed
        );
    }

    #[test]
    fn icloud_probe_projects_every_typed_state_blocker_and_error() {
        for (ffi, core) in [
            (ICloudBooleanState::True, CoreCloudBooleanState::True),
            (ICloudBooleanState::False, CoreCloudBooleanState::False),
            (ICloudBooleanState::Unknown, CoreCloudBooleanState::Unknown),
        ] {
            assert_eq!(core_icloud_boolean(ffi), core);
            assert_eq!(project_icloud_boolean(core), ffi);
        }
        for (ffi, core) in [
            (ICloudErrorState::Absent, CoreCloudErrorState::Absent),
            (ICloudErrorState::Present, CoreCloudErrorState::Present),
            (ICloudErrorState::Unknown, CoreCloudErrorState::Unknown),
        ] {
            assert_eq!(core_icloud_error(ffi), core);
            assert_eq!(project_icloud_error(core), ffi);
        }
        for (ffi, core) in [
            (
                ICloudLocalCopyState::Current,
                CoreCloudLocalCopyState::Current,
            ),
            (ICloudLocalCopyState::Stale, CoreCloudLocalCopyState::Stale),
            (
                ICloudLocalCopyState::NotDownloaded,
                CoreCloudLocalCopyState::NotDownloaded,
            ),
            (
                ICloudLocalCopyState::Unknown,
                CoreCloudLocalCopyState::Unknown,
            ),
        ] {
            assert_eq!(core_icloud_local_copy_state(ffi), core);
            assert_eq!(project_icloud_local_copy_state(core), ffi);
        }
        for (ffi, core) in [
            (
                ICloudIdentityFactState::Stable,
                CoreCloudIdentityFactState::Stable,
            ),
            (
                ICloudIdentityFactState::Unavailable,
                CoreCloudIdentityFactState::Unavailable,
            ),
            (
                ICloudIdentityFactState::ChangedDuringRead,
                CoreCloudIdentityFactState::ChangedDuringRead,
            ),
            (
                ICloudIdentityFactState::Unsupported,
                CoreCloudIdentityFactState::Unsupported,
            ),
        ] {
            assert_eq!(core_icloud_identity_state(ffi), core);
            assert_eq!(project_icloud_identity_state(core), ffi);
        }
        for (core, ffi) in [
            (
                CoreCloudEvictionBlockReason::UnsupportedItemKind,
                ICloudLocalCopyBlockReason::UnsupportedItemKind,
            ),
            (
                CoreCloudEvictionBlockReason::UbiquityUnknown,
                ICloudLocalCopyBlockReason::UbiquityUnknown,
            ),
            (
                CoreCloudEvictionBlockReason::NotUbiquitous,
                ICloudLocalCopyBlockReason::NotUbiquitous,
            ),
            (
                CoreCloudEvictionBlockReason::UploadStateUnknown,
                ICloudLocalCopyBlockReason::UploadStateUnknown,
            ),
            (
                CoreCloudEvictionBlockReason::UploadIncomplete,
                ICloudLocalCopyBlockReason::UploadIncomplete,
            ),
            (
                CoreCloudEvictionBlockReason::UploadActivityUnknown,
                ICloudLocalCopyBlockReason::UploadActivityUnknown,
            ),
            (
                CoreCloudEvictionBlockReason::UploadInProgress,
                ICloudLocalCopyBlockReason::UploadInProgress,
            ),
            (
                CoreCloudEvictionBlockReason::UploadErrorUnknown,
                ICloudLocalCopyBlockReason::UploadErrorUnknown,
            ),
            (
                CoreCloudEvictionBlockReason::UploadErrorPresent,
                ICloudLocalCopyBlockReason::UploadErrorPresent,
            ),
            (
                CoreCloudEvictionBlockReason::ConflictStateUnknown,
                ICloudLocalCopyBlockReason::ConflictStateUnknown,
            ),
            (
                CoreCloudEvictionBlockReason::UnresolvedConflicts,
                ICloudLocalCopyBlockReason::UnresolvedConflicts,
            ),
            (
                CoreCloudEvictionBlockReason::LocalCopyStateUnknown,
                ICloudLocalCopyBlockReason::LocalCopyStateUnknown,
            ),
            (
                CoreCloudEvictionBlockReason::StaleLocalCopy,
                ICloudLocalCopyBlockReason::StaleLocalCopy,
            ),
            (
                CoreCloudEvictionBlockReason::NoLocalCopy,
                ICloudLocalCopyBlockReason::NoLocalCopy,
            ),
            (
                CoreCloudEvictionBlockReason::DownloadRequestUnknown,
                ICloudLocalCopyBlockReason::DownloadRequestUnknown,
            ),
            (
                CoreCloudEvictionBlockReason::DownloadRequested,
                ICloudLocalCopyBlockReason::DownloadRequested,
            ),
            (
                CoreCloudEvictionBlockReason::DownloadActivityUnknown,
                ICloudLocalCopyBlockReason::DownloadActivityUnknown,
            ),
            (
                CoreCloudEvictionBlockReason::DownloadInProgress,
                ICloudLocalCopyBlockReason::DownloadInProgress,
            ),
            (
                CoreCloudEvictionBlockReason::DownloadErrorUnknown,
                ICloudLocalCopyBlockReason::DownloadErrorUnknown,
            ),
            (
                CoreCloudEvictionBlockReason::DownloadErrorPresent,
                ICloudLocalCopyBlockReason::DownloadErrorPresent,
            ),
            (
                CoreCloudEvictionBlockReason::SyncExclusionUnknown,
                ICloudLocalCopyBlockReason::SyncExclusionUnknown,
            ),
            (
                CoreCloudEvictionBlockReason::ExcludedFromSync,
                ICloudLocalCopyBlockReason::ExcludedFromSync,
            ),
            (
                CoreCloudEvictionBlockReason::AllocationUnknown,
                ICloudLocalCopyBlockReason::AllocationUnknown,
            ),
            (
                CoreCloudEvictionBlockReason::NoLocalAllocation,
                ICloudLocalCopyBlockReason::NoLocalAllocation,
            ),
            (
                CoreCloudEvictionBlockReason::InvalidObservationTime,
                ICloudLocalCopyBlockReason::InvalidObservationTime,
            ),
        ] {
            assert_eq!(project_icloud_block_reason(core), ffi);
        }
        for (core, ffi) in [
            (
                CoreCloudEvictionIdentityBlockReason::AccountIdentityUnavailable,
                ICloudIdentityBlockReason::AccountIdentityUnavailable,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::AccountIdentityChanged,
                ICloudIdentityBlockReason::AccountIdentityChanged,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::AccountIdentityUnsupported,
                ICloudIdentityBlockReason::AccountIdentityUnsupported,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::ContainerIdentityUnavailable,
                ICloudIdentityBlockReason::ContainerIdentityUnavailable,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::ContainerIdentityChanged,
                ICloudIdentityBlockReason::ContainerIdentityChanged,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::ContainerIdentityUnsupported,
                ICloudIdentityBlockReason::ContainerIdentityUnsupported,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::ItemGenerationUnavailable,
                ICloudIdentityBlockReason::ItemGenerationUnavailable,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::ItemGenerationChanged,
                ICloudIdentityBlockReason::ItemGenerationChanged,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::ItemGenerationUnsupported,
                ICloudIdentityBlockReason::ItemGenerationUnsupported,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::FileVersionUnavailable,
                ICloudIdentityBlockReason::FileVersionUnavailable,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::FileVersionChanged,
                ICloudIdentityBlockReason::FileVersionChanged,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::FileVersionUnsupported,
                ICloudIdentityBlockReason::FileVersionUnsupported,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::SharedStateUnknown,
                ICloudIdentityBlockReason::SharedStateUnknown,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::SharedItem,
                ICloudIdentityBlockReason::SharedItem,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::SyncPausedStateUnknown,
                ICloudIdentityBlockReason::SyncPausedStateUnknown,
            ),
            (
                CoreCloudEvictionIdentityBlockReason::SyncPaused,
                ICloudIdentityBlockReason::SyncPaused,
            ),
        ] {
            assert_eq!(project_icloud_identity_block_reason(core), ffi);
        }
        for (core, ffi) in [
            (
                CoreCloudEvictionProbeError::Closed,
                ICloudLocalCopyProbeError::Closed,
            ),
            (
                CoreCloudEvictionProbeError::WrongReview,
                ICloudLocalCopyProbeError::WrongReview,
            ),
            (
                CoreCloudEvictionProbeError::InvalidTarget,
                ICloudLocalCopyProbeError::InvalidTarget,
            ),
            (
                CoreCloudEvictionProbeError::ChangedSinceSnapshot,
                ICloudLocalCopyProbeError::ChangedSinceSnapshot,
            ),
            (
                CoreCloudEvictionProbeError::ReviewUnavailable,
                ICloudLocalCopyProbeError::ReviewUnavailable,
            ),
            (
                CoreCloudEvictionProbeError::PlatformUnsupported,
                ICloudLocalCopyProbeError::PlatformUnsupported,
            ),
            (
                CoreCloudEvictionProbeError::PlatformFailed,
                ICloudLocalCopyProbeError::PlatformFailed,
            ),
        ] {
            assert_eq!(map_icloud_local_copy_probe_error(core), ffi);
        }
    }

    #[test]
    fn icloud_probe_projects_core_owned_identity_allocation_time_and_no_path() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let (review, node_id, file) = icloud_probe_fixture(&temp, &engine, "ffi-icloud-eligible");
        let driver = FixedICloudMetadataDriver::new(
            ICloudLocalCopyMetadataResult::Observed {
                facts: eligible_icloud_facts(),
            },
            true,
        );
        let paths = Arc::clone(&driver.paths);
        let before = system_time_ms(SystemTime::now()).unwrap();
        let assessment = engine
            .probe_explorer_icloud_local_copy(
                Arc::clone(&review),
                icloud_selection(node_id),
                Box::new(driver),
            )
            .unwrap();
        let after = system_time_ms(SystemTime::now()).unwrap();

        assert_eq!(assessment.record_version, FFI_RECORD_VERSION);
        assert_eq!(assessment.provider, ICloudLocalCopyProvider::ICloudDrive);
        assert_eq!(assessment.item_kind, ICloudLocalCopyItemKind::RegularFile);
        assert!(assessment.local_allocated_bytes > 0);
        assert!((before..=after).contains(&assessment.observed_at_unix_ms));
        assert!(assessment.is_eligible_observation);
        assert!(assessment.blockers.is_empty());
        assert_eq!(assessment.ubiquitous, ICloudBooleanState::True);
        assert_eq!(assessment.upload_error, ICloudErrorState::Absent);
        assert_eq!(assessment.local_copy_state, ICloudLocalCopyState::Current);
        assert_eq!(assessment.account_identity, ICloudIdentityFactState::Stable);
        assert_eq!(
            assessment.container_identity,
            ICloudIdentityFactState::Unsupported
        );
        assert_eq!(assessment.item_generation, ICloudIdentityFactState::Stable);
        assert_eq!(assessment.file_version, ICloudIdentityFactState::Stable);
        assert_eq!(assessment.shared, ICloudBooleanState::False);
        assert_eq!(assessment.sync_paused, ICloudBooleanState::False);
        assert!(!assessment.is_identity_ready);
        assert_eq!(
            assessment.identity_blockers,
            [ICloudIdentityBlockReason::ContainerIdentityUnsupported]
        );
        assert_eq!(
            paths.lock().unwrap().as_slice(),
            [std::fs::canonicalize(file)
                .unwrap()
                .as_os_str()
                .as_bytes()
                .to_vec()]
        );
        let projected_debug = format!("{assessment:?}");
        assert!(!projected_debug.contains("icloud-current.bin"));
        assert!(!projected_debug.contains(temp.path().to_string_lossy().as_ref()));
        assert!(engine.close());
    }

    #[test]
    fn icloud_probe_preserves_unknown_facts_and_fixed_blocker_order() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let (review, node_id, _) = icloud_probe_fixture(&temp, &engine, "ffi-icloud-unknown");
        let unknown = ICloudLocalCopyRawFacts {
            record_version: FFI_RECORD_VERSION,
            ubiquitous: ICloudBooleanState::Unknown,
            uploaded: ICloudBooleanState::Unknown,
            uploading: ICloudBooleanState::Unknown,
            upload_error: ICloudErrorState::Unknown,
            unresolved_conflicts: ICloudBooleanState::Unknown,
            local_copy_state: ICloudLocalCopyState::Unknown,
            download_requested: ICloudBooleanState::Unknown,
            downloading: ICloudBooleanState::Unknown,
            download_error: ICloudErrorState::Unknown,
            excluded_from_sync: ICloudBooleanState::Unknown,
            account_identity: ICloudIdentityFactState::Unavailable,
            container_identity: ICloudIdentityFactState::Unsupported,
            item_generation: ICloudIdentityFactState::Unavailable,
            file_version: ICloudIdentityFactState::Unavailable,
            shared: ICloudBooleanState::Unknown,
            sync_paused: ICloudBooleanState::Unknown,
        };
        let assessment = engine
            .probe_explorer_icloud_local_copy(
                review,
                icloud_selection(node_id),
                Box::new(FixedICloudMetadataDriver::new(
                    ICloudLocalCopyMetadataResult::Observed { facts: unknown },
                    true,
                )),
            )
            .unwrap();
        assert!(!assessment.is_eligible_observation);
        assert_eq!(
            assessment.blockers,
            vec![
                ICloudLocalCopyBlockReason::UbiquityUnknown,
                ICloudLocalCopyBlockReason::UploadStateUnknown,
                ICloudLocalCopyBlockReason::UploadActivityUnknown,
                ICloudLocalCopyBlockReason::UploadErrorUnknown,
                ICloudLocalCopyBlockReason::ConflictStateUnknown,
                ICloudLocalCopyBlockReason::LocalCopyStateUnknown,
                ICloudLocalCopyBlockReason::DownloadRequestUnknown,
                ICloudLocalCopyBlockReason::DownloadActivityUnknown,
                ICloudLocalCopyBlockReason::DownloadErrorUnknown,
                ICloudLocalCopyBlockReason::SyncExclusionUnknown,
            ]
        );
        assert!(assessment.local_allocated_bytes > 0);
        assert!(!assessment.is_identity_ready);
        assert_eq!(
            assessment.identity_blockers,
            [
                ICloudIdentityBlockReason::AccountIdentityUnavailable,
                ICloudIdentityBlockReason::ContainerIdentityUnsupported,
                ICloudIdentityBlockReason::ItemGenerationUnavailable,
                ICloudIdentityBlockReason::FileVersionUnavailable,
                ICloudIdentityBlockReason::SharedStateUnknown,
                ICloudIdentityBlockReason::SyncPausedStateUnknown,
            ]
        );
        assert!(engine.close());
    }

    #[test]
    fn icloud_identity_readiness_requires_every_independent_fact() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let (review, node_id, _) =
            icloud_probe_fixture(&temp, &engine, "ffi-icloud-identity-ready");
        let mut facts = eligible_icloud_facts();
        facts.container_identity = ICloudIdentityFactState::Stable;

        let assessment = engine
            .probe_explorer_icloud_local_copy(
                review,
                icloud_selection(node_id),
                Box::new(FixedICloudMetadataDriver::new(
                    ICloudLocalCopyMetadataResult::Observed { facts },
                    true,
                )),
            )
            .unwrap();

        assert!(assessment.is_eligible_observation);
        assert!(assessment.is_identity_ready);
        assert!(assessment.identity_blockers.is_empty());
        assert!(engine.close());
    }

    #[test]
    fn icloud_probe_rejects_malformed_versions_and_unconsumed_observations() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let (review, node_id, _) =
            icloud_probe_fixture(&temp, &engine, "ffi-icloud-invalid-version");

        let unused = FixedICloudMetadataDriver::new(ICloudLocalCopyMetadataResult::Failed, false);
        let unused_calls = Arc::clone(&unused.calls);
        assert_eq!(
            engine.probe_explorer_icloud_local_copy(
                Arc::clone(&review),
                ICloudLocalCopyProbeSelection {
                    record_version: FFI_RECORD_VERSION + 1,
                    node_id,
                },
                Box::new(unused),
            ),
            Err(ICloudLocalCopyProbeError::InvalidRecordVersion)
        );
        assert_eq!(unused_calls.load(Ordering::Acquire), 0);

        let mut malformed = eligible_icloud_facts();
        malformed.record_version = FFI_RECORD_VERSION + 1;
        assert_eq!(
            engine.probe_explorer_icloud_local_copy(
                Arc::clone(&review),
                icloud_selection(node_id),
                Box::new(FixedICloudMetadataDriver::new(
                    ICloudLocalCopyMetadataResult::Observed { facts: malformed },
                    true,
                )),
            ),
            Err(ICloudLocalCopyProbeError::PlatformFailed)
        );
        assert_eq!(
            engine.probe_explorer_icloud_local_copy(
                review,
                icloud_selection(node_id),
                Box::new(FixedICloudMetadataDriver::new(
                    ICloudLocalCopyMetadataResult::Observed {
                        facts: eligible_icloud_facts(),
                    },
                    false,
                )),
            ),
            Err(ICloudLocalCopyProbeError::PlatformFailed)
        );
        assert!(engine.close());
    }

    #[test]
    fn icloud_probe_maps_wrong_closed_failure_unsupported_and_callback_panic() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let (review, node_id, _) = icloud_probe_fixture(&temp, &engine, "ffi-icloud-errors");
        let (_other_temp, other_engine) = self::engine();
        assert_eq!(
            other_engine.probe_explorer_icloud_local_copy(
                Arc::clone(&review),
                icloud_selection(node_id),
                Box::new(FixedICloudMetadataDriver::new(
                    ICloudLocalCopyMetadataResult::Failed,
                    false,
                )),
            ),
            Err(ICloudLocalCopyProbeError::WrongReview)
        );
        assert_eq!(
            engine.probe_explorer_icloud_local_copy(
                Arc::clone(&review),
                icloud_selection(node_id),
                Box::new(FixedICloudMetadataDriver::new(
                    ICloudLocalCopyMetadataResult::Unsupported,
                    false,
                )),
            ),
            Err(ICloudLocalCopyProbeError::PlatformUnsupported)
        );
        assert_eq!(
            engine.probe_explorer_icloud_local_copy(
                Arc::clone(&review),
                icloud_selection(node_id),
                Box::new(FixedICloudMetadataDriver::new(
                    ICloudLocalCopyMetadataResult::Failed,
                    false,
                )),
            ),
            Err(ICloudLocalCopyProbeError::PlatformFailed)
        );
        assert_eq!(
            engine.probe_explorer_icloud_local_copy(
                Arc::clone(&review),
                icloud_selection(node_id),
                Box::new(FixedICloudMetadataDriver::panicking()),
            ),
            Err(ICloudLocalCopyProbeError::PlatformFailed)
        );
        assert!(engine.close());
        assert_eq!(
            engine.probe_explorer_icloud_local_copy(
                review,
                icloud_selection(node_id),
                Box::new(FixedICloudMetadataDriver::new(
                    ICloudLocalCopyMetadataResult::Failed,
                    false,
                )),
            ),
            Err(ICloudLocalCopyProbeError::Closed)
        );
        assert!(other_engine.close());
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

    fn targeted_project_scan_request(
        anchor_unix_ms: i64,
        ordinal: u16,
        revision: Option<u64>,
    ) -> TargetedProjectScanRequest {
        TargetedProjectScanRequest {
            record_version: FFI_RECORD_VERSION,
            stable_volume_id: "01234567-89AB-CDEF-0123-456789ABCDEF".into(),
            capacity_anchor_unix_ms: anchor_unix_ms,
            selected_root_ordinal: ordinal,
            expected_configured_roots_revision: revision,
            expected_root_catalog_digest_sha256: None,
        }
    }

    fn observe_critical_capacity(engine: &DuxEngine) -> i64 {
        let gib = 1_024 * 1_024 * 1_024;
        let anchor = system_time_ms(SystemTime::now()).unwrap();
        let status = engine
            .observe_startup_volume(startup_observation(anchor, Some(5 * gib), Some(5 * gib)))
            .unwrap();
        assert_eq!(status.pressure, VolumePressure::Critical);
        anchor
    }

    #[test]
    fn targeted_project_scan_is_path_free_bounded_and_reuses_current_evidence() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("source.txt"), b"read-only discovery").unwrap();
        let encoded_root = configured_project_root_path(&root).unwrap();
        let configured = engine
            .set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION,
                roots: vec![encoded_root.clone()],
            })
            .unwrap()
            .roots;
        let anchor = observe_critical_capacity(&engine);

        let started = engine
            .start_targeted_project_scan(targeted_project_scan_request(
                anchor,
                0,
                Some(configured.revision),
            ))
            .unwrap();
        assert_eq!(started.record_version, FFI_RECORD_VERSION);
        assert_eq!(started.configured_roots_revision, configured.revision);
        assert_eq!(started.root_count, 1);
        let root_catalog = started.root_catalog.clone();
        assert_eq!(started.disposition, TargetedProjectScanDisposition::Started);
        assert_eq!(
            started.selection,
            Some(TargetedProjectScanSelection {
                record_version: FFI_RECORD_VERSION,
                ordinal: 0,
                kind: TargetedReclaimRootKind::ConfiguredProject,
                root: encoded_root.clone(),
                max_nodes: u32::try_from(dux_core::MAX_TARGETED_PROJECT_SCAN_NODES).unwrap(),
            })
        );
        let pressure = started.pressure.clone().unwrap();
        assert_eq!(
            pressure.stable_volume_id,
            "volume:macos:01234567-89ab-cdef-0123-456789abcdef"
        );
        assert_eq!(pressure.capacity_anchor_unix_ms, anchor);
        assert_eq!(pressure.pressure, TargetedProjectScanPressure::Critical);
        assert!(
            pressure.pressure_started_at_unix_ms <= pressure.current_episode_started_at_unix_ms
        );
        assert!(pressure.current_episode_started_at_unix_ms <= anchor);
        assert!(started.root_unavailable_reason.is_none());
        assert!(started.current_result.is_none());
        assert!(started.existing_task_observed_phase.is_none());
        let task = started.task.as_ref().unwrap();
        let terminal = wait_for_scan(task);
        assert_eq!(terminal.phase, TaskPhase::Succeeded);
        let terminal_result = terminal.result.unwrap();

        let mut current_request =
            targeted_project_scan_request(anchor, 0, Some(configured.revision));
        current_request.expected_root_catalog_digest_sha256 =
            Some(root_catalog.digest_sha256.clone());
        let current = engine.start_targeted_project_scan(current_request).unwrap();
        assert_eq!(current.disposition, TargetedProjectScanDisposition::Current);
        assert_eq!(current.selection.unwrap().root, encoded_root);
        assert_eq!(current.pressure.as_ref(), Some(&pressure));
        assert!(current.task.is_none());
        assert!(current.root_unavailable_reason.is_none());
        assert!(current.existing_task_observed_phase.is_none());
        let current_result = current.current_result.unwrap();
        assert_eq!(current_result.scan_id, terminal_result.scan_id);
        assert!(current_result.scan_id.starts_with("scan:targeted:"));
        assert_eq!(current_result.status, ScanTerminalStatus::Succeeded);
        assert!(current_result.snapshot_available);
        assert!(matches!(
            current_result.candidate_evaluation.status,
            ScanCandidateEvaluationStatus::Succeeded | ScanCandidateEvaluationStatus::Failed
        ));

        let checkpoint = engine
            .validate_targeted_project_scan_context(TargetedProjectScanCheckpointRequest {
                record_version: FFI_RECORD_VERSION,
                expected_pressure: pressure.clone(),
                expected_root_catalog: root_catalog.clone(),
            })
            .unwrap();
        assert_eq!(checkpoint.record_version, FFI_RECORD_VERSION);
        assert_eq!(checkpoint.configured_roots_revision, configured.revision);
        assert_eq!(checkpoint.root_count, 1);
        assert_eq!(checkpoint.root_catalog, root_catalog);
        assert_eq!(checkpoint.pressure, pressure);

        let ordering = engine
            .finalize_emergency_recovery(EmergencyRecoveryRequest {
                record_version: FFI_RECORD_VERSION,
                expected_pressure: pressure.clone(),
                expected_root_catalog: root_catalog.clone(),
            })
            .unwrap();
        assert_eq!(ordering.record_version, FFI_RECORD_VERSION);
        assert_eq!(ordering.policy_revision, EMERGENCY_RECOVERY_POLICY_REVISION);
        assert_eq!(ordering.pressure, pressure);
        assert_eq!(ordering.root_catalog, root_catalog);
        assert_eq!(ordering.observed_root_count, 1);
        assert_eq!(ordering.candidate_evaluated_root_count, 1);
        assert_eq!(ordering.unavailable_root_count, 0);
        assert_eq!(ordering.groups.len(), 1);
        assert_eq!(
            ordering.groups[0].lane,
            EmergencyRecoveryLane::GuidedExploration
        );
        assert_eq!(ordering.groups[0].rank, 0);
        assert_eq!(ordering.groups[0].sources.len(), 1);
        assert_eq!(
            ordering.groups[0].sources[0].scan_id,
            terminal_result.scan_id
        );
        assert!(ordering.groups[0].rule_id.is_none());
        assert!(ordering.groups[0].category.is_none());
        assert!(ordering.groups[0].sources[0].candidate_count.is_none());
        assert!(
            ordering.groups[0].sources[0]
                .permission_issue_count
                .is_none()
        );
        engine.reset_configured_project_roots().unwrap();
        assert_eq!(
            engine.finalize_emergency_recovery(EmergencyRecoveryRequest {
                record_version: FFI_RECORD_VERSION,
                expected_pressure: pressure,
                expected_root_catalog: root_catalog,
            }),
            Err(EmergencyRecoveryError::RegistryChanged)
        );
    }

    #[test]
    fn emergency_recovery_rejects_warning_pressure() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("warning-project");
        std::fs::create_dir_all(&root).unwrap();
        let configured = engine
            .set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION,
                roots: vec![configured_project_root_path(&root).unwrap()],
            })
            .unwrap()
            .roots;
        let gib = 1_024 * 1_024 * 1_024;
        let anchor = system_time_ms(SystemTime::now()).unwrap();
        let status = engine
            .observe_startup_volume(startup_observation(anchor, Some(20 * gib), Some(20 * gib)))
            .unwrap();
        assert_eq!(status.pressure, VolumePressure::Warning);
        let admission = engine
            .start_targeted_project_scan(targeted_project_scan_request(
                anchor,
                0,
                Some(configured.revision),
            ))
            .unwrap();
        assert_eq!(
            engine.finalize_emergency_recovery(EmergencyRecoveryRequest {
                record_version: FFI_RECORD_VERSION,
                expected_pressure: admission.pressure.unwrap(),
                expected_root_catalog: admission.root_catalog,
            }),
            Err(EmergencyRecoveryError::NotCritical)
        );
    }

    #[test]
    fn targeted_project_scan_preserves_empty_no_pressure_and_root_failure_shapes() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();

        let (_empty_temp, empty_engine) = engine();
        let empty_anchor = observe_critical_capacity(&empty_engine);
        let empty = empty_engine
            .start_targeted_project_scan(targeted_project_scan_request(empty_anchor, 0, None))
            .unwrap();
        assert_eq!(
            empty.disposition,
            TargetedProjectScanDisposition::EmptyRegistry
        );
        assert_eq!(empty.root_count, 0);
        assert!(empty.selection.is_none());
        assert!(empty.pressure.is_some());
        assert!(empty.root_unavailable_reason.is_none());
        assert!(empty.current_result.is_none());
        assert!(empty.task.is_none());
        assert!(empty.existing_task_observed_phase.is_none());

        let (healthy_temp, healthy_engine) = engine();
        let healthy_root = healthy_temp.path().join("healthy-project");
        std::fs::create_dir_all(&healthy_root).unwrap();
        let healthy_settings = healthy_engine
            .set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION,
                roots: vec![configured_project_root_path(&healthy_root).unwrap()],
            })
            .unwrap()
            .roots;
        let healthy_anchor = system_time_ms(SystemTime::now()).unwrap();
        let gib = 1_024 * 1_024 * 1_024;
        let healthy_status = healthy_engine
            .observe_startup_volume(startup_observation(
                healthy_anchor,
                Some(200 * gib),
                Some(200 * gib),
            ))
            .unwrap();
        assert_eq!(healthy_status.pressure, VolumePressure::Healthy);
        let no_pressure = healthy_engine
            .start_targeted_project_scan(targeted_project_scan_request(
                healthy_anchor,
                0,
                Some(healthy_settings.revision),
            ))
            .unwrap();
        assert_eq!(
            no_pressure.disposition,
            TargetedProjectScanDisposition::NoPressure
        );
        assert!(no_pressure.selection.is_some());
        assert!(no_pressure.pressure.is_none());
        assert!(no_pressure.root_unavailable_reason.is_none());
        assert!(no_pressure.current_result.is_none());
        assert!(no_pressure.task.is_none());
        assert!(no_pressure.existing_task_observed_phase.is_none());

        let (missing_temp, missing_engine) = engine();
        let missing_root = missing_temp.path().join("missing-project");
        let missing_settings = missing_engine
            .set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION,
                roots: vec![configured_project_root_path(&missing_root).unwrap()],
            })
            .unwrap()
            .roots;
        let missing_anchor = observe_critical_capacity(&missing_engine);
        let unavailable = missing_engine
            .start_targeted_project_scan(targeted_project_scan_request(
                missing_anchor,
                0,
                Some(missing_settings.revision),
            ))
            .unwrap();
        assert_eq!(
            unavailable.disposition,
            TargetedProjectScanDisposition::RootUnavailable
        );
        assert_eq!(
            unavailable.root_unavailable_reason,
            Some(TargetedProjectScanRootUnavailableReason::Missing)
        );
        assert!(unavailable.selection.is_some());
        assert!(unavailable.pressure.is_some());
        assert!(unavailable.current_result.is_none());
        assert!(unavailable.task.is_none());
        assert!(unavailable.existing_task_observed_phase.is_none());
        let incomplete = missing_engine
            .finalize_emergency_recovery(EmergencyRecoveryRequest {
                record_version: FFI_RECORD_VERSION,
                expected_pressure: unavailable.pressure.unwrap(),
                expected_root_catalog: unavailable.root_catalog,
            })
            .unwrap();
        assert_eq!(incomplete.observed_root_count, 0);
        assert_eq!(incomplete.candidate_evaluated_root_count, 0);
        assert_eq!(incomplete.unavailable_root_count, 1);
        assert_eq!(incomplete.groups.len(), 1);
        assert_eq!(
            incomplete.groups[0].lane,
            EmergencyRecoveryLane::PermissionGap
        );
        assert_eq!(incomplete.groups[0].unavailable_root_count, 1);
        assert!(incomplete.groups[0].sources.is_empty());
    }

    #[test]
    fn targeted_project_scan_rejects_malformed_requests_and_changed_registry() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let configured = engine
            .set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION,
                roots: vec![configured_project_root_path(&root).unwrap()],
            })
            .unwrap()
            .roots;
        let anchor = observe_critical_capacity(&engine);

        let mut request = targeted_project_scan_request(anchor, 0, Some(configured.revision));
        request.record_version += 1;
        assert!(matches!(
            engine.start_targeted_project_scan(request),
            Err(TargetedProjectScanError::InvalidRecordVersion)
        ));
        let mut request = targeted_project_scan_request(anchor, 0, Some(configured.revision));
        request.stable_volume_id = "not-a-volume".into();
        assert!(matches!(
            engine.start_targeted_project_scan(request),
            Err(TargetedProjectScanError::InvalidVolumeIdentity)
        ));
        let mut request = targeted_project_scan_request(anchor, 0, Some(configured.revision));
        request.capacity_anchor_unix_ms = -1;
        assert!(matches!(
            engine.start_targeted_project_scan(request),
            Err(TargetedProjectScanError::InvalidAnchor)
        ));
        assert!(matches!(
            engine.start_targeted_project_scan(targeted_project_scan_request(
                anchor,
                1,
                Some(configured.revision),
            )),
            Err(TargetedProjectScanError::InvalidCatalog)
        ));
        assert!(matches!(
            engine.start_targeted_project_scan(targeted_project_scan_request(
                anchor,
                0,
                Some(configured.revision + 1),
            )),
            Err(TargetedProjectScanError::RegistryChanged)
        ));

        let admission = engine
            .start_targeted_project_scan(targeted_project_scan_request(
                anchor,
                0,
                Some(configured.revision),
            ))
            .unwrap();
        let pressure = admission.pressure.unwrap();
        let root_catalog = admission.root_catalog;
        let mut malformed_recovery = EmergencyRecoveryRequest {
            record_version: FFI_RECORD_VERSION + 1,
            expected_pressure: pressure.clone(),
            expected_root_catalog: root_catalog.clone(),
        };
        assert_eq!(
            engine.finalize_emergency_recovery(malformed_recovery.clone()),
            Err(EmergencyRecoveryError::InvalidRecordVersion)
        );
        malformed_recovery.record_version = FFI_RECORD_VERSION;
        malformed_recovery.expected_root_catalog.digest_sha256.pop();
        assert_eq!(
            engine.finalize_emergency_recovery(malformed_recovery),
            Err(EmergencyRecoveryError::InvalidCatalog)
        );
        let mut invalid_ordinal =
            targeted_project_scan_request(anchor, 1, Some(configured.revision));
        invalid_ordinal.expected_root_catalog_digest_sha256 =
            Some(root_catalog.digest_sha256.clone());
        assert!(matches!(
            engine.start_targeted_project_scan(invalid_ordinal),
            Err(TargetedProjectScanError::InvalidOrdinal)
        ));
        let mut malformed_pressure = pressure.clone();
        malformed_pressure.record_version += 1;
        assert!(matches!(
            engine.validate_targeted_project_scan_context(TargetedProjectScanCheckpointRequest {
                record_version: FFI_RECORD_VERSION,
                expected_pressure: malformed_pressure,
                expected_root_catalog: root_catalog.clone(),
            }),
            Err(TargetedProjectScanError::InvalidRecordVersion)
        ));
        let mut malformed_pressure = pressure;
        malformed_pressure.pressure_started_at_unix_ms =
            malformed_pressure.capacity_anchor_unix_ms + 1;
        assert!(matches!(
            engine.validate_targeted_project_scan_context(TargetedProjectScanCheckpointRequest {
                record_version: FFI_RECORD_VERSION,
                expected_pressure: malformed_pressure,
                expected_root_catalog: root_catalog,
            }),
            Err(TargetedProjectScanError::InvalidAnchor)
        ));
    }

    #[test]
    fn emergency_recovery_projection_is_bounded_path_free_and_shape_checked() {
        use dux_core::{RuleId, RuleRef, RuleRevision};

        let pressure = CoreTargetedProjectScanPressureContext {
            volume_id: VolumeId::new("volume:emergency-projection").unwrap(),
            capacity_anchor: UNIX_EPOCH + Duration::from_secs(10),
            pressure: CoreTargetedProjectScanPressure::Critical,
            current_episode_started_at: UNIX_EPOCH + Duration::from_secs(5),
            pressure_started_at: UNIX_EPOCH + Duration::from_secs(1),
            policy_revision: 3,
        };
        let catalog = CoreTargetedReclaimRootCatalogStamp {
            known_roots_policy_revision: 1,
            configured_roots_revision: 7,
            known_user_library_caches_included: false,
            root_count: 1,
            digest_sha256: [9; 32],
        };
        let source = |candidate_count, blocked_candidate_count, permission_issue_count| {
            CoreEmergencyRecoverySource {
                root_ordinal: 0,
                scan_id: ScanId::new("scan:emergency:projection").unwrap(),
                observed_at: UNIX_EPOCH + Duration::from_secs(9),
                candidate_count,
                blocked_candidate_count,
                permission_issue_count,
            }
        };
        let ordering = CoreEmergencyRecoveryOrdering {
            policy_revision: EMERGENCY_RECOVERY_POLICY_REVISION,
            pressure: pressure.clone(),
            root_catalog: catalog.clone(),
            observed_root_count: 1,
            candidate_evaluated_root_count: 1,
            unavailable_root_count: 0,
            groups: vec![
                CoreEmergencyRecoveryGroup {
                    rank: 0,
                    lane: CoreEmergencyRecoveryLane::StaleSafeRegenerable,
                    rule: Some(RuleRef::new(
                        RuleId::new("developer.rust.target").unwrap(),
                        RuleRevision::new(3).unwrap(),
                    )),
                    category: Some(CoreCandidateCategory::DeveloperArtifact),
                    unavailable_root_count: 0,
                    sources: vec![source(Some(2), Some(1), None)],
                },
                CoreEmergencyRecoveryGroup {
                    rank: 1,
                    lane: CoreEmergencyRecoveryLane::GuidedExploration,
                    rule: None,
                    category: None,
                    unavailable_root_count: 0,
                    sources: vec![source(None, None, None)],
                },
                CoreEmergencyRecoveryGroup {
                    rank: 2,
                    lane: CoreEmergencyRecoveryLane::PermissionGap,
                    rule: None,
                    category: None,
                    unavailable_root_count: 0,
                    sources: vec![source(None, None, Some(4))],
                },
            ],
        };
        let projected = emergency_recovery_ordering(ordering.clone(), &pressure, &catalog).unwrap();
        assert_eq!(projected.observed_root_count, 1);
        assert_eq!(projected.candidate_evaluated_root_count, 1);
        assert_eq!(projected.unavailable_root_count, 0);
        assert_eq!(projected.groups.len(), 3);
        assert_eq!(
            projected
                .groups
                .iter()
                .map(|group| group.lane)
                .collect::<Vec<_>>(),
            vec![
                EmergencyRecoveryLane::StaleSafeRegenerable,
                EmergencyRecoveryLane::GuidedExploration,
                EmergencyRecoveryLane::PermissionGap,
            ]
        );
        assert_eq!(
            projected.groups[0].rule_id.as_deref(),
            Some("developer.rust.target")
        );
        assert_eq!(projected.groups[0].rule_revision, Some(3));
        assert_eq!(projected.groups[0].sources[0].candidate_count, Some(2));
        assert_eq!(
            projected.groups[0].sources[0].blocked_candidate_count,
            Some(1)
        );
        assert_eq!(
            projected.groups[2].sources[0].permission_issue_count,
            Some(4)
        );
        assert_eq!(projected.groups[2].unavailable_root_count, 0);

        let mut invalid_shape = ordering.clone();
        invalid_shape.groups[1].sources[0].candidate_count = Some(1);
        assert_eq!(
            emergency_recovery_ordering(invalid_shape, &pressure, &catalog),
            Err(EmergencyRecoveryError::InternalState)
        );
        let mut invalid_group_count = ordering.clone();
        invalid_group_count.groups[1].unavailable_root_count = 1;
        assert_eq!(
            emergency_recovery_ordering(invalid_group_count, &pressure, &catalog),
            Err(EmergencyRecoveryError::InternalState)
        );
        let mut invalid_rank = ordering.clone();
        invalid_rank.groups[1].rank = 7;
        assert_eq!(
            emergency_recovery_ordering(invalid_rank, &pressure, &catalog),
            Err(EmergencyRecoveryError::InternalState)
        );
        let mut unavailable_only = ordering.clone();
        unavailable_only.observed_root_count = 0;
        unavailable_only.candidate_evaluated_root_count = 0;
        unavailable_only.unavailable_root_count = 1;
        unavailable_only.groups = vec![CoreEmergencyRecoveryGroup {
            rank: 0,
            lane: CoreEmergencyRecoveryLane::PermissionGap,
            rule: None,
            category: None,
            unavailable_root_count: 1,
            sources: vec![],
        }];
        let projected_unavailable =
            emergency_recovery_ordering(unavailable_only.clone(), &pressure, &catalog).unwrap();
        assert_eq!(projected_unavailable.groups.len(), 1);
        assert_eq!(projected_unavailable.groups[0].unavailable_root_count, 1);
        assert!(projected_unavailable.groups[0].sources.is_empty());
        let mut mismatched_unavailable = unavailable_only;
        mismatched_unavailable.groups[0].unavailable_root_count = 0;
        assert_eq!(
            emergency_recovery_ordering(mismatched_unavailable, &pressure, &catalog),
            Err(EmergencyRecoveryError::InternalState)
        );
        let mut duplicate_semantic_group = ordering.clone();
        let mut duplicate = duplicate_semantic_group.groups[0].clone();
        duplicate.rank = 1;
        duplicate_semantic_group.groups.insert(1, duplicate);
        for (index, group) in duplicate_semantic_group.groups.iter_mut().enumerate() {
            group.rank = u16::try_from(index).unwrap();
        }
        assert_eq!(
            emergency_recovery_ordering(duplicate_semantic_group, &pressure, &catalog),
            Err(EmergencyRecoveryError::InternalState)
        );
        let mut invalid_counts = ordering.clone();
        invalid_counts.unavailable_root_count = 1;
        assert_eq!(
            emergency_recovery_ordering(invalid_counts, &pressure, &catalog),
            Err(EmergencyRecoveryError::InternalState)
        );
        let mut over_bound = ordering;
        over_bound.groups = (0..=dux_core::engine::MAX_EMERGENCY_RECOVERY_GROUPS)
            .map(|index| CoreEmergencyRecoveryGroup {
                rank: u16::try_from(index).unwrap(),
                lane: CoreEmergencyRecoveryLane::GuidedExploration,
                rule: None,
                category: None,
                unavailable_root_count: 0,
                sources: vec![source(None, None, None)],
            })
            .collect();
        assert_eq!(
            emergency_recovery_ordering(over_bound, &pressure, &catalog),
            Err(EmergencyRecoveryError::InternalState)
        );
    }

    #[test]
    fn targeted_task_bridge_refuses_user_scan_ownership() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("user-scan");
        std::fs::create_dir_all(&root).unwrap();
        let core = {
            let state = engine.state.lock().unwrap();
            let EngineState::Open(core) = &*state else {
                panic!("test engine must be open");
            };
            core.clone()
        };
        let user_task = core.start_scan(root).unwrap();
        assert!(matches!(
            targeted_project_scan_task(&core, user_task),
            Err(TargetedProjectScanError::InternalState)
        ));
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
        assert!(!initial.enabled);
        assert_eq!(initial.source, PermanentCleanupPolicySource::Default);
        assert_eq!(initial.revision, 0);
        assert_eq!(initial.updated_at_unix_ms, None);
        assert_eq!(
            permanent_cleanup_policy_status(CorePermanentCleanupPolicy {
                enabled: true,
                source: CorePermanentCleanupPolicySource::Default,
                revision: 0,
                updated_at: None,
            }),
            Err(PermanentCleanupPolicyError::InternalState)
        );

        let enabled = engine.set_permanent_cleanup_enabled(true).unwrap();
        assert!(enabled.changed);
        assert!(enabled.policy.enabled);
        assert_eq!(enabled.policy.source, PermanentCleanupPolicySource::Stored);
        assert_eq!(enabled.policy.revision, 1);
        assert!(enabled.policy.updated_at_unix_ms.is_some());
        assert_eq!(
            engine.get_permanent_cleanup_policy().unwrap(),
            enabled.policy
        );

        let exact = engine.set_permanent_cleanup_enabled(true).unwrap();
        assert!(!exact.changed);
        assert_eq!(exact.policy, enabled.policy);

        let reset = engine.reset_permanent_cleanup().unwrap();
        assert!(reset.changed);
        assert!(!reset.policy.enabled);
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
    fn configured_project_roots_round_trip_is_lossless_ordered_and_discovery_only() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let first = temp.path().join("project-first");
        let second = temp.path().join("project-second");
        let input_path = |path: &Path| configured_project_root_path(path).unwrap();

        let initial = engine.get_configured_project_roots().unwrap();
        assert_eq!(initial.record_version, FFI_RECORD_VERSION);
        assert!(initial.roots.is_empty());
        assert_eq!(initial.source, ConfiguredProjectRootsSource::Default);
        assert_eq!(initial.revision, 0);
        assert_eq!(initial.updated_at_unix_ms, None);

        let stored = engine
            .set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION,
                roots: vec![input_path(&second), input_path(&first)],
            })
            .unwrap();
        assert!(stored.changed);
        assert_eq!(stored.record_version, FFI_RECORD_VERSION);
        assert_eq!(stored.roots.source, ConfiguredProjectRootsSource::Stored);
        assert_eq!(stored.roots.revision, 1);
        assert!(stored.roots.updated_at_unix_ms.is_some());
        assert_eq!(
            stored.roots.roots,
            vec![input_path(&first), input_path(&second)]
        );
        assert_eq!(engine.get_configured_project_roots().unwrap(), stored.roots);

        let exact = engine
            .set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION,
                roots: vec![input_path(&first), input_path(&second)],
            })
            .unwrap();
        assert!(!exact.changed);
        assert_eq!(exact.roots, stored.roots);

        let reset = engine.reset_configured_project_roots().unwrap();
        assert!(reset.changed);
        assert_eq!(reset.roots.source, ConfiguredProjectRootsSource::Default);
        assert_eq!(reset.roots.revision, 0);
        assert!(reset.roots.roots.is_empty());
        assert_eq!(reset.roots.updated_at_unix_ms, None);
        assert!(!engine.reset_configured_project_roots().unwrap().changed);

        #[cfg(unix)]
        {
            let mut bytes = temp.path().as_os_str().as_bytes().to_vec();
            bytes.extend_from_slice(b"/project-\xff");
            let non_utf8 = ConfiguredProjectRootPath {
                encoding: SnapshotNameEncoding::UnixBytes,
                encoded_bytes: bytes,
            };
            let stored = engine
                .set_configured_project_roots(ConfiguredProjectRootsInput {
                    record_version: FFI_RECORD_VERSION,
                    roots: vec![non_utf8.clone()],
                })
                .unwrap();
            assert_eq!(stored.roots.roots, vec![non_utf8]);
            assert_eq!(engine.get_configured_project_roots().unwrap(), stored.roots);
        }

        assert!(engine.close());
        assert_eq!(
            engine.get_configured_project_roots(),
            Err(ConfiguredProjectRootsError::Closed)
        );
        assert_eq!(
            engine.reset_configured_project_roots(),
            Err(ConfiguredProjectRootsError::Closed)
        );
    }

    #[test]
    #[cfg(unix)]
    fn configured_project_roots_reject_malformed_overlapping_and_unbounded_input() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let path = |bytes: &[u8]| ConfiguredProjectRootPath {
            encoding: SnapshotNameEncoding::UnixBytes,
            encoded_bytes: bytes.to_vec(),
        };

        assert_eq!(
            engine.set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION + 1,
                roots: Vec::new(),
            }),
            Err(ConfiguredProjectRootsError::InvalidRecordVersion)
        );
        for invalid in [
            b"".as_slice(),
            b"/",
            b"relative",
            b"/tmp/trailing/",
            b"/tmp//duplicate",
            b"/tmp/./current",
            b"/tmp/../parent",
            b"/tmp/control\npath",
        ] {
            assert_eq!(
                engine.set_configured_project_roots(ConfiguredProjectRootsInput {
                    record_version: FFI_RECORD_VERSION,
                    roots: vec![path(invalid)],
                }),
                Err(ConfiguredProjectRootsError::InvalidPath),
                "accepted {invalid:?}"
            );
        }
        assert_eq!(
            engine.set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION,
                roots: vec![path(b"/tmp/project"), path(b"/tmp/project/nested")],
            }),
            Err(ConfiguredProjectRootsError::OverlappingPaths)
        );
        assert_eq!(
            engine.set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION,
                roots: vec![path(b"/tmp/project"), path(b"/tmp/project")],
            }),
            Err(ConfiguredProjectRootsError::OverlappingPaths)
        );
        assert_eq!(
            engine.set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION,
                roots: (0..=MAX_CONFIGURED_PROJECT_ROOT_COUNT)
                    .map(|index| path(format!("/tmp/project-{index}").as_bytes()))
                    .collect(),
            }),
            Err(ConfiguredProjectRootsError::TooManyPaths)
        );
        let oversized = std::iter::once(b'/')
            .chain(std::iter::repeat_n(
                b'x',
                MAX_CONFIGURED_PROJECT_ROOT_PATH_BYTES,
            ))
            .collect::<Vec<_>>();
        assert_eq!(
            engine.set_configured_project_roots(ConfiguredProjectRootsInput {
                record_version: FFI_RECORD_VERSION,
                roots: vec![path(&oversized)],
            }),
            Err(ConfiguredProjectRootsError::InvalidPath)
        );
    }

    #[test]
    fn configured_project_roots_projection_rejects_malformed_core_shapes() {
        let malformed_default = CoreConfiguredProjectRoots {
            roots: vec![PathBuf::from("/tmp/project")],
            source: CoreConfiguredProjectRootsSource::Default,
            revision: 0,
            updated_at: None,
        };
        assert_eq!(
            configured_project_roots_status(malformed_default),
            Err(ConfiguredProjectRootsError::InternalState)
        );

        let malformed_revision = CoreConfiguredProjectRoots {
            roots: Vec::new(),
            source: CoreConfiguredProjectRootsSource::Stored,
            revision: 0,
            updated_at: None,
        };
        assert_eq!(
            configured_project_roots_status(malformed_revision),
            Err(ConfiguredProjectRootsError::InternalState)
        );

        let overlapping = CoreConfiguredProjectRoots {
            roots: vec![
                PathBuf::from("/tmp/project"),
                PathBuf::from("/tmp/project/nested"),
            ],
            source: CoreConfiguredProjectRootsSource::Stored,
            revision: 1,
            updated_at: Some(UNIX_EPOCH),
        };
        assert_eq!(
            configured_project_roots_status(overlapping),
            Err(ConfiguredProjectRootsError::InternalState)
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
            MaintenanceKind::CandidateEvaluationRecovery,
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
                    if kind == MaintenanceKind::CandidateEvaluationRecovery
                        && poll.phase == TaskPhase::Succeeded
                    {
                        let result = poll.result.as_ref().unwrap();
                        assert_eq!(
                            result.outcome,
                            MaintenanceOutcome::CandidateEvaluationRecoveryNone
                        );
                        assert_eq!(result.primary_count_before, 0);
                        assert_eq!(result.primary_count_after, 0);
                        assert_eq!(result.secondary_count_before, 0);
                        assert_eq!(result.secondary_count_after, 0);
                        assert_eq!(result.tertiary_count_before, 0);
                        assert_eq!(result.tertiary_count_after, 0);
                        assert_eq!(result.quaternary_count_before, 0);
                        assert_eq!(result.quaternary_count_after, 0);
                        assert_eq!(result.charged_bytes_before, 0);
                        assert_eq!(result.charged_bytes_after, 0);
                        assert_eq!(result.removed_bytes, 0);
                        assert_eq!(result.cap_bytes, 0);
                        assert!(!result.has_more);
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
    fn candidate_evaluation_recovery_poll_shape_is_strict_and_path_free() {
        let kind = MaintenanceKind::CandidateEvaluationRecovery;
        let mut recovered = empty_result(
            kind,
            UNIX_EPOCH,
            MaintenanceOutcome::CandidateEvaluationRecoveryRecovered,
            true,
        )
        .unwrap();
        recovered.primary_count_after = 7;
        assert_eq!(
            validate_maintenance_poll_shape(kind, TaskPhase::Succeeded, None, Some(&recovered)),
            Ok(())
        );

        let incompatible = empty_result(
            kind,
            UNIX_EPOCH,
            MaintenanceOutcome::CandidateEvaluationRecoveryIncompatible,
            true,
        )
        .unwrap();
        assert_eq!(
            validate_maintenance_poll_shape(kind, TaskPhase::Succeeded, None, Some(&incompatible)),
            Ok(())
        );

        let none = empty_result(
            kind,
            UNIX_EPOCH,
            MaintenanceOutcome::CandidateEvaluationRecoveryNone,
            false,
        )
        .unwrap();
        assert_eq!(
            validate_maintenance_poll_shape(kind, TaskPhase::Succeeded, None, Some(&none)),
            Ok(())
        );

        let mut wrong_kind = recovered.clone();
        wrong_kind.kind = MaintenanceKind::History;
        assert_eq!(
            validate_maintenance_poll_shape(kind, TaskPhase::Succeeded, None, Some(&wrong_kind)),
            Err(EngineError::InternalState)
        );

        let mut wrong_outcome = recovered.clone();
        wrong_outcome.outcome = MaintenanceOutcome::HistoryApplied;
        assert_eq!(
            validate_maintenance_poll_shape(kind, TaskPhase::Succeeded, None, Some(&wrong_outcome)),
            Err(EngineError::InternalState)
        );

        let mut leaked_field = recovered.clone();
        leaked_field.removed_bytes = 1;
        assert_eq!(
            validate_maintenance_poll_shape(kind, TaskPhase::Succeeded, None, Some(&leaked_field)),
            Err(EngineError::InternalState)
        );

        let mut invalid_none = none.clone();
        invalid_none.primary_count_after = 1;
        assert_eq!(
            validate_maintenance_poll_shape(kind, TaskPhase::Succeeded, None, Some(&invalid_none)),
            Err(EngineError::InternalState)
        );
        invalid_none.primary_count_after = 0;
        invalid_none.has_more = true;
        assert_eq!(
            validate_maintenance_poll_shape(kind, TaskPhase::Succeeded, None, Some(&invalid_none)),
            Err(EngineError::InternalState)
        );

        assert_eq!(
            validate_maintenance_poll_shape(kind, TaskPhase::Running, None, Some(&recovered)),
            Err(EngineError::InternalState)
        );
        assert_eq!(
            validate_maintenance_poll_shape(
                kind,
                TaskPhase::Failed,
                Some(MaintenanceFailure::CorruptData),
                None
            ),
            Ok(())
        );
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
        let icloud_source = review
            .icloud_observation_source(SnapshotICloudObservationSourceRequest {
                record_version: FFI_RECORD_VERSION,
                scope_node_id: 0,
                max_results: 2,
            })
            .unwrap();
        assert_eq!(icloud_source.record_version, FFI_RECORD_VERSION);
        assert_eq!(icloud_source.scan_id, scan_id);
        assert_eq!(icloud_source.scope_node_id, 0);
        assert_eq!(icloud_source.requested_max_results, 2);
        assert_eq!(icloud_source.visited_node_count, 2);
        assert_eq!(icloud_source.total_ranked_files, 2);
        assert!(!icloud_source.has_more);
        assert_eq!(icloud_source.targets.len(), 2);
        assert_eq!(icloud_source.targets[0].record_version, FFI_RECORD_VERSION);
        assert_eq!(icloud_source.targets[0].rank, 0);
        assert_eq!(icloud_source.targets[0].node.name.display, "larger");
        assert_eq!(icloud_source.targets[0].node.kind, SnapshotNodeKind::File);
        assert!(
            icloud_source.targets[0]
                .node
                .allocated_bytes
                .is_some_and(|bytes| bytes > 0)
        );
        assert!(icloud_source.targets[0].parent_context.is_empty());
        assert!(!icloud_source.targets[0].context_truncated);
        assert_eq!(icloud_source.targets[1].rank, 1);
        assert_eq!(
            review.icloud_observation_source(SnapshotICloudObservationSourceRequest {
                record_version: FFI_RECORD_VERSION + 1,
                scope_node_id: 0,
                max_results: 1,
            }),
            Err(EngineError::InvalidSnapshotICloudObservationSourceRequest)
        );
        assert_eq!(
            review.icloud_observation_source(SnapshotICloudObservationSourceRequest {
                record_version: FFI_RECORD_VERSION,
                scope_node_id: 0,
                max_results: 0,
            }),
            Err(EngineError::InvalidSnapshotICloudObservationSourceRequest)
        );
        assert_eq!(
            review.icloud_observation_source(SnapshotICloudObservationSourceRequest {
                record_version: FFI_RECORD_VERSION,
                scope_node_id: first.nodes[0].id,
                max_results: 1,
            }),
            Err(EngineError::SnapshotNodeNotDirectory)
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
        assert_eq!(
            review.icloud_observation_source(SnapshotICloudObservationSourceRequest {
                record_version: FFI_RECORD_VERSION,
                scope_node_id: 0,
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
    #[expect(
        clippy::disallowed_methods,
        reason = "test renames only a TempDir-owned fixture to exercise removed snapshot transport"
    )]
    fn snapshot_diff_transport_is_parent_scoped_versioned_and_path_free() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("diff-transport-root");
        std::fs::create_dir_all(root.join("removed")).unwrap();
        std::fs::write(root.join("removed/old"), [1_u8; 9]).unwrap();
        std::fs::write(root.join("grown"), [2_u8; 3]).unwrap();
        let baseline_scan_id = scan_snapshot(&engine, &root);
        std::thread::sleep(Duration::from_millis(2));
        // DUX-DESTRUCTIVE: allow=test-ffi-snapshot-diff-removed-rename -- move only one TempDir-owned fixture outside the scanned root to create a historical removed observation
        std::fs::rename(root.join("removed"), temp.path().join("removed-outside")).unwrap();
        std::fs::write(root.join("grown"), [3_u8; 17]).unwrap();
        std::fs::write(root.join("added"), [4_u8; 5]).unwrap();
        let current_scan_id = scan_snapshot(&engine, &root);

        let parent = engine
            .acquire_explorer_snapshot_review(current_scan_id.clone())
            .unwrap();
        let diff = engine
            .prepare_explorer_snapshot_diff_review(Arc::clone(&parent))
            .unwrap();
        let info = diff.info().unwrap();
        assert_eq!(info.record_version, FFI_RECORD_VERSION);
        assert_eq!(info.current_scan_id, current_scan_id);
        assert_eq!(info.baseline_scan_id, baseline_scan_id);
        assert!(info.current_completed_at_unix_ms >= info.baseline_completed_at_unix_ms);
        assert!(!info.released);

        let root_node = diff.root_node().unwrap();
        assert_eq!(root_node.record_version, FFI_RECORD_VERSION);
        assert_eq!(root_node.id, 0);
        assert_eq!(root_node.current_kind, Some(SnapshotNodeKind::Directory));
        assert_eq!(root_node.baseline_kind, Some(SnapshotNodeKind::Directory));
        assert!(root_node.can_descend);
        let page = diff
            .child_nodes(0, SnapshotDiffNodeSort::NameAscending, 0, 10)
            .unwrap();
        assert_eq!(page.record_version, FFI_RECORD_VERSION);
        assert_eq!(page.total_children, 3);
        assert!(page.total_growth_bytes > 0);
        assert!(page.total_shrinkage_bytes > 0);
        assert!(page.nodes.iter().any(|node| {
            node.name.display == "added"
                && node.change == SnapshotDiffChange::Added
                && node.logical_change.direction == SnapshotDiffDirection::Growth
                && node.current_kind == Some(SnapshotNodeKind::File)
                && node.baseline_kind.is_none()
        }));
        let removed = page
            .nodes
            .iter()
            .find(|node| node.name.display == "removed")
            .unwrap();
        assert_eq!(removed.change, SnapshotDiffChange::Removed);
        assert!(removed.current_kind.is_none());
        assert_eq!(removed.baseline_kind, Some(SnapshotNodeKind::Directory));
        assert!(removed.can_descend);
        let removed_page = diff
            .child_nodes(removed.id, SnapshotDiffNodeSort::MagnitudeDescending, 0, 10)
            .unwrap();
        assert_eq!(removed_page.nodes[0].name.display, "old");
        assert_eq!(removed_page.nodes[0].change, SnapshotDiffChange::Removed);
        let treemap = diff.treemap(0, 1).unwrap();
        assert_eq!(treemap.record_version, FFI_RECORD_VERSION);
        assert_eq!(treemap.cells.len(), 1);
        assert!(treemap.other_growth_child_count > 0 || treemap.other_shrinkage_child_count > 0);
        assert_eq!(
            diff.child_nodes(0, SnapshotDiffNodeSort::NameAscending, 0, 0),
            Err(EngineError::InvalidSnapshotNodePage)
        );
        assert_eq!(
            diff.treemap(0, 0),
            Err(EngineError::InvalidSnapshotTreemapBudget)
        );
        assert_eq!(diff.release().unwrap(), ReviewReleaseOutcome::Released);
        assert_eq!(
            diff.release().unwrap(),
            ReviewReleaseOutcome::AlreadyReleased
        );
        let released_info = diff.info().unwrap();
        assert!(released_info.released);
        assert_eq!(released_info.current_scan_id, current_scan_id);
        assert_eq!(released_info.baseline_scan_id, baseline_scan_id);
        assert_eq!(diff.root_node(), Err(EngineError::ReviewExpired));
        assert!(parent.root_node().is_ok());
        assert_eq!(parent.release().unwrap(), ReviewReleaseOutcome::Released);

        let invalidated_parent = engine
            .acquire_explorer_snapshot_review(current_scan_id)
            .unwrap();
        let invalidated_diff = engine
            .prepare_explorer_snapshot_diff_review(Arc::clone(&invalidated_parent))
            .unwrap();
        assert_eq!(
            invalidated_parent.release().unwrap(),
            ReviewReleaseOutcome::Released
        );
        assert_eq!(
            invalidated_diff.root_node(),
            Err(EngineError::WrongParentReview)
        );
        assert_eq!(
            invalidated_diff.release().unwrap(),
            ReviewReleaseOutcome::Released
        );
        assert!(engine.close());
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

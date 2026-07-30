pub mod cache;
pub mod cleanup;
pub mod domain;
pub mod engine;
pub mod error;
// Path evidence and textual protection policy are intentionally staged ahead
// of trusted mount, volume, home-resolution, rule-scope, and planner witnesses.
#[allow(dead_code)]
pub(crate) mod path_validation;
// Planner witnesses are staged as read-only, crate-private evidence. They do
// not construct cleanup plans, clear blockers, or cross the FFI boundary.
pub mod persistence;
#[allow(dead_code)]
mod planner;
pub mod projection;

/// Non-shipping, filesystem-free invariant oracle for the isolated fuzz crate.
#[cfg(fuzzing)]
#[doc(hidden)]
pub fn __fuzz_path_validation_and_policy(input: &[u8]) {
    path_validation::fuzz_support::exercise(input);
}
pub mod scanner;
pub mod size;
pub mod tree;

mod time;

pub use cache::{
    CACHE_MAGIC, CACHE_VERSION, CacheMetadata, CachedScanConfig, cache_path_for, get_mtime,
    is_cache_valid, load_cache, save_cache, spot_check_mtimes,
};
pub use cleanup::{
    TrashEffectRequest, TrashEffectRequestError, TrashEffectTargetKind, TrashPlatformResult,
    TrashSelectionError,
};
pub use domain::{
    ActivityGuard, AvailableCapacitySource, BlockReason, CLEANUP_PLAN_VALIDITY, Candidate,
    CandidateAction, CandidateCategory, CandidateId, CandidateValidationError, CleanupMode,
    CleanupPlan, CleanupPlanId, CleanupPlanItem, CleanupPlanValidationError, CloudBooleanState,
    CloudErrorState, CloudEvictionAssessment, CloudEvictionBlockReason,
    CloudEvictionDiscoveryEvidence, CloudEvictionIdentityBlockReason, CloudEvictionIdentityFacts,
    CloudEvictionItemKind, CloudEvictionObservation, CloudEvictionObservationInput,
    CloudEvictionPlatformFacts, CloudEvictionProvider, CloudIdentityFactState, CloudLocalCopyState,
    CoveragePermille, CoveragePermilleError, DiskPressure, DiskPressureConfig,
    DiskPressureConfigError, DiskPressureEvaluation, DiskPressureRecoveryMargin,
    DiskPressureThreshold, Evidence, EvidenceKind, LocalizedTextKey, OperationStatus, PlanWarning,
    ProvenanceUrl, Rule, RuleDefinition, RuleGuards, RuleId, RuleMatcher, RuleMatcherDefinition,
    RuleRef, RuleRevision, RuleScope, RuleValidationError, SafetyTier, ScanCoverage,
    ScanCoverageStatus, ScanId, ScanIssue, ScanIssueKind, StableIdError, VolumeCapacity,
    VolumeCapacityError, VolumeId, assess_cloud_eviction,
};
pub use engine::{
    CancelOutcome, CandidateEvaluationTaskFailureKind, CandidateEvaluationTaskStatus,
    CapacityHistoryDisposition, CleanupExclusionSource, CleanupExclusions, CleanupExclusionsError,
    CleanupExclusionsUpdate, CloseOutcome, CloudEvictionProbeError,
    CloudEvictionProbePlatformError, CloudEvictionProbeRequest, CloudEvictionProbeRequestError,
    ConfiguredProjectRoots, ConfiguredProjectRootsError, ConfiguredProjectRootsSource,
    ConfiguredProjectRootsUpdate, DiskPressurePolicy, DiskPressurePolicyError,
    DiskPressurePolicySource, DiskPressurePolicyUpdate, DurableRuleOutcome,
    DurableRuleOutcomeBatch, DurableRuleOutcomeState, DurableScanCounts, DurableScanCoverage,
    DurableScanCoverageDetailsPage, DurableScanIssue, DurableScanIssueKind,
    DurableScanIssueLocation, DurableScanStatus, DurableScanSummary, DurableStorageThiefGroup,
    DurableStorageThiefRanking, EngineConfig, EngineConfigError, EngineConfigField,
    EngineConfigReason, EngineHandle, EngineLifecycle, EngineOpenError, FormatSizeBatchResult,
    FormattedSizeEntry, MAX_PRESSURE_EPISODE_HISTORY_LIMIT, MAX_SCAN_COVERAGE_DETAIL_PAGE_LIMIT,
    MAX_SNAPSHOT_REVIEW_ICLOUD_OBSERVATION_TARGETS,
    MAX_SNAPSHOT_REVIEW_ICLOUD_OBSERVATION_VISITED_NODES, MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS,
    MAX_SNAPSHOT_REVIEW_NODE_PAGE_LIMIT, MAX_SNAPSHOT_REVIEW_PARENT_CONTEXT_COMPONENTS,
    MAX_SNAPSHOT_REVIEW_TREEMAP_CELLS, MAX_STORAGE_THIEF_RANKING_GROUPS,
    MAX_STORAGE_THIEF_RANKING_SOURCE_SESSIONS, MAX_TARGETED_CONFIGURED_PROJECT_ROOTS,
    MAX_TARGETED_PRESSURE_CHAIN_EPISODES, MAX_TARGETED_PROJECT_SCAN_NODES,
    MAX_TARGETED_PROJECT_SCAN_PASS_NODES, MAX_TARGETED_RECLAIM_ROOTS,
    MAX_TARGETED_USER_LIBRARY_CACHES_SCAN_NODES, MIN_TARGETED_CONFIGURED_PROJECT_SCAN_NODES,
    PermanentCleanupPolicy, PermanentCleanupPolicyError, PermanentCleanupPolicySource,
    PermanentCleanupPolicyUpdate, PermanentSafeCleanupFailureKind, PressureEpisode,
    PressureEpisodeHistory, PressureEpisodeHistoryError, PressureEpisodeLevel, RecentScanHistory,
    RuleOutcomeError, RuleOutcomeNotEligibleReason, RustTargetCleanupError,
    RustTargetCleanupResult, RustTargetCleanupStartFailure, RustTargetDryRunError,
    RustTargetDryRunFailureKind, RustTargetDryRunResult, RustTargetDryRunStartFailure,
    ScanCoverageDetailsError, ScanHistoryError, ScanRootErrorKind, ScanTaskCounts, ScanTaskOrigin,
    ScanTaskResult, ScanTaskStatus, SnapshotRetentionCap, SnapshotRetentionCapError,
    SnapshotRetentionCapSource, SnapshotRetentionCapUpdate, SnapshotReviewCategory,
    SnapshotReviewError, SnapshotReviewICloudObservationSource,
    SnapshotReviewICloudObservationTarget, SnapshotReviewLargeFile, SnapshotReviewLargeFilePage,
    SnapshotReviewLiveTarget, SnapshotReviewLiveTargetKind, SnapshotReviewLiveTargetPurpose,
    SnapshotReviewName, SnapshotReviewNameEncoding, SnapshotReviewNode, SnapshotReviewNodeKind,
    SnapshotReviewNodePage, SnapshotReviewNodeSort, SnapshotReviewReleaseOutcome,
    SnapshotReviewScanFlags, SnapshotReviewSession, SnapshotReviewTimestamp, SnapshotReviewTreemap,
    SnapshotReviewTreemapCell, StandaloneScanScopeLease, StartSubtreeScanError, StartTaskError,
    StorageThiefError, TARGETED_RECLAIM_ROOT_POLICY_REVISION, TargetedProjectScanAdmission,
    TargetedProjectScanCheckpoint, TargetedProjectScanCurrent, TargetedProjectScanDisposition,
    TargetedProjectScanError, TargetedProjectScanPressure, TargetedProjectScanPressureContext,
    TargetedProjectScanSelection, TargetedReclaimCatalogError, TargetedReclaimRootCatalogSlot,
    TargetedReclaimRootCatalogStamp, TargetedReclaimRootKind, TargetedReclaimScanBudget,
    TaskAccessError, TaskEvent, TaskEventBatch, TaskEventKind, TaskFailureKind, TaskPhase,
    TaskPriority, TaskSnapshot, VolumeCapacityObservation, VolumeCapacityStatus,
    VolumeCapacityStatusError, targeted_reclaim_root_catalog_layout, targeted_reclaim_scan_budget,
};
pub use error::{DuxError, Result};
pub use persistence::{
    DATABASE_SCHEMA_VERSION, DatabaseAccess, DatabaseOpenErrorKind, DatabaseStatus,
    SNAPSHOT_FORMAT_VERSION, SnapshotOpenErrorKind,
};
pub use projection::{
    ArtifactClassification, ArtifactKind, BuildArtifactEntry, LargeFileEntry, StaleThreshold,
    classify_artifact, project_build_artifacts_at, project_large_files,
    refresh_artifact_staleness_at,
};
pub use scanner::{
    CancellationToken, ScanConfig, ScanMessage, ScanOutcome, ScanProgress, ScanTermination, Scanner,
};
pub use size::{format_count, format_size, format_size_short, size_percentage};
pub use tree::{DiskTree, NodeId, NodeKind, TreeNode};

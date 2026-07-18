pub mod cache;
pub mod cleanup;
pub mod domain;
pub mod engine;
pub mod error;
// Path evidence and textual protection policy are intentionally staged ahead
// of trusted mount, volume, home-resolution, rule-scope, and planner witnesses.
#[allow(dead_code)]
pub(crate) mod path_validation;
pub mod persistence;
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
pub use domain::{
    ActivityGuard, AvailableCapacitySource, BlockReason, CLEANUP_PLAN_VALIDITY, Candidate,
    CandidateAction, CandidateCategory, CandidateId, CandidateValidationError, CleanupMode,
    CleanupPlan, CleanupPlanId, CleanupPlanItem, CleanupPlanValidationError, CoveragePermille,
    CoveragePermilleError, DiskPressure, DiskPressureConfig, DiskPressureConfigError,
    DiskPressureEvaluation, DiskPressureRecoveryMargin, DiskPressureThreshold, Evidence,
    EvidenceKind, LocalizedTextKey, OperationStatus, PlanWarning, ProvenanceUrl, Rule,
    RuleDefinition, RuleGuards, RuleId, RuleMatcher, RuleMatcherDefinition, RuleRef, RuleRevision,
    RuleScope, RuleValidationError, SafetyTier, ScanCoverage, ScanCoverageStatus, ScanId,
    ScanIssue, ScanIssueKind, StableIdError, VolumeCapacity, VolumeCapacityError, VolumeId,
};
pub use engine::{
    CancelOutcome, CandidateEvaluationTaskFailureKind, CandidateEvaluationTaskStatus,
    CapacityHistoryDisposition, CloseOutcome, DiskPressurePolicy, DiskPressurePolicyError,
    DiskPressurePolicySource, DiskPressurePolicyUpdate, DurableScanCounts, DurableScanCoverage,
    DurableScanCoverageDetailsPage, DurableScanIssue, DurableScanIssueKind,
    DurableScanIssueLocation, DurableScanStatus, DurableScanSummary, EngineConfig,
    EngineConfigError, EngineConfigField, EngineConfigReason, EngineHandle, EngineLifecycle,
    EngineOpenError, FormatSizeBatchResult, FormattedSizeEntry,
    MAX_SCAN_COVERAGE_DETAIL_PAGE_LIMIT, MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS,
    MAX_SNAPSHOT_REVIEW_NODE_PAGE_LIMIT, MAX_SNAPSHOT_REVIEW_PARENT_CONTEXT_COMPONENTS,
    MAX_SNAPSHOT_REVIEW_TREEMAP_CELLS, RecentScanHistory, ScanCoverageDetailsError,
    ScanHistoryError, ScanRootErrorKind, ScanTaskCounts, ScanTaskResult, ScanTaskStatus,
    SnapshotRetentionCap, SnapshotRetentionCapError, SnapshotRetentionCapSource,
    SnapshotRetentionCapUpdate, SnapshotReviewCategory, SnapshotReviewError,
    SnapshotReviewLargeFile, SnapshotReviewLargeFilePage, SnapshotReviewLiveTarget,
    SnapshotReviewLiveTargetKind, SnapshotReviewLiveTargetPurpose, SnapshotReviewName,
    SnapshotReviewNameEncoding, SnapshotReviewNode, SnapshotReviewNodeKind, SnapshotReviewNodePage,
    SnapshotReviewNodeSort, SnapshotReviewReleaseOutcome, SnapshotReviewScanFlags,
    SnapshotReviewSession, SnapshotReviewTimestamp, SnapshotReviewTreemap,
    SnapshotReviewTreemapCell, StartTaskError, TaskAccessError, TaskEvent, TaskEventBatch,
    TaskEventKind, TaskFailureKind, TaskId, TaskKind, TaskPhase, TaskSnapshot,
    VolumeCapacityObservation, VolumeCapacityStatus, VolumeCapacityStatusError,
};
pub use error::{DuxError, Result};
pub use persistence::{
    DATABASE_SCHEMA_VERSION, DatabaseAccess, DatabaseOpenErrorKind, DatabaseStatus,
    SnapshotOpenErrorKind,
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

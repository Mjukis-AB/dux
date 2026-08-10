//! Shared engine lifecycle and bounded task orchestration.
//!
//! The engine runs read-only formatting, deterministic volume-pressure status,
//! durable full-scan, bounded DUX-owned history and snapshot-maintenance tasks,
//! and typed settings operations. One opaque reviewed Rust-target capability
//! can enter a serialized permanent-safe task; FFI transport and explicit
//! native confirmation remain separate boundaries.

mod ai_insight_cache;
mod ai_metadata_preview;
mod app_data_reset;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod app_data_reset_recovery;
mod automation;
mod automation_history_suggestion;
mod candidate_history;
mod cleanup_history;
mod cleanup_history_clear;
mod cleanup_recovery_diagnostic;
mod cloud_eviction_probe;
mod config;
mod emergency_recovery;
mod legacy_running_scan_dismissal;
mod managed_scan_cache;
mod registry;
mod rule_outcome;
mod running_scan_debt;
mod rust_target_cleanup;
mod rust_target_dry_run;
mod rust_target_plan_review;
mod scan_coverage_details;
mod settings;
mod snapshot_diff_review;
mod snapshot_review;
mod snapshot_storage_clear;
mod storage_footprint;
mod storage_thief;
mod targeted_project_scan;

pub(crate) use snapshot_review::SnapshotReviewTrashTarget;
mod task;
mod volume_status;

pub use ai_insight_cache::{
    AI_INSIGHT_CACHE_CLEAR_PREVIEW_LIFETIME, AiCachedExplanation, AiInsightCacheClearError,
    AiInsightCacheClearPreview, AiInsightCacheClearPreviewInfo, AiInsightCacheClearResult,
    AiInsightCacheError,
};
pub use ai_metadata_preview::{
    AI_EXPLANATION_ATTEMPT_LIFETIME, AI_EXPLANATION_MODEL, AI_EXPLANATION_PROVIDER,
    AI_EXPLANATION_PROVIDER_BINDING_REVISION, AI_EXPLANATION_TRANSPORT,
    AI_METADATA_PREVIEW_LIFETIME, AI_METADATA_PRIVACY_POLICY_REVISION, AiExplanationAttempt,
    AiExplanationAttemptError, AiExplanationAttemptInfo, AiExplanationGroup, AiExplanationResult,
    AiMetadataPreview, AiMetadataPreviewAgeSummary, AiMetadataPreviewChild, AiMetadataPreviewError,
    AiMetadataPreviewInfo, AiMetadataPreviewNodeKind,
};
pub use app_data_reset::AppDataResetShutdownError;
pub use automation::{
    AutomationGlobalControl, AutomationGlobalControlSource, AutomationGlobalControlUpdate,
    AutomationOverview, AutomationScheduleDraftDeleteOutcome,
    AutomationScheduleDraftEligibilityAssessment, AutomationScheduleDraftEligibilityStatus,
    AutomationScheduleDraftError, AutomationScheduleDraftUpdate, AutomationScheduleUpdate,
};
pub use automation_history_suggestion::{
    AutomationScheduleSuggestion, AutomationScheduleSuggestionError,
    AutomationScheduleSuggestionFeed, MAX_AUTOMATION_HISTORY_SUGGESTION_SOURCE_SESSIONS,
    MAX_AUTOMATION_HISTORY_SUGGESTIONS,
};
pub use candidate_history::{
    CandidateDetailError, CandidateReviewCommand, CandidateReviewError, CandidateReviewResult,
    DurableCandidateEvidence, DurableCandidateEvidenceItem, DurableCandidateEvidencePage,
    DurableCandidatePathItem, DurableCandidatePathPage, DurableObservedPath, DurablePathEncoding,
    MAX_CANDIDATE_DETAIL_PAGE_LIMIT,
};
pub use cleanup_history::{
    CleanupHistoryCursor, CleanupHistoryError, DurableCleanupErrorCategory,
    DurableCleanupHistoryPage, DurableCleanupItemStatus, DurableCleanupItemSummary,
    DurableCleanupMode, DurableCleanupRecordFormat, DurableCleanupSessionId,
    DurableCleanupSessionObservation, DurableCleanupSessionStatus, DurableCleanupSessionSummary,
    DurableCleanupStatusCounts, DurableCleanupTrigger, DurableCleanupWarning,
    MAX_RECENT_CLEANUP_HISTORY_LIMIT,
};
pub use cleanup_history_clear::{
    CleanupHistoryClearError, CleanupHistoryClearPreview, CleanupHistoryClearPreviewInfo,
    CleanupHistoryClearResult,
};
pub use cleanup_recovery_diagnostic::{
    CleanupRecoveryDiagnosticCensus, CleanupRecoveryDiagnosticCensusError,
    MAX_CLEANUP_RECOVERY_DIAGNOSTIC_CENSUS_ROWS,
};
pub use cloud_eviction_probe::{
    CloudEvictionProbeError, CloudEvictionProbePlatformError, CloudEvictionProbeRequest,
    CloudEvictionProbeRequestError,
};
pub use config::{EngineConfig, EngineConfigError, EngineConfigField, EngineConfigReason};
pub use emergency_recovery::{
    EMERGENCY_RECOVERY_MAX_EVIDENCE_AGE, EMERGENCY_RECOVERY_POLICY_REVISION,
    EmergencyRecoveryError, EmergencyRecoveryGroup, EmergencyRecoveryLane,
    EmergencyRecoveryOrdering, EmergencyRecoverySource, MAX_EMERGENCY_RECOVERY_GROUPS,
};
pub use legacy_running_scan_dismissal::{
    LEGACY_RUNNING_SCAN_DISMISSAL_PREVIEW_LIFETIME, LegacyRunningScanDismissalError,
    LegacyRunningScanDismissalPreview, LegacyRunningScanDismissalPreviewInfo,
    LegacyRunningScanDismissalResult, MAX_LEGACY_RUNNING_SCAN_DISMISSAL_ROWS,
};
pub use managed_scan_cache::{
    DuxManagedScanCacheClearError, DuxManagedScanCacheClearPreview,
    DuxManagedScanCacheClearPreviewInfo, DuxManagedScanCacheClearResult, DuxManagedScanCacheError,
};
pub use registry::{
    AppDataResetAdmissionOutcome, AppDataResetQuiesced, AppDataResetRecoveryPhase,
    AppDataResetShutdown, AppDataResetValidationOutcome, EngineHandle, StandaloneScanScopeLease,
};
pub use rule_outcome::{
    DurableRuleOutcome, DurableRuleOutcomeBatch, DurableRuleOutcomeState, RuleOutcomeError,
    RuleOutcomeNotEligibleReason,
};
pub use running_scan_debt::{
    ClaimedRunningScanProvenanceCensus, ClaimedRunningScanProvenanceCensusError,
    MAX_CLAIMED_RUNNING_SCAN_PROVENANCE_CENSUS_ROWS, MAX_RUNNING_SCAN_DEBT_CENSUS_ROWS,
    RunningScanDebtCensus, RunningScanDebtCensusError,
};
pub use rust_target_cleanup::{
    RustTargetCleanupError, RustTargetCleanupResult, RustTargetCleanupStartFailure,
};
pub use rust_target_dry_run::{
    RustTargetDryRunError, RustTargetDryRunResult, RustTargetDryRunStartFailure,
};
pub use rust_target_plan_review::{
    PendingRustTargetPlanReview, RustTargetPlanReview, RustTargetPlanReviewAdmission,
    RustTargetPlanReviewError, RustTargetPlanReviewInfo, ValidatedPendingRustTargetPlanReview,
};
pub use scan_coverage_details::{
    DurableScanCoverageDetailsPage, DurableScanIssue, DurableScanIssueKind,
    DurableScanIssueLocation, MAX_SCAN_COVERAGE_DETAIL_PAGE_LIMIT, ScanCoverageDetailsError,
};
pub use settings::{
    CleanupExclusionSource, CleanupExclusions, CleanupExclusionsError, CleanupExclusionsUpdate,
    ConfiguredProjectRoots, ConfiguredProjectRootsError, ConfiguredProjectRootsSource,
    ConfiguredProjectRootsUpdate, DirectCargoCodeSignature, DirectCargoEnrollmentError,
    DirectCargoEnrollmentPreview, DirectCargoEnrollmentState, DirectCargoEnrollmentStatus,
    DirectCargoEnrollmentUpdate, DirectCargoSignatureClass, DiskPressurePolicy,
    DiskPressurePolicyError, DiskPressurePolicySource, DiskPressurePolicyUpdate,
    PermanentCleanupPolicy, PermanentCleanupPolicyError, PermanentCleanupPolicySource,
    PermanentCleanupPolicyUpdate, SnapshotRetentionCap, SnapshotRetentionCapError,
    SnapshotRetentionCapSource, SnapshotRetentionCapUpdate,
};
pub use snapshot_diff_review::{
    SnapshotDiffChange, SnapshotDiffCoverage, SnapshotDiffDirection, SnapshotDiffInfo,
    SnapshotDiffNode, SnapshotDiffNodePage, SnapshotDiffNodeSort, SnapshotDiffReviewSession,
    SnapshotDiffTreemap, SnapshotDiffTreemapCell, SnapshotDiffValue,
};
pub use snapshot_review::{
    MAX_SNAPSHOT_REVIEW_ICLOUD_OBSERVATION_TARGETS,
    MAX_SNAPSHOT_REVIEW_ICLOUD_OBSERVATION_VISITED_NODES, MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS,
    MAX_SNAPSHOT_REVIEW_NODE_PAGE_LIMIT, MAX_SNAPSHOT_REVIEW_PARENT_CONTEXT_COMPONENTS,
    MAX_SNAPSHOT_REVIEW_TREEMAP_CELLS, SnapshotReviewCategory, SnapshotReviewError,
    SnapshotReviewICloudObservationSource, SnapshotReviewICloudObservationTarget,
    SnapshotReviewLargeFile, SnapshotReviewLargeFilePage, SnapshotReviewLiveTarget,
    SnapshotReviewLiveTargetKind, SnapshotReviewLiveTargetPurpose, SnapshotReviewName,
    SnapshotReviewNameEncoding, SnapshotReviewNode, SnapshotReviewNodeKind, SnapshotReviewNodePage,
    SnapshotReviewNodeSort, SnapshotReviewReleaseOutcome, SnapshotReviewScanFlags,
    SnapshotReviewSession, SnapshotReviewTimestamp, SnapshotReviewTreemap,
    SnapshotReviewTreemapCell,
};
pub use snapshot_storage_clear::{
    DuxSnapshotStorageClearError, DuxSnapshotStorageClearPreview,
    DuxSnapshotStorageClearPreviewInfo, DuxSnapshotStorageClearResult,
};
pub use storage_footprint::{
    DuxEmbeddedAiCacheFootprint, DuxLegacyExternalSnapshotStageCensus,
    DuxManagedScanCacheFootprint, DuxOwnedStorageFootprint, DuxOwnedStorageFootprintError,
    DuxOwnedStorageUsage, DuxSnapshotStorageFootprint,
};
pub use storage_thief::{
    DurableStorageThiefGroup, DurableStorageThiefRanking, MAX_STORAGE_THIEF_RANKING_GROUPS,
    MAX_STORAGE_THIEF_RANKING_SOURCE_SESSIONS, StorageThiefError,
};
pub use targeted_project_scan::{
    MAX_TARGETED_CONFIGURED_PROJECT_ROOTS, MAX_TARGETED_PRESSURE_CHAIN_EPISODES,
    MAX_TARGETED_PROJECT_SCAN_NODES, MAX_TARGETED_PROJECT_SCAN_PASS_NODES,
    MAX_TARGETED_RECLAIM_ROOTS, MAX_TARGETED_USER_LIBRARY_CACHES_SCAN_NODES,
    MIN_TARGETED_CONFIGURED_PROJECT_SCAN_NODES, TARGETED_RECLAIM_ROOT_POLICY_REVISION,
    TargetedProjectScanAdmission, TargetedProjectScanCheckpoint, TargetedProjectScanCurrent,
    TargetedProjectScanDisposition, TargetedProjectScanError, TargetedProjectScanPressure,
    TargetedProjectScanPressureContext, TargetedProjectScanSelection, TargetedReclaimCatalogError,
    TargetedReclaimRootCatalogSlot, TargetedReclaimRootCatalogStamp, TargetedReclaimRootKind,
    TargetedReclaimScanBudget, targeted_reclaim_root_catalog_layout, targeted_reclaim_scan_budget,
};
pub use task::{
    CancelOutcome, CandidateEvaluationRecoveryMaintenanceFailureKind,
    CandidateEvaluationRecoveryMaintenanceOutcome, CandidateEvaluationRecoveryMaintenanceResult,
    CandidateEvaluationRecoveryMaintenanceStartOutcome, CandidateEvaluationTaskFailureKind,
    CandidateEvaluationTaskStatus, CandidateHistoryError, CloseOutcome, DurableCandidateEvaluation,
    DurableCandidateEvaluationStatus, DurableCandidateStatus, DurableCandidateSummary,
    DurableScanCounts, DurableScanCoverage, DurableScanStatus, DurableScanSummary, EngineLifecycle,
    EngineOpenError, FormatSizeBatchResult, FormattedSizeEntry, HistoryMaintenanceFailureKind,
    HistoryMaintenanceResult, HistoryMaintenanceStartOutcome, PermanentSafeCleanupFailureKind,
    RecentScanHistory, RustTargetDryRunFailureKind, ScanHistoryError,
    ScanRecoveryMaintenanceFailureKind, ScanRecoveryMaintenanceOutcome,
    ScanRecoveryMaintenanceResult, ScanRecoveryMaintenanceStartOutcome, ScanRootErrorKind,
    ScanTaskCounts, ScanTaskOrigin, ScanTaskResult, ScanTaskStatus,
    SnapshotOrphanMaintenanceFailureKind, SnapshotOrphanMaintenanceOutcome,
    SnapshotOrphanMaintenanceResult, SnapshotOrphanMaintenanceStartOutcome,
    SnapshotProvisioningStageMaintenanceFailureKind, SnapshotProvisioningStageMaintenanceOutcome,
    SnapshotProvisioningStageMaintenanceResult, SnapshotProvisioningStageMaintenanceStartOutcome,
    SnapshotRetentionFailureKind, SnapshotRetentionOutcome, SnapshotRetentionResult,
    SnapshotRetentionStartOutcome, SnapshotTerminalTempMaintenanceFailureKind,
    SnapshotTerminalTempMaintenanceOutcome, SnapshotTerminalTempMaintenanceResult,
    SnapshotTerminalTempMaintenanceStartOutcome, SnapshotUnleasedTempMaintenanceFailureKind,
    SnapshotUnleasedTempMaintenanceOutcome, SnapshotUnleasedTempMaintenanceResult,
    SnapshotUnleasedTempMaintenanceStartOutcome, StartSubtreeScanError, StartTaskError,
    TaskAccessError, TaskEvent, TaskEventBatch, TaskEventKind, TaskFailureKind, TaskId, TaskKind,
    TaskPhase, TaskPriority, TaskSnapshot,
};
pub use volume_status::{
    CapacityHistoryDisposition, CapacityTrend, CapacityTrendChange, CapacityTrendPoint,
    CapacityTrendPointSource, MAX_PRESSURE_EPISODE_HISTORY_LIMIT, PressureEpisode,
    PressureEpisodeHistory, PressureEpisodeHistoryError, PressureEpisodeLevel,
    VolumeCapacityObservation, VolumeCapacityStatus, VolumeCapacityStatusError,
};

//! Shared engine lifecycle and bounded task orchestration.
//!
//! The engine runs read-only formatting, deterministic volume-pressure status,
//! durable full-scan, bounded DUX-owned history and snapshot-maintenance tasks,
//! and typed settings operations. It grants no cleanup authority; FFI
//! transport and cleanup execution remain separate boundaries.

mod candidate_history;
mod cleanup_history;
mod config;
mod registry;
mod scan_coverage_details;
mod settings;
mod snapshot_review;

pub(crate) use snapshot_review::SnapshotReviewTrashTarget;
mod task;
mod volume_status;

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
pub use config::{EngineConfig, EngineConfigError, EngineConfigField, EngineConfigReason};
pub use registry::EngineHandle;
pub use scan_coverage_details::{
    DurableScanCoverageDetailsPage, DurableScanIssue, DurableScanIssueKind,
    DurableScanIssueLocation, MAX_SCAN_COVERAGE_DETAIL_PAGE_LIMIT, ScanCoverageDetailsError,
};
pub use settings::{
    DirectCargoCodeSignature, DirectCargoEnrollmentError, DirectCargoEnrollmentPreview,
    DirectCargoEnrollmentState, DirectCargoEnrollmentStatus, DirectCargoEnrollmentUpdate,
    DirectCargoSignatureClass, DiskPressurePolicy, DiskPressurePolicyError,
    DiskPressurePolicySource, DiskPressurePolicyUpdate, SnapshotRetentionCap,
    SnapshotRetentionCapError, SnapshotRetentionCapSource, SnapshotRetentionCapUpdate,
};
pub use snapshot_review::{
    MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS, MAX_SNAPSHOT_REVIEW_NODE_PAGE_LIMIT,
    MAX_SNAPSHOT_REVIEW_PARENT_CONTEXT_COMPONENTS, MAX_SNAPSHOT_REVIEW_TREEMAP_CELLS,
    SnapshotReviewCategory, SnapshotReviewError, SnapshotReviewLargeFile,
    SnapshotReviewLargeFilePage, SnapshotReviewLiveTarget, SnapshotReviewLiveTargetKind,
    SnapshotReviewLiveTargetPurpose, SnapshotReviewName, SnapshotReviewNameEncoding,
    SnapshotReviewNode, SnapshotReviewNodeKind, SnapshotReviewNodePage, SnapshotReviewNodeSort,
    SnapshotReviewReleaseOutcome, SnapshotReviewScanFlags, SnapshotReviewSession,
    SnapshotReviewTimestamp, SnapshotReviewTreemap, SnapshotReviewTreemapCell,
};
pub use task::{
    CancelOutcome, CandidateEvaluationTaskFailureKind, CandidateEvaluationTaskStatus,
    CandidateHistoryError, CloseOutcome, DurableCandidateEvaluation,
    DurableCandidateEvaluationStatus, DurableCandidateStatus, DurableCandidateSummary,
    DurableScanCounts, DurableScanCoverage, DurableScanStatus, DurableScanSummary, EngineLifecycle,
    EngineOpenError, FormatSizeBatchResult, FormattedSizeEntry, HistoryMaintenanceFailureKind,
    HistoryMaintenanceResult, HistoryMaintenanceStartOutcome, RecentScanHistory, ScanHistoryError,
    ScanRecoveryMaintenanceFailureKind, ScanRecoveryMaintenanceOutcome,
    ScanRecoveryMaintenanceResult, ScanRecoveryMaintenanceStartOutcome, ScanRootErrorKind,
    ScanTaskCounts, ScanTaskResult, ScanTaskStatus, SnapshotOrphanMaintenanceFailureKind,
    SnapshotOrphanMaintenanceOutcome, SnapshotOrphanMaintenanceResult,
    SnapshotOrphanMaintenanceStartOutcome, SnapshotProvisioningStageMaintenanceFailureKind,
    SnapshotProvisioningStageMaintenanceOutcome, SnapshotProvisioningStageMaintenanceResult,
    SnapshotProvisioningStageMaintenanceStartOutcome, SnapshotRetentionFailureKind,
    SnapshotRetentionOutcome, SnapshotRetentionResult, SnapshotRetentionStartOutcome,
    SnapshotTerminalTempMaintenanceFailureKind, SnapshotTerminalTempMaintenanceOutcome,
    SnapshotTerminalTempMaintenanceResult, SnapshotTerminalTempMaintenanceStartOutcome,
    SnapshotUnleasedTempMaintenanceFailureKind, SnapshotUnleasedTempMaintenanceOutcome,
    SnapshotUnleasedTempMaintenanceResult, SnapshotUnleasedTempMaintenanceStartOutcome,
    StartSubtreeScanError, StartTaskError, TaskAccessError, TaskEvent, TaskEventBatch,
    TaskEventKind, TaskFailureKind, TaskId, TaskKind, TaskPhase, TaskSnapshot,
};
pub use volume_status::{
    CapacityHistoryDisposition, CapacityTrend, CapacityTrendChange, CapacityTrendPoint,
    CapacityTrendPointSource, VolumeCapacityObservation, VolumeCapacityStatus,
    VolumeCapacityStatusError,
};

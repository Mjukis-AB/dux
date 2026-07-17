//! Shared engine lifecycle and bounded task orchestration.
//!
//! The engine runs read-only formatting, durable full-scan, bounded DUX-owned
//! history and snapshot-retention tasks, and typed settings operations. It
//! grants no cleanup authority; FFI transport and cleanup execution remain
//! separate boundaries.

mod candidate_history;
mod cleanup_history;
mod config;
mod registry;
mod settings;
mod task;

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
pub use settings::{
    SnapshotRetentionCap, SnapshotRetentionCapError, SnapshotRetentionCapSource,
    SnapshotRetentionCapUpdate,
};
pub use task::{
    CancelOutcome, CandidateEvaluationTaskFailureKind, CandidateEvaluationTaskStatus,
    CandidateHistoryError, CloseOutcome, DurableCandidateEvaluation,
    DurableCandidateEvaluationStatus, DurableCandidateStatus, DurableCandidateSummary,
    DurableScanCounts, DurableScanCoverage, DurableScanStatus, DurableScanSummary, EngineLifecycle,
    EngineOpenError, FormatSizeBatchResult, FormattedSizeEntry, HistoryMaintenanceFailureKind,
    HistoryMaintenanceResult, HistoryMaintenanceStartOutcome, RecentScanHistory, ScanHistoryError,
    ScanRootErrorKind, ScanTaskCounts, ScanTaskResult, ScanTaskStatus,
    SnapshotOrphanMaintenanceFailureKind, SnapshotOrphanMaintenanceOutcome,
    SnapshotOrphanMaintenanceResult, SnapshotOrphanMaintenanceStartOutcome,
    SnapshotRetentionFailureKind, SnapshotRetentionOutcome, SnapshotRetentionResult,
    SnapshotRetentionStartOutcome, StartTaskError, TaskAccessError, TaskEvent, TaskEventBatch,
    TaskEventKind, TaskFailureKind, TaskId, TaskKind, TaskPhase, TaskSnapshot,
};

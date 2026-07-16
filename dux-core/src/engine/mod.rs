//! Shared engine lifecycle and bounded task orchestration.
//!
//! The engine runs read-only formatting, durable full-scan, bounded DUX-owned
//! history-maintenance tasks, and typed settings operations. It grants no
//! cleanup authority; FFI transport and cleanup execution remain separate
//! boundaries.

mod config;
mod registry;
mod settings;
mod task;

pub use config::{EngineConfig, EngineConfigError, EngineConfigField, EngineConfigReason};
pub use registry::EngineHandle;
pub use settings::{
    SnapshotRetentionCap, SnapshotRetentionCapError, SnapshotRetentionCapSource,
    SnapshotRetentionCapUpdate,
};
pub use task::{
    CancelOutcome, CandidateEvaluationTaskFailureKind, CandidateEvaluationTaskStatus, CloseOutcome,
    DurableScanCounts, DurableScanCoverage, DurableScanStatus, DurableScanSummary, EngineLifecycle,
    EngineOpenError, FormatSizeBatchResult, FormattedSizeEntry, HistoryMaintenanceFailureKind,
    HistoryMaintenanceResult, HistoryMaintenanceStartOutcome, RecentScanHistory, ScanHistoryError,
    ScanRootErrorKind, ScanTaskCounts, ScanTaskResult, ScanTaskStatus, StartTaskError,
    TaskAccessError, TaskEvent, TaskEventBatch, TaskEventKind, TaskFailureKind, TaskId, TaskKind,
    TaskPhase, TaskSnapshot,
};

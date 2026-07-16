//! Shared engine lifecycle and bounded task orchestration.
//!
//! The engine runs read-only formatting and durable full-scan tasks. It grants
//! no cleanup authority; FFI task transport and cleanup execution remain
//! separate boundaries.

mod config;
mod registry;
mod task;

pub use config::{EngineConfig, EngineConfigError, EngineConfigField, EngineConfigReason};
pub use registry::EngineHandle;
pub use task::{
    CancelOutcome, CandidateEvaluationTaskFailureKind, CandidateEvaluationTaskStatus, CloseOutcome,
    EngineLifecycle, EngineOpenError, FormatSizeBatchResult, FormattedSizeEntry, ScanRootErrorKind,
    ScanTaskCounts, ScanTaskResult, ScanTaskStatus, StartTaskError, TaskAccessError, TaskEvent,
    TaskEventBatch, TaskEventKind, TaskFailureKind, TaskId, TaskKind, TaskPhase, TaskSnapshot,
};

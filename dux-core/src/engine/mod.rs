//! Shared engine lifecycle and bounded task orchestration.
//!
//! The first real operation is deliberately read-only. This module grants no
//! cleanup authority and does not yet claim scan, persistence, or FFI task
//! integration.

mod config;
mod registry;
mod task;

pub use config::{EngineConfig, EngineConfigError, EngineConfigField, EngineConfigReason};
pub use registry::EngineHandle;
pub use task::{
    CancelOutcome, CloseOutcome, EngineLifecycle, EngineOpenError, FormatSizeBatchResult,
    FormattedSizeEntry, StartTaskError, TaskAccessError, TaskEvent, TaskEventBatch, TaskEventKind,
    TaskFailureKind, TaskId, TaskKind, TaskPhase, TaskSnapshot,
};

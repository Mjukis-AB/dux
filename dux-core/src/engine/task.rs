use std::num::NonZeroU64;
use std::sync::Arc;

/// Opaque, non-durable identifier scoped to one running DUX process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct TaskId(NonZeroU64);

impl TaskId {
    pub fn get(self) -> u64 {
        self.0.get()
    }

    pub(super) const fn from_nonzero(value: NonZeroU64) -> Self {
        Self(value)
    }
}

/// Closed set of engine-owned task kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    FormatSizeBatch,
}

/// Execution phase. Cancellation intent is reported separately until work is
/// actually quiescent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskPhase {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl TaskPhase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskFailureKind {
    InternalFailure,
}

/// Authoritative current state for one retained task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskSnapshot {
    pub id: TaskId,
    pub kind: TaskKind,
    pub phase: TaskPhase,
    pub cancellation_requested: bool,
    pub revision: u64,
    pub result_available: bool,
    pub failure: Option<TaskFailureKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskEventKind {
    Queued,
    Started,
    Progress { completed: u64, total: u64 },
    CancellationRequested,
    Terminal { phase: TaskPhase },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskEvent {
    pub sequence: u64,
    pub kind: TaskEventKind,
}

/// Bounded pull page of task events. `truncated` means the requested cursor
/// predates the oldest event still retained in the per-task ring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskEventBatch {
    pub events: Vec<TaskEvent>,
    pub next_sequence: u64,
    pub oldest_available_sequence: u64,
    pub truncated: bool,
    pub terminal: bool,
}

/// One deterministic read-only formatting result. The display value remains a
/// migration probe, not a localization or cleanup authority contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormattedSizeEntry {
    pub bytes: u64,
    pub display: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatSizeBatchResult {
    entries: Arc<[FormattedSizeEntry]>,
}

impl FormatSizeBatchResult {
    pub(super) fn new(entries: Vec<FormattedSizeEntry>) -> Self {
        Self {
            entries: entries.into(),
        }
    }

    pub fn entries(&self) -> &[FormattedSizeEntry] {
        &self.entries
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineLifecycle {
    Open,
    Closing,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelOutcome {
    CancelledBeforeStart,
    Requested,
    AlreadyRequested,
    AlreadyTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    Initiated,
    AlreadyClosing,
    AlreadyClosed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EngineOpenError {
    #[error("engine database is unavailable: {0:?}")]
    Database(crate::persistence::DatabaseOpenErrorKind),
    #[error("engine worker resources are unavailable")]
    WorkerUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StartTaskError {
    #[error("engine session is closed")]
    Closed,
    #[error("engine task queue is full")]
    QueueFull,
    #[error("task input exceeds the fixed limit of {limit} items")]
    InputTooLarge { limit: u16 },
    #[error("engine task identifiers are exhausted")]
    TaskIdExhausted,
    #[error("engine task registry is unavailable")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TaskAccessError {
    #[error("engine session is closed")]
    Closed,
    #[error("unknown or expired task identifier")]
    UnknownTask,
    #[error("event page limit must be between 1 and {max}")]
    InvalidEventLimit { max: u16 },
    #[error("event cursor is ahead of the task event stream")]
    InvalidEventCursor,
    #[error("task has a different result kind")]
    WrongTaskKind,
    #[error("engine task registry is unavailable")]
    InternalState,
}

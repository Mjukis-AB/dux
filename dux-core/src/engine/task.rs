use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::SystemTime;

use crate::domain::{ScanCoverage, ScanId};

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
#[non_exhaustive]
pub enum TaskKind {
    FormatSizeBatch,
    Scan,
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
#[non_exhaustive]
pub enum TaskFailureKind {
    InternalFailure,
    ScanRootChanged,
    ScanFailed,
    SnapshotRejected,
    PersistenceUnavailable,
    PersistenceOutcomeUnknown,
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
#[non_exhaustive]
pub enum TaskEventKind {
    Queued,
    Started,
    Progress {
        completed: u64,
        total: u64,
    },
    ScanProgress {
        files: u64,
        directories: u64,
        known_allocated_bytes: u64,
        errors: u64,
    },
    ScanFinalizing,
    CancellationRequested,
    Terminal {
        phase: TaskPhase,
    },
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

/// Frozen counts from one durable scan summary. Non-successful scans retain
/// zero counts and unknown allocation rather than presenting an unfinalized
/// partial tree as an exact total.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanTaskCounts {
    pub directory_count: u64,
    pub file_count: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanTaskStatus {
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

/// Immutable, non-authoritative durable result for one engine scan task.
/// Paths and tree nodes remain behind the paged snapshot APIs added later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanTaskResult {
    scan_id: ScanId,
    started_at: SystemTime,
    completed_at: SystemTime,
    status: ScanTaskStatus,
    counts: ScanTaskCounts,
    coverage: ScanCoverage,
    snapshot_available: bool,
}

impl ScanTaskResult {
    pub(super) fn new(
        scan_id: ScanId,
        started_at: SystemTime,
        completed_at: SystemTime,
        status: ScanTaskStatus,
        counts: ScanTaskCounts,
        coverage: ScanCoverage,
        snapshot_available: bool,
    ) -> Self {
        Self {
            scan_id,
            started_at,
            completed_at,
            status,
            counts,
            coverage,
            snapshot_available,
        }
    }

    pub fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    pub fn started_at(&self) -> SystemTime {
        self.started_at
    }

    pub fn completed_at(&self) -> SystemTime {
        self.completed_at
    }

    pub fn status(&self) -> ScanTaskStatus {
        self.status
    }

    pub fn counts(&self) -> ScanTaskCounts {
        self.counts
    }

    pub fn coverage(&self) -> &ScanCoverage {
        &self.coverage
    }

    pub fn snapshot_available(&self) -> bool {
        self.snapshot_available
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
    #[error("engine snapshot storage is unavailable: {0:?}")]
    Snapshot(crate::persistence::SnapshotOpenErrorKind),
    #[error("engine worker resources are unavailable")]
    WorkerUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StartTaskError {
    #[error("engine session is closed")]
    Closed,
    #[error("engine task queue is full")]
    QueueFull,
    #[error("task input exceeds the fixed limit of {limit} items")]
    InputTooLarge { limit: u16 },
    #[error("scan root is invalid or unavailable: {reason:?}")]
    InvalidScanRoot { reason: ScanRootErrorKind },
    #[error("a scan for this filesystem object is already active as task {existing:?}")]
    ScanAlreadyActive { existing: TaskId },
    #[error("the durable engine store is read-only")]
    ReadOnlyStore,
    #[error("the durable engine store is unavailable")]
    PersistenceUnavailable,
    #[error("engine task identifiers are exhausted")]
    TaskIdExhausted,
    #[error("engine task registry is unavailable")]
    InternalState,
}

/// Path-free scan-root validation categories suitable for UI and FFI mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanRootErrorKind {
    InvalidPath,
    Missing,
    AccessDenied,
    NotDirectory,
    Symlink,
    ChangedDuringValidation,
    IdentityUnavailable,
    UnsupportedPlatform,
    Unavailable,
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

use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use super::config::EngineConfig;
use super::task::{
    CancelOutcome, CloseOutcome, EngineLifecycle, EngineOpenError, FormatSizeBatchResult,
    FormattedSizeEntry, StartTaskError, TaskAccessError, TaskEvent, TaskEventBatch, TaskEventKind,
    TaskFailureKind, TaskId, TaskKind, TaskPhase, TaskSnapshot,
};
use crate::persistence::{DatabaseStatus, StoreCoordinator};

const FORMAT_BATCH_LIMIT: usize = 256;

#[derive(Clone, Copy)]
struct RegistryLimits {
    workers: usize,
    queued_tasks: usize,
    retained_terminal_tasks: usize,
    events_per_task: usize,
}

impl RegistryLimits {
    const PRODUCTION: Self = Self {
        workers: 2,
        queued_tasks: 16,
        retained_terminal_tasks: 64,
        events_per_task: 64,
    };

    #[cfg(test)]
    fn testing(
        workers: usize,
        queued_tasks: usize,
        retained_terminal_tasks: usize,
        events_per_task: usize,
    ) -> Self {
        assert!(workers > 0);
        assert!(queued_tasks > 0);
        assert!(retained_terminal_tasks > 0);
        assert!(events_per_task > 0);
        Self {
            workers,
            queued_tasks,
            retained_terminal_tasks,
            events_per_task,
        }
    }
}

struct TaskIdAllocator {
    next: Mutex<Option<NonZeroU64>>,
}

impl TaskIdAllocator {
    const fn new(first: NonZeroU64) -> Self {
        Self {
            next: Mutex::new(Some(first)),
        }
    }

    fn allocate(&self) -> Result<TaskId, StartTaskError> {
        let mut next = self
            .next
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        let current = next.ok_or(StartTaskError::TaskIdExhausted)?;
        *next = current.get().checked_add(1).and_then(NonZeroU64::new);
        Ok(TaskId::from_nonzero(current))
    }
}

static TASK_IDS: TaskIdAllocator = TaskIdAllocator::new(NonZeroU64::MIN);

#[derive(Clone)]
struct CancellationFlag(Arc<AtomicBool>);

impl CancellationFlag {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    fn request(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn is_requested(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

enum TaskResult {
    FormatSizeBatch(Arc<FormatSizeBatchResult>),
    #[cfg(test)]
    TestOnly,
}

enum WorkOutcome {
    Succeeded(TaskResult),
    Cancelled,
}

struct TaskContext {
    id: TaskId,
    cancellation: CancellationFlag,
    shared: Arc<Shared>,
}

impl TaskContext {
    fn is_cancellation_requested(&self) -> bool {
        self.cancellation.is_requested()
    }

    fn report_progress(&self, completed: u64, total: u64) {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && !record.phase.is_terminal()
        {
            record.push_event(TaskEventKind::Progress { completed, total }, event_limit);
        }
    }
}

type Work = Box<dyn FnOnce(TaskContext) -> WorkOutcome + Send + 'static>;

struct Job {
    id: TaskId,
    work: Work,
}

struct TaskRecord {
    id: TaskId,
    kind: TaskKind,
    phase: TaskPhase,
    cancellation_requested: bool,
    cancellation: CancellationFlag,
    revision: u64,
    events: VecDeque<TaskEvent>,
    next_event_sequence: u64,
    result: Option<TaskResult>,
    failure: Option<TaskFailureKind>,
}

impl TaskRecord {
    fn new(id: TaskId, kind: TaskKind, event_limit: usize) -> Self {
        let mut record = Self {
            id,
            kind,
            phase: TaskPhase::Queued,
            cancellation_requested: false,
            cancellation: CancellationFlag::new(),
            revision: 0,
            events: VecDeque::new(),
            next_event_sequence: 1,
            result: None,
            failure: None,
        };
        record.push_event(TaskEventKind::Queued, event_limit);
        record
    }

    fn snapshot(&self) -> TaskSnapshot {
        TaskSnapshot {
            id: self.id,
            kind: self.kind,
            phase: self.phase,
            cancellation_requested: self.cancellation_requested,
            revision: self.revision,
            result_available: self.result.is_some(),
            failure: self.failure,
        }
    }

    fn push_event(&mut self, kind: TaskEventKind, limit: usize) {
        let sequence = self.next_event_sequence;
        self.next_event_sequence = self.next_event_sequence.saturating_add(1);
        self.revision = self.revision.saturating_add(1);
        self.events.push_back(TaskEvent { sequence, kind });
        while self.events.len() > limit {
            self.events.pop_front();
        }
    }

    fn request_cancellation(&mut self, event_limit: usize) -> bool {
        if self.cancellation_requested {
            return false;
        }
        self.cancellation_requested = true;
        self.cancellation.request();
        self.push_event(TaskEventKind::CancellationRequested, event_limit);
        true
    }
}

struct Registry {
    lifecycle: EngineLifecycle,
    queue: VecDeque<Job>,
    records: HashMap<TaskId, TaskRecord>,
    terminal_order: VecDeque<TaskId>,
    running_tasks: usize,
    live_workers: usize,
}

impl Registry {
    fn new() -> Self {
        Self {
            lifecycle: EngineLifecycle::Open,
            queue: VecDeque::new(),
            records: HashMap::new(),
            terminal_order: VecDeque::new(),
            running_tasks: 0,
            live_workers: 0,
        }
    }

    fn retain_terminal(&mut self, id: TaskId, limit: usize) {
        self.terminal_order.push_back(id);
        while self.terminal_order.len() > limit {
            if let Some(expired) = self.terminal_order.pop_front() {
                self.records.remove(&expired);
            }
        }
    }
}

struct Shared {
    registry: Mutex<Registry>,
    workers_ready: Condvar,
    lifecycle_changed: Condvar,
    limits: RegistryLimits,
}

impl Shared {
    fn new(limits: RegistryLimits) -> Self {
        Self {
            registry: Mutex::new(Registry::new()),
            workers_ready: Condvar::new(),
            lifecycle_changed: Condvar::new(),
            limits,
        }
    }

    fn request_close(&self) -> CloseOutcome {
        let mut registry = self.lock_registry_recover();
        match registry.lifecycle {
            EngineLifecycle::Closing => return CloseOutcome::AlreadyClosing,
            EngineLifecycle::Closed => return CloseOutcome::AlreadyClosed,
            EngineLifecycle::Open => registry.lifecycle = EngineLifecycle::Closing,
        }

        let queued: Vec<_> = registry.queue.drain(..).map(|job| job.id).collect();
        for id in queued {
            if let Some(record) = registry.records.get_mut(&id) {
                record.request_cancellation(self.limits.events_per_task);
                record.phase = TaskPhase::Cancelled;
                record.push_event(
                    TaskEventKind::Terminal {
                        phase: TaskPhase::Cancelled,
                    },
                    self.limits.events_per_task,
                );
            }
            registry.retain_terminal(id, self.limits.retained_terminal_tasks);
        }

        let running: Vec<_> = registry
            .records
            .iter()
            .filter_map(|(id, record)| (record.phase == TaskPhase::Running).then_some(*id))
            .collect();
        for id in running {
            if let Some(record) = registry.records.get_mut(&id) {
                record.request_cancellation(self.limits.events_per_task);
            }
        }
        self.workers_ready.notify_all();
        CloseOutcome::Initiated
    }

    fn lock_registry_recover(&self) -> MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

struct EngineInner {
    config: EngineConfig,
    store: Arc<StoreCoordinator>,
    _snapshots: crate::persistence::snapshot::SnapshotRepository,
    shared: Arc<Shared>,
    workers: Mutex<Option<Vec<JoinHandle<()>>>>,
}

impl Drop for EngineInner {
    fn drop(&mut self) {
        self.shared.request_close();
    }
}

/// Cloneable application-scoped handle to the shared core engine.
#[derive(Clone)]
pub struct EngineHandle {
    inner: Arc<EngineInner>,
}

impl EngineHandle {
    pub fn open(config: EngineConfig) -> Result<Self, EngineOpenError> {
        Self::open_with_limits(config, RegistryLimits::PRODUCTION)
    }

    fn open_with_limits(
        config: EngineConfig,
        limits: RegistryLimits,
    ) -> Result<Self, EngineOpenError> {
        Self::open_with_limits_and_snapshot_hook(config, limits, || {})
    }

    #[cfg(test)]
    fn open_with_snapshot_hook(
        config: EngineConfig,
        hook: impl FnOnce(),
    ) -> Result<Self, EngineOpenError> {
        Self::open_with_limits_and_snapshot_hook(config, RegistryLimits::PRODUCTION, hook)
    }

    fn open_with_limits_and_snapshot_hook(
        config: EngineConfig,
        limits: RegistryLimits,
        between_status_and_snapshot_open: impl FnOnce(),
    ) -> Result<Self, EngineOpenError> {
        // Durable storage is validated and migrated before any worker becomes
        // observable, so a failed open cannot leave a live partial engine.
        let store = StoreCoordinator::open(config.database_path())
            .map_err(|error| EngineOpenError::Database(error.kind))?;
        let database_status = store
            .status()
            .map_err(|error| EngineOpenError::Database(error.kind))?;
        between_status_and_snapshot_open();
        let snapshot_access = match database_status.access {
            crate::persistence::DatabaseAccess::ReadWriteCurrent => {
                crate::persistence::snapshot::SnapshotStoreAccess::ReadWrite
            }
            crate::persistence::DatabaseAccess::ReadOnlyNewer { .. } => {
                crate::persistence::snapshot::SnapshotStoreAccess::ReadOnly
            }
        };
        let snapshots = crate::persistence::snapshot::SnapshotRepository::open(
            Arc::clone(&store),
            snapshot_access,
        )
        .map_err(|error| EngineOpenError::Snapshot(error.open_kind()))?;
        let shared = Arc::new(Shared::new(limits));
        let mut workers = Vec::with_capacity(limits.workers);
        for index in 0..limits.workers {
            let worker_shared = Arc::clone(&shared);
            let spawn = std::thread::Builder::new()
                .name(format!("dux-engine-{}", index + 1))
                .spawn(move || worker_loop(worker_shared));
            match spawn {
                Ok(handle) => {
                    shared.lock_registry_recover().live_workers += 1;
                    workers.push(handle);
                }
                Err(_) => {
                    shared.request_close();
                    for handle in workers {
                        let _ = handle.join();
                    }
                    return Err(EngineOpenError::WorkerUnavailable);
                }
            }
        }

        Ok(Self {
            inner: Arc::new(EngineInner {
                config,
                store,
                _snapshots: snapshots,
                shared,
                workers: Mutex::new(Some(workers)),
            }),
        })
    }

    pub fn config(&self) -> &EngineConfig {
        &self.inner.config
    }

    /// Path-free compatibility status for the engine's durable store.
    pub fn database_status(
        &self,
    ) -> Result<DatabaseStatus, crate::persistence::DatabaseOpenErrorKind> {
        self.inner.store.status().map_err(|error| error.kind)
    }

    pub fn lifecycle(&self) -> EngineLifecycle {
        self.inner.shared.lock_registry_recover().lifecycle
    }

    pub fn start_format_size_batch(&self, values: Vec<u64>) -> Result<TaskId, StartTaskError> {
        if values.len() > FORMAT_BATCH_LIMIT {
            return Err(StartTaskError::InputTooLarge {
                limit: FORMAT_BATCH_LIMIT as u16,
            });
        }
        self.submit(
            TaskKind::FormatSizeBatch,
            Box::new(move |context| {
                let total = values.len() as u64;
                let mut entries = Vec::with_capacity(values.len());
                for (index, bytes) in values.into_iter().enumerate() {
                    if context.is_cancellation_requested() {
                        return WorkOutcome::Cancelled;
                    }
                    entries.push(FormattedSizeEntry {
                        bytes,
                        display: crate::format_size(bytes),
                    });
                    context.report_progress((index + 1) as u64, total);
                }
                if context.is_cancellation_requested() {
                    WorkOutcome::Cancelled
                } else {
                    WorkOutcome::Succeeded(TaskResult::FormatSizeBatch(Arc::new(
                        FormatSizeBatchResult::new(entries),
                    )))
                }
            }),
        )
    }

    pub fn task_snapshot(&self, id: TaskId) -> Result<TaskSnapshot, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        registry
            .records
            .get(&id)
            .map(TaskRecord::snapshot)
            .ok_or(TaskAccessError::UnknownTask)
    }

    pub fn task_events(
        &self,
        id: TaskId,
        after_sequence: u64,
        limit: u16,
    ) -> Result<TaskEventBatch, TaskAccessError> {
        if limit == 0 || usize::from(limit) > self.inner.shared.limits.events_per_task {
            return Err(TaskAccessError::InvalidEventLimit {
                max: self.inner.shared.limits.events_per_task as u16,
            });
        }
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        let latest = record.next_event_sequence.saturating_sub(1);
        if after_sequence > latest {
            return Err(TaskAccessError::InvalidEventCursor);
        }
        let oldest = record
            .events
            .front()
            .map(|event| event.sequence)
            .unwrap_or(record.next_event_sequence);
        let events: Vec<_> = record
            .events
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .take(usize::from(limit))
            .cloned()
            .collect();
        let next_sequence = events
            .last()
            .map(|event| event.sequence)
            .unwrap_or(after_sequence);
        Ok(TaskEventBatch {
            events,
            next_sequence,
            oldest_available_sequence: oldest,
            truncated: after_sequence.saturating_add(1) < oldest,
            terminal: record.phase.is_terminal(),
        })
    }

    pub fn format_size_batch_result(
        &self,
        id: TaskId,
    ) -> Result<Option<Arc<FormatSizeBatchResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::FormatSizeBatch {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::FormatSizeBatch(result)) => Some(Arc::clone(result)),
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn cancel_task(&self, id: TaskId) -> Result<CancelOutcome, TaskAccessError> {
        let mut registry = self.lock_open_registry_mut()?;
        let phase = registry
            .records
            .get(&id)
            .map(|record| record.phase)
            .ok_or(TaskAccessError::UnknownTask)?;
        if phase.is_terminal() {
            return Ok(CancelOutcome::AlreadyTerminal);
        }
        if registry
            .records
            .get(&id)
            .is_some_and(|record| record.cancellation_requested)
        {
            return Ok(CancelOutcome::AlreadyRequested);
        }

        let event_limit = self.inner.shared.limits.events_per_task;
        if phase == TaskPhase::Queued {
            let position = registry.queue.iter().position(|job| job.id == id);
            if let Some(position) = position {
                registry.queue.remove(position);
                if let Some(record) = registry.records.get_mut(&id) {
                    record.request_cancellation(event_limit);
                    record.phase = TaskPhase::Cancelled;
                    record.push_event(
                        TaskEventKind::Terminal {
                            phase: TaskPhase::Cancelled,
                        },
                        event_limit,
                    );
                }
                registry.retain_terminal(id, self.inner.shared.limits.retained_terminal_tasks);
                return Ok(CancelOutcome::CancelledBeforeStart);
            }
        }

        if let Some(record) = registry.records.get_mut(&id) {
            record.request_cancellation(event_limit);
        }
        Ok(CancelOutcome::Requested)
    }

    pub fn close(&self) -> CloseOutcome {
        self.inner.shared.request_close()
    }

    /// Wait for workers to acknowledge close and quiesce. This never initiates
    /// shutdown and is intended for off-main clients and tests.
    pub fn wait_until_closed(&self, timeout: Duration) -> bool {
        let registry = self.inner.shared.lock_registry_recover();
        let (registry, _) = self
            .inner
            .shared
            .lifecycle_changed
            .wait_timeout_while(registry, timeout, |state| {
                state.lifecycle != EngineLifecycle::Closed
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let closed = registry.lifecycle == EngineLifecycle::Closed;
        drop(registry);
        if closed
            && let Ok(mut workers) = self.inner.workers.lock()
            && let Some(handles) = workers.take()
        {
            for handle in handles {
                let _ = handle.join();
            }
        }
        closed
    }

    fn submit(&self, kind: TaskKind, work: Work) -> Result<TaskId, StartTaskError> {
        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if registry.queue.len() >= self.inner.shared.limits.queued_tasks {
            return Err(StartTaskError::QueueFull);
        }
        let id = TASK_IDS.allocate()?;
        let record = TaskRecord::new(id, kind, self.inner.shared.limits.events_per_task);
        registry.records.insert(id, record);
        registry.queue.push_back(Job { id, work });
        self.inner.shared.workers_ready.notify_one();
        Ok(id)
    }

    fn lock_open_registry(&self) -> Result<std::sync::MutexGuard<'_, Registry>, TaskAccessError> {
        let registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| TaskAccessError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            Err(TaskAccessError::Closed)
        } else {
            Ok(registry)
        }
    }

    fn lock_open_registry_mut(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Registry>, TaskAccessError> {
        self.lock_open_registry()
    }

    #[cfg(test)]
    fn submit_test(&self, work: Work) -> Result<TaskId, StartTaskError> {
        self.submit(TaskKind::FormatSizeBatch, work)
    }
}

fn worker_loop(shared: Arc<Shared>) {
    loop {
        let job = {
            let mut registry = shared.lock_registry_recover();
            loop {
                if let Some(job) = registry.queue.pop_front() {
                    let event_limit = shared.limits.events_per_task;
                    if let Some(record) = registry.records.get_mut(&job.id) {
                        record.phase = TaskPhase::Running;
                        record.push_event(TaskEventKind::Started, event_limit);
                        registry.running_tasks += 1;
                        break job;
                    }
                    continue;
                }
                if registry.lifecycle != EngineLifecycle::Open {
                    registry.live_workers = registry.live_workers.saturating_sub(1);
                    if registry.live_workers == 0 {
                        registry.lifecycle = EngineLifecycle::Closed;
                        shared.lifecycle_changed.notify_all();
                    }
                    return;
                }
                registry = shared
                    .workers_ready
                    .wait(registry)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };

        let cancellation = {
            let registry = shared.lock_registry_recover();
            registry
                .records
                .get(&job.id)
                .map(|record| record.cancellation.clone())
        };
        let outcome = cancellation.map(|cancellation| {
            catch_unwind(AssertUnwindSafe(|| {
                (job.work)(TaskContext {
                    id: job.id,
                    cancellation,
                    shared: Arc::clone(&shared),
                })
            }))
        });
        finish_job(&shared, job.id, outcome);
    }
}

fn finish_job(
    shared: &Shared,
    id: TaskId,
    outcome: Option<Result<WorkOutcome, Box<dyn std::any::Any + Send>>>,
) {
    let mut registry = shared.lock_registry_recover();
    let event_limit = shared.limits.events_per_task;
    let Some(record) = registry.records.get_mut(&id) else {
        registry.running_tasks = registry.running_tasks.saturating_sub(1);
        return;
    };
    record.result = None;
    record.failure = None;
    // Cancellation is intent, not evidence that completed work was rolled back.
    // The operation's outcome is authoritative after its own safe checkpoints.
    match outcome {
        Some(Ok(WorkOutcome::Succeeded(result))) => {
            record.phase = TaskPhase::Succeeded;
            record.result = Some(result);
        }
        Some(Ok(WorkOutcome::Cancelled)) => {
            record.phase = TaskPhase::Cancelled;
        }
        Some(Err(_)) | None => {
            record.phase = TaskPhase::Failed;
            record.failure = Some(TaskFailureKind::InternalFailure);
        }
    }
    let phase = record.phase;
    record.push_event(TaskEventKind::Terminal { phase }, event_limit);
    registry.running_tasks = registry.running_tasks.saturating_sub(1);
    registry.retain_terminal(id, shared.limits.retained_terminal_tasks);
    shared.lifecycle_changed.notify_all();
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;

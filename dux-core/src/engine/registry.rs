use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::config::EngineConfig;
use super::task::{
    CancelOutcome, CandidateEvaluationTaskFailureKind, CandidateEvaluationTaskStatus, CloseOutcome,
    DurableScanCounts, DurableScanCoverage, DurableScanStatus, DurableScanSummary, EngineLifecycle,
    EngineOpenError, FormatSizeBatchResult, FormattedSizeEntry, RecentScanHistory,
    ScanHistoryError, ScanRootErrorKind, ScanTaskCounts, ScanTaskResult, ScanTaskStatus,
    StartTaskError, TaskAccessError, TaskEvent, TaskEventBatch, TaskEventKind, TaskFailureKind,
    TaskId, TaskKind, TaskPhase, TaskSnapshot,
};
use crate::domain::{
    CANDIDATE_CATALOG_SCHEMA_VERSION, CANDIDATE_CATALOG_SHA256, CANDIDATE_CONTEXT_FORMAT_VERSION,
    CANDIDATE_EVALUATOR_REVISION, CandidateEvaluationError, ScanCoverage, ScanId,
    candidate_evaluation_context_digest_sha256, evaluate_completed_scan_candidates,
    validate_bundled_candidate_catalog,
};
use crate::persistence::snapshot::from_scan::prepare_completed_scan;
use crate::persistence::snapshot::{
    HostValue, SnapshotRepository, SnapshotRepositoryErrorKind, SnapshotStoreAccess,
};
use crate::persistence::{
    CandidateEvaluationCompletion, CandidateEvaluationFailureKind, CandidateEvaluationIdentity,
    HistoryErrorKind, MAX_RECENT_SCAN_HISTORY_LIMIT, NewCandidateRecord, NewScanRecord,
    ScanCompletionRecord, ScanCounts, ScanStatus, TerminalScanStatus,
};
use crate::persistence::{DatabaseStatus, StoreCoordinator};
use crate::scanner::{CancellationToken, ScanConfig, ScanMessage, ScanTermination, Scanner};

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
    Scan(Arc<ScanTaskResult>),
    #[cfg(test)]
    TestOnly,
}

enum WorkOutcome {
    Succeeded(TaskResult),
    Cancelled(Option<TaskResult>),
    Failed(TaskFailureKind, Option<TaskResult>),
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

    fn report_scan_message(&self, message: ScanMessage) {
        let kind = match message {
            ScanMessage::Progress(progress) => Some(TaskEventKind::ScanProgress {
                files: progress.files_scanned,
                directories: progress.dirs_scanned,
                known_allocated_bytes: progress.bytes_scanned,
                errors: progress.errors,
            }),
            ScanMessage::Finalizing => Some(TaskEventKind::ScanFinalizing),
            ScanMessage::StartedDirectory(_)
            | ScanMessage::Completed
            | ScanMessage::Cancelled
            | ScanMessage::Error(_) => None,
        };
        let Some(kind) = kind else {
            return;
        };
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && !record.phase.is_terminal()
        {
            record.push_event(kind, event_limit);
        }
    }

    fn report_candidate_evaluation_started(&self) {
        self.report_candidate_event(TaskEventKind::CandidateEvaluationStarted);
    }

    fn report_candidate_evaluation_finished(&self, status: CandidateEvaluationTaskStatus) {
        self.report_candidate_event(TaskEventKind::CandidateEvaluationFinished { status });
    }

    fn report_candidate_event(&self, kind: TaskEventKind) {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && !record.phase.is_terminal()
        {
            record.push_event(kind, event_limit);
        }
    }

    fn install_scan_cancellation(&self, token: CancellationToken) {
        let mut registry = self.shared.lock_registry_recover();
        if let Some(record) = registry.records.get_mut(&self.id) {
            if record.cancellation_requested {
                token.cancel();
            }
            record.scan_cancellation = Some(token);
        } else {
            token.cancel();
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
    scan_cancellation: Option<CancellationToken>,
    scan_scope: Option<PathBuf>,
    revision: u64,
    events: VecDeque<TaskEvent>,
    next_event_sequence: u64,
    result: Option<TaskResult>,
    failure: Option<TaskFailureKind>,
}

impl TaskRecord {
    fn new(id: TaskId, kind: TaskKind, scan_scope: Option<PathBuf>, event_limit: usize) -> Self {
        let mut record = Self {
            id,
            kind,
            phase: TaskPhase::Queued,
            cancellation_requested: false,
            cancellation: CancellationFlag::new(),
            scan_cancellation: None,
            scan_scope,
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
        if let Some(token) = &self.scan_cancellation {
            token.cancel();
        }
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
    active_scan_roots: HashMap<PathBuf, TaskId>,
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
            active_scan_roots: HashMap::new(),
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
            let scope = registry
                .records
                .get(&id)
                .and_then(|record| record.scan_scope.clone());
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
            if let Some(scope) = scope
                && registry.active_scan_roots.get(&scope) == Some(&id)
            {
                registry.active_scan_roots.remove(&scope);
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
    snapshots: Arc<SnapshotRepository>,
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
        validate_bundled_candidate_catalog()
            .map_err(|_| EngineOpenError::CandidateCatalogInvalid)?;
        // Durable storage is validated and migrated before any worker becomes
        // observable, so a failed open cannot leave a live partial engine.
        let store = StoreCoordinator::open(config.database_path())
            .map_err(|error| EngineOpenError::Database(error.kind))?;
        let database_status = store
            .status()
            .map_err(|error| EngineOpenError::Database(error.kind))?;
        between_status_and_snapshot_open();
        let snapshot_access = match database_status.access {
            crate::persistence::DatabaseAccess::ReadWriteCurrent => SnapshotStoreAccess::ReadWrite,
            crate::persistence::DatabaseAccess::ReadOnlyNewer { .. } => {
                SnapshotStoreAccess::ReadOnly
            }
        };
        let snapshots = Arc::new(
            SnapshotRepository::open(Arc::clone(&store), snapshot_access)
                .map_err(|error| EngineOpenError::Snapshot(error.open_kind()))?,
        );
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
                snapshots,
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

    /// Load a bounded, path-free page of durable scan observations. This reads
    /// only SQLite metadata and never opens snapshots or candidate records.
    pub fn recent_scan_history(&self, limit: usize) -> Result<RecentScanHistory, ScanHistoryError> {
        if !(1..=MAX_RECENT_SCAN_HISTORY_LIMIT).contains(&limit) {
            return Err(ScanHistoryError::InvalidLimit {
                max: MAX_RECENT_SCAN_HISTORY_LIMIT,
            });
        }
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(ScanHistoryError::Closed);
        }
        let page = self
            .inner
            .store
            .load_recent_scans(limit)
            .map_err(|error| map_scan_history_error(error.kind))?;
        let scans = page
            .records()
            .iter()
            .map(|record| {
                let status = match record.status() {
                    ScanStatus::Queued => DurableScanStatus::Queued,
                    ScanStatus::Running => DurableScanStatus::Running,
                    ScanStatus::Succeeded => DurableScanStatus::Succeeded,
                    ScanStatus::Failed => DurableScanStatus::Failed,
                    ScanStatus::Cancelled => DurableScanStatus::Cancelled,
                    ScanStatus::Interrupted => DurableScanStatus::Interrupted,
                };
                let counts = (record.status() == ScanStatus::Succeeded).then(|| {
                    let counts = record.counts();
                    DurableScanCounts {
                        directory_count: counts.directory_count,
                        file_count: counts.file_count,
                        logical_bytes: counts.logical_bytes,
                        allocated_bytes: counts.allocated_bytes,
                    }
                });
                let coverage = record.coverage();
                DurableScanSummary {
                    scan_id: record.id().clone(),
                    started_at: record.started_at(),
                    completed_at: record.completed_at(),
                    status,
                    counts,
                    coverage: DurableScanCoverage {
                        status: coverage.status(),
                        measured_permille: coverage.measured_permille(),
                        issue_record_count: coverage.issues().len(),
                        issue_occurrence_count: coverage
                            .issues()
                            .iter()
                            .map(|issue| u64::from(issue.occurrence_count()))
                            .sum(),
                    },
                    snapshot_recorded: record.snapshot().is_some(),
                }
            })
            .collect();
        Ok(RecentScanHistory {
            scans,
            has_more: page.has_more(),
        })
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
            None,
            Box::new(move |context| {
                let total = values.len() as u64;
                let mut entries = Vec::with_capacity(values.len());
                for (index, bytes) in values.into_iter().enumerate() {
                    if context.is_cancellation_requested() {
                        return WorkOutcome::Cancelled(None);
                    }
                    entries.push(FormattedSizeEntry {
                        bytes,
                        display: crate::format_size(bytes),
                    });
                    context.report_progress((index + 1) as u64, total);
                }
                if context.is_cancellation_requested() {
                    WorkOutcome::Cancelled(None)
                } else {
                    WorkOutcome::Succeeded(TaskResult::FormatSizeBatch(Arc::new(
                        FormatSizeBatchResult::new(entries),
                    )))
                }
            }),
        )
    }

    /// Start one full, no-follow, same-filesystem scan. Root validation and
    /// overlapping-scope admission are synchronous; the durable scan ID is
    /// generated only after a worker starts, so queued cancellation leaves no
    /// history row.
    pub fn start_scan(&self, root: PathBuf) -> Result<TaskId, StartTaskError> {
        self.start_scan_with_hooks(root, |_| {}, || {}, || {})
    }

    #[cfg(test)]
    fn start_scan_with_hook(
        &self,
        root: PathBuf,
        before_traversal: impl FnOnce(&ScanId) + Send + 'static,
    ) -> Result<TaskId, StartTaskError> {
        self.start_scan_with_hooks(root, before_traversal, || {}, || {})
    }

    fn start_scan_with_hooks(
        &self,
        root: PathBuf,
        before_traversal: impl FnOnce(&ScanId) + Send + 'static,
        before_candidate_evaluation: impl FnOnce() + Send + 'static,
        before_candidate_persistence: impl FnOnce() + Send + 'static,
    ) -> Result<TaskId, StartTaskError> {
        let canonical_root = prepare_scan_root(&root)?;
        let status = self
            .inner
            .store
            .status()
            .map_err(|_| StartTaskError::PersistenceUnavailable)?;
        if !matches!(
            status.access,
            crate::persistence::DatabaseAccess::ReadWriteCurrent
        ) {
            return Err(StartTaskError::ReadOnlyStore);
        }
        let store = Arc::clone(&self.inner.store);
        let snapshots = Arc::clone(&self.inner.snapshots);
        self.submit(
            TaskKind::Scan,
            Some(canonical_root.clone()),
            Box::new(move |context| {
                run_scan_task(
                    context,
                    canonical_root,
                    store,
                    snapshots,
                    before_traversal,
                    before_candidate_evaluation,
                    before_candidate_persistence,
                )
            }),
        )
    }

    #[cfg(test)]
    fn start_scan_with_before_traversal_hook(
        &self,
        root: PathBuf,
        before_traversal: impl FnOnce(&ScanId) + Send + 'static,
    ) -> Result<TaskId, StartTaskError> {
        self.start_scan_with_hook(root, before_traversal)
    }

    #[cfg(test)]
    fn start_scan_with_before_candidate_evaluation_hook(
        &self,
        root: PathBuf,
        before_candidate_evaluation: impl FnOnce() + Send + 'static,
    ) -> Result<TaskId, StartTaskError> {
        self.start_scan_with_hooks(root, |_| {}, before_candidate_evaluation, || {})
    }

    #[cfg(test)]
    fn start_scan_with_before_candidate_persistence_hook(
        &self,
        root: PathBuf,
        before_candidate_persistence: impl FnOnce() + Send + 'static,
    ) -> Result<TaskId, StartTaskError> {
        self.start_scan_with_hooks(root, |_| {}, || {}, before_candidate_persistence)
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
            Some(TaskResult::Scan(_)) => return Err(TaskAccessError::WrongTaskKind),
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn scan_result(&self, id: TaskId) -> Result<Option<Arc<ScanTaskResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::Scan {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::Scan(result)) => Some(Arc::clone(result)),
            Some(TaskResult::FormatSizeBatch(_)) => return Err(TaskAccessError::WrongTaskKind),
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
                let scope = registry
                    .records
                    .get(&id)
                    .and_then(|record| record.scan_scope.clone());
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
                if let Some(scope) = scope
                    && registry.active_scan_roots.get(&scope) == Some(&id)
                {
                    registry.active_scan_roots.remove(&scope);
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

    fn submit(
        &self,
        kind: TaskKind,
        scan_scope: Option<PathBuf>,
        work: Work,
    ) -> Result<TaskId, StartTaskError> {
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
        if let Some(scope) = &scan_scope
            && let Some(existing) = registry
                .active_scan_roots
                .iter()
                .find_map(|(active, id)| super::config::paths_overlap(active, scope).then_some(*id))
        {
            return Err(StartTaskError::ScanAlreadyActive { existing });
        }
        let id = TASK_IDS.allocate()?;
        let record = TaskRecord::new(
            id,
            kind,
            scan_scope.clone(),
            self.inner.shared.limits.events_per_task,
        );
        if let Some(scope) = scan_scope {
            registry.active_scan_roots.insert(scope, id);
        }
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
        self.submit(TaskKind::FormatSizeBatch, None, work)
    }
}

fn prepare_scan_root(root: &Path) -> Result<PathBuf, StartTaskError> {
    if !root.is_absolute() {
        return Err(StartTaskError::InvalidScanRoot {
            reason: ScanRootErrorKind::InvalidPath,
        });
    }
    let canonical =
        std::fs::canonicalize(root).map_err(|error| StartTaskError::InvalidScanRoot {
            reason: match error.kind() {
                std::io::ErrorKind::NotFound => ScanRootErrorKind::Missing,
                std::io::ErrorKind::PermissionDenied => ScanRootErrorKind::AccessDenied,
                _ => ScanRootErrorKind::Unavailable,
            },
        })?;
    let metadata =
        std::fs::metadata(&canonical).map_err(|error| StartTaskError::InvalidScanRoot {
            reason: match error.kind() {
                std::io::ErrorKind::NotFound => ScanRootErrorKind::Missing,
                std::io::ErrorKind::PermissionDenied => ScanRootErrorKind::AccessDenied,
                _ => ScanRootErrorKind::Unavailable,
            },
        })?;
    if !metadata.is_dir() {
        return Err(StartTaskError::InvalidScanRoot {
            reason: ScanRootErrorKind::NotDirectory,
        });
    }
    HostValue::from_root(&canonical).map_err(|_| StartTaskError::InvalidScanRoot {
        reason: ScanRootErrorKind::InvalidPath,
    })?;
    Ok(canonical)
}

fn generate_scan_id() -> Result<ScanId, TaskFailureKind> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| TaskFailureKind::InternalFailure)?;
    let mut value = String::with_capacity("scan:".len() + random.len() * 2);
    value.push_str("scan:");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in random {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    ScanId::new(value).map_err(|_| TaskFailureKind::InternalFailure)
}

fn start_durable_scan(
    store: &StoreCoordinator,
    root: &Path,
) -> Result<NewScanRecord, TaskFailureKind> {
    const COLLISION_RETRIES: usize = 4;
    for _ in 0..COLLISION_RETRIES {
        let id = generate_scan_id()?;
        let start = NewScanRecord::try_new(id, root.to_path_buf(), SystemTime::now())
            .map_err(|_| TaskFailureKind::PersistenceUnavailable)?;
        match store.record_scan_started_reconciled(&start) {
            Ok(()) => return Ok(start),
            Err(error) if error.kind == HistoryErrorKind::AlreadyExists => continue,
            Err(error) if error.kind == HistoryErrorKind::OutcomeUnknown => {
                return Err(TaskFailureKind::PersistenceOutcomeUnknown);
            }
            Err(_) => return Err(TaskFailureKind::PersistenceUnavailable),
        }
    }
    Err(TaskFailureKind::PersistenceUnavailable)
}

fn completion_time(started_at: SystemTime) -> SystemTime {
    let observed = SystemTime::now().max(started_at);
    let Ok(duration) = observed.duration_since(UNIX_EPOCH) else {
        return started_at;
    };
    let milliseconds = duration.as_millis();
    let Ok(milliseconds) = u64::try_from(milliseconds) else {
        return started_at;
    };
    if milliseconds > i64::MAX as u64 {
        return started_at;
    }
    UNIX_EPOCH + Duration::from_millis(milliseconds)
}

fn public_counts(counts: ScanCounts) -> ScanTaskCounts {
    ScanTaskCounts {
        directory_count: counts.directory_count,
        file_count: counts.file_count,
        logical_bytes: counts.logical_bytes,
        allocated_bytes: counts.allocated_bytes,
    }
}

struct DurableScanGuard {
    store: Arc<StoreCoordinator>,
    id: ScanId,
    started_at: SystemTime,
    settled: bool,
}

impl DurableScanGuard {
    fn new(store: Arc<StoreCoordinator>, start: &NewScanRecord) -> Self {
        Self {
            store,
            id: start.id().clone(),
            started_at: start.started_at(),
            settled: false,
        }
    }

    fn settle(
        &mut self,
        terminal_status: TerminalScanStatus,
        result_status: ScanTaskStatus,
        completed_at: SystemTime,
        counts: ScanCounts,
        coverage: ScanCoverage,
    ) -> Result<Arc<ScanTaskResult>, TaskFailureKind> {
        let completion = ScanCompletionRecord::try_new_with_coverage(
            self.id.clone(),
            completed_at,
            terminal_status,
            counts,
            coverage.clone(),
        )
        .map_err(|_| TaskFailureKind::PersistenceUnavailable)?;
        if let Err(error) = self.store.record_scan_finished_reconciled(&completion) {
            return Err(self.map_settle_failure(error.kind));
        }
        self.settled = true;
        Ok(Arc::new(ScanTaskResult::without_snapshot(
            self.id.clone(),
            self.started_at,
            completed_at,
            result_status,
            public_counts(counts),
            coverage,
        )))
    }

    fn disarm(&mut self) {
        self.settled = true;
    }

    fn map_settle_failure(&mut self, kind: HistoryErrorKind) -> TaskFailureKind {
        if kind == HistoryErrorKind::OutcomeUnknown {
            // The exact completion may already be durable. Never let Drop
            // retry that ambiguity with different Interrupted facts.
            self.settled = true;
            TaskFailureKind::PersistenceOutcomeUnknown
        } else {
            TaskFailureKind::PersistenceUnavailable
        }
    }
}

impl Drop for DurableScanGuard {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let Ok(completion) = ScanCompletionRecord::try_new_with_coverage(
            self.id.clone(),
            completion_time(self.started_at),
            TerminalScanStatus::Interrupted,
            ScanCounts::default(),
            ScanCoverage::unknown(),
        ) else {
            return;
        };
        let _ = self.store.record_scan_finished_reconciled(&completion);
    }
}

fn failed_scan_outcome(
    failure: TaskFailureKind,
    result: Result<Arc<ScanTaskResult>, TaskFailureKind>,
) -> WorkOutcome {
    match result {
        Ok(result) => WorkOutcome::Failed(failure, Some(TaskResult::Scan(result))),
        Err(persistence_failure) => WorkOutcome::Failed(persistence_failure, None),
    }
}

fn prepare_candidate_evaluation(
    context: &TaskContext,
    scan_id: &ScanId,
    artifact: &crate::scanner::CompletedScanArtifact,
    scheduled_at: SystemTime,
) -> Result<
    (
        CandidateEvaluationIdentity,
        CandidateEvaluationCompletion,
        CandidateEvaluationTaskStatus,
    ),
    TaskFailureKind,
> {
    let identity = CandidateEvaluationIdentity::try_new(
        CANDIDATE_EVALUATOR_REVISION,
        CANDIDATE_CATALOG_SCHEMA_VERSION,
        CANDIDATE_CATALOG_SHA256,
        CANDIDATE_CONTEXT_FORMAT_VERSION,
        candidate_evaluation_context_digest_sha256(scan_id, artifact),
    )
    .map_err(|_| TaskFailureKind::InternalFailure)?;
    context.report_candidate_evaluation_started();
    if context.is_cancellation_requested() {
        return failed_candidate_evaluation(
            identity,
            completion_time(scheduled_at),
            CandidateEvaluationFailureKind::Cancelled,
        );
    }

    let batch = match evaluate_completed_scan_candidates(scan_id, artifact) {
        Ok(batch) => batch,
        Err(error) => {
            let kind = map_candidate_evaluation_error(error);
            return failed_candidate_evaluation(identity, completion_time(scheduled_at), kind);
        }
    };
    if context.is_cancellation_requested() {
        return failed_candidate_evaluation(
            identity,
            completion_time(scheduled_at),
            CandidateEvaluationFailureKind::Cancelled,
        );
    }

    let observed_identity = CandidateEvaluationIdentity::try_new(
        batch.evaluator_revision(),
        batch.catalog_schema_version(),
        batch.catalog_digest_sha256(),
        batch.context_format_version(),
        batch.context_digest_sha256(),
    )
    .map_err(|_| TaskFailureKind::InternalFailure)?;
    if observed_identity != identity {
        return failed_candidate_evaluation(
            identity,
            completion_time(scheduled_at),
            CandidateEvaluationFailureKind::ContextInvalid,
        );
    }

    let evaluated_at = completion_time(scheduled_at);
    let candidates = batch
        .into_candidates()
        .iter()
        .map(|candidate| NewCandidateRecord::try_from_candidate(candidate, evaluated_at))
        .collect::<Result<Vec<_>, _>>();
    let candidates = match candidates {
        Ok(candidates) => candidates,
        Err(_) => {
            return failed_candidate_evaluation(
                identity,
                evaluated_at,
                CandidateEvaluationFailureKind::CandidateInvalid,
            );
        }
    };
    let candidate_count =
        u32::try_from(candidates.len()).map_err(|_| TaskFailureKind::InternalFailure)?;
    let completion = CandidateEvaluationCompletion::succeeded(evaluated_at, candidates)
        .map_err(|_| TaskFailureKind::InternalFailure)?;
    Ok((
        identity,
        completion,
        CandidateEvaluationTaskStatus::Succeeded { candidate_count },
    ))
}

fn failed_candidate_evaluation(
    identity: CandidateEvaluationIdentity,
    completed_at: SystemTime,
    kind: CandidateEvaluationFailureKind,
) -> Result<
    (
        CandidateEvaluationIdentity,
        CandidateEvaluationCompletion,
        CandidateEvaluationTaskStatus,
    ),
    TaskFailureKind,
> {
    let completion = CandidateEvaluationCompletion::failed(completed_at, kind)
        .map_err(|_| TaskFailureKind::InternalFailure)?;
    Ok((
        identity,
        completion,
        CandidateEvaluationTaskStatus::Failed {
            kind: public_candidate_evaluation_failure(kind),
        },
    ))
}

const fn map_candidate_evaluation_error(
    error: CandidateEvaluationError,
) -> CandidateEvaluationFailureKind {
    match error {
        CandidateEvaluationError::InvalidBundledCatalog => {
            CandidateEvaluationFailureKind::CatalogInvalid
        }
        CandidateEvaluationError::InvalidArtifactProjection => {
            CandidateEvaluationFailureKind::EvaluationFailed
        }
        CandidateEvaluationError::CandidateLimitExceeded { .. } => {
            CandidateEvaluationFailureKind::LimitExceeded
        }
        CandidateEvaluationError::InvalidCandidate(_) => {
            CandidateEvaluationFailureKind::CandidateInvalid
        }
    }
}

const fn map_scan_history_error(kind: HistoryErrorKind) -> ScanHistoryError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => ScanHistoryError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => ScanHistoryError::QueryLimitExceeded,
        HistoryErrorKind::Busy => ScanHistoryError::Busy,
        HistoryErrorKind::UnsafeStorage => ScanHistoryError::UnsafeStorage,
        HistoryErrorKind::CorruptData => ScanHistoryError::CorruptData,
        HistoryErrorKind::InternalState => ScanHistoryError::InternalState,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::DatabaseUnavailable
        | HistoryErrorKind::OutcomeUnknown => ScanHistoryError::Unavailable,
    }
}

const fn public_candidate_evaluation_failure(
    kind: CandidateEvaluationFailureKind,
) -> CandidateEvaluationTaskFailureKind {
    match kind {
        CandidateEvaluationFailureKind::Cancelled => CandidateEvaluationTaskFailureKind::Cancelled,
        CandidateEvaluationFailureKind::CatalogInvalid => {
            CandidateEvaluationTaskFailureKind::CatalogInvalid
        }
        CandidateEvaluationFailureKind::ContextInvalid => {
            CandidateEvaluationTaskFailureKind::ContextInvalid
        }
        CandidateEvaluationFailureKind::EvaluationFailed => {
            CandidateEvaluationTaskFailureKind::EvaluationFailed
        }
        CandidateEvaluationFailureKind::CandidateInvalid => {
            CandidateEvaluationTaskFailureKind::CandidateInvalid
        }
        CandidateEvaluationFailureKind::LimitExceeded => {
            CandidateEvaluationTaskFailureKind::LimitExceeded
        }
    }
}

fn run_scan_task(
    context: TaskContext,
    admitted_root: PathBuf,
    store: Arc<StoreCoordinator>,
    snapshots: Arc<SnapshotRepository>,
    before_traversal: impl FnOnce(&ScanId),
    before_candidate_evaluation: impl FnOnce(),
    before_candidate_persistence: impl FnOnce(),
) -> WorkOutcome {
    if prepare_scan_root(&admitted_root).ok().as_deref() != Some(admitted_root.as_path()) {
        return WorkOutcome::Failed(TaskFailureKind::ScanRootChanged, None);
    }
    let start = match start_durable_scan(&store, &admitted_root) {
        Ok(start) => start,
        Err(failure) => return WorkOutcome::Failed(failure, None),
    };
    let mut durable = DurableScanGuard::new(Arc::clone(&store), &start);
    let cancellation = CancellationToken::new();
    context.install_scan_cancellation(cancellation.clone());
    before_traversal(start.id());

    let scanner = Scanner::new(ScanConfig {
        follow_symlinks: false,
        max_depth: None,
        same_filesystem: true,
        num_threads: 0,
    })
    .with_cancellation(cancellation);
    let (messages, scanner_handle) = scanner.scan(admitted_root);
    for message in messages {
        context.report_scan_message(message);
    }
    let outcome = match scanner_handle.join() {
        Ok(outcome) => outcome,
        Err(_) => {
            let completed_at = completion_time(start.started_at());
            let result = durable.settle(
                TerminalScanStatus::Interrupted,
                ScanTaskStatus::Interrupted,
                completed_at,
                ScanCounts::default(),
                ScanCoverage::unknown(),
            );
            return failed_scan_outcome(TaskFailureKind::InternalFailure, result);
        }
    };
    let completed_at = completion_time(start.started_at());
    match outcome.termination() {
        ScanTermination::Cancelled => {
            let coverage = outcome.coverage().clone();
            let result = durable.settle(
                TerminalScanStatus::Cancelled,
                ScanTaskStatus::Cancelled,
                completed_at,
                ScanCounts::default(),
                coverage,
            );
            match result {
                Ok(result) => WorkOutcome::Cancelled(Some(TaskResult::Scan(result))),
                Err(failure) => WorkOutcome::Failed(failure, None),
            }
        }
        ScanTermination::Failed => {
            let coverage = outcome.coverage().clone();
            let result = durable.settle(
                TerminalScanStatus::Failed,
                ScanTaskStatus::Failed,
                completed_at,
                ScanCounts::default(),
                coverage,
            );
            failed_scan_outcome(TaskFailureKind::ScanFailed, result)
        }
        ScanTermination::Completed => {
            let Some(artifact) = outcome.into_completed_artifact() else {
                let result = durable.settle(
                    TerminalScanStatus::Failed,
                    ScanTaskStatus::Failed,
                    completed_at,
                    ScanCounts::default(),
                    ScanCoverage::unknown(),
                );
                return failed_scan_outcome(TaskFailureKind::SnapshotRejected, result);
            };
            let prepared = match prepare_completed_scan(start.id().clone(), completed_at, &artifact)
            {
                Ok(prepared) => prepared,
                Err(_) => {
                    let result = durable.settle(
                        TerminalScanStatus::Failed,
                        ScanTaskStatus::Failed,
                        completed_at,
                        ScanCounts::default(),
                        ScanCoverage::unknown(),
                    );
                    return failed_scan_outcome(TaskFailureKind::SnapshotRejected, result);
                }
            };
            before_candidate_evaluation();
            let (evaluation_identity, evaluation, evaluation_status) =
                match prepare_candidate_evaluation(&context, start.id(), &artifact, completed_at) {
                    Ok(evaluation) => evaluation,
                    Err(failure) => {
                        let result = durable.settle(
                            TerminalScanStatus::Failed,
                            ScanTaskStatus::Failed,
                            completed_at,
                            ScanCounts::default(),
                            ScanCoverage::unknown(),
                        );
                        return failed_scan_outcome(failure, result);
                    }
                };
            let (document, counts, coverage) = prepared.into_parts();
            // Candidate evaluation's final cancellation observation has
            // passed. Requests after this point remain truthful task intent
            // but cannot rewrite the immutable terminal batch being committed.
            before_candidate_persistence();
            match snapshots.complete_scan_with_candidate_evaluation(
                completed_at,
                counts,
                &coverage,
                &document,
                &evaluation_identity,
                &evaluation,
            ) {
                Ok(_) => {
                    durable.disarm();
                    context.report_candidate_evaluation_finished(evaluation_status);
                    WorkOutcome::Succeeded(TaskResult::Scan(Arc::new(ScanTaskResult::succeeded(
                        start.id().clone(),
                        start.started_at(),
                        completed_at,
                        public_counts(counts),
                        coverage,
                        evaluation_status,
                    ))))
                }
                Err(error) => settle_after_snapshot_error(error.kind, &mut durable, completed_at),
            }
        }
    }
}

fn settle_after_snapshot_error(
    error: SnapshotRepositoryErrorKind,
    durable: &mut DurableScanGuard,
    completed_at: SystemTime,
) -> WorkOutcome {
    match durable.store.load_scan(&durable.id) {
        Ok(Some(record)) if record.status() == ScanStatus::Running => {
            let result = durable.settle(
                TerminalScanStatus::Failed,
                ScanTaskStatus::Failed,
                completed_at,
                ScanCounts::default(),
                ScanCoverage::unknown(),
            );
            let failure = if matches!(
                error,
                SnapshotRepositoryErrorKind::Codec(_)
                    | SnapshotRepositoryErrorKind::ReferenceMismatch
            ) {
                TaskFailureKind::SnapshotRejected
            } else {
                TaskFailureKind::PersistenceUnavailable
            };
            failed_scan_outcome(failure, result)
        }
        Ok(Some(_)) => {
            durable.disarm();
            WorkOutcome::Failed(TaskFailureKind::PersistenceOutcomeUnknown, None)
        }
        Ok(None) | Err(_) => {
            durable.disarm();
            WorkOutcome::Failed(TaskFailureKind::PersistenceOutcomeUnknown, None)
        }
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
    let scan_scope = record.scan_scope.clone();
    record.scan_cancellation = None;
    record.result = None;
    record.failure = None;
    // Cancellation is intent, not evidence that completed work was rolled back.
    // The operation's outcome is authoritative after its own safe checkpoints.
    match outcome {
        Some(Ok(WorkOutcome::Succeeded(result))) => {
            record.phase = TaskPhase::Succeeded;
            record.result = Some(result);
        }
        Some(Ok(WorkOutcome::Cancelled(result))) => {
            record.phase = TaskPhase::Cancelled;
            record.result = result;
        }
        Some(Ok(WorkOutcome::Failed(failure, result))) => {
            record.phase = TaskPhase::Failed;
            record.failure = Some(failure);
            record.result = result;
        }
        Some(Err(_)) | None => {
            record.phase = TaskPhase::Failed;
            record.failure = Some(TaskFailureKind::InternalFailure);
        }
    }
    let phase = record.phase;
    record.push_event(TaskEventKind::Terminal { phase }, event_limit);
    if let Some(scope) = scan_scope
        && registry.active_scan_roots.get(&scope) == Some(&id)
    {
        registry.active_scan_roots.remove(&scope);
    }
    registry.running_tasks = registry.running_tasks.saturating_sub(1);
    registry.retain_terminal(id, shared.limits.retained_terminal_tasks);
    shared.lifecycle_changed.notify_all();
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;

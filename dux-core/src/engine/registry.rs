use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::candidate_history::{
    CandidateDetailError, CandidateReviewCommand, CandidateReviewError, CandidateReviewResult,
    DurableCandidateEvidence, DurableCandidateEvidenceItem, DurableCandidateEvidencePage,
    DurableCandidatePathItem, DurableCandidatePathPage, DurableObservedPath, DurablePathEncoding,
    MAX_CANDIDATE_DETAIL_PAGE_LIMIT,
};
use super::cleanup_history::{
    CleanupHistoryCursor, CleanupHistoryError, DurableCleanupErrorCategory,
    DurableCleanupHistoryPage, DurableCleanupItemStatus, DurableCleanupItemSummary,
    DurableCleanupMode, DurableCleanupRecordFormat, DurableCleanupSessionId,
    DurableCleanupSessionObservation, DurableCleanupSessionStatus, DurableCleanupSessionSummary,
    DurableCleanupStatusCounts, DurableCleanupTrigger, DurableCleanupWarning,
    MAX_RECENT_CLEANUP_HISTORY_LIMIT,
};
use super::config::EngineConfig;
use super::settings::{
    SnapshotRetentionCap, SnapshotRetentionCapError, SnapshotRetentionCapSource,
    SnapshotRetentionCapUpdate,
};
use super::task::{
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
    SnapshotRetentionStartOutcome, SnapshotTerminalTempMaintenanceFailureKind,
    SnapshotTerminalTempMaintenanceOutcome, SnapshotTerminalTempMaintenanceResult,
    SnapshotTerminalTempMaintenanceStartOutcome, SnapshotUnleasedTempMaintenanceFailureKind,
    SnapshotUnleasedTempMaintenanceOutcome, SnapshotUnleasedTempMaintenanceResult,
    SnapshotUnleasedTempMaintenanceStartOutcome, StartTaskError, TaskAccessError, TaskEvent,
    TaskEventBatch, TaskEventKind, TaskFailureKind, TaskId, TaskKind, TaskPhase, TaskSnapshot,
};
use crate::domain::{
    CANDIDATE_CATALOG_SCHEMA_VERSION, CANDIDATE_CATALOG_SHA256, CANDIDATE_CONTEXT_FORMAT_VERSION,
    CANDIDATE_EVALUATOR_REVISION, CandidateEvaluationError, CandidateId, Evidence, ScanCoverage,
    ScanId, candidate_evaluation_context_digest_sha256, evaluate_completed_scan_candidates,
    validate_bundled_candidate_catalog,
};
use crate::persistence::snapshot::from_scan::prepare_completed_scan;
use crate::persistence::snapshot::{
    HostValue, SnapshotCodecErrorKind, SnapshotOrphanReconciliationBatchOutcome,
    SnapshotRepository, SnapshotRepositoryErrorKind, SnapshotRetentionBatchOutcome,
    SnapshotStorageErrorKind, SnapshotStoreAccess, SnapshotTerminalTempReconciliationBatchOutcome,
    SnapshotUnleasedTempReconciliationBatchOutcome,
};
use crate::persistence::{
    CandidateEvaluationCompletion, CandidateEvaluationFailureKind, CandidateEvaluationIdentity,
    CandidateEvaluationObservation, CandidateEvaluationRecord, CandidateEvaluationStatus,
    CandidateHistoryStatus, CandidateReviewAction, CleanupSessionId, CompleteCandidateRecord,
    HistoryErrorKind, HostPathObservationEncoding, MAX_RECENT_SCAN_HISTORY_LIMIT,
    NewCandidateRecord, NewScanRecord, ScanCompletionRecord, ScanCounts, ScanStatus,
    StoredCleanupErrorCategory, StoredCleanupHistoryCursor, StoredCleanupHistoryObservation,
    StoredCleanupItemStatus, StoredCleanupItemSummary, StoredCleanupMode,
    StoredCleanupRecordFormat, StoredCleanupSessionStatus, StoredCleanupSessionSummary,
    StoredCleanupStatusCounts, StoredCleanupTrigger, TerminalScanStatus, observe_host_path,
};
#[cfg(test)]
use crate::persistence::{CleanupTrigger, NewCleanupSessionRecord, StoredCandidateRecord};
use crate::persistence::{DatabaseStatus, StoreCoordinator};
use crate::persistence::{
    SnapshotRetentionCapSetting, SnapshotRetentionCapSettingSource,
    SnapshotRetentionCapSettingUpdate,
};
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
    HistoryMaintenance(Arc<HistoryMaintenanceResult>),
    SnapshotRetention(Arc<SnapshotRetentionResult>),
    SnapshotOrphanMaintenance(Arc<SnapshotOrphanMaintenanceResult>),
    SnapshotTerminalTempMaintenance(Arc<SnapshotTerminalTempMaintenanceResult>),
    SnapshotUnleasedTempMaintenance(Arc<SnapshotUnleasedTempMaintenanceResult>),
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

    /// Atomically order cancellation against the maintenance point of no
    /// return. If cancellation wins the registry lock, no transaction starts;
    /// if this method wins, the Applying event records that later cancellation
    /// is intent and cannot rewrite a committed outcome.
    fn try_begin_history_maintenance_batch(&self) -> bool {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::HistoryMaintenance
            && record.phase == TaskPhase::Running
            && !record.cancellation_requested
        {
            record.push_event(TaskEventKind::HistoryMaintenanceBatchApplying, event_limit);
            true
        } else {
            false
        }
    }

    fn report_history_maintenance_finished(&self, kind: TaskEventKind) {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::HistoryMaintenance
            && !record.phase.is_terminal()
        {
            record.push_event(kind, event_limit);
        }
    }

    /// Order cancellation against the first repository operation that may
    /// durably tombstone or physically remove an exact DUX snapshot.
    fn try_begin_snapshot_retention_batch(&self) -> bool {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::SnapshotRetention
            && record.phase == TaskPhase::Running
            && !record.cancellation_requested
        {
            record.push_event(TaskEventKind::SnapshotRetentionBatchApplying, event_limit);
            true
        } else {
            false
        }
    }

    fn report_snapshot_retention_finished(&self, kind: TaskEventKind) {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::SnapshotRetention
            && !record.phase.is_terminal()
        {
            record.push_event(kind, event_limit);
        }
    }

    /// Order cancellation against the first repository operation that may
    /// physically remove a decoded DUX-owned orphan final.
    fn try_begin_snapshot_orphan_maintenance_batch(&self) -> bool {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::SnapshotOrphanMaintenance
            && record.phase == TaskPhase::Running
            && !record.cancellation_requested
        {
            record.push_event(
                TaskEventKind::SnapshotOrphanMaintenanceBatchApplying,
                event_limit,
            );
            true
        } else {
            false
        }
    }

    fn report_snapshot_orphan_maintenance_finished(&self, kind: TaskEventKind) {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::SnapshotOrphanMaintenance
            && !record.phase.is_terminal()
        {
            record.push_event(kind, event_limit);
        }
    }

    /// Order cancellation against the first repository operation that may
    /// remove a terminal row-bound temp or consume its exact durable lease.
    fn try_begin_snapshot_terminal_temp_maintenance_batch(&self) -> bool {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::SnapshotTerminalTempMaintenance
            && record.phase == TaskPhase::Running
            && !record.cancellation_requested
        {
            record.push_event(
                TaskEventKind::SnapshotTerminalTempMaintenanceBatchApplying,
                event_limit,
            );
            true
        } else {
            false
        }
    }

    fn report_snapshot_terminal_temp_maintenance_finished(&self, kind: TaskEventKind) {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::SnapshotTerminalTempMaintenance
            && !record.phase.is_terminal()
        {
            record.push_event(kind, event_limit);
        }
    }

    /// Order cancellation against the first repository operation that may
    /// remove an unleased, marker-owned snapshot temporary.
    fn try_begin_snapshot_unleased_temp_maintenance_batch(&self) -> bool {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::SnapshotUnleasedTempMaintenance
            && record.phase == TaskPhase::Running
            && !record.cancellation_requested
        {
            record.push_event(
                TaskEventKind::SnapshotUnleasedTempMaintenanceBatchApplying,
                event_limit,
            );
            true
        } else {
            false
        }
    }

    fn report_snapshot_unleased_temp_maintenance_finished(&self, kind: TaskEventKind) {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::SnapshotUnleasedTempMaintenance
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
    active_history_maintenance: Option<TaskId>,
    active_snapshot_retention: Option<TaskId>,
    active_snapshot_orphan_maintenance: Option<TaskId>,
    active_snapshot_terminal_temp_maintenance: Option<TaskId>,
    active_snapshot_unleased_temp_maintenance: Option<TaskId>,
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
            active_history_maintenance: None,
            active_snapshot_retention: None,
            active_snapshot_orphan_maintenance: None,
            active_snapshot_terminal_temp_maintenance: None,
            active_snapshot_unleased_temp_maintenance: None,
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

    fn release_task_exclusivity(&mut self, id: TaskId, kind: TaskKind, scan_scope: Option<&Path>) {
        if let Some(scope) = scan_scope
            && self.active_scan_roots.get(scope) == Some(&id)
        {
            self.active_scan_roots.remove(scope);
        }
        if kind == TaskKind::HistoryMaintenance && self.active_history_maintenance == Some(id) {
            self.active_history_maintenance = None;
        }
        if kind == TaskKind::SnapshotRetention && self.active_snapshot_retention == Some(id) {
            self.active_snapshot_retention = None;
        }
        if kind == TaskKind::SnapshotOrphanMaintenance
            && self.active_snapshot_orphan_maintenance == Some(id)
        {
            self.active_snapshot_orphan_maintenance = None;
        }
        if kind == TaskKind::SnapshotTerminalTempMaintenance
            && self.active_snapshot_terminal_temp_maintenance == Some(id)
        {
            self.active_snapshot_terminal_temp_maintenance = None;
        }
        if kind == TaskKind::SnapshotUnleasedTempMaintenance
            && self.active_snapshot_unleased_temp_maintenance == Some(id)
        {
            self.active_snapshot_unleased_temp_maintenance = None;
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
            let identity = registry
                .records
                .get(&id)
                .map(|record| (record.kind, record.scan_scope.clone()));
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
            if let Some((kind, scope)) = identity {
                registry.release_task_exclusivity(id, kind, scope.as_deref());
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

    /// Load the effective, versioned snapshot-store size cap.
    ///
    /// The result is policy metadata only. It cannot select or remove a
    /// snapshot, and a future retention writer must reread it under its final
    /// database-to-snapshot lock boundary.
    pub fn snapshot_retention_cap(
        &self,
    ) -> Result<SnapshotRetentionCap, SnapshotRetentionCapError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(SnapshotRetentionCapError::Closed);
        }
        self.inner
            .store
            .load_snapshot_retention_cap()
            .map(public_snapshot_retention_cap)
            .map_err(|error| map_snapshot_retention_cap_error(error.kind))
    }

    /// Store one explicit snapshot cap in the shared DUX settings database.
    pub fn set_snapshot_retention_cap(
        &self,
        cap_bytes: u64,
    ) -> Result<SnapshotRetentionCapUpdate, SnapshotRetentionCapError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(SnapshotRetentionCapError::Closed);
        }
        self.inner
            .store
            .set_snapshot_retention_cap(cap_bytes)
            .map(public_snapshot_retention_cap_update)
            .map_err(|error| map_snapshot_retention_cap_error(error.kind))
    }

    /// Remove the explicit override and restore the versioned core default.
    pub fn reset_snapshot_retention_cap(
        &self,
    ) -> Result<SnapshotRetentionCapUpdate, SnapshotRetentionCapError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(SnapshotRetentionCapError::Closed);
        }
        self.inner
            .store
            .reset_snapshot_retention_cap()
            .map(public_snapshot_retention_cap_update)
            .map_err(|error| map_snapshot_retention_cap_error(error.kind))
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

    /// Return one keyset-bounded page of path-free cleanup-session summaries.
    /// Historical policy and outcomes are presentation observations only and
    /// cannot be reused as current validation, approval, or effect authority.
    pub fn recent_cleanup_history(
        &self,
        cursor: Option<&CleanupHistoryCursor>,
        limit: u16,
    ) -> Result<DurableCleanupHistoryPage, CleanupHistoryError> {
        if !(1..=MAX_RECENT_CLEANUP_HISTORY_LIMIT).contains(&limit) {
            return Err(CleanupHistoryError::InvalidLimit {
                maximum: MAX_RECENT_CLEANUP_HISTORY_LIMIT,
            });
        }
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CleanupHistoryError::Closed);
        }
        let stored_cursor = cursor.map(stored_cleanup_history_cursor).transpose()?;
        let page = self
            .inner
            .store
            .recent_cleanup_history(stored_cursor.as_ref(), usize::from(limit))
            .map_err(|error| map_cleanup_history_error(error.kind))?;
        let records = page
            .records
            .into_iter()
            .map(public_cleanup_session_summary)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = page
            .next_cursor
            .map(public_cleanup_history_cursor)
            .transpose()?;
        DurableCleanupHistoryPage::new(records, next_cursor)
            .ok_or(CleanupHistoryError::InternalState)
    }

    /// Return one fully validated, path-free cleanup journal observation. The
    /// complete stored graph is checked internally, but paths, evidence,
    /// candidate identities, execution fences, and claims stay sealed.
    pub fn cleanup_session_history(
        &self,
        session_id: &DurableCleanupSessionId,
    ) -> Result<DurableCleanupSessionObservation, CleanupHistoryError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CleanupHistoryError::Closed);
        }
        let stored_id = CleanupSessionId::new(session_id.as_str().to_owned())
            .map_err(|_| CleanupHistoryError::InternalState)?;
        let observation = self
            .inner
            .store
            .cleanup_history_session(&stored_id)
            .map_err(|error| map_cleanup_history_error(error.kind))?
            .ok_or(CleanupHistoryError::SessionNotFound)?;
        public_cleanup_history_observation(observation)
    }

    /// Load one exact, bounded candidate-discovery observation from durable
    /// history. Returned summaries deliberately omit paths and evidence
    /// payloads and cannot be converted into a cleanup plan or effect.
    pub fn candidate_history_for_scan(
        &self,
        scan_id: &ScanId,
    ) -> Result<DurableCandidateEvaluation, CandidateHistoryError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CandidateHistoryError::Closed);
        }
        let observation = self
            .inner
            .store
            .load_candidate_evaluation_for_scan(scan_id)
            .map_err(|error| map_candidate_history_error(error.kind))?;
        public_candidate_history(scan_id, observation)
    }

    /// Return one bounded page of exact historical candidate paths. This is an
    /// explicit local disclosure for presentation, not a validation or cleanup
    /// capability.
    pub fn candidate_path_page(
        &self,
        scan_id: &ScanId,
        candidate_id: &CandidateId,
        cursor: u16,
        limit: u16,
    ) -> Result<DurableCandidatePathPage, CandidateDetailError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CandidateDetailError::Closed);
        }
        validate_candidate_detail_limit(limit)?;
        let record = self.candidate_record_for_detail(scan_id, candidate_id)?;
        let total_paths =
            u16::try_from(record.paths().len()).map_err(|_| CandidateDetailError::InternalState)?;
        let range = candidate_detail_range(cursor, limit, total_paths)?;
        let paths = record.paths()[range.clone()]
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let ordinal = range
                    .start
                    .checked_add(index)
                    .and_then(|value| u16::try_from(value).ok())
                    .ok_or(CandidateDetailError::InternalState)?;
                Ok(DurableCandidatePathItem::new(
                    ordinal,
                    public_observed_path(path)?,
                ))
            })
            .collect::<Result<Vec<_>, CandidateDetailError>>()?;
        let next_cursor = next_candidate_detail_cursor(&range, total_paths)?;
        let candidate = public_candidate_summary(scan_id, &record)
            .map_err(map_candidate_history_to_detail_error)?;
        Ok(DurableCandidatePathPage::new(
            scan_id.clone(),
            candidate,
            cursor,
            next_cursor,
            total_paths,
            paths,
        ))
    }

    /// Return one bounded page of typed historical discovery evidence. The
    /// DTO is intentionally distinct from planner evidence.
    pub fn candidate_evidence_page(
        &self,
        scan_id: &ScanId,
        candidate_id: &CandidateId,
        cursor: u16,
        limit: u16,
    ) -> Result<DurableCandidateEvidencePage, CandidateDetailError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CandidateDetailError::Closed);
        }
        validate_candidate_detail_limit(limit)?;
        let record = self.candidate_record_for_detail(scan_id, candidate_id)?;
        let total_evidence = u16::try_from(record.evidence().len())
            .map_err(|_| CandidateDetailError::InternalState)?;
        let range = candidate_detail_range(cursor, limit, total_evidence)?;
        let evidence = record.evidence()[range.clone()]
            .iter()
            .enumerate()
            .map(|(index, evidence)| {
                let ordinal = range
                    .start
                    .checked_add(index)
                    .and_then(|value| u16::try_from(value).ok())
                    .ok_or(CandidateDetailError::InternalState)?;
                Ok(DurableCandidateEvidenceItem::new(
                    ordinal,
                    public_candidate_evidence(evidence)?,
                ))
            })
            .collect::<Result<Vec<_>, CandidateDetailError>>()?;
        let next_cursor = next_candidate_detail_cursor(&range, total_evidence)?;
        let candidate = public_candidate_summary(scan_id, &record)
            .map_err(map_candidate_history_to_detail_error)?;
        Ok(DurableCandidateEvidencePage::new(
            scan_id.clone(),
            candidate,
            cursor,
            next_cursor,
            total_evidence,
            evidence,
        ))
    }

    /// Apply one semantic, scan-bound review command. Selection is user intent
    /// only and cannot create a plan or authorize an effect.
    pub fn review_candidate(
        &self,
        scan_id: &ScanId,
        candidate_id: &CandidateId,
        command: CandidateReviewCommand,
    ) -> Result<CandidateReviewResult, CandidateReviewError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CandidateReviewError::Closed);
        }
        let action = match command {
            CandidateReviewCommand::Select => CandidateReviewAction::Select,
            CandidateReviewCommand::ClearSelection => CandidateReviewAction::ClearSelection,
            CandidateReviewCommand::Dismiss => CandidateReviewAction::Dismiss,
            CandidateReviewCommand::Restore => CandidateReviewAction::Restore,
        };
        let status = self
            .inner
            .store
            .review_candidate(scan_id, candidate_id, action)
            .map_err(|error| map_candidate_review_error(error.kind))?;
        Ok(CandidateReviewResult::new(
            scan_id.clone(),
            candidate_id.clone(),
            public_candidate_status(status),
        ))
    }

    fn candidate_record_for_detail(
        &self,
        scan_id: &ScanId,
        candidate_id: &CandidateId,
    ) -> Result<CompleteCandidateRecord, CandidateDetailError> {
        let observation = self
            .inner
            .store
            .load_candidate_evaluation_for_scan(scan_id)
            .map_err(|error| map_candidate_detail_error(error.kind))?;
        match observation {
            CandidateEvaluationObservation::MissingScan => Err(CandidateDetailError::ScanNotFound),
            CandidateEvaluationObservation::NotRun { .. }
            | CandidateEvaluationObservation::Pending(_)
            | CandidateEvaluationObservation::Failed(_) => {
                Err(CandidateDetailError::EvaluationNotSucceeded)
            }
            CandidateEvaluationObservation::Succeeded(record) => record
                .candidates()
                .iter()
                .find(|candidate| candidate.id() == candidate_id)
                .cloned()
                .ok_or(CandidateDetailError::CandidateNotFound),
        }
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

    /// Start one bounded DUX-owned history-maintenance batch. This can roll up
    /// and prune capacity telemetry and remove expired AI cache rows; it never
    /// mutates cleanup history or user data. A successful result's `has_more`
    /// flag lets an idle caller enqueue a later batch without monopolizing a
    /// worker or writer lease.
    pub fn start_history_maintenance(
        &self,
    ) -> Result<HistoryMaintenanceStartOutcome, StartTaskError> {
        self.start_history_maintenance_with_hooks(SystemTime::now, || {}, || {})
    }

    #[cfg(test)]
    fn start_history_maintenance_at(
        &self,
        observed_at: SystemTime,
    ) -> Result<HistoryMaintenanceStartOutcome, StartTaskError> {
        self.start_history_maintenance_with_hooks(move || observed_at, || {}, || {})
    }

    #[cfg(test)]
    fn start_history_maintenance_with_test_hooks(
        &self,
        observed_at: SystemTime,
        before_batch: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<HistoryMaintenanceStartOutcome, StartTaskError> {
        self.start_history_maintenance_with_hooks(move || observed_at, before_batch, after_batch)
    }

    fn start_history_maintenance_with_hooks(
        &self,
        clock: impl FnOnce() -> SystemTime + Send + 'static,
        before_batch: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<HistoryMaintenanceStartOutcome, StartTaskError> {
        if let Some(outcome) = self.history_maintenance_preflight()? {
            return Ok(outcome);
        }
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
        self.submit_history_maintenance(Box::new(move |context| {
            if context.is_cancellation_requested() {
                return WorkOutcome::Cancelled(None);
            }
            let observed_at = clock();
            before_batch();
            if !context.try_begin_history_maintenance_batch() {
                return WorkOutcome::Cancelled(None);
            }
            match store.run_history_retention_batch(observed_at) {
                Ok(result) => {
                    let result = Arc::new(HistoryMaintenanceResult::new(
                        result.observed_at,
                        result.daily_rollups_created,
                        result.raw_samples_pruned,
                        result.daily_rollups_pruned,
                        result.ai_insights_pruned,
                        result.has_more,
                    ));
                    // The transaction has committed. Cancellation requested
                    // from this point onward remains intent and cannot
                    // truthfully rewrite the successful durable outcome.
                    after_batch();
                    context.report_history_maintenance_finished(
                        TaskEventKind::HistoryMaintenanceBatchFinished {
                            daily_rollups_created: result.daily_rollups_created(),
                            raw_samples_pruned: result.raw_samples_pruned(),
                            daily_rollups_pruned: result.daily_rollups_pruned(),
                            ai_insights_pruned: result.ai_insights_pruned(),
                            has_more: result.has_more(),
                        },
                    );
                    WorkOutcome::Succeeded(TaskResult::HistoryMaintenance(result))
                }
                Err(error) => {
                    WorkOutcome::Failed(map_history_maintenance_failure(error.kind), None)
                }
            }
        }))
    }

    /// Avoid touching SQLite for requests that are already known to be closed,
    /// duplicate, or non-idle. Submission repeats these checks after the
    /// compatibility probe so a foreground task cannot race maintenance into
    /// a stale eligibility decision.
    fn history_maintenance_preflight(
        &self,
    ) -> Result<Option<HistoryMaintenanceStartOutcome>, StartTaskError> {
        let registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_history_maintenance {
            return Ok(Some(HistoryMaintenanceStartOutcome::AlreadyActive(
                existing,
            )));
        }
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(Some(HistoryMaintenanceStartOutcome::DeferredBusy));
        }
        Ok(None)
    }

    /// Start one bounded snapshot-cap enforcement decision. The repository
    /// repeats settings, pin, temp, history, identity, and usage validation
    /// under its final database-to-snapshot lock boundary and removes at most
    /// one DUX-owned final. `has_more` is only a later idle-rescheduling hint.
    pub fn start_snapshot_retention(
        &self,
    ) -> Result<SnapshotRetentionStartOutcome, StartTaskError> {
        self.start_snapshot_retention_with_hooks(SystemTime::now, || {}, || {}, || {})
    }

    #[cfg(test)]
    fn start_snapshot_retention_at(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotRetentionStartOutcome, StartTaskError> {
        self.start_snapshot_retention_with_hooks(move || observed_at, || {}, || {}, || {})
    }

    #[cfg(test)]
    fn start_snapshot_retention_with_test_hooks(
        &self,
        observed_at: SystemTime,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<SnapshotRetentionStartOutcome, StartTaskError> {
        self.start_snapshot_retention_with_hooks(
            move || observed_at,
            before_batch,
            after_applying,
            after_batch,
        )
    }

    fn start_snapshot_retention_with_hooks(
        &self,
        clock: impl FnOnce() -> SystemTime + Send + 'static,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<SnapshotRetentionStartOutcome, StartTaskError> {
        if let Some(outcome) = self.snapshot_retention_preflight()? {
            return Ok(outcome);
        }
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
        let snapshots = Arc::clone(&self.inner.snapshots);
        self.submit_snapshot_retention(Box::new(move |context| {
            if context.is_cancellation_requested() {
                return WorkOutcome::Cancelled(None);
            }
            let observed_at = clock();
            before_batch();
            if !context.try_begin_snapshot_retention_batch() {
                return WorkOutcome::Cancelled(None);
            }
            // Applying is the point of no return. Cancellation after this
            // point remains intent and cannot suppress or rewrite the exact
            // repository outcome.
            after_applying();
            match snapshots.enforce_retention_cap(observed_at) {
                Ok(result) => {
                    let outcome = public_snapshot_retention_outcome(&result.outcome);
                    let result = Arc::new(SnapshotRetentionResult::new(
                        result.observed_at,
                        outcome,
                        result.cap_bytes,
                        result.charged_bytes_before,
                        result.charged_bytes_after,
                        result.has_more,
                    ));
                    after_batch();
                    context.report_snapshot_retention_finished(
                        TaskEventKind::SnapshotRetentionBatchFinished {
                            outcome: result.outcome(),
                            cap_bytes: result.cap_bytes(),
                            charged_bytes_before: result.charged_bytes_before(),
                            charged_bytes_after: result.charged_bytes_after(),
                            has_more: result.has_more(),
                        },
                    );
                    WorkOutcome::Succeeded(TaskResult::SnapshotRetention(result))
                }
                Err(error) => WorkOutcome::Failed(map_snapshot_retention_failure(error.kind), None),
            }
        }))
    }

    fn snapshot_retention_preflight(
        &self,
    ) -> Result<Option<SnapshotRetentionStartOutcome>, StartTaskError> {
        let registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_snapshot_retention {
            return Ok(Some(SnapshotRetentionStartOutcome::AlreadyActive(existing)));
        }
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(Some(SnapshotRetentionStartOutcome::DeferredBusy));
        }
        Ok(None)
    }

    /// Reconcile at most one decoded DUX-owned snapshot final whose durable
    /// scan parent has no snapshot reference. Exact orphan identity stays
    /// inside persistence; `has_more` is only a later idle-rescheduling hint.
    pub fn start_snapshot_orphan_maintenance(
        &self,
    ) -> Result<SnapshotOrphanMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_orphan_maintenance_with_hooks(SystemTime::now, || {}, || {}, || {})
    }

    #[cfg(test)]
    fn start_snapshot_orphan_maintenance_at(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotOrphanMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_orphan_maintenance_with_hooks(move || observed_at, || {}, || {}, || {})
    }

    #[cfg(test)]
    fn start_snapshot_orphan_maintenance_with_test_hooks(
        &self,
        observed_at: SystemTime,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<SnapshotOrphanMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_orphan_maintenance_with_hooks(
            move || observed_at,
            before_batch,
            after_applying,
            after_batch,
        )
    }

    fn start_snapshot_orphan_maintenance_with_hooks(
        &self,
        clock: impl FnOnce() -> SystemTime + Send + 'static,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<SnapshotOrphanMaintenanceStartOutcome, StartTaskError> {
        if let Some(outcome) = self.snapshot_orphan_maintenance_preflight()? {
            return Ok(outcome);
        }
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
        let snapshots = Arc::clone(&self.inner.snapshots);
        self.submit_snapshot_orphan_maintenance(Box::new(move |context| {
            if context.is_cancellation_requested() {
                return WorkOutcome::Cancelled(None);
            }
            let observed_at = clock();
            before_batch();
            if !context.try_begin_snapshot_orphan_maintenance_batch() {
                return WorkOutcome::Cancelled(None);
            }
            // Applying is the point of no return. Later cancellation remains
            // visible intent but cannot suppress an exact repository result.
            after_applying();
            match snapshots.reconcile_physical_orphan(observed_at) {
                Ok(result) => {
                    let outcome = public_snapshot_orphan_maintenance_outcome(&result.outcome);
                    let result = Arc::new(SnapshotOrphanMaintenanceResult::new(
                        result.observed_at,
                        outcome,
                        result.orphan_count_before,
                        result.orphan_count_after,
                        result.orphan_charged_bytes_before,
                        result.orphan_charged_bytes_after,
                        result.has_more,
                    ));
                    after_batch();
                    context.report_snapshot_orphan_maintenance_finished(
                        TaskEventKind::SnapshotOrphanMaintenanceBatchFinished {
                            outcome: result.outcome(),
                            orphan_count_before: result.orphan_count_before(),
                            orphan_count_after: result.orphan_count_after(),
                            orphan_charged_bytes_before: result.orphan_charged_bytes_before(),
                            orphan_charged_bytes_after: result.orphan_charged_bytes_after(),
                            has_more: result.has_more(),
                        },
                    );
                    WorkOutcome::Succeeded(TaskResult::SnapshotOrphanMaintenance(result))
                }
                Err(error) => {
                    WorkOutcome::Failed(map_snapshot_orphan_maintenance_failure(error.kind), None)
                }
            }
        }))
    }

    fn snapshot_orphan_maintenance_preflight(
        &self,
    ) -> Result<Option<SnapshotOrphanMaintenanceStartOutcome>, StartTaskError> {
        let registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_snapshot_orphan_maintenance {
            return Ok(Some(SnapshotOrphanMaintenanceStartOutcome::AlreadyActive(
                existing,
            )));
        }
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(Some(SnapshotOrphanMaintenanceStartOutcome::DeferredBusy));
        }
        Ok(None)
    }

    /// Reconcile at most one terminal row-bound snapshot temporary. Exact
    /// scan, lease, owner, and filename identity stays inside persistence;
    /// `has_more` is only a later idle-rescheduling hint.
    pub fn start_snapshot_terminal_temp_maintenance(
        &self,
    ) -> Result<SnapshotTerminalTempMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_terminal_temp_maintenance_with_hooks(
            SystemTime::now,
            || {},
            || {},
            || {},
        )
    }

    #[cfg(test)]
    fn start_snapshot_terminal_temp_maintenance_at(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotTerminalTempMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_terminal_temp_maintenance_with_hooks(
            move || observed_at,
            || {},
            || {},
            || {},
        )
    }

    #[cfg(test)]
    fn start_snapshot_terminal_temp_maintenance_with_test_hooks(
        &self,
        observed_at: SystemTime,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<SnapshotTerminalTempMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_terminal_temp_maintenance_with_hooks(
            move || observed_at,
            before_batch,
            after_applying,
            after_batch,
        )
    }

    fn start_snapshot_terminal_temp_maintenance_with_hooks(
        &self,
        clock: impl FnOnce() -> SystemTime + Send + 'static,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<SnapshotTerminalTempMaintenanceStartOutcome, StartTaskError> {
        if let Some(outcome) = self.snapshot_terminal_temp_maintenance_preflight()? {
            return Ok(outcome);
        }
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
        let snapshots = Arc::clone(&self.inner.snapshots);
        self.submit_snapshot_terminal_temp_maintenance(Box::new(move |context| {
            if context.is_cancellation_requested() {
                return WorkOutcome::Cancelled(None);
            }
            let observed_at = clock();
            before_batch();
            if !context.try_begin_snapshot_terminal_temp_maintenance_batch() {
                return WorkOutcome::Cancelled(None);
            }
            // Applying is the point of no return. Later cancellation remains
            // visible intent but cannot suppress an exact repository result.
            after_applying();
            match snapshots.reconcile_terminal_snapshot_temp_residual(observed_at) {
                Ok(result) => {
                    let outcome =
                        public_snapshot_terminal_temp_maintenance_outcome(&result.outcome);
                    let result = Arc::new(SnapshotTerminalTempMaintenanceResult::new(
                        result.observed_at,
                        outcome,
                        result.terminal_lease_count_before,
                        result.terminal_lease_count_after,
                        result.active_terminal_lease_count_before,
                        result.active_terminal_lease_count_after,
                        result.terminal_charged_bytes_before,
                        result.terminal_charged_bytes_after,
                        result.has_more,
                    ));
                    after_batch();
                    context.report_snapshot_terminal_temp_maintenance_finished(
                        TaskEventKind::SnapshotTerminalTempMaintenanceBatchFinished {
                            outcome: result.outcome(),
                            terminal_lease_count_before: result.terminal_lease_count_before(),
                            terminal_lease_count_after: result.terminal_lease_count_after(),
                            active_terminal_lease_count_before: result
                                .active_terminal_lease_count_before(),
                            active_terminal_lease_count_after: result
                                .active_terminal_lease_count_after(),
                            terminal_charged_bytes_before: result.terminal_charged_bytes_before(),
                            terminal_charged_bytes_after: result.terminal_charged_bytes_after(),
                            has_more: result.has_more(),
                        },
                    );
                    WorkOutcome::Succeeded(TaskResult::SnapshotTerminalTempMaintenance(result))
                }
                Err(error) => WorkOutcome::Failed(
                    map_snapshot_terminal_temp_maintenance_failure(error.kind),
                    None,
                ),
            }
        }))
    }

    fn snapshot_terminal_temp_maintenance_preflight(
        &self,
    ) -> Result<Option<SnapshotTerminalTempMaintenanceStartOutcome>, StartTaskError> {
        let registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_snapshot_terminal_temp_maintenance {
            return Ok(Some(
                SnapshotTerminalTempMaintenanceStartOutcome::AlreadyActive(existing),
            ));
        }
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(Some(
                SnapshotTerminalTempMaintenanceStartOutcome::DeferredBusy,
            ));
        }
        Ok(None)
    }

    /// Reconcile at most one recognized unleased snapshot temporary. Exact
    /// filename identity stays inside persistence; `has_more` is only a later
    /// idle-rescheduling hint.
    pub fn start_snapshot_unleased_temp_maintenance(
        &self,
    ) -> Result<SnapshotUnleasedTempMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_unleased_temp_maintenance_with_hooks(
            SystemTime::now,
            || {},
            || {},
            || {},
        )
    }

    #[cfg(test)]
    fn start_snapshot_unleased_temp_maintenance_at(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotUnleasedTempMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_unleased_temp_maintenance_with_hooks(
            move || observed_at,
            || {},
            || {},
            || {},
        )
    }

    #[cfg(test)]
    fn start_snapshot_unleased_temp_maintenance_with_test_hooks(
        &self,
        observed_at: SystemTime,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<SnapshotUnleasedTempMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_unleased_temp_maintenance_with_hooks(
            move || observed_at,
            before_batch,
            after_applying,
            after_batch,
        )
    }

    fn start_snapshot_unleased_temp_maintenance_with_hooks(
        &self,
        clock: impl FnOnce() -> SystemTime + Send + 'static,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<SnapshotUnleasedTempMaintenanceStartOutcome, StartTaskError> {
        if let Some(outcome) = self.snapshot_unleased_temp_maintenance_preflight()? {
            return Ok(outcome);
        }
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
        let snapshots = Arc::clone(&self.inner.snapshots);
        self.submit_snapshot_unleased_temp_maintenance(Box::new(move |context| {
            if context.is_cancellation_requested() {
                return WorkOutcome::Cancelled(None);
            }
            let observed_at = clock();
            before_batch();
            if !context.try_begin_snapshot_unleased_temp_maintenance_batch() {
                return WorkOutcome::Cancelled(None);
            }
            // Applying is the point of no return. Later cancellation remains
            // visible intent but cannot suppress an exact repository result.
            after_applying();
            match snapshots.reconcile_unleased_snapshot_temp(observed_at) {
                Ok(result) => {
                    let outcome =
                        public_snapshot_unleased_temp_maintenance_outcome(&result.outcome);
                    let result = Arc::new(SnapshotUnleasedTempMaintenanceResult::new(
                        result.observed_at,
                        outcome,
                        result.unleased_temp_count_before,
                        result.unleased_temp_count_after,
                        result.active_unleased_temp_count_before,
                        result.active_unleased_temp_count_after,
                        result.unleased_charged_bytes_before,
                        result.unleased_charged_bytes_after,
                        result.has_more,
                    ));
                    after_batch();
                    context.report_snapshot_unleased_temp_maintenance_finished(
                        TaskEventKind::SnapshotUnleasedTempMaintenanceBatchFinished {
                            outcome: result.outcome(),
                            unleased_temp_count_before: result.unleased_temp_count_before(),
                            unleased_temp_count_after: result.unleased_temp_count_after(),
                            active_unleased_temp_count_before: result
                                .active_unleased_temp_count_before(),
                            active_unleased_temp_count_after: result
                                .active_unleased_temp_count_after(),
                            unleased_charged_bytes_before: result.unleased_charged_bytes_before(),
                            unleased_charged_bytes_after: result.unleased_charged_bytes_after(),
                            has_more: result.has_more(),
                        },
                    );
                    WorkOutcome::Succeeded(TaskResult::SnapshotUnleasedTempMaintenance(result))
                }
                Err(error) => WorkOutcome::Failed(
                    map_snapshot_unleased_temp_maintenance_failure(error.kind),
                    None,
                ),
            }
        }))
    }

    fn snapshot_unleased_temp_maintenance_preflight(
        &self,
    ) -> Result<Option<SnapshotUnleasedTempMaintenanceStartOutcome>, StartTaskError> {
        let registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_snapshot_unleased_temp_maintenance {
            return Ok(Some(
                SnapshotUnleasedTempMaintenanceStartOutcome::AlreadyActive(existing),
            ));
        }
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(Some(
                SnapshotUnleasedTempMaintenanceStartOutcome::DeferredBusy,
            ));
        }
        Ok(None)
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
            Some(
                TaskResult::Scan(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_),
            ) => {
                return Err(TaskAccessError::WrongTaskKind);
            }
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
            Some(
                TaskResult::FormatSizeBatch(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_),
            ) => {
                return Err(TaskAccessError::WrongTaskKind);
            }
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn history_maintenance_result(
        &self,
        id: TaskId,
    ) -> Result<Option<Arc<HistoryMaintenanceResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::HistoryMaintenance {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::HistoryMaintenance(result)) => Some(Arc::clone(result)),
            Some(
                TaskResult::FormatSizeBatch(_)
                | TaskResult::Scan(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_),
            ) => {
                return Err(TaskAccessError::WrongTaskKind);
            }
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn snapshot_retention_result(
        &self,
        id: TaskId,
    ) -> Result<Option<Arc<SnapshotRetentionResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::SnapshotRetention {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::SnapshotRetention(result)) => Some(Arc::clone(result)),
            Some(
                TaskResult::FormatSizeBatch(_)
                | TaskResult::Scan(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_),
            ) => return Err(TaskAccessError::WrongTaskKind),
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn snapshot_orphan_maintenance_result(
        &self,
        id: TaskId,
    ) -> Result<Option<Arc<SnapshotOrphanMaintenanceResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::SnapshotOrphanMaintenance {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::SnapshotOrphanMaintenance(result)) => Some(Arc::clone(result)),
            Some(
                TaskResult::FormatSizeBatch(_)
                | TaskResult::Scan(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_),
            ) => return Err(TaskAccessError::WrongTaskKind),
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn snapshot_terminal_temp_maintenance_result(
        &self,
        id: TaskId,
    ) -> Result<Option<Arc<SnapshotTerminalTempMaintenanceResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::SnapshotTerminalTempMaintenance {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::SnapshotTerminalTempMaintenance(result)) => Some(Arc::clone(result)),
            Some(
                TaskResult::FormatSizeBatch(_)
                | TaskResult::Scan(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_),
            ) => return Err(TaskAccessError::WrongTaskKind),
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn snapshot_unleased_temp_maintenance_result(
        &self,
        id: TaskId,
    ) -> Result<Option<Arc<SnapshotUnleasedTempMaintenanceResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::SnapshotUnleasedTempMaintenance {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::SnapshotUnleasedTempMaintenance(result)) => Some(Arc::clone(result)),
            Some(
                TaskResult::FormatSizeBatch(_)
                | TaskResult::Scan(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_),
            ) => return Err(TaskAccessError::WrongTaskKind),
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
                let identity = registry
                    .records
                    .get(&id)
                    .map(|record| (record.kind, record.scan_scope.clone()));
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
                if let Some((kind, scope)) = identity {
                    registry.release_task_exclusivity(id, kind, scope.as_deref());
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

    fn submit_history_maintenance(
        &self,
        work: Work,
    ) -> Result<HistoryMaintenanceStartOutcome, StartTaskError> {
        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_history_maintenance {
            return Ok(HistoryMaintenanceStartOutcome::AlreadyActive(existing));
        }
        // Retention is the lowest-priority engine work. Admit it only at an
        // observed idle boundary; foreground work submitted afterward may run
        // concurrently, but cannot be preempted into an in-flight transaction.
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(HistoryMaintenanceStartOutcome::DeferredBusy);
        }
        let id = TASK_IDS.allocate()?;
        let record = TaskRecord::new(
            id,
            TaskKind::HistoryMaintenance,
            None,
            self.inner.shared.limits.events_per_task,
        );
        registry.active_history_maintenance = Some(id);
        registry.records.insert(id, record);
        registry.queue.push_back(Job { id, work });
        self.inner.shared.workers_ready.notify_one();
        Ok(HistoryMaintenanceStartOutcome::Started(id))
    }

    fn submit_snapshot_retention(
        &self,
        work: Work,
    ) -> Result<SnapshotRetentionStartOutcome, StartTaskError> {
        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_snapshot_retention {
            return Ok(SnapshotRetentionStartOutcome::AlreadyActive(existing));
        }
        // Snapshot retention shares the lowest-priority idle boundary with
        // history maintenance. The queued/running check also prevents the two
        // maintenance classes from being active in one engine session.
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(SnapshotRetentionStartOutcome::DeferredBusy);
        }
        let id = TASK_IDS.allocate()?;
        let record = TaskRecord::new(
            id,
            TaskKind::SnapshotRetention,
            None,
            self.inner.shared.limits.events_per_task,
        );
        registry.active_snapshot_retention = Some(id);
        registry.records.insert(id, record);
        registry.queue.push_back(Job { id, work });
        self.inner.shared.workers_ready.notify_one();
        Ok(SnapshotRetentionStartOutcome::Started(id))
    }

    fn submit_snapshot_orphan_maintenance(
        &self,
        work: Work,
    ) -> Result<SnapshotOrphanMaintenanceStartOutcome, StartTaskError> {
        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_snapshot_orphan_maintenance {
            return Ok(SnapshotOrphanMaintenanceStartOutcome::AlreadyActive(
                existing,
            ));
        }
        // Physical-orphan reconciliation shares the same idle-only boundary
        // as both retention classes. Repeating this after the store status
        // probe closes the admission race with foreground and maintenance work.
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(SnapshotOrphanMaintenanceStartOutcome::DeferredBusy);
        }
        let id = TASK_IDS.allocate()?;
        let record = TaskRecord::new(
            id,
            TaskKind::SnapshotOrphanMaintenance,
            None,
            self.inner.shared.limits.events_per_task,
        );
        registry.active_snapshot_orphan_maintenance = Some(id);
        registry.records.insert(id, record);
        registry.queue.push_back(Job { id, work });
        self.inner.shared.workers_ready.notify_one();
        Ok(SnapshotOrphanMaintenanceStartOutcome::Started(id))
    }

    fn submit_snapshot_terminal_temp_maintenance(
        &self,
        work: Work,
    ) -> Result<SnapshotTerminalTempMaintenanceStartOutcome, StartTaskError> {
        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_snapshot_terminal_temp_maintenance {
            return Ok(SnapshotTerminalTempMaintenanceStartOutcome::AlreadyActive(
                existing,
            ));
        }
        // Terminal-temp reconciliation shares the same idle-only boundary as
        // every other maintenance class. Rechecking here closes the admission
        // race between the compatibility probe and foreground submission.
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(SnapshotTerminalTempMaintenanceStartOutcome::DeferredBusy);
        }
        let id = TASK_IDS.allocate()?;
        let record = TaskRecord::new(
            id,
            TaskKind::SnapshotTerminalTempMaintenance,
            None,
            self.inner.shared.limits.events_per_task,
        );
        registry.active_snapshot_terminal_temp_maintenance = Some(id);
        registry.records.insert(id, record);
        registry.queue.push_back(Job { id, work });
        self.inner.shared.workers_ready.notify_one();
        Ok(SnapshotTerminalTempMaintenanceStartOutcome::Started(id))
    }

    fn submit_snapshot_unleased_temp_maintenance(
        &self,
        work: Work,
    ) -> Result<SnapshotUnleasedTempMaintenanceStartOutcome, StartTaskError> {
        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_snapshot_unleased_temp_maintenance {
            return Ok(SnapshotUnleasedTempMaintenanceStartOutcome::AlreadyActive(
                existing,
            ));
        }
        // Unleased-temp reconciliation shares the same idle-only boundary as
        // every other maintenance class. Rechecking here closes the admission
        // race between the compatibility probe and foreground submission.
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(SnapshotUnleasedTempMaintenanceStartOutcome::DeferredBusy);
        }
        let id = TASK_IDS.allocate()?;
        let record = TaskRecord::new(
            id,
            TaskKind::SnapshotUnleasedTempMaintenance,
            None,
            self.inner.shared.limits.events_per_task,
        );
        registry.active_snapshot_unleased_temp_maintenance = Some(id);
        registry.records.insert(id, record);
        registry.queue.push_back(Job { id, work });
        self.inner.shared.workers_ready.notify_one();
        Ok(SnapshotUnleasedTempMaintenanceStartOutcome::Started(id))
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
    complete_candidate_evaluation(identity, evaluated_at, candidates)
}

fn complete_candidate_evaluation(
    identity: CandidateEvaluationIdentity,
    evaluated_at: SystemTime,
    candidates: Vec<NewCandidateRecord>,
) -> Result<
    (
        CandidateEvaluationIdentity,
        CandidateEvaluationCompletion,
        CandidateEvaluationTaskStatus,
    ),
    TaskFailureKind,
> {
    if !CandidateEvaluationCompletion::batch_fits_materialization_budget(&candidates)
        .map_err(|_| TaskFailureKind::InternalFailure)?
    {
        return failed_candidate_evaluation(
            identity,
            evaluated_at,
            CandidateEvaluationFailureKind::LimitExceeded,
        );
    }
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

fn stored_cleanup_history_cursor(
    cursor: &CleanupHistoryCursor,
) -> Result<StoredCleanupHistoryCursor, CleanupHistoryError> {
    let milliseconds = cursor
        .started_at()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CleanupHistoryError::InternalState)?
        .as_millis();
    let started_at_unix_ms =
        i64::try_from(milliseconds).map_err(|_| CleanupHistoryError::InternalState)?;
    let session_id = CleanupSessionId::new(cursor.session_id().as_str().to_owned())
        .map_err(|_| CleanupHistoryError::InternalState)?;
    StoredCleanupHistoryCursor::try_new(started_at_unix_ms, session_id)
        .map_err(|_| CleanupHistoryError::InternalState)
}

fn public_cleanup_history_cursor(
    cursor: StoredCleanupHistoryCursor,
) -> Result<CleanupHistoryCursor, CleanupHistoryError> {
    let milliseconds =
        u64::try_from(cursor.started_at_unix_ms).map_err(|_| CleanupHistoryError::CorruptData)?;
    let started_at = UNIX_EPOCH
        .checked_add(Duration::from_millis(milliseconds))
        .ok_or(CleanupHistoryError::CorruptData)?;
    let session_id = DurableCleanupSessionId::new(cursor.session_id.as_str().to_owned())
        .ok_or(CleanupHistoryError::CorruptData)?;
    Ok(CleanupHistoryCursor::new(started_at, session_id))
}

fn public_cleanup_session_summary(
    summary: StoredCleanupSessionSummary,
) -> Result<DurableCleanupSessionSummary, CleanupHistoryError> {
    let id = DurableCleanupSessionId::new(summary.session_id.as_str().to_owned())
        .ok_or(CleanupHistoryError::CorruptData)?;
    let format = match summary.format {
        StoredCleanupRecordFormat::LegacyIncomplete => DurableCleanupRecordFormat::LegacyIncomplete,
        StoredCleanupRecordFormat::CompleteV2 => DurableCleanupRecordFormat::Complete,
    };
    DurableCleanupSessionSummary::new(
        id,
        summary.plan_id.as_str().to_owned(),
        format,
        summary.source_scan_id,
        summary.started_at,
        summary.completed_at,
        summary.plan_created_at,
        summary.plan_expires_at,
        public_cleanup_mode(summary.mode),
        public_cleanup_trigger(summary.trigger),
        public_cleanup_session_status(summary.status),
        summary.estimated_bytes,
        summary.verified_capacity_delta_bytes,
        summary.cancellation_requested,
        summary.item_total,
        summary.path_total,
        summary.evidence_total,
        public_cleanup_status_counts(summary.item_status_counts)?,
        public_cleanup_status_counts(summary.path_status_counts)?,
    )
    .ok_or(CleanupHistoryError::CorruptData)
}

fn public_cleanup_history_observation(
    observation: StoredCleanupHistoryObservation,
) -> Result<DurableCleanupSessionObservation, CleanupHistoryError> {
    let format = observation.summary.format;
    let summary = public_cleanup_session_summary(observation.summary)?;
    let items = observation
        .items
        .into_iter()
        .map(|item| public_cleanup_item_summary(item, format))
        .collect::<Result<Vec<_>, _>>()?;
    let warnings = observation
        .warnings
        .into_iter()
        .map(public_cleanup_warning)
        .collect();
    DurableCleanupSessionObservation::new(summary, items, warnings)
        .ok_or(CleanupHistoryError::CorruptData)
}

fn public_cleanup_item_summary(
    item: StoredCleanupItemSummary,
    format: StoredCleanupRecordFormat,
) -> Result<DurableCleanupItemSummary, CleanupHistoryError> {
    let has_complete_policy = item.category.is_some()
        && item.safety.is_some()
        && item.action.is_some()
        && item.rule_schedule_eligible.is_some();
    if (format == StoredCleanupRecordFormat::CompleteV2) != has_complete_policy
        || (format == StoredCleanupRecordFormat::LegacyIncomplete
            && (item.newest_mtime.is_some()
                || item.evidence_count != 0
                || item.error_category.is_some()))
    {
        return Err(CleanupHistoryError::CorruptData);
    }
    let error_category = item
        .error_category
        .map(public_cleanup_error_category)
        .transpose()?;
    DurableCleanupItemSummary::new(
        u16::try_from(item.ordinal).map_err(|_| CleanupHistoryError::CorruptData)?,
        item.rule,
        item.category,
        item.safety,
        item.action,
        item.rule_schedule_eligible,
        item.newest_mtime,
        item.estimated_bytes,
        public_cleanup_item_status(item.status),
        item.error_recorded,
        error_category,
        item.path_count,
        item.evidence_count,
    )
    .ok_or(CleanupHistoryError::CorruptData)
}

fn public_cleanup_error_category(
    category: StoredCleanupErrorCategory,
) -> Result<DurableCleanupErrorCategory, CleanupHistoryError> {
    DurableCleanupErrorCategory::new(category.as_str().to_owned())
        .ok_or(CleanupHistoryError::CorruptData)
}

fn public_cleanup_status_counts(
    counts: StoredCleanupStatusCounts,
) -> Result<DurableCleanupStatusCounts, CleanupHistoryError> {
    let groups = [
        (DurableCleanupItemStatus::Planned, counts.planned),
        (DurableCleanupItemStatus::Validating, counts.validating),
        (DurableCleanupItemStatus::DryRun, counts.dry_run),
        (
            DurableCleanupItemStatus::EffectStarted,
            counts.effect_started,
        ),
        (DurableCleanupItemStatus::Trashed, counts.trashed),
        (DurableCleanupItemStatus::Removed, counts.removed),
        (DurableCleanupItemStatus::Evicted, counts.evicted),
        (DurableCleanupItemStatus::Skipped, counts.skipped),
        (DurableCleanupItemStatus::Rejected, counts.rejected),
        (DurableCleanupItemStatus::Failed, counts.failed),
        (
            DurableCleanupItemStatus::ChangedSincePlan,
            counts.changed_since_plan,
        ),
        (DurableCleanupItemStatus::Interrupted, counts.interrupted),
        (DurableCleanupItemStatus::Unavailable, counts.unavailable),
        (
            DurableCleanupItemStatus::OutcomeUnknown,
            counts.outcome_unknown,
        ),
    ];
    let statuses = groups
        .into_iter()
        .flat_map(|(status, count)| std::iter::repeat_n(status, usize::from(count)));
    let public = DurableCleanupStatusCounts::from_statuses(statuses)
        .ok_or(CleanupHistoryError::CorruptData)?;
    if public.total() != counts.total {
        return Err(CleanupHistoryError::CorruptData);
    }
    Ok(public)
}

const fn public_cleanup_mode(mode: StoredCleanupMode) -> DurableCleanupMode {
    match mode {
        StoredCleanupMode::DryRun => DurableCleanupMode::DryRun,
        StoredCleanupMode::Trash => DurableCleanupMode::Trash,
        StoredCleanupMode::PermanentSafe => DurableCleanupMode::PermanentSafe,
        StoredCleanupMode::EvictLocalCopy => DurableCleanupMode::EvictLocalCopy,
    }
}

const fn public_cleanup_trigger(trigger: StoredCleanupTrigger) -> DurableCleanupTrigger {
    match trigger {
        StoredCleanupTrigger::Manual => DurableCleanupTrigger::Manual,
        StoredCleanupTrigger::LowDisk => DurableCleanupTrigger::LowDisk,
        StoredCleanupTrigger::Scheduled => DurableCleanupTrigger::Scheduled,
        StoredCleanupTrigger::Cli => DurableCleanupTrigger::Cli,
    }
}

const fn public_cleanup_session_status(
    status: StoredCleanupSessionStatus,
) -> DurableCleanupSessionStatus {
    match status {
        StoredCleanupSessionStatus::Planned => DurableCleanupSessionStatus::Planned,
        StoredCleanupSessionStatus::Running => DurableCleanupSessionStatus::Running,
        StoredCleanupSessionStatus::Recovering => DurableCleanupSessionStatus::Recovering,
        StoredCleanupSessionStatus::Completed => DurableCleanupSessionStatus::Completed,
        StoredCleanupSessionStatus::PartiallyCompleted => {
            DurableCleanupSessionStatus::PartiallyCompleted
        }
        StoredCleanupSessionStatus::Failed => DurableCleanupSessionStatus::Failed,
        StoredCleanupSessionStatus::Cancelled => DurableCleanupSessionStatus::Cancelled,
        StoredCleanupSessionStatus::Interrupted => DurableCleanupSessionStatus::Interrupted,
        StoredCleanupSessionStatus::Rejected => DurableCleanupSessionStatus::Rejected,
        StoredCleanupSessionStatus::DryRun => DurableCleanupSessionStatus::DryRun,
    }
}

const fn public_cleanup_item_status(status: StoredCleanupItemStatus) -> DurableCleanupItemStatus {
    match status {
        StoredCleanupItemStatus::Planned => DurableCleanupItemStatus::Planned,
        StoredCleanupItemStatus::Validating => DurableCleanupItemStatus::Validating,
        StoredCleanupItemStatus::DryRun => DurableCleanupItemStatus::DryRun,
        StoredCleanupItemStatus::EffectStarted => DurableCleanupItemStatus::EffectStarted,
        StoredCleanupItemStatus::Trashed => DurableCleanupItemStatus::Trashed,
        StoredCleanupItemStatus::Removed => DurableCleanupItemStatus::Removed,
        StoredCleanupItemStatus::Evicted => DurableCleanupItemStatus::Evicted,
        StoredCleanupItemStatus::Skipped => DurableCleanupItemStatus::Skipped,
        StoredCleanupItemStatus::Rejected => DurableCleanupItemStatus::Rejected,
        StoredCleanupItemStatus::Failed => DurableCleanupItemStatus::Failed,
        StoredCleanupItemStatus::ChangedSincePlan => DurableCleanupItemStatus::ChangedSincePlan,
        StoredCleanupItemStatus::Interrupted => DurableCleanupItemStatus::Interrupted,
        StoredCleanupItemStatus::Unavailable => DurableCleanupItemStatus::Unavailable,
        StoredCleanupItemStatus::OutcomeUnknown => DurableCleanupItemStatus::OutcomeUnknown,
    }
}

const fn public_cleanup_warning(warning: crate::domain::PlanWarning) -> DurableCleanupWarning {
    match warning {
        crate::domain::PlanWarning::EstimatedBytesUnverified => {
            DurableCleanupWarning::EstimatedBytesUnverified
        }
        crate::domain::PlanWarning::DryRunDoesNotMutate => {
            DurableCleanupWarning::DryRunDoesNotMutate
        }
        crate::domain::PlanWarning::TrashDoesNotFreeSpaceImmediately => {
            DurableCleanupWarning::TrashDoesNotFreeSpaceImmediately
        }
        crate::domain::PlanWarning::PermanentRemovalCannotBeUndone => {
            DurableCleanupWarning::PermanentRemovalCannotBeUndone
        }
        crate::domain::PlanWarning::CloudEvictionRequiresNetworkToRedownload => {
            DurableCleanupWarning::CloudEvictionRequiresNetworkToRedownload
        }
    }
}

const fn map_cleanup_history_error(kind: HistoryErrorKind) -> CleanupHistoryError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => CleanupHistoryError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => CleanupHistoryError::QueryLimitExceeded,
        HistoryErrorKind::Busy => CleanupHistoryError::Busy,
        HistoryErrorKind::UnsafeStorage => CleanupHistoryError::UnsafeStorage,
        HistoryErrorKind::CorruptData => CleanupHistoryError::CorruptData,
        HistoryErrorKind::InternalState | HistoryErrorKind::InvalidInput => {
            CleanupHistoryError::InternalState
        }
        HistoryErrorKind::DatabaseUnavailable
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::OutcomeUnknown => CleanupHistoryError::Unavailable,
    }
}

fn public_candidate_history(
    requested_scan_id: &ScanId,
    observation: CandidateEvaluationObservation,
) -> Result<DurableCandidateEvaluation, CandidateHistoryError> {
    match observation {
        CandidateEvaluationObservation::MissingScan => Err(CandidateHistoryError::ScanNotFound),
        CandidateEvaluationObservation::NotRun { scan_status } => {
            Ok(DurableCandidateEvaluation::new(
                requested_scan_id.clone(),
                public_durable_scan_status(scan_status),
                None,
                None,
                DurableCandidateEvaluationStatus::NotRun,
                Vec::new(),
            ))
        }
        CandidateEvaluationObservation::Pending(record) => public_candidate_record(
            requested_scan_id,
            record,
            CandidateEvaluationStatus::Pending,
        ),
        CandidateEvaluationObservation::Succeeded(record) => {
            let status = record.status();
            if !matches!(status, CandidateEvaluationStatus::Succeeded { .. }) {
                return Err(CandidateHistoryError::CorruptData);
            }
            public_candidate_record(requested_scan_id, record, status)
        }
        CandidateEvaluationObservation::Failed(record) => {
            let status = record.status();
            if !matches!(status, CandidateEvaluationStatus::Failed { .. }) {
                return Err(CandidateHistoryError::CorruptData);
            }
            public_candidate_record(requested_scan_id, record, status)
        }
    }
}

fn public_candidate_record(
    requested_scan_id: &ScanId,
    record: CandidateEvaluationRecord,
    expected_status: CandidateEvaluationStatus,
) -> Result<DurableCandidateEvaluation, CandidateHistoryError> {
    if record.scan_id() != requested_scan_id || record.status() != expected_status {
        return Err(CandidateHistoryError::CorruptData);
    }
    let status = match record.status() {
        CandidateEvaluationStatus::Pending => {
            if record.completed_at().is_some() || !record.candidates().is_empty() {
                return Err(CandidateHistoryError::CorruptData);
            }
            DurableCandidateEvaluationStatus::Pending
        }
        CandidateEvaluationStatus::Succeeded { candidate_count } => {
            if record.completed_at().is_none()
                || usize::try_from(candidate_count).ok() != Some(record.candidates().len())
            {
                return Err(CandidateHistoryError::CorruptData);
            }
            DurableCandidateEvaluationStatus::Succeeded { candidate_count }
        }
        CandidateEvaluationStatus::Failed { kind } => {
            if record.completed_at().is_none() || !record.candidates().is_empty() {
                return Err(CandidateHistoryError::CorruptData);
            }
            DurableCandidateEvaluationStatus::Failed {
                kind: public_candidate_evaluation_failure(kind),
            }
        }
    };
    let candidates = record
        .candidates()
        .iter()
        .map(|candidate| public_candidate_summary(requested_scan_id, candidate))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DurableCandidateEvaluation::new(
        requested_scan_id.clone(),
        DurableScanStatus::Succeeded,
        Some(record.scheduled_at()),
        record.completed_at(),
        status,
        candidates,
    ))
}

fn public_candidate_summary(
    requested_scan_id: &ScanId,
    candidate: &CompleteCandidateRecord,
) -> Result<DurableCandidateSummary, CandidateHistoryError> {
    if candidate.source_scan_id() != requested_scan_id {
        return Err(CandidateHistoryError::CorruptData);
    }
    let path_count =
        u16::try_from(candidate.paths().len()).map_err(|_| CandidateHistoryError::InternalState)?;
    Ok(DurableCandidateSummary::new(
        candidate.id().clone(),
        candidate.rule().clone(),
        candidate.category(),
        candidate.estimated_bytes(),
        candidate.newest_mtime(),
        candidate.safety(),
        candidate.action(),
        candidate.rule_schedule_eligible(),
        path_count,
        candidate
            .evidence()
            .iter()
            .map(crate::domain::Evidence::kind)
            .collect(),
        candidate.blockers().to_vec(),
        candidate.created_at(),
        public_candidate_status(candidate.status()),
    ))
}

const fn public_candidate_status(status: CandidateHistoryStatus) -> DurableCandidateStatus {
    match status {
        CandidateHistoryStatus::Discovered => DurableCandidateStatus::Discovered,
        CandidateHistoryStatus::Selected => DurableCandidateStatus::Selected,
        CandidateHistoryStatus::Dismissed => DurableCandidateStatus::Dismissed,
        CandidateHistoryStatus::Stale => DurableCandidateStatus::Stale,
        CandidateHistoryStatus::Planned => DurableCandidateStatus::Planned,
        CandidateHistoryStatus::Completed => DurableCandidateStatus::Completed,
        CandidateHistoryStatus::Failed => DurableCandidateStatus::Failed,
        CandidateHistoryStatus::Unavailable => DurableCandidateStatus::Unavailable,
    }
}

const fn public_durable_scan_status(status: ScanStatus) -> DurableScanStatus {
    match status {
        ScanStatus::Queued => DurableScanStatus::Queued,
        ScanStatus::Running => DurableScanStatus::Running,
        ScanStatus::Succeeded => DurableScanStatus::Succeeded,
        ScanStatus::Failed => DurableScanStatus::Failed,
        ScanStatus::Cancelled => DurableScanStatus::Cancelled,
        ScanStatus::Interrupted => DurableScanStatus::Interrupted,
    }
}

const fn map_candidate_history_error(kind: HistoryErrorKind) -> CandidateHistoryError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => CandidateHistoryError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => CandidateHistoryError::QueryLimitExceeded,
        HistoryErrorKind::Busy => CandidateHistoryError::Busy,
        HistoryErrorKind::UnsafeStorage => CandidateHistoryError::UnsafeStorage,
        HistoryErrorKind::CorruptData => CandidateHistoryError::CorruptData,
        HistoryErrorKind::InternalState => CandidateHistoryError::InternalState,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::DatabaseUnavailable
        | HistoryErrorKind::OutcomeUnknown => CandidateHistoryError::Unavailable,
    }
}

fn validate_candidate_detail_limit(limit: u16) -> Result<(), CandidateDetailError> {
    if !(1..=MAX_CANDIDATE_DETAIL_PAGE_LIMIT).contains(&limit) {
        return Err(CandidateDetailError::InvalidLimit {
            maximum: MAX_CANDIDATE_DETAIL_PAGE_LIMIT,
        });
    }
    Ok(())
}

fn candidate_detail_range(
    cursor: u16,
    limit: u16,
    total: u16,
) -> Result<std::ops::Range<usize>, CandidateDetailError> {
    if cursor > total {
        return Err(CandidateDetailError::CursorOutOfRange);
    }
    let start = usize::from(cursor);
    let end = start
        .checked_add(usize::from(limit))
        .ok_or(CandidateDetailError::InternalState)?
        .min(usize::from(total));
    Ok(start..end)
}

fn next_candidate_detail_cursor(
    range: &std::ops::Range<usize>,
    total: u16,
) -> Result<Option<u16>, CandidateDetailError> {
    if range.end >= usize::from(total) {
        return Ok(None);
    }
    Ok(Some(
        u16::try_from(range.end).map_err(|_| CandidateDetailError::InternalState)?,
    ))
}

fn public_observed_path(path: &Path) -> Result<DurableObservedPath, CandidateDetailError> {
    let observation = observe_host_path(path).map_err(|_| CandidateDetailError::InternalState)?;
    let encoding = match observation.encoding() {
        HostPathObservationEncoding::Utf8 => DurablePathEncoding::Utf8,
        HostPathObservationEncoding::Utf16LittleEndian => DurablePathEncoding::Utf16LittleEndian,
    };
    Ok(DurableObservedPath::new(
        encoding,
        observation.bytes().to_vec(),
        path.display().to_string(),
    ))
}

fn public_candidate_evidence(
    evidence: &Evidence,
) -> Result<DurableCandidateEvidence, CandidateDetailError> {
    match evidence {
        Evidence::MatchedPath { path } => Ok(DurableCandidateEvidence::MatchedPath {
            path: public_observed_path(path)?,
        }),
        Evidence::RequiredMarker { path } => Ok(DurableCandidateEvidence::RequiredMarker {
            path: public_observed_path(path)?,
        }),
        Evidence::ForbiddenMarkerAbsent { path } => {
            Ok(DurableCandidateEvidence::ForbiddenMarkerAbsent {
                path: public_observed_path(path)?,
            })
        }
        Evidence::BundleIdentifier { path, identifier } => {
            Ok(DurableCandidateEvidence::BundleIdentifier {
                path: public_observed_path(path)?,
                identifier: Arc::<str>::from(identifier.as_str()),
            })
        }
        Evidence::MinimumAge {
            newest_mtime,
            minimum_age,
        } => Ok(DurableCandidateEvidence::MinimumAge {
            newest_mtime: *newest_mtime,
            minimum_age: *minimum_age,
        }),
        Evidence::MinimumSize {
            observed_bytes,
            minimum_bytes,
        } => Ok(DurableCandidateEvidence::MinimumSize {
            observed_bytes: *observed_bytes,
            minimum_bytes: *minimum_bytes,
        }),
        Evidence::InactiveProcess { identifier } => Ok(DurableCandidateEvidence::InactiveProcess {
            identifier: Arc::<str>::from(identifier.as_str()),
        }),
        Evidence::CloudUploadComplete { path } => {
            Ok(DurableCandidateEvidence::CloudUploadComplete {
                path: public_observed_path(path)?,
            })
        }
    }
}

const fn map_candidate_detail_error(kind: HistoryErrorKind) -> CandidateDetailError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => CandidateDetailError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => CandidateDetailError::QueryLimitExceeded,
        HistoryErrorKind::Busy => CandidateDetailError::Busy,
        HistoryErrorKind::UnsafeStorage => CandidateDetailError::UnsafeStorage,
        HistoryErrorKind::CorruptData => CandidateDetailError::CorruptData,
        HistoryErrorKind::InternalState => CandidateDetailError::InternalState,
        HistoryErrorKind::NotFound => CandidateDetailError::CandidateNotFound,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::DatabaseUnavailable
        | HistoryErrorKind::OutcomeUnknown => CandidateDetailError::Unavailable,
    }
}

const fn map_candidate_history_to_detail_error(
    error: CandidateHistoryError,
) -> CandidateDetailError {
    match error {
        CandidateHistoryError::Closed => CandidateDetailError::Closed,
        CandidateHistoryError::ScanNotFound => CandidateDetailError::ScanNotFound,
        CandidateHistoryError::IncompatibleSchema => CandidateDetailError::IncompatibleSchema,
        CandidateHistoryError::Busy => CandidateDetailError::Busy,
        CandidateHistoryError::UnsafeStorage => CandidateDetailError::UnsafeStorage,
        CandidateHistoryError::QueryLimitExceeded => CandidateDetailError::QueryLimitExceeded,
        CandidateHistoryError::CorruptData => CandidateDetailError::CorruptData,
        CandidateHistoryError::Unavailable => CandidateDetailError::Unavailable,
        CandidateHistoryError::InternalState => CandidateDetailError::InternalState,
    }
}

const fn map_candidate_review_error(kind: HistoryErrorKind) -> CandidateReviewError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => CandidateReviewError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => CandidateReviewError::QueryLimitExceeded,
        HistoryErrorKind::Busy => CandidateReviewError::Busy,
        HistoryErrorKind::UnsafeStorage => CandidateReviewError::UnsafeStorage,
        HistoryErrorKind::CorruptData => CandidateReviewError::CorruptData,
        HistoryErrorKind::OutcomeUnknown => CandidateReviewError::OutcomeUnknown,
        HistoryErrorKind::InternalState => CandidateReviewError::InternalState,
        HistoryErrorKind::NotFound => CandidateReviewError::CandidateNotFound,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::InvalidTransition => CandidateReviewError::NotReviewable,
        HistoryErrorKind::DatabaseUnavailable => CandidateReviewError::Unavailable,
    }
}

fn public_snapshot_retention_cap(setting: SnapshotRetentionCapSetting) -> SnapshotRetentionCap {
    SnapshotRetentionCap {
        cap_bytes: setting.cap_bytes,
        source: match setting.source {
            SnapshotRetentionCapSettingSource::Default => SnapshotRetentionCapSource::Default,
            SnapshotRetentionCapSettingSource::Stored => SnapshotRetentionCapSource::Stored,
        },
        updated_at: setting.updated_at,
    }
}

fn public_snapshot_retention_cap_update(
    update: SnapshotRetentionCapSettingUpdate,
) -> SnapshotRetentionCapUpdate {
    SnapshotRetentionCapUpdate {
        settings: public_snapshot_retention_cap(update.settings),
        changed: update.changed,
    }
}

const fn map_snapshot_retention_cap_error(kind: HistoryErrorKind) -> SnapshotRetentionCapError {
    match kind {
        HistoryErrorKind::InvalidInput => SnapshotRetentionCapError::InvalidClock,
        HistoryErrorKind::IncompatibleSchema => SnapshotRetentionCapError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => SnapshotRetentionCapError::QueryLimitExceeded,
        HistoryErrorKind::Busy => SnapshotRetentionCapError::Busy,
        HistoryErrorKind::UnsafeStorage => SnapshotRetentionCapError::UnsafeStorage,
        HistoryErrorKind::CorruptData => SnapshotRetentionCapError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => SnapshotRetentionCapError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => SnapshotRetentionCapError::OutcomeUnknown,
        HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::InternalState => SnapshotRetentionCapError::InternalState,
    }
}

const fn map_history_maintenance_failure(kind: HistoryErrorKind) -> TaskFailureKind {
    let kind = match kind {
        HistoryErrorKind::InvalidInput => HistoryMaintenanceFailureKind::InvalidClock,
        HistoryErrorKind::IncompatibleSchema => HistoryMaintenanceFailureKind::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => HistoryMaintenanceFailureKind::BudgetExceeded,
        HistoryErrorKind::Busy => HistoryMaintenanceFailureKind::Busy,
        HistoryErrorKind::UnsafeStorage => HistoryMaintenanceFailureKind::UnsafeStorage,
        HistoryErrorKind::CorruptData => HistoryMaintenanceFailureKind::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => HistoryMaintenanceFailureKind::Unavailable,
        HistoryErrorKind::OutcomeUnknown => HistoryMaintenanceFailureKind::OutcomeUnknown,
        HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::InternalState => HistoryMaintenanceFailureKind::InternalState,
    };
    TaskFailureKind::HistoryMaintenance(kind)
}

const fn public_snapshot_retention_outcome(
    outcome: &SnapshotRetentionBatchOutcome,
) -> SnapshotRetentionOutcome {
    match outcome {
        SnapshotRetentionBatchOutcome::UnderCap => SnapshotRetentionOutcome::UnderCap,
        SnapshotRetentionBatchOutcome::DeferredUnstable => {
            SnapshotRetentionOutcome::DeferredUnstable
        }
        SnapshotRetentionBatchOutcome::DeferredNoEligibleSnapshot => {
            SnapshotRetentionOutcome::DeferredNoEligibleSnapshot
        }
        SnapshotRetentionBatchOutcome::RemovedTombstonedResidual { bytes, .. } => {
            SnapshotRetentionOutcome::RemovedTombstonedResidual { bytes: *bytes }
        }
        SnapshotRetentionBatchOutcome::TombstonedAndRemoved { bytes, .. } => {
            SnapshotRetentionOutcome::TombstonedAndRemoved { bytes: *bytes }
        }
    }
}

const fn public_snapshot_orphan_maintenance_outcome(
    outcome: &SnapshotOrphanReconciliationBatchOutcome,
) -> SnapshotOrphanMaintenanceOutcome {
    match outcome {
        SnapshotOrphanReconciliationBatchOutcome::NoOrphan => {
            SnapshotOrphanMaintenanceOutcome::NoOrphan
        }
        SnapshotOrphanReconciliationBatchOutcome::Removed { bytes, .. } => {
            SnapshotOrphanMaintenanceOutcome::Removed { bytes: *bytes }
        }
    }
}

const fn public_snapshot_terminal_temp_maintenance_outcome(
    outcome: &SnapshotTerminalTempReconciliationBatchOutcome,
) -> SnapshotTerminalTempMaintenanceOutcome {
    match outcome {
        SnapshotTerminalTempReconciliationBatchOutcome::NoTerminalResidual => {
            SnapshotTerminalTempMaintenanceOutcome::NoTerminalResidual
        }
        SnapshotTerminalTempReconciliationBatchOutcome::DeferredActive => {
            SnapshotTerminalTempMaintenanceOutcome::DeferredActive
        }
        SnapshotTerminalTempReconciliationBatchOutcome::ReconciledRowOnly { .. } => {
            SnapshotTerminalTempMaintenanceOutcome::ReconciledRowOnly
        }
        SnapshotTerminalTempReconciliationBatchOutcome::RemovedTempAndLease { bytes, .. } => {
            SnapshotTerminalTempMaintenanceOutcome::RemovedTemp { bytes: *bytes }
        }
    }
}

const fn public_snapshot_unleased_temp_maintenance_outcome(
    outcome: &SnapshotUnleasedTempReconciliationBatchOutcome,
) -> SnapshotUnleasedTempMaintenanceOutcome {
    match outcome {
        SnapshotUnleasedTempReconciliationBatchOutcome::NoUnleasedTemp => {
            SnapshotUnleasedTempMaintenanceOutcome::NoUnleasedTemp
        }
        SnapshotUnleasedTempReconciliationBatchOutcome::DeferredActive => {
            SnapshotUnleasedTempMaintenanceOutcome::DeferredActive
        }
        SnapshotUnleasedTempReconciliationBatchOutcome::Removed { bytes } => {
            SnapshotUnleasedTempMaintenanceOutcome::Removed { bytes: *bytes }
        }
    }
}

const fn map_snapshot_retention_failure(kind: SnapshotRepositoryErrorKind) -> TaskFailureKind {
    let kind = match kind {
        SnapshotRepositoryErrorKind::ReadOnly
        | SnapshotRepositoryErrorKind::MissingStore
        | SnapshotRepositoryErrorKind::ReviewLeaseExpired => {
            SnapshotRetentionFailureKind::InternalState
        }
        SnapshotRepositoryErrorKind::MissingSnapshot
        | SnapshotRepositoryErrorKind::SnapshotUnavailable
        | SnapshotRepositoryErrorKind::ReferenceMismatch => {
            SnapshotRetentionFailureKind::CorruptData
        }
        SnapshotRepositoryErrorKind::IncompatibleVersion => {
            SnapshotRetentionFailureKind::IncompatibleSnapshot
        }
        SnapshotRepositoryErrorKind::Codec(kind) => match kind {
            SnapshotCodecErrorKind::InvalidInput => SnapshotRetentionFailureKind::InternalState,
            SnapshotCodecErrorKind::Io => SnapshotRetentionFailureKind::Unavailable,
            SnapshotCodecErrorKind::LimitExceeded => SnapshotRetentionFailureKind::BudgetExceeded,
            SnapshotCodecErrorKind::IncompatibleVersion => {
                SnapshotRetentionFailureKind::IncompatibleSnapshot
            }
            SnapshotCodecErrorKind::InvalidMagic
            | SnapshotCodecErrorKind::InvalidLength
            | SnapshotCodecErrorKind::ChecksumMismatch
            | SnapshotCodecErrorKind::CorruptData => SnapshotRetentionFailureKind::CorruptData,
        },
        SnapshotRepositoryErrorKind::Storage(kind) => match kind {
            SnapshotStorageErrorKind::InvalidConfiguration
            | SnapshotStorageErrorKind::InternalState => {
                SnapshotRetentionFailureKind::InternalState
            }
            SnapshotStorageErrorKind::UnsafeRoot
            | SnapshotStorageErrorKind::UnsafeObject
            | SnapshotStorageErrorKind::UnrecognizedStore => {
                SnapshotRetentionFailureKind::UnsafeStorage
            }
            SnapshotStorageErrorKind::Unavailable => SnapshotRetentionFailureKind::Unavailable,
            SnapshotStorageErrorKind::Busy => SnapshotRetentionFailureKind::Busy,
        },
        SnapshotRepositoryErrorKind::History(kind) => match kind {
            HistoryErrorKind::InvalidInput => SnapshotRetentionFailureKind::InvalidClock,
            HistoryErrorKind::IncompatibleSchema => {
                SnapshotRetentionFailureKind::IncompatibleSchema
            }
            HistoryErrorKind::QueryLimitExceeded => SnapshotRetentionFailureKind::BudgetExceeded,
            HistoryErrorKind::Busy => SnapshotRetentionFailureKind::Busy,
            HistoryErrorKind::UnsafeStorage => SnapshotRetentionFailureKind::UnsafeStorage,
            HistoryErrorKind::CorruptData => SnapshotRetentionFailureKind::CorruptData,
            HistoryErrorKind::DatabaseUnavailable => SnapshotRetentionFailureKind::Unavailable,
            HistoryErrorKind::OutcomeUnknown => SnapshotRetentionFailureKind::OutcomeUnknown,
            HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::NotFound
            | HistoryErrorKind::InvalidTransition
            | HistoryErrorKind::InternalState => SnapshotRetentionFailureKind::InternalState,
        },
    };
    TaskFailureKind::SnapshotRetention(kind)
}

const fn map_snapshot_orphan_maintenance_failure(
    kind: SnapshotRepositoryErrorKind,
) -> TaskFailureKind {
    let kind = match kind {
        SnapshotRepositoryErrorKind::ReadOnly
        | SnapshotRepositoryErrorKind::MissingStore
        | SnapshotRepositoryErrorKind::ReviewLeaseExpired => {
            SnapshotOrphanMaintenanceFailureKind::InternalState
        }
        SnapshotRepositoryErrorKind::MissingSnapshot
        | SnapshotRepositoryErrorKind::SnapshotUnavailable
        | SnapshotRepositoryErrorKind::ReferenceMismatch => {
            SnapshotOrphanMaintenanceFailureKind::CorruptData
        }
        SnapshotRepositoryErrorKind::IncompatibleVersion => {
            SnapshotOrphanMaintenanceFailureKind::IncompatibleSnapshot
        }
        SnapshotRepositoryErrorKind::Codec(kind) => match kind {
            SnapshotCodecErrorKind::InvalidInput => {
                SnapshotOrphanMaintenanceFailureKind::InternalState
            }
            SnapshotCodecErrorKind::Io => SnapshotOrphanMaintenanceFailureKind::Unavailable,
            SnapshotCodecErrorKind::LimitExceeded => {
                SnapshotOrphanMaintenanceFailureKind::BudgetExceeded
            }
            SnapshotCodecErrorKind::IncompatibleVersion => {
                SnapshotOrphanMaintenanceFailureKind::IncompatibleSnapshot
            }
            SnapshotCodecErrorKind::InvalidMagic
            | SnapshotCodecErrorKind::InvalidLength
            | SnapshotCodecErrorKind::ChecksumMismatch
            | SnapshotCodecErrorKind::CorruptData => {
                SnapshotOrphanMaintenanceFailureKind::CorruptData
            }
        },
        SnapshotRepositoryErrorKind::Storage(kind) => match kind {
            SnapshotStorageErrorKind::InvalidConfiguration
            | SnapshotStorageErrorKind::InternalState => {
                SnapshotOrphanMaintenanceFailureKind::InternalState
            }
            SnapshotStorageErrorKind::UnsafeRoot
            | SnapshotStorageErrorKind::UnsafeObject
            | SnapshotStorageErrorKind::UnrecognizedStore => {
                SnapshotOrphanMaintenanceFailureKind::UnsafeStorage
            }
            SnapshotStorageErrorKind::Unavailable => {
                SnapshotOrphanMaintenanceFailureKind::Unavailable
            }
            SnapshotStorageErrorKind::Busy => SnapshotOrphanMaintenanceFailureKind::Busy,
        },
        SnapshotRepositoryErrorKind::History(kind) => match kind {
            HistoryErrorKind::InvalidInput => SnapshotOrphanMaintenanceFailureKind::InvalidClock,
            HistoryErrorKind::IncompatibleSchema => {
                SnapshotOrphanMaintenanceFailureKind::IncompatibleSchema
            }
            HistoryErrorKind::QueryLimitExceeded => {
                SnapshotOrphanMaintenanceFailureKind::BudgetExceeded
            }
            HistoryErrorKind::Busy => SnapshotOrphanMaintenanceFailureKind::Busy,
            HistoryErrorKind::UnsafeStorage => SnapshotOrphanMaintenanceFailureKind::UnsafeStorage,
            HistoryErrorKind::CorruptData => SnapshotOrphanMaintenanceFailureKind::CorruptData,
            HistoryErrorKind::DatabaseUnavailable => {
                SnapshotOrphanMaintenanceFailureKind::Unavailable
            }
            HistoryErrorKind::OutcomeUnknown => {
                SnapshotOrphanMaintenanceFailureKind::OutcomeUnknown
            }
            HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::NotFound
            | HistoryErrorKind::InvalidTransition
            | HistoryErrorKind::InternalState => {
                SnapshotOrphanMaintenanceFailureKind::InternalState
            }
        },
    };
    TaskFailureKind::SnapshotOrphanMaintenance(kind)
}

const fn map_snapshot_terminal_temp_maintenance_failure(
    kind: SnapshotRepositoryErrorKind,
) -> TaskFailureKind {
    let kind = match kind {
        SnapshotRepositoryErrorKind::ReadOnly
        | SnapshotRepositoryErrorKind::MissingStore
        | SnapshotRepositoryErrorKind::ReviewLeaseExpired => {
            SnapshotTerminalTempMaintenanceFailureKind::InternalState
        }
        SnapshotRepositoryErrorKind::MissingSnapshot
        | SnapshotRepositoryErrorKind::SnapshotUnavailable
        | SnapshotRepositoryErrorKind::ReferenceMismatch => {
            SnapshotTerminalTempMaintenanceFailureKind::CorruptData
        }
        // Terminal temp bytes are deliberately never decoded. Reaching an
        // incompatible snapshot through this boundary is an engine contract
        // violation rather than a user-facing snapshot compatibility result.
        SnapshotRepositoryErrorKind::IncompatibleVersion => {
            SnapshotTerminalTempMaintenanceFailureKind::InternalState
        }
        SnapshotRepositoryErrorKind::Codec(kind) => match kind {
            SnapshotCodecErrorKind::InvalidInput | SnapshotCodecErrorKind::IncompatibleVersion => {
                SnapshotTerminalTempMaintenanceFailureKind::InternalState
            }
            SnapshotCodecErrorKind::Io => SnapshotTerminalTempMaintenanceFailureKind::Unavailable,
            SnapshotCodecErrorKind::LimitExceeded => {
                SnapshotTerminalTempMaintenanceFailureKind::BudgetExceeded
            }
            SnapshotCodecErrorKind::InvalidMagic
            | SnapshotCodecErrorKind::InvalidLength
            | SnapshotCodecErrorKind::ChecksumMismatch
            | SnapshotCodecErrorKind::CorruptData => {
                SnapshotTerminalTempMaintenanceFailureKind::CorruptData
            }
        },
        SnapshotRepositoryErrorKind::Storage(kind) => match kind {
            SnapshotStorageErrorKind::InvalidConfiguration
            | SnapshotStorageErrorKind::InternalState => {
                SnapshotTerminalTempMaintenanceFailureKind::InternalState
            }
            SnapshotStorageErrorKind::UnsafeRoot
            | SnapshotStorageErrorKind::UnsafeObject
            | SnapshotStorageErrorKind::UnrecognizedStore => {
                SnapshotTerminalTempMaintenanceFailureKind::UnsafeStorage
            }
            SnapshotStorageErrorKind::Unavailable => {
                SnapshotTerminalTempMaintenanceFailureKind::Unavailable
            }
            SnapshotStorageErrorKind::Busy => SnapshotTerminalTempMaintenanceFailureKind::Busy,
        },
        SnapshotRepositoryErrorKind::History(kind) => match kind {
            HistoryErrorKind::InvalidInput => {
                SnapshotTerminalTempMaintenanceFailureKind::InvalidClock
            }
            HistoryErrorKind::IncompatibleSchema => {
                SnapshotTerminalTempMaintenanceFailureKind::IncompatibleSchema
            }
            HistoryErrorKind::QueryLimitExceeded => {
                SnapshotTerminalTempMaintenanceFailureKind::BudgetExceeded
            }
            HistoryErrorKind::Busy => SnapshotTerminalTempMaintenanceFailureKind::Busy,
            HistoryErrorKind::UnsafeStorage => {
                SnapshotTerminalTempMaintenanceFailureKind::UnsafeStorage
            }
            HistoryErrorKind::CorruptData => {
                SnapshotTerminalTempMaintenanceFailureKind::CorruptData
            }
            HistoryErrorKind::DatabaseUnavailable => {
                SnapshotTerminalTempMaintenanceFailureKind::Unavailable
            }
            HistoryErrorKind::OutcomeUnknown => {
                SnapshotTerminalTempMaintenanceFailureKind::OutcomeUnknown
            }
            HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::NotFound
            | HistoryErrorKind::InvalidTransition
            | HistoryErrorKind::InternalState => {
                SnapshotTerminalTempMaintenanceFailureKind::InternalState
            }
        },
    };
    TaskFailureKind::SnapshotTerminalTempMaintenance(kind)
}

const fn map_snapshot_unleased_temp_maintenance_failure(
    kind: SnapshotRepositoryErrorKind,
) -> TaskFailureKind {
    let kind = match kind {
        SnapshotRepositoryErrorKind::ReadOnly
        | SnapshotRepositoryErrorKind::MissingStore
        | SnapshotRepositoryErrorKind::ReviewLeaseExpired => {
            SnapshotUnleasedTempMaintenanceFailureKind::InternalState
        }
        SnapshotRepositoryErrorKind::MissingSnapshot
        | SnapshotRepositoryErrorKind::SnapshotUnavailable
        | SnapshotRepositoryErrorKind::ReferenceMismatch => {
            SnapshotUnleasedTempMaintenanceFailureKind::CorruptData
        }
        // Unleased temp bytes are deliberately never decoded. Reaching an
        // incompatible snapshot through this boundary is a contract violation.
        SnapshotRepositoryErrorKind::IncompatibleVersion => {
            SnapshotUnleasedTempMaintenanceFailureKind::InternalState
        }
        SnapshotRepositoryErrorKind::Codec(kind) => match kind {
            SnapshotCodecErrorKind::InvalidInput | SnapshotCodecErrorKind::IncompatibleVersion => {
                SnapshotUnleasedTempMaintenanceFailureKind::InternalState
            }
            SnapshotCodecErrorKind::Io => SnapshotUnleasedTempMaintenanceFailureKind::Unavailable,
            SnapshotCodecErrorKind::LimitExceeded => {
                SnapshotUnleasedTempMaintenanceFailureKind::BudgetExceeded
            }
            SnapshotCodecErrorKind::InvalidMagic
            | SnapshotCodecErrorKind::InvalidLength
            | SnapshotCodecErrorKind::ChecksumMismatch
            | SnapshotCodecErrorKind::CorruptData => {
                SnapshotUnleasedTempMaintenanceFailureKind::CorruptData
            }
        },
        SnapshotRepositoryErrorKind::Storage(kind) => match kind {
            SnapshotStorageErrorKind::InvalidConfiguration
            | SnapshotStorageErrorKind::InternalState => {
                SnapshotUnleasedTempMaintenanceFailureKind::InternalState
            }
            SnapshotStorageErrorKind::UnsafeRoot
            | SnapshotStorageErrorKind::UnsafeObject
            | SnapshotStorageErrorKind::UnrecognizedStore => {
                SnapshotUnleasedTempMaintenanceFailureKind::UnsafeStorage
            }
            SnapshotStorageErrorKind::Unavailable => {
                SnapshotUnleasedTempMaintenanceFailureKind::Unavailable
            }
            SnapshotStorageErrorKind::Busy => SnapshotUnleasedTempMaintenanceFailureKind::Busy,
        },
        SnapshotRepositoryErrorKind::History(kind) => match kind {
            HistoryErrorKind::InvalidInput => {
                SnapshotUnleasedTempMaintenanceFailureKind::InvalidClock
            }
            HistoryErrorKind::IncompatibleSchema => {
                SnapshotUnleasedTempMaintenanceFailureKind::IncompatibleSchema
            }
            HistoryErrorKind::QueryLimitExceeded => {
                SnapshotUnleasedTempMaintenanceFailureKind::BudgetExceeded
            }
            HistoryErrorKind::Busy => SnapshotUnleasedTempMaintenanceFailureKind::Busy,
            HistoryErrorKind::UnsafeStorage => {
                SnapshotUnleasedTempMaintenanceFailureKind::UnsafeStorage
            }
            HistoryErrorKind::CorruptData => {
                SnapshotUnleasedTempMaintenanceFailureKind::CorruptData
            }
            HistoryErrorKind::DatabaseUnavailable => {
                SnapshotUnleasedTempMaintenanceFailureKind::Unavailable
            }
            HistoryErrorKind::OutcomeUnknown => {
                SnapshotUnleasedTempMaintenanceFailureKind::OutcomeUnknown
            }
            HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::NotFound
            | HistoryErrorKind::InvalidTransition
            | HistoryErrorKind::InternalState => {
                SnapshotUnleasedTempMaintenanceFailureKind::InternalState
            }
        },
    };
    TaskFailureKind::SnapshotUnleasedTempMaintenance(kind)
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
    let kind = record.kind;
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
    registry.release_task_exclusivity(id, kind, scan_scope.as_deref());
    registry.running_tasks = registry.running_tasks.saturating_sub(1);
    registry.retain_terminal(id, shared.limits.retained_terminal_tasks);
    shared.lifecycle_changed.notify_all();
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;

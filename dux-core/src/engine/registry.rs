use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(all(test, target_os = "macos"))]
use thiserror::Error;

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
use super::cleanup_history_clear::{
    CleanupHistoryClearError, CleanupHistoryClearPreview, CleanupHistoryClearResult,
};
use super::config::EngineConfig;
use super::rust_target_cleanup::{
    RustTargetCleanupError, RustTargetCleanupResult, RustTargetCleanupStartFailure,
};
#[cfg(target_os = "macos")]
use super::rust_target_cleanup::{
    failure_kind as rust_target_cleanup_failure_kind, generate_session_id,
    map_approval_error as map_rust_target_cleanup_approval_error,
    map_handoff_error as map_rust_target_cleanup_handoff_error,
    recovering_result as rust_target_cleanup_recovering_result,
    result as rust_target_cleanup_result,
};
use super::rust_target_plan_review::{
    PendingRustTargetPlanReview, RustTargetPlanReview, RustTargetPlanReviewAdmission,
    RustTargetPlanReviewError, ValidatedPendingRustTargetPlanReview,
};
use super::scan_coverage_details::{
    DurableScanCoverageDetailsPage, DurableScanIssue, DurableScanIssueLocation,
    MAX_SCAN_COVERAGE_DETAIL_PAGE_LIMIT, MAX_SCAN_COVERAGE_LOCATION_COMPONENT_CHARS,
    MAX_SCAN_COVERAGE_LOCATION_COMPONENTS, ScanCoverageDetailsError,
};
use super::settings::{
    CleanupExclusionSource, CleanupExclusions, CleanupExclusionsError, CleanupExclusionsUpdate,
    DirectCargoCodeSignature, DirectCargoEnrollmentError, DirectCargoEnrollmentPreview,
    DirectCargoEnrollmentState, DirectCargoEnrollmentStatus, DirectCargoEnrollmentUpdate,
    DirectCargoSignatureClass, DiskPressurePolicy, DiskPressurePolicyError,
    DiskPressurePolicySource, DiskPressurePolicyUpdate, PermanentCleanupPolicy,
    PermanentCleanupPolicyError, PermanentCleanupPolicySource, PermanentCleanupPolicyUpdate,
    SnapshotRetentionCap, SnapshotRetentionCapError, SnapshotRetentionCapSource,
    SnapshotRetentionCapUpdate,
};
use super::snapshot_review::{
    MAX_SNAPSHOT_REVIEW_CATEGORY_BYTES, MAX_SNAPSHOT_REVIEW_CATEGORY_ROOTS,
    SnapshotReviewCategoryRoot, SnapshotReviewError, SnapshotReviewOwner, SnapshotReviewSession,
    category_path_bytes, map_repository_error as map_snapshot_review_error,
};
use super::task::{
    CancelOutcome, CandidateEvaluationRecoveryError,
    CandidateEvaluationRecoveryMaintenanceFailureKind,
    CandidateEvaluationRecoveryMaintenanceOutcome, CandidateEvaluationRecoveryMaintenanceResult,
    CandidateEvaluationRecoveryMaintenanceStartOutcome, CandidateEvaluationRecoveryOutcome,
    CandidateEvaluationTaskFailureKind, CandidateEvaluationTaskStatus, CandidateHistoryError,
    CloseOutcome, DurableCandidateEvaluation, DurableCandidateEvaluationStatus,
    DurableCandidateStatus, DurableCandidateSummary, DurableScanCounts, DurableScanCoverage,
    DurableScanStatus, DurableScanSummary, EngineLifecycle, EngineOpenError, FormatSizeBatchResult,
    FormattedSizeEntry, HistoryMaintenanceFailureKind, HistoryMaintenanceResult,
    HistoryMaintenanceStartOutcome, RecentScanHistory, ScanHistoryError,
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
#[cfg(any(test, target_os = "macos"))]
use crate::cleanup::capacity::CleanupCapacitySampler;
#[cfg(target_os = "macos")]
use crate::cleanup::capacity::MacOSCleanupCapacitySampler;
use crate::cleanup::executor::{
    TrashAdmissionError, TrashExecutionError, TrashPlatformError, TrashSelectionExecutionError,
    UnsettledTrashAdmission, UnsettledTrashEffect,
};
#[cfg(test)]
use crate::cleanup::permanent_safe::execute_rust_target_session_with_capacity_and_clock_for_test;
use crate::cleanup::permanent_safe::{
    DescriptorRelativePermanentSafeDriver, PermanentSafeExecutionError,
    PermanentSafeRemovalSummary, PermanentSafeSessionSummary, UnsettledPermanentSafeEffect,
    execute_rust_target_contents, execute_rust_target_session,
    execute_rust_target_session_with_capacity,
};
use crate::cleanup::{TrashEffectRequest, TrashPlatformResult, TrashSelectionError};
use crate::domain::{
    CANDIDATE_CATALOG_SCHEMA_VERSION, CANDIDATE_CATALOG_SHA256, CANDIDATE_CONTEXT_FORMAT_VERSION,
    CANDIDATE_EVALUATOR_REVISION, CandidateEvaluationError, CandidateId,
    CandidateSnapshotReplayError, CleanupPlanId, Evidence, ScanCoverage, ScanId,
    candidate_evaluation_context_digest_sha256, evaluate_completed_scan_candidates,
    replay_snapshot_candidate_evaluation, validate_bundled_candidate_catalog,
};
use crate::path_validation::{FilesystemIdentity, capture_scan_root, validate_scan_root};
#[cfg(any(test, target_os = "macos"))]
use crate::persistence::CleanupTrigger;
use crate::persistence::snapshot::from_scan::prepare_completed_scan;
use crate::persistence::snapshot::{
    HostValue, SnapshotCodecErrorKind, SnapshotOrphanReconciliationBatchOutcome,
    SnapshotProvisioningStageReconciliationBatchOutcome, SnapshotReference, SnapshotRepository,
    SnapshotRepositoryErrorKind, SnapshotRetentionBatchOutcome, SnapshotStorageErrorKind,
    SnapshotStoreAccess, SnapshotTerminalTempReconciliationBatchOutcome,
    SnapshotUnleasedTempReconciliationBatchOutcome,
};
use crate::persistence::{
    CandidateEvaluationCompletion, CandidateEvaluationFailureKind, CandidateEvaluationIdentity,
    CandidateEvaluationObservation, CandidateEvaluationRecord, CandidateEvaluationStatus,
    CandidateHistoryStatus, CandidateReviewAction, CleanupHistoryClearStoreError, CleanupSessionId,
    CompleteCandidateRecord, HistoryErrorKind, HostPathObservationEncoding,
    MAX_RECENT_SCAN_HISTORY_LIMIT, NewCandidateRecord, NewScanRecord, ScanCompletionRecord,
    ScanCounts, ScanStatus, SnapshotReviewPurpose, StoredCleanupErrorCategory,
    StoredCleanupHistoryCursor, StoredCleanupHistoryObservation, StoredCleanupItemStatus,
    StoredCleanupItemSummary, StoredCleanupMode, StoredCleanupRecordFormat,
    StoredCleanupSessionStatus, StoredCleanupSessionSummary, StoredCleanupStatusCounts,
    StoredCleanupTrigger, TerminalScanStatus, observe_host_path,
};
use crate::persistence::{
    CargoCodeSignatureRecord, CargoEnrollmentSetting, CargoEnrollmentSettingUpdate,
    CargoEnrollmentState, CargoSignatureClass, CleanupExclusionSetting,
    CleanupExclusionSettingSource, CleanupExclusionSettingUpdate, DiskPressurePolicySetting,
    DiskPressurePolicySettingSource, DiskPressurePolicySettingUpdate, PermanentCleanupSetting,
    PermanentCleanupSettingSource, PermanentCleanupSettingUpdate, SnapshotRetentionCapSetting,
    SnapshotRetentionCapSettingSource, SnapshotRetentionCapSettingUpdate,
};
use crate::persistence::{CleanupJournalLease, DatabaseStatus, StoreCoordinator};
#[cfg(test)]
use crate::persistence::{NewCleanupSessionRecord, StoredCandidateRecord};
#[cfg(target_os = "macos")]
use crate::planner::ExactPathHandoffError;
use crate::planner::{ApprovedCleanupSession, ApprovedTrustedReviewedCleanupPlan};
#[cfg(unix)]
use crate::planner::{
    CleanupSessionStartError, RustTargetLiveWitness, RustTargetPipelineError, RustTargetPlanFacts,
    RustTargetPlanReviewFailure, prepare_rust_target_live_input,
};
#[cfg(all(test, target_os = "macos"))]
use crate::planner::{RustTargetJournalRequest, begin_rust_target_cleanup_session};
#[cfg(target_os = "macos")]
use crate::planner::{
    RustTargetPromotion, prepare_rust_target_plan_facts, prepare_rust_target_promotion,
    review_rust_target_plan_facts,
};
use crate::scanner::{
    CancellationToken, ScanConfig, ScanMessage, ScanObjectIdentity, ScanTermination, Scanner,
};
use crate::tree::NodeId;

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
    ScanRecoveryMaintenance(Arc<ScanRecoveryMaintenanceResult>),
    CandidateEvaluationRecoveryMaintenance(Arc<CandidateEvaluationRecoveryMaintenanceResult>),
    SnapshotRetention(Arc<SnapshotRetentionResult>),
    SnapshotOrphanMaintenance(Arc<SnapshotOrphanMaintenanceResult>),
    SnapshotProvisioningStageMaintenance(Arc<SnapshotProvisioningStageMaintenanceResult>),
    SnapshotTerminalTempMaintenance(Arc<SnapshotTerminalTempMaintenanceResult>),
    SnapshotUnleasedTempMaintenance(Arc<SnapshotUnleasedTempMaintenanceResult>),
    PermanentSafeCleanup(Arc<RustTargetCleanupResult>),
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

    fn engine_is_open(&self) -> bool {
        self.shared.lock_registry_recover().lifecycle == EngineLifecycle::Open
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

    /// Atomically order cancellation against durable running-scan recovery.
    /// If this wins, later cancellation remains intent and cannot rewrite the
    /// exact persistence batch outcome.
    fn try_begin_scan_recovery_maintenance_batch(&self) -> bool {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::ScanRecoveryMaintenance
            && record.phase == TaskPhase::Running
            && !record.cancellation_requested
        {
            record.push_event(
                TaskEventKind::ScanRecoveryMaintenanceBatchApplying,
                event_limit,
            );
            true
        } else {
            false
        }
    }

    fn report_scan_recovery_maintenance_finished(&self, kind: TaskEventKind) {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::ScanRecoveryMaintenance
            && !record.phase.is_terminal()
        {
            record.push_event(kind, event_limit);
        }
    }

    /// Atomically order cancellation against replaying one exact pending
    /// candidate evaluation. Once this wins, later cancellation cannot hide
    /// the exact replay result or failure.
    fn try_begin_candidate_evaluation_recovery_maintenance(&self) -> bool {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::CandidateEvaluationRecoveryMaintenance
            && record.phase == TaskPhase::Running
            && !record.cancellation_requested
        {
            record.push_event(
                TaskEventKind::CandidateEvaluationRecoveryMaintenanceApplying,
                event_limit,
            );
            true
        } else {
            false
        }
    }

    fn report_candidate_evaluation_recovery_maintenance_finished(&self, kind: TaskEventKind) {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::CandidateEvaluationRecoveryMaintenance
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
    /// physically remove an exact marker-owned provisioning stage.
    fn try_begin_snapshot_provisioning_stage_maintenance_batch(&self) -> bool {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::SnapshotProvisioningStageMaintenance
            && record.phase == TaskPhase::Running
            && !record.cancellation_requested
        {
            record.push_event(
                TaskEventKind::SnapshotProvisioningStageMaintenanceBatchApplying,
                event_limit,
            );
            true
        } else {
            false
        }
    }

    fn report_snapshot_provisioning_stage_maintenance_finished(&self, kind: TaskEventKind) {
        let mut registry = self.shared.lock_registry_recover();
        let event_limit = self.shared.limits.events_per_task;
        if let Some(record) = registry.records.get_mut(&self.id)
            && record.kind == TaskKind::SnapshotProvisioningStageMaintenance
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
    active_scan_recovery_maintenance: Option<TaskId>,
    active_candidate_evaluation_recovery_maintenance: Option<TaskId>,
    active_history_maintenance: Option<TaskId>,
    active_snapshot_retention: Option<TaskId>,
    active_snapshot_orphan_maintenance: Option<TaskId>,
    active_snapshot_provisioning_stage_maintenance: Option<TaskId>,
    active_snapshot_terminal_temp_maintenance: Option<TaskId>,
    active_snapshot_unleased_temp_maintenance: Option<TaskId>,
    active_cleanup_operation: Option<ActiveCleanupOperation>,
    next_trash_reservation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActiveCleanupOperation {
    PermanentSafe(TaskId),
    Trash(u64),
    Quarantined,
}

/// A retained lock/claim prevents any second cleanup after an ambiguous
/// journal transition. It is intentionally never inspected or retried by
/// ordinary task orchestration; process-lifetime quarantine prevents a
/// same-process reopen from outrunning durable restart recovery.
#[allow(
    dead_code,
    reason = "fields are retained capabilities, not observations"
)]
enum QuarantinedCleanup {
    Lease(CleanupJournalLease),
    TrashAdmission(Box<UnsettledTrashAdmission>),
    TrashEffect(Box<UnsettledTrashEffect>),
    Session {
        session: Box<ApprovedCleanupSession>,
        unsettled_effect: Option<Box<UnsettledPermanentSafeEffect>>,
    },
}

/// Ambiguous cleanup authority cannot be handed to same-process recovery:
/// process liveness intentionally ignores the random owner nonce. Retain every
/// exact capability for the rest of the process and key exclusion by the
/// process-unique store coordinator. A later engine opened on the same
/// physical store receives that same coordinator while this quarantine holds
/// it alive and therefore remains fail-closed.
struct ProcessQuarantinedCleanup {
    store: Arc<StoreCoordinator>,
    _authority: QuarantinedCleanup,
}

static PROCESS_CLEANUP_QUARANTINE: OnceLock<Mutex<Vec<ProcessQuarantinedCleanup>>> =
    OnceLock::new();

fn process_cleanup_quarantine() -> &'static Mutex<Vec<ProcessQuarantinedCleanup>> {
    PROCESS_CLEANUP_QUARANTINE.get_or_init(|| Mutex::new(Vec::new()))
}

fn store_is_quarantined(
    quarantine: &[ProcessQuarantinedCleanup],
    store: &Arc<StoreCoordinator>,
) -> bool {
    quarantine
        .iter()
        .any(|entry| Arc::ptr_eq(&entry.store, store))
}

fn quarantine_cleanup_for_store(
    store: &Arc<StoreCoordinator>,
    shared: &Shared,
    cleanup: QuarantinedCleanup,
) {
    let mut quarantine = process_cleanup_quarantine()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    quarantine.push(ProcessQuarantinedCleanup {
        store: Arc::clone(store),
        _authority: cleanup,
    });
    shared.lock_registry_recover().active_cleanup_operation =
        Some(ActiveCleanupOperation::Quarantined);
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
            active_scan_recovery_maintenance: None,
            active_candidate_evaluation_recovery_maintenance: None,
            active_history_maintenance: None,
            active_snapshot_retention: None,
            active_snapshot_orphan_maintenance: None,
            active_snapshot_provisioning_stage_maintenance: None,
            active_snapshot_terminal_temp_maintenance: None,
            active_snapshot_unleased_temp_maintenance: None,
            active_cleanup_operation: None,
            next_trash_reservation: 1,
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
        if kind == TaskKind::ScanRecoveryMaintenance
            && self.active_scan_recovery_maintenance == Some(id)
        {
            self.active_scan_recovery_maintenance = None;
        }
        if kind == TaskKind::CandidateEvaluationRecoveryMaintenance
            && self.active_candidate_evaluation_recovery_maintenance == Some(id)
        {
            self.active_candidate_evaluation_recovery_maintenance = None;
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
        if kind == TaskKind::SnapshotProvisioningStageMaintenance
            && self.active_snapshot_provisioning_stage_maintenance == Some(id)
        {
            self.active_snapshot_provisioning_stage_maintenance = None;
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
        if kind == TaskKind::PermanentSafeCleanup
            && self.active_cleanup_operation == Some(ActiveCleanupOperation::PermanentSafe(id))
        {
            self.active_cleanup_operation = None;
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
    owner: Arc<()>,
    config: EngineConfig,
    store: Arc<StoreCoordinator>,
    snapshots: Arc<SnapshotRepository>,
    snapshot_review_owner: Arc<SnapshotReviewOwner>,
    startup_volume_pressure: Mutex<super::volume_status::StartupVolumePressureBaseline>,
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

struct TrashCleanupReservation {
    shared: Arc<Shared>,
    token: u64,
}

impl Drop for TrashCleanupReservation {
    fn drop(&mut self) {
        let mut registry = self.shared.lock_registry_recover();
        if registry.active_cleanup_operation == Some(ActiveCleanupOperation::Trash(self.token)) {
            registry.active_cleanup_operation = None;
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
#[derive(Debug, Error)]
pub(crate) enum RustTargetPlanExecutionError {
    #[error("Rust-target planner/journal handoff failed: {0}")]
    Handoff(#[source] ExactPathHandoffError),
    #[error("Rust-target execution failed: {0}")]
    Execution(#[source] PermanentSafeExecutionError),
}

impl EngineHandle {
    fn reserve_trash_cleanup(&self) -> Result<TrashCleanupReservation, TrashSelectionError> {
        let quarantine = process_cleanup_quarantine()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if store_is_quarantined(&quarantine, &self.inner.store) {
            return Err(TrashSelectionError::OutcomeUnknown);
        }
        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| TrashSelectionError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(TrashSelectionError::InternalState);
        }
        match registry.active_cleanup_operation {
            Some(ActiveCleanupOperation::Quarantined) => {
                return Err(TrashSelectionError::OutcomeUnknown);
            }
            Some(_) => return Err(TrashSelectionError::Busy),
            None => {}
        }
        let token = registry.next_trash_reservation;
        registry.next_trash_reservation = registry.next_trash_reservation.wrapping_add(1).max(1);
        registry.active_cleanup_operation = Some(ActiveCleanupOperation::Trash(token));
        Ok(TrashCleanupReservation {
            shared: Arc::clone(&self.inner.shared),
            token,
        })
    }

    #[cfg(all(test, target_os = "macos"))]
    fn quarantine_cleanup(&self, cleanup: QuarantinedCleanup) {
        quarantine_cleanup_for_store(&self.inner.store, &self.inner.shared, cleanup);
    }

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
                owner: Arc::new(()),
                config,
                store,
                snapshots,
                snapshot_review_owner: Arc::new(SnapshotReviewOwner::new()),
                startup_volume_pressure: Mutex::new(
                    super::volume_status::StartupVolumePressureBaseline::new(),
                ),
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

    /// Worker-only replay seam reached after maintenance admission and the
    /// Applying cancellation boundary. It must not become a public engine API:
    /// callers may neither supply the clock nor bypass idle task admission.
    fn recover_pending_candidate_evaluation_after_admission(
        store: &StoreCoordinator,
        snapshots: &SnapshotRepository,
        observed_at: SystemTime,
    ) -> Result<CandidateEvaluationRecoveryOutcome, CandidateEvaluationRecoveryError> {
        let Some((scan, pending)) = store
            .load_pending_candidate_evaluation()
            .map_err(|error| map_candidate_recovery_error(error.kind))?
        else {
            return Ok(CandidateEvaluationRecoveryOutcome::NoPending);
        };
        let has_more = pending.has_more();
        let record = pending.record();
        let reference = scan
            .snapshot()
            .ok_or(HistoryErrorKind::CorruptData)
            .map_err(map_candidate_recovery_error)?;
        let request = record
            .request_for_snapshot(reference)
            .map_err(|error| map_candidate_recovery_error(error.kind))?;
        let completed_at = canonical_recovery_evaluation_time(observed_at, record.scheduled_at())
            .ok_or(HistoryErrorKind::InvalidInput)
            .map_err(map_candidate_recovery_error)?;

        let settle_failure = |kind: CandidateEvaluationFailureKind| {
            let completion = CandidateEvaluationCompletion::failed(completed_at, kind)
                .map_err(|error| map_candidate_recovery_error(error.kind))?;
            store
                .record_candidate_evaluation_completed_reconciled(&request, &completion)
                .map_err(|error| map_candidate_recovery_error(error.kind))
        };

        // Identity drift is not a reason to reinterpret an old snapshot with
        // new rules. Record a typed terminal discovery failure and leave all
        // cleanup authority absent.
        if !record.matches_current_scan_observation(&scan) {
            settle_failure(CandidateEvaluationFailureKind::ContextInvalid)?;
            return Ok(CandidateEvaluationRecoveryOutcome::Incompatible { has_more });
        }

        let document = snapshots
            .load_for_candidate_recovery(reference)
            .map_err(|error| {
                map_candidate_recovery_error(map_candidate_recovery_snapshot_error(error.kind))
            })?;
        if document.metadata.scan_id != *scan.id()
            || !document.metadata.root.matches_path(scan.root())
        {
            return Err(map_candidate_recovery_error(HistoryErrorKind::CorruptData));
        }

        let batch = match replay_snapshot_candidate_evaluation(
            &document,
            scan.coverage(),
            record.scheduled_at(),
        ) {
            Ok(batch) => batch,
            Err(error) => {
                settle_failure(candidate_replay_failure_kind(error))?;
                return Ok(CandidateEvaluationRecoveryOutcome::Recovered {
                    candidate_count: 0,
                    has_more,
                });
            }
        };
        if batch.evaluator_revision() != CANDIDATE_EVALUATOR_REVISION
            || batch.catalog_schema_version() != CANDIDATE_CATALOG_SCHEMA_VERSION
            || batch.catalog_digest_sha256() != CANDIDATE_CATALOG_SHA256
            || batch.context_format_version() != CANDIDATE_CONTEXT_FORMAT_VERSION
            || batch.context_digest_sha256()
                != crate::domain::candidate_evaluation_context_digest_for_observation(
                    scan.id(),
                    scan.root(),
                    scan.coverage(),
                    record.scheduled_at(),
                )
        {
            settle_failure(CandidateEvaluationFailureKind::ContextInvalid)?;
            return Ok(CandidateEvaluationRecoveryOutcome::Incompatible { has_more });
        }

        let candidates = batch
            .into_candidates()
            .iter()
            .map(|candidate| NewCandidateRecord::try_from_candidate(candidate, completed_at))
            .collect::<Result<Vec<_>, _>>();
        let candidates = match candidates {
            Ok(candidates) => candidates,
            Err(_) => {
                settle_failure(CandidateEvaluationFailureKind::CandidateInvalid)?;
                return Ok(CandidateEvaluationRecoveryOutcome::Recovered {
                    candidate_count: 0,
                    has_more,
                });
            }
        };
        let candidate_count = u32::try_from(candidates.len())
            .map_err(|_| map_candidate_recovery_error(HistoryErrorKind::QueryLimitExceeded))?;
        if !CandidateEvaluationCompletion::batch_fits_materialization_budget(&candidates)
            .map_err(|error| map_candidate_recovery_error(error.kind))?
        {
            settle_failure(CandidateEvaluationFailureKind::LimitExceeded)?;
            return Ok(CandidateEvaluationRecoveryOutcome::Recovered {
                candidate_count: 0,
                has_more,
            });
        }
        let completion = CandidateEvaluationCompletion::succeeded(completed_at, candidates)
            .map_err(|error| map_candidate_recovery_error(error.kind))?;
        store
            .record_candidate_evaluation_completed_reconciled(&request, &completion)
            .map_err(|error| map_candidate_recovery_error(error.kind))?;
        Ok(CandidateEvaluationRecoveryOutcome::Recovered {
            candidate_count,
            has_more,
        })
    }

    /// Evaluate one startup-volume capacity observation, retaining durable
    /// history when evidence is complete and a bounded session pressure
    /// baseline otherwise. The returned DTO is path-free and grants no cleanup
    /// authority.
    pub fn observe_volume_capacity(
        &self,
        observation: super::VolumeCapacityObservation,
    ) -> Result<super::VolumeCapacityStatus, super::VolumeCapacityStatusError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(super::VolumeCapacityStatusError::Closed);
        }
        super::volume_status::observe_volume_capacity(
            &self.inner.store,
            &self.inner.startup_volume_pressure,
            observation,
        )
    }

    /// Load bounded path-free capacity changes and UTC-day chart points for a
    /// stable volume. This is presentation telemetry only and grants no
    /// cleanup authority.
    pub fn capacity_trend(
        &self,
        volume_id: &crate::domain::VolumeId,
        anchor_at: SystemTime,
    ) -> Result<super::CapacityTrend, super::VolumeCapacityStatusError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(super::VolumeCapacityStatusError::Closed);
        }
        super::volume_status::load_capacity_trend(&self.inner.store, volume_id, anchor_at)
    }

    /// Acquire one exact Explorer-only review lease by durable scan identity.
    ///
    /// The scan observation used to find the reference grants no authority:
    /// the snapshot repository repeats the complete history, tombstone,
    /// identity, and file validation while acquiring its durable pin.
    pub fn acquire_explorer_snapshot_review(
        &self,
        scan_id: &ScanId,
    ) -> Result<SnapshotReviewSession, SnapshotReviewError> {
        self.acquire_explorer_snapshot_review_with_hook(scan_id, || {})
    }

    #[cfg(test)]
    fn acquire_explorer_snapshot_review_with_test_hook(
        &self,
        scan_id: &ScanId,
        after_lifecycle_preflight: impl FnOnce(),
    ) -> Result<SnapshotReviewSession, SnapshotReviewError> {
        self.acquire_explorer_snapshot_review_with_hook(scan_id, after_lifecycle_preflight)
    }

    fn acquire_explorer_snapshot_review_with_hook(
        &self,
        scan_id: &ScanId,
        after_lifecycle_preflight: impl FnOnce(),
    ) -> Result<SnapshotReviewSession, SnapshotReviewError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(SnapshotReviewError::Closed);
        }
        after_lifecycle_preflight();
        let scan = self
            .inner
            .store
            .load_scan(scan_id)
            .map_err(|error| {
                map_snapshot_review_error(SnapshotRepositoryErrorKind::History(error.kind))
            })?
            .ok_or(SnapshotReviewError::ScanNotFound)?;
        let reference = scan
            .snapshot()
            .cloned()
            .ok_or(SnapshotReviewError::SnapshotUnavailable)?;
        self.acquire_explorer_snapshot_review_reference(scan_id, &reference)
    }

    /// Acquire the newest non-tombstoned completed snapshot as one exact
    /// Explorer review. The history lookup is selection only; repository lease
    /// acquisition repeats the durable and filesystem safety validation.
    pub fn acquire_latest_explorer_snapshot_review(
        &self,
    ) -> Result<SnapshotReviewSession, SnapshotReviewError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(SnapshotReviewError::Closed);
        }
        let scan = self
            .inner
            .store
            .load_latest_available_snapshot_scan()
            .map_err(|error| {
                map_snapshot_review_error(SnapshotRepositoryErrorKind::History(error.kind))
            })?
            .ok_or(SnapshotReviewError::SnapshotUnavailable)?;
        let reference = scan
            .snapshot()
            .cloned()
            .ok_or(SnapshotReviewError::InternalState)?;
        self.acquire_explorer_snapshot_review_reference(scan.id(), &reference)
    }

    fn acquire_explorer_snapshot_review_reference(
        &self,
        scan_id: &ScanId,
        reference: &SnapshotReference,
    ) -> Result<SnapshotReviewSession, SnapshotReviewError> {
        let session_identity = self.inner.snapshot_review_owner.issue_session_identity()?;
        let lease = self
            .inner
            .snapshots
            .acquire_review_lease(
                reference,
                SnapshotReviewPurpose::Explorer,
                SystemTime::now(),
            )
            .map_err(|error| map_snapshot_review_error(error.kind))?;
        let category_roots = self.snapshot_review_category_roots(scan_id);
        // Linearize successful acquisition before close. If close won while
        // storage validation was in flight, do not publish a new session and
        // remove its exact durable pin while the lease is still available.
        if self.lifecycle() != EngineLifecycle::Open {
            let _ = lease.release();
            return Err(SnapshotReviewError::Closed);
        }
        Ok(SnapshotReviewSession::new(
            Arc::clone(&self.inner.snapshot_review_owner),
            session_identity,
            scan_id.clone(),
            lease,
            category_roots,
        ))
    }

    fn snapshot_review_category_roots(&self, scan_id: &ScanId) -> Vec<SnapshotReviewCategoryRoot> {
        let Ok(CandidateEvaluationObservation::Succeeded(evaluation)) =
            self.inner.store.load_candidate_evaluation_for_scan(scan_id)
        else {
            // Categories are optional historical display metadata. Missing,
            // legacy, pending, failed, busy, or corrupt discovery state must
            // not prevent review of an independently validated snapshot.
            return Vec::new();
        };
        let Some((root_count, root_bytes)) = evaluation.candidates().iter().try_fold(
            (0_usize, 0_usize),
            |(count, bytes), candidate| {
                let count = count.checked_add(candidate.paths().len())?;
                let bytes = candidate.paths().iter().try_fold(bytes, |total, path| {
                    total.checked_add(category_path_bytes(path))
                })?;
                Some((count, bytes))
            },
        ) else {
            return Vec::new();
        };
        if root_count > MAX_SNAPSHOT_REVIEW_CATEGORY_ROOTS
            || root_bytes > MAX_SNAPSHOT_REVIEW_CATEGORY_BYTES
        {
            return Vec::new();
        }
        let mut roots = Vec::new();
        if roots.try_reserve_exact(root_count).is_err() {
            return Vec::new();
        }
        for candidate in evaluation.candidates() {
            roots.extend(
                candidate
                    .paths()
                    .iter()
                    .cloned()
                    .map(|path| SnapshotReviewCategoryRoot::new(path, candidate.category())),
            );
        }
        roots
    }

    /// Inspect one exact direct Cargo executable without changing durable
    /// settings. The returned preview is bound to this engine session and must
    /// be explicitly consumed by `commit_direct_cargo_enrollment`.
    pub fn inspect_direct_cargo_enrollment(
        &self,
        executable: &Path,
    ) -> Result<DirectCargoEnrollmentPreview, DirectCargoEnrollmentError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DirectCargoEnrollmentError::Closed);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = executable;
            Err(DirectCargoEnrollmentError::UnsupportedPlatform)
        }
        #[cfg(target_os = "macos")]
        {
            let inner = crate::planner::inspect_direct_cargo_enrollment(
                Arc::clone(&self.inner.store),
                executable,
            )
            .map_err(map_direct_cargo_validation_error)?;
            if self.lifecycle() != EngineLifecycle::Open {
                return Err(DirectCargoEnrollmentError::Closed);
            }
            let path = inner.executable_path().to_path_buf();
            let executable_sha256 = inner.executable_sha256();
            let code_signature = public_direct_cargo_signature(inner.code_signature());
            Ok(DirectCargoEnrollmentPreview {
                path,
                executable_sha256,
                code_signature,
                inner,
                owner: Arc::clone(&self.inner.owner),
            })
        }
    }

    /// Persist exactly one previously inspected Cargo identity. A preview from
    /// another engine session is rejected even when both sessions use the same
    /// database path.
    pub fn commit_direct_cargo_enrollment(
        &self,
        preview: DirectCargoEnrollmentPreview,
    ) -> Result<DirectCargoEnrollmentUpdate, DirectCargoEnrollmentError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DirectCargoEnrollmentError::Closed);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = preview;
            Err(DirectCargoEnrollmentError::UnsupportedPlatform)
        }
        #[cfg(target_os = "macos")]
        {
            if !Arc::ptr_eq(&preview.owner, &self.inner.owner)
                || !preview.inner.belongs_to(&self.inner.store)
            {
                return Err(DirectCargoEnrollmentError::WrongEngine);
            }
            crate::planner::commit_direct_cargo_enrollment(preview.inner)
                .map(public_direct_cargo_update)
                .map_err(map_direct_cargo_commit_error)
        }
    }

    /// Load the revisioned explicit Cargo trust state. This observation grants
    /// discovery permission only and cannot authorize cleanup.
    pub fn direct_cargo_enrollment_status(
        &self,
    ) -> Result<DirectCargoEnrollmentStatus, DirectCargoEnrollmentError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DirectCargoEnrollmentError::Closed);
        }
        self.inner
            .store
            .load_cargo_enrollment()
            .map(public_direct_cargo_status)
            .map_err(|error| map_direct_cargo_history_error(error.kind))
    }

    /// Revoke any active direct Cargo enrollment. A retained revisioned
    /// tombstone prevents an older preview from recreating the prior state.
    pub fn revoke_direct_cargo_enrollment(
        &self,
    ) -> Result<DirectCargoEnrollmentUpdate, DirectCargoEnrollmentError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DirectCargoEnrollmentError::Closed);
        }
        self.inner
            .store
            .revoke_cargo_executable()
            .map(public_direct_cargo_update)
            .map_err(|error| map_direct_cargo_history_error(error.kind))
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

    /// Load the effective deterministic disk-pressure policy. This is
    /// classification policy only and grants no cleanup or scheduling
    /// authority.
    pub fn disk_pressure_policy(&self) -> Result<DiskPressurePolicy, DiskPressurePolicyError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DiskPressurePolicyError::Closed);
        }
        self.inner
            .store
            .load_disk_pressure_policy()
            .map(public_disk_pressure_policy)
            .map_err(|error| map_disk_pressure_policy_error(error.kind))
    }

    /// Persist one validated explicit pressure policy. This method never
    /// acquires the capacity-session mutex; observations retain the global
    /// session-then-store lock order.
    pub fn set_disk_pressure_policy(
        &self,
        config: crate::domain::DiskPressureConfig,
    ) -> Result<DiskPressurePolicyUpdate, DiskPressurePolicyError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DiskPressurePolicyError::Closed);
        }
        self.inner
            .store
            .set_disk_pressure_policy(config)
            .map(public_disk_pressure_policy_update)
            .map_err(|error| map_disk_pressure_policy_error(error.kind))
    }

    /// Restore the versioned default while retaining a new durable Default
    /// revision when an explicit policy was active.
    pub fn reset_disk_pressure_policy(
        &self,
    ) -> Result<DiskPressurePolicyUpdate, DiskPressurePolicyError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DiskPressurePolicyError::Closed);
        }
        self.inner
            .store
            .reset_disk_pressure_policy()
            .map(public_disk_pressure_policy_update)
            .map_err(|error| map_disk_pressure_policy_error(error.kind))
    }

    /// Load the effective global permanent-cleanup switch. This is a kill
    /// switch only and carries no plan, target, or effect authority.
    pub fn permanent_cleanup_policy(
        &self,
    ) -> Result<PermanentCleanupPolicy, PermanentCleanupPolicyError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(PermanentCleanupPolicyError::Closed);
        }
        self.inner
            .store
            .load_permanent_cleanup_setting()
            .map(public_permanent_cleanup_policy)
            .map_err(|error| map_permanent_cleanup_policy_error(error.kind))
    }

    /// Persist the global permanent-cleanup switch. Disabling takes the same
    /// cleanup exclusion as the final journal gate and therefore cannot race
    /// an already-admitted permanent effect.
    pub fn set_permanent_cleanup_enabled(
        &self,
        enabled: bool,
    ) -> Result<PermanentCleanupPolicyUpdate, PermanentCleanupPolicyError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(PermanentCleanupPolicyError::Closed);
        }
        self.inner
            .store
            .set_permanent_cleanup_enabled(enabled)
            .map(public_permanent_cleanup_policy_update)
            .map_err(|error| map_permanent_cleanup_policy_error(error.kind))
    }

    /// Restore the versioned default (enabled) while retaining a new durable
    /// revision when an explicit switch value was active.
    pub fn reset_permanent_cleanup(
        &self,
    ) -> Result<PermanentCleanupPolicyUpdate, PermanentCleanupPolicyError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(PermanentCleanupPolicyError::Closed);
        }
        self.inner
            .store
            .reset_permanent_cleanup()
            .map(public_permanent_cleanup_policy_update)
            .map_err(|error| map_permanent_cleanup_policy_error(error.kind))
    }

    /// Load the deny-only user exclusion set. Entries suppress matching
    /// cleanup targets but never authorize arbitrary paths.
    pub fn cleanup_exclusions(&self) -> Result<CleanupExclusions, CleanupExclusionsError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CleanupExclusionsError::Closed);
        }
        self.inner
            .store
            .load_cleanup_exclusions()
            .map(public_cleanup_exclusions)
            .map_err(|error| map_cleanup_exclusions_error(error.kind))
    }

    /// Execute one explicit Explorer Trash selection through the core-owned
    /// journal fence. The callback receives only a one-shot request created
    /// after the retained review target has been revalidated; it cannot choose
    /// or retry a path.
    pub fn execute_explorer_trash_selection<F>(
        &self,
        review: &mut SnapshotReviewSession,
        node_id: u64,
        driver: F,
    ) -> Result<TrashPlatformResult, TrashSelectionError>
    where
        F: FnOnce(TrashEffectRequest) -> TrashPlatformResult,
    {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(TrashSelectionError::InternalState);
        }
        if !review.belongs_to(&self.inner.snapshot_review_owner) {
            return Err(TrashSelectionError::InvalidRequest);
        }
        let _reservation = self.reserve_trash_cleanup()?;
        match crate::cleanup::execute_reviewed_trash_selection(
            &self.inner.store,
            review,
            node_id,
            driver,
        ) {
            Ok(result) => Ok(result),
            Err(TrashSelectionExecutionError::Selection(error)) => Err(error),
            Err(TrashSelectionExecutionError::JournalClaimUnresolved(unresolved)) => {
                quarantine_cleanup_for_store(
                    &self.inner.store,
                    &self.inner.shared,
                    QuarantinedCleanup::Lease(unresolved.into_lease()),
                );
                Err(TrashSelectionError::OutcomeUnknown)
            }
            Err(TrashSelectionExecutionError::ClaimedAdmissionUnresolved(unresolved)) => {
                quarantine_cleanup_for_store(
                    &self.inner.store,
                    &self.inner.shared,
                    QuarantinedCleanup::TrashAdmission(unresolved),
                );
                Err(TrashSelectionError::OutcomeUnknown)
            }
            Err(TrashSelectionExecutionError::UnresolvedEffect(unresolved)) => {
                self.settle_or_quarantine_trash_effect(unresolved)
            }
        }
    }

    fn settle_or_quarantine_trash_effect(
        &self,
        unresolved: Box<UnsettledTrashEffect>,
    ) -> Result<TrashPlatformResult, TrashSelectionError> {
        match unresolved.retry() {
            Ok(Ok(())) => Ok(TrashPlatformResult::Completed),
            Ok(Err(TrashExecutionError::Platform(error))) => {
                Ok(public_trash_platform_result(error))
            }
            Ok(Err(TrashExecutionError::Admission(error))) => Err(map_trash_admission_error(error)),
            Ok(Err(TrashExecutionError::UnresolvedEffect(unresolved))) | Err(unresolved) => {
                let platform_unknown = unresolved.observed_platform_error()
                    == Some(TrashPlatformError::OutcomeUnknown)
                    || unresolved.journal_outcome_is_unknown();
                quarantine_cleanup_for_store(
                    &self.inner.store,
                    &self.inner.shared,
                    QuarantinedCleanup::TrashEffect(unresolved),
                );
                if platform_unknown {
                    Ok(TrashPlatformResult::OutcomeUnknown)
                } else {
                    Err(TrashSelectionError::OutcomeUnknown)
                }
            }
        }
    }

    /// Execute one path from a planner-owned, approved permanent-safe session.
    ///
    /// This is intentionally crate-private: callers must first construct the
    /// non-cloneable approved session through the future deterministic planner
    /// join. The engine supplies the only concrete descriptor-relative driver;
    /// no caller path, callback, FFI handle, AI result, or CLI request can
    /// reach this boundary.
    #[allow(
        dead_code,
        reason = "the deterministic planner-to-engine bridge is staged before FFI/UI orchestration"
    )]
    pub(crate) fn execute_approved_permanent_safe(
        &self,
        session: &mut ApprovedCleanupSession,
        item_ordinal: usize,
        path_ordinal: usize,
        now: SystemTime,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PermanentSafeRemovalSummary, PermanentSafeExecutionError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(PermanentSafeExecutionError::Admission(
                HistoryErrorKind::InvalidTransition,
            ));
        }
        let mut driver = DescriptorRelativePermanentSafeDriver;
        execute_rust_target_contents(
            session,
            item_ordinal,
            path_ordinal,
            now,
            &mut driver,
            cancelled,
        )
    }

    /// Execute every path in one approved permanent-safe session in planner
    /// order. The session remains crate-private until trusted protected-root
    /// and volume grants, centralized orchestration, and user-facing review
    /// are complete.
    #[allow(
        dead_code,
        reason = "the multi-path permanent-safe orchestrator is staged before FFI/UI wiring"
    )]
    pub(crate) fn execute_approved_permanent_safe_session(
        &self,
        session: &mut ApprovedCleanupSession,
        now: SystemTime,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PermanentSafeSessionSummary, PermanentSafeExecutionError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(PermanentSafeExecutionError::Admission(
                HistoryErrorKind::InvalidTransition,
            ));
        }
        let mut driver = DescriptorRelativePermanentSafeDriver;
        execute_rust_target_session(session, now, &mut driver, cancelled)
    }

    /// Consume the private Rust-target facts and journal request through the
    /// same claimed-session executor used by every permanent-safe effect.
    ///
    /// This remains crate-private: callers cannot provide paths, callbacks,
    /// AI output, FFI values, or CLI requests. The journal handoff owns the
    /// final plan/authorization checks; the executor owns descriptor-relative
    /// identity checks, effect receipts, cancellation, and terminalization.
    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn execute_rust_target_plan_facts(
        &self,
        facts: RustTargetPlanFacts,
        request: RustTargetJournalRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PermanentSafeSessionSummary, RustTargetPlanExecutionError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(RustTargetPlanExecutionError::Execution(
                PermanentSafeExecutionError::Admission(HistoryErrorKind::InvalidTransition),
            ));
        }
        let begin = begin_rust_target_cleanup_session(facts, request, &self.inner.store);
        let mut session = match begin {
            Ok(session) => session,
            Err(CleanupSessionStartError::JournalClaimUnresolved(failure)) => match failure.retry()
            {
                Ok(session) => session,
                Err(CleanupSessionStartError::JournalClaimUnresolved(failure)) => {
                    self.quarantine_cleanup(QuarantinedCleanup::Lease(failure.into_lease()));
                    return Err(RustTargetPlanExecutionError::Execution(
                        PermanentSafeExecutionError::Admission(HistoryErrorKind::OutcomeUnknown),
                    ));
                }
                Err(CleanupSessionStartError::ClaimedPlanUnresolved(failure)) => {
                    self.quarantine_cleanup(QuarantinedCleanup::Session {
                        session: Box::new(failure.into_session()),
                        unsettled_effect: None,
                    });
                    return Err(RustTargetPlanExecutionError::Execution(
                        PermanentSafeExecutionError::Admission(HistoryErrorKind::OutcomeUnknown),
                    ));
                }
                Err(CleanupSessionStartError::Handoff(error)) => {
                    return Err(RustTargetPlanExecutionError::Handoff(error));
                }
            },
            Err(CleanupSessionStartError::ClaimedPlanUnresolved(failure)) => {
                self.quarantine_cleanup(QuarantinedCleanup::Session {
                    session: Box::new(failure.into_session()),
                    unsettled_effect: None,
                });
                return Err(RustTargetPlanExecutionError::Execution(
                    PermanentSafeExecutionError::Admission(HistoryErrorKind::OutcomeUnknown),
                ));
            }
            Err(CleanupSessionStartError::Handoff(error)) => {
                return Err(RustTargetPlanExecutionError::Handoff(error));
            }
        };
        match self.execute_approved_permanent_safe_session_with_bound_capacity(
            &mut session,
            SystemTime::now(),
            cancelled,
        ) {
            Ok(summary) => Ok(summary),
            Err(PermanentSafeExecutionError::UnsettledEffect(unsettled_effect)) => {
                self.quarantine_cleanup(QuarantinedCleanup::Session {
                    session: Box::new(session),
                    unsettled_effect: Some(unsettled_effect),
                });
                Err(RustTargetPlanExecutionError::Execution(
                    PermanentSafeExecutionError::Admission(HistoryErrorKind::OutcomeUnknown),
                ))
            }
            Err(error) => {
                self.quarantine_cleanup(QuarantinedCleanup::Session {
                    session: Box::new(session),
                    unsettled_effect: None,
                });
                Err(RustTargetPlanExecutionError::Execution(error))
            }
        }
    }

    /// Acquire one exact durable Rust-target candidate as a fresh live
    /// planner witness. This remains crate-private and stops before Cargo
    /// authority, protected-root grants, plans, journal claims, FFI, or
    /// effects; the owned witness lease is the only live capability returned.
    #[cfg(unix)]
    #[allow(
        dead_code,
        reason = "production planner acquisition is staged before Cargo and UI orchestration"
    )]
    pub(crate) fn prepare_rust_target_live_input(
        &self,
        scan_id: &ScanId,
        candidate_id: &CandidateId,
    ) -> Result<(crate::domain::Candidate, RustTargetLiveWitness), RustTargetPipelineError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(RustTargetPipelineError::Closed);
        }
        prepare_rust_target_live_input(
            Arc::clone(&self.inner.store),
            &self.inner.snapshots,
            scan_id,
            candidate_id,
        )
    }

    /// Join the private live Rust-target input to enrolled Cargo and trusted
    /// rule-scope evidence. The returned token still retains `ProtectedPath`
    /// and cannot construct a plan, claim a journal, cross FFI, schedule, or
    /// perform an effect.
    #[cfg(target_os = "macos")]
    #[allow(
        dead_code,
        reason = "production Rust-target authority join is staged before plan/UI orchestration"
    )]
    pub(crate) fn prepare_rust_target_promotion(
        &self,
        scan_id: &ScanId,
        candidate_id: &CandidateId,
    ) -> Result<RustTargetPromotion, RustTargetPipelineError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(RustTargetPipelineError::Closed);
        }
        prepare_rust_target_promotion(
            Arc::clone(&self.inner.store),
            &self.inner.snapshots,
            scan_id,
            candidate_id,
        )
    }

    /// Consume the private macOS Rust-target promotion into typed
    /// permanent-safe plan facts. The facts remain crate-private, retain the
    /// unresolved `ProtectedPath` candidate blocker, and cannot cross into
    /// review, approval, journal, FFI, scheduling, AI, or effects here.
    #[cfg(target_os = "macos")]
    #[allow(
        dead_code,
        reason = "typed Rust-target facts are staged before reviewed-plan/UI orchestration"
    )]
    pub(crate) fn prepare_rust_target_plan_facts(
        &self,
        scan_id: &ScanId,
        candidate_id: &CandidateId,
    ) -> Result<RustTargetPlanFacts, RustTargetPipelineError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(RustTargetPipelineError::Closed);
        }
        prepare_rust_target_plan_facts(
            Arc::clone(&self.inner.store),
            &self.inner.snapshots,
            scan_id,
            candidate_id,
        )
    }

    /// Build one exact, opaque Rust-target plan review from an active Explorer
    /// review. The scan identity comes only from that retained review; this
    /// operation creates no approval, journal row, status transition, or
    /// filesystem effect.
    pub fn prepare_rust_target_plan_review(
        &self,
        review: &mut SnapshotReviewSession,
        candidate_id: &CandidateId,
    ) -> Result<RustTargetPlanReview, RustTargetPlanReviewError> {
        let admission = self.begin_rust_target_plan_review(review, candidate_id)?;
        let pending = self.prepare_admitted_rust_target_plan_review(admission)?;
        self.finalize_rust_target_plan_review(review, pending)
    }

    /// Capture only the exact parent/scan/candidate admission under the
    /// Explorer review lock. The token carries no plan or execution authority.
    pub fn begin_rust_target_plan_review(
        &self,
        review: &SnapshotReviewSession,
        candidate_id: &CandidateId,
    ) -> Result<RustTargetPlanReviewAdmission, RustTargetPlanReviewError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(RustTargetPlanReviewError::Closed);
        }
        if !review.belongs_to(&self.inner.snapshot_review_owner) {
            return Err(RustTargetPlanReviewError::WrongEngine);
        }
        let before = SystemTime::now();
        let parent_expires_before = review
            .validate_and_expires_at(before)
            .map_err(map_snapshot_review_plan_review_error)?;
        Ok(RustTargetPlanReviewAdmission {
            owner: Arc::clone(&self.inner.snapshot_review_owner),
            parent_session_identity: review.session_identity(),
            parent_review_live: review.plan_review_liveness(),
            source_scan_id: review.scan_id().clone(),
            candidate_id: candidate_id.clone(),
            parent_review_expires_at: parent_expires_before,
        })
    }

    /// Perform the expensive deterministic Rust-target pipeline without
    /// retaining the Explorer review mutex. The result is still pending and
    /// cannot be observed until finalized against that exact parent.
    pub fn prepare_admitted_rust_target_plan_review(
        &self,
        admission: RustTargetPlanReviewAdmission,
    ) -> Result<PendingRustTargetPlanReview, RustTargetPlanReviewError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(RustTargetPlanReviewError::Closed);
        }
        if !Arc::ptr_eq(&admission.owner, &self.inner.snapshot_review_owner) {
            return Err(RustTargetPlanReviewError::WrongEngine);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = admission;
            return Err(RustTargetPlanReviewError::UnsupportedPlatform);
        }
        #[cfg(target_os = "macos")]
        {
            let facts = prepare_rust_target_plan_facts(
                Arc::clone(&self.inner.store),
                &self.inner.snapshots,
                &admission.source_scan_id,
                &admission.candidate_id,
            )
            .map_err(map_rust_target_plan_review_pipeline_error)?;
            let created_at = SystemTime::now();
            let plan_id = generate_rust_target_plan_review_id()?;
            let reviewed = review_rust_target_plan_facts(facts, plan_id, created_at)
                .map_err(|_| RustTargetPlanReviewError::ChangedDuringReview)?;
            Ok(PendingRustTargetPlanReview {
                owner: admission.owner,
                parent_session_identity: admission.parent_session_identity,
                parent_review_live: admission.parent_review_live,
                source_scan_id: admission.source_scan_id,
                candidate_id: admission.candidate_id,
                parent_review_expires_at: admission.parent_review_expires_at,
                reviewed,
            })
        }
    }

    /// Revalidate the exact parent after expensive work and publish only the
    /// still-current opaque observation. Any failure drops pending authority.
    pub fn finalize_rust_target_plan_review(
        &self,
        review: &SnapshotReviewSession,
        pending: PendingRustTargetPlanReview,
    ) -> Result<RustTargetPlanReview, RustTargetPlanReviewError> {
        let validated = self.validate_pending_rust_target_plan_review(review, pending)?;
        self.materialize_rust_target_plan_review(validated)
    }

    /// Revalidate only the exact parent binding under the caller's parent
    /// lock. Reviewed-plan filesystem revalidation remains deferred.
    pub fn validate_pending_rust_target_plan_review(
        &self,
        review: &SnapshotReviewSession,
        pending: PendingRustTargetPlanReview,
    ) -> Result<ValidatedPendingRustTargetPlanReview, RustTargetPlanReviewError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(RustTargetPlanReviewError::Closed);
        }
        if !Arc::ptr_eq(&pending.owner, &self.inner.snapshot_review_owner)
            || !review.belongs_to(&self.inner.snapshot_review_owner)
        {
            return Err(RustTargetPlanReviewError::WrongEngine);
        }
        if review.session_identity() != pending.parent_session_identity
            || review.scan_id() != &pending.source_scan_id
        {
            return Err(RustTargetPlanReviewError::ParentReviewUnavailable);
        }
        let observed_at = SystemTime::now();
        let parent_expires_after = review
            .validate_and_expires_at(observed_at)
            .map_err(map_snapshot_review_plan_review_error)?;
        Ok(ValidatedPendingRustTargetPlanReview {
            owner: pending.owner,
            candidate_id: pending.candidate_id,
            parent_review_expires_at: pending.parent_review_expires_at.min(parent_expires_after),
            parent_review_live: pending.parent_review_live,
            reviewed: pending.reviewed,
        })
    }

    /// Materialize the opaque child outside the parent mutex. A caller must
    /// still post-validate that same parent before publishing the result.
    pub fn materialize_rust_target_plan_review(
        &self,
        pending: ValidatedPendingRustTargetPlanReview,
    ) -> Result<RustTargetPlanReview, RustTargetPlanReviewError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(RustTargetPlanReviewError::Closed);
        }
        let observed_at = SystemTime::now();
        RustTargetPlanReview::new(
            pending.owner,
            pending.reviewed,
            &pending.candidate_id,
            pending.parent_review_expires_at,
            pending.parent_review_live,
            observed_at,
        )
    }

    /// Admit one exact Rust-target plan review to the engine-owned,
    /// consume-once permanent-safe task boundary.
    ///
    /// Rejected admission returns the unconsumed opaque review. Acceptance
    /// synchronously consumes and approves the exact child capability before
    /// returning, so releasing the parent review afterward cannot invalidate
    /// queued task authority. Queued cancellation still creates no journal.
    pub fn start_permanent_safe_cleanup(
        &self,
        review: RustTargetPlanReview,
    ) -> Result<TaskId, RustTargetCleanupStartFailure> {
        self.start_permanent_safe_cleanup_with_hook(review, Box::new(|| {}))
    }

    #[cfg(test)]
    fn start_permanent_safe_cleanup_with_before_begin_hook(
        &self,
        review: RustTargetPlanReview,
        before_begin: impl FnOnce() + Send + 'static,
    ) -> Result<TaskId, RustTargetCleanupStartFailure> {
        self.start_permanent_safe_cleanup_with_hook(review, Box::new(before_begin))
    }

    fn start_permanent_safe_cleanup_with_hook(
        &self,
        review: RustTargetPlanReview,
        before_begin: Box<dyn FnOnce() + Send>,
    ) -> Result<TaskId, RustTargetCleanupStartFailure> {
        if !review.belongs_to(&self.inner.snapshot_review_owner) {
            return Err(RustTargetCleanupStartFailure::new(
                RustTargetCleanupError::WrongEngine,
                review,
            ));
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = before_begin;
            return Err(RustTargetCleanupStartFailure::new(
                RustTargetCleanupError::Unavailable,
                review,
            ));
        }
        #[cfg(target_os = "macos")]
        {
            let quarantine = process_cleanup_quarantine()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if store_is_quarantined(&quarantine, &self.inner.store) {
                return Err(RustTargetCleanupStartFailure::new(
                    RustTargetCleanupError::OutcomeUnknown,
                    review,
                ));
            }
            let mut registry = match self.inner.shared.registry.lock() {
                Ok(registry) => registry,
                Err(_) => {
                    return Err(RustTargetCleanupStartFailure::new(
                        RustTargetCleanupError::InternalState,
                        review,
                    ));
                }
            };
            if registry.lifecycle != EngineLifecycle::Open {
                return Err(RustTargetCleanupStartFailure::new(
                    RustTargetCleanupError::Closed,
                    review,
                ));
            }
            if registry.queue.len() >= self.inner.shared.limits.queued_tasks {
                return Err(RustTargetCleanupStartFailure::new(
                    RustTargetCleanupError::QueueFull,
                    review,
                ));
            }
            match registry.active_cleanup_operation {
                Some(ActiveCleanupOperation::Quarantined) => {
                    return Err(RustTargetCleanupStartFailure::new(
                        RustTargetCleanupError::OutcomeUnknown,
                        review,
                    ));
                }
                Some(_) => {
                    return Err(RustTargetCleanupStartFailure::new(
                        RustTargetCleanupError::Busy,
                        review,
                    ));
                }
                None => {}
            }
            let id = match TASK_IDS.allocate() {
                Ok(id) => id,
                Err(_) => {
                    return Err(RustTargetCleanupStartFailure::new(
                        RustTargetCleanupError::InternalState,
                        review,
                    ));
                }
            };
            let approved_at = SystemTime::now();
            let admitted = review
                .into_reviewed_at(&self.inner.snapshot_review_owner, approved_at)
                .map_err(map_rust_target_cleanup_review_error)
                .and_then(|reviewed| {
                    reviewed
                        .approve(approved_at)
                        .map_err(map_rust_target_cleanup_approval_error)
                });
            let store = Arc::clone(&self.inner.store);
            let work: Work = Box::new(move |context| {
                run_permanent_safe_cleanup_task(store, admitted, before_begin, &context)
            });
            let record = TaskRecord::new(
                id,
                TaskKind::PermanentSafeCleanup,
                None,
                self.inner.shared.limits.events_per_task,
            );
            registry.active_cleanup_operation = Some(ActiveCleanupOperation::PermanentSafe(id));
            registry.records.insert(id, record);
            registry.queue.push_back(Job { id, work });
            self.inner.shared.workers_ready.notify_one();
            Ok(id)
        }
    }

    /// Production capacity-aware execution derives its sampler only from the
    /// exact trusted volume retained by the approved plan. Sampling failure
    /// leaves the verified delta unknown and never changes effect authority.
    #[cfg(all(test, target_os = "macos"))]
    fn execute_approved_permanent_safe_session_with_bound_capacity(
        &self,
        session: &mut ApprovedCleanupSession,
        now: SystemTime,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PermanentSafeSessionSummary, PermanentSafeExecutionError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(PermanentSafeExecutionError::Admission(
                HistoryErrorKind::InvalidTransition,
            ));
        }
        let mut sampler = session
            .capacity_scope(SystemTime::now())
            .ok()
            .and_then(MacOSCleanupCapacitySampler::new);
        let mut driver = DescriptorRelativePermanentSafeDriver;
        execute_rust_target_session_with_capacity(
            session,
            now,
            &mut driver,
            cancelled,
            sampler
                .as_mut()
                .map(|sampler| sampler as &mut dyn CleanupCapacitySampler),
        )
    }

    #[cfg(test)]
    pub(crate) fn execute_approved_permanent_safe_session_with_capacity_for_test(
        &self,
        session: &mut ApprovedCleanupSession,
        now: SystemTime,
        cancelled: &dyn Fn() -> bool,
        capacity_sampler: &mut dyn CleanupCapacitySampler,
    ) -> Result<PermanentSafeSessionSummary, PermanentSafeExecutionError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(PermanentSafeExecutionError::Admission(
                HistoryErrorKind::InvalidTransition,
            ));
        }
        let mut driver = DescriptorRelativePermanentSafeDriver;
        execute_rust_target_session_with_capacity(
            session,
            now,
            &mut driver,
            cancelled,
            Some(capacity_sampler),
        )
    }

    #[cfg(test)]
    pub(crate) fn execute_approved_permanent_safe_session_with_capacity_and_clock_for_test(
        &self,
        session: &mut ApprovedCleanupSession,
        now: SystemTime,
        cancelled: &dyn Fn() -> bool,
        capacity_sampler: &mut dyn CleanupCapacitySampler,
        authority_now: &mut dyn FnMut() -> SystemTime,
    ) -> Result<PermanentSafeSessionSummary, PermanentSafeExecutionError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(PermanentSafeExecutionError::Admission(
                HistoryErrorKind::InvalidTransition,
            ));
        }
        let mut driver = DescriptorRelativePermanentSafeDriver;
        execute_rust_target_session_with_capacity_and_clock_for_test(
            session,
            now,
            &mut driver,
            cancelled,
            Some(capacity_sampler),
            authority_now,
        )
    }

    /// Replace the bounded deny-only exclusion set. The core validates and
    /// stores paths losslessly; the setting cannot create a plan or effect.
    pub fn set_cleanup_exclusions(
        &self,
        paths: Vec<std::path::PathBuf>,
    ) -> Result<CleanupExclusionsUpdate, CleanupExclusionsError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CleanupExclusionsError::Closed);
        }
        self.inner
            .store
            .set_cleanup_exclusions(paths)
            .map(public_cleanup_exclusions_update)
            .map_err(|error| map_cleanup_exclusions_error(error.kind))
    }

    /// Remove all user exclusions and restore the empty versioned default.
    pub fn reset_cleanup_exclusions(
        &self,
    ) -> Result<CleanupExclusionsUpdate, CleanupExclusionsError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CleanupExclusionsError::Closed);
        }
        self.inner
            .store
            .reset_cleanup_exclusions()
            .map(public_cleanup_exclusions_update)
            .map_err(|error| map_cleanup_exclusions_error(error.kind))
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

    /// Load one bounded page of durable coverage issues for a stable scan ID.
    /// Locations are root-relative historical display components only; this
    /// method neither opens a snapshot nor grants filesystem authority.
    pub fn scan_coverage_details(
        &self,
        scan_id: &ScanId,
        offset: u16,
        limit: u16,
    ) -> Result<DurableScanCoverageDetailsPage, ScanCoverageDetailsError> {
        if !(1..=MAX_SCAN_COVERAGE_DETAIL_PAGE_LIMIT).contains(&limit) {
            return Err(ScanCoverageDetailsError::InvalidLimit {
                max: MAX_SCAN_COVERAGE_DETAIL_PAGE_LIMIT,
            });
        }
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(ScanCoverageDetailsError::Closed);
        }
        let record = self
            .inner
            .store
            .load_scan(scan_id)
            .map_err(|error| map_scan_coverage_details_error(error.kind))?
            .ok_or(ScanCoverageDetailsError::ScanNotFound)?;
        let coverage = record.coverage();
        let total_issue_records = u16::try_from(coverage.issues().len())
            .map_err(|_| ScanCoverageDetailsError::CorruptData)?;
        if offset > total_issue_records {
            return Err(ScanCoverageDetailsError::InvalidOffset);
        }
        let total_issue_occurrences = coverage
            .issues()
            .iter()
            .map(|issue| u64::from(issue.occurrence_count()))
            .sum();
        let end = usize::from(offset)
            .saturating_add(usize::from(limit))
            .min(coverage.issues().len());
        let issues = coverage.issues()[usize::from(offset)..end]
            .iter()
            .enumerate()
            .map(|(page_index, issue)| {
                let location = issue
                    .path()
                    .map(|path| historical_issue_location(record.root(), path))
                    .transpose()?;
                Ok(DurableScanIssue::new(
                    offset
                        .checked_add(
                            u16::try_from(page_index)
                                .map_err(|_| ScanCoverageDetailsError::CorruptData)?,
                        )
                        .ok_or(ScanCoverageDetailsError::CorruptData)?,
                    issue.kind().into(),
                    issue.occurrence_count(),
                    location,
                ))
            })
            .collect::<Result<Vec<_>, ScanCoverageDetailsError>>()?;
        Ok(DurableScanCoverageDetailsPage::new(
            scan_id.clone(),
            coverage.status(),
            coverage.measured_permille(),
            offset,
            total_issue_records,
            total_issue_occurrences,
            end < coverage.issues().len(),
            issues,
        ))
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

    /// Prepare one short-lived, consume-once confirmation for clearing the
    /// exact current terminal cleanup-history graph. The preview contains no
    /// session IDs, paths, plan facts, claims, or effect authority.
    pub fn prepare_cleanup_history_clear(
        &self,
    ) -> Result<CleanupHistoryClearPreview, CleanupHistoryClearError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CleanupHistoryClearError::Closed);
        }
        let prepared = self
            .inner
            .store
            .prepare_cleanup_history_clear()
            .map_err(map_cleanup_history_clear_store_error)?;
        let monotonic_now = Instant::now();
        let prepared_at = SystemTime::now();
        CleanupHistoryClearPreview::new(&self.inner.store, prepared, prepared_at, monotonic_now)
            .ok_or(CleanupHistoryClearError::CorruptData)
    }

    /// Consume one exact preview and clear only the unchanged terminal
    /// cleanup-history graph. No cleanup target or effect is accepted here.
    pub fn clear_cleanup_history(
        &self,
        preview: CleanupHistoryClearPreview,
    ) -> Result<CleanupHistoryClearResult, CleanupHistoryClearError> {
        self.clear_cleanup_history_at(preview, Instant::now())
    }

    fn clear_cleanup_history_at(
        &self,
        preview: CleanupHistoryClearPreview,
        now: Instant,
    ) -> Result<CleanupHistoryClearResult, CleanupHistoryClearError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CleanupHistoryClearError::Closed);
        }
        if !preview.belongs_to(&self.inner.store) {
            return Err(CleanupHistoryClearError::WrongEngine);
        }
        let info = preview.info_at(now)?;
        let prepared = preview.into_prepared(now)?;
        let result = self
            .inner
            .store
            .clear_cleanup_history(&prepared)
            .map_err(map_cleanup_history_clear_store_error)?;
        if result.sessions_removed != info.session_count() {
            return Err(CleanupHistoryClearError::OutcomeUnknown);
        }
        CleanupHistoryClearResult::new(result.sessions_removed)
            .ok_or(CleanupHistoryClearError::OutcomeUnknown)
    }

    #[cfg(test)]
    fn clear_cleanup_history_at_expiry_for_test(
        &self,
        preview: CleanupHistoryClearPreview,
    ) -> Result<CleanupHistoryClearResult, CleanupHistoryClearError> {
        let expires_at = preview.monotonic_expires_at_for_test();
        self.clear_cleanup_history_at(preview, expires_at)
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

    /// Recover at most one exact running scan whose durable process-instance
    /// owner is proven gone. This sealed idle-only task accepts no scan or
    /// owner input, and exposes only bounded path-free aggregate observations.
    pub fn start_scan_recovery_maintenance(
        &self,
    ) -> Result<ScanRecoveryMaintenanceStartOutcome, StartTaskError> {
        self.start_scan_recovery_maintenance_with_hooks(
            SystemTime::now,
            || {},
            || {},
            || {},
            |store, observed_at| {
                store
                    .run_scan_recovery_batch(observed_at)
                    .map_err(|error| error.kind)
            },
        )
    }

    #[cfg(test)]
    fn start_scan_recovery_maintenance_at(
        &self,
        observed_at: SystemTime,
    ) -> Result<ScanRecoveryMaintenanceStartOutcome, StartTaskError> {
        self.start_scan_recovery_maintenance_with_hooks(
            move || observed_at,
            || {},
            || {},
            || {},
            |store, observed_at| {
                store
                    .run_scan_recovery_batch(observed_at)
                    .map_err(|error| error.kind)
            },
        )
    }

    #[cfg(test)]
    fn start_scan_recovery_maintenance_with_test_hooks(
        &self,
        observed_at: SystemTime,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<ScanRecoveryMaintenanceStartOutcome, StartTaskError> {
        self.start_scan_recovery_maintenance_with_hooks(
            move || observed_at,
            before_batch,
            after_applying,
            after_batch,
            |store, observed_at| {
                store
                    .run_scan_recovery_batch(observed_at)
                    .map_err(|error| error.kind)
            },
        )
    }

    #[cfg(test)]
    fn start_scan_recovery_maintenance_with_test_batch(
        &self,
        observed_at: SystemTime,
        batch: crate::persistence::ScanRecoveryBatchResult,
    ) -> Result<ScanRecoveryMaintenanceStartOutcome, StartTaskError> {
        self.start_scan_recovery_maintenance_with_hooks(
            move || observed_at,
            || {},
            || {},
            || {},
            move |_, _| Ok(batch),
        )
    }

    fn start_scan_recovery_maintenance_with_hooks(
        &self,
        clock: impl FnOnce() -> SystemTime + Send + 'static,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
        run_batch: impl FnOnce(
            &StoreCoordinator,
            SystemTime,
        )
            -> Result<crate::persistence::ScanRecoveryBatchResult, HistoryErrorKind>
        + Send
        + 'static,
    ) -> Result<ScanRecoveryMaintenanceStartOutcome, StartTaskError> {
        if let Some(outcome) = self.scan_recovery_maintenance_preflight()? {
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
        self.submit_scan_recovery_maintenance(Box::new(move |context| {
            if context.is_cancellation_requested() {
                return WorkOutcome::Cancelled(None);
            }
            let observed_at = clock();
            before_batch();
            if !context.try_begin_scan_recovery_maintenance_batch() {
                return WorkOutcome::Cancelled(None);
            }
            // Applying is the point of no return. A later cancellation remains
            // visible intent but cannot suppress an exact durable outcome.
            after_applying();
            match run_batch(&store, observed_at) {
                Ok(batch) => {
                    let result = Arc::new(ScanRecoveryMaintenanceResult::new(
                        batch.observed_at,
                        public_scan_recovery_maintenance_outcome(&batch.outcome),
                        batch.claimed_count_before,
                        batch.claimed_count_after,
                        batch.alive_count,
                        batch.unknown_count,
                        batch.recoverable_count,
                        batch.has_more,
                    ));
                    after_batch();
                    context.report_scan_recovery_maintenance_finished(
                        TaskEventKind::ScanRecoveryMaintenanceBatchFinished {
                            outcome: result.outcome(),
                            claimed_count_before: result.claimed_count_before(),
                            claimed_count_after: result.claimed_count_after(),
                            alive_count: result.alive_count(),
                            unknown_count: result.unknown_count(),
                            recoverable_count: result.recoverable_count(),
                            has_more: result.has_more(),
                        },
                    );
                    WorkOutcome::Succeeded(TaskResult::ScanRecoveryMaintenance(result))
                }
                Err(kind) => WorkOutcome::Failed(map_scan_recovery_maintenance_failure(kind), None),
            }
        }))
    }

    fn scan_recovery_maintenance_preflight(
        &self,
    ) -> Result<Option<ScanRecoveryMaintenanceStartOutcome>, StartTaskError> {
        let registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_scan_recovery_maintenance {
            return Ok(Some(ScanRecoveryMaintenanceStartOutcome::AlreadyActive(
                existing,
            )));
        }
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(Some(ScanRecoveryMaintenanceStartOutcome::DeferredBusy));
        }
        Ok(None)
    }

    /// Recover at most one oldest durable pending candidate evaluation from
    /// its exact immutable snapshot. This idle-only task accepts no path,
    /// timestamp, scan identity, or evaluator input.
    pub fn start_candidate_evaluation_recovery_maintenance(
        &self,
    ) -> Result<CandidateEvaluationRecoveryMaintenanceStartOutcome, StartTaskError> {
        self.start_candidate_evaluation_recovery_maintenance_with_hooks(
            SystemTime::now,
            || {},
            || {},
        )
    }

    #[cfg(test)]
    fn start_candidate_evaluation_recovery_maintenance_at(
        &self,
        observed_at: SystemTime,
    ) -> Result<CandidateEvaluationRecoveryMaintenanceStartOutcome, StartTaskError> {
        self.start_candidate_evaluation_recovery_maintenance_with_hooks(
            move || observed_at,
            || {},
            || {},
        )
    }

    #[cfg(test)]
    fn start_candidate_evaluation_recovery_maintenance_with_test_hooks(
        &self,
        observed_at: SystemTime,
        before_recovery: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
    ) -> Result<CandidateEvaluationRecoveryMaintenanceStartOutcome, StartTaskError> {
        self.start_candidate_evaluation_recovery_maintenance_with_hooks(
            move || observed_at,
            before_recovery,
            after_applying,
        )
    }

    fn start_candidate_evaluation_recovery_maintenance_with_hooks(
        &self,
        clock: impl FnOnce() -> SystemTime + Send + 'static,
        before_recovery: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
    ) -> Result<CandidateEvaluationRecoveryMaintenanceStartOutcome, StartTaskError> {
        if let Some(outcome) = self.candidate_evaluation_recovery_maintenance_preflight()? {
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
        let snapshots = Arc::clone(&self.inner.snapshots);
        self.submit_candidate_evaluation_recovery_maintenance(Box::new(move |context| {
            if context.is_cancellation_requested() {
                return WorkOutcome::Cancelled(None);
            }
            let observed_at = clock();
            before_recovery();
            if !context.try_begin_candidate_evaluation_recovery_maintenance() {
                return WorkOutcome::Cancelled(None);
            }
            // Applying is the point of no return. A later cancellation remains
            // visible intent but cannot suppress the exact recovery outcome.
            after_applying();
            match Self::recover_pending_candidate_evaluation_after_admission(
                &store,
                &snapshots,
                observed_at,
            ) {
                Ok(outcome) => {
                    let (outcome, has_more) =
                        public_candidate_evaluation_recovery_maintenance_outcome(outcome);
                    let result = Arc::new(CandidateEvaluationRecoveryMaintenanceResult::new(
                        observed_at,
                        outcome,
                        has_more,
                    ));
                    context.report_candidate_evaluation_recovery_maintenance_finished(
                        TaskEventKind::CandidateEvaluationRecoveryMaintenanceFinished {
                            outcome: result.outcome(),
                            has_more: result.has_more(),
                        },
                    );
                    WorkOutcome::Succeeded(TaskResult::CandidateEvaluationRecoveryMaintenance(
                        result,
                    ))
                }
                Err(error) => WorkOutcome::Failed(
                    map_candidate_evaluation_recovery_maintenance_failure(error),
                    None,
                ),
            }
        }))
    }

    fn candidate_evaluation_recovery_maintenance_preflight(
        &self,
    ) -> Result<Option<CandidateEvaluationRecoveryMaintenanceStartOutcome>, StartTaskError> {
        let registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_candidate_evaluation_recovery_maintenance {
            return Ok(Some(
                CandidateEvaluationRecoveryMaintenanceStartOutcome::AlreadyActive(existing),
            ));
        }
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(Some(
                CandidateEvaluationRecoveryMaintenanceStartOutcome::DeferredBusy,
            ));
        }
        Ok(None)
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

    /// Reconcile at most one exact marker-owned snapshot provisioning stage
    /// beneath the retained database root. Exact root, name, and filesystem
    /// identity stay inside persistence; `has_more` is only a later idle
    /// rescheduling hint.
    pub fn start_snapshot_provisioning_stage_maintenance(
        &self,
    ) -> Result<SnapshotProvisioningStageMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_provisioning_stage_maintenance_with_hooks(
            SystemTime::now,
            || {},
            || {},
            || {},
        )
    }

    #[cfg(test)]
    fn start_snapshot_provisioning_stage_maintenance_at(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotProvisioningStageMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_provisioning_stage_maintenance_with_hooks(
            move || observed_at,
            || {},
            || {},
            || {},
        )
    }

    #[cfg(test)]
    fn start_snapshot_provisioning_stage_maintenance_with_test_hooks(
        &self,
        observed_at: SystemTime,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<SnapshotProvisioningStageMaintenanceStartOutcome, StartTaskError> {
        self.start_snapshot_provisioning_stage_maintenance_with_hooks(
            move || observed_at,
            before_batch,
            after_applying,
            after_batch,
        )
    }

    fn start_snapshot_provisioning_stage_maintenance_with_hooks(
        &self,
        clock: impl FnOnce() -> SystemTime + Send + 'static,
        before_batch: impl FnOnce() + Send + 'static,
        after_applying: impl FnOnce() + Send + 'static,
        after_batch: impl FnOnce() + Send + 'static,
    ) -> Result<SnapshotProvisioningStageMaintenanceStartOutcome, StartTaskError> {
        if let Some(outcome) = self.snapshot_provisioning_stage_maintenance_preflight()? {
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
        self.submit_snapshot_provisioning_stage_maintenance(Box::new(move |context| {
            if context.is_cancellation_requested() {
                return WorkOutcome::Cancelled(None);
            }
            let observed_at = clock();
            before_batch();
            if !context.try_begin_snapshot_provisioning_stage_maintenance_batch() {
                return WorkOutcome::Cancelled(None);
            }
            // Applying is the point of no return. Later cancellation remains
            // visible intent but cannot suppress an exact repository result.
            after_applying();
            match snapshots.reconcile_snapshot_provisioning_stage(observed_at) {
                Ok(result) => {
                    let outcome =
                        public_snapshot_provisioning_stage_maintenance_outcome(&result.outcome);
                    let result = Arc::new(SnapshotProvisioningStageMaintenanceResult::new(
                        result.observed_at,
                        outcome,
                        result.total_stage_count_before,
                        result.total_stage_count_after,
                        result.marker_owned_count_before,
                        result.marker_owned_count_after,
                        result.unproven_count_before,
                        result.unproven_count_after,
                        result.control_charged_bytes_before,
                        result.control_charged_bytes_after,
                        result.has_more,
                    ));
                    after_batch();
                    context.report_snapshot_provisioning_stage_maintenance_finished(
                        TaskEventKind::SnapshotProvisioningStageMaintenanceBatchFinished {
                            outcome: result.outcome(),
                            total_stage_count_before: result.total_stage_count_before(),
                            total_stage_count_after: result.total_stage_count_after(),
                            marker_owned_count_before: result.marker_owned_count_before(),
                            marker_owned_count_after: result.marker_owned_count_after(),
                            unproven_count_before: result.unproven_count_before(),
                            unproven_count_after: result.unproven_count_after(),
                            control_charged_bytes_before: result.control_charged_bytes_before(),
                            control_charged_bytes_after: result.control_charged_bytes_after(),
                            has_more: result.has_more(),
                        },
                    );
                    WorkOutcome::Succeeded(TaskResult::SnapshotProvisioningStageMaintenance(result))
                }
                Err(error) => WorkOutcome::Failed(
                    map_snapshot_provisioning_stage_maintenance_failure(error.kind),
                    None,
                ),
            }
        }))
    }

    fn snapshot_provisioning_stage_maintenance_preflight(
        &self,
    ) -> Result<Option<SnapshotProvisioningStageMaintenanceStartOutcome>, StartTaskError> {
        let registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_snapshot_provisioning_stage_maintenance {
            return Ok(Some(
                SnapshotProvisioningStageMaintenanceStartOutcome::AlreadyActive(existing),
            ));
        }
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(Some(
                SnapshotProvisioningStageMaintenanceStartOutcome::DeferredBusy,
            ));
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

    /// Start one standalone immutable scan rooted at a directory selected from
    /// this engine's exact Explorer review. The resolved path remains sealed
    /// inside Rust and is fenced by its current filesystem identity.
    pub fn start_subtree_scan(
        &self,
        review: &mut SnapshotReviewSession,
        node_id: u64,
    ) -> Result<TaskId, StartSubtreeScanError> {
        self.start_subtree_scan_with_hooks(review, node_id, |_| {}, || {}, || {})
    }

    #[cfg(test)]
    fn start_subtree_scan_with_before_traversal_hook(
        &self,
        review: &mut SnapshotReviewSession,
        node_id: u64,
        before_traversal: impl FnOnce(&ScanId) + Send + 'static,
    ) -> Result<TaskId, StartSubtreeScanError> {
        self.start_subtree_scan_with_hooks(review, node_id, before_traversal, || {}, || {})
    }

    #[cfg(test)]
    fn start_subtree_scan_with_before_candidate_persistence_hook(
        &self,
        review: &mut SnapshotReviewSession,
        node_id: u64,
        before_candidate_persistence: impl FnOnce() + Send + 'static,
    ) -> Result<TaskId, StartSubtreeScanError> {
        self.start_subtree_scan_with_hooks(
            review,
            node_id,
            |_| {},
            || {},
            before_candidate_persistence,
        )
    }

    fn start_subtree_scan_with_hooks(
        &self,
        review: &mut SnapshotReviewSession,
        node_id: u64,
        before_traversal: impl FnOnce(&ScanId) + Send + 'static,
        before_candidate_evaluation: impl FnOnce() + Send + 'static,
        before_candidate_persistence: impl FnOnce() + Send + 'static,
    ) -> Result<TaskId, StartSubtreeScanError> {
        if !review.belongs_to(&self.inner.snapshot_review_owner) {
            return Err(StartSubtreeScanError::ForeignReview);
        }
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(StartTaskError::Closed.into());
        }
        let target = review.subtree_scan_target(node_id)?;
        let status = self
            .inner
            .store
            .status()
            .map_err(|_| StartTaskError::PersistenceUnavailable)?;
        if !matches!(
            status.access,
            crate::persistence::DatabaseAccess::ReadWriteCurrent
        ) {
            return Err(StartTaskError::ReadOnlyStore.into());
        }
        let store = Arc::clone(&self.inner.store);
        let snapshots = Arc::clone(&self.inner.snapshots);
        let root = target.path;
        let expected_identity = target.identity;
        self.submit(
            TaskKind::Scan,
            Some(root.clone()),
            Box::new(move |context| {
                run_scan_task(
                    context,
                    AdmittedScanRoot {
                        path: root,
                        expected_identity: Some(expected_identity),
                    },
                    store,
                    snapshots,
                    before_traversal,
                    before_candidate_evaluation,
                    before_candidate_persistence,
                )
            }),
        )
        .map_err(StartSubtreeScanError::from)
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
                    AdmittedScanRoot {
                        path: canonical_root,
                        expected_identity: None,
                    },
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
                | TaskResult::ScanRecoveryMaintenance(_)
                | TaskResult::CandidateEvaluationRecoveryMaintenance(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotProvisioningStageMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_)
                | TaskResult::PermanentSafeCleanup(_),
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
                | TaskResult::ScanRecoveryMaintenance(_)
                | TaskResult::CandidateEvaluationRecoveryMaintenance(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotProvisioningStageMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_)
                | TaskResult::PermanentSafeCleanup(_),
            ) => {
                return Err(TaskAccessError::WrongTaskKind);
            }
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn scan_recovery_maintenance_result(
        &self,
        id: TaskId,
    ) -> Result<Option<Arc<ScanRecoveryMaintenanceResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::ScanRecoveryMaintenance {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::ScanRecoveryMaintenance(result)) => Some(Arc::clone(result)),
            Some(
                TaskResult::FormatSizeBatch(_)
                | TaskResult::Scan(_)
                | TaskResult::CandidateEvaluationRecoveryMaintenance(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotProvisioningStageMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_)
                | TaskResult::PermanentSafeCleanup(_),
            ) => return Err(TaskAccessError::WrongTaskKind),
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn candidate_evaluation_recovery_maintenance_result(
        &self,
        id: TaskId,
    ) -> Result<Option<Arc<CandidateEvaluationRecoveryMaintenanceResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::CandidateEvaluationRecoveryMaintenance {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::CandidateEvaluationRecoveryMaintenance(result)) => {
                Some(Arc::clone(result))
            }
            Some(
                TaskResult::FormatSizeBatch(_)
                | TaskResult::Scan(_)
                | TaskResult::ScanRecoveryMaintenance(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotProvisioningStageMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_)
                | TaskResult::PermanentSafeCleanup(_),
            ) => return Err(TaskAccessError::WrongTaskKind),
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
                | TaskResult::ScanRecoveryMaintenance(_)
                | TaskResult::CandidateEvaluationRecoveryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotProvisioningStageMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_)
                | TaskResult::PermanentSafeCleanup(_),
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
                | TaskResult::ScanRecoveryMaintenance(_)
                | TaskResult::CandidateEvaluationRecoveryMaintenance(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotProvisioningStageMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_)
                | TaskResult::PermanentSafeCleanup(_),
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
                | TaskResult::ScanRecoveryMaintenance(_)
                | TaskResult::CandidateEvaluationRecoveryMaintenance(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotProvisioningStageMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_)
                | TaskResult::PermanentSafeCleanup(_),
            ) => return Err(TaskAccessError::WrongTaskKind),
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn snapshot_provisioning_stage_maintenance_result(
        &self,
        id: TaskId,
    ) -> Result<Option<Arc<SnapshotProvisioningStageMaintenanceResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::SnapshotProvisioningStageMaintenance {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::SnapshotProvisioningStageMaintenance(result)) => {
                Some(Arc::clone(result))
            }
            Some(
                TaskResult::FormatSizeBatch(_)
                | TaskResult::Scan(_)
                | TaskResult::ScanRecoveryMaintenance(_)
                | TaskResult::CandidateEvaluationRecoveryMaintenance(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_)
                | TaskResult::PermanentSafeCleanup(_),
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
                | TaskResult::ScanRecoveryMaintenance(_)
                | TaskResult::CandidateEvaluationRecoveryMaintenance(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotProvisioningStageMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_)
                | TaskResult::PermanentSafeCleanup(_),
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
                | TaskResult::ScanRecoveryMaintenance(_)
                | TaskResult::CandidateEvaluationRecoveryMaintenance(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotProvisioningStageMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::PermanentSafeCleanup(_),
            ) => return Err(TaskAccessError::WrongTaskKind),
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn permanent_safe_cleanup_result(
        &self,
        id: TaskId,
    ) -> Result<Option<Arc<RustTargetCleanupResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::PermanentSafeCleanup {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::PermanentSafeCleanup(result)) => Some(Arc::clone(result)),
            Some(
                TaskResult::FormatSizeBatch(_)
                | TaskResult::Scan(_)
                | TaskResult::ScanRecoveryMaintenance(_)
                | TaskResult::CandidateEvaluationRecoveryMaintenance(_)
                | TaskResult::HistoryMaintenance(_)
                | TaskResult::SnapshotRetention(_)
                | TaskResult::SnapshotOrphanMaintenance(_)
                | TaskResult::SnapshotProvisioningStageMaintenance(_)
                | TaskResult::SnapshotTerminalTempMaintenance(_)
                | TaskResult::SnapshotUnleasedTempMaintenance(_),
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
            && let Some((active, existing)) = registry
                .active_scan_roots
                .iter()
                .find(|(active, _)| super::config::paths_overlap(active, scope))
        {
            return if active.as_path() == scope.as_path() {
                Err(StartTaskError::ScanAlreadyActive {
                    existing: *existing,
                })
            } else {
                Err(StartTaskError::ScanScopeBusy)
            };
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

    fn submit_scan_recovery_maintenance(
        &self,
        work: Work,
    ) -> Result<ScanRecoveryMaintenanceStartOutcome, StartTaskError> {
        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_scan_recovery_maintenance {
            return Ok(ScanRecoveryMaintenanceStartOutcome::AlreadyActive(existing));
        }
        // Recovery shares the lowest-priority idle boundary with every other
        // maintenance class. Recheck after the compatibility probe to close
        // admission races with foreground and maintenance work.
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(ScanRecoveryMaintenanceStartOutcome::DeferredBusy);
        }
        let id = TASK_IDS.allocate()?;
        let record = TaskRecord::new(
            id,
            TaskKind::ScanRecoveryMaintenance,
            None,
            self.inner.shared.limits.events_per_task,
        );
        registry.active_scan_recovery_maintenance = Some(id);
        registry.records.insert(id, record);
        registry.queue.push_back(Job { id, work });
        self.inner.shared.workers_ready.notify_one();
        Ok(ScanRecoveryMaintenanceStartOutcome::Started(id))
    }

    fn submit_candidate_evaluation_recovery_maintenance(
        &self,
        work: Work,
    ) -> Result<CandidateEvaluationRecoveryMaintenanceStartOutcome, StartTaskError> {
        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_candidate_evaluation_recovery_maintenance {
            return Ok(CandidateEvaluationRecoveryMaintenanceStartOutcome::AlreadyActive(existing));
        }
        // Candidate recovery shares the lowest-priority idle boundary with
        // every foreground and maintenance task. Recheck after the store probe
        // to close admission races.
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(CandidateEvaluationRecoveryMaintenanceStartOutcome::DeferredBusy);
        }
        let id = TASK_IDS.allocate()?;
        let record = TaskRecord::new(
            id,
            TaskKind::CandidateEvaluationRecoveryMaintenance,
            None,
            self.inner.shared.limits.events_per_task,
        );
        registry.active_candidate_evaluation_recovery_maintenance = Some(id);
        registry.records.insert(id, record);
        registry.queue.push_back(Job { id, work });
        self.inner.shared.workers_ready.notify_one();
        Ok(CandidateEvaluationRecoveryMaintenanceStartOutcome::Started(
            id,
        ))
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

    fn submit_snapshot_provisioning_stage_maintenance(
        &self,
        work: Work,
    ) -> Result<SnapshotProvisioningStageMaintenanceStartOutcome, StartTaskError> {
        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
        }
        if let Some(existing) = registry.active_snapshot_provisioning_stage_maintenance {
            return Ok(SnapshotProvisioningStageMaintenanceStartOutcome::AlreadyActive(existing));
        }
        // Provisioning-stage reconciliation shares the same idle-only
        // boundary as every other maintenance class. Rechecking here closes
        // the admission race between the compatibility probe and submission.
        if registry.running_tasks != 0 || !registry.queue.is_empty() {
            return Ok(SnapshotProvisioningStageMaintenanceStartOutcome::DeferredBusy);
        }
        let id = TASK_IDS.allocate()?;
        let record = TaskRecord::new(
            id,
            TaskKind::SnapshotProvisioningStageMaintenance,
            None,
            self.inner.shared.limits.events_per_task,
        );
        registry.active_snapshot_provisioning_stage_maintenance = Some(id);
        registry.records.insert(id, record);
        registry.queue.push_back(Job { id, work });
        self.inner.shared.workers_ready.notify_one();
        Ok(SnapshotProvisioningStageMaintenanceStartOutcome::Started(
            id,
        ))
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

#[cfg(target_os = "macos")]
fn generate_rust_target_plan_review_id() -> Result<CleanupPlanId, RustTargetPlanReviewError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| RustTargetPlanReviewError::InternalState)?;
    let mut value = String::with_capacity("plan:rust-target-review:".len() + random.len() * 2);
    value.push_str("plan:rust-target-review:");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in random {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    CleanupPlanId::new(value).map_err(|_| RustTargetPlanReviewError::InternalState)
}

#[cfg(target_os = "macos")]
fn map_rust_target_plan_review_pipeline_error(
    error: RustTargetPipelineError,
) -> RustTargetPlanReviewError {
    match error.plan_review_failure() {
        RustTargetPlanReviewFailure::Closed => RustTargetPlanReviewError::Closed,
        RustTargetPlanReviewFailure::CandidateUnavailable => {
            RustTargetPlanReviewError::CandidateUnavailable
        }
        RustTargetPlanReviewFailure::CargoNotEnrolled => {
            RustTargetPlanReviewError::CargoNotEnrolled
        }
        RustTargetPlanReviewFailure::ActiveProcesses => RustTargetPlanReviewError::ActiveProcesses,
        RustTargetPlanReviewFailure::ChangedDuringReview => {
            RustTargetPlanReviewError::ChangedDuringReview
        }
        RustTargetPlanReviewFailure::UnsupportedPlatform => {
            RustTargetPlanReviewError::UnsupportedPlatform
        }
        RustTargetPlanReviewFailure::BudgetExceeded => RustTargetPlanReviewError::BudgetExceeded,
        RustTargetPlanReviewFailure::Busy => RustTargetPlanReviewError::Busy,
        RustTargetPlanReviewFailure::UnsafeStorage => RustTargetPlanReviewError::UnsafeStorage,
        RustTargetPlanReviewFailure::CorruptData => RustTargetPlanReviewError::CorruptData,
        RustTargetPlanReviewFailure::Unavailable => RustTargetPlanReviewError::Unavailable,
        RustTargetPlanReviewFailure::InternalState => RustTargetPlanReviewError::InternalState,
    }
}

#[cfg(target_os = "macos")]
fn run_permanent_safe_cleanup_task(
    store: Arc<StoreCoordinator>,
    admitted: Result<ApprovedTrustedReviewedCleanupPlan, RustTargetCleanupError>,
    before_begin: Box<dyn FnOnce() + Send>,
    context: &TaskContext,
) -> WorkOutcome {
    if context.is_cancellation_requested() || !context.engine_is_open() {
        if let Ok(approved) = admitted {
            approved.release();
        }
        return WorkOutcome::Cancelled(None);
    }
    let approved = match admitted {
        Ok(approved) => approved,
        Err(error) => return permanent_safe_cleanup_failed(error),
    };
    let quarantine = process_cleanup_quarantine()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if store_is_quarantined(&quarantine, &store) {
        approved.release();
        return permanent_safe_cleanup_failed(RustTargetCleanupError::OutcomeUnknown);
    }
    drop(quarantine);
    let session_id = match generate_session_id() {
        Ok(session_id) => session_id,
        Err(error) => return permanent_safe_cleanup_failed(error),
    };
    before_begin();
    let started_at = SystemTime::now();
    let begin = approved.begin_cleanup_session(
        &store,
        session_id.clone(),
        started_at,
        CleanupTrigger::Manual,
        Duration::from_secs(5),
    );
    let mut session = match begin {
        Ok(session) => session,
        Err(CleanupSessionStartError::JournalClaimUnresolved(failure)) => match failure.retry() {
            Ok(session) => session,
            Err(CleanupSessionStartError::JournalClaimUnresolved(failure)) => {
                quarantine_cleanup_for_store(
                    &store,
                    &context.shared,
                    QuarantinedCleanup::Lease(failure.into_lease()),
                );
                return permanent_safe_cleanup_failed_with_session(
                    RustTargetCleanupError::OutcomeUnknown,
                    &session_id,
                );
            }
            Err(CleanupSessionStartError::ClaimedPlanUnresolved(failure)) => {
                quarantine_cleanup_for_store(
                    &store,
                    &context.shared,
                    QuarantinedCleanup::Session {
                        session: Box::new(failure.into_session()),
                        unsettled_effect: None,
                    },
                );
                return permanent_safe_cleanup_failed_with_session(
                    RustTargetCleanupError::OutcomeUnknown,
                    &session_id,
                );
            }
            Err(CleanupSessionStartError::Handoff(error)) => {
                return permanent_safe_cleanup_handoff_failed(error, &session_id);
            }
        },
        Err(CleanupSessionStartError::ClaimedPlanUnresolved(failure)) => {
            quarantine_cleanup_for_store(
                &store,
                &context.shared,
                QuarantinedCleanup::Session {
                    session: Box::new(failure.into_session()),
                    unsettled_effect: None,
                },
            );
            return permanent_safe_cleanup_failed_with_session(
                RustTargetCleanupError::OutcomeUnknown,
                &session_id,
            );
        }
        Err(CleanupSessionStartError::Handoff(error)) => {
            return permanent_safe_cleanup_handoff_failed(error, &session_id);
        }
    };

    let summary = match execute_permanent_safe_task_session(
        &mut session,
        SystemTime::now(),
        &|| context.is_cancellation_requested(),
        context,
    ) {
        Ok(summary) => summary,
        Err(PermanentSafeExecutionError::UnsettledEffect(unsettled_effect)) => {
            quarantine_cleanup_for_store(
                &store,
                &context.shared,
                QuarantinedCleanup::Session {
                    session: Box::new(session),
                    unsettled_effect: Some(unsettled_effect),
                },
            );
            return permanent_safe_cleanup_failed_with_session(
                RustTargetCleanupError::OutcomeUnknown,
                &session_id,
            );
        }
        Err(_) => {
            quarantine_cleanup_for_store(
                &store,
                &context.shared,
                QuarantinedCleanup::Session {
                    session: Box::new(session),
                    unsettled_effect: None,
                },
            );
            return permanent_safe_cleanup_failed_with_session(
                RustTargetCleanupError::OutcomeUnknown,
                &session_id,
            );
        }
    };
    let result = match rust_target_cleanup_result(&session_id, summary) {
        Ok(result) => Arc::new(result),
        Err(error) => return permanent_safe_cleanup_failed(error),
    };
    if result.status() == DurableCleanupSessionStatus::Cancelled {
        WorkOutcome::Cancelled(Some(TaskResult::PermanentSafeCleanup(result)))
    } else {
        WorkOutcome::Succeeded(TaskResult::PermanentSafeCleanup(result))
    }
}

#[cfg(target_os = "macos")]
fn execute_permanent_safe_task_session(
    session: &mut ApprovedCleanupSession,
    now: SystemTime,
    cancelled: &dyn Fn() -> bool,
    context: &TaskContext,
) -> Result<PermanentSafeSessionSummary, PermanentSafeExecutionError> {
    if !context.engine_is_open() {
        return Err(PermanentSafeExecutionError::Admission(
            HistoryErrorKind::InvalidTransition,
        ));
    }
    let mut sampler = session
        .capacity_scope(SystemTime::now())
        .ok()
        .and_then(MacOSCleanupCapacitySampler::new);
    let mut driver = DescriptorRelativePermanentSafeDriver;
    execute_rust_target_session_with_capacity(
        session,
        now,
        &mut driver,
        cancelled,
        sampler
            .as_mut()
            .map(|sampler| sampler as &mut dyn CleanupCapacitySampler),
    )
}

#[cfg(target_os = "macos")]
const fn map_rust_target_cleanup_review_error(
    error: RustTargetPlanReviewError,
) -> RustTargetCleanupError {
    match error {
        RustTargetPlanReviewError::Closed => RustTargetCleanupError::Closed,
        RustTargetPlanReviewError::WrongEngine => RustTargetCleanupError::WrongEngine,
        RustTargetPlanReviewError::ParentReviewUnavailable => {
            RustTargetCleanupError::ParentReviewUnavailable
        }
        RustTargetPlanReviewError::ReviewExpired => RustTargetCleanupError::ReviewExpired,
        RustTargetPlanReviewError::ChangedDuringReview
        | RustTargetPlanReviewError::CandidateUnavailable
        | RustTargetPlanReviewError::CargoNotEnrolled
        | RustTargetPlanReviewError::ActiveProcesses => RustTargetCleanupError::ChangedDuringReview,
        RustTargetPlanReviewError::UnsupportedPlatform | RustTargetPlanReviewError::Unavailable => {
            RustTargetCleanupError::Unavailable
        }
        RustTargetPlanReviewError::BudgetExceeded => RustTargetCleanupError::BudgetExceeded,
        RustTargetPlanReviewError::Busy => RustTargetCleanupError::Busy,
        RustTargetPlanReviewError::UnsafeStorage => RustTargetCleanupError::UnsafeStorage,
        RustTargetPlanReviewError::CorruptData => RustTargetCleanupError::CorruptData,
        RustTargetPlanReviewError::InternalState => RustTargetCleanupError::InternalState,
    }
}

#[cfg(target_os = "macos")]
fn permanent_safe_cleanup_failed(error: RustTargetCleanupError) -> WorkOutcome {
    WorkOutcome::Failed(
        TaskFailureKind::PermanentSafeCleanup(rust_target_cleanup_failure_kind(error)),
        None,
    )
}

#[cfg(target_os = "macos")]
fn permanent_safe_cleanup_failed_with_session(
    error: RustTargetCleanupError,
    session_id: &CleanupSessionId,
) -> WorkOutcome {
    let result = rust_target_cleanup_recovering_result(session_id)
        .ok()
        .map(Arc::new)
        .map(TaskResult::PermanentSafeCleanup);
    WorkOutcome::Failed(
        TaskFailureKind::PermanentSafeCleanup(rust_target_cleanup_failure_kind(error)),
        result,
    )
}

#[cfg(target_os = "macos")]
fn permanent_safe_cleanup_handoff_failed(
    error: ExactPathHandoffError,
    session_id: &CleanupSessionId,
) -> WorkOutcome {
    let error = map_rust_target_cleanup_handoff_error(error);
    if error == RustTargetCleanupError::OutcomeUnknown {
        permanent_safe_cleanup_failed_with_session(error, session_id)
    } else {
        permanent_safe_cleanup_failed(error)
    }
}

const fn public_trash_platform_result(error: TrashPlatformError) -> TrashPlatformResult {
    match error {
        TrashPlatformError::Unsupported => TrashPlatformResult::Unsupported,
        TrashPlatformError::Failed => TrashPlatformResult::Failed,
        TrashPlatformError::OutcomeUnknown => TrashPlatformResult::OutcomeUnknown,
    }
}

fn map_trash_admission_error(error: TrashAdmissionError) -> TrashSelectionError {
    match error {
        TrashAdmissionError::TargetUnavailable
        | TrashAdmissionError::UnsupportedTargetKind
        | TrashAdmissionError::UnsupportedEffectMode
        | TrashAdmissionError::TargetNotBound => TrashSelectionError::Review,
        TrashAdmissionError::TargetChanged => TrashSelectionError::ChangedSincePlan,
        TrashAdmissionError::Journal(kind) => match kind {
            HistoryErrorKind::Busy => TrashSelectionError::Busy,
            HistoryErrorKind::UnsafeStorage => TrashSelectionError::UnsafeStorage,
            HistoryErrorKind::IncompatibleSchema => TrashSelectionError::IncompatibleSchema,
            HistoryErrorKind::CorruptData => TrashSelectionError::CorruptData,
            HistoryErrorKind::DatabaseUnavailable => TrashSelectionError::Unavailable,
            HistoryErrorKind::OutcomeUnknown => TrashSelectionError::OutcomeUnknown,
            HistoryErrorKind::InvalidInput => TrashSelectionError::InvalidRequest,
            HistoryErrorKind::InvalidTransition
            | HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::NotFound
            | HistoryErrorKind::QueryLimitExceeded
            | HistoryErrorKind::InternalState => TrashSelectionError::InternalState,
        },
    }
}

fn map_snapshot_review_plan_review_error(error: SnapshotReviewError) -> RustTargetPlanReviewError {
    match error {
        SnapshotReviewError::Closed => RustTargetPlanReviewError::Closed,
        SnapshotReviewError::ScanNotFound
        | SnapshotReviewError::SnapshotUnavailable
        | SnapshotReviewError::LeaseExpired => RustTargetPlanReviewError::ParentReviewUnavailable,
        SnapshotReviewError::Busy => RustTargetPlanReviewError::Busy,
        SnapshotReviewError::UnsafeStorage => RustTargetPlanReviewError::UnsafeStorage,
        SnapshotReviewError::BudgetExceeded => RustTargetPlanReviewError::BudgetExceeded,
        SnapshotReviewError::CorruptData
        | SnapshotReviewError::IncompatibleSchema
        | SnapshotReviewError::IncompatibleSnapshot => RustTargetPlanReviewError::CorruptData,
        SnapshotReviewError::ReadOnlyStore
        | SnapshotReviewError::Unavailable
        | SnapshotReviewError::OutcomeUnknown => RustTargetPlanReviewError::Unavailable,
        SnapshotReviewError::InternalState => RustTargetPlanReviewError::InternalState,
        _ => RustTargetPlanReviewError::InternalState,
    }
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
        candidate_evaluation_context_digest_sha256(scan_id, artifact, scheduled_at),
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

    let batch = match evaluate_completed_scan_candidates(scan_id, artifact, scheduled_at) {
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

const fn candidate_replay_failure_kind(
    error: CandidateSnapshotReplayError,
) -> CandidateEvaluationFailureKind {
    match error {
        CandidateSnapshotReplayError::Evaluation(error) => map_candidate_evaluation_error(error),
        CandidateSnapshotReplayError::ResourceLimit => {
            CandidateEvaluationFailureKind::LimitExceeded
        }
        CandidateSnapshotReplayError::InvalidSnapshot
        | CandidateSnapshotReplayError::BatchMismatch => {
            CandidateEvaluationFailureKind::CandidateInvalid
        }
    }
}

fn map_candidate_recovery_snapshot_error(error: SnapshotRepositoryErrorKind) -> HistoryErrorKind {
    match error {
        SnapshotRepositoryErrorKind::History(kind) => kind,
        SnapshotRepositoryErrorKind::Storage(SnapshotStorageErrorKind::Busy) => {
            HistoryErrorKind::Busy
        }
        SnapshotRepositoryErrorKind::Storage(SnapshotStorageErrorKind::Unavailable)
        | SnapshotRepositoryErrorKind::MissingStore
        | SnapshotRepositoryErrorKind::MissingSnapshot => HistoryErrorKind::DatabaseUnavailable,
        SnapshotRepositoryErrorKind::SnapshotUnavailable => HistoryErrorKind::NotFound,
        SnapshotRepositoryErrorKind::ReadOnly => HistoryErrorKind::InvalidTransition,
        SnapshotRepositoryErrorKind::IncompatibleVersion
        | SnapshotRepositoryErrorKind::ReferenceMismatch
        | SnapshotRepositoryErrorKind::Codec(_)
        | SnapshotRepositoryErrorKind::Storage(
            SnapshotStorageErrorKind::InvalidConfiguration
            | SnapshotStorageErrorKind::UnsafeRoot
            | SnapshotStorageErrorKind::UnsafeObject
            | SnapshotStorageErrorKind::UnrecognizedStore
            | SnapshotStorageErrorKind::InternalState,
        )
        | SnapshotRepositoryErrorKind::ReviewLeaseExpired => HistoryErrorKind::CorruptData,
    }
}

fn canonical_recovery_evaluation_time(
    observed_at: SystemTime,
    scheduled_at: SystemTime,
) -> Option<SystemTime> {
    let observed = observed_at.max(scheduled_at);
    let duration = observed.duration_since(UNIX_EPOCH).ok()?;
    let millis = u64::try_from(duration.as_millis()).ok()?;
    i64::try_from(millis).ok()?;
    UNIX_EPOCH.checked_add(Duration::from_millis(millis))
}

const fn map_candidate_recovery_error(kind: HistoryErrorKind) -> CandidateEvaluationRecoveryError {
    match kind {
        HistoryErrorKind::InvalidInput => CandidateEvaluationRecoveryError::InvalidClock,
        HistoryErrorKind::InvalidTransition => CandidateEvaluationRecoveryError::InternalState,
        HistoryErrorKind::NotFound => CandidateEvaluationRecoveryError::Unavailable,
        HistoryErrorKind::IncompatibleSchema => {
            CandidateEvaluationRecoveryError::IncompatibleSchema
        }
        HistoryErrorKind::QueryLimitExceeded => CandidateEvaluationRecoveryError::BudgetExceeded,
        HistoryErrorKind::Busy => CandidateEvaluationRecoveryError::Busy,
        HistoryErrorKind::UnsafeStorage => CandidateEvaluationRecoveryError::UnsafeStorage,
        HistoryErrorKind::CorruptData => CandidateEvaluationRecoveryError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => CandidateEvaluationRecoveryError::Unavailable,
        HistoryErrorKind::InternalState => CandidateEvaluationRecoveryError::InternalState,
        HistoryErrorKind::OutcomeUnknown => CandidateEvaluationRecoveryError::OutcomeUnknown,
        HistoryErrorKind::AlreadyExists => CandidateEvaluationRecoveryError::InternalState,
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

const fn map_scan_coverage_details_error(kind: HistoryErrorKind) -> ScanCoverageDetailsError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => ScanCoverageDetailsError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => ScanCoverageDetailsError::QueryLimitExceeded,
        HistoryErrorKind::Busy => ScanCoverageDetailsError::Busy,
        HistoryErrorKind::UnsafeStorage => ScanCoverageDetailsError::UnsafeStorage,
        HistoryErrorKind::CorruptData => ScanCoverageDetailsError::CorruptData,
        HistoryErrorKind::InternalState => ScanCoverageDetailsError::InternalState,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::DatabaseUnavailable
        | HistoryErrorKind::OutcomeUnknown => ScanCoverageDetailsError::Unavailable,
    }
}

fn historical_issue_location(
    root: &Path,
    observed: &Path,
) -> Result<DurableScanIssueLocation, ScanCoverageDetailsError> {
    let relative = observed
        .strip_prefix(root)
        .map_err(|_| ScanCoverageDetailsError::CorruptData)?;
    let raw_components = relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(value) => Ok(value.to_string_lossy()),
            _ => Err(ScanCoverageDetailsError::CorruptData),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if raw_components.is_empty() {
        return Ok(DurableScanIssueLocation::new(true, false, Vec::new()));
    }
    let retained_start = raw_components
        .len()
        .saturating_sub(MAX_SCAN_COVERAGE_LOCATION_COMPONENTS);
    let mut context_truncated = retained_start > 0;
    let components = raw_components[retained_start..]
        .iter()
        .map(|component| {
            let mut chars = component.chars();
            let display = chars
                .by_ref()
                .take(MAX_SCAN_COVERAGE_LOCATION_COMPONENT_CHARS)
                .collect::<String>();
            if chars.next().is_some() {
                context_truncated = true;
            }
            Arc::<str>::from(display)
        })
        .collect();
    Ok(DurableScanIssueLocation::new(
        false,
        context_truncated,
        components,
    ))
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

const fn map_cleanup_history_clear_store_error(
    error: CleanupHistoryClearStoreError,
) -> CleanupHistoryClearError {
    match error {
        CleanupHistoryClearStoreError::NothingToClear => CleanupHistoryClearError::NothingToClear,
        CleanupHistoryClearStoreError::ActiveCleanup => CleanupHistoryClearError::ActiveCleanup,
        CleanupHistoryClearStoreError::ChangedSincePreview => {
            CleanupHistoryClearError::ChangedSincePreview
        }
        CleanupHistoryClearStoreError::History(history) => match history.kind {
            HistoryErrorKind::IncompatibleSchema => CleanupHistoryClearError::IncompatibleSchema,
            HistoryErrorKind::QueryLimitExceeded => CleanupHistoryClearError::QueryLimitExceeded,
            HistoryErrorKind::Busy => CleanupHistoryClearError::Busy,
            HistoryErrorKind::UnsafeStorage => CleanupHistoryClearError::UnsafeStorage,
            HistoryErrorKind::CorruptData => CleanupHistoryClearError::CorruptData,
            HistoryErrorKind::OutcomeUnknown => CleanupHistoryClearError::OutcomeUnknown,
            HistoryErrorKind::DatabaseUnavailable => CleanupHistoryClearError::Unavailable,
            HistoryErrorKind::InternalState
            | HistoryErrorKind::InvalidInput
            | HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::NotFound
            | HistoryErrorKind::InvalidTransition => CleanupHistoryClearError::InternalState,
        },
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

fn public_direct_cargo_signature(signature: &CargoCodeSignatureRecord) -> DirectCargoCodeSignature {
    DirectCargoCodeSignature {
        class: match signature.class {
            CargoSignatureClass::AdHoc => DirectCargoSignatureClass::AdHoc,
            CargoSignatureClass::Cms => DirectCargoSignatureClass::Cms,
        },
        flags: signature.flags,
        code_directory_hashes: signature.code_directory_hashes.clone(),
        signing_identifier: signature.signing_identifier.clone(),
        team_identifier: signature.team_identifier.clone(),
        designated_requirement_sha256: signature.designated_requirement_sha256,
    }
}

fn public_direct_cargo_status(setting: CargoEnrollmentSetting) -> DirectCargoEnrollmentStatus {
    let state = match setting.state {
        CargoEnrollmentState::NotEnrolled => DirectCargoEnrollmentState::NotEnrolled,
        CargoEnrollmentState::Revoked => DirectCargoEnrollmentState::Revoked,
        CargoEnrollmentState::Enrolled(identity) => DirectCargoEnrollmentState::Enrolled {
            path: identity.path().to_path_buf(),
            executable_sha256: identity.executable_sha256,
            version_sha256: identity.version_sha256,
            cargo_release: identity.cargo_release,
            code_signature: Box::new(public_direct_cargo_signature(&identity.signature)),
        },
    };
    DirectCargoEnrollmentStatus {
        revision: setting.revision,
        state,
        updated_at: setting.updated_at,
    }
}

fn public_direct_cargo_update(update: CargoEnrollmentSettingUpdate) -> DirectCargoEnrollmentUpdate {
    DirectCargoEnrollmentUpdate {
        status: public_direct_cargo_status(update.setting),
        changed: update.changed,
    }
}

const fn map_direct_cargo_history_error(kind: HistoryErrorKind) -> DirectCargoEnrollmentError {
    match kind {
        HistoryErrorKind::InvalidInput => DirectCargoEnrollmentError::InvalidClock,
        HistoryErrorKind::InvalidTransition => DirectCargoEnrollmentError::RevisionExhausted,
        HistoryErrorKind::IncompatibleSchema => DirectCargoEnrollmentError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => DirectCargoEnrollmentError::QueryLimitExceeded,
        HistoryErrorKind::Busy => DirectCargoEnrollmentError::Busy,
        HistoryErrorKind::UnsafeStorage => DirectCargoEnrollmentError::UnsafeStorage,
        HistoryErrorKind::CorruptData => DirectCargoEnrollmentError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => DirectCargoEnrollmentError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => DirectCargoEnrollmentError::OutcomeUnknown,
        HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InternalState => DirectCargoEnrollmentError::InternalState,
    }
}

#[cfg(target_os = "macos")]
fn map_direct_cargo_validation_error(
    error: crate::planner::CargoMetadataValidationError,
) -> DirectCargoEnrollmentError {
    use crate::planner::CargoMetadataValidationError as Error;
    match error {
        Error::InvalidExecutableLocator | Error::Lexical(_) => {
            DirectCargoEnrollmentError::InvalidExecutableLocator
        }
        Error::ExecutableNotRegular => DirectCargoEnrollmentError::ExecutableNotRegular,
        Error::ExecutableChanged
        | Error::CargoVersionChanged
        | Error::CargoEnrollmentChanged
        | Error::CargoWorkingDirectoryChanged
        | Error::CargoWorkspaceGlobChanged
        | Error::WorkspaceManifestChanged
        | Error::CargoPackageMetadataChanged
        | Error::CargoTargetNamespaceChanged
        | Error::CargoManifestProbesChanged
        | Error::CargoConfigurationChanged
        | Error::LiveEvidenceChanged
        | Error::Filesystem(_)
        | Error::FileDigest(_) => DirectCargoEnrollmentError::ChangedDuringInspection,
        Error::InvalidResolutionEnvironment => {
            DirectCargoEnrollmentError::InvalidResolutionEnvironment
        }
        Error::InvalidCargoVersion => DirectCargoEnrollmentError::InvalidCargoVersion,
        Error::InvalidCodeSignature => DirectCargoEnrollmentError::InvalidCodeSignature,
        Error::Spawn { .. }
        | Error::PipeConfiguration
        | Error::OutputRead { .. }
        | Error::CargoWorkspaceGlobUnsupported
        | Error::CargoWorkspaceGlobUnavailable
        | Error::CargoManifestProbesUnsupported
        | Error::CargoManifestProbesUnavailable
        | Error::CargoPackageMetadataUnsupported
        | Error::CargoPackageMetadataUnavailable
        | Error::CargoTargetNamespaceUnsupported
        | Error::CargoTargetNamespaceUnavailable
        | Error::CargoConfigurationUnsupported
        | Error::CargoConfigurationUnavailable
        | Error::ProcessFailed => DirectCargoEnrollmentError::InspectionUnavailable,
        Error::Timeout | Error::OutputLimit { .. } => {
            DirectCargoEnrollmentError::InspectionLimitExceeded
        }
        Error::CargoNotEnrolled
        | Error::InvalidMetadata
        | Error::InvalidWorkspaceMembers
        | Error::InvalidPathDependencies
        | Error::CargoPathDependenciesUnsupported
        | Error::CargoDependencyManifestUnsupported
        | Error::WorkspaceManifestUnavailable
        | Error::WorkspaceMismatch
        | Error::TargetDirectoryMismatch => DirectCargoEnrollmentError::InternalState,
        Error::CargoEnrollmentStore { kind } => map_direct_cargo_history_error(kind),
    }
}

#[cfg(target_os = "macos")]
fn map_direct_cargo_commit_error(
    error: crate::planner::DirectCargoEnrollmentCommitError,
) -> DirectCargoEnrollmentError {
    match error {
        crate::planner::DirectCargoEnrollmentCommitError::Validation(error) => {
            map_direct_cargo_validation_error(error)
        }
        crate::planner::DirectCargoEnrollmentCommitError::History(kind) => {
            map_direct_cargo_history_error(kind)
        }
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

fn public_disk_pressure_policy(setting: DiskPressurePolicySetting) -> DiskPressurePolicy {
    DiskPressurePolicy {
        config: setting.config,
        source: match setting.source {
            DiskPressurePolicySettingSource::Default => DiskPressurePolicySource::Default,
            DiskPressurePolicySettingSource::Stored => DiskPressurePolicySource::Stored,
        },
        revision: setting.revision,
        updated_at: setting.updated_at,
    }
}

fn public_disk_pressure_policy_update(
    update: DiskPressurePolicySettingUpdate,
) -> DiskPressurePolicyUpdate {
    DiskPressurePolicyUpdate {
        settings: public_disk_pressure_policy(update.settings),
        changed: update.changed,
    }
}

fn public_permanent_cleanup_policy(setting: PermanentCleanupSetting) -> PermanentCleanupPolicy {
    PermanentCleanupPolicy {
        enabled: setting.enabled,
        source: match setting.source {
            PermanentCleanupSettingSource::Default => PermanentCleanupPolicySource::Default,
            PermanentCleanupSettingSource::Stored => PermanentCleanupPolicySource::Stored,
        },
        revision: setting.revision,
        updated_at: setting.updated_at,
    }
}

fn public_permanent_cleanup_policy_update(
    update: PermanentCleanupSettingUpdate,
) -> PermanentCleanupPolicyUpdate {
    PermanentCleanupPolicyUpdate {
        policy: public_permanent_cleanup_policy(update.settings),
        changed: update.changed,
    }
}

fn public_cleanup_exclusions(setting: CleanupExclusionSetting) -> CleanupExclusions {
    CleanupExclusions {
        paths: setting.paths,
        source: match setting.source {
            CleanupExclusionSettingSource::Default => CleanupExclusionSource::Default,
            CleanupExclusionSettingSource::Stored => CleanupExclusionSource::Stored,
        },
        revision: setting.revision,
        updated_at: setting.updated_at,
    }
}

fn public_cleanup_exclusions_update(
    update: CleanupExclusionSettingUpdate,
) -> CleanupExclusionsUpdate {
    CleanupExclusionsUpdate {
        exclusions: public_cleanup_exclusions(update.settings),
        changed: update.changed,
    }
}

const fn map_cleanup_exclusions_error(kind: HistoryErrorKind) -> CleanupExclusionsError {
    match kind {
        HistoryErrorKind::InvalidInput => CleanupExclusionsError::InvalidInput,
        HistoryErrorKind::InvalidTransition => CleanupExclusionsError::RevisionExhausted,
        HistoryErrorKind::IncompatibleSchema => CleanupExclusionsError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => CleanupExclusionsError::QueryLimitExceeded,
        HistoryErrorKind::Busy => CleanupExclusionsError::Busy,
        HistoryErrorKind::UnsafeStorage => CleanupExclusionsError::UnsafeStorage,
        HistoryErrorKind::CorruptData => CleanupExclusionsError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => CleanupExclusionsError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => CleanupExclusionsError::OutcomeUnknown,
        HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InternalState => CleanupExclusionsError::InternalState,
    }
}

const fn map_permanent_cleanup_policy_error(kind: HistoryErrorKind) -> PermanentCleanupPolicyError {
    match kind {
        HistoryErrorKind::InvalidInput => PermanentCleanupPolicyError::InvalidClock,
        HistoryErrorKind::InvalidTransition => PermanentCleanupPolicyError::RevisionExhausted,
        HistoryErrorKind::IncompatibleSchema => PermanentCleanupPolicyError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => PermanentCleanupPolicyError::QueryLimitExceeded,
        HistoryErrorKind::Busy => PermanentCleanupPolicyError::Busy,
        HistoryErrorKind::UnsafeStorage => PermanentCleanupPolicyError::UnsafeStorage,
        HistoryErrorKind::CorruptData => PermanentCleanupPolicyError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => PermanentCleanupPolicyError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => PermanentCleanupPolicyError::OutcomeUnknown,
        HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InternalState => PermanentCleanupPolicyError::InternalState,
    }
}

const fn map_disk_pressure_policy_error(kind: HistoryErrorKind) -> DiskPressurePolicyError {
    match kind {
        HistoryErrorKind::InvalidInput => DiskPressurePolicyError::InvalidClock,
        HistoryErrorKind::InvalidTransition => DiskPressurePolicyError::RevisionExhausted,
        HistoryErrorKind::IncompatibleSchema => DiskPressurePolicyError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => DiskPressurePolicyError::QueryLimitExceeded,
        HistoryErrorKind::Busy => DiskPressurePolicyError::Busy,
        HistoryErrorKind::UnsafeStorage => DiskPressurePolicyError::UnsafeStorage,
        HistoryErrorKind::CorruptData => DiskPressurePolicyError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => DiskPressurePolicyError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => DiskPressurePolicyError::OutcomeUnknown,
        HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InternalState => DiskPressurePolicyError::InternalState,
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

const fn map_scan_recovery_maintenance_failure(kind: HistoryErrorKind) -> TaskFailureKind {
    let kind = match kind {
        HistoryErrorKind::InvalidInput => ScanRecoveryMaintenanceFailureKind::InvalidClock,
        HistoryErrorKind::IncompatibleSchema => {
            ScanRecoveryMaintenanceFailureKind::IncompatibleSchema
        }
        HistoryErrorKind::QueryLimitExceeded => ScanRecoveryMaintenanceFailureKind::BudgetExceeded,
        HistoryErrorKind::Busy => ScanRecoveryMaintenanceFailureKind::Busy,
        HistoryErrorKind::UnsafeStorage => ScanRecoveryMaintenanceFailureKind::UnsafeStorage,
        HistoryErrorKind::CorruptData => ScanRecoveryMaintenanceFailureKind::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => ScanRecoveryMaintenanceFailureKind::Unavailable,
        HistoryErrorKind::OutcomeUnknown => ScanRecoveryMaintenanceFailureKind::OutcomeUnknown,
        HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::InternalState => ScanRecoveryMaintenanceFailureKind::InternalState,
    };
    TaskFailureKind::ScanRecoveryMaintenance(kind)
}

const fn public_scan_recovery_maintenance_outcome(
    outcome: &crate::persistence::ScanRecoveryBatchOutcome,
) -> ScanRecoveryMaintenanceOutcome {
    match outcome {
        crate::persistence::ScanRecoveryBatchOutcome::NoClaim => {
            ScanRecoveryMaintenanceOutcome::NoClaim
        }
        crate::persistence::ScanRecoveryBatchOutcome::DeferredUnproven => {
            ScanRecoveryMaintenanceOutcome::DeferredUnproven
        }
        crate::persistence::ScanRecoveryBatchOutcome::Interrupted => {
            ScanRecoveryMaintenanceOutcome::Interrupted
        }
        crate::persistence::ScanRecoveryBatchOutcome::ChangedConcurrently => {
            ScanRecoveryMaintenanceOutcome::ChangedConcurrently
        }
    }
}

const fn public_candidate_evaluation_recovery_maintenance_outcome(
    outcome: CandidateEvaluationRecoveryOutcome,
) -> (CandidateEvaluationRecoveryMaintenanceOutcome, bool) {
    match outcome {
        CandidateEvaluationRecoveryOutcome::NoPending => {
            (CandidateEvaluationRecoveryMaintenanceOutcome::None, false)
        }
        CandidateEvaluationRecoveryOutcome::Recovered {
            candidate_count,
            has_more,
        } => (
            CandidateEvaluationRecoveryMaintenanceOutcome::Recovered { candidate_count },
            has_more,
        ),
        CandidateEvaluationRecoveryOutcome::Incompatible { has_more } => (
            CandidateEvaluationRecoveryMaintenanceOutcome::Incompatible,
            has_more,
        ),
    }
}

const fn map_candidate_evaluation_recovery_maintenance_failure(
    error: CandidateEvaluationRecoveryError,
) -> TaskFailureKind {
    let kind = match error {
        CandidateEvaluationRecoveryError::InvalidClock => {
            CandidateEvaluationRecoveryMaintenanceFailureKind::InvalidClock
        }
        CandidateEvaluationRecoveryError::IncompatibleSchema => {
            CandidateEvaluationRecoveryMaintenanceFailureKind::IncompatibleSchema
        }
        CandidateEvaluationRecoveryError::Busy => {
            CandidateEvaluationRecoveryMaintenanceFailureKind::Busy
        }
        CandidateEvaluationRecoveryError::UnsafeStorage => {
            CandidateEvaluationRecoveryMaintenanceFailureKind::UnsafeStorage
        }
        CandidateEvaluationRecoveryError::BudgetExceeded => {
            CandidateEvaluationRecoveryMaintenanceFailureKind::BudgetExceeded
        }
        CandidateEvaluationRecoveryError::CorruptData => {
            CandidateEvaluationRecoveryMaintenanceFailureKind::CorruptData
        }
        CandidateEvaluationRecoveryError::Unavailable => {
            CandidateEvaluationRecoveryMaintenanceFailureKind::Unavailable
        }
        CandidateEvaluationRecoveryError::OutcomeUnknown => {
            CandidateEvaluationRecoveryMaintenanceFailureKind::OutcomeUnknown
        }
        CandidateEvaluationRecoveryError::InternalState => {
            CandidateEvaluationRecoveryMaintenanceFailureKind::InternalState
        }
    };
    TaskFailureKind::CandidateEvaluationRecoveryMaintenance(kind)
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

const fn public_snapshot_provisioning_stage_maintenance_outcome(
    outcome: &SnapshotProvisioningStageReconciliationBatchOutcome,
) -> SnapshotProvisioningStageMaintenanceOutcome {
    match outcome {
        SnapshotProvisioningStageReconciliationBatchOutcome::NoStage => {
            SnapshotProvisioningStageMaintenanceOutcome::NoStage
        }
        SnapshotProvisioningStageReconciliationBatchOutcome::DeferredUnproven => {
            SnapshotProvisioningStageMaintenanceOutcome::DeferredUnproven
        }
        SnapshotProvisioningStageReconciliationBatchOutcome::RemovedMarkerOnly { bytes } => {
            SnapshotProvisioningStageMaintenanceOutcome::RemovedMarkerOnly { bytes: *bytes }
        }
        SnapshotProvisioningStageReconciliationBatchOutcome::RemovedMarkerComplete { bytes } => {
            SnapshotProvisioningStageMaintenanceOutcome::RemovedMarkerComplete { bytes: *bytes }
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

const fn map_snapshot_provisioning_stage_maintenance_failure(
    kind: SnapshotRepositoryErrorKind,
) -> TaskFailureKind {
    let kind = match kind {
        SnapshotRepositoryErrorKind::ReadOnly
        | SnapshotRepositoryErrorKind::MissingStore
        | SnapshotRepositoryErrorKind::ReviewLeaseExpired => {
            SnapshotProvisioningStageMaintenanceFailureKind::InternalState
        }
        SnapshotRepositoryErrorKind::MissingSnapshot
        | SnapshotRepositoryErrorKind::SnapshotUnavailable
        | SnapshotRepositoryErrorKind::ReferenceMismatch => {
            SnapshotProvisioningStageMaintenanceFailureKind::CorruptData
        }
        // Provisioning-stage control bytes are never decoded as snapshots.
        SnapshotRepositoryErrorKind::IncompatibleVersion => {
            SnapshotProvisioningStageMaintenanceFailureKind::InternalState
        }
        SnapshotRepositoryErrorKind::Codec(kind) => match kind {
            SnapshotCodecErrorKind::InvalidInput | SnapshotCodecErrorKind::IncompatibleVersion => {
                SnapshotProvisioningStageMaintenanceFailureKind::InternalState
            }
            SnapshotCodecErrorKind::Io => {
                SnapshotProvisioningStageMaintenanceFailureKind::Unavailable
            }
            SnapshotCodecErrorKind::LimitExceeded => {
                SnapshotProvisioningStageMaintenanceFailureKind::BudgetExceeded
            }
            SnapshotCodecErrorKind::InvalidMagic
            | SnapshotCodecErrorKind::InvalidLength
            | SnapshotCodecErrorKind::ChecksumMismatch
            | SnapshotCodecErrorKind::CorruptData => {
                SnapshotProvisioningStageMaintenanceFailureKind::CorruptData
            }
        },
        SnapshotRepositoryErrorKind::Storage(kind) => match kind {
            SnapshotStorageErrorKind::InvalidConfiguration
            | SnapshotStorageErrorKind::InternalState => {
                SnapshotProvisioningStageMaintenanceFailureKind::InternalState
            }
            SnapshotStorageErrorKind::UnsafeRoot
            | SnapshotStorageErrorKind::UnsafeObject
            | SnapshotStorageErrorKind::UnrecognizedStore => {
                SnapshotProvisioningStageMaintenanceFailureKind::UnsafeStorage
            }
            SnapshotStorageErrorKind::Unavailable => {
                SnapshotProvisioningStageMaintenanceFailureKind::Unavailable
            }
            SnapshotStorageErrorKind::Busy => SnapshotProvisioningStageMaintenanceFailureKind::Busy,
        },
        SnapshotRepositoryErrorKind::History(kind) => match kind {
            HistoryErrorKind::InvalidInput => {
                SnapshotProvisioningStageMaintenanceFailureKind::InvalidClock
            }
            HistoryErrorKind::IncompatibleSchema => {
                SnapshotProvisioningStageMaintenanceFailureKind::IncompatibleSchema
            }
            HistoryErrorKind::QueryLimitExceeded => {
                SnapshotProvisioningStageMaintenanceFailureKind::BudgetExceeded
            }
            HistoryErrorKind::Busy => SnapshotProvisioningStageMaintenanceFailureKind::Busy,
            HistoryErrorKind::UnsafeStorage => {
                SnapshotProvisioningStageMaintenanceFailureKind::UnsafeStorage
            }
            HistoryErrorKind::CorruptData => {
                SnapshotProvisioningStageMaintenanceFailureKind::CorruptData
            }
            HistoryErrorKind::DatabaseUnavailable => {
                SnapshotProvisioningStageMaintenanceFailureKind::Unavailable
            }
            HistoryErrorKind::OutcomeUnknown => {
                SnapshotProvisioningStageMaintenanceFailureKind::OutcomeUnknown
            }
            HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::NotFound
            | HistoryErrorKind::InvalidTransition
            | HistoryErrorKind::InternalState => {
                SnapshotProvisioningStageMaintenanceFailureKind::InternalState
            }
        },
    };
    TaskFailureKind::SnapshotProvisioningStageMaintenance(kind)
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

struct AdmittedScanRoot {
    path: PathBuf,
    expected_identity: Option<FilesystemIdentity>,
}

// Home scans feed a retained DiskTree and path index. Keep the interactive
// app's default bounded so a large home directory cannot consume unbounded
// memory; the resulting scan is marked partial through IssueLimitReached.
const MAX_HOME_SCAN_NODES: usize = 200_000;

fn run_scan_task(
    context: TaskContext,
    admitted: AdmittedScanRoot,
    store: Arc<StoreCoordinator>,
    snapshots: Arc<SnapshotRepository>,
    before_traversal: impl FnOnce(&ScanId),
    before_candidate_evaluation: impl FnOnce(),
    before_candidate_persistence: impl FnOnce(),
) -> WorkOutcome {
    let AdmittedScanRoot {
        path: admitted_root,
        expected_identity: expected_root_identity,
    } = admitted;
    if prepare_scan_root(&admitted_root).ok().as_deref() != Some(admitted_root.as_path())
        || !current_root_matches(&admitted_root, expected_root_identity)
    {
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

    if !current_root_matches(&admitted_root, expected_root_identity) {
        return settle_changed_scan_root(&mut durable, start.started_at());
    }

    let scanner = Scanner::new(ScanConfig {
        follow_symlinks: false,
        max_depth: None,
        max_nodes: Some(MAX_HOME_SCAN_NODES),
        same_filesystem: true,
        // A single traversal worker avoids an additional unbounded jwalk
        // prefetch queue while the main thread retains each node/path.
        num_threads: 1,
    })
    .with_cancellation(cancellation);
    let (messages, scanner_handle) = scanner.scan(admitted_root.clone());
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
            if !current_root_matches(&admitted_root, expected_root_identity)
                || !artifact_root_matches(&artifact, expected_root_identity)
            {
                return settle_changed_scan_root(&mut durable, completed_at);
            }
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
            if !current_root_matches(&admitted_root, expected_root_identity) {
                return settle_changed_scan_root(&mut durable, completed_at);
            }
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

fn current_root_matches(root: &Path, expected: Option<FilesystemIdentity>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let Ok(root) = validate_scan_root(root) else {
        return false;
    };
    capture_scan_root(root).is_ok_and(|observed| observed.identity() == expected)
}

fn artifact_root_matches(
    artifact: &crate::scanner::CompletedScanArtifact,
    expected: Option<FilesystemIdentity>,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let (_, facts, _) = artifact.parts();
    scan_identity_matches(
        expected,
        facts.node(NodeId::ROOT).and_then(|facts| facts.identity),
    )
}

#[cfg(unix)]
fn scan_identity_matches(
    expected: FilesystemIdentity,
    observed: Option<ScanObjectIdentity>,
) -> bool {
    matches!(
        observed,
        Some(ScanObjectIdentity::Unix { device, inode })
            if expected.volume() == device && expected.object() == u128::from(inode)
    )
}

#[cfg(windows)]
fn scan_identity_matches(
    expected: FilesystemIdentity,
    observed: Option<ScanObjectIdentity>,
) -> bool {
    matches!(
        observed,
        Some(ScanObjectIdentity::Windows { volume_serial, file_id })
            if expected.volume() == volume_serial
                && expected.object() == u128::from_le_bytes(file_id)
    )
}

fn settle_changed_scan_root(
    durable: &mut DurableScanGuard,
    observed_at: SystemTime,
) -> WorkOutcome {
    let result = durable.settle(
        TerminalScanStatus::Failed,
        ScanTaskStatus::Failed,
        completion_time(observed_at),
        ScanCounts::default(),
        ScanCoverage::unknown(),
    );
    failed_scan_outcome(TaskFailureKind::ScanRootChanged, result)
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

#[cfg(test)]
#[path = "snapshot_performance_tests.rs"]
mod snapshot_performance_tests;

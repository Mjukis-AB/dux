use std::collections::{BTreeSet, HashMap, VecDeque};
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
#[cfg(all(test, target_os = "macos"))]
use thiserror::Error;

use super::ai_insight_cache::{
    AiCachedExplanation, AiInsightCacheClearError, AiInsightCacheClearPreview,
    AiInsightCacheClearResult, AiInsightCacheError,
    cache_binding_from_preview as ai_cache_binding_from_preview,
    map_cached_validation_error as map_ai_cached_validation_error,
    map_clear_store_error as map_ai_cache_clear_store_error,
    map_prepare_clear_error as map_ai_cache_prepare_clear_error,
    map_preview_error as map_ai_cache_preview_error, map_store_error as map_ai_cache_store_error,
    public_clear_result as public_ai_cache_clear_result, validate_and_prepare_cache_record,
};
use super::ai_metadata_preview::{
    AiExplanationAttempt, AiExplanationAttemptError, AiMetadataPreview, AiMetadataPreviewError,
    begin_anthropic_messages_v1_explanation as begin_bound_anthropic_messages_v1_explanation,
    prepare_ai_metadata_preview as prepare_bound_ai_metadata_preview,
};
use super::app_data_reset::{
    AppDataResetCompositionOutcome, AppDataResetCoreAdmission, AppDataResetPostTerminalRefusal,
    AppDataResetPreTerminalRefusal, AppDataResetRuntimeBlockers, AppDataResetTerminalOwner,
    with_terminal_store_preflight_until,
};
use super::automation::{
    AutomationGlobalControl, AutomationGlobalControlSource, AutomationGlobalControlUpdate,
    AutomationOverview, AutomationScheduleDraftDeleteOutcome,
    AutomationScheduleDraftEligibilityAssessment, AutomationScheduleDraftError,
    AutomationScheduleDraftUpdate, AutomationScheduleUpdate,
};
use super::automation_history_suggestion::{
    AutomationScheduleSuggestion, AutomationScheduleSuggestionError,
    AutomationScheduleSuggestionFeed, MAX_AUTOMATION_HISTORY_SUGGESTION_SOURCE_SESSIONS,
    MAX_AUTOMATION_HISTORY_SUGGESTIONS,
};
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
use super::cleanup_recovery_diagnostic::{
    CleanupRecoveryDiagnosticCensus, CleanupRecoveryDiagnosticCensusError,
    MAX_CLEANUP_RECOVERY_DIAGNOSTIC_CENSUS_ROWS,
};
use super::cloud_eviction_probe::{
    CloudEvictionProbeError, CloudEvictionProbePlatformError, CloudEvictionProbeRequest,
    probe_selected_file as probe_selected_cloud_eviction_file,
};
use super::config::EngineConfig;
use super::emergency_recovery::{
    EMERGENCY_RECOVERY_POLICY_REVISION, EmergencyRecoveryError, EmergencyRecoveryOrdering,
    EmergencyRecoveryScanObservation, build_emergency_recovery_groups,
    emergency_recovery_evidence_is_fresh,
};
use super::legacy_running_scan_dismissal::{
    LegacyRunningScanDismissalError, LegacyRunningScanDismissalPreview,
    LegacyRunningScanDismissalResult,
};
use super::managed_scan_cache::{
    DuxManagedScanCacheClearError, DuxManagedScanCacheClearPreview, DuxManagedScanCacheClearResult,
    DuxManagedScanCacheError, DuxOwnedStorageFootprintCacheError, ManagedScanCache,
};
use super::rule_outcome::{
    DurableRuleOutcome, DurableRuleOutcomeBatch, DurableRuleOutcomeState, RuleOutcomeError,
    RuleOutcomeNotEligibleReason,
};
use super::running_scan_debt::{
    ClaimedRunningScanProvenanceCensus, ClaimedRunningScanProvenanceCensusError,
    MAX_CLAIMED_RUNNING_SCAN_PROVENANCE_CENSUS_ROWS, MAX_RUNNING_SCAN_DEBT_CENSUS_ROWS,
    RunningScanDebtCensus, RunningScanDebtCensusError,
};
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
use super::rust_target_dry_run::{
    RustTargetDryRunError, RustTargetDryRunResult, RustTargetDryRunStartFailure,
};
#[cfg(target_os = "macos")]
use super::rust_target_dry_run::{
    failure_kind as rust_target_dry_run_failure_kind,
    generate_session_id as generate_dry_run_session_id,
    map_history_error as map_dry_run_history_error, result as rust_target_dry_run_result,
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
    ConfiguredProjectRoots, ConfiguredProjectRootsError, ConfiguredProjectRootsSource,
    ConfiguredProjectRootsUpdate, DirectCargoCodeSignature, DirectCargoEnrollmentError,
    DirectCargoEnrollmentPreview, DirectCargoEnrollmentState, DirectCargoEnrollmentStatus,
    DirectCargoEnrollmentUpdate, DirectCargoSignatureClass, DiskPressurePolicy,
    DiskPressurePolicyError, DiskPressurePolicySource, DiskPressurePolicyUpdate,
    PermanentCleanupPolicy, PermanentCleanupPolicyError, PermanentCleanupPolicySource,
    PermanentCleanupPolicyUpdate, SnapshotRetentionCap, SnapshotRetentionCapError,
    SnapshotRetentionCapSource, SnapshotRetentionCapUpdate,
};
use super::snapshot_diff_review::{
    SnapshotDiffCoverage, SnapshotDiffMetadata, SnapshotDiffReviewSession,
};
use super::snapshot_review::{
    MAX_SNAPSHOT_REVIEW_CATEGORY_BYTES, MAX_SNAPSHOT_REVIEW_CATEGORY_ROOTS,
    SnapshotReviewCategoryRoot, SnapshotReviewError, SnapshotReviewOwner, SnapshotReviewSession,
    category_path_bytes, map_repository_error as map_snapshot_review_error,
};
use super::snapshot_storage_clear::{
    DuxSnapshotStorageClearError, DuxSnapshotStorageClearPreview, DuxSnapshotStorageClearResult,
    map_clear_error as map_snapshot_storage_clear_error,
    public_clear_result as public_snapshot_storage_clear_result,
};
use super::storage_footprint::{
    DuxEmbeddedAiCacheFootprint, DuxLegacyExternalSnapshotStageCensus,
    DuxManagedScanCacheFootprint, DuxOwnedStorageFootprint, DuxOwnedStorageFootprintError,
    DuxOwnedStorageUsage, DuxSnapshotStorageFootprint,
};
use super::storage_thief::{
    DurableStorageThiefGroup, DurableStorageThiefRanking, MAX_STORAGE_THIEF_RANKING_GROUPS,
    MAX_STORAGE_THIEF_RANKING_SOURCE_SESSIONS, StorageThiefError,
};
use super::targeted_project_scan::{
    MAX_TARGETED_PRESSURE_CHAIN_EPISODES, TARGETED_RECLAIM_ROOT_POLICY_REVISION,
    TargetedProjectScanAdmission, TargetedProjectScanCheckpoint, TargetedProjectScanCurrent,
    TargetedProjectScanDisposition, TargetedProjectScanError, TargetedProjectScanPressure,
    TargetedProjectScanPressureContext, TargetedProjectScanSelection,
    TargetedReclaimRootCatalogStamp, TargetedReclaimRootKind, derive_low_pressure_chain,
    targeted_reclaim_root_catalog_layout,
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
    ScanTaskCounts, ScanTaskOrigin, ScanTaskResult, ScanTaskStatus,
    SnapshotOrphanMaintenanceFailureKind, SnapshotOrphanMaintenanceOutcome,
    SnapshotOrphanMaintenanceResult, SnapshotOrphanMaintenanceStartOutcome,
    SnapshotProvisioningStageMaintenanceFailureKind, SnapshotProvisioningStageMaintenanceOutcome,
    SnapshotProvisioningStageMaintenanceResult, SnapshotProvisioningStageMaintenanceStartOutcome,
    SnapshotRetentionFailureKind, SnapshotRetentionOutcome, SnapshotRetentionResult,
    SnapshotRetentionStartOutcome, SnapshotTerminalTempMaintenanceFailureKind,
    SnapshotTerminalTempMaintenanceOutcome, SnapshotTerminalTempMaintenanceResult,
    SnapshotTerminalTempMaintenanceStartOutcome, SnapshotUnleasedTempMaintenanceFailureKind,
    SnapshotUnleasedTempMaintenanceOutcome, SnapshotUnleasedTempMaintenanceResult,
    SnapshotUnleasedTempMaintenanceStartOutcome, StartSubtreeScanError, StartTaskError,
    TaskAccessError, TaskEvent, TaskEventBatch, TaskEventKind, TaskFailureKind, TaskId, TaskKind,
    TaskPhase, TaskPriority, TaskSnapshot,
};
use crate::cache::{
    CacheMetadata, CachedScanConfig, ManagedCacheStoreAccess, is_cache_valid, spot_check_mtimes,
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
    AutomationScheduleCadence, AutomationScheduleDraftConfig, AutomationScheduleId,
    AutomationSchedulePauseReason, AutomationScheduleState, CANDIDATE_CATALOG_SCHEMA_VERSION,
    CANDIDATE_CATALOG_SHA256, CANDIDATE_CONTEXT_FORMAT_VERSION, CANDIDATE_EVALUATOR_REVISION,
    CandidateEvaluationError, CandidateEvaluationScope, CandidateId, CandidateSnapshotReplayError,
    CleanupPlanId, CloudEvictionAssessment, CloudEvictionPlatformFacts, Evidence, ScanCoverage,
    ScanId, ScanIssueKind, bundled_automation_draft_policy_preflight,
    bundled_automation_eligible_rule_count, bundled_automation_history_suggestion_rules,
    candidate_evaluation_context_digest_sha256, evaluate_completed_scan_candidates,
    replay_snapshot_candidate_evaluation, validate_bundled_candidate_catalog,
};
use crate::path_validation::{
    CanonicalPathError, FilesystemIdentity, KnownUserLibraryCachesPath, TrustedHomeMountWitness,
    capture_scan_root, validate_scan_root,
};
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
    AppDataResetCompletedEngineOpenError, AppDataResetCoordinator,
    AppDataResetCoordinatorErrorKind, AppDataResetEngineLease, AppDataResetEngineLeaseOutcome,
    AppDataResetPhase, CandidateEvaluationCompletion, CandidateEvaluationFailureKind,
    CandidateEvaluationIdentity, CandidateEvaluationObservation, CandidateEvaluationRecord,
    CandidateEvaluationStatus, CandidateHistoryStatus, CandidateReviewAction,
    ClaimedRunningScanProvenanceCensus as StoredClaimedRunningScanProvenanceCensus,
    CleanupHistoryClearStoreError,
    CleanupRecoveryDiagnosticCensus as StoredCleanupRecoveryDiagnosticCensus, CleanupSessionId,
    CompleteCandidateRecord, DryRunJournalFailure, HistoryErrorKind, HostPathObservationEncoding,
    LegacyRunningScanDismissalStoreError, MAX_RECENT_SCAN_HISTORY_LIMIT, NewCandidateRecord,
    NewScanRecord, RunningScanDebtCensus as StoredRunningScanDebtCensus, ScanCompletionRecord,
    ScanCounts, ScanRecord, ScanStatus, SnapshotReviewPurpose, StoredCleanupErrorCategory,
    StoredCleanupHistoryCursor, StoredCleanupHistoryObservation, StoredCleanupItemStatus,
    StoredCleanupItemSummary, StoredCleanupMode, StoredCleanupRecordFormat,
    StoredCleanupSessionStatus, StoredCleanupSessionSummary, StoredCleanupStatusCounts,
    StoredCleanupTrigger, StoredRuleOutcome, StoredRuleOutcomeBatch,
    StoredRuleOutcomeNotEligibleReason, StoredRuleOutcomeState, TerminalScanStatus,
    ValidatedDryRunOutcome, observe_host_path,
};
use crate::persistence::{
    AutomationGlobalControl as StoredAutomationGlobalControl,
    AutomationGlobalControlSource as StoredAutomationGlobalControlSource,
    AutomationGlobalControlUpdate as StoredAutomationGlobalControlUpdate,
    AutomationScheduleDraftStoreUpdate, CargoCodeSignatureRecord, CargoEnrollmentSetting,
    CargoEnrollmentSettingUpdate, CargoEnrollmentState, CargoSignatureClass,
    CleanupExclusionSetting, CleanupExclusionSettingSource, CleanupExclusionSettingUpdate,
    ConfiguredProjectRootSetting, ConfiguredProjectRootSettingSource,
    ConfiguredProjectRootSettingUpdate, DiskPressurePolicySetting, DiskPressurePolicySettingSource,
    DiskPressurePolicySettingUpdate, PermanentCleanupSetting, PermanentCleanupSettingSource,
    PermanentCleanupSettingUpdate, SnapshotRetentionCapSetting, SnapshotRetentionCapSettingSource,
    SnapshotRetentionCapSettingUpdate, validate_configured_project_roots,
};
use crate::persistence::{
    CleanupJournalLease, DatabaseStatus,
    DuxOwnedStorageFootprint as StoredDuxOwnedStorageFootprint,
    LEGACY_EXTERNAL_SNAPSHOT_STAGE_CENSUS_MAX_ENTRIES, LegacyExternalSnapshotStageCensus,
    OwnedStorageUsage as StoredOwnedStorageUsage, ScanScopeLeaseErrorKind, ScanScopeLeaseToken,
    StoreCoordinator,
};
use crate::persistence::{
    MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS as STORED_MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS,
    MAX_AUTOMATION_HISTORY_SUGGESTIONS as STORED_MAX_AUTOMATION_HISTORY_SUGGESTIONS,
    MAX_STORAGE_THIEF_GROUPS, MAX_STORAGE_THIEF_SOURCE_SESSIONS,
    StoredAutomationScheduleSuggestion, StoredAutomationScheduleSuggestionFeed,
    StoredStorageThiefGroup, StoredStorageThiefRanking, compare_storage_thief_rates,
    storage_thief_rate_per_day,
};
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
#[cfg(target_os = "macos")]
use crate::planner::{
    RustTargetDryRunValidationError, RustTargetPromotion, TrustedRustTargetDryRun,
    prepare_rust_target_plan_facts, prepare_rust_target_promotion, review_rust_target_plan_facts,
};
#[cfg(all(test, target_os = "macos"))]
use crate::planner::{RustTargetJournalRequest, begin_rust_target_cleanup_session};
use crate::scanner::{
    CancellationToken, ScanConfig, ScanMessage, ScanObjectIdentity, ScanTermination, Scanner,
};
use crate::tree::{DiskTree, NodeId};

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
    RustTargetDryRun(Arc<RustTargetDryRunResult>),
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

#[derive(Clone, Copy)]
struct CancellationBoundary {
    cancellation_requested: bool,
    engine_open: bool,
}

impl TaskContext {
    fn is_cancellation_requested(&self) -> bool {
        self.cancellation.is_requested()
    }

    fn engine_is_open(&self) -> bool {
        self.shared.lock_registry_recover().lifecycle == EngineLifecycle::Open
    }

    /// Atomically closes the point at which cancellation can change this
    /// task's result. A cancellation accepted before this boundary is
    /// returned to the worker; later requests report that terminalization has
    /// already begun instead of claiming cancellation was accepted.
    fn close_cancellation_boundary(&self) -> CancellationBoundary {
        let mut registry = self.shared.lock_registry_recover();
        let engine_open = registry.lifecycle == EngineLifecycle::Open;
        let cancellation_requested = registry
            .records
            .get_mut(&self.id)
            .map(|record| {
                record.cancellation_closed = true;
                record.cancellation_requested
            })
            .unwrap_or(true);
        CancellationBoundary {
            cancellation_requested,
            engine_open,
        }
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
    priority: TaskPriority,
    work: Work,
}

enum TargetedScanSubmission {
    Started(TaskId),
    Existing { task_id: TaskId, phase: TaskPhase },
}

struct TargetedReclaimCatalog {
    stamp: TargetedReclaimRootCatalogStamp,
    roots: Vec<TargetedReclaimCatalogRoot>,
}

struct TargetedReclaimCatalogRoot {
    ordinal: u16,
    kind: TargetedReclaimRootKind,
    configured_root_ordinal: Option<u16>,
    display_root: PathBuf,
    prepared: Result<(PathBuf, FilesystemIdentity), ScanRootErrorKind>,
    known_user_cache: Option<KnownUserLibraryCachesPath>,
    excluded_subtrees: Vec<PathBuf>,
    max_nodes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TargetedScanAdmissionIdentity {
    root_kind: TargetedReclaimRootKind,
    root_ordinal: u16,
    root_identity: FilesystemIdentity,
    root_catalog_digest_sha256: [u8; 32],
    max_nodes: u32,
    excluded_subtrees: Vec<PathBuf>,
    pressure: TargetedProjectScanPressureContext,
}

struct TaskRecord {
    id: TaskId,
    kind: TaskKind,
    priority: TaskPriority,
    scan_origin: Option<ScanTaskOrigin>,
    targeted_root_kind: Option<TargetedReclaimRootKind>,
    targeted_admission: Option<TargetedScanAdmissionIdentity>,
    phase: TaskPhase,
    cancellation_requested: bool,
    cancellation_closed: bool,
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
        let priority = match kind {
            TaskKind::FormatSizeBatch => TaskPriority::UserInteractive,
            TaskKind::Scan => TaskPriority::UserFull,
            TaskKind::RustTargetDryRun | TaskKind::PermanentSafeCleanup => TaskPriority::Cleanup,
            TaskKind::ScanRecoveryMaintenance
            | TaskKind::CandidateEvaluationRecoveryMaintenance
            | TaskKind::HistoryMaintenance
            | TaskKind::SnapshotRetention
            | TaskKind::SnapshotOrphanMaintenance
            | TaskKind::SnapshotProvisioningStageMaintenance
            | TaskKind::SnapshotTerminalTempMaintenance
            | TaskKind::SnapshotUnleasedTempMaintenance => TaskPriority::Maintenance,
        };
        let scan_origin = (kind == TaskKind::Scan).then_some(ScanTaskOrigin::UserFull);
        Self::new_with_priority(id, kind, priority, scan_origin, scan_scope, event_limit)
    }

    fn new_scan(
        id: TaskId,
        origin: ScanTaskOrigin,
        scan_scope: PathBuf,
        event_limit: usize,
    ) -> Self {
        let priority = match origin {
            ScanTaskOrigin::UserFull => TaskPriority::UserFull,
            ScanTaskOrigin::UserSubtree => TaskPriority::UserSubtree,
            ScanTaskOrigin::TargetedRecommendation => TaskPriority::Targeted,
        };
        Self::new_with_priority(
            id,
            TaskKind::Scan,
            priority,
            Some(origin),
            Some(scan_scope),
            event_limit,
        )
    }

    fn new_with_priority(
        id: TaskId,
        kind: TaskKind,
        priority: TaskPriority,
        scan_origin: Option<ScanTaskOrigin>,
        scan_scope: Option<PathBuf>,
        event_limit: usize,
    ) -> Self {
        let mut record = Self {
            id,
            kind,
            priority,
            scan_origin,
            targeted_root_kind: None,
            targeted_admission: None,
            phase: TaskPhase::Queued,
            cancellation_requested: false,
            cancellation_closed: false,
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
            priority: self.priority,
            scan_origin: self.scan_origin,
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
    terminal_intent: Option<TerminalIntent>,
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
    RustTargetDryRun(TaskId),
    PermanentSafe(TaskId),
    Trash(u64),
    Quarantined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalIntent {
    OrdinaryClose,
    AppDataReset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalRequestOutcome {
    Initiated,
    AlreadyClosing(Option<TerminalIntent>),
    AlreadyClosed(Option<TerminalIntent>),
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
            terminal_intent: None,
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

    fn reset_workers_are_quiesced(&self) -> bool {
        let cleanup_worker_is_quiesced = matches!(
            self.active_cleanup_operation,
            None | Some(ActiveCleanupOperation::Trash(_))
                | Some(ActiveCleanupOperation::Quarantined)
        );
        self.queue.is_empty()
            && self.running_tasks == 0
            && self.live_workers == 0
            && self.active_scan_roots.is_empty()
            && self.active_scan_recovery_maintenance.is_none()
            && self
                .active_candidate_evaluation_recovery_maintenance
                .is_none()
            && self.active_history_maintenance.is_none()
            && self.active_snapshot_retention.is_none()
            && self.active_snapshot_orphan_maintenance.is_none()
            && self
                .active_snapshot_provisioning_stage_maintenance
                .is_none()
            && self.active_snapshot_terminal_temp_maintenance.is_none()
            && self.active_snapshot_unleased_temp_maintenance.is_none()
            && cleanup_worker_is_quiesced
    }

    fn retain_terminal(&mut self, id: TaskId, limit: usize) {
        self.terminal_order.push_back(id);
        while self.terminal_order.len() > limit {
            if let Some(expired) = self.terminal_order.pop_front() {
                self.records.remove(&expired);
            }
        }
    }

    fn enqueue(&mut self, job: Job) {
        let position = self
            .queue
            .iter()
            .position(|queued| queued.priority < job.priority);
        if let Some(position) = position {
            self.queue.insert(position, job);
        } else {
            self.queue.push_back(job);
        }
    }

    fn cancel_queued_task(
        &mut self,
        id: TaskId,
        event_limit: usize,
        terminal_limit: usize,
    ) -> Option<Job> {
        let position = self.queue.iter().position(|job| job.id == id)?;
        let job = self.queue.remove(position)?;
        let identity = self
            .records
            .get(&id)
            .map(|record| (record.kind, record.scan_scope.clone()));
        if let Some(record) = self.records.get_mut(&id) {
            if record.phase != TaskPhase::Queued {
                self.queue.insert(position, job);
                return None;
            }
            record.request_cancellation(event_limit);
            record.phase = TaskPhase::Cancelled;
            record.push_event(
                TaskEventKind::Terminal {
                    phase: TaskPhase::Cancelled,
                },
                event_limit,
            );
        } else {
            self.queue.insert(position, job);
            return None;
        }
        if let Some((kind, scope)) = identity {
            self.release_task_exclusivity(id, kind, scope.as_deref());
        }
        self.retain_terminal(id, terminal_limit);
        Some(job)
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
        if kind == TaskKind::RustTargetDryRun
            && self.active_cleanup_operation == Some(ActiveCleanupOperation::RustTargetDryRun(id))
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
    reset_engine_lease: Mutex<Option<AppDataResetEngineLease>>,
}

impl Shared {
    fn new(limits: RegistryLimits, reset_engine_lease: AppDataResetEngineLease) -> Self {
        Self {
            registry: Mutex::new(Registry::new()),
            workers_ready: Condvar::new(),
            lifecycle_changed: Condvar::new(),
            limits,
            reset_engine_lease: Mutex::new(Some(reset_engine_lease)),
        }
    }

    fn request_close(&self) -> CloseOutcome {
        match self.request_terminal(TerminalIntent::OrdinaryClose) {
            TerminalRequestOutcome::Initiated => CloseOutcome::Initiated,
            TerminalRequestOutcome::AlreadyClosing(_) => CloseOutcome::AlreadyClosing,
            TerminalRequestOutcome::AlreadyClosed(_) => CloseOutcome::AlreadyClosed,
        }
    }

    fn request_terminal(&self, intent: TerminalIntent) -> TerminalRequestOutcome {
        let registry = self.lock_registry_recover();
        self.request_terminal_with_registry(intent, registry)
    }

    fn request_terminal_until(
        &self,
        intent: TerminalIntent,
        deadline: Instant,
    ) -> Result<TerminalRequestOutcome, ()> {
        let registry = self.lock_registry_until(deadline)?;
        if Instant::now() >= deadline {
            return Err(());
        }
        Ok(self.request_terminal_with_registry(intent, registry))
    }

    fn request_terminal_with_registry(
        &self,
        intent: TerminalIntent,
        mut registry: MutexGuard<'_, Registry>,
    ) -> TerminalRequestOutcome {
        match registry.lifecycle {
            EngineLifecycle::Closing => {
                return TerminalRequestOutcome::AlreadyClosing(registry.terminal_intent);
            }
            EngineLifecycle::Closed => {
                return TerminalRequestOutcome::AlreadyClosed(registry.terminal_intent);
            }
            EngineLifecycle::Open => {
                registry.lifecycle = EngineLifecycle::Closing;
                registry.terminal_intent = Some(intent);
            }
        }

        // Keep queued closures alive until after the registry mutex is
        // released. Scan closures retain their cross-process scope lease, and
        // releasing one may acquire the persistence coordinator.
        let queued: Vec<_> = registry.queue.iter().map(|job| job.id).collect();
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
        drop(registry);
        TerminalRequestOutcome::Initiated
    }

    fn lock_registry_recover(&self) -> MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_registry_until(&self, deadline: Instant) -> Result<MutexGuard<'_, Registry>, ()> {
        loop {
            if Instant::now() >= deadline {
                return Err(());
            }
            match self.registry.try_lock() {
                Ok(registry) => return Ok(registry),
                Err(TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
                Err(TryLockError::WouldBlock) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(());
                    }
                    std::thread::sleep(
                        Duration::from_millis(5).min(deadline.saturating_duration_since(now)),
                    );
                }
            }
        }
    }
}

struct EngineInner {
    owner: Arc<()>,
    config: EngineConfig,
    store: Arc<StoreCoordinator>,
    snapshots: Arc<SnapshotRepository>,
    managed_scan_cache: ManagedScanCache,
    snapshot_review_owner: Arc<SnapshotReviewOwner>,
    startup_volume_pressure: Mutex<super::volume_status::StartupVolumePressureBaseline>,
    scan_admission: Mutex<()>,
    shared: Arc<Shared>,
    workers: Mutex<Option<Vec<JoinHandle<()>>>>,
}

impl EngineInner {
    fn join_workers(&self) {
        let mut workers = self
            .workers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(handles) = workers.take() {
            for handle in handles {
                let _ = handle.join();
            }
        }
    }
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

/// Unique terminal-lifecycle claim returned only to the reset contender that
/// atomically won against ordinary close and every second reset contender.
#[must_use = "dropping the winning reset claim leaves the old engine terminal without reset authority"]
pub struct AppDataResetShutdown {
    inner: Arc<EngineInner>,
}

/// Proof that the winning reset claim has drained all engine workers.
///
/// This non-cloneable capability will be consumed by the namespace-detachment
/// slice. It deliberately exposes no filesystem authority on its own.
#[must_use = "quiescence proof must be consumed by the reset effect boundary"]
pub struct AppDataResetQuiesced {
    _inner: Arc<EngineInner>,
}

/// Path-free result of the dormant cross-crate reset validation seam.
///
/// This type deliberately reports only bounded lifecycle and recovery state.
/// It carries no filesystem identity, transaction, journal, target, callback,
/// or reset-effect authority. No variant authorizes a reset effect.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[must_use = "reset validation determines whether the old engine became terminal"]
pub enum AppDataResetValidationOutcome {
    Validated,
    RefusedBeforeTerminal,
    RecoveryRequiredBeforeTerminal { phase: AppDataResetRecoveryPhase },
    TerminalOwnedByOrdinaryClose,
    TerminalOwnedByAppDataReset,
    RecoveryRequiredAfterTerminal { phase: AppDataResetRecoveryPhase },
    ShutdownIncomplete,
    TerminalWithoutValidation,
}

/// Path-free durable phase carried by reset-recovery validation outcomes.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AppDataResetRecoveryPhase {
    Prepared,
    CacheDetached,
    DataDetached,
    FreshNamespaceReady,
    Draining,
    Complete,
}

impl From<AppDataResetPhase> for AppDataResetRecoveryPhase {
    fn from(value: AppDataResetPhase) -> Self {
        match value {
            AppDataResetPhase::Prepared => Self::Prepared,
            AppDataResetPhase::CacheDetached => Self::CacheDetached,
            AppDataResetPhase::DataDetached => Self::DataDetached,
            AppDataResetPhase::FreshNamespaceReady => Self::FreshNamespaceReady,
            AppDataResetPhase::Draining => Self::Draining,
            AppDataResetPhase::Complete => Self::Complete,
        }
    }
}

fn classify_app_data_reset_validation_outcome(
    outcome: AppDataResetCompositionOutcome<()>,
) -> AppDataResetValidationOutcome {
    match outcome {
        AppDataResetCompositionOutcome::Admitted(()) => AppDataResetValidationOutcome::Validated,
        AppDataResetCompositionOutcome::PreTerminalRefused(
            AppDataResetPreTerminalRefusal::RecoveryRequired { phase },
        ) => AppDataResetValidationOutcome::RecoveryRequiredBeforeTerminal {
            phase: phase.into(),
        },
        AppDataResetCompositionOutcome::PreTerminalRefused(_) => {
            AppDataResetValidationOutcome::RefusedBeforeTerminal
        }
        AppDataResetCompositionOutcome::TerminalOwnedElsewhere(
            AppDataResetTerminalOwner::OrdinaryClose,
        ) => AppDataResetValidationOutcome::TerminalOwnedByOrdinaryClose,
        AppDataResetCompositionOutcome::TerminalOwnedElsewhere(
            AppDataResetTerminalOwner::AppDataReset,
        ) => AppDataResetValidationOutcome::TerminalOwnedByAppDataReset,
        AppDataResetCompositionOutcome::TerminalWithoutAdmission(
            AppDataResetPostTerminalRefusal::RecoveryRequired { phase },
        ) => AppDataResetValidationOutcome::RecoveryRequiredAfterTerminal {
            phase: phase.into(),
        },
        AppDataResetCompositionOutcome::TerminalWithoutAdmission(
            AppDataResetPostTerminalRefusal::Shutdown(
                super::AppDataResetShutdownError::ShutdownIncomplete,
            ),
        ) => AppDataResetValidationOutcome::ShutdownIncomplete,
        AppDataResetCompositionOutcome::TerminalWithoutAdmission(_) => {
            AppDataResetValidationOutcome::TerminalWithoutValidation
        }
    }
}

/// Result of atomically claiming the engine's terminal lifecycle for reset.
#[must_use = "reset admission determines whether this caller owns terminal shutdown"]
pub enum AppDataResetAdmissionOutcome {
    Admitted(AppDataResetShutdown),
    OrdinaryCloseWon,
    AlreadyResetting,
    InternalState,
}

impl AppDataResetShutdown {
    /// Consume the unique reset claim and wait for every worker to quiesce.
    ///
    /// Timeout consumes the claim permanently. The old engine remains closed
    /// and no reset effect capability is returned.
    pub fn wait_until_quiesced(
        self,
        timeout: Duration,
    ) -> Result<AppDataResetQuiesced, super::AppDataResetShutdownError> {
        self.wait_until_quiesced_for(timeout)
    }

    pub(crate) fn wait_until_quiesced_until(
        self,
        deadline: Instant,
    ) -> Result<AppDataResetQuiesced, super::AppDataResetShutdownError> {
        let registry = self
            .inner
            .shared
            .lock_registry_until(deadline)
            .map_err(|()| super::AppDataResetShutdownError::ShutdownIncomplete)?;
        let now = Instant::now();
        if now >= deadline {
            return Err(super::AppDataResetShutdownError::ShutdownIncomplete);
        }
        let (registry, _) = self
            .inner
            .shared
            .lifecycle_changed
            .wait_timeout_while(registry, deadline.saturating_duration_since(now), |state| {
                state.lifecycle != EngineLifecycle::Closed
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if Instant::now() >= deadline || registry.lifecycle != EngineLifecycle::Closed {
            return Err(super::AppDataResetShutdownError::ShutdownIncomplete);
        }
        self.finish_quiesced(registry)
    }

    fn wait_until_quiesced_for(
        self,
        timeout: Duration,
    ) -> Result<AppDataResetQuiesced, super::AppDataResetShutdownError> {
        let registry = self.inner.shared.lock_registry_recover();
        let (registry, _) = self
            .inner
            .shared
            .lifecycle_changed
            .wait_timeout_while(registry, timeout, |state| {
                state.lifecycle != EngineLifecycle::Closed
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if registry.lifecycle != EngineLifecycle::Closed {
            return Err(super::AppDataResetShutdownError::ShutdownIncomplete);
        }
        self.finish_quiesced(registry)
    }

    fn finish_quiesced(
        &self,
        registry: MutexGuard<'_, Registry>,
    ) -> Result<AppDataResetQuiesced, super::AppDataResetShutdownError> {
        if registry.terminal_intent != Some(TerminalIntent::AppDataReset) {
            return Err(super::AppDataResetShutdownError::InternalState);
        }
        if !registry.reset_workers_are_quiesced() {
            return Err(super::AppDataResetShutdownError::InternalState);
        }
        drop(registry);
        self.inner.join_workers();
        Ok(AppDataResetQuiesced {
            _inner: Arc::clone(&self.inner),
        })
    }
}

/// Process-wide exclusion for one standalone observation scan.
///
/// The handle carries no cleanup, snapshot, candidate, or filesystem-effect
/// authority. It is used by the interactive CLI, whose progressive TUI still
/// drives `Scanner` directly. Dropping it after the scanner has quiesced
/// releases only this exact random lease.
pub struct StandaloneScanScopeLease {
    canonical_root: PathBuf,
    store: Arc<StoreCoordinator>,
    token: Option<ScanScopeLeaseToken>,
}

impl StandaloneScanScopeLease {
    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    fn into_leased_work(mut self, work: Work) -> Work {
        Box::new(move |context| {
            let outcome = work(context);
            self.release();
            outcome
        })
    }

    fn release(&mut self) {
        if let Some(token) = self.token.take() {
            // The persistence boundary exact-reconciles release. If storage is
            // unavailable even for reconciliation, retaining the row is the
            // fail-closed outcome until this process is proven gone.
            let _ = self.store.release_scan_scope_lease(&token);
        }
    }
}

impl Drop for StandaloneScanScopeLease {
    fn drop(&mut self) {
        self.release();
    }
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
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the private reset composition is consumed by the namespace-witness slice"
        )
    )]
    fn app_data_reset_runtime_blockers_until(
        &self,
        deadline: Instant,
    ) -> Result<AppDataResetRuntimeBlockers, ()> {
        // Quarantine publication uses this same quarantine -> registry order.
        // Neither mutex survives the scalar observation.
        let quarantine = loop {
            if Instant::now() >= deadline {
                return Err(());
            }
            match process_cleanup_quarantine().try_lock() {
                Ok(quarantine) => break quarantine,
                Err(TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(());
                    }
                    std::thread::sleep(
                        Duration::from_millis(5).min(deadline.saturating_duration_since(now)),
                    );
                }
            }
        };
        let process_cleanup_quarantine = store_is_quarantined(&quarantine, &self.inner.store);
        let registry = self.inner.shared.lock_registry_until(deadline)?;
        if Instant::now() >= deadline {
            return Err(());
        }
        let active_cleanup_operation = registry.active_cleanup_operation.is_some();
        Ok(AppDataResetRuntimeBlockers::new(
            active_cleanup_operation,
            process_cleanup_quarantine,
        ))
    }

    /// Privately compose coordinator-first preflight, terminal worker
    /// quiescence, and retained data-namespace/cleanup/database/snapshot/cache
    /// exclusion.
    ///
    /// The callback receives no target or effect method and cannot let any
    /// retained proof escape. Present and absent cache namespaces are both
    /// fenced without provisioning. Its consume-once admission method may
    /// commit only the coordinator's `Prepared` intent; no reset-target
    /// namespace effect is exposed.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the private reset composition is consumed by reset intent and namespace-detachment slices"
        )
    )]
    pub(crate) fn with_app_data_reset_core_admission_until<T>(
        &self,
        deadline: Instant,
        admitted: impl for<
            'session,
            'storage,
            'guard,
            'store,
            'quiesced,
            'data,
            'snapshot,
            'cache,
            'runtime,
            'transaction,
        > FnOnce(
            AppDataResetCoreAdmission<
                'session,
                'storage,
                'guard,
                'store,
                'quiesced,
                'data,
                'snapshot,
                'cache,
                'runtime,
                'transaction,
            >,
        ) -> T,
    ) -> AppDataResetCompositionOutcome<T> {
        with_terminal_store_preflight_until(
            self,
            &self.inner.store,
            &self.inner.snapshots,
            &self.inner.managed_scan_cache,
            deadline,
            |deadline| self.app_data_reset_runtime_blockers_until(deadline),
            admitted,
        )
    }

    /// Run the dormant, path-free reset validation boundary using the caller's
    /// original absolute deadline.
    ///
    /// This is a Rust adapter seam for the FFI crate, not a UniFFI export. The
    /// method accepts no caller callback or payload. It writes no reset
    /// journal, grants no validation witness, and performs no namespace effect.
    #[doc(hidden)]
    pub fn __validate_app_data_reset_until(
        &self,
        deadline: Instant,
    ) -> AppDataResetValidationOutcome {
        classify_app_data_reset_validation_outcome(
            self.with_app_data_reset_core_admission_until(deadline, |_| ()),
        )
    }

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
        let reset_deadline = super::app_data_reset_recovery::deadline()
            .map_err(|_| EngineOpenError::ResetCoordinatorUnavailable)?;
        let data_root = config
            .database_path()
            .parent()
            .ok_or(EngineOpenError::ResetCoordinatorUnavailable)?;
        let data_root_name = data_root
            .file_name()
            .ok_or(EngineOpenError::ResetCoordinatorUnavailable)?;
        let data_parent = data_root
            .parent()
            .ok_or(EngineOpenError::ResetCoordinatorUnavailable)?;
        let canonical_data_parent = data_parent
            .canonicalize()
            .map_err(|_| EngineOpenError::ResetCoordinatorUnavailable)?;
        let canonical_data_root = canonical_data_parent.join(data_root_name);
        let database_name = config
            .database_path()
            .file_name()
            .ok_or(EngineOpenError::ResetCoordinatorUnavailable)?;
        let canonical_database_path = canonical_data_root.join(database_name);
        let (reset_engine_lease, completed_reset_store) =
            match AppDataResetCoordinator::acquire_engine_lease_until(
                &canonical_data_root,
                reset_deadline,
            )
            .map_err(|_| EngineOpenError::ResetCoordinatorUnavailable)?
            {
                AppDataResetEngineLeaseOutcome::Admitted(lease) => (lease, None),
                AppDataResetEngineLeaseOutcome::CompletedStateValidationRequired(intent) => {
                    match intent.open_store_until(
                        &canonical_database_path,
                        config.cache_directory(),
                        reset_deadline,
                    ) {
                        Ok((lease, store)) => (lease, Some(store)),
                        Err(AppDataResetCompletedEngineOpenError::Reset(error))
                            if matches!(
                                error.kind(),
                                AppDataResetCoordinatorErrorKind::Busy
                                    | AppDataResetCoordinatorErrorKind::ChangedSinceRead
                                    | AppDataResetCoordinatorErrorKind::InvalidTransition
                                    | AppDataResetCoordinatorErrorKind::Unavailable
                                    | AppDataResetCoordinatorErrorKind::OutcomeUnknown
                            ) =>
                        {
                            return Err(EngineOpenError::ResetRecoveryRequired);
                        }
                        Err(AppDataResetCompletedEngineOpenError::Reset(_)) => {
                            return Err(EngineOpenError::ResetCoordinatorUnavailable);
                        }
                        Err(AppDataResetCompletedEngineOpenError::Database(kind)) => {
                            return Err(EngineOpenError::Database(kind));
                        }
                    }
                }
                AppDataResetEngineLeaseOutcome::RecoveryRequired(intent) => {
                    match super::app_data_reset_recovery::recover_app_data_reset_before_open_until(
                        &canonical_database_path,
                        config.cache_directory(),
                        intent,
                        reset_deadline,
                    ) {
                        Ok(super::app_data_reset_recovery::AppDataResetPreOpenRecoveryOutcome::RecoveryRequired { phase }) => {
                            let _ = phase;
                            return Err(EngineOpenError::ResetRecoveryRequired);
                        }
                        Ok(super::app_data_reset_recovery::AppDataResetPreOpenRecoveryOutcome::CoordinatorUnavailable) => {
                            return Err(EngineOpenError::ResetCoordinatorUnavailable);
                        }
                        Err(
                            AppDataResetCoordinatorErrorKind::Busy
                            | AppDataResetCoordinatorErrorKind::ChangedSinceRead
                            | AppDataResetCoordinatorErrorKind::InvalidTransition
                            | AppDataResetCoordinatorErrorKind::Unavailable
                            | AppDataResetCoordinatorErrorKind::OutcomeUnknown,
                        ) => {
                            // An incomplete journal was already proven under
                            // the shared lease. Contention, drift, or outcome
                            // uncertainty cannot downgrade it into an ordinary
                            // open or a generic outage.
                            return Err(EngineOpenError::ResetRecoveryRequired);
                        }
                        Err(_) => return Err(EngineOpenError::ResetCoordinatorUnavailable),
                    }
                }
            };
        // Durable storage is validated and migrated before any worker becomes
        // observable, so a failed open cannot leave a live partial engine.
        let store = match completed_reset_store {
            Some(store) => store,
            None => StoreCoordinator::open(config.database_path())
                .map_err(|error| EngineOpenError::Database(error.kind))?,
        };
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
        let cache_access = match database_status.access {
            crate::persistence::DatabaseAccess::ReadWriteCurrent => {
                ManagedCacheStoreAccess::ReadWrite
            }
            crate::persistence::DatabaseAccess::ReadOnlyNewer { .. } => {
                ManagedCacheStoreAccess::ReadOnly
            }
        };
        // The managed cache accelerates presentation only. An unsafe,
        // unavailable, or unsupported cache is retained as typed state for
        // diagnostics, but it cannot prevent the engine from opening or a
        // fresh scan from running.
        let managed_scan_cache =
            ManagedScanCache::new(config.cache_directory().to_path_buf(), cache_access);
        let shared = Arc::new(Shared::new(limits, reset_engine_lease));
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
                managed_scan_cache,
                snapshot_review_owner: Arc::new(SnapshotReviewOwner::new()),
                startup_volume_pressure: Mutex::new(
                    super::volume_status::StartupVolumePressureBaseline::new(),
                ),
                scan_admission: Mutex::new(()),
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
        if record
            .scan_id()
            .as_str()
            .starts_with(crate::domain::KNOWN_USER_CACHE_SCAN_ID_PREFIX)
        {
            // Recovery has only an untrusted durable ID/root pair, not the
            // live OS-account cache-root witness required to re-enter this
            // scope. A later pressure pass may safely rescan it.
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
            CandidateEvaluationScope::SelectedScanRoot,
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

    /// Load a bounded newest-first page of durable Warning/Critical pressure
    /// intervals. This path-free telemetry grants no cleanup authority.
    pub fn pressure_episode_history(
        &self,
        volume_id: &crate::domain::VolumeId,
        anchor_at: SystemTime,
        limit: usize,
    ) -> Result<super::PressureEpisodeHistory, super::PressureEpisodeHistoryError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(super::PressureEpisodeHistoryError::Closed);
        }
        super::volume_status::load_pressure_episode_history(
            &self.inner.store,
            volume_id,
            anchor_at,
            limit,
        )
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

    /// Mint one short-lived path-free AI metadata preview from the exact
    /// retained Explorer review. The caller supplies only a snapshot node ID;
    /// the durable pin supplies both immutable bytes and typed coverage.
    pub fn prepare_ai_metadata_preview(
        &self,
        parent: &mut SnapshotReviewSession,
        selected_node_id: u64,
    ) -> Result<AiMetadataPreview, AiMetadataPreviewError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AiMetadataPreviewError::Closed);
        }
        let preview = prepare_bound_ai_metadata_preview(
            &self.inner.snapshot_review_owner,
            parent,
            selected_node_id,
        )?;
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AiMetadataPreviewError::Closed);
        }
        Ok(preview)
    }

    /// Consume one exact privacy preview into a single fixed Anthropic
    /// Messages v1 validation attempt. Provider, transport, model, request
    /// bytes, and the effective deadline are selected exclusively by core.
    pub fn begin_anthropic_messages_v1_explanation(
        &self,
        preview: AiMetadataPreview,
        parent: &SnapshotReviewSession,
    ) -> Result<AiExplanationAttempt, AiExplanationAttemptError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AiExplanationAttemptError::Closed);
        }
        let attempt = begin_bound_anthropic_messages_v1_explanation(preview, parent)?;
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AiExplanationAttemptError::Closed);
        }
        Ok(attempt)
    }

    /// Load one exact cached Anthropic Messages v1 explanation, if present.
    ///
    /// The canonical row never crosses this boundary. A hit is reparsed and
    /// projected through the live retained-review privacy proof, so stored
    /// snapshot identifiers cannot be reused as authority.
    pub fn load_cached_anthropic_messages_v1_explanation(
        &self,
        preview: &AiMetadataPreview,
        parent: &SnapshotReviewSession,
    ) -> Result<Option<AiCachedExplanation>, AiInsightCacheError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AiInsightCacheError::Closed);
        }
        let binding = ai_cache_binding_from_preview(
            preview.info(parent).map_err(map_ai_cache_preview_error)?,
        )?;
        let stored = self
            .inner
            .store
            .load_ai_insight_cache(&binding, SystemTime::now())
            .map_err(|error| map_ai_cache_store_error(error.kind))?;
        let Some(stored) = stored else {
            return Ok(None);
        };
        let result = preview
            .validate_cached_anthropic_messages_v1_output(parent, stored.canonical_payload())
            .map_err(map_ai_cached_validation_error)?;
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AiInsightCacheError::Closed);
        }
        Ok(Some(AiCachedExplanation::new(
            result,
            stored.created_at(),
            stored.expires_at(),
        )))
    }

    /// Consume one fixed provider attempt, validate its output, and make a
    /// best-effort sealed cache write. Persistence failure never discards or
    /// changes a safely validated in-memory explanation and never retries the
    /// provider request.
    pub fn validate_and_cache_anthropic_messages_v1_explanation(
        &self,
        attempt: AiExplanationAttempt,
        output_json_utf8: &[u8],
    ) -> Result<super::AiExplanationResult, AiExplanationAttemptError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AiExplanationAttemptError::Closed);
        }
        let (result, record) =
            validate_and_prepare_cache_record(attempt, output_json_utf8, SystemTime::now())?;
        if self.lifecycle() == EngineLifecycle::Open
            && let Some(record) = record
        {
            let _ = self.inner.store.insert_ai_insight_cache(&record);
        }
        Ok(result)
    }

    /// Attach the immediately preceding comparable retained snapshot to one
    /// exact Explorer review. The returned child is historical display state
    /// only and cannot resolve live paths or enter any cleanup boundary.
    pub fn prepare_explorer_snapshot_diff_review(
        &self,
        current: &SnapshotReviewSession,
    ) -> Result<SnapshotDiffReviewSession, SnapshotReviewError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(SnapshotReviewError::Closed);
        }
        if !current.belongs_to(&self.inner.snapshot_review_owner)
            || current.is_released()
            || current.scan_id().as_str().starts_with("scan:targeted:")
        {
            return Err(SnapshotReviewError::WrongParentReview);
        }
        current.validate_current()?;
        let current_scan = self
            .inner
            .store
            .load_scan(current.scan_id())
            .map_err(|error| {
                map_snapshot_review_error(SnapshotRepositoryErrorKind::History(error.kind))
            })?
            .ok_or(SnapshotReviewError::ScanNotFound)?;
        let baseline_scan = match self
            .inner
            .store
            .load_previous_comparable_snapshot_scan(&current_scan)
        {
            Ok(Some(scan)) => scan,
            Ok(None) => return Err(SnapshotReviewError::ComparableSnapshotUnavailable),
            Err(error) if error.kind == HistoryErrorKind::NotFound => {
                return Err(SnapshotReviewError::ComparableSnapshotUnavailable);
            }
            Err(error) => {
                return Err(map_snapshot_review_error(
                    SnapshotRepositoryErrorKind::History(error.kind),
                ));
            }
        };
        let baseline_reference = baseline_scan
            .snapshot()
            .cloned()
            .ok_or(SnapshotReviewError::InternalState)?;
        if current_scan.root() != baseline_scan.root()
            || current_scan.root_identity_v1_sha256().is_none()
            || current_scan.root_identity_v1_sha256() != baseline_scan.root_identity_v1_sha256()
        {
            return Err(SnapshotReviewError::ComparableSnapshotUnavailable);
        }
        let metadata = SnapshotDiffMetadata {
            current_scan_id: current_scan.id().clone(),
            baseline_scan_id: baseline_scan.id().clone(),
            current_started_at: current_scan.started_at(),
            current_completed_at: current_scan
                .completed_at()
                .ok_or(SnapshotReviewError::InternalState)?,
            baseline_started_at: baseline_scan.started_at(),
            baseline_completed_at: baseline_scan
                .completed_at()
                .ok_or(SnapshotReviewError::InternalState)?,
            current_coverage: snapshot_diff_coverage(current_scan.coverage())?,
            baseline_coverage: snapshot_diff_coverage(baseline_scan.coverage())?,
        };
        let mut baseline = self
            .acquire_explorer_snapshot_review_reference(baseline_scan.id(), &baseline_reference)?;
        if self.lifecycle() != EngineLifecycle::Open {
            let _ = baseline.release();
            return Err(SnapshotReviewError::Closed);
        }
        if let Err(error) = current.validate_current() {
            let _ = baseline.release();
            return Err(error);
        }
        Ok(SnapshotDiffReviewSession::new(
            Arc::clone(&self.inner.snapshot_review_owner),
            current.session_identity(),
            current.plan_review_liveness(),
            metadata,
            baseline,
        ))
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

    /// Load one managed scan-cache entry for presentation acceleration.
    ///
    /// Failure never authorizes fallback decoding or cleanup. Callers may
    /// treat every error as a cache miss and perform a fresh scan.
    pub fn load_managed_scan_cache(
        &self,
        root: &Path,
        config: &CachedScanConfig,
    ) -> Result<Option<(CacheMetadata, DiskTree)>, DuxManagedScanCacheError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DuxManagedScanCacheError::Closed);
        }
        match self.inner.managed_scan_cache.load(root, config)? {
            Some((metadata, tree))
                if is_cache_valid(&metadata, root, config) && spot_check_mtimes(&tree, 32) =>
            {
                Ok(Some((metadata, tree)))
            }
            _ => Ok(None),
        }
    }

    /// Publish one completed scan into the private managed cache.
    ///
    /// This is cache-only persistence. The matching standalone scan-scope
    /// lease is consumed and remains held through publication.
    pub fn save_managed_scan_cache(
        &self,
        lease: StandaloneScanScopeLease,
        config: &CachedScanConfig,
        metadata: &CacheMetadata,
        tree: &DiskTree,
    ) -> Result<(), DuxManagedScanCacheError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DuxManagedScanCacheError::Closed);
        }
        if !Arc::ptr_eq(&lease.store, &self.inner.store) {
            return Err(DuxManagedScanCacheError::InternalState);
        }
        self.inner
            .managed_scan_cache
            .save(lease.canonical_root(), config, metadata, tree)
    }

    /// Observe DUX's active marker-owned database, snapshot, and cache stores.
    ///
    /// This is a bounded, path-free read. It carries no cleanup authority and
    /// does not count the conventional outer cache container or legacy
    /// caller-selected CLI cache files. AI content is an embedded SQLite
    /// subset and is not added to `physical_total`.
    pub fn owned_storage_footprint(
        &self,
    ) -> Result<DuxOwnedStorageFootprint, DuxOwnedStorageFootprintError> {
        self.owned_storage_footprint_at(SystemTime::now())
    }

    fn owned_storage_footprint_at(
        &self,
        observed_at: SystemTime,
    ) -> Result<DuxOwnedStorageFootprint, DuxOwnedStorageFootprintError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DuxOwnedStorageFootprintError::Closed);
        }
        let observed = self
            .inner
            .snapshots
            .inspect_owned_storage_footprint_with(observed_at, || {
                self.inner.managed_scan_cache.footprint()
            })
            .map_err(|error| map_owned_storage_footprint_error(error.kind))?;
        let (footprint, managed_scan_cache) =
            observed.map_err(map_owned_storage_footprint_cache_error)?;
        let legacy_external_snapshot_stages = self
            .inner
            .store
            .legacy_external_snapshot_stage_census()
            .map_err(|error| map_owned_storage_footprint_history_error(error.kind))?;
        public_owned_storage_footprint(
            footprint,
            managed_scan_cache,
            legacy_external_snapshot_stages,
        )
    }

    /// Prepare one short-lived, path-free confirmation for clearing every
    /// exact currently removable retained snapshot final.
    ///
    /// Latest-two snapshots, active review pins, orphans, temporaries,
    /// provisioning stages, and store controls remain excluded.
    pub fn prepare_snapshot_storage_clear(
        &self,
    ) -> Result<DuxSnapshotStorageClearPreview, DuxSnapshotStorageClearError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DuxSnapshotStorageClearError::Closed);
        }
        let prepared_at = SystemTime::now();
        let monotonic_now = Instant::now();
        let prepared = self
            .inner
            .snapshots
            .prepare_snapshot_storage_clear(prepared_at)
            .map_err(|error| map_snapshot_storage_clear_error(error.kind))?
            .ok_or(DuxSnapshotStorageClearError::NothingToClear)?;
        DuxSnapshotStorageClearPreview::new(
            &self.inner.snapshots,
            prepared,
            prepared_at,
            monotonic_now,
        )
        .ok_or(DuxSnapshotStorageClearError::InternalState)
    }

    /// Prepare one two-minute, engine-bound confirmation over the complete
    /// current AI explanation cache. It exposes aggregate facts only.
    pub fn prepare_ai_insight_cache_clear(
        &self,
    ) -> Result<AiInsightCacheClearPreview, AiInsightCacheClearError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AiInsightCacheClearError::Closed);
        }
        let monotonic_now = Instant::now();
        let prepared = self
            .inner
            .store
            .prepare_ai_insight_cache_clear(SystemTime::now())
            .map_err(|error| map_ai_cache_prepare_clear_error(error.kind))?
            .ok_or(AiInsightCacheClearError::NothingToClear)?;
        AiInsightCacheClearPreview::new(&self.inner.store, prepared, monotonic_now)
            .ok_or(AiInsightCacheClearError::InternalState)
    }

    /// Consume one exact complete-population preview and clear only the
    /// unchanged AI cache. No digest, provider, path, or row selector is
    /// accepted.
    pub fn clear_ai_insight_cache(
        &self,
        preview: AiInsightCacheClearPreview,
    ) -> Result<AiInsightCacheClearResult, AiInsightCacheClearError> {
        self.clear_ai_insight_cache_at(preview, Instant::now())
    }

    fn clear_ai_insight_cache_at(
        &self,
        preview: AiInsightCacheClearPreview,
        now: Instant,
    ) -> Result<AiInsightCacheClearResult, AiInsightCacheClearError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AiInsightCacheClearError::Closed);
        }
        if !preview.belongs_to(&self.inner.store) {
            return Err(AiInsightCacheClearError::WrongEngine);
        }
        let info = preview.info_at(now)?;
        let prepared = preview.into_prepared(now)?;
        let stored = self
            .inner
            .store
            .clear_ai_insight_cache(prepared)
            .map_err(map_ai_cache_clear_store_error)?;
        public_ai_cache_clear_result(stored, info)
    }

    #[cfg(test)]
    fn clear_ai_insight_cache_at_expiry_for_test(
        &self,
        preview: AiInsightCacheClearPreview,
    ) -> Result<AiInsightCacheClearResult, AiInsightCacheClearError> {
        let expires_at = preview.monotonic_expires_at_for_test();
        self.clear_ai_insight_cache_at(preview, expires_at)
    }

    /// Consume one exact preview and clear only the unchanged removable
    /// snapshot-final population. The caller supplies no path or selector.
    pub fn clear_snapshot_storage(
        &self,
        preview: DuxSnapshotStorageClearPreview,
    ) -> Result<DuxSnapshotStorageClearResult, DuxSnapshotStorageClearError> {
        self.clear_snapshot_storage_at(preview, Instant::now(), SystemTime::now())
    }

    fn clear_snapshot_storage_at(
        &self,
        preview: DuxSnapshotStorageClearPreview,
        monotonic_now: Instant,
        observed_at: SystemTime,
    ) -> Result<DuxSnapshotStorageClearResult, DuxSnapshotStorageClearError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DuxSnapshotStorageClearError::Closed);
        }
        if !preview.belongs_to(&self.inner.snapshots) {
            return Err(DuxSnapshotStorageClearError::WrongEngine);
        }
        let info = preview.info_at(monotonic_now)?;
        let prepared = preview.into_prepared(monotonic_now)?;
        let result = self
            .inner
            .snapshots
            .clear_snapshot_storage(prepared, observed_at)
            .map_err(|error| map_snapshot_storage_clear_error(error.kind))?;
        public_snapshot_storage_clear_result(result, info)
    }

    #[cfg(test)]
    fn clear_snapshot_storage_at_expiry_for_test(
        &self,
        preview: DuxSnapshotStorageClearPreview,
    ) -> Result<DuxSnapshotStorageClearResult, DuxSnapshotStorageClearError> {
        let expires_at = preview.monotonic_expires_at_for_test();
        self.clear_snapshot_storage_at(preview, expires_at, SystemTime::now())
    }

    /// Prepare one short-lived, path-free confirmation for clearing the exact
    /// current marker-owned scan-cache population.
    pub fn prepare_managed_scan_cache_clear(
        &self,
    ) -> Result<DuxManagedScanCacheClearPreview, DuxManagedScanCacheClearError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DuxManagedScanCacheClearError::Closed);
        }
        self.inner
            .managed_scan_cache
            .prepare_clear(SystemTime::now(), Instant::now())
    }

    /// Consume one exact preview and clear only the unchanged managed cache.
    ///
    /// The caller supplies no path, key, selector, or cleanup effect.
    pub fn clear_managed_scan_cache(
        &self,
        preview: DuxManagedScanCacheClearPreview,
    ) -> Result<DuxManagedScanCacheClearResult, DuxManagedScanCacheClearError> {
        self.clear_managed_scan_cache_at(preview, Instant::now())
    }

    fn clear_managed_scan_cache_at(
        &self,
        preview: DuxManagedScanCacheClearPreview,
        now: Instant,
    ) -> Result<DuxManagedScanCacheClearResult, DuxManagedScanCacheClearError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(DuxManagedScanCacheClearError::Closed);
        }
        self.inner.managed_scan_cache.clear(preview, now)
    }

    #[cfg(test)]
    fn clear_managed_scan_cache_at_expiry_for_test(
        &self,
        preview: DuxManagedScanCacheClearPreview,
    ) -> Result<DuxManagedScanCacheClearResult, DuxManagedScanCacheClearError> {
        let expires_at = preview.monotonic_expires_at_for_test();
        self.clear_managed_scan_cache_at(preview, expires_at)
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

    /// Load the effective global permanent-cleanup opt-in. This deny-by-default
    /// gate carries no plan, target, or effect authority.
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

    /// Persist the global permanent-cleanup opt-in. Disabling takes the same
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

    /// Restore the versioned default (disabled) while retaining a new durable
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

    /// Load the complete bounded schedule registry, dedicated master control,
    /// and shipped-policy preflight. Persisted activation is not execution
    /// authority; the execution gate remains closed in this checkpoint.
    pub fn automation_overview(&self) -> Result<AutomationOverview, AutomationScheduleDraftError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AutomationScheduleDraftError::Closed);
        }
        let global_control = self
            .inner
            .store
            .load_automation_global_control()
            .map(public_automation_global_control)
            .map_err(|error| map_automation_schedule_error(error.kind))?;
        let schedules = self
            .inner
            .store
            .load_automation_schedule_drafts()
            .map_err(|error| map_automation_schedule_error(error.kind))?;
        let eligible_rule_count = bundled_automation_eligible_rule_count()
            .map_err(|_| AutomationScheduleDraftError::InternalState)?;
        let schedule_eligibility = schedules
            .iter()
            .map(|schedule| {
                let preflight = bundled_automation_draft_policy_preflight(schedule.config())
                    .map_err(|_| AutomationScheduleDraftError::InternalState)?;
                Ok(AutomationScheduleDraftEligibilityAssessment::new(
                    schedule.id().clone(),
                    schedule.revision(),
                    preflight.included_rule_count,
                    preflight.reasons,
                ))
            })
            .collect::<Result<Vec<_>, AutomationScheduleDraftError>>()?;
        Ok(AutomationOverview {
            global_control,
            execution_available: false,
            eligible_rule_count,
            schedules,
            schedule_eligibility,
        })
    }

    /// Set the dedicated global automation master control under exact
    /// revision CAS. Enabling remains effect-dormant and is refused if an
    /// already-enabled schedule would require the deferred atomic rebase.
    pub fn set_automation_global_enabled(
        &self,
        expected_revision: u64,
        enabled: bool,
    ) -> Result<AutomationGlobalControlUpdate, AutomationScheduleDraftError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AutomationScheduleDraftError::Closed);
        }
        if expected_revision > i64::MAX as u64 {
            return Err(AutomationScheduleDraftError::InvalidInput);
        }
        if enabled {
            let schedules = self
                .inner
                .store
                .load_automation_schedule_drafts()
                .map_err(|error| map_automation_schedule_error(error.kind))?;
            if schedules
                .iter()
                .any(|schedule| schedule.state() == AutomationScheduleState::Enabled)
            {
                return Err(AutomationScheduleDraftError::ActivationUnavailable);
            }
        }
        self.inner
            .store
            .set_automation_global_enabled(expected_revision, enabled)
            .map(public_automation_global_control_update)
            .map_err(|error| map_automation_schedule_error(error.kind))
    }

    /// Restore the disabled default without deleting an existing revision.
    pub fn reset_automation_global_control(
        &self,
        expected_revision: u64,
    ) -> Result<AutomationGlobalControlUpdate, AutomationScheduleDraftError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AutomationScheduleDraftError::Closed);
        }
        if expected_revision > i64::MAX as u64 {
            return Err(AutomationScheduleDraftError::InvalidInput);
        }
        self.inner
            .store
            .reset_automation_global_control(expected_revision)
            .map(public_automation_global_control_update)
            .map_err(|error| map_automation_schedule_error(error.kind))
    }

    /// Suggest exact current rules from a bounded window of manual,
    /// permanent-safe attempts. This read cannot create or mutate a schedule.
    pub fn automation_schedule_suggestions(
        &self,
    ) -> Result<AutomationScheduleSuggestionFeed, AutomationScheduleSuggestionError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AutomationScheduleSuggestionError::Closed);
        }
        let rules = bundled_automation_history_suggestion_rules()
            .map_err(|_| AutomationScheduleSuggestionError::InternalState)?;
        let feed = self
            .inner
            .store
            .automation_schedule_suggestions(&rules)
            .map_err(|error| map_automation_schedule_suggestion_error(error.kind))?;
        public_automation_schedule_suggestion_feed(feed, rules.len())
    }

    /// Create an inert disabled draft with a core-generated opaque ID.
    pub fn create_automation_schedule_draft(
        &self,
        config: AutomationScheduleDraftConfig,
    ) -> Result<AutomationScheduleDraftUpdate, AutomationScheduleDraftError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AutomationScheduleDraftError::Closed);
        }
        if self
            .inner
            .store
            .load_automation_schedule_drafts()
            .map_err(|error| map_automation_schedule_error(error.kind))?
            .len()
            >= crate::domain::MAX_AUTOMATION_SCHEDULE_DRAFTS
        {
            return Err(AutomationScheduleDraftError::DraftLimitExceeded);
        }
        for _ in 0..3 {
            let id = generate_automation_schedule_id()?;
            match self.inner.store.create_automation_schedule_draft(
                id,
                config.clone(),
                SystemTime::now(),
            ) {
                Ok(update) => return Ok(public_automation_schedule_update(update)),
                Err(error) if error.kind == HistoryErrorKind::AlreadyExists => continue,
                Err(error) if error.kind == HistoryErrorKind::QueryLimitExceeded => {
                    return Err(AutomationScheduleDraftError::DraftLimitExceeded);
                }
                Err(error) => return Err(map_automation_schedule_error(error.kind)),
            }
        }
        Err(AutomationScheduleDraftError::InternalState)
    }

    /// Replace only the exact revision most recently reviewed by the caller.
    /// The resulting row remains a disabled draft.
    pub fn replace_automation_schedule_draft(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
        config: AutomationScheduleDraftConfig,
    ) -> Result<AutomationScheduleDraftUpdate, AutomationScheduleDraftError> {
        let schedule = self.automation_schedule_for_transition(id, expected_revision)?;
        if schedule.state() != AutomationScheduleState::Disabled {
            return Err(AutomationScheduleDraftError::InvalidStateTransition);
        }
        self.inner
            .store
            .replace_automation_schedule_draft(id, expected_revision, config, SystemTime::now())
            .map(public_automation_schedule_update)
            .map_err(|error| map_automation_schedule_error(error.kind))
    }

    /// Save explicit activation consent for one exact disabled calendar
    /// schedule. Static shipped policy must already admit its complete scope;
    /// runtime eligibility and execution remain unavailable.
    pub fn enable_automation_schedule(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
    ) -> Result<AutomationScheduleUpdate, AutomationScheduleDraftError> {
        let schedule = self.automation_schedule_for_transition(id, expected_revision)?;
        if schedule.state() != AutomationScheduleState::Disabled {
            return Err(AutomationScheduleDraftError::InvalidStateTransition);
        }
        if schedule.config().cadence() == AutomationScheduleCadence::LowDiskOnly {
            return Err(AutomationScheduleDraftError::ActivationUnavailable);
        }
        self.require_automation_static_preflight(&schedule)?;
        self.inner
            .store
            .enable_automation_schedule_periodic(id, expected_revision, SystemTime::now())
            .map(public_automation_schedule_update)
            .map_err(|error| map_automation_schedule_error(error.kind))
    }

    /// Pause one exact enabled schedule. This changes saved state only and
    /// invalidates any stale future observation through the schedule revision.
    pub fn pause_automation_schedule(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
    ) -> Result<AutomationScheduleUpdate, AutomationScheduleDraftError> {
        let schedule = self.automation_schedule_for_transition(id, expected_revision)?;
        if schedule.state() != AutomationScheduleState::Enabled {
            return Err(AutomationScheduleDraftError::InvalidStateTransition);
        }
        self.inner
            .store
            .pause_automation_schedule(
                id,
                expected_revision,
                AutomationSchedulePauseReason::User,
                SystemTime::now(),
            )
            .map(public_automation_schedule_update)
            .map_err(|error| map_automation_schedule_error(error.kind))
    }

    /// Resume only a user-paused calendar schedule. Failure-paused and
    /// low-disk schedules require separately reviewed evidence adapters.
    pub fn resume_automation_schedule(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
    ) -> Result<AutomationScheduleUpdate, AutomationScheduleDraftError> {
        let schedule = self.automation_schedule_for_transition(id, expected_revision)?;
        if schedule.state() != AutomationScheduleState::Paused(AutomationSchedulePauseReason::User)
        {
            return Err(AutomationScheduleDraftError::InvalidStateTransition);
        }
        if schedule.config().cadence() == AutomationScheduleCadence::LowDiskOnly {
            return Err(AutomationScheduleDraftError::ActivationUnavailable);
        }
        self.require_automation_static_preflight(&schedule)?;
        self.inner
            .store
            .resume_automation_schedule(id, expected_revision, SystemTime::now())
            .map(public_automation_schedule_update)
            .map_err(|error| map_automation_schedule_error(error.kind))
    }

    /// End one exact activation and return it to editable disabled state.
    pub fn disable_automation_schedule(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
    ) -> Result<AutomationScheduleUpdate, AutomationScheduleDraftError> {
        let schedule = self.automation_schedule_for_transition(id, expected_revision)?;
        if schedule.state() == AutomationScheduleState::Disabled {
            return Ok(AutomationScheduleUpdate {
                schedule,
                changed: false,
            });
        }
        self.inner
            .store
            .disable_automation_schedule(id, expected_revision, SystemTime::now())
            .map(public_automation_schedule_update)
            .map_err(|error| map_automation_schedule_error(error.kind))
    }

    /// Delete one exact schedule in any state. A repeated delete is idempotent
    /// and reports `deleted = false`.
    pub fn delete_automation_schedule_draft(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
    ) -> Result<AutomationScheduleDraftDeleteOutcome, AutomationScheduleDraftError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AutomationScheduleDraftError::Closed);
        }
        if expected_revision == 0 {
            return Err(AutomationScheduleDraftError::InvalidInput);
        }
        if expected_revision > i64::MAX as u64 {
            return Err(AutomationScheduleDraftError::InvalidInput);
        }
        self.inner
            .store
            .delete_automation_schedule_draft(id, expected_revision)
            .map(|deleted| AutomationScheduleDraftDeleteOutcome { deleted })
            .map_err(|error| map_automation_schedule_error(error.kind))
    }

    fn automation_schedule_for_transition(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
    ) -> Result<crate::domain::AutomationSchedule, AutomationScheduleDraftError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(AutomationScheduleDraftError::Closed);
        }
        if expected_revision == 0 {
            return Err(AutomationScheduleDraftError::InvalidInput);
        }
        if expected_revision >= i64::MAX as u64 {
            return Err(AutomationScheduleDraftError::RevisionExhausted);
        }
        let schedule = self
            .inner
            .store
            .load_automation_schedule_drafts()
            .map_err(|error| map_automation_schedule_error(error.kind))?
            .into_iter()
            .find(|schedule| schedule.id() == id)
            .ok_or(AutomationScheduleDraftError::NotFound)?;
        if schedule.revision() != expected_revision {
            return Err(AutomationScheduleDraftError::RevisionConflict);
        }
        Ok(schedule)
    }

    fn require_automation_static_preflight(
        &self,
        schedule: &crate::domain::AutomationSchedule,
    ) -> Result<(), AutomationScheduleDraftError> {
        let preflight = bundled_automation_draft_policy_preflight(schedule.config())
            .map_err(|_| AutomationScheduleDraftError::InternalState)?;
        if preflight.included_rule_count == 0 || !preflight.reasons.is_empty() {
            return Err(AutomationScheduleDraftError::StaticPolicyBlocked);
        }
        Ok(())
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

    /// Load the bounded, discovery-only configured project-root registry.
    ///
    /// Roots are scan hints and never authorize filesystem access or cleanup.
    pub fn configured_project_roots(
        &self,
    ) -> Result<ConfiguredProjectRoots, ConfiguredProjectRootsError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(ConfiguredProjectRootsError::Closed);
        }
        self.inner
            .store
            .load_configured_project_roots()
            .map(public_configured_project_roots)
            .map_err(|error| map_configured_project_roots_error(error.kind))
    }

    /// Inspect one core-selected Explorer file for iCloud local-copy eviction
    /// eligibility. This is read-only and returns no path or cleanup authority.
    pub fn probe_explorer_cloud_eviction<F>(
        &self,
        review: &mut SnapshotReviewSession,
        node_id: u64,
        probe: F,
    ) -> Result<CloudEvictionAssessment, CloudEvictionProbeError>
    where
        F: FnOnce(
            CloudEvictionProbeRequest,
        ) -> Result<CloudEvictionPlatformFacts, CloudEvictionProbePlatformError>,
    {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CloudEvictionProbeError::Closed);
        }
        if !review.belongs_to(&self.inner.snapshot_review_owner) {
            return Err(CloudEvictionProbeError::WrongReview);
        }
        probe_selected_cloud_eviction_file(review, node_id, probe)
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

    /// Admit one exact Rust-target plan review to an effect-free, consume-once
    /// dry-run task.
    ///
    /// Refused admission returns the unconsumed opaque review. Accepted
    /// admission converts it inside core to a mode-bound dry-run capability
    /// that has no effect witness, platform driver, capacity sampler, or
    /// permanent-cleanup policy dependency.
    pub fn start_rust_target_dry_run(
        &self,
        review: RustTargetPlanReview,
    ) -> Result<TaskId, RustTargetDryRunStartFailure> {
        self.start_rust_target_dry_run_with_hook(review, Box::new(|| {}))
    }

    fn start_rust_target_dry_run_with_hook(
        &self,
        review: RustTargetPlanReview,
        before_validation: Box<dyn FnOnce() + Send>,
    ) -> Result<TaskId, RustTargetDryRunStartFailure> {
        if !review.belongs_to(&self.inner.snapshot_review_owner) {
            return Err(RustTargetDryRunStartFailure::new(
                RustTargetDryRunError::WrongEngine,
                review,
            ));
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = before_validation;
            return Err(RustTargetDryRunStartFailure::new(
                RustTargetDryRunError::Unavailable,
                review,
            ));
        }
        #[cfg(target_os = "macos")]
        {
            let quarantine = process_cleanup_quarantine()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if store_is_quarantined(&quarantine, &self.inner.store) {
                return Err(RustTargetDryRunStartFailure::new(
                    RustTargetDryRunError::HistoryUnresolved,
                    review,
                ));
            }
            let mut registry = match self.inner.shared.registry.lock() {
                Ok(registry) => registry,
                Err(_) => {
                    return Err(RustTargetDryRunStartFailure::new(
                        RustTargetDryRunError::InternalState,
                        review,
                    ));
                }
            };
            if registry.lifecycle != EngineLifecycle::Open {
                return Err(RustTargetDryRunStartFailure::new(
                    RustTargetDryRunError::Closed,
                    review,
                ));
            }
            if registry.queue.len() >= self.inner.shared.limits.queued_tasks {
                return Err(RustTargetDryRunStartFailure::new(
                    RustTargetDryRunError::QueueFull,
                    review,
                ));
            }
            match registry.active_cleanup_operation {
                Some(ActiveCleanupOperation::Quarantined) => {
                    return Err(RustTargetDryRunStartFailure::new(
                        RustTargetDryRunError::HistoryUnresolved,
                        review,
                    ));
                }
                Some(_) => {
                    return Err(RustTargetDryRunStartFailure::new(
                        RustTargetDryRunError::Busy,
                        review,
                    ));
                }
                None => {}
            }
            let id = match TASK_IDS.allocate() {
                Ok(id) => id,
                Err(_) => {
                    return Err(RustTargetDryRunStartFailure::new(
                        RustTargetDryRunError::InternalState,
                        review,
                    ));
                }
            };
            let admitted_at = SystemTime::now();
            let admitted = review
                .into_reviewed_at(&self.inner.snapshot_review_owner, admitted_at)
                .map_err(map_rust_target_dry_run_review_error)
                .and_then(|reviewed| {
                    reviewed
                        .into_rust_target_dry_run()
                        .map_err(|_| RustTargetDryRunError::ChangedDuringReview)
                });
            let store = Arc::clone(&self.inner.store);
            let work: Work = Box::new(move |context| {
                run_rust_target_dry_run_task(
                    store,
                    admitted,
                    admitted_at,
                    before_validation,
                    &context,
                )
            });
            let record = TaskRecord::new(
                id,
                TaskKind::RustTargetDryRun,
                None,
                self.inner.shared.limits.events_per_task,
            );
            registry.active_cleanup_operation = Some(ActiveCleanupOperation::RustTargetDryRun(id));
            registry.records.insert(id, record);
            registry.enqueue(Job {
                id,
                priority: TaskPriority::Cleanup,
                work,
            });
            self.inner.shared.workers_ready.notify_one();
            Ok(id)
        }
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
            registry.enqueue(Job {
                id,
                priority: TaskPriority::Cleanup,
                work,
            });
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

    /// Replace the bounded configured project-root registry. The core stores
    /// normalized paths losslessly, but does not start a scan or create cleanup
    /// authority from them.
    pub fn set_configured_project_roots(
        &self,
        roots: Vec<std::path::PathBuf>,
    ) -> Result<ConfiguredProjectRootsUpdate, ConfiguredProjectRootsError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(ConfiguredProjectRootsError::Closed);
        }
        validate_configured_project_roots(&roots)
            .map_err(|_| ConfiguredProjectRootsError::InvalidInput)?;
        self.inner
            .store
            .set_configured_project_roots(roots)
            .map(public_configured_project_roots_update)
            .map_err(|error| map_configured_project_roots_error(error.kind))
    }

    /// Delete the stored registry and restore the empty Default revision zero.
    pub fn reset_configured_project_roots(
        &self,
    ) -> Result<ConfiguredProjectRootsUpdate, ConfiguredProjectRootsError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(ConfiguredProjectRootsError::Closed);
        }
        self.inner
            .store
            .reset_configured_project_roots()
            .map(public_configured_project_roots_update)
            .map_err(|error| map_configured_project_roots_error(error.kind))
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
        let scans = page.records().iter().map(public_scan_summary).collect();
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

    /// Derive one path-free outcome for every item in an exact schema-v2
    /// cleanup session. This performs no durable write and grants no cleanup,
    /// scan, candidate, scheduling, or filesystem authority.
    pub fn rule_outcomes_for_cleanup_session(
        &self,
        session_id: &DurableCleanupSessionId,
    ) -> Result<DurableRuleOutcomeBatch, RuleOutcomeError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(RuleOutcomeError::Closed);
        }
        let stored_id = CleanupSessionId::new(session_id.as_str().to_owned())
            .map_err(|_| RuleOutcomeError::InternalState)?;
        let batch = self
            .inner
            .store
            .rule_outcomes_for_cleanup_session(&stored_id)
            .map_err(|error| map_rule_outcome_error(error.kind))?
            .ok_or(RuleOutcomeError::SessionNotFound)?;
        public_rule_outcome_batch(batch)
    }

    /// Rank recurring deterministic rule groups from one fixed, recent
    /// permanent-safe history window. This read exposes no path, cleanup,
    /// candidate, scheduling, AI, or filesystem authority.
    pub fn recurring_storage_thieves(
        &self,
    ) -> Result<DurableStorageThiefRanking, StorageThiefError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(StorageThiefError::Closed);
        }
        let ranking = self
            .inner
            .store
            .recurring_storage_thieves()
            .map_err(|error| map_storage_thief_error(error.kind))?;
        public_storage_thief_ranking(ranking)
    }

    /// Return one bounded, path-free census of legacy running rows that have
    /// no process claim. This read performs no liveness probe or mutation.
    pub fn running_scan_debt_census(
        &self,
    ) -> Result<RunningScanDebtCensus, RunningScanDebtCensusError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(RunningScanDebtCensusError::Closed);
        }
        let census = self
            .inner
            .store
            .running_scan_debt_census()
            .map_err(|error| map_running_scan_debt_census_error(error.kind))?;
        public_running_scan_debt_census(census)
    }

    /// Prepare one short-lived, consume-once user confirmation for annotating
    /// an exact bounded page of pristine unclaimed scan history. Missing
    /// ownership is not interpreted as death, and no filesystem authority is
    /// included in the preview.
    pub fn prepare_legacy_running_scan_dismissal(
        &self,
    ) -> Result<LegacyRunningScanDismissalPreview, LegacyRunningScanDismissalError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(LegacyRunningScanDismissalError::Closed);
        }
        let prepared_at = SystemTime::now();
        let monotonic_now = Instant::now();
        let prepared = self
            .inner
            .store
            .prepare_legacy_running_scan_dismissal(prepared_at)
            .map_err(map_legacy_running_scan_dismissal_store_error)?;
        LegacyRunningScanDismissalPreview::new(
            &self.inner.store,
            prepared,
            prepared_at,
            monotonic_now,
        )
        .ok_or(LegacyRunningScanDismissalError::InternalState)
    }

    /// Consume one exact preview and annotate only unchanged legacy scan
    /// history. The operation does not remove files or snapshot-temp leases.
    pub fn dismiss_legacy_running_scans(
        &self,
        preview: LegacyRunningScanDismissalPreview,
    ) -> Result<LegacyRunningScanDismissalResult, LegacyRunningScanDismissalError> {
        self.dismiss_legacy_running_scans_at(preview, Instant::now())
    }

    fn dismiss_legacy_running_scans_at(
        &self,
        preview: LegacyRunningScanDismissalPreview,
        now: Instant,
    ) -> Result<LegacyRunningScanDismissalResult, LegacyRunningScanDismissalError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(LegacyRunningScanDismissalError::Closed);
        }
        if !preview.belongs_to(&self.inner.store) {
            return Err(LegacyRunningScanDismissalError::WrongEngine);
        }
        let info = preview.info_at(now)?;
        let prepared = preview.into_prepared(now)?;
        let dismissed = self
            .inner
            .store
            .dismiss_legacy_running_scans(&prepared)
            .map_err(map_legacy_running_scan_dismissal_store_error)?;
        if dismissed != info.eligible_count() {
            return Err(LegacyRunningScanDismissalError::OutcomeUnknown);
        }
        LegacyRunningScanDismissalResult::new(dismissed, info.has_more())
            .ok_or(LegacyRunningScanDismissalError::OutcomeUnknown)
    }

    #[cfg(test)]
    fn dismiss_legacy_running_scans_at_expiry_for_test(
        &self,
        preview: LegacyRunningScanDismissalPreview,
    ) -> Result<LegacyRunningScanDismissalResult, LegacyRunningScanDismissalError> {
        let expires_at = preview.monotonic_expires_at_for_test();
        self.dismiss_legacy_running_scans_at(preview, expires_at)
    }

    /// Return one bounded, path-free census of claimed running rows grouped
    /// only by stored/current host and boot provenance. This read performs no
    /// claimed-process liveness probe and carries no recovery authority.
    pub fn claimed_running_scan_provenance_census(
        &self,
    ) -> Result<ClaimedRunningScanProvenanceCensus, ClaimedRunningScanProvenanceCensusError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(ClaimedRunningScanProvenanceCensusError::Closed);
        }
        let census = self
            .inner
            .store
            .claimed_running_scan_provenance_census()
            .map_err(|error| map_claimed_running_scan_provenance_census_error(error.kind))?;
        public_claimed_running_scan_provenance_census(census)
    }

    /// Return one bounded, path- and identity-free census of active cleanup
    /// journals grouped by phase and stored/current host and boot provenance.
    /// This read performs no owner liveness probe and carries no recovery or
    /// cleanup-effect authority.
    pub fn cleanup_recovery_diagnostic_census(
        &self,
    ) -> Result<CleanupRecoveryDiagnosticCensus, CleanupRecoveryDiagnosticCensusError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(CleanupRecoveryDiagnosticCensusError::Closed);
        }
        let census = self
            .inner
            .store
            .cleanup_recovery_diagnostic_census()
            .map_err(|error| map_cleanup_recovery_diagnostic_census_error(error.kind))?;
        public_cleanup_recovery_diagnostic_census(census)
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

    /// Reserve one canonical scope for a standalone observation scanner.
    ///
    /// This exists for the progressive interactive CLI. The returned handle
    /// must outlive the scanner and be dropped only after its worker has
    /// quiesced. It grants exclusion only and cannot create durable scan,
    /// snapshot, candidate, plan, or cleanup state.
    pub fn acquire_standalone_scan_scope(
        &self,
        root: PathBuf,
    ) -> Result<StandaloneScanScopeLease, StartTaskError> {
        let (canonical_root, _) = prepare_scan_root(&root)?;
        self.acquire_prepared_scan_scope(canonical_root)
    }

    fn acquire_prepared_scan_scope(
        &self,
        canonical_root: PathBuf,
    ) -> Result<StandaloneScanScopeLease, StartTaskError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(StartTaskError::Closed);
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
        let token = self
            .inner
            .store
            .acquire_scan_scope_lease(&canonical_root)
            .map_err(|error| map_scan_scope_lease_error(error.kind))?;
        Ok(StandaloneScanScopeLease {
            canonical_root,
            store: Arc::clone(&self.inner.store),
            token: Some(token),
        })
    }

    /// Admit at most one observation-only scan for a configured project root
    /// during the exact contiguous low-pressure interval proven at
    /// `capacity_anchor`. The caller selects only a stored ordinal; no caller
    /// path, cleanup plan, or effect authority crosses this boundary.
    #[cfg(test)]
    pub fn start_targeted_project_scan(
        &self,
        volume_id: &crate::domain::VolumeId,
        capacity_anchor: SystemTime,
        selected_root_ordinal: u16,
        expected_configured_roots_revision: Option<u64>,
    ) -> Result<TargetedProjectScanAdmission, TargetedProjectScanError> {
        self.start_targeted_project_scan_with_hooks(
            volume_id,
            capacity_anchor,
            selected_root_ordinal,
            expected_configured_roots_revision,
            None,
            |_| {},
            || {},
            || {},
        )
    }

    /// Contract-v39 targeted admission. After the first response, callers must
    /// echo the exact path-free catalog digest so an OS-account, identity,
    /// overlap, exclusion, or settings change cannot silently retarget a later
    /// ordinal.
    pub fn start_targeted_reclaim_scan(
        &self,
        volume_id: &crate::domain::VolumeId,
        capacity_anchor: SystemTime,
        selected_root_ordinal: u16,
        expected_configured_roots_revision: Option<u64>,
        expected_root_catalog_digest_sha256: Option<[u8; 32]>,
    ) -> Result<TargetedProjectScanAdmission, TargetedProjectScanError> {
        self.start_targeted_project_scan_with_hooks(
            volume_id,
            capacity_anchor,
            selected_root_ordinal,
            expected_configured_roots_revision,
            expected_root_catalog_digest_sha256,
            |_| {},
            || {},
            || {},
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn start_targeted_project_scan_with_test_hooks(
        &self,
        volume_id: &crate::domain::VolumeId,
        capacity_anchor: SystemTime,
        selected_root_ordinal: u16,
        expected_configured_roots_revision: Option<u64>,
        before_traversal: impl FnOnce(&ScanId) + Send + 'static,
        before_candidate_evaluation: impl FnOnce() + Send + 'static,
        before_candidate_persistence: impl FnOnce() + Send + 'static,
    ) -> Result<TargetedProjectScanAdmission, TargetedProjectScanError> {
        self.start_targeted_project_scan_with_hooks(
            volume_id,
            capacity_anchor,
            selected_root_ordinal,
            expected_configured_roots_revision,
            None,
            before_traversal,
            before_candidate_evaluation,
            before_candidate_persistence,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn start_targeted_project_scan_with_hooks(
        &self,
        volume_id: &crate::domain::VolumeId,
        capacity_anchor: SystemTime,
        selected_root_ordinal: u16,
        expected_configured_roots_revision: Option<u64>,
        expected_root_catalog_digest_sha256: Option<[u8; 32]>,
        before_traversal: impl FnOnce(&ScanId) + Send + 'static,
        before_candidate_evaluation: impl FnOnce() + Send + 'static,
        before_candidate_persistence: impl FnOnce() + Send + 'static,
    ) -> Result<TargetedProjectScanAdmission, TargetedProjectScanError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(TargetedProjectScanError::Closed);
        }
        let status = self
            .inner
            .store
            .status()
            .map_err(|_| TargetedProjectScanError::Unavailable)?;
        if !matches!(
            status.access,
            crate::persistence::DatabaseAccess::ReadWriteCurrent
        ) {
            return Err(TargetedProjectScanError::ReadOnlyStore);
        }

        let configured = self
            .inner
            .store
            .load_configured_project_roots()
            .map_err(|error| map_targeted_project_scan_history_error(error.kind))?;
        if let Some(expected) = expected_configured_roots_revision
            && expected != configured.revision
        {
            return Err(TargetedProjectScanError::RegistryChanged {
                expected_revision: expected,
                actual_revision: configured.revision,
            });
        }
        let catalog = build_targeted_reclaim_catalog(&self.inner.config, &configured)?;
        if expected_root_catalog_digest_sha256
            .is_some_and(|expected| expected != catalog.stamp.digest_sha256)
        {
            return Err(TargetedProjectScanError::CatalogChanged);
        }
        let root_count = catalog.stamp.root_count;
        let root_catalog = catalog.stamp.clone();
        self.ensure_no_active_targeted_catalog_conflict(&root_catalog)?;

        let episodes = self
            .inner
            .store
            .load_pressure_episode_page_at_anchor(
                volume_id,
                capacity_anchor,
                MAX_TARGETED_PRESSURE_CHAIN_EPISODES + 1,
            )
            .map_err(|error| map_targeted_project_scan_history_error(error.kind))?;
        // Targeted admission is only meaningful for the latest accepted
        // startup-volume observation. Historical anchors remain valid for
        // charts, but cannot trigger new background filesystem work.
        let volume_mount_path = self
            .inner
            .store
            .load_volume_mount_path_at_anchor(volume_id, capacity_anchor)
            .map_err(|error| map_targeted_project_scan_history_error(error.kind))?;
        let chain = derive_low_pressure_chain(&episodes, capacity_anchor)
            .map_err(map_targeted_project_scan_history_error)?;
        let pressure = chain.map(|chain| TargetedProjectScanPressureContext {
            volume_id: volume_id.clone(),
            capacity_anchor,
            pressure: chain.pressure,
            current_episode_started_at: chain.current_episode_started_at,
            pressure_started_at: chain.started_at,
            policy_revision: chain.policy_revision,
        });

        if catalog.roots.is_empty() {
            return Ok(TargetedProjectScanAdmission {
                configured_roots_revision: configured.revision,
                root_count,
                root_catalog,
                selection: None,
                pressure,
                disposition: TargetedProjectScanDisposition::EmptyRegistry,
            });
        }
        let Some(selected) = catalog.roots.get(usize::from(selected_root_ordinal)) else {
            return Err(TargetedProjectScanError::InvalidOrdinal {
                ordinal: selected_root_ordinal,
                root_count,
            });
        };
        if selected_root_ordinal > 0 && expected_root_catalog_digest_sha256.is_none() {
            return Err(TargetedProjectScanError::InvalidCatalog);
        }
        let selection = TargetedProjectScanSelection {
            ordinal: selected_root_ordinal,
            kind: selected.kind,
            root: selected.display_root.clone(),
            max_nodes: selected.max_nodes,
        };
        let selected_kind = selected.kind;
        let selected_root = selected.display_root.clone();
        let prepared_root = selected.prepared.clone();
        let excluded_subtrees = selected.excluded_subtrees.clone();
        let targeted_max_nodes = usize::try_from(selection.max_nodes)
            .map_err(|_| TargetedProjectScanError::InternalState)?;
        let Some(pressure) = pressure else {
            return Ok(TargetedProjectScanAdmission {
                configured_roots_revision: configured.revision,
                root_count,
                root_catalog,
                selection: Some(selection),
                pressure: None,
                disposition: TargetedProjectScanDisposition::NoPressure,
            });
        };
        let (canonical_root, expected_identity) = match prepared_root {
            Ok(root) => root,
            Err(reason) => {
                self.ensure_targeted_reclaim_catalog_unchanged(&configured, &root_catalog)?;
                return Ok(TargetedProjectScanAdmission {
                    configured_roots_revision: configured.revision,
                    root_count,
                    root_catalog,
                    selection: Some(selection),
                    pressure: Some(pressure),
                    disposition: TargetedProjectScanDisposition::RootUnavailable { reason },
                });
            }
        };
        let trusted_home_mount = if selected_kind == TargetedReclaimRootKind::KnownUserLibraryCaches
        {
            let known = selected
                .known_user_cache
                .as_ref()
                .ok_or(TargetedProjectScanError::InternalState)?;
            let lexical = validate_scan_root(&selected_root)
                .map_err(|_| TargetedProjectScanError::CatalogChanged)?;
            let captured =
                capture_scan_root(lexical).map_err(|_| TargetedProjectScanError::CatalogChanged)?;
            if captured.canonical_path() != canonical_root
                || captured.identity() != expected_identity
            {
                return Err(TargetedProjectScanError::CatalogChanged);
            }
            Some(
                known
                    .capture_mount_witness(&captured)
                    .map_err(|_| TargetedProjectScanError::CatalogChanged)?,
            )
        } else {
            None
        };
        if let Err(reason) = prove_root_on_affected_volume(&canonical_root, &volume_mount_path) {
            self.ensure_targeted_reclaim_catalog_unchanged(&configured, &root_catalog)?;
            return Ok(TargetedProjectScanAdmission {
                configured_roots_revision: configured.revision,
                root_count,
                root_catalog,
                selection: Some(selection),
                pressure: Some(pressure),
                disposition: TargetedProjectScanDisposition::RootUnavailable { reason },
            });
        }

        // Close the settings/read-to-filesystem gap before using the selected
        // root. A concurrent registry edit never silently retargets admission.
        self.ensure_targeted_reclaim_catalog_unchanged(&configured, &root_catalog)?;

        let reusable_record = self
            .inner
            .store
            .load_latest_scan_for_exact_root_since(
                &canonical_root,
                pressure.current_episode_started_at,
            )
            .map_err(|error| map_targeted_project_scan_history_error(error.kind))?
            .filter(|record| targeted_scan_record_matches_kind(record.id(), selected_kind))
            .filter(|record| {
                targeted_scan_snapshot_matches_root(
                    &self.inner.snapshots,
                    record,
                    &canonical_root,
                    expected_identity,
                )
            });
        if let Some(record) = reusable_record {
            let observation = self
                .inner
                .store
                .load_candidate_evaluation_for_scan(record.id())
                .map_err(|error| map_targeted_project_scan_history_error(error.kind))?;
            let is_current = match &observation {
                CandidateEvaluationObservation::Succeeded(evaluation)
                | CandidateEvaluationObservation::Failed(evaluation) => {
                    evaluation.matches_current_scan_observation(&record)
                }
                CandidateEvaluationObservation::MissingScan
                | CandidateEvaluationObservation::NotRun { .. }
                | CandidateEvaluationObservation::Pending(_) => false,
            };
            if is_current {
                let scan = public_scan_summary(&record);
                let candidate_evaluation = public_candidate_history(record.id(), observation)
                    .map_err(map_targeted_candidate_history_error)?;
                if !matches!(
                    candidate_evaluation.status(),
                    DurableCandidateEvaluationStatus::Succeeded { .. }
                        | DurableCandidateEvaluationStatus::Failed { .. }
                ) {
                    return Err(TargetedProjectScanError::CorruptData);
                }
                return Ok(TargetedProjectScanAdmission {
                    configured_roots_revision: configured.revision,
                    root_count,
                    root_catalog,
                    selection: Some(selection),
                    pressure: Some(pressure),
                    disposition: TargetedProjectScanDisposition::Current(Box::new(
                        TargetedProjectScanCurrent {
                            scan,
                            candidate_evaluation,
                        },
                    )),
                });
            }
        }

        self.ensure_targeted_reclaim_catalog_unchanged(&configured, &root_catalog)?;
        let store = Arc::clone(&self.inner.store);
        let snapshots = Arc::clone(&self.inner.snapshots);
        let work_root = canonical_root.clone();
        let targeted_admission = TargetedScanAdmissionIdentity {
            root_kind: selected_kind,
            root_ordinal: selected_root_ordinal,
            root_identity: expected_identity,
            root_catalog_digest_sha256: root_catalog.digest_sha256,
            max_nodes: selection.max_nodes,
            excluded_subtrees: excluded_subtrees.clone(),
            pressure: pressure.clone(),
        };
        let work = Box::new(move |context| {
            run_scan_task(
                context,
                AdmittedScanRoot {
                    path: work_root,
                    expected_identity: Some(expected_identity),
                    origin: ScanTaskOrigin::TargetedRecommendation,
                    max_nodes: targeted_max_nodes,
                    excluded_subtrees,
                    trusted_home_mount,
                    targeted_root_kind: Some(selected_kind),
                },
                store,
                snapshots,
                before_traversal,
                before_candidate_evaluation,
                before_candidate_persistence,
            )
        });
        let disposition =
            match self.submit_targeted_scan(canonical_root, targeted_admission, work)? {
                TargetedScanSubmission::Started(task_id) => {
                    TargetedProjectScanDisposition::Started { task_id }
                }
                TargetedScanSubmission::Existing { task_id, phase } => {
                    TargetedProjectScanDisposition::ExistingTask { task_id, phase }
                }
            };
        Ok(TargetedProjectScanAdmission {
            configured_roots_revision: configured.revision,
            root_count,
            root_catalog,
            selection: Some(selection),
            pressure: Some(pressure),
            disposition,
        })
    }

    fn ensure_no_active_targeted_catalog_conflict(
        &self,
        requested: &TargetedReclaimRootCatalogStamp,
    ) -> Result<(), TargetedProjectScanError> {
        let registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| TargetedProjectScanError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            return Err(TargetedProjectScanError::Closed);
        }
        for task_id in registry.active_scan_roots.values() {
            let record = registry
                .records
                .get(task_id)
                .ok_or(TargetedProjectScanError::InternalState)?;
            if record.scan_origin == Some(ScanTaskOrigin::TargetedRecommendation)
                && record.targeted_admission.as_ref().is_some_and(|admission| {
                    admission.root_catalog_digest_sha256 != requested.digest_sha256
                })
            {
                return Err(TargetedProjectScanError::Busy);
            }
        }
        Ok(())
    }

    /// Revalidate the path-free registry and pressure facts after a bounded
    /// selected-root pass. This starts no task and grants no cleanup authority.
    #[cfg(test)]
    pub fn validate_targeted_project_scan_context(
        &self,
        expected_pressure: &TargetedProjectScanPressureContext,
        expected_configured_roots_revision: u64,
        expected_root_count: u16,
    ) -> Result<TargetedProjectScanCheckpoint, TargetedProjectScanError> {
        self.validate_targeted_reclaim_scan_context_inner(
            expected_pressure,
            expected_configured_roots_revision,
            expected_root_count,
            None,
        )
    }

    pub fn validate_targeted_reclaim_scan_context(
        &self,
        expected_pressure: &TargetedProjectScanPressureContext,
        expected_root_catalog: &TargetedReclaimRootCatalogStamp,
    ) -> Result<TargetedProjectScanCheckpoint, TargetedProjectScanError> {
        self.validate_targeted_reclaim_scan_context_inner(
            expected_pressure,
            expected_root_catalog.configured_roots_revision,
            expected_root_catalog.root_count,
            Some(expected_root_catalog),
        )
    }

    /// Atomically finalize a path-free Critical-pressure recovery ordering.
    ///
    /// The caller can only echo the exact pressure and catalog observations
    /// returned by targeted admission. Rust privately reselects every current
    /// exact-root scan, validates its snapshot and terminal evaluator
    /// observation, builds a bounded display projection, and revalidates the
    /// proof before returning. This starts no work and grants no plan, path,
    /// approval, or effect authority.
    pub fn finalize_emergency_recovery(
        &self,
        expected_pressure: &TargetedProjectScanPressureContext,
        expected_root_catalog: &TargetedReclaimRootCatalogStamp,
    ) -> Result<EmergencyRecoveryOrdering, EmergencyRecoveryError> {
        let checkpoint = self
            .validate_targeted_reclaim_scan_context(expected_pressure, expected_root_catalog)
            .map_err(map_targeted_project_scan_to_emergency_recovery_error)?;
        if checkpoint.pressure.pressure != TargetedProjectScanPressure::Critical {
            return Err(EmergencyRecoveryError::NotCritical);
        }

        let configured = self
            .inner
            .store
            .load_configured_project_roots()
            .map_err(|error| map_emergency_recovery_history_error(error.kind))?;
        let catalog = build_targeted_reclaim_catalog(&self.inner.config, &configured)
            .map_err(map_targeted_project_scan_to_emergency_recovery_error)?;
        if catalog.stamp != checkpoint.root_catalog {
            return Err(EmergencyRecoveryError::CatalogChanged);
        }

        let mut observations = Vec::new();
        let mut candidate_evaluated_root_count = 0_u16;
        observations
            .try_reserve_exact(catalog.roots.len())
            .map_err(|_| EmergencyRecoveryError::BudgetExceeded)?;
        for root in &catalog.roots {
            let Ok((canonical_root, expected_identity)) = &root.prepared else {
                continue;
            };
            let Some(record) = self
                .inner
                .store
                .load_latest_scan_for_exact_root_since(
                    canonical_root,
                    checkpoint.pressure.current_episode_started_at,
                )
                .map_err(|error| map_emergency_recovery_history_error(error.kind))?
                .filter(|record| targeted_scan_record_matches_kind(record.id(), root.kind))
                .filter(|record| {
                    targeted_scan_snapshot_matches_root(
                        &self.inner.snapshots,
                        record,
                        canonical_root,
                        *expected_identity,
                    )
                })
            else {
                continue;
            };
            let Some(completed_at) = record.completed_at() else {
                return Err(EmergencyRecoveryError::CorruptData);
            };
            if !emergency_recovery_evidence_is_fresh(
                completed_at,
                checkpoint.pressure.capacity_anchor,
            ) {
                continue;
            }
            let candidate_observation = self
                .inner
                .store
                .load_candidate_evaluation_for_scan(record.id())
                .map_err(|error| map_emergency_recovery_history_error(error.kind))?;
            let evaluation = match candidate_observation {
                CandidateEvaluationObservation::Succeeded(evaluation)
                | CandidateEvaluationObservation::Failed(evaluation)
                    if evaluation.matches_current_scan_observation(&record) =>
                {
                    evaluation
                }
                CandidateEvaluationObservation::MissingScan
                | CandidateEvaluationObservation::NotRun { .. }
                | CandidateEvaluationObservation::Pending(_)
                | CandidateEvaluationObservation::Succeeded(_)
                | CandidateEvaluationObservation::Failed(_) => continue,
            };
            let candidate_evaluated = matches!(
                evaluation.status(),
                CandidateEvaluationStatus::Succeeded { .. }
            );
            let candidates = if candidate_evaluated {
                candidate_evaluated_root_count = candidate_evaluated_root_count
                    .checked_add(1)
                    .ok_or(EmergencyRecoveryError::BudgetExceeded)?;
                evaluation
                    .candidates()
                    .iter()
                    .map(|candidate| {
                        public_candidate_summary(record.id(), candidate)
                            .map_err(map_candidate_history_to_emergency_recovery_error)
                    })
                    .collect::<Result<Vec<_>, EmergencyRecoveryError>>()?
            } else {
                Vec::new()
            };
            let permission_issue_count = record
                .coverage()
                .issues()
                .iter()
                .filter(|issue| issue.kind() == ScanIssueKind::PermissionDenied)
                .try_fold(0_u64, |total, issue| {
                    total
                        .checked_add(u64::from(issue.occurrence_count()))
                        .ok_or(EmergencyRecoveryError::BudgetExceeded)
                })?;
            observations.push(EmergencyRecoveryScanObservation {
                root_ordinal: root.ordinal,
                scan_id: record.id().clone(),
                observed_at: completed_at,
                permission_issue_count,
                candidates,
            });
        }

        let observed_root_count = u16::try_from(observations.len())
            .map_err(|_| EmergencyRecoveryError::BudgetExceeded)?;
        if observed_root_count > checkpoint.root_count
            || candidate_evaluated_root_count > observed_root_count
        {
            return Err(EmergencyRecoveryError::CorruptData);
        }
        let unavailable_root_count = checkpoint
            .root_count
            .checked_sub(observed_root_count)
            .ok_or(EmergencyRecoveryError::CorruptData)?;
        let groups = build_emergency_recovery_groups(observations, unavailable_root_count)?;
        let final_checkpoint = self
            .validate_targeted_reclaim_scan_context(expected_pressure, expected_root_catalog)
            .map_err(map_targeted_project_scan_to_emergency_recovery_error)?;
        if final_checkpoint != checkpoint {
            return Err(EmergencyRecoveryError::PressureChanged);
        }
        Ok(EmergencyRecoveryOrdering {
            policy_revision: EMERGENCY_RECOVERY_POLICY_REVISION,
            pressure: final_checkpoint.pressure,
            root_catalog: final_checkpoint.root_catalog,
            observed_root_count,
            candidate_evaluated_root_count,
            unavailable_root_count,
            groups,
        })
    }

    fn validate_targeted_reclaim_scan_context_inner(
        &self,
        expected_pressure: &TargetedProjectScanPressureContext,
        expected_configured_roots_revision: u64,
        expected_root_count: u16,
        expected_root_catalog: Option<&TargetedReclaimRootCatalogStamp>,
    ) -> Result<TargetedProjectScanCheckpoint, TargetedProjectScanError> {
        if self.lifecycle() != EngineLifecycle::Open {
            return Err(TargetedProjectScanError::Closed);
        }
        let status = self
            .inner
            .store
            .status()
            .map_err(|_| TargetedProjectScanError::Unavailable)?;
        if !matches!(
            status.access,
            crate::persistence::DatabaseAccess::ReadWriteCurrent
        ) {
            return Err(TargetedProjectScanError::ReadOnlyStore);
        }
        let configured = self
            .inner
            .store
            .load_configured_project_roots()
            .map_err(|error| map_targeted_project_scan_history_error(error.kind))?;
        if configured.revision != expected_configured_roots_revision {
            return Err(TargetedProjectScanError::RegistryChanged {
                expected_revision: expected_configured_roots_revision,
                actual_revision: configured.revision,
            });
        }
        let catalog = build_targeted_reclaim_catalog(&self.inner.config, &configured)?;
        let root_count = catalog.stamp.root_count;
        if root_count != expected_root_count {
            return Err(TargetedProjectScanError::CatalogChanged);
        }
        if expected_root_catalog.is_some_and(|expected| *expected != catalog.stamp) {
            return Err(TargetedProjectScanError::CatalogChanged);
        }
        let episodes = self
            .inner
            .store
            .load_pressure_episode_page_at_anchor(
                &expected_pressure.volume_id,
                expected_pressure.capacity_anchor,
                MAX_TARGETED_PRESSURE_CHAIN_EPISODES + 1,
            )
            .map_err(|error| map_targeted_project_scan_history_error(error.kind))?;
        let _ = self
            .inner
            .store
            .load_volume_mount_path_at_anchor(
                &expected_pressure.volume_id,
                expected_pressure.capacity_anchor,
            )
            .map_err(|error| map_targeted_project_scan_history_error(error.kind))?;
        let chain = derive_low_pressure_chain(&episodes, expected_pressure.capacity_anchor)
            .map_err(map_targeted_project_scan_history_error)?
            .ok_or(TargetedProjectScanError::PressureChanged)?;
        let current = TargetedProjectScanPressureContext {
            volume_id: expected_pressure.volume_id.clone(),
            capacity_anchor: expected_pressure.capacity_anchor,
            pressure: chain.pressure,
            current_episode_started_at: chain.current_episode_started_at,
            pressure_started_at: chain.started_at,
            policy_revision: chain.policy_revision,
        };
        if &current != expected_pressure {
            return Err(TargetedProjectScanError::PressureChanged);
        }
        Ok(TargetedProjectScanCheckpoint {
            configured_roots_revision: configured.revision,
            root_count,
            root_catalog: catalog.stamp,
            pressure: current,
        })
    }

    fn ensure_configured_roots_unchanged(
        &self,
        expected: &ConfiguredProjectRootSetting,
    ) -> Result<(), TargetedProjectScanError> {
        let current = self
            .inner
            .store
            .load_configured_project_roots()
            .map_err(|error| map_targeted_project_scan_history_error(error.kind))?;
        if current == *expected {
            Ok(())
        } else {
            Err(TargetedProjectScanError::RegistryChanged {
                expected_revision: expected.revision,
                actual_revision: current.revision,
            })
        }
    }

    fn ensure_targeted_reclaim_catalog_unchanged(
        &self,
        expected_configured: &ConfiguredProjectRootSetting,
        expected_catalog: &TargetedReclaimRootCatalogStamp,
    ) -> Result<(), TargetedProjectScanError> {
        self.ensure_configured_roots_unchanged(expected_configured)?;
        let current = build_targeted_reclaim_catalog(&self.inner.config, expected_configured)?;
        if current.stamp == *expected_catalog {
            Ok(())
        } else {
            Err(TargetedProjectScanError::CatalogChanged)
        }
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
        self.submit_scan(
            ScanTaskOrigin::UserSubtree,
            root.clone(),
            Box::new(move |context| {
                run_scan_task(
                    context,
                    AdmittedScanRoot {
                        path: root,
                        expected_identity: Some(expected_identity),
                        origin: ScanTaskOrigin::UserSubtree,
                        max_nodes: MAX_HOME_SCAN_NODES,
                        excluded_subtrees: Vec::new(),
                        trusted_home_mount: None,
                        targeted_root_kind: None,
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
        let (canonical_root, expected_identity) = prepare_scan_root(&root)?;
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
        self.submit_scan(
            ScanTaskOrigin::UserFull,
            canonical_root.clone(),
            Box::new(move |context| {
                run_scan_task(
                    context,
                    AdmittedScanRoot {
                        path: canonical_root,
                        expected_identity: Some(expected_identity),
                        origin: ScanTaskOrigin::UserFull,
                        max_nodes: MAX_HOME_SCAN_NODES,
                        excluded_subtrees: Vec::new(),
                        trusted_home_mount: None,
                        targeted_root_kind: None,
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
                | TaskResult::RustTargetDryRun(_)
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
                | TaskResult::RustTargetDryRun(_)
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
                | TaskResult::RustTargetDryRun(_)
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
                | TaskResult::RustTargetDryRun(_)
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
                | TaskResult::RustTargetDryRun(_)
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
                | TaskResult::RustTargetDryRun(_)
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
                | TaskResult::RustTargetDryRun(_)
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
                | TaskResult::RustTargetDryRun(_)
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
                | TaskResult::RustTargetDryRun(_)
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
                | TaskResult::RustTargetDryRun(_)
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
                | TaskResult::SnapshotUnleasedTempMaintenance(_)
                | TaskResult::RustTargetDryRun(_),
            ) => return Err(TaskAccessError::WrongTaskKind),
            #[cfg(test)]
            Some(TaskResult::TestOnly) => return Err(TaskAccessError::WrongTaskKind),
            None => None,
        })
    }

    pub fn rust_target_dry_run_result(
        &self,
        id: TaskId,
    ) -> Result<Option<Arc<RustTargetDryRunResult>>, TaskAccessError> {
        let registry = self.lock_open_registry()?;
        let record = registry
            .records
            .get(&id)
            .ok_or(TaskAccessError::UnknownTask)?;
        if record.kind != TaskKind::RustTargetDryRun {
            return Err(TaskAccessError::WrongTaskKind);
        }
        Ok(match &record.result {
            Some(TaskResult::RustTargetDryRun(result)) => Some(Arc::clone(result)),
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
                | TaskResult::SnapshotUnleasedTempMaintenance(_)
                | TaskResult::PermanentSafeCleanup(_),
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
        if registry
            .records
            .get(&id)
            .is_some_and(|record| record.cancellation_closed)
        {
            return Ok(CancelOutcome::AlreadyTerminal);
        }

        let event_limit = self.inner.shared.limits.events_per_task;
        if phase == TaskPhase::Queued {
            let position = registry.queue.iter().position(|job| job.id == id);
            if let Some(position) = position {
                let job = registry
                    .queue
                    .remove(position)
                    .ok_or(TaskAccessError::InternalState)?;
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
                // A queued scan closure owns its cross-process scope lease.
                // Release it only after the task-registry mutex is gone.
                drop(registry);
                drop(job);
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

    /// Atomically claim this engine's terminal lifecycle for a future
    /// whole-app-data reset.
    ///
    /// This creates no durable journal intent and performs no filesystem
    /// effect. Only the winning caller receives the non-cloneable shutdown
    /// capability.
    pub fn begin_app_data_reset_shutdown(&self) -> AppDataResetAdmissionOutcome {
        self.app_data_reset_outcome(
            self.inner
                .shared
                .request_terminal(TerminalIntent::AppDataReset),
        )
    }

    pub(crate) fn begin_app_data_reset_shutdown_until(
        &self,
        deadline: Instant,
    ) -> Result<AppDataResetAdmissionOutcome, ()> {
        self.inner
            .shared
            .request_terminal_until(TerminalIntent::AppDataReset, deadline)
            .map(|outcome| self.app_data_reset_outcome(outcome))
    }

    fn app_data_reset_outcome(
        &self,
        outcome: TerminalRequestOutcome,
    ) -> AppDataResetAdmissionOutcome {
        match outcome {
            TerminalRequestOutcome::Initiated => {
                AppDataResetAdmissionOutcome::Admitted(AppDataResetShutdown {
                    inner: Arc::clone(&self.inner),
                })
            }
            TerminalRequestOutcome::AlreadyClosing(Some(TerminalIntent::OrdinaryClose))
            | TerminalRequestOutcome::AlreadyClosed(Some(TerminalIntent::OrdinaryClose)) => {
                AppDataResetAdmissionOutcome::OrdinaryCloseWon
            }
            TerminalRequestOutcome::AlreadyClosing(Some(TerminalIntent::AppDataReset))
            | TerminalRequestOutcome::AlreadyClosed(Some(TerminalIntent::AppDataReset)) => {
                AppDataResetAdmissionOutcome::AlreadyResetting
            }
            TerminalRequestOutcome::AlreadyClosing(None)
            | TerminalRequestOutcome::AlreadyClosed(None) => {
                AppDataResetAdmissionOutcome::InternalState
            }
        }
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
        if closed {
            self.inner.join_workers();
        }
        closed
    }

    fn submit(
        &self,
        kind: TaskKind,
        scan_scope: Option<PathBuf>,
        work: Work,
    ) -> Result<TaskId, StartTaskError> {
        debug_assert!(kind != TaskKind::Scan);
        debug_assert!(scan_scope.is_none());
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
        let record = TaskRecord::new(
            id,
            kind,
            scan_scope.clone(),
            self.inner.shared.limits.events_per_task,
        );
        let priority = record.priority;
        if let Some(scope) = scan_scope {
            registry.active_scan_roots.insert(scope, id);
        }
        registry.records.insert(id, record);
        registry.enqueue(Job { id, priority, work });
        self.inner.shared.workers_ready.notify_one();
        Ok(id)
    }

    fn submit_scan(
        &self,
        origin: ScanTaskOrigin,
        scan_scope: PathBuf,
        work: Work,
    ) -> Result<TaskId, StartTaskError> {
        debug_assert!(matches!(
            origin,
            ScanTaskOrigin::UserFull | ScanTaskOrigin::UserSubtree
        ));
        let _admission = self
            .inner
            .scan_admission
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;

        let (id, preempted_jobs) = {
            let mut registry = self
                .inner
                .shared
                .registry
                .lock()
                .map_err(|_| StartTaskError::InternalState)?;
            if registry.lifecycle != EngineLifecycle::Open {
                return Err(StartTaskError::Closed);
            }
            let overlapping = registry
                .active_scan_roots
                .iter()
                .filter(|(active, _)| super::config::paths_overlap(active, &scan_scope))
                .map(|(active, id)| (active.clone(), *id))
                .collect::<Vec<_>>();
            let mut preempt = Vec::new();
            for (active, id) in &overlapping {
                let Some(record) = registry.records.get(id) else {
                    return Err(StartTaskError::InternalState);
                };
                if record.phase == TaskPhase::Queued
                    && record.scan_origin == Some(ScanTaskOrigin::TargetedRecommendation)
                    && registry.queue.iter().any(|job| job.id == *id)
                {
                    preempt.push(*id);
                    continue;
                }
                return if active == &scan_scope {
                    Err(StartTaskError::ScanAlreadyActive { existing: *id })
                } else {
                    Err(StartTaskError::ScanScopeBusy)
                };
            }
            let id = TASK_IDS.allocate()?;
            let mut preempted_jobs = Vec::with_capacity(preempt.len());
            for id in preempt {
                let job = registry
                    .cancel_queued_task(
                        id,
                        self.inner.shared.limits.events_per_task,
                        self.inner.shared.limits.retained_terminal_tasks,
                    )
                    .ok_or(StartTaskError::InternalState)?;
                preempted_jobs.push(job);
            }
            if registry.queue.len() >= self.inner.shared.limits.queued_tasks {
                drop(registry);
                drop(preempted_jobs);
                return Err(StartTaskError::QueueFull);
            }
            (id, preempted_jobs)
        };

        // Dropping a preempted targeted closure releases its exact durable
        // lease and may enter persistence; never do that under the registry.
        drop(preempted_jobs);
        let lease = self.acquire_prepared_scan_scope(scan_scope.clone())?;

        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| StartTaskError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            drop(registry);
            drop(lease);
            return Err(StartTaskError::Closed);
        }
        if registry.queue.len() >= self.inner.shared.limits.queued_tasks {
            drop(registry);
            drop(lease);
            return Err(StartTaskError::QueueFull);
        }
        let record = TaskRecord::new_scan(
            id,
            origin,
            scan_scope.clone(),
            self.inner.shared.limits.events_per_task,
        );
        let priority = record.priority;
        registry.active_scan_roots.insert(scan_scope, id);
        registry.records.insert(id, record);
        registry.enqueue(Job {
            id,
            priority,
            work: lease.into_leased_work(work),
        });
        self.inner.shared.workers_ready.notify_one();
        Ok(id)
    }

    fn submit_targeted_scan(
        &self,
        scan_scope: PathBuf,
        admission: TargetedScanAdmissionIdentity,
        work: Work,
    ) -> Result<TargetedScanSubmission, TargetedProjectScanError> {
        let _admission = self
            .inner
            .scan_admission
            .lock()
            .map_err(|_| TargetedProjectScanError::InternalState)?;
        let id = {
            let registry = self
                .inner
                .shared
                .registry
                .lock()
                .map_err(|_| TargetedProjectScanError::InternalState)?;
            if registry.lifecycle != EngineLifecycle::Open {
                return Err(TargetedProjectScanError::Closed);
            }
            let existing = registry
                .active_scan_roots
                .iter()
                .filter(|(active, _)| super::config::paths_overlap(active, &scan_scope))
                .map(|(active, id)| (active.clone(), *id))
                .min_by_key(|(_, id)| *id);
            if let Some((active_scope, task_id)) = existing {
                let record = registry
                    .records
                    .get(&task_id)
                    .ok_or(TargetedProjectScanError::InternalState)?;
                if active_scope != scan_scope
                    || record.scan_origin != Some(ScanTaskOrigin::TargetedRecommendation)
                    || record.targeted_root_kind != Some(admission.root_kind)
                    || record.targeted_admission.as_ref() != Some(&admission)
                {
                    return Err(TargetedProjectScanError::Busy);
                }
                return Ok(TargetedScanSubmission::Existing {
                    task_id,
                    phase: record.phase,
                });
            }
            if registry.queue.len() >= self.inner.shared.limits.queued_tasks {
                return Err(TargetedProjectScanError::QueueFull);
            }
            TASK_IDS.allocate().map_err(|error| match error {
                StartTaskError::TaskIdExhausted => TargetedProjectScanError::TaskIdExhausted,
                _ => TargetedProjectScanError::InternalState,
            })?
        };

        let lease = self
            .acquire_prepared_scan_scope(scan_scope.clone())
            .map_err(|error| match error {
                StartTaskError::Closed => TargetedProjectScanError::Closed,
                StartTaskError::QueueFull => TargetedProjectScanError::QueueFull,
                StartTaskError::ReadOnlyStore => TargetedProjectScanError::ReadOnlyStore,
                StartTaskError::ScanScopeBusy | StartTaskError::ScanAlreadyActive { .. } => {
                    TargetedProjectScanError::Busy
                }
                StartTaskError::TaskIdExhausted => TargetedProjectScanError::TaskIdExhausted,
                StartTaskError::InvalidScanRoot { .. } => TargetedProjectScanError::CorruptData,
                StartTaskError::PersistenceUnavailable => TargetedProjectScanError::Unavailable,
                StartTaskError::InputTooLarge { .. } | StartTaskError::InternalState => {
                    TargetedProjectScanError::InternalState
                }
            })?;

        let mut registry = self
            .inner
            .shared
            .registry
            .lock()
            .map_err(|_| TargetedProjectScanError::InternalState)?;
        if registry.lifecycle != EngineLifecycle::Open {
            drop(registry);
            drop(lease);
            return Err(TargetedProjectScanError::Closed);
        }
        if registry.queue.len() >= self.inner.shared.limits.queued_tasks {
            drop(registry);
            drop(lease);
            return Err(TargetedProjectScanError::QueueFull);
        }
        let mut record = TaskRecord::new_scan(
            id,
            ScanTaskOrigin::TargetedRecommendation,
            scan_scope.clone(),
            self.inner.shared.limits.events_per_task,
        );
        record.targeted_root_kind = Some(admission.root_kind);
        record.targeted_admission = Some(admission);
        registry.active_scan_roots.insert(scan_scope, id);
        registry.records.insert(id, record);
        registry.enqueue(Job {
            id,
            priority: TaskPriority::Targeted,
            work: lease.into_leased_work(work),
        });
        self.inner.shared.workers_ready.notify_one();
        Ok(TargetedScanSubmission::Started(id))
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
        registry.enqueue(Job {
            id,
            priority: TaskPriority::Maintenance,
            work,
        });
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
        registry.enqueue(Job {
            id,
            priority: TaskPriority::Maintenance,
            work,
        });
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
        registry.enqueue(Job {
            id,
            priority: TaskPriority::Maintenance,
            work,
        });
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
        registry.enqueue(Job {
            id,
            priority: TaskPriority::Maintenance,
            work,
        });
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
        registry.enqueue(Job {
            id,
            priority: TaskPriority::Maintenance,
            work,
        });
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
        registry.enqueue(Job {
            id,
            priority: TaskPriority::Maintenance,
            work,
        });
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
        registry.enqueue(Job {
            id,
            priority: TaskPriority::Maintenance,
            work,
        });
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
        registry.enqueue(Job {
            id,
            priority: TaskPriority::Maintenance,
            work,
        });
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

fn map_scan_scope_lease_error(kind: ScanScopeLeaseErrorKind) -> StartTaskError {
    match kind {
        ScanScopeLeaseErrorKind::Busy => StartTaskError::ScanScopeBusy,
        ScanScopeLeaseErrorKind::IncompatibleSchema => StartTaskError::ReadOnlyStore,
        ScanScopeLeaseErrorKind::InvalidRoot => StartTaskError::InvalidScanRoot {
            reason: ScanRootErrorKind::InvalidPath,
        },
        ScanScopeLeaseErrorKind::QueryLimitExceeded
        | ScanScopeLeaseErrorKind::UnsafeStorage
        | ScanScopeLeaseErrorKind::CorruptData
        | ScanScopeLeaseErrorKind::Unavailable
        | ScanScopeLeaseErrorKind::OutcomeUnknown => StartTaskError::PersistenceUnavailable,
        ScanScopeLeaseErrorKind::InternalState => StartTaskError::InternalState,
    }
}

fn prepare_scan_root(root: &Path) -> Result<(PathBuf, FilesystemIdentity), StartTaskError> {
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
    let lexical = validate_scan_root(&canonical).map_err(|_| StartTaskError::InvalidScanRoot {
        reason: ScanRootErrorKind::InvalidPath,
    })?;
    let captured = capture_scan_root(lexical).map_err(|error| StartTaskError::InvalidScanRoot {
        reason: map_targeted_root_capture_error(error),
    })?;
    Ok((captured.canonical_path().to_path_buf(), captured.identity()))
}

fn prepare_targeted_scan_root(
    root: &Path,
) -> Result<(PathBuf, FilesystemIdentity), ScanRootErrorKind> {
    let normalized_root = normalize_macos_system_path_alias(root);
    let metadata =
        std::fs::symlink_metadata(&normalized_root).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ScanRootErrorKind::Missing,
            std::io::ErrorKind::PermissionDenied => ScanRootErrorKind::AccessDenied,
            _ => ScanRootErrorKind::Unavailable,
        })?;
    if metadata.file_type().is_symlink() {
        return Err(ScanRootErrorKind::Symlink);
    }
    if !metadata.is_dir() {
        return Err(ScanRootErrorKind::NotDirectory);
    }
    let lexical =
        validate_scan_root(&normalized_root).map_err(|_| ScanRootErrorKind::InvalidPath)?;
    let canonical = capture_scan_root(lexical).map_err(map_targeted_root_capture_error)?;
    Ok((
        canonical.canonical_path().to_path_buf(),
        canonical.identity(),
    ))
}

fn build_targeted_reclaim_catalog(
    config: &EngineConfig,
    configured: &ConfiguredProjectRootSetting,
) -> Result<TargetedReclaimCatalog, TargetedProjectScanError> {
    let known_user_cache = KnownUserLibraryCachesPath::capture()
        .ok()
        .filter(|known| config.cache_directory() == known.path().join("Dux"));
    let known_path = known_user_cache
        .as_ref()
        .map(|known| known.path().to_path_buf());

    let mut roots = Vec::with_capacity(
        configured
            .roots
            .len()
            .saturating_add(usize::from(known_user_cache.is_some())),
    );
    if let Some(known) = known_user_cache {
        let display_root = known.path().to_path_buf();
        let prepared = prepare_targeted_scan_root(&display_root);
        let excluded_subtrees = prepared
            .as_ref()
            .ok()
            .and_then(|(canonical, _)| {
                let cache = config.cache_directory();
                cache
                    .strip_prefix(&display_root)
                    .ok()
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .map(|relative| canonical.join(relative))
            })
            .into_iter()
            .collect();
        roots.push(TargetedReclaimCatalogRoot {
            ordinal: 0,
            kind: TargetedReclaimRootKind::KnownUserLibraryCaches,
            configured_root_ordinal: None,
            display_root,
            prepared,
            known_user_cache: Some(known),
            excluded_subtrees,
            max_nodes: 0,
        });
    }

    for (configured_index, root) in configured.roots.iter().enumerate() {
        if known_path
            .as_deref()
            .is_some_and(|known| super::config::paths_overlap(known, root))
        {
            continue;
        }
        let prepared = prepare_targeted_scan_root(root);
        let canonical_overlap = prepared.as_ref().ok().is_some_and(|(canonical, _)| {
            roots.iter().any(|existing| {
                existing
                    .prepared
                    .as_ref()
                    .ok()
                    .is_some_and(|(other, _)| super::config::paths_overlap(canonical, other))
            })
        });
        if canonical_overlap {
            continue;
        }
        roots.push(TargetedReclaimCatalogRoot {
            ordinal: 0,
            kind: TargetedReclaimRootKind::ConfiguredProject,
            configured_root_ordinal: Some(
                u16::try_from(configured_index)
                    .map_err(|_| TargetedProjectScanError::CorruptData)?,
            ),
            display_root: root.clone(),
            prepared,
            known_user_cache: None,
            excluded_subtrees: Vec::new(),
            max_nodes: 0,
        });
    }

    let include_known = roots
        .first()
        .is_some_and(|root| root.kind == TargetedReclaimRootKind::KnownUserLibraryCaches);
    let configured_count = u16::try_from(
        roots
            .iter()
            .filter(|root| root.kind == TargetedReclaimRootKind::ConfiguredProject)
            .count(),
    )
    .map_err(|_| TargetedProjectScanError::CorruptData)?;
    let layout = targeted_reclaim_root_catalog_layout(configured_count, include_known)
        .map_err(|_| TargetedProjectScanError::CorruptData)?;
    if layout.len() != roots.len() {
        return Err(TargetedProjectScanError::InternalState);
    }
    for (root, slot) in roots.iter_mut().zip(layout) {
        if root.kind != slot.kind {
            return Err(TargetedProjectScanError::InternalState);
        }
        root.ordinal = slot.ordinal;
        root.max_nodes = slot.max_nodes;
    }

    let root_count =
        u16::try_from(roots.len()).map_err(|_| TargetedProjectScanError::CorruptData)?;
    let digest_sha256 = targeted_reclaim_catalog_digest(configured.revision, &roots)?;
    Ok(TargetedReclaimCatalog {
        stamp: TargetedReclaimRootCatalogStamp {
            known_roots_policy_revision: TARGETED_RECLAIM_ROOT_POLICY_REVISION,
            configured_roots_revision: configured.revision,
            known_user_library_caches_included: include_known,
            root_count,
            digest_sha256,
        },
        roots,
    })
}

fn targeted_reclaim_catalog_digest(
    configured_roots_revision: u64,
    roots: &[TargetedReclaimCatalogRoot],
) -> Result<[u8; 32], TargetedProjectScanError> {
    let mut digest = Sha256::new();
    digest.update(b"dux-targeted-reclaim-root-catalog-v1\0");
    digest.update(TARGETED_RECLAIM_ROOT_POLICY_REVISION.to_le_bytes());
    digest.update(configured_roots_revision.to_le_bytes());
    digest.update(
        u16::try_from(roots.len())
            .map_err(|_| TargetedProjectScanError::CorruptData)?
            .to_le_bytes(),
    );
    for root in roots {
        digest.update(root.ordinal.to_le_bytes());
        digest.update([match root.kind {
            TargetedReclaimRootKind::KnownUserLibraryCaches => 1,
            TargetedReclaimRootKind::ConfiguredProject => 2,
        }]);
        match root.configured_root_ordinal {
            Some(ordinal) => {
                digest.update([1]);
                digest.update(ordinal.to_le_bytes());
            }
            None => digest.update([0]),
        }
        update_targeted_catalog_path(&mut digest, &root.display_root)?;
        match &root.prepared {
            Ok((canonical, identity)) => {
                digest.update([1]);
                update_targeted_catalog_path(&mut digest, canonical)?;
                digest.update(identity.volume().to_le_bytes());
                digest.update(identity.object().to_le_bytes());
            }
            Err(reason) => {
                digest.update([0, targeted_root_reason_rank(*reason)]);
            }
        }
        digest.update(root.max_nodes.to_le_bytes());
        digest.update(
            u16::try_from(root.excluded_subtrees.len())
                .map_err(|_| TargetedProjectScanError::CorruptData)?
                .to_le_bytes(),
        );
        for excluded in &root.excluded_subtrees {
            update_targeted_catalog_path(&mut digest, excluded)?;
        }
    }
    Ok(digest.finalize().into())
}

fn update_targeted_catalog_path(
    digest: &mut Sha256,
    path: &Path,
) -> Result<(), TargetedProjectScanError> {
    let value = HostValue::from_root(path).map_err(|_| TargetedProjectScanError::CorruptData)?;
    digest.update([value.encoding() as u8]);
    digest.update(
        u64::try_from(value.bytes().len())
            .map_err(|_| TargetedProjectScanError::CorruptData)?
            .to_le_bytes(),
    );
    digest.update(value.bytes());
    Ok(())
}

const fn targeted_root_reason_rank(reason: ScanRootErrorKind) -> u8 {
    match reason {
        ScanRootErrorKind::InvalidPath => 1,
        ScanRootErrorKind::Missing => 2,
        ScanRootErrorKind::AccessDenied => 3,
        ScanRootErrorKind::NotDirectory => 4,
        ScanRootErrorKind::Symlink => 5,
        ScanRootErrorKind::ChangedDuringValidation => 6,
        ScanRootErrorKind::IdentityUnavailable => 7,
        ScanRootErrorKind::VolumeMismatch => 8,
        ScanRootErrorKind::VolumeUnproven => 9,
        ScanRootErrorKind::UnsupportedPlatform => 10,
        ScanRootErrorKind::Unavailable => 11,
    }
}

fn targeted_scan_record_matches_kind(scan_id: &ScanId, kind: TargetedReclaimRootKind) -> bool {
    let is_known = scan_id
        .as_str()
        .starts_with(crate::domain::KNOWN_USER_CACHE_SCAN_ID_PREFIX);
    match kind {
        TargetedReclaimRootKind::KnownUserLibraryCaches => is_known,
        TargetedReclaimRootKind::ConfiguredProject => {
            scan_id.as_str().starts_with("scan:targeted:") && !is_known
        }
    }
}

fn targeted_scan_snapshot_matches_root(
    snapshots: &SnapshotRepository,
    record: &ScanRecord,
    canonical_root: &Path,
    expected_identity: FilesystemIdentity,
) -> bool {
    let Some(reference) = record.snapshot() else {
        return false;
    };
    let Ok(document) = snapshots.load_for_candidate_recovery(reference) else {
        return false;
    };
    if document.metadata.scan_id != *record.id()
        || !document.metadata.root.matches_path(canonical_root)
    {
        return false;
    }
    #[cfg(unix)]
    {
        let Some(identity) = document.nodes.first().and_then(|node| node.unix_identity) else {
            return false;
        };
        identity.device() == expected_identity.volume()
            && u128::from(identity.inode()) == expected_identity.object()
    }
    #[cfg(not(unix))]
    {
        let _ = expected_identity;
        false
    }
}

fn normalize_macos_system_path_alias(root: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        for (alias, canonical) in [
            (Path::new("/var"), Path::new("/private/var")),
            (Path::new("/tmp"), Path::new("/private/tmp")),
            (Path::new("/etc"), Path::new("/private/etc")),
        ] {
            if let Ok(relative) = root.strip_prefix(alias) {
                return canonical.join(relative);
            }
        }
    }
    root.to_path_buf()
}

fn map_targeted_root_capture_error(error: CanonicalPathError) -> ScanRootErrorKind {
    match error {
        CanonicalPathError::UnsupportedPlatform => ScanRootErrorKind::UnsupportedPlatform,
        CanonicalPathError::Missing { .. } => ScanRootErrorKind::Missing,
        CanonicalPathError::AccessDenied { .. } => ScanRootErrorKind::AccessDenied,
        CanonicalPathError::Io { kind, .. } => match kind {
            std::io::ErrorKind::NotFound => ScanRootErrorKind::Missing,
            std::io::ErrorKind::PermissionDenied => ScanRootErrorKind::AccessDenied,
            _ => ScanRootErrorKind::Unavailable,
        },
        CanonicalPathError::ScanRootNotDirectory
        | CanonicalPathError::NonDirectoryAncestor { .. } => ScanRootErrorKind::NotDirectory,
        CanonicalPathError::SymlinkOrReparsePoint { .. }
        | CanonicalPathError::CanonicalPathMismatch { .. }
        | CanonicalPathError::CanonicalEscapesScanRoot => ScanRootErrorKind::Symlink,
        CanonicalPathError::IdentityUnavailable { .. } => ScanRootErrorKind::IdentityUnavailable,
        CanonicalPathError::ChangedDuringValidation { .. } => {
            ScanRootErrorKind::ChangedDuringValidation
        }
        CanonicalPathError::CrossVolume { .. } => ScanRootErrorKind::VolumeMismatch,
        CanonicalPathError::CanonicalizationFailed { source, .. } => match source.kind() {
            std::io::ErrorKind::NotFound => ScanRootErrorKind::Missing,
            std::io::ErrorKind::PermissionDenied => ScanRootErrorKind::AccessDenied,
            _ => ScanRootErrorKind::Unavailable,
        },
        CanonicalPathError::UnsupportedTargetKind
        | CanonicalPathError::MismatchedScanRoot
        | CanonicalPathError::BoundaryTooDeep { .. } => ScanRootErrorKind::Unavailable,
    }
}

fn prove_root_on_affected_volume(
    root: &Path,
    volume_mount_path: &Path,
) -> Result<(), ScanRootErrorKind> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let canonical_mount = std::fs::canonicalize(volume_mount_path)
            .map_err(|_| ScanRootErrorKind::VolumeUnproven)?;
        let root_metadata =
            std::fs::metadata(root).map_err(|_| ScanRootErrorKind::VolumeUnproven)?;
        let mount_metadata =
            std::fs::metadata(&canonical_mount).map_err(|_| ScanRootErrorKind::VolumeUnproven)?;
        if root_metadata.dev() == mount_metadata.dev() {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        if canonical_mount == Path::new("/") {
            let data_metadata = std::fs::metadata("/System/Volumes/Data")
                .map_err(|_| ScanRootErrorKind::VolumeUnproven)?;
            if root_metadata.dev() == data_metadata.dev() {
                return Ok(());
            }
        }
        Err(ScanRootErrorKind::VolumeMismatch)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, volume_mount_path);
        Err(ScanRootErrorKind::VolumeUnproven)
    }
}

fn generate_scan_id(
    origin: ScanTaskOrigin,
    targeted_root_kind: Option<TargetedReclaimRootKind>,
) -> Result<ScanId, TaskFailureKind> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| TaskFailureKind::InternalFailure)?;
    let prefix = match (origin, targeted_root_kind) {
        (
            ScanTaskOrigin::TargetedRecommendation,
            Some(TargetedReclaimRootKind::KnownUserLibraryCaches),
        ) => crate::domain::KNOWN_USER_CACHE_SCAN_ID_PREFIX,
        (ScanTaskOrigin::TargetedRecommendation, _) => "scan:targeted:",
        (ScanTaskOrigin::UserFull | ScanTaskOrigin::UserSubtree, _) => "scan:",
    };
    let mut value = String::with_capacity(prefix.len() + random.len() * 2);
    value.push_str(prefix);
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
fn run_rust_target_dry_run_task(
    store: Arc<StoreCoordinator>,
    admitted: Result<TrustedRustTargetDryRun, RustTargetDryRunError>,
    started_at: SystemTime,
    before_validation: Box<dyn FnOnce() + Send>,
    context: &TaskContext,
) -> WorkOutcome {
    if context.is_cancellation_requested() || !context.engine_is_open() {
        if let Ok(dry_run) = admitted {
            dry_run.release();
        }
        return WorkOutcome::Cancelled(None);
    }
    let dry_run = match admitted {
        Ok(dry_run) => dry_run,
        Err(error) => return rust_target_dry_run_failed(error),
    };
    let quarantine = process_cleanup_quarantine()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if store_is_quarantined(&quarantine, &store) {
        dry_run.release();
        return rust_target_dry_run_failed(RustTargetDryRunError::HistoryUnresolved);
    }
    drop(quarantine);

    let session_id = match generate_dry_run_session_id() {
        Ok(session_id) => session_id,
        Err(error) => {
            dry_run.release();
            return rust_target_dry_run_failed(error);
        }
    };
    let lease = match store.acquire_cleanup_journal_lease(Duration::from_secs(5)) {
        Ok(lease) => lease,
        Err(error) => {
            dry_run.release();
            return rust_target_dry_run_failed(map_dry_run_history_error(error.kind));
        }
    };
    before_validation();
    let observed_at = SystemTime::now();
    let (mut requested_outcome, mut completed_at) =
        if context.is_cancellation_requested() || !context.engine_is_open() {
            (
                ValidatedDryRunOutcome::Cancelled("cancelled"),
                SystemTime::now(),
            )
        } else {
            match dry_run.validate_at(observed_at) {
                Ok(validated_at) => {
                    if context.is_cancellation_requested() || !context.engine_is_open() {
                        (
                            ValidatedDryRunOutcome::Cancelled("cancelled"),
                            SystemTime::now(),
                        )
                    } else {
                        (ValidatedDryRunOutcome::DryRun, validated_at)
                    }
                }
                Err(error) => (
                    rust_target_dry_run_validation_outcome(error),
                    SystemTime::now(),
                ),
            }
        };
    let boundary = context.close_cancellation_boundary();
    if matches!(requested_outcome, ValidatedDryRunOutcome::DryRun)
        && (boundary.cancellation_requested || !boundary.engine_open)
    {
        requested_outcome = ValidatedDryRunOutcome::Cancelled("cancelled");
        completed_at = SystemTime::now();
    }
    let first_record: Result<_, DryRunJournalFailure> = lease.record_validated_dry_run(
        session_id.clone(),
        dry_run.plan(),
        CleanupTrigger::Manual,
        requested_outcome,
        started_at,
        completed_at,
    );
    let recorded = match first_record {
        Ok(status) => Ok(status),
        Err(failure) if failure.may_have_committed() => {
            failure.into_lease().record_validated_dry_run(
                session_id.clone(),
                dry_run.plan(),
                CleanupTrigger::Manual,
                requested_outcome,
                started_at,
                completed_at,
            )
        }
        Err(failure) => {
            let error = map_dry_run_history_error(failure.kind());
            drop(failure.into_lease());
            dry_run.release();
            return rust_target_dry_run_failed(error);
        }
    };
    let status = match recorded {
        Ok(status) => status,
        Err(failure) => {
            drop(failure.into_lease());
            dry_run.release();
            return rust_target_dry_run_failed(RustTargetDryRunError::HistoryUnresolved);
        }
    };
    dry_run.release();
    let result = match rust_target_dry_run_result(&session_id, status) {
        Ok(result) => Arc::new(result),
        Err(error) => return rust_target_dry_run_failed(error),
    };
    if result.status() == DurableCleanupSessionStatus::Cancelled {
        WorkOutcome::Cancelled(Some(TaskResult::RustTargetDryRun(result)))
    } else {
        WorkOutcome::Succeeded(TaskResult::RustTargetDryRun(result))
    }
}

#[cfg(target_os = "macos")]
fn rust_target_dry_run_validation_outcome(
    error: RustTargetDryRunValidationError,
) -> ValidatedDryRunOutcome {
    match error {
        RustTargetDryRunValidationError::Expired => {
            ValidatedDryRunOutcome::ChangedSincePlan("review_expired")
        }
        RustTargetDryRunValidationError::InvalidPlan => {
            ValidatedDryRunOutcome::Failed("invalid_plan")
        }
        error if error.is_unavailable() => {
            ValidatedDryRunOutcome::Unavailable("validation_unavailable")
        }
        RustTargetDryRunValidationError::Authorization(_) => {
            ValidatedDryRunOutcome::ChangedSincePlan("authorization_changed")
        }
        RustTargetDryRunValidationError::RuleEvidence(_) => {
            ValidatedDryRunOutcome::ChangedSincePlan("evidence_changed")
        }
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
const fn map_rust_target_dry_run_review_error(
    error: RustTargetPlanReviewError,
) -> RustTargetDryRunError {
    match error {
        RustTargetPlanReviewError::Closed => RustTargetDryRunError::Closed,
        RustTargetPlanReviewError::WrongEngine => RustTargetDryRunError::WrongEngine,
        RustTargetPlanReviewError::ParentReviewUnavailable => {
            RustTargetDryRunError::ParentReviewUnavailable
        }
        RustTargetPlanReviewError::ReviewExpired => RustTargetDryRunError::ReviewExpired,
        RustTargetPlanReviewError::ChangedDuringReview
        | RustTargetPlanReviewError::CandidateUnavailable
        | RustTargetPlanReviewError::CargoNotEnrolled
        | RustTargetPlanReviewError::ActiveProcesses => RustTargetDryRunError::ChangedDuringReview,
        RustTargetPlanReviewError::UnsupportedPlatform | RustTargetPlanReviewError::Unavailable => {
            RustTargetDryRunError::Unavailable
        }
        RustTargetPlanReviewError::BudgetExceeded => RustTargetDryRunError::BudgetExceeded,
        RustTargetPlanReviewError::Busy => RustTargetDryRunError::Busy,
        RustTargetPlanReviewError::UnsafeStorage => RustTargetDryRunError::UnsafeStorage,
        RustTargetPlanReviewError::CorruptData => RustTargetDryRunError::CorruptData,
        RustTargetPlanReviewError::InternalState => RustTargetDryRunError::InternalState,
    }
}

#[cfg(target_os = "macos")]
fn rust_target_dry_run_failed(error: RustTargetDryRunError) -> WorkOutcome {
    WorkOutcome::Failed(
        TaskFailureKind::RustTargetDryRun(rust_target_dry_run_failure_kind(error)),
        None,
    )
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
    root_identity: FilesystemIdentity,
    origin: ScanTaskOrigin,
    targeted_root_kind: Option<TargetedReclaimRootKind>,
) -> Result<NewScanRecord, TaskFailureKind> {
    const COLLISION_RETRIES: usize = 4;
    for _ in 0..COLLISION_RETRIES {
        let id = generate_scan_id(origin, targeted_root_kind)?;
        let start = NewScanRecord::try_new_with_root_identity(
            id,
            root.to_path_buf(),
            SystemTime::now(),
            root_identity,
        )
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
    scope: CandidateEvaluationScope,
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

    let batch = match evaluate_completed_scan_candidates(scan_id, artifact, scheduled_at, scope) {
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

const fn map_legacy_running_scan_dismissal_store_error(
    error: LegacyRunningScanDismissalStoreError,
) -> LegacyRunningScanDismissalError {
    match error {
        LegacyRunningScanDismissalStoreError::NothingEligible => {
            LegacyRunningScanDismissalError::NothingEligible
        }
        LegacyRunningScanDismissalStoreError::ChangedSincePreview => {
            LegacyRunningScanDismissalError::ChangedSincePreview
        }
        LegacyRunningScanDismissalStoreError::History(history) => match history.kind {
            HistoryErrorKind::IncompatibleSchema => {
                LegacyRunningScanDismissalError::IncompatibleSchema
            }
            HistoryErrorKind::QueryLimitExceeded => {
                LegacyRunningScanDismissalError::QueryLimitExceeded
            }
            HistoryErrorKind::Busy => LegacyRunningScanDismissalError::Busy,
            HistoryErrorKind::UnsafeStorage => LegacyRunningScanDismissalError::UnsafeStorage,
            HistoryErrorKind::CorruptData => LegacyRunningScanDismissalError::CorruptData,
            HistoryErrorKind::OutcomeUnknown => LegacyRunningScanDismissalError::OutcomeUnknown,
            HistoryErrorKind::DatabaseUnavailable => LegacyRunningScanDismissalError::Unavailable,
            HistoryErrorKind::InternalState
            | HistoryErrorKind::InvalidInput
            | HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::NotFound
            | HistoryErrorKind::InvalidTransition => LegacyRunningScanDismissalError::InternalState,
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

fn public_scan_summary(record: &ScanRecord) -> DurableScanSummary {
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
        status: public_durable_scan_status(record.status()),
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
}

fn snapshot_diff_coverage(
    coverage: &crate::domain::ScanCoverage,
) -> Result<SnapshotDiffCoverage, SnapshotReviewError> {
    let issue_record_count =
        u64::try_from(coverage.issues().len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let issue_occurrence_count = coverage.issues().iter().try_fold(0_u64, |total, issue| {
        total.checked_add(u64::from(issue.occurrence_count()))
    });
    Ok(SnapshotDiffCoverage {
        status: coverage.status(),
        measured_permille: coverage.measured_permille().map(|value| value.get()),
        issue_record_count,
        issue_occurrence_count: issue_occurrence_count
            .ok_or(SnapshotReviewError::BudgetExceeded)?,
    })
}

const fn map_targeted_project_scan_history_error(
    kind: HistoryErrorKind,
) -> TargetedProjectScanError {
    match kind {
        HistoryErrorKind::InvalidInput => TargetedProjectScanError::InvalidAnchor,
        HistoryErrorKind::IncompatibleSchema => TargetedProjectScanError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => TargetedProjectScanError::BudgetExceeded,
        HistoryErrorKind::Busy => TargetedProjectScanError::Busy,
        HistoryErrorKind::UnsafeStorage => TargetedProjectScanError::UnsafeStorage,
        HistoryErrorKind::CorruptData | HistoryErrorKind::NotFound => {
            TargetedProjectScanError::CorruptData
        }
        HistoryErrorKind::DatabaseUnavailable => TargetedProjectScanError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => TargetedProjectScanError::OutcomeUnknown,
        HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::InternalState => TargetedProjectScanError::InternalState,
    }
}

const fn map_targeted_candidate_history_error(
    error: CandidateHistoryError,
) -> TargetedProjectScanError {
    match error {
        CandidateHistoryError::Closed => TargetedProjectScanError::Closed,
        CandidateHistoryError::ScanNotFound | CandidateHistoryError::CorruptData => {
            TargetedProjectScanError::CorruptData
        }
        CandidateHistoryError::IncompatibleSchema => TargetedProjectScanError::IncompatibleSchema,
        CandidateHistoryError::Busy => TargetedProjectScanError::Busy,
        CandidateHistoryError::UnsafeStorage => TargetedProjectScanError::UnsafeStorage,
        CandidateHistoryError::QueryLimitExceeded => TargetedProjectScanError::BudgetExceeded,
        CandidateHistoryError::Unavailable => TargetedProjectScanError::Unavailable,
        CandidateHistoryError::InternalState => TargetedProjectScanError::InternalState,
    }
}

const fn map_targeted_project_scan_to_emergency_recovery_error(
    error: TargetedProjectScanError,
) -> EmergencyRecoveryError {
    match error {
        TargetedProjectScanError::Closed => EmergencyRecoveryError::Closed,
        TargetedProjectScanError::ReadOnlyStore => EmergencyRecoveryError::ReadOnlyStore,
        TargetedProjectScanError::RegistryChanged { .. } => EmergencyRecoveryError::RegistryChanged,
        TargetedProjectScanError::InvalidCatalog => EmergencyRecoveryError::InvalidCatalog,
        TargetedProjectScanError::CatalogChanged => EmergencyRecoveryError::CatalogChanged,
        TargetedProjectScanError::InvalidAnchor | TargetedProjectScanError::PressureChanged => {
            EmergencyRecoveryError::PressureChanged
        }
        TargetedProjectScanError::IncompatibleSchema => EmergencyRecoveryError::IncompatibleSchema,
        TargetedProjectScanError::Busy => EmergencyRecoveryError::Busy,
        TargetedProjectScanError::UnsafeStorage => EmergencyRecoveryError::UnsafeStorage,
        TargetedProjectScanError::BudgetExceeded => EmergencyRecoveryError::BudgetExceeded,
        TargetedProjectScanError::CorruptData => EmergencyRecoveryError::CorruptData,
        TargetedProjectScanError::Unavailable => EmergencyRecoveryError::Unavailable,
        TargetedProjectScanError::OutcomeUnknown => EmergencyRecoveryError::OutcomeUnknown,
        TargetedProjectScanError::InvalidOrdinal { .. }
        | TargetedProjectScanError::QueueFull
        | TargetedProjectScanError::TaskIdExhausted
        | TargetedProjectScanError::InternalState => EmergencyRecoveryError::InternalState,
    }
}

const fn map_emergency_recovery_history_error(kind: HistoryErrorKind) -> EmergencyRecoveryError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => EmergencyRecoveryError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => EmergencyRecoveryError::BudgetExceeded,
        HistoryErrorKind::Busy => EmergencyRecoveryError::Busy,
        HistoryErrorKind::UnsafeStorage => EmergencyRecoveryError::UnsafeStorage,
        HistoryErrorKind::CorruptData | HistoryErrorKind::NotFound => {
            EmergencyRecoveryError::CorruptData
        }
        HistoryErrorKind::DatabaseUnavailable => EmergencyRecoveryError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => EmergencyRecoveryError::OutcomeUnknown,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::InternalState => EmergencyRecoveryError::InternalState,
    }
}

const fn map_candidate_history_to_emergency_recovery_error(
    error: CandidateHistoryError,
) -> EmergencyRecoveryError {
    match error {
        CandidateHistoryError::Closed => EmergencyRecoveryError::Closed,
        CandidateHistoryError::ScanNotFound | CandidateHistoryError::CorruptData => {
            EmergencyRecoveryError::CorruptData
        }
        CandidateHistoryError::IncompatibleSchema => EmergencyRecoveryError::IncompatibleSchema,
        CandidateHistoryError::Busy => EmergencyRecoveryError::Busy,
        CandidateHistoryError::UnsafeStorage => EmergencyRecoveryError::UnsafeStorage,
        CandidateHistoryError::QueryLimitExceeded => EmergencyRecoveryError::BudgetExceeded,
        CandidateHistoryError::Unavailable => EmergencyRecoveryError::Unavailable,
        CandidateHistoryError::InternalState => EmergencyRecoveryError::InternalState,
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

fn public_rule_outcome_batch(
    batch: StoredRuleOutcomeBatch,
) -> Result<DurableRuleOutcomeBatch, RuleOutcomeError> {
    let session_id = DurableCleanupSessionId::new(batch.session_id.as_str().to_owned())
        .ok_or(RuleOutcomeError::CorruptData)?;
    let outcomes = batch
        .outcomes
        .into_iter()
        .enumerate()
        .map(|(expected, outcome)| {
            if outcome.item_ordinal != expected {
                return Err(RuleOutcomeError::CorruptData);
            }
            public_rule_outcome(outcome)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DurableRuleOutcomeBatch::new(session_id, outcomes))
}

fn public_rule_outcome(outcome: StoredRuleOutcome) -> Result<DurableRuleOutcome, RuleOutcomeError> {
    let item_ordinal =
        u16::try_from(outcome.item_ordinal).map_err(|_| RuleOutcomeError::CorruptData)?;
    let state = match outcome.state {
        StoredRuleOutcomeState::NotEligible { reason } => DurableRuleOutcomeState::NotEligible {
            reason: match reason {
                StoredRuleOutcomeNotEligibleReason::SourceCleanupIncomplete => {
                    RuleOutcomeNotEligibleReason::SourceCleanupIncomplete
                }
                StoredRuleOutcomeNotEligibleReason::ItemNotSuccessfulPermanentRegenerable => {
                    RuleOutcomeNotEligibleReason::ItemNotSuccessfulPermanentRegenerable
                }
                StoredRuleOutcomeNotEligibleReason::SourceScanNotComparable => {
                    RuleOutcomeNotEligibleReason::SourceScanNotComparable
                }
                StoredRuleOutcomeNotEligibleReason::SourceEvaluationNotComparable => {
                    RuleOutcomeNotEligibleReason::SourceEvaluationNotComparable
                }
                StoredRuleOutcomeNotEligibleReason::SourceEvaluationAfterPlan => {
                    RuleOutcomeNotEligibleReason::SourceEvaluationAfterPlan
                }
                StoredRuleOutcomeNotEligibleReason::SourceCandidateMismatch => {
                    RuleOutcomeNotEligibleReason::SourceCandidateMismatch
                }
            },
        },
        StoredRuleOutcomeState::AwaitingComparableScan { cleaned_at } => {
            DurableRuleOutcomeState::AwaitingComparableScan { cleaned_at }
        }
        StoredRuleOutcomeState::Superseded {
            cleaned_at,
            superseded_at,
        } => DurableRuleOutcomeState::Superseded {
            cleaned_at,
            superseded_at,
        },
        StoredRuleOutcomeState::LaterSizeObserved {
            cleaned_at,
            observed_at,
            observed_bytes,
        } => DurableRuleOutcomeState::LaterSizeObserved {
            cleaned_at,
            observed_at,
            observed_bytes,
        },
        StoredRuleOutcomeState::ZeroBaselineObserved {
            cleaned_at,
            observed_at,
        } => DurableRuleOutcomeState::ZeroBaselineObserved {
            cleaned_at,
            observed_at,
        },
        StoredRuleOutcomeState::Regrown {
            cleaned_at,
            zero_observed_at,
            observed_at,
            observed_bytes,
        } => DurableRuleOutcomeState::Regrown {
            cleaned_at,
            zero_observed_at,
            observed_at,
            observed_bytes,
            regrowth_duration: observed_at
                .duration_since(zero_observed_at)
                .map_err(|_| RuleOutcomeError::CorruptData)?,
        },
    };
    Ok(DurableRuleOutcome::new(item_ordinal, outcome.rule, state))
}

fn public_storage_thief_ranking(
    ranking: StoredStorageThiefRanking,
) -> Result<DurableStorageThiefRanking, StorageThiefError> {
    if MAX_STORAGE_THIEF_GROUPS != usize::from(MAX_STORAGE_THIEF_RANKING_GROUPS)
        || MAX_STORAGE_THIEF_SOURCE_SESSIONS
            != usize::from(MAX_STORAGE_THIEF_RANKING_SOURCE_SESSIONS)
        || ranking.permanent_safe_session_count > MAX_STORAGE_THIEF_SOURCE_SESSIONS
        || ranking.manual_cleanup_session_count > ranking.permanent_safe_session_count
        || ranking.groups.len() > MAX_STORAGE_THIEF_GROUPS
        || ranking.groups.len() > ranking.ranked_rule_count
    {
        return Err(StorageThiefError::CorruptData);
    }
    let mut rule_ids = std::collections::HashSet::new();
    for group in &ranking.groups {
        if !rule_ids.insert(group.latest_rule.id().as_str())
            || group.observed_revision_count == 0
            || group.successful_cleanup_count == 0
            || group.successful_manual_cleanup_count > group.successful_cleanup_count
            || group.observed_regrowth_cycle_count == 0
            || group.manual_regrowth_cycle_count > group.observed_regrowth_cycle_count
            || group.total_observed_regrown_bytes == 0
            || group.total_regrowth_duration.is_zero()
            || group.latest_regrowth_at < UNIX_EPOCH
            || group.latest_cleanup_at < UNIX_EPOCH
            || (group.automation_history_threshold_met
                && (group.successful_manual_cleanup_count < 2
                    || group.manual_regrowth_cycle_count == 0))
        {
            return Err(StorageThiefError::CorruptData);
        }
        let expected_rate = storage_thief_rate_per_day(
            u128::from(group.total_observed_regrown_bytes),
            group.total_regrowth_duration.as_nanos(),
        );
        if expected_rate != (group.bytes_regrown_per_day, group.rate_capped) {
            return Err(StorageThiefError::CorruptData);
        }
    }
    for pair in ranking.groups.windows(2) {
        if storage_thief_group_order(&pair[0], &pair[1]) == std::cmp::Ordering::Greater {
            return Err(StorageThiefError::CorruptData);
        }
    }

    let permanent_safe_session_count = u16::try_from(ranking.permanent_safe_session_count)
        .map_err(|_| StorageThiefError::CorruptData)?;
    let manual_cleanup_session_count = u16::try_from(ranking.manual_cleanup_session_count)
        .map_err(|_| StorageThiefError::CorruptData)?;
    let ranked_rule_count =
        u16::try_from(ranking.ranked_rule_count).map_err(|_| StorageThiefError::CorruptData)?;
    let groups = ranking
        .groups
        .into_iter()
        .map(public_storage_thief_group)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DurableStorageThiefRanking::new(
        permanent_safe_session_count,
        manual_cleanup_session_count,
        ranked_rule_count,
        ranking.has_older_permanent_safe_sessions,
        groups,
    ))
}

fn public_automation_schedule_suggestion_feed(
    feed: StoredAutomationScheduleSuggestionFeed,
    current_rule_count: usize,
) -> Result<AutomationScheduleSuggestionFeed, AutomationScheduleSuggestionError> {
    if STORED_MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS
        != usize::from(MAX_AUTOMATION_HISTORY_SUGGESTION_SOURCE_SESSIONS)
        || STORED_MAX_AUTOMATION_HISTORY_SUGGESTIONS
            != usize::from(MAX_AUTOMATION_HISTORY_SUGGESTIONS)
        || feed.source_session_count > STORED_MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS
        || feed.suggestions.len() > STORED_MAX_AUTOMATION_HISTORY_SUGGESTIONS
        || feed.qualifying_rule_count > current_rule_count
        || feed.suggestions.len()
            != feed
                .qualifying_rule_count
                .min(STORED_MAX_AUTOMATION_HISTORY_SUGGESTIONS)
    {
        return Err(AutomationScheduleSuggestionError::InternalState);
    }
    let mut unique_rules = BTreeSet::new();
    if feed.suggestions.iter().any(|suggestion| {
        !unique_rules.insert(suggestion.rule.clone())
            || suggestion
                .latest_manual_attempt_at
                .duration_since(UNIX_EPOCH)
                .is_err()
            || suggestion
                .latest_regrowth_at
                .duration_since(UNIX_EPOCH)
                .is_err()
    }) || feed
        .suggestions
        .windows(2)
        .any(|pair| automation_schedule_suggestion_order(&pair[0], &pair[1]).is_ge())
    {
        return Err(AutomationScheduleSuggestionError::InternalState);
    }
    let source_session_count = u16::try_from(feed.source_session_count)
        .map_err(|_| AutomationScheduleSuggestionError::InternalState)?;
    let qualifying_rule_count = u16::try_from(feed.qualifying_rule_count)
        .map_err(|_| AutomationScheduleSuggestionError::InternalState)?;
    if feed.has_older_source_sessions
        && source_session_count != MAX_AUTOMATION_HISTORY_SUGGESTION_SOURCE_SESSIONS
    {
        return Err(AutomationScheduleSuggestionError::InternalState);
    }
    let suggestions = feed
        .suggestions
        .into_iter()
        .map(|suggestion| public_automation_schedule_suggestion(suggestion, source_session_count))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AutomationScheduleSuggestionFeed::new(
        source_session_count,
        qualifying_rule_count,
        feed.has_older_source_sessions,
        suggestions,
    ))
}

fn automation_schedule_suggestion_order(
    left: &StoredAutomationScheduleSuggestion,
    right: &StoredAutomationScheduleSuggestion,
) -> std::cmp::Ordering {
    right
        .latest_regrowth_at
        .cmp(&left.latest_regrowth_at)
        .then_with(|| {
            right
                .successful_manual_run_count
                .cmp(&left.successful_manual_run_count)
        })
        .then_with(|| {
            right
                .manual_regrowth_cycle_count
                .cmp(&left.manual_regrowth_cycle_count)
        })
        .then_with(|| left.rule.cmp(&right.rule))
}

fn public_automation_schedule_suggestion(
    suggestion: StoredAutomationScheduleSuggestion,
    source_session_count: u16,
) -> Result<AutomationScheduleSuggestion, AutomationScheduleSuggestionError> {
    let successful_manual_run_count = u16::try_from(suggestion.successful_manual_run_count)
        .map_err(|_| AutomationScheduleSuggestionError::InternalState)?;
    let manual_regrowth_cycle_count = u16::try_from(suggestion.manual_regrowth_cycle_count)
        .map_err(|_| AutomationScheduleSuggestionError::InternalState)?;
    if successful_manual_run_count < 2
        || successful_manual_run_count > source_session_count
        || manual_regrowth_cycle_count == 0
        || manual_regrowth_cycle_count > successful_manual_run_count
    {
        return Err(AutomationScheduleSuggestionError::InternalState);
    }
    Ok(AutomationScheduleSuggestion::new(
        suggestion.rule,
        successful_manual_run_count,
        manual_regrowth_cycle_count,
        suggestion.latest_manual_attempt_at,
        suggestion.latest_regrowth_at,
    ))
}

fn public_running_scan_debt_census(
    census: StoredRunningScanDebtCensus,
) -> Result<RunningScanDebtCensus, RunningScanDebtCensusError> {
    if census.inspected_unclaimed_count > MAX_RUNNING_SCAN_DEBT_CENSUS_ROWS
        || census
            .pristine_unclaimed_count
            .checked_add(census.unexplained_unclaimed_count)
            != Some(census.inspected_unclaimed_count)
        || (census.has_more
            && census.inspected_unclaimed_count != MAX_RUNNING_SCAN_DEBT_CENSUS_ROWS)
    {
        return Err(RunningScanDebtCensusError::CorruptData);
    }
    Ok(RunningScanDebtCensus::new(
        census.inspected_unclaimed_count,
        census.pristine_unclaimed_count,
        census.unexplained_unclaimed_count,
        census.has_more,
    ))
}

fn public_claimed_running_scan_provenance_census(
    census: StoredClaimedRunningScanProvenanceCensus,
) -> Result<ClaimedRunningScanProvenanceCensus, ClaimedRunningScanProvenanceCensusError> {
    let comparable = census
        .same_host_current_boot_count
        .checked_add(census.same_host_prior_boot_count)
        .and_then(|count| count.checked_add(census.foreign_host_count));
    let classified = comparable
        .and_then(|count| count.checked_add(census.stored_unproven_count))
        .and_then(|count| count.checked_add(census.current_context_unavailable_count));
    if census.inspected_claimed_count > MAX_CLAIMED_RUNNING_SCAN_PROVENANCE_CENSUS_ROWS
        || classified != Some(census.inspected_claimed_count)
        || (census.current_context_unavailable_count > 0 && comparable != Some(0))
        || (census.has_more
            && census.inspected_claimed_count != MAX_CLAIMED_RUNNING_SCAN_PROVENANCE_CENSUS_ROWS)
    {
        return Err(ClaimedRunningScanProvenanceCensusError::CorruptData);
    }
    Ok(ClaimedRunningScanProvenanceCensus::new(
        census.inspected_claimed_count,
        census.same_host_current_boot_count,
        census.same_host_prior_boot_count,
        census.foreign_host_count,
        census.stored_unproven_count,
        census.current_context_unavailable_count,
        census.has_more,
    ))
}

fn public_cleanup_recovery_diagnostic_census(
    census: StoredCleanupRecoveryDiagnosticCensus,
) -> Result<CleanupRecoveryDiagnosticCensus, CleanupRecoveryDiagnosticCensusError> {
    let phases = census.running_count.checked_add(census.recovering_count);
    let comparable = census
        .same_host_current_boot_count
        .checked_add(census.same_host_prior_boot_count)
        .and_then(|count| count.checked_add(census.foreign_host_count));
    let classified = comparable
        .and_then(|count| count.checked_add(census.stored_unproven_count))
        .and_then(|count| count.checked_add(census.current_context_unavailable_count));
    if census.active_total > MAX_CLEANUP_RECOVERY_DIAGNOSTIC_CENSUS_ROWS
        || phases != Some(census.active_total)
        || classified != Some(census.active_total)
        || (census.current_context_unavailable_count > 0 && comparable != Some(0))
        || (census.has_more && census.active_total != MAX_CLEANUP_RECOVERY_DIAGNOSTIC_CENSUS_ROWS)
    {
        return Err(CleanupRecoveryDiagnosticCensusError::CorruptData);
    }
    Ok(CleanupRecoveryDiagnosticCensus::new(
        census.active_total,
        census.running_count,
        census.recovering_count,
        census.same_host_current_boot_count,
        census.same_host_prior_boot_count,
        census.foreign_host_count,
        census.stored_unproven_count,
        census.current_context_unavailable_count,
        census.has_more,
    ))
}

fn storage_thief_group_order(
    left: &StoredStorageThiefGroup,
    right: &StoredStorageThiefGroup,
) -> std::cmp::Ordering {
    compare_storage_thief_rates(
        right.total_observed_regrown_bytes,
        right.total_regrowth_duration,
        left.total_observed_regrown_bytes,
        left.total_regrowth_duration,
    )
    .then_with(|| {
        right
            .successful_cleanup_count
            .cmp(&left.successful_cleanup_count)
    })
    .then_with(|| {
        right
            .observed_regrowth_cycle_count
            .cmp(&left.observed_regrowth_cycle_count)
    })
    .then_with(|| right.latest_regrowth_at.cmp(&left.latest_regrowth_at))
    .then_with(|| {
        left.latest_rule
            .id()
            .as_str()
            .cmp(right.latest_rule.id().as_str())
    })
}

fn public_storage_thief_group(
    group: StoredStorageThiefGroup,
) -> Result<DurableStorageThiefGroup, StorageThiefError> {
    Ok(DurableStorageThiefGroup::new(
        group.latest_rule,
        u16::try_from(group.observed_revision_count).map_err(|_| StorageThiefError::CorruptData)?,
        u16::try_from(group.successful_cleanup_count)
            .map_err(|_| StorageThiefError::CorruptData)?,
        u16::try_from(group.successful_manual_cleanup_count)
            .map_err(|_| StorageThiefError::CorruptData)?,
        u16::try_from(group.observed_regrowth_cycle_count)
            .map_err(|_| StorageThiefError::CorruptData)?,
        u16::try_from(group.manual_regrowth_cycle_count)
            .map_err(|_| StorageThiefError::CorruptData)?,
        group.total_observed_regrown_bytes,
        group.total_regrowth_duration,
        group.bytes_regrown_per_day,
        group.rate_capped,
        group.latest_cleanup_at,
        group.latest_regrowth_at,
        group.automation_history_threshold_met,
    ))
}

const fn map_rule_outcome_error(kind: HistoryErrorKind) -> RuleOutcomeError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => RuleOutcomeError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => RuleOutcomeError::QueryLimitExceeded,
        HistoryErrorKind::Busy => RuleOutcomeError::Busy,
        HistoryErrorKind::UnsafeStorage => RuleOutcomeError::UnsafeStorage,
        HistoryErrorKind::CorruptData => RuleOutcomeError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable | HistoryErrorKind::OutcomeUnknown => {
            RuleOutcomeError::Unavailable
        }
        HistoryErrorKind::InternalState => RuleOutcomeError::InternalState,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition => RuleOutcomeError::CorruptData,
    }
}

const fn map_storage_thief_error(kind: HistoryErrorKind) -> StorageThiefError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => StorageThiefError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => StorageThiefError::QueryLimitExceeded,
        HistoryErrorKind::Busy => StorageThiefError::Busy,
        HistoryErrorKind::UnsafeStorage => StorageThiefError::UnsafeStorage,
        HistoryErrorKind::CorruptData => StorageThiefError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable | HistoryErrorKind::OutcomeUnknown => {
            StorageThiefError::Unavailable
        }
        HistoryErrorKind::InternalState => StorageThiefError::InternalState,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition => StorageThiefError::CorruptData,
    }
}

const fn map_automation_schedule_suggestion_error(
    kind: HistoryErrorKind,
) -> AutomationScheduleSuggestionError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => {
            AutomationScheduleSuggestionError::IncompatibleSchema
        }
        HistoryErrorKind::QueryLimitExceeded => {
            AutomationScheduleSuggestionError::QueryLimitExceeded
        }
        HistoryErrorKind::Busy => AutomationScheduleSuggestionError::Busy,
        HistoryErrorKind::UnsafeStorage => AutomationScheduleSuggestionError::UnsafeStorage,
        HistoryErrorKind::CorruptData => AutomationScheduleSuggestionError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable | HistoryErrorKind::OutcomeUnknown => {
            AutomationScheduleSuggestionError::Unavailable
        }
        HistoryErrorKind::InternalState => AutomationScheduleSuggestionError::InternalState,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition => AutomationScheduleSuggestionError::CorruptData,
    }
}

const fn map_running_scan_debt_census_error(kind: HistoryErrorKind) -> RunningScanDebtCensusError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => RunningScanDebtCensusError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => RunningScanDebtCensusError::QueryLimitExceeded,
        HistoryErrorKind::Busy => RunningScanDebtCensusError::Busy,
        HistoryErrorKind::UnsafeStorage => RunningScanDebtCensusError::UnsafeStorage,
        HistoryErrorKind::CorruptData => RunningScanDebtCensusError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable | HistoryErrorKind::OutcomeUnknown => {
            RunningScanDebtCensusError::Unavailable
        }
        HistoryErrorKind::InternalState => RunningScanDebtCensusError::InternalState,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition => RunningScanDebtCensusError::CorruptData,
    }
}

const fn map_claimed_running_scan_provenance_census_error(
    kind: HistoryErrorKind,
) -> ClaimedRunningScanProvenanceCensusError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => {
            ClaimedRunningScanProvenanceCensusError::IncompatibleSchema
        }
        HistoryErrorKind::QueryLimitExceeded => {
            ClaimedRunningScanProvenanceCensusError::QueryLimitExceeded
        }
        HistoryErrorKind::Busy => ClaimedRunningScanProvenanceCensusError::Busy,
        HistoryErrorKind::UnsafeStorage => ClaimedRunningScanProvenanceCensusError::UnsafeStorage,
        HistoryErrorKind::CorruptData => ClaimedRunningScanProvenanceCensusError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable | HistoryErrorKind::OutcomeUnknown => {
            ClaimedRunningScanProvenanceCensusError::Unavailable
        }
        HistoryErrorKind::InternalState => ClaimedRunningScanProvenanceCensusError::InternalState,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition => {
            ClaimedRunningScanProvenanceCensusError::CorruptData
        }
    }
}

const fn map_cleanup_recovery_diagnostic_census_error(
    kind: HistoryErrorKind,
) -> CleanupRecoveryDiagnosticCensusError {
    match kind {
        HistoryErrorKind::IncompatibleSchema => {
            CleanupRecoveryDiagnosticCensusError::IncompatibleSchema
        }
        HistoryErrorKind::QueryLimitExceeded => {
            CleanupRecoveryDiagnosticCensusError::QueryLimitExceeded
        }
        HistoryErrorKind::Busy => CleanupRecoveryDiagnosticCensusError::Busy,
        HistoryErrorKind::UnsafeStorage => CleanupRecoveryDiagnosticCensusError::UnsafeStorage,
        HistoryErrorKind::CorruptData => CleanupRecoveryDiagnosticCensusError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable | HistoryErrorKind::OutcomeUnknown => {
            CleanupRecoveryDiagnosticCensusError::Unavailable
        }
        HistoryErrorKind::InternalState => CleanupRecoveryDiagnosticCensusError::InternalState,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition => CleanupRecoveryDiagnosticCensusError::CorruptData,
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

fn public_owned_storage_usage(usage: StoredOwnedStorageUsage) -> DuxOwnedStorageUsage {
    DuxOwnedStorageUsage {
        logical_bytes: usage.logical_bytes,
        allocated_bytes: usage.allocated_bytes,
        charged_bytes: usage.charged_bytes,
    }
}

fn public_owned_storage_footprint(
    footprint: StoredDuxOwnedStorageFootprint,
    managed_scan_cache: DuxManagedScanCacheFootprint,
    legacy_external_snapshot_stages: LegacyExternalSnapshotStageCensus,
) -> Result<DuxOwnedStorageFootprint, DuxOwnedStorageFootprintError> {
    let snapshots = footprint.snapshots;
    let database = public_owned_storage_usage(footprint.database);
    let snapshot_total = public_owned_storage_usage(snapshots.total);
    let database_and_snapshots = checked_public_storage_usage_add(database, snapshot_total)?;
    if database_and_snapshots != public_owned_storage_usage(footprint.physical_total) {
        return Err(DuxOwnedStorageFootprintError::InternalState);
    }
    let physical_total =
        checked_public_storage_usage_add(database_and_snapshots, managed_scan_cache.total)?;
    if legacy_external_snapshot_stages.inspected_parent_entry_count
        > LEGACY_EXTERNAL_SNAPSHOT_STAGE_CENSUS_MAX_ENTRIES
        || legacy_external_snapshot_stages.stage_shaped_entry_count
            > legacy_external_snapshot_stages.inspected_parent_entry_count
    {
        return Err(DuxOwnedStorageFootprintError::InternalState);
    }
    Ok(DuxOwnedStorageFootprint {
        observed_at: footprint.observed_at,
        database,
        snapshots: DuxSnapshotStorageFootprint {
            cap_bytes: snapshots.cap_bytes,
            cap_excess_bytes: snapshots.cap_excess_bytes,
            controls: public_owned_storage_usage(snapshots.controls),
            available: public_owned_storage_usage(snapshots.available),
            protected: public_owned_storage_usage(snapshots.protected),
            retention_eligible: public_owned_storage_usage(snapshots.retention_eligible),
            tombstoned_residual: public_owned_storage_usage(snapshots.tombstoned_residual),
            orphan: public_owned_storage_usage(snapshots.orphan),
            temporary_active: public_owned_storage_usage(snapshots.temporary_active),
            temporary_quiescent: public_owned_storage_usage(snapshots.temporary_quiescent),
            temporary_unleased: public_owned_storage_usage(snapshots.temporary_unleased),
            total: public_owned_storage_usage(snapshots.total),
            available_count: snapshots.available_count,
            protected_count: snapshots.protected_count,
            retention_eligible_count: snapshots.retention_eligible_count,
            tombstoned_residual_count: snapshots.tombstoned_residual_count,
            orphan_count: snapshots.orphan_count,
            active_temporary_count: snapshots.active_temporary_count,
            quiescent_temporary_count: snapshots.quiescent_temporary_count,
            unleased_temporary_count: snapshots.unleased_temporary_count,
            residual_temporary_lease_count: snapshots.residual_temporary_lease_count,
            active_pin_rows: snapshots.active_pin_rows,
            expired_pin_rows: snapshots.expired_pin_rows,
            non_evictable_over_cap: snapshots.non_evictable_over_cap,
            accounting_unstable: snapshots.accounting_unstable,
        },
        managed_scan_cache,
        legacy_external_snapshot_stages: DuxLegacyExternalSnapshotStageCensus {
            inspected_parent_entry_count: legacy_external_snapshot_stages
                .inspected_parent_entry_count,
            stage_shaped_entry_count: legacy_external_snapshot_stages.stage_shaped_entry_count,
            inspection_complete: legacy_external_snapshot_stages.inspection_complete,
        },
        embedded_ai_cache: DuxEmbeddedAiCacheFootprint {
            record_count: footprint.embedded_ai_cache.record_count,
            logical_content_bytes: footprint.embedded_ai_cache.logical_content_bytes,
            expired_record_count: footprint.embedded_ai_cache.expired_record_count,
            expired_logical_content_bytes: footprint
                .embedded_ai_cache
                .expired_logical_content_bytes,
        },
        physical_total,
    })
}

fn checked_public_storage_usage_add(
    left: DuxOwnedStorageUsage,
    right: DuxOwnedStorageUsage,
) -> Result<DuxOwnedStorageUsage, DuxOwnedStorageFootprintError> {
    Ok(DuxOwnedStorageUsage {
        logical_bytes: left
            .logical_bytes
            .checked_add(right.logical_bytes)
            .ok_or(DuxOwnedStorageFootprintError::InternalState)?,
        allocated_bytes: left
            .allocated_bytes
            .checked_add(right.allocated_bytes)
            .ok_or(DuxOwnedStorageFootprintError::InternalState)?,
        charged_bytes: left
            .charged_bytes
            .checked_add(right.charged_bytes)
            .ok_or(DuxOwnedStorageFootprintError::InternalState)?,
    })
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

fn public_automation_global_control(
    control: StoredAutomationGlobalControl,
) -> AutomationGlobalControl {
    AutomationGlobalControl {
        enabled: control.enabled,
        source: match control.source {
            StoredAutomationGlobalControlSource::Default => AutomationGlobalControlSource::Default,
            StoredAutomationGlobalControlSource::Stored => AutomationGlobalControlSource::Stored,
        },
        revision: control.revision,
        updated_at: control.updated_at,
    }
}

fn public_automation_global_control_update(
    update: StoredAutomationGlobalControlUpdate,
) -> AutomationGlobalControlUpdate {
    AutomationGlobalControlUpdate {
        control: public_automation_global_control(update.control),
        changed: update.changed,
    }
}

fn public_automation_schedule_update(
    update: AutomationScheduleDraftStoreUpdate,
) -> AutomationScheduleUpdate {
    AutomationScheduleUpdate {
        schedule: update.draft,
        changed: update.changed,
    }
}

fn generate_automation_schedule_id() -> Result<AutomationScheduleId, AutomationScheduleDraftError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| AutomationScheduleDraftError::InternalState)?;
    let mut value = String::with_capacity("automation:".len() + random.len() * 2);
    value.push_str("automation:");
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}")
            .map_err(|_| AutomationScheduleDraftError::InternalState)?;
    }
    AutomationScheduleId::new(value).map_err(|_| AutomationScheduleDraftError::InternalState)
}

const fn map_automation_schedule_error(kind: HistoryErrorKind) -> AutomationScheduleDraftError {
    match kind {
        HistoryErrorKind::InvalidInput => AutomationScheduleDraftError::InvalidClock,
        HistoryErrorKind::AlreadyExists => AutomationScheduleDraftError::InternalState,
        HistoryErrorKind::NotFound => AutomationScheduleDraftError::NotFound,
        HistoryErrorKind::InvalidTransition => AutomationScheduleDraftError::RevisionConflict,
        HistoryErrorKind::IncompatibleSchema => AutomationScheduleDraftError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => AutomationScheduleDraftError::QueryLimitExceeded,
        HistoryErrorKind::Busy => AutomationScheduleDraftError::Busy,
        HistoryErrorKind::UnsafeStorage => AutomationScheduleDraftError::UnsafeStorage,
        HistoryErrorKind::CorruptData => AutomationScheduleDraftError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => AutomationScheduleDraftError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => AutomationScheduleDraftError::OutcomeUnknown,
        HistoryErrorKind::InternalState => AutomationScheduleDraftError::InternalState,
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

fn public_configured_project_roots(
    setting: ConfiguredProjectRootSetting,
) -> ConfiguredProjectRoots {
    ConfiguredProjectRoots {
        roots: setting.roots,
        source: match setting.source {
            ConfiguredProjectRootSettingSource::Default => ConfiguredProjectRootsSource::Default,
            ConfiguredProjectRootSettingSource::Stored => ConfiguredProjectRootsSource::Stored,
        },
        revision: setting.revision,
        updated_at: setting.updated_at,
    }
}

fn public_configured_project_roots_update(
    update: ConfiguredProjectRootSettingUpdate,
) -> ConfiguredProjectRootsUpdate {
    ConfiguredProjectRootsUpdate {
        settings: public_configured_project_roots(update.settings),
        changed: update.changed,
    }
}

const fn map_configured_project_roots_error(kind: HistoryErrorKind) -> ConfiguredProjectRootsError {
    match kind {
        HistoryErrorKind::InvalidInput => ConfiguredProjectRootsError::InvalidClock,
        HistoryErrorKind::InvalidTransition => ConfiguredProjectRootsError::RevisionExhausted,
        HistoryErrorKind::IncompatibleSchema => ConfiguredProjectRootsError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => ConfiguredProjectRootsError::QueryLimitExceeded,
        HistoryErrorKind::Busy => ConfiguredProjectRootsError::Busy,
        HistoryErrorKind::UnsafeStorage => ConfiguredProjectRootsError::UnsafeStorage,
        HistoryErrorKind::CorruptData => ConfiguredProjectRootsError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => ConfiguredProjectRootsError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => ConfiguredProjectRootsError::OutcomeUnknown,
        HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InternalState => ConfiguredProjectRootsError::InternalState,
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

const fn map_owned_storage_footprint_error(
    kind: SnapshotRepositoryErrorKind,
) -> DuxOwnedStorageFootprintError {
    match kind {
        SnapshotRepositoryErrorKind::ReadOnly => DuxOwnedStorageFootprintError::IncompatibleSchema,
        SnapshotRepositoryErrorKind::MissingStore
        | SnapshotRepositoryErrorKind::MissingSnapshot
        | SnapshotRepositoryErrorKind::SnapshotUnavailable => {
            DuxOwnedStorageFootprintError::Unavailable
        }
        SnapshotRepositoryErrorKind::IncompatibleVersion
        | SnapshotRepositoryErrorKind::ReferenceMismatch
        | SnapshotRepositoryErrorKind::ReviewLeaseExpired
        | SnapshotRepositoryErrorKind::Codec(_) => DuxOwnedStorageFootprintError::CorruptData,
        SnapshotRepositoryErrorKind::Storage(kind) => match kind {
            SnapshotStorageErrorKind::UnsafeRoot
            | SnapshotStorageErrorKind::UnsafeObject
            | SnapshotStorageErrorKind::UnrecognizedStore => {
                DuxOwnedStorageFootprintError::UnsafeStorage
            }
            SnapshotStorageErrorKind::Busy => DuxOwnedStorageFootprintError::Busy,
            SnapshotStorageErrorKind::Unavailable => DuxOwnedStorageFootprintError::Unavailable,
            SnapshotStorageErrorKind::InvalidConfiguration
            | SnapshotStorageErrorKind::InternalState => {
                DuxOwnedStorageFootprintError::InternalState
            }
        },
        SnapshotRepositoryErrorKind::History(kind) => match kind {
            HistoryErrorKind::InvalidInput => DuxOwnedStorageFootprintError::InvalidClock,
            HistoryErrorKind::IncompatibleSchema => {
                DuxOwnedStorageFootprintError::IncompatibleSchema
            }
            HistoryErrorKind::QueryLimitExceeded => DuxOwnedStorageFootprintError::BudgetExceeded,
            HistoryErrorKind::Busy => DuxOwnedStorageFootprintError::Busy,
            HistoryErrorKind::UnsafeStorage => DuxOwnedStorageFootprintError::UnsafeStorage,
            HistoryErrorKind::CorruptData => DuxOwnedStorageFootprintError::CorruptData,
            HistoryErrorKind::DatabaseUnavailable => DuxOwnedStorageFootprintError::Unavailable,
            HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::NotFound
            | HistoryErrorKind::InvalidTransition
            | HistoryErrorKind::OutcomeUnknown
            | HistoryErrorKind::InternalState => DuxOwnedStorageFootprintError::InternalState,
        },
    }
}

const fn map_owned_storage_footprint_history_error(
    kind: HistoryErrorKind,
) -> DuxOwnedStorageFootprintError {
    match kind {
        HistoryErrorKind::InvalidInput => DuxOwnedStorageFootprintError::InvalidClock,
        HistoryErrorKind::IncompatibleSchema => DuxOwnedStorageFootprintError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => DuxOwnedStorageFootprintError::BudgetExceeded,
        HistoryErrorKind::Busy => DuxOwnedStorageFootprintError::Busy,
        HistoryErrorKind::UnsafeStorage => DuxOwnedStorageFootprintError::UnsafeStorage,
        HistoryErrorKind::CorruptData => DuxOwnedStorageFootprintError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => DuxOwnedStorageFootprintError::Unavailable,
        HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::OutcomeUnknown
        | HistoryErrorKind::InternalState => DuxOwnedStorageFootprintError::InternalState,
    }
}

const fn map_owned_storage_footprint_cache_error(
    error: DuxOwnedStorageFootprintCacheError,
) -> DuxOwnedStorageFootprintError {
    match error {
        DuxOwnedStorageFootprintCacheError::Busy => DuxOwnedStorageFootprintError::Busy,
        DuxOwnedStorageFootprintCacheError::UnsafeStorage => {
            DuxOwnedStorageFootprintError::UnsafeStorage
        }
        DuxOwnedStorageFootprintCacheError::BudgetExceeded => {
            DuxOwnedStorageFootprintError::BudgetExceeded
        }
        DuxOwnedStorageFootprintCacheError::CorruptData => {
            DuxOwnedStorageFootprintError::CorruptData
        }
        DuxOwnedStorageFootprintCacheError::Unavailable => {
            DuxOwnedStorageFootprintError::Unavailable
        }
        DuxOwnedStorageFootprintCacheError::InternalState => {
            DuxOwnedStorageFootprintError::InternalState
        }
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
    origin: ScanTaskOrigin,
    max_nodes: usize,
    excluded_subtrees: Vec<PathBuf>,
    trusted_home_mount: Option<TrustedHomeMountWitness>,
    targeted_root_kind: Option<TargetedReclaimRootKind>,
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
        origin,
        max_nodes,
        excluded_subtrees,
        trusted_home_mount,
        targeted_root_kind,
    } = admitted;
    if !prepare_scan_root(&admitted_root).is_ok_and(|(observed_root, observed_identity)| {
        observed_root == admitted_root && Some(observed_identity) == expected_root_identity
    }) || !current_root_matches(&admitted_root, expected_root_identity)
        || trusted_home_mount
            .as_ref()
            .is_some_and(|witness| witness.revalidate().is_err())
    {
        return WorkOutcome::Failed(TaskFailureKind::ScanRootChanged, None);
    }
    let Some(admitted_root_identity) = expected_root_identity else {
        return WorkOutcome::Failed(TaskFailureKind::ScanRootChanged, None);
    };
    let start = match start_durable_scan(
        &store,
        &admitted_root,
        admitted_root_identity,
        origin,
        targeted_root_kind,
    ) {
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
    if trusted_home_mount
        .as_ref()
        .is_some_and(|witness| witness.revalidate().is_err())
    {
        return settle_changed_scan_root(&mut durable, start.started_at());
    }

    let scanner = Scanner::new(ScanConfig {
        follow_symlinks: false,
        max_depth: None,
        max_nodes: Some(max_nodes),
        excluded_subtrees,
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
                || trusted_home_mount
                    .as_ref()
                    .is_some_and(|witness| witness.revalidate().is_err())
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
                match prepare_candidate_evaluation(
                    &context,
                    start.id(),
                    &artifact,
                    completed_at,
                    match targeted_root_kind {
                        Some(TargetedReclaimRootKind::KnownUserLibraryCaches) => {
                            CandidateEvaluationScope::UserCacheDirectory
                        }
                        Some(TargetedReclaimRootKind::ConfiguredProject) | None => {
                            CandidateEvaluationScope::SelectedScanRoot
                        }
                    },
                ) {
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
            if !current_root_matches(&admitted_root, expected_root_identity)
                || trusted_home_mount
                    .as_ref()
                    .is_some_and(|witness| witness.revalidate().is_err())
            {
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
    'worker: loop {
        let job = {
            let mut registry = shared.lock_registry_recover();
            loop {
                if registry.lifecycle != EngineLifecycle::Open {
                    if let Some(job) = registry.queue.pop_front() {
                        drop(registry);
                        drop(job);
                        continue 'worker;
                    }
                    registry.live_workers = registry.live_workers.saturating_sub(1);
                    if registry.live_workers == 0 {
                        drop(
                            shared
                                .reset_engine_lease
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .take(),
                        );
                        registry.lifecycle = EngineLifecycle::Closed;
                        shared.lifecycle_changed.notify_all();
                    }
                    return;
                }
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

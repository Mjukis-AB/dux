//! Private, versioned SQLite storage owned by the shared engine.
//!
//! Raw SQL and connections remain private. Typed history values are
//! presentation observations only and never cleanup authority.

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the private reset coordinator is intentionally dormant until the engine lifecycle slice"
    )
)]
mod app_data_reset;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "reset store admission is consumed by the namespace handoff slice"
    )
)]
mod app_data_reset_blocker;
mod candidate_evaluation_history;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "typed candidate persistence is integrated by the later evaluator task slice"
    )
)]
mod candidate_history;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "typed capacity persistence integrates with the volume monitor in a later slice"
    )
)]
mod capacity_history;
mod cargo_enrollment;
mod cleanup_exclusions;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "typed cleanup history is integrated by the later planner/executor slices"
    )
)]
mod cleanup_history;
mod cleanup_history_clear;
mod cleanup_history_query;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "cleanup journal state integrates with the engine executor in a later slice"
    )
)]
mod cleanup_journal;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the frozen v1 byte codec is consumed by the next typed persistence slice"
    )
)]
mod codec;
mod configured_project_roots;
mod footprint;
mod history;
mod migrations;
mod permanent_cleanup;
mod pressure_settings;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "process-instance liveness is consumed by the next fenced journal-transition slice"
    )
)]
mod process_liveness;
mod retention;
mod rule_outcome;
mod running_scan_debt;
mod scan_coverage_history;
mod scan_process_claim;
mod scan_scope_lease;
mod settings;
pub(crate) mod snapshot;
mod snapshot_retention;
mod snapshot_retention_inventory;
mod snapshot_review_pin;
mod snapshot_temp_lease;
mod snapshot_terminal_temp_inventory;
mod snapshot_unleased_temp_inventory;
mod status;
mod storage;
mod storage_thief;
mod store;

pub(crate) use app_data_reset::{
    AppDataResetAdmittedStoreOutcome, AppDataResetCacheDrainAuthority,
    AppDataResetCacheStageRetireAuthority, AppDataResetCoordinator, AppDataResetCoordinatorError,
    AppDataResetCoordinatorErrorKind, AppDataResetCoordinatorSession, AppDataResetEngineLease,
    AppDataResetEngineLeaseOutcome, AppDataResetJournal,
    AppDataResetOldDatabasePayloadDrainAuthority, AppDataResetPhase, AppDataResetRecoveryIntent,
    AppDataResetSnapshotPayloadDrainAuthority, AppDataResetSnapshotStoreRetireAuthority,
    AppDataResetStoreIdentity,
};
#[cfg(test)]
pub(crate) use app_data_reset::{
    TestAppDataResetCoordinatorPostcheckFault, TestJournalWriteFault,
    set_test_app_data_reset_coordinator_postcheck_fault, set_test_journal_write_fault,
};
pub(crate) use app_data_reset_blocker::AppDataResetStoreBlockers;
pub(crate) use candidate_evaluation_history::{
    CandidateEvaluationCompletion, CandidateEvaluationFailureKind, CandidateEvaluationIdentity,
    CandidateEvaluationObservation, CandidateEvaluationRecord, CandidateEvaluationStatus,
    CandidateValidationSourceRecord,
};
#[cfg(test)]
pub(crate) use candidate_history::StoredCandidateRecord;
pub(crate) use candidate_history::{
    CandidateBatchMaterializationBudget, CandidateHistoryStatus, CompleteCandidateRecord,
    NewCandidateRecord,
};
#[allow(
    unused_imports,
    reason = "pressure episode pages are consumed by later trend and notification slices"
)]
pub(crate) use capacity_history::{
    CapacityChange, CapacityPressureBaseline, CapacityTrend, CapacityTrendPoint,
    CapacityTrendPointSource, CapacityWriteOutcome, RawCapacityObservation, StoredPressureEpisode,
};
pub(crate) use cargo_enrollment::{
    CARGO_CODE_SIGN_ADHOC_FLAG, CARGO_ENROLLMENT_SUPPORTED_RELEASE,
    CARGO_SIGNATURE_POLICY_REVISION, CargoCodeSignatureRecord, CargoEnrollmentSetting,
    CargoEnrollmentSettingUpdate, CargoEnrollmentState, CargoExecutableEnrollmentIdentity,
    CargoSignatureClass,
};
pub(crate) use cleanup_exclusions::{
    CleanupExclusionSetting, CleanupExclusionSettingSource, CleanupExclusionSettingUpdate,
    load_cleanup_exclusions,
};
pub(crate) use cleanup_history::{CleanupSessionId, canonical_started_at};
pub(crate) use cleanup_history::{CleanupTrigger, NewCleanupSessionRecord};
pub(crate) use cleanup_history_clear::{
    CleanupHistoryClearStoreError, PreparedCleanupHistoryClear,
};
pub(crate) use cleanup_history_query::{
    StoredCleanupErrorCategory, StoredCleanupHistoryCursor, StoredCleanupHistoryObservation,
    StoredCleanupItemStatus, StoredCleanupItemSummary, StoredCleanupMode,
    StoredCleanupRecordFormat, StoredCleanupSessionStatus, StoredCleanupSessionSummary,
    StoredCleanupStatusCounts, StoredCleanupTrigger,
};
pub(crate) use cleanup_journal::{
    CleanupJournalClaim, CleanupJournalLease, DryRunJournalFailure, EffectOutcome,
    EffectStartReceipt, JournalLeaseFailure, TerminalSessionStatus, ValidatedDryRunOutcome,
    ValidationOutcome,
};
#[cfg(test)]
pub(crate) use cleanup_journal::{
    fail_next_write_after_commit_and_reconcile_read_for_test,
    fail_next_write_after_commit_and_two_reconcile_reads_for_test,
};
pub(crate) use codec::{HostPathObservationEncoding, observe_host_path};
pub(crate) use configured_project_roots::{
    ConfiguredProjectRootSetting, ConfiguredProjectRootSettingSource,
    ConfiguredProjectRootSettingUpdate, validate_configured_project_roots,
};
pub(crate) use footprint::{DuxOwnedStorageFootprint, OwnedStorageUsage};
pub(crate) use history::{
    HistoryError, HistoryErrorKind, MAX_RECENT_SCAN_HISTORY_LIMIT, NewScanRecord,
    ScanCompletionRecord, ScanCounts, ScanRecord, ScanStatus, TerminalScanStatus,
};
pub(crate) use permanent_cleanup::{
    PermanentCleanupSetting, PermanentCleanupSettingSource, PermanentCleanupSettingUpdate,
    load_permanent_cleanup_setting,
};
pub(crate) use pressure_settings::{
    DiskPressurePolicySetting, DiskPressurePolicySettingSource, DiskPressurePolicySettingUpdate,
};
pub(crate) use rule_outcome::{
    StoredRuleOutcome, StoredRuleOutcomeBatch, StoredRuleOutcomeNotEligibleReason,
    StoredRuleOutcomeState,
};
pub(crate) use running_scan_debt::RunningScanDebtCensus;
pub(crate) use scan_process_claim::{
    ClaimedRunningScanProvenanceCensus, ScanRecoveryBatchOutcome, ScanRecoveryBatchResult,
};
#[allow(
    unused_imports,
    reason = "the engine integration consumes the new lease error in the adjacent slice"
)]
pub(crate) use scan_scope_lease::{
    ScanScopeLeaseError, ScanScopeLeaseErrorKind, ScanScopeLeaseToken,
};
pub(crate) use settings::{
    SnapshotRetentionCapSetting, SnapshotRetentionCapSettingSource,
    SnapshotRetentionCapSettingUpdate,
};
pub use snapshot::{SNAPSHOT_FORMAT_VERSION, SnapshotOpenErrorKind};
pub(crate) use snapshot_review_pin::SnapshotReviewPurpose;
pub use status::{DATABASE_SCHEMA_VERSION, DatabaseAccess, DatabaseOpenErrorKind, DatabaseStatus};
#[cfg(test)]
pub(crate) use storage::{
    AppDataResetOldDatabasePayloadDrainFault, TestAppDataResetDataDetachFault,
    TestAppDataResetFreshNamespaceFault, TestAppDataResetSnapshotPostcheckFault,
    set_test_app_data_reset_data_detach_fault, set_test_app_data_reset_fresh_namespace_fault,
    set_test_app_data_reset_old_database_payload_drain_fault,
    set_test_app_data_reset_snapshot_postcheck_fault,
};
pub(crate) use storage_thief::{
    MAX_STORAGE_THIEF_GROUPS, MAX_STORAGE_THIEF_SOURCE_SESSIONS, StoredStorageThiefGroup,
    StoredStorageThiefRanking, compare_storage_thief_rates, storage_thief_rate_per_day,
};
pub(crate) use store::{
    AppDataResetDataNamespaceAdmission, AppDataResetFreshNamespace,
    AppDataResetFreshNamespaceLocation, AppDataResetOldDatabaseDrainingAdmission,
    AppDataResetOldDatabasePayloadState, AppDataResetRecoveryDataLocation,
    AppDataResetRecoveryDataNamespace, AppDataResetSnapshotDrainingAdmission,
};
pub(crate) use store::{AppDataResetStoreGuard, CandidateReviewAction, StoreCoordinator};

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;

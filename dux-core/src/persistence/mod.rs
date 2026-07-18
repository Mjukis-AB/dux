//! Private, versioned SQLite storage owned by the shared engine.
//!
//! Raw SQL and connections remain private. Typed history values are
//! presentation observations only and never cleanup authority.

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
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "typed cleanup history is integrated by the later planner/executor slices"
    )
)]
mod cleanup_history;
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
mod history;
mod migrations;
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
mod scan_coverage_history;
mod scan_process_claim;
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
mod store;

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
pub(crate) use capacity_history::{
    CapacityPressureBaseline, CapacityWriteOutcome, RawCapacityObservation,
};
pub(crate) use cargo_enrollment::{
    CARGO_CODE_SIGN_ADHOC_FLAG, CARGO_ENROLLMENT_SUPPORTED_RELEASE,
    CARGO_SIGNATURE_POLICY_REVISION, CargoCodeSignatureRecord, CargoEnrollmentSetting,
    CargoEnrollmentSettingUpdate, CargoEnrollmentState, CargoExecutableEnrollmentIdentity,
    CargoSignatureClass,
};
pub(crate) use cleanup_history::CleanupSessionId;
#[cfg(test)]
pub(crate) use cleanup_history::{CleanupTrigger, NewCleanupSessionRecord};
pub(crate) use cleanup_history_query::{
    StoredCleanupErrorCategory, StoredCleanupHistoryCursor, StoredCleanupHistoryObservation,
    StoredCleanupItemStatus, StoredCleanupItemSummary, StoredCleanupMode,
    StoredCleanupRecordFormat, StoredCleanupSessionStatus, StoredCleanupSessionSummary,
    StoredCleanupStatusCounts, StoredCleanupTrigger,
};
pub(crate) use cleanup_journal::{CleanupJournalClaim, EffectStartReceipt, ValidationOutcome};
pub(crate) use codec::{HostPathObservationEncoding, observe_host_path};
pub(crate) use history::{
    HistoryErrorKind, MAX_RECENT_SCAN_HISTORY_LIMIT, NewScanRecord, ScanCompletionRecord,
    ScanCounts, ScanStatus, TerminalScanStatus,
};
pub(crate) use pressure_settings::{
    DiskPressurePolicySetting, DiskPressurePolicySettingSource, DiskPressurePolicySettingUpdate,
};
pub(crate) use scan_process_claim::{ScanRecoveryBatchOutcome, ScanRecoveryBatchResult};
pub(crate) use settings::{
    SnapshotRetentionCapSetting, SnapshotRetentionCapSettingSource,
    SnapshotRetentionCapSettingUpdate,
};
pub use snapshot::SnapshotOpenErrorKind;
pub(crate) use snapshot_review_pin::SnapshotReviewPurpose;
pub use status::{DATABASE_SCHEMA_VERSION, DatabaseAccess, DatabaseOpenErrorKind, DatabaseStatus};
pub(crate) use store::CandidateReviewAction;
pub(crate) use store::StoreCoordinator;

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;

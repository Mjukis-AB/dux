//! Product-neutral discovery and deterministic rule policy types.
//!
//! Candidates are immutable scan findings, not cleanup authorization. Path
//! validation, overlap planning, execution, persistence DTOs, and FFI DTOs live
//! at later boundaries and must not infer authority from these values.

mod automation_eligibility;
mod automation_schedule;
mod automation_scheduler;
mod candidate;
mod candidate_evaluator;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "candidate grouping remains sealed until trusted planner authority exists"
    )
)]
mod candidate_groups;
pub(crate) use candidate_groups::{
    CandidateGroupSet, CandidateGroupingError, CandidateOverlapReason, CandidateOverlapResolution,
    group_candidates as group_candidates_for_review,
};
mod cleanup_plan;
mod cloud_eviction;
mod id;
mod policy;
mod rule;
mod rule_document;
mod scan_coverage;
mod volume;

pub use automation_eligibility::{
    AUTOMATION_CURRENT_EVIDENCE_MAX_AGE, AUTOMATION_ELIGIBILITY_POLICY_REVISION,
    AUTOMATION_REQUIRED_MANUAL_SUCCESSES, AUTOMATION_REQUIRED_RECENT_RUNS,
    AutomationActivityEvidence, AutomationCurrentCandidateEvidence,
    AutomationCurrentValidationEvidence, AutomationDraftPolicyReason,
    AutomationEligibilityAssessment, AutomationEligibilityDecision, AutomationEligibilityGate,
    AutomationEligibilityGateAssessment, AutomationEligibilityGateStatus,
    AutomationEligibilityInput, AutomationEligibilityReason, AutomationManualHistoryEvidence,
    AutomationRecentRunEvidence, AutomationRuntimeIdentityEvidence,
    MAX_AUTOMATION_DRAFT_POLICY_REASONS, assess_automation_eligibility,
};
pub(crate) use automation_eligibility::{
    AutomationDraftPolicyPreflight, assess_automation_draft_policy,
};
pub use automation_schedule::{
    AutomationConfirmationMode, AutomationScheduleCadence, AutomationScheduleConfigError,
    AutomationScheduleDraft, AutomationScheduleDraftConfig, AutomationScheduleScope,
    DEFAULT_AUTOMATION_MAXIMUM_BYTES_PER_RUN, DEFAULT_AUTOMATION_MINIMUM_AGE,
    DEFAULT_AUTOMATION_MINIMUM_RECLAIMABLE_BYTES, DEFAULT_AUTOMATION_PRE_RUN_NOTIFICATIONS,
    MAX_AUTOMATION_SCHEDULE_DRAFTS, MAX_AUTOMATION_SCHEDULE_EXCLUSIONS,
};
pub use automation_scheduler::{
    AUTOMATION_SCHEDULER_DECISION_POLICY_REVISION, AutomationGlobalSwitchEvidence,
    AutomationScheduleActivationEvidence, AutomationScheduleClockObservation,
    AutomationSchedulerAssessment, AutomationSchedulerDecision, AutomationSchedulerDecisionInput,
    AutomationSchedulerDeferredReason, AutomationSchedulerDormantReason,
    AutomationSchedulerInputError, AutomationSchedulerRevalidationObservation,
    AutomationSchedulerRuntimeEvidence, AutomationSchedulerScheduleObservation,
    AutomationSchedulerSignal, AutomationSchedulerTrigger, MAX_AUTOMATION_SCHEDULER_OBSERVATIONS,
    assess_automation_scheduler,
};
#[cfg(test)]
pub(crate) use candidate::CandidateInput;
pub use candidate::{BlockReason, Candidate, CandidateValidationError, Evidence, EvidenceKind};
pub(crate) use candidate_evaluator::{
    CANDIDATE_CATALOG_SCHEMA_VERSION, CANDIDATE_CATALOG_SHA256, CANDIDATE_CONTEXT_FORMAT_VERSION,
    CANDIDATE_EVALUATOR_REVISION, CandidateEvaluationError, CandidateEvaluationScope,
    CandidateSnapshotReplayError, KNOWN_USER_CACHE_SCAN_ID_PREFIX, MAX_EVALUATED_CANDIDATES,
    SAFE_RUST_RULE_MINIMUM_AGE, SAFE_RUST_RULE_REVISION, bundled_automation_draft_policy_preflight,
    bundled_automation_eligible_rule_count, bundled_automation_history_suggestion_rules,
    candidate_evaluation_context_digest_for_observation,
    candidate_evaluation_context_digest_sha256, candidate_from_complete_record,
    current_rust_target_candidate_id, evaluate_completed_scan_candidates,
    replay_snapshot_candidate_evaluation, validate_bundled_candidate_catalog,
    verify_snapshot_candidate_evaluation,
};
pub(crate) use cleanup_plan::CleanupPlanCandidateFacts;
pub use cleanup_plan::{
    CLEANUP_PLAN_VALIDITY, CleanupMode, CleanupPlan, CleanupPlanItem, CleanupPlanValidationError,
    OperationStatus, PlanWarning,
};
pub use cloud_eviction::{
    CloudBooleanState, CloudErrorState, CloudEvictionAssessment, CloudEvictionBlockReason,
    CloudEvictionDiscoveryEvidence, CloudEvictionIdentityBlockReason, CloudEvictionIdentityFacts,
    CloudEvictionItemKind, CloudEvictionObservation, CloudEvictionObservationInput,
    CloudEvictionPlatformFacts, CloudEvictionProvider, CloudIdentityFactState, CloudLocalCopyState,
    assess_cloud_eviction,
};
pub use id::{
    AutomationScheduleId, CandidateId, CleanupPlanId, LocalizedTextKey, RuleId, RuleRef,
    RuleRevision, ScanId, StableIdError, VolumeId,
};
pub use policy::{CandidateAction, CandidateCategory, SafetyTier};
pub use rule::{
    ActivityGuard, ProvenanceUrl, Rule, RuleDefinition, RuleGuards, RuleMatcher,
    RuleMatcherDefinition, RuleScope, RuleValidationError,
};
pub use scan_coverage::{
    CoveragePermille, CoveragePermilleError, ScanCoverage, ScanCoverageStatus, ScanIssue,
    ScanIssueKind,
};
pub(crate) use scan_coverage::{MAX_SCAN_ISSUE_OCCURRENCES, MAX_SCAN_ISSUES};
pub use volume::{
    AvailableCapacitySource, DiskPressure, DiskPressureConfig, DiskPressureConfigError,
    DiskPressureEvaluation, DiskPressureRecoveryMargin, DiskPressureThreshold, VolumeCapacity,
    VolumeCapacityError,
};

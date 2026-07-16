//! Product-neutral discovery and deterministic rule policy types.
//!
//! Candidates are immutable scan findings, not cleanup authorization. Path
//! validation, overlap planning, execution, persistence DTOs, and FFI DTOs live
//! at later boundaries and must not infer authority from these values.

mod candidate;
mod cleanup_plan;
mod id;
mod policy;
mod rule;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the validated catalog loader remains internal until a signed bundled-rule source exists"
    )
)]
mod rule_document;
mod scan_coverage;
mod volume;

#[cfg(test)]
pub(crate) use candidate::CandidateInput;
pub use candidate::{BlockReason, Candidate, CandidateValidationError, Evidence, EvidenceKind};
pub use cleanup_plan::{
    CLEANUP_PLAN_VALIDITY, CleanupMode, CleanupPlan, CleanupPlanItem, CleanupPlanValidationError,
    OperationStatus, PlanWarning,
};
pub use id::{
    CandidateId, CleanupPlanId, LocalizedTextKey, RuleId, RuleRef, RuleRevision, ScanId,
    StableIdError, VolumeId,
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
pub use volume::DiskPressure;

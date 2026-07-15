//! Product-neutral discovery and deterministic rule policy types.
//!
//! Candidates are immutable scan findings, not cleanup authorization. Path
//! validation, overlap planning, execution, persistence DTOs, and FFI DTOs live
//! at later boundaries and must not infer authority from these values.

mod candidate;
mod id;
mod policy;
mod rule;

pub use candidate::{BlockReason, Candidate, CandidateValidationError, Evidence, EvidenceKind};
pub use id::{CandidateId, LocalizedTextKey, RuleId, RuleRef, RuleRevision, ScanId, StableIdError};
pub use policy::{CandidateAction, CandidateCategory, SafetyTier};
pub use rule::{
    ActivityGuard, ProvenanceUrl, Rule, RuleDefinition, RuleGuards, RuleMatcher,
    RuleMatcherDefinition, RuleScope, RuleValidationError,
};

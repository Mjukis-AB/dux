//! Private admission of a blocked Rust target into the next planner stage.
//!
//! This token deliberately does not remove the discovery blocker, create a
//! plan, approve anything, cross FFI, schedule work, or mutate the filesystem.
//! It only retains the immutable blocked candidate together with the
//! revalidated Cargo/home/protected-root authority for a later typed planner
//! boundary.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "promotion token is consumed by the subsequent typed planner slice"
    )
)]

use thiserror::Error;

use crate::domain::{
    BlockReason, Candidate, CandidateAction, CandidateCategory, CleanupMode, CleanupPlan,
    CleanupPlanCandidateFacts, CleanupPlanId, CleanupPlanValidationError, Evidence, SafetyTier,
    ScanId, current_rust_target_candidate_id,
};
use crate::path_validation::{CanonicalPathSnapshot, CanonicalScanRoot};

use super::rule_scope_grant::{RuleScopeAuthorization, RuleScopeGrantError};

const RUST_TARGET_RULE: &str = "developer.rust.target";
const RUST_TARGET_REVISION: u32 = 2;

#[must_use = "the private Rust-target promotion must be consumed by a later planner boundary"]
pub(super) struct RustTargetPromotion {
    candidate: Candidate,
    authorization: RuleScopeAuthorization,
}

/// Planner-owned, path-private facts for a future Rust-target plan
/// constructor. This capability still retains the original blocked candidate;
/// it is not a plan, approval, journal claim, schedule, FFI value, or effect
/// witness.
#[must_use = "Rust-target plan facts must be consumed by the next planner boundary"]
pub(super) struct RustTargetPlanFacts {
    promotion: RustTargetPromotion,
    scan_root: CanonicalScanRoot,
    target: CanonicalPathSnapshot,
}

#[derive(Debug, Error)]
pub(super) enum RustTargetPromotionError {
    #[error("candidate is not the exact staged Rust-target policy")]
    CandidatePolicy,
    #[error("candidate source scan does not match the supplied scan")]
    SourceScanMismatch,
    #[error("candidate must retain exactly the ProtectedPath blocker")]
    ProtectedPathBlocker,
    #[error("candidate does not contain the exact default Cargo target evidence")]
    CandidateEvidence,
    #[error("candidate ID does not match the staged Rust target layout")]
    CandidateId,
    #[error("candidate facts do not match the retained durable discovery record")]
    DurableCandidateMismatch,
    #[error("candidate target path does not match the reviewed live target")]
    TargetMismatch,
    #[error("trusted Rust-target authorization failed: {0}")]
    Authorization(#[source] RuleScopeGrantError),
    #[error("trusted Rust-target plan construction failed: {0}")]
    Plan(#[source] CleanupPlanValidationError),
    #[error("Rust-target plan facts require permanent-safe mode")]
    UnsupportedPlanMode,
}

/// Admit one exact durable candidate only after the code-owned authorization
/// has revalidated its source scan, target snapshot, Cargo provenance, mount,
/// and protected-root policy. The candidate remains blocked inside the token.
pub(super) fn admit_rust_target_candidate(
    candidate: Candidate,
    authorization: RuleScopeAuthorization,
    source_scan_id: &ScanId,
    scan_root: &CanonicalScanRoot,
    target: &CanonicalPathSnapshot,
) -> Result<RustTargetPromotion, RustTargetPromotionError> {
    validate_candidate_shape(&candidate, source_scan_id, target)?;
    authorization
        .revalidate_for_candidate_binding(source_scan_id, candidate.id(), scan_root, target)
        .map_err(RustTargetPromotionError::Authorization)?;
    if !authorization.matches_durable_candidate(&candidate) {
        return Err(RustTargetPromotionError::DurableCandidateMismatch);
    }
    Ok(RustTargetPromotion {
        candidate,
        authorization,
    })
}

fn validate_candidate_shape(
    candidate: &Candidate,
    source_scan_id: &ScanId,
    target: &CanonicalPathSnapshot,
) -> Result<(), RustTargetPromotionError> {
    if candidate.source_scan_id() != source_scan_id {
        return Err(RustTargetPromotionError::SourceScanMismatch);
    }
    if candidate.rule().id().as_str() != RUST_TARGET_RULE
        || candidate.rule().revision().get() != RUST_TARGET_REVISION
        || candidate.category() != CandidateCategory::DeveloperArtifact
        || candidate.safety() != SafetyTier::SafeRegenerable
        || candidate.action() != CandidateAction::RemoveKnownRegenerableContents
        || candidate.rule_marks_schedule_eligible()
    {
        return Err(RustTargetPromotionError::CandidatePolicy);
    }
    if candidate.blockers() != [BlockReason::ProtectedPath] {
        return Err(RustTargetPromotionError::ProtectedPathBlocker);
    }
    let [candidate_path] = candidate.paths() else {
        return Err(RustTargetPromotionError::CandidateEvidence);
    };
    if candidate_path != target.requested_path() {
        return Err(RustTargetPromotionError::TargetMismatch);
    }
    let expected_manifest = candidate_path
        .parent()
        .map(|parent| parent.join("Cargo.toml"))
        .ok_or(RustTargetPromotionError::CandidateEvidence)?;
    let expected_cache_tag = candidate_path.join("CACHEDIR.TAG");
    let mut matched_target = false;
    let mut matched_manifest = false;
    let mut matched_cache_tag = false;
    for evidence in candidate.evidence() {
        match evidence {
            Evidence::MatchedPath { path } if path == candidate_path && !matched_target => {
                matched_target = true;
            }
            Evidence::RequiredMarker { path }
                if path == &expected_manifest && !matched_manifest =>
            {
                matched_manifest = true;
            }
            Evidence::RequiredMarker { path }
                if path == &expected_cache_tag && !matched_cache_tag =>
            {
                matched_cache_tag = true;
            }
            _ => return Err(RustTargetPromotionError::CandidateEvidence),
        }
    }
    if candidate.evidence().len() != 3 || !matched_target || !matched_manifest || !matched_cache_tag
    {
        return Err(RustTargetPromotionError::CandidateEvidence);
    }
    let expected_id = current_rust_target_candidate_id(source_scan_id, candidate_path)
        .map_err(|_| RustTargetPromotionError::CandidateId)?;
    if candidate.id() != &expected_id {
        return Err(RustTargetPromotionError::CandidateId);
    }
    Ok(())
}

impl RustTargetPromotion {
    /// Consume the promotion token into typed plan facts after one immediate
    /// grant revalidation. The candidate remains blocked inside the retained
    /// promotion and cannot be converted to a domain plan by this operation.
    pub(super) fn into_plan_facts(
        self,
        scan_root: CanonicalScanRoot,
        target: CanonicalPathSnapshot,
    ) -> Result<RustTargetPlanFacts, RustTargetPromotionError> {
        self.revalidate(&scan_root, &target)
            .map_err(RustTargetPromotionError::Authorization)?;
        Ok(RustTargetPlanFacts {
            promotion: self,
            scan_root,
            target,
        })
    }

    pub(super) fn revalidate(
        &self,
        scan_root: &CanonicalScanRoot,
        target: &CanonicalPathSnapshot,
    ) -> Result<(), RuleScopeGrantError> {
        self.authorization.revalidate_for_candidate_binding(
            self.candidate.source_scan_id(),
            self.candidate.id(),
            scan_root,
            target,
        )
    }

    pub(super) fn release(self) {}
}

impl RustTargetPlanFacts {
    /// Consume typed facts into the first domain cleanup plan representation.
    /// The returned authorization remains paired with the plan for the later
    /// reviewed/approved executor handoff; this method still performs no
    /// journal write, FFI call, scheduling, or filesystem effect.
    pub(super) fn into_trusted_permanent_plan(
        self,
        plan_id: CleanupPlanId,
        created_at: std::time::SystemTime,
        mode: CleanupMode,
    ) -> Result<(CleanupPlan, RuleScopeAuthorization), RustTargetPromotionError> {
        if mode != CleanupMode::PermanentSafe {
            return Err(RustTargetPromotionError::UnsupportedPlanMode);
        }
        self.revalidate()?;
        let RustTargetPlanFacts {
            promotion:
                RustTargetPromotion {
                    candidate,
                    authorization,
                },
            ..
        } = self;
        if candidate.blockers() != [BlockReason::ProtectedPath] {
            return Err(RustTargetPromotionError::ProtectedPathBlocker);
        }
        let facts = CleanupPlanCandidateFacts {
            candidate_id: candidate.id().clone(),
            source_scan_id: candidate.source_scan_id().clone(),
            rule: candidate.rule().clone(),
            category: candidate.category(),
            paths: candidate.paths().to_vec(),
            estimated_bytes: candidate.estimated_bytes(),
            newest_mtime: candidate.newest_mtime(),
            evidence: candidate.evidence().to_vec(),
            safety: candidate.safety(),
            action: candidate.action(),
            rule_schedule_eligible: candidate.rule_marks_schedule_eligible(),
        };
        let plan =
            CleanupPlan::try_from_trusted_candidate_facts(plan_id, created_at, mode, &[facts])
                .map_err(RustTargetPromotionError::Plan)?;
        Ok((plan, authorization))
    }

    pub(super) fn revalidate(&self) -> Result<(), RustTargetPromotionError> {
        self.promotion
            .revalidate(&self.scan_root, &self.target)
            .map_err(RustTargetPromotionError::Authorization)
    }

    pub(super) fn release(self) {
        self.promotion.release();
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    #[cfg(target_os = "macos")]
    use super::super::rule_scope_grant::authorize_rule_target;
    use super::*;
    use crate::domain::{
        CandidateInput, LocalizedTextKey, ProvenanceUrl, Rule, RuleDefinition, RuleGuards, RuleId,
        RuleMatcher, RuleMatcherDefinition, RuleRef, RuleRevision, RuleScope,
    };
    use crate::path_validation::{
        capture_path_snapshot, capture_scan_root, validate_cleanup_path, validate_scan_root,
    };

    fn fixture() -> (TempDir, Candidate, ScanId, CanonicalPathSnapshot) {
        let temp = TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let root_path = std::fs::canonicalize(temp.path()).unwrap();
        let lexical_root = validate_scan_root(&root_path).unwrap();
        let root = capture_scan_root(lexical_root.clone()).unwrap();
        let target_path = root_path.join("target");
        std::fs::create_dir_all(&target_path).unwrap();
        std::fs::write(
            target_path.join("CACHEDIR.TAG"),
            b"Signature: 8a477f597d28d172789f06886806bc55",
        )
        .unwrap();
        let lexical_target = validate_cleanup_path(&lexical_root, &target_path).unwrap();
        let target = capture_path_snapshot(&root, lexical_target).unwrap();
        let scan_id = ScanId::new("scan:promotion").unwrap();
        let rule = Rule::try_new(RuleDefinition {
            reference: RuleRef::new(
                RuleId::new(RUST_TARGET_RULE).unwrap(),
                RuleRevision::new(RUST_TARGET_REVISION).unwrap(),
            ),
            title_key: LocalizedTextKey::new("rule.rust.title").unwrap(),
            category: CandidateCategory::DeveloperArtifact,
            scope: RuleScope::ConfiguredProjectRoots,
            matcher: RuleMatcher::try_new(RuleMatcherDefinition {
                path_component: Some("target".to_owned()),
                required_ancestor_markers_any: Vec::new(),
                required_markers_all: Vec::new(),
                forbidden_markers_any: Vec::new(),
                exact_bundle_identifiers: Vec::new(),
                excluded_descendants: Vec::new(),
                protected_descendants: Vec::new(),
            })
            .unwrap(),
            guards: RuleGuards::try_new(None, 0, Vec::new(), false).unwrap(),
            safety: SafetyTier::SafeRegenerable,
            action: CandidateAction::RemoveKnownRegenerableContents,
            schedule_eligible: false,
            explanation_key: LocalizedTextKey::new("rule.rust.explanation").unwrap(),
            provenance: vec![ProvenanceUrl::new("https://example.com/rust").unwrap()],
        })
        .unwrap();
        let id = current_rust_target_candidate_id(&scan_id, &target_path).unwrap();
        let candidate = Candidate::try_from_rule(
            &rule,
            CandidateInput::new(
                id,
                vec![target_path.clone()],
                42,
                None,
                vec![
                    Evidence::MatchedPath {
                        path: target_path.clone(),
                    },
                    Evidence::RequiredMarker {
                        path: target_path.parent().unwrap().join("Cargo.toml"),
                    },
                    Evidence::RequiredMarker {
                        path: target_path.join("CACHEDIR.TAG"),
                    },
                ],
                vec![BlockReason::ProtectedPath],
                scan_id.clone(),
            ),
        )
        .unwrap();
        (temp, candidate, scan_id, target)
    }

    #[test]
    fn valid_shape_is_still_blocked() {
        let (_temp, candidate, scan_id, target) = fixture();
        validate_candidate_shape(&candidate, &scan_id, &target).unwrap();
        assert_eq!(candidate.blockers(), [BlockReason::ProtectedPath]);
    }

    #[test]
    fn forged_candidate_id_is_rejected_before_authorization() {
        let (_temp, candidate, _scan_id, target) = fixture();
        // The test fixture's candidate ID is private; replacing it is modeled
        // by using a different source scan, which must fail the same boundary.
        let foreign_scan = ScanId::new("scan:foreign").unwrap();
        assert!(matches!(
            validate_candidate_shape(&candidate, &foreign_scan, &target),
            Err(RustTargetPromotionError::SourceScanMismatch)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_only_no_cargo_authorization_cannot_admit_candidate() {
        let Some(fixture) = super::super::rust_target_tests::Fixture::try_in_current_account_home(
            super::super::rust_target_tests::CARGO_CACHE_TAG_SIGNATURE,
        ) else {
            eprintln!("skipping home-bound promotion test: current home is not writable");
            return;
        };
        let lexical_root = validate_scan_root(&fixture.root).unwrap();
        let scan_root = capture_scan_root(lexical_root.clone()).unwrap();
        let lexical_target = validate_cleanup_path(&lexical_root, &fixture.target).unwrap();
        let target = capture_path_snapshot(&scan_root, lexical_target).unwrap();
        let id = current_rust_target_candidate_id(&fixture.scan_id, &fixture.target).unwrap();
        let candidate = Candidate::try_from_rule(
            &super::super::rust_target_tests::rust_rule(false),
            CandidateInput::new(
                id,
                vec![fixture.target.clone()],
                4_096,
                None,
                super::super::rust_target_tests::exact_evidence(&fixture.target),
                vec![BlockReason::ProtectedPath],
                fixture.scan_id.clone(),
            ),
        )
        .unwrap();
        assert_eq!(candidate.blockers(), [BlockReason::ProtectedPath]);
        let authorization =
            authorize_rule_target(&scan_root, target.clone(), candidate.rule()).unwrap();

        assert!(matches!(
            admit_rust_target_candidate(
                candidate,
                authorization,
                &fixture.scan_id,
                &scan_root,
                &target,
            ),
            Err(RustTargetPromotionError::Authorization(
                RuleScopeGrantError::CargoBoundaryMismatch
            ))
        ));
    }
}

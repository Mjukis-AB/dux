//! Sealed exact-path review evidence for a future cleanup planner.
//!
//! This boundary joins deterministic candidate grouping to one code-owned
//! canonical scan-root observation. It performs lexical validation and a
//! no-follow live identity capture for every selected path, but it deliberately
//! stops before approval or an executor capability exists.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;

use crate::domain::{
    Candidate, CandidateAction, CandidateCategory, CandidateGroupSet, CandidateGroupingError,
    CandidateId, CandidateOverlapReason, CleanupMode, CleanupPlan, CleanupPlanId,
    CleanupPlanValidationError, Evidence, PlanWarning, RuleRef, SafetyTier, ScanId,
};
use crate::path_validation::{
    CanonicalPathError, CanonicalPathSnapshot, CanonicalScanRoot, FilesystemBoundarySnapshot,
    LexicalPathError, ProtectedPathForm, ProtectedPathKind, ProtectedRootDisposition,
    ProtectedRootError, ProtectedRootRegistry, capture_filesystem_boundary, validate_cleanup_path,
    validate_scan_root,
};
use crate::persistence::canonical_started_at;
use crate::persistence::{
    CleanupJournalClaim, CleanupSessionId, CleanupTrigger, HistoryError, NewCleanupSessionRecord,
    StoreCoordinator,
};

use super::rule_scope_grant::{RuleScopeAuthorization, RuleScopeGrantError};
use super::rust_target::{
    RustTargetEffectWitness, RustTargetLiveValidationError, validate_rust_target_effect,
};
#[cfg(unix)]
use super::rust_target_promotion::{RustTargetPlanFacts, RustTargetPromotionError};

pub(crate) const MAX_EXACT_REVIEW_CANDIDATES: usize = 64;
pub(crate) const MAX_EXACT_REVIEW_PATHS: usize = 256;

/// The protected-root policy is intentionally not treated as available yet.
/// A lexical match or a successful filesystem probe can never turn this into
/// permission to mutate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExactPathProtection {
    Denied {
        form: ProtectedPathForm,
        kind: ProtectedPathKind,
        policy_revision: u32,
    },
    SpecificRuleRequired {
        form: ProtectedPathForm,
        kind: ProtectedPathKind,
        policy_revision: u32,
    },
    NoTextualMatch {
        policy_revision: u32,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExactPathReviewPath {
    snapshot: CanonicalPathSnapshot,
    protection: ExactPathProtection,
}

impl ExactPathReviewPath {
    pub(crate) fn snapshot(&self) -> &CanonicalPathSnapshot {
        &self.snapshot
    }

    pub(crate) fn requested_path(&self) -> &Path {
        self.snapshot.requested_path()
    }

    pub(crate) fn canonical_path(&self) -> &Path {
        self.snapshot.canonical_path()
    }

    pub(crate) fn relative_path(&self) -> &Path {
        self.snapshot.relative_path()
    }

    pub(crate) fn protection(&self) -> ExactPathProtection {
        self.protection
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExactPathReviewItem {
    candidate_id: CandidateId,
    rule: RuleRef,
    category: CandidateCategory,
    safety: SafetyTier,
    action: CandidateAction,
    rule_schedule_eligible: bool,
    estimated_bytes: u64,
    newest_mtime: Option<std::time::SystemTime>,
    evidence: Vec<Evidence>,
    paths: Vec<ExactPathReviewPath>,
}

impl ExactPathReviewItem {
    pub(crate) fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    pub(crate) fn rule(&self) -> &RuleRef {
        &self.rule
    }

    pub(crate) fn category(&self) -> CandidateCategory {
        self.category
    }

    pub(crate) fn safety(&self) -> SafetyTier {
        self.safety
    }

    pub(crate) fn action(&self) -> CandidateAction {
        self.action
    }

    pub(crate) fn rule_schedule_eligible(&self) -> bool {
        self.rule_schedule_eligible
    }

    pub(crate) fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }

    pub(crate) fn newest_mtime(&self) -> Option<std::time::SystemTime> {
        self.newest_mtime
    }

    pub(crate) fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }

    pub(crate) fn paths(&self) -> &[ExactPathReviewPath] {
        &self.paths
    }
}

/// Exact review evidence. This type is intentionally neither `Clone` nor
/// serializable: it must not become a second authority or a durable shortcut
/// around the future planner's fresh validation and approval gates.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExactPathReview {
    scan_root: PathBuf,
    boundary: FilesystemBoundarySnapshot,
    source_scan_id: ScanId,
    mode: CleanupMode,
    items: Vec<ExactPathReviewItem>,
    estimated_bytes: u64,
    warnings: Vec<PlanWarning>,
    groups: CandidateGroupSet,
    candidates: Vec<Candidate>,
}

/// A domain plan paired with every exact trusted rule-scope authorization used
/// to construct it. This remains crate-private and has no approval, journal,
/// FFI, schedule, or effect method; the future executor must revalidate the
/// retained authorizations immediately before any mutation.
#[must_use = "reviewed plans must be explicitly consumed by the execution boundary"]
pub(crate) struct TrustedReviewedCleanupPlan {
    plan: CleanupPlan,
    authorizations: Vec<RuleScopeAuthorization>,
}

/// An explicitly approved reviewed plan. This capability is still crate
/// private and deliberately has no journal, FFI, scheduling, or effect method;
/// it only proves that a caller opted in while the plan and its authorizations
/// were current.
#[must_use = "approved plans must be explicitly handed to a future execution boundary"]
pub(crate) struct ApprovedTrustedReviewedCleanupPlan {
    reviewed: TrustedReviewedCleanupPlan,
    approved_at: std::time::SystemTime,
}

/// The first private planner-to-journal handoff. It retains the approved
/// capability and the exact owner-fenced claim together, while intentionally
/// exposing no path, callback, FFI, or filesystem-effect operation.
#[must_use = "an approved cleanup session must be consumed by the engine executor"]
pub(crate) struct ApprovedCleanupSession {
    approved: ApprovedTrustedReviewedCleanupPlan,
    claim: CleanupJournalClaim,
    session_id: CleanupSessionId,
}

#[derive(Debug, Error)]
pub(crate) enum ExactPathHandoffError {
    #[error("approved cleanup plan is expired or its rule authorization changed")]
    Approval(#[from] ExactPathApprovalError),
    #[error("cleanup journal handoff failed: {0}")]
    Journal(HistoryError),
    #[error("rule-specific permanent-safe evidence failed: {0}")]
    RuleEvidence(#[source] RustTargetLiveValidationError),
}

impl TrustedReviewedCleanupPlan {
    pub(crate) fn plan(&self) -> &CleanupPlan {
        &self.plan
    }

    pub(crate) fn revalidate(&self) -> Result<(), ExactPathPlanError> {
        for authorization in &self.authorizations {
            authorization
                .revalidate()
                .map_err(ExactPathPlanError::Authorization)?;
        }
        Ok(())
    }

    fn authorization_for_path(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
    ) -> Option<&RuleScopeAuthorization> {
        let offset = self
            .plan
            .items()
            .iter()
            .take(item_ordinal)
            .try_fold(0_usize, |offset, item| {
                offset.checked_add(item.paths().len())
            })?
            .checked_add(path_ordinal)?;
        self.authorizations.get(offset)
    }

    pub(crate) fn release(self) {}

    pub(crate) fn approve(
        self,
        approved_at: std::time::SystemTime,
    ) -> Result<ApprovedTrustedReviewedCleanupPlan, ExactPathApprovalError> {
        if self.plan.has_expired_at(approved_at) {
            return Err(ExactPathApprovalError::Expired);
        }
        self.revalidate().map_err(|error| match error {
            ExactPathPlanError::Authorization(source) => {
                ExactPathApprovalError::Authorization(source)
            }
            ExactPathPlanError::UnsupportedMode
            | ExactPathPlanError::AuthorizationCount { .. }
            | ExactPathPlanError::AuthorizationMismatch
            | ExactPathPlanError::Plan(_) => {
                unreachable!("trusted plan already passed construction validation")
            }
            #[cfg(unix)]
            ExactPathPlanError::RustTargetPromotion(_) => {
                unreachable!("trusted plan already passed construction validation")
            }
        })?;
        Ok(ApprovedTrustedReviewedCleanupPlan {
            reviewed: self,
            approved_at,
        })
    }
}

impl ApprovedTrustedReviewedCleanupPlan {
    pub(crate) fn plan(&self) -> &CleanupPlan {
        self.reviewed.plan()
    }

    pub(crate) fn approved_at(&self) -> std::time::SystemTime {
        self.approved_at
    }

    pub(crate) fn revalidate(
        &self,
        now: std::time::SystemTime,
    ) -> Result<(), ExactPathApprovalError> {
        if self.plan().has_expired_at(now) {
            return Err(ExactPathApprovalError::Expired);
        }
        self.reviewed.revalidate().map_err(|error| match error {
            ExactPathPlanError::Authorization(source) => {
                ExactPathApprovalError::Authorization(source)
            }
            ExactPathPlanError::UnsupportedMode
            | ExactPathPlanError::AuthorizationCount { .. }
            | ExactPathPlanError::AuthorizationMismatch
            | ExactPathPlanError::Plan(_) => {
                unreachable!("trusted plan already passed construction validation")
            }
            #[cfg(unix)]
            ExactPathPlanError::RustTargetPromotion(_) => {
                unreachable!("trusted plan already passed construction validation")
            }
        })
    }

    /// Revalidate only the next ordered authorization in a multi-path
    /// execution. Earlier targets may have been deliberately mutated by the
    /// same approved session, so rechecking their historical identity would
    /// incorrectly reject unrelated future paths. Plan expiry remains global;
    /// the selected path's grant is still revalidated immediately before its
    /// live witness is rebuilt.
    pub(crate) fn revalidated_target_for_path(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
        now: std::time::SystemTime,
    ) -> Result<crate::path_validation::CanonicalPathSnapshot, ExactPathApprovalError> {
        if self.plan().has_expired_at(now) {
            return Err(ExactPathApprovalError::Expired);
        }
        let authorization = self
            .reviewed
            .authorization_for_path(item_ordinal, path_ordinal)
            .ok_or(ExactPathApprovalError::AuthorizationMismatch)?;
        authorization
            .revalidated_target_snapshot()
            .map_err(ExactPathApprovalError::Authorization)
    }

    /// Persist a planned cleanup session only after the approved capability has
    /// been revalidated at the persistence boundary. The stored session is a
    /// bounded history/journal observation; it does not retain this capability
    /// and cannot authorize a filesystem effect.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "planned-session persistence is consumed by the later engine executor join"
        )
    )]
    pub(crate) fn persist_planned(
        &self,
        store: &StoreCoordinator,
        session_id: CleanupSessionId,
        started_at: std::time::SystemTime,
        trigger: CleanupTrigger,
    ) -> Result<(), ExactPathApprovalError> {
        self.revalidate(started_at)?;
        let record =
            NewCleanupSessionRecord::try_from_plan(session_id, self.plan(), started_at, trigger)
                .map_err(ExactPathApprovalError::Persistence)?;
        store
            .record_cleanup_session_planned(&record)
            .map_err(ExactPathApprovalError::Persistence)
    }

    /// Revalidate, persist, and claim one exact planned session. The journal
    /// row is compared before and after the owner claim so a same-ID row
    /// substitution cannot become an execution input. The returned type is
    /// crate-private and still requires a later effect-specific executor.
    pub(crate) fn begin_cleanup_session(
        self,
        store: &Arc<StoreCoordinator>,
        session_id: CleanupSessionId,
        started_at: std::time::SystemTime,
        trigger: CleanupTrigger,
        lock_timeout: Duration,
    ) -> Result<ApprovedCleanupSession, ExactPathHandoffError> {
        let started_at = canonical_started_at(started_at)
            .map_err(ExactPathApprovalError::Persistence)
            .map_err(ExactPathHandoffError::Approval)?;
        self.revalidate(started_at)?;
        let record = NewCleanupSessionRecord::try_from_plan(
            session_id.clone(),
            self.plan(),
            started_at,
            trigger,
        )
        .map_err(ExactPathApprovalError::Persistence)?;
        store
            .record_cleanup_session_planned(&record)
            .map_err(ExactPathApprovalError::Persistence)?;

        let lease = store
            .acquire_cleanup_journal_lease(lock_timeout)
            .map_err(ExactPathHandoffError::Journal)?;
        lease
            .validate_planned_plan(&session_id, self.plan())
            .map_err(ExactPathHandoffError::Journal)?;
        let claim = lease
            .claim_planned(&session_id, started_at)
            .map_err(|failure| ExactPathHandoffError::Journal(HistoryError::new(failure.kind())))?;
        claim
            .validate_planned_plan(self.plan())
            .map_err(ExactPathHandoffError::Journal)?;
        Ok(ApprovedCleanupSession {
            approved: self,
            claim,
            session_id,
        })
    }

    pub(crate) fn release(self) {}
}

impl ApprovedCleanupSession {
    pub(crate) fn plan(&self) -> &CleanupPlan {
        self.approved.plan()
    }

    pub(crate) fn session_id(&self) -> &CleanupSessionId {
        &self.session_id
    }

    pub(crate) fn claim(&self) -> &CleanupJournalClaim {
        &self.claim
    }

    pub(crate) fn claim_mut(&mut self) -> &mut CleanupJournalClaim {
        &mut self.claim
    }

    /// Last planner/journal check immediately before a future permanent-safe
    /// driver is allowed to receive a reviewed target. This deliberately
    /// returns only a unit witness: target-specific identity validation and
    /// effect receipt admission must be added by the executor itself.
    pub(crate) fn revalidate_for_effect(
        &self,
        now: std::time::SystemTime,
    ) -> Result<(), ExactPathHandoffError> {
        self.approved.revalidate(now)?;
        self.claim
            .validate_planned_plan(self.approved.plan())
            .map_err(ExactPathHandoffError::Journal)
    }

    /// Revalidate one ordered trusted target and its deterministic Rust-target
    /// markers immediately before a future permanent-safe effect. The witness
    /// is private and inert; this method performs no journal transition or
    /// filesystem mutation.
    pub(crate) fn revalidated_rust_target_effect(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
        now: std::time::SystemTime,
    ) -> Result<RustTargetEffectWitness, ExactPathHandoffError> {
        self.approved.revalidate(now)?;
        self.revalidated_rust_target_effect_path(item_ordinal, path_ordinal, now)
    }

    /// Revalidate one ordered target after earlier paths in the same approved
    /// session have already settled. The plan expiry and this path's grant
    /// are checked, while settled earlier targets are intentionally not
    /// revalidated as if they were still pending work.
    pub(crate) fn revalidated_rust_target_effect_for_ordered_session(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
        now: std::time::SystemTime,
    ) -> Result<RustTargetEffectWitness, ExactPathHandoffError> {
        self.revalidated_rust_target_effect_path(item_ordinal, path_ordinal, now)
    }

    fn revalidated_rust_target_effect_path(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
        now: std::time::SystemTime,
    ) -> Result<RustTargetEffectWitness, ExactPathHandoffError> {
        self.claim
            .validate_validating_path(item_ordinal, path_ordinal)
            .map_err(ExactPathHandoffError::Journal)?;
        let item = self
            .approved
            .plan()
            .items()
            .get(item_ordinal)
            .ok_or_else(|| {
                ExactPathHandoffError::Journal(HistoryError::new(
                    crate::persistence::HistoryErrorKind::InvalidInput,
                ))
            })?;
        if item.rule().id().as_str() != "developer.rust.target"
            || item.rule().revision().get() != 2
            || item.safety() != SafetyTier::SafeRegenerable
            || item.action() != CandidateAction::RemoveKnownRegenerableContents
            || item.rule_marks_schedule_eligible()
        {
            return Err(ExactPathHandoffError::Journal(HistoryError::new(
                crate::persistence::HistoryErrorKind::InvalidTransition,
            )));
        }
        let expected_path = item.paths().get(path_ordinal).ok_or_else(|| {
            ExactPathHandoffError::Journal(HistoryError::new(
                crate::persistence::HistoryErrorKind::InvalidInput,
            ))
        })?;
        let target = self
            .approved
            .revalidated_target_for_path(item_ordinal, path_ordinal, now)
            .map_err(ExactPathHandoffError::Approval)?;
        if target.requested_path() != expected_path {
            return Err(ExactPathHandoffError::Journal(HistoryError::new(
                crate::persistence::HistoryErrorKind::InvalidTransition,
            )));
        }
        validate_rust_target_effect(target).map_err(ExactPathHandoffError::RuleEvidence)
    }

    pub(crate) fn release(self) {}
}

impl ExactPathReview {
    pub(crate) fn scan_root(&self) -> &Path {
        &self.scan_root
    }

    /// Repeated no-follow ancestry and mount evidence captured for the
    /// planner-owned scan root. This remains observational until a separate
    /// trusted volume/location grant is joined to it.
    pub(crate) fn boundary(&self) -> &FilesystemBoundarySnapshot {
        &self.boundary
    }

    pub(crate) fn source_scan_id(&self) -> &ScanId {
        &self.source_scan_id
    }

    pub(crate) fn mode(&self) -> CleanupMode {
        self.mode
    }

    pub(crate) fn items(&self) -> &[ExactPathReviewItem] {
        &self.items
    }

    pub(crate) fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }

    pub(crate) fn warnings(&self) -> &[PlanWarning] {
        &self.warnings
    }

    /// The immutable, planner-owned grouping witness used to select `items`.
    /// Keeping it with the review prevents later callers from reconstructing
    /// overlap decisions from a reordered or otherwise different candidate
    /// slice. It remains descriptive only; approval and execution are separate
    /// authority boundaries.
    pub(crate) fn candidate_groups(&self) -> &CandidateGroupSet {
        &self.groups
    }

    /// No current review can be actionable because protected-root and volume
    /// grants have not yet been bound to the planner-owned witness.
    pub(crate) const fn is_actionable(&self) -> bool {
        false
    }

    /// Consume exact review evidence plus one matching trusted authorization
    /// per selected path into a permanent-safe domain plan. Only the known
    /// deterministic rule grants can reach this boundary; no effect is
    /// invoked and the plan remains unapproved and non-executable.
    pub(crate) fn into_trusted_permanent_plan(
        self,
        plan_id: CleanupPlanId,
        created_at: std::time::SystemTime,
        authorizations: Vec<RuleScopeAuthorization>,
    ) -> Result<TrustedReviewedCleanupPlan, ExactPathPlanError> {
        if self.mode != CleanupMode::PermanentSafe {
            return Err(ExactPathPlanError::UnsupportedMode);
        }
        let expected = self
            .items
            .iter()
            .map(|item| item.paths.len())
            .sum::<usize>();
        if authorizations.len() != expected {
            return Err(ExactPathPlanError::AuthorizationCount {
                expected,
                actual: authorizations.len(),
            });
        }
        let mut available = authorizations.into_iter().map(Some).collect::<Vec<_>>();
        let mut ordered_authorizations = Vec::with_capacity(expected);
        for item in &self.items {
            for path in &item.paths {
                let Some(index) = available.iter().enumerate().position(|(_, authorization)| {
                    authorization.as_ref().is_some_and(|authorization| {
                        authorization.matches(&item.rule, path.snapshot())
                    })
                }) else {
                    return Err(ExactPathPlanError::AuthorizationMismatch);
                };
                let authorization = available[index]
                    .take()
                    .expect("authorization position was available");
                authorization
                    .revalidate()
                    .map_err(ExactPathPlanError::Authorization)?;
                ordered_authorizations.push(authorization);
            }
        }
        if available.iter().any(Option::is_some) {
            return Err(ExactPathPlanError::AuthorizationMismatch);
        }
        let plan =
            CleanupPlan::try_from_candidates(plan_id, created_at, self.mode, &self.candidates)
                .map_err(ExactPathPlanError::Plan)?;
        Ok(TrustedReviewedCleanupPlan {
            plan,
            authorizations: ordered_authorizations,
        })
    }
}

#[cfg(unix)]
pub(crate) fn approve_rust_target_plan_facts(
    facts: RustTargetPlanFacts,
    plan_id: CleanupPlanId,
    created_at: std::time::SystemTime,
    approved_at: std::time::SystemTime,
) -> Result<ApprovedTrustedReviewedCleanupPlan, ExactPathApprovalError> {
    let reviewed =
        ExactPathReview::trusted_reviewed_plan_from_rust_target_facts(facts, plan_id, created_at)
            .map_err(ExactPathApprovalError::Plan)?;
    reviewed.approve(approved_at)
}

#[cfg(unix)]
pub(crate) struct RustTargetJournalRequest {
    pub(crate) plan_id: CleanupPlanId,
    pub(crate) created_at: std::time::SystemTime,
    pub(crate) approved_at: std::time::SystemTime,
    pub(crate) session_id: CleanupSessionId,
    pub(crate) started_at: std::time::SystemTime,
    pub(crate) trigger: CleanupTrigger,
    pub(crate) lock_timeout: Duration,
}

#[cfg(unix)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Rust-target journal handoff is consumed by the next executor orchestration slice"
    )
)]
pub(crate) fn begin_rust_target_cleanup_session(
    facts: RustTargetPlanFacts,
    request: RustTargetJournalRequest,
    store: &Arc<StoreCoordinator>,
) -> Result<ApprovedCleanupSession, ExactPathHandoffError> {
    let approved = approve_rust_target_plan_facts(
        facts,
        request.plan_id,
        request.created_at,
        request.approved_at,
    )
    .map_err(ExactPathHandoffError::Approval)?;
    approved.begin_cleanup_session(
        store,
        request.session_id,
        request.started_at,
        request.trigger,
        request.lock_timeout,
    )
}

#[cfg(unix)]
impl ExactPathReview {
    /// Consume one fully admitted Rust-target facts capability into the same
    /// reviewed-plan wrapper used by the later approval/journal boundary.
    /// This private handoff is intentionally not reachable from generic exact
    /// review, FFI, Swift, CLI, scheduling, or filesystem effects.
    pub(crate) fn trusted_reviewed_plan_from_rust_target_facts(
        facts: RustTargetPlanFacts,
        plan_id: CleanupPlanId,
        created_at: std::time::SystemTime,
    ) -> Result<TrustedReviewedCleanupPlan, ExactPathPlanError> {
        let (plan, authorization) = facts
            .into_trusted_permanent_plan(plan_id, created_at, CleanupMode::PermanentSafe)
            .map_err(ExactPathPlanError::RustTargetPromotion)?;
        let [item] = plan.items() else {
            return Err(ExactPathPlanError::AuthorizationMismatch);
        };
        let [path] = item.paths() else {
            return Err(ExactPathPlanError::AuthorizationMismatch);
        };
        let target = canonical_target_for_plan(&authorization, path)?;
        if !authorization.matches(item.rule(), &target) {
            return Err(ExactPathPlanError::AuthorizationMismatch);
        }
        authorization
            .revalidate()
            .map_err(ExactPathPlanError::Authorization)?;
        Ok(TrustedReviewedCleanupPlan {
            plan,
            authorizations: vec![authorization],
        })
    }
}

#[cfg(unix)]
fn canonical_target_for_plan(
    authorization: &RuleScopeAuthorization,
    path: &Path,
) -> Result<CanonicalPathSnapshot, ExactPathPlanError> {
    let target = authorization
        .revalidated_target_snapshot()
        .map_err(ExactPathPlanError::Authorization)?;
    if target.requested_path() != path {
        return Err(ExactPathPlanError::AuthorizationMismatch);
    }
    Ok(target)
}

#[derive(Debug, Error)]
pub(crate) enum ExactPathReviewError {
    #[error("exact-path review requires at least one candidate")]
    MissingCandidates,
    #[error("exact-path review received {actual} candidates; maximum is {maximum}")]
    TooManyCandidates { actual: usize, maximum: usize },
    #[error("exact-path review received {actual} paths; maximum is {maximum}")]
    TooManyPaths { actual: usize, maximum: usize },
    #[error("candidate grouping failed: {0}")]
    Grouping(#[from] CandidateGroupingError),
    #[error("candidate overlap {first_index}/{second_index} remains unresolved: {reason:?}")]
    UnresolvedOverlap {
        first_index: usize,
        second_index: usize,
        reason: CandidateOverlapReason,
    },
    #[error("candidate {candidate_index} has unresolved blockers")]
    BlockedCandidate { candidate_index: usize },
    #[error("candidate {candidate_index} does not describe a cleanup operation")]
    NonCleanupCandidate { candidate_index: usize },
    #[error("candidate {candidate_index} action {action:?} is incompatible with mode {mode:?}")]
    IncompatibleMode {
        candidate_index: usize,
        mode: CleanupMode,
        action: CandidateAction,
    },
    #[error("scan root lexical validation failed: {0}")]
    ScanRootLexical(#[source] LexicalPathError),
    #[error("scan root filesystem boundary could not be captured: {0}")]
    ScanRootBoundary(#[source] CanonicalPathError),
    #[error("scan root filesystem boundary changed during exact review: {0}")]
    ScanRootBoundaryChanged(#[source] CanonicalPathError),
    #[error("candidate {candidate_index} path {path_index} lexical validation failed: {source}")]
    PathLexical {
        candidate_index: usize,
        path_index: usize,
        #[source]
        source: LexicalPathError,
    },
    #[error("candidate {candidate_index} path {path_index} live validation failed: {source}")]
    PathLive {
        candidate_index: usize,
        path_index: usize,
        #[source]
        source: CanonicalPathError,
    },
    #[error(
        "protected-root policy could not be evaluated for candidate {candidate_index} path {path_index}: {source}"
    )]
    ProtectedPolicyUnavailable {
        candidate_index: usize,
        path_index: usize,
        #[source]
        source: ProtectedRootError,
    },
    #[error("candidate {candidate_index} path {path_index} is denied by protected-root policy")]
    ProtectedPathDenied {
        candidate_index: usize,
        path_index: usize,
        form: ProtectedPathForm,
        kind: ProtectedPathKind,
    },
    #[error(
        "candidate {candidate_index} path {path_index} is a multiply-linked regular file ({hard_link_count} links)"
    )]
    MultiplyLinkedRegularFile {
        candidate_index: usize,
        path_index: usize,
        hard_link_count: u64,
    },
    #[error("exact-path review estimated byte total overflowed")]
    EstimatedBytesOverflow,
}

#[derive(Debug, Error)]
pub(crate) enum ExactPathPlanError {
    #[error("trusted permanent-safe plan construction requires permanent-safe mode")]
    UnsupportedMode,
    #[error("trusted plan requires {expected} exact authorizations but received {actual}")]
    AuthorizationCount { expected: usize, actual: usize },
    #[error("a trusted authorization did not match the reviewed rule and target")]
    AuthorizationMismatch,
    #[error("trusted rule-scope authorization failed: {0}")]
    Authorization(#[source] RuleScopeGrantError),
    #[cfg(unix)]
    #[error("Rust-target promotion failed: {0}")]
    RustTargetPromotion(#[source] RustTargetPromotionError),
    #[error("domain cleanup plan validation failed: {0}")]
    Plan(#[source] CleanupPlanValidationError),
}

#[derive(Debug, Error)]
pub(crate) enum ExactPathApprovalError {
    #[error("trusted cleanup plan has expired")]
    Expired,
    #[error("trusted cleanup plan has no authorization for the requested path")]
    AuthorizationMismatch,
    #[error("trusted rule-scope authorization failed: {0}")]
    Authorization(#[source] RuleScopeGrantError),
    #[error("trusted Rust-target plan construction failed: {0}")]
    Plan(#[source] ExactPathPlanError),
    #[error("planned cleanup history could not be persisted: {0}")]
    Persistence(#[source] HistoryError),
}

/// Capture exact, current path evidence for a selected candidate set.
///
/// `scan_root` must already be a code-owned canonical observation. The
/// returned review remains non-actionable because no trusted protected-root or
/// volume grant is attached yet.
pub(crate) fn review_exact_paths(
    scan_root: &CanonicalScanRoot,
    candidates: &[Candidate],
    mode: CleanupMode,
) -> Result<ExactPathReview, ExactPathReviewError> {
    if candidates.is_empty() {
        return Err(ExactPathReviewError::MissingCandidates);
    }
    if candidates.len() > MAX_EXACT_REVIEW_CANDIDATES {
        return Err(ExactPathReviewError::TooManyCandidates {
            actual: candidates.len(),
            maximum: MAX_EXACT_REVIEW_CANDIDATES,
        });
    }

    for (candidate_index, candidate) in candidates.iter().enumerate() {
        if !candidate.has_no_known_blockers() {
            return Err(ExactPathReviewError::BlockedCandidate { candidate_index });
        }
        if !candidate.has_cleanup_operation() {
            return Err(ExactPathReviewError::NonCleanupCandidate { candidate_index });
        }
        if !mode_accepts(mode, candidate.safety(), candidate.action()) {
            return Err(ExactPathReviewError::IncompatibleMode {
                candidate_index,
                mode,
                action: candidate.action(),
            });
        }
    }

    let groups = crate::domain::group_candidates_for_review(candidates)?;
    if let Some(decision) = groups.overlap_decisions().iter().find(|decision| {
        matches!(
            decision.resolution(),
            crate::domain::CandidateOverlapResolution::Unresolved(_)
        )
    }) {
        let crate::domain::CandidateOverlapResolution::Unresolved(reason) = decision.resolution()
        else {
            unreachable!("unresolved overlap search returned a coalesced decision")
        };
        return Err(ExactPathReviewError::UnresolvedOverlap {
            first_index: decision.first_index(),
            second_index: decision.second_index(),
            reason,
        });
    }

    let lexical_root = validate_scan_root(scan_root.requested_path())
        .map_err(ExactPathReviewError::ScanRootLexical)?;
    let boundary =
        capture_filesystem_boundary(scan_root).map_err(ExactPathReviewError::ScanRootBoundary)?;
    #[cfg(test)]
    let protected_registry = ProtectedRootRegistry::for_exact_review_fixture();
    #[cfg(not(test))]
    let protected_registry = ProtectedRootRegistry::from_current_account().map_err(|source| {
        ExactPathReviewError::ProtectedPolicyUnavailable {
            candidate_index: 0,
            path_index: 0,
            source,
        }
    })?;
    let total_paths = candidates.iter().try_fold(0_usize, |total, candidate| {
        total
            .checked_add(candidate.paths().len())
            .ok_or(ExactPathReviewError::TooManyPaths {
                actual: usize::MAX,
                maximum: MAX_EXACT_REVIEW_PATHS,
            })
    })?;
    if total_paths > MAX_EXACT_REVIEW_PATHS {
        return Err(ExactPathReviewError::TooManyPaths {
            actual: total_paths,
            maximum: MAX_EXACT_REVIEW_PATHS,
        });
    }

    let mut selected_indices = groups
        .groups()
        .iter()
        .flat_map(|group| group.selected_indices().iter().copied())
        .collect::<Vec<_>>();
    selected_indices.sort_by(|left, right| {
        candidates[*left]
            .id()
            .as_str()
            .cmp(candidates[*right].id().as_str())
    });

    let mut items = Vec::with_capacity(selected_indices.len());
    let mut selected_candidates = Vec::with_capacity(selected_indices.len());
    let mut estimated_bytes = 0_u64;
    for candidate_index in selected_indices {
        let candidate = &candidates[candidate_index];
        selected_candidates.push(candidate.clone());
        estimated_bytes = estimated_bytes
            .checked_add(candidate.estimated_bytes())
            .ok_or(ExactPathReviewError::EstimatedBytesOverflow)?;

        let mut path_indices = (0..candidate.paths().len()).collect::<Vec<_>>();
        path_indices
            .sort_by(|left, right| candidate.paths()[*left].cmp(&candidate.paths()[*right]));
        let mut paths = Vec::with_capacity(path_indices.len());
        for path_index in path_indices {
            let path = &candidate.paths()[path_index];
            let lexical_path = validate_cleanup_path(&lexical_root, path).map_err(|source| {
                ExactPathReviewError::PathLexical {
                    candidate_index,
                    path_index,
                    source,
                }
            })?;
            let requested_protection =
                protected_registry
                    .preflight(&lexical_path)
                    .map_err(|source| ExactPathReviewError::ProtectedPolicyUnavailable {
                        candidate_index,
                        path_index,
                        source,
                    })?;
            if let ProtectedRootDisposition::Denied { form, kind, .. } = requested_protection {
                return Err(ExactPathReviewError::ProtectedPathDenied {
                    candidate_index,
                    path_index,
                    form,
                    kind,
                });
            }
            let snapshot = crate::path_validation::capture_path_snapshot(scan_root, lexical_path)
                .map_err(|source| ExactPathReviewError::PathLive {
                candidate_index,
                path_index,
                source,
            })?;
            let canonical_protection =
                protected_registry
                    .assess(scan_root, &snapshot)
                    .map_err(|source| ExactPathReviewError::ProtectedPolicyUnavailable {
                        candidate_index,
                        path_index,
                        source,
                    })?;
            if let ProtectedRootDisposition::Denied { form, kind, .. } = canonical_protection {
                return Err(ExactPathReviewError::ProtectedPathDenied {
                    candidate_index,
                    path_index,
                    form,
                    kind,
                });
            }
            if candidate.action().is_permanent_removal()
                && snapshot.target_kind()
                    == crate::path_validation::FilesystemEntryKind::RegularFile
                && snapshot.hard_link_count() > 1
            {
                return Err(ExactPathReviewError::MultiplyLinkedRegularFile {
                    candidate_index,
                    path_index,
                    hard_link_count: snapshot.hard_link_count(),
                });
            }
            paths.push(ExactPathReviewPath {
                snapshot,
                protection: combine_protection(requested_protection, canonical_protection),
            });
        }

        items.push(ExactPathReviewItem {
            candidate_id: candidate.id().clone(),
            rule: candidate.rule().clone(),
            category: candidate.category(),
            safety: candidate.safety(),
            action: candidate.action(),
            rule_schedule_eligible: candidate.rule_marks_schedule_eligible(),
            estimated_bytes: candidate.estimated_bytes(),
            newest_mtime: candidate.newest_mtime(),
            evidence: candidate.evidence().to_vec(),
            paths,
        });
    }

    boundary
        .revalidate()
        .map_err(ExactPathReviewError::ScanRootBoundaryChanged)?;

    Ok(ExactPathReview {
        scan_root: scan_root.requested_path().to_path_buf(),
        boundary,
        source_scan_id: candidates[0].source_scan_id().clone(),
        mode,
        items,
        estimated_bytes,
        warnings: derive_warnings(mode, candidates),
        groups,
        candidates: selected_candidates,
    })
}

fn combine_protection(
    requested: ProtectedRootDisposition,
    canonical: ProtectedRootDisposition,
) -> ExactPathProtection {
    let specific = |disposition| match disposition {
        ProtectedRootDisposition::SpecificRuleRequired {
            form,
            kind,
            policy_revision,
        } => Some(ExactPathProtection::SpecificRuleRequired {
            form,
            kind,
            policy_revision,
        }),
        _ => None,
    };
    specific(requested)
        .or_else(|| specific(canonical))
        .unwrap_or(match canonical {
            ProtectedRootDisposition::NoTextualMatch { policy_revision }
            | ProtectedRootDisposition::Denied {
                policy_revision, ..
            }
            | ProtectedRootDisposition::SpecificRuleRequired {
                policy_revision, ..
            } => ExactPathProtection::NoTextualMatch { policy_revision },
        })
}

fn mode_accepts(mode: CleanupMode, safety: SafetyTier, action: CandidateAction) -> bool {
    match mode {
        CleanupMode::DryRun => action.is_cleanup_operation(),
        CleanupMode::Trash => {
            safety == SafetyTier::ReviewRequired && action == CandidateAction::MoveToTrash
        }
        CleanupMode::PermanentSafe => {
            safety == SafetyTier::SafeRegenerable
                && action == CandidateAction::RemoveKnownRegenerableContents
        }
        CleanupMode::EvictLocalCopy => {
            safety == SafetyTier::SafeEvictable && action == CandidateAction::EvictLocalCopy
        }
    }
}

fn derive_warnings(mode: CleanupMode, candidates: &[Candidate]) -> Vec<PlanWarning> {
    let dry_run = mode == CleanupMode::DryRun;
    let has_action = |action| {
        candidates
            .iter()
            .any(|candidate| candidate.action() == action)
    };
    let warn_trash =
        mode == CleanupMode::Trash || (dry_run && has_action(CandidateAction::MoveToTrash));
    let warn_permanent = mode == CleanupMode::PermanentSafe
        || (dry_run && has_action(CandidateAction::RemoveKnownRegenerableContents));
    let warn_eviction = mode == CleanupMode::EvictLocalCopy
        || (dry_run && has_action(CandidateAction::EvictLocalCopy));

    let mut warnings = vec![PlanWarning::EstimatedBytesUnverified];
    if dry_run {
        warnings.push(PlanWarning::DryRunDoesNotMutate);
    }
    if warn_trash {
        warnings.push(PlanWarning::TrashDoesNotFreeSpaceImmediately);
    }
    if warn_permanent {
        warnings.push(PlanWarning::PermanentRemovalCannotBeUndone);
    }
    if warn_eviction {
        warnings.push(PlanWarning::CloudEvictionRequiresNetworkToRedownload);
    }
    warnings
}

#[cfg(test)]
#[path = "exact_path_review_tests.rs"]
mod tests;

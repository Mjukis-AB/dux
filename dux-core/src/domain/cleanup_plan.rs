//! Immutable cleanup-plan representation without execution authority.
//!
//! A plan freezes deterministic candidate facts for review, but it is not an
//! approval, validation witness, or executable capability. Construction stays
//! private until the planner can consume canonical, protected-root-checked
//! paths from the dedicated validator. This module performs no filesystem I/O.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use thiserror::Error;

use super::{
    Candidate, CandidateAction, CandidateCategory, CandidateId, CleanupPlanId, Evidence, RuleId,
    RuleRef, RuleRevision, SafetyTier, ScanId,
};

pub const CLEANUP_PLAN_VALIDITY: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupPlan {
    id: CleanupPlanId,
    created_at: SystemTime,
    source_scan_id: ScanId,
    mode: CleanupMode,
    items: Vec<CleanupPlanItem>,
    estimated_bytes: u64,
    warnings: Vec<PlanWarning>,
    expires_at: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupPlanItem {
    candidate_id: CandidateId,
    rule: RuleRef,
    category: CandidateCategory,
    paths: Vec<PathBuf>,
    estimated_bytes: u64,
    newest_mtime: Option<SystemTime>,
    evidence: Vec<Evidence>,
    safety: SafetyTier,
    action: CandidateAction,
    rule_schedule_eligible: bool,
}

/// Internal planner projection for a candidate whose authority was already
/// joined by a typed, crate-private facts capability. It intentionally has no
/// blocker field: callers cannot construct this projection from a raw
/// `Candidate`; only the planner facts boundary can produce it.
#[derive(Debug)]
pub(crate) struct CleanupPlanCandidateFacts {
    pub(crate) candidate_id: CandidateId,
    pub(crate) source_scan_id: ScanId,
    pub(crate) rule: RuleRef,
    pub(crate) category: CandidateCategory,
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) estimated_bytes: u64,
    pub(crate) newest_mtime: Option<SystemTime>,
    pub(crate) evidence: Vec<Evidence>,
    pub(crate) safety: SafetyTier,
    pub(crate) action: CandidateAction,
    pub(crate) rule_schedule_eligible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CleanupMode {
    DryRun,
    Trash,
    PermanentSafe,
    EvictLocalCopy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanWarning {
    EstimatedBytesUnverified,
    DryRunDoesNotMutate,
    TrashDoesNotFreeSpaceImmediately,
    PermanentRemovalCannotBeUndone,
    CloudEvictionRequiresNetworkToRedownload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationStatus {
    Planned,
    DryRun,
    Trashed,
    Removed,
    Evicted,
    Skipped,
    Rejected,
    Failed,
    ChangedSincePlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CleanupPlanValidationError {
    #[error("cleanup plan must contain at least one candidate")]
    MissingCandidates,
    #[error("candidate {duplicate_index} duplicates candidate {first_index}")]
    DuplicateCandidate {
        first_index: usize,
        duplicate_index: usize,
    },
    #[error(
        "candidate {conflicting_index} belongs to a different scan than candidate {first_index}"
    )]
    MixedSourceScans {
        first_index: usize,
        conflicting_index: usize,
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
    #[error(
        "candidate/path {second_candidate_index}/{second_path_index} overlaps candidate/path {first_candidate_index}/{first_path_index}"
    )]
    OverlappingPaths {
        first_candidate_index: usize,
        first_path_index: usize,
        second_candidate_index: usize,
        second_path_index: usize,
    },
    #[error("cleanup plan estimated byte total overflowed")]
    EstimatedBytesOverflow,
    #[error("the Explorer Trash selection path is invalid")]
    InvalidSelectionPath,
    #[error("cleanup plan expiration could not be represented")]
    ExpirationOverflow,
    #[error("only a permanent-safe plan can be projected into this dry run")]
    InvalidDryRunSourceMode,
}

impl CleanupPlan {
    /// Construct a plan from a typed planner projection. This boundary repeats
    /// all plan-shape validation but accepts no raw candidate or blocker flag.
    /// The projection type is crate-private and is produced only by the
    /// planner's trusted facts capability.
    pub(crate) fn try_from_trusted_candidate_facts(
        id: CleanupPlanId,
        created_at: SystemTime,
        mode: CleanupMode,
        facts: &[CleanupPlanCandidateFacts],
    ) -> Result<Self, CleanupPlanValidationError> {
        let Some(first) = facts.first() else {
            return Err(CleanupPlanValidationError::MissingCandidates);
        };
        let expires_at = created_at
            .checked_add(CLEANUP_PLAN_VALIDITY)
            .ok_or(CleanupPlanValidationError::ExpirationOverflow)?;
        let source_scan_id = first.source_scan_id.clone();
        let mut first_seen = HashMap::<&CandidateId, usize>::new();
        let mut estimated_bytes = 0_u64;
        for (candidate_index, fact) in facts.iter().enumerate() {
            if let Some(first_index) = first_seen.insert(&fact.candidate_id, candidate_index) {
                return Err(CleanupPlanValidationError::DuplicateCandidate {
                    first_index,
                    duplicate_index: candidate_index,
                });
            }
            if fact.source_scan_id != source_scan_id {
                return Err(CleanupPlanValidationError::MixedSourceScans {
                    first_index: 0,
                    conflicting_index: candidate_index,
                });
            }
            if !fact.action.is_cleanup_operation() {
                return Err(CleanupPlanValidationError::NonCleanupCandidate { candidate_index });
            }
            if !mode_accepts(mode, fact.safety, fact.action) {
                return Err(CleanupPlanValidationError::IncompatibleMode {
                    candidate_index,
                    mode,
                    action: fact.action,
                });
            }
            estimated_bytes = estimated_bytes
                .checked_add(fact.estimated_bytes)
                .ok_or(CleanupPlanValidationError::EstimatedBytesOverflow)?;
        }
        reject_fact_path_overlaps(facts)?;
        let warnings = derive_fact_warnings(mode, facts);
        let items = facts.iter().map(CleanupPlanItem::from_facts).collect();
        Ok(Self {
            id,
            created_at,
            source_scan_id,
            mode,
            items,
            estimated_bytes,
            warnings,
            expires_at,
        })
    }

    /// Freeze candidate facts for a future reviewed planner.
    ///
    /// This intentionally remains private: candidates do not yet carry the
    /// canonical path-validation witness required by a real planner. Even a
    /// value produced here would remain review data, never executor authority.
    pub(crate) fn try_from_candidates(
        id: CleanupPlanId,
        created_at: SystemTime,
        mode: CleanupMode,
        candidates: &[Candidate],
    ) -> Result<Self, CleanupPlanValidationError> {
        let Some(first_candidate) = candidates.first() else {
            return Err(CleanupPlanValidationError::MissingCandidates);
        };
        let expires_at = created_at
            .checked_add(CLEANUP_PLAN_VALIDITY)
            .ok_or(CleanupPlanValidationError::ExpirationOverflow)?;
        let source_scan_id = first_candidate.source_scan_id().clone();
        let mut first_seen = HashMap::<&CandidateId, usize>::new();
        let mut estimated_bytes = 0_u64;

        for (candidate_index, candidate) in candidates.iter().enumerate() {
            if let Some(first_index) = first_seen.insert(candidate.id(), candidate_index) {
                return Err(CleanupPlanValidationError::DuplicateCandidate {
                    first_index,
                    duplicate_index: candidate_index,
                });
            }
            if candidate.source_scan_id() != &source_scan_id {
                return Err(CleanupPlanValidationError::MixedSourceScans {
                    first_index: 0,
                    conflicting_index: candidate_index,
                });
            }
            if !candidate.has_no_known_blockers() {
                return Err(CleanupPlanValidationError::BlockedCandidate { candidate_index });
            }
            if !candidate.has_cleanup_operation() {
                return Err(CleanupPlanValidationError::NonCleanupCandidate { candidate_index });
            }
            if !mode_accepts(mode, candidate.safety(), candidate.action()) {
                return Err(CleanupPlanValidationError::IncompatibleMode {
                    candidate_index,
                    mode,
                    action: candidate.action(),
                });
            }
            estimated_bytes = estimated_bytes
                .checked_add(candidate.estimated_bytes())
                .ok_or(CleanupPlanValidationError::EstimatedBytesOverflow)?;
        }

        reject_path_overlaps(candidates)?;
        let warnings = derive_warnings(mode, candidates);
        let items = candidates
            .iter()
            .map(CleanupPlanItem::from_candidate)
            .collect();

        Ok(Self {
            id,
            created_at,
            source_scan_id,
            mode,
            items,
            estimated_bytes,
            warnings,
            expires_at,
        })
    }

    /// Build the single-item plan used by an explicit Explorer Trash action.
    ///
    /// This constructor is intentionally crate-private and accepts only the
    /// already live-validated path supplied by the Explorer review boundary.
    /// It does not accept a caller-provided rule, safety tier, or action; the
    /// synthetic review-selection policy is fixed to `ReviewRequired` and
    /// `MoveToTrash`, is never schedule eligible, and carries no reclaimable
    /// byte estimate.
    pub(crate) fn try_from_trash_selection(
        id: CleanupPlanId,
        created_at: SystemTime,
        source_scan_id: ScanId,
        candidate_id: CandidateId,
        path: PathBuf,
    ) -> Result<Self, CleanupPlanValidationError> {
        if !is_absolute_clean_path(&path) {
            return Err(CleanupPlanValidationError::InvalidSelectionPath);
        }
        let expires_at = created_at
            .checked_add(CLEANUP_PLAN_VALIDITY)
            .ok_or(CleanupPlanValidationError::ExpirationOverflow)?;
        let rule = RuleRef::new(
            RuleId::new("explorer.selection.trash")
                .expect("the built-in Explorer Trash rule ID is valid"),
            RuleRevision::new(1).expect("the built-in Explorer Trash rule revision is non-zero"),
        );
        let item = CleanupPlanItem {
            candidate_id,
            rule,
            category: CandidateCategory::LargeReviewItem,
            paths: vec![path.clone()],
            estimated_bytes: 0,
            newest_mtime: None,
            evidence: vec![Evidence::MatchedPath { path }],
            safety: SafetyTier::ReviewRequired,
            action: CandidateAction::MoveToTrash,
            rule_schedule_eligible: false,
        };
        Ok(Self {
            id,
            created_at,
            source_scan_id,
            mode: CleanupMode::Trash,
            items: vec![item],
            estimated_bytes: 0,
            warnings: vec![
                PlanWarning::EstimatedBytesUnverified,
                PlanWarning::TrashDoesNotFreeSpaceImmediately,
            ],
            expires_at,
        })
    }

    #[cfg(test)]
    pub(crate) fn try_from_candidates_for_persistence_test(
        id: CleanupPlanId,
        created_at: SystemTime,
        mode: CleanupMode,
        candidates: &[Candidate],
    ) -> Result<Self, CleanupPlanValidationError> {
        Self::try_from_candidates(id, created_at, mode, candidates)
    }

    pub fn id(&self) -> &CleanupPlanId {
        &self.id
    }

    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    pub fn source_scan_id(&self) -> &ScanId {
        &self.source_scan_id
    }

    pub fn mode(&self) -> CleanupMode {
        self.mode
    }

    pub fn items(&self) -> &[CleanupPlanItem] {
        &self.items
    }

    pub fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }

    pub fn warnings(&self) -> &[PlanWarning] {
        &self.warnings
    }

    pub fn expires_at(&self) -> SystemTime {
        self.expires_at
    }

    /// Consume a permanent-safe plan into its observation-only projection.
    ///
    /// The immutable identity, source, item facts, estimates, and original
    /// expiry are retained. Only the mode and its mandatory warning set
    /// change. This is crate-private because callers must not use a mode
    /// conversion to bypass the planner authority that admitted the original
    /// permanent-safe plan.
    pub(crate) fn into_dry_run(mut self) -> Result<Self, CleanupPlanValidationError> {
        if self.mode != CleanupMode::PermanentSafe {
            return Err(CleanupPlanValidationError::InvalidDryRunSourceMode);
        }
        self.mode = CleanupMode::DryRun;
        self.warnings = derive_item_warnings(self.mode, &self.items);
        Ok(self)
    }

    /// Expiration is necessary but never sufficient evidence for execution.
    pub fn has_expired_at(&self, now: SystemTime) -> bool {
        now >= self.expires_at
    }
}

fn is_absolute_clean_path(path: &Path) -> bool {
    path.is_absolute()
        && !path.as_os_str().is_empty()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
        && path.components().count() != 1
}

impl CleanupPlanItem {
    fn from_candidate(candidate: &Candidate) -> Self {
        Self {
            candidate_id: candidate.id().clone(),
            rule: candidate.rule().clone(),
            category: candidate.category(),
            paths: candidate.paths().to_vec(),
            estimated_bytes: candidate.estimated_bytes(),
            newest_mtime: candidate.newest_mtime(),
            evidence: candidate.evidence().to_vec(),
            safety: candidate.safety(),
            action: candidate.action(),
            rule_schedule_eligible: candidate.rule_marks_schedule_eligible(),
        }
    }

    fn from_facts(facts: &CleanupPlanCandidateFacts) -> Self {
        Self {
            candidate_id: facts.candidate_id.clone(),
            rule: facts.rule.clone(),
            category: facts.category,
            paths: facts.paths.clone(),
            estimated_bytes: facts.estimated_bytes,
            newest_mtime: facts.newest_mtime,
            evidence: facts.evidence.clone(),
            safety: facts.safety,
            action: facts.action,
            rule_schedule_eligible: facts.rule_schedule_eligible,
        }
    }

    pub fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    pub fn rule(&self) -> &RuleRef {
        &self.rule
    }

    pub fn category(&self) -> CandidateCategory {
        self.category
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }

    pub fn newest_mtime(&self) -> Option<SystemTime> {
        self.newest_mtime
    }

    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }

    pub fn safety(&self) -> SafetyTier {
        self.safety
    }

    pub fn action(&self) -> CandidateAction {
        self.action
    }

    pub fn rule_marks_schedule_eligible(&self) -> bool {
        self.rule_schedule_eligible
    }
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

fn reject_path_overlaps(candidates: &[Candidate]) -> Result<(), CleanupPlanValidationError> {
    let mut paths = candidates
        .iter()
        .enumerate()
        .flat_map(|(candidate_index, candidate)| {
            candidate
                .paths()
                .iter()
                .enumerate()
                .map(move |(path_index, path)| (path.as_path(), candidate_index, path_index))
        })
        .collect::<Vec<_>>();
    paths.sort_unstable_by(|left, right| left.0.cmp(right.0));

    for pair in paths.windows(2) {
        let (first_path, first_candidate_index, first_path_index) = pair[0];
        let (second_path, second_candidate_index, second_path_index) = pair[1];
        if paths_overlap(first_path, second_path) {
            return Err(CleanupPlanValidationError::OverlappingPaths {
                first_candidate_index,
                first_path_index,
                second_candidate_index,
                second_path_index,
            });
        }
    }
    Ok(())
}

fn reject_fact_path_overlaps(
    facts: &[CleanupPlanCandidateFacts],
) -> Result<(), CleanupPlanValidationError> {
    let mut paths = facts
        .iter()
        .enumerate()
        .flat_map(|(candidate_index, fact)| {
            fact.paths
                .iter()
                .enumerate()
                .map(move |(path_index, path)| (path.as_path(), candidate_index, path_index))
        })
        .collect::<Vec<_>>();
    paths.sort_unstable_by(|left, right| left.0.cmp(right.0));
    for pair in paths.windows(2) {
        let (first_path, first_candidate_index, first_path_index) = pair[0];
        let (second_path, second_candidate_index, second_path_index) = pair[1];
        if paths_overlap(first_path, second_path) {
            return Err(CleanupPlanValidationError::OverlappingPaths {
                first_candidate_index,
                first_path_index,
                second_candidate_index,
                second_path_index,
            });
        }
    }
    Ok(())
}

fn paths_overlap(first: &Path, second: &Path) -> bool {
    first == second || first.starts_with(second) || second.starts_with(first)
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

fn derive_fact_warnings(
    mode: CleanupMode,
    facts: &[CleanupPlanCandidateFacts],
) -> Vec<PlanWarning> {
    let dry_run = mode == CleanupMode::DryRun;
    let has_action = |action| facts.iter().any(|fact| fact.action == action);
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

fn derive_item_warnings(mode: CleanupMode, items: &[CleanupPlanItem]) -> Vec<PlanWarning> {
    let dry_run = mode == CleanupMode::DryRun;
    let has_action = |action| items.iter().any(|item| item.action == action);
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
#[path = "cleanup_plan_tests.rs"]
mod tests;

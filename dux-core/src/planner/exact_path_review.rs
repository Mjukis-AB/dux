//! Sealed exact-path review evidence for a future cleanup planner.
//!
//! This boundary joins a deterministic candidate selection to one code-owned
//! canonical scan-root observation. It performs lexical validation and a
//! no-follow live identity capture for every selected path, but it deliberately
//! stops before protected-root grants, approval, plan construction, or an
//! executor capability exist.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::domain::{
    Candidate, CandidateAction, CandidateCategory, CandidateGroupingError, CandidateId,
    CandidateOverlapReason, CleanupMode, Evidence, PlanWarning, RuleRef, SafetyTier, ScanId,
};
use crate::path_validation::{
    CanonicalPathError, CanonicalPathSnapshot, CanonicalScanRoot, LexicalPathError,
    validate_cleanup_path, validate_scan_root,
};

pub(crate) const MAX_EXACT_REVIEW_CANDIDATES: usize = 64;
pub(crate) const MAX_EXACT_REVIEW_PATHS: usize = 256;

/// The protected-root policy is intentionally not treated as available yet.
/// A lexical match or a successful filesystem probe can never turn this into
/// permission to mutate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExactPathProtection {
    TrustedPolicyNotAvailable,
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
    source_scan_id: ScanId,
    mode: CleanupMode,
    items: Vec<ExactPathReviewItem>,
    estimated_bytes: u64,
    warnings: Vec<PlanWarning>,
}

impl ExactPathReview {
    pub(crate) fn scan_root(&self) -> &Path {
        &self.scan_root
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

    /// No current review can be actionable because protected-root and volume
    /// grants have not yet been bound to the planner-owned witness.
    pub(crate) const fn is_actionable(&self) -> bool {
        false
    }
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
    let mut estimated_bytes = 0_u64;
    for candidate_index in selected_indices {
        let candidate = &candidates[candidate_index];
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
            let snapshot = crate::path_validation::capture_path_snapshot(scan_root, lexical_path)
                .map_err(|source| ExactPathReviewError::PathLive {
                candidate_index,
                path_index,
                source,
            })?;
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
                protection: ExactPathProtection::TrustedPolicyNotAvailable,
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

    Ok(ExactPathReview {
        scan_root: scan_root.requested_path().to_path_buf(),
        source_scan_id: candidates[0].source_scan_id().clone(),
        mode,
        items,
        estimated_bytes,
        warnings: derive_warnings(mode, candidates),
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

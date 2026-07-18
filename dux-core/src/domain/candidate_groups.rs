//! Deterministic presentation grouping and conservative candidate overlap facts.
//!
//! This module does not create a cleanup plan or grant any filesystem
//! authority. It preserves every candidate observation, records which
//! equivalent targets may be coalesced, and marks ambiguous ownership as
//! unresolved so a later planner can fail closed.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};

use thiserror::Error;

use super::{Candidate, CandidateAction, CandidateCategory, CandidateId, SafetyTier, ScanId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CandidateGroupKey {
    pub(crate) category: CandidateCategory,
    pub(crate) safety: SafetyTier,
    pub(crate) action: CandidateAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateGroup {
    key: CandidateGroupKey,
    member_indices: Vec<usize>,
    selected_indices: Vec<usize>,
    estimated_bytes: u64,
    actionable_bytes: u64,
    blocked_member_count: usize,
}

impl CandidateGroup {
    pub(crate) const fn key(&self) -> CandidateGroupKey {
        self.key
    }

    pub(crate) fn member_indices(&self) -> &[usize] {
        &self.member_indices
    }

    pub(crate) fn selected_indices(&self) -> &[usize] {
        &self.selected_indices
    }

    pub(crate) const fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }

    pub(crate) const fn actionable_bytes(&self) -> u64 {
        self.actionable_bytes
    }

    pub(crate) const fn blocked_member_count(&self) -> usize {
        self.blocked_member_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateOverlapDecision {
    first_index: usize,
    second_index: usize,
    resolution: CandidateOverlapResolution,
}

impl CandidateOverlapDecision {
    pub(crate) const fn first_index(&self) -> usize {
        self.first_index
    }

    pub(crate) const fn second_index(&self) -> usize {
        self.second_index
    }

    pub(crate) const fn resolution(&self) -> CandidateOverlapResolution {
        self.resolution
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateOverlapResolution {
    Coalesced {
        retained_index: usize,
        suppressed_index: usize,
    },
    Unresolved(CandidateOverlapReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateOverlapReason {
    BlockedCandidate,
    ConflictingFacts,
    DifferentRule,
    DifferentPolicy,
    InternalCandidateOverlap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateGroupSet {
    source_scan_id: ScanId,
    groups: Vec<CandidateGroup>,
    overlap_decisions: Vec<CandidateOverlapDecision>,
}

impl CandidateGroupSet {
    pub(crate) fn source_scan_id(&self) -> &ScanId {
        &self.source_scan_id
    }

    pub(crate) fn groups(&self) -> &[CandidateGroup] {
        &self.groups
    }

    pub(crate) fn overlap_decisions(&self) -> &[CandidateOverlapDecision] {
        &self.overlap_decisions
    }

    pub(crate) fn has_unresolved_overlaps(&self) -> bool {
        self.overlap_decisions.iter().any(|decision| {
            matches!(
                decision.resolution,
                CandidateOverlapResolution::Unresolved(_)
            )
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum CandidateGroupingError {
    #[error("candidate grouping requires at least one candidate")]
    MissingCandidates,
    #[error("candidate {duplicate_index} duplicates candidate {first_index}")]
    DuplicateCandidateId {
        first_index: usize,
        duplicate_index: usize,
    },
    #[error("candidate {candidate_index} belongs to a different scan")]
    MixedSourceScan { candidate_index: usize },
    #[error("candidate {candidate_index} has an invalid path shape")]
    InvalidPath { candidate_index: usize },
    #[error("candidate {candidate_index} has an estimated-byte overflow")]
    EstimatedBytesOverflow { candidate_index: usize },
}

/// Group complete candidates and conservatively resolve only equivalent
/// same-policy overlaps. The input order never determines a winner.
pub(crate) fn group_candidates(
    candidates: &[Candidate],
) -> Result<CandidateGroupSet, CandidateGroupingError> {
    let Some(first) = candidates.first() else {
        return Err(CandidateGroupingError::MissingCandidates);
    };

    let source_scan_id = first.source_scan_id().clone();
    let mut first_ids = HashMap::<&CandidateId, usize>::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if candidate.source_scan_id() != &source_scan_id {
            return Err(CandidateGroupingError::MixedSourceScan {
                candidate_index: index,
            });
        }
        if let Some(first_index) = first_ids.insert(candidate.id(), index) {
            return Err(CandidateGroupingError::DuplicateCandidateId {
                first_index,
                duplicate_index: index,
            });
        }
        if candidate
            .paths()
            .iter()
            .any(|path| !is_absolute_clean_path(path))
        {
            return Err(CandidateGroupingError::InvalidPath {
                candidate_index: index,
            });
        }
    }

    let mut decisions = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if let Some(reason) = internal_overlap_reason(candidate) {
            decisions.push(CandidateOverlapDecision {
                first_index: index,
                second_index: index,
                resolution: CandidateOverlapResolution::Unresolved(reason),
            });
        }
    }
    for index in 0..candidates.len() {
        for other in (index + 1)..candidates.len() {
            if !candidates_overlap(&candidates[index], &candidates[other]) {
                continue;
            }
            decisions.push(resolve_pair(
                &candidates[index],
                &candidates[other],
                index,
                other,
            ));
        }
    }
    decisions.sort_by_key(|decision| {
        (
            candidates[decision.first_index].id().as_str(),
            candidates[decision.second_index].id().as_str(),
        )
    });

    let mut suppressed = HashSet::new();
    for decision in &decisions {
        if let CandidateOverlapResolution::Coalesced {
            suppressed_index, ..
        } = decision.resolution
        {
            suppressed.insert(suppressed_index);
        }
    }

    let mut group_indices = HashMap::<CandidateGroupKey, Vec<usize>>::new();
    for (index, candidate) in candidates.iter().enumerate() {
        group_indices
            .entry(CandidateGroupKey {
                category: candidate.category(),
                safety: candidate.safety(),
                action: candidate.action(),
            })
            .or_default()
            .push(index);
    }
    let mut group_keys = group_indices.keys().copied().collect::<Vec<_>>();
    group_keys.sort_by_key(|key| (key.category as u8, key.safety as u8, key.action as u8));

    let mut groups = Vec::with_capacity(group_keys.len());
    for key in group_keys {
        let mut member_indices = group_indices.remove(&key).expect("group key exists");
        member_indices.sort_by_key(|index| candidates[*index].id().as_str());
        let selected_indices = member_indices
            .iter()
            .copied()
            .filter(|index| !suppressed.contains(index))
            .collect::<Vec<_>>();
        let estimated_bytes = sum_estimates(candidates, &selected_indices).ok_or(
            CandidateGroupingError::EstimatedBytesOverflow {
                candidate_index: *selected_indices.last().unwrap_or(&member_indices[0]),
            },
        )?;
        let actionable_bytes = if decisions.iter().any(|decision| {
            matches!(
                decision.resolution,
                CandidateOverlapResolution::Unresolved(_)
            ) && (member_indices.contains(&decision.first_index)
                || member_indices.contains(&decision.second_index))
        }) {
            0
        } else {
            let actionable_indices = selected_indices
                .iter()
                .filter(|index| candidates[**index].has_no_known_blockers())
                .copied()
                .collect::<Vec<_>>();
            sum_estimates(candidates, &actionable_indices).ok_or(
                CandidateGroupingError::EstimatedBytesOverflow {
                    candidate_index: *actionable_indices.last().unwrap_or(&member_indices[0]),
                },
            )?
        };
        groups.push(CandidateGroup {
            key,
            member_indices,
            selected_indices,
            estimated_bytes,
            actionable_bytes,
            blocked_member_count: candidates
                .iter()
                .enumerate()
                .filter(|(index, candidate)| {
                    group_indices_key(candidate) == key && !candidates[*index].blockers().is_empty()
                })
                .count(),
        });
    }

    Ok(CandidateGroupSet {
        source_scan_id,
        groups,
        overlap_decisions: decisions,
    })
}

fn group_indices_key(candidate: &Candidate) -> CandidateGroupKey {
    CandidateGroupKey {
        category: candidate.category(),
        safety: candidate.safety(),
        action: candidate.action(),
    }
}

fn sum_estimates(candidates: &[Candidate], indices: &[usize]) -> Option<u64> {
    indices
        .iter()
        .map(|index| candidates[*index].estimated_bytes())
        .try_fold(0_u64, u64::checked_add)
}

fn is_absolute_clean_path(path: &Path) -> bool {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return false;
    }

    let mut components = path.components();
    let first = components.next();
    let second = components.next();
    let is_bare_root = match (first, second) {
        (Some(Component::RootDir), None) => true,
        (Some(Component::Prefix(_)), Some(Component::RootDir)) => components.next().is_none(),
        _ => false,
    };
    !is_bare_root
}

fn internal_overlap_reason(candidate: &Candidate) -> Option<CandidateOverlapReason> {
    for (index, first) in candidate.paths().iter().enumerate() {
        if candidate.paths()[index + 1..]
            .iter()
            .any(|second| paths_overlap(first, second))
        {
            return Some(CandidateOverlapReason::InternalCandidateOverlap);
        }
    }
    None
}

fn candidates_overlap(first: &Candidate, second: &Candidate) -> bool {
    first.paths().iter().any(|first_path| {
        second
            .paths()
            .iter()
            .any(|second_path| paths_overlap(first_path, second_path))
    })
}

fn resolve_pair(
    first: &Candidate,
    second: &Candidate,
    first_index: usize,
    second_index: usize,
) -> CandidateOverlapDecision {
    let resolution = if first.blockers().is_empty() && second.blockers().is_empty() {
        if equivalent_facts(first, second) {
            let (retained_index, suppressed_index) = if first.id().as_str() <= second.id().as_str()
            {
                (first_index, second_index)
            } else {
                (second_index, first_index)
            };
            CandidateOverlapResolution::Coalesced {
                retained_index,
                suppressed_index,
            }
        } else if first.rule() != second.rule() {
            CandidateOverlapResolution::Unresolved(CandidateOverlapReason::DifferentRule)
        } else if !same_policy(first, second) {
            CandidateOverlapResolution::Unresolved(CandidateOverlapReason::DifferentPolicy)
        } else if parent_owns_all_paths(first, second) {
            CandidateOverlapResolution::Coalesced {
                retained_index: first_index,
                suppressed_index: second_index,
            }
        } else if parent_owns_all_paths(second, first) {
            CandidateOverlapResolution::Coalesced {
                retained_index: second_index,
                suppressed_index: first_index,
            }
        } else {
            CandidateOverlapResolution::Unresolved(CandidateOverlapReason::ConflictingFacts)
        }
    } else {
        CandidateOverlapResolution::Unresolved(CandidateOverlapReason::BlockedCandidate)
    };
    CandidateOverlapDecision {
        first_index,
        second_index,
        resolution,
    }
}

fn equivalent_facts(first: &Candidate, second: &Candidate) -> bool {
    first.rule() == second.rule()
        && first.category() == second.category()
        && first.paths() == second.paths()
        && first.estimated_bytes() == second.estimated_bytes()
        && first.newest_mtime() == second.newest_mtime()
        && first.evidence() == second.evidence()
        && first.safety() == second.safety()
        && first.action() == second.action()
        && first.rule_marks_schedule_eligible() == second.rule_marks_schedule_eligible()
        && first.blockers() == second.blockers()
        && first.source_scan_id() == second.source_scan_id()
}

fn same_policy(first: &Candidate, second: &Candidate) -> bool {
    first.category() == second.category()
        && first.safety() == second.safety()
        && first.action() == second.action()
        && first.rule_marks_schedule_eligible() == second.rule_marks_schedule_eligible()
}

fn parent_owns_all_paths(parent: &Candidate, child: &Candidate) -> bool {
    child.paths().iter().all(|child_path| {
        parent
            .paths()
            .iter()
            .any(|parent_path| parent_path != child_path && child_path.starts_with(parent_path))
    })
}

fn paths_overlap(first: &Path, second: &Path) -> bool {
    first == second || first.starts_with(second) || second.starts_with(first)
}

#[cfg(test)]
#[path = "candidate_groups_tests.rs"]
mod tests;

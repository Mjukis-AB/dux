use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use thiserror::Error;

use super::{
    CandidateBatch, CandidateEvaluationError, MAX_EVALUATED_CANDIDATES, ObservedArtifact,
    evaluate_observed_artifacts,
};
use crate::domain::{Candidate, ScanCoverage, ScanCoverageStatus};
use crate::persistence::snapshot::{
    HostValue, SnapshotNodeKind, SnapshotReviewDocument, SnapshotTimestamp,
};
use crate::persistence::{CandidateBatchMaterializationBudget, CompleteCandidateRecord};
use crate::projection::{
    ARTIFACT_PATTERNS, ArtifactKind, ArtifactMarkerLocation, ArtifactMarkerMatch, ArtifactPattern,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(crate) enum CandidateSnapshotReplayError {
    #[error("the retained snapshot cannot reproduce the candidate observation")]
    InvalidSnapshot,
    #[error("the retained snapshot candidate evaluation failed: {0}")]
    Evaluation(CandidateEvaluationError),
    #[error("the retained snapshot candidate batch differs from durable history")]
    BatchMismatch,
    #[error("the retained snapshot candidate batch exceeds the materialization budget")]
    ResourceLimit,
}

#[derive(Default)]
struct DirectoryMarkers {
    exact: Vec<(&'static str, u64)>,
    extension_py: Option<u64>,
}

impl DirectoryMarkers {
    fn exact(&self, names: &[&str]) -> Option<u64> {
        names.iter().find_map(|expected| {
            self.exact
                .iter()
                .find_map(|(name, node)| (*name == *expected).then_some(*node))
        })
    }

    fn extension(&self, extension: &str) -> Option<u64> {
        (extension == "py").then_some(self.extension_py).flatten()
    }
}

struct PendingArtifact {
    component: &'static str,
    kind: ArtifactKind,
    evidence_nodes: Vec<u64>,
}

struct DirectoryFrame {
    node: u64,
    depth: u32,
    suppresses_descendants: bool,
    markers: DirectoryMarkers,
    newest_mtime: Option<SystemTime>,
    mtime_coverage_complete: bool,
    known_allocated_bytes: u64,
    pending: Option<PendingArtifact>,
}

pub(crate) fn verify_snapshot_candidate_evaluation(
    document: &SnapshotReviewDocument,
    coverage: &ScanCoverage,
    expected: &[CompleteCandidateRecord],
    evaluated_at: SystemTime,
) -> Result<(), CandidateSnapshotReplayError> {
    let replayed = replay_snapshot_candidate_evaluation(document, coverage, evaluated_at)?;
    exact_batch_matches(&replayed, expected)
        .then_some(())
        .ok_or(CandidateSnapshotReplayError::BatchMismatch)
}

pub(crate) fn replay_snapshot_candidate_evaluation(
    document: &SnapshotReviewDocument,
    coverage: &ScanCoverage,
    evaluated_at: SystemTime,
) -> Result<CandidateBatch, CandidateSnapshotReplayError> {
    let root = document
        .metadata
        .root
        .to_path_buf()
        .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?;
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(
            usize::try_from(crate::persistence::snapshot::MAX_SNAPSHOT_DEPTH)
                .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?
                .saturating_add(1),
        )
        .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?;
    let mut artifacts = Vec::new();
    let mut observed_matches = 0_usize;
    let mut materialization_budget = CandidateBatchMaterializationBudget::default();
    let base_blocker_count = 1 + usize::from(coverage.status() != ScanCoverageStatus::Complete);

    for (index, node) in document.nodes.iter().enumerate() {
        while frames
            .last()
            .is_some_and(|frame: &DirectoryFrame| frame.depth >= node.depth)
        {
            close_directory(
                document,
                &root,
                &mut frames,
                &mut artifacts,
                &mut materialization_budget,
                base_blocker_count,
                evaluated_at,
            )?;
        }

        if node.kind == SnapshotNodeKind::Directory {
            let markers = directory_markers(document, index)?;
            let ancestor_classified = frames
                .last()
                .is_some_and(|frame| frame.suppresses_descendants);
            let classification = classify_directory(
                node.name.as_ref(),
                frames.last().map(|frame| &frame.markers),
                &markers,
            );
            let classified = classification.is_some();
            let pending = if classification.is_some() && !ancestor_classified {
                observed_matches = observed_matches.saturating_add(1);
                if observed_matches > MAX_EVALUATED_CANDIDATES {
                    return Err(CandidateSnapshotReplayError::Evaluation(
                        CandidateEvaluationError::CandidateLimitExceeded {
                            observed_at_least: observed_matches,
                            maximum: MAX_EVALUATED_CANDIDATES,
                        },
                    ));
                }
                classification.map(|(pattern, evidence_nodes)| PendingArtifact {
                    component: pattern.component,
                    kind: pattern.kind,
                    evidence_nodes,
                })
            } else {
                None
            };
            frames.push(DirectoryFrame {
                node: node.id,
                depth: node.depth,
                suppresses_descendants: ancestor_classified || classified,
                markers,
                newest_mtime: node
                    .modified_at
                    .map(snapshot_time)
                    .transpose()
                    .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?,
                mtime_coverage_complete: node.modified_at.is_some(),
                known_allocated_bytes: 0,
                pending,
            });
        } else {
            let parent = frames
                .last_mut()
                .ok_or(CandidateSnapshotReplayError::InvalidSnapshot)?;
            parent.known_allocated_bytes = parent
                .known_allocated_bytes
                .checked_add(node.allocated_bytes.unwrap_or(0))
                .ok_or(CandidateSnapshotReplayError::InvalidSnapshot)?;
            if let Some(modified_at) = node.modified_at {
                update_newest(
                    &mut parent.newest_mtime,
                    snapshot_time(modified_at)
                        .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?,
                );
            } else if node.kind == SnapshotNodeKind::File {
                parent.mtime_coverage_complete = false;
            }
        }
    }
    while !frames.is_empty() {
        close_directory(
            document,
            &root,
            &mut frames,
            &mut artifacts,
            &mut materialization_budget,
            base_blocker_count,
            evaluated_at,
        )?;
    }

    evaluate_observed_artifacts(
        &document.metadata.scan_id,
        &root,
        coverage,
        artifacts,
        evaluated_at,
    )
    .map_err(map_evaluation)
}

fn close_directory(
    document: &SnapshotReviewDocument,
    root: &std::path::Path,
    frames: &mut Vec<DirectoryFrame>,
    artifacts: &mut Vec<ObservedArtifact>,
    materialization_budget: &mut CandidateBatchMaterializationBudget,
    base_blocker_count: usize,
    evaluated_at: SystemTime,
) -> Result<(), CandidateSnapshotReplayError> {
    let frame = frames
        .pop()
        .ok_or(CandidateSnapshotReplayError::InvalidSnapshot)?;
    if let Some(pending) = frame.pending {
        let path = historical_path(document, root, frame.node)?;
        let evidence_paths = pending
            .evidence_nodes
            .into_iter()
            .map(|node| historical_path(document, root, node))
            .collect::<Result<Vec<_>, _>>()?;
        let is_rust_target = pending.component == "target" && pending.kind == ArtifactKind::Rust;
        let recency_satisfied = is_rust_target
            && frame.mtime_coverage_complete
            && frame.newest_mtime.is_some_and(|newest_mtime| {
                evaluated_at
                    .duration_since(newest_mtime)
                    .is_ok_and(|age| age >= super::SAFE_RUST_RULE_MINIMUM_AGE)
            });
        let blocker_count = base_blocker_count + usize::from(is_rust_target && !recency_satisfied);
        let scalar_evidence_count = usize::from(recency_satisfied);
        if !materialization_budget
            .charge_observed_candidate(&path, &evidence_paths, scalar_evidence_count, blocker_count)
            .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?
        {
            return Err(CandidateSnapshotReplayError::ResourceLimit);
        }
        artifacts
            .try_reserve(1)
            .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?;
        artifacts.push(ObservedArtifact {
            path,
            component: pending.component,
            kind: pending.kind,
            size: frame.known_allocated_bytes,
            newest_mtime: frame.newest_mtime,
            mtime_coverage_complete: frame.mtime_coverage_complete,
            evidence_paths,
        });
    }
    if let Some(parent) = frames.last_mut() {
        parent.known_allocated_bytes = parent
            .known_allocated_bytes
            .checked_add(frame.known_allocated_bytes)
            .ok_or(CandidateSnapshotReplayError::InvalidSnapshot)?;
        if let Some(newest) = frame.newest_mtime {
            update_newest(&mut parent.newest_mtime, newest);
        }
        parent.mtime_coverage_complete &= frame.mtime_coverage_complete;
    }
    Ok(())
}

fn directory_markers(
    document: &SnapshotReviewDocument,
    directory: usize,
) -> Result<DirectoryMarkers, CandidateSnapshotReplayError> {
    let children = document
        .direct_child_indices(directory)
        .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?;
    let mut markers = DirectoryMarkers::default();
    for child in children {
        let child = document
            .nodes
            .get(
                usize::try_from(*child)
                    .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?,
            )
            .ok_or(CandidateSnapshotReplayError::InvalidSnapshot)?;
        if child.kind != SnapshotNodeKind::File {
            continue;
        }
        let name = child
            .name
            .as_ref()
            .ok_or(CandidateSnapshotReplayError::InvalidSnapshot)?;
        if name.has_ascii_extension("py")
            && markers.extension_py.is_none_or(|current| {
                document.nodes[current as usize]
                    .name
                    .as_ref()
                    .is_some_and(|current| name.bytes() < current.bytes())
            })
        {
            markers.extension_py = Some(child.id);
        }
        for expected in exact_marker_names() {
            if name.matches_ascii_component(expected)
                && !markers.exact.iter().any(|(name, _)| *name == expected)
            {
                markers
                    .exact
                    .try_reserve(1)
                    .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?;
                markers.exact.push((expected, child.id));
            }
        }
    }
    Ok(markers)
}

fn exact_marker_names() -> impl Iterator<Item = &'static str> {
    ARTIFACT_PATTERNS
        .iter()
        .flat_map(|pattern| pattern.markers.iter())
        .filter_map(|requirement| match requirement.matcher {
            ArtifactMarkerMatch::Exact(names) => Some(names),
            ArtifactMarkerMatch::Extension(_) => None,
        })
        .flatten()
        .copied()
}

fn classify_directory(
    name: Option<&HostValue>,
    parent: Option<&DirectoryMarkers>,
    own: &DirectoryMarkers,
) -> Option<(&'static ArtifactPattern, Vec<u64>)> {
    let name = name?;
    let pattern = ARTIFACT_PATTERNS
        .iter()
        .find(|pattern| name.matches_ascii_component(pattern.component))?;
    let mut evidence = Vec::with_capacity(pattern.markers.len());
    for requirement in pattern.markers {
        let markers = match requirement.location {
            ArtifactMarkerLocation::Sibling => parent?,
            ArtifactMarkerLocation::Child => own,
        };
        let node = match requirement.matcher {
            ArtifactMarkerMatch::Exact(names) => markers.exact(names)?,
            ArtifactMarkerMatch::Extension(extension) => markers.extension(extension)?,
        };
        evidence.push(node);
    }
    Some((pattern, evidence))
}

fn historical_path(
    document: &SnapshotReviewDocument,
    root: &std::path::Path,
    node: u64,
) -> Result<PathBuf, CandidateSnapshotReplayError> {
    let mut components = Vec::new();
    let mut current = node;
    while current != 0 {
        let observed = document
            .nodes
            .get(
                usize::try_from(current)
                    .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?,
            )
            .ok_or(CandidateSnapshotReplayError::InvalidSnapshot)?;
        components
            .try_reserve(1)
            .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?;
        components.push(
            observed
                .name
                .as_ref()
                .ok_or(CandidateSnapshotReplayError::InvalidSnapshot)?,
        );
        current = observed
            .parent
            .ok_or(CandidateSnapshotReplayError::InvalidSnapshot)?;
    }
    let mut path = root.to_path_buf();
    for component in components.into_iter().rev() {
        path.push(
            component
                .to_path_buf()
                .map_err(|_| CandidateSnapshotReplayError::InvalidSnapshot)?,
        );
    }
    Ok(path)
}

fn snapshot_time(value: SnapshotTimestamp) -> Result<SystemTime, ()> {
    UNIX_EPOCH
        .checked_add(Duration::new(
            value.seconds_since_unix_epoch(),
            value.nanoseconds(),
        ))
        .ok_or(())
}

fn update_newest(current: &mut Option<SystemTime>, observed: SystemTime) {
    *current = Some(current.map_or(observed, |current| current.max(observed)));
}

fn exact_batch_matches(batch: &CandidateBatch, expected: &[CompleteCandidateRecord]) -> bool {
    if batch.candidates.len() != expected.len() {
        return false;
    }
    let expected = expected
        .iter()
        .map(|candidate| (candidate.id(), candidate))
        .collect::<HashMap<_, _>>();
    expected.len() == batch.candidates.len()
        && batch.candidates.iter().all(|candidate| {
            expected
                .get(candidate.id())
                .is_some_and(|stored| exact_candidate_matches(candidate, stored))
        })
}

fn exact_candidate_matches(candidate: &Candidate, stored: &CompleteCandidateRecord) -> bool {
    candidate.id() == stored.id()
        && candidate.source_scan_id() == stored.source_scan_id()
        && candidate.rule() == stored.rule()
        && candidate.category() == stored.category()
        && candidate.paths() == stored.paths()
        && candidate.estimated_bytes() == stored.estimated_bytes()
        && candidate.newest_mtime() == stored.newest_mtime()
        && candidate.evidence() == stored.evidence()
        && candidate.safety() == stored.safety()
        && candidate.action() == stored.action()
        && candidate.rule_marks_schedule_eligible() == stored.rule_schedule_eligible()
        && candidate.blockers() == stored.blockers()
}

fn map_evaluation(error: CandidateEvaluationError) -> CandidateSnapshotReplayError {
    CandidateSnapshotReplayError::Evaluation(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ScanCoverage, ScanId};
    use crate::persistence::snapshot::{
        HostValue, SnapshotDocument, SnapshotMetadata, SnapshotNode, SnapshotScanFlags,
        SnapshotTotals,
    };

    #[test]
    fn exact_candidate_limit_replays_without_truncation() {
        let coverage = ScanCoverage::from_validated_terminal_issues(Vec::new());
        let evaluated_at = UNIX_EPOCH + Duration::from_secs(30 * 86_400);
        let maximum =
            SnapshotReviewDocument::from_document_for_test(rust_targets(MAX_EVALUATED_CANDIDATES))
                .unwrap();
        let batch =
            replay_snapshot_candidate_evaluation(&maximum, &coverage, evaluated_at).unwrap();
        assert_eq!(batch.candidates.len(), MAX_EVALUATED_CANDIDATES);

        let overflow = SnapshotReviewDocument::from_document_for_test(rust_targets(
            MAX_EVALUATED_CANDIDATES + 1,
        ))
        .unwrap();
        assert_eq!(
            replay_snapshot_candidate_evaluation(&overflow, &coverage, evaluated_at),
            Err(CandidateSnapshotReplayError::Evaluation(
                CandidateEvaluationError::CandidateLimitExceeded {
                    observed_at_least: MAX_EVALUATED_CANDIDATES + 1,
                    maximum: MAX_EVALUATED_CANDIDATES,
                }
            ))
        );
    }

    #[test]
    fn replay_refuses_candidate_paths_beyond_the_durable_batch_budget() {
        let coverage = ScanCoverage::from_validated_terminal_issues(Vec::new());
        let evaluated_at = UNIX_EPOCH + Duration::from_secs(30 * 86_400);
        let document =
            SnapshotReviewDocument::from_document_for_test(deep_rust_targets(200, 30, 900))
                .unwrap();
        assert_eq!(
            replay_snapshot_candidate_evaluation(&document, &coverage, evaluated_at),
            Err(CandidateSnapshotReplayError::ResourceLimit)
        );
    }

    #[test]
    fn replay_is_clock_bound_and_matches_special_node_mtime_semantics() {
        let coverage = ScanCoverage::from_validated_terminal_issues(Vec::new());
        let old = SnapshotTimestamp::new(10, 0).unwrap();
        let mut document = rust_targets(1);
        document.nodes[3].modified_at = Some(old);
        document.nodes[3].child_count = 2;
        document.nodes[4].modified_at = Some(old);
        document.nodes.push(node(
            5,
            Some(3),
            3,
            SnapshotNodeKind::Other,
            Some("socket-like-entry"),
            0,
            0,
            0,
        ));
        let document = SnapshotReviewDocument::from_document_for_test(document).unwrap();
        let evaluated_at =
            UNIX_EPOCH + Duration::from_secs(10) + super::super::SAFE_RUST_RULE_MINIMUM_AGE;

        let first =
            replay_snapshot_candidate_evaluation(&document, &coverage, evaluated_at).unwrap();
        let repeated =
            replay_snapshot_candidate_evaluation(&document, &coverage, evaluated_at).unwrap();
        assert_eq!(first.candidates(), repeated.candidates());
        assert_eq!(
            first.context_digest_sha256(),
            repeated.context_digest_sha256()
        );
        assert_eq!(
            first.candidates()[0].blockers(),
            [crate::domain::BlockReason::ProtectedPath]
        );
        assert!(
            first.candidates()[0]
                .evidence()
                .iter()
                .any(|evidence| matches!(evidence, crate::domain::Evidence::MinimumAge { .. }))
        );

        let shifted = replay_snapshot_candidate_evaluation(
            &document,
            &coverage,
            evaluated_at + Duration::from_nanos(1),
        )
        .unwrap();
        assert_ne!(
            first.context_digest_sha256(),
            shifted.context_digest_sha256()
        );
        let too_early = replay_snapshot_candidate_evaluation(
            &document,
            &coverage,
            evaluated_at - Duration::from_nanos(1),
        )
        .unwrap();
        assert_eq!(
            too_early.candidates()[0].blockers(),
            [
                crate::domain::BlockReason::RecentActivity,
                crate::domain::BlockReason::ProtectedPath,
            ]
        );
        assert!(
            !too_early.candidates()[0]
                .evidence()
                .iter()
                .any(|evidence| matches!(evidence, crate::domain::Evidence::MinimumAge { .. }))
        );

        let mut missing_file_time = rust_targets(1);
        missing_file_time.nodes[3].modified_at = Some(old);
        let missing_file_time =
            SnapshotReviewDocument::from_document_for_test(missing_file_time).unwrap();
        let missing =
            replay_snapshot_candidate_evaluation(&missing_file_time, &coverage, evaluated_at)
                .unwrap();
        assert_eq!(
            missing.candidates()[0].blockers(),
            [
                crate::domain::BlockReason::MissingModificationTime,
                crate::domain::BlockReason::ProtectedPath,
            ]
        );
    }

    fn rust_targets(count: usize) -> SnapshotDocument {
        let mut nodes = Vec::with_capacity(count.saturating_mul(4).saturating_add(1));
        let total_bytes = u64::try_from(count).unwrap().checked_mul(2).unwrap();
        nodes.push(node(
            0,
            None,
            0,
            SnapshotNodeKind::Directory,
            None,
            total_bytes,
            u64::try_from(count).unwrap().checked_mul(2).unwrap(),
            u64::try_from(count).unwrap(),
        ));
        for project in 0..count {
            let project_id = nodes.len() as u64;
            nodes.push(node(
                project_id,
                Some(0),
                1,
                SnapshotNodeKind::Directory,
                Some(&format!("project-{project:04}")),
                2,
                2,
                2,
            ));
            nodes.push(node(
                nodes.len() as u64,
                Some(project_id),
                2,
                SnapshotNodeKind::File,
                Some("Cargo.toml"),
                1,
                1,
                0,
            ));
            let target_id = nodes.len() as u64;
            nodes.push(node(
                target_id,
                Some(project_id),
                2,
                SnapshotNodeKind::Directory,
                Some("target"),
                1,
                1,
                1,
            ));
            nodes.push(node(
                nodes.len() as u64,
                Some(target_id),
                3,
                SnapshotNodeKind::File,
                Some("CACHEDIR.TAG"),
                1,
                1,
                0,
            ));
        }
        SnapshotDocument {
            metadata: SnapshotMetadata {
                scan_id: ScanId::new("scan:snapshot-replay-limit").unwrap(),
                root: HostValue::from_root(snapshot_test_root().as_path()).unwrap(),
                captured_at: SnapshotTimestamp::new(1_750_000_000, 0).unwrap(),
                totals: SnapshotTotals {
                    directory_count: u64::try_from(count)
                        .unwrap()
                        .checked_mul(2)
                        .unwrap()
                        .checked_add(1)
                        .unwrap(),
                    file_count: u64::try_from(count).unwrap().checked_mul(2).unwrap(),
                    logical_bytes: total_bytes,
                    allocated_bytes: Some(total_bytes),
                },
            },
            nodes,
        }
    }

    fn deep_rust_targets(
        count: usize,
        prefix_depth: usize,
        component_bytes: usize,
    ) -> SnapshotDocument {
        let total_bytes = u64::try_from(count).unwrap().checked_mul(2).unwrap();
        let total_files = u64::try_from(count).unwrap().checked_mul(2).unwrap();
        let mut nodes =
            Vec::with_capacity(prefix_depth.saturating_add(count.saturating_mul(4) + 1));
        nodes.push(node(
            0,
            None,
            0,
            SnapshotNodeKind::Directory,
            None,
            total_bytes,
            total_files,
            1,
        ));
        let mut parent = 0_u64;
        for depth in 0..prefix_depth {
            let id = nodes.len() as u64;
            let prefix = format!("prefix-{depth:04}-");
            let name = format!(
                "{prefix}{}",
                "x".repeat(component_bytes.saturating_sub(prefix.len()))
            );
            nodes.push(node(
                id,
                Some(parent),
                u32::try_from(depth + 1).unwrap(),
                SnapshotNodeKind::Directory,
                Some(&name),
                total_bytes,
                total_files,
                if depth + 1 == prefix_depth {
                    u64::try_from(count).unwrap()
                } else {
                    1
                },
            ));
            parent = id;
        }
        let project_depth = u32::try_from(prefix_depth + 1).unwrap();
        for project in 0..count {
            let project_id = nodes.len() as u64;
            nodes.push(node(
                project_id,
                Some(parent),
                project_depth,
                SnapshotNodeKind::Directory,
                Some(&format!("project-{project:04}")),
                2,
                2,
                2,
            ));
            nodes.push(node(
                nodes.len() as u64,
                Some(project_id),
                project_depth + 1,
                SnapshotNodeKind::File,
                Some("Cargo.toml"),
                1,
                1,
                0,
            ));
            let target_id = nodes.len() as u64;
            nodes.push(node(
                target_id,
                Some(project_id),
                project_depth + 1,
                SnapshotNodeKind::Directory,
                Some("target"),
                1,
                1,
                1,
            ));
            nodes.push(node(
                nodes.len() as u64,
                Some(target_id),
                project_depth + 2,
                SnapshotNodeKind::File,
                Some("CACHEDIR.TAG"),
                1,
                1,
                0,
            ));
        }
        SnapshotDocument {
            metadata: SnapshotMetadata {
                scan_id: ScanId::new("scan:snapshot-replay-resource-limit").unwrap(),
                root: HostValue::from_root(snapshot_test_root().as_path()).unwrap(),
                captured_at: SnapshotTimestamp::new(1_750_000_000, 0).unwrap(),
                totals: SnapshotTotals {
                    directory_count: u64::try_from(prefix_depth)
                        .unwrap()
                        .checked_add(u64::try_from(count).unwrap().checked_mul(2).unwrap())
                        .unwrap()
                        .checked_add(1)
                        .unwrap(),
                    file_count: total_files,
                    logical_bytes: total_bytes,
                    allocated_bytes: Some(total_bytes),
                },
            },
            nodes,
        }
    }

    fn snapshot_test_root() -> PathBuf {
        #[cfg(unix)]
        {
            PathBuf::from("/snapshot-replay")
        }
        #[cfg(windows)]
        {
            PathBuf::from(r"C:\snapshot-replay")
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn node(
        id: u64,
        parent: Option<u64>,
        depth: u32,
        kind: SnapshotNodeKind,
        name: Option<&str>,
        bytes: u64,
        file_count: u64,
        child_count: u64,
    ) -> SnapshotNode {
        SnapshotNode {
            id,
            parent,
            depth,
            kind,
            name: name.map(|name| HostValue::from_component(name.as_ref()).unwrap()),
            logical_bytes: bytes,
            allocated_bytes: Some(bytes),
            file_count,
            child_count,
            modified_at: None,
            accessed_at: None,
            scan_flags: SnapshotScanFlags::NONE,
            unix_identity: None,
        }
    }
}

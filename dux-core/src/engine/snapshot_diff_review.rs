//! Bounded, read-only comparison of two exact retained Explorer snapshots.
//!
//! Matching is performed only from lossless snapshot component bytes. The
//! projection is historical display evidence: it contains no current path,
//! candidate identity, reclaimability claim, plan, approval, or effect token.

use std::cmp::Ordering;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::SystemTime;

use crate::domain::{ScanCoverageStatus, ScanId};
use crate::persistence::snapshot::{
    HostValue, SnapshotNode as StoredNode, SnapshotNodeKind as StoredNodeKind,
};

use super::snapshot_review::{
    MAX_SNAPSHOT_REVIEW_NODE_PAGE_LIMIT, MAX_SNAPSHOT_REVIEW_TREEMAP_CELLS, SnapshotReviewCategory,
    SnapshotReviewError, SnapshotReviewName, SnapshotReviewNodeKind, SnapshotReviewReleaseOutcome,
    SnapshotReviewScanFlags, SnapshotReviewSession, map_repository_error,
};

const MAX_SNAPSHOT_DIFF_UNION_CHILDREN: usize = 999_999;
const OPAQUE_ID_SHIFT: u32 = 2;
const OPAQUE_ID_CURRENT: u64 = 1;
const OPAQUE_ID_BASELINE: u64 = 2;
const OPAQUE_ID_SIDE_MASK: u64 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotDiffNodeSort {
    NameAscending,
    MagnitudeDescending,
    CurrentBytesDescending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotDiffChange {
    Added,
    Removed,
    Grew,
    Shrank,
    Unchanged,
    Replaced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotDiffDirection {
    Growth,
    Shrinkage,
    Unchanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotDiffValue {
    pub direction: SnapshotDiffDirection,
    pub magnitude_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotDiffCoverage {
    pub status: ScanCoverageStatus,
    pub measured_permille: Option<u16>,
    pub issue_record_count: u64,
    pub issue_occurrence_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotDiffInfo {
    pub current_scan_id: ScanId,
    pub baseline_scan_id: ScanId,
    pub current_started_at: SystemTime,
    pub current_completed_at: SystemTime,
    pub baseline_started_at: SystemTime,
    pub baseline_completed_at: SystemTime,
    pub current_coverage: SnapshotDiffCoverage,
    pub baseline_coverage: SnapshotDiffCoverage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotDiffNode {
    /// Opaque only within this exact comparison session.
    pub id: u64,
    pub parent_id: Option<u64>,
    pub depth: u32,
    pub name: SnapshotReviewName,
    pub kind: SnapshotReviewNodeKind,
    pub category: SnapshotReviewCategory,
    pub change: SnapshotDiffChange,
    pub logical_change: SnapshotDiffValue,
    pub current_logical_bytes: Option<u64>,
    pub baseline_logical_bytes: Option<u64>,
    pub current_allocated_bytes: Option<u64>,
    pub baseline_allocated_bytes: Option<u64>,
    pub allocated_change: Option<SnapshotDiffValue>,
    pub current_file_count: Option<u64>,
    pub baseline_file_count: Option<u64>,
    pub current_child_count: Option<u64>,
    pub baseline_child_count: Option<u64>,
    pub current_scan_flags: Option<SnapshotReviewScanFlags>,
    pub baseline_scan_flags: Option<SnapshotReviewScanFlags>,
    pub can_descend: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotDiffNodePage {
    pub parent_id: u64,
    pub offset: u64,
    pub total_children: u64,
    pub has_more: bool,
    pub total_growth_bytes: u64,
    pub total_shrinkage_bytes: u64,
    pub unchanged_child_count: u64,
    pub replaced_child_count: u64,
    pub nodes: Vec<SnapshotDiffNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotDiffTreemapCell {
    pub node: SnapshotDiffNode,
    /// Rank among all non-zero changes in magnitude-descending order.
    pub magnitude_rank: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotDiffTreemap {
    pub parent_id: u64,
    pub total_children: u64,
    pub changed_child_count: u64,
    pub total_growth_bytes: u64,
    pub total_shrinkage_bytes: u64,
    pub other_growth_child_count: u64,
    pub other_growth_bytes: u64,
    pub other_shrinkage_child_count: u64,
    pub other_shrinkage_bytes: u64,
    pub unchanged_child_count: u64,
    pub replaced_child_count: u64,
    pub cells: Vec<SnapshotDiffTreemapCell>,
}

#[derive(Clone)]
pub(super) struct SnapshotDiffMetadata {
    pub(super) current_scan_id: ScanId,
    pub(super) baseline_scan_id: ScanId,
    pub(super) current_started_at: SystemTime,
    pub(super) current_completed_at: SystemTime,
    pub(super) baseline_started_at: SystemTime,
    pub(super) baseline_completed_at: SystemTime,
    pub(super) current_coverage: SnapshotDiffCoverage,
    pub(super) baseline_coverage: SnapshotDiffCoverage,
}

/// One baseline lease attached to one exact parent Explorer review.
///
/// Every projection requires the caller to supply and revalidate that same
/// parent. Releasing this comparison never releases the parent review.
#[must_use = "retain the comparison while Explorer is presenting snapshot changes"]
pub struct SnapshotDiffReviewSession {
    owner: Arc<super::snapshot_review::SnapshotReviewOwner>,
    parent_session_identity: u64,
    parent_liveness: Arc<AtomicBool>,
    metadata: SnapshotDiffMetadata,
    baseline: SnapshotReviewSession,
}

impl SnapshotDiffReviewSession {
    pub(super) fn new(
        owner: Arc<super::snapshot_review::SnapshotReviewOwner>,
        parent_session_identity: u64,
        parent_liveness: Arc<AtomicBool>,
        metadata: SnapshotDiffMetadata,
        baseline: SnapshotReviewSession,
    ) -> Self {
        Self {
            owner,
            parent_session_identity,
            parent_liveness,
            metadata,
            baseline,
        }
    }

    pub fn info(
        &mut self,
        current: &mut SnapshotReviewSession,
    ) -> Result<SnapshotDiffInfo, SnapshotReviewError> {
        self.validate_pair(current)?;
        Ok(self.metadata_info())
    }

    /// Return immutable identity and coverage metadata only after this child
    /// has released its baseline lease. This lets transports report terminal
    /// lifecycle state without reopening either snapshot.
    pub fn released_info(&self) -> Option<SnapshotDiffInfo> {
        self.is_released().then(|| self.metadata_info())
    }

    fn metadata_info(&self) -> SnapshotDiffInfo {
        SnapshotDiffInfo {
            current_scan_id: self.metadata.current_scan_id.clone(),
            baseline_scan_id: self.metadata.baseline_scan_id.clone(),
            current_started_at: self.metadata.current_started_at,
            current_completed_at: self.metadata.current_completed_at,
            baseline_started_at: self.metadata.baseline_started_at,
            baseline_completed_at: self.metadata.baseline_completed_at,
            current_coverage: self.metadata.current_coverage,
            baseline_coverage: self.metadata.baseline_coverage,
        }
    }

    pub fn renew(
        &mut self,
        current: &mut SnapshotReviewSession,
    ) -> Result<SystemTime, SnapshotReviewError> {
        self.validate_parent(current)?;
        current.validate_current()?;
        let baseline_expiry = self.baseline.renew()?;
        current.validate_current()?;
        Ok(baseline_expiry)
    }

    pub fn release(&mut self) -> Result<SnapshotReviewReleaseOutcome, SnapshotReviewError> {
        self.baseline.release()
    }

    pub fn is_released(&self) -> bool {
        self.baseline.is_released()
    }

    pub fn root_node(
        &mut self,
        current: &mut SnapshotReviewSession,
    ) -> Result<SnapshotDiffNode, SnapshotReviewError> {
        self.ensure_documents(current)?;
        let pair = NodePair {
            current_id: Some(0),
            baseline_id: Some(0),
        };
        let node = project_pair(current, &self.baseline, pair, None)?;
        self.validate_pair(current)?;
        Ok(node)
    }

    pub fn child_nodes(
        &mut self,
        current: &mut SnapshotReviewSession,
        parent_id: u64,
        sort: SnapshotDiffNodeSort,
        offset: u64,
        limit: u16,
    ) -> Result<SnapshotDiffNodePage, SnapshotReviewError> {
        if limit == 0 || limit > MAX_SNAPSHOT_REVIEW_NODE_PAGE_LIMIT {
            return Err(SnapshotReviewError::InvalidPage);
        }
        self.ensure_documents(current)?;
        let parent = resolve_pair(current, &self.baseline, parent_id)?;
        ensure_pair_directory(current, &self.baseline, parent)?;
        let mut pairs = union_children(current, &self.baseline, parent)?;
        sort_pairs(current, &self.baseline, &mut pairs, sort)?;
        let totals = change_totals(current, &self.baseline, &pairs)?;
        let total_children =
            u64::try_from(pairs.len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
        let start = usize::try_from(offset).map_err(|_| SnapshotReviewError::InvalidPage)?;
        if start > pairs.len() {
            return Err(SnapshotReviewError::InvalidPage);
        }
        let end = start.saturating_add(usize::from(limit)).min(pairs.len());
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(end.saturating_sub(start))
            .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
        for pair in &pairs[start..end] {
            nodes.push(project_pair(
                current,
                &self.baseline,
                *pair,
                Some(parent_id),
            )?);
        }
        self.validate_pair(current)?;
        Ok(SnapshotDiffNodePage {
            parent_id,
            offset,
            total_children,
            has_more: end < pairs.len(),
            total_growth_bytes: totals.growth_bytes,
            total_shrinkage_bytes: totals.shrinkage_bytes,
            unchanged_child_count: totals.unchanged_count,
            replaced_child_count: totals.replaced_count,
            nodes,
        })
    }

    pub fn treemap(
        &mut self,
        current: &mut SnapshotReviewSession,
        parent_id: u64,
        max_cells: u16,
    ) -> Result<SnapshotDiffTreemap, SnapshotReviewError> {
        if max_cells == 0 || max_cells > MAX_SNAPSHOT_REVIEW_TREEMAP_CELLS {
            return Err(SnapshotReviewError::InvalidTreemapBudget);
        }
        self.ensure_documents(current)?;
        let parent = resolve_pair(current, &self.baseline, parent_id)?;
        ensure_pair_directory(current, &self.baseline, parent)?;
        let mut pairs = union_children(current, &self.baseline, parent)?;
        sort_pairs(
            current,
            &self.baseline,
            &mut pairs,
            SnapshotDiffNodeSort::MagnitudeDescending,
        )?;
        let totals = change_totals(current, &self.baseline, &pairs)?;
        let changed_pairs = pairs
            .iter()
            .take_while(|pair| {
                logical_change_for_pair(current, &self.baseline, **pair).magnitude_bytes > 0
            })
            .copied()
            .collect::<Vec<_>>();
        let cell_count = usize::from(max_cells).min(changed_pairs.len());
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(cell_count)
            .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
        let mut represented_growth_bytes = 0_u64;
        let mut represented_shrinkage_bytes = 0_u64;
        let mut represented_growth_count = 0_u64;
        let mut represented_shrinkage_count = 0_u64;
        for (rank, pair) in changed_pairs.iter().take(cell_count).enumerate() {
            let node = project_pair(current, &self.baseline, *pair, Some(parent_id))?;
            match node.logical_change.direction {
                SnapshotDiffDirection::Growth => {
                    represented_growth_bytes = represented_growth_bytes
                        .checked_add(node.logical_change.magnitude_bytes)
                        .ok_or(SnapshotReviewError::BudgetExceeded)?;
                    represented_growth_count = represented_growth_count
                        .checked_add(1)
                        .ok_or(SnapshotReviewError::BudgetExceeded)?;
                }
                SnapshotDiffDirection::Shrinkage => {
                    represented_shrinkage_bytes = represented_shrinkage_bytes
                        .checked_add(node.logical_change.magnitude_bytes)
                        .ok_or(SnapshotReviewError::BudgetExceeded)?;
                    represented_shrinkage_count = represented_shrinkage_count
                        .checked_add(1)
                        .ok_or(SnapshotReviewError::BudgetExceeded)?;
                }
                SnapshotDiffDirection::Unchanged => {
                    return Err(SnapshotReviewError::InternalState);
                }
            }
            cells.push(SnapshotDiffTreemapCell {
                node,
                magnitude_rank: u64::try_from(rank)
                    .map_err(|_| SnapshotReviewError::BudgetExceeded)?,
            });
        }
        let changed_child_count =
            u64::try_from(changed_pairs.len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
        let growth_count = u64::try_from(
            changed_pairs
                .iter()
                .filter(|pair| {
                    logical_change_for_pair(current, &self.baseline, **pair).direction
                        == SnapshotDiffDirection::Growth
                })
                .count(),
        )
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
        let shrinkage_count = changed_child_count
            .checked_sub(growth_count)
            .ok_or(SnapshotReviewError::InternalState)?;
        self.validate_pair(current)?;
        Ok(SnapshotDiffTreemap {
            parent_id,
            total_children: u64::try_from(pairs.len())
                .map_err(|_| SnapshotReviewError::BudgetExceeded)?,
            changed_child_count,
            total_growth_bytes: totals.growth_bytes,
            total_shrinkage_bytes: totals.shrinkage_bytes,
            other_growth_child_count: growth_count
                .checked_sub(represented_growth_count)
                .ok_or(SnapshotReviewError::InternalState)?,
            other_growth_bytes: totals
                .growth_bytes
                .checked_sub(represented_growth_bytes)
                .ok_or(SnapshotReviewError::InternalState)?,
            other_shrinkage_child_count: shrinkage_count
                .checked_sub(represented_shrinkage_count)
                .ok_or(SnapshotReviewError::InternalState)?,
            other_shrinkage_bytes: totals
                .shrinkage_bytes
                .checked_sub(represented_shrinkage_bytes)
                .ok_or(SnapshotReviewError::InternalState)?,
            unchanged_child_count: totals.unchanged_count,
            replaced_child_count: totals.replaced_count,
            cells,
        })
    }

    fn ensure_documents(
        &mut self,
        current: &mut SnapshotReviewSession,
    ) -> Result<(), SnapshotReviewError> {
        self.validate_parent(current)?;
        let current_root = current.document_for_diff()?.metadata.root.clone();
        let baseline_root = self.baseline.document_for_diff()?.metadata.root.clone();
        if current_root != baseline_root {
            return Err(SnapshotReviewError::CorruptData);
        }
        self.validate_pair(current)
    }

    fn validate_parent(&self, current: &SnapshotReviewSession) -> Result<(), SnapshotReviewError> {
        if !self.parent_liveness.load(AtomicOrdering::Acquire)
            || !current.belongs_to(&self.owner)
            || current.session_identity() != self.parent_session_identity
            || current.scan_id() != &self.metadata.current_scan_id
            || current.is_released()
        {
            return Err(SnapshotReviewError::WrongParentReview);
        }
        Ok(())
    }

    fn validate_pair(&self, current: &SnapshotReviewSession) -> Result<(), SnapshotReviewError> {
        self.validate_parent(current)?;
        current.validate_current()?;
        self.baseline.validate_current()?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct NodePair {
    current_id: Option<u64>,
    baseline_id: Option<u64>,
}

#[derive(Default)]
struct ChangeTotals {
    growth_bytes: u64,
    shrinkage_bytes: u64,
    unchanged_count: u64,
    replaced_count: u64,
}

fn resolve_pair(
    current: &SnapshotReviewSession,
    baseline: &SnapshotReviewSession,
    opaque_id: u64,
) -> Result<NodePair, SnapshotReviewError> {
    if opaque_id == 0 {
        return Ok(NodePair {
            current_id: Some(0),
            baseline_id: Some(0),
        });
    }
    match opaque_id & OPAQUE_ID_SIDE_MASK {
        OPAQUE_ID_CURRENT => {
            let current_id = opaque_id >> OPAQUE_ID_SHIFT;
            let baseline_id = find_counterpart(current, baseline, current_id)?;
            Ok(NodePair {
                current_id: Some(current_id),
                baseline_id,
            })
        }
        OPAQUE_ID_BASELINE => {
            let baseline_id = opaque_id >> OPAQUE_ID_SHIFT;
            let current_id = find_counterpart(baseline, current, baseline_id)?;
            Ok(NodePair {
                current_id,
                baseline_id: Some(baseline_id),
            })
        }
        _ => Err(SnapshotReviewError::NodeNotFound),
    }
}

fn find_counterpart(
    source: &SnapshotReviewSession,
    destination: &SnapshotReviewSession,
    source_id: u64,
) -> Result<Option<u64>, SnapshotReviewError> {
    let source_document = source
        .document_for_diff_readonly()
        .ok_or(SnapshotReviewError::InternalState)?;
    let destination_document = destination
        .document_for_diff_readonly()
        .ok_or(SnapshotReviewError::InternalState)?;
    let mut source_index =
        usize::try_from(source_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    let source_node = source_document
        .nodes
        .get(source_index)
        .filter(|node| node.id == source_id)
        .ok_or(SnapshotReviewError::NodeNotFound)?;
    if source_node.id == 0 {
        return Ok(Some(0));
    }
    let mut components = Vec::new();
    components
        .try_reserve_exact(usize::try_from(source_node.depth).unwrap_or(0))
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    while source_index != 0 {
        let node = source_document
            .nodes
            .get(source_index)
            .ok_or(SnapshotReviewError::CorruptData)?;
        components.push(node.name.as_ref().ok_or(SnapshotReviewError::CorruptData)?);
        source_index = usize::try_from(node.parent.ok_or(SnapshotReviewError::CorruptData)?)
            .map_err(|_| SnapshotReviewError::CorruptData)?;
    }

    let mut destination_index = 0_usize;
    for component in components.iter().rev() {
        let children = destination_document
            .direct_child_indices(destination_index)
            .map_err(|error| map_repository_error(error.kind))?;
        let mut match_index = None;
        for child_index in children {
            let index =
                usize::try_from(*child_index).map_err(|_| SnapshotReviewError::CorruptData)?;
            let child = destination_document
                .nodes
                .get(index)
                .ok_or(SnapshotReviewError::CorruptData)?;
            if child.name.as_ref() == Some(*component) && match_index.replace(index).is_some() {
                return Err(SnapshotReviewError::CorruptData);
            }
        }
        let Some(index) = match_index else {
            return Ok(None);
        };
        destination_index = index;
    }
    Ok(Some(
        destination_document
            .nodes
            .get(destination_index)
            .ok_or(SnapshotReviewError::CorruptData)?
            .id,
    ))
}

fn ensure_pair_directory(
    current: &SnapshotReviewSession,
    baseline: &SnapshotReviewSession,
    pair: NodePair,
) -> Result<(), SnapshotReviewError> {
    let current_directory = pair
        .current_id
        .and_then(|id| stored_node(current, id).ok())
        .is_some_and(|node| node.kind == StoredNodeKind::Directory);
    let baseline_directory = pair
        .baseline_id
        .and_then(|id| stored_node(baseline, id).ok())
        .is_some_and(|node| node.kind == StoredNodeKind::Directory);
    if current_directory || baseline_directory {
        Ok(())
    } else {
        Err(SnapshotReviewError::NodeNotDirectory)
    }
}

fn union_children(
    current: &SnapshotReviewSession,
    baseline: &SnapshotReviewSession,
    parent: NodePair,
) -> Result<Vec<NodePair>, SnapshotReviewError> {
    let current_children = child_ids(current, parent.current_id)?;
    let baseline_children = child_ids(baseline, parent.baseline_id)?;
    let union_bound = current_children
        .len()
        .checked_add(baseline_children.len())
        .ok_or(SnapshotReviewError::BudgetExceeded)?;
    if union_bound > MAX_SNAPSHOT_DIFF_UNION_CHILDREN {
        return Err(SnapshotReviewError::BudgetExceeded);
    }
    let mut current_children = current_children;
    let mut baseline_children = baseline_children;
    current_children.sort_unstable_by(|left, right| compare_node_names(current, *left, *right));
    baseline_children.sort_unstable_by(|left, right| compare_node_names(baseline, *left, *right));
    ensure_unique_names(current, &current_children)?;
    ensure_unique_names(baseline, &baseline_children)?;

    let mut pairs = Vec::new();
    pairs
        .try_reserve_exact(union_bound)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut current_index = 0_usize;
    let mut baseline_index = 0_usize;
    while current_index < current_children.len() || baseline_index < baseline_children.len() {
        match (
            current_children.get(current_index),
            baseline_children.get(baseline_index),
        ) {
            (Some(current_id), Some(baseline_id)) => {
                match compare_cross_names(current, *current_id, baseline, *baseline_id)? {
                    Ordering::Less => {
                        pairs.push(NodePair {
                            current_id: Some(*current_id),
                            baseline_id: None,
                        });
                        current_index += 1;
                    }
                    Ordering::Equal => {
                        pairs.push(NodePair {
                            current_id: Some(*current_id),
                            baseline_id: Some(*baseline_id),
                        });
                        current_index += 1;
                        baseline_index += 1;
                    }
                    Ordering::Greater => {
                        pairs.push(NodePair {
                            current_id: None,
                            baseline_id: Some(*baseline_id),
                        });
                        baseline_index += 1;
                    }
                }
            }
            (Some(current_id), None) => {
                pairs.push(NodePair {
                    current_id: Some(*current_id),
                    baseline_id: None,
                });
                current_index += 1;
            }
            (None, Some(baseline_id)) => {
                pairs.push(NodePair {
                    current_id: None,
                    baseline_id: Some(*baseline_id),
                });
                baseline_index += 1;
            }
            (None, None) => break,
        }
    }
    Ok(pairs)
}

fn child_ids(
    session: &SnapshotReviewSession,
    parent_id: Option<u64>,
) -> Result<Vec<u64>, SnapshotReviewError> {
    let Some(parent_id) = parent_id else {
        return Ok(Vec::new());
    };
    let document = session
        .document_for_diff_readonly()
        .ok_or(SnapshotReviewError::InternalState)?;
    let parent_index = usize::try_from(parent_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    let parent = document
        .nodes
        .get(parent_index)
        .filter(|node| node.id == parent_id)
        .ok_or(SnapshotReviewError::NodeNotFound)?;
    if parent.kind != StoredNodeKind::Directory {
        return Ok(Vec::new());
    }
    let indices = document
        .direct_child_indices(parent_index)
        .map_err(|error| map_repository_error(error.kind))?;
    let mut ids = Vec::new();
    ids.try_reserve_exact(indices.len())
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    for index in indices {
        let index = usize::try_from(*index).map_err(|_| SnapshotReviewError::CorruptData)?;
        ids.push(
            document
                .nodes
                .get(index)
                .ok_or(SnapshotReviewError::CorruptData)?
                .id,
        );
    }
    Ok(ids)
}

fn sort_pairs(
    current: &SnapshotReviewSession,
    baseline: &SnapshotReviewSession,
    pairs: &mut [NodePair],
    sort: SnapshotDiffNodeSort,
) -> Result<(), SnapshotReviewError> {
    pairs.sort_unstable_by(|left, right| match sort {
        SnapshotDiffNodeSort::NameAscending => compare_pair_names(current, baseline, *left, *right),
        SnapshotDiffNodeSort::MagnitudeDescending => {
            logical_change_for_pair(current, baseline, *right)
                .magnitude_bytes
                .cmp(&logical_change_for_pair(current, baseline, *left).magnitude_bytes)
                .then_with(|| compare_pair_names(current, baseline, *left, *right))
        }
        SnapshotDiffNodeSort::CurrentBytesDescending => pair_current_bytes(current, *right)
            .cmp(&pair_current_bytes(current, *left))
            .then_with(|| compare_pair_names(current, baseline, *left, *right)),
    });
    Ok(())
}

fn change_totals(
    current: &SnapshotReviewSession,
    baseline: &SnapshotReviewSession,
    pairs: &[NodePair],
) -> Result<ChangeTotals, SnapshotReviewError> {
    let mut totals = ChangeTotals::default();
    for pair in pairs {
        let change = logical_change_for_pair(current, baseline, *pair);
        match change.direction {
            SnapshotDiffDirection::Growth => {
                totals.growth_bytes = totals
                    .growth_bytes
                    .checked_add(change.magnitude_bytes)
                    .ok_or(SnapshotReviewError::BudgetExceeded)?;
            }
            SnapshotDiffDirection::Shrinkage => {
                totals.shrinkage_bytes = totals
                    .shrinkage_bytes
                    .checked_add(change.magnitude_bytes)
                    .ok_or(SnapshotReviewError::BudgetExceeded)?;
            }
            SnapshotDiffDirection::Unchanged => {
                totals.unchanged_count = totals
                    .unchanged_count
                    .checked_add(1)
                    .ok_or(SnapshotReviewError::BudgetExceeded)?;
            }
        }
        if pair_kind_changed(current, baseline, *pair) {
            totals.replaced_count = totals
                .replaced_count
                .checked_add(1)
                .ok_or(SnapshotReviewError::BudgetExceeded)?;
        }
    }
    Ok(totals)
}

fn project_pair(
    current: &SnapshotReviewSession,
    baseline: &SnapshotReviewSession,
    pair: NodePair,
    parent_id: Option<u64>,
) -> Result<SnapshotDiffNode, SnapshotReviewError> {
    let current_node = pair
        .current_id
        .map(|id| current.project_node_for_diff(id))
        .transpose()?;
    let baseline_node = pair
        .baseline_id
        .map(|id| baseline.project_node_for_diff(id))
        .transpose()?;
    let display_node = current_node
        .as_ref()
        .or(baseline_node.as_ref())
        .ok_or(SnapshotReviewError::InternalState)?;
    let id = opaque_id(pair)?;
    let logical_change = diff_values(
        current_node.as_ref().map(|node| node.logical_bytes),
        baseline_node.as_ref().map(|node| node.logical_bytes),
    );
    let kind_changed = current_node
        .as_ref()
        .zip(baseline_node.as_ref())
        .is_some_and(|(current, baseline)| current.kind != baseline.kind);
    let change = if kind_changed {
        SnapshotDiffChange::Replaced
    } else {
        match (
            current_node.as_ref(),
            baseline_node.as_ref(),
            logical_change.direction,
        ) {
            (Some(_), None, _) => SnapshotDiffChange::Added,
            (None, Some(_), _) => SnapshotDiffChange::Removed,
            (Some(_), Some(_), SnapshotDiffDirection::Growth) => SnapshotDiffChange::Grew,
            (Some(_), Some(_), SnapshotDiffDirection::Shrinkage) => SnapshotDiffChange::Shrank,
            (Some(_), Some(_), SnapshotDiffDirection::Unchanged) => SnapshotDiffChange::Unchanged,
            _ => return Err(SnapshotReviewError::InternalState),
        }
    };
    let current_allocated_bytes = current_node.as_ref().and_then(|node| node.allocated_bytes);
    let baseline_allocated_bytes = baseline_node.as_ref().and_then(|node| node.allocated_bytes);
    let allocated_change = match (&current_node, &baseline_node) {
        (Some(_), Some(_)) => current_allocated_bytes
            .zip(baseline_allocated_bytes)
            .map(|(current, baseline)| diff_exact_values(current, baseline)),
        (Some(_), None) => current_allocated_bytes.map(|value| diff_exact_values(value, 0)),
        (None, Some(_)) => baseline_allocated_bytes.map(|value| diff_exact_values(0, value)),
        (None, None) => None,
    };
    let can_descend = current_node
        .as_ref()
        .is_some_and(|node| node.kind == SnapshotReviewNodeKind::Directory)
        || baseline_node
            .as_ref()
            .is_some_and(|node| node.kind == SnapshotReviewNodeKind::Directory);
    Ok(SnapshotDiffNode {
        id,
        parent_id,
        depth: display_node.depth,
        name: display_node.name.clone(),
        kind: display_node.kind,
        category: current_node
            .as_ref()
            .map_or(SnapshotReviewCategory::Unclassified, |node| node.category),
        change,
        logical_change,
        current_logical_bytes: current_node.as_ref().map(|node| node.logical_bytes),
        baseline_logical_bytes: baseline_node.as_ref().map(|node| node.logical_bytes),
        current_allocated_bytes,
        baseline_allocated_bytes,
        allocated_change,
        current_file_count: current_node.as_ref().map(|node| node.file_count),
        baseline_file_count: baseline_node.as_ref().map(|node| node.file_count),
        current_child_count: current_node.as_ref().map(|node| node.child_count),
        baseline_child_count: baseline_node.as_ref().map(|node| node.child_count),
        current_scan_flags: current_node.as_ref().map(|node| node.scan_flags),
        baseline_scan_flags: baseline_node.as_ref().map(|node| node.scan_flags),
        can_descend,
    })
}

fn opaque_id(pair: NodePair) -> Result<u64, SnapshotReviewError> {
    if pair.current_id == Some(0) && pair.baseline_id == Some(0) {
        return Ok(0);
    }
    if let Some(id) = pair.current_id {
        return id
            .checked_shl(OPAQUE_ID_SHIFT)
            .and_then(|value| value.checked_add(OPAQUE_ID_CURRENT))
            .ok_or(SnapshotReviewError::BudgetExceeded);
    }
    let id = pair.baseline_id.ok_or(SnapshotReviewError::InternalState)?;
    id.checked_shl(OPAQUE_ID_SHIFT)
        .and_then(|value| value.checked_add(OPAQUE_ID_BASELINE))
        .ok_or(SnapshotReviewError::BudgetExceeded)
}

fn diff_values(current: Option<u64>, baseline: Option<u64>) -> SnapshotDiffValue {
    diff_exact_values(current.unwrap_or(0), baseline.unwrap_or(0))
}

fn diff_exact_values(current: u64, baseline: u64) -> SnapshotDiffValue {
    match current.cmp(&baseline) {
        Ordering::Greater => SnapshotDiffValue {
            direction: SnapshotDiffDirection::Growth,
            magnitude_bytes: current - baseline,
        },
        Ordering::Less => SnapshotDiffValue {
            direction: SnapshotDiffDirection::Shrinkage,
            magnitude_bytes: baseline - current,
        },
        Ordering::Equal => SnapshotDiffValue {
            direction: SnapshotDiffDirection::Unchanged,
            magnitude_bytes: 0,
        },
    }
}

fn logical_change_for_pair(
    current: &SnapshotReviewSession,
    baseline: &SnapshotReviewSession,
    pair: NodePair,
) -> SnapshotDiffValue {
    diff_values(
        pair.current_id
            .and_then(|id| stored_node(current, id).ok())
            .map(|node| node.logical_bytes),
        pair.baseline_id
            .and_then(|id| stored_node(baseline, id).ok())
            .map(|node| node.logical_bytes),
    )
}

fn pair_kind_changed(
    current: &SnapshotReviewSession,
    baseline: &SnapshotReviewSession,
    pair: NodePair,
) -> bool {
    pair.current_id
        .and_then(|id| stored_node(current, id).ok())
        .zip(
            pair.baseline_id
                .and_then(|id| stored_node(baseline, id).ok()),
        )
        .is_some_and(|(current, baseline)| current.kind != baseline.kind)
}

fn pair_current_bytes(current: &SnapshotReviewSession, pair: NodePair) -> u64 {
    pair.current_id
        .and_then(|id| stored_node(current, id).ok())
        .map_or(0, |node| node.logical_bytes)
}

fn stored_node(
    session: &SnapshotReviewSession,
    id: u64,
) -> Result<&StoredNode, SnapshotReviewError> {
    let document = session
        .document_for_diff_readonly()
        .ok_or(SnapshotReviewError::InternalState)?;
    let index = usize::try_from(id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    document
        .nodes
        .get(index)
        .filter(|node| node.id == id)
        .ok_or(SnapshotReviewError::NodeNotFound)
}

fn compare_node_names(session: &SnapshotReviewSession, left: u64, right: u64) -> Ordering {
    let left_name = stored_node(session, left)
        .ok()
        .and_then(|node| node.name.as_ref());
    let right_name = stored_node(session, right)
        .ok()
        .and_then(|node| node.name.as_ref());
    compare_optional_names(left_name, right_name).then_with(|| left.cmp(&right))
}

fn compare_cross_names(
    left_session: &SnapshotReviewSession,
    left_id: u64,
    right_session: &SnapshotReviewSession,
    right_id: u64,
) -> Result<Ordering, SnapshotReviewError> {
    let left = stored_node(left_session, left_id)?
        .name
        .as_ref()
        .ok_or(SnapshotReviewError::CorruptData)?;
    let right = stored_node(right_session, right_id)?
        .name
        .as_ref()
        .ok_or(SnapshotReviewError::CorruptData)?;
    Ok(compare_host_values(left, right))
}

fn compare_pair_names(
    current: &SnapshotReviewSession,
    baseline: &SnapshotReviewSession,
    left: NodePair,
    right: NodePair,
) -> Ordering {
    let left_name = pair_name(current, baseline, left);
    let right_name = pair_name(current, baseline, right);
    compare_optional_names(left_name, right_name).then_with(|| {
        left.current_id
            .cmp(&right.current_id)
            .then_with(|| left.baseline_id.cmp(&right.baseline_id))
    })
}

fn pair_name<'a>(
    current: &'a SnapshotReviewSession,
    baseline: &'a SnapshotReviewSession,
    pair: NodePair,
) -> Option<&'a HostValue> {
    pair.current_id
        .and_then(|id| stored_node(current, id).ok())
        .and_then(|node| node.name.as_ref())
        .or_else(|| {
            pair.baseline_id
                .and_then(|id| stored_node(baseline, id).ok())
                .and_then(|node| node.name.as_ref())
        })
}

fn ensure_unique_names(
    session: &SnapshotReviewSession,
    children: &[u64],
) -> Result<(), SnapshotReviewError> {
    for adjacent in children.windows(2) {
        if compare_cross_names(session, adjacent[0], session, adjacent[1])? == Ordering::Equal {
            return Err(SnapshotReviewError::CorruptData);
        }
    }
    Ok(())
}

fn compare_optional_names(left: Option<&HostValue>, right: Option<&HostValue>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => compare_host_values(left, right),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_host_values(left: &HostValue, right: &HostValue) -> Ordering {
    (left.encoding() as u8)
        .cmp(&(right.encoding() as u8))
        .then_with(|| left.bytes().cmp(right.bytes()))
}

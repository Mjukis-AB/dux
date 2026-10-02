use super::*;

pub(super) fn sorted_child_indices(
    document: &StoredReviewDocument,
    parent_id: u64,
    sort: SnapshotReviewNodeSort,
    offset: u64,
) -> Result<Vec<u32>, SnapshotReviewError> {
    let parent_index = usize::try_from(parent_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    let parent = document
        .nodes
        .get(parent_index)
        .filter(|node| node.id == parent_id)
        .ok_or(SnapshotReviewError::NodeNotFound)?;
    if parent.kind != SnapshotNodeKind::Directory {
        return Err(SnapshotReviewError::NodeNotDirectory);
    }
    if offset > parent.child_count {
        return Err(SnapshotReviewError::InvalidPage);
    }
    if parent.child_count > MAX_SNAPSHOT_REVIEW_SORTABLE_CHILDREN {
        return Err(SnapshotReviewError::BudgetExceeded);
    }
    let child_capacity =
        usize::try_from(parent.child_count).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let direct = document
        .direct_child_indices(parent_index)
        .map_err(|error| map_repository_error(error.kind))?;
    if direct.len() != child_capacity {
        return Err(SnapshotReviewError::CorruptData);
    }
    let mut child_indices = Vec::<u32>::new();
    child_indices
        .try_reserve_exact(child_capacity)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    child_indices.extend_from_slice(direct);
    child_indices.sort_unstable_by(|left, right| {
        compare_nodes(
            &document.nodes[*left as usize],
            &document.nodes[*right as usize],
            sort,
        )
    });
    Ok(child_indices)
}

pub(super) fn build_child_page(
    document: &SnapshotDocument,
    child_indices: &[u32],
    parent_id: u64,
    offset: u64,
    limit: u16,
    category_index: &SnapshotReviewCategoryIndex,
) -> Result<SnapshotReviewNodePage, SnapshotReviewError> {
    let parent_index = usize::try_from(parent_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    let parent = document
        .nodes
        .get(parent_index)
        .filter(|node| node.id == parent_id)
        .ok_or(SnapshotReviewError::NodeNotFound)?;
    let start = usize::try_from(offset).map_err(|_| SnapshotReviewError::InvalidPage)?;
    if start > child_indices.len() {
        return Err(SnapshotReviewError::InvalidPage);
    }
    let end = start
        .saturating_add(usize::from(limit))
        .min(child_indices.len());
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(end - start)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    for index in &child_indices[start..end] {
        let node = &document.nodes[*index as usize];
        let name = node.name.as_ref().ok_or(SnapshotReviewError::CorruptData)?;
        let category = category_for_node(document, node, category_index)?;
        nodes.push(project_node(node, name, category));
    }
    Ok(SnapshotReviewNodePage {
        parent_id,
        offset,
        total_children: parent.child_count,
        has_more: end < child_indices.len(),
        nodes,
    })
}

pub(super) fn build_treemap(
    document: &SnapshotDocument,
    child_indices: &[u32],
    parent_id: u64,
    max_cells: u16,
    category_index: &SnapshotReviewCategoryIndex,
) -> Result<SnapshotReviewTreemap, SnapshotReviewError> {
    let parent_index = usize::try_from(parent_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    let parent = document
        .nodes
        .get(parent_index)
        .filter(|node| node.id == parent_id)
        .ok_or(SnapshotReviewError::NodeNotFound)?;
    if parent.kind != SnapshotNodeKind::Directory {
        return Err(SnapshotReviewError::NodeNotDirectory);
    }
    let total_children =
        u64::try_from(child_indices.len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    if total_children != parent.child_count {
        return Err(SnapshotReviewError::CorruptData);
    }

    let mut total_child_logical_bytes = 0_u64;
    let mut zero_logical_child_count = 0_u64;
    for index in child_indices {
        let node = document
            .nodes
            .get(*index as usize)
            .ok_or(SnapshotReviewError::CorruptData)?;
        total_child_logical_bytes = total_child_logical_bytes
            .checked_add(node.logical_bytes)
            .ok_or(SnapshotReviewError::CorruptData)?;
        if node.logical_bytes == 0 {
            zero_logical_child_count = zero_logical_child_count
                .checked_add(1)
                .ok_or(SnapshotReviewError::CorruptData)?;
        }
    }

    let cell_capacity = usize::from(max_cells).min(child_indices.len());
    let mut cells = Vec::new();
    cells
        .try_reserve_exact(cell_capacity)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut represented_logical_bytes = 0_u64;
    for (rank, index) in child_indices.iter().enumerate() {
        if cells.len() == cell_capacity {
            break;
        }
        let node = document
            .nodes
            .get(*index as usize)
            .ok_or(SnapshotReviewError::CorruptData)?;
        if node.logical_bytes == 0 {
            break;
        }
        let name = node.name.as_ref().ok_or(SnapshotReviewError::CorruptData)?;
        represented_logical_bytes = represented_logical_bytes
            .checked_add(node.logical_bytes)
            .ok_or(SnapshotReviewError::CorruptData)?;
        let category = category_for_node(document, node, category_index)?;
        cells.push(SnapshotReviewTreemapCell {
            node: project_node(node, name, category),
            logical_rank: u64::try_from(rank).map_err(|_| SnapshotReviewError::BudgetExceeded)?,
        });
    }

    let represented_children =
        u64::try_from(cells.len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    Ok(SnapshotReviewTreemap {
        parent_id,
        total_children,
        total_child_logical_bytes,
        other_child_count: total_children
            .checked_sub(represented_children)
            .ok_or(SnapshotReviewError::CorruptData)?,
        other_logical_bytes: total_child_logical_bytes
            .checked_sub(represented_logical_bytes)
            .ok_or(SnapshotReviewError::CorruptData)?,
        zero_logical_child_count,
        cells,
    })
}

pub(super) fn build_disk_map(
    document: &SnapshotDocument,
    child_indices: &[u32],
    parent_id: u64,
    max_cells: u16,
    category_index: &SnapshotReviewCategoryIndex,
) -> Result<SnapshotReviewDiskMap, SnapshotReviewError> {
    let parent_index = usize::try_from(parent_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    let parent = document
        .nodes
        .get(parent_index)
        .filter(|node| node.id == parent_id)
        .ok_or(SnapshotReviewError::NodeNotFound)?;
    if parent.kind != SnapshotNodeKind::Directory {
        return Err(SnapshotReviewError::NodeNotDirectory);
    }
    let total_children =
        u64::try_from(child_indices.len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    if total_children != parent.child_count {
        return Err(SnapshotReviewError::CorruptData);
    }

    let mut total_child_allocated_bytes = 0_u64;
    let mut unknown_allocated_child_count = 0_u64;
    let mut zero_allocated_child_count = 0_u64;
    for index in child_indices {
        let node = document
            .nodes
            .get(*index as usize)
            .ok_or(SnapshotReviewError::CorruptData)?;
        match node.allocated_bytes {
            Some(bytes) => {
                total_child_allocated_bytes = total_child_allocated_bytes
                    .checked_add(bytes)
                    .ok_or(SnapshotReviewError::CorruptData)?;
                if bytes == 0 {
                    zero_allocated_child_count = zero_allocated_child_count
                        .checked_add(1)
                        .ok_or(SnapshotReviewError::CorruptData)?;
                }
            }
            None => {
                unknown_allocated_child_count = unknown_allocated_child_count
                    .checked_add(1)
                    .ok_or(SnapshotReviewError::CorruptData)?;
            }
        }
    }

    let cell_capacity = usize::from(max_cells).min(child_indices.len());
    let mut cells = Vec::new();
    cells
        .try_reserve_exact(cell_capacity)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut represented_allocated_bytes = 0_u64;
    for (rank, index) in child_indices.iter().enumerate() {
        if cells.len() == cell_capacity {
            break;
        }
        let node = document
            .nodes
            .get(*index as usize)
            .ok_or(SnapshotReviewError::CorruptData)?;
        let Some(allocated_bytes) = node.allocated_bytes else {
            break;
        };
        if allocated_bytes == 0 {
            break;
        }
        let name = node.name.as_ref().ok_or(SnapshotReviewError::CorruptData)?;
        represented_allocated_bytes = represented_allocated_bytes
            .checked_add(allocated_bytes)
            .ok_or(SnapshotReviewError::CorruptData)?;
        let category = category_for_node(document, node, category_index)?;
        cells.push(SnapshotReviewDiskMapCell {
            node: project_node(node, name, category),
            allocated_rank: u64::try_from(rank).map_err(|_| SnapshotReviewError::BudgetExceeded)?,
        });
    }

    let represented_children =
        u64::try_from(cells.len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    Ok(SnapshotReviewDiskMap {
        parent_id,
        total_children,
        total_child_allocated_bytes,
        other_child_count: total_children
            .checked_sub(represented_children)
            .ok_or(SnapshotReviewError::CorruptData)?,
        other_allocated_bytes: total_child_allocated_bytes
            .checked_sub(represented_allocated_bytes)
            .ok_or(SnapshotReviewError::CorruptData)?,
        unknown_allocated_child_count,
        zero_allocated_child_count,
        cells,
    })
}

#[derive(Clone, Copy)]
struct LargeFileCandidate<'document> {
    node: &'document SnapshotNode,
}

impl PartialEq for LargeFileCandidate<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.node.id == other.node.id
    }
}

impl Eq for LargeFileCandidate<'_> {}

impl PartialOrd for LargeFileCandidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LargeFileCandidate<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_large_file_nodes(self.node, other.node)
    }
}

struct ICloudObservationCandidate<'document> {
    node: &'document SnapshotNode,
}

impl PartialEq for ICloudObservationCandidate<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.node.id == other.node.id
    }
}

impl Eq for ICloudObservationCandidate<'_> {}

impl PartialOrd for ICloudObservationCandidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ICloudObservationCandidate<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_icloud_observation_nodes(self.node, other.node)
    }
}

pub(super) fn build_large_file_page(
    document: &SnapshotDocument,
    minimum_logical_bytes: u64,
    modified_before: Option<SnapshotReviewTimestamp>,
    max_results: u16,
    category_index: &SnapshotReviewCategoryIndex,
) -> Result<SnapshotReviewLargeFilePage, SnapshotReviewError> {
    let capacity = usize::from(max_results);
    let mut selected = BinaryHeap::new();
    selected
        .try_reserve(capacity)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut total_matching_files = 0_u64;
    let mut total_matching_logical_bytes = 0_u64;

    for node in &document.nodes {
        if node.kind != SnapshotNodeKind::File
            || node.logical_bytes < minimum_logical_bytes
            || !modified_before_matches(node.modified_at, modified_before)
        {
            continue;
        }
        if node.name.is_none() {
            return Err(SnapshotReviewError::CorruptData);
        }
        total_matching_files = total_matching_files
            .checked_add(1)
            .ok_or(SnapshotReviewError::BudgetExceeded)?;
        total_matching_logical_bytes = total_matching_logical_bytes
            .checked_add(node.logical_bytes)
            .ok_or(SnapshotReviewError::CorruptData)?;
        let candidate = LargeFileCandidate { node };
        if selected.len() < capacity {
            selected.push(candidate);
        } else if selected
            .peek()
            .is_some_and(|worst| candidate.cmp(worst) == Ordering::Less)
        {
            selected.pop();
            selected.push(candidate);
        }
    }

    let mut selected = selected.into_vec();
    selected.sort_unstable();
    let mut files = Vec::new();
    files
        .try_reserve_exact(selected.len())
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    for candidate in selected {
        let name = candidate
            .node
            .name
            .as_ref()
            .ok_or(SnapshotReviewError::CorruptData)?;
        let (parent_context, context_truncated) = build_parent_context(document, candidate.node)?;
        let category = category_for_node(document, candidate.node, category_index)?;
        files.push(SnapshotReviewLargeFile {
            node: project_node(candidate.node, name, category),
            parent_context,
            context_truncated,
        });
    }

    Ok(SnapshotReviewLargeFilePage {
        total_matching_files,
        total_matching_logical_bytes,
        has_more: total_matching_files
            > u64::try_from(files.len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?,
        files,
    })
}

pub(super) fn build_icloud_observation_source(
    document: &SnapshotDocument,
    scope_node_id: u64,
    max_results: u16,
    category_index: &SnapshotReviewCategoryIndex,
) -> Result<SnapshotReviewICloudObservationSource, SnapshotReviewError> {
    let scope_index =
        usize::try_from(scope_node_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    let scope = document
        .nodes
        .get(scope_index)
        .filter(|node| node.id == scope_node_id)
        .ok_or(SnapshotReviewError::NodeNotFound)?;
    if scope.kind != SnapshotNodeKind::Directory {
        return Err(SnapshotReviewError::NodeNotDirectory);
    }

    let capacity = usize::from(max_results);
    let mut selected = BinaryHeap::new();
    selected
        .try_reserve(capacity)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut visited_node_count = 0_u64;
    let mut total_ranked_files = 0_u64;

    for node in document.nodes.iter().skip(scope_index.saturating_add(1)) {
        if node.depth <= scope.depth {
            break;
        }
        visited_node_count = visited_node_count
            .checked_add(1)
            .ok_or(SnapshotReviewError::BudgetExceeded)?;
        if visited_node_count > MAX_SNAPSHOT_REVIEW_ICLOUD_OBSERVATION_VISITED_NODES {
            return Err(SnapshotReviewError::BudgetExceeded);
        }
        if node.kind != SnapshotNodeKind::File
            || node.allocated_bytes.is_none_or(|bytes| bytes == 0)
            || node.scan_flags != SnapshotScanFlags::NONE
        {
            continue;
        }
        if node.name.is_none() {
            return Err(SnapshotReviewError::CorruptData);
        }
        total_ranked_files = total_ranked_files
            .checked_add(1)
            .ok_or(SnapshotReviewError::BudgetExceeded)?;
        let candidate = ICloudObservationCandidate { node };
        if selected.len() < capacity {
            selected.push(candidate);
        } else if selected
            .peek()
            .is_some_and(|worst| candidate.cmp(worst) == Ordering::Less)
        {
            selected.pop();
            selected.push(candidate);
        }
    }

    let mut selected = selected.into_vec();
    selected.sort_unstable();
    let mut targets = Vec::new();
    targets
        .try_reserve_exact(selected.len())
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    for (rank, candidate) in selected.into_iter().enumerate() {
        let name = candidate
            .node
            .name
            .as_ref()
            .ok_or(SnapshotReviewError::CorruptData)?;
        let (parent_context, context_truncated) = build_parent_context(document, candidate.node)?;
        let category = category_for_node(document, candidate.node, category_index)?;
        targets.push(SnapshotReviewICloudObservationTarget {
            rank: u16::try_from(rank).map_err(|_| SnapshotReviewError::BudgetExceeded)?,
            node: project_node(candidate.node, name, category),
            parent_context,
            context_truncated,
        });
    }

    Ok(SnapshotReviewICloudObservationSource {
        scope_node_id,
        requested_max_results: max_results,
        visited_node_count,
        total_ranked_files,
        has_more: total_ranked_files
            > u64::try_from(targets.len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?,
        targets,
    })
}

pub(super) fn modified_before_matches(
    observed: Option<SnapshotTimestamp>,
    cutoff: Option<SnapshotReviewTimestamp>,
) -> bool {
    match cutoff {
        None => true,
        Some(cutoff) => observed.is_some_and(|observed| {
            timestamp_key(observed) < (cutoff.seconds_since_unix_epoch, cutoff.nanoseconds)
        }),
    }
}

fn build_parent_context(
    document: &SnapshotDocument,
    file: &SnapshotNode,
) -> Result<(Vec<SnapshotReviewName>, bool), SnapshotReviewError> {
    let mut reverse_context = Vec::new();
    reverse_context
        .try_reserve_exact(MAX_SNAPSHOT_REVIEW_PARENT_CONTEXT_COMPONENTS)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut next_parent = file.parent;

    while let Some(parent_id) = next_parent {
        let parent_index =
            usize::try_from(parent_id).map_err(|_| SnapshotReviewError::CorruptData)?;
        let parent = document
            .nodes
            .get(parent_index)
            .filter(|node| node.id == parent_id && node.kind == SnapshotNodeKind::Directory)
            .ok_or(SnapshotReviewError::CorruptData)?;
        if parent.id == 0 {
            reverse_context.reverse();
            return Ok((reverse_context, false));
        }
        if reverse_context.len() == MAX_SNAPSHOT_REVIEW_PARENT_CONTEXT_COMPONENTS {
            reverse_context.reverse();
            return Ok((reverse_context, true));
        }
        let name = parent
            .name
            .as_ref()
            .ok_or(SnapshotReviewError::CorruptData)?;
        reverse_context.push(project_name(name));
        next_parent = parent.parent;
    }

    Err(SnapshotReviewError::CorruptData)
}

fn compare_large_file_nodes(left: &SnapshotNode, right: &SnapshotNode) -> Ordering {
    right
        .logical_bytes
        .cmp(&left.logical_bytes)
        .then_with(|| compare_names(left, right))
        .then_with(|| left.id.cmp(&right.id))
}

fn compare_icloud_observation_nodes(left: &SnapshotNode, right: &SnapshotNode) -> Ordering {
    right
        .allocated_bytes
        .cmp(&left.allocated_bytes)
        .then_with(|| right.logical_bytes.cmp(&left.logical_bytes))
        .then_with(|| left.id.cmp(&right.id))
}

pub(super) fn compare_nodes(
    left: &SnapshotNode,
    right: &SnapshotNode,
    sort: SnapshotReviewNodeSort,
) -> Ordering {
    let ordering = match sort {
        SnapshotReviewNodeSort::NameAscending => compare_names(left, right),
        SnapshotReviewNodeSort::LogicalBytesDescending => right
            .logical_bytes
            .cmp(&left.logical_bytes)
            .then_with(|| compare_names(left, right)),
        SnapshotReviewNodeSort::AllocatedBytesDescending => right
            .allocated_bytes
            .cmp(&left.allocated_bytes)
            .then_with(|| right.logical_bytes.cmp(&left.logical_bytes))
            .then_with(|| compare_names(left, right)),
        SnapshotReviewNodeSort::ModifiedNewest => right
            .modified_at
            .map(timestamp_key)
            .cmp(&left.modified_at.map(timestamp_key))
            .then_with(|| compare_names(left, right)),
    };
    ordering.then_with(|| left.id.cmp(&right.id))
}

fn compare_names(left: &SnapshotNode, right: &SnapshotNode) -> Ordering {
    match (left.name.as_ref(), right.name.as_ref()) {
        (Some(left), Some(right)) => encoding_key(left.encoding())
            .cmp(&encoding_key(right.encoding()))
            .then_with(|| left.bytes().cmp(right.bytes())),
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
    }
}

const fn encoding_key(encoding: HostEncoding) -> u8 {
    match encoding {
        HostEncoding::UnixBytes => 0,
        HostEncoding::WindowsUtf16Le => 1,
    }
}

const fn timestamp_key(timestamp: SnapshotTimestamp) -> (u64, u32) {
    (
        timestamp.seconds_since_unix_epoch(),
        timestamp.nanoseconds(),
    )
}

pub(super) fn category_for_node(
    document: &SnapshotDocument,
    node: &SnapshotNode,
    category_index: &SnapshotReviewCategoryIndex,
) -> Result<SnapshotReviewCategory, SnapshotReviewError> {
    if category_index.roots.is_empty() {
        return Ok(SnapshotReviewCategory::Unclassified);
    }
    let scan_root = normalize_category_path(
        &document
            .metadata
            .root
            .to_path_buf()
            .map_err(|_| SnapshotReviewError::CorruptData)?,
    )
    .ok_or(SnapshotReviewError::CorruptData)?;
    let mut node_path = normalize_category_path(&historical_node_path(document, node)?)
        .ok_or(SnapshotReviewError::CorruptData)?;
    if !node_path.starts_with(&scan_root) {
        return Err(SnapshotReviewError::CorruptData);
    }
    loop {
        if let Some(entry) = category_index.roots.get(&node_path) {
            return Ok(if entry.conflicting {
                SnapshotReviewCategory::Unclassified
            } else {
                entry.category
            });
        }
        if node_path == scan_root {
            break;
        }
        if !node_path.pop() || !node_path.starts_with(&scan_root) {
            return Err(SnapshotReviewError::CorruptData);
        }
    }
    Ok(SnapshotReviewCategory::Unclassified)
}

pub(super) fn normalize_category_path(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::Normal(_) => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => return None,
        }
    }
    Some(normalized)
}

fn historical_node_path(
    document: &SnapshotDocument,
    target: &SnapshotNode,
) -> Result<PathBuf, SnapshotReviewError> {
    let mut path = document
        .metadata
        .root
        .to_path_buf()
        .map_err(|_| SnapshotReviewError::CorruptData)?;
    if target.id == 0 {
        return Ok(path);
    }
    let capacity =
        usize::try_from(target.depth).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut components = Vec::new();
    components
        .try_reserve_exact(capacity)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut current = target;
    while current.id != 0 {
        components.push(
            current
                .name
                .as_ref()
                .ok_or(SnapshotReviewError::CorruptData)?,
        );
        let parent_id = current.parent.ok_or(SnapshotReviewError::CorruptData)?;
        let parent_index =
            usize::try_from(parent_id).map_err(|_| SnapshotReviewError::CorruptData)?;
        current = document
            .nodes
            .get(parent_index)
            .filter(|node| node.id == parent_id)
            .ok_or(SnapshotReviewError::CorruptData)?;
        if components.len() > capacity {
            return Err(SnapshotReviewError::CorruptData);
        }
    }
    if components.len() != capacity {
        return Err(SnapshotReviewError::CorruptData);
    }
    for component in components.into_iter().rev() {
        path.push(
            component
                .to_path_buf()
                .map_err(|_| SnapshotReviewError::CorruptData)?,
        );
    }
    Ok(path)
}

pub(super) fn project_node(
    node: &SnapshotNode,
    name: &HostValue,
    category: SnapshotReviewCategory,
) -> SnapshotReviewNode {
    SnapshotReviewNode {
        id: node.id,
        parent_id: node.parent,
        depth: node.depth,
        kind: match node.kind {
            SnapshotNodeKind::Directory => SnapshotReviewNodeKind::Directory,
            SnapshotNodeKind::File => SnapshotReviewNodeKind::File,
            SnapshotNodeKind::Symlink => SnapshotReviewNodeKind::Symlink,
            SnapshotNodeKind::Other => SnapshotReviewNodeKind::Other,
            SnapshotNodeKind::Error => SnapshotReviewNodeKind::Error,
        },
        category,
        name: project_name(name),
        logical_bytes: node.logical_bytes,
        allocated_bytes: node.allocated_bytes,
        file_count: node.file_count,
        child_count: node.child_count,
        modified_at: node.modified_at.map(project_timestamp),
        accessed_at: node.accessed_at.map(project_timestamp),
        scan_flags: project_flags(node.scan_flags),
    }
}

fn project_name(name: &HostValue) -> SnapshotReviewName {
    SnapshotReviewName {
        encoding: match name.encoding() {
            HostEncoding::UnixBytes => SnapshotReviewNameEncoding::UnixBytes,
            HostEncoding::WindowsUtf16Le => SnapshotReviewNameEncoding::WindowsUtf16LittleEndian,
        },
        encoded_bytes: Arc::from(name.bytes()),
        display: Arc::from(name.display_lossy()),
    }
}

const fn project_timestamp(timestamp: SnapshotTimestamp) -> SnapshotReviewTimestamp {
    SnapshotReviewTimestamp {
        seconds_since_unix_epoch: timestamp.seconds_since_unix_epoch(),
        nanoseconds: timestamp.nanoseconds(),
    }
}

const fn project_flags(flags: SnapshotScanFlags) -> SnapshotReviewScanFlags {
    SnapshotReviewScanFlags {
        inaccessible: flags.contains(SnapshotScanFlags::INACCESSIBLE),
        timed_out: flags.contains(SnapshotScanFlags::TIMED_OUT),
        hard_link_duplicate: flags.contains(SnapshotScanFlags::HARD_LINK_DUPLICATE),
        mount_boundary: flags.contains(SnapshotScanFlags::MOUNT_BOUNDARY),
    }
}

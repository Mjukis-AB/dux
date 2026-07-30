use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::node::{NodeId, NodeKind, TreeNode};

#[cfg(unix)]
fn native_path_bytes(path: &Path) -> usize {
    use std::os::unix::ffi::OsStrExt as _;

    path.as_os_str().as_bytes().len()
}

#[cfg(windows)]
fn native_path_bytes(path: &Path) -> usize {
    use std::os::windows::ffi::OsStrExt as _;

    path.as_os_str().encode_wide().count().saturating_mul(2)
}

#[cfg(not(any(unix, windows)))]
fn native_path_bytes(path: &Path) -> usize {
    path.as_os_str().to_string_lossy().len()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManagedCacheTreeValidationError {
    InvalidGraph,
    InvalidName,
    InvalidAggregate,
    LimitExceeded,
    AllocationFailed,
}

#[derive(Debug)]
pub(crate) struct ManagedCacheNodeRecord {
    pub(crate) id: u64,
    pub(crate) parent: Option<u64>,
    pub(crate) sibling_ordinal: u32,
    pub(crate) name: String,
    pub(crate) kind: NodeKind,
    pub(crate) size: u64,
    pub(crate) file_count: u64,
    pub(crate) depth: u16,
    pub(crate) mtime: Option<SystemTime>,
    pub(crate) path_is_symlink: bool,
}

/// Arena-allocated directory tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskTree {
    nodes: Vec<Option<TreeNode>>,
    root_path: PathBuf,
}

impl DiskTree {
    pub fn new(root_path: PathBuf) -> Self {
        let root_name = root_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| root_path.to_string_lossy().to_string());

        let root_node = TreeNode::new(
            NodeId::ROOT,
            root_name,
            NodeKind::Directory,
            root_path.clone(),
            None,
            0,
        );

        Self {
            nodes: vec![Some(root_node)],
            root_path,
        }
    }

    /// Add a new node and return its ID
    pub fn add_node(
        &mut self,
        name: String,
        kind: NodeKind,
        path: PathBuf,
        parent: NodeId,
    ) -> NodeId {
        let parent_depth = self.get(parent).map(|n| n.depth).unwrap_or(0);
        let id = NodeId(self.nodes.len());

        let node = TreeNode::new(id, name, kind, path, Some(parent), parent_depth + 1);

        self.nodes.push(Some(node));
        if let Some(parent_node) = self.get_mut(parent) {
            parent_node.children.push(id);
        }

        id
    }

    /// Get a reference to a node
    pub fn get(&self, id: NodeId) -> Option<&TreeNode> {
        self.nodes.get(id.index()).and_then(|opt| opt.as_ref())
    }

    /// Get a mutable reference to a node
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut TreeNode> {
        self.nodes.get_mut(id.index()).and_then(|opt| opt.as_mut())
    }

    /// Get the root node
    pub fn root(&self) -> &TreeNode {
        self.nodes[0].as_ref().expect("Root node must exist")
    }

    /// Get the root path
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// Reconstruct paths and UI state for all nodes after deserialization
    /// Must be called after loading from cache since paths and is_expanded are not serialized
    pub fn rebuild_paths(&mut self) {
        // Set root path and expand root
        if let Some(root) = self.nodes.get_mut(0).and_then(|o| o.as_mut()) {
            root.path = self.root_path.clone();
            root.is_expanded = true;
        }

        // Process nodes in order (parents before children due to arena structure)
        for i in 1..self.nodes.len() {
            if let Some(node) = &self.nodes[i] {
                let parent_path = node
                    .parent
                    .and_then(|pid| self.nodes.get(pid.index()))
                    .and_then(|o| o.as_ref())
                    .map(|p| p.path.clone());

                if let Some(pp) = parent_path {
                    let name = self.nodes[i].as_ref().map(|n| n.name.clone());
                    if let (Some(node), Some(name)) = (self.nodes[i].as_mut(), name) {
                        node.path = pp.join(&name);
                    }
                }
            }
        }
    }

    /// Get total number of nodes (including tombstones)
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Get count of live (non-tombstone) nodes
    pub fn live_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.is_some()).count()
    }

    /// Check if tree is empty (only has root)
    pub fn is_empty(&self) -> bool {
        self.live_count() <= 1
    }

    /// Set size for a node
    pub fn set_size(&mut self, id: NodeId, size: u64) {
        if let Some(node) = self.get_mut(id) {
            node.size = size;
        }
    }

    /// Propagate sizes from children to parents (bottom-up)
    pub fn aggregate_sizes(&mut self) {
        // Process nodes in reverse order (children before parents)
        for i in (0..self.nodes.len()).rev() {
            let node = match &self.nodes[i] {
                Some(n) => n,
                None => continue, // Skip tombstones
            };
            if node.kind.is_directory() {
                let children = node.children.clone();
                let mut total_size = 0u64;
                let mut total_files = 0u64;

                for child_id in &children {
                    if let Some(child) = self.get(*child_id) {
                        total_size += child.size;
                        total_files += child.file_count;
                    }
                }

                if let Some(node) = self.get_mut(NodeId(i)) {
                    node.size = total_size;
                    node.file_count = total_files;
                }
            }
        }
    }

    /// Sort all children by size descending
    pub fn sort_by_size(&mut self) {
        // Clone only one directory's child IDs at a time. Cloning every full
        // path here can multiply memory use during million-node finalization.
        for index in 0..self.nodes.len() {
            let Some(mut children) = self.nodes[index].as_ref().map(|node| node.children.clone())
            else {
                continue;
            };
            children.sort_by(|left, right| {
                let left = self.get(*left);
                let right = self.get(*right);
                right
                    .map(|node| node.size)
                    .cmp(&left.map(|node| node.size))
                    .then_with(|| {
                        left.map(|node| node.path.as_path())
                            .cmp(&right.map(|node| node.path.as_path()))
                    })
            });
            if let Some(node) = self.nodes[index].as_mut() {
                node.children = children;
            }
        }
    }

    /// Toggle expanded state for a node
    pub fn toggle_expanded(&mut self, id: NodeId) {
        if let Some(node) = self.get_mut(id)
            && node.kind.is_directory()
        {
            node.is_expanded = !node.is_expanded;
        }
    }

    /// Set expanded state for a node
    pub fn set_expanded(&mut self, id: NodeId, expanded: bool) {
        if let Some(node) = self.get_mut(id)
            && node.kind.is_directory()
        {
            node.is_expanded = expanded;
        }
    }

    /// Get visible nodes in tree order (respecting expansion state)
    pub fn visible_nodes(&self, root: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        self.collect_visible(root, &mut result);
        result
    }

    fn collect_visible(&self, id: NodeId, result: &mut Vec<NodeId>) {
        result.push(id);

        if let Some(node) = self.get(id)
            && node.is_expanded
        {
            for &child_id in &node.children {
                self.collect_visible(child_id, result);
            }
        }
    }

    /// Get the path from root to a node
    pub fn path_to_node(&self, id: NodeId) -> Vec<NodeId> {
        let mut path = Vec::new();
        let mut current = Some(id);

        while let Some(node_id) = current {
            path.push(node_id);
            current = self.get(node_id).and_then(|n| n.parent);
        }

        path.reverse();
        path
    }

    /// Get breadcrumb string for a node
    pub fn breadcrumbs(&self, id: NodeId) -> String {
        let path = self.path_to_node(id);
        path.iter()
            .filter_map(|&id| self.get(id).map(|n| n.name.as_str()))
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Expand all ancestors of a node
    pub fn expand_to(&mut self, id: NodeId) {
        let path = self.path_to_node(id);
        for node_id in path {
            self.set_expanded(node_id, true);
        }
    }

    /// Get total size of the tree
    pub fn total_size(&self) -> u64 {
        self.root().size
    }

    /// Get total file count
    pub fn total_files(&self) -> u64 {
        self.root().file_count
    }

    /// Iterator over all live nodes (skips tombstones)
    pub fn iter(&self) -> impl Iterator<Item = &TreeNode> {
        self.nodes.iter().filter_map(|opt| opt.as_ref())
    }

    /// Validate every serialized tree fact before a managed cache hit can be
    /// published. Cache bytes are untrusted observations: a checksum is not a
    /// graph, name, depth, or aggregate proof.
    pub(crate) fn validate_managed_cache_tree(
        &self,
        expected_root: &Path,
        maximum_nodes: usize,
        maximum_depth: u16,
        maximum_component_bytes: usize,
        maximum_path_bytes: usize,
    ) -> Result<(), ManagedCacheTreeValidationError> {
        use std::collections::HashSet;
        use std::path::Component;

        if maximum_nodes == 0
            || maximum_component_bytes == 0
            || maximum_path_bytes == 0
            || self.nodes.is_empty()
            || self.nodes.len() > maximum_nodes
            || self.live_count() != self.nodes.len()
            || native_path_bytes(expected_root) > maximum_path_bytes
        {
            return Err(ManagedCacheTreeValidationError::LimitExceeded);
        }
        if self.root_path != expected_root {
            return Err(ManagedCacheTreeValidationError::InvalidGraph);
        }
        let Some(root) = self.nodes.first().and_then(Option::as_ref) else {
            return Err(ManagedCacheTreeValidationError::InvalidGraph);
        };
        if root.id != NodeId::ROOT
            || root.parent.is_some()
            || root.depth != 0
            || root.kind != NodeKind::Directory
            || root.name
                != expected_root
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| expected_root.to_string_lossy().into_owned())
        {
            return Err(ManagedCacheTreeValidationError::InvalidGraph);
        }
        if root.name.len() > maximum_component_bytes {
            return Err(ManagedCacheTreeValidationError::LimitExceeded);
        }

        let mut referenced = Vec::new();
        referenced
            .try_reserve_exact(self.nodes.len())
            .map_err(|_| ManagedCacheTreeValidationError::AllocationFailed)?;
        referenced.resize(self.nodes.len(), false);
        referenced[0] = true;
        for (index, slot) in self.nodes.iter().enumerate() {
            let Some(node) = slot.as_ref() else {
                return Err(ManagedCacheTreeValidationError::InvalidGraph);
            };
            if node.id != NodeId(index) {
                return Err(ManagedCacheTreeValidationError::InvalidGraph);
            }
            if index != 0 {
                let mut components = Path::new(&node.name).components();
                if node.name.is_empty()
                    || node.name.chars().any(char::is_control)
                    || !matches!(components.next(), Some(Component::Normal(_)))
                    || components.next().is_some()
                {
                    return Err(ManagedCacheTreeValidationError::InvalidName);
                }
                if node.name.len() > maximum_component_bytes {
                    return Err(ManagedCacheTreeValidationError::LimitExceeded);
                }
            }
            if node.depth > maximum_depth {
                return Err(ManagedCacheTreeValidationError::LimitExceeded);
            }
            if !node.kind.is_directory() && !node.children.is_empty() {
                return Err(ManagedCacheTreeValidationError::InvalidGraph);
            }

            let mut child_ids = HashSet::new();
            let mut child_names = HashSet::new();
            child_ids
                .try_reserve(node.children.len())
                .map_err(|_| ManagedCacheTreeValidationError::AllocationFailed)?;
            child_names
                .try_reserve(node.children.len())
                .map_err(|_| ManagedCacheTreeValidationError::AllocationFailed)?;
            for child_id in &node.children {
                let child_index = child_id.index();
                let child = self.nodes.get(child_index).and_then(Option::as_ref);
                if child_index <= index
                    || child_index >= self.nodes.len()
                    || !child_ids.insert(*child_id)
                    || !child_names.insert(
                        child
                            .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?
                            .name
                            .as_str(),
                    )
                    || referenced[child_index]
                {
                    return Err(ManagedCacheTreeValidationError::InvalidGraph);
                }
                let child = child.ok_or(ManagedCacheTreeValidationError::InvalidGraph)?;
                let expected_depth = node
                    .depth
                    .checked_add(1)
                    .ok_or(ManagedCacheTreeValidationError::LimitExceeded)?;
                if child.parent != Some(node.id) || child.depth != expected_depth {
                    return Err(ManagedCacheTreeValidationError::InvalidGraph);
                }
                referenced[child_index] = true;
            }
        }
        if referenced.iter().any(|value| !value) {
            return Err(ManagedCacheTreeValidationError::InvalidGraph);
        }

        for (index, slot) in self.nodes.iter().enumerate() {
            let node = slot
                .as_ref()
                .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?;
            if index == 0 {
                if node.path != expected_root {
                    return Err(ManagedCacheTreeValidationError::InvalidGraph);
                }
            } else {
                let parent = self
                    .get(
                        node.parent
                            .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?,
                    )
                    .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?;
                if node.path.parent() != Some(parent.path.as_path())
                    || node.path.file_name() != Some(std::ffi::OsStr::new(&node.name))
                {
                    return Err(ManagedCacheTreeValidationError::InvalidGraph);
                }
            }
            if native_path_bytes(&node.path) > maximum_path_bytes {
                return Err(ManagedCacheTreeValidationError::LimitExceeded);
            }
        }

        for index in (0..self.nodes.len()).rev() {
            let node = self.nodes[index]
                .as_ref()
                .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?;
            if node.kind.is_directory() {
                let mut size = 0_u64;
                let mut file_count = 0_u64;
                for child_id in &node.children {
                    let child = self
                        .get(*child_id)
                        .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?;
                    size = size
                        .checked_add(child.size)
                        .ok_or(ManagedCacheTreeValidationError::InvalidAggregate)?;
                    file_count = file_count
                        .checked_add(child.file_count)
                        .ok_or(ManagedCacheTreeValidationError::InvalidAggregate)?;
                }
                if node.size != size || node.file_count != file_count {
                    return Err(ManagedCacheTreeValidationError::InvalidAggregate);
                }
            } else {
                let expected_files = u64::from(node.kind == NodeKind::File);
                if node.file_count != expected_files {
                    return Err(ManagedCacheTreeValidationError::InvalidAggregate);
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn managed_cache_records(
        &self,
    ) -> Result<Vec<ManagedCacheNodeRecord>, ManagedCacheTreeValidationError> {
        let ordinals = self.managed_cache_ordinals()?;
        let mut records = Vec::new();
        records
            .try_reserve_exact(self.nodes.len())
            .map_err(|_| ManagedCacheTreeValidationError::AllocationFailed)?;
        for (index, slot) in self.nodes.iter().enumerate() {
            let node = slot
                .as_ref()
                .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?;
            records.push(ManagedCacheNodeRecord {
                id: u64::try_from(index)
                    .map_err(|_| ManagedCacheTreeValidationError::LimitExceeded)?,
                parent: node
                    .parent
                    .map(|parent| u64::try_from(parent.index()))
                    .transpose()
                    .map_err(|_| ManagedCacheTreeValidationError::LimitExceeded)?,
                sibling_ordinal: ordinals[index],
                name: node.name.clone(),
                kind: node.kind,
                size: node.size,
                file_count: node.file_count,
                depth: node.depth,
                mtime: node.mtime,
                path_is_symlink: node.path_is_symlink,
            });
        }
        Ok(records)
    }

    pub(crate) fn managed_cache_ordinals(
        &self,
    ) -> Result<Vec<u32>, ManagedCacheTreeValidationError> {
        let mut ordinals = Vec::new();
        ordinals
            .try_reserve_exact(self.nodes.len())
            .map_err(|_| ManagedCacheTreeValidationError::AllocationFailed)?;
        ordinals.resize(self.nodes.len(), u32::MAX);
        ordinals[0] = 0;
        for slot in &self.nodes {
            let node = slot
                .as_ref()
                .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?;
            for (ordinal, child) in node.children.iter().enumerate() {
                let ordinal = u32::try_from(ordinal)
                    .map_err(|_| ManagedCacheTreeValidationError::LimitExceeded)?;
                let value = ordinals
                    .get_mut(child.index())
                    .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?;
                if *value != u32::MAX {
                    return Err(ManagedCacheTreeValidationError::InvalidGraph);
                }
                *value = ordinal;
            }
        }
        if ordinals.contains(&u32::MAX) {
            return Err(ManagedCacheTreeValidationError::InvalidGraph);
        }
        Ok(ordinals)
    }

    pub(crate) fn from_managed_cache_records(
        root_path: PathBuf,
        records: Vec<ManagedCacheNodeRecord>,
    ) -> Result<Self, ManagedCacheTreeValidationError> {
        let Some(root_record) = records.first() else {
            return Err(ManagedCacheTreeValidationError::InvalidGraph);
        };
        if root_record.id != 0 || root_record.parent.is_some() || root_record.sibling_ordinal != 0 {
            return Err(ManagedCacheTreeValidationError::InvalidGraph);
        }

        let mut child_counts = Vec::new();
        child_counts
            .try_reserve_exact(records.len())
            .map_err(|_| ManagedCacheTreeValidationError::AllocationFailed)?;
        child_counts.resize(records.len(), 0_usize);
        let mut ordinals = Vec::new();
        ordinals
            .try_reserve_exact(records.len())
            .map_err(|_| ManagedCacheTreeValidationError::AllocationFailed)?;
        for (index, record) in records.iter().enumerate() {
            if record.id
                != u64::try_from(index)
                    .map_err(|_| ManagedCacheTreeValidationError::LimitExceeded)?
            {
                return Err(ManagedCacheTreeValidationError::InvalidGraph);
            }
            ordinals.push(record.sibling_ordinal);
            if index != 0 {
                let parent_index = usize::try_from(
                    record
                        .parent
                        .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?,
                )
                .map_err(|_| ManagedCacheTreeValidationError::LimitExceeded)?;
                if parent_index >= index {
                    return Err(ManagedCacheTreeValidationError::InvalidGraph);
                }
                child_counts[parent_index] = child_counts[parent_index]
                    .checked_add(1)
                    .ok_or(ManagedCacheTreeValidationError::LimitExceeded)?;
            }
        }

        let mut root_node_path = Some(try_clone_path(&root_path)?);
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(records.len())
            .map_err(|_| ManagedCacheTreeValidationError::AllocationFailed)?;
        for (index, record) in records.into_iter().enumerate() {
            let parent = record
                .parent
                .map(usize::try_from)
                .transpose()
                .map_err(|_| ManagedCacheTreeValidationError::LimitExceeded)?
                .map(NodeId);
            let path = if index == 0 {
                root_node_path
                    .take()
                    .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?
            } else {
                let parent_path = nodes
                    .get(
                        parent
                            .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?
                            .index(),
                    )
                    .and_then(Option::as_ref)
                    .map(|node: &TreeNode| node.path.as_path())
                    .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?;
                try_join_path(parent_path, &record.name)?
            };
            let mut children = Vec::new();
            children
                .try_reserve_exact(child_counts[index])
                .map_err(|_| ManagedCacheTreeValidationError::AllocationFailed)?;
            nodes.push(Some(TreeNode {
                id: NodeId(index),
                name: record.name,
                kind: record.kind,
                size: record.size,
                file_count: record.file_count,
                parent,
                children,
                depth: record.depth,
                mtime: record.mtime,
                is_expanded: index == 0,
                path_is_symlink: record.path_is_symlink,
                path,
            }));
        }

        for index in 1..nodes.len() {
            let parent = nodes[index]
                .as_ref()
                .and_then(|node| node.parent)
                .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?;
            let children = &mut nodes
                .get_mut(parent.index())
                .and_then(Option::as_mut)
                .ok_or(ManagedCacheTreeValidationError::InvalidGraph)?
                .children;
            if children.len() >= children.capacity() {
                return Err(ManagedCacheTreeValidationError::InvalidGraph);
            }
            children.push(NodeId(index));
        }

        for slot in &mut nodes {
            let Some(node) = slot.as_mut() else {
                return Err(ManagedCacheTreeValidationError::InvalidGraph);
            };
            node.children
                .sort_unstable_by_key(|child| ordinals.get(child.index()).copied());
            for (expected, child) in node.children.iter().enumerate() {
                let expected = u32::try_from(expected)
                    .map_err(|_| ManagedCacheTreeValidationError::LimitExceeded)?;
                if ordinals.get(child.index()).copied() != Some(expected) {
                    return Err(ManagedCacheTreeValidationError::InvalidGraph);
                }
            }
        }
        Ok(Self { nodes, root_path })
    }

    #[cfg(test)]
    fn root_mut(&mut self) -> &mut TreeNode {
        self.nodes[0].as_mut().expect("root exists")
    }

    /// Find a node by its path
    pub fn find_by_path(&self, path: &Path) -> Option<NodeId> {
        for (i, node_opt) in self.nodes.iter().enumerate() {
            if let Some(node) = node_opt
                && node.path == path
            {
                return Some(NodeId(i));
            }
        }
        None
    }

    /// Collect all descendant node IDs
    fn collect_descendants(&self, id: NodeId, result: &mut Vec<NodeId>) {
        if let Some(node) = self.get(id) {
            for &child_id in &node.children {
                result.push(child_id);
                self.collect_descendants(child_id, result);
            }
        }
    }

    /// Remove node and descendants, return bytes freed
    /// Does NOT perform filesystem operations - only updates tree structure
    pub fn remove_node(&mut self, id: NodeId) -> u64 {
        // Never remove root
        if id == NodeId::ROOT {
            return 0;
        }

        // Get node info before removal
        let (size, file_count, parent_id) = match self.get(id) {
            Some(node) => (node.size, node.file_count, node.parent),
            None => return 0, // Already removed
        };

        // Remove from parent's children
        if let Some(pid) = parent_id
            && let Some(parent) = self.get_mut(pid)
        {
            parent.children.retain(|&c| c != id);
        }

        // Collect all descendants to tombstone
        let mut to_remove = vec![id];
        self.collect_descendants(id, &mut to_remove);

        // Tombstone node and all descendants
        for nid in to_remove {
            if let Some(slot) = self.nodes.get_mut(nid.index()) {
                *slot = None;
            }
        }

        // Propagate size decrease up to root
        let mut current = parent_id;
        while let Some(nid) = current {
            if let Some(node) = self.get_mut(nid) {
                node.size = node.size.saturating_sub(size);
                node.file_count = node.file_count.saturating_sub(file_count);
                current = node.parent;
            } else {
                break;
            }
        }

        size
    }
}

fn try_clone_path(path: &Path) -> Result<PathBuf, ManagedCacheTreeValidationError> {
    let mut value = std::ffi::OsString::new();
    value
        .try_reserve_exact(path.as_os_str().len())
        .map_err(|_| ManagedCacheTreeValidationError::AllocationFailed)?;
    value.push(path.as_os_str());
    Ok(PathBuf::from(value))
}

fn try_join_path(parent: &Path, name: &str) -> Result<PathBuf, ManagedCacheTreeValidationError> {
    let mut path = try_clone_path(parent)?;
    path.as_mut_os_string()
        .try_reserve(name.len().saturating_add(4))
        .map_err(|_| ManagedCacheTreeValidationError::AllocationFailed)?;
    path.push(name);
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANAGED_MAX_NODES: usize = 32;
    const MANAGED_MAX_DEPTH: u16 = 8;
    const MANAGED_MAX_COMPONENT_BYTES: usize = 64;
    const MANAGED_MAX_PATH_BYTES: usize = 256;

    fn valid_managed_tree() -> DiskTree {
        let root_path = PathBuf::from("/managed");
        let mut tree = DiskTree::new(root_path.clone());
        let directory = tree.add_node(
            "directory".to_owned(),
            NodeKind::Directory,
            root_path.join("directory"),
            NodeId::ROOT,
        );
        let file = tree.add_node(
            "file".to_owned(),
            NodeKind::File,
            root_path.join("directory/file"),
            directory,
        );
        tree.set_size(file, 11);
        tree.aggregate_sizes();
        tree
    }

    fn validate_managed(tree: &DiskTree) -> Result<(), ManagedCacheTreeValidationError> {
        tree.validate_managed_cache_tree(
            Path::new("/managed"),
            MANAGED_MAX_NODES,
            MANAGED_MAX_DEPTH,
            MANAGED_MAX_COMPONENT_BYTES,
            MANAGED_MAX_PATH_BYTES,
        )
    }

    #[test]
    fn managed_validation_accepts_a_complete_consistent_tree() {
        assert_eq!(validate_managed(&valid_managed_tree()), Ok(()));
    }

    #[test]
    fn managed_validation_requires_exact_root_identity_and_name() {
        let mut tree = valid_managed_tree();
        assert_eq!(
            tree.validate_managed_cache_tree(
                Path::new("/other"),
                MANAGED_MAX_NODES,
                MANAGED_MAX_DEPTH,
                MANAGED_MAX_COMPONENT_BYTES,
                MANAGED_MAX_PATH_BYTES,
            ),
            Err(ManagedCacheTreeValidationError::InvalidGraph)
        );

        tree.root_mut().name = "other".to_owned();
        assert_eq!(
            validate_managed(&tree),
            Err(ManagedCacheTreeValidationError::InvalidGraph)
        );
    }

    #[test]
    fn managed_validation_rejects_duplicate_sibling_names_and_bad_topology() {
        let mut duplicate = valid_managed_tree();
        let root_path = duplicate.root_path().to_path_buf();
        duplicate.add_node(
            "directory".to_owned(),
            NodeKind::Directory,
            root_path.join("directory"),
            NodeId::ROOT,
        );
        duplicate.aggregate_sizes();
        assert_eq!(
            validate_managed(&duplicate),
            Err(ManagedCacheTreeValidationError::InvalidGraph)
        );

        let mut bad_parent = valid_managed_tree();
        bad_parent.get_mut(NodeId(2)).expect("file").parent = Some(NodeId::ROOT);
        assert_eq!(
            validate_managed(&bad_parent),
            Err(ManagedCacheTreeValidationError::InvalidGraph)
        );
    }

    #[test]
    fn managed_validation_rejects_aggregate_and_name_drift() {
        let mut aggregate = valid_managed_tree();
        aggregate.get_mut(NodeId(1)).expect("directory").size += 1;
        assert_eq!(
            validate_managed(&aggregate),
            Err(ManagedCacheTreeValidationError::InvalidAggregate)
        );

        let mut name = valid_managed_tree();
        name.get_mut(NodeId(2)).expect("file").name = "../escape".to_owned();
        assert_eq!(
            validate_managed(&name),
            Err(ManagedCacheTreeValidationError::InvalidName)
        );
    }

    #[test]
    fn managed_validation_enforces_depth_component_and_full_path_limits() {
        let tree = valid_managed_tree();
        assert_eq!(
            tree.validate_managed_cache_tree(
                Path::new("/managed"),
                MANAGED_MAX_NODES,
                1,
                MANAGED_MAX_COMPONENT_BYTES,
                MANAGED_MAX_PATH_BYTES,
            ),
            Err(ManagedCacheTreeValidationError::LimitExceeded)
        );
        assert_eq!(
            tree.validate_managed_cache_tree(
                Path::new("/managed"),
                MANAGED_MAX_NODES,
                MANAGED_MAX_DEPTH,
                3,
                MANAGED_MAX_PATH_BYTES,
            ),
            Err(ManagedCacheTreeValidationError::LimitExceeded)
        );
        assert_eq!(
            tree.validate_managed_cache_tree(
                Path::new("/managed"),
                MANAGED_MAX_NODES,
                MANAGED_MAX_DEPTH,
                MANAGED_MAX_COMPONENT_BYTES,
                20,
            ),
            Err(ManagedCacheTreeValidationError::LimitExceeded)
        );
    }

    #[test]
    fn managed_records_round_trip_sibling_order_and_node_facts() {
        let tree = valid_managed_tree();
        let records = tree.managed_cache_records().expect("records");
        let file_name_allocation = records[2].name.as_ptr();
        let rebuilt = DiskTree::from_managed_cache_records(PathBuf::from("/managed"), records)
            .expect("rebuilt tree");

        assert_eq!(validate_managed(&rebuilt), Ok(()));
        assert_eq!(rebuilt.root().children, tree.root().children);
        assert_eq!(
            rebuilt.get(NodeId(2)).expect("rebuilt file").name.as_ptr(),
            file_name_allocation,
            "decoded names should move into the arena rather than being cloned"
        );
        assert_eq!(
            rebuilt.get(NodeId(2)).expect("rebuilt file").size,
            tree.get(NodeId(2)).expect("file").size
        );
    }

    #[test]
    fn test_tree_creation() {
        let tree = DiskTree::new(PathBuf::from("/test"));
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.root().name, "test");
    }

    #[test]
    fn test_add_nodes() {
        let mut tree = DiskTree::new(PathBuf::from("/test"));

        let file_id = tree.add_node(
            "file.txt".to_string(),
            NodeKind::File,
            PathBuf::from("/test/file.txt"),
            NodeId::ROOT,
        );

        assert_eq!(tree.len(), 2);
        assert_eq!(tree.get(file_id).unwrap().name, "file.txt");
        assert_eq!(tree.root().children.len(), 1);
    }

    #[test]
    fn test_remove_node_with_size_propagation() {
        let mut tree = DiskTree::new(PathBuf::from("/test"));

        // Add a subdirectory
        let subdir_id = tree.add_node(
            "subdir".to_string(),
            NodeKind::Directory,
            PathBuf::from("/test/subdir"),
            NodeId::ROOT,
        );

        // Add files under subdir
        let file1_id = tree.add_node(
            "file1.txt".to_string(),
            NodeKind::File,
            PathBuf::from("/test/subdir/file1.txt"),
            subdir_id,
        );
        tree.set_size(file1_id, 1000);

        let file2_id = tree.add_node(
            "file2.txt".to_string(),
            NodeKind::File,
            PathBuf::from("/test/subdir/file2.txt"),
            subdir_id,
        );
        tree.set_size(file2_id, 2000);

        // Aggregate sizes
        tree.aggregate_sizes();

        // Verify initial state
        assert_eq!(tree.root().size, 3000);
        assert_eq!(tree.get(subdir_id).unwrap().size, 3000);

        // Remove file1
        let freed = tree.remove_node(file1_id);
        assert_eq!(freed, 1000);

        // Verify sizes propagated correctly
        assert_eq!(tree.root().size, 2000);
        assert_eq!(tree.get(subdir_id).unwrap().size, 2000);

        // file1 should be tombstoned
        assert!(tree.get(file1_id).is_none());

        // file2 should still exist
        assert!(tree.get(file2_id).is_some());
    }

    #[test]
    fn test_remove_node_removes_descendants() {
        let mut tree = DiskTree::new(PathBuf::from("/test"));

        // Add a subdirectory
        let subdir_id = tree.add_node(
            "subdir".to_string(),
            NodeKind::Directory,
            PathBuf::from("/test/subdir"),
            NodeId::ROOT,
        );

        // Add files under subdir
        let file1_id = tree.add_node(
            "file1.txt".to_string(),
            NodeKind::File,
            PathBuf::from("/test/subdir/file1.txt"),
            subdir_id,
        );
        tree.set_size(file1_id, 1000);

        tree.aggregate_sizes();

        // Remove subdir (should also remove file1)
        let freed = tree.remove_node(subdir_id);
        assert_eq!(freed, 1000);

        // Both should be tombstoned
        assert!(tree.get(subdir_id).is_none());
        assert!(tree.get(file1_id).is_none());

        // Root should have no children
        assert_eq!(tree.root().children.len(), 0);
        assert_eq!(tree.root().size, 0);
    }

    #[test]
    fn test_find_by_path() {
        let mut tree = DiskTree::new(PathBuf::from("/test"));

        let file_id = tree.add_node(
            "file.txt".to_string(),
            NodeKind::File,
            PathBuf::from("/test/file.txt"),
            NodeId::ROOT,
        );

        // Should find by path
        assert_eq!(
            tree.find_by_path(&PathBuf::from("/test/file.txt")),
            Some(file_id)
        );

        // Should return None for non-existent path
        assert_eq!(
            tree.find_by_path(&PathBuf::from("/test/nonexistent.txt")),
            None
        );
    }
}

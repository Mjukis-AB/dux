use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::node::{NodeId, NodeKind, TreeNode};

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

    /// Reconstruct paths for all nodes after deserialization
    /// Must be called after loading from cache since paths are not serialized
    pub fn rebuild_paths(&mut self) {
        // Set root path first
        if let Some(root) = self.nodes.get_mut(0).and_then(|o| o.as_mut()) {
            root.path = self.root_path.clone();
        }

        // Process nodes in order (parents before children due to arena structure)
        for i in 1..self.nodes.len() {
            if let Some(node) = &self.nodes[i] {
                let parent_path = node.parent
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
        // First collect all the size information
        let sizes: Vec<u64> = self.nodes.iter().map(|n| n.as_ref().map(|n| n.size).unwrap_or(0)).collect();

        // Then sort each node's children
        for node_opt in &mut self.nodes {
            if let Some(node) = node_opt {
                node.children.sort_by(|a, b| {
                    let size_a = sizes.get(a.index()).copied().unwrap_or(0);
                    let size_b = sizes.get(b.index()).copied().unwrap_or(0);
                    size_b.cmp(&size_a)
                });
            }
        }
    }

    /// Toggle expanded state for a node
    pub fn toggle_expanded(&mut self, id: NodeId) {
        if let Some(node) = self.get_mut(id) {
            if node.kind.is_directory() {
                node.is_expanded = !node.is_expanded;
            }
        }
    }

    /// Set expanded state for a node
    pub fn set_expanded(&mut self, id: NodeId, expanded: bool) {
        if let Some(node) = self.get_mut(id) {
            if node.kind.is_directory() {
                node.is_expanded = expanded;
            }
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

        if let Some(node) = self.get(id) {
            if node.is_expanded {
                for &child_id in &node.children {
                    self.collect_visible(child_id, result);
                }
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

    /// Find a node by its path
    pub fn find_by_path(&self, path: &Path) -> Option<NodeId> {
        for (i, node_opt) in self.nodes.iter().enumerate() {
            if let Some(node) = node_opt {
                if node.path == path {
                    return Some(NodeId(i));
                }
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
        if let Some(pid) = parent_id {
            if let Some(parent) = self.get_mut(pid) {
                parent.children.retain(|&c| c != id);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

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

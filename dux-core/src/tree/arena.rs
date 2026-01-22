use std::path::{Path, PathBuf};

use super::node::{NodeId, NodeKind, TreeNode};

/// Arena-allocated directory tree
#[derive(Debug)]
pub struct DiskTree {
    nodes: Vec<TreeNode>,
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
            nodes: vec![root_node],
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
        let parent_depth = self.nodes[parent.index()].depth;
        let id = NodeId(self.nodes.len());

        let node = TreeNode::new(id, name, kind, path, Some(parent), parent_depth + 1);

        self.nodes.push(node);
        self.nodes[parent.index()].children.push(id);

        id
    }

    /// Get a reference to a node
    pub fn get(&self, id: NodeId) -> Option<&TreeNode> {
        self.nodes.get(id.index())
    }

    /// Get a mutable reference to a node
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut TreeNode> {
        self.nodes.get_mut(id.index())
    }

    /// Get the root node
    pub fn root(&self) -> &TreeNode {
        &self.nodes[0]
    }

    /// Get the root path
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// Get total number of nodes
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if tree is empty (only has root)
    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    /// Set size for a node
    pub fn set_size(&mut self, id: NodeId, size: u64) {
        if let Some(node) = self.nodes.get_mut(id.index()) {
            node.size = size;
        }
    }

    /// Propagate sizes from children to parents (bottom-up)
    pub fn aggregate_sizes(&mut self) {
        // Process nodes in reverse order (children before parents)
        for i in (0..self.nodes.len()).rev() {
            let node = &self.nodes[i];
            if node.kind.is_directory() {
                let children = node.children.clone();
                let mut total_size = 0u64;
                let mut total_files = 0u64;

                for child_id in &children {
                    if let Some(child) = self.nodes.get(child_id.index()) {
                        total_size += child.size;
                        total_files += child.file_count;
                    }
                }

                self.nodes[i].size = total_size;
                self.nodes[i].file_count = total_files;
            }
        }
    }

    /// Sort all children by size descending
    pub fn sort_by_size(&mut self) {
        // First collect all the size information
        let sizes: Vec<u64> = self.nodes.iter().map(|n| n.size).collect();

        // Then sort each node's children
        for node in &mut self.nodes {
            node.children.sort_by(|a, b| {
                let size_a = sizes.get(a.index()).copied().unwrap_or(0);
                let size_b = sizes.get(b.index()).copied().unwrap_or(0);
                size_b.cmp(&size_a)
            });
        }
    }

    /// Toggle expanded state for a node
    pub fn toggle_expanded(&mut self, id: NodeId) {
        if let Some(node) = self.nodes.get_mut(id.index()) {
            if node.kind.is_directory() {
                node.is_expanded = !node.is_expanded;
            }
        }
    }

    /// Set expanded state for a node
    pub fn set_expanded(&mut self, id: NodeId, expanded: bool) {
        if let Some(node) = self.nodes.get_mut(id.index()) {
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

    /// Iterator over all nodes
    pub fn iter(&self) -> impl Iterator<Item = &TreeNode> {
        self.nodes.iter()
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
}

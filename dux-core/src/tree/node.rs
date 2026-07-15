use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Unique identifier for a node in the tree
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub usize);

impl NodeId {
    pub const ROOT: NodeId = NodeId(0);

    pub fn index(&self) -> usize {
        self.0
    }
}

/// Type of filesystem entry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Directory,
    File,
    Symlink,
    Other,
    Error,
}

impl NodeKind {
    pub fn icon(&self) -> &'static str {
        match self {
            NodeKind::Directory => "📁",
            NodeKind::File => "📄",
            NodeKind::Symlink => "🔗",
            NodeKind::Other => "❓",
            NodeKind::Error => "⚠️",
        }
    }

    pub fn is_directory(&self) -> bool {
        matches!(self, NodeKind::Directory)
    }
}

/// A node in the disk tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    /// Actual disk usage in bytes
    pub size: u64,
    /// Number of files (including self if file)
    pub file_count: u64,
    /// Parent node (None for root)
    pub parent: Option<NodeId>,
    /// Children sorted by size descending
    pub children: Vec<NodeId>,
    /// Depth in tree (0 for root)
    pub depth: u16,
    /// Modification time (directories only, for cache invalidation)
    pub mtime: Option<SystemTime>,
    /// Whether directory is expanded in UI
    #[serde(skip)]
    pub is_expanded: bool,
    /// Whether this path component is a symlink, even when scans follow links
    pub path_is_symlink: bool,
    /// Full path to this node (reconstructed on cache load)
    #[serde(skip)]
    pub path: PathBuf,
}

impl TreeNode {
    pub fn new(
        id: NodeId,
        name: String,
        kind: NodeKind,
        path: PathBuf,
        parent: Option<NodeId>,
        depth: u16,
    ) -> Self {
        Self {
            id,
            name,
            kind,
            size: 0,
            file_count: if kind == NodeKind::File { 1 } else { 0 },
            parent,
            children: Vec::new(),
            depth,
            mtime: None,
            is_expanded: depth == 0, // Root starts expanded
            path_is_symlink: false,
            path,
        }
    }

    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }

    pub fn is_expandable(&self) -> bool {
        self.kind.is_directory() && !self.children.is_empty()
    }
}

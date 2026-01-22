use std::path::PathBuf;

use dux_core::{DiskTree, NodeId, ScanProgress};

/// Application mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    /// Scanning filesystem
    Scanning,
    /// Finalizing scan (aggregating sizes)
    Finalizing,
    /// Browsing results
    Browsing,
    /// Showing help overlay
    Help,
}

/// Application state
pub struct AppState {
    /// Current mode
    pub mode: AppMode,
    /// Root path being scanned
    pub root_path: PathBuf,
    /// Disk tree (None while scanning)
    pub tree: Option<DiskTree>,
    /// Current scan progress
    pub progress: ScanProgress,
    /// Currently selected node index in visible list
    pub selected_index: usize,
    /// Current view root (for drill-down)
    pub view_root: NodeId,
    /// Navigation history (for going back)
    pub history: Vec<NodeId>,
    /// Scroll offset for tree view
    pub scroll_offset: usize,
    /// Visible area height (set by UI)
    pub visible_height: usize,
    /// Whether app should quit
    pub should_quit: bool,
    /// Spinner frame for animation
    pub spinner_frame: usize,
    /// Error message to display
    pub error_message: Option<String>,
}

impl AppState {
    pub fn new(root_path: PathBuf) -> Self {
        Self {
            mode: AppMode::Scanning,
            root_path,
            tree: None,
            progress: ScanProgress::default(),
            selected_index: 0,
            view_root: NodeId::ROOT,
            history: Vec::new(),
            scroll_offset: 0,
            visible_height: 20,
            should_quit: false,
            spinner_frame: 0,
            error_message: None,
        }
    }

    /// Set the tree after scanning completes
    pub fn set_tree(&mut self, tree: DiskTree) {
        self.tree = Some(tree);
        self.mode = AppMode::Browsing;
        self.selected_index = 0;
        self.view_root = NodeId::ROOT;
    }

    /// Update scan progress
    pub fn update_progress(&mut self, progress: ScanProgress) {
        self.progress = progress;
    }

    /// Set finalizing mode
    pub fn set_finalizing(&mut self) {
        self.mode = AppMode::Finalizing;
    }

    /// Advance spinner animation
    pub fn tick_spinner(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % 10;
    }

    /// Get visible nodes in current view
    pub fn visible_nodes(&self) -> Vec<NodeId> {
        match &self.tree {
            Some(tree) => tree.visible_nodes(self.view_root),
            None => Vec::new(),
        }
    }

    /// Get currently selected node ID
    pub fn selected_node(&self) -> Option<NodeId> {
        let nodes = self.visible_nodes();
        nodes.get(self.selected_index).copied()
    }

    /// Move selection up
    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.ensure_visible();
        }
    }

    /// Move selection down
    pub fn move_down(&mut self) {
        let nodes = self.visible_nodes();
        if self.selected_index < nodes.len().saturating_sub(1) {
            self.selected_index += 1;
            self.ensure_visible();
        }
    }

    /// Move selection up by a page
    pub fn page_up(&mut self) {
        let page_size = self.visible_height.saturating_sub(2);
        self.selected_index = self.selected_index.saturating_sub(page_size);
        self.ensure_visible();
    }

    /// Move selection down by a page
    pub fn page_down(&mut self) {
        let page_size = self.visible_height.saturating_sub(2);
        let nodes = self.visible_nodes();
        self.selected_index = (self.selected_index + page_size).min(nodes.len().saturating_sub(1));
        self.ensure_visible();
    }

    /// Go to first item
    pub fn go_to_first(&mut self) {
        self.selected_index = 0;
        self.ensure_visible();
    }

    /// Go to last item
    pub fn go_to_last(&mut self) {
        let nodes = self.visible_nodes();
        self.selected_index = nodes.len().saturating_sub(1);
        self.ensure_visible();
    }

    /// Ensure selected item is visible
    fn ensure_visible(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + self.visible_height {
            self.scroll_offset = self.selected_index - self.visible_height + 1;
        }
    }

    /// Toggle expand/collapse for selected node
    pub fn toggle_selected(&mut self) {
        if let Some(node_id) = self.selected_node() {
            if let Some(tree) = &mut self.tree {
                tree.toggle_expanded(node_id);
            }
        }
    }

    /// Expand selected node
    pub fn expand_selected(&mut self) {
        if let Some(node_id) = self.selected_node() {
            if let Some(tree) = &mut self.tree {
                tree.set_expanded(node_id, true);
            }
        }
    }

    /// Collapse selected node
    pub fn collapse_selected(&mut self) {
        if let Some(node_id) = self.selected_node() {
            if let Some(tree) = &mut self.tree {
                let node = tree.get(node_id);
                if let Some(node) = node {
                    if node.is_expanded {
                        tree.set_expanded(node_id, false);
                    } else if let Some(parent) = node.parent {
                        // If already collapsed, go to parent
                        tree.set_expanded(parent, false);
                        // Find parent's index in visible list
                        let nodes = tree.visible_nodes(self.view_root);
                        if let Some(idx) = nodes.iter().position(|&id| id == parent) {
                            self.selected_index = idx;
                            self.ensure_visible();
                        }
                    }
                }
            }
        }
    }

    /// Drill down into selected directory
    pub fn drill_down(&mut self) {
        if let Some(node_id) = self.selected_node() {
            if let Some(tree) = &self.tree {
                if let Some(node) = tree.get(node_id) {
                    if node.kind.is_directory() && node.has_children() {
                        self.history.push(self.view_root);
                        self.view_root = node_id;
                        self.selected_index = 0;
                        self.scroll_offset = 0;
                    }
                }
            }
        }
    }

    /// Go back to previous view
    pub fn go_back(&mut self) {
        if let Some(prev_root) = self.history.pop() {
            self.view_root = prev_root;
            self.selected_index = 0;
            self.scroll_offset = 0;
        }
    }

    /// Show help overlay
    pub fn show_help(&mut self) {
        self.mode = AppMode::Help;
    }

    /// Hide help overlay
    pub fn hide_help(&mut self) {
        self.mode = AppMode::Browsing;
    }

    /// Request quit
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Set error message
    pub fn set_error(&mut self, message: String) {
        self.error_message = Some(message);
    }

    /// Clear error message
    #[allow(dead_code)]
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }
}

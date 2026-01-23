use std::path::{Path, PathBuf};
use std::sync::mpsc;

use dux_core::{DiskTree, NodeId, ScanProgress};

/// Statistics tracked during the session
#[derive(Debug, Default, Clone)]
pub struct SessionStats {
    /// Total bytes freed by deletions
    pub bytes_freed: u64,
    /// Number of items deleted
    pub items_deleted: u32,
}

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
    /// Showing delete confirmation dialog
    ConfirmDelete,
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
    /// Item pending deletion (node ID and path for confirmation dialog)
    pub pending_delete: Option<(NodeId, PathBuf)>,
    /// Session statistics (deleted items, freed space)
    pub session_stats: SessionStats,
    /// Whether tree was loaded from cache
    pub loaded_from_cache: bool,
    /// Receiver for async delete results
    pub delete_receiver: Option<mpsc::Receiver<Result<(NodeId, u64), String>>>,
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
            pending_delete: None,
            session_stats: SessionStats::default(),
            loaded_from_cache: false,
            delete_receiver: None,
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
        if let Some(node_id) = self.selected_node()
            && let Some(tree) = &mut self.tree
        {
            tree.toggle_expanded(node_id);
        }
    }

    /// Expand selected node
    pub fn expand_selected(&mut self) {
        if let Some(node_id) = self.selected_node()
            && let Some(tree) = &mut self.tree
        {
            tree.set_expanded(node_id, true);
        }
    }

    /// Collapse selected node
    pub fn collapse_selected(&mut self) {
        if let Some(node_id) = self.selected_node()
            && let Some(tree) = &mut self.tree
        {
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

    /// Drill down into selected directory
    pub fn drill_down(&mut self) {
        if let Some(node_id) = self.selected_node()
            && let Some(tree) = &self.tree
            && let Some(node) = tree.get(node_id)
            && node.kind.is_directory()
            && node.has_children()
        {
            self.history.push(self.view_root);
            self.view_root = node_id;
            self.selected_index = 0;
            self.scroll_offset = 0;
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

    /// Open selected item in Finder (macOS)
    #[cfg(target_os = "macos")]
    pub fn open_in_finder(&self) {
        if let Some(node_id) = self.selected_node()
            && let Some(tree) = &self.tree
            && let Some(node) = tree.get(node_id)
        {
            std::process::Command::new("open")
                .arg("-R") // Reveal in Finder
                .arg(&node.path)
                .spawn()
                .ok();
        }
    }

    /// Open selected item in Finder (no-op on non-macOS)
    #[cfg(not(target_os = "macos"))]
    pub fn open_in_finder(&self) {
        // No-op on non-macOS platforms
    }

    /// Request delete - shows confirmation dialog
    pub fn request_delete(&mut self) {
        if let Some(node_id) = self.selected_node()
            && let Some(tree) = &self.tree
            && let Some(node) = tree.get(node_id)
        {
            self.pending_delete = Some((node_id, node.path.clone()));
            self.mode = AppMode::ConfirmDelete;
        }
    }

    /// Confirm and start async delete operation
    pub fn confirm_delete(&mut self) {
        if let Some((node_id, path)) = self.pending_delete.take() {
            // Get size before deletion
            let size = self
                .tree
                .as_ref()
                .and_then(|t| t.get(node_id))
                .map(|n| n.size)
                .unwrap_or(0);

            // Update tree immediately (optimistic update)
            if let Some(tree) = &mut self.tree {
                tree.remove_node(node_id);
            }
            self.adjust_selection_after_delete();

            // Spawn background deletion
            let (tx, rx) = mpsc::channel();
            self.delete_receiver = Some(rx);

            // Return to browsing immediately - deletion happens in background
            self.mode = AppMode::Browsing;

            std::thread::spawn(move || {
                let result = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };

                match result {
                    Ok(()) => {
                        let _ = tx.send(Ok((node_id, size)));
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("Delete failed: {}", e)));
                    }
                }
            });
        }
    }

    /// Check if async delete completed and handle result
    pub fn poll_delete(&mut self) {
        if let Some(rx) = &self.delete_receiver
            && let Ok(result) = rx.try_recv()
        {
            match result {
                Ok((_node_id, size)) => {
                    self.session_stats.bytes_freed += size;
                    self.session_stats.items_deleted += 1;
                }
                Err(e) => {
                    // Delete failed - we already removed from tree optimistically
                    // Could restore here but simpler to just show error
                    self.error_message = Some(e);
                }
            }
            self.delete_receiver = None;
            self.mode = AppMode::Browsing;
        }
    }

    /// Adjust selection after a node is deleted
    fn adjust_selection_after_delete(&mut self) {
        let nodes = self.visible_nodes();
        if nodes.is_empty() {
            self.selected_index = 0;
        } else if self.selected_index >= nodes.len() {
            self.selected_index = nodes.len().saturating_sub(1);
        }
        // Also reset scroll if needed
        if self.scroll_offset > 0 && self.scroll_offset >= nodes.len() {
            self.scroll_offset = nodes.len().saturating_sub(1);
        }
    }

    /// Cancel delete operation
    pub fn cancel_delete(&mut self) {
        self.pending_delete = None;
        self.mode = AppMode::Browsing;
    }

    /// Get path of pending delete item
    pub fn pending_delete_path(&self) -> Option<&Path> {
        self.pending_delete.as_ref().map(|(_, path)| path.as_path())
    }

    /// Get size of pending delete item
    pub fn pending_delete_size(&self) -> Option<u64> {
        let (node_id, _) = self.pending_delete.as_ref()?;
        self.tree.as_ref()?.get(*node_id).map(|n| n.size)
    }
}

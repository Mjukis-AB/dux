use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use dux_core::{DiskTree, NodeId, ScanProgress};

use super::views::ComputedViews;

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
    /// Showing delete confirmation dialog (single item)
    ConfirmDelete,
    /// Showing multi-delete confirmation dialog
    ConfirmMultiDelete,
    /// Multi-delete in progress with progress overlay
    MultiDeleting,
}

/// Which data projection is displayed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Tree,
    LargeFiles,
    BuildArtifacts,
}

/// Per-view selection state
#[derive(Debug, Clone, Default)]
pub struct ViewState {
    pub selected_index: usize,
    pub scroll_offset: usize,
}

/// Result from a single item in a multi-delete batch
pub enum MultiDeleteResult {
    Success { size: u64 },
    Failure { path: PathBuf, error: String },
}

/// Progress tracker for multi-delete operations
pub struct MultiDeleteProgress {
    pub total: usize,
    pub completed: usize,
    pub bytes_freed: u64,
    pub failures: Vec<(PathBuf, String)>,
    pub receiver: mpsc::Receiver<MultiDeleteResult>,
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
    /// Currently selected node index in visible list (tree view)
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
    /// Whether the tree has been modified (e.g. by deletion) and needs cache update
    pub tree_modified: bool,
    /// Receiver for async delete results
    pub delete_receiver: Option<mpsc::Receiver<Result<(NodeId, u64), String>>>,
    /// Current view mode
    pub view_mode: ViewMode,
    /// Large files view state
    pub large_files_state: ViewState,
    /// Build artifacts view state
    pub build_artifacts_state: ViewState,
    /// Pre-computed view data
    pub computed_views: ComputedViews,
    /// Multi-selected nodes (stable arena indices)
    pub selected_nodes: HashSet<NodeId>,
    /// Whether selecting mode is active (v toggles)
    pub selecting_mode: bool,
    /// Items pending multi-delete confirmation
    pub pending_multi_delete: Option<Vec<(NodeId, PathBuf, u64)>>,
    /// Multi-delete progress tracker
    pub multi_delete_progress: Option<MultiDeleteProgress>,
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
            tree_modified: false,
            delete_receiver: None,
            view_mode: ViewMode::Tree,
            large_files_state: ViewState::default(),
            build_artifacts_state: ViewState::default(),
            computed_views: ComputedViews::new(),
            selected_nodes: HashSet::new(),
            selecting_mode: false,
            pending_multi_delete: None,
            multi_delete_progress: None,
        }
    }

    /// Set the tree after scanning completes
    pub fn set_tree(&mut self, tree: DiskTree) {
        self.computed_views.rebuild(&tree);
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

    /// Get currently selected node ID (works for any view)
    pub fn selected_node(&self) -> Option<NodeId> {
        match self.view_mode {
            ViewMode::Tree => {
                let nodes = self.visible_nodes();
                nodes.get(self.selected_index).copied()
            }
            ViewMode::LargeFiles => self
                .computed_views
                .large_files
                .get(self.large_files_state.selected_index)
                .map(|e| e.node_id),
            ViewMode::BuildArtifacts => self
                .computed_views
                .build_artifacts
                .get(self.build_artifacts_state.selected_index)
                .map(|e| e.node_id),
        }
    }

    /// Get total item count for current view
    fn current_item_count(&self) -> usize {
        match self.view_mode {
            ViewMode::Tree => self.visible_nodes().len(),
            ViewMode::LargeFiles => self.computed_views.large_files.len(),
            ViewMode::BuildArtifacts => self.computed_views.build_artifacts.len(),
        }
    }

    /// Get mutable references to the active selection state
    fn active_selection_mut(&mut self) -> (&mut usize, &mut usize) {
        match self.view_mode {
            ViewMode::Tree => (&mut self.selected_index, &mut self.scroll_offset),
            ViewMode::LargeFiles => (
                &mut self.large_files_state.selected_index,
                &mut self.large_files_state.scroll_offset,
            ),
            ViewMode::BuildArtifacts => (
                &mut self.build_artifacts_state.selected_index,
                &mut self.build_artifacts_state.scroll_offset,
            ),
        }
    }

    /// Ensure the given index is visible within the scroll viewport
    fn ensure_visible_for(selected: &mut usize, scroll: &mut usize, visible_height: usize) {
        if *selected < *scroll {
            *scroll = *selected;
        } else if *selected >= *scroll + visible_height {
            *scroll = *selected - visible_height + 1;
        }
    }

    /// Move selection up
    pub fn move_up(&mut self) {
        let vh = self.visible_height;
        let (sel, scroll) = self.active_selection_mut();
        if *sel > 0 {
            *sel -= 1;
        }
        Self::ensure_visible_for(sel, scroll, vh);
    }

    /// Move selection down
    pub fn move_down(&mut self) {
        let count = self.current_item_count();
        let vh = self.visible_height;
        let (sel, scroll) = self.active_selection_mut();
        if *sel < count.saturating_sub(1) {
            *sel += 1;
        }
        Self::ensure_visible_for(sel, scroll, vh);
    }

    /// Move selection up by a page
    pub fn page_up(&mut self) {
        let vh = self.visible_height;
        let page_size = vh.saturating_sub(2);
        let (sel, scroll) = self.active_selection_mut();
        *sel = sel.saturating_sub(page_size);
        Self::ensure_visible_for(sel, scroll, vh);
    }

    /// Move selection down by a page
    pub fn page_down(&mut self) {
        let count = self.current_item_count();
        let vh = self.visible_height;
        let page_size = vh.saturating_sub(2);
        let (sel, scroll) = self.active_selection_mut();
        *sel = (*sel + page_size).min(count.saturating_sub(1));
        Self::ensure_visible_for(sel, scroll, vh);
    }

    /// Go to first item
    pub fn go_to_first(&mut self) {
        let vh = self.visible_height;
        let (sel, scroll) = self.active_selection_mut();
        *sel = 0;
        Self::ensure_visible_for(sel, scroll, vh);
    }

    /// Go to last item
    pub fn go_to_last(&mut self) {
        let count = self.current_item_count();
        let vh = self.visible_height;
        let (sel, scroll) = self.active_selection_mut();
        *sel = count.saturating_sub(1);
        Self::ensure_visible_for(sel, scroll, vh);
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
                        let scroll = &mut self.scroll_offset;
                        let sel = &mut self.selected_index;
                        Self::ensure_visible_for(sel, scroll, self.visible_height);
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

    /// Switch to next view mode
    pub fn next_view(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::Tree => ViewMode::LargeFiles,
            ViewMode::LargeFiles => ViewMode::BuildArtifacts,
            ViewMode::BuildArtifacts => ViewMode::Tree,
        };
        self.selected_nodes.clear();
        self.selecting_mode = false;
        self.ensure_views_computed();
    }

    /// Switch to previous view mode
    pub fn prev_view(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::Tree => ViewMode::BuildArtifacts,
            ViewMode::LargeFiles => ViewMode::Tree,
            ViewMode::BuildArtifacts => ViewMode::LargeFiles,
        };
        self.selected_nodes.clear();
        self.selecting_mode = false;
        self.ensure_views_computed();
    }

    /// Ensure computed views are up to date, clamp selections
    pub fn ensure_views_computed(&mut self) {
        if self.computed_views.dirty {
            if let Some(tree) = &self.tree {
                self.computed_views.rebuild(tree);
            }
            // Clamp selection indices
            let lf_count = self.computed_views.large_files.len();
            if self.large_files_state.selected_index >= lf_count {
                self.large_files_state.selected_index = lf_count.saturating_sub(1);
            }
            let ba_count = self.computed_views.build_artifacts.len();
            if self.build_artifacts_state.selected_index >= ba_count {
                self.build_artifacts_state.selected_index = ba_count.saturating_sub(1);
            }
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

    /// Request delete - shows confirmation dialog (single or multi)
    pub fn request_delete(&mut self) {
        // Guard: reject if a delete is already in progress
        if self.delete_receiver.is_some() || self.multi_delete_progress.is_some() {
            return;
        }

        if !self.selected_nodes.is_empty() {
            self.request_multi_delete();
        } else if let Some(node_id) = self.selected_node()
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
                self.tree_modified = true;
                self.computed_views.dirty = true;
            }
            // Remove from selection if present
            self.selected_nodes.remove(&node_id);
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
        match self.view_mode {
            ViewMode::Tree => {
                let nodes = self.visible_nodes();
                if nodes.is_empty() {
                    self.selected_index = 0;
                } else if self.selected_index >= nodes.len() {
                    self.selected_index = nodes.len().saturating_sub(1);
                }
                if self.scroll_offset > 0 && self.scroll_offset >= nodes.len() {
                    self.scroll_offset = nodes.len().saturating_sub(1);
                }
            }
            ViewMode::LargeFiles => {
                let count = self.computed_views.large_files.len();
                if self.large_files_state.selected_index >= count {
                    self.large_files_state.selected_index = count.saturating_sub(1);
                }
            }
            ViewMode::BuildArtifacts => {
                let count = self.computed_views.build_artifacts.len();
                if self.build_artifacts_state.selected_index >= count {
                    self.build_artifacts_state.selected_index = count.saturating_sub(1);
                }
            }
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

    // --- Selection methods ---

    /// Get the NodeId at a given visible index for the current view
    fn node_at_index(&self, idx: usize) -> Option<NodeId> {
        match self.view_mode {
            ViewMode::Tree => {
                let nodes = self.visible_nodes();
                nodes.get(idx).copied()
            }
            ViewMode::LargeFiles => self.computed_views.large_files.get(idx).map(|e| e.node_id),
            ViewMode::BuildArtifacts => self
                .computed_views
                .build_artifacts
                .get(idx)
                .map(|e| e.node_id),
        }
    }

    /// Add a node to the multi-selection set
    pub fn add_to_selection(&mut self, node_id: NodeId) {
        // Never select root
        if node_id != NodeId::ROOT {
            self.selected_nodes.insert(node_id);
        }
    }

    /// Toggle current item in/out of selection, entering selecting mode if needed
    pub fn toggle_select(&mut self) {
        if let Some(node_id) = self.node_at_index(self.current_selected_index()) {
            if node_id == NodeId::ROOT {
                return;
            }
            if self.selected_nodes.contains(&node_id) {
                self.selected_nodes.remove(&node_id);
                // Exit selecting mode if nothing left
                if self.selected_nodes.is_empty() {
                    self.selecting_mode = false;
                }
            } else {
                self.selected_nodes.insert(node_id);
                self.selecting_mode = true;
            }
        }
    }

    /// Clear the multi-selection and exit selecting mode
    pub fn clear_selection(&mut self) {
        self.selected_nodes.clear();
        self.selecting_mode = false;
    }

    /// Number of nodes in the multi-selection
    pub fn selection_count(&self) -> usize {
        self.selected_nodes.len()
    }

    /// Add current node to selection, move up, add new node
    pub fn select_move_up(&mut self) {
        if let Some(node_id) = self.node_at_index(self.current_selected_index()) {
            self.add_to_selection(node_id);
        }
        self.move_up();
        if let Some(node_id) = self.node_at_index(self.current_selected_index()) {
            self.add_to_selection(node_id);
        }
    }

    /// Add current node to selection, move down, add new node
    pub fn select_move_down(&mut self) {
        if let Some(node_id) = self.node_at_index(self.current_selected_index()) {
            self.add_to_selection(node_id);
        }
        self.move_down();
        if let Some(node_id) = self.node_at_index(self.current_selected_index()) {
            self.add_to_selection(node_id);
        }
    }

    /// Add range to selection while paging up
    pub fn select_page_up(&mut self) {
        let start = self.current_selected_index();
        self.page_up();
        let end = self.current_selected_index();
        for idx in end..=start {
            if let Some(node_id) = self.node_at_index(idx) {
                self.add_to_selection(node_id);
            }
        }
    }

    /// Add range to selection while paging down
    pub fn select_page_down(&mut self) {
        let start = self.current_selected_index();
        self.page_down();
        let end = self.current_selected_index();
        for idx in start..=end {
            if let Some(node_id) = self.node_at_index(idx) {
                self.add_to_selection(node_id);
            }
        }
    }

    /// Add range to selection while jumping to first
    pub fn select_to_first(&mut self) {
        let start = self.current_selected_index();
        self.go_to_first();
        for idx in 0..=start {
            if let Some(node_id) = self.node_at_index(idx) {
                self.add_to_selection(node_id);
            }
        }
    }

    /// Add range to selection while jumping to last
    pub fn select_to_last(&mut self) {
        let start = self.current_selected_index();
        self.go_to_last();
        let end = self.current_selected_index();
        for idx in start..=end {
            if let Some(node_id) = self.node_at_index(idx) {
                self.add_to_selection(node_id);
            }
        }
    }

    /// Get current selected index for the active view
    fn current_selected_index(&self) -> usize {
        match self.view_mode {
            ViewMode::Tree => self.selected_index,
            ViewMode::LargeFiles => self.large_files_state.selected_index,
            ViewMode::BuildArtifacts => self.build_artifacts_state.selected_index,
        }
    }

    // --- Multi-delete methods ---

    /// Remove children whose ancestor is also selected
    fn dedup_selected_nodes(&self) -> Vec<NodeId> {
        let tree = match &self.tree {
            Some(t) => t,
            None => return Vec::new(),
        };

        let mut result: Vec<NodeId> = Vec::new();
        for &node_id in &self.selected_nodes {
            // Walk up to check if any ancestor is also in the set
            let mut ancestor_selected = false;
            let mut current = node_id;
            while let Some(node) = tree.get(current) {
                if let Some(parent) = node.parent {
                    if self.selected_nodes.contains(&parent) {
                        ancestor_selected = true;
                        break;
                    }
                    current = parent;
                } else {
                    break;
                }
            }
            if !ancestor_selected {
                result.push(node_id);
            }
        }
        result
    }

    /// Prepare multi-delete: dedup, build item list, show confirm dialog
    fn request_multi_delete(&mut self) {
        let tree = match &self.tree {
            Some(t) => t,
            None => return,
        };

        let deduped = self.dedup_selected_nodes();
        if deduped.is_empty() {
            return;
        }

        let items: Vec<(NodeId, PathBuf, u64)> = deduped
            .into_iter()
            .filter_map(|id| {
                let node = tree.get(id)?;
                // Never delete root
                if id == NodeId::ROOT {
                    return None;
                }
                Some((id, node.path.clone(), node.size))
            })
            .collect();

        if items.is_empty() {
            return;
        }

        self.pending_multi_delete = Some(items);
        self.mode = AppMode::ConfirmMultiDelete;
    }

    /// Confirm multi-delete: optimistic tree removal + spawn concurrent threads
    pub fn confirm_multi_delete(&mut self) {
        let items = match self.pending_multi_delete.take() {
            Some(items) => items,
            None => return,
        };

        let total = items.len();

        // Optimistic tree removal
        if let Some(tree) = &mut self.tree {
            for &(node_id, _, _) in &items {
                tree.remove_node(node_id);
            }
            self.tree_modified = true;
            self.computed_views.dirty = true;
        }
        self.selected_nodes.clear();
        self.selecting_mode = false;
        self.adjust_selection_after_delete();

        // Shared channel for all delete threads
        let (tx, rx) = mpsc::channel();

        self.multi_delete_progress = Some(MultiDeleteProgress {
            total,
            completed: 0,
            bytes_freed: 0,
            failures: Vec::new(),
            receiver: rx,
        });
        self.mode = AppMode::MultiDeleting;

        // Spawn one thread per item (concurrent deletion)
        for (_node_id, path, size) in items {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let result = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                let msg = match result {
                    Ok(()) => MultiDeleteResult::Success { size },
                    Err(e) => MultiDeleteResult::Failure {
                        path,
                        error: format!("{}", e),
                    },
                };
                let _ = tx.send(msg);
            });
        }
    }

    /// Poll multi-delete channel, update progress, transition when done
    pub fn poll_multi_delete(&mut self) {
        let progress = match &mut self.multi_delete_progress {
            Some(p) => p,
            None => return,
        };

        while let Ok(result) = progress.receiver.try_recv() {
            progress.completed += 1;
            match result {
                MultiDeleteResult::Success { size, .. } => {
                    progress.bytes_freed += size;
                    self.session_stats.bytes_freed += size;
                    self.session_stats.items_deleted += 1;
                }
                MultiDeleteResult::Failure { path, error } => {
                    progress.failures.push((path, error));
                }
            }
        }

        if progress.completed >= progress.total {
            let failures = std::mem::take(
                &mut self
                    .multi_delete_progress
                    .as_mut()
                    .expect("checked above")
                    .failures,
            );
            if !failures.is_empty() {
                let msg = if failures.len() == 1 {
                    format!(
                        "Delete failed: {}: {}",
                        failures[0].0.display(),
                        failures[0].1
                    )
                } else {
                    format!(
                        "{} deletions failed (first: {})",
                        failures.len(),
                        failures[0].1
                    )
                };
                self.error_message = Some(msg);
            }
            self.multi_delete_progress = None;
            self.mode = AppMode::Browsing;
        }
    }

    /// Cancel multi-delete confirmation
    pub fn cancel_multi_delete(&mut self) {
        self.pending_multi_delete = None;
        self.mode = AppMode::Browsing;
    }
}

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

use super::views::ComputedViews;
use dux_core::{DiskTree, NodeId, ScanCoverage, ScanProgress};

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
    /// Whether tree was loaded from cache
    pub loaded_from_cache: bool,
    /// Original scan time for the current tree, including cache-backed trees
    scan_time: Option<SystemTime>,
    /// Coverage that qualifies the current tree. Legacy cache files cannot
    /// carry this value and therefore load as explicitly unknown.
    scan_coverage: ScanCoverage,
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
            loaded_from_cache: false,
            scan_time: None,
            scan_coverage: ScanCoverage::unknown(),
            view_mode: ViewMode::Tree,
            large_files_state: ViewState::default(),
            build_artifacts_state: ViewState::default(),
            computed_views: ComputedViews::new(),
            selected_nodes: HashSet::new(),
            selecting_mode: false,
        }
    }

    /// Set the tree after scanning completes
    #[cfg(test)]
    pub fn set_tree(&mut self, tree: DiskTree) {
        self.install_tree(tree);
        self.loaded_from_cache = false;
        self.scan_time = None;
        self.scan_coverage = ScanCoverage::unknown();
    }

    pub fn set_cached_tree(&mut self, tree: DiskTree, scan_time: SystemTime) {
        self.install_tree(tree);
        self.loaded_from_cache = true;
        self.scan_time = Some(scan_time);
        self.scan_coverage = ScanCoverage::unknown();
    }

    pub fn set_scanned_tree(
        &mut self,
        tree: DiskTree,
        scan_time: SystemTime,
        coverage: ScanCoverage,
    ) {
        self.install_tree(tree);
        self.loaded_from_cache = false;
        self.scan_time = Some(scan_time);
        self.scan_coverage = coverage;
    }

    fn install_tree(&mut self, tree: DiskTree) {
        self.computed_views.rebuild(&tree);
        self.tree = Some(tree);
        self.mode = AppMode::Browsing;
        self.selected_index = 0;
        self.view_root = NodeId::ROOT;
        self.history.clear();
        self.scroll_offset = 0;
        self.large_files_state = ViewState::default();
        self.build_artifacts_state = ViewState::default();
        self.selected_nodes.clear();
        self.selecting_mode = false;
    }

    pub fn scan_time(&self) -> Option<SystemTime> {
        self.scan_time
    }

    pub fn scan_coverage(&self) -> &ScanCoverage {
        &self.scan_coverage
    }

    pub fn prepare_rescan(&mut self) -> bool {
        self.mode = AppMode::Scanning;
        self.progress = ScanProgress::default();
        self.spinner_frame = 0;
        self.error_message = None;
        true
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
    #[expect(
        clippy::disallowed_methods,
        reason = "non-shell Finder reveal uses a fixed executable and argument structure"
    )]
    pub fn open_in_finder(&self) {
        if let Some(node_id) = self.selected_node()
            && let Some(tree) = &self.tree
            && let Some(node) = tree.get(node_id)
        {
            // DUX-DESTRUCTIVE: allow=finder-reveal -- fixed open command reveals one selected path without shell evaluation
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

    /// Total bytes affected by the selection after removing nested overlap.
    pub fn selection_total_size(&self) -> u64 {
        let Some(tree) = &self.tree else {
            return 0;
        };
        self.effective_selected_nodes()
            .into_iter()
            .filter_map(|node_id| tree.get(node_id))
            .fold(0, |total, node| total.saturating_add(node.size))
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

    /// Selected live nodes whose ancestors are not also selected.
    fn effective_selected_nodes(&self) -> Vec<NodeId> {
        let tree = match &self.tree {
            Some(t) => t,
            None => return Vec::new(),
        };

        let mut result = Vec::new();
        for &node_id in &self.selected_nodes {
            if node_id == NodeId::ROOT || tree.get(node_id).is_none() {
                continue;
            }
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
        result.sort_by_key(|node_id| node_id.index());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use dux_core::NodeKind;

    fn state_with_child() -> (AppState, NodeId) {
        let root = PathBuf::from("/test-root");
        let mut tree = DiskTree::new(root.clone());
        let child = tree.add_node(
            "file.txt".to_string(),
            NodeKind::File,
            root.join("file.txt"),
            NodeId::ROOT,
        );
        tree.set_size(child, 10);
        tree.aggregate_sizes();

        let mut state = AppState::new(root);
        state.set_tree(tree);
        (state, child)
    }

    #[test]
    fn prepare_rescan_retains_current_tree_until_replacement_succeeds() {
        let (mut state, child) = state_with_child();
        let tree = state.tree.take().unwrap();
        state.set_cached_tree(tree, SystemTime::UNIX_EPOCH);
        state.selected_nodes.insert(child);
        state.history.push(NodeId::ROOT);
        state.view_root = child;

        assert!(state.prepare_rescan());

        assert_eq!(state.mode, AppMode::Scanning);
        assert!(state.tree.is_some());
        assert!(state.loaded_from_cache);
        assert_eq!(state.scan_time(), Some(SystemTime::UNIX_EPOCH));
        assert!(state.selected_nodes.contains(&child));
        assert_eq!(state.history, vec![NodeId::ROOT]);
        assert_eq!(state.view_root, child);

        let replacement_time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(60);
        state.set_scanned_tree(
            DiskTree::new(state.root_path.clone()),
            replacement_time,
            ScanCoverage::unknown(),
        );
        assert_eq!(state.mode, AppMode::Browsing);
        assert!(!state.loaded_from_cache);
        assert_eq!(state.scan_time(), Some(replacement_time));
        assert!(state.selected_nodes.is_empty());
        assert!(state.history.is_empty());
        assert_eq!(state.view_root, NodeId::ROOT);
    }

    #[test]
    fn selection_total_counts_nested_nodes_only_once() {
        let root = PathBuf::from("/test-root");
        let mut tree = DiskTree::new(root.clone());
        let directory = tree.add_node(
            "directory".to_string(),
            NodeKind::Directory,
            root.join("directory"),
            NodeId::ROOT,
        );
        let child = tree.add_node(
            "child.bin".to_string(),
            NodeKind::File,
            root.join("directory/child.bin"),
            directory,
        );
        let sibling = tree.add_node(
            "sibling.bin".to_string(),
            NodeKind::File,
            root.join("sibling.bin"),
            NodeId::ROOT,
        );
        tree.set_size(directory, 100);
        tree.set_size(child, 40);
        tree.set_size(sibling, 25);
        let mut state = AppState::new(root);
        state.set_tree(tree);
        state.selected_nodes.extend([directory, child, sibling]);

        assert_eq!(state.selection_count(), 3);
        assert_eq!(state.effective_selected_nodes(), vec![directory, sibling]);
        assert_eq!(state.selection_total_size(), 125);
    }

    #[test]
    fn selection_total_ignores_stale_ids_and_saturates() {
        let root = PathBuf::from("/test-root");
        let mut tree = DiskTree::new(root.clone());
        let first = tree.add_node(
            "first.bin".to_string(),
            NodeKind::File,
            root.join("first.bin"),
            NodeId::ROOT,
        );
        let second = tree.add_node(
            "second.bin".to_string(),
            NodeKind::File,
            root.join("second.bin"),
            NodeId::ROOT,
        );
        tree.set_size(first, u64::MAX);
        tree.set_size(second, 1);
        let mut state = AppState::new(root);
        state.set_tree(tree);
        state.view_mode = ViewMode::LargeFiles;
        state
            .selected_nodes
            .extend([first, second, NodeId(usize::MAX)]);

        assert_eq!(state.effective_selected_nodes(), vec![first, second]);
        assert_eq!(state.selection_total_size(), u64::MAX);
    }

    #[test]
    fn quit_exits_immediately() {
        let (mut state, _) = state_with_child();

        state.quit();

        assert!(state.should_quit);
    }
}

use std::collections::{HashMap, HashSet};
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;

use dux_core::{DiskTree, NodeId, ScanProgress};

use super::deletion::{PlannedDelete, capture_delete_plan, execute_planned_delete};
use super::views::ComputedViews;

const MULTI_DELETE_WORKER_LIMIT: usize = 4;

type WorkerMain = Box<dyn FnOnce() + Send + 'static>;

fn spawn_bounded_workers<T, F>(
    items: Vec<T>,
    worker_limit: usize,
    work: F,
) -> io::Result<Vec<JoinHandle<()>>>
where
    T: Send + 'static,
    F: Fn(T) + Send + Sync + 'static,
{
    spawn_bounded_workers_with(items, worker_limit, work, |index, worker_main| {
        std::thread::Builder::new()
            .name(format!("dux-delete-{index}"))
            .spawn(worker_main)
    })
}

fn spawn_bounded_workers_with<T, F, S>(
    items: Vec<T>,
    worker_limit: usize,
    work: F,
    mut spawn: S,
) -> io::Result<Vec<JoinHandle<()>>>
where
    T: Send + 'static,
    F: Fn(T) + Send + Sync + 'static,
    S: FnMut(usize, WorkerMain) -> io::Result<JoinHandle<()>>,
{
    assert!(worker_limit > 0, "worker limit must be non-zero");
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let worker_count = worker_limit.min(items.len());
    let (task_tx, task_rx) = crossbeam_channel::unbounded();
    let work = Arc::new(work);
    let mut workers = Vec::with_capacity(worker_count);
    for index in 0..worker_count {
        let task_rx = task_rx.clone();
        let work = Arc::clone(&work);
        let worker_main: WorkerMain = Box::new(move || {
            while let Ok(item) = task_rx.recv() {
                work(item);
            }
        });
        match spawn(index, worker_main) {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                drop(task_tx);
                for worker in workers {
                    let _ = worker.join();
                }
                return Err(error);
            }
        }
    }

    for item in items {
        if task_tx.send(item).is_err() {
            drop(task_tx);
            for worker in workers {
                let _ = worker.join();
            }
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "delete worker queue disconnected",
            ));
        }
    }
    drop(task_tx);
    Ok(workers)
}

struct MultiDeleteTask {
    node_id: NodeId,
    path: PathBuf,
    size: u64,
    plan: PlannedDelete,
}

fn execute_multi_delete_task<E, F>(task: MultiDeleteTask, execute: F) -> MultiDeleteResult
where
    E: std::fmt::Display,
    F: FnOnce(&Path, PlannedDelete) -> Result<(), E>,
{
    let MultiDeleteTask {
        node_id,
        path,
        size,
        plan,
    } = task;
    let failure_path = path.clone();
    match catch_unwind(AssertUnwindSafe(|| match execute(&path, plan) {
        Ok(()) => MultiDeleteResult::Success { node_id, size },
        Err(error) => MultiDeleteResult::Failure {
            path,
            error: error.to_string(),
        },
    })) {
        Ok(result) => result,
        Err(_) => MultiDeleteResult::Failure {
            path: failure_path,
            error: "delete worker panicked while deleting this item".to_string(),
        },
    }
}

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
    Success { node_id: NodeId, size: u64 },
    Failure { path: PathBuf, error: String },
}

struct DeleteWorker {
    path: PathBuf,
    handle: JoinHandle<()>,
}

/// Progress tracker for multi-delete operations
pub struct MultiDeleteProgress {
    pub total: usize,
    pub completed: usize,
    pub bytes_freed: u64,
    pub failures: Vec<(PathBuf, String)>,
    pub receiver: mpsc::Receiver<MultiDeleteResult>,
    workers: Vec<DeleteWorker>,
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
    /// Whether quit is waiting for active deletion workers to finish
    pub quit_requested: bool,
    /// Spinner frame for animation
    pub spinner_frame: usize,
    /// Error message to display
    pub error_message: Option<String>,
    /// Item pending deletion (node ID and path for confirmation dialog)
    pub pending_delete: Option<(NodeId, PathBuf)>,
    /// Target and artifact-evidence identities captured before confirmation
    pending_delete_plan: Option<PlannedDelete>,
    /// Session statistics (deleted items, freed space)
    pub session_stats: SessionStats,
    /// Whether tree was loaded from cache
    pub loaded_from_cache: bool,
    /// Whether the tree has been modified (e.g. by deletion) and needs cache update
    pub tree_modified: bool,
    /// Receiver for async delete results
    pub delete_receiver: Option<mpsc::Receiver<Result<(NodeId, u64), String>>>,
    /// Tracked worker for the current single-item deletion
    delete_worker: Option<DeleteWorker>,
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
    /// Target and artifact-evidence identities captured before batch confirmation
    pending_multi_delete_plans: HashMap<NodeId, PlannedDelete>,
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
            quit_requested: false,
            spinner_frame: 0,
            error_message: None,
            pending_delete: None,
            pending_delete_plan: None,
            session_stats: SessionStats::default(),
            loaded_from_cache: false,
            tree_modified: false,
            delete_receiver: None,
            delete_worker: None,
            view_mode: ViewMode::Tree,
            large_files_state: ViewState::default(),
            build_artifacts_state: ViewState::default(),
            computed_views: ComputedViews::new(),
            selected_nodes: HashSet::new(),
            selecting_mode: false,
            pending_multi_delete: None,
            pending_multi_delete_plans: HashMap::new(),
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
        if self.has_in_flight_deletions() {
            self.quit_requested = true;
            if self.multi_delete_progress.is_some() {
                self.mode = AppMode::MultiDeleting;
            }
        } else {
            self.should_quit = true;
        }
    }

    fn has_in_flight_deletions(&self) -> bool {
        self.delete_receiver.is_some() || self.multi_delete_progress.is_some()
    }

    fn finish_deferred_quit_if_idle(&mut self) {
        if self.quit_requested && !self.has_in_flight_deletions() {
            self.should_quit = true;
        }
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
            && node_id != NodeId::ROOT
            && let Some(tree) = &self.tree
            && let Some(node) = tree.get(node_id)
        {
            let path = node.path.clone();
            if self.has_symlink_ancestor(node_id) {
                self.error_message = Some(format!(
                    "Delete refused because {} is beneath a followed symlink",
                    path.display()
                ));
                return;
            }
            let evidence_paths = self.artifact_evidence_paths(node_id);
            match capture_delete_plan(&self.root_path, &path, &evidence_paths) {
                Ok(plan) => {
                    self.pending_delete = Some((node_id, path));
                    self.pending_delete_plan = Some(plan);
                    self.mode = AppMode::ConfirmDelete;
                }
                Err(error) => {
                    self.error_message = Some(format!(
                        "Could not prepare delete for {}: {error}",
                        path.display()
                    ));
                }
            }
        }
    }

    /// Confirm and start async delete operation
    pub fn confirm_delete(&mut self) {
        if let Some((node_id, path)) = self.pending_delete.take() {
            let plan = self.pending_delete_plan.take();

            // Defense in depth: the scan root must never be deleted.
            if node_id == NodeId::ROOT {
                self.mode = AppMode::Browsing;
                return;
            }

            let Some(plan) = plan else {
                self.error_message = Some(
                    "Delete cancelled because its filesystem identity was not captured. Rescan before trying again."
                        .to_string(),
                );
                self.mode = AppMode::Browsing;
                return;
            };

            // Get size before deletion
            let size = self
                .tree
                .as_ref()
                .and_then(|t| t.get(node_id))
                .map(|n| n.size)
                .unwrap_or(0);

            // Spawn background deletion
            let (tx, rx) = mpsc::channel();
            self.delete_receiver = Some(rx);

            // Return to browsing immediately - deletion happens in background
            self.mode = AppMode::Browsing;

            let worker_path = path.clone();
            let handle = std::thread::spawn(move || {
                let result = execute_planned_delete(&path, plan);

                match result {
                    Ok(()) => {
                        let _ = tx.send(Ok((node_id, size)));
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                    }
                }
            });
            self.delete_worker = Some(DeleteWorker {
                path: worker_path,
                handle,
            });
        }
    }

    /// Check if async delete completed and handle result
    pub fn poll_delete(&mut self) {
        let Some(rx) = self.delete_receiver.take() else {
            return;
        };

        match rx.try_recv() {
            Ok(result) => {
                if let Some(worker) = self.delete_worker.take() {
                    let _ = worker.handle.join();
                }
                match result {
                    Ok((node_id, size)) => {
                        if let Some(tree) = &mut self.tree {
                            tree.remove_node(node_id);
                            self.tree_modified = true;
                            self.computed_views.dirty = true;
                        }
                        self.selected_nodes.remove(&node_id);
                        self.adjust_selection_after_delete();
                        self.session_stats.bytes_freed += size;
                        self.session_stats.items_deleted += 1;
                    }
                    Err(e) => {
                        self.error_message = Some(e);
                    }
                }
                self.mode = AppMode::Browsing;
                self.finish_deferred_quit_if_idle();
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.delete_receiver = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                if let Some(worker) = self.delete_worker.take() {
                    let path = worker.path;
                    let _ = worker.handle.join();
                    self.error_message = Some(format!(
                        "Delete worker stopped without a result: {}",
                        path.display()
                    ));
                } else {
                    self.error_message = Some("Delete worker stopped without a result".to_string());
                }
                self.mode = AppMode::Browsing;
                self.finish_deferred_quit_if_idle();
            }
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
        self.pending_delete_plan = None;
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

        let mut plans = HashMap::with_capacity(items.len());
        for (node_id, path, _) in &items {
            if self.has_symlink_ancestor(*node_id) {
                self.error_message = Some(format!(
                    "Delete refused because {} is beneath a followed symlink. No deletion was started.",
                    path.display()
                ));
                return;
            }
            let evidence_paths = self.artifact_evidence_paths(*node_id);
            match capture_delete_plan(&self.root_path, path, &evidence_paths) {
                Ok(plan) => {
                    plans.insert(*node_id, plan);
                }
                Err(error) => {
                    self.error_message = Some(format!(
                        "Could not prepare delete for {}: {error}. No deletion was started.",
                        path.display()
                    ));
                    return;
                }
            }
        }

        self.pending_multi_delete = Some(items);
        self.pending_multi_delete_plans = plans;
        self.mode = AppMode::ConfirmMultiDelete;
    }

    /// Confirm multi-delete and spawn concurrent filesystem operations
    pub fn confirm_multi_delete(&mut self) {
        let items = match self.pending_multi_delete.take() {
            Some(items) => items,
            None => return,
        };
        let mut plans = std::mem::take(&mut self.pending_multi_delete_plans);

        let mut planned_items = Vec::with_capacity(items.len());
        for (node_id, path, size) in items {
            if node_id == NodeId::ROOT {
                self.error_message =
                    Some("Multi-delete cancelled because it included the scan root.".to_string());
                self.mode = AppMode::Browsing;
                return;
            }
            let Some(plan) = plans.remove(&node_id) else {
                self.error_message = Some(
                    "Multi-delete cancelled because a filesystem identity was not captured. Rescan before trying again."
                        .to_string(),
                );
                self.mode = AppMode::Browsing;
                return;
            };
            planned_items.push(MultiDeleteTask {
                node_id,
                path,
                size,
                plan,
            });
        }

        let total = planned_items.len();
        let (tx, rx) = mpsc::channel();
        let worker_path = self.root_path.clone();
        let handles =
            match spawn_bounded_workers(planned_items, MULTI_DELETE_WORKER_LIMIT, move |task| {
                let msg = execute_multi_delete_task(task, execute_planned_delete);
                let _ = tx.send(msg);
            }) {
                Ok(handles) => handles,
                Err(error) => {
                    self.error_message = Some(format!(
                        "Could not start multi-delete workers: {error}. No deletion was started."
                    ));
                    self.mode = AppMode::Browsing;
                    return;
                }
            };

        self.selected_nodes.clear();
        self.selecting_mode = false;

        self.multi_delete_progress = Some(MultiDeleteProgress {
            total,
            completed: 0,
            bytes_freed: 0,
            failures: Vec::new(),
            receiver: rx,
            workers: Vec::with_capacity(MULTI_DELETE_WORKER_LIMIT.min(total)),
        });
        self.mode = AppMode::MultiDeleting;

        for handle in handles {
            self.multi_delete_progress
                .as_mut()
                .expect("created above")
                .workers
                .push(DeleteWorker {
                    path: worker_path.clone(),
                    handle,
                });
        }
    }

    /// Poll multi-delete channel, update progress, transition when done
    pub fn poll_multi_delete(&mut self) {
        let mut progress = match self.multi_delete_progress.take() {
            Some(progress) => progress,
            None => return,
        };

        while let Ok(result) = progress.receiver.try_recv() {
            progress.completed += 1;
            match result {
                MultiDeleteResult::Success { node_id, size } => {
                    if let Some(tree) = &mut self.tree {
                        tree.remove_node(node_id);
                        self.tree_modified = true;
                        self.computed_views.dirty = true;
                    }
                    progress.bytes_freed += size;
                    self.session_stats.bytes_freed += size;
                    self.session_stats.items_deleted += 1;
                }
                MultiDeleteResult::Failure { path, error } => {
                    progress.failures.push((path, error));
                }
            }
        }

        let all_workers_finished = progress
            .workers
            .iter()
            .all(|worker| worker.handle.is_finished());
        if all_workers_finished {
            while let Ok(result) = progress.receiver.try_recv() {
                progress.completed += 1;
                match result {
                    MultiDeleteResult::Success { node_id, size } => {
                        if let Some(tree) = &mut self.tree {
                            tree.remove_node(node_id);
                            self.tree_modified = true;
                            self.computed_views.dirty = true;
                        }
                        progress.bytes_freed += size;
                        self.session_stats.bytes_freed += size;
                        self.session_stats.items_deleted += 1;
                    }
                    MultiDeleteResult::Failure { path, error } => {
                        progress.failures.push((path, error));
                    }
                }
            }
        }

        if progress.completed >= progress.total || all_workers_finished {
            let missing_results = progress.total.saturating_sub(progress.completed);
            let mut panicked_workers = 0;
            for worker in progress.workers.drain(..) {
                let path = worker.path;
                if worker.handle.join().is_err() {
                    panicked_workers += 1;
                    progress
                        .failures
                        .push((path, "delete worker panicked".to_string()));
                }
            }
            let unexplained_missing = missing_results.saturating_sub(panicked_workers);
            if unexplained_missing > 0 {
                progress.failures.push((
                    self.root_path.clone(),
                    format!("{unexplained_missing} delete worker result(s) were lost"),
                ));
            }

            self.adjust_selection_after_delete();
            let failures = std::mem::take(&mut progress.failures);
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
            self.mode = AppMode::Browsing;
            self.finish_deferred_quit_if_idle();
        } else {
            self.multi_delete_progress = Some(progress);
        }
    }

    /// Cancel multi-delete confirmation
    pub fn cancel_multi_delete(&mut self) {
        self.pending_multi_delete = None;
        self.pending_multi_delete_plans.clear();
        self.mode = AppMode::Browsing;
    }

    fn artifact_evidence_paths(&self, node_id: NodeId) -> Vec<PathBuf> {
        self.computed_views
            .build_artifacts
            .iter()
            .find(|entry| entry.node_id == node_id)
            .map(|entry| entry.evidence_paths.clone())
            .unwrap_or_default()
    }

    fn has_symlink_ancestor(&self, node_id: NodeId) -> bool {
        let Some(tree) = &self.tree else {
            return true;
        };
        let mut parent_id = tree.get(node_id).and_then(|node| node.parent);
        while let Some(id) = parent_id {
            let Some(parent) = tree.get(id) else {
                return true;
            };
            if parent.path_is_symlink {
                return true;
            }
            parent_id = parent.parent;
        }
        false
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        if let Some(worker) = self.delete_worker.take() {
            let _ = worker.handle.join();
        }
        if let Some(progress) = &mut self.multi_delete_progress {
            for worker in progress.workers.drain(..) {
                let _ = worker.handle.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use dux_core::NodeKind;
    use tempfile::TempDir;

    #[test]
    fn bounded_workers_cap_concurrency_and_process_every_item() {
        const LIMIT: usize = 4;
        const ITEMS: usize = 12;

        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let first_wave = Arc::new(Barrier::new(LIMIT + 1));

        let worker_active = Arc::clone(&active);
        let worker_peak = Arc::clone(&peak);
        let worker_completed = Arc::clone(&completed);
        let worker_first_wave = Arc::clone(&first_wave);
        let workers = spawn_bounded_workers((0..ITEMS).collect(), LIMIT, move |item| {
            let now_active = worker_active.fetch_add(1, Ordering::SeqCst) + 1;
            worker_peak.fetch_max(now_active, Ordering::SeqCst);
            if item < LIMIT {
                worker_first_wave.wait();
            }
            worker_completed.fetch_add(1, Ordering::SeqCst);
            worker_active.fetch_sub(1, Ordering::SeqCst);
        })
        .unwrap();

        assert_eq!(workers.len(), LIMIT);
        first_wave.wait();
        for worker in workers {
            worker.join().unwrap();
        }

        assert_eq!(peak.load(Ordering::SeqCst), LIMIT);
        assert_eq!(completed.load(Ordering::SeqCst), ITEMS);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn partial_worker_start_failure_runs_no_items() {
        let executed = Arc::new(AtomicUsize::new(0));
        let worker_executed = Arc::clone(&executed);

        let result = spawn_bounded_workers_with(
            vec![1, 2, 3, 4],
            4,
            move |_| {
                worker_executed.fetch_add(1, Ordering::SeqCst);
            },
            |index, worker_main| {
                if index == 2 {
                    Err(io::Error::other("injected spawn failure"))
                } else {
                    std::thread::Builder::new().spawn(worker_main)
                }
            },
        );

        assert!(result.is_err());
        assert_eq!(executed.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn multi_delete_task_panic_becomes_an_item_failure() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("item.txt");
        std::fs::write(&path, b"keep").unwrap();
        let plan = capture_delete_plan(temp.path(), &path, &[]).unwrap();

        let result = execute_multi_delete_task(
            MultiDeleteTask {
                node_id: NodeId(1),
                path: path.clone(),
                size: 4,
                plan,
            },
            |_, _| -> Result<(), String> { panic!("intentional task panic") },
        );

        let MultiDeleteResult::Failure {
            path: failed_path,
            error,
        } = result
        else {
            panic!("panicking task unexpectedly succeeded");
        };
        assert_eq!(failed_path, path);
        assert!(error.contains("panicked"));
        assert!(path.exists());
    }

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
    fn single_delete_never_targets_root() {
        let (mut state, _) = state_with_child();

        state.request_delete();

        assert!(state.pending_delete.is_none());
        assert_eq!(state.mode, AppMode::Browsing);
    }

    #[test]
    fn single_delete_confirmation_rejects_root() {
        let (mut state, _) = state_with_child();
        state.pending_delete = Some((NodeId::ROOT, state.root_path.clone()));
        state.mode = AppMode::ConfirmDelete;

        state.confirm_delete();

        assert!(state.delete_receiver.is_none());
        assert!(
            state
                .tree
                .as_ref()
                .and_then(|tree| tree.get(NodeId::ROOT))
                .is_some()
        );
        assert_eq!(state.mode, AppMode::Browsing);
    }

    #[test]
    fn failed_single_delete_keeps_tree_node() {
        let (mut state, child) = state_with_child();
        let (tx, rx) = mpsc::channel();
        state.delete_receiver = Some(rx);
        tx.send(Err("Delete failed".to_string())).unwrap();

        state.poll_delete();

        assert!(
            state
                .tree
                .as_ref()
                .and_then(|tree| tree.get(child))
                .is_some()
        );
        assert!(!state.tree_modified);
        assert_eq!(state.session_stats.items_deleted, 0);
    }

    #[test]
    fn successful_single_delete_updates_tree() {
        let (mut state, child) = state_with_child();
        let (tx, rx) = mpsc::channel();
        state.delete_receiver = Some(rx);
        tx.send(Ok((child, 10))).unwrap();

        state.poll_delete();

        assert!(
            state
                .tree
                .as_ref()
                .and_then(|tree| tree.get(child))
                .is_none()
        );
        assert!(state.tree_modified);
        assert_eq!(state.session_stats.items_deleted, 1);
        assert_eq!(state.session_stats.bytes_freed, 10);
    }

    #[test]
    fn failed_multi_delete_keeps_tree_node() {
        let (mut state, child) = state_with_child();
        let (tx, rx) = mpsc::channel();
        state.multi_delete_progress = Some(MultiDeleteProgress {
            total: 1,
            completed: 0,
            bytes_freed: 0,
            failures: Vec::new(),
            receiver: rx,
            workers: Vec::new(),
        });
        tx.send(MultiDeleteResult::Failure {
            path: PathBuf::from("/test-root/file.txt"),
            error: "Delete failed".to_string(),
        })
        .unwrap();

        state.poll_multi_delete();

        assert!(
            state
                .tree
                .as_ref()
                .and_then(|tree| tree.get(child))
                .is_some()
        );
        assert!(!state.tree_modified);
        assert_eq!(state.session_stats.items_deleted, 0);
    }

    #[test]
    fn successful_multi_delete_updates_tree() {
        let (mut state, child) = state_with_child();
        let (tx, rx) = mpsc::channel();
        state.multi_delete_progress = Some(MultiDeleteProgress {
            total: 1,
            completed: 0,
            bytes_freed: 0,
            failures: Vec::new(),
            receiver: rx,
            workers: Vec::new(),
        });
        tx.send(MultiDeleteResult::Success {
            node_id: child,
            size: 10,
        })
        .unwrap();

        state.poll_multi_delete();

        assert!(
            state
                .tree
                .as_ref()
                .and_then(|tree| tree.get(child))
                .is_none()
        );
        assert!(state.tree_modified);
        assert_eq!(state.session_stats.items_deleted, 1);
    }

    #[test]
    fn quit_without_deletion_exits_immediately() {
        let (mut state, _) = state_with_child();

        state.quit();

        assert!(state.should_quit);
        assert!(!state.quit_requested);
    }

    #[test]
    fn quit_waits_for_single_delete_and_applies_its_result() {
        let (mut state, child) = state_with_child();
        let (release_tx, release_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        state.delete_receiver = Some(result_rx);
        state.delete_worker = Some(DeleteWorker {
            path: PathBuf::from("/test-root/file.txt"),
            handle: std::thread::spawn(move || {
                release_rx.recv().unwrap();
                result_tx.send(Ok((child, 10))).unwrap();
            }),
        });

        state.quit();
        state.poll_delete();
        assert!(state.quit_requested);
        assert!(!state.should_quit);

        release_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !state.should_quit {
            assert!(std::time::Instant::now() < deadline);
            state.poll_delete();
            std::thread::yield_now();
        }

        assert!(
            state
                .tree
                .as_ref()
                .and_then(|tree| tree.get(child))
                .is_none()
        );
        assert!(state.tree_modified);
        assert_eq!(state.session_stats.items_deleted, 1);
        assert!(state.delete_worker.is_none());
    }

    #[test]
    fn confirmed_real_delete_finishes_before_quit() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("delete-me.txt");
        std::fs::write(&file_path, b"payload").unwrap();

        let mut tree = DiskTree::new(temp.path().to_path_buf());
        let child = tree.add_node(
            "delete-me.txt".to_string(),
            NodeKind::File,
            file_path.clone(),
            NodeId::ROOT,
        );
        tree.set_size(child, 7);
        let mut state = AppState::new(temp.path().to_path_buf());
        state.set_tree(tree);
        state.selected_index = 1;
        state.request_delete();
        assert_eq!(state.mode, AppMode::ConfirmDelete);

        state.confirm_delete();
        state.quit();
        assert!(!state.should_quit);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !state.should_quit {
            assert!(std::time::Instant::now() < deadline);
            state.poll_delete();
            std::thread::yield_now();
        }

        assert!(!file_path.exists());
        assert!(state.tree.as_ref().unwrap().get(child).is_none());
        assert!(state.delete_worker.is_none());
    }

    #[test]
    fn single_delete_plan_fails_closed_when_path_cannot_be_inspected() {
        let (mut state, _) = state_with_child();
        state.selected_index = 1;

        state.request_delete();

        assert!(state.pending_delete.is_none());
        assert!(state.pending_delete_plan.is_none());
        assert_eq!(state.mode, AppMode::Browsing);
        assert!(
            state
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("Could not prepare delete"))
        );
    }

    #[test]
    fn multi_delete_skips_replaced_item_and_deletes_unchanged_item() {
        let temp = TempDir::new().unwrap();
        let first_path = temp.path().join("first.txt");
        let second_path = temp.path().join("second.txt");
        let original_second = temp.path().join("original-second.txt");
        std::fs::write(&first_path, b"first").unwrap();
        std::fs::write(&second_path, b"second").unwrap();

        let mut tree = DiskTree::new(temp.path().to_path_buf());
        let first = tree.add_node(
            "first.txt".to_string(),
            NodeKind::File,
            first_path.clone(),
            NodeId::ROOT,
        );
        let second = tree.add_node(
            "second.txt".to_string(),
            NodeKind::File,
            second_path.clone(),
            NodeId::ROOT,
        );
        tree.set_size(first, 5);
        tree.set_size(second, 6);

        let mut state = AppState::new(temp.path().to_path_buf());
        state.set_tree(tree);
        state.selected_nodes.insert(first);
        state.selected_nodes.insert(second);
        state.request_delete();
        assert_eq!(state.mode, AppMode::ConfirmMultiDelete);

        std::fs::rename(&second_path, &original_second).unwrap();
        std::fs::write(&second_path, b"replacement").unwrap();
        state.confirm_multi_delete();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while state.multi_delete_progress.is_some() {
            assert!(std::time::Instant::now() < deadline);
            state.poll_multi_delete();
            std::thread::yield_now();
        }

        assert!(!first_path.exists());
        assert_eq!(std::fs::read(&second_path).unwrap(), b"replacement");
        assert_eq!(std::fs::read(&original_second).unwrap(), b"second");
        let tree = state.tree.as_ref().unwrap();
        assert!(tree.get(first).is_none());
        assert!(tree.get(second).is_some());
        assert_eq!(state.session_stats.items_deleted, 1);
        assert_eq!(state.session_stats.bytes_freed, 5);
        assert!(
            state
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("changed"))
        );
    }

    #[test]
    fn multi_delete_uses_four_workers_for_larger_batches() {
        const ITEM_COUNT: usize = MULTI_DELETE_WORKER_LIMIT + 3;

        let temp = TempDir::new().unwrap();
        let mut tree = DiskTree::new(temp.path().to_path_buf());
        let mut node_ids = Vec::new();
        let mut paths = Vec::new();
        for index in 0..ITEM_COUNT {
            let name = format!("item-{index}.txt");
            let path = temp.path().join(&name);
            std::fs::write(&path, b"x").unwrap();
            let node_id = tree.add_node(name, NodeKind::File, path.clone(), NodeId::ROOT);
            tree.set_size(node_id, 1);
            node_ids.push(node_id);
            paths.push(path);
        }

        let mut state = AppState::new(temp.path().to_path_buf());
        state.set_tree(tree);
        state.selected_nodes.extend(node_ids.iter().copied());
        state.request_delete();
        assert_eq!(state.mode, AppMode::ConfirmMultiDelete);

        state.confirm_multi_delete();

        let progress = state.multi_delete_progress.as_ref().unwrap();
        assert_eq!(progress.total, ITEM_COUNT);
        assert_eq!(progress.workers.len(), MULTI_DELETE_WORKER_LIMIT);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while state.multi_delete_progress.is_some() {
            assert!(std::time::Instant::now() < deadline);
            state.poll_multi_delete();
            std::thread::yield_now();
        }

        assert!(paths.iter().all(|path| !path.exists()));
        assert!(
            node_ids
                .iter()
                .all(|node_id| state.tree.as_ref().unwrap().get(*node_id).is_none())
        );
        assert_eq!(state.session_stats.items_deleted, ITEM_COUNT as u32);
        assert_eq!(state.session_stats.bytes_freed, ITEM_COUNT as u64);
    }

    #[test]
    fn changed_artifact_marker_blocks_state_driven_delete() {
        let temp = TempDir::new().unwrap();
        let manifest = temp.path().join("Cargo.toml");
        let original_manifest = temp.path().join("original-Cargo.toml");
        let target = temp.path().join("target");
        std::fs::write(&manifest, b"[package]").unwrap();
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("output"), b"build output").unwrap();

        let mut tree = DiskTree::new(temp.path().to_path_buf());
        tree.add_node(
            "Cargo.toml".to_string(),
            NodeKind::File,
            manifest.clone(),
            NodeId::ROOT,
        );
        let target_id = tree.add_node(
            "target".to_string(),
            NodeKind::Directory,
            target.clone(),
            NodeId::ROOT,
        );
        let mut state = AppState::new(temp.path().to_path_buf());
        state.set_tree(tree);
        state.view_mode = ViewMode::BuildArtifacts;
        assert_eq!(state.computed_views.build_artifacts.len(), 1);

        state.request_delete();
        assert_eq!(state.mode, AppMode::ConfirmDelete);
        std::fs::rename(&manifest, &original_manifest).unwrap();
        std::fs::write(&manifest, b"replacement").unwrap();
        state.confirm_delete();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while state.delete_receiver.is_some() {
            assert!(std::time::Instant::now() < deadline);
            state.poll_delete();
            std::thread::yield_now();
        }

        assert!(target.exists());
        assert!(state.tree.as_ref().unwrap().get(target_id).is_some());
        assert_eq!(state.session_stats.items_deleted, 0);
        assert!(
            state
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("artifact evidence"))
        );
    }

    #[test]
    fn delete_beneath_followed_symlink_is_refused_before_planning() {
        let root = PathBuf::from("/test-root");
        let mut tree = DiskTree::new(root.clone());
        let linked = tree.add_node(
            "linked".to_string(),
            NodeKind::Directory,
            root.join("linked"),
            NodeId::ROOT,
        );
        tree.get_mut(linked).unwrap().path_is_symlink = true;
        tree.add_node(
            "file.txt".to_string(),
            NodeKind::File,
            root.join("linked/file.txt"),
            linked,
        );
        let mut state = AppState::new(root);
        state.set_tree(tree);
        state.view_mode = ViewMode::LargeFiles;

        state.request_delete();

        assert!(state.pending_delete.is_none());
        assert!(state.pending_delete_plan.is_none());
        assert!(
            state
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("beneath a followed symlink"))
        );
    }

    #[test]
    fn disconnected_single_worker_does_not_block_deferred_quit() {
        let (mut state, child) = state_with_child();
        let (result_tx, result_rx) = mpsc::channel::<Result<(NodeId, u64), String>>();
        drop(result_tx);
        state.delete_receiver = Some(result_rx);
        state.delete_worker = Some(DeleteWorker {
            path: PathBuf::from("/test-root/file.txt"),
            handle: std::thread::spawn(|| {}),
        });

        state.quit();
        state.poll_delete();

        assert!(state.should_quit);
        assert!(state.error_message.is_some());
        assert!(
            state
                .tree
                .as_ref()
                .and_then(|tree| tree.get(child))
                .is_some()
        );
    }

    #[test]
    fn quit_waits_for_active_and_queued_multi_delete_items() {
        const ITEM_COUNT: usize = MULTI_DELETE_WORKER_LIMIT + 2;

        let root = PathBuf::from("/test-root");
        let mut tree = DiskTree::new(root.clone());
        let mut tasks = Vec::new();
        for index in 0..ITEM_COUNT {
            let node_id = tree.add_node(
                format!("item-{index}.txt"),
                NodeKind::File,
                root.join(format!("item-{index}.txt")),
                NodeId::ROOT,
            );
            tree.set_size(node_id, 1);
            tasks.push((node_id, 1));
        }
        let mut state = AppState::new(root.clone());
        state.set_tree(tree);

        let (result_tx, result_rx) = mpsc::channel();
        let (release_tx, release_rx) = crossbeam_channel::bounded(ITEM_COUNT);
        let workers = spawn_bounded_workers(tasks, MULTI_DELETE_WORKER_LIMIT, move |task| {
            release_rx.recv().unwrap();
            result_tx
                .send(MultiDeleteResult::Success {
                    node_id: task.0,
                    size: task.1,
                })
                .unwrap();
        })
        .unwrap()
        .into_iter()
        .map(|handle| DeleteWorker {
            path: root.clone(),
            handle,
        })
        .collect();
        state.multi_delete_progress = Some(MultiDeleteProgress {
            total: ITEM_COUNT,
            completed: 0,
            bytes_freed: 0,
            failures: Vec::new(),
            receiver: result_rx,
            workers,
        });
        state.mode = AppMode::MultiDeleting;

        state.quit();
        for _ in 0..MULTI_DELETE_WORKER_LIMIT {
            release_tx.send(()).unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while state
            .multi_delete_progress
            .as_ref()
            .is_some_and(|progress| progress.completed < MULTI_DELETE_WORKER_LIMIT)
        {
            assert!(std::time::Instant::now() < deadline);
            state.poll_multi_delete();
            std::thread::yield_now();
        }
        assert!(!state.should_quit);

        for _ in MULTI_DELETE_WORKER_LIMIT..ITEM_COUNT {
            release_tx.send(()).unwrap();
        }
        while !state.should_quit {
            assert!(std::time::Instant::now() < deadline);
            state.poll_multi_delete();
            std::thread::yield_now();
        }

        let tree = state.tree.as_ref().unwrap();
        assert!(
            (1..=ITEM_COUNT)
                .map(NodeId)
                .all(|node_id| tree.get(node_id).is_none())
        );
        assert_eq!(state.session_stats.items_deleted, ITEM_COUNT as u32);
        assert_eq!(state.session_stats.bytes_freed, ITEM_COUNT as u64);
    }

    #[test]
    fn missing_multi_delete_result_does_not_wedge_quit() {
        let (mut state, child) = state_with_child();
        let (result_tx, result_rx) = mpsc::channel();
        drop(result_tx);
        state.multi_delete_progress = Some(MultiDeleteProgress {
            total: 1,
            completed: 0,
            bytes_freed: 0,
            failures: Vec::new(),
            receiver: result_rx,
            workers: vec![DeleteWorker {
                path: PathBuf::from("/test-root/file.txt"),
                handle: std::thread::spawn(|| {}),
            }],
        });
        state.mode = AppMode::MultiDeleting;

        state.quit();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !state.should_quit {
            assert!(std::time::Instant::now() < deadline);
            state.poll_multi_delete();
            std::thread::yield_now();
        }

        assert!(state.error_message.is_some());
        assert!(state.tree.as_ref().unwrap().get(child).is_some());
        assert!(state.multi_delete_progress.is_none());
    }

    #[test]
    fn panicked_multi_delete_worker_does_not_wedge_quit() {
        let (mut state, child) = state_with_child();
        let (_result_tx, result_rx) = mpsc::channel();
        state.multi_delete_progress = Some(MultiDeleteProgress {
            total: 1,
            completed: 0,
            bytes_freed: 0,
            failures: Vec::new(),
            receiver: result_rx,
            workers: vec![DeleteWorker {
                path: PathBuf::from("/test-root/file.txt"),
                handle: std::thread::spawn(|| panic!("intentional delete worker panic")),
            }],
        });
        state.mode = AppMode::MultiDeleting;

        state.quit();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !state.should_quit {
            assert!(std::time::Instant::now() < deadline);
            state.poll_multi_delete();
            std::thread::yield_now();
        }

        assert!(
            state
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("delete worker panicked"))
        );
        assert!(state.tree.as_ref().unwrap().get(child).is_some());
    }
}

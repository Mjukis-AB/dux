mod app;
mod tui;
mod ui;

use std::io::{self, stdout};
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::SystemTime;

use clap::Parser;
use color_eyre::Result;
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use dux_core::{
    CacheMetadata, CachedScanConfig, CancellationToken, DiskTree, ScanConfig, ScanMessage, Scanner,
    cache_path_for, get_mtime, is_cache_valid, load_cache, save_cache, spot_check_mtimes,
};
use ratatui::{Terminal, backend::CrosstermBackend, style::Style, widgets::Widget};

use app::{Action, AppMode, AppState, ViewMode};
use tui::{AppEvent, EventHandler, handle_key};
use ui::{
    AppLayout, BuildArtifactsView, ConfirmDeleteView, ConfirmMultiDeleteView, Footer, Header,
    HelpView, LargeFilesView, MultiDeleteProgressView, ProgressView, Theme, TreeView,
};

/// DUX - Interactive Terminal Disk Usage Analyzer
#[derive(Parser, Debug)]
#[command(name = "dux")]
#[command(about = "An interactive, DaisyDisk-like terminal disk usage analyzer")]
#[command(version)]
struct Args {
    /// Path to analyze (defaults to current directory)
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Maximum depth to scan
    #[arg(short, long)]
    max_depth: Option<usize>,

    /// Follow symbolic links
    #[arg(short, long)]
    follow_symlinks: bool,

    /// Cross filesystem boundaries
    #[arg(short = 'x', long)]
    cross_filesystems: bool,

    /// Disable cache (always perform fresh scan)
    #[arg(long)]
    no_cache: bool,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();

    // Resolve path
    let path = args
        .path
        .clone()
        .canonicalize()
        .unwrap_or(args.path.clone());

    // Validate path
    if !path.exists() {
        eprintln!("Error: Path does not exist: {}", path.display());
        std::process::exit(1);
    }
    if !path.is_dir() {
        eprintln!("Error: Path is not a directory: {}", path.display());
        std::process::exit(1);
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Run app
    let result = run_app(&mut terminal, path, &args);

    // Restore terminal
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    path: PathBuf,
    args: &Args,
) -> Result<()> {
    let theme = Theme::default();
    let mut state = AppState::new(path.clone());
    let event_handler = EventHandler::new(50); // 50ms tick rate

    // Scan configuration
    let scan_config = ScanConfig {
        follow_symlinks: args.follow_symlinks,
        max_depth: args.max_depth,
        same_filesystem: !args.cross_filesystems,
        num_threads: 0,
    };

    // Cache configuration (for validation)
    let cache_config = CachedScanConfig {
        follow_symlinks: args.follow_symlinks,
        same_filesystem: !args.cross_filesystems,
        max_depth: args.max_depth,
    };

    // Try to load from cache
    let cache_dir = dirs::cache_dir().map(|d| d.join("dux"));
    let cache_path = cache_dir.as_ref().map(|d| cache_path_for(&path, d));
    let mut loaded_from_cache = false;

    if !args.no_cache
        && let Some(ref cp) = cache_path
        && let Ok((meta, tree)) = load_cache(cp)
        && is_cache_valid(&meta, &path, &cache_config)
        && spot_check_mtimes(&tree, 32)
    {
        state.set_cached_tree(tree, meta.scan_time);
        loaded_from_cache = true;
    }

    // Start scanner only if not loaded from cache
    let cancel_token = CancellationToken::new();
    let (progress_rx, scan_handle) = if !loaded_from_cache {
        let scanner = Scanner::new(scan_config.clone()).with_cancellation(cancel_token.clone());
        let (rx, handle) = scanner.scan(path.clone());
        (Some(rx), Some(handle))
    } else {
        (None, None)
    };

    // Store the join handle in an Option so we can take it once
    let mut progress_rx = progress_rx;
    let mut scan_handle: Option<JoinHandle<DiskTree>> = scan_handle;
    let mut cache_save_handle: Option<JoinHandle<dux_core::Result<()>>> = None;

    // For cache saving after scan
    let cache_path_for_save = cache_path.clone();
    let cache_config_for_save = cache_config.clone();
    let root_path_for_save = path.clone();

    loop {
        // Check for scan progress/completion (only if scanning)
        let mut scan_completed = false;
        let mut scan_cancelled = false;
        let mut scan_disconnected = false;
        if let Some(ref rx) = progress_rx {
            loop {
                match rx.try_recv() {
                    Ok(ScanMessage::Progress(progress)) => {
                        state.update_progress(progress);
                    }
                    Ok(ScanMessage::Finalizing) => {
                        state.set_finalizing();
                    }
                    Ok(ScanMessage::Completed) => {
                        scan_completed = true;
                        break;
                    }
                    Ok(ScanMessage::Cancelled) => {
                        scan_cancelled = true;
                        state.quit();
                        break;
                    }
                    Ok(ScanMessage::Error(error)) => {
                        state.set_error(error);
                    }
                    Ok(_) => {}
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        scan_disconnected = true;
                        break;
                    }
                }
            }
        }

        if scan_completed {
            progress_rx = None;
            let handle = scan_handle.take().ok_or_else(|| {
                color_eyre::eyre::eyre!("scanner completed without a worker handle")
            })?;
            match handle.join() {
                Ok(tree) => {
                    // Never let an older background writer rename over this newer scan.
                    join_cache_save(cache_save_handle.take())?;
                    let scan_time = SystemTime::now();
                    if let Some(ref cp) = cache_path_for_save {
                        let tree_for_cache = tree.clone();
                        let cache_path = cp.clone();
                        let meta = cache_metadata_for_tree(
                            &tree_for_cache,
                            root_path_for_save.clone(),
                            cache_config_for_save.clone(),
                            scan_time,
                        );
                        cache_save_handle = Some(std::thread::spawn(move || {
                            save_cache(&cache_path, &tree_for_cache, &meta)
                        }));
                    }
                    state.set_scanned_tree(tree, scan_time);
                }
                Err(_) => recover_from_scan_failure(
                    &mut state,
                    "Scanner worker panicked after reporting completion".to_string(),
                )?,
            }
        } else if scan_cancelled {
            progress_rx = None;
            if let Some(handle) = scan_handle.take() {
                let _ = handle.join();
            }
        } else if scan_disconnected {
            progress_rx = None;
            let join_result = scan_handle.take().map(JoinHandle::join);
            let message = match join_result {
                Some(Err(_)) => "Scanner worker panicked before completing".to_string(),
                Some(Ok(_)) => "Scanner stopped before reporting completion".to_string(),
                None => "Scanner channel disconnected without a worker handle".to_string(),
            };
            recover_from_scan_failure(&mut state, message)?;
        }

        // Draw UI
        terminal.draw(|frame| {
            let area = frame.area();
            let layout = AppLayout::new(area);

            // Background
            frame
                .buffer_mut()
                .set_style(area, Style::default().bg(theme.bg));

            // Update visible height for scrolling
            state.visible_height = layout.tree.height as usize;

            // Header
            Header::new(&state, &theme).render(layout.header, frame.buffer_mut());

            // Size bar (total)
            render_size_bar(&state, &theme, layout.size_bar, frame.buffer_mut());

            // Main content
            match state.mode {
                AppMode::Scanning | AppMode::Finalizing => {
                    ProgressView::new(
                        &state.progress,
                        state.spinner_frame,
                        state.mode == AppMode::Finalizing,
                        &theme,
                    )
                    .render(layout.tree, frame.buffer_mut());
                }
                AppMode::Browsing
                | AppMode::Help
                | AppMode::ConfirmDelete
                | AppMode::ConfirmMultiDelete
                | AppMode::MultiDeleting => {
                    state.ensure_views_computed();

                    match state.view_mode {
                        ViewMode::Tree => {
                            if let Some(tree) = &state.tree {
                                TreeView::new(
                                    tree,
                                    state.view_root,
                                    state.selected_index,
                                    state.scroll_offset,
                                    &state.selected_nodes,
                                    &theme,
                                )
                                .render(layout.tree, frame.buffer_mut());
                            }
                        }
                        ViewMode::LargeFiles => {
                            LargeFilesView::new(
                                &state.computed_views.large_files,
                                state.large_files_state.selected_index,
                                state.large_files_state.scroll_offset,
                                &state.selected_nodes,
                                &theme,
                            )
                            .render(layout.tree, frame.buffer_mut());
                        }
                        ViewMode::BuildArtifacts => {
                            BuildArtifactsView::new(
                                &state.computed_views.build_artifacts,
                                state.build_artifacts_state.selected_index,
                                state.build_artifacts_state.scroll_offset,
                                state.computed_views.stale_threshold,
                                &state.selected_nodes,
                                &theme,
                            )
                            .render(layout.tree, frame.buffer_mut());
                        }
                    }

                    // Help overlay
                    if state.mode == AppMode::Help {
                        HelpView::new(&theme).render(area, frame.buffer_mut());
                    }

                    // Multi-delete confirmation dialog (check before single)
                    if state.mode == AppMode::ConfirmMultiDelete
                        && let Some(ref items) = state.pending_multi_delete
                    {
                        ConfirmMultiDeleteView::new(items, &theme).render(area, frame.buffer_mut());
                    }

                    // Single delete confirmation dialog
                    if state.mode == AppMode::ConfirmDelete
                        && let Some(path) = state.pending_delete_path()
                    {
                        let size = state.pending_delete_size();
                        ConfirmDeleteView::new(path, size, &theme).render(area, frame.buffer_mut());
                    }

                    // Multi-delete progress overlay
                    if state.mode == AppMode::MultiDeleting
                        && let Some(ref progress) = state.multi_delete_progress
                    {
                        MultiDeleteProgressView::new(progress, state.quit_requested, &theme)
                            .render(area, frame.buffer_mut());
                    }
                }
            }

            // Compute selection size for footer
            let selection_size = state.selection_total_size();

            // Footer
            Footer::new(state.mode, state.view_mode, &theme, &state.session_stats)
                .with_stale_threshold(state.computed_views.stale_threshold)
                .with_quit_requested(state.quit_requested)
                .with_selection(
                    state.selection_count(),
                    selection_size,
                    state.selecting_mode,
                )
                .render(layout.footer, frame.buffer_mut());
        })?;

        // Poll for async delete completion
        state.poll_delete();
        state.poll_multi_delete();

        // Handle events
        match event_handler.next()? {
            AppEvent::Key(key) => {
                let action = handle_key(
                    key,
                    state.mode,
                    state.selection_count() > 0,
                    state.selecting_mode,
                );
                if action == Action::Rescan {
                    // A completed older scan must finish writing before a newer scan starts.
                    join_cache_save(cache_save_handle.take())?;
                    if state.prepare_rescan() {
                        let scanner = Scanner::new(scan_config.clone())
                            .with_cancellation(cancel_token.clone());
                        let (rx, handle) = scanner.scan(path.clone());
                        progress_rx = Some(rx);
                        scan_handle = Some(handle);
                    }
                } else {
                    handle_action(&mut state, action);
                }
            }
            AppEvent::Resize(_, _) => {
                // Terminal will redraw on next loop
            }
            AppEvent::Tick => {
                state.tick_spinner();
            }
            _ => {}
        }

        if state.should_quit {
            cancel_token.cancel();
            break;
        }
    }

    // Ensure the initial post-scan snapshot cannot overwrite a newer tree
    // after deletion results have been applied.
    join_cache_save(cache_save_handle.take())?;

    // Save cache if tree was modified (e.g. deletions)
    if state.tree_modified
        && let Some(ref tree) = state.tree
        && let Some(ref cp) = cache_path_for_save
    {
        let meta = cache_metadata_for_tree(
            tree,
            root_path_for_save.clone(),
            cache_config_for_save.clone(),
            state.scan_time().unwrap_or_else(SystemTime::now),
        );
        save_cache(cp, tree, &meta)?;
    }

    Ok(())
}

fn cache_metadata_for_tree(
    tree: &DiskTree,
    root_path: PathBuf,
    config: CachedScanConfig,
    scan_time: SystemTime,
) -> CacheMetadata {
    let root_mtime = get_mtime(&root_path).unwrap_or(SystemTime::UNIX_EPOCH);
    CacheMetadata {
        version: dux_core::CACHE_VERSION,
        root_path,
        scan_time,
        root_mtime,
        total_size: tree.total_size(),
        node_count: tree.live_count(),
        config,
    }
}

fn recover_from_scan_failure(state: &mut AppState, message: String) -> Result<()> {
    if state.tree.is_some() {
        state.mode = AppMode::Browsing;
        state.set_error(format!("{message}; keeping the previous scan"));
        Ok(())
    } else {
        Err(color_eyre::eyre::eyre!(message))
    }
}

fn join_cache_save(handle: Option<JoinHandle<dux_core::Result<()>>>) -> Result<()> {
    if let Some(handle) = handle {
        handle
            .join()
            .map_err(|_| color_eyre::eyre::eyre!("background cache writer panicked"))??;
    }
    Ok(())
}

fn handle_action(state: &mut AppState, action: Action) {
    match action {
        Action::MoveUp => state.move_up(),
        Action::MoveDown => state.move_down(),
        Action::PageUp => state.page_up(),
        Action::PageDown => state.page_down(),
        Action::GoToFirst => state.go_to_first(),
        Action::GoToLast => state.go_to_last(),
        // Selection actions
        Action::SelectUp => state.select_move_up(),
        Action::SelectDown => state.select_move_down(),
        Action::SelectPageUp => state.select_page_up(),
        Action::SelectPageDown => state.select_page_down(),
        Action::SelectToFirst => state.select_to_first(),
        Action::SelectToLast => state.select_to_last(),
        Action::ToggleSelect => state.toggle_select(),
        Action::ClearSelection => state.clear_selection(),
        // Tree-specific actions: only apply in Tree view
        Action::Expand => {
            if state.view_mode == ViewMode::Tree {
                state.expand_selected();
            }
        }
        Action::Collapse => {
            if state.view_mode == ViewMode::Tree {
                state.collapse_selected();
            }
        }
        Action::Toggle => {
            if state.view_mode == ViewMode::Tree {
                state.toggle_selected();
            }
        }
        Action::DrillDown => {
            if state.view_mode == ViewMode::Tree {
                state.drill_down();
            }
        }
        Action::GoBack => {
            if state.view_mode == ViewMode::Tree {
                state.go_back();
            }
        }
        Action::NextView => state.next_view(),
        Action::PrevView => state.prev_view(),
        Action::CycleStaleThreshold => {
            if state.view_mode == ViewMode::BuildArtifacts {
                state.computed_views.cycle_stale_threshold();
            }
        }
        Action::Rescan => {}
        Action::ShowHelp => state.show_help(),
        Action::HideHelp => state.hide_help(),
        Action::OpenInFinder => state.open_in_finder(),
        Action::Delete => state.request_delete(),
        Action::ConfirmDelete => state.confirm_delete(),
        Action::CancelDelete => state.cancel_delete(),
        Action::ConfirmMultiDelete => state.confirm_multi_delete(),
        Action::CancelMultiDelete => state.cancel_multi_delete(),
        Action::Quit => state.quit(),
        Action::Tick => {}
    }
}

fn render_size_bar(
    state: &AppState,
    theme: &Theme,
    area: ratatui::layout::Rect,
    buf: &mut ratatui::buffer::Buffer,
) {
    use ratatui::style::Style;

    if area.width < 10 {
        return;
    }

    let is_scanning = matches!(state.mode, AppMode::Scanning | AppMode::Finalizing);
    let total_size = if is_scanning {
        state.progress.bytes_scanned
    } else {
        state
            .tree
            .as_ref()
            .map(|tree| tree.total_size())
            .unwrap_or(0)
    };
    let bar_width = area.width.saturating_sub(20) as usize;

    // During scanning, show a pulsing/growing bar; after complete, show full bar
    let (bar, _) = ui::bar_chart::render_bar(100.0, bar_width, theme.green);

    // Bar
    buf.set_string(area.x + 1, area.y, &bar, Style::default().fg(theme.green));

    // Label - don't show "100%" during scanning since we don't know the final total
    let label = if is_scanning {
        format!("{} scanned", dux_core::format_size(total_size))
    } else {
        format!("{} total", dux_core::format_size(total_size))
    };
    let label = ui::text::truncate_start(&label, area.width.saturating_sub(2) as usize);
    buf.set_string(
        area.x
            + area
                .width
                .saturating_sub(ui::text::char_count(&label) as u16 + 1),
        area.y,
        &label,
        Style::default().fg(theme.fg_dim),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use dux_core::{NodeId, NodeKind};

    fn cache_metadata(root: PathBuf, tree: &DiskTree) -> CacheMetadata {
        CacheMetadata {
            version: dux_core::CACHE_VERSION,
            root_path: root,
            scan_time: SystemTime::now(),
            root_mtime: SystemTime::now(),
            total_size: tree.total_size(),
            node_count: tree.live_count(),
            config: CachedScanConfig {
                follow_symlinks: false,
                same_filesystem: true,
                max_depth: None,
            },
        }
    }

    #[test]
    fn post_delete_cache_write_follows_initial_writer() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("root");
        let cache_path = temp.path().join("scan.dux");

        let mut initial_tree = DiskTree::new(root.clone());
        initial_tree.add_node(
            "deleted.txt".to_string(),
            NodeKind::File,
            root.join("deleted.txt"),
            NodeId::ROOT,
        );
        let initial_meta = cache_metadata(root.clone(), &initial_tree);
        let final_tree = DiskTree::new(root.clone());
        let final_meta = cache_metadata(root.clone(), &final_tree);

        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let initial_path = cache_path.clone();
        let initial_writer = std::thread::spawn(move || {
            release_rx.recv().unwrap();
            save_cache(&initial_path, &initial_tree, &initial_meta)
        });

        let (finalizer_started_tx, finalizer_started_rx) = std::sync::mpsc::channel();
        let final_path = cache_path.clone();
        let finalizer = std::thread::spawn(move || {
            finalizer_started_tx.send(()).unwrap();
            join_cache_save(Some(initial_writer)).unwrap();
            save_cache(&final_path, &final_tree, &final_meta).unwrap();
        });

        finalizer_started_rx.recv().unwrap();
        assert!(!cache_path.exists());
        release_tx.send(()).unwrap();
        finalizer.join().unwrap();

        let (_, loaded_tree) = load_cache(&cache_path).unwrap();
        assert_eq!(loaded_tree.live_count(), 1);
        assert!(
            loaded_tree
                .find_by_path(&root.join("deleted.txt"))
                .is_none()
        );
    }

    #[test]
    fn cache_update_preserves_the_tree_original_scan_time() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().to_path_buf();
        let cache_path = temp.path().join("scan.dux");
        let tree = DiskTree::new(root.clone());
        let original_scan_time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(12_345);
        let config = CachedScanConfig {
            follow_symlinks: false,
            same_filesystem: true,
            max_depth: None,
        };

        let meta = cache_metadata_for_tree(&tree, root, config, original_scan_time);
        save_cache(&cache_path, &tree, &meta).unwrap();

        let (loaded_meta, _) = load_cache(&cache_path).unwrap();
        assert_eq!(loaded_meta.scan_time, original_scan_time);
    }

    #[test]
    fn rescan_size_bar_uses_progress_instead_of_retained_tree_size() {
        let mut tree = DiskTree::new(PathBuf::from("/scan"));
        tree.set_size(NodeId::ROOT, 1024 * 1024);
        let mut state = AppState::new(PathBuf::from("/scan"));
        state.set_cached_tree(tree, SystemTime::UNIX_EPOCH);
        assert!(state.prepare_rescan());
        state.progress.bytes_scanned = 2 * 1024;
        let area = ratatui::layout::Rect::new(0, 0, 80, 1);
        let mut buffer = ratatui::buffer::Buffer::empty(area);

        render_size_bar(&state, &Theme::default(), area, &mut buffer);

        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("2.0 KB scanned"), "{text}");
        assert!(!text.contains("total"), "{text}");
    }

    #[test]
    fn failed_rescan_restores_browsing_with_the_previous_tree() {
        let mut state = AppState::new(PathBuf::from("/scan"));
        state.set_cached_tree(
            DiskTree::new(PathBuf::from("/scan")),
            SystemTime::UNIX_EPOCH,
        );
        assert!(state.prepare_rescan());

        recover_from_scan_failure(&mut state, "injected scan failure".to_string()).unwrap();

        assert_eq!(state.mode, AppMode::Browsing);
        assert!(state.tree.is_some());
        assert!(state.loaded_from_cache);
        assert_eq!(state.scan_time(), Some(SystemTime::UNIX_EPOCH));
        assert!(
            state
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("keeping the previous scan"))
        );
    }
}

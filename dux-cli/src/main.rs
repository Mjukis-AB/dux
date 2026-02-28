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

use app::{Action, AppMode, AppState};
use tui::{AppEvent, EventHandler, handle_key};
use ui::{AppLayout, ConfirmDeleteView, Footer, Header, HelpView, ProgressView, Theme, TreeView};

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
        state.set_tree(tree);
        state.loaded_from_cache = true;
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
    let mut scan_handle: Option<JoinHandle<DiskTree>> = scan_handle;

    // For cache saving after scan
    let cache_path_for_save = cache_path.clone();
    let cache_config_for_save = cache_config.clone();
    let root_path_for_save = path.clone();

    loop {
        // Check for scan progress/completion (only if scanning)
        if let Some(ref rx) = progress_rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    ScanMessage::Progress(progress) => {
                        state.update_progress(progress);
                    }
                    ScanMessage::Finalizing => {
                        state.set_finalizing();
                    }
                    ScanMessage::Completed => {
                        // Scanner completed, get the tree
                        if let Some(handle) = scan_handle.take()
                            && let Ok(tree) = handle.join()
                        {
                            // Save to cache in background
                            if let Some(ref cp) = cache_path_for_save {
                                let tree_for_cache = tree.clone();
                                let cache_path = cp.clone();
                                let config = cache_config_for_save.clone();
                                let root = root_path_for_save.clone();
                                let root_mtime = get_mtime(&root).unwrap_or(SystemTime::UNIX_EPOCH);
                                std::thread::spawn(move || {
                                    let meta = CacheMetadata {
                                        version: dux_core::CACHE_VERSION,
                                        root_path: root,
                                        scan_time: SystemTime::now(),
                                        root_mtime,
                                        total_size: tree_for_cache.total_size(),
                                        node_count: tree_for_cache.live_count(),
                                        config,
                                    };
                                    let _ = save_cache(&cache_path, &tree_for_cache, &meta);
                                });
                            }
                            state.set_tree(tree);
                        }
                        break;
                    }
                    ScanMessage::Cancelled => {
                        state.quit();
                    }
                    ScanMessage::Error(e) => {
                        state.set_error(e);
                    }
                    _ => {}
                }
            }
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
                AppMode::Browsing | AppMode::Help | AppMode::ConfirmDelete => {
                    if let Some(tree) = &state.tree {
                        TreeView::new(
                            tree,
                            state.view_root,
                            state.selected_index,
                            state.scroll_offset,
                            &theme,
                        )
                        .render(layout.tree, frame.buffer_mut());
                    }

                    // Help overlay
                    if state.mode == AppMode::Help {
                        HelpView::new(&theme).render(area, frame.buffer_mut());
                    }

                    // Delete confirmation dialog
                    if state.mode == AppMode::ConfirmDelete
                        && let Some(path) = state.pending_delete_path()
                    {
                        let size = state.pending_delete_size();
                        ConfirmDeleteView::new(path, size, &theme).render(area, frame.buffer_mut());
                    }
                }
            }

            // Footer
            Footer::new(state.mode, &theme, &state.session_stats)
                .render(layout.footer, frame.buffer_mut());
        })?;

        // Poll for async delete completion
        state.poll_delete();

        // Handle events
        match event_handler.next()? {
            AppEvent::Key(key) => {
                let action = handle_key(key, state.mode);
                handle_action(&mut state, action);
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

    // Save cache if tree was modified (e.g. deletions)
    if state.tree_modified
        && let Some(ref tree) = state.tree
        && let Some(ref cp) = cache_path_for_save
    {
        let root_mtime = get_mtime(&root_path_for_save).unwrap_or(SystemTime::UNIX_EPOCH);
        let meta = CacheMetadata {
            version: dux_core::CACHE_VERSION,
            root_path: root_path_for_save.clone(),
            scan_time: SystemTime::now(),
            root_mtime,
            total_size: tree.total_size(),
            node_count: tree.live_count(),
            config: cache_config_for_save.clone(),
        };
        let _ = save_cache(cp, tree, &meta);
    }

    // Drop tree in background to avoid blocking on deallocation
    if let Some(tree) = state.tree.take() {
        std::thread::spawn(move || drop(tree));
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
        Action::Expand => state.expand_selected(),
        Action::Collapse => state.collapse_selected(),
        Action::Toggle => state.toggle_selected(),
        Action::DrillDown => state.drill_down(),
        Action::GoBack => state.go_back(),
        Action::ShowHelp => state.show_help(),
        Action::HideHelp => state.hide_help(),
        Action::OpenInFinder => state.open_in_finder(),
        Action::Delete => state.request_delete(),
        Action::ConfirmDelete => state.confirm_delete(),
        Action::CancelDelete => state.cancel_delete(),
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

    let total_size = state
        .tree
        .as_ref()
        .map(|t| t.total_size())
        .unwrap_or(state.progress.bytes_scanned);

    let is_scanning = state.tree.is_none();
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
    buf.set_string(
        area.x + area.width - label.len() as u16 - 1,
        area.y,
        &label,
        Style::default().fg(theme.fg_dim),
    );
}

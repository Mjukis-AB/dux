use dux_core::{ScanCoverage, ScanCoverageStatus};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};
use std::time::SystemTime;

use crate::app::{AppState, ViewMode};

use super::progress::progress_indicator;
use super::text::{char_count, truncate_start};
use super::theme::Theme;

/// Header widget showing title, path, and status
pub struct Header<'a> {
    state: &'a AppState,
    theme: &'a Theme,
}

impl<'a> Header<'a> {
    pub fn new(state: &'a AppState, theme: &'a Theme) -> Self {
        Self { state, theme }
    }
}

impl Widget for Header<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 1 {
            return;
        }

        // Title
        let title = "DUX";
        let title_style = Style::default()
            .fg(self.theme.blue)
            .add_modifier(Modifier::BOLD);
        buf.set_string(area.x + 1, area.y, title, title_style);

        let mut content_x = area.x + 5;

        // View mode indicator (only for non-Tree views)
        let view_label = match self.state.view_mode {
            ViewMode::Tree => None,
            ViewMode::LargeFiles => Some("Large Files"),
            ViewMode::BuildArtifacts => Some("Build Artifacts"),
        };

        if let Some(label) = view_label {
            // Separator
            buf.set_string(
                content_x,
                area.y,
                "─",
                Style::default().fg(self.theme.border),
            );
            content_x += 2;

            let view_style = Style::default()
                .fg(self.theme.blue)
                .add_modifier(Modifier::BOLD);
            buf.set_string(content_x, area.y, label, view_style);
            content_x += label.len() as u16 + 1;
        }

        // Separator
        buf.set_string(
            content_x,
            area.y,
            "─",
            Style::default().fg(self.theme.border),
        );
        content_x += 2;

        let is_scanning = matches!(
            self.state.mode,
            crate::app::AppMode::Scanning | crate::app::AppMode::Finalizing
        );
        let status = if is_scanning {
            progress_indicator(&self.state.progress, self.state.spinner_frame)
        } else if let Some(tree) = &self.state.tree {
            let cached_indicator = if self.state.loaded_from_cache {
                self.state
                    .scan_time()
                    .map(|scan_time| {
                        format!(
                            " (cached {} ago)",
                            format_cache_age(scan_time, SystemTime::now())
                        )
                    })
                    .unwrap_or_else(|| " (cached)".to_string())
            } else {
                String::new()
            };
            format!(
                "{} files, {}, {}{}",
                dux_core::format_count(tree.total_files()),
                dux_core::format_size(tree.total_size()),
                format_coverage(self.state.scan_coverage()),
                cached_indicator
            )
        } else {
            String::new()
        };
        let status = truncate_start(&status, area.width.saturating_sub(2) as usize);

        // Path/breadcrumbs
        let path = match self.state.view_mode {
            ViewMode::Tree => {
                if let Some(tree) = &self.state.tree {
                    tree.breadcrumbs(self.state.view_root)
                } else {
                    self.state.root_path.to_string_lossy().to_string()
                }
            }
            ViewMode::LargeFiles | ViewMode::BuildArtifacts => {
                self.state.root_path.to_string_lossy().to_string()
            }
        };

        let reserved_width = content_x
            .saturating_sub(area.x)
            .saturating_add(char_count(&status) as u16 + 3);
        let max_path_len = area.width.saturating_sub(reserved_width) as usize;
        let display_path = truncate_start(&path, max_path_len);

        buf.set_string(
            content_x,
            area.y,
            &display_path,
            Style::default().fg(self.theme.fg),
        );

        // Status (right-aligned)
        let status_x = area.x + area.width.saturating_sub(char_count(&status) as u16 + 2);
        let status_style = if is_scanning
            || matches!(
                self.state.scan_coverage().status(),
                ScanCoverageStatus::LimitedAccess | ScanCoverageStatus::Partial
            ) {
            Style::default().fg(self.theme.yellow)
        } else {
            Style::default().fg(self.theme.fg_dim)
        };
        buf.set_string(status_x, area.y, &status, status_style);
    }
}

fn format_coverage(coverage: &ScanCoverage) -> String {
    let occurrences = coverage
        .issues()
        .iter()
        .map(|issue| u64::from(issue.occurrence_count()))
        .sum::<u64>();
    match coverage.status() {
        ScanCoverageStatus::Unknown => "coverage unknown".to_owned(),
        ScanCoverageStatus::Complete => "complete coverage".to_owned(),
        ScanCoverageStatus::LimitedAccess => {
            format!(
                "limited access · {occurrences} issue{}",
                plural(occurrences)
            )
        }
        ScanCoverageStatus::Partial => {
            format!("partial · {occurrences} issue{}", plural(occurrences))
        }
    }
}

fn plural(count: u64) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn format_cache_age(scan_time: SystemTime, now: SystemTime) -> String {
    let seconds = now.duration_since(scan_time).unwrap_or_default().as_secs();
    match seconds {
        0..=59 => "<1m".to_string(),
        60..=3_599 => format!("{}m", seconds / 60),
        3_600..=86_399 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use dux_core::DiskTree;

    #[test]
    fn cache_age_uses_compact_stable_units() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10 * 86_400);

        assert_eq!(format_cache_age(now - Duration::from_secs(30), now), "<1m");
        assert_eq!(
            format_cache_age(now - Duration::from_secs(15 * 60), now),
            "15m"
        );
        assert_eq!(
            format_cache_age(now - Duration::from_secs(3 * 3_600), now),
            "3h"
        );
        assert_eq!(
            format_cache_age(now - Duration::from_secs(2 * 86_400), now),
            "2d"
        );
        assert_eq!(format_cache_age(now + Duration::from_secs(60), now), "<1m");
    }

    #[test]
    fn cached_header_shows_original_scan_age() {
        let mut state = AppState::new("/scan".into());
        state.set_cached_tree(
            DiskTree::new("/scan".into()),
            SystemTime::now() - Duration::from_secs(2 * 3_600),
        );
        let area = Rect::new(0, 0, 120, 1);
        let mut buffer = Buffer::empty(area);

        Header::new(&state, &Theme::default()).render(area, &mut buffer);

        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("cached 2h ago"), "{text}");
        assert!(text.contains("coverage unknown"), "{text}");
    }

    #[test]
    fn rescan_progress_takes_precedence_over_retained_cached_tree() {
        let mut state = AppState::new("/scan".into());
        state.set_cached_tree(DiskTree::new("/scan".into()), SystemTime::UNIX_EPOCH);
        assert!(state.prepare_rescan());
        let area = Rect::new(0, 0, 120, 1);
        let mut buffer = Buffer::empty(area);

        Header::new(&state, &Theme::default()).render(area, &mut buffer);

        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!text.contains("cached"), "{text}");
        assert!(text.contains("0 files, 0 B"), "{text}");
    }

    #[test]
    fn fresh_scan_header_shows_complete_or_partial_coverage() {
        use dux_core::{ScanConfig, Scanner};

        let complete_root = tempfile::TempDir::new().unwrap();
        let scanner = Scanner::new(ScanConfig::default());
        let (rx, handle) = scanner.scan(complete_root.path().to_path_buf());
        for _ in rx {}
        let (tree, coverage, _) = handle.join().unwrap().into_parts();
        let mut complete = AppState::new(complete_root.path().to_path_buf());
        complete.set_scanned_tree(tree, SystemTime::now(), coverage);

        let partial_root = tempfile::TempDir::new().unwrap();
        std::fs::write(partial_root.path().join("hidden"), b"payload").unwrap();
        let scanner = Scanner::new(ScanConfig {
            max_depth: Some(0),
            ..ScanConfig::default()
        });
        let (rx, handle) = scanner.scan(partial_root.path().to_path_buf());
        for _ in rx {}
        let (tree, coverage, _) = handle.join().unwrap().into_parts();
        let mut partial = AppState::new(partial_root.path().to_path_buf());
        partial.set_scanned_tree(tree, SystemTime::now(), coverage);

        let render = |state: &AppState| {
            let area = Rect::new(0, 0, 140, 1);
            let mut buffer = Buffer::empty(area);
            Header::new(state, &Theme::default()).render(area, &mut buffer);
            buffer
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
        };
        #[cfg(unix)]
        assert!(render(&complete).contains("complete coverage"));
        #[cfg(not(unix))]
        assert!(render(&complete).contains("partial · 1 issue"));
        #[cfg(unix)]
        assert!(render(&partial).contains("partial · 1 issue"));
        #[cfg(not(unix))]
        assert!(render(&partial).contains("partial · 2 issues"));
    }
}

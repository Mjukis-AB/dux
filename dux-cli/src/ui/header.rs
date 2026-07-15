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
                "{} files, {}{}",
                dux_core::format_count(tree.total_files()),
                dux_core::format_size(tree.total_size()),
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
        let status_style = if is_scanning {
            Style::default().fg(self.theme.yellow)
        } else {
            Style::default().fg(self.theme.fg_dim)
        };
        buf.set_string(status_x, area.y, &status, status_style);
    }
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
}

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};

use crate::app::views::{StaleThreshold, stale_threshold_label};
use crate::app::{AppMode, SessionStats, ViewMode};

use super::text::{char_count, truncate_start};
use super::theme::Theme;

/// Footer widget showing keyboard hints and session stats
pub struct Footer<'a> {
    mode: AppMode,
    view_mode: ViewMode,
    theme: &'a Theme,
    session_stats: &'a SessionStats,
    stale_threshold: Option<StaleThreshold>,
    selection_count: usize,
    selection_size: u64,
    selecting_mode: bool,
    quit_requested: bool,
}

impl<'a> Footer<'a> {
    pub fn new(
        mode: AppMode,
        view_mode: ViewMode,
        theme: &'a Theme,
        session_stats: &'a SessionStats,
    ) -> Self {
        Self {
            mode,
            view_mode,
            theme,
            session_stats,
            stale_threshold: None,
            selection_count: 0,
            selection_size: 0,
            selecting_mode: false,
            quit_requested: false,
        }
    }

    pub fn with_stale_threshold(mut self, threshold: StaleThreshold) -> Self {
        self.stale_threshold = Some(threshold);
        self
    }

    pub fn with_selection(mut self, count: usize, size: u64, selecting: bool) -> Self {
        self.selection_count = count;
        self.selection_size = size;
        self.selecting_mode = selecting;
        self
    }

    pub fn with_quit_requested(mut self, quit_requested: bool) -> Self {
        self.quit_requested = quit_requested;
        self
    }
}

impl Widget for Footer<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 20 || area.height < 1 {
            return;
        }

        let select_hint = if self.selecting_mode {
            ("v/Esc", "Stop select".to_string())
        } else {
            ("v", "Select".to_string())
        };

        let hints: Vec<(&str, String)> = if self.quit_requested {
            vec![("…", "Waiting for deletion before quitting".to_string())]
        } else {
            match self.mode {
                AppMode::Scanning | AppMode::Finalizing => vec![("q", "Quit".to_string())],
                AppMode::Browsing => match self.view_mode {
                    ViewMode::Tree => vec![
                        ("Tab", "Views".to_string()),
                        ("↑↓", "Navigate".to_string()),
                        select_hint.clone(),
                        ("←→", "Collapse/Expand".to_string()),
                        ("d", "Delete".to_string()),
                        ("r", "Rescan".to_string()),
                        ("?", "Help".to_string()),
                        ("q", "Quit".to_string()),
                    ],
                    ViewMode::LargeFiles => vec![
                        ("Tab", "Views".to_string()),
                        ("↑↓", "Navigate".to_string()),
                        select_hint.clone(),
                        ("d", "Delete".to_string()),
                        ("r", "Rescan".to_string()),
                        ("?", "Help".to_string()),
                        ("q", "Quit".to_string()),
                    ],
                    ViewMode::BuildArtifacts => {
                        let stale_label = self
                            .stale_threshold
                            .map(|threshold| format!("Stale:{}", stale_threshold_label(threshold)))
                            .unwrap_or_else(|| "Stale".to_string());
                        vec![
                            ("Tab", "Views".to_string()),
                            ("↑↓", "Navigate".to_string()),
                            select_hint.clone(),
                            ("s", stale_label),
                            ("d", "Delete".to_string()),
                            ("r", "Rescan".to_string()),
                            ("?", "Help".to_string()),
                            ("q", "Quit".to_string()),
                        ]
                    }
                },
                AppMode::Help => vec![("Esc", "Close help".to_string()), ("q", "Quit".to_string())],
                AppMode::ConfirmDelete | AppMode::ConfirmMultiDelete => {
                    vec![("y", "Yes".to_string()), ("n", "Cancel".to_string())]
                }
                AppMode::MultiDeleting => vec![("q", "Quit after deletions".to_string())],
            }
        };

        let key_style = Style::default()
            .fg(self.theme.fg)
            .add_modifier(Modifier::BOLD);
        let desc_style = Style::default().fg(self.theme.fg_dim);
        let sep_style = Style::default().fg(self.theme.border);

        let mut x = area.x + 1;
        for (i, (key, desc)) in hints.iter().enumerate() {
            // Key
            buf.set_string(x, area.y, *key, key_style);
            x += char_count(key) as u16 + 1;

            // Description
            buf.set_string(x, area.y, desc.as_str(), desc_style);
            x += char_count(desc) as u16;

            // Separator
            if i < hints.len() - 1 {
                buf.set_string(x, area.y, "  │  ", sep_style);
                x += 5;
            }

            if x >= area.x + area.width - 5 {
                break;
            }
        }

        // Right side: selection info or freed space
        let right_text = if self.selection_count > 0 {
            Some((
                format!(
                    "{} selected ({})",
                    self.selection_count,
                    dux_core::format_size(self.selection_size),
                ),
                Style::default()
                    .fg(self.theme.purple)
                    .add_modifier(Modifier::BOLD),
            ))
        } else if self.session_stats.items_deleted > 0 {
            Some((
                format!(
                    "Freed: {} ({} item{})",
                    dux_core::format_size(self.session_stats.bytes_freed),
                    self.session_stats.items_deleted,
                    if self.session_stats.items_deleted == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
                Style::default()
                    .fg(self.theme.green)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            None
        };

        if let Some((text, style)) = right_text {
            let text = truncate_start(&text, area.width.saturating_sub(2) as usize);
            let stats_x = area.x + area.width.saturating_sub(char_count(&text) as u16 + 1);
            if stats_x > x + 2 {
                buf.set_string(stats_x, area.y, &text, style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_quit_copy_does_not_claim_background_survival() {
        let area = Rect::new(0, 0, 80, 1);
        let mut buffer = Buffer::empty(area);
        Footer::new(
            AppMode::MultiDeleting,
            ViewMode::Tree,
            &Theme::default(),
            &SessionStats::default(),
        )
        .with_quit_requested(true)
        .render(area, &mut buffer);

        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Waiting for deletion before quitting"));
        assert!(!text.contains("continue"));
    }
}

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};

use crate::app::views::StaleThreshold;
use crate::app::{AppMode, SessionStats, ViewMode};

use super::theme::Theme;

/// Footer widget showing keyboard hints and session stats
pub struct Footer<'a> {
    mode: AppMode,
    view_mode: ViewMode,
    theme: &'a Theme,
    session_stats: &'a SessionStats,
    stale_threshold: Option<StaleThreshold>,
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
        }
    }

    pub fn with_stale_threshold(mut self, threshold: StaleThreshold) -> Self {
        self.stale_threshold = Some(threshold);
        self
    }
}

impl Widget for Footer<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 20 || area.height < 1 {
            return;
        }

        let hints: Vec<(&str, String)> = match self.mode {
            AppMode::Scanning | AppMode::Finalizing => vec![("q", "Quit".to_string())],
            AppMode::Browsing => match self.view_mode {
                ViewMode::Tree => vec![
                    ("Tab", "Views".to_string()),
                    ("↑↓", "Navigate".to_string()),
                    ("←→", "Collapse/Expand".to_string()),
                    ("Enter", "Drill down".to_string()),
                    ("o", "Open".to_string()),
                    ("d", "Delete".to_string()),
                    ("?", "Help".to_string()),
                    ("q", "Quit".to_string()),
                ],
                ViewMode::LargeFiles => vec![
                    ("Tab", "Views".to_string()),
                    ("↑↓", "Navigate".to_string()),
                    ("o", "Open".to_string()),
                    ("d", "Delete".to_string()),
                    ("?", "Help".to_string()),
                    ("q", "Quit".to_string()),
                ],
                ViewMode::BuildArtifacts => {
                    let stale_label = self
                        .stale_threshold
                        .map(|t| format!("Stale:{}", t.label()))
                        .unwrap_or_else(|| "Stale".to_string());
                    vec![
                        ("Tab", "Views".to_string()),
                        ("↑↓", "Navigate".to_string()),
                        ("s", stale_label),
                        ("o", "Open".to_string()),
                        ("d", "Delete".to_string()),
                        ("?", "Help".to_string()),
                        ("q", "Quit".to_string()),
                    ]
                }
            },
            AppMode::Help => vec![("Esc", "Close help".to_string()), ("q", "Quit".to_string())],
            AppMode::ConfirmDelete => {
                vec![("y", "Yes".to_string()), ("n", "Cancel".to_string())]
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
            x += key.len() as u16 + 1;

            // Description
            buf.set_string(x, area.y, desc.as_str(), desc_style);
            x += desc.len() as u16;

            // Separator
            if i < hints.len() - 1 {
                buf.set_string(x, area.y, "  │  ", sep_style);
                x += 5;
            }

            if x >= area.x + area.width - 5 {
                break;
            }
        }

        // Show freed space on the right side (only if items have been deleted)
        if self.session_stats.items_deleted > 0 {
            let freed_text = format!(
                "Freed: {} ({} item{})",
                dux_core::format_size(self.session_stats.bytes_freed),
                self.session_stats.items_deleted,
                if self.session_stats.items_deleted == 1 {
                    ""
                } else {
                    "s"
                }
            );
            let stats_style = Style::default()
                .fg(self.theme.green)
                .add_modifier(Modifier::BOLD);
            let stats_x = area.x + area.width - freed_text.len() as u16 - 1;
            if stats_x > x + 2 {
                buf.set_string(stats_x, area.y, &freed_text, stats_style);
            }
        }
    }
}

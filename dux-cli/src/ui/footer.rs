use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};

use crate::app::{AppMode, SessionStats};

use super::theme::Theme;

/// Footer widget showing keyboard hints and session stats
pub struct Footer<'a> {
    mode: AppMode,
    theme: &'a Theme,
    session_stats: &'a SessionStats,
}

impl<'a> Footer<'a> {
    pub fn new(mode: AppMode, theme: &'a Theme, session_stats: &'a SessionStats) -> Self {
        Self { mode, theme, session_stats }
    }
}

impl Widget for Footer<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 20 || area.height < 1 {
            return;
        }

        let hints = match self.mode {
            AppMode::Scanning | AppMode::Finalizing => vec![("q", "Quit")],
            AppMode::Browsing => vec![
                ("↑↓", "Navigate"),
                ("←→", "Collapse/Expand"),
                ("Enter", "Drill down"),
                ("o", "Open"),
                ("d", "Delete"),
                ("?", "Help"),
                ("q", "Quit"),
            ],
            AppMode::Help => vec![("Esc", "Close help"), ("q", "Quit")],
            AppMode::ConfirmDelete => vec![("y", "Yes"), ("n", "Cancel")],
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
            buf.set_string(x, area.y, *desc, desc_style);
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
                if self.session_stats.items_deleted == 1 { "" } else { "s" }
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

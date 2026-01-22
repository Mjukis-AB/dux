use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};

use crate::app::AppMode;

use super::theme::Theme;

/// Footer widget showing keyboard hints
pub struct Footer<'a> {
    mode: AppMode,
    theme: &'a Theme,
}

impl<'a> Footer<'a> {
    pub fn new(mode: AppMode, theme: &'a Theme) -> Self {
        Self { mode, theme }
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
    }
}

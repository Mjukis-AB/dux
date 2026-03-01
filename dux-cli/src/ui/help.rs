use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Clear, Padding, Widget},
};

use super::theme::Theme;

/// Help overlay widget
pub struct HelpView<'a> {
    theme: &'a Theme,
}

impl<'a> HelpView<'a> {
    pub fn new(theme: &'a Theme) -> Self {
        Self { theme }
    }
}

impl Widget for HelpView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Center the help box
        let width = 50.min(area.width.saturating_sub(4));
        let height = 31.min(area.height.saturating_sub(4));
        let x = area.x + (area.width - width) / 2;
        let y = area.y + (area.height - height) / 2;
        let help_area = Rect::new(x, y, width, height);

        // Clear the area
        Clear.render(help_area, buf);

        // Draw border
        let block = Block::default()
            .title(" Help ")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.blue))
            .style(Style::default().bg(self.theme.bg_surface))
            .padding(Padding::uniform(1));

        let inner = block.inner(help_area);
        block.render(help_area, buf);

        let key_style = Style::default()
            .fg(self.theme.yellow)
            .add_modifier(Modifier::BOLD);
        let desc_style = Style::default().fg(self.theme.fg);
        let section_style = Style::default()
            .fg(self.theme.blue)
            .add_modifier(Modifier::BOLD);

        let help_items = [
            ("", "Views", true),
            ("Tab", "Next view", false),
            ("S-Tab", "Previous view", false),
            ("s", "Cycle stale threshold (Build Artifacts)", false),
            ("", "", false),
            ("", "Navigation", true),
            ("↑ k", "Move up", false),
            ("↓ j", "Move down", false),
            ("v", "Enter/exit select mode", false),
            ("K", "Select up (or ↑ in select mode)", false),
            ("J", "Select down (or ↓ in select mode)", false),
            ("PgUp/PgDn", "Page up/down", false),
            ("Home g", "Go to first", false),
            ("End G", "Go to last", false),
            ("Esc", "Clear selection / Go back", false),
            ("", "", false),
            ("", "Tree", true),
            ("→ l", "Expand directory", false),
            ("← h", "Collapse directory", false),
            ("Space", "Toggle expand/collapse", false),
            ("Enter", "Drill down into directory", false),
            ("Backspace", "Go back", false),
            ("", "", false),
            ("", "Actions", true),
            ("o", "Open in Finder", false),
            ("d", "Delete selected item(s)", false),
            ("", "", false),
            ("", "Other", true),
            ("?", "Toggle this help", false),
            ("q Ctrl+C", "Quit", false),
        ];

        for (i, (key, desc, is_section)) in help_items.iter().enumerate() {
            if i >= inner.height as usize {
                break;
            }

            let y = inner.y + i as u16;

            if *is_section {
                buf.set_string(inner.x, y, *desc, section_style);
            } else if !key.is_empty() {
                buf.set_string(inner.x, y, format!("{:12}", key), key_style);
                buf.set_string(inner.x + 12, y, *desc, desc_style);
            }
        }
    }
}

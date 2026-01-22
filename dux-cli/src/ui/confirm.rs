use std::path::Path;

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Clear, Padding, Widget},
};

use super::theme::Theme;

/// Delete confirmation dialog widget
pub struct ConfirmDeleteView<'a> {
    path: &'a Path,
    size: Option<u64>,
    theme: &'a Theme,
}

impl<'a> ConfirmDeleteView<'a> {
    pub fn new(path: &'a Path, size: Option<u64>, theme: &'a Theme) -> Self {
        Self { path, size, theme }
    }
}

impl Widget for ConfirmDeleteView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Center the dialog box
        let width = 50.min(area.width.saturating_sub(4));
        let height = 9.min(area.height.saturating_sub(4));
        let x = area.x + (area.width - width) / 2;
        let y = area.y + (area.height - height) / 2;
        let dialog_area = Rect::new(x, y, width, height);

        // Clear the area
        Clear.render(dialog_area, buf);

        // Draw border
        let block = Block::default()
            .title(" Delete? ")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.red))
            .style(Style::default().bg(self.theme.bg_surface))
            .padding(Padding::uniform(1));

        let inner = block.inner(dialog_area);
        block.render(dialog_area, buf);

        let text_style = Style::default().fg(self.theme.fg);
        let path_style = Style::default()
            .fg(self.theme.yellow)
            .add_modifier(Modifier::BOLD);
        let key_style = Style::default()
            .fg(self.theme.green)
            .add_modifier(Modifier::BOLD);

        // Path to delete (truncated if needed)
        let path_str = self.path.to_string_lossy();
        let max_path_len = (inner.width as usize).saturating_sub(2);
        let display_path = if path_str.len() > max_path_len {
            format!("...{}", &path_str[path_str.len() - max_path_len + 3..])
        } else {
            path_str.to_string()
        };

        buf.set_string(inner.x, inner.y, "Delete:", text_style);
        buf.set_string(inner.x, inner.y + 1, &display_path, path_style);

        // Size info
        if let Some(size) = self.size {
            let size_str = format!("Size: {}", dux_core::format_size(size));
            buf.set_string(inner.x, inner.y + 3, &size_str, text_style);
        }

        // Action hints
        let hints_y = inner.y + inner.height.saturating_sub(1);
        buf.set_string(inner.x, hints_y, "[y]", key_style);
        buf.set_string(inner.x + 4, hints_y, "Yes, delete", text_style);
        buf.set_string(inner.x + 18, hints_y, "[n]", key_style);
        buf.set_string(inner.x + 22, hints_y, "Cancel", text_style);
    }
}

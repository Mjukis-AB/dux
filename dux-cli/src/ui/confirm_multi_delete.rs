use std::path::PathBuf;

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Clear, Padding, Widget},
};

use super::theme::Theme;

/// Multi-delete confirmation dialog widget
pub struct ConfirmMultiDeleteView<'a> {
    items: &'a [(dux_core::NodeId, PathBuf, u64)],
    theme: &'a Theme,
}

impl<'a> ConfirmMultiDeleteView<'a> {
    pub fn new(items: &'a [(dux_core::NodeId, PathBuf, u64)], theme: &'a Theme) -> Self {
        Self { items, theme }
    }
}

impl Widget for ConfirmMultiDeleteView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let count = self.items.len();
        let total_size: u64 = self.items.iter().map(|(_, _, s)| s).sum();
        let show_count = count.min(5);
        let has_more = count > 5;

        // Dynamic height: title(1) + padding(2) + "Delete N items:"(1) + paths(show_count)
        // + "...and N more"(if has_more) + blank(1) + total_size(1) + blank(1) + hints(1) + border(2) + padding(2)
        let content_lines = 1 + show_count + if has_more { 1 } else { 0 } + 1 + 1 + 1 + 1;
        let height = (content_lines as u16 + 4).min(area.height.saturating_sub(4)); // +4 for borders+padding
        let width = 60.min(area.width.saturating_sub(4));

        let x = area.x + (area.width - width) / 2;
        let y = area.y + (area.height - height) / 2;
        let dialog_area = Rect::new(x, y, width, height);

        Clear.render(dialog_area, buf);

        let block = Block::default()
            .title(" Delete Multiple? ")
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
        let dim_style = Style::default().fg(self.theme.fg_dim);
        let key_style = Style::default()
            .fg(self.theme.green)
            .add_modifier(Modifier::BOLD);

        let mut row = inner.y;
        let max_w = inner.width as usize - 2;

        // Header line
        let header = format!(
            "Delete {} item{}:",
            count,
            if count == 1 { "" } else { "s" }
        );
        buf.set_string(inner.x, row, &header, text_style);
        row += 1;

        // List up to 5 paths with sizes
        for (_, path, size) in self.items.iter().take(5) {
            let size_str = dux_core::format_size(*size);
            let path_str = path.to_string_lossy();
            // Reserve space for "  path  (size)"
            let size_part = format!("  ({})", size_str);
            let avail = max_w.saturating_sub(size_part.len() + 2);
            let display_path = if path_str.len() > avail {
                format!("...{}", &path_str[path_str.len() - avail + 3..])
            } else {
                path_str.to_string()
            };
            buf.set_string(inner.x + 1, row, &display_path, path_style);
            buf.set_string(
                inner.x + 1 + display_path.len() as u16,
                row,
                &size_part,
                dim_style,
            );
            row += 1;
        }

        // "...and N more"
        if has_more {
            let more_text = format!("  ...and {} more", count - 5);
            buf.set_string(inner.x, row, &more_text, dim_style);
            row += 1;
        }

        row += 1; // blank line

        // Total size
        let total_str = format!("Total: {}", dux_core::format_size(total_size));
        buf.set_string(inner.x, row, &total_str, text_style);
        row += 1;

        // Action hints at bottom
        let hints_y = row.max(inner.y + inner.height.saturating_sub(1));
        buf.set_string(inner.x, hints_y, "[y]", key_style);
        buf.set_string(inner.x + 4, hints_y, "Yes, delete all", text_style);
        buf.set_string(inner.x + 22, hints_y, "[n]", key_style);
        buf.set_string(inner.x + 26, hints_y, "Cancel", text_style);
    }
}

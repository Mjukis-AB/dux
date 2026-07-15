use std::path::PathBuf;

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Clear, Padding, Widget},
};

use super::text::{char_count, truncate_start};
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
        let total_size = self
            .items
            .iter()
            .map(|(_, _, size)| *size)
            .fold(0u64, u64::saturating_add);
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
            .title(" Permanent Delete? ")
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
        let max_w = (inner.width as usize).saturating_sub(2);

        // Header line
        let header = format!(
            "Permanently delete {} item{}:",
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
            let display_path = truncate_start(&path_str, avail);
            buf.set_string(inner.x + 1, row, &display_path, path_style);
            buf.set_string(
                inner.x + 1 + char_count(&display_path) as u16,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn render_items(items: &[(dux_core::NodeId, PathBuf, u64)]) -> String {
        let area = Rect::new(0, 0, 90, 24);
        let mut buffer = Buffer::empty(area);
        ConfirmMultiDeleteView::new(items, &Theme::default()).render(area, &mut buffer);
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn renders_effective_item_count_with_correct_pluralization() {
        let one = vec![(dux_core::NodeId::ROOT, PathBuf::from("/tmp/a"), 1)];
        assert!(render_items(&one).contains("Permanently delete 1 item:"));

        let two = vec![
            (dux_core::NodeId::ROOT, PathBuf::from("/tmp/a"), 1),
            (dux_core::NodeId::ROOT, PathBuf::from("/tmp/b"), 2),
        ];
        let rendered = render_items(&two);
        assert!(rendered.contains("Permanently delete 2 items:"));
        assert!(rendered.contains("[y]"));
        assert!(rendered.contains("[n]"));
    }

    #[test]
    fn total_size_saturates_like_the_footer() {
        let items = vec![
            (dux_core::NodeId::ROOT, PathBuf::from("/tmp/huge"), u64::MAX),
            (dux_core::NodeId::ROOT, PathBuf::from("/tmp/extra"), 1),
        ];

        let rendered = render_items(&items);

        assert!(rendered.contains(&format!("Total: {}", dux_core::format_size(u64::MAX))));
    }
}

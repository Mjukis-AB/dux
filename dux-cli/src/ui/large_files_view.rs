use dux_core::format_size;
use ratatui::{buffer::Buffer, layout::Rect, style::Style, widgets::Widget};

use crate::app::views::LargeFileEntry;

use super::bar_chart::render_bar;
use super::theme::Theme;

/// Large files flat list view
pub struct LargeFilesView<'a> {
    entries: &'a [LargeFileEntry],
    selected_index: usize,
    scroll_offset: usize,
    theme: &'a Theme,
}

impl<'a> LargeFilesView<'a> {
    pub fn new(
        entries: &'a [LargeFileEntry],
        selected_index: usize,
        scroll_offset: usize,
        theme: &'a Theme,
    ) -> Self {
        Self {
            entries,
            selected_index,
            scroll_offset,
            theme,
        }
    }
}

impl Widget for LargeFilesView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 1 || area.width < 40 {
            return;
        }

        if self.entries.is_empty() {
            let msg = "No large files found";
            let x = area.x + (area.width.saturating_sub(msg.len() as u16)) / 2;
            let y = area.y + area.height / 2;
            buf.set_string(x, y, msg, Style::default().fg(self.theme.fg_dim));
            return;
        }

        // Column widths (same as TreeView)
        let bar_width: usize = 24;
        let pct_width: usize = 6;
        let size_width: usize = 10;
        let path_width = area.width as usize - bar_width - pct_width - size_width - 4;

        for (i, entry) in self
            .entries
            .iter()
            .skip(self.scroll_offset)
            .take(area.height as usize)
            .enumerate()
        {
            let y = area.y + i as u16;
            let is_selected = i + self.scroll_offset == self.selected_index;

            let row_style = if is_selected {
                Style::default()
                    .bg(self.theme.selection_bg)
                    .fg(self.theme.selection_fg)
            } else {
                Style::default().fg(self.theme.fg)
            };

            // Clear the row
            for x in 0..area.width {
                buf.set_string(area.x + x, y, " ", row_style);
            }

            let mut x = area.x;

            // Icon
            let icon_style = if is_selected {
                row_style
            } else {
                Style::default().fg(self.theme.fg_dim)
            };
            buf.set_string(x, y, "📄", icon_style);
            x += 2;

            // Path (truncated with leading ... if too long)
            let max_path_len = path_width.saturating_sub(3); // 2 for icon + 1 space
            let display_path = if entry.relative_path.len() > max_path_len {
                let start = entry.relative_path.len() - max_path_len + 3;
                format!("...{}", &entry.relative_path[start..])
            } else {
                entry.relative_path.clone()
            };

            let path_style = if is_selected {
                row_style
            } else {
                Style::default().fg(self.theme.fg)
            };
            buf.set_string(x, y, &display_path, path_style);

            // Right-aligned section
            let right_x =
                area.x + area.width - bar_width as u16 - pct_width as u16 - size_width as u16 - 2;

            // Size bar
            let bar_color = if is_selected {
                self.theme.selection_fg
            } else {
                self.theme.size_color(entry.percentage)
            };
            let (bar, _) = render_bar(entry.percentage, bar_width.saturating_sub(2), bar_color);
            buf.set_string(
                right_x,
                y,
                &bar,
                if is_selected {
                    row_style
                } else {
                    Style::default().fg(bar_color)
                },
            );

            // Percentage
            let pct_str = format!("{:>5.1}%", entry.percentage);
            let pct_style = if is_selected {
                row_style
            } else {
                Style::default().fg(self.theme.fg_dim)
            };
            buf.set_string(right_x + bar_width as u16 - 1, y, &pct_str, pct_style);

            // Size
            let size_str = format!("{:>9}", format_size(entry.size));
            let size_style = if is_selected {
                row_style
            } else {
                Style::default().fg(self.theme.fg_muted)
            };
            buf.set_string(
                right_x + bar_width as u16 + pct_width as u16 - 1,
                y,
                &size_str,
                size_style,
            );
        }
    }
}

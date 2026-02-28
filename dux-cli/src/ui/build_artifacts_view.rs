use dux_core::format_size;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};

use crate::app::views::{BuildArtifactEntry, StaleThreshold};

use super::bar_chart::render_bar;
use super::theme::Theme;

/// Build artifacts flat list view
pub struct BuildArtifactsView<'a> {
    entries: &'a [BuildArtifactEntry],
    selected_index: usize,
    scroll_offset: usize,
    stale_threshold: StaleThreshold,
    theme: &'a Theme,
}

impl<'a> BuildArtifactsView<'a> {
    pub fn new(
        entries: &'a [BuildArtifactEntry],
        selected_index: usize,
        scroll_offset: usize,
        stale_threshold: StaleThreshold,
        theme: &'a Theme,
    ) -> Self {
        Self {
            entries,
            selected_index,
            scroll_offset,
            stale_threshold,
            theme,
        }
    }
}

impl Widget for BuildArtifactsView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 2 || area.width < 40 {
            return;
        }

        // Subtitle row showing stale threshold
        let subtitle = format!("Stale: >{} (s to change)", self.stale_threshold.label());
        buf.set_string(
            area.x + 1,
            area.y,
            &subtitle,
            Style::default().fg(self.theme.fg_dim),
        );

        let list_area = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );

        if self.entries.is_empty() {
            let msg = "No build artifacts found";
            let x = list_area.x + (list_area.width.saturating_sub(msg.len() as u16)) / 2;
            let y = list_area.y + list_area.height / 2;
            buf.set_string(x, y, msg, Style::default().fg(self.theme.fg_dim));
            return;
        }

        // Column widths (same as TreeView)
        let bar_width: usize = 24;
        let pct_width: usize = 6;
        let size_width: usize = 10;
        // Reserve space for kind label + stale indicator
        let kind_width: usize = 12; // "[CocoaPods] " max
        let stale_width: usize = 6; // "stale " or "      "
        let path_width = list_area.width as usize
            - bar_width
            - pct_width
            - size_width
            - kind_width
            - stale_width
            - 4;

        for (i, entry) in self
            .entries
            .iter()
            .skip(self.scroll_offset)
            .take(list_area.height as usize)
            .enumerate()
        {
            let y = list_area.y + i as u16;
            let is_selected = i + self.scroll_offset == self.selected_index;

            let row_style = if is_selected {
                Style::default()
                    .bg(self.theme.selection_bg)
                    .fg(self.theme.selection_fg)
            } else {
                Style::default().fg(self.theme.fg)
            };

            // Clear the row
            for x in 0..list_area.width {
                buf.set_string(list_area.x + x, y, " ", row_style);
            }

            let mut x = list_area.x;

            // Icon
            let icon_style = if is_selected {
                row_style
            } else {
                Style::default().fg(self.theme.yellow)
            };
            buf.set_string(x, y, "📁", icon_style);
            x += 2;

            // Path
            let max_path_len = path_width.saturating_sub(3);
            let display_path = if entry.relative_path.len() > max_path_len {
                let start = entry.relative_path.len() - max_path_len + 3;
                format!("...{}", &entry.relative_path[start..])
            } else {
                entry.relative_path.clone()
            };

            let path_style = if is_selected {
                row_style.add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(self.theme.fg)
                    .add_modifier(Modifier::BOLD)
            };
            buf.set_string(x, y, &display_path, path_style);
            x += display_path.len() as u16 + 1;

            // Kind label
            let kind_label = format!("[{}]", entry.kind.label());
            let kind_style = if is_selected {
                row_style
            } else {
                Style::default().fg(self.theme.fg_muted)
            };
            buf.set_string(x, y, &kind_label, kind_style);
            x += kind_label.len() as u16 + 1;

            // Stale indicator
            if entry.is_stale {
                let stale_style = if is_selected {
                    row_style
                } else {
                    Style::default().fg(self.theme.yellow)
                };
                buf.set_string(x, y, "stale", stale_style);
            }

            // Right-aligned section
            let right_x = list_area.x + list_area.width
                - bar_width as u16
                - pct_width as u16
                - size_width as u16
                - 2;

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

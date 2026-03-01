use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Clear, Padding, Widget},
};

use crate::app::MultiDeleteProgress;

use super::bar_chart::render_bar;
use super::theme::Theme;

/// Progress overlay shown during multi-delete
pub struct MultiDeleteProgressView<'a> {
    progress: &'a MultiDeleteProgress,
    theme: &'a Theme,
}

impl<'a> MultiDeleteProgressView<'a> {
    pub fn new(progress: &'a MultiDeleteProgress, theme: &'a Theme) -> Self {
        Self { progress, theme }
    }
}

impl Widget for MultiDeleteProgressView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let width = 50.min(area.width.saturating_sub(4));
        let height = 10.min(area.height.saturating_sub(4));
        let x = area.x + (area.width - width) / 2;
        let y = area.y + (area.height - height) / 2;
        let dialog_area = Rect::new(x, y, width, height);

        Clear.render(dialog_area, buf);

        let block = Block::default()
            .title(" Deleting... ")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.yellow))
            .style(Style::default().bg(self.theme.bg_surface))
            .padding(Padding::uniform(1));

        let inner = block.inner(dialog_area);
        block.render(dialog_area, buf);

        let text_style = Style::default().fg(self.theme.fg);
        let dim_style = Style::default().fg(self.theme.fg_dim);

        let mut row = inner.y;

        // Progress count
        let count_str = format!(
            "{} / {} completed",
            self.progress.completed, self.progress.total
        );
        buf.set_string(inner.x, row, &count_str, text_style);
        row += 1;

        // Progress bar
        let bar_width = (inner.width as usize).saturating_sub(2);
        let pct = if self.progress.total > 0 {
            (self.progress.completed as f64 / self.progress.total as f64) * 100.0
        } else {
            0.0
        };
        let (bar, _) = render_bar(pct, bar_width, self.theme.green);
        buf.set_string(inner.x, row, &bar, Style::default().fg(self.theme.green));
        row += 2;

        // Freed bytes
        let freed_str = format!(
            "Freed: {}",
            dux_core::format_size(self.progress.bytes_freed)
        );
        buf.set_string(inner.x, row, &freed_str, text_style);
        row += 1;

        // Failures
        if !self.progress.failures.is_empty() {
            let fail_str = format!("{} failed", self.progress.failures.len());
            buf.set_string(
                inner.x,
                row,
                &fail_str,
                Style::default()
                    .fg(self.theme.red)
                    .add_modifier(Modifier::BOLD),
            );
            row += 1;
        }

        // Hint at bottom
        let hint_y = row.max(inner.y + inner.height.saturating_sub(1));
        buf.set_string(
            inner.x,
            hint_y,
            "Press q to quit (deletions continue in background)",
            dim_style,
        );
    }
}

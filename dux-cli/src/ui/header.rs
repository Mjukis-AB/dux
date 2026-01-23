use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};

use crate::app::AppState;

use super::progress::progress_indicator;
use super::theme::Theme;

/// Header widget showing title, path, and status
pub struct Header<'a> {
    state: &'a AppState,
    theme: &'a Theme,
}

impl<'a> Header<'a> {
    pub fn new(state: &'a AppState, theme: &'a Theme) -> Self {
        Self { state, theme }
    }
}

impl Widget for Header<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 1 {
            return;
        }

        // Title
        let title = "DUX";
        let title_style = Style::default()
            .fg(self.theme.blue)
            .add_modifier(Modifier::BOLD);
        buf.set_string(area.x + 1, area.y, title, title_style);

        // Separator
        buf.set_string(
            area.x + 5,
            area.y,
            "─",
            Style::default().fg(self.theme.border),
        );

        // Path/breadcrumbs
        let path = if let Some(tree) = &self.state.tree {
            tree.breadcrumbs(self.state.view_root)
        } else {
            self.state.root_path.to_string_lossy().to_string()
        };

        let max_path_len = area.width.saturating_sub(30) as usize;
        let display_path = if path.len() > max_path_len {
            format!("...{}", &path[path.len() - max_path_len + 3..])
        } else {
            path
        };

        buf.set_string(
            area.x + 7,
            area.y,
            &display_path,
            Style::default().fg(self.theme.fg),
        );

        // Status (right-aligned)
        let status = if self.state.tree.is_none() {
            progress_indicator(&self.state.progress, self.state.spinner_frame)
        } else if let Some(tree) = &self.state.tree {
            let cached_indicator = if self.state.loaded_from_cache {
                " (cached)"
            } else {
                ""
            };
            format!(
                "{} files, {}{}",
                dux_core::format_count(tree.total_files()),
                dux_core::format_size(tree.total_size()),
                cached_indicator
            )
        } else {
            String::new()
        };

        let status_x = area.x + area.width - status.len() as u16 - 2;
        let status_style = if self.state.tree.is_none() {
            Style::default().fg(self.theme.yellow)
        } else {
            Style::default().fg(self.theme.fg_dim)
        };
        buf.set_string(status_x, area.y, &status, status_style);
    }
}

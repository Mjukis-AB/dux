use dux_core::{ScanProgress, format_count, format_size};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Block, Borders, Padding, Widget},
};

use super::theme::Theme;

/// Braille spinner characters
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Progress widget shown during scanning
pub struct ProgressView<'a> {
    progress: &'a ScanProgress,
    spinner_frame: usize,
    finalizing: bool,
    theme: &'a Theme,
}

impl<'a> ProgressView<'a> {
    pub fn new(
        progress: &'a ScanProgress,
        spinner_frame: usize,
        finalizing: bool,
        theme: &'a Theme,
    ) -> Self {
        Self {
            progress,
            spinner_frame,
            finalizing,
            theme,
        }
    }
}

impl Widget for ProgressView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Draw border
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border))
            .padding(Padding::horizontal(1));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 || inner.width < 20 {
            return;
        }

        // Spinner
        let spinner = SPINNER[self.spinner_frame % SPINNER.len()];
        let spinner_style = Style::default()
            .fg(self.theme.blue)
            .add_modifier(Modifier::BOLD);

        buf.set_string(inner.x, inner.y, &spinner.to_string(), spinner_style);

        // Status text
        let status_text = if self.finalizing {
            " Finalizing... (calculating sizes)"
        } else {
            " Scanning..."
        };
        buf.set_string(
            inner.x + 2,
            inner.y,
            status_text,
            Style::default().fg(self.theme.fg),
        );

        // Current path (truncated) - only show during scanning, not finalizing
        if !self.finalizing {
            if let Some(path) = &self.progress.current_path {
                let path_str = path.to_string_lossy();
                let max_len = inner.width.saturating_sub(2) as usize;
                let display_path = if path_str.len() > max_len {
                    format!("...{}", &path_str[path_str.len() - max_len + 3..])
                } else {
                    path_str.to_string()
                };

                buf.set_string(
                    inner.x,
                    inner.y + 1,
                    &display_path,
                    Style::default().fg(self.theme.fg_dim),
                );
            }
        }

        // Stats line
        let stats = format!(
            "{} files  {} dirs  {} errors  {}",
            format_count(self.progress.files_scanned),
            format_count(self.progress.dirs_scanned),
            format_count(self.progress.errors),
            format_size(self.progress.bytes_scanned),
        );

        buf.set_string(
            inner.x,
            inner.y + 2,
            &stats,
            Style::default().fg(self.theme.fg_muted),
        );
    }
}

/// Compact progress indicator for header
pub fn progress_indicator(progress: &ScanProgress, spinner_frame: usize) -> String {
    let spinner = SPINNER[spinner_frame % SPINNER.len()];
    format!(
        "{} {} files, {}",
        spinner,
        format_count(progress.files_scanned + progress.dirs_scanned),
        format_size(progress.bytes_scanned)
    )
}

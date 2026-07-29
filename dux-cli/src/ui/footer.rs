use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};

use crate::app::views::{StaleThreshold, stale_threshold_label};
use crate::app::{AppMode, ViewMode};

use super::text::{char_count, truncate_start};
use super::theme::Theme;

/// Footer widget showing keyboard hints and selection totals.
pub struct Footer<'a> {
    mode: AppMode,
    view_mode: ViewMode,
    theme: &'a Theme,
    stale_threshold: Option<StaleThreshold>,
    selection_count: usize,
    selection_size: u64,
    selecting_mode: bool,
}

impl<'a> Footer<'a> {
    pub fn new(mode: AppMode, view_mode: ViewMode, theme: &'a Theme) -> Self {
        Self {
            mode,
            view_mode,
            theme,
            stale_threshold: None,
            selection_count: 0,
            selection_size: 0,
            selecting_mode: false,
        }
    }

    pub fn with_stale_threshold(mut self, threshold: StaleThreshold) -> Self {
        self.stale_threshold = Some(threshold);
        self
    }

    pub fn with_selection(mut self, count: usize, size: u64, selecting: bool) -> Self {
        self.selection_count = count;
        self.selection_size = size;
        self.selecting_mode = selecting;
        self
    }
}

impl Widget for Footer<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 20 || area.height < 1 {
            return;
        }

        let select_hint = if self.selecting_mode {
            ("v/Esc", "Stop select".to_string())
        } else {
            ("v", "Select".to_string())
        };

        let hints: Vec<(&str, String)> = match self.mode {
            AppMode::Scanning | AppMode::Finalizing => vec![("q", "Quit".to_string())],
            AppMode::Browsing => match self.view_mode {
                ViewMode::Tree => vec![
                    ("Tab", "Views".to_string()),
                    ("↑↓", "Navigate".to_string()),
                    select_hint.clone(),
                    ("←→", "Collapse/Expand".to_string()),
                    ("r", "Rescan".to_string()),
                    ("?", "Help".to_string()),
                    ("q", "Quit".to_string()),
                ],
                ViewMode::LargeFiles => vec![
                    ("Tab", "Views".to_string()),
                    ("↑↓", "Navigate".to_string()),
                    select_hint.clone(),
                    ("r", "Rescan".to_string()),
                    ("?", "Help".to_string()),
                    ("q", "Quit".to_string()),
                ],
                ViewMode::BuildArtifacts => {
                    let stale_label = self
                        .stale_threshold
                        .map(|threshold| format!("Stale:{}", stale_threshold_label(threshold)))
                        .unwrap_or_else(|| "Stale".to_string());
                    vec![
                        ("Tab", "Views".to_string()),
                        ("↑↓", "Navigate".to_string()),
                        select_hint.clone(),
                        ("s", stale_label),
                        ("r", "Rescan".to_string()),
                        ("?", "Help".to_string()),
                        ("q", "Quit".to_string()),
                    ]
                }
            },
            AppMode::Help => vec![("Esc", "Close help".to_string()), ("q", "Quit".to_string())],
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
            x += char_count(key) as u16 + 1;

            // Description
            buf.set_string(x, area.y, desc.as_str(), desc_style);
            x += char_count(desc) as u16;

            // Separator
            if i < hints.len() - 1 {
                buf.set_string(x, area.y, "  │  ", sep_style);
                x += 5;
            }

            if x >= area.x + area.width - 5 {
                break;
            }
        }

        // Right side: selection info.
        let right_text = if self.selection_count > 0 {
            Some((
                format!(
                    "{} selected ({})",
                    self.selection_count,
                    dux_core::format_size(self.selection_size),
                ),
                Style::default()
                    .fg(self.theme.purple)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            None
        };

        if let Some((text, style)) = right_text {
            let text = truncate_start(&text, area.width.saturating_sub(2) as usize);
            let stats_x = area.x + area.width.saturating_sub(char_count(&text) as u16 + 1);
            if stats_x > x + 2 {
                buf.set_string(stats_x, area.y, &text, style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsing_copy_has_navigation_without_a_delete_action() {
        let area = Rect::new(0, 0, 240, 1);
        let mut buffer = Buffer::empty(area);
        Footer::new(AppMode::Browsing, ViewMode::Tree, &Theme::default()).render(area, &mut buffer);

        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Navigate"));
        assert!(text.contains("Rescan"));
        assert!(!text.contains("delete"));
        assert!(!text.contains("Delete"));
    }
}

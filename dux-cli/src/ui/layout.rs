use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Main application layout
pub struct AppLayout {
    pub header: Rect,
    pub size_bar: Rect,
    pub tree: Rect,
    pub footer: Rect,
}

impl AppLayout {
    pub fn new(area: Rect) -> Self {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Length(1), // Size bar
                Constraint::Min(5),    // Tree view
                Constraint::Length(1), // Footer
            ])
            .split(area);

        Self {
            header: chunks[0],
            size_bar: chunks[1],
            tree: chunks[2],
            footer: chunks[3],
        }
    }
}

/// Calculate centered rectangle for overlays
#[allow(dead_code)]
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;

    let x = area.x + (area.width - popup_width) / 2;
    let y = area.y + (area.height - popup_height) / 2;

    Rect::new(x, y, popup_width, popup_height)
}

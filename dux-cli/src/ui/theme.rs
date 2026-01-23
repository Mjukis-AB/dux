use ratatui::style::Color;

/// Catppuccin Mocha-inspired dark theme with 24-bit RGB colors
#[allow(dead_code)]
pub struct Theme {
    // Base colors
    pub bg: Color,
    pub bg_surface: Color,
    pub bg_highlight: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub fg_muted: Color,

    // Accent colors
    pub blue: Color,
    pub green: Color,
    pub yellow: Color,
    pub red: Color,
    pub purple: Color,
    pub teal: Color,

    // UI elements
    pub border: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,

    // Size gradient (green -> yellow -> red)
    pub size_small: Color,
    pub size_medium: Color,
    pub size_large: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            // Catppuccin Mocha base
            bg: Color::Rgb(30, 30, 46),           // Base
            bg_surface: Color::Rgb(49, 50, 68),   // Surface0
            bg_highlight: Color::Rgb(69, 71, 90), // Surface1
            fg: Color::Rgb(205, 214, 244),        // Text
            fg_dim: Color::Rgb(166, 173, 200),    // Subtext0
            fg_muted: Color::Rgb(127, 132, 156),  // Overlay0

            // Accent colors
            blue: Color::Rgb(137, 180, 250),   // Blue
            green: Color::Rgb(166, 227, 161),  // Green
            yellow: Color::Rgb(249, 226, 175), // Yellow
            red: Color::Rgb(243, 139, 168),    // Red
            purple: Color::Rgb(203, 166, 247), // Mauve
            teal: Color::Rgb(148, 226, 213),   // Teal

            // UI
            border: Color::Rgb(88, 91, 112),         // Surface2
            selection_bg: Color::Rgb(137, 180, 250), // Blue
            selection_fg: Color::Rgb(30, 30, 46),    // Base

            // Size gradient
            size_small: Color::Rgb(166, 227, 161),  // Green
            size_medium: Color::Rgb(249, 226, 175), // Yellow
            size_large: Color::Rgb(243, 139, 168),  // Red
        }
    }
}

impl Theme {
    /// Get color for a size percentage (0-100)
    pub fn size_color(&self, percentage: f64) -> Color {
        if percentage < 10.0 {
            self.size_small
        } else if percentage < 30.0 {
            // Interpolate between green and yellow
            let t = (percentage - 10.0) / 20.0;
            interpolate_color(self.size_small, self.size_medium, t)
        } else if percentage < 50.0 {
            self.size_medium
        } else {
            // Interpolate between yellow and red
            let t = ((percentage - 50.0) / 50.0).min(1.0);
            interpolate_color(self.size_medium, self.size_large, t)
        }
    }

    /// Get icon color based on node kind
    pub fn icon_color(&self, is_directory: bool) -> Color {
        if is_directory {
            self.yellow
        } else {
            self.fg_dim
        }
    }
}

/// Interpolate between two RGB colors
fn interpolate_color(from: Color, to: Color, t: f64) -> Color {
    match (from, to) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let r = lerp(r1, r2, t);
            let g = lerp(g1, g2, t);
            let b = lerp(b1, b2, t);
            Color::Rgb(r, g, b)
        }
        _ => to,
    }
}

fn lerp(a: u8, b: u8, t: f64) -> u8 {
    let a = a as f64;
    let b = b as f64;
    (a + (b - a) * t).round() as u8
}

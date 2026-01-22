use ratatui::style::Color;

/// Unicode partial block characters for smooth progress bars
const BLOCKS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

/// Render a size bar using partial block characters
pub fn render_bar(percentage: f64, width: usize, color: Color) -> (String, Color) {
    if width == 0 {
        return (String::new(), color);
    }

    let percentage = percentage.clamp(0.0, 100.0);
    let filled_width = (percentage / 100.0) * width as f64;
    let full_blocks = filled_width.floor() as usize;
    let partial = ((filled_width - full_blocks as f64) * 8.0).round() as usize;

    let mut bar = String::with_capacity(width * 3); // Unicode chars can be multi-byte

    // Full blocks
    for _ in 0..full_blocks.min(width) {
        bar.push(BLOCKS[8]);
    }

    // Partial block
    if full_blocks < width && partial > 0 {
        bar.push(BLOCKS[partial]);
    }

    // Pad to width
    let current_len = bar.chars().count();
    for _ in current_len..width {
        bar.push(' ');
    }

    (bar, color)
}

/// Render a full-width total size bar
#[allow(dead_code)]
pub fn render_total_bar(percentage: f64, width: usize, color: Color) -> String {
    let (bar, _) = render_bar(percentage, width, color);
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_bar_empty() {
        let (bar, _) = render_bar(0.0, 10, Color::Green);
        assert_eq!(bar.chars().count(), 10);
        assert!(bar.chars().all(|c| c == ' '));
    }

    #[test]
    fn test_render_bar_full() {
        let (bar, _) = render_bar(100.0, 10, Color::Green);
        assert_eq!(bar.chars().count(), 10);
        assert!(bar.chars().all(|c| c == '█'));
    }

    #[test]
    fn test_render_bar_half() {
        let (bar, _) = render_bar(50.0, 10, Color::Green);
        assert_eq!(bar.chars().count(), 10);
    }
}

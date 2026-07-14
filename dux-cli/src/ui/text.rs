pub fn char_count(value: &str) -> usize {
    value.chars().count()
}

/// Keep the beginning of a string, adding an ellipsis when truncated.
pub fn truncate_end(value: &str, max_chars: usize) -> String {
    let count = char_count(value);
    if count <= max_chars {
        return value.to_string();
    }

    match max_chars {
        0 => String::new(),
        1 => "…".to_string(),
        _ => value
            .chars()
            .take(max_chars - 1)
            .chain(std::iter::once('…'))
            .collect(),
    }
}

/// Keep the end of a string, adding an ellipsis when truncated.
pub fn truncate_start(value: &str, max_chars: usize) -> String {
    let count = char_count(value);
    if count <= max_chars {
        return value.to_string();
    }

    match max_chars {
        0 => String::new(),
        1 => "…".to_string(),
        _ => std::iter::once('…')
            .chain(value.chars().skip(count - (max_chars - 1)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_unicode_at_character_boundaries() {
        assert_eq!(truncate_end("räksmörgås", 5), "räks…");
        assert_eq!(truncate_start("räksmörgås", 5), "…rgås");
    }

    #[test]
    fn handles_tiny_widths() {
        assert_eq!(truncate_end("hello", 0), "");
        assert_eq!(truncate_start("hello", 1), "…");
    }
}

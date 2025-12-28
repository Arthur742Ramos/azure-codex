//! Text preview utilities for extracting and truncating preview text.

/// Truncate a string to a maximum number of characters, respecting UTF-8 char boundaries.
pub fn truncate_to_char_boundary(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Extract a short preview from thinking/reasoning text.
///
/// Attempts to extract the first sentence if it is short enough,
/// otherwise truncates to 50 characters with an ellipsis.
pub fn extract_thinking_preview(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Try to find the first sentence (ending with . or ? or !)
    let first_sentence_end = trimmed.find(['.', '?', '!']).map(|pos| pos + 1);

    let preview = if let Some(end) = first_sentence_end {
        let sentence = &trimmed[..end];
        if sentence.len() <= 60 {
            sentence.trim().to_string()
        } else {
            // Sentence too long, truncate
            format!("{}...", truncate_to_char_boundary(trimmed, 50).trim())
        }
    } else if trimmed.chars().count() > 50 {
        format!("{}...", truncate_to_char_boundary(trimmed, 50).trim())
    } else {
        trimmed.to_string()
    };

    Some(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_to_char_boundary() {
        assert_eq!(truncate_to_char_boundary("hello", 3), "hel");
        assert_eq!(truncate_to_char_boundary("hello", 10), "hello");
    }

    #[test]
    fn test_extract_thinking_preview_empty() {
        assert_eq!(extract_thinking_preview(""), None);
        assert_eq!(extract_thinking_preview("   "), None);
    }

    #[test]
    fn test_extract_thinking_preview_short_sentence() {
        assert_eq!(
            extract_thinking_preview("Hello world."),
            Some("Hello world.".to_string())
        );
    }

    #[test]
    fn test_extract_thinking_preview_long_truncates() {
        let long_text = "a".repeat(100);
        let result = extract_thinking_preview(&long_text).unwrap();
        assert!(result.ends_with("..."));
        assert!(result.len() <= 54);
    }
}

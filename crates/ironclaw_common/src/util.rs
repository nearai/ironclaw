//! Shared utility functions.

/// Truncate a string to at most `max_bytes` bytes at a char boundary, appending "...".
///
/// If the input is wrapped in `<tool_output ...>...</tool_output>` and truncation
/// removes the closing tag, the tag is re-appended so downstream XML parsers
/// never see an unclosed element.
pub fn truncate_preview(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Walk backwards from max_bytes to find a valid char boundary
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut result = format!("{}...", &s[..end]);

    // Re-close <tool_output> if truncation cut through the closing tag.
    if s.starts_with("<tool_output") && !result.ends_with("</tool_output>") {
        result.push_str("\n</tool_output>");
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_preview_short_string() {
        assert_eq!(truncate_preview("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_preview_exact_boundary() {
        assert_eq!(truncate_preview("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_preview_truncates_ascii() {
        assert_eq!(truncate_preview("hello world", 5), "hello...");
    }

    #[test]
    fn test_truncate_preview_empty_string() {
        assert_eq!(truncate_preview("", 10), "");
    }

    #[test]
    fn test_truncate_preview_multibyte_char_boundary() {
        let s = "a\u{20AC}b";
        let result = truncate_preview(s, 3);
        assert_eq!(result, "a...");
    }

    #[test]
    fn test_truncate_preview_emoji() {
        let s = "hi\u{1F980}";
        let result = truncate_preview(s, 4);
        assert_eq!(result, "hi...");
    }

    #[test]
    fn test_truncate_preview_cjk() {
        let s = "\u{4F60}\u{597D}\u{4E16}\u{754C}";
        let result = truncate_preview(s, 7);
        assert_eq!(result, "\u{4F60}\u{597D}...");
    }

    #[test]
    fn test_truncate_preview_zero_max_bytes() {
        assert_eq!(truncate_preview("hello", 0), "...");
    }

    #[test]
    fn test_truncate_preview_closes_tool_output_tag() {
        let s = "<tool_output name=\"search\">\nSome very long content here\n</tool_output>";
        let result = truncate_preview(s, 60);
        assert!(result.ends_with("</tool_output>"));
        assert!(result.contains("..."));
    }

    #[test]
    fn test_truncate_preview_no_extra_close_when_intact() {
        let s = "<tool_output name=\"echo\">\nshort\n</tool_output>";
        let result = truncate_preview(s, 500);
        assert_eq!(result, s);
        assert_eq!(result.matches("</tool_output>").count(), 1);
    }

    #[test]
    fn test_truncate_preview_non_xml_unaffected() {
        let s = "Just a plain long string that gets truncated";
        let result = truncate_preview(s, 10);
        assert_eq!(result, "Just a pla...");
        assert!(!result.contains("</tool_output>"));
    }
}

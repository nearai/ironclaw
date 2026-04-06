fn is_called_tool_marker(trimmed: &str) -> bool {
    trimmed.starts_with("[Called tool ") && trimmed.ends_with(']')
}

fn is_tool_result_marker(trimmed: &str) -> bool {
    trimmed.starts_with("[Tool ") && trimmed.contains(" returned:") && trimmed.ends_with(']')
}

fn is_internal_tool_marker(trimmed: &str) -> bool {
    is_called_tool_marker(trimmed) || is_tool_result_marker(trimmed)
}

/// Strip internal `[Called tool ...]` and `[Tool ... returned: ...]` markers
/// from provider-flattened text before it is shown to users.
///
/// `fallback` is returned when stripping leaves an empty string. Callers pass
/// context-specific messages (e.g. chat vs lightweight routine) so the
/// user-visible fallback matches the interaction they are in.
pub(crate) fn strip_internal_tool_call_text(text: &str, fallback: &str) -> String {
    let result = text
        .lines()
        .filter(|line| !is_internal_tool_marker(line.trim()))
        .fold(String::new(), |mut acc, line| {
            if !acc.is_empty() {
                acc.push('\n');
            }
            acc.push_str(line);
            acc
        });

    let result = result.trim();
    if result.is_empty() {
        fallback.to_string()
    } else {
        result.to_string()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn strip_internal_tool_call_text_removes_markers() {
        let input = "[Called tool search({\"query\": \"test\"})]\nHere is the answer.";
        let result = super::strip_internal_tool_call_text(input, "fallback");
        assert_eq!(result, "Here is the answer.");
    }

    #[test]
    fn strip_internal_tool_call_text_removes_returned_markers() {
        let input = "[Tool search returned: some result]\nSummary of findings.";
        let result = super::strip_internal_tool_call_text(input, "fallback");
        assert_eq!(result, "Summary of findings.");
    }

    #[test]
    fn strip_internal_tool_call_text_all_markers_yields_fallback() {
        let input = "[Called tool search({\"query\": \"test\"})]\n[Tool search returned: error]";
        let result = super::strip_internal_tool_call_text(input, "fallback");
        assert_eq!(result, "fallback");
    }

    #[test]
    fn strip_internal_tool_call_text_preserves_normal_text() {
        let input = "This is a normal response with [brackets] inside.";
        let result = super::strip_internal_tool_call_text(input, "fallback");
        assert_eq!(result, input);
    }

    #[test]
    fn strip_internal_tool_call_text_preserves_multiline_text_around_markers() {
        let input = "Line1\n[Called tool search({\"query\": \"test\"})]\nLine2";
        let result = super::strip_internal_tool_call_text(input, "fallback");
        assert_eq!(result, "Line1\nLine2");
    }
}

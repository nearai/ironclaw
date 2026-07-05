//! Deterministic repair for malformed provider tool-call argument JSON.

use serde_json::Value;

const MAX_REPAIR_BYTES: usize = 64 * 1024;

/// Repair provider-emitted tool-call argument JSON when a minimal deterministic
/// fix makes it parse.
///
/// Inspired by the LAB Meta-Harness `toolcall_json_repair` mechanism: Lee et
/// al., "Meta-Harness: End-to-End Optimization of Model Harnesses",
/// arXiv:2603.28052 (MIT).
pub(crate) fn repair_tool_args(args: &str, allow_completion: bool) -> Option<Value> {
    if args.len() > MAX_REPAIR_BYTES || args.trim().is_empty() || parses_json(args) {
        return None;
    }

    let stripped = strip_trailing_leaked_tags(args);
    if stripped != args
        && let Ok(value) = serde_json::from_str::<Value>(&stripped)
    {
        return Some(value);
    }

    if !allow_completion {
        return None;
    }

    if stripped != args
        && let Some(value) = complete_truncated_json(&stripped)
            .filter(|completed| completed != args)
            .and_then(|completed| serde_json::from_str::<Value>(&completed).ok())
    {
        return Some(value);
    }

    complete_truncated_json(args)
        .filter(|completed| completed != args)
        .and_then(|completed| serde_json::from_str::<Value>(&completed).ok())
}

fn parses_json(input: &str) -> bool {
    serde_json::from_str::<serde::de::IgnoredAny>(input).is_ok()
}

fn strip_trailing_leaked_tags(input: &str) -> String {
    let trimmed_end = trim_end_index(input);
    if !may_end_with_xmlish_tag(&input[..trimmed_end]) {
        return input.to_owned();
    }

    let mut candidate = input.to_owned();

    loop {
        let trimmed_end = trim_end_index(&candidate);
        let Some(start) = trailing_xmlish_tag_start(&candidate[..trimmed_end]) else {
            break;
        };
        candidate.truncate(start);
    }

    candidate
}

fn may_end_with_xmlish_tag(input: &str) -> bool {
    match input.char_indices().next_back() {
        Some((_, '>')) => true,
        Some((_, ch)) => is_tag_name_continue(ch),
        None => false,
    }
}

fn trim_end_index(input: &str) -> usize {
    input
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0)
}

fn trailing_xmlish_tag_start(input: &str) -> Option<usize> {
    if input.is_empty() {
        return None;
    }

    let tag_body_end = match input.char_indices().next_back() {
        Some((idx, '>')) => idx,
        Some((idx, ch)) => idx + ch.len_utf8(),
        None => return None,
    };
    let name_end = trim_end_index(&input[..tag_body_end]);
    let start = input[..name_end].rfind('<')?;
    let mut chars = input[start + '<'.len_utf8()..name_end].chars().peekable();

    if chars.next_if_eq(&'/').is_none() || chars.peek().is_none() {
        return None;
    }

    let first = chars.next()?;
    if !is_tag_name_start(first) {
        return None;
    }
    if !chars.all(is_tag_name_continue) {
        return None;
    }

    Some(start)
}

fn is_tag_name_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || ch == '\u{2581}'
}

fn is_tag_name_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '\u{2581}' | '.' | '-')
}

fn complete_truncated_json(input: &str) -> Option<String> {
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for ch in input.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                // Bind before the check so this stays a plain arm body rather
                // than a match guard (keeps clippy::collapsible_match quiet and
                // the stack side effect explicit).
                let matched = stack.pop() == Some(ch);
                if !matched {
                    return None;
                }
            }
            _ => {}
        }
    }

    if stack.is_empty() && !in_string && !escaped {
        return None;
    }

    let mut candidate = input.to_owned();
    if in_string {
        if let Some(start) = incomplete_escape_start(&candidate) {
            candidate.truncate(start);
        }
        candidate.push('"');
    }
    for closer in stack.iter().rev() {
        candidate.push(*closer);
    }

    Some(candidate)
}

fn incomplete_escape_start(input: &str) -> Option<usize> {
    let slash = input.rfind('\\')?;
    if !is_active_escape(input, slash) {
        return None;
    }

    let tail = &input[slash + '\\'.len_utf8()..];
    if tail.is_empty() {
        return Some(slash);
    }

    let digits = tail.strip_prefix('u')?;
    let digit_count = digits.chars().count();
    if digit_count < 4 && digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(slash)
    } else {
        None
    }
}

fn is_active_escape(input: &str, slash: usize) -> bool {
    input[..slash]
        .chars()
        .rev()
        .take_while(|ch| *ch == '\\')
        .count()
        % 2
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn repaired_value(input: &str) -> Value {
        repair_tool_args(input, true).expect("input should repair")
    }

    #[test]
    fn valid_json_returns_none() {
        assert_eq!(repair_tool_args(r#"{"command":"ls"}"#, true), None);
    }

    #[test]
    fn strips_trailing_parameter_tag() {
        let value = repaired_value(r#"{"command":"ls"}</parameter>"#);
        assert_eq!(value["command"], "ls");
    }

    #[test]
    fn completes_object_truncated_mid_string() {
        let value = repaired_value(r#"{"command":"cat","content":"hello"#);
        assert_eq!(value["command"], "cat");
        assert_eq!(value["content"], "hello");
    }

    #[test]
    fn strips_deepseek_sentencepiece_tool_call_tag() {
        let value = repaired_value("{\"content\":\"ok\"}</tool\u{2581}call>");
        assert_eq!(value["content"], "ok");
    }

    #[test]
    fn balances_nested_array_truncation() {
        let value = repaired_value(r#"{"items":[{"content":"one"},{"content":"two"#);
        assert_eq!(value["items"][0]["content"], "one");
        assert_eq!(value["items"][1]["content"], "two");
    }

    #[test]
    fn drops_dangling_escape_before_closing_open_string() {
        let value = repaired_value(r#"{"content":"abc\"#);
        assert_eq!(value["content"], "abc");
    }

    #[test]
    fn drops_incomplete_unicode_escape_before_closing_open_string() {
        let value = repaired_value(r#"{"command":"cat","content":"ab\u003"#);
        assert_eq!(value["command"], "cat");
        assert_eq!(value["content"], "ab");
    }

    #[test]
    fn strips_then_completes_trailing_tag_inside_open_string() {
        let value = repaired_value(r#"{"command":"cat","content":"hell</parameter>"#);
        assert_eq!(value["command"], "cat");
        assert_eq!(value["content"], "hell");
    }

    #[test]
    fn strips_two_stacked_trailing_tags() {
        let value = repaired_value(r#"{"k":"v"}</parameter></tool_call>"#);
        assert_eq!(value["k"], "v");
    }

    #[test]
    fn does_not_strip_incomplete_opening_tag_content() {
        assert_eq!(repair_tool_args(r#"{"content":"see <b"#, false), None);

        let value = repaired_value(r#"{"content":"see <b"#);
        assert_eq!(value["content"], "see <b");
    }

    #[test]
    fn empty_or_whitespace_returns_none() {
        assert_eq!(repair_tool_args("", true), None);
        assert_eq!(repair_tool_args(" \n\t", true), None);
    }

    #[test]
    fn mismatched_bracket_returns_none() {
        assert_eq!(repair_tool_args(r#"{"foo":[}"#, true), None);
    }

    #[test]
    fn completion_disallowed_does_not_complete_truncated_json() {
        assert_eq!(
            repair_tool_args(r#"{"path":"/tmp/a","content":"hel"#, false),
            None
        );
    }

    #[test]
    fn completion_disallowed_still_strips_complete_trailing_tag() {
        let value = repair_tool_args(r#"{"command":"cat"}</parameter>"#, false)
            .expect("stripping should repair complete JSON");
        assert_eq!(value["command"], "cat");
    }

    #[test]
    fn oversized_input_returns_none() {
        let input = format!("{}x", " ".repeat(MAX_REPAIR_BYTES));
        assert_eq!(repair_tool_args(&input, true), None);
    }

    #[test]
    fn unrepairable_garbage_returns_none() {
        assert_eq!(repair_tool_args("not json", true), None);
    }
}

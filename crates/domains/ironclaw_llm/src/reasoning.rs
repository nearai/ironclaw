//! Model-response text cleanup and textual tool-call recovery.
//!
//! Providers occasionally emit reasoning tags or tool calls as prose in the
//! content field instead of the structured `tool_calls` array. This module
//! strips the former and recovers the latter before the response reaches the
//! runner's model gateway.

use std::sync::LazyLock;

use regex::Regex;

use ironclaw_common::provider_transcript::strip_provider_transcript_artifact_lines;

use crate::ToolCall;

/// Seed value used as the second argument to `generate_tool_call_id` when
/// recovering tool calls from malformed LLM text responses. This must differ
/// from the `0` seed used in `rig_adapter::normalized_tool_call_id` to avoid
/// ID collisions between provider-generated and text-recovered tool calls at
/// the same positional index.
const RECOVERED_TOOL_CALL_SEED: usize = 99;

/// Quick-check: bail early if no reasoning/final tags are present at all.
static QUICK_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<\s*/?\s*(?:think(?:ing)?|thought|thoughts|antthinking|reasoning|reflection|scratchpad|inner_monologue|final)\b").expect("QUICK_TAG_RE") // safety: hardcoded literal
});

/// Matches thinking/reasoning open and close tags. Capture group 1 is "/" for close tags.
/// Whitespace-tolerant, case-insensitive, attribute-aware.
static THINKING_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<\s*(/?)\s*(?:think(?:ing)?|thought|thoughts|antthinking|reasoning|reflection|scratchpad|inner_monologue)\b[^<>]*>").expect("THINKING_TAG_RE") // safety: hardcoded literal
});

/// Matches `<final>` / `</final>` tags. Capture group 1 is "/" for close tags.
static FINAL_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<\s*(/?)\s*final\b[^<>]*>").expect("FINAL_TAG_RE")); // safety: hardcoded literal

/// Matches pipe-delimited reasoning tags: `<|think|>...<|/think|>` etc.
static PIPE_REASONING_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<\|(/?)\s*(?:think(?:ing)?|thought|thoughts|antthinking|reasoning|reflection|scratchpad|inner_monologue)\|>").expect("PIPE_REASONING_TAG_RE") // safety: hardcoded literal
});

/// A byte range in the source text that is inside a code region (fenced or inline).
#[derive(Debug, Clone, Copy)]
struct CodeRegion {
    start: usize,
    end: usize,
}

/// Detect fenced code blocks (``` and ~~~) and inline backtick spans.
/// Returns sorted `Vec<CodeRegion>` of byte ranges. Tags inside these ranges are
/// skipped during stripping so code examples mentioning `<thinking>` are preserved.
fn find_code_regions(text: &str) -> Vec<CodeRegion> {
    let mut regions = Vec::new();

    // Fenced code blocks: line starting with 3+ backticks or tildes
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        // Must be at start of line (i==0 or previous char is \n)
        if i > 0 && bytes[i - 1] != b'\n' {
            if let Some(nl) = text[i..].find('\n') {
                i += nl + 1;
            } else {
                break;
            }
            continue;
        }

        // Skip optional leading whitespace
        let line_start = i;
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }

        let fence_char = if i < bytes.len() && (bytes[i] == b'`' || bytes[i] == b'~') {
            bytes[i]
        } else {
            // Not a fence line, skip to next line
            if let Some(nl) = text[i..].find('\n') {
                i += nl + 1;
            } else {
                break;
            }
            continue;
        };

        // Count fence chars
        let fence_start = i;
        while i < bytes.len() && bytes[i] == fence_char {
            i += 1;
        }
        let fence_len = i - fence_start;
        if fence_len < 3 {
            // Not a real fence
            if let Some(nl) = text[i..].find('\n') {
                i += nl + 1;
            } else {
                break;
            }
            continue;
        }

        // Skip rest of opening fence line (info string)
        if let Some(nl) = text[i..].find('\n') {
            i += nl + 1;
        } else {
            // Fence at EOF with no content — region extends to end
            regions.push(CodeRegion {
                start: line_start,
                end: bytes.len(),
            });
            break;
        }

        // Find closing fence: line starting with >= fence_len of same char
        let content_start = i;
        let mut found_close = false;
        while i < bytes.len() {
            let cl_start = i;
            // Skip optional leading whitespace
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == fence_char {
                let close_fence_start = i;
                while i < bytes.len() && bytes[i] == fence_char {
                    i += 1;
                }
                let close_fence_len = i - close_fence_start;
                // Must be at least as long, and rest of line must be empty/whitespace
                if close_fence_len >= fence_len {
                    // Skip to end of line
                    while i < bytes.len() && bytes[i] != b'\n' {
                        if bytes[i] != b' ' && bytes[i] != b'\t' {
                            break;
                        }
                        i += 1;
                    }
                    if i >= bytes.len() || bytes[i] == b'\n' {
                        if i < bytes.len() {
                            i += 1; // skip the \n
                        }
                        regions.push(CodeRegion {
                            start: line_start,
                            end: i,
                        });
                        found_close = true;
                        break;
                    }
                }
            }
            // Not a closing fence, skip to next line
            if let Some(nl) = text[cl_start..].find('\n') {
                i = cl_start + nl + 1;
            } else {
                i = bytes.len();
                break;
            }
        }
        if !found_close {
            // Unclosed fence extends to EOF
            let _ = content_start; // suppress unused warning
            regions.push(CodeRegion {
                start: line_start,
                end: bytes.len(),
            });
        }
    }

    // Inline backtick spans (not inside fenced blocks)
    let mut j = 0;
    while j < bytes.len() {
        if bytes[j] != b'`' {
            j += 1;
            continue;
        }
        // Inside a fenced block? Skip
        if regions.iter().any(|r| j >= r.start && j < r.end) {
            j += 1;
            continue;
        }
        // Count opening backtick run
        let tick_start = j;
        while j < bytes.len() && bytes[j] == b'`' {
            j += 1;
        }
        let tick_len = j - tick_start;
        // Find matching closing run of exactly tick_len backticks
        let search_from = j;
        let mut found = false;
        let mut k = search_from;
        while k < bytes.len() {
            if bytes[k] != b'`' {
                k += 1;
                continue;
            }
            let close_start = k;
            while k < bytes.len() && bytes[k] == b'`' {
                k += 1;
            }
            if k - close_start == tick_len {
                regions.push(CodeRegion {
                    start: tick_start,
                    end: k,
                });
                j = k;
                found = true;
                break;
            }
        }
        if !found {
            j = tick_start + tick_len; // no match, move past
        }
    }

    regions.sort_by_key(|r| r.start);
    regions
}

/// Check if a byte position falls inside any code region.
fn is_inside_code(pos: usize, regions: &[CodeRegion]) -> bool {
    regions.iter().any(|r| pos >= r.start && pos < r.end)
}

/// Check whether a byte range overlaps any code region.
fn overlaps_code_region(start: usize, end: usize, regions: &[CodeRegion]) -> bool {
    regions.iter().any(|r| start < r.end && end > r.start)
}

/// Return the byte bounds of the line containing `pos`, excluding the trailing newline.
///
/// `pos` is clamped to `text.len()` and adjusted to the nearest char boundary,
/// so callers need not guarantee that `pos` falls on a boundary.
fn line_bounds(text: &str, pos: usize) -> (usize, usize) {
    let pos = pos.min(text.len());
    // Walk backward to find a valid char boundary (at most 3 bytes for UTF-8).
    let mut safe = pos;
    while safe > 0 && !text.is_char_boundary(safe) {
        safe -= 1;
    }
    let start = text[..safe].rfind('\n').map_or(0, |idx| idx + 1);
    let end = text[safe..].find('\n').map_or(text.len(), |idx| safe + idx);
    (start, end)
}

/// Only recover XML-style tool calls when they are isolated content outside
/// markdown code and quote contexts. This avoids converting code examples or
/// quoted snippets into executable tool calls.
fn is_recoverable_tool_call_segment(
    text: &str,
    start: usize,
    end: usize,
    code_regions: &[CodeRegion],
) -> bool {
    if overlaps_code_region(start, end, code_regions) {
        return false;
    }

    let (first_line_start, first_line_end) = line_bounds(text, start);
    let first_line = &text[first_line_start..first_line_end];

    if first_line.trim_start().starts_with('>') {
        return false;
    }

    let (_, last_line_end) = line_bounds(text, end.saturating_sub(1));
    let first_line_prefix = &text[first_line_start..start];
    let last_line_suffix = &text[end..last_line_end];

    if !first_line_prefix.trim().is_empty() || !last_line_suffix.trim().is_empty() {
        return false;
    }

    true
}

pub fn recover_codex_text_tool_calls_from_tool_names(
    content: &str,
    tool_names: &[String],
) -> Vec<ToolCall> {
    let tool_names = tool_names
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    recover_codex_text_tool_calls_from_name_set(content, &tool_names)
}

fn recover_codex_text_tool_calls_from_name_set(
    content: &str,
    tool_names: &std::collections::HashSet<&str>,
) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    recover_codex_text_tool_calls(content, tool_names, &mut calls);
    calls
}

fn recover_codex_text_tool_calls(
    content: &str,
    tool_names: &std::collections::HashSet<&str>,
    calls: &mut Vec<ToolCall>,
) {
    let code_regions = find_code_regions(content);
    let mut search_from = 0;
    while let Some(offset) = content[search_from..].find("to=") {
        let start = search_from + offset;
        let Some((name, arguments, end)) = parse_codex_text_tool_call_at(content, start) else {
            search_from = start + "to=".len();
            continue;
        };
        search_from = end.max(start + 1);

        if !is_recoverable_tool_call_segment(content, start, end, &code_regions) {
            continue;
        }

        if !tool_names.contains(name.as_str()) {
            continue;
        }

        calls.push(ToolCall {
            id: super::provider::generate_tool_call_id(calls.len(), RECOVERED_TOOL_CALL_SEED),
            name,
            arguments,
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
        });
    }
}

pub fn contains_codex_text_tool_call_syntax(content: &str) -> bool {
    let code_regions = find_code_regions(content);
    let mut search_from = 0;
    while let Some(offset) = content[search_from..].find("to=") {
        let start = search_from + offset;
        if let Some((_, _, end)) = parse_codex_text_tool_call_at(content, start)
            && is_recoverable_tool_call_segment(content, start, end, &code_regions)
        {
            return true;
        }
        search_from = start + "to=".len();
    }
    false
}

fn parse_codex_text_tool_call_at(
    content: &str,
    start: usize,
) -> Option<(String, serde_json::Value, usize)> {
    let after_prefix = content.get(start..)?.strip_prefix("to=")?;
    let name_start = start + "to=".len();
    let name_end = after_prefix
        .find(|ch: char| ch.is_whitespace() || ch == '{')
        .map(|relative| name_start + relative)
        .unwrap_or(content.len());
    let name = &content[name_start..name_end];
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return None;
    }

    let separator = &content[name_end..];
    let brace_relative = separator.find('{')?;
    if !separator[..brace_relative].trim_end().ends_with("json") {
        return None;
    }
    let brace_start = name_end + brace_relative;
    let mut json_stream = serde_json::Deserializer::from_str(&content[brace_start..])
        .into_iter::<serde_json::Value>();
    let arguments = json_stream.next()?.ok()?;
    let consumed = json_stream.byte_offset();
    Some((name.to_string(), arguments, brace_start + consumed.max(1)))
}

/// `<tool_call>tool_list</tool_call>` or `<|tool_call|>` in the content field
/// instead of using the standard OpenAI tool_calls array. We strip all of
/// these before the response reaches channels/users.
///
/// Pipeline:
/// 1. Quick-check — bail if no reasoning/final tags
/// 2. Build code regions (fenced blocks + inline backticks)
/// 3. Strip thinking tags (regex, code-aware, strict mode for unclosed)
/// 4. If `<final>` tags present: extract only `<final>` content
///    Else: use the thinking-stripped text as-is
/// 5. Strip pipe-delimited reasoning tags (code-aware)
/// 6. Strip tool tags (string matching — no code-awareness needed)
/// 7. Collapse triple+ newlines, trim
pub fn clean_response(text: &str) -> String {
    // 1. Quick-check
    let mut result = if !QUICK_TAG_RE.is_match(text) {
        text.to_string()
    } else {
        // 2 + 3. Build code regions, strip thinking tags
        let code_regions = find_code_regions(text);
        let after_thinking = strip_thinking_tags_regex(text, &code_regions);

        // 4. If <final> tags present, extract only their content
        if FINAL_TAG_RE.is_match(&after_thinking) {
            let fresh_regions = find_code_regions(&after_thinking);
            extract_final_content(&after_thinking, &fresh_regions).unwrap_or(after_thinking)
        } else {
            after_thinking
        }
    };

    // 5. Strip pipe-delimited reasoning tags (code-aware)
    result = strip_pipe_reasoning_tags(&result);

    // 6. Strip tool tags (string matching, not code-aware)
    for tag in TOOL_TAGS {
        result = strip_xml_tag(&result, tag);
        result = strip_pipe_tag(&result, tag);
    }

    // 6b. Strip legacy bracket-format inline tool calls:
    // [Called tool `name` with arguments: {...}]
    result = strip_bracket_tool_calls(&result);
    result = strip_provider_transcript_artifact_lines(&result);

    // 6c. Strip markdown-fenced tool calls: ```tool_call\n{json}\n```
    // These pass cleanly through the XML/pipe strippers because they
    // use backticks instead of angle brackets, but they're still
    // tool-call syntax that should never reach the user. Recovery
    // (`recover_tool_calls_from_content`) extracts the JSON above; this
    // strips any leftover residue (malformed JSON, repeated emissions,
    // model echo) so the user-visible text is clean.
    for tag in TOOL_TAGS {
        result = strip_markdown_fence_block(&result, tag);
    }

    // 6d. Strip Codex textual tool-call syntax emitted by fallback model
    // paths, e.g. `to=tool.name ...json\n{...}`. Recovery handles valid
    // advertised tools before cleanup; this prevents residual call syntax
    // from becoming user-visible prose.
    result = strip_codex_text_tool_calls(&result);

    // 7. Collapse triple+ newlines, trim
    collapse_newlines(&result)
}

fn strip_codex_text_tool_calls(text: &str) -> String {
    let code_regions = find_code_regions(text);
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("to=") {
        let start = cursor + offset;
        if let Some((_, _, end)) = parse_codex_text_tool_call_at(text, start)
            && is_recoverable_tool_call_segment(text, start, end, &code_regions)
        {
            result.push_str(&text[cursor..start]);
            if result.chars().last().is_some_and(|ch| !ch.is_whitespace())
                && text[end..]
                    .chars()
                    .next()
                    .is_some_and(|ch| !ch.is_whitespace())
            {
                result.push(' ');
            }
            cursor = end;
        } else {
            let consumed = start + "to=".len();
            result.push_str(&text[cursor..consumed]);
            cursor = consumed;
        }
    }
    result.push_str(&text[cursor..]);
    result
}

/// Strip markdown-fenced tool-call blocks like ` ```tool_call\n{...}\n``` `.
///
/// Mirrors the recovery pass in `recover_tool_calls_from_content` so the
/// user-visible text has no fence residue when the LLM emits a markdown
/// fence instead of a structured tool call. Only fences with the exact
/// `tag` info string at line start are removed — inline backtick spans
/// (`` `like this` ``) and other unrelated fenced code stay intact.
fn strip_markdown_fence_block(text: &str, tag: &str) -> String {
    let opening_pat = format!("```{tag}");
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    loop {
        let Some(rel_offset) = remaining.find(&opening_pat) else {
            result.push_str(remaining);
            return result;
        };
        let abs_open = rel_offset;
        // Opening must be at line start (avoid inline backtick spans
        // and code-comment references like ` ```tool_call ` shown
        // inside another fenced block).
        let at_line_start = abs_open == 0
            || remaining[..abs_open]
                .chars()
                .last()
                .is_some_and(|c| c == '\n');
        // Character right after the tag must be whitespace/newline so
        // we don't accidentally match `tool_callX`.
        let after_tag = &remaining[abs_open + opening_pat.len()..];
        let valid_terminator = after_tag
            .chars()
            .next()
            .is_none_or(|c| c == '\n' || c.is_whitespace());
        if !at_line_start || !valid_terminator {
            // Skip past this false match and keep scanning.
            let consumed = abs_open + opening_pat.len();
            result.push_str(&remaining[..consumed]);
            remaining = &remaining[consumed..];
            continue;
        }

        // Push everything before the fence opener (including the
        // newline that put us at line start) so we don't leave a
        // stray blank line.
        let trim_to = remaining[..abs_open].trim_end_matches('\n').len();
        result.push_str(&remaining[..trim_to]);

        // Walk forward to the closing fence line.
        let body_start = match after_tag.find('\n') {
            Some(nl) => abs_open + opening_pat.len() + nl + 1,
            None => {
                // Unterminated opener; drop the rest.
                return result;
            }
        };
        let close_search = &remaining[body_start..];
        let mut idx = 0usize;
        let mut consumed_to = remaining.len();
        while idx <= close_search.len() {
            let line_start = idx;
            let line_end = close_search[idx..]
                .find('\n')
                .map(|n| idx + n)
                .unwrap_or(close_search.len());
            let line = &close_search[line_start..line_end];
            if line.trim_start().starts_with("```") {
                // Skip past the closing fence's trailing newline (if
                // any) so the next chunk starts cleanly.
                consumed_to = body_start + line_end + usize::from(line_end < close_search.len());
                break;
            }
            if line_end == close_search.len() {
                // Reached EOF without a closing fence — drop the rest.
                return result;
            }
            idx = line_end + 1;
        }
        remaining = &remaining[consumed_to..];
    }
}

/// Strip legacy bracket-format inline tool calls.
///
/// Removes patterns like `[Called tool `name` with arguments: {...}]` from text
/// so old transcript artifacts or model echoes do not reach the user. New
/// provider flattening code must not generate this format.
fn strip_bracket_tool_calls(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("[Called tool `") {
        result.push_str(&remaining[..start]);
        let after = &remaining[start..];
        // Find the closing "]" for this bracket expression
        if let Some(end) = after.find("]\n").map(|i| i + 2).or_else(|| {
            // If it's at the end of the string, just find "]"
            after.rfind(']').map(|i| i + 1)
        }) {
            remaining = &after[end..];
        } else {
            // Malformed — keep the rest
            result.push_str(after);
            return result;
        }
    }
    result.push_str(remaining);
    result
}

/// Tool-related tags stripped with simple string matching (no code-awareness needed).
const TOOL_TAGS: &[&str] = &["tool_call", "function_call", "tool_calls"];

/// Strip thinking/reasoning tags using regex, respecting code regions.
///
/// Strict mode: an unclosed opening tag discards all trailing text after it.
fn strip_thinking_tags_regex(text: &str, code_regions: &[CodeRegion]) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_index = 0;
    let mut in_thinking = false;

    for m in THINKING_TAG_RE.find_iter(text) {
        let idx = m.start();

        if is_inside_code(idx, code_regions) {
            continue;
        }

        // Check if this is a close tag by looking at capture group
        let caps = THINKING_TAG_RE.captures(&text[idx..]);
        let is_close = caps
            .and_then(|c| c.get(1))
            .is_some_and(|g| g.as_str() == "/");

        if !in_thinking {
            // Append text before this tag
            result.push_str(&text[last_index..idx]);
            if !is_close {
                in_thinking = true;
            }
        } else if is_close {
            in_thinking = false;
        }

        last_index = m.end();
    }

    // Strict mode: if still inside an unclosed thinking tag, discard trailing text
    // BUT preserve any <final> block embedded in the discarded region
    if !in_thinking {
        result.push_str(&text[last_index..]);
    } else {
        let trailing = &text[last_index..];
        let trailing_regions = find_code_regions(trailing);
        if let Some(final_content) = extract_final_content(trailing, &trailing_regions) {
            result.push_str(&final_content);
        }
    }

    result
}

/// Extract content inside `<final>` tags. Returns `None` if no non-code `<final>` tags found.
///
/// When `<final>` tags are present, ONLY content inside them reaches the user.
/// This discards any untagged reasoning that leaked outside `<think>` tags.
fn extract_final_content(text: &str, code_regions: &[CodeRegion]) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    let mut in_final = false;
    let mut last_index = 0;
    let mut found_any = false;

    for m in FINAL_TAG_RE.find_iter(text) {
        let idx = m.start();

        if is_inside_code(idx, code_regions) {
            continue;
        }

        let caps = FINAL_TAG_RE.captures(&text[idx..]);
        let is_close = caps
            .and_then(|c| c.get(1))
            .is_some_and(|g| g.as_str() == "/");

        if !in_final && !is_close {
            // Opening <final>
            in_final = true;
            found_any = true;
            last_index = m.end();
        } else if in_final && is_close {
            // Closing </final>
            parts.push(&text[last_index..idx]);
            in_final = false;
            last_index = m.end();
        }
    }

    if !found_any {
        return None;
    }

    // Unclosed <final> — include trailing content
    if in_final {
        parts.push(&text[last_index..]);
    }

    Some(parts.join(""))
}

/// Strip pipe-delimited reasoning tags, respecting code regions.
fn strip_pipe_reasoning_tags(text: &str) -> String {
    if !PIPE_REASONING_TAG_RE.is_match(text) {
        return text.to_string();
    }

    let code_regions = find_code_regions(text);
    let mut result = String::with_capacity(text.len());
    let mut last_index = 0;
    let mut in_tag = false;

    for m in PIPE_REASONING_TAG_RE.find_iter(text) {
        let idx = m.start();

        if is_inside_code(idx, &code_regions) {
            continue;
        }

        let caps = PIPE_REASONING_TAG_RE.captures(&text[idx..]);
        let is_close = caps
            .and_then(|c| c.get(1))
            .is_some_and(|g| g.as_str() == "/");

        if !in_tag {
            result.push_str(&text[last_index..idx]);
            if !is_close {
                in_tag = true;
            }
        } else if is_close {
            in_tag = false;
        }

        last_index = m.end();
    }

    if !in_tag {
        result.push_str(&text[last_index..]);
    }

    result
}

/// Strip `<tag>...</tag>` and `<tag ...>...</tag>` blocks from text.
/// Used for tool tags only (no code-awareness needed).
fn strip_xml_tag(text: &str, tag: &str) -> String {
    let open_exact = format!("<{}>", tag);
    let open_prefix = format!("<{} ", tag); // for <tag attr="...">
    let close = format!("</{}>", tag);

    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    loop {
        // Find the next opening tag (exact or with attributes)
        let exact_pos = remaining.find(&open_exact);
        let prefix_pos = remaining.find(&open_prefix);
        let start = match (exact_pos, prefix_pos) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };

        // Add everything before the tag
        result.push_str(&remaining[..start]);

        // Find the end of the opening tag (the closing >)
        let after_open = &remaining[start..];
        let open_end = match after_open.find('>') {
            Some(pos) => start + pos + 1,
            None => break, // malformed, stop
        };

        // Find the closing tag
        if let Some(close_offset) = remaining[open_end..].find(&close) {
            let end = open_end + close_offset + close.len();
            remaining = &remaining[end..];
        } else {
            // No closing tag, discard from here (malformed)
            remaining = "";
            break;
        }
    }

    result.push_str(remaining);
    result
}

/// Strip `<|tag|>...<|/tag|>` pipe-delimited blocks from text.
/// Used for tool tags only (no code-awareness needed).
fn strip_pipe_tag(text: &str, tag: &str) -> String {
    let open = format!("<|{}|>", tag);
    let close = format!("<|/{}|>", tag);

    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find(&open) {
        result.push_str(&remaining[..start]);

        if let Some(close_offset) = remaining[start..].find(&close) {
            let end = start + close_offset + close.len();
            remaining = &remaining[end..];
        } else {
            remaining = "";
            break;
        }
    }

    result.push_str(remaining);
    result
}

/// Collapse triple+ newlines to double, then trim.
fn collapse_newlines(text: &str) -> String {
    let mut result = text.to_string();
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Basic thinking tag stripping ----

    #[test]
    fn test_strip_thinking_tags_basic() {
        let input = "<thinking>Let me think about this...</thinking>Hello, user!";
        assert_eq!(clean_response(input), "Hello, user!");
    }

    #[test]
    fn test_strip_thinking_tags_multiple() {
        let input =
            "<thinking>First thought</thinking>Hello<thinking>Second thought</thinking> world!";
        assert_eq!(clean_response(input), "Hello world!");
    }

    #[test]
    fn test_strip_thinking_tags_multiline() {
        let input = "<thinking>\nI need to consider:\n1. What the user wants\n2. How to respond\n</thinking>\nHere is my response to your question.";
        assert_eq!(
            clean_response(input),
            "Here is my response to your question."
        );
    }

    #[test]
    fn test_strip_thinking_tags_no_tags() {
        let input = "Just a normal response without thinking tags.";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_strip_thinking_tags_unclosed() {
        // Strict mode: unclosed tag discards trailing text
        let input = "Hello <thinking>this never closes";
        assert_eq!(clean_response(input), "Hello");
    }

    // ---- Different tag names ----

    #[test]
    fn test_strip_think_tags() {
        let input = "<think>Let me reason about this...</think>The answer is 42.";
        assert_eq!(clean_response(input), "The answer is 42.");
    }

    #[test]
    fn test_strip_thought_tags() {
        let input = "<thought>The user wants X.</thought>Sure, here you go.";
        assert_eq!(clean_response(input), "Sure, here you go.");
    }

    #[test]
    fn test_strip_thoughts_tags() {
        let input = "<thoughts>Multiple thoughts...</thoughts>Result.";
        assert_eq!(clean_response(input), "Result.");
    }

    #[test]
    fn test_strip_reasoning_tags() {
        let input = "<reasoning>Analyzing the request...</reasoning>\n\nHere's what I found.";
        assert_eq!(clean_response(input), "Here's what I found.");
    }

    #[test]
    fn test_strip_reflection_tags() {
        let input = "<reflection>Am I answering correctly? Yes.</reflection>The capital is Paris.";
        assert_eq!(clean_response(input), "The capital is Paris.");
    }

    #[test]
    fn test_strip_scratchpad_tags() {
        let input =
            "<scratchpad>Step 1: check memory\nStep 2: respond</scratchpad>\n\nI found the answer.";
        assert_eq!(clean_response(input), "I found the answer.");
    }

    #[test]
    fn test_strip_inner_monologue_tags() {
        let input = "<inner_monologue>Processing query...</inner_monologue>Done!";
        assert_eq!(clean_response(input), "Done!");
    }

    #[test]
    fn test_strip_antthinking_tags() {
        let input = "<antthinking>Claude reasoning here</antthinking>Visible answer.";
        assert_eq!(clean_response(input), "Visible answer.");
    }

    // ---- Regex flexibility: whitespace, case, attributes ----

    #[test]
    fn test_whitespace_in_tags() {
        let input = "< think >reasoning</ think >Answer.";
        assert_eq!(clean_response(input), "Answer.");
    }

    #[test]
    fn test_case_insensitive_tags() {
        let input = "<THINKING>Upper case reasoning</THINKING>Visible.";
        assert_eq!(clean_response(input), "Visible.");
    }

    #[test]
    fn test_mixed_case_tags() {
        let input = "<Think>Mixed case</Think>Output.";
        assert_eq!(clean_response(input), "Output.");
    }

    #[test]
    fn test_tags_with_attributes() {
        let input = "<thinking type=\"deep\" level=\"3\">reasoning</thinking>Answer.";
        assert_eq!(clean_response(input), "Answer.");
    }

    // ---- Tool call tags ----

    #[test]
    fn test_strip_tool_call_tags() {
        let input = "<tool_call>tool_list</tool_call>";
        assert_eq!(clean_response(input), "");
    }

    #[test]
    fn test_strip_tool_call_with_surrounding_text() {
        let input = "Here is my answer.\n\n<tool_call>\n{\"name\": \"search\", \"arguments\": {}}\n</tool_call>";
        assert_eq!(clean_response(input), "Here is my answer.");
    }

    #[test]
    fn test_strip_function_call_tags() {
        let input = "Response text<function_call>{\"name\": \"foo\"}</function_call>";
        assert_eq!(clean_response(input), "Response text");
    }

    #[test]
    fn test_strip_tool_calls_plural() {
        let input = "<tool_calls>[{\"id\": \"1\"}]</tool_calls>Actual response.";
        assert_eq!(clean_response(input), "Actual response.");
    }

    #[test]
    fn test_strip_xml_tag_with_attributes() {
        let input = "<tool_call type=\"function\">search()</tool_call>Done.";
        assert_eq!(clean_response(input), "Done.");
    }

    // ---- Pipe-delimited tags ----

    #[test]
    fn test_strip_pipe_delimited_tags() {
        let input = "<|tool_call|>{\"name\": \"search\"}<|/tool_call|>Hello!";
        assert_eq!(clean_response(input), "Hello!");
    }

    #[test]
    fn test_strip_pipe_delimited_thinking() {
        let input = "<|thinking|>reasoning here<|/thinking|>The answer is 42.";
        assert_eq!(clean_response(input), "The answer is 42.");
    }

    #[test]
    fn test_strip_pipe_delimited_think() {
        let input = "<|think|>reasoning here<|/think|>The answer is 42.";
        assert_eq!(clean_response(input), "The answer is 42.");
    }

    // ---- Mixed tags ----

    #[test]
    fn test_strip_multiple_internal_tags() {
        let input = "<thinking>Let me think</thinking>Hello!\n<tool_call>some_tool</tool_call>";
        assert_eq!(clean_response(input), "Hello!");
    }

    #[test]
    fn test_strip_multiple_reasoning_tag_types() {
        let input = "<think>Initial analysis</think>Intermediate.\n<reflection>Double-check</reflection>Final answer.";
        assert_eq!(clean_response(input), "Intermediate.\nFinal answer.");
    }

    #[test]
    fn test_clean_response_preserves_normal_content() {
        let input = "The function tool_call_handler works great. No tags here!";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_clean_response_thinking_tags_with_trailing_text() {
        let input = "<thinking>Internal thought</thinking>Some text.\n\nHere's the answer.";
        assert_eq!(clean_response(input), "Some text.\n\nHere's the answer.");
    }

    #[test]
    fn test_clean_response_thinking_tags_reasoning_properly_tagged() {
        let input = "<thinking>The user is asking about my name.</thinking>\n\nI'm IronClaw, a secure personal AI assistant.";
        assert_eq!(
            clean_response(input),
            "I'm IronClaw, a secure personal AI assistant."
        );
    }

    // ---- Code-awareness: tags inside code blocks are preserved ----

    #[test]
    fn test_tags_in_fenced_code_block_preserved() {
        let input =
            "Here is an example:\n\n```\n<thinking>This is inside code</thinking>\n```\n\nDone.";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_tags_in_tilde_fenced_block_preserved() {
        let input = "Example:\n\n~~~\n<think>code example</think>\n~~~\n\nEnd.";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_tags_in_inline_backticks_preserved() {
        let input = "Use the `<thinking>` tag for reasoning.";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_mixed_real_and_code_tags() {
        let input = "<thinking>real reasoning</thinking>Use `<thinking>` tags.\n\n```\n<thinking>code example</thinking>\n```";
        let expected = "Use `<thinking>` tags.\n\n```\n<thinking>code example</thinking>\n```";
        assert_eq!(clean_response(input), expected);
    }

    #[test]
    fn test_code_block_with_info_string() {
        let input = "```xml\n<thinking>xml example</thinking>\n```\nVisible.";
        assert_eq!(clean_response(input), input);
    }

    // ---- <final> tag extraction ----

    #[test]
    fn test_final_tag_basic() {
        let input = "<think>reasoning</think><final>answer</final>";
        assert_eq!(clean_response(input), "answer");
    }

    #[test]
    fn test_final_tag_strips_untagged_reasoning() {
        let input = "Untagged reasoning.\n<final>answer</final>";
        assert_eq!(clean_response(input), "answer");
    }

    #[test]
    fn test_final_tag_multiple_blocks() {
        let input =
            "<think>part 1</think><final>Hello </final><think>part 2</think><final>world!</final>";
        assert_eq!(clean_response(input), "Hello world!");
    }

    #[test]
    fn test_no_final_tag_fallthrough() {
        // Without <final>, thinking-stripped text returned as-is
        let input = "<think>reasoning</think>Just the answer.";
        assert_eq!(clean_response(input), "Just the answer.");
    }

    #[test]
    fn test_no_tags_at_all() {
        let input = "Just a normal response";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_final_tag_in_code_preserved() {
        // <final> inside code block should not trigger extraction
        let input = "Use `<final>` to mark output.\n\nHello.";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_final_tag_unclosed_includes_trailing() {
        let input = "<think>reasoning</think><final>answer continues";
        assert_eq!(clean_response(input), "answer continues");
    }

    // ---- Unicode content ----

    #[test]
    fn test_unicode_content_preserved() {
        let input = "<thinking>日本語の推論</thinking>こんにちは世界！";
        assert_eq!(clean_response(input), "こんにちは世界！");
    }

    #[test]
    fn test_unicode_in_final() {
        let input = "<think>推論</think><final>答え：42</final>";
        assert_eq!(clean_response(input), "答え：42");
    }

    // ---- Newline collapsing ----

    #[test]
    fn test_collapse_triple_newlines() {
        let input = "<thinking>removed</thinking>\n\n\nVisible.";
        assert_eq!(clean_response(input), "Visible.");
    }

    #[test]
    fn test_trims_whitespace() {
        let input = "  <thinking>removed</thinking>  Hello, user!  \n";
        assert_eq!(clean_response(input), "Hello, user!");
    }

    // ---- Code region detection ----

    #[test]
    fn test_find_code_regions_fenced() {
        let text = "before\n```\ncode\n```\nafter";
        let regions = find_code_regions(text);
        assert_eq!(regions.len(), 1);
        assert!(text[regions[0].start..regions[0].end].contains("code"));
    }

    #[test]
    fn test_find_code_regions_inline() {
        let text = "Use `<thinking>` tag.";
        let regions = find_code_regions(text);
        assert_eq!(regions.len(), 1);
        assert!(text[regions[0].start..regions[0].end].contains("<thinking>"));
    }

    #[test]
    fn test_find_code_regions_unclosed_fence() {
        let text = "before\n```\ncode goes on\nno closing fence";
        let regions = find_code_regions(text);
        assert_eq!(regions.len(), 1);
        // Unclosed fence extends to EOF
        assert_eq!(regions[0].end, text.len());
    }

    // ---- line_bounds UTF-8 safety (issue #1669) ----

    #[test]
    fn test_line_bounds_ascii() {
        let text = "hello\nworld\n";
        assert_eq!(line_bounds(text, 0), (0, 5));
        assert_eq!(line_bounds(text, 6), (6, 11));
    }

    #[test]
    fn test_line_bounds_at_text_len() {
        let text = "abc";
        assert_eq!(line_bounds(text, 3), (0, 3));
    }

    #[test]
    fn test_line_bounds_mid_multibyte_char() {
        // '🔥' is 4 bytes (F0 9F 94 A5). Passing pos=1 lands inside the char.
        // line_bounds must not panic — it should snap to a valid boundary.
        let text = "🔥\n<tool_call>";
        // All mid-char positions should snap back to byte 0 (start of '🔥'),
        // so line bounds cover the first line: "🔥" = bytes 0..4.
        assert_eq!(line_bounds(text, 1), (0, 4)); // would panic before fix
        assert_eq!(line_bounds(text, 2), (0, 4));
        assert_eq!(line_bounds(text, 3), (0, 4));
    }

    #[test]
    fn test_line_bounds_emoji_before_newline() {
        // 'Result: 🔥\n<tool_call>' — end.saturating_sub(1) from the \n position
        // should not panic even with multi-byte chars on the same line.
        let text = "Result: 🔥\n<tool_call>";
        let newline_pos = text.find('\n').unwrap();
        // saturating_sub(1) lands inside '🔥' (byte 11 → 10, but char ends at 12).
        // Snaps back to byte 8 (start of '🔥'), line covers "Result: 🔥" = bytes 0..12.
        assert_eq!(line_bounds(text, newline_pos.saturating_sub(1)), (0, 12));
    }

    #[test]
    fn test_line_bounds_pos_beyond_len() {
        let text = "abc";
        // pos > text.len() should be clamped, not panic
        assert_eq!(line_bounds(text, 100), (0, 3));
    }

    /// `clean_response` must strip a markdown-fenced tool-call block
    /// even when the JSON inside is malformed (so recovery would have
    /// skipped it). Otherwise the fence syntax leaks to the user.
    #[test]
    fn test_clean_response_strips_markdown_fenced_tool_call() {
        let input =
            "Here you go:\n\n```tool_call\n{\"name\":\"get_balances\",\"arguments\":{}}\n```\n";
        let cleaned = clean_response(input);
        assert!(
            !cleaned.contains("```"),
            "markdown fence must be stripped: {cleaned:?}"
        );
        assert!(
            !cleaned.contains("get_balances"),
            "JSON body inside fence must be stripped: {cleaned:?}"
        );
        assert!(
            cleaned.contains("Here you go"),
            "prose outside the fence must remain: {cleaned:?}"
        );
    }

    #[test]
    fn test_clean_response_strips_malformed_markdown_fence() {
        // Even malformed JSON inside the fence must be stripped —
        // recovery skips it but the user must not see it.
        let input = "Reply\n```tool_call\nNOT JSON\n```\n";
        let cleaned = clean_response(input);
        assert!(!cleaned.contains("```"), "fence must go: {cleaned:?}");
        assert!(!cleaned.contains("NOT JSON"), "body must go: {cleaned:?}");
    }

    // ---- Unclosed think before final (Bug #564-3) ----

    #[test]
    fn test_unclosed_think_before_final() {
        assert_eq!(
            clean_response("<think>reasoning no close tag <final>actual answer</final>"),
            "actual answer"
        );
    }

    #[test]
    fn test_unclosed_thinking_before_final() {
        assert_eq!(
            clean_response("<thinking>long reasoning... <final>the real answer</final>"),
            "the real answer"
        );
    }

    #[test]
    fn test_unclosed_think_before_final_with_prefix() {
        assert_eq!(
            clean_response("Hello <think>reasoning <final>world</final>"),
            "Hello world"
        );
    }

    #[test]
    fn test_unclosed_think_no_final_still_discards() {
        assert_eq!(clean_response("Hello <thinking>this never closes"), "Hello");
    }

    #[test]
    fn test_clean_response_strips_bracket_tool_calls() {
        let input = "Let me fetch that.\n[Called tool `http` with arguments: {\"method\":\"GET\",\"url\":\"https://example.com\"}]\nHere are the results.";
        let cleaned = clean_response(input);
        assert!(!cleaned.contains("[Called tool"));
        assert!(cleaned.contains("Let me fetch that."));
        assert!(cleaned.contains("Here are the results."));
    }

    #[test]
    fn test_clean_response_strips_codex_text_tool_calls() {
        let input = "Searching now.\nto=notion.notion-search weirdjson\n{\"query\":\"\",\"query_type\":\"internal\"}\nI am waiting for results.";
        let cleaned = clean_response(input);
        assert!(!cleaned.contains("to=notion.notion-search"));
        assert!(!cleaned.contains("\"query_type\""));
        assert!(cleaned.contains("Searching now."));
        assert!(cleaned.contains("I am waiting for results."));
    }

    #[test]
    fn test_clean_response_strips_flattened_tool_history_lines() {
        let input = "Done.\nPrevious tool event: demo__echo was invoked.\nPrevious tool result from demo__echo: hi\nTool result from demo__echo: hi";
        assert_eq!(clean_response(input), "Done.");
    }

    #[test]
    fn test_clean_response_strips_replay_only_flattened_tool_history_to_empty() {
        let input = "Previous tool event: demo__echo was invoked.";
        assert_eq!(clean_response(input), "");
    }

    #[test]
    fn test_clean_response_strips_multiline_replay_only_flattened_tool_history_to_empty() {
        let input = "Previous tool event: demo__echo was invoked.\nTool result from demo__echo: ok";
        assert_eq!(clean_response(input), "");
    }

    // ---- Issue #789: OpenAI reasoning models negative test ----

    #[test]
    fn test_openai_reasoning_models_not_detected() {
        use crate::reasoning_models::has_native_thinking;
        assert!(!has_native_thinking("o1"));
        assert!(!has_native_thinking("o1-mini"));
        assert!(!has_native_thinking("o1-preview"));
        assert!(!has_native_thinking("o3-mini"));
        assert!(!has_native_thinking("o4-mini"));
    }
}

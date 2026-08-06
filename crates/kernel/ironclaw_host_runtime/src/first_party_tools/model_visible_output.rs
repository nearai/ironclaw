//! Shared helpers for shaping first-party tool results before they enter model context.

use serde_json::Value;

pub(super) fn serialized_json_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |serialized| serialized.len())
}

pub(super) fn truncate_string_for_json_content_budget(
    value: String,
    max_serialized_content_bytes: usize,
) -> (String, bool) {
    let (truncated, was_truncated) =
        truncate_str_for_json_content_budget(&value, max_serialized_content_bytes);
    if !was_truncated {
        return (value, false);
    }
    (truncated.to_string(), true)
}

pub(super) fn truncate_str_for_json_content_budget(
    value: &str,
    max_serialized_content_bytes: usize,
) -> (&str, bool) {
    let mut used = 0_usize;
    for (index, character) in value.char_indices() {
        let next = used.saturating_add(json_escaped_character_len(character));
        if next > max_serialized_content_bytes {
            return (&value[..index], true);
        }
        used = next;
    }
    (value, false)
}

pub(super) fn serialized_json_content_len(value: &str) -> usize {
    value.chars().map(json_escaped_character_len).sum()
}

pub(super) fn max_binary_bytes_for_base64_budget(max_serialized_content_bytes: usize) -> usize {
    max_serialized_content_bytes / 4 * 3
}

fn json_escaped_character_len(character: char) -> usize {
    match character {
        '"' | '\\' => 2,
        '\u{08}' | '\t' | '\n' | '\u{0c}' | '\r' => 2,
        '\u{00}'..='\u{1f}' => 6,
        _ => character.len_utf8(),
    }
}

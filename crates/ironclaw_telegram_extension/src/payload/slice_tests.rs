// Unit tests for the private text-slicing helpers; child module of
// `payload` so `use super::*` reaches crate-private items.
use super::*;

#[test]
fn zero_length_slice_at_offset_zero_returns_empty() {
    assert_eq!(slice_text_by_offset("", 0, 0), Some(""));
    assert_eq!(slice_text_by_offset("hello", 0, 0), Some(""));
}

#[test]
fn full_string_slice() {
    assert_eq!(slice_text_by_offset("hello", 0, 5), Some("hello"));
}

#[test]
fn slice_at_end_zero_length() {
    assert_eq!(slice_text_by_offset("hello", 5, 0), Some(""));
}

#[test]
fn slice_past_end_returns_none() {
    assert_eq!(slice_text_by_offset("hello", 6, 0), None);
    assert_eq!(slice_text_by_offset("hello", 5, 1), None);
}

#[test]
fn multibyte_slice_respects_utf16_offsets() {
    // "🦀" is 1 char, 2 UTF-16 code units, 4 bytes in UTF-8.
    let text = "ab🦀cd";
    // Slice "🦀" => offset 2 (after "ab"), length 2 (one surrogate pair).
    assert_eq!(slice_text_by_offset(text, 2, 2), Some("🦀"));
    // Slice the whole string.
    assert_eq!(slice_text_by_offset(text, 0, 6), Some("ab🦀cd"));
}

#[test]
fn slice_to_end_handles_empty_text() {
    assert_eq!(slice_text_to_end("", 0), Some(""));
}

#[test]
fn slice_to_end_at_string_end() {
    assert_eq!(slice_text_to_end("hello", 5), Some(""));
}

#[test]
fn slice_to_end_past_string_returns_none() {
    assert_eq!(slice_text_to_end("hello", 6), None);
}

#[test]
fn slice_to_end_basic() {
    assert_eq!(slice_text_to_end("hello world", 6), Some("world"));
}

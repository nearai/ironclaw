use std::cell::OnceCell;

use unicode_normalization::UnicodeNormalization;

use super::CodingCapabilityError;

use super::{
    operation_error,
    types::{FileEncoding, LineEnding, MatchMethod},
};

pub(super) fn reject_binary_probe(bytes: &[u8]) -> Result<(), CodingCapabilityError> {
    if detect_encoding(bytes) == FileEncoding::Utf16Le {
        return Ok(());
    }
    let probe_len = bytes.len().min(8192);
    if bytes[..probe_len].contains(&0) {
        return Err(operation_error());
    }
    Ok(())
}

pub(super) fn decode_text(
    bytes: &[u8],
) -> Result<(String, FileEncoding, LineEnding), CodingCapabilityError> {
    let encoding = detect_encoding(bytes);
    let raw = match encoding {
        FileEncoding::Utf8 => String::from_utf8(bytes.to_vec()).map_err(|_| operation_error())?,
        FileEncoding::Utf16Le => {
            let data = bytes.get(2..).unwrap_or_default();
            let units = data
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>();
            String::from_utf16(&units).map_err(|_| operation_error())?
        }
    };
    let line_ending = detect_line_ending(&raw);
    Ok((
        raw.replace("\r\n", "\n").replace('\r', "\n"),
        encoding,
        line_ending,
    ))
}

/// Lenient binary probe for the **read** path only. Unlike [`reject_binary_probe`]
/// (which rejects on a single NUL — correct for the patch path where exact byte
/// fidelity matters), this rejects only when NUL bytes are *dense* in the probe
/// window. Real binaries (images, executables) are NUL-heavy; a text log may
/// carry a few stray NULs and should still be readable. Pairs with
/// [`decode_text_lossy`] so logs with occasional non-UTF-8 bytes decode instead
/// of hard-failing read_file (pinchbench syslog tasks).
pub(super) fn reject_binary_probe_lenient(bytes: &[u8]) -> Result<(), CodingCapabilityError> {
    if detect_encoding(bytes) == FileEncoding::Utf16Le {
        return Ok(());
    }
    let probe = &bytes[..bytes.len().min(8192)];
    if probe.is_empty() {
        return Ok(());
    }
    let nul_count = probe.iter().filter(|&&byte| byte == 0).count();
    // Reject only when NULs are both numerous (absolute floor, so a small log with
    // one stray NUL isn't condemned by the ratio) AND dense (>~1% of the probe).
    // Genuine binaries clear both bars; text logs with a few stray NULs pass.
    const MIN_NUL_FLOOR: usize = 8;
    if nul_count > MIN_NUL_FLOOR && nul_count.saturating_mul(100) > probe.len() {
        return Err(operation_error());
    }
    Ok(())
}

/// Lossy decode for the **read** path only. Mirrors [`decode_text`] but replaces
/// invalid UTF-8 / UTF-16 sequences with U+FFFD instead of failing, so a log with
/// a Latin-1 byte or a truncated multibyte sequence is still readable. Never use
/// this for the patch path: a lossy round-trip through `encode_text` would corrupt
/// the file on write-back.
pub(super) fn decode_text_lossy(bytes: &[u8]) -> (String, FileEncoding, LineEnding) {
    let encoding = detect_encoding(bytes);
    let raw = match encoding {
        FileEncoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        FileEncoding::Utf16Le => {
            let data = bytes.get(2..).unwrap_or_default();
            let units = data
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>();
            String::from_utf16_lossy(&units)
        }
    };
    let line_ending = detect_line_ending(&raw);
    (
        raw.replace("\r\n", "\n").replace('\r', "\n"),
        encoding,
        line_ending,
    )
}

pub(super) fn encode_text(
    content: &str,
    encoding: FileEncoding,
    line_ending: LineEnding,
) -> Vec<u8> {
    let output = match line_ending {
        LineEnding::Lf => content.to_string(),
        LineEnding::CrLf => content.replace('\n', "\r\n"),
        LineEnding::Cr => content.replace('\n', "\r"),
    };
    match encoding {
        FileEncoding::Utf8 => output.into_bytes(),
        FileEncoding::Utf16Le => {
            let mut bytes = vec![0xFF, 0xFE];
            for unit in output.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        }
    }
}

fn detect_encoding(bytes: &[u8]) -> FileEncoding {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        FileEncoding::Utf16Le
    } else {
        FileEncoding::Utf8
    }
}

fn detect_line_ending(content: &str) -> LineEnding {
    let crlf = content.matches("\r\n").count();
    let cr_only = content.matches('\r').count().saturating_sub(crlf);
    let lf_only = content.matches('\n').count().saturating_sub(crlf);
    if crlf >= lf_only && crlf >= cr_only {
        if crlf == 0 {
            LineEnding::Lf
        } else {
            LineEnding::CrLf
        }
    } else if cr_only > lf_only {
        LineEnding::Cr
    } else {
        LineEnding::Lf
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TextEdit<'a> {
    pub(super) old_string: &'a str,
    pub(super) new_string: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReplaceContentError {
    EmptyOld,
    InvalidEditCount,
    NotFound {
        edit_index: usize,
    },
    Duplicate {
        edit_index: usize,
        occurrences: usize,
    },
    Overlap {
        previous_edit_index: usize,
        current_edit_index: usize,
    },
    NoChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReplaceContentOutcome {
    pub(super) content: String,
    pub(super) replacements: usize,
    pub(super) match_method: MatchMethod,
}

#[derive(Debug, Clone, Copy)]
struct MatchSpan {
    start: usize,
    end: usize,
    method: MatchMethod,
}

#[derive(Debug)]
struct EditMatches {
    spans: Vec<MatchSpan>,
    occurrence_count: usize,
}

#[derive(Debug)]
struct MatchedEdit {
    edit_index: usize,
    start: usize,
    end: usize,
    new_string: String,
}

#[derive(Debug)]
struct NormalizedText {
    text: String,
    spans: Vec<SourceSpan>,
}

#[derive(Debug, Clone, Copy)]
struct SourceSpan {
    start: usize,
    end: usize,
}

struct TextMatcher<'a> {
    content: &'a str,
    normalized_content: OnceCell<NormalizedText>,
}

impl<'a> TextMatcher<'a> {
    fn new(content: &'a str) -> Self {
        Self {
            content,
            normalized_content: OnceCell::new(),
        }
    }

    fn find_edit_matches(&self, old_string: &str, replace_all: bool) -> EditMatches {
        let match_limit = if replace_all { None } else { Some(2) };
        let exact_spans = find_exact_spans(self.content, old_string, match_limit)
            .into_iter()
            .map(|(start, end)| MatchSpan {
                start,
                end,
                method: MatchMethod::Exact,
            })
            .collect::<Vec<_>>();
        if !exact_spans.is_empty() {
            if old_string.trim().is_empty()
                || !replace_all && exact_spans.len() > 1
                || !content_may_need_fuzzy_normalization(self.content)
            {
                return EditMatches {
                    occurrence_count: exact_spans.len(),
                    spans: exact_spans,
                };
            }

            let fuzzy_spans = self.find_fuzzy_spans(old_string, match_limit);
            let occurrence_count = if fuzzy_spans.is_empty() {
                exact_spans.len()
            } else {
                fuzzy_spans.len().max(exact_spans.len())
            };
            return EditMatches {
                spans: merge_exact_and_fuzzy_spans(exact_spans, fuzzy_spans),
                occurrence_count,
            };
        }

        let fuzzy_spans = self.find_fuzzy_spans(old_string, match_limit);
        EditMatches {
            occurrence_count: fuzzy_spans.len(),
            spans: fuzzy_spans,
        }
    }

    fn find_fuzzy_spans(&self, old_string: &str, match_limit: Option<usize>) -> Vec<MatchSpan> {
        if old_string.trim().is_empty() {
            return Vec::new();
        }

        let normalized_old = normalize_for_fuzzy_match(old_string);
        if normalized_old.is_empty() {
            return Vec::new();
        }

        let normalized_content = self
            .normalized_content
            .get_or_init(|| normalize_with_source_map(self.content));
        find_exact_spans(&normalized_content.text, &normalized_old, match_limit)
            .into_iter()
            .filter_map(|(start, end)| {
                normalized_span_to_source_span(normalized_content, start, end).map(|span| {
                    MatchSpan {
                        start: span.start,
                        end: span.end,
                        method: MatchMethod::FuzzyNormalization,
                    }
                })
            })
            .collect()
    }
}

fn merge_exact_and_fuzzy_spans(
    mut exact_spans: Vec<MatchSpan>,
    fuzzy_spans: Vec<MatchSpan>,
) -> Vec<MatchSpan> {
    for fuzzy in fuzzy_spans {
        if exact_spans.iter().any(|exact| spans_overlap(*exact, fuzzy)) {
            continue;
        }
        exact_spans.push(fuzzy);
    }
    exact_spans.sort_by_key(|span| (span.start, span.end));
    exact_spans
}

fn spans_overlap(left: MatchSpan, right: MatchSpan) -> bool {
    left.start < right.end && right.start < left.end
}

pub(super) fn replace_content(
    content: &str,
    edits: &[TextEdit<'_>],
    replace_all: bool,
) -> Result<ReplaceContentOutcome, ReplaceContentError> {
    for edit in edits {
        if edit.old_string.is_empty() {
            return Err(ReplaceContentError::EmptyOld);
        }
    }
    if edits.is_empty() {
        return Err(ReplaceContentError::InvalidEditCount);
    }
    if replace_all && edits.len() != 1 {
        return Err(ReplaceContentError::InvalidEditCount);
    }

    let mut matched_edits = Vec::with_capacity(edits.len());
    let mut match_method = MatchMethod::Exact;
    let matcher = TextMatcher::new(content);
    for (edit_index, edit) in edits.iter().enumerate() {
        let matches = matcher.find_edit_matches(edit.old_string, replace_all);
        if matches.spans.is_empty() {
            return Err(ReplaceContentError::NotFound { edit_index });
        }
        if !replace_all && matches.occurrence_count > 1 {
            return Err(ReplaceContentError::Duplicate {
                edit_index,
                occurrences: matches.occurrence_count,
            });
        }

        let spans = if replace_all {
            matches.spans
        } else {
            vec![matches.spans[0]]
        };
        for span in spans {
            if span.method != MatchMethod::Exact {
                match_method = MatchMethod::FuzzyNormalization;
            }
            matched_edits.push(MatchedEdit {
                edit_index,
                start: span.start,
                end: span.end,
                new_string: edit.new_string.to_string(),
            });
        }
    }

    matched_edits.sort_by_key(|edit| edit.start);
    for pair in matched_edits.windows(2) {
        let previous = &pair[0];
        let current = &pair[1];
        if previous.end > current.start {
            return Err(ReplaceContentError::Overlap {
                previous_edit_index: previous.edit_index,
                current_edit_index: current.edit_index,
            });
        }
    }

    let new_content = apply_matched_edits(content, &matched_edits);
    if new_content == content {
        return Err(ReplaceContentError::NoChange);
    }
    Ok(ReplaceContentOutcome {
        content: new_content,
        replacements: matched_edits.len(),
        match_method,
    })
}

fn apply_matched_edits(content: &str, matched_edits: &[MatchedEdit]) -> String {
    let new_len = matched_edits.iter().fold(content.len(), |len, edit| {
        len.saturating_sub(edit.end - edit.start) + edit.new_string.len()
    });
    let mut rebuilt = String::with_capacity(new_len);
    let mut last = 0usize;
    for edit in matched_edits {
        rebuilt.push_str(&content[last..edit.start]);
        rebuilt.push_str(&edit.new_string);
        last = edit.end;
    }
    rebuilt.push_str(&content[last..]);
    rebuilt
}

fn find_exact_spans(
    haystack: &str,
    needle: &str,
    match_limit: Option<usize>,
) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut search_offset = 0usize;
    while let Some(index) = haystack[search_offset..].find(needle) {
        let start = search_offset + index;
        let end = start + needle.len();
        spans.push((start, end));
        if match_limit.is_some_and(|limit| spans.len() >= limit) {
            break;
        }
        search_offset = end;
    }
    spans
}

fn normalize_for_fuzzy_match(value: &str) -> String {
    normalize_with_source_map(value).text
}

fn normalize_with_source_map(value: &str) -> NormalizedText {
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut base_offset = 0usize;

    for segment in value.split_inclusive('\n') {
        let has_newline = segment.ends_with('\n');
        let line = if has_newline {
            &segment[..segment.len() - 1] // safety: split_inclusive matched an ASCII '\n', so len - 1 is a char boundary.
        } else {
            segment
        };
        let mut line_text = String::new();
        let mut line_spans = Vec::new();
        for (offset, ch) in line.char_indices() {
            let source_span = SourceSpan {
                start: base_offset + offset,
                end: base_offset + offset + ch.len_utf8(),
            };
            for normalized in normalize_char_for_fuzzy_match(ch) {
                line_text.push(normalized);
                line_spans.extend(std::iter::repeat_n(source_span, normalized.len_utf8()));
            }
        }
        while let Some(last_char) = line_text.chars().last() {
            if !last_char.is_whitespace() {
                break;
            }
            line_text.pop();
            for _ in 0..last_char.len_utf8() {
                line_spans.pop();
            }
        }
        text.push_str(&line_text);
        spans.extend(line_spans);
        if has_newline {
            text.push('\n');
            spans.push(SourceSpan {
                start: base_offset + segment.len() - 1,
                end: base_offset + segment.len(),
            });
        }
        base_offset += segment.len();
    }

    NormalizedText { text, spans }
}

fn normalize_char_for_fuzzy_match(ch: char) -> Vec<char> {
    std::iter::once(ch)
        .nfkd()
        .map(normalize_fuzzy_char)
        .collect()
}

fn normalize_fuzzy_char(ch: char) -> char {
    match ch {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' => '"',
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
        | '\u{2212}' => '-',
        '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2004}' | '\u{2005}' | '\u{2006}'
        | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200A}' | '\u{202F}' | '\u{205F}'
        | '\u{3000}' => ' ',
        _ => ch,
    }
}

fn normalized_span_to_source_span(
    normalized: &NormalizedText,
    start: usize,
    end: usize,
) -> Option<SourceSpan> {
    if start >= end || end > normalized.spans.len() {
        return None;
    }
    if start > 0 && same_source_span(normalized.spans[start], normalized.spans[start - 1]) {
        return None;
    }
    if end < normalized.spans.len()
        && same_source_span(normalized.spans[end], normalized.spans[end - 1])
    {
        return None;
    }
    Some(SourceSpan {
        start: normalized.spans.get(start)?.start,
        end: normalized.spans.get(end - 1)?.end,
    })
}

fn same_source_span(left: SourceSpan, right: SourceSpan) -> bool {
    left.start == right.start && left.end == right.end
}

fn content_may_need_fuzzy_normalization(value: &str) -> bool {
    for segment in value.split_inclusive('\n') {
        let line = segment.strip_suffix('\n').unwrap_or(segment);
        if line.chars().last().is_some_and(char::is_whitespace) {
            return true;
        }
        if line.chars().any(char_needs_fuzzy_normalization) {
            return true;
        }
    }
    false
}

fn char_needs_fuzzy_normalization(ch: char) -> bool {
    let normalized = normalize_char_for_fuzzy_match(ch);
    normalized.len() != 1 || normalized[0] != ch
}

pub(super) fn previous_char_boundary(value: &str, mut end: usize) -> usize {
    end = end.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_content_ignores_unlocatable_trailing_whitespace_normalization() {
        let result = replace_content(
            "\na",
            &[TextEdit {
                old_string: "\n ",
                new_string: "\nx",
            }],
            false,
        );

        assert_eq!(result, Err(ReplaceContentError::NotFound { edit_index: 0 }));
    }

    #[test]
    fn fuzzy_replacement_preserves_unrelated_original_content() {
        let content = "target\u{00A0}text\nuntouched\u{00A0}text   \n";
        let result = replace_content(
            content,
            &[TextEdit {
                old_string: "target text",
                new_string: "changed text",
            }],
            false,
        )
        .expect("fuzzy replacement");

        assert_eq!(result.content, "changed text\nuntouched\u{00A0}text   \n");
        assert_eq!(result.match_method, MatchMethod::FuzzyNormalization);
    }

    #[test]
    fn fuzzy_replacement_rejects_partial_source_character_match() {
        let result = replace_content(
            "caf\u{00E9}\n",
            &[TextEdit {
                old_string: "cafe",
                new_string: "coffee",
            }],
            false,
        );

        assert_eq!(result, Err(ReplaceContentError::NotFound { edit_index: 0 }));
    }

    #[test]
    fn replace_all_replaces_fuzzy_matches_when_exact_text_is_absent() {
        let content = "hello\u{00A0}world\nhello\u{2003}world\n";
        let result = replace_content(
            content,
            &[TextEdit {
                old_string: "hello world",
                new_string: "hello universe",
            }],
            true,
        )
        .expect("replace all");

        assert_eq!(result.content, "hello universe\nhello universe\n");
        assert_eq!(result.replacements, 2);
        assert_eq!(result.match_method, MatchMethod::FuzzyNormalization);
    }

    #[test]
    fn replace_all_replaces_mixed_exact_and_fuzzy_matches() {
        let content = "hello world\nhello\u{00A0}world\n";
        let result = replace_content(
            content,
            &[TextEdit {
                old_string: "hello world",
                new_string: "hello universe",
            }],
            true,
        )
        .expect("replace all");

        assert_eq!(result.content, "hello universe\nhello universe\n");
        assert_eq!(result.replacements, 2);
        assert_eq!(result.match_method, MatchMethod::FuzzyNormalization);
    }

    #[test]
    fn mixed_exact_and_fuzzy_matches_are_duplicate_without_replace_all() {
        let result = replace_content(
            "hello world\nhello\u{00A0}world\n",
            &[TextEdit {
                old_string: "hello world",
                new_string: "hello universe",
            }],
            false,
        );

        assert_eq!(
            result,
            Err(ReplaceContentError::Duplicate {
                edit_index: 0,
                occurrences: 2
            })
        );
    }

    #[test]
    fn lenient_probe_passes_stray_nul_rejects_dense() {
        // Small log with one stray NUL: floor (>8) not cleared -> passes.
        let mut log = b"auth failure for root\n".to_vec();
        log.push(0);
        assert!(reject_binary_probe_lenient(&log).is_ok());

        // NUL-dense binary (25%): both floor and ratio cleared -> rejected.
        let binary: Vec<u8> = (0..4096)
            .map(|i| if i % 4 == 0 { 0u8 } else { b'A' })
            .collect();
        assert!(reject_binary_probe_lenient(&binary).is_err());

        // The strict probe still rejects even a single NUL (patch-path contract).
        assert!(reject_binary_probe(&log).is_err());
    }

    #[test]
    fn lossy_decode_replaces_invalid_utf8_instead_of_failing() {
        let bytes = b"valid \xff text\r\nsecond line\n";
        // Strict decode fails; lossy decode succeeds and normalizes CRLF.
        assert!(decode_text(bytes).is_err());
        let (content, _enc, _le) = decode_text_lossy(bytes);
        assert!(content.contains("valid"));
        assert!(content.contains("second line"));
        assert!(!content.contains("\r\n"));
    }
}

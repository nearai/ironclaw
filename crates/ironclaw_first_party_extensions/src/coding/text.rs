use super::CodingCapabilityError;

use super::{
    operation_error,
    types::{FileEncoding, FuzzyMatch, LineEnding, MatchMethod},
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

/// Match ranges collected by a single scan of [`find_match_from`].
pub(super) struct MatchSet {
    /// Byte ranges of every located match, in order.
    pub(super) ranges: Vec<(usize, usize)>,
    /// Match method of the first hit (representative for reporting).
    pub(super) method: MatchMethod,
    /// True when scanning stopped at `limit` — `ranges.len()` is then a
    /// floor ("N or more"), not an exact count.
    pub(super) truncated: bool,
}

/// Collect match ranges in a single `find_match_from` pass.
///
/// This is the single source of truth for both the uniqueness validation and
/// the replacement ranges in `apply_patch`: counting and replacing share one
/// scan, so they cannot disagree (mirrors the v1 apply_patch fix in
/// `src/tools/builtin/file.rs`). `limit` bounds the scan — pass `Some(2)`
/// when the caller only needs to distinguish "exactly one" from "more than
/// one", so a large file with many occurrences is not scanned end-to-end.
/// A degenerate empty match (a needle that normalizes to nothing) is an
/// operation error rather than a silent stop.
pub(super) fn collect_matches(
    haystack: &str,
    needle: &str,
    limit: Option<usize>,
) -> Result<MatchSet, CodingCapabilityError> {
    let mut ranges = Vec::new();
    let mut method = MatchMethod::Exact;
    let mut truncated = false;
    let mut search_offset = 0usize;
    while let Some(item) = find_match_from(haystack, needle, search_offset) {
        // Unreachable by construction (find_match_from filters degenerate
        // spans); kept as a fail-closed guard against infinite scanning.
        if item.end <= item.start {
            return Err(operation_error());
        }
        if ranges.is_empty() {
            method = item.method;
        }
        ranges.push((item.start, item.end));
        search_offset = item.end;
        if limit.is_some_and(|limit| ranges.len() >= limit) {
            truncated = true;
            break;
        }
    }
    Ok(MatchSet {
        ranges,
        method,
        truncated,
    })
}

/// Rebuild `content` with `new_string` substituted at each of `ranges`
/// (non-overlapping, in order — as produced by [`collect_matches`]).
pub(super) fn replace_ranges(content: &str, ranges: &[(usize, usize)], new_string: &str) -> String {
    let mut rebuilt = String::with_capacity(content.len());
    let mut last = 0usize;
    for &(start, end) in ranges {
        rebuilt.push_str(&content[last..start]);
        rebuilt.push_str(new_string);
        last = end;
    }
    rebuilt.push_str(&content[last..]);
    rebuilt
}

fn find_match_from(haystack: &str, needle: &str, start_offset: usize) -> Option<FuzzyMatch> {
    let search = haystack.get(start_offset..)?;
    if !needle.is_empty()
        && let Some(index) = search.find(needle)
    {
        let start = start_offset + index;
        return Some(FuzzyMatch {
            start,
            end: start + needle.len(),
            method: MatchMethod::Exact,
        });
    }
    // A normalization strategy can locate a span whose original-byte range is
    // empty (the needle normalized to nothing at that position). Such a span
    // is unlocatable, not a match — skip to the next strategy instead of
    // returning a degenerate range the caller would have to special-case.
    let needle_stripped = strip_trailing_whitespace(needle);
    let haystack_stripped = strip_trailing_whitespace(search);
    if let Some((start, end)) = find_normalized_span(search, &haystack_stripped, &needle_stripped)
        && end > start
    {
        return Some(FuzzyMatch {
            start: start_offset + start,
            end: start_offset + end,
            method: MatchMethod::TrailingWhitespace,
        });
    }
    let needle_normalized = normalize_quotes(needle);
    let haystack_normalized = normalize_quotes(search);
    if !needle_normalized.is_empty()
        && let Some(index) = haystack_normalized.find(&needle_normalized)
    {
        let char_start = haystack_normalized[..index].chars().count();
        let char_len = needle_normalized.chars().count();
        let start = char_to_byte_idx(search, char_start)?;
        let end = char_to_byte_idx(search, char_start + char_len)?;
        if end > start {
            return Some(FuzzyMatch {
                start: start_offset + start,
                end: start_offset + end,
                method: MatchMethod::QuoteNormalization,
            });
        }
    }
    let needle_both = normalize_quotes(&needle_stripped);
    let haystack_both = normalize_quotes(&haystack_stripped);
    find_normalized_span(search, &haystack_both, &needle_both)
        .filter(|(start, end)| end > start)
        .map(|(start, end)| FuzzyMatch {
            start: start_offset + start,
            end: start_offset + end,
            method: MatchMethod::Both,
        })
}

fn strip_trailing_whitespace(value: &str) -> String {
    value
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_quotes(value: &str) -> String {
    value
        .replace(['\u{2018}', '\u{2019}', '\u{2032}'], "'")
        .replace(['\u{201C}', '\u{201D}', '\u{2033}'], "\"")
}

fn find_normalized_span(original: &str, normalized: &str, needle: &str) -> Option<(usize, usize)> {
    let index = normalized.find(needle)?;
    let char_index = normalized[..index].chars().count();
    let char_len = needle.chars().count();
    let start = map_normalized_char_to_original_byte(original, char_index)?;
    let end = map_normalized_char_to_original_byte(original, char_index + char_len)?;
    Some((start, end))
}

fn char_to_byte_idx(value: &str, char_index: usize) -> Option<usize> {
    if char_index == value.chars().count() {
        return Some(value.len());
    }
    value.char_indices().nth(char_index).map(|(index, _)| index)
}

fn map_normalized_char_to_original_byte(
    original: &str,
    normalized_char_index: usize,
) -> Option<usize> {
    if normalized_char_index == 0 {
        return Some(0);
    }
    let mut normalized_seen = 0usize;
    let mut original_byte = 0usize;
    for segment in original.split_inclusive('\n') {
        let (line, has_newline) = if let Some(stripped) = segment.strip_suffix('\n') {
            (stripped, true)
        } else {
            (segment, false)
        };
        let trimmed = line.trim_end();
        let trimmed_chars = trimmed.chars().count();
        if normalized_char_index <= normalized_seen + trimmed_chars {
            let within_line = normalized_char_index - normalized_seen;
            return Some(original_byte + char_to_byte_idx(line, within_line)?);
        }
        normalized_seen += trimmed_chars;
        original_byte += line.len();
        if has_newline {
            if normalized_char_index == normalized_seen + 1 {
                return Some(original_byte);
            }
            normalized_seen += 1;
            original_byte += 1;
        }
    }
    if normalized_char_index == normalized_seen {
        Some(original_byte)
    } else {
        None
    }
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
    fn collect_matches_ignores_unlocatable_trailing_whitespace_normalization() {
        let matches = collect_matches("\na", "\n ", None).expect("no degenerate match");

        assert!(matches.ranges.is_empty());
        assert_eq!(matches.method, MatchMethod::Exact);
        assert!(!matches.truncated);
    }

    #[test]
    fn collect_matches_limit_short_circuits_and_marks_truncated() {
        let matches = collect_matches("x x x x", "x", Some(2)).expect("matches collect");

        assert_eq!(matches.ranges, vec![(0, 1), (2, 3)]);
        assert!(matches.truncated);

        let all = collect_matches("x x x x", "x", None).expect("matches collect");
        assert_eq!(all.ranges.len(), 4);
        assert!(!all.truncated);
    }

    #[test]
    fn replace_ranges_substitutes_every_range() {
        let matches = collect_matches("a b a", "a", None).expect("matches collect");
        assert_eq!(replace_ranges("a b a", &matches.ranges, "z"), "z b z");
    }
}

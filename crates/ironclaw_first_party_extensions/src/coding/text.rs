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

pub(super) fn replace_content(
    content: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
    match_count: usize,
) -> Result<(String, usize), CodingCapabilityError> {
    if replace_all {
        let mut matches = Vec::new();
        let mut search_offset = 0usize;
        while let Some(item) = find_match_from(content, old_string, search_offset) {
            if item.end <= item.start {
                return Err(operation_error());
            }
            search_offset = item.end;
            matches.push((item.start, item.end));
        }
        if matches.len() != match_count {
            return Err(operation_error());
        }
        let mut rebuilt = String::with_capacity(content.len());
        let mut last = 0usize;
        for (start, end) in matches {
            rebuilt.push_str(&content[last..start]);
            rebuilt.push_str(new_string);
            last = end;
        }
        rebuilt.push_str(&content[last..]);
        Ok((rebuilt, match_count))
    } else {
        let item = find_match(content, old_string).ok_or_else(operation_error)?;
        let mut rebuilt =
            String::with_capacity(content.len() - (item.end - item.start) + new_string.len());
        rebuilt.push_str(&content[..item.start]);
        rebuilt.push_str(new_string);
        rebuilt.push_str(&content[item.end..]);
        Ok((rebuilt, 1))
    }
}

fn find_match(haystack: &str, needle: &str) -> Option<FuzzyMatch> {
    find_match_from(haystack, needle, 0)
}

fn find_match_from(haystack: &str, needle: &str, start_offset: usize) -> Option<FuzzyMatch> {
    let search = haystack.get(start_offset..)?;
    if let Some(index) = search.find(needle) {
        let start = start_offset + index;
        return Some(FuzzyMatch {
            start,
            end: start + needle.len(),
            method: MatchMethod::Exact,
        });
    }
    let needle_stripped = strip_trailing_whitespace(needle);
    let haystack_stripped = strip_trailing_whitespace(search);
    if let Some((start, end)) = find_normalized_span(search, &haystack_stripped, &needle_stripped) {
        return Some(FuzzyMatch {
            start: start_offset + start,
            end: start_offset + end,
            method: MatchMethod::TrailingWhitespace,
        });
    }
    let needle_normalized = normalize_quotes(needle);
    let haystack_normalized = normalize_quotes(search);
    if let Some(index) = haystack_normalized.find(&needle_normalized) {
        let char_start = haystack_normalized[..index].chars().count();
        let char_len = needle_normalized.chars().count();
        let start = char_to_byte_idx(search, char_start)?;
        let end = char_to_byte_idx(search, char_start + char_len)?;
        return Some(FuzzyMatch {
            start: start_offset + start,
            end: start_offset + end,
            method: MatchMethod::QuoteNormalization,
        });
    }
    let needle_both = normalize_quotes(&needle_stripped);
    let haystack_both = normalize_quotes(&haystack_stripped);
    find_normalized_span(search, &haystack_both, &needle_both).map(|(start, end)| FuzzyMatch {
        start: start_offset + start,
        end: start_offset + end,
        method: MatchMethod::Both,
    })
}

pub(super) fn count_matches(haystack: &str, needle: &str) -> (usize, MatchMethod) {
    let mut count = 0usize;
    let mut method = MatchMethod::Exact;
    let mut search_offset = 0usize;
    while let Some(item) = find_match_from(haystack, needle, search_offset) {
        if item.end <= item.start {
            break;
        }
        if count == 0 {
            method = item.method;
        }
        count += 1;
        search_offset = item.end;
    }
    (count, method)
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
    fn count_matches_ignores_unlocatable_trailing_whitespace_normalization() {
        let (count, method) = count_matches("\na", "\n ");

        assert_eq!(count, 0);
        assert_eq!(method, MatchMethod::Exact);
    }
}

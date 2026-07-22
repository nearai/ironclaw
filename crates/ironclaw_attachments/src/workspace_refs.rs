use std::collections::HashSet;

use ironclaw_host_api::ScopedPath;

const WORKSPACE_PREFIX: &str = "/workspace/";

/// Extract workspace file references using the same recognition rules as the
/// WebUI file chips: ignore closed Markdown code spans and fenced code blocks,
/// require the complete token to end in an alphanumeric extension, accept only bare/local Markdown path
/// boundaries (never a suffix inside an external URL), deduplicate, and
/// preserve first-seen order.
pub fn extract_workspace_attachment_paths(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    let visible = strip_closed_code_spans(content);
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    let mut search_from = 0;

    while let Some(relative_start) = visible
        .get(search_from..)
        .and_then(|remaining| remaining.find(WORKSPACE_PREFIX))
    {
        let start = search_from + relative_start;
        if !has_workspace_left_boundary(&visible, start)
            || has_external_url_context(&visible, start)
        {
            search_from = start.saturating_add(WORKSPACE_PREFIX.len());
            continue;
        }
        let Some(remaining) = visible.get(start..) else {
            break;
        };
        let token_end = remaining
            .char_indices()
            .take_while(|(_, character)| is_workspace_token_character(*character))
            .map(|(offset, character)| offset + character.len_utf8())
            .last()
            .map_or(start, |length| start + length);
        if !has_workspace_right_boundary(&visible, token_end) {
            search_from = start.saturating_add(WORKSPACE_PREFIX.len());
            continue;
        }
        let Some(token) = visible.get(start..token_end) else {
            break;
        };
        if let Some(path) = workspace_file_path(token)
            && seen.insert(path.to_string())
        {
            paths.push(path.to_string());
        }
        search_from = start.saturating_add(WORKSPACE_PREFIX.len());
    }

    paths
}

fn has_external_url_context(content: &str, start: usize) -> bool {
    let Some(prefix) = content.get(..start) else {
        return true;
    };
    let token_start = prefix
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map_or(0, |(offset, character)| offset + character.len_utf8());
    prefix
        .get(token_start..)
        .is_some_and(|context| context.contains("://"))
}

fn has_workspace_left_boundary(content: &str, start: usize) -> bool {
    if start == 0 {
        return true;
    }
    content
        .get(..start)
        .and_then(|prefix| prefix.chars().next_back())
        .is_some_and(|character| {
            !character.is_alphanumeric()
                && !matches!(
                    character,
                    '/' | ':' | '?' | '=' | '&' | '%' | '#' | '_' | '-' | '.'
                )
        })
}

fn has_workspace_right_boundary(content: &str, end: usize) -> bool {
    content
        .get(end..)
        .and_then(|suffix| suffix.chars().next())
        .is_none_or(|character| !matches!(character, '?' | '#' | '%' | '&' | '='))
}

fn is_workspace_token_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '/')
}

fn workspace_file_path(token: &str) -> Option<&str> {
    // A sentence-ending period is punctuation, not part of the path. Other
    // token characters (notably `-`) are never truncated: doing so would turn
    // a longer lookalike such as `report.pdf-old` into `report.pdf`.
    let candidate = token.trim_end_matches('.');
    let filename = candidate.rsplit('/').next()?;
    let (_, extension) = filename.rsplit_once('.')?;
    if !extension.is_empty()
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        && ScopedPath::new(candidate).is_ok_and(|path| path.as_str() == candidate)
    {
        Some(candidate)
    } else {
        None
    }
}

fn strip_closed_code_spans(content: &str) -> String {
    let mut visible = content.as_bytes().to_vec();
    mask_fenced_code_blocks(&mut visible);
    mask_inline_code_spans(&mut visible);
    String::from_utf8(visible).unwrap_or_else(|_| content.to_string())
}

fn mask_fenced_code_blocks(bytes: &mut [u8]) {
    let mut line_start = 0;
    while line_start < bytes.len() {
        let opening_line_end = line_end(bytes, line_start);
        let Some((fence, run_length)) = opening_fence(bytes, line_start, opening_line_end) else {
            line_start = next_line_start(bytes, opening_line_end);
            continue;
        };
        let mut close_start = next_line_start(bytes, opening_line_end);
        let mut close_end = None;
        while close_start < bytes.len() {
            let candidate_end = line_end(bytes, close_start);
            if is_closing_fence(bytes, close_start, candidate_end, fence, run_length) {
                close_end = Some(next_line_start(bytes, candidate_end));
                break;
            }
            close_start = next_line_start(bytes, candidate_end);
        }
        let Some(end) = close_end else {
            line_start = next_line_start(bytes, opening_line_end);
            continue;
        };
        bytes[line_start..end].fill(b' ');
        line_start = end;
    }
}

fn opening_fence(bytes: &[u8], line_start: usize, line_end: usize) -> Option<(u8, usize)> {
    let marker_start = indentation_end(bytes, line_start, line_end)?;
    let marker = *bytes.get(marker_start)?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let run_length = byte_run_length(bytes, marker_start, line_end, marker);
    (run_length >= 3).then_some((marker, run_length))
}

fn is_closing_fence(
    bytes: &[u8],
    line_start: usize,
    line_end: usize,
    marker: u8,
    opening_length: usize,
) -> bool {
    let Some(marker_start) = indentation_end(bytes, line_start, line_end) else {
        return false;
    };
    if bytes.get(marker_start) != Some(&marker) {
        return false;
    }
    let run_length = byte_run_length(bytes, marker_start, line_end, marker);
    run_length >= opening_length
        && bytes[marker_start + run_length..line_end]
            .iter()
            .all(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
}

fn indentation_end(bytes: &[u8], line_start: usize, line_end: usize) -> Option<usize> {
    let mut cursor = line_start;
    while cursor < line_end && bytes.get(cursor) == Some(&b' ') && cursor - line_start < 3 {
        cursor += 1;
    }
    if bytes.get(cursor) == Some(&b' ') {
        None
    } else {
        Some(cursor)
    }
}

fn line_end(bytes: &[u8], line_start: usize) -> usize {
    bytes
        .get(line_start..)
        .and_then(|line| line.iter().position(|byte| *byte == b'\n'))
        .map_or(bytes.len(), |offset| line_start + offset)
}

fn next_line_start(bytes: &[u8], line_end: usize) -> usize {
    if bytes.get(line_end) == Some(&b'\n') {
        line_end + 1
    } else {
        line_end
    }
}

fn mask_inline_code_spans(bytes: &mut [u8]) {
    let mut cursor = 0;
    while let Some(open) = find_byte(bytes, b'`', cursor) {
        let run_length = byte_run_length(bytes, open, bytes.len(), b'`');
        let mut candidate = open + run_length;
        let mut close = None;
        while let Some(next) = find_byte(bytes, b'`', candidate) {
            let candidate_length = byte_run_length(bytes, next, bytes.len(), b'`');
            if candidate_length == run_length {
                close = Some(next);
                break;
            }
            candidate = next + candidate_length;
        }
        let Some(close) = close else {
            cursor = open + run_length;
            continue;
        };
        let end = close + run_length;
        bytes[open..end].fill(b' ');
        cursor = end;
    }
}

fn byte_run_length(bytes: &[u8], start: usize, end: usize, needle: u8) -> usize {
    bytes[start..end]
        .iter()
        .take_while(|byte| **byte == needle)
        .count()
}

fn find_byte(bytes: &[u8], needle: u8, from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .iter()
        .position(|byte| *byte == needle)
        .map(|offset| from + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_bare_markdown_and_nested_workspace_paths() {
        assert_eq!(
            extract_workspace_attachment_paths(
                "Here: /workspace/report.csv and [chart](/workspace/out/chart.png)."
            ),
            vec!["/workspace/report.csv", "/workspace/out/chart.png"]
        );
    }

    #[test]
    fn ignores_code_spans_and_deduplicates_in_first_seen_order() {
        let text = "Use /workspace/report.pdf, not `/workspace/secret.pdf`.\n```\n/workspace/code.csv\n```\n/workspace/report.pdf /workspace/chart.png";
        assert_eq!(
            extract_workspace_attachment_paths(text),
            vec!["/workspace/report.pdf", "/workspace/chart.png"]
        );
    }

    #[test]
    fn ignores_non_workspace_and_extensionless_paths() {
        assert!(
            extract_workspace_attachment_paths(
                "Not /etc/passwd, /workspace, or /project/report.csv"
            )
            .is_empty()
        );
    }

    #[test]
    fn ignores_workspace_looking_paths_inside_external_urls() {
        assert!(
            extract_workspace_attachment_paths(
                "See https://example.test/workspace/report.pdf and https://example.test/?next=/workspace/secret.csv"
            )
            .is_empty()
        );
        assert_eq!(
            extract_workspace_attachment_paths(
                "The local copy is /workspace/report.pdf, '/workspace/report.pdf', or [download it](/workspace/report.pdf)."
            ),
            vec!["/workspace/report.pdf"]
        );
    }

    #[test]
    fn ignores_traversal_encoded_and_relative_url_lookalikes() {
        assert!(
            extract_workspace_attachment_paths(
                "No /workspace/../secret.txt, ./workspace/relative.pdf, /workspace/report.pdf?download=1, /workspace/report.pdf#preview, or /workspace/report.pdf%2Fother."
            )
            .is_empty()
        );
    }

    #[test]
    fn ignores_matching_backtick_runs_and_tilde_fences() {
        let text =
            "``/workspace/double.pdf``\n~~~text\n/workspace/tilde.csv\n~~~\n/workspace/visible.txt";
        assert_eq!(
            extract_workspace_attachment_paths(text),
            vec!["/workspace/visible.txt"]
        );
    }

    #[test]
    fn ignores_external_url_query_wrappers_and_longer_lookalikes() {
        assert!(
            extract_workspace_attachment_paths(
                "https://example.test/?next=(/workspace/a.pdf) /workspace/report.pdf-old"
            )
            .is_empty()
        );
    }
}

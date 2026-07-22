use std::collections::HashSet;

use ironclaw_host_api::ScopedPath;

const WORKSPACE_PREFIX: &str = "/workspace/";

/// Extract workspace file references using the same recognition rules as the
/// WebUI file chips: ignore closed Markdown code spans, keep the longest token
/// ending in an alphanumeric extension, accept only bare/local Markdown path
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
        if !has_workspace_left_boundary(&visible, start) {
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
        if let Some(path) = longest_file_prefix(token)
            && seen.insert(path.to_string())
        {
            paths.push(path.to_string());
        }
        search_from = start.saturating_add(WORKSPACE_PREFIX.len());
    }

    paths
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

fn longest_file_prefix(token: &str) -> Option<&str> {
    let bytes = token.as_bytes();
    for end in (WORKSPACE_PREFIX.len() + 2..=bytes.len()).rev() {
        if !bytes[end - 1].is_ascii_alphanumeric() {
            continue;
        }
        let candidate = token.get(..end)?;
        let filename = candidate.rsplit('/').next()?;
        let (_, extension) = filename.rsplit_once('.')?;
        if !extension.is_empty()
            && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
            && ScopedPath::new(candidate).is_ok_and(|path| path.as_str() == candidate)
        {
            return Some(candidate);
        }
    }
    None
}

fn strip_closed_code_spans(content: &str) -> String {
    let mut visible = content.as_bytes().to_vec();
    mask_closed_ranges(&mut visible, b"```");
    mask_closed_ranges(&mut visible, b"`");
    String::from_utf8(visible).unwrap_or_else(|_| content.to_string())
}

fn mask_closed_ranges(bytes: &mut [u8], delimiter: &[u8]) {
    let mut cursor = 0;
    while let Some(open) = find_bytes(bytes, delimiter, cursor) {
        let body_start = open + delimiter.len();
        let Some(close) = find_bytes(bytes, delimiter, body_start) else {
            break;
        };
        bytes[open..close + delimiter.len()].fill(b' ');
        cursor = close + delimiter.len();
    }
}

fn find_bytes(bytes: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
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
}

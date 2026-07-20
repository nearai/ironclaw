use std::collections::HashSet;

const WORKSPACE_PREFIX: &str = "/workspace/";

/// Extract workspace file references using the same recognition rules as the
/// WebUI file chips: ignore closed Markdown code spans, keep the longest token
/// ending in an alphanumeric extension, deduplicate, and preserve first-seen
/// order.
pub fn extract_workspace_attachment_paths(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    let visible = strip_closed_code_spans(content);
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    let mut search_from = 0;

    while let Some(relative_start) = visible[search_from..].find(WORKSPACE_PREFIX) {
        let start = search_from + relative_start;
        let token_end = visible[start..]
            .char_indices()
            .take_while(|(_, character)| is_workspace_token_character(*character))
            .map(|(offset, character)| offset + character.len_utf8())
            .last()
            .map_or(start, |length| start + length);
        let token = &visible[start..token_end];
        if let Some(path) = longest_file_prefix(token)
            && seen.insert(path.to_string())
        {
            paths.push(path.to_string());
        }
        search_from = start.saturating_add(WORKSPACE_PREFIX.len());
    }

    paths
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
        if !extension.is_empty() && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
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
}

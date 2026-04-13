//! DingTalk send helpers: markdown chunking and session webhook delivery.

use reqwest::Client;
use tracing::debug;

use crate::error::ChannelError;

/// Default maximum characters per markdown chunk.
pub const DEFAULT_CHUNK_LIMIT: usize = 3800;

/// Split `text` into chunks of at most `limit` characters, splitting on `\n`
/// boundaries where possible.
///
/// If the text spans a fenced code block when a split occurs, the fence is
/// closed at the end of the current chunk and reopened at the start of the
/// next chunk.
///
/// When there is more than one chunk, each chunk receives a `(N/M)` suffix
/// appended to the first line (or prepended as a header line).
pub fn split_markdown_chunks(text: &str, limit: usize) -> Vec<String> {
    if text.len() <= limit {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut inside_fence = false;
    // Track the fence delimiter so we can reopen the same kind.
    let mut fence_delim = String::new();

    for line in text.split('\n') {
        // Detect fence toggle
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let delim: String = trimmed.chars().take(3).collect();
            if inside_fence {
                // Closing fence
                inside_fence = false;
                fence_delim.clear();
            } else {
                inside_fence = true;
                fence_delim = delim;
            }
        }

        // +1 for the newline we'll add when joining
        let needed = if current.is_empty() {
            line.len()
        } else {
            line.len() + 1
        };

        if !current.is_empty() && current.len() + needed > limit {
            // Need to flush current chunk
            if inside_fence {
                // Close the fence before cutting
                current.push('\n');
                current.push_str(&fence_delim);
            }
            chunks.push(current.clone());
            current.clear();

            // Reopen fence at start of new chunk if we were inside one
            if inside_fence {
                current.push_str(&fence_delim);
                current.push('\n');
            }
        }

        // If the line itself exceeds the limit (no newlines), split at char
        // boundary.
        if needed > limit {
            let mut remaining = line;
            while !remaining.is_empty() {
                if !current.is_empty() {
                    current.push('\n');
                }
                let available = limit.saturating_sub(current.len());
                if available == 0 {
                    chunks.push(current.clone());
                    current.clear();
                    continue;
                }
                // Find safe char boundary
                let split_at = if available >= remaining.len() {
                    remaining.len()
                } else {
                    let mut pos = available;
                    while pos > 0 && !remaining.is_char_boundary(pos) {
                        pos -= 1;
                    }
                    pos
                };
                current.push_str(&remaining[..split_at]);
                remaining = &remaining[split_at..];
                if !remaining.is_empty() {
                    chunks.push(current.clone());
                    current.clear();
                }
            }
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    // Apply (N/M) suffix when there is more than one chunk
    if chunks.len() > 1 {
        let total = chunks.len();
        for (i, chunk) in chunks.iter_mut().enumerate() {
            let suffix = format!(" ({}/{})", i + 1, total);
            // Insert suffix after the first line
            match chunk.find('\n') {
                Some(pos) => chunk.insert_str(pos, &suffix),
                None => chunk.push_str(&suffix),
            }
        }
    }

    chunks
}

/// Detect whether `text` contains markdown syntax and extract a title.
///
/// Returns `(is_markdown, title)`.
///
/// - `is_markdown` is `true` if the text contains `#`, `*`, `` ` ``, `[`,
///   `|`, or `\n`.
/// - `title` is extracted from the first `#` heading (max 30 chars), or
///   defaults to `"IronClaw 消息"`.
pub fn detect_markdown(text: &str) -> (bool, String) {
    let is_markdown = text.contains('#')
        || text.contains('*')
        || text.contains('`')
        || text.contains('[')
        || text.contains('|')
        || text.contains('\n');

    let title = text
        .lines()
        .find(|l| l.starts_with('#'))
        .map(|l| {
            let heading = l.trim_start_matches('#').trim();
            let mut chars = heading.char_indices();
            let end = chars.nth(30).map(|(i, _)| i).unwrap_or(heading.len());
            heading[..end].to_string()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "IronClaw 消息".to_string());

    (is_markdown, title)
}

fn is_success_code(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Number(number) => {
            number.as_i64() == Some(0) || number.as_i64() == Some(200)
        }
        serde_json::Value::String(code) => {
            let normalized = code.trim().to_ascii_lowercase();
            normalized.is_empty()
                || normalized == "0"
                || normalized == "200"
                || normalized == "ok"
                || normalized == "success"
        }
        _ => true,
    }
}

fn validate_business_response(
    status: reqwest::StatusCode,
    body_text: &str,
    context: &str,
) -> Result<Option<serde_json::Value>, ChannelError> {
    if !status.is_success() {
        return Err(ChannelError::Http(format!(
            "{context} returned {status}: {body_text}"
        )));
    }

    let trimmed = body_text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let Ok(body_json) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return Ok(None);
    };

    if matches!(body_json.get("success").and_then(|v| v.as_bool()), Some(false)) {
        return Err(ChannelError::Http(format!(
            "{context} business failure: {trimmed}"
        )));
    }

    if let Some(code) = body_json.get("errcode").or_else(|| body_json.get("code")) {
        if !is_success_code(code) {
            return Err(ChannelError::Http(format!(
                "{context} business failure: {trimmed}"
            )));
        }
    }

    Ok(Some(body_json))
}

pub(super) async fn parse_business_response(
    resp: reqwest::Response,
    context: &str,
) -> Result<Option<serde_json::Value>, ChannelError> {
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    validate_business_response(status, &body_text, context)
}

pub(super) async fn ensure_business_success(
    resp: reqwest::Response,
    context: &str,
) -> Result<(), ChannelError> {
    parse_business_response(resp, context).await.map(|_| ())
}

/// Send `text` via a DingTalk session webhook (no auth header required).
///
/// The body is formatted as a markdown message regardless of content, since
/// session webhooks accept markdown without extra auth.
pub async fn send_via_webhook(
    client: &Client,
    webhook_url: &str,
    title: &str,
    text: &str,
) -> Result<(), ChannelError> {
    let body = serde_json::json!({
        "msgtype": "markdown",
        "markdown": {
            "title": title,
            "text": text,
        }
    });

    debug!(webhook_url = %webhook_url, "Sending via session webhook");

    let resp = client
        .post(webhook_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::Http(format!("webhook send: {e}")))?;

    ensure_business_success(resp, "session webhook").await
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── split_markdown_chunks ────────────────────────────────────────────────

    #[test]
    fn short_text_returns_single_chunk() {
        let text = "Hello, world!";
        let chunks = split_markdown_chunks(text, DEFAULT_CHUNK_LIMIT);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn long_text_splits_on_newlines() {
        // Build text that exceeds 100-char limit when combined
        let line = "A".repeat(40);
        let text = format!("{line}\n{line}\n{line}");
        let chunks = split_markdown_chunks(&text, 90);
        assert!(chunks.len() > 1, "should split into multiple chunks");
        // Each chunk (minus suffix) should be ≤ 90 chars (suffix adds a little)
        for chunk in &chunks {
            // Remove (N/M) suffix for length check — suffix is small
            assert!(chunk.len() < 120, "chunk too long: {}", chunk.len());
        }
    }

    #[test]
    fn multi_chunk_gets_n_of_m_suffix() {
        let line = "X".repeat(60);
        let text = format!("{line}\n{line}\n{line}");
        let chunks = split_markdown_chunks(&text, 100);
        assert!(chunks.len() > 1);
        assert!(chunks[0].contains("(1/"), "first chunk missing suffix");
        let last = chunks.last().unwrap();
        let total = chunks.len();
        assert!(
            last.contains(&format!("({total}/{total})")),
            "last chunk missing suffix"
        );
    }

    #[test]
    fn single_chunk_no_suffix() {
        let text = "Short text";
        let chunks = split_markdown_chunks(text, DEFAULT_CHUNK_LIMIT);
        assert_eq!(chunks.len(), 1);
        assert!(
            !chunks[0].contains("(1/1)"),
            "single chunk should not have suffix"
        );
    }

    #[test]
    fn code_fence_closed_and_reopened_on_split() {
        // Construct text where a code fence straddles the split boundary
        let preamble = "Intro\n```rust\n".to_string();
        let code_lines: String = (0..10).map(|i| format!("let x{i} = {i};\n")).collect();
        let closing = "```\nDone";
        let text = format!("{preamble}{code_lines}{closing}");

        let chunks = split_markdown_chunks(&text, 60);
        if chunks.len() > 1 {
            // First chunk should close the fence
            assert!(
                chunks[0].trim_end().ends_with("```"),
                "first chunk should close fence, got: {:?}",
                chunks[0]
            );
            // Second chunk should reopen the fence
            assert!(
                chunks[1].starts_with("```"),
                "second chunk should reopen fence, got: {:?}",
                chunks[1]
            );
        }
    }

    #[test]
    fn line_longer_than_limit_splits_at_char_boundary() {
        // One very long line with no newlines
        let text = "A".repeat(200);
        let chunks = split_markdown_chunks(&text, 100);
        assert!(chunks.len() > 1, "long single line should be split");
        for chunk in &chunks {
            // Rough bound; suffix adds small overhead
            assert!(chunk.len() <= 120, "chunk too long: {}", chunk.len());
        }
    }

    #[test]
    fn utf8_multibyte_split_is_safe() {
        // Chinese characters are 3 bytes each; limit by byte count
        let text: String = "你好世界".repeat(40); // 640 bytes
        let chunks = split_markdown_chunks(&text, 100);
        // All chunks should be valid UTF-8 (no panic means success)
        for chunk in &chunks {
            assert!(std::str::from_utf8(chunk.as_bytes()).is_ok());
        }
    }

    // ── detect_markdown ──────────────────────────────────────────────────────

    #[test]
    fn plain_text_not_markdown() {
        let (is_md, title) = detect_markdown("Hello there, how are you today");
        assert!(!is_md);
        assert_eq!(title, "IronClaw 消息");
    }

    #[test]
    fn text_with_hash_is_markdown() {
        let (is_md, _) = detect_markdown("# Heading\nSome text");
        assert!(is_md);
    }

    #[test]
    fn extracts_title_from_heading() {
        let (_, title) = detect_markdown("# My Title\nSome content");
        assert_eq!(title, "My Title");
    }

    #[test]
    fn title_truncated_at_30_chars() {
        let long_heading = format!("# {}", "A".repeat(50));
        let (_, title) = detect_markdown(&long_heading);
        assert_eq!(title.chars().count(), 30);
    }

    #[test]
    fn multiline_text_is_markdown() {
        let (is_md, _) = detect_markdown("line one\nline two");
        assert!(is_md);
    }

    #[test]
    fn backtick_is_markdown() {
        let (is_md, _) = detect_markdown("Use `code` here");
        assert!(is_md);
    }

    #[test]
    fn asterisk_is_markdown() {
        let (is_md, _) = detect_markdown("**bold** text");
        assert!(is_md);
    }

    #[test]
    fn default_title_when_no_heading() {
        let (_, title) = detect_markdown("**bold** text without heading");
        assert_eq!(title, "IronClaw 消息");
    }
}

//! Knowledge card metadata generation for memory documents.
//!
//! Generates human-readable title, summary, and tags for each memory document
//! to power the knowledge card view in the web UI. Metadata is stored in the
//! document's existing `metadata` JSON field under `card_*` keys.

use serde::{Deserialize, Serialize};

/// Structured metadata for knowledge card rendering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardMetadata {
    pub card_title: String,
    pub card_summary: String,
    pub card_tags: Vec<String>,
}

/// Well-known paths that should be hidden from the knowledge card view.
/// These are system/identity files, not user-generated knowledge.
///
/// Note: MEMORY.md is NOT in this list. It is the canonical memory index file
/// that agents write to (Claude Code auto-memory, manual memory appends, etc.).
/// It is expanded into individual cards via `parse_memory_index_entries()` in
/// the cards handler — each bulleted entry becomes its own card.
const HIDDEN_FROM_CARDS: &[&str] = &[
    "IDENTITY.md",
    "SOUL.md",
    "AGENTS.md",
    "USER.md",
    "TOOLS.md",
    "BOOTSTRAP.md",
    "HEARTBEAT.md",
    "README.md",
    ".config",
];

/// Path of the canonical memory index file. Agents append bulleted memory
/// entries to this file; the card view parses each bullet into an individual
/// card so users see their memories directly in the UI.
pub const MEMORY_INDEX_PATH: &str = "MEMORY.md";

/// A single parsed entry from MEMORY.md.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryIndexEntry {
    /// The full bullet text (without leading marker).
    pub text: String,
    /// The first 80 chars, used as the card title.
    pub title: String,
    /// The full text (or up to 200 chars), used as the card summary.
    pub summary: String,
}

/// Parse MEMORY.md content into individual entries.
///
/// Two entry shapes are recognized:
/// - **Bulleted entries**: lines starting with `- `, `* `, or `N. `. Each
///   bullet is its own entry. Indented continuation lines (2+ spaces or tab)
///   are appended to the preceding bullet.
/// - **Paragraph entries**: contiguous runs of non-bullet, non-structural
///   lines separated by blank lines. Each paragraph block becomes one entry.
///
/// Structural markup is filtered out and never becomes an entry:
/// - Markdown headers (`# ...`, `## ...`, …)
/// - Horizontal rules (a standalone `---` line that is not YAML frontmatter)
/// - YAML frontmatter — a `---`/`---` pair at the very top of the file
///
/// Paragraph support exists because agents writing via `memory_write`
/// (`target=memory`, `append=true`) pass raw prose — not pre-formatted
/// bullets — and users still expect those memories to appear in the Memory
/// tab. See `append_memory` in `workspace::mod`.
pub fn parse_memory_index_entries(content: &str) -> Vec<MemoryIndexEntry> {
    #[derive(PartialEq)]
    enum Mode {
        Idle,
        Bullet,
        Paragraph,
    }

    let mut entries: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    let mut mode = Mode::Idle;
    let mut in_frontmatter = false;
    let mut seen_non_empty = false;

    for line in content.lines() {
        let trimmed_start = line.trim_start();
        let trimmed = line.trim();

        // Frontmatter: a `---` pair is treated as YAML frontmatter only when
        // the opening `---` is the first non-empty line of the document.
        // Any other `---` is a horizontal rule and separates entries.
        if trimmed == "---" {
            if let Some(prev) = current.take() {
                entries.push(prev);
            }
            mode = Mode::Idle;
            if in_frontmatter {
                in_frontmatter = false;
            } else if !seen_non_empty {
                in_frontmatter = true;
                seen_non_empty = true;
            }
            continue;
        }
        if in_frontmatter {
            if !trimmed.is_empty() {
                seen_non_empty = true;
            }
            continue;
        }
        if !trimmed.is_empty() {
            seen_non_empty = true;
        }

        // Headers are structural, not content.
        if trimmed_start.starts_with('#') {
            if let Some(prev) = current.take() {
                entries.push(prev);
            }
            mode = Mode::Idle;
            continue;
        }

        // Detect bullet markers: "- ", "* ", "1. ", "23. " etc.
        let is_bullet = trimmed_start.starts_with("- ")
            || trimmed_start.starts_with("* ")
            || trimmed_start
                .split_once(". ")
                .map(|(prefix, _)| !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()))
                .unwrap_or(false);

        if is_bullet {
            if let Some(prev) = current.take() {
                entries.push(prev);
            }
            let text = if let Some(rest) = trimmed_start.strip_prefix("- ") {
                rest
            } else if let Some(rest) = trimmed_start.strip_prefix("* ") {
                rest
            } else {
                trimmed_start
                    .split_once(". ")
                    .map(|(_, rest)| rest)
                    .unwrap_or(trimmed_start)
            };
            current = Some(text.to_string());
            mode = Mode::Bullet;
            continue;
        }

        match mode {
            Mode::Bullet => {
                // Indented continuation line extends the current bullet;
                // a blank line ends it; a non-indented non-bullet line is
                // ignored (callers sometimes interleave stray prose with a
                // list and don't mean a new entry without a blank separator).
                let line_has_content = !trimmed_start.is_empty();
                let is_indented = line.starts_with("  ") || line.starts_with('\t');
                if let Some(ref mut buf) = current {
                    if line_has_content && is_indented {
                        buf.push('\n');
                        buf.push_str(trimmed_start);
                    } else if !line_has_content {
                        entries.push(current.take().unwrap_or_default());
                        mode = Mode::Idle;
                    }
                }
            }
            Mode::Paragraph => {
                if trimmed.is_empty() {
                    if let Some(prev) = current.take() {
                        entries.push(prev);
                    }
                    mode = Mode::Idle;
                } else if let Some(ref mut buf) = current {
                    buf.push('\n');
                    buf.push_str(trimmed);
                }
            }
            Mode::Idle => {
                if !trimmed.is_empty() {
                    current = Some(trimmed.to_string());
                    mode = Mode::Paragraph;
                }
            }
        }
    }
    if let Some(last) = current.take() {
        entries.push(last);
    }

    entries
        .into_iter()
        .filter(|e| !e.trim().is_empty())
        .map(|text| {
            let first_line = text.lines().next().unwrap_or("").trim();
            let title = truncate_chars(first_line, 80);
            let summary = truncate_chars(text.trim(), 200);
            MemoryIndexEntry { text, title, summary }
        })
        .collect()
}

/// Truncate a &str to at most `max_chars` characters (char-boundary safe).
fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut result = String::with_capacity(s.len().min(max_chars * 4));
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            result.push('…');
            break;
        }
        result.push(ch);
    }
    result
}

/// Check if a path should be hidden from the knowledge card view.
pub fn is_hidden_from_cards(path: &str) -> bool {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    HIDDEN_FROM_CARDS.contains(&file_name) || path.starts_with("context/") || file_name == ".config"
}

/// Generate fallback metadata from document content (no LLM needed).
///
/// Uses simple heuristics: first non-empty line as title, first 150 chars as summary.
/// This is called synchronously so cards always have something to show.
pub fn generate_fallback_metadata(content: &str, path: &str) -> CardMetadata {
    let lines: Vec<&str> = content.lines().collect();

    // Find first non-empty, non-frontmatter line for title
    let mut title = String::new();
    let mut in_frontmatter = false;
    for line in &lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            in_frontmatter = !in_frontmatter;
            continue;
        }
        if in_frontmatter || trimmed.is_empty() {
            continue;
        }
        // Strip leading # for markdown headings
        title = trimmed.trim_start_matches('#').trim().to_string();
        break;
    }

    if title.is_empty() {
        // Use filename without extension as fallback title
        let file_name = path.rsplit('/').next().unwrap_or(path);
        title = file_name
            .strip_suffix(".md")
            .unwrap_or(file_name)
            .replace(['-', '_'], " ");
    }

    // Truncate title to 80 chars on a char boundary
    if title.len() > 80 {
        let mut end = 80;
        while end > 0 && !title.is_char_boundary(end) {
            end -= 1;
        }
        title.truncate(end);
        title.push_str("...");
    }

    // Build summary from first meaningful content (skip frontmatter and title)
    let mut summary_parts = Vec::new();
    let mut chars_collected = 0;
    let mut past_title = false;
    let mut in_fm = false;
    for line in &lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            in_fm = !in_fm;
            continue;
        }
        if in_fm || trimmed.is_empty() {
            continue;
        }
        if !past_title {
            past_title = true;
            continue; // skip the title line
        }
        // Skip heading lines for summary
        if trimmed.starts_with('#') {
            continue;
        }
        summary_parts.push(trimmed);
        chars_collected += trimmed.len();
        if chars_collected >= 150 {
            break;
        }
    }

    let mut summary = summary_parts.join(" ");
    if summary.len() > 150 {
        let mut end = 150;
        while end > 0 && !summary.is_char_boundary(end) {
            end -= 1;
        }
        summary.truncate(end);
        summary.push_str("...");
    }

    CardMetadata {
        card_title: title,
        card_summary: summary,
        card_tags: Vec::new(),
    }
}

/// Build the LLM prompt for metadata generation.
pub fn build_metadata_prompt(content: &str) -> (String, String) {
    let system = "You generate concise metadata for knowledge documents. \
        Return ONLY valid JSON with exactly these fields: \
        {\"title\": \"short human-readable title\", \
        \"summary\": \"1-2 sentence plain-language summary\", \
        \"tags\": [\"tag1\", \"tag2\"]}. \
        Title should be descriptive (not a file path). \
        Summary should explain what this document is about in plain language. \
        Tags should be 1-3 semantic labels. \
        Return ONLY the JSON, no markdown fences."
        .to_string();

    // Truncate content to avoid blowing up the LLM context
    let max_content = 2000;
    let truncated = if content.len() > max_content {
        let mut end = max_content;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        &content[..end]
    } else {
        content
    };

    let user = format!("Generate metadata for this document:\n\n{truncated}");
    (system, user)
}

/// Parse LLM response into CardMetadata.
pub fn parse_llm_response(response: &str) -> Option<CardMetadata> {
    // Try to find JSON in the response (handle markdown fences)
    let json_str = response
        .trim()
        .strip_prefix("```json")
        .or_else(|| response.trim().strip_prefix("```"))
        .unwrap_or(response.trim());
    let json_str = json_str.strip_suffix("```").unwrap_or(json_str).trim();

    #[derive(Deserialize)]
    struct LlmMetadata {
        title: Option<String>,
        summary: Option<String>,
        tags: Option<Vec<String>>,
    }

    let parsed: LlmMetadata = serde_json::from_str(json_str).ok()?;

    let title = parsed.title.filter(|t| !t.is_empty())?;
    let summary = parsed.summary.unwrap_or_default();
    let tags = parsed
        .tags
        .unwrap_or_default()
        .into_iter()
        .take(3)
        .collect();

    Some(CardMetadata {
        card_title: title,
        card_summary: summary,
        card_tags: tags,
    })
}

/// Extract CardMetadata from a document's metadata JSON field.
pub fn extract_card_metadata(metadata: &serde_json::Value) -> Option<CardMetadata> {
    let obj = metadata.as_object()?;
    let title = obj.get("card_title")?.as_str()?.to_string();
    if title.is_empty() {
        return None;
    }
    let summary = obj
        .get("card_summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let tags = obj
        .get("card_tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Some(CardMetadata {
        card_title: title,
        card_summary: summary,
        card_tags: tags,
    })
}

/// Merge card metadata into a document's metadata JSON value.
pub fn merge_card_metadata(metadata: &mut serde_json::Value, card: &CardMetadata) {
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert(
            "card_title".to_string(),
            serde_json::Value::String(card.card_title.clone()),
        );
        obj.insert(
            "card_summary".to_string(),
            serde_json::Value::String(card.card_summary.clone()),
        );
        obj.insert("card_tags".to_string(), serde_json::json!(card.card_tags));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_metadata_from_markdown() {
        let content =
            "# My Project Notes\n\nThis is about the deployment process.\nWe use Docker and K8s.\n";
        let meta = generate_fallback_metadata(content, "projects/notes.md");
        assert_eq!(meta.card_title, "My Project Notes");
        assert!(meta.card_summary.contains("deployment"));
        assert!(meta.card_tags.is_empty());
    }

    #[test]
    fn fallback_metadata_with_frontmatter() {
        let content = "---\ndate: 2026-01-01\n---\n# Title Here\n\nSome content follows.\n";
        let meta = generate_fallback_metadata(content, "test.md");
        assert_eq!(meta.card_title, "Title Here");
        assert!(meta.card_summary.contains("content"));
    }

    #[test]
    fn fallback_metadata_empty_content() {
        let meta = generate_fallback_metadata("", "notes.md");
        assert_eq!(meta.card_title, "notes");
        assert!(meta.card_summary.is_empty());
    }

    #[test]
    fn fallback_metadata_single_line() {
        let meta = generate_fallback_metadata("Just one line", "test.md");
        assert_eq!(meta.card_title, "Just one line");
        assert!(meta.card_summary.is_empty());
    }

    #[test]
    fn fallback_metadata_heading_stripped() {
        let content = "## Sub Heading\n\nParagraph text here.\n";
        let meta = generate_fallback_metadata(content, "test.md");
        assert_eq!(meta.card_title, "Sub Heading");
    }

    #[test]
    fn parse_llm_response_valid() {
        let response = r#"{"title": "Deployment Guide", "summary": "How to deploy services.", "tags": ["ops", "deploy"]}"#;
        let meta = parse_llm_response(response).unwrap();
        assert_eq!(meta.card_title, "Deployment Guide");
        assert_eq!(meta.card_summary, "How to deploy services.");
        assert_eq!(meta.card_tags, vec!["ops", "deploy"]);
    }

    #[test]
    fn parse_llm_response_with_fences() {
        let response =
            "```json\n{\"title\": \"Test\", \"summary\": \"A test.\", \"tags\": [\"test\"]}\n```";
        let meta = parse_llm_response(response).unwrap();
        assert_eq!(meta.card_title, "Test");
    }

    #[test]
    fn parse_llm_response_invalid() {
        assert!(parse_llm_response("not json at all").is_none());
    }

    #[test]
    fn parse_llm_response_missing_title() {
        let response = r#"{"summary": "No title", "tags": []}"#;
        assert!(parse_llm_response(response).is_none());
    }

    #[test]
    fn extract_and_merge_round_trip() {
        let card = CardMetadata {
            card_title: "Test Title".to_string(),
            card_summary: "Test summary.".to_string(),
            card_tags: vec!["tag1".to_string()],
        };
        let mut metadata = serde_json::json!({});
        merge_card_metadata(&mut metadata, &card);
        let extracted = extract_card_metadata(&metadata).unwrap();
        assert_eq!(card, extracted);
    }

    #[test]
    fn is_hidden_filters_system_files() {
        assert!(is_hidden_from_cards("SOUL.md"));
        assert!(is_hidden_from_cards("IDENTITY.md"));
        assert!(is_hidden_from_cards("AGENTS.md"));
        assert!(is_hidden_from_cards("HEARTBEAT.md"));
        assert!(is_hidden_from_cards("context/profile.json"));
        assert!(is_hidden_from_cards("context/assistant-directives.md"));
        // MEMORY.md is NOT hidden — it is expanded into per-bullet cards
        assert!(!is_hidden_from_cards("MEMORY.md"));
        assert!(!is_hidden_from_cards("projects/notes.md"));
        assert!(!is_hidden_from_cards("daily/2026-01-01.md"));
    }

    #[test]
    fn parse_memory_index_dash_bullets() {
        let content = "- First memory\n- Second memory\n- Third memory\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].text, "First memory");
        assert_eq!(entries[1].text, "Second memory");
        assert_eq!(entries[2].text, "Third memory");
    }

    #[test]
    fn parse_memory_index_star_bullets() {
        let content = "* Alpha\n* Beta\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "Alpha");
        assert_eq!(entries[1].text, "Beta");
    }

    #[test]
    fn parse_memory_index_numbered_bullets() {
        let content = "1. First\n2. Second\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "First");
        assert_eq!(entries[1].text, "Second");
    }

    #[test]
    fn parse_memory_index_treats_preamble_paragraph_as_entry() {
        // Before Fix B this was "skip preamble"; now non-structural prose
        // becomes its own entry so agents writing raw text via
        // `memory_write target=memory append=true` still surface in the UI.
        // Headers (`# ...`) are still filtered out.
        let content = "# My Memory\n\nSome intro text.\n\n- Real entry 1\n- Real entry 2\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].text, "Some intro text.");
        assert_eq!(entries[1].text, "Real entry 1");
        assert_eq!(entries[2].text, "Real entry 2");
    }

    #[test]
    fn parse_memory_index_multiline_continuation() {
        let content = "- First entry\n  continues here\n- Second entry\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "First entry\ncontinues here");
        assert_eq!(entries[1].text, "Second entry");
    }

    #[test]
    fn parse_memory_index_empty() {
        assert_eq!(parse_memory_index_entries("").len(), 0);
        // Whitespace-only and header-only inputs still produce no entries;
        // only real content (prose or bullets) does.
        assert_eq!(parse_memory_index_entries("   \n\n").len(), 0);
        assert_eq!(parse_memory_index_entries("# Only a header\n").len(), 0);
    }

    #[test]
    fn parse_memory_index_prose_without_bullets_becomes_entry() {
        // Regression for the reported bug: agent appends plain text via
        // `memory_write target=memory append=true content="..."`. The content
        // lands in MEMORY.md with no bullet marker. Previously the parser
        // returned [], so the Memory tab was empty even though the file had
        // content. Fix B: prose becomes a paragraph entry.
        let content = "# No bullets\nJust prose\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "Just prose");
    }

    #[test]
    fn parse_memory_index_entry_title_and_summary() {
        let content = "- This is a memory entry about user preferences that is fairly long\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].title.is_empty());
        assert!(entries[0].title.chars().count() <= 81); // 80 + possible ellipsis
    }

    #[test]
    fn parse_memory_index_claude_auto_memory_format() {
        // Real Claude Code auto-memory format: `- [Title](path.md) — description`
        let content = "\
- [No Claude co-author](feedback_no_coauthor.md) — Never add Co-Authored-By Claude to commits
- [User role](user_role.md) — User is a senior backend engineer
";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].text.contains("No Claude co-author"));
        assert!(entries[1].text.contains("User role"));
    }

    #[test]
    fn parse_memory_index_mixed_markers() {
        // Mixed bullet markers should all be recognized
        let content = "- Dash entry\n* Star entry\n1. Numbered entry\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn parse_memory_index_mixes_paragraphs_and_bullets() {
        // Headers and horizontal rules are still filtered. Paragraphs that
        // stand alone (separated by blank lines) become their own entries
        // and interleave with bullets in document order.
        let content = "\
# Memory Index

Some prose before the list.

- First entry
- Second entry

---

- Third entry
";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].text, "Some prose before the list.");
        assert_eq!(entries[1].text, "First entry");
        assert_eq!(entries[2].text, "Second entry");
        assert_eq!(entries[3].text, "Third entry");
    }

    #[test]
    fn parse_memory_index_utf8_safe_truncation() {
        // Ensure UTF-8 chars are not split mid-byte when truncating
        let long_chinese = "测试".repeat(100); // 200 Chinese chars (600 bytes)
        let content = format!("- {long_chinese}\n");
        let entries = parse_memory_index_entries(&content);
        assert_eq!(entries.len(), 1);
        // Title should be truncated to 80 chars + ellipsis, no panic
        assert!(entries[0].title.chars().count() <= 81);
        // Summary should be truncated to 200 chars + ellipsis, no panic
        assert!(entries[0].summary.chars().count() <= 201);
    }

    #[test]
    fn parse_memory_index_single_line_no_trailing_newline() {
        // Common case: agent writes a single bullet with no trailing newline
        let content = "- Lone entry";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "Lone entry");
    }

    #[test]
    fn parse_memory_index_whitespace_only_bullets_filtered() {
        // Bullets that are just whitespace should not produce cards
        let content = "- \n-  \n- Real entry\n";
        let entries = parse_memory_index_entries(content);
        // "- " with nothing after still creates an entry but it's empty and gets filtered
        // Only the real entry remains
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "Real entry");
    }

    #[test]
    fn parse_memory_index_near_bullet_markers_coalesce_as_paragraph() {
        // Tokens that look bullet-ish but aren't real markers ("1 apple" with
        // no period, "-tight" with no space) are prose. With no blank lines
        // between them they form a single paragraph entry.
        let content = "1 apple\n-tight\n+plus\n> blockquote\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "1 apple\n-tight\n+plus\n> blockquote");
    }

    #[test]
    fn parse_memory_index_paragraphs_split_on_blank_lines() {
        // Two blank-line-separated prose blocks become two entries.
        // This is the primary shape produced by `append_memory`, which
        // joins successive appends with `\n\n`.
        let content = "User prefers dark mode\n\nDeploys on Mondays\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "User prefers dark mode");
        assert_eq!(entries[1].text, "Deploys on Mondays");
    }

    #[test]
    fn parse_memory_index_multiline_paragraph_coalesces() {
        // Non-blank lines inside a paragraph are joined with newlines — the
        // whole block is one entry, not one per line.
        let content = "First line of a note\ncontinues on line two\nand line three\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].text,
            "First line of a note\ncontinues on line two\nand line three"
        );
    }

    #[test]
    fn parse_memory_index_yaml_frontmatter_skipped() {
        // A `---` pair at the top of the document is YAML frontmatter and
        // never becomes an entry. A `---` elsewhere is a horizontal rule
        // (also skipped) and separates entries.
        let content = "\
---
title: My Memories
author: alice
---

Some prose after frontmatter.

- A bullet
";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "Some prose after frontmatter.");
        assert_eq!(entries[1].text, "A bullet");
    }

    #[test]
    fn parse_memory_index_nested_bullets_become_flat_entries() {
        // Nested bullets are flattened to top-level entries. This keeps the
        // parser simple and matches how users typically read a memory list.
        let content = "- Parent\n  - Child entry\n- Sibling\n";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].text, "Parent");
        assert_eq!(entries[1].text, "Child entry");
        assert_eq!(entries[2].text, "Sibling");
    }

    #[test]
    fn truncate_chars_utf8_safe() {
        // Multi-byte characters must not panic
        let s = "héllo wörld";
        assert_eq!(truncate_chars(s, 5), "héllo…");
        // Exactly at limit — no ellipsis
        assert_eq!(truncate_chars("abc", 3), "abc");
        assert_eq!(truncate_chars("abc", 10), "abc");
    }

    #[test]
    fn truncate_chars_empty() {
        assert_eq!(truncate_chars("", 10), "");
    }

    // Integration-style test: verifies the pipeline from MEMORY.md content
    // to the shape the cards handler will produce. This protects the contract
    // between the parser and the handler's synthetic-path construction.
    #[test]
    fn memory_index_to_cards_contract() {
        let content = "\
# User's memories

- [Preference A](a.md) — Uses tabs over spaces
- [Preference B](b.md) — Deploys on Mondays
";
        let entries = parse_memory_index_entries(content);
        assert_eq!(entries.len(), 2);

        // Simulate what the handler does: build synthetic paths
        let synthetic_paths: Vec<String> = entries
            .iter()
            .enumerate()
            .map(|(i, _)| format!("{MEMORY_INDEX_PATH}#entry-{i}"))
            .collect();
        assert_eq!(synthetic_paths[0], "MEMORY.md#entry-0");
        assert_eq!(synthetic_paths[1], "MEMORY.md#entry-1");

        // Synthetic paths round-trip through split_once('#')
        for (i, p) in synthetic_paths.iter().enumerate() {
            let (base, frag) = p.split_once('#').unwrap();
            assert_eq!(base, "MEMORY.md");
            assert_eq!(frag, format!("entry-{i}"));
            let parsed_idx: usize = frag.strip_prefix("entry-").unwrap().parse().unwrap();
            assert_eq!(parsed_idx, i);
        }

        // The parsed entry at index N matches what the read handler would return
        for (i, entry) in entries.iter().enumerate() {
            // Re-parse the content (same as what the read handler does) and
            // verify indexing is stable.
            let re_parsed = parse_memory_index_entries(content);
            assert_eq!(re_parsed[i].text, entry.text);
        }
    }

    #[test]
    fn fallback_metadata_chinese_content() {
        let content = "# 部署指南\n\n这是关于K8s部署的笔记。需要注意内存配置。\n";
        let meta = generate_fallback_metadata(content, "deploy.md");
        assert_eq!(meta.card_title, "部署指南");
        assert!(meta.card_summary.contains("K8s"));
    }
}

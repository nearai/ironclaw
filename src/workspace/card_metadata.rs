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
const HIDDEN_FROM_CARDS: &[&str] = &[
    "IDENTITY.md",
    "SOUL.md",
    "AGENTS.md",
    "USER.md",
    "TOOLS.md",
    "BOOTSTRAP.md",
    "MEMORY.md",
    "HEARTBEAT.md",
    "README.md",
    ".config",
];

/// Check if a path should be hidden from the knowledge card view.
pub fn is_hidden_from_cards(path: &str) -> bool {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    HIDDEN_FROM_CARDS.contains(&file_name)
        || path.starts_with("context/")
        || file_name == ".config"
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
        obj.insert(
            "card_tags".to_string(),
            serde_json::json!(card.card_tags),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_metadata_from_markdown() {
        let content = "# My Project Notes\n\nThis is about the deployment process.\nWe use Docker and K8s.\n";
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
        let response = "```json\n{\"title\": \"Test\", \"summary\": \"A test.\", \"tags\": [\"test\"]}\n```";
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
        assert!(is_hidden_from_cards("MEMORY.md"));
        assert!(is_hidden_from_cards("HEARTBEAT.md"));
        assert!(is_hidden_from_cards("context/profile.json"));
        assert!(is_hidden_from_cards("context/assistant-directives.md"));
        assert!(!is_hidden_from_cards("projects/notes.md"));
        assert!(!is_hidden_from_cards("daily/2026-01-01.md"));
    }

    #[test]
    fn fallback_metadata_chinese_content() {
        let content = "# 部署指南\n\n这是关于K8s部署的笔记。需要注意内存配置。\n";
        let meta = generate_fallback_metadata(content, "deploy.md");
        assert_eq!(meta.card_title, "部署指南");
        assert!(meta.card_summary.contains("K8s"));
    }
}

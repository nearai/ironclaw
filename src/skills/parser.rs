//! SKILL.md parser for the OpenClaw skill format.
//!
//! Parses files with YAML frontmatter delimited by `---` lines, followed by a
//! markdown prompt body.

use crate::skills::{SkillManifest, validate_skill_name};

/// Error type for SKILL.md parsing failures.
#[derive(Debug, thiserror::Error)]
pub enum SkillParseError {
    #[error("Missing YAML frontmatter delimiters (expected `---` at start of file)")]
    MissingFrontmatter,

    #[error("Invalid YAML frontmatter: {0}")]
    InvalidYaml(String),

    #[error("Prompt body is empty (no content after frontmatter)")]
    EmptyPrompt,

    #[error("Invalid skill name '{name}': must match [a-zA-Z0-9][a-zA-Z0-9._-]{{0,63}}")]
    InvalidName { name: String },

    #[error("SKILL.md too large: {size} bytes (max {max} bytes)")]
    FileTooLarge { size: u64, max: u64 },
}

/// Result of parsing a SKILL.md file.
#[derive(Debug)]
pub struct ParsedSkill {
    /// Parsed manifest from YAML frontmatter.
    pub manifest: SkillManifest,
    /// Prompt content (markdown body after frontmatter).
    pub prompt_content: String,
}

/// Parse a SKILL.md file from its raw content string.
///
/// Expected format:
/// ```text
/// ---
/// name: my-skill
/// description: Does something
/// activation:
///   keywords: ["foo", "bar"]
/// ---
///
/// You are a helpful assistant that...
/// ```
pub fn parse_skill_md(content: &str) -> Result<ParsedSkill, SkillParseError> {
    // Strip optional UTF-8 BOM
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);

    // Find the first `---` delimiter (must be at line 1)
    let trimmed = content.trim_start_matches(['\n', '\r']);
    if !trimmed.starts_with("---") {
        return Err(SkillParseError::MissingFrontmatter);
    }

    // Find the second `---` delimiter
    let after_first = &trimmed[3..];
    // Skip the rest of the first `---` line (including any trailing chars/newline)
    let after_first_line = match after_first.find('\n') {
        Some(pos) => &after_first[pos + 1..],
        None => return Err(SkillParseError::MissingFrontmatter),
    };

    // Find closing `---` on its own line
    let yaml_end =
        find_closing_delimiter(after_first_line).ok_or(SkillParseError::MissingFrontmatter)?;

    let yaml_str = &after_first_line[..yaml_end];

    // Parse YAML frontmatter
    let mut manifest: SkillManifest =
        serde_yaml::from_str(yaml_str).map_err(|e| SkillParseError::InvalidYaml(e.to_string()))?;

    // Validate skill name
    if !validate_skill_name(&manifest.name) {
        return Err(SkillParseError::InvalidName {
            name: manifest.name.clone(),
        });
    }

    // Enforce activation criteria limits
    manifest.activation.enforce_limits();

    // Extract prompt content (everything after the closing `---` line)
    let after_yaml = &after_first_line[yaml_end..];
    // Skip the `---` line itself
    let prompt_start = after_yaml
        .find('\n')
        .map(|p| p + 1)
        .unwrap_or(after_yaml.len());
    let prompt_content = after_yaml[prompt_start..]
        .trim_start_matches('\n')
        .to_string();

    if prompt_content.trim().is_empty() {
        return Err(SkillParseError::EmptyPrompt);
    }

    Ok(ParsedSkill {
        manifest,
        prompt_content,
    })
}

/// Find the position of a closing `---` delimiter on its own line.
/// Returns the byte offset of the start of the `---` line within `content`.
fn find_closing_delimiter(content: &str) -> Option<usize> {
    let mut pos = 0;
    for line in content.lines() {
        if line.trim() == "---" {
            return Some(pos);
        }
        pos += line.len() + 1; // +1 for newline
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_full() {
        let content = r#"---
name: writing-assistant
version: "1.0.0"
description: Professional writing help
activation:
  keywords: ["write", "edit", "proofread"]
  max_context_tokens: 2000
metadata:
  openclaw:
    requires:
      bins: ["vale"]
      env: ["VALE_CONFIG"]
---

You are a writing assistant. When the user asks to write or edit...
"#;
        let result = parse_skill_md(content).expect("should parse");
        assert_eq!(result.manifest.name, "writing-assistant");
        assert_eq!(result.manifest.version, "1.0.0");
        assert_eq!(result.manifest.activation.keywords.len(), 3);
        assert!(result.prompt_content.starts_with("You are a writing"));

        let meta = result.manifest.metadata.unwrap();
        let openclaw = meta.openclaw.unwrap();
        assert_eq!(openclaw.requires.bins, vec!["vale"]);
    }

    #[test]
    fn test_parse_minimal() {
        let content = "---\nname: minimal\n---\n\nHello world.\n";
        let result = parse_skill_md(content).expect("should parse");
        assert_eq!(result.manifest.name, "minimal");
        assert_eq!(result.manifest.version, "0.0.0"); // default
        assert_eq!(result.prompt_content.trim(), "Hello world.");
    }

    #[test]
    fn test_missing_frontmatter() {
        let content = "Just some markdown text without frontmatter.";
        let err = parse_skill_md(content).unwrap_err();
        assert!(matches!(err, SkillParseError::MissingFrontmatter));
    }

    #[test]
    fn test_malformed_yaml() {
        let content = "---\nname: [invalid yaml\n---\n\nPrompt text.\n";
        let err = parse_skill_md(content).unwrap_err();
        assert!(matches!(err, SkillParseError::InvalidYaml(_)));
    }

    #[test]
    fn test_empty_body() {
        let content = "---\nname: empty-body\n---\n\n   \n";
        let err = parse_skill_md(content).unwrap_err();
        assert!(matches!(err, SkillParseError::EmptyPrompt));
    }

    #[test]
    fn test_invalid_name() {
        let content = "---\nname: has spaces\n---\n\nPrompt.\n";
        let err = parse_skill_md(content).unwrap_err();
        assert!(matches!(err, SkillParseError::InvalidName { .. }));
    }

    #[test]
    fn test_activation_with_patterns_and_tags() {
        let content = r#"---
name: regex-skill
activation:
  keywords: ["test"]
  patterns: ["(?i)\\bwrite\\b"]
  tags: ["writing", "email"]
---

Test prompt.
"#;
        let result = parse_skill_md(content).expect("should parse");
        assert_eq!(result.manifest.activation.patterns.len(), 1);
        assert_eq!(result.manifest.activation.tags.len(), 2);
    }

    #[test]
    fn test_bom_handling() {
        let content = "\u{feff}---\nname: bom-skill\n---\n\nPrompt with BOM.\n";
        let result = parse_skill_md(content).expect("should handle BOM");
        assert_eq!(result.manifest.name, "bom-skill");
    }
}

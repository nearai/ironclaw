//! Secure prompt-based skills system for IronClaw.
//!
//! Skills are directories containing a `skill.toml` manifest and `prompt.md` content
//! file that extend the agent's behavior through prompt-level instructions. Unlike
//! code-level tools (WASM/MCP), skills operate in the LLM context and are subject to
//! trust-based authority attenuation.
//!
//! # Trust Model
//!
//! Skills have three trust tiers that determine their authority:
//! - **Local**: User-placed skills with full trust (all tools available)
//! - **Verified**: Signed skills from known publishers (declared tools + read-only)
//! - **Community**: Untrusted marketplace skills (read-only tools only)
//!
//! The effective tool ceiling is determined by the *lowest-trust* active skill,
//! preventing privilege escalation through skill mixing.

pub mod attenuation;
pub mod registry;
pub mod scanner;
pub mod selector;

pub use attenuation::{AttenuationResult, attenuate_tools};
pub use registry::SkillRegistry;
pub use scanner::{SkillScanResult, SkillScanner};
pub use selector::prefilter_skills;

use std::path::PathBuf;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Maximum number of keywords allowed per skill to prevent scoring manipulation.
const MAX_KEYWORDS_PER_SKILL: usize = 20;

/// Maximum number of regex patterns allowed per skill.
const MAX_PATTERNS_PER_SKILL: usize = 5;

/// Maximum file size for prompt.md (64 KiB).
pub const MAX_PROMPT_FILE_SIZE: u64 = 64 * 1024;

/// Regex for validating skill names: alphanumeric, hyphens, underscores, dots.
static SKILL_NAME_PATTERN: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$").unwrap());

/// Validate a skill name against the allowed pattern.
pub fn validate_skill_name(name: &str) -> bool {
    SKILL_NAME_PATTERN.is_match(name)
}

/// Trust tier for a skill, determining its authority ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillTrust {
    /// Untrusted marketplace skill. Read-only tools only.
    Community = 0,
    /// Signed skill from a known publisher. Declared tools + read-only.
    Verified = 1,
    /// User-placed local skill. Full trust, all tools available.
    Local = 2,
}

impl std::fmt::Display for SkillTrust {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Community => write!(f, "community"),
            Self::Verified => write!(f, "verified"),
            Self::Local => write!(f, "local"),
        }
    }
}

/// Where a skill was loaded from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    /// Local filesystem (~/.ironclaw/skills/).
    Local(PathBuf),
    /// Downloaded from marketplace.
    Marketplace { url: String },
}

/// Activation criteria parsed from skill.toml `[activation]` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivationCriteria {
    /// Keywords that trigger this skill (exact and substring match).
    /// Capped at `MAX_KEYWORDS_PER_SKILL` during loading.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Regex patterns for more complex matching.
    /// Capped at `MAX_PATTERNS_PER_SKILL` during loading.
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Tags for broad category matching.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Maximum context tokens this skill's prompt should consume.
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
}

impl ActivationCriteria {
    /// Enforce limits on keywords and patterns to prevent scoring manipulation.
    pub fn enforce_limits(&mut self) {
        self.keywords.truncate(MAX_KEYWORDS_PER_SKILL);
        self.patterns.truncate(MAX_PATTERNS_PER_SKILL);
    }
}

fn default_max_context_tokens() -> usize {
    2000
}

/// A tool permission request declared in skill.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermissionDeclaration {
    /// Why this tool is needed.
    #[serde(default)]
    pub reason: String,
    /// Constrained parameter patterns (e.g., allowed commands).
    #[serde(default)]
    pub allowed_patterns: Vec<serde_json::Value>,
}

/// Parsed skill manifest from `skill.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    /// Core skill metadata.
    pub skill: SkillMeta,
    /// Activation criteria.
    #[serde(default)]
    pub activation: ActivationCriteria,
    /// Tool permissions requested by this skill (tool_name -> declaration).
    #[serde(default)]
    pub permissions: std::collections::HashMap<String, ToolPermissionDeclaration>,
    /// Integrity information.
    #[serde(default)]
    pub integrity: IntegrityInfo,
}

/// Core skill metadata from `[skill]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

/// Integrity information from `[integrity]` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntegrityInfo {
    /// SHA-256 hash of the prompt content (format: "sha256:hex...").
    #[serde(default)]
    pub prompt_hash: Option<String>,
}

/// A fully loaded skill ready for activation.
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    /// Parsed manifest.
    pub manifest: SkillManifest,
    /// Raw prompt content from `prompt.md`.
    pub prompt_content: String,
    /// Trust tier (determined by source).
    pub trust: SkillTrust,
    /// Where this skill was loaded from.
    pub source: SkillSource,
    /// SHA-256 hash of the prompt content (computed at load time).
    pub content_hash: String,
    /// Scanner warnings from loading (empty = clean).
    pub scan_warnings: Vec<String>,
    /// Pre-compiled regex patterns from activation criteria (compiled at load time).
    pub compiled_patterns: Vec<Regex>,
}

impl LoadedSkill {
    /// Get the skill name.
    pub fn name(&self) -> &str {
        &self.manifest.skill.name
    }

    /// Get the skill version.
    pub fn version(&self) -> &str {
        &self.manifest.skill.version
    }

    /// Get the declared tool permissions.
    pub fn declared_tools(&self) -> Vec<&str> {
        self.manifest
            .permissions
            .keys()
            .map(|s| s.as_str())
            .collect()
    }

    /// Compile regex patterns from activation criteria. Invalid patterns are logged and skipped.
    pub fn compile_patterns(patterns: &[String]) -> Vec<Regex> {
        patterns
            .iter()
            .filter_map(|p| match Regex::new(p) {
                Ok(re) => Some(re),
                Err(e) => {
                    tracing::warn!("Invalid activation regex pattern '{}': {}", p, e);
                    None
                }
            })
            .collect()
    }
}

/// Escape a string for safe inclusion in XML attributes.
/// Prevents attribute injection attacks via skill name/version fields.
pub fn escape_xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape prompt content to prevent tag breakout from `<skill>` delimiters.
/// Replaces `</skill` sequences that could close the wrapper tag.
pub fn escape_skill_content(content: &str) -> String {
    // Replace closing tag variants that could break structural isolation
    content
        .replace("</skill>", "&lt;/skill&gt;")
        .replace("</skill ", "&lt;/skill ")
        .replace("</SKILL>", "&lt;/SKILL&gt;")
        .replace("</SKILL ", "&lt;/SKILL ")
        .replace("</Skill>", "&lt;/Skill&gt;")
        .replace("</Skill ", "&lt;/Skill ")
}

/// Normalize line endings to LF before hashing to ensure cross-platform consistency.
pub fn normalize_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_trust_ordering() {
        assert!(SkillTrust::Community < SkillTrust::Verified);
        assert!(SkillTrust::Verified < SkillTrust::Local);
    }

    #[test]
    fn test_skill_trust_display() {
        assert_eq!(SkillTrust::Community.to_string(), "community");
        assert_eq!(SkillTrust::Verified.to_string(), "verified");
        assert_eq!(SkillTrust::Local.to_string(), "local");
    }

    #[test]
    fn test_parse_skill_manifest() {
        let toml_str = r#"
[skill]
name = "writing-assistant"
version = "1.0.0"
description = "Professional writing and editing"
author = "test"
tags = ["writing", "editing"]

[activation]
keywords = ["write", "edit", "proofread"]
patterns = ["(?i)\\b(write|draft)\\b.*\\b(email|letter)\\b"]
max_context_tokens = 2000

[permissions.shell]
reason = "Run grammar tools"
allowed_patterns = [{command = "vale *"}]

[integrity]
prompt_hash = "sha256:abc123"
"#;
        let manifest: SkillManifest = toml::from_str(toml_str).expect("parse failed");
        assert_eq!(manifest.skill.name, "writing-assistant");
        assert_eq!(manifest.activation.keywords.len(), 3);
        assert!(manifest.permissions.contains_key("shell"));
        assert_eq!(
            manifest.integrity.prompt_hash,
            Some("sha256:abc123".to_string())
        );
    }

    #[test]
    fn test_loaded_skill_declared_tools() {
        let manifest: SkillManifest = toml::from_str(
            r#"
[skill]
name = "test"

[permissions.shell]
reason = "need shell"
[permissions.http]
reason = "need http"
"#,
        )
        .unwrap();

        let skill = LoadedSkill {
            manifest,
            prompt_content: "test prompt".to_string(),
            trust: SkillTrust::Local,
            source: SkillSource::Local(PathBuf::from("/tmp/test")),
            content_hash: "sha256:000".to_string(),
            scan_warnings: vec![],
            compiled_patterns: vec![],
        };

        let tools = skill.declared_tools();
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&"shell"));
        assert!(tools.contains(&"http"));
    }

    #[test]
    fn test_validate_skill_name_valid() {
        assert!(validate_skill_name("writing-assistant"));
        assert!(validate_skill_name("my_skill"));
        assert!(validate_skill_name("skill.v2"));
        assert!(validate_skill_name("a"));
        assert!(validate_skill_name("ABC123"));
    }

    #[test]
    fn test_validate_skill_name_invalid() {
        assert!(!validate_skill_name(""));
        assert!(!validate_skill_name("-starts-with-dash"));
        assert!(!validate_skill_name(".starts-with-dot"));
        assert!(!validate_skill_name("has spaces"));
        assert!(!validate_skill_name("has/slashes"));
        assert!(!validate_skill_name("has<angle>brackets"));
        assert!(!validate_skill_name("has\"quotes"));
        assert!(!validate_skill_name(
            "very-long-name-that-exceeds-the-sixty-four-character-limit-for-skill-names-wow"
        ));
    }

    #[test]
    fn test_escape_xml_attr() {
        assert_eq!(escape_xml_attr("normal"), "normal");
        assert_eq!(
            escape_xml_attr(r#"" trust="LOCAL"#),
            "&quot; trust=&quot;LOCAL"
        );
        assert_eq!(escape_xml_attr("<script>"), "&lt;script&gt;");
        assert_eq!(escape_xml_attr("a&b"), "a&amp;b");
    }

    #[test]
    fn test_escape_skill_content() {
        assert_eq!(escape_skill_content("normal text"), "normal text");
        assert_eq!(
            escape_skill_content("</skill>breakout"),
            "&lt;/skill&gt;breakout"
        );
        assert_eq!(escape_skill_content("</SKILL>UPPER"), "&lt;/SKILL&gt;UPPER");
    }

    #[test]
    fn test_normalize_line_endings() {
        assert_eq!(normalize_line_endings("a\r\nb\r\n"), "a\nb\n");
        assert_eq!(normalize_line_endings("a\rb\r"), "a\nb\n");
        assert_eq!(normalize_line_endings("a\nb\n"), "a\nb\n");
    }

    #[test]
    fn test_enforce_keyword_limits() {
        let mut criteria = ActivationCriteria {
            keywords: (0..30).map(|i| format!("kw{}", i)).collect(),
            patterns: (0..10).map(|i| format!("pat{}", i)).collect(),
            ..Default::default()
        };
        criteria.enforce_limits();
        assert_eq!(criteria.keywords.len(), MAX_KEYWORDS_PER_SKILL);
        assert_eq!(criteria.patterns.len(), MAX_PATTERNS_PER_SKILL);
    }

    #[test]
    fn test_compile_patterns() {
        let patterns = vec![
            r"(?i)\bwrite\b".to_string(),
            "[invalid".to_string(), // bad regex
            r"(?i)\bedit\b".to_string(),
        ];
        let compiled = LoadedSkill::compile_patterns(&patterns);
        assert_eq!(compiled.len(), 2); // invalid one skipped
    }
}

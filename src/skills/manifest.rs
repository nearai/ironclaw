//! Skill manifest: TOML-based definition of a skill's metadata, permissions, and prompt.

use serde::{Deserialize, Serialize};

use crate::skills::SkillError;

/// A skill manifest parsed from TOML.
///
/// Example:
/// ```toml
/// [skill]
/// name = "pr-review"
/// version = "1.0.0"
/// description = "Reviews GitHub pull requests for code quality"
/// author = "alice"
/// command = "review"
/// activation = "command"
///
/// [permissions]
/// tools = ["http", "json", "memory_search"]
/// domains = ["api.github.com"]
/// workspace_read = ["projects/"]
/// max_tool_calls = 15
///
/// [prompt]
/// content = "You are reviewing a GitHub pull request..."
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub skill: SkillMeta,
    #[serde(default)]
    pub permissions: SkillPermissions,
    pub prompt: SkillPrompt,
}

/// Core metadata for a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: Option<String>,
    pub source_url: Option<String>,
    /// Slash command binding (e.g. "review" -> user types /review).
    pub command: Option<String>,
    /// How the skill is activated. Defaults to "explicit".
    #[serde(default)]
    pub activation: ActivationMode,
}

/// How the skill gets activated.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivationMode {
    /// User must explicitly activate via `/skill activate <name>`.
    #[default]
    Explicit,
    /// Activated via slash command defined in `command` field.
    Command,
}

/// Permissions declared by a skill (sandbox boundaries).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillPermissions {
    /// Tool whitelist. Only these tools are visible when the skill is active.
    #[serde(default)]
    pub tools: Vec<String>,
    /// HTTP domains the skill can reach.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Workspace paths the skill can read (prefix match).
    #[serde(default)]
    pub workspace_read: Vec<String>,
    /// Max tool calls per turn (budget cap).
    pub max_tool_calls: Option<u32>,
}

/// The skill's prompt content injected into LLM context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPrompt {
    pub content: String,
}

impl SkillManifest {
    /// Parse a skill manifest from TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, SkillError> {
        let manifest: SkillManifest =
            toml::from_str(toml_str).map_err(|e| SkillError::ParseError {
                reason: e.to_string(),
            })?;

        manifest.validate()?;
        Ok(manifest)
    }

    /// Serialize this manifest to TOML string.
    pub fn to_toml(&self) -> Result<String, SkillError> {
        toml::to_string_pretty(self).map_err(|e| SkillError::Serialization {
            reason: e.to_string(),
        })
    }

    /// Convenience accessor for the skill name.
    pub fn name(&self) -> &str {
        &self.skill.name
    }

    /// Convenience accessor for the slash command (if any).
    pub fn command(&self) -> Option<&str> {
        self.skill.command.as_deref()
    }

    /// Validate internal consistency.
    fn validate(&self) -> Result<(), SkillError> {
        if self.skill.name.is_empty() {
            return Err(SkillError::ParseError {
                reason: "Skill name cannot be empty".to_string(),
            });
        }

        if self.skill.version.is_empty() {
            return Err(SkillError::ParseError {
                reason: "Skill version cannot be empty".to_string(),
            });
        }

        if self.prompt.content.is_empty() {
            return Err(SkillError::ParseError {
                reason: "Skill prompt content cannot be empty".to_string(),
            });
        }

        // Command activation requires a command field
        if self.skill.activation == ActivationMode::Command && self.skill.command.is_none() {
            return Err(SkillError::ParseError {
                reason: "Skill with activation='command' must define a 'command' field".to_string(),
            });
        }

        // Skill name must be alphanumeric + hyphens (filesystem-safe)
        if !self
            .skill
            .name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(SkillError::ParseError {
                reason:
                    "Skill name must contain only alphanumeric characters, hyphens, and underscores"
                        .to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::skills::manifest::{ActivationMode, SkillManifest};

    #[test]
    fn test_parse_minimal_manifest() {
        let toml = r#"
[skill]
name = "test-skill"
version = "0.1.0"
description = "A test skill"

[prompt]
content = "Do the thing."
"#;
        let manifest = SkillManifest::from_toml(toml).expect("should parse");
        assert_eq!(manifest.name(), "test-skill");
        assert_eq!(manifest.skill.version, "0.1.0");
        assert_eq!(manifest.skill.activation, ActivationMode::Explicit);
        assert!(manifest.permissions.tools.is_empty());
        assert!(manifest.permissions.max_tool_calls.is_none());
        assert_eq!(manifest.prompt.content, "Do the thing.");
    }

    #[test]
    fn test_parse_full_manifest() {
        let toml = r#"
[skill]
name = "pr-review"
version = "1.0.0"
description = "Reviews GitHub pull requests"
author = "alice"
source_url = "https://github.com/alice/skills"
command = "review"
activation = "command"

[permissions]
tools = ["http", "json", "memory_search"]
domains = ["api.github.com", "github.com"]
workspace_read = ["projects/", "context/"]
max_tool_calls = 15

[prompt]
content = "You are reviewing a pull request."
"#;
        let manifest = SkillManifest::from_toml(toml).expect("should parse");
        assert_eq!(manifest.name(), "pr-review");
        assert_eq!(manifest.skill.activation, ActivationMode::Command);
        assert_eq!(manifest.command(), Some("review"));
        assert_eq!(
            manifest.permissions.tools,
            vec!["http", "json", "memory_search"]
        );
        assert_eq!(
            manifest.permissions.domains,
            vec!["api.github.com", "github.com"]
        );
        assert_eq!(
            manifest.permissions.workspace_read,
            vec!["projects/", "context/"]
        );
        assert_eq!(manifest.permissions.max_tool_calls, Some(15));
    }

    #[test]
    fn test_parse_rejects_empty_name() {
        let toml = r#"
[skill]
name = ""
version = "1.0.0"
description = "Bad"

[prompt]
content = "Something"
"#;
        assert!(SkillManifest::from_toml(toml).is_err());
    }

    #[test]
    fn test_parse_rejects_empty_prompt() {
        let toml = r#"
[skill]
name = "test"
version = "1.0.0"
description = "Bad"

[prompt]
content = ""
"#;
        assert!(SkillManifest::from_toml(toml).is_err());
    }

    #[test]
    fn test_parse_rejects_command_without_command_field() {
        let toml = r#"
[skill]
name = "test"
version = "1.0.0"
description = "Bad"
activation = "command"

[prompt]
content = "Something"
"#;
        assert!(SkillManifest::from_toml(toml).is_err());
    }

    #[test]
    fn test_parse_rejects_unsafe_name() {
        let toml = r#"
[skill]
name = "../escape"
version = "1.0.0"
description = "Bad"

[prompt]
content = "Something"
"#;
        assert!(SkillManifest::from_toml(toml).is_err());
    }

    #[test]
    fn test_roundtrip_toml() {
        let toml = r#"
[skill]
name = "roundtrip"
version = "1.0.0"
description = "Test roundtrip"

[permissions]
tools = ["echo"]

[prompt]
content = "Hello."
"#;
        let manifest = SkillManifest::from_toml(toml).expect("should parse");
        let serialized = manifest.to_toml().expect("should serialize");
        let reparsed = SkillManifest::from_toml(&serialized).expect("should reparse");
        assert_eq!(reparsed.name(), "roundtrip");
        assert_eq!(reparsed.permissions.tools, vec!["echo"]);
    }

    #[test]
    fn test_invalid_toml_syntax() {
        let toml = "this is not valid toml {{{";
        assert!(SkillManifest::from_toml(toml).is_err());
    }

    #[test]
    fn test_missing_required_sections() {
        // Missing [prompt] section
        let toml = r#"
[skill]
name = "test"
version = "1.0.0"
description = "No prompt"
"#;
        assert!(SkillManifest::from_toml(toml).is_err());
    }

    #[test]
    fn test_default_permissions() {
        let toml = r#"
[skill]
name = "minimal"
version = "1.0.0"
description = "Minimal"

[prompt]
content = "Do stuff."
"#;
        let manifest = SkillManifest::from_toml(toml).expect("should parse");
        assert!(manifest.permissions.tools.is_empty());
        assert!(manifest.permissions.domains.is_empty());
        assert!(manifest.permissions.workspace_read.is_empty());
        assert!(manifest.permissions.max_tool_calls.is_none());
    }
}

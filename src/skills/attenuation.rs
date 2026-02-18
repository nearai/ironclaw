//! Trust-based tool filtering (authority attenuation).
//!
//! The core defense mechanism: the minimum trust level of any active skill
//! determines a *tool ceiling* -- tools above the ceiling are removed from
//! the LLM's tool list entirely. The LLM cannot be manipulated into calling
//! a tool it doesn't know exists.
//!
//! | Trust State        | Tool Ceiling                                      |
//! |--------------------|---------------------------------------------------|
//! | No skills active   | All tools (normal behavior)                       |
//! | Trusted only       | All tools (user placed these, full trust)         |
//! | Installed present  | Read-only tools ONLY                              |

use crate::llm::ToolDefinition;
use crate::skills::{LoadedSkill, SkillTrust};

/// Tools that are always safe -- read-only, no side effects.
///
/// **Maintenance note**: This list is intentionally hardcoded and conservative.
/// When adding new tools to IronClaw, they default to *excluded* from the
/// read-only list (i.e., blocked under Installed ceilings). A tool
/// should only be added here if it is provably free of side effects -- it must
/// not write files, make network requests, execute commands, or modify any state.
/// Review by the security team is required before expanding this list.
///
const READ_ONLY_TOOLS: &[&str] = &[
    "memory_search",
    "memory_read",
    "memory_tree",
    "time",
    "echo",
    "json",
    "skill_list",
    "skill_search",
];

/// Result of tool attenuation, including transparency information.
#[derive(Debug, Clone)]
pub struct AttenuationResult {
    /// The filtered tool definitions to send to the LLM.
    pub tools: Vec<ToolDefinition>,
    /// The minimum trust level across all active skills.
    pub min_trust: SkillTrust,
    /// Human-readable explanation of what was removed and why.
    pub explanation: String,
    /// Names of tools that were removed.
    pub removed_tools: Vec<String>,
}

/// Filter tool definitions based on the trust level of active skills.
///
/// This is the hard security gate: tools above the trust ceiling are removed
/// from the tool list before it reaches the LLM. The LLM cannot call tools
/// it doesn't know exist, regardless of what a skill prompt instructs.
pub fn attenuate_tools(
    tools: &[ToolDefinition],
    active_skills: &[LoadedSkill],
) -> AttenuationResult {
    // No active skills = no attenuation
    if active_skills.is_empty() {
        return AttenuationResult {
            tools: tools.to_vec(),
            min_trust: SkillTrust::Trusted,
            explanation: "No skills active, all tools available".to_string(),
            removed_tools: vec![],
        };
    }

    // Compute minimum trust across all active skills
    let min_trust = active_skills
        .iter()
        .map(|s| s.trust)
        .min()
        .unwrap_or(SkillTrust::Trusted);

    match min_trust {
        SkillTrust::Trusted => {
            // Trusted skills have full trust -- no filtering
            AttenuationResult {
                tools: tools.to_vec(),
                min_trust,
                explanation: "All active skills are trusted (full trust), all tools available"
                    .to_string(),
                removed_tools: vec![],
            }
        }
        SkillTrust::Installed => {
            // Installed: read-only tools ONLY
            let mut kept = Vec::new();
            let mut removed = Vec::new();

            for tool in tools {
                if READ_ONLY_TOOLS.contains(&tool.name.as_str()) {
                    kept.push(tool.clone());
                } else {
                    removed.push(tool.name.clone());
                }
            }

            let explanation = format!(
                "Installed skill present: restricted to read-only tools, removed {} tool(s): {}",
                removed.len(),
                removed.join(", ")
            );

            AttenuationResult {
                tools: kept,
                min_trust,
                explanation,
                removed_tools: removed,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{ActivationCriteria, SkillManifest, SkillSource};
    use std::path::PathBuf;

    fn make_tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("{} tool", name),
            parameters: serde_json::json!({}),
        }
    }

    fn make_skill_with_trust(name: &str, trust: SkillTrust) -> LoadedSkill {
        LoadedSkill {
            manifest: SkillManifest {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: String::new(),
                activation: ActivationCriteria::default(),
                metadata: None,
            },
            prompt_content: "test".to_string(),
            trust,
            source: SkillSource::User(PathBuf::from("/tmp")),
            content_hash: "sha256:000".to_string(),
            compiled_patterns: vec![],
            lowercased_keywords: vec![],
            lowercased_tags: vec![],
        }
    }

    fn all_tools() -> Vec<ToolDefinition> {
        vec![
            make_tool("shell"),
            make_tool("http"),
            make_tool("memory_write"),
            make_tool("memory_search"),
            make_tool("memory_read"),
            make_tool("memory_tree"),
            make_tool("time"),
            make_tool("echo"),
            make_tool("json"),
        ]
    }

    #[test]
    fn test_no_skills_returns_all_tools() {
        let tools = all_tools();
        let result = attenuate_tools(&tools, &[]);
        assert_eq!(result.tools.len(), tools.len());
        assert!(result.removed_tools.is_empty());
    }

    #[test]
    fn test_trusted_skills_no_filtering() {
        let tools = all_tools();
        let skills = vec![make_skill_with_trust("trusted_skill", SkillTrust::Trusted)];
        let result = attenuate_tools(&tools, &skills);
        assert_eq!(result.tools.len(), tools.len());
        assert!(result.removed_tools.is_empty());
        assert_eq!(result.min_trust, SkillTrust::Trusted);
    }

    #[test]
    fn test_installed_only_read_only() {
        let tools = all_tools();
        let skills = vec![make_skill_with_trust(
            "installed_skill",
            SkillTrust::Installed,
        )];
        let result = attenuate_tools(&tools, &skills);

        let kept_names: Vec<&str> = result.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(!kept_names.contains(&"shell"));
        assert!(!kept_names.contains(&"http"));
        assert!(!kept_names.contains(&"memory_write"));
        assert!(kept_names.contains(&"memory_search"));
        assert!(kept_names.contains(&"memory_read"));
        assert!(kept_names.contains(&"time"));
        assert_eq!(result.min_trust, SkillTrust::Installed);
    }

    #[test]
    fn test_mixed_trust_drops_to_lowest() {
        let tools = all_tools();
        let skills = vec![
            make_skill_with_trust("trusted_skill", SkillTrust::Trusted),
            make_skill_with_trust("installed_skill", SkillTrust::Installed),
        ];
        let result = attenuate_tools(&tools, &skills);

        // Mixed: installed + trusted = installed ceiling
        assert_eq!(result.min_trust, SkillTrust::Installed);
        let kept_names: Vec<&str> = result.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(!kept_names.contains(&"shell"));
    }

    #[test]
    fn test_attenuation_result_has_explanation() {
        let tools = vec![make_tool("shell"), make_tool("time")];
        let skills = vec![make_skill_with_trust("installed", SkillTrust::Installed)];
        let result = attenuate_tools(&tools, &skills);

        assert!(!result.explanation.is_empty());
        assert!(result.removed_tools.contains(&"shell".to_string()));
        assert!(!result.removed_tools.contains(&"time".to_string()));
    }
}

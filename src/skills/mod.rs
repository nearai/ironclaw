//! Skills system for IronClaw.
//!
//! This module contains main-crate skill logic that depends on types from
//! other `src/` modules (e.g. `crate::llm::ToolDefinition`, `crate::secrets`).
//! For core skill types, parsing, and registry, import from `ironclaw_skills` directly.
//!
//! The `attenuation` submodule remains here because it depends on
//! `crate::llm::ToolDefinition` which is a main-crate type.
//!
//! # V1 migration notes
//!
//! The following items in this module exist **only for the v1 agent** (`src/agent/`).
//! Once the v1 agent is removed and all users are on ENGINE_V2, they can be deleted:
//!
//! - **`attenuation` module** — Trust-based tool filtering. In v2, the Python
//!   orchestrator handles skill trust via the `format_skills()` function and
//!   the policy engine handles tool access via capability leases.
//! - **`register_skill_credentials()`** — Registers credential mappings from v1
//!   `LoadedSkill` into `SharedCredentialRegistry`. In v2, credentials are declared
//!   in the SKILL.md frontmatter and registered at migration time in `skill_migration.rs`.
//! - **`credential_spec_to_mapping()` / `convert_credential_location()`** — Conversion
//!   helpers used by `register_skill_credentials()`. Same lifecycle.
//! - **This entire module** — Once v1 is gone, the remaining local items
//!   can be deleted and this file removed.
//!
//! The `ironclaw_skills` crate itself remains (types, parser, validation, v2 types).

pub mod attenuation;
pub mod bundled;

// Items from `ironclaw_skills` are no longer glob-re-exported.
// Callers should import from `ironclaw_skills` directly.

// Re-export attenuation at the same path as before.
pub use attenuation::{AttenuationResult, attenuate_tools};

use crate::secrets::{CredentialLocation, CredentialMapping};
use ironclaw_skills::{LoadedSkill, SkillCredentialLocation, SkillCredentialSpec};

/// Convert a skill credential location to the main crate's [`CredentialLocation`].
fn convert_credential_location(loc: &SkillCredentialLocation) -> CredentialLocation {
    match loc {
        SkillCredentialLocation::Bearer => CredentialLocation::AuthorizationBearer,
        SkillCredentialLocation::BasicAuth { username } => CredentialLocation::AuthorizationBasic {
            username: username.clone(),
        },
        SkillCredentialLocation::Header { name, prefix } => CredentialLocation::Header {
            name: name.clone(),
            prefix: prefix.clone(),
        },
        SkillCredentialLocation::QueryParam { name } => {
            CredentialLocation::QueryParam { name: name.clone() }
        }
    }
}

/// Convert a [`SkillCredentialSpec`] to a [`CredentialMapping`] for the
/// [`SharedCredentialRegistry`](crate::tools::wasm::SharedCredentialRegistry).
pub fn credential_spec_to_mapping(spec: &SkillCredentialSpec) -> CredentialMapping {
    CredentialMapping {
        secret_name: spec.name.clone(),
        location: convert_credential_location(&spec.location),
        host_patterns: spec.hosts.clone(),
    }
}

/// Register credential mappings from loaded skills into the shared registry.
///
/// Validates each spec before registration; invalid specs are logged and skipped.
pub fn register_skill_credentials(
    skills: &[LoadedSkill],
    registry: &crate::tools::wasm::SharedCredentialRegistry,
) {
    let mut count = 0usize;
    for skill in skills {
        for spec in &skill.manifest.credentials {
            let errors = ironclaw_skills::validation::validate_credential_spec(spec);
            if !errors.is_empty() {
                tracing::warn!(
                    skill = %skill.name(),
                    credential = %spec.name,
                    errors = ?errors,
                    "Skipping invalid credential spec"
                );
                continue;
            }
            let mapping = credential_spec_to_mapping(spec);
            tracing::debug!(
                skill = %skill.name(),
                credential = %spec.name,
                hosts = ?spec.hosts,
                "Registering skill credential mapping"
            );
            registry.add_mappings(std::iter::once(mapping));
            count += 1;
        }
    }
    if count > 0 {
        tracing::debug!(count, "Registered skill credential mappings");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_bearer_location() {
        let loc = ironclaw_skills::SkillCredentialLocation::Bearer;
        let converted = convert_credential_location(&loc);
        assert!(matches!(
            converted,
            crate::secrets::CredentialLocation::AuthorizationBearer
        ));
    }

    #[test]
    fn test_convert_basic_auth_location() {
        let loc = ironclaw_skills::SkillCredentialLocation::BasicAuth {
            username: "admin".to_string(),
        };
        let converted = convert_credential_location(&loc);
        match converted {
            crate::secrets::CredentialLocation::AuthorizationBasic { username } => {
                assert_eq!(username, "admin");
            }
            _ => panic!("expected AuthorizationBasic"),
        }
    }

    #[test]
    fn test_convert_header_location() {
        let loc = ironclaw_skills::SkillCredentialLocation::Header {
            name: "X-API-Key".to_string(),
            prefix: Some("Token".to_string()),
        };
        let converted = convert_credential_location(&loc);
        match converted {
            crate::secrets::CredentialLocation::Header { name, prefix } => {
                assert_eq!(name, "X-API-Key");
                assert_eq!(prefix, Some("Token".to_string()));
            }
            _ => panic!("expected Header"),
        }
    }

    #[test]
    fn test_convert_query_param_location() {
        let loc = ironclaw_skills::SkillCredentialLocation::QueryParam {
            name: "key".to_string(),
        };
        let converted = convert_credential_location(&loc);
        match converted {
            crate::secrets::CredentialLocation::QueryParam { name } => {
                assert_eq!(name, "key");
            }
            _ => panic!("expected QueryParam"),
        }
    }

    #[test]
    fn test_credential_spec_to_mapping() {
        let spec = ironclaw_skills::SkillCredentialSpec {
            name: "github_token".to_string(),
            provider: "github".to_string(),
            location: ironclaw_skills::SkillCredentialLocation::Bearer,
            hosts: vec!["api.github.com".to_string(), "*.github.com".to_string()],
            oauth: None,
            setup_instructions: None,
        };
        let mapping = super::credential_spec_to_mapping(&spec);
        assert_eq!(mapping.secret_name, "github_token");
        assert!(matches!(
            mapping.location,
            crate::secrets::CredentialLocation::AuthorizationBearer
        ));
        assert_eq!(mapping.host_patterns.len(), 2);
        assert_eq!(mapping.host_patterns[0], "api.github.com");
    }

    #[test]
    fn test_register_skill_credentials_valid() {
        use ironclaw_skills::types::*;
        use std::path::PathBuf;

        let skill = ironclaw_skills::LoadedSkill {
            manifest: SkillManifest {
                name: "test-api".to_string(),
                version: "1.0.0".to_string(),
                description: "Test".to_string(),
                activation: ActivationCriteria::default(),
                credentials: vec![SkillCredentialSpec {
                    name: "test_token".to_string(),
                    provider: "test".to_string(),
                    location: SkillCredentialLocation::Bearer,
                    hosts: vec!["api.test.com".to_string()],
                    oauth: None,
                    setup_instructions: None,
                }],
                requires: GatingRequirements::default(),
            },
            prompt_content: "test".to_string(),
            trust: SkillTrust::Trusted,
            source: SkillSource::User(PathBuf::from("/tmp/test")),
            content_hash: "sha256:000".to_string(),
            compiled_patterns: vec![],
            lowercased_keywords: vec![],
            lowercased_exclude_keywords: vec![],
            lowercased_tags: vec![],
        };

        let registry = crate::tools::wasm::SharedCredentialRegistry::new();
        register_skill_credentials(&[skill], &registry);

        assert!(registry.has_credentials_for_host("api.test.com"));
        assert!(!registry.has_credentials_for_host("other.host.com"));
    }

    #[test]
    fn test_register_skill_credentials_invalid_skipped() {
        use ironclaw_skills::types::*;
        use std::path::PathBuf;

        let skill = ironclaw_skills::LoadedSkill {
            manifest: SkillManifest {
                name: "bad-skill".to_string(),
                version: "1.0.0".to_string(),
                description: "Test".to_string(),
                activation: ActivationCriteria::default(),
                credentials: vec![SkillCredentialSpec {
                    name: "INVALID_NAME".to_string(), // uppercase = invalid
                    provider: "test".to_string(),
                    location: SkillCredentialLocation::Bearer,
                    hosts: vec!["api.test.com".to_string()],
                    oauth: None,
                    setup_instructions: None,
                }],
                requires: GatingRequirements::default(),
            },
            prompt_content: "test".to_string(),
            trust: SkillTrust::Trusted,
            source: SkillSource::User(PathBuf::from("/tmp/test")),
            content_hash: "sha256:000".to_string(),
            compiled_patterns: vec![],
            lowercased_keywords: vec![],
            lowercased_exclude_keywords: vec![],
            lowercased_tags: vec![],
        };

        let registry = crate::tools::wasm::SharedCredentialRegistry::new();
        register_skill_credentials(&[skill], &registry);

        // Invalid spec should be skipped — host should NOT be registered
        assert!(!registry.has_credentials_for_host("api.test.com"));
    }
}

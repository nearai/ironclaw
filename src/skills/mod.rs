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
use crate::tools::wasm::OAuthRefreshConfig;
use crate::{
    auth::{AuthDescriptor, AuthDescriptorKind, OAuthFlowDescriptor, upsert_auth_descriptor},
    db::SettingsStore,
};
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
        path_patterns: spec.path_patterns.clone(),
        // Skill credentials are required by default; the spec doesn't yet
        // expose an `optional` field, so we conservatively mark required.
        optional: false,
    }
}

fn credential_spec_to_oauth_refresh(spec: &SkillCredentialSpec) -> Option<OAuthRefreshConfig> {
    let oauth = spec.oauth.as_ref()?;
    match &oauth.refresh {
        ironclaw_skills::ProviderRefreshStrategy::ReauthorizeOnly => return None,
        ironclaw_skills::ProviderRefreshStrategy::Standard => {}
        ironclaw_skills::ProviderRefreshStrategy::Custom {
            refresh_url,
            extra_params,
        } => {
            let builtin = crate::auth::oauth::builtin_credentials(&spec.name);
            let exchange_proxy_url = crate::auth::oauth::exchange_proxy_url();
            let client_id = oauth
                .client_id
                .clone()
                .or_else(|| {
                    oauth
                        .client_id_env
                        .as_ref()
                        .and_then(|env| std::env::var(env).ok())
                })
                .or_else(|| builtin.as_ref().map(|c| c.client_id.to_string()))?;
            let client_secret = oauth
                .client_secret
                .clone()
                .or_else(|| {
                    oauth
                        .client_secret_env
                        .as_ref()
                        .and_then(|env| std::env::var(env).ok())
                })
                .or_else(|| builtin.as_ref().map(|c| c.client_secret.to_string()));
            let client_secret = crate::auth::oauth::hosted_proxy_client_secret(
                &client_secret,
                builtin.as_ref(),
                exchange_proxy_url.is_some(),
            );

            return Some(OAuthRefreshConfig {
                token_url: refresh_url.clone(),
                client_id,
                client_secret,
                exchange_proxy_url,
                gateway_token: crate::auth::oauth::oauth_proxy_auth_token(),
                secret_name: spec.name.clone(),
                provider: Some(spec.provider.clone()),
                extra_refresh_params: extra_params.clone(),
            });
        }
    }

    let builtin = crate::auth::oauth::builtin_credentials(&spec.name);
    let exchange_proxy_url = crate::auth::oauth::exchange_proxy_url();
    let client_id = oauth
        .client_id
        .clone()
        .or_else(|| {
            oauth
                .client_id_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok())
        })
        .or_else(|| builtin.as_ref().map(|c| c.client_id.to_string()))?;
    let client_secret = oauth
        .client_secret
        .clone()
        .or_else(|| {
            oauth
                .client_secret_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok())
        })
        .or_else(|| builtin.as_ref().map(|c| c.client_secret.to_string()));
    let client_secret = crate::auth::oauth::hosted_proxy_client_secret(
        &client_secret,
        builtin.as_ref(),
        exchange_proxy_url.is_some(),
    );

    Some(OAuthRefreshConfig {
        token_url: oauth.token_url.clone(),
        client_id,
        client_secret,
        exchange_proxy_url,
        gateway_token: crate::auth::oauth::oauth_proxy_auth_token(),
        secret_name: spec.name.clone(),
        provider: Some(spec.provider.clone()),
        extra_refresh_params: std::collections::HashMap::new(),
    })
}

fn credential_spec_to_auth_descriptor(
    skill_name: &str,
    spec: &SkillCredentialSpec,
) -> AuthDescriptor {
    AuthDescriptor {
        kind: AuthDescriptorKind::SkillCredential,
        secret_name: spec.name.clone(),
        integration_name: skill_name.to_string(),
        display_name: Some(spec.provider.clone()),
        provider: Some(spec.provider.clone()),
        setup_url: None,
        oauth: spec.oauth.as_ref().map(|oauth| OAuthFlowDescriptor {
            authorization_url: oauth.authorization_url.clone(),
            token_url: oauth.token_url.clone(),
            client_id: oauth.client_id.clone(),
            client_id_env: oauth.client_id_env.clone(),
            client_secret: oauth.client_secret.clone(),
            client_secret_env: oauth.client_secret_env.clone(),
            scopes: oauth.scopes.clone(),
            use_pkce: oauth.use_pkce,
            extra_params: oauth.extra_params.clone(),
            access_token_field: "access_token".to_string(),
            validation_url: oauth.test_url.clone(),
        }),
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
            if let Some(oauth) = credential_spec_to_oauth_refresh(spec) {
                registry
                    .add_oauth_refresh_configs(std::iter::once((oauth.secret_name.clone(), oauth)));
            }
            count += 1;
        }
    }
    if count > 0 {
        tracing::debug!(count, "Registered skill credential mappings");
    }
}

pub async fn persist_skill_auth_descriptors(
    skills: &[LoadedSkill],
    store: Option<&dyn SettingsStore>,
    user_id: &str,
) {
    for skill in skills {
        for spec in &skill.manifest.credentials {
            let errors = ironclaw_skills::validation::validate_credential_spec(spec);
            if !errors.is_empty() {
                continue;
            }

            let descriptor = credential_spec_to_auth_descriptor(skill.name(), spec);
            upsert_auth_descriptor(store, user_id, descriptor).await;
        }
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
            path_patterns: Vec::new(),
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
                    path_patterns: Vec::new(),
                    oauth: None,
                    setup_instructions: None,
                }],
                requires: GatingRequirements::default(),
            },
            prompt_content: "test".to_string(),
            trust: SkillTrust::Trusted,
            source: SkillSource::User(PathBuf::from("/tmp/test")), // safety: dummy path in test, not used for I/O
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
    fn test_register_skill_credentials_registers_oauth_refresh_config() {
        use ironclaw_skills::types::*;
        use std::path::PathBuf;

        let skill = ironclaw_skills::LoadedSkill {
            manifest: SkillManifest {
                name: "gmail".to_string(),
                version: "1.0.0".to_string(),
                description: "Test".to_string(),
                activation: ActivationCriteria::default(),
                credentials: vec![SkillCredentialSpec {
                    name: "google_oauth_token".to_string(),
                    provider: "google".to_string(),
                    location: SkillCredentialLocation::Bearer,
                    hosts: vec!["www.googleapis.com".to_string()],
                    path_patterns: Vec::new(),
                    oauth: Some(SkillOAuthConfig {
                        authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
                            .to_string(),
                        token_url: "https://oauth2.googleapis.com/token".to_string(),
                        client_id: Some("client-id".to_string()),
                        client_id_env: None,
                        client_secret: Some("client-secret".to_string()),
                        client_secret_env: None,
                        scopes: vec![],
                        use_pkce: true,
                        extra_params: std::collections::HashMap::new(),
                        refresh: ProviderRefreshStrategy::Standard,
                        test_url: None,
                    }),
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

        let oauth = registry
            .oauth_refresh_for_secret("google_oauth_token")
            .expect("oauth refresh config");
        assert_eq!(oauth.secret_name, "google_oauth_token");
        assert_eq!(oauth.token_url, "https://oauth2.googleapis.com/token");
        assert_eq!(oauth.client_id, "client-id");
        assert_eq!(oauth.client_secret.as_deref(), Some("client-secret"));
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
                    path_patterns: Vec::new(),
                    oauth: None,
                    setup_instructions: None,
                }],
                requires: GatingRequirements::default(),
            },
            prompt_content: "test".to_string(),
            trust: SkillTrust::Trusted,
            source: SkillSource::User(PathBuf::from("/tmp/test")), // safety: dummy path in test, not used for I/O
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

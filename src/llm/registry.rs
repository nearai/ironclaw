//! Declarative LLM provider registry.
//!
//! Providers are defined in JSON (compiled-in defaults + optional user file)
//! so adding a new OpenAI-compatible provider requires zero Rust code changes.
//!
//! ```text
//!   ┌─────────────────────┐    ┌──────────────────────────┐
//!   │  providers.json     │    │ ~/.ironclaw/providers.json│
//!   │  (built-in, embed)  │    │ (user overrides/extras)  │
//!   └────────┬────────────┘    └────────────┬─────────────┘
//!            │                              │
//!            └──────────┬───────────────────┘
//!                       ▼
//!              ┌──────────────────┐
//!              │ ProviderRegistry │
//!              │  .find("groq")   │──▶ ProviderDefinition
//!              │  .all()          │        ├ protocol
//!              │  .selectable()   │        ├ default_base_url
//!              └──────────────────┘        ├ api_key_env
//!                                          └ ...
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// API protocol a provider speaks.
///
/// Determines which rig-core client constructor to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    /// OpenAI Chat Completions API (`/v1/chat/completions`).
    /// Used by: OpenAI, Tinfoil, Groq, NVIDIA NIM, OpenRouter, etc.
    OpenAiCompletions,
    /// Anthropic Messages API.
    Anthropic,
    /// Ollama API (OpenAI-ish, no API key required).
    Ollama,
}

/// How the setup wizard should collect credentials for this provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SetupHint {
    /// Collect an API key and store it in the encrypted secrets store.
    ApiKey {
        /// Key name in the secrets store (e.g., "llm_groq_api_key").
        secret_name: String,
        /// URL where the user can generate an API key.
        #[serde(default)]
        key_url: Option<String>,
        /// Human-readable name for display in the wizard.
        display_name: String,
        /// Whether this provider supports `/v1/models` listing.
        #[serde(default)]
        can_list_models: bool,
        /// Optional filter for model listing (e.g., "chat").
        #[serde(default)]
        models_filter: Option<String>,
    },
    /// Ollama-style setup: just a base URL, no API key.
    Ollama {
        display_name: String,
        #[serde(default)]
        can_list_models: bool,
    },
    /// Generic OpenAI-compatible: ask for base URL + optional API key.
    OpenAiCompatible {
        secret_name: String,
        display_name: String,
        #[serde(default)]
        can_list_models: bool,
    },
}

impl SetupHint {
    pub fn display_name(&self) -> &str {
        match self {
            Self::ApiKey { display_name, .. } => display_name,
            Self::Ollama { display_name, .. } => display_name,
            Self::OpenAiCompatible { display_name, .. } => display_name,
        }
    }

    pub fn can_list_models(&self) -> bool {
        match self {
            Self::ApiKey {
                can_list_models, ..
            } => *can_list_models,
            Self::Ollama {
                can_list_models, ..
            } => *can_list_models,
            Self::OpenAiCompatible {
                can_list_models, ..
            } => *can_list_models,
        }
    }

    pub fn secret_name(&self) -> Option<&str> {
        match self {
            Self::ApiKey { secret_name, .. } => Some(secret_name),
            Self::OpenAiCompatible { secret_name, .. } => Some(secret_name),
            Self::Ollama { .. } => None,
        }
    }

    pub fn models_filter(&self) -> Option<&str> {
        match self {
            Self::ApiKey { models_filter, .. } => models_filter.as_deref(),
            _ => None,
        }
    }
}

/// Validates unsupported_params during deserialization.
///
/// Only allows: "temperature", "max_tokens", "stop_sequences".
/// Invalid parameter names cause a deserialization error.
mod unsupported_params_de {
    use serde::{Deserialize, Deserializer};

    const VALID_PARAMS: &[&str] = &["temperature", "max_tokens", "stop_sequences"];

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let params: Vec<String> = Deserialize::deserialize(deserializer)?;
        for param in &params {
            if !VALID_PARAMS.contains(&param.as_str()) {
                return Err(serde::de::Error::custom(format!(
                    "unsupported parameter name '{}': must be one of: {}",
                    param,
                    VALID_PARAMS.join(", ")
                )));
            }
        }
        Ok(params)
    }
}

/// Declarative definition of an LLM provider.
///
/// One JSON object in `providers.json` maps to one `ProviderDefinition`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDefinition {
    /// Unique identifier used in `LLM_BACKEND` (e.g., "groq", "tinfoil").
    pub id: String,
    /// Alternative names accepted in `LLM_BACKEND` (e.g., ["nvidia_nim", "nim"]).
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Which API protocol to use.
    pub protocol: ProviderProtocol,
    /// Default base URL. `None` means use the rig-core default for the protocol.
    #[serde(default)]
    pub default_base_url: Option<String>,
    /// Env var for base URL override (e.g., "OPENAI_BASE_URL").
    #[serde(default)]
    pub base_url_env: Option<String>,
    /// Whether a base URL is required (for generic openai_compatible).
    #[serde(default)]
    pub base_url_required: bool,
    /// Env var for the API key (e.g., "GROQ_API_KEY").
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Whether an API key is required to use this provider.
    #[serde(default)]
    pub api_key_required: bool,
    /// Env var for the model name (e.g., "GROQ_MODEL").
    pub model_env: String,
    /// Default model if none specified.
    pub default_model: String,
    /// Human-readable one-line description.
    pub description: String,
    /// Env var for extra HTTP headers (format: `Key:Value,Key2:Value2`).
    #[serde(default)]
    pub extra_headers_env: Option<String>,
    /// Setup wizard hints.
    #[serde(default)]
    pub setup: Option<SetupHint>,
    /// Parameter names that this provider does not support (e.g., `["temperature"]`).
    /// Supported keys: `"temperature"`, `"max_tokens"`, `"stop_sequences"`.
    /// Listed parameters are stripped from requests before sending to avoid 400 errors.
    /// Invalid parameter names cause a deserialization error.
    #[serde(default, deserialize_with = "unsupported_params_de::deserialize")]
    pub unsupported_params: Vec<String>,
}

/// Registry of known LLM providers.
///
/// Built from compiled-in `providers.json` plus optional user overrides
/// from `~/.ironclaw/providers.json`.
pub struct ProviderRegistry {
    providers: Vec<ProviderDefinition>,
    /// Lowercase id/alias → index into `providers`.
    lookup: HashMap<String, usize>,
}

impl ProviderRegistry {
    /// Build a registry from a list of provider definitions.
    ///
    /// Later entries with duplicate IDs/aliases override earlier ones.
    pub fn new(providers: Vec<ProviderDefinition>) -> Self {
        let mut lookup = HashMap::new();
        for (idx, def) in providers.iter().enumerate() {
            lookup.insert(def.id.to_lowercase(), idx);
            for alias in &def.aliases {
                lookup.insert(alias.to_lowercase(), idx);
            }
        }
        Self { providers, lookup }
    }

    /// Load the default registry: built-in providers + user overrides.
    ///
    /// User providers from `~/.ironclaw/providers.json` are appended,
    /// with later entries overriding earlier ones by ID/alias.
    pub fn load() -> Self {
        let builtins: Vec<ProviderDefinition> =
            serde_json::from_str(include_str!("../../providers.json"))
                .expect("built-in providers.json must be valid JSON"); // safety: compile-time embedded file

        let mut all = builtins;

        if let Some(user_path) = user_providers_path()
            && user_path.exists()
        {
            match std::fs::read_to_string(&user_path) {
                Ok(contents) => match serde_json::from_str::<Vec<ProviderDefinition>>(&contents) {
                    Ok(user_defs) => {
                        tracing::info!(
                            count = user_defs.len(),
                            path = %user_path.display(),
                            "Loaded user provider definitions"
                        );
                        all.extend(user_defs);
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %user_path.display(),
                            error = %e,
                            "Failed to parse user providers.json, skipping"
                        );
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        path = %user_path.display(),
                        error = %e,
                        "Failed to read user providers.json, skipping"
                    );
                }
            }
        }

        Self::new(all)
    }

    /// Look up a provider by ID or alias (case-insensitive).
    pub fn find(&self, id: &str) -> Option<&ProviderDefinition> {
        self.lookup
            .get(&id.to_lowercase())
            .map(|&idx| &self.providers[idx])
    }

    /// All registered providers (built-in + user).
    pub fn all(&self) -> &[ProviderDefinition] {
        &self.providers
    }

    /// Providers that should appear in the setup wizard's selection menu.
    ///
    /// Returns all providers that have a `setup` hint, in registry order.
    /// NearAI is not in the registry (handled specially) so it won't appear here.
    pub fn selectable(&self) -> Vec<&ProviderDefinition> {
        // Deduplicate: only keep the last definition for each ID
        let mut seen = HashMap::new();
        for def in &self.providers {
            seen.insert(def.id.as_str(), def);
        }
        // Preserve order of first appearance, but use the last (overridden)
        // definition for each ID. A user override that adds `setup` to a
        // provider that previously lacked it will be included correctly.
        let mut result = Vec::new();
        let mut emitted = std::collections::HashSet::new();
        for def in &self.providers {
            if emitted.insert(def.id.as_str()) {
                let final_def = seen[def.id.as_str()];
                if final_def.setup.is_some() {
                    result.push(final_def);
                }
            }
        }
        result
    }

    /// Check whether a backend string is a known provider (NearAI or registry).
    pub fn is_known(&self, backend: &str) -> bool {
        backend == "nearai"
            || backend == "near_ai"
            || backend == "near"
            || self.find(backend).is_some()
    }

    /// Get the model env var for a backend string.
    ///
    /// Returns the registry provider's `model_env` if found,
    /// or `"NEARAI_MODEL"` for the NearAI backend.
    pub fn model_env_var(&self, backend: &str) -> &str {
        if backend == "nearai" || backend == "near_ai" || backend == "near" {
            return "NEARAI_MODEL";
        }
        self.find(backend)
            .map(|def| def.model_env.as_str())
            .unwrap_or("LLM_MODEL")
    }
}

fn user_providers_path() -> Option<std::path::PathBuf> {
    Some(crate::bootstrap::ironclaw_base_dir().join("providers.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_registry_loads() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        assert!(
            registry.all().len() >= 5,
            "should have at least 5 built-in providers"
        );
    }

    #[test]
    fn test_find_by_id() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        let openai = registry.find("openai").expect("openai should exist");
        assert_eq!(openai.id, "openai");
        assert_eq!(openai.protocol, ProviderProtocol::OpenAiCompletions);
    }

    #[test]
    fn test_find_by_alias() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        let openai = registry
            .find("open_ai")
            .expect("alias open_ai should resolve");
        assert_eq!(openai.id, "openai");
    }

    #[test]
    fn test_novita_provider_metadata() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        let novita = registry.find("novita").expect("novita should exist");
        assert_eq!(novita.id, "novita");
        assert_eq!(
            novita.default_base_url.as_deref(),
            Some("https://api.novita.ai/v1")
        );
        assert_eq!(
            novita
                .setup
                .as_ref()
                .map(|setup| setup.can_list_models()),
            Some(true)
        );

        let aliased = registry
            .find("novita_ai")
            .expect("novita alias should resolve");
        assert_eq!(aliased.id, "novita");
    }

    #[test]
    fn test_find_case_insensitive() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        assert!(registry.find("OpenAI").is_some());
        assert!(registry.find("GROQ").is_some());
        assert!(registry.find("Tinfoil").is_some());
    }

    #[test]
    fn test_find_unknown_returns_none() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        assert!(registry.find("nonexistent_provider").is_none());
    }

    #[test]
    fn test_selectable_has_setup_hints() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        let selectable = registry.selectable();
        assert!(!selectable.is_empty());
        for def in &selectable {
            assert!(
                def.setup.is_some(),
                "selectable provider {} must have setup hint",
                def.id
            );
        }
    }

    #[test]
    fn test_user_override_wins() {
        let builtins: Vec<ProviderDefinition> =
            serde_json::from_str(include_str!("../../providers.json")).unwrap();
        let mut all = builtins;
        // Simulate user overriding tinfoil with a different default model
        all.push(ProviderDefinition {
            id: "tinfoil".to_string(),
            aliases: vec![],
            protocol: ProviderProtocol::OpenAiCompletions,
            default_base_url: Some("https://custom.tinfoil.example/v1".to_string()),
            base_url_env: None,
            base_url_required: false,
            api_key_env: Some("TINFOIL_API_KEY".to_string()),
            api_key_required: true,
            model_env: "TINFOIL_MODEL".to_string(),
            default_model: "custom-model".to_string(),
            description: "Custom tinfoil".to_string(),
            extra_headers_env: None,
            setup: None,
            unsupported_params: vec![],
        });
        let registry = ProviderRegistry::new(all);
        let tf = registry.find("tinfoil").expect("tinfoil should exist");
        assert_eq!(tf.default_model, "custom-model", "user override should win");
    }

    #[test]
    fn test_model_env_var_nearai() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        assert_eq!(registry.model_env_var("nearai"), "NEARAI_MODEL");
        assert_eq!(registry.model_env_var("near_ai"), "NEARAI_MODEL");
    }

    #[test]
    fn test_model_env_var_registry_provider() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        assert_eq!(registry.model_env_var("groq"), "GROQ_MODEL");
        assert_eq!(registry.model_env_var("tinfoil"), "TINFOIL_MODEL");
        assert_eq!(registry.model_env_var("openai"), "OPENAI_MODEL");
    }

    #[test]
    fn test_model_env_var_unknown_fallback() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        assert_eq!(registry.model_env_var("nonexistent"), "LLM_MODEL");
    }

    #[test]
    fn test_is_known() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        assert!(registry.is_known("nearai"));
        assert!(registry.is_known("openai"));
        assert!(registry.is_known("groq"));
        assert!(!registry.is_known("nonexistent"));
    }

    #[test]
    fn test_all_providers_have_required_fields() {
        let providers: Vec<ProviderDefinition> =
            serde_json::from_str(include_str!("../../providers.json")).unwrap();
        for def in &providers {
            assert!(!def.id.is_empty(), "provider must have an id");
            assert!(!def.model_env.is_empty(), "{}: model_env required", def.id);
            assert!(
                !def.default_model.is_empty(),
                "{}: default_model required",
                def.id
            );
            assert!(
                !def.description.is_empty(),
                "{}: description required",
                def.id
            );
        }
    }

    #[test]
    fn test_openai_compatible_providers_have_base_url() {
        let providers: Vec<ProviderDefinition> =
            serde_json::from_str(include_str!("../../providers.json")).unwrap();
        for def in &providers {
            if def.protocol == ProviderProtocol::OpenAiCompletions
                && def.id != "openai"
                && def.id != "openai_compatible"
                && def.id != "bedrock"
                && def.id != "cloudflare"
            {
                assert!(
                    def.default_base_url.is_some(),
                    "{}: OpenAI-completions provider should have a default_base_url",
                    def.id
                );
            }
        }
    }

    #[test]
    fn test_models_filter_accessor() {
        let registry = ProviderRegistry::new(
            serde_json::from_str(include_str!("../../providers.json")).unwrap(),
        );
        // Groq has models_filter: "chat"
        let groq = registry.find("groq").expect("groq should exist");
        let filter = groq
            .setup
            .as_ref()
            .and_then(|s| s.models_filter())
            .expect("groq should have models_filter");
        assert_eq!(filter, "chat");

        // OpenAI has no models_filter
        let openai = registry.find("openai").expect("openai should exist");
        assert!(
            openai
                .setup
                .as_ref()
                .and_then(|s| s.models_filter())
                .is_none(),
            "openai should not have models_filter"
        );

        // Ollama setup hint variant should return None
        let ollama = registry.find("ollama").expect("ollama should exist");
        assert!(
            ollama
                .setup
                .as_ref()
                .and_then(|s| s.models_filter())
                .is_none(),
            "ollama should not have models_filter"
        );
    }

    #[test]
    fn test_selectable_user_override_adds_setup() {
        // A built-in provider without setup hint should NOT appear in selectable().
        // But if a user override adds a setup hint, it SHOULD appear.
        let mut providers: Vec<ProviderDefinition> = vec![ProviderDefinition {
            id: "custom".to_string(),
            aliases: vec![],
            protocol: ProviderProtocol::OpenAiCompletions,
            default_base_url: Some("http://localhost/v1".to_string()),
            base_url_env: None,
            base_url_required: false,
            api_key_env: None,
            api_key_required: false,
            model_env: "CUSTOM_MODEL".to_string(),
            default_model: "m1".to_string(),
            description: "No setup".to_string(),
            extra_headers_env: None,
            setup: None, // no setup hint
            unsupported_params: vec![],
        }];

        let registry = ProviderRegistry::new(providers.clone());
        assert!(
            registry.selectable().is_empty(),
            "provider without setup should not be selectable"
        );

        // User override adds a setup hint
        providers.push(ProviderDefinition {
            id: "custom".to_string(),
            aliases: vec![],
            protocol: ProviderProtocol::OpenAiCompletions,
            default_base_url: Some("http://localhost/v1".to_string()),
            base_url_env: None,
            base_url_required: false,
            api_key_env: Some("CUSTOM_API_KEY".to_string()),
            api_key_required: true,
            model_env: "CUSTOM_MODEL".to_string(),
            default_model: "m1".to_string(),
            description: "Now with setup".to_string(),
            extra_headers_env: None,
            setup: Some(SetupHint::ApiKey {
                secret_name: "llm_custom_api_key".to_string(),
                key_url: None,
                display_name: "Custom".to_string(),
                can_list_models: false,
                models_filter: None,
            }),
            unsupported_params: vec![],
        });

        let registry = ProviderRegistry::new(providers);
        let selectable = registry.selectable();
        assert_eq!(
            selectable.len(),
            1,
            "user override with setup should appear"
        );
        assert_eq!(selectable[0].id, "custom");
        assert_eq!(
            selectable[0].description, "Now with setup",
            "should use the overridden definition"
        );
    }

    #[test]
    fn test_selectable_user_override_removes_setup() {
        // If a built-in has setup but user override removes it, it should
        // NOT appear in selectable().
        let providers = vec![
            ProviderDefinition {
                id: "provider_a".to_string(),
                aliases: vec![],
                protocol: ProviderProtocol::OpenAiCompletions,
                default_base_url: Some("http://a/v1".to_string()),
                base_url_env: None,
                base_url_required: false,
                api_key_env: Some("A_KEY".to_string()),
                api_key_required: true,
                model_env: "A_MODEL".to_string(),
                default_model: "m1".to_string(),
                description: "Has setup".to_string(),
                extra_headers_env: None,
                setup: Some(SetupHint::ApiKey {
                    secret_name: "a".to_string(),
                    key_url: None,
                    display_name: "A".to_string(),
                    can_list_models: false,
                    models_filter: None,
                }),
                unsupported_params: vec![],
            },
            // User override removes setup
            ProviderDefinition {
                id: "provider_a".to_string(),
                aliases: vec![],
                protocol: ProviderProtocol::OpenAiCompletions,
                default_base_url: Some("http://a/v1".to_string()),
                base_url_env: None,
                base_url_required: false,
                api_key_env: Some("A_KEY".to_string()),
                api_key_required: false,
                model_env: "A_MODEL".to_string(),
                default_model: "m1".to_string(),
                description: "No setup now".to_string(),
                extra_headers_env: None,
                setup: None,
                unsupported_params: vec![],
            },
        ];

        let registry = ProviderRegistry::new(providers);
        assert!(
            registry.selectable().is_empty(),
            "user override removing setup should exclude from selectable"
        );
        // But find() should still work (uses the override)
        let def = registry
            .find("provider_a")
            .expect("should still be findable");
        assert_eq!(def.description, "No setup now");
    }

    #[test]
    fn test_selectable_preserves_order_with_dedup() {
        // If providers A, B, C are defined, and a user override for B comes
        // later, selectable() should return A, B, C (not A, C, B).
        let providers = vec![
            ProviderDefinition {
                id: "aaa".to_string(),
                aliases: vec![],
                protocol: ProviderProtocol::OpenAiCompletions,
                default_base_url: Some("http://a/v1".to_string()),
                base_url_env: None,
                base_url_required: false,
                api_key_env: None,
                api_key_required: false,
                model_env: "A".to_string(),
                default_model: "m".to_string(),
                description: "A".to_string(),
                extra_headers_env: None,
                setup: Some(SetupHint::Ollama {
                    display_name: "A".to_string(),
                    can_list_models: false,
                }),
                unsupported_params: vec![],
            },
            ProviderDefinition {
                id: "bbb".to_string(),
                aliases: vec![],
                protocol: ProviderProtocol::OpenAiCompletions,
                default_base_url: Some("http://b/v1".to_string()),
                base_url_env: None,
                base_url_required: false,
                api_key_env: None,
                api_key_required: false,
                model_env: "B".to_string(),
                default_model: "m".to_string(),
                description: "B-original".to_string(),
                extra_headers_env: None,
                setup: Some(SetupHint::Ollama {
                    display_name: "B".to_string(),
                    can_list_models: false,
                }),
                unsupported_params: vec![],
            },
            ProviderDefinition {
                id: "ccc".to_string(),
                aliases: vec![],
                protocol: ProviderProtocol::OpenAiCompletions,
                default_base_url: Some("http://c/v1".to_string()),
                base_url_env: None,
                base_url_required: false,
                api_key_env: None,
                api_key_required: false,
                model_env: "C".to_string(),
                default_model: "m".to_string(),
                description: "C".to_string(),
                extra_headers_env: None,
                setup: Some(SetupHint::Ollama {
                    display_name: "C".to_string(),
                    can_list_models: false,
                }),
                unsupported_params: vec![],
            },
            // User override for B
            ProviderDefinition {
                id: "bbb".to_string(),
                aliases: vec![],
                protocol: ProviderProtocol::OpenAiCompletions,
                default_base_url: Some("http://b-new/v1".to_string()),
                base_url_env: None,
                base_url_required: false,
                api_key_env: None,
                api_key_required: false,
                model_env: "B".to_string(),
                default_model: "m".to_string(),
                description: "B-override".to_string(),
                extra_headers_env: None,
                setup: Some(SetupHint::Ollama {
                    display_name: "B".to_string(),
                    can_list_models: false,
                }),
                unsupported_params: vec![],
            },
        ];

        let registry = ProviderRegistry::new(providers);
        let selectable = registry.selectable();
        let ids: Vec<&str> = selectable.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["aaa", "bbb", "ccc"], "order should be preserved");
        assert_eq!(
            selectable[1].description, "B-override",
            "should use the overridden definition"
        );
    }

    #[test]
    fn test_unsupported_params_deserialized() {
        let providers: Vec<ProviderDefinition> =
            serde_json::from_str(include_str!("../../providers.json")).unwrap();

        // Tinfoil should have temperature in unsupported_params
        let tinfoil = providers.iter().find(|p| p.id == "tinfoil").unwrap();
        assert!(
            tinfoil
                .unsupported_params
                .contains(&"temperature".to_string()),
            "tinfoil should have 'temperature' in unsupported_params"
        );

        // OpenAI should also have temperature in unsupported_params
        let openai = providers.iter().find(|p| p.id == "openai").unwrap();
        assert!(
            openai
                .unsupported_params
                .contains(&"temperature".to_string()),
            "openai should have 'temperature' in unsupported_params"
        );

        // Providers without the field in JSON should deserialize to empty vec
        let groq = providers.iter().find(|p| p.id == "groq").unwrap();
        assert!(
            groq.unsupported_params.is_empty(),
            "groq should have empty unsupported_params (field absent in JSON)"
        );

        // All entries should only contain valid param names
        // (Invalid names should be rejected at deserialization time)
        for def in &providers {
            for param in &def.unsupported_params {
                assert!(
                    !param.is_empty(),
                    "{}: unsupported_params contains empty string",
                    def.id
                );
                assert!(
                    matches!(
                        param.as_str(),
                        "temperature" | "max_tokens" | "stop_sequences"
                    ),
                    "{}: unsupported_params contains invalid parameter '{}'",
                    def.id,
                    param
                );
            }
        }
    }

    #[test]
    fn test_unsupported_params_validation_rejects_invalid() {
        // Invalid parameter names should cause deserialization error
        let invalid_json = r#"[{
            "id": "test",
            "protocol": "open_ai_completions",
            "model_env": "TEST_MODEL",
            "default_model": "test-model",
            "description": "Test provider",
            "unsupported_params": ["temperrature"]
        }]"#;

        let result: Result<Vec<ProviderDefinition>, _> = serde_json::from_str(invalid_json);
        assert!(
            result.is_err(),
            "should reject invalid parameter name 'temperrature'"
        );
        assert!(
            result.err().unwrap().to_string().contains("temperrature"),
            "error message should mention the invalid parameter"
        );
    }

    #[test]
    fn test_all_builtin_api_key_providers_have_api_key_env() {
        // Every built-in provider with SetupHint::ApiKey must have api_key_env
        // set, otherwise inject_llm_keys_from_secrets can't map the secret.
        let providers: Vec<ProviderDefinition> =
            serde_json::from_str(include_str!("../../providers.json")).unwrap();
        for def in &providers {
            if let Some(SetupHint::ApiKey { .. }) = &def.setup {
                assert!(
                    def.api_key_env.is_some(),
                    "{}: ApiKey setup hint requires api_key_env to be set",
                    def.id
                );
            }
        }
    }
}

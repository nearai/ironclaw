use std::path::PathBuf;
use std::sync::Once;

use secrecy::SecretString;

use crate::bootstrap::ironclaw_base_dir;
use crate::config::helpers::{
    optional_env, parse_optional_env, validate_base_url, validate_operator_base_url,
};
use crate::error::ConfigError;
use crate::llm::config::*;
use crate::llm::registry::{ProviderProtocol, ProviderRegistry};
use crate::llm::session::SessionConfig;
use crate::settings::Settings;

static LOG_LLM_BACKEND_RESOLUTION: Once = Once::new();

impl LlmConfig {
    fn selected_model_override(settings: &Settings) -> Option<String> {
        crate::llm::normalized_model_override(settings.selected_model.as_deref())
            .map(str::to_string)
    }

    /// Create a test-friendly config without reading env vars.
    #[cfg(feature = "libsql")]
    pub fn for_testing() -> Self {
        Self {
            backend: "nearai".to_string(),
            session: SessionConfig {
                auth_base_url: "http://localhost:0".to_string(),
                session_path: std::env::temp_dir().join("ironclaw-test-session.json"),
            },
            nearai: NearAiConfig {
                model: "test-model".to_string(),
                cheap_model: None,
                base_url: "http://localhost:0".to_string(),
                api_key: None,
                fallback_model: None,
                max_retries: 0,
                circuit_breaker_threshold: None,
                circuit_breaker_recovery_secs: 30,
                response_cache_enabled: false,
                response_cache_ttl_secs: 3600,
                response_cache_max_entries: 100,
                failover_cooldown_secs: 300,
                failover_cooldown_threshold: 3,
                smart_routing_cascade: false,
            },
            provider: None,
            bedrock: None,
            gemini_oauth: None,
            openai_codex: None,
            request_timeout_secs: 120,
            cheap_model: None,
            smart_routing_cascade: false,
        }
    }

    /// Resolve a model name from env var -> settings.selected_model -> hardcoded default.
    ///
    /// Env vars take priority over DB settings so cloud deployments can
    /// override via platform Variables UI (12-factor app behavior).
    fn resolve_model(
        env_var: &str,
        settings: &Settings,
        default: &str,
    ) -> Result<String, ConfigError> {
        if let Some(model) = optional_env(env_var)? {
            Ok(model)
        } else if let Some(model) = settings.selected_model.clone() {
            Ok(model)
        } else {
            Ok(default.to_string())
        }
    }

    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let registry = ProviderRegistry::load();

        // Determine backend: env var > db settings > default ("nearai")
        //
        // Env vars take priority over DB settings for LLM config so that
        // cloud deployments (Railway, Render, Fly.io) can override the
        // onboarding-persisted backend via their Variables UI — standard
        // 12-factor app behavior.
        let (backend, backend_source) = if let Some(b) = optional_env("LLM_BACKEND")? {
            (b, "env:LLM_BACKEND")
        } else if let Some(ref b) = settings.llm_backend {
            (b.clone(), "db:llm_backend")
        } else {
            ("nearai".to_string(), "default")
        };
        LOG_LLM_BACKEND_RESOLUTION.call_once(|| {
            tracing::debug!(
                backend = %backend,
                source = %backend_source,
                db_llm_backend = ?settings.llm_backend,
                custom_providers_count = settings.llm_custom_providers.len(),
                "Resolving LLM backend"
            );
        });
        // Inform operators when an env var overrides a DB-persisted value.
        if backend_source == "env:LLM_BACKEND"
            && settings
                .llm_backend
                .as_ref()
                .is_some_and(|db_val| db_val != &backend)
        {
            tracing::info!(
                env_value = %backend,
                db_value = ?settings.llm_backend,
                "LLM_BACKEND env var overrides DB setting"
            );
        }

        // Validate the backend is known
        let backend_lower = backend.to_lowercase();
        let is_nearai =
            backend_lower == "nearai" || backend_lower == "near_ai" || backend_lower == "near";
        let is_bedrock =
            backend_lower == "bedrock" || backend_lower == "aws_bedrock" || backend_lower == "aws";
        let is_gemini_oauth = backend_lower == "gemini_oauth" || backend_lower == "gemini-oauth";
        let is_openai_codex = backend_lower == "openai_codex"
            || backend_lower == "openai-codex"
            || backend_lower == "codex";

        // Check custom providers defined
        let custom_provider = settings
            .llm_custom_providers
            .iter()
            .find(|p| p.id.to_lowercase() == backend_lower);

        if !is_nearai
            && !is_bedrock
            && !is_gemini_oauth
            && !is_openai_codex
            && custom_provider.is_none()
            && registry.find(&backend_lower).is_none()
        {
            tracing::warn!(
                "Unknown LLM backend '{}'. Will attempt as openai_compatible fallback.",
                backend
            );
        }

        // Session config (used by NearAI provider for OAuth/session-token auth)
        let nearai_auth_url = optional_env("NEARAI_AUTH_URL")?
            .unwrap_or_else(|| "https://private.near.ai".to_string());
        validate_base_url(&nearai_auth_url, "NEARAI_AUTH_URL")?;
        let session = SessionConfig {
            auth_base_url: nearai_auth_url,
            session_path: optional_env("NEARAI_SESSION_PATH")?
                .map(PathBuf::from)
                .unwrap_or_else(default_session_path),
        };

        // Always resolve NEAR AI config (used for embeddings even when not the primary backend)
        // Priority: env > DB (builtin_overrides) > default
        let nearai_override = settings.llm_builtin_overrides.get("nearai");
        let nearai_api_key = optional_env("NEARAI_API_KEY")?
            .map(SecretString::from)
            .or_else(|| {
                nearai_override
                    .and_then(|o| o.api_key.as_ref())
                    .map(|key| SecretString::from(key.clone()))
            });
        // Model priority: env > selected_model (DB) > builtin_overrides (DB) > default
        let nearai_model = if let Some(model) = optional_env("NEARAI_MODEL")? {
            model
        } else if let Some(model) = Self::selected_model_override(settings) {
            model
        } else if let Some(model) = nearai_override.and_then(|o| o.model.clone()) {
            model
        } else {
            crate::llm::DEFAULT_MODEL.to_string()
        };
        let nearai_base_url = if let Some(url) = optional_env("NEARAI_BASE_URL")? {
            url
        } else if let Some(url) = nearai_override.and_then(|o| o.base_url.clone()) {
            url
        } else if nearai_api_key.is_some() {
            "https://cloud-api.near.ai".to_string()
        } else {
            "https://private.near.ai".to_string()
        };
        validate_base_url(&nearai_base_url, "NEARAI_BASE_URL")?;
        let nearai = NearAiConfig {
            model: nearai_model,
            cheap_model: optional_env("NEARAI_CHEAP_MODEL")?,
            base_url: nearai_base_url,
            api_key: nearai_api_key,
            fallback_model: optional_env("NEARAI_FALLBACK_MODEL")?,
            max_retries: parse_optional_env("NEARAI_MAX_RETRIES", 3)?,
            circuit_breaker_threshold: optional_env("CIRCUIT_BREAKER_THRESHOLD")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "CIRCUIT_BREAKER_THRESHOLD".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?,
            circuit_breaker_recovery_secs: parse_optional_env("CIRCUIT_BREAKER_RECOVERY_SECS", 30)?,
            response_cache_enabled: parse_optional_env("RESPONSE_CACHE_ENABLED", false)?,
            response_cache_ttl_secs: parse_optional_env("RESPONSE_CACHE_TTL_SECS", 3600)?,
            response_cache_max_entries: parse_optional_env("RESPONSE_CACHE_MAX_ENTRIES", 1000)?,
            failover_cooldown_secs: parse_optional_env("LLM_FAILOVER_COOLDOWN_SECS", 300)?,
            failover_cooldown_threshold: parse_optional_env("LLM_FAILOVER_THRESHOLD", 3)?,
            smart_routing_cascade: parse_optional_env("SMART_ROUTING_CASCADE", true)?,
        };

        // Resolve registry provider config (for non-NearAI, non-Bedrock, non-Gemini, non-Codex backends)
        let provider = if is_nearai || is_bedrock || is_gemini_oauth || is_openai_codex {
            None
        } else if let Some(custom) = custom_provider {
            Some(Self::resolve_custom_provider(custom, settings)?)
        } else {
            Some(Self::resolve_registry_provider(
                &backend_lower,
                &registry,
                settings,
            )?)
        };

        let bedrock = if is_bedrock {
            let explicit_region =
                optional_env("BEDROCK_REGION")?.or_else(|| settings.bedrock_region.clone());
            if explicit_region.is_none() {
                tracing::info!("BEDROCK_REGION not set, defaulting to us-east-1");
            }
            let region = explicit_region.unwrap_or_else(|| "us-east-1".to_string());
            let model = optional_env("BEDROCK_MODEL")?
                .or(Self::selected_model_override(settings))
                .ok_or_else(|| ConfigError::MissingRequired {
                    key: "BEDROCK_MODEL".to_string(),
                    hint: "Set BEDROCK_MODEL or selected_model when LLM_BACKEND=bedrock"
                        .to_string(),
                })?;
            let cross_region = optional_env("BEDROCK_CROSS_REGION")?
                .or_else(|| settings.bedrock_cross_region.clone());
            if let Some(ref cr) = cross_region
                && !matches!(cr.as_str(), "us" | "eu" | "apac" | "global")
            {
                return Err(ConfigError::InvalidValue {
                    key: "BEDROCK_CROSS_REGION".to_string(),
                    message: format!(
                        "'{}' is not valid, expected one of: us, eu, apac, global",
                        cr
                    ),
                });
            }
            let profile = optional_env("AWS_PROFILE")?.or_else(|| settings.bedrock_profile.clone());
            Some(BedrockConfig {
                region,
                model,
                cross_region,
                profile,
            })
        } else {
            None
        };

        // Resolve OpenAI Codex config
        let openai_codex = if is_openai_codex {
            // Model: OPENAI_CODEX_MODEL > OPENAI_MODEL > settings.selected_model > default
            let model = optional_env("OPENAI_CODEX_MODEL")?
                .or(optional_env("OPENAI_MODEL")?)
                .or_else(|| settings.selected_model.clone())
                .unwrap_or_else(|| "gpt-5.3-codex".to_string());
            let auth_endpoint = optional_env("OPENAI_CODEX_AUTH_URL")?
                .unwrap_or_else(|| "https://auth.openai.com".to_string());
            validate_base_url(&auth_endpoint, "OPENAI_CODEX_AUTH_URL")?;
            let api_base_url = optional_env("OPENAI_CODEX_API_URL")?
                .unwrap_or_else(|| "https://chatgpt.com/backend-api/codex".to_string());
            validate_base_url(&api_base_url, "OPENAI_CODEX_API_URL")?;
            let client_id = optional_env("OPENAI_CODEX_CLIENT_ID")?
                .unwrap_or_else(|| "app_EMoamEEZ73f0CkXaXp7hrann".to_string());
            let session_path = optional_env("OPENAI_CODEX_SESSION_PATH")?
                .map(PathBuf::from)
                .unwrap_or_else(|| ironclaw_base_dir().join("openai_codex_session.json"));
            let token_refresh_margin_secs =
                parse_optional_env("OPENAI_CODEX_REFRESH_MARGIN_SECS", 300)?;
            Some(OpenAiCodexConfig {
                model,
                auth_endpoint,
                api_base_url,
                client_id,
                session_path,
                token_refresh_margin_secs,
            })
        } else {
            None
        };

        let request_timeout_secs = parse_optional_env("LLM_REQUEST_TIMEOUT_SECS", 120)?;

        let gemini_oauth = if backend_lower == "gemini_oauth" || backend_lower == "gemini-oauth" {
            let model = Self::resolve_model("GEMINI_MODEL", settings, "gemini-2.5-flash")?;
            let credentials_path = optional_env("GEMINI_CREDENTIALS_PATH")?
                .map(PathBuf::from)
                .unwrap_or_else(GeminiOauthConfig::default_credentials_path);
            Some(GeminiOauthConfig {
                model,
                credentials_path,
            })
        } else {
            None
        };

        // Generic cheap model (works with any backend).
        // Falls back to NearAI-specific cheap_model in provider chain logic.
        let cheap_model = optional_env("LLM_CHEAP_MODEL")?;

        // Generic smart routing cascade flag.
        // Defaults to true. Overrides NearAI-specific smart_routing_cascade.
        let smart_routing_cascade = parse_optional_env("SMART_ROUTING_CASCADE", true)?;

        Ok(Self {
            backend: if is_nearai {
                "nearai".to_string()
            } else if is_bedrock {
                "bedrock".to_string()
            } else if is_gemini_oauth {
                "gemini_oauth".to_string()
            } else if is_openai_codex {
                "openai_codex".to_string()
            } else if let Some(ref p) = provider {
                p.provider_id.clone()
            } else {
                backend_lower
            },
            session,
            nearai,
            provider,
            bedrock,
            gemini_oauth,
            openai_codex,
            request_timeout_secs,
            cheap_model,
            smart_routing_cascade,
        })
    }

    /// Resolve a `RegistryProviderConfig` from a user-defined custom provider.
    fn resolve_custom_provider(
        custom: &crate::settings::CustomLlmProviderSettings,
        settings: &Settings,
    ) -> Result<RegistryProviderConfig, ConfigError> {
        tracing::info!(
            id = %custom.id,
            adapter = %custom.adapter,
            base_url = ?custom.base_url,
            "Resolving custom LLM provider"
        );
        let protocol = match custom.adapter.as_str() {
            "anthropic" => ProviderProtocol::Anthropic,
            "ollama" => ProviderProtocol::Ollama,
            _ => ProviderProtocol::OpenAiCompletions,
        };

        let api_key = custom
            .api_key
            .as_ref()
            .filter(|k| !k.is_empty())
            .map(|k| SecretString::from(k.clone()));

        let base_url = custom.base_url.clone().unwrap_or_default();
        if base_url.is_empty() {
            tracing::warn!(id = %custom.id, "Custom provider has no base_url configured — requests will fail");
        } else {
            validate_operator_base_url(
                &base_url,
                &format!("custom provider '{}' base_url", custom.id),
            )?;
        }

        let model = optional_env("LLM_MODEL")?
            .or(Self::selected_model_override(settings))
            .or_else(|| custom.default_model.clone())
            .unwrap_or_default();
        if model.is_empty() {
            tracing::warn!(id = %custom.id, "Custom provider has no model configured — requests may fail");
        }

        Ok(RegistryProviderConfig {
            protocol,
            provider_id: custom.id.clone(),
            api_key,
            base_url,
            model,
            extra_headers: Vec::new(),
            oauth_token: None,
            is_codex_chatgpt: false,
            refresh_token: None,
            auth_path: None,
            cache_retention: CacheRetention::default(),
            unsupported_params: Vec::new(),
        })
    }

    /// Resolve a `RegistryProviderConfig` from the registry and env vars.
    fn resolve_registry_provider(
        backend: &str,
        registry: &ProviderRegistry,
        settings: &Settings,
    ) -> Result<RegistryProviderConfig, ConfigError> {
        // Look up provider definition. Fall back to openai_compatible if unknown.
        let def = registry
            .find(backend)
            .or_else(|| registry.find("openai_compatible"));

        let (
            canonical_id,
            protocol,
            api_key_env,
            base_url_env,
            model_env,
            default_model,
            default_base_url,
            extra_headers_env,
            api_key_required,
            base_url_required,
            unsupported_params,
        ) = if let Some(def) = def {
            (
                def.id.as_str(),
                def.protocol,
                def.api_key_env.as_deref(),
                def.base_url_env.as_deref(),
                def.model_env.as_str(),
                def.default_model.as_str(),
                def.default_base_url.as_deref(),
                def.extra_headers_env.as_deref(),
                def.api_key_required,
                def.base_url_required,
                def.unsupported_params.clone(),
            )
        } else {
            // Absolute fallback: treat as generic openai_completions
            (
                backend,
                ProviderProtocol::OpenAiCompletions,
                Some("LLM_API_KEY"),
                Some("LLM_BASE_URL"),
                "LLM_MODEL",
                "default",
                None,
                Some("LLM_EXTRA_HEADERS"),
                false,
                true,
                Vec::new(),
            )
        };

        // Codex auth.json override: when LLM_USE_CODEX_AUTH=true,
        // credentials from the Codex CLI's auth.json take highest priority
        // (over env vars AND secrets store). In ChatGPT mode, the base URL
        // is also overridden to the private ChatGPT backend endpoint.
        let mut codex_base_url_override: Option<String> = None;
        let codex_creds = if parse_optional_env("LLM_USE_CODEX_AUTH", false)? {
            let path = optional_env("CODEX_AUTH_PATH")?
                .map(std::path::PathBuf::from)
                .unwrap_or_else(crate::llm::codex_auth::default_codex_auth_path);
            crate::llm::codex_auth::load_codex_credentials(&path)
        } else {
            None
        };

        let codex_refresh_token = codex_creds.as_ref().and_then(|c| c.refresh_token.clone());
        let codex_auth_path = codex_creds.as_ref().and_then(|c| c.auth_path.clone());

        let api_key = if let Some(creds) = codex_creds {
            if creds.is_chatgpt_mode {
                codex_base_url_override = Some(creds.base_url().to_string());
            }
            Some(creds.token)
        } else if let Some(env_var) = api_key_env {
            // Resolve API key: env var > settings override (DB)
            // Env vars take priority so cloud deployments can override via
            // platform Variables UI (12-factor app behavior).
            optional_env(env_var)?.map(SecretString::from).or_else(|| {
                settings
                    .llm_builtin_overrides
                    .get(backend)
                    .and_then(|o| o.api_key.as_ref())
                    .map(|key| SecretString::from(key.clone()))
            })
        } else {
            None
        };

        if api_key_required && api_key.is_none() {
            // Don't hard-fail here. The key might be injected later from the secrets store
            // via inject_llm_keys_from_secrets(). Log a warning instead.
            if let Some(env_var) = api_key_env {
                tracing::debug!(
                    "API key not found in {env_var} for backend '{backend}'. \
                     Will be injected from secrets store if available."
                );
            }
        }

        // Resolve base URL: codex override > env var > builtin_overrides (DB) > legacy settings (DB) > registry default
        let is_codex_chatgpt = codex_base_url_override.is_some();
        let env_base_url = if let Some(env_var) = base_url_env {
            optional_env(env_var)?
        } else {
            None
        };
        let base_url = codex_base_url_override
            .or(env_base_url)
            .or_else(|| {
                // DB settings: per-provider base_url override
                settings
                    .llm_builtin_overrides
                    .get(backend)
                    .and_then(|o| o.base_url.clone())
            })
            .or_else(|| {
                // DB settings: legacy settings fields
                match backend {
                    "ollama" => settings.ollama_base_url.clone(),
                    "openai_compatible" | "openrouter" => {
                        settings.openai_compatible_base_url.clone()
                    }
                    _ => None,
                }
            })
            .or_else(|| default_base_url.map(String::from))
            .unwrap_or_default();

        if base_url_required
            && base_url.is_empty()
            && let Some(env_var) = base_url_env
        {
            return Err(ConfigError::MissingRequired {
                key: env_var.to_string(),
                hint: format!("Set {env_var} when LLM_BACKEND={backend}"),
            });
        }

        // Provider base URLs are explicit operator configuration, so allow
        // private/local endpoints while still rejecting unsafe schemes,
        // public plaintext HTTP, and special blocked addresses.
        if !base_url.is_empty() {
            let field = base_url_env.unwrap_or("LLM_BASE_URL");
            validate_operator_base_url(&base_url, field)?;
        }

        // Resolve model: env var > selected_model (DB) > per-provider override (DB) > registry default
        let model = optional_env(model_env)?
            .or(Self::selected_model_override(settings))
            .or_else(|| {
                settings
                    .llm_builtin_overrides
                    .get(backend)
                    .and_then(|o| o.model.clone())
            })
            .unwrap_or_else(|| default_model.to_string());

        // Resolve extra headers
        let extra_headers = if let Some(env_var) = extra_headers_env {
            optional_env(env_var)?
                .map(|val| parse_extra_headers_with_key(&val, env_var))
                .transpose()?
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let extra_headers = if canonical_id == "github_copilot" {
            merge_extra_headers(
                crate::llm::github_copilot_auth::default_headers(),
                extra_headers,
            )
        } else {
            extra_headers
        };

        // Resolve OAuth token (Anthropic-specific: `claude login` flow).
        // Only check for OAuth token when the provider is actually Anthropic.
        let oauth_token = if canonical_id == "anthropic" {
            optional_env("ANTHROPIC_OAUTH_TOKEN")?.map(SecretString::from)
        } else {
            None
        };
        let api_key = if api_key.is_none() && oauth_token.is_some() {
            // OAuth token present but no API key: use a placeholder so the
            // config block is populated. The provider factory will route to
            // the OAuth provider instead of rig-core's x-api-key client.
            Some(SecretString::from(OAUTH_PLACEHOLDER.to_string()))
        } else {
            api_key
        };

        // Resolve Anthropic prompt cache retention from env (default: Short).
        let cache_retention: CacheRetention = if canonical_id == "anthropic" {
            optional_env("ANTHROPIC_CACHE_RETENTION")?
                .and_then(|val| match val.parse::<CacheRetention>() {
                    Ok(r) => Some(r),
                    Err(e) => {
                        tracing::warn!(
                            "Invalid ANTHROPIC_CACHE_RETENTION: {e}; defaulting to short"
                        );
                        None
                    }
                })
                .unwrap_or_default()
        } else {
            CacheRetention::default()
        };

        Ok(RegistryProviderConfig {
            protocol,
            provider_id: canonical_id.to_string(),
            api_key,
            base_url,
            model,
            extra_headers,
            oauth_token,
            is_codex_chatgpt,
            refresh_token: codex_refresh_token,
            auth_path: codex_auth_path,
            cache_retention,
            unsupported_params,
        })
    }
}

/// Parse `LLM_EXTRA_HEADERS` value into a list of (key, value) pairs.
///
/// Format: `Key1:Value1,Key2:Value2` (colon-separated, not `=`, because
/// header values often contain `=`).
fn parse_extra_headers_with_key(
    val: &str,
    env_var_name: &str,
) -> Result<Vec<(String, String)>, ConfigError> {
    if val.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut headers = Vec::new();
    for pair in val.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let Some((key, value)) = pair.split_once(':') else {
            return Err(ConfigError::InvalidValue {
                key: env_var_name.to_string(),
                message: format!("malformed header entry '{}', expected Key:Value", pair),
            });
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: env_var_name.to_string(),
                message: format!("empty header name in entry '{}'", pair),
            });
        }
        headers.push((key.to_string(), value.trim().to_string()));
    }
    Ok(headers)
}

fn merge_extra_headers(
    defaults: Vec<(String, String)>,
    overrides: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let mut merged = Vec::new();
    let mut positions = std::collections::HashMap::<String, usize>::new();

    for (key, value) in defaults.into_iter().chain(overrides) {
        let normalized = key.to_ascii_lowercase();
        if let Some(existing_index) = positions.get(&normalized).copied() {
            merged[existing_index] = (key, value);
        } else {
            positions.insert(normalized, merged.len());
            merged.push((key, value));
        }
    }

    merged
}

/// Get the default session file path (~/.ironclaw/session.json).
pub fn default_session_path() -> PathBuf {
    ironclaw_base_dir().join("session.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;
    use crate::settings::Settings;
    use crate::testing::credentials::*;

    /// Convenience wrapper for tests — uses "TEST_HEADERS" as the env var name.
    fn parse_extra_headers(val: &str) -> Result<Vec<(String, String)>, ConfigError> {
        parse_extra_headers_with_key(val, "TEST_HEADERS")
    }

    /// Clear all openai-compatible-related env vars.
    fn clear_openai_compatible_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("LLM_BASE_URL");
            std::env::remove_var("LLM_MODEL");
        }
    }

    #[test]
    fn openai_compatible_uses_selected_model_when_llm_model_unset() {
        let _guard = lock_env();
        clear_openai_compatible_env();

        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("https://openrouter.ai/api/v1".to_string()),
            selected_model: Some("openai/gpt-5.1-codex".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");

        assert_eq!(provider.model, "openai/gpt-5.1-codex");
    }

    #[test]
    fn openai_compatible_env_model_overrides_db() {
        let _guard = lock_env();
        clear_openai_compatible_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("LLM_MODEL", "openai/gpt-5-codex");
        }

        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("https://openrouter.ai/api/v1".to_string()),
            selected_model: Some("openai/gpt-5.1-codex".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");

        assert_eq!(
            provider.model, "openai/gpt-5-codex",
            "LLM_MODEL env var should take priority over DB selected_model (12-factor)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_MODEL");
        }
    }

    #[test]
    fn test_extra_headers_parsed() {
        let result = parse_extra_headers("HTTP-Referer:https://myapp.com,X-Title:MyApp").unwrap();
        assert_eq!(
            result,
            vec![
                ("HTTP-Referer".to_string(), "https://myapp.com".to_string()),
                ("X-Title".to_string(), "MyApp".to_string()),
            ]
        );
    }

    #[test]
    fn test_extra_headers_empty_string() {
        let result = parse_extra_headers("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_extra_headers_whitespace_only() {
        let result = parse_extra_headers("  ").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_extra_headers_malformed() {
        let result = parse_extra_headers("NoColonHere");
        assert!(result.is_err());
    }

    #[test]
    fn test_extra_headers_empty_key() {
        let result = parse_extra_headers(":value");
        assert!(result.is_err());
    }

    #[test]
    fn test_extra_headers_value_with_colons() {
        let result = parse_extra_headers("Authorization:Bearer abc:def").unwrap();
        assert_eq!(
            result,
            vec![("Authorization".to_string(), "Bearer abc:def".to_string())]
        );
    }

    #[test]
    fn test_extra_headers_trailing_comma() {
        let result = parse_extra_headers("X-Title:MyApp,").unwrap();
        assert_eq!(result, vec![("X-Title".to_string(), "MyApp".to_string())]);
    }

    #[test]
    fn test_extra_headers_with_spaces() {
        let result =
            parse_extra_headers(" HTTP-Referer : https://myapp.com , X-Title : MyApp ").unwrap();
        assert_eq!(
            result,
            vec![
                ("HTTP-Referer".to_string(), "https://myapp.com".to_string()),
                ("X-Title".to_string(), "MyApp".to_string()),
            ]
        );
    }

    #[test]
    fn merge_extra_headers_prefers_overrides_case_insensitively() {
        let merged = merge_extra_headers(
            vec![
                ("User-Agent".to_string(), "default-agent".to_string()),
                ("X-Test".to_string(), "default".to_string()),
            ],
            vec![
                ("user-agent".to_string(), "override-agent".to_string()),
                ("X-Extra".to_string(), "present".to_string()),
            ],
        );

        assert_eq!(
            merged,
            vec![
                ("user-agent".to_string(), "override-agent".to_string()),
                ("X-Test".to_string(), "default".to_string()),
                ("X-Extra".to_string(), "present".to_string()),
            ]
        );
    }

    /// Clear all ollama-related env vars.
    fn clear_ollama_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("OLLAMA_BASE_URL");
            std::env::remove_var("OLLAMA_MODEL");
        }
    }

    #[test]
    fn ollama_uses_selected_model_when_ollama_model_unset() {
        let _guard = lock_env();
        clear_ollama_env();

        let settings = Settings {
            llm_backend: Some("ollama".to_string()),
            selected_model: Some("llama3.2".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");

        assert_eq!(provider.model, "llama3.2");
    }

    #[test]
    fn ollama_env_model_overrides_db() {
        let _guard = lock_env();
        clear_ollama_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("OLLAMA_MODEL", "mistral:latest");
        }

        let settings = Settings {
            llm_backend: Some("ollama".to_string()),
            selected_model: Some("llama3.2".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");

        assert_eq!(
            provider.model, "mistral:latest",
            "OLLAMA_MODEL env var should take priority over DB selected_model (12-factor)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("OLLAMA_MODEL");
        }
    }

    #[test]
    fn openai_compatible_preserves_dotted_model_name() {
        let _guard = lock_env();
        clear_openai_compatible_env();

        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("http://localhost:11434/v1".to_string()),
            selected_model: Some("llama3.2".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");

        assert_eq!(
            provider.model, "llama3.2",
            "model name with dot must not be truncated"
        );
    }

    #[test]
    fn openai_compatible_allows_https_localhost_base_url() {
        let _guard = lock_env();
        clear_openai_compatible_env();

        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("https://localhost:8443/v1".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");

        assert_eq!(provider.base_url, "https://localhost:8443/v1");
    }

    #[test]
    fn openai_compatible_allows_http_private_network_base_url() {
        let _guard = lock_env();
        clear_openai_compatible_env();

        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("http://100.64.0.10:8000/v1".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");

        assert_eq!(provider.base_url, "http://100.64.0.10:8000/v1");
    }

    #[test]
    fn registry_provider_resolves_groq() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("GROQ_API_KEY");
            std::env::remove_var("GROQ_MODEL");
        }

        let settings = Settings {
            llm_backend: Some("groq".to_string()),
            selected_model: Some("llama-3.3-70b-versatile".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(cfg.backend, "groq");
        let provider = cfg.provider.expect("provider config should be present");
        assert_eq!(provider.provider_id, "groq");
        assert_eq!(provider.model, "llama-3.3-70b-versatile");
        assert_eq!(provider.base_url, "https://api.groq.com/openai/v1");
        assert_eq!(provider.protocol, ProviderProtocol::OpenAiCompletions);
    }

    #[test]
    fn registry_provider_resolves_tinfoil() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("TINFOIL_API_KEY");
            std::env::remove_var("TINFOIL_MODEL");
        }

        let settings = Settings {
            llm_backend: Some("tinfoil".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(cfg.backend, "tinfoil");
        let provider = cfg.provider.expect("provider config should be present");
        assert_eq!(provider.base_url, "https://inference.tinfoil.sh/v1");
        assert_eq!(provider.model, "kimi-k2-5");
        assert!(
            provider
                .unsupported_params
                .contains(&"temperature".to_string()),
            "tinfoil should propagate unsupported_params from registry"
        );
    }

    #[test]
    fn registry_provider_alias_resolves_zai() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("ZAI_API_KEY");
            std::env::remove_var("ZAI_MODEL");
        }

        let settings = Settings {
            llm_backend: Some("bigmodel".to_string()),
            selected_model: Some("glm-5".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(cfg.backend, "zai");
        let provider = cfg.provider.expect("provider config should be present");
        assert_eq!(provider.provider_id, "zai");
        assert_eq!(provider.model, "glm-5");
        assert_eq!(provider.base_url, "https://api.z.ai/api/paas/v4");
        assert_eq!(provider.protocol, ProviderProtocol::OpenAiCompletions);
    }

    #[test]
    fn registry_provider_resolves_github_copilot_alias() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("LLM_BACKEND", "github-copilot");
            std::env::set_var("GITHUB_COPILOT_TOKEN", "gho_test_token");
            std::env::set_var(
                "GITHUB_COPILOT_EXTRA_HEADERS",
                "Copilot-Integration-Id:custom-chat,X-Test:enabled",
            );
        }

        let settings = Settings::default();

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(cfg.backend, "github_copilot");
        let provider = cfg.provider.expect("provider config should be present");
        assert_eq!(provider.provider_id, "github_copilot");
        assert_eq!(provider.base_url, "https://api.githubcopilot.com");
        assert_eq!(provider.model, "gpt-4o");
        assert!(
            provider
                .extra_headers
                .iter()
                .any(|(key, value)| { key == "Copilot-Integration-Id" && value == "custom-chat" })
        );
        assert!(
            provider
                .extra_headers
                .iter()
                .any(|(key, value)| key == "User-Agent" && value == "GitHubCopilotChat/0.26.7")
        );
        assert!(
            provider
                .extra_headers
                .iter()
                .any(|(key, value)| key == "X-Test" && value == "enabled")
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("GITHUB_COPILOT_TOKEN");
            std::env::remove_var("GITHUB_COPILOT_EXTRA_HEADERS");
        }
    }

    #[test]
    fn nearai_backend_has_no_registry_provider() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
        }

        let settings = Settings::default();
        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(cfg.backend, "nearai");
        assert!(cfg.provider.is_none());
    }

    #[test]
    fn backend_alias_normalized_to_canonical_id() {
        let _guard = lock_env();
        clear_openai_compatible_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("LLM_BACKEND", "open_ai");
            std::env::set_var("OPENAI_API_KEY", TEST_API_KEY);
        }

        let settings = Settings::default();
        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(
            cfg.backend, "openai",
            "alias 'open_ai' should be normalized to canonical 'openai'"
        );
        let provider = cfg.provider.expect("should have provider config");
        assert_eq!(provider.provider_id, "openai");

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("OPENAI_API_KEY");
        }
    }

    #[test]
    fn unknown_backend_falls_back_to_openai_compatible() {
        let _guard = lock_env();
        clear_openai_compatible_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("LLM_BACKEND", "some_custom_provider");
            std::env::set_var("LLM_BASE_URL", "http://localhost:8080/v1");
        }

        let settings = Settings::default();
        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(cfg.backend, "openai_compatible");
        let provider = cfg.provider.expect("should have provider config");
        assert_eq!(provider.provider_id, "openai_compatible");
        assert_eq!(provider.base_url, "http://localhost:8080/v1");

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("LLM_BASE_URL");
        }
    }

    #[test]
    fn nearai_aliases_all_resolve_to_nearai() {
        let _guard = lock_env();

        for alias in &["nearai", "near_ai", "near"] {
            // SAFETY: Under ENV_MUTEX.
            unsafe {
                std::env::set_var("LLM_BACKEND", alias);
            }
            let settings = Settings::default();
            let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
            assert_eq!(
                cfg.backend, "nearai",
                "alias '{alias}' should resolve to 'nearai'"
            );
            assert!(
                cfg.provider.is_none(),
                "nearai should not have a registry provider"
            );
        }

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
        }
    }

    #[test]
    fn base_url_resolution_priority() {
        let _guard = lock_env();
        clear_openai_compatible_env();

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("LLM_BACKEND", "openai_compatible");
            std::env::set_var("LLM_BASE_URL", "http://localhost:8000/v1");
        }

        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("http://localhost:9000/v1".to_string()),
            ..Default::default()
        };

        // Env var should take priority over DB settings (12-factor)
        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("should have provider config");
        assert_eq!(
            provider.base_url, "http://localhost:8000/v1",
            "env var should take priority over DB settings (12-factor)"
        );

        // Without env var, DB settings should win over registry default
        unsafe {
            std::env::remove_var("LLM_BASE_URL");
        }

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("should have provider config");
        assert_eq!(
            provider.base_url, "http://localhost:9000/v1",
            "DB settings should take priority over registry default when env var is not set"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
        }
    }

    // ── OAuth resolution tests ──────────────────────────────────────

    /// Clear all Anthropic-related env vars.
    fn clear_anthropic_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("ANTHROPIC_API_KEY");
            std::env::remove_var("ANTHROPIC_OAUTH_TOKEN");
            std::env::remove_var("ANTHROPIC_MODEL");
            std::env::remove_var("ANTHROPIC_BASE_URL");
        }
    }

    #[test]
    fn anthropic_oauth_token_sets_placeholder_api_key() {
        use secrecy::ExposeSecret;

        let _guard = lock_env();
        clear_anthropic_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("ANTHROPIC_OAUTH_TOKEN", TEST_ANTHROPIC_OAUTH_TOKEN);
        }

        let settings = Settings {
            llm_backend: Some("anthropic".to_string()),
            ..Default::default()
        };
        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");

        assert_eq!(
            provider
                .api_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some(OAUTH_PLACEHOLDER.to_string()),
            "api_key should be the OAuth placeholder when only OAuth token is set"
        );
        assert!(
            provider.oauth_token.is_some(),
            "oauth_token should be populated"
        );
        assert_eq!(
            provider.oauth_token.as_ref().unwrap().expose_secret(),
            TEST_ANTHROPIC_OAUTH_TOKEN
        );

        clear_anthropic_env();
    }

    #[test]
    fn anthropic_api_key_takes_priority_over_oauth() {
        use secrecy::ExposeSecret;

        let _guard = lock_env();
        clear_anthropic_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", TEST_ANTHROPIC_API_KEY);
            std::env::set_var("ANTHROPIC_OAUTH_TOKEN", TEST_ANTHROPIC_OAUTH_TOKEN);
        }

        let settings = Settings {
            llm_backend: Some("anthropic".to_string()),
            ..Default::default()
        };
        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");

        assert_eq!(
            provider
                .api_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some(TEST_ANTHROPIC_API_KEY.to_string()),
            "real API key should take priority over OAuth placeholder"
        );
        assert!(
            provider.oauth_token.is_some(),
            "oauth_token should still be populated"
        );

        clear_anthropic_env();
    }

    #[test]
    fn non_anthropic_provider_has_no_oauth_token() {
        let _guard = lock_env();
        clear_anthropic_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("ANTHROPIC_OAUTH_TOKEN", TEST_ANTHROPIC_OAUTH_TOKEN);
        }

        let settings = Settings {
            llm_backend: Some("openai".to_string()),
            ..Default::default()
        };
        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");

        assert!(
            provider.oauth_token.is_none(),
            "non-Anthropic providers should not pick up ANTHROPIC_OAUTH_TOKEN"
        );

        clear_anthropic_env();
    }

    // ── Cache retention tests ───────────────────────────────────────

    #[test]
    fn cache_retention_from_str_primary_values() {
        assert_eq!(
            "none".parse::<CacheRetention>().unwrap(),
            CacheRetention::None
        );
        assert_eq!(
            "short".parse::<CacheRetention>().unwrap(),
            CacheRetention::Short
        );
        assert_eq!(
            "long".parse::<CacheRetention>().unwrap(),
            CacheRetention::Long
        );
    }

    #[test]
    fn cache_retention_from_str_aliases() {
        assert_eq!(
            "off".parse::<CacheRetention>().unwrap(),
            CacheRetention::None
        );
        assert_eq!(
            "disabled".parse::<CacheRetention>().unwrap(),
            CacheRetention::None
        );
        assert_eq!(
            "5m".parse::<CacheRetention>().unwrap(),
            CacheRetention::Short
        );
        assert_eq!(
            "ephemeral".parse::<CacheRetention>().unwrap(),
            CacheRetention::Short
        );
        assert_eq!(
            "1h".parse::<CacheRetention>().unwrap(),
            CacheRetention::Long
        );
    }

    #[test]
    fn cache_retention_from_str_case_insensitive() {
        assert_eq!(
            "NONE".parse::<CacheRetention>().unwrap(),
            CacheRetention::None
        );
        assert_eq!(
            "Short".parse::<CacheRetention>().unwrap(),
            CacheRetention::Short
        );
        assert_eq!(
            "LONG".parse::<CacheRetention>().unwrap(),
            CacheRetention::Long
        );
        assert_eq!(
            "Ephemeral".parse::<CacheRetention>().unwrap(),
            CacheRetention::Short
        );
    }

    #[test]
    fn cache_retention_from_str_invalid() {
        let err = "bogus".parse::<CacheRetention>().unwrap_err();
        assert!(
            err.contains("bogus"),
            "error should mention the invalid value"
        );
    }

    #[test]
    fn cache_retention_display_round_trip() {
        for variant in [
            CacheRetention::None,
            CacheRetention::Short,
            CacheRetention::Long,
        ] {
            let s = variant.to_string();
            let parsed: CacheRetention = s.parse().unwrap();
            assert_eq!(parsed, variant, "round-trip failed for {s}");
        }
    }

    #[test]
    fn test_request_timeout_defaults_to_120() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_REQUEST_TIMEOUT_SECS");
        }
        let config = LlmConfig::resolve(&Settings::default()).expect("resolve");
        assert_eq!(config.request_timeout_secs, 120);
    }

    #[test]
    fn test_request_timeout_configurable() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("LLM_REQUEST_TIMEOUT_SECS", "300");
        }
        let config = LlmConfig::resolve(&Settings::default()).expect("resolve");
        assert_eq!(config.request_timeout_secs, 300);
        // SAFETY: Cleanup
        unsafe {
            std::env::remove_var("LLM_REQUEST_TIMEOUT_SECS");
        }
    }

    // ── Custom provider tests ───────────────────────────────────────

    #[test]
    fn custom_provider_resolves_when_backend_matches_id() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("LLM_MODEL");
        }

        let settings = Settings {
            llm_backend: Some("myprovider".to_string()),
            llm_custom_providers: vec![crate::settings::CustomLlmProviderSettings {
                id: "myprovider".to_string(),
                name: "My Provider".to_string(),
                adapter: "open_ai_completions".to_string(),
                base_url: Some("http://localhost:9090/v1".to_string()),
                default_model: Some("my-model".to_string()),
                api_key: Some("sk-test".to_string()),
                builtin: false,
            }],
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(cfg.backend, "myprovider");
        let provider = cfg.provider.expect("provider config should be present");
        assert_eq!(provider.provider_id, "myprovider");
        assert_eq!(provider.base_url, "http://localhost:9090/v1");
        assert_eq!(provider.model, "my-model");
        assert_eq!(
            provider.protocol,
            crate::llm::registry::ProviderProtocol::OpenAiCompletions
        );
    }

    #[test]
    fn env_llm_backend_takes_priority_over_db() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX. RAII guard removes LLM_BACKEND on drop so
        // a panicking assertion cannot leak the env var to other tests.
        struct RemoveOnDrop(&'static str);
        impl Drop for RemoveOnDrop {
            fn drop(&mut self) {
                unsafe { std::env::remove_var(self.0) };
            }
        }
        let _cleanup = RemoveOnDrop("LLM_BACKEND");
        unsafe {
            std::env::set_var("LLM_BACKEND", "nearai");
            std::env::remove_var("LLM_MODEL");
        }

        let settings = Settings {
            llm_backend: Some("myprovider".to_string()),
            llm_custom_providers: vec![crate::settings::CustomLlmProviderSettings {
                id: "myprovider".to_string(),
                name: "My Provider".to_string(),
                adapter: "open_ai_completions".to_string(),
                base_url: Some("http://localhost:9090/v1".to_string()),
                default_model: Some("my-model".to_string()),
                api_key: None,
                builtin: false,
            }],
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(
            cfg.backend, "nearai",
            "LLM_BACKEND env var should override DB setting (12-factor)"
        );
    }

    // ── OpenAI Codex tests ──────────────────────────────────────────

    /// Clear all openai-codex-related env vars.
    fn clear_openai_codex_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("OPENAI_CODEX_MODEL");
            std::env::remove_var("OPENAI_MODEL");
        }
    }

    #[test]
    fn builtin_override_model_used_when_no_selected_model() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("GROQ_MODEL");
        }

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "groq".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: None,
                model: Some("llama-3.1-8b-instant".to_string()),
                base_url: None,
            },
        );
        let settings = Settings {
            llm_backend: Some("groq".to_string()),
            llm_builtin_overrides: overrides,
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");
        assert_eq!(
            provider.model, "llama-3.1-8b-instant",
            "builtin override model should be used when selected_model is unset"
        );
    }

    #[test]
    fn openai_codex_resolves_config() {
        let _guard = lock_env();
        clear_openai_codex_env();

        let settings = Settings {
            llm_backend: Some("openai_codex".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(cfg.backend, "openai_codex");
        let codex = cfg.openai_codex.expect("codex config should be present");
        assert_eq!(codex.model, "gpt-5.3-codex"); // default
        assert!(
            cfg.provider.is_none(),
            "codex should not use registry provider"
        );
    }

    #[test]
    fn selected_model_takes_priority_over_builtin_override_model() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("GROQ_MODEL");
        }

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "groq".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: None,
                model: Some("llama-3.1-8b-instant".to_string()),
                base_url: None,
            },
        );
        let settings = Settings {
            llm_backend: Some("groq".to_string()),
            selected_model: Some("llama-3.3-70b-versatile".to_string()),
            llm_builtin_overrides: overrides,
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");
        assert_eq!(
            provider.model, "llama-3.3-70b-versatile",
            "selected_model (/model command) must take priority over builtin override"
        );
    }

    #[test]
    fn openai_codex_model_env_resolution() {
        let _guard = lock_env();
        clear_openai_codex_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("OPENAI_CODEX_MODEL", "o3-pro");
        }

        let settings = Settings {
            llm_backend: Some("openai_codex".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let codex = cfg.openai_codex.expect("codex config should be present");
        assert_eq!(codex.model, "o3-pro");

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("OPENAI_CODEX_MODEL");
        }
    }

    #[test]
    fn builtin_override_api_key_used_when_no_env_var() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("GROQ_API_KEY");
            std::env::remove_var("GROQ_MODEL");
        }

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "groq".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: Some("gsk_test_key".to_string()),
                model: Some("llama-3.3-70b-versatile".to_string()),
                base_url: None,
            },
        );
        let settings = Settings {
            llm_backend: Some("groq".to_string()),
            llm_builtin_overrides: overrides,
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");
        use secrecy::ExposeSecret as _;
        let key = provider
            .api_key
            .expect("api_key should be set from builtin override");
        assert_eq!(
            key.expose_secret(),
            "gsk_test_key",
            "builtin override api_key should be used when env var is absent"
        );
    }

    #[test]
    fn openai_codex_falls_back_to_openai_model() {
        let _guard = lock_env();
        clear_openai_codex_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("OPENAI_MODEL", "gpt-4o");
        }

        let settings = Settings {
            llm_backend: Some("openai_codex".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let codex = cfg.openai_codex.expect("codex config should be present");
        assert_eq!(codex.model, "gpt-4o");

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("OPENAI_MODEL");
        }
    }

    #[test]
    fn openai_codex_falls_back_to_selected_model() {
        let _guard = lock_env();
        clear_openai_codex_env();

        let settings = Settings {
            llm_backend: Some("openai_codex".to_string()),
            selected_model: Some("gpt-4o-mini".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let codex = cfg.openai_codex.expect("codex config should be present");
        assert_eq!(codex.model, "gpt-4o-mini");
    }

    /// Regression: SSRF validation on OPENAI_CODEX_API_URL (#1103).
    #[test]
    fn openai_codex_rejects_ssrf_api_url() {
        let _guard = lock_env();
        clear_openai_codex_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var(
                "OPENAI_CODEX_API_URL",
                "http://169.254.169.254/latest/meta-data",
            );
        }

        let settings = Settings {
            llm_backend: Some("openai_codex".to_string()),
            ..Default::default()
        };

        let err = LlmConfig::resolve(&settings).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("OPENAI_CODEX_API_URL"),
            "error should reference the field name: {msg}"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("OPENAI_CODEX_API_URL");
        }
    }

    /// Regression: SSRF validation on OPENAI_CODEX_AUTH_URL (#1103).
    #[test]
    fn openai_codex_rejects_ssrf_auth_url() {
        let _guard = lock_env();
        clear_openai_codex_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("OPENAI_CODEX_AUTH_URL", "http://10.0.0.1");
        }

        let settings = Settings {
            llm_backend: Some("openai_codex".to_string()),
            ..Default::default()
        };

        let err = LlmConfig::resolve(&settings).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("OPENAI_CODEX_AUTH_URL"),
            "error should reference the field name: {msg}"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("OPENAI_CODEX_AUTH_URL");
        }
    }

    // ── DB > ENV priority tests ─────────────────────────────────────

    #[test]
    fn env_api_key_wins_over_builtin_override() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::set_var("GROQ_API_KEY", "gsk_from_env");
            std::env::remove_var("GROQ_MODEL");
        }

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "groq".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: Some("gsk_from_db".to_string()),
                model: None,
                base_url: None,
            },
        );
        let settings = Settings {
            llm_backend: Some("groq".to_string()),
            llm_builtin_overrides: overrides,
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");
        use secrecy::ExposeSecret as _;
        assert_eq!(
            provider
                .api_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some("gsk_from_env".to_string()),
            "GROQ_API_KEY env var must take priority over DB builtin_override (12-factor)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("GROQ_API_KEY");
        }
    }

    #[test]
    fn env_model_wins_over_builtin_override() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::set_var("GROQ_MODEL", "model-from-env");
        }

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "groq".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: None,
                model: Some("model-from-db".to_string()),
                base_url: None,
            },
        );
        let settings = Settings {
            llm_backend: Some("groq".to_string()),
            llm_builtin_overrides: overrides,
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");
        assert_eq!(
            provider.model, "model-from-env",
            "GROQ_MODEL env var must take priority over DB builtin_override (12-factor)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("GROQ_MODEL");
        }
    }

    #[test]
    fn env_model_wins_over_custom_provider_db() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::set_var("LLM_MODEL", "model-from-env");
        }

        let settings = Settings {
            llm_backend: Some("myprovider".to_string()),
            selected_model: Some("model-from-db".to_string()),
            llm_custom_providers: vec![crate::settings::CustomLlmProviderSettings {
                id: "myprovider".to_string(),
                name: "My Provider".to_string(),
                adapter: "open_ai_completions".to_string(),
                base_url: Some("http://localhost:9090/v1".to_string()),
                default_model: Some("default-model".to_string()),
                api_key: None,
                builtin: false,
            }],
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");
        assert_eq!(
            provider.model, "model-from-env",
            "LLM_MODEL env var must take priority over DB selected_model (12-factor)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_MODEL");
        }
    }

    #[test]
    fn env_model_wins_over_openai_codex_db() {
        let _guard = lock_env();
        clear_openai_codex_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("OPENAI_CODEX_MODEL", "codex-from-env");
        }

        let settings = Settings {
            llm_backend: Some("openai_codex".to_string()),
            selected_model: Some("codex-from-db".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let codex = cfg.openai_codex.expect("codex config should be present");
        assert_eq!(
            codex.model, "codex-from-env",
            "OPENAI_CODEX_MODEL env var must take priority over DB selected_model (12-factor)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("OPENAI_CODEX_MODEL");
        }
    }

    #[test]
    fn env_nearai_model_wins_over_db() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::set_var("NEARAI_MODEL", "nearai-from-env");
        }

        let settings = Settings {
            llm_backend: Some("nearai".to_string()),
            selected_model: Some("nearai-from-db".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(
            cfg.nearai.model, "nearai-from-env",
            "NEARAI_MODEL env var must take priority over DB selected_model (12-factor)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("NEARAI_MODEL");
        }
    }

    #[test]
    fn env_nearai_model_wins_over_override() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::set_var("NEARAI_MODEL", "model-from-env");
        }

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "nearai".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: None,
                model: Some("model-from-db-override".to_string()),
                base_url: None,
            },
        );
        let settings = Settings {
            llm_backend: Some("nearai".to_string()),
            llm_builtin_overrides: overrides,
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(
            cfg.nearai.model, "model-from-env",
            "NEARAI_MODEL env var must take priority over DB builtin_overrides (12-factor)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("NEARAI_MODEL");
        }
    }

    #[test]
    fn nearai_selected_model_wins_over_override_model() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("NEARAI_MODEL");
        }

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "nearai".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: None,
                model: Some("model-from-override".to_string()),
                base_url: None,
            },
        );
        let settings = Settings {
            llm_backend: Some("nearai".to_string()),
            selected_model: Some("model-from-selected".to_string()),
            llm_builtin_overrides: overrides,
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(
            cfg.nearai.model, "model-from-selected",
            "selected_model must take priority over builtin_overrides model"
        );
    }

    #[test]
    fn env_nearai_base_url_wins_over_override() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::set_var("NEARAI_BASE_URL", "http://localhost:9001");
            std::env::remove_var("NEARAI_API_KEY");
        }

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "nearai".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: None,
                model: None,
                base_url: Some("http://localhost:9002".to_string()),
            },
        );
        let settings = Settings {
            llm_backend: Some("nearai".to_string()),
            llm_builtin_overrides: overrides,
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(
            cfg.nearai.base_url, "http://localhost:9001",
            "NEARAI_BASE_URL env var must take priority over DB builtin_overrides (12-factor)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("NEARAI_BASE_URL");
        }
    }

    #[test]
    fn nearai_env_base_url_used_when_no_override() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::set_var("NEARAI_BASE_URL", "http://localhost:9001");
            std::env::remove_var("NEARAI_API_KEY");
        }

        let settings = Settings {
            llm_backend: Some("nearai".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(
            cfg.nearai.base_url, "http://localhost:9001",
            "NEARAI_BASE_URL env var should be used when no DB override exists"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("NEARAI_BASE_URL");
        }
    }

    #[test]
    fn env_nearai_api_key_wins_over_override() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::set_var("NEARAI_API_KEY", "key-from-env");
        }

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "nearai".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: Some("key-from-db".to_string()),
                model: None,
                base_url: None,
            },
        );
        let settings = Settings {
            llm_backend: Some("nearai".to_string()),
            llm_builtin_overrides: overrides,
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        use secrecy::ExposeSecret as _;
        assert_eq!(
            cfg.nearai
                .api_key
                .as_ref()
                .map(|k| k.expose_secret().to_string()),
            Some("key-from-env".to_string()),
            "NEARAI_API_KEY env var must take priority over DB builtin_overrides (12-factor)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("NEARAI_API_KEY");
        }
    }

    #[test]
    fn nearai_base_url_auto_selects_when_no_override_or_env() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("NEARAI_BASE_URL");
            std::env::remove_var("NEARAI_API_KEY");
        }

        // No API key → should default to private.near.ai
        let settings = Settings {
            llm_backend: Some("nearai".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(
            cfg.nearai.base_url, "https://private.near.ai",
            "Without API key, should default to private.near.ai"
        );

        // With API key → should default to cloud-api.near.ai
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "nearai".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: Some("some-key".to_string()),
                model: None,
                base_url: None,
            },
        );
        let settings_with_key = Settings {
            llm_backend: Some("nearai".to_string()),
            llm_builtin_overrides: overrides,
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings_with_key).expect("resolve should succeed");
        assert_eq!(
            cfg.nearai.base_url, "https://cloud-api.near.ai",
            "With API key, should default to cloud-api.near.ai"
        );
    }

    #[test]
    fn registry_provider_override_base_url_wins_over_env() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::set_var("GROQ_BASE_URL", "http://localhost:9003");
            std::env::remove_var("GROQ_API_KEY");
            std::env::remove_var("GROQ_MODEL");
        }

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "groq".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: None,
                model: None,
                base_url: Some("http://localhost:9004".to_string()),
            },
        );
        let settings = Settings {
            llm_backend: Some("groq".to_string()),
            llm_builtin_overrides: overrides,
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let provider = cfg.provider.expect("provider config should be present");
        assert_eq!(
            provider.base_url, "http://localhost:9004",
            "DB builtin_overrides base_url must take priority over GROQ_BASE_URL env var"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("GROQ_BASE_URL");
        }
    }

    #[test]
    fn selected_model_override_ignores_default_sentinel() {
        let settings = Settings {
            selected_model: Some(" default ".to_string()),
            ..Default::default()
        };

        assert_eq!(LlmConfig::selected_model_override(&settings), None);
    }

    #[test]
    fn nearai_resolve_ignores_default_selected_model() {
        let _guard = lock_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::set_var("NEARAI_MODEL", "env-model");
        }

        let settings = Settings {
            llm_backend: Some("nearai".to_string()),
            selected_model: Some("default".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(cfg.nearai.model, "env-model");

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("NEARAI_MODEL");
        }
    }
}

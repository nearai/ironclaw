use std::path::PathBuf;

use secrecy::SecretString;

use crate::config::helpers::{optional_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Which LLM backend to use.
///
/// Defaults to `NearAi` to keep IronClaw close to the NEAR ecosystem.
/// Users can override with `LLM_BACKEND` env var to use their own API keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LlmBackend {
    /// NEAR AI proxy (default) -- session or API key auth
    #[default]
    NearAi,
    /// Direct OpenAI API
    OpenAi,
    /// Direct Anthropic API
    Anthropic,
    /// Local Ollama instance
    Ollama,
    /// Any OpenAI-compatible endpoint (e.g. vLLM, LiteLLM, Together)
    OpenAiCompatible,
    /// Tinfoil private inference
    Tinfoil,
    /// OpenAI Codex via Responses API (ChatGPT OAuth or API key)
    OpenAiCodex,
}

impl std::str::FromStr for LlmBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "nearai" | "near_ai" | "near" => Ok(Self::NearAi),
            "openai" | "open_ai" => Ok(Self::OpenAi),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            "openai_compatible" | "openai-compatible" | "compatible" => Ok(Self::OpenAiCompatible),
            "tinfoil" => Ok(Self::Tinfoil),
            "openai_codex" | "codex" => Ok(Self::OpenAiCodex),
            _ => Err(format!(
                "invalid LLM backend '{}', expected one of: nearai, openai, anthropic, ollama, openai_compatible, tinfoil, openai_codex",
                s
            )),
        }
    }
}

impl std::fmt::Display for LlmBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NearAi => write!(f, "nearai"),
            Self::OpenAi => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Ollama => write!(f, "ollama"),
            Self::OpenAiCompatible => write!(f, "openai_compatible"),
            Self::Tinfoil => write!(f, "tinfoil"),
            Self::OpenAiCodex => write!(f, "openai_codex"),
        }
    }
}

/// Configuration for direct OpenAI API access.
#[derive(Debug, Clone)]
pub struct OpenAiDirectConfig {
    pub api_key: SecretString,
    pub model: String,
    /// Optional base URL override (e.g. for proxies like VibeProxy).
    pub base_url: Option<String>,
}

/// Configuration for direct Anthropic API access.
#[derive(Debug, Clone)]
pub struct AnthropicDirectConfig {
    pub api_key: SecretString,
    pub model: String,
    /// Optional base URL override (e.g. for proxies like VibeProxy).
    pub base_url: Option<String>,
}

/// Configuration for local Ollama.
#[derive(Debug, Clone)]
pub struct OllamaConfig {
    pub base_url: String,
    pub model: String,
}

/// Configuration for any OpenAI-compatible endpoint.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleConfig {
    pub base_url: String,
    pub api_key: Option<SecretString>,
    pub model: String,
    /// Extra HTTP headers injected into every LLM request.
    /// Parsed from `LLM_EXTRA_HEADERS` env var (format: `Key:Value,Key2:Value2`).
    pub extra_headers: Vec<(String, String)>,
}

/// Configuration for Tinfoil private inference.
#[derive(Debug, Clone)]
pub struct TinfoilConfig {
    pub api_key: SecretString,
    pub model: String,
}

/// Configuration for OpenAI Codex via Responses API.
///
/// Supports two auth modes:
/// - **API key**: Standard OpenAI billing via `api.openai.com/v1/responses`
/// - **OAuth**: ChatGPT subscription billing via `chatgpt.com/backend-api/codex/responses`,
///   using tokens from the Codex CLI (`~/.codex/auth.json`)
#[derive(Debug, Clone)]
pub struct OpenAiCodexConfig {
    /// Model name (default: "gpt-5.3-codex").
    pub model: String,
    /// Base URL. Defaults based on auth mode:
    /// - API key: `https://api.openai.com/v1`
    /// - OAuth: `https://chatgpt.com/backend-api/codex`
    pub base_url: String,
    /// API key for api.openai.com (standard billing).
    pub api_key: Option<SecretString>,
    /// Path to Codex CLI auth.json for OAuth tokens.
    /// Default: `~/.codex/auth.json` (or `$CODEX_HOME/auth.json`).
    pub auth_path: PathBuf,
    /// OpenAI account ID (required for ChatGPT endpoint).
    pub account_id: Option<String>,
}

/// LLM provider configuration.
///
/// NEAR AI remains the default backend. Users can switch to other providers
/// by setting `LLM_BACKEND` (e.g. `openai`, `anthropic`, `ollama`).
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// Which backend to use (default: NearAi)
    pub backend: LlmBackend,
    /// NEAR AI config (always populated for NEAR AI embeddings, etc.)
    pub nearai: NearAiConfig,
    /// Direct OpenAI config (populated when backend=openai)
    pub openai: Option<OpenAiDirectConfig>,
    /// Direct Anthropic config (populated when backend=anthropic)
    pub anthropic: Option<AnthropicDirectConfig>,
    /// Ollama config (populated when backend=ollama)
    pub ollama: Option<OllamaConfig>,
    /// OpenAI-compatible config (populated when backend=openai_compatible)
    pub openai_compatible: Option<OpenAiCompatibleConfig>,
    /// Tinfoil config (populated when backend=tinfoil)
    pub tinfoil: Option<TinfoilConfig>,
    /// OpenAI Codex config (populated when backend=openai_codex)
    pub openai_codex: Option<OpenAiCodexConfig>,
}

/// NEAR AI configuration.
#[derive(Debug, Clone)]
pub struct NearAiConfig {
    /// Model to use (e.g., "claude-3-5-sonnet-20241022", "gpt-4o")
    pub model: String,
    /// Cheap/fast model for lightweight tasks (heartbeat, routing, evaluation).
    /// Falls back to the main model if not set.
    pub cheap_model: Option<String>,
    /// Base URL for the NEAR AI API.
    /// Default: `https://private.near.ai` (session token) or `https://cloud-api.near.ai` (API key)
    pub base_url: String,
    /// Base URL for auth/refresh endpoints (default: https://private.near.ai)
    pub auth_base_url: String,
    /// Path to session file (default: ~/.ironclaw/session.json)
    pub session_path: PathBuf,
    /// API key for NEAR AI Cloud. When set, uses API key auth; otherwise uses session token auth.
    pub api_key: Option<SecretString>,
    /// Optional fallback model for failover (default: None).
    /// When set, a secondary provider is created with this model and wrapped
    /// in a `FailoverProvider` so transient errors on the primary model
    /// automatically fall through to the fallback.
    pub fallback_model: Option<String>,
    /// Maximum number of retries for transient errors (default: 3).
    /// With the default of 3, the provider makes up to 4 total attempts
    /// (1 initial + 3 retries) before giving up.
    pub max_retries: u32,
    /// Consecutive transient failures before the circuit breaker opens.
    /// None = disabled (default). E.g. 5 means after 5 consecutive failures
    /// all requests are rejected until recovery timeout elapses.
    pub circuit_breaker_threshold: Option<u32>,
    /// How long (seconds) the circuit stays open before allowing a probe (default: 30).
    pub circuit_breaker_recovery_secs: u64,
    /// Enable in-memory response caching for `complete()` calls.
    /// Saves tokens on repeated prompts within a session. Default: false.
    pub response_cache_enabled: bool,
    /// TTL in seconds for cached responses (default: 3600 = 1 hour).
    pub response_cache_ttl_secs: u64,
    /// Max cached responses before LRU eviction (default: 1000).
    pub response_cache_max_entries: usize,
    /// Cooldown duration in seconds for the failover provider (default: 300).
    /// When a provider accumulates enough consecutive failures it is skipped
    /// for this many seconds.
    pub failover_cooldown_secs: u64,
    /// Number of consecutive retryable failures before a provider enters
    /// cooldown (default: 3).
    pub failover_cooldown_threshold: u32,
    /// Enable cascade mode for smart routing: when a moderate-complexity task
    /// gets an uncertain response from the cheap model, re-send to primary.
    /// Default: true.
    pub smart_routing_cascade: bool,
}

impl LlmConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        // Determine backend: env var > settings > default (NearAi)
        let backend: LlmBackend = if let Some(b) = optional_env("LLM_BACKEND")? {
            b.parse().map_err(|e| ConfigError::InvalidValue {
                key: "LLM_BACKEND".to_string(),
                message: e,
            })?
        } else if let Some(ref b) = settings.llm_backend {
            match b.parse() {
                Ok(backend) => backend,
                Err(e) => {
                    tracing::warn!(
                        "Invalid llm_backend '{}' in settings: {}. Using default NearAi.",
                        b,
                        e
                    );
                    LlmBackend::NearAi
                }
            }
        } else {
            LlmBackend::NearAi
        };

        // Resolve NEAR AI config only when backend is NearAi (or when explicitly configured)
        let nearai_api_key = optional_env("NEARAI_API_KEY")?.map(SecretString::from);

        let nearai = NearAiConfig {
            model: optional_env("NEARAI_MODEL")?
                .or_else(|| settings.selected_model.clone())
                .unwrap_or_else(|| "zai-org/GLM-latest".to_string()),
            cheap_model: optional_env("NEARAI_CHEAP_MODEL")?,
            base_url: optional_env("NEARAI_BASE_URL")?.unwrap_or_else(|| {
                if nearai_api_key.is_some() {
                    "https://cloud-api.near.ai".to_string()
                } else {
                    "https://private.near.ai".to_string()
                }
            }),
            auth_base_url: optional_env("NEARAI_AUTH_URL")?
                .unwrap_or_else(|| "https://private.near.ai".to_string()),
            session_path: optional_env("NEARAI_SESSION_PATH")?
                .map(PathBuf::from)
                .unwrap_or_else(default_session_path),
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

        // Resolve provider-specific configs based on backend
        let openai = if backend == LlmBackend::OpenAi {
            let api_key = optional_env("OPENAI_API_KEY")?
                .map(SecretString::from)
                .ok_or_else(|| ConfigError::MissingRequired {
                    key: "OPENAI_API_KEY".to_string(),
                    hint: "Set OPENAI_API_KEY when LLM_BACKEND=openai".to_string(),
                })?;
            let model = optional_env("OPENAI_MODEL")?.unwrap_or_else(|| "gpt-4o".to_string());
            let base_url = optional_env("OPENAI_BASE_URL")?;
            Some(OpenAiDirectConfig {
                api_key,
                model,
                base_url,
            })
        } else {
            None
        };

        let anthropic = if backend == LlmBackend::Anthropic {
            let api_key = optional_env("ANTHROPIC_API_KEY")?
                .map(SecretString::from)
                .ok_or_else(|| ConfigError::MissingRequired {
                    key: "ANTHROPIC_API_KEY".to_string(),
                    hint: "Set ANTHROPIC_API_KEY when LLM_BACKEND=anthropic".to_string(),
                })?;
            let model = optional_env("ANTHROPIC_MODEL")?
                .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
            let base_url = optional_env("ANTHROPIC_BASE_URL")?;
            Some(AnthropicDirectConfig {
                api_key,
                model,
                base_url,
            })
        } else {
            None
        };

        let ollama = if backend == LlmBackend::Ollama {
            let base_url = optional_env("OLLAMA_BASE_URL")?
                .or_else(|| settings.ollama_base_url.clone())
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            let model = optional_env("OLLAMA_MODEL")?.unwrap_or_else(|| "llama3".to_string());
            Some(OllamaConfig { base_url, model })
        } else {
            None
        };

        let openai_compatible = if backend == LlmBackend::OpenAiCompatible {
            let base_url = optional_env("LLM_BASE_URL")?
                .or_else(|| settings.openai_compatible_base_url.clone())
                .ok_or_else(|| ConfigError::MissingRequired {
                    key: "LLM_BASE_URL".to_string(),
                    hint: "Set LLM_BASE_URL when LLM_BACKEND=openai_compatible".to_string(),
                })?;
            let api_key = optional_env("LLM_API_KEY")?.map(SecretString::from);
            let model = optional_env("LLM_MODEL")?
                .or_else(|| settings.selected_model.clone())
                .unwrap_or_else(|| "default".to_string());
            let extra_headers = optional_env("LLM_EXTRA_HEADERS")?
                .map(|val| parse_extra_headers(&val))
                .transpose()?
                .unwrap_or_default();
            Some(OpenAiCompatibleConfig {
                base_url,
                api_key,
                model,
                extra_headers,
            })
        } else {
            None
        };

        let tinfoil = if backend == LlmBackend::Tinfoil {
            let api_key = optional_env("TINFOIL_API_KEY")?
                .map(SecretString::from)
                .ok_or_else(|| ConfigError::MissingRequired {
                    key: "TINFOIL_API_KEY".to_string(),
                    hint: "Set TINFOIL_API_KEY when LLM_BACKEND=tinfoil".to_string(),
                })?;
            let model = optional_env("TINFOIL_MODEL")?.unwrap_or_else(|| "kimi-k2-5".to_string());
            Some(TinfoilConfig { api_key, model })
        } else {
            None
        };

        let openai_codex = if backend == LlmBackend::OpenAiCodex {
            let api_key = optional_env("OPENAI_CODEX_API_KEY")?.map(SecretString::from);
            let model =
                optional_env("OPENAI_CODEX_MODEL")?.unwrap_or_else(|| "gpt-5.3-codex".to_string());
            let auth_path = optional_env("CODEX_AUTH_PATH")?
                .map(PathBuf::from)
                .unwrap_or_else(default_codex_auth_path);
            let account_id = optional_env("OPENAI_CODEX_ACCOUNT_ID")?;
            let base_url = optional_env("OPENAI_CODEX_BASE_URL")?.unwrap_or_else(|| {
                if api_key.is_some() {
                    "https://api.openai.com/v1".to_string()
                } else {
                    "https://chatgpt.com/backend-api/codex".to_string()
                }
            });
            Some(OpenAiCodexConfig {
                model,
                base_url,
                api_key,
                auth_path,
                account_id,
            })
        } else {
            None
        };

        Ok(Self {
            backend,
            nearai,
            openai,
            anthropic,
            ollama,
            openai_compatible,
            tinfoil,
            openai_codex,
        })
    }
}

/// Parse `LLM_EXTRA_HEADERS` value into a list of (key, value) pairs.
///
/// Format: `Key1:Value1,Key2:Value2` — colon-separated key:value, comma-separated pairs.
/// Colon is used as the separator (not `=`) because header values often contain `=`
/// (e.g., base64 tokens).
fn parse_extra_headers(val: &str) -> Result<Vec<(String, String)>, ConfigError> {
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
                key: "LLM_EXTRA_HEADERS".to_string(),
                message: format!("malformed header entry '{}', expected Key:Value", pair),
            });
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "LLM_EXTRA_HEADERS".to_string(),
                message: format!("empty header name in entry '{}'", pair),
            });
        }
        headers.push((key.to_string(), value.trim().to_string()));
    }
    Ok(headers)
}

/// Get the default Codex CLI auth.json path.
///
/// Respects `$CODEX_HOME` if set, otherwise defaults to `~/.codex/auth.json`.
fn default_codex_auth_path() -> PathBuf {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        return PathBuf::from(codex_home).join("auth.json");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("auth.json")
}

/// Extract an OAuth access token from a Codex CLI `auth.json` file.
///
/// Tries fields in order: `tokens.access_token`, `token`, `api_key`, `access_token`.
/// Returns `None` on any failure (file not found, parse error, no matching field).
pub fn extract_codex_oauth_token(auth_path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(auth_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Try nested tokens.access_token first (Codex CLI format)
    if let Some(token) = json
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        && !token.is_empty()
    {
        return Some(token.to_string());
    }

    // Try top-level fields
    for field in &["token", "api_key", "access_token"] {
        if let Some(val) = json.get(field).and_then(|v| v.as_str())
            && !val.is_empty()
        {
            return Some(val.to_string());
        }
    }

    None
}

/// Get the default session file path (~/.ironclaw/session.json).
fn default_session_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("session.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::ENV_MUTEX;
    use crate::settings::Settings;

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
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_openai_compatible_env();

        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("https://openrouter.ai/api/v1".to_string()),
            selected_model: Some("openai/gpt-5.1-codex".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let compat = cfg
            .openai_compatible
            .expect("openai-compatible config should be present");

        assert_eq!(compat.model, "openai/gpt-5.1-codex");
    }

    #[test]
    fn openai_compatible_llm_model_env_overrides_selected_model() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
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
        let compat = cfg
            .openai_compatible
            .expect("openai-compatible config should be present");

        assert_eq!(compat.model, "openai/gpt-5-codex");

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
        // Values can contain colons (e.g., URLs)
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

    /// Clear codex-related env vars for testing.
    fn clear_codex_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("OPENAI_CODEX_API_KEY");
            std::env::remove_var("OPENAI_CODEX_MODEL");
            std::env::remove_var("OPENAI_CODEX_BASE_URL");
            std::env::remove_var("OPENAI_CODEX_ACCOUNT_ID");
            std::env::remove_var("CODEX_AUTH_PATH");
        }
    }

    #[test]
    fn codex_defaults_model_and_oauth_base_url() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_codex_env();

        let settings = Settings {
            llm_backend: Some("openai_codex".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let codex = cfg.openai_codex.expect("codex config should be present");

        assert_eq!(codex.model, "gpt-5.3-codex");
        // No API key → OAuth mode → ChatGPT base URL
        assert!(codex.api_key.is_none());
        assert_eq!(codex.base_url, "https://chatgpt.com/backend-api/codex");
        assert!(codex.auth_path.to_string_lossy().contains("auth.json"));
    }

    #[test]
    fn codex_api_key_sets_openai_base_url() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_codex_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("OPENAI_CODEX_API_KEY", "sk-test-key");
        }

        let settings = Settings {
            llm_backend: Some("openai_codex".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let codex = cfg.openai_codex.expect("codex config should be present");

        assert!(codex.api_key.is_some());
        assert_eq!(codex.base_url, "https://api.openai.com/v1");

        // Cleanup
        unsafe {
            std::env::remove_var("OPENAI_CODEX_API_KEY");
        }
    }

    #[test]
    fn codex_env_vars_override_defaults() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_codex_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("OPENAI_CODEX_MODEL", "gpt-5.1-codex");
            std::env::set_var("OPENAI_CODEX_BASE_URL", "https://custom.example.com/v1");
            std::env::set_var("OPENAI_CODEX_ACCOUNT_ID", "acct_123");
            std::env::set_var("CODEX_AUTH_PATH", "/tmp/test-auth.json");
        }

        let settings = Settings {
            llm_backend: Some("openai_codex".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let codex = cfg.openai_codex.expect("codex config should be present");

        assert_eq!(codex.model, "gpt-5.1-codex");
        assert_eq!(codex.base_url, "https://custom.example.com/v1");
        assert_eq!(codex.account_id.as_deref(), Some("acct_123"));
        assert_eq!(
            codex.auth_path,
            std::path::PathBuf::from("/tmp/test-auth.json")
        );

        // Cleanup
        unsafe {
            std::env::remove_var("OPENAI_CODEX_MODEL");
            std::env::remove_var("OPENAI_CODEX_BASE_URL");
            std::env::remove_var("OPENAI_CODEX_ACCOUNT_ID");
            std::env::remove_var("CODEX_AUTH_PATH");
        }
    }

    #[test]
    fn codex_not_populated_for_other_backends() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_codex_env();

        let settings = Settings {
            llm_backend: Some("nearai".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        assert!(cfg.openai_codex.is_none());
    }

    #[test]
    fn test_extract_codex_oauth_token_nested() {
        let dir = std::env::temp_dir().join("ironclaw-test-codex");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("auth-nested.json");
        std::fs::write(
            &path,
            r#"{"tokens":{"access_token":"oauth-tok-123","refresh_token":"rt_456"}}"#,
        )
        .expect("write test file");

        let token = extract_codex_oauth_token(&path);
        assert_eq!(token, Some("oauth-tok-123".to_string()));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_extract_codex_oauth_token_flat() {
        let dir = std::env::temp_dir().join("ironclaw-test-codex");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("auth-flat.json");
        std::fs::write(&path, r#"{"token":"flat-tok-789"}"#).expect("write test file");

        let token = extract_codex_oauth_token(&path);
        assert_eq!(token, Some("flat-tok-789".to_string()));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_extract_codex_oauth_token_missing_file() {
        let path = std::path::Path::new("/tmp/ironclaw-nonexistent-auth.json");
        assert!(extract_codex_oauth_token(path).is_none());
    }

    #[test]
    fn test_extract_codex_oauth_token_empty_fields() {
        let dir = std::env::temp_dir().join("ironclaw-test-codex");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("auth-empty.json");
        std::fs::write(
            &path,
            r#"{"tokens":{"access_token":""},"token":"","api_key":""}"#,
        )
        .expect("write test file");

        let token = extract_codex_oauth_token(&path);
        assert!(token.is_none());

        let _ = std::fs::remove_file(&path);
    }
}

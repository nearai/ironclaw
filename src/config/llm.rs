use std::path::PathBuf;

use secrecy::SecretString;

use crate::bootstrap::ironclaw_base_dir;
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
    /// Venice.ai uncensored inference
    VeniceAi,
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
            "venice" | "venice_ai" | "veniceai" => Ok(Self::VeniceAi),
            _ => Err(format!(
                "invalid LLM backend '{}', expected one of: nearai, openai, anthropic, ollama, openai_compatible, tinfoil",
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
            Self::VeniceAi => write!(f, "veniceai"),
        }
    }
}

impl LlmBackend {
    /// The environment variable that configures the model name for this backend.
    ///
    /// Used by both `LlmConfig::resolve()` (reads the var) and the setup wizard
    /// (writes the var to `.env`). Centralised here so the two stay in sync.
    pub fn model_env_var(&self) -> &'static str {
        match self {
            Self::NearAi => "NEARAI_MODEL",
            Self::OpenAi => "OPENAI_MODEL",
            Self::Anthropic => "ANTHROPIC_MODEL",
            Self::Ollama => "OLLAMA_MODEL",
            Self::OpenAiCompatible => "LLM_MODEL",
            Self::Tinfoil => "TINFOIL_MODEL",
            Self::VeniceAi => "VENICE_MODEL",
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

/// Configuration for Venice.ai uncensored inference.
#[derive(Debug, Clone)]
pub struct VeniceAiConfig {
    pub api_key: SecretString,
    pub model: String,
    /// Optional base URL override (default: https://api.venice.ai/api/v1)
    pub base_url: Option<String>,
    /// Web search mode: "on", "off", or "auto" (default: "off")
    pub web_search: Option<String>,
    /// Enable web scraping for URLs in prompts (default: false)
    pub web_scraping: bool,
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
    /// Venice.ai config (populated when backend=veniceai)
    pub veniceai: Option<VeniceAiConfig>,
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
    /// Resolve a model name from env var → settings.selected_model → hardcoded default.
    fn resolve_model(
        env_var: &str,
        settings: &Settings,
        default: &str,
    ) -> Result<String, ConfigError> {
        Ok(optional_env(env_var)?
            .or_else(|| settings.selected_model.clone())
            .unwrap_or_else(|| default.to_string()))
    }

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
            model: Self::resolve_model("NEARAI_MODEL", settings, "zai-org/GLM-latest")?,
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
            let model = Self::resolve_model("OPENAI_MODEL", settings, "gpt-4o")?;
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
            let model =
                Self::resolve_model("ANTHROPIC_MODEL", settings, "claude-sonnet-4-20250514")?;
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
            let model = Self::resolve_model("OLLAMA_MODEL", settings, "llama3")?;
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
            let model = Self::resolve_model("LLM_MODEL", settings, "default")?;
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
            let model = Self::resolve_model("TINFOIL_MODEL", settings, "kimi-k2-5")?;
            Some(TinfoilConfig { api_key, model })
        } else {
            None
        };

        let veniceai = if backend == LlmBackend::VeniceAi {
            let api_key = optional_env("VENICE_API_KEY")?
                .map(SecretString::from)
                .ok_or_else(|| ConfigError::MissingRequired {
                    key: "VENICE_API_KEY".to_string(),
                    hint: "Set VENICE_API_KEY when LLM_BACKEND=veniceai".to_string(),
                })?;
            let model =
                optional_env("VENICE_MODEL")?.unwrap_or_else(|| "llama-3.3-70b".to_string());
            let base_url = optional_env("VENICE_BASE_URL")?;
            let web_search = optional_env("VENICE_WEB_SEARCH")?;
            let web_scraping = parse_optional_env("VENICE_WEB_SCRAPING", false)?;
            Some(VeniceAiConfig {
                api_key,
                model,
                base_url,
                web_search,
                web_scraping,
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
            veniceai,
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

/// Get the default session file path (~/.ironclaw/session.json).
fn default_session_path() -> PathBuf {
    ironclaw_base_dir().join("session.json")
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
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_ollama_env();

        let settings = Settings {
            llm_backend: Some("ollama".to_string()),
            selected_model: Some("llama3.2".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let ollama = cfg.ollama.expect("ollama config should be present");

        assert_eq!(ollama.model, "llama3.2");
    }

    #[test]
    fn ollama_model_env_overrides_selected_model() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
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
        let ollama = cfg.ollama.expect("ollama config should be present");

        assert_eq!(ollama.model, "mistral:latest");

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("OLLAMA_MODEL");
        }
    }

    #[test]
    fn openai_compatible_preserves_dotted_model_name() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_openai_compatible_env();

        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("http://localhost:11434/v1".to_string()),
            selected_model: Some("llama3.2".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let compat = cfg
            .openai_compatible
            .expect("openai-compatible config should be present");

        assert_eq!(
            compat.model, "llama3.2",
            "model name with dot must not be truncated"
        );
    }
}

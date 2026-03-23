//! LLM configuration types.
//!
//! These types define the configuration for LLM providers. They are defined
//! here (in the `llm` module) so that the module is self-contained and can be
//! extracted into a standalone crate. Resolution logic (reading env vars,
//! settings) lives in `crate::config::llm`.

use std::path::PathBuf;

use secrecy::SecretString;

use crate::bootstrap::ironclaw_base_dir;
use crate::llm::registry::ProviderProtocol;
use crate::llm::session::SessionConfig;

/// Sentinel value used as `api_key` when only an OAuth token is present.
///
/// When we only have an OAuth token the provider factory in `llm/mod.rs`
/// checks for this value and routes to `AnthropicOAuthProvider`, so this
/// placeholder is never sent over the wire.
pub const OAUTH_PLACEHOLDER: &str = "oauth-placeholder";

/// Prompt cache retention policy for Anthropic.
///
/// Controls Anthropic's automatic prompt caching via a top-level
/// `cache_control` field injected through rig-core's `additional_params`.
/// - `None` — caching disabled, no `cache_control` injected.
/// - `Short` — 5-minute TTL (default), `{"type": "ephemeral"}`, 1.25× write surcharge.
/// - `Long` — 1-hour TTL, `{"type": "ephemeral", "ttl": "1h"}`, 2× write surcharge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheRetention {
    /// No prompt caching.
    None,
    /// 5-minute TTL (default). Write cost: 1.25× base input.
    #[default]
    Short,
    /// 1-hour TTL. Write cost: 2× base input.
    Long,
}

impl std::str::FromStr for CacheRetention {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" | "off" | "disabled" => Ok(Self::None),
            "short" | "5m" | "ephemeral" => Ok(Self::Short),
            "long" | "1h" => Ok(Self::Long),
            _ => Err(format!(
                "invalid cache retention '{}', expected one of: none, short, long",
                s
            )),
        }
    }
}

impl std::fmt::Display for CacheRetention {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Short => write!(f, "short"),
            Self::Long => write!(f, "long"),
        }
    }
}

/// Resolved configuration for a registry-based provider.
///
/// This single struct replaces what used to be five separate config types
/// (`OpenAiDirectConfig`, `AnthropicDirectConfig`, `OllamaConfig`,
/// `OpenAiCompatibleConfig`, `TinfoilConfig`). The `protocol` field
/// determines which rig-core client constructor to use.
#[derive(Debug, Clone)]
pub struct RegistryProviderConfig {
    /// Which API protocol to use (determines the rig-core client).
    pub protocol: ProviderProtocol,
    /// Provider identifier (e.g., "groq", "openai", "tinfoil").
    pub provider_id: String,
    /// API key (optional for some providers like Ollama).
    /// For Anthropic OAuth, this is set to `OAUTH_PLACEHOLDER`.
    pub api_key: Option<SecretString>,
    /// Base URL for the API endpoint.
    pub base_url: String,
    /// Model identifier.
    pub model: String,
    /// Extra HTTP headers injected into every request.
    pub extra_headers: Vec<(String, String)>,
    /// OAuth token for providers that support Bearer auth (e.g. Anthropic via `claude login`).
    /// When set, the provider factory routes to the OAuth-specific provider implementation.
    pub oauth_token: Option<SecretString>,
    /// When true, route traffic to the ChatGPT/Codex OAuth-specific
    /// Responses provider instead of the generic registry protocol path.
    pub is_codex_chatgpt: bool,
    /// OAuth refresh token for Codex ChatGPT token refresh.
    pub refresh_token: Option<SecretString>,
    /// Path to Codex auth.json for persisting refreshed tokens.
    pub auth_path: Option<PathBuf>,
    /// Prompt cache retention (Anthropic-specific).
    pub cache_retention: CacheRetention,
    /// Parameter names that this provider does not support (e.g., `["temperature"]`).
    /// Supported keys: `"temperature"`, `"max_tokens"`, `"stop_sequences"`.
    /// Listed parameters are stripped from requests before sending to avoid 400 errors.
    pub unsupported_params: Vec<String>,
}

/// Configuration for OpenAI Codex (ChatGPT subscription OAuth).
#[derive(Debug, Clone)]
pub struct OpenAiCodexConfig {
    /// Model to use (default: "gpt-5.3-codex").
    pub model: String,
    /// OAuth authorization server (default: "https://auth.openai.com").
    pub auth_endpoint: String,
    /// Responses API base URL (default: "https://chatgpt.com/backend-api/codex").
    pub api_base_url: String,
    /// OAuth client ID (default: OpenAI's public Codex client).
    pub client_id: String,
    /// Path to session file (default: ~/.ironclaw/openai_codex_session.json).
    pub session_path: PathBuf,
    /// Seconds before expiry to proactively refresh (default: 300).
    pub token_refresh_margin_secs: u64,
}

impl Default for OpenAiCodexConfig {
    fn default() -> Self {
        Self {
            model: "gpt-5.3-codex".to_string(),
            auth_endpoint: "https://auth.openai.com".to_string(),
            api_base_url: "https://chatgpt.com/backend-api/codex".to_string(),
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann".to_string(),
            session_path: ironclaw_base_dir().join("openai_codex_session.json"),
            token_refresh_margin_secs: 300,
        }
    }
}

/// Configuration for AWS Bedrock (native Converse API).
#[derive(Debug, Clone)]
pub struct BedrockConfig {
    /// AWS region (e.g. "us-east-1").
    pub region: String,
    /// Bedrock model ID (e.g. "anthropic.claude-opus-4-6-v1").
    pub model: String,
    /// Cross-region inference prefix: "us", "eu", "apac", "global", or None.
    pub cross_region: Option<String>,
    /// AWS named profile (for SSO / assume-role workflows).
    pub profile: Option<String>,
}

/// LLM provider configuration.
///
/// NearAI remains the default backend with its own config struct (session auth).
/// All other providers are resolved through the provider registry, producing
/// a generic `RegistryProviderConfig`.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// Backend identifier (e.g., "nearai", "openai", "groq", "tinfoil").
    pub backend: String,
    /// Session manager configuration (auth URL, token persistence path).
    /// Used by the NearAI provider for OAuth/session-token auth.
    pub session: SessionConfig,
    /// NEAR AI config (always populated, also used for embeddings).
    pub nearai: NearAiConfig,
    /// Resolved provider config for registry-based providers.
    /// `None` when backend is "nearai" or "bedrock".
    pub provider: Option<RegistryProviderConfig>,
    /// AWS Bedrock config (populated when backend=bedrock, requires --features bedrock).
    pub bedrock: Option<BedrockConfig>,
    /// Gemini OAuth config (populated when backend=gemini_oauth).
    pub gemini_oauth: Option<GeminiOauthConfig>,
    /// OpenAI Codex config (populated when backend=openai_codex).
    pub openai_codex: Option<OpenAiCodexConfig>,
    /// HTTP request timeout in seconds for LLM API calls.
    /// Default: 120. Increase for local LLMs (Ollama, vLLM, LM Studio) that
    /// need more time for prompt evaluation on consumer hardware.
    pub request_timeout_secs: u64,
    /// Generic cheap/fast model for lightweight tasks (heartbeat, routing, evaluation).
    /// Works with any backend. Set via `LLM_CHEAP_MODEL` env var.
    /// When set, takes priority over the NearAI-specific `NEARAI_CHEAP_MODEL`.
    pub cheap_model: Option<String>,
    /// Enable cascade mode for smart routing (retry with primary if cheap model
    /// response seems uncertain). Default: true. Set via `SMART_ROUTING_CASCADE`.
    pub smart_routing_cascade: bool,
}

impl LlmConfig {
    /// Resolve the effective cheap model name.
    ///
    /// Resolution order:
    /// 1. `LLM_CHEAP_MODEL` (generic, works with any backend)
    /// 2. `NEARAI_CHEAP_MODEL` (NearAI-only, backward compatibility)
    pub fn cheap_model_name(&self) -> Option<&str> {
        self.cheap_model.as_deref().or_else(|| {
            if self.backend == "nearai" {
                self.nearai.cheap_model.as_deref()
            } else {
                None
            }
        })
    }
}

/// NEAR AI configuration.
#[derive(Debug, Clone)]
pub struct NearAiConfig {
    /// Model to use (e.g., "claude-3-5-sonnet-20241022", "gpt-4o")
    pub model: String,
    /// Cheap/fast model for lightweight tasks (heartbeat, routing, evaluation).
    pub cheap_model: Option<String>,
    /// Base URL for the NEAR AI API.
    pub base_url: String,
    /// API key for NEAR AI Cloud.
    pub api_key: Option<SecretString>,
    /// Optional fallback model for failover.
    pub fallback_model: Option<String>,
    /// Maximum number of retries for transient errors (default: 3).
    pub max_retries: u32,
    /// Consecutive failures before circuit breaker opens. None = disabled.
    pub circuit_breaker_threshold: Option<u32>,
    /// Seconds the circuit stays open before probing (default: 30).
    pub circuit_breaker_recovery_secs: u64,
    /// Enable in-memory response caching. Default: false.
    pub response_cache_enabled: bool,
    /// TTL in seconds for cached responses (default: 3600).
    pub response_cache_ttl_secs: u64,
    /// Max cached responses before LRU eviction (default: 1000).
    pub response_cache_max_entries: usize,
    /// Cooldown duration in seconds for failover (default: 300).
    pub failover_cooldown_secs: u64,
    /// Consecutive failures before failover cooldown (default: 3).
    pub failover_cooldown_threshold: u32,
    /// Enable cascade mode for smart routing. Default: true.
    pub smart_routing_cascade: bool,
}

impl NearAiConfig {
    /// Create a minimal config suitable for listing available models.
    ///
    /// Reads `NEARAI_API_KEY` from the environment and selects the
    /// appropriate base URL (cloud-api when API key is present,
    /// private.near.ai for session-token auth).
    pub(crate) fn for_model_discovery() -> Self {
        let api_key = crate::config::helpers::env_or_override("NEARAI_API_KEY")
            .filter(|k| !k.is_empty())
            .map(SecretString::from);

        let default_base = if api_key.is_some() {
            "https://cloud-api.near.ai"
        } else {
            "https://private.near.ai"
        };
        let base_url = crate::config::helpers::env_or_override("NEARAI_BASE_URL")
            .unwrap_or_else(|| default_base.to_string());

        Self {
            model: String::new(),
            cheap_model: None,
            base_url,
            api_key,
            fallback_model: None,
            max_retries: 3,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: true,
        }
    }
}

/// Configuration for Gemini OAuth integration.
///
/// Extended generation config parameters (topP, topK, seed, etc.) are read from
/// environment variables at request time:
/// - `GEMINI_TOP_P` — nucleus sampling (0.0–1.0)
/// - `GEMINI_TOP_K` — top-k sampling (integer)
/// - `GEMINI_SEED` — deterministic generation seed
/// - `GEMINI_PRESENCE_PENALTY` — presence penalty (-2.0–2.0)
/// - `GEMINI_FREQUENCY_PENALTY` — frequency penalty (-2.0–2.0)
/// - `GEMINI_RESPONSE_MIME_TYPE` — e.g. "application/json"
/// - `GEMINI_RESPONSE_JSON_SCHEMA` — JSON schema string for structured output
/// - `GEMINI_CACHED_CONTENT` — cached content resource name
/// - `GEMINI_CLI_CUSTOM_HEADERS` — custom headers (key:value,key:value)
/// - `GOOGLE_GENAI_API_VERSION` — API version (default: v1beta)
/// - `GEMINI_API_KEY` — optional API key for non-OAuth auth mode
/// - `GEMINI_API_KEY_AUTH_MECHANISM` — "x-goog-api-key" (default) or "bearer"
#[derive(Debug, Clone)]
pub struct GeminiOauthConfig {
    pub model: String,
    pub credentials_path: PathBuf,
}

impl GeminiOauthConfig {
    pub fn default_credentials_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".gemini")
            .join("oauth_creds.json")
    }
}

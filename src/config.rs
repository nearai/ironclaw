//! Configuration for IronClaw.

use std::path::PathBuf;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use crate::error::ConfigError;

/// Main configuration for the agent.
#[derive(Debug, Clone)]
pub struct Config {
    pub database: DatabaseConfig,
    pub llm: LlmConfig,
    pub embeddings: EmbeddingsConfig,
    pub tunnel: TunnelConfig,
    pub channels: ChannelsConfig,
    pub agent: AgentConfig,
    pub safety: SafetyConfig,
    pub wasm: WasmConfig,
    pub secrets: SecretsConfig,
    pub builder: BuilderModeConfig,
    pub heartbeat: HeartbeatConfig,
    pub sandbox: SandboxModeConfig,
}

impl Config {
    /// Load configuration from environment variables.
    pub fn from_env() -> Result<Self, ConfigError> {
        // Load .env file if present (ignore errors if not found)
        let _ = dotenvy::dotenv();

        Ok(Self {
            database: DatabaseConfig::from_env()?,
            llm: LlmConfig::from_env()?,
            embeddings: EmbeddingsConfig::from_env()?,
            tunnel: TunnelConfig::from_env()?,
            channels: ChannelsConfig::from_env()?,
            agent: AgentConfig::from_env()?,
            safety: SafetyConfig::from_env()?,
            wasm: WasmConfig::from_env()?,
            secrets: SecretsConfig::from_env()?,
            builder: BuilderModeConfig::from_env()?,
            heartbeat: HeartbeatConfig::from_env()?,
            sandbox: SandboxModeConfig::from_env()?,
        })
    }
}

/// Tunnel configuration for exposing the agent to the internet.
///
/// Used by channels and tools that need public webhook endpoints.
/// The tunnel URL is shared across all channels (Telegram, Slack, etc.).
///
/// # Security Notes
///
/// **Webhook endpoints** (e.g., `/webhook/telegram`) should NOT use tunnel-level
/// authentication because webhook providers (Telegram, Slack, GitHub) need
/// unauthenticated access to POST updates. Security for webhooks comes from:
/// - Webhook signature verification (provider-specific secrets)
/// - IP allowlisting (if supported by provider)
///
/// **Non-webhook endpoints** (admin APIs, health checks) CAN be protected using
/// tunnel provider features:
/// - ngrok: Basic Auth, OAuth, IP restrictions
/// - Cloudflare: Access policies, mTLS
///
/// These protections are configured in the tunnel provider, not here.
///
/// # Supported Providers
///
/// - **ngrok**: `ngrok http 8080` -> `https://abc123.ngrok.io`
/// - **Cloudflare Tunnel**: `cloudflared tunnel --url http://localhost:8080`
/// - **localtunnel**: `lt --port 8080`
/// - Any service that provides a public HTTPS URL to localhost
#[derive(Debug, Clone, Default)]
pub struct TunnelConfig {
    /// Public URL from tunnel provider (e.g., "https://abc123.ngrok.io").
    ///
    /// When set, channels that support webhooks will register their endpoints
    /// with this base URL instead of using polling.
    pub public_url: Option<String>,
}

impl TunnelConfig {
    fn from_env() -> Result<Self, ConfigError> {
        // Priority: env var > settings file
        let public_url = optional_env("TUNNEL_URL")?.or_else(|| {
            crate::settings::Settings::load()
                .tunnel
                .public_url
                .filter(|s| !s.is_empty())
        });

        // Validate URL format if provided
        if let Some(ref url) = public_url {
            if !url.starts_with("https://") {
                return Err(ConfigError::InvalidValue {
                    key: "TUNNEL_URL".to_string(),
                    message: "must start with https:// (webhooks require HTTPS)".to_string(),
                });
            }
        }

        Ok(Self { public_url })
    }

    /// Check if a tunnel is configured.
    pub fn is_enabled(&self) -> bool {
        self.public_url.is_some()
    }

    /// Get the webhook URL for a given path.
    ///
    /// Returns `None` if no tunnel is configured.
    pub fn webhook_url(&self, path: &str) -> Option<String> {
        self.public_url.as_ref().map(|base| {
            let base = base.trim_end_matches('/');
            let path = path.trim_start_matches('/');
            format!("{}/{}", base, path)
        })
    }
}

/// Database configuration.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub url: SecretString,
    pub pool_size: usize,
}

impl DatabaseConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let settings = crate::settings::Settings::load();

        // Priority: env var > settings > error (required)
        let url = optional_env("DATABASE_URL")?
            .or(settings.database_url.clone())
            .ok_or_else(|| ConfigError::MissingRequired {
                key: "database_url".to_string(),
                hint: "Run 'ironclaw onboard' or set DATABASE_URL environment variable".to_string(),
            })?;

        // Priority: env var > settings > default
        let pool_size = optional_env("DATABASE_POOL_SIZE")?
            .map(|s| s.parse())
            .transpose()
            .map_err(|e| ConfigError::InvalidValue {
                key: "DATABASE_POOL_SIZE".to_string(),
                message: format!("must be a positive integer: {e}"),
            })?
            .or(settings.database_pool_size)
            .unwrap_or(10);

        Ok(Self {
            url: SecretString::from(url),
            pool_size,
        })
    }

    /// Get the database URL (exposes the secret).
    pub fn url(&self) -> &str {
        self.url.expose_secret()
    }
}

/// LLM provider configuration (NEAR AI only).
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub nearai: NearAiConfig,
}

/// API mode for NEAR AI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NearAiApiMode {
    /// Use the Responses API (chat-api proxy) - session-based auth
    #[default]
    Responses,
    /// Use the Chat Completions API (cloud-api) - API key auth
    ChatCompletions,
}

impl std::str::FromStr for NearAiApiMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "responses" | "response" => Ok(Self::Responses),
            "chat_completions" | "chatcompletions" | "chat" | "completions" => {
                Ok(Self::ChatCompletions)
            }
            _ => Err(format!(
                "invalid API mode '{}', expected 'responses' or 'chat_completions'",
                s
            )),
        }
    }
}

/// NEAR AI chat-api configuration.
#[derive(Debug, Clone)]
pub struct NearAiConfig {
    /// Model to use (e.g., "claude-3-5-sonnet-20241022", "gpt-4o")
    pub model: String,
    /// Base URL for the NEAR AI API (default: https://api.near.ai)
    pub base_url: String,
    /// Base URL for auth/refresh endpoints (default: https://private.near.ai)
    pub auth_base_url: String,
    /// Path to session file (default: ~/.ironclaw/session.json)
    pub session_path: PathBuf,
    /// API mode: "responses" (chat-api) or "chat_completions" (cloud-api)
    pub api_mode: NearAiApiMode,
    /// API key for cloud-api (required for chat_completions mode)
    pub api_key: Option<SecretString>,
    /// Maximum number of retries for transient errors (default: 3).
    pub max_retries: u32,
}

impl LlmConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let api_key = optional_env("NEARAI_API_KEY")?.map(SecretString::from);

        // Determine API mode: explicit setting, or infer from API key presence
        let api_mode = if let Some(mode_str) = optional_env("NEARAI_API_MODE")? {
            mode_str.parse().map_err(|e| ConfigError::InvalidValue {
                key: "NEARAI_API_MODE".to_string(),
                message: e,
            })?
        } else if api_key.is_some() {
            // If API key is provided, default to chat_completions mode
            NearAiApiMode::ChatCompletions
        } else {
            NearAiApiMode::Responses
        };

        Ok(Self {
            nearai: NearAiConfig {
                // Load model from saved settings first, then env, then default
                model: crate::settings::Settings::load()
                    .selected_model
                    .or_else(|| optional_env("NEARAI_MODEL").ok().flatten())
                    .unwrap_or_else(|| {
                        "fireworks::accounts/fireworks/models/llama4-maverick-instruct-basic"
                            .to_string()
                    }),
                base_url: optional_env("NEARAI_BASE_URL")?
                    .unwrap_or_else(|| "https://cloud-api.near.ai".to_string()),
                auth_base_url: optional_env("NEARAI_AUTH_URL")?
                    .unwrap_or_else(|| "https://private.near.ai".to_string()),
                session_path: optional_env("NEARAI_SESSION_PATH")?
                    .map(PathBuf::from)
                    .unwrap_or_else(default_session_path),
                api_mode,
                api_key,
                max_retries: parse_optional_env("NEARAI_MAX_RETRIES", 3)?,
            },
        })
    }
}

/// Embeddings provider configuration.
#[derive(Debug, Clone)]
pub struct EmbeddingsConfig {
    /// Whether embeddings are enabled.
    pub enabled: bool,
    /// Provider to use: "openai" or "nearai"
    pub provider: String,
    /// OpenAI API key (for OpenAI provider).
    pub openai_api_key: Option<SecretString>,
    /// Model to use for embeddings.
    /// For OpenAI: "text-embedding-3-small", "text-embedding-3-large", "text-embedding-ada-002"
    /// For NEAR AI: Uses the configured session for auth.
    pub model: String,
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "openai".to_string(),
            openai_api_key: None,
            model: "text-embedding-3-small".to_string(),
        }
    }
}

impl EmbeddingsConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let settings = crate::settings::Settings::load();
        let openai_api_key = optional_env("OPENAI_API_KEY")?.map(SecretString::from);

        // Priority: env var > settings > default
        let provider = optional_env("EMBEDDING_PROVIDER")?
            .unwrap_or_else(|| settings.embeddings.provider.clone());

        let model =
            optional_env("EMBEDDING_MODEL")?.unwrap_or_else(|| settings.embeddings.model.clone());

        // Priority: env var > settings > auto-detect from API key
        let enabled = optional_env("EMBEDDING_ENABLED")?
            .map(|s| s.parse())
            .transpose()
            .map_err(|e| ConfigError::InvalidValue {
                key: "EMBEDDING_ENABLED".to_string(),
                message: format!("must be 'true' or 'false': {e}"),
            })?
            .unwrap_or_else(|| {
                // Check settings, or auto-enable if API key present
                settings.embeddings.enabled || openai_api_key.is_some()
            });

        Ok(Self {
            enabled,
            provider,
            openai_api_key,
            model,
        })
    }

    /// Get the OpenAI API key if configured.
    pub fn openai_api_key(&self) -> Option<&str> {
        self.openai_api_key.as_ref().map(|s| s.expose_secret())
    }
}

/// Get the default session file path (~/.ironclaw/session.json).
fn default_session_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("session.json")
}

/// Channel configurations.
#[derive(Debug, Clone)]
pub struct ChannelsConfig {
    pub cli: CliConfig,
    pub http: Option<HttpConfig>,
    pub gateway: Option<GatewayConfig>,
    /// Directory containing WASM channel modules (default: ~/.ironclaw/channels/).
    pub wasm_channels_dir: std::path::PathBuf,
    /// Whether WASM channels are enabled.
    pub wasm_channels_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct CliConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
    pub webhook_secret: Option<SecretString>,
    pub user_id: String,
}

/// Web gateway configuration.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub host: String,
    pub port: u16,
    /// Bearer token for authentication. Random hex generated at startup if unset.
    pub auth_token: Option<String>,
    pub user_id: String,
}

impl ChannelsConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let http = if optional_env("HTTP_PORT")?.is_some() || optional_env("HTTP_HOST")?.is_some() {
            Some(HttpConfig {
                host: optional_env("HTTP_HOST")?.unwrap_or_else(|| "0.0.0.0".to_string()),
                port: optional_env("HTTP_PORT")?
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|e| ConfigError::InvalidValue {
                        key: "HTTP_PORT".to_string(),
                        message: format!("must be a valid port number: {e}"),
                    })?
                    .unwrap_or(8080),
                webhook_secret: optional_env("HTTP_WEBHOOK_SECRET")?.map(SecretString::from),
                user_id: optional_env("HTTP_USER_ID")?.unwrap_or_else(|| "http".to_string()),
            })
        } else {
            None
        };

        let gateway = if optional_env("GATEWAY_ENABLED")?
            .map(|s| s.to_lowercase() == "true" || s == "1")
            .unwrap_or(false)
        {
            Some(GatewayConfig {
                host: optional_env("GATEWAY_HOST")?.unwrap_or_else(|| "127.0.0.1".to_string()),
                port: optional_env("GATEWAY_PORT")?
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|e| ConfigError::InvalidValue {
                        key: "GATEWAY_PORT".to_string(),
                        message: format!("must be a valid port number: {e}"),
                    })?
                    .unwrap_or(3000),
                auth_token: optional_env("GATEWAY_AUTH_TOKEN")?,
                user_id: optional_env("GATEWAY_USER_ID")?.unwrap_or_else(|| "default".to_string()),
            })
        } else {
            None
        };

        let cli_enabled = optional_env("CLI_ENABLED")?
            .map(|s| s.to_lowercase() != "false" && s != "0")
            .unwrap_or(true);

        Ok(Self {
            cli: CliConfig {
                enabled: cli_enabled,
            },
            http,
            gateway,
            wasm_channels_dir: optional_env("WASM_CHANNELS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_channels_dir),
            wasm_channels_enabled: optional_env("WASM_CHANNELS_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "WASM_CHANNELS_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
        })
    }
}

/// Get the default channels directory (~/.ironclaw/channels/).
fn default_channels_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("channels")
}

/// Agent behavior configuration.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub max_parallel_jobs: usize,
    pub job_timeout: Duration,
    pub stuck_threshold: Duration,
    pub repair_check_interval: Duration,
    pub max_repair_attempts: u32,
    /// Whether to use planning before tool execution.
    pub use_planning: bool,
    /// Session idle timeout. Sessions inactive longer than this are pruned.
    pub session_idle_timeout: Duration,
}

impl AgentConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let settings = crate::settings::Settings::load();

        Ok(Self {
            // Priority: env var > settings > default
            name: optional_env("AGENT_NAME")?.unwrap_or_else(|| settings.agent.name.clone()),
            max_parallel_jobs: optional_env("AGENT_MAX_PARALLEL_JOBS")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "AGENT_MAX_PARALLEL_JOBS".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?
                .unwrap_or(settings.agent.max_parallel_jobs as usize),
            job_timeout: Duration::from_secs(
                optional_env("AGENT_JOB_TIMEOUT_SECS")?
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|e| ConfigError::InvalidValue {
                        key: "AGENT_JOB_TIMEOUT_SECS".to_string(),
                        message: format!("must be a positive integer: {e}"),
                    })?
                    .unwrap_or(settings.agent.job_timeout_secs),
            ),
            stuck_threshold: Duration::from_secs(
                optional_env("AGENT_STUCK_THRESHOLD_SECS")?
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|e| ConfigError::InvalidValue {
                        key: "AGENT_STUCK_THRESHOLD_SECS".to_string(),
                        message: format!("must be a positive integer: {e}"),
                    })?
                    .unwrap_or(settings.agent.stuck_threshold_secs),
            ),
            repair_check_interval: Duration::from_secs(
                optional_env("SELF_REPAIR_CHECK_INTERVAL_SECS")?
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|e| ConfigError::InvalidValue {
                        key: "SELF_REPAIR_CHECK_INTERVAL_SECS".to_string(),
                        message: format!("must be a positive integer: {e}"),
                    })?
                    .unwrap_or(settings.agent.repair_check_interval_secs),
            ),
            max_repair_attempts: optional_env("SELF_REPAIR_MAX_ATTEMPTS")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "SELF_REPAIR_MAX_ATTEMPTS".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?
                .unwrap_or(settings.agent.max_repair_attempts),
            use_planning: optional_env("AGENT_USE_PLANNING")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "AGENT_USE_PLANNING".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(settings.agent.use_planning),
            session_idle_timeout: Duration::from_secs(
                optional_env("SESSION_IDLE_TIMEOUT_SECS")?
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|e| ConfigError::InvalidValue {
                        key: "SESSION_IDLE_TIMEOUT_SECS".to_string(),
                        message: format!("must be a positive integer: {e}"),
                    })?
                    .unwrap_or(settings.agent.session_idle_timeout_secs),
            ),
        })
    }
}

/// Safety configuration.
#[derive(Debug, Clone)]
pub struct SafetyConfig {
    pub max_output_length: usize,
    pub injection_check_enabled: bool,
}

impl SafetyConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            max_output_length: parse_optional_env("SAFETY_MAX_OUTPUT_LENGTH", 100_000)?,
            injection_check_enabled: optional_env("SAFETY_INJECTION_CHECK_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "SAFETY_INJECTION_CHECK_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
        })
    }
}

/// WASM sandbox configuration.
#[derive(Debug, Clone)]
pub struct WasmConfig {
    /// Whether WASM tool execution is enabled.
    pub enabled: bool,
    /// Directory containing installed WASM tools (default: ~/.ironclaw/tools/).
    pub tools_dir: PathBuf,
    /// Default memory limit in bytes (default: 10 MB).
    pub default_memory_limit: u64,
    /// Default execution timeout in seconds (default: 60).
    pub default_timeout_secs: u64,
    /// Default fuel limit for CPU metering (default: 10M).
    pub default_fuel_limit: u64,
    /// Whether to cache compiled modules.
    pub cache_compiled: bool,
    /// Directory for compiled module cache.
    pub cache_dir: Option<PathBuf>,
}

/// Secrets management configuration.
#[derive(Clone, Default)]
pub struct SecretsConfig {
    /// Master key for encrypting secrets.
    /// Source determined by KeySource in settings.
    pub master_key: Option<SecretString>,
    /// Whether secrets management is enabled.
    pub enabled: bool,
    /// Source of the master key.
    pub source: crate::settings::KeySource,
}

impl std::fmt::Debug for SecretsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretsConfig")
            .field("master_key", &self.master_key.is_some())
            .field("enabled", &self.enabled)
            .field("source", &self.source)
            .finish()
    }
}

impl SecretsConfig {
    fn from_env() -> Result<Self, ConfigError> {
        use crate::settings::KeySource;

        let settings = crate::settings::Settings::load();

        // Priority: env var > keychain (based on settings) > disabled
        let (master_key, source) = if let Some(env_key) = optional_env("SECRETS_MASTER_KEY")? {
            // Env var takes priority (for CI/Docker)
            (Some(SecretString::from(env_key)), KeySource::Env)
        } else {
            match settings.secrets_master_key_source {
                KeySource::Keychain => {
                    // Try to load from OS keychain
                    match crate::secrets::keychain::get_master_key() {
                        Ok(key_bytes) => {
                            let key_hex: String =
                                key_bytes.iter().map(|b| format!("{:02x}", b)).collect();
                            (Some(SecretString::from(key_hex)), KeySource::Keychain)
                        }
                        Err(_) => {
                            // Keychain configured but key not found
                            // This might happen if keychain was cleared
                            tracing::warn!(
                                "Secrets configured for keychain but key not found. \
                                 Run 'ironclaw onboard' to reconfigure."
                            );
                            (None, KeySource::None)
                        }
                    }
                }
                KeySource::Env => {
                    // Settings say env, but no env var found
                    tracing::warn!(
                        "Secrets configured for env var but SECRETS_MASTER_KEY not set."
                    );
                    (None, KeySource::None)
                }
                KeySource::None => (None, KeySource::None),
            }
        };

        let enabled = master_key.is_some();

        // Validate master key length if provided
        if let Some(ref key) = master_key {
            if key.expose_secret().len() < 32 {
                return Err(ConfigError::InvalidValue {
                    key: "SECRETS_MASTER_KEY".to_string(),
                    message: "must be at least 32 bytes for AES-256-GCM".to_string(),
                });
            }
        }

        Ok(Self {
            master_key,
            enabled,
            source,
        })
    }

    /// Get the master key if configured.
    pub fn master_key(&self) -> Option<&SecretString> {
        self.master_key.as_ref()
    }
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            tools_dir: default_tools_dir(),
            default_memory_limit: 10 * 1024 * 1024, // 10 MB
            default_timeout_secs: 60,
            default_fuel_limit: 10_000_000,
            cache_compiled: true,
            cache_dir: None,
        }
    }
}

/// Get the default tools directory (~/.ironclaw/tools/).
fn default_tools_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("tools")
}

impl WasmConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: optional_env("WASM_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "WASM_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
            tools_dir: optional_env("WASM_TOOLS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_tools_dir),
            default_memory_limit: parse_optional_env(
                "WASM_DEFAULT_MEMORY_LIMIT",
                10 * 1024 * 1024,
            )?,
            default_timeout_secs: parse_optional_env("WASM_DEFAULT_TIMEOUT_SECS", 60)?,
            default_fuel_limit: parse_optional_env("WASM_DEFAULT_FUEL_LIMIT", 10_000_000)?,
            cache_compiled: optional_env("WASM_CACHE_COMPILED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "WASM_CACHE_COMPILED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
            cache_dir: optional_env("WASM_CACHE_DIR")?.map(PathBuf::from),
        })
    }

    /// Convert to WasmRuntimeConfig.
    pub fn to_runtime_config(&self) -> crate::tools::wasm::WasmRuntimeConfig {
        use crate::tools::wasm::{FuelConfig, ResourceLimits, WasmRuntimeConfig};
        use std::time::Duration;

        WasmRuntimeConfig {
            default_limits: ResourceLimits {
                memory_bytes: self.default_memory_limit,
                fuel: self.default_fuel_limit,
                timeout: Duration::from_secs(self.default_timeout_secs),
            },
            fuel_config: FuelConfig {
                initial_fuel: self.default_fuel_limit,
                enabled: true,
            },
            cache_compiled: self.cache_compiled,
            cache_dir: self.cache_dir.clone(),
            optimization_level: wasmtime::OptLevel::Speed,
        }
    }
}

/// Builder mode configuration.
#[derive(Debug, Clone)]
pub struct BuilderModeConfig {
    /// Whether the software builder tool is enabled.
    pub enabled: bool,
    /// Directory for build artifacts (default: temp dir).
    pub build_dir: Option<PathBuf>,
    /// Maximum iterations for the build loop.
    pub max_iterations: u32,
    /// Build timeout in seconds.
    pub timeout_secs: u64,
    /// Whether to automatically register built WASM tools.
    pub auto_register: bool,
}

impl Default for BuilderModeConfig {
    fn default() -> Self {
        Self {
            enabled: true, // Builder enabled by default
            build_dir: None,
            max_iterations: 20,
            timeout_secs: 600,
            auto_register: true,
        }
    }
}

impl BuilderModeConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: optional_env("BUILDER_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "BUILDER_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true), // Builder enabled by default
            build_dir: optional_env("BUILDER_DIR")?.map(PathBuf::from),
            max_iterations: parse_optional_env("BUILDER_MAX_ITERATIONS", 20)?,
            timeout_secs: parse_optional_env("BUILDER_TIMEOUT_SECS", 600)?,
            auto_register: optional_env("BUILDER_AUTO_REGISTER")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "BUILDER_AUTO_REGISTER".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
        })
    }

    /// Convert to BuilderConfig for the builder tool.
    pub fn to_builder_config(&self) -> crate::tools::BuilderConfig {
        crate::tools::BuilderConfig {
            build_dir: self.build_dir.clone().unwrap_or_else(std::env::temp_dir),
            max_iterations: self.max_iterations,
            timeout: Duration::from_secs(self.timeout_secs),
            cleanup_on_failure: true,
            validate_wasm: true,
            run_tests: true,
            auto_register: self.auto_register,
            wasm_output_dir: None,
        }
    }
}

/// Heartbeat configuration.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Whether heartbeat is enabled.
    pub enabled: bool,
    /// Interval between heartbeat checks in seconds.
    pub interval_secs: u64,
    /// Channel to notify on heartbeat findings.
    pub notify_channel: Option<String>,
    /// User ID to notify on heartbeat findings.
    pub notify_user: Option<String>,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 1800, // 30 minutes
            notify_channel: None,
            notify_user: None,
        }
    }
}

impl HeartbeatConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let settings = crate::settings::Settings::load();

        Ok(Self {
            // Priority: env var > settings > default
            enabled: optional_env("HEARTBEAT_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "HEARTBEAT_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(settings.heartbeat.enabled),
            interval_secs: optional_env("HEARTBEAT_INTERVAL_SECS")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "HEARTBEAT_INTERVAL_SECS".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?
                .unwrap_or(settings.heartbeat.interval_secs),
            notify_channel: optional_env("HEARTBEAT_NOTIFY_CHANNEL")?
                .or(settings.heartbeat.notify_channel.clone()),
            notify_user: optional_env("HEARTBEAT_NOTIFY_USER")?
                .or(settings.heartbeat.notify_user.clone()),
        })
    }
}

/// Docker sandbox configuration.
#[derive(Debug, Clone)]
pub struct SandboxModeConfig {
    /// Whether the Docker sandbox is enabled.
    pub enabled: bool,
    /// Sandbox policy: "readonly", "workspace_write", or "full_access".
    pub policy: String,
    /// Command timeout in seconds.
    pub timeout_secs: u64,
    /// Memory limit in megabytes.
    pub memory_limit_mb: u64,
    /// CPU shares (relative weight).
    pub cpu_shares: u32,
    /// Docker image for the sandbox.
    pub image: String,
    /// Whether to auto-pull the image if not found.
    pub auto_pull_image: bool,
    /// Additional domains to allow through the network proxy.
    pub extra_allowed_domains: Vec<String>,
}

impl Default for SandboxModeConfig {
    fn default() -> Self {
        Self {
            enabled: true, // Enabled by default
            policy: "readonly".to_string(),
            timeout_secs: 120,
            memory_limit_mb: 2048,
            cpu_shares: 1024,
            image: "ghcr.io/nearai/sandbox:latest".to_string(),
            auto_pull_image: true,
            extra_allowed_domains: Vec::new(),
        }
    }
}

impl SandboxModeConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let extra_domains = optional_env("SANDBOX_EXTRA_DOMAINS")?
            .map(|s| s.split(',').map(|d| d.trim().to_string()).collect())
            .unwrap_or_default();

        Ok(Self {
            enabled: optional_env("SANDBOX_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "SANDBOX_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
            policy: optional_env("SANDBOX_POLICY")?.unwrap_or_else(|| "readonly".to_string()),
            timeout_secs: parse_optional_env("SANDBOX_TIMEOUT_SECS", 120)?,
            memory_limit_mb: parse_optional_env("SANDBOX_MEMORY_LIMIT_MB", 2048)?,
            cpu_shares: parse_optional_env("SANDBOX_CPU_SHARES", 1024)?,
            image: optional_env("SANDBOX_IMAGE")?
                .unwrap_or_else(|| "ghcr.io/nearai/sandbox:latest".to_string()),
            auto_pull_image: optional_env("SANDBOX_AUTO_PULL")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "SANDBOX_AUTO_PULL".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
            extra_allowed_domains: extra_domains,
        })
    }

    /// Convert to SandboxConfig for the sandbox module.
    pub fn to_sandbox_config(&self) -> crate::sandbox::SandboxConfig {
        use crate::sandbox::SandboxPolicy;
        use std::time::Duration;

        let policy = self.policy.parse().unwrap_or(SandboxPolicy::ReadOnly);

        let mut allowlist = crate::sandbox::default_allowlist();
        allowlist.extend(self.extra_allowed_domains.clone());

        crate::sandbox::SandboxConfig {
            enabled: self.enabled,
            policy,
            timeout: Duration::from_secs(self.timeout_secs),
            memory_limit_mb: self.memory_limit_mb,
            cpu_shares: self.cpu_shares,
            network_allowlist: allowlist,
            image: self.image.clone(),
            auto_pull_image: self.auto_pull_image,
            proxy_port: 0, // Auto-assign
        }
    }
}

// Helper functions

pub(crate) fn optional_env(key: &str) -> Result<Option<String>, ConfigError> {
    match std::env::var(key) {
        Ok(val) if val.is_empty() => Ok(None),
        Ok(val) => Ok(Some(val)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(ConfigError::ParseError(format!(
            "failed to read {key}: {e}"
        ))),
    }
}

pub(crate) fn parse_optional_env<T>(key: &str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    optional_env(key)?
        .map(|s| {
            s.parse().map_err(|e| ConfigError::InvalidValue {
                key: key.to_string(),
                message: format!("{e}"),
            })
        })
        .transpose()
        .map(|opt| opt.unwrap_or(default))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env vars are process-global, so serialize tests that mutate them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // --- optional_env tests ---

    #[test]
    fn optional_env_returns_none_for_missing_var() {
        let _lock = ENV_LOCK.lock();
        // Use a unique key that won't exist in the real environment.
        unsafe { std::env::remove_var("_TEST_CFG_MISSING_42") };
        let result = optional_env("_TEST_CFG_MISSING_42").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn optional_env_returns_none_for_empty_string() {
        let _lock = ENV_LOCK.lock();
        unsafe { std::env::set_var("_TEST_CFG_EMPTY_42", "") };
        let result = optional_env("_TEST_CFG_EMPTY_42").unwrap();
        assert!(result.is_none());
        unsafe { std::env::remove_var("_TEST_CFG_EMPTY_42") };
    }

    #[test]
    fn optional_env_returns_value_when_set() {
        let _lock = ENV_LOCK.lock();
        unsafe { std::env::set_var("_TEST_CFG_SET_42", "hello") };
        let result = optional_env("_TEST_CFG_SET_42").unwrap();
        assert_eq!(result, Some("hello".to_string()));
        unsafe { std::env::remove_var("_TEST_CFG_SET_42") };
    }

    // --- parse_optional_env tests ---

    #[test]
    fn parse_optional_env_returns_default_when_missing() {
        let _lock = ENV_LOCK.lock();
        unsafe { std::env::remove_var("_TEST_CFG_PARSE_MISSING_42") };
        let result: u64 = parse_optional_env("_TEST_CFG_PARSE_MISSING_42", 999).unwrap();
        assert_eq!(result, 999);
    }

    #[test]
    fn parse_optional_env_parses_value() {
        let _lock = ENV_LOCK.lock();
        unsafe { std::env::set_var("_TEST_CFG_PARSE_VAL_42", "42") };
        let result: u64 = parse_optional_env("_TEST_CFG_PARSE_VAL_42", 0).unwrap();
        assert_eq!(result, 42);
        unsafe { std::env::remove_var("_TEST_CFG_PARSE_VAL_42") };
    }

    #[test]
    fn parse_optional_env_returns_error_for_invalid_value() {
        let _lock = ENV_LOCK.lock();
        unsafe { std::env::set_var("_TEST_CFG_PARSE_BAD_42", "not_a_number") };
        let result: Result<u64, _> = parse_optional_env("_TEST_CFG_PARSE_BAD_42", 0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::InvalidValue { .. }));
        unsafe { std::env::remove_var("_TEST_CFG_PARSE_BAD_42") };
    }

    // --- NearAiApiMode::from_str tests ---

    #[test]
    fn nearai_api_mode_parses_responses() {
        assert_eq!(
            "responses".parse::<NearAiApiMode>().unwrap(),
            NearAiApiMode::Responses
        );
        assert_eq!(
            "response".parse::<NearAiApiMode>().unwrap(),
            NearAiApiMode::Responses
        );
        assert_eq!(
            "RESPONSES".parse::<NearAiApiMode>().unwrap(),
            NearAiApiMode::Responses
        );
    }

    #[test]
    fn nearai_api_mode_parses_chat_completions() {
        assert_eq!(
            "chat_completions".parse::<NearAiApiMode>().unwrap(),
            NearAiApiMode::ChatCompletions
        );
        assert_eq!(
            "chatcompletions".parse::<NearAiApiMode>().unwrap(),
            NearAiApiMode::ChatCompletions
        );
        assert_eq!(
            "chat".parse::<NearAiApiMode>().unwrap(),
            NearAiApiMode::ChatCompletions
        );
        assert_eq!(
            "completions".parse::<NearAiApiMode>().unwrap(),
            NearAiApiMode::ChatCompletions
        );
    }

    #[test]
    fn nearai_api_mode_rejects_invalid() {
        assert!("unknown".parse::<NearAiApiMode>().is_err());
        assert!("".parse::<NearAiApiMode>().is_err());
    }

    #[test]
    fn nearai_api_mode_default_is_responses() {
        assert_eq!(NearAiApiMode::default(), NearAiApiMode::Responses);
    }

    // --- TunnelConfig tests ---

    #[test]
    fn tunnel_is_enabled_when_url_set() {
        let config = TunnelConfig {
            public_url: Some("https://abc.ngrok.io".to_string()),
        };
        assert!(config.is_enabled());
    }

    #[test]
    fn tunnel_is_disabled_when_url_none() {
        let config = TunnelConfig { public_url: None };
        assert!(!config.is_enabled());
    }

    #[test]
    fn tunnel_webhook_url_combines_base_and_path() {
        let config = TunnelConfig {
            public_url: Some("https://abc.ngrok.io".to_string()),
        };
        assert_eq!(
            config.webhook_url("/webhook/telegram"),
            Some("https://abc.ngrok.io/webhook/telegram".to_string())
        );
    }

    #[test]
    fn tunnel_webhook_url_strips_trailing_slash() {
        let config = TunnelConfig {
            public_url: Some("https://abc.ngrok.io/".to_string()),
        };
        assert_eq!(
            config.webhook_url("/webhook/slack"),
            Some("https://abc.ngrok.io/webhook/slack".to_string())
        );
    }

    #[test]
    fn tunnel_webhook_url_returns_none_when_disabled() {
        let config = TunnelConfig { public_url: None };
        assert!(config.webhook_url("/webhook/test").is_none());
    }

    // --- EmbeddingsConfig default tests ---

    #[test]
    fn embeddings_config_defaults() {
        let config = EmbeddingsConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.provider, "openai");
        assert!(config.openai_api_key.is_none());
        assert_eq!(config.model, "text-embedding-3-small");
    }

    #[test]
    fn embeddings_config_openai_api_key_accessor() {
        let config = EmbeddingsConfig {
            openai_api_key: Some(SecretString::from("sk-test123".to_string())),
            ..Default::default()
        };
        assert_eq!(config.openai_api_key(), Some("sk-test123"));

        let config_none = EmbeddingsConfig::default();
        assert!(config_none.openai_api_key().is_none());
    }

    // --- WasmConfig default tests ---

    #[test]
    fn wasm_config_defaults() {
        let config = WasmConfig::default();
        assert!(config.enabled);
        assert_eq!(config.default_memory_limit, 10 * 1024 * 1024);
        assert_eq!(config.default_timeout_secs, 60);
        assert_eq!(config.default_fuel_limit, 10_000_000);
        assert!(config.cache_compiled);
        assert!(config.cache_dir.is_none());
    }

    // --- BuilderModeConfig default tests ---

    #[test]
    fn builder_config_defaults() {
        let config = BuilderModeConfig::default();
        assert!(config.enabled);
        assert!(config.build_dir.is_none());
        assert_eq!(config.max_iterations, 20);
        assert_eq!(config.timeout_secs, 600);
        assert!(config.auto_register);
    }

    // --- HeartbeatConfig default tests ---

    #[test]
    fn heartbeat_config_defaults() {
        let config = HeartbeatConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.interval_secs, 1800);
        assert!(config.notify_channel.is_none());
        assert!(config.notify_user.is_none());
    }

    // --- SandboxModeConfig default tests ---

    #[test]
    fn sandbox_config_defaults() {
        let config = SandboxModeConfig::default();
        assert!(config.enabled);
        assert_eq!(config.policy, "readonly");
        assert_eq!(config.timeout_secs, 120);
        assert_eq!(config.memory_limit_mb, 2048);
        assert_eq!(config.cpu_shares, 1024);
        assert_eq!(config.image, "ghcr.io/nearai/sandbox:latest");
        assert!(config.auto_pull_image);
        assert!(config.extra_allowed_domains.is_empty());
    }

    // --- SafetyConfig from_env (no env vars set, uses defaults) ---

    #[test]
    fn safety_config_defaults_without_env() {
        let _lock = ENV_LOCK.lock();
        // Clear relevant env vars to ensure defaults are used.
        unsafe { std::env::remove_var("SAFETY_MAX_OUTPUT_LENGTH") };
        unsafe { std::env::remove_var("SAFETY_INJECTION_CHECK_ENABLED") };

        let config = SafetyConfig::from_env().unwrap();
        assert_eq!(config.max_output_length, 100_000);
        assert!(config.injection_check_enabled);
    }

    #[test]
    fn safety_config_respects_env_vars() {
        let _lock = ENV_LOCK.lock();
        unsafe { std::env::set_var("SAFETY_MAX_OUTPUT_LENGTH", "5000") };
        unsafe { std::env::set_var("SAFETY_INJECTION_CHECK_ENABLED", "false") };

        let config = SafetyConfig::from_env().unwrap();
        assert_eq!(config.max_output_length, 5000);
        assert!(!config.injection_check_enabled);

        unsafe { std::env::remove_var("SAFETY_MAX_OUTPUT_LENGTH") };
        unsafe { std::env::remove_var("SAFETY_INJECTION_CHECK_ENABLED") };
    }

    #[test]
    fn safety_config_invalid_bool_returns_error() {
        let _lock = ENV_LOCK.lock();
        unsafe { std::env::set_var("SAFETY_INJECTION_CHECK_ENABLED", "maybe") };

        let result = SafetyConfig::from_env();
        assert!(result.is_err());

        unsafe { std::env::remove_var("SAFETY_INJECTION_CHECK_ENABLED") };
    }

    // --- SecretsConfig debug does not leak key ---

    #[test]
    fn secrets_config_debug_hides_key() {
        let config = SecretsConfig {
            master_key: Some(SecretString::from("supersecretkey1234567890abcdefghij")),
            enabled: true,
            source: crate::settings::KeySource::Env,
        };
        let debug = format!("{:?}", config);
        assert!(!debug.contains("supersecretkey"));
        assert!(debug.contains("true")); // master_key shows as `true` (is_some)
    }

    // --- DatabaseConfig.url() accessor ---

    #[test]
    fn database_config_url_exposes_secret() {
        let config = DatabaseConfig {
            url: SecretString::from("postgres://user:pass@localhost/db"),
            pool_size: 5,
        };
        assert_eq!(config.url(), "postgres://user:pass@localhost/db");
    }

    // --- TunnelConfig default ---

    #[test]
    fn tunnel_config_default_is_disabled() {
        let config = TunnelConfig::default();
        assert!(!config.is_enabled());
        assert!(config.public_url.is_none());
    }

    // --- Helper: set env vars, run closure, clean up ---
    // NOTE: Callers must hold ENV_LOCK before calling these helpers.
    // The helpers do NOT acquire the lock themselves so they can be nested.

    fn with_env_vars<F, R>(vars: &[(&str, &str)], f: F) -> R
    where
        F: FnOnce() -> R,
    {
        for (key, val) in vars {
            unsafe { std::env::set_var(key, val) };
        }
        let result = f();
        for (key, _) in vars {
            unsafe { std::env::remove_var(key) };
        }
        result
    }

    fn without_env_vars<F, R>(keys: &[&str], f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let saved: Vec<(&str, Option<String>)> =
            keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
        for key in keys {
            unsafe { std::env::remove_var(key) };
        }
        let result = f();
        for (key, val) in &saved {
            match val {
                Some(v) => unsafe { std::env::set_var(key, v) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
        result
    }

    // --- TunnelConfig::from_env validation ---

    #[test]
    fn tunnel_from_env_rejects_http_url() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("TUNNEL_URL", "http://example.com")], || {
            let result = TunnelConfig::from_env();
            assert!(result.is_err());
            match result.unwrap_err() {
                ConfigError::InvalidValue { key, message } => {
                    assert_eq!(key, "TUNNEL_URL");
                    assert!(message.contains("https://"));
                }
                other => panic!("expected InvalidValue, got: {other:?}"),
            }
        });
    }

    #[test]
    fn tunnel_from_env_accepts_https_url() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("TUNNEL_URL", "https://my-tunnel.example.com")], || {
            let cfg = TunnelConfig::from_env().unwrap();
            assert_eq!(
                cfg.public_url,
                Some("https://my-tunnel.example.com".to_string())
            );
        });
    }

    // --- WasmConfig::from_env ---

    #[test]
    fn wasm_config_from_env_with_defaults() {
        let _lock = ENV_LOCK.lock();
        without_env_vars(
            &[
                "WASM_ENABLED",
                "WASM_TOOLS_DIR",
                "WASM_DEFAULT_MEMORY_LIMIT",
                "WASM_DEFAULT_TIMEOUT_SECS",
                "WASM_DEFAULT_FUEL_LIMIT",
                "WASM_CACHE_COMPILED",
                "WASM_CACHE_DIR",
            ],
            || {
                let cfg = WasmConfig::from_env().unwrap();
                assert!(cfg.enabled);
                assert_eq!(cfg.default_memory_limit, 10 * 1024 * 1024);
                assert_eq!(cfg.default_timeout_secs, 60);
                assert_eq!(cfg.default_fuel_limit, 10_000_000);
                assert!(cfg.cache_compiled);
                assert!(cfg.cache_dir.is_none());
            },
        );
    }

    #[test]
    fn wasm_config_from_env_custom_values() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("WASM_ENABLED", "false"),
                ("WASM_TOOLS_DIR", "/tmp/my-tools"),
                ("WASM_DEFAULT_MEMORY_LIMIT", "5242880"),
                ("WASM_DEFAULT_TIMEOUT_SECS", "30"),
                ("WASM_DEFAULT_FUEL_LIMIT", "5000000"),
                ("WASM_CACHE_COMPILED", "false"),
                ("WASM_CACHE_DIR", "/tmp/cache"),
            ],
            || {
                let cfg = WasmConfig::from_env().unwrap();
                assert!(!cfg.enabled);
                assert_eq!(cfg.tools_dir, PathBuf::from("/tmp/my-tools"));
                assert_eq!(cfg.default_memory_limit, 5_242_880);
                assert_eq!(cfg.default_timeout_secs, 30);
                assert_eq!(cfg.default_fuel_limit, 5_000_000);
                assert!(!cfg.cache_compiled);
                assert_eq!(cfg.cache_dir, Some(PathBuf::from("/tmp/cache")));
            },
        );
    }

    #[test]
    fn wasm_config_from_env_invalid_enabled() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("WASM_ENABLED", "maybe")], || {
            let result = WasmConfig::from_env();
            assert!(result.is_err());
        });
    }

    // --- BuilderModeConfig::from_env ---

    #[test]
    fn builder_config_from_env_with_defaults() {
        let _lock = ENV_LOCK.lock();
        without_env_vars(
            &[
                "BUILDER_ENABLED",
                "BUILDER_DIR",
                "BUILDER_MAX_ITERATIONS",
                "BUILDER_TIMEOUT_SECS",
                "BUILDER_AUTO_REGISTER",
            ],
            || {
                let cfg = BuilderModeConfig::from_env().unwrap();
                assert!(cfg.enabled);
                assert!(cfg.build_dir.is_none());
                assert_eq!(cfg.max_iterations, 20);
                assert_eq!(cfg.timeout_secs, 600);
                assert!(cfg.auto_register);
            },
        );
    }

    #[test]
    fn builder_config_from_env_custom_values() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("BUILDER_ENABLED", "false"),
                ("BUILDER_DIR", "/tmp/builds"),
                ("BUILDER_MAX_ITERATIONS", "10"),
                ("BUILDER_TIMEOUT_SECS", "300"),
                ("BUILDER_AUTO_REGISTER", "false"),
            ],
            || {
                let cfg = BuilderModeConfig::from_env().unwrap();
                assert!(!cfg.enabled);
                assert_eq!(cfg.build_dir, Some(PathBuf::from("/tmp/builds")));
                assert_eq!(cfg.max_iterations, 10);
                assert_eq!(cfg.timeout_secs, 300);
                assert!(!cfg.auto_register);
            },
        );
    }

    #[test]
    fn builder_config_from_env_invalid_max_iterations() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("BUILDER_MAX_ITERATIONS", "abc")], || {
            let result = BuilderModeConfig::from_env();
            assert!(result.is_err());
        });
    }

    // --- HeartbeatConfig::from_env ---

    #[test]
    fn heartbeat_config_from_env_custom_values() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("HEARTBEAT_ENABLED", "true"),
                ("HEARTBEAT_INTERVAL_SECS", "900"),
                ("HEARTBEAT_NOTIFY_CHANNEL", "telegram"),
                ("HEARTBEAT_NOTIFY_USER", "user123"),
            ],
            || {
                let cfg = HeartbeatConfig::from_env().unwrap();
                assert!(cfg.enabled);
                assert_eq!(cfg.interval_secs, 900);
                assert_eq!(cfg.notify_channel, Some("telegram".to_string()));
                assert_eq!(cfg.notify_user, Some("user123".to_string()));
            },
        );
    }

    #[test]
    fn heartbeat_config_from_env_invalid_interval() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("HEARTBEAT_INTERVAL_SECS", "not_a_number")], || {
            let result = HeartbeatConfig::from_env();
            assert!(result.is_err());
        });
    }

    #[test]
    fn heartbeat_config_from_env_invalid_enabled() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("HEARTBEAT_ENABLED", "invalid")], || {
            let result = HeartbeatConfig::from_env();
            assert!(result.is_err());
        });
    }

    // --- SandboxModeConfig::from_env ---

    #[test]
    fn sandbox_config_from_env_with_defaults() {
        let _lock = ENV_LOCK.lock();
        without_env_vars(
            &[
                "SANDBOX_ENABLED",
                "SANDBOX_POLICY",
                "SANDBOX_TIMEOUT_SECS",
                "SANDBOX_MEMORY_LIMIT_MB",
                "SANDBOX_CPU_SHARES",
                "SANDBOX_IMAGE",
                "SANDBOX_AUTO_PULL",
                "SANDBOX_EXTRA_DOMAINS",
            ],
            || {
                let cfg = SandboxModeConfig::from_env().unwrap();
                assert!(cfg.enabled);
                assert_eq!(cfg.policy, "readonly");
                assert_eq!(cfg.timeout_secs, 120);
                assert_eq!(cfg.memory_limit_mb, 2048);
                assert_eq!(cfg.cpu_shares, 1024);
                assert_eq!(cfg.image, "ghcr.io/nearai/sandbox:latest");
                assert!(cfg.auto_pull_image);
                assert!(cfg.extra_allowed_domains.is_empty());
            },
        );
    }

    #[test]
    fn sandbox_config_from_env_custom_values() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("SANDBOX_ENABLED", "false"),
                ("SANDBOX_POLICY", "full_access"),
                ("SANDBOX_TIMEOUT_SECS", "60"),
                ("SANDBOX_MEMORY_LIMIT_MB", "4096"),
                ("SANDBOX_CPU_SHARES", "512"),
                ("SANDBOX_IMAGE", "custom-image:v1"),
                ("SANDBOX_AUTO_PULL", "false"),
                ("SANDBOX_EXTRA_DOMAINS", "api.example.com, cdn.example.com"),
            ],
            || {
                let cfg = SandboxModeConfig::from_env().unwrap();
                assert!(!cfg.enabled);
                assert_eq!(cfg.policy, "full_access");
                assert_eq!(cfg.timeout_secs, 60);
                assert_eq!(cfg.memory_limit_mb, 4096);
                assert_eq!(cfg.cpu_shares, 512);
                assert_eq!(cfg.image, "custom-image:v1");
                assert!(!cfg.auto_pull_image);
                assert_eq!(
                    cfg.extra_allowed_domains,
                    vec!["api.example.com", "cdn.example.com"]
                );
            },
        );
    }

    #[test]
    fn sandbox_config_from_env_invalid_timeout() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("SANDBOX_TIMEOUT_SECS", "bad")], || {
            let result = SandboxModeConfig::from_env();
            assert!(result.is_err());
        });
    }

    #[test]
    fn sandbox_config_from_env_invalid_enabled() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("SANDBOX_ENABLED", "maybe")], || {
            let result = SandboxModeConfig::from_env();
            assert!(result.is_err());
        });
    }

    // --- ChannelsConfig::from_env ---

    #[test]
    fn channels_config_no_http_when_not_configured() {
        let _lock = ENV_LOCK.lock();
        without_env_vars(
            &[
                "HTTP_PORT",
                "HTTP_HOST",
                "HTTP_WEBHOOK_SECRET",
                "HTTP_USER_ID",
                "CLI_ENABLED",
                "GATEWAY_ENABLED",
                "WASM_CHANNELS_DIR",
                "WASM_CHANNELS_ENABLED",
            ],
            || {
                let cfg = ChannelsConfig::from_env().unwrap();
                assert!(cfg.http.is_none());
                assert!(cfg.cli.enabled);
                assert!(cfg.gateway.is_none());
            },
        );
    }

    #[test]
    fn channels_config_http_from_port() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("HTTP_PORT", "9090")], || {
            without_env_vars(&["HTTP_HOST", "HTTP_WEBHOOK_SECRET", "HTTP_USER_ID"], || {
                let cfg = ChannelsConfig::from_env().unwrap();
                let http = cfg.http.unwrap();
                assert_eq!(http.port, 9090);
                assert_eq!(http.host, "0.0.0.0");
                assert_eq!(http.user_id, "http");
                assert!(http.webhook_secret.is_none());
            });
        });
    }

    #[test]
    fn channels_config_http_invalid_port() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("HTTP_PORT", "not_a_port")], || {
            let result = ChannelsConfig::from_env();
            assert!(result.is_err());
        });
    }

    #[test]
    fn channels_config_cli_disabled_with_false() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("CLI_ENABLED", "false")], || {
            without_env_vars(&["HTTP_PORT", "HTTP_HOST", "GATEWAY_ENABLED"], || {
                let cfg = ChannelsConfig::from_env().unwrap();
                assert!(!cfg.cli.enabled);
            });
        });
    }

    #[test]
    fn channels_config_cli_disabled_with_zero() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("CLI_ENABLED", "0")], || {
            without_env_vars(&["HTTP_PORT", "HTTP_HOST", "GATEWAY_ENABLED"], || {
                let cfg = ChannelsConfig::from_env().unwrap();
                assert!(!cfg.cli.enabled);
            });
        });
    }

    #[test]
    fn channels_config_gateway_enabled() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("GATEWAY_ENABLED", "true"),
                ("GATEWAY_PORT", "4000"),
                ("GATEWAY_HOST", "0.0.0.0"),
                ("GATEWAY_AUTH_TOKEN", "my-secret"),
                ("GATEWAY_USER_ID", "admin"),
            ],
            || {
                without_env_vars(&["HTTP_PORT", "HTTP_HOST"], || {
                    let cfg = ChannelsConfig::from_env().unwrap();
                    let gw = cfg.gateway.unwrap();
                    assert_eq!(gw.port, 4000);
                    assert_eq!(gw.host, "0.0.0.0");
                    assert_eq!(gw.auth_token, Some("my-secret".to_string()));
                    assert_eq!(gw.user_id, "admin");
                });
            },
        );
    }

    #[test]
    fn channels_config_gateway_invalid_port() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[("GATEWAY_ENABLED", "true"), ("GATEWAY_PORT", "xyz")],
            || {
                let result = ChannelsConfig::from_env();
                assert!(result.is_err());
            },
        );
    }

    // --- LlmConfig::from_env ---

    #[test]
    fn llm_config_from_env_defaults() {
        let _lock = ENV_LOCK.lock();
        without_env_vars(
            &[
                "NEARAI_MODEL",
                "NEARAI_BASE_URL",
                "NEARAI_AUTH_URL",
                "NEARAI_SESSION_PATH",
                "NEARAI_API_MODE",
                "NEARAI_API_KEY",
            ],
            || {
                let cfg = LlmConfig::from_env().unwrap();
                assert_eq!(cfg.nearai.api_mode, NearAiApiMode::Responses);
                assert!(cfg.nearai.api_key.is_none());
                assert_eq!(cfg.nearai.base_url, "https://cloud-api.near.ai");
                assert_eq!(cfg.nearai.auth_base_url, "https://private.near.ai");
            },
        );
    }

    #[test]
    fn llm_config_api_key_infers_chat_completions() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("NEARAI_API_KEY", "sk-test-key-123")], || {
            without_env_vars(&["NEARAI_API_MODE"], || {
                let cfg = LlmConfig::from_env().unwrap();
                assert_eq!(cfg.nearai.api_mode, NearAiApiMode::ChatCompletions);
                assert!(cfg.nearai.api_key.is_some());
            });
        });
    }

    #[test]
    fn llm_config_explicit_mode_overrides_inference() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("NEARAI_API_KEY", "sk-test-key-123"),
                ("NEARAI_API_MODE", "responses"),
            ],
            || {
                let cfg = LlmConfig::from_env().unwrap();
                assert_eq!(cfg.nearai.api_mode, NearAiApiMode::Responses);
            },
        );
    }

    #[test]
    fn llm_config_invalid_api_mode() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("NEARAI_API_MODE", "invalid_mode")], || {
            let result = LlmConfig::from_env();
            assert!(result.is_err());
        });
    }

    #[test]
    fn llm_config_custom_urls() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("NEARAI_BASE_URL", "https://custom.api.ai"),
                ("NEARAI_AUTH_URL", "https://custom.auth.ai"),
                ("NEARAI_SESSION_PATH", "/tmp/session.json"),
            ],
            || {
                without_env_vars(&["NEARAI_API_KEY", "NEARAI_API_MODE"], || {
                    let cfg = LlmConfig::from_env().unwrap();
                    assert_eq!(cfg.nearai.base_url, "https://custom.api.ai");
                    assert_eq!(cfg.nearai.auth_base_url, "https://custom.auth.ai");
                    assert_eq!(
                        cfg.nearai.session_path,
                        PathBuf::from("/tmp/session.json")
                    );
                });
            },
        );
    }

    // --- EmbeddingsConfig::from_env ---

    #[test]
    fn embeddings_config_auto_enables_with_api_key() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("OPENAI_API_KEY", "sk-test")], || {
            without_env_vars(
                &["EMBEDDING_ENABLED", "EMBEDDING_PROVIDER", "EMBEDDING_MODEL"],
                || {
                    let cfg = EmbeddingsConfig::from_env().unwrap();
                    assert!(cfg.enabled);
                    assert!(cfg.openai_api_key.is_some());
                },
            );
        });
    }

    #[test]
    fn embeddings_config_explicit_disabled_overrides_api_key() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("OPENAI_API_KEY", "sk-test"),
                ("EMBEDDING_ENABLED", "false"),
            ],
            || {
                let cfg = EmbeddingsConfig::from_env().unwrap();
                assert!(!cfg.enabled);
            },
        );
    }

    #[test]
    fn embeddings_config_custom_provider_and_model() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("EMBEDDING_PROVIDER", "nearai"),
                ("EMBEDDING_MODEL", "text-embedding-3-large"),
            ],
            || {
                without_env_vars(&["OPENAI_API_KEY", "EMBEDDING_ENABLED"], || {
                    let cfg = EmbeddingsConfig::from_env().unwrap();
                    assert_eq!(cfg.provider, "nearai");
                    assert_eq!(cfg.model, "text-embedding-3-large");
                });
            },
        );
    }

    #[test]
    fn embeddings_config_invalid_enabled() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("EMBEDDING_ENABLED", "notbool")], || {
            let result = EmbeddingsConfig::from_env();
            assert!(result.is_err());
        });
    }

    // --- AgentConfig::from_env ---

    #[test]
    fn agent_config_from_env_custom_values() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("AGENT_NAME", "myagent"),
                ("AGENT_MAX_PARALLEL_JOBS", "10"),
                ("AGENT_JOB_TIMEOUT_SECS", "7200"),
                ("AGENT_STUCK_THRESHOLD_SECS", "600"),
                ("SELF_REPAIR_CHECK_INTERVAL_SECS", "120"),
                ("SELF_REPAIR_MAX_ATTEMPTS", "5"),
                ("AGENT_USE_PLANNING", "false"),
                ("SESSION_IDLE_TIMEOUT_SECS", "86400"),
            ],
            || {
                let cfg = AgentConfig::from_env().unwrap();
                assert_eq!(cfg.name, "myagent");
                assert_eq!(cfg.max_parallel_jobs, 10);
                assert_eq!(cfg.job_timeout, Duration::from_secs(7200));
                assert_eq!(cfg.stuck_threshold, Duration::from_secs(600));
                assert_eq!(cfg.repair_check_interval, Duration::from_secs(120));
                assert_eq!(cfg.max_repair_attempts, 5);
                assert!(!cfg.use_planning);
                assert_eq!(cfg.session_idle_timeout, Duration::from_secs(86400));
            },
        );
    }

    #[test]
    fn agent_config_invalid_max_parallel_jobs() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("AGENT_MAX_PARALLEL_JOBS", "abc")], || {
            let result = AgentConfig::from_env();
            assert!(result.is_err());
        });
    }

    #[test]
    fn agent_config_invalid_use_planning() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("AGENT_USE_PLANNING", "maybe")], || {
            let result = AgentConfig::from_env();
            assert!(result.is_err());
        });
    }

    // --- DatabaseConfig::from_env ---

    #[test]
    fn database_config_from_env_with_url() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("DATABASE_URL", "postgres://user:pass@localhost/testdb"),
                ("DATABASE_POOL_SIZE", "20"),
            ],
            || {
                let cfg = DatabaseConfig::from_env().unwrap();
                assert_eq!(cfg.url(), "postgres://user:pass@localhost/testdb");
                assert_eq!(cfg.pool_size, 20);
            },
        );
    }

    #[test]
    fn database_config_invalid_pool_size() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(
            &[
                ("DATABASE_URL", "postgres://localhost/test"),
                ("DATABASE_POOL_SIZE", "notanum"),
            ],
            || {
                let result = DatabaseConfig::from_env();
                assert!(result.is_err());
            },
        );
    }

    #[test]
    fn database_config_default_pool_size() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("DATABASE_URL", "postgres://localhost/test")], || {
            without_env_vars(&["DATABASE_POOL_SIZE"], || {
                let cfg = DatabaseConfig::from_env().unwrap();
                assert_eq!(cfg.pool_size, 10);
            });
        });
    }

    // --- SecretsConfig::from_env ---

    #[test]
    fn secrets_config_short_master_key_rejected() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("SECRETS_MASTER_KEY", "tooshort")], || {
            let result = SecretsConfig::from_env();
            assert!(result.is_err());
            match result.unwrap_err() {
                ConfigError::InvalidValue { key, message } => {
                    assert_eq!(key, "SECRETS_MASTER_KEY");
                    assert!(message.contains("32 bytes"));
                }
                other => panic!("expected InvalidValue, got: {other:?}"),
            }
        });
    }

    #[test]
    fn secrets_config_valid_master_key() {
        let _lock = ENV_LOCK.lock();
        let key = "a".repeat(64);
        with_env_vars(&[("SECRETS_MASTER_KEY", &key)], || {
            let cfg = SecretsConfig::from_env().unwrap();
            assert!(cfg.enabled);
            assert!(cfg.master_key.is_some());
            assert_eq!(cfg.source, crate::settings::KeySource::Env);
        });
    }

    #[test]
    fn secrets_config_disabled_when_no_key() {
        let _lock = ENV_LOCK.lock();
        without_env_vars(&["SECRETS_MASTER_KEY"], || {
            let cfg = SecretsConfig::from_env().unwrap();
            assert_eq!(cfg.enabled, cfg.master_key.is_some());
        });
    }

    #[test]
    fn secrets_config_master_key_accessor() {
        let cfg = SecretsConfig {
            master_key: Some(SecretString::from("a".repeat(64))),
            enabled: true,
            source: crate::settings::KeySource::Env,
        };
        assert!(cfg.master_key().is_some());

        let cfg_none = SecretsConfig::default();
        assert!(cfg_none.master_key().is_none());
    }

    // --- Default path helpers ---

    #[test]
    fn default_session_path_ends_with_expected() {
        let path = default_session_path();
        assert!(path.ends_with(".ironclaw/session.json"));
    }

    #[test]
    fn default_tools_dir_ends_with_expected() {
        let path = default_tools_dir();
        assert!(path.ends_with(".ironclaw/tools"));
    }

    #[test]
    fn default_channels_dir_ends_with_expected() {
        let path = default_channels_dir();
        assert!(path.ends_with(".ironclaw/channels"));
    }

    // --- Safety config invalid max output length ---

    #[test]
    fn safety_config_invalid_max_output_returns_error() {
        let _lock = ENV_LOCK.lock();
        with_env_vars(&[("SAFETY_MAX_OUTPUT_LENGTH", "not_a_number")], || {
            let result = SafetyConfig::from_env();
            assert!(result.is_err());
        });
    }
}

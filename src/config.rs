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
                hint: "Run 'ironclaw setup' or set DATABASE_URL environment variable".to_string(),
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
                                 Run 'ironclaw setup' to reconfigure."
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

fn optional_env(key: &str) -> Result<Option<String>, ConfigError> {
    match std::env::var(key) {
        Ok(val) if val.is_empty() => Ok(None),
        Ok(val) => Ok(Some(val)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(ConfigError::ParseError(format!(
            "failed to read {key}: {e}"
        ))),
    }
}

fn parse_optional_env<T>(key: &str, default: T) -> Result<T, ConfigError>
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

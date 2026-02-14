//! Configuration for IronClaw.
//!
//! Settings are loaded with priority: env var > database > default.
//! `DATABASE_URL` lives in `~/.ironclaw/.env` (loaded via dotenvy early
//! in startup). Everything else comes from env vars, the DB settings
//! table, or auto-detection.

use std::path::PathBuf;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use crate::error::ConfigError;
use crate::settings::Settings;

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
    pub routines: RoutineConfig,
    pub sandbox: SandboxModeConfig,
    pub claude_code: ClaudeCodeConfig,
}

impl Config {
    /// Load configuration from environment variables and the database.
    ///
    /// Priority: env var > DB settings > default.
    /// This is the primary way to load config after DB is connected.
    pub async fn from_db(
        store: &dyn crate::db::Database,
        user_id: &str,
    ) -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        crate::bootstrap::load_ironclaw_env();

        // Load all settings from DB into a Settings struct
        let db_settings = match store.get_all_settings(user_id).await {
            Ok(map) => Settings::from_db_map(&map),
            Err(e) => {
                tracing::warn!("Failed to load settings from DB, using defaults: {}", e);
                Settings::default()
            }
        };

        Self::build(&db_settings).await
    }

    /// Load configuration from environment variables only (no database).
    ///
    /// Used during early startup before the database is connected,
    /// and by CLI commands that don't have DB access.
    /// Falls back to legacy `settings.json` on disk if present.
    ///
    /// Loads both `./.env` (standard, higher priority) and `~/.ironclaw/.env`
    /// (lower priority) via dotenvy, which never overwrites existing vars.
    pub async fn from_env() -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        crate::bootstrap::load_ironclaw_env();
        let settings = Settings::load();
        Self::build(&settings).await
    }

    /// Build config from settings (shared by from_env and from_db).
    async fn build(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            database: DatabaseConfig::resolve()?,
            llm: LlmConfig::resolve(settings)?,
            embeddings: EmbeddingsConfig::resolve(settings)?,
            tunnel: TunnelConfig::resolve(settings)?,
            channels: ChannelsConfig::resolve(settings)?,
            agent: AgentConfig::resolve(settings)?,
            safety: SafetyConfig::resolve()?,
            wasm: WasmConfig::resolve()?,
            secrets: SecretsConfig::resolve().await?,
            builder: BuilderModeConfig::resolve()?,
            heartbeat: HeartbeatConfig::resolve(settings)?,
            routines: RoutineConfig::resolve()?,
            sandbox: SandboxModeConfig::resolve()?,
            claude_code: ClaudeCodeConfig::resolve()?,
        })
    }
}

/// Tunnel configuration for exposing the agent to the internet.
///
/// Used by channels and tools that need public webhook endpoints.
/// The tunnel URL is shared across all channels (Telegram, Slack, etc.).
#[derive(Debug, Clone, Default)]
pub struct TunnelConfig {
    /// Public URL from tunnel provider (e.g., "https://abc123.ngrok.io").
    pub public_url: Option<String>,
}

impl TunnelConfig {
    fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let public_url = optional_env("TUNNEL_URL")?
            .or_else(|| settings.tunnel.public_url.clone().filter(|s| !s.is_empty()));

        if let Some(ref url) = public_url
            && !url.starts_with("https://")
        {
            return Err(ConfigError::InvalidValue {
                key: "TUNNEL_URL".to_string(),
                message: "must start with https:// (webhooks require HTTPS)".to_string(),
            });
        }

        Ok(Self { public_url })
    }

    /// Check if a tunnel is configured.
    pub fn is_enabled(&self) -> bool {
        self.public_url.is_some()
    }

    /// Get the webhook URL for a given path.
    pub fn webhook_url(&self, path: &str) -> Option<String> {
        self.public_url.as_ref().map(|base| {
            let base = base.trim_end_matches('/');
            let path = path.trim_start_matches('/');
            format!("{}/{}", base, path)
        })
    }
}

/// Which database backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DatabaseBackend {
    /// PostgreSQL via deadpool-postgres (default).
    #[default]
    Postgres,
    /// libSQL/Turso embedded database.
    LibSql,
}

impl std::str::FromStr for DatabaseBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "postgres" | "postgresql" | "pg" => Ok(Self::Postgres),
            "libsql" | "turso" | "sqlite" => Ok(Self::LibSql),
            _ => Err(format!(
                "invalid database backend '{}', expected 'postgres' or 'libsql'",
                s
            )),
        }
    }
}

/// Database configuration.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// Which backend to use (default: Postgres).
    pub backend: DatabaseBackend,

    // -- PostgreSQL fields --
    pub url: SecretString,
    pub pool_size: usize,

    // -- libSQL fields --
    /// Path to local libSQL database file (default: ~/.ironclaw/ironclaw.db).
    pub libsql_path: Option<PathBuf>,
    /// Turso cloud URL for remote sync (optional).
    pub libsql_url: Option<String>,
    /// Turso auth token (required when libsql_url is set).
    pub libsql_auth_token: Option<SecretString>,
}

impl DatabaseConfig {
    fn resolve() -> Result<Self, ConfigError> {
        let backend: DatabaseBackend = if let Some(b) = optional_env("DATABASE_BACKEND")? {
            b.parse().map_err(|e| ConfigError::InvalidValue {
                key: "DATABASE_BACKEND".to_string(),
                message: e,
            })?
        } else {
            DatabaseBackend::default()
        };

        // PostgreSQL URL is required only when using the postgres backend.
        // For libsql backend, default to an empty placeholder.
        // DATABASE_URL is loaded from ~/.ironclaw/.env via dotenvy early in startup.
        let url = optional_env("DATABASE_URL")?
            .or_else(|| {
                if backend == DatabaseBackend::LibSql {
                    Some("unused://libsql".to_string())
                } else {
                    None
                }
            })
            .ok_or_else(|| ConfigError::MissingRequired {
                key: "database_url".to_string(),
                hint: "Run 'ironclaw onboard' or set DATABASE_URL environment variable".to_string(),
            })?;

        let pool_size = parse_optional_env("DATABASE_POOL_SIZE", 10)?;

        let libsql_path = optional_env("LIBSQL_PATH")?.map(PathBuf::from).or_else(|| {
            if backend == DatabaseBackend::LibSql {
                Some(default_libsql_path())
            } else {
                None
            }
        });

        let libsql_url = optional_env("LIBSQL_URL")?;
        let libsql_auth_token = optional_env("LIBSQL_AUTH_TOKEN")?.map(SecretString::from);

        if libsql_url.is_some() && libsql_auth_token.is_none() {
            return Err(ConfigError::MissingRequired {
                key: "LIBSQL_AUTH_TOKEN".to_string(),
                hint: "LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set".to_string(),
            });
        }

        Ok(Self {
            backend,
            url: SecretString::from(url),
            pool_size,
            libsql_path,
            libsql_url,
            libsql_auth_token,
        })
    }

    /// Get the database URL (exposes the secret).
    pub fn url(&self) -> &str {
        self.url.expose_secret()
    }
}

/// Default libSQL database path (~/.ironclaw/ironclaw.db).
pub fn default_libsql_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("ironclaw.db")
}

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
            _ => Err(format!(
                "invalid LLM backend '{}', expected one of: nearai, openai, anthropic, ollama, openai_compatible",
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
        }
    }
}

/// Configuration for direct OpenAI API access.
#[derive(Debug, Clone)]
pub struct OpenAiDirectConfig {
    pub api_key: SecretString,
    pub model: String,
}

/// Configuration for direct Anthropic API access.
#[derive(Debug, Clone)]
pub struct AnthropicDirectConfig {
    pub api_key: SecretString,
    pub model: String,
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
    /// Optional fallback model for failover (default: None).
    /// When set, a secondary provider is created with this model and wrapped
    /// in a `FailoverProvider` so transient errors on the primary model
    /// automatically fall through to the fallback.
    pub fallback_model: Option<String>,
    /// Maximum number of retries for transient errors (default: 3).
    /// With the default of 3, the provider makes up to 4 total attempts
    /// (1 initial + 3 retries) before giving up.
    pub max_retries: u32,
}

impl LlmConfig {
    fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        // Determine backend (default: NearAi)
        let backend: LlmBackend = if let Some(b) = optional_env("LLM_BACKEND")? {
            b.parse().map_err(|e| ConfigError::InvalidValue {
                key: "LLM_BACKEND".to_string(),
                message: e,
            })?
        } else {
            LlmBackend::NearAi
        };

        // Always resolve NEAR AI config (used as fallback and for embeddings)
        let nearai_api_key = optional_env("NEARAI_API_KEY")?.map(SecretString::from);

        let api_mode = if let Some(mode_str) = optional_env("NEARAI_API_MODE")? {
            mode_str.parse().map_err(|e| ConfigError::InvalidValue {
                key: "NEARAI_API_MODE".to_string(),
                message: e,
            })?
        } else if nearai_api_key.is_some() {
            NearAiApiMode::ChatCompletions
        } else {
            NearAiApiMode::Responses
        };

        let nearai = NearAiConfig {
            model: optional_env("NEARAI_MODEL")?
                .or_else(|| settings.selected_model.clone())
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
            api_key: nearai_api_key,
            fallback_model: optional_env("NEARAI_FALLBACK_MODEL")?,
            max_retries: parse_optional_env("NEARAI_MAX_RETRIES", 3)?,
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
            Some(OpenAiDirectConfig { api_key, model })
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
            Some(AnthropicDirectConfig { api_key, model })
        } else {
            None
        };

        let ollama = if backend == LlmBackend::Ollama {
            let base_url = optional_env("OLLAMA_BASE_URL")?
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            let model = optional_env("OLLAMA_MODEL")?.unwrap_or_else(|| "llama3".to_string());
            Some(OllamaConfig { base_url, model })
        } else {
            None
        };

        let openai_compatible = if backend == LlmBackend::OpenAiCompatible {
            let base_url =
                optional_env("LLM_BASE_URL")?.ok_or_else(|| ConfigError::MissingRequired {
                    key: "LLM_BASE_URL".to_string(),
                    hint: "Set LLM_BASE_URL when LLM_BACKEND=openai_compatible".to_string(),
                })?;
            let api_key = optional_env("LLM_API_KEY")?.map(SecretString::from);
            let model = optional_env("LLM_MODEL")?.unwrap_or_else(|| "default".to_string());
            Some(OpenAiCompatibleConfig {
                base_url,
                api_key,
                model,
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
    fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let openai_api_key = optional_env("OPENAI_API_KEY")?.map(SecretString::from);

        let provider = optional_env("EMBEDDING_PROVIDER")?
            .unwrap_or_else(|| settings.embeddings.provider.clone());

        let model =
            optional_env("EMBEDDING_MODEL")?.unwrap_or_else(|| settings.embeddings.model.clone());

        let enabled = optional_env("EMBEDDING_ENABLED")?
            .map(|s| s.parse())
            .transpose()
            .map_err(|e| ConfigError::InvalidValue {
                key: "EMBEDDING_ENABLED".to_string(),
                message: format!("must be 'true' or 'false': {e}"),
            })?
            .unwrap_or_else(|| settings.embeddings.enabled || openai_api_key.is_some());

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
    /// Telegram owner user ID. When set, the bot only responds to this user.
    pub telegram_owner_id: Option<i64>,
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
    fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
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
            .unwrap_or(true)
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
            telegram_owner_id: optional_env("TELEGRAM_OWNER_ID")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "TELEGRAM_OWNER_ID".to_string(),
                    message: format!("must be an integer: {e}"),
                })?
                .or(settings.channels.telegram_owner_id),
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
    /// Allow chat to use filesystem/shell tools directly (bypass sandbox).
    pub allow_local_tools: bool,
}

impl AgentConfig {
    fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
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
            allow_local_tools: optional_env("ALLOW_LOCAL_TOOLS")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "ALLOW_LOCAL_TOOLS".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(false),
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
    fn resolve() -> Result<Self, ConfigError> {
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
    /// Auto-detect secrets master key from env var, then OS keychain.
    ///
    /// Sequential probe: SECRETS_MASTER_KEY env var first, then OS keychain.
    /// No saved "source" needed; just try each source in order.
    async fn resolve() -> Result<Self, ConfigError> {
        use crate::settings::KeySource;

        let (master_key, source) = if let Some(env_key) = optional_env("SECRETS_MASTER_KEY")? {
            (Some(SecretString::from(env_key)), KeySource::Env)
        } else {
            // Probe the OS keychain; if a key is stored, use it
            match crate::secrets::keychain::get_master_key().await {
                Ok(key_bytes) => {
                    let key_hex: String = key_bytes.iter().map(|b| format!("{:02x}", b)).collect();
                    (Some(SecretString::from(key_hex)), KeySource::Keychain)
                }
                Err(_) => (None, KeySource::None),
            }
        };

        let enabled = master_key.is_some();

        if let Some(ref key) = master_key
            && key.expose_secret().len() < 32
        {
            return Err(ConfigError::InvalidValue {
                key: "SECRETS_MASTER_KEY".to_string(),
                message: "must be at least 32 bytes for AES-256-GCM".to_string(),
            });
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
    fn resolve() -> Result<Self, ConfigError> {
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
            enabled: true,
            build_dir: None,
            max_iterations: 20,
            timeout_secs: 600,
            auto_register: true,
        }
    }
}

impl BuilderModeConfig {
    fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: optional_env("BUILDER_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "BUILDER_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
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
    fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
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
                .or_else(|| settings.heartbeat.notify_channel.clone()),
            notify_user: optional_env("HEARTBEAT_NOTIFY_USER")?
                .or_else(|| settings.heartbeat.notify_user.clone()),
        })
    }
}

/// Routines configuration.
#[derive(Debug, Clone)]
pub struct RoutineConfig {
    /// Whether the routines system is enabled.
    pub enabled: bool,
    /// How often (seconds) to poll for cron routines that need firing.
    pub cron_check_interval_secs: u64,
    /// Max routines executing concurrently across all users.
    pub max_concurrent_routines: usize,
    /// Default cooldown between fires (seconds).
    pub default_cooldown_secs: u64,
    /// Max output tokens for lightweight routine LLM calls.
    pub max_lightweight_tokens: u32,
}

impl Default for RoutineConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cron_check_interval_secs: 15,
            max_concurrent_routines: 10,
            default_cooldown_secs: 300,
            max_lightweight_tokens: 4096,
        }
    }
}

impl RoutineConfig {
    fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: optional_env("ROUTINES_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "ROUTINES_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
            cron_check_interval_secs: parse_optional_env("ROUTINES_CRON_INTERVAL", 15)?,
            max_concurrent_routines: parse_optional_env("ROUTINES_MAX_CONCURRENT", 10)?,
            default_cooldown_secs: parse_optional_env("ROUTINES_DEFAULT_COOLDOWN", 300)?,
            max_lightweight_tokens: parse_optional_env("ROUTINES_MAX_TOKENS", 4096)?,
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
            enabled: true,
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
    fn resolve() -> Result<Self, ConfigError> {
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

/// Claude Code sandbox configuration.
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    /// Whether Claude Code sandbox mode is available.
    pub enabled: bool,
    /// Host directory containing Claude auth session (mounted read-only).
    pub config_dir: std::path::PathBuf,
    /// Claude model to use (e.g. "sonnet", "opus").
    pub model: String,
    /// Maximum agentic turns before stopping.
    pub max_turns: u32,
    /// Memory limit in MB for Claude Code containers (heavier than workers).
    pub memory_limit_mb: u64,
    /// Allowed tool patterns for Claude Code permission settings.
    ///
    /// Written to `/workspace/.claude/settings.json` before spawning the CLI.
    /// Provides defense-in-depth: only explicitly listed tools are auto-approved.
    /// Any new/unknown tools would require interactive approval (which times out
    /// in the non-interactive container, failing safely).
    ///
    /// Patterns follow Claude Code syntax: `"Bash(*)"`, `"Read"`, `"Edit(*)"`, etc.
    pub allowed_tools: Vec<String>,
}

/// Default allowed tools for Claude Code inside containers.
///
/// These cover all standard Claude Code tools needed for autonomous operation.
/// The Docker container provides the primary security boundary; this allowlist
/// provides defense-in-depth by preventing any future unknown tools from being
/// silently auto-approved.
fn default_claude_code_allowed_tools() -> Vec<String> {
    [
        "Bash(*)",
        "Read",
        "Edit(*)",
        "Glob",
        "Grep",
        "WebFetch(*)",
        "Task(*)",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            config_dir: dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".claude"),
            model: "sonnet".to_string(),
            max_turns: 50,
            memory_limit_mb: 4096,
            allowed_tools: default_claude_code_allowed_tools(),
        }
    }
}

impl ClaudeCodeConfig {
    /// Load from environment variables only (used inside containers where
    /// there is no database or full config).
    pub fn from_env() -> Self {
        match Self::resolve() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to resolve ClaudeCodeConfig: {e}, using defaults");
                Self::default()
            }
        }
    }

    fn resolve() -> Result<Self, ConfigError> {
        let defaults = Self::default();
        Ok(Self {
            enabled: optional_env("CLAUDE_CODE_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "CLAUDE_CODE_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(defaults.enabled),
            config_dir: optional_env("CLAUDE_CONFIG_DIR")?
                .map(std::path::PathBuf::from)
                .unwrap_or(defaults.config_dir),
            model: optional_env("CLAUDE_CODE_MODEL")?.unwrap_or(defaults.model),
            max_turns: parse_optional_env("CLAUDE_CODE_MAX_TURNS", defaults.max_turns)?,
            memory_limit_mb: parse_optional_env(
                "CLAUDE_CODE_MEMORY_LIMIT_MB",
                defaults.memory_limit_mb,
            )?,
            allowed_tools: optional_env("CLAUDE_CODE_ALLOWED_TOOLS")?
                .map(|s| {
                    s.split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or(defaults.allowed_tools),
        })
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

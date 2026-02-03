//! Configuration for the NEAR Agent.

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
    pub channels: ChannelsConfig,
    pub agent: AgentConfig,
    pub safety: SafetyConfig,
    pub wasm: WasmConfig,
    pub secrets: SecretsConfig,
    pub builder: BuilderModeConfig,
    pub heartbeat: HeartbeatConfig,
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
            channels: ChannelsConfig::from_env()?,
            agent: AgentConfig::from_env()?,
            safety: SafetyConfig::from_env()?,
            wasm: WasmConfig::from_env()?,
            secrets: SecretsConfig::from_env()?,
            builder: BuilderModeConfig::from_env()?,
            heartbeat: HeartbeatConfig::from_env()?,
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
        Ok(Self {
            url: SecretString::from(required_env("DATABASE_URL")?),
            pool_size: optional_env("DATABASE_POOL_SIZE")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "DATABASE_POOL_SIZE".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?
                .unwrap_or(10),
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

/// NEAR AI chat-api configuration.
#[derive(Debug, Clone)]
pub struct NearAiConfig {
    /// Model to use (e.g., "claude-3-5-sonnet-20241022", "gpt-4o")
    pub model: String,
    /// Base URL for the NEAR AI chat-api (default: https://api.near.ai)
    pub base_url: String,
    /// Base URL for auth/refresh endpoints (default: https://private.near.ai)
    pub auth_base_url: String,
    /// Path to session file (default: ~/.near-agent/session.json)
    pub session_path: PathBuf,
}

impl LlmConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            nearai: NearAiConfig {
                model: optional_env("NEARAI_MODEL")?
                    .unwrap_or_else(|| "claude-3-5-sonnet-20241022".to_string()),
                base_url: optional_env("NEARAI_BASE_URL")?
                    .unwrap_or_else(|| "https://api.near.ai".to_string()),
                auth_base_url: optional_env("NEARAI_AUTH_URL")?
                    .unwrap_or_else(|| "https://private.near.ai".to_string()),
                session_path: optional_env("NEARAI_SESSION_PATH")?
                    .map(PathBuf::from)
                    .unwrap_or_else(default_session_path),
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
        let openai_api_key = optional_env("OPENAI_API_KEY")?.map(SecretString::from);
        let provider = optional_env("EMBEDDING_PROVIDER")?.unwrap_or_else(|| "openai".to_string());

        // Auto-enable if we have an API key
        let enabled = optional_env("EMBEDDING_ENABLED")?
            .map(|s| s.parse())
            .transpose()
            .map_err(|e| ConfigError::InvalidValue {
                key: "EMBEDDING_ENABLED".to_string(),
                message: format!("must be 'true' or 'false': {e}"),
            })?
            .unwrap_or(openai_api_key.is_some());

        Ok(Self {
            enabled,
            provider,
            openai_api_key,
            model: optional_env("EMBEDDING_MODEL")?
                .unwrap_or_else(|| "text-embedding-3-small".to_string()),
        })
    }

    /// Get the OpenAI API key if configured.
    pub fn openai_api_key(&self) -> Option<&str> {
        self.openai_api_key.as_ref().map(|s| s.expose_secret())
    }
}

/// Get the default session file path (~/.near-agent/session.json).
fn default_session_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".near-agent")
        .join("session.json")
}

/// Channel configurations.
#[derive(Debug, Clone)]
pub struct ChannelsConfig {
    pub cli: CliConfig,
    pub http: Option<HttpConfig>,
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

        Ok(Self {
            cli: CliConfig { enabled: true },
            http,
        })
    }
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
        Ok(Self {
            name: optional_env("AGENT_NAME")?.unwrap_or_else(|| "near-agent".to_string()),
            max_parallel_jobs: parse_optional_env("AGENT_MAX_PARALLEL_JOBS", 5)?,
            job_timeout: Duration::from_secs(parse_optional_env("AGENT_JOB_TIMEOUT_SECS", 3600)?),
            stuck_threshold: Duration::from_secs(parse_optional_env(
                "AGENT_STUCK_THRESHOLD_SECS",
                300,
            )?),
            repair_check_interval: Duration::from_secs(parse_optional_env(
                "SELF_REPAIR_CHECK_INTERVAL_SECS",
                60,
            )?),
            max_repair_attempts: parse_optional_env("SELF_REPAIR_MAX_ATTEMPTS", 3)?,
            use_planning: optional_env("AGENT_USE_PLANNING")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "AGENT_USE_PLANNING".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true), // Default to planning enabled
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
    /// Directory containing installed WASM tools (default: ~/.near-agent/tools/).
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
    /// Master key for encrypting secrets (loaded from SECRETS_MASTER_KEY env var).
    /// Must be at least 32 bytes for AES-256-GCM.
    pub master_key: Option<SecretString>,
    /// Whether secrets management is enabled.
    pub enabled: bool,
}

impl std::fmt::Debug for SecretsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretsConfig")
            .field("master_key", &self.master_key.is_some())
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl SecretsConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let master_key = optional_env("SECRETS_MASTER_KEY")?.map(SecretString::from);
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

/// Get the default tools directory (~/.near-agent/tools/).
fn default_tools_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".near-agent")
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
            enabled: false,
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
                .unwrap_or(false),
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
        Ok(Self {
            enabled: optional_env("HEARTBEAT_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "HEARTBEAT_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(false),
            interval_secs: parse_optional_env("HEARTBEAT_INTERVAL_SECS", 1800)?,
            notify_channel: optional_env("HEARTBEAT_NOTIFY_CHANNEL")?,
            notify_user: optional_env("HEARTBEAT_NOTIFY_USER")?,
        })
    }
}

// Helper functions

fn required_env(key: &str) -> Result<String, ConfigError> {
    std::env::var(key).map_err(|_| ConfigError::MissingEnvVar(key.to_string()))
}

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

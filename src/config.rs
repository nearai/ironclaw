//! Configuration for the NEAR Agent.

use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use crate::error::ConfigError;

/// Main configuration for the agent.
#[derive(Debug, Clone)]
pub struct Config {
    pub database: DatabaseConfig,
    pub llm: LlmConfig,
    pub channels: ChannelsConfig,
    pub agent: AgentConfig,
    pub safety: SafetyConfig,
}

impl Config {
    /// Load configuration from environment variables.
    pub fn from_env() -> Result<Self, ConfigError> {
        // Load .env file if present (ignore errors if not found)
        let _ = dotenvy::dotenv();

        Ok(Self {
            database: DatabaseConfig::from_env()?,
            llm: LlmConfig::from_env()?,
            channels: ChannelsConfig::from_env()?,
            agent: AgentConfig::from_env()?,
            safety: SafetyConfig::from_env()?,
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

/// LLM provider configuration.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    pub openai: Option<OpenAiConfig>,
    pub anthropic: Option<AnthropicConfig>,
    pub nearai: Option<NearAiConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProvider {
    OpenAi,
    Anthropic,
    NearAi,
}

impl std::str::FromStr for LlmProvider {
    type Err = ConfigError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            "nearai" | "near-ai" | "near_ai" => Ok(Self::NearAi),
            _ => Err(ConfigError::InvalidValue {
                key: "LLM_PROVIDER".to_string(),
                message: format!(
                    "unknown provider: {s}, expected 'openai', 'anthropic', or 'nearai'"
                ),
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    pub api_key: SecretString,
    pub model: String,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub api_key: SecretString,
    pub model: String,
    pub base_url: Option<String>,
}

/// NEAR AI chat-api configuration.
#[derive(Debug, Clone)]
pub struct NearAiConfig {
    /// Session token for authentication (format: sess_xxx)
    pub session_token: SecretString,
    /// Model to use (e.g., "claude-3-5-sonnet-20241022", "gpt-4o")
    pub model: String,
    /// Base URL for the NEAR AI chat-api (default: https://api.near.ai)
    pub base_url: String,
}

impl LlmConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let provider: LlmProvider = optional_env("LLM_PROVIDER")?
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(LlmProvider::NearAi);

        let openai = if let Some(api_key) = optional_env("OPENAI_API_KEY")? {
            Some(OpenAiConfig {
                api_key: SecretString::from(api_key),
                model: optional_env("OPENAI_MODEL")?.unwrap_or_else(|| "gpt-4-turbo".to_string()),
                base_url: optional_env("OPENAI_BASE_URL")?,
            })
        } else {
            None
        };

        let anthropic = if let Some(api_key) = optional_env("ANTHROPIC_API_KEY")? {
            Some(AnthropicConfig {
                api_key: SecretString::from(api_key),
                model: optional_env("ANTHROPIC_MODEL")?
                    .unwrap_or_else(|| "claude-3-opus-20240229".to_string()),
                base_url: optional_env("ANTHROPIC_BASE_URL")?,
            })
        } else {
            None
        };

        let nearai = if let Some(session_token) = optional_env("NEARAI_SESSION_TOKEN")? {
            Some(NearAiConfig {
                session_token: SecretString::from(session_token),
                model: optional_env("NEARAI_MODEL")?
                    .unwrap_or_else(|| "claude-3-5-sonnet-20241022".to_string()),
                base_url: optional_env("NEARAI_BASE_URL")?
                    .unwrap_or_else(|| "https://api.near.ai".to_string()),
            })
        } else {
            None
        };

        // Validate that the selected provider has configuration
        match provider {
            LlmProvider::OpenAi if openai.is_none() => {
                return Err(ConfigError::MissingEnvVar("OPENAI_API_KEY".to_string()));
            }
            LlmProvider::Anthropic if anthropic.is_none() => {
                return Err(ConfigError::MissingEnvVar("ANTHROPIC_API_KEY".to_string()));
            }
            LlmProvider::NearAi if nearai.is_none() => {
                return Err(ConfigError::MissingEnvVar(
                    "NEARAI_SESSION_TOKEN".to_string(),
                ));
            }
            // Provider has valid configuration
            LlmProvider::OpenAi | LlmProvider::Anthropic | LlmProvider::NearAi => {}
        }

        Ok(Self {
            provider,
            openai,
            anthropic,
            nearai,
        })
    }
}

/// Channel configurations.
#[derive(Debug, Clone)]
pub struct ChannelsConfig {
    pub cli: CliConfig,
    pub slack: Option<SlackConfig>,
    pub telegram: Option<TelegramConfig>,
    pub http: Option<HttpConfig>,
}

#[derive(Debug, Clone)]
pub struct CliConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct SlackConfig {
    pub bot_token: SecretString,
    pub app_token: SecretString,
    pub signing_secret: SecretString,
}

#[derive(Debug, Clone)]
pub struct TelegramConfig {
    pub bot_token: SecretString,
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
    pub webhook_secret: Option<SecretString>,
}

impl ChannelsConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let slack = match (
            optional_env("SLACK_BOT_TOKEN")?,
            optional_env("SLACK_APP_TOKEN")?,
            optional_env("SLACK_SIGNING_SECRET")?,
        ) {
            (Some(bot_token), Some(app_token), Some(signing_secret)) => Some(SlackConfig {
                bot_token: SecretString::from(bot_token),
                app_token: SecretString::from(app_token),
                signing_secret: SecretString::from(signing_secret),
            }),
            (None, None, None) => None,
            _ => {
                return Err(ConfigError::InvalidValue {
                    key: "SLACK_*".to_string(),
                    message: "all Slack environment variables must be set together".to_string(),
                });
            }
        };

        let telegram = optional_env("TELEGRAM_BOT_TOKEN")?.map(|token| TelegramConfig {
            bot_token: SecretString::from(token),
        });

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
            slack,
            telegram,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_provider_parsing() {
        assert_eq!(
            "openai".parse::<LlmProvider>().unwrap(),
            LlmProvider::OpenAi
        );
        assert_eq!(
            "anthropic".parse::<LlmProvider>().unwrap(),
            LlmProvider::Anthropic
        );
        assert_eq!(
            "OpenAI".parse::<LlmProvider>().unwrap(),
            LlmProvider::OpenAi
        );
        assert_eq!(
            "nearai".parse::<LlmProvider>().unwrap(),
            LlmProvider::NearAi
        );
        assert_eq!(
            "near-ai".parse::<LlmProvider>().unwrap(),
            LlmProvider::NearAi
        );
        assert_eq!(
            "near_ai".parse::<LlmProvider>().unwrap(),
            LlmProvider::NearAi
        );
        assert!("invalid".parse::<LlmProvider>().is_err());
    }
}

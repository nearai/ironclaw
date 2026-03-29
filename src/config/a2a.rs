use std::time::Duration;

use crate::config::helpers::{parse_bool_env, parse_optional_env, parse_string_env};
use crate::error::ConfigError;

/// Configuration for the A2A (Agent-to-Agent) protocol bridge.
///
/// Connects to a remote agent via the Google A2A protocol (JSON-RPC 2.0 + SSE
/// streaming). All agent-specific values (URL, assistant ID) must be set
/// explicitly — no hardcoded defaults.
#[derive(Debug, Clone)]
pub struct A2aConfig {
    /// Whether the A2A bridge is enabled.
    pub enabled: bool,
    /// Base URL of the remote agent (required when enabled).
    pub agent_url: String,
    /// Assistant ID for the remote agent (required when enabled).
    pub assistant_id: String,
    /// Tool name exposed to the LLM (default: `"a2a_query"`).
    pub tool_name: String,
    /// Tool description exposed to the LLM.
    pub tool_description: String,
    /// Prefix for push-notification messages from the background SSE consumer.
    pub message_prefix: String,
    /// Timeout for reading the first SSE event after connection.
    pub request_timeout: Duration,
    /// Timeout for the entire background SSE stream consumption.
    pub task_timeout: Duration,
    /// Secret name in the secrets store for the API key.
    pub api_key_secret: String,
}

impl A2aConfig {
    pub(crate) fn resolve() -> Result<Option<Self>, ConfigError> {
        let enabled = parse_bool_env("A2A_ENABLED", false)?;
        if !enabled {
            return Ok(None);
        }

        let agent_url = parse_string_env("A2A_AGENT_URL", "")?;
        if agent_url.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "A2A_AGENT_URL".to_string(),
                message: "must be set when A2A_ENABLED=true".to_string(),
            });
        }

        let assistant_id = parse_string_env("A2A_ASSISTANT_ID", "")?;
        if assistant_id.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "A2A_ASSISTANT_ID".to_string(),
                message: "must be set when A2A_ENABLED=true".to_string(),
            });
        }

        let tool_name = parse_string_env("A2A_TOOL_NAME", "a2a_query")?;
        let tool_description = parse_string_env(
            "A2A_TOOL_DESCRIPTION",
            "Query a remote AI agent via the A2A (Agent-to-Agent) protocol. \
             Supports multi-turn conversations with thread_id for context continuity.",
        )?;
        let message_prefix = parse_string_env("A2A_MESSAGE_PREFIX", "[a2a]")?;
        let request_timeout_ms: u64 = parse_optional_env("A2A_REQUEST_TIMEOUT_MS", 60_000)?;
        let task_timeout_ms: u64 = parse_optional_env("A2A_TASK_TIMEOUT_MS", 1_200_000)?;
        let api_key_secret = parse_string_env("A2A_API_KEY_SECRET", "a2a_api_key")?;

        Ok(Some(Self {
            enabled,
            agent_url,
            assistant_id,
            tool_name,
            tool_description,
            message_prefix,
            request_timeout: Duration::from_millis(request_timeout_ms),
            task_timeout: Duration::from_millis(task_timeout_ms),
            api_key_secret,
        }))
    }

    /// Whether the API key secret name is configured (non-empty).
    pub fn has_api_key_configured(&self) -> bool {
        !self.api_key_secret.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        let _guard = crate::config::helpers::ENV_MUTEX.lock();
        unsafe {
            std::env::remove_var("A2A_ENABLED");
        }
        let result = A2aConfig::resolve().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn requires_agent_url_when_enabled() {
        let _guard = crate::config::helpers::ENV_MUTEX.lock();
        unsafe {
            std::env::set_var("A2A_ENABLED", "true");
            std::env::remove_var("A2A_AGENT_URL");
        }
        let result = A2aConfig::resolve();
        assert!(result.is_err());
        unsafe {
            std::env::remove_var("A2A_ENABLED");
        }
    }

    #[test]
    fn has_api_key_configured_checks_non_empty() {
        let config = A2aConfig {
            enabled: true,
            agent_url: "https://example.com".to_string(),
            assistant_id: "test-id".to_string(),
            tool_name: "a2a_query".to_string(),
            tool_description: "test".to_string(),
            message_prefix: "[a2a]".to_string(),
            request_timeout: Duration::from_secs(60),
            task_timeout: Duration::from_secs(1200),
            api_key_secret: "my_key".to_string(),
        };
        assert!(config.has_api_key_configured());

        let config_empty = A2aConfig {
            api_key_secret: String::new(),
            ..config
        };
        assert!(!config_empty.has_api_key_configured());
    }
}

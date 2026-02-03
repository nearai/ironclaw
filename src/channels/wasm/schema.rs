//! JSON schema for WASM channel capabilities files.
//!
//! External WASM channels declare their required capabilities via a sidecar JSON file
//! (e.g., `slack.capabilities.json`). This module defines the schema for those files
//! and provides conversion to runtime [`ChannelCapabilities`].
//!
//! # Example Capabilities File
//!
//! ```json
//! {
//!   "type": "channel",
//!   "name": "slack",
//!   "description": "Slack Events API channel",
//!   "capabilities": {
//!     "http": {
//!       "allowlist": [
//!         { "host": "slack.com", "path_prefix": "/api/" }
//!       ],
//!       "credentials": {
//!         "slack_bot": {
//!           "secret_name": "slack_bot_token",
//!           "location": { "type": "bearer" },
//!           "host_patterns": ["slack.com"]
//!         }
//!       }
//!     },
//!     "secrets": { "allowed_names": ["slack_*"] },
//!     "channel": {
//!       "allowed_paths": ["/webhook/slack"],
//!       "allow_polling": false,
//!       "workspace_prefix": "channels/slack/",
//!       "emit_rate_limit": { "messages_per_minute": 100 }
//!     }
//!   },
//!   "config": {
//!     "signing_secret_name": "slack_signing_secret"
//!   }
//! }
//! ```

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::channels::wasm::capabilities::{
    ChannelCapabilities, EmitRateLimitConfig, MIN_POLL_INTERVAL_MS,
};
use crate::tools::wasm::{CapabilitiesFile as ToolCapabilitiesFile, RateLimitSchema};

/// Root schema for a channel capabilities JSON file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelCapabilitiesFile {
    /// File type, must be "channel".
    #[serde(default = "default_type")]
    pub r#type: String,

    /// Channel name.
    pub name: String,

    /// Channel description.
    #[serde(default)]
    pub description: Option<String>,

    /// Capabilities (tool + channel specific).
    #[serde(default)]
    pub capabilities: ChannelCapabilitiesSchema,

    /// Channel-specific configuration passed to on_start.
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

fn default_type() -> String {
    "channel".to_string()
}

impl ChannelCapabilitiesFile {
    /// Parse from JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Parse from JSON bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// Convert to runtime ChannelCapabilities.
    pub fn to_capabilities(&self) -> ChannelCapabilities {
        self.capabilities.to_channel_capabilities(&self.name)
    }

    /// Get the channel config as JSON string.
    pub fn config_json(&self) -> String {
        serde_json::to_string(&self.config).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Schema for channel capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelCapabilitiesSchema {
    /// Tool capabilities (HTTP, secrets, workspace_read).
    #[serde(flatten)]
    pub tool: Option<ToolCapabilitiesFile>,

    /// Channel-specific capabilities.
    #[serde(default)]
    pub channel: Option<ChannelSpecificCapabilitiesSchema>,
}

impl ChannelCapabilitiesSchema {
    /// Convert to runtime ChannelCapabilities.
    pub fn to_channel_capabilities(&self, channel_name: &str) -> ChannelCapabilities {
        let tool_caps = self
            .tool
            .as_ref()
            .map(|t| t.to_capabilities())
            .unwrap_or_default();

        let mut caps =
            ChannelCapabilities::for_channel(channel_name).with_tool_capabilities(tool_caps);

        if let Some(channel) = &self.channel {
            caps.allowed_paths = channel.allowed_paths.clone();
            caps.allow_polling = channel.allow_polling;
            caps.min_poll_interval_ms = channel
                .min_poll_interval_ms
                .unwrap_or(MIN_POLL_INTERVAL_MS)
                .max(MIN_POLL_INTERVAL_MS);

            if let Some(prefix) = &channel.workspace_prefix {
                caps.workspace_prefix = prefix.clone();
            }

            if let Some(rate) = &channel.emit_rate_limit {
                caps.emit_rate_limit = rate.to_emit_rate_limit();
            }

            if let Some(max_size) = channel.max_message_size {
                caps.max_message_size = max_size;
            }

            if let Some(timeout_secs) = channel.callback_timeout_secs {
                caps.callback_timeout = Duration::from_secs(timeout_secs);
            }
        }

        caps
    }
}

/// Channel-specific capabilities schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelSpecificCapabilitiesSchema {
    /// HTTP paths the channel can register for webhooks.
    #[serde(default)]
    pub allowed_paths: Vec<String>,

    /// Whether polling is allowed.
    #[serde(default)]
    pub allow_polling: bool,

    /// Minimum poll interval in milliseconds.
    #[serde(default)]
    pub min_poll_interval_ms: Option<u32>,

    /// Workspace prefix for storage (overrides default).
    #[serde(default)]
    pub workspace_prefix: Option<String>,

    /// Rate limiting for emit_message.
    #[serde(default)]
    pub emit_rate_limit: Option<EmitRateLimitSchema>,

    /// Maximum message content size in bytes.
    #[serde(default)]
    pub max_message_size: Option<usize>,

    /// Callback timeout in seconds.
    #[serde(default)]
    pub callback_timeout_secs: Option<u64>,
}

/// Schema for emit rate limiting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmitRateLimitSchema {
    /// Maximum messages per minute.
    #[serde(default = "default_messages_per_minute")]
    pub messages_per_minute: u32,

    /// Maximum messages per hour.
    #[serde(default = "default_messages_per_hour")]
    pub messages_per_hour: u32,
}

fn default_messages_per_minute() -> u32 {
    100
}

fn default_messages_per_hour() -> u32 {
    5000
}

impl EmitRateLimitSchema {
    fn to_emit_rate_limit(&self) -> EmitRateLimitConfig {
        EmitRateLimitConfig {
            messages_per_minute: self.messages_per_minute,
            messages_per_hour: self.messages_per_hour,
        }
    }
}

impl From<RateLimitSchema> for EmitRateLimitSchema {
    fn from(schema: RateLimitSchema) -> Self {
        Self {
            messages_per_minute: schema.requests_per_minute,
            messages_per_hour: schema.requests_per_hour,
        }
    }
}

/// Channel configuration returned by on_start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Display name for the channel.
    pub display_name: String,

    /// HTTP endpoints to register.
    #[serde(default)]
    pub http_endpoints: Vec<HttpEndpointConfigSchema>,

    /// Polling configuration.
    #[serde(default)]
    pub poll: Option<PollConfigSchema>,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            display_name: "WASM Channel".to_string(),
            http_endpoints: Vec::new(),
            poll: None,
        }
    }
}

/// HTTP endpoint configuration schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpEndpointConfigSchema {
    /// Path to register.
    pub path: String,

    /// HTTP methods to accept.
    #[serde(default)]
    pub methods: Vec<String>,

    /// Whether secret validation is required.
    #[serde(default)]
    pub require_secret: bool,
}

/// Polling configuration schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollConfigSchema {
    /// Polling interval in milliseconds.
    pub interval_ms: u32,

    /// Whether polling is enabled.
    #[serde(default)]
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use crate::channels::wasm::schema::ChannelCapabilitiesFile;

    #[test]
    fn test_parse_minimal() {
        let json = r#"{
            "name": "test"
        }"#;
        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(file.name, "test");
        assert_eq!(file.r#type, "channel");
    }

    #[test]
    fn test_parse_full_slack_example() {
        let json = r#"{
            "type": "channel",
            "name": "slack",
            "description": "Slack Events API channel",
            "capabilities": {
                "http": {
                    "allowlist": [
                        { "host": "slack.com", "path_prefix": "/api/" }
                    ],
                    "credentials": {
                        "slack_bot": {
                            "secret_name": "slack_bot_token",
                            "location": { "type": "bearer" },
                            "host_patterns": ["slack.com"]
                        }
                    },
                    "rate_limit": { "requests_per_minute": 50, "requests_per_hour": 1000 }
                },
                "secrets": { "allowed_names": ["slack_*"] },
                "channel": {
                    "allowed_paths": ["/webhook/slack"],
                    "allow_polling": false,
                    "emit_rate_limit": { "messages_per_minute": 100, "messages_per_hour": 5000 }
                }
            },
            "config": {
                "signing_secret_name": "slack_signing_secret"
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(file.name, "slack");
        assert_eq!(
            file.description,
            Some("Slack Events API channel".to_string())
        );

        let caps = file.to_capabilities();
        assert!(caps.is_path_allowed("/webhook/slack"));
        assert!(!caps.allow_polling);
        assert_eq!(caps.workspace_prefix, "channels/slack/");

        // Check tool capabilities were parsed
        assert!(caps.tool_capabilities.http.is_some());
        assert!(caps.tool_capabilities.secrets.is_some());

        // Check config
        let config_json = file.config_json();
        assert!(config_json.contains("signing_secret_name"));
    }

    #[test]
    fn test_parse_with_polling() {
        let json = r#"{
            "name": "telegram",
            "capabilities": {
                "channel": {
                    "allowed_paths": [],
                    "allow_polling": true,
                    "min_poll_interval_ms": 60000
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        assert!(caps.allow_polling);
        assert_eq!(caps.min_poll_interval_ms, 60000);
    }

    #[test]
    fn test_min_poll_interval_enforced() {
        let json = r#"{
            "name": "test",
            "capabilities": {
                "channel": {
                    "allow_polling": true,
                    "min_poll_interval_ms": 1000
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        // Should be clamped to minimum
        assert_eq!(caps.min_poll_interval_ms, 30000);
    }

    #[test]
    fn test_workspace_prefix_override() {
        let json = r#"{
            "name": "custom",
            "capabilities": {
                "channel": {
                    "workspace_prefix": "integrations/custom/"
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        assert_eq!(caps.workspace_prefix, "integrations/custom/");
    }

    #[test]
    fn test_emit_rate_limit() {
        let json = r#"{
            "name": "test",
            "capabilities": {
                "channel": {
                    "emit_rate_limit": {
                        "messages_per_minute": 50,
                        "messages_per_hour": 1000
                    }
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        assert_eq!(caps.emit_rate_limit.messages_per_minute, 50);
        assert_eq!(caps.emit_rate_limit.messages_per_hour, 1000);
    }
}

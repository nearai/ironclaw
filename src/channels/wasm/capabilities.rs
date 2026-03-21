//! Channel-specific capabilities for WASM channels.
//!
//! Defines the capability system that controls what a WASM channel can do.
//! Channels have additional capabilities beyond tools: HTTP endpoint registration,
//! message emission, and workspace write access within their namespace.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::tools::wasm::{Capabilities as ToolCapabilities, RateLimitConfig};

/// Minimum allowed polling interval (30 seconds).
pub const MIN_POLL_INTERVAL_MS: u32 = 30_000;

/// Default emit rate limit.
pub const DEFAULT_EMIT_RATE_PER_MINUTE: u32 = 100;
pub const DEFAULT_EMIT_RATE_PER_HOUR: u32 = 5000;

/// Capabilities specific to WASM channels.
///
/// Extends tool capabilities with channel-specific permissions.
#[derive(Debug, Clone)]
pub struct ChannelCapabilities {
    /// Base tool capabilities (HTTP, secrets, workspace_read, etc.).
    pub tool_capabilities: ToolCapabilities,

    /// HTTP paths this channel can register for webhooks.
    /// Paths must start with "/webhook/" by convention.
    pub allowed_paths: Vec<String>,

    /// Whether polling is allowed for this channel.
    pub allow_polling: bool,

    /// Minimum poll interval in milliseconds.
    /// Enforced to be at least MIN_POLL_INTERVAL_MS.
    pub min_poll_interval_ms: u32,

    /// Workspace prefix for this channel's storage.
    /// All workspace writes are automatically prefixed.
    /// Example: "channels/slack/" means writes to "state.json" become "channels/slack/state.json".
    pub workspace_prefix: String,

    /// Rate limiting for emit_message calls.
    pub emit_rate_limit: EmitRateLimitConfig,

    /// Maximum message content size in bytes.
    pub max_message_size: usize,

    /// Callback timeout duration.
    pub callback_timeout: Duration,

    /// Socket Mode configuration (WebSocket-based event delivery).
    pub socket_mode: Option<SocketModeConfig>,
}

impl Default for ChannelCapabilities {
    fn default() -> Self {
        Self {
            tool_capabilities: ToolCapabilities::default(),
            allowed_paths: Vec::new(),
            allow_polling: false,
            min_poll_interval_ms: MIN_POLL_INTERVAL_MS,
            workspace_prefix: String::new(),
            emit_rate_limit: EmitRateLimitConfig::default(),
            max_message_size: 64 * 1024, // 64 KB
            callback_timeout: Duration::from_secs(30),
            socket_mode: None,
        }
    }
}

impl ChannelCapabilities {
    /// Create capabilities for a channel with the given name.
    pub fn for_channel(name: &str) -> Self {
        Self {
            workspace_prefix: format!("channels/{}/", name),
            ..Default::default()
        }
    }

    /// Add an allowed HTTP path.
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.allowed_paths.push(path.into());
        self
    }

    /// Enable polling with the given minimum interval.
    pub fn with_polling(mut self, min_interval_ms: u32) -> Self {
        self.allow_polling = true;
        self.min_poll_interval_ms = min_interval_ms.max(MIN_POLL_INTERVAL_MS);
        self
    }

    /// Set the emit rate limit.
    pub fn with_emit_rate_limit(mut self, rate_limit: EmitRateLimitConfig) -> Self {
        self.emit_rate_limit = rate_limit;
        self
    }

    /// Set the callback timeout.
    pub fn with_callback_timeout(mut self, timeout: Duration) -> Self {
        self.callback_timeout = timeout;
        self
    }

    /// Set the base tool capabilities.
    pub fn with_tool_capabilities(mut self, capabilities: ToolCapabilities) -> Self {
        self.tool_capabilities = capabilities;
        self
    }

    /// Check if a path is allowed for this channel.
    pub fn is_path_allowed(&self, path: &str) -> bool {
        self.allowed_paths.iter().any(|p| p == path)
    }

    /// Validate and normalize a poll interval.
    ///
    /// Returns the interval clamped to minimum, or an error if polling is disabled.
    pub fn validate_poll_interval(&self, interval_ms: u32) -> Result<u32, String> {
        if !self.allow_polling {
            return Err("Polling not allowed for this channel".to_string());
        }

        Ok(interval_ms.max(self.min_poll_interval_ms))
    }

    /// Prefix a workspace path for this channel.
    ///
    /// Ensures all workspace writes are scoped to the channel's namespace.
    pub fn prefix_workspace_path(&self, path: &str) -> String {
        if self.workspace_prefix.is_empty() {
            path.to_string()
        } else {
            format!("{}{}", self.workspace_prefix, path)
        }
    }

    /// Check if a workspace path is valid for this channel.
    ///
    /// Paths cannot escape the channel's namespace.
    pub fn validate_workspace_path(&self, path: &str) -> Result<String, String> {
        // Block absolute paths
        if path.starts_with('/') {
            return Err("Absolute paths not allowed".to_string());
        }

        // Block path traversal
        if path.contains("..") {
            return Err("Parent directory references not allowed".to_string());
        }

        // Block null bytes
        if path.contains('\0') {
            return Err("Null bytes not allowed".to_string());
        }

        // Prefix with channel namespace
        Ok(self.prefix_workspace_path(path))
    }
}

/// Runtime configuration for Socket Mode (WebSocket-based event delivery).
///
/// When present, the host manages a persistent WebSocket connection to the
/// messaging platform and bridges events into the WASM `on_http_request` callback.
#[derive(Debug, Clone)]
pub struct SocketModeConfig {
    /// URL for obtaining a WebSocket connection URL (e.g., `apps.connections.open`).
    pub open_url: String,

    /// Secret name for the app-level token used to authenticate the connection.
    pub app_token_secret: String,

    /// Base reconnect delay in milliseconds.
    pub reconnect_delay_ms: u64,

    /// Maximum number of reconnection attempts before giving up.
    pub max_reconnect_attempts: u32,
}

/// Configuration for an HTTP endpoint the channel wants to register.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpEndpointConfig {
    /// Path to register (e.g., "/webhook/slack").
    pub path: String,

    /// HTTP methods to accept (e.g., ["POST"]).
    pub methods: Vec<String>,

    /// Whether secret validation is required.
    pub require_secret: bool,
}

impl HttpEndpointConfig {
    /// Create a POST webhook endpoint.
    pub fn post_webhook(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            methods: vec!["POST".to_string()],
            require_secret: true,
        }
    }
}

/// Polling configuration returned by the channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollConfig {
    /// Polling interval in milliseconds.
    pub interval_ms: u32,

    /// Whether polling is enabled.
    pub enabled: bool,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            interval_ms: MIN_POLL_INTERVAL_MS,
            enabled: false,
        }
    }
}

/// Rate limiting configuration for message emission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmitRateLimitConfig {
    /// Maximum messages per minute.
    pub messages_per_minute: u32,

    /// Maximum messages per hour.
    pub messages_per_hour: u32,
}

impl Default for EmitRateLimitConfig {
    fn default() -> Self {
        Self {
            messages_per_minute: DEFAULT_EMIT_RATE_PER_MINUTE,
            messages_per_hour: DEFAULT_EMIT_RATE_PER_HOUR,
        }
    }
}

impl From<RateLimitConfig> for EmitRateLimitConfig {
    fn from(config: RateLimitConfig) -> Self {
        Self {
            messages_per_minute: config.requests_per_minute,
            messages_per_hour: config.requests_per_hour,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::channels::wasm::capabilities::{
        ChannelCapabilities, EmitRateLimitConfig, HttpEndpointConfig, MIN_POLL_INTERVAL_MS,
    };

    #[test]
    fn test_default_capabilities() {
        let caps = ChannelCapabilities::default();
        assert!(caps.allowed_paths.is_empty());
        assert!(!caps.allow_polling);
        assert_eq!(caps.min_poll_interval_ms, MIN_POLL_INTERVAL_MS);
    }

    #[test]
    fn test_for_channel() {
        let caps = ChannelCapabilities::for_channel("slack");
        assert_eq!(caps.workspace_prefix, "channels/slack/");
    }

    #[test]
    fn test_path_allowed() {
        let caps = ChannelCapabilities::default()
            .with_path("/webhook/slack")
            .with_path("/webhook/slack/events");

        assert!(caps.is_path_allowed("/webhook/slack"));
        assert!(caps.is_path_allowed("/webhook/slack/events"));
        assert!(!caps.is_path_allowed("/webhook/telegram"));
    }

    #[test]
    fn test_poll_interval_validation() {
        let caps = ChannelCapabilities::default().with_polling(60_000);

        // Valid interval
        assert_eq!(caps.validate_poll_interval(90_000).unwrap(), 90_000);

        // Too short, clamped to minimum
        assert_eq!(caps.validate_poll_interval(1000).unwrap(), 60_000);

        // Polling disabled
        let no_poll_caps = ChannelCapabilities::default();
        assert!(no_poll_caps.validate_poll_interval(60_000).is_err());
    }

    #[test]
    fn test_workspace_path_validation() {
        let caps = ChannelCapabilities::for_channel("slack");

        // Valid path
        let result = caps.validate_workspace_path("state.json");
        assert_eq!(result.unwrap(), "channels/slack/state.json");

        // Nested path
        let result = caps.validate_workspace_path("data/users.json");
        assert_eq!(result.unwrap(), "channels/slack/data/users.json");

        // Block absolute paths
        let result = caps.validate_workspace_path("/etc/passwd");
        assert!(result.is_err());

        // Block path traversal
        let result = caps.validate_workspace_path("../secrets/key.txt");
        assert!(result.is_err());

        // Block null bytes
        let result = caps.validate_workspace_path("file\0.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_http_endpoint_config() {
        let endpoint = HttpEndpointConfig::post_webhook("/webhook/slack");
        assert_eq!(endpoint.path, "/webhook/slack");
        assert_eq!(endpoint.methods, vec!["POST"]);
        assert!(endpoint.require_secret);
    }

    #[test]
    fn test_emit_rate_limit_default() {
        let limit = EmitRateLimitConfig::default();
        assert_eq!(limit.messages_per_minute, 100);
        assert_eq!(limit.messages_per_hour, 5000);
    }
}

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

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::channels::wasm::capabilities::{
    ChannelCapabilities, EmitRateLimitConfig, MIN_POLL_INTERVAL_MS,
};
use crate::channels::wasm::is_reserved_runtime_config_key;
use crate::tools::wasm::{CapabilitiesFile as ToolCapabilitiesFile, RateLimitSchema};

/// Root schema for a channel capabilities JSON file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelCapabilitiesFile {
    /// Extension version (semver).
    #[serde(default)]
    pub version: Option<String>,

    /// WIT interface version this channel was compiled against (semver).
    #[serde(default)]
    pub wit_version: Option<String>,

    /// File type, must be "channel".
    #[serde(default = "default_type")]
    pub r#type: String,

    /// Channel name.
    pub name: String,

    /// Channel description.
    #[serde(default)]
    pub description: Option<String>,

    /// Setup configuration for the wizard.
    #[serde(default)]
    pub setup: SetupSchema,

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

    /// Validate the capabilities file and emit warnings for common misconfigurations.
    ///
    /// Called once at load time to catch issues early. Warnings are emitted via
    /// `tracing::warn` so they show up in startup logs without blocking loading.
    pub fn validate(&self) {
        const MIN_PROMPT_LENGTH: usize = 30;

        // Check for short prompts in required_secrets
        for secret in &self.setup.required_secrets {
            if secret.prompt.len() < MIN_PROMPT_LENGTH {
                tracing::warn!(
                    channel = self.name,
                    secret = secret.name,
                    prompt = secret.prompt,
                    "setup.required_secrets prompt is shorter than {} chars — \
                     consider a more descriptive prompt that tells the user where to find this value",
                    MIN_PROMPT_LENGTH
                );
            }
        }

        // Has required_secrets but no setup_url
        if !self.setup.required_secrets.is_empty() && self.setup.setup_url.is_none() {
            tracing::warn!(
                channel = self.name,
                "setup.required_secrets defined but no setup.setup_url — \
                 user has no link to obtain credentials"
            );
        }

        // Validate manifest-driven runtime-config secret injection. Called
        // purely for its side effect: emitting warnings for unsafe mapping
        // entries (empty keys, reserved runtime keys, undeclared secret
        // names). The runtime path uses `validated_secret_config_mappings`
        // (silent filter).
        let _ = self.validated_secret_config_mappings_with_warnings();
    }

    /// Convert to runtime ChannelCapabilities.
    pub fn to_capabilities(&self) -> ChannelCapabilities {
        self.capabilities.to_channel_capabilities(&self.name)
    }

    /// Get the channel config as JSON string.
    pub fn config_json(&self) -> String {
        serde_json::to_string(&self.config).unwrap_or_else(|_| "{}".to_string())
    }

    /// Get the webhook secret header name for this channel.
    ///
    /// Returns the configured header name from capabilities, or a sensible default.
    pub fn webhook_secret_header(&self) -> Option<&str> {
        self.capabilities
            .channel
            .as_ref()
            .and_then(|c| c.webhook.as_ref())
            .and_then(|w| w.secret_header.as_deref())
    }

    /// Get the signature verification key secret name for this channel.
    ///
    /// Returns the secret name declared in `webhook.signature_key_secret_name`,
    /// used to look up the Ed25519 public key in the secrets store.
    pub fn signature_key_secret_name(&self) -> Option<&str> {
        self.capabilities
            .channel
            .as_ref()
            .and_then(|c| c.webhook.as_ref())
            .and_then(|w| w.signature_key_secret_name.as_deref())
    }

    /// Get the HMAC-SHA256 signing secret name for this channel.
    ///
    /// Returns the secret name declared in `webhook.hmac_secret_name`,
    /// used to look up the HMAC signing secret in the secrets store (Slack-style).
    pub fn hmac_secret_name(&self) -> Option<&str> {
        self.capabilities
            .channel
            .as_ref()
            .and_then(|c| c.webhook.as_ref())
            .and_then(|w| w.hmac_secret_name.as_deref())
    }

    /// Get the webhook secret name for this channel.
    ///
    /// Returns the configured secret name or defaults to "{channel_name}_webhook_secret".
    pub fn webhook_secret_name(&self) -> String {
        self.capabilities
            .channel
            .as_ref()
            .and_then(|c| c.webhook.as_ref())
            .and_then(|w| w.secret_name.clone())
            .unwrap_or_else(|| format!("{}_webhook_secret", self.name))
    }

    /// Whether the host should enforce generic webhook-secret validation.
    ///
    /// Defaults to true. Channels can opt out when they validate the shared
    /// secret themselves using provider-specific request body fields.
    pub fn webhook_secret_managed_by_host(&self) -> bool {
        self.capabilities
            .channel
            .as_ref()
            .and_then(|c| c.webhook.as_ref())
            .and_then(|w| w.managed_by_host)
            .unwrap_or(true)
    }

    /// Return setup.secret_config_mappings after applying security constraints.
    ///
    /// Only mappings for declared `setup.required_secrets` are allowed, and
    /// mappings cannot target reserved runtime config keys.
    pub fn validated_secret_config_mappings(&self) -> Vec<SecretConfigMappingSchema> {
        self.collect_validated_secret_config_mappings(false)
    }

    fn validated_secret_config_mappings_with_warnings(&self) -> Vec<SecretConfigMappingSchema> {
        self.collect_validated_secret_config_mappings(true)
    }

    fn collect_validated_secret_config_mappings(
        &self,
        emit_warnings: bool,
    ) -> Vec<SecretConfigMappingSchema> {
        let required_secret_names: HashSet<String> = self
            .setup
            .required_secrets
            .iter()
            .map(|secret| secret.name.trim().to_ascii_lowercase())
            .filter(|name| !name.is_empty())
            .collect();

        self.setup
            .secret_config_mappings
            .iter()
            .filter_map(|mapping| {
                let config_key = mapping.config_key.trim();
                let secret_name = mapping.secret_name.trim();

                if config_key.is_empty() {
                    if emit_warnings {
                        tracing::warn!(
                            channel = self.name,
                            secret_name = %mapping.secret_name,
                            "Ignoring setup.secret_config_mappings entry with empty config_key"
                        );
                    }
                    return None;
                }

                if is_reserved_runtime_config_key(config_key) {
                    if emit_warnings {
                        tracing::warn!(
                            channel = self.name,
                            config_key = %config_key,
                            "Ignoring setup.secret_config_mappings entry targeting reserved runtime config key"
                        );
                    }
                    return None;
                }

                if secret_name.is_empty() {
                    if emit_warnings {
                        tracing::warn!(
                            channel = self.name,
                            config_key = %config_key,
                            "Ignoring setup.secret_config_mappings entry with empty secret_name"
                        );
                    }
                    return None;
                }

                if !required_secret_names.contains(&secret_name.to_ascii_lowercase()) {
                    if emit_warnings {
                        tracing::warn!(
                            channel = self.name,
                            config_key = %config_key,
                            secret_name = %secret_name,
                            "Ignoring setup.secret_config_mappings entry for undeclared secret (must be listed in setup.required_secrets)"
                        );
                    }
                    return None;
                }

                Some(SecretConfigMappingSchema {
                    config_key: config_key.to_string(),
                    secret_name: secret_name.to_string(),
                })
            })
            .collect()
    }
}

/// Schema for channel capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelCapabilitiesSchema {
    /// Tool capabilities (HTTP, secrets, workspace_read).
    /// Note: Using the struct directly (not Option) because #[serde(flatten)]
    /// with Option<T> doesn't work correctly when T has all-optional fields.
    #[serde(flatten)]
    pub tool: ToolCapabilitiesFile,

    /// Channel-specific capabilities.
    #[serde(default)]
    pub channel: Option<ChannelSpecificCapabilitiesSchema>,
}

impl ChannelCapabilitiesSchema {
    /// Convert to runtime ChannelCapabilities.
    pub fn to_channel_capabilities(&self, channel_name: &str) -> ChannelCapabilities {
        let tool_caps = self.tool.to_capabilities();

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

            if !channel.durable_workspace_paths.is_empty() {
                caps.durable_workspace_paths = channel
                    .durable_workspace_paths
                    .iter()
                    .map(|path| caps.prefix_workspace_path(path))
                    .collect();
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

    /// Workspace paths that are safe to persist across restarts.
    #[serde(default)]
    pub durable_workspace_paths: Vec<String>,

    /// Rate limiting for emit_message.
    #[serde(default)]
    pub emit_rate_limit: Option<EmitRateLimitSchema>,

    /// Maximum message content size in bytes.
    #[serde(default)]
    pub max_message_size: Option<usize>,

    /// Callback timeout in seconds.
    #[serde(default)]
    pub callback_timeout_secs: Option<u64>,

    /// Webhook configuration (secret header, etc.).
    #[serde(default)]
    pub webhook: Option<WebhookSchema>,
}

/// Webhook configuration schema.
///
/// Allows channels to specify their webhook validation requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookSchema {
    /// HTTP header name for secret validation.
    ///
    /// Examples:
    /// - Telegram: "X-Telegram-Bot-Api-Secret-Token"
    /// - Slack: "X-Slack-Signature"
    /// - GitHub: "X-Hub-Signature-256"
    /// - Generic: "X-Webhook-Secret"
    #[serde(default)]
    pub secret_header: Option<String>,

    /// Secret name in secrets store for webhook validation.
    /// Default: "{channel_name}_webhook_secret"
    #[serde(default)]
    pub secret_name: Option<String>,

    /// Secret name in secrets store containing the Ed25519 public key
    /// for signature verification (e.g., Discord interaction verification).
    #[serde(default)]
    pub signature_key_secret_name: Option<String>,

    /// Secret name in secrets store for HMAC-SHA256 signing (Slack-style).
    #[serde(default)]
    pub hmac_secret_name: Option<String>,

    /// Whether the host/router should enforce generic webhook-secret
    /// validation before the channel sees the request.
    ///
    /// Default: true. Set to false when the provider sends the shared secret
    /// in a provider-specific request field rather than the configured header.
    #[serde(default)]
    pub managed_by_host: Option<bool>,
}

/// Setup configuration schema.
///
/// Allows channels to declare their setup requirements for the wizard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SetupSchema {
    /// Required secrets that must be configured during setup.
    #[serde(default)]
    pub required_secrets: Vec<SecretSetupSchema>,

    /// Optional mapping from secret names to runtime config keys.
    ///
    /// Use this when a channel needs raw secret values in its runtime config
    /// (for example, provider token exchanges that require secrets in request
    /// bodies). This is explicit opt-in to avoid over-injecting secrets.
    #[serde(default)]
    pub secret_config_mappings: Vec<SecretConfigMappingSchema>,

    /// Optional validation endpoint to verify configuration.
    /// Placeholders like {secret_name} are replaced with actual values.
    #[serde(default)]
    pub validation_endpoint: Option<String>,

    /// User-facing URL where they can create/manage credentials.
    #[serde(default)]
    pub setup_url: Option<String>,
}

/// Configuration for a secret required during setup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretSetupSchema {
    /// Secret name in the secrets store (e.g., "telegram_bot_token").
    pub name: String,

    /// Prompt to show the user during setup.
    pub prompt: String,

    /// Optional regex for validation.
    #[serde(default)]
    pub validation: Option<String>,

    /// Whether this secret is optional.
    #[serde(default)]
    pub optional: bool,

    /// Auto-generate configuration if the user doesn't provide a value.
    #[serde(default)]
    pub auto_generate: Option<AutoGenerateSchema>,
}

/// Mapping from a secret in the secrets store to a runtime config key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretConfigMappingSchema {
    /// Config key to inject into the channel runtime config.
    pub config_key: String,
    /// Secret name in the secrets store.
    pub secret_name: String,
}

/// Configuration for auto-generating a secret value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoGenerateSchema {
    /// Length of the generated value in bytes (will be hex-encoded).
    #[serde(default = "default_auto_generate_length")]
    pub length: usize,
}

fn default_auto_generate_length() -> usize {
    32
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
    fn test_durable_workspace_paths_are_prefixed() {
        let json = r#"{
            "name": "slack",
            "capabilities": {
                "channel": {
                    "workspace_prefix": "channels/slack/",
                    "durable_workspace_paths": ["state/active_threads"]
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        assert_eq!(
            caps.durable_workspace_paths,
            vec!["channels/slack/state/active_threads".to_string()]
        );
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

    #[test]
    fn test_webhook_schema() {
        let json = r#"{
            "name": "telegram",
            "capabilities": {
                "channel": {
                    "allowed_paths": ["/webhook/telegram"],
                    "webhook": {
                        "secret_header": "X-Telegram-Bot-Api-Secret-Token",
                        "secret_name": "telegram_webhook_secret"
                    }
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(
            file.webhook_secret_header(),
            Some("X-Telegram-Bot-Api-Secret-Token")
        );
        assert_eq!(file.webhook_secret_name(), "telegram_webhook_secret");
        assert!(file.webhook_secret_managed_by_host());
    }

    #[test]
    fn test_webhook_schema_can_disable_host_managed_secret_validation() {
        let json = r#"{
            "name": "feishu",
            "capabilities": {
                "channel": {
                    "webhook": {
                        "secret_name": "feishu_verification_token",
                        "managed_by_host": false
                    }
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert!(!file.webhook_secret_managed_by_host());
    }

    #[test]
    fn test_webhook_secret_name_default() {
        let json = r#"{
            "name": "mybot",
            "capabilities": {}
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(file.webhook_secret_header(), None);
        assert_eq!(file.webhook_secret_name(), "mybot_webhook_secret");
    }

    #[test]
    fn test_setup_schema() {
        let json = r#"{
            "name": "telegram",
            "setup": {
                "required_secrets": [
                    {
                        "name": "telegram_bot_token",
                        "prompt": "Enter your Telegram Bot Token",
                        "validation": "^[0-9]+:[A-Za-z0-9_-]+$"
                    },
                    {
                        "name": "telegram_webhook_secret",
                        "prompt": "Webhook secret (leave empty to auto-generate)",
                        "optional": true,
                        "auto_generate": { "length": 64 }
                    }
                ],
                "secret_config_mappings": [
                    {
                        "config_key": "bot_token",
                        "secret_name": "telegram_bot_token"
                    }
                ],
                "validation_endpoint": "https://api.telegram.org/bot{telegram_bot_token}/getMe"
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(file.setup.required_secrets.len(), 2);
        assert_eq!(file.setup.secret_config_mappings.len(), 1);
        assert_eq!(file.setup.secret_config_mappings[0].config_key, "bot_token");
        assert_eq!(
            file.setup.secret_config_mappings[0].secret_name,
            "telegram_bot_token"
        );
        assert_eq!(file.validated_secret_config_mappings().len(), 1);
        assert_eq!(file.setup.required_secrets[0].name, "telegram_bot_token");
        assert!(!file.setup.required_secrets[0].optional);
        assert!(file.setup.required_secrets[1].optional);
        assert_eq!(
            file.setup.required_secrets[1]
                .auto_generate
                .as_ref()
                .unwrap()
                .length,
            64
        );
    }

    #[test]
    fn test_validated_secret_config_mappings_filters_unsafe_entries() {
        let json = r#"{
            "name": "feishu",
            "setup": {
                "required_secrets": [
                    {
                        "name": "feishu_app_id",
                        "prompt": "Enter your Feishu app id from open.feishu.cn"
                    }
                ],
                "secret_config_mappings": [
                    { "config_key": "app_id", "secret_name": "feishu_app_id" },
                    { "config_key": "webhook_secret", "secret_name": "feishu_app_id" },
                    { "config_key": "bot_username", "secret_name": "feishu_app_id" },
                    { "config_key": "stolen_key", "secret_name": "openai_api_key" },
                    { "config_key": "", "secret_name": "feishu_app_id" }
                ]
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        let mappings = file.validated_secret_config_mappings();
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].config_key, "app_id");
        assert_eq!(mappings[0].secret_name, "feishu_app_id");
    }

    #[test]
    fn test_telegram_bundled_setup_includes_webhook_secret() {
        let json = include_str!("../../../channels-src/telegram/telegram.capabilities.json");
        let file = ChannelCapabilitiesFile::from_json(json).unwrap();

        let webhook_secret = file
            .setup
            .required_secrets
            .iter()
            .find(|secret| secret.name == "telegram_webhook_secret")
            .expect("telegram webhook secret should be declared in the bundled manifest");

        assert!(webhook_secret.optional);
        assert_eq!(
            webhook_secret
                .auto_generate
                .as_ref()
                .expect("telegram webhook secret should auto-generate")
                .length,
            64
        );
    }

    // ── Category 5: Discord Capabilities Setup & Configuration ──────────

    #[test]
    fn test_validate_channel_short_prompt() {
        // prompt < 30 chars — should not panic
        let json = r#"{
            "name": "test-channel",
            "setup": {
                "required_secrets": [
                    { "name": "bot_token", "prompt": "Bot token" }
                ],
                "setup_url": "https://example.com"
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        // Should not panic; warning emitted for short prompt
        file.validate();
    }

    #[test]
    fn test_validate_channel_missing_setup_url() {
        // required_secrets without setup_url — should not panic
        let json = r#"{
            "name": "test-channel",
            "setup": {
                "required_secrets": [
                    {
                        "name": "bot_token",
                        "prompt": "Enter your bot token from the developer portal settings"
                    }
                ]
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        // Should not panic; warning emitted for missing setup_url
        file.validate();
    }

    #[test]
    fn test_validate_clean_channel() {
        // Well-configured channel — should not panic or warn
        let json = r#"{
            "name": "good-channel",
            "setup": {
                "required_secrets": [
                    {
                        "name": "bot_token",
                        "prompt": "Enter your bot token from https://example.com/bot-settings"
                    }
                ],
                "setup_url": "https://example.com/bot-settings"
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        // Should not panic and emits no warnings
        file.validate();
    }

    #[test]
    fn test_discord_capabilities_has_public_key_secret() {
        let json = include_str!("../../../channels-src/discord/discord.capabilities.json");
        let file = ChannelCapabilitiesFile::from_json(json).unwrap();

        let secret_names: Vec<&str> = file
            .setup
            .required_secrets
            .iter()
            .map(|s| s.name.as_str())
            .collect();

        assert!(
            secret_names.contains(&"discord_public_key"),
            "discord.capabilities.json must include discord_public_key in setup.required_secrets, \
             found: {:?}",
            secret_names
        );
    }

    #[test]
    fn test_webhook_schema_signature_key_secret_name() {
        let json = r#"{
            "name": "discord",
            "capabilities": {
                "channel": {
                    "allowed_paths": ["/webhook/discord"],
                    "webhook": {
                        "signature_key_secret_name": "discord_public_key"
                    }
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(file.signature_key_secret_name(), Some("discord_public_key"));
    }

    #[test]
    fn test_signature_key_secret_name_none_when_missing() {
        let json = r#"{
            "name": "telegram",
            "capabilities": {
                "channel": {
                    "allowed_paths": ["/webhook/telegram"],
                    "webhook": {
                        "secret_header": "X-Telegram-Bot-Api-Secret-Token"
                    }
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(file.signature_key_secret_name(), None);
    }

    #[test]
    fn test_discord_capabilities_signature_key() {
        let json = include_str!("../../../channels-src/discord/discord.capabilities.json");
        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(
            file.signature_key_secret_name(),
            Some("discord_public_key"),
            "discord.capabilities.json must declare signature_key_secret_name"
        );
    }

    #[test]
    fn test_discord_capabilities_secrets_allowlist() {
        let json = include_str!("../../../channels-src/discord/discord.capabilities.json");
        let file = ChannelCapabilitiesFile::from_json(json).unwrap();

        let caps = file.to_capabilities();
        let secrets_caps = caps
            .tool_capabilities
            .secrets
            .expect("Discord should have secrets capability");

        assert!(
            secrets_caps.is_allowed("discord_public_key"),
            "discord_public_key must be in the secrets allowlist"
        );
    }
}

//! Channel-relay service configuration.

use secrecy::SecretString;

/// Configuration for connecting to a channel-relay service.
#[derive(Clone)]
pub struct RelayConfig {
    /// Base URL of the channel-relay service (e.g., `http://localhost:3001`).
    pub url: String,
    /// Bearer token for authenticated channel-relay endpoints (`sk-agent-*`).
    pub api_key: SecretString,
    /// Signing secret for verifying webhook callbacks from channel-relay.
    pub signing_secret: SecretString,
    /// Override for the OAuth callback URL (e.g., a tunnel URL).
    pub callback_url: Option<String>,
    /// Override for the instance identifier.
    pub instance_id: Option<String>,
    /// HTTP request timeout in seconds (default: 30).
    pub request_timeout_secs: u64,
    /// Path for the webhook callback endpoint (default: `/relay/events`).
    pub webhook_path: String,
}

impl std::fmt::Debug for RelayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayConfig")
            .field("url", &self.url)
            .field("api_key", &"[REDACTED]")
            .field("signing_secret", &"[REDACTED]")
            .field("callback_url", &self.callback_url)
            .field("instance_id", &self.instance_id)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("webhook_path", &self.webhook_path)
            .finish()
    }
}

impl RelayConfig {
    /// Load relay config from environment variables.
    ///
    /// Returns `None` if any required env var (`CHANNEL_RELAY_URL`,
    /// `CHANNEL_RELAY_API_KEY`, `CHANNEL_RELAY_SIGNING_SECRET`) is not set,
    /// making the relay integration opt-in.
    pub fn from_env() -> Option<Self> {
        Self::from_env_reader(|key| std::env::var(key).ok())
    }

    /// Build a config for tests without touching the process environment.
    pub fn from_values(
        url: impl Into<String>,
        api_key: impl Into<String>,
        signing_secret: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            api_key: SecretString::from(api_key.into()),
            signing_secret: SecretString::from(signing_secret.into()),
            callback_url: None,
            instance_id: None,
            request_timeout_secs: 30,
            webhook_path: "/relay/events".into(),
        }
    }

    /// Internal constructor that reads values through a closure, enabling safe testing.
    fn from_env_reader(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let url = env("CHANNEL_RELAY_URL")?;
        let api_key = SecretString::from(env("CHANNEL_RELAY_API_KEY")?);
        let signing_secret = SecretString::from(env("CHANNEL_RELAY_SIGNING_SECRET")?);
        Some(Self {
            url,
            api_key,
            signing_secret,
            callback_url: env("IRONCLAW_OAUTH_CALLBACK_URL"),
            instance_id: env("IRONCLAW_INSTANCE_ID"),
            request_timeout_secs: env("RELAY_REQUEST_TIMEOUT_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            webhook_path: env("RELAY_WEBHOOK_PATH").unwrap_or_else(|| "/relay/events".into()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_reader_returns_none_when_unset() {
        let config = RelayConfig::from_env_reader(|_| None);
        assert!(config.is_none());
    }

    #[test]
    fn from_env_reader_returns_none_without_signing_secret() {
        let config = RelayConfig::from_env_reader(|key| match key {
            "CHANNEL_RELAY_URL" => Some("http://localhost:3001".into()),
            "CHANNEL_RELAY_API_KEY" => Some("test-key".into()),
            _ => None,
        });
        assert!(config.is_none(), "signing_secret is required");
    }

    #[test]
    fn from_env_reader_loads_all_required() {
        let config = RelayConfig::from_env_reader(|key| match key {
            "CHANNEL_RELAY_URL" => Some("http://localhost:3001".into()),
            "CHANNEL_RELAY_API_KEY" => Some("test-key".into()),
            "CHANNEL_RELAY_SIGNING_SECRET" => Some("test-sign".into()),
            _ => None,
        })
        .expect("config should be Some");

        assert_eq!(config.url, "http://localhost:3001");
        assert_eq!(config.request_timeout_secs, 30);
        assert_eq!(config.webhook_path, "/relay/events");
        assert!(config.callback_url.is_none());
        assert!(config.instance_id.is_none());
    }

    #[test]
    fn from_env_reader_loads_overrides() {
        let config = RelayConfig::from_env_reader(|key| match key {
            "CHANNEL_RELAY_URL" => Some("http://relay:3001".into()),
            "CHANNEL_RELAY_API_KEY" => Some("secret".into()),
            "CHANNEL_RELAY_SIGNING_SECRET" => Some("sign-secret".into()),
            "IRONCLAW_OAUTH_CALLBACK_URL" => Some("https://tunnel.example.com".into()),
            "IRONCLAW_INSTANCE_ID" => Some("my-instance".into()),
            "RELAY_REQUEST_TIMEOUT_SECS" => Some("60".into()),
            "RELAY_WEBHOOK_PATH" => Some("/custom/events".into()),
            _ => None,
        })
        .expect("config should be Some");

        assert_eq!(
            config.callback_url.as_deref(),
            Some("https://tunnel.example.com")
        );
        assert_eq!(config.instance_id.as_deref(), Some("my-instance"));
        assert_eq!(config.request_timeout_secs, 60);
        assert_eq!(config.webhook_path, "/custom/events");
    }

    #[test]
    fn from_values_builds_with_defaults() {
        let config = RelayConfig::from_values("http://localhost:3001", "key", "sign");
        assert_eq!(config.url, "http://localhost:3001");
        assert_eq!(config.request_timeout_secs, 30);
    }

    #[test]
    fn debug_redacts_secrets() {
        let config =
            RelayConfig::from_values("http://localhost:3001", "super-secret", "sign-secret");
        let debug = format!("{:?}", config);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret"));
        assert!(!debug.contains("sign-secret"));
    }
}

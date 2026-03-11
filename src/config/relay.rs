//! Channel-relay service configuration.

use secrecy::SecretString;

/// Configuration for connecting to a channel-relay service.
#[derive(Clone)]
pub struct RelayConfig {
    /// Base URL of the channel-relay service (e.g., `http://localhost:3001`).
    pub url: String,
    /// API key for authenticated channel-relay endpoints.
    pub api_key: SecretString,
    /// Override for the OAuth callback URL (e.g., a tunnel URL).
    pub callback_url: Option<String>,
    /// Override for the instance identifier.
    pub instance_id: Option<String>,
    /// HTTP request timeout in seconds (default: 30).
    pub request_timeout_secs: u64,
    /// SSE stream long-poll timeout in seconds (default: 86400 = 24 h).
    pub stream_timeout_secs: u64,
    /// Initial exponential backoff in milliseconds (default: 1000).
    pub backoff_initial_ms: u64,
    /// Maximum exponential backoff in milliseconds (default: 60000).
    pub backoff_max_ms: u64,
}

impl std::fmt::Debug for RelayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayConfig")
            .field("url", &self.url)
            .field("api_key", &"[REDACTED]")
            .field("callback_url", &self.callback_url)
            .field("instance_id", &self.instance_id)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("stream_timeout_secs", &self.stream_timeout_secs)
            .field("backoff_initial_ms", &self.backoff_initial_ms)
            .field("backoff_max_ms", &self.backoff_max_ms)
            .finish()
    }
}

impl RelayConfig {
    /// Load relay config from environment variables.
    ///
    /// Returns `None` if either `CHANNEL_RELAY_URL` or `CHANNEL_RELAY_API_KEY`
    /// is not set, making the relay integration opt-in.
    pub fn from_env() -> Option<Self> {
        Self::from_env_reader(|key| std::env::var(key).ok())
    }

    /// Build a config for tests without touching the process environment.
    pub fn from_values(url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            api_key: SecretString::from(api_key.into()),
            callback_url: None,
            instance_id: None,
            request_timeout_secs: 30,
            stream_timeout_secs: 86400,
            backoff_initial_ms: 1000,
            backoff_max_ms: 60000,
        }
    }

    /// Internal constructor that reads values through a closure, enabling safe testing.
    fn from_env_reader(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let url = env("CHANNEL_RELAY_URL")?;
        let api_key = SecretString::from(env("CHANNEL_RELAY_API_KEY")?);
        Some(Self {
            url,
            api_key,
            callback_url: env("IRONCLAW_OAUTH_CALLBACK_URL"),
            instance_id: env("IRONCLAW_INSTANCE_ID"),
            request_timeout_secs: env("RELAY_REQUEST_TIMEOUT_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            stream_timeout_secs: env("RELAY_STREAM_TIMEOUT_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(86400),
            backoff_initial_ms: env("RELAY_BACKOFF_INITIAL_MS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
            backoff_max_ms: env("RELAY_BACKOFF_MAX_MS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(60000),
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
    fn from_env_reader_loads_defaults() {
        let config = RelayConfig::from_env_reader(|key| match key {
            "CHANNEL_RELAY_URL" => Some("http://localhost:3001".into()),
            "CHANNEL_RELAY_API_KEY" => Some("test-key".into()),
            _ => None,
        })
        .expect("config should be Some");

        assert_eq!(config.url, "http://localhost:3001");
        assert_eq!(config.request_timeout_secs, 30);
        assert_eq!(config.stream_timeout_secs, 86400);
        assert_eq!(config.backoff_initial_ms, 1000);
        assert_eq!(config.backoff_max_ms, 60000);
        assert!(config.callback_url.is_none());
        assert!(config.instance_id.is_none());
    }

    #[test]
    fn from_env_reader_loads_overrides() {
        let config = RelayConfig::from_env_reader(|key| match key {
            "CHANNEL_RELAY_URL" => Some("http://relay:3001".into()),
            "CHANNEL_RELAY_API_KEY" => Some("secret".into()),
            "IRONCLAW_OAUTH_CALLBACK_URL" => Some("https://tunnel.example.com".into()),
            "IRONCLAW_INSTANCE_ID" => Some("my-instance".into()),
            "RELAY_REQUEST_TIMEOUT_SECS" => Some("60".into()),
            "RELAY_STREAM_TIMEOUT_SECS" => Some("43200".into()),
            "RELAY_BACKOFF_INITIAL_MS" => Some("2000".into()),
            "RELAY_BACKOFF_MAX_MS" => Some("120000".into()),
            _ => None,
        })
        .expect("config should be Some");

        assert_eq!(
            config.callback_url.as_deref(),
            Some("https://tunnel.example.com")
        );
        assert_eq!(config.instance_id.as_deref(), Some("my-instance"));
        assert_eq!(config.request_timeout_secs, 60);
        assert_eq!(config.stream_timeout_secs, 43200);
        assert_eq!(config.backoff_initial_ms, 2000);
        assert_eq!(config.backoff_max_ms, 120000);
    }

    #[test]
    fn from_values_builds_with_defaults() {
        let config = RelayConfig::from_values("http://localhost:3001", "key");
        assert_eq!(config.url, "http://localhost:3001");
        assert_eq!(config.request_timeout_secs, 30);
    }

    #[test]
    fn debug_redacts_api_key() {
        let config = RelayConfig::from_values("http://localhost:3001", "super-secret");
        let debug = format!("{:?}", config);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret"));
    }
}

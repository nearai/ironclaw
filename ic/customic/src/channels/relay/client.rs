//! HTTP client for the channel-relay service.
//!
//! Wraps reqwest for all channel-relay API calls: OAuth initiation,
//! approvals, signing-secret fetch, and Slack API proxy.

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

/// Known relay event types.
pub mod event_types {
    pub const MESSAGE: &str = "message";
    pub const DIRECT_MESSAGE: &str = "direct_message";
    pub const MENTION: &str = "mention";
}

/// A parsed event from the channel-relay webhook callback.
///
/// Field names match the channel-relay `ChannelEvent` struct exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelEvent {
    /// Unique event ID.
    #[serde(default)]
    pub id: String,
    /// Event type enum from channel-relay (e.g., "direct_message", "message", "mention").
    pub event_type: String,
    /// Provider (e.g., "slack").
    #[serde(default)]
    pub provider: String,
    /// Team/workspace ID (called `provider_scope` in channel-relay).
    #[serde(alias = "team_id", default)]
    pub provider_scope: String,
    /// Channel or DM conversation ID.
    #[serde(default)]
    pub channel_id: String,
    /// Sender user ID.
    #[serde(default)]
    pub sender_id: String,
    /// Sender display name.
    #[serde(default)]
    pub sender_name: Option<String>,
    /// Message text content (called `content` in channel-relay).
    #[serde(alias = "text", default)]
    pub content: Option<String>,
    /// Thread ID (for threaded replies, called `thread_id` in channel-relay).
    #[serde(alias = "thread_ts", default)]
    pub thread_id: Option<String>,
    /// Full raw event data.
    #[serde(default)]
    pub raw: serde_json::Value,
    /// Event timestamp (ISO 8601 from channel-relay).
    #[serde(default)]
    pub timestamp: Option<String>,
}

impl ChannelEvent {
    /// Get the team_id (provider_scope).
    pub fn team_id(&self) -> &str {
        &self.provider_scope
    }

    /// Get the message text content.
    pub fn text(&self) -> &str {
        self.content.as_deref().unwrap_or("")
    }

    /// Get the sender name or fallback to sender_id.
    pub fn display_name(&self) -> &str {
        self.sender_name.as_deref().unwrap_or(&self.sender_id)
    }

    /// Check if this is a message-like event that should be forwarded to the agent.
    pub fn is_message(&self) -> bool {
        matches!(
            self.event_type.as_str(),
            event_types::MESSAGE | event_types::DIRECT_MESSAGE | event_types::MENTION
        )
    }
}

/// Connection info returned by list_connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub provider: String,
    pub team_id: String,
    pub team_name: Option<String>,
    pub connected: bool,
}

/// HTTP client for the channel-relay service.
#[derive(Clone)]
pub struct RelayClient {
    http: reqwest::Client,
    base_url: String,
    api_key: SecretString,
}

impl RelayClient {
    /// Create a new relay client.
    pub fn new(
        base_url: String,
        api_key: SecretString,
        request_timeout_secs: u64,
    ) -> Result<Self, RelayError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(request_timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| RelayError::Network(format!("Failed to build HTTP client: {e}")))?;

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        })
    }

    /// Initiate Slack OAuth flow via channel-relay.
    ///
    /// Calls `GET /oauth/slack/auth` with `redirect(Policy::none())` and
    /// returns the `Location` header (Slack OAuth URL) without following it.
    /// Initiate Slack OAuth. Channel-relay derives all URLs from the trusted
    /// instance_url in chat-api. IronClaw only passes an optional CSRF nonce
    /// for validating the callback — no URLs.
    pub async fn initiate_oauth(&self, state_nonce: Option<&str>) -> Result<String, RelayError> {
        let mut query: Vec<(&str, &str)> = vec![];
        if let Some(nonce) = state_nonce {
            query.push(("state_nonce", nonce));
        }
        let resp = self
            .http
            .get(format!("{}/oauth/slack/auth", self.base_url))
            .bearer_auth(self.api_key.expose_secret())
            .query(&query)
            .send()
            .await
            .map_err(|e| RelayError::Network(e.to_string()))?;

        let status = resp.status();
        if status.is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    RelayError::Protocol("Redirect response missing Location header".to_string())
                })?;
            Ok(location)
        } else if status.is_success() {
            // Some relay implementations return the URL in JSON body instead
            let body: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| RelayError::Protocol(e.to_string()))?;
            body.get("auth_url")
                .or_else(|| body.get("url"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| RelayError::Protocol("Response missing auth_url field".to_string()))
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(RelayError::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    /// Register a pending approval and return the opaque approval token.
    ///
    /// Calls `POST /approvals` with the target team/channel/request identifiers.
    /// The returned token is embedded in Slack button values instead of routing fields.
    /// The relay derives the authorized approver from the connection's authed_user_id.
    pub async fn create_approval(
        &self,
        team_id: &str,
        channel_id: &str,
        thread_ts: Option<&str>,
        request_id: &str,
    ) -> Result<String, RelayError> {
        let mut body = serde_json::json!({
            "team_id": team_id,
            "channel_id": channel_id,
            "request_id": request_id,
        });
        if let Some(ts) = thread_ts {
            body["thread_ts"] = serde_json::Value::String(ts.to_string());
        }

        let resp = self
            .http
            .post(format!("{}/approvals", self.base_url))
            .bearer_auth(self.api_key.expose_secret())
            .json(&body)
            .send()
            .await
            .map_err(|e| RelayError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(RelayError::Api {
                status,
                message: body,
            });
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| RelayError::Protocol(e.to_string()))?;

        result
            .get("approval_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| RelayError::Protocol("missing approval_token in response".to_string()))
    }

    pub async fn proxy_provider(
        &self,
        provider: &str,
        team_id: &str,
        method: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, RelayError> {
        let query: Vec<(&str, &str)> = vec![("team_id", team_id)];
        let resp = self
            .http
            .post(format!("{}/proxy/{}/{}", self.base_url, provider, method))
            .bearer_auth(self.api_key.expose_secret())
            .query(&query)
            .json(&body)
            .send()
            .await
            .map_err(|e| RelayError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(RelayError::Api {
                status,
                message: body,
            });
        }

        resp.json()
            .await
            .map_err(|e| RelayError::Protocol(e.to_string()))
    }

    /// Fetch the per-instance callback signing secret from channel-relay.
    ///
    /// Calls `GET /relay/signing-secret` (authenticated) and returns the decoded
    /// 32-byte secret. Called once at activation time; the result is cached in the
    /// extension manager so subsequent calls to `relay_signing_secret()` use it.
    pub async fn get_signing_secret(&self, team_id: &str) -> Result<Vec<u8>, RelayError> {
        let resp = self
            .http
            .get(format!("{}/relay/signing-secret", self.base_url))
            .bearer_auth(self.api_key.expose_secret())
            .query(&[("team_id", team_id)])
            .send()
            .await
            .map_err(|e| RelayError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(RelayError::Api {
                status,
                message: body,
            });
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| RelayError::Protocol(e.to_string()))?;

        body.get("signing_secret")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RelayError::Protocol("missing signing_secret in response".to_string()))
            .and_then(|raw| {
                let decoded = hex::decode(raw).map_err(|e| {
                    RelayError::Protocol(format!("invalid signing_secret hex: {e}"))
                })?;
                if decoded.len() != 32 {
                    return Err(RelayError::Protocol(format!(
                        "invalid signing_secret length: expected 32 bytes, got {}",
                        decoded.len()
                    )));
                }
                Ok(decoded)
            })
    }

    /// List active connections for an instance.
    pub async fn list_connections(&self, instance_id: &str) -> Result<Vec<Connection>, RelayError> {
        let resp = self
            .http
            .get(format!("{}/connections", self.base_url))
            .bearer_auth(self.api_key.expose_secret())
            .query(&[("instance_id", instance_id)])
            .send()
            .await
            .map_err(|e| RelayError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(RelayError::Api {
                status,
                message: body,
            });
        }

        resp.json()
            .await
            .map_err(|e| RelayError::Protocol(e.to_string()))
    }
}

/// Errors from relay client operations.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("API error (HTTP {status}): {message}")]
    Api { status: u16, message: String },

    #[error("Protocol error: {0}")]
    Protocol(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_event_deserialize_minimal() {
        let json = r#"{"event_type": "message", "content": "hello"}"#;
        let event: ChannelEvent = serde_json::from_str(json).expect("parse failed");
        assert_eq!(event.event_type, "message");
        assert_eq!(event.text(), "hello");
        assert!(event.provider_scope.is_empty());
    }

    #[test]
    fn channel_event_deserialize_relay_format() {
        // Matches the actual channel-relay ChannelEvent serialization format.
        let json = r#"{
            "id": "evt_123",
            "event_type": "direct_message",
            "provider": "slack",
            "provider_scope": "T123",
            "channel_id": "D456",
            "sender_id": "U789",
            "sender_name": "bob",
            "content": "hi there",
            "thread_id": "1234567890.123456",
            "raw": {},
            "timestamp": "2026-03-09T21:00:00Z"
        }"#;
        let event: ChannelEvent = serde_json::from_str(json).expect("parse failed");
        assert_eq!(event.provider, "slack");
        assert_eq!(event.team_id(), "T123");
        assert_eq!(event.display_name(), "bob");
        assert_eq!(event.thread_id, Some("1234567890.123456".to_string()));
        assert!(event.is_message());
    }

    #[test]
    fn channel_event_is_message() {
        let make = |et: &str| ChannelEvent {
            id: String::new(),
            event_type: et.to_string(),
            provider: String::new(),
            provider_scope: String::new(),
            channel_id: String::new(),
            sender_id: String::new(),
            sender_name: None,
            content: None,
            thread_id: None,
            raw: serde_json::Value::Null,
            timestamp: None,
        };
        assert!(make("message").is_message());
        assert!(make("direct_message").is_message());
        assert!(make("mention").is_message());
        assert!(!make("reaction").is_message());
    }

    #[test]
    fn connection_deserialize() {
        let json = r#"{"provider": "slack", "team_id": "T123", "team_name": "My Team", "connected": true}"#;
        let conn: Connection = serde_json::from_str(json).expect("parse failed");
        assert_eq!(conn.provider, "slack");
        assert!(conn.connected);
    }

    #[test]
    fn relay_error_display() {
        let err = RelayError::Network("timeout".into());
        assert_eq!(err.to_string(), "Network error: timeout");

        let err = RelayError::Api {
            status: 401,
            message: "unauthorized".into(),
        };
        assert_eq!(err.to_string(), "API error (HTTP 401): unauthorized");
    }

    #[test]
    fn event_type_constants_match_is_message() {
        let make = |et: &str| ChannelEvent {
            id: String::new(),
            event_type: et.to_string(),
            provider: String::new(),
            provider_scope: String::new(),
            channel_id: String::new(),
            sender_id: String::new(),
            sender_name: None,
            content: None,
            thread_id: None,
            raw: serde_json::Value::Null,
            timestamp: None,
        };
        assert!(make(event_types::MESSAGE).is_message());
        assert!(make(event_types::DIRECT_MESSAGE).is_message());
        assert!(make(event_types::MENTION).is_message());
    }
}

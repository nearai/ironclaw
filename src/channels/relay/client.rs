//! HTTP client for the channel-relay service.
//!
//! Wraps reqwest for all channel-relay API calls: OAuth initiation,
//! SSE streaming, token renewal, and Slack API proxy.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// A parsed SSE event from the channel-relay stream.
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
            "message" | "direct_message" | "mention"
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
    pub async fn initiate_oauth(
        &self,
        instance_id: &str,
        user_id: &str,
        callback_url: &str,
    ) -> Result<String, RelayError> {
        let resp = self
            .http
            .get(format!("{}/oauth/slack/auth", self.base_url))
            .header("X-API-Key", self.api_key.expose_secret())
            .query(&[
                ("instance_id", instance_id),
                ("user_id", user_id),
                ("callback", callback_url),
            ])
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

    /// Connect to the SSE event stream.
    ///
    /// Returns a stream of parsed `ChannelEvent`s and the `JoinHandle` of the
    /// background SSE parser task. The caller is responsible for reconnection
    /// logic on stream end/error and for aborting the handle on shutdown.
    pub async fn connect_stream(
        &self,
        stream_token: &str,
        stream_timeout_secs: u64,
    ) -> Result<(ChannelEventStream, tokio::task::JoinHandle<()>), RelayError> {
        let resp = self
            .http
            .get(format!("{}/stream", self.base_url))
            .query(&[("token", stream_token)])
            .timeout(std::time::Duration::from_secs(stream_timeout_secs))
            .send()
            .await
            .map_err(|e| RelayError::Network(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(RelayError::TokenExpired);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(RelayError::Api {
                status: status.as_u16(),
                message: body,
            });
        }

        // Spawn a background task that reads the SSE stream and sends parsed events
        let (tx, rx) = mpsc::channel(64);
        let byte_stream = resp.bytes_stream();
        let handle = tokio::spawn(parse_sse_stream(byte_stream, tx));

        Ok((ChannelEventStream { rx }, handle))
    }

    /// Renew an expired stream token.
    ///
    /// Calls `POST /stream/renew` with API key auth, returns a new stream token.
    pub async fn renew_token(
        &self,
        instance_id: &str,
        user_id: &str,
    ) -> Result<String, RelayError> {
        let resp = self
            .http
            .post(format!("{}/stream/renew", self.base_url))
            .header("X-API-Key", self.api_key.expose_secret())
            .json(&serde_json::json!({
                "instance_id": instance_id,
                "user_id": user_id,
            }))
            .send()
            .await
            .map_err(|e| RelayError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(RelayError::Api {
                status: status.as_u16(),
                message: body,
            });
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| RelayError::Protocol(e.to_string()))?;
        body.get("stream_token")
            .or_else(|| body.get("token"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| RelayError::Protocol("Response missing stream_token field".to_string()))
    }

    /// Proxy a Slack API call through channel-relay.
    ///
    /// Calls `POST /proxy/slack/{method}?team_id=X&instance_id=Y` with the given JSON body.
    pub async fn proxy_slack(
        &self,
        team_id: &str,
        method: &str,
        body: serde_json::Value,
        instance_id: Option<&str>,
    ) -> Result<serde_json::Value, RelayError> {
        let mut query: Vec<(&str, &str)> = vec![("team_id", team_id)];
        if let Some(iid) = instance_id {
            query.push(("instance_id", iid));
        }
        let resp = self
            .http
            .post(format!("{}/proxy/slack/{}", self.base_url, method))
            .header("X-API-Key", self.api_key.expose_secret())
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

    /// Proxy an API call through channel-relay for any provider.
    ///
    /// Calls `POST /proxy/{provider}/{method}?team_id=X&instance_id=Y` with the given JSON body.
    pub async fn proxy_provider(
        &self,
        provider: &str,
        team_id: &str,
        method: &str,
        body: serde_json::Value,
        instance_id: Option<&str>,
    ) -> Result<serde_json::Value, RelayError> {
        let mut query: Vec<(&str, &str)> = vec![("team_id", team_id)];
        if let Some(iid) = instance_id {
            query.push(("instance_id", iid));
        }
        let resp = self
            .http
            .post(format!("{}/proxy/{}/{}", self.base_url, provider, method))
            .header("X-API-Key", self.api_key.expose_secret())
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

    /// List active connections for an instance.
    pub async fn list_connections(&self, instance_id: &str) -> Result<Vec<Connection>, RelayError> {
        let resp = self
            .http
            .get(format!("{}/connections", self.base_url))
            .header("X-API-Key", self.api_key.expose_secret())
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

/// Async stream of parsed channel events from SSE.
pub struct ChannelEventStream {
    rx: mpsc::Receiver<ChannelEvent>,
}

impl Stream for ChannelEventStream {
    type Item = ChannelEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// Parse SSE format from a reqwest bytes stream.
///
/// SSE format:
/// ```text
/// event: message
/// data: {"key": "value"}
///
/// ```
/// Blank line terminates an event.
async fn parse_sse_stream(
    byte_stream: impl futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
    tx: mpsc::Sender<ChannelEvent>,
) {
    use futures::StreamExt;

    let mut buffer = String::new();
    let mut event_type = String::new();
    let mut data_lines = Vec::new();

    let mut byte_stream = std::pin::pin!(byte_stream);
    while let Some(chunk_result) = byte_stream.next().await {
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(error = %e, "SSE stream chunk error");
                break;
            }
        };

        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Process complete lines
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                // Blank line = end of event
                if !data_lines.is_empty() {
                    let data = data_lines.join("\n");
                    if let Ok(mut event) = serde_json::from_str::<ChannelEvent>(&data) {
                        if event.event_type.is_empty() && !event_type.is_empty() {
                            event.event_type = event_type.clone();
                        }
                        if tx.send(event).await.is_err() {
                            return; // receiver dropped
                        }
                    } else {
                        tracing::debug!(
                            event_type = %event_type,
                            data_len = data.len(),
                            "Failed to parse SSE event data as ChannelEvent"
                        );
                    }
                }
                event_type.clear();
                data_lines.clear();
            } else if let Some(value) = line.strip_prefix("event:") {
                event_type = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("data:") {
                data_lines.push(value.trim().to_string());
            }
            // Ignore other fields (id:, retry:, comments)
        }
    }

    tracing::debug!("SSE stream ended");
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

    #[error("Stream token expired")]
    TokenExpired,
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

        let err = RelayError::TokenExpired;
        assert_eq!(err.to_string(), "Stream token expired");
    }
}

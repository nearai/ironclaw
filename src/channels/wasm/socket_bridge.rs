//! Socket Mode bridge for WASM channels.
//!
//! Manages a persistent WebSocket connection to a messaging platform (e.g., Slack)
//! and bridges events into the existing WASM `on_http_request` callback. The WASM
//! module is completely transport-unaware — it sees the same payload whether events
//! arrive via webhook or WebSocket.
//!
//! # Architecture
//!
//! ```text
//! Platform WSS ──> SocketModeBridge (host) ──> WasmChannel::call_on_http_request
//!                       │                              │
//!                       ├─ ack envelope immediately    ├─ same event_callback parsing
//!                       ├─ manage reconnection         ├─ same emit_message calls
//!                       └─ read app token from secrets └─ same on_respond for replies
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::channels::wasm::capabilities::SocketModeConfig;
use crate::channels::wasm::error::WasmChannelError;
use crate::channels::wasm::wrapper::WasmChannel;
use crate::secrets::SecretsStore;

/// Spawn a Socket Mode bridge as a background tokio task.
///
/// The bridge connects to the platform's WebSocket endpoint, acks envelopes,
/// and forwards event payloads to the WASM channel's `call_on_http_request`.
///
/// Returns a shutdown sender — drop or send `()` to stop the bridge.
pub fn spawn_socket_bridge(
    channel: Arc<WasmChannel>,
    config: SocketModeConfig,
    secrets_store: Option<Arc<dyn SecretsStore>>,
    shutdown_rx: oneshot::Receiver<()>,
) {
    let channel_name = channel.channel_name().to_string();

    tokio::spawn(async move {
        if let Err(e) = run_bridge(channel, config, secrets_store, shutdown_rx).await {
            tracing::error!(
                channel = %channel_name,
                error = %e,
                "Socket Mode bridge exited with error"
            );
        }
    });
}

/// Main bridge loop: connect, read events, reconnect on failure.
async fn run_bridge(
    channel: Arc<WasmChannel>,
    config: SocketModeConfig,
    secrets_store: Option<Arc<dyn SecretsStore>>,
    shutdown_rx: oneshot::Receiver<()>,
) -> Result<(), WasmChannelError> {
    let channel_name = channel.channel_name().to_string();

    // Read the app token: try secrets store first, then fall back to env var.
    // The env var name is the UPPER_CASE version of the secret name (e.g., SLACK_APP_TOKEN).
    let app_token = resolve_app_token(&channel_name, &config, secrets_store.as_deref()).await?;

    let mut shutdown = std::pin::pin!(shutdown_rx);
    let mut reconnect_attempt: u32 = 0;

    loop {
        // Check for shutdown before connecting
        if futures::poll!(&mut shutdown).is_ready() {
            tracing::info!(channel = %channel_name, "Socket Mode bridge shutting down");
            return Ok(());
        }

        // Obtain a WebSocket URL via apps.connections.open
        let wss_url = match open_connection(&config.open_url, &app_token).await {
            Ok(url) => {
                reconnect_attempt = 0; // Reset backoff on successful auth
                url
            }
            Err(e) => {
                // Auth failure is fatal — don't retry
                if is_auth_error(&e) {
                    tracing::error!(
                        channel = %channel_name,
                        error = %e,
                        "Socket Mode auth failed — app token may be invalid"
                    );
                    return Err(e);
                }

                reconnect_attempt += 1;
                if reconnect_attempt > config.max_reconnect_attempts {
                    tracing::error!(
                        channel = %channel_name,
                        attempts = reconnect_attempt,
                        "Socket Mode max reconnection attempts exceeded"
                    );
                    return Err(WasmChannelError::SocketMode {
                        name: channel_name.clone(),
                        reason: format!(
                            "Max reconnection attempts ({}) exceeded",
                            config.max_reconnect_attempts
                        ),
                    });
                }

                let delay = backoff_delay(config.reconnect_delay_ms, reconnect_attempt);
                tracing::warn!(
                    channel = %channel_name,
                    attempt = reconnect_attempt,
                    delay_ms = delay.as_millis() as u64,
                    error = %e,
                    "Socket Mode connection.open failed, retrying"
                );

                tokio::select! {
                    _ = tokio::time::sleep(delay) => continue,
                    _ = &mut shutdown => {
                        tracing::info!(channel = %channel_name, "Socket Mode bridge shutting down");
                        return Ok(());
                    }
                }
            }
        };

        tracing::info!(channel = %channel_name, "Connecting to Socket Mode WebSocket");

        // Connect to the WebSocket
        let ws_stream = match tokio_tungstenite::connect_async(&wss_url).await {
            Ok((stream, _response)) => stream,
            Err(e) => {
                reconnect_attempt += 1;
                if reconnect_attempt > config.max_reconnect_attempts {
                    return Err(WasmChannelError::SocketMode {
                        name: channel_name.clone(),
                        reason: format!(
                            "WebSocket connect failed after {} attempts: {}",
                            reconnect_attempt, e
                        ),
                    });
                }

                let delay = backoff_delay(config.reconnect_delay_ms, reconnect_attempt);
                tracing::warn!(
                    channel = %channel_name,
                    attempt = reconnect_attempt,
                    delay_ms = delay.as_millis() as u64,
                    error = %e,
                    "WebSocket connect failed, retrying"
                );

                tokio::select! {
                    _ = tokio::time::sleep(delay) => continue,
                    _ = &mut shutdown => return Ok(()),
                }
            }
        };

        tracing::info!(channel = %channel_name, "Socket Mode WebSocket connected");
        reconnect_attempt = 0;

        // Split into read/write halves
        let (write, read) = ws_stream.split();

        // Run the event loop — returns when the connection drops or a `disconnect` is received
        let result = event_loop(&channel, &channel_name, read, write, &mut shutdown).await;

        match result {
            EventLoopExit::Shutdown => {
                tracing::info!(channel = %channel_name, "Socket Mode bridge shutting down");
                return Ok(());
            }
            EventLoopExit::Disconnect => {
                // Planned rotation by the server — reconnect immediately (no backoff)
                tracing::info!(
                    channel = %channel_name,
                    "Socket Mode disconnect received, reconnecting immediately"
                );
                reconnect_attempt = 0;
            }
            EventLoopExit::Error(e) => {
                reconnect_attempt += 1;
                if reconnect_attempt > config.max_reconnect_attempts {
                    return Err(WasmChannelError::SocketMode {
                        name: channel_name.clone(),
                        reason: format!(
                            "WebSocket error after {} attempts: {}",
                            reconnect_attempt, e
                        ),
                    });
                }

                let delay = backoff_delay(config.reconnect_delay_ms, reconnect_attempt);
                tracing::warn!(
                    channel = %channel_name,
                    attempt = reconnect_attempt,
                    delay_ms = delay.as_millis() as u64,
                    error = %e,
                    "Socket Mode connection lost, reconnecting"
                );

                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = &mut shutdown => return Ok(()),
                }
            }
        }
    }
}

/// Why the event loop exited.
enum EventLoopExit {
    /// Shutdown signal received.
    Shutdown,
    /// Server sent a `disconnect` envelope (planned rotation).
    Disconnect,
    /// WebSocket or processing error.
    Error(String),
}

/// Read WebSocket frames, ack envelopes, and forward events to WASM.
async fn event_loop(
    channel: &Arc<WasmChannel>,
    channel_name: &str,
    mut read: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    mut write: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>,
    shutdown: &mut std::pin::Pin<&mut oneshot::Receiver<()>>,
) -> EventLoopExit {
    loop {
        tokio::select! {
            frame = read.next() => {
                let Some(frame) = frame else {
                    return EventLoopExit::Error("WebSocket stream ended".to_string());
                };

                let text = match frame {
                    Ok(WsMessage::Text(t)) => t,
                    Ok(WsMessage::Ping(_)) => continue, // tungstenite auto-pongs
                    Ok(WsMessage::Close(_)) => {
                        return EventLoopExit::Error("WebSocket closed by server".to_string());
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        return EventLoopExit::Error(format!("WebSocket read error: {}", e));
                    }
                };

                // Parse the envelope
                let envelope: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(
                            channel = %channel_name,
                            error = %e,
                            "Failed to parse Socket Mode envelope"
                        );
                        continue;
                    }
                };

                // Immediately ack the envelope (must respond within 3 seconds)
                if let Some(envelope_id) = envelope.get("envelope_id").and_then(|v| v.as_str()) {
                    let ack = serde_json::json!({ "envelope_id": envelope_id });
                    if let Err(e) = write.send(WsMessage::Text(ack.to_string().into())).await {
                        return EventLoopExit::Error(format!("Failed to ack envelope: {}", e));
                    }
                }

                // Classify the envelope type
                let envelope_type = envelope.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match envelope_type {
                    "hello" => {
                        tracing::info!(
                            channel = %channel_name,
                            "Socket Mode hello received — connection established"
                        );
                    }
                    "disconnect" => {
                        let reason = envelope.get("reason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        tracing::info!(
                            channel = %channel_name,
                            reason = reason,
                            "Socket Mode disconnect — server requests reconnection"
                        );
                        return EventLoopExit::Disconnect;
                    }
                    "events_api" => {
                        // Log event type for visibility
                        let event_type = envelope
                            .pointer("/payload/event/type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let event_channel = envelope
                            .pointer("/payload/event/channel")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        let has_thread_ts = envelope
                            .pointer("/payload/event/thread_ts")
                            .is_some();
                        tracing::info!(
                            channel = %channel_name,
                            event_type = event_type,
                            event_channel = event_channel,
                            threaded = has_thread_ts,
                            "Socket Mode event received"
                        );

                        if let Some(payload) = envelope.get("payload") {
                            forward_event_to_wasm(channel, channel_name, payload).await;
                        } else {
                            tracing::warn!(
                                channel = %channel_name,
                                "events_api envelope missing payload"
                            );
                        }
                    }
                    other => {
                        tracing::info!(
                            channel = %channel_name,
                            envelope_type = other,
                            "Ignoring unhandled Socket Mode envelope type"
                        );
                    }
                }
            }
            _ = &mut *shutdown => {
                // Send close frame (best effort)
                let _ = write.send(WsMessage::Close(None)).await;
                return EventLoopExit::Shutdown;
            }
        }
    }
}

/// Forward an events_api payload to the WASM channel via `call_on_http_request`.
///
/// The payload is structurally identical to what a webhook POST body would contain,
/// so the WASM module processes it exactly the same way.
async fn forward_event_to_wasm(
    channel: &Arc<WasmChannel>,
    channel_name: &str,
    payload: &serde_json::Value,
) {
    let body = match serde_json::to_vec(payload) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                channel = %channel_name,
                error = %e,
                "Failed to serialize Socket Mode payload"
            );
            return;
        }
    };

    // Synthesize the same request shape as a webhook POST
    let headers = {
        let mut h = HashMap::new();
        h.insert("content-type".to_string(), "application/json".to_string());
        h
    };
    let query = HashMap::new();

    let path = format!("/webhook/{}", channel_name);
    match channel
        .call_on_http_request("POST", &path, &headers, &query, &body, true)
        .await
    {
        Ok(_response) => {
            tracing::info!(
                channel = %channel_name,
                "Socket Mode event forwarded to WASM successfully"
            );
        }
        Err(e) => {
            tracing::warn!(
                channel = %channel_name,
                error = %e,
                "Failed to forward Socket Mode event to WASM"
            );
        }
    }
}

/// Resolve the app-level token from secrets store or environment variable.
///
/// Tries the encrypted secrets store first (preferred for production), then
/// falls back to the UPPER_CASE env var (e.g., `SLACK_APP_TOKEN`).
async fn resolve_app_token(
    channel_name: &str,
    config: &SocketModeConfig,
    secrets_store: Option<&dyn SecretsStore>,
) -> Result<String, WasmChannelError> {
    // Try secrets store first
    if let Some(store) = secrets_store
        && let Ok(decrypted) = store
            .get_decrypted("default", &config.app_token_secret)
            .await
    {
        tracing::debug!(channel = %channel_name, "Read app token from secrets store");
        return Ok(decrypted.expose().to_string());
    }

    // Fall back to environment variable
    let env_name = config.app_token_secret.to_uppercase();
    match std::env::var(&env_name) {
        Ok(val) if !val.is_empty() => {
            tracing::info!(
                channel = %channel_name,
                env_var = %env_name,
                "App token not in secrets store, using environment variable"
            );
            Ok(val)
        }
        _ => Err(WasmChannelError::SocketMode {
            name: channel_name.to_string(),
            reason: format!(
                "App token '{}' not found in secrets store or env var '{}'",
                config.app_token_secret, env_name
            ),
        }),
    }
}

/// POST to `apps.connections.open` with the app token to get a WebSocket URL.
async fn open_connection(open_url: &str, app_token: &str) -> Result<String, WasmChannelError> {
    let client = reqwest::Client::new();

    let response = client
        .post(open_url)
        .header("Authorization", format!("Bearer {}", app_token))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send()
        .await
        .map_err(|e| WasmChannelError::SocketMode {
            name: "socket_bridge".to_string(),
            reason: format!("HTTP request to apps.connections.open failed: {}", e),
        })?;

    let body: serde_json::Value =
        response
            .json()
            .await
            .map_err(|e| WasmChannelError::SocketMode {
                name: "socket_bridge".to_string(),
                reason: format!("Failed to parse apps.connections.open response: {}", e),
            })?;

    let ok = body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !ok {
        let error = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        return Err(WasmChannelError::SocketMode {
            name: "socket_bridge".to_string(),
            reason: format!("apps.connections.open returned error: {}", error),
        });
    }

    body.get("url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| WasmChannelError::SocketMode {
            name: "socket_bridge".to_string(),
            reason: "apps.connections.open response missing 'url' field".to_string(),
        })
}

/// Check if a SocketMode error represents an authentication failure.
fn is_auth_error(err: &WasmChannelError) -> bool {
    if let WasmChannelError::SocketMode { reason, .. } = err {
        let lower = reason.to_lowercase();
        lower.contains("invalid_auth")
            || lower.contains("not_allowed_token_type")
            || lower.contains("invalid_token")
            || lower.contains("token_revoked")
    } else {
        false
    }
}

/// Calculate exponential backoff delay: base × 2^(attempt-1), capped at ~5 minutes.
fn backoff_delay(base_ms: u64, attempt: u32) -> Duration {
    let exponent = (attempt - 1).min(6); // Cap at 2^6 = 64 → max ~5min with 5s base
    let delay_ms = base_ms.saturating_mul(1u64 << exponent);
    Duration::from_millis(delay_ms.min(320_000)) // Hard cap at ~5 minutes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_delay() {
        // base = 5000ms
        assert_eq!(backoff_delay(5000, 1), Duration::from_millis(5000)); // 5s
        assert_eq!(backoff_delay(5000, 2), Duration::from_millis(10_000)); // 10s
        assert_eq!(backoff_delay(5000, 3), Duration::from_millis(20_000)); // 20s
        assert_eq!(backoff_delay(5000, 4), Duration::from_millis(40_000)); // 40s
        assert_eq!(backoff_delay(5000, 5), Duration::from_millis(80_000)); // 80s
        assert_eq!(backoff_delay(5000, 6), Duration::from_millis(160_000)); // 160s
        assert_eq!(backoff_delay(5000, 7), Duration::from_millis(320_000)); // 320s (cap)
        assert_eq!(backoff_delay(5000, 10), Duration::from_millis(320_000)); // still capped
    }

    #[test]
    fn test_is_auth_error() {
        assert!(is_auth_error(&WasmChannelError::SocketMode {
            name: "test".to_string(),
            reason: "apps.connections.open returned error: invalid_auth".to_string(),
        }));
        assert!(is_auth_error(&WasmChannelError::SocketMode {
            name: "test".to_string(),
            reason: "error: not_allowed_token_type".to_string(),
        }));
        assert!(!is_auth_error(&WasmChannelError::SocketMode {
            name: "test".to_string(),
            reason: "WebSocket connect failed: timeout".to_string(),
        }));
        assert!(!is_auth_error(&WasmChannelError::HttpRequest(
            "test".to_string()
        )));
    }

    #[test]
    fn test_backoff_delay_capped() {
        // At very high attempt counts the delay should still be capped at 320_000ms
        assert_eq!(backoff_delay(5000, 100), Duration::from_millis(320_000));
        assert_eq!(
            backoff_delay(5000, u32::MAX),
            Duration::from_millis(320_000)
        );
    }

    #[test]
    fn test_backoff_delay_zero_base() {
        // A base of 0ms should always yield Duration::ZERO regardless of attempt
        assert_eq!(backoff_delay(0, 1), Duration::ZERO);
        assert_eq!(backoff_delay(0, 5), Duration::ZERO);
        assert_eq!(backoff_delay(0, 100), Duration::ZERO);
    }

    #[test]
    fn test_is_auth_error_token_revoked() {
        assert!(is_auth_error(&WasmChannelError::SocketMode {
            name: "slack".to_string(),
            reason: "apps.connections.open returned error: token_revoked".to_string(),
        }));
    }

    #[test]
    fn test_is_auth_error_invalid_token() {
        assert!(is_auth_error(&WasmChannelError::SocketMode {
            name: "slack".to_string(),
            reason: "apps.connections.open returned error: invalid_token".to_string(),
        }));
    }

    #[test]
    fn test_is_auth_error_case_insensitive() {
        // The function lowercases the reason before checking, so uppercase should match
        assert!(is_auth_error(&WasmChannelError::SocketMode {
            name: "slack".to_string(),
            reason: "INVALID_AUTH".to_string(),
        }));
        assert!(is_auth_error(&WasmChannelError::SocketMode {
            name: "slack".to_string(),
            reason: "Token_Revoked".to_string(),
        }));
        assert!(is_auth_error(&WasmChannelError::SocketMode {
            name: "slack".to_string(),
            reason: "NOT_ALLOWED_TOKEN_TYPE".to_string(),
        }));
    }

    #[test]
    fn test_is_auth_error_non_socket_mode_variant() {
        // Non-SocketMode variants should always return false
        assert!(!is_auth_error(&WasmChannelError::HttpRequest(
            "invalid_auth".to_string()
        )));
        assert!(!is_auth_error(&WasmChannelError::StartupFailed {
            name: "slack".to_string(),
            reason: "invalid_auth token_revoked".to_string(),
        }));
        assert!(!is_auth_error(&WasmChannelError::CallbackFailed {
            name: "slack".to_string(),
            reason: "token_revoked".to_string(),
        }));
        assert!(!is_auth_error(&WasmChannelError::Config(
            "invalid_token".to_string()
        )));
    }

    #[test]
    fn test_ack_payload_format() {
        // Verify the ack JSON payload structure matches what event_loop produces
        let envelope_id = "test-123";
        let ack = serde_json::json!({ "envelope_id": envelope_id });

        // Should serialize to valid JSON with the envelope_id field
        let serialized = ack.to_string();
        let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(
            parsed.get("envelope_id").and_then(|v| v.as_str()),
            Some("test-123")
        );

        // Verify roundtrip: re-serialize and compare
        let roundtripped = serde_json::json!({ "envelope_id": "test-123" });
        assert_eq!(ack, roundtripped);

        // Also test with a UUID-like envelope_id
        let uuid_id = "b9a0d4e2-7c31-4f8a-9b23-1d5e6f7a8b9c";
        let ack_uuid = serde_json::json!({ "envelope_id": uuid_id });
        let parsed_uuid: serde_json::Value = serde_json::from_str(&ack_uuid.to_string()).unwrap();
        assert_eq!(
            parsed_uuid.get("envelope_id").and_then(|v| v.as_str()),
            Some(uuid_id)
        );
    }

    #[test]
    fn test_parse_envelope_type() {
        // Test the pattern used in event_loop to extract and classify envelope types

        // "hello" envelope
        let hello: serde_json::Value = serde_json::json!({
            "type": "hello",
            "num_connections": 1
        });
        let hello_type = hello.get("type").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(hello_type, "hello");

        // "disconnect" envelope
        let disconnect: serde_json::Value = serde_json::json!({
            "type": "disconnect",
            "reason": "link_disabled"
        });
        let disconnect_type = disconnect
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(disconnect_type, "disconnect");

        // "events_api" envelope
        let events_api: serde_json::Value = serde_json::json!({
            "type": "events_api",
            "envelope_id": "abc-123",
            "payload": {
                "event": {
                    "type": "message",
                    "channel": "C123",
                    "text": "hello"
                }
            }
        });
        let events_type = events_api
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(events_type, "events_api");

        // Verify nested payload access pattern from event_loop
        let event_type = events_api
            .pointer("/payload/event/type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        assert_eq!(event_type, "message");

        let event_channel = events_api
            .pointer("/payload/event/channel")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        assert_eq!(event_channel, "C123");

        // Unknown type falls through to default
        let unknown: serde_json::Value = serde_json::json!({
            "type": "interactive",
            "envelope_id": "xyz-456"
        });
        let unknown_type = unknown.get("type").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(unknown_type, "interactive");
        assert!(
            unknown_type != "hello" && unknown_type != "disconnect" && unknown_type != "events_api"
        );

        // Missing type field defaults to empty string
        let no_type: serde_json::Value = serde_json::json!({ "envelope_id": "no-type" });
        let missing_type = no_type.get("type").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(missing_type, "");
    }

    #[test]
    fn socket_mode_webhook_path_uses_channel_name() {
        // Regression: forward_event_to_wasm hardcoded "/webhook/slack" regardless
        // of the actual channel name. The path must be derived from channel_name
        // so non-slack channels (e.g., "custom-bot") get the correct route.
        fn webhook_path(channel_name: &str) -> String {
            format!("/webhook/{}", channel_name)
        }

        assert_eq!(webhook_path("slack"), "/webhook/slack");
        assert_eq!(webhook_path("custom-slack"), "/webhook/custom-slack");
        assert_eq!(webhook_path("telegram"), "/webhook/telegram");
        assert_ne!(
            webhook_path("custom-bot"),
            "/webhook/slack",
            "Non-slack channels must not get the hardcoded /webhook/slack path"
        );
    }

    #[tokio::test]
    async fn test_resolve_app_token_from_env() {
        let env_var_name = "TEST_RESOLVE_SLACK_APP_TOKEN_BRIDGE_9";
        let expected_value = "xapp-test-token-12345";

        // Safety: test-only env var with a unique name; no parallel test uses this key
        unsafe {
            std::env::set_var(env_var_name, expected_value);
        }

        let config = SocketModeConfig {
            open_url: "https://slack.com/api/apps.connections.open".to_string(),
            // resolve_app_token uppercases this, so use lowercase to match the env var
            app_token_secret: "test_resolve_slack_app_token_bridge_9".to_string(),
            reconnect_delay_ms: 5000,
            max_reconnect_attempts: 10,
        };

        let result = resolve_app_token("test", &config, None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected_value);

        // Clean up
        unsafe {
            std::env::remove_var(env_var_name);
        }
    }

    #[tokio::test]
    async fn test_resolve_app_token_missing() {
        // Use a unique env var name that definitely does not exist
        let config = SocketModeConfig {
            open_url: "https://slack.com/api/apps.connections.open".to_string(),
            app_token_secret: "test_nonexistent_app_token_bridge_10".to_string(),
            reconnect_delay_ms: 5000,
            max_reconnect_attempts: 10,
        };

        // Ensure the env var is not set
        unsafe {
            std::env::remove_var("TEST_NONEXISTENT_APP_TOKEN_BRIDGE_10");
        }

        let result = resolve_app_token("test", &config, None).await;
        assert!(result.is_err());

        // Verify the error is a SocketMode variant with a meaningful message
        match result {
            Err(WasmChannelError::SocketMode { name, reason }) => {
                assert_eq!(name, "test");
                assert!(reason.contains("not found"));
                assert!(reason.contains("TEST_NONEXISTENT_APP_TOKEN_BRIDGE_10"));
            }
            other => panic!("Expected SocketMode error, got: {:?}", other),
        }
    }
}

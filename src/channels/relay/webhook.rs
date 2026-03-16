//! Webhook endpoint for receiving events from channel-relay.
//!
//! Channel-relay POSTs `ChannelEvent` JSON to this endpoint instead of
//! using SSE. The handler verifies the HMAC signature and pushes events
//! into an mpsc channel consumed by `RelayChannel`.

use std::sync::Arc;

use axum::{
    Router, body::Bytes, extract::State, http::HeaderMap, response::IntoResponse, routing::post,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::sync::mpsc;

use super::client::ChannelEvent;

type HmacSha256 = Hmac<Sha256>;

/// Maximum allowed age of a callback timestamp (5 minutes).
const MAX_TIMESTAMP_AGE_SECS: i64 = 300;

/// Shared state for the relay webhook endpoint.
#[derive(Clone)]
pub struct RelayWebhookState {
    pub event_tx: mpsc::Sender<ChannelEvent>,
    pub signing_secret: Arc<Vec<u8>>,
}

/// Build an axum Router for the relay webhook endpoint.
pub fn webhook_router(state: RelayWebhookState) -> Router {
    Router::new()
        .route("/relay/events", post(relay_events_handler))
        .with_state(state)
}

async fn relay_events_handler(
    State(state): State<RelayWebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Verify signature
    let signature = match headers
        .get("x-relay-signature")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s.to_string(),
        None => {
            tracing::warn!("relay callback missing X-Relay-Signature header");
            return (axum::http::StatusCode::UNAUTHORIZED, "missing signature").into_response();
        }
    };

    let timestamp = match headers
        .get("x-relay-timestamp")
        .and_then(|v| v.to_str().ok())
    {
        Some(t) => t.to_string(),
        None => {
            tracing::warn!("relay callback missing X-Relay-Timestamp header");
            return (axum::http::StatusCode::UNAUTHORIZED, "missing timestamp").into_response();
        }
    };

    // Check timestamp freshness
    if let Ok(ts) = timestamp.parse::<i64>() {
        let now = chrono::Utc::now().timestamp();
        if (now - ts).abs() > MAX_TIMESTAMP_AGE_SECS {
            tracing::warn!(
                timestamp = ts,
                now = now,
                "relay callback timestamp too old"
            );
            return (axum::http::StatusCode::UNAUTHORIZED, "stale timestamp").into_response();
        }
    }

    // Verify HMAC
    if !verify_signature(&state.signing_secret, &timestamp, &body, &signature) {
        tracing::warn!("relay callback signature verification failed");
        return (axum::http::StatusCode::UNAUTHORIZED, "invalid signature").into_response();
    }

    // Parse event
    let event: ChannelEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "relay callback invalid JSON");
            return (axum::http::StatusCode::BAD_REQUEST, "invalid JSON").into_response();
        }
    };

    tracing::debug!(
        event_type = %event.event_type,
        sender = %event.sender_id,
        channel = %event.channel_id,
        "received relay callback event"
    );

    // Push to channel (non-blocking — if full, log and drop)
    if let Err(e) = state.event_tx.try_send(event) {
        tracing::warn!(error = %e, "relay callback event channel full or closed");
    }

    axum::Json(serde_json::json!({"ok": true})).into_response()
}

/// Verify a relay callback HMAC signature.
pub fn verify_relay_signature(
    secret: &[u8],
    timestamp: &str,
    body: &[u8],
    signature: &str,
) -> bool {
    verify_signature(secret, timestamp, body, signature)
}

fn verify_signature(secret: &[u8], timestamp: &str, body: &[u8], signature: &str) -> bool {
    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), signature.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_signature(secret: &[u8], timestamp: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn verify_valid_signature() {
        let secret = b"test-secret";
        let body = b"hello";
        let ts = "1234567890";
        let sig = make_signature(secret, ts, body);
        assert!(verify_signature(secret, ts, body, &sig));
    }

    #[test]
    fn verify_wrong_secret_fails() {
        let body = b"hello";
        let ts = "1234567890";
        let sig = make_signature(b"correct", ts, body);
        assert!(!verify_signature(b"wrong", ts, body, &sig));
    }

    #[test]
    fn verify_tampered_body_fails() {
        let secret = b"secret";
        let ts = "1234567890";
        let sig = make_signature(secret, ts, b"original");
        assert!(!verify_signature(secret, ts, b"tampered", &sig));
    }
}

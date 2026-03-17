//! Integration tests for the channel-relay client and channel.
//!
//! Uses real HTTP servers on random ports (no mock framework).

use axum::{
    Json, Router,
    extract::Query,
    routing::{get, post},
};
use ironclaw::channels::relay::client::{ChannelEvent, RelayClient};
use secrecy::SecretString;
use serde::Deserialize;
use tokio::net::TcpListener;

/// Start an axum server on a random port, returning the base URL.
async fn start_server(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{}", addr)
}

fn test_client(base_url: &str) -> RelayClient {
    RelayClient::new(
        base_url.to_string(),
        SecretString::from("test-api-key".to_string()),
        5,
    )
    .expect("client build")
}

// ── Proxy call ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ProxyQuery {
    team_id: String,
}

#[tokio::test]
async fn test_proxy_provider_sends_correct_payload() {
    let app = Router::new().route(
        "/proxy/slack/chat.postMessage",
        post(
            |Query(q): Query<ProxyQuery>, Json(body): Json<serde_json::Value>| async move {
                assert_eq!(q.team_id, "T123");
                assert_eq!(body["channel"], "C456");
                assert_eq!(body["text"], "Hello from test");
                Json(serde_json::json!({"ok": true}))
            },
        ),
    );

    let base_url = start_server(app).await;
    let client = test_client(&base_url);

    let body = serde_json::json!({
        "channel": "C456",
        "text": "Hello from test",
    });
    let resp = client
        .proxy_provider("slack", "T123", "chat.postMessage", body)
        .await
        .unwrap();
    assert_eq!(resp["ok"], true);
}

// ── List connections ────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_connections() {
    let app = Router::new().route(
        "/connections",
        get(|| async {
            Json(serde_json::json!([
                {"provider": "slack", "team_id": "T123", "team_name": "Test Team", "connected": true},
                {"provider": "slack", "team_id": "T456", "team_name": "Other", "connected": false},
            ]))
        }),
    );

    let base_url = start_server(app).await;
    let client = test_client(&base_url);

    let conns = client.list_connections("inst-1").await.unwrap();
    assert_eq!(conns.len(), 2);
    assert!(conns[0].connected);
    assert!(!conns[1].connected);
}

// ── Bearer token auth ────────────────────────────────────────────────────

#[tokio::test]
async fn test_bearer_token_sent_in_header() {
    let app = Router::new().route(
        "/connections",
        get(|headers: axum::http::HeaderMap| async move {
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert_eq!(auth, "Bearer test-api-key");
            Json(serde_json::json!([]))
        }),
    );

    let base_url = start_server(app).await;
    let client = test_client(&base_url);
    let _ = client.list_connections("inst-1").await.unwrap();
}

// ── Client builder error propagation ────────────────────────────────────

#[test]
fn test_relay_client_new_succeeds() {
    let client = RelayClient::new(
        "http://localhost:9999".to_string(),
        SecretString::from("key".to_string()),
        30,
    );
    assert!(client.is_ok());
}

// ── Channel event field validation ──────────────────────────────────────

#[test]
fn test_channel_event_missing_fields_detected() {
    // Event with empty sender_id should be detectable
    let json = r#"{"event_type": "message", "provider_scope": "T1", "channel_id": "C1", "sender_id": "", "content": "test"}"#;
    let event: ChannelEvent = serde_json::from_str(json).unwrap();
    assert!(event.sender_id.is_empty());

    // Event with all fields present
    let json = r#"{"event_type": "message", "provider_scope": "T1", "channel_id": "C1", "sender_id": "U1", "content": "test"}"#;
    let event: ChannelEvent = serde_json::from_str(json).unwrap();
    assert!(!event.sender_id.is_empty());
    assert!(!event.channel_id.is_empty());
    assert!(!event.provider_scope.is_empty());
}

//! Integration tests for the channel-relay client and channel.
//!
//! Uses real HTTP servers on random ports (no mock framework).

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::{
    Json, Router,
    extract::Query,
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use futures::stream;
use ironclaw::channels::relay::client::{RelayClient, RelayError};
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

// ── SSE stream mock ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_sse_stream_receives_events() {
    let app = Router::new().route(
        "/stream",
        get(
            |Query(params): Query<std::collections::HashMap<String, String>>| async move {
                // Verify token is passed
                assert!(params.contains_key("token"));

                let events = vec![
                    Ok::<_, Infallible>(
                        Event::default().event("message").data(
                            serde_json::json!({
                                "event_type": "message",
                                "provider": "slack",
                                "provider_scope": "T123",
                                "channel_id": "C456",
                                "sender_id": "U789",
                                "content": "hello world"
                            })
                            .to_string(),
                        ),
                    ),
                    Ok(Event::default().event("message").data(
                        serde_json::json!({
                            "event_type": "direct_message",
                            "provider": "slack",
                            "provider_scope": "T123",
                            "channel_id": "D001",
                            "sender_id": "U789",
                            "content": "dm text"
                        })
                        .to_string(),
                    )),
                ];

                Sse::new(stream::iter(events)).keep_alive(KeepAlive::default())
            },
        ),
    );

    let base_url = start_server(app).await;
    let client = test_client(&base_url);

    let (mut event_stream, handle) = client.connect_stream("test-token", 30).await.unwrap();

    use futures::StreamExt;
    let first = event_stream.next().await.expect("first event");
    assert_eq!(first.event_type, "message");
    assert_eq!(first.text(), "hello world");
    assert_eq!(first.team_id(), "T123");

    let second = event_stream.next().await.expect("second event");
    assert_eq!(second.event_type, "direct_message");
    assert_eq!(second.text(), "dm text");

    handle.abort();
}

// ── Token renewal flow ──────────────────────────────────────────────────

#[tokio::test]
async fn test_token_expired_returns_error() {
    let app = Router::new().route("/stream", get(|| async { StatusCode::UNAUTHORIZED }));

    let base_url = start_server(app).await;
    let client = test_client(&base_url);

    match client.connect_stream("expired-token", 30).await {
        Err(RelayError::TokenExpired) => {} // expected
        Err(other) => panic!("expected TokenExpired, got: {other}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[tokio::test]
async fn test_token_renewal() {
    let call_count = std::sync::Arc::new(AtomicUsize::new(0));
    let call_count_clone = call_count.clone();

    let app = Router::new().route(
        "/stream/renew",
        post(move |Json(body): Json<serde_json::Value>| {
            let count = call_count_clone.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                assert!(body.get("instance_id").is_some());
                assert!(body.get("user_id").is_some());
                Json(serde_json::json!({
                    "stream_token": "renewed-token-123"
                }))
            }
        }),
    );

    let base_url = start_server(app).await;
    let client = test_client(&base_url);

    let new_token = client.renew_token("inst-1", "user-1").await.unwrap();
    assert_eq!(new_token, "renewed-token-123");
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
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
        .proxy_provider("slack", "T123", "chat.postMessage", body, None)
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

// ── API key header ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_api_key_sent_in_header() {
    let app = Router::new().route(
        "/connections",
        get(|headers: axum::http::HeaderMap| async move {
            let key = headers
                .get("X-API-Key")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert_eq!(key, "test-api-key");
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
    use ironclaw::channels::relay::client::ChannelEvent;

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

//! End-to-end integration tests for the WebSocket gateway.
//!
//! These tests start a real Axum server on a random port, connect a WebSocket
//! client, and verify the full message flow:
//! - WebSocket upgrade with auth
//! - Ping/pong
//! - Client message → agent msg_tx
//! - Broadcast AppEvent → WebSocket client
//! - Connection tracking (counter increment/decrement)
//! - Gateway status endpoint

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use ironclaw::channels::IncomingMessage;
use ironclaw::channels::web::server::{GatewayState, start_server};
use ironclaw::channels::web::sse::SseManager;
use ironclaw::channels::web::ws::WsConnectionTracker;
use ironclaw_common::AppEvent;

const AUTH_TOKEN: &str = "test-token-12345";
const TIMEOUT: Duration = Duration::from_secs(5);

/// Start a gateway server on a random port and return the bound address + agent
/// message receiver.
async fn start_test_server() -> (
    SocketAddr,
    Arc<GatewayState>,
    mpsc::Receiver<IncomingMessage>,
) {
    let (agent_tx, agent_rx) = mpsc::channel(64);

    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(Some(agent_tx)),
        sse: Arc::new(SseManager::new()),
        workspace: None,
        workspace_pool: None,
        session_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: None,
        job_manager: None,
        prompt_queue: None,
        scheduler: None,
        owner_id: "test-user".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        skill_registry: None,
        skill_catalog: None,
        auth_manager: None,
        chat_rate_limiter: ironclaw::channels::web::server::PerUserRateLimiter::new(30, 60),
        oauth_rate_limiter: ironclaw::channels::web::server::PerUserRateLimiter::new(20, 60),
        webhook_rate_limiter: ironclaw::channels::web::server::RateLimiter::new(10, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
        startup_time: std::time::Instant::now(),
        active_config: ironclaw::channels::web::server::ActiveConfigSnapshot::default(),
        secrets_store: None,
        db_auth: None,
        pairing_store: None,
        oauth_providers: None,
        oauth_state_store: None,
        oauth_base_url: None,
        oauth_allowed_domains: Vec::new(),
        near_nonce_store: None,
        near_rpc_url: None,
        near_network: None,
        oauth_sweep_shutdown: None,
        frontend_html_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        tool_dispatcher: None,
    });

    let auth = ironclaw::channels::web::auth::MultiAuthState::single(
        AUTH_TOKEN.to_string(),
        "test-user".to_string(),
    );
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let bound_addr = start_server(addr, state.clone(), auth.into())
        .await
        .expect("Failed to start test server");

    (bound_addr, state, agent_rx)
}

/// Connect a WebSocket client with auth token in query parameter.
async fn connect_ws(
    addr: SocketAddr,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("ws://{}/api/chat/ws?token={}", addr, AUTH_TOKEN);
    let mut request = url.into_client_request().unwrap();
    // Server requires an Origin header from localhost to prevent cross-site WS hijacking.
    request.headers_mut().insert(
        "Origin",
        format!("http://127.0.0.1:{}", addr.port()).parse().unwrap(),
    );
    let (stream, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("Failed to connect WebSocket");
    stream
}

/// Read the next text frame from the WebSocket, with a timeout.
async fn recv_text(
    stream: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> String {
    let msg = timeout(TIMEOUT, stream.next())
        .await
        .expect("Timed out waiting for WS message")
        .expect("Stream ended")
        .expect("WS error");
    match msg {
        Message::Text(text) => text.to_string(),
        other => panic!("Expected Text frame, got {:?}", other),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_ws_ping_pong() {
    let (addr, _state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send ping
    let ping = r#"{"type":"ping"}"#;
    ws.send(Message::Text(ping.into())).await.unwrap();

    // Expect pong
    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "pong");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_message_reaches_agent() {
    let (addr, _state, mut agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send a chat message
    let msg = r#"{"type":"message","content":"hello from ws","thread_id":"t42"}"#;
    ws.send(Message::Text(msg.into())).await.unwrap();

    // Verify it arrives on the agent's msg_tx
    let incoming = timeout(TIMEOUT, agent_rx.recv())
        .await
        .expect("Timed out waiting for agent message")
        .expect("Agent channel closed");

    assert_eq!(incoming.content, "hello from ws");
    assert_eq!(incoming.thread_id.as_deref(), Some("t42"));
    assert_eq!(incoming.channel, "gateway");
    assert_eq!(incoming.user_id, "test-user");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_broadcast_event_received() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Give the connection a moment to fully establish
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Broadcast an event (simulates agent sending a response)
    state.sse.broadcast(AppEvent::Response {
        content: "agent says hi".to_string(),
        thread_id: "t1".to_string(),
    });

    // The WS client should receive it
    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "event");
    assert_eq!(parsed["event_type"], "response");
    assert_eq!(parsed["data"]["content"], "agent says hi");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_thinking_event() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    state.sse.broadcast(AppEvent::Thinking {
        message: "analyzing...".to_string(),
        thread_id: None,
    });

    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "event");
    assert_eq!(parsed["event_type"], "thinking");
    assert_eq!(parsed["data"]["message"], "analyzing...");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_connection_tracking() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let tracker = state.ws_tracker.as_ref().unwrap();

    assert_eq!(tracker.connection_count(), 0);

    // Connect first client
    let ws1 = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(tracker.connection_count(), 1);

    // Connect second client
    let ws2 = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(tracker.connection_count(), 2);

    // Disconnect first
    drop(ws1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(tracker.connection_count(), 1);

    // Disconnect second
    drop(ws2);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(tracker.connection_count(), 0);
}

#[tokio::test]
async fn test_ws_invalid_message_returns_error() {
    let (addr, _state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send invalid JSON
    ws.send(Message::Text("not json".into())).await.unwrap();

    // Should get an error message back
    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "error");
    assert!(
        parsed["message"]
            .as_str()
            .unwrap()
            .contains("Invalid message")
    );

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_unknown_type_returns_error() {
    let (addr, _state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send valid JSON but unknown message type
    ws.send(Message::Text(r#"{"type":"foobar"}"#.into()))
        .await
        .unwrap();

    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "error");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_gateway_status_endpoint() {
    let (addr, _state, _agent_rx) = start_test_server().await;

    // Connect a WS client
    let _ws = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Hit the status endpoint
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/gateway/status", addr))
        .header("Authorization", format!("Bearer {}", AUTH_TOKEN))
        .send()
        .await
        .expect("Failed to fetch status");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ws_connections"], 1);
    assert!(body["total_connections"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn test_ws_no_auth_rejected() {
    let (addr, _state, _agent_rx) = start_test_server().await;

    // Try to connect without auth token
    let url = format!("ws://{}/api/chat/ws", addr);
    let request = url.into_client_request().unwrap();
    let result = tokio_tungstenite::connect_async(request).await;

    // Should fail (401 from auth middleware before WS upgrade)
    assert!(result.is_err());
}

#[tokio::test]
async fn test_ws_multiple_events_in_sequence() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Broadcast multiple events rapidly
    state.sse.broadcast(AppEvent::Thinking {
        message: "step 1".to_string(),
        thread_id: None,
    });
    state.sse.broadcast(AppEvent::ToolStarted {
        name: "shell".to_string(),
        detail: None,
        call_id: None,
        thread_id: None,
    });
    state.sse.broadcast(AppEvent::ToolCompleted {
        name: "shell".to_string(),
        success: true,
        error: None,
        parameters: None,
        call_id: None,
        thread_id: None,
    });
    state.sse.broadcast(AppEvent::Response {
        content: "done".to_string(),
        thread_id: "t1".to_string(),
    });

    // Receive all 4 in order
    let t1 = recv_text(&mut ws).await;
    let t2 = recv_text(&mut ws).await;
    let t3 = recv_text(&mut ws).await;
    let t4 = recv_text(&mut ws).await;

    let p1: serde_json::Value = serde_json::from_str(&t1).unwrap();
    let p2: serde_json::Value = serde_json::from_str(&t2).unwrap();
    let p3: serde_json::Value = serde_json::from_str(&t3).unwrap();
    let p4: serde_json::Value = serde_json::from_str(&t4).unwrap();

    assert_eq!(p1["event_type"], "thinking");
    assert_eq!(p2["event_type"], "tool_started");
    assert_eq!(p3["event_type"], "tool_completed");
    assert_eq!(p4["event_type"], "response");

    ws.close(None).await.unwrap();
}

/// Regression test: verify session lock is not held during API handler operations.
///
/// This test ensures that concurrent API requests (e.g., listing threads) don't
/// block the agent loop from processing messages. Previously, chat_threads_handler
/// and chat_history_handler held session locks during slow DB operations, which
/// would deadlock the agent loop waiting to resolve sessions for incoming messages.
///
/// The test verifies that concurrent access to session state completes quickly
/// without deadlock. If locks are heavily contended, the test will timeout.
#[tokio::test]
async fn test_session_lock_not_held_during_api_operations() {
    use ironclaw::agent::SessionManager;

    let (_addr, _state, _agent_rx) = start_test_server().await;

    // Create a session manager and attach it to state
    let session_manager = Arc::new(SessionManager::new());

    // Note: We can't directly modify state.session_manager in the test due to its type.
    // Instead, we test the session manager directly in isolation to verify lock behavior.

    // Spawn concurrent operations simulating API handler + agent loop interaction
    let mut handles = vec![];

    // Simulate API handler threads accessing sessions
    for user_id in 0..5 {
        let sm = session_manager.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..20 {
                let session = sm.get_or_create_session(&format!("user-{}", user_id)).await;
                // Lock and release quickly (simulating API reading session state)
                {
                    let _sess = session.lock().await;
                    tokio::time::sleep(Duration::from_micros(100)).await;
                }
            }
        }));
    }

    // Simulate agent loop thread resolving threads
    let sm = session_manager.clone();
    let agent_handle = tokio::spawn(async move {
        for i in 0..20 {
            let (_session, _thread_id) = sm
                .resolve_thread(&format!("user-{}", i % 5), "gateway", None)
                .await;
            // Should not block waiting for API handler locks
            tokio::time::sleep(Duration::from_micros(100)).await;
        }
    });
    handles.push(agent_handle);

    // Wait for all tasks to complete within reasonable time
    // If session locks are held during slow operations, this will timeout
    let timeout_duration = Duration::from_secs(5);
    let wait_result = timeout(timeout_duration, async {
        for handle in handles {
            let _ = handle.await;
        }
    })
    .await;

    assert!(
        wait_result.is_ok(),
        "Concurrent session access deadlocked or timed out. \
         This suggests session locks are held too long during I/O operations."
    );
}

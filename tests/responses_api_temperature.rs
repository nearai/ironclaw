//! Caller-level regression tests for per-request `temperature` on the
//! Responses API (PR #3641, serrrfirat's Medium-severity follow-up).
//!
//! The handler used to reject any `temperature` field with 400; PR #3641
//! removed that rejection and instead stamps the value into the outgoing
//! `IncomingMessage.metadata` so the agent dispatcher can apply it as a
//! per-request override before consulting user/admin settings.
//!
//! Per `.claude/rules/testing.md` ("Test Through the Caller, Not Just the
//! Helper"), exercising only the dispatcher's `resolve_settings_temperature`
//! helper is not enough — the endpoint→metadata wiring sits between the
//! POST body and the helper, and a future refactor that drops the field
//! on the floor would not break a helper-level test. These tests drive the
//! full router with a captured `msg_tx` and assert that the
//! `IncomingMessage` the agent loop would receive carries the value.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ironclaw::channels::IncomingMessage;
use ironclaw::channels::web::auth::MultiAuthState;
use ironclaw::channels::web::platform::router::start_server;
use ironclaw::channels::web::platform::state::GatewayState;
use ironclaw::channels::web::test_helpers::TestGatewayBuilder;
use tokio::sync::{mpsc, oneshot};

const AUTH_TOKEN: &str = "test-responses-api-temperature-token";
const USER_ID: &str = "test-user";

/// RAII guard that shuts the gateway test server down when dropped.
struct ServerGuard {
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

async fn start_test_server_with_capture() -> (
    SocketAddr,
    Arc<GatewayState>,
    mpsc::Receiver<IncomingMessage>,
    ServerGuard,
) {
    let (tx, rx) = mpsc::channel::<IncomingMessage>(8);
    let state = TestGatewayBuilder::new()
        .user_id(USER_ID)
        .msg_tx(tx)
        .build();
    let auth = MultiAuthState::single(AUTH_TOKEN.to_string(), USER_ID.to_string());
    let addr: SocketAddr = "127.0.0.1:0"
        .parse()
        .expect("hard-coded address must parse");
    let bound = start_server(addr, state.clone(), auth.into())
        .await
        .expect("start gateway test server");
    let shutdown = state.shutdown_tx.write().await.take();
    (bound, state, rx, ServerGuard { shutdown })
}

fn client() -> reqwest::Client {
    // Short timeout — the handler waits for SSE events that never arrive
    // in this test fixture, so the HTTP call always times out from the
    // client side. We only care about the IncomingMessage that lands on
    // `rx` the moment the handler calls `send_to_agent`, which happens
    // long before the SSE wait. The handler task is torn down when the
    // gateway server is dropped at the end of the test.
    reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .expect("build test http client")
}

/// POST `/v1/responses` with `temperature` set must land an
/// `IncomingMessage` on the agent channel whose `metadata["temperature"]`
/// matches the request body. Regression: pre-#3641 the handler 400'd; if
/// a future change drops the `metadata["temperature"]` write, the agent
/// dispatcher's per-request override would never see it and the
/// `resolve_settings_temperature` helper test alone would not notice.
#[tokio::test]
async fn responses_request_temperature_lands_in_incoming_metadata() {
    let (addr, _state, mut rx, _guard) = start_test_server_with_capture().await;
    let url = format!("http://{}/v1/responses", addr);

    let http = client();
    let request = async move {
        // The handler will block waiting for SSE events that never come;
        // we don't care about the response, only the captured message.
        let _ = http
            .post(&url)
            .bearer_auth(AUTH_TOKEN)
            .json(&serde_json::json!({
                "model": "default",
                "input": "hello",
                "temperature": 0.42,
            }))
            .send()
            .await;
    };
    let captured = async {
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("agent channel must receive a message within 2s")
            .expect("agent channel must not be closed")
    };

    let (_, msg) = tokio::join!(request, captured);

    let metadata = &msg.metadata;
    let t = metadata
        .get("temperature")
        .unwrap_or_else(|| panic!("metadata missing 'temperature': {metadata}"));
    let t = t
        .as_f64()
        .unwrap_or_else(|| panic!("metadata 'temperature' not a number: {t}"));
    assert!(
        (t - 0.42).abs() < 1e-6,
        "expected metadata['temperature']=0.42, got {t}"
    );
}

/// POST `/v1/responses` with `temperature` outside the OpenAI-compatible
/// `[0, 2]` range must reject with a 400 `invalid_request_error` at the
/// API boundary and must NOT enqueue an `IncomingMessage` on the agent
/// channel. The provider-side `Reasoning::respond_with_tools` path
/// clamps later, but callers expect the request boundary to fail loudly
/// rather than silently turn `temperature: 9.0` into `2.0`.
#[tokio::test]
async fn responses_request_temperature_out_of_range_rejects_and_does_not_enqueue() {
    let (addr, _state, mut rx, _guard) = start_test_server_with_capture().await;
    let url = format!("http://{}/v1/responses", addr);
    let http = client();

    for bad_temperature in [-0.5_f32, 2.5_f32] {
        let resp = http
            .post(&url)
            .bearer_auth(AUTH_TOKEN)
            .json(&serde_json::json!({
                "model": "default",
                "input": "hello",
                "temperature": bad_temperature,
            }))
            .send()
            .await
            .expect("send /v1/responses request");
        assert_eq!(
            resp.status().as_u16(),
            400,
            "temperature {bad_temperature} must be rejected with 400",
        );
        let body: serde_json::Value = resp.json().await.expect("parse JSON error body");
        let kind = body
            .get("error")
            .and_then(|e| e.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        assert_eq!(
            kind, "invalid_request_error",
            "error.type for bad temperature should be invalid_request_error, body={body}",
        );
    }

    // No `IncomingMessage` may have been enqueued by either rejected request.
    match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
        Ok(Some(msg)) => panic!(
            "no IncomingMessage should be enqueued for rejected temperatures, got: {:?}",
            msg.metadata
        ),
        Ok(None) => panic!("agent channel must not be closed"),
        Err(_) => {} // timeout = nothing enqueued, expected
    }
}

/// POST `/v1/responses` *without* a `temperature` field must not
/// fabricate one in metadata. The dispatcher uses
/// `metadata.get("temperature").is_some()` as the per-request signal —
/// an unconditional default here would override every user's settings
/// value silently.
#[tokio::test]
async fn responses_request_without_temperature_omits_metadata_field() {
    let (addr, _state, mut rx, _guard) = start_test_server_with_capture().await;
    let url = format!("http://{}/v1/responses", addr);

    let http = client();
    let request = async move {
        let _ = http
            .post(&url)
            .bearer_auth(AUTH_TOKEN)
            .json(&serde_json::json!({
                "model": "default",
                "input": "hello",
            }))
            .send()
            .await;
    };
    let captured = async {
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("agent channel must receive a message within 2s")
            .expect("agent channel must not be closed")
    };

    let (_, msg) = tokio::join!(request, captured);
    assert!(
        msg.metadata.get("temperature").is_none(),
        "metadata must not carry a fabricated temperature when the request \
         body has none — got {}",
        msg.metadata
    );
}

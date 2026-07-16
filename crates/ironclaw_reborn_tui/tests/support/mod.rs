//! Shared mock WebChat v2 server fixture for `ironclaw_reborn_tui` client
//! tests. One axum server per test, scripted per-route via `queue`/`queue_sse`.
//! Defined fully here (B1.3); extended (new route scripting only, never
//! signature changes) by later client test files.
//!
//! `dead_code` allowed: this file is compiled once per integration-test
//! binary (`mod support;`), so a field/method only exercised by a later
//! test file (e.g. `queue_sse`/`last_event_id`, used starting in B1.8) is
//! flagged as unused by every earlier binary that doesn't call it.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{OriginalUri, Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

pub(crate) const TEST_TOKEN: &str = "test-token-xyz";

#[derive(Clone)]
pub(crate) struct ScriptedResponse {
    pub(crate) status: u16,
    pub(crate) body: serde_json::Value,
}

impl ScriptedResponse {
    pub(crate) fn ok(body: serde_json::Value) -> Self {
        Self { status: 200, body }
    }

    pub(crate) fn status(status: u16, body: serde_json::Value) -> Self {
        Self { status, body }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) authorization: Option<String>,
    pub(crate) last_event_id: Option<String>,
    pub(crate) body: Option<serde_json::Value>,
}

#[derive(Clone)]
pub(crate) struct SseScriptEvent {
    pub(crate) event: String,
    pub(crate) id: Option<String>,
    pub(crate) data: serde_json::Value,
}

#[derive(Clone, Default)]
pub(crate) struct SseScript {
    pub(crate) events: Vec<SseScriptEvent>,
}

#[derive(Default)]
pub(crate) struct MockState {
    scripts: Mutex<HashMap<String, VecDeque<ScriptedResponse>>>,
    requests: Mutex<Vec<RecordedRequest>>,
    sse_scripts: Mutex<VecDeque<SseScript>>,
}

pub(crate) struct MockServer {
    pub(crate) base_url: String,
    pub(crate) state: Arc<MockState>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl MockServer {
    pub(crate) async fn start() -> Self {
        let state = Arc::new(MockState::default());
        let router = Router::new()
            .fallback(fallback_handler)
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .expect("mock server run");
        });
        Self {
            base_url,
            state,
            shutdown: Some(tx),
        }
    }

    /// Queue one scripted response for `"{METHOD} {axum_pattern}"`, e.g.
    /// `"GET /api/webchat/v2/threads"`. FIFO per key; once the queue for a
    /// key is empty, the last entry repeats.
    pub(crate) fn queue(&self, method_and_pattern: &str, response: ScriptedResponse) {
        self.state
            .scripts
            .lock()
            .expect("scripts lock")
            .entry(method_and_pattern.to_string())
            .or_default()
            .push_back(response);
    }

    /// Queue one scripted SSE connection attempt for the events route.
    pub(crate) fn queue_sse(&self, script: SseScript) {
        self.state
            .sse_scripts
            .lock()
            .expect("sse scripts lock")
            .push_back(script);
    }

    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.state.requests.lock().expect("requests lock").clone()
    }

    pub(crate) fn client(&self) -> ironclaw_reborn_tui::client::ApiClient {
        ironclaw_reborn_tui::client::ApiClient::new(self.base_url.clone(), TEST_TOKEN.to_string())
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

async fn fallback_handler(
    State(state): State<Arc<MockState>>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    request: Request,
) -> Response {
    let path = uri.path().to_string();
    let key = format!("{method} {path}");
    let authorization = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    if path.ends_with("/events") {
        return sse_response(&state, authorization, last_event_id, path);
    }

    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
        .await
        .unwrap_or_default();
    let body: Option<serde_json::Value> = if bytes.is_empty() {
        None
    } else {
        serde_json::from_slice(&bytes).ok()
    };

    state
        .requests
        .lock()
        .expect("requests lock")
        .push(RecordedRequest {
            method: method.to_string(),
            path: path.clone(),
            authorization,
            last_event_id,
            body,
        });

    let mut scripts = state.scripts.lock().expect("scripts lock");
    let queue = scripts.entry(key).or_default();
    let scripted = if queue.len() > 1 {
        queue.pop_front()
    } else {
        queue.front().cloned()
    };
    match scripted {
        Some(scripted) => {
            let status = StatusCode::from_u16(scripted.status).unwrap_or(StatusCode::OK);
            (status, axum::Json(scripted.body)).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "unscripted_route", "path": path})),
        )
            .into_response(),
    }
}

fn sse_response(
    state: &Arc<MockState>,
    authorization: Option<String>,
    last_event_id: Option<String>,
    path: String,
) -> Response {
    state
        .requests
        .lock()
        .expect("requests lock")
        .push(RecordedRequest {
            method: "GET".to_string(),
            path,
            authorization,
            last_event_id,
            body: None,
        });
    let script = state
        .sse_scripts
        .lock()
        .expect("sse scripts lock")
        .pop_front()
        .unwrap_or_default();
    let mut body = String::new();
    for event in script.events {
        body.push_str(&format!("event: {}\n", event.event));
        if let Some(id) = event.id {
            body.push_str(&format!("id: {id}\n"));
        }
        let data = serde_json::to_string(&event.data).expect("serialize sse data");
        body.push_str(&format!("data: {data}\n\n"));
    }
    (
        StatusCode::OK,
        [("content-type", "text/event-stream")],
        body,
    )
        .into_response()
}

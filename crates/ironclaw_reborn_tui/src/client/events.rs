//! SSE subscriber for `/api/webchat/v2/threads/{thread_id}/events`.
//!
//! Line-protocol parser (no external SSE crate — see the crate's pinned
//! dependency list) + `Last-Event-ID` resume + a bounded reconnect budget (3
//! attempts / 60s sliding window), matching the interface contract.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use futures::Stream;
use ironclaw_product_workflow::webchat_schema::WebChatV2EventFrame;

use super::{ApiClient, ClientError};

const RECONNECT_MAX_ATTEMPTS: usize = 3;
const RECONNECT_WINDOW: Duration = Duration::from_secs(60);

struct SubscribeState {
    http: reqwest::Client,
    base_url: String,
    token: String,
    thread_id: String,
    last_event_id: Option<String>,
    reconnect_attempts: VecDeque<Instant>,
    pending: VecDeque<Result<WebChatV2EventFrame, ClientError>>,
    exhausted: bool,
}

pub fn subscribe(
    client: &ApiClient,
    thread_id: &str,
    last_event_id: Option<String>,
) -> impl Stream<Item = Result<WebChatV2EventFrame, ClientError>> + use<> {
    let state = SubscribeState {
        http: client.http.clone(),
        base_url: client.base_url.clone(),
        token: client.token.clone(),
        thread_id: thread_id.to_string(),
        last_event_id,
        reconnect_attempts: VecDeque::new(),
        pending: VecDeque::new(),
        exhausted: false,
    };
    futures::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.pending.pop_front() {
                return Some((item, state));
            }
            if state.exhausted {
                return None;
            }
            match connect_and_drain(&mut state).await {
                Ok(()) => {
                    // Stream ended cleanly (server-side lifetime cap, or the
                    // scripted end in tests) — reconnect if budget allows.
                    if !record_reconnect_attempt(&mut state) {
                        state.exhausted = true;
                        state
                            .pending
                            .push_back(Err(ClientError::ReconnectBudgetExhausted {
                                attempts: RECONNECT_MAX_ATTEMPTS as u8,
                                window_secs: RECONNECT_WINDOW.as_secs(),
                            }));
                    }
                }
                Err(ClientError::Unauthorized) => {
                    state.exhausted = true;
                    state.pending.push_back(Err(ClientError::Unauthorized));
                }
                Err(other) => {
                    state.pending.push_back(Err(other));
                    if !record_reconnect_attempt(&mut state) {
                        state.exhausted = true;
                        state
                            .pending
                            .push_back(Err(ClientError::ReconnectBudgetExhausted {
                                attempts: RECONNECT_MAX_ATTEMPTS as u8,
                                window_secs: RECONNECT_WINDOW.as_secs(),
                            }));
                    }
                }
            }
        }
    })
}

/// Returns `false` when the reconnect budget is exhausted (caller must stop).
fn record_reconnect_attempt(state: &mut SubscribeState) -> bool {
    let now = Instant::now();
    while let Some(front) = state.reconnect_attempts.front() {
        if now.duration_since(*front) > RECONNECT_WINDOW {
            state.reconnect_attempts.pop_front();
        } else {
            break;
        }
    }
    if state.reconnect_attempts.len() >= RECONNECT_MAX_ATTEMPTS {
        return false;
    }
    state.reconnect_attempts.push_back(now);
    true
}

/// Opens one connection, parses every SSE block until the body ends, pushing
/// each parsed frame into `state.pending`. Updates `state.last_event_id` from
/// each block's `id:` line (verbatim) so a subsequent reconnect resumes
/// correctly.
async fn connect_and_drain(state: &mut SubscribeState) -> Result<(), ClientError> {
    use futures::StreamExt;

    let url = format!(
        "{}/api/webchat/v2/threads/{}/events",
        state.base_url.trim_end_matches('/'),
        state.thread_id
    );
    let mut request = state.http.get(&url).bearer_auth(&state.token);
    if let Some(id) = &state.last_event_id {
        request = request.header("last-event-id", id);
    }
    let response = request.send().await?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(ClientError::Unauthorized);
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ClientError::Server {
            status: status.as_u16(),
            body,
        });
    }

    let mut byte_stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut block_event = String::new();
    let mut block_id: Option<String> = None;
    let mut block_data = String::new();

    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(newline) = buffer.find('\n') {
            let line: String = buffer.drain(..=newline).collect();
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !block_data.is_empty() {
                    if let Some(id) = block_id.take() {
                        state.last_event_id = Some(id);
                    }
                    match serde_json::from_str::<WebChatV2EventFrame>(&block_data) {
                        Ok(frame) => state.pending.push_back(Ok(frame)),
                        Err(error) => state
                            .pending
                            .push_back(Err(ClientError::StreamParse(error.to_string()))),
                    }
                }
                block_event.clear();
                block_data.clear();
                continue;
            }
            if let Some(rest) = line.strip_prefix("event:") {
                block_event = rest.trim_start().to_string();
            } else if let Some(rest) = line.strip_prefix("id:") {
                block_id = Some(rest.trim_start().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                if !block_data.is_empty() {
                    block_data.push('\n');
                }
                block_data.push_str(rest.trim_start());
            }
            // `retry:` / comment lines (`:`) intentionally ignored — MVP has
            // no server-advertised retry interval and no comment-based
            // keepalive parsing beyond the blank-line block boundary.
        }
    }
    let _ = &block_event; // reserved for a future per-event-type fast path
    Ok(())
}

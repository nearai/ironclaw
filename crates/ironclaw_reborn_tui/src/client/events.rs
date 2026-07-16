//! SSE subscriber for `/api/webchat/v2/threads/{thread_id}/events`.
//!
//! Line-protocol parser (no external SSE crate — see the crate's pinned
//! dependency list) + `Last-Event-ID` resume + a bounded reconnect budget (3
//! attempts / 60s sliding window), matching the interface contract.
//!
//! Frames are yielded to the caller as soon as they're decoded, not buffered
//! until the connection closes: the real `ironclaw-reborn serve` holds an SSE
//! response open for its full lifetime (`SSE_MAX_LIFETIME`, ~5 minutes), so a
//! reader that only surfaces frames once the HTTP body ends would never
//! return control during a live turn. `OpenConnection` therefore lives in
//! `SubscribeState` across `futures::stream::unfold` polls, and each poll
//! reads at most one chunk off the wire before checking whether that chunk
//! completed a full SSE block.

use std::collections::VecDeque;
use std::pin::Pin;
use std::time::{Duration, Instant};

use futures::Stream;
use ironclaw_product_workflow::webchat_schema::WebChatV2EventFrame;

use super::{ApiClient, ClientError};

const RECONNECT_MAX_ATTEMPTS: usize = 3;
const RECONNECT_WINDOW: Duration = Duration::from_secs(60);

type ByteStream = Pin<Box<dyn Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>;

/// The still-open HTTP response body for one SSE connection attempt, plus
/// the in-progress line/block parser state. Held across `unfold` polls so a
/// chunk read on poll N can be resumed and completed on poll N+1 without
/// re-opening the connection.
struct OpenConnection {
    byte_stream: ByteStream,
    buffer: String,
    block_id: Option<String>,
    block_data: String,
}

struct SubscribeState {
    http: reqwest::Client,
    base_url: String,
    token: String,
    thread_id: String,
    last_event_id: Option<String>,
    reconnect_attempts: VecDeque<Instant>,
    pending: VecDeque<Result<WebChatV2EventFrame, ClientError>>,
    exhausted: bool,
    connection: Option<OpenConnection>,
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
        connection: None,
    };
    futures::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.pending.pop_front() {
                return Some((item, state));
            }
            if state.exhausted {
                return None;
            }
            if state.connection.is_none() {
                match open_connection(&mut state).await {
                    Ok(connection) => {
                        state.connection = Some(connection);
                        continue;
                    }
                    Err(ClientError::Unauthorized) => {
                        state.exhausted = true;
                        state.pending.push_back(Err(ClientError::Unauthorized));
                        continue;
                    }
                    Err(other) => {
                        state.pending.push_back(Err(other));
                        exhaust_or_record_reconnect(&mut state);
                        continue;
                    }
                }
            }

            // Disjoint-field borrow: `read_next_chunk` needs `&mut
            // connection`, `&mut last_event_id`, and `&mut pending`
            // simultaneously, which a `state.connection.as_mut()` method
            // call alone can't express alongside the other two fields.
            let SubscribeState {
                connection,
                last_event_id,
                pending,
                ..
            } = &mut state;
            let conn = connection
                .as_mut()
                .expect("checked Some via the branch above");
            match read_next_chunk(conn, last_event_id, pending).await {
                Ok(true) => {
                    // A chunk was read (it may or may not have completed a
                    // block) — loop back so a newly-pending frame is
                    // returned immediately rather than blocking on another
                    // read.
                    continue;
                }
                Ok(false) => {
                    // Body ended cleanly (server-side lifetime cap, or the
                    // scripted end in tests) — reconnect if budget allows.
                    state.connection = None;
                    exhaust_or_record_reconnect(&mut state);
                }
                Err(error) => {
                    state.connection = None;
                    state.pending.push_back(Err(error));
                    exhaust_or_record_reconnect(&mut state);
                }
            }
        }
    })
}

/// Records one reconnect attempt; if the budget is exhausted, marks `state`
/// exhausted and queues the terminal error. Shared by every place a
/// connection attempt (fresh or reconnect) ends without yielding an
/// unrecoverable error of its own.
fn exhaust_or_record_reconnect(state: &mut SubscribeState) {
    if !record_reconnect_attempt(state) {
        state.exhausted = true;
        state
            .pending
            .push_back(Err(ClientError::ReconnectBudgetExhausted {
                attempts: RECONNECT_MAX_ATTEMPTS as u8,
                window_secs: RECONNECT_WINDOW.as_secs(),
            }));
    }
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

/// Opens one connection and wraps the response body as a boxed byte stream,
/// with fresh (empty) block-parser state. Sends `Last-Event-ID` from the
/// previous connection attempt, if any, so a reconnect resumes correctly.
async fn open_connection(state: &mut SubscribeState) -> Result<OpenConnection, ClientError> {
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

    Ok(OpenConnection {
        byte_stream: Box::pin(response.bytes_stream()),
        buffer: String::new(),
        block_id: None,
        block_data: String::new(),
    })
}

/// Reads exactly one chunk off `connection.byte_stream`, appends it to the
/// line buffer, and parses every complete block the buffer now contains
/// (there can be more than one per chunk), pushing each into `pending` and
/// updating `last_event_id` from the block's `id:` line.
///
/// Returns `Ok(true)` if a chunk was read (the connection may still have
/// more to send), `Ok(false)` if the body ended cleanly (no more chunks).
async fn read_next_chunk(
    connection: &mut OpenConnection,
    last_event_id: &mut Option<String>,
    pending: &mut VecDeque<Result<WebChatV2EventFrame, ClientError>>,
) -> Result<bool, ClientError> {
    use futures::StreamExt;

    let Some(chunk) = connection.byte_stream.next().await else {
        return Ok(false);
    };
    let chunk = chunk?;
    connection.buffer.push_str(&String::from_utf8_lossy(&chunk));

    while let Some(newline) = connection.buffer.find('\n') {
        let line: String = connection.buffer.drain(..=newline).collect();
        let line = line.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            if !connection.block_data.is_empty() {
                if let Some(id) = connection.block_id.take() {
                    *last_event_id = Some(id);
                }
                match serde_json::from_str::<WebChatV2EventFrame>(&connection.block_data) {
                    Ok(frame) => pending.push_back(Ok(frame)),
                    Err(error) => {
                        pending.push_back(Err(ClientError::StreamParse(error.to_string())))
                    }
                }
            }
            connection.block_data.clear();
            continue;
        }
        if line.strip_prefix("event:").is_some() {
            // Event type is currently unused (`event_name()` is derived
            // from the decoded frame's own `type` tag instead) — reserved
            // for a future per-event-type fast path, matching the previous
            // (pre-mid-connection-yield) parser's behavior.
        } else if let Some(rest) = line.strip_prefix("id:") {
            connection.block_id = Some(rest.trim_start().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !connection.block_data.is_empty() {
                connection.block_data.push('\n');
            }
            connection.block_data.push_str(rest.trim_start());
        }
        // `retry:` / comment lines (`:`) intentionally ignored — MVP has
        // no server-advertised retry interval and no comment-based
        // keepalive parsing beyond the blank-line block boundary.
    }
    Ok(true)
}

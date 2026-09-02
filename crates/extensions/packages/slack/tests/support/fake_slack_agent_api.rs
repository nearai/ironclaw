//! An in-crate fake of the Slack Web API surface the reply sink and the
//! delivery half call: a state machine per streaming message
//! (`chat.startStream` → `chat.appendStream` → `chat.stopStream`), session
//! status per (channel, thread) (`agents.sessions.setStatus`), posted
//! messages (`chat.postMessage`), the external upload flow, and the
//! `conversations.replies` read-back of a stream's accumulated text — with
//! fault injection per method.
//!
//! It answers over the restricted-egress seam exactly as the host's egress
//! would (status + body + parsed `Retry-After`), records every request, and
//! lets a test assert exact request bodies and the provider-side state the
//! sink claims to have reached. Shapes follow the documented methods
//! (docs.slack.dev/reference/methods/chat.startStream etc.).

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
};
use ironclaw_host_api::action::NetworkMethod;
use ironclaw_slack_extension::SlackWebApiMethod;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamState {
    Streaming,
    Stopped {
        session_status: String,
    },
    /// The person pressed Slack's stop button: further appends answer
    /// `stopped_by_user`, a stop answers `message_not_in_streaming_state`.
    StoppedByUser,
}

#[derive(Debug, Clone)]
pub struct FakeStream {
    pub channel: String,
    pub thread_ts: Option<String>,
    pub recipient_user_id: Option<String>,
    pub recipient_team_id: Option<String>,
    pub task_display_mode: Option<String>,
    /// Concatenation of every markdown chunk, in order — what
    /// `conversations.replies` reads back as the message `text`.
    pub text: String,
    /// Every `task_update` chunk received, in order.
    pub task_updates: Vec<Value>,
    /// Every `plan_update` chunk received, in order.
    pub plan_updates: Vec<Value>,
    /// Every `blocks` chunk received, in order.
    pub block_chunks: Vec<Value>,
    pub state: StreamState,
    pub append_calls: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCall {
    pub channel_id: Option<String>,
    pub thread_ts: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedMessage {
    pub channel: String,
    pub thread_ts: Option<String>,
    pub text: String,
    pub ts: String,
}

/// One injected failure, consumed by the next call of its method.
#[derive(Debug, Clone)]
pub enum Fault {
    /// HTTP 429 with `Retry-After` and `{"ok":false,"error":"ratelimited"}`.
    RateLimited {
        method: SlackWebApiMethod,
        retry_after: Duration,
    },
    /// HTTP 500 with `internal_error`.
    ServerError { method: SlackWebApiMethod },
    /// The request is applied provider-side, then the transport fails
    /// before the answer arrives.
    TransportAfterAccept { method: SlackWebApiMethod },
    /// The transport fails before the request reaches the provider.
    TransportBeforeAccept { method: SlackWebApiMethod },
    /// `{"ok":false,"error":"<error>"}` with HTTP 200.
    SlackError {
        method: SlackWebApiMethod,
        error: &'static str,
    },
    /// The request is applied provider-side and answered HTTP 200, but the
    /// body is not JSON (a proxy page, a truncated body).
    InvalidBody { method: SlackWebApiMethod },
    /// The request is applied provider-side and answered HTTP 200
    /// `{"ok":true}` with none of the method's documented fields (no
    /// `messages`, no `ts`).
    BareOk { method: SlackWebApiMethod },
}

impl Fault {
    fn method(&self) -> SlackWebApiMethod {
        match self {
            Self::RateLimited { method, .. }
            | Self::ServerError { method }
            | Self::TransportAfterAccept { method }
            | Self::TransportBeforeAccept { method }
            | Self::SlackError { method, .. }
            | Self::InvalidBody { method }
            | Self::BareOk { method } => *method,
        }
    }
}

#[derive(Debug, Clone)]
struct StagedUpload {
    filename: String,
    length: u64,
    shared_to: Option<(String, Option<String>)>,
}

#[derive(Default)]
struct State {
    requests: Vec<RestrictedEgressRequest>,
    /// `conversations.replies` answers the message WITHOUT a `text` field —
    /// Slack's shape for a message rendered only from blocks.
    read_back_omits_text: bool,
    streams: BTreeMap<String, FakeStream>,
    sessions: Vec<SessionCall>,
    posted: Vec<PostedMessage>,
    uploads: BTreeMap<String, StagedUpload>,
    faults: VecDeque<Fault>,
    next_ts: u64,
    next_file: u64,
}

#[derive(Default)]
pub struct FakeSlackAgentApi {
    state: Mutex<State>,
}

impl FakeSlackAgentApi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inject(&self, fault: Fault) {
        self.lock().faults.push_back(fault);
    }

    /// Read-backs answer a found message with no `text` field, as Slack does
    /// for a message rendered only from blocks.
    pub fn omit_read_back_text(&self) {
        self.lock().read_back_omits_text = true;
    }

    /// Slack's stop button: the stream stops answering appends.
    pub fn stop_by_user(&self, ts: &str) {
        if let Some(stream) = self.lock().streams.get_mut(ts) {
            stream.state = StreamState::StoppedByUser;
        }
    }

    pub fn requests(&self) -> Vec<RestrictedEgressRequest> {
        self.lock().requests.clone()
    }

    /// The Slack method names called, in order (`files.slack.com` uploads
    /// appear as `upload`).
    pub fn calls(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|request| match endpoint_for(request) {
                Some(method) => method.name().to_string(),
                None => "upload".to_string(),
            })
            .collect()
    }

    /// Parsed JSON bodies of every call to `method`, in order.
    pub fn bodies(&self, method: SlackWebApiMethod) -> Vec<Value> {
        self.requests()
            .iter()
            .filter(|request| endpoint_for(request) == Some(method))
            .map(|request| {
                serde_json::from_slice(request.body.as_deref().unwrap_or(b"null"))
                    .expect("json body")
            })
            .collect()
    }

    pub fn streams(&self) -> Vec<(String, FakeStream)> {
        self.lock()
            .streams
            .iter()
            .map(|(ts, stream)| (ts.clone(), stream.clone()))
            .collect()
    }

    pub fn stream(&self, ts: &str) -> Option<FakeStream> {
        self.lock().streams.get(ts).cloned()
    }

    pub fn sessions(&self) -> Vec<SessionCall> {
        self.lock().sessions.clone()
    }

    pub fn posted(&self) -> Vec<PostedMessage> {
        self.lock().posted.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn endpoint_for(request: &RestrictedEgressRequest) -> Option<SlackWebApiMethod> {
    let path = request
        .url
        .strip_prefix("https://slack.com")?
        .split('?')
        .next()?;
    SlackWebApiMethod::ALL
        .iter()
        .copied()
        .find(|method| method.path() == path)
}

fn query(request: &RestrictedEgressRequest) -> BTreeMap<String, String> {
    url::Url::parse(&request.url)
        .map(|url| {
            url.query_pairs()
                .map(|(name, value)| (name.into_owned(), value.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

fn body_json(request: &RestrictedEgressRequest) -> Value {
    serde_json::from_slice(request.body.as_deref().unwrap_or(b"{}")).unwrap_or(Value::Null)
}

fn ok(body: Value) -> RestrictedEgressResponse {
    RestrictedEgressResponse {
        status: 200,
        body: serde_json::to_vec(&body).expect("json"),
        retry_after: None,
    }
}

fn slack_error(error: &str) -> RestrictedEgressResponse {
    ok(json!({ "ok": false, "error": error }))
}

fn transport_error(reason: &str) -> RestrictedEgressError {
    RestrictedEgressError::Transport {
        reason: reason.to_string(),
    }
}

#[async_trait]
impl RestrictedEgress for FakeSlackAgentApi {
    async fn send(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        let mut state = self.lock();
        state.requests.push(request.clone());
        let endpoint = endpoint_for(&request);

        let fault = endpoint.and_then(|endpoint| {
            let position = state
                .faults
                .iter()
                .position(|fault| fault.method() == endpoint)?;
            state.faults.remove(position)
        });
        match fault {
            Some(Fault::RateLimited { retry_after, .. }) => {
                return Ok(RestrictedEgressResponse {
                    status: 429,
                    body: br#"{"ok":false,"error":"ratelimited"}"#.to_vec(),
                    retry_after: Some(retry_after),
                });
            }
            Some(Fault::ServerError { .. }) => {
                return Ok(RestrictedEgressResponse {
                    status: 500,
                    body: br#"{"ok":false,"error":"internal_error"}"#.to_vec(),
                    retry_after: None,
                });
            }
            Some(Fault::TransportBeforeAccept { .. }) => {
                return Err(transport_error("connection refused before write"));
            }
            Some(Fault::SlackError { error, .. }) => return Ok(slack_error(error)),
            Some(Fault::TransportAfterAccept { .. }) => {
                let _ = handle(&mut state, &request, endpoint);
                return Err(transport_error("connection reset after write"));
            }
            Some(Fault::InvalidBody { .. }) => {
                let _ = handle(&mut state, &request, endpoint);
                return Ok(RestrictedEgressResponse {
                    status: 200,
                    body: b"<html><body>upstream error</body></html>".to_vec(),
                    retry_after: None,
                });
            }
            Some(Fault::BareOk { .. }) => {
                let _ = handle(&mut state, &request, endpoint);
                return Ok(ok(json!({ "ok": true })));
            }
            None => {}
        }
        handle(&mut state, &request, endpoint)
    }
}

fn handle(
    state: &mut State,
    request: &RestrictedEgressRequest,
    endpoint: Option<SlackWebApiMethod>,
) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
    if request.url.starts_with("https://files.slack.com/upload/") {
        let length = request.body.as_ref().map_or(0, Vec::len);
        return Ok(RestrictedEgressResponse {
            status: 200,
            body: format!("OK - {length}").into_bytes(),
            retry_after: None,
        });
    }
    let Some(endpoint) = endpoint else {
        return Err(RestrictedEgressError::UndeclaredHost {
            host: request.url.clone(),
        });
    };
    if request.method != endpoint.http_method() {
        return Err(RestrictedEgressError::UndeclaredMethod);
    }
    if request.credential.as_ref().map(|handle| handle.as_str()) != Some("slack_bot_token") {
        return Err(RestrictedEgressError::UndeclaredCredential {
            handle: request
                .credential
                .as_ref()
                .map(|handle| handle.as_str().to_string())
                .unwrap_or_default(),
        });
    }
    let body = body_json(request);
    let field = |name: &str| body.get(name).and_then(Value::as_str).map(str::to_string);
    Ok(match endpoint {
        SlackWebApiMethod::AgentsSessionsSetStatus => {
            let Some(status) = field("status") else {
                return Ok(slack_error("invalid_arguments"));
            };
            state.sessions.push(SessionCall {
                channel_id: field("channel_id"),
                thread_ts: field("thread_ts"),
                status: status.clone(),
            });
            ok(json!({ "ok": true, "status": status, "agent_status": status }))
        }
        SlackWebApiMethod::ChatStartStream => {
            let Some(channel) = field("channel") else {
                return Ok(slack_error("channel_not_found"));
            };
            // `recipient_user_id` / `recipient_team_id` are "Required when
            // streaming to channels".
            if !channel.starts_with('D') {
                if field("recipient_user_id").is_none() {
                    return Ok(slack_error("missing_recipient_user_id"));
                }
                if field("recipient_team_id").is_none() {
                    return Ok(slack_error("missing_recipient_team_id"));
                }
            }
            if body.get("markdown_text").is_some() && body.get("chunks").is_some() {
                return Ok(slack_error("cannot_provide_both_markdown_text_and_chunks"));
            }
            state.next_ts += 1;
            let ts = format!("1710000100.{:06}", state.next_ts);
            let mut stream = FakeStream {
                channel: channel.clone(),
                thread_ts: field("thread_ts"),
                recipient_user_id: field("recipient_user_id"),
                recipient_team_id: field("recipient_team_id"),
                task_display_mode: field("task_display_mode"),
                text: String::new(),
                task_updates: Vec::new(),
                plan_updates: Vec::new(),
                block_chunks: Vec::new(),
                state: StreamState::Streaming,
                append_calls: 0,
            };
            if let Err(error) = absorb_chunks(&mut stream, &body) {
                return Ok(slack_error(error));
            }
            state.streams.insert(ts.clone(), stream);
            ok(json!({ "ok": true, "channel": channel, "ts": ts }))
        }
        SlackWebApiMethod::ChatAppendStream => {
            let Some(ts) = field("ts") else {
                return Ok(slack_error("message_not_found"));
            };
            let Some(stream) = state.streams.get_mut(&ts) else {
                return Ok(slack_error("message_not_found"));
            };
            match &stream.state {
                StreamState::Streaming => {}
                StreamState::StoppedByUser => return Ok(slack_error("stopped_by_user")),
                StreamState::Stopped { .. } => {
                    return Ok(slack_error("message_not_in_streaming_state"));
                }
            }
            if body.get("markdown_text").is_none() && body.get("chunks").is_none() {
                return Ok(slack_error("markdown_text_or_chunks_required"));
            }
            stream.append_calls += 1;
            if let Err(error) = absorb_chunks(stream, &body) {
                return Ok(slack_error(error));
            }
            let channel = stream.channel.clone();
            ok(json!({ "ok": true, "channel": channel, "ts": ts }))
        }
        SlackWebApiMethod::ChatStopStream => {
            let Some(ts) = field("ts") else {
                return Ok(slack_error("message_not_found"));
            };
            let Some(stream) = state.streams.get_mut(&ts) else {
                return Ok(slack_error("message_not_found"));
            };
            if stream.state != StreamState::Streaming {
                return Ok(slack_error("message_not_in_streaming_state"));
            }
            if let Err(error) = absorb_chunks(stream, &body) {
                return Ok(slack_error(error));
            }
            let session_status = field("session_status").unwrap_or_else(|| "active".to_string());
            stream.state = StreamState::Stopped {
                session_status: session_status.clone(),
            };
            let channel = stream.channel.clone();
            let text = stream.text.clone();
            state.sessions.push(SessionCall {
                channel_id: Some(channel.clone()),
                thread_ts: state.streams[&ts].thread_ts.clone(),
                status: session_status,
            });
            ok(json!({
                "ok": true,
                "channel": channel,
                "ts": ts,
                "message": { "text": text, "ts": ts, "type": "message", "subtype": "bot_message" }
            }))
        }
        SlackWebApiMethod::ChatPostMessage | SlackWebApiMethod::ChatPostEphemeral => {
            let Some(channel) = field("channel") else {
                return Ok(slack_error("channel_not_found"));
            };
            state.next_ts += 1;
            let ts = format!("1710000200.{:06}", state.next_ts);
            state.posted.push(PostedMessage {
                channel: channel.clone(),
                thread_ts: field("thread_ts"),
                text: field("text").unwrap_or_default(),
                ts: ts.clone(),
            });
            ok(json!({ "ok": true, "channel": channel, "ts": ts }))
        }
        SlackWebApiMethod::ConversationsReplies => {
            let params = query(request);
            let Some(ts) = params.get("ts") else {
                return Ok(slack_error("invalid_arguments"));
            };
            if state.read_back_omits_text {
                if state.streams.contains_key(ts)
                    || state.posted.iter().any(|posted| &posted.ts == ts)
                {
                    return Ok(ok(json!({
                        "ok": true,
                        "messages": [{ "type": "message", "ts": ts }],
                        "has_more": false
                    })));
                }
                return Ok(slack_error("thread_not_found"));
            }
            if let Some(stream) = state.streams.get(ts) {
                ok(json!({
                    "ok": true,
                    "messages": [{ "type": "message", "ts": ts, "text": stream.text }],
                    "has_more": false
                }))
            } else if let Some(posted) = state.posted.iter().find(|posted| &posted.ts == ts) {
                ok(json!({
                    "ok": true,
                    "messages": [{ "type": "message", "ts": ts, "text": posted.text }],
                    "has_more": false
                }))
            } else {
                slack_error("thread_not_found")
            }
        }
        SlackWebApiMethod::ConversationsHistory => ok(json!({ "ok": true, "messages": [] })),
        SlackWebApiMethod::FilesGetUploadUrlExternal => {
            let params = query(request);
            state.next_file += 1;
            let file_id = format!("FAKE{}", state.next_file);
            let length = params
                .get("length")
                .and_then(|length| length.parse::<u64>().ok())
                .unwrap_or_default();
            state.uploads.insert(
                file_id.clone(),
                StagedUpload {
                    filename: params.get("filename").cloned().unwrap_or_default(),
                    length,
                    shared_to: None,
                },
            );
            ok(json!({
                "ok": true,
                "upload_url": format!("https://files.slack.com/upload/v1/{}", state.next_file),
                "file_id": file_id
            }))
        }
        SlackWebApiMethod::FilesCompleteUploadExternal => {
            let channel = field("channel_id");
            let thread_ts = field("thread_ts");
            let mut files = Vec::new();
            for file in body
                .get("files")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(id) = file.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(upload) = state.uploads.get_mut(id) {
                    upload.shared_to = channel.clone().map(|channel| (channel, thread_ts.clone()));
                    files.push(json!({ "id": id, "title": file.get("title") }));
                }
            }
            ok(json!({ "ok": true, "files": files }))
        }
        SlackWebApiMethod::FilesInfo => {
            let params = query(request);
            let Some(upload) = params
                .get("file")
                .and_then(|file| state.uploads.get(file).map(|upload| (file.clone(), upload)))
            else {
                return Ok(slack_error("file_not_found"));
            };
            let (id, upload) = upload;
            let mut file = json!({
                "id": id,
                "name": upload.filename,
                "mimetype": "application/octet-stream",
                "size": upload.length,
            });
            if let Some((channel, thread_ts)) = &upload.shared_to {
                file["channels"] = json!([channel]);
                file["ims"] = json!([channel]);
                file["shares"] = json!({ "public": { channel: [{ "ts": "1710000300.000001", "thread_ts": thread_ts }] } });
            }
            ok(json!({ "ok": true, "file": file }))
        }
        SlackWebApiMethod::ChatDelete
        | SlackWebApiMethod::ConversationsOpen
        | SlackWebApiMethod::ReactionsAdd
        | SlackWebApiMethod::ReactionsRemove => ok(json!({ "ok": true })),
    })
}

/// Fold a request's `markdown_text` / `chunks` into the stream, validating
/// the chunk shapes the sink is allowed to send.
fn absorb_chunks(stream: &mut FakeStream, body: &Value) -> Result<(), &'static str> {
    if let Some(text) = body.get("markdown_text").and_then(Value::as_str) {
        stream.text.push_str(text);
    }
    for chunk in body
        .get("chunks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match chunk.get("type").and_then(Value::as_str) {
            Some("markdown_text") => {
                let Some(text) = chunk.get("text").and_then(Value::as_str) else {
                    return Err("invalid_chunks");
                };
                if text.chars().count() > 12_000 {
                    return Err("invalid_chunks");
                }
                stream.text.push_str(text);
            }
            Some("task_update") => {
                let valid_status = matches!(
                    chunk.get("status").and_then(Value::as_str),
                    Some("in_progress" | "complete" | "error")
                );
                if !valid_status || chunk.get("id").and_then(Value::as_str).is_none() {
                    return Err("invalid_chunks");
                }
                stream.task_updates.push(chunk.clone());
            }
            Some("plan_update") => {
                if chunk.get("title").and_then(Value::as_str).is_none() {
                    return Err("invalid_chunks");
                }
                stream.plan_updates.push(chunk.clone());
            }
            Some("blocks") => {
                if chunk.get("blocks").and_then(Value::as_array).is_none() {
                    return Err("invalid_chunks");
                }
                stream.block_chunks.push(chunk.clone());
            }
            _ => return Err("invalid_chunks"),
        }
    }
    Ok(())
}

/// The `NetworkMethod` an endpoint expects, for tests asserting on requests.
pub fn expected_method(method: SlackWebApiMethod) -> NetworkMethod {
    method.http_method()
}

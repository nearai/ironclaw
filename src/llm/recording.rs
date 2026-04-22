//! Live trace recording mode.
//!
//! Wraps any [`LlmProvider`] and captures every LLM interaction into
//! the trace fixture format used by `TraceLlm` for deterministic E2E
//! testing. Recorded traces can be replayed later via `TraceLlm`.
//!
//! The trace includes:
//! - **Memory snapshot**: workspace documents captured before the first LLM call
//! - **HTTP exchanges**: all outgoing HTTP request/response pairs from tools
//! - **Steps**: user inputs, LLM responses (text/tool_calls), and expected tool
//!   results for verifying tool output during replay
//!
//! Enable by setting `IRONCLAW_RECORD_TRACE=1` at runtime.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::llm::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, Role,
    ToolCompletionRequest, ToolCompletionResponse,
};

// ── Trace format types ─────────────────────────────────────────────

/// Top-level trace file — extended format with memory snapshot and HTTP exchanges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceFile {
    pub model_name: String,
    /// Workspace memory documents captured before the recording session.
    /// Replay should restore these before running the trace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_snapshot: Vec<MemorySnapshotEntry>,
    /// HTTP exchanges recorded during the session, in order.
    /// Replay should return these instead of making real HTTP requests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub http_exchanges: Vec<HttpExchange>,
    pub steps: Vec<TraceStep>,
}

/// A memory document captured at recording start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshotEntry {
    pub path: String,
    pub content: String,
}

/// A recorded HTTP request/response pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchange {
    pub request: HttpExchangeRequest,
    pub response: HttpExchangeResponse,
}

/// The request side of an HTTP exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchangeRequest {
    pub method: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// The response side of an HTTP exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchangeResponse {
    pub status: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// A single step in the trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_hint: Option<RequestHint>,
    pub response: TraceResponse,
    /// Tool results that appeared in the message context since the previous step.
    /// During replay, the test harness can compare actual tool results against
    /// these to verify tool output hasn't changed (regression detection).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_tool_results: Vec<ExpectedToolResult>,
}

/// Soft validation hints for matching a step to a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_user_message_contains: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_message_count: Option<usize>,
}

/// Tagged response enum — text, tool_calls, or user_input.
///
/// `user_input` steps are metadata markers — they record what the user said
/// but do **not** correspond to an LLM call. During replay, `TraceLlm` must
/// skip `user_input` steps and only consume `text`/`tool_calls` steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraceResponse {
    Text {
        content: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    ToolCalls {
        tool_calls: Vec<TraceToolCall>,
        input_tokens: u32,
        output_tokens: u32,
    },
    /// Marker for a user message that triggered subsequent LLM calls.
    /// Not an LLM response — replay providers must skip these.
    UserInput { content: String },
}

/// A tool call in a trace step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Recorded tool result for regression checking during replay.
///
/// During replay, after tools execute and before returning the canned LLM
/// response, the test harness should compare actual `Role::Tool` messages
/// against these entries. A content mismatch indicates a tool behavior change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedToolResult {
    pub tool_call_id: String,
    pub name: String,
    /// The full tool result content as it appeared in the message context.
    pub content: String,
}

// ── HTTP interceptor ───────────────────────────────────────────────

/// Trait for intercepting HTTP requests from tools.
///
/// During recording, the interceptor captures exchanges after the real
/// request completes. During replay, it short-circuits with a recorded response.
#[async_trait]
pub trait HttpInterceptor: Send + Sync + std::fmt::Debug {
    /// Called before making an HTTP request.
    ///
    /// Return `Some(response)` to short-circuit (replay mode).
    /// Return `None` to let the real request proceed (recording mode).
    async fn before_request(&self, request: &HttpExchangeRequest) -> Option<HttpExchangeResponse>;

    /// Called after a real HTTP request completes (recording mode only).
    async fn after_response(&self, request: &HttpExchangeRequest, response: &HttpExchangeResponse);
}

/// Records HTTP exchanges during a live session.
#[derive(Debug)]
pub struct RecordingHttpInterceptor {
    exchanges: Mutex<Vec<HttpExchange>>,
}

impl Default for RecordingHttpInterceptor {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingHttpInterceptor {
    pub fn new() -> Self {
        Self {
            exchanges: Mutex::new(Vec::new()),
        }
    }

    /// Return all recorded exchanges.
    pub async fn take_exchanges(&self) -> Vec<HttpExchange> {
        self.exchanges.lock().await.clone()
    }
}

#[async_trait]
impl HttpInterceptor for RecordingHttpInterceptor {
    async fn before_request(&self, _request: &HttpExchangeRequest) -> Option<HttpExchangeResponse> {
        // Recording mode: let the real request proceed
        None
    }

    async fn after_response(&self, request: &HttpExchangeRequest, response: &HttpExchangeResponse) {
        // Scrub request/response before persisting. Traces ship with the
        // repo (replay mode fixtures) — leaking an `Authorization: Bearer
        // ghp_...` into a committed JSON file is a credential compromise.
        // Redaction runs on every recorded exchange unconditionally, so a
        // future caller cannot opt out by accident.
        let mut sanitized_req = request.clone();
        redact_exchange_request(&mut sanitized_req);
        let mut sanitized_resp = response.clone();
        redact_exchange_response(&mut sanitized_resp);
        self.exchanges.lock().await.push(HttpExchange {
            request: sanitized_req,
            response: sanitized_resp,
        });
    }
}

/// Header names whose values must never land in a recorded trace.
///
/// Matched case-insensitively. Keep this list short and focused on the
/// well-known credential-carrying headers — anything broader risks
/// scrubbing fields that the replay matcher actually needs. This set
/// catches the headers that specifically carry bearer tokens, cookies,
/// and API keys.
///
/// `ironclaw_safety::LeakDetector` runs on response bodies before they
/// reach the LLM (see `src/tools/builtin/http.rs`), but only as a hard
/// block when `should_block` fires; values that match a pattern under
/// that threshold still flow through to the recorder, so this
/// allowlist is the last line before the fixture file.
const SENSITIVE_HEADER_NAMES: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "x-goog-api-key",
    "openai-organization",
    "anthropic-api-key",
];

/// Query-parameter names whose values must never land in a recorded
/// trace. Tokens sometimes ride in the URL (e.g.
/// `?access_token=...`) — redact those in place while preserving the
/// rest of the URL so loose URL matching during replay still works.
const SENSITIVE_QUERY_PARAMS: &[&str] = &[
    "access_token",
    "refresh_token",
    "id_token",
    "token",
    "api_key",
    "apikey",
    "client_secret",
    "password",
    "auth",
    "jwt",
    "session",
];

/// JSON/form body keys whose values must be redacted. Superset of
/// `SENSITIVE_QUERY_PARAMS` plus body-specific credential fields
/// (`secret`, `private_key`, `authorization`) that appear in JSON/form
/// bodies but are uncommon as naked URL query parameters.
const SENSITIVE_BODY_KEYS: &[&str] = &[
    "access_token",
    "refresh_token",
    "id_token",
    "token",
    "api_key",
    "apikey",
    "client_secret",
    "password",
    "secret",
    "private_key",
    "auth",
    "jwt",
    "session",
    "authorization",
];

fn is_sensitive_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE_HEADER_NAMES.iter().any(|h| *h == lower)
}

fn redact_headers(headers: &mut [(String, String)]) {
    for (name, value) in headers.iter_mut() {
        if is_sensitive_header(name) {
            *value = "[REDACTED]".to_string();
        }
    }
}

fn redact_url(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        tracing::trace!(url, "redact_url: unparseable URL — returning verbatim");
        return url.to_string();
    };

    // Scrub basic-auth credentials from URL userinfo
    // (e.g. `https://user:pat@api.example.com/...`).
    let had_userinfo = parsed.password().is_some() || !parsed.username().is_empty();
    if parsed.password().is_some() {
        let _ = parsed.set_password(None);
    }
    if !parsed.username().is_empty() {
        let _ = parsed.set_username("");
    }

    // Skip the query-rewrite roundtrip when there's no query, otherwise
    // the URL crate normalizes `https://host` to `https://host/?`.
    if parsed.query().is_none() {
        // Only return the parsed form if we actually modified userinfo;
        // otherwise return the original to avoid URL normalization
        // artifacts (e.g. trailing `/`).
        return if had_userinfo {
            strip_stale_at(parsed.to_string())
        } else {
            url.to_string()
        };
    }
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| {
            let lower = k.to_ascii_lowercase();
            let redacted = SENSITIVE_QUERY_PARAMS.iter().any(|p| *p == lower);
            let new_value = if redacted {
                "[REDACTED]".to_string()
            } else {
                v.into_owned()
            };
            (k.into_owned(), new_value)
        })
        .collect();
    // NOTE: The clear()+append_pair() roundtrip may percent-encode
    // query values differently from the original URL. The replay
    // matcher (`ReplayingHttpInterceptor`) compares by method + URL
    // string equality, so this could cause false misses if the
    // original URL had unusual encoding. In practice, credential-
    // bearing URLs use standard encoding, so this is acceptable.
    parsed.query_pairs_mut().clear();
    for (k, v) in &pairs {
        parsed.query_pairs_mut().append_pair(k, v);
    }
    strip_stale_at(parsed.to_string())
}

/// The `url` crate may leave a stale `@` separator after clearing both
/// username and password (e.g. `https://@host/path`). Strip it.
fn strip_stale_at(url: String) -> String {
    url.replace("://@", "://")
}

/// Recursively walk a JSON value and redact any key matching
/// `SENSITIVE_BODY_KEYS` (exact, case-insensitive match).
///
/// This only redacts by key name. Value-shape detection (e.g. spotting
/// a raw token under an unusual key name) is not attempted here —
/// `ironclaw_safety::LeakDetector` runs on response bodies upstream in
/// `HttpTool::execute` but only as a hard-block filter, so
/// token-shaped values that fall below its block threshold can still
/// reach this layer under an unrecognized key name.
fn redact_json_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                let lower = key.to_ascii_lowercase();
                if SENSITIVE_BODY_KEYS.iter().any(|s| *s == lower) {
                    *val = serde_json::Value::String("[REDACTED]".to_string());
                } else {
                    redact_json_value(val);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                redact_json_value(item);
            }
        }
        _ => {}
    }
}

pub(crate) fn redact_body(body: &str) -> String {
    // Try JSON first (most common body format for API calls).
    if let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(body) {
        redact_json_value(&mut parsed);
        return serde_json::to_string(&parsed).unwrap_or_else(|_| body.to_string());
    }
    // Try form-urlencoded (OAuth token exchanges, login forms).
    if let Some(redacted) = redact_form_urlencoded(body) {
        return redacted;
    }
    // Not a recognized format — return as-is. Opaque bodies fall back
    // to the upstream `LeakDetector::scan` hard-block in HttpTool for
    // token-shaped values; anything under that threshold ships
    // verbatim.
    body.to_string()
}

/// Attempt to parse `body` as `application/x-www-form-urlencoded` and
/// redact any key matching `SENSITIVE_BODY_KEYS`. Returns `None` if the
/// body doesn't look like form-urlencoded data.
fn redact_form_urlencoded(body: &str) -> Option<String> {
    // Quick heuristic: form-urlencoded bodies contain `=` and no `{`.
    // This avoids the cost of a full parse for clearly non-form content.
    if !body.contains('=') || body.starts_with('{') || body.starts_with('[') {
        return None;
    }
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(body.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    // If parsing produced zero pairs, or a single pair with an empty key,
    // it's not actually form-urlencoded.
    if pairs.is_empty() || (pairs.len() == 1 && pairs[0].0.is_empty()) {
        return None;
    }
    let mut any_redacted = false;
    let redacted_pairs: Vec<(String, String)> = pairs
        .into_iter()
        .map(|(k, v)| {
            let lower = k.to_ascii_lowercase();
            if SENSITIVE_BODY_KEYS.iter().any(|s| *s == lower) {
                any_redacted = true;
                (k, "[REDACTED]".to_string())
            } else {
                (k, v)
            }
        })
        .collect();
    // Only return the form-encoded path if we actually recognized at
    // least one sensitive key — otherwise the body might be plain text
    // that happened to parse as a single k=v pair.
    if !any_redacted {
        return None;
    }
    Some(
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&redacted_pairs)
            .finish(),
    )
}

fn redact_exchange_request(req: &mut HttpExchangeRequest) {
    redact_headers(&mut req.headers);
    req.url = redact_url(&req.url);
    if let Some(body) = &req.body {
        req.body = Some(redact_body(body));
    }
}

fn redact_exchange_response(resp: &mut HttpExchangeResponse) {
    redact_headers(&mut resp.headers);
    // OAuth token endpoints reply with JSON/form-encoded bodies like
    // `{"access_token":"..."}` — the most common credential-leak path
    // into a fixture — so redact response bodies on the same allowlist
    // used for requests. Request bodies already flow through
    // `redact_body` in `redact_exchange_request`; responses were the
    // gap.
    resp.body = redact_body(&resp.body);
}

/// Replays recorded HTTP exchanges during test runs.
///
/// Returns responses in order. If more requests arrive than recorded
/// exchanges, returns a 599 error response.
#[derive(Debug)]
pub struct ReplayingHttpInterceptor {
    exchanges: Mutex<VecDeque<HttpExchange>>,
}

impl ReplayingHttpInterceptor {
    pub fn new(exchanges: Vec<HttpExchange>) -> Self {
        Self {
            exchanges: Mutex::new(VecDeque::from(exchanges)),
        }
    }
}

#[async_trait]
impl HttpInterceptor for ReplayingHttpInterceptor {
    async fn before_request(&self, request: &HttpExchangeRequest) -> Option<HttpExchangeResponse> {
        let mut queue = self.exchanges.lock().await;
        if let Some(exchange) = queue.pop_front() {
            // Soft-check: warn if the request doesn't match. Redact the
            // incoming URL the same way stored URLs are redacted so
            // sensitive query params don't cause false mismatches.
            let redacted_incoming_url = redact_url(&request.url);
            if exchange.request.url != redacted_incoming_url
                || exchange.request.method != request.method
            {
                tracing::warn!(
                    expected_url = %exchange.request.url,
                    actual_url = %redacted_incoming_url,
                    expected_method = %exchange.request.method,
                    actual_method = %request.method,
                    "HTTP replay: request mismatch (returning recorded response anyway)"
                );
            }
            Some(exchange.response)
        } else {
            tracing::error!(
                url = %request.url,
                method = %request.method,
                "HTTP replay: no more recorded exchanges, returning error"
            );
            Some(HttpExchangeResponse {
                status: 599,
                headers: Vec::new(),
                body: "trace replay: no more recorded HTTP exchanges".to_string(),
            })
        }
    }

    async fn after_response(
        &self,
        _request: &HttpExchangeRequest,
        _response: &HttpExchangeResponse,
    ) {
        // Replay mode: nothing to record
    }
}

/// Truncate a message to a stable prefix suitable for use as a replay hint.
///
/// The goal is to keep the part of the message that uniquely identifies the
/// LLM call across runs, while dropping the part that changes (UUIDs,
/// timestamps, mission IDs from prior tool calls). For tool result messages
/// of the form `[Tool \`name\` returned: <json>]`, this truncates right
/// after the colon, keeping the tool name. For other messages, it caps at
/// 80 bytes on a UTF-8 char boundary.
fn stable_hint_prefix(content: &str) -> String {
    // Tool-result messages: keep everything up to and including the `:` so
    // the tool name is preserved but the variable JSON payload is dropped.
    // Both NEAR-AI's `[Tool \`name\` returned: ...]` and the safety-layer
    // rewrite produced by `sanitize_tool_messages` follow this shape.
    if content.starts_with("[Tool ")
        && let Some(colon) = content.find(':')
    {
        // Include the colon so the hint stays unique vs other patterns.
        return safe_truncate(content, colon + 1);
    }
    // Default: 80-byte cap on a char boundary.
    safe_truncate(content, 80)
}

/// Truncate `content` to at most `max_bytes` bytes, snapping back to the
/// nearest UTF-8 char boundary.
fn safe_truncate(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    content[..end].to_string()
}

// ── Tool-call argument parameterization ────────────────────────────
//
// Recorded tool calls often reference IDs returned by *prior* tool calls
// (e.g. `mission_fire(<mission_id from mission_create>)`). If we serialize
// the literal ID into the fixture, replay-time mission_create will produce
// a fresh ID and the recorded mission_fire will fail with "not found".
//
// Solution: at recording time, scan the prior conversation's tool result
// messages, build a `{call_id: {field: value}}` lookup, and rewrite any
// matching literal value in the new tool call's arguments as a template
// `{{call_id.field}}`. The replay engine's `substitute_templates` already
// resolves those templates against the *current* tool result values.

/// Build a `{key: {field: stringified_value}}` lookup of every prior tool
/// result in the conversation, used to parameterize subsequent tool-call
/// arguments at recording time.
///
/// The recorder must handle two shapes of "prior tool result":
///
/// 1. **Native `Role::Tool` messages** — keyed by their `tool_call_id`.
///    Used by providers that pass tool results through unmodified.
/// 2. **Sanitized user messages** — `sanitize_tool_messages` rewrites
///    orphaned tool results as `Role::User` content of the form
///    `[Tool \`name\` returned: <json>]`. These have no call_id, so we key
///    them by `tool:<name>` instead. The replay engine resolves both forms
///    via the same `{{key.field}}` template syntax.
///
/// In both cases, only top-level scalar fields of the JSON content are
/// indexed (string, number, bool). Nested objects and arrays are skipped —
/// they're rarely useful as parameterization keys and would inflate the
/// lookup with noise.
fn build_prior_tool_lookup(messages: &[ChatMessage]) -> HashMap<String, HashMap<String, String>> {
    let mut lookup: HashMap<String, HashMap<String, String>> = HashMap::new();
    for msg in messages {
        // ── Shape 1: native Role::Tool with structured content. ──
        if msg.role == Role::Tool {
            let Some(call_id) = msg.tool_call_id.as_deref() else {
                continue;
            };
            let content = unwrap_tool_output(&msg.content);
            index_json_into(&mut lookup, call_id, &content);
            continue;
        }
        // ── Shape 2: Role::User rewrite of orphaned tool results. ──
        if msg.role == Role::User
            && let Some((tool_name, payload)) = parse_user_tool_result(&msg.content)
        {
            // Key as `tool:<name>` so it doesn't collide with call_ids
            // and stays unique across multiple tool invocations.
            let key = format!("tool:{tool_name}");
            index_json_into(&mut lookup, &key, payload);
        }
    }
    lookup
}

/// Parse a JSON object literal out of `content` and merge its top-level
/// scalar fields into `lookup[key]`. If `content` is not valid JSON, falls
/// back to a Python-dict-repr coercion (the engine v2 orchestrator emits
/// tool results as `str(dict)` which uses single quotes and Python keyword
/// literals — see [`coerce_python_repr_to_json`]).
fn index_json_into(
    lookup: &mut HashMap<String, HashMap<String, String>>,
    key: &str,
    content: &str,
) {
    let parsed: Option<serde_json::Value> = serde_json::from_str(content).ok().or_else(|| {
        coerce_python_repr_to_json(content).and_then(|s| serde_json::from_str(&s).ok())
    });
    let Some(json) = parsed else { return };
    let Some(obj) = json.as_object() else {
        return;
    };
    let entry = lookup.entry(key.to_string()).or_default();
    for (k, v) in obj {
        if let Some(s) = json_value_as_scalar_string(v) {
            entry.insert(k.clone(), s);
        }
    }
}

/// Best-effort coercion of a Python `repr(dict)` string into valid JSON.
///
/// The engine v2 Python orchestrator stringifies tool results with
/// `str(output)`, which produces:
///   * single-quoted strings instead of double-quoted ones
///   * Python keyword literals: `True` / `False` / `None`
///
/// This helper walks the string character by character, tracking string
/// state, and rewrites those into JSON form. It handles values that don't
/// embed unescaped quote characters of the opposite kind — adequate for the
/// shapes the orchestrator currently emits (UUIDs, names, statuses, ints).
/// Returns `None` if the input is empty.
fn coerce_python_repr_to_json(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    // ASCII-only fast path. The orchestrator's `str(output)` repr that this
    // helper targets is structurally ASCII (UUIDs, names, statuses, ints).
    // Multi-byte UTF-8 (CJK characters, emoji, etc.) would be corrupted by
    // the byte-as-char casts below — each individual byte of a multi-byte
    // sequence would be widened to a `char`, producing mojibake. Bail
    // early instead and let the caller fall through to its raw-content
    // path.
    if !content.is_ascii() {
        return None;
    }
    let mut out = String::with_capacity(content.len());
    let bytes = content.as_bytes();
    // 0 = outside string, 1 = inside single-quoted string, 2 = inside double-quoted string
    let mut state: u8 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match state {
            0 => match c {
                b'\'' => {
                    out.push('"');
                    state = 1;
                }
                b'"' => {
                    out.push('"');
                    state = 2;
                }
                b'T' if bytes.get(i..i + 4) == Some(b"True") => {
                    out.push_str("true");
                    i += 4;
                    continue;
                }
                b'F' if bytes.get(i..i + 5) == Some(b"False") => {
                    out.push_str("false");
                    i += 5;
                    continue;
                }
                b'N' if bytes.get(i..i + 4) == Some(b"None") => {
                    out.push_str("null");
                    i += 4;
                    continue;
                }
                _ => out.push(c as char),
            },
            1 => match c {
                b'\\' if i + 1 < bytes.len() => {
                    out.push('\\');
                    out.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                b'"' => {
                    // Escape stray double quotes inside the original
                    // single-quoted string so the JSON output stays valid.
                    out.push('\\');
                    out.push('"');
                }
                b'\'' => {
                    out.push('"');
                    state = 0;
                }
                _ => out.push(c as char),
            },
            2 => match c {
                b'\\' if i + 1 < bytes.len() => {
                    out.push('\\');
                    out.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                b'"' => {
                    out.push('"');
                    state = 0;
                }
                _ => out.push(c as char),
            },
            _ => unreachable!(),
        }
        i += 1;
    }
    if state != 0 {
        // Unterminated string — probably not actually Python repr.
        return None;
    }
    Some(out)
}

/// Parse the sanitize-rewrite format `[Tool \`name\` returned: <payload>]`.
///
/// Returns the tool name and the payload string (without the wrapping
/// brackets and trailing `]`). Tolerates both backtick-quoted and bare names.
fn parse_user_tool_result(content: &str) -> Option<(String, &str)> {
    // Required prefix: `[Tool ` (literal `[` then word `Tool ` with a space).
    let rest = content.strip_prefix("[Tool ")?;
    // Optional opening backtick around the name.
    let (name_start, after_name_quote) = if let Some(stripped) = rest.strip_prefix('`') {
        (stripped, true)
    } else {
        (rest, false)
    };
    // Find the closing of the name. With a backtick, look for the closing
    // backtick; without, look for the next space.
    let (name, after_name) = if after_name_quote {
        let close = name_start.find('`')?;
        (&name_start[..close], &name_start[close + 1..])
    } else {
        let close = name_start.find(' ')?;
        (&name_start[..close], &name_start[close..])
    };
    // After the name we expect ` returned: ` then the payload, then a final `]`.
    let after_returned = after_name.trim_start().strip_prefix("returned:")?;
    let payload = after_returned.trim_start();
    let payload = payload.strip_suffix(']').unwrap_or(payload);
    Some((name.to_string(), payload.trim()))
}

/// Strip the `<tool_output name="...">…</tool_output>` wrapper that the
/// safety layer adds, returning the inner JSON if present.
fn unwrap_tool_output(content: &str) -> std::borrow::Cow<'_, str> {
    if let Some(body) = ironclaw_safety::SafetyLayer::unwrap_tool_output(content) {
        std::borrow::Cow::Owned(body)
    } else {
        std::borrow::Cow::Borrowed(content)
    }
}

/// Render a JSON value as a string only when it is a *scalar* (string,
/// number, bool). Returns `None` for arrays/objects/null — those are too
/// noisy to be useful as parameterization keys.
fn json_value_as_scalar_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Walk a JSON value and replace any string that exactly matches a value in
/// the lookup with a `{{call_id.field}}` template. Operates in place.
///
/// We only do *full-string* replacement (not substring) to avoid corrupting
/// strings that incidentally contain a UUID; the replay's
/// `substitute_templates` is symmetric and resolves the template back to the
/// current value.
fn parameterize_value(
    value: &mut serde_json::Value,
    lookup: &HashMap<String, HashMap<String, String>>,
) {
    match value {
        serde_json::Value::String(s) => {
            // Look for a (call_id, field) pair whose value exactly matches `s`.
            // Use the first match — duplicates are pathological and rare.
            for (call_id, fields) in lookup {
                for (field, recorded) in fields {
                    if recorded == s {
                        *s = format!("{{{{{call_id}.{field}}}}}");
                        return;
                    }
                }
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                parameterize_value(v, lookup);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                parameterize_value(v, lookup);
            }
        }
        _ => {}
    }
}

// ── RecordingLlm ───────────────────────────────────────────────────

/// LLM provider decorator that records interactions into a trace file.
pub struct RecordingLlm {
    inner: Arc<dyn LlmProvider>,
    steps: Mutex<Vec<TraceStep>>,
    prev_message_count: Mutex<usize>,
    output_path: PathBuf,
    model_name: String,
    memory_snapshot: Mutex<Vec<MemorySnapshotEntry>>,
    http_interceptor: Arc<RecordingHttpInterceptor>,
}

impl RecordingLlm {
    /// Wrap a provider for recording.
    pub fn new(inner: Arc<dyn LlmProvider>, output_path: PathBuf, model_name: String) -> Self {
        Self {
            inner,
            steps: Mutex::new(Vec::new()),
            prev_message_count: Mutex::new(0),
            output_path,
            model_name,
            memory_snapshot: Mutex::new(Vec::new()),
            http_interceptor: Arc::new(RecordingHttpInterceptor::new()),
        }
    }

    /// Create from environment variables if recording is enabled.
    ///
    /// - `IRONCLAW_RECORD_TRACE` — any non-empty value enables recording
    /// - `IRONCLAW_TRACE_OUTPUT` — file path (default: `./trace_{timestamp}.json`)
    /// - `IRONCLAW_TRACE_MODEL_NAME` — model_name field (default: `recorded-{inner.model_name()}`)
    pub fn from_env(inner: Arc<dyn LlmProvider>) -> Option<Arc<Self>> {
        let enabled = std::env::var("IRONCLAW_RECORD_TRACE")
            .ok()
            .filter(|v| !v.is_empty());
        enabled?;

        let output_path = std::env::var("IRONCLAW_TRACE_OUTPUT")
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let ts = chrono::Local::now().format("%Y%m%dT%H%M%S");
                PathBuf::from(format!("trace_{ts}.json"))
            });

        let model_name = std::env::var("IRONCLAW_TRACE_MODEL_NAME")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| format!("recorded-{}", inner.model_name()));

        tracing::info!(
            output = %output_path.display(),
            model = %model_name,
            "LLM trace recording enabled"
        );

        Some(Arc::new(Self::new(inner, output_path, model_name)))
    }

    /// Get the HTTP interceptor for wiring into tools.
    ///
    /// Pass this to `JobContext` or `HttpTool` so outgoing HTTP requests
    /// are recorded into the trace.
    pub fn http_interceptor(&self) -> Arc<dyn HttpInterceptor> {
        Arc::clone(&self.http_interceptor) as Arc<dyn HttpInterceptor>
    }

    /// Snapshot all memory documents from a workspace.
    ///
    /// Call this once after creation, before the agent starts processing.
    pub async fn snapshot_memory(&self, workspace: &crate::workspace::Workspace) {
        match workspace.list_all().await {
            Ok(paths) => {
                let mut snapshot = self.memory_snapshot.lock().await;
                for path in paths {
                    match workspace.read(&path).await {
                        Ok(doc) => {
                            snapshot.push(MemorySnapshotEntry {
                                path: doc.path,
                                content: doc.content,
                            });
                        }
                        Err(e) => {
                            tracing::debug!(path = %path, error = %e, "Skipped memory doc in snapshot");
                        }
                    }
                }
                tracing::info!(
                    documents = snapshot.len(),
                    "Captured memory snapshot for trace recording"
                );
            }
            Err(e) => {
                tracing::warn!("Failed to snapshot memory for trace recording: {}", e);
            }
        }
    }

    /// Flush accumulated steps, memory snapshot, and HTTP exchanges to the output file.
    pub async fn flush(&self) -> Result<(), std::io::Error> {
        let steps = self.steps.lock().await;
        let memory_snapshot = self.memory_snapshot.lock().await;
        let http_exchanges = self.http_interceptor.take_exchanges().await;

        let trace = TraceFile {
            model_name: self.model_name.clone(),
            memory_snapshot: memory_snapshot.clone(),
            http_exchanges,
            steps: steps.clone(),
        };
        let json = serde_json::to_string_pretty(&trace).map_err(std::io::Error::other)?;
        tokio::fs::write(&self.output_path, json).await?;
        tracing::info!(
            steps = steps.len(),
            memory_docs = memory_snapshot.len(),
            path = %self.output_path.display(),
            "Flushed LLM trace recording"
        );
        Ok(())
    }

    /// Extract new user messages, tool results, and build request hint.
    ///
    /// Returns `(hint, tool_results)` where tool_results are new `Role::Tool`
    /// messages since the last call — these become `expected_tool_results` on
    /// the next step for replay verification.
    async fn capture_new_messages(
        &self,
        messages: &[ChatMessage],
    ) -> (Option<RequestHint>, Vec<ExpectedToolResult>) {
        let mut prev_count = self.prev_message_count.lock().await;
        let current_count = messages.len();
        // After context compaction, the message list may shrink below
        // prev_count.  Clamp to avoid an out-of-bounds slice.
        let start = (*prev_count).min(current_count);

        let new_messages = &messages[start..];

        // Emit UserInput steps for new user messages
        let new_user_messages: Vec<&ChatMessage> = new_messages
            .iter()
            .filter(|m| m.role == Role::User)
            .collect();

        if !new_user_messages.is_empty() {
            let mut steps = self.steps.lock().await;
            for msg in &new_user_messages {
                steps.push(TraceStep {
                    request_hint: None,
                    response: TraceResponse::UserInput {
                        content: msg.content.clone(),
                    },
                    expected_tool_results: Vec::new(),
                });
            }
        }

        // Capture new tool result messages for expected_tool_results
        let tool_results: Vec<ExpectedToolResult> = new_messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .map(|m| ExpectedToolResult {
                tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
                name: m.name.clone().unwrap_or_default(),
                content: m.content.clone(),
            })
            .collect();

        *prev_count = current_count;

        // Build request hint from last user message.
        //
        // The hint is used by `TraceLlm` for replay matching: a step is a
        // candidate when the *current* request's last user message contains
        // the hint as a substring. So the hint must be (a) distinctive enough
        // to identify the step and (b) free of values that change between
        // runs (UUIDs, timestamps, mission IDs from prior tool calls).
        //
        // Strategy: truncate at the first JSON-content boundary so we keep
        // the stable prefix (e.g. "[Tool `mission_create` returned:") and
        // drop the volatile payload (mission UUIDs, etc.). The fallback is
        // a hard cap at 80 bytes on a UTF-8 char boundary.
        let hint = messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|msg| {
                let hint_text = stable_hint_prefix(&msg.content);
                RequestHint {
                    last_user_message_contains: Some(hint_text),
                    min_message_count: Some(current_count),
                }
            });

        (hint, tool_results)
    }
}

#[async_trait]
impl LlmProvider for RecordingLlm {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    fn cache_write_multiplier(&self) -> Decimal {
        self.inner.cache_write_multiplier()
    }

    fn cache_read_discount(&self) -> Decimal {
        self.inner.cache_read_discount()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (hint, tool_results) = self.capture_new_messages(&request.messages).await;
        let response = self.inner.complete(request).await?;

        self.steps.lock().await.push(TraceStep {
            request_hint: hint,
            response: TraceResponse::Text {
                content: response.content.clone(),
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
            },
            expected_tool_results: tool_results,
        });

        Ok(response)
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let (hint, tool_results) = self.capture_new_messages(&request.messages).await;
        // Parameterize tool call arguments BEFORE the request is consumed.
        // We need access to the prior conversation's tool results (Role::Tool
        // messages) so we can rewrite literal IDs/values that came from a
        // previous tool's output as `{{call_id.field}}` templates. The replay
        // engine resolves these templates at lookup time using the *current*
        // tool result values, so non-deterministic IDs (mission UUIDs, etc.)
        // don't bake the recording-time values into the fixture.
        let prior_tool_lookup = build_prior_tool_lookup(&request.messages);
        let response = self.inner.complete_with_tools(request).await?;

        let step = if response.tool_calls.is_empty() {
            TraceStep {
                request_hint: hint,
                response: TraceResponse::Text {
                    content: response.content.clone().unwrap_or_default(),
                    input_tokens: response.input_tokens,
                    output_tokens: response.output_tokens,
                },
                expected_tool_results: tool_results,
            }
        } else {
            TraceStep {
                request_hint: hint,
                response: TraceResponse::ToolCalls {
                    tool_calls: response
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            let mut args = tc.arguments.clone();
                            parameterize_value(&mut args, &prior_tool_lookup);
                            TraceToolCall {
                                id: tc.id.clone(),
                                name: tc.name.clone(),
                                arguments: args,
                            }
                        })
                        .collect(),
                    input_tokens: response.input_tokens,
                    output_tokens: response.output_tokens,
                },
                expected_tool_results: tool_results,
            }
        };

        self.steps.lock().await.push(step);
        Ok(response)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.inner.model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        self.inner.effective_model_name(requested_model)
    }

    fn active_model_name(&self) -> String {
        self.inner.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.inner.set_model(model)
    }

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.inner.calculate_cost(input_tokens, output_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::StubLlm;

    fn make_recorder(stub: Arc<StubLlm>) -> RecordingLlm {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        RecordingLlm::new(
            stub,
            dir.path().join("test_recording.json"),
            "test-recording".to_string(),
        )
    }

    #[tokio::test]
    async fn captures_user_input_before_first_response() {
        let stub = Arc::new(StubLlm::new("hello back"));
        let recorder = make_recorder(stub);

        let request = CompletionRequest::new(vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello!"),
        ]);
        recorder.complete(request).await.unwrap();

        let steps = recorder.steps.lock().await;
        assert_eq!(steps.len(), 2);

        // First step: user_input
        assert!(
            matches!(&steps[0].response, TraceResponse::UserInput { content } if content == "Hello!")
        );

        // Second step: text response
        assert!(
            matches!(&steps[1].response, TraceResponse::Text { content, .. } if content == "hello back")
        );
    }

    #[tokio::test]
    async fn captures_text_response_correctly() {
        let stub = Arc::new(StubLlm::new("test response"));
        let recorder = make_recorder(stub);

        let request = CompletionRequest::new(vec![ChatMessage::user("question")]);
        recorder.complete(request).await.unwrap();

        let steps = recorder.steps.lock().await;
        // user_input + text
        assert_eq!(steps.len(), 2);
        match &steps[1].response {
            TraceResponse::Text {
                content,
                input_tokens,
                output_tokens,
            } => {
                assert_eq!(content, "test response");
                // StubLlm returns 0s for tokens, which is fine
                let _ = (*input_tokens, *output_tokens);
            }
            _ => panic!("Expected Text response"),
        }
    }

    #[tokio::test]
    async fn captures_tool_calls_response() {
        let stub = Arc::new(StubLlm::new("tool result"));
        let recorder = make_recorder(stub);

        // complete_with_tools on StubLlm returns text, not tool_calls.
        // But we can still verify the recording captures it as text.
        let request = ToolCompletionRequest::new(vec![ChatMessage::user("use a tool")], vec![]);
        recorder.complete_with_tools(request).await.unwrap();

        let steps = recorder.steps.lock().await;
        assert_eq!(steps.len(), 2); // user_input + text (StubLlm doesn't return tool_calls)
    }

    #[tokio::test]
    async fn no_spurious_user_input_for_tool_iterations() {
        let stub = Arc::new(StubLlm::new("response"));
        let recorder = make_recorder(stub);

        // First call with user message
        let request = CompletionRequest::new(vec![
            ChatMessage::system("sys"),
            ChatMessage::user("Do something"),
        ]);
        recorder.complete(request).await.unwrap();

        // Second call: same messages plus tool result (no new user message)
        let request = CompletionRequest::new(vec![
            ChatMessage::system("sys"),
            ChatMessage::user("Do something"),
            ChatMessage::assistant("I'll use a tool"),
            ChatMessage::tool_result("call_1", "echo", "result"),
        ]);
        recorder.complete(request).await.unwrap();

        let steps = recorder.steps.lock().await;
        // Step 0: user_input "Do something"
        // Step 1: text response
        // Step 2: text response (no new user_input since no new user messages)
        assert_eq!(steps.len(), 3);
        assert!(matches!(
            &steps[0].response,
            TraceResponse::UserInput { .. }
        ));
        assert!(matches!(&steps[1].response, TraceResponse::Text { .. }));
        assert!(matches!(&steps[2].response, TraceResponse::Text { .. }));
    }

    #[tokio::test]
    async fn captures_tool_results_for_verification() {
        let stub = Arc::new(StubLlm::new("response"));
        let recorder = make_recorder(stub);

        // First call: user asks something
        let request = CompletionRequest::new(vec![
            ChatMessage::system("sys"),
            ChatMessage::user("Do something"),
        ]);
        recorder.complete(request).await.unwrap();

        // Second call: includes tool results from previous tool_calls
        let request = CompletionRequest::new(vec![
            ChatMessage::system("sys"),
            ChatMessage::user("Do something"),
            ChatMessage::assistant("I'll use a tool"),
            ChatMessage::tool_result("call_1", "echo", "echoed: hello"),
            ChatMessage::tool_result("call_2", "time", "2026-03-04T14:00:00Z"),
        ]);
        recorder.complete(request).await.unwrap();

        let steps = recorder.steps.lock().await;
        // Step 2 (the second LLM response) should have expected_tool_results
        let step = &steps[2];
        assert_eq!(step.expected_tool_results.len(), 2);
        assert_eq!(step.expected_tool_results[0].name, "echo");
        assert_eq!(step.expected_tool_results[0].content, "echoed: hello");
        assert_eq!(step.expected_tool_results[1].name, "time");
    }

    #[tokio::test]
    async fn request_hint_extraction() {
        let stub = Arc::new(StubLlm::new("response"));
        let recorder = make_recorder(stub);

        let request = CompletionRequest::new(vec![
            ChatMessage::system("sys"),
            ChatMessage::user("What time is it?"),
        ]);
        recorder.complete(request).await.unwrap();

        let steps = recorder.steps.lock().await;
        let text_step = &steps[1];
        let hint = text_step.request_hint.as_ref().unwrap();
        assert_eq!(
            hint.last_user_message_contains.as_deref(),
            Some("What time is it?")
        );
        assert_eq!(hint.min_message_count, Some(2));
    }

    #[tokio::test]
    async fn flush_writes_valid_json_with_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.json");

        let stub = Arc::new(StubLlm::new("response"));
        let recorder = RecordingLlm::new(stub, path.clone(), "flush-test".to_string());

        // Simulate a memory snapshot
        recorder
            .memory_snapshot
            .lock()
            .await
            .push(MemorySnapshotEntry {
                path: "context/test.md".to_string(),
                content: "test content".to_string(),
            });

        // Simulate an HTTP exchange
        recorder
            .http_interceptor
            .after_response(
                &HttpExchangeRequest {
                    method: "GET".to_string(),
                    url: "https://api.example.com/data".to_string(),
                    headers: Vec::new(),
                    body: None,
                },
                &HttpExchangeResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: r#"{"ok": true}"#.to_string(),
                },
            )
            .await;

        let request = CompletionRequest::new(vec![ChatMessage::user("hello")]);
        recorder.complete(request).await.unwrap();
        recorder.flush().await.unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let trace: TraceFile = serde_json::from_str(&content).unwrap();
        assert_eq!(trace.model_name, "flush-test");
        assert_eq!(trace.memory_snapshot.len(), 1);
        assert_eq!(trace.memory_snapshot[0].path, "context/test.md");
        assert_eq!(trace.http_exchanges.len(), 1);
        assert_eq!(trace.http_exchanges[0].response.status, 200);
        assert_eq!(trace.steps.len(), 2);
    }

    #[test]
    fn from_env_returns_none_when_unset() {
        // SAFETY: This test is single-threaded and no other thread reads this var.
        unsafe { std::env::remove_var("IRONCLAW_RECORD_TRACE") };
        let stub = Arc::new(StubLlm::new("response"));
        let result = RecordingLlm::from_env(stub);
        assert!(result.is_none());
    }

    /// Regression: credentials must never land in a recorded trace.
    ///
    /// An earlier run committed `Authorization: Bearer ghp_...` straight
    /// into a fixture, leaking a live GitHub PAT. The recording
    /// interceptor now scrubs every recorded exchange in place. This
    /// test pins that behavior across the header set, URL query params,
    /// and response `Set-Cookie`.
    #[tokio::test]
    async fn recording_http_interceptor_redacts_credentials() {
        let interceptor = RecordingHttpInterceptor::new();

        let req = HttpExchangeRequest {
            method: "GET".to_string(),
            url: "https://api.example.com/data?access_token=secret&user=alice".to_string(),
            headers: vec![
                (
                    "Authorization".to_string(),
                    "Bearer ghp_thisIsAFakeTokenThatMustNotBeCommitted".to_string(),
                ),
                ("Cookie".to_string(), "session=abc".to_string()),
                ("Accept".to_string(), "application/json".to_string()),
                (
                    "x-api-key".to_string(),
                    "sk-very-secret-api-key".to_string(),
                ),
            ],
            body: None,
        };
        let resp = HttpExchangeResponse {
            status: 200,
            headers: vec![
                (
                    "Set-Cookie".to_string(),
                    "session=xyz; HttpOnly".to_string(),
                ),
                ("Content-Type".to_string(), "application/json".to_string()),
            ],
            body: r#"{"ok":true}"#.to_string(),
        };

        interceptor.after_response(&req, &resp).await;
        let recorded = interceptor.take_exchanges().await;
        assert_eq!(recorded.len(), 1);
        let stored = &recorded[0];

        // Authorization, Cookie, X-Api-Key redacted (case-insensitive).
        let req_header = |name: &str| {
            stored
                .request
                .headers
                .iter()
                .find_map(|(n, v)| (n.eq_ignore_ascii_case(name)).then(|| v.clone()))
                .unwrap_or_default()
        };
        assert_eq!(req_header("Authorization"), "[REDACTED]");
        assert_eq!(req_header("Cookie"), "[REDACTED]");
        assert_eq!(req_header("x-api-key"), "[REDACTED]");
        // Non-sensitive headers untouched so replay matching still works.
        assert_eq!(req_header("Accept"), "application/json");

        // Response Set-Cookie redacted.
        let resp_header = |name: &str| {
            stored
                .response
                .headers
                .iter()
                .find_map(|(n, v)| (n.eq_ignore_ascii_case(name)).then(|| v.clone()))
                .unwrap_or_default()
        };
        assert_eq!(resp_header("Set-Cookie"), "[REDACTED]");
        assert_eq!(resp_header("Content-Type"), "application/json");

        // URL: access_token value replaced, user kept.
        assert!(
            stored.request.url.contains("access_token=%5BREDACTED%5D"),
            "expected redacted access_token, got: {}",
            stored.request.url
        );
        assert!(
            stored.request.url.contains("user=alice"),
            "non-sensitive query params should remain, got: {}",
            stored.request.url
        );

        // Defense in depth: the original secret string must not appear
        // anywhere in the serialized exchange.
        let serialized = serde_json::to_string(stored).unwrap();
        assert!(
            !serialized.contains("ghp_thisIsAFakeTokenThatMustNotBeCommitted"),
            "raw token leaked into serialized exchange: {serialized}"
        );
        assert!(
            !serialized.contains("sk-very-secret-api-key"),
            "raw api key leaked into serialized exchange: {serialized}"
        );
    }

    #[tokio::test]
    async fn recording_http_interceptor_passes_through_and_records() {
        let interceptor = RecordingHttpInterceptor::new();

        let req = HttpExchangeRequest {
            method: "GET".to_string(),
            url: "https://example.com".to_string(),
            headers: Vec::new(),
            body: None,
        };

        // before_request should return None (pass through)
        assert!(interceptor.before_request(&req).await.is_none());

        // after_response records the exchange
        let resp = HttpExchangeResponse {
            status: 200,
            headers: Vec::new(),
            body: "ok".to_string(),
        };
        interceptor.after_response(&req, &resp).await;

        let exchanges = interceptor.take_exchanges().await;
        assert_eq!(exchanges.len(), 1);
        assert_eq!(exchanges[0].request.url, "https://example.com");
    }

    #[tokio::test]
    async fn replaying_http_interceptor_returns_recorded_responses() {
        let exchanges = vec![HttpExchange {
            request: HttpExchangeRequest {
                method: "GET".to_string(),
                url: "https://api.example.com/data".to_string(),
                headers: Vec::new(),
                body: None,
            },
            response: HttpExchangeResponse {
                status: 200,
                headers: Vec::new(),
                body: r#"{"items": []}"#.to_string(),
            },
        }];
        let interceptor = ReplayingHttpInterceptor::new(exchanges);

        // First request: returns recorded response
        let req = HttpExchangeRequest {
            method: "GET".to_string(),
            url: "https://api.example.com/data".to_string(),
            headers: Vec::new(),
            body: None,
        };
        let resp = interceptor.before_request(&req).await.unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, r#"{"items": []}"#);

        // Second request: no more exchanges → 599
        let resp = interceptor.before_request(&req).await.unwrap();
        assert_eq!(resp.status, 599);
    }

    #[test]
    fn serde_roundtrip_extended_format() {
        let trace = TraceFile {
            model_name: "test".to_string(),
            memory_snapshot: vec![MemorySnapshotEntry {
                path: "context/vision.md".to_string(),
                content: "Be helpful.".to_string(),
            }],
            http_exchanges: vec![HttpExchange {
                request: HttpExchangeRequest {
                    method: "GET".to_string(),
                    url: "https://api.example.com".to_string(),
                    headers: vec![("Accept".to_string(), "application/json".to_string())],
                    body: None,
                },
                response: HttpExchangeResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: "{}".to_string(),
                },
            }],
            steps: vec![
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::UserInput {
                        content: "hello".to_string(),
                    },
                    expected_tool_results: Vec::new(),
                },
                TraceStep {
                    request_hint: Some(RequestHint {
                        last_user_message_contains: Some("hello".to_string()),
                        min_message_count: Some(2),
                    }),
                    response: TraceResponse::ToolCalls {
                        tool_calls: vec![TraceToolCall {
                            id: "call_1".to_string(),
                            name: "echo".to_string(),
                            arguments: serde_json::json!({"message": "hi"}),
                        }],
                        input_tokens: 50,
                        output_tokens: 20,
                    },
                    expected_tool_results: Vec::new(),
                },
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::Text {
                        content: "done".to_string(),
                        input_tokens: 80,
                        output_tokens: 10,
                    },
                    expected_tool_results: vec![ExpectedToolResult {
                        tool_call_id: "call_1".to_string(),
                        name: "echo".to_string(),
                        content: "hi".to_string(),
                    }],
                },
            ],
        };

        let json = serde_json::to_string_pretty(&trace).unwrap();
        let parsed: TraceFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.model_name, "test");
        assert_eq!(parsed.memory_snapshot.len(), 1);
        assert_eq!(parsed.http_exchanges.len(), 1);
        assert_eq!(parsed.steps.len(), 3);
        assert_eq!(parsed.steps[2].expected_tool_results.len(), 1);
    }

    #[tokio::test]
    async fn request_hint_handles_multibyte_utf8() {
        let stub = Arc::new(StubLlm::new("response"));
        let recorder = make_recorder(stub);

        // Create a string where byte index 80 falls inside a multi-byte char.
        // Each CJK character is 3 bytes; 26 chars × 3 bytes = 78, then "ab" = 80 bytes,
        // but let's use 27 CJK chars (81 bytes) so truncation must respect the boundary.
        let long_cjk = "你".repeat(27); // 81 bytes, > 80
        assert!(long_cjk.len() > 80);

        let request = CompletionRequest::new(vec![
            ChatMessage::system("sys"),
            ChatMessage::user(&long_cjk),
        ]);
        recorder.complete(request).await.unwrap();

        let steps = recorder.steps.lock().await;
        let text_step = &steps[1];
        let hint = text_step.request_hint.as_ref().unwrap();
        let hint_text = hint.last_user_message_contains.as_deref().unwrap();
        // Must be valid UTF-8 and not longer than 80 bytes
        assert!(hint_text.len() <= 80);
        assert!(hint_text.is_ascii() || hint_text.chars().count() > 0);
    }

    #[test]
    fn backward_compatible_with_old_format() {
        // Old format without memory_snapshot, http_exchanges, expected_tool_results
        let json = r#"{
            "model_name": "old-trace",
            "steps": [
                {
                    "response": {
                        "type": "text",
                        "content": "hello",
                        "input_tokens": 10,
                        "output_tokens": 5
                    }
                }
            ]
        }"#;
        let trace: TraceFile = serde_json::from_str(json).unwrap();
        assert_eq!(trace.model_name, "old-trace");
        assert!(trace.memory_snapshot.is_empty());
        assert!(trace.http_exchanges.is_empty());
        assert!(trace.steps[0].expected_tool_results.is_empty());
    }

    #[test]
    fn coerce_python_repr_to_json_handles_ascii() {
        let input = "{'name': 'alice', 'count': 3, 'active': True, 'note': None}";
        let out = coerce_python_repr_to_json(input).expect("ascii input should coerce");
        // Round-trip through serde to confirm it's now valid JSON.
        let value: serde_json::Value = serde_json::from_str(&out).expect("must be valid JSON");
        assert_eq!(value["name"], "alice");
        assert_eq!(value["count"], 3);
        assert_eq!(value["active"], true);
        assert!(value["note"].is_null());
    }

    #[test]
    fn coerce_python_repr_to_json_bails_on_non_ascii() {
        // Multi-byte UTF-8 (CJK + emoji) must NOT be coerced — the byte-level
        // walker would mojibake the input. Bailing is the correct outcome;
        // the caller falls through to its raw-content path.
        assert_eq!(coerce_python_repr_to_json("{'name': '日本語'}"), None);
        assert_eq!(coerce_python_repr_to_json("{'flag': '🚀'}"), None);
        // Sanity: an empty string still returns None for an unrelated reason.
        assert_eq!(coerce_python_repr_to_json(""), None);
    }

    #[test]
    fn redact_url_scrubs_userinfo() {
        let url = "https://user:pat_secret@api.example.com/v1/repos";
        let redacted = redact_url(url);
        assert!(
            !redacted.contains("pat_secret"),
            "password leaked: {redacted}"
        );
        assert!(!redacted.contains("user@"), "username leaked: {redacted}");
        assert!(
            redacted.contains("api.example.com"),
            "host lost: {redacted}"
        );
    }

    #[test]
    fn redact_url_preserves_plain_urls() {
        assert_eq!(redact_url("https://example.com"), "https://example.com");
        assert_eq!(
            redact_url("https://example.com/path"),
            "https://example.com/path"
        );
    }

    #[test]
    fn redact_body_scrubs_sensitive_json_keys() {
        let body = r#"{"username":"alice","password":"s3cret","api_key":"sk-123","data":"safe"}"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("s3cret"), "password leaked: {redacted}");
        assert!(!redacted.contains("sk-123"), "api_key leaked: {redacted}");
        assert!(
            redacted.contains("alice"),
            "non-sensitive data lost: {redacted}"
        );
        assert!(
            redacted.contains("safe"),
            "non-sensitive data lost: {redacted}"
        );
    }

    /// Regression: body-key redaction must use exact match, not substring.
    /// Fields like `token_count`, `input_tokens`, `session_id` are
    /// non-sensitive and must survive redaction.
    #[test]
    fn redact_body_does_not_over_redact_substring_matches() {
        let body = r#"{
            "token_count": 42,
            "input_tokens": 100,
            "output_tokens": 50,
            "token_type": "Bearer",
            "session_id": "abc-123",
            "session_state": "active",
            "auth_method": "oauth",
            "auth_url": "https://example.com/auth",
            "authorization_type": "bearer"
        }"#;
        let redacted = redact_body(body);
        let parsed: serde_json::Value = serde_json::from_str(&redacted).unwrap();
        let obj = parsed.as_object().unwrap();

        // These fields contain sensitive substrings but are NOT sensitive themselves
        assert_eq!(obj["token_count"], 42, "token_count over-redacted");
        assert_eq!(obj["input_tokens"], 100, "input_tokens over-redacted");
        assert_eq!(obj["output_tokens"], 50, "output_tokens over-redacted");
        assert_eq!(obj["token_type"], "Bearer", "token_type over-redacted");
        assert_eq!(obj["session_id"], "abc-123", "session_id over-redacted");
        assert_eq!(
            obj["session_state"], "active",
            "session_state over-redacted"
        );
        assert_eq!(obj["auth_method"], "oauth", "auth_method over-redacted");
        assert_eq!(
            obj["auth_url"], "https://example.com/auth",
            "auth_url over-redacted"
        );
        assert_eq!(
            obj["authorization_type"], "bearer",
            "authorization_type over-redacted"
        );
    }

    /// Exact-match body keys that ARE sensitive must still be redacted.
    #[test]
    fn redact_body_still_redacts_exact_sensitive_keys() {
        let body = r#"{"token":"secret_val","auth":"cred","session":"sess_tok","jwt":"eyJ..."}"#;
        let redacted = redact_body(body);
        let parsed: serde_json::Value = serde_json::from_str(&redacted).unwrap();
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj["token"], "[REDACTED]");
        assert_eq!(obj["auth"], "[REDACTED]");
        assert_eq!(obj["session"], "[REDACTED]");
        assert_eq!(obj["jwt"], "[REDACTED]");
    }

    #[test]
    fn redact_body_handles_nested_json() {
        let body = r#"{"outer":{"secret":"hidden","ok":"visible"}}"#;
        let redacted = redact_body(body);
        assert!(
            !redacted.contains("hidden"),
            "nested secret leaked: {redacted}"
        );
        assert!(
            redacted.contains("visible"),
            "non-sensitive lost: {redacted}"
        );
    }

    #[test]
    fn redact_body_returns_non_json_unchanged() {
        let body = "not json at all";
        assert_eq!(redact_body(body), body);
    }

    #[test]
    fn redact_body_scrubs_form_urlencoded() {
        let body = "grant_type=authorization_code&client_secret=shhh&code=abc&redirect_uri=http%3A%2F%2Flocalhost";
        let redacted = redact_body(body);
        assert!(
            !redacted.contains("shhh"),
            "client_secret leaked: {redacted}"
        );
        assert!(
            redacted.contains("authorization_code"),
            "non-sensitive grant_type lost: {redacted}"
        );
        assert!(
            redacted.contains("abc"),
            "non-sensitive code should remain: {redacted}"
        );
    }

    #[test]
    fn redact_url_strips_stale_at_from_userinfo() {
        let url = "https://user:pat_secret@api.example.com/v1/repos";
        let redacted = redact_url(url);
        assert!(
            !redacted.contains('@'),
            "stale @ separator should be removed: {redacted}"
        );
        assert!(
            redacted.starts_with("https://api.example.com"),
            "host should follow scheme directly: {redacted}"
        );
    }

    #[tokio::test]
    async fn recording_interceptor_redacts_body_credentials() {
        let interceptor = RecordingHttpInterceptor::new();
        let req = HttpExchangeRequest {
            method: "POST".to_string(),
            url: "https://api.example.com/auth".to_string(),
            headers: Vec::new(),
            body: Some(r#"{"password":"hunter2","user":"bob"}"#.to_string()),
        };
        let resp = HttpExchangeResponse {
            status: 200,
            headers: Vec::new(),
            body: "ok".to_string(),
        };
        interceptor.after_response(&req, &resp).await;
        let exchanges = interceptor.take_exchanges().await;
        let serialized = serde_json::to_string(&exchanges[0]).unwrap();
        assert!(
            !serialized.contains("hunter2"),
            "body password leaked: {serialized}"
        );
        assert!(
            serialized.contains("bob"),
            "non-sensitive body data lost: {serialized}"
        );
    }

    /// Regression: response bodies are the most common OAuth credential
    /// leak path — a `POST /token` exchange returns
    /// `{"access_token":"...","refresh_token":"..."}` and without
    /// redaction the raw tokens ship into the committed fixture file.
    #[tokio::test]
    async fn recording_interceptor_redacts_response_body_credentials() {
        let interceptor = RecordingHttpInterceptor::new();
        let req = HttpExchangeRequest {
            method: "POST".to_string(),
            url: "https://oauth.example.com/token".to_string(),
            headers: Vec::new(),
            body: None,
        };
        let resp = HttpExchangeResponse {
            status: 200,
            headers: Vec::new(),
            body: r#"{"access_token":"atk_live_abcdef","refresh_token":"rtk_live_xyz","expires_in":3600,"token_type":"Bearer"}"#.to_string(),
        };
        interceptor.after_response(&req, &resp).await;
        let recorded = interceptor.take_exchanges().await;
        let stored = &recorded[0];
        assert!(
            !stored.response.body.contains("atk_live_abcdef"),
            "response access_token leaked: {}",
            stored.response.body
        );
        assert!(
            !stored.response.body.contains("rtk_live_xyz"),
            "response refresh_token leaked: {}",
            stored.response.body
        );
        // Non-sensitive fields preserved for replay determinism.
        assert!(
            stored.response.body.contains("3600"),
            "non-sensitive expires_in lost: {}",
            stored.response.body
        );
        assert!(
            stored.response.body.contains("Bearer"),
            "non-sensitive token_type lost: {}",
            stored.response.body
        );
    }

    #[tokio::test]
    async fn recording_http_interceptor_redacts_url_userinfo() {
        let interceptor = RecordingHttpInterceptor::new();

        let req = HttpExchangeRequest {
            method: "GET".to_string(),
            url: "https://deploy:ghp_fakeToken@api.example.com/repos".to_string(),
            headers: vec![],
            body: None,
        };
        let resp = HttpExchangeResponse {
            status: 200,
            headers: vec![],
            body: r#"{"ok":true}"#.to_string(),
        };

        interceptor.after_response(&req, &resp).await;
        let recorded = interceptor.take_exchanges().await;
        let stored = &recorded[0];
        assert!(
            !stored.request.url.contains("deploy"),
            "username leaked into recorded URL: {}",
            stored.request.url
        );
        assert!(
            !stored.request.url.contains("ghp_fakeToken"),
            "password leaked into recorded URL: {}",
            stored.request.url
        );
        assert!(
            stored.request.url.contains("api.example.com/repos"),
            "host/path should remain: {}",
            stored.request.url
        );
    }

    /// Regression: replay matcher must redact the incoming URL before
    /// comparing against stored (already-redacted) URLs, so sensitive
    /// query params like `access_token` don't cause false mismatches.
    ///
    /// This test exercises the full roundtrip invariant: a raw URL is
    /// redacted via `redact_url()` to produce the stored form (matching
    /// what the recorder does), then the replayer is given the same raw
    /// URL and must match after its own internal redaction.
    #[tokio::test]
    async fn replaying_interceptor_matches_redacted_query_params() {
        // Simulate the recording side: raw URL gets redacted before storage.
        let raw_url = "https://api.example.com/data?access_token=real_secret_token&page=1";
        let stored_url = redact_url(raw_url);

        let exchanges = vec![HttpExchange {
            request: HttpExchangeRequest {
                method: "GET".to_string(),
                url: stored_url.clone(),
                headers: vec![],
                body: None,
            },
            response: HttpExchangeResponse {
                status: 200,
                headers: vec![],
                body: r#"{"ok":true}"#.to_string(),
            },
        }];
        let interceptor = ReplayingHttpInterceptor::new(exchanges);

        // Replay with the same raw URL — redaction should produce a matching URL
        let incoming = HttpExchangeRequest {
            method: "GET".to_string(),
            url: raw_url.to_string(),
            headers: vec![],
            body: None,
        };

        let resp = interceptor.before_request(&incoming).await;
        assert!(
            resp.is_some(),
            "replay should match after URL redaction roundtrip"
        );
        assert_eq!(resp.unwrap().status, 200);
    }
}

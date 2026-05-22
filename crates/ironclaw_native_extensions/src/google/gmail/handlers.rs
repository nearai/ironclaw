//! First-party capability handlers for the six Gmail capabilities.
//!
//! Each handler is an `Arc`-shared struct holding the construction-time
//! dependencies (the credential resolver and the shared `OAuthProvider`). On
//! dispatch a handler:
//!
//! 1. parses and validates its typed input,
//! 2. resolves the shared `google_oauth_token` credential metadata for the
//!    request scope, failing closed on a missing credential or a scope
//!    mismatch (the handler reads only `granted_scopes`/`missing_scopes`, never
//!    the raw token),
//! 3. issues the Gmail API call through the per-invocation
//!    [`RuntimeHttpEgress`] supplied in `request.services.runtime_http_egress`,
//!    declaring a host-staged credential injection so the host egress service
//!    leases, injects, redacts, and audits the `google_oauth_token` — the
//!    handler never holds the access token or its own HTTP transport,
//! 4. projects the raw Gmail response onto a whitelisted output struct so no
//!    access token, internal id, or raw header set leaks into handler output.
//!
//! Routing through `runtime_http_egress` keeps these handlers behind the
//! host's fail-closed egress boundary (`HostHttpEgressService`): staged
//! network policy, credential injection, redaction, auditing, and the ability
//! to disable outbound HTTP in tests all apply. Building a standalone
//! transport would bypass that boundary.
//!
//! Approval gating for the four write capabilities is *not* implemented here —
//! it is descriptor-level (`PermissionMode::Ask` + `EffectKind::ExternalWrite`,
//! see [`super::manifest`]). The host authorization layer is responsible for
//! blocking an unapproved write before `dispatch` is ever called.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_host_api::{
    NetworkMethod, NetworkPolicy, ResourceScope, ResourceUsage, RuntimeCredentialInjection,
    RuntimeCredentialSource, RuntimeCredentialTarget, RuntimeDispatchErrorKind,
    RuntimeHttpEgressError, RuntimeHttpEgressReasonCode, RuntimeHttpEgressRequest, RuntimeKind,
    SecretHandle,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRequest,
    FirstPartyCapabilityResult,
};
use ironclaw_oauth::OAuthProvider;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::google::credential::{
    GOOGLE_CREDENTIAL_NAME, GoogleCredentialError, GoogleCredentialResolver,
};

const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1";

/// Cap on a Gmail response body (1 MiB).
const RESPONSE_BODY_LIMIT: u64 = 1024 * 1024;

/// Per-call host-egress timeout.
const TIMEOUT_MS: u32 = 30_000;

/// Maximum size of an inline message-part body that is projected verbatim;
/// larger base64 payloads are dropped and replaced with an `attachment_ref`.
const INLINE_BODY_LIMIT: usize = 64 * 1024;

/// Shared construction-time dependencies for every Gmail handler.
///
/// Handlers do not own an HTTP client: the transport is the per-invocation
/// [`RuntimeHttpEgress`](ironclaw_host_api::RuntimeHttpEgress) from
/// `InvocationServices`. The resolver is retained only to read credential
/// metadata (`granted_scopes`) for a scope-mismatch preflight; the raw token
/// is leased and injected by the host egress service.
#[derive(Clone)]
pub struct GmailHandlerDeps {
    resolver: Arc<GoogleCredentialResolver>,
    provider: Arc<dyn OAuthProvider>,
    /// OAuth scopes this capability requires (`gmail.readonly` / `gmail.send` /
    /// `gmail.modify`).
    required_scopes: Vec<String>,
}

impl GmailHandlerDeps {
    pub fn new(
        resolver: Arc<GoogleCredentialResolver>,
        provider: Arc<dyn OAuthProvider>,
        required_scopes: Vec<String>,
    ) -> Self {
        Self {
            resolver,
            provider,
            required_scopes,
        }
    }

    /// Preflight the shared Google credential, failing closed on a missing
    /// credential or an OAuth scope mismatch.
    ///
    /// This only inspects credential *metadata*: it never returns or logs the
    /// access token. The token itself is leased and injected by the host
    /// egress service via the staged credential-injection plan.
    async fn preflight_credential(
        &self,
        scope: &ResourceScope,
    ) -> Result<(), FirstPartyCapabilityError> {
        let credential = self
            .resolver
            .resolve(scope, self.provider.as_ref(), &self.required_scopes)
            .await
            .map_err(map_credential_error)?;
        if !credential.missing_scopes.is_empty() {
            // Scope mismatch is an authorization failure the user must resolve
            // by re-consenting; surface it as a client error. The auth-required
            // run-state transition is owned by the phase-2 host obligation
            // layer.
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::Client,
            ));
        }
        Ok(())
    }
}

/// Map a credential-resolution failure to a redacted dispatch error.
fn map_credential_error(error: GoogleCredentialError) -> FirstPartyCapabilityError {
    let kind = match error {
        // Missing credential: fail closed — the user has not connected Google.
        // The host obligation layer (phase 2) is responsible for translating
        // this into an auth-required run-state transition / OAuth bootstrap.
        GoogleCredentialError::Missing => RuntimeDispatchErrorKind::Client,
        _ => RuntimeDispatchErrorKind::Backend,
    };
    FirstPartyCapabilityError::new(kind)
}

/// Map a host runtime-egress failure to a redacted [`RuntimeDispatchErrorKind`].
///
/// Mirrors the built-in `builtin.http` handler's mapping so first-party
/// network failures are classified consistently.
fn map_egress_error(error: &RuntimeHttpEgressError) -> FirstPartyCapabilityError {
    let kind = match error.reason_code() {
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
        RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
        RuntimeHttpEgressReasonCode::NetworkError => RuntimeDispatchErrorKind::NetworkDenied,
        RuntimeHttpEgressReasonCode::ResponseError => RuntimeDispatchErrorKind::OutputDecode,
        RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            RuntimeDispatchErrorKind::OutputTooLarge
        }
    };
    FirstPartyCapabilityError::new(kind)
}

/// Parse a handler's typed input, mapping a decode failure to `InputEncode`.
fn parse_input<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, FirstPartyCapabilityError> {
    serde_json::from_value(input)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))
}

/// Serialize a handler's whitelisted output, mapping a failure to `InvalidResult`.
fn encode_output<T: Serialize>(output: &T) -> Result<Value, FirstPartyCapabilityError> {
    serde_json::to_value(output)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InvalidResult))
}

/// Build a `FirstPartyCapabilityResult` with wall-clock and output-byte usage.
fn finish(output: Value, started: Instant) -> FirstPartyCapabilityResult {
    let wall_clock_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let output_bytes = serde_json::to_vec(&output)
        .map(|b| b.len() as u64)
        .unwrap_or(0);
    FirstPartyCapabilityResult::new(
        output,
        ResourceUsage {
            wall_clock_ms,
            output_bytes,
            ..ResourceUsage::default()
        },
    )
}

/// URL-encode a path/query segment (message id, query string) so values
/// containing `@`, `#`, `/`, or spaces cannot escape the intended API path.
fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Base64url-encode bytes without padding (RFC 4648 §5), as required by the
/// Gmail API `raw` field. No external base64 crate is on the dependency graph,
/// so the encoder is kept self-contained here.
fn base64url_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3F) as usize] as char);
        }
    }
    out
}

/// The host-staged credential-injection plan for a Gmail API call.
///
/// The handler declares *what* should be injected — the `google_oauth_token`
/// secret, into the `authorization` header with a `Bearer ` prefix — sourced
/// from a `StagedObligation` for this capability. The host egress service
/// (`HostHttpEgressService`) is what leases the secret and performs the
/// injection; the handler never touches the token material.
fn google_credential_injection(
    capability_id: &ironclaw_host_api::CapabilityId,
) -> Result<RuntimeCredentialInjection, FirstPartyCapabilityError> {
    let handle = SecretHandle::new(GOOGLE_CREDENTIAL_NAME)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend))?;
    Ok(RuntimeCredentialInjection {
        handle,
        source: RuntimeCredentialSource::StagedObligation {
            capability_id: capability_id.clone(),
        },
        target: RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required: true,
    })
}

/// Issue a Gmail API call through the host runtime-egress boundary.
///
/// `body` is `None` for `GET` and `Some(json)` for write methods. The JSON
/// response body is parsed and returned; an empty `2xx` body becomes
/// [`Value::Null`].
async fn call_gmail(
    request: &FirstPartyCapabilityRequest,
    method: NetworkMethod,
    url: String,
    body: Option<Value>,
) -> Result<Value, FirstPartyCapabilityError> {
    let egress = request
        .services
        .runtime_http_egress
        .as_ref()
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied))?
        .clone();

    let mut headers = Vec::new();
    let body_bytes = match body {
        Some(value) => {
            headers.push(("content-type".to_string(), "application/json".to_string()));
            serde_json::to_vec(&value).map_err(|_| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
            })?
        }
        None => Vec::new(),
    };

    let http_request = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        method,
        url,
        headers,
        body: body_bytes,
        // First-party network policy is staged in HostHttpEgressService from
        // the grant obligation for this scope/capability; this fallback field
        // is ignored on the production path and only used by test services.
        network_policy: NetworkPolicy::default(),
        credential_injections: vec![google_credential_injection(&request.capability_id)?],
        response_body_limit: Some(RESPONSE_BODY_LIMIT),
        timeout_ms: Some(TIMEOUT_MS),
    };

    let response = tokio::task::spawn_blocking(move || egress.execute(http_request))
        .await
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend))?
        .map_err(|error| map_egress_error(&error))?;

    if !(200..300).contains(&response.status) {
        // Non-2xx Gmail responses (auth/scope/quota failures) are surfaced as
        // client errors; the phase-2 host layer owns the auth-required
        // run-state transition.
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Client,
        ));
    }
    if response.body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&response.body)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputDecode))
}

// ---------------------------------------------------------------------------
// Whitelisted output projections.
//
// Output structs deliberately project only non-sensitive, whitelisted fields.
// The raw Gmail response is never echoed: internal ids (`internalDate`,
// `historyId`), the raw `payload.headers` array, and large base64 attachment
// bodies are dropped. The access token never appears in output.
// ---------------------------------------------------------------------------

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Whitelisted projection of a single message reference from `messages.list`.
#[derive(Debug, Serialize)]
struct MessageRef {
    id: String,
    thread_id: Option<String>,
}

impl MessageRef {
    fn from_google(value: &Value) -> Self {
        Self {
            id: string_field(value, "id").unwrap_or_default(),
            thread_id: string_field(value, "threadId"),
        }
    }
}

#[derive(Debug, Serialize)]
struct ListMessagesOutput {
    messages: Vec<MessageRef>,
    next_page_token: Option<String>,
    result_size_estimate: Option<i64>,
}

/// Whitelisted header projection: only the user-meaningful envelope headers
/// are kept; the raw `payload.headers` array is never echoed.
#[derive(Debug, Default, Serialize)]
struct MessageHeaders {
    from: Option<String>,
    to: Option<String>,
    cc: Option<String>,
    subject: Option<String>,
    date: Option<String>,
}

impl MessageHeaders {
    /// Extract the whitelisted headers from a Gmail `payload.headers` array.
    fn from_payload(payload: Option<&Value>) -> Self {
        let mut headers = Self::default();
        let Some(list) = payload
            .and_then(|p| p.get("headers"))
            .and_then(Value::as_array)
        else {
            return headers;
        };
        for header in list {
            let (Some(name), Some(val)) = (
                header.get("name").and_then(Value::as_str),
                header.get("value").and_then(Value::as_str),
            ) else {
                continue;
            };
            match name.to_ascii_lowercase().as_str() {
                "from" => headers.from = Some(val.to_string()),
                "to" => headers.to = Some(val.to_string()),
                "cc" => headers.cc = Some(val.to_string()),
                "subject" => headers.subject = Some(val.to_string()),
                "date" => headers.date = Some(val.to_string()),
                _ => {}
            }
        }
        headers
    }
}

/// A whitelisted projection of a message body part. Large base64 attachment
/// bodies are dropped and replaced with `attachment_ref` for separate fetch.
#[derive(Debug, Serialize)]
struct MessagePart {
    mime_type: Option<String>,
    filename: Option<String>,
    /// Inline body data (base64url) — only kept when below `INLINE_BODY_LIMIT`.
    body_data: Option<String>,
    /// Opaque attachment id for parts whose body was withheld.
    attachment_ref: Option<String>,
    size: Option<i64>,
}

impl MessagePart {
    fn from_google(value: &Value) -> Self {
        let body = value.get("body");
        let size = body.and_then(|b| b.get("size")).and_then(Value::as_i64);
        let raw_data = body.and_then(|b| b.get("data")).and_then(Value::as_str);
        let attachment_id = body
            .and_then(|b| b.get("attachmentId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        // Drop large inline bodies; surface an attachment_ref instead so the
        // caller can fetch the payload separately rather than inlining it.
        let (body_data, attachment_ref) = match (raw_data, attachment_id) {
            (Some(data), _) if data.len() <= INLINE_BODY_LIMIT => (Some(data.to_string()), None),
            (Some(_), Some(att_id)) => (None, Some(att_id)),
            (Some(_), None) => (None, Some("oversized-inline-body".to_string())),
            (None, att_id) => (None, att_id),
        };
        Self {
            mime_type: string_field(value, "mimeType"),
            filename: string_field(value, "filename"),
            body_data,
            attachment_ref,
            size,
        }
    }
}

/// Whitelisted projection of a Gmail message.
///
/// Internal fields (`internalDate`, `historyId`, the raw `payload.headers`
/// array, `raw`) are intentionally dropped.
#[derive(Debug, Serialize)]
struct MessageOutput {
    id: String,
    thread_id: Option<String>,
    label_ids: Vec<String>,
    snippet: Option<String>,
    size_estimate: Option<i64>,
    headers: MessageHeaders,
    parts: Vec<MessagePart>,
}

impl MessageOutput {
    fn from_google(value: &Value) -> Self {
        let payload = value.get("payload");
        let label_ids = value
            .get("labelIds")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        // A message either has a single top-level body part or a `parts` array.
        let parts = match payload
            .and_then(|p| p.get("parts"))
            .and_then(Value::as_array)
        {
            Some(items) => items.iter().map(MessagePart::from_google).collect(),
            None => match payload {
                Some(p) => vec![MessagePart::from_google(p)],
                None => Vec::new(),
            },
        };
        Self {
            id: string_field(value, "id").unwrap_or_default(),
            thread_id: string_field(value, "threadId"),
            label_ids,
            snippet: string_field(value, "snippet"),
            size_estimate: value.get("sizeEstimate").and_then(Value::as_i64),
            headers: MessageHeaders::from_payload(payload),
            parts,
        }
    }
}

/// Whitelisted projection of a `messages.send` response.
#[derive(Debug, Serialize)]
struct SentMessageOutput {
    id: String,
    thread_id: Option<String>,
    label_ids: Vec<String>,
}

impl SentMessageOutput {
    fn from_google(value: &Value) -> Self {
        let label_ids = value
            .get("labelIds")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            id: string_field(value, "id").unwrap_or_default(),
            thread_id: string_field(value, "threadId"),
            label_ids,
        }
    }
}

/// Whitelisted projection of a `drafts.create` response.
#[derive(Debug, Serialize)]
struct DraftOutput {
    id: String,
    message_id: Option<String>,
    thread_id: Option<String>,
}

impl DraftOutput {
    fn from_google(value: &Value) -> Self {
        let message = value.get("message");
        Self {
            id: string_field(value, "id").unwrap_or_default(),
            message_id: message.and_then(|m| string_field(m, "id")),
            thread_id: message.and_then(|m| string_field(m, "threadId")),
        }
    }
}

/// Whitelisted projection of a `messages.trash` response.
#[derive(Debug, Serialize)]
struct TrashedMessageOutput {
    id: String,
    trashed: bool,
    label_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// Typed inputs.
// ---------------------------------------------------------------------------

fn default_message_format() -> String {
    "full".to_string()
}

#[derive(Debug, Deserialize)]
struct ListMessagesInput {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    label_ids: Vec<String>,
    #[serde(default)]
    max_results: Option<u32>,
    #[serde(default)]
    page_token: Option<String>,
    #[serde(default)]
    include_spam_trash: bool,
}

#[derive(Debug, Deserialize)]
struct GetMessageInput {
    message_id: String,
    #[serde(default = "default_message_format")]
    format: String,
}

#[derive(Debug, Deserialize)]
struct SendMessageInput {
    to: Vec<String>,
    #[serde(default)]
    cc: Vec<String>,
    #[serde(default)]
    bcc: Vec<String>,
    subject: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct CreateDraftInput {
    to: Vec<String>,
    #[serde(default)]
    cc: Vec<String>,
    #[serde(default)]
    bcc: Vec<String>,
    subject: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct ReplyToMessageInput {
    /// Id of the message being replied to.
    message_id: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct TrashMessageInput {
    message_id: String,
}

/// Build an RFC 822 message from envelope fields and a plain-text body.
fn build_rfc822(
    to: &[String],
    cc: &[String],
    bcc: &[String],
    subject: &str,
    body: &str,
    extra_headers: &[(&str, &str)],
) -> String {
    let mut message = String::new();
    message.push_str(&format!("To: {}\r\n", to.join(", ")));
    if !cc.is_empty() {
        message.push_str(&format!("Cc: {}\r\n", cc.join(", ")));
    }
    if !bcc.is_empty() {
        message.push_str(&format!("Bcc: {}\r\n", bcc.join(", ")));
    }
    message.push_str(&format!("Subject: {subject}\r\n"));
    for (name, value) in extra_headers {
        message.push_str(&format!("{name}: {value}\r\n"));
    }
    message.push_str("Content-Type: text/plain; charset=\"UTF-8\"\r\n");
    message.push_str("MIME-Version: 1.0\r\n");
    message.push_str("\r\n");
    message.push_str(body);
    message
}

/// Extract a header value (case-insensitive) from a Gmail `payload.headers`
/// array.
fn header_value(payload: Option<&Value>, name: &str) -> Option<String> {
    payload
        .and_then(|p| p.get("headers"))
        .and_then(Value::as_array)?
        .iter()
        .find(|header| {
            header
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|header| header.get("value").and_then(Value::as_str))
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Read handlers.
// ---------------------------------------------------------------------------

/// `gmail.list_messages` — list messages in the user's mailbox.
pub struct ListMessagesHandler {
    deps: GmailHandlerDeps,
}

impl ListMessagesHandler {
    pub fn new(deps: GmailHandlerDeps) -> Self {
        Self { deps }
    }
}

#[async_trait]
impl FirstPartyCapabilityHandler for ListMessagesHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        let input: ListMessagesInput = parse_input(request.input.clone())?;
        self.deps.preflight_credential(&request.scope).await?;
        let mut url = format!(
            "{GMAIL_API_BASE}/users/me/messages?includeSpamTrash={}",
            input.include_spam_trash
        );
        if let Some(query) = &input.query {
            url.push_str(&format!("&q={}", encode_segment(query)));
        }
        for label in &input.label_ids {
            url.push_str(&format!("&labelIds={}", encode_segment(label)));
        }
        if let Some(max_results) = input.max_results {
            url.push_str(&format!("&maxResults={max_results}"));
        }
        if let Some(page_token) = &input.page_token {
            url.push_str(&format!("&pageToken={}", encode_segment(page_token)));
        }
        let body = call_gmail(&request, NetworkMethod::Get, url, None).await?;
        let messages = body
            .get("messages")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(MessageRef::from_google).collect())
            .unwrap_or_default();
        let output = encode_output(&ListMessagesOutput {
            messages,
            next_page_token: string_field(&body, "nextPageToken"),
            result_size_estimate: body.get("resultSizeEstimate").and_then(Value::as_i64),
        })?;
        Ok(finish(output, started))
    }
}

/// `gmail.get_message` — fetch one message by id.
pub struct GetMessageHandler {
    deps: GmailHandlerDeps,
}

impl GetMessageHandler {
    pub fn new(deps: GmailHandlerDeps) -> Self {
        Self { deps }
    }
}

#[async_trait]
impl FirstPartyCapabilityHandler for GetMessageHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        let input: GetMessageInput = parse_input(request.input.clone())?;
        self.deps.preflight_credential(&request.scope).await?;
        let url = format!(
            "{GMAIL_API_BASE}/users/me/messages/{}?format={}",
            encode_segment(&input.message_id),
            encode_segment(&input.format),
        );
        let body = call_gmail(&request, NetworkMethod::Get, url, None).await?;
        let output = encode_output(&MessageOutput::from_google(&body))?;
        Ok(finish(output, started))
    }
}

// ---------------------------------------------------------------------------
// Write handlers — all descriptor-gated with `PermissionMode::Ask`.
// ---------------------------------------------------------------------------

/// `gmail.send_message` — compose and send a message (RequiresApproval).
pub struct SendMessageHandler {
    deps: GmailHandlerDeps,
}

impl SendMessageHandler {
    pub fn new(deps: GmailHandlerDeps) -> Self {
        Self { deps }
    }
}

#[async_trait]
impl FirstPartyCapabilityHandler for SendMessageHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        let input: SendMessageInput = parse_input(request.input.clone())?;
        self.deps.preflight_credential(&request.scope).await?;
        let rfc822 = build_rfc822(
            &input.to,
            &input.cc,
            &input.bcc,
            &input.subject,
            &input.body,
            &[],
        );
        let request_body = serde_json::json!({ "raw": base64url_encode(rfc822.as_bytes()) });
        let url = format!("{GMAIL_API_BASE}/users/me/messages/send");
        let body = call_gmail(&request, NetworkMethod::Post, url, Some(request_body)).await?;
        let output = encode_output(&SentMessageOutput::from_google(&body))?;
        Ok(finish(output, started))
    }
}

/// `gmail.create_draft` — create a draft without sending (RequiresApproval).
pub struct CreateDraftHandler {
    deps: GmailHandlerDeps,
}

impl CreateDraftHandler {
    pub fn new(deps: GmailHandlerDeps) -> Self {
        Self { deps }
    }
}

#[async_trait]
impl FirstPartyCapabilityHandler for CreateDraftHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        let input: CreateDraftInput = parse_input(request.input.clone())?;
        self.deps.preflight_credential(&request.scope).await?;
        let rfc822 = build_rfc822(
            &input.to,
            &input.cc,
            &input.bcc,
            &input.subject,
            &input.body,
            &[],
        );
        let request_body = serde_json::json!({
            "message": { "raw": base64url_encode(rfc822.as_bytes()) }
        });
        let url = format!("{GMAIL_API_BASE}/users/me/drafts");
        let body = call_gmail(&request, NetworkMethod::Post, url, Some(request_body)).await?;
        let output = encode_output(&DraftOutput::from_google(&body))?;
        Ok(finish(output, started))
    }
}

/// `gmail.reply_to_message` — reply within an existing thread (RequiresApproval).
pub struct ReplyToMessageHandler {
    deps: GmailHandlerDeps,
}

impl ReplyToMessageHandler {
    pub fn new(deps: GmailHandlerDeps) -> Self {
        Self { deps }
    }
}

#[async_trait]
impl FirstPartyCapabilityHandler for ReplyToMessageHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        let input: ReplyToMessageInput = parse_input(request.input.clone())?;
        self.deps.preflight_credential(&request.scope).await?;

        // Fetch the original message metadata so the reply preserves the
        // thread and references the original Message-ID.
        let metadata_url = format!(
            "{GMAIL_API_BASE}/users/me/messages/{}?format=metadata",
            encode_segment(&input.message_id),
        );
        let original = call_gmail(&request, NetworkMethod::Get, metadata_url, None).await?;
        let payload = original.get("payload");
        let thread_id = string_field(&original, "threadId");
        let original_subject = header_value(payload, "Subject").unwrap_or_default();
        let original_message_id =
            header_value(payload, "Message-ID").or_else(|| header_value(payload, "Message-Id"));
        let original_references = header_value(payload, "References");
        let original_from = header_value(payload, "From").unwrap_or_default();

        let reply_subject = if original_subject.to_ascii_lowercase().starts_with("re:") {
            original_subject
        } else {
            format!("Re: {original_subject}")
        };
        // Build In-Reply-To / References per RFC 5322 threading conventions.
        let references = match (&original_references, &original_message_id) {
            (Some(refs), Some(mid)) => format!("{refs} {mid}"),
            (None, Some(mid)) => mid.clone(),
            (Some(refs), None) => refs.clone(),
            (None, None) => String::new(),
        };
        let mut extra_headers: Vec<(&str, &str)> = Vec::new();
        if let Some(mid) = &original_message_id {
            extra_headers.push(("In-Reply-To", mid.as_str()));
        }
        if !references.is_empty() {
            extra_headers.push(("References", references.as_str()));
        }
        let rfc822 = build_rfc822(
            &[original_from],
            &[],
            &[],
            &reply_subject,
            &input.body,
            &extra_headers,
        );
        let mut request_body = serde_json::json!({ "raw": base64url_encode(rfc822.as_bytes()) });
        if let Some(thread) = &thread_id {
            request_body["threadId"] = Value::String(thread.clone());
        }
        let url = format!("{GMAIL_API_BASE}/users/me/messages/send");
        let body = call_gmail(&request, NetworkMethod::Post, url, Some(request_body)).await?;
        let output = encode_output(&SentMessageOutput::from_google(&body))?;
        Ok(finish(output, started))
    }
}

/// `gmail.trash_message` — move a message to trash (RequiresApproval).
pub struct TrashMessageHandler {
    deps: GmailHandlerDeps,
}

impl TrashMessageHandler {
    pub fn new(deps: GmailHandlerDeps) -> Self {
        Self { deps }
    }
}

#[async_trait]
impl FirstPartyCapabilityHandler for TrashMessageHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        let input: TrashMessageInput = parse_input(request.input.clone())?;
        self.deps.preflight_credential(&request.scope).await?;
        let url = format!(
            "{GMAIL_API_BASE}/users/me/messages/{}/trash",
            encode_segment(&input.message_id),
        );
        let body = call_gmail(
            &request,
            NetworkMethod::Post,
            url,
            Some(serde_json::json!({})),
        )
        .await?;
        let label_ids = body
            .get("labelIds")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let output = encode_output(&TrashedMessageOutput {
            id: input.message_id,
            trashed: true,
            label_ids,
        })?;
        Ok(finish(output, started))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_segment_escapes_path_breaking_characters() {
        assert_eq!(encode_segment("msg-1"), "msg-1");
        assert_eq!(encode_segment("user@example.com"), "user%40example.com");
        assert_eq!(encode_segment("a/b"), "a%2Fb");
        assert_eq!(encode_segment("is:unread from:x"), "is%3Aunread%20from%3Ax");
    }

    #[test]
    fn base64url_encode_matches_known_vectors() {
        assert_eq!(base64url_encode(b""), "");
        assert_eq!(base64url_encode(b"f"), "Zg");
        assert_eq!(base64url_encode(b"fo"), "Zm8");
        assert_eq!(base64url_encode(b"foo"), "Zm9v");
        assert_eq!(base64url_encode(b"foobar"), "Zm9vYmFy");
        // base64url uses `-`/`_` and never emits `+`/`/` or padding.
        let encoded = base64url_encode(&[0xFB, 0xFF, 0xBF]);
        assert!(!encoded.contains('+') && !encoded.contains('/') && !encoded.contains('='));
    }

    #[test]
    fn message_output_drops_internal_fields_and_raw_headers() {
        let raw = serde_json::json!({
            "id": "msg-1",
            "threadId": "thr-1",
            "internalDate": "1716200000000",
            "historyId": "987654",
            "snippet": "Hello there",
            "labelIds": ["INBOX", "UNREAD"],
            "payload": {
                "headers": [
                    { "name": "From", "value": "alice@example.com" },
                    { "name": "Subject", "value": "Hi" },
                    { "name": "X-Internal-Trace", "value": "leak-me" }
                ]
            }
        });
        let projected = serde_json::to_value(MessageOutput::from_google(&raw)).unwrap();
        assert_eq!(projected.get("id").and_then(Value::as_str), Some("msg-1"));
        assert!(projected.get("internalDate").is_none());
        assert!(projected.get("historyId").is_none());
        let serialized = serde_json::to_string(&projected).unwrap();
        // Whitelisted headers survive; un-whitelisted raw headers do not.
        assert!(serialized.contains("alice@example.com"));
        assert!(!serialized.contains("X-Internal-Trace"));
        assert!(!serialized.contains("leak-me"));
    }

    #[test]
    fn message_part_replaces_oversized_body_with_attachment_ref() {
        let big = "A".repeat(INLINE_BODY_LIMIT + 1);
        let raw = serde_json::json!({
            "mimeType": "application/pdf",
            "filename": "report.pdf",
            "body": { "data": big, "attachmentId": "att-99", "size": 200000 }
        });
        let part = MessagePart::from_google(&raw);
        assert!(part.body_data.is_none(), "oversized body must be dropped");
        assert_eq!(part.attachment_ref.as_deref(), Some("att-99"));
    }

    #[test]
    fn credential_injection_targets_authorization_header_with_bearer_prefix() {
        let capability_id = ironclaw_host_api::CapabilityId::new("gmail.send_message").unwrap();
        let injection = google_credential_injection(&capability_id).unwrap();
        assert_eq!(injection.handle.as_str(), GOOGLE_CREDENTIAL_NAME);
        assert!(injection.required);
        match injection.source {
            RuntimeCredentialSource::StagedObligation { capability_id: id } => {
                assert_eq!(id, capability_id);
            }
            RuntimeCredentialSource::SecretStoreLease => {
                panic!("must use a staged obligation, not a direct secret-store lease")
            }
        }
        match injection.target {
            RuntimeCredentialTarget::Header { name, prefix } => {
                assert_eq!(name, "authorization");
                assert_eq!(prefix.as_deref(), Some("Bearer "));
            }
            RuntimeCredentialTarget::QueryParam { .. } => {
                panic!("OAuth bearer token must be a header, not a query param")
            }
        }
    }

    #[test]
    fn build_rfc822_includes_envelope_and_threading_headers() {
        let message = build_rfc822(
            &["bob@example.com".to_string()],
            &["cc@example.com".to_string()],
            &[],
            "Re: Status",
            "Looks good.",
            &[("In-Reply-To", "<orig@mail>")],
        );
        assert!(message.contains("To: bob@example.com\r\n"));
        assert!(message.contains("Cc: cc@example.com\r\n"));
        assert!(message.contains("Subject: Re: Status\r\n"));
        assert!(message.contains("In-Reply-To: <orig@mail>\r\n"));
        assert!(message.ends_with("Looks good."));
    }
}

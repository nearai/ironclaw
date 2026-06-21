//! Route-independent WebUI inbound DTO contract.
//!
//! These DTOs normalize authenticated WebUI callers plus browser request bodies
//! into canonical Reborn commands without depending on WebUI route handlers,
//! product adapters, protocol auth evidence, WASM, or adapter registries.

use ironclaw_attachments::InboundAttachment;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    CancelRunRequest, GateRef, IdempotencyKey, SanitizedCancelReason, TurnActor, TurnRunId,
    TurnScope,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const CLIENT_ACTION_ID_MAX_BYTES: usize = 256;
const USER_MESSAGE_TEXT_MAX_BYTES: usize = 64 * 1024;
const GATE_REF_MAX_BYTES: usize = 256;
const CREDENTIAL_REF_MAX_BYTES: usize = 512;
/// Inline-attachment budgets, mirroring the v1 web gateway: at most
/// `MAX_INLINE_ATTACHMENTS` files, `MAX_INLINE_ATTACHMENT_BYTES` decoded bytes
/// per file, and `MAX_INLINE_TOTAL_ATTACHMENT_BYTES` decoded bytes total.
const MAX_INLINE_ATTACHMENTS: usize = 10;
const MAX_INLINE_ATTACHMENT_BYTES: usize = 5 * 1024 * 1024;
const MAX_INLINE_TOTAL_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
const ATTACHMENT_FILENAME_MAX_BYTES: usize = 256;

/// Browser-facing inline-attachment contract advertised to the WebUI.
///
/// Carries the `accept` tokens generated from the shared
/// [`ironclaw_common`] format registry (so the file picker can never drift
/// from the server's allowed MIME set) plus the same budgets
/// [`WebUiSendMessageRequest::decode_attachments`] enforces. The browser
/// uses this only for pre-submit hints; the server-side decode remains the
/// sole authority on what is accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebUiAttachmentCapabilities {
    /// HTML file-input `accept` tokens from the shared registry: exact MIME
    /// types plus extensions, e.g. `["image/png", ".png", "application/pdf",
    /// ".pdf"]` — never `image/*` wildcards (which would advertise unsupported
    /// formats, and which break folder navigation in the native macOS picker).
    pub accept: Vec<String>,
    /// Maximum number of attachments per message.
    pub max_count: usize,
    /// Maximum decoded byte size of a single attachment.
    pub max_file_bytes: usize,
    /// Maximum combined decoded byte size of all attachments in one message.
    pub max_total_bytes: usize,
}

/// The inline-attachment contract advertised to browsers. Generated from the
/// shared format registry and the budgets `decode_attachments` enforces, so
/// the picker and the server stay in lockstep by construction.
pub fn webui_attachment_capabilities() -> WebUiAttachmentCapabilities {
    WebUiAttachmentCapabilities {
        accept: ironclaw_common::accept_tokens(),
        max_count: MAX_INLINE_ATTACHMENTS,
        max_file_bytes: MAX_INLINE_ATTACHMENT_BYTES,
        max_total_bytes: MAX_INLINE_TOTAL_ATTACHMENT_BYTES,
    }
}

/// Authenticated WebUI caller after route auth has already completed.
///
/// This is authority-bearing input supplied by the host/router layer, not by
/// the browser body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebUiAuthenticatedCaller {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

impl WebUiAuthenticatedCaller {
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id,
            project_id,
        }
    }

    pub fn actor(&self) -> TurnActor {
        TurnActor::new(self.user_id.clone())
    }

    pub fn turn_scope(&self, thread_id: ThreadId) -> TurnScope {
        TurnScope::new_with_owner(
            self.tenant_id.clone(),
            self.agent_id.clone(),
            self.project_id.clone(),
            thread_id,
            Some(self.user_id.clone()),
        )
    }
}

/// Browser body for WebUI create-thread mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiCreateThreadRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_thread_id: Option<String>,
    /// Optional project the new thread should be scoped to. The browser only
    /// *proposes* it — the facade authorizes the caller's access to the project
    /// before adopting it as scope, so the body is never trusted on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

/// One inline attachment in a browser send-message body.
///
/// `data_base64` is the base64-encoded file bytes; `mime_type` is validated
/// against the shared attachment format registry. This is the only place raw
/// upload bytes enter the workflow — they are decoded, budgeted, and landed in
/// storage, never carried on the (serializable) inbound command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiInboundAttachment {
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub data_base64: String,
}

/// Browser body for WebUI send-message mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiSendMessageRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<WebUiInboundAttachment>,
}

impl WebUiSendMessageRequest {
    /// Validate and decode the inline attachments into bytes-bearing
    /// [`InboundAttachment`]s ready for landing.
    ///
    /// Enforces the per-file / per-message / count budgets and rejects
    /// unsupported MIME types (per the shared format registry) and malformed
    /// base64 with a stable validation error. Kept separate from
    /// [`Self::into_command`] so the serializable command never carries raw
    /// bytes.
    pub fn decode_attachments(
        &self,
    ) -> Result<Vec<InboundAttachment>, WebUiInboundValidationError> {
        use base64::Engine;

        if self.attachments.len() > MAX_INLINE_ATTACHMENTS {
            return Err(WebUiInboundValidationError::new(
                "attachments",
                WebUiInboundValidationCode::TooLong,
            ));
        }

        let mut decoded = Vec::with_capacity(self.attachments.len());
        let mut total_bytes = 0usize;
        for (index, attachment) in self.attachments.iter().enumerate() {
            let mime = ironclaw_common::normalize_mime_type(&attachment.mime_type);
            if !ironclaw_common::is_supported_mime(&mime) {
                return Err(WebUiInboundValidationError::new(
                    "attachments.mime_type",
                    WebUiInboundValidationCode::InvalidValue,
                ));
            }

            let bytes = base64::engine::general_purpose::STANDARD
                .decode(attachment.data_base64.as_bytes())
                .map_err(|_| {
                    WebUiInboundValidationError::new(
                        "attachments.data_base64",
                        WebUiInboundValidationCode::InvalidValue,
                    )
                })?;
            if bytes.len() > MAX_INLINE_ATTACHMENT_BYTES {
                return Err(WebUiInboundValidationError::new(
                    "attachments",
                    WebUiInboundValidationCode::TooLong,
                ));
            }
            total_bytes = total_bytes.saturating_add(bytes.len());
            if total_bytes > MAX_INLINE_TOTAL_ATTACHMENT_BYTES {
                return Err(WebUiInboundValidationError::new(
                    "attachments",
                    WebUiInboundValidationCode::TooLong,
                ));
            }

            let filename = attachment
                .filename
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty());
            if let Some(name) = filename
                && name.len() > ATTACHMENT_FILENAME_MAX_BYTES
            {
                return Err(WebUiInboundValidationError::new(
                    "attachments.filename",
                    WebUiInboundValidationCode::TooLong,
                ));
            }

            // `kind` and the fallback filename extension are derived from
            // `mime_type` inside the landing bridge, so the DTO carries only the
            // raw upload fields here.
            decoded.push(InboundAttachment {
                id: format!("webui-attachment-{index}"),
                mime_type: mime,
                filename: filename.map(str::to_string),
                bytes,
            });
        }
        Ok(decoded)
    }
}

/// Browser body for WebUI cancel-run mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiCancelRunRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Browser query for WebUI list-threads read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiListThreadsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Browser query for WebUI automation listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiListAutomationsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_limit: Option<u32>,
    /// When `true`, soft-completed (fire-once) automations are included in the
    /// response alongside active ones. Defaults to `false` (active-only) so
    /// existing callers that do not set this flag are unaffected.
    #[serde(default)]
    pub include_completed: bool,
}

/// Browser body for WebUI extension-setup interaction.
///
/// This is the v2 entrypoint inventory's "extensions onboarding" row.
/// The native facade exposes the route surface so callers can
/// inventory the API without v1 dependency. Concrete implementations return a
/// product-safe lifecycle projection; auth, approval, and pairing requirements
/// remain blockers owned by their dedicated Reborn services, not lifecycle
/// phases.
///
/// The package id is not part of the body — it is bound from the route
/// path and lifted into a lifecycle package ref by the handler before
/// it crosses the facade boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiSetupExtensionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// Browser body for WebUI gate-resolution mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebUiResolveGateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub always: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebUiCancelReason {
    UserRequested,
    Superseded,
    Timeout,
    OperatorRequested,
    Policy,
}

impl From<WebUiCancelReason> for SanitizedCancelReason {
    fn from(value: WebUiCancelReason) -> Self {
        match value {
            WebUiCancelReason::UserRequested => Self::UserRequested,
            WebUiCancelReason::Superseded => Self::Superseded,
            WebUiCancelReason::Timeout => Self::Timeout,
            WebUiCancelReason::OperatorRequested => Self::OperatorRequested,
            WebUiCancelReason::Policy => Self::Policy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "resolution", rename_all = "snake_case")]
pub enum WebUiGateResolution {
    Approved {
        #[serde(default)]
        always: bool,
    },
    /// Unified decline variant — covers both user-initiated approval denial
    /// ("denied") and auth-gate cancellation ("cancelled"). Both legacy wire
    /// strings deserialize to this variant; new serializations use "declined".
    #[serde(alias = "denied", alias = "cancelled")]
    Declined,
    /// A host-stored credential reference, not a raw secret/token.
    CredentialProvided { credential_ref: String },
}

/// Canonical route-independent WebUI command produced after validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum WebUiInboundCommand {
    CreateThread {
        caller: WebUiAuthenticatedCaller,
        client_action_id: IdempotencyKey,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requested_thread_id: Option<ThreadId>,
    },
    SendMessage {
        scope: TurnScope,
        actor: TurnActor,
        client_action_id: IdempotencyKey,
        content: String,
    },
    CancelRun {
        request: CancelRunRequest,
    },
    ResolveGate {
        scope: TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        gate_ref: GateRef,
        client_action_id: IdempotencyKey,
        resolution: WebUiGateResolution,
    },
}

impl WebUiCreateThreadRequest {
    pub fn into_command(
        self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<WebUiInboundCommand, WebUiInboundValidationError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let requested_thread_id = self
            .requested_thread_id
            .map(|value| parse_thread_id_value("requested_thread_id", value))
            .transpose()?;

        Ok(WebUiInboundCommand::CreateThread {
            caller,
            client_action_id,
            requested_thread_id,
        })
    }
}

impl WebUiSendMessageRequest {
    pub fn into_command(
        self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<WebUiInboundCommand, WebUiInboundValidationError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let thread_id = parse_thread_id(self.thread_id)?;
        let content = required_text(
            "content",
            self.content,
            USER_MESSAGE_TEXT_MAX_BYTES,
            TextMode::MessageContent,
        )?;

        Ok(WebUiInboundCommand::SendMessage {
            scope: caller.turn_scope(thread_id),
            actor: caller.actor(),
            client_action_id,
            content,
        })
    }
}

impl WebUiCancelRunRequest {
    pub fn into_command(
        self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<WebUiInboundCommand, WebUiInboundValidationError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let thread_id = parse_thread_id(self.thread_id)?;
        let run_id = parse_run_id(self.run_id)?;
        let reason = parse_cancel_reason(self.reason)?;

        Ok(WebUiInboundCommand::CancelRun {
            request: CancelRunRequest {
                scope: caller.turn_scope(thread_id),
                actor: caller.actor(),
                run_id,
                reason,
                idempotency_key: client_action_id,
            },
        })
    }
}

impl WebUiResolveGateRequest {
    pub fn into_command(
        self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<WebUiInboundCommand, WebUiInboundValidationError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let thread_id = parse_thread_id(self.thread_id)?;
        let run_id = parse_run_id(self.run_id)?;
        let gate_ref = parse_gate_ref(self.gate_ref)?;
        let resolution = parse_gate_resolution(self.resolution, self.always, self.credential_ref)?;

        Ok(WebUiInboundCommand::ResolveGate {
            scope: caller.turn_scope(thread_id),
            actor: caller.actor(),
            run_id,
            gate_ref,
            client_action_id,
            resolution,
        })
    }
}

/// Stable validation error code for WebUI inbound DTOs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebUiInboundValidationCode {
    MissingField,
    Blank,
    TooLong,
    InvalidControlCharacter,
    InvalidId,
    InvalidValue,
}

/// Stable validation error shape for WebUI clients and facade tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("invalid WebUI inbound field {field}: {code:?}")]
pub struct WebUiInboundValidationError {
    pub field: String,
    pub code: WebUiInboundValidationCode,
}

impl WebUiInboundValidationError {
    pub fn new(field: &'static str, code: WebUiInboundValidationCode) -> Self {
        Self {
            field: field.to_string(),
            code,
        }
    }
}

fn parse_client_action_id(
    value: Option<String>,
) -> Result<IdempotencyKey, WebUiInboundValidationError> {
    let value = required_text(
        "client_action_id",
        value,
        CLIENT_ACTION_ID_MAX_BYTES,
        TextMode::Token,
    )?;
    IdempotencyKey::new(value).map_err(|_| {
        WebUiInboundValidationError::new("client_action_id", WebUiInboundValidationCode::InvalidId)
    })
}

fn parse_thread_id(value: Option<String>) -> Result<ThreadId, WebUiInboundValidationError> {
    let value = required_text("thread_id", value, 256, TextMode::Token)?;
    parse_thread_id_value("thread_id", value)
}

fn parse_thread_id_value(
    field: &'static str,
    value: String,
) -> Result<ThreadId, WebUiInboundValidationError> {
    ThreadId::new(value)
        .map_err(|_| WebUiInboundValidationError::new(field, WebUiInboundValidationCode::InvalidId))
}

fn parse_run_id(value: Option<String>) -> Result<TurnRunId, WebUiInboundValidationError> {
    let value = required_text("run_id", value, 64, TextMode::Token)?;
    Uuid::parse_str(&value)
        .map(TurnRunId::from_uuid)
        .map_err(|_| {
            WebUiInboundValidationError::new("run_id", WebUiInboundValidationCode::InvalidId)
        })
}

fn parse_gate_ref(value: Option<String>) -> Result<GateRef, WebUiInboundValidationError> {
    let value = required_text("gate_ref", value, GATE_REF_MAX_BYTES, TextMode::Token)?;
    GateRef::new(value).map_err(|_| {
        WebUiInboundValidationError::new("gate_ref", WebUiInboundValidationCode::InvalidId)
    })
}

fn parse_cancel_reason(
    value: Option<String>,
) -> Result<SanitizedCancelReason, WebUiInboundValidationError> {
    let Some(value) = value else {
        return Ok(SanitizedCancelReason::UserRequested);
    };
    validate_text_value("reason", &value, 64, TextMode::Token)?;
    match value.as_str() {
        "user_requested" => Ok(SanitizedCancelReason::UserRequested),
        "superseded" => Ok(SanitizedCancelReason::Superseded),
        "timeout" => Ok(SanitizedCancelReason::Timeout),
        "operator_requested" => Ok(SanitizedCancelReason::OperatorRequested),
        "policy" => Ok(SanitizedCancelReason::Policy),
        _ => Err(WebUiInboundValidationError::new(
            "reason",
            WebUiInboundValidationCode::InvalidValue,
        )),
    }
}

fn parse_gate_resolution(
    resolution: Option<String>,
    always: Option<bool>,
    credential_ref: Option<String>,
) -> Result<WebUiGateResolution, WebUiInboundValidationError> {
    let resolution = required_text("resolution", resolution, 64, TextMode::Token)?;
    match resolution.as_str() {
        "approved" => Ok(WebUiGateResolution::Approved {
            always: always.unwrap_or(false),
        }),
        "denied" | "cancelled" => Ok(WebUiGateResolution::Declined),
        "credential_provided" => Ok(WebUiGateResolution::CredentialProvided {
            credential_ref: required_text(
                "credential_ref",
                credential_ref,
                CREDENTIAL_REF_MAX_BYTES,
                TextMode::Token,
            )?,
        }),
        _ => Err(WebUiInboundValidationError::new(
            "resolution",
            WebUiInboundValidationCode::InvalidValue,
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextMode {
    Token,
    MessageContent,
}

fn required_text(
    field: &'static str,
    value: Option<String>,
    max_bytes: usize,
    mode: TextMode,
) -> Result<String, WebUiInboundValidationError> {
    let value = value.ok_or_else(|| {
        WebUiInboundValidationError::new(field, WebUiInboundValidationCode::MissingField)
    })?;
    validate_text_value(field, &value, max_bytes, mode)?;
    Ok(value)
}

fn validate_text_value(
    field: &'static str,
    value: &str,
    max_bytes: usize,
    mode: TextMode,
) -> Result<(), WebUiInboundValidationError> {
    if value.trim().is_empty() {
        return Err(WebUiInboundValidationError::new(
            field,
            WebUiInboundValidationCode::Blank,
        ));
    }
    if value.len() > max_bytes {
        return Err(WebUiInboundValidationError::new(
            field,
            WebUiInboundValidationCode::TooLong,
        ));
    }
    let has_invalid_control = value.chars().any(|c| match mode {
        TextMode::Token => c == '\0' || c.is_control(),
        TextMode::MessageContent => c == '\0' || (c.is_control() && c != '\n' && c != '\t'),
    });
    if has_invalid_control {
        return Err(WebUiInboundValidationError::new(
            field,
            WebUiInboundValidationCode::InvalidControlCharacter,
        ));
    }
    Ok(())
}

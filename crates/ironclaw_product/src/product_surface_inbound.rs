//! Product-surface inbound DTO contract.
//!
//! These DTOs normalize authenticated terminal callers plus request bodies into
//! canonical Reborn commands without depending on route handlers, protocol auth
//! evidence, WASM, or adapter registries.

use ironclaw_attachments::InboundAttachment;
use ironclaw_host_api::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceValidationCode, ThreadId, TurnActor,
    TurnScope,
};
use ironclaw_turns::{CancelRunRequest, GateRef, IdempotencyKey, SanitizedCancelReason, TurnRunId};
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
const MAX_INLINE_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
const MAX_INLINE_TOTAL_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
const ATTACHMENT_FILENAME_MAX_BYTES: usize = 256;

/// Browser-facing inline-attachment contract advertised to the WebUI.
///
/// Carries the `accept` tokens generated from the shared
/// [`ironclaw_common`] format registry (so the file picker can never drift
/// from the server's allowed MIME set) plus the same budgets
/// [`ProductSubmitTurnRequest::decode_attachments`] enforces. The browser
/// uses this only for pre-submit hints; the server-side decode remains the
/// sole authority on what is accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductAttachmentCapabilities {
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
pub fn product_attachment_capabilities() -> ProductAttachmentCapabilities {
    ProductAttachmentCapabilities {
        accept: ironclaw_common::accept_tokens(),
        max_count: MAX_INLINE_ATTACHMENTS,
        max_file_bytes: MAX_INLINE_ATTACHMENT_BYTES,
        max_total_bytes: MAX_INLINE_TOTAL_ATTACHMENT_BYTES,
    }
}

/// Browser body for WebUI create-thread mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductCreateThreadRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_thread_id: Option<String>,
    /// Optional project the new thread should be scoped to. The browser only
    /// *proposes* it — the service authorizes the caller's access to the project
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
pub struct ProductInboundAttachment {
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub data_base64: String,
}

/// Browser body for WebUI send-message mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductSubmitTurnRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ProductInboundAttachment>,
    /// Caller-selected model for this turn. A hint routed to when the operator
    /// has it configured, otherwise the run falls back to the deployment's
    /// active model. The `"default"` alias and empty values are treated as "no
    /// selection". `None` for clients that don't pick a model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl ProductSubmitTurnRequest {
    /// Validate and decode the inline attachments into bytes-bearing
    /// [`InboundAttachment`]s ready for landing.
    ///
    /// Enforces the per-file / per-message / count budgets and rejects
    /// unsupported MIME types (per the shared format registry) and malformed
    /// base64 with a stable validation error. Kept separate from
    /// [`Self::into_command`] so the serializable command never carries raw
    /// bytes.
    pub fn decode_attachments(&self) -> Result<Vec<InboundAttachment>, ProductSurfaceError> {
        use base64::Engine;

        if self.attachments.len() > MAX_INLINE_ATTACHMENTS {
            return Err(validation_error(
                "attachments",
                ProductSurfaceValidationCode::TooLong,
            ));
        }

        let mut decoded = Vec::with_capacity(self.attachments.len());
        let mut total_bytes = 0usize;
        for (index, attachment) in self.attachments.iter().enumerate() {
            let mime = ironclaw_common::normalize_mime_type(&attachment.mime_type);
            if !ironclaw_common::is_supported_mime(&mime) {
                return Err(validation_error(
                    "attachments.mime_type",
                    ProductSurfaceValidationCode::InvalidValue,
                ));
            }

            let bytes = base64::engine::general_purpose::STANDARD
                .decode(attachment.data_base64.as_bytes())
                .map_err(|_| {
                    validation_error(
                        "attachments.data_base64",
                        ProductSurfaceValidationCode::InvalidValue,
                    )
                })?;
            if bytes.len() > MAX_INLINE_ATTACHMENT_BYTES {
                return Err(validation_error(
                    "attachments",
                    ProductSurfaceValidationCode::TooLong,
                ));
            }
            total_bytes = total_bytes.saturating_add(bytes.len());
            if total_bytes > MAX_INLINE_TOTAL_ATTACHMENT_BYTES {
                return Err(validation_error(
                    "attachments",
                    ProductSurfaceValidationCode::TooLong,
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
                return Err(validation_error(
                    "attachments.filename",
                    ProductSurfaceValidationCode::TooLong,
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
pub struct ProductCancelRunRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Browser body for WebUI failed-run retry mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductRetryRunRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

/// Browser query for WebUI list-threads read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductListThreadsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_thread_id: Option<String>,
    #[serde(default)]
    pub needs_approval: bool,
}

impl ProductListThreadsRequest {
    pub fn set_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn set_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    pub fn set_candidate_thread_id(mut self, candidate_thread_id: impl Into<String>) -> Self {
        self.candidate_thread_id = Some(candidate_thread_id.into());
        self
    }

    pub fn set_needs_approval(mut self, needs_approval: bool) -> Self {
        self.needs_approval = needs_approval;
        self
    }
}

/// Browser query for WebUI automation listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductListAutomationsRequest {
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

impl ProductListAutomationsRequest {
    pub fn set_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn set_run_limit(mut self, run_limit: u32) -> Self {
        self.run_limit = Some(run_limit);
        self
    }

    pub fn set_include_completed(mut self, include_completed: bool) -> Self {
        self.include_completed = include_completed;
        self
    }
}

/// Browser body for WebUI automation rename mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductRenameAutomationRequest {
    /// Optional at the DTO boundary so `{}` returns the stable field-level
    /// `missing_field` validation error instead of a generic JSON rejection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Browser body for WebUI extension-setup interaction.
///
/// This is the v2 entrypoint inventory's "extensions onboarding" row.
/// The native service exposes the route surface so callers can
/// inventory the API without v1 dependency. Concrete implementations return a
/// product-safe lifecycle projection; auth, approval, and pairing requirements
/// remain blockers owned by their dedicated Reborn services, not lifecycle
/// phases.
///
/// The package id is not part of the body — it is bound from the route
/// path and lifted into a lifecycle package ref by the handler before
/// it crosses the service boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductSetupExtensionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// Browser body for WebUI gate-resolution mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductResolveGateRequest {
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
    /// Attested-signing proof family for `resolution = "attested"`: one of
    /// `injected_wallet`, `near_redirect`, `wallet_connect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attested_proof_kind: Option<String>,
    /// Lowercase hex of the approved-tx hash the wallet attests to. Carried as
    /// an untrusted claim; the authoritative binding persisted at gate raise is
    /// what the proof is actually verified against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attested_approved_tx_hash: Option<String>,
    /// Opaque, provider-specific proof payload. Re-decoded by the continuation
    /// port; never interpreted here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attested_proof: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductCancelReason {
    UserRequested,
    Superseded,
    Timeout,
    OperatorRequested,
    Policy,
}

impl From<ProductCancelReason> for SanitizedCancelReason {
    fn from(value: ProductCancelReason) -> Self {
        match value {
            ProductCancelReason::UserRequested => Self::UserRequested,
            ProductCancelReason::Superseded => Self::Superseded,
            ProductCancelReason::Timeout => Self::Timeout,
            ProductCancelReason::OperatorRequested => Self::OperatorRequested,
            ProductCancelReason::Policy => Self::Policy,
        }
    }
}

// `Attested` carries an opaque `serde_json::Value` proof payload, which is
// `PartialEq` but not `Eq`; the enum therefore drops the `Eq` derive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "resolution", rename_all = "snake_case")]
pub enum ProductGateResolution {
    Approved {
        #[serde(default)]
        always: bool,
    },
    /// Unified decline variant — covers both user-initiated approval denial
    /// and auth-gate cancellation; the wire value is "declined".
    Declined,
    /// A host-stored credential reference, not a raw secret/token.
    CredentialProvided { credential_ref: String },
    /// An external-wallet / custodial attested-signing proof for a
    /// `BlockedAttested` gate. Validated-shape strings and JSON only — no trust
    /// is conferred here; the continuation port verifies against the
    /// authoritative binding.
    Attested {
        kind: String,
        approved_tx_hash_hex: String,
        proof_json: serde_json::Value,
    },
}

/// Canonical route-independent WebUI command produced after validation.
// `ResolveGate` carries a `ProductGateResolution`, whose `Attested` variant
// holds a non-`Eq` proof payload, so this enum drops `Eq` as well.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ProductInboundCommand {
    CreateThread {
        caller: ProductSurfaceCaller,
        client_action_id: IdempotencyKey,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requested_thread_id: Option<ThreadId>,
    },
    SendMessage {
        scope: TurnScope,
        actor: TurnActor,
        client_action_id: IdempotencyKey,
        content: String,
        /// Normalized caller-requested model hint (`"default"`/empty already
        /// dropped to `None`). Set on the submitted turn's `requested_model`.
        requested_model: Option<String>,
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
        resolution: ProductGateResolution,
    },
    RetryRun {
        scope: TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        client_action_id: IdempotencyKey,
    },
}

impl ProductCreateThreadRequest {
    pub fn into_command(
        self,
        caller: ProductSurfaceCaller,
    ) -> Result<ProductInboundCommand, ProductSurfaceError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let requested_thread_id = self
            .requested_thread_id
            .map(|value| parse_thread_id_value("requested_thread_id", value))
            .transpose()?;

        Ok(ProductInboundCommand::CreateThread {
            caller,
            client_action_id,
            requested_thread_id,
        })
    }
}

impl ProductSubmitTurnRequest {
    pub fn into_command(
        self,
        caller: ProductSurfaceCaller,
    ) -> Result<ProductInboundCommand, ProductSurfaceError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let thread_id = parse_thread_id(self.thread_id)?;
        let content = required_text(
            "content",
            self.content,
            USER_MESSAGE_TEXT_MAX_BYTES,
            TextMode::MessageContent,
        )?;

        Ok(ProductInboundCommand::SendMessage {
            scope: caller.turn_scope(thread_id),
            actor: caller.actor(),
            client_action_id,
            content,
            requested_model: self
                .model
                .as_deref()
                .and_then(ironclaw_common::model_selection::requested_model_hint),
        })
    }
}

impl ProductCancelRunRequest {
    pub fn into_command(
        self,
        caller: ProductSurfaceCaller,
    ) -> Result<ProductInboundCommand, ProductSurfaceError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let thread_id = parse_thread_id(self.thread_id)?;
        let run_id = parse_run_id(self.run_id)?;
        let reason = parse_cancel_reason(self.reason)?;

        Ok(ProductInboundCommand::CancelRun {
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

impl ProductRetryRunRequest {
    pub fn into_command(
        self,
        caller: ProductSurfaceCaller,
    ) -> Result<ProductInboundCommand, ProductSurfaceError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let thread_id = parse_thread_id(self.thread_id)?;
        let run_id = parse_run_id(self.run_id)?;

        Ok(ProductInboundCommand::RetryRun {
            scope: caller.turn_scope(thread_id),
            actor: caller.actor(),
            run_id,
            client_action_id,
        })
    }
}

impl ProductResolveGateRequest {
    pub fn into_command(
        self,
        caller: ProductSurfaceCaller,
    ) -> Result<ProductInboundCommand, ProductSurfaceError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let thread_id = parse_thread_id(self.thread_id)?;
        let run_id = parse_run_id(self.run_id)?;
        let gate_ref = parse_gate_ref(self.gate_ref)?;
        let resolution = parse_gate_resolution(
            self.resolution,
            self.always,
            self.credential_ref,
            self.attested_proof_kind,
            self.attested_approved_tx_hash,
            self.attested_proof,
        )?;

        Ok(ProductInboundCommand::ResolveGate {
            scope: caller.turn_scope(thread_id),
            actor: caller.actor(),
            run_id,
            gate_ref,
            client_action_id,
            resolution,
        })
    }
}

fn validation_error(
    field: &'static str,
    code: ProductSurfaceValidationCode,
) -> ProductSurfaceError {
    ProductSurfaceError::validation(field, code)
}

fn parse_client_action_id(value: Option<String>) -> Result<IdempotencyKey, ProductSurfaceError> {
    let value = required_text(
        "client_action_id",
        value,
        CLIENT_ACTION_ID_MAX_BYTES,
        TextMode::Token,
    )?;
    IdempotencyKey::new(value)
        .map_err(|_| validation_error("client_action_id", ProductSurfaceValidationCode::InvalidId))
}

fn parse_thread_id(value: Option<String>) -> Result<ThreadId, ProductSurfaceError> {
    let value = required_text("thread_id", value, 256, TextMode::Token)?;
    parse_thread_id_value("thread_id", value)
}

fn parse_thread_id_value(
    field: &'static str,
    value: String,
) -> Result<ThreadId, ProductSurfaceError> {
    ThreadId::new(value)
        .map_err(|_| validation_error(field, ProductSurfaceValidationCode::InvalidId))
}

fn parse_run_id(value: Option<String>) -> Result<TurnRunId, ProductSurfaceError> {
    let value = required_text("run_id", value, 64, TextMode::Token)?;
    Uuid::parse_str(&value)
        .map(TurnRunId::from_uuid)
        .map_err(|_| validation_error("run_id", ProductSurfaceValidationCode::InvalidId))
}

fn parse_gate_ref(value: Option<String>) -> Result<GateRef, ProductSurfaceError> {
    let value = required_text("gate_ref", value, GATE_REF_MAX_BYTES, TextMode::Token)?;
    GateRef::new(value)
        .map_err(|_| validation_error("gate_ref", ProductSurfaceValidationCode::InvalidId))
}

fn parse_cancel_reason(
    value: Option<String>,
) -> Result<SanitizedCancelReason, ProductSurfaceError> {
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
        _ => Err(validation_error(
            "reason",
            ProductSurfaceValidationCode::InvalidValue,
        )),
    }
}

fn parse_gate_resolution(
    resolution: Option<String>,
    always: Option<bool>,
    credential_ref: Option<String>,
    attested_proof_kind: Option<String>,
    attested_approved_tx_hash: Option<String>,
    attested_proof: Option<serde_json::Value>,
) -> Result<ProductGateResolution, ProductSurfaceError> {
    let resolution = required_text("resolution", resolution, 64, TextMode::Token)?;
    match resolution.as_str() {
        "approved" => Ok(ProductGateResolution::Approved {
            always: always.unwrap_or(false),
        }),
        "declined" => Ok(ProductGateResolution::Declined),
        "credential_provided" => Ok(ProductGateResolution::CredentialProvided {
            credential_ref: required_text(
                "credential_ref",
                credential_ref,
                CREDENTIAL_REF_MAX_BYTES,
                TextMode::Token,
            )?,
        }),
        "attested" => Ok(ProductGateResolution::Attested {
            kind: required_text(
                "attested_proof_kind",
                attested_proof_kind,
                64,
                TextMode::Token,
            )?,
            approved_tx_hash_hex: required_text(
                "attested_approved_tx_hash",
                attested_approved_tx_hash,
                128,
                TextMode::Token,
            )?,
            proof_json: attested_proof.ok_or_else(|| {
                validation_error("attested_proof", ProductSurfaceValidationCode::MissingField)
            })?,
        }),
        _ => Err(validation_error(
            "resolution",
            ProductSurfaceValidationCode::InvalidValue,
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
) -> Result<String, ProductSurfaceError> {
    let value =
        value.ok_or_else(|| validation_error(field, ProductSurfaceValidationCode::MissingField))?;
    validate_text_value(field, &value, max_bytes, mode)?;
    Ok(value)
}

fn validate_text_value(
    field: &'static str,
    value: &str,
    max_bytes: usize,
    mode: TextMode,
) -> Result<(), ProductSurfaceError> {
    if value.trim().is_empty() {
        return Err(validation_error(field, ProductSurfaceValidationCode::Blank));
    }
    if value.len() > max_bytes {
        return Err(validation_error(
            field,
            ProductSurfaceValidationCode::TooLong,
        ));
    }
    let has_invalid_control = value.chars().any(|c| match mode {
        TextMode::Token => c == '\0' || c.is_control(),
        TextMode::MessageContent => c == '\0' || (c.is_control() && c != '\n' && c != '\t'),
    });
    if has_invalid_control {
        return Err(validation_error(
            field,
            ProductSurfaceValidationCode::InvalidControlCharacter,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod attested_resolution_tests {
    use super::*;

    fn request(resolution: &str) -> ProductResolveGateRequest {
        ProductResolveGateRequest {
            client_action_id: Some("client-1".to_string()),
            thread_id: Some("thread-1".to_string()),
            run_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
            gate_ref: Some("gate:attested-1".to_string()),
            resolution: Some(resolution.to_string()),
            always: None,
            credential_ref: None,
            attested_proof_kind: Some("injected_wallet".to_string()),
            attested_approved_tx_hash: Some("ab".repeat(32)),
            attested_proof: Some(serde_json::json!({ "signature": "0xdead" })),
        }
    }

    #[test]
    fn an_attested_resolution_parses_into_the_attested_variant() {
        let parsed = parse_gate_resolution(
            Some("attested".to_string()),
            None,
            None,
            Some("injected_wallet".to_string()),
            Some("ab".repeat(32)),
            Some(serde_json::json!({ "signature": "0xdead" })),
        )
        .expect("attested resolution parses");
        let ProductGateResolution::Attested {
            kind,
            approved_tx_hash_hex,
            proof_json,
        } = parsed
        else {
            panic!("expected the attested variant");
        };
        assert_eq!(kind, "injected_wallet");
        assert_eq!(approved_tx_hash_hex, "ab".repeat(32));
        assert_eq!(proof_json["signature"], "0xdead");
    }

    /// Every attested field is required. A partially-specified proof must be a
    /// validation error, never a silently-defaulted claim the resolver would
    /// then carry to the device path.
    #[test]
    fn a_partial_attested_resolution_is_refused() {
        for (kind, hash, proof) in [
            (None, Some("ab".repeat(32)), Some(serde_json::json!({}))),
            (
                Some("injected_wallet".to_string()),
                None,
                Some(serde_json::json!({})),
            ),
            (
                Some("injected_wallet".to_string()),
                Some("ab".repeat(32)),
                None,
            ),
        ] {
            assert!(
                parse_gate_resolution(Some("attested".to_string()), None, None, kind, hash, proof)
                    .is_err(),
                "a partial attested resolution must fail validation"
            );
        }
    }

    /// The proof payload is opaque: this layer must not interpret it, so an
    /// arbitrary JSON shape passes through untouched to the continuation port.
    #[test]
    fn the_proof_payload_passes_through_uninterpreted() {
        let odd = serde_json::json!({ "nested": { "a": [1, 2, 3] }, "unknown_field": true });
        let parsed = parse_gate_resolution(
            Some("attested".to_string()),
            None,
            None,
            Some("wallet_connect".to_string()),
            Some("cd".repeat(32)),
            Some(odd.clone()),
        )
        .expect("parses");
        let ProductGateResolution::Attested { proof_json, .. } = parsed else {
            panic!("expected attested");
        };
        assert_eq!(proof_json, odd, "the payload must not be reshaped here");
    }

    #[test]
    fn a_non_attested_resolution_ignores_the_attested_fields() {
        let mut req = request("declined");
        req.resolution = Some("declined".to_string());
        let caller = ProductSurfaceCaller::new(
            ironclaw_host_api::TenantId::new("tenant-a").expect("tenant"),
            ironclaw_host_api::UserId::new("alice").expect("user"),
            None,
            None,
        );
        let command = req.into_command(caller).expect("declined parses");
        let ProductInboundCommand::ResolveGate { resolution, .. } = command else {
            panic!("expected ResolveGate");
        };
        assert_eq!(resolution, ProductGateResolution::Declined);
    }
}

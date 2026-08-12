//! Product-surface inbound normalization.
//!
//! The request *body shapes* moved to
//! [`ironclaw_product_contracts::inbound_requests`] with the WS5 port inversion
//! (PROPOSAL §6.1.3) so WebUI and the OpenAI-compatible adapter can construct
//! them without compiling this crate. What stays here is everything that could
//! not follow them across the contracts allowlist:
//!
//! - [`ProductInboundCommand`] — its `CancelRun` variant carries
//!   `ironclaw_turns::CancelRunRequest`.
//! - the validation/normalization that turns a body into a canonical command,
//!   which produces the command above and is not contract vocabulary in any case.
//!
//! The browser-facing attachment contract that used to live here
//! (`ProductAttachmentCapabilities` / `product_attachment_capabilities`) went
//! the other way, to `ironclaw_attachments`, with the WS5 attachments widening:
//! it is the advertised half of the size ceilings that crate enforces, and one
//! home for a ceiling is the point (PROPOSAL §6.4.9). Transports import
//! `ironclaw_attachments::attachment_capabilities` directly.
//!
//! Because the DTOs now live in another crate, the normalization is exposed as
//! the [`IntoProductInboundCommand`] and [`DecodeInboundAttachments`] extension
//! traits rather than inherent methods; call sites are unchanged apart from
//! bringing the trait into scope.

use ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS;
use ironclaw_host_api::turn::{IdempotencyKey, SanitizedCancelReason, TurnGateRef, TurnRunId};
use ironclaw_host_api::{
    attachment::InboundAttachment,
    ids::ThreadId,
    turn::{TurnActor, TurnScope},
};
use ironclaw_product_contracts::inbound_requests::{
    ProductCancelRunRequest, ProductCreateThreadRequest, ProductGateResolution,
    ProductResolveGateRequest, ProductRetryRunRequest, ProductSubmitTurnRequest,
};
use ironclaw_product_contracts::surface::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceValidationCode,
};
use ironclaw_turns::CancelRunRequest;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const CLIENT_ACTION_ID_MAX_BYTES: usize = 256;
const USER_MESSAGE_TEXT_MAX_BYTES: usize = 64 * 1024;
const GATE_REF_MAX_BYTES: usize = 256;
const CREDENTIAL_REF_MAX_BYTES: usize = 512;
const ATTACHMENT_FILENAME_MAX_BYTES: usize = 256;

/// Decode the inline attachments a submit-turn body carries into bytes-bearing
/// [`InboundAttachment`]s ready for landing.
pub trait DecodeInboundAttachments {
    /// Validate and decode the inline attachments.
    ///
    /// Enforces the per-file / per-message / count budgets and rejects
    /// unsupported MIME types (per the shared format registry) and malformed
    /// base64 with a stable validation error. Kept separate from
    /// [`IntoProductInboundCommand::into_command`] so the serializable command
    /// never carries raw bytes.
    fn decode_attachments(&self) -> Result<Vec<InboundAttachment>, ProductSurfaceError>;
}

impl DecodeInboundAttachments for ProductSubmitTurnRequest {
    fn decode_attachments(&self) -> Result<Vec<InboundAttachment>, ProductSurfaceError> {
        use base64::Engine;

        if self.attachments.len() > DEFAULT_ATTACHMENT_BUDGETS.max_count {
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
            if bytes.len() > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes {
                return Err(validation_error(
                    "attachments",
                    ProductSurfaceValidationCode::TooLong,
                ));
            }
            total_bytes = total_bytes.saturating_add(bytes.len());
            if total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes {
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

/// Canonical route-independent WebUI command produced after validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        /// The session channel extension named by the generic session-inbound
        /// route. `None` for transports submitting under the legacy session
        /// surface identity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extension_id: Option<String>,
    },
    CancelRun {
        request: CancelRunRequest,
    },
    ResolveGate {
        scope: TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        gate_ref: TurnGateRef,
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

/// Normalize a transport request body into the canonical inbound command.
///
/// One trait rather than five inherent methods because the bodies now live in
/// `ironclaw_product_contracts` and the normalization — which produces
/// [`ProductInboundCommand`] — cannot follow them there.
pub trait IntoProductInboundCommand {
    fn into_command(
        self,
        caller: ProductSurfaceCaller,
    ) -> Result<ProductInboundCommand, ProductSurfaceError>;
}

impl IntoProductInboundCommand for ProductCreateThreadRequest {
    fn into_command(
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

impl IntoProductInboundCommand for ProductSubmitTurnRequest {
    fn into_command(
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

        let extension_id = self
            .extension_id
            .map(|value| {
                validate_text_value("extension_id", &value, 256, TextMode::Token)?;
                Ok::<_, ProductSurfaceError>(value)
            })
            .transpose()?;

        Ok(ProductInboundCommand::SendMessage {
            scope: caller.turn_scope(thread_id),
            actor: caller.actor(),
            client_action_id,
            content,
            requested_model: self
                .model
                .as_deref()
                .and_then(ironclaw_common::model_selection::requested_model_hint),
            extension_id,
        })
    }
}

impl IntoProductInboundCommand for ProductCancelRunRequest {
    fn into_command(
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

impl IntoProductInboundCommand for ProductRetryRunRequest {
    fn into_command(
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

impl IntoProductInboundCommand for ProductResolveGateRequest {
    fn into_command(
        self,
        caller: ProductSurfaceCaller,
    ) -> Result<ProductInboundCommand, ProductSurfaceError> {
        let client_action_id = parse_client_action_id(self.client_action_id)?;
        let thread_id = parse_thread_id(self.thread_id)?;
        let run_id = parse_run_id(self.run_id)?;
        let gate_ref = parse_gate_ref(self.gate_ref)?;
        let resolution = parse_gate_resolution(self.resolution, self.always, self.credential_ref)?;

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

fn parse_gate_ref(value: Option<String>) -> Result<TurnGateRef, ProductSurfaceError> {
    let value = required_text("gate_ref", value, GATE_REF_MAX_BYTES, TextMode::Token)?;
    TurnGateRef::new(value)
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

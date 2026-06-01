//! Inbound envelope, payload, and acknowledgement types.

use chrono::{DateTime, Utc};
use ironclaw_turns::{AcceptedMessageRef, TurnRunId};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::auth::{ProtocolAuthEvidence, VerifiedAuthClaim};
use crate::error::ProductAdapterError;
use crate::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentDescriptor,
};
use crate::identity::{AdapterInstallationId, ProductAdapterId};
use crate::outbound::ProjectionCursor;
use crate::redaction::RedactedString;

const USER_MESSAGE_TEXT_MAX_BYTES: usize = 64 * 1024;
const COMMAND_MAX_BYTES: usize = 256;
const COMMAND_ARGUMENTS_MAX_BYTES: usize = 64 * 1024;
const THREAD_HINT_MAX_BYTES: usize = 512;
const ACTION_ID_MAX_BYTES: usize = 512;
const ACTION_DATA_MAX_BYTES: usize = 16 * 1024;
const INTERACTION_REF_MAX_BYTES: usize = 512;
const CREDENTIAL_REF_MAX_BYTES: usize = 512;

fn malformed(reason: impl Into<String>) -> ProductAdapterError {
    ProductAdapterError::MalformedInboundPayload {
        reason: RedactedString::new(reason.into()),
    }
}

fn validate_payload_string(
    kind: &'static str,
    value: &str,
    max: usize,
) -> Result<(), ProductAdapterError> {
    validate_bounded_string(kind, value, max, true, true)
}

fn validate_token_string(
    kind: &'static str,
    value: &str,
    max: usize,
) -> Result<(), ProductAdapterError> {
    validate_bounded_string(kind, value, max, false, false)
}

fn validate_command_name(value: &str) -> Result<(), ProductAdapterError> {
    validate_token_string("command", value, COMMAND_MAX_BYTES)?;
    if value
        .chars()
        .any(|c| c.is_whitespace() || c == '/' || c == '\\')
    {
        return Err(malformed(
            "command contains unsupported whitespace or slash characters",
        ));
    }
    Ok(())
}

fn validate_bounded_string(
    kind: &'static str,
    value: &str,
    max: usize,
    allow_empty: bool,
    allow_newline_tab: bool,
) -> Result<(), ProductAdapterError> {
    if !allow_empty && value.is_empty() {
        return Err(malformed(format!("{kind} must not be empty")));
    }
    if value.len() > max {
        return Err(malformed(format!("{kind} exceeds {max}-byte limit")));
    }
    if value
        .chars()
        .any(|c| c == '\0' || c.is_control() && !(allow_newline_tab && (c == '\n' || c == '\t')))
    {
        return Err(malformed(format!(
            "{kind} contains unsupported control characters"
        )));
    }
    Ok(())
}

/// Why an adapter is forwarding a group/supergroup/channel message into the
/// canonical pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductTriggerReason {
    DirectChat,
    BotMention,
    ReplyToBot,
    BotCommand,
    LinkedThreadAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserMessagePayload {
    pub text: String,
    pub attachments: Vec<ProductAttachmentDescriptor>,
    pub trigger: ProductTriggerReason,
}

impl UserMessagePayload {
    pub fn new(
        text: impl Into<String>,
        attachments: Vec<ProductAttachmentDescriptor>,
        trigger: ProductTriggerReason,
    ) -> Result<Self, ProductAdapterError> {
        let payload = Self {
            text: text.into(),
            attachments,
            trigger,
        };
        payload.validate()?;
        Ok(payload)
    }

    pub fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_payload_string("user message text", &self.text, USER_MESSAGE_TEXT_MAX_BYTES)
    }
}

#[derive(Deserialize)]
struct UserMessagePayloadWire {
    text: String,
    attachments: Vec<ProductAttachmentDescriptor>,
    trigger: ProductTriggerReason,
}

impl<'de> Deserialize<'de> for UserMessagePayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UserMessagePayloadWire::deserialize(deserializer)?;
        Self::new(wire.text, wire.attachments, wire.trigger).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InboundCommandPayload {
    pub command: String,
    pub arguments: String,
    pub trigger: ProductTriggerReason,
}

impl InboundCommandPayload {
    pub fn new(
        command: impl Into<String>,
        arguments: impl Into<String>,
        trigger: ProductTriggerReason,
    ) -> Result<Self, ProductAdapterError> {
        let command = command.into();
        let arguments = arguments.into();
        validate_command_name(&command)?;
        validate_payload_string("command arguments", &arguments, COMMAND_ARGUMENTS_MAX_BYTES)?;
        Ok(Self {
            command,
            arguments,
            trigger,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProductSlashCommandParseError {
    #[error("slash command is empty")]
    Empty,
    #[error("slash command payload is invalid: {0}")]
    InvalidPayload(String),
}

/// Parse a raw slash command into a normalized command payload. Returns
/// `Ok(None)` when the input is ordinary user text.
pub fn parse_product_slash_command(
    input: &str,
    trigger: ProductTriggerReason,
) -> Result<Option<InboundCommandPayload>, ProductSlashCommandParseError> {
    let trimmed = input.trim();
    let Some(without_slash) = trimmed.strip_prefix('/') else {
        return Ok(None);
    };
    let without_slash = without_slash.trim_start();
    if without_slash.is_empty() {
        return Err(ProductSlashCommandParseError::Empty);
    }

    let command_end = without_slash
        .char_indices()
        .find_map(|(idx, c)| c.is_whitespace().then_some(idx))
        .unwrap_or(without_slash.len());
    let command_slice = &without_slash[..command_end];
    let arguments_slice = without_slash[command_end..].trim_start();
    validate_command_name(command_slice)
        .map_err(|error| ProductSlashCommandParseError::InvalidPayload(error.to_string()))?;
    validate_payload_string(
        "command arguments",
        arguments_slice,
        COMMAND_ARGUMENTS_MAX_BYTES,
    )
    .map_err(|error| ProductSlashCommandParseError::InvalidPayload(error.to_string()))?;

    let command = command_slice.to_ascii_lowercase();
    let arguments = arguments_slice.to_string();
    InboundCommandPayload::new(command, arguments, trigger)
        .map(Some)
        .map_err(|error| ProductSlashCommandParseError::InvalidPayload(error.to_string()))
}

#[derive(Deserialize)]
struct InboundCommandPayloadWire {
    command: String,
    arguments: String,
    trigger: ProductTriggerReason,
}

impl<'de> Deserialize<'de> for InboundCommandPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = InboundCommandPayloadWire::deserialize(deserializer)?;
        Self::new(wire.command, wire.arguments, wire.trigger).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    ApproveOnce,
    Deny,
    AlwaysAllow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalResolutionPayload {
    pub gate_ref: String,
    pub decision: ApprovalDecision,
}

impl ApprovalResolutionPayload {
    pub fn new(
        gate_ref: impl Into<String>,
        decision: ApprovalDecision,
    ) -> Result<Self, ProductAdapterError> {
        let gate_ref = gate_ref.into();
        validate_token_string("gate ref", &gate_ref, INTERACTION_REF_MAX_BYTES)?;
        Ok(Self { gate_ref, decision })
    }
}

#[derive(Deserialize)]
struct ApprovalResolutionPayloadWire {
    gate_ref: String,
    decision: ApprovalDecision,
}

impl<'de> Deserialize<'de> for ApprovalResolutionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ApprovalResolutionPayloadWire::deserialize(deserializer)?;
        Self::new(wire.gate_ref, wire.decision).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthResolutionResult {
    CredentialProvided { credential_ref: String },
    CallbackCompleted { callback_ref: String },
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthResolutionPayload {
    pub auth_request_ref: String,
    pub result: AuthResolutionResult,
}

impl AuthResolutionPayload {
    pub fn new(
        auth_request_ref: impl Into<String>,
        result: AuthResolutionResult,
    ) -> Result<Self, ProductAdapterError> {
        let auth_request_ref = auth_request_ref.into();
        validate_token_string(
            "auth request ref",
            &auth_request_ref,
            INTERACTION_REF_MAX_BYTES,
        )?;
        validate_auth_resolution_result(&result)?;
        Ok(Self {
            auth_request_ref,
            result,
        })
    }
}

#[derive(Deserialize)]
struct AuthResolutionPayloadWire {
    auth_request_ref: String,
    result: AuthResolutionResult,
}

impl<'de> Deserialize<'de> for AuthResolutionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AuthResolutionPayloadWire::deserialize(deserializer)?;
        Self::new(wire.auth_request_ref, wire.result).map_err(serde::de::Error::custom)
    }
}

fn validate_auth_resolution_result(
    result: &AuthResolutionResult,
) -> Result<(), ProductAdapterError> {
    match result {
        AuthResolutionResult::CredentialProvided { credential_ref } => {
            validate_token_string("credential ref", credential_ref, CREDENTIAL_REF_MAX_BYTES)
        }
        AuthResolutionResult::CallbackCompleted { callback_ref } => {
            validate_token_string("callback ref", callback_ref, INTERACTION_REF_MAX_BYTES)
        }
        AuthResolutionResult::Denied => Ok(()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectionSubscriptionPayload {
    pub thread_id_hint: Option<String>,
    pub after_cursor: Option<ProjectionCursor>,
}

impl ProjectionSubscriptionPayload {
    pub fn new(
        thread_id_hint: Option<String>,
        after_cursor: Option<ProjectionCursor>,
    ) -> Result<Self, ProductAdapterError> {
        if let Some(hint) = &thread_id_hint {
            validate_token_string("thread id hint", hint, THREAD_HINT_MAX_BYTES)?;
        }
        Ok(Self {
            thread_id_hint,
            after_cursor,
        })
    }
}

#[derive(Deserialize)]
struct ProjectionSubscriptionPayloadWire {
    thread_id_hint: Option<String>,
    after_cursor: Option<ProjectionCursor>,
}

impl<'de> Deserialize<'de> for ProjectionSubscriptionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProjectionSubscriptionPayloadWire::deserialize(deserializer)?;
        Self::new(wire.thread_id_hint, wire.after_cursor).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkedThreadActionPayload {
    pub action_id: String,
    pub data: Option<String>,
    pub reply_target_message_id: Option<String>,
}

impl LinkedThreadActionPayload {
    pub fn new(
        action_id: impl Into<String>,
        data: Option<String>,
        reply_target_message_id: Option<String>,
    ) -> Result<Self, ProductAdapterError> {
        let action_id = action_id.into();
        validate_token_string("linked action id", &action_id, ACTION_ID_MAX_BYTES)?;
        if let Some(data) = &data {
            validate_payload_string("linked action data", data, ACTION_DATA_MAX_BYTES)?;
        }
        if let Some(reply_target_message_id) = &reply_target_message_id {
            validate_token_string(
                "linked action reply target",
                reply_target_message_id,
                INTERACTION_REF_MAX_BYTES,
            )?;
        }
        Ok(Self {
            action_id,
            data,
            reply_target_message_id,
        })
    }
}

#[derive(Deserialize)]
struct LinkedThreadActionPayloadWire {
    action_id: String,
    data: Option<String>,
    reply_target_message_id: Option<String>,
}

impl<'de> Deserialize<'de> for LinkedThreadActionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = LinkedThreadActionPayloadWire::deserialize(deserializer)?;
        Self::new(wire.action_id, wire.data, wire.reply_target_message_id)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductInboundPayload {
    UserMessage(UserMessagePayload),
    Command(InboundCommandPayload),
    ApprovalResolution(ApprovalResolutionPayload),
    AuthResolution(AuthResolutionPayload),
    SubscriptionRequest(ProjectionSubscriptionPayload),
    LinkedThreadAction(LinkedThreadActionPayload),
    NoOp,
}

/// Adapter-produced parse result. It deliberately excludes host-trusted fields
/// (adapter id, installation id, verified auth claim, and received_at).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedProductInbound {
    pub external_event_id: ExternalEventId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub payload: ProductInboundPayload,
}

impl ParsedProductInbound {
    pub fn new(
        external_event_id: ExternalEventId,
        external_actor_ref: ExternalActorRef,
        external_conversation_ref: ExternalConversationRef,
        payload: ProductInboundPayload,
    ) -> Result<Self, ProductAdapterError> {
        Ok(Self {
            external_event_id,
            external_actor_ref,
            external_conversation_ref,
            payload,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedInboundContext {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
    received_at: DateTime<Utc>,
    auth_claim: VerifiedAuthClaim,
}

impl TrustedInboundContext {
    pub fn from_verified_evidence(
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        received_at: DateTime<Utc>,
        auth_evidence: &ProtocolAuthEvidence,
    ) -> Result<Self, ProductAdapterError> {
        let auth_claim =
            auth_evidence
                .claim()
                .cloned()
                .ok_or(ProductAdapterError::Authentication(
                    crate::ProtocolAuthFailure::Missing,
                ))?;
        Ok(Self {
            adapter_id,
            installation_id,
            received_at,
            auth_claim,
        })
    }
}

/// Trusted inbound envelope handed to the workflow facade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProductInboundEnvelope {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
    external_event_id: ExternalEventId,
    external_actor_ref: ExternalActorRef,
    external_conversation_ref: ExternalConversationRef,
    auth_claim: VerifiedAuthClaim,
    received_at: DateTime<Utc>,
    payload: ProductInboundPayload,
}

impl ProductInboundEnvelope {
    pub fn from_trusted_parse(
        context: TrustedInboundContext,
        parsed: ParsedProductInbound,
    ) -> Result<Self, ProductAdapterError> {
        Ok(Self {
            adapter_id: context.adapter_id,
            installation_id: context.installation_id,
            external_event_id: parsed.external_event_id,
            external_actor_ref: parsed.external_actor_ref,
            external_conversation_ref: parsed.external_conversation_ref,
            auth_claim: context.auth_claim,
            received_at: context.received_at,
            payload: parsed.payload,
        })
    }

    pub fn adapter_id(&self) -> &ProductAdapterId {
        &self.adapter_id
    }

    pub fn installation_id(&self) -> &AdapterInstallationId {
        &self.installation_id
    }

    pub fn external_event_id(&self) -> &ExternalEventId {
        &self.external_event_id
    }

    pub fn external_actor_ref(&self) -> &ExternalActorRef {
        &self.external_actor_ref
    }

    pub fn external_conversation_ref(&self) -> &ExternalConversationRef {
        &self.external_conversation_ref
    }

    pub fn auth_claim(&self) -> &VerifiedAuthClaim {
        &self.auth_claim
    }

    pub fn received_at(&self) -> DateTime<Utc> {
        self.received_at
    }

    pub fn payload(&self) -> &ProductInboundPayload {
        &self.payload
    }

    /// Preserve host-stamped trusted context while replacing only the
    /// user-message payload after workflow-owned before-inbound policy rewrite.
    pub fn with_rewritten_user_message(
        &self,
        payload: UserMessagePayload,
    ) -> Result<Self, ProductAdapterError> {
        if !matches!(self.payload(), ProductInboundPayload::UserMessage(_)) {
            return Err(malformed("cannot rewrite non-user-message payload"));
        }
        payload.validate()?;
        let mut envelope = self.clone();
        envelope.payload = ProductInboundPayload::UserMessage(payload);
        Ok(envelope)
    }

    pub fn source_binding_key(&self) -> String {
        self.external_conversation_ref.conversation_fingerprint()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductRejectionKind {
    BindingRequired,
    AccessDenied,
    UnknownInstallation,
    InvalidRequest,
    PolicyDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductRejectionDisposition {
    Permanent,
    Retryable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductRejection {
    pub kind: ProductRejectionKind,
    pub reason: RedactedString,
    pub disposition: ProductRejectionDisposition,
}

impl ProductRejection {
    pub fn permanent(kind: ProductRejectionKind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            reason: RedactedString::new(reason.into()),
            disposition: ProductRejectionDisposition::Permanent,
        }
    }

    pub fn retryable(kind: ProductRejectionKind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            reason: RedactedString::new(reason.into()),
            disposition: ProductRejectionDisposition::Retryable,
        }
    }

    pub fn disposition(&self) -> ProductRejectionDisposition {
        self.disposition
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboundRetryDisposition {
    DoNotRetry,
    Retry,
    ReplayPrior,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProductCommandResultPayload(Value);

impl Eq for ProductCommandResultPayload {}

impl ProductCommandResultPayload {
    pub fn new(value: Value) -> Self {
        Self(value)
    }

    pub fn as_value(&self) -> &Value {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductInboundAck {
    Accepted {
        accepted_message_ref: AcceptedMessageRef,
        submitted_run_id: TurnRunId,
    },
    DeferredBusy {
        accepted_message_ref: AcceptedMessageRef,
        active_run_id: TurnRunId,
    },
    Rejected(ProductRejection),
    CommandResult {
        command: String,
        payload: ProductCommandResultPayload,
    },
    Duplicate {
        prior: Box<ProductInboundAck>,
    },
    NoOp,
}

impl ProductInboundAck {
    pub fn is_durable_outcome(&self) -> bool {
        match self {
            Self::Accepted { .. }
            | Self::DeferredBusy { .. }
            | Self::Duplicate { .. }
            | Self::CommandResult { .. }
            | Self::NoOp => true,
            Self::Rejected(rejection) => {
                rejection.disposition == ProductRejectionDisposition::Permanent
            }
        }
    }

    pub fn retry_disposition(&self) -> InboundRetryDisposition {
        match self {
            Self::Rejected(rejection)
                if rejection.disposition == ProductRejectionDisposition::Retryable =>
            {
                InboundRetryDisposition::Retry
            }
            Self::Duplicate { .. } => InboundRetryDisposition::ReplayPrior,
            _ => InboundRetryDisposition::DoNotRetry,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthRequirement;
    use crate::external::{ExternalActorRef, ExternalConversationRef, ExternalEventId};

    fn sample_context() -> TrustedInboundContext {
        let evidence = ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
            },
            "telegram_install_alpha",
        );
        TrustedInboundContext::from_verified_evidence(
            ProductAdapterId::new("telegram_v2").expect("valid"),
            AdapterInstallationId::new("install_alpha").expect("valid"),
            Utc::now(),
            &evidence,
        )
        .expect("verified")
    }

    fn sample_parsed(payload: ProductInboundPayload) -> ParsedProductInbound {
        ParsedProductInbound::new(
            ExternalEventId::new("update:42").expect("valid"),
            ExternalActorRef::new("telegram_user", "777", Option::<String>::None).expect("valid"),
            ExternalConversationRef::new(None, "12345", Some("topic-7"), Some("msg-100"))
                .expect("valid"),
            payload,
        )
        .expect("parsed")
    }

    #[test]
    fn user_message_text_length_bounded() {
        let oversize = "a".repeat(USER_MESSAGE_TEXT_MAX_BYTES + 1);
        assert!(
            UserMessagePayload::new(oversize, vec![], ProductTriggerReason::DirectChat).is_err()
        );
    }

    #[test]
    fn user_message_text_length_bounded_through_serde() {
        let empty = serde_json::json!({
            "text": "",
            "attachments": [],
            "trigger": "direct_chat"
        });
        assert!(serde_json::from_value::<UserMessagePayload>(empty).is_ok());

        let at_limit = serde_json::json!({
            "text": "a".repeat(USER_MESSAGE_TEXT_MAX_BYTES),
            "attachments": [],
            "trigger": "direct_chat"
        });
        assert!(serde_json::from_value::<UserMessagePayload>(at_limit).is_ok());

        let forged = serde_json::json!({
            "text": "a".repeat(USER_MESSAGE_TEXT_MAX_BYTES + 1),
            "attachments": [],
            "trigger": "direct_chat"
        });
        assert!(serde_json::from_value::<UserMessagePayload>(forged).is_err());
    }

    #[test]
    fn command_payload_bounds_are_enforced_through_serde() {
        assert!(
            InboundCommandPayload::new(
                "h".repeat(COMMAND_MAX_BYTES + 1),
                "",
                ProductTriggerReason::BotCommand
            )
            .is_err()
        );
        assert!(
            InboundCommandPayload::new("bad name", "", ProductTriggerReason::BotCommand).is_err()
        );
        assert!(
            InboundCommandPayload::new("bad/name", "", ProductTriggerReason::BotCommand).is_err()
        );
        let empty_command = serde_json::json!({
            "command": "",
            "arguments": "",
            "trigger": "bot_command"
        });
        assert!(serde_json::from_value::<InboundCommandPayload>(empty_command).is_err());

        let at_limit = serde_json::json!({
            "command": "h".repeat(COMMAND_MAX_BYTES),
            "arguments": "",
            "trigger": "bot_command"
        });
        assert!(serde_json::from_value::<InboundCommandPayload>(at_limit).is_ok());

        let forged = serde_json::json!({
            "command": "h".repeat(COMMAND_MAX_BYTES + 1),
            "arguments": "",
            "trigger": "bot_command"
        });
        assert!(serde_json::from_value::<InboundCommandPayload>(forged).is_err());

        let forged_slash = serde_json::json!({
            "command": "bad/name",
            "arguments": "",
            "trigger": "bot_command"
        });
        assert!(serde_json::from_value::<InboundCommandPayload>(forged_slash).is_err());
    }

    #[test]
    fn envelope_is_built_from_trusted_context() {
        let envelope = ProductInboundEnvelope::from_trusted_parse(
            sample_context(),
            sample_parsed(ProductInboundPayload::NoOp),
        )
        .expect("envelope");
        assert_eq!(envelope.adapter_id().as_str(), "telegram_v2");
        assert_eq!(envelope.payload(), &ProductInboundPayload::NoOp);
    }

    #[test]
    fn rewritten_user_message_rejects_non_user_message_envelope() {
        let envelope = ProductInboundEnvelope::from_trusted_parse(
            sample_context(),
            sample_parsed(ProductInboundPayload::NoOp),
        )
        .expect("envelope");
        let rewrite =
            UserMessagePayload::new("rewritten", vec![], ProductTriggerReason::DirectChat)
                .expect("valid rewrite");

        let err = envelope
            .with_rewritten_user_message(rewrite)
            .expect_err("non-user-message envelope must not be rewritten");

        assert!(matches!(
            err,
            ProductAdapterError::MalformedInboundPayload { .. }
        ));
    }

    #[test]
    fn failed_auth_cannot_build_context() {
        let evidence = ProtocolAuthEvidence::failed(crate::ProtocolAuthFailure::Missing);
        assert!(
            TrustedInboundContext::from_verified_evidence(
                ProductAdapterId::new("telegram_v2").expect("valid"),
                AdapterInstallationId::new("install_alpha").expect("valid"),
                Utc::now(),
                &evidence,
            )
            .is_err()
        );
    }

    #[test]
    fn ack_durable_outcomes_classify_correctly() {
        assert!(
            ProductInboundAck::Accepted {
                accepted_message_ref: AcceptedMessageRef::new("msg").expect("valid"),
                submitted_run_id: TurnRunId::new(),
            }
            .is_durable_outcome()
        );
        assert!(ProductInboundAck::NoOp.is_durable_outcome());
        assert!(
            ProductInboundAck::CommandResult {
                command: "extension_install".to_string(),
                payload: ProductCommandResultPayload::new(serde_json::json!({
                    "phase": "installed",
                })),
            }
            .is_durable_outcome()
        );
        assert!(
            ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                "policy denied",
            ))
            .is_durable_outcome()
        );
        assert!(
            !ProductInboundAck::Rejected(ProductRejection::retryable(
                ProductRejectionKind::PolicyDenied,
                "rate limited",
            ))
            .is_durable_outcome()
        );
        assert_eq!(
            ProductInboundAck::Duplicate {
                prior: Box::new(ProductInboundAck::NoOp),
            }
            .retry_disposition(),
            InboundRetryDisposition::ReplayPrior
        );
    }
}

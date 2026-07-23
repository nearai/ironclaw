//! Inbound envelope, payload, and acknowledgement types.

use std::fmt;

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
const REQUESTED_MODEL_MAX_BYTES: usize = 256;
const COMMAND_MAX_BYTES: usize = 256;
const COMMAND_ARGUMENTS_MAX_BYTES: usize = 64 * 1024;
const THREAD_HINT_MAX_BYTES: usize = 512;
const ACTION_ID_MAX_BYTES: usize = 512;
const ACTION_DATA_MAX_BYTES: usize = 16 * 1024;
const INTERACTION_REF_MAX_BYTES: usize = 512;
const CREDENTIAL_REF_MAX_BYTES: usize = 512;
const SOURCE_CHANNEL_MAX_BYTES: usize = 512;

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

/// Optional host-side reclassification for a normalized channel message before
/// it enters the product surface.
///
/// `None` at the call site means the normalized message is an ordinary user
/// message. These variants cover only protocol-specific interaction replies
/// that should not become user turns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelInboundClassification {
    ApprovalResolution(ApprovalResolutionPayload),
    ScopedApprovalResolution(ScopedApprovalResolutionPayload),
    AuthResolution(AuthResolutionPayload),
    NoOp,
}

impl From<ChannelInboundClassification> for ProductInboundPayload {
    fn from(classification: ChannelInboundClassification) -> Self {
        match classification {
            ChannelInboundClassification::ApprovalResolution(payload) => {
                Self::ApprovalResolution(payload)
            }
            ChannelInboundClassification::ScopedApprovalResolution(payload) => {
                Self::ScopedApprovalResolution(payload)
            }
            ChannelInboundClassification::AuthResolution(payload) => Self::AuthResolution(payload),
            ChannelInboundClassification::NoOp => Self::NoOp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserMessagePayload {
    pub text: String,
    pub attachments: Vec<ProductAttachmentDescriptor>,
    pub trigger: ProductTriggerReason,
    /// Caller-requested model for this turn (e.g. an OpenAI-compatible client's
    /// `model` field). A model *hint*, not authority: the coordinator routes to
    /// it only when the operator has it configured, otherwise it falls back to
    /// the deployment's active model. `None` for surfaces that don't select a
    /// model (chat UI, channels).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_model: Option<String>,
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
            requested_model: None,
        };
        payload.validate()?;
        Ok(payload)
    }

    /// Attach a caller-requested model to this payload. See
    /// [`UserMessagePayload::requested_model`].
    pub fn with_requested_model(mut self, requested_model: Option<String>) -> Self {
        self.requested_model = requested_model.filter(|model| !model.is_empty());
        self
    }

    pub fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_payload_string("user message text", &self.text, USER_MESSAGE_TEXT_MAX_BYTES)?;
        if let Some(model) = &self.requested_model {
            validate_payload_string("requested model", model, REQUESTED_MODEL_MAX_BYTES)?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct UserMessagePayloadWire {
    text: String,
    attachments: Vec<ProductAttachmentDescriptor>,
    trigger: ProductTriggerReason,
    #[serde(default)]
    requested_model: Option<String>,
}

impl<'de> Deserialize<'de> for UserMessagePayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UserMessagePayloadWire::deserialize(deserializer)?;
        let payload = Self::new(wire.text, wire.attachments, wire.trigger)
            .map(|payload| payload.with_requested_model(wire.requested_model))
            .map_err(serde::de::Error::custom)?;
        // `new` validated the payload while `requested_model` was still `None`;
        // re-validate the assembled value so the wire-supplied model hint is
        // bounded like every other ingress field (bypass flagged in PR review).
        payload.validate().map_err(serde::de::Error::custom)?;
        Ok(payload)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trigger: Option<ProductTriggerReason>,
}

impl ApprovalResolutionPayload {
    pub fn new(
        gate_ref: impl Into<String>,
        decision: ApprovalDecision,
    ) -> Result<Self, ProductAdapterError> {
        let gate_ref = gate_ref.into();
        validate_token_string("gate ref", &gate_ref, INTERACTION_REF_MAX_BYTES)?;
        Ok(Self {
            gate_ref,
            decision,
            source_trigger: None,
        })
    }

    pub fn with_source_trigger(mut self, source_trigger: ProductTriggerReason) -> Self {
        self.source_trigger = Some(source_trigger);
        self
    }
}

/// Approval command scoped by the current product conversation/actor binding.
///
/// Surfaces use this for thread-local shorthand such as `approve` / `deny`
/// where the gate reference is intentionally resolved by the trusted workflow
/// layer instead of being supplied by the adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScopedApprovalResolutionPayload {
    pub decision: ApprovalDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trigger: Option<ProductTriggerReason>,
}

impl ScopedApprovalResolutionPayload {
    pub fn new(decision: ApprovalDecision) -> Result<Self, ProductAdapterError> {
        Ok(Self {
            decision,
            source_trigger: None,
        })
    }

    pub fn with_source_trigger(mut self, source_trigger: ProductTriggerReason) -> Self {
        self.source_trigger = Some(source_trigger);
        self
    }
}

#[derive(Deserialize)]
struct ApprovalResolutionPayloadWire {
    gate_ref: String,
    decision: ApprovalDecision,
    source_trigger: Option<ProductTriggerReason>,
}

impl<'de> Deserialize<'de> for ApprovalResolutionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ApprovalResolutionPayloadWire::deserialize(deserializer)?;
        let payload = Self::new(wire.gate_ref, wire.decision).map_err(serde::de::Error::custom)?;
        Ok(match wire.source_trigger {
            Some(source_trigger) => payload.with_source_trigger(source_trigger),
            None => payload,
        })
    }
}

#[derive(Deserialize)]
struct ScopedApprovalResolutionPayloadWire {
    decision: ApprovalDecision,
    source_trigger: Option<ProductTriggerReason>,
}

impl<'de> Deserialize<'de> for ScopedApprovalResolutionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ScopedApprovalResolutionPayloadWire::deserialize(deserializer)?;
        let payload = Self::new(wire.decision).map_err(serde::de::Error::custom)?;
        Ok(match wire.source_trigger {
            Some(source_trigger) => payload.with_source_trigger(source_trigger),
            None => payload,
        })
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trigger: Option<ProductTriggerReason>,
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
            source_trigger: None,
        })
    }

    pub fn with_source_trigger(mut self, source_trigger: ProductTriggerReason) -> Self {
        self.source_trigger = Some(source_trigger);
        self
    }
}

#[derive(Deserialize)]
struct AuthResolutionPayloadWire {
    auth_request_ref: String,
    result: AuthResolutionResult,
    source_trigger: Option<ProductTriggerReason>,
}

impl<'de> Deserialize<'de> for AuthResolutionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AuthResolutionPayloadWire::deserialize(deserializer)?;
        let payload =
            Self::new(wire.auth_request_ref, wire.result).map_err(serde::de::Error::custom)?;
        Ok(match wire.source_trigger {
            Some(source_trigger) => payload.with_source_trigger(source_trigger),
            None => payload,
        })
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
pub struct ProjectionReadPayload {
    pub thread_id_hint: Option<String>,
    pub after_cursor: Option<ProjectionCursor>,
    pub limit: Option<u16>,
}

impl ProjectionReadPayload {
    pub fn new(
        thread_id_hint: Option<String>,
        after_cursor: Option<ProjectionCursor>,
        limit: Option<u16>,
    ) -> Result<Self, ProductAdapterError> {
        if let Some(hint) = &thread_id_hint {
            validate_token_string("thread id hint", hint, THREAD_HINT_MAX_BYTES)?;
        }
        Ok(Self {
            thread_id_hint,
            after_cursor,
            limit,
        })
    }
}

#[derive(Deserialize)]
struct ProjectionReadPayloadWire {
    thread_id_hint: Option<String>,
    after_cursor: Option<ProjectionCursor>,
    limit: Option<u16>,
}

impl<'de> Deserialize<'de> for ProjectionReadPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProjectionReadPayloadWire::deserialize(deserializer)?;
        Self::new(wire.thread_id_hint, wire.after_cursor, wire.limit)
            .map_err(serde::de::Error::custom)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ProductControlActionPayload {
    CancelRun { run_id: TurnRunId },
}

impl ProductControlActionPayload {
    pub fn cancel_run(run_id: &str) -> Result<Self, ProductAdapterError> {
        let run_id =
            TurnRunId::parse(run_id).map_err(|_| ProductAdapterError::MalformedInboundPayload {
                reason: RedactedString::new("invalid run id"),
            })?;
        Ok(Self::CancelRun { run_id })
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
    ScopedApprovalResolution(ScopedApprovalResolutionPayload),
    AuthResolution(AuthResolutionPayload),
    ProjectionRead(ProjectionReadPayload),
    SubscriptionRequest(ProjectionSubscriptionPayload),
    ControlAction(ProductControlActionPayload),
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

/// Product-facing source channel stamped by ingress before workflow admission.
///
/// This is intentionally distinct from adapter installation identity: first-party
/// ingress can stamp a first-party terminal name, while external adapters
/// usually default to their adapter id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProductSourceChannel(String);

impl ProductSourceChannel {
    pub fn new(value: impl Into<String>) -> Result<Self, ProductAdapterError> {
        let value = value.into();
        validate_token_string("source_channel", &value, SOURCE_CHANNEL_MAX_BYTES)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl TryFrom<String> for ProductSourceChannel {
    type Error = ProductAdapterError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ProductSourceChannel> for String {
    fn from(value: ProductSourceChannel) -> Self {
        value.0
    }
}

impl fmt::Display for ProductSourceChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedInboundContext {
    adapter_id: ProductAdapterId,
    source_channel: ProductSourceChannel,
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
        let source_channel = ProductSourceChannel::new(adapter_id.as_str())?;
        Ok(Self {
            adapter_id,
            source_channel,
            installation_id,
            received_at,
            auth_claim,
        })
    }

    pub fn from_verified_evidence_with_source_channel(
        adapter_id: ProductAdapterId,
        source_channel: ProductSourceChannel,
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
            source_channel,
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
    source_channel: ProductSourceChannel,
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
            source_channel: context.source_channel,
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

    pub fn source_channel(&self) -> &ProductSourceChannel {
        &self.source_channel
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
    AmbiguousResolution,
    /// The approval gate was already approved or denied — it is no longer pending.
    /// Distinct from `PolicyDenied`, which means an active policy refused the request.
    StaleGate,
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

impl ProductRejectionKind {
    /// Returns a sanitized, user-facing hint for this rejection kind.
    ///
    /// Never interpolates internal state, reasons, or redacted strings.
    pub fn user_facing_hint(&self) -> &'static str {
        match self {
            Self::BindingRequired => {
                "I couldn't match this reply to an active conversation. Reply in the approval thread, or use `approve gate:<ref>`."
            }
            Self::AccessDenied => "You don't have access to resolve this request.",
            Self::UnknownInstallation => "This workspace isn't set up with IronClaw yet.",
            Self::InvalidRequest => {
                "I couldn't read that request. Use `approve` / `deny`, optionally with `gate:<ref>`."
            }
            Self::PolicyDenied => "That request was declined by policy.",
            Self::AmbiguousResolution => {
                "Multiple requests are pending in this conversation. Use `approve gate:<ref>` or `deny gate:<ref>` to pick one."
            }
            Self::StaleGate => {
                "This approval request is no longer pending — it was already approved or denied."
            }
        }
    }

    /// Auth-resolution-flavored variant of [`Self::user_facing_hint`]: kinds whose
    /// generic hint references approval commands get auth-specific guidance
    /// (`auth deny <auth-request-ref>`); all other kinds reuse the generic hint.
    pub fn user_facing_auth_hint(&self) -> &'static str {
        match self {
            Self::BindingRequired => {
                "I couldn't match this reply to an active auth request. Reply in the auth prompt thread, or use `auth deny <auth-request-ref>` to decline."
            }
            Self::InvalidRequest => {
                "I couldn't read that request. Use `auth deny <auth-request-ref>` to decline an auth request."
            }
            Self::AmbiguousResolution => {
                "Multiple auth requests are pending in this conversation. Use `auth deny <auth-request-ref>` to target a specific one."
            }
            _ => self.user_facing_hint(),
        }
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
    RejectedBusy {
        accepted_message_ref: AcceptedMessageRef,
        active_run_id: Option<TurnRunId>,
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
            | Self::RejectedBusy { .. }
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

    #[test]
    fn user_message_payload_round_trips_and_filters_requested_model() {
        let with_model = UserMessagePayload::new("hi", vec![], ProductTriggerReason::DirectChat)
            .unwrap()
            .with_requested_model(Some("gpt-4o".to_string()));
        assert_eq!(with_model.requested_model.as_deref(), Some("gpt-4o"));
        // Round-trips over the wire (custom Deserialize via the wire struct).
        let decoded: UserMessagePayload =
            serde_json::from_str(&serde_json::to_string(&with_model).unwrap()).unwrap();
        assert_eq!(decoded.requested_model.as_deref(), Some("gpt-4o"));

        // Omitted → None, and not serialized when absent.
        let without =
            UserMessagePayload::new("hi", vec![], ProductTriggerReason::DirectChat).unwrap();
        assert!(without.requested_model.is_none());
        assert!(
            !serde_json::to_string(&without)
                .unwrap()
                .contains("requested_model")
        );

        // An empty requested model is filtered to None.
        assert!(
            UserMessagePayload::new("hi", vec![], ProductTriggerReason::DirectChat)
                .unwrap()
                .with_requested_model(Some(String::new()))
                .requested_model
                .is_none()
        );
    }

    #[test]
    fn user_message_payload_bounds_requested_model_on_every_path() {
        let over_limit = "m".repeat(REQUESTED_MODEL_MAX_BYTES + 1);

        // Explicit validation after the builder rejects an over-long hint.
        let built = UserMessagePayload::new("hi", vec![], ProductTriggerReason::DirectChat)
            .unwrap()
            .with_requested_model(Some(over_limit.clone()));
        assert!(built.validate().is_err());

        // Deserialization must not smuggle an unbounded hint past validation:
        // the wire path attaches `requested_model` after `new`, so it re-validates.
        let wire = serde_json::json!({
            "text": "hi",
            "attachments": [],
            "trigger": "direct_chat",
            "requested_model": over_limit,
        })
        .to_string();
        let decoded: Result<UserMessagePayload, _> = serde_json::from_str(&wire);
        assert!(
            decoded.is_err(),
            "an over-long requested_model must be rejected during deserialization"
        );

        // A hint at the cap is accepted on both paths.
        let at_cap = "m".repeat(REQUESTED_MODEL_MAX_BYTES);
        assert!(
            UserMessagePayload::new("hi", vec![], ProductTriggerReason::DirectChat)
                .unwrap()
                .with_requested_model(Some(at_cap))
                .validate()
                .is_ok()
        );
    }

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
        assert_eq!(envelope.source_channel().as_str(), "telegram_v2");
        assert_eq!(envelope.payload(), &ProductInboundPayload::NoOp);
    }

    #[test]
    fn trusted_context_can_stamp_explicit_source_channel() {
        let evidence = ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
            },
            "telegram_install_alpha",
        );
        let context = TrustedInboundContext::from_verified_evidence_with_source_channel(
            ProductAdapterId::new("extension_gateway").expect("valid"),
            ProductSourceChannel::new("vendor_chat").expect("valid"),
            AdapterInstallationId::new("install_alpha").expect("valid"),
            Utc::now(),
            &evidence,
        )
        .expect("verified");
        let envelope = ProductInboundEnvelope::from_trusted_parse(
            context,
            sample_parsed(ProductInboundPayload::NoOp),
        )
        .expect("envelope");
        assert_eq!(envelope.adapter_id().as_str(), "extension_gateway");
        assert_eq!(envelope.source_channel().as_str(), "vendor_chat");
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

    #[test]
    fn rejection_kind_user_facing_hint_is_exhaustive_and_sanitized() {
        // Every variant must return a non-empty, static hint with no internal state.
        let cases = [
            (ProductRejectionKind::BindingRequired, "approve gate:"),
            (ProductRejectionKind::AccessDenied, "access"),
            (ProductRejectionKind::UnknownInstallation, "workspace"),
            (ProductRejectionKind::InvalidRequest, "approve"),
            (ProductRejectionKind::PolicyDenied, "policy"),
            (ProductRejectionKind::AmbiguousResolution, "approve gate:"),
        ];
        for (kind, expected_substr) in &cases {
            let hint = kind.user_facing_hint();
            assert!(!hint.is_empty(), "{kind:?} hint must not be empty");
            assert!(
                hint.contains(expected_substr),
                "{kind:?} hint '{hint}' must contain '{expected_substr}'"
            );
        }

        // Hints must be pairwise distinct — two kinds sharing a hint would
        // make the user-facing feedback ambiguous about what went wrong.
        let mut hints: Vec<&str> = cases
            .iter()
            .map(|(kind, _)| kind.user_facing_hint())
            .collect();
        hints.sort_unstable();
        hints.dedup();
        assert_eq!(
            hints.len(),
            cases.len(),
            "every ProductRejectionKind must have a distinct user-facing hint"
        );
    }

    #[test]
    fn rejection_kind_user_facing_auth_hint_overrides_approval_kinds_and_falls_through() {
        // BindingRequired and InvalidRequest must return auth-specific guidance,
        // not the approval-command text from user_facing_hint().
        let binding_hint = ProductRejectionKind::BindingRequired.user_facing_auth_hint();
        assert!(
            binding_hint.contains("auth deny"),
            "BindingRequired auth hint must reference 'auth deny', got: {binding_hint}"
        );
        assert!(
            !binding_hint.contains("approve gate:"),
            "BindingRequired auth hint must not contain approval command, got: {binding_hint}"
        );

        let invalid_hint = ProductRejectionKind::InvalidRequest.user_facing_auth_hint();
        assert!(
            invalid_hint.contains("auth deny"),
            "InvalidRequest auth hint must reference 'auth deny', got: {invalid_hint}"
        );
        assert!(
            !invalid_hint.contains("approve"),
            "InvalidRequest auth hint must not contain approval command, got: {invalid_hint}"
        );

        // AmbiguousResolution must also return auth-specific guidance, not approval text.
        let ambiguous_hint = ProductRejectionKind::AmbiguousResolution.user_facing_auth_hint();
        assert!(
            ambiguous_hint.contains("auth deny"),
            "AmbiguousResolution auth hint must reference 'auth deny', got: {ambiguous_hint}"
        );

        // All other kinds fall through to user_facing_hint().
        for kind in [
            ProductRejectionKind::AccessDenied,
            ProductRejectionKind::UnknownInstallation,
            ProductRejectionKind::PolicyDenied,
        ] {
            assert_eq!(
                kind.user_facing_auth_hint(),
                kind.user_facing_hint(),
                "{kind:?} auth hint must fall through to user_facing_hint()"
            );
        }
    }

    // BUG 3 regression: StaleGate must have a distinct hint that does NOT say
    // "declined by policy" — it means the gate was already resolved.
    #[test]
    fn stale_gate_hint_is_distinct_from_policy_denied() {
        let stale_hint = ProductRejectionKind::StaleGate.user_facing_hint();
        let policy_hint = ProductRejectionKind::PolicyDenied.user_facing_hint();
        assert_ne!(
            stale_hint, policy_hint,
            "StaleGate hint must differ from PolicyDenied hint"
        );
        assert!(
            !stale_hint.contains("declined by policy"),
            "StaleGate hint must not say 'declined by policy', got: {stale_hint}"
        );
        assert!(
            stale_hint.contains("already approved or denied"),
            "StaleGate hint must mention 'already approved or denied', got: {stale_hint}"
        );
    }

    #[test]
    fn policy_denied_hint_unchanged() {
        // Regression: PolicyDenied string must remain stable — existing usages
        // in other approval flows depend on it.
        assert_eq!(
            ProductRejectionKind::PolicyDenied.user_facing_hint(),
            "That request was declined by policy."
        );
    }
}

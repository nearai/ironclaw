//! Inbound envelope, payload, and acknowledgement types.

use std::fmt;

use chrono::{DateTime, Utc};
use ironclaw_host_api::turn::{AcceptedMessageRef, TurnRunId};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::outbound::ProjectionCursor;
use ironclaw_extension_contracts::channel_adapter::ChannelAttachmentRef;
use ironclaw_extension_contracts::channel_adapter::ProductTriggerReason;
use ironclaw_extension_contracts::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentDescriptor,
};
use ironclaw_host_api::product_adapter::auth::{ProtocolAuthEvidence, VerifiedAuthClaim};
use ironclaw_host_api::product_adapter::identity::{AdapterInstallationId, ProductAdapterId};
use ironclaw_host_api::product_adapter_error::ProductAdapterError;
use ironclaw_host_api::product_adapter_error::RedactedString;

const USER_MESSAGE_TEXT_MAX_BYTES: usize = 64 * 1024;
/// Matches `ironclaw_extension_contracts::channel_adapter::MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES`
/// (the adapter-side bound the ingress host enforces at fetch time).
const CHANNEL_CONTEXT_MAX_BYTES: usize = 32 * 1024;
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

/// Optional host-side reclassification for a normalized channel message before
/// it enters the product surface.
///
/// `None` at the call site means the normalized message is an ordinary user
/// message. These variants cover channel-neutral interaction replies and
/// slash commands that should not become user turns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelInboundClassification {
    Command(InboundCommandPayload),
    ApprovalResolution(ApprovalResolutionPayload),
    ScopedApprovalResolution(ScopedApprovalResolutionPayload),
    AuthResolution(AuthResolutionPayload),
    NoOp,
}

impl From<ChannelInboundClassification> for ProductInboundPayload {
    fn from(classification: ChannelInboundClassification) -> Self {
        match classification {
            ChannelInboundClassification::Command(payload) => Self::Command(payload),
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
    /// Recent vendor-side conversation history fetched host-side at channel
    /// ingress for shared-channel triggers. UNTRUSTED third-party text quoted
    /// for context; advisory only and absent everywhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_context: Option<String>,
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
            channel_context: None,
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

    /// Attach host-fetched channel conversation context to this payload. See
    /// [`UserMessagePayload::channel_context`].
    pub fn with_channel_context(mut self, channel_context: Option<String>) -> Self {
        self.channel_context = channel_context.filter(|context| !context.is_empty());
        self
    }

    pub fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_payload_string("user message text", &self.text, USER_MESSAGE_TEXT_MAX_BYTES)?;
        if let Some(model) = &self.requested_model {
            validate_payload_string("requested model", model, REQUESTED_MODEL_MAX_BYTES)?;
        }
        if let Some(context) = &self.channel_context {
            validate_payload_string("channel context", context, CHANNEL_CONTEXT_MAX_BYTES)?;
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
    #[serde(default)]
    channel_context: Option<String>,
}

impl<'de> Deserialize<'de> for UserMessagePayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UserMessagePayloadWire::deserialize(deserializer)?;
        let payload = Self::new(wire.text, wire.attachments, wire.trigger)
            .map(|payload| {
                payload
                    .with_requested_model(wire.requested_model)
                    .with_channel_context(wire.channel_context)
            })
            .map_err(serde::de::Error::custom)?;
        // `new` validated the payload while `requested_model` and
        // `channel_context` were still `None`; re-validate the assembled value
        // so the wire-supplied fields are bounded like every other ingress
        // field (bypass flagged in PR review).
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
    // Vendor adapters remove an address only when it names the verified
    // current bot. A surviving `@target` therefore belongs to another bot and
    // must remain ordinary conversation text, not become an Ironclaw command.
    if command_slice.contains('@') {
        return Ok(None);
    }
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

/// Classify channel text reserved for product interactions or commands.
///
/// Returns `None` when text remains an ordinary user message. Confident
/// reserved syntax that cannot be parsed is classified as `NoOp` so the
/// channel ingress fails closed rather than forwarding malformed control text.
pub fn classify_channel_inbound_text(
    text: &str,
    trigger: ProductTriggerReason,
) -> Option<ChannelInboundClassification> {
    match crate::interaction_commands::parse_interaction_resolution_text(
        crate::interaction_commands::strip_wrapping_inline_code(text),
        trigger,
    ) {
        Ok(Some(ProductInboundPayload::ApprovalResolution(payload))) => {
            return Some(ChannelInboundClassification::ApprovalResolution(payload));
        }
        Ok(Some(ProductInboundPayload::ScopedApprovalResolution(payload))) => {
            return Some(ChannelInboundClassification::ScopedApprovalResolution(
                payload,
            ));
        }
        Ok(Some(ProductInboundPayload::AuthResolution(payload))) => {
            return Some(ChannelInboundClassification::AuthResolution(payload));
        }
        Ok(Some(ProductInboundPayload::NoOp)) | Err(_) => {
            return Some(ChannelInboundClassification::NoOp);
        }
        Ok(Some(_)) | Ok(None) => {}
    }

    match parse_product_slash_command(text, trigger) {
        Ok(Some(command)) => Some(ChannelInboundClassification::Command(command)),
        Ok(None) => None,
        Err(_) => Some(ChannelInboundClassification::NoOp),
    }
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
                    ironclaw_host_api::product_adapter_error::ProtocolAuthFailure::Missing,
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
                    ironclaw_host_api::product_adapter_error::ProtocolAuthFailure::Missing,
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

/// Trusted inbound envelope handed to the workflow service.
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
    /// Provider transfer references for channel attachments. Host-only and
    /// transient: the serializable payload carries descriptors, while these
    /// refs survive only long enough for accepted workflow intake to fetch
    /// bytes through restricted egress.
    #[serde(skip)]
    channel_attachment_refs: Vec<ChannelAttachmentRef>,
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
            channel_attachment_refs: Vec::new(),
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

    /// Attach the transient channel transfer references corresponding exactly
    /// to the user-message descriptors already present in this envelope.
    pub fn with_channel_attachment_refs(
        mut self,
        channel_attachment_refs: Vec<ChannelAttachmentRef>,
    ) -> Result<Self, ProductAdapterError> {
        let ProductInboundPayload::UserMessage(payload) = &self.payload else {
            if channel_attachment_refs.is_empty() {
                return Ok(self);
            }
            return Err(malformed(
                "channel attachment references require a user-message payload",
            ));
        };
        let descriptors = channel_attachment_refs
            .iter()
            .map(|attachment| &attachment.descriptor)
            .collect::<Vec<_>>();
        if descriptors
            != payload
                .attachments
                .iter()
                .collect::<Vec<&ProductAttachmentDescriptor>>()
        {
            return Err(malformed(
                "channel attachment references do not match payload descriptors",
            ));
        }
        self.channel_attachment_refs = channel_attachment_refs;
        Ok(self)
    }

    /// Host-only transient provider references for deferred attachment fetch.
    pub fn channel_attachment_refs(&self) -> &[ChannelAttachmentRef] {
        &self.channel_attachment_refs
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
        let channel_attachment_refs = if self.channel_attachment_refs.is_empty() {
            Vec::new()
        } else {
            let mut used = vec![false; self.channel_attachment_refs.len()];
            let mut reconciled = Vec::with_capacity(payload.attachments.len());
            for descriptor in &payload.attachments {
                let matching = self
                    .channel_attachment_refs
                    .iter()
                    .enumerate()
                    .filter(|(_, source)| source.descriptor == *descriptor)
                    .collect::<Vec<_>>();
                let [(index, source)] = matching.as_slice() else {
                    return Err(malformed(
                        "policy-rewritten attachment has no unique original channel source",
                    ));
                };
                if used[*index] {
                    return Err(malformed(
                        "policy-rewritten attachment reuses an original channel source",
                    ));
                }
                used[*index] = true;
                reconciled.push((*source).clone());
            }
            reconciled
        };
        let mut envelope = self.clone();
        envelope.payload = ProductInboundPayload::UserMessage(payload);
        envelope.channel_attachment_refs = channel_attachment_refs;
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
mod tests;

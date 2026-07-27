//! Provider tool-call and capability-invocation DTOs, capability failure/denied
//! reason kinds, and the [`LoopCapabilityPort`] host boundary.

use async_trait::async_trait;
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityId, CorrelationId, ExtensionId, HostApiError, ProviderToolName,
    Resolution, ResolutionBatch, RuntimeKind,
};
use serde::{Deserialize, Deserializer, Serialize};

use crate::run_profile::content_digest::ContentDigest;
use crate::run_profile::model_observation::{CapabilityFailureDetail, ModelVisibleToolObservation};
use crate::{CapabilityActivityId, LoopResultRef};

use super::error::{AgentLoopHostError, AgentLoopHostErrorKind, unsupported_host_method};
use super::model::CapabilityCallCandidate;
use super::refs::{CapabilityInputRef, CapabilityResumeToken, CapabilitySurfaceVersion};
use super::validate::validate_loop_safe_identifier;

/// Capability ids a provider tool call may touch before it is staged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderToolCallCapabilityIds {
    /// Canonical capability id backing the provider-facing tool name.
    pub provider_capability_id: CapabilityId,
    /// Capabilities whose policy surface is used by this call.
    pub effective_capability_ids: Vec<CapabilityId>,
}

impl ProviderToolCallCapabilityIds {
    pub fn single(capability_id: CapabilityId) -> Self {
        Self {
            provider_capability_id: capability_id.clone(),
            effective_capability_ids: vec![capability_id],
        }
    }
}

/// Provider-originated tool-call metadata needed to replay tool results back to the same provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolCallReplay {
    /// Provider identity selected by the host route.
    pub provider_id: String,
    /// Concrete provider model selected by the host route.
    pub provider_model_id: String,
    /// Provider turn grouping token for reconstructing assistant tool calls.
    pub provider_turn_id: String,
    /// Provider call id referenced by the matching tool result.
    pub provider_call_id: String,
    /// Provider-facing tool name advertised to the model.
    pub provider_tool_name: ProviderToolName,
    /// Provider-facing tool arguments captured from the model tool call.
    pub arguments: serde_json::Value,
    /// Provider response-level reasoning attached to the tool-call batch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_reasoning: Option<String>,
    /// Provider call-level reasoning attached to this tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Opaque provider thought-signature metadata, not an IronClaw auth signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl ProviderToolCallReplay {
    pub fn from_tool_call(tool_call: ProviderToolCall, provider_turn_id: String) -> Self {
        Self {
            provider_id: tool_call.provider_id,
            provider_model_id: tool_call.provider_model_id,
            provider_turn_id,
            provider_call_id: tool_call.id,
            provider_tool_name: tool_call.name,
            arguments: tool_call.arguments,
            response_reasoning: tool_call.response_reasoning,
            reasoning: tool_call.reasoning,
            signature: tool_call.signature,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VisibleCapabilityRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleCapabilitySurface {
    pub version: CapabilitySurfaceVersion,
    pub descriptors: Vec<CapabilityDescriptorView>,
    /// Capability IDs the model may *invoke* this turn — the reachable authorized
    /// catalog — as distinct from `descriptors`, which is the *advertised* subset.
    ///
    /// Under progressive tool disclosure the advertised set (token economy) is a
    /// narrowed view, but the model can still reach catalog tools beyond it via
    /// the tool-call bridge / forgiving-direct path. Call-time authorization (the
    /// model-visible capability filter) must therefore validate against this
    /// wider "callable" set, while advertising and prompt rendering stay narrow.
    ///
    /// `None` means "same as `descriptors`" — no disclosure narrowing is in
    /// effect, so callable == advertised. `Some(_)` is an explicit callable set
    /// that may legitimately be empty (no callable capabilities this turn),
    /// which the sentinel-free encoding keeps distinct from the un-narrowed case.
    /// Producers that don't narrow leave this `None`; consumers fall back to
    /// `descriptors`.
    #[serde(default)]
    pub callable_capability_ids: Option<Vec<CapabilityId>>,
}

/// Concurrency hint for a capability surfaced to an agent loop driver.
///
/// Derived at the adapter boundary from the underlying
/// `CapabilityDescriptor.effects` Vec. The lower-layer `CapabilityDescriptor`
/// is NOT modified; `effects` remains the source of truth and the hint is a
/// computed projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConcurrencyHint {
    /// Capability has no exclusive side effects; multiple invocations may run
    /// in parallel without ordering hazards.
    SafeForParallel,
    /// Capability must be invoked serially within a loop run — parallel
    /// invocation would violate ordering or isolation constraints.
    Exclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDescriptorView {
    pub capability_id: CapabilityId,
    pub provider: Option<ExtensionId>,
    pub runtime: RuntimeKind,
    pub safe_name: String,
    pub safe_description: String,
    pub concurrency_hint: ConcurrencyHint,
    #[serde(default)]
    pub parameters_schema: serde_json::Value,
}

/// Provider-facing tool definition derived from a visible IronClaw capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolDefinition {
    /// Canonical IronClaw capability id backing this provider tool.
    pub capability_id: CapabilityId,
    /// Provider-safe tool name sent to the model.
    pub name: ProviderToolName,
    /// Provider-safe tool description sent to the model.
    pub description: String,
    /// JSON object schema for provider tool arguments.
    pub parameters: serde_json::Value,
}

impl ProviderToolDefinition {
    pub fn from_parts(
        capability_id: CapabilityId,
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Result<Self, AgentLoopHostError> {
        let name = ProviderToolName::new(name.into()).map_err(provider_tool_name_error)?;
        Ok(Self::from_typed_parts(
            capability_id,
            name,
            description,
            parameters,
        ))
    }

    /// Builds a definition from a provider-safe name that has already passed
    /// [`ProviderToolName`] validation. Use [`Self::from_parts`] for raw
    /// provider names that still need validation.
    pub fn from_typed_parts(
        capability_id: CapabilityId,
        name: ProviderToolName,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            capability_id,
            name,
            description: description.into(),
            parameters,
        }
    }

    pub fn validate_name(name: &str) -> Result<ProviderToolName, AgentLoopHostError> {
        ProviderToolName::new(name).map_err(provider_tool_name_error)
    }
}

/// Tool call emitted by a provider-backed model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolCall {
    /// Provider identity selected by the host route.
    pub provider_id: String,
    /// Concrete provider model selected by the host route.
    pub provider_model_id: String,
    /// Provider turn grouping token for reconstructing assistant tool calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Provider call id referenced by the matching tool result.
    pub id: String,
    /// Provider-facing tool name returned by the model.
    pub name: ProviderToolName,
    /// Provider-facing tool arguments returned by the model.
    pub arguments: serde_json::Value,
    /// Provider response-level reasoning attached to the tool-call batch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_reasoning: Option<String>,
    /// Provider call-level reasoning attached to this tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Opaque provider thought-signature metadata, not an IronClaw auth signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl ProviderToolCall {
    pub fn from_parts(
        provider_id: impl Into<String>,
        provider_model_id: impl Into<String>,
        turn_id: Option<String>,
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Result<Self, AgentLoopHostError> {
        Ok(Self {
            provider_id: provider_id.into(),
            provider_model_id: provider_model_id.into(),
            turn_id,
            id: id.into(),
            name: ProviderToolName::new(name.into()).map_err(provider_tool_name_error)?,
            arguments,
            response_reasoning: None,
            reasoning: None,
            signature: None,
        })
    }
}

fn provider_tool_name_error(error: HostApiError) -> AgentLoopHostError {
    let detail = match error {
        HostApiError::InvalidId { reason, .. } => reason,
        other => other.to_string(),
    };
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        format!("tool name cannot be represented as a provider tool name: {detail}"),
    )
}

/// Durable reference to provider tool-call metadata for tool-result replay.
///
/// This is [`ProviderToolCallReplay`] plus the canonical IronClaw
/// `capability_id`. The replay fields are `#[serde(flatten)]`ed so the
/// serialized shape stays byte-identical to the historical field-per-field
/// layout while the nine shared fields live in exactly one place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolCallReference {
    /// Provider-originated tool-call metadata needed to replay tool results
    /// back to the same provider.
    #[serde(flatten)]
    pub replay: ProviderToolCallReplay,
    /// Canonical IronClaw capability id backing this provider tool.
    pub capability_id: CapabilityId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterProviderToolCallRequest {
    pub tool_call: ProviderToolCall,
    /// Activity identity to bind to this provider call. When set, the host
    /// must register the call with this id, rejecting if the same input_ref was
    /// already registered with another id. When absent, the host creates an id
    /// for the first registration and returns that same id for duplicate
    /// registrations of the same input_ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_id: Option<CapabilityActivityId>,
}

impl RegisterProviderToolCallRequest {
    pub fn new(tool_call: ProviderToolCall) -> Self {
        Self {
            tool_call,
            activity_id: None,
        }
    }

    pub fn for_activity(tool_call: ProviderToolCall, activity_id: CapabilityActivityId) -> Self {
        Self {
            tool_call,
            activity_id: Some(activity_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopRequest {
    /// Stable activity identity for this invocation. Runtime hosts derive
    /// their execution identity from it rather than minting a second id.
    pub activity_id: CapabilityActivityId,
    pub surface_version: CapabilitySurfaceVersion,
    pub capability_id: CapabilityId,
    pub input_ref: CapabilityInputRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_resume: Option<CapabilityApprovalResume>,
    /// Set when the invocation was previously auth-blocked and the auth
    /// gate has now been resolved. Carries the original activity token so
    /// re-dispatch reuses it rather than minting a new one, preserving any
    /// prior approval lease whose scope embeds that id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_resume: Option<CapabilityAuthResume>,
}

/// Approval-gate resume identity carried by the loop.
///
/// Raw runtime `input`/`estimate` no longer ride here (arch-simplification §5.3
/// Stage 2a-i): the host persists the host-private `ReplayPayload`
/// (`ironclaw_capabilities`) at the gate raise, keyed by the invocation id
/// encoded in `resume_token`, and reconstitutes it host-side on resume. Keeping
/// the raw tool args out of the loop's serialized checkpoint retires the
/// charter-violating exposure flagged in `ironclaw_agent_loop`'s `CLAUDE.md`
/// ("Do not store raw prompts, raw model output, tool args ... in state").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityApprovalResume {
    pub approval_request_id: ApprovalRequestId,
    pub resume_token: CapabilityResumeToken,
    #[serde(default = "CorrelationId::new")]
    pub correlation_id: CorrelationId,
    pub input_ref: CapabilityInputRef,
}

/// Prior-approval identity carried through an auth-gate resume.
///
/// Both fields are semantically all-or-none: the pair is present only when
/// the invocation previously passed a one-shot approval gate.  Modelling
/// them as a single optional struct makes the compile-time invariant explicit —
/// `approval_request_id` and `correlation_id` cannot be independently absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResumeApprovalIdentity {
    /// Identifies the prior approval request so the host can locate and
    /// claim the matching fingerprinted lease without requiring a second
    /// human approval for the same action.
    pub approval_request_id: ApprovalRequestId,
    /// Original correlation identifier from the prior approval gate.
    /// Restored onto the invocation context so the same trace-correlation
    /// identifier flows through the full capability lifecycle.
    pub correlation_id: CorrelationId,
}

/// Auth-gate resume identity.
///
/// Carries the original activity identity (encoded as a resume token) so
/// that re-dispatch after credential completion reuses the same activity
/// rather than minting a fresh one.  When the prior invocation also passed
/// an approval gate, `prior_approval` carries the approval identity so the
/// host can claim the matching fingerprinted lease without requiring a second
/// human approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityAuthResume {
    /// Encodes the original activity identity so the host can reuse the
    /// matching execution context after auth completes. A denied gate does not
    /// re-dispatch, so its activity identity is already carried by
    /// [`LoopRequest::activity_id`] and no resume token is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<CapabilityResumeToken>,
    /// A terminal user denial is carried through the same capability lifecycle
    /// seam as a successful auth resume. The host terminalizes the blocked
    /// invocation without dispatching the capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<crate::GateResumeDisposition>,
    /// Present when the invocation previously passed a one-shot approval gate.
    /// The two sub-fields are always set together; see [`AuthResumeApprovalIdentity`].
    //
    // Design note (not part of this field's contract): the raw runtime
    // `input`/`estimate` no longer ride this struct (arch-simplification §5.3
    // Stage 2a-i) — capability input refs are scoped to a loop run and may be
    // consumed by the first dispatch, so the host persists the host-private
    // `ReplayPayload` (`ironclaw_capabilities`) at the gate raise — keyed by
    // the invocation id encoded in `resume_token` — and reconstitutes it
    // host-side on resume, rather than round-tripping raw tool args through
    // the loop checkpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_approval: Option<AuthResumeApprovalIdentity>,
}

impl CapabilityAuthResume {
    pub fn resolved(
        resume_token: CapabilityResumeToken,
        prior_approval: Option<AuthResumeApprovalIdentity>,
    ) -> Self {
        Self {
            resume_token: Some(resume_token),
            disposition: None,
            prior_approval,
        }
    }

    pub fn denied() -> Self {
        Self {
            resume_token: None,
            disposition: Some(crate::GateResumeDisposition::Denied),
            prior_approval: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopRequestBatch {
    pub invocations: Vec<LoopRequest>,
    pub stop_on_first_suspension: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityProgress {
    /// Older hosts, or hosts that cannot classify progress yet.
    #[default]
    Unknown,
    /// The capability produced new evidence or changed host/runtime state.
    #[serde(alias = "complete")]
    MadeProgress,
    /// The capability ran successfully but observed the same state/evidence as
    /// before.
    NoChange,
    /// The capability reached a deterministic non-suspending blocker.
    Blocked,
}

/// The agent-loop executor's reconstructed view of a completed capability result.
///
/// Producers no longer emit this (they emit [`Resolution`](ironclaw_host_api::Resolution)
/// directly, §5.3 Stage 2b); the executor rebuilds it from the host_api
/// [`Outcome`](ironclaw_host_api::Outcome) channel (`capability_result_from_outcome`)
/// to feed its result-admission/strategy pipeline. It is loop-internal working
/// vocabulary, no longer a wire/producer DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityResultMessage {
    pub result_ref: LoopResultRef,
    pub safe_summary: String,
    #[serde(default)]
    pub progress: CapabilityProgress,
    #[serde(default)]
    pub terminate_hint: bool,
    #[serde(default)]
    pub byte_len: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_digest: Option<ContentDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_observation: Option<ModelVisibleToolObservation>,
}

/// The agent-loop executor's reconstructed view of a recoverable capability
/// failure, rebuilt from the host_api `RecoverableFailure` verdict
/// (`capability_failure_from_recoverable`) to drive retry/explain recovery.
/// Loop-internal working vocabulary, no longer a wire/producer DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityFailure {
    pub error_kind: CapabilityFailureKind,
    pub safe_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<CapabilityFailureDetail>,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CapabilityDeniedReasonKind {
    EmptySurface,
    Unknown(CapabilityDeniedReasonKindValue),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityDeniedReasonKindValue(String);

impl CapabilityDeniedReasonKindValue {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_loop_safe_identifier(value.into(), "capability denied reason kind", 128).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl CapabilityDeniedReasonKind {
    pub fn unknown(value: impl Into<String>) -> Result<Self, String> {
        CapabilityDeniedReasonKindValue::new(value).map(Self::Unknown)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::EmptySurface => "empty_surface",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

impl std::fmt::Display for CapabilityDeniedReasonKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for CapabilityDeniedReasonKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CapabilityDeniedReasonKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "empty_surface" => Ok(Self::EmptySurface),
            _ => Self::unknown(value).map_err(serde::de::Error::custom),
        }
    }
}

// Deliberately NOT `#[non_exhaustive]`: the `Unknown(CapabilityFailureKindValue)`
// variant is the forward-compat / open-set escape hatch (a newer producer's
// unrecognized wire string deserializes into `Unknown`), and the manual
// `Serialize`/`Deserialize` impls below route every value through `as_str()` /
// that variant. Leaving the attribute on would force callers — notably the
// recovery classifier `capability_error_class` — to keep a wildcard `_ =>` arm,
// which silently buckets any newly-added *named* variant (e.g. a future
// `QuotaExceeded`) into a run-aborting class. Without the attribute, those
// classifiers match exhaustively, so a new named variant fails to compile until
// it is deliberately classified. See
// `docs/plans/2026-06-28-reborn-error-recoverability-audit.md` §6.1.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CapabilityFailureKind {
    Authorization,
    Backend,
    Cancelled,
    Dispatcher,
    GateDeclined,
    InvalidInput,
    InvalidOutput,
    MissingRuntime,
    Network,
    OperationFailed,
    OutputTooLarge,
    PolicyDenied,
    Process,
    Resource,
    Transient,
    Unavailable,
    Internal,
    Permanent,
    Unknown(CapabilityFailureKindValue),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityFailureKindValue(String);

impl CapabilityFailureKindValue {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_loop_safe_identifier(value.into(), "capability failure kind", 128).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl CapabilityFailureKind {
    pub fn unknown(value: impl Into<String>) -> Result<Self, String> {
        CapabilityFailureKindValue::new(value).map(Self::Unknown)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Authorization => "authorization",
            Self::Backend => "backend",
            Self::Cancelled => "cancelled",
            Self::Dispatcher => "dispatcher",
            Self::GateDeclined => "gate_declined",
            Self::InvalidInput => "invalid_input",
            Self::InvalidOutput => "invalid_output",
            Self::MissingRuntime => "missing_runtime",
            Self::Network => "network",
            Self::OperationFailed => "operation_failed",
            Self::OutputTooLarge => "output_too_large",
            Self::PolicyDenied => "policy_denied",
            Self::Process => "process",
            Self::Resource => "resource",
            Self::Transient => "transient",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
            Self::Permanent => "permanent",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

impl std::fmt::Display for CapabilityFailureKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for CapabilityFailureKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CapabilityFailureKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "authorization" => Ok(Self::Authorization),
            "backend" => Ok(Self::Backend),
            "cancelled" => Ok(Self::Cancelled),
            "dispatcher" => Ok(Self::Dispatcher),
            "gate_declined" => Ok(Self::GateDeclined),
            "invalid_input" => Ok(Self::InvalidInput),
            "invalid_output" => Ok(Self::InvalidOutput),
            "missing_runtime" => Ok(Self::MissingRuntime),
            "network" => Ok(Self::Network),
            "operation_failed" => Ok(Self::OperationFailed),
            "output_too_large" => Ok(Self::OutputTooLarge),
            "policy_denied" => Ok(Self::PolicyDenied),
            "process" => Ok(Self::Process),
            "resource" => Ok(Self::Resource),
            "transient" => Ok(Self::Transient),
            "unavailable" => Ok(Self::Unavailable),
            "internal" => Ok(Self::Internal),
            "permanent" => Ok(Self::Permanent),
            _ => Self::unknown(value).map_err(serde::de::Error::custom),
        }
    }
}

#[async_trait]
pub trait LoopCapabilityPort: Send + Sync {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        Ok(Vec::new())
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        let Some(definition) = self
            .tool_definitions()?
            .into_iter()
            .find(|definition| definition.name == tool_call.name)
        else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool call is outside the visible capability surface",
            ));
        };
        Ok(ProviderToolCallCapabilityIds::single(
            definition.capability_id,
        ))
    }

    fn validate_provider_tool_call(
        &self,
        _tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        Ok(())
    }

    async fn register_provider_tool_call(
        &self,
        _request: RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        Err(unsupported_host_method("register_provider_tool_call"))
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError>;

    fn current_visible_capabilities(
        &self,
    ) -> Result<Option<VisibleCapabilitySurface>, AgentLoopHostError> {
        Ok(None)
    }

    async fn invoke_capability(
        &self,
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError>;

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DefinitionPort {
        definitions: Vec<ProviderToolDefinition>,
    }

    #[async_trait]
    impl LoopCapabilityPort for DefinitionPort {
        fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
            Ok(self.definitions.clone())
        }

        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            unreachable!("not used by this test")
        }

        async fn invoke_capability(
            &self,
            _request: LoopRequest,
        ) -> Result<Resolution, AgentLoopHostError> {
            unreachable!("not used by this test")
        }

        async fn invoke_capability_batch(
            &self,
            _request: LoopRequestBatch,
        ) -> Result<ResolutionBatch, AgentLoopHostError> {
            unreachable!("not used by this test")
        }
    }

    fn provider_tool_call(name: &str) -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            turn_id: Some("turn".to_string()),
            id: "call".to_string(),
            name: ProviderToolName::new(name).expect("provider tool name"),
            arguments: serde_json::json!({}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    #[test]
    fn provider_tool_call_capability_ids_rejects_unknown_tool_name() {
        let port = DefinitionPort {
            definitions: vec![ProviderToolDefinition {
                capability_id: CapabilityId::new("demo.allowed").expect("valid capability id"),
                name: ProviderToolName::new("demo__allowed").expect("provider tool name"),
                description: "allowed".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }],
        };

        let error = port
            .provider_tool_call_capability_ids(&provider_tool_call("demo__missing"))
            .expect_err("unknown provider tool must fail closed");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }
}

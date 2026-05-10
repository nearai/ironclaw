use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, ExtensionId, RuntimeKind, ThreadId};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::{
    LoopDiagnosticRef, LoopGateRef, LoopMessageRef, LoopResultRef, RunProfileVersion,
    TurnCheckpointId, TurnId, TurnRunId, TurnScope,
};

use super::{
    refs::{CheckpointSchemaId, LoopDriverId, ModelProfileId},
    snapshot::ResolvedRunProfile,
};

fn validate_bounded_loop_string(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, String> {
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(format!("{label} must be at most {max_bytes} bytes"));
    }
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(format!("{label} must not contain NUL/control characters"));
    }
    Ok(value)
}

fn validate_prefixed_loop_ref(
    label: &'static str,
    prefix: &'static str,
    max_bytes: usize,
    value: String,
) -> Result<String, String> {
    let value = validate_bounded_loop_string(value, label, max_bytes)?;
    if !value.starts_with(prefix) {
        return Err(format!("{label} must start with `{prefix}`"));
    }
    Ok(value)
}

fn validate_loop_opaque_token(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, String> {
    let value = validate_bounded_loop_string(value, label, max_bytes)?;
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        return Err(format!(
            "{label} must contain only ASCII letters, digits, _, -, or ."
        ));
    }
    Ok(value)
}

fn validate_loop_safe_identifier(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, String> {
    let value = validate_bounded_loop_string(value, label, max_bytes)?;
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
    }) {
        return Err(format!(
            "{label} must contain only ASCII letters, digits, _, -, ., or :"
        ));
    }

    let lower = value.to_ascii_lowercase();
    for forbidden in [
        "access_token",
        "access-token",
        "api_key",
        "apikey",
        "authorization",
        "bearer",
        "password",
        "passwd",
        "secret",
    ] {
        if lower.contains(forbidden) {
            return Err(format!(
                "{label} must not contain sensitive marker `{forbidden}`"
            ));
        }
    }
    if lower
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .any(|token| token.starts_with("sk-"))
    {
        return Err(format!("{label} must not contain API-key-like tokens"));
    }
    Ok(value)
}

fn validate_loop_safe_summary(value: String) -> Result<String, String> {
    let value = validate_bounded_loop_string(value, "loop safe summary", 512)?;
    if value.chars().any(|character| {
        matches!(
            character,
            '{' | '}' | '[' | ']' | '`' | '<' | '>' | '/' | '\\'
        )
    }) {
        return Err(
            "loop safe summary must not contain raw payload or path delimiters".to_string(),
        );
    }

    let lower = value.to_ascii_lowercase();
    for forbidden in [
        "access token",
        "api key",
        "api_key",
        "apikey",
        "authorization:",
        "bearer ",
        "host path",
        "invalid api key",
        "invalid_api_key",
        "password",
        "passwd",
        "provider error",
        "raw runtime",
        "secret",
        "stack trace",
        "tool input",
        "tool_input",
        "traceback",
    ] {
        if lower.contains(forbidden) {
            return Err(format!(
                "loop safe summary must not contain sensitive marker `{forbidden}`"
            ));
        }
    }
    if lower
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .any(|token| token.starts_with("sk-"))
    {
        return Err("loop safe summary must not contain API-key-like tokens".to_string());
    }
    Ok(value)
}

macro_rules! bounded_loop_ref {
    ($name:ident, $label:literal, $prefix:literal, $max:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                validate_prefixed_loop_ref($label, $prefix, $max, value.into()).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

bounded_loop_ref!(CapabilityInputRef, "capability input ref", "input:", 256);
bounded_loop_ref!(
    LoopCheckpointStateRef,
    "loop checkpoint state ref",
    "checkpoint:",
    256
);
bounded_loop_ref!(
    LoopInputCursorToken,
    "loop input cursor token",
    "input-cursor:",
    256
);
bounded_loop_ref!(LoopProcessRef, "loop process ref", "process:", 256);

impl LoopCheckpointStateRef {
    pub fn for_run(context: &LoopRunContext, token: impl Into<String>) -> Result<Self, String> {
        let token = validate_loop_opaque_token(token.into(), "loop checkpoint state token", 96)?;
        Self::new(format!("checkpoint:{}:{token}", context.run_id))
    }

    pub fn is_for_run(&self, context: &LoopRunContext) -> bool {
        let Some(token) = self
            .0
            .strip_prefix(&format!("checkpoint:{}:", context.run_id))
        else {
            return false;
        };
        validate_loop_opaque_token(token.to_string(), "loop checkpoint state token", 96).is_ok()
    }
}

/// Opaque reference to a host-built prompt bundle for one loop run.
///
/// Serialized refs use `prompt:{run_id}:{opaque_token}`. Consumers must treat
/// the token as opaque metadata and must not infer or persist raw prompt text
/// from this value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct LoopPromptBundleRef(String);

impl LoopPromptBundleRef {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value =
            validate_prefixed_loop_ref("loop prompt bundle ref", "prompt:", 256, value.into())?;
        let suffix = value
            .strip_prefix("prompt:")
            .ok_or_else(|| "loop prompt bundle ref must start with `prompt:`".to_string())?;
        let (run_id, token) = suffix.split_once(':').ok_or_else(|| {
            "loop prompt bundle ref must include scoped run id and opaque token".to_string()
        })?;
        uuid::Uuid::parse_str(run_id)
            .map_err(|_| "loop prompt bundle ref run id must be a UUID".to_string())?;
        validate_loop_opaque_token(token.to_string(), "loop prompt bundle token", 96)?;
        Ok(Self(value))
    }

    pub fn for_run(context: &LoopRunContext, token: impl Into<String>) -> Result<Self, String> {
        let token = validate_loop_opaque_token(token.into(), "loop prompt bundle token", 96)?;
        Self::new(format!("prompt:{}:{token}", context.run_id))
    }

    pub(crate) fn fresh_for_run(context: &LoopRunContext) -> Self {
        Self(format!(
            "prompt:{}:{}",
            context.run_id,
            uuid::Uuid::new_v4()
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_for_run(&self, context: &LoopRunContext) -> bool {
        self.0.starts_with(&format!("prompt:{}:", context.run_id))
    }
}

impl AsRef<str> for LoopPromptBundleRef {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for LoopPromptBundleRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for LoopPromptBundleRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct LoopSafeSummary(String);

impl LoopSafeSummary {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_loop_safe_summary(value.into()).map(Self)
    }

    pub fn model_gateway_failed() -> Self {
        Self("model gateway failed".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for LoopSafeSummary {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for LoopSafeSummary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for LoopSafeSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

fn origin_input_cursor_token() -> LoopInputCursorToken {
    LoopInputCursorToken("input-cursor:origin".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopRunContext {
    pub scope: TurnScope,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub resolved_run_profile: ResolvedRunProfile,
    pub loop_driver_id: LoopDriverId,
    pub loop_driver_version: RunProfileVersion,
    pub checkpoint_schema_id: CheckpointSchemaId,
    pub checkpoint_schema_version: RunProfileVersion,
}

impl LoopRunContext {
    pub fn new(
        scope: TurnScope,
        turn_id: TurnId,
        run_id: TurnRunId,
        resolved_run_profile: ResolvedRunProfile,
    ) -> Self {
        let thread_id = scope.thread_id.clone();
        Self {
            scope,
            thread_id,
            turn_id,
            run_id,
            loop_driver_id: resolved_run_profile.loop_driver.id.clone(),
            loop_driver_version: resolved_run_profile.loop_driver.version,
            checkpoint_schema_id: resolved_run_profile.checkpoint_schema_id.clone(),
            checkpoint_schema_version: resolved_run_profile.checkpoint_schema_version,
            resolved_run_profile,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopHostErrorKind {
    Unauthorized,
    ScopeMismatch,
    StaleSurface,
    InvalidInvocation,
    PolicyDenied,
    BudgetExceeded,
    Unavailable,
    Cancelled,
    CheckpointRejected,
    TranscriptWriteFailed,
    Internal,
}

impl AgentLoopHostErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::ScopeMismatch => "scope_mismatch",
            Self::StaleSurface => "stale_surface",
            Self::InvalidInvocation => "invalid_invocation",
            Self::PolicyDenied => "policy_denied",
            Self::BudgetExceeded => "budget_exceeded",
            Self::Unavailable => "unavailable",
            Self::Cancelled => "cancelled",
            Self::CheckpointRejected => "checkpoint_rejected",
            Self::TranscriptWriteFailed => "transcript_write_failed",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("agent loop host {kind:?}: {safe_summary}")]
pub struct AgentLoopHostError {
    pub kind: AgentLoopHostErrorKind,
    pub safe_summary: String,
    pub diagnostic_ref: Option<LoopDiagnosticRef>,
}

impl AgentLoopHostError {
    pub fn new(kind: AgentLoopHostErrorKind, safe_summary: impl Into<String>) -> Self {
        Self {
            kind,
            safe_summary: safe_summary.into(),
            diagnostic_ref: None,
        }
    }

    pub fn with_diagnostic_ref(mut self, diagnostic_ref: LoopDiagnosticRef) -> Self {
        self.diagnostic_ref = Some(diagnostic_ref);
        self
    }
}

pub trait LoopRunInfoPort: Send + Sync {
    fn run_context(&self) -> &LoopRunContext;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopContextRequest {
    pub after: Option<LoopInputCursor>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopContextBundle {
    pub messages: Vec<LoopContextMessage>,
    pub instruction_snippets: Vec<LoopContextSnippet>,
    pub memory_snippets: Vec<LoopContextSnippet>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopContextMessage {
    pub message_ref: LoopMessageRef,
    pub role: String,
    pub safe_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopContextSnippet {
    pub snippet_ref: String,
    pub safe_summary: String,
}

#[async_trait]
pub trait LoopContextPort: Send + Sync {
    async fn load_loop_context(
        &self,
        request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInputCursor {
    scope: TurnScope,
    run_id: TurnRunId,
    token: LoopInputCursorToken,
}

impl LoopInputCursor {
    pub fn origin_for_run(context: &LoopRunContext) -> Self {
        Self {
            scope: context.scope.clone(),
            run_id: context.run_id,
            token: origin_input_cursor_token(),
        }
    }

    pub fn from_host_token(context: &LoopRunContext, token: LoopInputCursorToken) -> Self {
        Self {
            scope: context.scope.clone(),
            run_id: context.run_id,
            token,
        }
    }

    pub fn scope(&self) -> &TurnScope {
        &self.scope
    }

    pub fn run_id(&self) -> TurnRunId {
        self.run_id
    }

    pub fn token(&self) -> &LoopInputCursorToken {
        &self.token
    }

    pub fn is_for_run(&self, context: &LoopRunContext) -> bool {
        self.scope == context.scope && self.run_id == context.run_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInputBatch {
    pub inputs: Vec<LoopInput>,
    pub next_cursor: LoopInputCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopInput {
    UserMessage { message_ref: LoopMessageRef },
    FollowUp { message_ref: LoopMessageRef },
    Steering { message_ref: LoopMessageRef },
    Interrupt { kind: LoopInterruptKind },
    Cancel { reason_kind: LoopCancelReasonKind },
    GateResolved { gate_ref: LoopGateRef },
    CapabilitySurfaceChanged { version: CapabilitySurfaceVersion },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopInterruptKind {
    UserInterrupt,
    HostShutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCancelReasonKind {
    UserRequested,
    Superseded,
    Policy,
}

#[async_trait]
pub trait LoopInputPort: Send + Sync {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError>;

    async fn ack_inputs(&self, cursor: LoopInputCursor) -> Result<(), AgentLoopHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct CapabilitySurfaceVersion(String);

impl CapabilitySurfaceVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_loop_safe_identifier(value.into(), "capability surface version", 128).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for CapabilitySurfaceVersion {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for CapabilitySurfaceVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CapabilitySurfaceVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelRequest {
    pub messages: Vec<LoopModelMessage>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    pub model_preference: Option<ModelProfileId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelMessage {
    pub role: String,
    pub content_ref: LoopMessageRef,
}

/// Prompt construction mode requested by an agent-loop driver.
///
/// `TextOnly` builds a prompt from transcript/context message refs and is the
/// only mode supported by [`crate::run_profile::HostManagedLoopPromptPort`]
/// today. `CodeAct` is reserved for a future checkpoint/tool-aware prompt
/// bundle flow and is rejected by the text-only host port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMode {
    TextOnly,
    #[serde(rename = "codeact")]
    CodeAct,
}

impl PromptMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TextOnly => "text_only",
            Self::CodeAct => "codeact",
        }
    }
}

/// Request for a host-managed prompt bundle.
///
/// The optional cursor and checkpoint refs are run-scoped and are validated by
/// host ports before context is loaded. `max_messages` is a host budget hint;
/// zero is rejected and oversized values may be clamped by the implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopPromptBundleRequest {
    pub mode: PromptMode,
    pub context_cursor: Option<LoopInputCursor>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    pub checkpoint_state_ref: Option<LoopCheckpointStateRef>,
    pub max_messages: Option<u32>,
}

/// Prompt bundle returned to a driver.
///
/// The bundle carries model-message references rather than raw prompt text.
/// Drivers pass these refs to [`LoopModelPort`], allowing the host to resolve
/// content under the same run scope and policy checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopPromptBundle {
    pub bundle_ref: LoopPromptBundleRef,
    pub messages: Vec<LoopModelMessage>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
}

/// Host boundary for building prompt bundles before model invocation.
///
/// Implementations own context loading, scoping, prompt-shape policy, and
/// milestone emission. Drivers should not assemble raw prompt strings when a
/// prompt port is available.
#[async_trait]
pub trait LoopPromptPort: Send + Sync {
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelResponse {
    pub chunks: Vec<ModelStreamChunk>,
    pub output: ParentLoopOutput,
    pub effective_model_profile_id: ModelProfileId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelStreamChunk {
    pub safe_text_delta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentLoopOutput {
    AssistantReply(AssistantReply),
    CapabilityCalls(Vec<CapabilityCallCandidate>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantReply {
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityCallCandidate {
    pub surface_version: CapabilitySurfaceVersion,
    pub capability_id: CapabilityId,
    pub input_ref: CapabilityInputRef,
}

#[async_trait]
pub trait LoopModelPort: Send + Sync {
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VisibleCapabilityRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleCapabilitySurface {
    pub version: CapabilitySurfaceVersion,
    pub descriptors: Vec<CapabilityDescriptorView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDescriptorView {
    pub capability_id: CapabilityId,
    pub provider: Option<ExtensionId>,
    pub runtime: RuntimeKind,
    pub safe_name: String,
    pub safe_description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityInvocation {
    pub surface_version: CapabilitySurfaceVersion,
    pub capability_id: CapabilityId,
    pub input_ref: CapabilityInputRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityBatchInvocation {
    pub invocations: Vec<CapabilityInvocation>,
    pub stop_on_first_suspension: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityBatchOutcome {
    pub outcomes: Vec<CapabilityOutcome>,
    pub stopped_on_suspension: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOutcome {
    Completed(CapabilityResultMessage),
    ApprovalRequired {
        gate_ref: LoopGateRef,
        safe_summary: String,
    },
    AuthRequired {
        gate_ref: LoopGateRef,
        safe_summary: String,
    },
    ResourceBlocked {
        gate_ref: LoopGateRef,
        safe_summary: String,
    },
    SpawnedProcess(ProcessHandleSummary),
    Denied(CapabilityDenied),
    Failed(CapabilityFailure),
}

impl CapabilityOutcome {
    pub fn is_suspension(&self) -> bool {
        matches!(
            self,
            Self::ApprovalRequired { .. }
                | Self::AuthRequired { .. }
                | Self::ResourceBlocked { .. }
                | Self::SpawnedProcess(_)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityResultMessage {
    pub result_ref: LoopResultRef,
    pub safe_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessHandleSummary {
    pub process_ref: LoopProcessRef,
    pub safe_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDenied {
    pub reason_kind: CapabilityDeniedReasonKind,
    pub safe_summary: String,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityFailure {
    pub error_kind: String,
    pub safe_summary: String,
}

#[async_trait]
pub trait LoopCapabilityPort: Send + Sync {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError>;

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError>;

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeginAssistantDraft {
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAssistantDraft {
    pub message_ref: LoopMessageRef,
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalizeAssistantMessage {
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendCapabilityResultRef {
    pub result_ref: LoopResultRef,
    pub safe_summary: String,
}

#[async_trait]
pub trait LoopTranscriptPort: Send + Sync {
    async fn begin_assistant_draft(
        &self,
        _request: BeginAssistantDraft,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Err(unsupported_host_method("begin_assistant_draft"))
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraft,
    ) -> Result<(), AgentLoopHostError> {
        Err(unsupported_host_method("update_assistant_draft"))
    }

    async fn finalize_assistant_message(
        &self,
        request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError>;

    async fn append_capability_result_ref(
        &self,
        _request: AppendCapabilityResultRef,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Err(unsupported_host_method("append_capability_result_ref"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopCheckpointRequest {
    pub kind: LoopCheckpointKind,
    pub state_ref: LoopCheckpointStateRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCheckpointKind {
    BeforeModel,
    BeforeSideEffect,
    BeforeBlock,
    Final,
}

impl LoopCheckpointKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BeforeModel => "before_model",
            Self::BeforeSideEffect => "before_side_effect",
            Self::BeforeBlock => "before_block",
            Self::Final => "final",
        }
    }
}

#[async_trait]
pub trait LoopCheckpointPort: Send + Sync {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopProgressEvent {
    DriverNote {
        kind: LoopDriverNoteKind,
        safe_summary: LoopSafeSummary,
    },
}

impl LoopProgressEvent {
    pub fn driver_note(
        kind: LoopDriverNoteKind,
        safe_summary: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self::DriverNote {
            kind,
            safe_summary: LoopSafeSummary::new(safe_summary)?,
        })
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::DriverNote { .. } => "driver_note",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopDriverNoteKind {
    Planning,
    Waiting,
    Retrying,
}

#[async_trait]
pub trait LoopProgressPort: Send + Sync {
    async fn emit_loop_progress(&self, event: LoopProgressEvent) -> Result<(), AgentLoopHostError>;
}

pub trait AgentLoopDriverHost:
    LoopRunInfoPort
    + LoopContextPort
    + LoopPromptPort
    + LoopInputPort
    + LoopModelPort
    + LoopCapabilityPort
    + LoopTranscriptPort
    + LoopCheckpointPort
    + LoopProgressPort
    + Send
    + Sync
{
}

impl<T> AgentLoopDriverHost for T where
    T: LoopRunInfoPort
        + LoopContextPort
        + LoopPromptPort
        + LoopInputPort
        + LoopModelPort
        + LoopCapabilityPort
        + LoopTranscriptPort
        + LoopCheckpointPort
        + LoopProgressPort
        + Send
        + Sync
{
}

pub trait AgentLoopHost: AgentLoopDriverHost {}

impl<T> AgentLoopHost for T where T: AgentLoopDriverHost + ?Sized {}

fn unsupported_host_method(method: &'static str) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        format!("agent loop host method {method} is unavailable"),
    )
}

//! The canonical turn vocabulary for product surfaces and turn services.
//!
//! `ironclaw_turns` owns coordination, admission, scheduling, persistence, and
//! state transitions. This module owns the complete stable language those
//! services, product surfaces, and channel adapters exchange: the typed ids and
//! refs, the turn scope/actor/owner, the run status and its gate
//! correspondence, the event cursor, the run-origin adapter identity, and the
//! sanitized failure/cancel shapes.
//!
//! Nothing here executes, persists, or transitions anything. A crate that only
//! needs to *name* a turn depends on `ironclaw_host_api`, not on
//! `ironclaw_turns` — that is what makes this module complete rather than
//! partial.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    decision::RuntimeCredentialAuthRequirement,
    ids::{AgentId, ProjectId, TenantId, ThreadId, UserId},
    resource::{ResourceScope, SYSTEM_RESERVED_ID},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnId(Uuid);

impl TurnId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for TurnId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TurnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnRunId(Uuid);

impl TurnRunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for TurnRunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TurnRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for TurnRunId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilityActivityId(Uuid);

impl CapabilityActivityId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for CapabilityActivityId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CapabilityActivityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnCheckpointId(Uuid);

impl TurnCheckpointId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for TurnCheckpointId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnLeaseToken(Uuid);

impl TurnLeaseToken {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for TurnLeaseToken {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnRunnerId(Uuid);

impl TurnRunnerId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for TurnRunnerId {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! bounded_ref {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_ref($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

macro_rules! loop_ref {
    ($name:ident, $kind:literal, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_loop_ref($kind, $prefix, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

bounded_ref!(AcceptedMessageRef, "accepted_message_ref");
bounded_ref!(SourceBindingRef, "source_binding_ref");
bounded_ref!(ReplyTargetBindingRef, "reply_target_binding_ref");
bounded_ref!(TurnGateRef, "turn_gate_ref");
bounded_ref!(IdempotencyKey, "idempotency_key");
bounded_ref!(RunProfileRequest, "run_profile_request");
bounded_ref!(RunProfileId, "run_profile_id");
loop_ref!(LoopExitId, "loop_exit_id", "exit:");
loop_ref!(LoopMessageRef, "loop_message_ref", "msg:");
loop_ref!(LoopResultRef, "loop_result_ref", "result:");
loop_ref!(LoopGateRef, "loop_gate_ref", "gate:");

impl PartialEq<LoopGateRef> for TurnGateRef {
    fn eq(&self, other: &LoopGateRef) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<TurnGateRef> for LoopGateRef {
    fn eq(&self, other: &TurnGateRef) -> bool {
        self.as_str() == other.as_str()
    }
}

impl RunProfileId {
    pub fn default_profile() -> Self {
        Self::from_trusted_static("default")
    }

    pub fn interactive_default() -> Self {
        Self::from_trusted_static("interactive_default")
    }

    pub fn long_running_mission() -> Self {
        Self::from_trusted_static("long_running_mission")
    }

    pub fn scheduled_trigger() -> Self {
        Self::from_trusted_static("scheduled_trigger")
    }

    pub fn is_interactive_default(&self) -> bool {
        self == &Self::interactive_default()
    }

    fn from_trusted_static(value: &'static str) -> Self {
        debug_assert!(validate_ref("run_profile_id", value).is_ok());
        Self(value.to_string())
    }

    pub fn from_request(request: &RunProfileRequest) -> Self {
        Self(request.as_str().to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunProfileVersion(u64);

impl RunProfileVersion {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

fn validate_ref(kind: &'static str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > 256 {
        return Err(format!("{kind} must be at most 256 bytes"));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(format!("{kind} must not contain control characters"));
    }
    Ok(())
}

fn validate_loop_ref(kind: &'static str, prefix: &'static str, value: &str) -> Result<(), String> {
    validate_ref(kind, value)?;
    let Some(suffix) = value.strip_prefix(prefix) else {
        return Err(format!("{kind} must start with {prefix}"));
    };
    if suffix.is_empty() {
        return Err(format!("{kind} must include an opaque id after {prefix}"));
    }
    if !suffix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(format!(
            "{kind} opaque id must contain only ASCII letters, digits, _, -, or ."
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnScope {
    pub tenant_id: TenantId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "TurnThreadOwner::is_actor_fallback")]
    pub thread_owner: TurnThreadOwner,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum TurnThreadOwner {
    #[default]
    ActorFallback,
    #[serde(alias = "explicit")]
    ExplicitUser {
        owner_user_id: UserId,
    },
    Ownerless,
}

impl TurnThreadOwner {
    pub fn explicit(owner_user_id: Option<UserId>) -> Self {
        match owner_user_id {
            Some(owner_user_id) => Self::ExplicitUser { owner_user_id },
            None => Self::Ownerless,
        }
    }

    fn is_actor_fallback(&self) -> bool {
        matches!(self, Self::ActorFallback)
    }

    pub fn explicit_owner_user_id(&self) -> Option<&UserId> {
        match self {
            Self::ExplicitUser { owner_user_id } => Some(owner_user_id),
            Self::ActorFallback | Self::Ownerless => None,
        }
    }

    pub fn is_explicit(&self) -> bool {
        !self.is_actor_fallback()
    }
}

impl TurnScope {
    pub fn new(
        tenant_id: TenantId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        thread_id: ThreadId,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            thread_id,
            thread_owner: TurnThreadOwner::ActorFallback,
        }
    }

    pub fn new_with_owner(
        tenant_id: TenantId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        thread_id: ThreadId,
        owner_user_id: Option<UserId>,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            thread_id,
            thread_owner: TurnThreadOwner::explicit(owner_user_id),
        }
    }

    pub fn explicit_owner_user_id(&self) -> Option<&UserId> {
        self.thread_owner.explicit_owner_user_id()
    }

    pub fn same_thread(&self, other: &Self) -> bool {
        self.tenant_id == other.tenant_id
            && self.agent_id == other.agent_id
            && self.project_id == other.project_id
            && self.thread_id == other.thread_id
    }

    pub fn has_explicit_thread_owner(&self) -> bool {
        self.thread_owner.is_explicit()
    }

    pub fn product_owner(&self, actor: &TurnActor) -> TurnOwner {
        if let Some(user) = self.explicit_owner_user_id() {
            TurnOwner::Personal { user: user.clone() }
        } else if let Some(agent) = &self.agent_id {
            TurnOwner::SharedAgent {
                agent: agent.clone(),
                project: self.project_id.clone(),
            }
        } else {
            TurnOwner::Personal {
                user: actor.user_id.clone(),
            }
        }
    }

    pub fn to_resource_scope(&self) -> ResourceScope {
        let mut scope = ResourceScope::system();
        scope.tenant_id = self.tenant_id.clone();
        scope.user_id = self
            .explicit_owner_user_id()
            .cloned()
            .unwrap_or_else(|| UserId::from_trusted(SYSTEM_RESERVED_ID.to_string()));
        scope.agent_id = self.agent_id.clone();
        scope.project_id = self.project_id.clone();
        scope.mission_id = None;
        scope.thread_id = Some(self.thread_id.clone());
        scope
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnActor {
    pub user_id: UserId,
}

impl TurnActor {
    pub fn new(user_id: UserId) -> Self {
        Self { user_id }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TurnOwner {
    Personal {
        user: UserId,
    },
    SharedAgent {
        agent: AgentId,
        project: Option<ProjectId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SanitizedFailure {
    category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

const MODEL_INVALID_OUTPUT_DETAIL_MAX_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelInvalidOutputDetailReason {
    EmptyAssistantResponse,
    TextualToolCallSyntax,
    OutsideCapabilitySurface,
    ToolUseFinishWithoutToolCalls,
    UnsupportedToolCallsForTextOnlyLoop,
    InvalidReturnedToolName,
    InvalidToolCallArguments,
    MalformedToolCallArguments,
}

impl ModelInvalidOutputDetailReason {
    pub const TOOL_CALL_ARGUMENTS_PARSE_ERROR_PREFIX: &'static str =
        "failed to parse tool-call arguments JSON:";

    pub fn safe_summary(self) -> &'static str {
        match self {
            Self::EmptyAssistantResponse => "model returned an empty assistant response",
            Self::TextualToolCallSyntax => {
                "model returned textual tool-call syntax instead of structured tool calls"
            }
            Self::OutsideCapabilitySurface => {
                "model returned a tool call outside the advertised capability surface"
            }
            Self::ToolUseFinishWithoutToolCalls => {
                "model returned tool-use finish without tool calls"
            }
            Self::UnsupportedToolCallsForTextOnlyLoop => {
                "model returned unsupported tool calls for a text-only loop"
            }
            Self::InvalidReturnedToolName => "model returned an invalid provider tool name",
            Self::InvalidToolCallArguments => "model returned invalid tool-call arguments",
            Self::MalformedToolCallArguments => Self::TOOL_CALL_ARGUMENTS_PARSE_ERROR_PREFIX,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::EmptyAssistantResponse => "empty_assistant_response",
            Self::TextualToolCallSyntax => "textual_tool_call_syntax",
            Self::OutsideCapabilitySurface => "outside_capability_surface",
            Self::ToolUseFinishWithoutToolCalls => "tool_use_finish_without_tool_calls",
            Self::UnsupportedToolCallsForTextOnlyLoop => {
                "unsupported_tool_calls_for_text_only_loop"
            }
            Self::InvalidReturnedToolName => "invalid_returned_tool_name",
            Self::InvalidToolCallArguments => "invalid_tool_call_arguments",
            Self::MalformedToolCallArguments => "malformed_tool_call_arguments",
        }
    }

    pub fn from_failure_category_and_safe_summary(
        category: &str,
        safe_summary: Option<&str>,
    ) -> Option<Self> {
        if !matches!(category, "model_invalid_output" | "invalid_model_output") {
            return None;
        }
        Self::from_safe_summary(safe_summary?)
    }

    pub fn from_safe_summary(safe_summary: &str) -> Option<Self> {
        if !is_model_invalid_output_detail_shape(safe_summary) {
            return None;
        }
        match safe_summary {
            "model returned an empty assistant response" => Some(Self::EmptyAssistantResponse),
            "model returned textual tool-call syntax instead of structured tool calls" => {
                Some(Self::TextualToolCallSyntax)
            }
            "model returned a tool call outside the advertised capability surface" => {
                Some(Self::OutsideCapabilitySurface)
            }
            "model returned tool-use finish without tool calls" => {
                Some(Self::ToolUseFinishWithoutToolCalls)
            }
            "model returned unsupported tool calls for a text-only loop" => {
                Some(Self::UnsupportedToolCallsForTextOnlyLoop)
            }
            "model returned an invalid provider tool name" => Some(Self::InvalidReturnedToolName),
            "model returned invalid tool-call arguments" => Some(Self::InvalidToolCallArguments),
            _ if safe_summary.starts_with(Self::TOOL_CALL_ARGUMENTS_PARSE_ERROR_PREFIX) => {
                Some(Self::MalformedToolCallArguments)
            }
            _ => None,
        }
    }
}

fn is_model_invalid_output_detail_shape(detail: &str) -> bool {
    if detail.is_empty() || detail.len() > MODEL_INVALID_OUTPUT_DETAIL_MAX_BYTES {
        return false;
    }
    if !detail.is_ascii() {
        return false;
    }
    let bytes = detail.as_bytes();
    !bytes[0].is_ascii_whitespace()
        && !bytes[bytes.len() - 1].is_ascii_whitespace()
        && !bytes.iter().any(u8::is_ascii_control)
}

impl SanitizedFailure {
    pub fn new(category: impl Into<String>) -> Result<Self, String> {
        let category = category.into();
        validate_sanitized_category("failure_category", &category)?;
        Ok(Self {
            category,
            detail: None,
        })
    }

    pub fn from_trusted_static(category: &'static str) -> Self {
        debug_assert!(validate_sanitized_category("failure_category", category).is_ok());
        Self {
            category: category.to_string(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    pub fn into_category(self) -> String {
        self.category
    }

    pub fn public_projection(&self) -> Self {
        Self {
            category: self.category.clone(),
            detail: None,
        }
    }
}

impl<'de> Deserialize<'de> for SanitizedFailure {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireFailure {
            category: String,
            #[serde(default)]
            detail: Option<String>,
        }

        let wire = WireFailure::deserialize(deserializer)?;
        let normalized = match wire.category.split_once(':') {
            Some((left, right))
                if !left.is_empty() && !right.is_empty() && !right.contains(':') =>
            {
                format!("{left}_{right}")
            }
            _ => wire.category,
        };
        let mut failure = Self::new(normalized).map_err(serde::de::Error::custom)?;
        failure.detail = wire.detail;
        Ok(failure)
    }
}

fn validate_sanitized_category(kind: &'static str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > 256 {
        return Err(format!("{kind} must be at most 256 bytes"));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(format!("{kind} must not contain control characters"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(format!(
            "{kind} must contain only lowercase ASCII letters, digits, or underscores"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SanitizedCancelReason {
    UserRequested,
    Superseded,
    Timeout,
    OperatorRequested,
    Policy,
}

impl SanitizedCancelReason {
    pub fn category(self) -> &'static str {
        match self {
            Self::UserRequested => "user_requested",
            Self::Superseded => "superseded",
            Self::Timeout => "timeout",
            Self::OperatorRequested => "operator_requested",
            Self::Policy => "policy",
        }
    }
}

/// Monotonic position in a turn's lifecycle event stream. Projections and
/// delivery handoffs carry it so a reader can resume exactly where it stopped.
///
/// Distinct from `ironclaw_events::EventCursor`, which positions the generic
/// durable event log rather than one turn's lifecycle stream.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
#[serde(transparent)]
pub struct EventCursor(pub u64);

/// Maximum byte length for a [`RunOriginAdapter`] value. Mirrors `AdapterKind`'s
/// validation bound in `ironclaw_conversations` so that any valid `AdapterKind`
/// always converts without narrowing. If `AdapterKind`'s limit changes, update
/// this constant to match.
pub const MAX_RUN_ORIGIN_ADAPTER_BYTES: usize = 512;

/// Generic adapter identity carried into the turn context. Bounded validated string;
/// callers convert their rich adapter id (e.g. `ProductAdapterId`, `AdapterKind`) into this.
///
/// Serializes as a plain string. Deserialization validates via `TryFrom<String>` so
/// persisted payloads with empty or oversized values are rejected at the boundary.
///
/// The byte-length cap matches `AdapterKind`'s validation bound (512 bytes) so that
/// any valid `AdapterKind` always converts into a `RunOriginAdapter` without silent
/// narrowing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct RunOriginAdapter(String);

impl RunOriginAdapter {
    /// The single rejection message every invalid-adapter path renders. Callers
    /// surface it verbatim through their own error types, so it is part of the
    /// contract rather than an implementation detail.
    pub const INVALID_MESSAGE: &'static str = "invalid run-origin adapter: must be 1..=512 bytes";

    fn validate(value: &str) -> Result<(), String> {
        if value.is_empty() || value.len() > MAX_RUN_ORIGIN_ADAPTER_BYTES {
            return Err(Self::INVALID_MESSAGE.to_string());
        }
        Ok(())
    }

    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for RunOriginAdapter {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl AsRef<str> for RunOriginAdapter {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RunOriginAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<RunOriginAdapter> for String {
    fn from(value: RunOriginAdapter) -> Self {
        value.0
    }
}

/// The lifecycle position of one turn run. Terminal, gate-parked, and in-flight
/// statuses are distinguished by the predicates below rather than by scattered
/// match tables at the call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnStatus {
    Queued,
    Running,
    BlockedApproval,
    BlockedAuth,
    BlockedResource,
    BlockedDependentRun,
    /// Blocked on a client-supplied ("external") tool call: the model invoked a
    /// caller-declared tool that the agent loop does not execute. The run is
    /// parked, control returns to the API client, and the client resumes the run
    /// by submitting the tool output. Non-terminal; keeps the active lock.
    BlockedExternalTool,
    CancelRequested,
    Cancelled,
    Completed,
    Failed,
    RecoveryRequired,
}

impl TurnStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Cancelled | Self::Completed | Self::Failed | Self::RecoveryRequired
        )
    }

    /// Whether the run is parked on a gate (approval/auth/resource/dependent
    /// run/external tool). Used to decide when a transition changed the set of
    /// gate-blocked runs and therefore needs durable persistence.
    pub fn is_blocked(self) -> bool {
        GateKind::from_status(self).is_some()
    }

    pub fn keeps_active_lock(self) -> bool {
        !self.is_terminal()
    }
}

/// The canonical kind of gate a run can park on. One value per blocked
/// [`TurnStatus`]; every other blocked-gate representation in the stack
/// ([`BlockedReason`], `TurnBlockedGateKind`, `LoopBlockedKind`,
/// `ResumeTurnPrecondition`) maps to and from this so the correspondence lives
/// in one place and adding a gate kind is a compiler-forced edit here rather
/// than across ~6 scattered match tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    Approval,
    Auth,
    Resource,
    AwaitDependentRun,
    ExternalTool,
}

impl GateKind {
    /// The blocked [`TurnStatus`] a run in this gate holds. The single
    /// authoritative `GateKind -> TurnStatus` correspondence.
    pub fn blocked_status(self) -> TurnStatus {
        match self {
            Self::Approval => TurnStatus::BlockedApproval,
            Self::Auth => TurnStatus::BlockedAuth,
            Self::Resource => TurnStatus::BlockedResource,
            Self::AwaitDependentRun => TurnStatus::BlockedDependentRun,
            Self::ExternalTool => TurnStatus::BlockedExternalTool,
        }
    }

    /// The gate kind a [`TurnStatus`] represents, or `None` when the status is
    /// not a blocked-gate status. Exhaustive over [`TurnStatus`], so a new
    /// blocked variant fails to compile until it is classified here.
    pub fn from_status(status: TurnStatus) -> Option<Self> {
        match status {
            TurnStatus::BlockedApproval => Some(Self::Approval),
            TurnStatus::BlockedAuth => Some(Self::Auth),
            TurnStatus::BlockedResource => Some(Self::Resource),
            TurnStatus::BlockedDependentRun => Some(Self::AwaitDependentRun),
            TurnStatus::BlockedExternalTool => Some(Self::ExternalTool),
            TurnStatus::Queued
            | TurnStatus::Running
            | TurnStatus::CancelRequested
            | TurnStatus::Cancelled
            | TurnStatus::Completed
            | TurnStatus::Failed
            | TurnStatus::RecoveryRequired => None,
        }
    }

    /// Build the data-carrying [`BlockedReason`] for this gate kind. Only `Auth`
    /// carries credential requirements; the rest ignore them.
    pub fn into_blocked_reason(
        self,
        gate_ref: TurnGateRef,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    ) -> BlockedReason {
        match self {
            Self::Approval => BlockedReason::Approval { gate_ref },
            Self::Auth => BlockedReason::Auth {
                gate_ref,
                credential_requirements,
            },
            Self::Resource => BlockedReason::Resource { gate_ref },
            Self::AwaitDependentRun => BlockedReason::AwaitDependentRun { gate_ref },
            Self::ExternalTool => BlockedReason::ExternalTool { gate_ref },
        }
    }
}

/// Why a run is parked, with the gate ref a resume must present. The
/// data-carrying counterpart of [`GateKind`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockedReason {
    Approval {
        gate_ref: TurnGateRef,
    },
    Auth {
        gate_ref: TurnGateRef,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    },
    Resource {
        gate_ref: TurnGateRef,
    },
    #[serde(alias = "DependentRun")]
    AwaitDependentRun {
        gate_ref: TurnGateRef,
    },
    ExternalTool {
        gate_ref: TurnGateRef,
    },
}

impl BlockedReason {
    /// The gate kind this reason represents.
    pub fn gate_kind(&self) -> GateKind {
        match self {
            Self::Approval { .. } => GateKind::Approval,
            Self::Auth { .. } => GateKind::Auth,
            Self::Resource { .. } => GateKind::Resource,
            Self::AwaitDependentRun { .. } => GateKind::AwaitDependentRun,
            Self::ExternalTool { .. } => GateKind::ExternalTool,
        }
    }

    pub fn status(&self) -> TurnStatus {
        self.gate_kind().blocked_status()
    }

    pub fn gate_ref(&self) -> &TurnGateRef {
        match self {
            Self::Approval { gate_ref }
            | Self::Auth { gate_ref, .. }
            | Self::Resource { gate_ref }
            | Self::AwaitDependentRun { gate_ref }
            | Self::ExternalTool { gate_ref } => gate_ref,
        }
    }

    pub fn credential_requirements(&self) -> &[RuntimeCredentialAuthRequirement] {
        match self {
            Self::Auth {
                credential_requirements,
                ..
            } => credential_requirements,
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_external_tool_status_is_non_terminal_and_keeps_lock() {
        assert!(!TurnStatus::BlockedExternalTool.is_terminal());
        assert!(TurnStatus::BlockedExternalTool.keeps_active_lock());
    }

    #[test]
    fn blocked_external_tool_status_round_trips() {
        let json = serde_json::to_string(&TurnStatus::BlockedExternalTool).expect("serialize");
        assert_eq!(json, "\"BlockedExternalTool\"");
        let decoded: TurnStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, TurnStatus::BlockedExternalTool);
    }

    #[test]
    fn blocked_reason_external_tool_maps_status_and_exposes_gate_ref() {
        let gate_ref = TurnGateRef::new("gate:ext-tool").expect("valid gate ref");
        let reason = BlockedReason::ExternalTool {
            gate_ref: gate_ref.clone(),
        };
        assert_eq!(reason.status(), TurnStatus::BlockedExternalTool);
        assert_eq!(reason.gate_ref(), &gate_ref);
        assert!(reason.credential_requirements().is_empty());

        // Round-trips through the untagged-by-variant-name BlockedReason enum.
        let json = serde_json::to_string(&reason).expect("serialize");
        let decoded: BlockedReason = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, reason);
    }

    #[test]
    fn turn_gate_ref_is_bounded_only_unlike_the_prefix_validated_loop_gate_ref() {
        // `TurnGateRef` is `bounded_ref!`, NOT `loop_ref!`: production mints
        // `gate:approval-{id}` / `gate:auth-{id}` and `is_auth_gate_ref`-style
        // predicates match that prefix, but the *type* enforces only
        // non-empty / <= 256 bytes / no control characters. Callers and
        // fixtures across the workspace legitimately hold unprefixed refs
        // (`"gate-alpha"`, `"custom-auth-gate"`, `"stress-gate:{run_id}"`), so
        // tightening this constructor would break them and orphan every
        // persisted ref. `LoopGateRef` is the prefix-validated family.
        for accepted in [
            "gate-alpha",
            "custom-auth-gate",
            "approval:missing",
            "gate:auth-1",
        ] {
            assert!(
                TurnGateRef::new(accepted).is_ok(),
                "{accepted:?} must remain a valid TurnGateRef"
            );
        }
        assert!(TurnGateRef::new("").is_err());
        assert!(TurnGateRef::new("x".repeat(257)).is_err());
        assert!(TurnGateRef::new("gate:has\u{7f}control").is_err());

        // The contrast that motivates this test.
        assert!(LoopGateRef::new("gate:alpha").is_ok());
        assert!(
            LoopGateRef::new("gate-alpha").is_err(),
            "LoopGateRef is the prefix-validated family"
        );
    }

    #[test]
    fn run_origin_adapter_rejects_empty() {
        assert!(RunOriginAdapter::new("").is_err());
    }

    #[test]
    fn run_origin_adapter_accepts_at_max_bytes() {
        // Exactly at the limit must succeed — mirrors AdapterKind's 512-byte cap.
        let at_limit = "a".repeat(MAX_RUN_ORIGIN_ADAPTER_BYTES);
        assert!(
            RunOriginAdapter::new(at_limit).is_ok(),
            "adapter at exactly {MAX_RUN_ORIGIN_ADAPTER_BYTES} bytes must be accepted"
        );
    }

    #[test]
    fn run_origin_adapter_rejects_over_512_bytes() {
        let overlong = "a".repeat(MAX_RUN_ORIGIN_ADAPTER_BYTES + 1);
        assert!(
            RunOriginAdapter::new(overlong).is_err(),
            "adapter exceeding {MAX_RUN_ORIGIN_ADAPTER_BYTES} bytes must be rejected"
        );
    }

    #[test]
    fn run_origin_adapter_rejection_message_is_the_one_callers_render() {
        // `ironclaw_product` and `ironclaw_conversations` surface this string
        // verbatim inside their own submission errors, and `TurnError::
        // InvalidRunOriginAdapter` renders the same wording, so the text is a
        // contract rather than an implementation detail.
        assert_eq!(
            RunOriginAdapter::new("").unwrap_err(),
            RunOriginAdapter::INVALID_MESSAGE
        );
        assert_eq!(
            RunOriginAdapter::INVALID_MESSAGE,
            "invalid run-origin adapter: must be 1..=512 bytes"
        );
    }

    #[test]
    fn run_origin_adapter_round_trips_as_a_plain_string_and_validates_on_read() {
        let adapter = RunOriginAdapter::new("slack").expect("valid adapter");
        let json = serde_json::to_string(&adapter).expect("serialize");
        assert_eq!(json, "\"slack\"");
        assert_eq!(
            serde_json::from_str::<RunOriginAdapter>(&json).expect("deserialize"),
            adapter
        );
        assert!(
            serde_json::from_str::<RunOriginAdapter>("\"\"").is_err(),
            "an empty persisted adapter must be rejected at the boundary"
        );
    }

    #[test]
    fn model_invalid_output_detail_reason_round_trips_fixed_safe_summaries() {
        use ModelInvalidOutputDetailReason as Reason;

        for reason in [
            Reason::EmptyAssistantResponse,
            Reason::TextualToolCallSyntax,
            Reason::OutsideCapabilitySurface,
            Reason::ToolUseFinishWithoutToolCalls,
            Reason::UnsupportedToolCallsForTextOnlyLoop,
            Reason::InvalidReturnedToolName,
            Reason::InvalidToolCallArguments,
        ] {
            assert_eq!(
                Reason::from_failure_category_and_safe_summary(
                    "model_invalid_output",
                    Some(reason.safe_summary()),
                ),
                Some(reason),
                "{reason:?} safe summary should parse back to the same reason"
            );
        }
    }

    #[test]
    fn model_invalid_output_detail_reason_accepts_safe_parse_error_prefix() {
        assert_eq!(
            ModelInvalidOutputDetailReason::from_failure_category_and_safe_summary(
                "model_invalid_output",
                Some("failed to parse tool-call arguments JSON: expected value at line 1 column 1"),
            ),
            Some(ModelInvalidOutputDetailReason::MalformedToolCallArguments)
        );
    }

    #[test]
    fn model_invalid_output_detail_reason_is_category_gated() {
        assert_eq!(
            ModelInvalidOutputDetailReason::from_failure_category_and_safe_summary(
                "model_unavailable",
                Some(ModelInvalidOutputDetailReason::EmptyAssistantResponse.safe_summary()),
            ),
            None
        );
    }

    #[test]
    fn model_invalid_output_detail_reason_rejects_unvalidated_detail() {
        let oversized = format!(
            "failed to parse tool-call arguments JSON: {}",
            "x".repeat(512)
        );

        for detail in [
            " model returned an empty assistant response",
            "model returned an empty assistant response\n",
            "model returned an empty assistant response\0",
            oversized.as_str(),
        ] {
            assert_eq!(
                ModelInvalidOutputDetailReason::from_failure_category_and_safe_summary(
                    "model_invalid_output",
                    Some(detail),
                ),
                None,
                "{detail:?} should not be accepted for projection matching"
            );
        }
    }

    #[test]
    fn sanitized_failure_accepts_snake_case_category() {
        let failure =
            SanitizedFailure::new("host_stage_unavailable_model").expect("category is valid");
        assert_eq!(failure.category(), "host_stage_unavailable_model");
    }

    #[test]
    fn sanitized_failure_rejects_colons() {
        for invalid in [
            "host_stage_unavailable:model",
            "a::b",
            ":model",
            "host_stage_unavailable:",
            ":",
        ] {
            assert!(
                SanitizedFailure::new(invalid).is_err(),
                "category {invalid:?} with a colon must be rejected"
            );
        }
    }

    #[test]
    fn sanitized_failure_deserialize_normalizes_legacy_colon_categories() {
        // Historical persisted rows used the single-colon category
        // `host_stage_unavailable:model`. The strict write path rejects it, but
        // loading a snapshot must not fail — the read path normalizes that exact
        // shape so old data stays loadable.
        let failure: SanitizedFailure =
            serde_json::from_str(r#"{"category":"host_stage_unavailable:model"}"#)
                .expect("legacy colon category must deserialize");
        assert_eq!(failure.category(), "host_stage_unavailable_model");
    }

    #[test]
    fn sanitized_failure_deserialize_rejects_malformed_colon_categories() {
        // Normalization is restricted to the one legacy shape. Malformed colon
        // payloads must still be rejected, not silently minted into values the
        // strict write path could never produce (e.g. `a::b` -> `a__b`).
        for malformed in ["a::b", ":model", "host_stage_unavailable:", ":", "a:b:c"] {
            let json = format!(r#"{{"category":"{malformed}"}}"#);
            assert!(
                serde_json::from_str::<SanitizedFailure>(&json).is_err(),
                "malformed colon category {malformed:?} must be rejected"
            );
        }
    }

    #[test]
    fn sanitized_failure_legacy_row_without_detail_round_trips() {
        // Pre-detail persisted rows omit the field. `serde(default)` must
        // rehydrate them as `detail == None`, and re-serializing must not
        // re-introduce a `detail` key (`skip_serializing_if`).
        let failure: SanitizedFailure = serde_json::from_str(r#"{"category":"model_unavailable"}"#)
            .expect("legacy row without detail must deserialize");
        assert_eq!(failure.category(), "model_unavailable");
        assert_eq!(failure.detail(), None);

        let reserialized = serde_json::to_string(&failure).expect("serialize");
        assert_eq!(reserialized, r#"{"category":"model_unavailable"}"#);
    }

    #[test]
    fn sanitized_failure_with_detail_round_trips() {
        let failure = SanitizedFailure::new("model_unavailable")
            .expect("category")
            .with_detail("HTTP 404 model not found");
        assert_eq!(failure.detail(), Some("HTTP 404 model not found"));

        let json = serde_json::to_string(&failure).expect("serialize");
        let restored: SanitizedFailure = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, failure);
        assert_eq!(restored.detail(), Some("HTTP 404 model not found"));
    }

    #[test]
    fn public_projection_strips_detail_and_keeps_category() {
        let failure = SanitizedFailure::new("model_unavailable")
            .expect("category")
            .with_detail("HTTP 500 from provider at /internal/models/route-xyz");

        let public = failure.public_projection();
        assert_eq!(public.category(), "model_unavailable");
        assert_eq!(
            public.detail(),
            None,
            "public projection must not carry the model-visible detail"
        );

        // Serialized public shape omits the detail key entirely, and the
        // original is left untouched (projection is a copy).
        let rendered = serde_json::to_string(&public).expect("serialize");
        assert_eq!(rendered, r#"{"category":"model_unavailable"}"#);
        assert_eq!(
            failure.detail(),
            Some("HTTP 500 from provider at /internal/models/route-xyz")
        );
    }
}

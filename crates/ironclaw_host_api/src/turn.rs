//! Shared turn vocabulary for product surfaces and turn services.
//!
//! The turn service crate owns coordination, scheduling, persistence, and
//! state transitions. This module owns the stable language that product
//! surfaces, channel adapters, and those services all exchange.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AgentId, ProjectId, ResourceScope, SYSTEM_RESERVED_ID, TenantId, ThreadId, UserId};

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
loop_ref!(LoopDiagnosticRef, "loop_diagnostic_ref", "diag:");

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

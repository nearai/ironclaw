use serde::{Deserialize, Serialize};
use thiserror::Error;

use ironclaw_host_api::RuntimeCredentialAuthRequirement;

use crate::{
    AcceptedMessageRef, CapabilityActivityId, GateRef, ProductTurnContext, ReplyTargetBindingRef,
    ResolvedRunProfile, RunProfileId, RunProfileVersion, SourceBindingRef, TurnActor,
    TurnAdmissionClass, TurnCheckpointId, TurnId, TurnRunId, TurnScope, events::EventCursor,
    request::TurnTimestamp, run_profile::LoopModelRouteSnapshot,
};

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
        matches!(
            self,
            Self::BlockedApproval
                | Self::BlockedAuth
                | Self::BlockedResource
                | Self::BlockedDependentRun
                | Self::BlockedExternalTool
        )
    }

    pub fn keeps_active_lock(self) -> bool {
        !self.is_terminal()
    }
}

/// The recoverability-critical transition boundary (#6263 Step 3 / #6284 / Step 5b).
///
/// A run status is **recoverability-critical** when it is a gate-park
/// ([`TurnStatus::is_blocked`]), a terminal ([`TurnStatus::is_terminal`]), or a
/// [`TurnStatus::CancelRequested`]:
///
/// * losing a gate-park strands a run away from the human who must act on it;
/// * losing a terminal re-runs an already-performed side effect, or loses the
///   sanitized, model-visible failure cause the model must see;
/// * losing a `CancelRequested` re-runs work the caller was told was cancelled:
///   `request_cancel` reports success once the transition is committed, so a
///   write-behind crash that reverts it to `Running`/`Queued` would execute a
///   run the caller successfully cancelled (and drop its idempotency record).
///   The caller is waiting on this transition exactly as on a gate-park.
///
/// These transitions MUST stay synchronously durable even under async
/// write-behind: the async path may move only NON-critical transitions off the
/// synchronous ack. The row store's `delta_is_recoverability_critical` also
/// treats a brand-new run (one `baseline` has never seen — `submit_turn`,
/// `submit_child_turn`, and the runs `resume_turn`/`retry_turn` spawn) as
/// critical: it has no durable fallback to recover from if lost. The
/// crash-consistency suite references THIS function (not a copy) as the
/// single boundary write-behind flips.
pub fn is_recoverability_critical(status: TurnStatus) -> bool {
    status.is_blocked() || status.is_terminal() || matches!(status, TurnStatus::CancelRequested)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnActiveRunRefState {
    Missing,
    Nonterminal,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TurnRunProfile {
    pub id: RunProfileId,
    pub version: RunProfileVersion,
    #[serde(default = "TurnAdmissionClass::interactive")]
    pub admission_class: TurnAdmissionClass,
    pub allow_steering: bool,
    pub auto_queue_followups: bool,
    pub resolved: ResolvedRunProfile,
}

impl TurnRunProfile {
    pub fn from_resolved(resolved: ResolvedRunProfile) -> Self {
        let id = compatibility_profile_id(&resolved);
        Self {
            id,
            version: resolved.profile_version,
            admission_class: TurnAdmissionClass::interactive(),
            allow_steering: resolved.steering_policy.allow_steering,
            auto_queue_followups: false,
            resolved,
        }
    }
}

impl<'de> Deserialize<'de> for TurnRunProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireProfile {
            id: RunProfileId,
            version: RunProfileVersion,
            #[serde(default = "TurnAdmissionClass::interactive")]
            admission_class: TurnAdmissionClass,
            allow_steering: bool,
            auto_queue_followups: bool,
            resolved: Option<ResolvedRunProfile>,
        }

        let wire = WireProfile::deserialize(deserializer)?;
        let resolved = wire.resolved.unwrap_or_else(|| {
            ResolvedRunProfile::legacy_compatibility(
                wire.id.clone(),
                wire.version,
                wire.allow_steering,
            )
        });
        Ok(Self {
            id: wire.id,
            version: wire.version,
            admission_class: wire.admission_class,
            allow_steering: wire.allow_steering,
            auto_queue_followups: wire.auto_queue_followups,
            resolved,
        })
    }
}

fn compatibility_profile_id(resolved: &ResolvedRunProfile) -> RunProfileId {
    if resolved.profile_id.is_interactive_default() {
        RunProfileId::default_profile()
    } else {
        resolved.profile_id.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockedReason {
    Approval {
        gate_ref: GateRef,
    },
    Auth {
        gate_ref: GateRef,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    },
    Resource {
        gate_ref: GateRef,
    },
    #[serde(alias = "DependentRun")]
    AwaitDependentRun {
        gate_ref: GateRef,
    },
    ExternalTool {
        gate_ref: GateRef,
    },
}

impl BlockedReason {
    pub fn status(&self) -> TurnStatus {
        match self {
            Self::Approval { .. } => TurnStatus::BlockedApproval,
            Self::Auth { .. } => TurnStatus::BlockedAuth,
            Self::Resource { .. } => TurnStatus::BlockedResource,
            Self::AwaitDependentRun { .. } => TurnStatus::BlockedDependentRun,
            Self::ExternalTool { .. } => TurnStatus::BlockedExternalTool,
        }
    }

    pub fn gate_ref(&self) -> &GateRef {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SanitizedFailure {
    category: String,
    /// Secret-scrubbed, model-visible raw cause for this failure (e.g.
    /// `"HTTP 404 model not found"`). Unlike `category`, this is free-form text
    /// — the producer is responsible for scrubbing secret VALUES upstream (see
    /// [`crate::run_profile::sanitize_model_visible_text`]). Optional and
    /// serialized only when present so persisted pre-detail rows round-trip
    /// without migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

const MODEL_INVALID_OUTPUT_DETAIL_MAX_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    pub(crate) fn from_trusted_static(category: &'static str) -> Self {
        debug_assert!(validate_sanitized_category("failure_category", category).is_ok());
        Self {
            category: category.to_string(),
            detail: None,
        }
    }

    /// Attach a secret-scrubbed, model-visible detail string. The caller is
    /// responsible for scrubbing secret VALUES before calling this (see
    /// [`Self::detail`]).
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

    /// A copy of this failure with the model-visible `detail` stripped.
    ///
    /// `detail` is free-form, model-visible raw cause text intended only for
    /// the failure-explainer prompt — it can carry backend diagnostics (paths,
    /// provider errors, internal context) and is scrubbed for secret VALUES,
    /// not for public exposure. Public/WebUI surfaces must serialize this
    /// projection instead of the raw failure so that internal detail never
    /// reaches the browser; `category` (the user-facing signal) is retained.
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
        // Backward compatibility: historical rows used the colon-delimited
        // category `host_stage_unavailable:model` (one colon between two
        // non-empty parts) before the charset was tightened to reject `:`.
        // Normalize *only* that exact shape on the read path so loading a
        // persisted snapshot never borks — a failed deserialize here would
        // defeat the whole no-borking-failures goal. Any other colon payload
        // (`a::b`, `:model`, `host_stage_unavailable:`, `:`) is passed through
        // untouched so `Self::new` still rejects it; we must not mint values the
        // strict write path could never produce. The write path stays strict.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionRejectionReason {
    TenantLimit,
    ProfileRejected,
    Policy,
    Unauthorized,
    Unavailable,
}

impl AdmissionRejectionReason {
    pub fn category(self) -> &'static str {
        match self {
            Self::TenantLimit => "tenant_limit",
            Self::ProfileRejected => "profile_rejected",
            Self::Policy => "policy",
            Self::Unauthorized => "unauthorized",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionRejection {
    pub reason: AdmissionRejectionReason,
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_denial: Option<crate::TurnAdmissionCapacityDenial>,
}

impl AdmissionRejection {
    pub fn new(reason: AdmissionRejectionReason) -> Self {
        Self {
            reason,
            retry_after_ms: None,
            capacity_denial: None,
        }
    }

    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    pub fn with_capacity_denial(mut self, denial: crate::TurnAdmissionCapacityDenial) -> Self {
        self.retry_after_ms = denial.retry_after_ms;
        self.capacity_denial = Some(denial);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRunState {
    pub scope: TurnScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<TurnActor>,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub resolved_run_profile_id: RunProfileId,
    pub resolved_run_profile_version: RunProfileVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    /// Cumulative provider-reported token usage for this run's model calls,
    /// captured at loop exit. `None` for runs that reported no usage (replay
    /// stubs) or that pre-date usage capture. Read by the OpenAI-compatible
    /// Responses/Chat surfaces to report `usage` and cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<crate::run_profile::LoopModelUsage>,
    pub received_at: TurnTimestamp,
    pub checkpoint_id: Option<TurnCheckpointId>,
    pub gate_ref: Option<GateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_activity_id: Option<CapabilityActivityId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    pub failure: Option<SanitizedFailure>,
    pub event_cursor: EventCursor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<ProductTurnContext>,
    #[serde(
        rename = "auth_resume_disposition",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub resume_disposition: Option<crate::GateResumeDisposition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnErrorCategory {
    ThreadBusy,
    AdmissionRejected,
    ScopeNotFound,
    Unauthorized,
    InvalidRequest,
    Unavailable,
    Conflict,
    CapacityExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnCapacityResource {
    SpawnTreeDescendants,
    SubmitTurn,
    #[serde(other)]
    Replayed,
}

impl TurnCapacityResource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SpawnTreeDescendants => "spawn_tree_descendants",
            Self::SubmitTurn => "submit_turn",
            Self::Replayed => "replayed",
        }
    }
}

impl std::fmt::Display for TurnCapacityResource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TurnError {
    #[error("thread already has an active run")]
    ThreadBusy(crate::response::ThreadBusy),
    #[error("turn admission rejected: {0:?}")]
    AdmissionRejected(AdmissionRejection),
    #[error("turn run not found")]
    ScopeNotFound,
    #[error("turn request is unauthorized")]
    Unauthorized,
    #[error("invalid turn request: {reason}")]
    InvalidRequest { reason: String },
    #[error("turn service unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("turn conflict: {reason}")]
    Conflict { reason: String },
    #[error("turn capacity exceeded for {resource}: cap {cap}")]
    CapacityExceeded {
        resource: TurnCapacityResource,
        cap: u64,
    },
    #[error("turn run {run_id} is not retryable")]
    RunNotRetryable { run_id: TurnRunId },
    #[error("invalid turn transition from {from:?} to {to:?}")]
    InvalidTransition { from: TurnStatus, to: TurnStatus },
    #[error("turn run lease mismatch")]
    LeaseMismatch,
    // Keep the byte limit in sync with `origin::MAX_RUN_ORIGIN_ADAPTER_BYTES`.
    #[error("invalid run-origin adapter: must be 1..=512 bytes")]
    InvalidRunOriginAdapter,
}

impl TurnError {
    pub fn category(&self) -> TurnErrorCategory {
        match self {
            Self::ThreadBusy(_) => TurnErrorCategory::ThreadBusy,
            Self::AdmissionRejected(rejection) => match rejection.reason {
                AdmissionRejectionReason::TenantLimit => TurnErrorCategory::AdmissionRejected,
                AdmissionRejectionReason::ProfileRejected => TurnErrorCategory::InvalidRequest,
                AdmissionRejectionReason::Policy | AdmissionRejectionReason::Unauthorized => {
                    TurnErrorCategory::Unauthorized
                }
                AdmissionRejectionReason::Unavailable => TurnErrorCategory::Unavailable,
            },
            Self::ScopeNotFound => TurnErrorCategory::ScopeNotFound,
            Self::Unauthorized => TurnErrorCategory::Unauthorized,
            Self::InvalidRequest { .. } => TurnErrorCategory::InvalidRequest,
            Self::Unavailable { .. } => TurnErrorCategory::Unavailable,
            Self::Conflict { .. }
            | Self::RunNotRetryable { .. }
            | Self::InvalidTransition { .. }
            | Self::LeaseMismatch => TurnErrorCategory::Conflict,
            Self::CapacityExceeded { .. } => TurnErrorCategory::CapacityExceeded,
            Self::InvalidRunOriginAdapter => TurnErrorCategory::InvalidRequest,
        }
    }

    pub fn capacity_exceeded(resource: TurnCapacityResource, cap: u64) -> Self {
        Self::CapacityExceeded { resource, cap }
    }

    pub fn is_expected_admission_outcome(&self) -> bool {
        matches!(self, Self::ThreadBusy(_) | Self::AdmissionRejected(_))
    }

    pub fn adapter_status_code(&self) -> u16 {
        match self.category() {
            TurnErrorCategory::ThreadBusy | TurnErrorCategory::Conflict => 409,
            TurnErrorCategory::AdmissionRejected => 429,
            TurnErrorCategory::CapacityExceeded => 429,
            TurnErrorCategory::ScopeNotFound => 404,
            TurnErrorCategory::Unauthorized => 403,
            TurnErrorCategory::InvalidRequest => 400,
            TurnErrorCategory::Unavailable => 503,
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
        let gate_ref = GateRef::new("gate:ext-tool").expect("valid gate ref");
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

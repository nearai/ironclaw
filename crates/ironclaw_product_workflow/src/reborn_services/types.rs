use chrono::{DateTime, Utc};
use ironclaw_host_api::ThreadId;
use ironclaw_product_adapters::{ProductOutboundEnvelope, ProjectionCursor};
use ironclaw_threads::{SessionThreadRecord, SummaryArtifact, ThreadMessageRecord};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunResponse, EventCursor, GateRef, ResumeTurnResponse,
    SanitizedFailure, TurnCheckpointId, TurnRunId, TurnRunState, TurnStatus,
};
use serde::{Deserialize, Deserializer, Serialize, de};

use crate::{
    LifecyclePackageRef, LifecyclePhase, LifecycleProductPayload, LifecycleReadinessBlocker,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornConnectableChannelListResponse {
    pub channels: Vec<RebornConnectableChannelInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornConnectableChannelInfo {
    pub channel: String,
    pub display_name: String,
    pub strategy: RebornChannelConnectStrategy,
    pub action: RebornChannelConnectAction,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_aliases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornChannelConnectStrategy {
    InboundProofCode,
    WebGeneratedCode,
    QrCode,
    OAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornChannelConnectAction {
    pub title: String,
    pub instructions: String,
    pub code_placeholder: String,
    pub submit_label: String,
    pub success_message: String,
    pub error_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornCreateThreadResponse {
    pub thread: SessionThreadRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RebornSubmitTurnResponse {
    Submitted {
        thread_id: ThreadId,
        accepted_message_ref: AcceptedMessageRef,
        turn_id: String,
        run_id: TurnRunId,
        status: TurnStatus,
        resolved_run_profile_id: String,
        resolved_run_profile_version: u64,
        event_cursor: EventCursor,
    },
    DeferredBusy {
        thread_id: ThreadId,
        accepted_message_ref: AcceptedMessageRef,
        active_run_id: TurnRunId,
        status: TurnStatus,
        event_cursor: EventCursor,
    },
    AlreadySubmitted {
        thread_id: ThreadId,
        accepted_message_ref: AcceptedMessageRef,
        run_id: TurnRunId,
        status: TurnStatus,
        event_cursor: EventCursor,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornTimelineRequest {
    pub thread_id: String,
    /// Maximum number of messages returned in one response. The facade
    /// clamps to the [`TIMELINE_DEFAULT_PAGE_SIZE`,
    /// `TIMELINE_MAX_PAGE_SIZE`] range so callers cannot bypass the
    /// per-response size bound by asking for an unbounded page. Falls
    /// back to the default when absent.
    ///
    /// [`TIMELINE_DEFAULT_PAGE_SIZE`]: super::TIMELINE_DEFAULT_PAGE_SIZE
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Opaque pagination cursor returned in the previous response's
    /// `next_cursor`. Browsers do not need to interpret the value; the
    /// facade encodes the earliest message sequence the page should
    /// include here and round-trips it on each follow-up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornTimelineResponse {
    pub thread: SessionThreadRecord,
    pub messages: Vec<ThreadMessageRecord>,
    pub summary_artifacts: Vec<SummaryArtifact>,
    /// Opaque cursor to pass back as `cursor` on the follow-up request
    /// to load the older page. `None` means the caller has reached the
    /// start of the thread and there is nothing more to load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornStreamEventsRequest {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_cursor: Option<ProjectionCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornStreamEventsResponse {
    pub events: Vec<ProductOutboundEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornCancelRunResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
    pub already_terminal: bool,
}

impl From<CancelRunResponse> for RebornCancelRunResponse {
    fn from(value: CancelRunResponse) -> Self {
        Self {
            run_id: value.run_id,
            status: value.status,
            event_cursor: value.event_cursor,
            already_terminal: value.already_terminal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornResumeGateResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
}

impl From<ResumeTurnResponse> for RebornResumeGateResponse {
    fn from(value: ResumeTurnResponse) -> Self {
        Self {
            run_id: value.run_id,
            status: value.status,
            event_cursor: value.event_cursor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RebornResolveGateResponse {
    Resumed(RebornResumeGateResponse),
    Cancelled(RebornCancelRunResponse),
}

/// Browser body for the WebUI run-state read.
///
/// Pure read — no idempotency key. Caller authority is supplied separately by
/// `WebUiAuthenticatedCaller` and combined with `thread_id` to produce the
/// canonical [`ironclaw_turns::TurnScope`] inside the facade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornGetRunStateRequest {
    pub thread_id: String,
    pub run_id: String,
}

/// Stable run-state projection returned to WebUI route handlers.
///
/// Deliberately omits M3-internal fields carried on [`TurnRunState`]:
/// `scope`, `source_binding_ref`, `reply_target_binding_ref`, and
/// `resolved_model_route`. Route handlers and downstream M5 consumers must
/// build their views from this surface only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornGetRunStateResponse {
    pub turn_id: String,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
    pub accepted_message_ref: AcceptedMessageRef,
    pub resolved_run_profile_id: String,
    pub resolved_run_profile_version: u64,
    pub received_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<TurnCheckpointId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_ref: Option<GateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<SanitizedFailure>,
}

impl From<TurnRunState> for RebornGetRunStateResponse {
    fn from(value: TurnRunState) -> Self {
        Self {
            turn_id: value.turn_id.to_string(),
            run_id: value.run_id,
            status: value.status,
            event_cursor: value.event_cursor,
            accepted_message_ref: value.accepted_message_ref,
            resolved_run_profile_id: value.resolved_run_profile_id.as_str().to_string(),
            resolved_run_profile_version: value.resolved_run_profile_version.as_u64(),
            received_at: value.received_at,
            checkpoint_id: value.checkpoint_id,
            gate_ref: value.gate_ref,
            failure: value.failure,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornListThreadsResponse {
    pub threads: Vec<SessionThreadRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Bounded browser projection for caller-scoped automations.
///
/// The beta API currently returns one capped page without a cursor. Future
/// pagination can extend this response with an optional cursor without changing
/// the source-tagged automation rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornListAutomationsResponse {
    pub automations: Vec<RebornAutomationInfo>,
}

/// Allowlisted terminal status exposed by automation list projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornAutomationRunStatus {
    Ok,
    Error,
}

/// Allowlisted browser-visible state for automation list projections.
///
/// Unknown runtime states are collapsed to `unknown` so the browser DTO stays
/// typed without surfacing raw backend strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornAutomationState {
    Active,
    Scheduled,
    Paused,
    Disabled,
    Inactive,
    Completed,
    Unknown,
}

impl<'de> Deserialize<'de> for RebornAutomationState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RebornAutomationStateVisitor;

        impl<'de> de::Visitor<'de> for RebornAutomationStateVisitor {
            type Value = RebornAutomationState;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a snake_case automation state string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(match value {
                    "active" => RebornAutomationState::Active,
                    "scheduled" => RebornAutomationState::Scheduled,
                    "paused" => RebornAutomationState::Paused,
                    "disabled" => RebornAutomationState::Disabled,
                    "inactive" => RebornAutomationState::Inactive,
                    "completed" => RebornAutomationState::Completed,
                    "unknown" => RebornAutomationState::Unknown,
                    _ => RebornAutomationState::Unknown,
                })
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_str(RebornAutomationStateVisitor)
    }
}

/// Browser-safe automation row returned by the WebUI facade.
///
/// This deliberately exposes source, state, run timestamps, and sanitized
/// status only; trigger repository internals remain behind the product facade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAutomationInfo {
    pub automation_id: String,
    pub name: String,
    pub source: RebornAutomationSource,
    pub state: RebornAutomationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<RebornAutomationRunStatus>,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

/// Source discriminator for automation rows.
///
/// WebUI v2 exposes only user-facing schedules. The wire tag remains
/// source-discriminated so future sources can be added without overloading the
/// schedule fields or advertising unsupported sources early.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RebornAutomationSource {
    Schedule { cron: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionListResponse {
    pub extensions: Vec<RebornExtensionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionRegistryResponse {
    pub entries: Vec<RebornExtensionRegistryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionRegistryEntry {
    pub package_ref: LifecyclePackageRef,
    pub display_name: String,
    pub kind: String,
    pub description: String,
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionInfo {
    pub package_ref: LifecyclePackageRef,
    pub display_name: String,
    pub kind: String,
    pub description: String,
    pub authenticated: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    pub needs_setup: bool,
    pub has_auth: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding_state: Option<RebornExtensionOnboardingState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding: Option<RebornExtensionOnboardingPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionActionResponse {
    pub success: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awaiting_token: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding_state: Option<RebornExtensionOnboardingState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding: Option<RebornExtensionOnboardingPayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornExtensionOnboardingState {
    AuthRequired,
    SetupRequired,
    Installed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionOnboardingPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_next_step: Option<String>,
}

/// WebUI v2 setup projection for extension lifecycle.
///
/// This intentionally uses the v2 `phase`/`blockers` lifecycle contract and
/// omits the legacy `status` field from the earlier unimplemented route shape.
/// The live browser consumer still uses the v1 setup route, so this v2 contract
/// can become lifecycle-native before it has compatibility consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSetupExtensionResponse {
    pub package_ref: LifecyclePackageRef,
    pub phase: LifecyclePhase,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<LifecycleReadinessBlocker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<LifecycleProductPayload>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<RebornExtensionSetupSecret>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<RebornExtensionSetupField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding: Option<RebornExtensionOnboardingPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionSetupSecret {
    pub name: String,
    pub provider: String,
    pub prompt: String,
    pub optional: bool,
    pub provided: bool,
    pub setup: RebornExtensionCredentialSetup,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RebornExtensionCredentialSetup {
    ManualToken,
    #[serde(rename = "oauth")]
    OAuth {
        account_label: String,
        scopes: Vec<String>,
        invocation_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionSetupField {
    pub name: String,
    pub prompt: String,
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

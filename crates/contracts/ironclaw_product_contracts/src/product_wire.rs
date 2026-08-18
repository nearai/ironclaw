//! The `Reborn*` product wire DTOs — the JSON shapes every product transport
//! serializes across the `ProductSurface` boundary (PROPOSAL §6.1.3, "product
//! wire DTO homes").
//!
//! WebUI, the OpenAI-compatible adapter, and the operator surface all speak
//! these; none of them should compile `ironclaw_assistant` to name a response
//! body. The *inventory* of concrete commands, capabilities, and views that
//! produce them stays in product as its frozen surface — this module is the
//! payload vocabulary only, and holds no service, handler, or projection
//! reducer.
//!
//! Nine of this family's members could not follow it here and stay in
//! `ironclaw_assistant::reborn_services::types`, each because its fields name a
//! crate outside the contracts allowlist (`ironclaw_host_api` +
//! `ironclaw_extension_contracts`): `RebornCreateThreadResponse`,
//! `RebornListThreadsResponse` and `RebornTimelineResponse` carry
//! `ironclaw_threads` records; `RebornAuthAccount`, `RebornVendorAuthAccounts`,
//! `RebornExtensionInfo` and `RebornExtensionListResponse` carry `ironclaw_auth`
//! account state; `RebornGetRunStateResponse` carries
//! `ironclaw_common::llm_costs::RunCost` and
//! `ironclaw_loop_contracts::LoopModelUsage`; and
//! `RebornExecuteProductCommandResponse` carries
//! `ironclaw_assistant::commands::CommandResultView`, the command grammar §6.9.1
//! keeps there.
//!
//! The three `From<ironclaw_turns::…>` conversions stayed too — with both sides
//! outside product they would be orphan impls, so they are free functions there.
// arch-exempt: large_file, one contract surface — splitting by feature area at move time would give the same names two import paths, plan #7008
use chrono::{DateTime, Utc};
use ironclaw_extension_contracts::state::LifecyclePublicState;
use ironclaw_host_api::ids::{ThreadId, UserId};
use ironclaw_host_api::turn::{AcceptedMessageRef, EventCursor, TurnRunId, TurnStatus};
use secrecy::SecretString;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, de};

use crate::outbound::{ProductOutboundEnvelope, ProjectionCursor};
use crate::package_lifecycle::{
    ChannelConnectionRequirement, LifecyclePackageRef, LifecycleProductPayload,
    LifecycleReadinessBlocker,
};

const OUTBOUND_DELIVERY_TARGET_ID_MAX_BYTES: usize = 512;
const OUTBOUND_DELIVERY_CHANNEL_MAX_BYTES: usize = 128;
const OUTBOUND_DELIVERY_DISPLAY_NAME_MAX_BYTES: usize = 256;
const OUTBOUND_DELIVERY_DESCRIPTION_MAX_BYTES: usize = 1024;

/// Readiness verdict for one operator status check, and for the roll-up over
/// all of them.
///
/// The roll-up (`RebornOperatorStatusResponse::overall`) is computed by
/// precedence, not by worst-severity: any `Blocked` check makes the whole
/// response `Blocked`; otherwise any `Degraded` *or* `NotConfigured` check makes
/// it `Degraded`; otherwise `Ready`. `Unsupported` is deliberately excluded from
/// that fold — see its variant note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOperatorStatusState {
    /// The subsystem is wired and validated for its profile.
    Ready,
    /// Usable, but not on a production footing — a dev-only or preview-storage
    /// profile, or a non-blocking readiness diagnostic.
    Degraded,
    /// A blocking readiness diagnostic. Wins the roll-up outright.
    Blocked,
    /// **No probe exists for this subsystem yet** — a statement about this
    /// build's coverage, not about the deployment. Reported today for the
    /// `channels` and `extensions` checks, always at `Info` severity, and
    /// excluded from the `overall` fold so an unwritten probe can never degrade
    /// a healthy host.
    Unsupported,
    /// A required service or worker is not wired, or the runtime profile is
    /// disabled. Distinct from `Degraded` in cause but folded into it in the
    /// roll-up.
    NotConfigured,
}

/// How loudly a status check should be surfaced. Independent of
/// [`RebornOperatorStatusState`]: an `Unsupported` check is `Info`, while a
/// `NotConfigured` one is `Warning`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOperatorStatusSeverity {
    Info,
    Warning,
    Critical,
}

/// One named readiness probe — its verdict, how loudly to surface it, a
/// human-readable summary, and the operator action that would clear it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorStatusCheck {
    /// Stable check identifier (`runtime`, `storage`, `secrets`,
    /// `provider_model`, `webui`, `trigger_poller`, `channels`, `extensions`,
    /// or a readiness diagnostic's own id).
    pub id: String,
    pub status: RebornOperatorStatusState,
    pub severity: RebornOperatorStatusSeverity,
    /// One-line human-readable description of what was observed.
    pub summary: String,
    /// The operator action that would clear this check, when one is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

/// A full operator readiness snapshot: every check plus the precedence roll-up
/// over them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorStatusResponse {
    /// When this snapshot was taken. Status is computed per request, never
    /// cached, so this is the observation time.
    pub generated_at: DateTime<Utc>,
    /// Precedence roll-up over `checks` — see [`RebornOperatorStatusState`].
    pub overall: RebornOperatorStatusState,
    pub checks: Vec<RebornOperatorStatusCheck>,
}

/// Severity of an operator log entry, and the filter a log query narrows by.
/// Serialized lowercase to match the `tracing` level vocabulary the ring
/// captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RebornLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// A query against the operator log ring. Every field is an optional narrowing
/// filter; the default value selects the most recent entries unfiltered.
///
/// Context-valued filters (`thread_id`, `run_id`, `turn_id`, `tool_call_id`,
/// `tool_name`, `source`) must be bounded with
/// [`normalize_operator_log_context_value`] before comparison — the ring
/// normalizes on write, so an un-normalized filter would stop matching the
/// entries it was meant to select.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornLogQueryRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<RebornLogLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub tail: bool,
    #[serde(default)]
    pub follow: bool,
}

impl RebornLogQueryRequest {
    pub fn set_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn set_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    pub fn set_level(mut self, level: RebornLogLevel) -> Self {
        self.level = Some(level);
        self
    }

    pub fn set_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn set_thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    pub fn set_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn set_turn_id(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    pub fn set_tool_call_id(mut self, tool_call_id: impl Into<String>) -> Self {
        self.tool_call_id = Some(tool_call_id.into());
        self
    }

    pub fn set_tool_name(mut self, tool_name: impl Into<String>) -> Self {
        self.tool_name = Some(tool_name.into());
        self
    }

    pub fn set_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn set_tail(mut self, tail: bool) -> Self {
        self.tail = tail;
        self
    }

    pub fn set_follow(mut self, follow: bool) -> Self {
        self.follow = follow;
        self
    }
}

/// One captured operator log line, plus whatever turn-kernel context the
/// `tracing` span carried when it was recorded. Every context field is optional
/// because most log lines are emitted outside a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornLogEntry {
    /// Ring-assigned identifier, also the cursor value for pagination.
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub level: RebornLogLevel,
    /// The `tracing` target the line was emitted under (module path).
    pub target: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// A page of operator log entries, with the capabilities of the backing ring so
/// a client can decide which controls to offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornLogQueryResponse {
    /// Which log source answered (identifies the ring behind this response).
    pub source: String,
    pub entries: Vec<RebornLogEntry>,
    /// Cursor for the next page; absent when this page is the last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Whether this source honours `RebornLogQueryRequest::tail`.
    pub tail_supported: bool,
    /// Whether this source honours `RebornLogQueryRequest::follow`.
    pub follow_supported: bool,
}

/// The OS-service operation an operator requests. `Status` is read-only; the
/// other three mutate the host's service registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornServiceLifecycleAction {
    Install,
    Start,
    Stop,
    Status,
}

/// The service state observed after a lifecycle action.
///
/// Product folds these into surface availability, and the split is not the
/// obvious one: `Installed`, `Running`, `Stopped`, **and `Unknown`** all mean
/// the lifecycle surface is *available*; only `Unsupported` and `Failed` mark it
/// unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornServiceLifecycleState {
    /// The unit or plist was written successfully.
    Installed,
    /// The service manager reports the service active.
    Running,
    /// The service manager reports the service inactive, or a stop succeeded.
    Stopped,
    /// **This OS target has no supported local service manager** — a capability
    /// statement, not a failure. The deployment is expected to be run under an
    /// external process supervisor instead.
    Unsupported,
    /// The operation failed, or the service manager reports the service failed.
    /// Covers an unresolvable home directory or executable path, a command
    /// failure, and a status query that errored or timed out.
    Failed,
    /// The service manager answered, but with a state this build does not map.
    /// Distinct from `Failed`, where the query itself did not succeed — and
    /// treated as *available*, because the surface is working even though the
    /// state string is unrecognized.
    Unknown,
}

/// Request to perform one OS-service lifecycle action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornServiceLifecycleRequest {
    pub action: RebornServiceLifecycleAction,
}

/// Outcome of a lifecycle action: the state observed afterwards, what happened,
/// and the operator action that would clear a bad state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornServiceLifecycleResponse {
    /// Echoes the requested action, so a response is self-describing.
    pub action: RebornServiceLifecycleAction,
    pub state: RebornServiceLifecycleState,
    /// One-line human-readable description of what happened.
    pub message: String,
    /// The operator action that would clear this state, when one is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornChannelConnectAction {
    pub title: String,
    pub instructions: String,
    #[serde(rename = "input_placeholder")]
    pub input_placeholder: String,
    pub submit_label: String,
    pub success_message: String,
    pub error_message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornDeleteThreadRequest {
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornDeleteThreadResponse {
    pub thread_id: ThreadId,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornGlobalAutoApproveRequest {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornGlobalAutoApproveResponse {
    pub enabled: bool,
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
    RejectedBusy {
        thread_id: ThreadId,
        accepted_message_ref: AcceptedMessageRef,
        /// The run that was blocking at the time of rejection.
        ///
        /// `Some` on a fresh `ThreadBusy` rejection (the run is known and
        /// still queryable). `None` on an idempotent replay where the original
        /// blocking run may have already terminated and its id cannot be
        /// recovered from the stored message record.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active_run_id: Option<TurnRunId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<TurnStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_cursor: Option<EventCursor>,
        notice: String,
    },
    DeferredBusy {
        thread_id: ThreadId,
        accepted_message_ref: AcceptedMessageRef,
        active_run_id: TurnRunId,
        status: TurnStatus,
        event_cursor: EventCursor,
        notice: String,
    },
    AlreadySubmitted {
        thread_id: ThreadId,
        accepted_message_ref: AcceptedMessageRef,
        run_id: TurnRunId,
        status: TurnStatus,
        event_cursor: EventCursor,
    },
}

/// One active suggestion projected for any product surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSuggestion {
    pub id: String,
    pub title: String,
    pub description: String,
    pub suggested_prompt: String,
    pub icon: String,
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

/// The durable generation phase projected to product surfaces. The lease
/// owner, storage errors, and model details remain internal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornSuggestionGenerationStatus {
    Empty,
    Generating,
    Ready,
    Failed,
}

/// One caller-scoped suggestion snapshot and its durable generation status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSuggestionsResponse {
    pub status: RebornSuggestionGenerationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u32>,
    pub suggestions: Vec<RebornSuggestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSuggestionStartResponse {
    pub suggestion_id: String,
    pub thread_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSuggestionDismissResponse {
    pub suggestion_id: String,
    pub dismissed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornTimelineRequest {
    pub thread_id: String,
    /// Maximum number of messages returned in one response. The service
    /// clamps it to `[1, TIMELINE_MAX_PAGE_SIZE]` so callers cannot bypass
    /// the per-response size bound by asking for an unbounded page, and
    /// substitutes `TIMELINE_DEFAULT_PAGE_SIZE` when absent. Both bounds are
    /// the service's, not the wire's: they live beside the clamp in
    /// `ironclaw_assistant::reborn_services` and are crate-private there, so
    /// this is deliberately a description rather than a link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Opaque pagination cursor returned in the previous response's
    /// `next_cursor`. Browsers do not need to interpret the value; the
    /// service encodes the earliest message sequence the page should
    /// include here and round-trips it on each follow-up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl RebornTimelineRequest {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            ..Self::default()
        }
    }

    pub fn set_thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = thread_id.into();
        self
    }

    pub fn set_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn set_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }
}

/// Request the raw bytes of one landed attachment, addressed by the thread and
/// message that carry it plus the attachment's per-message id. The triple is
/// required because an attachment id is only unique within its message, not
/// across a thread. The caller's authority comes from the authenticated session
/// (the scope is derived server-side), never from these path values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAttachmentRequest {
    pub thread_id: String,
    pub message_id: String,
    pub attachment_id: String,
}

/// Raw bytes of one landed attachment plus the metadata a browser needs to
/// render or download it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAttachmentBytes {
    pub mime_type: String,
    pub filename: Option<String>,
    pub bytes: Vec<u8>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornResumeGateResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornRetryRunResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
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
/// `ProductSurfaceCaller` and combined with `thread_id` to produce the
/// canonical `ironclaw_turns::TurnScope` inside the service (the kernel
/// type this crate may not name in a link).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornGetRunStateRequest {
    pub thread_id: String,
    pub run_id: String,
}

/// Bounded product projection for caller-scoped automations.
///
/// The beta API currently returns one capped page without a cursor. Future
/// pagination can extend this response with an optional cursor without changing
/// the source-tagged automation rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornListAutomationsResponse {
    pub automations: Vec<RebornAutomationInfo>,
    /// Whether the background trigger poller (scheduler) is running. When
    /// `false`, listed schedule automations will never actually fire, and the
    /// browser surfaces a "scheduling is off" notice. Defaults to `true` on the
    /// wire so an older payload without the field is not misreported as off.
    #[serde(default = "default_scheduler_enabled")]
    pub scheduler_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAutomationMutationResponse {
    pub updated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation: Option<RebornAutomationInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_result: Option<RebornAutomationRunMutationResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAutomationRunMutationResult {
    pub status: RebornAutomationRunMutationStatus,
    pub run_id: TurnRunId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornAutomationRunMutationStatus {
    Submitted,
    Replayed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAutomationRequest {
    pub automation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornRenameAutomationProductRequest {
    pub automation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

fn default_scheduler_enabled() -> bool {
    true
}

/// Product-safe status for a stored outbound delivery target.
///
/// This is channel-neutral: it describes whether a stored target can be
/// resolved through the target authority layer, not how any particular product
/// surface should render that state. `NoneConfigured` is the "nothing stored at
/// all" default; a per-entry status is only `Available` or `Unavailable`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOutboundDeliveryTargetStatus {
    #[default]
    NoneConfigured,
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOutboundDeliveryTargetListResponse {
    pub targets: Vec<RebornOutboundDeliveryTargetOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOutboundDeliveryTargetOption {
    pub target: RebornOutboundDeliveryTargetSummary,
    pub capabilities: RebornOutboundDeliveryTargetCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedRebornOutboundDeliveryTargetSummary")]
pub struct RebornOutboundDeliveryTargetSummary {
    pub target_id: RebornOutboundDeliveryTargetId,
    pub channel: RebornOutboundDeliveryTargetChannel,
    pub display_name: RebornOutboundDeliveryTargetDisplayName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<RebornOutboundDeliveryTargetDescription>,
}

impl RebornOutboundDeliveryTargetSummary {
    pub fn new(
        target_id: RebornOutboundDeliveryTargetId,
        channel: impl Into<String>,
        display_name: impl Into<String>,
        description: Option<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            target_id,
            channel: RebornOutboundDeliveryTargetChannel::new(channel)?,
            display_name: RebornOutboundDeliveryTargetDisplayName::new(display_name)?,
            description: description
                .map(RebornOutboundDeliveryTargetDescription::new)
                .transpose()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct UncheckedRebornOutboundDeliveryTargetSummary {
    target_id: RebornOutboundDeliveryTargetId,
    channel: String,
    display_name: String,
    #[serde(default)]
    description: Option<String>,
}

impl TryFrom<UncheckedRebornOutboundDeliveryTargetSummary> for RebornOutboundDeliveryTargetSummary {
    type Error = String;

    fn try_from(value: UncheckedRebornOutboundDeliveryTargetSummary) -> Result<Self, Self::Error> {
        Self::new(
            value.target_id,
            value.channel,
            value.display_name,
            value.description,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct RebornOutboundDeliveryTargetChannel(String);

impl RebornOutboundDeliveryTargetChannel {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_outbound_delivery_display_field(
            "outbound delivery channel",
            &value,
            OUTBOUND_DELIVERY_CHANNEL_MAX_BYTES,
            true,
        )?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for RebornOutboundDeliveryTargetChannel {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for RebornOutboundDeliveryTargetChannel {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for RebornOutboundDeliveryTargetChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<RebornOutboundDeliveryTargetChannel> for String {
    fn from(value: RebornOutboundDeliveryTargetChannel) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct RebornOutboundDeliveryTargetDisplayName(String);

impl RebornOutboundDeliveryTargetDisplayName {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_outbound_delivery_display_field(
            "outbound delivery display name",
            &value,
            OUTBOUND_DELIVERY_DISPLAY_NAME_MAX_BYTES,
            true,
        )?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for RebornOutboundDeliveryTargetDisplayName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for RebornOutboundDeliveryTargetDisplayName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for RebornOutboundDeliveryTargetDisplayName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<RebornOutboundDeliveryTargetDisplayName> for String {
    fn from(value: RebornOutboundDeliveryTargetDisplayName) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct RebornOutboundDeliveryTargetDescription(String);

impl RebornOutboundDeliveryTargetDescription {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_outbound_delivery_display_field(
            "outbound delivery description",
            &value,
            OUTBOUND_DELIVERY_DESCRIPTION_MAX_BYTES,
            false,
        )?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for RebornOutboundDeliveryTargetDescription {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for RebornOutboundDeliveryTargetDescription {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for RebornOutboundDeliveryTargetDescription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<RebornOutboundDeliveryTargetDescription> for String {
    fn from(value: RebornOutboundDeliveryTargetDescription) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOutboundDeliveryTargetCapabilities {
    pub final_replies: bool,
    pub gate_prompts: bool,
    pub auth_prompts: bool,
    /// This target can receive blocked-automation notifications. Independent of
    /// `final_replies`: the notification-channel picker filters on this, the
    /// model-delivery list filters on `final_replies`.
    #[serde(default)]
    pub notifications: bool,
}

/// Client-safe opaque outbound delivery target id.
///
/// Must be non-empty, at most 512 bytes, and free of leading/trailing
/// whitespace, control characters, and unsafe invisible Unicode formatting
/// characters.
///
/// Composition resolves this id to an adapter-owned reply target before writing
/// outbound preferences.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct RebornOutboundDeliveryTargetId(String);

impl RebornOutboundDeliveryTargetId {
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

    fn validate(value: &str) -> Result<(), String> {
        validate_outbound_delivery_display_field(
            "outbound delivery target id",
            value,
            OUTBOUND_DELIVERY_TARGET_ID_MAX_BYTES,
            true,
        )
    }
}

impl TryFrom<String> for RebornOutboundDeliveryTargetId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for RebornOutboundDeliveryTargetId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for RebornOutboundDeliveryTargetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<RebornOutboundDeliveryTargetId> for String {
    fn from(value: RebornOutboundDeliveryTargetId) -> Self {
        value.0
    }
}

fn validate_outbound_delivery_display_field(
    field_name: &str,
    value: &str,
    max_bytes: usize,
    require_non_empty: bool,
) -> Result<(), String> {
    if require_non_empty && value.trim().is_empty() {
        return Err(format!("{field_name} must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(format!("{field_name} must be at most {max_bytes} bytes"));
    }
    if value.trim() != value {
        return Err(format!(
            "{field_name} must not contain leading or trailing whitespace"
        ));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(format!("{field_name} must not contain control characters"));
    }
    if has_unsafe_unicode_format_character(value) {
        return Err(format!(
            "{field_name} must not contain unsafe Unicode formatting characters"
        ));
    }
    if has_line_or_paragraph_separator(value) {
        return Err(format!(
            "{field_name} must not contain line or paragraph separators"
        ));
    }
    Ok(())
}

fn has_unsafe_unicode_format_character(value: &str) -> bool {
    value.chars().any(|c| {
        matches!(
            c,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{00ad}'
                | '\u{034f}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200d}'
                | '\u{2060}'
                | '\u{feff}'
        )
    })
}

fn has_line_or_paragraph_separator(value: &str) -> bool {
    value.chars().any(|c| matches!(c, '\u{2028}' | '\u{2029}'))
}

/// Full-replace request for the caller's notification-channel target list
/// (spec §7). An empty list means notifications stay in the web app only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSetNotificationChannelsRequest {
    // Deliberately NOT `#[serde(default)]`: this is a full-replace request,
    // and an omitted field must be a 400, never an implicit clear-all. An
    // explicit empty list is the only way to clear the set.
    pub target_ids: Vec<RebornOutboundDeliveryTargetId>,
}

/// One caller notification-channel entry, projected from a stored
/// notification-target id (or the legacy single-slot fallback, spec §7) with
/// its resolution status. `option` carries the full channel details only when
/// `status` is `Available`; a stored id that no longer resolves through the
/// caller-scoped target registry is still represented — as `Unavailable` with
/// `option: None` — rather than silently dropped, so a caller can distinguish
/// "2 channels" from "3, one broken". `status` is never `NoneConfigured` here
/// (that variant describes the "nothing stored at all" state, not a per-entry
/// state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornNotificationChannel {
    pub target_id: RebornOutboundDeliveryTargetId,
    pub status: RebornOutboundDeliveryTargetStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option: Option<RebornOutboundDeliveryTargetOption>,
}

/// Resolved notification-channel list, projected from the caller's stored
/// notification-target ids (or the legacy single-slot fallback, spec §7).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornNotificationChannelsResponse {
    #[serde(default)]
    pub channels: Vec<RebornNotificationChannel>,
}

/// Browser query naming the channel one notification-setup read is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RebornNotificationSetupRequest {
    pub extension_id: String,
}

/// Browser body for the generic notification-setup enable/disable commands.
/// `payload` is a channel-opaque document only the channel's adapter (and
/// its own client) interpret — generic code passes it through verbatim.
/// `extension_id` is defaulted because the ROUTE path is its canonical
/// source: the handler overwrites whatever the body carries, so a body may
/// omit it entirely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RebornNotificationSetupMutationRequest {
    #[serde(default)]
    pub extension_id: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

/// One channel's per-user notification-setup state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornNotificationSetupStatusResponse {
    pub extension_id: String,
    /// Whether this channel needs per-user enrollment at all (from its
    /// manifest declaration). Channels without setup report `enabled: true`.
    pub requires_setup: bool,
    /// Whether notification delivery is enabled for the caller right now.
    pub enabled: bool,
    /// Channel-opaque detail for the channel's own client. Never
    /// interpreted by generic code.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub detail: serde_json::Value,
}

/// Allowlisted terminal status exposed by automation list projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornAutomationRunStatus {
    Ok,
    Error,
}

/// Client-visible status for an individual automation run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornAutomationRecentRunStatus {
    Running,
    Ok,
    Error,
    #[default]
    #[serde(other)]
    Unknown,
}

/// Client-safe automation run projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAutomationRecentRunInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<TurnRunId>,
    /// Canonical thread id for this run, or `None` if no canonical conversation
    /// thread has been established yet (e.g. pre-acceptance or failed runs).
    /// The WebUI panel must not render a chat link when this field is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_slot: Option<DateTime<Utc>>,
    #[serde(default)]
    pub status: RebornAutomationRecentRunStatus,
    pub submitted_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

/// Allowlisted client-visible state for automation list projections.
///
/// Unknown runtime states are collapsed to `unknown` so the client DTO stays
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

/// Browser-safe automation row returned by the WebUI service.
///
/// This deliberately exposes source, state, run timestamps, sanitized status,
/// and bounded recent-run history; trigger repository internals remain behind
/// the product service.
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
    pub recent_runs: Vec<RebornAutomationRecentRunInfo>,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// Present while this automation's active fire is held (gate-parked or
    /// still running) and scheduled fires are being skipped (#5886). Derived
    /// at read time from the active run's state; never persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_hold: Option<RebornAutomationActiveHold>,
}

/// Why an automation's schedule is currently held, plus elapsed-occurrence
/// accounting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAutomationActiveHold {
    pub reason: RebornAutomationHoldReason,
    /// The held fire's claimed slot — when the pause effectively began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,
    /// Scheduled occurrences elapsed while held; display-only, capped. Not a
    /// count of runs the poller attempted — accrues from wall-clock cron
    /// slots regardless of poller activity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_occurrences: Option<u32>,
    /// True when `elapsed_occurrences` hit the cap — render as "N+".
    #[serde(default)]
    pub elapsed_occurrences_capped: bool,
}

/// Client-visible hold reason. `in_progress` = the previous run is still
/// executing; the gate-parked reasons need the user to act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornAutomationHoldReason {
    Approval,
    Auth,
    InProgress,
    Other,
}

/// Source discriminator for automation rows.
///
/// WebUI v2 exposes only user-facing schedules. The wire tag remains
/// source-discriminated so future sources can be added without overloading the
/// schedule fields or advertising unsupported sources early.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RebornAutomationSource {
    Schedule {
        cron: String,
        /// IANA timezone name in which the cron expression is evaluated
        /// (e.g. "America/New_York"). Always "UTC" for legacy rows.
        timezone: String,
    },
    /// A one-time trigger that fires once at `at`, then completes.
    Once {
        /// One-shot fire time as an RFC3339 UTC timestamp.
        at: String,
        /// IANA timezone the one-shot was scheduled in (for display).
        timezone: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSkillListResponse {
    pub skills: Vec<RebornSkillInfo>,
    pub count: usize,
    /// Global default criteria-based skill auto-activation master switch. When
    /// `false`, skills activate only via an explicit `/name` mention. Defaults
    /// to `true` for back-compat with producers that predate the flag.
    #[serde(default = "default_true")]
    pub auto_activate_learned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSkillContentResponse {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSkillSearchResponse {
    #[serde(default)]
    pub catalog: Vec<serde_json::Value>,
    #[serde(default)]
    pub installed: Vec<RebornSkillInfo>,
    #[serde(default)]
    pub registry_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSkillActionResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSkillInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub trust: RebornSkillTrustLevel,
    pub source: RebornSkillSourceKind,
    pub source_kind: RebornSkillSourceKind,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_source_url: Option<String>,
    #[serde(default)]
    pub has_requirements: bool,
    #[serde(default)]
    pub has_scripts: bool,
    #[serde(default)]
    pub can_edit: bool,
    #[serde(default)]
    pub can_delete: bool,
    /// Whether the skill auto-activates on matching requests. `false` means it
    /// only runs when explicitly invoked with `/name`. Defaults to `true`.
    #[serde(default = "default_true")]
    pub auto_activate: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornSkillTrustLevel {
    Trusted,
    Installed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornSkillSourceKind {
    User,
    Installed,
    Workspace,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionRegistryResponse {
    pub entries: Vec<RebornExtensionRegistryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionRegistryEntry {
    pub package_ref: LifecyclePackageRef,
    pub display_name: String,
    /// Runtime implementation name (`wasm` / `mcp` / `first_party` / ...).
    /// Implementation detail — product taxonomy lives in `surfaces`.
    pub runtime: String,
    pub description: String,
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Declared product surfaces (tool / auth / channel-with-direction).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<RebornExtensionSurface>,
}

/// One product-facing surface an installed extension declares, as rendered on
/// the extensions wire. `channel` carries typed direction (inbound = external
/// messages arrive here; outbound = the host delivers final replies /
/// notifications here) plus the caller-scoped connection state and connect
/// affordance when the surface requires an account binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RebornExtensionSurface {
    Tool,
    Auth,
    Channel {
        inbound: bool,
        outbound: bool,
        /// The auth account this channel surface resolves to for its vendor,
        /// when the surface binds a caller-scoped account. `None` until an
        /// account exists. One account per vendor today (ADR 0001 keeps the
        /// list shape); the id points into the `auth_accounts` of
        /// `ironclaw_assistant::reborn_services::types::RebornExtensionInfo`, which
        /// stayed in product because it carries `ironclaw_auth` account state.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved_account_id: Option<String>,
        /// How the resolved account was chosen: the per-(user, vendor) default
        /// or an explicit per-extension binding. Always `Default` today — no
        /// binding behavior ships until the multi-account follow-up.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binding_source: Option<RebornAccountBindingSource>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connection: Option<ChannelConnectionRequirement>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornTraceHoldAuthorizeProductRequest {
    pub submission_id: String,
}

/// How a surface's resolved account was chosen (ADR 0001). Always `Default`
/// today — explicit per-extension bindings ship with the multi-account PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornAccountBindingSource {
    Default,
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
    /// The caller-visible setup phase (§6.1) -- the host checkpoint folded
    /// together with this caller's credential readiness, never the raw
    /// internal checkpoint.
    pub phase: LifecyclePublicState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<LifecycleReadinessBlocker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
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
    /// Channel pairing: the setup card routes to the channel's pairing panel
    /// (host-issued code + deep link), never a token-submit form.
    Pairing,
    /// Device link: the setup card routes to the multi-step device-link panel,
    /// never a token-submit form. There is no secret for the user to paste —
    /// the vendor issues the payload and the host takes custody of the
    /// resulting session — so a card that fell back to the manual-token form
    /// here would ask for a value that does not exist.
    DeviceLink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionSetupField {
    pub name: String,
    pub prompt: String,
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOperatorArea {
    Setup,
    Config,
    Diagnostics,
    Logs,
    Status,
    ServiceLifecycle,
}

impl RebornOperatorArea {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Config => "config",
            Self::Diagnostics => "diagnostics",
            Self::Logs => "logs",
            Self::Status => "status",
            Self::ServiceLifecycle => "service_lifecycle",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorCommandPlaneResponse {
    pub area: RebornOperatorArea,
    pub status: RebornOperatorSurfaceStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_status: Option<RebornOperatorStatusResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs: Option<RebornLogQueryResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_lifecycle: Option<RebornServiceLifecycleResponse>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<RebornOperatorConfigDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOperatorSurfaceStatus {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RebornOperatorSetupRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub webui_access_token: Option<SecretString>,
}

impl RebornOperatorSetupRequest {
    pub fn set_provider_id(mut self, provider_id: impl Into<String>) -> Self {
        self.provider_id = Some(provider_id.into());
        self
    }

    pub fn set_adapter(mut self, adapter: impl Into<String>) -> Self {
        self.adapter = Some(adapter.into());
        self
    }

    pub fn set_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    pub fn set_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn set_api_key(mut self, api_key: SecretString) -> Self {
        self.api_key = Some(api_key);
        self
    }

    pub fn set_profile_id(mut self, profile_id: impl Into<String>) -> Self {
        self.profile_id = Some(profile_id.into());
        self
    }

    pub fn set_webui_access_token(mut self, webui_access_token: SecretString) -> Self {
        self.webui_access_token = Some(webui_access_token);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorSetupResponse {
    pub area: RebornOperatorArea,
    pub status: RebornOperatorSetupStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<RebornOperatorSetupStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<RebornOperatorConfigDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOperatorSetupStatus {
    Complete,
    Incomplete,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorSetupStep {
    pub name: String,
    pub status: RebornOperatorSetupStepStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOperatorSetupStepStatus {
    Complete,
    Required,
    Unsupported,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorConfigValidateRequest {
    #[serde(default)]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorLogsQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub level: Option<RebornLogLevel>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub tail: bool,
    #[serde(default)]
    pub follow: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorServiceLifecycleRequest {
    pub action: RebornOperatorServiceLifecycleAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOperatorServiceLifecycleAction {
    Install,
    Start,
    Stop,
    Status,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornOperatorConfigListResponse {
    pub entries: Vec<RebornOperatorConfigEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub precedence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<RebornOperatorConfigDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornOperatorConfigGetResponse {
    pub entry: RebornOperatorConfigEntry,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RebornOperatorConfigEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub source: String,
    pub redacted: bool,
    pub mutable: bool,
}

impl Serialize for RebornOperatorConfigEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("RebornOperatorConfigEntry", 5)?;
        state.serialize_field("key", &self.key)?;
        if self.redacted {
            state.serialize_field("value", &serde_json::Value::Null)?;
        } else {
            state.serialize_field("value", &self.value)?;
        }
        state.serialize_field("source", &self.source)?;
        state.serialize_field("redacted", &self.redacted)?;
        state.serialize_field("mutable", &self.mutable)?;
        state.end()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornOperatorConfigSetRequest {
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornOperatorConfigSetProductRequest {
    pub key: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorConfigValidateResponse {
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<RebornOperatorConfigDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorConfigDiagnostic {
    pub key: String,
    pub severity: RebornOperatorConfigDiagnosticSeverity,
    pub reason_code: String,
    pub message: String,
    pub owning_area: RebornOperatorArea,
    pub remediation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOperatorConfigDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// One command entry in the caller's audience-filtered `product.commands.list`
/// response. Presentation metadata only (Task 1's descriptor fields) — the
/// caller's audience has already been applied by the service, so no
/// `audience` field is carried here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornProductCommandInfo {
    pub name: String,
    pub title: String,
    pub description: String,
    pub usage: String,
}

/// Response for `product.commands.list`: the registry, filtered to the
/// commands the caller's audience may see, in registry order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornProductCommandListResponse {
    pub commands: Vec<RebornProductCommandInfo>,
}

/// Request for `product.commands.execute`: a raw slash-command line plus the
/// bound thread it should be attributed to (needed by `/status`, harmless for
/// commands that ignore it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExecuteProductCommandRequest {
    pub thread_id: String,
    pub text: String,
}

/// Sanitized, role-filtered rejection for a `product.commands.execute` call.
/// `message` is always safe to render — it is either a fixed copy string or
/// audience-filtered help text; the underlying `ProductRejection`'s internal
/// `reason` never crosses this boundary (leak rule, matching the channel
/// observer's `InvalidRequest` -> help-text behavior).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornCommandRejection {
    pub kind: crate::inbound::ProductRejectionKind,
    pub message: String,
}

// --- Trace Commons contributor projections ---------------------------------

/// Read-only Trace Commons credit summary scoped to one user.
///
/// All aggregates are the contributor-local view as of the last credit
/// sync (see `TRACE_CREDITS_NOTE`, the server-authoritative wording in
/// `ironclaw_assistant::reborn_services::trace_credits`). A user with no local Trace
/// Commons state gets the unenrolled zero-state, never an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornTraceCreditsResponse {
    /// Whether the caller's standing trace-contribution policy is enabled.
    pub enrolled: bool,
    pub pending_credit: f32,
    pub final_credit: f32,
    pub delayed_credit_delta: f32,
    pub submissions_total: u32,
    pub submissions_submitted: u32,
    pub submissions_accepted: u32,
    pub submissions_revoked: u32,
    pub submissions_expired: u32,
    pub credit_events_total: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_submission_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_credit_sync_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_explanations: Vec<String>,
    /// Count of traces held awaiting the caller's manual-review authorization
    /// (e.g. High residual-PII-risk). These are retained, not submitted.
    #[serde(default)]
    pub manual_review_hold_count: u32,
    /// The held traces awaiting authorization. Sanitized: submission id and a
    /// safe hold reason only — never raw trace content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<RebornTraceHold>,
    /// Server-authoritative framing — always `TRACE_CREDITS_NOTE`, defined
    /// with the builder in `ironclaw_assistant::reborn_services::trace_credits`.
    pub note: String,
}

/// One trace held awaiting the caller's manual-review authorization. Carries
/// only the submission id (to authorize against) and a sanitized hold reason;
/// no raw trace payload is ever exposed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornTraceHold {
    pub submission_id: String,
    pub reason: String,
}

/// One submitted trace record as returned by the Trace Commons server.
/// Carries only the fields the UI needs; unknown server fields are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebornAccountTrace {
    pub submission_id: String,
    pub status: String,
    pub pending_credit: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_credit: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_at: Option<String>,
}

/// Read-only list of the caller's submitted Trace Commons traces.
///
/// `enrolled` mirrors the caller's contribution-policy enrollment status
/// (same semantics as [`RebornTraceCreditsResponse::enrolled`]).
/// `traces` is the server-returned list in reverse-chronological order;
/// an empty list is normal for an enrolled user who has not yet submitted
/// any traces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebornAccountTracesResponse {
    pub enrolled: bool,
    pub traces: Vec<RebornAccountTrace>,
}

/// Result of authorizing a held trace for submission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornTraceHoldAuthorizeResponse {
    /// True when a held trace matching the submission id was found and
    /// authorized for submission; false when there was no such held trace
    /// (already authorized, already submitted, or never held).
    pub authorized: bool,
}

/// One-time Trace Commons browser login link, minted for the authenticated
/// caller. SECURITY: the `url` is a code-bearing account-access credential.
/// It is delivered ONLY over the authenticated WebUI response to the caller's
/// own browser — it must never be logged, persisted, or placed on any
/// model-visible surface.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct RebornAccountLoginLinkResponse {
    /// Whether a link was minted. `false` with `enrolled: false` is the
    /// unenrolled zero-state, not an error.
    pub minted: bool,
    pub enrolled: bool,
    /// The one-time login URL (present iff `minted`). Expires shortly and is
    /// single-use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Reports whether a URL is present, never the URL. The doc above promises the
/// code-bearing link "must never be logged"; a derived `Debug` is exactly how
/// that promise breaks, so the type enforces it instead of asking callers to.
impl std::fmt::Debug for RebornAccountLoginLinkResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornAccountLoginLinkResponse")
            .field("minted", &self.minted)
            .field("enrolled", &self.enrolled)
            .field("url", &self.url.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

// --- Artifact export requests ------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornRunArtifactRequest {
    pub thread_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornThreadArtifactRequest {
    pub thread_id: String,
}

/// Admin-authorized, read-only thread collection request for one tenant user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAdminThreadScrapeListRequest {
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Admin-authorized request for an existing full-thread artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAdminThreadScrapeArtifactRequest {
    pub user_id: UserId,
    pub thread_id: String,
}

/// Admin-authorized request for an existing single-run artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAdminThreadScrapeRunArtifactRequest {
    pub user_id: UserId,
    pub thread_id: String,
    pub run_id: String,
}

// --- Operator settings vocabulary --------------------------------------------

/// Requested per-capability tool-permission state on the settings surface: the
/// three resolved permission values plus `default`, which clears the stored
/// per-capability override. The serialized strings must stay byte-identical to
/// what product's `parse_tool_permission_state` accepts and
/// `tool_permission_state_wire` emits — the
/// `settings_tool_permission_state_wire_strings_stay_linked` test there pins
/// that link so the request enum cannot drift from the storage vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsToolPermissionState {
    Default,
    AlwaysAllow,
    AskEachTime,
    Disabled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The outbound-delivery display newtypes are the only fence between an
    /// operator-supplied target label and the browser that renders it, and the
    /// validator behind them moved into this crate with the DTOs — it has no
    /// caller-side test anywhere else in the workspace. Each rejection below is
    /// a distinct attack or corruption shape, driven through the public
    /// constructor rather than the private validator.
    #[test]
    fn outbound_delivery_display_newtypes_reject_every_unsafe_shape() {
        assert!(RebornOutboundDeliveryTargetDisplayName::new("Ops room").is_ok());

        for (label, candidate) in [
            ("blank", "   ".to_string()),
            ("leading whitespace", " Ops".to_string()),
            ("trailing whitespace", "Ops ".to_string()),
            ("control character", "Ops\u{0007}".to_string()),
            // Bidi override: renders the label backwards in the picker and can
            // disguise which target a message is about to go to.
            ("bidi override", "Ops\u{202E}room".to_string()),
            // Zero-width joiner: two distinct targets render identically.
            ("zero-width joiner", "Ops\u{200D}room".to_string()),
            ("line separator", "Ops\u{2028}room".to_string()),
            ("paragraph separator", "Ops\u{2029}room".to_string()),
            ("over the byte cap", "x".repeat(257)),
        ] {
            let rejected = RebornOutboundDeliveryTargetDisplayName::new(candidate.clone());
            assert!(
                rejected.is_err(),
                "{label} must be rejected, got {rejected:?}"
            );
            // The same fence guards the deserialization path a browser body
            // takes, not only the constructor a service calls.
            assert!(
                RebornOutboundDeliveryTargetDisplayName::try_from(candidate).is_err(),
                "{label} must be rejected through TryFrom as well"
            );
        }

        // The caps differ per field; a copy-paste of the wrong constant would
        // silently widen one of them.
        assert!(RebornOutboundDeliveryTargetChannel::new("c".repeat(128)).is_ok());
        assert!(RebornOutboundDeliveryTargetChannel::new("c".repeat(129)).is_err());
        assert!(RebornOutboundDeliveryTargetId::new("i".repeat(512)).is_ok());
        assert!(RebornOutboundDeliveryTargetId::new("i".repeat(513)).is_err());
        assert!(RebornOutboundDeliveryTargetDescription::new("d".repeat(1024)).is_ok());
        assert!(RebornOutboundDeliveryTargetDescription::new("d".repeat(1025)).is_err());
        // Description is the one optional field: empty is a legitimate value.
        assert!(RebornOutboundDeliveryTargetDescription::new("").is_ok());
    }

    /// `RebornAutomationState` hand-writes `Deserialize` so an unrecognized
    /// state degrades to `Unknown` instead of failing the whole response. A
    /// derived impl would reject the payload, and one newer state on the server
    /// would blank the browser's entire automations list rather than one row.
    #[test]
    fn automation_state_degrades_an_unknown_wire_value_instead_of_failing_the_page() {
        for (wire, expected) in [
            ("active", RebornAutomationState::Active),
            ("scheduled", RebornAutomationState::Scheduled),
            ("paused", RebornAutomationState::Paused),
            ("disabled", RebornAutomationState::Disabled),
            ("inactive", RebornAutomationState::Inactive),
            ("completed", RebornAutomationState::Completed),
            ("unknown", RebornAutomationState::Unknown),
        ] {
            let parsed: RebornAutomationState =
                serde_json::from_value(serde_json::json!(wire)).expect("known state parses");
            assert_eq!(parsed, expected, "{wire} round-trips");
            assert_eq!(
                serde_json::to_value(expected).expect("serialize"),
                serde_json::json!(wire),
                "{wire} must serialize back to the same token"
            );
        }

        let future_state: RebornAutomationState =
            serde_json::from_value(serde_json::json!("some_state_from_a_newer_server"))
                .expect("an unknown state must not fail the page");
        assert_eq!(future_state, RebornAutomationState::Unknown);

        serde_json::from_value::<RebornAutomationState>(serde_json::json!(7))
            .expect_err("a non-string is a malformed payload, not a future state");
    }

    #[test]
    fn operator_config_entry_masks_redacted_value_when_serialized() {
        let entry = RebornOperatorConfigEntry {
            key: "secret.api_key".to_string(),
            value: json!("should-not-leak"),
            source: "secret".to_string(),
            redacted: true,
            mutable: true,
        };

        let serialized = serde_json::to_value(entry).expect("serialize entry");
        assert_eq!(serialized.get("value"), Some(&serde_json::Value::Null));
        assert_eq!(
            serialized
                .get("redacted")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    /// The login link is a code-bearing account-access credential whose own doc
    /// comment says it "must never be logged". A derived `Debug` is precisely
    /// how that promise breaks. Two-sided: the presence flag must still be
    /// legible (the unenrolled zero-state is diagnosable from `minted`/
    /// `enrolled` alone), and the absent case must not render a phantom secret.
    #[test]
    fn account_login_link_debug_reports_presence_and_never_the_url() {
        let minted = RebornAccountLoginLinkResponse {
            minted: true,
            enrolled: true,
            url: Some("https://traces.example/login?code=SUPERSECRET".to_string()),
        };
        let rendered = format!("{minted:?}");
        assert!(
            !rendered.contains("SUPERSECRET") && !rendered.contains("traces.example"),
            "the one-time login URL must never reach a diagnostic: {rendered}"
        );
        assert!(
            rendered.contains("minted: true") && rendered.contains("enrolled: true"),
            "the presence flags stay legible: {rendered}"
        );

        let absent = RebornAccountLoginLinkResponse {
            minted: false,
            enrolled: false,
            url: None,
        };
        let rendered = format!("{absent:?}");
        assert!(
            rendered.contains("url: None"),
            "an absent link renders as absent, not as a redacted one: {rendered}"
        );
    }
}

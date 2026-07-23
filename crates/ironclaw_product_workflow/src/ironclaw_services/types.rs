// arch-exempt: large_file, WebUI facade DTOs live with their contract awaiting the ProductSurface domain-port split, plan #5985
use chrono::{DateTime, Utc};
use ironclaw_auth::{AuthAccountLastError, AuthAccountState};
use ironclaw_common::llm_costs::RunCost;
use ironclaw_host_api::{InstallationState, ThreadId};
use ironclaw_product_adapters::{ProductOutboundEnvelope, ProjectionCursor};
use ironclaw_threads::{SessionThreadRecord, SummaryArtifact, ThreadMessageRecord};
use ironclaw_turns::run_profile::LoopModelUsage;
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunResponse, EventCursor, GateRef, ResumeTurnResponse,
    RetryTurnResponse, SanitizedFailure, TurnCheckpointId, TurnRunId, TurnRunState, TurnStatus,
};
use secrecy::SecretString;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, de};
use tokio::sync::mpsc;

use crate::{
    ChannelConnectionRequirement, LifecycleInstallScope, LifecyclePackageRef,
    LifecycleProductPayload, LifecycleReadinessBlocker,
};

const OUTBOUND_DELIVERY_TARGET_ID_MAX_BYTES: usize = 512;
const OUTBOUND_DELIVERY_CHANNEL_MAX_BYTES: usize = 128;
const OUTBOUND_DELIVERY_DISPLAY_NAME_MAX_BYTES: usize = 256;
const OUTBOUND_DELIVERY_DESCRIPTION_MAX_BYTES: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawOperatorStatusState {
    Ready,
    Degraded,
    Blocked,
    Unsupported,
    NotConfigured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawOperatorStatusSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawOperatorStatusCheck {
    pub id: String,
    pub status: IronClawOperatorStatusState,
    pub severity: IronClawOperatorStatusSeverity,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawOperatorStatusResponse {
    pub generated_at: DateTime<Utc>,
    pub overall: IronClawOperatorStatusState,
    pub checks: Vec<IronClawOperatorStatusCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IronClawLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawLogQueryRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<IronClawLogLevel>,
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

impl IronClawLogQueryRequest {
    pub fn set_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn set_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    pub fn set_level(mut self, level: IronClawLogLevel) -> Self {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawLogEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub level: IronClawLogLevel,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawLogQueryResponse {
    pub source: String,
    pub entries: Vec<IronClawLogEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub tail_supported: bool,
    pub follow_supported: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawServiceLifecycleAction {
    Install,
    Start,
    Stop,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawServiceLifecycleState {
    Installed,
    Running,
    Stopped,
    Unsupported,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawServiceLifecycleRequest {
    pub action: IronClawServiceLifecycleAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawServiceLifecycleResponse {
    pub action: IronClawServiceLifecycleAction,
    pub state: IronClawServiceLifecycleState,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawChannelConnectStrategy {
    InboundProofCode,
    AdminManagedChannels,
    WebGeneratedCode,
    QrCode,
    #[serde(rename = "oauth")]
    OAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawChannelConnectAction {
    pub title: String,
    pub instructions: String,
    #[serde(rename = "input_placeholder")]
    pub input_placeholder: String,
    pub submit_label: String,
    pub success_message: String,
    pub error_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawCreateThreadResponse {
    pub thread: SessionThreadRecord,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawDeleteThreadRequest {
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawDeleteThreadResponse {
    pub thread_id: ThreadId,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawGlobalAutoApproveRequest {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawGlobalAutoApproveResponse {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum IronClawSubmitTurnResponse {
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
    AlreadySubmitted {
        thread_id: ThreadId,
        accepted_message_ref: AcceptedMessageRef,
        run_id: TurnRunId,
        status: TurnStatus,
        event_cursor: EventCursor,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawTimelineRequest {
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

impl IronClawTimelineRequest {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawTimelineResponse {
    pub thread: SessionThreadRecord,
    pub messages: Vec<ThreadMessageRecord>,
    pub summary_artifacts: Vec<SummaryArtifact>,
    /// Opaque cursor to pass back as `cursor` on the follow-up request
    /// to load the older page. `None` means the caller has reached the
    /// start of the thread and there is nothing more to load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Request the raw bytes of one landed attachment, addressed by the thread and
/// message that carry it plus the attachment's per-message id. The triple is
/// required because an attachment id is only unique within its message, not
/// across a thread. The caller's authority comes from the authenticated session
/// (the scope is derived server-side), never from these path values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawAttachmentRequest {
    pub thread_id: String,
    pub message_id: String,
    pub attachment_id: String,
}

/// Raw bytes of one landed attachment plus the metadata a browser needs to
/// render or download it. Returned by [`super::ATTACHMENT_READ_OPERATION`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronClawAttachmentBytes {
    pub mime_type: String,
    pub filename: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawStreamEventsRequest {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_cursor: Option<ProjectionCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawStreamEventsResponse {
    pub events: Vec<ProductOutboundEnvelope>,
}

pub struct IronClawStreamEventsSubscription {
    receiver: mpsc::Receiver<Result<ProductOutboundEnvelope, super::IronClawServicesError>>,
}

impl IronClawStreamEventsSubscription {
    pub fn new(
        receiver: mpsc::Receiver<Result<ProductOutboundEnvelope, super::IronClawServicesError>>,
    ) -> Self {
        Self { receiver }
    }

    pub async fn next(
        &mut self,
    ) -> Option<Result<ProductOutboundEnvelope, super::IronClawServicesError>> {
        self.receiver.recv().await
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawCancelRunResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
    pub already_terminal: bool,
}

impl From<CancelRunResponse> for IronClawCancelRunResponse {
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
pub struct IronClawResumeGateResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
}

impl From<ResumeTurnResponse> for IronClawResumeGateResponse {
    fn from(value: ResumeTurnResponse) -> Self {
        Self {
            run_id: value.run_id,
            status: value.status,
            event_cursor: value.event_cursor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawRetryRunResponse {
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
}

impl From<RetryTurnResponse> for IronClawRetryRunResponse {
    fn from(value: RetryTurnResponse) -> Self {
        Self {
            run_id: value.run_id,
            status: value.status,
            event_cursor: value.event_cursor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum IronClawResolveGateResponse {
    Resumed(IronClawResumeGateResponse),
    Cancelled(IronClawCancelRunResponse),
}

/// Browser body for the WebUI run-state read.
///
/// Pure read — no idempotency key. Caller authority is supplied separately by
/// `WebUiAuthenticatedCaller` and combined with `thread_id` to produce the
/// canonical [`ironclaw_turns::TurnScope`] inside the facade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawGetRunStateRequest {
    pub thread_id: String,
    pub run_id: String,
}

/// Stable run-state projection returned to WebUI route handlers.
///
/// Deliberately omits M3-internal fields carried on [`TurnRunState`]:
/// `scope`, `source_binding_ref`, `reply_target_binding_ref`, and
/// `resolved_model_route`. Route handlers and downstream M5 consumers must
/// build their views from this surface only. Per-run token `usage` and USD
/// `cost` are surfaced (mirroring the OpenAI-compatible API); the resolved
/// model route stays internal — only its model id feeds cost pricing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawGetRunStateResponse {
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
    /// Cumulative token usage for the run, once the model has reported it.
    /// `None` until the first model response lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<LoopModelUsage>,
    /// USD cost priced from `usage` for the run's resolved model. `None` when
    /// usage is absent or no concrete model was resolved (a default-model run
    /// reports usage without cost until the active model is surfaced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<RunCost>,
}

impl IronClawGetRunStateResponse {
    /// Build the WebUI run-state projection from canonical run state, pricing
    /// USD `cost` from the run's `model_usage`.
    ///
    /// The model that feeds the cost table is chosen in this order:
    /// 1. the run's `resolved_model_route` model id, when the caller explicitly
    ///    selected/routed a model (only its id crosses; the route stays
    ///    internal);
    /// 2. otherwise `active_model_id` — the runtime's live default model, which
    ///    for a default (unrouted) run is the model that actually ran.
    ///
    /// `cost` stays `None` only when no usage was reported or neither source
    /// yields a concrete model id (the `"default"` alias and empty strings are
    /// treated as non-concrete so a run is never mispriced against a sentinel).
    pub fn from_run_state(value: TurnRunState, active_model_id: Option<&str>) -> Self {
        let priced_model = value
            .resolved_model_route
            .as_ref()
            .map(|route| route.model_id())
            .or(active_model_id)
            .map(str::trim)
            .filter(|model| !model.is_empty() && !model.eq_ignore_ascii_case("default"));
        let cost = match (value.model_usage, priced_model) {
            (Some(usage), Some(model_id)) => Some(RunCost::from_usage(
                model_id,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_input_tokens,
                usage.cache_creation_input_tokens,
            )),
            _ => None,
        };
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
            // Public WebUI shape: strip the model-visible `detail` so free-form
            // backend cause text never reaches the browser. `category` (the
            // user-facing signal) is retained. See
            // `SanitizedFailure::public_projection`.
            failure: value
                .failure
                .as_ref()
                .map(SanitizedFailure::public_projection),
            usage: value.model_usage,
            cost,
        }
    }
}

impl From<TurnRunState> for IronClawGetRunStateResponse {
    /// Convenience conversion with no default-model fallback: a default-model
    /// run (no `resolved_model_route`) reports usage without cost. Callers that
    /// can supply the live active model — the facade's `get_run_state` — use
    /// [`IronClawGetRunStateResponse::from_run_state`] so those runs are priced.
    fn from(value: TurnRunState) -> Self {
        Self::from_run_state(value, None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawListThreadsResponse {
    pub threads: Vec<SessionThreadRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Bounded product projection for caller-scoped automations.
///
/// The beta API currently returns one capped page without a cursor. Future
/// pagination can extend this response with an optional cursor without changing
/// the source-tagged automation rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawListAutomationsResponse {
    pub automations: Vec<IronClawAutomationInfo>,
    /// Whether the background trigger poller (scheduler) is running. When
    /// `false`, listed schedule automations will never actually fire, and the
    /// browser surfaces a "scheduling is off" notice. Defaults to `true` on the
    /// wire so an older payload without the field is not misreported as off.
    #[serde(default = "default_scheduler_enabled")]
    pub scheduler_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawAutomationMutationResponse {
    pub updated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation: Option<IronClawAutomationInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawAutomationRequest {
    pub automation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawRenameAutomationProductRequest {
    pub automation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

fn default_scheduler_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "IronClawOutboundPreferencesResponseWire")]
pub struct IronClawOutboundPreferencesResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_reply_target: Option<IronClawOutboundDeliveryTargetSummary>,
    #[serde(default)]
    pub final_reply_target_status: IronClawOutboundDeliveryTargetStatus,
    #[serde(default)]
    pub default_modality: IronClawOutboundDeliveryModality,
}

impl Default for IronClawOutboundPreferencesResponse {
    fn default() -> Self {
        Self {
            final_reply_target: None,
            final_reply_target_status: IronClawOutboundDeliveryTargetStatus::NoneConfigured,
            default_modality: IronClawOutboundDeliveryModality::Text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct IronClawOutboundPreferencesResponseWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    final_reply_target: Option<IronClawOutboundDeliveryTargetSummary>,
    #[serde(default)]
    final_reply_target_status: Option<IronClawOutboundDeliveryTargetStatus>,
    #[serde(default)]
    default_modality: Option<IronClawOutboundDeliveryModality>,
}

impl From<IronClawOutboundPreferencesResponseWire> for IronClawOutboundPreferencesResponse {
    fn from(value: IronClawOutboundPreferencesResponseWire) -> Self {
        let final_reply_target_status = match (
            value.final_reply_target.as_ref(),
            value.final_reply_target_status,
        ) {
            (Some(_), None) => IronClawOutboundDeliveryTargetStatus::Available,
            (_, Some(status)) => status,
            (None, None) => IronClawOutboundDeliveryTargetStatus::NoneConfigured,
        };

        Self {
            final_reply_target: value.final_reply_target,
            final_reply_target_status,
            default_modality: value.default_modality.unwrap_or_default(),
        }
    }
}

/// Product-safe status for a saved outbound delivery target.
///
/// This is channel-neutral: it describes whether the configured default can be
/// resolved through the target authority layer, not how any particular product
/// surface should render that state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawOutboundDeliveryTargetStatus {
    #[default]
    NoneConfigured,
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawOutboundDeliveryTargetListResponse {
    pub targets: Vec<IronClawOutboundDeliveryTargetOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawOutboundDeliveryTargetOption {
    pub target: IronClawOutboundDeliveryTargetSummary,
    pub capabilities: IronClawOutboundDeliveryTargetCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedIronClawOutboundDeliveryTargetSummary")]
pub struct IronClawOutboundDeliveryTargetSummary {
    pub target_id: IronClawOutboundDeliveryTargetId,
    pub channel: IronClawOutboundDeliveryTargetChannel,
    pub display_name: IronClawOutboundDeliveryTargetDisplayName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<IronClawOutboundDeliveryTargetDescription>,
}

impl IronClawOutboundDeliveryTargetSummary {
    pub fn new(
        target_id: IronClawOutboundDeliveryTargetId,
        channel: impl Into<String>,
        display_name: impl Into<String>,
        description: Option<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            target_id,
            channel: IronClawOutboundDeliveryTargetChannel::new(channel)?,
            display_name: IronClawOutboundDeliveryTargetDisplayName::new(display_name)?,
            description: description
                .map(IronClawOutboundDeliveryTargetDescription::new)
                .transpose()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct UncheckedIronClawOutboundDeliveryTargetSummary {
    target_id: IronClawOutboundDeliveryTargetId,
    channel: String,
    display_name: String,
    #[serde(default)]
    description: Option<String>,
}

impl TryFrom<UncheckedIronClawOutboundDeliveryTargetSummary>
    for IronClawOutboundDeliveryTargetSummary
{
    type Error = String;

    fn try_from(
        value: UncheckedIronClawOutboundDeliveryTargetSummary,
    ) -> Result<Self, Self::Error> {
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
pub struct IronClawOutboundDeliveryTargetChannel(String);

impl IronClawOutboundDeliveryTargetChannel {
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

impl TryFrom<String> for IronClawOutboundDeliveryTargetChannel {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for IronClawOutboundDeliveryTargetChannel {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for IronClawOutboundDeliveryTargetChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<IronClawOutboundDeliveryTargetChannel> for String {
    fn from(value: IronClawOutboundDeliveryTargetChannel) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct IronClawOutboundDeliveryTargetDisplayName(String);

impl IronClawOutboundDeliveryTargetDisplayName {
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

impl TryFrom<String> for IronClawOutboundDeliveryTargetDisplayName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for IronClawOutboundDeliveryTargetDisplayName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for IronClawOutboundDeliveryTargetDisplayName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<IronClawOutboundDeliveryTargetDisplayName> for String {
    fn from(value: IronClawOutboundDeliveryTargetDisplayName) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct IronClawOutboundDeliveryTargetDescription(String);

impl IronClawOutboundDeliveryTargetDescription {
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

impl TryFrom<String> for IronClawOutboundDeliveryTargetDescription {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for IronClawOutboundDeliveryTargetDescription {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for IronClawOutboundDeliveryTargetDescription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<IronClawOutboundDeliveryTargetDescription> for String {
    fn from(value: IronClawOutboundDeliveryTargetDescription) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawOutboundDeliveryTargetCapabilities {
    pub final_replies: bool,
    pub gate_prompts: bool,
    pub auth_prompts: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawOutboundDeliveryModality {
    #[default]
    Text,
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
pub struct IronClawOutboundDeliveryTargetId(String);

impl IronClawOutboundDeliveryTargetId {
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

impl TryFrom<String> for IronClawOutboundDeliveryTargetId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for IronClawOutboundDeliveryTargetId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for IronClawOutboundDeliveryTargetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<IronClawOutboundDeliveryTargetId> for String {
    fn from(value: IronClawOutboundDeliveryTargetId) -> Self {
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawSetOutboundPreferencesRequest {
    /// `Some(id)` sets the final-reply target; `None` clears it.
    ///
    /// The field defaults to `None` when omitted, so clients that want to leave
    /// an existing value unchanged must use the read endpoint instead of
    /// submitting a partial update without this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_reply_target_id: Option<IronClawOutboundDeliveryTargetId>,
}

/// Allowlisted terminal status exposed by automation list projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawAutomationRunStatus {
    Ok,
    Error,
}

/// Client-visible status for an individual automation run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawAutomationRecentRunStatus {
    Running,
    Ok,
    Error,
    #[default]
    #[serde(other)]
    Unknown,
}

/// Client-safe automation run projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawAutomationRecentRunInfo {
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
    pub status: IronClawAutomationRecentRunStatus,
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
pub enum IronClawAutomationState {
    Active,
    Scheduled,
    Paused,
    Disabled,
    Inactive,
    Completed,
    Unknown,
}

impl<'de> Deserialize<'de> for IronClawAutomationState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct IronClawAutomationStateVisitor;

        impl<'de> de::Visitor<'de> for IronClawAutomationStateVisitor {
            type Value = IronClawAutomationState;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a snake_case automation state string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(match value {
                    "active" => IronClawAutomationState::Active,
                    "scheduled" => IronClawAutomationState::Scheduled,
                    "paused" => IronClawAutomationState::Paused,
                    "disabled" => IronClawAutomationState::Disabled,
                    "inactive" => IronClawAutomationState::Inactive,
                    "completed" => IronClawAutomationState::Completed,
                    "unknown" => IronClawAutomationState::Unknown,
                    _ => IronClawAutomationState::Unknown,
                })
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_str(IronClawAutomationStateVisitor)
    }
}

/// Browser-safe automation row returned by the WebUI facade.
///
/// This deliberately exposes source, state, run timestamps, sanitized status,
/// and bounded recent-run history; trigger repository internals remain behind
/// the product facade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawAutomationInfo {
    pub automation_id: String,
    pub name: String,
    pub source: IronClawAutomationSource,
    pub state: IronClawAutomationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<IronClawAutomationRunStatus>,
    #[serde(default)]
    pub recent_runs: Vec<IronClawAutomationRecentRunInfo>,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// Present while this automation's active fire is held (gate-parked or
    /// still running) and scheduled fires are being skipped (#5886). Derived
    /// at read time from the active run's state; never persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_hold: Option<IronClawAutomationActiveHold>,
}

/// Why an automation's schedule is currently held, plus elapsed-occurrence
/// accounting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawAutomationActiveHold {
    pub reason: IronClawAutomationHoldReason,
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
pub enum IronClawAutomationHoldReason {
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
pub enum IronClawAutomationSource {
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
pub struct IronClawExtensionListResponse {
    pub extensions: Vec<IronClawExtensionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawSkillListResponse {
    pub skills: Vec<IronClawSkillInfo>,
    pub count: usize,
    /// Global default criteria-based skill auto-activation master switch. When
    /// `false`, skills activate only via an explicit `/name` mention. Defaults
    /// to `true` for back-compat with producers that predate the flag.
    #[serde(default = "default_true")]
    pub auto_activate_learned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawSkillContentResponse {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawSkillSearchResponse {
    #[serde(default)]
    pub catalog: Vec<serde_json::Value>,
    #[serde(default)]
    pub installed: Vec<IronClawSkillInfo>,
    #[serde(default)]
    pub registry_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawSkillActionResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawSkillInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub trust: IronClawSkillTrustLevel,
    pub source: IronClawSkillSourceKind,
    pub source_kind: IronClawSkillSourceKind,
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
pub enum IronClawSkillTrustLevel {
    Trusted,
    Installed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawSkillSourceKind {
    User,
    Installed,
    Workspace,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawExtensionRegistryResponse {
    pub entries: Vec<IronClawExtensionRegistryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawExtensionRegistryEntry {
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
    pub surfaces: Vec<IronClawExtensionSurface>,
}

/// One product-facing surface an installed extension declares, as rendered on
/// the extensions wire. `channel` carries typed direction (inbound = external
/// messages arrive here; outbound = the host delivers final replies /
/// notifications here) plus the caller-scoped connection state and connect
/// affordance when the surface requires an account binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IronClawExtensionSurface {
    Tool,
    Auth,
    Channel {
        inbound: bool,
        outbound: bool,
        /// The auth account this channel surface resolves to for its vendor,
        /// when the surface binds a caller-scoped account. `None` until an
        /// account exists. One account per vendor today (ADR 0001 keeps the
        /// list shape); the id points into
        /// [`IronClawExtensionInfo::auth_accounts`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved_account_id: Option<String>,
        /// How the resolved account was chosen: the per-(user, vendor) default
        /// or an explicit per-extension binding. Always `Default` today — no
        /// binding behavior ships until the multi-account follow-up.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binding_source: Option<IronClawAccountBindingSource>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connection: Option<ChannelConnectionRequirement>,
    },
}

/// A vendor's connected accounts, as modeled on the extensions wire
/// (overview §6.4, `adr/0001-multiple-accounts-per-vendor.md`). One account per
/// vendor per user today; the list shape is frozen so the accepted
/// multi-account follow-up extends behavior without a wire break. The connect
/// card reads `accounts[0]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawVendorAuthAccounts {
    /// The credential-authority vendor id (the provider namespace a recipe
    /// authenticates against).
    pub vendor: String,
    pub accounts: Vec<IronClawAuthAccount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawTraceHoldAuthorizeProductRequest {
    pub submission_id: String,
}

/// One connected account for a vendor. `state` is the shared §6.3 auth-account
/// state machine, exposed exactly — no vendor- or extension-specific state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawAuthAccount {
    pub account_id: String,
    /// User-facing label (defaulted from the recipe's identity claim — email,
    /// workspace name — falling back to the extension display name until
    /// identity extraction lands).
    pub label: String,
    pub state: AuthAccountState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<AuthAccountLastError>,
    /// Exactly one account per (user, vendor) is the default whenever any
    /// account exists. Always `true` today (list length ≤ 1).
    pub is_default: bool,
}

/// How a surface's resolved account was chosen (ADR 0001). Always `Default`
/// today — explicit per-extension bindings ship with the multi-account PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawAccountBindingSource {
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawExtensionInfo {
    pub package_ref: LifecyclePackageRef,
    pub display_name: String,
    /// Runtime implementation name (`wasm` / `mcp` / `first_party` / ...).
    /// Implementation detail — product taxonomy lives in `surfaces`.
    pub runtime: String,
    pub description: String,
    pub authenticated: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    pub needs_setup: bool,
    pub has_auth: bool,
    /// The installation lifecycle state (§6.1), projected honestly and exposed
    /// exactly: one of `installed` / `configured` / `active` / `disabled` /
    /// `failed` / `unsupported`. `failed` is a terminal non-auth activation
    /// failure and carries its redacted reason in `activation_error`;
    /// auth-rejection failures surface on the `auth_accounts` axis instead.
    pub installation_state: InstallationState,
    /// Redacted reason for a `failed` installation state (the durable
    /// installation record's `last_error`); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding_state: Option<IronClawExtensionOnboardingState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding: Option<IronClawExtensionOnboardingPayload>,
    /// Per-vendor connected accounts (§6.4, ADR 0001). Length ≤ 1 today; the
    /// list shape is frozen for the multi-account follow-up. The connect card
    /// reads `accounts[0]`; affordances derive from this state + the
    /// installation state + config completeness.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auth_accounts: Vec<IronClawVendorAuthAccounts>,
    /// Declared product surfaces (tool / auth / channel-with-direction).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<IronClawExtensionSurface>,
    /// Whether this install is tenant-shared or private to the caller
    /// (#5459 P1); `None` on pre-#5459 payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_scope: Option<LifecycleInstallScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawExtensionActionResponse {
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
    pub onboarding_state: Option<IronClawExtensionOnboardingState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding: Option<IronClawExtensionOnboardingPayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawExtensionOnboardingState {
    AuthRequired,
    SetupRequired,
    Installed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawExtensionOnboardingPayload {
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
pub struct IronClawSetupExtensionResponse {
    pub package_ref: LifecyclePackageRef,
    pub phase: InstallationState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<LifecycleReadinessBlocker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<LifecycleProductPayload>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<IronClawExtensionSetupSecret>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<IronClawExtensionSetupField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding: Option<IronClawExtensionOnboardingPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawExtensionSetupSecret {
    pub name: String,
    pub provider: String,
    pub prompt: String,
    pub optional: bool,
    pub provided: bool,
    pub setup: IronClawExtensionCredentialSetup,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IronClawExtensionCredentialSetup {
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawExtensionSetupField {
    pub name: String,
    pub prompt: String,
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawOperatorArea {
    Setup,
    Config,
    Diagnostics,
    Logs,
    Status,
    ServiceLifecycle,
}

impl IronClawOperatorArea {
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
pub struct IronClawOperatorCommandPlaneResponse {
    pub area: IronClawOperatorArea,
    pub status: IronClawOperatorSurfaceStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_status: Option<IronClawOperatorStatusResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs: Option<IronClawLogQueryResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_lifecycle: Option<IronClawServiceLifecycleResponse>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<IronClawOperatorConfigDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawOperatorSurfaceStatus {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IronClawOperatorSetupRequest {
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

impl IronClawOperatorSetupRequest {
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
pub struct IronClawOperatorSetupResponse {
    pub area: IronClawOperatorArea,
    pub status: IronClawOperatorSetupStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<IronClawOperatorSetupStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<IronClawOperatorConfigDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawOperatorSetupStatus {
    Complete,
    Incomplete,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawOperatorSetupStep {
    pub name: String,
    pub status: IronClawOperatorSetupStepStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawOperatorSetupStepStatus {
    Complete,
    Required,
    Unsupported,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawOperatorConfigValidateRequest {
    #[serde(default)]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawOperatorLogsQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub level: Option<IronClawLogLevel>,
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
pub struct IronClawOperatorServiceLifecycleRequest {
    pub action: IronClawOperatorServiceLifecycleAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawOperatorServiceLifecycleAction {
    Install,
    Start,
    Stop,
    Status,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IronClawOperatorConfigListResponse {
    pub entries: Vec<IronClawOperatorConfigEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub precedence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<IronClawOperatorConfigDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IronClawOperatorConfigGetResponse {
    pub entry: IronClawOperatorConfigEntry,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct IronClawOperatorConfigEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub source: String,
    pub redacted: bool,
    pub mutable: bool,
}

impl Serialize for IronClawOperatorConfigEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("IronClawOperatorConfigEntry", 5)?;
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
pub struct IronClawOperatorConfigSetRequest {
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IronClawOperatorConfigSetProductRequest {
    pub key: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawOperatorConfigValidateResponse {
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<IronClawOperatorConfigDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawOperatorConfigDiagnostic {
    pub key: String,
    pub severity: IronClawOperatorConfigDiagnosticSeverity,
    pub reason_code: String,
    pub message: String,
    pub owning_area: IronClawOperatorArea,
    pub remediation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawOperatorConfigDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn operator_config_entry_masks_redacted_value_when_serialized() {
        let entry = IronClawOperatorConfigEntry {
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
}

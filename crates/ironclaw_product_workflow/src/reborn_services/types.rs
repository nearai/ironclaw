use chrono::{DateTime, Utc};
use ironclaw_host_api::ThreadId;
use ironclaw_product_adapters::{ProductOutboundEnvelope, ProjectionCursor};
use ironclaw_threads::{SessionThreadRecord, SummaryArtifact, ThreadMessageRecord};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunResponse, EventCursor, GateRef, ResumeTurnResponse,
    SanitizedFailure, TurnCheckpointId, TurnRunId, TurnRunState, TurnStatus,
};
use secrecy::SecretString;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, de};

use crate::{
    LifecyclePackageRef, LifecyclePhase, LifecycleProductPayload, LifecycleReadinessBlocker,
};

const OUTBOUND_DELIVERY_TARGET_ID_MAX_BYTES: usize = 512;
const OUTBOUND_DELIVERY_CHANNEL_MAX_BYTES: usize = 128;
const OUTBOUND_DELIVERY_DISPLAY_NAME_MAX_BYTES: usize = 256;
const OUTBOUND_DELIVERY_DESCRIPTION_MAX_BYTES: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOperatorStatusState {
    Ready,
    Degraded,
    Blocked,
    Unsupported,
    NotConfigured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOperatorStatusSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorStatusCheck {
    pub id: String,
    pub status: RebornOperatorStatusState,
    pub severity: RebornOperatorStatusSeverity,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOperatorStatusResponse {
    pub generated_at: DateTime<Utc>,
    pub overall: RebornOperatorStatusState,
    pub checks: Vec<RebornOperatorStatusCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RebornLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

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
    #[serde(default)]
    pub tail: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornLogEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub level: RebornLogLevel,
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornLogQueryResponse {
    pub source: String,
    pub entries: Vec<RebornLogEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub tail_supported: bool,
    pub follow_supported: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornServiceLifecycleAction {
    Install,
    Start,
    Stop,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornServiceLifecycleState {
    Installed,
    Running,
    Stopped,
    Unsupported,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornServiceLifecycleRequest {
    pub action: RebornServiceLifecycleAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornServiceLifecycleResponse {
    pub action: RebornServiceLifecycleAction,
    pub state: RebornServiceLifecycleState,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

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
    AdminManagedChannels,
    WebGeneratedCode,
    QrCode,
    OAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornChannelConnectAction {
    pub title: String,
    pub instructions: String,
    #[serde(rename = "input_placeholder", alias = "code_placeholder")]
    pub input_placeholder: String,
    pub submit_label: String,
    pub success_message: String,
    pub error_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornCreateThreadResponse {
    pub thread: SessionThreadRecord,
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

/// Bounded product projection for caller-scoped automations.
///
/// The beta API currently returns one capped page without a cursor. Future
/// pagination can extend this response with an optional cursor without changing
/// the source-tagged automation rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornListAutomationsResponse {
    pub automations: Vec<RebornAutomationInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RebornOutboundPreferencesResponseWire")]
pub struct RebornOutboundPreferencesResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_reply_target: Option<RebornOutboundDeliveryTargetSummary>,
    #[serde(default)]
    pub final_reply_target_status: RebornOutboundDeliveryTargetStatus,
    #[serde(default)]
    pub default_modality: RebornOutboundDeliveryModality,
}

impl Default for RebornOutboundPreferencesResponse {
    fn default() -> Self {
        Self {
            final_reply_target: None,
            final_reply_target_status: RebornOutboundDeliveryTargetStatus::NoneConfigured,
            default_modality: RebornOutboundDeliveryModality::Text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RebornOutboundPreferencesResponseWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    final_reply_target: Option<RebornOutboundDeliveryTargetSummary>,
    #[serde(default)]
    final_reply_target_status: Option<RebornOutboundDeliveryTargetStatus>,
    #[serde(default)]
    default_modality: Option<RebornOutboundDeliveryModality>,
}

impl From<RebornOutboundPreferencesResponseWire> for RebornOutboundPreferencesResponse {
    fn from(value: RebornOutboundPreferencesResponseWire) -> Self {
        let final_reply_target_status = match (
            value.final_reply_target.as_ref(),
            value.final_reply_target_status,
        ) {
            (Some(_), None) => RebornOutboundDeliveryTargetStatus::Available,
            (_, Some(status)) => status,
            (None, None) => RebornOutboundDeliveryTargetStatus::NoneConfigured,
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
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornOutboundDeliveryModality {
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSetOutboundPreferencesRequest {
    /// `Some(id)` sets the final-reply target; `None` clears it.
    ///
    /// The field defaults to `None` when omitted, so clients that want to leave
    /// an existing value unchanged must use the read endpoint instead of
    /// submitting a partial update without this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_reply_target_id: Option<RebornOutboundDeliveryTargetId>,
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
    pub thread_id: ThreadId,
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

/// Browser-safe automation row returned by the WebUI facade.
///
/// This deliberately exposes source, state, run timestamps, sanitized status,
/// and bounded recent-run history; trigger repository internals remain behind
/// the product facade.
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornExtensionListResponse {
    pub extensions: Vec<RebornExtensionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSkillListResponse {
    pub skills: Vec<RebornSkillInfo>,
    pub count: usize,
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
    pub tail: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}

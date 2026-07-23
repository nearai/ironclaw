//! Outbound envelope, projection-derived payloads, and projection cursor.
// arch-exempt: large_file, product activity view compatibility before DTO split, plan #6543

use crate::{
    CapabilityId, ExtensionId, InvocationId, NetworkMethod, ProcessId, RuntimeKind, ThreadId,
};
use crate::{ReplyTargetBindingRef, SanitizedFailure, TurnRunId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::product_adapter::error::ProductAdapterError;
use crate::product_adapter::external::{ExternalActorRef, ExternalConversationRef};
use crate::product_adapter::identity::{AdapterInstallationId, ProductAdapterId};

const PROJECTION_CURSOR_MAX_BYTES: usize = 1024;
const PROJECTION_THREAD_ID_MAX_BYTES: usize = 512;
const PROJECTION_ITEM_ID_MAX_BYTES: usize = 512;
pub const PROJECTION_TEXT_MAX_BYTES: usize = 128 * 1024;
const PROJECTION_WORK_SUMMARY_MAX_BYTES: usize = 1024;
/// Maximum byte length for a projected skill activation name.
pub const PROJECTION_SKILL_NAME_MAX_BYTES: usize = 128;
/// Maximum byte length for a projected skill activation feedback note.
pub const PROJECTION_SKILL_FEEDBACK_MAX_BYTES: usize = 1024;
/// Maximum number of skill activation names or feedback notes per projection item.
pub const PROJECTION_SKILL_ACTIVATION_MAX_ITEMS: usize = 16;
const CAPABILITY_ACTIVITY_ERROR_KIND_MAX_BYTES: usize = 64;
const CAPABILITY_ACTIVITY_ERROR_KIND_SEGMENT_MAX_BYTES: usize = 24;
const CAPABILITY_ACTIVITY_UNCLASSIFIED_ERROR_KIND: &str = "Unclassified";
pub const CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES: usize = 2 * 1024;
pub const CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES: usize = 16 * 1024;
pub const CAPABILITY_DISPLAY_KIND_MAX_BYTES: usize = 32;
pub const CAPABILITY_DISPLAY_RESULT_REF_MAX_BYTES: usize = 256;
const APPROVAL_PROMPT_TEXT_MAX_BYTES: usize = 2 * 1024;
const APPROVAL_PROMPT_DETAIL_MAX_ITEMS: usize = 16;

/// Everything the host knows when encoding a preference reply-target
/// binding ref: the durable installation identity plus the external
/// conversation the target delivers to. Vendor codecs turn it into their
/// binding-ref grammar.
#[derive(Debug, Clone, Copy)]
pub struct PreferenceTargetEncodeRequest<'a> {
    pub installation_id: &'a AdapterInstallationId,
    pub agent_id: &'a crate::AgentId,
    pub project_id: Option<&'a crate::ProjectId>,
    pub conversation: &'a ExternalConversationRef,
}

/// Encodes and decodes stored preference reply-target bindings. The
/// binding-ref grammar is authored by the channel's vendor integration (the
/// channel extension crate), so both halves live there; this port is the
/// only place vendor knowledge enters the generic outbound-target and
/// triggered-delivery paths.
pub trait PreferenceTargetCodec: Send + Sync {
    /// Decode a preference binding ref into the conversation it targets.
    fn conversation_for_target(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<ExternalConversationRef>;

    /// Whether the binding ref targets a personal 1:1 direct message (the
    /// only surface OAuth authorization URLs may land on).
    fn is_personal_direct_message(&self, target: &ReplyTargetBindingRef) -> bool;

    /// The external actor id a personal direct-message binding ref carries
    /// (`None` for shared-conversation or malformed refs). The generic
    /// outbound-target provider matches this against the provisioned
    /// DM-target record so a tampered actor segment never resolves.
    fn direct_message_actor_for_target(&self, target: &ReplyTargetBindingRef) -> Option<String>;

    /// Encode a shared-conversation reply-target binding ref (the inverse of
    /// [`Self::conversation_for_target`] for shared conversations). `None`
    /// when the conversation cannot be encoded in the vendor grammar — the
    /// caller offers no target rather than a malformed one (fail closed).
    fn encode_shared_conversation_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
    ) -> Option<ReplyTargetBindingRef>;

    /// Encode a personal direct-message reply-target binding ref carrying
    /// the proven external actor — the only refs
    /// [`Self::is_personal_direct_message`] accepts. `None` when the
    /// conversation or actor cannot be encoded (fail closed).
    fn encode_personal_direct_message_target(
        &self,
        request: PreferenceTargetEncodeRequest<'_>,
        external_actor_id: &str,
    ) -> Option<ReplyTargetBindingRef>;
}

fn serialize_failure_category<S>(
    value: &Option<SanitizedFailure>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(failure) => serializer.serialize_some(failure.category()),
        None => serializer.serialize_none(),
    }
}

fn invalid(kind: &'static str, reason: impl Into<String>) -> ProductAdapterError {
    ProductAdapterError::InvalidIdentifier {
        kind,
        reason: reason.into(),
    }
}

fn validate_bounded_text(
    kind: &'static str,
    value: &str,
    max: usize,
) -> Result<(), ProductAdapterError> {
    if value.is_empty() {
        return Err(invalid(kind, "must not be empty"));
    }
    if value.len() > max {
        return Err(invalid(kind, format!("must be at most {max} bytes")));
    }
    if value
        .chars()
        .any(|c| c == '\0' || c.is_control() && c != '\n' && c != '\t')
    {
        return Err(invalid(
            kind,
            "must not contain unsupported control characters",
        ));
    }
    Ok(())
}

fn validate_error_kind(kind: &'static str, value: &str) -> Result<(), ProductAdapterError> {
    if value == CAPABILITY_ACTIVITY_UNCLASSIFIED_ERROR_KIND {
        return Ok(());
    }
    if value.is_empty() {
        return Err(invalid(kind, "must not be empty"));
    }
    if value.len() > CAPABILITY_ACTIVITY_ERROR_KIND_MAX_BYTES {
        return Err(invalid(
            kind,
            format!("must be at most {CAPABILITY_ACTIVITY_ERROR_KIND_MAX_BYTES} bytes"),
        ));
    }
    if !value.as_bytes()[0].is_ascii_lowercase() {
        return Err(invalid(kind, "must start with a lowercase ASCII letter"));
    }
    if value.bytes().any(|byte| {
        !byte.is_ascii_lowercase() && !byte.is_ascii_digit() && !matches!(byte, b'_' | b'.' | b':')
    }) {
        return Err(invalid(
            kind,
            "must contain only safe error-kind characters",
        ));
    }
    for segment in value.split(['.', ':']) {
        if segment.is_empty() || segment.len() > CAPABILITY_ACTIVITY_ERROR_KIND_SEGMENT_MAX_BYTES {
            return Err(invalid(kind, "contains an invalid segment"));
        }
        if !segment.as_bytes()[0].is_ascii_lowercase() {
            return Err(invalid(
                kind,
                "each segment must start with a lowercase ASCII letter",
            ));
        }
    }
    Ok(())
}

fn validate_optional_display_text(
    kind: &'static str,
    value: Option<&str>,
    max: usize,
) -> Result<(), ProductAdapterError> {
    if let Some(value) = value {
        validate_bounded_text(kind, value, max)?;
    }
    Ok(())
}

fn validate_display_preview(value: Option<&str>) -> Result<(), ProductAdapterError> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_bounded_text(
        "capability_display_output_preview",
        value,
        CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES,
    )
}

fn validate_display_kind(value: Option<&str>) -> Result<(), ProductAdapterError> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_bounded_text(
        "capability_display_output_kind",
        value,
        CAPABILITY_DISPLAY_KIND_MAX_BYTES,
    )?;
    if !value.as_bytes()[0].is_ascii_lowercase()
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_lowercase() && !byte.is_ascii_digit() && byte != b'_')
    {
        return Err(invalid(
            "capability_display_output_kind",
            "must be snake_case ASCII",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ProjectionCursor(String);

impl ProjectionCursor {
    pub fn new(value: impl Into<String>) -> Result<Self, ProductAdapterError> {
        let value = value.into();
        validate_bounded_text("projection_cursor", &value, PROJECTION_CURSOR_MAX_BYTES)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProjectionCursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalReplyView {
    pub turn_run_id: TurnRunId,
    pub text: String,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressUpdateView {
    pub turn_run_id: TurnRunId,
    pub kind: ProgressKind,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressKind {
    Typing,
    ToolRunning,
    Reflecting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductWorkSummaryPhase {
    Planning,
    Waiting,
    Retrying,
    Context,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityActivityView {
    pub invocation_id: InvocationId,
    pub turn_run_id: Option<TurnRunId>,
    pub thread_id: Option<ThreadId>,
    pub capability_id: CapabilityId,
    pub status: CapabilityActivityStatusView,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
    /// Bounded, sanitized failure summary for a failed activity (e.g. a
    /// builtin's `"invalid JSON: ..."` message), so the live per-tool card can
    /// show the real reason instead of only `error_kind`.
    pub error_detail: Option<String>,
    /// Inline primary-argument detail for the activity row, surfaced while the
    /// invocation is still running (the completed card carries its own).
    pub subtitle: Option<String>,
    /// Per-tool parameter summary, surfaced while the invocation is still
    /// running so the row's Parameters tab is populated before the result.
    pub input_summary: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub activity_order: Option<u64>,
}

impl Serialize for CapabilityActivityView {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;

        #[derive(Serialize)]
        struct Wire<'a> {
            invocation_id: &'a InvocationId,
            #[serde(skip_serializing_if = "Option::is_none")]
            turn_run_id: &'a Option<TurnRunId>,
            thread_id: &'a Option<ThreadId>,
            capability_id: &'a CapabilityId,
            status: CapabilityActivityStatusView,
            provider: &'a Option<ExtensionId>,
            runtime: &'a Option<RuntimeKind>,
            process_id: &'a Option<ProcessId>,
            output_bytes: Option<u64>,
            error_kind: &'a Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            error_detail: &'a Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            subtitle: &'a Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            input_summary: &'a Option<String>,
            updated_at: &'a DateTime<Utc>,
            #[serde(skip_serializing_if = "Option::is_none")]
            activity_order: Option<u64>,
        }

        Wire {
            invocation_id: &self.invocation_id,
            turn_run_id: &self.turn_run_id,
            thread_id: &self.thread_id,
            capability_id: &self.capability_id,
            status: self.status,
            provider: &self.provider,
            runtime: &self.runtime,
            process_id: &self.process_id,
            output_bytes: self.output_bytes,
            error_kind: &self.error_kind,
            error_detail: &self.error_detail,
            subtitle: &self.subtitle,
            input_summary: &self.input_summary,
            updated_at: &self.updated_at,
            activity_order: self.activity_order,
        }
        .serialize(serializer)
    }
}

impl CapabilityActivityView {
    pub fn new(input: CapabilityActivityViewInput) -> Result<Self, ProductAdapterError> {
        let value = Self {
            invocation_id: input.invocation_id,
            turn_run_id: input.turn_run_id,
            thread_id: input.thread_id,
            capability_id: input.capability_id,
            status: input.status,
            provider: input.provider,
            runtime: input.runtime,
            process_id: input.process_id,
            output_bytes: input.output_bytes,
            error_kind: input.error_kind,
            error_detail: input.error_detail,
            subtitle: input.subtitle,
            input_summary: input.input_summary,
            updated_at: input.updated_at,
            activity_order: input.activity_order,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), ProductAdapterError> {
        if let Some(error_kind) = self.error_kind.as_deref() {
            validate_error_kind("capability_activity_error_kind", error_kind)?;
        }
        // Sanitized/bounded upstream; re-validate at the product boundary so an
        // upstream regression can't leak unbounded/control-char text.
        validate_optional_display_text(
            "capability_activity_error_detail",
            self.error_detail.as_deref(),
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )?;
        // The running-frame input fields are sanitized/byte-bounded upstream;
        // re-validate them at the product boundary (as `error_kind` is) so an
        // upstream regression can't leak unbounded/control-char text to the
        // browser.
        validate_optional_display_text(
            "capability_activity_subtitle",
            self.subtitle.as_deref(),
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "capability_activity_input_summary",
            self.input_summary.as_deref(),
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityActivityViewInput {
    pub invocation_id: InvocationId,
    pub turn_run_id: Option<TurnRunId>,
    pub thread_id: Option<ThreadId>,
    pub capability_id: CapabilityId,
    pub status: CapabilityActivityStatusView,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
    pub error_detail: Option<String>,
    pub subtitle: Option<String>,
    pub input_summary: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub activity_order: Option<u64>,
}

impl<'de> Deserialize<'de> for CapabilityActivityView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            invocation_id: InvocationId,
            #[serde(default)]
            turn_run_id: Option<TurnRunId>,
            thread_id: Option<ThreadId>,
            capability_id: CapabilityId,
            status: CapabilityActivityStatusView,
            provider: Option<ExtensionId>,
            #[serde(
                default,
                deserialize_with = "crate::deserialize_trusted_optional_runtime_kind"
            )]
            runtime: Option<RuntimeKind>,
            process_id: Option<ProcessId>,
            output_bytes: Option<u64>,
            error_kind: Option<String>,
            #[serde(default)]
            error_detail: Option<String>,
            #[serde(default)]
            subtitle: Option<String>,
            #[serde(default)]
            input_summary: Option<String>,
            updated_at: DateTime<Utc>,
            activity_order: Option<u64>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(CapabilityActivityViewInput {
            invocation_id: wire.invocation_id,
            turn_run_id: wire.turn_run_id,
            thread_id: wire.thread_id,
            capability_id: wire.capability_id,
            status: wire.status,
            provider: wire.provider,
            runtime: wire.runtime,
            process_id: wire.process_id,
            output_bytes: wire.output_bytes,
            error_kind: wire.error_kind,
            error_detail: wire.error_detail,
            subtitle: wire.subtitle,
            input_summary: wire.input_summary,
            updated_at: wire.updated_at,
            activity_order: wire.activity_order,
        })
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityActivityStatusView {
    Started,
    Running,
    Completed,
    Failed,
    Killed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDisplayPreviewView {
    pub timeline_message_id: Option<String>,
    pub invocation_id: InvocationId,
    pub turn_run_id: Option<TurnRunId>,
    pub thread_id: Option<ThreadId>,
    pub capability_id: CapabilityId,
    pub status: CapabilityActivityStatusView,
    pub error_kind: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
    pub input_summary: Option<String>,
    pub output_summary: Option<String>,
    pub output_preview: Option<String>,
    pub output_kind: Option<String>,
    pub output_bytes: Option<u64>,
    pub result_ref: Option<String>,
    pub truncated: bool,
    pub updated_at: DateTime<Utc>,
    pub activity_order: Option<u64>,
}

impl CapabilityDisplayPreviewView {
    pub fn new(input: CapabilityDisplayPreviewViewInput) -> Result<Self, ProductAdapterError> {
        let value = Self {
            timeline_message_id: input.timeline_message_id,
            invocation_id: input.invocation_id,
            turn_run_id: input.turn_run_id,
            thread_id: input.thread_id,
            capability_id: input.capability_id,
            status: input.status,
            error_kind: input.error_kind,
            title: input.title,
            subtitle: input.subtitle,
            input_summary: input.input_summary,
            output_summary: input.output_summary,
            output_preview: input.output_preview,
            output_kind: input.output_kind,
            output_bytes: input.output_bytes,
            result_ref: input.result_ref,
            truncated: input.truncated,
            updated_at: input.updated_at,
            activity_order: input.activity_order,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_bounded_text(
            "capability_display_title",
            &self.title,
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "capability_display_subtitle",
            self.subtitle.as_deref(),
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "capability_display_input_summary",
            self.input_summary.as_deref(),
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "capability_display_output_summary",
            self.output_summary.as_deref(),
            CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES,
        )?;
        validate_display_preview(self.output_preview.as_deref())?;
        validate_display_kind(self.output_kind.as_deref())?;
        if let Some(error_kind) = self.error_kind.as_deref() {
            validate_error_kind("capability_display_error_kind", error_kind)?;
        }
        validate_optional_display_text(
            "capability_display_timeline_message_id",
            self.timeline_message_id.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "capability_display_result_ref",
            self.result_ref.as_deref(),
            CAPABILITY_DISPLAY_RESULT_REF_MAX_BYTES,
        )?;
        Ok(())
    }
}

impl Serialize for CapabilityDisplayPreviewView {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;

        #[derive(Serialize)]
        struct Wire<'a> {
            timeline_message_id: &'a Option<String>,
            invocation_id: &'a InvocationId,
            #[serde(skip_serializing_if = "Option::is_none")]
            turn_run_id: &'a Option<TurnRunId>,
            thread_id: &'a Option<ThreadId>,
            capability_id: &'a CapabilityId,
            status: CapabilityActivityStatusView,
            #[serde(skip_serializing_if = "Option::is_none")]
            error_kind: &'a Option<String>,
            title: &'a str,
            subtitle: &'a Option<String>,
            input_summary: &'a Option<String>,
            output_summary: &'a Option<String>,
            output_preview: &'a Option<String>,
            output_kind: &'a Option<String>,
            output_bytes: Option<u64>,
            result_ref: &'a Option<String>,
            truncated: bool,
            updated_at: &'a DateTime<Utc>,
            #[serde(skip_serializing_if = "Option::is_none")]
            activity_order: Option<u64>,
        }

        Wire {
            timeline_message_id: &self.timeline_message_id,
            invocation_id: &self.invocation_id,
            turn_run_id: &self.turn_run_id,
            thread_id: &self.thread_id,
            capability_id: &self.capability_id,
            status: self.status,
            error_kind: &self.error_kind,
            title: &self.title,
            subtitle: &self.subtitle,
            input_summary: &self.input_summary,
            output_summary: &self.output_summary,
            output_preview: &self.output_preview,
            output_kind: &self.output_kind,
            output_bytes: self.output_bytes,
            result_ref: &self.result_ref,
            truncated: self.truncated,
            updated_at: &self.updated_at,
            activity_order: self.activity_order,
        }
        .serialize(serializer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDisplayPreviewViewInput {
    pub timeline_message_id: Option<String>,
    pub invocation_id: InvocationId,
    pub turn_run_id: Option<TurnRunId>,
    pub thread_id: Option<ThreadId>,
    pub capability_id: CapabilityId,
    pub status: CapabilityActivityStatusView,
    pub error_kind: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
    pub input_summary: Option<String>,
    pub output_summary: Option<String>,
    pub output_preview: Option<String>,
    pub output_kind: Option<String>,
    pub output_bytes: Option<u64>,
    pub result_ref: Option<String>,
    pub truncated: bool,
    pub updated_at: DateTime<Utc>,
    pub activity_order: Option<u64>,
}

impl<'de> Deserialize<'de> for CapabilityDisplayPreviewView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            timeline_message_id: Option<String>,
            invocation_id: InvocationId,
            #[serde(default)]
            turn_run_id: Option<TurnRunId>,
            thread_id: Option<ThreadId>,
            capability_id: CapabilityId,
            status: CapabilityActivityStatusView,
            error_kind: Option<String>,
            title: String,
            subtitle: Option<String>,
            input_summary: Option<String>,
            output_summary: Option<String>,
            output_preview: Option<String>,
            output_kind: Option<String>,
            output_bytes: Option<u64>,
            result_ref: Option<String>,
            truncated: bool,
            updated_at: DateTime<Utc>,
            activity_order: Option<u64>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(CapabilityDisplayPreviewViewInput {
            timeline_message_id: wire.timeline_message_id,
            invocation_id: wire.invocation_id,
            turn_run_id: wire.turn_run_id,
            thread_id: wire.thread_id,
            capability_id: wire.capability_id,
            status: wire.status,
            error_kind: wire.error_kind,
            title: wire.title,
            subtitle: wire.subtitle,
            input_summary: wire.input_summary,
            output_summary: wire.output_summary,
            output_preview: wire.output_preview,
            output_kind: wire.output_kind,
            output_bytes: wire.output_bytes,
            result_ref: wire.result_ref,
            truncated: wire.truncated,
            updated_at: wire.updated_at,
            activity_order: wire.activity_order,
        })
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatePromptView {
    pub turn_run_id: TurnRunId,
    pub gate_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<InvocationId>,
    pub headline: String,
    pub body: String,
    #[serde(default)]
    pub allow_always: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_context: Option<ApprovalPromptContextView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalPromptContextView {
    pub tool_name: String,
    pub action: ApprovalPromptActionView,
    pub scope: ApprovalPromptScopeView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<ApprovalPromptDestinationView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<ApprovalPromptDetailView>,
}

impl ApprovalPromptContextView {
    pub fn new(
        tool_name: impl Into<String>,
        action: ApprovalPromptActionView,
        scope: ApprovalPromptScopeView,
        reason: Option<String>,
        destination: Option<ApprovalPromptDestinationView>,
        details: Vec<ApprovalPromptDetailView>,
    ) -> Result<Self, ProductAdapterError> {
        let view = Self {
            tool_name: tool_name.into(),
            action,
            scope,
            reason,
            destination,
            details,
        };
        view.validate()?;
        Ok(view)
    }

    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_bounded_text(
            "approval_prompt_tool_name",
            &self.tool_name,
            APPROVAL_PROMPT_TEXT_MAX_BYTES,
        )?;
        self.action.validate()?;
        self.scope.validate()?;
        validate_optional_display_text(
            "approval_prompt_reason",
            self.reason.as_deref(),
            APPROVAL_PROMPT_TEXT_MAX_BYTES,
        )?;
        if let Some(destination) = &self.destination {
            destination.validate()?;
        }
        if self.details.len() > APPROVAL_PROMPT_DETAIL_MAX_ITEMS {
            return Err(invalid(
                "approval_prompt_details",
                format!("must contain at most {APPROVAL_PROMPT_DETAIL_MAX_ITEMS} items"),
            ));
        }
        for detail in &self.details {
            detail.validate()?;
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ApprovalPromptContextView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            tool_name: String,
            action: ApprovalPromptActionView,
            scope: ApprovalPromptScopeView,
            #[serde(default)]
            reason: Option<String>,
            #[serde(default)]
            destination: Option<ApprovalPromptDestinationView>,
            #[serde(default)]
            details: Vec<ApprovalPromptDetailView>,
        }

        let wire = Wire::deserialize(deserializer)?;
        ApprovalPromptContextView::new(
            wire.tool_name,
            wire.action,
            wire.scope,
            wire.reason,
            wire.destination,
            wire.details,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalPromptActionView {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<NetworkMethod>,
}

impl ApprovalPromptActionView {
    pub fn new(
        label: impl Into<String>,
        method: Option<NetworkMethod>,
    ) -> Result<Self, ProductAdapterError> {
        let view = Self {
            label: label.into(),
            method,
        };
        view.validate()?;
        Ok(view)
    }

    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_bounded_text(
            "approval_prompt_action_label",
            &self.label,
            APPROVAL_PROMPT_TEXT_MAX_BYTES,
        )
    }
}

impl<'de> Deserialize<'de> for ApprovalPromptActionView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            label: String,
            #[serde(default)]
            method: Option<NetworkMethod>,
        }

        let wire = Wire::deserialize(deserializer)?;
        ApprovalPromptActionView::new(wire.label, wire.method).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalPromptScopeView {
    pub label: String,
    pub reusable: bool,
}

impl ApprovalPromptScopeView {
    pub fn new(label: impl Into<String>, reusable: bool) -> Result<Self, ProductAdapterError> {
        let view = Self {
            label: label.into(),
            reusable,
        };
        view.validate()?;
        Ok(view)
    }

    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_bounded_text(
            "approval_prompt_scope_label",
            &self.label,
            APPROVAL_PROMPT_TEXT_MAX_BYTES,
        )
    }
}

impl<'de> Deserialize<'de> for ApprovalPromptScopeView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            label: String,
            reusable: bool,
        }

        let wire = Wire::deserialize(deserializer)?;
        ApprovalPromptScopeView::new(wire.label, wire.reusable).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalPromptDestinationView {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

impl ApprovalPromptDestinationView {
    pub fn new(
        label: impl Into<String>,
        url: Option<String>,
        domain: Option<String>,
    ) -> Result<Self, ProductAdapterError> {
        let view = Self {
            label: label.into(),
            url,
            domain,
        };
        view.validate()?;
        Ok(view)
    }

    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_bounded_text(
            "approval_prompt_destination_label",
            &self.label,
            APPROVAL_PROMPT_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "approval_prompt_destination_url",
            self.url.as_deref(),
            APPROVAL_PROMPT_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "approval_prompt_destination_domain",
            self.domain.as_deref(),
            APPROVAL_PROMPT_TEXT_MAX_BYTES,
        )
    }
}

impl<'de> Deserialize<'de> for ApprovalPromptDestinationView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            label: String,
            #[serde(default)]
            url: Option<String>,
            #[serde(default)]
            domain: Option<String>,
        }

        let wire = Wire::deserialize(deserializer)?;
        ApprovalPromptDestinationView::new(wire.label, wire.url, wire.domain)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalPromptDetailView {
    pub label: String,
    pub value: String,
}

impl ApprovalPromptDetailView {
    pub fn new(
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, ProductAdapterError> {
        let view = Self {
            label: label.into(),
            value: value.into(),
        };
        view.validate()?;
        Ok(view)
    }

    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_bounded_text(
            "approval_prompt_detail_label",
            &self.label,
            APPROVAL_PROMPT_TEXT_MAX_BYTES,
        )?;
        validate_bounded_text(
            "approval_prompt_detail_value",
            &self.value,
            APPROVAL_PROMPT_TEXT_MAX_BYTES,
        )
    }
}

impl<'de> Deserialize<'de> for ApprovalPromptDetailView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            label: String,
            value: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        ApprovalPromptDetailView::new(wire.label, wire.value).map_err(serde::de::Error::custom)
    }
}

/// Discriminator for the kind of auth challenge surfaced in an `AuthPromptView`.
///
/// Added in issue #4112 as additive optional context. Legacy consumers that
/// serialized `AuthPromptView` before this field existed will deserialize it
/// as `None` (via `serde(default)`) without error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthPromptChallengeKind {
    /// Browser-based OAuth relay challenge. When `authorization_url` is present,
    /// the browser can open it in a new tab and wait for the OAuth callback to
    /// resume the run server-side. When the provider is unavailable or
    /// unconfigured, the URL may be absent so UI can still render an
    /// OAuth-specific unavailable state instead of the generic auth fallback.
    ///
    /// Wire value is `oauth_url` (for browser OAuth). The challenge kind is
    /// always re-derived at projection time from the persisted credential
    /// setup, never deserialized back from the wire.
    #[serde(rename = "oauth_url")]
    OAuthUrl,
    /// User pastes a secret string into the chat form. Wire value is
    /// `manual_token` (via `rename_all = "snake_case"`): paste a credential
    /// such as a GitHub PAT or API key.
    ManualToken,
    /// Host-issued channel pairing (WebGeneratedCode direction): the UI shows
    /// a code/deep-link panel and completion happens on the EXTERNAL side
    /// (e.g. Telegram `/start <code>`), then the run resumes server-side.
    /// Nothing is pasted into IronClaw. Wire value is `pairing`.
    Pairing,
    /// Other challenge kind (account selection, setup required, reauthorize).
    /// The UI should fall back to a generic "authentication required" card.
    Other,
}

/// Manifest-derived connection context for a channel authentication challenge.
/// The strategy selects presentation while `channel` identifies the generic
/// connection route. Additive + serde-default so older rows deserialize as
/// `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConnectionPromptContext {
    /// Connectable channel id (e.g. `telegram`).
    pub channel: String,
    /// Connect strategy wire value (e.g. `web_generated_code`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    /// Backend-authored connect instructions for the pairing card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Placeholder for the paste input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_placeholder: Option<String>,
    /// Submit button label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_label: Option<String>,
    /// Error copy shown when the pasted code is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Host-issued pairing affordance for a `Pairing` auth challenge. The code,
/// deep link, expiry, and copy all come from the same manifest-driven pairing
/// service used by WebUI; channel adapters only choose native formatting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingPromptView {
    pub channel: String,
    pub display_name: String,
    pub instructions: String,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPromptView {
    pub turn_run_id: TurnRunId,
    pub auth_request_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<InvocationId>,
    pub headline: String,
    pub body: String,
    /// Challenge kind — present when the projection layer has auth-flow
    /// metadata available for this gate. Absent on rows written before #4112.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_kind: Option<AuthPromptChallengeKind>,
    /// Short provider id (e.g. `"google"`, `"github"`, `"notion"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Human-readable account label (e.g. `"work@example.com"`, `"GitHub PAT"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    /// Opaque IDP authorization URL. Only present for `OAuthUrl` challenges.
    /// This is the same URL already surfaced in the legacy
    /// `AppEvent::OnboardingState.auth_url` field — safe to render in the
    /// browser. Never contains a PKCE verifier, client secret, or token.
    ///
    /// Upstream projection converts this from validated `OAuthAuthorizationUrl`;
    /// the DTO stores a `String` only to preserve the stable JSON wire shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    /// Challenge expiry. Present when the auth flow has a bounded TTL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Manifest-derived channel connection context. Presentation is selected
    /// by `challenge_kind` and `connection.strategy`, never by provider name.
    /// Additive + serde-default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<ConnectionPromptContext>,
    /// Host-issued WebGeneratedCode presentation. Present only when
    /// `challenge_kind == Pairing` and the target is authorized to receive the
    /// bearer challenge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing: Option<PairingPromptView>,
}

/// Render one structured auth challenge for text-based channel adapters.
/// Recipe selection and challenge materialization happened upstream; this
/// helper only formats the already-typed view without naming a provider.
pub fn render_channel_auth_prompt(view: &AuthPromptView, direct_message: bool) -> String {
    let body = view
        .pairing
        .as_ref()
        .map(|pairing| pairing.instructions.as_str())
        .unwrap_or(view.body.as_str());
    let mut text = format!("{}\n\n{}", view.headline, body);
    if let Some(pairing) = view.pairing.as_ref() {
        text.push_str("\n\nPairing code: `");
        text.push_str(&pairing.code);
        text.push('`');
        if let Some(deep_link) = pairing.deep_link.as_deref() {
            text.push_str("\n\nOpen ");
            text.push_str(&pairing.display_name);
            text.push_str(": ");
            text.push_str(deep_link);
        }
        text.push_str("\n\nExpires: ");
        text.push_str(&pairing.expires_at.to_rfc3339());
    }
    text.push_str("\n\n");
    if direct_message {
        text.push_str("Reply `auth deny ");
        text.push_str(&view.auth_request_ref);
        text.push_str("` here to cancel this run.");
    } else {
        text.push_str("Mention me with `auth deny ");
        text.push_str(&view.auth_request_ref);
        text.push_str("` in this thread to cancel this run.");
    }
    if let Some(url) = view.authorization_url.as_deref() {
        text.push_str("\n\nSetup link: ");
        text.push_str(url);
    }
    text
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthPromptContextView {
    pub challenge_kind: AuthPromptChallengeKind,
    /// Short provider id (e.g. `"google"`, `"github"`, `"notion"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Human-readable account label (e.g. `"work@example.com"`, `"GitHub PAT"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    /// Opaque IDP authorization URL. Only present for `OAuthUrl` challenges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    /// Challenge expiry. Present when the auth flow has a bounded TTL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Channel-pairing connection context — see [`AuthPromptView::connection`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<ConnectionPromptContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing: Option<PairingPromptView>,
}

impl AuthPromptContextView {
    pub fn new(
        challenge_kind: AuthPromptChallengeKind,
        provider: Option<String>,
        account_label: Option<String>,
        authorization_url: Option<String>,
        expires_at: Option<DateTime<Utc>>,
        connection: Option<ConnectionPromptContext>,
    ) -> Result<Self, ProductAdapterError> {
        Self::new_with_pairing(
            challenge_kind,
            provider,
            account_label,
            authorization_url,
            expires_at,
            connection,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_pairing(
        challenge_kind: AuthPromptChallengeKind,
        provider: Option<String>,
        account_label: Option<String>,
        authorization_url: Option<String>,
        expires_at: Option<DateTime<Utc>>,
        connection: Option<ConnectionPromptContext>,
        pairing: Option<PairingPromptView>,
    ) -> Result<Self, ProductAdapterError> {
        let view = Self {
            challenge_kind,
            provider,
            account_label,
            authorization_url,
            expires_at,
            connection,
            pairing,
        };
        view.validate()?;
        Ok(view)
    }

    pub fn from_auth_prompt(prompt: &AuthPromptView) -> Result<Option<Self>, ProductAdapterError> {
        let Some(challenge_kind) = prompt.challenge_kind else {
            return Ok(None);
        };
        Self::new_with_pairing(
            challenge_kind,
            prompt.provider.clone(),
            prompt.account_label.clone(),
            prompt.authorization_url.clone(),
            prompt.expires_at,
            prompt.connection.clone(),
            prompt.pairing.clone(),
        )
        .map(Some)
    }

    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_optional_display_text(
            "auth_prompt_provider",
            self.provider.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "auth_prompt_account_label",
            self.account_label.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "auth_prompt_authorization_url",
            self.authorization_url.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        if let Some(connection) = self.connection.as_ref() {
            connection.validate()?;
        }
        if let Some(pairing) = self.pairing.as_ref() {
            pairing.validate()?;
        }
        Ok(())
    }
}

impl PairingPromptView {
    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_bounded_text(
            "pairing_prompt_channel",
            &self.channel,
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_bounded_text(
            "pairing_prompt_display_name",
            &self.display_name,
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_bounded_text(
            "pairing_prompt_instructions",
            &self.instructions,
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_bounded_text(
            "pairing_prompt_code",
            &self.code,
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "pairing_prompt_deep_link",
            self.deep_link.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )
    }
}

impl ConnectionPromptContext {
    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_optional_display_text(
            "connection_channel",
            Some(self.channel.as_str()),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "connection_strategy",
            self.strategy.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "connection_instructions",
            self.instructions.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "connection_input_placeholder",
            self.input_placeholder.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "connection_submit_label",
            self.submit_label.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "connection_error_message",
            self.error_message.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )
    }
}

impl<'de> Deserialize<'de> for AuthPromptContextView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            challenge_kind: AuthPromptChallengeKind,
            #[serde(default)]
            provider: Option<String>,
            #[serde(default)]
            account_label: Option<String>,
            #[serde(default)]
            authorization_url: Option<String>,
            #[serde(default)]
            expires_at: Option<DateTime<Utc>>,
            #[serde(default)]
            connection: Option<ConnectionPromptContext>,
            #[serde(default)]
            pairing: Option<PairingPromptView>,
        }

        let wire = Wire::deserialize(deserializer)?;
        AuthPromptContextView::new_with_pairing(
            wire.challenge_kind,
            wire.provider,
            wire.account_label,
            wire.authorization_url,
            wire.expires_at,
            wire.connection,
            wire.pairing,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductGateKind {
    Approval,
    Auth,
    Resource,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductProjectionItem {
    Text {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<TurnRunId>,
        body: String,
    },
    Thinking {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        run_id: Option<TurnRunId>,
        body: String,
    },
    CapabilityActivity(CapabilityActivityView),
    WorkSummary {
        id: String,
        run_id: TurnRunId,
        phase: ProductWorkSummaryPhase,
        body: String,
    },
    RunStatus {
        run_id: TurnRunId,
        status: String,
        /// Sanitized, opaque product category. Projection sources may use
        /// different internal namespaces; clients should not parse this for
        /// user-facing copy and should prefer `failure_summary` when present.
        #[serde(
            skip_serializing_if = "Option::is_none",
            serialize_with = "serialize_failure_category"
        )]
        failure_category: Option<SanitizedFailure>,
        /// User-facing sanitized explanation for terminal failure states.
        #[serde(skip_serializing_if = "Option::is_none")]
        failure_summary: Option<String>,
        /// Present only for failed runs: whether the run recorded a resumable
        /// checkpoint and can be retried via the run retry endpoint.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retryable: Option<bool>,
    },
    Gate {
        run_id: TurnRunId,
        gate_kind: ProductGateKind,
        gate_ref: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        invocation_id: Option<InvocationId>,
        headline: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<String>,
        #[serde(default)]
        allow_always: bool,
        /// Auth challenge context. For a `manual_token` gate whose paste is a
        /// pairing code, its `connection` field carries the channel-connection
        /// render copy + resolve route — the single canonical place the
        /// projection-gate consumer reads it (no duplicate top-level field, per
        /// the wire-contract no-duplicate-fields rule).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth_context: Option<Box<AuthPromptContextView>>,
    },
    SkillActivation {
        id: String,
        run_id: TurnRunId,
        skill_names: Vec<String>,
        feedback: Vec<String>,
    },
}

impl ProductProjectionItem {
    fn validate(&self) -> Result<(), ProductAdapterError> {
        match self {
            Self::Text { id, body, .. } | Self::Thinking { id, body, .. } => {
                validate_bounded_text("projection_item_id", id, PROJECTION_ITEM_ID_MAX_BYTES)?;
                validate_bounded_text("projection_text", body, PROJECTION_TEXT_MAX_BYTES)
            }
            Self::CapabilityActivity(activity) => activity.validate(),
            Self::WorkSummary { id, body, .. } => {
                validate_bounded_text("projection_item_id", id, PROJECTION_ITEM_ID_MAX_BYTES)?;
                validate_bounded_text(
                    "projection_work_summary",
                    body,
                    PROJECTION_WORK_SUMMARY_MAX_BYTES,
                )
            }
            Self::RunStatus {
                status,
                failure_category: _,
                failure_summary,
                retryable,
                ..
            } => {
                validate_bounded_text(
                    "projection_run_status",
                    status,
                    PROJECTION_ITEM_ID_MAX_BYTES,
                )?;
                if retryable.is_some() && status != "failed" {
                    return Err(invalid(
                        "projection_run_retryable",
                        "retryable is only valid for failed run status entries",
                    ));
                }
                if let Some(summary) = failure_summary {
                    validate_bounded_text(
                        "projection_failure_summary",
                        summary,
                        PROJECTION_TEXT_MAX_BYTES,
                    )?;
                }
                Ok(())
            }
            Self::Gate {
                gate_ref,
                headline,
                body,
                auth_context,
                ..
            } => {
                validate_bounded_text(
                    "projection_gate_ref",
                    gate_ref,
                    PROJECTION_ITEM_ID_MAX_BYTES,
                )?;
                validate_bounded_text(
                    "projection_gate_headline",
                    headline,
                    PROJECTION_TEXT_MAX_BYTES,
                )?;
                if let Some(body) = body {
                    validate_bounded_text("projection_gate_body", body, PROJECTION_TEXT_MAX_BYTES)?;
                }
                if let Some(context) = auth_context {
                    context.validate()?;
                }
                Ok(())
            }
            Self::SkillActivation {
                id,
                skill_names,
                feedback,
                ..
            } => {
                validate_bounded_text("projection_item_id", id, PROJECTION_ITEM_ID_MAX_BYTES)?;
                if skill_names.len() > PROJECTION_SKILL_ACTIVATION_MAX_ITEMS {
                    return Err(invalid(
                        "projection_skill_names",
                        "too many activated skills",
                    ));
                }
                if feedback.len() > PROJECTION_SKILL_ACTIVATION_MAX_ITEMS {
                    return Err(invalid(
                        "projection_skill_feedback",
                        "too many skill activation feedback entries",
                    ));
                }
                for skill_name in skill_names {
                    validate_bounded_text(
                        "projection_skill_name",
                        skill_name,
                        PROJECTION_SKILL_NAME_MAX_BYTES,
                    )?;
                }
                for note in feedback {
                    validate_bounded_text(
                        "projection_skill_feedback",
                        note,
                        PROJECTION_SKILL_FEEDBACK_MAX_BYTES,
                    )?;
                }
                Ok(())
            }
        }
    }
}

impl<'de> Deserialize<'de> for ProductProjectionItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Wire {
            Text {
                id: String,
                #[serde(default)]
                run_id: Option<TurnRunId>,
                body: String,
            },
            Thinking {
                id: String,
                #[serde(default)]
                run_id: Option<TurnRunId>,
                body: String,
            },
            CapabilityActivity(CapabilityActivityView),
            WorkSummary {
                id: String,
                run_id: TurnRunId,
                phase: ProductWorkSummaryPhase,
                body: String,
            },
            RunStatus {
                run_id: TurnRunId,
                status: String,
                #[serde(default)]
                failure_category: Option<String>,
                #[serde(default)]
                failure_summary: Option<String>,
                #[serde(default)]
                retryable: Option<bool>,
            },
            Gate {
                run_id: TurnRunId,
                gate_kind: ProductGateKind,
                gate_ref: String,
                #[serde(default)]
                invocation_id: Option<InvocationId>,
                headline: String,
                #[serde(default)]
                body: Option<String>,
                #[serde(default)]
                allow_always: bool,
                #[serde(default)]
                auth_context: Option<Box<AuthPromptContextView>>,
            },
            SkillActivation {
                id: String,
                run_id: TurnRunId,
                skill_names: Vec<String>,
                feedback: Vec<String>,
            },
        }
        let value = match Wire::deserialize(deserializer)? {
            Wire::Text { id, run_id, body } => ProductProjectionItem::Text { id, run_id, body },
            Wire::Thinking { id, run_id, body } => {
                ProductProjectionItem::Thinking { id, run_id, body }
            }
            Wire::CapabilityActivity(activity) => {
                ProductProjectionItem::CapabilityActivity(activity)
            }
            Wire::WorkSummary {
                id,
                run_id,
                phase,
                body,
            } => ProductProjectionItem::WorkSummary {
                id,
                run_id,
                phase,
                body,
            },
            Wire::RunStatus {
                run_id,
                status,
                failure_category,
                failure_summary,
                retryable,
            } => ProductProjectionItem::RunStatus {
                run_id,
                status,
                failure_category: failure_category
                    .map(SanitizedFailure::new)
                    .transpose()
                    .map_err(serde::de::Error::custom)?,
                failure_summary,
                retryable,
            },
            Wire::Gate {
                run_id,
                gate_kind,
                gate_ref,
                invocation_id,
                headline,
                body,
                allow_always,
                auth_context,
            } => ProductProjectionItem::Gate {
                run_id,
                gate_kind,
                gate_ref,
                invocation_id,
                headline,
                body,
                allow_always,
                auth_context,
            },
            Wire::SkillActivation {
                id,
                run_id,
                skill_names,
                feedback,
            } => ProductProjectionItem::SkillActivation {
                id,
                run_id,
                skill_names,
                feedback,
            },
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProductProjectionState {
    pub thread_id: String,
    pub items: Vec<ProductProjectionItem>,
}

impl ProductProjectionState {
    pub fn new(
        thread_id: impl Into<String>,
        items: Vec<ProductProjectionItem>,
    ) -> Result<Self, ProductAdapterError> {
        let thread_id = thread_id.into();
        validate_bounded_text(
            "projection_thread_id",
            &thread_id,
            PROJECTION_THREAD_ID_MAX_BYTES,
        )?;
        if items.is_empty() {
            return Err(invalid("projection_items", "must include renderable state"));
        }
        for item in &items {
            item.validate()?;
        }
        Ok(Self { thread_id, items })
    }
}

impl<'de> Deserialize<'de> for ProductProjectionState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            thread_id: String,
            items: Vec<ProductProjectionItem>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.thread_id, wire.items).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductOutboundPayload {
    FinalReply(FinalReplyView),
    Progress(ProgressUpdateView),
    CapabilityActivity(CapabilityActivityView),
    CapabilityDisplayPreview(CapabilityDisplayPreviewView),
    GatePrompt(GatePromptView),
    AuthPrompt(AuthPromptView),
    ProjectionSnapshot { state: ProductProjectionState },
    ProjectionUpdate { state: ProductProjectionState },
    KeepAlive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductOutboundTarget {
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub external_actor_ref: Option<ExternalActorRef>,
}

impl ProductOutboundTarget {
    pub fn new(
        reply_target_binding_ref: ReplyTargetBindingRef,
        external_conversation_ref: ExternalConversationRef,
        external_actor_ref: Option<ExternalActorRef>,
    ) -> Self {
        Self {
            reply_target_binding_ref,
            external_conversation_ref,
            external_actor_ref,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductSynchronousResponse {
    pub content_type: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductRenderOutcome {
    DeliveryRecorded,
    SynchronousResponse(ProductSynchronousResponse),
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductOutboundEnvelope {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub target: ProductOutboundTarget,
    pub projection_cursor: ProjectionCursor,
    pub payload: ProductOutboundPayload,
    pub delivery_attempt_id: Uuid,
}

impl ProductOutboundEnvelope {
    pub fn new(
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        target: ProductOutboundTarget,
        projection_cursor: ProjectionCursor,
        payload: ProductOutboundPayload,
    ) -> Self {
        Self {
            adapter_id,
            installation_id,
            target,
            projection_cursor,
            payload,
            delivery_attempt_id: Uuid::new_v4(),
        }
    }

    pub fn projection_cursor(&self) -> &ProjectionCursor {
        &self.projection_cursor
    }

    pub fn payload(&self) -> &ProductOutboundPayload {
        &self.payload
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_prompt_challenge_kind_all_variants_roundtrip() {
        // Stable wire values: browser OAuth, pasted credentials, and
        // host-issued channel pairing.
        for (variant, expected) in [
            (AuthPromptChallengeKind::OAuthUrl, "\"oauth_url\""),
            (AuthPromptChallengeKind::ManualToken, "\"manual_token\""),
            (AuthPromptChallengeKind::Pairing, "\"pairing\""),
            (AuthPromptChallengeKind::Other, "\"other\""),
        ] {
            let serialized = serde_json::to_string(&variant).expect("serialize challenge kind");
            assert_eq!(serialized, expected);
            let decoded: AuthPromptChallengeKind =
                serde_json::from_str(&serialized).expect("deserialize challenge kind");
            assert_eq!(decoded, variant);
        }
    }

    #[test]
    fn auth_prompt_context_round_trips_channel_connection() {
        // A host-issued pairing challenge carries manifest-derived connection
        // context. Round-trip it through the wire.
        let run_id = TurnRunId::new();
        let state = ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::Gate {
                run_id,
                gate_kind: ProductGateKind::Auth,
                gate_ref: "gate:connect-slack".to_string(),
                invocation_id: None,
                headline: "Connect Slack".to_string(),
                body: Some("Message the app to get a pairing code.".to_string()),
                allow_always: false,
                auth_context: Some(Box::new(
                    AuthPromptContextView::new(
                        AuthPromptChallengeKind::Pairing,
                        Some("telegram".to_string()),
                        None,
                        None,
                        None,
                        Some(ConnectionPromptContext {
                            channel: "telegram".to_string(),
                            strategy: Some("web_generated_code".to_string()),
                            instructions: Some("Open the app with the generated code.".to_string()),
                            input_placeholder: None,
                            submit_label: Some("Connect".to_string()),
                            error_message: Some("Invalid or expired pairing code.".to_string()),
                        }),
                    )
                    .expect("valid connection auth context"),
                )),
            }],
        )
        .expect("valid connection gate projection");
        let value = serde_json::to_value(&state).expect("serialize");

        // Connection context rides the gate's auth_context (single canonical
        // place); there is no duplicate top-level Gate.connection field.
        assert_eq!(
            value["items"][0]["gate"]["auth_context"]["challenge_kind"],
            "pairing"
        );
        assert_eq!(
            value["items"][0]["gate"]["auth_context"]["connection"]["channel"],
            "telegram"
        );
        assert!(
            value["items"][0]["gate"]["auth_context"]["connection"]
                .get("input_placeholder")
                .is_none()
        );
        let decoded: ProductProjectionState =
            serde_json::from_value(value).expect("deserialize connection gate projection");
        assert_eq!(decoded, state);
    }

    #[test]
    fn cursor_round_trips() {
        let cursor = ProjectionCursor::new("thread:42#cursor:7").expect("valid");
        let json = serde_json::to_string(&cursor).expect("serialize");
        let parsed: ProjectionCursor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(cursor, parsed);
    }

    #[test]
    fn cursor_rejects_oversize() {
        assert!(ProjectionCursor::new("a".repeat(PROJECTION_CURSOR_MAX_BYTES + 1)).is_err());
    }

    #[test]
    fn projection_state_requires_renderable_items() {
        assert!(ProductProjectionState::new("thread-1", vec![]).is_err());
    }

    #[test]
    fn projection_state_round_trips_thinking_item() {
        let run_id = TurnRunId::new();
        let state = ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::Thinking {
                id: "thinking:run:1".to_string(),
                run_id: Some(run_id),
                body: "checking context".to_string(),
            }],
        )
        .expect("valid thinking projection");
        let value = serde_json::to_value(&state).expect("serialize");
        assert_eq!(value["items"][0]["thinking"]["run_id"], run_id.to_string());
        assert_eq!(value["items"][0]["thinking"]["body"], "checking context");
        let decoded: ProductProjectionState =
            serde_json::from_value(value).expect("deserialize thinking projection");
        assert_eq!(decoded, state);
    }

    #[test]
    fn projection_state_round_trips_capability_activity_item() {
        let invocation_id = InvocationId::new();
        let state = ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::CapabilityActivity(
                CapabilityActivityView::new(CapabilityActivityViewInput {
                    invocation_id,
                    turn_run_id: None,
                    thread_id: Some(ThreadId::new("thread-1").unwrap()),
                    capability_id: CapabilityId::new("builtin.http").unwrap(),
                    status: CapabilityActivityStatusView::Started,
                    provider: None,
                    runtime: None,
                    process_id: None,
                    output_bytes: None,
                    error_kind: None,
                    error_detail: None,
                    subtitle: None,
                    input_summary: None,
                    updated_at: Utc::now(),
                    activity_order: None,
                })
                .expect("valid capability activity"),
            )],
        )
        .expect("valid capability activity projection");
        let value = serde_json::to_value(&state).expect("serialize");
        assert_eq!(
            value["items"][0]["capability_activity"]["capability_id"],
            "builtin.http"
        );
        assert_eq!(
            value["items"][0]["capability_activity"]["status"],
            "started"
        );
        let decoded: ProductProjectionState =
            serde_json::from_value(value).expect("deserialize capability activity projection");
        assert_eq!(decoded, state);
    }

    #[test]
    fn projection_state_round_trips_work_summary_item() {
        let run_id = TurnRunId::new();
        let state = ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::WorkSummary {
                id: "work-summary:run:1".to_string(),
                run_id,
                phase: ProductWorkSummaryPhase::Planning,
                body: "checking branch state".to_string(),
            }],
        )
        .expect("valid work summary projection");
        let value = serde_json::to_value(&state).expect("serialize");
        assert_eq!(
            value["items"][0]["work_summary"]["body"],
            "checking branch state"
        );
        assert_eq!(value["items"][0]["work_summary"]["phase"], "planning");
        let decoded: ProductProjectionState =
            serde_json::from_value(value).expect("deserialize work summary projection");
        assert_eq!(decoded, state);
    }

    #[test]
    fn projection_state_round_trips_run_status_failure_details() {
        let run_id = TurnRunId::new();
        let state = ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::RunStatus {
                run_id,
                status: "failed".to_string(),
                failure_category: Some(SanitizedFailure::new("lease_expired").unwrap()),
                failure_summary: Some(
                    "The run failed because its runner lease expired.".to_string(),
                ),
                retryable: None,
            }],
        )
        .expect("valid run status projection");
        let value = serde_json::to_value(&state).expect("serialize");
        assert_eq!(
            value["items"][0]["run_status"]["failure_category"],
            "lease_expired"
        );
        assert_eq!(
            value["items"][0]["run_status"]["failure_summary"],
            "The run failed because its runner lease expired."
        );
        let decoded: ProductProjectionState =
            serde_json::from_value(value).expect("deserialize run status projection");
        assert_eq!(decoded, state);
    }

    #[test]
    fn projection_state_allows_retryable_failed_run_status() {
        ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::RunStatus {
                run_id: TurnRunId::new(),
                status: "failed".to_string(),
                failure_category: None,
                failure_summary: None,
                retryable: Some(true),
            }],
        )
        .expect("failed run status may carry retryability");
    }

    #[test]
    fn projection_state_rejects_retryable_non_failed_run_status() {
        let error = ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::RunStatus {
                run_id: TurnRunId::new(),
                status: "running".to_string(),
                failure_category: None,
                failure_summary: None,
                retryable: Some(true),
            }],
        )
        .expect_err("running run status must not carry retryability");

        assert!(
            matches!(
                error,
                ProductAdapterError::InvalidIdentifier {
                    kind: "projection_run_retryable",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn projection_state_accepts_legacy_run_status_without_failure_details() {
        let run_id = TurnRunId::new();
        let json = serde_json::json!({
            "thread_id": "thread-1",
            "items": [{
                "run_status": {
                    "run_id": run_id,
                    "status": "failed"
                }
            }]
        });

        let decoded: ProductProjectionState =
            serde_json::from_value(json).expect("deserialize legacy run status projection");
        assert_eq!(
            decoded.items,
            vec![ProductProjectionItem::RunStatus {
                run_id,
                status: "failed".to_string(),
                failure_category: None,
                failure_summary: None,
                retryable: None,
            }]
        );
    }

    #[test]
    fn projection_state_round_trips_gate_item_with_stable_identity() {
        let run_id = TurnRunId::new();
        let invocation_id = InvocationId::new();
        let state = ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::Gate {
                run_id,
                gate_kind: ProductGateKind::Approval,
                gate_ref: "gate:approval-test".to_string(),
                invocation_id: Some(invocation_id),
                headline: "Approval required".to_string(),
                body: Some("capability requires approval".to_string()),
                allow_always: true,
                auth_context: None,
            }],
        )
        .expect("valid gate projection");
        let value = serde_json::to_value(&state).expect("serialize");

        assert_eq!(value["items"][0]["gate"]["run_id"], run_id.to_string());
        assert_eq!(value["items"][0]["gate"]["gate_kind"], "approval");
        assert_eq!(value["items"][0]["gate"]["gate_ref"], "gate:approval-test");
        assert_eq!(
            value["items"][0]["gate"]["invocation_id"],
            invocation_id.to_string()
        );
        assert_eq!(value["items"][0]["gate"]["headline"], "Approval required");
        assert_eq!(
            value["items"][0]["gate"]["body"],
            "capability requires approval"
        );
        assert_eq!(value["items"][0]["gate"]["allow_always"], true);
        let decoded: ProductProjectionState =
            serde_json::from_value(value).expect("deserialize gate projection");
        assert_eq!(decoded, state);
    }

    #[test]
    fn projection_state_round_trips_auth_gate_context() {
        let run_id = TurnRunId::new();
        let state = ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::Gate {
                run_id,
                gate_kind: ProductGateKind::Auth,
                gate_ref: "gate:auth-test".to_string(),
                invocation_id: None,
                headline: "Authentication required".to_string(),
                body: Some("Authenticate to continue this run.".to_string()),
                allow_always: false,
                auth_context: Some(Box::new(
                    AuthPromptContextView::new(
                        AuthPromptChallengeKind::OAuthUrl,
                        Some("github".to_string()),
                        None,
                        Some("https://github.com/login/oauth/authorize".to_string()),
                        None,
                        None,
                    )
                    .expect("valid auth context"),
                )),
            }],
        )
        .expect("valid auth gate projection");
        let value = serde_json::to_value(&state).expect("serialize");

        assert_eq!(
            value["items"][0]["gate"]["auth_context"]["challenge_kind"],
            "oauth_url"
        );
        assert_eq!(
            value["items"][0]["gate"]["auth_context"]["provider"],
            "github"
        );
        assert_eq!(
            value["items"][0]["gate"]["auth_context"]["authorization_url"],
            "https://github.com/login/oauth/authorize"
        );
        let decoded: ProductProjectionState =
            serde_json::from_value(value).expect("deserialize auth gate projection");
        assert_eq!(decoded, state);
    }

    #[test]
    fn auth_prompt_context_from_prompt_rejects_invalid_prompt_context() {
        let prompt = AuthPromptView {
            turn_run_id: TurnRunId::new(),
            auth_request_ref: "gate:auth-test".to_string(),
            invocation_id: None,
            headline: "Authentication required".to_string(),
            body: "Authenticate to continue this run.".to_string(),
            challenge_kind: Some(AuthPromptChallengeKind::OAuthUrl),
            provider: Some("github".to_string()),
            account_label: None,
            authorization_url: Some("x".repeat(PROJECTION_TEXT_MAX_BYTES + 1)),
            expires_at: None,
            connection: None,
            pairing: None,
        };

        assert!(AuthPromptContextView::from_auth_prompt(&prompt).is_err());
    }

    #[test]
    fn projection_state_rejects_gate_item_without_run_identity() {
        let json = serde_json::json!({
            "thread_id": "thread-1",
            "items": [{
                "gate": {
                    "gate_kind": "approval",
                    "gate_ref": "gate:approval-test",
                    "headline": "Approval required"
                }
            }]
        });

        assert!(serde_json::from_value::<ProductProjectionState>(json).is_err());
    }

    #[test]
    fn gate_prompt_view_round_trips_approval_context() {
        let view = GatePromptView {
            turn_run_id: TurnRunId::new(),
            gate_ref: "gate:approval-test".to_string(),
            invocation_id: Some(InvocationId::new()),
            headline: "Approval required".to_string(),
            body: "capability requires approval".to_string(),
            allow_always: true,
            approval_context: Some(
                ApprovalPromptContextView::new(
                    "builtin.http",
                    ApprovalPromptActionView::new("Network request", Some(NetworkMethod::Post))
                        .expect("valid action"),
                    ApprovalPromptScopeView::new("Reusable grant", true).expect("valid scope"),
                    Some("capability requires approval".to_string()),
                    Some(
                        ApprovalPromptDestinationView::new(
                            "POST https://example.com",
                            Some("https://example.com".to_string()),
                            Some("example.com".to_string()),
                        )
                        .expect("valid destination"),
                    ),
                    vec![
                        ApprovalPromptDetailView::new("Method", "POST").expect("valid detail"),
                        ApprovalPromptDetailView::new("Estimated transfer", "4096 bytes")
                            .expect("valid detail"),
                    ],
                )
                .expect("valid approval context"),
            ),
        };

        let value = serde_json::to_value(&view).expect("serialize");
        assert_eq!(
            value["approval_context"]["action"]["method"],
            serde_json::json!("post")
        );
        assert_eq!(
            value["approval_context"]["destination"]["domain"],
            "example.com"
        );

        let decoded: GatePromptView =
            serde_json::from_value(value).expect("deserialize approval prompt");
        assert_eq!(decoded, view);
    }

    #[test]
    fn approval_prompt_context_rejects_oversized_display_text() {
        let json = serde_json::json!({
            "tool_name": "x".repeat(APPROVAL_PROMPT_TEXT_MAX_BYTES + 1),
            "action": {
                "label": "Run tool"
            },
            "scope": {
                "label": "This request only",
                "reusable": false
            },
            "details": []
        });

        assert!(serde_json::from_value::<ApprovalPromptContextView>(json).is_err());
    }

    #[test]
    fn approval_prompt_context_rejects_excessive_details() {
        let details = (0..=APPROVAL_PROMPT_DETAIL_MAX_ITEMS)
            .map(|index| {
                serde_json::json!({
                    "label": format!("Detail {index}"),
                    "value": "value"
                })
            })
            .collect::<Vec<_>>();
        let json = serde_json::json!({
            "tool_name": "builtin.http",
            "action": {
                "label": "Run tool"
            },
            "scope": {
                "label": "This request only",
                "reusable": false
            },
            "details": details
        });

        assert!(serde_json::from_value::<ApprovalPromptContextView>(json).is_err());
    }

    #[test]
    fn projection_state_round_trips_skill_activation_item() {
        let run_id = TurnRunId::new();
        let state = ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::SkillActivation {
                id: "skill-activation:run:1".to_string(),
                run_id,
                skill_names: vec!["code-review".to_string()],
                feedback: vec!["code-review: force-activated via explicit mention".to_string()],
            }],
        )
        .expect("valid skill activation projection");
        let value = serde_json::to_value(&state).expect("serialize");
        assert_eq!(
            value["items"][0]["skill_activation"]["skill_names"][0],
            "code-review"
        );
        let decoded: ProductProjectionState =
            serde_json::from_value(value).expect("deserialize skill activation projection");
        assert_eq!(decoded, state);
    }

    #[test]
    fn projection_state_rejects_oversized_work_summary_body() {
        let json = serde_json::json!({
            "thread_id": "thread-1",
            "items": [{
                "work_summary": {
                    "id": "work-summary:run:1",
                    "run_id": TurnRunId::new(),
                    "phase": "planning",
                    "body": "x".repeat(PROJECTION_WORK_SUMMARY_MAX_BYTES + 1),
                }
            }]
        });

        assert!(serde_json::from_value::<ProductProjectionState>(json).is_err());
    }

    #[test]
    fn projection_state_rejects_oversized_run_status_failure_details() {
        let oversized_category = serde_json::json!({
            "thread_id": "thread-1",
            "items": [{
                "run_status": {
                    "run_id": TurnRunId::new(),
                    "status": "failed",
                    "failure_category": "x".repeat(PROJECTION_ITEM_ID_MAX_BYTES + 1),
                    "failure_summary": "The run failed."
                }
            }]
        });
        assert!(serde_json::from_value::<ProductProjectionState>(oversized_category).is_err());

        let oversized_summary = serde_json::json!({
            "thread_id": "thread-1",
            "items": [{
                "run_status": {
                    "run_id": TurnRunId::new(),
                    "status": "failed",
                    "failure_category": "driver_failed",
                    "failure_summary": "x".repeat(PROJECTION_TEXT_MAX_BYTES + 1)
                }
            }]
        });
        assert!(serde_json::from_value::<ProductProjectionState>(oversized_summary).is_err());
    }

    #[test]
    fn final_reply_serializes_with_plaintext() {
        let view = FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hello world".into(),
            generated_at: Utc::now(),
        };
        let json = serde_json::to_value(&view).expect("serialize");
        assert_eq!(json["text"], "hello world");
    }

    #[test]
    fn capability_activity_view_is_metadata_only() {
        let run_id = TurnRunId::new();
        let view = CapabilityActivityView::new(CapabilityActivityViewInput {
            invocation_id: InvocationId::new(),
            turn_run_id: Some(run_id),
            thread_id: Some(ThreadId::new("thread-tool-activity").expect("thread id")),
            capability_id: CapabilityId::new("script.echo").expect("capability id"),
            status: CapabilityActivityStatusView::Completed,
            provider: Some(ExtensionId::new("script").expect("provider id")),
            runtime: Some(RuntimeKind::Script),
            process_id: None,
            output_bytes: Some(12),
            error_kind: None,
            error_detail: None,
            subtitle: None,
            input_summary: None,
            updated_at: Utc::now(),
            activity_order: None,
        })
        .expect("valid activity");
        let json = serde_json::to_value(&view).expect("serialize");
        let rendered = serde_json::to_string(&json).expect("render");

        assert_eq!(json["status"], "completed");
        assert_eq!(json["turn_run_id"], run_id.to_string());
        assert_eq!(json["output_bytes"], 12);
        for forbidden in [
            "arguments",
            "args",
            "result",
            "raw_output",
            "command",
            "host_path",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "capability activity leaked raw field name: {forbidden}"
            );
        }
    }

    #[test]
    fn capability_display_preview_view_allows_bounded_display_material() {
        let run_id = TurnRunId::new();
        let view = CapabilityDisplayPreviewView::new(CapabilityDisplayPreviewViewInput {
            timeline_message_id: Some("timeline-message-1".to_string()),
            invocation_id: InvocationId::new(),
            turn_run_id: Some(run_id),
            thread_id: Some(ThreadId::new("thread-tool-preview").expect("thread id")),
            capability_id: CapabilityId::new("builtin.read_file").expect("capability id"),
            status: CapabilityActivityStatusView::Completed,
            error_kind: None,
            title: "read_file".to_string(),
            subtitle: Some("src/main.rs".to_string()),
            input_summary: Some("path: src/main.rs".to_string()),
            output_summary: Some("read file".to_string()),
            output_preview: Some("fn main() {}".to_string()),
            output_kind: Some("text".to_string()),
            output_bytes: Some(12),
            result_ref: Some("result:tool-output".to_string()),
            truncated: false,
            updated_at: Utc::now(),
            activity_order: None,
        })
        .expect("valid preview");

        let json = serde_json::to_value(&view).expect("serialize");
        assert_eq!(json["title"], "read_file");
        assert_eq!(json["turn_run_id"], run_id.to_string());
        assert_eq!(json["subtitle"], "src/main.rs");
        assert_eq!(json["output_kind"], "text");
    }

    #[test]
    fn capability_display_preview_view_round_trips_error_kind() {
        let json = serde_json::json!({
            "invocation_id": InvocationId::new(),
            "thread_id": "thread-tool-preview",
            "capability_id": "builtin.extension_install",
            "status": "failed",
            "error_kind": "gate_declined",
            "title": "extension_install",
            "output_summary": "tool failed: gate_declined",
            "output_preview": "tool failed: gate_declined",
            "output_kind": "text",
            "truncated": false,
            "updated_at": Utc::now(),
        });

        let view = serde_json::from_value::<CapabilityDisplayPreviewView>(json)
            .expect("preview error kind is valid");
        assert_eq!(view.error_kind.as_deref(), Some("gate_declined"));
        let serialized = serde_json::to_value(&view).expect("serialize");
        assert_eq!(serialized["error_kind"], "gate_declined");
    }

    #[test]
    fn capability_display_preview_view_rejects_unbounded_timeline_message_id() {
        let json = serde_json::json!({
            "timeline_message_id": "x".repeat(PROJECTION_ITEM_ID_MAX_BYTES + 1),
            "invocation_id": InvocationId::new(),
            "thread_id": "thread-tool-preview",
            "capability_id": "builtin.read_file",
            "status": "completed",
            "title": "read_file",
            "subtitle": "src/main.rs",
            "input_summary": "path: src/main.rs",
            "output_summary": "read file",
            "output_preview": "fn main() {}",
            "output_kind": "text",
            "output_bytes": 12,
            "result_ref": "result:tool-output",
            "truncated": false,
            "updated_at": Utc::now(),
        });

        assert!(serde_json::from_value::<CapabilityDisplayPreviewView>(json).is_err());
    }

    #[test]
    fn capability_display_preview_view_deserializes_without_timeline_message_id() {
        let json = serde_json::json!({
            "invocation_id": InvocationId::new(),
            "thread_id": "thread-tool-preview",
            "capability_id": "builtin.read_file",
            "status": "completed",
            "title": "read_file",
            "subtitle": "src/main.rs",
            "input_summary": "path: src/main.rs",
            "output_summary": "read file",
            "output_preview": "fn main() {}",
            "output_kind": "text",
            "output_bytes": 12,
            "result_ref": "result:tool-output",
            "truncated": false,
            "updated_at": Utc::now(),
        });

        let view = serde_json::from_value::<CapabilityDisplayPreviewView>(json)
            .expect("old preview payload deserializes");
        assert!(view.timeline_message_id.is_none());
    }

    #[test]
    fn capability_display_preview_view_accepts_many_preview_lines() {
        let json = serde_json::json!({
            "invocation_id": InvocationId::new(),
            "thread_id": "thread-tool-preview",
            "capability_id": "builtin.read_file",
            "status": "completed",
            "title": "read_file",
            "subtitle": "src/main.rs",
            "input_summary": "path: src/main.rs",
            "output_summary": "read file",
            "output_preview": (0..=240).map(|_| "line").collect::<Vec<_>>().join("\n"),
            "output_kind": "text",
            "output_bytes": 12,
            "result_ref": "result:tool-output",
            "truncated": true,
            "updated_at": Utc::now(),
        });

        let view =
            serde_json::from_value::<CapabilityDisplayPreviewView>(json).expect("preview is valid");
        assert!(
            view.output_preview
                .as_deref()
                .is_some_and(|preview| { preview.lines().count() == 241 })
        );
    }

    #[test]
    fn capability_display_preview_view_rejects_preview_over_byte_cap() {
        let json = serde_json::json!({
            "invocation_id": InvocationId::new(),
            "thread_id": "thread-tool-preview",
            "capability_id": "builtin.read_file",
            "status": "completed",
            "title": "read_file",
            "subtitle": "src/main.rs",
            "input_summary": "path: src/main.rs",
            "output_summary": "read file",
            "output_preview": "x".repeat(CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES + 1),
            "output_kind": "text",
            "output_bytes": 12,
            "result_ref": "result:tool-output",
            "truncated": true,
            "updated_at": Utc::now(),
        });

        assert!(serde_json::from_value::<CapabilityDisplayPreviewView>(json).is_err());
    }

    #[test]
    fn capability_activity_view_rejects_unsafe_error_kind_on_deserialize() {
        let json = serde_json::json!({
            "invocation_id": InvocationId::new(),
            "thread_id": "thread-tool-activity",
            "capability_id": "script.echo",
            "status": "failed",
            "provider": "script",
            "runtime": "script",
            "process_id": null,
            "output_bytes": null,
            "error_kind": "/tmp/private-host-path",
            "updated_at": Utc::now(),
        });

        assert!(serde_json::from_value::<CapabilityActivityView>(json).is_err());
    }

    #[test]
    fn capability_activity_view_rejects_error_kind_with_unsafe_character_after_safe_prefix() {
        let json = serde_json::json!({
            "invocation_id": InvocationId::new(),
            "thread_id": "thread-tool-activity",
            "capability_id": "script.echo",
            "status": "failed",
            "provider": "script",
            "runtime": "script",
            "process_id": null,
            "output_bytes": null,
            "error_kind": "safe/path",
            "updated_at": Utc::now(),
        });

        assert!(serde_json::from_value::<CapabilityActivityView>(json).is_err());
    }

    #[test]
    fn capability_activity_view_rejects_oversized_and_malformed_error_kind_segments() {
        for error_kind in [
            "a".repeat(CAPABILITY_ACTIVITY_ERROR_KIND_MAX_BYTES + 1),
            "missing..runtime".to_string(),
            "missing_runtime.".to_string(),
            format!(
                "aa.{}",
                "a".repeat(CAPABILITY_ACTIVITY_ERROR_KIND_SEGMENT_MAX_BYTES + 1)
            ),
        ] {
            let json = serde_json::json!({
                "invocation_id": InvocationId::new(),
                "thread_id": "thread-tool-activity",
                "capability_id": "script.echo",
                "status": "failed",
                "provider": "script",
                "runtime": "script",
                "process_id": null,
                "output_bytes": null,
                "error_kind": error_kind,
                "updated_at": Utc::now(),
            });

            assert!(serde_json::from_value::<CapabilityActivityView>(json).is_err());
        }
    }

    #[test]
    fn capability_activity_view_rejects_unsafe_error_kind_on_serialize() {
        let view = CapabilityActivityView {
            invocation_id: InvocationId::new(),
            turn_run_id: Some(TurnRunId::new()),
            thread_id: Some(ThreadId::new("thread-tool-activity").expect("thread id")),
            capability_id: CapabilityId::new("script.echo").expect("capability id"),
            status: CapabilityActivityStatusView::Failed,
            provider: Some(ExtensionId::new("script").expect("provider id")),
            runtime: Some(RuntimeKind::Script),
            process_id: None,
            output_bytes: None,
            error_kind: Some("/tmp/private-host-path".to_string()),
            error_detail: None,
            subtitle: None,
            input_summary: None,
            updated_at: Utc::now(),
            activity_order: None,
        };

        assert!(serde_json::to_value(view).is_err());
    }

    #[test]
    fn capability_activity_view_accepts_sanitized_unclassified_error_kind() {
        let json = serde_json::json!({
            "invocation_id": InvocationId::new(),
            "thread_id": "thread-tool-activity",
            "capability_id": "script.echo",
            "status": "failed",
            "provider": "script",
            "runtime": "script",
            "process_id": null,
            "output_bytes": null,
            "error_kind": CAPABILITY_ACTIVITY_UNCLASSIFIED_ERROR_KIND,
            "updated_at": Utc::now(),
        });

        let view = serde_json::from_value::<CapabilityActivityView>(json)
            .expect("sanitized fallback error kind is accepted");
        assert_eq!(
            view.error_kind.as_deref(),
            Some(CAPABILITY_ACTIVITY_UNCLASSIFIED_ERROR_KIND)
        );
    }

    fn capability_activity_view_input_for_detail_test() -> CapabilityActivityViewInput {
        CapabilityActivityViewInput {
            invocation_id: InvocationId::new(),
            turn_run_id: Some(TurnRunId::new()),
            thread_id: Some(ThreadId::new("thread-tool-activity").expect("thread id")),
            capability_id: CapabilityId::new("nearai.web_search").expect("capability id"),
            status: CapabilityActivityStatusView::Started,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: None,
            error_detail: None,
            subtitle: None,
            input_summary: None,
            updated_at: Utc::now(),
            activity_order: None,
        }
    }

    #[test]
    fn capability_activity_view_rejects_oversized_running_input_fields() {
        let oversized = "a".repeat(CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES + 1);

        let subtitle_input = CapabilityActivityViewInput {
            subtitle: Some(oversized.clone()),
            ..capability_activity_view_input_for_detail_test()
        };
        assert!(CapabilityActivityView::new(subtitle_input).is_err());

        let summary_input = CapabilityActivityViewInput {
            input_summary: Some(oversized),
            ..capability_activity_view_input_for_detail_test()
        };
        assert!(CapabilityActivityView::new(summary_input).is_err());
    }

    #[test]
    fn capability_activity_view_rejects_control_char_running_input_on_serialize() {
        let view = CapabilityActivityView {
            subtitle: Some("query\u{0}with-nul".to_string()),
            ..CapabilityActivityView::new(capability_activity_view_input_for_detail_test())
                .expect("base view")
        };

        assert!(serde_json::to_value(view).is_err());
    }
}

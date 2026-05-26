//! Outbound envelope, projection-derived payloads, and projection cursor.

use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, ProcessId, RuntimeKind, ThreadId,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::error::ProductAdapterError;
use crate::external::{ExternalActorRef, ExternalConversationRef};
use crate::identity::{AdapterInstallationId, ProductAdapterId};

const PROJECTION_CURSOR_MAX_BYTES: usize = 1024;
const PROJECTION_THREAD_ID_MAX_BYTES: usize = 512;
const PROJECTION_ITEM_ID_MAX_BYTES: usize = 512;
const PROJECTION_TEXT_MAX_BYTES: usize = 128 * 1024;
const CAPABILITY_ACTIVITY_ERROR_KIND_MAX_BYTES: usize = 64;
const CAPABILITY_ACTIVITY_ERROR_KIND_SEGMENT_MAX_BYTES: usize = 24;
const CAPABILITY_ACTIVITY_UNCLASSIFIED_ERROR_KIND: &str = "Unclassified";
pub const CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES: usize = 2 * 1024;
pub const CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES: usize = 16 * 1024;
pub const CAPABILITY_DISPLAY_PREVIEW_MAX_LINES: usize = 120;
pub const CAPABILITY_DISPLAY_KIND_MAX_BYTES: usize = 32;
pub const CAPABILITY_DISPLAY_RESULT_REF_MAX_BYTES: usize = 256;

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
    )?;
    if value
        .lines()
        .nth(CAPABILITY_DISPLAY_PREVIEW_MAX_LINES)
        .is_some()
    {
        return Err(invalid(
            "capability_display_output_preview",
            format!("must be at most {CAPABILITY_DISPLAY_PREVIEW_MAX_LINES} lines"),
        ));
    }
    Ok(())
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityActivityView {
    pub invocation_id: InvocationId,
    pub thread_id: Option<ThreadId>,
    pub capability_id: CapabilityId,
    pub status: CapabilityActivityStatusView,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
    pub updated_at: DateTime<Utc>,
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
            thread_id: &'a Option<ThreadId>,
            capability_id: &'a CapabilityId,
            status: CapabilityActivityStatusView,
            provider: &'a Option<ExtensionId>,
            runtime: &'a Option<RuntimeKind>,
            process_id: &'a Option<ProcessId>,
            output_bytes: Option<u64>,
            error_kind: &'a Option<String>,
            updated_at: &'a DateTime<Utc>,
        }

        Wire {
            invocation_id: &self.invocation_id,
            thread_id: &self.thread_id,
            capability_id: &self.capability_id,
            status: self.status,
            provider: &self.provider,
            runtime: &self.runtime,
            process_id: &self.process_id,
            output_bytes: self.output_bytes,
            error_kind: &self.error_kind,
            updated_at: &self.updated_at,
        }
        .serialize(serializer)
    }
}

impl CapabilityActivityView {
    pub fn new(input: CapabilityActivityViewInput) -> Result<Self, ProductAdapterError> {
        let value = Self {
            invocation_id: input.invocation_id,
            thread_id: input.thread_id,
            capability_id: input.capability_id,
            status: input.status,
            provider: input.provider,
            runtime: input.runtime,
            process_id: input.process_id,
            output_bytes: input.output_bytes,
            error_kind: input.error_kind,
            updated_at: input.updated_at,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), ProductAdapterError> {
        if let Some(error_kind) = self.error_kind.as_deref() {
            validate_error_kind("capability_activity_error_kind", error_kind)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityActivityViewInput {
    pub invocation_id: InvocationId,
    pub thread_id: Option<ThreadId>,
    pub capability_id: CapabilityId,
    pub status: CapabilityActivityStatusView,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl<'de> Deserialize<'de> for CapabilityActivityView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            invocation_id: InvocationId,
            thread_id: Option<ThreadId>,
            capability_id: CapabilityId,
            status: CapabilityActivityStatusView,
            provider: Option<ExtensionId>,
            runtime: Option<RuntimeKind>,
            process_id: Option<ProcessId>,
            output_bytes: Option<u64>,
            error_kind: Option<String>,
            updated_at: DateTime<Utc>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(CapabilityActivityViewInput {
            invocation_id: wire.invocation_id,
            thread_id: wire.thread_id,
            capability_id: wire.capability_id,
            status: wire.status,
            provider: wire.provider,
            runtime: wire.runtime,
            process_id: wire.process_id,
            output_bytes: wire.output_bytes,
            error_kind: wire.error_kind,
            updated_at: wire.updated_at,
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
    pub invocation_id: InvocationId,
    pub thread_id: Option<ThreadId>,
    pub capability_id: CapabilityId,
    pub status: CapabilityActivityStatusView,
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
}

impl CapabilityDisplayPreviewView {
    pub fn new(input: CapabilityDisplayPreviewViewInput) -> Result<Self, ProductAdapterError> {
        let value = Self {
            invocation_id: input.invocation_id,
            thread_id: input.thread_id,
            capability_id: input.capability_id,
            status: input.status,
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
            invocation_id: &'a InvocationId,
            thread_id: &'a Option<ThreadId>,
            capability_id: &'a CapabilityId,
            status: CapabilityActivityStatusView,
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
        }

        Wire {
            invocation_id: &self.invocation_id,
            thread_id: &self.thread_id,
            capability_id: &self.capability_id,
            status: self.status,
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
        }
        .serialize(serializer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDisplayPreviewViewInput {
    pub invocation_id: InvocationId,
    pub thread_id: Option<ThreadId>,
    pub capability_id: CapabilityId,
    pub status: CapabilityActivityStatusView,
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
}

impl<'de> Deserialize<'de> for CapabilityDisplayPreviewView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            invocation_id: InvocationId,
            thread_id: Option<ThreadId>,
            capability_id: CapabilityId,
            status: CapabilityActivityStatusView,
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
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(CapabilityDisplayPreviewViewInput {
            invocation_id: wire.invocation_id,
            thread_id: wire.thread_id,
            capability_id: wire.capability_id,
            status: wire.status,
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
        })
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatePromptView {
    pub turn_run_id: TurnRunId,
    pub gate_ref: String,
    pub headline: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPromptView {
    pub turn_run_id: TurnRunId,
    pub auth_request_ref: String,
    pub headline: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductProjectionItem {
    Text { id: String, body: String },
    Thinking { id: String, body: String },
    RunStatus { run_id: TurnRunId, status: String },
    Gate { gate_ref: String, headline: String },
}

impl ProductProjectionItem {
    fn validate(&self) -> Result<(), ProductAdapterError> {
        match self {
            Self::Text { id, body } | Self::Thinking { id, body } => {
                validate_bounded_text("projection_item_id", id, PROJECTION_ITEM_ID_MAX_BYTES)?;
                validate_bounded_text("projection_text", body, PROJECTION_TEXT_MAX_BYTES)
            }
            Self::RunStatus { status, .. } => validate_bounded_text(
                "projection_run_status",
                status,
                PROJECTION_ITEM_ID_MAX_BYTES,
            ),
            Self::Gate { gate_ref, headline } => {
                validate_bounded_text(
                    "projection_gate_ref",
                    gate_ref,
                    PROJECTION_ITEM_ID_MAX_BYTES,
                )?;
                validate_bounded_text(
                    "projection_gate_headline",
                    headline,
                    PROJECTION_TEXT_MAX_BYTES,
                )
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
            Text { id: String, body: String },
            Thinking { id: String, body: String },
            RunStatus { run_id: TurnRunId, status: String },
            Gate { gate_ref: String, headline: String },
        }
        let value = match Wire::deserialize(deserializer)? {
            Wire::Text { id, body } => ProductProjectionItem::Text { id, body },
            Wire::Thinking { id, body } => ProductProjectionItem::Thinking { id, body },
            Wire::RunStatus { run_id, status } => {
                ProductProjectionItem::RunStatus { run_id, status }
            }
            Wire::Gate { gate_ref, headline } => ProductProjectionItem::Gate { gate_ref, headline },
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
        let state = ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::Thinking {
                id: "thinking:run:1".to_string(),
                body: "checking context".to_string(),
            }],
        )
        .expect("valid thinking projection");
        let value = serde_json::to_value(&state).expect("serialize");
        assert_eq!(value["items"][0]["thinking"]["body"], "checking context");
        let decoded: ProductProjectionState =
            serde_json::from_value(value).expect("deserialize thinking projection");
        assert_eq!(decoded, state);
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
        let view = CapabilityActivityView::new(CapabilityActivityViewInput {
            invocation_id: InvocationId::new(),
            thread_id: Some(ThreadId::new("thread-tool-activity").expect("thread id")),
            capability_id: CapabilityId::new("script.echo").expect("capability id"),
            status: CapabilityActivityStatusView::Completed,
            provider: Some(ExtensionId::new("script").expect("provider id")),
            runtime: Some(RuntimeKind::Script),
            process_id: None,
            output_bytes: Some(12),
            error_kind: None,
            updated_at: Utc::now(),
        })
        .expect("valid activity");
        let json = serde_json::to_value(&view).expect("serialize");
        let rendered = serde_json::to_string(&json).expect("render");

        assert_eq!(json["status"], "completed");
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
        let view = CapabilityDisplayPreviewView::new(CapabilityDisplayPreviewViewInput {
            invocation_id: InvocationId::new(),
            thread_id: Some(ThreadId::new("thread-tool-preview").expect("thread id")),
            capability_id: CapabilityId::new("builtin.read_file").expect("capability id"),
            status: CapabilityActivityStatusView::Completed,
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
        })
        .expect("valid preview");

        let json = serde_json::to_value(&view).expect("serialize");
        assert_eq!(json["title"], "read_file");
        assert_eq!(json["subtitle"], "src/main.rs");
        assert_eq!(json["output_kind"], "text");
    }

    #[test]
    fn capability_display_preview_view_rejects_unbounded_preview() {
        let json = serde_json::json!({
            "invocation_id": InvocationId::new(),
            "thread_id": "thread-tool-preview",
            "capability_id": "builtin.read_file",
            "status": "completed",
            "title": "read_file",
            "subtitle": "src/main.rs",
            "input_summary": "path: src/main.rs",
            "output_summary": "read file",
            "output_preview": (0..=CAPABILITY_DISPLAY_PREVIEW_MAX_LINES).map(|_| "line").collect::<Vec<_>>().join("\n"),
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
            thread_id: Some(ThreadId::new("thread-tool-activity").expect("thread id")),
            capability_id: CapabilityId::new("script.echo").expect("capability id"),
            status: CapabilityActivityStatusView::Failed,
            provider: Some(ExtensionId::new("script").expect("provider id")),
            runtime: Some(RuntimeKind::Script),
            process_id: None,
            output_bytes: None,
            error_kind: Some("/tmp/private-host-path".to_string()),
            updated_at: Utc::now(),
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
}

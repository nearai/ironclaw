//! Outbound envelope, projection-derived payloads, and projection cursor.

use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, ProcessId, RuntimeKind, ThreadId,
};
use ironclaw_turns::{ReplyTargetBindingRef, SanitizedFailure, TurnRunId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::error::ProductAdapterError;
use crate::external::{ExternalActorRef, ExternalConversationRef};
use crate::identity::{AdapterInstallationId, ProductAdapterId};

const PROJECTION_CURSOR_MAX_BYTES: usize = 1024;
const PROJECTION_THREAD_ID_MAX_BYTES: usize = 512;
const PROJECTION_ITEM_ID_MAX_BYTES: usize = 512;
const PROJECTION_TEXT_MAX_BYTES: usize = 128 * 1024;
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
            updated_at: &'a DateTime<Utc>,
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
            updated_at: &self.updated_at,
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
    pub turn_run_id: Option<TurnRunId>,
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
            #[serde(default)]
            turn_run_id: Option<TurnRunId>,
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
            turn_run_id: wire.turn_run_id,
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
    pub timeline_message_id: Option<String>,
    pub invocation_id: InvocationId,
    pub turn_run_id: Option<TurnRunId>,
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
            timeline_message_id: input.timeline_message_id,
            invocation_id: input.invocation_id,
            turn_run_id: input.turn_run_id,
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
            timeline_message_id: &self.timeline_message_id,
            invocation_id: &self.invocation_id,
            turn_run_id: &self.turn_run_id,
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
    pub timeline_message_id: Option<String>,
    pub invocation_id: InvocationId,
    pub turn_run_id: Option<TurnRunId>,
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
            #[serde(default)]
            timeline_message_id: Option<String>,
            invocation_id: InvocationId,
            #[serde(default)]
            turn_run_id: Option<TurnRunId>,
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
            timeline_message_id: wire.timeline_message_id,
            invocation_id: wire.invocation_id,
            turn_run_id: wire.turn_run_id,
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
    #[serde(default)]
    pub allow_always: bool,
}

/// Discriminator for the kind of auth challenge surfaced in an `AuthPromptView`.
///
/// Added in issue #4112 as additive optional context. Legacy consumers that
/// serialized `AuthPromptView` before this field existed will deserialize it
/// as `None` (via `serde(default)`) without error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthPromptChallengeKind {
    /// Browser must open `authorization_url` in a new tab and wait for the
    /// OAuth callback to resume the run server-side.
    #[serde(rename = "oauth_url")]
    OAuthUrl,
    /// User must type a manual token (PAT, API key) into the chat form.
    ManualToken,
    /// Other challenge kind (account selection, setup required, reauthorize).
    /// The UI should fall back to a generic "authentication required" card.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPromptView {
    pub turn_run_id: TurnRunId,
    pub auth_request_ref: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductProjectionItem {
    Text {
        id: String,
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
    },
    Gate {
        gate_ref: String,
        headline: String,
        #[serde(default)]
        allow_always: bool,
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
            Self::Text { id, body } | Self::Thinking { id, body, .. } => {
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
                ..
            } => {
                validate_bounded_text(
                    "projection_run_status",
                    status,
                    PROJECTION_ITEM_ID_MAX_BYTES,
                )?;
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
                gate_ref, headline, ..
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
                )
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
            },
            Gate {
                gate_ref: String,
                headline: String,
                #[serde(default)]
                allow_always: bool,
            },
            SkillActivation {
                id: String,
                run_id: TurnRunId,
                skill_names: Vec<String>,
                feedback: Vec<String>,
            },
        }
        let value = match Wire::deserialize(deserializer)? {
            Wire::Text { id, body } => ProductProjectionItem::Text { id, body },
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
            } => ProductProjectionItem::RunStatus {
                run_id,
                status,
                failure_category: failure_category
                    .map(SanitizedFailure::new)
                    .transpose()
                    .map_err(serde::de::Error::custom)?,
                failure_summary,
            },
            Wire::Gate {
                gate_ref,
                headline,
                allow_always,
            } => ProductProjectionItem::Gate {
                gate_ref,
                headline,
                allow_always,
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
        for (variant, expected) in [
            (AuthPromptChallengeKind::OAuthUrl, "\"oauth_url\""),
            (AuthPromptChallengeKind::ManualToken, "\"manual_token\""),
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
                    updated_at: Utc::now(),
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
            }]
        );
    }

    #[test]
    fn projection_state_round_trips_gate_item_with_allow_always() {
        let state = ProductProjectionState::new(
            "thread-1",
            vec![ProductProjectionItem::Gate {
                gate_ref: "gate:approval-test".to_string(),
                headline: "Approval required".to_string(),
                allow_always: true,
            }],
        )
        .expect("valid gate projection");
        let value = serde_json::to_value(&state).expect("serialize");

        assert_eq!(value["items"][0]["gate"]["gate_ref"], "gate:approval-test");
        assert_eq!(value["items"][0]["gate"]["headline"], "Approval required");
        assert_eq!(value["items"][0]["gate"]["allow_always"], true);
        let decoded: ProductProjectionState =
            serde_json::from_value(value).expect("deserialize gate projection");
        assert_eq!(decoded, state);
    }

    #[test]
    fn projection_state_accepts_legacy_gate_item_without_allow_always() {
        let json = serde_json::json!({
            "thread_id": "thread-1",
            "items": [{
                "gate": {
                    "gate_ref": "gate:approval-test",
                    "headline": "Approval required"
                }
            }]
        });

        let decoded: ProductProjectionState =
            serde_json::from_value(json).expect("deserialize legacy gate projection");
        assert_eq!(
            decoded.items,
            vec![ProductProjectionItem::Gate {
                gate_ref: "gate:approval-test".to_string(),
                headline: "Approval required".to_string(),
                allow_always: false,
            }]
        );
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
            updated_at: Utc::now(),
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
        assert_eq!(json["turn_run_id"], run_id.to_string());
        assert_eq!(json["subtitle"], "src/main.rs");
        assert_eq!(json["output_kind"], "text");
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

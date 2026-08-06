//! Operator-only diagnostic vocabulary for the Web Debug Inspector.
//!
//! These DTOs cross the product boundary through the dedicated inspection
//! surface; snapshots and updates flow out, while cursors round-trip to resume
//! reads. They are deliberately separate from product projection events: raw
//! prompt components and tool details must never enter the normal product
//! stream.

use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    ids::{TenantId, ThreadId, UserId},
    turn::{CapabilityActivityId, TurnRunId},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::descriptors::ProductView;

pub const INSPECTOR_SNAPSHOT_VIEW: ProductView<DiagnosticRunRequest, serde_json::Value> =
    ProductView::unpaginated("inspector.snapshot");
pub const INSPECTOR_PROMPT_VIEW: ProductView<DiagnosticRunRequest, serde_json::Value> =
    ProductView::unpaginated("inspector.prompt");
pub const INSPECTOR_TOOL_VIEW: ProductView<DiagnosticToolRequest, serde_json::Value> =
    ProductView::unpaginated("inspector.tool");
pub const INSPECTOR_UPDATES_VIEW: ProductView<DiagnosticRunRequest, serde_json::Value> =
    ProductView::unpaginated("inspector.updates");

pub const PROMPT_COMPONENT_CONTENT_MAX_BYTES: usize = 64 * 1024;
pub const PROMPT_COMPONENT_TOTAL_MAX_BYTES: usize = 256 * 1024;
pub const RECONSTRUCTED_PROMPT_MAX_BYTES: usize = 256 * 1024;
pub const TOOL_ARGUMENTS_MAX_BYTES: usize = 64 * 1024;
pub const TOOL_RESULT_MAX_BYTES: usize = 50 * 1024;
pub const DIAGNOSTIC_LABEL_MAX_BYTES: usize = 256;
pub const DIAGNOSTIC_SUMMARY_MAX_BYTES: usize = 2 * 1024;
pub const MAX_PROMPT_COMPONENTS: usize = 128;
pub const MAX_ACTIVE_SKILLS: usize = 64;
pub const MAX_MODELS_IN_STATS: usize = 64;
// Keep the process-wide defaults conservative because retained tool payloads
// may each contain both bounded arguments and a bounded result.
pub const DEFAULT_MAX_ACTIVITY_ENTRIES: usize = 1_000;
pub const DEFAULT_MAX_TRACKED_SESSIONS: usize = 8;
pub const DEFAULT_MAX_RETAINED_RUNS_PER_SESSION: usize = 2;
pub const DEFAULT_MAX_LIVE_UPDATE_SCOPES: usize =
    DEFAULT_MAX_TRACKED_SESSIONS * DEFAULT_MAX_RETAINED_RUNS_PER_SESSION;
pub const DEFAULT_MAX_MODEL_CALLS_PER_RUN: usize = 128;
pub const DEFAULT_MAX_TOOL_EXECUTIONS_PER_RUN: usize = 16;
pub const DEFAULT_MAX_RETAINED_UPDATES_PER_RUN: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiagnosticModelCallId(Uuid);

impl DiagnosticModelCallId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for DiagnosticModelCallId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DiagnosticModelCallId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiagnosticStreamId(Uuid);

impl DiagnosticStreamId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }
}

impl Default for DiagnosticStreamId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DiagnosticStreamId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct DiagnosticSequence(u64);

impl DiagnosticSequence {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DiagnosticCursor {
    pub stream_id: DiagnosticStreamId,
    pub sequence: DiagnosticSequence,
}

impl DiagnosticCursor {
    pub const fn new(stream_id: DiagnosticStreamId, sequence: DiagnosticSequence) -> Self {
        Self {
            stream_id,
            sequence,
        }
    }

    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let (stream_id, sequence) = value
            .split_once(':')
            .ok_or("diagnostic cursor has an invalid shape")?;
        let stream_id = Uuid::parse_str(stream_id)
            .map(DiagnosticStreamId::from_uuid)
            .map_err(|_| "diagnostic cursor has an invalid stream id")?;
        let sequence = sequence
            .parse::<u64>()
            .map(DiagnosticSequence::new)
            .map_err(|_| "diagnostic cursor has an invalid sequence")?;
        Ok(Self::new(stream_id, sequence))
    }
}

impl std::fmt::Display for DiagnosticCursor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}", self.stream_id, self.sequence.as_u64())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticRunRequest {
    pub thread_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticToolRequest {
    pub thread_id: String,
    pub run_id: String,
    pub activity_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct DiagnosticScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub thread_id: ThreadId,
    pub run_id: TurnRunId,
}

impl DiagnosticScope {
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        thread_id: ThreadId,
        run_id: TurnRunId,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            thread_id,
            run_id,
        }
    }
}

/// UTF-8 text with explicit original-size and truncation metadata.
///
/// Construction is limited to the purpose-specific constructors so callers
/// cannot silently select an unbounded maximum.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct BoundedDiagnosticText {
    content: String,
    original_bytes: u64,
    truncated: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundedDiagnosticTextWire {
    content: String,
    original_bytes: u64,
    truncated: bool,
}

impl<'de> Deserialize<'de> for BoundedDiagnosticText {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = BoundedDiagnosticTextWire::deserialize(deserializer)?;
        if wire.content.len() > RECONSTRUCTED_PROMPT_MAX_BYTES {
            return Err(serde::de::Error::custom(
                "diagnostic text exceeds the maximum retained byte length",
            ));
        }
        let retained_bytes = u64::try_from(wire.content.len()).map_err(|_| {
            serde::de::Error::custom("diagnostic text retained byte length is not representable")
        })?;
        if wire.original_bytes < retained_bytes {
            return Err(serde::de::Error::custom(
                "diagnostic text original byte length is smaller than retained text",
            ));
        }
        if wire.truncated != (wire.original_bytes > retained_bytes) {
            return Err(serde::de::Error::custom(
                "diagnostic text truncation metadata is inconsistent",
            ));
        }
        Ok(Self {
            content: wire.content,
            original_bytes: wire.original_bytes,
            truncated: wire.truncated,
        })
    }
}

impl std::fmt::Debug for BoundedDiagnosticText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BoundedDiagnosticText")
            .field("content", &"[diagnostic content redacted]")
            .field("retained_bytes", &self.content.len())
            .field("original_bytes", &self.original_bytes)
            .field("truncated", &self.truncated)
            .finish()
    }
}

impl BoundedDiagnosticText {
    pub fn label(value: impl Into<String>) -> Self {
        Self::bounded(value.into(), DIAGNOSTIC_LABEL_MAX_BYTES)
    }

    pub fn summary(value: impl Into<String>) -> Self {
        Self::bounded(value.into(), DIAGNOSTIC_SUMMARY_MAX_BYTES)
    }

    pub fn prompt_component(value: impl Into<String>) -> Self {
        Self::bounded(value.into(), PROMPT_COMPONENT_CONTENT_MAX_BYTES)
    }

    pub fn reconstructed_prompt(value: impl Into<String>) -> Self {
        Self::bounded(value.into(), RECONSTRUCTED_PROMPT_MAX_BYTES)
    }

    pub fn tool_arguments(value: impl Into<String>) -> Self {
        Self::bounded(value.into(), TOOL_ARGUMENTS_MAX_BYTES)
    }

    pub fn tool_result(value: impl Into<String>) -> Self {
        Self::bounded(value.into(), TOOL_RESULT_MAX_BYTES)
    }

    pub fn retained_tool_result(
        value: impl Into<String>,
        original_bytes: u64,
    ) -> Result<Self, &'static str> {
        let mut bounded = Self::bounded(value.into(), TOOL_RESULT_MAX_BYTES);
        let retained_bytes = u64::try_from(bounded.content.len())
            .map_err(|_| "diagnostic text retained byte length is not representable")?;
        if original_bytes < retained_bytes {
            return Err("diagnostic text original byte length is smaller than retained text");
        }
        bounded.original_bytes = original_bytes;
        bounded.truncated = original_bytes > retained_bytes;
        Ok(bounded)
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub const fn original_bytes(&self) -> u64 {
        self.original_bytes
    }

    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    fn validate_retained_max(self, max_bytes: usize) -> Result<Self, &'static str> {
        if self.content.len() > max_bytes {
            return Err("diagnostic text exceeds its field byte limit");
        }
        Ok(self)
    }

    fn rebound(self, max_bytes: usize) -> Self {
        if self.content.len() <= max_bytes {
            return self;
        }
        let original_bytes = self.original_bytes;
        let mut bounded = Self::bounded(self.content, max_bytes);
        bounded.original_bytes = original_bytes;
        bounded.truncated = true;
        bounded
    }

    fn bounded(value: String, max_bytes: usize) -> Self {
        let original_bytes = u64::try_from(value.len()).unwrap_or(u64::MAX);
        if value.len() <= max_bytes {
            return Self {
                content: value,
                original_bytes,
                truncated: false,
            };
        }
        let mut end = max_bytes.min(value.len());
        while end > 0 && !value.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            content: value[..end].to_string(), // safety: `end` is a verified UTF-8 boundary.
            original_bytes,
            truncated: true,
        }
    }
}

fn deserialize_bounded_label<'de, D>(deserializer: D) -> Result<BoundedDiagnosticText, D::Error>
where
    D: serde::Deserializer<'de>,
{
    BoundedDiagnosticText::deserialize(deserializer)?
        .validate_retained_max(DIAGNOSTIC_LABEL_MAX_BYTES)
        .map_err(serde::de::Error::custom)
}

fn deserialize_bounded_prompt_component<'de, D>(
    deserializer: D,
) -> Result<BoundedDiagnosticText, D::Error>
where
    D: serde::Deserializer<'de>,
{
    BoundedDiagnosticText::deserialize(deserializer)?
        .validate_retained_max(PROMPT_COMPONENT_CONTENT_MAX_BYTES)
        .map_err(serde::de::Error::custom)
}

fn deserialize_optional_bounded_tool_arguments<'de, D>(
    deserializer: D,
) -> Result<Option<BoundedDiagnosticText>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<BoundedDiagnosticText>::deserialize(deserializer)?
        .map(|value| value.validate_retained_max(TOOL_ARGUMENTS_MAX_BYTES))
        .transpose()
        .map_err(serde::de::Error::custom)
}

fn deserialize_optional_bounded_tool_result<'de, D>(
    deserializer: D,
) -> Result<Option<BoundedDiagnosticText>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<BoundedDiagnosticText>::deserialize(deserializer)?
        .map(|value| value.validate_retained_max(TOOL_RESULT_MAX_BYTES))
        .transpose()
        .map_err(serde::de::Error::custom)
}

fn deserialize_optional_bounded_label<'de, D>(
    deserializer: D,
) -> Result<Option<BoundedDiagnosticText>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<BoundedDiagnosticText>::deserialize(deserializer)?
        .map(|value| value.validate_retained_max(DIAGNOSTIC_LABEL_MAX_BYTES))
        .transpose()
        .map_err(serde::de::Error::custom)
}

fn deserialize_optional_bounded_summary<'de, D>(
    deserializer: D,
) -> Result<Option<BoundedDiagnosticText>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<BoundedDiagnosticText>::deserialize(deserializer)?
        .map(|value| value.validate_retained_max(DIAGNOSTIC_SUMMARY_MAX_BYTES))
        .transpose()
        .map_err(serde::de::Error::custom)
}

fn deserialize_bounded_model_counts<'de, D>(
    deserializer: D,
) -> Result<Vec<DiagnosticModelCount>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Vec::<DiagnosticModelCount>::deserialize(deserializer)?;
    if values.len() > MAX_MODELS_IN_STATS {
        return Err(serde::de::Error::custom(
            "diagnostic model counts exceed the item limit",
        ));
    }
    Ok(values)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptComponentKind {
    System,
    Identity,
    Instruction,
    Skill,
    Capability,
    Conversation,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptComponentDiagnostic {
    pub kind: PromptComponentKind,
    #[serde(deserialize_with = "deserialize_bounded_label")]
    pub label: BoundedDiagnosticText,
    #[serde(deserialize_with = "deserialize_bounded_prompt_component")]
    pub content: BoundedDiagnosticText,
    pub estimated_tokens: Option<u64>,
}

impl PromptComponentDiagnostic {
    pub fn new(
        kind: PromptComponentKind,
        label: impl Into<String>,
        content: impl Into<String>,
        estimated_tokens: Option<u64>,
    ) -> Self {
        Self {
            kind,
            label: BoundedDiagnosticText::label(label),
            content: BoundedDiagnosticText::prompt_component(content),
            estimated_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptDiagnostic {
    pub captured_at: DateTime<Utc>,
    pub components: Vec<PromptComponentDiagnostic>,
    pub components_truncated: bool,
    pub reconstructed_prompt: BoundedDiagnosticText,
    pub total_estimated_tokens: Option<u64>,
    pub message_count: u32,
    pub identity_message_count: u32,
    pub instruction_snippet_count: u32,
    pub active_skills: Vec<BoundedDiagnosticText>,
    pub active_skills_truncated: bool,
    pub capability_count: u32,
    pub requested_model: Option<BoundedDiagnosticText>,
    pub effective_model: Option<BoundedDiagnosticText>,
    pub context_limit: Option<u64>,
}

impl PromptDiagnostic {
    // arch-exempt: too_many_args, one validated path bounds the prompt DTO, plan #7219
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        captured_at: DateTime<Utc>,
        components: Vec<PromptComponentDiagnostic>,
        reconstructed_prompt: impl Into<String>,
        total_estimated_tokens: Option<u64>,
        message_count: u32,
        identity_message_count: u32,
        instruction_snippet_count: u32,
        active_skills: Vec<String>,
        capability_count: u32,
        requested_model: Option<String>,
        effective_model: Option<String>,
        context_limit: Option<u64>,
    ) -> Self {
        let active_skills_truncated = active_skills.len() > MAX_ACTIVE_SKILLS;
        let active_skills = active_skills
            .into_iter()
            .take(MAX_ACTIVE_SKILLS)
            .map(BoundedDiagnosticText::label)
            .collect();

        Self {
            captured_at,
            components,
            components_truncated: false,
            reconstructed_prompt: BoundedDiagnosticText::reconstructed_prompt(reconstructed_prompt),
            total_estimated_tokens,
            message_count,
            identity_message_count,
            instruction_snippet_count,
            active_skills,
            active_skills_truncated,
            capability_count,
            requested_model: requested_model.map(BoundedDiagnosticText::label),
            effective_model: effective_model.map(BoundedDiagnosticText::label),
            context_limit,
        }
        .into_bounded()
    }

    /// Reapplies every owning prompt limit at a trust boundary.
    ///
    /// The fields remain public wire DTO fields, so callers may construct this
    /// type without using [`Self::new`]. Store and transport boundaries should
    /// call this method before retaining an externally constructed value.
    pub fn into_bounded(mut self) -> Self {
        let original_component_count = self.components.len();
        let mut remaining = PROMPT_COMPONENT_TOTAL_MAX_BYTES;
        let mut bounded_components =
            Vec::with_capacity(original_component_count.min(MAX_PROMPT_COMPONENTS));
        for mut component in self.components.into_iter().take(MAX_PROMPT_COMPONENTS) {
            if remaining == 0 {
                break;
            }
            component.label = component.label.rebound(DIAGNOSTIC_LABEL_MAX_BYTES);
            component.content = component
                .content
                .rebound(PROMPT_COMPONENT_CONTENT_MAX_BYTES);
            let retained = component.content.content().len().min(remaining);
            component.content = component.content.rebound(retained);
            remaining = remaining.saturating_sub(component.content.content().len());
            bounded_components.push(component);
        }
        self.components_truncated |= bounded_components.len() < original_component_count;
        self.components = bounded_components;

        let original_skill_count = self.active_skills.len();
        self.active_skills = self
            .active_skills
            .into_iter()
            .take(MAX_ACTIVE_SKILLS)
            .map(|skill| skill.rebound(DIAGNOSTIC_LABEL_MAX_BYTES))
            .collect();
        self.active_skills_truncated |= self.active_skills.len() < original_skill_count;
        self.reconstructed_prompt = self
            .reconstructed_prompt
            .rebound(RECONSTRUCTED_PROMPT_MAX_BYTES);
        self.requested_model = self
            .requested_model
            .map(|model| model.rebound(DIAGNOSTIC_LABEL_MAX_BYTES));
        self.effective_model = self
            .effective_model
            .map(|model| model.rebound(DIAGNOSTIC_LABEL_MAX_BYTES));
        self
    }

    pub fn any_content_truncated(&self) -> bool {
        self.components_truncated
            || self.reconstructed_prompt.truncated()
            || self.active_skills_truncated
            || self
                .components
                .iter()
                .any(|component| component.label.truncated() || component.content.truncated())
            || self
                .active_skills
                .iter()
                .any(BoundedDiagnosticText::truncated)
            || self
                .requested_model
                .as_ref()
                .is_some_and(BoundedDiagnosticText::truncated)
            || self
                .effective_model
                .as_ref()
                .is_some_and(BoundedDiagnosticText::truncated)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectorModelCallStatus {
    Started,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCallDiagnostic {
    pub call_id: DiagnosticModelCallId,
    pub iteration: u32,
    #[serde(deserialize_with = "deserialize_bounded_label")]
    pub requested_model: BoundedDiagnosticText,
    #[serde(default, deserialize_with = "deserialize_optional_bounded_label")]
    pub effective_model: Option<BoundedDiagnosticText>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub status: InspectorModelCallStatus,
    pub usage: Option<ModelTokenUsage>,
    #[serde(default, deserialize_with = "deserialize_optional_bounded_summary")]
    pub failure_summary: Option<BoundedDiagnosticText>,
}

impl ModelCallDiagnostic {
    // arch-exempt: too_many_args, atomically construct one bounded model call, plan #7219
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        call_id: DiagnosticModelCallId,
        iteration: u32,
        requested_model: impl Into<String>,
        effective_model: Option<String>,
        started_at: DateTime<Utc>,
        completed_at: Option<DateTime<Utc>>,
        duration_ms: Option<u64>,
        status: InspectorModelCallStatus,
        usage: Option<ModelTokenUsage>,
        failure_summary: Option<String>,
    ) -> Self {
        Self {
            call_id,
            iteration,
            requested_model: BoundedDiagnosticText::label(requested_model),
            effective_model: effective_model.map(BoundedDiagnosticText::label),
            started_at,
            completed_at,
            duration_ms,
            status,
            usage,
            failure_summary: failure_summary.map(BoundedDiagnosticText::summary),
        }
    }

    pub fn into_bounded(mut self) -> Self {
        self.requested_model = self.requested_model.rebound(DIAGNOSTIC_LABEL_MAX_BYTES);
        self.effective_model = self
            .effective_model
            .map(|model| model.rebound(DIAGNOSTIC_LABEL_MAX_BYTES));
        self.failure_summary = self
            .failure_summary
            .map(|summary| summary.rebound(DIAGNOSTIC_SUMMARY_MAX_BYTES));
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionStatus {
    Started,
    Succeeded,
    Failed,
}

#[derive(Deserialize)]
struct ToolExecutionDiagnosticWire {
    activity_id: CapabilityActivityId,
    model_call_id: Option<DiagnosticModelCallId>,
    #[serde(deserialize_with = "deserialize_bounded_label")]
    capability_name: BoundedDiagnosticText,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_bounded_tool_arguments"
    )]
    arguments: Option<BoundedDiagnosticText>,
    #[serde(default, deserialize_with = "deserialize_optional_bounded_tool_result")]
    result: Option<BoundedDiagnosticText>,
    status: ToolExecutionStatus,
    duration_ms: Option<u64>,
    output_bytes: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_bounded_label")]
    failure_category: Option<BoundedDiagnosticText>,
    #[serde(default, deserialize_with = "deserialize_optional_bounded_summary")]
    failure_summary: Option<BoundedDiagnosticText>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ToolExecutionDiagnosticWire")]
pub struct ToolExecutionDiagnostic {
    pub activity_id: CapabilityActivityId,
    pub model_call_id: Option<DiagnosticModelCallId>,
    pub capability_name: BoundedDiagnosticText,
    pub arguments: Option<BoundedDiagnosticText>,
    pub result: Option<BoundedDiagnosticText>,
    pub status: ToolExecutionStatus,
    pub duration_ms: Option<u64>,
    pub output_bytes: Option<u64>,
    pub failure_category: Option<BoundedDiagnosticText>,
    pub failure_summary: Option<BoundedDiagnosticText>,
}

impl TryFrom<ToolExecutionDiagnosticWire> for ToolExecutionDiagnostic {
    type Error = &'static str;

    fn try_from(wire: ToolExecutionDiagnosticWire) -> Result<Self, Self::Error> {
        if let Some(result) = wire.result.as_ref()
            && wire.output_bytes != Some(result.original_bytes())
        {
            return Err("tool result byte metadata is inconsistent");
        }
        Ok(Self {
            activity_id: wire.activity_id,
            model_call_id: wire.model_call_id,
            capability_name: wire.capability_name,
            arguments: wire.arguments,
            result: wire.result,
            status: wire.status,
            duration_ms: wire.duration_ms,
            output_bytes: wire.output_bytes,
            failure_category: wire.failure_category,
            failure_summary: wire.failure_summary,
        })
    }
}

impl ToolExecutionDiagnostic {
    // arch-exempt: too_many_args, atomically construct one bounded tool record, plan #7219
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        activity_id: CapabilityActivityId,
        model_call_id: Option<DiagnosticModelCallId>,
        capability_name: impl Into<String>,
        arguments: Option<String>,
        result: Option<String>,
        status: ToolExecutionStatus,
        duration_ms: Option<u64>,
        output_bytes: Option<u64>,
        failure_category: Option<String>,
        failure_summary: Option<String>,
    ) -> Self {
        let result = result.map(BoundedDiagnosticText::tool_result);
        let output_bytes = result
            .as_ref()
            .map(BoundedDiagnosticText::original_bytes)
            .or(output_bytes);
        Self {
            activity_id,
            model_call_id,
            capability_name: BoundedDiagnosticText::label(capability_name),
            arguments: arguments.map(BoundedDiagnosticText::tool_arguments),
            result,
            status,
            duration_ms,
            output_bytes,
            failure_category: failure_category.map(BoundedDiagnosticText::label),
            failure_summary: failure_summary.map(BoundedDiagnosticText::summary),
        }
    }

    pub fn into_bounded(mut self) -> Self {
        self.capability_name = self.capability_name.rebound(DIAGNOSTIC_LABEL_MAX_BYTES);
        self.arguments = self
            .arguments
            .map(|arguments| arguments.rebound(TOOL_ARGUMENTS_MAX_BYTES));
        self.result = self
            .result
            .map(|result| result.rebound(TOOL_RESULT_MAX_BYTES));
        if let Some(result) = self.result.as_ref() {
            self.output_bytes = Some(result.original_bytes());
        }
        self.failure_category = self
            .failure_category
            .map(|category| category.rebound(DIAGNOSTIC_LABEL_MAX_BYTES));
        self.failure_summary = self
            .failure_summary
            .map(|summary| summary.rebound(DIAGNOSTIC_SUMMARY_MAX_BYTES));
        self
    }

    pub fn result_truncated(&self) -> bool {
        self.result
            .as_ref()
            .is_some_and(BoundedDiagnosticText::truncated)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticActivityKind {
    TurnStarted,
    PromptPrepared,
    ModelCallStarted,
    ModelCallCompleted,
    ModelCallFailed,
    Progress,
    ToolStarted,
    ToolCompleted,
    ToolFailed,
    GateBlocked,
    FinalResponseCompleted,
    StreamDisconnected,
    StreamResumed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticActivityEvent {
    pub occurred_at: DateTime<Utc>,
    pub kind: DiagnosticActivityKind,
    pub iteration: Option<u32>,
    pub activity_id: Option<CapabilityActivityId>,
    pub model_call_id: Option<DiagnosticModelCallId>,
    #[serde(default, deserialize_with = "deserialize_optional_bounded_summary")]
    pub summary: Option<BoundedDiagnosticText>,
}

impl DiagnosticActivityEvent {
    pub fn new(
        occurred_at: DateTime<Utc>,
        kind: DiagnosticActivityKind,
        iteration: Option<u32>,
        activity_id: Option<CapabilityActivityId>,
        model_call_id: Option<DiagnosticModelCallId>,
        summary: Option<String>,
    ) -> Self {
        Self {
            occurred_at,
            kind,
            iteration,
            activity_id,
            model_call_id,
            summary: summary.map(BoundedDiagnosticText::summary),
        }
    }

    pub fn into_bounded(mut self) -> Self {
        self.summary = self
            .summary
            .map(|summary| summary.rebound(DIAGNOSTIC_SUMMARY_MAX_BYTES));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticActivityEntry {
    pub sequence: DiagnosticSequence,
    pub event: DiagnosticActivityEvent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticMetricTotal {
    pub known_total: u64,
    pub unavailable_samples: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticModelCount {
    #[serde(deserialize_with = "deserialize_bounded_label")]
    pub model: BoundedDiagnosticText,
    pub calls: u64,
}

impl DiagnosticModelCount {
    pub fn new(model: impl Into<String>, calls: u64) -> Self {
        Self {
            model: BoundedDiagnosticText::label(model),
            calls,
        }
    }

    pub fn into_bounded(mut self) -> Self {
        self.model = self.model.rebound(DIAGNOSTIC_LABEL_MAX_BYTES);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDiagnosticStats {
    pub total_model_calls: u64,
    #[serde(deserialize_with = "deserialize_bounded_model_counts")]
    pub calls_per_model: Vec<DiagnosticModelCount>,
    pub calls_per_model_truncated: bool,
    pub input_tokens: DiagnosticMetricTotal,
    pub output_tokens: DiagnosticMetricTotal,
    pub cache_read_input_tokens: DiagnosticMetricTotal,
    pub cache_creation_input_tokens: DiagnosticMetricTotal,
    pub total_latency_ms: DiagnosticMetricTotal,
    pub total_tool_calls: u64,
    pub successful_tool_calls: u64,
    pub failed_tool_calls: u64,
}

impl SessionDiagnosticStats {
    pub fn into_bounded(mut self) -> Self {
        if self.calls_per_model.len() > MAX_MODELS_IN_STATS {
            self.calls_per_model.truncate(MAX_MODELS_IN_STATS);
            self.calls_per_model_truncated = true;
        }
        self.calls_per_model = self
            .calls_per_model
            .into_iter()
            .map(DiagnosticModelCount::into_bounded)
            .collect();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum DiagnosticUpdateKind {
    PromptUpdated {
        component_count: u32,
        total_estimated_tokens: Option<u64>,
        truncated: bool,
    },
    ModelCall(ModelCallDiagnostic),
    ToolExecutionUpdated {
        activity_id: CapabilityActivityId,
        model_call_id: Option<DiagnosticModelCallId>,
        #[serde(deserialize_with = "deserialize_bounded_label")]
        capability_name: BoundedDiagnosticText,
        status: ToolExecutionStatus,
        duration_ms: Option<u64>,
        output_bytes: Option<u64>,
        result_truncated: bool,
    },
    Activity(DiagnosticActivityEvent),
    Stats(SessionDiagnosticStats),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticUpdateEnvelope {
    pub scope: DiagnosticScope,
    pub stream_id: DiagnosticStreamId,
    pub sequence: DiagnosticSequence,
    pub emitted_at: DateTime<Utc>,
    pub update: DiagnosticUpdateKind,
}

impl DiagnosticUpdateEnvelope {
    pub const fn cursor(&self) -> DiagnosticCursor {
        DiagnosticCursor::new(self.stream_id, self.sequence)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticUpdateBatch {
    pub updates: Vec<DiagnosticUpdateEnvelope>,
    pub retention_floor: Option<DiagnosticCursor>,
    pub latest_cursor: Option<DiagnosticCursor>,
    pub rebase_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticSnapshot {
    pub scope: DiagnosticScope,
    pub stream_id: DiagnosticStreamId,
    pub prompt: Option<PromptDiagnostic>,
    pub model_calls: Vec<ModelCallDiagnostic>,
    pub tool_executions: Vec<ToolExecutionDiagnostic>,
    pub activity: Vec<DiagnosticActivityEntry>,
    pub stats: SessionDiagnosticStats,
    pub latest_sequence: DiagnosticSequence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticSnapshotResponse {
    pub snapshot: Option<DiagnosticSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticPromptResponse {
    pub prompt: Option<PromptDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticToolResponse {
    pub tool: Option<ToolExecutionDiagnostic>,
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;

    use super::*;

    fn assert_unit_enum_wire<T>(cases: &[(T, &str)])
    where
        T: std::fmt::Debug + PartialEq + Serialize + DeserializeOwned,
    {
        for (value, wire_name) in cases {
            let encoded = serde_json::to_value(value).expect("serialize wire enum");
            assert_eq!(encoded, serde_json::Value::String((*wire_name).to_string()));
            let decoded: T = serde_json::from_value(encoded).expect("deserialize wire enum");
            assert_eq!(&decoded, value);
        }
    }

    fn oversized_bounded_text(max_bytes: usize) -> serde_json::Value {
        serde_json::json!({
            "content": "x".repeat(max_bytes + 1),
            "original_bytes": max_bytes + 1,
            "truncated": false,
        })
    }

    #[test]
    fn bounded_text_preserves_utf8_and_reports_original_size() {
        let value = "€".repeat(TOOL_RESULT_MAX_BYTES);
        let bounded = BoundedDiagnosticText::tool_result(value.clone());
        assert!(bounded.truncated());
        assert!(bounded.content().len() <= TOOL_RESULT_MAX_BYTES);
        assert_eq!(
            bounded.content().chars().count(),
            TOOL_RESULT_MAX_BYTES / '€'.len_utf8()
        );
        assert_eq!(bounded.original_bytes(), value.len() as u64);
    }

    #[test]
    fn cursor_round_trips_through_the_opaque_wire_token() {
        let cursor = DiagnosticCursor::new(
            DiagnosticStreamId::from_uuid(Uuid::nil()),
            DiagnosticSequence::new(42),
        );
        assert_eq!(DiagnosticCursor::parse(&cursor.to_string()), Ok(cursor));
        assert!(DiagnosticCursor::parse("not-a-cursor").is_err());
    }

    #[test]
    fn diagnostic_cursor_round_trips_through_json() {
        let cursor = DiagnosticCursor::new(DiagnosticStreamId::new(), DiagnosticSequence::new(42));

        let encoded = serde_json::to_value(cursor).expect("serialize cursor");
        let decoded: DiagnosticCursor =
            serde_json::from_value(encoded).expect("deserialize cursor");

        assert_eq!(decoded, cursor);
    }

    #[test]
    fn unit_wire_enums_round_trip_with_their_snake_case_names() {
        assert_unit_enum_wire(&[
            (PromptComponentKind::System, "system"),
            (PromptComponentKind::Identity, "identity"),
            (PromptComponentKind::Instruction, "instruction"),
            (PromptComponentKind::Skill, "skill"),
            (PromptComponentKind::Capability, "capability"),
            (PromptComponentKind::Conversation, "conversation"),
            (PromptComponentKind::Other, "other"),
        ]);
        assert_unit_enum_wire(&[
            (InspectorModelCallStatus::Started, "started"),
            (InspectorModelCallStatus::Succeeded, "succeeded"),
            (InspectorModelCallStatus::Failed, "failed"),
        ]);
        assert_unit_enum_wire(&[
            (ToolExecutionStatus::Started, "started"),
            (ToolExecutionStatus::Succeeded, "succeeded"),
            (ToolExecutionStatus::Failed, "failed"),
        ]);
        assert_unit_enum_wire(&[
            (DiagnosticActivityKind::TurnStarted, "turn_started"),
            (DiagnosticActivityKind::PromptPrepared, "prompt_prepared"),
            (
                DiagnosticActivityKind::ModelCallStarted,
                "model_call_started",
            ),
            (
                DiagnosticActivityKind::ModelCallCompleted,
                "model_call_completed",
            ),
            (DiagnosticActivityKind::ModelCallFailed, "model_call_failed"),
            (DiagnosticActivityKind::Progress, "progress"),
            (DiagnosticActivityKind::ToolStarted, "tool_started"),
            (DiagnosticActivityKind::ToolCompleted, "tool_completed"),
            (DiagnosticActivityKind::ToolFailed, "tool_failed"),
            (DiagnosticActivityKind::GateBlocked, "gate_blocked"),
            (
                DiagnosticActivityKind::FinalResponseCompleted,
                "final_response_completed",
            ),
            (
                DiagnosticActivityKind::StreamDisconnected,
                "stream_disconnected",
            ),
            (DiagnosticActivityKind::StreamResumed, "stream_resumed"),
        ]);
    }

    #[test]
    fn diagnostic_update_variants_round_trip_with_their_tagged_wire_names() {
        let model_call_id = DiagnosticModelCallId::new();
        let updates = [
            (
                DiagnosticUpdateKind::PromptUpdated {
                    component_count: 2,
                    total_estimated_tokens: Some(3),
                    truncated: false,
                },
                "prompt_updated",
            ),
            (
                DiagnosticUpdateKind::ModelCall(ModelCallDiagnostic::new(
                    model_call_id,
                    1,
                    "requested-model",
                    Some("effective-model".to_string()),
                    Utc::now(),
                    Some(Utc::now()),
                    Some(4),
                    InspectorModelCallStatus::Succeeded,
                    Some(ModelTokenUsage {
                        input_tokens: Some(5),
                        output_tokens: Some(6),
                        ..ModelTokenUsage::default()
                    }),
                    None,
                )),
                "model_call",
            ),
            (
                DiagnosticUpdateKind::ToolExecutionUpdated {
                    activity_id: CapabilityActivityId::new(),
                    model_call_id: Some(model_call_id),
                    capability_name: BoundedDiagnosticText::label("filesystem.read"),
                    status: ToolExecutionStatus::Succeeded,
                    duration_ms: Some(7),
                    output_bytes: Some(8),
                    result_truncated: false,
                },
                "tool_execution_updated",
            ),
            (
                DiagnosticUpdateKind::Activity(DiagnosticActivityEvent::new(
                    Utc::now(),
                    DiagnosticActivityKind::Progress,
                    Some(1),
                    None,
                    Some(model_call_id),
                    Some("working".to_string()),
                )),
                "activity",
            ),
            (
                DiagnosticUpdateKind::Stats(SessionDiagnosticStats {
                    total_model_calls: 1,
                    calls_per_model: vec![DiagnosticModelCount::new("model", 1)],
                    ..SessionDiagnosticStats::default()
                }),
                "stats",
            ),
        ];

        for (update, wire_name) in updates {
            let encoded = serde_json::to_value(&update).expect("serialize diagnostic update");
            assert_eq!(encoded["type"], wire_name);
            let decoded: DiagnosticUpdateKind =
                serde_json::from_value(encoded).expect("deserialize diagnostic update");
            assert_eq!(decoded, update);
        }
    }

    #[test]
    fn owning_diagnostic_records_round_trip_through_json() {
        let component = PromptComponentDiagnostic::new(
            PromptComponentKind::Instruction,
            "policy",
            "keep responses concise",
            Some(4),
        );
        let encoded = serde_json::to_value(&component).expect("serialize prompt component");
        let decoded = serde_json::from_value::<PromptComponentDiagnostic>(encoded)
            .expect("deserialize prompt component");
        assert_eq!(decoded, component);

        let tool = ToolExecutionDiagnostic::new(
            CapabilityActivityId::new(),
            Some(DiagnosticModelCallId::new()),
            "filesystem.read",
            Some("{\"path\":\"notes.txt\"}".to_string()),
            Some("contents".to_string()),
            ToolExecutionStatus::Succeeded,
            Some(2),
            Some(1),
            Some("none".to_string()),
            Some("completed".to_string()),
        );
        let encoded = serde_json::to_value(&tool).expect("serialize tool execution");
        let decoded = serde_json::from_value::<ToolExecutionDiagnostic>(encoded)
            .expect("deserialize tool execution");
        assert_eq!(decoded, tool);
    }

    #[test]
    fn prompt_component_deserialization_enforces_its_field_limits() {
        let component = PromptComponentDiagnostic::new(
            PromptComponentKind::Instruction,
            "policy",
            "content",
            None,
        );
        let payload = serde_json::to_value(component).expect("serialize prompt component");

        for (field, max_bytes) in [
            ("label", DIAGNOSTIC_LABEL_MAX_BYTES),
            ("content", PROMPT_COMPONENT_CONTENT_MAX_BYTES),
        ] {
            let mut oversized = payload.clone();
            oversized[field] = oversized_bounded_text(max_bytes);

            let error = serde_json::from_value::<PromptComponentDiagnostic>(oversized)
                .expect_err("oversized prompt component field must be rejected");
            assert!(error.to_string().contains("field byte limit"));
        }
    }

    #[test]
    fn tool_execution_deserialization_enforces_its_field_limits() {
        let tool = ToolExecutionDiagnostic::new(
            CapabilityActivityId::new(),
            None,
            "filesystem.read",
            Some("arguments".to_string()),
            Some("result".to_string()),
            ToolExecutionStatus::Failed,
            None,
            None,
            Some("provider".to_string()),
            Some("failed".to_string()),
        );
        let payload = serde_json::to_value(tool).expect("serialize tool execution");

        for (field, max_bytes) in [
            ("capability_name", DIAGNOSTIC_LABEL_MAX_BYTES),
            ("arguments", TOOL_ARGUMENTS_MAX_BYTES),
            ("result", TOOL_RESULT_MAX_BYTES),
            ("failure_category", DIAGNOSTIC_LABEL_MAX_BYTES),
            ("failure_summary", DIAGNOSTIC_SUMMARY_MAX_BYTES),
        ] {
            let mut oversized = payload.clone();
            oversized[field] = oversized_bounded_text(max_bytes);

            let error = serde_json::from_value::<ToolExecutionDiagnostic>(oversized)
                .expect_err("oversized tool execution field must be rejected");
            assert!(error.to_string().contains("field byte limit"));
        }
    }

    #[test]
    fn tool_execution_deserialization_rejects_inconsistent_result_size() {
        let tool = ToolExecutionDiagnostic::new(
            CapabilityActivityId::new(),
            None,
            "filesystem.read",
            None,
            Some("result".to_string()),
            ToolExecutionStatus::Succeeded,
            None,
            None,
            None,
            None,
        );
        let mut payload = serde_json::to_value(tool).expect("serialize tool execution");
        payload["output_bytes"] = serde_json::json!(1);

        let error = serde_json::from_value::<ToolExecutionDiagnostic>(payload)
            .expect_err("inconsistent tool result byte metadata must be rejected");
        assert!(error.to_string().contains("byte metadata is inconsistent"));
    }

    #[test]
    fn bounded_text_deserialization_rejects_the_global_retained_byte_limit() {
        let content = "x".repeat(RECONSTRUCTED_PROMPT_MAX_BYTES + 1);
        let payload = serde_json::json!({
            "content": content,
            "original_bytes": RECONSTRUCTED_PROMPT_MAX_BYTES + 1,
            "truncated": false,
        });

        let error = serde_json::from_value::<BoundedDiagnosticText>(payload)
            .expect_err("oversized diagnostic text must be rejected");

        assert!(error.to_string().contains("maximum retained byte length"));
    }

    #[test]
    fn bounded_text_deserialization_rejects_inconsistent_metadata() {
        for payload in [
            serde_json::json!({
                "content": "ok",
                "original_bytes": 1,
                "truncated": false,
            }),
            serde_json::json!({
                "content": "ok",
                "original_bytes": 2,
                "truncated": true,
            }),
            serde_json::json!({
                "content": "ok",
                "original_bytes": 3,
                "truncated": false,
            }),
        ] {
            serde_json::from_value::<BoundedDiagnosticText>(payload)
                .expect_err("inconsistent diagnostic text metadata must be rejected");
        }
    }

    #[test]
    fn diagnostic_update_deserialization_enforces_the_owning_field_limit() {
        let update = DiagnosticUpdateKind::ToolExecutionUpdated {
            activity_id: CapabilityActivityId::new(),
            model_call_id: None,
            capability_name: BoundedDiagnosticText::label("filesystem.read"),
            status: ToolExecutionStatus::Succeeded,
            duration_ms: None,
            output_bytes: None,
            result_truncated: false,
        };
        let mut payload = serde_json::to_value(update).expect("serialize diagnostic update");
        let oversized_label = "x".repeat(DIAGNOSTIC_LABEL_MAX_BYTES + 1);
        payload["data"]["capability_name"] = serde_json::json!({
            "content": oversized_label,
            "original_bytes": DIAGNOSTIC_LABEL_MAX_BYTES + 1,
            "truncated": false,
        });

        let error = serde_json::from_value::<DiagnosticUpdateKind>(payload)
            .expect_err("oversized capability name must be rejected");

        assert!(error.to_string().contains("field byte limit"));
    }

    #[test]
    fn diagnostic_stats_deserialization_rejects_too_many_model_counts() {
        let update = DiagnosticUpdateKind::Stats(SessionDiagnosticStats::default());
        let mut payload = serde_json::to_value(update).expect("serialize diagnostic stats");
        payload["data"]["calls_per_model"] = serde_json::to_value(
            (0..=MAX_MODELS_IN_STATS)
                .map(|index| DiagnosticModelCount::new(format!("model-{index}"), 1))
                .collect::<Vec<_>>(),
        )
        .expect("serialize model counts");

        let error = serde_json::from_value::<DiagnosticUpdateKind>(payload)
            .expect_err("too many model counts must be rejected");

        assert!(error.to_string().contains("item limit"));
    }

    #[test]
    fn bounded_text_debug_never_exposes_content() {
        let bounded = BoundedDiagnosticText::tool_result("super-secret-value");
        let debug = format!("{bounded:?}");
        assert!(!debug.contains("super-secret-value"));
        assert!(debug.contains("diagnostic content redacted"));
    }

    #[test]
    fn prompt_constructor_applies_component_and_skill_caps() {
        let components = (0..=MAX_PROMPT_COMPONENTS)
            .map(|index| {
                PromptComponentDiagnostic::new(
                    PromptComponentKind::Instruction,
                    format!("component-{index}"),
                    "x",
                    Some(1),
                )
            })
            .collect();
        let skills = (0..=MAX_ACTIVE_SKILLS)
            .map(|index| format!("skill-{index}"))
            .collect();
        let prompt = PromptDiagnostic::new(
            Utc::now(),
            components,
            "prompt",
            Some(1),
            1,
            0,
            1,
            skills,
            0,
            None,
            None,
            None,
        );
        assert_eq!(prompt.components.len(), MAX_PROMPT_COMPONENTS);
        assert!(prompt.components_truncated);
        assert_eq!(prompt.active_skills.len(), MAX_ACTIVE_SKILLS);
        assert!(prompt.active_skills_truncated);
    }

    #[test]
    fn prompt_component_content_truncation_does_not_claim_the_list_was_shortened() {
        let prompt = PromptDiagnostic::new(
            Utc::now(),
            vec![PromptComponentDiagnostic::new(
                PromptComponentKind::Instruction,
                "large-component",
                "x".repeat(PROMPT_COMPONENT_CONTENT_MAX_BYTES + 1),
                None,
            )],
            "prompt",
            None,
            1,
            0,
            1,
            Vec::new(),
            0,
            None,
            None,
            None,
        );

        assert_eq!(prompt.components.len(), 1);
        assert!(!prompt.components_truncated);
        assert!(prompt.components[0].content.truncated());
        assert!(prompt.any_content_truncated());
    }

    #[test]
    fn prompt_constructor_enforces_the_total_component_byte_budget() {
        let mut components = (0..3)
            .map(|index| {
                PromptComponentDiagnostic::new(
                    PromptComponentKind::Instruction,
                    format!("full-{index}"),
                    "x".repeat(PROMPT_COMPONENT_CONTENT_MAX_BYTES),
                    None,
                )
            })
            .collect::<Vec<_>>();
        components.push(PromptComponentDiagnostic::new(
            PromptComponentKind::Instruction,
            "partial-prefix",
            "x".repeat(40_000),
            None,
        ));
        components.push(PromptComponentDiagnostic::new(
            PromptComponentKind::Instruction,
            "partially-retained",
            "x".repeat(PROMPT_COMPONENT_CONTENT_MAX_BYTES),
            None,
        ));
        components.push(PromptComponentDiagnostic::new(
            PromptComponentKind::Instruction,
            "dropped",
            "x",
            None,
        ));

        let prompt = PromptDiagnostic::new(
            Utc::now(),
            components,
            "prompt",
            None,
            1,
            0,
            1,
            Vec::new(),
            0,
            None,
            None,
            None,
        );

        assert_eq!(prompt.components.len(), 5);
        assert_eq!(
            prompt
                .components
                .iter()
                .map(|component| component.content.content().len())
                .sum::<usize>(),
            PROMPT_COMPONENT_TOTAL_MAX_BYTES
        );
        assert_eq!(prompt.components[4].content.content().len(), 25_536);
        assert!(prompt.components[4].content.truncated());
        assert!(prompt.components_truncated);
    }

    #[test]
    fn tool_result_uses_its_original_length_over_caller_supplied_output_bytes() {
        assert_eq!(TOOL_RESULT_MAX_BYTES, 50 * 1024);
        let tool = ToolExecutionDiagnostic::new(
            CapabilityActivityId::new(),
            None,
            "filesystem.read",
            None,
            Some("x".repeat(TOOL_RESULT_MAX_BYTES + 1)),
            ToolExecutionStatus::Succeeded,
            None,
            Some(7),
            None,
            None,
        );
        assert!(tool.result_truncated());
        assert_eq!(tool.output_bytes, Some((TOOL_RESULT_MAX_BYTES + 1) as u64));
    }

    #[test]
    fn stats_bound_the_per_model_breakdown_and_mark_truncation() {
        let stats = SessionDiagnosticStats {
            calls_per_model: (0..=MAX_MODELS_IN_STATS)
                .map(|index| DiagnosticModelCount::new(format!("model-{index}"), 1))
                .collect(),
            ..SessionDiagnosticStats::default()
        }
        .into_bounded();
        assert_eq!(stats.calls_per_model.len(), MAX_MODELS_IN_STATS);
        assert!(stats.calls_per_model_truncated);
    }
}

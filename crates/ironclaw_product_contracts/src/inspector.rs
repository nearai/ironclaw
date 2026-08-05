//! Operator-only diagnostic vocabulary for the Web Debug Inspector.
//!
//! These output-only DTOs cross the product boundary through the dedicated
//! inspection surface. They are deliberately separate from product projection
//! events: raw prompt components and tool details must never enter the normal
//! product stream.

use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    ids::{TenantId, ThreadId, UserId},
    turn::{CapabilityActivityId, TurnRunId},
};
use serde::Serialize;
use uuid::Uuid;

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
pub const DEFAULT_MAX_MODEL_CALLS_PER_RUN: usize = 128;
pub const DEFAULT_MAX_TOOL_EXECUTIONS_PER_RUN: usize = 16;
pub const DEFAULT_MAX_RETAINED_UPDATES_PER_RUN: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct DiagnosticStreamId(Uuid);

impl DiagnosticStreamId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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

    pub fn content(&self) -> &str {
        &self.content
    }

    pub const fn original_bytes(&self) -> u64 {
        self.original_bytes
    }

    pub const fn truncated(&self) -> bool {
        self.truncated
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptComponentDiagnostic {
    pub kind: PromptComponentKind,
    pub label: BoundedDiagnosticText,
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
        let original_component_count = components.len();
        let mut remaining = PROMPT_COMPONENT_TOTAL_MAX_BYTES;
        let mut bounded_components =
            Vec::with_capacity(original_component_count.min(MAX_PROMPT_COMPONENTS));
        let mut components_truncated = original_component_count > MAX_PROMPT_COMPONENTS;
        for mut component in components.into_iter().take(MAX_PROMPT_COMPONENTS) {
            if remaining == 0 {
                components_truncated = true;
                break;
            }
            let retained = component.content.content().len().min(remaining);
            component.content = component.content.rebound(retained);
            components_truncated |= component.content.truncated();
            remaining = remaining.saturating_sub(component.content.content().len());
            bounded_components.push(component);
        }

        let active_skills_truncated = active_skills.len() > MAX_ACTIVE_SKILLS;
        let active_skills = active_skills
            .into_iter()
            .take(MAX_ACTIVE_SKILLS)
            .map(BoundedDiagnosticText::label)
            .collect();

        Self {
            captured_at,
            components: bounded_components,
            components_truncated,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ModelTokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectorModelCallStatus {
    Started,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelCallDiagnostic {
    pub call_id: DiagnosticModelCallId,
    pub iteration: u32,
    pub requested_model: BoundedDiagnosticText,
    pub effective_model: Option<BoundedDiagnosticText>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub status: InspectorModelCallStatus,
    pub usage: Option<ModelTokenUsage>,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionStatus {
    Started,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

    pub fn result_truncated(&self) -> bool {
        self.result
            .as_ref()
            .is_some_and(BoundedDiagnosticText::truncated)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticActivityEvent {
    pub occurred_at: DateTime<Utc>,
    pub kind: DiagnosticActivityKind,
    pub iteration: Option<u32>,
    pub activity_id: Option<CapabilityActivityId>,
    pub model_call_id: Option<DiagnosticModelCallId>,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticActivityEntry {
    pub sequence: DiagnosticSequence,
    pub event: DiagnosticActivityEvent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiagnosticMetricTotal {
    pub known_total: u64,
    pub unavailable_samples: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticModelCount {
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SessionDiagnosticStats {
    pub total_model_calls: u64,
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
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum DiagnosticUpdateKind {
    PromptUpdated {
        component_count: usize,
        total_estimated_tokens: Option<u64>,
        truncated: bool,
    },
    ModelCall(ModelCallDiagnostic),
    ToolExecutionUpdated {
        activity_id: CapabilityActivityId,
        model_call_id: Option<DiagnosticModelCallId>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_text_preserves_utf8_and_reports_original_size() {
        let value = "€".repeat(TOOL_RESULT_MAX_BYTES);
        let bounded = BoundedDiagnosticText::tool_result(value.clone());
        assert!(bounded.truncated());
        assert!(bounded.content().len() <= TOOL_RESULT_MAX_BYTES);
        assert!(std::str::from_utf8(bounded.content().as_bytes()).is_ok());
        assert_eq!(bounded.original_bytes(), value.len() as u64);
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
    fn tool_result_uses_the_fifty_kibibyte_contract() {
        assert_eq!(TOOL_RESULT_MAX_BYTES, 50 * 1024);
        let tool = ToolExecutionDiagnostic::new(
            CapabilityActivityId::new(),
            None,
            "filesystem.read",
            None,
            Some("x".repeat(TOOL_RESULT_MAX_BYTES + 1)),
            ToolExecutionStatus::Succeeded,
            None,
            Some((TOOL_RESULT_MAX_BYTES + 1) as u64),
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

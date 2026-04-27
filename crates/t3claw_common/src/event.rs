//! Application-wide event types.
//!
//! `AppEvent` is the real-time event protocol used across the entire
//! application.  The web gateway serialises these to SSE / WebSocket
//! frames, but other subsystems (agent loop, orchestrator, extensions)
//! produce and consume them too.

use crate::identity::ExtensionName;
use serde::{Deserialize, Serialize};

/// Terminal status of a sandbox job's `JobResult` event.
///
/// Previously transported as a `String` (`"completed"` / `"failed"` /
/// `"cancelled"`) where producers and consumers agreed by convention only,
/// with no compiler enforcement — see bugs #2570, #2531, #2517 for
/// variant drift that a typed enum prevents.
///
/// Wire format is snake_case, matching the legacy string values so
/// existing SSE consumers (browser clients, external integrations) need
/// no changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobResultStatus {
    Completed,
    Failed,
    Cancelled,
    /// Worker timeout / stuck-state path. Emitted when a job's context
    /// is transitioned to `JobState::Stuck` (see `worker/job.rs`
    /// `mark_stuck`). Distinct from `Failed` so the UI and analytics
    /// can surface recovery-eligible runs separately from hard errors.
    Stuck,
}

impl JobResultStatus {
    /// Returns `true` only for `Completed` — matches the prior
    /// `status == "completed"` predicate at consumer sites.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed)
    }

    /// Canonical wire-format string (snake_case), stable for log lines
    /// and user-facing messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Stuck => "stuck",
        }
    }
}

impl std::fmt::Display for JobResultStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an untyped string into a
/// [`JobResultStatus`] fails. Exposed as a typed error so boundaries
/// (container JSON payloads, legacy persisted rows) can log a warning
/// and fall back to `Failed` without swallowing the original input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobResultStatusParseError {
    pub value: String,
}

impl std::fmt::Display for JobResultStatusParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown JobResultStatus value: {:?}", self.value)
    }
}

impl std::error::Error for JobResultStatusParseError {}

/// Parse a wire-format string into a [`JobResultStatus`].
///
/// Accepts the canonical snake_case variants (`"completed"`, `"failed"`,
/// `"cancelled"`, `"stuck"`) plus the legacy alias `"error"` → `Failed`
/// that pre-refactor producers (`claude_bridge`, `acp_bridge`) still
/// emit on the wire. Input is trimmed and matched case-insensitively
/// (ASCII-only) so slightly-varied payloads — `"  COMPLETED  "`,
/// `"Failed"` — deserialize cleanly instead of falling to the
/// `Err`-then-default path in consumers.
///
/// Empty / whitespace-only input returns `Err` so the caller can log a
/// distinct warning for "missing status" vs "unknown status".
impl std::str::FromStr for JobResultStatus {
    type Err = JobResultStatusParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(JobResultStatusParseError {
                value: s.to_string(),
            });
        }
        if trimmed.eq_ignore_ascii_case("completed") {
            Ok(Self::Completed)
        } else if trimmed.eq_ignore_ascii_case("failed") {
            Ok(Self::Failed)
        } else if trimmed.eq_ignore_ascii_case("cancelled") {
            Ok(Self::Cancelled)
        } else if trimmed.eq_ignore_ascii_case("stuck") {
            Ok(Self::Stuck)
        } else if trimmed.eq_ignore_ascii_case("error") {
            // Legacy alias — pre-refactor claude_bridge / acp_bridge
            // producers emit `"error"`. Keep the alias so those wire
            // payloads deserialize into `Failed` instead of hitting the
            // consumer's default-on-unknown branch (which also emits a
            // warn log — spammy for a known, expected value).
            Ok(Self::Failed)
        } else {
            Err(JobResultStatusParseError {
                value: s.to_string(),
            })
        }
    }
}

/// A single step in a plan progress update (SSE DTO).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStepDto {
    pub index: usize,
    pub title: String,
    /// One of: "pending", "in_progress", "completed", "failed".
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// A single tool decision in a reasoning update (SSE DTO).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDecisionDto {
    pub tool_name: String,
    pub rationale: String,
}

impl ToolDecisionDto {
    /// Parse a list of tool decisions from a JSON array value.
    pub fn from_json_array(value: &serde_json::Value) -> Vec<Self> {
        value
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|d| {
                        Some(Self {
                            tool_name: d.get("tool_name")?.as_str()?.to_string(),
                            rationale: d.get("rationale")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStateDto {
    SetupRequired,
    AuthRequired,
    PairingRequired,
    Ready,
    Failed,
}

impl OnboardingStateDto {
    /// Build the canonical `AppEvent::OnboardingState` for a
    /// pairing-required transition.
    ///
    /// `auth_url` and `setup_url` are always `None` for pairing —
    /// forcing construction through this function prevents the three
    /// emit sites (auth-token submit, setup-handler submit, activation
    /// post-pairing) from silently disagreeing when new fields land on
    /// `AppEvent::OnboardingState`.
    pub fn pairing_required(
        extension_name: ExtensionName,
        request_id: Option<String>,
        thread_id: Option<String>,
        message: Option<String>,
        instructions: Option<String>,
        onboarding: Option<serde_json::Value>,
    ) -> AppEvent {
        AppEvent::OnboardingState {
            extension_name,
            state: Self::PairingRequired,
            request_id,
            message,
            instructions,
            auth_url: None,
            setup_url: None,
            onboarding,
            thread_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AppEvent {
    #[serde(rename = "response")]
    Response { content: String, thread_id: String },
    #[serde(rename = "thinking")]
    Thinking {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "tool_started")]
    ToolStarted {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "tool_completed")]
    ToolCompleted {
        name: String,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parameters: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        name: String,
        preview: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "stream_chunk")]
    StreamChunk {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "status")]
    Status {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "job_started")]
    JobStarted {
        job_id: String,
        title: String,
        browse_url: String,
    },
    #[serde(rename = "approval_needed")]
    ApprovalNeeded {
        request_id: String,
        tool_name: String,
        description: String,
        parameters: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        /// Whether the "always" auto-approve option should be shown.
        allow_always: bool,
    },
    #[serde(rename = "onboarding_state")]
    OnboardingState {
        extension_name: ExtensionName,
        state: OnboardingStateDto,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        setup_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        onboarding: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "gate_required")]
    GateRequired {
        request_id: String,
        gate_name: String,
        tool_name: String,
        description: String,
        parameters: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        extension_name: Option<ExtensionName>,
        resume_kind: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "gate_resolved")]
    GateResolved {
        request_id: String,
        gate_name: String,
        tool_name: String,
        resolution: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "error")]
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "heartbeat")]
    Heartbeat,

    // Sandbox job streaming events (worker + Claude Code bridge)
    #[serde(rename = "job_message")]
    JobMessage {
        job_id: String,
        role: String,
        content: String,
    },
    #[serde(rename = "job_tool_use")]
    JobToolUse {
        job_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "job_tool_result")]
    JobToolResult {
        job_id: String,
        tool_name: String,
        output: String,
    },
    #[serde(rename = "job_status")]
    JobStatus { job_id: String, message: String },
    #[serde(rename = "job_result")]
    JobResult {
        job_id: String,
        status: JobResultStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fallback_deliverable: Option<serde_json::Value>,
    },

    /// An image was generated by a tool.
    #[serde(rename = "image_generated")]
    ImageGenerated {
        event_id: String,
        data_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },

    /// Suggested follow-up messages for the user.
    #[serde(rename = "suggestions")]
    Suggestions {
        suggestions: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },

    /// Per-turn token usage and cost summary.
    #[serde(rename = "turn_cost")]
    TurnCost {
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },

    /// Skills activated for a conversation turn.
    ///
    /// `feedback` is a list of human-readable notes about the
    /// activation (e.g. "chain-loaded from code-review", "ceo-setup
    /// excluded by setup marker"). May be empty — `skip_serializing_if`
    /// keeps the SSE payload lean for the common no-note case and
    /// preserves wire-format backwards compatibility.
    #[serde(rename = "skill_activated")]
    SkillActivated {
        skill_names: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        feedback: Vec<String>,
    },

    /// Extension activation status change (WASM channels).
    #[serde(rename = "extension_status")]
    ExtensionStatus {
        extension_name: ExtensionName,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Agent reasoning update (why it chose specific tools).
    #[serde(rename = "reasoning_update")]
    ReasoningUpdate {
        narrative: String,
        decisions: Vec<ToolDecisionDto>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },

    /// Reasoning update for a sandbox job.
    #[serde(rename = "job_reasoning")]
    JobReasoning {
        job_id: String,
        narrative: String,
        decisions: Vec<ToolDecisionDto>,
    },

    /// Full (non-truncated) tool output (verbose/debug mode only).
    #[serde(rename = "tool_result_full")]
    ToolResultFull {
        name: String,
        output: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },

    /// Per-LLM-call metrics with model, tokens, and timing (verbose/debug mode only).
    #[serde(rename = "turn_metrics")]
    TurnMetrics {
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        model: String,
        duration_ms: u64,
        iteration: usize,
    },

    // ── Engine v2 thread lifecycle events ──
    /// Engine thread changed state (e.g. Running → Completed).
    #[serde(rename = "thread_state_changed")]
    ThreadStateChanged {
        thread_id: String,
        from_state: String,
        to_state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// A child thread was spawned by a parent thread.
    #[serde(rename = "child_thread_spawned")]
    ChildThreadSpawned {
        parent_thread_id: String,
        child_thread_id: String,
        goal: String,
    },

    /// A mission spawned a new thread.
    #[serde(rename = "mission_thread_spawned")]
    MissionThreadSpawned {
        mission_id: String,
        thread_id: String,
        mission_name: String,
    },

    /// Plan progress update — full checklist snapshot.
    ///
    /// Emitted when a plan is created, approved, or when any step changes
    /// status. The UI replaces the entire step list on each event.
    #[serde(rename = "plan_update")]
    PlanUpdate {
        /// Plan identifier (MemoryDoc ID or slug).
        plan_id: String,
        /// Plan title.
        title: String,
        /// Overall status: "draft", "approved", "executing", "completed", "failed".
        status: String,
        /// Full step checklist (not incremental — UI replaces entire list).
        steps: Vec<PlanStepDto>,
        /// Associated mission ID (once approved and executing).
        #[serde(skip_serializing_if = "Option::is_none")]
        mission_id: Option<String>,
        /// Thread scope for SSE filtering.
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
}

impl AppEvent {
    /// The wire-format event type string (matches the `#[serde(rename)]` value).
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::Response { .. } => "response",
            Self::Thinking { .. } => "thinking",
            Self::ToolStarted { .. } => "tool_started",
            Self::ToolCompleted { .. } => "tool_completed",
            Self::ToolResult { .. } => "tool_result",
            Self::StreamChunk { .. } => "stream_chunk",
            Self::Status { .. } => "status",
            Self::JobStarted { .. } => "job_started",
            Self::ApprovalNeeded { .. } => "approval_needed",
            Self::OnboardingState { .. } => "onboarding_state",
            Self::GateRequired { .. } => "gate_required",
            Self::GateResolved { .. } => "gate_resolved",
            Self::Error { .. } => "error",
            Self::Heartbeat => "heartbeat",
            Self::JobMessage { .. } => "job_message",
            Self::JobToolUse { .. } => "job_tool_use",
            Self::JobToolResult { .. } => "job_tool_result",
            Self::JobStatus { .. } => "job_status",
            Self::JobResult { .. } => "job_result",
            Self::ImageGenerated { .. } => "image_generated",
            Self::Suggestions { .. } => "suggestions",
            Self::TurnCost { .. } => "turn_cost",
            Self::SkillActivated { .. } => "skill_activated",
            Self::ExtensionStatus { .. } => "extension_status",
            Self::ReasoningUpdate { .. } => "reasoning_update",
            Self::JobReasoning { .. } => "job_reasoning",
            Self::ToolResultFull { .. } => "tool_result_full",
            Self::TurnMetrics { .. } => "turn_metrics",
            Self::ThreadStateChanged { .. } => "thread_state_changed",
            Self::ChildThreadSpawned { .. } => "child_thread_spawned",
            Self::MissionThreadSpawned { .. } => "mission_thread_spawned",
            Self::PlanUpdate { .. } => "plan_update",
        }
    }

    /// Whether this event should only be delivered to verbose/debug subscribers.
    pub fn is_verbose_only(&self) -> bool {
        matches!(self, Self::ToolResultFull { .. } | Self::TurnMetrics { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `event_type()` returns the same string as the serde
    /// `"type"` field for every variant.  This catches drift between the
    /// `#[serde(rename)]` attributes and the manual match arms.
    #[test]
    fn event_type_matches_serde_type_field() {
        let variants: Vec<AppEvent> = vec![
            AppEvent::Response {
                content: String::new(),
                thread_id: String::new(),
            },
            AppEvent::Thinking {
                message: String::new(),
                thread_id: None,
            },
            AppEvent::ToolStarted {
                name: String::new(),
                detail: None,
                call_id: None,
                thread_id: None,
            },
            AppEvent::ToolCompleted {
                name: String::new(),
                success: true,
                error: None,
                parameters: None,
                call_id: None,
                duration_ms: None,
                thread_id: None,
            },
            AppEvent::ToolResult {
                name: String::new(),
                preview: String::new(),
                call_id: None,
                thread_id: None,
            },
            AppEvent::StreamChunk {
                content: String::new(),
                thread_id: None,
            },
            AppEvent::Status {
                message: String::new(),
                thread_id: None,
            },
            AppEvent::JobStarted {
                job_id: String::new(),
                title: String::new(),
                browse_url: String::new(),
            },
            AppEvent::ApprovalNeeded {
                request_id: String::new(),
                tool_name: String::new(),
                description: String::new(),
                parameters: String::new(),
                thread_id: None,
                allow_always: false,
            },
            AppEvent::OnboardingState {
                extension_name: ExtensionName::from_trusted(String::new()),
                state: OnboardingStateDto::AuthRequired,
                request_id: None,
                message: None,
                instructions: None,
                auth_url: None,
                setup_url: None,
                onboarding: None,
                thread_id: None,
            },
            AppEvent::GateRequired {
                request_id: String::new(),
                gate_name: String::new(),
                tool_name: String::new(),
                description: String::new(),
                parameters: String::new(),
                extension_name: None,
                resume_kind: serde_json::Value::Null,
                thread_id: None,
            },
            AppEvent::Error {
                message: String::new(),
                thread_id: None,
            },
            AppEvent::Heartbeat,
            AppEvent::JobMessage {
                job_id: String::new(),
                role: String::new(),
                content: String::new(),
            },
            AppEvent::JobToolUse {
                job_id: String::new(),
                tool_name: String::new(),
                input: serde_json::Value::Null,
            },
            AppEvent::JobToolResult {
                job_id: String::new(),
                tool_name: String::new(),
                output: String::new(),
            },
            AppEvent::JobStatus {
                job_id: String::new(),
                message: String::new(),
            },
            AppEvent::JobResult {
                job_id: String::new(),
                status: JobResultStatus::Completed,
                session_id: None,
                fallback_deliverable: None,
            },
            AppEvent::ImageGenerated {
                event_id: String::new(),
                data_url: String::new(),
                path: None,
                thread_id: None,
            },
            AppEvent::Suggestions {
                suggestions: vec![],
                thread_id: None,
            },
            AppEvent::TurnCost {
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: String::new(),
                thread_id: None,
            },
            AppEvent::SkillActivated {
                skill_names: vec![],
                thread_id: None,
                feedback: vec![],
            },
            AppEvent::ExtensionStatus {
                extension_name: ExtensionName::from_trusted(String::new()),
                status: String::new(),
                message: None,
            },
            AppEvent::ReasoningUpdate {
                narrative: String::new(),
                decisions: vec![],
                thread_id: None,
            },
            AppEvent::JobReasoning {
                job_id: String::new(),
                narrative: String::new(),
                decisions: vec![],
            },
            AppEvent::ToolResultFull {
                name: String::new(),
                output: String::new(),
                truncated: None,
                call_id: None,
                thread_id: None,
            },
            AppEvent::TurnMetrics {
                thread_id: None,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                model: String::new(),
                duration_ms: 0,
                iteration: 0,
            },
            AppEvent::ThreadStateChanged {
                thread_id: String::new(),
                from_state: String::new(),
                to_state: String::new(),
                reason: None,
            },
            AppEvent::ChildThreadSpawned {
                parent_thread_id: String::new(),
                child_thread_id: String::new(),
                goal: String::new(),
            },
            AppEvent::MissionThreadSpawned {
                mission_id: String::new(),
                thread_id: String::new(),
                mission_name: String::new(),
            },
            AppEvent::PlanUpdate {
                plan_id: String::new(),
                title: String::new(),
                status: String::new(),
                steps: vec![],
                mission_id: None,
                thread_id: None,
            },
        ];

        for variant in &variants {
            let json: serde_json::Value = serde_json::to_value(variant).unwrap();
            let serde_type = json["type"].as_str().unwrap();
            assert_eq!(
                variant.event_type(),
                serde_type,
                "event_type() mismatch for variant: {:?}",
                variant
            );
        }
    }

    #[test]
    fn pairing_required_constructor_sets_invariant_fields() {
        let event = OnboardingStateDto::pairing_required(
            ExtensionName::new("telegram").unwrap(),
            Some("req-1".to_string()),
            Some("thread-1".to_string()),
            Some("Paired!".to_string()),
            Some("Send /start to the bot.".to_string()),
            Some(serde_json::json!({ "pairing_code": "ABC123" })),
        );

        match event {
            AppEvent::OnboardingState {
                extension_name,
                state,
                request_id,
                message,
                instructions,
                auth_url,
                setup_url,
                onboarding,
                thread_id,
            } => {
                assert_eq!(extension_name, "telegram");
                assert_eq!(state, OnboardingStateDto::PairingRequired);
                assert_eq!(request_id.as_deref(), Some("req-1"));
                assert_eq!(thread_id.as_deref(), Some("thread-1"));
                assert_eq!(message.as_deref(), Some("Paired!"));
                assert_eq!(instructions.as_deref(), Some("Send /start to the bot."));
                assert!(auth_url.is_none(), "auth_url must be None for pairing");
                assert!(setup_url.is_none(), "setup_url must be None for pairing");
                assert_eq!(
                    onboarding,
                    Some(serde_json::json!({ "pairing_code": "ABC123" }))
                );
            }
            other => panic!("expected OnboardingState, got {other:?}"),
        }
    }

    #[test]
    fn pairing_required_constructor_serializes_to_onboarding_state_event() {
        let event = OnboardingStateDto::pairing_required(
            ExtensionName::new("telegram").unwrap(),
            None,
            None,
            None,
            None,
            None,
        );
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "onboarding_state");
        assert_eq!(json["state"], "pairing_required");
        assert_eq!(json["extension_name"], "telegram");
        assert!(json.get("auth_url").is_none());
        assert!(json.get("setup_url").is_none());
    }

    #[test]
    fn round_trip_deserialize() {
        let original = AppEvent::Response {
            content: "hello".to_string(),
            thread_id: "t1".to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: AppEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.event_type(), "response");
    }

    // === JobResultStatus: wire format + parsing ===
    // Regression for the stringly-typed JobResult.status field that
    // previously admitted arbitrary values and forced consumers to
    // compare via `status == "completed"`. The enum preserves the
    // snake_case wire format so SSE/browser clients need no update.

    #[test]
    fn job_result_status_serializes_as_snake_case() {
        use std::str::FromStr;

        assert_eq!(
            serde_json::to_string(&JobResultStatus::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&JobResultStatus::Failed).unwrap(),
            "\"failed\""
        );
        assert_eq!(
            serde_json::to_string(&JobResultStatus::Cancelled).unwrap(),
            "\"cancelled\""
        );
        assert_eq!(
            serde_json::to_string(&JobResultStatus::Stuck).unwrap(),
            "\"stuck\""
        );

        assert_eq!(
            JobResultStatus::from_str("completed").unwrap(),
            JobResultStatus::Completed
        );
        assert_eq!(
            JobResultStatus::from_str("failed").unwrap(),
            JobResultStatus::Failed
        );
        assert_eq!(
            JobResultStatus::from_str("cancelled").unwrap(),
            JobResultStatus::Cancelled
        );
        assert_eq!(
            JobResultStatus::from_str("stuck").unwrap(),
            JobResultStatus::Stuck
        );
        assert!(JobResultStatus::from_str("unknown_xyz").is_err());
    }

    #[test]
    fn job_result_status_accepts_error_alias_as_failed() {
        use std::str::FromStr;

        // Legacy `claude_bridge` / `acp_bridge` producers emit
        // `"error"` on the wire. The alias keeps those payloads
        // deserializing into `Failed` without touching the producers.
        assert_eq!(
            JobResultStatus::from_str("error").unwrap(),
            JobResultStatus::Failed
        );
        assert_eq!(
            JobResultStatus::from_str("ERROR").unwrap(),
            JobResultStatus::Failed
        );
    }

    #[test]
    fn job_result_status_from_str_is_case_insensitive_and_trims() {
        use std::str::FromStr;

        assert_eq!(
            JobResultStatus::from_str("  COMPLETED  ").unwrap(),
            JobResultStatus::Completed
        );
        assert_eq!(
            JobResultStatus::from_str("Failed").unwrap(),
            JobResultStatus::Failed
        );
        assert_eq!(
            JobResultStatus::from_str("\tCancelled\n").unwrap(),
            JobResultStatus::Cancelled
        );
        // Empty / whitespace-only inputs are a distinct error so the
        // caller can distinguish "missing status" from "unknown status".
        assert!(JobResultStatus::from_str("").is_err());
        assert!(JobResultStatus::from_str("   ").is_err());
    }

    #[test]
    fn job_result_status_parse_error_preserves_original_input() {
        use std::str::FromStr;

        // Keep whitespace / casing in the error's `value` field for
        // debugging — trimming is only for matching, not preservation.
        let err = JobResultStatus::from_str("  GARBAGE  ").unwrap_err();
        assert_eq!(err.value, "  GARBAGE  ");
    }

    #[test]
    fn job_result_event_preserves_snake_case_wire_format() {
        // Producers wrote `"completed"` / `"failed"` as raw strings
        // before this refactor. The enum must emit the same wire bytes
        // for backwards compat with SSE and external consumers.
        let event = AppEvent::JobResult {
            job_id: "job-1".to_string(),
            status: JobResultStatus::Completed,
            session_id: None,
            fallback_deliverable: None,
        };
        let json: serde_json::Value = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "job_result");
        assert_eq!(json["status"], "completed");

        // Round-trip: a wire payload written by an older producer
        // (still as a JSON string) must deserialize cleanly.
        let raw = serde_json::json!({
            "type": "job_result",
            "job_id": "job-2",
            "status": "failed",
        });
        let parsed: AppEvent = serde_json::from_value(raw).unwrap();
        match parsed {
            AppEvent::JobResult { status, .. } => {
                assert_eq!(status, JobResultStatus::Failed);
                assert!(!status.is_success());
            }
            other => panic!("expected JobResult, got {other:?}"),
        }
    }

    #[test]
    fn job_result_status_is_success_only_for_completed() {
        assert!(JobResultStatus::Completed.is_success());
        assert!(!JobResultStatus::Failed.is_success());
        assert!(!JobResultStatus::Cancelled.is_success());
    }
}

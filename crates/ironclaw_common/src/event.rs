//! Application-wide event types.
//!
//! `AppEvent` is the real-time event protocol used across the entire
//! application.  The web gateway serialises these to SSE / WebSocket
//! frames, but other subsystems (agent loop, orchestrator, extensions)
//! produce and consume them too.

use serde::{Deserialize, Serialize};

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
        thread_id: Option<String>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        name: String,
        preview: String,
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
    #[serde(rename = "auth_required")]
    AuthRequired {
        extension_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        setup_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "auth_completed")]
    AuthCompleted {
        extension_name: String,
        success: bool,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "pairing_required")]
    PairingRequired {
        channel: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        onboarding: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "pairing_completed")]
    PairingCompleted {
        channel: String,
        success: bool,
        message: String,
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
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fallback_deliverable: Option<serde_json::Value>,
    },

    /// An image was generated by a tool.
    #[serde(rename = "image_generated")]
    ImageGenerated {
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
    #[serde(rename = "skill_activated")]
    SkillActivated {
        skill_names: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },

    /// Extension activation status change (WASM channels).
    #[serde(rename = "extension_status")]
    ExtensionStatus {
        extension_name: String,
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
            Self::AuthRequired { .. } => "auth_required",
            Self::AuthCompleted { .. } => "auth_completed",
            Self::PairingRequired { .. } => "pairing_required",
            Self::PairingCompleted { .. } => "pairing_completed",
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
                thread_id: None,
            },
            AppEvent::ToolCompleted {
                name: String::new(),
                success: true,
                error: None,
                parameters: None,
                thread_id: None,
            },
            AppEvent::ToolResult {
                name: String::new(),
                preview: String::new(),
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
            AppEvent::AuthRequired {
                extension_name: String::new(),
                instructions: None,
                auth_url: None,
                setup_url: None,
                thread_id: None,
            },
            AppEvent::AuthCompleted {
                extension_name: String::new(),
                success: true,
                message: String::new(),
                thread_id: None,
            },
            AppEvent::PairingRequired {
                channel: String::new(),
                instructions: None,
                onboarding: None,
                thread_id: None,
            },
            AppEvent::PairingCompleted {
                channel: String::new(),
                success: true,
                message: String::new(),
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
                status: String::new(),
                session_id: None,
                fallback_deliverable: None,
            },
            AppEvent::ImageGenerated {
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
            },
            AppEvent::ExtensionStatus {
                extension_name: String::new(),
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
    fn round_trip_deserialize() {
        let original = AppEvent::Response {
            content: "hello".to_string(),
            thread_id: "t1".to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: AppEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.event_type(), "response");
    }
}

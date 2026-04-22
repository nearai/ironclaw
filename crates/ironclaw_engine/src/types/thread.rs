//! Thread — the unit of work.
//!
//! A thread is a bounded task or investigation. It unifies the concepts of
//! Session (interactive conversation), Job (background work), Routine
//! (scheduled execution), and Sub-agent (delegated reasoning) into a single
//! abstraction with a shared state machine.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::capability::LeaseId;
use crate::types::error::EngineError;
use crate::types::event::{EventKind, ThreadEvent};
use crate::types::memory::DocId;
use crate::types::message::ThreadMessage;
use crate::types::project::ProjectId;

use super::{OwnerId, default_user_id};

/// Strongly-typed thread identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadId(pub Uuid);

impl ThreadId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ThreadId {
    fn default() -> Self {
        Self::new()
    }
}

// ── State machine ───────────────────────────────────────────

/// Thread lifecycle state.
///
/// ```text
/// Created → Running → Waiting → Running (resume)
///                   → Suspended → Running (resume)
///                   → Completed → Done
///                   → Failed
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThreadState {
    /// Thread has been created but not yet started.
    Created,
    /// Thread is actively executing steps.
    Running,
    /// Waiting for external input (user approval, child completion).
    Waiting,
    /// Paused by system (resource pressure, priority preemption).
    Suspended,
    /// Execution finished successfully.
    Completed,
    /// Fully finished (terminal).
    Done,
    /// Terminal failure.
    Failed,
}

impl ThreadState {
    /// Check whether a transition to `target` is valid.
    pub fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            // From Created
            (Self::Created, Self::Running)
            | (Self::Created, Self::Failed)
            // From Running
            | (Self::Running, Self::Waiting)
            | (Self::Running, Self::Suspended)
            | (Self::Running, Self::Completed)
            | (Self::Running, Self::Failed)
            // From Waiting
            | (Self::Waiting, Self::Running)
            | (Self::Waiting, Self::Failed)
            // From Suspended
            | (Self::Suspended, Self::Running)
            | (Self::Suspended, Self::Failed)
            // From Completed
            | (Self::Completed, Self::Done)
        )
    }

    /// Whether this state is terminal (no further transitions possible).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }

    /// Whether this state represents active work.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Waiting)
    }
}

// ── Thread type ─────────────────────────────────────────────

/// The nature of the work a thread performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadType {
    /// Interactive conversation with a user.
    Foreground,
    /// Background research or sub-task.
    Research,
    /// Long-running goal that spawns threads over time.
    Mission,
}

// ── Thread configuration ────────────────────────────────────

/// Execution parameters for a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadConfig {
    /// Maximum number of LLM call iterations.
    pub max_iterations: usize,
    /// Maximum wall-clock duration for the thread.
    pub max_duration: Option<std::time::Duration>,
    /// Whether to detect and nudge on tool intent without action calls.
    pub enable_tool_intent_nudge: bool,
    /// Maximum number of tool intent nudges per thread.
    pub max_tool_intent_nudges: u32,

    // ── Execution obligation ──
    /// Whether to require at least one action/code attempt before accepting
    /// a text response. Set by the router when explicit execution intent is
    /// detected in the user's message (e.g. "run it", "fetch the data").
    #[serde(default)]
    pub require_action_attempt: bool,
    /// Maximum corrective nudges when require_action_attempt is true.
    #[serde(default = "default_max_action_requirement_nudges")]
    pub max_action_requirement_nudges: u32,

    // ── Budget controls (Phase 4, from RLM cross-reference) ──
    /// Maximum cumulative input+output tokens before termination.
    pub max_tokens_total: Option<u64>,
    /// Maximum consecutive steps with errors before termination.
    /// Resets to 0 on any successful step (matching official RLM behavior).
    pub max_consecutive_errors: Option<u32>,
    /// Model context limit in tokens (for compaction threshold calculation).
    /// Default: 128,000. Used to trigger compaction at 85% usage.
    pub model_context_limit: usize,
    /// Whether to enable automatic compaction when context grows large.
    pub enable_compaction: bool,
    /// Compaction threshold as fraction of model_context_limit (0.0-1.0).
    /// Default: 0.85 (matching official RLM).
    pub compaction_threshold: f64,
    /// Maximum cumulative USD cost before termination.
    /// Requires the LlmBackend to populate `TokenUsage::cost_usd`.
    pub max_budget_usd: Option<f64>,
    /// Depth of this thread in the recursive call tree.
    /// Root threads are depth 0. Sub-calls via rlm_query() increment depth.
    pub depth: u32,
    /// Maximum recursion depth for rlm_query() sub-calls.
    pub max_depth: u32,
}

impl Default for ThreadConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            max_duration: None,
            enable_tool_intent_nudge: true,
            max_tool_intent_nudges: 2,
            require_action_attempt: false,
            max_action_requirement_nudges: 2,
            max_tokens_total: None,
            max_consecutive_errors: Some(5),
            max_budget_usd: None,
            model_context_limit: 128_000,
            enable_compaction: false,
            compaction_threshold: 0.85,
            depth: 0,
            max_depth: 1,
        }
    }
}

fn default_max_action_requirement_nudges() -> u32 {
    2
}

/// Provenance for a skill that was active during thread execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveSkillProvenance {
    pub doc_id: DocId,
    pub name: String,
    pub version: u32,
    #[serde(default)]
    pub snippet_names: Vec<String>,
    #[serde(default)]
    pub force_activated: bool,
}

const ACTIVE_SKILLS_METADATA_KEY: &str = "active_skills";

// ── Thread ──────────────────────────────────────────────────

/// A thread — the unit of work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: ThreadId,
    pub goal: String,
    /// Short human-readable label for the thread, used by channel UIs
    /// (e.g. the gateway sidebar). `goal` is the execution prompt and
    /// may be a multi-paragraph meta-prompt; `title` is the compact
    /// label the user sees.
    #[serde(default)]
    pub title: Option<String>,
    pub thread_type: ThreadType,
    pub state: ThreadState,
    pub project_id: ProjectId,
    /// Tenant isolation: the user who owns this thread.
    #[serde(default = "default_user_id")]
    pub user_id: String,
    pub parent_id: Option<ThreadId>,
    pub config: ThreadConfig,
    /// User-visible transcript for the thread.
    pub messages: Vec<ThreadMessage>,
    /// Internal execution transcript used by the orchestrator for inference,
    /// tool traces, compaction, and resumable working state.
    #[serde(default)]
    pub internal_messages: Vec<ThreadMessage>,
    pub events: Vec<ThreadEvent>,
    pub capability_leases: Vec<LeaseId>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub step_count: usize,
    pub total_tokens_used: u64,
    /// Cumulative USD cost across all steps.
    pub total_cost_usd: f64,
}

impl Thread {
    /// Create a new thread in the `Created` state.
    pub fn new(
        goal: impl Into<String>,
        thread_type: ThreadType,
        project_id: ProjectId,
        user_id: impl Into<String>,
        config: ThreadConfig,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: ThreadId::new(),
            goal: goal.into(),
            title: None,
            thread_type,
            state: ThreadState::Created,
            project_id,
            user_id: user_id.into(),
            parent_id: None,
            config,
            messages: Vec::new(),
            internal_messages: Vec::new(),
            events: Vec::new(),
            capability_leases: Vec::new(),
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            created_at: now,
            updated_at: now,
            completed_at: None,
            step_count: 0,
            total_tokens_used: 0,
            total_cost_usd: 0.0,
        }
    }

    /// Create a child thread with a parent reference.
    pub fn with_parent(mut self, parent_id: ThreadId) -> Self {
        self.parent_id = Some(parent_id);
        self
    }

    /// Derive a short sidebar title from a free-form user message.
    ///
    /// Takes the first non-empty line, trims leading and trailing
    /// whitespace, and truncates by character count (not bytes) so
    /// multibyte input doesn't panic. Returns `None` if the message is
    /// all whitespace.
    pub fn derive_title_from_message(message: &str) -> Option<String> {
        const MAX_CHARS: usize = 60;
        let trimmed = message.lines().find(|l| !l.trim().is_empty())?.trim();
        // Streaming truncation avoids an O(n) `chars().count()` on long
        // single-line input: take up to MAX_CHARS-1 chars, then peek the
        // rest. If exactly one more char remains, append it (result is
        // MAX_CHARS with no ellipsis); if more remain, append '…'.
        let mut chars = trimmed.chars();
        let mut out: String = chars.by_ref().take(MAX_CHARS - 1).collect();
        if let Some(next) = chars.next() {
            if chars.next().is_none() {
                out.push(next);
            } else {
                out.push('…');
            }
        }
        Some(out)
    }

    pub fn owner_id(&self) -> OwnerId<'_> {
        OwnerId::from_user_id(&self.user_id)
    }

    pub fn is_owned_by(&self, user_id: &str) -> bool {
        self.owner_id().matches_user(user_id)
    }

    /// Persist active skill provenance in thread metadata.
    pub fn set_active_skills(
        &mut self,
        active_skills: &[ActiveSkillProvenance],
    ) -> Result<(), EngineError> {
        let metadata = self
            .metadata
            .as_object_mut()
            .ok_or_else(|| EngineError::Store {
                reason: "thread metadata is not a JSON object".into(),
            })?;
        metadata.insert(
            ACTIVE_SKILLS_METADATA_KEY.into(),
            serde_json::to_value(active_skills).map_err(|e| EngineError::Store {
                reason: format!("failed to serialize active skill provenance: {e}"),
            })?,
        );
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Load active skill provenance from thread metadata.
    pub fn active_skills(&self) -> Vec<ActiveSkillProvenance> {
        self.metadata
            .get(ACTIVE_SKILLS_METADATA_KEY)
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }

    /// Transition to a new state, recording an event.
    pub fn transition_to(
        &mut self,
        new_state: ThreadState,
        reason: Option<String>,
    ) -> Result<(), EngineError> {
        if !self.state.can_transition_to(new_state) {
            return Err(EngineError::InvalidTransition {
                from: self.state,
                to: new_state,
            });
        }

        let event = ThreadEvent::new(
            self.id,
            EventKind::StateChanged {
                from: self.state,
                to: new_state,
                reason,
            },
        );
        self.events.push(event);
        self.state = new_state;
        self.updated_at = Utc::now();

        if new_state == ThreadState::Completed || new_state == ThreadState::Done {
            self.completed_at = Some(Utc::now());
        }

        Ok(())
    }

    /// Add an event to this thread's log.
    pub fn add_event(&mut self, kind: EventKind) {
        self.events.push(ThreadEvent::new(self.id, kind));
        self.updated_at = Utc::now();
    }

    /// Add a message to this thread's conversation.
    pub fn add_message(&mut self, message: ThreadMessage) {
        let preview = if message.content.chars().count() > 80 {
            let p: String = message.content.chars().take(80).collect();
            format!("{p}...")
        } else {
            message.content.clone()
        };
        self.add_event(EventKind::MessageAdded {
            role: format!("{:?}", message.role),
            content_preview: preview,
        });
        self.messages.push(message);
    }

    /// Add a message to the internal execution transcript without exposing it
    /// as a user-visible conversation message.
    pub fn add_internal_message(&mut self, message: ThreadMessage) {
        self.internal_messages.push(message);
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::memory::DocId;

    fn make_thread() -> Thread {
        Thread::new(
            "test goal",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        )
    }

    // ── State machine tests ─────────────────────────────────

    #[test]
    fn default_config_has_concrete_consecutive_error_limit() {
        // Regression: a None default serializes to null and makes the Python
        // orchestrator's `consecutive_action_errors >= max_consecutive_errors + 2`
        // guard crash with TypeError on the first action error, since
        // dict.get("key", 5) returns None (not 5) when the key is present
        // with a null value.
        let config = ThreadConfig::default();
        assert_eq!(config.max_consecutive_errors, Some(5));
    }

    #[test]
    fn created_can_transition_to_running() {
        assert!(ThreadState::Created.can_transition_to(ThreadState::Running));
    }

    #[test]
    fn created_can_transition_to_failed() {
        assert!(ThreadState::Created.can_transition_to(ThreadState::Failed));
    }

    #[test]
    fn created_cannot_transition_to_completed() {
        assert!(!ThreadState::Created.can_transition_to(ThreadState::Completed));
    }

    #[test]
    fn running_can_transition_to_waiting() {
        assert!(ThreadState::Running.can_transition_to(ThreadState::Waiting));
    }

    #[test]
    fn running_can_transition_to_suspended() {
        assert!(ThreadState::Running.can_transition_to(ThreadState::Suspended));
    }

    #[test]
    fn running_can_transition_to_completed() {
        assert!(ThreadState::Running.can_transition_to(ThreadState::Completed));
    }

    #[test]
    fn running_can_transition_to_failed() {
        assert!(ThreadState::Running.can_transition_to(ThreadState::Failed));
    }

    #[test]
    fn waiting_can_resume_to_running() {
        assert!(ThreadState::Waiting.can_transition_to(ThreadState::Running));
    }

    #[test]
    fn suspended_can_resume_to_running() {
        assert!(ThreadState::Suspended.can_transition_to(ThreadState::Running));
    }

    #[test]
    fn completed_can_transition_to_done() {
        assert!(ThreadState::Completed.can_transition_to(ThreadState::Done));
    }

    #[test]
    fn done_is_terminal() {
        assert!(ThreadState::Done.is_terminal());
        assert!(!ThreadState::Done.can_transition_to(ThreadState::Running));
    }

    #[test]
    fn failed_is_terminal() {
        assert!(ThreadState::Failed.is_terminal());
        assert!(!ThreadState::Failed.can_transition_to(ThreadState::Running));
    }

    #[test]
    fn running_is_active() {
        assert!(ThreadState::Running.is_active());
    }

    #[test]
    fn waiting_is_active() {
        assert!(ThreadState::Waiting.is_active());
    }

    #[test]
    fn created_is_not_active() {
        assert!(!ThreadState::Created.is_active());
    }

    // ── Thread lifecycle tests ──────────────────────────────

    #[test]
    fn new_thread_is_created() {
        let t = make_thread();
        assert_eq!(t.state, ThreadState::Created);
        assert!(t.events.is_empty());
        assert!(t.messages.is_empty());
    }

    #[test]
    fn valid_transition_succeeds() {
        let mut t = make_thread();
        assert!(t.transition_to(ThreadState::Running, None).is_ok());
        assert_eq!(t.state, ThreadState::Running);
        assert_eq!(t.events.len(), 1);
    }

    #[test]
    fn invalid_transition_fails() {
        let mut t = make_thread();
        let result = t.transition_to(ThreadState::Completed, None);
        assert!(result.is_err());
        assert_eq!(t.state, ThreadState::Created);
    }

    #[test]
    fn full_lifecycle_created_to_done() {
        let mut t = make_thread();
        t.transition_to(ThreadState::Running, None).unwrap();
        t.transition_to(ThreadState::Completed, Some("finished".into()))
            .unwrap();
        t.transition_to(ThreadState::Done, None).unwrap();
        assert!(t.state.is_terminal());
        assert_eq!(t.events.len(), 3);
        assert!(t.completed_at.is_some());
    }

    #[test]
    fn add_message_records_event() {
        let mut t = make_thread();
        t.add_message(ThreadMessage::user("hello"));
        assert_eq!(t.messages.len(), 1);
        assert_eq!(t.events.len(), 1);
        match &t.events[0].kind {
            EventKind::MessageAdded { role, .. } => assert_eq!(role, "User"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn child_thread_has_parent() {
        let parent = make_thread();
        let child = Thread::new(
            "child goal",
            ThreadType::Research,
            parent.project_id,
            "test-user",
            ThreadConfig::default(),
        )
        .with_parent(parent.id);
        assert_eq!(child.parent_id, Some(parent.id));
    }

    // ── Title derivation ─────────────────────────────────────

    #[test]
    fn derive_title_short_single_line() {
        assert_eq!(
            Thread::derive_title_from_message("Hello world"),
            Some("Hello world".to_string())
        );
    }

    #[test]
    fn derive_title_uses_first_nonempty_line() {
        let msg = "\n\n  \nactual first line\nsecond line\n";
        assert_eq!(
            Thread::derive_title_from_message(msg),
            Some("actual first line".to_string())
        );
    }

    #[test]
    fn derive_title_trims_whitespace() {
        assert_eq!(
            Thread::derive_title_from_message("  spaced out   "),
            Some("spaced out".to_string())
        );
    }

    #[test]
    fn derive_title_truncates_long_input_by_chars() {
        let long = "a".repeat(120);
        let title = Thread::derive_title_from_message(&long).expect("some title");
        assert_eq!(title.chars().count(), 60);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn derive_title_is_utf8_safe_on_multibyte_input() {
        // 120 multibyte chars — byte-slicing would panic; char-count works.
        let emoji = "😀".repeat(120);
        let title = Thread::derive_title_from_message(&emoji).expect("some title");
        assert_eq!(title.chars().count(), 60);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn derive_title_returns_none_for_blank_input() {
        assert_eq!(Thread::derive_title_from_message(""), None);
        assert_eq!(Thread::derive_title_from_message("   \n\t\n  "), None);
    }

    #[test]
    fn new_thread_has_no_title_by_default() {
        let t = make_thread();
        assert_eq!(t.title, None);
    }

    #[test]
    fn thread_without_title_deserializes_legacy_json() {
        // Regression: existing persisted threads lack a `title` field.
        // `#[serde(default)]` must let them load as `title: None`.
        let t = make_thread();
        let mut value: serde_json::Value = serde_json::to_value(&t).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("title")
            .expect("serialized thread should have had a title key");
        let restored: Thread = serde_json::from_value(value).unwrap();
        assert_eq!(restored.title, None);
    }

    #[test]
    fn active_skill_provenance_roundtrips_through_metadata() {
        let mut thread = make_thread();
        let skills = vec![ActiveSkillProvenance {
            doc_id: DocId::new(),
            name: "github-pr-workflow".to_string(),
            version: 3,
            snippet_names: vec!["list_prs".to_string()],
            force_activated: true,
        }];

        thread.set_active_skills(&skills).unwrap();

        assert_eq!(thread.active_skills(), skills);
    }
}

//! Job state machine.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::llm::recording::HttpInterceptor;
use crate::tools::ApprovalContext;

/// Error returned when a job exceeds its token budget.
#[derive(Debug, thiserror::Error)]
#[error("Token budget exceeded: used {used} of {limit} allowed tokens")]
pub struct TokenBudgetExceeded {
    /// Total tokens consumed (including the call that exceeded the budget).
    pub used: u64,
    /// Configured token limit for this job.
    pub limit: u64,
}

/// State of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Job is waiting to be started.
    Pending,
    /// Job is currently being worked on.
    InProgress,
    /// Job work is complete, awaiting submission.
    Completed,
    /// Job has been submitted for review.
    Submitted,
    /// Job was accepted/paid.
    Accepted,
    /// Job failed and cannot be completed.
    Failed,
    /// Job is stuck and needs repair.
    Stuck,
    /// Job was cancelled.
    Cancelled,
}

impl JobState {
    /// Check if this state allows transitioning to another state.
    pub fn can_transition_to(&self, target: JobState) -> bool {
        use JobState::*;

        // Allow idempotent Completed -> Completed transition.
        // Both the execution loop and the worker wrapper may race to mark a
        // job complete; the second call should be a harmless no-op rather
        // than an error that masks the successful completion.
        if matches!((self, target), (Completed, Completed)) {
            return true;
        }

        matches!(
            (self, target),
            // From Pending
            (Pending, InProgress) | (Pending, Cancelled) |
            // From InProgress
            (InProgress, Completed) | (InProgress, Failed) |
            (InProgress, Stuck) | (InProgress, Cancelled) |
            // From Completed
            (Completed, Submitted) | (Completed, Failed) |
            // From Submitted
            (Submitted, Accepted) | (Submitted, Failed) |
            // From Stuck (can recover or fail)
            (Stuck, InProgress) | (Stuck, Failed) | (Stuck, Cancelled)
        )
    }

    /// Check if this is a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Accepted | Self::Failed | Self::Cancelled)
    }

    /// Check if the job is active (not terminal).
    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }

    /// Check if this job consumes a parallel execution slot.
    ///
    /// Only jobs in Pending, InProgress, or Stuck states consume execution resources
    /// and should count toward the parallel job limit. Completed and Submitted jobs
    /// are in the state machine but are no longer actively executing.
    pub fn is_parallel_blocking(&self) -> bool {
        matches!(self, Self::Pending | Self::InProgress | Self::Stuck)
    }
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Submitted => "submitted",
            Self::Accepted => "accepted",
            Self::Failed => "failed",
            Self::Stuck => "stuck",
            Self::Cancelled => "cancelled",
        };
        write!(f, "{}", s)
    }
}

/// A state transition event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTransition {
    /// Previous state.
    pub from: JobState,
    /// New state.
    pub to: JobState,
    /// When the transition occurred.
    pub timestamp: DateTime<Utc>,
    /// Reason for the transition.
    pub reason: Option<String>,
}

/// Context for a running job.
#[derive(Debug, Clone, Serialize)]
pub struct JobContext {
    /// Unique job ID.
    pub job_id: Uuid,
    /// Current state.
    pub state: JobState,
    /// User ID that owns this job (for workspace scoping).
    pub user_id: String,
    /// Shared workspace scope for this job when operating in a team workspace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Channel-specific requester/actor ID, when different from the owner scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester_id: Option<String>,
    /// Conversation ID if linked to a conversation.
    pub conversation_id: Option<Uuid>,
    /// Job title.
    pub title: String,
    /// Job description.
    pub description: String,
    /// Job category.
    pub category: Option<String>,
    /// Budget amount (if from marketplace).
    pub budget: Option<Decimal>,
    /// Budget token (e.g., "NEAR", "USD").
    pub budget_token: Option<String>,
    /// Our bid amount.
    pub bid_amount: Option<Decimal>,
    /// Estimated cost to complete.
    pub estimated_cost: Option<Decimal>,
    /// Estimated time to complete.
    pub estimated_duration: Option<Duration>,
    /// Actual cost so far.
    pub actual_cost: Decimal,
    /// Total tokens consumed by LLM calls in this job.
    pub total_tokens_used: u64,
    /// Maximum tokens allowed per job (0 = unlimited).
    pub max_tokens: u64,
    /// When the job was created.
    pub created_at: DateTime<Utc>,
    /// When the job was started.
    pub started_at: Option<DateTime<Utc>>,
    /// When the job was completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Number of repair attempts.
    pub repair_attempts: u32,
    /// State transition history.
    pub transitions: Vec<StateTransition>,
    /// Metadata.
    pub metadata: serde_json::Value,
    /// Extra environment variables to inject into spawned child processes.
    ///
    /// Used by the worker runtime to pass fetched credentials to tools
    /// (e.g., shell commands) without mutating the global process environment
    /// via `std::env::set_var`, which is unsafe in multi-threaded programs.
    ///
    /// Wrapped in `Arc` for cheap cloning on every tool invocation.
    #[serde(skip)]
    pub extra_env: Arc<HashMap<String, String>>,
    /// Optional HTTP interceptor for trace recording/replay.
    ///
    /// When set, tools that make outgoing HTTP requests should check this
    /// interceptor before sending real requests. During recording, the
    /// interceptor captures request/response pairs. During replay, it
    /// returns pre-recorded responses.
    #[serde(skip)]
    pub http_interceptor: Option<Arc<dyn HttpInterceptor>>,
    /// Stash of full tool outputs keyed by tool_call_id.
    ///
    /// Tool outputs may be truncated before reaching the LLM context window,
    /// but subsequent tools (e.g., `json`) may need the full output. This
    /// stash stores the complete, unsanitized output so tools can reference
    /// previous results by ID via `$tool_call_id` parameter syntax.
    ///
    /// Also used for cross-tool implicit state (keys prefixed with `__`) such
    /// as `__routine_last_name` for fallback recovery in routine tool chains.
    #[serde(skip)]
    pub tool_output_stash: Arc<tokio::sync::RwLock<HashMap<String, String>>>,
    /// User's preferred timezone (IANA name, e.g. "America/New_York"). Defaults to "UTC".
    pub user_timezone: String,
    /// Approval context for tool execution in this job.
    ///
    /// When set, tools check this context before executing to determine
    /// if they're allowed to run in autonomous/non-interactive contexts.
    #[serde(skip)]
    pub approval_context: Option<ApprovalContext>,
}

impl JobContext {
    /// Create a new job context.
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self::with_user("default", title, description)
    }

    /// Create a new job context with a specific user ID.
    pub fn with_user(
        user_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            job_id: Uuid::new_v4(),
            state: JobState::Pending,
            user_id: user_id.into(),
            workspace_id: None,
            requester_id: None,
            conversation_id: None,
            title: title.into(),
            description: description.into(),
            category: None,
            budget: None,
            budget_token: None,
            bid_amount: None,
            estimated_cost: None,
            estimated_duration: None,
            actual_cost: Decimal::ZERO,
            total_tokens_used: 0,
            max_tokens: 0,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            repair_attempts: 0,
            transitions: Vec::new(),
            extra_env: Arc::new(HashMap::new()),
            http_interceptor: None,
            metadata: serde_json::Value::Null,
            tool_output_stash: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            user_timezone: "UTC".to_string(),
            approval_context: None,
        }
    }

    /// Set the user timezone on this context.
    pub fn with_timezone(mut self, tz: impl Into<String>) -> Self {
        self.user_timezone = tz.into();
        self
    }

    /// Set the channel-specific requester/actor ID.
    pub fn with_requester_id(mut self, requester_id: impl Into<String>) -> Self {
        self.requester_id = Some(requester_id.into());
        self
    }

    /// Set the approval context on this context.
    pub fn with_approval_context(mut self, ctx: ApprovalContext) -> Self {
        self.approval_context = Some(ctx);
        self
    }

    /// Transition to a new state.
    pub fn transition_to(
        &mut self,
        new_state: JobState,
        reason: Option<String>,
    ) -> Result<(), String> {
        if !self.state.can_transition_to(new_state) {
            return Err(format!(
                "Cannot transition from {} to {}",
                self.state, new_state
            ));
        }

        // Idempotent: already in the target state, skip recording a duplicate
        // transition. This handles the Completed -> Completed race between
        // execution_loop and the worker wrapper.
        if self.state == new_state {
            tracing::debug!(
                job_id = %self.job_id,
                state = %self.state,
                "idempotent state transition (already in target state), skipping"
            );
            return Ok(());
        }

        let transition = StateTransition {
            from: self.state,
            to: new_state,
            timestamp: Utc::now(),
            reason,
        };

        self.transitions.push(transition);

        // Cap transition history to prevent unbounded memory growth
        const MAX_TRANSITIONS: usize = 200;
        if self.transitions.len() > MAX_TRANSITIONS {
            let drain_count = self.transitions.len() - MAX_TRANSITIONS;
            self.transitions.drain(..drain_count);
        }

        self.state = new_state;

        // Update timestamps
        match new_state {
            JobState::InProgress if self.started_at.is_none() => {
                self.started_at = Some(Utc::now());
            }
            JobState::Completed | JobState::Accepted | JobState::Failed | JobState::Cancelled => {
                self.completed_at = Some(Utc::now());
            }
            _ => {}
        }

        Ok(())
    }

    /// Add to the actual cost.
    pub fn add_cost(&mut self, cost: Decimal) {
        self.actual_cost += cost;
    }

    /// Record token usage from an LLM call. Returns an error if the token
    /// budget has been exceeded after this addition.
    pub fn add_tokens(&mut self, tokens: u64) -> Result<(), TokenBudgetExceeded> {
        self.total_tokens_used += tokens;
        if self.max_tokens > 0 && self.total_tokens_used > self.max_tokens {
            Err(TokenBudgetExceeded {
                used: self.total_tokens_used,
                limit: self.max_tokens,
            })
        } else {
            Ok(())
        }
    }

    /// Check whether the monetary budget has been exceeded.
    pub fn budget_exceeded(&self) -> bool {
        if let Some(ref budget) = self.budget {
            self.actual_cost > *budget
        } else {
            false
        }
    }

    /// Get the duration since the job started.
    pub fn elapsed(&self) -> Option<Duration> {
        self.started_at.map(|start| {
            let end = self.completed_at.unwrap_or_else(Utc::now);
            let duration = end.signed_duration_since(start);
            Duration::from_secs(duration.num_seconds().max(0) as u64)
        })
    }

    /// Mark the job as stuck.
    pub fn mark_stuck(&mut self, reason: impl Into<String>) -> Result<(), String> {
        self.transition_to(JobState::Stuck, Some(reason.into()))
    }

    /// Attempt to recover from stuck state.
    pub fn attempt_recovery(&mut self) -> Result<(), String> {
        if self.state != JobState::Stuck {
            return Err("Job is not stuck".to_string());
        }
        self.repair_attempts += 1;
        self.transition_to(JobState::InProgress, Some("Recovery attempt".to_string()))
    }
}

impl Default for JobContext {
    fn default() -> Self {
        // Default has no approval_context - safer default that requires explicit
        // opt-in for autonomous execution. Code that creates JobContext directly
        // must use with_approval_context() to enable autonomous tool use.
        Self::with_user("default", "Untitled", "No description")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_transitions() {
        assert!(JobState::Pending.can_transition_to(JobState::InProgress));
        assert!(JobState::InProgress.can_transition_to(JobState::Completed));
        assert!(!JobState::Completed.can_transition_to(JobState::Pending));
        assert!(!JobState::Accepted.can_transition_to(JobState::InProgress));
    }

    #[test]
    fn test_completed_to_completed_is_idempotent() {
        // Regression test for the race condition where both execution_loop
        // and the worker wrapper call mark_completed(). The second call
        // must succeed without error and must not record a duplicate
        // transition.
        let mut ctx = JobContext::new("Test", "Idempotent completion test");
        ctx.transition_to(JobState::InProgress, None).unwrap();
        ctx.transition_to(JobState::Completed, Some("first".into()))
            .unwrap();
        assert_eq!(ctx.state, JobState::Completed);
        let transitions_before = ctx.transitions.len();

        // Second Completed -> Completed must be a no-op
        let result = ctx.transition_to(JobState::Completed, Some("duplicate".into()));
        assert!(
            result.is_ok(),
            "Completed -> Completed should be idempotent"
        );
        assert_eq!(ctx.state, JobState::Completed);
        assert_eq!(
            ctx.transitions.len(),
            transitions_before,
            "idempotent transition should not record a new history entry"
        );
    }

    #[test]
    fn test_other_self_transitions_still_rejected() {
        // Ensure we only allow Completed -> Completed, not arbitrary X -> X.
        assert!(!JobState::Pending.can_transition_to(JobState::Pending));
        assert!(!JobState::InProgress.can_transition_to(JobState::InProgress));
        assert!(!JobState::Failed.can_transition_to(JobState::Failed));
        assert!(!JobState::Stuck.can_transition_to(JobState::Stuck));
        assert!(!JobState::Submitted.can_transition_to(JobState::Submitted));
        assert!(!JobState::Accepted.can_transition_to(JobState::Accepted));
        assert!(!JobState::Cancelled.can_transition_to(JobState::Cancelled));
    }

    #[test]
    fn test_terminal_states() {
        assert!(JobState::Accepted.is_terminal());
        assert!(JobState::Failed.is_terminal());
        assert!(JobState::Cancelled.is_terminal());
        assert!(!JobState::InProgress.is_terminal());
    }

    #[test]
    fn test_job_context_transitions() {
        let mut ctx = JobContext::new("Test", "Test job");
        assert_eq!(ctx.state, JobState::Pending);

        ctx.transition_to(JobState::InProgress, None).unwrap();
        assert_eq!(ctx.state, JobState::InProgress);
        assert!(ctx.started_at.is_some());

        ctx.transition_to(JobState::Completed, Some("Done".to_string()))
            .unwrap();
        assert_eq!(ctx.state, JobState::Completed);
    }

    #[test]
    fn test_transition_history_capped() {
        let mut ctx = JobContext::new("Test", "Transition cap test");
        // Cycle through Pending -> InProgress -> Stuck -> InProgress -> Stuck ...
        ctx.transition_to(JobState::InProgress, None).unwrap();
        for i in 0..250 {
            ctx.mark_stuck(format!("stuck {}", i)).unwrap();
            ctx.attempt_recovery().unwrap();
        }
        // 1 initial + 250*2 = 501 transitions, should be capped at 200
        assert!(
            ctx.transitions.len() <= 200,
            "transitions should be capped at 200, got {}",
            ctx.transitions.len()
        );
    }

    #[test]
    fn test_add_tokens_enforces_budget() {
        let mut ctx = JobContext::new("Test", "Budget test");
        ctx.max_tokens = 1000;
        assert!(ctx.add_tokens(500).is_ok());
        assert_eq!(ctx.total_tokens_used, 500);
        assert!(ctx.add_tokens(600).is_err());
        assert_eq!(ctx.total_tokens_used, 1100); // tokens still recorded
    }

    #[test]
    fn test_add_tokens_unlimited() {
        let mut ctx = JobContext::new("Test", "No budget");
        // max_tokens = 0 means unlimited
        assert!(ctx.add_tokens(1_000_000).is_ok());
    }

    #[test]
    fn test_budget_exceeded() {
        let mut ctx = JobContext::new("Test", "Money test");
        ctx.budget = Some(Decimal::new(100, 0)); // $100
        assert!(!ctx.budget_exceeded());
        ctx.add_cost(Decimal::new(50, 0));
        assert!(!ctx.budget_exceeded());
        ctx.add_cost(Decimal::new(60, 0));
        assert!(ctx.budget_exceeded());
    }

    #[test]
    fn test_budget_exceeded_none() {
        let ctx = JobContext::new("Test", "No budget");
        assert!(!ctx.budget_exceeded()); // No budget = never exceeded
    }

    #[test]
    fn test_stuck_recovery() {
        let mut ctx = JobContext::new("Test", "Test job");
        ctx.transition_to(JobState::InProgress, None).unwrap();
        ctx.mark_stuck("Timed out").unwrap();
        assert_eq!(ctx.state, JobState::Stuck);

        ctx.attempt_recovery().unwrap();
        assert_eq!(ctx.state, JobState::InProgress);
        assert_eq!(ctx.repair_attempts, 1);
    }
}

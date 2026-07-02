use std::collections::HashSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub parent_id: Option<TaskId>,
    pub description: String,
    pub depth: u32,
}

impl Task {
    pub fn root(description: impl Into<String>, id: TaskId) -> Self {
        Self {
            id,
            parent_id: None,
            description: description.into(),
            depth: 0,
        }
    }

    pub fn child(
        parent: &Task,
        description: impl Into<String>,
        id: TaskId,
    ) -> Self {
        Self {
            id,
            parent_id: Some(parent.id.clone()),
            description: description.into(),
            depth: parent.depth.saturating_add(1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Completed,
    Failed,
    Delegated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: TaskId,
    pub agent_id: String,
    pub status: TaskStatus,
    pub summary: String,
    pub child_results: Vec<TaskResult>,
    pub error: Option<String>,
}

impl TaskResult {
    pub fn completed(
        task_id: TaskId,
        agent_id: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            task_id,
            agent_id: agent_id.into(),
            status: TaskStatus::Completed,
            summary: summary.into(),
            child_results: Vec::new(),
            error: None,
        }
    }

    pub fn failed(
        task_id: TaskId,
        agent_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        let reason = reason.into();
        Self {
            task_id: task_id.clone(),
            agent_id: agent_id.into(),
            status: TaskStatus::Failed,
            summary: reason.clone(),
            child_results: Vec::new(),
            error: Some(reason),
        }
    }

    pub fn delegated(
        task_id: TaskId,
        agent_id: impl Into<String>,
        summary: impl Into<String>,
        child_results: Vec<TaskResult>,
    ) -> Self {
        Self {
            task_id,
            agent_id: agent_id.into(),
            status: TaskStatus::Delegated,
            summary: summary.into(),
            child_results,
            error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegationPlan {
    pub execute_directly: bool,
    pub subtasks: Vec<String>,
    /// Human-readable explanation of why this decision was made.
    #[serde(default)]
    pub reason: String,
}

impl DelegationPlan {
    /// Execute the task locally — no delegation.
    pub fn local(reason: impl Into<String>) -> Self {
        Self {
            execute_directly: true,
            subtasks: Vec::new(),
            reason: reason.into(),
        }
    }

    /// Backward-compatible alias for `local`.
    pub fn execute_directly() -> Self {
        Self::local("direct execution")
    }

    /// Split into independent subtasks that run as delegated AgentRuns.
    pub fn split(subtasks: Vec<String>, reason: impl Into<String>) -> Self {
        Self {
            execute_directly: false,
            subtasks,
            reason: reason.into(),
        }
    }

    /// Backward-compatible alias for `split` (no reason string).
    pub fn delegate(subtasks: Vec<String>) -> Self {
        let n = subtasks.len();
        Self::split(subtasks, format!("split into {n} independent tasks"))
    }
}

#[derive(Debug, Clone)]
pub struct AgentContext {
    pub max_depth: u32,
    pub max_iterations: u32,
    pub iterations_used: u32,
    pub task_timeout: Duration,
    pub active_chain: HashSet<TaskId>,
}

impl AgentContext {
    pub fn new(max_depth: u32, max_iterations: u32, task_timeout: Duration) -> Self {
        Self {
            max_depth,
            max_iterations,
            iterations_used: 0,
            task_timeout,
            active_chain: HashSet::new(),
        }
    }

    pub fn register_task(&mut self, task_id: &TaskId) -> Result<(), super::MultiAgentError> {
        if self.active_chain.contains(task_id) {
            return Err(super::MultiAgentError::CycleDetected {
                task_id: task_id.0.clone(),
            });
        }
        self.active_chain.insert(task_id.clone());
        Ok(())
    }

    pub fn unregister_task(&mut self, task_id: &TaskId) {
        self.active_chain.remove(task_id);
    }

    pub fn consume_iteration(&mut self) -> Result<(), super::MultiAgentError> {
        self.iterations_used = self.iterations_used.saturating_add(1);
        if self.iterations_used > self.max_iterations {
            return Err(super::MultiAgentError::MaxIterationsExceeded {
                max_iterations: self.max_iterations,
            });
        }
        Ok(())
    }

    pub fn can_delegate(&self, task: &Task) -> bool {
        task.depth < self.max_depth
    }
}

/// The structured local-vs-delegated split every AgentRun (master or
/// delegated) produces before doing any work. Derived from a
/// [`DelegationPlan`]'s subtasks so the same agent that would otherwise
/// delegate everything always keeps at least one unit of work for itself,
/// and only hands the rest off as new AgentRuns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub local_tasks: Vec<String>,
    pub delegated_tasks: Vec<String>,
}

impl ExecutionPlan {
    /// Build a plan from a delegation planner's subtasks: the first subtask
    /// stays with the current agent, the rest become delegated AgentRuns.
    /// Applying this uniformly at every recursion level is what lets any
    /// delegated AgentRun make the same "do some, delegate some" decision.
    pub fn from_subtasks(mut subtasks: Vec<String>) -> Self {
        if subtasks.is_empty() {
            return Self {
                local_tasks: Vec::new(),
                delegated_tasks: Vec::new(),
            };
        }
        let local = subtasks.remove(0);
        Self {
            local_tasks: vec![local],
            delegated_tasks: subtasks,
        }
    }

    /// True when there was only one unit of work, so nothing is actually
    /// worth delegating (the caller should just execute it directly).
    pub fn is_pure_local(&self) -> bool {
        self.delegated_tasks.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiAgentRunReport {
    pub master_task: String,
    pub root_id: String,
    pub root_result: TaskResult,
    pub final_summary: String,
    #[serde(default)]
    pub jobs: Vec<super::job_model::AgentJob>,
    #[serde(default)]
    pub events: Vec<super::job_model::AgentEvent>,
}

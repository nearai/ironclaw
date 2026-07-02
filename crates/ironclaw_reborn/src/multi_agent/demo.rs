use std::collections::HashMap;
use std::io::{self, Write};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::error::MultiAgentError;
use super::executor::{PlaceholderTaskExecutor, TaskExecutor};
use super::planner::{DelegationPlanner, HeuristicDelegationPlanner};
use super::types::{AgentContext, Task, TaskId};

/// What a run returns to the harness after planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAction {
    Execute,
    Delegate { tasks: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    Pending,
    Running,
    WaitingForDelegated,
    Complete,
    Failed,
}

/// One persisted run record in the harness store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub parent_id: Option<String>,
    pub root_id: String,
    pub task: String,
    pub depth: u32,
    pub status: RunStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub delegated_run_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessEvent {
    pub run_id: String,
    pub message: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AgentDemoConfig {
    pub max_depth: u32,
    pub max_iterations: u32,
    pub live_events: bool,
}

impl AgentDemoConfig {
    pub fn new(max_depth: u32, max_iterations: u32) -> Self {
        Self {
            max_depth,
            max_iterations,
            live_events: false,
        }
    }

    pub fn with_live_events(mut self, live_events: bool) -> Self {
        self.live_events = live_events;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDemoResult {
    pub root_run_id: String,
    pub root_task: String,
    pub runs: Vec<RunRecord>,
    pub events: Vec<HarnessEvent>,
    pub final_result: String,
}

struct RunHarness {
    runs: HashMap<String, RunRecord>,
    run_order: Vec<String>,
    events: Vec<HarnessEvent>,
    started_at: Instant,
    next_run_id: u64,
    iterations: u32,
    planner: HeuristicDelegationPlanner,
    executor: PlaceholderTaskExecutor,
    live_events: bool,
}

impl RunHarness {
    fn new(live_events: bool) -> Self {
        Self {
            runs: HashMap::new(),
            run_order: Vec::new(),
            events: Vec::new(),
            started_at: Instant::now(),
            next_run_id: 1,
            iterations: 0,
            planner: HeuristicDelegationPlanner,
            executor: PlaceholderTaskExecutor,
            live_events,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    fn format_timestamp(elapsed_ms: u64) -> String {
        let total_secs = elapsed_ms / 1000;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{mins:02}:{secs:02}")
    }

    fn create_run(
        &mut self,
        parent_id: Option<&str>,
        task: impl Into<String>,
        depth: u32,
        root_id: Option<&str>,
    ) -> String {
        let id = format!("run-{}", self.next_run_id);
        self.next_run_id = self.next_run_id.saturating_add(1);
        let task = task.into();
        let root_id = root_id
            .map(str::to_string)
            .unwrap_or_else(|| id.clone());
        let record = RunRecord {
            id: id.clone(),
            parent_id: parent_id.map(str::to_string),
            root_id,
            task,
            depth,
            status: RunStatus::Pending,
            result: None,
            error: None,
            delegated_run_ids: Vec::new(),
        };
        self.runs.insert(id.clone(), record);
        self.run_order.push(id.clone());
        id
    }

    fn emit(&mut self, run_id: &str, message: impl Into<String>) {
        let message = message.into();
        let event = HarnessEvent {
            run_id: run_id.to_string(),
            message: message.clone(),
            timestamp_ms: self.elapsed_ms(),
        };
        if self.live_events {
            let stamp = Self::format_timestamp(event.timestamp_ms);
            println!("[{stamp}] {run_id} {message}");
            let _ = io::stdout().flush();
        }
        self.events.push(event);
    }

    fn set_status(&mut self, run_id: &str, status: RunStatus) {
        if let Some(run) = self.runs.get_mut(run_id) {
            run.status = status;
        }
    }

    fn complete_run(&mut self, run_id: &str, result: impl Into<String>) {
        if let Some(run) = self.runs.get_mut(run_id) {
            run.status = RunStatus::Complete;
            run.result = Some(result.into());
        }
    }

    fn fail_run(&mut self, run_id: &str, error: impl Into<String>) {
        if let Some(run) = self.runs.get_mut(run_id) {
            run.status = RunStatus::Failed;
            run.error = Some(error.into());
        }
    }

    fn decide_action(&self, run: &RunRecord, max_depth: u32, max_iterations: u32) -> RunAction {
        let task = Task {
            id: TaskId::new(run.id.clone()),
            parent_id: run.parent_id.clone().map(TaskId::new),
            description: run.task.clone(),
            depth: run.depth,
        };
        let ctx = AgentContext::new(max_depth, max_iterations, std::time::Duration::from_secs(3600));
        let plan = self.planner.plan(&task, &ctx);
        if plan.execute_directly || run.depth >= max_depth || plan.subtasks.is_empty() {
            RunAction::Execute
        } else {
            RunAction::Delegate {
                tasks: plan.subtasks,
            }
        }
    }

    async fn process_run(
        &mut self,
        run_id: &str,
        config: &AgentDemoConfig,
    ) -> Result<(), MultiAgentError> {
        self.iterations = self.iterations.saturating_add(1);
        if self.iterations > config.max_iterations {
            return Err(MultiAgentError::MaxIterationsExceeded {
                max_iterations: config.max_iterations,
            });
        }

        self.set_status(run_id, RunStatus::Running);
        self.emit(run_id, "received task");

        let run = self
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| MultiAgentError::OrchestrationFailed {
                reason: format!("run not found: {run_id}"),
            })?;
        let action = self.decide_action(&run, config.max_depth, config.max_iterations);

        match action {
            RunAction::Execute => {
                self.emit(run_id, "Execute");
                let task = Task {
                    id: TaskId::new(run.id.clone()),
                    parent_id: run.parent_id.clone().map(TaskId::new),
                    description: run.task.clone(),
                    depth: run.depth,
                };
                match self.executor.execute(&task).await {
                    Ok(result) => {
                        self.complete_run(run_id, result);
                        self.emit(run_id, "completed");
                    }
                    Err(error) => {
                        self.fail_run(run_id, error.to_string());
                        self.emit(run_id, format!("failed: {error}"));
                    }
                }
            }
            RunAction::Delegate { tasks } => {
                self.emit(run_id, format!("Delegate {{ tasks: {tasks:?} }}"));
                let root_id = run.root_id.clone();
                let depth = run.depth;
                let mut delegated_ids = Vec::with_capacity(tasks.len());
                for delegated_task in tasks {
                    let child_id =
                        self.create_run(Some(run_id), delegated_task, depth + 1, Some(&root_id));
                    delegated_ids.push(child_id);
                }

                if let Some(parent) = self.runs.get_mut(run_id) {
                    parent.delegated_run_ids = delegated_ids.clone();
                    parent.status = RunStatus::WaitingForDelegated;
                }
                self.emit(
                    run_id,
                    format!(
                        "created {} delegated run(s): {}",
                        delegated_ids.len(),
                        delegated_ids.join(", ")
                    ),
                );

                // Same execution path for every delegated run. Sequential for MVP.
                for delegated_id in &delegated_ids {
                    Box::pin(self.process_run(delegated_id, config)).await?;
                }

                let aggregated = self.aggregate_delegated(&delegated_ids);
                self.complete_run(run_id, aggregated);
                self.emit(run_id, "aggregated delegated results");
            }
        }

        Ok(())
    }

    fn aggregate_delegated(&self, delegated_ids: &[String]) -> String {
        delegated_ids
            .iter()
            .map(|run_id| {
                let run = &self.runs[run_id];
                format!(
                    "{run_id}: {}",
                    run.result
                        .as_deref()
                        .or(run.error.as_deref())
                        .unwrap_or("(no output)")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn into_result(self, root_run_id: String, root_task: String) -> AgentDemoResult {
        let final_result = self
            .runs
            .get(&root_run_id)
            .and_then(|run| run.result.clone())
            .unwrap_or_else(|| "harness finished without a result".to_string());
        let runs = self
            .run_order
            .iter()
            .filter_map(|id| self.runs.get(id).cloned())
            .collect();
        AgentDemoResult {
            root_run_id,
            root_task,
            runs,
            events: self.events,
            final_result,
        }
    }
}

/// Run the minimal in-memory delegation harness.
pub async fn run_multi_agent_demo(
    task: impl Into<String>,
    config: AgentDemoConfig,
) -> Result<AgentDemoResult, MultiAgentError> {
    let task = task.into();
    let mut harness = RunHarness::new(config.live_events);
    let root_run_id = harness.create_run(None, task.clone(), 0, None);

    if config.live_events {
        println!("Delegation Harness Demo");
        println!("Root task: \"{task}\"");
        println!();
        let _ = io::stdout().flush();
    }

    harness.process_run(&root_run_id, &config).await?;
    Ok(harness.into_result(root_run_id, task))
}

pub fn format_demo_progress(result: &AgentDemoResult) -> String {
    let mut output = String::new();
    output.push_str("Delegation Harness Demo\n");
    output.push_str(&format!("Root task: \"{}\"\n", result.root_task));
    output.push('\n');
    output.push_str("Runs:\n");
    for run in &result.runs {
        output.push_str(&format!(
            "- {} [{:?}] {}",
            run.id, run.status, run.task
        ));
        if !run.delegated_run_ids.is_empty() {
            output.push_str(&format!(
                " (delegated: {})",
                run.delegated_run_ids.join(", ")
            ));
        }
        output.push('\n');
    }
    output.push('\n');
    output.push_str("Event log:\n");
    for event in &result.events {
        let stamp = RunHarness::format_timestamp(event.timestamp_ms);
        output.push_str(&format!(
            "[{stamp}] {} {}\n",
            event.run_id, event.message
        ));
    }
    output.push('\n');
    output.push_str("Final result:\n");
    output.push_str(&result.final_result);
    output.push('\n');
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn narrow_task_executes_directly_without_delegation() {
        let result = run_multi_agent_demo("Say hello", AgentDemoConfig::new(2, 16))
            .await
            .expect("demo run");
        assert_eq!(result.runs.len(), 1);
        assert!(result.runs[0].delegated_run_ids.is_empty());
        assert_eq!(result.runs[0].status, RunStatus::Complete);
        assert!(
            result
                .events
                .iter()
                .any(|event| event.message == "Execute")
        );
    }

    #[tokio::test]
    async fn broad_task_emits_delegate_and_creates_real_runs() {
        let result = run_multi_agent_demo(
            "Search the news about NEAR AI and create a presentation summary",
            AgentDemoConfig::new(2, 16),
        )
        .await
        .expect("demo run");

        assert_eq!(result.runs.len(), 3, "root plus two delegated runs");
        assert!(
            result
                .events
                .iter()
                .any(|event| event.run_id == "run-1" && event.message.starts_with("Delegate {"))
        );
        assert_eq!(result.runs[0].delegated_run_ids, vec!["run-2", "run-3"]);
        assert_eq!(result.runs[1].status, RunStatus::Complete);
        assert_eq!(result.runs[2].status, RunStatus::Complete);
        assert!(result.final_result.contains("run-2:"));
        assert!(result.final_result.contains("run-3:"));
    }

    #[tokio::test]
    async fn delegated_runs_use_same_execution_path() {
        let result = run_multi_agent_demo(
            "Research X; Plan implementation",
            AgentDemoConfig::new(2, 16),
        )
        .await
        .expect("demo run");

        for run in &result.runs[1..] {
            assert!(
                result
                    .events
                    .iter()
                    .any(|event| event.run_id == run.id && event.message == "received task")
            );
            assert!(
                result
                    .events
                    .iter()
                    .any(|event| event.run_id == run.id && event.message == "Execute")
            );
            assert!(
                result
                    .events
                    .iter()
                    .any(|event| event.run_id == run.id && event.message == "completed")
            );
        }
    }

    #[tokio::test]
    async fn recursion_stops_at_max_depth() {
        let result = run_multi_agent_demo("one; two; three; four", AgentDemoConfig::new(1, 32))
            .await
            .expect("depth capped demo");
        assert!(
            result
                .runs
                .iter()
                .filter(|run| run.depth == 1)
                .all(|run| run.delegated_run_ids.is_empty())
        );
    }

    #[tokio::test]
    async fn demo_output_shows_runs_events_and_final_result() {
        let result = run_multi_agent_demo(
            "Research X; Plan implementation",
            AgentDemoConfig::new(2, 16),
        )
        .await
        .expect("demo run");
        let rendered = format_demo_progress(&result);
        assert!(rendered.contains("Delegation Harness Demo"));
        assert!(rendered.contains("Runs:"));
        assert!(rendered.contains("Event log:"));
        assert!(rendered.contains("Final result:"));
        assert!(rendered.contains("Delegate {"));
    }
}

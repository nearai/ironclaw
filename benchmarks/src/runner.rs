use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use tokio::sync::Mutex;
use uuid::Uuid;

use ironclaw::agent::{Agent, AgentDeps};
use ironclaw::channels::{ChannelManager, IncomingMessage};
use ironclaw::config::AgentConfig;
use ironclaw::llm::LlmProvider;
use ironclaw::safety::SafetyLayer;
use ironclaw::tools::ToolRegistry;

use crate::channel::BenchChannel;
use crate::config::{BenchConfig, MatrixEntry};
use crate::error::BenchError;
use crate::instrumented_llm::InstrumentedLlm;
use crate::results::{
    RunResult, TaskResult, Trace, append_task_result, completed_task_ids, run_dir, run_json_path,
    tasks_jsonl_path, write_run_result, write_task_results,
};
use crate::suite::{BenchSuite, BenchTask, ConversationTurn, TaskSubmission, TurnRole};

/// Parameters for running a single task in isolation.
struct TaskRunParams<'a> {
    task: &'a BenchTask,
    suite_id: &'a str,
    config_label: &'a str,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    timeout: std::time::Duration,
    additional_tools: &'a [Arc<dyn ironclaw::tools::Tool>],
}

/// Orchestrates benchmark execution: loads tasks, runs agent per task,
/// scores results, writes JSONL output.
pub struct BenchRunner {
    suite: Arc<dyn BenchSuite>,
    config: BenchConfig,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
}

impl BenchRunner {
    pub fn new(
        suite: Box<dyn BenchSuite>,
        config: BenchConfig,
        llm: Arc<dyn LlmProvider>,
        safety: Arc<SafetyLayer>,
    ) -> Self {
        Self {
            suite: Arc::from(suite),
            config,
            llm,
            safety,
        }
    }

    /// Run the benchmark for one matrix entry.
    ///
    /// Returns the run_id for result retrieval.
    pub async fn run(
        &self,
        matrix: &MatrixEntry,
        sample: Option<usize>,
        task_filter: Option<&[String]>,
        tag_filter: Option<&[String]>,
        resume_run_id: Option<Uuid>,
    ) -> Result<Uuid, BenchError> {
        let run_id = resume_run_id.unwrap_or_else(Uuid::new_v4);
        let results_base = &self.config.results_dir;
        let dir = run_dir(results_base, run_id);
        std::fs::create_dir_all(&dir)?;

        let jsonl_path = tasks_jsonl_path(results_base, run_id);
        let json_path = run_json_path(results_base, run_id);

        // Load completed task IDs for resume support
        let completed: HashSet<String> = if resume_run_id.is_some() {
            completed_task_ids(&jsonl_path)?
        } else {
            HashSet::new()
        };

        if !completed.is_empty() {
            tracing::info!(
                "Resuming run {}: {} tasks already completed",
                run_id,
                completed.len()
            );
        }

        // Load and filter tasks
        let mut tasks = self.suite.load_tasks().await?;

        if let Some(ids) = task_filter {
            let id_set: HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
            tasks.retain(|t| id_set.contains(t.id.as_str()));
        }

        if let Some(tags) = tag_filter {
            let tag_set: HashSet<&str> = tags.iter().map(|s| s.as_str()).collect();
            tasks.retain(|t| t.tags.iter().any(|tag| tag_set.contains(tag.as_str())));
        }

        // Filter out already-completed tasks
        tasks.retain(|t| !completed.contains(&t.id));

        // Sample if requested
        if let Some(n) = sample {
            tasks.truncate(n);
        }

        let total_tasks = tasks.len() + completed.len();
        tracing::info!(
            "Running {} tasks for suite '{}' (config: {}, run: {})",
            tasks.len(),
            self.suite.id(),
            matrix.label,
            run_id
        );

        let started_at = Utc::now();
        let all_results: Arc<Mutex<Vec<TaskResult>>> =
            Arc::new(Mutex::new(Vec::with_capacity(tasks.len())));

        if self.config.parallelism <= 1 {
            // Sequential execution
            let additional_tools = self.suite.additional_tools();
            for (i, task) in tasks.iter().enumerate() {
                tracing::info!(
                    "[{}/{}] Running task: {}",
                    i + 1 + completed.len(),
                    total_tasks,
                    task.id
                );
                if let Err(e) = self.suite.setup_task(task).await {
                    tracing::warn!("setup_task failed for {}: {}", task.id, e);
                }
                let params = TaskRunParams {
                    task,
                    suite_id: self.suite.id(),
                    config_label: &matrix.label,
                    llm: Arc::clone(&self.llm),
                    safety: Arc::clone(&self.safety),
                    timeout: task.timeout.unwrap_or(self.config.task_timeout),
                    additional_tools: &additional_tools,
                };
                let result = run_task_isolated(params).await;
                if let Err(e) = self.suite.teardown_task(task).await {
                    tracing::warn!("teardown_task failed for {}: {}", task.id, e);
                }
                append_task_result(&jsonl_path, &result)?;
                all_results.lock().await.push(result);
            }
        } else {
            // Parallel execution with bounded concurrency
            let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.parallelism));

            let mut handles = Vec::new();
            for (i, task) in tasks.into_iter().enumerate() {
                let sem = Arc::clone(&semaphore);
                let suite = Arc::clone(&self.suite);
                let config_label = matrix.label.clone();
                let llm = Arc::clone(&self.llm);
                let safety = Arc::clone(&self.safety);
                let timeout = task.timeout.unwrap_or(self.config.task_timeout);
                let results_ref = Arc::clone(&all_results);
                let jsonl = jsonl_path.clone();
                let completed_count = completed.len();
                let total = total_tasks;
                let additional_tools = self.suite.additional_tools();

                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.expect("semaphore closed");
                    tracing::info!(
                        "[{}/{}] Running task: {}",
                        i + 1 + completed_count,
                        total,
                        task.id
                    );
                    if let Err(e) = suite.setup_task(&task).await {
                        tracing::warn!("setup_task failed for {}: {}", task.id, e);
                    }
                    let suite_id = suite.id().to_string();
                    let params = TaskRunParams {
                        task: &task,
                        suite_id: &suite_id,
                        config_label: &config_label,
                        llm,
                        safety,
                        timeout,
                        additional_tools: &additional_tools,
                    };
                    let result = run_task_isolated(params).await;
                    if let Err(e) = suite.teardown_task(&task).await {
                        tracing::warn!("teardown_task failed for {}: {}", task.id, e);
                    }
                    if let Err(e) = append_task_result(&jsonl, &result) {
                        tracing::error!("Failed to write result for {}: {}", task.id, e);
                    }
                    results_ref.lock().await.push(result);
                }));
            }

            for handle in handles {
                if let Err(e) = handle.await {
                    tracing::error!("Task panicked: {}", e);
                }
            }
        }

        // Score all results (load tasks once, index by ID)
        let results = all_results.lock().await;
        let task_index: HashMap<String, BenchTask> = self
            .suite
            .load_tasks()
            .await?
            .into_iter()
            .map(|t| (t.id.clone(), t))
            .collect();

        let mut scored: Vec<TaskResult> = Vec::with_capacity(results.len());
        for result in results.iter() {
            if let Some(task) = task_index.get(&result.task_id) {
                let submission = TaskSubmission {
                    response: result.response.clone(),
                    conversation: vec![],
                    tool_calls: result
                        .trace
                        .tool_calls
                        .iter()
                        .map(|tc| tc.name.clone())
                        .collect(),
                    error: result.error.clone(),
                };
                match self.suite.score(task, &submission).await {
                    Ok(score) => {
                        let mut scored_result = result.clone();
                        scored_result.score = score;
                        scored.push(scored_result);
                    }
                    Err(e) => {
                        tracing::warn!("Scoring failed for {}: {}", result.task_id, e);
                        scored.push(result.clone());
                    }
                }
            } else {
                scored.push(result.clone());
            }
        }

        // Combine with any previously completed results for the aggregate
        let mut all_for_aggregate = crate::results::read_task_results(&jsonl_path)?;
        // De-duplicate (prefer the newer scored versions)
        let scored_ids: HashSet<String> = scored.iter().map(|r| r.task_id.clone()).collect();
        all_for_aggregate.retain(|r| !scored_ids.contains(&r.task_id));
        all_for_aggregate.extend(scored);

        // Rewrite JSONL with scored results so `results` command shows final scores
        write_task_results(&jsonl_path, &all_for_aggregate)?;

        let model_name = matrix.model.as_deref().unwrap_or(self.llm.model_name());

        let run_result = RunResult::from_tasks(
            run_id,
            self.suite.id(),
            &matrix.label,
            model_name,
            total_tasks,
            &all_for_aggregate,
            started_at,
        );

        write_run_result(&json_path, &run_result)?;

        tracing::info!(
            "Run {} complete: {:.1}% pass rate, {:.3} avg score, ${:.4} cost",
            run_id,
            run_result.pass_rate * 100.0,
            run_result.avg_score,
            run_result.total_cost_usd,
        );

        Ok(run_id)
    }
}

/// Run a single benchmark task in complete isolation.
///
/// Creates a fresh Agent + BenchChannel + InstrumentedLlm for the task,
/// injects the prompt, waits for the response, and returns the result.
async fn run_task_isolated(params: TaskRunParams<'_>) -> TaskResult {
    let TaskRunParams {
        task,
        suite_id,
        config_label,
        llm,
        safety,
        timeout,
        additional_tools,
    } = params;

    let started_at = Utc::now();
    let start = Instant::now();

    // Wrap LLM with instrumentation
    let instrumented = Arc::new(InstrumentedLlm::new(llm));

    // Create bench channel
    let (bench_channel, msg_tx) = BenchChannel::new();
    let capture = bench_channel.capture();

    // Build tool registry
    let tools = Arc::new(ToolRegistry::new());
    tools.register_builtin_tools();

    // Register additional suite-specific tools
    for tool in additional_tools {
        tools.register(Arc::clone(tool)).await;
    }

    // Build agent config (minimal, headless)
    let agent_config = AgentConfig {
        name: format!("bench-{}", task.id),
        max_parallel_jobs: 1,
        job_timeout: timeout,
        stuck_threshold: timeout,
        repair_check_interval: timeout + std::time::Duration::from_secs(999),
        max_repair_attempts: 0,
        use_planning: false,
        session_idle_timeout: timeout,
        allow_local_tools: true,
    };

    let deps = AgentDeps {
        store: None,
        llm: instrumented.clone() as Arc<dyn LlmProvider>,
        cheap_llm: None,
        safety,
        tools,
        workspace: None,
        extension_manager: None,
        hooks: Arc::new(ironclaw::hooks::HookRegistry::new()),
    };

    let mut channels = ChannelManager::new();
    channels.add(Box::new(bench_channel));

    let agent = Agent::new(agent_config, deps, channels, None, None, None, None);

    // Build the full prompt with context
    let full_prompt = if let Some(ref ctx) = task.context {
        format!("{}\n\nContext:\n{}", task.prompt, ctx)
    } else {
        task.prompt.clone()
    };

    // Inject the task prompt
    let incoming = IncomingMessage::new("bench", "bench-user", &full_prompt);
    if msg_tx.send(incoming).await.is_err() {
        return make_error_result(
            task,
            suite_id,
            config_label,
            started_at,
            "failed to send prompt",
        );
    }

    // Record prompt in conversation
    {
        let mut cap = capture.lock().await;
        cap.conversation.push(ConversationTurn {
            role: TurnRole::User,
            content: full_prompt,
        });
    }

    // Run agent with timeout.
    // After the first response, send /quit to end the session.
    let quit_tx = msg_tx.clone();
    let capture_for_quit = Arc::clone(&capture);
    let quit_handle = tokio::spawn(async move {
        // Poll for first response
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let cap = capture_for_quit.lock().await;
            if !cap.responses.is_empty() {
                break;
            }
        }
        // Give a small grace period for any final status events
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let quit = IncomingMessage::new("bench", "bench-user", "/quit");
        let _ = quit_tx.send(quit).await;
    });

    let agent_result = tokio::time::timeout(timeout, agent.run()).await;

    quit_handle.abort();

    let wall_time = start.elapsed();
    let hit_timeout = agent_result.is_err();

    if let Ok(Err(e)) = &agent_result {
        tracing::warn!("Agent error for task {}: {}", task.id, e);
    }

    // Extract results from capture
    let cap = capture.lock().await;
    let response = cap.responses.last().cloned().unwrap_or_default();

    let trace = Trace {
        wall_time_ms: wall_time.as_millis() as u64,
        llm_calls: instrumented.call_count(),
        input_tokens: instrumented.total_input_tokens(),
        output_tokens: instrumented.total_output_tokens(),
        estimated_cost_usd: instrumented.estimated_cost(),
        tool_calls: cap.tool_calls.clone(),
        turns: cap.responses.len() as u32,
        hit_iteration_limit: false,
        hit_timeout,
    };

    let error = if hit_timeout {
        Some(format!("timeout after {}s", timeout.as_secs()))
    } else if let Ok(Err(e)) = &agent_result {
        Some(e.to_string())
    } else {
        None
    };

    TaskResult {
        task_id: task.id.clone(),
        suite_id: suite_id.to_string(),
        score: crate::suite::BenchScore {
            value: 0.0,
            label: "pending".to_string(),
            details: None,
        },
        trace,
        response,
        started_at,
        finished_at: Utc::now(),
        config_label: config_label.to_string(),
        error,
    }
}

fn make_error_result(
    task: &BenchTask,
    suite_id: &str,
    config_label: &str,
    started_at: chrono::DateTime<Utc>,
    reason: &str,
) -> TaskResult {
    TaskResult {
        task_id: task.id.clone(),
        suite_id: suite_id.to_string(),
        score: crate::suite::BenchScore::fail(reason),
        trace: Trace {
            wall_time_ms: 0,
            llm_calls: 0,
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_usd: 0.0,
            tool_calls: vec![],
            turns: 0,
            hit_iteration_limit: false,
            hit_timeout: false,
        },
        response: String::new(),
        started_at,
        finished_at: Utc::now(),
        config_label: config_label.to_string(),
        error: Some(reason.to_string()),
    }
}

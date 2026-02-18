use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use tokio::sync::Mutex;
use uuid::Uuid;

use ironclaw::llm::LlmProvider;

use crate::agentic::AgenticLoop;
use crate::config::{BenchConfig, MatrixEntry};
use crate::error::BenchError;
use crate::instrumented_llm::InstrumentedLlm;
use crate::results::{
    RunResult, TaskResult, Trace, append_task_result, completed_task_ids, run_dir, run_json_path,
    tasks_jsonl_path, write_run_result, write_task_results,
};
use crate::suite::{BenchSuite, BenchTask, TaskSubmission};

/// Parameters for running a single task in isolation.
struct TaskRunParams {
    task_id: String,
    suite_id: String,
    config_label: String,
    llm: Arc<dyn LlmProvider>,
    timeout: std::time::Duration,
    max_iterations: usize,
    task_tools: Vec<Arc<dyn ironclaw::tools::Tool>>,
    system_prompt: String,
    user_prompt: String,
}

const DEFAULT_SYSTEM_PROMPT: &str = "\
You are an AI assistant completing a benchmark task. \
Use the provided tools to accomplish the task described in the user message.";

/// Orchestrates benchmark execution: loads tasks, runs the agentic loop per
/// task, scores results, writes JSONL output.
pub struct BenchRunner {
    suite: Arc<dyn BenchSuite>,
    config: BenchConfig,
    llm: Arc<dyn LlmProvider>,
}

impl BenchRunner {
    pub fn new(suite: Box<dyn BenchSuite>, config: BenchConfig, llm: Arc<dyn LlmProvider>) -> Self {
        Self {
            suite: Arc::from(suite),
            config,
            llm,
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

        // Load all tasks once (used for both execution and scoring)
        let all_tasks = self.suite.load_tasks().await?;
        let task_index: HashMap<String, BenchTask> = all_tasks
            .iter()
            .map(|t| (t.id.clone(), t.clone()))
            .collect();

        // Filter tasks for execution
        let mut tasks = all_tasks;

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
        let model_label = matrix.model.as_deref().unwrap_or(self.llm.model_name());
        let commit_hash = git_short_hash();
        let system_prompt = self
            .suite
            .system_prompt()
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());

        tracing::info!(
            "[{} @ {}] Running {} tasks for suite '{}' (run: {})",
            model_label,
            commit_hash,
            tasks.len(),
            self.suite.id(),
            run_id
        );

        let started_at = Utc::now();
        let all_results: Arc<Mutex<Vec<TaskResult>>> =
            Arc::new(Mutex::new(Vec::with_capacity(tasks.len())));

        if self.config.parallelism <= 1 {
            // Sequential execution
            for (i, task) in tasks.iter().enumerate() {
                tracing::info!(
                    "[{}/{}] Running task: {}",
                    i + 1 + completed.len(),
                    total_tasks,
                    task.id
                );
                if let Err(e) = self.suite.setup_task(task).await {
                    tracing::warn!("setup_task failed for {}: {}", task.id, e);
                    let result = make_error_result(
                        task,
                        self.suite.id(),
                        &matrix.label,
                        Utc::now(),
                        &format!("setup_task failed: {e}"),
                    );
                    append_task_result(&jsonl_path, &result)?;
                    all_results.lock().await.push(result);
                    continue;
                }

                let user_prompt = build_user_prompt(task);
                let params = TaskRunParams {
                    task_id: task.id.clone(),
                    suite_id: self.suite.id().to_string(),
                    config_label: matrix.label.clone(),
                    llm: Arc::clone(&self.llm),
                    timeout: task.timeout.unwrap_or(self.config.task_timeout),
                    max_iterations: self.config.max_iterations,
                    task_tools: self.suite.task_tools(task),
                    system_prompt: system_prompt.clone(),
                    user_prompt,
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
                let timeout = task.timeout.unwrap_or(self.config.task_timeout);
                let max_iterations = self.config.max_iterations;
                let results_ref = Arc::clone(&all_results);
                let completed_count = completed.len();
                let total = total_tasks;
                let sys_prompt = system_prompt.clone();

                handles.push(tokio::spawn(async move {
                    let _permit = match sem.acquire().await {
                        Ok(p) => p,
                        Err(_) => {
                            tracing::error!("Semaphore closed for task {}", task.id);
                            return;
                        }
                    };
                    tracing::info!(
                        "[{}/{}] Running task: {}",
                        i + 1 + completed_count,
                        total,
                        task.id
                    );
                    if let Err(e) = suite.setup_task(&task).await {
                        tracing::warn!("setup_task failed for {}: {}", task.id, e);
                        let result = make_error_result(
                            &task,
                            suite.id(),
                            &config_label,
                            Utc::now(),
                            &format!("setup_task failed: {e}"),
                        );
                        results_ref.lock().await.push(result);
                        return;
                    }

                    let user_prompt = build_user_prompt(&task);
                    let suite_id = suite.id().to_string();
                    let task_tools = suite.task_tools(&task);
                    let params = TaskRunParams {
                        task_id: task.id.clone(),
                        suite_id,
                        config_label,
                        llm,
                        timeout,
                        max_iterations,
                        task_tools,
                        system_prompt: sys_prompt,
                        user_prompt,
                    };
                    let result = run_task_isolated(params).await;
                    if let Err(e) = suite.teardown_task(&task).await {
                        tracing::warn!("teardown_task failed for {}: {}", task.id, e);
                    }
                    results_ref.lock().await.push(result);
                }));
            }

            for handle in handles {
                if let Err(e) = handle.await {
                    tracing::error!("Task panicked: {}", e);
                }
            }

            // Write all results to JSONL after parallel execution completes.
            // This avoids the race condition of concurrent file appends.
            let results = all_results.lock().await;
            for result in results.iter() {
                append_task_result(&jsonl_path, result)?;
            }
        }

        // Score all results using the cached task index
        let results = all_results.lock().await;
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
            &commit_hash,
            total_tasks,
            &all_for_aggregate,
            started_at,
        );

        write_run_result(&json_path, &run_result)?;

        tracing::info!(
            "[{} @ {}] Run {} complete: {:.1}% pass rate, {:.3} avg score, ${:.4} cost",
            model_name,
            commit_hash,
            run_id,
            run_result.pass_rate * 100.0,
            run_result.avg_score,
            run_result.total_cost_usd,
        );

        Ok(run_id)
    }
}

/// Build the user prompt from a task, including context if present.
fn build_user_prompt(task: &BenchTask) -> String {
    if let Some(ref ctx) = task.context {
        format!("{}\n\nContext:\n{}", task.prompt, ctx)
    } else {
        task.prompt.clone()
    }
}

/// Run a single benchmark task using the direct agentic loop.
///
/// Creates an InstrumentedLlm + AgenticLoop, runs until completion or timeout,
/// and returns the TaskResult.
async fn run_task_isolated(params: TaskRunParams) -> TaskResult {
    let TaskRunParams {
        task_id,
        suite_id,
        config_label,
        llm,
        timeout,
        max_iterations,
        task_tools,
        system_prompt,
        user_prompt,
    } = params;

    let started_at = Utc::now();
    let start = Instant::now();

    // Wrap LLM with instrumentation
    let instrumented = Arc::new(InstrumentedLlm::new(llm));

    let agentic = AgenticLoop::new(
        instrumented.clone() as Arc<dyn LlmProvider>,
        task_tools,
        max_iterations,
    );

    let agentic_result =
        tokio::time::timeout(timeout, agentic.run(&system_prompt, &user_prompt)).await;

    let wall_time = start.elapsed();

    match agentic_result {
        Ok(Ok(result)) => {
            let trace = Trace {
                wall_time_ms: wall_time.as_millis() as u64,
                llm_calls: instrumented.call_count(),
                input_tokens: instrumented.total_input_tokens(),
                output_tokens: instrumented.total_output_tokens(),
                estimated_cost_usd: instrumented.estimated_cost(),
                tool_calls: result.tool_calls,
                turns: result.iterations as u32,
                hit_iteration_limit: result.hit_iteration_limit,
                hit_timeout: false,
            };

            TaskResult {
                task_id,
                suite_id,
                score: crate::suite::BenchScore {
                    value: 0.0,
                    label: "pending".to_string(),
                    details: None,
                },
                trace,
                response: result.response,
                started_at,
                finished_at: Utc::now(),
                config_label,
                error: None,
            }
        }
        Ok(Err(e)) => {
            tracing::warn!("Agentic loop error for task {task_id}: {e}");
            make_error_result_raw(
                &task_id,
                &suite_id,
                &config_label,
                started_at,
                &e.to_string(),
            )
        }
        Err(_) => {
            tracing::warn!("Task {task_id} timed out after {}s", timeout.as_secs());
            let trace = Trace {
                wall_time_ms: wall_time.as_millis() as u64,
                llm_calls: instrumented.call_count(),
                input_tokens: instrumented.total_input_tokens(),
                output_tokens: instrumented.total_output_tokens(),
                estimated_cost_usd: instrumented.estimated_cost(),
                tool_calls: vec![],
                turns: 0,
                hit_iteration_limit: false,
                hit_timeout: true,
            };
            TaskResult {
                task_id,
                suite_id,
                score: crate::suite::BenchScore {
                    value: 0.0,
                    label: "pending".to_string(),
                    details: None,
                },
                trace,
                response: String::new(),
                started_at,
                finished_at: Utc::now(),
                config_label,
                error: Some(format!("timeout after {}s", timeout.as_secs())),
            }
        }
    }
}

fn make_error_result(
    task: &BenchTask,
    suite_id: &str,
    config_label: &str,
    started_at: chrono::DateTime<Utc>,
    reason: &str,
) -> TaskResult {
    make_error_result_raw(&task.id, suite_id, config_label, started_at, reason)
}

fn make_error_result_raw(
    task_id: &str,
    suite_id: &str,
    config_label: &str,
    started_at: chrono::DateTime<Utc>,
    reason: &str,
) -> TaskResult {
    TaskResult {
        task_id: task_id.to_string(),
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

/// Get the short git commit hash of HEAD, or "unknown" if not in a repo.
fn git_short_hash() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

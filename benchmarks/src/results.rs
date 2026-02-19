use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::BenchError;
use crate::suite::BenchScore;

/// Metrics from a single task run: LLM usage, timing, tool calls.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Trace {
    pub wall_time_ms: u64,
    pub llm_calls: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub estimated_cost_usd: f64,
    pub tool_calls: Vec<TraceToolCall>,
    pub turns: u32,
    pub hit_iteration_limit: bool,
    pub hit_timeout: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TraceToolCall {
    pub name: String,
    pub duration_ms: u64,
    pub success: bool,
}

/// Result of running a single benchmark task.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskResult {
    pub task_id: String,
    pub suite_id: String,
    pub score: BenchScore,
    pub trace: Trace,
    pub response: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub config_label: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// Aggregate results for a full benchmark run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunResult {
    pub run_id: Uuid,
    pub suite_id: String,
    pub config_label: String,
    pub model: String,
    /// Short git commit hash at the time of the run.
    #[serde(default)]
    pub commit_hash: String,
    pub pass_rate: f64,
    pub avg_score: f64,
    pub total_tasks: usize,
    pub completed_tasks: usize,
    pub total_cost_usd: f64,
    pub total_wall_time_ms: u64,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

impl RunResult {
    /// Build aggregate from individual task results.
    #[allow(clippy::too_many_arguments)]
    pub fn from_tasks(
        run_id: Uuid,
        suite_id: &str,
        config_label: &str,
        model: &str,
        commit_hash: &str,
        total_tasks: usize,
        tasks: &[TaskResult],
        started_at: DateTime<Utc>,
    ) -> Self {
        let pass_count = tasks.iter().filter(|t| t.score.value >= 1.0).count();
        let pass_rate = if tasks.is_empty() {
            0.0
        } else {
            pass_count as f64 / tasks.len() as f64
        };
        let avg_score = if tasks.is_empty() {
            0.0
        } else {
            tasks.iter().map(|t| t.score.value).sum::<f64>() / tasks.len() as f64
        };
        let total_cost: f64 = tasks.iter().map(|t| t.trace.estimated_cost_usd).sum();
        let total_wall: u64 = tasks.iter().map(|t| t.trace.wall_time_ms).sum();

        Self {
            run_id,
            suite_id: suite_id.to_string(),
            config_label: config_label.to_string(),
            model: model.to_string(),
            commit_hash: commit_hash.to_string(),
            pass_rate,
            avg_score,
            total_tasks,
            completed_tasks: tasks.len(),
            total_cost_usd: total_cost,
            total_wall_time_ms: total_wall,
            started_at,
            finished_at: Utc::now(),
        }
    }
}

/// Append a single task result as one JSON line to the JSONL file.
pub fn append_task_result(path: &Path, result: &TaskResult) -> Result<(), BenchError> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(result)?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Overwrite the JSONL file with the given results (used after scoring).
pub fn write_task_results(path: &Path, results: &[TaskResult]) -> Result<(), BenchError> {
    let mut file = std::fs::File::create(path)?;
    for result in results {
        let line = serde_json::to_string(result)?;
        writeln!(file, "{line}")?;
    }
    Ok(())
}

/// Read all task results from a JSONL file.
pub fn read_task_results(path: &Path) -> Result<Vec<TaskResult>, BenchError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut results = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let result: TaskResult = serde_json::from_str(trimmed)?;
        results.push(result);
    }
    Ok(results)
}

/// Write the aggregate run result as JSON.
pub fn write_run_result(path: &Path, result: &RunResult) -> Result<(), BenchError> {
    let json = serde_json::to_string_pretty(result)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Read the aggregate run result from JSON.
pub fn read_run_result(path: &Path) -> Result<RunResult, BenchError> {
    let json = std::fs::read_to_string(path)?;
    let result: RunResult = serde_json::from_str(&json)?;
    Ok(result)
}

/// Get the set of already-completed task IDs from a JSONL file (for resume).
///
/// Only includes tasks that have been scored (label != "pending"). Tasks that
/// were written but not scored (e.g., from an interrupted run) will be re-executed.
pub fn completed_task_ids(path: &Path) -> Result<HashSet<String>, BenchError> {
    let results = read_task_results(path)?;
    Ok(results
        .into_iter()
        .filter(|r| r.score.label != "pending")
        .map(|r| r.task_id)
        .collect())
}

/// Get the results directory for a specific run.
pub fn run_dir(base: &Path, run_id: Uuid) -> PathBuf {
    base.join(run_id.to_string())
}

/// Get the tasks JSONL path for a run.
pub fn tasks_jsonl_path(base: &Path, run_id: Uuid) -> PathBuf {
    run_dir(base, run_id).join("tasks.jsonl")
}

/// Get the run JSON path for a run.
pub fn run_json_path(base: &Path, run_id: Uuid) -> PathBuf {
    run_dir(base, run_id).join("run.json")
}

/// Find the latest run directory by the modification time of its `run.json`.
///
/// Falls back to `tasks.jsonl` mtime, then directory mtime. This avoids the
/// issue where modifying files inside a directory doesn't update the directory's
/// mtime on many filesystems.
pub fn find_latest_run(base: &Path) -> Result<Option<Uuid>, BenchError> {
    if !base.exists() {
        return Ok(None);
    }
    let mut entries: Vec<_> = std::fs::read_dir(base)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let uuid = Uuid::parse_str(&name).ok()?;
            let dir_path = e.path();
            // Prefer run.json mtime, fall back to tasks.jsonl, then directory
            let modified = std::fs::metadata(dir_path.join("run.json"))
                .and_then(|m| m.modified())
                .or_else(|_| {
                    std::fs::metadata(dir_path.join("tasks.jsonl")).and_then(|m| m.modified())
                })
                .or_else(|_| e.metadata().and_then(|m| m.modified()))
                .ok()?;
            Some((uuid, modified))
        })
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    Ok(entries.first().map(|(uuid, _)| *uuid))
}

/// Print a summary table of task results.
pub fn print_results_table(tasks: &[TaskResult], run: &RunResult) {
    println!();
    let commit_suffix = if run.commit_hash.is_empty() {
        String::new()
    } else {
        format!(" | Commit: {}", run.commit_hash)
    };
    println!(
        "Run: {} | Suite: {} | Model: {}{}",
        run.run_id, run.suite_id, run.model, commit_suffix
    );
    println!(
        "Pass rate: {:.1}% | Avg score: {:.3} | Tasks: {}/{} | Cost: ${:.4} | Time: {:.1}s",
        run.pass_rate * 100.0,
        run.avg_score,
        run.completed_tasks,
        run.total_tasks,
        run.total_cost_usd,
        run.total_wall_time_ms as f64 / 1000.0,
    );
    println!();

    // Header
    println!(
        "{:<30} {:>6} {:>7} {:>8} {:>10} {:>6} {:>8}",
        "Task ID", "Score", "Label", "Tokens", "Cost", "Turns", "Time"
    );
    println!("{}", "-".repeat(80));

    for task in tasks {
        let total_tokens = task.trace.input_tokens + task.trace.output_tokens;
        let task_id_display = if task.task_id.len() > 28 {
            let truncated: String = task.task_id.chars().take(25).collect();
            format!("{truncated}...")
        } else {
            task.task_id.clone()
        };
        println!(
            "{:<30} {:>6.3} {:>7} {:>8} {:>10.4} {:>6} {:>7.1}s",
            task_id_display,
            task.score.value,
            task.score.label,
            total_tokens,
            task.trace.estimated_cost_usd,
            task.trace.turns,
            task.trace.wall_time_ms as f64 / 1000.0,
        );
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_result_from_tasks() {
        let tasks = vec![
            TaskResult {
                task_id: "t1".to_string(),
                suite_id: "custom".to_string(),
                score: BenchScore {
                    value: 1.0,
                    label: "pass".to_string(),
                    details: None,
                },
                trace: Trace {
                    wall_time_ms: 1000,
                    llm_calls: 2,
                    input_tokens: 100,
                    output_tokens: 50,
                    estimated_cost_usd: 0.01,
                    tool_calls: vec![],
                    turns: 1,
                    hit_iteration_limit: false,
                    hit_timeout: false,
                },
                response: "answer".to_string(),
                started_at: Utc::now(),
                finished_at: Utc::now(),
                config_label: "default".to_string(),
                error: None,
            },
            TaskResult {
                task_id: "t2".to_string(),
                suite_id: "custom".to_string(),
                score: BenchScore {
                    value: 0.0,
                    label: "fail".to_string(),
                    details: Some("wrong".to_string()),
                },
                trace: Trace {
                    wall_time_ms: 2000,
                    llm_calls: 3,
                    input_tokens: 200,
                    output_tokens: 100,
                    estimated_cost_usd: 0.02,
                    tool_calls: vec![],
                    turns: 2,
                    hit_iteration_limit: false,
                    hit_timeout: false,
                },
                response: "wrong answer".to_string(),
                started_at: Utc::now(),
                finished_at: Utc::now(),
                config_label: "default".to_string(),
                error: None,
            },
        ];

        let run = RunResult::from_tasks(
            Uuid::new_v4(),
            "custom",
            "default",
            "test-model",
            "abc1234",
            2,
            &tasks,
            Utc::now(),
        );

        assert_eq!(run.pass_rate, 0.5);
        assert_eq!(run.avg_score, 0.5);
        assert_eq!(run.total_tasks, 2);
        assert_eq!(run.completed_tasks, 2);
        assert!((run.total_cost_usd - 0.03).abs() < f64::EPSILON);
        assert_eq!(run.total_wall_time_ms, 3000);
    }

    #[test]
    fn test_jsonl_roundtrip() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("tasks.jsonl");

        let result = TaskResult {
            task_id: "round-trip-test".to_string(),
            suite_id: "custom".to_string(),
            score: BenchScore::pass(),
            trace: Trace {
                wall_time_ms: 500,
                llm_calls: 1,
                input_tokens: 10,
                output_tokens: 5,
                estimated_cost_usd: 0.001,
                tool_calls: vec![],
                turns: 1,
                hit_iteration_limit: false,
                hit_timeout: false,
            },
            response: "hello".to_string(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            config_label: "test".to_string(),
            error: None,
        };

        append_task_result(&path, &result).expect("append");
        append_task_result(&path, &result).expect("append");

        let loaded = read_task_results(&path).expect("read");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].task_id, "round-trip-test");
    }

    #[test]
    fn test_completed_task_ids() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("tasks.jsonl");

        let result = TaskResult {
            task_id: "unique-id-1".to_string(),
            suite_id: "custom".to_string(),
            score: BenchScore::pass(),
            trace: Trace {
                wall_time_ms: 100,
                llm_calls: 1,
                input_tokens: 10,
                output_tokens: 5,
                estimated_cost_usd: 0.0,
                tool_calls: vec![],
                turns: 1,
                hit_iteration_limit: false,
                hit_timeout: false,
            },
            response: "x".to_string(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            config_label: "test".to_string(),
            error: None,
        };
        append_task_result(&path, &result).expect("append");

        let ids = completed_task_ids(&path).expect("ids");
        assert!(ids.contains("unique-id-1"));
        assert!(!ids.contains("unique-id-2"));
    }

    #[test]
    fn test_write_task_results_overwrites() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("tasks.jsonl");

        // Write initial "pending" result via append
        let pending = TaskResult {
            task_id: "t1".to_string(),
            suite_id: "spot".to_string(),
            score: BenchScore {
                value: 0.0,
                label: "pending".to_string(),
                details: None,
            },
            trace: Trace {
                wall_time_ms: 100,
                llm_calls: 1,
                input_tokens: 10,
                output_tokens: 5,
                estimated_cost_usd: 0.001,
                tool_calls: vec![],
                turns: 1,
                hit_iteration_limit: false,
                hit_timeout: false,
            },
            response: "42".to_string(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            config_label: "default".to_string(),
            error: None,
        };
        append_task_result(&path, &pending).expect("append");

        // Verify pending score
        let before = read_task_results(&path).expect("read");
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].score.label, "pending");

        // Overwrite with scored result
        let mut scored = pending;
        scored.score = BenchScore::pass();
        write_task_results(&path, &[scored]).expect("write");

        // Verify scored result replaced pending
        let after = read_task_results(&path).expect("read");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].score.label, "pass");
        assert_eq!(after[0].score.value, 1.0);
    }
}

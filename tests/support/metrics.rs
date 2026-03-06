#![allow(dead_code)]
//! Metrics types for test instrumentation.
//!
//! These types were previously in the `ironclaw::benchmark::metrics` module.
//! They now live directly in the test support crate to keep benchmark-specific
//! types out of the main library.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Per-scenario metrics
// ---------------------------------------------------------------------------

/// Execution metrics collected from a single scenario run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceMetrics {
    /// Wall-clock time in milliseconds for the entire scenario.
    pub wall_time_ms: u64,
    /// Number of LLM API calls made.
    pub llm_calls: u32,
    /// Total input tokens across all LLM calls.
    pub input_tokens: u32,
    /// Total output tokens across all LLM calls.
    pub output_tokens: u32,
    /// Estimated cost in USD (input + output token costs).
    pub estimated_cost_usd: f64,
    /// Per-tool-call invocation records.
    pub tool_calls: Vec<ToolInvocation>,
    /// Number of agent turns (message send -> response cycles).
    pub turns: u32,
    /// Whether the agent hit its max_tool_iterations limit.
    pub hit_iteration_limit: bool,
    /// Whether the scenario timed out waiting for responses.
    pub hit_timeout: bool,
}

impl TraceMetrics {
    /// Total number of tool invocations.
    pub fn total_tool_calls(&self) -> usize {
        self.tool_calls.len()
    }

    /// Number of tool invocations that failed.
    pub fn failed_tool_calls(&self) -> usize {
        self.tool_calls.iter().filter(|t| !t.success).count()
    }

    /// Total tool execution time in milliseconds.
    pub fn total_tool_time_ms(&self) -> u64 {
        self.tool_calls.iter().map(|t| t.duration_ms).sum()
    }
}

/// A single tool invocation with timing and success status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvocation {
    /// Tool name.
    pub name: String,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the tool completed successfully.
    pub success: bool,
}

// ---------------------------------------------------------------------------
// Per-turn metrics (multi-turn scenarios)
// ---------------------------------------------------------------------------

/// Per-turn metrics for multi-turn scenarios.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnMetrics {
    pub turn_index: usize,
    pub user_message: String,
    pub wall_time_ms: u64,
    pub llm_calls: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub tool_calls: Vec<ToolInvocation>,
    pub response: String,
    pub assertions_passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_score: Option<u8>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Scenario result
// ---------------------------------------------------------------------------

/// Result of running a single test scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    /// Unique identifier for this scenario (e.g., test function name).
    pub scenario_id: String,
    /// Whether all assertions passed.
    pub passed: bool,
    /// Execution metrics.
    pub trace: TraceMetrics,
    /// The agent's final response text.
    pub response: String,
    /// Error message if the scenario failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Per-turn metrics for multi-turn scenarios.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub turn_metrics: Vec<TurnMetrics>,
}

// ---------------------------------------------------------------------------
// Run result (aggregate)
// ---------------------------------------------------------------------------

/// Aggregate results across multiple scenario runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    /// Unique run identifier.
    pub run_id: String,
    /// Fraction of scenarios that passed (0.0 - 1.0).
    pub pass_rate: f64,
    /// Total estimated cost across all scenarios.
    pub total_cost_usd: f64,
    /// Total wall-clock time across all scenarios.
    pub total_wall_time_ms: u64,
    /// Individual scenario results.
    pub scenarios: Vec<ScenarioResult>,
    /// Git commit hash for reproducibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_hash: Option<String>,
    /// Number of scenarios skipped (e.g., due to budget cap).
    #[serde(default)]
    pub skipped_scenarios: usize,
}

impl RunResult {
    /// Build a RunResult from a list of scenario results.
    pub fn from_scenarios(run_id: impl Into<String>, scenarios: Vec<ScenarioResult>) -> Self {
        let passed = scenarios.iter().filter(|s| s.passed).count();
        let pass_rate = if scenarios.is_empty() {
            0.0
        } else {
            passed as f64 / scenarios.len() as f64
        };
        let total_cost_usd: f64 = scenarios.iter().map(|s| s.trace.estimated_cost_usd).sum();
        let total_wall_time_ms: u64 = scenarios.iter().map(|s| s.trace.wall_time_ms).sum();

        Self {
            run_id: run_id.into(),
            pass_rate,
            total_cost_usd,
            total_wall_time_ms,
            scenarios,
            commit_hash: None,
            skipped_scenarios: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Baseline comparison
// ---------------------------------------------------------------------------

/// A single metric comparison between baseline and current run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDelta {
    pub scenario_id: String,
    pub metric: String,
    pub baseline: f64,
    pub current: f64,
    pub delta: f64,
    /// Positive delta means regression (worse), negative means improvement.
    pub is_regression: bool,
}

/// Compare a current run against a baseline, identifying regressions and improvements.
pub fn compare_runs(baseline: &RunResult, current: &RunResult, threshold: f64) -> Vec<MetricDelta> {
    let mut deltas = Vec::new();

    for current_scenario in &current.scenarios {
        let Some(baseline_scenario) = baseline
            .scenarios
            .iter()
            .find(|b| b.scenario_id == current_scenario.scenario_id)
        else {
            continue;
        };

        // Wall time comparison.
        let b_time = baseline_scenario.trace.wall_time_ms as f64;
        let c_time = current_scenario.trace.wall_time_ms as f64;
        if b_time > 0.0 {
            let delta = (c_time - b_time) / b_time;
            if delta.abs() > threshold {
                deltas.push(MetricDelta {
                    scenario_id: current_scenario.scenario_id.clone(),
                    metric: "wall_time_ms".to_string(),
                    baseline: b_time,
                    current: c_time,
                    delta,
                    is_regression: delta > 0.0,
                });
            }
        }

        // Token count comparison (input + output).
        let b_tokens =
            (baseline_scenario.trace.input_tokens + baseline_scenario.trace.output_tokens) as f64;
        let c_tokens =
            (current_scenario.trace.input_tokens + current_scenario.trace.output_tokens) as f64;
        if b_tokens > 0.0 {
            let delta = (c_tokens - b_tokens) / b_tokens;
            if delta.abs() > threshold {
                deltas.push(MetricDelta {
                    scenario_id: current_scenario.scenario_id.clone(),
                    metric: "total_tokens".to_string(),
                    baseline: b_tokens,
                    current: c_tokens,
                    delta,
                    is_regression: delta > 0.0,
                });
            }
        }

        // LLM calls comparison.
        let b_calls = baseline_scenario.trace.llm_calls as f64;
        let c_calls = current_scenario.trace.llm_calls as f64;
        if b_calls > 0.0 {
            let delta = (c_calls - b_calls) / b_calls;
            if delta.abs() > threshold {
                deltas.push(MetricDelta {
                    scenario_id: current_scenario.scenario_id.clone(),
                    metric: "llm_calls".to_string(),
                    baseline: b_calls,
                    current: c_calls,
                    delta,
                    is_regression: delta > 0.0,
                });
            }
        }

        // Tool call count comparison.
        let b_tools = baseline_scenario.trace.tool_calls.len() as f64;
        let c_tools = current_scenario.trace.tool_calls.len() as f64;
        if b_tools > 0.0 {
            let delta = (c_tools - b_tools) / b_tools;
            if delta.abs() > threshold {
                deltas.push(MetricDelta {
                    scenario_id: current_scenario.scenario_id.clone(),
                    metric: "tool_calls".to_string(),
                    baseline: b_tools,
                    current: c_tools,
                    delta,
                    is_regression: delta > 0.0,
                });
            }
        }
    }

    deltas
}

//! Metrics types for agent benchmarking.
//!
//! Matches the metric model from `nearai/benchmarks` (Trace, TaskResult, RunResult)
//! so that results are comparable across the two harnesses.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Per-scenario metrics (matches nearai/benchmarks Trace)
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
        }
    }

    /// Set the git commit hash.
    pub fn with_commit_hash(mut self, hash: impl Into<String>) -> Self {
        self.commit_hash = Some(hash.into());
        self
    }

    /// Average input tokens per scenario.
    pub fn avg_input_tokens(&self) -> f64 {
        if self.scenarios.is_empty() {
            return 0.0;
        }
        let total: u32 = self.scenarios.iter().map(|s| s.trace.input_tokens).sum();
        total as f64 / self.scenarios.len() as f64
    }

    /// Average output tokens per scenario.
    pub fn avg_output_tokens(&self) -> f64 {
        if self.scenarios.is_empty() {
            return 0.0;
        }
        let total: u32 = self.scenarios.iter().map(|s| s.trace.output_tokens).sum();
        total as f64 / self.scenarios.len() as f64
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
///
/// Thresholds:
/// - Cost regression: current > baseline * (1 + threshold)
/// - Latency regression: current > baseline * (1 + threshold)
/// - Token regression: current > baseline * (1 + threshold)
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trace(wall_time_ms: u64, llm_calls: u32, input_tokens: u32) -> TraceMetrics {
        TraceMetrics {
            wall_time_ms,
            llm_calls,
            input_tokens,
            output_tokens: 20,
            estimated_cost_usd: 0.001,
            tool_calls: vec![
                ToolInvocation {
                    name: "echo".to_string(),
                    duration_ms: 5,
                    success: true,
                },
                ToolInvocation {
                    name: "write_file".to_string(),
                    duration_ms: 10,
                    success: false,
                },
            ],
            turns: 1,
            hit_iteration_limit: false,
            hit_timeout: false,
        }
    }

    #[test]
    fn test_trace_metrics_helpers() {
        let trace = sample_trace(100, 2, 50);
        assert_eq!(trace.total_tool_calls(), 2);
        assert_eq!(trace.failed_tool_calls(), 1);
        assert_eq!(trace.total_tool_time_ms(), 15);
    }

    #[test]
    fn test_run_result_aggregation() {
        let scenarios = vec![
            ScenarioResult {
                scenario_id: "test_a".to_string(),
                passed: true,
                trace: sample_trace(100, 2, 50),
                response: "ok".to_string(),
                error: None,
                turn_metrics: Vec::new(),
            },
            ScenarioResult {
                scenario_id: "test_b".to_string(),
                passed: false,
                trace: sample_trace(200, 3, 80),
                response: "fail".to_string(),
                error: Some("assertion failed".to_string()),
                turn_metrics: Vec::new(),
            },
        ];
        let run = RunResult::from_scenarios("run-1", scenarios);
        assert_eq!(run.pass_rate, 0.5);
        assert_eq!(run.total_wall_time_ms, 300);
        assert_eq!(run.avg_input_tokens(), 65.0); // (50 + 80) / 2
    }

    #[test]
    fn test_baseline_comparison_detects_regression() {
        let baseline = RunResult::from_scenarios(
            "baseline",
            vec![ScenarioResult {
                scenario_id: "test_a".to_string(),
                passed: true,
                trace: sample_trace(100, 2, 50),
                response: "ok".to_string(),
                error: None,
                turn_metrics: Vec::new(),
            }],
        );
        let current = RunResult::from_scenarios(
            "current",
            vec![ScenarioResult {
                scenario_id: "test_a".to_string(),
                passed: true,
                // Double the wall time -- should be a regression.
                trace: sample_trace(200, 2, 50),
                response: "ok".to_string(),
                error: None,
                turn_metrics: Vec::new(),
            }],
        );

        let deltas = compare_runs(&baseline, &current, 0.10);
        let time_delta = deltas.iter().find(|d| d.metric == "wall_time_ms");
        assert!(time_delta.is_some(), "Expected wall_time_ms delta");
        let d = time_delta.unwrap();
        assert!(d.is_regression);
        assert!((d.delta - 1.0).abs() < 0.01); // 100% increase
    }

    #[test]
    fn test_baseline_comparison_detects_improvement() {
        let baseline = RunResult::from_scenarios(
            "baseline",
            vec![ScenarioResult {
                scenario_id: "test_a".to_string(),
                passed: true,
                trace: sample_trace(200, 4, 100),
                response: "ok".to_string(),
                error: None,
                turn_metrics: Vec::new(),
            }],
        );
        let current = RunResult::from_scenarios(
            "current",
            vec![ScenarioResult {
                scenario_id: "test_a".to_string(),
                passed: true,
                // Half the wall time and calls -- improvement.
                trace: sample_trace(100, 2, 50),
                response: "ok".to_string(),
                error: None,
                turn_metrics: Vec::new(),
            }],
        );

        let deltas = compare_runs(&baseline, &current, 0.10);
        let time_delta = deltas.iter().find(|d| d.metric == "wall_time_ms");
        assert!(time_delta.is_some());
        assert!(!time_delta.unwrap().is_regression);

        let call_delta = deltas.iter().find(|d| d.metric == "llm_calls");
        assert!(call_delta.is_some());
        assert!(!call_delta.unwrap().is_regression);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let trace = sample_trace(100, 2, 50);
        let json = serde_json::to_string(&trace).unwrap();
        let deserialized: TraceMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.wall_time_ms, trace.wall_time_ms);
        assert_eq!(deserialized.llm_calls, trace.llm_calls);
        assert_eq!(deserialized.tool_calls.len(), trace.tool_calls.len());
    }
}

//! Human-readable benchmark reports.

use crate::benchmark::metrics::{compare_runs, RunResult};

/// Format a comparison report between baseline and current run.
pub fn format_report(current: &RunResult, baseline: Option<&RunResult>) -> String {
    let mut out = String::new();

    // Header.
    out.push_str(&format!("Benchmark Run: {}\n", current.run_id));
    if let Some(hash) = &current.commit_hash {
        out.push_str(&format!("Commit: {hash}\n"));
    }
    out.push('\n');

    // Correctness.
    let passed = current.scenarios.iter().filter(|s| s.passed).count();
    let total = current.scenarios.len();
    out.push_str(&format!(
        "Scenarios: {passed}/{total} passed ({:.0}%)\n",
        current.pass_rate * 100.0
    ));

    // Per-scenario results.
    for s in &current.scenarios {
        let status = if s.passed { "PASS" } else { "FAIL" };
        out.push_str(&format!(
            "  {} {} ({}ms, {} tools, {} tokens)\n",
            status,
            s.scenario_id,
            s.trace.wall_time_ms,
            s.trace.total_tool_calls(),
            s.trace.input_tokens + s.trace.output_tokens,
        ));
        if let Some(ref err) = s.error {
            out.push_str(&format!("       {err}\n"));
        }
    }

    // Totals.
    out.push_str(&format!(
        "\nTotal: {:.4} USD, {}ms\n",
        current.total_cost_usd, current.total_wall_time_ms
    ));

    // Baseline comparison.
    if let Some(baseline) = baseline {
        out.push('\n');
        out.push_str("--- Baseline Comparison ---\n");

        let baseline_passed = baseline.scenarios.iter().filter(|s| s.passed).count();
        let baseline_total = baseline.scenarios.len();
        out.push_str(&format!(
            "Pass rate: {passed}/{total} (was {baseline_passed}/{baseline_total})\n"
        ));

        // Detect fixed / regressed scenarios.
        for s in &current.scenarios {
            let baseline_scenario = baseline
                .scenarios
                .iter()
                .find(|b| b.scenario_id == s.scenario_id);
            if let Some(bs) = baseline_scenario {
                if s.passed && !bs.passed {
                    out.push_str(&format!("  + {} FIXED\n", s.scenario_id));
                } else if !s.passed && bs.passed {
                    out.push_str(&format!("  - {} REGRESSED\n", s.scenario_id));
                }
            } else {
                out.push_str(&format!("  ? {} NEW\n", s.scenario_id));
            }
        }

        // Efficiency deltas.
        let deltas = compare_runs(baseline, current, 0.10);
        if !deltas.is_empty() {
            out.push_str("\nEfficiency changes (>10% threshold):\n");
            for d in &deltas {
                let direction = if d.is_regression {
                    "REGRESSED"
                } else {
                    "IMPROVED"
                };
                out.push_str(&format!(
                    "  {} {}: {:.0} -> {:.0} ({:+.0}%) {}\n",
                    d.scenario_id,
                    d.metric,
                    d.baseline,
                    d.current,
                    d.delta * 100.0,
                    direction,
                ));
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::metrics::{RunResult, ScenarioResult, ToolInvocation, TraceMetrics};

    fn sample_trace() -> TraceMetrics {
        TraceMetrics {
            wall_time_ms: 1500,
            llm_calls: 3,
            input_tokens: 500,
            output_tokens: 100,
            estimated_cost_usd: 0.005,
            tool_calls: vec![ToolInvocation {
                name: "echo".to_string(),
                duration_ms: 5,
                success: true,
            }],
            turns: 1,
            hit_iteration_limit: false,
            hit_timeout: false,
        }
    }

    #[test]
    fn test_format_report_no_baseline() {
        let run = RunResult::from_scenarios(
            "test-run",
            vec![ScenarioResult {
                scenario_id: "test-echo".to_string(),
                passed: true,
                trace: sample_trace(),
                response: "hello".to_string(),
                error: None,
            }],
        );
        let report = format_report(&run, None);
        assert!(report.contains("1/1 passed"));
        assert!(report.contains("PASS"));
        assert!(report.contains("test-echo"));
    }

    #[test]
    fn test_format_report_with_baseline_regression() {
        let baseline = RunResult::from_scenarios(
            "baseline",
            vec![ScenarioResult {
                scenario_id: "test-echo".to_string(),
                passed: true,
                trace: sample_trace(),
                response: "hello".to_string(),
                error: None,
            }],
        );
        let mut current_trace = sample_trace();
        current_trace.wall_time_ms = 3000; // 100% slower
        let current = RunResult::from_scenarios(
            "current",
            vec![ScenarioResult {
                scenario_id: "test-echo".to_string(),
                passed: true,
                trace: current_trace,
                response: "hello".to_string(),
                error: None,
            }],
        );
        let report = format_report(&current, Some(&baseline));
        assert!(report.contains("Baseline Comparison"));
        assert!(report.contains("REGRESSED"));
    }
}

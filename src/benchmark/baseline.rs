//! Baseline management: load, save, and promote benchmark results.

use std::path::Path;

use crate::benchmark::metrics::RunResult;

const BASELINE_FILE: &str = "benchmarks/baselines/baseline.json";

/// Load the baseline from the default path.
pub fn load_baseline() -> Result<Option<RunResult>, String> {
    load_baseline_from(BASELINE_FILE)
}

/// Load a baseline from a specific path.
pub fn load_baseline_from(path: &str) -> Result<Option<RunResult>, String> {
    let path = Path::new(path);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read baseline: {e}"))?;
    let run: RunResult =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse baseline: {e}"))?;
    Ok(Some(run))
}

/// Save a run result to the results directory.
pub fn save_result(result: &RunResult) -> Result<String, String> {
    let dir = format!("{}/benchmarks/results", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create results dir: {e}"))?;

    let filename = format!("{}.json", result.run_id);
    let path = format!("{dir}/{filename}");
    let content =
        serde_json::to_string_pretty(result).map_err(|e| format!("Failed to serialize: {e}"))?;
    std::fs::write(&path, content).map_err(|e| format!("Failed to write result: {e}"))?;
    Ok(path)
}

/// Save per-scenario JSON results alongside the run summary.
/// Creates a directory `benchmarks/results/{run_id}/` with:
/// - `summary.json` (the full RunResult)
/// - `{scenario_id}.json` (each ScenarioResult with turn_metrics)
pub fn save_scenario_results(result: &RunResult) -> Result<String, String> {
    let base = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let dir = format!("{base}/benchmarks/results/{}", result.run_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create results dir: {e}"))?;

    // Save summary.
    let summary_path = format!("{dir}/summary.json");
    let summary = serde_json::to_string_pretty(result)
        .map_err(|e| format!("Failed to serialize summary: {e}"))?;
    std::fs::write(&summary_path, summary).map_err(|e| format!("Failed to write summary: {e}"))?;

    // Save per-scenario.
    for scenario in &result.scenarios {
        let scenario_path = format!("{dir}/{}.json", scenario.scenario_id);
        let content = serde_json::to_string_pretty(scenario)
            .map_err(|e| format!("Failed to serialize {}: {e}", scenario.scenario_id))?;
        std::fs::write(&scenario_path, content)
            .map_err(|e| format!("Failed to write {}: {e}", scenario.scenario_id))?;
    }

    Ok(dir)
}

/// Promote a result file to the baseline.
pub fn promote_to_baseline(result_path: &str) -> Result<(), String> {
    let baseline_path = format!("{}/{BASELINE_FILE}", env!("CARGO_MANIFEST_DIR"));
    let baseline_dir = Path::new(&baseline_path)
        .parent()
        .ok_or_else(|| "Invalid baseline path".to_string())?;
    std::fs::create_dir_all(baseline_dir)
        .map_err(|e| format!("Failed to create baselines dir: {e}"))?;
    std::fs::copy(result_path, &baseline_path)
        .map_err(|e| format!("Failed to promote baseline: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::metrics::{ScenarioResult, TraceMetrics};

    #[test]
    fn test_load_nonexistent_baseline() {
        let result = load_baseline_from("/tmp/nonexistent_baseline_ironclaw_12345.json");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_save_scenario_results() {
        let result = RunResult::from_scenarios(
            "test-save-scenarios",
            vec![ScenarioResult {
                scenario_id: "test-a".to_string(),
                passed: true,
                trace: TraceMetrics {
                    wall_time_ms: 100,
                    llm_calls: 1,
                    input_tokens: 10,
                    output_tokens: 5,
                    estimated_cost_usd: 0.001,
                    tool_calls: Vec::new(),
                    turns: 1,
                    hit_iteration_limit: false,
                    hit_timeout: false,
                },
                response: "ok".to_string(),
                error: None,
                turn_metrics: Vec::new(),
            }],
        );
        let dir = save_scenario_results(&result).unwrap();
        // Verify files exist.
        assert!(std::path::Path::new(&format!("{dir}/summary.json")).exists());
        assert!(std::path::Path::new(&format!("{dir}/test-a.json")).exists());
        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }
}

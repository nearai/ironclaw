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

    #[test]
    fn test_load_nonexistent_baseline() {
        let result = load_baseline_from("/tmp/nonexistent_baseline_ironclaw_12345.json");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}

//! Time estimation.

use std::collections::HashMap;
use std::time::Duration;

/// Estimates time for tools and operations.
pub struct TimeEstimator {
    /// Base durations per tool.
    tool_durations: HashMap<String, Duration>,
}

impl TimeEstimator {
    /// Create a new time estimator.
    pub fn new() -> Self {
        let mut tool_durations = HashMap::new();

        // Default tool durations
        tool_durations.insert("http".to_string(), Duration::from_secs(5));
        tool_durations.insert("echo".to_string(), Duration::from_millis(10));
        tool_durations.insert("time".to_string(), Duration::from_millis(1));
        tool_durations.insert("json".to_string(), Duration::from_millis(5));

        Self { tool_durations }
    }

    /// Estimate duration for a tool call.
    pub fn estimate_tool(&self, tool_name: &str) -> Duration {
        self.tool_durations
            .get(tool_name)
            .copied()
            .unwrap_or(Duration::from_secs(5)) // Default for unknown tools
    }

    /// Estimate LLM response time.
    pub fn estimate_llm_response(&self, estimated_tokens: u32) -> Duration {
        // Rough estimate: ~50 tokens/second
        let seconds = estimated_tokens as f64 / 50.0;
        Duration::from_secs_f64(seconds.max(1.0))
    }

    /// Set a tool's base duration.
    pub fn set_tool_duration(&mut self, tool_name: impl Into<String>, duration: Duration) {
        self.tool_durations.insert(tool_name.into(), duration);
    }

    /// Get all tool durations.
    pub fn all_tool_durations(&self) -> &HashMap<String, Duration> {
        &self.tool_durations
    }
}

impl Default for TimeEstimator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_time_estimation() {
        let estimator = TimeEstimator::new();

        assert!(estimator.estimate_tool("echo") < Duration::from_secs(1));
        assert!(estimator.estimate_tool("http") >= Duration::from_secs(1));
    }

    #[test]
    fn test_llm_time_estimation() {
        let estimator = TimeEstimator::new();

        let duration = estimator.estimate_llm_response(500);
        assert!(duration >= Duration::from_secs(1));
    }
}

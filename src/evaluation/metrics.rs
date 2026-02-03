//! Quality metrics tracking.

use std::collections::HashMap;
use std::time::Duration;

use rust_decimal::Decimal;

/// Quality metrics for evaluation.
#[derive(Debug, Clone, Default)]
pub struct QualityMetrics {
    /// Total actions taken.
    pub total_actions: u64,
    /// Successful actions.
    pub successful_actions: u64,
    /// Failed actions.
    pub failed_actions: u64,
    /// Total execution time.
    pub total_time: Duration,
    /// Total cost.
    pub total_cost: Decimal,
    /// Metrics per tool.
    pub tool_metrics: HashMap<String, ToolMetrics>,
    /// Error types encountered.
    pub error_types: HashMap<String, u64>,
}

/// Metrics for a single tool.
#[derive(Debug, Clone, Default)]
pub struct ToolMetrics {
    pub calls: u64,
    pub successes: u64,
    pub failures: u64,
    pub total_time: Duration,
    pub avg_time: Duration,
    pub total_cost: Decimal,
}

impl ToolMetrics {
    /// Calculate success rate.
    pub fn success_rate(&self) -> f64 {
        if self.calls == 0 {
            0.0
        } else {
            self.successes as f64 / self.calls as f64
        }
    }
}

/// Collects and aggregates quality metrics.
pub struct MetricsCollector {
    metrics: QualityMetrics,
}

impl MetricsCollector {
    /// Create a new metrics collector.
    pub fn new() -> Self {
        Self {
            metrics: QualityMetrics::default(),
        }
    }

    /// Record a successful action.
    pub fn record_success(&mut self, tool_name: &str, duration: Duration, cost: Option<Decimal>) {
        self.metrics.total_actions += 1;
        self.metrics.successful_actions += 1;
        self.metrics.total_time += duration;

        if let Some(c) = cost {
            self.metrics.total_cost += c;
        }

        let tool = self
            .metrics
            .tool_metrics
            .entry(tool_name.to_string())
            .or_default();
        tool.calls += 1;
        tool.successes += 1;
        tool.total_time += duration;
        tool.avg_time = tool.total_time / tool.calls as u32;

        if let Some(c) = cost {
            tool.total_cost += c;
        }
    }

    /// Record a failed action.
    pub fn record_failure(&mut self, tool_name: &str, error: &str, duration: Duration) {
        self.metrics.total_actions += 1;
        self.metrics.failed_actions += 1;
        self.metrics.total_time += duration;

        let tool = self
            .metrics
            .tool_metrics
            .entry(tool_name.to_string())
            .or_default();
        tool.calls += 1;
        tool.failures += 1;
        tool.total_time += duration;
        tool.avg_time = tool.total_time / tool.calls as u32;

        // Categorize error
        let error_type = categorize_error(error);
        *self.metrics.error_types.entry(error_type).or_default() += 1;
    }

    /// Get current metrics.
    pub fn metrics(&self) -> &QualityMetrics {
        &self.metrics
    }

    /// Get success rate.
    pub fn success_rate(&self) -> f64 {
        if self.metrics.total_actions == 0 {
            0.0
        } else {
            self.metrics.successful_actions as f64 / self.metrics.total_actions as f64
        }
    }

    /// Get metrics for a specific tool.
    pub fn tool_metrics(&self, tool_name: &str) -> Option<&ToolMetrics> {
        self.metrics.tool_metrics.get(tool_name)
    }

    /// Reset metrics.
    pub fn reset(&mut self) {
        self.metrics = QualityMetrics::default();
    }

    /// Generate a summary report.
    pub fn summary(&self) -> MetricsSummary {
        MetricsSummary {
            total_actions: self.metrics.total_actions,
            success_rate: self.success_rate(),
            total_time: self.metrics.total_time,
            total_cost: self.metrics.total_cost,
            most_used_tool: self
                .metrics
                .tool_metrics
                .iter()
                .max_by_key(|(_, m)| m.calls)
                .map(|(name, _)| name.clone()),
            most_failed_tool: self
                .metrics
                .tool_metrics
                .iter()
                .max_by_key(|(_, m)| m.failures)
                .map(|(name, _)| name.clone()),
            top_errors: self
                .metrics
                .error_types
                .iter()
                .take(3)
                .map(|(e, c)| (e.clone(), *c))
                .collect(),
        }
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary of collected metrics.
#[derive(Debug)]
pub struct MetricsSummary {
    pub total_actions: u64,
    pub success_rate: f64,
    pub total_time: Duration,
    pub total_cost: Decimal,
    pub most_used_tool: Option<String>,
    pub most_failed_tool: Option<String>,
    pub top_errors: Vec<(String, u64)>,
}

/// Categorize an error message into a type.
fn categorize_error(error: &str) -> String {
    let lower = error.to_lowercase();

    if lower.contains("timeout") {
        "timeout".to_string()
    } else if lower.contains("rate limit") {
        "rate_limit".to_string()
    } else if lower.contains("auth") || lower.contains("unauthorized") {
        "auth".to_string()
    } else if lower.contains("not found") || lower.contains("404") {
        "not_found".to_string()
    } else if lower.contains("invalid") || lower.contains("parameter") {
        "invalid_input".to_string()
    } else if lower.contains("network") || lower.contains("connection") {
        "network".to_string()
    } else {
        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_metrics_collection() {
        let mut collector = MetricsCollector::new();

        collector.record_success("tool1", Duration::from_secs(1), Some(dec!(0.01)));
        collector.record_success("tool1", Duration::from_secs(2), Some(dec!(0.02)));
        collector.record_failure("tool2", "timeout error", Duration::from_secs(5));

        assert_eq!(collector.metrics().total_actions, 3);
        assert_eq!(collector.metrics().successful_actions, 2);
        assert_eq!(collector.metrics().failed_actions, 1);

        let tool1 = collector.tool_metrics("tool1").unwrap();
        assert_eq!(tool1.calls, 2);
        assert_eq!(tool1.successes, 2);
    }

    #[test]
    fn test_error_categorization() {
        assert_eq!(categorize_error("Request timeout after 30s"), "timeout");
        assert_eq!(categorize_error("Rate limit exceeded"), "rate_limit");
        assert_eq!(categorize_error("Unauthorized access"), "auth");
    }

    #[test]
    fn test_success_rate() {
        let mut collector = MetricsCollector::new();

        collector.record_success("tool", Duration::from_secs(1), None);
        collector.record_success("tool", Duration::from_secs(1), None);
        collector.record_failure("tool", "error", Duration::from_secs(1));

        let rate = collector.success_rate();
        assert!((rate - 0.666).abs() < 0.01);
    }
}

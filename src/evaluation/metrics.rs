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

    // --- QualityMetrics default ---

    #[test]
    fn test_quality_metrics_default() {
        let m = QualityMetrics::default();
        assert_eq!(m.total_actions, 0);
        assert_eq!(m.successful_actions, 0);
        assert_eq!(m.failed_actions, 0);
        assert_eq!(m.total_time, Duration::ZERO);
        assert_eq!(m.total_cost, Decimal::ZERO);
        assert!(m.tool_metrics.is_empty());
        assert!(m.error_types.is_empty());
    }

    // --- ToolMetrics::success_rate ---

    #[test]
    fn test_tool_metrics_success_rate_zero_calls() {
        let tm = ToolMetrics::default();
        assert_eq!(tm.success_rate(), 0.0);
    }

    #[test]
    fn test_tool_metrics_success_rate_mixed() {
        let tm = ToolMetrics {
            calls: 4,
            successes: 3,
            failures: 1,
            ..Default::default()
        };
        assert!((tm.success_rate() - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn test_tool_metrics_success_rate_all_failures() {
        let tm = ToolMetrics {
            calls: 5,
            successes: 0,
            failures: 5,
            ..Default::default()
        };
        assert_eq!(tm.success_rate(), 0.0);
    }

    // --- MetricsCollector ---

    #[test]
    fn test_collector_default_is_new() {
        let a = MetricsCollector::new();
        let b = MetricsCollector::default();
        assert_eq!(a.metrics().total_actions, b.metrics().total_actions);
        assert_eq!(a.success_rate(), b.success_rate());
    }

    #[test]
    fn test_success_rate_empty_collector() {
        let collector = MetricsCollector::new();
        assert_eq!(collector.success_rate(), 0.0);
    }

    #[test]
    fn test_record_success_accumulates_cost() {
        let mut c = MetricsCollector::new();
        c.record_success("a", Duration::from_millis(100), Some(dec!(1.50)));
        c.record_success("a", Duration::from_millis(200), Some(dec!(2.50)));
        assert_eq!(c.metrics().total_cost, dec!(4.00));
        let tool = c.tool_metrics("a").unwrap();
        assert_eq!(tool.total_cost, dec!(4.00));
    }

    #[test]
    fn test_record_success_none_cost_does_not_change_total() {
        let mut c = MetricsCollector::new();
        c.record_success("x", Duration::from_secs(1), Some(dec!(1.00)));
        c.record_success("x", Duration::from_secs(1), None);
        assert_eq!(c.metrics().total_cost, dec!(1.00));
    }

    #[test]
    fn test_record_failure_does_not_add_cost() {
        let mut c = MetricsCollector::new();
        c.record_failure("t", "oops", Duration::from_secs(1));
        assert_eq!(c.metrics().total_cost, Decimal::ZERO);
    }

    #[test]
    fn test_tool_avg_time_updates() {
        let mut c = MetricsCollector::new();
        c.record_success("t", Duration::from_secs(2), None);
        c.record_success("t", Duration::from_secs(4), None);
        let tool = c.tool_metrics("t").unwrap();
        // total 6s / 2 calls = 3s avg
        assert_eq!(tool.avg_time, Duration::from_secs(3));
    }

    #[test]
    fn test_total_time_across_success_and_failure() {
        let mut c = MetricsCollector::new();
        c.record_success("a", Duration::from_secs(3), None);
        c.record_failure("b", "err", Duration::from_secs(7));
        assert_eq!(c.metrics().total_time, Duration::from_secs(10));
    }

    #[test]
    fn test_tool_metrics_returns_none_for_unknown() {
        let c = MetricsCollector::new();
        assert!(c.tool_metrics("nonexistent").is_none());
    }

    #[test]
    fn test_reset_clears_everything() {
        let mut c = MetricsCollector::new();
        c.record_success("t", Duration::from_secs(1), Some(dec!(5.00)));
        c.record_failure("t", "error", Duration::from_secs(1));
        c.reset();
        assert_eq!(c.metrics().total_actions, 0);
        assert_eq!(c.metrics().successful_actions, 0);
        assert_eq!(c.metrics().failed_actions, 0);
        assert_eq!(c.metrics().total_cost, Decimal::ZERO);
        assert!(c.metrics().tool_metrics.is_empty());
        assert!(c.metrics().error_types.is_empty());
        assert_eq!(c.success_rate(), 0.0);
    }

    #[test]
    fn test_multiple_tools_tracked_independently() {
        let mut c = MetricsCollector::new();
        c.record_success("alpha", Duration::from_secs(1), None);
        c.record_success("alpha", Duration::from_secs(1), None);
        c.record_failure("beta", "bad", Duration::from_secs(1));
        c.record_success("beta", Duration::from_secs(1), None);

        let alpha = c.tool_metrics("alpha").unwrap();
        assert_eq!(alpha.calls, 2);
        assert_eq!(alpha.successes, 2);
        assert_eq!(alpha.failures, 0);

        let beta = c.tool_metrics("beta").unwrap();
        assert_eq!(beta.calls, 2);
        assert_eq!(beta.successes, 1);
        assert_eq!(beta.failures, 1);
    }

    // --- categorize_error ---

    #[test]
    fn test_categorize_error_all_types() {
        assert_eq!(categorize_error("Connection timeout"), "timeout");
        assert_eq!(categorize_error("TIMEOUT exceeded"), "timeout");
        assert_eq!(categorize_error("rate limit hit"), "rate_limit");
        assert_eq!(categorize_error("Rate Limit 429"), "rate_limit");
        assert_eq!(categorize_error("auth failure"), "auth");
        assert_eq!(categorize_error("Unauthorized"), "auth");
        assert_eq!(categorize_error("resource not found"), "not_found");
        assert_eq!(categorize_error("HTTP 404"), "not_found");
        assert_eq!(categorize_error("invalid parameter X"), "invalid_input");
        assert_eq!(categorize_error("bad parameter"), "invalid_input");
        assert_eq!(categorize_error("Invalid JSON"), "invalid_input");
        assert_eq!(categorize_error("network error"), "network");
        assert_eq!(categorize_error("connection refused"), "network");
        assert_eq!(categorize_error("something else entirely"), "unknown");
        assert_eq!(categorize_error(""), "unknown");
    }

    #[test]
    fn test_error_types_accumulated_in_collector() {
        let mut c = MetricsCollector::new();
        c.record_failure("t", "timeout!", Duration::from_secs(1));
        c.record_failure("t", "another timeout", Duration::from_secs(1));
        c.record_failure("t", "auth denied", Duration::from_secs(1));

        assert_eq!(c.metrics().error_types.get("timeout"), Some(&2));
        assert_eq!(c.metrics().error_types.get("auth"), Some(&1));
    }

    // --- MetricsSummary ---

    #[test]
    fn test_summary_empty_collector() {
        let c = MetricsCollector::new();
        let s = c.summary();
        assert_eq!(s.total_actions, 0);
        assert_eq!(s.success_rate, 0.0);
        assert_eq!(s.total_cost, Decimal::ZERO);
        assert!(s.most_used_tool.is_none());
        assert!(s.most_failed_tool.is_none());
        assert!(s.top_errors.is_empty());
    }

    #[test]
    fn test_summary_most_used_and_most_failed() {
        let mut c = MetricsCollector::new();
        // "alpha" gets 3 calls (all success)
        c.record_success("alpha", Duration::from_secs(1), None);
        c.record_success("alpha", Duration::from_secs(1), None);
        c.record_success("alpha", Duration::from_secs(1), None);
        // "beta" gets 2 calls (both failures)
        c.record_failure("beta", "err", Duration::from_secs(1));
        c.record_failure("beta", "err", Duration::from_secs(1));

        let s = c.summary();
        assert_eq!(s.most_used_tool.as_deref(), Some("alpha"));
        assert_eq!(s.most_failed_tool.as_deref(), Some("beta"));
        assert_eq!(s.total_actions, 5);
    }

    #[test]
    fn test_summary_top_errors_populated() {
        let mut c = MetricsCollector::new();
        c.record_failure("t", "timeout", Duration::from_secs(1));
        c.record_failure("t", "auth error", Duration::from_secs(1));
        let s = c.summary();
        assert!(!s.top_errors.is_empty());
        assert!(s.top_errors.len() <= 3);
    }
}

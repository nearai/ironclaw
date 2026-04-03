//! Tool reliability tracking with exponential moving averages.
//!
//! Tracks per-action success rate and latency using EMA (exponential moving
//! average) to smooth out noise. This data can be injected into the context
//! builder to inform the LLM about unreliable tools.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

/// EMA smoothing factor. Higher = more weight on recent observations.
const EMA_ALPHA: f64 = 0.3;

/// Per-action reliability metrics.
#[derive(Debug, Clone)]
pub struct ActionMetrics {
    /// EMA of success rate (0.0 to 1.0).
    pub success_rate: f64,
    /// EMA of latency in milliseconds.
    pub avg_latency_ms: f64,
    /// Total number of calls recorded.
    pub call_count: u64,
    /// Last error message (if any).
    pub last_error: Option<String>,
}

impl Default for ActionMetrics {
    fn default() -> Self {
        Self {
            success_rate: 1.0, // assume success until proven otherwise
            avg_latency_ms: 0.0,
            call_count: 0,
            last_error: None,
        }
    }
}

/// Thread-safe registry of per-action reliability metrics.
#[derive(Clone)]
pub struct ReliabilityTracker {
    metrics: Arc<RwLock<HashMap<String, ActionMetrics>>>,
}

impl ReliabilityTracker {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Record a successful action execution.
    pub async fn record_success(&self, action_name: &str, latency: Duration) {
        let mut metrics = self.metrics.write().await;
        let entry = metrics.entry(action_name.to_string()).or_default();
        entry.call_count += 1;
        let latency_ms = latency.as_millis() as f64;

        if entry.call_count == 1 {
            // First observation — use raw values
            entry.avg_latency_ms = latency_ms;
            // success_rate stays at 1.0
        } else {
            entry.success_rate = ema(entry.success_rate, 1.0);
            entry.avg_latency_ms = ema(entry.avg_latency_ms, latency_ms);
        }
    }

    /// Record a failed action execution.
    pub async fn record_failure(&self, action_name: &str, error: &str) {
        let mut metrics = self.metrics.write().await;
        let entry = metrics.entry(action_name.to_string()).or_default();
        entry.call_count += 1;
        entry.last_error = Some(error.to_string());

        if entry.call_count == 1 {
            entry.success_rate = 0.0;
        } else {
            entry.success_rate = ema(entry.success_rate, 0.0);
        }
    }

    /// Get metrics for a specific action.
    pub async fn get_metrics(&self, action_name: &str) -> Option<ActionMetrics> {
        let metrics = self.metrics.read().await;
        metrics.get(action_name).cloned()
    }

    /// Get all metrics, sorted by success rate (worst first).
    pub async fn all_metrics(&self) -> Vec<(String, ActionMetrics)> {
        let metrics = self.metrics.read().await;
        let mut entries: Vec<(String, ActionMetrics)> = metrics
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        entries.sort_by(|a, b| {
            a.1.success_rate
                .partial_cmp(&b.1.success_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries
    }

    /// Get actions with reliability below a threshold.
    pub async fn unreliable_actions(&self, threshold: f64) -> Vec<(String, ActionMetrics)> {
        let all = self.all_metrics().await;
        all.into_iter()
            .filter(|(_, m)| m.success_rate < threshold)
            .collect()
    }
}

impl Default for ReliabilityTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute exponential moving average.
fn ema(prev: f64, new: f64) -> f64 {
    EMA_ALPHA * new + (1.0 - EMA_ALPHA) * prev
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ema_moves_toward_new() {
        let result = ema(1.0, 0.0);
        // 0.3 * 0.0 + 0.7 * 1.0 = 0.7
        assert!((result - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn ema_converges_on_repeated() {
        let mut val = 1.0;
        for _ in 0..20 {
            val = ema(val, 0.0);
        }
        // Should converge toward 0.0
        assert!(val < 0.01);
    }

    #[tokio::test]
    async fn track_success() {
        let tracker = ReliabilityTracker::new();
        tracker
            .record_success("tool_a", Duration::from_millis(100))
            .await;
        tracker
            .record_success("tool_a", Duration::from_millis(200))
            .await;

        let m = tracker.get_metrics("tool_a").await.unwrap();
        assert_eq!(m.call_count, 2);
        assert!((m.success_rate - 1.0).abs() < f64::EPSILON);
        assert!(m.avg_latency_ms > 100.0); // EMA of 100 and 200
    }

    #[tokio::test]
    async fn track_failure_lowers_success_rate() {
        let tracker = ReliabilityTracker::new();
        tracker
            .record_success("tool_b", Duration::from_millis(50))
            .await;
        tracker.record_failure("tool_b", "not found").await;

        let m = tracker.get_metrics("tool_b").await.unwrap();
        assert_eq!(m.call_count, 2);
        assert!(m.success_rate < 1.0);
        assert_eq!(m.last_error, Some("not found".into()));
    }

    #[tokio::test]
    async fn unreliable_actions_filters() {
        let tracker = ReliabilityTracker::new();
        tracker
            .record_success("good_tool", Duration::from_millis(10))
            .await;
        tracker.record_failure("bad_tool", "always fails").await;

        let unreliable = tracker.unreliable_actions(0.5).await;
        assert_eq!(unreliable.len(), 1);
        assert_eq!(unreliable[0].0, "bad_tool");
    }

    #[tokio::test]
    async fn unknown_action_returns_none() {
        let tracker = ReliabilityTracker::new();
        assert!(tracker.get_metrics("nonexistent").await.is_none());
    }
}

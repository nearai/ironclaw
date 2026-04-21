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

    /// Get actions with reliability below a threshold, requiring at least
    /// `min_calls` observations. Filters out cold-start noise where a single
    /// failure of a brand-new action shouldn't poison its reputation.
    ///
    /// Recommended for system-prompt injection: `min_calls = 10` filters out
    /// tools that haven't been exercised enough to establish a reliability
    /// signal.
    pub async fn unreliable_with_min_calls(
        &self,
        threshold: f64,
        min_calls: u64,
    ) -> Vec<(String, ActionMetrics)> {
        let all = self.all_metrics().await;
        all.into_iter()
            .filter(|(_, m)| m.call_count >= min_calls && m.success_rate < threshold)
            .collect()
    }

    /// Format the top-N unreliable actions as a system-prompt section. Returns
    /// `None` if no actions meet the threshold or if the cap is zero.
    ///
    /// The returned string is intended to be appended to the CodeAct system
    /// prompt so the model sees which tools have been flaky in recent history
    /// and can pick alternatives.
    ///
    /// Default thresholds (tuned in #2800 PR-B): threshold=0.7, min_calls=10,
    /// cap=5. These match the `EMA_ALPHA=0.3` smoothing — by the time the EMA
    /// crosses 0.7 after 10 calls, we've seen a clear signal, not jitter.
    pub async fn format_notes_section(
        &self,
        threshold: f64,
        min_calls: u64,
        cap: usize,
    ) -> Option<String> {
        if cap == 0 {
            return None;
        }
        let unreliable = self.unreliable_with_min_calls(threshold, min_calls).await;
        if unreliable.is_empty() {
            return None;
        }
        let mut lines = vec!["## Action reliability notes".to_string()];
        for (name, metrics) in unreliable.into_iter().take(cap) {
            // Keep each entry short — never exceed ~120 chars — to bound the
            // system-prompt bloat. `last_error` is truncated to the first
            // line and 80 chars.
            let err_hint = metrics
                .last_error
                .as_deref()
                .map(|e| {
                    let first_line = e.lines().next().unwrap_or("").trim();
                    let truncated: String = first_line.chars().take(80).collect();
                    if !truncated.is_empty() {
                        format!(" (last error: {truncated})")
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();
            lines.push(format!(
                "- `{name}` — success rate {:.0}% over {} calls{}",
                metrics.success_rate * 100.0,
                metrics.call_count,
                err_hint,
            ));
        }
        Some(lines.join("\n"))
    }
}

/// `EffectExecutor` decorator that records every action outcome in a
/// `ReliabilityTracker`. Wrap your real effect executor with this at
/// construction so success rate / latency data flows into the tracker
/// automatically — the engine's execution loop doesn't need to know.
///
/// ```rust,ignore
/// let tracker = Arc::new(ReliabilityTracker::new());
/// let real_effects: Arc<dyn EffectExecutor> = Arc::new(MyEffects::new());
/// let recorded: Arc<dyn EffectExecutor> = Arc::new(
///     ReliabilityRecordingEffects::new(real_effects, tracker.clone())
/// );
/// // Pass `recorded` into the engine; `tracker` stays available for reads.
/// ```
pub struct ReliabilityRecordingEffects {
    inner: Arc<dyn crate::traits::effect::EffectExecutor>,
    tracker: Arc<ReliabilityTracker>,
}

impl ReliabilityRecordingEffects {
    pub fn new(
        inner: Arc<dyn crate::traits::effect::EffectExecutor>,
        tracker: Arc<ReliabilityTracker>,
    ) -> Self {
        Self { inner, tracker }
    }
}

#[async_trait::async_trait]
impl crate::traits::effect::EffectExecutor for ReliabilityRecordingEffects {
    async fn execute_action(
        &self,
        name: &str,
        params: serde_json::Value,
        lease: &crate::types::capability::CapabilityLease,
        ctx: &crate::traits::effect::ThreadExecutionContext,
    ) -> Result<crate::types::step::ActionResult, crate::types::error::EngineError> {
        let start = std::time::Instant::now();
        let result = self.inner.execute_action(name, params, lease, ctx).await;
        let elapsed = start.elapsed();
        match &result {
            Ok(action_result) if !action_result.is_error => {
                self.tracker.record_success(name, elapsed).await;
            }
            Ok(action_result) => {
                // Treat a non-panicking but is_error=true ActionResult as a
                // recorded failure. Include a short excerpt of the output for
                // the last_error hint.
                let err_summary = action_result.output.to_string();
                let excerpt: String = err_summary.chars().take(120).collect();
                self.tracker.record_failure(name, &excerpt).await;
            }
            Err(e) => {
                self.tracker.record_failure(name, &e.to_string()).await;
            }
        }
        result
    }

    async fn available_actions(
        &self,
        leases: &[crate::types::capability::CapabilityLease],
    ) -> Result<Vec<crate::types::capability::ActionDef>, crate::types::error::EngineError> {
        self.inner.available_actions(leases).await
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

    /// `unreliable_with_min_calls` must filter out actions below the call
    /// count floor so a single failure on a brand-new tool doesn't surface
    /// in the system prompt. Regression for #2800 PR-B design: threshold
    /// alone was not enough.
    #[tokio::test]
    async fn unreliable_with_min_calls_filters_cold_start() {
        let tracker = ReliabilityTracker::new();
        // Cold-start tool: one failure, below min_calls floor.
        tracker.record_failure("new_tool", "boom").await;

        // Established tool: ten failures, well below threshold.
        for _ in 0..10 {
            tracker.record_failure("old_tool", "fails").await;
        }

        let result = tracker.unreliable_with_min_calls(0.7, 10).await;
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "old_tool");
    }

    #[tokio::test]
    async fn format_notes_section_returns_none_when_empty() {
        let tracker = ReliabilityTracker::new();
        assert!(tracker.format_notes_section(0.7, 10, 5).await.is_none());
    }

    #[tokio::test]
    async fn format_notes_section_returns_none_when_cap_zero() {
        let tracker = ReliabilityTracker::new();
        for _ in 0..10 {
            tracker.record_failure("bad", "err").await;
        }
        // Cap of 0 → no entries surfaced even though unreliable ones exist.
        assert!(tracker.format_notes_section(0.7, 10, 0).await.is_none());
    }

    #[tokio::test]
    async fn format_notes_section_includes_name_rate_and_count() {
        let tracker = ReliabilityTracker::new();
        for _ in 0..10 {
            tracker
                .record_failure("flaky_tool", "network timeout")
                .await;
        }
        let notes = tracker
            .format_notes_section(0.7, 10, 5)
            .await
            .expect("notes section should be produced");
        assert!(notes.contains("## Action reliability notes"));
        assert!(notes.contains("`flaky_tool`"));
        assert!(notes.contains("10 calls"));
        assert!(notes.contains("network timeout"));
    }

    /// The decorator records successful calls in the underlying tracker.
    /// Caller-level test per `.claude/rules/testing.md`: drives through the
    /// trait impl, not `record_success` directly — if the decorator drops
    /// an argument or mis-routes the success path, this catches it.
    #[tokio::test]
    async fn recording_decorator_records_successful_execute_action() {
        use crate::traits::effect::{EffectExecutor, ThreadExecutionContext};
        use crate::types::capability::{CapabilityLease, GrantedActions, LeaseId};
        use crate::types::project::ProjectId;
        use crate::types::step::ActionResult;
        use crate::types::thread::ThreadId;

        struct SuccessfulEffects;

        #[async_trait::async_trait]
        impl EffectExecutor for SuccessfulEffects {
            async fn execute_action(
                &self,
                name: &str,
                _params: serde_json::Value,
                _lease: &CapabilityLease,
                _ctx: &ThreadExecutionContext,
            ) -> Result<ActionResult, crate::types::error::EngineError> {
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: name.to_string(),
                    output: serde_json::json!({"ok": true}),
                    is_error: false,
                    duration: Duration::from_millis(42),
                })
            }
            async fn available_actions(
                &self,
                _leases: &[CapabilityLease],
            ) -> Result<Vec<crate::types::capability::ActionDef>, crate::types::error::EngineError>
            {
                Ok(vec![])
            }
        }

        let tracker = Arc::new(ReliabilityTracker::new());
        let inner: Arc<dyn EffectExecutor> = Arc::new(SuccessfulEffects);
        let decorated: Arc<dyn EffectExecutor> =
            Arc::new(ReliabilityRecordingEffects::new(inner, tracker.clone()));

        let lease = CapabilityLease {
            id: LeaseId::new(),
            thread_id: ThreadId::new(),
            capability_name: "cap".into(),
            granted_actions: GrantedActions::All,
            granted_at: chrono::Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
            revoked_reason: None,
        };
        let ctx = ThreadExecutionContext {
            thread_id: ThreadId::new(),
            thread_type: crate::types::thread::ThreadType::Foreground,
            project_id: ProjectId::new(),
            user_id: "u".into(),
            step_id: crate::types::step::StepId::new(),
            current_call_id: None,
            source_channel: None,
            user_timezone: None,
            thread_goal: None,
        };

        decorated
            .execute_action("ok_tool", serde_json::json!({}), &lease, &ctx)
            .await
            .expect("should succeed");

        let m = tracker.get_metrics("ok_tool").await.unwrap();
        assert_eq!(m.call_count, 1);
        assert!((m.success_rate - 1.0).abs() < f64::EPSILON);
    }

    /// The decorator must record `is_error: true` ActionResults as failures,
    /// not successes. A tool that returns `Ok(ActionResult { is_error: true,
    /// .. })` is a semantic failure even though the Rust result is Ok.
    #[tokio::test]
    async fn recording_decorator_records_is_error_as_failure() {
        use crate::traits::effect::{EffectExecutor, ThreadExecutionContext};
        use crate::types::capability::{CapabilityLease, GrantedActions, LeaseId};
        use crate::types::project::ProjectId;
        use crate::types::step::ActionResult;
        use crate::types::thread::ThreadId;

        struct IsErrorEffects;

        #[async_trait::async_trait]
        impl EffectExecutor for IsErrorEffects {
            async fn execute_action(
                &self,
                name: &str,
                _params: serde_json::Value,
                _lease: &CapabilityLease,
                _ctx: &ThreadExecutionContext,
            ) -> Result<ActionResult, crate::types::error::EngineError> {
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: name.to_string(),
                    output: serde_json::json!({"error": "bad request"}),
                    is_error: true,
                    duration: Duration::from_millis(5),
                })
            }
            async fn available_actions(
                &self,
                _leases: &[CapabilityLease],
            ) -> Result<Vec<crate::types::capability::ActionDef>, crate::types::error::EngineError>
            {
                Ok(vec![])
            }
        }

        let tracker = Arc::new(ReliabilityTracker::new());
        let inner: Arc<dyn EffectExecutor> = Arc::new(IsErrorEffects);
        let decorated: Arc<dyn EffectExecutor> =
            Arc::new(ReliabilityRecordingEffects::new(inner, tracker.clone()));

        let lease = CapabilityLease {
            id: LeaseId::new(),
            thread_id: ThreadId::new(),
            capability_name: "cap".into(),
            granted_actions: GrantedActions::All,
            granted_at: chrono::Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
            revoked_reason: None,
        };
        let ctx = ThreadExecutionContext {
            thread_id: ThreadId::new(),
            thread_type: crate::types::thread::ThreadType::Foreground,
            project_id: ProjectId::new(),
            user_id: "u".into(),
            step_id: crate::types::step::StepId::new(),
            current_call_id: None,
            source_channel: None,
            user_timezone: None,
            thread_goal: None,
        };

        decorated
            .execute_action("bad_tool", serde_json::json!({}), &lease, &ctx)
            .await
            .expect("Ok(is_error=true) is still Ok");

        let m = tracker.get_metrics("bad_tool").await.unwrap();
        assert_eq!(m.call_count, 1);
        assert_eq!(m.success_rate, 0.0);
        assert!(m.last_error.is_some());
    }
}

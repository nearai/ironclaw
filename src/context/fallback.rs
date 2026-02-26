//! Structured fallback deliverables for failed or stuck jobs.
//!
//! When a job fails or is detected as stuck, a [`FallbackDeliverable`] captures
//! what was accomplished before the failure: partial results, action statistics,
//! cost, and timing. This gives users visibility into terminal jobs instead of
//! just an error string.
//!
//! Fallback deliverables are stored in `JobContext.metadata["fallback_deliverable"]`
//! and surfaced through the `job_status` tool.

use serde::{Deserialize, Serialize};

use crate::context::memory::Memory;
use crate::context::state::JobContext;

/// Structured summary of a failed or stuck job.
///
/// Stored in `JobContext.metadata["fallback_deliverable"]` when a job fails
/// or is marked stuck. Surfaced through the `job_status` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackDeliverable {
    /// True if at least one action succeeded before failure.
    pub partial: bool,
    /// Why the job failed.
    pub failure_reason: String,
    /// Last action taken before failure.
    pub last_action: Option<LastAction>,
    /// Aggregate action statistics.
    pub action_stats: ActionStats,
    /// Total tokens consumed.
    pub tokens_used: u64,
    /// Total cost incurred (decimal as string for JSON safety).
    pub cost: String,
    /// Wall-clock elapsed time in seconds.
    pub elapsed_secs: f64,
    /// Number of self-repair attempts.
    pub repair_attempts: u32,
}

/// Summary of the last action taken before failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastAction {
    pub tool_name: String,
    /// Truncated to 200 bytes (UTF-8 safe).
    pub output_preview: String,
    pub success: bool,
}

/// Aggregate action counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionStats {
    pub total: u32,
    pub successful: u32,
    pub failed: u32,
}

impl FallbackDeliverable {
    /// Build a fallback deliverable from a job context and its memory.
    pub fn build(ctx: &JobContext, memory: &Memory, reason: &str) -> Self {
        let successful = memory.successful_actions() as u32;
        let failed = memory.failed_actions() as u32;
        let total = memory.actions.len() as u32;

        let last_action = memory.last_action().map(|a| {
            // Use sanitized output to avoid leaking secrets through the fallback API surface.
            let preview_source = a
                .output_sanitized
                .as_ref()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                })
                .unwrap_or_default();
            let preview = truncate_str(&preview_source, 200);
            LastAction {
                tool_name: a.tool_name.clone(),
                output_preview: preview.to_string(),
                success: a.success,
            }
        });

        let elapsed_secs = ctx.elapsed().map_or(0.0, |d| d.as_secs_f64());

        Self {
            partial: successful > 0,
            failure_reason: truncate_str(reason, 1000).to_string(),
            last_action,
            action_stats: ActionStats {
                total,
                successful,
                failed,
            },
            tokens_used: ctx.total_tokens_used,
            cost: ctx.actual_cost.to_string(),
            elapsed_secs,
            repair_attempts: ctx.repair_attempts,
        }
    }
}

/// Truncate a string to at most `max_len` bytes on a char boundary.
fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    // Find the last char boundary at or before max_len
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::memory::Memory;
    use crate::context::state::JobContext;
    use chrono::{Duration, Utc};
    use rust_decimal::Decimal;
    use std::time::Duration as StdDuration;

    #[test]
    fn test_fallback_zero_actions() {
        let ctx = JobContext::new("Test", "Empty job");
        let memory = Memory::new(ctx.job_id);

        let fb = FallbackDeliverable::build(&ctx, &memory, "timed out");

        assert!(!fb.partial);
        assert_eq!(fb.failure_reason, "timed out");
        assert!(fb.last_action.is_none());
        assert_eq!(fb.action_stats.total, 0);
        assert_eq!(fb.action_stats.successful, 0);
        assert_eq!(fb.action_stats.failed, 0);
        assert_eq!(fb.tokens_used, 0);
        assert_eq!(fb.cost, "0");
        assert_eq!(fb.repair_attempts, 0);
    }

    #[test]
    fn test_fallback_mixed_actions() {
        let mut ctx = JobContext::new("Test", "Mixed job");
        ctx.total_tokens_used = 5000;
        ctx.actual_cost = Decimal::new(42, 2); // 0.42
        ctx.repair_attempts = 1;

        let mut memory = Memory::new(ctx.job_id);

        // 3 successes
        for _ in 0..3 {
            let action = memory
                .create_action("tool_a", serde_json::json!({}))
                .succeed(
                    Some("output".to_string()),
                    serde_json::json!({}),
                    StdDuration::from_secs(1),
                );
            memory.record_action(action);
        }
        // 2 failures
        for _ in 0..2 {
            let action = memory
                .create_action("tool_b", serde_json::json!({}))
                .fail("broke", StdDuration::from_secs(1));
            memory.record_action(action);
        }

        let fb = FallbackDeliverable::build(&ctx, &memory, "max iterations");

        assert!(fb.partial);
        assert_eq!(fb.action_stats.total, 5);
        assert_eq!(fb.action_stats.successful, 3);
        assert_eq!(fb.action_stats.failed, 2);
        assert_eq!(fb.tokens_used, 5000);
        assert_eq!(fb.cost, "0.42");
        assert_eq!(fb.repair_attempts, 1);
        assert!(fb.last_action.is_some());
        let la = fb.last_action.unwrap();
        assert_eq!(la.tool_name, "tool_b");
        assert!(!la.success);
    }

    #[test]
    fn test_fallback_last_action_truncation() {
        let ctx = JobContext::new("Test", "Truncation");
        let mut memory = Memory::new(ctx.job_id);

        let long_output = "x".repeat(500);
        let action = memory
            .create_action("tool_c", serde_json::json!({}))
            .succeed(
                Some(long_output.clone()),
                serde_json::Value::String(long_output),
                StdDuration::from_secs(1),
            );
        memory.record_action(action);

        let fb = FallbackDeliverable::build(&ctx, &memory, "failed");
        let la = fb.last_action.unwrap();
        assert!(la.output_preview.len() <= 200);
        assert!(!la.output_preview.is_empty());
    }

    #[test]
    fn test_fallback_uses_sanitized_output() {
        let ctx = JobContext::new("Test", "Sanitized");
        let mut memory = Memory::new(ctx.job_id);

        let action = memory
            .create_action("tool_d", serde_json::json!({}))
            .succeed(
                Some("[REDACTED]".to_string()),
                serde_json::json!({"api_key": "sk-secret-key-12345"}),
                StdDuration::from_secs(1),
            );
        memory.record_action(action);

        let fb = FallbackDeliverable::build(&ctx, &memory, "failed");
        let la = fb.last_action.unwrap();
        // Must use sanitized output, not raw
        assert!(!la.output_preview.contains("sk-secret"));
        assert!(la.output_preview.contains("REDACTED"));
    }

    #[test]
    fn test_fallback_elapsed_time() {
        let mut ctx = JobContext::new("Test", "Timing");
        let now = Utc::now();
        ctx.started_at = Some(now - Duration::seconds(10));
        ctx.completed_at = Some(now);

        let memory = Memory::new(ctx.job_id);
        let fb = FallbackDeliverable::build(&ctx, &memory, "failed");

        // Should be approximately 10 seconds
        assert!((fb.elapsed_secs - 10.0).abs() < 0.1);
    }

    #[test]
    fn test_fallback_no_started_at() {
        let ctx = JobContext::new("Test", "Never started");
        let memory = Memory::new(ctx.job_id);

        let fb = FallbackDeliverable::build(&ctx, &memory, "failed");
        assert!((fb.elapsed_secs - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_fallback_elapsed_time_no_completed_at() {
        let mut ctx = JobContext::new("Test", "Still running");
        ctx.started_at = Some(Utc::now() - Duration::seconds(5));
        // completed_at is None — should use Utc::now() as fallback

        let memory = Memory::new(ctx.job_id);
        let fb = FallbackDeliverable::build(&ctx, &memory, "stuck");

        // Should be approximately 5 seconds (using now as end time)
        assert!(fb.elapsed_secs >= 4.0 && fb.elapsed_secs <= 7.0);
    }

    #[test]
    fn test_fallback_failure_reason_truncation() {
        let ctx = JobContext::new("Test", "Long reason");
        let memory = Memory::new(ctx.job_id);

        let long_reason = "x".repeat(5000);
        let fb = FallbackDeliverable::build(&ctx, &memory, &long_reason);

        assert!(fb.failure_reason.len() <= 1000);
        assert!(!fb.failure_reason.is_empty());
    }

    #[test]
    fn test_truncate_str_ascii() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 5), "hello");
    }

    #[test]
    fn test_truncate_str_unicode() {
        // "é" is 2 bytes in UTF-8
        let s = "café";
        assert_eq!(truncate_str(s, 10), "café");
        // Truncating at 4 would split "é", should back up to 3
        assert_eq!(truncate_str(s, 4), "caf");
    }

    #[test]
    fn test_fallback_serialization() {
        let ctx = JobContext::new("Test", "Serialize");
        let memory = Memory::new(ctx.job_id);
        let fb = FallbackDeliverable::build(&ctx, &memory, "test error");

        // Should serialize to JSON and back without error
        let json = serde_json::to_value(&fb).unwrap();
        let deserialized: FallbackDeliverable = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.failure_reason, "test error");
    }
}

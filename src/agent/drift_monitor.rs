//! Anti-drift self-checks for the agentic loop.
//!
//! `DriftMonitor` detects common failure patterns during tool execution
//! and produces corrective system messages that delegates inject before
//! the next LLM call. All detection is rule-based — no extra LLM calls.
//!
//! ## Temporal semantics
//!
//! Recording happens in `execute_tool_calls()` / `handle_text_response()`
//! during iteration N; detection runs in `before_llm_call()` at the start
//! of iteration N+1. Corrections are always one iteration behind.
//!
//! ## Silence counting
//!
//! Silence is counted per iteration, not per tool call. If the same
//! iteration also has visible assistant text (`content: Some(text)`),
//! that counts as communication and resets the silence counter.
//!
//! ## Approval-resume limitation
//!
//! In chat mode, approved-tool execution and deferred tools handled in
//! `process_approval()` run outside `ChatDelegate::execute_tool_calls()`
//! and are excluded from drift history. A fresh `ChatDelegate` (and thus
//! a fresh `DriftMonitor`) is created after approval-resume.
//!
//! ## Hashing
//!
//! `hash_arguments` relies on `serde_json::to_string()` producing stable
//! output for semantically identical `serde_json::Value` objects. The
//! current repo does not enable `serde_json`'s `preserve_order` feature,
//! so objects use `BTreeMap` internally. Stability is covered by a unit
//! test; if this assumption breaks, the repetition rule may under-detect.

use std::collections::{HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};

/// Configuration for drift detection thresholds.
///
/// All fields have sensible defaults. The feature can be disabled entirely
/// by setting `enabled` to `false` (env: `IRONCLAW_DRIFT_ENABLED=false`).
#[derive(Debug, Clone)]
pub struct DriftConfig {
    /// Master switch. When false, `check_and_mark` always returns `None`.
    pub enabled: bool,
    /// Number of identical (name, args_hash) tool calls in the window
    /// that triggers a repetition correction. Default: 3.
    pub repetition_threshold: usize,
    /// Sliding window size for repetition detection. Default: 10.
    pub repetition_window: usize,
    /// Number of consecutive failed tool calls that triggers a failure
    /// spiral correction. Default: 4.
    pub failure_spiral_threshold: usize,
    /// Sliding window size for tool cycling detection (A-B-A-B pattern,
    /// cross-iteration only). Default: 6.
    pub cycling_window: usize,
    /// Number of silent iterations (no text communication) that triggers
    /// a silence drift correction. Default: 15.
    pub silence_threshold: usize,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            repetition_threshold: 3,
            repetition_window: 10,
            failure_spiral_threshold: 4,
            cycling_window: 6,
            silence_threshold: 15,
        }
    }
}

/// Which kind of drift was detected. Used for cooldown/suppression tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriftCorrectionKind {
    FailureSpiral,
    Repetition,
    ToolCycling,
    SilenceDrift,
}

/// A detected drift pattern with context for the correction message.
#[derive(Debug, Clone)]
pub enum DriftCorrection {
    FailureSpiral { count: usize, last_tool: String },
    Repetition { tool_name: String, count: usize },
    ToolCycling { tools: (String, String) },
    SilenceDrift { iterations: usize },
}

impl DriftCorrection {
    /// The correction kind, used for suppression tracking.
    pub fn kind(&self) -> DriftCorrectionKind {
        match self {
            Self::FailureSpiral { .. } => DriftCorrectionKind::FailureSpiral,
            Self::Repetition { .. } => DriftCorrectionKind::Repetition,
            Self::ToolCycling { .. } => DriftCorrectionKind::ToolCycling,
            Self::SilenceDrift { .. } => DriftCorrectionKind::SilenceDrift,
        }
    }

    /// Human-readable correction message to inject as a system message.
    pub fn message(&self) -> String {
        match self {
            Self::FailureSpiral { count, last_tool } => format!(
                "The last {count} tool calls have all failed (last: `{last_tool}`). \
                 Stop and analyze the error messages before making more tool calls. \
                 Explain your understanding of the problem."
            ),
            Self::Repetition { tool_name, count } => format!(
                "You have called `{tool_name}` with identical arguments {count} times. \
                 This is not making progress. Try a different approach or explain \
                 what went wrong."
            ),
            Self::ToolCycling { tools: (a, b) } => format!(
                "You are alternating between `{a}` and `{b}` without making progress. \
                 Step back and reconsider your approach."
            ),
            Self::SilenceDrift { iterations } => format!(
                "You have been working for {iterations} iterations without communicating \
                 status to the user. Provide a brief progress update."
            ),
        }
    }
}

/// A recorded tool call for drift pattern analysis.
struct ToolCallRecord {
    name: String,
    arguments_hash: u64,
    succeeded: bool,
    iteration: usize,
}

/// Detects common failure patterns in the agentic loop and produces
/// corrective system messages.
///
/// Each delegate (chat, job, container) owns a `DriftMonitor` behind a
/// `tokio::sync::Mutex`. Recording happens in `execute_tool_calls()` and
/// `handle_text_response()`; detection in `before_llm_call()`.
pub struct DriftMonitor {
    config: DriftConfig,
    history: VecDeque<ToolCallRecord>,
    max_history: usize,
    iterations_since_communication: usize,
    suppressed: HashSet<DriftCorrectionKind>,
}

impl DriftMonitor {
    /// Create a new monitor with the given configuration.
    pub fn new(config: DriftConfig) -> Self {
        let max_history = config
            .repetition_window
            .max(config.cycling_window)
            .max(config.failure_spiral_threshold);
        Self {
            config,
            history: VecDeque::with_capacity(max_history + 1),
            max_history,
            iterations_since_communication: 0,
            suppressed: HashSet::new(),
        }
    }

    /// Create a monitor that never triggers (for testing / opt-out).
    pub fn disabled() -> Self {
        Self::new(DriftConfig {
            enabled: false,
            ..DriftConfig::default()
        })
    }

    /// Record tool calls from the current iteration.
    ///
    /// Called once per iteration (not once per tool). Increments the
    /// silence counter by 1 and clears suppression flags when recovery
    /// events are detected.
    pub fn record_tool_calls(&mut self, calls: &[(String, u64, bool)], iteration: usize) {
        if !self.config.enabled {
            return;
        }

        for (name, args_hash, succeeded) in calls {
            let record = ToolCallRecord {
                name: name.clone(),
                arguments_hash: *args_hash,
                succeeded: *succeeded,
                iteration,
            };

            // Check recovery events before inserting
            if *succeeded {
                self.suppressed.remove(&DriftCorrectionKind::FailureSpiral);
            }

            // Different tool call clears repetition suppression
            if let Some(last) = self.history.back()
                && (last.name != record.name || last.arguments_hash != record.arguments_hash)
            {
                self.suppressed.remove(&DriftCorrectionKind::Repetition);
            }

            // Non-alternating call clears cycling suppression
            if self.history.len() >= 2 {
                let prev = &self.history[self.history.len() - 2];
                if record.name != prev.name {
                    self.suppressed.remove(&DriftCorrectionKind::ToolCycling);
                }
            }

            self.history.push_back(record);
            if self.history.len() > self.max_history {
                self.history.pop_front();
            }
        }

        // Silence: one increment per iteration, not per tool
        self.iterations_since_communication += 1;
    }

    /// Record that visible text was communicated to the user.
    ///
    /// Resets the silence counter and clears silence suppression.
    pub fn record_communication(&mut self) {
        self.iterations_since_communication = 0;
        self.suppressed.remove(&DriftCorrectionKind::SilenceDrift);
    }

    /// Check for drift patterns and mark the detected kind as suppressed.
    ///
    /// Returns `None` if disabled, no pattern detected, or the detected
    /// pattern is currently suppressed (waiting for a recovery event).
    pub fn check_and_mark(&mut self) -> Option<DriftCorrection> {
        if !self.config.enabled {
            return None;
        }

        // Priority order: failure spiral > repetition > cycling > silence
        let detectors: &[fn(&Self) -> Option<DriftCorrection>] = &[
            Self::detect_failure_spiral,
            Self::detect_repetition,
            Self::detect_cycling,
            Self::detect_silence,
        ];

        for detect in detectors {
            if let Some(correction) = detect(self) {
                let kind = correction.kind();
                if !self.suppressed.contains(&kind) {
                    self.suppressed.insert(kind);
                    return Some(correction);
                }
            }
        }

        None
    }

    fn detect_failure_spiral(&self) -> Option<DriftCorrection> {
        let threshold = self.config.failure_spiral_threshold;
        if self.history.len() < threshold {
            return None;
        }

        let tail = self.history.iter().rev().take(threshold);
        let mut last_tool = String::new();
        for record in tail {
            if record.succeeded {
                return None;
            }
            if last_tool.is_empty() {
                last_tool = record.name.clone();
            }
        }

        Some(DriftCorrection::FailureSpiral {
            count: threshold,
            last_tool,
        })
    }

    fn detect_repetition(&self) -> Option<DriftCorrection> {
        let window = self.config.repetition_window.min(self.history.len());
        if window == 0 {
            return None;
        }

        // Count occurrences of each (name, hash) in the window
        let start = self.history.len() - window;
        let mut counts: std::collections::HashMap<(&str, u64), usize> =
            std::collections::HashMap::new();

        for record in self.history.range(start..) {
            let key = (record.name.as_str(), record.arguments_hash);
            *counts.entry(key).or_default() += 1;
        }

        // Pick the tool with the highest repetition count.
        // Ties are broken by lexical order on tool name for determinism
        // (HashMap iteration order is not stable across runs).
        let mut best: Option<(&str, usize)> = None;
        for ((name, _), count) in &counts {
            if *count >= self.config.repetition_threshold {
                let dominated = match best {
                    Some((_, best_count)) if *count < best_count => true,
                    Some((best_name, best_count)) if *count == best_count && *name >= best_name => {
                        true
                    }
                    _ => false,
                };
                if !dominated {
                    best = Some((name, *count));
                }
            }
        }

        best.map(|(name, count)| DriftCorrection::Repetition {
            tool_name: name.to_string(),
            count,
        })
    }

    fn detect_cycling(&self) -> Option<DriftCorrection> {
        let window = self.config.cycling_window;
        if self.history.len() < window {
            return None;
        }

        // Extract cross-iteration tool name sequence (deduplicate within
        // the same iteration to avoid false positives from multi-tool calls)
        let start = self.history.len().saturating_sub(window * 2);
        let mut cross_iter_names: Vec<&str> = Vec::new();
        let mut last_iteration = usize::MAX;

        for record in self.history.range(start..) {
            if record.iteration != last_iteration {
                cross_iter_names.push(&record.name);
                last_iteration = record.iteration;
            }
        }

        // Need at least `window` cross-iteration entries to detect A-B-A-B
        if cross_iter_names.len() < window {
            return None;
        }

        // Check the last `window` entries for strict A-B alternation
        let check = &cross_iter_names[cross_iter_names.len() - window..];
        if check.len() < 2 {
            return None;
        }
        let a = check[0];
        let b = check[1];
        if a == b {
            return None;
        }

        for (i, name) in check.iter().enumerate() {
            let expected = if i % 2 == 0 { a } else { b };
            if *name != expected {
                return None;
            }
        }

        Some(DriftCorrection::ToolCycling {
            tools: (a.to_string(), b.to_string()),
        })
    }

    fn detect_silence(&self) -> Option<DriftCorrection> {
        if self.iterations_since_communication >= self.config.silence_threshold {
            return Some(DriftCorrection::SilenceDrift {
                iterations: self.iterations_since_communication,
            });
        }
        None
    }
}

/// Hash a `serde_json::Value` for deduplication.
///
/// Uses `serde_json::to_string()` which, under the current repo config
/// (no `preserve_order` feature), serializes object keys in sorted order
/// via `BTreeMap`. Stability is covered by `test_hash_stability`.
pub fn hash_arguments(v: &serde_json::Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    // to_string() is cheap and deterministic for our use case
    let s = serde_json::to_string(v).unwrap_or_default();
    s.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> DriftConfig {
        DriftConfig::default()
    }

    fn record(monitor: &mut DriftMonitor, name: &str, hash: u64, succeeded: bool, iter: usize) {
        monitor.record_tool_calls(&[(name.to_string(), hash, succeeded)], iter);
    }

    #[test]
    fn test_repetition_detected() {
        let mut m = DriftMonitor::new(make_config());
        record(&mut m, "search", 42, true, 1);
        record(&mut m, "search", 42, true, 2);
        record(&mut m, "search", 42, true, 3);

        let correction = m.check_and_mark();
        assert!(correction.is_some());
        assert!(matches!(
            correction.unwrap(),
            DriftCorrection::Repetition { count: 3, .. }
        ));
    }

    #[test]
    fn test_repetition_below_threshold() {
        let mut m = DriftMonitor::new(make_config());
        record(&mut m, "search", 42, true, 1);
        record(&mut m, "search", 42, true, 2);

        assert!(m.check_and_mark().is_none());
    }

    #[test]
    fn test_repetition_outside_window() {
        let mut m = DriftMonitor::new(DriftConfig {
            repetition_window: 5,
            ..make_config()
        });
        // Fill window with other calls
        record(&mut m, "search", 42, true, 1);
        for i in 2..=10 {
            record(&mut m, "other", i as u64, true, i);
        }
        record(&mut m, "search", 42, true, 11);
        record(&mut m, "search", 42, true, 12);

        // Only 2 "search" calls in the last 5 entries — below threshold
        assert!(m.check_and_mark().is_none());
    }

    #[test]
    fn test_failure_spiral_detected() {
        let mut m = DriftMonitor::new(make_config());
        record(&mut m, "cmd_a", 1, false, 1);
        record(&mut m, "cmd_b", 2, false, 2);
        record(&mut m, "cmd_c", 3, false, 3);
        record(&mut m, "cmd_d", 4, false, 4);

        let correction = m.check_and_mark();
        assert!(matches!(
            correction,
            Some(DriftCorrection::FailureSpiral { count: 4, .. })
        ));
    }

    #[test]
    fn test_failure_spiral_broken_by_success() {
        let mut m = DriftMonitor::new(make_config());
        record(&mut m, "cmd", 1, false, 1);
        record(&mut m, "cmd", 2, false, 2);
        record(&mut m, "cmd", 3, false, 3);
        record(&mut m, "cmd", 4, true, 4); // success breaks the spiral
        record(&mut m, "cmd", 5, false, 5);

        assert!(m.check_and_mark().is_none());
    }

    #[test]
    fn test_cycling_detected() {
        let mut m = DriftMonitor::new(make_config());
        // Use different argument hashes each time to avoid triggering repetition first
        record(&mut m, "tool_a", 10, true, 1);
        record(&mut m, "tool_b", 20, true, 2);
        record(&mut m, "tool_a", 11, true, 3);
        record(&mut m, "tool_b", 21, true, 4);
        record(&mut m, "tool_a", 12, true, 5);
        record(&mut m, "tool_b", 22, true, 6);

        let correction = m.check_and_mark();
        assert!(matches!(
            correction,
            Some(DriftCorrection::ToolCycling { .. })
        ));
    }

    #[test]
    fn test_cycling_same_iteration_no_trigger() {
        let mut m = DriftMonitor::new(DriftConfig {
            repetition_threshold: 100, // disable repetition for this test
            ..make_config()
        });
        // All in iteration 1 — should not trigger cross-iteration cycling
        m.record_tool_calls(
            &[
                ("tool_a".to_string(), 1, true),
                ("tool_b".to_string(), 2, true),
                ("tool_a".to_string(), 1, true),
                ("tool_b".to_string(), 2, true),
                ("tool_a".to_string(), 1, true),
                ("tool_b".to_string(), 2, true),
            ],
            1,
        );

        assert!(m.check_and_mark().is_none());
    }

    #[test]
    fn test_cycling_three_tools_no_trigger() {
        let mut m = DriftMonitor::new(make_config());
        record(&mut m, "a", 1, true, 1);
        record(&mut m, "b", 2, true, 2);
        record(&mut m, "c", 3, true, 3);
        record(&mut m, "a", 1, true, 4);
        record(&mut m, "b", 2, true, 5);
        record(&mut m, "c", 3, true, 6);

        assert!(m.check_and_mark().is_none());
    }

    #[test]
    fn test_silence_drift_detected() {
        let mut m = DriftMonitor::new(make_config());
        for i in 1..=15 {
            record(&mut m, "tool", i as u64, true, i);
        }

        let correction = m.check_and_mark();
        assert!(matches!(
            correction,
            Some(DriftCorrection::SilenceDrift { iterations: 15 })
        ));
    }

    #[test]
    fn test_silence_reset_on_communication() {
        let mut m = DriftMonitor::new(make_config());
        for i in 1..=14 {
            record(&mut m, "tool", i as u64, true, i);
        }
        m.record_communication();
        record(&mut m, "tool", 15, true, 15);

        assert!(m.check_and_mark().is_none());
    }

    #[test]
    fn test_silence_with_content_net_zero() {
        let mut m = DriftMonitor::new(make_config());
        // Simulate an iteration with tool calls then content-as-communication
        record(&mut m, "tool", 1, true, 1);
        m.record_communication(); // content alongside tool calls
        assert_eq!(m.iterations_since_communication, 0);
    }

    #[test]
    fn test_disabled_monitor_never_triggers() {
        let mut m = DriftMonitor::disabled();
        for i in 1..=20 {
            record(&mut m, "search", 42, false, i);
        }
        assert!(m.check_and_mark().is_none());
    }

    #[test]
    fn test_priority_order() {
        // Set up both failure spiral AND repetition
        let mut m = DriftMonitor::new(DriftConfig {
            failure_spiral_threshold: 3,
            repetition_threshold: 3,
            ..make_config()
        });
        record(&mut m, "cmd", 42, false, 1);
        record(&mut m, "cmd", 42, false, 2);
        record(&mut m, "cmd", 42, false, 3);

        // Should get failure spiral (higher priority) not repetition
        let correction = m.check_and_mark().unwrap();
        assert!(matches!(correction, DriftCorrection::FailureSpiral { .. }));
    }

    #[test]
    fn test_cooldown_suppresses_until_recovery() {
        let mut m = DriftMonitor::new(make_config());
        record(&mut m, "search", 42, true, 1);
        record(&mut m, "search", 42, true, 2);
        record(&mut m, "search", 42, true, 3);

        // First check fires
        assert!(m.check_and_mark().is_some());

        // Same pattern — suppressed
        record(&mut m, "search", 42, true, 4);
        assert!(m.check_and_mark().is_none());

        // Different tool call → recovery event
        record(&mut m, "other_tool", 99, true, 5);
        // Followed by new repetition
        record(&mut m, "search", 42, true, 6);
        record(&mut m, "search", 42, true, 7);
        record(&mut m, "search", 42, true, 8);

        // Should fire again
        assert!(m.check_and_mark().is_some());
    }

    #[test]
    fn test_silence_cooldown_clears_on_communication() {
        let mut m = DriftMonitor::new(DriftConfig {
            silence_threshold: 3,
            ..make_config()
        });

        for i in 1..=3 {
            record(&mut m, "tool", i as u64, true, i);
        }
        assert!(m.check_and_mark().is_some()); // fires

        record(&mut m, "tool", 4, true, 4);
        assert!(m.check_and_mark().is_none()); // suppressed

        m.record_communication(); // recovery

        for i in 5..=7 {
            record(&mut m, "tool", i as u64, true, i);
        }
        assert!(m.check_and_mark().is_some()); // fires again
    }

    #[test]
    fn test_hash_stability() {
        // Same object keys, different insertion order in code
        let v1 = serde_json::json!({"b": 2, "a": 1});
        let v2 = serde_json::json!({"a": 1, "b": 2});
        assert_eq!(hash_arguments(&v1), hash_arguments(&v2));

        // Different values should (almost certainly) differ
        let v3 = serde_json::json!({"a": 1, "b": 3});
        assert_ne!(hash_arguments(&v1), hash_arguments(&v3));
    }

    /// Simulates the chat approval-resume scenario: a fresh DriftMonitor
    /// should not produce false corrections from tools it never saw.
    #[test]
    fn test_fresh_monitor_no_false_correction() {
        // Simulate: original ChatDelegate saw 2 identical tool calls, then
        // approval interrupted the loop. A fresh monitor is created for
        // the resume — it should not inherit the previous history.
        let config = make_config();

        // Original delegate's monitor (would have seen 2 calls)
        let mut original = DriftMonitor::new(config.clone());
        record(&mut original, "search", 42, true, 1);
        record(&mut original, "search", 42, true, 2);
        // Not yet at threshold — no correction
        assert!(original.check_and_mark().is_none());

        // Fresh delegate's monitor (approval-resume creates new ChatDelegate)
        let mut fresh = DriftMonitor::new(config);
        // First call in the fresh monitor — must not trigger
        record(&mut fresh, "search", 42, true, 3);
        assert!(
            fresh.check_and_mark().is_none(),
            "fresh monitor must not trigger from tools it never saw"
        );
    }

    /// When multiple tools exceed the repetition threshold simultaneously,
    /// the one with the highest count is reported (deterministic).
    #[test]
    fn test_repetition_picks_highest_count() {
        let mut m = DriftMonitor::new(DriftConfig {
            repetition_threshold: 2,
            repetition_window: 10,
            failure_spiral_threshold: 100, // disable
            ..make_config()
        });
        // tool_a: 2 occurrences, tool_b: 3 occurrences
        record(&mut m, "tool_a", 1, true, 1);
        record(&mut m, "tool_b", 2, true, 2);
        record(&mut m, "tool_a", 1, true, 3);
        record(&mut m, "tool_b", 2, true, 4);
        record(&mut m, "tool_b", 2, true, 5);

        let correction = m.check_and_mark().unwrap();
        match correction {
            DriftCorrection::Repetition { tool_name, count } => {
                assert_eq!(tool_name, "tool_b");
                assert_eq!(count, 3);
            }
            other => panic!("expected Repetition, got {:?}", other),
        }
    }

    /// When multiple tools tie for the highest repetition count,
    /// the lexically first tool name wins (deterministic tiebreaker).
    #[test]
    fn test_repetition_tie_uses_lexical_order() {
        let mut m = DriftMonitor::new(DriftConfig {
            repetition_threshold: 2,
            repetition_window: 10,
            failure_spiral_threshold: 100, // disable
            ..make_config()
        });
        // Both tools: 2 occurrences each (tied)
        record(&mut m, "zebra", 1, true, 1);
        record(&mut m, "alpha", 2, true, 2);
        record(&mut m, "zebra", 1, true, 3);
        record(&mut m, "alpha", 2, true, 4);

        let correction = m.check_and_mark().unwrap();
        match correction {
            DriftCorrection::Repetition { tool_name, .. } => {
                assert_eq!(tool_name, "alpha", "lexically first should win on tie");
            }
            other => panic!("expected Repetition, got {:?}", other),
        }
    }

    /// Verify that all three delegate paths would produce the same
    /// drift corrections for the same tool call sequence. This tests
    /// the monitor in isolation — the recording API is the same
    /// regardless of which delegate calls it.
    #[test]
    fn test_behavioral_parity_across_delegates() {
        let config = make_config();

        // Simulate identical sequences for chat/job/container
        for label in &["chat", "job", "container"] {
            let mut m = DriftMonitor::new(config.clone());

            // 4 consecutive failures → should trigger failure spiral
            for i in 1..=4 {
                record(&mut m, "http_fetch", i as u64, false, i);
            }

            let correction = m.check_and_mark();
            assert!(
                matches!(
                    correction,
                    Some(DriftCorrection::FailureSpiral { count: 4, .. })
                ),
                "{label}: expected FailureSpiral, got {correction:?}"
            );
        }
    }

    /// Verify that whitespace-only text does NOT count as communication.
    /// All three delegates use trimmed checks before calling
    /// `record_communication()`.
    #[test]
    fn test_whitespace_only_text_not_communication() {
        let mut m = DriftMonitor::new(DriftConfig {
            silence_threshold: 2,
            ..make_config()
        });
        record(&mut m, "tool", 1, true, 1);
        // Simulate whitespace-only text: do NOT call record_communication
        // (delegates check !text.trim().is_empty() before calling it)
        record(&mut m, "tool", 2, true, 2);

        let correction = m.check_and_mark();
        assert!(
            matches!(
                correction,
                Some(DriftCorrection::SilenceDrift { iterations: 2 })
            ),
            "whitespace-only text should not reset silence"
        );
    }

    /// Preflight rejections (hook/policy denials) should NOT be recorded
    /// in drift history. Only actually-executed tool calls count.
    #[test]
    fn test_preflight_rejections_excluded() {
        let mut m = DriftMonitor::new(DriftConfig {
            failure_spiral_threshold: 3,
            ..make_config()
        });
        // Simulate: 2 executed failures + 1 preflight rejection (not recorded)
        record(&mut m, "tool_a", 1, false, 1);
        record(&mut m, "tool_b", 2, false, 2);
        // Preflight rejection would NOT be recorded by the delegate
        // → only 2 failures in history, below threshold of 3

        assert!(
            m.check_and_mark().is_none(),
            "2 executed failures + 1 unrecorded rejection should not trigger spiral"
        );
    }
}

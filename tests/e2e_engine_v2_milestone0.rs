//! Milestone 0 replay/eval suite for Engine V2 quality work.
//!
//! These tests snapshot a small set of representative engine-v2 scenarios using
//! stable quality-oriented metrics so we can measure whether orchestrator-first
//! changes actually improve behavior.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod milestone0 {
    use std::sync::OnceLock;
    use std::time::Duration;

    use serde::Serialize;

    use crate::assert_replay_snapshot;
    use crate::support::metrics::TraceMetrics;
    use crate::support::replay_outcome::ReplayOutcome;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/llm_traces/engine_v2"
    );

    fn engine_v2_test_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    #[derive(Debug, Serialize)]
    struct Milestone0ScenarioOutcome {
        scenario_id: String,
        response_count: usize,
        has_final_response: bool,
        llm_calls: u32,
        tool_calls_total: usize,
        tool_calls_failed: usize,
        max_failed_tool_streak: usize,
        hit_iteration_limit: bool,
        safety_warning_count: usize,
        tool_sequence: Vec<String>,
        engine_threads: Vec<Milestone0ThreadSummary>,
    }

    #[derive(Debug, Serialize)]
    struct Milestone0ThreadSummary {
        final_state: String,
        step_count: usize,
        action_failed_events: usize,
        code_execution_failed_events: usize,
        approval_requested_events: usize,
        issue_categories: Vec<String>,
    }

    impl Milestone0ScenarioOutcome {
        fn from_parts(scenario_id: &str, replay: ReplayOutcome, metrics: TraceMetrics) -> Self {
            let tool_sequence = replay.tool_calls.iter().map(|t| t.name.clone()).collect();
            let engine_threads = replay
                .engine_threads
                .into_iter()
                .map(|thread| {
                    let action_failed_events = thread
                        .event_kinds
                        .iter()
                        .filter(|kind| kind.as_str() == "ActionFailed")
                        .count();
                    let code_execution_failed_events = thread
                        .event_kinds
                        .iter()
                        .filter(|kind| kind.as_str() == "CodeExecutionFailed")
                        .count();
                    let approval_requested_events = thread
                        .event_kinds
                        .iter()
                        .filter(|kind| kind.as_str() == "ApprovalRequested")
                        .count();
                    let issue_categories = thread
                        .issues
                        .into_iter()
                        .map(|issue| issue.category)
                        .collect();

                    Milestone0ThreadSummary {
                        final_state: thread.final_state,
                        step_count: thread.step_count,
                        action_failed_events,
                        code_execution_failed_events,
                        approval_requested_events,
                        issue_categories,
                    }
                })
                .collect();

            Self {
                scenario_id: scenario_id.to_string(),
                response_count: replay.response_count,
                has_final_response: replay.has_final_response,
                llm_calls: metrics.llm_calls,
                tool_calls_total: metrics.total_tool_calls(),
                tool_calls_failed: metrics.failed_tool_calls(),
                max_failed_tool_streak: max_failed_tool_streak(&metrics),
                hit_iteration_limit: metrics.hit_iteration_limit,
                safety_warning_count: replay.safety_warning_count,
                tool_sequence,
                engine_threads,
            }
        }
    }

    fn max_failed_tool_streak(metrics: &TraceMetrics) -> usize {
        let mut max_streak = 0usize;
        let mut current = 0usize;
        for call in &metrics.tool_calls {
            if call.success {
                current = 0;
            } else {
                current += 1;
                max_streak = max_streak.max(current);
            }
        }
        max_streak
    }

    fn requests_contain_substring(
        requests: &[Vec<ironclaw::llm::ChatMessage>],
        needle: &str,
    ) -> bool {
        let needle_lower = needle.to_lowercase();
        requests.iter().any(|request| {
            request
                .iter()
                .any(|msg| msg.content.to_lowercase().contains(&needle_lower))
        })
    }

    async fn snapshot_single_turn_scenario(
        name: &str,
        prompt: &str,
        timeout: Duration,
        expected_request_substring: Option<&str>,
    ) {
        let _guard = engine_v2_test_lock().lock().await;
        let trace = LlmTrace::from_file(format!("{FIXTURES}/{name}.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message(prompt).await;
        let responses = rig.wait_for_responses(1, timeout).await;
        rig.verify_trace_expects(&trace, &responses);

        if let Some(needle) = expected_request_substring {
            let requests = rig.captured_llm_requests();
            assert!(
                requests_contain_substring(&requests, needle),
                "expected captured LLM requests to contain {:?}, got {:#?}",
                needle,
                requests
            );
        }

        let replay = ReplayOutcome::capture(&rig, &responses).await;
        let metrics = rig.collect_metrics().await;
        let outcome = Milestone0ScenarioOutcome::from_parts(name, replay, metrics);
        assert_replay_snapshot!(format!("milestone0_{}", name), outcome);
        rig.shutdown();
    }

    /// Baseline for the simplest deterministic tool-use path.
    #[tokio::test]
    async fn m0_single_tool_echo() {
        snapshot_single_turn_scenario(
            "single_tool_echo",
            "Use the echo tool to repeat: 'V2 echo test'",
            Duration::from_secs(30),
            None,
        )
        .await;
    }

    /// Baseline for a deterministic multi-tool chain.
    #[tokio::test]
    async fn m0_multi_tool_chain() {
        snapshot_single_turn_scenario(
            "multi_tool_chain",
            "Use the echo tool to say 'chain step 1', then check the time.",
            Duration::from_secs(30),
            None,
        )
        .await;
    }

    /// Baseline for recovery after a failed attempt/tool path.
    #[tokio::test]
    async fn m0_tool_error_recovery() {
        snapshot_single_turn_scenario(
            "tool_error_recovery",
            "Parse this json for me: not valid json {",
            Duration::from_secs(30),
            None,
        )
        .await;
    }

    /// Baseline for execution-obligation nudge behavior.
    #[tokio::test]
    async fn m0_execution_obligation_nudge() {
        snapshot_single_turn_scenario(
            "execution_obligation_nudge",
            "run the echo tool with 'obligation echo test'",
            Duration::from_secs(30),
            None,
        )
        .await;
    }

    /// Simple single-tool tasks should get a direct finalization hint after a
    /// successful action instead of drifting into more exploration.
    #[tokio::test]
    async fn m0_simple_task_finalization_hint() {
        snapshot_single_turn_scenario(
            "simple_task_finalization_hint",
            "Use the echo tool to repeat: 'simple finalization success'",
            Duration::from_secs(30),
            Some("already have the tool results you need in context"),
        )
        .await;
    }

    /// Repeating the same successful action/result should trigger a no-new-
    /// evidence cutoff nudge that tells the model to answer from existing
    /// results rather than re-running the same tool.
    #[tokio::test]
    async fn m0_no_new_evidence_cutoff() {
        snapshot_single_turn_scenario(
            "no_new_evidence_cutoff",
            "Use the echo tool to repeat: 'evidence reuse test'",
            Duration::from_secs(30),
            Some("did not add meaningful new evidence"),
        )
        .await;
    }

    /// Identical failing action batches should be cut off quickly: first nudge,
    /// then forced honest completion instead of a long retry loop.
    #[tokio::test]
    async fn m0_repeated_action_error_forced_finalize() {
        snapshot_single_turn_scenario(
            "repeated_action_error_forced_finalize",
            "Parse this json for me: not valid json {",
            Duration::from_secs(30),
            Some("same action error just repeated"),
        )
        .await;
    }
}

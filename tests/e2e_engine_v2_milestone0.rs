//! Milestone 0 replay/eval suite for Engine V2 quality work.
//!
//! These tests snapshot a small set of representative engine-v2 scenarios using
//! stable quality-oriented metrics so we can measure whether orchestrator-first
//! changes actually improve behavior.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod milestone0 {
    use std::collections::HashSet;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use std::time::Duration;

    use serde::Serialize;

    use ironclaw::channels::OutgoingResponse;

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
        turn_count: u32,
        llm_calls: u32,
        tool_calls_total: usize,
        tool_calls_failed: usize,
        duplicate_tool_calls: usize,
        max_failed_tool_streak: usize,
        hit_iteration_limit: bool,
        safety_warning_count: usize,
        saw_finalization_hint: bool,
        saw_no_new_evidence_hint: bool,
        saw_repeated_error_nudge: bool,
        saw_local_execution_bias_hint: bool,
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

    #[derive(Debug, Serialize)]
    struct Milestone0ExpectationCheck {
        expected_request_substrings: Vec<String>,
        missing_request_substrings: Vec<String>,
        forbidden_request_substrings: Vec<String>,
        unexpected_request_substrings: Vec<String>,
    }

    impl Milestone0ExpectationCheck {
        fn passed(&self) -> bool {
            self.missing_request_substrings.is_empty()
                && self.unexpected_request_substrings.is_empty()
        }
    }

    #[derive(Debug, Serialize)]
    struct Milestone0ScenarioReport {
        scenario_id: String,
        passed: bool,
        final_response_preview: Option<String>,
        expectation_check: Milestone0ExpectationCheck,
        outcome: Milestone0ScenarioOutcome,
    }

    impl Milestone0ScenarioOutcome {
        fn from_parts(
            scenario_id: &str,
            replay: ReplayOutcome,
            metrics: TraceMetrics,
            requests: &[Vec<ironclaw::llm::ChatMessage>],
        ) -> Self {
            let tool_sequence: Vec<String> =
                replay.tool_calls.iter().map(|t| t.name.clone()).collect();
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
                turn_count: metrics.turns,
                llm_calls: metrics.llm_calls,
                tool_calls_total: metrics.total_tool_calls(),
                tool_calls_failed: metrics.failed_tool_calls(),
                duplicate_tool_calls: duplicate_tool_calls(&tool_sequence),
                max_failed_tool_streak: max_failed_tool_streak(&metrics),
                hit_iteration_limit: metrics.hit_iteration_limit,
                safety_warning_count: replay.safety_warning_count,
                saw_finalization_hint: requests_contain_substring(
                    requests,
                    "already have the tool results you need in context",
                ),
                saw_no_new_evidence_hint: requests_contain_substring(
                    requests,
                    "did not add meaningful new evidence",
                ),
                saw_repeated_error_nudge: requests_contain_substring(
                    requests,
                    "same action error just repeated",
                ),
                saw_local_execution_bias_hint: requests_contain_substring(
                    requests,
                    "direct local execution path",
                ),
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

    fn duplicate_tool_calls(tool_sequence: &[String]) -> usize {
        let unique = tool_sequence.iter().collect::<HashSet<_>>().len();
        tool_sequence.len().saturating_sub(unique)
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

    fn expectation_check(
        requests: &[Vec<ironclaw::llm::ChatMessage>],
        expected_request_substrings: &[&str],
        forbidden_request_substrings: &[&str],
    ) -> Milestone0ExpectationCheck {
        let missing_request_substrings = expected_request_substrings
            .iter()
            .filter(|needle| !requests_contain_substring(requests, needle))
            .map(|needle| (*needle).to_string())
            .collect();
        let unexpected_request_substrings = forbidden_request_substrings
            .iter()
            .filter(|needle| requests_contain_substring(requests, needle))
            .map(|needle| (*needle).to_string())
            .collect();

        Milestone0ExpectationCheck {
            expected_request_substrings: expected_request_substrings
                .iter()
                .map(|needle| (*needle).to_string())
                .collect(),
            missing_request_substrings,
            forbidden_request_substrings: forbidden_request_substrings
                .iter()
                .map(|needle| (*needle).to_string())
                .collect(),
            unexpected_request_substrings,
        }
    }

    fn report_path() -> Option<PathBuf> {
        std::env::var_os("IRONCLAW_M0_REPORT_JSONL").map(PathBuf::from)
    }

    fn final_response_preview(responses: &[OutgoingResponse]) -> Option<String> {
        responses
            .last()
            .map(|response| truncate_text(&response.content, 180))
    }

    fn truncate_text(value: &str, max_chars: usize) -> String {
        let mut chars = value.chars();
        let truncated: String = chars.by_ref().take(max_chars).collect();
        if chars.next().is_some() {
            format!("{truncated}…")
        } else {
            truncated
        }
    }

    fn append_report_line(path: &PathBuf, report: &Milestone0ScenarioReport) {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap_or_else(|err| panic!("failed to open {:?}: {err}", path));
        serde_json::to_writer(&mut file, report)
            .unwrap_or_else(|err| panic!("failed to serialize report line to {:?}: {err}", path));
        writeln!(&mut file)
            .unwrap_or_else(|err| panic!("failed to write newline to {:?}: {err}", path));
    }

    fn assert_expectation_check(
        scenario_id: &str,
        check: &Milestone0ExpectationCheck,
        requests: &[Vec<ironclaw::llm::ChatMessage>],
    ) {
        assert!(
            check.missing_request_substrings.is_empty(),
            "scenario {} missing request substrings {:?}; captured requests: {:#?}",
            scenario_id,
            check.missing_request_substrings,
            requests,
        );
        assert!(
            check.unexpected_request_substrings.is_empty(),
            "scenario {} unexpectedly contained request substrings {:?}; captured requests: {:#?}",
            scenario_id,
            check.unexpected_request_substrings,
            requests,
        );
    }

    async fn snapshot_single_turn_scenario(
        name: &str,
        prompt: &str,
        timeout: Duration,
        expected_request_substrings: &[&str],
        forbidden_request_substrings: &[&str],
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
        if report_path().is_none() {
            rig.verify_trace_expects(&trace, &responses);
        }

        let requests = rig.captured_llm_requests();
        let check = expectation_check(
            &requests,
            expected_request_substrings,
            forbidden_request_substrings,
        );
        let replay = ReplayOutcome::capture(&rig, &responses).await;
        let metrics = rig.collect_metrics().await;
        let outcome = Milestone0ScenarioOutcome::from_parts(name, replay, metrics, &requests);
        let report = Milestone0ScenarioReport {
            scenario_id: name.to_string(),
            passed: check.passed(),
            final_response_preview: final_response_preview(&responses),
            expectation_check: check,
            outcome,
        };

        if let Some(path) = report_path() {
            append_report_line(&path, &report);
        } else {
            assert_expectation_check(name, &report.expectation_check, &requests);
            assert_replay_snapshot!(format!("milestone0_{}", name), report.outcome);
        }
        rig.shutdown();
    }

    /// Baseline for the simplest deterministic tool-use path.
    #[tokio::test]
    async fn m0_single_tool_echo() {
        snapshot_single_turn_scenario(
            "single_tool_echo",
            "Use the echo tool to repeat: 'V2 echo test'",
            Duration::from_secs(30),
            &[],
            &[],
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
            &[],
            &[],
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
            &[],
            &[],
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
            &[],
            &[],
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
            &["already have the tool results you need in context"],
            &[],
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
            &["did not add meaningful new evidence"],
            &[],
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
            &["same action error just repeated"],
            &[],
        )
        .await;
    }

    /// Repeating a whole successful multi-tool batch should still count as
    /// no-new-evidence and force the model to finalize from prior outputs.
    #[tokio::test]
    async fn m0_multi_tool_no_new_evidence_cutoff() {
        snapshot_single_turn_scenario(
            "multi_tool_no_new_evidence_cutoff",
            "Use the echo tool twice: first say 'alpha evidence', then say 'beta evidence', then answer concisely.",
            Duration::from_secs(30),
            &["did not add meaningful new evidence"],
            &[],
        )
        .await;
    }

    /// After the same action fails twice, the model should switch strategies
    /// instead of hammering the identical failing tool call.
    #[tokio::test]
    async fn m0_repeated_action_error_switches_strategy() {
        snapshot_single_turn_scenario(
            "repeated_action_error_switches_strategy",
            "Parse this json for me: not valid json {",
            Duration::from_secs(30),
            &[
                "same action error just repeated",
                "already have the tool results you need in context",
            ],
            &[],
        )
        .await;
    }

    /// Repo-local execution requests should bias the orchestrator toward local
    /// tools before optional web/search detours.
    #[tokio::test]
    async fn m0_local_execution_bias_repo_scan() {
        snapshot_single_turn_scenario(
            "local_execution_bias_repo_scan",
            "In this repository, run a quick local scan and use the echo tool to say 'local scan ok'. Report the result.",
            Duration::from_secs(30),
            &[
                "direct local execution path",
                "already have the tool results you need in context",
            ],
            &[],
        )
        .await;
    }

    /// A repeated local-only action should still get cut off as no-new-
    /// evidence, not loop forever just because the task is repo-local.
    #[tokio::test]
    async fn m0_local_execution_bias_no_new_evidence_cutoff() {
        snapshot_single_turn_scenario(
            "local_execution_bias_no_new_evidence_cutoff",
            "In this repository, run a quick local scan, use the echo tool to say 'local alpha', repeat that same local check once more, then answer.",
            Duration::from_secs(30),
            &[
                "direct local execution path",
                "did not add meaningful new evidence",
            ],
            &[],
        )
        .await;
    }

    /// Obviously multi-step requests should not get an early finalize-now hint
    /// after the first successful action batch.
    #[tokio::test]
    async fn m0_multi_step_request_avoids_premature_finalization() {
        snapshot_single_turn_scenario(
            "multi_step_request_avoids_premature_finalization",
            "First use the echo tool to say 'phase one complete', then use the echo tool to say 'phase two complete', then answer with both results.",
            Duration::from_secs(30),
            &[],
            &[
                "already have the tool results you need in context",
                "did not add meaningful new evidence",
            ],
        )
        .await;
    }
}

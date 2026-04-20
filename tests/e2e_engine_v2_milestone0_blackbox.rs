//! Black-box replay/eval suite for Engine V2 Milestone 0 quality work.
//!
//! Unlike the white-box M0 suite, this layer does not assert on internal
//! orchestrator marker strings directly. Instead, it scores observable
//! outcomes: completion, response quality, tool-call count, duplicate retries,
//! and repeated-failure streaks.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod milestone0_blackbox {
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
        "/tests/fixtures/llm_traces/engine_v2_blackbox"
    );

    fn engine_v2_test_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    #[derive(Debug, Serialize)]
    struct BlackboxScenarioOutcome {
        scenario_id: String,
        response_count: usize,
        has_final_response: bool,
        turn_count: u32,
        llm_calls: u32,
        tool_calls_total: usize,
        tool_calls_failed: usize,
        duplicate_tool_calls: usize,
        max_failed_tool_streak: usize,
        final_state: String,
        tool_sequence: Vec<String>,
    }

    #[derive(Debug, Serialize)]
    struct BlackboxExpectation<'a> {
        response_contains: &'a [&'a str],
        response_not_contains: &'a [&'a str],
        require_final_response: bool,
        max_llm_calls: u32,
        max_tool_calls: usize,
        max_duplicate_tool_calls: usize,
        max_failed_tool_streak: usize,
        allowed_final_states: &'a [&'a str],
    }

    #[derive(Debug, Serialize)]
    struct BlackboxExpectationCheck {
        missing_response_substrings: Vec<String>,
        unexpected_response_substrings: Vec<String>,
        final_response_missing: bool,
        llm_calls_exceeded_by: Option<u32>,
        tool_calls_exceeded_by: Option<usize>,
        duplicate_tool_calls_exceeded_by: Option<usize>,
        failed_tool_streak_exceeded_by: Option<usize>,
        unexpected_final_state: Option<String>,
    }

    impl BlackboxExpectationCheck {
        fn passed(&self) -> bool {
            self.missing_response_substrings.is_empty()
                && self.unexpected_response_substrings.is_empty()
                && !self.final_response_missing
                && self.llm_calls_exceeded_by.is_none()
                && self.tool_calls_exceeded_by.is_none()
                && self.duplicate_tool_calls_exceeded_by.is_none()
                && self.failed_tool_streak_exceeded_by.is_none()
                && self.unexpected_final_state.is_none()
        }
    }

    #[derive(Debug, Serialize)]
    struct BlackboxScenarioReport<'a> {
        scenario_id: String,
        passed: bool,
        final_response_preview: Option<String>,
        expectation: BlackboxExpectation<'a>,
        check: BlackboxExpectationCheck,
        outcome: BlackboxScenarioOutcome,
    }

    impl BlackboxScenarioOutcome {
        fn from_parts(scenario_id: &str, replay: ReplayOutcome, metrics: TraceMetrics) -> Self {
            let tool_sequence: Vec<String> = replay
                .tool_calls
                .iter()
                .map(|tool| tool.name.clone())
                .collect();
            let final_state = replay
                .engine_threads
                .first()
                .map(|thread| thread.final_state.clone())
                .unwrap_or_else(|| "unknown".to_string());

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
                final_state,
                tool_sequence,
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

    fn joined_response_text(responses: &[OutgoingResponse]) -> String {
        responses
            .iter()
            .map(|response| response.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn evaluate_expectation(
        responses: &[OutgoingResponse],
        outcome: &BlackboxScenarioOutcome,
        expectation: &BlackboxExpectation<'_>,
    ) -> BlackboxExpectationCheck {
        let response_text = joined_response_text(responses);
        let response_lower = response_text.to_lowercase();

        let missing_response_substrings = expectation
            .response_contains
            .iter()
            .filter(|needle| !response_lower.contains(&needle.to_lowercase()))
            .map(|needle| (*needle).to_string())
            .collect();
        let unexpected_response_substrings = expectation
            .response_not_contains
            .iter()
            .filter(|needle| response_lower.contains(&needle.to_lowercase()))
            .map(|needle| (*needle).to_string())
            .collect();

        BlackboxExpectationCheck {
            missing_response_substrings,
            unexpected_response_substrings,
            final_response_missing: expectation.require_final_response
                && !outcome.has_final_response,
            llm_calls_exceeded_by: (outcome.llm_calls > expectation.max_llm_calls)
                .then_some(outcome.llm_calls - expectation.max_llm_calls),
            tool_calls_exceeded_by: (outcome.tool_calls_total > expectation.max_tool_calls)
                .then_some(outcome.tool_calls_total - expectation.max_tool_calls),
            duplicate_tool_calls_exceeded_by: (outcome.duplicate_tool_calls
                > expectation.max_duplicate_tool_calls)
                .then_some(outcome.duplicate_tool_calls - expectation.max_duplicate_tool_calls),
            failed_tool_streak_exceeded_by: (outcome.max_failed_tool_streak
                > expectation.max_failed_tool_streak)
                .then_some(outcome.max_failed_tool_streak - expectation.max_failed_tool_streak),
            unexpected_final_state: if expectation
                .allowed_final_states
                .iter()
                .any(|state| *state == outcome.final_state)
            {
                None
            } else {
                Some(outcome.final_state.clone())
            },
        }
    }

    fn report_path() -> Option<PathBuf> {
        std::env::var_os("IRONCLAW_M0_BLACKBOX_REPORT_JSONL").map(PathBuf::from)
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

    fn append_report_line(path: &PathBuf, report: &BlackboxScenarioReport<'_>) {
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

    fn assert_blackbox_report(report: &BlackboxScenarioReport<'_>, responses: &[OutgoingResponse]) {
        assert!(
            report.check.passed(),
            "black-box scenario {} failed expectation check {:#?}; responses={:#?}",
            report.scenario_id,
            report.check,
            responses,
        );
    }

    async fn evaluate_blackbox_scenario(
        name: &str,
        prompt: &str,
        expectation: BlackboxExpectation<'_>,
        timeout: Duration,
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

        let replay = ReplayOutcome::capture(&rig, &responses).await;
        let metrics = rig.collect_metrics().await;
        let outcome = BlackboxScenarioOutcome::from_parts(name, replay, metrics);
        let check = evaluate_expectation(&responses, &outcome, &expectation);
        let report = BlackboxScenarioReport {
            scenario_id: name.to_string(),
            passed: check.passed(),
            final_response_preview: final_response_preview(&responses),
            expectation,
            check,
            outcome,
        };

        if let Some(path) = report_path() {
            append_report_line(&path, &report);
        } else {
            assert_blackbox_report(&report, &responses);
            assert_replay_snapshot!(format!("milestone0_blackbox_{}", name), report.outcome);
        }
        rig.shutdown();
    }

    /// Simple tasks should finish without an extra exploration tool call.
    #[tokio::test]
    async fn bb_simple_task_finishes_without_extra_exploration() {
        evaluate_blackbox_scenario(
            "simple_task_finishes_without_extra_exploration",
            "Use the echo tool to repeat: 'simple blackbox success'",
            BlackboxExpectation {
                response_contains: &["simple blackbox success"],
                response_not_contains: &[],
                require_final_response: true,
                max_llm_calls: 2,
                max_tool_calls: 1,
                max_duplicate_tool_calls: 0,
                max_failed_tool_streak: 0,
                allowed_final_states: &["Done"],
            },
            Duration::from_secs(30),
        )
        .await;
    }

    /// When a repeated batch adds no new evidence, the agent should stop after
    /// the second duplicate tool call instead of running the same tool again.
    #[tokio::test]
    async fn bb_no_new_evidence_stops_after_second_repeat() {
        evaluate_blackbox_scenario(
            "no_new_evidence_stops_after_second_repeat",
            "First use the echo tool to repeat: 'holdout evidence', then repeat that same tool call once more, then answer concisely.",
            BlackboxExpectation {
                response_contains: &["holdout evidence"],
                response_not_contains: &[],
                require_final_response: true,
                max_llm_calls: 3,
                max_tool_calls: 2,
                max_duplicate_tool_calls: 1,
                max_failed_tool_streak: 0,
                allowed_final_states: &["Done"],
            },
            Duration::from_secs(30),
        )
        .await;
    }

    /// After the same tool error repeats twice, the agent should stop the
    /// retry loop and answer honestly instead of taking a third identical hit.
    #[tokio::test]
    async fn bb_repeated_error_stops_after_two_failures() {
        evaluate_blackbox_scenario(
            "repeated_error_stops_after_two_failures",
            "First try parsing this invalid json once, then retry the same parse once more, then answer honestly: not valid json {",
            BlackboxExpectation {
                response_contains: &["invalid json"],
                response_not_contains: &["temporarily unavailable"],
                require_final_response: true,
                max_llm_calls: 3,
                max_tool_calls: 2,
                max_duplicate_tool_calls: 1,
                max_failed_tool_streak: 2,
                allowed_final_states: &["Done"],
            },
            Duration::from_secs(30),
        )
        .await;
    }

    /// Repo-local requests should take the local path directly rather than a
    /// detour that burns an extra tool call first.
    #[tokio::test]
    async fn bb_local_request_prefers_direct_path() {
        evaluate_blackbox_scenario(
            "local_request_prefers_direct_path",
            "In this repository, do a quick local scan for me and report 'local scan ok'.",
            BlackboxExpectation {
                response_contains: &["local scan ok"],
                response_not_contains: &[],
                require_final_response: true,
                max_llm_calls: 2,
                max_tool_calls: 1,
                max_duplicate_tool_calls: 0,
                max_failed_tool_streak: 0,
                allowed_final_states: &["Done"],
            },
            Duration::from_secs(30),
        )
        .await;
    }

    /// Guardrail: M0 should not prematurely finalize a clearly multi-step task.
    #[tokio::test]
    async fn bb_multi_step_task_still_completes_all_steps() {
        evaluate_blackbox_scenario(
            "multi_step_task_still_completes_all_steps",
            "First use the echo tool to say 'phase one complete', then use the echo tool to say 'phase two complete', then answer with both results.",
            BlackboxExpectation {
                response_contains: &["phase one complete", "phase two complete"],
                response_not_contains: &[],
                require_final_response: true,
                max_llm_calls: 3,
                max_tool_calls: 2,
                max_duplicate_tool_calls: 1,
                max_failed_tool_streak: 0,
                allowed_final_states: &["Done"],
            },
            Duration::from_secs(30),
        )
        .await;
    }

    /// Guardrail: basic tool-error recovery should still work under the new
    /// routing/finalization behavior.
    #[tokio::test]
    async fn bb_basic_tool_error_recovery_still_answers() {
        evaluate_blackbox_scenario(
            "basic_tool_error_recovery_still_answers",
            "Parse this json for me: not valid json {",
            BlackboxExpectation {
                response_contains: &["not valid json", "error"],
                response_not_contains: &["temporarily unavailable"],
                require_final_response: true,
                max_llm_calls: 2,
                max_tool_calls: 1,
                max_duplicate_tool_calls: 0,
                max_failed_tool_streak: 1,
                allowed_final_states: &["Done"],
            },
            Duration::from_secs(30),
        )
        .await;
    }
}

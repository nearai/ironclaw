//! Bug-bash regression snapshots.
//!
//! Each test replays a fixture from `tests/fixtures/llm_traces/bug_bash/`
//! and pins the `ReplayOutcome` as a snapshot. The snapshot encodes the
//! specific regression property the bug report is about — reintroducing
//! the bug causes the snapshot to drift.
//!
//! See `tests/fixtures/llm_traces/bug_bash/README.md` for the bug ↔ fixture
//! map and for instructions on recording new fixtures against staging.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod bug_bash_tests {
    use std::time::Duration;

    use crate::assert_replay_snapshot;
    use crate::support::replay_outcome::ReplayOutcome;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/llm_traces/bug_bash"
    );

    /// Regression for [#2541](https://github.com/near/ironclaw/issues/2541):
    /// agent must invoke a tool (not answer from training data) when the user
    /// asks it to do something. The snapshot pins `tool_calls` to a non-empty
    /// list with the `echo` tool. If the agent regresses to text-only
    /// responses, the snapshot drifts to `tool_calls: []`.
    #[tokio::test]
    async fn snapshot_summarization_uses_tools() {
        let trace =
            LlmTrace::from_file(format!("{FIXTURES}/summarization_uses_tools.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("Echo 'status ok' and then tell me what you heard")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        let outcome = ReplayOutcome::capture(&rig, &responses).await;
        assert_replay_snapshot!("bug_bash_summarization_uses_tools", outcome);
        rig.shutdown();
    }
}

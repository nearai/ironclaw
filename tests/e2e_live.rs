//! Dual-mode E2E tests: live LLM with recording, or replay from saved traces.
//!
//! These tests exercise the full agent loop with real tool execution.
//!
//! # Running
//!
//! **Replay mode** (deterministic, needs committed trace fixture):
//! ```bash
//! cargo test --features libsql --test e2e_live -- --ignored
//! ```
//!
//! **Live mode** (real LLM calls, records/updates trace fixture):
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql --test e2e_live -- --ignored
//! ```
//!
//! See `tests/support/live_harness.rs` for the harness documentation.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod live_tests {
    use std::time::Duration;

    use crate::support::live_harness::LiveTestHarnessBuilder;

    /// Test: ask ironclaw to run zizmor (a GitHub Actions security scanner).
    ///
    /// The agent should figure out how to install and run zizmor, execute it
    /// against the current workspace, and produce a security scan report with
    /// findings categorized by severity.
    ///
    /// In live mode, an LLM judge verifies the response quality.
    /// In replay mode, structural assertions verify the recorded behavior.
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn zizmor_scan() {
        let harness = LiveTestHarnessBuilder::new("zizmor_scan")
            .with_max_tool_iterations(40)
            .with_timeout(Duration::from_secs(300))
            .build()
            .await;

        let user_input = "can we run https://github.com/zizmorcore/zizmor";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig
            .wait_for_responses(1, Duration::from_secs(300))
            .await;

        assert!(!responses.is_empty(), "Expected at least one response");

        // The agent should have used the shell tool to install/run zizmor.
        let tools = rig.tool_calls_started();
        assert!(
            tools.iter().any(|t| t == "shell"),
            "Expected shell tool to be used for running zizmor, got: {tools:?}"
        );

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let joined = text.join("\n").to_lowercase();

        // The response should mention zizmor and contain scan findings.
        assert!(
            joined.contains("zizmor"),
            "Response should mention zizmor: {joined}"
        );

        // LLM judge for semantic verification (live mode only).
        if let Some(verdict) = harness
            .judge(
                &text,
                "The response contains a zizmor security scan report for GitHub Actions \
                 workflows. It lists findings with severity levels (error, warning, etc.). \
                 It mentions specific finding types such as template-injection, artipacked, \
                 excessive-permissions, dangerous-triggers, or similar GitHub Actions \
                 security issues.",
            )
            .await
        {
            assert!(verdict.pass, "LLM judge failed: {}", verdict.reasoning);
        }

        harness.finish(user_input, &text).await;
    }
}

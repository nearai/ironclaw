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

    use crate::support::live_harness::{LiveTestHarness, LiveTestHarnessBuilder};

    const ZIZMOR_JUDGE_CRITERIA: &str = "\
        The response contains a zizmor security scan report for GitHub Actions \
        workflows. It lists findings with severity levels (error, warning, etc.). \
        It mentions specific finding types such as template-injection, artipacked, \
        excessive-permissions, dangerous-triggers, or similar GitHub Actions \
        security issues.";

    /// Shared logic for zizmor scan tests (v1 and v2 engines).
    async fn run_zizmor_scan(harness: LiveTestHarness) {
        let user_input = "can we run https://github.com/zizmorcore/zizmor";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(300)).await;

        assert!(!responses.is_empty(), "Expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();

        // Log diagnostics before asserting.
        eprintln!("[ZizmorScan] Tools used: {tools:?}");
        eprintln!(
            "[ZizmorScan] Response preview: {}",
            text.join("\n").chars().take(500).collect::<String>()
        );

        // The agent should have used the shell tool to install/run zizmor.
        assert!(
            tools.iter().any(|t| t == "shell"),
            "Expected shell tool to be used for running zizmor, got: {tools:?}"
        );

        let joined = text.join("\n").to_lowercase();

        // The response should mention zizmor and contain scan findings.
        assert!(
            joined.contains("zizmor"),
            "Response should mention zizmor: {joined}"
        );

        // LLM judge for semantic verification (live mode only).
        if let Some(verdict) = harness.judge(&text, ZIZMOR_JUDGE_CRITERIA).await {
            assert!(verdict.pass, "LLM judge failed: {}", verdict.reasoning);
        }

        harness.finish(user_input, &text).await;
    }

    /// Zizmor scan via engine v1 (default agentic loop).
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn zizmor_scan() {
        let harness = LiveTestHarnessBuilder::new("zizmor_scan")
            .with_max_tool_iterations(40)
            .with_auto_approve_tools(true)
            .build()
            .await;

        run_zizmor_scan(harness).await;
    }

    /// Zizmor scan via engine v2.
    ///
    /// NOTE: Engine v2 does not yet honor `auto_approve_tools` from config —
    /// it only checks the per-session "always" set. This means tool calls
    /// that require approval (shell, file_write, etc.) will be paused.
    /// The test currently validates that v2 at least attempts the task and
    /// mentions zizmor in its response (even if it can't execute shell).
    /// When v2 gains auto-approve support, update this to use `run_zizmor_scan`.
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn zizmor_scan_v2() {
        let harness = LiveTestHarnessBuilder::new("zizmor_scan_v2")
            .with_engine_v2(true)
            .with_max_tool_iterations(40)
            .build()
            .await;

        let user_input = "can we run https://github.com/zizmorcore/zizmor";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(300)).await;

        assert!(!responses.is_empty(), "Expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();

        eprintln!("[ZizmorScanV2] Tools used: {tools:?}");
        eprintln!(
            "[ZizmorScanV2] Response preview: {}",
            text.join("\n").chars().take(500).collect::<String>()
        );

        let joined = text.join("\n").to_lowercase();

        // V2 without auto-approve hits an approval gate for shell/tool_install.
        // The response may be the approval prompt itself rather than agent output.
        // Verify the agent at least attempted a relevant action.
        let attempted_relevant_tool = tools.iter().any(|t| {
            t == "shell"
                || t == "tool_install"
                || t.starts_with("tool_search")
                || t.starts_with("skill_search")
        });
        assert!(
            attempted_relevant_tool,
            "Expected agent to attempt a relevant tool, got: {tools:?}"
        );

        // The response should mention zizmor or approval (approval gate).
        assert!(
            joined.contains("zizmor") || joined.contains("approval"),
            "Response should mention zizmor or approval: {joined}"
        );

        harness.finish(user_input, &text).await;
    }
}

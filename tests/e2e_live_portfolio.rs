//! Live E2E tests for the portfolio WASM tool.
//!
//! These tests exercise the full agent loop with a real LLM, real WASM
//! sandbox (fuel metering), and real HTTP to FastNEAR/Intear APIs.
//!
//! # Running
//!
//! **Live mode** (real LLM + real APIs, records trace fixture):
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql --test e2e_live_portfolio -- --ignored --test-threads=1
//! ```
//!
//! **Note:** `--test-threads=1` is required. Engine v2 has a thread-registry
//! race when two `LiveTestHarness` instances run in parallel (engine v2 join
//! error: thread not found). Serial execution avoids the race. Each test
//! individually passes.
//!
//! **Replay mode** (deterministic, uses committed trace fixture):
//! ```bash
//! cargo test --features libsql --test e2e_live_portfolio -- --ignored
//! ```
//!
//! Requires the portfolio WASM to be built:
//! ```bash
//! cargo component build -p portfolio-tool --target wasm32-wasip2 --release
//! ```

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod live_tests {
    use std::time::Duration;

    use crate::support::live_harness::LiveTestHarnessBuilder;

    const PORTFOLIO_JUDGE_CRITERIA: &str = "\
        The response contains a portfolio scan result for a NEAR wallet. \
        It should list token holdings with symbols and USD values. \
        It should mention NEAR as one of the holdings. \
        The response should NOT contain error messages about fuel exhaustion \
        or missing credentials.";


    /// Scan root.near and verify the agent returns portfolio positions.
    ///
    /// This is the critical end-to-end test that caught multiple bugs:
    /// - Fuel exhaustion (10M limit vs 500M needed)
    /// - Stale WASM binary not picked up
    /// - DB-persisted fuel limit overriding code default
    /// - Intear /tokens endpoint (3.2MB) vs /list-token-price (235KB)
    /// - Wallet positions silently dropped by analyzer
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys + network access
    async fn portfolio_scan_root_near() {
        use crate::support::live_harness::TestMode;

        let harness = LiveTestHarnessBuilder::new("portfolio_scan_root_near")
            .with_max_tool_iterations(20)
            .with_auto_approve_tools(true)
            .with_engine_v2(true)
            .with_no_trace_recording()
            .build()
            .await;

        if harness.mode() == TestMode::Replay {
            eprintln!(
                "[PortfolioScan] Live-only test — skipping outside IRONCLAW_LIVE_TEST=1. \
                 WASM tool HTTP calls can't be replayed from LLM traces. \
                 Deterministic coverage lives in e2e_wasm_portfolio.rs."
            );
            return;
        }

        let user_input =
            "Scan the NEAR wallet root.near and show me the token holdings with USD values";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(180)).await;
        assert!(!responses.is_empty(), "Expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();
        let completed = rig.tool_calls_completed();

        eprintln!("[PortfolioScan] Tools used: {tools:?}");
        eprintln!(
            "[PortfolioScan] Response preview: {}",
            text.join("\n").chars().take(500).collect::<String>()
        );

        // The agent must have called the portfolio tool
        assert!(
            tools.iter().any(|t| t.starts_with("portfolio")),
            "Expected portfolio tool to be used, got: {tools:?}"
        );

        // The portfolio tool call must have succeeded (not fuel-exhausted)
        let portfolio_succeeded = completed
            .iter()
            .any(|(name, success)| (name.starts_with("portfolio")) && *success);
        assert!(
            portfolio_succeeded,
            "Portfolio tool should succeed (check fuel limits). Completed: {completed:?}"
        );

        let joined = text.join("\n").to_lowercase();

        // Response should mention NEAR holdings
        assert!(
            joined.contains("near"),
            "Response should mention NEAR: {joined}"
        );

        // Response should NOT contain fuel/error messages
        assert!(
            !joined.contains("fuel exhausted") && !joined.contains("fuel limit"),
            "Response should not mention fuel errors: {joined}"
        );

        // LLM judge for semantic verification (live mode only)
        if let Some(verdict) = harness.judge(&text, PORTFOLIO_JUDGE_CRITERIA).await {
            assert!(verdict.pass, "LLM judge failed: {}", verdict.reasoning);
        }

        harness.finish(user_input, &text).await;
    }

    /// Scan root.near and propose yield strategies.
    ///
    /// Tests the full pipeline: scan → propose. For root.near (wallet
    /// holdings only), propose should return 0 ready proposals since
    /// wallet positions don't match the stablecoin-yield-floor strategy.
    /// The test verifies the pipeline completes without errors.
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys + network access
    async fn portfolio_scan_and_propose_root_near() {
        use crate::support::live_harness::TestMode;

        let harness = LiveTestHarnessBuilder::new("portfolio_scan_and_propose_root_near")
            .with_max_tool_iterations(30)
            .with_auto_approve_tools(true)
            .with_engine_v2(true)
            .with_no_trace_recording()
            .build()
            .await;

        if harness.mode() == TestMode::Replay {
            eprintln!(
                "[PortfolioPipeline] Live-only test — skipping outside IRONCLAW_LIVE_TEST=1. \
                 Deterministic coverage lives in e2e_wasm_portfolio.rs."
            );
            return;
        }

        let user_input = "Scan root.near and check if there are any yield optimization opportunities. \
                          Use source=auto for the scan, then run propose with the results.";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(300)).await;
        assert!(!responses.is_empty(), "Expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();
        let completed = rig.tool_calls_completed();

        eprintln!("[PortfolioPipeline] Tools used ({}):", tools.len());
        for t in &tools {
            eprintln!("  - {t}");
        }
        eprintln!(
            "[PortfolioPipeline] Response preview: {}",
            text.join("\n").chars().take(800).collect::<String>()
        );

        // The agent must have called portfolio at least once (scan)
        let portfolio_calls: Vec<_> = tools.iter().filter(|t| t.starts_with("portfolio")).collect();
        assert!(
            !portfolio_calls.is_empty(),
            "Expected at least one portfolio tool call, got: {tools:?}"
        );

        // At least one portfolio call must have succeeded (the scan).
        // The LLM may retry propose with wrong args, so we don't require
        // all calls to succeed — just that the scan worked.
        let portfolio_results: Vec<_> = completed
            .iter()
            .filter(|(name, _)| name.starts_with("portfolio"))
            .collect();
        let any_succeeded = portfolio_results.iter().any(|(_, success)| *success);
        assert!(
            any_succeeded,
            "At least one portfolio call should succeed. Results: {portfolio_results:?}"
        );

        let joined = text.join("\n").to_lowercase();

        // Should not have fuel errors
        assert!(
            !joined.contains("fuel exhausted") && !joined.contains("fuel limit"),
            "Response should not mention fuel errors"
        );

        // Should mention portfolio/positions/holdings
        assert!(
            joined.contains("near") || joined.contains("portfolio") || joined.contains("position"),
            "Response should mention portfolio results: {joined}"
        );

        harness.finish(user_input, &text).await;
    }
}

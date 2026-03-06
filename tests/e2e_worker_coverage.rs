//! E2E trace tests: worker execution paths (#571).
//!
//! Covers parallel tool calls, error feedback loops, unknown tools,
//! invalid parameters, rate limiting, iteration limits, and planning mode.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use serde_json::json;

    use ironclaw::context::JobContext;
    use ironclaw::tools::{Tool, ToolError, ToolOutput};

    use crate::support::cleanup::CleanupGuard;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    // -- Stub tools for rate-limit and timeout tests --------------------------

    /// A tool that always returns RateLimited.
    struct StubRateLimitTool;

    #[async_trait]
    impl Tool for StubRateLimitTool {
        fn name(&self) -> &str {
            "stub_rate_limit"
        }
        fn description(&self) -> &str {
            "Always returns rate limited error"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            json!({ "type": "object", "properties": {} })
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Err(ToolError::RateLimited(Some(Duration::from_secs(60))))
        }
    }

    // -----------------------------------------------------------------------
    // Test 1: parallel_three_tools
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn parallel_three_tools() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/worker/parallel_three_tools.json"
        ))
        .expect("failed to load parallel_three_tools.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Run three tools in parallel").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Verify all three tools were started.
        let started = rig.tool_calls_started();
        assert!(
            started.contains(&"echo".to_string()),
            "echo not started: {started:?}"
        );
        assert!(
            started.contains(&"time".to_string()),
            "time not started: {started:?}"
        );
        assert!(
            started.contains(&"json".to_string()),
            "json not started: {started:?}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 2: tool_error_feedback
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn tool_error_feedback() {
        let test_dir = "/tmp/ironclaw_error_feedback_test";
        let _ = std::fs::remove_dir_all(test_dir);
        std::fs::create_dir_all(test_dir).expect("create test dir");
        let _cleanup = CleanupGuard::new().dir(test_dir);

        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/worker/tool_error_feedback.json"
        ))
        .expect("failed to load tool_error_feedback.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Write a file to a bad path then recover")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Verify the recovery file exists.
        let content = std::fs::read_to_string(format!("{test_dir}/recovered.txt"))
            .expect("recovered.txt should exist");
        assert!(
            content.contains("recovered"),
            "Expected 'recovered' in file, got: {content:?}"
        );

        // At least one tool call should have failed (the bad path).
        let completed = rig.tool_calls_completed();
        let failures: Vec<_> = completed.iter().filter(|(_, ok)| !ok).collect();
        assert!(
            !failures.is_empty(),
            "Expected at least one failed tool call, got: {completed:?}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 3: unknown_tool_name
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn unknown_tool_name() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/worker/unknown_tool.json"
        ))
        .expect("failed to load unknown_tool.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Deploy to production").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // The deploy_to_production tool should have been attempted but failed.
        let completed = rig.tool_calls_completed();
        let deploy_results: Vec<_> = completed
            .iter()
            .filter(|(name, _)| name == "deploy_to_production")
            .collect();
        assert!(
            !deploy_results.is_empty(),
            "deploy_to_production should have been attempted: {completed:?}"
        );
        assert!(
            deploy_results.iter().all(|(_, ok)| !ok),
            "deploy_to_production should fail: {deploy_results:?}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 4: invalid_tool_params
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn invalid_tool_params() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/worker/invalid_params.json"
        ))
        .expect("failed to load invalid_params.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Echo something with wrong params first")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Echo should have been called at least twice (bad then good).
        let started = rig.tool_calls_started();
        let echo_count = started.iter().filter(|n| n.as_str() == "echo").count();
        assert!(
            echo_count >= 2,
            "Expected >= 2 echo calls, got {echo_count}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 5: rate_limit_cascade
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rate_limit_cascade() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/worker/rate_limit_cascade.json"
        ))
        .expect("failed to load rate_limit_cascade.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_extra_tools(vec![Arc::new(StubRateLimitTool) as Arc<dyn Tool>])
            .build()
            .await;

        rig.send_message("Call the rate limited tool").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Both calls should have failed due to rate limiting.
        let completed = rig.tool_calls_completed();
        let rl_calls: Vec<_> = completed
            .iter()
            .filter(|(name, _)| name == "stub_rate_limit")
            .collect();
        assert!(
            !rl_calls.is_empty(),
            "Expected stub_rate_limit calls: {completed:?}"
        );
        assert!(
            rl_calls.iter().all(|(_, ok)| !ok),
            "All stub_rate_limit calls should fail: {rl_calls:?}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 6: worker_timeout (iteration limit)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn worker_timeout() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/worker/worker_timeout.json"
        ))
        .expect("failed to load worker_timeout.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_max_tool_iterations(2)
            .build()
            .await;

        rig.send_message("Keep calling tools until the limit").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        // We should still get a response even with iteration limit.
        assert!(
            !responses.is_empty(),
            "Expected at least one response with iteration limit"
        );

        // Metrics should show we hit the iteration limit.
        let metrics = rig.collect_metrics().await;
        assert!(
            metrics.tool_calls.len() <= 2,
            "Expected at most 2 tool calls with limit=2, got {}",
            metrics.tool_calls.len()
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 7: post_plan_work_remaining
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn post_plan_work_remaining() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/worker/plan_remaining_work.json"
        ))
        .expect("failed to load plan_remaining_work.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Plan and execute a task").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Verify echo was called during execution.
        let started = rig.tool_calls_started();
        assert!(
            started.contains(&"echo".to_string()),
            "echo should be called: {started:?}"
        );

        rig.shutdown();
    }
}

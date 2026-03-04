//! E2E test: validates that the metrics collection layer works.
//!
//! Exercises `TraceMetrics`, `ScenarioResult`, `RunResult`, and `compare_runs`
//! through actual agent execution via the TestRig.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use ironclaw::tools::ToolRegistry;

    use crate::support::assertions::assert_all_tools_succeeded;
    use crate::support::cleanup::CleanupGuard;
    use crate::support::metrics::{RunResult, ScenarioResult, compare_runs};
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    const TEST_DIR: &str = "/tmp/ironclaw_metrics_test";

    fn setup_test_dir() {
        let _ = std::fs::remove_dir_all(TEST_DIR);
        std::fs::create_dir_all(TEST_DIR).expect("failed to create test directory");
    }

    fn tools_with_file_support() -> Arc<ToolRegistry> {
        let registry = Arc::new(ToolRegistry::new());
        registry.register_builtin_tools();
        registry.register_dev_tools();
        registry
    }

    /// Verify that metrics are collected from a simple text-only trace.
    #[tokio::test]
    async fn test_metrics_collected_from_text_trace() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/simple_text.json"
        ))
        .expect("failed to load simple_text.json");

        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("hello").await;
        let _responses = rig.wait_for_responses(1, Duration::from_secs(10)).await;

        // Collect metrics.
        let metrics = rig.collect_metrics().await;

        // Should have made at least 1 LLM call.
        assert!(
            metrics.llm_calls >= 1,
            "Expected >= 1 LLM call, got {}",
            metrics.llm_calls
        );

        // Token counts should match the fixture (50 input, 10 output).
        assert!(
            metrics.input_tokens >= 50,
            "Expected >= 50 input tokens, got {}",
            metrics.input_tokens
        );
        assert!(
            metrics.output_tokens >= 10,
            "Expected >= 10 output tokens, got {}",
            metrics.output_tokens
        );

        // Wall time should be > 0 (we waited for a response).
        assert!(
            metrics.wall_time_ms > 0,
            "Expected wall_time_ms > 0, got {}",
            metrics.wall_time_ms
        );

        // No tools in this trace.
        assert!(
            metrics.tool_calls.is_empty(),
            "Expected no tool calls, got {:?}",
            metrics.tool_calls
        );

        // Should have at least 1 turn.
        assert!(
            metrics.turns >= 1,
            "Expected >= 1 turn, got {}",
            metrics.turns
        );

        rig.shutdown();
    }

    /// Verify that metrics capture tool calls from a file write/read flow.
    #[tokio::test]
    async fn test_metrics_collected_from_tool_trace() {
        setup_test_dir();
        let _cleanup = CleanupGuard::new().dir(TEST_DIR);

        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/file_write_read.json"
        ))
        .expect("failed to load file_write_read.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .with_tools(tools_with_file_support())
            .build()
            .await;

        rig.send_message("Please write a greeting to a file and read it back.")
            .await;
        let _responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        // Assert all tools completed successfully.
        let completed = rig.tool_calls_completed();
        assert_all_tools_succeeded(&completed);

        let metrics = rig.collect_metrics().await;

        // Should have made 3 LLM calls (write_file, read_file, final text).
        assert!(
            metrics.llm_calls >= 3,
            "Expected >= 3 LLM calls, got {}",
            metrics.llm_calls
        );

        // Token counts should be non-trivial.
        assert!(metrics.input_tokens > 0, "Expected input_tokens > 0");
        assert!(metrics.output_tokens > 0, "Expected output_tokens > 0");

        // Should have captured tool invocations.
        assert!(
            metrics.total_tool_calls() >= 2,
            "Expected >= 2 tool calls, got {}",
            metrics.total_tool_calls()
        );

        // Both tools should have succeeded.
        assert_eq!(
            metrics.failed_tool_calls(),
            0,
            "Expected 0 failed tool calls"
        );

        // Verify specific tool names.
        let tool_names: Vec<&str> = metrics.tool_calls.iter().map(|t| t.name.as_str()).collect();
        assert!(
            tool_names.contains(&"write_file"),
            "Expected write_file in tool calls, got {:?}",
            tool_names
        );
        assert!(
            tool_names.contains(&"read_file"),
            "Expected read_file in tool calls, got {:?}",
            tool_names
        );

        rig.shutdown();
    }

    /// Verify that metrics serialize to JSON correctly (for CI consumption).
    #[tokio::test]
    async fn test_metrics_json_serialization() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/simple_text.json"
        ))
        .expect("failed to load simple_text.json");

        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("hello").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(10)).await;

        let metrics = rig.collect_metrics().await;

        // Build a ScenarioResult.
        let scenario = ScenarioResult {
            scenario_id: "test_metrics_json_serialization".to_string(),
            passed: true,
            trace: metrics,
            response: responses
                .first()
                .map(|r| r.content.clone())
                .unwrap_or_default(),
            error: None,
        };

        // Should serialize to valid JSON.
        let json = serde_json::to_string_pretty(&scenario).expect("JSON serialization failed");
        assert!(json.contains("\"scenario_id\""));
        assert!(json.contains("\"wall_time_ms\""));
        assert!(json.contains("\"llm_calls\""));
        assert!(json.contains("\"input_tokens\""));
        assert!(json.contains("\"output_tokens\""));

        // Should deserialize back.
        let deserialized: ScenarioResult =
            serde_json::from_str(&json).expect("JSON deserialization failed");
        assert_eq!(deserialized.scenario_id, scenario.scenario_id);
        assert_eq!(deserialized.passed, scenario.passed);

        rig.shutdown();
    }

    /// Verify RunResult aggregation and baseline comparison.
    #[tokio::test]
    async fn test_run_result_and_baseline_comparison() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/simple_text.json"
        ))
        .expect("failed to load simple_text.json");

        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("hello").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(10)).await;

        let metrics = rig.collect_metrics().await;

        // Create a "current" run result.
        let current_scenario = ScenarioResult {
            scenario_id: "smoke_test".to_string(),
            passed: true,
            trace: metrics,
            response: responses
                .first()
                .map(|r| r.content.clone())
                .unwrap_or_default(),
            error: None,
        };
        let current_run = RunResult::from_scenarios("current-run", vec![current_scenario]);

        // Verify aggregation.
        assert_eq!(current_run.pass_rate, 1.0);
        assert_eq!(current_run.scenarios.len(), 1);
        assert!(current_run.total_wall_time_ms > 0);

        // Create a synthetic "baseline" with double the tokens (simulating regression).
        let mut baseline_trace = current_run.scenarios[0].trace.clone();
        baseline_trace.input_tokens /= 2; // Baseline had fewer tokens.
        let baseline_scenario = ScenarioResult {
            scenario_id: "smoke_test".to_string(),
            passed: true,
            trace: baseline_trace,
            response: "baseline response".to_string(),
            error: None,
        };
        let baseline_run = RunResult::from_scenarios("baseline-run", vec![baseline_scenario]);

        // Compare should detect token regression (current uses more tokens than baseline).
        let deltas = compare_runs(&baseline_run, &current_run, 0.10);
        let token_delta = deltas.iter().find(|d| d.metric == "total_tokens");
        if let Some(d) = token_delta {
            assert!(d.is_regression, "Expected token regression");
            assert!(d.delta > 0.0, "Expected positive delta for regression");
        }

        rig.shutdown();
    }

    /// Verify that accessor methods on TestRig match InstrumentedLlm data.
    #[tokio::test]
    async fn test_rig_metric_accessors() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/simple_text.json"
        ))
        .expect("failed to load simple_text.json");

        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        // Before sending any message, metrics should be zero.
        assert_eq!(rig.llm_call_count(), 0);
        assert_eq!(rig.total_input_tokens(), 0);
        assert_eq!(rig.total_output_tokens(), 0);

        rig.send_message("hello").await;
        let _responses = rig.wait_for_responses(1, Duration::from_secs(10)).await;

        // After the agent processes, metrics should be populated.
        assert!(rig.llm_call_count() >= 1);
        assert!(rig.total_input_tokens() > 0);
        assert!(rig.total_output_tokens() > 0);
        assert!(rig.elapsed_ms() > 0);

        rig.shutdown();
    }
}

//! E2E trace tests: tool coverage.
//!
//! Exercises tools that were previously untested: json, shell, list_dir,
//! apply_patch, memory_read, and memory_tree.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use crate::support::cleanup::CleanupGuard;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    const TEST_DIR_BASE: &str = "/tmp/ironclaw_coverage_test";

    fn setup_test_dir(suffix: &str) -> String {
        let dir = format!("{TEST_DIR_BASE}_{suffix}");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("failed to create test directory");
        dir
    }

    // -----------------------------------------------------------------------
    // json tool
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_json_operations() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/coverage/json_operations.json"
        ))
        .expect("failed to load json_operations.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Parse and query this json data").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Extra: verify json tool was called at least 3 times.
        let started = rig.tool_calls_started();
        assert!(
            started.iter().filter(|n| n.as_str() == "json").count() >= 3,
            "Expected at least 3 json tool calls, got: {:?}",
            started
        );

        // Extra: metrics checks.
        let metrics = rig.collect_metrics().await;
        assert!(
            metrics.llm_calls >= 4,
            "Expected >= 4 LLM calls, got {}",
            metrics.llm_calls
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // shell tool
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_shell_echo() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/coverage/shell_echo.json"
        ))
        .expect("failed to load shell_echo.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Run a shell command for me").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // list_dir tool
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_dir() {
        let test_dir = setup_test_dir("list_dir");
        let _cleanup = CleanupGuard::new().dir(&test_dir);
        std::fs::write(format!("{test_dir}/file_a.txt"), "content a").unwrap();
        std::fs::write(format!("{test_dir}/file_b.txt"), "content b").unwrap();

        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/coverage/list_dir.json"
        ))
        .expect("failed to load list_dir.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("List the test directory").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // apply_patch tool
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_apply_patch_chain() {
        let test_dir = setup_test_dir("apply_patch");
        let _cleanup = CleanupGuard::new().dir(&test_dir);

        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/coverage/apply_patch_chain.json"
        ))
        .expect("failed to load apply_patch_chain.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Write a file and patch it").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Extra: verify the patch was applied on disk.
        let content = std::fs::read_to_string(format!("{test_dir}/patch_target.txt"))
            .expect("patch_target.txt should exist");
        assert!(
            content.contains("PATCHED"),
            "Expected 'PATCHED' in file content, got: {content:?}"
        );
        assert!(
            !content.contains("original"),
            "Expected 'original' to be replaced, but it still exists in: {content:?}"
        );

        // Extra: metrics checks.
        let metrics = rig.collect_metrics().await;
        assert!(metrics.llm_calls >= 4, "Expected >= 4 LLM calls");
        assert!(metrics.total_tool_calls() >= 3, "Expected >= 3 tool calls");

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // memory_read + memory_tree (full memory cycle)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_memory_full_cycle() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/coverage/memory_full_cycle.json"
        ))
        .expect("failed to load memory_full_cycle.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Exercise all four memory operations")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Extra: metrics checks.
        let metrics = rig.collect_metrics().await;
        assert!(metrics.llm_calls >= 5, "Expected >= 5 LLM calls");
        assert!(metrics.total_tool_calls() >= 4, "Expected >= 4 tool calls");

        rig.shutdown();
    }
}

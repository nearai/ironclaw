//! E2E trace tests: builtin tool coverage (#573).
//!
//! Covers time (parse, diff, invalid), routine (create, list, update, delete,
//! history), job (create, status, list, cancel), and HTTP replay.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    // -----------------------------------------------------------------------
    // Test 1: time_parse_and_diff
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn time_parse_and_diff() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/time_parse_diff.json"
        ))
        .expect("failed to load time_parse_diff.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Parse a time and compute a diff").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Time tool should have been called twice (parse + diff).
        let started = rig.tool_calls_started();
        let time_count = started.iter().filter(|n| n.as_str() == "time").count();
        assert!(
            time_count >= 2,
            "Expected >= 2 time tool calls, got {time_count}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 2: time_parse_invalid
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn time_parse_invalid() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/time_parse_invalid.json"
        ))
        .expect("failed to load time_parse_invalid.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Parse an invalid timestamp").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // The time tool call should have failed (invalid timestamp).
        let completed = rig.tool_calls_completed();
        let time_results: Vec<_> = completed
            .iter()
            .filter(|(name, _)| name == "time")
            .collect();
        assert!(!time_results.is_empty(), "Expected time tool to be called");
        assert!(
            time_results.iter().any(|(_, ok)| !ok),
            "Expected at least one failed time call: {time_results:?}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 3: routine_create_list
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn routine_create_list() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/routine_create_list.json"
        ))
        .expect("failed to load routine_create_list.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Create a daily routine and list all routines")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Both routine_create and routine_list should have succeeded.
        let completed = rig.tool_calls_completed();
        assert!(
            completed.iter().any(|(n, ok)| n == "routine_create" && *ok),
            "routine_create should succeed: {completed:?}"
        );
        assert!(
            completed.iter().any(|(n, ok)| n == "routine_list" && *ok),
            "routine_list should succeed: {completed:?}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 4: routine_update_delete
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn routine_update_delete() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/routine_update_delete.json"
        ))
        .expect("failed to load routine_update_delete.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Create, update, and delete a routine")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        let started = rig.tool_calls_started();
        assert!(
            started.contains(&"routine_create".to_string()),
            "routine_create not started"
        );
        assert!(
            started.contains(&"routine_update".to_string()),
            "routine_update not started"
        );
        assert!(
            started.contains(&"routine_delete".to_string()),
            "routine_delete not started"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 5: routine_history
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn routine_history() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/routine_history.json"
        ))
        .expect("failed to load routine_history.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Create a routine and check its history")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        let started = rig.tool_calls_started();
        assert!(
            started.contains(&"routine_create".to_string()),
            "routine_create missing"
        );
        assert!(
            started.contains(&"routine_history".to_string()),
            "routine_history missing"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 6: job_create_and_list
    // -----------------------------------------------------------------------
    // Note: job_status and cancel_job require a dynamic UUID from create_job
    // which trace-based tests cannot forward between steps. We verify
    // create_job + list_jobs (both ID-independent) and assert success.

    #[tokio::test]
    async fn job_create_and_list() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/job_create_status.json"
        ))
        .expect("failed to load job_create_status.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Create a job and list jobs").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Both tools should have succeeded.
        let completed = rig.tool_calls_completed();
        assert!(
            completed.iter().any(|(n, ok)| n == "create_job" && *ok),
            "create_job should succeed: {completed:?}"
        );
        assert!(
            completed.iter().any(|(n, ok)| n == "list_jobs" && *ok),
            "list_jobs should succeed: {completed:?}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 7: job_list_cancel
    // -----------------------------------------------------------------------
    // cancel_job uses a canned job_id ("latest") which is not a valid UUID
    // or hex prefix — we explicitly verify it fails while create_job and
    // list_jobs succeed.

    #[tokio::test]
    async fn job_list_cancel() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/job_list_cancel.json"
        ))
        .expect("failed to load job_list_cancel.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Create a job, list jobs, then cancel it")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        let completed = rig.tool_calls_completed();
        // create_job and list_jobs should succeed.
        assert!(
            completed.iter().any(|(n, ok)| n == "create_job" && *ok),
            "create_job should succeed: {completed:?}"
        );
        assert!(
            completed.iter().any(|(n, ok)| n == "list_jobs" && *ok),
            "list_jobs should succeed: {completed:?}"
        );
        // cancel_job should fail (canned "latest" is not a valid job ID).
        let cancel_results: Vec<_> = completed
            .iter()
            .filter(|(n, _)| n == "cancel_job")
            .collect();
        assert!(
            !cancel_results.is_empty(),
            "cancel_job should have been attempted: {completed:?}"
        );
        assert!(
            cancel_results.iter().all(|(_, ok)| !ok),
            "cancel_job should fail with invalid job_id: {cancel_results:?}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 8: http_get_with_replay
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn http_get_with_replay() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/http_get_replay.json"
        ))
        .expect("failed to load http_get_replay.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Make an http GET request").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // HTTP tool should have succeeded with the replayed exchange.
        let completed = rig.tool_calls_completed();
        assert!(
            completed.iter().any(|(n, ok)| n == "http" && *ok),
            "http tool should succeed: {completed:?}"
        );

        rig.shutdown();
    }
}

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
            .with_auto_approve_tools(true)
            .with_skills()
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
            .with_auto_approve_tools(true)
            .with_skills()
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
            .with_auto_approve_tools(true)
            .with_skills()
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
            .with_auto_approve_tools(true)
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
            .with_auto_approve_tools(true)
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
    // Test 6: routine_system_event_emit
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn routine_system_event_emit() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/routine_system_event_emit.json"
        ))
        .expect("failed to load routine_system_event_emit.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("Create a system-event routine and emit an event")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        let completed = rig.tool_calls_completed();
        assert!(
            completed.iter().any(|(n, ok)| n == "event_emit" && *ok),
            "event_emit should succeed: {completed:?}"
        );

        let results = rig.tool_results();
        let emit_result = results
            .iter()
            .find(|(n, _)| n == "event_emit")
            .expect("event_emit result missing");
        assert!(
            emit_result.1.contains("fired_routines"),
            "event_emit should report fired routine count: {:?}",
            emit_result.1
        );
        // Verify at least one routine actually fired (not just that the key exists).
        let emit_json: serde_json::Value =
            serde_json::from_str(&emit_result.1).expect("event_emit result should be valid JSON");
        assert!(
            emit_json["fired_routines"].as_u64().unwrap_or(0) > 0,
            "event_emit should have fired at least one routine: {:?}",
            emit_result.1
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 7: skill_install_routine_webhook_sim
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn skill_install_routine_webhook_sim() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/skill_install_routine_webhook_sim.json"
        ))
        .expect("failed to load skill_install_routine_webhook_sim.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_skills()
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("Install the workflow skill template and simulate a webhook routine run")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(20)).await;
        rig.verify_trace_expects(&trace, &responses);

        let completed = rig.tool_calls_completed();
        assert!(
            completed.iter().any(|(n, _)| n == "skill_install"),
            "skill_install should be called: {completed:?}"
        );
        for tool in &["routine_create", "event_emit", "routine_history"] {
            assert!(
                completed.iter().any(|(n, ok)| n == tool && *ok),
                "{tool} should succeed: {completed:?}"
            );
        }

        let results = rig.tool_results();
        let emit_result = results
            .iter()
            .find(|(n, _)| n == "event_emit")
            .expect("event_emit result missing");
        assert!(
            emit_result.1.contains("fired_routines"),
            "event_emit should include fired_routines: {:?}",
            emit_result.1
        );

        let _history_result = results
            .iter()
            .find(|(n, _)| n == "routine_history")
            .expect("routine_history result missing");

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 8: job_create_status
    // -----------------------------------------------------------------------
    // Uses {{call_cj_1.job_id}} template to forward the dynamic UUID from
    // create_job's result into job_status's arguments.

    #[tokio::test]
    async fn job_create_status() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/job_create_status.json"
        ))
        .expect("failed to load job_create_status.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("Create a job and check its status").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Both tools should have succeeded.
        let completed = rig.tool_calls_completed();
        assert!(
            completed.iter().any(|(n, ok)| n == "create_job" && *ok),
            "create_job should succeed: {completed:?}"
        );
        assert!(
            completed.iter().any(|(n, ok)| n == "job_status" && *ok),
            "job_status should succeed: {completed:?}"
        );

        // Verify tool results contain expected content.
        let results = rig.tool_results();
        let create_result = results
            .iter()
            .find(|(n, _)| n == "create_job")
            .expect("create_job result missing");
        assert!(
            create_result.1.contains("job_id"),
            "create_job should return a job_id: {:?}",
            create_result.1
        );
        assert!(
            create_result.1.contains("in_progress"),
            "create_job should dispatch through the scheduler, not stay pending: {:?}",
            create_result.1
        );
        assert!(
            !create_result.1.contains("scheduler unavailable"),
            "create_job should not fall back to the unscheduled path: {:?}",
            create_result.1
        );
        let status_result = results
            .iter()
            .find(|(n, _)| n == "job_status")
            .expect("job_status result missing");
        assert!(
            status_result.1.contains("Test analysis job"),
            "job_status should return the job title: {:?}",
            status_result.1
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 9: job_list_cancel
    // -----------------------------------------------------------------------
    // Uses {{call_cj_lc.job_id}} template to forward the dynamic UUID from
    // create_job into cancel_job.

    #[tokio::test]
    async fn job_list_cancel() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/job_list_cancel.json"
        ))
        .expect("failed to load job_list_cancel.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("Create a job, list jobs, then cancel it")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // All three tools should have succeeded.
        let completed = rig.tool_calls_completed();
        assert!(
            completed.iter().any(|(n, ok)| n == "create_job" && *ok),
            "create_job should succeed: {completed:?}"
        );
        assert!(
            completed.iter().any(|(n, ok)| n == "list_jobs" && *ok),
            "list_jobs should succeed: {completed:?}"
        );
        assert!(
            completed.iter().any(|(n, ok)| n == "cancel_job" && *ok),
            "cancel_job should succeed: {completed:?}"
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
            .with_auto_approve_tools(true)
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

    // -----------------------------------------------------------------------
    // Test: tool_info_discovery (two-level detail)
    // -----------------------------------------------------------------------
    // Verifies the tool_info built-in returns:
    // - Default (no include_schema): name, description, parameter names array
    // - With include_schema: true: adds full typed JSON Schema

    #[tokio::test]
    async fn tool_info_discovery() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/tool_info_discovery.json"
        ))
        .expect("failed to load tool_info_discovery.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("What is the schema for the echo and time tools?")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // tool_info should have been called twice (echo + time), both succeeding.
        let completed = rig.tool_calls_completed();
        let tool_info_calls: Vec<_> = completed.iter().filter(|(n, _)| n == "tool_info").collect();
        assert_eq!(
            tool_info_calls.len(),
            2,
            "Expected 2 tool_info calls, got {tool_info_calls:?}"
        );
        assert!(
            tool_info_calls.iter().all(|(_, ok)| *ok),
            "All tool_info calls should succeed: {tool_info_calls:?}"
        );

        // Verify the results contain expected fields.
        let results = rig.tool_results();
        let info_results: Vec<_> = results.iter().filter(|(n, _)| n == "tool_info").collect();

        // First call was for "echo" (default, no include_schema) — result should
        // contain "echo" and "parameters" as an array of names (not full schema).
        let echo_result = info_results
            .iter()
            .find(|(_, preview)| preview.contains("echo"))
            .expect("tool_info result should contain 'echo'");
        assert!(
            echo_result.1.contains("message"),
            "echo default result should list 'message' parameter name: {:?}",
            echo_result.1
        );
        // Default mode should NOT include the full "schema" key
        let echo_json: serde_json::Value = serde_json::from_str(&echo_result.1)
            .expect("echo tool_info result should be valid JSON");
        assert!(
            echo_json.get("schema").is_none(),
            "Default tool_info should not include schema field: {:?}",
            echo_result.1
        );

        // Second call was for "time" with include_schema: true — result should
        // contain "time", "schema" field with full object.
        let time_result = info_results
            .iter()
            .find(|(_, preview)| preview.contains("time"))
            .expect("tool_info result should contain 'time'");
        let time_json: serde_json::Value = serde_json::from_str(&time_result.1)
            .expect("time tool_info result should be valid JSON");
        assert!(
            time_json.get("schema").is_some(),
            "include_schema: true should include schema field: {:?}",
            time_result.1
        );
        assert!(
            time_json["schema"]["properties"].is_object(),
            "schema should have properties: {:?}",
            time_result.1
        );

        rig.shutdown();
    }
}

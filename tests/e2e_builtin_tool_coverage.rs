//! E2E trace tests: builtin tool coverage (#573).
//!
//! Covers time (parse, diff, invalid), routine (create, list, update, delete,
//! history), job (create, status, list, cancel), and HTTP replay.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use ironclaw::agent::routine::{RoutineAction, Trigger};

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

        let routine = rig
            .database()
            .get_routine_by_name("test-user", "daily-check")
            .await
            .expect("get_routine_by_name")
            .expect("daily-check should exist");

        match &routine.trigger {
            Trigger::Cron { schedule, timezone } => {
                assert_eq!(schedule, "0 0 9 * * * *");
                assert_eq!(timezone.as_deref(), Some("America/New_York"));
            }
            other => panic!("expected cron trigger, got {other:?}"),
        }

        match &routine.action {
            RoutineAction::Lightweight {
                prompt,
                context_paths,
                use_tools,
                max_tool_rounds,
                ..
            } => {
                assert!(prompt.contains("Check system status"));
                assert_eq!(context_paths, &vec!["context/priorities.md".to_string()]);
                assert!(*use_tools, "lightweight routine should keep use_tools=true");
                assert_eq!(*max_tool_rounds, 2);
            }
            other => panic!("expected lightweight routine action, got {other:?}"),
        }

        assert_eq!(routine.notify.channel.as_deref(), Some("telegram"));
        assert_eq!(routine.notify.user.as_deref(), Some("ops-team"));
        assert_eq!(routine.guardrails.cooldown.as_secs(), 600);

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
    // Test 5: routine_manual_create
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn routine_manual_create() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/routine_manual_create.json"
        ))
        .expect("failed to load routine_manual_create.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("Create a manual routine for bug triage")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        let routine = rig
            .database()
            .get_routine_by_name("test-user", "manual-triage")
            .await
            .expect("get_routine_by_name")
            .expect("manual-triage should exist");

        assert!(matches!(routine.trigger, Trigger::Manual));
        assert!(
            matches!(&routine.action, RoutineAction::Lightweight { use_tools, .. } if !*use_tools),
            "manual routine should default to lightweight without tools: {:?}",
            routine.action
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 6: routine_history
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
    // Test 7: routine_system_event_emit
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

        let routine = rig
            .database()
            .get_routine_by_name("test-user", "gh-issue-emit-test")
            .await
            .expect("get_routine_by_name")
            .expect("gh-issue-emit-test should exist");

        match &routine.trigger {
            Trigger::SystemEvent {
                source,
                event_type,
                filters,
            } => {
                assert_eq!(source, "github");
                assert_eq!(event_type, "issue.opened");
                assert_eq!(
                    filters.get("repository").map(String::as_str),
                    Some("nearai/ironclaw")
                );
                assert_eq!(filters.get("priority").map(String::as_str), Some("p1"));
            }
            other => panic!("expected system_event trigger, got {other:?}"),
        }

        match &routine.action {
            RoutineAction::FullJob { description, .. } => {
                assert!(description.contains("Summarize the new issue"));
            }
            other => panic!("expected full_job action, got {other:?}"),
        }

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 8: routine_create_grouped
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn routine_create_grouped() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/routine_create_grouped.json"
        ))
        .expect("failed to load routine_create_grouped.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("Create a grouped cron routine with delivery settings")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        let routine = rig
            .database()
            .get_routine_by_name("test-user", "weekday-digest")
            .await
            .expect("get_routine_by_name")
            .expect("weekday-digest should exist");

        match &routine.trigger {
            Trigger::Cron { schedule, timezone } => {
                assert_eq!(schedule, "0 0 9 * * MON-FRI");
                assert_eq!(timezone.as_deref(), Some("UTC"));
            }
            other => panic!("expected cron trigger, got {other:?}"),
        }

        match &routine.action {
            RoutineAction::FullJob { description, .. } => {
                assert!(description.contains("Prepare the morning digest"));
            }
            other => panic!("expected full_job action, got {other:?}"),
        }

        assert_eq!(routine.notify.channel.as_deref(), Some("telegram"));
        assert_eq!(routine.notify.user.as_deref(), Some("ops-team"));
        assert_eq!(routine.guardrails.cooldown.as_secs(), 30);

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 9: routine_system_event_emit_grouped
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn routine_system_event_emit_grouped() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/routine_system_event_emit_grouped.json"
        ))
        .expect("failed to load routine_system_event_emit_grouped.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("Create a grouped system-event routine and emit a matching event")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        let routine = rig
            .database()
            .get_routine_by_name("test-user", "grouped-gh-issue-watch")
            .await
            .expect("get_routine_by_name")
            .expect("grouped-gh-issue-watch should exist");

        match &routine.trigger {
            Trigger::SystemEvent {
                source,
                event_type,
                filters,
            } => {
                assert_eq!(source, "github");
                assert_eq!(event_type, "issue.opened");
                assert_eq!(
                    filters.get("repository").map(String::as_str),
                    Some("nearai/ironclaw")
                );
                assert_eq!(filters.get("priority").map(String::as_str), Some("p1"));
            }
            other => panic!("expected system_event trigger, got {other:?}"),
        }

        let results = rig.tool_results();
        let emit_result = results
            .iter()
            .find(|(n, _)| n == "event_emit")
            .expect("event_emit result missing");
        let emit_json: serde_json::Value =
            serde_json::from_str(&emit_result.1).expect("event_emit result should be valid JSON");
        assert!(
            emit_json["fired_routines"].as_u64().unwrap_or(0) > 0,
            "event_emit should have fired at least one grouped routine: {:?}",
            emit_result.1
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 10: skill_install_routine_webhook_sim
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
    // Test: tool_info_discovery (three-level detail)
    // -----------------------------------------------------------------------
    // Verifies the tool_info built-in returns:
    // - Default (no include_schema): name, description, parameter names array
    // - `detail: "summary"`: curated summary guidance
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

        // tool_info should have been called three times (echo + routine_create + time), all succeeding.
        let completed = rig.tool_calls_completed();
        let tool_info_calls: Vec<_> = completed.iter().filter(|(n, _)| n == "tool_info").collect();
        assert_eq!(
            tool_info_calls.len(),
            3,
            "Expected 3 tool_info calls, got {tool_info_calls:?}"
        );
        assert!(
            tool_info_calls.iter().all(|(_, ok)| *ok),
            "All tool_info calls should succeed: {tool_info_calls:?}"
        );

        // Verify the results contain expected fields.
        let results = rig.tool_results();
        let info_results: Vec<_> = results.iter().filter(|(n, _)| n == "tool_info").collect();
        let info_json: Vec<serde_json::Value> = info_results
            .iter()
            .map(|(_, preview)| {
                serde_json::from_str(preview)
                    .expect("tool_info result preview should be valid JSON")
            })
            .collect();

        // First call was for "echo" (default, no include_schema) — result should
        // contain "echo" and "parameters" as an array of names (not full schema).
        let echo_json = info_json
            .iter()
            .find(|info| info["name"] == "echo")
            .expect("tool_info result should contain 'echo'");
        assert!(
            echo_json["parameters"]
                .as_array()
                .is_some_and(|params| params.iter().any(|param| param == "message")),
            "echo default result should list 'message' parameter name: {:?}",
            echo_json
        );
        // Default mode should NOT include the full "schema" key
        assert!(
            echo_json.get("schema").is_none(),
            "Default tool_info should not include schema field: {:?}",
            echo_json
        );

        // Second call was for "routine_create" with detail: "summary" — result
        // should contain a summary object with rules/examples.
        let routine_json = info_json
            .iter()
            .find(|info| info["name"] == "routine_create")
            .expect("tool_info result should contain 'routine_create'");
        assert!(
            routine_json.get("summary").is_some(),
            "detail: summary should include summary field: {:?}",
            routine_json
        );
        assert!(
            routine_json["summary"]["conditional_requirements"]
                .as_array()
                .is_some_and(|rules| rules.iter().any(|rule| {
                    rule.as_str()
                        .is_some_and(|rule| rule.contains("request.kind='cron'"))
                })),
            "routine_create summary should mention cron requirement: {:?}",
            routine_json
        );

        // Third call was for "time" with include_schema: true — result should
        // contain "time", "schema" field with full object.
        let time_json = info_json
            .iter()
            .find(|info| info["name"] == "time")
            .expect("tool_info result should contain 'time'");
        assert!(
            time_json.get("schema").is_some(),
            "include_schema: true should include schema field: {:?}",
            time_json
        );
        assert!(
            time_json["schema"]["properties"].is_object(),
            "schema should have properties: {:?}",
            time_json
        );

        rig.shutdown();
    }
}

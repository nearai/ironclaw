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
    use ironclaw::context::{JobContext, JobState};
    use uuid::Uuid;

    use crate::support::test_rig::{TestRig, TestRigBuilder};
    use crate::support::trace_llm::{
        LlmTrace, RequestHint, TraceResponse, TraceStep, TraceToolCall, TraceTurn,
    };

    fn text_step(content: &str) -> TraceStep {
        TraceStep {
            request_hint: None,
            response: TraceResponse::Text {
                content: content.to_string(),
                input_tokens: 10,
                output_tokens: 5,
            },
            expected_tool_results: Vec::new(),
        }
    }

    fn hinted_text_step(content: &str, last_user_message_contains: &str) -> TraceStep {
        TraceStep {
            request_hint: Some(RequestHint {
                last_user_message_contains: Some(last_user_message_contains.to_string()),
                min_message_count: None,
            }),
            response: TraceResponse::Text {
                content: content.to_string(),
                input_tokens: 10,
                output_tokens: 5,
            },
            expected_tool_results: Vec::new(),
        }
    }

    fn extract_job_id(response: &str) -> Option<Uuid> {
        response
            .split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
            .find_map(|token| Uuid::parse_str(token).ok())
    }

    async fn resolve_created_job_id(
        rig: &TestRig,
        responses: &[ironclaw::channels::OutgoingResponse],
        expected_title: &str,
    ) -> Uuid {
        if let Some(job_id) = responses
            .iter()
            .find_map(|response| extract_job_id(&response.content))
        {
            return job_id;
        }

        rig.database()
            .list_agent_jobs_for_user("test-user")
            .await
            .expect("list_agent_jobs_for_user should succeed")
            .into_iter()
            .find(|job| job.title == expected_title)
            .map(|job| job.id)
            .unwrap_or_else(|| {
                panic!(
                    "failed to resolve job id for title {expected_title:?}; responses were: {:?}",
                    responses
                        .iter()
                        .map(|response| &response.content)
                        .collect::<Vec<_>>()
                )
            })
    }

    async fn wait_for_job_state(rig: &TestRig, job_id: Uuid, expected: JobState) -> JobContext {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

        loop {
            if let Some(job) = rig
                .database()
                .get_job(job_id)
                .await
                .expect("get_job should succeed")
                && job.state == expected
            {
                return job;
            }

            assert!(
                tokio::time::Instant::now() < deadline,
                "job {job_id} did not reach state {expected:?} before timeout"
            );

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn requests_contain(requests: &[Vec<ironclaw::llm::ChatMessage>], needle: &str) -> bool {
        requests
            .iter()
            .flatten()
            .any(|message| message.content.contains(needle))
    }

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
            .get_routine_by_name("test-user", None, "daily-check")
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
    // Test 5: routine_update_fail_delete_fallback
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn routine_update_fail_delete_fallback() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/routine_update_fail_delete_fallback.json"
        ))
        .expect("failed to load routine_update_fail_delete_fallback.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("Try converting a routine trigger, then recover by deleting it")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        let completed = rig.tool_calls_completed();
        assert!(
            completed.iter().any(|(n, ok)| n == "routine_update" && !ok),
            "routine_update should fail in this regression path: {completed:?}"
        );
        assert!(
            completed.iter().any(|(n, ok)| n == "routine_delete" && *ok),
            "routine_delete should recover successfully via preserved routine identity: {completed:?}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 6: routine_manual_create_defaults_to_tools_enabled
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn routine_manual_create_defaults_to_tools_enabled() {
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
            .get_routine_by_name("test-user", None, "manual-triage")
            .await
            .expect("get_routine_by_name")
            .expect("manual-triage should exist");

        assert!(matches!(routine.trigger, Trigger::Manual));
        assert!(
            matches!(&routine.action, RoutineAction::Lightweight { use_tools, .. } if *use_tools),
            "manual routine should default to lightweight with tools enabled: {:?}",
            routine.action
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 7: routine_manual_create_explicit_no_tools
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn routine_manual_create_explicit_no_tools() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/tools/routine_manual_create_no_tools.json"
        ))
        .expect("failed to load routine_manual_create_no_tools.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("Create a manual routine for quiet text-only bug triage")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        let routine = rig
            .database()
            .get_routine_by_name("test-user", None, "manual-triage-no-tools")
            .await
            .expect("get_routine_by_name")
            .expect("manual-triage-no-tools should exist");

        assert!(matches!(routine.trigger, Trigger::Manual));
        assert!(
            matches!(&routine.action, RoutineAction::Lightweight { use_tools, .. } if !*use_tools),
            "manual routine should preserve explicit use_tools=false: {:?}",
            routine.action
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 8: routine_history
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
    // Test 8: routine_system_event_emit
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
            .get_routine_by_name("test-user", None, "gh-issue-emit-test")
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
            .get_routine_by_name("test-user", None, "weekday-digest")
            .await
            .expect("get_routine_by_name")
            .expect("weekday-digest should exist");

        match &routine.trigger {
            Trigger::Cron { schedule, timezone } => {
                assert_eq!(schedule, "0 0 9 * * MON-FRI *");
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
            .get_routine_by_name("test-user", None, "grouped-gh-issue-watch")
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
    // Test 8a: command_job_fails_fast_on_repeated_empty_tool_completions
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn command_job_fails_fast_on_repeated_empty_tool_completions() {
        let trace = LlmTrace::single_turn(
            "test-empty-tool-recovery-fail",
            "(worker only)",
            vec![
                text_step(""),
                text_step(""),
                hinted_text_step("", "valid arguments"),
                text_step(""),
                hinted_text_step("", "Do not call any more tools in the next reply."),
            ],
        );

        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("/job reproduce empty tool completion loop")
            .await;
        let create_responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;
        let job_id = resolve_created_job_id(
            &rig,
            &create_responses,
            "reproduce empty tool completion loop",
        )
        .await;

        let job = wait_for_job_state(&rig, job_id, JobState::Failed).await;
        assert_eq!(job.title, "reproduce empty tool completion loop");

        let failure_reason = rig
            .database()
            .get_agent_job_failure_reason(job_id)
            .await
            .expect("get_agent_job_failure_reason should succeed")
            .expect("failed job should persist a failure reason");
        assert!(
            failure_reason
                .contains("repeatedly returned empty or malformed tool-completion responses"),
            "unexpected failure reason: {failure_reason}"
        );
        assert!(
            !failure_reason.contains("max iterations"),
            "failure should not surface as iteration exhaustion: {failure_reason}"
        );

        assert_eq!(
            rig.llm_call_count(),
            5,
            "worker should stop after the bounded recovery flow"
        );
        assert!(
            !rig.collect_metrics().await.hit_iteration_limit,
            "bounded recovery should stop before iteration-limit reporting"
        );

        let requests = rig.captured_llm_requests();
        assert!(
            requests_contain(&requests, "call it now with valid arguments"),
            "expected targeted tool-mode recovery nudge in worker requests"
        );
        assert!(
            requests_contain(&requests, "Do not call any more tools in the next reply."),
            "expected forced text-only recovery prompt in worker requests"
        );

        rig.clear().await;
        rig.send_message(&format!("/status {}", job_id)).await;
        let status_responses = rig.wait_for_responses(1, Duration::from_secs(5)).await;
        assert!(
            status_responses[0].content.contains("Status: Failed"),
            "unexpected status response: {:?}",
            status_responses[0].content
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 8b: command_job_text_recovery_can_complete
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn command_job_text_recovery_can_complete() {
        let trace = LlmTrace::single_turn(
            "test-empty-tool-recovery-success",
            "(worker only)",
            vec![
                text_step(""),
                text_step(""),
                hinted_text_step("", "valid arguments"),
                text_step(""),
                hinted_text_step(
                    "The job is complete. I finished the requested work and there is nothing left to do.",
                    "Do not call any more tools in the next reply.",
                ),
            ],
        );

        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("/job recover after malformed tool completions")
            .await;
        let create_responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;
        let job_id = resolve_created_job_id(
            &rig,
            &create_responses,
            "recover after empty tool completions",
        )
        .await;

        let job = wait_for_job_state(&rig, job_id, JobState::Completed).await;
        assert_eq!(job.title, "recover after malformed tool completions");

        assert_eq!(
            rig.llm_call_count(),
            5,
            "worker should complete within the bounded recovery flow"
        );

        let requests = rig.captured_llm_requests();
        assert!(
            requests_contain(&requests, "call it now with valid arguments"),
            "expected targeted tool-mode recovery nudge in worker requests"
        );
        assert!(
            requests_contain(&requests, "Do not call any more tools in the next reply."),
            "expected forced text-only recovery prompt in worker requests"
        );

        rig.clear().await;
        rig.send_message(&format!("/status {}", job_id)).await;
        let status_responses = rig.wait_for_responses(1, Duration::from_secs(5)).await;
        assert!(
            status_responses[0].content.contains("Status: Completed"),
            "unexpected status response: {:?}",
            status_responses[0].content
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

    #[tokio::test]
    async fn tool_info_clarifies_message_and_channel_setup_roles() {
        let trace = LlmTrace::new(
            "test-tool-info-channel-message-clarity",
            vec![TraceTurn {
                user_input: "How do message and channels differ?".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_tool_info_message".to_string(),
                                name: "tool_info".to_string(),
                                arguments: serde_json::json!({"name": "message"}),
                            }],
                            input_tokens: 100,
                            output_tokens: 20,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_tool_info_tool_search".to_string(),
                                name: "tool_info".to_string(),
                                arguments: serde_json::json!({"name": "tool_search"}),
                            }],
                            input_tokens: 140,
                            output_tokens: 20,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "I checked both tool descriptions.".to_string(),
                            input_tokens: 220,
                            output_tokens: 30,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: Default::default(),
            }],
        );

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_auto_approve_tools(true)
            .build()
            .await;

        rig.send_message("How do message and channels differ?")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        let results = rig.tool_results();
        let info_results: Vec<_> = results.iter().filter(|(n, _)| n == "tool_info").collect();
        assert_eq!(info_results.len(), 2, "Expected two tool_info results");

        let info_json: Vec<serde_json::Value> = info_results
            .iter()
            .map(|(_, preview)| {
                serde_json::from_str(preview)
                    .expect("tool_info result preview should be valid JSON")
            })
            .collect();

        let message_json = info_json
            .iter()
            .find(|info| info["name"] == "message")
            .expect("tool_info result should contain 'message'");
        let message_description = message_json["description"]
            .as_str()
            .expect("message description should be a string");
        assert!(
            message_description.contains("Use normal assistant output to reply"),
            "message description should distinguish normal replies: {message_description}"
        );
        assert!(
            message_description.contains("proactive notifications"),
            "message description should describe proactive sends: {message_description}"
        );

        let tool_search_json = info_json
            .iter()
            .find(|info| info["name"] == "tool_search")
            .expect("tool_info result should contain 'tool_search'");
        let tool_search_description = tool_search_json["description"]
            .as_str()
            .expect("tool_search description should be a string");
        assert!(
            tool_search_description.contains("`tool_install`")
                && tool_search_description.contains("`tool_activate`"),
            "tool_search description should describe setup/activation via tool_install and \
             tool_activate: {tool_search_description}"
        );
        assert!(
            tool_search_description.contains("use the `message` tool for proactive outbound sends"),
            "tool_search description should point outbound sends to message: {tool_search_description}"
        );

        rig.shutdown();
    }
}

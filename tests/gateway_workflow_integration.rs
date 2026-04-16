//! Live-ish gateway workflow integration using an in-process mock OpenAI server.
//! This exercises the same path as manual validation:
//! - chat send through gateway
//! - routine creation via tool call
//! - system-event emission via tool call
//! - webhook ingestion via generic tools webhook server
//! - status/runs checks via routines API

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw::agent::routine::{
        NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger,
        reset_routine_verification_state, routine_verification_fingerprint,
    };
    use ironclaw::context::JobContext;
    use ironclaw::tools::{Tool, ToolError, ToolOutput};
    use uuid::Uuid;

    use crate::support::gateway_workflow_harness::GatewayWorkflowHarness;
    use crate::support::mock_openai_server::{
        MockOpenAiResponse, MockOpenAiRule, MockOpenAiServerBuilder, MockToolCall,
    };

    struct BlockingTool;

    #[async_trait]
    impl Tool for BlockingTool {
        fn name(&self) -> &str {
            "block_once"
        }

        fn description(&self) -> &str {
            "Sleeps long enough for cancellation-path integration tests"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(ToolOutput::text("block complete", Duration::from_secs(30)))
        }
    }

    #[tokio::test]
    async fn gateway_workflow_harness_chat_and_webhook() {
        let mock = MockOpenAiServerBuilder::new()
            .with_rule(MockOpenAiRule::on_user_contains(
                "create workflow routine",
                MockOpenAiResponse::ToolCalls(vec![MockToolCall::new(
                    "call_create_1",
                    "routine_create",
                    serde_json::json!({
                        "name": "wf-ci-webhook-demo",
                        "description": "CI webhook workflow demo",
                        "trigger_type": "system_event",
                        "event_source": "github",
                        "event_type": "issue.opened",
                        "event_filters": {"repository": "nearai/ironclaw"},
                        // This test fires the same routine via event_emit and then the
                        // webhook endpoint back-to-back, so cooldown must not suppress
                        // the second path.
                        "cooldown_secs": 0,
                        "action_type": "lightweight",
                        "prompt": "Summarize webhook and report issue number"
                    }),
                )]),
            ))
            .with_rule(MockOpenAiRule::on_user_contains(
                "emit webhook event",
                MockOpenAiResponse::ToolCalls(vec![MockToolCall::new(
                    "call_emit_1",
                    "event_emit",
                    serde_json::json!({
                        "source": "github",
                        "event_type": "issue.opened",
                        "payload": {
                            "repository": "nearai/ironclaw",
                            "issue": {"number": 777, "title": "Infra test"}
                        }
                    }),
                )]),
            ))
            .with_default_response(MockOpenAiResponse::Text("ack".to_string()))
            .start()
            .await;

        let harness =
            GatewayWorkflowHarness::start_openai_compatible(&mock.openai_base_url(), "mock-model")
                .await;

        let thread_id = harness.create_thread().await;
        harness
            .send_chat(&thread_id, "create workflow routine")
            .await;
        harness
            .wait_for_turns(&thread_id, 1, Duration::from_secs(10))
            .await;

        let mut routine = None;
        for _ in 0..30 {
            routine = harness.routine_by_name("wf-ci-webhook-demo").await;
            if routine.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let routine = if let Some(r) = routine {
            r
        } else {
            let history_dbg = harness.history(&thread_id).await;
            let started_dbg = harness.test_channel.tool_calls_started();
            let requests_dbg = mock.requests().await;
            panic!(
                "routine not created; tool_calls_started={started_dbg:?}; history={history_dbg}; mock_requests={requests_dbg:?}"
            );
        };
        let routine_id = routine["id"].as_str().expect("routine id missing");

        harness.send_chat(&thread_id, "emit webhook event").await;

        let history = harness
            .wait_for_turns(&thread_id, 2, Duration::from_secs(10))
            .await;
        let turns = history["turns"].as_array().expect("turns array missing");
        assert!(turns.len() >= 2, "expected at least 2 turns");

        let runs_before = harness.routine_runs(routine_id).await;
        let before_count = runs_before["runs"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or_default();

        let hook = harness
            .github_webhook(
                "issues",
                serde_json::json!({
                    "action": "opened",
                    "repository": {"full_name": "nearai/ironclaw"},
                    "issue": {"number": 778, "title": "Webhook endpoint test"}
                }),
            )
            .await;

        assert_eq!(hook["status"], "accepted");
        assert_eq!(hook["emitted_events"], 1);
        assert!(
            hook["fired_routines"].as_u64().unwrap_or(0) >= 1,
            "expected webhook to fire at least one routine"
        );

        let mut after_count = before_count;
        for _ in 0..50 {
            let runs_after = harness.routine_runs(routine_id).await;
            after_count = runs_after["runs"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or_default();
            if after_count > before_count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            after_count > before_count,
            "expected routine runs to increase after webhook; before={before_count}, after={after_count}"
        );

        let requests = mock.requests().await;
        assert!(
            requests.len() >= 2,
            "expected mock LLM server to receive requests"
        );

        harness.shutdown().await;
        mock.shutdown().await;
    }

    #[tokio::test]
    async fn routines_toggle_reenable_cron_recomputes_next_fire_at() {
        let mock = MockOpenAiServerBuilder::new()
            .with_rule(MockOpenAiRule::on_user_contains(
                "create cron routine",
                MockOpenAiResponse::ToolCalls(vec![MockToolCall::new(
                    "call_create_cron_1",
                    "routine_create",
                    serde_json::json!({
                        "name": "wf-cron-toggle-reenable",
                        "description": "Cron toggle regression test",
                        "trigger_type": "cron",
                        "schedule": "0 */5 * * * *",
                        "timezone": "UTC",
                        "action_type": "lightweight",
                        "prompt": "noop"
                    }),
                )]),
            ))
            .with_default_response(MockOpenAiResponse::Text("ack".to_string()))
            .start()
            .await;

        let harness =
            GatewayWorkflowHarness::start_openai_compatible(&mock.openai_base_url(), "mock-model")
                .await;

        let thread_id = harness.create_thread().await;
        harness.send_chat(&thread_id, "create cron routine").await;
        harness
            .wait_for_turns(&thread_id, 1, Duration::from_secs(10))
            .await;

        let routine = harness
            .routine_by_name("wf-cron-toggle-reenable")
            .await
            .expect("routine should exist");
        let routine_id = routine
            .get("id")
            .and_then(|v| v.as_str())
            .expect("routine id missing");

        let routine_uuid = Uuid::parse_str(routine_id).expect("valid routine uuid");

        // Disable through the web toggle endpoint.
        harness
            .client
            .post(format!(
                "{}/api/routines/{routine_id}/toggle",
                harness.base_url()
            ))
            .bearer_auth(&harness.auth_token)
            .json(&serde_json::json!({ "enabled": false }))
            .send()
            .await
            .expect("disable toggle request failed")
            .error_for_status()
            .expect("disable toggle non-2xx");

        // Simulate an unscheduled disabled cron routine (next_fire_at missing).
        let mut stored = harness
            .db
            .get_routine(routine_uuid)
            .await
            .expect("db get_routine")
            .expect("routine should still exist");
        stored.next_fire_at = None;
        harness
            .db
            .update_routine(&stored)
            .await
            .expect("db update_routine");

        // Re-enable through the web toggle endpoint.
        harness
            .client
            .post(format!(
                "{}/api/routines/{routine_id}/toggle",
                harness.base_url()
            ))
            .bearer_auth(&harness.auth_token)
            .json(&serde_json::json!({ "enabled": true }))
            .send()
            .await
            .expect("enable toggle request failed")
            .error_for_status()
            .expect("enable toggle non-2xx");

        let detail = harness
            .client
            .get(format!("{}/api/routines/{routine_id}", harness.base_url()))
            .bearer_auth(&harness.auth_token)
            .send()
            .await
            .expect("detail request failed")
            .error_for_status()
            .expect("detail non-2xx")
            .json::<serde_json::Value>()
            .await
            .expect("invalid detail response");

        assert_eq!(detail["enabled"].as_bool(), Some(true));
        assert!(
            detail["next_fire_at"].as_str().is_some(),
            "expected next_fire_at to be recomputed when re-enabling cron routine, got {detail}"
        );

        harness.shutdown().await;
        mock.shutdown().await;
    }

    #[tokio::test]
    async fn routines_detail_omits_legacy_full_job_permission_surface() {
        let mock = MockOpenAiServerBuilder::new()
            .with_default_response(MockOpenAiResponse::Text("ack".to_string()))
            .start()
            .await;

        let harness =
            GatewayWorkflowHarness::start_openai_compatible(&mock.openai_base_url(), "mock-model")
                .await;

        let routine = Routine {
            id: Uuid::new_v4(),
            name: "wf-full-job-permissions".to_string(),
            description: "Permission detail regression test".to_string(),
            user_id: harness.user_id.clone(),
            workspace_id: None,
            enabled: true,
            trigger: Trigger::Manual,
            action: RoutineAction::FullJob {
                title: "permission-detail".to_string(),
                description: "Check effective permission detail".to_string(),
                max_iterations: 3,
            },
            guardrails: RoutineGuardrails {
                cooldown: Duration::from_secs(0),
                max_concurrent: 1,
                dedup_window: None,
            },
            notify: NotifyConfig::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        harness
            .db
            .create_routine(&routine)
            .await
            .expect("create routine");

        let detail = harness
            .client
            .get(format!(
                "{}/api/routines/{}",
                harness.base_url(),
                routine.id
            ))
            .bearer_auth(&harness.auth_token)
            .send()
            .await
            .expect("detail request failed")
            .error_for_status()
            .expect("detail non-2xx")
            .json::<serde_json::Value>()
            .await
            .expect("invalid detail response");

        assert!(
            detail.get("full_job_permissions").is_none(),
            "detail response should not expose legacy permission fields: {detail}"
        );
        assert_eq!(detail["action"]["type"].as_str(), Some("full_job"));
        assert_eq!(
            detail["action"]["description"].as_str(),
            Some("Check effective permission detail")
        );

        harness.shutdown().await;
        mock.shutdown().await;
    }

    #[tokio::test]
    async fn routines_api_surfaces_unverified_status_for_new_routine() {
        let mock = MockOpenAiServerBuilder::new()
            .with_default_response(MockOpenAiResponse::Text("ack".to_string()))
            .start()
            .await;

        let harness =
            GatewayWorkflowHarness::start_openai_compatible(&mock.openai_base_url(), "mock-model")
                .await;

        let mut routine = Routine {
            id: Uuid::new_v4(),
            name: "wf-unverified".to_string(),
            description: "Unverified status regression test".to_string(),
            user_id: harness.user_id.clone(),
            workspace_id: None,
            enabled: true,
            trigger: Trigger::Manual,
            action: RoutineAction::Lightweight {
                prompt: "Check verification status".to_string(),
                context_paths: Vec::new(),
                max_tokens: 512,
                use_tools: false,
                max_tool_rounds: 1,
            },
            guardrails: RoutineGuardrails {
                cooldown: Duration::from_secs(0),
                max_concurrent: 1,
                dedup_window: None,
            },
            notify: NotifyConfig::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        routine.state = reset_routine_verification_state(
            &routine.state,
            routine_verification_fingerprint(&routine),
        );
        harness
            .db
            .create_routine(&routine)
            .await
            .expect("create routine");

        let mut disabled_routine = routine.clone();
        disabled_routine.id = Uuid::new_v4();
        disabled_routine.name = "wf-unverified-disabled".to_string();
        disabled_routine.enabled = false;
        disabled_routine.state = reset_routine_verification_state(
            &disabled_routine.state,
            routine_verification_fingerprint(&disabled_routine),
        );
        harness
            .db
            .create_routine(&disabled_routine)
            .await
            .expect("create disabled routine");

        let list = harness.list_routines().await;
        let routine_id = routine.id.to_string();
        let listed = list["routines"]
            .as_array()
            .expect("routines array")
            .iter()
            .find(|item| item["id"].as_str() == Some(routine_id.as_str()))
            .expect("routine should be listed");
        assert_eq!(listed["status"].as_str(), Some("unverified"));
        assert_eq!(listed["verification_status"].as_str(), Some("unverified"));

        let summary = harness
            .client
            .get(format!("{}/api/routines/summary", harness.base_url()))
            .bearer_auth(&harness.auth_token)
            .send()
            .await
            .expect("summary request failed")
            .error_for_status()
            .expect("summary non-2xx")
            .json::<serde_json::Value>()
            .await
            .expect("invalid summary response");
        assert_eq!(summary["unverified"].as_u64(), Some(2));

        let detail = harness
            .client
            .get(format!(
                "{}/api/routines/{}",
                harness.base_url(),
                routine_id
            ))
            .bearer_auth(&harness.auth_token)
            .send()
            .await
            .expect("detail request failed")
            .error_for_status()
            .expect("detail non-2xx")
            .json::<serde_json::Value>()
            .await
            .expect("invalid detail response");
        assert_eq!(detail["status"].as_str(), Some("unverified"));
        assert_eq!(detail["verification_status"].as_str(), Some("unverified"));

        harness.shutdown().await;
        mock.shutdown().await;
    }

    /// Regression test for issue #1076: web API toggle must immediately
    /// invalidate the in-memory event cache so disabled routines stop firing.
    #[tokio::test]
    async fn web_toggle_disables_system_event_routine_without_restart() {
        let mock = MockOpenAiServerBuilder::new()
            .with_rule(MockOpenAiRule::on_user_contains(
                "create webhook routine",
                MockOpenAiResponse::ToolCalls(vec![MockToolCall::new(
                    "call_create_webhook_1",
                    "routine_create",
                    serde_json::json!({
                        "name": "wf-toggle-system-event",
                        "description": "System event toggle regression test",
                        "trigger_type": "system_event",
                        "event_source": "github",
                        "event_type": "issue.opened",
                        "event_filters": {"repository": "nearai/ironclaw"},
                        "action_type": "lightweight",
                        "prompt": "summarize issue"
                    }),
                )]),
            ))
            .with_default_response(MockOpenAiResponse::Text("ack".to_string()))
            .start()
            .await;

        let harness =
            GatewayWorkflowHarness::start_openai_compatible(&mock.openai_base_url(), "mock-model")
                .await;

        let thread_id = harness.create_thread().await;
        harness
            .send_chat(&thread_id, "create webhook routine")
            .await;
        harness
            .wait_for_turns(&thread_id, 1, Duration::from_secs(10))
            .await;

        let mut routine = None;
        for _ in 0..30 {
            routine = harness.routine_by_name("wf-toggle-system-event").await;
            if routine.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let routine = routine.expect("routine should exist after retries");
        let routine_id = routine
            .get("id")
            .and_then(|v| v.as_str())
            .expect("routine id missing");

        let runs_before = harness.routine_runs(routine_id).await;
        let before_count = runs_before["runs"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or_default();

        // Disable through web API (non-tool mutation path).
        harness
            .client
            .post(format!(
                "{}/api/routines/{routine_id}/toggle",
                harness.base_url()
            ))
            .bearer_auth(&harness.auth_token)
            .json(&serde_json::json!({ "enabled": false }))
            .send()
            .await
            .expect("disable toggle request failed")
            .error_for_status()
            .expect("disable toggle non-2xx");

        // Fire a webhook that would match the now-disabled routine.
        let hook = harness
            .github_webhook(
                "issues",
                serde_json::json!({
                    "action": "opened",
                    "repository": {"full_name": "nearai/ironclaw"},
                    "issue": {"number": 881, "title": "Toggle disable regression"}
                }),
            )
            .await;
        assert_eq!(hook["status"], "accepted");
        assert_eq!(hook["emitted_events"], 1);
        assert_eq!(
            hook["fired_routines"].as_u64().unwrap_or(0),
            0,
            "disabled routine should not fire after web toggle"
        );

        let runs_after = harness.routine_runs(routine_id).await;
        let after_count = runs_after["runs"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or_default();
        assert_eq!(
            after_count, before_count,
            "run count should not increase for disabled routine"
        );

        harness.shutdown().await;
        mock.shutdown().await;
    }

    #[tokio::test]
    async fn cancelling_running_full_job_routine_finalizes_run() {
        let mock = MockOpenAiServerBuilder::new()
            .with_rule(MockOpenAiRule::on_user_contains(
                "create blocking routine",
                MockOpenAiResponse::ToolCalls(vec![MockToolCall::new(
                    "call_create_blocking_1",
                    "routine_create",
                    serde_json::json!({
                        "name": "wf-cancel-full-job",
                        "description": "Full-job cancellation regression test",
                        "trigger_type": "manual",
                        "action_type": "full_job",
                        "prompt": "Use block_once then finish.",
                        "max_iterations": 10
                    }),
                )]),
            ))
            .with_rule(MockOpenAiRule::on_user_contains(
                "use block_once then finish",
                MockOpenAiResponse::ToolCalls(vec![MockToolCall::new(
                    "call_block_once_1",
                    "block_once",
                    serde_json::json!({}),
                )]),
            ))
            .with_default_response(MockOpenAiResponse::Text("ack".to_string()))
            .start()
            .await;

        let harness =
            GatewayWorkflowHarness::start_openai_compatible(&mock.openai_base_url(), "mock-model")
                .await;
        harness.register_tool(Arc::new(BlockingTool)).await;

        let thread_id = harness.create_thread().await;
        harness
            .send_chat(&thread_id, "create blocking routine")
            .await;
        harness
            .wait_for_turns(&thread_id, 1, Duration::from_secs(10))
            .await;

        let routine = harness
            .routine_by_name("wf-cancel-full-job")
            .await
            .expect("blocking full-job routine should exist");
        let routine_id = routine["id"].as_str().expect("routine id missing");

        let trigger = harness
            .client
            .post(format!(
                "{}/api/routines/{routine_id}/trigger",
                harness.base_url()
            ))
            .bearer_auth(&harness.auth_token)
            .send()
            .await
            .expect("trigger request failed")
            .error_for_status()
            .expect("trigger non-2xx")
            .json::<serde_json::Value>()
            .await
            .expect("invalid trigger response");
        assert_eq!(trigger["status"].as_str(), Some("triggered"));

        let mut job_id = None;
        for _ in 0..100 {
            let runs = harness.routine_runs(routine_id).await;
            if let Some(found_job_id) = runs["runs"]
                .as_array()
                .and_then(|runs| runs.first())
                .and_then(|run| run["job_id"].as_str())
            {
                job_id = Some(found_job_id.to_string());
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let job_id = job_id.expect("routine run should be linked to a job");

        let mut saw_job_execution_request = false;
        for _ in 0..50 {
            if mock.requests().await.len() >= 2 {
                saw_job_execution_request = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            saw_job_execution_request,
            "job should issue an execution LLM request before cancellation"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;

        let cancel_response = harness
            .client
            .post(format!("{}/api/jobs/{job_id}/cancel", harness.base_url()))
            .bearer_auth(&harness.auth_token)
            .send()
            .await
            .expect("cancel request failed")
            .error_for_status()
            .expect("cancel non-2xx")
            .json::<serde_json::Value>()
            .await
            .expect("invalid cancel response");
        assert_eq!(cancel_response["status"].as_str(), Some("cancelled"));

        let mut final_job = None;
        for _ in 0..100 {
            let job = harness
                .client
                .get(format!("{}/api/jobs/{job_id}", harness.base_url()))
                .bearer_auth(&harness.auth_token)
                .send()
                .await
                .expect("final job detail request failed")
                .error_for_status()
                .expect("final job detail non-2xx")
                .json::<serde_json::Value>()
                .await
                .expect("invalid final job detail");
            if job["state"].as_str() == Some("cancelled") {
                final_job = Some(job);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let final_job = final_job.expect("job should reach cancelled state");
        assert_eq!(final_job["state"].as_str(), Some("cancelled"));

        let mut final_run = None;
        for _ in 0..100 {
            let runs = harness.routine_runs(routine_id).await;
            let run = runs["runs"]
                .as_array()
                .and_then(|runs| runs.first())
                .cloned()
                .expect("routine run should exist");
            if run["status"].as_str() != Some("running") {
                final_run = Some(run);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let final_run = final_run.expect("routine run should reach terminal state");
        assert_eq!(final_run["status"].as_str(), Some("failed"));
        assert_eq!(final_run["job_id"].as_str(), Some(job_id.as_str()));
        assert!(
            final_run["result_summary"]
                .as_str()
                .is_some_and(|summary| summary.contains("finished (failed)")),
            "expected failed routine summary, got {final_run}"
        );

        harness.shutdown().await;
        mock.shutdown().await;
    }
}

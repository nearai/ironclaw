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
    use std::time::Duration;

    use crate::support::gateway_workflow_harness::GatewayWorkflowHarness;
    use crate::support::mock_openai_server::{
        MockOpenAiResponse, MockOpenAiRule, MockOpenAiServerBuilder, MockToolCall,
    };

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

        tokio::time::sleep(Duration::from_millis(500)).await;
        let runs_after = harness.routine_runs(routine_id).await;
        let after_count = runs_after["runs"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or_default();
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
}

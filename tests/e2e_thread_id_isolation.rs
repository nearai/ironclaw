//! E2E regression test: forged thread IDs must not cross user boundaries.
//!
//! Demonstrates that a client cannot provide another user's conversation UUID
//! and get that history hydrated into prompt context or written into.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use ironclaw::channels::{IncomingMessage, OutgoingResponse};
    use uuid::Uuid;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::{LlmTrace, TraceResponse, TraceStep};

    fn assert_safe_thread_rejection(response: &OutgoingResponse) {
        let msg = response.content.to_lowercase();
        assert!(
            msg.contains("thread") && (msg.contains("invalid") || msg.contains("unauthorized")),
            "expected safe thread-id rejection response, got: {}",
            response.content
        );
    }

    #[tokio::test]
    async fn forged_existing_foreign_thread_id_is_rejected_without_hydration_or_persistence() {
        let trace = LlmTrace::single_turn(
            "thread-id-isolation",
            "attacker turn",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "safe response".to_string(),
                    input_tokens: 12,
                    output_tokens: 4,
                },
                expected_tool_results: Vec::new(),
            }],
        );

        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        let foreign_thread_id = Uuid::new_v4();
        let marker = format!("FOREIGN-MARKER-{}", Uuid::new_v4());
        let store = rig.database();
        assert!(
            store
                .ensure_conversation(
                    foreign_thread_id,
                    "gateway",
                    "victim-user",
                    None,
                    Some("gateway")
                )
                .await
                .expect("failed to create victim conversation"),
            "test setup failed: victim conversation was not created"
        );
        store
            .add_conversation_message(
                foreign_thread_id,
                "user",
                &format!("victim-only secret marker: {marker}"),
            )
            .await
            .expect("failed to seed victim conversation message");

        let before_messages = store
            .list_conversation_messages(foreign_thread_id)
            .await
            .expect("failed to read victim conversation before forged send");
        assert!(
            before_messages.iter().any(|m| m.content.contains(&marker)),
            "test setup failed: victim marker message missing"
        );
        let before_len = before_messages.len();

        let forged = IncomingMessage::new("test", "test-user", "attacker turn")
            .with_thread(foreign_thread_id.to_string());
        rig.send_incoming(forged).await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(20)).await;
        assert_eq!(
            responses.len(),
            1,
            "expected one assistant response for forged-thread request"
        );
        assert_safe_thread_rejection(&responses[0]);

        let captured = rig.captured_llm_requests();
        assert!(
            captured.is_empty(),
            "forged thread-id request should be rejected before any LLM call"
        );
        let prompt_dump = captured
            .iter()
            .flat_map(|req| req.iter().map(|m| m.content.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !prompt_dump.contains(&marker),
            "forged thread_id leaked foreign marker into LLM prompt context: {prompt_dump}"
        );

        let after_messages = store
            .list_conversation_messages(foreign_thread_id)
            .await
            .expect("failed to read victim conversation after forged send");
        assert_eq!(
            after_messages.len(),
            before_len,
            "forged thread_id wrote new messages into victim conversation"
        );
        assert!(
            after_messages
                .iter()
                .all(|m| m.content != "attacker turn" && m.content != "safe response"),
            "forged request content was persisted to victim conversation"
        );

        rig.shutdown();
    }

    #[tokio::test]
    async fn forged_nonexistent_thread_id_is_rejected_and_followup_request_still_works() {
        let trace = LlmTrace::single_turn(
            "thread-id-isolation-nonexistent",
            "real follow-up turn",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "safe response".to_string(),
                    input_tokens: 12,
                    output_tokens: 4,
                },
                expected_tool_results: Vec::new(),
            }],
        );

        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        let forged_thread_id = Uuid::new_v4();
        let store = rig.database();

        let forged = IncomingMessage::new("test", "test-user", "attacker turn")
            .with_thread(forged_thread_id.to_string());
        rig.send_incoming(forged).await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(20)).await;
        assert_eq!(
            responses.len(),
            1,
            "expected one response for forged nonexistent-thread request"
        );
        assert_safe_thread_rejection(&responses[0]);
        assert!(
            rig.captured_llm_requests().is_empty(),
            "forged nonexistent thread-id request should be rejected before any LLM call"
        );
        assert!(
            store
                .get_conversation_metadata(forged_thread_id)
                .await
                .expect("get metadata for forged thread id")
                .is_none(),
            "forged nonexistent thread id must not create a conversation row"
        );

        rig.send_message("real follow-up turn").await;
        let responses = rig.wait_for_responses(2, Duration::from_secs(20)).await;
        assert_eq!(
            responses.len(),
            2,
            "expected follow-up response after rejection"
        );
        assert_eq!(
            responses[1].content, "safe response",
            "follow-up valid request should still be handled normally"
        );
        assert_eq!(
            rig.captured_llm_requests().len(),
            1,
            "only follow-up request should reach LLM"
        );

        rig.shutdown();
    }
}

//! E2E regression test: forged thread IDs must not cross user boundaries.
//!
//! Demonstrates that a client cannot provide another user's conversation UUID
//! and get that history hydrated into prompt context or written into.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use ironclaw::channels::IncomingMessage;
    use uuid::Uuid;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::{LlmTrace, TraceResponse, TraceStep};

    #[tokio::test]
    async fn forged_thread_id_does_not_hydrate_or_persist_cross_user_data() {
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

        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .build()
            .await;

        let foreign_thread_id = Uuid::new_v4();
        let marker = format!("FOREIGN-MARKER-{}", Uuid::new_v4());
        let store = rig.database();
        store
            .ensure_conversation(foreign_thread_id, "gateway", "victim-user", None)
            .await
            .expect("failed to create victim conversation");
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
            .with_thread(&foreign_thread_id.to_string());
        rig.send_incoming(forged).await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(20)).await;
        assert_eq!(
            responses.len(),
            1,
            "expected one assistant response for forged-thread request"
        );

        let captured = rig.captured_llm_requests();
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
}

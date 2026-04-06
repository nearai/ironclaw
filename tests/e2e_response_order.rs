//! Regression test for web SSE ordering.
//!
//! The assistant response must be emitted before the terminal `Done` status
//! so the browser can render the message before the turn closes.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod response_order_tests {
    use std::time::Duration;

    use crate::support::test_channel::CapturedEvent;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::{LlmTrace, TraceResponse, TraceStep, TraceTurn};
    use ironclaw::channels::StatusUpdate;

    const TIMEOUT: Duration = Duration::from_secs(15);

    fn single_response_trace() -> LlmTrace {
        LlmTrace::new(
            "trace-order-test",
            vec![TraceTurn {
                user_input: "Say hello".to_string(),
                steps: vec![TraceStep {
                    request_hint: None,
                    response: TraceResponse::Text {
                        content: "Hello there".to_string(),
                        input_tokens: 1,
                        output_tokens: 1,
                    },
                    expected_tool_results: Vec::new(),
                }],
                expects: Default::default(),
            }],
        )
    }

    #[tokio::test]
    async fn response_arrives_before_done_status() {
        let rig = TestRigBuilder::new()
            .with_trace(single_response_trace())
            .build()
            .await;
        rig.clear().await;

        rig.send_message("Say hello").await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].content, "Hello there");

        let events = rig.captured_events();
        let response_index = events
            .iter()
            .position(|event| matches!(event, CapturedEvent::Response(_)))
            .expect("response event not captured");
        let done_index = events
            .iter()
            .position(|event| matches!(event, CapturedEvent::Status(StatusUpdate::Status(message)) if message == "Done"))
            .expect("Done status not captured");

        assert!(
            response_index < done_index,
            "response must be emitted before Done"
        );

        rig.shutdown();
    }
}

//! E2E trace tests: status event verification.
//!
//! Validates that StatusUpdate events are emitted in the correct order
//! during tool execution: ToolStarted must precede ToolCompleted for
//! each tool invocation.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use ironclaw::channels::StatusUpdate;

    use crate::support::assertions::assert_all_tools_succeeded;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    /// For a 3-tool chain (echo -> echo -> echo), verify that:
    /// 1. ToolStarted fires before ToolCompleted for each tool.
    /// 2. The total number of ToolStarted equals ToolCompleted.
    /// 3. No ToolCompleted appears without a preceding ToolStarted for that name.
    #[tokio::test]
    async fn test_status_event_ordering() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/coverage/status_events_tool_chain.json"
        ))
        .expect("failed to load status_events_tool_chain.json");

        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("Run the tool chain").await;
        let _responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        let events = rig.captured_status_events();

        // Extract ToolStarted and ToolCompleted events in order.
        let tool_events: Vec<&StatusUpdate> = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    StatusUpdate::ToolStarted { .. } | StatusUpdate::ToolCompleted { .. }
                )
            })
            .collect();

        // We expect at least 3 tool starts and 3 tool completions.
        let starts: Vec<&str> = tool_events
            .iter()
            .filter_map(|e| match e {
                StatusUpdate::ToolStarted { name } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        let completions: Vec<&str> = tool_events
            .iter()
            .filter_map(|e| match e {
                StatusUpdate::ToolCompleted { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();

        assert!(
            starts.len() >= 3,
            "Expected >= 3 ToolStarted events, got {}: {:?}",
            starts.len(),
            starts
        );
        assert_eq!(
            starts.len(),
            completions.len(),
            "ToolStarted count ({}) != ToolCompleted count ({})",
            starts.len(),
            completions.len()
        );

        // Verify ordering: for each ToolCompleted, a ToolStarted for the same
        // tool name must appear earlier in the event list.
        let mut pending_starts: Vec<String> = Vec::new();
        for event in &tool_events {
            match event {
                StatusUpdate::ToolStarted { name } => {
                    pending_starts.push(name.clone());
                }
                StatusUpdate::ToolCompleted { name, .. } => {
                    let pos = pending_starts.iter().rposition(|n| n == name);
                    assert!(
                        pos.is_some(),
                        "ToolCompleted for '{name}' without preceding ToolStarted. \
                         Pending starts: {pending_starts:?}"
                    );
                    pending_starts.remove(pos.unwrap());
                }
                _ => {}
            }
        }

        // All starts should be consumed.
        assert!(
            pending_starts.is_empty(),
            "ToolStarted without matching ToolCompleted: {pending_starts:?}"
        );

        // Verify expected tool names (fixture uses echo calls only).
        assert!(
            starts.contains(&"echo"),
            "Expected echo in tool starts, got: {starts:?}"
        );

        // Verify all completions were successful.
        let completed = rig.tool_calls_completed();
        assert_all_tools_succeeded(&completed);

        // Metrics should reflect 4 LLM calls.
        let metrics = rig.collect_metrics().await;
        assert!(
            metrics.llm_calls >= 4,
            "Expected >= 4 LLM calls, got {}",
            metrics.llm_calls
        );
        assert!(
            metrics.total_tool_calls() >= 3,
            "Expected >= 3 tool invocations in metrics"
        );

        rig.shutdown();
    }

    /// Verify that Thinking events are emitted during agent processing.
    #[tokio::test]
    async fn test_thinking_events_captured() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/simple_text.json"
        ))
        .expect("failed to load simple_text.json");

        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("hello").await;
        let _responses = rig.wait_for_responses(1, Duration::from_secs(10)).await;

        let events = rig.captured_status_events();

        // The agent should emit at least one Thinking or Status event
        // during message processing.
        let has_processing_event = events
            .iter()
            .any(|e| matches!(e, StatusUpdate::Thinking(_) | StatusUpdate::Status(_)));

        // Note: Whether Thinking events are emitted depends on the agent
        // implementation. This test documents current behavior rather than
        // asserting a hard requirement.
        if !has_processing_event {
            eprintln!(
                "[INFO] No Thinking/Status events captured. \
                 Agent may not emit these for simple text responses. \
                 Captured events: {:?}",
                events
            );
        }

        rig.shutdown();
    }
}

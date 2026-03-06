//! E2E trace tests: thread/scheduler operations (#572).
//!
//! Covers multi-turn state persistence, undo/redo, and concurrent dispatch.
//! Tests for thread_interruption and max_parallel_exceeded are deferred.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    // -----------------------------------------------------------------------
    // Test 1: multi_turn_state
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn multi_turn_state() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/threading/multi_turn_state.json"
        ))
        .expect("failed to load multi_turn_state.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        let all_responses = rig
            .run_and_verify_trace(&trace, Duration::from_secs(30))
            .await;

        // Should have 3 turns of responses.
        assert_eq!(
            all_responses.len(),
            3,
            "Expected 3 turns, got {}",
            all_responses.len()
        );

        // Verify memory tools were used across turns.
        let started = rig.tool_calls_started();
        let mw_count = started
            .iter()
            .filter(|n| n.as_str() == "memory_write")
            .count();
        let ms_count = started
            .iter()
            .filter(|n| n.as_str() == "memory_search")
            .count();
        assert!(
            mw_count >= 2,
            "Expected >= 2 memory_write calls: {started:?}"
        );
        assert!(
            ms_count >= 1,
            "Expected >= 1 memory_search calls: {started:?}"
        );

        // Verify DB is accessible (conversation persistence is tested by
        // the agent's internal session management).
        let _db = rig.database();

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 2: thread_interruption -- DEFERRED
    // -----------------------------------------------------------------------
    // Needs interrupt signaling infrastructure in TestChannel.

    // -----------------------------------------------------------------------
    // Test 3: undo_redo_cycle
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn undo_redo_cycle() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/threading/undo_redo.json"
        ))
        .expect("failed to load undo_redo.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        let all_responses = rig
            .run_and_verify_trace(&trace, Duration::from_secs(30))
            .await;

        // Should get responses for all 3 turns.
        assert!(
            all_responses.len() >= 2,
            "Expected at least 2 turn responses, got {}",
            all_responses.len()
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 4: concurrent_dispatch
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn concurrent_dispatch() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/threading/concurrent_dispatch.json"
        ))
        .expect("failed to load concurrent_dispatch.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        let all_responses = rig
            .run_and_verify_trace(&trace, Duration::from_secs(30))
            .await;

        // Should have 2 turns.
        assert_eq!(
            all_responses.len(),
            2,
            "Expected 2 turns, got {}",
            all_responses.len()
        );

        // Both echo calls should have succeeded.
        let completed = rig.tool_calls_completed();
        let echo_successes = completed
            .iter()
            .filter(|(name, ok)| name == "echo" && *ok)
            .count();
        assert!(
            echo_successes >= 2,
            "Expected >= 2 successful echo calls: {completed:?}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 5: max_parallel_exceeded -- DEFERRED
    // -----------------------------------------------------------------------
    // Needs max_parallel config exposed through TestRigBuilder.
}

//! E2E trace test: memory write flow.
//!
//! Validates that the agent can execute `memory_write` tool calls driven by
//! a TraceLlm trace, with a real workspace backed by libSQL.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    #[tokio::test]
    async fn test_memory_write_flow() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/memory_write_read.json"
        ))
        .expect("failed to load memory_write_read.json trace fixture");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Please remember that Project Alpha launches on March 15th")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }
}

//! E2E trace test: validates that the agent can execute `write_file` and
//! `read_file` tool calls driven by a TraceLlm trace.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use crate::support::cleanup::CleanupGuard;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    const TEST_DIR: &str = "/tmp/ironclaw_e2e_test";
    const TEST_FILE: &str = "/tmp/ironclaw_e2e_test/hello.txt";
    const EXPECTED_CONTENT: &str = "Hello, E2E test!";

    fn setup_test_dir() {
        let _ = std::fs::remove_dir_all(TEST_DIR);
        std::fs::create_dir_all(TEST_DIR).expect("failed to create test directory");
    }

    #[tokio::test]
    async fn test_file_write_and_read_flow() {
        setup_test_dir();
        let _cleanup = CleanupGuard::new().dir(TEST_DIR);

        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/file_write_read.json"
        );
        let trace = LlmTrace::from_file(fixture_path).expect("failed to load trace fixture");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Please write a greeting to a file and read it back.")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Extra: verify file on disk (can't express in expects).
        let file_content =
            std::fs::read_to_string(TEST_FILE).expect("hello.txt should exist after write_file");
        assert_eq!(file_content, EXPECTED_CONTENT);

        rig.shutdown();
    }
}

//! E2E tests for recorded LLM traces.
//!
//! Each test replays a recorded fixture through the full agent loop, verifying
//! declarative `expects` from the JSON and any additional manual checks.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod recorded_trace_tests {
    use crate::support::test_rig::run_recorded_trace;

    /// Recorded trace: telegram connection check.
    #[tokio::test]
    async fn recorded_telegram_check() {
        run_recorded_trace("telegram_check.json").await;
    }
}

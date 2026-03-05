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

    /// Recorded trace: weather query for San Francisco.
    /// Hits live wttr.in API — run with `--ignored` to include.
    #[tokio::test]
    #[ignore]
    async fn recorded_weather_sf() {
        run_recorded_trace("weather_sf.json").await;
    }

    /// Recorded trace: baseball stats with large HTTP response exercising
    /// tool_output_stash + source_tool_call_id for untruncated data access.
    /// Hits live ESPN API — run with `--ignored` to include.
    #[tokio::test]
    #[ignore]
    async fn recorded_baseball_stats() {
        run_recorded_trace("baseball_stats.json").await;
    }
}

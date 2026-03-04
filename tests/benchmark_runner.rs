//! Integration test for the benchmark runner.
//!
//! Requires a real LLM provider configured via environment variables.
//! Run with: cargo test --features "libsql,benchmark" --test benchmark_runner -- --ignored

#[cfg(all(feature = "libsql", feature = "benchmark"))]
mod tests {
    use std::sync::Arc;

    use ironclaw::benchmark::baseline;
    use ironclaw::benchmark::report::format_report;
    use ironclaw::benchmark::runner::BenchmarkConfig;
    use ironclaw::llm::{SessionConfig, SessionManager, create_llm_provider};

    /// Run the full benchmark suite with a real LLM.
    #[tokio::test]
    #[ignore] // Requires LLM API keys
    async fn run_full_benchmark() {
        // Load config from environment.
        let config = ironclaw::config::Config::from_env()
            .await
            .expect("Config::from_env failed -- set LLM env vars");

        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let llm = create_llm_provider(&config.llm, session)
            .expect("Failed to create LLM provider -- check env vars");

        let bench_config = BenchmarkConfig::default();
        let result = ironclaw::benchmark::runner::run_all(&bench_config, llm)
            .await
            .expect("Benchmark run failed");

        // Save result.
        let result_path = baseline::save_result(&result).expect("Failed to save result");
        eprintln!("Results saved to: {result_path}");

        // Load baseline and compare.
        let baseline_result = baseline::load_baseline().expect("Failed to load baseline");
        let report = format_report(&result, baseline_result.as_ref());
        eprintln!("\n{report}");

        // The test itself just verifies the runner doesn't crash.
        assert!(
            !result.scenarios.is_empty(),
            "Expected at least one scenario to run"
        );
    }
}

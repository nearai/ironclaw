//! E2E test for per-user message concurrency limiting.
//!
//! Verifies that a single user cannot exceed their per-user permit limit
//! even when the global limit would allow more concurrent messages.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod per_user_concurrency_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use rust_decimal::Decimal;

    use crate::support::test_rig::TestRigBuilder;
    use ironclaw::channels::IncomingMessage;
    use ironclaw::config::Config;
    use ironclaw::llm::{
        CompletionRequest, CompletionResponse, FinishReason, LlmError, LlmProvider,
        ToolCompletionRequest, ToolCompletionResponse,
    };

    /// An LLM provider that sleeps on every completion call, tracking
    /// the peak number of concurrent invocations.
    struct SlowLlm {
        delay: Duration,
        concurrent: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }

    impl SlowLlm {
        fn new(delay: Duration) -> (Self, Arc<AtomicUsize>) {
            let peak = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    delay,
                    concurrent: Arc::new(AtomicUsize::new(0)),
                    peak: Arc::clone(&peak),
                },
                peak,
            )
        }
    }

    #[async_trait]
    impl LlmProvider for SlowLlm {
        fn model_name(&self) -> &str {
            "slow-test-model"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let current = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(current, Ordering::SeqCst);

            tokio::time::sleep(self.delay).await;

            self.concurrent.fetch_sub(1, Ordering::SeqCst);

            Ok(CompletionResponse {
                content: "Done.".to_string(),
                input_tokens: 10,
                output_tokens: 2,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            let current = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(current, Ordering::SeqCst);

            tokio::time::sleep(self.delay).await;

            self.concurrent.fetch_sub(1, Ordering::SeqCst);

            Ok(ToolCompletionResponse {
                content: Some("Done.".to_string()),
                tool_calls: Vec::new(),
                input_tokens: 10,
                output_tokens: 2,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }
    }

    /// An LLM provider that sleeps on every completion call, tracking
    /// the peak number of concurrent invocations with per-user granularity.
    struct PerUserSlowLlm {
        delay: Duration,
        global_concurrent: Arc<AtomicUsize>,
        global_peak: Arc<AtomicUsize>,
    }

    impl PerUserSlowLlm {
        fn new(delay: Duration) -> (Self, Arc<AtomicUsize>) {
            let global_peak = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    delay,
                    global_concurrent: Arc::new(AtomicUsize::new(0)),
                    global_peak: Arc::clone(&global_peak),
                },
                global_peak,
            )
        }

        fn track_enter(&self) {
            let current = self.global_concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            self.global_peak.fetch_max(current, Ordering::SeqCst);
        }

        fn track_exit(&self) {
            self.global_concurrent.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl LlmProvider for PerUserSlowLlm {
        fn model_name(&self) -> &str {
            "slow-test-model"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            self.track_enter();
            tokio::time::sleep(self.delay).await;
            self.track_exit();
            Ok(CompletionResponse {
                content: "Done.".to_string(),
                input_tokens: 10,
                output_tokens: 2,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            self.track_enter();
            tokio::time::sleep(self.delay).await;
            self.track_exit();
            Ok(ToolCompletionResponse {
                content: Some("Done.".to_string()),
                tool_calls: Vec::new(),
                input_tokens: 10,
                output_tokens: 2,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }
    }

    const TIMEOUT: Duration = Duration::from_secs(30);

    /// Verify that a single user's concurrent messages are bounded by
    /// `max_parallel_threads_per_user`, not the larger global limit.
    ///
    /// Setup: global=4, per_user=2, send 4 messages.
    /// Expected: at most 2 LLM calls run concurrently (the per-user cap).
    #[tokio::test]
    async fn single_user_bounded_by_per_user_limit() {
        let (slow_llm, peak) = SlowLlm::new(Duration::from_secs(1));

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let db_path = temp_dir.path().join("concurrency_test.db");
        let skills_dir = temp_dir.path().join("skills");
        let installed_skills_dir = temp_dir.path().join("installed_skills");
        let _ = std::fs::create_dir_all(&skills_dir);
        let _ = std::fs::create_dir_all(&installed_skills_dir);
        let mut config = Config::for_testing(db_path, skills_dir, installed_skills_dir);
        config.agent.max_parallel_threads = 4;
        config.agent.max_parallel_threads_per_user = Some(2);

        let rig = TestRigBuilder::new()
            .with_config(config)
            .with_llm(Arc::new(slow_llm))
            .build()
            .await;
        rig.clear().await;

        // Fire 4 messages to different threads. Per-user limit (2) should
        // prevent more than 2 from being processed simultaneously, even though
        // they target different threads and the global limit allows 4.
        for i in 0..4 {
            let msg = IncomingMessage::new("test", "test-user", format!("Message {i}"))
                .with_thread(format!("thread-{i}"));
            rig.send_incoming(msg).await;
        }

        // Wait for all 4 responses (each takes ~1s with 2-wide concurrency = ~2s).
        let responses = rig.wait_for_responses(4, TIMEOUT).await;
        assert_eq!(
            responses.len(),
            4,
            "expected 4 responses, got {}",
            responses.len()
        );

        let observed_peak = peak.load(Ordering::SeqCst);
        assert!(
            observed_peak <= 2,
            "per-user limit is 2 but observed peak concurrency of {observed_peak}"
        );
        // Also verify some parallelism actually happened (not fully serial).
        assert!(
            observed_peak >= 2,
            "expected at least 2-wide parallelism but observed peak of {observed_peak}"
        );

        rig.shutdown();
    }

    /// Verify that user B is not starved when user A has saturated their
    /// per-user limit. Both users should make progress concurrently.
    ///
    /// Setup: global=4, per_user=2.
    /// User A sends 4 messages (saturates their per-user limit of 2).
    /// User B sends 1 message immediately after.
    /// Expected: all 5 complete, global peak ≤ 3 (A's 2 + B's 1), and
    /// total wall-clock time is well under the serial case (5s).
    #[tokio::test]
    async fn second_user_not_starved_by_first() {
        let (slow_llm, global_peak) = PerUserSlowLlm::new(Duration::from_secs(1));

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let db_path = temp_dir.path().join("multi_user_test.db");
        let skills_dir = temp_dir.path().join("skills");
        let installed_skills_dir = temp_dir.path().join("installed_skills");
        let _ = std::fs::create_dir_all(&skills_dir);
        let _ = std::fs::create_dir_all(&installed_skills_dir);
        let mut config = Config::for_testing(db_path, skills_dir, installed_skills_dir);
        config.agent.max_parallel_threads = 4;
        config.agent.max_parallel_threads_per_user = Some(2);

        let rig = TestRigBuilder::new()
            .with_config(config)
            .with_llm(Arc::new(slow_llm))
            .build()
            .await;
        rig.clear().await;

        let start = std::time::Instant::now();

        // User A: 4 messages to 4 different threads.
        for i in 0..4 {
            let msg = IncomingMessage::new("test", "alice", format!("Alice msg {i}"))
                .with_thread(format!("alice-thread-{i}"));
            rig.send_incoming(msg).await;
        }

        // User B: 1 message to its own thread.
        let msg = IncomingMessage::new("test", "bob", "Bob msg 0".to_string())
            .with_thread("bob-thread-0".to_string());
        rig.send_incoming(msg).await;

        // Wait for all 5 responses.
        let responses = rig.wait_for_responses(5, TIMEOUT).await;
        let elapsed = start.elapsed();

        assert_eq!(
            responses.len(),
            5,
            "expected 5 responses, got {}",
            responses.len()
        );

        let peak = global_peak.load(Ordering::SeqCst);

        // Per-user limit enforced: at most A(2) + B(1) = 3 concurrent.
        assert!(
            peak <= 3,
            "per-user limits should cap global peak at 3, but observed {peak}"
        );

        // Some parallelism occurred (not fully serial).
        assert!(
            peak >= 2,
            "expected at least 2-wide parallelism but observed peak of {peak}"
        );

        // Wall-clock sanity: 5 messages at 1s each serial = 5s.
        // With per-user=2, A's 4 messages take ~2s; B runs concurrently.
        // Total should be well under 5s.
        assert!(
            elapsed < Duration::from_secs(4),
            "expected < 4s wall-clock but took {elapsed:?} — messages may not be running concurrently"
        );

        rig.shutdown();
    }
}

//! Dual-mode test harness: live LLM calls with recording, or replay from saved traces.
//!
//! # Modes
//!
//! - **Live mode** (`IRONCLAW_LIVE_TEST=1`): Uses real LLM provider from
//!   `~/.ironclaw/.env`, records traces to `tests/fixtures/llm_traces/live/`.
//! - **Replay mode** (default): Loads saved trace JSON, deterministic, no API keys.
//!
//! # Usage
//!
//! ```rust,ignore
//! let harness = LiveTestHarnessBuilder::new("my_test")
//!     .with_max_tool_iterations(30)
//!     .with_timeout(Duration::from_secs(120))
//!     .build()
//!     .await;
//!
//! harness.rig().send_message("do something").await;
//! let responses = harness.rig().wait_for_responses(1, Duration::from_secs(120)).await;
//!
//! // LLM judge (live mode only, returns None in replay)
//! if let Some(verdict) = harness.judge(&texts, "criteria here").await {
//!     assert!(verdict.pass, "Judge: {}", verdict.reasoning);
//! }
//!
//! harness.finish().await;
//! ```

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ironclaw::llm::recording::RecordingLlm;
use ironclaw::llm::{ChatMessage, CompletionRequest, LlmProvider, SessionConfig, SessionManager};

use crate::support::test_rig::{TestRig, TestRigBuilder};
use crate::support::trace_llm::LlmTrace;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Whether the harness is running live (real LLM) or replaying a saved trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestMode {
    Live,
    Replay,
}

/// Result of an LLM judge evaluation.
pub struct JudgeVerdict {
    pub pass: bool,
    pub reasoning: String,
}

/// A running test harness wrapping a `TestRig` with dual-mode support.
pub struct LiveTestHarness {
    rig: TestRig,
    recording_handle: Option<Arc<RecordingLlm>>,
    judge_llm: Option<Arc<dyn LlmProvider>>,
    test_name: String,
    mode: TestMode,
}

impl LiveTestHarness {
    /// Access the underlying `TestRig` for sending messages and inspecting results.
    pub fn rig(&self) -> &TestRig {
        &self.rig
    }

    /// The mode this harness is running in.
    pub fn mode(&self) -> TestMode {
        self.mode
    }

    /// Use an LLM judge to evaluate collected responses against criteria.
    ///
    /// Returns `None` in replay mode (no judge provider available).
    pub async fn judge(&self, responses: &[String], criteria: &str) -> Option<JudgeVerdict> {
        let provider = self.judge_llm.as_ref()?;
        let joined = responses.join("\n\n---\n\n");
        Some(judge_response(provider.as_ref(), &joined, criteria).await)
    }

    /// Flush the recorded trace (if live mode), save a human-readable session
    /// log, and shut down the agent.
    ///
    /// `user_input` is the message that was sent to the agent.
    /// `responses` are the agent's text responses (from `wait_for_responses`).
    ///
    /// The session log is written to `tests/fixtures/llm_traces/live/{name}.log`.
    pub async fn finish(self, user_input: &str, responses: &[String]) {
        self.save_session_log(user_input, responses);

        if let Some(ref recorder) = self.recording_handle {
            if let Err(e) = recorder.flush().await {
                eprintln!("[LiveTest] WARNING: Failed to flush trace: {e}");
            } else {
                eprintln!("[LiveTest] Trace recorded successfully");
            }
        }
        self.rig.shutdown();
    }

    /// Write a human-readable session log.
    ///
    /// Live mode writes to `tests/fixtures/llm_traces/live/{name}.log` (committed).
    /// Replay mode writes to a temp file so it can be diffed against the live log.
    fn save_session_log(&self, user_input: &str, responses: &[String]) {
        use ironclaw::channels::StatusUpdate;

        let (log_path, live_log_path) = match self.mode {
            TestMode::Live => {
                let p = trace_fixture_path(&self.test_name).with_extension("log");
                (p, None)
            }
            TestMode::Replay => {
                let replay_dir = std::env::temp_dir().join("ironclaw-live-tests");
                let _ = std::fs::create_dir_all(&replay_dir);
                let p = replay_dir.join(format!("{}.replay.log", self.test_name));
                let live = trace_fixture_path(&self.test_name).with_extension("log");
                (p, Some(live))
            }
        };
        let mut log = String::new();

        log.push_str(&format!(
            "# Live Test Session: {}\n# Mode: {:?}\n",
            self.test_name, self.mode,
        ));
        log.push_str(&format!(
            "# LLM calls: {}, Input tokens: {}, Output tokens: {}\n",
            self.rig.llm_call_count(),
            self.rig.total_input_tokens(),
            self.rig.total_output_tokens(),
        ));
        log.push_str(&format!(
            "# Wall time: {:.1}s, Cost: ${:.4}\n",
            self.rig.elapsed_ms() as f64 / 1000.0,
            self.rig.estimated_cost_usd(),
        ));
        log.push_str("# ──────────────────────────────────────────────────\n\n");

        // User input
        log.push_str(&format!("› {user_input}\n"));

        // Tool activity from status events
        for event in self.rig.captured_status_events() {
            match event {
                StatusUpdate::ToolStarted { name } => {
                    log.push_str(&format!("  ● {name}\n"));
                }
                StatusUpdate::ToolCompleted {
                    name,
                    success,
                    error,
                    ..
                } => {
                    if success {
                        log.push_str(&format!("  ✓ {name}\n"));
                    } else {
                        let err = error.as_deref().unwrap_or("unknown error");
                        log.push_str(&format!("  ✗ {name}: {err}\n"));
                    }
                }
                StatusUpdate::ToolResult { name, preview } => {
                    let short = if preview.len() > 200 {
                        format!("{}…", &preview[..200])
                    } else {
                        preview
                    };
                    log.push_str(&format!("    {name} → {short}\n"));
                }
                StatusUpdate::Thinking(msg) => {
                    log.push_str(&format!("  ○ {msg}\n"));
                }
                StatusUpdate::Status(msg) => {
                    log.push_str(&format!("  … {msg}\n"));
                }
                _ => {}
            }
        }

        // Agent response(s)
        log.push_str("────────────────────────────────────────────────────\n");
        for response in responses {
            log.push_str(response);
            log.push('\n');
        }

        if let Err(e) = std::fs::write(&log_path, &log) {
            eprintln!("[LiveTest] WARNING: Failed to write session log: {e}");
        } else {
            eprintln!("[LiveTest] Session log: {}", log_path.display());
            if let Some(live) = live_log_path.filter(|p| p.exists()) {
                eprintln!(
                    "[LiveTest] Diff: diff {} {}",
                    live.display(),
                    log_path.display()
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder for constructing a `LiveTestHarness`.
pub struct LiveTestHarnessBuilder {
    test_name: String,
    max_tool_iterations: usize,
    timeout: Duration,
    engine_v2: Option<bool>,
}

impl LiveTestHarnessBuilder {
    /// Create a new builder for a test with the given name.
    ///
    /// The name determines the trace fixture filename:
    /// `tests/fixtures/llm_traces/live/{test_name}.json`
    pub fn new(test_name: impl Into<String>) -> Self {
        Self {
            test_name: test_name.into(),
            max_tool_iterations: 30,
            timeout: Duration::from_secs(120),
            engine_v2: None,
        }
    }

    /// Set the maximum number of tool iterations per agentic loop invocation.
    pub fn with_max_tool_iterations(mut self, n: usize) -> Self {
        self.max_tool_iterations = n;
        self
    }

    /// Set the default timeout for the test (used as a hint; callers still
    /// pass timeout to `wait_for_responses`).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Force engine v2 on or off, overriding the env-resolved value.
    pub fn with_engine_v2(mut self, enabled: bool) -> Self {
        self.engine_v2 = Some(enabled);
        self
    }

    /// Build the harness, auto-detecting mode from the `IRONCLAW_LIVE_TEST` env var.
    #[cfg(feature = "libsql")]
    pub async fn build(self) -> LiveTestHarness {
        let trace_path = trace_fixture_path(&self.test_name);
        let is_live = std::env::var("IRONCLAW_LIVE_TEST")
            .ok()
            .filter(|v| !v.is_empty() && v != "0")
            .is_some();

        if is_live {
            self.build_live(trace_path).await
        } else {
            self.build_replay(trace_path).await
        }
    }

    #[cfg(feature = "libsql")]
    async fn build_live(self, trace_path: PathBuf) -> LiveTestHarness {
        eprintln!(
            "[LiveTest] Mode: LIVE — recording to {}",
            trace_path.display()
        );

        // Load env from ~/.ironclaw/.env so LLM API keys are available.
        let _ = dotenvy::dotenv();
        ironclaw::bootstrap::load_ironclaw_env();

        // Resolve full config (reads LLM_BACKEND, ENGINE_V2, ALLOW_LOCAL_TOOLS, etc.)
        // This mirrors the exact config the real `ironclaw` binary would use.
        let mut config = ironclaw::config::Config::from_env().await.expect(
            "Failed to load config for live test. \
                 Ensure ~/.ironclaw/.env has valid LLM credentials.",
        );

        // Apply builder overrides.
        if let Some(v2) = self.engine_v2 {
            config.agent.engine_v2 = v2;
        }

        eprintln!(
            "[LiveTest] Config: engine_v2={}, allow_local_tools={}, auto_approve={}",
            config.agent.engine_v2, config.agent.allow_local_tools, config.agent.auto_approve_tools,
        );

        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let (provider, cheap_llm, _) = ironclaw::llm::build_provider_chain(&config.llm, session)
            .await
            .expect("Failed to build LLM provider chain for live test");

        // Wrap with RecordingLlm to capture the trace.
        let model_name = format!("live-{}", self.test_name);
        let recorder = Arc::new(RecordingLlm::new(provider, trace_path, model_name));
        let http_interceptor = recorder.http_interceptor();
        let llm: Arc<dyn LlmProvider> = Arc::clone(&recorder) as Arc<dyn LlmProvider>;

        // Pass the real config so TestRig mirrors real binary behavior:
        // - allow_local_tools controls shell/file tool availability
        // - engine_v2 controls which agentic loop path is used
        // - auto_approve_tools controls approval gates
        let rig = TestRigBuilder::new()
            .with_config(config)
            .with_llm(llm)
            .with_http_interceptor(http_interceptor)
            .with_max_tool_iterations(self.max_tool_iterations)
            .with_auto_approve_tools(true)
            .build()
            .await;

        // Use cheap LLM for judge if available.
        let judge_llm = cheap_llm;

        LiveTestHarness {
            rig,
            recording_handle: Some(recorder),
            judge_llm,
            test_name: self.test_name,
            mode: TestMode::Live,
        }
    }

    #[cfg(feature = "libsql")]
    async fn build_replay(self, trace_path: PathBuf) -> LiveTestHarness {
        eprintln!(
            "[LiveTest] Mode: REPLAY — loading from {}",
            trace_path.display()
        );

        let trace = LlmTrace::from_file(&trace_path).unwrap_or_else(|e| {
            panic!(
                "Failed to load trace fixture '{}': {e}\n\
                 Hint: Run with IRONCLAW_LIVE_TEST=1 to record the trace first.",
                trace_path.display()
            )
        });

        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .with_max_tool_iterations(self.max_tool_iterations)
            .with_auto_approve_tools(true)
            .build()
            .await;

        LiveTestHarness {
            rig,
            recording_handle: None,
            judge_llm: None,
            test_name: self.test_name,
            mode: TestMode::Replay,
        }
    }
}

// ---------------------------------------------------------------------------
// LLM Judge
// ---------------------------------------------------------------------------

/// Use an LLM to evaluate whether a response satisfies test criteria.
///
/// Makes a single LLM call with a structured evaluation prompt.
pub async fn judge_response(
    provider: &dyn LlmProvider,
    agent_response: &str,
    criteria: &str,
) -> JudgeVerdict {
    let prompt = format!(
        "You are a test evaluator for an AI coding assistant. \
         Evaluate whether the assistant's response satisfies the given criteria.\n\n\
         ## Criteria\n{criteria}\n\n\
         ## Response to evaluate\n{agent_response}\n\n\
         Respond with exactly one line in this format:\n\
         PASS: <one-line reasoning>\n\
         or\n\
         FAIL: <one-line reasoning>"
    );

    let request = CompletionRequest::new(vec![ChatMessage::user(&prompt)]);

    match provider.complete(request).await {
        Ok(response) => {
            let trimmed = response.content.trim();
            let pass = trimmed.starts_with("PASS");
            JudgeVerdict {
                pass,
                reasoning: trimmed.to_string(),
            }
        }
        Err(e) => JudgeVerdict {
            pass: false,
            reasoning: format!("Judge LLM call failed: {e}"),
        },
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute the path to a live trace fixture file.
fn trace_fixture_path(test_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/llm_traces/live")
        .join(format!("{test_name}.json"))
}

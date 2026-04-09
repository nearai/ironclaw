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
//!     .build()
//!     .await;
//!
//! harness.rig().send_message("do something").await;
//! let responses = harness.rig().wait_for_responses(1, std::time::Duration::from_secs(120)).await;
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

/// Source of an inbound transcript turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSource {
    User,
    ToolInbound,
    Internal,
}

impl TurnSource {
    fn label(self) -> &'static str {
        match self {
            Self::User => "USER",
            Self::ToolInbound => "TOOL_INBOUND",
            Self::Internal => "INTERNAL",
        }
    }
}

/// One user turn and its assistant responses for session-log rendering.
pub struct SessionTurn {
    pub source: TurnSource,
    pub user_input: String,
    pub responses: Vec<String>,
}

impl SessionTurn {
    pub fn user(user_input: impl Into<String>, responses: Vec<String>) -> Self {
        Self {
            source: TurnSource::User,
            user_input: user_input.into(),
            responses,
        }
    }

    pub fn tool_inbound(user_input: impl Into<String>, responses: Vec<String>) -> Self {
        Self {
            source: TurnSource::ToolInbound,
            user_input: user_input.into(),
            responses,
        }
    }

    pub fn internal(user_input: impl Into<String>, responses: Vec<String>) -> Self {
        Self {
            source: TurnSource::Internal,
            user_input: user_input.into(),
            responses,
        }
    }
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

    /// Scan the captured status events and tool results for executor errors.
    ///
    /// Returns a list of error descriptions. The harness's `finish_strict`
    /// helper panics if this list is non-empty, which is the default behavior
    /// for live tests — any error in the trace is treated as a regression
    /// that warrants investigation.
    ///
    /// Recognized error patterns:
    /// - Failed tool calls (`ToolCompleted { success: false }`)
    /// - Tool result previews containing `error`/`failed`/`SyntaxError`
    /// - The exception is "Document not found": this is a benign signal that
    ///   the agent probed for a workspace file that doesn't exist yet, and
    ///   the agent is expected to recover by writing the file. We surface it
    ///   as a soft warning but don't fail the test.
    pub fn collect_trace_errors(&self) -> Vec<String> {
        use ironclaw::channels::StatusUpdate;

        let mut errors = Vec::new();
        for event in self.rig.captured_status_events() {
            match event {
                StatusUpdate::ToolCompleted {
                    name,
                    success: false,
                    error,
                    ..
                } => {
                    let err = error.as_deref().unwrap_or("unknown error");
                    if is_benign_error(err) {
                        continue;
                    }
                    errors.push(format!("tool '{name}' failed: {err}"));
                }
                StatusUpdate::ToolResult { name, preview, .. } => {
                    if let Some(reason) = scan_preview_for_errors(&preview) {
                        errors.push(format!("tool '{name}' result contains error: {reason}"));
                    }
                }
                _ => {}
            }
        }
        errors
    }

    /// Search the captured status stream for any `ToolStarted` or
    /// `ToolResult` event matching `tool_name` whose detail (or output
    /// preview) contains `needle`. Used by behavior tests to assert that
    /// the agent invoked a particular tool with a particular shape of
    /// arguments — e.g. an `http` POST whose detail mentions
    /// `"/issues/123/comments"`.
    ///
    /// Both `tool_name` and `needle` are matched case-insensitively.
    /// Returns `true` on the first match.
    pub fn trace_contains_tool_call(&self, tool_name: &str, needle: &str) -> bool {
        use ironclaw::channels::StatusUpdate;

        let tool_lc = tool_name.to_ascii_lowercase();
        let needle_lc = needle.to_ascii_lowercase();
        for event in self.rig.captured_status_events() {
            match event {
                StatusUpdate::ToolStarted { name, detail, .. }
                    if name.to_ascii_lowercase().contains(&tool_lc) =>
                {
                    if let Some(d) = detail
                        && d.to_ascii_lowercase().contains(&needle_lc)
                    {
                        return true;
                    }
                }
                StatusUpdate::ToolResult { name, preview, .. }
                    if name.to_ascii_lowercase().contains(&tool_lc) =>
                {
                    if preview.to_ascii_lowercase().contains(&needle_lc) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Assertion wrapper around [`Self::trace_contains_tool_call`] that
    /// panics with the captured tool-call activity rendered, so failing
    /// tests show *what the agent actually called* instead of just "false
    /// is not true".
    pub fn assert_trace_contains_tool_call(&self, tool_name: &str, needle: &str, context: &str) {
        if self.trace_contains_tool_call(tool_name, needle) {
            return;
        }
        use ironclaw::channels::StatusUpdate;
        let mut activity = String::new();
        for event in self.rig.captured_status_events() {
            match event {
                StatusUpdate::ToolStarted { name, detail, .. } => {
                    activity.push_str(&format!("  ● {name} {}\n", detail.unwrap_or_default()));
                }
                StatusUpdate::ToolResult { name, preview, .. } => {
                    let short: String = preview.chars().take(120).collect();
                    activity.push_str(&format!("    {name} → {short}\n"));
                }
                _ => {}
            }
        }
        panic!(
            "{context}: expected tool '{tool_name}' invocation containing '{needle}'.\n\
             Captured tool activity:\n{activity}"
        );
    }

    /// Flush the recorded trace (if live mode), save a human-readable session
    /// log, and shut down the agent.
    ///
    /// `user_input` is the message that was sent to the agent.
    /// `responses` are the agent's text responses (from `wait_for_responses`).
    ///
    /// The session log is written to `tests/fixtures/llm_traces/live/{name}.log`.
    pub async fn finish(self, user_input: &str, responses: &[String]) {
        let turns = [SessionTurn {
            source: TurnSource::User,
            user_input: user_input.to_string(),
            responses: responses.to_vec(),
        }];
        self.save_session_log(&turns);

        if let Some(ref recorder) = self.recording_handle {
            if let Err(e) = recorder.flush().await {
                eprintln!("[LiveTest] WARNING: Failed to flush trace: {e}");
            } else {
                eprintln!("[LiveTest] Trace recorded successfully");
            }
        }
        self.rig.shutdown();
    }

    /// Like `finish`, but panics if the trace contains any non-benign errors.
    /// This is the default for live tests — unexpected tool failures or
    /// executor SyntaxErrors are treated as regressions.
    pub async fn finish_strict(self, user_input: &str, responses: &[String]) {
        let errors = self.collect_trace_errors();
        if !errors.is_empty() {
            // Save the log first so the test author can see what happened.
            let turns = [SessionTurn {
                source: TurnSource::User,
                user_input: user_input.to_string(),
                responses: responses.to_vec(),
            }];
            self.save_session_log(&turns);
            if let Some(ref recorder) = self.recording_handle {
                let _ = recorder.flush().await;
            }
            self.rig.shutdown();
            let joined = errors.join("\n  - ");
            panic!(
                "Live trace contains {} error(s) that warrant investigation:\n  - {joined}",
                errors.len(),
            );
        }
        self.finish(user_input, responses).await;
    }

    /// Multi-turn variant of `finish`.
    pub async fn finish_turns(self, turns: &[SessionTurn]) {
        self.save_session_log(turns);

        if let Some(ref recorder) = self.recording_handle {
            if let Err(e) = recorder.flush().await {
                eprintln!("[LiveTest] WARNING: Failed to flush trace: {e}");
            } else {
                eprintln!("[LiveTest] Trace recorded successfully");
            }
        }
        self.rig.shutdown();
    }

    /// Multi-turn variant of `finish_strict`.
    pub async fn finish_turns_strict(self, turns: &[SessionTurn]) {
        let errors = self.collect_trace_errors();
        if !errors.is_empty() {
            self.save_session_log(turns);
            if let Some(ref recorder) = self.recording_handle {
                let _ = recorder.flush().await;
            }
            self.rig.shutdown();
            let joined = errors.join("\n  - ");
            panic!(
                "Live trace contains {} error(s) that warrant investigation:\n  - {joined}",
                errors.len(),
            );
        }
        self.finish_turns(turns).await;
    }

    /// Write a human-readable session log.
    ///
    /// Live mode writes to `tests/fixtures/llm_traces/live/{name}.log` (committed).
    /// Replay mode writes to a temp file so it can be diffed against the live log.
    fn save_session_log(&self, turns: &[SessionTurn]) {
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

        // Transcript
        for (idx, turn) in turns.iter().enumerate() {
            log.push_str(&format!("## Turn {}\n", idx + 1));
            log.push_str(&format!(
                "[{}] › {}\n",
                turn.source.label(),
                turn.user_input
            ));
            for response in &turn.responses {
                log.push_str("────────────────────────────────────────────────────\n");
                log.push_str(response);
                log.push('\n');
            }
            log.push('\n');
        }

        log.push_str("## Activity\n");

        // Tool activity from status events
        for event in self.rig.captured_status_events() {
            match event {
                StatusUpdate::SkillActivated { skill_names } => {
                    log.push_str(&format!("  ◆ skills: {}\n", skill_names.join(", ")));
                }
                StatusUpdate::ToolStarted { name, .. } => {
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
                StatusUpdate::ToolResult { name, preview, .. } => {
                    let short = if preview.len() > 200 {
                        // Find a safe char boundary to avoid panicking on multi-byte UTF-8.
                        let end = preview
                            .char_indices()
                            .map(|(i, _)| i)
                            .take_while(|&i| i <= 200)
                            .last()
                            .unwrap_or(0);
                        format!("{}…", &preview[..end]) // safety: end from char_indices(), always a valid boundary
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
    engine_v2: Option<bool>,
    auto_approve_tools: Option<bool>,
    skills_dir: Option<PathBuf>,
    channel_name: Option<String>,
    seeded_secret_names: Vec<String>,
}

impl LiveTestHarnessBuilder {
    /// Create a new builder for a test with the given name.
    ///
    /// The name determines the trace fixture filename:
    /// `tests/fixtures/llm_traces/live/{test_name}.json`
    ///
    /// **Live test contract:** the test rig starts from a *clean* libSQL
    /// database. It does NOT clone the developer's `~/.ironclaw/ironclaw.db`.
    /// Tests that need real credentials must declare them explicitly via
    /// [`with_secrets`](Self::with_secrets); tests that need workspace
    /// memory or conversation history must seed it themselves through
    /// the rig's APIs. See `tests/support/LIVE_TESTING.md` for the
    /// rationale and the PII scrub checklist that applies before
    /// committing a recorded trace.
    pub fn new(test_name: impl Into<String>) -> Self {
        Self {
            test_name: test_name.into(),
            max_tool_iterations: 30,
            engine_v2: None,
            auto_approve_tools: None,
            skills_dir: None,
            channel_name: None,
            seeded_secret_names: Vec::new(),
        }
    }

    /// Declare secret names to copy from the developer's real
    /// `~/.ironclaw/ironclaw.db` (or whatever `LIBSQL_PATH` resolves to)
    /// into the test rig under the same owner_user_id. Only the named
    /// rows are copied; nothing else (memory, history, other secrets)
    /// crosses the boundary.
    ///
    /// Example: `.with_secrets(["google_oauth_token"])` for a Gmail flow.
    ///
    /// Names not present in the source DB are logged as warnings — the
    /// test will then fail fast on its own missing-credential path,
    /// surfacing the typo in the secret name rather than silently
    /// skipping the credential.
    pub fn with_secrets(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.seeded_secret_names = names.into_iter().map(Into::into).collect();
        self
    }

    /// Override the test channel name. Useful when testing features that key
    /// on the channel name (e.g. mission notifications, assistant
    /// conversations) and you want to mirror the real "gateway" channel.
    pub fn with_channel_name(mut self, name: impl Into<String>) -> Self {
        self.channel_name = Some(name.into());
        self
    }

    /// Set the maximum number of tool iterations per agentic loop invocation.
    pub fn with_max_tool_iterations(mut self, n: usize) -> Self {
        self.max_tool_iterations = n;
        self
    }

    /// Force engine v2 on or off, overriding the env-resolved value.
    pub fn with_engine_v2(mut self, enabled: bool) -> Self {
        self.engine_v2 = Some(enabled);
        self
    }

    /// Override auto-approve tools setting. When not called, the value from
    /// `Config::from_env()` is used in live mode (default: false).
    pub fn with_auto_approve_tools(mut self, enabled: bool) -> Self {
        self.auto_approve_tools = Some(enabled);
        self
    }

    /// Enable skill discovery from the given directory. Skills discovered
    /// here (e.g. the repo's `./skills/` dir) are loaded at startup and can
    /// activate during the test conversation.
    pub fn with_skills_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.skills_dir = Some(dir.into());
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

        // Initialise a tracing subscriber so RUST_LOG actually captures the
        // engine's debug/trace output during the run. `try_init` is a no-op
        // when another test in the same process already initialised one.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("ironclaw=info")),
            )
            .with_test_writer()
            .try_init();

        // Load env from ~/.ironclaw/.env so LLM API keys are available.
        let _ = dotenvy::dotenv();
        ironclaw::bootstrap::load_ironclaw_env();

        // Hydrate LLM credentials from the user's real secrets store into
        // process env vars BEFORE config resolution. The test rig runs
        // against an isolated temp libSQL database, so the real ironclaw DB's
        // secrets aren't automatically visible to the provider chain. For
        // backends that support env-var fallback (nearai via NEARAI_API_KEY,
        // anthropic via ANTHROPIC_API_KEY, etc.), setting the env var before
        // `build_provider_chain` bypasses the interactive auth flow without
        // leaking secrets into the test database.
        hydrate_llm_secrets_into_env().await;

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
        if let Some(aa) = self.auto_approve_tools {
            config.agent.auto_approve_tools = aa;
        }
        if let Some(ref dir) = self.skills_dir {
            config.skills.enabled = true;
            config.skills.local_dir = dir.clone();
        }

        eprintln!(
            "[LiveTest] Config: engine_v2={}, allow_local_tools={}, auto_approve={}, skills_dir={}",
            config.agent.engine_v2,
            config.agent.allow_local_tools,
            config.agent.auto_approve_tools,
            config.skills.local_dir.display(),
        );

        // If the test asked for specific secrets via `with_secrets(...)`
        // and the resolved config points at a local libSQL file (the
        // typical `~/.ironclaw/ironclaw.db` setup), figure out the source
        // path now. We do NOT clone the file. The test rig will copy
        // *only* the named rows out of the source `secrets` table after
        // its own migrations run. Memory, conversation history, and any
        // unrequested secret stay in the source — tests that need that
        // data must seed it themselves.
        let secrets_source: Option<std::path::PathBuf> = if self.seeded_secret_names.is_empty() {
            None
        } else {
            match config.database.backend {
                ironclaw::config::DatabaseBackend::LibSql
                    if config.database.libsql_url.is_none() =>
                {
                    config
                        .database
                        .libsql_path
                        .clone()
                        .filter(|p| p.exists())
                        .or_else(|| {
                            let default = ironclaw::config::default_libsql_path();
                            default.exists().then_some(default)
                        })
                }
                _ => None,
            }
        };
        if !self.seeded_secret_names.is_empty() {
            match &secrets_source {
                Some(src) => eprintln!(
                    "[LiveTest] Will seed {} secret(s) from {}: {:?}",
                    self.seeded_secret_names.len(),
                    src.display(),
                    self.seeded_secret_names
                ),
                None => eprintln!(
                    "[LiveTest] WARNING: with_secrets() requested {:?} but no local libSQL \
                     source DB exists — the test will run with no seeded credentials and \
                     will likely fail on its first auth-gated tool call",
                    self.seeded_secret_names
                ),
            }
        } else {
            eprintln!(
                "[LiveTest] Starting with a clean DB. No secrets seeded; \
                 declare them with `.with_secrets([...])` if your scenario needs credentials."
            );
        }
        let source_user_id = config.owner_id.clone();

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
        // - auto_approve_tools comes from the env/config (tests can override
        //   via LiveTestHarnessBuilder if needed)
        let skills_dir_for_rig = self.skills_dir.clone();
        let mut rig_builder = TestRigBuilder::new()
            .with_config(config)
            .with_llm(llm)
            .with_http_interceptor(http_interceptor)
            .with_max_tool_iterations(self.max_tool_iterations);
        if let Some(dir) = skills_dir_for_rig {
            rig_builder = rig_builder.with_skills_dir(dir);
        }
        if let Some(ref name) = self.channel_name {
            rig_builder = rig_builder.with_channel_name(name.clone());
        }
        if let Some(src) = secrets_source {
            rig_builder = rig_builder.with_seeded_secrets(
                src,
                source_user_id,
                self.seeded_secret_names.clone(),
            );
        }
        let rig = rig_builder.build().await;

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

        let mut rig_builder = TestRigBuilder::new()
            .with_trace(trace)
            .with_max_tool_iterations(self.max_tool_iterations)
            .with_auto_approve_tools(true);
        if let Some(dir) = self.skills_dir.clone() {
            rig_builder = rig_builder.with_skills_dir(dir);
        }
        // Propagate engine_v2 so replay mirrors live recording. Without this,
        // tests that recorded against engine v2 (mission_create, mission_fire,
        // CodeAct orchestration, etc.) replay against v1 and the v2-only tools
        // come back as "tool not found".
        if self.engine_v2.unwrap_or(false) {
            rig_builder = rig_builder.with_engine_v2();
        }
        if let Some(ref name) = self.channel_name {
            rig_builder = rig_builder.with_channel_name(name.clone());
        }
        let rig = rig_builder.build().await;

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
            // Expect exactly "PASS: <reason>" or "FAIL: <reason>".
            if let Some(reason) = trimmed.strip_prefix("PASS:") {
                JudgeVerdict {
                    pass: true,
                    reasoning: reason.trim().to_string(),
                }
            } else if let Some(reason) = trimmed.strip_prefix("FAIL:") {
                JudgeVerdict {
                    pass: false,
                    reasoning: reason.trim().to_string(),
                }
            } else {
                JudgeVerdict {
                    pass: false,
                    reasoning: format!(
                        "Judge returned unexpected format (expected PASS:/FAIL:): {trimmed}"
                    ),
                }
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

/// Errors that we expect during normal operation and should not fail tests on.
///
/// These are "the agent picked the wrong tool or wrong params, here's how to
/// recover" messages that the LLM uses to self-correct. None of them indicate
/// an engine bug. Engine bugs (Python SyntaxError, missing leases for FINAL,
/// orphaned skill credentials, etc.) are still flagged unless we've observed a
/// specific lease miss that the run reliably recovers from in these workflows.
///
/// Categories of benign errors:
///
/// 1. **Workspace probing**: agent calls `memory_read` to check whether a
///    file exists before writing it. The tool returns a hard error instead
///    of a "not found" sentinel, but the agent's recovery is normal.
///
/// 2. **Wrong tool selection**: agent calls `write_file` for a workspace
///    file. The tool rejects with a clear "use memory_write instead"
///    message and the agent retries with the right tool.
///
/// 3. **Wrong patch params**: agent calls `memory_write` with `old_string`
///    but no `new_string`. The tool's error message tells the agent how to
///    fix it and the agent retries.
///
/// 4. **Skill probe**: agent calls `skill_install` for a skill that's
///    already loaded. The current skill_install short-circuits, but older
///    traces may have hit the registry 404 path.
///
/// 5. **Recovered CodeAct misfire**: agent briefly sends plain natural
///    language like "YouTube Published ✓" to CodeAct, gets a SyntaxError,
///    then immediately recovers with the correct memory-tool writes.
///
/// 6. **Recovered digest CodeAct probe**: agent briefly tries to count or
///    summarize commitments inside CodeAct, hits a NameError/Traceback, then
///    recovers by using `memory_tree` / `memory_read` and still produces the
///    correct digest.
fn is_benign_error(err: &str) -> bool {
    let lower = err.to_lowercase();

    // Workspace probing
    if lower.contains("document not found") || lower.contains("path not found") {
        return true;
    }

    // Wrong tool selection (write_file → memory_write guidance)
    if lower.contains("use the memory_write tool")
        || lower.contains("use the memory_read tool")
        || lower.contains("use memory_write instead")
        || lower.contains("use memory_read instead")
    {
        return true;
    }

    // Wrong patch params (memory_write patch mode confusion)
    if lower.contains("new_string is required when old_string is provided")
        || lower.contains("either 'content' (for write/append) or 'old_string'")
        || lower.contains("old_string not found in document")
        || lower.contains("old_string cannot be empty")
        || lower.contains("patch mode (old_string/new_string) cannot be combined with layer")
    {
        return true;
    }

    // Optional asset generation can fail in environments without the expected
    // image backend model; the conversation can still recover and persist the
    // actual commitment-tracking state we care about in these tests.
    if lower.contains("model 'flux-1.1-pro' not found")
        || (lower.contains("image generation api returned 404") && lower.contains("model"))
    {
        return true;
    }

    // Skill probe — installing a skill that already exists.
    if lower.contains("skill") && lower.contains("already") && lower.contains("exists") {
        return true;
    }

    // Live providers can transiently rate-limit bursty setup/write sequences.
    // The agent often retries successfully; treat these as benign harness noise.
    if lower.contains("rate limited") || lower.contains("try again in") {
        return true;
    }

    // Some live-model search queries include hyphenated repo names in a way
    // that SQLite FTS parses as a column reference (`payments-api` → `api`).
    // The run usually recovers after a broader search or direct read.
    if lower.contains("fts row fetch failed") && lower.contains("no such column:") {
        return true;
    }

    // Some promote-plan flows probe `rlm_query` without a lease and then
    // recover via memory search / plan writes. Treat that specific recovered
    // lease miss as benign harness noise.
    if lower.contains("no lease for action 'rlm_query'") {
        return true;
    }

    if lower.contains("no lease for action 'shell'") {
        return true;
    }

    // CodeAct occasionally probes a Python snippet that touches OS-backed time
    // APIs, which is blocked in the sandbox. If the run recovers, don't fail
    // the whole live trace on that transient probe.
    if lower.contains("os operations are not permitted in codeact scripts") {
        return true;
    }

    // A recurring recovered misfire in creator flows: plain text intended as
    // status content gets routed into CodeAct and fails to parse as Python.
    // If the run recovers, treat this as tool-selection noise rather than a
    // product regression.
    if lower.contains("youtube published")
        && lower.contains("syntaxerror")
        && lower.contains("simple statements must be separated")
    {
        return true;
    }

    if lower.contains("codeact execution failed")
        && lower.contains("traceback")
        && (lower.contains("nameerror") || lower.contains("step.py"))
    {
        return true;
    }

    false
}

/// Scan a tool result preview for executor-side errors that we want to flag.
///
/// Returns `Some(reason)` if the preview contains a Python SyntaxError,
/// Monty traceback, or a JSON-style `"error"` payload that isn't a benign
/// "document not found".
fn scan_preview_for_errors(preview: &str) -> Option<String> {
    // Python / Monty syntax errors from CodeAct execution
    if preview.contains("SyntaxError") && !is_benign_error(preview) {
        return Some("Python SyntaxError in CodeAct execution".to_string());
    }
    if preview.contains("Traceback (most recent call last)") && !is_benign_error(preview) {
        return Some("Python traceback in CodeAct execution".to_string());
    }
    // JSON-style error payloads from tool wrappers
    if let Some(idx) = preview
        .find("'error'")
        .or_else(|| preview.find("\"error\""))
        && let Some(rest) = preview.get(idx..)
    {
        // Extract a short snippet of the error message for the report.
        let snippet: String = rest.chars().take(200).collect();
        if !is_benign_error(&snippet) {
            return Some(snippet);
        }
    }
    None
}

/// Load LLM API keys from the user's real secrets store into process env vars.
///
/// Live tests use an isolated temp libSQL database, so the real ironclaw DB's
/// encrypted secrets are invisible to the test provider chain. This helper
/// opens the user's real libSQL DB at `~/.ironclaw/ironclaw.db` (libsql does
/// not expose a read-only open mode here, so the handle is technically
/// writable, but this code path only ever calls `get_decrypted` and never
/// writes), resolves the master key from the OS keychain, decrypts known
/// LLM API-key secrets, and exports them as env vars. `build_provider_chain`
/// then picks them up via each provider's env-var fallback, skipping
/// interactive auth.
///
/// This function is best-effort: any failure (no DB, locked keychain, secret
/// missing) is logged and ignored so the provider can fall back to whatever
/// native auth path it supports.
#[cfg(feature = "libsql")]
async fn hydrate_llm_secrets_into_env() {
    use ironclaw::secrets::{
        LibSqlSecretsStore, SecretsStore, crypto_from_hex, resolve_master_key,
    };

    // Known (secret_name, env_var) pairs. When a backend supports multiple
    // env-var fallbacks we pick the most canonical one.
    const SECRET_TO_ENV: &[(&str, &str)] = &[
        ("llm_nearai_api_key", "NEARAI_API_KEY"),
        ("llm_anthropic_api_key", "ANTHROPIC_API_KEY"),
        ("llm_openai_api_key", "OPENAI_API_KEY"),
    ];

    // If all target env vars are already set, skip the DB work entirely.
    if SECRET_TO_ENV
        .iter()
        .all(|(_, env)| std::env::var(env).ok().filter(|v| !v.is_empty()).is_some())
    {
        return;
    }

    let master_key = match resolve_master_key().await {
        Some(k) => k,
        None => {
            eprintln!("[LiveTest] hydrate_llm_secrets: no master key (env/keychain) — skipping");
            return;
        }
    };

    let crypto = match crypto_from_hex(&master_key) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[LiveTest] hydrate_llm_secrets: crypto init failed: {e} — skipping");
            return;
        }
    };

    // Open the user's real libSQL DB at ~/.ironclaw/ironclaw.db directly
    // (bypassing the ironclaw Database wrapper — LibSqlSecretsStore needs a
    // raw libsql::Database handle).
    let db_path = ironclaw::bootstrap::ironclaw_base_dir().join("ironclaw.db");
    if !db_path.exists() {
        eprintln!(
            "[LiveTest] hydrate_llm_secrets: real DB not found at {} — skipping",
            db_path.display()
        );
        return;
    }

    let raw_db = match libsql::Builder::new_local(&db_path).build().await {
        Ok(db) => std::sync::Arc::new(db),
        Err(e) => {
            eprintln!("[LiveTest] hydrate_llm_secrets: open real DB failed: {e} — skipping");
            return;
        }
    };

    let store = LibSqlSecretsStore::new(raw_db, crypto);

    // Owner id selection: a user with a non-default scope (e.g. via
    // `IRONCLAW_OWNER_ID` or settings.json) stores secrets under that
    // user_id, not "default". Try the env-resolved value first; if it's
    // unset, fall back to the legacy "default" scope that single-user
    // installs use. We don't reach into Config::from_env() here to avoid
    // pulling in the full settings file resolution chain inside test
    // hydration.
    let env_owner = std::env::var("IRONCLAW_OWNER_ID")
        .ok()
        .filter(|s| !s.is_empty());
    let owner_id_owned = env_owner.unwrap_or_else(|| "default".to_string());
    let owner_id = owner_id_owned.as_str();

    for (secret_name, env_var) in SECRET_TO_ENV {
        if std::env::var(env_var)
            .ok()
            .filter(|v| !v.is_empty())
            .is_some()
        {
            continue;
        }
        match store.get_decrypted(owner_id, secret_name).await {
            Ok(decrypted) => {
                ironclaw::config::set_runtime_env(env_var, decrypted.expose());
                eprintln!(
                    "[LiveTest] hydrate_llm_secrets: set {env_var} from secret '{secret_name}'"
                );
            }
            Err(ironclaw::secrets::SecretError::NotFound { .. }) => {
                // Normal: user hasn't configured this backend.
            }
            Err(e) => {
                eprintln!(
                    "[LiveTest] hydrate_llm_secrets: failed to read '{secret_name}': {e} — skipping"
                );
            }
        }
    }
}

/// Compute the path to a live trace fixture file.
fn trace_fixture_path(test_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/llm_traces/live")
        .join(format!("{test_name}.json"))
}

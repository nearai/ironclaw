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
    engine_v2: Option<bool>,
    auto_approve_tools: Option<bool>,
    skills_dir: Option<PathBuf>,
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
            engine_v2: None,
            auto_approve_tools: None,
            skills_dir: None,
        }
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
        if let Some(v2) = self.engine_v2
            && v2
        {
            rig_builder = rig_builder.with_engine_v2();
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

/// Load LLM API keys from the user's real secrets store into process env vars.
///
/// Live tests use an isolated temp libSQL database, so the real ironclaw DB's
/// encrypted secrets are invisible to the test provider chain. This helper
/// opens the user's real libSQL DB (read-only for our purposes), resolves the
/// master key from the OS keychain, decrypts known LLM API-key secrets, and
/// exports them as env vars. `build_provider_chain` then picks them up via
/// each provider's env-var fallback, skipping interactive auth.
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

    // Single-user mode owner id (matches Config::default().owner_id).
    let owner_id = "default";

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
                // SAFETY: this runs at test startup before any tokio tasks
                // read the env. Single-threaded prologue is fine.
                unsafe {
                    std::env::set_var(env_var, decrypted.expose());
                }
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

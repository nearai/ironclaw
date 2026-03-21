//! TestRig -- a builder for wiring a real Agent with a replay LLM and test channel.
//!
//! Constructs a full `Agent` with real tools but a `TraceLlm` (or custom LLM)
//! and a `TestChannel`, runs the agent in a background tokio task, and provides
//! methods to inject messages, wait for responses, and inspect tool calls.

#![allow(dead_code)] // Public API consumed by later test modules (Task 4+).

use std::sync::Arc;
use std::time::{Duration, Instant};

use ironclaw::agent::{Agent, AgentDeps};
use ironclaw::app::{AppBuilder, AppBuilderFlags};
use ironclaw::channels::web::log_layer::LogBroadcaster;
use ironclaw::channels::{OutgoingResponse, StatusUpdate};
use ironclaw::config::Config;
use ironclaw::db::Database;
use ironclaw::llm::{LlmProvider, SessionConfig, SessionManager};
use ironclaw::tools::Tool;

use crate::support::instrumented_llm::InstrumentedLlm;
use crate::support::metrics::{ToolInvocation, TraceMetrics};
use crate::support::test_channel::{TestChannel, TestChannelHandle};
use crate::support::trace_llm::{LlmTrace, TraceLlm};

use ironclaw::llm::recording::{HttpExchange, ReplayingHttpInterceptor};

// ---------------------------------------------------------------------------
// TestRig
// ---------------------------------------------------------------------------

/// A running test agent with methods to inject messages and inspect results.
pub struct TestRig {
    /// The test channel for sending messages and reading captures.
    channel: Arc<TestChannel>,
    /// Instrumented LLM for collecting token/call metrics.
    instrumented_llm: Arc<InstrumentedLlm>,
    /// When the rig was created (for wall-time measurement).
    start_time: Instant,
    /// Maximum tool-call iterations per agentic loop (for count-based limit detection).
    max_tool_iterations: usize,
    /// Handle to the background agent task (wrapped in Option so Drop can take it).
    agent_handle: Option<tokio::task::JoinHandle<()>>,
    /// Database handle for direct queries in tests.
    #[cfg(feature = "libsql")]
    db: Arc<dyn Database>,
    /// Workspace handle for direct memory operations in tests.
    #[cfg(feature = "libsql")]
    workspace: Option<Arc<ironclaw::workspace::Workspace>>,
    /// The underlying TraceLlm for inspecting captured requests.
    #[cfg(feature = "libsql")]
    trace_llm: Option<Arc<TraceLlm>>,
    /// Extension manager for direct extension operations in tests.
    #[cfg(feature = "libsql")]
    extension_manager: Option<Arc<ironclaw::extensions::ExtensionManager>>,
    /// Session manager for direct session/thread access in tests.
    #[cfg(feature = "libsql")]
    session_manager: Arc<ironclaw::agent::SessionManager>,
    /// Temp directory guard -- keeps the libSQL database file alive.
    #[cfg(feature = "libsql")]
    _temp_dir: tempfile::TempDir,
}

impl TestRig {
    /// Inject a user message into the agent.
    pub async fn send_message(&self, content: &str) {
        self.channel.send_message(content).await;
    }

    /// Inject a raw `IncomingMessage` (for tests that need attachments, etc.).
    pub async fn send_incoming(&self, msg: ironclaw::channels::IncomingMessage) {
        self.channel.send_incoming(msg).await;
    }

    /// Return all message lists that were sent to the LLM provider.
    ///
    /// Only available when the rig was built with a `TraceLlm` (i.e., via `.with_trace()`).
    pub fn captured_llm_requests(&self) -> Vec<Vec<ironclaw::llm::ChatMessage>> {
        self.trace_llm
            .as_ref()
            .map(|t| t.captured_requests())
            .unwrap_or_default()
    }

    /// Return the extension manager for direct extension operations in tests.
    pub fn extension_manager(&self) -> Option<&Arc<ironclaw::extensions::ExtensionManager>> {
        self.extension_manager.as_ref()
    }

    /// Return the session manager for direct session/thread access in tests.
    pub fn session_manager(&self) -> &Arc<ironclaw::agent::SessionManager> {
        &self.session_manager
    }

    /// Wait until at least `n` responses have been captured, or `timeout` elapses.
    pub async fn wait_for_responses(&self, n: usize, timeout: Duration) -> Vec<OutgoingResponse> {
        self.channel.wait_for_responses(n, timeout).await
    }

    /// Return the names of all `ToolStarted` events captured so far.
    pub fn tool_calls_started(&self) -> Vec<String> {
        self.channel.tool_calls_started()
    }

    /// Return `(name, success)` for all `ToolCompleted` events captured so far.
    pub fn tool_calls_completed(&self) -> Vec<(String, bool)> {
        self.channel.tool_calls_completed()
    }

    /// Return `(name, preview)` for all `ToolResult` events captured so far.
    pub fn tool_results(&self) -> Vec<(String, String)> {
        self.channel.tool_results()
    }

    /// Return `(name, duration_ms)` for all completed tools with timing data.
    pub fn tool_timings(&self) -> Vec<(String, u64)> {
        self.channel.tool_timings()
    }

    /// Return a snapshot of all captured status events.
    pub fn captured_status_events(&self) -> Vec<StatusUpdate> {
        self.channel.captured_status_events()
    }

    /// Clear all captured responses and status events.
    pub async fn clear(&self) {
        self.channel.clear().await;
    }

    /// Number of LLM calls made so far.
    pub fn llm_call_count(&self) -> u32 {
        self.instrumented_llm.call_count()
    }

    /// Total input tokens across all LLM calls.
    pub fn total_input_tokens(&self) -> u32 {
        self.instrumented_llm.total_input_tokens()
    }

    /// Total output tokens across all LLM calls.
    pub fn total_output_tokens(&self) -> u32 {
        self.instrumented_llm.total_output_tokens()
    }

    /// Estimated total cost in USD.
    pub fn estimated_cost_usd(&self) -> f64 {
        self.instrumented_llm.estimated_cost_usd()
    }

    /// Wall-clock time since rig creation.
    pub fn elapsed_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// Collect a complete `TraceMetrics` snapshot from all captured data.
    ///
    /// Call this after `wait_for_responses()` to get the full metrics for the
    /// scenario. The `turns` count is based on the number of captured responses.
    pub async fn collect_metrics(&self) -> TraceMetrics {
        let completed = self.tool_calls_completed();

        // Build ToolInvocation records from ToolStarted/ToolCompleted pairs,
        // matching each completion with its captured timing data.
        let timings = self.tool_timings();
        let mut timing_iter_by_name: std::collections::HashMap<&str, Vec<u64>> =
            std::collections::HashMap::new();
        for (name, ms) in &timings {
            timing_iter_by_name
                .entry(name.as_str())
                .or_default()
                .push(*ms);
        }

        let tool_invocations: Vec<ToolInvocation> = completed
            .iter()
            .map(|(name, success)| {
                let duration_ms = timing_iter_by_name
                    .get_mut(name.as_str())
                    .and_then(|v| {
                        if v.is_empty() {
                            None
                        } else {
                            Some(v.remove(0))
                        }
                    })
                    .unwrap_or(0);
                ToolInvocation {
                    name: name.clone(),
                    duration_ms,
                    success: *success,
                }
            })
            .collect();

        // Detect if iteration limit was hit by comparing completed tool-call count
        // against the configured max_tool_iterations threshold.
        let hit_iteration_limit = completed.len() >= self.max_tool_iterations;

        // Count turns as the number of captured responses.
        let responses = self.channel.captured_responses();
        let turns = responses.len() as u32;

        TraceMetrics {
            wall_time_ms: self.elapsed_ms(),
            llm_calls: self.instrumented_llm.call_count(),
            input_tokens: self.instrumented_llm.total_input_tokens(),
            output_tokens: self.instrumented_llm.total_output_tokens(),
            estimated_cost_usd: self.instrumented_llm.estimated_cost_usd(),
            tool_calls: tool_invocations,
            turns,
            hit_iteration_limit,
            hit_timeout: false, // Caller can set this based on wait_for_responses result.
        }
    }

    /// Run a complete multi-turn trace, injecting user messages from the trace
    /// and waiting for responses after each turn.
    ///
    /// Returns a `Vec` of response lists, one per turn. Status events and tool
    /// call data accumulate across all turns (no clearing between turns), so
    /// post-run assertions like `tool_calls_started()` reflect the whole trace.
    pub async fn run_trace(
        &self,
        trace: &LlmTrace,
        timeout: Duration,
    ) -> Vec<Vec<OutgoingResponse>> {
        let mut all_responses: Vec<Vec<OutgoingResponse>> = Vec::new();
        let mut total_responses = 0usize;
        for turn in &trace.turns {
            self.send_message(&turn.user_input).await;
            let responses = self.wait_for_responses(total_responses + 1, timeout).await;
            // Extract only the new responses from this turn.
            let turn_responses: Vec<OutgoingResponse> =
                responses.into_iter().skip(total_responses).collect();
            total_responses += turn_responses.len();
            all_responses.push(turn_responses);
        }
        all_responses
    }

    /// Run a trace, then verify all declarative `expects` (top-level and per-turn).
    ///
    /// Returns the per-turn response lists for additional manual assertions.
    pub async fn run_and_verify_trace(
        &self,
        trace: &LlmTrace,
        timeout: Duration,
    ) -> Vec<Vec<OutgoingResponse>> {
        use crate::support::assertions::verify_expects;

        let all_responses = self.run_trace(trace, timeout).await;

        // Verify top-level expects against all accumulated data.
        if !trace.expects.is_empty() {
            let all_response_strings: Vec<String> = all_responses
                .iter()
                .flat_map(|turn| turn.iter().map(|r| r.content.clone()))
                .collect();
            let started = self.tool_calls_started();
            let completed = self.tool_calls_completed();
            let mut results = self.tool_results();
            for status in self.channel.captured_status_events() {
                if let ironclaw::channels::StatusUpdate::ToolCompleted {
                    name,
                    success: false,
                    error,
                    parameters,
                } = status
                {
                    let detail = format!(
                        "error={}; params={}",
                        error.unwrap_or_else(|| "unknown".to_string()),
                        parameters.unwrap_or_else(|| "{}".to_string())
                    );
                    results.push((name, detail));
                }
            }
            verify_expects(
                &trace.expects,
                &all_response_strings,
                &started,
                &completed,
                &results,
                "top-level",
            );
        }

        all_responses
    }

    /// Verify top-level `expects` from a trace against already-captured data.
    ///
    /// Call this after `send_message()` + `wait_for_responses()` for flat-format
    /// traces. For multi-turn traces, use `run_and_verify_trace()` instead.
    pub fn verify_trace_expects(&self, trace: &LlmTrace, responses: &[OutgoingResponse]) {
        use crate::support::assertions::verify_expects;

        if trace.expects.is_empty() {
            return;
        }
        let response_strings: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let started = self.tool_calls_started();
        let completed = self.tool_calls_completed();
        let mut results = self.tool_results();
        for status in self.channel.captured_status_events() {
            if let ironclaw::channels::StatusUpdate::ToolCompleted {
                name,
                success: false,
                error,
                parameters,
            } = status
            {
                let detail = format!(
                    "error={}; params={}",
                    error.unwrap_or_else(|| "unknown".to_string()),
                    parameters.unwrap_or_else(|| "{}".to_string())
                );
                results.push((name, detail));
            }
        }
        verify_expects(
            &trace.expects,
            &response_strings,
            &started,
            &completed,
            &results,
            "top-level",
        );
    }

    /// Signal the channel to shut down and abort the background agent task.
    pub fn shutdown(mut self) {
        self.channel.signal_shutdown();
        if let Some(handle) = self.agent_handle.take() {
            handle.abort();
        }
    }
}

impl Drop for TestRig {
    fn drop(&mut self) {
        if let Some(handle) = self.agent_handle.take()
            && !handle.is_finished()
        {
            handle.abort();
        }
    }
}

// ---------------------------------------------------------------------------
// TestRigBuilder
// ---------------------------------------------------------------------------

/// Builder for constructing a `TestRig`.
pub struct TestRigBuilder {
    trace: Option<LlmTrace>,
    llm: Option<Arc<dyn LlmProvider>>,
    max_tool_iterations: usize,
    injection_check: bool,
    auto_approve_tools: Option<bool>,
    enable_skills: bool,
    enable_routines: bool,
    http_exchanges: Vec<HttpExchange>,
    extra_tools: Vec<Arc<dyn Tool>>,
    keep_bootstrap: bool,
}

impl TestRigBuilder {
    /// Create a new builder with defaults.
    pub fn new() -> Self {
        Self {
            trace: None,
            llm: None,
            max_tool_iterations: 10,
            injection_check: false,
            auto_approve_tools: Some(true),
            enable_skills: false,
            enable_routines: false,
            http_exchanges: Vec::new(),
            extra_tools: Vec::new(),
            keep_bootstrap: false,
        }
    }

    /// Set the LLM trace to replay.
    pub fn with_trace(mut self, trace: LlmTrace) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Override the LLM provider directly (takes precedence over trace).
    pub fn with_llm(mut self, llm: Arc<dyn LlmProvider>) -> Self {
        self.llm = Some(llm);
        self
    }

    /// Set the maximum number of tool iterations per agentic loop invocation.
    pub fn with_max_tool_iterations(mut self, n: usize) -> Self {
        self.max_tool_iterations = n;
        self
    }

    /// Register additional custom tools (e.g. stub tools for testing).
    pub fn with_extra_tools(mut self, tools: Vec<Arc<dyn Tool>>) -> Self {
        self.extra_tools = tools;
        self
    }

    /// Enable prompt injection detection in the safety layer.
    ///
    /// When enabled, tool outputs are scanned for injection patterns
    /// (e.g., "ignore previous instructions", special tokens like `<|endoftext|>`)
    /// and critical patterns are escaped before reaching the LLM.
    pub fn with_injection_check(mut self, enable: bool) -> Self {
        self.injection_check = enable;
        self
    }

    /// Override agent-level automatic approval of `UnlessAutoApproved` tools.
    pub fn with_auto_approve_tools(mut self, enable: bool) -> Self {
        self.auto_approve_tools = Some(enable);
        self
    }

    /// Enable skill discovery and registration for this test rig.
    pub fn with_skills(mut self) -> Self {
        self.enable_skills = true;
        self
    }

    /// Enable the routines system so the scheduler is wired with a `RoutineEngine`,
    /// allowing routine jobs to actually execute. Routine tools are always registered
    /// but require the engine to dispatch jobs.
    pub fn with_routines(mut self) -> Self {
        self.enable_routines = true;
        self
    }

    /// Keep `bootstrap_pending` so the proactive greeting fires on startup.
    pub fn with_bootstrap(mut self) -> Self {
        self.keep_bootstrap = true;
        self
    }

    /// Add pre-recorded HTTP exchanges for the `ReplayingHttpInterceptor`.
    ///
    /// When set, all `http` tool calls will return these responses in order
    /// instead of making real network requests.
    pub fn with_http_exchanges(mut self, exchanges: Vec<HttpExchange>) -> Self {
        self.http_exchanges = exchanges;
        self
    }

    /// Build the test rig, creating a real agent and spawning it in the background.
    ///
    /// Uses `AppBuilder::build_all()` to get the same component set as the real
    /// binary, with only the LLM swapped for TraceLlm.
    ///
    /// Requires the `libsql` feature for the embedded test database.
    #[cfg(feature = "libsql")]
    pub async fn build(self) -> TestRig {
        use ironclaw::channels::ChannelManager;
        use ironclaw::db::libsql::LibSqlBackend;

        // Destructure self up front to avoid partial-move issues.
        let TestRigBuilder {
            trace,
            llm,
            max_tool_iterations,
            injection_check,
            auto_approve_tools,
            enable_skills,
            enable_routines,
            http_exchanges: explicit_http_exchanges,
            extra_tools,
            keep_bootstrap,
        } = self;

        // 1. Create temp dir + libSQL database + run migrations.
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = temp_dir.path().join("test_rig.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("failed to create test LibSqlBackend");
        backend
            .run_migrations()
            .await
            .expect("failed to run migrations");
        let db: Arc<dyn ironclaw::db::Database> = Arc::new(backend);

        // 2. Build Config::for_testing().
        let skills_dir = temp_dir.path().join("skills");
        let installed_skills_dir = temp_dir.path().join("installed_skills");
        let _ = std::fs::create_dir_all(&skills_dir);
        let _ = std::fs::create_dir_all(&installed_skills_dir);
        let mut config = Config::for_testing(db_path, skills_dir, installed_skills_dir);
        config.agent.max_tool_iterations = max_tool_iterations;
        config.safety.injection_check_enabled = injection_check;
        config.skills.enabled = enable_skills;
        if let Some(v) = auto_approve_tools {
            config.agent.auto_approve_tools = v;
        }

        // 3. Create SessionManager + LogBroadcaster.
        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let log_broadcaster = Arc::new(LogBroadcaster::new());

        // 4. Create TraceLlm + InstrumentedLlm, extract HTTP exchanges for replay.
        let trace_http_exchanges = trace
            .as_ref()
            .map(|t| t.http_exchanges.clone())
            .unwrap_or_default();

        let mut trace_llm_ref: Option<Arc<TraceLlm>> = None;
        let base_llm: Arc<dyn LlmProvider> = if let Some(llm) = llm {
            llm
        } else if let Some(trace) = trace {
            let tlm = Arc::new(TraceLlm::from_trace(trace));
            trace_llm_ref = Some(Arc::clone(&tlm));
            tlm
        } else {
            let trace = LlmTrace::single_turn(
                "test-rig-default",
                "(default)",
                vec![crate::support::trace_llm::TraceStep {
                    request_hint: None,
                    response: crate::support::trace_llm::TraceResponse::Text {
                        content: "Hello from test rig!".to_string(),
                        input_tokens: 10,
                        output_tokens: 5,
                    },
                    expected_tool_results: Vec::new(),
                }],
            );
            let tlm = Arc::new(TraceLlm::from_trace(trace));
            trace_llm_ref = Some(Arc::clone(&tlm));
            tlm
        };
        let instrumented = Arc::new(InstrumentedLlm::new(base_llm));
        let llm: Arc<dyn LlmProvider> = Arc::clone(&instrumented) as Arc<dyn LlmProvider>;

        // 5. Build AppComponents via AppBuilder with injected DB and LLM.
        let mut builder = AppBuilder::new(
            config,
            AppBuilderFlags::default(),
            None,
            session,
            log_broadcaster,
        );
        builder.with_database(Arc::clone(&db));
        builder.with_llm(llm);
        let mut components = builder
            .build_all()
            .await
            .expect("AppBuilder::build_all() failed in test rig");

        // Clear bootstrap flag so tests don't get an unexpected proactive greeting
        // (unless the test explicitly wants to test the bootstrap flow).
        if !keep_bootstrap && let Some(ref ws) = components.workspace {
            ws.take_bootstrap_pending();
        }

        // AppBuilder may re-resolve config from env/TOML and override test defaults.
        // Force test-rig agent flags to the requested deterministic values.
        components.config.agent.auto_approve_tools = auto_approve_tools.unwrap_or(true);
        components.config.agent.allow_local_tools = true;

        let scheduler_slot: ironclaw::tools::builtin::SchedulerSlot =
            Arc::new(tokio::sync::RwLock::new(None));

        // 6. Register job tools, routine tools, and extra tools.
        {
            // Ensure filesystem/shell dev tools are always available in the
            // test rig, even if upstream builder flags/config disable local tools.
            components.tools.register_dev_tools();

            components.tools.register_job_tools(
                Arc::clone(&components.context_manager),
                Some(scheduler_slot.clone()),
                None,
                components.db.clone(),
                None,
                None,
                None,
                None,
            );

            // Routine tools: create a RoutineEngine with the LLM and workspace.
            if let (Some(db_arc), Some(ws)) = (&components.db, &components.workspace) {
                use ironclaw::agent::routine_engine::RoutineEngine;
                use ironclaw::config::RoutineConfig;

                let routine_config = RoutineConfig::default();
                let (notify_tx, _notify_rx) = tokio::sync::mpsc::channel(16);
                let engine = Arc::new(RoutineEngine::new(
                    routine_config,
                    Arc::clone(db_arc),
                    components.llm.clone(),
                    Arc::clone(ws),
                    notify_tx,
                    None,
                    None,
                    components.tools.clone(),
                    components.safety.clone(),
                    ironclaw::agent::SandboxReadiness::Available, // tests don't use real Docker
                ));
                components
                    .tools
                    .register_routine_tools(Arc::clone(db_arc), engine);
            }

            // Skills tools: ensure tests use temp skill dirs (sandbox-safe) even if
            // AppBuilder did not wire them for this environment.
            if enable_skills {
                let registry = Arc::new(std::sync::RwLock::new(
                    ironclaw::skills::SkillRegistry::new(temp_dir.path().join("skills"))
                        .with_installed_dir(temp_dir.path().join("installed_skills")),
                ));
                let catalog = ironclaw::skills::catalog::shared_catalog();
                components
                    .tools
                    .register_skill_tools(Arc::clone(&registry), Arc::clone(&catalog));
                components.skill_registry = Some(registry);
                components.skill_catalog = Some(catalog);
            }

            // Register any extra test-specific tools.
            for tool in extra_tools {
                components.tools.register(tool).await;
            }
        }

        // Save references for test accessors.
        let db_ref = components.db.clone().expect("test rig requires a database");
        let workspace_ref = components.workspace.clone();
        let ext_mgr_ref = components.extension_manager.clone();
        let session_manager_ref = Arc::new(ironclaw::agent::SessionManager::new());

        // 7. Construct AgentDeps from AppComponents (mirrors main.rs).
        let deps = AgentDeps {
            owner_id: components.config.owner_id.clone(),
            store: components.db,
            llm: components.llm,
            cheap_llm: components.cheap_llm,
            safety: components.safety,
            tools: components.tools,
            workspace: components.workspace,
            extension_manager: components.extension_manager,
            skill_registry: components.skill_registry,
            skill_catalog: components.skill_catalog,
            skills_config: components.config.skills.clone(),
            hooks: components.hooks,
            cost_guard: components.cost_guard,
            sse_tx: None,
            http_interceptor: {
                // Prefer explicit exchanges from with_http_exchanges(), fall back to trace.
                let exchanges = if explicit_http_exchanges.is_empty() {
                    trace_http_exchanges
                } else {
                    explicit_http_exchanges
                };
                if exchanges.is_empty() {
                    None
                } else {
                    Some(Arc::new(ReplayingHttpInterceptor::new(exchanges))
                        as Arc<dyn ironclaw::llm::recording::HttpInterceptor>)
                }
            },
            transcription: None,
            document_extraction: None,
            sandbox_readiness: ironclaw::agent::SandboxReadiness::Available, // tests don't use real Docker
            builder: None,
        };

        // 7. Create TestChannel and ChannelManager.
        // When testing bootstrap, the channel must be named "gateway" because
        // the bootstrap greeting targets only the gateway channel.
        let test_channel = if keep_bootstrap {
            Arc::new(TestChannel::new().with_name("gateway"))
        } else {
            Arc::new(TestChannel::new())
        };
        let handle = TestChannelHandle::new(Arc::clone(&test_channel));
        let channel_manager = ChannelManager::new();
        channel_manager.add(Box::new(handle)).await;
        let channels = Arc::new(channel_manager);

        // 7b. Register message tool so routines can send messages to channels.
        deps.tools
            .register_message_tools(Arc::clone(&channels), deps.extension_manager.clone())
            .await;

        // 8. Create Agent.
        let routine_config = if enable_routines {
            Some(ironclaw::config::RoutineConfig {
                enabled: true,
                cron_check_interval_secs: 60,
                max_concurrent_routines: 3,
                default_cooldown_secs: 300,
                max_lightweight_tokens: 4096,
                lightweight_tools_enabled: true,
                lightweight_max_iterations: 3,
            })
        } else {
            None
        };
        let agent = Agent::new(
            components.config.agent.clone(),
            deps,
            channels,
            None, // heartbeat_config
            None, // hygiene_config
            routine_config,
            Some(Arc::clone(&components.context_manager)),
            Some(Arc::clone(&session_manager_ref)),
        );

        // Match main.rs: fill the scheduler slot once Agent::new has created it.
        *scheduler_slot.write().await = Some(agent.scheduler());

        // 9. Spawn agent in background task.
        let agent_handle = tokio::spawn(async move {
            if let Err(e) = agent.run().await {
                eprintln!("[TestRig] Agent exited with error: {e}");
            }
        });

        // 10. Wait for the agent to call channel.start() (up to 5 seconds).
        if let Some(rx) = test_channel.take_ready_rx().await {
            let _ = tokio::time::timeout(Duration::from_secs(5), rx).await;
        }

        TestRig {
            channel: test_channel,
            instrumented_llm: instrumented,
            start_time: Instant::now(),
            max_tool_iterations,
            agent_handle: Some(agent_handle),
            db: db_ref,
            workspace: workspace_ref,
            trace_llm: trace_llm_ref,
            extension_manager: ext_mgr_ref,
            session_manager: session_manager_ref,
            _temp_dir: temp_dir,
        }
    }
}

impl Default for TestRigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestRig {
    /// Get the database handle for direct queries.
    #[cfg(feature = "libsql")]
    pub fn database(&self) -> &Arc<dyn Database> {
        &self.db
    }

    /// Get the workspace handle for direct memory operations.
    #[cfg(feature = "libsql")]
    pub fn workspace(&self) -> Option<&Arc<ironclaw::workspace::Workspace>> {
        self.workspace.as_ref()
    }

    /// Get the underlying TraceLlm for inspecting captured requests.
    #[cfg(feature = "libsql")]
    pub fn trace_llm(&self) -> Option<&Arc<TraceLlm>> {
        self.trace_llm.as_ref()
    }

    /// Check if any captured status events contain safety/injection warnings.
    pub fn has_safety_warnings(&self) -> bool {
        self.captured_status_events().iter().any(|s| {
            matches!(s, StatusUpdate::Status(msg) if msg.contains("sanitiz") || msg.contains("inject") || msg.contains("warning"))
        })
    }
}

// ---------------------------------------------------------------------------
// Convenience: run a recorded trace fixture end-to-end
// ---------------------------------------------------------------------------

/// Load a recorded trace fixture, build a rig, run and verify expects, then shut down.
///
/// `filename` is relative to `tests/fixtures/llm_traces/recorded/`.
#[cfg(feature = "libsql")]
pub async fn run_recorded_trace(filename: &str) {
    let path = format!(
        "{}/tests/fixtures/llm_traces/recorded/{filename}",
        env!("CARGO_MANIFEST_DIR")
    );
    let trace = LlmTrace::from_file(&path)
        .unwrap_or_else(|e| panic!("failed to load trace {filename}: {e}"));
    let rig = TestRigBuilder::new()
        .with_trace(trace.clone())
        .build()
        .await;
    rig.run_and_verify_trace(&trace, Duration::from_secs(30))
        .await;
    rig.shutdown();
}

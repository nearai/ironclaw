//! TestRig -- a builder for wiring a real Agent with a replay LLM and test channel.
//!
//! Constructs a full `Agent` with real tools but a `TraceLlm` (or custom LLM)
//! and a `TestChannel`, runs the agent in a background tokio task, and provides
//! methods to inject messages, wait for responses, and inspect tool calls.

#![allow(dead_code)] // Public API consumed by later test modules (Task 4+).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use ironclaw::agent::cost_guard::{CostGuard, CostGuardConfig};
use ironclaw::agent::{Agent, AgentDeps};
use ironclaw::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use ironclaw::config::{AgentConfig, SafetyConfig, SkillsConfig};
use ironclaw::db::Database;
use ironclaw::error::ChannelError;
use ironclaw::hooks::HookRegistry;
use ironclaw::llm::LlmProvider;
use ironclaw::safety::SafetyLayer;
use ironclaw::tools::ToolRegistry;

use crate::support::instrumented_llm::InstrumentedLlm;
use crate::support::metrics::{ToolInvocation, TraceMetrics};
use crate::support::test_channel::TestChannel;
use crate::support::trace_llm::{LlmTrace, TraceLlm};

// ---------------------------------------------------------------------------
// TestChannelHandle -- wraps Arc<TestChannel> as Box<dyn Channel>
// ---------------------------------------------------------------------------

/// A thin wrapper around `Arc<TestChannel>` that implements `Channel`.
///
/// This lets us hand a `Box<dyn Channel>` to `ChannelManager::add()` while
/// keeping an `Arc<TestChannel>` in the `TestRig` for sending messages and
/// reading captures.
struct TestChannelHandle {
    inner: Arc<TestChannel>,
}

impl TestChannelHandle {
    fn new(inner: Arc<TestChannel>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl Channel for TestChannelHandle {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        self.inner.start().await
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.inner.respond(msg, response).await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        self.inner.send_status(status, metadata).await
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.inner.broadcast(user_id, response).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        self.inner.health_check().await
    }

    fn conversation_context(&self, metadata: &serde_json::Value) -> HashMap<String, String> {
        self.inner.conversation_context(metadata)
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.inner.shutdown().await
    }
}

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
    /// Handle to the background agent task (wrapped in Option so Drop can take it).
    agent_handle: Option<tokio::task::JoinHandle<()>>,
    /// Temp directory guard -- keeps the libSQL database file alive.
    #[cfg(feature = "libsql")]
    _temp_dir: tempfile::TempDir,
}

impl TestRig {
    /// Inject a user message into the agent.
    pub async fn send_message(&self, content: &str) {
        self.channel.send_message(content).await;
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
        let status_events = self.captured_status_events();

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

        // Detect if iteration limit was hit by checking for the status pattern:
        // If max_tool_iterations was reached, the dispatcher forces a text response.
        // We detect this by checking if the last status event before the response
        // was a ToolCompleted and the response count matches expectations.
        let hit_iteration_limit = status_events.iter().any(|s| {
            matches!(s, StatusUpdate::Status(msg) if msg.contains("iteration") || msg.contains("limit"))
        });

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
    tools: Option<Arc<ToolRegistry>>,
    max_tool_iterations: usize,
    enable_workspace: bool,
    injection_check: bool,
}

impl TestRigBuilder {
    /// Create a new builder with defaults.
    pub fn new() -> Self {
        Self {
            trace: None,
            llm: None,
            tools: None,
            max_tool_iterations: 10,
            enable_workspace: false,
            injection_check: false,
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

    /// Override the tool registry.
    pub fn with_tools(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set the maximum number of tool iterations per agentic loop invocation.
    pub fn with_max_tool_iterations(mut self, n: usize) -> Self {
        self.max_tool_iterations = n;
        self
    }

    /// Enable workspace and memory tools in the test rig.
    ///
    /// When enabled, the rig creates a `Workspace` backed by the same libSQL
    /// database and registers memory tools (memory_write, memory_read,
    /// memory_search, memory_tree).
    pub fn with_workspace(mut self, enable: bool) -> Self {
        self.enable_workspace = enable;
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

    /// Build the test rig, creating a real agent and spawning it in the background.
    ///
    /// Requires the `libsql` feature for the embedded test database.
    #[cfg(feature = "libsql")]
    pub async fn build(self) -> TestRig {
        use ironclaw::channels::ChannelManager;
        use ironclaw::db::libsql::LibSqlBackend;

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
        let db: Arc<dyn Database> = Arc::new(backend);

        // 2. Create LLM provider and wrap with InstrumentedLlm.
        let base_llm: Arc<dyn LlmProvider> = if let Some(llm) = self.llm {
            llm
        } else if let Some(trace) = self.trace {
            Arc::new(TraceLlm::from_trace(trace))
        } else {
            // Default: single-step text trace.
            let trace = LlmTrace {
                model_name: "test-rig-default".to_string(),
                steps: vec![crate::support::trace_llm::TraceStep {
                    request_hint: None,
                    response: crate::support::trace_llm::TraceResponse::Text {
                        content: "Hello from test rig!".to_string(),
                        input_tokens: 10,
                        output_tokens: 5,
                    },
                }],
            };
            Arc::new(TraceLlm::from_trace(trace))
        };
        let instrumented = Arc::new(InstrumentedLlm::new(base_llm));
        let llm: Arc<dyn LlmProvider> = Arc::clone(&instrumented) as Arc<dyn LlmProvider>;

        // 3. Create tool registry and optional workspace.
        let enable_workspace = self.enable_workspace;
        let workspace = if enable_workspace {
            Some(Arc::new(ironclaw::workspace::Workspace::new_with_db(
                "test-user",
                Arc::clone(&db),
            )))
        } else {
            None
        };

        let tools = self.tools.unwrap_or_else(|| {
            let t = Arc::new(ToolRegistry::new());
            t.register_builtin_tools();
            if let Some(ref ws) = workspace {
                t.register_memory_tools(Arc::clone(ws));
            }
            t
        });

        // 4. Create SafetyLayer (injection check configurable via builder).
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: self.injection_check,
        }));

        // 5. Create other deps.
        let hooks = Arc::new(HookRegistry::new());
        let cost_guard = Arc::new(CostGuard::new(CostGuardConfig {
            max_cost_per_day_cents: None,
            max_actions_per_hour: None,
        }));

        let deps = AgentDeps {
            store: Some(Arc::clone(&db)),
            llm,
            cheap_llm: None,
            safety,
            tools,
            workspace,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: SkillsConfig::default(),
            hooks,
            cost_guard,
        };

        // 6. Create TestChannel (Arc) and TestChannelHandle.
        let test_channel = Arc::new(TestChannel::new());
        let handle = TestChannelHandle::new(Arc::clone(&test_channel));

        // 7. Create ChannelManager and add the handle.
        let channel_manager = ChannelManager::new();
        channel_manager.add(Box::new(handle)).await;
        let channels = Arc::new(channel_manager);

        // 8. Create test-friendly AgentConfig.
        let config = AgentConfig {
            name: "test-rig".to_string(),
            max_parallel_jobs: 1,
            job_timeout: Duration::from_secs(30),
            stuck_threshold: Duration::from_secs(300),
            repair_check_interval: Duration::from_secs(3600), // Very high -- no repair in tests.
            max_repair_attempts: 0,
            use_planning: false,
            session_idle_timeout: Duration::from_secs(3600),
            allow_local_tools: true,
            max_cost_per_day_cents: None,
            max_actions_per_hour: None,
            max_tool_iterations: self.max_tool_iterations,
            auto_approve_tools: true,
        };

        // 9. Create Agent.
        let agent = Agent::new(
            config, deps, channels, None, // heartbeat_config
            None, // hygiene_config
            None, // routine_config
            None, // context_manager
            None, // session_manager
        );

        // 10. Spawn agent in background task.
        let agent_handle = tokio::spawn(async move {
            if let Err(e) = agent.run().await {
                eprintln!("[TestRig] Agent exited with error: {e}");
            }
        });

        // 11. Give the agent a moment to start its event loop.
        tokio::time::sleep(Duration::from_millis(100)).await;

        TestRig {
            channel: test_channel,
            instrumented_llm: instrumented,
            start_time: Instant::now(),
            agent_handle: Some(agent_handle),
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
    /// Check if any captured status events contain safety/injection warnings.
    pub fn has_safety_warnings(&self) -> bool {
        self.captured_status_events().iter().any(|s| {
            matches!(s, StatusUpdate::Status(msg) if msg.contains("sanitiz") || msg.contains("inject") || msg.contains("warning"))
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::trace_llm::{LlmTrace, TraceResponse, TraceStep};

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_rig_builds_and_runs() {
        // Create a simple 1-step text trace.
        let trace = LlmTrace {
            model_name: "test-model".to_string(),
            steps: vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "I am the test rig response.".to_string(),
                    input_tokens: 50,
                    output_tokens: 15,
                },
            }],
        };

        // Build the rig.
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        // Send a message.
        rig.send_message("Hello test rig").await;

        // Wait for a response (up to 10 seconds).
        let responses = rig.wait_for_responses(1, Duration::from_secs(10)).await;

        // Verify we got at least one response containing the trace text.
        assert!(
            !responses.is_empty(),
            "Expected at least one response from the agent"
        );
        let found = responses
            .iter()
            .any(|r| r.content.contains("I am the test rig response."));
        assert!(
            found,
            "Expected a response containing the trace text, got: {:?}",
            responses.iter().map(|r| &r.content).collect::<Vec<_>>()
        );

        // Shutdown.
        rig.shutdown();
    }
}

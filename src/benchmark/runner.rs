//! Benchmark runner -- loads scenarios, wires up a real agent per scenario, and
//! collects metrics.
//!
//! Feature-gated on `libsql` because it uses `LibSqlBackend::new_memory()` for
//! zero-dependency ephemeral databases.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
use crate::agent::{Agent, AgentDeps};
use crate::benchmark::bench_channel::{BenchChannel, BenchChannelHandle};
use crate::benchmark::instrumented::InstrumentedLlm;
use crate::benchmark::judge::judge_turn;
use crate::benchmark::metrics::{RunResult, ScenarioResult, ToolInvocation, TraceMetrics, TurnMetrics};
use crate::benchmark::scenario::{BenchScenario, EvalContext, Scenario};
use crate::channels::{ChannelManager, IncomingMessage};
use crate::config::{AgentConfig, SafetyConfig, SkillsConfig};
use crate::db::Database;
use crate::hooks::HookRegistry;
use crate::llm::LlmProvider;
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;

/// Configuration for a benchmark run.
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    /// Directory containing scenario JSON files.
    pub scenarios_dir: PathBuf,
    /// Global timeout in seconds (applies per-scenario if the scenario's own
    /// timeout is larger).
    pub global_timeout_secs: u64,
    /// Optional scenario ID filter (substring match).
    pub filter: Option<String>,
    /// Optional category filter (exact match).
    pub category_filter: Option<String>,
    /// Optional tag filter for `BenchScenario` loading. A scenario must have
    /// at least one matching tag to be included.
    pub tags_filter: Option<Vec<String>>,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            scenarios_dir: PathBuf::from("benchmarks/scenarios"),
            global_timeout_secs: 120,
            filter: None,
            category_filter: None,
            tags_filter: None,
        }
    }
}

/// Load all scenarios from the configured directory, applying optional filters.
pub fn load_scenarios(config: &BenchmarkConfig) -> Result<Vec<Scenario>, String> {
    let dir = &config.scenarios_dir;
    if !dir.exists() {
        return Err(format!("Scenarios directory not found: {}", dir.display()));
    }

    let mut scenarios = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read scenarios dir: {e}"))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        let file_scenarios: Vec<Scenario> = serde_json::from_str(&contents)
            .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
        scenarios.extend(file_scenarios);
    }

    // Apply filters.
    if let Some(ref filter) = config.filter {
        scenarios.retain(|s| s.id.contains(filter.as_str()));
    }
    if let Some(ref category) = config.category_filter {
        scenarios.retain(|s| s.category == *category);
    }

    Ok(scenarios)
}

/// Recursively discover `.json` files under `dir` and deserialize each as a
/// single `BenchScenario`, appending to `scenarios`.
fn load_json_recursive(
    dir: &std::path::Path,
    scenarios: &mut Vec<(PathBuf, BenchScenario)>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry in {}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            load_json_recursive(&path, scenarios)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let contents = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
            let scenario: BenchScenario = serde_json::from_str(&contents)
                .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
            scenarios.push((path, scenario));
        }
    }
    Ok(())
}

/// Load all `BenchScenario`s from the configured trajectories directory,
/// applying optional name and tag filters.
///
/// Each `.json` file in (or under) `config.scenarios_dir` is expected to
/// contain a single `BenchScenario` object. Subdirectories are traversed
/// recursively. Results are sorted by file path for deterministic ordering.
pub fn load_bench_scenarios(config: &BenchmarkConfig) -> Result<Vec<BenchScenario>, String> {
    let dir = &config.scenarios_dir;
    if !dir.exists() {
        return Err(format!(
            "Trajectories directory not found: {}",
            dir.display()
        ));
    }

    let mut entries: Vec<(PathBuf, BenchScenario)> = Vec::new();
    load_json_recursive(dir, &mut entries)?;

    // Sort by file path for deterministic ordering.
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));

    let mut scenarios: Vec<BenchScenario> = entries.into_iter().map(|(_, s)| s).collect();

    // Apply name substring filter.
    if let Some(ref filter) = config.filter {
        scenarios.retain(|s| s.name.contains(filter.as_str()));
    }

    // Apply tag filter: scenario must have at least one matching tag.
    if let Some(ref tags) = config.tags_filter {
        scenarios.retain(|s| s.tags.iter().any(|t| tags.contains(t)));
    }

    Ok(scenarios)
}

/// Run a single scenario against the given LLM provider.
///
/// Creates an ephemeral in-memory database, wires a real agent with a
/// `BenchChannel`, sends the scenario input, waits for a response, evaluates
/// criteria, and returns a `ScenarioResult`.
pub async fn run_scenario(
    scenario: &Scenario,
    llm: Arc<dyn LlmProvider>,
    global_timeout_secs: u64,
) -> ScenarioResult {
    use crate::db::libsql::LibSqlBackend;

    let scenario_start = Instant::now();
    let timeout_secs = scenario.timeout_secs.min(global_timeout_secs);
    let timeout = Duration::from_secs(timeout_secs);

    // 1. Create in-memory database + run migrations.
    let backend = match LibSqlBackend::new_memory().await {
        Ok(b) => b,
        Err(e) => {
            return ScenarioResult {
                scenario_id: scenario.id.clone(),
                passed: false,
                trace: empty_trace(scenario_start),
                response: String::new(),
                error: Some(format!("Failed to create database: {e}")),
                turn_metrics: Vec::new(),
            };
        }
    };
    if let Err(e) = backend.run_migrations().await {
        return ScenarioResult {
            scenario_id: scenario.id.clone(),
            passed: false,
            trace: empty_trace(scenario_start),
            response: String::new(),
            error: Some(format!("Failed to run migrations: {e}")),
            turn_metrics: Vec::new(),
        };
    }
    let db: Arc<dyn Database> = Arc::new(backend);

    // 2. Wrap LLM in InstrumentedLlm.
    let instrumented = Arc::new(InstrumentedLlm::new(llm));
    let llm_for_agent: Arc<dyn LlmProvider> = Arc::clone(&instrumented) as Arc<dyn LlmProvider>;

    // 3. Create workspace + tools.
    let workspace = Some(Arc::new(crate::workspace::Workspace::new_with_db(
        "bench-user",
        Arc::clone(&db),
    )));
    let tools = Arc::new(ToolRegistry::new());
    tools.register_builtin_tools();
    if let Some(ref ws) = workspace {
        tools.register_memory_tools(Arc::clone(ws));
    }

    // 4. Safety layer, hooks, cost guard.
    let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
        max_output_length: 100_000,
        injection_check_enabled: false,
    }));
    let hooks = Arc::new(HookRegistry::new());
    let cost_guard = Arc::new(CostGuard::new(CostGuardConfig {
        max_cost_per_day_cents: None,
        max_actions_per_hour: None,
    }));

    // 5. Create BenchChannel.
    let (msg_tx, msg_rx) = mpsc::channel::<IncomingMessage>(64);
    let bench_channel = Arc::new(BenchChannel::new(msg_rx));
    let handle = BenchChannelHandle::new(Arc::clone(&bench_channel));

    let channel_manager = ChannelManager::new();
    channel_manager.add(Box::new(handle)).await;
    let channels = Arc::new(channel_manager);

    // 6. Agent config.
    let agent_config = AgentConfig {
        name: "benchmark".to_string(),
        max_parallel_jobs: 1,
        job_timeout: Duration::from_secs(timeout_secs),
        stuck_threshold: Duration::from_secs(300),
        repair_check_interval: Duration::from_secs(3600),
        max_repair_attempts: 0,
        use_planning: false,
        session_idle_timeout: Duration::from_secs(3600),
        allow_local_tools: true,
        max_cost_per_day_cents: None,
        max_actions_per_hour: None,
        max_tool_iterations: scenario.max_tool_iterations,
        auto_approve_tools: true,
    };

    // 7. Build deps.
    let deps = AgentDeps {
        store: Some(Arc::clone(&db)),
        llm: llm_for_agent,
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

    // 8. Create and spawn agent.
    let agent = Agent::new(agent_config, deps, channels, None, None, None, None, None);
    let agent_handle = tokio::spawn(async move {
        if let Err(e) = agent.run().await {
            tracing::debug!("[benchmark] Agent exited: {e}");
        }
    });

    // Give the agent a moment to start.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 9. Inject the scenario message.
    let incoming = IncomingMessage::new("benchmark", "bench-user", &scenario.input);
    if msg_tx.send(incoming).await.is_err() {
        agent_handle.abort();
        return ScenarioResult {
            scenario_id: scenario.id.clone(),
            passed: false,
            trace: empty_trace(scenario_start),
            response: String::new(),
            error: Some("Failed to send message to agent".to_string()),
            turn_metrics: Vec::new(),
        };
    }

    // 10. Wait for a response (with timeout).
    let response = tokio::time::timeout(timeout, bench_channel.wait_for_response()).await;

    let hit_timeout = response.is_err();
    let response_text = response.unwrap_or_default();

    // 11. Collect tool metrics.
    let tool_calls_completed = bench_channel.tool_calls_completed().await;
    let tool_timings = bench_channel.tool_timings().await;

    // Build tool invocation records.
    let mut timing_map: std::collections::HashMap<String, Vec<u64>> =
        std::collections::HashMap::new();
    for (name, ms) in &tool_timings {
        timing_map.entry(name.clone()).or_default().push(*ms);
    }
    let tool_invocations: Vec<ToolInvocation> = tool_calls_completed
        .iter()
        .map(|(name, success)| {
            let duration_ms = timing_map
                .get_mut(name)
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

    let hit_iteration_limit = tool_calls_completed.len() >= scenario.max_tool_iterations;

    let trace = TraceMetrics {
        wall_time_ms: scenario_start.elapsed().as_millis() as u64,
        llm_calls: instrumented.call_count(),
        input_tokens: instrumented.total_input_tokens(),
        output_tokens: instrumented.total_output_tokens(),
        estimated_cost_usd: instrumented.estimated_cost_usd(),
        tool_calls: tool_invocations,
        turns: 1,
        hit_iteration_limit,
        hit_timeout,
    };

    // 12. Evaluate criteria.
    let eval_ctx = EvalContext {
        response: response_text.clone(),
        tool_calls: tool_calls_completed,
    };
    let mut all_passed = !hit_timeout;
    let mut error_reasons = Vec::new();
    for criterion in &scenario.success_criteria {
        let result = criterion.evaluate(&eval_ctx);
        if !result.passed {
            all_passed = false;
            error_reasons.push(result.reason);
        }
    }

    // 13. Cleanup: abort the agent.
    agent_handle.abort();

    ScenarioResult {
        scenario_id: scenario.id.clone(),
        passed: all_passed,
        trace,
        response: response_text,
        error: if error_reasons.is_empty() {
            None
        } else {
            Some(error_reasons.join("; "))
        },
        turn_metrics: Vec::new(),
    }
}

/// Run all scenarios matching the config against the given LLM provider.
pub async fn run_all(
    config: &BenchmarkConfig,
    llm: Arc<dyn LlmProvider>,
) -> Result<RunResult, String> {
    let scenarios = load_scenarios(config)?;
    if scenarios.is_empty() {
        return Err("No scenarios matched the given filters".to_string());
    }

    tracing::info!("Running {} benchmark scenario(s)", scenarios.len());

    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in &scenarios {
        tracing::info!(
            "[bench] Running scenario: {} (category: {})",
            scenario.id,
            scenario.category
        );
        let result = run_scenario(scenario, Arc::clone(&llm), config.global_timeout_secs).await;

        let status = if result.passed { "PASS" } else { "FAIL" };
        tracing::info!(
            "[bench] {} -- {} ({}ms, {} LLM calls)",
            scenario.id,
            status,
            result.trace.wall_time_ms,
            result.trace.llm_calls,
        );

        results.push(result);
    }

    let run_id = format!("bench-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let mut run_result = RunResult::from_scenarios(run_id, results);
    if let Some(hash) = git_commit_hash() {
        run_result = run_result.with_commit_hash(hash);
    }

    Ok(run_result)
}

/// Seed workspace with documents from setup configuration.
async fn seed_workspace(
    workspace: &crate::workspace::Workspace,
    setup: &crate::benchmark::scenario::WorkspaceSetup,
) -> Result<(), String> {
    for doc in &setup.documents {
        workspace
            .write(&doc.path, &doc.content)
            .await
            .map_err(|e| format!("Failed to seed document '{}': {e}", doc.path))?;
    }

    // Load fixtures from directory if specified.
    if let Some(ref fixtures_dir) = setup.fixtures_dir {
        let dir = std::path::Path::new(fixtures_dir);
        if dir.exists() && dir.is_dir() {
            load_fixtures_recursive(workspace, dir, dir).await?;
        }
    }

    Ok(())
}

/// Recursively load fixture files from a directory into the workspace.
async fn load_fixtures_recursive(
    workspace: &crate::workspace::Workspace,
    base_dir: &std::path::Path,
    current_dir: &std::path::Path,
) -> Result<(), String> {
    let entries = std::fs::read_dir(current_dir)
        .map_err(|e| format!("Failed to read fixtures dir {}: {e}", current_dir.display()))?;

    for entry in entries {
        let entry =
            entry.map_err(|e| format!("Failed to read entry in {}: {e}", current_dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            Box::pin(load_fixtures_recursive(workspace, base_dir, &path)).await?;
        } else {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read fixture {}: {e}", path.display()))?;
            let relative = path
                .strip_prefix(base_dir)
                .map_err(|e| format!("Failed to strip prefix: {e}"))?;
            let ws_path = format!("fixtures/{}", relative.display());
            workspace
                .write(&ws_path, &content)
                .await
                .map_err(|e| format!("Failed to write fixture '{}': {e}", ws_path))?;
        }
    }
    Ok(())
}

/// Build tool invocation records from BenchChannel data.
fn build_tool_invocations(
    tool_calls_completed: &[(String, bool)],
    tool_timings: &[(String, u64)],
) -> Vec<ToolInvocation> {
    let mut timing_map: std::collections::HashMap<String, Vec<u64>> =
        std::collections::HashMap::new();
    for (name, ms) in tool_timings {
        timing_map.entry(name.clone()).or_default().push(*ms);
    }
    tool_calls_completed
        .iter()
        .map(|(name, success)| {
            let duration_ms = timing_map
                .get_mut(name)
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
        .collect()
}

/// Create an error ScenarioResult for early-return error cases in bench scenarios.
fn error_result(name: &str, start: Instant, error: String) -> ScenarioResult {
    ScenarioResult {
        scenario_id: name.to_string(),
        passed: false,
        trace: empty_trace(start),
        response: String::new(),
        error: Some(error),
        turn_metrics: Vec::new(),
    }
}

/// Run a single `BenchScenario` against the given LLM provider.
///
/// Creates an ephemeral in-memory database, seeds workspace documents, wires a
/// real agent with a `BenchChannel`, runs all turns sequentially, collects
/// per-turn metrics and assertions, and returns a `ScenarioResult`.
pub async fn run_bench_scenario(
    scenario: &BenchScenario,
    llm: Arc<dyn LlmProvider>,
    global_timeout_secs: u64,
) -> ScenarioResult {
    use crate::db::libsql::LibSqlBackend;

    let scenario_start = Instant::now();
    let timeout_secs = scenario.timeout_secs.min(global_timeout_secs);
    let timeout = Duration::from_secs(timeout_secs);

    // 1. Create in-memory database + run migrations.
    let backend = match LibSqlBackend::new_memory().await {
        Ok(b) => b,
        Err(e) => {
            return error_result(&scenario.name, scenario_start, format!("Failed to create database: {e}"));
        }
    };
    if let Err(e) = backend.run_migrations().await {
        return error_result(&scenario.name, scenario_start, format!("Failed to run migrations: {e}"));
    }
    let db: Arc<dyn Database> = Arc::new(backend);

    // 2. Wrap LLM in InstrumentedLlm.
    let instrumented = Arc::new(InstrumentedLlm::new(llm));
    let llm_for_agent: Arc<dyn LlmProvider> = Arc::clone(&instrumented) as Arc<dyn LlmProvider>;

    // 3. Create workspace + tools.
    let workspace = Some(Arc::new(crate::workspace::Workspace::new_with_db(
        "bench-user",
        Arc::clone(&db),
    )));
    let tools = Arc::new(ToolRegistry::new());
    tools.register_builtin_tools();
    if let Some(ref ws) = workspace {
        tools.register_memory_tools(Arc::clone(ws));
    }

    // 4. Seed workspace documents from setup.
    if let Some(ref ws_setup) = scenario.setup.workspace
        && let Some(ref ws) = workspace
        && let Err(e) = seed_workspace(ws, ws_setup).await
    {
        return error_result(&scenario.name, scenario_start, format!("Failed to seed workspace: {e}"));
    }

    // 5. Safety layer, hooks, cost guard.
    let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
        max_output_length: 100_000,
        injection_check_enabled: false,
    }));
    let hooks = Arc::new(HookRegistry::new());
    let cost_guard = Arc::new(CostGuard::new(CostGuardConfig {
        max_cost_per_day_cents: None,
        max_actions_per_hour: None,
    }));

    // 6. Create BenchChannel.
    let (msg_tx, msg_rx) = mpsc::channel::<IncomingMessage>(64);
    let bench_channel = Arc::new(BenchChannel::new(msg_rx));
    let handle = BenchChannelHandle::new(Arc::clone(&bench_channel));

    let channel_manager = ChannelManager::new();
    channel_manager.add(Box::new(handle)).await;
    let channels = Arc::new(channel_manager);

    // 7. Agent config.
    let agent_config = AgentConfig {
        name: "benchmark".to_string(),
        max_parallel_jobs: 1,
        job_timeout: Duration::from_secs(timeout_secs),
        stuck_threshold: Duration::from_secs(300),
        repair_check_interval: Duration::from_secs(3600),
        max_repair_attempts: 0,
        use_planning: false,
        session_idle_timeout: Duration::from_secs(3600),
        allow_local_tools: true,
        max_cost_per_day_cents: None,
        max_actions_per_hour: None,
        max_tool_iterations: scenario.max_tool_iterations,
        auto_approve_tools: true,
    };

    // 8. Build deps.
    let deps = AgentDeps {
        store: Some(Arc::clone(&db)),
        llm: llm_for_agent,
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

    // 9. Create and spawn agent.
    let agent = Agent::new(agent_config, deps, channels, None, None, None, None, None);
    let agent_handle = tokio::spawn(async move {
        if let Err(e) = agent.run().await {
            tracing::debug!("[benchmark] Agent exited: {e}");
        }
    });

    // Give the agent a moment to start.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 10. Run turns sequentially.
    let mut turn_metrics_list = Vec::with_capacity(scenario.turns.len());
    let mut all_turns_passed = true;
    let mut last_response = String::new();
    let mut aggregated_errors = Vec::new();

    for (turn_idx, turn) in scenario.turns.iter().enumerate() {
        let turn_start = Instant::now();
        let tokens_before_input = instrumented.total_input_tokens();
        let tokens_before_output = instrumented.total_output_tokens();
        let calls_before = instrumented.call_count();

        // Send user message.
        let incoming = IncomingMessage::new("benchmark", "bench-user", &turn.user);
        if msg_tx.send(incoming).await.is_err() {
            agent_handle.abort();
            return error_result(
                &scenario.name,
                scenario_start,
                format!("Failed to send message for turn {turn_idx}"),
            );
        }

        // Wait for response (with remaining timeout).
        let elapsed = scenario_start.elapsed();
        let remaining = timeout.saturating_sub(elapsed);
        let response = tokio::time::timeout(remaining, bench_channel.wait_for_response()).await;

        let hit_timeout = response.is_err();
        let response_text = response.unwrap_or_default();

        // Collect per-turn tool metrics.
        let tool_calls_completed = bench_channel.tool_calls_completed().await;
        let tool_timings = bench_channel.tool_timings().await;
        let tool_invocations = build_tool_invocations(&tool_calls_completed, &tool_timings);

        // Per-turn token/call deltas.
        let turn_input_tokens = instrumented.total_input_tokens() - tokens_before_input;
        let turn_output_tokens = instrumented.total_output_tokens() - tokens_before_output;
        let turn_llm_calls = instrumented.call_count() - calls_before;

        // Evaluate per-turn assertions.
        let eval_ctx = EvalContext {
            response: response_text.clone(),
            tool_calls: tool_calls_completed,
        };
        let criteria = turn.assertions.to_criteria();
        let mut turn_passed = !hit_timeout;
        let mut turn_errors = Vec::new();

        for criterion in &criteria {
            let result = criterion.evaluate(&eval_ctx);
            if !result.passed {
                turn_passed = false;
                turn_errors.push(result.reason.clone());
            }
        }

        // Check circuit-breaker assertions (latency, cost).
        if let Some(max_latency) = turn.assertions.max_latency_secs {
            let turn_secs = turn_start.elapsed().as_secs_f64();
            if turn_secs > max_latency {
                turn_passed = false;
                turn_errors.push(format!(
                    "Turn latency {turn_secs:.1}s exceeded max {max_latency:.1}s"
                ));
            }
        }

        if hit_timeout {
            turn_errors.push("Turn timed out".to_string());
        }

        // LLM-as-judge scoring (if configured for this turn).
        let judge_score = if let Some(ref judge_config) = turn.judge {
            let llm_for_judge: Arc<dyn LlmProvider> =
                Arc::clone(&instrumented) as Arc<dyn LlmProvider>;
            let score = judge_turn(
                &llm_for_judge,
                &turn.user,
                &response_text,
                &eval_ctx.tool_calls,
                &judge_config.criteria,
            )
            .await;
            if let Some(s) = score
                && s < judge_config.min_score
            {
                turn_passed = false;
                turn_errors.push(format!(
                    "Judge score {s} is below minimum {}",
                    judge_config.min_score
                ));
            }
            score
        } else {
            None
        };

        if !turn_passed {
            all_turns_passed = false;
            aggregated_errors.extend(turn_errors.iter().map(|e| format!("turn {turn_idx}: {e}")));
        }

        last_response.clone_from(&response_text);

        turn_metrics_list.push(TurnMetrics {
            turn_index: turn_idx,
            user_message: turn.user.clone(),
            wall_time_ms: turn_start.elapsed().as_millis() as u64,
            llm_calls: turn_llm_calls,
            input_tokens: turn_input_tokens,
            output_tokens: turn_output_tokens,
            tool_calls: tool_invocations,
            response: response_text,
            assertions_passed: turn_passed,
            judge_score,
            errors: turn_errors,
        });

        // Clear channel state for next turn.
        bench_channel.clear_for_next_turn().await;

        // If we hit the timeout, don't continue with more turns.
        if hit_timeout {
            break;
        }
    }

    // 11. Build aggregate trace.
    let trace = TraceMetrics {
        wall_time_ms: scenario_start.elapsed().as_millis() as u64,
        llm_calls: instrumented.call_count(),
        input_tokens: instrumented.total_input_tokens(),
        output_tokens: instrumented.total_output_tokens(),
        estimated_cost_usd: instrumented.estimated_cost_usd(),
        tool_calls: turn_metrics_list
            .iter()
            .flat_map(|t| t.tool_calls.clone())
            .collect(),
        turns: turn_metrics_list.len() as u32,
        hit_iteration_limit: false,
        hit_timeout: turn_metrics_list.last().is_some_and(|t| {
            t.errors.iter().any(|e| e.contains("timed out"))
        }),
    };

    // 12. Cleanup: abort the agent.
    agent_handle.abort();

    ScenarioResult {
        scenario_id: scenario.name.clone(),
        passed: all_turns_passed,
        trace,
        response: last_response,
        error: if aggregated_errors.is_empty() {
            None
        } else {
            Some(aggregated_errors.join("; "))
        },
        turn_metrics: turn_metrics_list,
    }
}

/// Run all `BenchScenario`s matching the config against the given LLM provider.
pub async fn run_all_bench(
    config: &BenchmarkConfig,
    llm: Arc<dyn LlmProvider>,
) -> Result<RunResult, String> {
    let scenarios = load_bench_scenarios(config)?;
    if scenarios.is_empty() {
        return Err("No bench scenarios matched the given filters".to_string());
    }

    tracing::info!("Running {} bench scenario(s)", scenarios.len());

    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in &scenarios {
        tracing::info!(
            "[bench] Running scenario: {} (tags: {:?})",
            scenario.name,
            scenario.tags
        );
        let result =
            run_bench_scenario(scenario, Arc::clone(&llm), config.global_timeout_secs).await;

        let status = if result.passed { "PASS" } else { "FAIL" };
        tracing::info!(
            "[bench] {} -- {} ({}ms, {} LLM calls, {} turns)",
            scenario.name,
            status,
            result.trace.wall_time_ms,
            result.trace.llm_calls,
            result.trace.turns,
        );

        results.push(result);
    }

    let run_id = format!("bench-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let mut run_result = RunResult::from_scenarios(run_id, results);
    if let Some(hash) = git_commit_hash() {
        run_result = run_result.with_commit_hash(hash);
    }

    Ok(run_result)
}

/// Try to get the current git commit hash for reproducibility.
fn git_commit_hash() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
}

/// Create an empty TraceMetrics for error cases.
fn empty_trace(start: Instant) -> TraceMetrics {
    TraceMetrics {
        wall_time_ms: start.elapsed().as_millis() as u64,
        llm_calls: 0,
        input_tokens: 0,
        output_tokens: 0,
        estimated_cost_usd: 0.0,
        tool_calls: Vec::new(),
        turns: 0,
        hit_iteration_limit: false,
        hit_timeout: false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_scenarios_from_repo() {
        let config = BenchmarkConfig::default();
        // Only run if the scenarios directory exists (i.e., in the repo).
        if !config.scenarios_dir.exists() {
            return;
        }
        let scenarios = load_scenarios(&config).expect("should load scenarios");
        assert!(!scenarios.is_empty(), "expected at least one scenario");
    }

    #[test]
    fn test_load_scenarios_with_filter() {
        let config = BenchmarkConfig {
            filter: Some("ts-time".to_string()),
            ..BenchmarkConfig::default()
        };
        if !config.scenarios_dir.exists() {
            return;
        }
        let scenarios = load_scenarios(&config).expect("should load scenarios");
        for s in &scenarios {
            assert!(
                s.id.contains("ts-time"),
                "filter should only include matching IDs, got: {}",
                s.id
            );
        }
    }

    #[test]
    fn test_load_scenarios_with_category_filter() {
        let config = BenchmarkConfig {
            category_filter: Some("tool_selection".to_string()),
            ..BenchmarkConfig::default()
        };
        if !config.scenarios_dir.exists() {
            return;
        }
        let scenarios = load_scenarios(&config).expect("should load scenarios");
        for s in &scenarios {
            assert_eq!(
                s.category, "tool_selection",
                "category filter should only include matching categories"
            );
        }
    }

    #[test]
    fn test_git_commit_hash() {
        // Should return Some in a git repo, None otherwise.
        let hash = git_commit_hash();
        if let Some(ref h) = hash {
            assert!(!h.is_empty(), "hash should not be empty");
            assert!(h.len() <= 12, "short hash should be <= 12 chars");
        }
    }

    // -----------------------------------------------------------------------
    // BenchScenario loader tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_load_bench_scenarios_from_trajectories() {
        let config = BenchmarkConfig {
            scenarios_dir: PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/benchmarks/trajectories"
            )),
            ..BenchmarkConfig::default()
        };
        if !config.scenarios_dir.exists() {
            return;
        }
        let scenarios =
            load_bench_scenarios(&config).expect("should load bench scenarios from trajectories");
        assert!(
            !scenarios.is_empty(),
            "expected at least one BenchScenario in benchmarks/trajectories/"
        );
    }

    #[test]
    fn test_load_bench_scenarios_with_tag_filter() {
        let config = BenchmarkConfig {
            scenarios_dir: PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/benchmarks/trajectories"
            )),
            tags_filter: Some(vec!["tools".to_string()]),
            ..BenchmarkConfig::default()
        };
        if !config.scenarios_dir.exists() {
            return;
        }
        let scenarios =
            load_bench_scenarios(&config).expect("should load bench scenarios with tag filter");
        assert!(
            !scenarios.is_empty(),
            "expected at least one scenario with tag 'tools'"
        );
        for s in &scenarios {
            assert!(
                s.tags.contains(&"tools".to_string()),
                "scenario '{}' should have tag 'tools', got tags: {:?}",
                s.name,
                s.tags
            );
        }
    }

    #[test]
    fn test_load_bench_scenarios_with_name_filter() {
        let config = BenchmarkConfig {
            scenarios_dir: PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/benchmarks/trajectories"
            )),
            filter: Some("pick-time".to_string()),
            ..BenchmarkConfig::default()
        };
        if !config.scenarios_dir.exists() {
            return;
        }
        let scenarios =
            load_bench_scenarios(&config).expect("should load bench scenarios with name filter");
        assert!(
            !scenarios.is_empty(),
            "expected at least one scenario matching 'pick-time'"
        );
        for s in &scenarios {
            assert!(
                s.name.contains("pick-time"),
                "scenario '{}' should contain 'pick-time' in name",
                s.name
            );
        }
    }
}

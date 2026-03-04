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
use crate::benchmark::metrics::{RunResult, ScenarioResult, ToolInvocation, TraceMetrics};
use crate::benchmark::scenario::{EvalContext, Scenario};
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
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            scenarios_dir: PathBuf::from("benchmarks/scenarios"),
            global_timeout_secs: 120,
            filter: None,
            category_filter: None,
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
}

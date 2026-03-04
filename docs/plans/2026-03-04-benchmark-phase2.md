# Benchmark Phase 2: YAML Scenarios, Multi-Turn, CLI, LLM-as-Judge

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the gaps between the Phase 1 benchmark foundation (PR #512) and the full spec in issue #467 / design gist.

**Architecture:** Extend the existing `src/benchmark/` module. Migrate scenario format from JSON to YAML with a rich `setup:` block. Refactor `run_scenario` to loop over multiple turns. Add `ResponseNotContains` + per-turn circuit breakers. Add LLM-as-judge scoring as an optional evaluation layer. Expose everything through a `Benchmark` CLI subcommand on `ironclaw benchmark`.

**Tech Stack:** Rust, serde_yml (already in Cargo.toml), clap (already in Cargo.toml), tokio, libsql

---

### Task 1: Extend Scenario Types for YAML + Multi-Turn + Setup

**Files:**
- Modify: `src/benchmark/scenario.rs`
- Test: inline `#[cfg(test)] mod tests`

This task replaces the flat JSON `Scenario` struct with the YAML-native types from the spec. The old `Scenario` gets replaced by `BenchScenario` with `setup`, `turns`, and `tags`.

**Step 1: Write the failing tests**

Add these tests to the bottom of `src/benchmark/scenario.rs` (inside `mod tests`):

```rust
#[test]
fn test_bench_scenario_yaml_deserialize() {
    let yaml = r#"
name: test-echo
description: Test basic echo
tags: [tools, basic]
setup:
  tools: [echo, time]
turns:
  - user: "Echo hello"
    assertions:
      tools_called: [echo]
      response_contains: ["hello"]
      max_tool_calls: 3
"#;
    let scenario: BenchScenario = serde_yml::from_str(yaml).unwrap();
    assert_eq!(scenario.name, "test-echo");
    assert_eq!(scenario.tags, vec!["tools", "basic"]);
    assert_eq!(scenario.setup.tools, Some(vec!["echo".into(), "time".into()]));
    assert_eq!(scenario.turns.len(), 1);
    assert_eq!(scenario.turns[0].user, "Echo hello");
    assert_eq!(scenario.turns[0].assertions.tools_called, Some(vec!["echo".into()]));
    assert_eq!(scenario.turns[0].assertions.max_tool_calls, Some(3));
}

#[test]
fn test_bench_scenario_multi_turn() {
    let yaml = r#"
name: save-and-recall
description: Multi-turn memory test
tags: [memory, multi-turn]
setup:
  tools: [memory_write, memory_search, memory_read]
  workspace:
    documents:
      - path: "context/project.md"
        content: "# Project Alpha\nLaunches March 15"
turns:
  - user: "Save a note: meeting at 3pm tomorrow"
    assertions:
      tools_called: [memory_write]
  - user: "What did I save about a meeting?"
    assertions:
      tools_called: [memory_search]
      response_contains: ["3pm"]
"#;
    let scenario: BenchScenario = serde_yml::from_str(yaml).unwrap();
    assert_eq!(scenario.turns.len(), 2);
    let docs = scenario.setup.workspace.as_ref().unwrap().documents.as_ref().unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].path, "context/project.md");
}

#[test]
fn test_bench_scenario_with_judge() {
    let yaml = r#"
name: quality-check
description: Test with judge scoring
tags: [quality]
turns:
  - user: "Explain how memory works"
    judge:
      criteria: |
        Is the explanation clear and accurate?
        Does it reference workspace memory?
      min_score: 7
"#;
    let scenario: BenchScenario = serde_yml::from_str(yaml).unwrap();
    let judge = scenario.turns[0].judge.as_ref().unwrap();
    assert_eq!(judge.min_score, 7);
    assert!(judge.criteria.contains("clear and accurate"));
}

#[test]
fn test_bench_scenario_with_identity() {
    let yaml = r#"
name: identity-test
description: Test identity overrides
tags: [identity]
setup:
  identity:
    USER.md: |
      Name: Test User
      Timezone: UTC
turns:
  - user: "What's my name?"
    assertions:
      response_contains: ["Test User"]
"#;
    let scenario: BenchScenario = serde_yml::from_str(yaml).unwrap();
    let identity = scenario.setup.identity.as_ref().unwrap();
    assert!(identity.contains_key("USER.md"));
}

#[test]
fn test_turn_assertions_circuit_breakers() {
    let yaml = r#"
name: circuit-breaker-test
description: Test circuit breakers
tags: [efficiency]
turns:
  - user: "Do something"
    assertions:
      max_tool_calls: 5
      max_cost_usd: 0.10
      max_latency_secs: 30
"#;
    let scenario: BenchScenario = serde_yml::from_str(yaml).unwrap();
    let a = &scenario.turns[0].assertions;
    assert_eq!(a.max_tool_calls, Some(5));
    assert_eq!(a.max_cost_usd, Some(0.10));
    assert_eq!(a.max_latency_secs, Some(30));
}

#[test]
fn test_turn_assertions_response_not_contains() {
    let yaml = r#"
name: not-contains-test
description: Test response_not_contains
tags: [basic]
turns:
  - user: "Hello"
    assertions:
      response_not_contains: ["error", "sorry"]
"#;
    let scenario: BenchScenario = serde_yml::from_str(yaml).unwrap();
    let a = &scenario.turns[0].assertions;
    assert_eq!(a.response_not_contains, Some(vec!["error".into(), "sorry".into()]));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --features libsql benchmark::scenario::tests -- --nocapture 2>&1 | head -30`
Expected: FAIL -- `BenchScenario` doesn't exist yet.

**Step 3: Implement the new types**

Add these types to `src/benchmark/scenario.rs` (above the existing `Scenario` struct, which stays for backward compat):

```rust
use std::collections::HashMap;

/// A benchmark scenario in the YAML format from the design spec.
///
/// Supports multi-turn conversations, workspace seeding, identity overrides,
/// tool/skill restriction, per-turn assertions, and LLM-as-judge scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchScenario {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub setup: ScenarioSetup,
    pub turns: Vec<Turn>,
    /// Scenario-level timeout in seconds (applies to entire scenario).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Max tool iterations per turn (agent-level limit).
    #[serde(default = "default_max_tool_iterations")]
    pub max_tool_iterations: usize,
}

fn default_timeout() -> u64 { 120 }
fn default_max_tool_iterations() -> usize { 20 }

/// Environment setup for a benchmark scenario.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScenarioSetup {
    /// Tools to register (if None, registers all builtins).
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    /// Skills to activate.
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    /// Workspace documents to seed before execution.
    #[serde(default)]
    pub workspace: Option<WorkspaceSetup>,
    /// Identity file overrides (e.g., {"USER.md": "Name: Zaki"}).
    #[serde(default)]
    pub identity: Option<HashMap<String, String>>,
}

/// Workspace seeding configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSetup {
    /// Documents to write into workspace memory.
    #[serde(default)]
    pub documents: Option<Vec<SeedDocument>>,
    /// Directory of fixture files to load into workspace.
    #[serde(default)]
    pub fixtures_dir: Option<String>,
}

/// A document to seed into workspace memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedDocument {
    pub path: String,
    pub content: String,
}

/// A single conversation turn in a benchmark scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    /// The user message to send.
    pub user: String,
    /// Hard assertions for this turn.
    #[serde(default)]
    pub assertions: TurnAssertions,
    /// Optional LLM-as-judge scoring for this turn.
    #[serde(default)]
    pub judge: Option<JudgeConfig>,
}

/// Hard assertions evaluated per turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnAssertions {
    #[serde(default)]
    pub tools_called: Option<Vec<String>>,
    #[serde(default)]
    pub tools_not_called: Option<Vec<String>>,
    #[serde(default)]
    pub response_contains: Option<Vec<String>>,
    #[serde(default)]
    pub response_not_contains: Option<Vec<String>>,
    #[serde(default)]
    pub response_matches: Option<Vec<String>>,
    /// Circuit breaker: max tool calls this turn.
    #[serde(default)]
    pub max_tool_calls: Option<usize>,
    /// Circuit breaker: max cost in USD this turn.
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    /// Circuit breaker: max latency in seconds this turn.
    #[serde(default)]
    pub max_latency_secs: Option<u64>,
}

/// LLM-as-judge configuration for a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeConfig {
    /// Free-form criteria for the judge LLM to evaluate.
    pub criteria: String,
    /// Minimum acceptable score (1-10). Scores below this fail the turn.
    pub min_score: u8,
}
```

Also add a helper to convert `TurnAssertions` into the existing `Vec<Criterion>` format for backward compatibility with the evaluation engine:

```rust
impl TurnAssertions {
    /// Convert to the existing `Criterion` vector for evaluation.
    pub fn to_criteria(&self) -> Vec<Criterion> {
        let mut criteria = Vec::new();
        if let Some(ref tools) = self.tools_called {
            for tool in tools {
                criteria.push(Criterion::ToolUsed { tool: tool.clone() });
            }
        }
        if let Some(ref tools) = self.tools_not_called {
            for tool in tools {
                criteria.push(Criterion::ToolNotUsed { tool: tool.clone() });
            }
        }
        if let Some(ref texts) = self.response_contains {
            for text in texts {
                criteria.push(Criterion::ResponseContains { text: text.clone() });
            }
        }
        if let Some(ref texts) = self.response_not_contains {
            for text in texts {
                criteria.push(Criterion::ResponseNotContains { text: text.clone() });
            }
        }
        if let Some(ref patterns) = self.response_matches {
            for pattern in patterns {
                criteria.push(Criterion::ResponseMatches { pattern: pattern.clone() });
            }
        }
        if let Some(max) = self.max_tool_calls {
            criteria.push(Criterion::ToolCallCountMax { max });
        }
        criteria
    }
}
```

Add `ResponseNotContains` variant to the `Criterion` enum:

```rust
/// The agent's final response must NOT contain this text (case-insensitive).
ResponseNotContains { text: String },
```

And its `evaluate` implementation:

```rust
Criterion::ResponseNotContains { text } => {
    let contains = ctx.response.to_lowercase().contains(&text.to_lowercase());
    CriterionResult {
        criterion: format!("response_not_contains:{text}"),
        passed: !contains,
        reason: if !contains {
            format!("Response correctly does not contain '{text}'")
        } else {
            format!("Response contains '{text}' but should not")
        },
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --features libsql benchmark::scenario::tests -- --nocapture`
Expected: ALL PASS

**Step 5: Run clippy**

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: 0 warnings

**Step 6: Commit**

```bash
git add src/benchmark/scenario.rs
git commit -m "feat(benchmark): add YAML scenario types with multi-turn, setup, judge"
```

---

### Task 2: Migrate Scenario Loader from JSON to YAML

**Files:**
- Modify: `src/benchmark/runner.rs` (the `load_scenarios` function)
- Create: `benchmarks/trajectories/tool-selection.yaml` (migrate from JSON)
- Create: `benchmarks/trajectories/multi-turn/save-and-recall.yaml`
- Modify: `src/benchmark/scenario.rs` (update `test_load_scenario_files` test)
- Test: inline tests

**Step 1: Write the failing test**

Add to `src/benchmark/runner.rs` tests:

```rust
#[test]
fn test_load_yaml_scenarios() {
    let config = BenchmarkConfig {
        scenarios_dir: PathBuf::from("benchmarks/trajectories"),
        ..BenchmarkConfig::default()
    };
    if !config.scenarios_dir.exists() {
        return;
    }
    let scenarios = load_bench_scenarios(&config).expect("should load YAML scenarios");
    assert!(!scenarios.is_empty(), "expected at least one YAML scenario");
}

#[test]
fn test_load_yaml_scenarios_with_tag_filter() {
    let config = BenchmarkConfig {
        scenarios_dir: PathBuf::from("benchmarks/trajectories"),
        tags_filter: Some(vec!["tools".to_string()]),
        ..BenchmarkConfig::default()
    };
    if !config.scenarios_dir.exists() {
        return;
    }
    let scenarios = load_bench_scenarios(&config).expect("should load YAML scenarios");
    for s in &scenarios {
        assert!(
            s.tags.contains(&"tools".to_string()),
            "tag filter should only include matching tags, got: {:?}",
            s.tags
        );
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --features libsql benchmark::runner::tests -- --nocapture 2>&1 | head -20`
Expected: FAIL -- `load_bench_scenarios` and `tags_filter` don't exist.

**Step 3: Implement**

Add `tags_filter` to `BenchmarkConfig`:

```rust
/// Optional tag filter (scenario must have at least one matching tag).
pub tags_filter: Option<Vec<String>>,
```

And default it to `None`.

Add `load_bench_scenarios` function to `runner.rs`:

```rust
use crate::benchmark::scenario::BenchScenario;

/// Load YAML scenarios from the configured directory, recursively.
pub fn load_bench_scenarios(config: &BenchmarkConfig) -> Result<Vec<BenchScenario>, String> {
    let dir = &config.scenarios_dir;
    if !dir.exists() {
        return Err(format!("Scenarios directory not found: {}", dir.display()));
    }

    let mut scenarios = Vec::new();
    load_yaml_recursive(dir, &mut scenarios)?;

    // Apply filters.
    if let Some(ref filter) = config.filter {
        scenarios.retain(|s| s.name.contains(filter.as_str()));
    }
    if let Some(ref tags) = config.tags_filter {
        scenarios.retain(|s| tags.iter().any(|t| s.tags.contains(t)));
    }

    Ok(scenarios)
}

fn load_yaml_recursive(dir: &std::path::Path, scenarios: &mut Vec<BenchScenario>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read dir {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            load_yaml_recursive(&path, scenarios)?;
        } else if path.extension().is_some_and(|ext| ext == "yaml" || ext == "yml") {
            let contents = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
            let scenario: BenchScenario = serde_yml::from_str(&contents)
                .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
            scenarios.push(scenario);
        }
    }
    Ok(())
}
```

**Step 4: Create initial YAML scenario files**

Create `benchmarks/trajectories/tool-selection/pick-time-tool.yaml`:

```yaml
name: pick-time-tool
description: Agent should use the time tool, not shell
tags: [tools, basic]
setup:
  tools: [time, echo, shell]
timeout_secs: 30
max_tool_iterations: 5
turns:
  - user: "What time is it right now?"
    assertions:
      tools_called: [time]
      tools_not_called: [shell]
      response_matches: ["\\d{1,2}:\\d{2}"]
      max_tool_calls: 3
```

Create `benchmarks/trajectories/tool-selection/pick-echo-tool.yaml`:

```yaml
name: pick-echo-tool
description: Agent should use echo tool, not shell
tags: [tools, basic]
setup:
  tools: [echo, shell]
timeout_secs: 30
max_tool_iterations: 5
turns:
  - user: "Use the echo tool to say 'benchmark test'."
    assertions:
      tools_called: [echo]
      tools_not_called: [shell]
      response_contains: ["benchmark test"]
      max_tool_calls: 3
```

Create `benchmarks/trajectories/multi-turn/save-and-recall.yaml`:

```yaml
name: save-and-recall
description: Save a note in turn 1, recall it in turn 2
tags: [memory, multi-turn]
setup:
  tools: [memory_write, memory_search, memory_read]
timeout_secs: 60
max_tool_iterations: 10
turns:
  - user: "Save a note: Project Alpha launches on March 15th"
    assertions:
      tools_called: [memory_write]
      response_contains: ["saved", "note"]
  - user: "When does Project Alpha launch?"
    assertions:
      tools_called: [memory_search]
      response_contains: ["March 15"]
```

Create `benchmarks/trajectories/efficiency/simple-question.yaml`:

```yaml
name: simple-question
description: Simple math should need zero tool calls
tags: [efficiency, basic]
timeout_secs: 30
max_tool_iterations: 5
turns:
  - user: "What is 2 + 2?"
    assertions:
      response_contains: ["4"]
      max_tool_calls: 0
```

**Step 5: Run tests to verify they pass**

Run: `cargo test --features libsql benchmark::runner::tests -- --nocapture`
Expected: ALL PASS

**Step 6: Commit**

```bash
git add src/benchmark/runner.rs benchmarks/trajectories/
git commit -m "feat(benchmark): add YAML scenario loader with recursive discovery and tag filter"
```

---

### Task 3: Refactor Runner for Multi-Turn Execution + Workspace Seeding

**Files:**
- Modify: `src/benchmark/runner.rs`
- Modify: `src/benchmark/bench_channel.rs`
- Modify: `src/benchmark/metrics.rs`
- Test: inline tests

This is the biggest task. The current `run_scenario` sends one message and waits. We need to:
1. Seed workspace documents from `setup.workspace`
2. Loop over `turns`, sending each user message and collecting per-turn metrics
3. Support tool restriction from `setup.tools`
4. Track per-turn metrics in `TraceMetrics`

**Step 1: Add `clear_for_next_turn` to BenchChannel**

The BenchChannel currently accumulates responses. For multi-turn, we need to clear state between turns.

Add to `bench_channel.rs`:

```rust
/// Clear captured state for the next turn.
pub async fn clear_for_next_turn(&self) {
    self.responses.lock().await.clear();
    self.status_events.lock().await.clear();
    self.tool_start_times.lock().await.clear();
    self.tool_timings.lock().await.clear();
}
```

**Step 2: Add per-turn tracking to TraceMetrics**

Add to `metrics.rs`:

```rust
/// Per-turn metrics for multi-turn scenarios.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnMetrics {
    /// Which turn (0-indexed).
    pub turn_index: usize,
    /// The user message sent.
    pub user_message: String,
    /// Wall-clock time for this turn only.
    pub wall_time_ms: u64,
    /// LLM calls during this turn.
    pub llm_calls: u32,
    /// Input tokens during this turn.
    pub input_tokens: u32,
    /// Output tokens during this turn.
    pub output_tokens: u32,
    /// Tool invocations during this turn.
    pub tool_calls: Vec<ToolInvocation>,
    /// The agent's response text for this turn.
    pub response: String,
    /// Whether hard assertions passed for this turn.
    pub assertions_passed: bool,
    /// Judge score for this turn (if judge was configured).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_score: Option<u8>,
    /// Assertion error messages.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub errors: Vec<String>,
}
```

Add `turn_metrics` field to `ScenarioResult`:

```rust
/// Per-turn metrics for multi-turn scenarios.
#[serde(skip_serializing_if = "Vec::is_empty", default)]
pub turn_metrics: Vec<TurnMetrics>,
```

And update `ScenarioResult` struct and `RunResult::from_scenarios` accordingly.

**Step 3: Add `run_bench_scenario` function**

Add a new function `run_bench_scenario` to `runner.rs` that handles the full YAML scenario lifecycle:

```rust
/// Run a single YAML benchmark scenario with multi-turn support.
pub async fn run_bench_scenario(
    scenario: &BenchScenario,
    llm: Arc<dyn LlmProvider>,
    global_timeout_secs: u64,
) -> ScenarioResult {
    use crate::db::libsql::LibSqlBackend;

    let scenario_start = Instant::now();
    let timeout_secs = scenario.timeout_secs.min(global_timeout_secs);
    let timeout = Duration::from_secs(timeout_secs);

    // 1. Create in-memory database.
    let backend = match LibSqlBackend::new_memory().await {
        Ok(b) => b,
        Err(e) => return error_result(&scenario.name, scenario_start, &format!("DB create: {e}")),
    };
    if let Err(e) = backend.run_migrations().await {
        return error_result(&scenario.name, scenario_start, &format!("Migrations: {e}"));
    }
    let db: Arc<dyn Database> = Arc::new(backend);

    // 2. Wrap LLM.
    let instrumented = Arc::new(InstrumentedLlm::new(llm));
    let llm_for_agent: Arc<dyn LlmProvider> = Arc::clone(&instrumented) as Arc<dyn LlmProvider>;

    // 3. Create workspace + seed documents.
    let workspace = Some(Arc::new(crate::workspace::Workspace::new_with_db(
        "bench-user", Arc::clone(&db),
    )));
    if let Some(ref ws_setup) = scenario.setup.workspace {
        if let Some(ref ws) = workspace {
            if let Err(e) = seed_workspace(ws, ws_setup).await {
                return error_result(&scenario.name, scenario_start, &format!("Workspace seed: {e}"));
            }
        }
    }

    // 4. Register tools (restricted if setup.tools specified).
    let tools = Arc::new(ToolRegistry::new());
    if let Some(ref tool_names) = scenario.setup.tools {
        tools.register_builtin_tools();
        if let Some(ref ws) = workspace {
            tools.register_memory_tools(Arc::clone(ws));
        }
        // Note: tool restriction is advisory -- we register all then the assertion
        // checks which tools were actually called. Full tool filtering would require
        // ToolRegistry API changes (future work).
    } else {
        tools.register_builtin_tools();
        if let Some(ref ws) = workspace {
            tools.register_memory_tools(Arc::clone(ws));
        }
    }

    // 5. Safety, hooks, cost guard.
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

    // 8. Build and spawn agent.
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
    let agent = Agent::new(agent_config, deps, channels, None, None, None, None, None);
    let agent_handle = tokio::spawn(async move {
        if let Err(e) = agent.run().await {
            tracing::debug!("[benchmark] Agent exited: {e}");
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 9. Execute turns.
    let mut turn_metrics = Vec::new();
    let mut all_passed = true;
    let mut all_errors = Vec::new();
    let mut last_response = String::new();
    let mut total_tool_invocations = Vec::new();

    for (i, turn) in scenario.turns.iter().enumerate() {
        let turn_start = Instant::now();
        let tokens_before_input = instrumented.total_input_tokens();
        let tokens_before_output = instrumented.total_output_tokens();
        let calls_before = instrumented.call_count();

        bench_channel.clear_for_next_turn().await;

        let incoming = IncomingMessage::new("benchmark", "bench-user", &turn.user);
        if msg_tx.send(incoming).await.is_err() {
            all_passed = false;
            all_errors.push(format!("Turn {i}: failed to send message"));
            break;
        }

        let turn_timeout = turn.assertions.max_latency_secs
            .map(Duration::from_secs)
            .unwrap_or(timeout);
        let response = tokio::time::timeout(
            turn_timeout,
            bench_channel.wait_for_response(),
        ).await;

        let hit_timeout = response.is_err();
        let response_text = response.unwrap_or_default();
        last_response = response_text.clone();

        let tool_calls_completed = bench_channel.tool_calls_completed().await;
        let tool_timings_raw = bench_channel.tool_timings().await;

        let mut timing_map: std::collections::HashMap<String, Vec<u64>> =
            std::collections::HashMap::new();
        for (name, ms) in &tool_timings_raw {
            timing_map.entry(name.clone()).or_default().push(*ms);
        }
        let tool_invocations: Vec<ToolInvocation> = tool_calls_completed
            .iter()
            .map(|(name, success)| {
                let duration_ms = timing_map
                    .get_mut(name)
                    .and_then(|v| if v.is_empty() { None } else { Some(v.remove(0)) })
                    .unwrap_or(0);
                ToolInvocation { name: name.clone(), duration_ms, success: *success }
            })
            .collect();
        total_tool_invocations.extend(tool_invocations.clone());

        // Evaluate hard assertions.
        let criteria = turn.assertions.to_criteria();
        let eval_ctx = EvalContext {
            response: response_text.clone(),
            tool_calls: tool_calls_completed,
        };
        let mut turn_passed = !hit_timeout;
        let mut turn_errors = Vec::new();
        if hit_timeout {
            turn_errors.push(format!("Turn {i}: timed out"));
        }
        for criterion in &criteria {
            let result = criterion.evaluate(&eval_ctx);
            if !result.passed {
                turn_passed = false;
                turn_errors.push(format!("Turn {i}: {}", result.reason));
            }
        }

        if !turn_passed {
            all_passed = false;
            all_errors.extend(turn_errors.clone());
        }

        turn_metrics.push(TurnMetrics {
            turn_index: i,
            user_message: turn.user.clone(),
            wall_time_ms: turn_start.elapsed().as_millis() as u64,
            llm_calls: instrumented.call_count() - calls_before,
            input_tokens: instrumented.total_input_tokens() - tokens_before_input,
            output_tokens: instrumented.total_output_tokens() - tokens_before_output,
            tool_calls: tool_invocations,
            response: response_text,
            assertions_passed: turn_passed,
            judge_score: None, // Filled in Task 5
            errors: turn_errors,
        });
    }

    agent_handle.abort();

    let trace = TraceMetrics {
        wall_time_ms: scenario_start.elapsed().as_millis() as u64,
        llm_calls: instrumented.call_count(),
        input_tokens: instrumented.total_input_tokens(),
        output_tokens: instrumented.total_output_tokens(),
        estimated_cost_usd: instrumented.estimated_cost_usd(),
        tool_calls: total_tool_invocations,
        turns: scenario.turns.len() as u32,
        hit_iteration_limit: false,
        hit_timeout: false,
    };

    ScenarioResult {
        scenario_id: scenario.name.clone(),
        passed: all_passed,
        trace,
        response: last_response,
        error: if all_errors.is_empty() { None } else { Some(all_errors.join("; ")) },
        turn_metrics,
    }
}

fn error_result(name: &str, start: Instant, msg: &str) -> ScenarioResult {
    ScenarioResult {
        scenario_id: name.to_string(),
        passed: false,
        trace: empty_trace(start),
        response: String::new(),
        error: Some(msg.to_string()),
        turn_metrics: Vec::new(),
    }
}

/// Seed workspace memory with documents from the scenario setup.
async fn seed_workspace(
    workspace: &crate::workspace::Workspace,
    setup: &crate::benchmark::scenario::WorkspaceSetup,
) -> Result<(), String> {
    if let Some(ref docs) = setup.documents {
        for doc in docs {
            workspace.write(&doc.path, &doc.content).await
                .map_err(|e| format!("Failed to seed '{}': {e}", doc.path))?;
        }
    }
    if let Some(ref fixtures_dir) = setup.fixtures_dir {
        let dir = std::path::Path::new(fixtures_dir);
        if !dir.exists() {
            return Err(format!("Fixtures dir not found: {}", dir.display()));
        }
        for entry in std::fs::read_dir(dir)
            .map_err(|e| format!("Failed to read fixtures: {e}"))?
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_file() {
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read fixture {}: {e}", path.display()))?;
                let name = path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                workspace.write(&name, &content).await
                    .map_err(|e| format!("Failed to seed fixture '{}': {e}", name))?;
            }
        }
    }
    Ok(())
}
```

Also add `run_all_bench` that uses the new types:

```rust
/// Run all YAML benchmark scenarios.
pub async fn run_all_bench(
    config: &BenchmarkConfig,
    llm: Arc<dyn LlmProvider>,
) -> Result<RunResult, String> {
    let scenarios = load_bench_scenarios(config)?;
    if scenarios.is_empty() {
        return Err("No scenarios matched the given filters".to_string());
    }

    tracing::info!("Running {} benchmark scenario(s)", scenarios.len());

    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in &scenarios {
        tracing::info!("[bench] Running: {} (tags: {:?})", scenario.name, scenario.tags);
        let result = run_bench_scenario(scenario, Arc::clone(&llm), config.global_timeout_secs).await;
        let status = if result.passed { "PASS" } else { "FAIL" };
        tracing::info!(
            "[bench] {} -- {} ({}ms, {} LLM calls)",
            scenario.name, status, result.trace.wall_time_ms, result.trace.llm_calls,
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
```

**Step 4: Run tests**

Run: `cargo test --features libsql benchmark -- --nocapture`
Expected: ALL PASS

**Step 5: Run clippy**

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: 0 warnings

**Step 6: Commit**

```bash
git add src/benchmark/ benchmarks/
git commit -m "feat(benchmark): multi-turn runner with workspace seeding and per-turn metrics"
```

---

### Task 4: Add LLM-as-Judge Scoring

**Files:**
- Create: `src/benchmark/judge.rs`
- Modify: `src/benchmark/mod.rs`
- Modify: `src/benchmark/runner.rs`
- Test: inline tests in `judge.rs`

**Step 1: Write the failing tests**

Create `src/benchmark/judge.rs` with test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_judge_prompt() {
        let prompt = format_judge_prompt(
            "What time is it?",
            "It's 3:00 PM",
            &[("time".to_string(), true)],
            "Did the agent use the time tool?\nWas the response clear?",
        );
        assert!(prompt.contains("What time is it?"));
        assert!(prompt.contains("It's 3:00 PM"));
        assert!(prompt.contains("time"));
        assert!(prompt.contains("clear?"));
    }

    #[test]
    fn test_parse_judge_score_valid() {
        assert_eq!(parse_judge_score("SCORE: 8\nGood job"), Some(8));
        assert_eq!(parse_judge_score("The score is 7 out of 10"), Some(7));
        assert_eq!(parse_judge_score("SCORE: 10"), Some(10));
    }

    #[test]
    fn test_parse_judge_score_invalid() {
        assert_eq!(parse_judge_score("No score here"), None);
        assert_eq!(parse_judge_score("SCORE: 11"), None); // Out of range
        assert_eq!(parse_judge_score("SCORE: 0"), None);  // Out of range
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --features libsql benchmark::judge -- --nocapture 2>&1 | head -10`
Expected: FAIL -- module doesn't exist.

**Step 3: Implement**

```rust
//! LLM-as-judge scoring for benchmark scenarios.
//!
//! Sends trajectory summaries to a separate LLM call and parses a 1-10 score.

use std::sync::Arc;
use crate::llm::LlmProvider;

/// Format the prompt sent to the judge LLM.
pub fn format_judge_prompt(
    user_message: &str,
    agent_response: &str,
    tool_calls: &[(String, bool)],
    criteria: &str,
) -> String {
    let tools_summary = if tool_calls.is_empty() {
        "No tools were called.".to_string()
    } else {
        tool_calls
            .iter()
            .map(|(name, success)| {
                let status = if *success { "success" } else { "failed" };
                format!("  - {name} ({status})")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"You are evaluating an AI agent's performance on a task. Score from 1 (terrible) to 10 (perfect).

## User Message
{user_message}

## Agent's Tool Calls
{tools_summary}

## Agent's Response
{agent_response}

## Evaluation Criteria
{criteria}

Evaluate the agent's performance against the criteria above. Be strict but fair.

Respond with exactly this format:
SCORE: <number>
<brief justification>"#
    )
}

/// Parse a 1-10 score from the judge LLM's response.
pub fn parse_judge_score(response: &str) -> Option<u8> {
    // Look for "SCORE: N" pattern.
    let re = regex::Regex::new(r"SCORE:\s*(\d{1,2})").ok()?;
    let caps = re.captures(response)?;
    let score: u8 = caps[1].parse().ok()?;
    if (1..=10).contains(&score) {
        Some(score)
    } else {
        None
    }
}

/// Run the judge LLM on a single turn and return the score.
pub async fn judge_turn(
    llm: &Arc<dyn LlmProvider>,
    user_message: &str,
    agent_response: &str,
    tool_calls: &[(String, bool)],
    criteria: &str,
) -> Option<u8> {
    let prompt = format_judge_prompt(user_message, agent_response, tool_calls, criteria);

    let messages = vec![crate::llm::provider::Message {
        role: crate::llm::provider::Role::User,
        content: prompt,
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    match llm.complete(&messages).await {
        Ok(response) => parse_judge_score(&response.content),
        Err(e) => {
            tracing::warn!("[benchmark] Judge LLM call failed: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    // ... tests from Step 1 above
}
```

Add `pub mod judge;` to `src/benchmark/mod.rs`.

**Step 4: Wire judge into `run_bench_scenario`**

In `runner.rs`, after the hard assertion evaluation for each turn, add:

```rust
// Judge scoring (if configured).
if let Some(ref judge_config) = turn.judge {
    let score = crate::benchmark::judge::judge_turn(
        &(Arc::clone(&instrumented) as Arc<dyn LlmProvider>),
        &turn.user,
        &response_text,
        &eval_ctx.tool_calls,
        &judge_config.criteria,
    ).await;
    if let Some(s) = score {
        if s < judge_config.min_score {
            turn_passed = false;
            turn_errors.push(format!(
                "Turn {i}: judge score {s} < min {}", judge_config.min_score
            ));
        }
    }
    // Store judge score in turn_metrics (set judge_score field).
}
```

**Step 5: Run tests**

Run: `cargo test --features libsql benchmark -- --nocapture`
Expected: ALL PASS

**Step 6: Commit**

```bash
git add src/benchmark/judge.rs src/benchmark/mod.rs src/benchmark/runner.rs
git commit -m "feat(benchmark): add LLM-as-judge scoring with format/parse/evaluate"
```

---

### Task 5: Add CLI Subcommand

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/benchmark.rs`
- Modify: `src/main.rs`
- Test: compile test (no unit tests needed for CLI glue)

**Step 1: Add `Benchmark` variant to the `Command` enum**

In `src/cli/mod.rs`, add to the `Command` enum:

```rust
/// Run benchmark scenarios against a real LLM
#[cfg(feature = "benchmark")]
#[command(
    about = "Run agent benchmark scenarios",
    long_about = "Runs benchmark scenarios against a real LLM provider.\nExamples:\n  ironclaw benchmark                        # Full suite\n  ironclaw benchmark --tags basic,tools     # Tagged subset\n  ironclaw benchmark --scenario pick-time   # Single scenario\n  ironclaw benchmark --no-judge             # Skip judge scoring\n  ironclaw benchmark --parallel 4           # Parallel execution\n  ironclaw benchmark --update-baseline      # Save as baseline"
)]
Benchmark(BenchmarkCommand),
```

**Step 2: Create `src/cli/benchmark.rs`**

```rust
//! CLI for the benchmark runner.

use std::path::PathBuf;
use clap::Args;

#[derive(Args, Debug)]
pub struct BenchmarkCommand {
    /// Directory containing scenario YAML files
    #[arg(long, default_value = "benchmarks/trajectories")]
    pub scenarios_dir: PathBuf,

    /// Filter scenarios by tag (comma-separated, scenario must match at least one)
    #[arg(long, value_delimiter = ',')]
    pub tags: Option<Vec<String>>,

    /// Filter to a single scenario by name (substring match)
    #[arg(long)]
    pub scenario: Option<String>,

    /// Skip LLM-as-judge scoring (assertions only)
    #[arg(long)]
    pub no_judge: bool,

    /// Number of scenarios to run in parallel
    #[arg(long, default_value = "1")]
    pub parallel: usize,

    /// Maximum total cost in USD for the entire run
    #[arg(long)]
    pub max_total_cost: Option<f64>,

    /// Global timeout per scenario in seconds
    #[arg(long, default_value = "120")]
    pub timeout: u64,

    /// Save results as the new baseline
    #[arg(long)]
    pub update_baseline: bool,
}
```

**Step 3: Wire into `src/cli/mod.rs`**

Add:
```rust
#[cfg(feature = "benchmark")]
mod benchmark;
#[cfg(feature = "benchmark")]
pub use benchmark::BenchmarkCommand;
```

**Step 4: Add `run_benchmark_command` function to `src/cli/benchmark.rs`**

```rust
pub async fn run_benchmark_command(cmd: &BenchmarkCommand) -> anyhow::Result<()> {
    use crate::benchmark::runner::{BenchmarkConfig, run_all_bench, load_bench_scenarios};
    use crate::benchmark::baseline::{load_baseline, save_result, promote_to_baseline};
    use crate::benchmark::report::format_report;
    use crate::config::Config;
    use crate::llm::create_llm_provider;

    let config = Config::from_env()?;
    let llm = create_llm_provider(&config).await?;

    let bench_config = BenchmarkConfig {
        scenarios_dir: cmd.scenarios_dir.clone(),
        global_timeout_secs: cmd.timeout,
        filter: cmd.scenario.clone(),
        category_filter: None,
        tags_filter: cmd.tags.clone(),
    };

    let result = run_all_bench(&bench_config, llm).await
        .map_err(|e| anyhow::anyhow!(e))?;

    // Save results.
    let result_path = save_result(&result).map_err(|e| anyhow::anyhow!(e))?;
    eprintln!("Results saved to: {result_path}");

    // Load baseline for comparison.
    let baseline = load_baseline().map_err(|e| anyhow::anyhow!(e))?;

    // Print report.
    let report = format_report(&result, baseline.as_ref());
    println!("{report}");

    // Update baseline if requested.
    if cmd.update_baseline {
        promote_to_baseline(&result_path).map_err(|e| anyhow::anyhow!(e))?;
        eprintln!("Baseline updated.");
    }

    // Exit with non-zero if any scenario failed.
    if result.pass_rate < 1.0 {
        std::process::exit(1);
    }

    Ok(())
}
```

Export it: `pub use benchmark::run_benchmark_command;`

**Step 5: Add to `main.rs`**

In the match arm for `Command`, add:

```rust
#[cfg(feature = "benchmark")]
Some(Command::Benchmark(cmd)) => {
    init_cli_tracing();
    ironclaw::cli::run_benchmark_command(&cmd).await?;
}
```

**Step 6: Verify compilation**

Run: `cargo check --features "libsql,benchmark"`
Expected: Compiles with 0 errors

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: 0 warnings

**Step 7: Commit**

```bash
git add src/cli/benchmark.rs src/cli/mod.rs src/main.rs
git commit -m "feat(benchmark): add CLI subcommand (ironclaw benchmark)"
```

---

### Task 6: Add Per-Scenario JSON Output + Trajectory Recording

**Files:**
- Modify: `src/benchmark/metrics.rs`
- Modify: `src/benchmark/runner.rs`
- Modify: `src/benchmark/baseline.rs`
- Test: inline tests

This task adds full trajectory output to per-scenario JSON files matching the spec format.

**Step 1: Add trajectory output to `save_result`**

Modify `baseline.rs` to also save individual scenario results:

```rust
/// Save per-scenario JSON results alongside the run summary.
pub fn save_scenario_results(result: &RunResult) -> Result<String, String> {
    let dir = format!("benchmarks/results/{}", result.run_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {e}"))?;

    // Save summary.
    let summary_path = format!("{dir}/summary.json");
    let summary = serde_json::to_string_pretty(result)
        .map_err(|e| format!("Failed to serialize summary: {e}"))?;
    std::fs::write(&summary_path, summary)
        .map_err(|e| format!("Failed to write summary: {e}"))?;

    // Save per-scenario.
    for scenario in &result.scenarios {
        let scenario_path = format!("{dir}/{}.json", scenario.scenario_id);
        let content = serde_json::to_string_pretty(scenario)
            .map_err(|e| format!("Failed to serialize {}: {e}", scenario.scenario_id))?;
        std::fs::write(&scenario_path, content)
            .map_err(|e| format!("Failed to write {}: {e}", scenario.scenario_id))?;
    }

    Ok(dir)
}
```

**Step 2: Wire into CLI**

Update `run_benchmark_command` to use `save_scenario_results` instead of `save_result`:

```rust
let result_dir = save_scenario_results(&result).map_err(|e| anyhow::anyhow!(e))?;
eprintln!("Results saved to: {result_dir}/");
```

**Step 3: Test**

Run: `cargo test --features libsql benchmark -- --nocapture`
Expected: ALL PASS

**Step 4: Commit**

```bash
git add src/benchmark/baseline.rs src/cli/benchmark.rs
git commit -m "feat(benchmark): per-scenario JSON output with full trajectory"
```

---

## Summary

| Task | What It Does | Key Files |
|------|-------------|-----------|
| 1 | YAML types: BenchScenario, Turn, TurnAssertions, JudgeConfig, ScenarioSetup, ResponseNotContains | `scenario.rs` |
| 2 | YAML loader with recursive discovery + tag filter, initial YAML scenarios | `runner.rs`, `benchmarks/trajectories/` |
| 3 | Multi-turn runner with workspace seeding, per-turn metrics, clear_for_next_turn | `runner.rs`, `bench_channel.rs`, `metrics.rs` |
| 4 | LLM-as-judge: prompt formatting, score parsing, judge_turn() | `judge.rs`, `runner.rs` |
| 5 | CLI subcommand: `ironclaw benchmark --tags --scenario --no-judge --parallel --update-baseline` | `cli/benchmark.rs`, `cli/mod.rs`, `main.rs` |
| 6 | Per-scenario JSON output with full trajectory | `baseline.rs`, `cli/benchmark.rs` |

**Not included (intentionally deferred):**
- **Parallel execution** (`--parallel N`): Requires careful tokio::JoinSet orchestration + rate limiting. Better as a separate PR.
- **Budget caps** (`--max-total-cost`): Needs mid-run cost tracking in the runner loop. Straightforward follow-up.
- **Identity overrides**: Requires changes to how Agent reads identity files from workspace. Separate concern.
- **Skill activation**: Requires SkillRegistry wiring in the runner. Lower priority.
- **nearai/benchmarks integration**: Research task, not implementation.

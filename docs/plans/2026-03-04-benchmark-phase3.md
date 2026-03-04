# Benchmark Phase 3: Parallel Execution, Budget Caps, and Setup Wiring

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Wire the stubbed `setup.tools`, `setup.identity` fields into the runner, add parallel scenario execution, and add run-level budget caps.

**Architecture:** Phase 2 defined the types (`ScenarioSetup.tools`, `.identity`) but never wired them into `run_bench_scenario()`. Phase 3 adds a `retain_only()` method to `ToolRegistry`, writes identity files into workspace before the agent starts, replaces the sequential loop in `run_all_bench()` with `tokio::JoinSet` for parallel execution, and adds a `--max-cost` budget cap that aborts remaining scenarios when exceeded.

**Tech Stack:** Rust, tokio (JoinSet), serde, clap

---

### Task 1: Add `retain_only()` to ToolRegistry + Wire Tool Filtering

**Files:**
- Modify: `src/tools/registry.rs` (add `retain_only` method)
- Modify: `src/benchmark/runner.rs:558-562` (wire tool filtering after `register_builtin_tools()`)

**Step 1: Write the failing test for `retain_only`**

Add to the existing `mod tests` in `src/tools/registry.rs`:

```rust
#[tokio::test]
async fn test_retain_only_filters_tools() {
    let registry = ToolRegistry::new();
    registry.register_builtin_tools();

    // Should have multiple tools.
    let all = registry.list().await;
    assert!(all.len() > 2, "expected multiple built-in tools");

    // Retain only "echo" and "time".
    registry.retain_only(&["echo", "time"]).await;

    let remaining = registry.list().await;
    assert_eq!(remaining.len(), 2);
    assert!(remaining.contains(&"echo".to_string()));
    assert!(remaining.contains(&"time".to_string()));
}

#[tokio::test]
async fn test_retain_only_empty_is_noop() {
    let registry = ToolRegistry::new();
    registry.register_builtin_tools();
    let before = registry.list().await.len();

    // Empty allowlist = no filtering (keep all).
    registry.retain_only(&[]).await;

    let after = registry.list().await.len();
    assert_eq!(before, after);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --all-features tools::registry::tests::test_retain_only -v`
Expected: FAIL with "no method named `retain_only`"

**Step 3: Implement `retain_only` on ToolRegistry**

Add this method to `ToolRegistry` in `src/tools/registry.rs` (after the `list()` method, around line 170):

```rust
/// Retain only tools whose names are in the given allowlist.
///
/// If `names` is empty, this is a no-op (all tools are kept).
/// This is used by the benchmark runner to restrict the tool set
/// per scenario based on `setup.tools`.
pub async fn retain_only(&self, names: &[&str]) {
    if names.is_empty() {
        return;
    }
    let mut tools = self.tools.write().await;
    tools.retain(|k, _| names.contains(&k.as_str()));
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --all-features tools::registry::tests::test_retain_only -v`
Expected: PASS (both tests)

**Step 5: Wire tool filtering into `run_bench_scenario()`**

In `src/benchmark/runner.rs`, after line 562 (`tools.register_memory_tools(...)` block), add:

```rust
// Filter tools to scenario allowlist (if specified).
if !scenario.setup.tools.is_empty() {
    let names: Vec<&str> = scenario.setup.tools.iter().map(|s| s.as_str()).collect();
    tools.retain_only(&names).await;
}
```

**Step 6: Run all benchmark tests**

Run: `cargo test --all-features benchmark -- -v`
Expected: All existing tests still pass

**Step 7: Commit**

```bash
git add src/tools/registry.rs src/benchmark/runner.rs
git commit -m "feat(benchmark): add ToolRegistry::retain_only and wire tool filtering in scenarios"
```

---

### Task 2: Wire Identity Overrides into Workspace

**Files:**
- Modify: `src/benchmark/runner.rs:564-574` (add identity seeding after workspace seeding)

**Step 1: Write the failing test**

Add to `mod tests` in `src/benchmark/runner.rs`:

```rust
#[tokio::test]
async fn test_seed_identity_files() {
    use crate::db::libsql::LibSqlBackend;
    use crate::workspace::Workspace;

    let backend = LibSqlBackend::new_memory().await.unwrap();
    backend.run_migrations().await.unwrap();
    let db: Arc<dyn crate::db::Database> = Arc::new(backend);
    let ws = Workspace::new_with_db("bench-user", db);

    let mut identity = std::collections::HashMap::new();
    identity.insert("IDENTITY.md".to_string(), "You are TestBot.".to_string());
    identity.insert("USER.md".to_string(), "The user is a tester.".to_string());

    seed_identity(&ws, &identity).await.unwrap();

    let id_doc = ws.read("IDENTITY.md").await.unwrap();
    assert_eq!(id_doc.content, "You are TestBot.");
    let user_doc = ws.read("USER.md").await.unwrap();
    assert_eq!(user_doc.content, "The user is a tester.");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --all-features benchmark::runner::tests::test_seed_identity -v`
Expected: FAIL with "cannot find function `seed_identity`"

**Step 3: Implement `seed_identity` helper**

Add this function in `src/benchmark/runner.rs` (after `seed_workspace`):

```rust
/// Seed identity override files into the workspace.
///
/// Each entry in `identity` maps a workspace path (e.g., "IDENTITY.md") to
/// its content. These are written before the agent starts so that
/// `workspace.system_prompt()` picks them up.
async fn seed_identity(
    workspace: &crate::workspace::Workspace,
    identity: &std::collections::HashMap<String, String>,
) -> Result<(), String> {
    for (path, content) in identity {
        workspace
            .write(path, content)
            .await
            .map_err(|e| format!("Failed to seed identity file '{}': {e}", path))?;
    }
    Ok(())
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --all-features benchmark::runner::tests::test_seed_identity -v`
Expected: PASS

**Step 5: Wire identity seeding into `run_bench_scenario()`**

In `src/benchmark/runner.rs`, after the workspace seeding block (around line 574), add:

```rust
// 4b. Seed identity override files from setup.
if !scenario.setup.identity.is_empty() {
    if let Some(ref ws) = workspace {
        if let Err(e) = seed_identity(ws, &scenario.setup.identity).await {
            return error_result(
                &scenario.name,
                scenario_start,
                format!("Failed to seed identity: {e}"),
            );
        }
    }
}
```

**Step 6: Run all benchmark tests**

Run: `cargo test --all-features benchmark -- -v`
Expected: All tests pass

**Step 7: Commit**

```bash
git add src/benchmark/runner.rs
git commit -m "feat(benchmark): wire identity overrides into workspace before agent start"
```

---

### Task 3: Add `--parallel` and `--max-cost` CLI Flags

**Files:**
- Modify: `src/cli/benchmark.rs:13-38` (add new fields to BenchmarkCommand)
- Modify: `src/benchmark/runner.rs:32-45` (add new fields to BenchmarkConfig)

**Step 1: Add CLI fields to BenchmarkCommand**

In `src/cli/benchmark.rs`, add these fields to the `BenchmarkCommand` struct (after `update_baseline`):

```rust
/// Number of scenarios to run in parallel (default: 1 = sequential)
#[arg(long, default_value = "1")]
pub parallel: usize,

/// Maximum total cost in USD across all scenarios; abort remaining if exceeded
#[arg(long)]
pub max_cost: Option<f64>,
```

**Step 2: Add config fields to BenchmarkConfig**

In `src/benchmark/runner.rs`, add these fields to `BenchmarkConfig` (after `tags_filter`):

```rust
/// Number of scenarios to run in parallel (1 = sequential).
pub parallel: usize,
/// Maximum total cost in USD. If exceeded, remaining scenarios are skipped.
pub max_total_cost_usd: Option<f64>,
```

Update the `Default` impl to include:

```rust
parallel: 1,
max_total_cost_usd: None,
```

**Step 3: Wire CLI to config**

In `src/cli/benchmark.rs`, update the `BenchmarkConfig` construction in `run_benchmark_command()` to include:

```rust
parallel: cmd.parallel,
max_total_cost_usd: cmd.max_cost,
```

**Step 4: Run compilation check**

Run: `cargo check --all-features`
Expected: Compiles cleanly (no test changes needed yet since `run_all_bench` doesn't use the new fields)

**Step 5: Update snapshot tests if needed**

Run: `cargo test --all-features cli::tests -v`
If snapshot tests fail due to updated help text, run `cargo insta accept`.

**Step 6: Commit**

```bash
git add src/cli/benchmark.rs src/benchmark/runner.rs
git commit -m "feat(benchmark): add --parallel and --max-cost CLI flags"
```

---

### Task 4: Parallel Execution in `run_all_bench()`

**Files:**
- Modify: `src/benchmark/runner.rs:804-846` (replace sequential loop with JoinSet)
- Modify: `src/benchmark/metrics.rs` (add `skipped_scenarios` field to RunResult)
- Modify: `src/benchmark/report.rs` (display skipped count)

**Step 1: Write the failing test for parallel execution**

Add to `mod tests` in `src/benchmark/runner.rs`:

```rust
#[test]
fn test_parallel_config_defaults() {
    let config = BenchmarkConfig::default();
    assert_eq!(config.parallel, 1);
    assert!(config.max_total_cost_usd.is_none());
}
```

**Step 2: Run test to verify it passes (sanity check)**

Run: `cargo test --all-features benchmark::runner::tests::test_parallel_config -v`
Expected: PASS

**Step 3: Add `skipped_scenarios` to RunResult**

In `src/benchmark/metrics.rs`, add to `RunResult`:

```rust
/// Number of scenarios skipped (e.g., due to budget cap).
#[serde(default)]
pub skipped_scenarios: usize,
```

Update `RunResult::from_scenarios()` to include `skipped_scenarios: 0`.

**Step 4: Implement parallel `run_all_bench()`**

Replace the body of `run_all_bench()` in `src/benchmark/runner.rs` with:

```rust
pub async fn run_all_bench(
    config: &BenchmarkConfig,
    llm: Arc<dyn LlmProvider>,
) -> Result<RunResult, String> {
    let scenarios = load_bench_scenarios(config)?;
    if scenarios.is_empty() {
        return Err("No bench scenarios matched the given filters".to_string());
    }

    tracing::info!(
        "Running {} bench scenario(s) (parallel: {})",
        scenarios.len(),
        config.parallel
    );

    let mut results = Vec::with_capacity(scenarios.len());
    let mut skipped = 0usize;

    if config.parallel <= 1 {
        // Sequential execution (original behavior).
        for scenario in &scenarios {
            // Check budget cap before starting next scenario.
            if let Some(max_cost) = config.max_total_cost_usd {
                let running_cost: f64 = results.iter().map(|r: &ScenarioResult| r.trace.estimated_cost_usd).sum();
                if running_cost >= max_cost {
                    tracing::warn!(
                        "[bench] Budget cap ${:.4} reached (spent ${:.4}), skipping remaining {} scenarios",
                        max_cost, running_cost, scenarios.len() - results.len()
                    );
                    skipped = scenarios.len() - results.len();
                    break;
                }
            }

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
    } else {
        // Parallel execution with bounded concurrency.
        use tokio::task::JoinSet;

        let semaphore = Arc::new(tokio::sync::Semaphore::new(config.parallel));
        let mut join_set = JoinSet::new();

        for scenario in scenarios {
            let llm = Arc::clone(&llm);
            let timeout = config.global_timeout_secs;
            let sem = Arc::clone(&semaphore);

            join_set.spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore closed");
                tracing::info!(
                    "[bench] Running scenario: {} (tags: {:?})",
                    scenario.name,
                    scenario.tags
                );
                let result = run_bench_scenario(&scenario, llm, timeout).await;
                let status = if result.passed { "PASS" } else { "FAIL" };
                tracing::info!(
                    "[bench] {} -- {} ({}ms, {} LLM calls, {} turns)",
                    scenario.name,
                    status,
                    result.trace.wall_time_ms,
                    result.trace.llm_calls,
                    result.trace.turns,
                );
                result
            });
        }

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok(result) => results.push(result),
                Err(e) => {
                    tracing::error!("[bench] Scenario task panicked: {e}");
                }
            }
        }

        // Sort results by scenario_id for deterministic ordering.
        results.sort_by(|a, b| a.scenario_id.cmp(&b.scenario_id));
    }

    let run_id = format!("bench-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let mut run_result = RunResult::from_scenarios(run_id, results);
    run_result.skipped_scenarios = skipped;
    if let Some(hash) = git_commit_hash() {
        run_result = run_result.with_commit_hash(hash);
    }

    Ok(run_result)
}
```

**Step 5: Update report to show skipped scenarios**

In `src/benchmark/report.rs`, after the "Scenarios: N/M passed" line (around line 22), add:

```rust
if current.skipped_scenarios > 0 {
    out.push_str(&format!(
        "Skipped: {} (budget cap reached)\n",
        current.skipped_scenarios
    ));
}
```

**Step 6: Run all tests**

Run: `cargo test --all-features -- -v`
Expected: All tests pass. Some tests may need `skipped_scenarios: 0` added to RunResult constructions.

**Step 7: Commit**

```bash
git add src/benchmark/runner.rs src/benchmark/metrics.rs src/benchmark/report.rs
git commit -m "feat(benchmark): parallel execution with JoinSet and budget cap enforcement"
```

---

### Task 5: Add Test Scenario Exercising Tool Restriction and Identity

**Files:**
- Create: `benchmarks/trajectories/setup/tool-restriction.json`
- Create: `benchmarks/trajectories/setup/identity-override.json`

**Step 1: Create tool restriction scenario**

Create `benchmarks/trajectories/setup/tool-restriction.json`:

```json
{
    "name": "tool-restriction-echo-only",
    "description": "Verify that tool restriction limits available tools to only echo",
    "tags": ["setup", "tools"],
    "setup": {
        "tools": ["echo"]
    },
    "turns": [
        {
            "user": "Use the echo tool to say 'restricted test'",
            "assertions": {
                "tools_called": ["echo"],
                "tools_not_called": ["time", "json", "http"],
                "response_contains": ["restricted test"]
            }
        }
    ],
    "timeout_secs": 60
}
```

**Step 2: Create identity override scenario**

Create `benchmarks/trajectories/setup/identity-override.json`:

```json
{
    "name": "identity-override-custom-name",
    "description": "Verify that identity overrides are injected into agent system prompt",
    "tags": ["setup", "identity"],
    "setup": {
        "identity": {
            "IDENTITY.md": "Your name is BenchBot. You always introduce yourself as BenchBot when asked your name."
        }
    },
    "turns": [
        {
            "user": "What is your name?",
            "assertions": {
                "response_contains": ["BenchBot"]
            }
        }
    ],
    "timeout_secs": 60
}
```

**Step 3: Verify scenarios load correctly**

Add a test in `src/benchmark/runner.rs` tests:

```rust
#[test]
fn test_load_bench_scenarios_setup_directory() {
    let config = BenchmarkConfig {
        scenarios_dir: PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/benchmarks/trajectories"
        )),
        tags_filter: Some(vec!["setup".to_string()]),
        ..BenchmarkConfig::default()
    };
    if !config.scenarios_dir.exists() {
        return;
    }
    let scenarios =
        load_bench_scenarios(&config).expect("should load setup scenarios");
    assert!(
        scenarios.len() >= 2,
        "expected at least 2 setup scenarios, got {}",
        scenarios.len()
    );
    // Verify tool restriction scenario has tools in setup.
    let tool_restricted = scenarios.iter().find(|s| s.name.contains("tool-restriction"));
    assert!(tool_restricted.is_some(), "expected tool-restriction scenario");
    assert!(!tool_restricted.unwrap().setup.tools.is_empty());
}
```

**Step 4: Run test**

Run: `cargo test --all-features benchmark::runner::tests::test_load_bench_scenarios_setup -v`
Expected: PASS

**Step 5: Commit**

```bash
git add benchmarks/trajectories/setup/ src/benchmark/runner.rs
git commit -m "feat(benchmark): add tool restriction and identity override test scenarios"
```

---

### Task 6: Final Quality Gate

**Step 1: Run full formatting check**

Run: `cargo fmt --check`
If not clean: `cargo fmt`

**Step 2: Run full clippy**

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: 0 warnings. Fix any that appear.

**Step 3: Run full test suite**

Run: `cargo test --all-features`
Expected: All tests pass.

**Step 4: Run default-features compilation check**

Run: `cargo check`
Expected: Clean compilation.

**Step 5: Update snapshot tests if needed**

Run: `cargo insta accept` if any snapshots are pending.

**Step 6: Commit any fixes**

```bash
git add -A
git commit -m "chore: fix formatting and clippy warnings for Phase 3"
```

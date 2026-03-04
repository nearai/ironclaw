# Agent Benchmark System Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a task-based benchmark system that measures agent effectiveness with real LLM calls, tracks improvement over time via baseline comparison, and supports cross-model evaluation.

**Architecture:** Promote existing test metrics types (`TraceMetrics`, `ScenarioResult`, `RunResult`, `compare_runs`) to library code. Add a `Scenario` definition format with programmatic success criteria. Build a benchmark runner that spins up a real agent per scenario (reusing `TestRigBuilder` patterns), evaluates criteria, collects metrics, and compares against stored baselines.

**Tech Stack:** Rust, tokio, serde_json, libSQL (embedded test DB), existing IronClaw agent/tools/LLM infrastructure.

---

### Task 1: Promote metrics types from test support to library code

**Files:**
- Create: `src/benchmark/mod.rs`
- Create: `src/benchmark/metrics.rs`
- Modify: `src/lib.rs` — add `pub mod benchmark;`
- Modify: `tests/support/metrics.rs` — replace with re-export from `src/benchmark/metrics.rs`

**Context:** `TraceMetrics`, `ScenarioResult`, `RunResult`, `MetricDelta`, and `compare_runs()` currently live in `tests/support/metrics.rs`. The benchmark runner needs them in library code so it can be used from both tests and a standalone binary.

**Step 1: Create `src/benchmark/mod.rs`**

```rust
//! Agent benchmark system.
//!
//! Measures agent effectiveness with real LLM calls, tracks improvement
//! via baseline comparison, and supports cross-model evaluation.

pub mod metrics;
```

**Step 2: Create `src/benchmark/metrics.rs`**

Copy the full contents of `tests/support/metrics.rs` into `src/benchmark/metrics.rs`. Remove the `#![allow(dead_code)]` attribute. Change the module doc comment to:

```rust
//! Metrics types for agent benchmarking.
//!
//! Matches the metric model from `nearai/benchmarks` (Trace, TaskResult, RunResult)
//! so that results are comparable across the two harnesses.
```

Keep all structs, impls, and the `compare_runs()` function. Keep the `#[cfg(test)] mod tests` block.

**Step 3: Add `pub mod benchmark;` to `src/lib.rs`**

Find the module declarations section and add:
```rust
pub mod benchmark;
```

**Step 4: Update `tests/support/metrics.rs` to re-export**

Replace the entire contents of `tests/support/metrics.rs` with:

```rust
//! Re-export benchmark metrics for backward compatibility with existing tests.

pub use ironclaw::benchmark::metrics::*;
```

**Step 5: Run tests to verify nothing broke**

Run: `cargo test --features libsql`
Expected: all tests pass (existing tests that `use crate::support::metrics::*` still work).

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: 0 warnings.

**Step 6: Commit**

```bash
git add src/benchmark/ src/lib.rs tests/support/metrics.rs
git commit -m "refactor: promote benchmark metrics types to library code"
```

---

### Task 2: Define the Scenario and Criterion types

**Files:**
- Create: `src/benchmark/scenario.rs`
- Modify: `src/benchmark/mod.rs` — add `pub mod scenario;`

**Context:** A `Scenario` is a task definition with an input message, success criteria, and resource limits. Scenarios are loaded from JSON files. `Criterion` is an enum of programmatic checks (tool_used, response_contains, etc.).

**Step 1: Write failing tests in `src/benchmark/scenario.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scenario_deserialize() {
        let json = r#"{
            "id": "test-echo",
            "category": "tool_selection",
            "input": "Say hello",
            "success_criteria": [
                {"type": "tool_used", "tool": "echo"},
                {"type": "response_contains", "text": "hello"}
            ],
            "timeout_secs": 30,
            "max_tool_iterations": 10
        }"#;
        let scenario: Scenario = serde_json::from_str(json).unwrap();
        assert_eq!(scenario.id, "test-echo");
        assert_eq!(scenario.success_criteria.len(), 2);
    }

    #[test]
    fn test_criterion_tool_used_pass() {
        let criterion = Criterion::ToolUsed { tool: "echo".to_string() };
        let ctx = EvalContext {
            response: "hello".to_string(),
            tool_calls: vec![("echo".to_string(), true)],
        };
        let result = criterion.evaluate(&ctx);
        assert!(result.passed);
    }

    #[test]
    fn test_criterion_tool_used_fail() {
        let criterion = Criterion::ToolUsed { tool: "echo".to_string() };
        let ctx = EvalContext {
            response: "hello".to_string(),
            tool_calls: vec![("time".to_string(), true)],
        };
        let result = criterion.evaluate(&ctx);
        assert!(!result.passed);
    }

    #[test]
    fn test_criterion_tool_not_used_pass() {
        let criterion = Criterion::ToolNotUsed { tool: "shell".to_string() };
        let ctx = EvalContext {
            response: "hello".to_string(),
            tool_calls: vec![("echo".to_string(), true)],
        };
        let result = criterion.evaluate(&ctx);
        assert!(result.passed);
    }

    #[test]
    fn test_criterion_response_contains_pass() {
        let criterion = Criterion::ResponseContains { text: "hello".to_string() };
        let ctx = EvalContext {
            response: "I said hello to you".to_string(),
            tool_calls: vec![],
        };
        let result = criterion.evaluate(&ctx);
        assert!(result.passed);
    }

    #[test]
    fn test_criterion_response_contains_case_insensitive() {
        let criterion = Criterion::ResponseContains { text: "hello".to_string() };
        let ctx = EvalContext {
            response: "I said HELLO to you".to_string(),
            tool_calls: vec![],
        };
        let result = criterion.evaluate(&ctx);
        assert!(result.passed);
    }

    #[test]
    fn test_criterion_response_matches_regex() {
        let criterion = Criterion::ResponseMatches { pattern: r"20\d{2}".to_string() };
        let ctx = EvalContext {
            response: "The year is 2026".to_string(),
            tool_calls: vec![],
        };
        let result = criterion.evaluate(&ctx);
        assert!(result.passed);
    }

    #[test]
    fn test_criterion_tool_call_count_max() {
        let criterion = Criterion::ToolCallCountMax { max: 3 };
        let ctx = EvalContext {
            response: "done".to_string(),
            tool_calls: vec![
                ("a".to_string(), true),
                ("b".to_string(), true),
                ("c".to_string(), true),
                ("d".to_string(), true),
            ],
        };
        let result = criterion.evaluate(&ctx);
        assert!(!result.passed);
        assert!(result.reason.contains("4"));
    }

    #[test]
    fn test_evaluate_all_criteria() {
        let criteria = vec![
            Criterion::ToolUsed { tool: "echo".to_string() },
            Criterion::ResponseContains { text: "hello".to_string() },
        ];
        let ctx = EvalContext {
            response: "hello world".to_string(),
            tool_calls: vec![("echo".to_string(), true)],
        };
        let (passed, results) = evaluate_criteria(&criteria, &ctx);
        assert!(passed);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.passed));
    }

    #[test]
    fn test_evaluate_criteria_one_fails() {
        let criteria = vec![
            Criterion::ToolUsed { tool: "echo".to_string() },
            Criterion::ToolUsed { tool: "time".to_string() },
        ];
        let ctx = EvalContext {
            response: "hello".to_string(),
            tool_calls: vec![("echo".to_string(), true)],
        };
        let (passed, results) = evaluate_criteria(&criteria, &ctx);
        assert!(!passed);
        assert_eq!(results.iter().filter(|r| !r.passed).count(), 1);
    }

    #[test]
    fn test_load_scenarios_from_json_array() {
        let json = r#"[
            {
                "id": "s1",
                "category": "test",
                "input": "hello",
                "success_criteria": [],
                "timeout_secs": 10,
                "max_tool_iterations": 5
            },
            {
                "id": "s2",
                "category": "test",
                "input": "world",
                "success_criteria": [],
                "timeout_secs": 10,
                "max_tool_iterations": 5
            }
        ]"#;
        let scenarios: Vec<Scenario> = serde_json::from_str(json).unwrap();
        assert_eq!(scenarios.len(), 2);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --features libsql benchmark::scenario`
Expected: compilation error (types don't exist yet).

**Step 3: Implement the types**

```rust
//! Benchmark scenario definitions and success criteria evaluation.

use serde::{Deserialize, Serialize};

/// A benchmark scenario: a task with input, success criteria, and resource limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    /// Unique identifier (e.g., "file-write-read-roundtrip").
    pub id: String,
    /// Category for grouping (e.g., "tool_selection", "error_recovery").
    pub category: String,
    /// The user message to send to the agent.
    pub input: String,
    /// All criteria must pass for the scenario to pass.
    pub success_criteria: Vec<Criterion>,
    /// Maximum seconds before the scenario times out.
    pub timeout_secs: u64,
    /// Maximum tool iterations the agent is allowed.
    pub max_tool_iterations: usize,
}

/// A single success criterion. Evaluated programmatically — no LLM judgment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Criterion {
    /// The agent must have called this tool at least once.
    ToolUsed { tool: String },
    /// The agent must NOT have called this tool.
    ToolNotUsed { tool: String },
    /// Total tool calls must not exceed this count.
    ToolCallCountMax { max: usize },
    /// The agent's final response must contain this text (case-insensitive).
    ResponseContains { text: String },
    /// The agent's final response must match this regex pattern.
    ResponseMatches { pattern: String },
}

/// Context provided to criteria for evaluation.
pub struct EvalContext {
    /// The agent's final text response.
    pub response: String,
    /// Tool calls made: (name, success).
    pub tool_calls: Vec<(String, bool)>,
}

/// Result of evaluating a single criterion.
#[derive(Debug, Clone)]
pub struct CriterionResult {
    /// Which criterion was evaluated.
    pub criterion: String,
    /// Whether it passed.
    pub passed: bool,
    /// Human-readable explanation.
    pub reason: String,
}

impl Criterion {
    /// Evaluate this criterion against the given context.
    pub fn evaluate(&self, ctx: &EvalContext) -> CriterionResult {
        match self {
            Criterion::ToolUsed { tool } => {
                let used = ctx.tool_calls.iter().any(|(name, _)| name == tool);
                CriterionResult {
                    criterion: format!("tool_used:{tool}"),
                    passed: used,
                    reason: if used {
                        format!("Tool '{tool}' was called")
                    } else {
                        format!("Tool '{tool}' was NOT called. Tools used: {:?}",
                            ctx.tool_calls.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>())
                    },
                }
            }
            Criterion::ToolNotUsed { tool } => {
                let used = ctx.tool_calls.iter().any(|(name, _)| name == tool);
                CriterionResult {
                    criterion: format!("tool_not_used:{tool}"),
                    passed: !used,
                    reason: if !used {
                        format!("Tool '{tool}' was correctly not called")
                    } else {
                        format!("Tool '{tool}' was called but should not have been")
                    },
                }
            }
            Criterion::ToolCallCountMax { max } => {
                let count = ctx.tool_calls.len();
                CriterionResult {
                    criterion: format!("tool_call_count_max:{max}"),
                    passed: count <= *max,
                    reason: format!("{count} tool calls (max {max})"),
                }
            }
            Criterion::ResponseContains { text } => {
                let contains = ctx.response.to_lowercase().contains(&text.to_lowercase());
                CriterionResult {
                    criterion: format!("response_contains:{text}"),
                    passed: contains,
                    reason: if contains {
                        format!("Response contains '{text}'")
                    } else {
                        format!("Response does NOT contain '{text}'")
                    },
                }
            }
            Criterion::ResponseMatches { pattern } => {
                match regex::Regex::new(pattern) {
                    Ok(re) => {
                        let matches = re.is_match(&ctx.response);
                        CriterionResult {
                            criterion: format!("response_matches:{pattern}"),
                            passed: matches,
                            reason: if matches {
                                format!("Response matches pattern '{pattern}'")
                            } else {
                                format!("Response does NOT match pattern '{pattern}'")
                            },
                        }
                    }
                    Err(e) => CriterionResult {
                        criterion: format!("response_matches:{pattern}"),
                        passed: false,
                        reason: format!("Invalid regex pattern: {e}"),
                    },
                }
            }
        }
    }
}

/// Evaluate all criteria against the context. Returns (all_passed, individual_results).
pub fn evaluate_criteria(criteria: &[Criterion], ctx: &EvalContext) -> (bool, Vec<CriterionResult>) {
    let results: Vec<CriterionResult> = criteria.iter().map(|c| c.evaluate(ctx)).collect();
    let all_passed = results.iter().all(|r| r.passed);
    (all_passed, results)
}
```

**Step 4: Add `regex` dependency check**

The `regex` crate is likely already a transitive dependency. Check with:
```bash
cargo tree -p regex 2>/dev/null | head -3
```
If not present, it needs to be added to `Cargo.toml` (ask user before adding).

**Step 5: Run tests**

Run: `cargo test --features libsql benchmark::scenario`
Expected: all 10 tests pass.

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: 0 warnings.

**Step 6: Commit**

```bash
git add src/benchmark/scenario.rs src/benchmark/mod.rs
git commit -m "feat: add Scenario and Criterion types for agent benchmarking"
```

---

### Task 3: Create the initial scenario suite

**Files:**
- Create: `benchmarks/scenarios/tool_selection.json`
- Create: `benchmarks/scenarios/tool_chaining.json`
- Create: `benchmarks/scenarios/error_recovery.json`
- Create: `benchmarks/scenarios/efficiency.json`
- Create: `benchmarks/scenarios/memory_operations.json`

**Context:** These are the task definitions the benchmark runner will execute. Each file is a JSON array of `Scenario` objects. Start with 3-4 scenarios per category.

**Step 1: Create `benchmarks/scenarios/tool_selection.json`**

```json
[
  {
    "id": "ts-time-query",
    "category": "tool_selection",
    "input": "What time is it right now?",
    "success_criteria": [
      {"type": "tool_used", "tool": "time"},
      {"type": "tool_not_used", "tool": "shell"},
      {"type": "response_matches", "pattern": "\\d{1,2}:\\d{2}"}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 5
  },
  {
    "id": "ts-direct-answer",
    "category": "tool_selection",
    "input": "What is 2 + 2?",
    "success_criteria": [
      {"type": "response_contains", "text": "4"},
      {"type": "tool_call_count_max", "max": 0}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 5
  },
  {
    "id": "ts-echo-not-shell",
    "category": "tool_selection",
    "input": "Use the echo tool to say 'benchmark test'.",
    "success_criteria": [
      {"type": "tool_used", "tool": "echo"},
      {"type": "tool_not_used", "tool": "shell"},
      {"type": "response_contains", "text": "benchmark test"}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 5
  }
]
```

**Step 2: Create `benchmarks/scenarios/tool_chaining.json`**

```json
[
  {
    "id": "tc-write-read-file",
    "category": "tool_chaining",
    "input": "Write the text 'IronClaw benchmark test' to /tmp/ironclaw_bench_write_read.txt, then read it back and tell me the contents.",
    "success_criteria": [
      {"type": "tool_used", "tool": "write_file"},
      {"type": "tool_used", "tool": "read_file"},
      {"type": "response_contains", "text": "IronClaw benchmark test"}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 10
  },
  {
    "id": "tc-json-parse-query",
    "category": "tool_chaining",
    "input": "Parse this JSON: {\"name\": \"IronClaw\", \"version\": \"0.13.0\"} and tell me the version field.",
    "success_criteria": [
      {"type": "tool_used", "tool": "json"},
      {"type": "response_contains", "text": "0.13.0"}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 10
  },
  {
    "id": "tc-memory-write-search",
    "category": "tool_chaining",
    "input": "Save a note to memory at path 'bench/test-note.md' with content 'The secret code is ALPHA-7'. Then search your memory for 'secret code' and tell me what you find.",
    "success_criteria": [
      {"type": "tool_used", "tool": "memory_write"},
      {"type": "tool_used", "tool": "memory_search"},
      {"type": "response_contains", "text": "ALPHA-7"}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 10
  }
]
```

**Step 3: Create `benchmarks/scenarios/error_recovery.json`**

```json
[
  {
    "id": "er-read-missing-file",
    "category": "error_recovery",
    "input": "Read the file /tmp/ironclaw_bench_nonexistent_12345.txt and tell me what happened.",
    "success_criteria": [
      {"type": "tool_used", "tool": "read_file"},
      {"type": "tool_call_count_max", "max": 3},
      {"type": "response_matches", "pattern": "(?i)(not found|does not exist|error|no such file|couldn't|unable)"}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 5
  },
  {
    "id": "er-invalid-json",
    "category": "error_recovery",
    "input": "Parse this invalid JSON: {broken and tell me what's wrong with it.",
    "success_criteria": [
      {"type": "tool_used", "tool": "json"},
      {"type": "tool_call_count_max", "max": 3},
      {"type": "response_matches", "pattern": "(?i)(invalid|error|parse|syntax|malformed)"}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 5
  }
]
```

**Step 4: Create `benchmarks/scenarios/efficiency.json`**

```json
[
  {
    "id": "ef-simple-echo",
    "category": "efficiency",
    "input": "Echo the message 'hello world' using the echo tool.",
    "success_criteria": [
      {"type": "tool_used", "tool": "echo"},
      {"type": "tool_call_count_max", "max": 2},
      {"type": "response_contains", "text": "hello world"}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 5
  },
  {
    "id": "ef-single-file-read",
    "category": "efficiency",
    "input": "Read /tmp/ironclaw_bench_efficiency.txt and tell me its contents.",
    "success_criteria": [
      {"type": "tool_used", "tool": "read_file"},
      {"type": "tool_call_count_max", "max": 2},
      {"type": "response_contains", "text": "efficiency test content"}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 5
  }
]
```

**Step 5: Create `benchmarks/scenarios/memory_operations.json`**

```json
[
  {
    "id": "mo-full-cycle",
    "category": "memory_operations",
    "input": "Write a note to memory at 'bench/cycle-test.md' with content 'The answer is 42'. Then list the memory tree to confirm it exists. Then read it back and tell me the answer.",
    "success_criteria": [
      {"type": "tool_used", "tool": "memory_write"},
      {"type": "tool_used", "tool": "memory_tree"},
      {"type": "tool_used", "tool": "memory_read"},
      {"type": "response_contains", "text": "42"}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 15
  },
  {
    "id": "mo-search-recall",
    "category": "memory_operations",
    "input": "Save a note to memory at 'bench/search-test.md' with content 'Project deadline is March 15th'. Then search memory for 'deadline' and tell me when it is.",
    "success_criteria": [
      {"type": "tool_used", "tool": "memory_write"},
      {"type": "tool_used", "tool": "memory_search"},
      {"type": "response_contains", "text": "March 15"}
    ],
    "timeout_secs": 30,
    "max_tool_iterations": 10
  }
]
```

**Step 6: Add a deserialization test**

Add to `src/benchmark/scenario.rs` tests:

```rust
#[test]
fn test_load_scenario_files() {
    let scenarios_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/benchmarks/scenarios");
    for entry in std::fs::read_dir(scenarios_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            let content = std::fs::read_to_string(&path).unwrap();
            let scenarios: Vec<Scenario> = serde_json::from_str(&content)
                .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", path.display()));
            assert!(!scenarios.is_empty(), "Empty scenario file: {}", path.display());
            for s in &scenarios {
                assert!(!s.id.is_empty(), "Empty scenario ID in {}", path.display());
                assert!(!s.input.is_empty(), "Empty input in scenario {}", s.id);
            }
        }
    }
}
```

**Step 7: Run tests**

Run: `cargo test --features libsql benchmark::scenario::tests::test_load_scenario_files`
Expected: passes (all 5 JSON files parse correctly).

**Step 8: Commit**

```bash
git add benchmarks/scenarios/ src/benchmark/scenario.rs
git commit -m "feat: add initial benchmark scenario suite (15 scenarios across 5 categories)"
```

---

### Task 4: Build the benchmark runner

**Files:**
- Create: `src/benchmark/runner.rs`
- Modify: `src/benchmark/mod.rs` — add `pub mod runner;`

**Context:** The runner loads scenarios, spins up a real agent per scenario (using patterns from `TestRigBuilder::build()`), sends the input, evaluates criteria, and collects metrics into `ScenarioResult` / `RunResult`. Uses real LLM configured from environment variables.

**Step 1: Write the runner module**

```rust
//! Benchmark runner: executes scenarios against a real agent with real LLM.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::benchmark::metrics::{RunResult, ScenarioResult, TraceMetrics, ToolInvocation};
use crate::benchmark::scenario::{evaluate_criteria, EvalContext, Scenario};

/// Configuration for a benchmark run.
pub struct BenchmarkConfig {
    /// Directory containing scenario JSON files.
    pub scenarios_dir: String,
    /// Maximum wall-clock seconds per scenario (overrides scenario-level timeout).
    pub global_timeout_secs: Option<u64>,
    /// Filter to specific scenario IDs (empty = run all).
    pub filter: Vec<String>,
    /// Filter to specific categories (empty = run all).
    pub category_filter: Vec<String>,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            scenarios_dir: format!(
                "{}/benchmarks/scenarios",
                env!("CARGO_MANIFEST_DIR")
            ),
            global_timeout_secs: None,
            filter: vec![],
            category_filter: vec![],
        }
    }
}

/// Load all scenarios from the scenarios directory.
pub fn load_scenarios(config: &BenchmarkConfig) -> Result<Vec<Scenario>, String> {
    let dir = Path::new(&config.scenarios_dir);
    if !dir.exists() {
        return Err(format!("Scenarios directory not found: {}", config.scenarios_dir));
    }

    let mut scenarios = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read scenarios dir: {e}"))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
            let file_scenarios: Vec<Scenario> = serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
            scenarios.extend(file_scenarios);
        }
    }

    // Apply filters.
    if !config.filter.is_empty() {
        scenarios.retain(|s| config.filter.contains(&s.id));
    }
    if !config.category_filter.is_empty() {
        scenarios.retain(|s| config.category_filter.contains(&s.category));
    }

    Ok(scenarios)
}

/// Run a single scenario against a real agent. Returns the scenario result.
///
/// This is the core function. It:
/// 1. Creates a fresh libSQL database
/// 2. Creates a real LLM provider from the environment
/// 3. Wraps it in InstrumentedLlm for metrics
/// 4. Builds an Agent with real tools and workspace
/// 5. Sends the scenario input
/// 6. Waits for the response (with timeout)
/// 7. Evaluates success criteria
/// 8. Collects TraceMetrics
#[cfg(feature = "libsql")]
pub async fn run_scenario(
    scenario: &Scenario,
    llm: Arc<dyn crate::llm::LlmProvider>,
    global_timeout: Option<Duration>,
) -> ScenarioResult {
    use crate::agent::agent_loop::{Agent, AgentConfig, AgentDeps};
    use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
    use crate::channels::manager::ChannelManager;
    use crate::db::libsql_backend::LibSqlBackend;
    use crate::db::Database;
    use crate::hooks::HookRegistry;
    use crate::safety::{SafetyConfig, SafetyLayer};
    use crate::skills::SkillsConfig;
    use crate::tools::ToolRegistry;
    use crate::workspace::Workspace;

    let start = Instant::now();
    let scenario_id = scenario.id.clone();

    // 1. Fresh database.
    let temp_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            return ScenarioResult {
                scenario_id,
                passed: false,
                trace: empty_trace(),
                response: String::new(),
                error: Some(format!("Failed to create temp dir: {e}")),
            };
        }
    };
    let db_path = temp_dir.path().join("bench.db");
    let backend = match LibSqlBackend::new_local(&db_path).await {
        Ok(b) => b,
        Err(e) => {
            return ScenarioResult {
                scenario_id,
                passed: false,
                trace: empty_trace(),
                response: String::new(),
                error: Some(format!("Failed to create DB: {e}")),
            };
        }
    };
    if let Err(e) = backend.run_migrations().await {
        return ScenarioResult {
            scenario_id,
            passed: false,
            trace: empty_trace(),
            response: String::new(),
            error: Some(format!("Failed to run migrations: {e}")),
        };
    }
    let db: Arc<dyn Database> = Arc::new(backend);

    // 2. Wrap LLM in InstrumentedLlm.
    let instrumented = Arc::new(crate::benchmark::instrumented::InstrumentedLlm::new(
        Arc::clone(&llm),
    ));
    let llm_for_agent: Arc<dyn crate::llm::LlmProvider> =
        Arc::clone(&instrumented) as Arc<dyn crate::llm::LlmProvider>;

    // 3. Tools + workspace.
    let workspace = Arc::new(Workspace::new_with_db("bench-user", Arc::clone(&db)));
    let tools = Arc::new(ToolRegistry::new());
    tools.register_builtin_tools();
    tools.register_dev_tools();
    tools.register_memory_tools(Arc::clone(&workspace));

    // 4. Safety, hooks, cost guard.
    let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
        max_output_length: 100_000,
        injection_check_enabled: false,
    }));
    let hooks = Arc::new(HookRegistry::new());
    let cost_guard = Arc::new(CostGuard::new(CostGuardConfig {
        max_cost_per_day_cents: None,
        max_actions_per_hour: None,
    }));

    let deps = AgentDeps {
        store: Some(Arc::clone(&db)),
        llm: llm_for_agent,
        cheap_llm: None,
        safety,
        tools,
        workspace: Some(workspace),
        extension_manager: None,
        skill_registry: None,
        skill_catalog: None,
        skills_config: SkillsConfig::default(),
        hooks,
        cost_guard,
    };

    // 5. Channel.
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let bench_channel = Arc::new(BenchChannel::new(rx));
    let bench_channel_ref = Arc::clone(&bench_channel);
    let channel_manager = ChannelManager::new();
    channel_manager
        .add(Box::new(BenchChannelHandle::new(Arc::clone(&bench_channel))))
        .await;
    let channels = Arc::new(channel_manager);

    // 6. Agent config.
    let agent_config = AgentConfig {
        name: format!("bench-{}", scenario.id),
        max_parallel_jobs: 1,
        job_timeout: Duration::from_secs(scenario.timeout_secs),
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

    // 7. Spawn agent.
    let agent = Agent::new(agent_config, deps, channels, None, None, None, None, None);
    let agent_handle = tokio::spawn(async move {
        let _ = agent.run().await;
    });

    // 8. Send input and wait for response.
    let timeout = global_timeout.unwrap_or(Duration::from_secs(scenario.timeout_secs));
    let _ = tx
        .send(crate::channels::channel::IncomingMessage::new(
            &scenario.input,
            "bench-user",
            "bench",
        ))
        .await;

    let response = tokio::time::timeout(timeout, bench_channel_ref.wait_for_response()).await;
    agent_handle.abort();

    let wall_time_ms = start.elapsed().as_millis() as u64;

    // 9. Build result.
    let (response_text, hit_timeout) = match response {
        Ok(text) => (text, false),
        Err(_) => ("(timeout)".to_string(), true),
    };

    let tool_calls_completed = bench_channel_ref.tool_calls_completed();
    let tool_timings = bench_channel_ref.tool_timings();

    // Evaluate criteria.
    let eval_ctx = EvalContext {
        response: response_text.clone(),
        tool_calls: tool_calls_completed.clone(),
    };
    let (passed, criterion_results) = evaluate_criteria(&scenario.success_criteria, &eval_ctx);

    // Build trace metrics.
    let tool_invocations: Vec<ToolInvocation> = tool_calls_completed
        .iter()
        .enumerate()
        .map(|(i, (name, success))| {
            let duration_ms = tool_timings.get(i).map(|(_, ms)| *ms).unwrap_or(0);
            ToolInvocation {
                name: name.clone(),
                duration_ms,
                success: *success,
            }
        })
        .collect();

    let trace = TraceMetrics {
        wall_time_ms,
        llm_calls: instrumented.call_count(),
        input_tokens: instrumented.total_input_tokens(),
        output_tokens: instrumented.total_output_tokens(),
        estimated_cost_usd: instrumented.estimated_cost_usd(),
        tool_calls: tool_invocations,
        turns: 1,
        hit_iteration_limit: tool_calls_completed.len() >= scenario.max_tool_iterations,
        hit_timeout,
    };

    let error = if !passed {
        let failures: Vec<String> = criterion_results
            .iter()
            .filter(|r| !r.passed)
            .map(|r| format!("{}: {}", r.criterion, r.reason))
            .collect();
        Some(failures.join("; "))
    } else {
        None
    };

    ScenarioResult {
        scenario_id,
        passed,
        trace,
        response: response_text,
        error,
    }
}

fn empty_trace() -> TraceMetrics {
    TraceMetrics {
        wall_time_ms: 0,
        llm_calls: 0,
        input_tokens: 0,
        output_tokens: 0,
        estimated_cost_usd: 0.0,
        tool_calls: vec![],
        turns: 0,
        hit_iteration_limit: false,
        hit_timeout: false,
    }
}

/// Run all scenarios and produce a RunResult.
#[cfg(feature = "libsql")]
pub async fn run_all(
    config: &BenchmarkConfig,
    llm: Arc<dyn crate::llm::LlmProvider>,
) -> Result<RunResult, String> {
    let scenarios = load_scenarios(config)?;
    if scenarios.is_empty() {
        return Err("No scenarios found".to_string());
    }

    let global_timeout = config.global_timeout_secs.map(Duration::from_secs);
    let mut results = Vec::with_capacity(scenarios.len());

    for scenario in &scenarios {
        eprintln!("  Running: {} ...", scenario.id);
        let result = run_scenario(scenario, Arc::clone(&llm), global_timeout).await;
        let status = if result.passed { "PASS" } else { "FAIL" };
        eprintln!(
            "  {} {} ({}ms, {} tool calls, {} tokens)",
            status,
            scenario.id,
            result.trace.wall_time_ms,
            result.trace.total_tool_calls(),
            result.trace.input_tokens + result.trace.output_tokens,
        );
        results.push(result);
    }

    let run_id = format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        &git_commit_hash().unwrap_or_else(|| "unknown".to_string())[..7.min(
            git_commit_hash().unwrap_or_default().len()
        )]
    );

    let mut run = RunResult::from_scenarios(run_id, results);
    if let Some(hash) = git_commit_hash() {
        run = run.with_commit_hash(hash);
    }

    Ok(run)
}

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
```

**Note:** This references `BenchChannel` and `BenchChannelHandle` — lightweight channel types similar to `TestChannel` but minimal. These will be created in Task 5.

**Also references:** `crate::benchmark::instrumented::InstrumentedLlm` — the promoted version from Task 5.

**Step 2: Run compilation check**

Run: `cargo check --features libsql`
Expected: errors about missing `BenchChannel`, `BenchChannelHandle`, `instrumented` module. These are built in the next task.

**Step 3: Commit (partial — will compile after Task 5)**

Don't commit yet — wait for Task 5 to make it compile.

---

### Task 5: Promote InstrumentedLlm and create BenchChannel

**Files:**
- Create: `src/benchmark/instrumented.rs`
- Create: `src/benchmark/bench_channel.rs`
- Modify: `src/benchmark/mod.rs` — add modules
- Modify: `tests/support/instrumented_llm.rs` — re-export

**Context:** `InstrumentedLlm` needs to be in library code for the runner. `BenchChannel` is a minimal Channel implementation for the benchmark runner — captures responses, tool status events, and provides a `wait_for_response()` future.

**Step 1: Create `src/benchmark/instrumented.rs`**

Copy the full contents of `tests/support/instrumented_llm.rs` into `src/benchmark/instrumented.rs`. Adjust imports: change any `use crate::support::...` to the correct library paths. Remove `#![allow(dead_code)]` if present. The struct should use `crate::llm::LlmProvider` etc.

**Step 2: Create `src/benchmark/bench_channel.rs`**

```rust
//! Minimal channel implementation for benchmark scenarios.
//!
//! Captures agent responses and tool status events without TUI or HTTP overhead.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use futures::stream;
use tokio::sync::{mpsc, Mutex, Notify};

use crate::channels::channel::{
    Channel, ChannelError, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};

/// Minimal channel for benchmark execution.
pub struct BenchChannel {
    rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    responses: Arc<Mutex<Vec<OutgoingResponse>>>,
    status_events: Arc<Mutex<Vec<StatusUpdate>>>,
    tool_start_times: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    tool_timings: Arc<Mutex<Vec<(String, u64)>>>,
    response_notify: Arc<Notify>,
}

impl BenchChannel {
    pub fn new(rx: mpsc::Receiver<IncomingMessage>) -> Self {
        Self {
            rx: Mutex::new(Some(rx)),
            responses: Arc::new(Mutex::new(Vec::new())),
            status_events: Arc::new(Mutex::new(Vec::new())),
            tool_start_times: Arc::new(Mutex::new(HashMap::new())),
            tool_timings: Arc::new(Mutex::new(Vec::new())),
            response_notify: Arc::new(Notify::new()),
        }
    }

    /// Wait for the agent to produce a text response.
    pub async fn wait_for_response(&self) -> String {
        loop {
            // Check if we already have a response.
            let responses = self.responses.lock().await;
            if let Some(r) = responses.last() {
                return r.text.clone();
            }
            drop(responses);
            self.response_notify.notified().await;
        }
    }

    /// Return (name, success) for all completed tool calls.
    pub fn tool_calls_completed(&self) -> Vec<(String, bool)> {
        self.status_events
            .try_lock()
            .expect("lock")
            .iter()
            .filter_map(|s| match s {
                StatusUpdate::ToolCompleted { name, success } => {
                    Some((name.clone(), *success))
                }
                _ => None,
            })
            .collect()
    }

    /// Return (name, duration_ms) for all timed tool calls.
    pub fn tool_timings(&self) -> Vec<(String, u64)> {
        self.tool_timings.try_lock().expect("lock").clone()
    }
}

#[async_trait]
impl Channel for BenchChannel {
    fn name(&self) -> &str {
        "benchmark"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let rx = self
            .rx
            .lock()
            .await
            .take()
            .ok_or(ChannelError::AlreadyStarted)?;
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.responses.lock().await.push(response);
        self.response_notify.notify_waiters();
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Capture timing.
        match &status {
            StatusUpdate::ToolStarted { name } => {
                self.tool_start_times
                    .lock()
                    .await
                    .entry(name.clone())
                    .or_default()
                    .push(Instant::now());
            }
            StatusUpdate::ToolCompleted { name, .. } => {
                if let Some(starts) = self.tool_start_times.lock().await.get_mut(name) {
                    if let Some(start) = starts.pop() {
                        self.tool_timings
                            .lock()
                            .await
                            .push((name.clone(), start.elapsed().as_millis() as u64));
                    }
                }
            }
            _ => {}
        }
        self.status_events.lock().await.push(status);
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

/// Handle wrapper for ChannelManager (same pattern as TestChannelHandle).
pub struct BenchChannelHandle {
    inner: Arc<BenchChannel>,
}

impl BenchChannelHandle {
    pub fn new(inner: Arc<BenchChannel>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl Channel for BenchChannelHandle {
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
    async fn health_check(&self) -> Result<(), ChannelError> {
        self.inner.health_check().await
    }
}
```

**Step 3: Update `src/benchmark/mod.rs`**

```rust
pub mod metrics;
pub mod scenario;
pub mod runner;
pub mod instrumented;
pub mod bench_channel;
```

**Step 4: Update `tests/support/instrumented_llm.rs` to re-export**

Replace contents with:
```rust
pub use ironclaw::benchmark::instrumented::*;
```

**Step 5: Run full compilation and tests**

Run: `cargo check --features libsql`
Expected: compiles.

Run: `cargo test --features libsql`
Expected: all tests pass.

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: 0 warnings.

**Step 6: Commit**

```bash
git add src/benchmark/ tests/support/instrumented_llm.rs
git commit -m "feat: add benchmark runner with BenchChannel and InstrumentedLlm"
```

---

### Task 6: Add the benchmark entry point and baseline management

**Files:**
- Create: `src/benchmark/baseline.rs`
- Create: `src/benchmark/report.rs`
- Modify: `src/benchmark/mod.rs` — add modules
- Create: `benchmarks/.gitignore` — ignore `results/`
- Create: `tests/benchmark_runner.rs` — integration test (feature-gated)

**Context:** Baseline management handles loading/saving/promoting baselines. The report module formats the comparison output. The integration test runs the full benchmark with a real LLM (gated behind `benchmark` feature and `#[ignore]`).

**Step 1: Create `src/benchmark/baseline.rs`**

```rust
//! Baseline management: load, save, and promote benchmark results.

use std::path::Path;

use crate::benchmark::metrics::RunResult;

const BASELINE_FILE: &str = "benchmarks/baselines/baseline.json";

/// Load the baseline from the default path.
pub fn load_baseline() -> Result<Option<RunResult>, String> {
    load_baseline_from(BASELINE_FILE)
}

/// Load a baseline from a specific path.
pub fn load_baseline_from(path: &str) -> Result<Option<RunResult>, String> {
    let path = Path::new(path);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read baseline: {e}"))?;
    let run: RunResult =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse baseline: {e}"))?;
    Ok(Some(run))
}

/// Save a run result to the results directory.
pub fn save_result(result: &RunResult) -> Result<String, String> {
    let dir = format!("{}/benchmarks/results", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create results dir: {e}"))?;

    let filename = format!("{}.json", result.run_id);
    let path = format!("{dir}/{filename}");
    let content =
        serde_json::to_string_pretty(result).map_err(|e| format!("Failed to serialize: {e}"))?;
    std::fs::write(&path, content).map_err(|e| format!("Failed to write result: {e}"))?;
    Ok(path)
}

/// Promote a result file to the baseline.
pub fn promote_to_baseline(result_path: &str) -> Result<(), String> {
    let baseline_path = format!("{}/{BASELINE_FILE}", env!("CARGO_MANIFEST_DIR"));
    let baseline_dir = Path::new(&baseline_path).parent().unwrap();
    std::fs::create_dir_all(baseline_dir)
        .map_err(|e| format!("Failed to create baselines dir: {e}"))?;
    std::fs::copy(result_path, &baseline_path)
        .map_err(|e| format!("Failed to promote baseline: {e}"))?;
    Ok(())
}
```

**Step 2: Create `src/benchmark/report.rs`**

```rust
//! Human-readable benchmark reports.

use crate::benchmark::metrics::{compare_runs, MetricDelta, RunResult};

/// Format a comparison report between baseline and current run.
pub fn format_report(current: &RunResult, baseline: Option<&RunResult>) -> String {
    let mut out = String::new();

    // Header.
    out.push_str(&format!(
        "Benchmark Run: {}\n",
        current.run_id,
    ));
    if let Some(hash) = &current.commit_hash {
        out.push_str(&format!("Commit: {hash}\n"));
    }
    out.push('\n');

    // Correctness.
    let passed = current.scenarios.iter().filter(|s| s.passed).count();
    let total = current.scenarios.len();
    out.push_str(&format!(
        "Scenarios: {passed}/{total} passed ({:.0}%)\n",
        current.pass_rate * 100.0
    ));

    // Per-scenario results.
    for s in &current.scenarios {
        let status = if s.passed { "PASS" } else { "FAIL" };
        out.push_str(&format!(
            "  {} {} ({}ms, {} tools, {} tokens)\n",
            status,
            s.scenario_id,
            s.trace.wall_time_ms,
            s.trace.total_tool_calls(),
            s.trace.input_tokens + s.trace.output_tokens,
        ));
        if let Some(ref err) = s.error {
            out.push_str(&format!("       {err}\n"));
        }
    }

    // Totals.
    out.push_str(&format!(
        "\nTotal: {:.4} USD, {}ms\n",
        current.total_cost_usd, current.total_wall_time_ms
    ));

    // Baseline comparison.
    if let Some(baseline) = baseline {
        out.push('\n');
        out.push_str("--- Baseline Comparison ---\n");

        let baseline_passed = baseline.scenarios.iter().filter(|s| s.passed).count();
        let baseline_total = baseline.scenarios.len();
        out.push_str(&format!(
            "Pass rate: {passed}/{total} (was {baseline_passed}/{baseline_total})\n"
        ));

        // Detect fixed / regressed scenarios.
        for s in &current.scenarios {
            let baseline_scenario = baseline.scenarios.iter().find(|b| b.scenario_id == s.scenario_id);
            if let Some(bs) = baseline_scenario {
                if s.passed && !bs.passed {
                    out.push_str(&format!("  + {} FIXED\n", s.scenario_id));
                } else if !s.passed && bs.passed {
                    out.push_str(&format!("  - {} REGRESSED\n", s.scenario_id));
                }
            } else {
                out.push_str(&format!("  ? {} NEW\n", s.scenario_id));
            }
        }

        // Efficiency deltas.
        let deltas = compare_runs(baseline, current, 0.10);
        if !deltas.is_empty() {
            out.push_str("\nEfficiency changes (>10% threshold):\n");
            for d in &deltas {
                let direction = if d.is_regression {
                    "REGRESSED"
                } else {
                    "IMPROVED"
                };
                out.push_str(&format!(
                    "  {} {}: {:.0} -> {:.0} ({:+.0}%) {}\n",
                    d.scenario_id,
                    d.metric,
                    d.baseline,
                    d.current,
                    d.delta * 100.0,
                    direction,
                ));
            }
        }
    }

    out
}
```

**Step 3: Create `benchmarks/.gitignore`**

```
results/
```

**Step 4: Create `benchmarks/baselines/.gitkeep`**

Empty file to ensure the directory exists in git.

**Step 5: Update `src/benchmark/mod.rs`**

```rust
pub mod metrics;
pub mod scenario;
pub mod runner;
pub mod instrumented;
pub mod bench_channel;
pub mod baseline;
pub mod report;
```

**Step 6: Add unit tests for baseline and report**

Add to `src/benchmark/baseline.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_nonexistent_baseline() {
        let result = load_baseline_from("/tmp/nonexistent_baseline_12345.json");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
```

Add to `src/benchmark/report.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::metrics::{RunResult, ScenarioResult, TraceMetrics, ToolInvocation};

    fn sample_trace() -> TraceMetrics {
        TraceMetrics {
            wall_time_ms: 1500,
            llm_calls: 3,
            input_tokens: 500,
            output_tokens: 100,
            estimated_cost_usd: 0.005,
            tool_calls: vec![ToolInvocation {
                name: "echo".to_string(),
                duration_ms: 5,
                success: true,
            }],
            turns: 1,
            hit_iteration_limit: false,
            hit_timeout: false,
        }
    }

    #[test]
    fn test_format_report_no_baseline() {
        let run = RunResult::from_scenarios(
            "test-run",
            vec![ScenarioResult {
                scenario_id: "test-echo".to_string(),
                passed: true,
                trace: sample_trace(),
                response: "hello".to_string(),
                error: None,
            }],
        );
        let report = format_report(&run, None);
        assert!(report.contains("1/1 passed"));
        assert!(report.contains("PASS"));
        assert!(report.contains("test-echo"));
    }

    #[test]
    fn test_format_report_with_baseline_regression() {
        let baseline = RunResult::from_scenarios(
            "baseline",
            vec![ScenarioResult {
                scenario_id: "test-echo".to_string(),
                passed: true,
                trace: sample_trace(),
                response: "hello".to_string(),
                error: None,
            }],
        );
        let mut current_trace = sample_trace();
        current_trace.wall_time_ms = 3000; // 100% slower
        let current = RunResult::from_scenarios(
            "current",
            vec![ScenarioResult {
                scenario_id: "test-echo".to_string(),
                passed: true,
                trace: current_trace,
                response: "hello".to_string(),
                error: None,
            }],
        );
        let report = format_report(&current, Some(&baseline));
        assert!(report.contains("Baseline Comparison"));
        assert!(report.contains("REGRESSED"));
    }
}
```

**Step 7: Create the integration test entry point**

Create `tests/benchmark_runner.rs`:

```rust
//! Integration test for the benchmark runner.
//!
//! Requires a real LLM provider configured via environment variables.
//! Run with: cargo test --features "libsql,benchmark" --test benchmark_runner -- --ignored

#[cfg(all(feature = "libsql", feature = "benchmark"))]
mod tests {
    use std::sync::Arc;

    use ironclaw::benchmark::baseline;
    use ironclaw::benchmark::metrics::compare_runs;
    use ironclaw::benchmark::report::format_report;
    use ironclaw::benchmark::runner::{run_all, BenchmarkConfig};

    /// Run the full benchmark suite with a real LLM.
    #[tokio::test]
    #[ignore] // Requires LLM API keys
    async fn run_full_benchmark() {
        // Load LLM from environment.
        let config = ironclaw::config::Config::from_env().await.unwrap();
        let session = Arc::new(ironclaw::llm::session::SessionManager::new(
            ironclaw::config::SessionConfig::default(),
        ));
        let llm = ironclaw::llm::create_llm_provider(&config.llm, session).unwrap();

        let bench_config = BenchmarkConfig::default();
        let result = run_all(&bench_config, llm).await.unwrap();

        // Save result.
        let result_path = baseline::save_result(&result).unwrap();
        eprintln!("Results saved to: {result_path}");

        // Load baseline and compare.
        let baseline = baseline::load_baseline().unwrap();
        let report = format_report(&result, baseline.as_ref());
        eprintln!("\n{report}");

        // The test itself just verifies the runner doesn't crash.
        // Pass/fail of individual scenarios is informational.
        assert!(
            !result.scenarios.is_empty(),
            "Expected at least one scenario to run"
        );
    }
}
```

**Step 8: Add `benchmark` feature flag to `Cargo.toml`**

Find the `[features]` section and add:
```toml
benchmark = ["libsql"]
```

**Step 9: Run compilation and tests**

Run: `cargo check --features libsql`
Expected: compiles.

Run: `cargo test --features libsql`
Expected: all tests pass (benchmark_runner test is `#[ignore]`).

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: 0 warnings.

**Step 10: Commit**

```bash
git add src/benchmark/ benchmarks/ tests/benchmark_runner.rs Cargo.toml
git commit -m "feat: add benchmark baseline management, report formatting, and runner entry point"
```

---

## Verification

After all 6 tasks, run the full quality gate:

```bash
cargo fmt --check
cargo clippy --all --benches --tests --examples --all-features
cargo test --features libsql
```

Expected: 0 formatting issues, 0 clippy warnings, 0 test failures.

To run a real benchmark (requires LLM API keys):

```bash
cargo test --features benchmark --test benchmark_runner -- --ignored --nocapture
```

## Summary

| Task | What it builds |
|------|----------------|
| 1 | Promote metrics types to library code |
| 2 | Scenario + Criterion types with evaluation |
| 3 | 15 benchmark scenarios across 5 categories |
| 4 | Benchmark runner (core execution loop) |
| 5 | InstrumentedLlm + BenchChannel in library code |
| 6 | Baseline management, reports, entry point, feature flag |

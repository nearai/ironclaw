# E2E Test Hardening Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix false positives, broken fixtures, missing metrics, and resource leaks in the E2E trace test infrastructure.

**Architecture:** Nine sequential tasks. Tasks 1-2 build shared infrastructure, Task 3 fixes fixture bugs, Tasks 4-5 strengthen assertions and metrics, Tasks 6-9 fix resource management and design issues. Each task is a failing-test-first TDD cycle with a commit at the end.

**Tech Stack:** Rust, tokio, tempfile, serde_json. All code in `tests/` (test-only, `.unwrap()` is fine).

---

### Task 1: Extract shared assertion helpers to `tests/support/assertions.rs`

**Files:**
- Create: `tests/support/assertions.rs`
- Modify: `tests/support/mod.rs`
- Modify: `tests/e2e_spot_checks.rs` (remove local helpers, use shared ones)

**Context:** `e2e_spot_checks.rs` defines 5 assertion helpers locally (lines 33-75). Other test files duplicate similar logic inline. Move them to a shared module and add the critical missing helper: `assert_all_tools_succeeded`.

**Step 1: Write the failing test**

In `tests/support/assertions.rs`, add unit tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assert_all_tools_succeeded_passes() {
        let completed = vec![
            ("echo".to_string(), true),
            ("time".to_string(), true),
        ];
        assert_all_tools_succeeded(&completed); // should not panic
    }

    #[test]
    #[should_panic(expected = "failed tools")]
    fn test_assert_all_tools_succeeded_catches_failure() {
        let completed = vec![
            ("echo".to_string(), true),
            ("time".to_string(), false),
        ];
        assert_all_tools_succeeded(&completed);
    }

    #[test]
    fn test_assert_tools_used_passes() {
        let started = vec!["echo".to_string(), "time".to_string()];
        assert_tools_used(&started, &["echo", "time"]);
    }

    #[test]
    #[should_panic(expected = "not called")]
    fn test_assert_tools_used_catches_missing() {
        let started = vec!["echo".to_string()];
        assert_tools_used(&started, &["time"]);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --features libsql -p ironclaw --test e2e_spot_checks support::assertions`
Expected: compilation error (module doesn't exist yet)

**Step 3: Write the implementation**

Create `tests/support/assertions.rs` with these functions moved from `e2e_spot_checks.rs`:

```rust
//! Shared assertion helpers for E2E trace tests.

use regex::Regex;

/// Assert every tool in `expected` appears in `started` (by name).
pub fn assert_tools_used(started: &[String], expected: &[&str]) {
    for tool in expected {
        assert!(
            started.iter().any(|s| s == tool),
            "tools_used: \"{tool}\" not called, got: {started:?}"
        );
    }
}

/// Assert none of `forbidden` tools appear in `started`.
pub fn assert_tools_not_used(started: &[String], forbidden: &[&str]) {
    for tool in forbidden {
        assert!(
            !started.iter().any(|s| s == tool),
            "tools_not_used: \"{tool}\" was called, got: {started:?}"
        );
    }
}

/// Assert total tool calls <= max.
pub fn assert_max_tool_calls(started: &[String], max: usize) {
    assert!(
        started.len() <= max,
        "max_tool_calls: expected <= {max}, got {}. Tools: {started:?}",
        started.len()
    );
}

/// Assert response text contains all `needles` (case-insensitive).
pub fn assert_response_contains(response: &str, needles: &[&str]) {
    let lower = response.to_lowercase();
    for needle in needles {
        assert!(
            lower.contains(&needle.to_lowercase()),
            "response_contains: missing \"{needle}\" in response: {response}"
        );
    }
}

/// Assert response text matches a regex pattern.
pub fn assert_response_matches(response: &str, pattern: &str) {
    let re = Regex::new(pattern).expect("invalid regex pattern");
    assert!(
        re.is_match(response),
        "response_matches: /{pattern}/ did not match response: {response}"
    );
}

/// Assert ALL completed tools succeeded. Panics listing failed tools.
pub fn assert_all_tools_succeeded(completed: &[(String, bool)]) {
    let failed: Vec<&str> = completed
        .iter()
        .filter(|(_, success)| !*success)
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(
        failed.is_empty(),
        "Expected all tools to succeed, but these failed tools: {failed:?}. All: {completed:?}"
    );
}

/// Assert a specific tool completed successfully at least once.
pub fn assert_tool_succeeded(completed: &[(String, bool)], tool_name: &str) {
    let found = completed
        .iter()
        .any(|(name, success)| name == tool_name && *success);
    assert!(
        found,
        "Expected '{tool_name}' to complete successfully, got: {completed:?}"
    );
}
```

**Step 4: Wire up in mod.rs**

Add `pub mod assertions;` to `tests/support/mod.rs`.

**Step 5: Migrate e2e_spot_checks.rs**

Remove the 5 local helper functions and replace with:
```rust
use crate::support::assertions::*;
```

**Step 6: Run tests to verify**

Run: `cargo test --features libsql -p ironclaw --test e2e_spot_checks`
Expected: All 10 spot check tests still pass.

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: 0 warnings.

**Step 7: Commit**

```bash
git add tests/support/assertions.rs tests/support/mod.rs tests/e2e_spot_checks.rs
git commit -m "refactor: extract shared assertion helpers to support/assertions.rs"
```

---

### Task 2: Add tool output capture to TestChannel and TestRig

**Files:**
- Modify: `tests/support/test_channel.rs`
- Modify: `tests/support/test_rig.rs`

**Context:** `StatusUpdate::ToolResult { name, preview }` events are already captured in the `status_events` vec, but there's no accessor to extract them. Tests need to verify tool outputs, not just names.

**Step 1: Write the failing test**

Add to `tests/support/test_channel.rs` unit tests:
```rust
#[tokio::test]
async fn test_channel_tool_results() {
    let channel = TestChannel::new();
    // Simulate ToolResult event
    channel
        .send_status(
            StatusUpdate::ToolResult {
                name: "echo".to_string(),
                preview: "hello world".to_string(),
            },
            &serde_json::Value::Null,
        )
        .await
        .unwrap();

    let results = channel.tool_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "echo");
    assert_eq!(results[0].1, "hello world");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --features libsql -p ironclaw --test e2e_spot_checks support::test_channel::tests::test_channel_tool_results`
Expected: compilation error (`tool_results` method doesn't exist)

**Step 3: Implement `tool_results()` on TestChannel**

Add to `test_channel.rs`:
```rust
/// Return `(name, preview)` for all `ToolResult` events captured so far.
pub fn tool_results(&self) -> Vec<(String, String)> {
    self.captured_status_events()
        .iter()
        .filter_map(|s| match s {
            StatusUpdate::ToolResult { name, preview } => {
                Some((name.clone(), preview.clone()))
            }
            _ => None,
        })
        .collect()
}
```

**Step 4: Add wrapper to TestRig**

Add to `test_rig.rs`:
```rust
/// Return `(name, preview)` for all `ToolResult` events captured so far.
pub fn tool_results(&self) -> Vec<(String, String)> {
    self.channel.tool_results()
}
```

**Step 5: Run tests**

Run: `cargo test --features libsql -p ironclaw --test e2e_spot_checks`
Expected: all pass, including the new test.

**Step 6: Commit**

```bash
git add tests/support/test_channel.rs tests/support/test_rig.rs
git commit -m "feat: add tool output capture via tool_results() accessor"
```

---

### Task 3: Fix broken fixture files

**Files:**
- Modify: `tests/fixtures/llm_traces/spot/tool_time.json`
- Modify: `tests/fixtures/llm_traces/spot/robust_correct_tool.json`
- Modify: `tests/fixtures/llm_traces/coverage/memory_full_cycle.json`

**Context:** Three fixtures have wrong tool parameters. The tests pass anyway because assertions are too weak (P0 issue). Fixing the fixtures is a prerequisite for Task 4 (adding success assertions).

**Step 1: Fix `tool_time.json`**

Change `"arguments": {}` to `"arguments": { "operation": "now" }` (line 14).

The `time` tool requires `operation` (enum: "now", "parse", "format", "diff"). Missing it causes `require_str` to return `ToolError::InvalidParameters`.

**Step 2: Fix `robust_correct_tool.json`**

Same change: `"arguments": {}` to `"arguments": { "operation": "now" }` (line 14).

**Step 3: Fix `memory_full_cycle.json`**

In step 1 (memory_write call), change `"path": "test/coverage-note.md"` to `"target": "test/coverage-note.md"` (line 13).

The `memory_write` tool uses `target` parameter (optional, defaults to "daily_log"). Using `path` is silently ignored and the write goes to the wrong location.

**Step 4: Run affected tests**

Run: `cargo test --features libsql -p ironclaw --test e2e_spot_checks spot_tool_time spot_robust_correct_tool`
Expected: both pass (fixtures now have correct parameters, tools succeed).

Run: `cargo test --features libsql -p ironclaw --test e2e_tool_coverage test_memory_full_cycle`
Expected: passes.

**Step 5: Commit**

```bash
git add tests/fixtures/llm_traces/spot/tool_time.json \
        tests/fixtures/llm_traces/spot/robust_correct_tool.json \
        tests/fixtures/llm_traces/coverage/memory_full_cycle.json
git commit -m "fix: correct tool parameters in 3 broken trace fixtures"
```

---

### Task 4: Strengthen assertions across all E2E tests

**Files:**
- Modify: `tests/e2e_spot_checks.rs`
- Modify: `tests/e2e_trace_file_tools.rs`
- Modify: `tests/e2e_trace_memory.rs`
- Modify: `tests/e2e_advanced_traces.rs`
- Modify: `tests/e2e_safety_layer.rs`
- Modify: `tests/e2e_status_events.rs`
- Modify: `tests/e2e_tool_coverage.rs`
- Modify: `tests/e2e_metrics_test.rs`

**Context:** After Tasks 1-3, we have shared helpers and correct fixtures. Now add `assert_all_tools_succeeded` (or `assert_tool_succeeded`) calls to every test that exercises tools. Also add `tool_results()` assertions where possible.

**Step 1: Add imports to each test file**

Every test file that uses tools needs:
```rust
use crate::support::assertions::{assert_all_tools_succeeded, assert_tool_succeeded};
```

**Step 2: Add success assertions**

For each test that calls tools, after the existing `tool_calls_started()` checks, add:
```rust
let completed = rig.tool_calls_completed();
assert_all_tools_succeeded(&completed);
```

Tests to update (each gets this pattern):

- `e2e_spot_checks.rs`: `spot_tool_time`, `spot_robust_correct_tool`, `spot_chain_write_read`, `spot_memory_save_recall`, `spot_bench_meeting` (5 tests)
- `e2e_trace_file_tools.rs`: `test_file_write_and_read` (1 test)
- `e2e_trace_memory.rs`: `test_memory_write_flow` (1 test)
- `e2e_advanced_traces.rs`: `multi_turn_memory_coherence`, `workspace_semantic_search`, `long_tool_chain` (3 tests -- skip `tool_error_recovery` and `iteration_limit_stops_runaway` which expect failures)
- `e2e_metrics_test.rs`: `test_metrics_collected_from_tool_trace` (1 test)

For `e2e_tool_coverage.rs` and `e2e_safety_layer.rs`: already have per-tool success checks, but add `assert_all_tools_succeeded` as a catch-all.

For `e2e_status_events.rs`: already checks `all_success` inline, refactor to use shared helper.

**Step 3: Add tool result content assertions where meaningful**

For tests where we know what the tool should output:
- `spot_tool_time`: assert `tool_results()` contains a result for "time" with preview matching `20\d{2}`
- `spot_chain_write_read`: assert `tool_results()` for "read_file" contains "ironclaw spot check"
- `test_memory_full_cycle`: assert `tool_results()` for "memory_read" contains "answer is 42"

**Step 4: Run all tests**

Run: `cargo test --features libsql`
Expected: all pass.

Run: `cargo clippy --all --benches --tests --examples --all-features`
Expected: 0 warnings.

**Step 5: Commit**

```bash
git add tests/e2e_*.rs
git commit -m "fix: add tool success and output assertions to eliminate false positives"
```

---

### Task 5: Capture per-tool timing from status events

**Files:**
- Modify: `tests/support/test_channel.rs`
- Modify: `tests/support/test_rig.rs`
- Modify: `tests/support/metrics.rs`

**Context:** `TraceMetrics.tool_calls[].duration_ms` is always 0. We can compute it from the timestamp delta between `ToolStarted` and `ToolCompleted` events for each tool.

**Step 1: Write the failing test**

Add to `tests/support/test_channel.rs`:
```rust
#[tokio::test]
async fn test_channel_tool_timings() {
    let channel = TestChannel::new();
    channel
        .send_status(StatusUpdate::ToolStarted { name: "echo".to_string() }, &serde_json::Value::Null)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    channel
        .send_status(
            StatusUpdate::ToolCompleted { name: "echo".to_string(), success: true },
            &serde_json::Value::Null,
        )
        .await
        .unwrap();

    let timings = channel.tool_timings();
    assert_eq!(timings.len(), 1);
    assert_eq!(timings[0].0, "echo");
    assert!(timings[0].1 >= 40, "Expected >= 40ms, got {}ms", timings[0].1);
}
```

**Step 2: Implement tool timing capture**

Add to `TestChannel`:
- A new field: `tool_start_times: Arc<Mutex<HashMap<String, Instant>>>`
- In `send_status`, when `ToolStarted` arrives, record `Instant::now()` keyed by tool name
- When `ToolCompleted` arrives, compute elapsed and store in a `tool_timings: Arc<Mutex<Vec<(String, u64)>>>` vec
- Add accessor: `pub fn tool_timings(&self) -> Vec<(String, u64)>`

**Step 3: Wire into TestRig::collect_metrics()**

Replace the hardcoded `duration_ms: 0` with actual timing from `self.channel.tool_timings()`:
```rust
let timings = self.channel.tool_timings();
let tool_invocations: Vec<ToolInvocation> = completed
    .iter()
    .enumerate()
    .map(|(i, (name, success))| {
        let duration_ms = timings
            .iter()
            .filter(|(n, _)| n == name)
            .nth(/* match index */)
            .map(|(_, ms)| *ms)
            .unwrap_or(0);
        ToolInvocation {
            name: name.clone(),
            duration_ms,
            success: *success,
        }
    })
    .collect();
```

**Step 4: Run tests**

Run: `cargo test --features libsql -p ironclaw --test e2e_spot_checks support::test_channel`
Expected: all pass including new timing test.

Run: `cargo test --features libsql`
Expected: all pass.

**Step 5: Commit**

```bash
git add tests/support/test_channel.rs tests/support/test_rig.rs tests/support/metrics.rs
git commit -m "feat: capture per-tool timing from ToolStarted/ToolCompleted events"
```

---

### Task 6: Add cleanup guards for temp file tests

**Files:**
- Create: `tests/support/cleanup.rs`
- Modify: `tests/support/mod.rs`
- Modify: `tests/e2e_spot_checks.rs`
- Modify: `tests/e2e_advanced_traces.rs`
- Modify: `tests/e2e_tool_coverage.rs`
- Modify: `tests/e2e_trace_file_tools.rs`
- Modify: `tests/e2e_metrics_test.rs`

**Context:** Tests create files in hardcoded `/tmp/` paths (required because fixture JSON is static). If a test panics before cleanup, files persist. A `Drop`-based guard ensures cleanup on both success and panic.

**Step 1: Create cleanup guard**

```rust
//! RAII cleanup guard for test directories and files.

/// Removes listed paths when dropped, ensuring cleanup even on panic.
pub struct CleanupGuard {
    paths: Vec<String>,
}

impl CleanupGuard {
    pub fn new() -> Self {
        Self { paths: Vec::new() }
    }

    /// Register a file path for cleanup on drop.
    pub fn file(mut self, path: impl Into<String>) -> Self {
        self.paths.push(path.into());
        self
    }

    /// Register a directory path for cleanup on drop.
    pub fn dir(mut self, path: impl Into<String>) -> Self {
        self.paths.push(path.into());
        self
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
```

**Step 2: Add unit test**

```rust
#[test]
fn test_cleanup_guard_removes_file() {
    let path = "/tmp/ironclaw_cleanup_guard_test.txt";
    std::fs::write(path, "test").unwrap();
    {
        let _guard = CleanupGuard::new().file(path);
        assert!(std::path::Path::new(path).exists());
    } // guard dropped here
    assert!(!std::path::Path::new(path).exists());
}
```

**Step 3: Migrate test files**

Replace manual cleanup patterns with guard. Example for `spot_chain_write_read`:

Before:
```rust
let _ = std::fs::remove_file("/tmp/ironclaw_spot_test.txt");
// ... test body ...
let _ = std::fs::remove_file("/tmp/ironclaw_spot_test.txt");
```

After:
```rust
let _cleanup = CleanupGuard::new().file("/tmp/ironclaw_spot_test.txt");
let _ = std::fs::remove_file("/tmp/ironclaw_spot_test.txt"); // pre-clean
// ... test body ...
// cleanup happens automatically on drop
```

For directory-based tests (e2e_tool_coverage.rs):
```rust
let _cleanup = CleanupGuard::new().dir("/tmp/ironclaw_coverage_test_list_dir");
// setup + test body
// cleanup automatic
```

Remove all `cleanup_test_dir()` calls and function definitions from test files. Keep `setup_test_dir()` (creates the dir) but remove the manual cleanup calls.

**Step 4: Run tests**

Run: `cargo test --features libsql`
Expected: all pass.

**Step 5: Commit**

```bash
git add tests/support/cleanup.rs tests/support/mod.rs tests/e2e_*.rs
git commit -m "refactor: use RAII CleanupGuard for deterministic temp file cleanup"
```

---

### Task 7: Add graceful shutdown and Drop impl for TestRig

**Files:**
- Modify: `tests/support/test_rig.rs`
- Modify: `tests/support/test_channel.rs`

**Context:** `TestRig::shutdown()` calls `.abort()` which forcefully kills the agent. No `Drop` impl means forgetting `shutdown()` leaks the task. Add a graceful path with timeout, and a `Drop` impl as a safety net.

**Step 1: Add a shutdown signal to TestChannel**

Add a `shutdown` flag to TestChannel:
```rust
shutdown: Arc<AtomicBool>,
```

Add method:
```rust
pub fn signal_shutdown(&self) {
    self.shutdown.store(true, Ordering::SeqCst);
}
```

**Step 2: Refactor TestRig::shutdown()**

```rust
pub async fn shutdown_graceful(self) {
    self.channel.signal_shutdown();
    // Wait up to 2 seconds for graceful exit.
    let timeout = tokio::time::timeout(Duration::from_secs(2), &mut self.agent_handle);
    if timeout.await.is_err() {
        self.agent_handle.abort();
    }
}

/// Synchronous abort (backwards-compatible, for use in sync Drop).
pub fn shutdown(self) {
    self.agent_handle.abort();
}
```

**Step 3: Add Drop impl**

```rust
impl Drop for TestRig {
    fn drop(&mut self) {
        // If shutdown() wasn't called explicitly, abort the agent task.
        if !self.agent_handle.is_finished() {
            self.agent_handle.abort();
        }
    }
}
```

Note: `TestRig::shutdown(self)` consumes self, so Drop won't fire after it. Drop only fires if the rig is dropped without calling shutdown (e.g., test panic). Move `agent_handle` to `Option<JoinHandle>` to support both paths.

**Step 4: Run tests**

Run: `cargo test --features libsql`
Expected: all pass.

**Step 5: Commit**

```bash
git add tests/support/test_rig.rs tests/support/test_channel.rs
git commit -m "fix: add Drop impl and graceful shutdown for TestRig"
```

---

### Task 8: Replace agent startup sleep with ready signal

**Files:**
- Modify: `tests/support/test_rig.rs`
- Modify: `tests/support/test_channel.rs`

**Context:** `TestRigBuilder::build()` uses `tokio::time::sleep(Duration::from_millis(100))` after spawning the agent. This is a race condition on slow systems.

**Step 1: Add a ready signal**

Add to TestChannel:
```rust
ready_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
ready_rx: Arc<Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
```

In `TestChannel::new()`, create a oneshot channel. In `Channel::start()` (which the agent calls during startup), send the ready signal after returning the stream.

**Step 2: Wait for ready in build()**

Replace:
```rust
tokio::time::sleep(Duration::from_millis(100)).await;
```

With:
```rust
// Wait for the agent to call channel.start() (up to 5 seconds).
let ready_rx = test_channel.take_ready_rx();
if let Some(rx) = ready_rx {
    let _ = tokio::time::timeout(Duration::from_secs(5), rx).await;
}
```

**Step 3: Run tests**

Run: `cargo test --features libsql`
Expected: all pass, potentially faster (no 100ms sleep).

**Step 4: Commit**

```bash
git add tests/support/test_rig.rs tests/support/test_channel.rs
git commit -m "fix: replace agent startup sleep with ready signal"
```

---

### Task 9: Replace string-matching hit_iteration_limit with structured detection

**Files:**
- Modify: `tests/support/test_rig.rs`
- Modify: `tests/support/test_channel.rs`

**Context:** `collect_metrics()` uses `msg.contains("iteration") || msg.contains("limit")` which is fragile. Instead, count ToolCompleted events and compare against the configured `max_tool_iterations`.

**Step 1: Store max_tool_iterations in TestRig**

Add field:
```rust
max_tool_iterations: usize,
```

Set it from `TestRigBuilder` during `build()`.

**Step 2: Replace heuristic with count-based detection**

```rust
let hit_iteration_limit = {
    let tool_completed_count = completed.len();
    tool_completed_count >= self.max_tool_iterations
};
```

This is deterministic: if the number of completed tool calls equals or exceeds the configured limit, the iteration limit was hit.

**Step 3: Make hit_timeout settable**

Add a method or parameter so callers can indicate timeout:
```rust
pub async fn collect_metrics_with_timeout(&self, hit_timeout: bool) -> TraceMetrics {
    let mut metrics = self.collect_metrics().await;
    metrics.hit_timeout = hit_timeout;
    metrics
}
```

Or simply: change `collect_metrics` to take an optional `hit_timeout: bool` param.

**Step 4: Run tests**

Run: `cargo test --features libsql`
Expected: all pass. The `iteration_limit_stops_runaway` test in `e2e_advanced_traces.rs` should still detect the iteration limit correctly.

**Step 5: Commit**

```bash
git add tests/support/test_rig.rs tests/support/test_channel.rs
git commit -m "fix: replace fragile string-matching iteration limit detection with count-based"
```

---

## Verification

After all 9 tasks, run the full quality gate:

```bash
cargo fmt --check
cargo clippy --all --benches --tests --examples --all-features
cargo test --features libsql
```

Expected: 0 formatting issues, 0 clippy warnings, 0 test failures.

## Summary

| Task | Category | What it fixes |
|------|----------|---------------|
| 1 | P0 | Shared assertion helpers (foundation for Task 4) |
| 2 | P0 | Tool output capture (enables content assertions) |
| 3 | P0 | 3 broken fixtures (time, memory_write params) |
| 4 | P0 | False positives in 15+ tests |
| 5 | P1 | Per-tool timing metrics always zero |
| 6 | P1 | Temp file cleanup on panic |
| 7 | P2 | Leaked agent tasks, no graceful shutdown |
| 8 | P2 | Agent startup race condition (100ms sleep) |
| 9 | P2 | Fragile iteration limit detection |

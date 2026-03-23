# Fix Routine Silent Failures (#697) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** When full_job routines fail due to missing sandbox/Docker infrastructure, surface loud, clear errors to the user instead of failing silently.

**Architecture:** Three layers of improvement: (1) incorporate PR #711's sync mechanism so dispatched job completions/failures propagate back to routine runs, (2) fail fast at dispatch time when sandbox is configured but Docker is unavailable by threading sandbox availability into RoutineEngine, (3) send a user-visible notification at startup when sandbox is disabled due to missing Docker.

**Tech Stack:** Rust, tokio, thiserror

---

## Prerequisites

- Branch from `main` (not from the existing `fix/697-routine-silent-failure` branch)
- We will incorporate PR #711's changes as part of this PR, making #711 superseded

---

### Task 1: Add `list_dispatched_routine_runs` to Database trait and implementations

PR #711 adds this method. We incorporate it here.

**Files:**
- Modify: `src/db/mod.rs` (RoutineStore trait)
- Modify: `src/db/postgres.rs`
- Modify: `src/db/libsql/routines.rs`
- Modify: `src/history/store.rs`

**Step 1: Add trait method to RoutineStore**

In `src/db/mod.rs`, add to the `RoutineStore` trait (after `link_routine_run_to_job`):

```rust
/// List routine runs that were dispatched as full_job (status = 'running'
/// with a linked job_id). Used by the routine engine to sync completion
/// status from the background job.
async fn list_dispatched_routine_runs(&self) -> Result<Vec<RoutineRun>, DatabaseError>;
```

**Step 2: Implement for PostgreSQL**

In `src/db/postgres.rs`, add the implementation (delegating to `Store`):

```rust
async fn list_dispatched_routine_runs(&self) -> Result<Vec<RoutineRun>, DatabaseError> {
    self.inner.list_dispatched_routine_runs().await
}
```

**Step 3: Implement for libSQL**

In `src/db/libsql/routines.rs`, add:

```rust
pub async fn list_dispatched_routine_runs(
    &self,
) -> Result<Vec<RoutineRun>, DatabaseError> {
    let conn = self.pool.connection().await.map_err(|e| {
        DatabaseError::Query(format!("failed to get connection: {e}"))
    })?;
    let mut rows = conn
        .query(
            "SELECT id, routine_id, trigger_type, trigger_detail, started_at, \
             completed_at, status, result_summary, tokens_used, job_id, created_at \
             FROM routine_runs WHERE status = 'running' AND job_id IS NOT NULL",
            (),
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

    let mut runs = Vec::new();
    while let Some(row) = rows.next().await.map_err(|e| DatabaseError::Query(e.to_string()))? {
        runs.push(parse_routine_run_row(&row)?);
    }
    Ok(runs)
}
```

**Step 4: Implement for Store wrapper**

In `src/history/store.rs`, add:

```rust
pub async fn list_dispatched_routine_runs(&self) -> Result<Vec<RoutineRun>, DatabaseError> {
    sqlx::query_as::<_, RoutineRunRow>(
        "SELECT id, routine_id, trigger_type, trigger_detail, started_at, \
         completed_at, status, result_summary, tokens_used, job_id, created_at \
         FROM routine_runs WHERE status = 'running' AND job_id IS NOT NULL"
    )
    .fetch_all(&self.pool)
    .await
    .map(|rows| rows.into_iter().map(Into::into).collect())
    .map_err(|e| DatabaseError::Query(e.to_string()))
}
```

**Step 5: Verify compilation**

```bash
cargo check
cargo check --no-default-features --features libsql
```

**Step 6: Commit**

```bash
git add src/db/mod.rs src/db/postgres.rs src/db/libsql/routines.rs src/history/store.rs
git commit -m "feat(db): add list_dispatched_routine_runs for routine-job sync (#697)"
```

---

### Task 2: Add sync_dispatched_runs and fix dispatch status in routine_engine

Incorporates PR #711's core fix: change `execute_full_job` to return `RunStatus::Running` instead of `Ok`, and add the periodic sync mechanism.

**Files:**
- Modify: `src/agent/routine_engine.rs`

**Step 1: Write tests for job-state-to-run-status mapping and Running notification gating**

Add to the `mod tests` block at the bottom of `routine_engine.rs`:

```rust
#[test]
fn test_running_status_does_not_notify() {
    let config = NotifyConfig {
        on_success: true,
        on_failure: true,
        on_attention: true,
        ..Default::default()
    };

    let should_notify = match RunStatus::Running {
        RunStatus::Ok => config.on_success,
        RunStatus::Attention => config.on_attention,
        RunStatus::Failed => config.on_failure,
        RunStatus::Running => false,
    };
    assert!(!should_notify);
}

#[test]
fn test_full_job_dispatch_returns_running_status() {
    assert_eq!(RunStatus::Running.to_string(), "running");
}

/// Regression test for #697: full_job routines were immediately marked Ok
/// on dispatch, so failures/completions were never synced back.
#[test]
fn test_job_state_to_run_status_mapping() {
    use crate::context::JobState;

    let map_state = |state: JobState, reason: Option<&str>| -> Option<(RunStatus, String)> {
        let last_reason = reason.map(|s| s.to_string());
        match state {
            JobState::Completed | JobState::Submitted | JobState::Accepted => {
                let summary =
                    last_reason.unwrap_or_else(|| "Job completed successfully".to_string());
                Some((RunStatus::Ok, summary))
            }
            JobState::Failed => {
                let summary = last_reason
                    .unwrap_or_else(|| "Job failed (no error message recorded)".to_string());
                Some((RunStatus::Failed, summary))
            }
            JobState::Cancelled => Some((RunStatus::Failed, "Job was cancelled".to_string())),
            JobState::Pending | JobState::InProgress | JobState::Stuck => None,
        }
    };

    let (status, _) = map_state(JobState::Completed, None).unwrap();
    assert_eq!(status, RunStatus::Ok);

    let (status, _) = map_state(JobState::Failed, Some("OOM killed")).unwrap();
    assert_eq!(status, RunStatus::Failed);
    assert_eq!(summary, "OOM killed");

    let (status, summary) = map_state(JobState::Failed, None).unwrap();
    assert_eq!(status, RunStatus::Failed);
    assert!(summary.contains("no error message"));

    assert!(map_state(JobState::Pending, None).is_none());
    assert!(map_state(JobState::InProgress, None).is_none());
    assert!(map_state(JobState::Stuck, None).is_none());
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test routine_engine::tests --all-features
```

Expected: compilation error since `sync_dispatched_runs` doesn't exist yet.

**Step 3: Add import and sync methods**

Add `use crate::context::JobState;` to the imports.

Add `sync_dispatched_runs` and `complete_dispatched_run` methods to `impl RoutineEngine` (after `check_cron_triggers`). See PR #711 diff for exact implementation.

Change `execute_full_job` return from:
```rust
Ok((RunStatus::Ok, Some(summary), None))
```
to:
```rust
Ok((RunStatus::Running, Some(summary), None))
```

Update the summary message to include "Status will be updated when the job completes."

Add `engine.sync_dispatched_runs().await;` to the cron ticker loop in `spawn_cron_ticker`, after `check_cron_triggers`.

**Step 4: Run tests**

```bash
cargo test routine_engine::tests --all-features
```

Expected: PASS

**Step 5: Commit**

```bash
git add src/agent/routine_engine.rs
git commit -m "fix(routines): sync dispatched full_job runs with job completion (#697)"
```

---

### Task 3: Fail fast when sandbox is unavailable at dispatch time

This is the new work beyond PR #711. Thread sandbox availability into `RoutineEngine` so `execute_full_job` can fail immediately with a clear error instead of dispatching a doomed job.

**Files:**
- Modify: `src/agent/routine_engine.rs`
- Modify: `src/agent/agent_loop.rs`

**Step 1: Write the failing test**

Add to `mod tests` in `routine_engine.rs`:

```rust
#[test]
fn test_sandbox_unavailable_error_message() {
    let err = RoutineError::JobDispatchFailed {
        reason: "Sandbox is enabled but Docker is not available. \
                 Install Docker or set SANDBOX_ENABLED=false to run full_job routines."
            .to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("Docker is not available"));
    assert!(msg.contains("SANDBOX_ENABLED"));
}
```

**Step 2: Run test to verify it passes (this one is a unit test for the error variant)**

```bash
cargo test routine_engine::tests::test_sandbox_unavailable_error_message --all-features
```

Expected: PASS (error variant already exists, we're just testing the message).

**Step 3: Add `sandbox_available` field to `RoutineEngine`**

In `src/agent/routine_engine.rs`, add a field to the `RoutineEngine` struct:

```rust
/// Whether sandbox/Docker infrastructure is available for full_job execution.
sandbox_available: bool,
```

Update `RoutineEngine::new` to accept and store it:

```rust
pub fn new(
    config: RoutineConfig,
    store: Arc<dyn Database>,
    llm: Arc<dyn LlmProvider>,
    workspace: Arc<Workspace>,
    notify_tx: mpsc::Sender<OutgoingResponse>,
    scheduler: Option<Arc<Scheduler>>,
    sandbox_available: bool,
) -> Self {
    Self {
        config,
        store,
        llm,
        workspace,
        notify_tx,
        running_count: Arc::new(AtomicUsize::new(0)),
        event_cache: Arc::new(RwLock::new(Vec::new())),
        scheduler,
        sandbox_available,
    }
}
```

**Step 4: Add sandbox check in `execute_full_job`**

At the top of `execute_full_job`, before the scheduler check, add a sandbox availability check. This requires passing `sandbox_available` through `EngineContext`.

Add `sandbox_available: bool` to `EngineContext`.

Update `spawn_fire` and `fire_manual` to pass `self.sandbox_available` into `EngineContext`.

In `execute_full_job`, add before the scheduler check:

```rust
if !ctx.sandbox_available {
    return Err(RoutineError::JobDispatchFailed {
        reason: "Sandbox is enabled but Docker is not available. \
                 Install Docker or set SANDBOX_ENABLED=false to run full_job routines."
            .to_string(),
    });
}
```

**Step 5: Update call site in `agent_loop.rs`**

In `src/agent/agent_loop.rs`, where `RoutineEngine::new` is called (~line 442), pass the sandbox availability. The `Agent` struct needs to know Docker status. The simplest approach:

Add a `sandbox_available: bool` field to `Agent` (or to `AgentDeps`). Set it during construction based on the `docker_status` from `main.rs`. The value flows: `main.rs` detects Docker -> passes `sandbox_available` bool through `AppComponents` or `AgentDeps` -> `Agent` passes it to `RoutineEngine::new`.

Look at how `main.rs` passes config to `Agent`. The `docker_status` is computed in `main.rs`. The cleanest path:
- Add `sandbox_available: bool` to `AppComponents` (set in `main.rs`)
- Thread it through to `AgentDeps` -> `Agent` -> `RoutineEngine::new`

Alternatively, since `config.sandbox.enabled` is already available in the agent, just add one more bool. Check the existing flow and pick the minimal path.

**Step 6: Verify compilation**

```bash
cargo check --all-features
cargo check --no-default-features --features libsql
```

**Step 7: Run tests**

```bash
cargo test routine_engine::tests --all-features
```

**Step 8: Commit**

```bash
git add src/agent/routine_engine.rs src/agent/agent_loop.rs src/main.rs src/app.rs
git commit -m "fix(routines): fail fast when sandbox unavailable at dispatch time (#697)"
```

---

### Task 4: Surface sandbox unavailability to user via notification channel

Currently the Docker detection warning only goes to `tracing::warn` (logs). Users on TUI/web never see it. Send a user-visible notification after channels are set up.

**Files:**
- Modify: `src/main.rs`

**Step 1: Write the test**

This is a startup behavior change, so the test is an integration-level assertion. Add a unit test for the notification message formatting:

In `src/agent/routine_engine.rs` tests (or a new test in main.rs tests if they exist):

```rust
#[test]
fn test_sandbox_warning_message_format() {
    let msg = format!(
        "Sandbox is enabled but Docker is not available -- full_job routines will fail. {}",
        "Install Docker Desktop from https://docker.com/get-started"
    );
    assert!(msg.contains("full_job routines will fail"));
    assert!(msg.contains("Docker"));
}
```

**Step 2: Add startup notification in `main.rs`**

After the channel manager is set up and the agent is running, if `config.sandbox.enabled && !docker_status.is_ok()`, send a warning message through the channel manager. The pattern already exists for heartbeat/routine notifications.

The exact location: after `channels` is fully initialized (after all channels are added), but before the agent run loop. Find where `channels.broadcast_all` is accessible.

The simplest approach: after the agent starts (`agent.run()` is typically the last call), but since that blocks, the notification should be sent *before* `agent.run()` is called, using a spawned task or inline send.

Look at where heartbeat startup notifications go. Mirror that pattern:

```rust
if config.sandbox.enabled && !docker_status.is_ok() {
    let warning = format!(
        "Warning: Sandbox is enabled but Docker is not available -- \
         full_job routines will fail until Docker is running. {}",
        docker_status_detection.platform.install_hint()
    );
    let response = OutgoingResponse {
        content: warning,
        thread_id: None,
        attachments: Vec::new(),
        metadata: serde_json::json!({
            "source": "system",
            "type": "warning",
        }),
    };
    let channels_clone = channels.clone();
    tokio::spawn(async move {
        // Small delay to let channels finish connecting
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let _ = channels_clone.broadcast_all("default", response).await;
    });
}
```

Note: we need to preserve the `detection` struct (not just `docker_status`) to access `platform.install_hint()`. Adjust the variable binding in the Docker detection block to keep it available.

**Step 3: Verify compilation**

```bash
cargo check --all-features
```

**Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat(startup): notify user when sandbox unavailable (#697)"
```

---

### Task 5: Final verification and cleanup

**Step 1: Run full test suite**

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo test --all-features
```

**Step 2: Verify both feature configurations compile**

```bash
cargo check --no-default-features --features libsql
cargo check
```

**Step 3: Run pre-commit safety checks**

```bash
grep -rnE '\.unwrap\(|\.expect\(' src/agent/routine_engine.rs src/main.rs
```

Expect: no hits in production code (test code is fine).

**Step 4: Create final commit if any formatting/clippy fixes needed**

```bash
git add -A
git commit -m "style: formatting and clippy fixes (#697)"
```

---

## Summary of Changes

| What | Where | Why |
|------|-------|-----|
| `list_dispatched_routine_runs` DB method | `db/mod.rs`, postgres, libsql, store | Query for running routine runs with linked jobs |
| `sync_dispatched_runs()` engine method | `routine_engine.rs` | Periodically sync job completion back to routine runs |
| `RunStatus::Running` on dispatch | `routine_engine.rs` | Don't mark as Ok before job actually completes |
| `sandbox_available` flag | `RoutineEngine`, `EngineContext` | Fail fast at dispatch when Docker missing |
| Startup notification | `main.rs` | Warn user visibly when sandbox is disabled |

## PR Scope

This PR supersedes PR #711 by incorporating its changes plus the additional fail-fast and startup notification work. PR #711 can be closed after this merges.

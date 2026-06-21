# Reborn Concurrent Turn Runners Implementation Plan

> **For agentic workers:** Implement task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the Reborn runtime execute multiple turn runs (chat + triggers) concurrently instead of strictly one-at-a-time, with a configurable global worker pool and an optional per-user concurrency cap.

**Architecture:** Today exactly one `TurnRunnerWorker` is spawned; its claim loop awaits each run to completion before claiming the next, so conversation 2 (or a trigger) waits behind conversation 1. Fix = two orthogonal, composable knobs: (1) spawn **N** distinct workers sharing the same atomic claim port + cloned wake receiver; (2) enforce a **per-user (tenant_id, user_id)** cap inside the store's atomic `claim_next_run`, so one user cannot occupy the whole pool. Both are config-driven through the existing `[runner]` TOML section.

**Tech Stack:** Rust, tokio, `crates/ironclaw_turns` (claim/store contracts), `crates/ironclaw_reborn` (worker + runtime composition), `crates/ironclaw_reborn_composition` (spawn + runtime input), `crates/ironclaw_reborn_config` (TOML), `crates/ironclaw_reborn_cli` (settings wiring).

## Global Constraints

- No `.unwrap()` / `.expect()` in production code (tests fine).
- Strong types over primitives: cap is `Option<NonZeroU32>`, worker count is `NonZeroUsize`.
- Per-user identity key = `(TenantId, UserId)` where `UserId` comes from `TurnScope::thread_owner.explicit_owner_user_id()`. Runs whose owner is `ActorFallback` or `Ownerless` are **not** counted against any per-user bucket (they remain bounded by the global pool only).
- The per-user cap is a **concurrency** cap (counts only runs currently in `TurnStatus::Running`), NOT an admission cap. Do not reuse / extend the `admission_reservations` system — it has submit-time queued+running+blocked semantics, which is the wrong granularity.
- Background tasks must use `debug!`, never `info!`/`warn!` for routine flow (REPL/TUI corruption rule).
- All new persistence-affecting state must survive store snapshot round-trips (libSQL filesystem store rebuilds from snapshot).
- Run `cargo fmt` + `cargo clippy --all --benches --tests --examples --all-features` (zero warnings) + `cargo test -p ironclaw_architecture` after composition/boundary changes.

---

## File Structure

| File | Responsibility | Stream |
|---|---|---|
| `crates/ironclaw_turns/src/memory.rs` | per-user running counter + claim-skip + limit field | A |
| `crates/ironclaw_turns/src/store.rs` (or limits home) | (only if limits type lives outside memory.rs) | A |
| `crates/ironclaw_reborn/src/runtime.rs` | build N workers; `worker_count` config field; composition holds `Vec` | B |
| `crates/ironclaw_reborn/src/turn_runner.rs` | (read-only reference; no logic change expected) | B |
| `crates/ironclaw_reborn_composition/src/runtime.rs` | spawn N tasks; `Vec<JoinHandle>`; shutdown sites; wire store limit | B (spawn) / C (limit wire) |
| `crates/ironclaw_reborn_composition/src/runtime_input.rs` | `TurnRunnerSettings` gains `worker_count` + `max_concurrent_runs_per_user` | C |
| `crates/ironclaw_reborn_config/src/config_file.rs` | `[runner]` TOML gains `worker_count`, `max_concurrent_runs_per_user` | C |
| `crates/ironclaw_reborn_cli/src/runtime/mod.rs` | `runner_settings()` reads new TOML fields into settings | C |

**Streams A and B touch disjoint crates and run in parallel. Stream C runs after A+B and wires config into the fields they exposed.**

---

## Interfaces (the contract between streams)

Stream A **produces**:
```rust
// crates/ironclaw_turns/src/memory.rs
pub struct InMemoryTurnStateStoreLimits {
    // ...existing fields unchanged...
    /// Max runs in `TurnStatus::Running` per (tenant_id, owner user_id).
    /// `None` = unlimited (current behavior). Owner-less / actor-fallback runs are never counted.
    pub max_concurrent_runs_per_user: Option<std::num::NonZeroU32>,
}
```
`InMemoryTurnStateStoreLimits::default()` sets the new field to `None`.

Stream B **produces**:
```rust
// crates/ironclaw_reborn/src/runtime.rs  (DefaultPlannedRuntimeConfig)
pub struct DefaultPlannedRuntimeConfig {
    pub worker: TurnRunnerWorkerConfig,
    pub worker_count: std::num::NonZeroUsize, // default 4
    pub text_only_driver: TextOnlyModelReplyDriverConfig,
    pub host: TextOnlyLoopHostConfig,
}
// RebornRuntimeLoopComposition.workers: Vec<Arc<TurnRunnerWorker>>  (was: worker: Arc<TurnRunnerWorker>)
```

Stream C **consumes** both: reads `[runner].worker_count` / `[runner].max_concurrent_runs_per_user` from TOML, threads into `DefaultPlannedRuntimeConfig.worker_count` and `InMemoryTurnStateStoreLimits.max_concurrent_runs_per_user` at their composition construction sites.

---

## Stream A — Per-user concurrency cap in the store

All changes in `crates/ironclaw_turns/`. Self-contained crate; test with `cargo test -p ironclaw_turns`.

### Task A1: Add the limit field

**Files:** Modify `crates/ironclaw_turns/src/memory.rs:42-59`

- [ ] **Step 1:** Add field to `InMemoryTurnStateStoreLimits`:
```rust
pub max_concurrent_runs_per_user: Option<std::num::NonZeroU32>,
```
- [ ] **Step 2:** In `Default`, add `max_concurrent_runs_per_user: None,`.
- [ ] **Step 3:** `cargo build -p ironclaw_turns` — expect compile error at any struct-literal construction of the limits (there should be none outside `Default`; if any exist, add the field). Fix until it builds.
- [ ] **Step 4:** Commit: `feat(turns): add max_concurrent_runs_per_user limit field`

### Task A2: Maintain a per-user running counter

The counter tracks runs currently in `TurnStatus::Running`, keyed by `(TenantId, UserId)`.

**Files:** Modify `crates/ironclaw_turns/src/memory.rs` (`Inner` struct ~line 74; `claim_next_run` ~1224; every site that transitions a record OUT of `Running`).

- [ ] **Step 1: Write failing test** in the `tests` module of `memory.rs`:
```rust
#[tokio::test]
async fn running_counter_tracks_per_user_across_lifecycle() {
    // Build a store, submit + claim a run owned by user U under tenant T.
    // assert store.debug_running_count(&tenant, &user) == 1 after claim.
    // Complete the run (terminal). assert count == 0.
    // (Use existing test helpers in this module to submit/claim/complete.)
}
```
Add a `#[cfg(test)]`-only accessor on `InMemoryTurnStateStore`:
```rust
#[cfg(test)]
pub(crate) fn debug_running_count(&self, tenant: &TenantId, user: &UserId) -> u32 {
    self.inner.lock().expect("lock").running_by_user
        .get(&(tenant.clone(), user.clone())).copied().unwrap_or(0)
}
```
- [ ] **Step 2:** Run `cargo test -p ironclaw_turns running_counter_tracks_per_user_across_lifecycle` — expect FAIL (field/method missing).
- [ ] **Step 3: Implement.**
  - Add to `Inner`: `running_by_user: HashMap<(TenantId, UserId), u32>,` (derive `Default` already present — `HashMap::default()` is empty).
  - Add a private helper on `Inner`:
```rust
fn run_user_key(scope: &TurnScope) -> Option<(TenantId, UserId)> {
    scope
        .thread_owner
        .explicit_owner_user_id()
        .map(|user| (scope.tenant_id.clone(), user.clone()))
}

fn increment_running(&mut self, scope: &TurnScope) {
    if let Some(key) = Self::run_user_key(scope) {
        *self.running_by_user.entry(key).or_insert(0) += 1;
    }
}

fn decrement_running(&mut self, scope: &TurnScope) {
    if let Some(key) = Self::run_user_key(scope) {
        if let Some(count) = self.running_by_user.get_mut(&key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.running_by_user.remove(&key);
            }
        }
    }
}
```
  - In `claim_next_run` (after `record.status = TurnStatus::Running;`, ~line 1234): call `inner.increment_running(&record.scope);`.
  - At EVERY site that moves a record from `Running` to a non-Running status, call `inner.decrement_running(&record.scope)` exactly once. Find them by grepping `crates/ironclaw_turns/src/memory.rs` for transitions: terminal application (`mark_terminal` / `apply_loop_transition` terminal arm), `block_run` (Running→Blocked), `recover_expired_leases` (Running→Queued/Failed on lease expiry), and any cancel-completion (CancelRequested→Cancelled) path that was Running. For each, add the decrement on the branch where the OLD status was `Running`.
- [ ] **Step 4:** Run the test — expect PASS.
- [ ] **Step 5: Add exhaustive lifecycle tests** (one each), asserting the counter returns to 0:
  - complete, fail, block-then-resume-then-complete (block decrements; resume re-claims and re-increments), cancel, lease-expiry recovery.
  Run `cargo test -p ironclaw_turns` — all PASS.
- [ ] **Step 6: Snapshot rebuild.** In `from_persistence_snapshot` (~line 1483) rebuild `running_by_user` by scanning restored records: for each record with `status == TurnStatus::Running`, apply `increment_running(&record.scope)`. Add a test that a snapshot containing a Running run restores `debug_running_count == 1`. Run, expect PASS.
- [ ] **Step 7: Commit:** `feat(turns): track per-user running-run counter`

### Task A3: Enforce the cap in claim selection

**Files:** Modify `crates/ironclaw_turns/src/memory.rs` (`pop_matching_queued_run` ~1887; `claim_next_run` ~1224).

- [ ] **Step 1: Write failing test:**
```rust
#[tokio::test]
async fn claim_skips_user_at_concurrency_cap() {
    // limits.max_concurrent_runs_per_user = NonZeroU32::new(1)
    // Submit two runs on DIFFERENT threads, both owned by user U / tenant T.
    // First claim -> returns run1 (now Running, U at cap 1).
    // Second claim (different runner_id/lease) -> returns None (U capped), run2 stays Queued.
    // Submit a run owned by user V -> claim returns it (V not capped).
    // Complete run1 -> claim now returns run2.
}
```
- [ ] **Step 2:** Run — expect FAIL (claim still returns run2).
- [ ] **Step 3: Implement.** `pop_matching_queued_run` already scans the `queued_runs` deque, cycling non-matching runs to the back. Extend the predicate: after the existing `scope_filter` match check passes, also check the per-user cap. Because `pop_matching_queued_run` is `&mut self` it can read `self.limits` and `self.running_by_user`:
```rust
// inside the loop, after fetching `record` for `run_id`, before returning Some(run_id):
let scope_ok = scope_filter.is_none_or(|scope| scope == &record.scope);
let user_ok = match self.limits.max_concurrent_runs_per_user {
    None => true,
    Some(cap) => match Inner::run_user_key(&record.scope) {
        None => true, // owner-less / actor-fallback runs are never capped
        Some(key) => self.running_by_user.get(&key).copied().unwrap_or(0) < cap.get(),
    },
};
if scope_ok && user_ok {
    return Some(run_id);
}
self.queued_runs.push_back(run_id);
```
Note: `record` here must be read without removing it from `records` (the existing code only borrows to read `record.scope`); keep that borrow shape. If the current code `take_record`s, mirror the existing return-to-queue handling so a skipped run is not mutated.
- [ ] **Step 4:** Run the test — expect PASS. Run full `cargo test -p ironclaw_turns` — all PASS (no regression in existing claim/FIFO tests).
- [ ] **Step 5: Commit:** `feat(turns): cap concurrent runs per user in claim selection`

### Task A4: Stream A quality gate

- [ ] `cargo fmt -p ironclaw_turns`
- [ ] `cargo clippy -p ironclaw_turns --all-features --tests` — zero warnings
- [ ] `cargo test -p ironclaw_turns` — green
- [ ] Commit any fmt/clippy fixups.

---

## Stream B — N-worker global pool

Changes in `crates/ironclaw_reborn/` + `crates/ironclaw_reborn_composition/` (spawn site only). Disjoint from Stream A.

### Task B1: Add `worker_count` to runtime config

**Files:** Modify `crates/ironclaw_reborn/src/runtime.rs:60-65`.

- [ ] **Step 1:** Change `DefaultPlannedRuntimeConfig` to add `pub worker_count: std::num::NonZeroUsize,`. It currently `#[derive(Default)]`; `NonZeroUsize` has no `Default`, so replace the derive with a hand-written `Default` that sets `worker_count` to `NonZeroUsize::new(4).expect("4 is non-zero")` and the other fields to their `Default`.
- [ ] **Step 2:** `cargo build -p ironclaw_reborn` — fix any struct-literal sites (e.g. `..DefaultPlannedRuntimeConfig::default()` usages keep working; explicit literals need the new field).
- [ ] **Step 3: Commit:** `feat(reborn): add worker_count to DefaultPlannedRuntimeConfig`

### Task B2: Build N distinct workers; composition holds a Vec

**Files:** Modify `crates/ironclaw_reborn/src/runtime.rs` (`RebornRuntimeLoopComposition` ~line 162; worker build ~line 542).

- [ ] **Step 1: Write failing test** in `crates/ironclaw_reborn/tests/loop_driver_host.rs` (or a new `tests/concurrent_workers.rs`) — a caller-level test that drives the runtime with a driver whose `run` blocks on a shared `tokio::sync::Barrier` of size 2, submits two runs on different threads (different `thread_id`, same or different owner under a cap that allows ≥2), and asserts BOTH reach `Running` (both hit the barrier) before either completes. With a single worker this deadlocks/ times out; with `worker_count = 2` it passes. Use the existing test harness builders in that file as the model (they already construct `TurnRunnerWorker` and spawn it).
- [ ] **Step 2:** Run — expect FAIL/timeout (only one worker, or composition exposes a single `worker`).
- [ ] **Step 3: Implement.**
  - Change `RebornRuntimeLoopComposition.worker: Arc<TurnRunnerWorker>` → `pub workers: Vec<Arc<TurnRunnerWorker>>`.
  - At the build site (~542), build `parts.config.worker_count.get()` workers in a loop. Each `TurnRunnerWorker::new(...)` already mints its own `runner_id`. Share by cloning the `Arc` deps (`Arc::clone(&transition_port)`, `Arc::clone(&loop_exit_applier)`, `Arc::clone(&driver_registry)`, `host_factory.clone()`) and the wake receiver (`wake_receiver.clone()` — `TurnRunnerWakeReceiver` is `Clone`). Collect into `workers: Vec<Arc<TurnRunnerWorker>>`. Return it in the composition.
- [ ] **Step 4:** Build `cargo build -p ironclaw_reborn`. Fix the now-broken `composition.worker` consumer in `ironclaw_reborn_composition` in Task B3 (build will fail there — expected; B3 fixes it).
- [ ] **Step 5: Commit:** `feat(reborn): build a pool of N turn-runner workers`

### Task B3: Spawn N tasks; `Vec<JoinHandle>`; shutdown sites

**Files:** Modify `crates/ironclaw_reborn_composition/src/runtime.rs` — spawn site ~2900-2905; `RebornRuntime.worker_handle` field ~408; usages at ~1474, ~1649, ~1705, ~1765, ~3612.

- [ ] **Step 1:** Change the `RebornRuntime` field `worker_handle: JoinHandle<()>` → `worker_handles: Vec<JoinHandle<()>>`.
- [ ] **Step 2:** At the spawn site, replace the single spawn with a loop over `composition.workers`, each spawned with a clone of `worker_cancel`:
```rust
let worker_cancel = CancellationToken::new();
let worker_handles: Vec<JoinHandle<()>> = composition
    .workers
    .iter()
    .map(|worker| {
        let worker = Arc::clone(worker);
        let cancel = worker_cancel.clone();
        tokio::spawn(async move { worker.run(cancel).await })
    })
    .collect();
```
Set `services.readiness.workers.turn_runner = !worker_handles.is_empty();` and store `worker_handles` in the struct literal (~2937).
- [ ] **Step 3:** Update the 5 usage sites:
  - `is_finished()` checks (~1474, ~1705, ~1765, ~3612): `self.worker_handles.iter().any(|h| h.is_finished())` (a finished worker = a crashed runner; preserve existing semantics — if the original treated finished-as-degraded, keep that meaning).
  - `.await` shutdown join (~1649): await all handles, e.g.
    ```rust
    for handle in self.worker_handles.drain(..) {
        if let Err(error) = handle.await {
            debug!(%error, "turn runner worker task join failed");
        }
    }
    ```
    (match the existing error handling/log level at that site).
- [ ] **Step 4:** `cargo build -p ironclaw_reborn_composition`. Then run the Task B2 test with `worker_count = 2` via the test harness — expect PASS.
- [ ] **Step 5:** `cargo test -p ironclaw_reborn` and `cargo test -p ironclaw_reborn_composition` (at least the turn-runner/runtime tests) — green.
- [ ] **Step 6: Commit:** `feat(reborn): spawn N concurrent turn-runner worker tasks`

### Task B4: Stream B quality gate

- [ ] `cargo fmt -p ironclaw_reborn -p ironclaw_reborn_composition`
- [ ] `cargo clippy -p ironclaw_reborn -p ironclaw_reborn_composition --all-features --tests` — zero warnings
- [ ] Commit fixups.

---

## Stream C — Config plumbing (runs AFTER A + B)

Wire both TOML knobs into the fields A and B exposed.

### Task C1: TOML fields

**Files:** Modify `crates/ironclaw_reborn_config/src/config_file.rs:154-157` (the `[runner]` struct).

- [ ] **Step 1:** Add two optional fields to the runner config struct:
```rust
pub worker_count: Option<usize>,
pub max_concurrent_runs_per_user: Option<u32>,
```
(Keep `Option` so absent TOML preserves defaults; serde already `#[serde(default)]`-style for this section — match the existing field attributes.)
- [ ] **Step 2: Test** — add a config-file parse test (mirror existing runner-section tests in this crate) asserting a TOML with `worker_count = 3` and `max_concurrent_runs_per_user = 2` round-trips into the struct, and that an absent `[runner]` leaves both `None`.
- [ ] **Step 3:** `cargo test -p ironclaw_reborn_config` — PASS.
- [ ] **Step 4: Commit:** `feat(reborn-config): parse worker_count + max_concurrent_runs_per_user`

### Task C2: Thread settings through `TurnRunnerSettings`

**Files:** Modify `crates/ironclaw_reborn_composition/src/runtime_input.rs` (`TurnRunnerSettings` ~298) and `crates/ironclaw_reborn_cli/src/runtime/mod.rs` (`runner_settings()` ~753-774).

- [ ] **Step 1:** Add to `TurnRunnerSettings`: `pub worker_count: std::num::NonZeroUsize,` and `pub max_concurrent_runs_per_user: Option<std::num::NonZeroU32>,`. Give the struct's constructor/default `worker_count = NonZeroUsize::new(4)` and cap `None`.
- [ ] **Step 2:** In `runner_settings()`, map the TOML `Option<usize>` → `NonZeroUsize` (treat `Some(0)` or absent as the default 4; clamp a sane max, e.g. 32, with a `debug!` if clamped) and `Option<u32>` → `Option<NonZeroU32>` (treat `Some(0)` as `None` = unlimited).
- [ ] **Step 3:** Test `runner_settings()` mapping (0 → default/unlimited, present → value, clamp).
- [ ] **Step 4: Commit:** `feat(reborn-cli): map runner TOML into TurnRunnerSettings`

### Task C3: Wire into the two construction sites

**Files:** Modify `crates/ironclaw_reborn_composition/src/runtime.rs` — `DefaultPlannedRuntimeConfig` construction (~2661) and the `InMemoryTurnStateStore` / limits construction site (grep for `InMemoryTurnStateStoreLimits` or `with_limits` in this crate).

- [ ] **Step 1:** At the `DefaultPlannedRuntimeConfig` construction, set `worker_count: runner.worker_count,`.
- [ ] **Step 2:** Find where the runtime builds its `InMemoryTurnStateStoreLimits` (or default store) and set `max_concurrent_runs_per_user: runner.max_concurrent_runs_per_user,`. If the runtime currently uses `InMemoryTurnStateStore::default()` with no explicit limits, switch to `with_limits(InMemoryTurnStateStoreLimits { max_concurrent_runs_per_user: ..., ..Default::default() })`. (For the libSQL/filesystem store, locate the equivalent limits pass-through; the per-user counter rebuilds from snapshot per Task A2 Step 6, so only the limit value must be threaded.)
- [ ] **Step 3:** Build the whole workspace `cargo build`. Run a caller-level integration test (in `ironclaw_reborn_composition/tests` or `ironclaw_reborn/tests`) that sets `worker_count=3`, `max_concurrent_runs_per_user=1`, submits 3 runs for one user on 3 threads + 1 run for a second user, and asserts: at most 1 of user-A's runs runs concurrently while user-B's run proceeds. (Barrier-driver pattern from B2.)
- [ ] **Step 4: Commit:** `feat(reborn): wire runner concurrency config into runtime composition`

### Task C4: Full quality gate

- [ ] `cargo fmt`
- [ ] `cargo clippy --all --benches --tests --examples --all-features` — zero warnings
- [ ] `cargo test` (and `cargo test -p ironclaw_architecture` for boundary checks)
- [ ] Update `.env.example` / config docs if the repo documents `[runner]` TOML keys elsewhere (grep `heartbeat_interval_secs` to find doc sites; add the two new keys with a one-line comment each).
- [ ] Commit: `docs(reborn): document runner concurrency config keys`

---

## Self-Review checklist (run before opening PR)

1. **Serialization fixed:** integration test proves ≥2 runs execute concurrently (B2/C3).
2. **Per-user cap:** test proves one user is capped while another proceeds (A3/C3).
3. **No double-claim:** existing `ironclaw_turns` claim tests still green (atomic claim preserved).
4. **Snapshot safety:** `running_by_user` rebuilds from snapshot (A2 Step 6).
5. **Defaults preserve behavior path:** absent TOML → `worker_count=4`, cap `None`. (If a conservative rollout is wanted, default `worker_count=1` — confirm with maintainer; plan default is 4 per design decision.)
6. **No placeholders / TODO left.** Decrement sites all covered (A2 Step 5 lifecycle tests).

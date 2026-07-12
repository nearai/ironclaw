# P1 Reborn Runtime Wedge — Implementation Plan

**Date:** 2026-06-24
**Branch:** `fix/reborn-p1-runtime-wedge` (base: main tip `f08f09209`)
**Worktree:** `/Users/henry/Code/ironclaw-wt-p1`
**Source triage:** `docs/plans/2026-06-24-reborn-runtime-wedge-triage.md`
**Companion (canonical fix design):** `docs/plans/2026-06-22-turn-state-lock-wedge-fix.md`

This plan covers the two P1 *amplifiers* that turned a slow provider call into a
total runtime freeze: worker-pool starvation (Wedge 2) and the redundant
in-process serializer / lock convoy + secrets map leak (Wedge 3 + leak). Both
are Reborn-only. Wedge 1 (provider hang) and the primary-call timeout gap are
NOT in this track.

---

## Re-verification (done before planning — do not trust the triage blindly)

### Incident logs
- `~/Downloads/logs.1782330623559.log` + `logs.1782331100360.log` (1000 lines each).
- **Total silence window confirmed:** 82 log lines at `19:51:05`, then *nothing*
  `19:51:06 → 19:54:58`, resuming `19:54:59` with `ironclaw_runner::runtime:
  loaded boot config` — i.e. the process **died and restarted** after the ~4-min
  wedge. The slice captured here shows post-restart lease recovery (8
  `lease_expired`); the mass-expiry burst lives in the wider prod log set the
  triage cites, but the silence-then-restart signature is firmly reproduced.
- Burst confirmed: multiple concurrent `turn_run{...}` model completions at
  `19:51:05` (deepseek-ai/DeepSeek-V4-Flash), consistent with the ~40-turn
  fan-out.

### PR A claims — ALL VALID
| Claim | Verdict | Evidence |
|---|---|---|
| `runtime.execute` is sync `pub fn`, called from `async fn` with no `spawn_blocking` | VALID | `ironclaw_host_runtime/src/services/runtime_adapters.rs:619` (`execute_prepared_wasm`, plain `fn` :591) ← `WasmRuntimeAdapter::dispatch_json` (`async fn` :525). `WitToolRuntime::execute` is sync (`ironclaw_wasm/src/runtime.rs:58`). |
| `host.rs` uses `block_in_place(\|\| handle.block_on(..))` on MultiThread | VALID | `ironclaw_wasm/src/host.rs:392`, in `block_on_runtime_http_egress`. A side-runtime alternative already exists: `run_runtime_http_egress_on_worker` (:401) over `WASM_HTTP_EGRESS_RUNTIME` (LazyLock 1-thread rt, :18). |
| `wasm_credentials.rs` same pattern | VALID | `ironclaw_host_runtime/src/wasm_credentials.rs:292`; side-runtime `run_credential_restage_on_worker` (:299) over `WASM_CREDENTIAL_RESTAGE_RUNTIME` (:324). |
| Legacy uses `spawn_blocking` (correct pattern to mirror) | VALID | `src/tools/wasm/wrapper.rs:1357` moves fully-owned `wrapper` into `spawn_blocking`. |
| Reached per-turn on production hot path | VALID (codegraph) | `CapabilityDispatcher::dispatch_json` → `dyn RuntimeAdapter::dispatch_json` → `WasmRuntimeAdapter::dispatch_json` → `execute_prepared_wasm` → `runtime.execute`. |
| `WitToolRuntime` / host / `PreparedWitTool` are `Send + 'static`-cloneable for `spawn_blocking` | VALID (nuance) | `Engine` is `Send+Sync`; `WitToolHost: Clone+Send`; `PreparedWitTool` already `Arc`-wrapped at call sites. **Adapter holds `runtime: WitToolRuntime` by value**, so `execute_prepared_wasm` only has `&WitToolRuntime` — the spawn_blocking port must move an *owned* `WitToolRuntime` (it is `Clone`) or hold `Arc<WitToolRuntime>`. |

### PR B claims — VALID in substance, with a precision correction
The triage calls these "lock held across `.await`" bugs. **Precision:** every one
of these locks is a `tokio::sync::Mutex` (not `std::sync::Mutex`), and holding a
*tokio* async mutex guard across `.await` is the intended use of that type — it
does NOT block an OS thread. So the narrow "lock-across-await is UB" reading is
*not* the bug. The **substantive** problem (documented at length in
`2026-06-22-turn-state-lock-wedge-fix.md`) is real:

- These per-path/per-key `tokio::Mutex`es are **redundant in-process serializers**
  over backends (`LibSql`/`InMemory`) that already do real versioned CAS, so the
  lock adds no correctness — only a wedge surface. One writer that stalls inside
  its read-modify-write **freezes every other writer for that scope** (claim,
  heartbeat, complete, lease recovery, new submit). Latent since #3679, weaponized
  by #5085's concurrent scheduler.
- #5142 already fixed exactly one of the five copies (`ironclaw_turns`) by
  **removing the mutex** and relying on bounded-CAS-retry + a 15s timeout. The
  prescribed canonical fix is to extract that pattern into one shared helper and
  delete the remaining 4 copies — NOT to patch each in place.

| Claim | Verdict | Evidence |
|---|---|---|
| `ironclaw_turns` already mutex-free via CAS+15s timeout (the reference) | VALID | `ironclaw_turns/src/filesystem_store.rs:65-66,248-309` (own `put_with_cas`/`cas_retry_backoff`/`PutError` — currently a *local* copy, not yet shared). |
| `run_state` per-scope `tokio::Mutex` guard across `apply_update`/`put_with_cas` awaits; map uses `Weak`+prune | VALID | `ironclaw_run_state/src/lib.rs` guards 645/680/697/713/730 + approval store 880-957; `Weak` map + `retain` at :1104/:1116. Has CAS retry (8) already. |
| `threads` `ensure_thread` holds guard across `read_thread_versioned`+`put_with_cas`; `Weak`+prune | VALID | `ironclaw_threads/src/filesystem_service.rs:1066` guard; awaits 1067-1115; `Weak` map :2693, `retain` :2701. `ensure_thread` reconciles VersionMismatch once (no loop); other write paths already lock-free w/ CAS retry. |
| `resources` `update_with_scope` holds guard across `update_snapshot().await`; **NO retry loop**; `Weak`+prune | VALID | `ironclaw_resources/src/cas_snapshot.rs:160-161`; `update_snapshot` single attempt, returns `E::storage(...)` on VersionMismatch (:201-207). `Weak` map :353-355, `retain` :364. |
| `secrets` holds guard across `cas_mutate().await`; map uses `Arc` (NOT Weak), never pruned → **unbounded leak** | VALID | `ironclaw_secrets/src/filesystem_store.rs` guards 463/538/881/909; **leak**: `HashMap<String, Arc<tokio::Mutex<()>>>` at :1122-1133, no `Weak`, no `retain` — every distinct lease-UUID / session key permanently retained. `cas_mutate` already has a 3-attempt retry. |
| All target state mounts are CAS-capable (gate won't reject prod) | VALID | per `2026-06-22-...:§6`: `/turns` + sibling state roots resolve under `/tenants/...` → LibSql/InMemory (real CAS). Byte-only `LocalFilesystem` backs only `/workspace`+`/projects`, never state stores. `BackendCapabilities` + `TxnCapability::CompareAndSwap` already exist (`ironclaw_filesystem/src/types.rs:362`, `RootFilesystem::capabilities()` `root.rs:43`). |

**Nothing was found invalid.** The only adjustment is the framing of PR B (it is a
redundant-serializer / wedge-surface removal, not a use-after-free-style "across
await" bug), which aligns the implementation with the canonical #5142 approach.

---

## PR granularity decision

**Two PRs, stacked.**

- **PR A — `fix/reborn-p1-runtime-wedge`** (this branch): WASM worker-pool
  starvation. Self-contained, low-risk, high-value, ships first.
- **PR B — `fix/reborn-p1-cas-helper`** (branch off PR A): the canonical
  `cas_update` helper in `ironclaw_filesystem`, migrate all five stores
  (turns/run_state/threads/resources/secrets) onto it, delete the five
  `FILESYSTEM_RECORD_LOCKS` copies, and fix the secrets `Arc`→(removed) leak.

**Rationale.**
1. PR A is orthogonal to PR B (different crates, different mechanism) and removes
   the worst amplifier (a burst ≥ worker_count freezing the scheduler). It is
   safe to ship alone.
2. PR B is a concurrency-sensitive refactor across five crates. The #5142 plan
   (§7) is explicit that the correct move is **one shared helper, delete the 5
   copies** — *not* copy-paste the retry loop into each. Doing it as a focused,
   separately-reviewed PR (Concurrency review is the #1 priority) is materially
   safer than bundling it with PR A.
3. Stacking PR B on PR A keeps PR A independently mergeable; if PR B needs more
   review iterations it does not hold up the high-value worker-pool fix.

This matches the instruction: ship PR A first; do PR B as a precise planned
follow-up rather than rushing a risky concurrency change.

---

## PR A — De-wedge the WASM worker pool

Crates: `ironclaw_host_runtime`, `ironclaw_wasm`.

### A1. Offload sync WASM execution to a blocking thread (`spawn_blocking`)
`runtime_adapters.rs::execute_prepared_wasm` is the per-turn hot path that runs
the sync wasmtime call on a tokio worker. Mirror legacy `wrapper.rs:1357`:
- Make `WasmRuntimeAdapter` hold `runtime: Arc<WitToolRuntime>` (or clone an owned
  `WitToolRuntime` into the closure — `WitToolRuntime: Clone`). Move the owned
  runtime + `Arc<PreparedWitTool>` + owned host/args into
  `tokio::task::spawn_blocking(move || runtime.execute(...))` and `.await` it.
- Map `JoinError` (panic / cancellation) to the existing WASM execution error
  variant, mirroring legacy `WasmError::ExecutionPanicked`.
- Keep the function signature/behavior otherwise identical (same inputs/outputs).

### A2. Replace `block_in_place` with the existing side-runtime path
Both `host.rs:392` and `wasm_credentials.rs:292` already have a dedicated
1-thread side runtime (`run_*_on_worker`) used for the single-thread flavor.
Route the MultiThread branch through the same side-runtime helper and delete the
`block_in_place` branch, so WASM host egress/credential restage never blocks a
turn worker thread. (Verify the side-runtime helper is `Send` over the future;
the egress/restage futures already are — they run on the side rt in the
single-thread case today.)

### A3. Bound concurrent WASM execution with a semaphore
Add a process-wide (or per-runtime) `tokio::sync::Semaphore` acquired before the
`spawn_blocking` in A1, sized to leave headroom under the blocking pool /
worker_count. This caps how many turns can be inside native WASM execution at
once, so a burst can never exhaust the runtime. Default bound: a named const
(e.g. `DEFAULT_MAX_CONCURRENT_WASM_EXEC`) — choose a value comfortably above
normal steady-state but below the blocking-pool ceiling; document the reasoning
inline. Acquire-with-timeout is unnecessary (blocking pool is large), but the
permit must be released on all paths (RAII `OwnedSemaphorePermit` moved into the
spawned task).

> Note: PR A intentionally does NOT touch the primary-assistant-call timeout
> (Phase 1c in the triage) — that is Wedge-1-adjacent and out of this track.

### PR A tests
- `ironclaw_host_runtime`: a test that drives `dispatch_json` (the caller, per the
  testing rule) and asserts WASM execution completes off the worker thread and the
  semaphore bounds concurrency (e.g. N+1 concurrent calls serialize at the bound,
  none error). Use an in-process fake WASM tool / minimal module.
- `ironclaw_wasm` / `ironclaw_host_runtime`: assert the egress + credential
  restage paths run via the side runtime under a MultiThread runtime (no
  `block_in_place`); a unit test invoking the host egress under
  `#[tokio::test(flavor = "multi_thread")]` and asserting it returns without
  parking is sufficient.

---

## PR B — Canonical CAS helper; delete the five lock copies + secrets leak

Crates: `ironclaw_filesystem` (new helper), `ironclaw_turns`, `ironclaw_run_state`,
`ironclaw_threads`, `ironclaw_resources`, `ironclaw_secrets`.

### B1. `cas_update` helper in `ironclaw_filesystem`
One generic, mutex-free, bounded-CAS-retry helper that owns the
read-modify-write contract. Port verbatim semantics from
`ironclaw_turns/src/filesystem_store.rs` (the proven #5142 implementation):
- `FILESYSTEM_CAS_RETRIES = 32`, jittered exponential backoff
  (`FILESYSTEM_CAS_BACKOFF_BASE=2ms`, `MAX=50ms`), wrapped in a
  `tokio::time::timeout(15s)` (defense-in-depth — §7c).
- Signature (sketch):
  `pub async fn cas_update<F, T, E, Apply, Fut>(fs, scope, path, decode, encode, apply, deadline) -> Result<T, E>`
  where `apply: FnMut(Snapshot) -> Fut`, returning `(outcome, new_snapshot)`;
  re-reads + reapplies on `VersionMismatch`; no per-record mutex; holds no lock
  across the awaited backend I/O.
- **Capability gate (§7a.2):** assert the mount's
  `BackendCapabilities`/`TxnCapability::CompareAndSwap`; fail closed if the
  backend cannot CAS (so a byte-only `LocalFilesystem` mount cannot silently
  degrade to blind-overwrite). This centralizes the §6/§8 reasoning in one place.
- Keep the `PutError { VersionMismatch, Other(E) }` classification internal;
  expose a typed error mapping so each store can map to its own error enum.

Design judgment: the helper must NOT leak store-specific types. It is generic over
the snapshot/record type, the caller's error type (via an `E: From`/mapper), and
the scope. Re-home the turns copy onto it too (so there is exactly one
implementation, per the thermo-nuclear "one owner per concern" bar).

### B2. Migrate the five stores; delete every `FILESYSTEM_RECORD_LOCKS`
- `ironclaw_turns`: replace its local `apply`/`put_with_cas`/`cas_retry_backoff`
  with calls into the shared helper (keep the 500ms read cache — that is a
  separate, benign `std::Mutex` snapshot cache, no await inside).
- `ironclaw_run_state`: route all 5 write sites + the approval store through the
  helper; delete `filesystem_record_lock` + the static.
- `ironclaw_threads`: route `ensure_thread` through the helper (it gains a proper
  retry loop, replacing the single reconcile); delete the static.
- `ironclaw_resources`: route `update_with_scope`/`update_snapshot` through the
  helper — **this is the one that gains a retry loop it never had**; delete the
  static.
- `ironclaw_secrets`: route `consume`/`revoke`/`validate_session`/
  `consume_session_use` through the helper; delete the `Arc` lock map entirely —
  **the unbounded leak disappears by deletion, not by switching to `Weak`.**

Post-condition (verification gate §8.3): `grep -rn FILESYSTEM_RECORD_LOCKS crates/`
returns empty, and no `record_lock.lock().await` remains in the five files.

### PR B tests (mostly at the helper; thin caller tests per the testing rule)
- Helper (`ironclaw_filesystem`): high-contention CAS storm (N concurrent writers,
  same path, all succeed, no lost update, no spurious exhaustion); a stalled
  backend write does not block other-path writers (the wedge regression);
  capability gate rejects a byte-only mount; deterministic jittered backoff.
- Caller adoption tests (drive through the public method, integration tier where
  the store has integration coverage): turns (`submit_turn`/`complete`),
  run_state (block/complete), threads (`ensure_thread`), resources
  (`update_with_scope`), secrets (`consume`/`revoke` — plus an assertion that no
  unbounded per-key state accumulates).
- `ironclaw_host_runtime` scheduler: a writer that stalls inside persistence must
  not block its own heartbeat nor other workers' claims for the same user
  (self-deadlock regression — falls out of removing the mutex).

---

## Quality gate (both PRs)
From the worktree:
```
cargo fmt
cargo clippy --all --benches --tests --examples --all-features   # zero warnings
cargo test -p <touched crates>            # + --features integration on persistence paths
```
Capture real output as evidence; no green claims without it.

## Out of scope (this track)
- Wedge 1 (NEAR AI connect/total timeouts) — shared crate, separate track.
- Primary-assistant-call `tokio::time::timeout` (triage Phase 1c) — Wedge-1-adjacent.
- Trigger-storm load amplifier, alerting/guardrails (triage Phase 3).

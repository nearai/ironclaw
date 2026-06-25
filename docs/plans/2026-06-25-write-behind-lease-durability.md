# Fusion Design — Write-Behind Lease Durability + Side-Effect Gate

**Status:** Accepted (council signoff — anthropic-slot Opus 4.8: ACCEPT; openai-slot GPT 5.5 XHigh: ACCEPT_WITH_NONBLOCKING_NOTES). No unresolved blockers.
**Origin:** fusion-design-council (Opus 4.8 + GPT 5.5 XHigh), 1 draft round + 1 cross-review round + signoff.
**Scope:** Reborn turn-runner lease persistence; generalizable async-durability layer.

## 1. Problem statement
On the deployed Postgres/Supabase backend, runner heartbeats are a durable per-run CAS write (`/turns/runner-leases/<run_id>`, post-#5232). Under Postgres per-write latency / pool pressure / cross-region RTT, the heartbeat is delayed or fails, and the scheduler **insta-fails the run on a single failed heartbeat** (`scheduler_heartbeat_failed`, turn_scheduler.rs:673). Result: "constantly getting lease expired and many other problems." Local libSQL (in-memory, instant writes) never reproduces it. We want the heartbeat hot path independent of per-write Postgres latency **without** weakening cross-process/replica crash recovery — including the hard case of a network partition where a stale owner could keep producing real side effects.

## 2. Goals / Non-goals
**Goals:** (G1) heartbeat hot path independent of per-write Postgres latency; (G2) no run killed by a single transient durable-write failure; (G3) provably-bounded recovery staleness with NO false reclaim of a live run and NO double side-effects under partition; (G4) explicit cache-eligibility rules + a reusable system-wide layer; (G5) reversible, feature-flagged rollout safe on both backends.
**Non-goals:** replacing Postgres; full event-store/schema redesign; Redis as a requirement; changing the 90/30/10 constants (kept).

## 3. Constraints / Assumptions
- DB is source of truth; the write-behind buffer is a per-replica **cache**, never a second source of truth.
- Dual-backend: identical caller code for Postgres and libSQL; no per-backend `#[cfg]` in callers; the wrapper sits above the `Store`/`RootFilesystem` trait.
- Run ownership is exclusive via lease; the in-memory cache is per-replica; other replicas' recovery pollers read ONLY the durable store.
- Model-call-timeout < lease-TTL invariant (#5204) preserved.
- Named timing variables: `T_lease=90s`, `T_hb=30s`, `T_poll=10s`, `T_flush=5s`, `T_write=5s` (per-write timeout), `D_stop=45s` (max durable-lag before self-stop), `S=max_clock_skew` (configured constant). **`S < 15s` is the load-bearing inequality** (`D_stop + S < 60`); enable is gated on a real Railway↔Supabase measurement and **hard-fails** if violated.

## 4. Final design

### 4.1 Two durability planes (the hard rule)
- **SyncCritical** — synchronously durable before the caller observes success / before any externally-visible side effect: `claim`, ownership transfer, `complete`, `fail`, `block`/`unblock`, scheduling-gating state snapshots, **idempotency keys**, **tool side-effect commit records**.
- **AsyncLossTolerantCoalesced** — may be cached, coalesced, lost on crash: **runner-lease heartbeats**, progress timestamps, non-authoritative counters, derived indexes, cacheable summaries.
- **AsyncBestEffortTelemetry** — droppable under pressure.

**Eligibility test:** cacheable iff (a) idempotent under monotonic-max / last-writer, (b) safely droppable/reconstructable on crash, (c) its loss moves toward a *safe* state (more-eager recovery). Anything that gates a side effect, transfers ownership, advances the state machine, or whose loss/reorder could duplicate a side effect or silently lose work is **SyncCritical**. Enforced by a `DurableWrite` newtype + a `scripts/pre-commit-safety.sh` grep flagging cache/write-behind use on transition paths.

### 4.2 Liveness plane: write-behind lease cache + drain (STARTER)
`LeaseWriteBehind` — per-replica `Arc` singleton:
- `dirty: std::sync::Mutex<HashMap<RunId, LeaseIntent>>`, `LeaseIntent{ expires_at, fence, last_durable_flush_succeeded_at, last_flushed_expiry, enqueued_at, consecutive_flush_failures }`.
- **Hot path `renew(run, fence, now)`** (30s heartbeat tick): `want = now + T_lease`; lock std mutex (NO `.await`, NO I/O); `if fence >= e.fence { e.fence = fence; e.expires_at = e.expires_at.max(want) }`. Non-blocking; cannot fail from Postgres latency. **The heartbeat tick no longer does a durable write → the single-heartbeat insta-fail is removed** (G1, G2).
- **Drain task** (one supervised `tokio::spawn` per replica): every `T_flush`, snapshot+coalesce (one row per run; **skip CAS if `expires_at == last_flushed_expiry`**), write via the existing `put_with_cas` sidecar with bounded concurrency; success → record `last_durable_flush_succeeded_at` + `last_flushed_expiry`; transient failure → keep dirty + bump `consecutive_flush_failures`; **CAS/fence conflict → mark reclaimed → cancel the local executor**. Durable write rate falls to ≤ 1 / `T_hb` per run (write *less*, not just *async*).

### 4.3 The side-effect gate (load-bearing correctness mechanism)
Reactive fencing alone is insufficient: between flush attempts a partitioned owner can keep dispatching real side effects until another replica reclaims → double execution. Bound exposure with two purely-local (no Postgres round-trip) checks:

1. **Per-dispatch freshness gate (primary).** Synchronously, **immediately before each side-effecting tool/model call**: `remaining = last_flushed_expiry − now`; require `remaining > expected_op_duration + cancel_grace + S`. Else attempt **one synchronous flush**; if still no runway → **abort/pause the run** (`lease_degraded`). Evaluated **on the dispatch path itself**, off the 30s/5s cadence. **Uses the DURABLE `last_flushed_expiry`, never the in-memory `expires_at`** — a `renew` that never flushed grants NO runway.
2. **Run-level voluntary self-stop (backstop).** If `now − last_durable_flush_succeeded_at > D_stop (45s)`, cancel the run. `D_stop=45 < 60` (heartbeat→durable-expiry margin) leaves a `15s − S` buffer to stop **before** any other replica could legitimately reclaim.

A **back-pressure rejection counts as "no durable progress"** feeding the same `D_stop`/gate logic — never swallowed as a transient error.

### 4.4 Reactive fence on ALL ownership-sensitive writes (second line)
A monotonic `fence` (generation) per run persisted on the lease sidecar AND validated on **every** correctness write — `complete`, `fail`, `block`, **tool-result/side-effect commit**, and recovery `reclaim` — not only the heartbeat CAS. A reclaim bumps/tombstones the fence; any stale-fence write fails CAS. **Idempotency records (SyncCritical) are the third line.**

### 4.5 Bounds (explicit, named variables)
- **No false reclaim of a live run:** owner self-stops at `D_stop=45s`; durable lease valid `≥ 60s` past the last flushed heartbeat; `45 + S < 60` ⇒ owner stops before reclaim is possible (requires `S < 15s`).
- **Dead-run strand bound:** `≤ T_lease + T_poll + S` (≈105s, S=5). Crash loses pending heartbeats ⇒ *earlier* (safe) reclaim. The flush interval is additive only if a fresh lease was flushed immediately before death — **the worst case includes that additive term** and must still be `≤ T_lease + T_poll + S`.
- **Double-side-effect window:** bounded by `expected_op_duration + cancel_grace` of one in-flight op; the gate refuses to *start* an op without runway.

### 4.6 Dual-backend
`LeaseWriteBehind` sits above `Store`; libSQL receives identical `put_with_cas` calls, flushes in microseconds (indistinguishable from synchronous), and exercises the same drain machinery for test fidelity. Optional `T_flush=0` ⇒ synchronous passthrough for local.

## 5. Key decisions & alternatives rejected
- **D1** Two-plane split + eligibility test + `DurableWrite` newtype guardrail.
- **D2** Remove single-heartbeat insta-fail; executor aborts only on reclaim (fence conflict), self-stop (`D_stop`), or per-dispatch gate failure.
- **D3** **Side-effect gate pinned to the dispatch boundary + run-level `D_stop` self-stop** — the central decision (GPT's block basis; Opus required-change #1).
- **D4** Fence validated on ALL correctness writes, not just heartbeat CAS.
- **D5** Coalesce + skip-if-unchanged (write less).
- **D6** Fail-budget breach → local `lease_degraded` + stop side effects, NOT just metrics.
- **D7** Graceful-shutdown bounded `final_flush` (soonest-expiring first) to avoid deploy reclaim storms.
- **D8** Dark-mode rollout + tiered alerts (25/35/45s) + single feature flag governs write-behind + insta-fail removal together; reversible per-deploy env flip.
- **Rejected:** Redis primary (infra + second source of truth; option only); tune-constants-only (ignores root cause); retry-harder (more pool pressure); WAL/local-disk buffer (second source of truth, gone on container recycle); dedicated lease table + `FOR UPDATE SKIP LOCKED` (schema redesign non-goal; future ADR); global shared flush queue (serializes; unnecessary cross-run ordering); fire-and-forget without a deadline (false reclaim / double side-effects under slowness).

## 6. Implications (data / runtime / security)
- **Data:** lease sidecar must carry `fence` (generation) + `runner_id` durably. **Pre-enable decision, MUST be explicit in the implementation ADR (both slots flagged — do not leave implicit):** persist `fence` on the lease record; if a format bump is non-trivial, ship `runner_id`-only CAS as the committed fallback (a reclaim always rotates `runner_id`). Commit to one before enable.
- **Runtime:** one extra supervised drain task per replica; hot path becomes a mutex-guarded map insert; tool-dispatch path gains a local timestamp comparison.
- **Security/correctness:** idempotency records and tool side-effect commits are SyncCritical, bounding duplicate external effects even in the worst case.

## 7. Test & validation plan
Driven through the **scheduler/dispatch path** (per repo testing rule), via a `LatencyFaultStore` decorator (injectable fixed/p99 latency, transient `Err`, hard timeout, tombstone-from-other-replica):
- **Unit:** `renew` monotonicity + stale-fence ignore; coalesce K→1, unchanged→0 CAS; back-pressure rejection feeds the deadline (not swallowed).
- **T1** hot-path independence (2s/write latency ⇒ heartbeat tick <1ms, run not failed).
- **T2** no single-failure kill (one transient `Err` ⇒ run survives).
- **T3** live-run false-reclaim boundary (sustained failure ⇒ owner self-stops at `D_stop` BEFORE any reclaim; pin the tick).
- **T4** **double-side-effect / partition test (key):** partition owner from store, keep it "executing"; assert the per-dispatch gate refuses new side-effecting calls once runway < op+grace+S, the reclaiming replica only runs after the owner stopped, and external side-effect (outbox/idempotency) count == 1.
- **T5** dead-run strand bound `≤ T_lease + T_poll + S`, never before TTL — **assert the worst case that includes the additive fresh-flush-before-death term**, not just steady state.
- **T6** revived-lease: A stalls, B reclaims+bumps fence, A's late flush CAS fails ⇒ A aborts; no double completion.
- **T7** graceful-shutdown final flush within deadline; no spurious post-restart reclaim.
- **T8** dual-backend parity (T1–T7 on both libSQL + Postgres; identical decisions).
- **T9** flag-off legacy parity (forced write failure still fails the run — we didn't silently change legacy).
- **Chaos/staging:** N concurrent runs + injected Supabase RTT + pool exhaustion; metrics: heartbeat-tick p99 (flat <5ms), durable-flush-lag p50/95/99, false-reclaim (0), voluntary-stops, lease write QPS (large drop vs baseline). Compare to the "constantly lease expired" baseline.

## 8. Rollout / migration
1. Land behind `LEASE_WRITE_BEHIND_ENABLED=false`; CI runs full matrix (both flag states × both backends).
2. **Measure** Railway↔Supabase clock skew + lease-write p99/p999. **HARD-FAIL the rollout (do not warn-and-continue) if measured `skew + margin` violates `D_stop + S < 60` (i.e. `S ≥ 15s`).**
3. **Dark mode** in deployed staging: cache+drain run, emit lag/latency/QPS metrics, sync writes stay authoritative. 24–48h soak.
4. Flip behavior on one **canary** replica (env flag is per-process); compare lease-expired rate vs control.
5. Fleet-enable; keep flag one release; then delete the legacy synchronous insta-fail path + flag (cleanup ADR).
6. **Rollback:** flip env flag → instant revert; no migration, no schema change.

## 9. Risks & mitigations
- **Partition double side-effect** → per-dispatch gate (D3) + fence-everywhere (D4) + idempotency SyncCritical; T4 covers it.
- **Clock skew invalidates bounds** → `S` named configured constant; enable hard-gated on measurement; bounds `≤ T_lease+T_poll+S` and `D_stop+S<60`.
- **Fence/`claim_epoch` not durable** → resolved pre-enable (§6), explicit in ADR.
- **Drain dies silently** → supervised restart + `lease_drain_alive` metric; worst case == replica crash (safe reclaim).
- **Back-pressure silent liveness hole** → rejection counts as no-durable-progress feeding `D_stop`.
- **Graceful flush exceeds deploy deadline** → bounded `final_flush`, soonest-expiring first; residual reclaim accepted.
- **Feature-flag drift** → single flag governs both behaviors; T9 asserts legacy parity.

## 10. Agreement ledger
| Decision | anthropic-slot (Opus 4.8) | openai-slot (GPT 5.5 XHigh) |
|---|---|---|
| Liveness/correctness split + DurabilityClass taxonomy + newtype guardrail | ACCEPT | ACCEPT |
| Per-run coalescing cache + 5s drain + skip-if-unchanged + remove insta-fail | ACCEPT | ACCEPT |
| **Side-effect gate at dispatch boundary + run-level `D_stop`** | required-change #1 → ACCEPT | required (BLOCK basis) → ACCEPT |
| Fence on ALL correctness writes | ACCEPT (adopted) | required → ACCEPT |
| Fail-budget → `lease_degraded` stop (not just metrics) | ACCEPT (adopted) | required → ACCEPT |
| Dark-mode rollout + tiered alerts + single flag + reversible | ACCEPT | ACCEPT |
| Explicit bounds + named `S` + enable hard-gated on skew | required-change #4 → ACCEPT | required → ACCEPT |
| Resolve fence/`runner_id` durability pre-enable | required-change #2 → resolved §6 | open-Q → resolved §6 |

## 11. Unresolved blockers
**None.** Both slots signed off (Opus 4.8 ACCEPT; GPT 5.5 XHigh ACCEPT_WITH_NONBLOCKING_NOTES). Nonblocking notes folded into §4.3 (durable-runway strictness), §6 (explicit fence/`runner_id` decision), §7 T5 (additive worst-case), §8 step 2 (hard-fail on skew).

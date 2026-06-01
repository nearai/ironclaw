# Reborn cost-based budgets — follow-ups from #3841

Status: drafted 2026-05-22, post-merge of the foundation PR (689cd7901).

Implementation status: items **A1, A2, B1, C1, C2, D1, E1, F1** landed
in this branch on 2026-05-22 ahead of merging back to
`reborn-integration`. Item **G1** (BackgroundKind scheduler call sites)
is intentionally deferred until a Reborn scheduler exists. See the
"Acceptance evidence" appendix at the bottom of this file for the
regression tests added per item.

The foundation PR shipped the implementation seams (ledger, accountant,
gate, period rules, audit-sink contract, progress primitives, config
plumbing) and explicitly deferred the production wiring. Reviewers also
opened a small number of follow-ups during the review pass. This plan
consolidates *everything still open* into a single sequenced rollout so
no item gets dropped.

## What's still open

| # | Track | Item | Source |
|---|---|---|---|
| A1 | Production wiring | Wire `GovernorBackedAccountant` into prod host factory, driven by `BudgetDefaults` | PR body "Out of scope" #1 |
| A2 | Audit / SSE | Project `BudgetEvent` into the gateway event stream | PR body "Out of scope" #3 |
| B1 | Persistence | Filesystem-backed `BudgetGateStore` (mirror of `FilesystemResourceGovernorStore`) | PR body #2 + review comment #3 |
| C1 | Correctness | Cancellation-safe reservation release (Drop-based guard) so tokio cancel doesn't orphan a hold | Review comment #2 |
| C2 | Correctness | Thread real `input_tokens` / `output_tokens` / cost through `LoopModelResponse`; drop conservative-estimate fallback in `usage_for_response` | Review comment #1 |
| D1 | Cascade shape | Reshape `CascadeOutcome` to carry `(Vec<Warning>, terminal)` so accumulated warnings reach the UI before a pause or hard deny | Gemini comments + comments #4 and #6 |
| E1 | Cleanup | Remove dead `with_budget_accountant` plumbing on `ThreadBackedLoopModelPort` (or collapse with `HostManagedLoopModelPort`) | Review comment #5 |
| F1 | Stop detection | Implement the sliding-window stuck-loop / diminishing-returns strategy that consumes `ParamHash`, wire it into the executor as a stop-condition | PR body #5 + review comment #7 |
| G1 | Scheduler | Pass `BackgroundKind` from heartbeat / routine / mission / container-job call sites once those exist in Reborn | PR body #4 |

Items already fixed during the review-response commit (ccc1b66c7) are
**not** in this plan — see the resolved comments on #3841 for that list
(provider-error post-call propagation, overlapping reservations,
seeding cache poisoning, threshold 100% case, rolling-24h snapshot
window, env precedence test, TOML parser tests, `is_correlation_key`
allocations, UUID/timestamp normalization allocations, `ThresholdInputs`
refactor). The list above is only the open work.

## Suggested PR order

The order is driven by two things: (1) production safety wins land
first, (2) contract-changing PRs land before consumers that depend on
the new shape.

### PR 1 — A1: wire `GovernorBackedAccountant` into production

The biggest leverage in the smallest diff. The field already exists on
`RebornLoopDriverHostFactory::with_model_budget_accountant`; today it
defaults to `NoOpBudgetAccountant`, which means daily USD caps never
actually fire in prod. This PR:

1. Build a `GovernorBackedAccountant` in `crates/ironclaw_reborn_composition/src/lib.rs` (production composition) using:
   - the already-built `PersistentResourceGovernor` (`resource_store` /
     `governor` at lib.rs:370-371);
   - the `BudgetDefaults` resolved from
     `ironclaw_reborn_config::BudgetDefaults`;
   - a `ZeroCostTable` for now (until a cost table per provider lands —
     deferred).
2. Pass the accountant into `RebornLoopDriverHostFactory` via the
   existing `with_model_budget_accountant` builder
   (`crates/ironclaw_reborn/src/runtime.rs:346-348`).
3. Wire the same factory parameter through the
   `RebornRuntimeLoopParts` struct so non-production callers can still
   pass `None` and get `NoOpBudgetAccountant`.
4. Add a smoke test in `crates/ironclaw_reborn_composition/tests/` that
   builds the production substrate, executes one model call, and
   asserts a non-zero reservation reached the governor.

Acceptance: production runs deplete the per-user daily USD budget; the
no-op accountant remains the test default.

### PR 2 — B1: filesystem-backed `BudgetGateStore`

In-memory only ships today (`crates/ironclaw_resources/src/gate.rs:128`).
A restart drops every pending approval gate, forcing users to
re-request approval. Mirror the existing
`FilesystemResourceGovernorStore` shape:

- New file `crates/ironclaw_resources/src/filesystem_gate_store.rs`.
- Generic over `RootFilesystem` like the governor store.
- Atomic-replace + parent-dir-sync pattern (same as
  `filesystem_store.rs`).
- One JSON snapshot per scope:
  `/resources/budget-gates/<scope-key>.json`.
- Wire into `crates/ironclaw_reborn_composition/src/lib.rs` alongside
  the existing resource store.
- Contract tests in `crates/ironclaw_resources/tests/` covering open,
  resolve, expiry, and reload-after-restart parity with the in-memory
  store.

Acceptance: pending gates survive a process restart; the in-memory
store stays for tests.

### PR 3 — C1: cancellation-safe reservation release

Today both wrappers (`HostManagedLoopModelPort::stream_model`,
`ThreadBackedLoopModelPort::stream_model`) take a reservation in
`pre_model_call` and release / reconcile it in `post_model_call`. If
the future is cancelled mid-`stream_model`, the reservation orphans
until the period rolls over.

Approach: a `ReservationGuard` RAII struct that:

- Holds an `Arc<dyn LoopModelBudgetAccountant>`, a `TurnRunId`, and the
  reservation id;
- On `Drop`, *if not explicitly disarmed*, spawns a release task using
  `tokio::spawn` with a fallback to a synchronous best-effort log on
  shutdown;
- Gets `disarm()` called by the success and explicit-failure paths
  before returning.

This replaces the manual `in_flight.remove` + `release` pair. The
guard lives in `crates/ironclaw_loop_support/src/budget_accountant.rs`
(near the in-flight map) so the contract stays in one file.

Acceptance: drop-in regression test that cancels a `stream_model`
future mid-await and asserts the governor's active reservation count
returns to zero within one tokio tick.

### PR 4 — E1: kill the dead inner accountant

`ThreadBackedLoopModelPort` (`crates/ironclaw_loop_support/src/lib.rs:731-1004`)
carries an `Option<Arc<dyn LoopModelBudgetAccountant>>` and a
`with_budget_accountant` builder, but the production wiring at
`crates/ironclaw_reborn/src/loop_driver_host.rs:598-617` never sets
it — accountant work lives in the outer `HostManagedLoopModelPort`.
This is exactly the "optional Arc that is required in production"
smell in `.claude/rules/architecture.md` (#2), except inverted: the
inner field is *never* set, so every `is_some()` branch is dead.

Delete the inner accountant field, the `with_budget_accountant`
builder, and the dead branches in `stream_model`. Update the inner
tests to drop the now-unused setup. (Collapsing the two wrappers
entirely is a larger refactor; not in scope for this follow-up.)

Acceptance: `clippy::dead_code` clean; the `crates/ironclaw_loop_support`
unit + contract tests still pass; only the outer port runs accountant
hooks.

### PR 5 — D1: `CascadeOutcome` carries warnings alongside the terminal verdict

Today `evaluate_cascade_for_account`
(`crates/ironclaw_resources/src/lib.rs:1716`) short-circuits on the
first non-Allow intervention, dropping any warnings accumulated before
a pause/deny. The Gemini comment and review comments #4 + #6 hit this
from two directions:

- 85% USD warn + 92% concurrency pause → user sees pause, never sees
  the warn.
- 85% USD warn + 105% USD hard deny → user sees "budget exceeded",
  never sees the warning that should have preceded.

Reshape:

```rust
enum CascadeOutcome {
    Allow(Vec<BudgetWarning>),
    RequiresApproval {
        warnings: Vec<BudgetWarning>,
        needed: ResourceApprovalNeeded,
    },
    Deny {
        warnings: Vec<BudgetWarning>,
        denial: ResourceDenial,
    },
}
```

Call sites in `ResourceGovernor::reserve_with_outcome` thread the
warnings through `ReservationOutcome`. `BudgetEvent::Warned` fires for
every warning, even on the deny / approval path.

Acceptance: regression tests for both the warn-then-pause and
warn-then-deny shapes; existing 17 contract tests still pass.

### PR 6 — A2: project `BudgetEvent` into the gateway

Once D1 lands the event stream is complete; this PR connects the audit
sink to the gateway.

Sequence:

1. Plug a real `BudgetEventSink` into the governor in
   `crates/ironclaw_reborn_composition/src/lib.rs` next to the existing
   governor build. Sink writes into a `tokio::sync::broadcast` channel
   keyed by `ResourceScope`.
2. New projection in `src/bridge/router.rs` consumes the channel and
   emits `AppEvent::BudgetWarn` / `BudgetPause` / `BudgetDenied` /
   `BudgetLimitChanged`. The projection is the **only** producer of
   these `AppEvent` variants — see `.claude/rules/gateway-events.md`.
3. Wire-stable enum names: `snake_case` per `.claude/rules/types.md`;
   add an `#[serde(alias = …)]` migration path is not needed (these are
   new variants).
4. Frontend: subscribe in `crates/ironclaw_gateway/static/js/`; render
   a banner for warnings, a modal for pause/approval (the modal reuses
   the existing approval-gate UI from #3841 once B1 lands), a toast for
   denied.

Acceptance: a forced reservation that crosses warn → pause emits both
events to the SSE stream, with no direct `sse.broadcast` calls outside
the projection.

### PR 7 — C2: thread real provider token usage

The current "fail safe by reconciling the estimate" landed in
ccc1b66c7. That makes daily caps deplete but overstates ~20%. Long-term
fix: thread the real numbers from each provider response into
`LoopModelResponse` and use them in `usage_for_response`.

Touchpoints:

- `crates/ironclaw_turns/src/run_profile/host.rs:1011-1016` — add
  `input_tokens: Option<u64>`, `output_tokens: Option<u64>`,
  `usd: Option<Decimal>` fields to `LoopModelResponse`.
- All `LoopModelGateway` implementations: have them fill the fields
  when the provider returns them. For providers that don't (NEAR AI
  local, Ollama free), leave `None` and fall back to the estimate.
- `crates/ironclaw_loop_support/src/budget_accountant.rs::usage_for_response`:
  prefer response-provided numbers; fall back to estimate only when
  `None`.

This is a contract change on `LoopModelResponse`, but the new fields
default to `None` so existing callers keep compiling.

Acceptance: a `RecordingGateway` returning explicit token counts
reconciles to those counts (not the estimate); a gateway returning
`None` falls back to the estimate; daily cap depletion matches actual
spend within provider-reported precision.

### PR 8 — F1: stuck-loop detection wired into the executor

`ParamHash` ships. The strategy that consumes it does not.

Approach:

1. New file
   `crates/ironclaw_agent_loop/src/strategies/progress_strategy.rs`
   (a new strategy axis — see the strategies CLAUDE.md, "one decision
   axis per file"). Implement two sliding-window detectors per the
   `progress.rs` module docs:
   - Diminishing-returns: average assistant-output delta over the last
     N steps; below `min_delta_tokens` for `noprogress_consecutive_window`
     ticks → `StuckNoProgress`.
   - Repeated-tool-call: deque of recent `(CapabilityId, ParamHash)`;
     `repeat_threshold` identical entries in a row → `StuckLoop`.
2. Add a typed state slot for the deque + delta history under
   `crates/ironclaw_agent_loop/src/state/`.
3. Compose into the default planner
   (`crates/ironclaw_agent_loop/src/default_planner.rs`). The strategy
   produces a typed `LoopExit::StuckNoProgress` /
   `LoopExit::StuckLoop` — both already exist in
   `crates/ironclaw_turns::loop_exit` since #3841.
4. New family-level integration test in
   `crates/ironclaw_agent_loop/tests/`.

Acceptance: a stub family that calls the same tool with the same
normalized args three times exits as `StuckLoop`; one that produces
zero-delta assistant output for N consecutive steps exits as
`StuckNoProgress`; productive runs never trip either.

### PR 9 — G1: pass `BackgroundKind` from scheduler call sites

Lowest priority because **there are no production scheduler call sites
yet in Reborn** (heartbeat in `turn_runner` is lease-keepalive, not
agent ticking — confirmed in the PR body). This PR lands together with
whatever first introduces a real periodic call site. Until then the
enum stays exported; no scheduler code changes.

If the first scheduler arrives before this plan completes, the PR is:

- New `pre_model_call` overload (or a `BackgroundKind`-carrying
  request struct) that records the kind alongside the reservation;
- Per-kind ledgers in the governor (skip-and-persist on exhaustion);
- Tests for "container job exhausts container-job budget but
  user-initiated calls still pass."

## Cross-cutting acceptance gates

Every PR above must:

- Stay zero-warning under `cargo clippy --workspace --all-targets --all-features`.
- Pass `cargo test --workspace --all-features` end-to-end.
- Not add `#[allow(clippy::too_many_arguments)]` without an
  `arch-exempt: too_many_args, …, plan #NNNN` line above it
  (`.claude/rules/architecture.md`).
- Not add a `with_*` builder that production always invokes paired
  with an `Option<Arc<…>>` field that's only `None` in tests
  (`.claude/rules/architecture.md` #2). PR 4 deletes the existing
  instance of this; new ones are violations.
- Route any new `AppEvent` emission through a projection function
  (`.claude/rules/gateway-events.md`). PR 6 is the only one in this
  plan that touches SSE.
- Use newtypes for any new identity / scope / id values
  (`.claude/rules/types.md`).
- Fail loud on DB/IO/workspace reads — no
  `.unwrap_or_default()` on a `Result` without a `// silent-ok:`
  comment naming the operation (`.claude/rules/error-handling.md`).

## Acceptance evidence (2026-05-22)

Each item below ships with at least one regression test that locks the new
contract in:

| # | Test(s) |
|---|------|
| C2 | `ironclaw_loop_support::budget_accountant::tests::post_model_call_reconciles_provider_usage_when_response_threads_real_tokens` + e2e `c1_provider_tokens_reconcile_to_actual_usd` |
| D1 | `ironclaw_resources::tests::limit_exceeded_carries_warnings_from_other_dimensions` + e2e `d1_agent_deny_preserves_user_warn_event` |
| C1 | `ironclaw_loop_support::budget_accountant::tests::release_in_flight_drains_orphan_reservation_on_cancellation` |
| E1 | Covered by removing the dead field + 14 pre-existing accountant tests; `clippy::dead_code` clean. |
| Cost table | `ironclaw_reborn::model_gateway::LlmModelProfilePolicy::build_cost_table` exercised by the A1 wiring + e2e `c2_unknown_model_in_cost_table_uses_default_cost_fallback`. |
| B1 | `ironclaw_resources::filesystem_gate_store::tests::pending_gate_survives_restart_via_fresh_handle` + `list_pending_does_not_leak_across_tenants` + `terminal_gates_older_than_retention_are_pruned_on_next_write` (review feedback High #1 + Medium #7). |
| A1 | `ironclaw_reborn_composition::factory` exposes `local_runtime.{resource_governor, budget_event_sink, budget_gate_store, broadcast_budget_event_sink}`; production-shape helper `build_default_budget_accountant` wires the seeding policy + overestimate factor + gate store; unit test `seeds_compiled_default_user_cap_on_first_touch` + e2e `d3_seeding_policy_installs_default_cap_on_first_touch`. |
| A2 | `ironclaw_resources::tests::governor_emits_budget_events_through_event_sink` + composition e2e `broadcast_sink_publishes_events_to_subscribers` + bridge unit tests (`account_label_handles_every_cascade_level`, `sse_user_id_resolves_for_user_scoped_events`, `sse_user_id_is_none_for_tenant_scoped_events`, `budget_event_to_app_event_warn_carries_dimension_and_utilization`). The SSE projection ships under `src/bridge/budget_events.rs::spawn_budget_event_projection`; the binary spawns the task against `RebornRuntime::broadcast_budget_event_sink()` at startup. |
| F1 | `ironclaw_agent_loop::state::signature::tests::capability_call_signature_collapses_calls_that_differ_only_by_request_id` + `…_embedded_uuid`; `strategies::stop::tests::default_stop_condition_strategy::four_consecutive_low_token_turns_trigger_no_progress` + `occasional_low_token_turn_does_not_trip_no_progress`. Executor now populates `LoopExecutionState.recent_output_token_counts` from `LoopModelResponse::usage` after every turn; the default stop strategy reads it for the diminishing-returns escape (`StopKind::NoProgressDetected`). |
| 13 e2e | `crates/ironclaw_reborn_composition/tests/budget_e2e.rs` + `budget_approval_e2e.rs` cover F1, F2, F3, F4, F5, F6, C1, C2, C3, D1, D3, scripted multi-turn, broadcast subscription. F7 is covered by the unit-level cancellation test; D2 by the existing rolling-24h snapshot anchor test. B-series (background ticks) await G1's scheduler. |

Two pre-existing test failures are deliberately not in this list: the
legacy `ironclaw cli::completion::tests::test_run_generates_output`
debug-build stack overflow (passes with `RUST_MIN_STACK=16777216`, present
on `main`), and the `parameters_schema` fixture parse error in
`ironclaw_capabilities` / `ironclaw_reborn` / `ironclaw_processes`
test data (present on `main`). Neither shares a code path with this work.

### Review-feedback fixes layered on top

The original review pass on PR #3899 flagged 2 High and 5 Medium
findings; all were addressed before merge:

| Severity | Finding | Fix |
|---|---|---|
| High #1 | `FilesystemBudgetGateStore` cross-tenant leakage (everyone wrote into `__SYSTEM__`) | `new(filesystem, ResourceScope)` — per-tenant scoping + `list_pending_does_not_leak_across_tenants` regression. |
| High #2 | Accountant wired without default caps; production governor stays empty so `reserve_with_outcome_in_state` skips accounts without limits | Composition calls `BudgetSeedingPolicy::new(...)` from `BudgetDefaults::compiled_defaults().with_env()`; `d3` e2e asserts the seeded $5 user cap is installed on first model call. |
| Med #3 | RAII guard disarmed before `post_model_call.await` — cancellation during that await orphaned the reservation | Disarm AFTER `post_model_call` returns; `release_in_flight` made idempotent (peek-release-then-remove). |
| Med #4 | Failed release dropped the in-flight entry so transient storage errors lost the retry handle | Peek-release-then-remove; the entry stays on error with a warning log. |
| Med #5 | Missing cost-table entries silently reconciled paid usage as $0.00 | New `default_cost` field (≈GPT-4o pricing) used by both `estimate_for` and `usage_for_response`; `ZeroCostTable` remains the explicit opt-out for free / Ollama. |
| Med #6 | Paused dimension was dropped when a different dimension hard-denied | `absorb` closure pushes a `BudgetWarning` for the paused dimension AND stores it in the approval slot. |
| Med #7 | Terminal gates grew the snapshot without bound | `with_terminal_retention` (default 30 days) prunes on every mutation; `terminal_gates_older_than_retention_are_pruned_on_next_write` regression. |

## What this plan does NOT cover

- Replacing `MAX_WORKER_ITERATIONS = 500` with budget-based caps
  (#2843 Phase D in the original budgets plan). That's a separate
  decommission and not a follow-up of #3841.
- A real `ModelCostTable` per provider. PR 1 uses `ZeroCostTable`
  intentionally so prod gets the depletion-via-estimate behavior; a
  real cost table is its own deliverable.
- WebUI design for the approval modal. PR 6 wires the event; the UI
  reuses the existing approval-gate shell from #3841.

# Cost-Based Budgets — Follow-up Milestones

## Context

Issue [#2843](https://github.com/nearai/ironclaw/issues/2843) introduced
USD-based budgets cascading user → project → mission → thread → background
invocation, replacing the arbitrary iteration/time caps (`ThreadConfig::max_iterations
= 50`, `ORCHESTRATOR_DEFAULT_MAX_DURATION_SECS = 300`, `MAX_WORKER_ITERATIONS = 500`).

PR [#2847](https://github.com/nearai/ironclaw/pull/2847) landed the foundation:
types, DB schema (V25 on both backends), libSQL + PostgreSQL `BudgetStore` impls
(with a 100-task concurrency test proving exactly 50 of 100 reservations succeed
against a tight limit), the `BudgetEnforcer` runtime with cascade / graduated
intervention / period arithmetic, `HybridStore` delegation, the
`BASH_MAX_OUTPUT_LENGTH` per-call output cap, and `.env.example` documentation
of `BUDGET_*` vars.

The enforcer in #2847 is correct and tested in isolation but **not yet wired
into the runtime**. `BUDGET_ENFORCEMENT_MODE` defaults to `off` so upgrading
deployments see no behavior change. This plan sequences the remaining work so
each follow-up is a clean, reviewable PR with its own acceptance test.

## Guiding principles (unchanged from #2843)

1. **USD is primary.** Tokens and wall-clock are secondary backstops. Iteration count is not a budget dimension at all.
2. **Pre-flight reservation, post-flight reconciliation.** No "many concurrent threads each individually under budget, collectively 10× over" race.
3. **Graduated intervention.** Info ≥ 50%, Warn ≥ 75%, RequiresApproval ≥ 90% (Enforce mode only), ExhaustedUsd > 100%.
4. **Background work is explicitly budgeted at schedule time.** Heartbeat / routines / missions / jobs each allocate a per-invocation budget before dispatch.
5. **User override is explicit and audited.** Increasing a budget writes a `budget_events` row; there's no env-var escape hatch.
6. **Progress detection is orthogonal.** A thread can be killed for being stuck even with budget remaining. Loop detection runs alongside, not as a substitute.
7. **Hard caps remain as backstops.** `HARD_CAP_WALL_CLOCK_SECS = 86_400`, `HARD_CAP_ITERATIONS = 10_000`, `HARD_CAP_BUDGET_USD_STR = "100.00"` live in `ironclaw_common` and fail validation at startup when exceeded.

## Rollout sequence

The six milestones below are **sequenced, not independent**. Each assumes the previous milestones have landed and expects a specific enforcement-mode transition:

```
#2847 landed ──▶  M1  ──▶  M2  ──▶  M3  ──▶  M4  ──▶  shadow-mode staging (7d)
                                                             │
                                                             ▼
                                                     flip staging to warn
                                                             │
                                                             ▼
                                                             M5
                                                             │
                                                             ▼
                                                     flip staging to enforce
                                                             │
                                                             ▼
                                                             M6  (delete old caps)
                                                             │
                                                             ▼
                                                     flip production to enforce
```

---

## M1. Execution-loop LlmBackend decorator wiring

**Goal.** Every `LlmBackend::complete()` call in the engine is preceded by
`BudgetEnforcer::reserve()` and followed by `reconcile()`. A budget denial
either fails the thread with `ThreadState::Failed { reason: BudgetExhausted }`
or transitions to `ThreadState::Waiting` behind an approval gate.

### Design

Wrap `LlmBackend` with a host-side decorator in `src/bridge/budget_gate_llm.rs`:

```rust
pub struct BudgetedLlmBackend {
    inner: Arc<dyn LlmBackend>,
    enforcer: Arc<BudgetEnforcer>,
    scope_resolver: Arc<dyn ScopeResolver>,
}

#[async_trait]
impl LlmBackend for BudgetedLlmBackend {
    async fn complete(&self, request: LlmRequest) -> Result<LlmOutput, LlmError> {
        let scopes = self.scope_resolver.resolve(&request).await?;
        let estimate = estimate_cost(&request);
        let ticket = match self.enforcer.reserve(&scopes, estimate.usd, estimate.tokens, Utc::now()).await? {
            Ok(ticket) => ticket,
            Err(denial) => return Err(LlmError::BudgetDenied(denial)),
        };
        match self.inner.complete(request).await {
            Ok(output) => {
                let actual = actual_cost(&output);
                self.enforcer.reconcile(&ticket, actual.usd, actual.tokens, Utc::now()).await?;
                Ok(output)
            }
            Err(e) => {
                self.enforcer.release(&ticket, Utc::now()).await.ok();
                Err(e)
            }
        }
    }
}
```

A new `LlmError::BudgetDenied(BudgetDenial)` variant carries the structured
denial so the execution loop can map it to the right `ThreadState` without
string-matching.

`ScopeResolver` is a thin trait the bridge implements: given a thread (or the
request metadata containing thread_id/user_id/project_id/mission_id), return
the cascade `Vec<BudgetScope>` in `user → project → mission → thread` order.
`HybridStore` already has all the state needed to answer this.

**Cost estimation.** Pre-flight uses the provider's `cost_per_token()` × the
configured max output tokens (or a 4K fallback). Conservative estimate; the
reconcile step releases the overshoot.

### Engine changes

`crates/ironclaw_engine/src/executor/loop_engine.rs` handles
`LlmError::BudgetDenied`:

- `ExhaustedUsd` / `ExhaustedTokens` → `ThreadState::Failed { reason: BudgetExhausted { details } }`, emit `ThreadEvent::BudgetExhausted`.
- `RequiresApproval` → `ThreadState::Waiting { gate: BudgetApproval { scope, utilization, request_id } }`, emit `ThreadEvent::BudgetApprovalRequired`. Resume path matches existing tool-approval gate shape.

New `ThreadEvent` variants (append, don't renumber):
- `BudgetReserved { scope, amount_usd, utilization_after }`
- `BudgetWarning { scope, tier, utilization }`
- `BudgetExhausted { scope, limit, spent }`
- `BudgetApprovalRequired { scope, utilization, request_id }`
- `BudgetApproved { scope, new_limit }` / `BudgetOverride { scope, old, new }`

### Rollout gate

Requires #2847 merged and `BUDGET_ENFORCEMENT_MODE=off` still default. In
shadow mode the decorator records reservations but never returns
`LlmError::BudgetDenied` — this is the mode we calibrate defaults against.

### Testing criteria

Unit:
- `decorator_reserves_before_complete` — MockLlm + FakeStore; the reserve call happens, then complete.
- `decorator_reconciles_with_actual_cost` — actual < reserved releases the overshoot, actual > reserved pushes spent past the reservation.
- `decorator_releases_on_llm_error` — LLM returns an error, reservation is released (not reconciled).
- `decorator_passes_through_in_off_mode` — `mode=Off`, no store calls at all.
- `denial_maps_to_thread_failed` — `BudgetDenial::ExhaustedUsd` → thread state goes to `Failed { BudgetExhausted }`.
- `approval_gate_maps_to_waiting` — `BudgetDenial::RequiresApproval` → thread state `Waiting { BudgetApproval }` with a `request_id`.
- `approval_resume_calls_enforcer_override_then_proceeds` — approve path calls `enforcer` to extend the budget, then resumes.

Integration (`cargo test --features integration`):
- `test_thread_fails_cleanly_when_user_daily_exhausted` — PG; seed user budget at cap, spawn thread, assert `Failed { BudgetExhausted }`.
- `test_warning_surfaces_on_thread_event_stream` — 80% utilisation crossing emits `ThreadEvent::BudgetWarning` on the broadcast channel.

### Acceptance

- [ ] `LlmBackend` callers in the engine go through `BudgetedLlmBackend` when `mode != Off`.
- [ ] `LlmError::BudgetDenied` variant exists and the decorator never string-matches.
- [ ] Zero new `ThreadEvent` variant numbering conflicts (append-only).
- [ ] Engine integration tests at least 5 new cases.
- [ ] PR description includes five measured threads with their reserved vs. actual USD.

---

## M2. Scheduler wiring: heartbeat, routines, missions, jobs, container

**Goal.** Every background dispatcher allocates a per-invocation budget at
schedule time. If the user's daily cap is exhausted, the tick is **skipped**
(logs a `BudgetExhausted` event, keeps the schedule alive) — the scheduler
does not crash-loop.

### Five touch-points

Each gets the same shape: `ScheduleSpec` grows a `budget_usd: Option<Decimal>`;
at dispatch time, the scheduler builds a `BackgroundInvocation` scope with the
appropriate `BackgroundKind` + a fresh correlation id, and calls
`BudgetEnforcer::reserve()` with the cascade `[user, project?, background_invocation]`.

| Dispatcher | File | BackgroundKind | Default (from `BudgetConfig`) |
|---|---|---|---|
| Heartbeat tick | `src/agent/heartbeat.rs` | `Heartbeat` | `heartbeat_per_tick_usd` |
| Routine (lightweight) | `src/agent/routine_engine.rs` | `RoutineLightweight` | `routine_lightweight_usd` |
| Routine (standard) | same | `RoutineStandard` | `routine_standard_usd` |
| Mission tick | `crates/ironclaw_engine/src/runtime/mission.rs` | `MissionTick` | `mission_per_tick_usd` |
| Container job | `src/worker/{job.rs, container.rs}` | `ContainerJob` | `job_default_usd` |

A dispatched tick holds its reservation across its entire LLM run — the
execution-loop decorator (M1) walks the same cascade and reuses the same
tickets, avoiding double-booking.

### Wire contract changes

- `HeartbeatConfig::per_tick_budget_usd: Option<Decimal>` (None ⇒ use default).
- `RoutineConfig::{lightweight_budget_usd, standard_budget_usd}` replace
  `max_lightweight_tokens` / `lightweight_max_iterations` — but only in M6 (the
  deletion milestone). M2 adds the new fields alongside; M6 removes the old.
- `Mission.per_tick_budget_usd: Option<Decimal>` with migration default = `NULL`.
- `JobSpec.budget_usd: Option<Decimal>` with migration default = `NULL`.
- `src/orchestrator/job_manager.rs` — the existing 1..500 iteration clamp stays as a backstop; the real limit is the budget. The clamp is deleted in M6.

### Rollout gate

Requires M1 merged and running in staging. M2 is inert in `mode=off`.

### Testing criteria

Unit:
- `heartbeat_tick_skipped_when_user_cap_exhausted` — seed ledger at cap, tick fires, invocation is dropped, `BudgetExhausted` event recorded, schedule persists.
- `routine_dispatch_uses_kind_specific_default` — lightweight tick reserves `routine_lightweight_usd`, standard reserves `routine_standard_usd`.
- `mission_tick_inherits_cascade` — tick fires against a mission with a parent project; reserve walks `[user, project, mission, background_invocation]` in order.
- `container_job_budget_denies_before_container_start` — budget denial fires pre-start, no Docker resources touched.

Integration:
- `test_e2e_heartbeat_runs_skips_when_daily_cap_exhausted` — run for 3 heartbeat ticks; first succeeds, second + third skipped when cap hits, schedule still active.
- `test_e2e_mission_budget_runs_across_days_with_daily_cap` — rolling-24h budget resets next day; mission that was denied yesterday succeeds today.

### Acceptance

- [ ] Five schedulers each allocate a per-invocation budget at dispatch time.
- [ ] Skipped ticks log a `BudgetExhausted` event but **do not** delete the schedule.
- [ ] No scheduler grows a crash-loop when its user runs out of budget.
- [ ] PR description includes a 7-day shadow-mode trace from staging.

---

## M3. Progress / loop detection (orthogonal safety net)

**Goal.** A thread can be killed for being **stuck** even with budget
remaining. Budget catches cost overruns; progress detection catches productive-
looking waste (repeating tool calls, no token delta, infinite "planning"
loops).

### Design

New module `crates/ironclaw_engine/src/runtime/progress.rs`:

- **Diminishing returns**: over the last `PROGRESS_NOPROGRESS_WINDOW` steps (default 3), if the sum of non-boilerplate token deltas is below `PROGRESS_DIMINISHING_TOKENS` (default 500), emit `ThreadEvent::NoProgress`. Two consecutive `NoProgress` events fail the thread with `StuckNoProgress`.
- **Repeated tool call**: hash `(tool_name, normalized_params)` where normalisation strips timestamps, request IDs, and random UUIDs. If the same hash appears `PROGRESS_LOOP_THRESHOLD` times (default 3) consecutively, inject a nudge message. At 5× consecutive, fail with `StuckLoop`.

Parameter normalisation helper in `crates/ironclaw_engine/src/util/param_hash.rs`:

```rust
pub fn normalized_hash(tool: &str, params: &serde_json::Value) -> u64 {
    let mut canonical = params.clone();
    strip_timestamps(&mut canonical);
    strip_uuids(&mut canonical);
    strip_counter_like_ints(&mut canonical);
    hash_bytes(tool, &canonical.to_string())
}
```

Normalisation is intentionally conservative — we'd rather miss a loop than
kill productive work that happens to include timestamps. The test corpus
covers both directions.

### New ThreadEvent variants (append-only)

- `NoProgress { window, tokens_delta }`
- `RepeatedToolCall { tool, count }`
- `StuckNoProgress { reason }` (terminal)
- `StuckLoop { tool, count }` (terminal)

### Configuration

```
PROGRESS_ENABLED=true
PROGRESS_NOPROGRESS_WINDOW=3
PROGRESS_DIMINISHING_TOKENS=500
PROGRESS_LOOP_THRESHOLD=3
PROGRESS_LOOP_FAIL_THRESHOLD=5
```

Defaults in `src/config/progress.rs` mirror the budget-config pattern.

### Rollout gate

Independent of budget mode — can ship on `staging` while budget is still in
shadow. Default `PROGRESS_ENABLED=true`; the thresholds are conservative
enough that a well-behaved thread never trips them. Shadow-mode equivalent:
emit events but don't terminate (`PROGRESS_ENFORCE=false`).

### Testing criteria

Unit:
- `diminishing_returns_fires_after_3_low_delta_steps` — fabricate step history, assert event order.
- `diminishing_returns_does_not_fire_when_tokens_still_growing` — 400-token deltas pass.
- `repeated_tool_call_normalises_timestamps` — same tool + params with `t1`, `t2`, `t3` counts as 3 consecutive.
- `repeated_tool_call_normalises_uuids` — random UUIDs in params don't defeat detection.
- `repeated_tool_call_does_not_normalise_semantic_ids` — `user_id=alice` in one call vs `user_id=bob` in the next is NOT a repeat.
- `stuck_loop_failure_is_terminal_and_not_retried` — after `StuckLoop`, thread is `Failed`, no re-schedule.
- `stuck_loop_event_fires_at_exactly_5x_not_4x_not_6x` — off-by-one regression guard.

Integration:
- `test_e2e_loop_caught_before_budget_exhausted` — craft a prompt that induces a 10-iteration tool loop; thread is killed inside 3s with `StuckLoop`, budget ledger shows <$0.02 spent.

### Acceptance

- [ ] Parameter normalisation test corpus: ≥ 12 cases (strip-timestamps, strip-uuids, preserve-semantic-ids).
- [ ] Off-by-one tests for both `NoProgress` window and `StuckLoop` threshold.
- [ ] PR description calls out that this is orthogonal to budget and gives an example trace of a budget-healthy but stuck thread caught by progress detection.

---

## M4. CLI + `ToolDispatcher`-registered budget tools

**Goal.** Users can see their current budget state, override limits, approve/
cancel gated threads — all through the CLI and through agent-callable tools.

### CLI surface (`src/cli/budget.rs`)

```
ironclaw budget show [--scope user|project:<id>|mission:<id>]
ironclaw budget set  --scope <...> --daily-usd 10.00 [--weekly-usd ...]
ironclaw budget history [--scope <...>] [--since <duration>]
ironclaw budget approve <thread_id>
ironclaw budget cancel  <thread_id>
```

Each CLI subcommand builds params and dispatches through `ToolDispatcher`
(per `.claude/rules/tools.md`) — there is no direct-DB path. The CLI is just
a user-friendly frontend over the tools.

### Tools (`src/tools/builtin/budget_tools.rs`)

| Tool | Params | Behavior |
|---|---|---|
| `budget_show` | `{ scope? }` | Returns active budgets + ledger state for the authenticated user's scope cascade. |
| `budget_increase` | `{ scope, new_limit_usd, reason }` | Validates against hard-cap invariant; writes a new `Budget` row with `source=UserOverride`; deactivates the previous row; appends `budget_events` row with `event_kind='override'`. |
| `budget_history` | `{ scope?, since? }` | Lists `budget_events` rows, paginated. |
| `budget_approve` | `{ thread_id, extend_usd? }` | Resolves an open approval gate. If `extend_usd` given, extends the relevant scope's limit before resuming. |
| `budget_cancel` | `{ thread_id }` | Resolves the gate by transitioning the thread to `Failed { BudgetExhausted }` without extending. |

All tools follow the existing Tool trait pattern (`Tool`, `ToolOutput`,
`ToolError`). Each declares `requires_sanitization=true` for any string
params. `sensitive_params()` on `budget_increase` returns nothing (budgets
aren't secret; the audit record is).

### Rollout gate

Requires M1 + M2 (approve/cancel only make sense once approval gates actually
fire). Can ship while budget mode is still `shadow` or `warn`.

### Testing criteria

Unit:
- `budget_show_returns_cascade` — creates user+project+thread budgets; tool returns all four.
- `budget_increase_rejects_above_hard_cap` — try to set $1000; fails with `ToolError::InvalidInput`.
- `budget_increase_writes_audit_row` — verifies `budget_events` row appears with `event_kind='override'` and correct `actor_user_id`.
- `budget_approve_without_extend_resumes_thread` — extend=None path works if there's headroom somewhere in the cascade (shouldn't in practice).
- `budget_approve_with_extend_writes_new_budget_row` — old row deactivated, new row active.

Integration:
- `test_e2e_cli_set_then_show` — CLI set $10, CLI show reflects $10.
- `test_e2e_cli_budget_tools_go_through_dispatcher` — static-check ensures no direct DB calls.

### Acceptance

- [ ] Pre-commit `scripts/pre-commit-safety.sh` passes — no `state.store.*` lines added.
- [ ] Every tool goes through `ToolDispatcher::dispatch` with a test for each.
- [ ] CLI → tool → DB flow documented in `src/cli/budget.rs` module docs.
- [ ] `ironclaw budget --help` output is clear and short.

---

## M5. Web UI — budget chip + approval gate modal

**Goal.** The web UI shows a budget status chip in every thread header, and
when a thread hits the 90% approval gate, the user sees a modal to extend or
cancel.

### Design

New JS module `crates/ironclaw_gateway/static/js/core/budget.js` — **do not
inline into app.js**, follow the split pattern documented in
`CLAUDE.md` → "Extension/Auth Invariants" (same reasoning: split modules stay
reviewable). Concat order updated in `src/assets.rs`.

Rust side:
- `src/channels/web/server.rs` rehydrates budget approval gates on history
  replay the same way it rehydrates auth gates today (the v2 path with
  `request_id` through `/api/chat/gate/resolve`). Budget gates share the
  pending-gate machinery — no new wire shape.
- New SSE events surface `ThreadEvent::BudgetWarning`, `BudgetExhausted`,
  `BudgetApprovalRequired` — each as its own discriminated-type event.
  (See `.claude/skills/add-sse-event/`.)

### UI specifics

| Utilization | Chip color | Chip text |
|---|---|---|
| < 50% | neutral | `$X.XX / $Y.YY today` |
| 50–75% | info | same |
| 75–90% | warning | `$X.XX / $Y.YY today (ZZ%)` |
| ≥ 90% | critical | `$X.XX / $Y.YY today (ZZ%) — approve to continue` |

Approval modal:
```
┌─────────────────────────────────────────┐
│ Budget approval required                │
│                                         │
│ This thread has reached 92% of your     │
│ daily budget ($4.60 / $5.00).           │
│                                         │
│ [Extend by $2] [Extend by $5]           │
│ [Cancel thread]                         │
└─────────────────────────────────────────┘
```

Extend buttons call `budget_approve` tool with the corresponding `extend_usd`.
Cancel calls `budget_cancel`.

### Rollout gate

Requires M4 (the tools it calls).

### Testing criteria

Unit (JS):
- `budget_chip_renders_color_by_utilization` — four utilization buckets.
- `approval_modal_extend_dispatches_to_tool` — click → fetch to `/api/chat/gate/resolve` with budget-approval shape.

Integration (e2e, `tests/e2e/`):
- `test_e2e_web_ui_shows_budget_chip` — spawn thread, chip appears with initial $0.00.
- `test_e2e_web_ui_approval_modal_extend_resumes` — saturate to 92%, modal appears, click extend $2, thread resumes.
- `test_e2e_web_ui_approval_modal_cancel_fails_thread` — same setup, cancel, thread transitions to `Failed`.

### Acceptance

- [ ] `budget.js` ships as its own module, concat order in `assets.rs` updated.
- [ ] Rehydration test: refresh the browser mid-gate; the modal re-opens.
- [ ] No backend changes to `/api/chat/gate/resolve` — budget gates ride the existing path.
- [ ] Screenshot in PR body of the chip and the modal at 92%.

---

## M6. Delete old caps

**Goal.** Once shadow → warn → enforce has run cleanly in production for
≥ 7 days, remove the pre-budget caps that are now dead weight.

### Removals

- `ironclaw_common::MAX_WORKER_ITERATIONS` — delete.
- `ThreadConfig::max_iterations` — delete. Threads are bounded by budget + progress detection.
- `ThreadConfig::max_duration` — delete. Wall-clock is either a hard-cap invariant (24h) or a budget dimension.
- `ThreadConfig::max_tokens_total` — delete; rolled into `BudgetLimit::tokens`.
- `ThreadConfig::max_budget_usd` — delete; rolled into the scope-scoped `Budget` rows.
- `ORCHESTRATOR_DEFAULT_MAX_DURATION_SECS = 300` in `crates/ironclaw_engine/src/executor/orchestrator.rs` — delete the env-tunable path. Keep a 24h `ORCHESTRATOR_HARD_CAP_SECS` constant as an absolute backstop.
- `src/config/routines.rs::max_lightweight_tokens` and `lightweight_max_iterations` — delete. Replaced by `routine_lightweight_usd` / `routine_standard_usd` landed in M2.
- `src/orchestrator/job_manager.rs` iteration clamp (1..500) — delete.

### Keeps

- Per-tool timeouts (shell 120s, HTTP 30s default / 300s max, memory 15s). These are per-*call* timeouts, not session-wide, and remain correct.
- `BASH_MAX_OUTPUT_LENGTH` (30KB default / 150KB cap) — per-call output cap, unchanged from #2847.
- Consecutive-error cap (`max_consecutive_errors`). Error-rate safety, orthogonal to cost.
- Tool-intent / action-requirement nudges. UX nudges, unaffected.

### Invariants kept

- `HARD_CAP_WALL_CLOCK_SECS = 86_400` (24h).
- `HARD_CAP_ITERATIONS = 10_000`.
- `HARD_CAP_BUDGET_USD_STR = "100.00"`.

A config that tries to exceed any of these still fails validation at startup —
these are invariants, not defaults.

### Rollout gate

**Do not merge until** `BUDGET_ENFORCEMENT_MODE=enforce` has run in production
for ≥ 7 days with no emergency rollback. That is the only signal that says
"the new system works well enough that the old backstops can come out."

### Testing criteria

- `grep -R "max_iterations" src/ crates/` — returns zero non-test hits outside historical docs.
- `grep -R "MAX_WORKER_ITERATIONS" src/ crates/` — returns zero hits.
- Regression: `regression_orchestrator_5min_cap_no_longer_applies` — run an orchestrator step that legitimately takes 8 minutes under a 15-minute thread budget; completes successfully.
- Regression: `regression_200_iteration_thread_completes` — synthetic thread that legitimately needs 200 LLM calls under a sufficient budget; completes successfully.
- Full test suite + integration + e2e green.

### Acceptance

- [ ] Production has been in `mode=enforce` for ≥ 7 days at time of merge.
- [ ] `budget_events` from the prior week show no emergency overrides.
- [ ] The two regression tests above land in the same PR.
- [ ] `CLAUDE.md` is updated — the "Job State Machine" section no longer
      mentions iteration-count termination; instead points at
      `BudgetExhausted` and `StuckLoop`/`StuckNoProgress`.

---

## Cross-cutting testing

Beyond each milestone's own tests, these acceptance tests span multiple
milestones and live in `tests/e2e/`:

- `test_e2e_long_running_thread_1_hour_under_generous_budget_completes` — with a $10 budget and `mode=enforce`, run a multi-tool research task that runs ~1 hour; verify completion and that no iteration/wall-clock cap fires. The "can run for hours" acceptance test from #2843.
- `test_e2e_stuck_thread_killed_in_seconds_despite_budget` — craft a repeating-tool-call loop under a $100 budget; thread killed by `StuckLoop` within 10s. The "doesn't waste cycles" acceptance test.

Both tests land in M6, after the old caps are gone — their value is
demonstrating the *new* system works without any hidden backstops.

## Out of scope for this plan

- Per-tool costing beyond LLM cost (e.g. "Brave Search API calls cost $0.001 each"). The ledger is already tool-agnostic; per-tool pricing is a follow-up to this plan.
- Dynamic pricing fetching from providers. `src/llm/costs.rs` remains a static table; manual updates.
- Currency other than USD.
- Retroactive budget enforcement (setting a $2 cap after $5 has been spent does not claw back the spend).
- Multi-user missions (`Mission` is currently single-user; if that ever changes, the cascade needs to revisit).

## References

- Issue: [#2843](https://github.com/nearai/ironclaw/issues/2843) — full rework motivation, architecture, and 29-test acceptance checklist.
- Foundation PR: [#2847](https://github.com/nearai/ironclaw/pull/2847) — types, DB schema, store impls, enforcer, bash cap.
- `.claude/rules/tools.md` — every new tool goes through `ToolDispatcher::dispatch`.
- `.claude/rules/database.md` — both backends on every new persistence feature.
- `.claude/rules/types.md` — canonical newtype template for any new IDs.
- `.claude/rules/error-handling.md` — fail-loud DB reads, no `unwrap_or_default()` without `// silent-ok:` justification.

# Subagent + Compaction — Soundness Eval + Implementation Plan

Date: 2026-06-06.
Source design doc: `docs/reborn/2026-06-04-subagent-compaction-design.md`.
Verification: 5 parallel sonnet workers grounded by codegraph against the live tree.

---

## Part 1 — Soundness evaluation

### Verdict

**Architecturally sound, executionally under-specified.** The single-seam thesis (`PostCapabilityStage` between CapabilityStage and StopStage) holds up against the code. The trait-first compaction policy and the mailbox-drain port shape both survive scrutiny. Step 1 (proactive compaction) is genuinely shippable independent of durability.

But the doc has eight concrete gaps that block implementation as written. Most are scope misses, not thesis errors — three of them block step 1 directly and must be closed before any PR opens.

### Score by section

| Area | Verdict |
|---|---|
| Single-seam thesis | Sound. `PostCapabilityStage` at the 7→8 seam is structurally feasible — `TurnCompletedStep` is the right in/out type and `ExecutorStage<TurnCompletedStep>` is implementable. |
| `CompactionForceStrategy` trait | Sound shape, wrong sig. `BudgetState` type does not exist; signature must accept `&LoopExecutionState` + `&dyn BudgetStrategy` (or a new wrapper). Sizing via `LoopResultRef` requires a store fetch — flagged as perf concern. |
| `skip_model_this_iteration` | **WRONG MECHANISM.** PromptStage already returns to `canonical.rs` before ModelStage runs; PromptStage cannot short-circuit ModelStage via a state flag. Flag must land on `PromptStep::Prepared` output (or new variant), AND `canonical.rs` must read it between the match arm and `.stop.observe()`. Doc skipped this. |
| Background re-enable single gate | Sound. One rejection site in `TryFrom`, confirmed. But `SpawnSubagentArgs` drops `mode` after decode AND the tool JSON schema does not advertise `mode` — both must be patched or model never requests background. |
| Depth/concurrency policy | **Doc factually wrong.** `SubagentSpawnLimits.max_depth` already exists, defaults to 1. No `max_concurrent_background_children` — that gap is real. |
| Durable stores list | Incomplete + wrong path. `gate_resolution.rs` lives in `ironclaw_reborn/subagent/`, not `ironclaw_loop_support/`. Gate store has 3 denormalized maps under one mutex (atomic migration). A 4th in-memory store (`BoundedSubagentResultTombstoneStore`) is entirely missing from the doc. |
| `CapabilityResultStore` trait | Does not exist. Doc treats the durable swap as drop-in; reality requires introducing the trait first. `SubagentRestartReconciler` is a stub enum member only — no interface, no boot-replay prior art anywhere in the codebase. |
| Event projection chain | Mostly correct, scope overstated AND understated. Constructor blast radius = 1 struct (`RuntimeEventPayload`) via post-construction assignment — doc implies 17 sites. But a second emission site (`live_progress.rs:147 product_items_for_live_update`) was missed. JOIN alternative is cheaper than doc implies — `ironclaw_event_projections` already depends on `ironclaw_turns`. |

### Blocking gaps to close before Step 1 merges

1. **PromptStage cannot short-circuit ModelStage from inside itself.** Real mechanism: add `skip_model_this_iteration: bool` to `PromptStep::Prepared`; thread it through `canonical.rs` to skip the `match` branch's `ModelStage.process(...)` call AND go straight to `.stop.observe(...)`. Spec must document this or step 1 doesn't ship.
2. **`CompactionForceStrategy` trait sig.** `BudgetState` doesn't exist. Either introduce it as a tiny wrapper around `BudgetStrategy::iteration_limit` + token snapshot, OR drop the budget arg from v1 and pass `&LoopExecutionState`.
3. **Unit mismatch.** `observed_prompt_tokens` is `char/4` estimate, not real tokens, not bytes. A v1 `ByteCapStrategy` table introduces a THIRD unit. Either convert byte caps through the same char/4 heuristic, OR explicitly accept the three-unit gap and document the v1 deferral.
4. **CapabilityStage batches calls per turn.** `PostCapabilityStage` fires once per turn after the whole batch. Doc says "single oversize capability result" — must read as "any result in the batch trips the policy."

### Blocking gaps for Steps 2–5

5. **Path correction:** `BoundedSubagentGateResolutionStore` is in `crates/ironclaw_reborn/src/subagent/gate_resolution.rs`, not `ironclaw_loop_support/`.
6. **Add tombstone store** (`BoundedSubagentResultTombstoneStore`) to the durability list. Its production-readiness check already blocks production.
7. **`CapabilityResultStore` trait must be introduced** before any durable backend can drop in. Cannot ship as direct backend swap.
8. **Spawn schema + args field.** `SpawnSubagentArgs` needs `mode: SpawnSubagentMode` and `spawn_subagent_parameters_schema()` needs a `mode: enum[blocking, background]` property.

### Soft corrections

- Pipeline position numbers (0–11) are doc-only fiction. Real pipeline has 9 named stages; `CheckpointStage` is inline. Use stage names, not positions.
- Threshold formula: `context_limit - max(reserve, main_loop_max_output)`. Doc's `128k - 20k = 108k` is only correct when `main_loop_max_output_tokens = 0` (current default).
- Line numbers drift up to ~50: `state.rs:48→49`, `slots.rs:21→23`, `projection.rs:1025→967`. Use symbol names, not line numbers.
- `force_compact_on_next_iteration` is set at 5 sites (incl. `model.rs:245` ShrinkContext recovery). Existing flag, not new.
- Settlement semantics are **first-writer-wins** in memory (`gate_resolution.rs:69` skips re-recording). Durable store should match: `INSERT OR IGNORE` / `ON CONFLICT DO NOTHING`, not last-write-wins.
- `CapabilityOutcome::SpawnedChildRun` variant already exists with `child_run_id`, `result_ref`, `safe_summary`. Use it for background, don't overload `Completed`.

### Tech-lead read

The doc is ~80% accurate at the seam-and-trait level and ~60% accurate at the symbol-and-line level. Treat it as a design rationale, not a build sheet. Step 1 is shippable in ~1 week with the four blocking gaps closed. Steps 2–6 need the schema+parity sub-spec the doc itself calls for, written FIRST.

---

## Part 2 — Implementation plan

Split into six independently-tracked work units matching the doc's `Build order`. Step 1 ships first as a standalone PR. Steps 2–6 wait on the sub-spec (Step 2 prework).

### Naming

- WU-A: Step 1 — PostCapabilityStage shell + Responsibility 1 (proactive compaction). **Ships standalone.**
- WU-B: Step 2 prework — Durability schema + parity sub-spec.
- WU-C: Step 2 — Durable stores (libsql + postgres) + `RestartReconciler` + idempotency ledger.
- WU-D: Step 3 — Background mode re-enable behind feature toggle.
- WU-E: Step 4 — `LoopBackgroundChildPort` + `drain_settled` + Responsibility 2 fill.
- WU-F: Step 5 — WebUI parent-child nesting.
- WU-G: Step 6 — Parity test (#4431 follow-on).

Blocking edges:

```
WU-A  (ships now, blocks nothing)
WU-B → WU-C → { WU-D, WU-E, WU-F → WU-G }
```

---

### WU-A — PostCapabilityStage + Responsibility 1 (ships standalone)

Goal: proactive compaction when a capability result trips the policy. Background-drain branch is no-op (owner-of-record set; producers don't exist yet).

**Boundary-compliant per `crates/ironclaw_agent_loop/CLAUDE.md` + `_contract-freeze-index.md`:** no new agent_loop deps; state stores counters/refs only; lifecycle mechanics owned by the stage, not `canonical.rs`.

**Files touched:**

| File | Change |
|---|---|
| `crates/ironclaw_agent_loop/src/state.rs` | Add `pending_capability_bytes: BTreeMap<CapabilityId, u64>` to `LoopExecutionState` (host_api dep already present). **`BTreeMap`, not `HashMap`** — existing slot precedent (`RecoveryStrategyState.attempts_by_class` at `slots.rs:216`) uses `BTreeMap` for deterministic serde order across checkpoint/replay. Per-cap accounting needed for future `BudgetFractionPolicy`. **Accumulate-vs-clear semantics:** `push_completed_result` accumulates per call within a turn; `PostCapabilityStage` clears the whole map AFTER deciding to trip (a single oversize call trips the policy; remaining calls in the same batch are already counted into the trip decision because batch loop finishes before `PostCapabilityStage` runs). |
| `crates/ironclaw_agent_loop/src/state/slots.rs` | Add `skip_model_this_iteration: bool` as a NEW typed slot (not on `CompactionStrategyState` — it's a one-shot pipeline directive, not strategy resumable state). |
| `crates/ironclaw_agent_loop/src/strategies/compaction.rs` | New trait `CompactionForceStrategy { fn should_force_compact(&self, state: &LoopExecutionState) -> Option<CompactionReason>; }`. v1 impl `ByteCapStrategy { caps: BTreeMap<CapabilityId, u64> }`. Defaults: `SPAWN_SUBAGENT_CAPABILITY_ID → 48_000`, `builtin.http → 32_000`, `builtin.web_fetch → 32_000`, default fallback 32_000. Sig accepts `&LoopExecutionState` only (no `BudgetState` type exists; v2 `BudgetFractionPolicy` extends sig if #4311 needs more). |
| `crates/ironclaw_loop_support/src/capability_port.rs` | **Widen `LoopCapabilityResultWriter::write_capability_result` return** from `Result<LoopResultRef, AgentLoopHostError>` to `Result<(LoopResultRef, u64), AgentLoopHostError>` (return the byte_len already computed in `StagedCapabilityResult.byte_len` at `product_live_adapters.rs:94,239`). Every impl (`ProductLiveCapabilityIo` at `product_live_adapters.rs:226`, any test fakes) must update. Boundary-acceptable: trait already in `ironclaw_loop_support` adapter glue; return widening is a contract-internal change, not a new persistence trait. **No new field on `LoopResultRef`** — boundary preserved (refs stay opaque per `ironclaw_turns/CLAUDE.md`). |
| `crates/ironclaw_agent_loop/src/executor/capability_helpers.rs` | `push_completed_result` reads the writer-supplied `byte_len`; increments `state.pending_capability_bytes.entry(capability_id).or_insert(0) += byte_len`. |
| `crates/ironclaw_agent_loop/src/executor/post_capability.rs` (new file per crate "one decision axis per file" rule) | `pub struct PostCapabilityStage; impl ExecutorStage<TurnCompletedStep> for PostCapabilityStage`. Process body: (1) `drain_settled` helper — returns empty `Vec` today, no-op until background producers exist (NOT a stub stage; R2 is owner-of-record reserved for #4474 follow-on, lives in this same file); (2) call `CompactionForceStrategy::should_force_compact(&state)`; (3) on Some → set `state.compaction_state.force_compact_on_next_iteration = true` AND `state.skip_model_this_iteration = true`, clear `pending_capability_bytes`, emit a NEW `LoopProgressEvent::CompactionScheduled { initiator: CompactionInitiator::CapabilityResultOverflow }` variant (NOT `CompactionStarted` which exists at `host.rs:1919`); (4) return `TurnCompletedStep` unchanged. Stage docstring documents the dual responsibility per design doc. **Contract-change callout:** adding both `LoopProgressEvent::CompactionScheduled` variant AND `CompactionInitiator::CapabilityResultOverflow` variant lives in `ironclaw_turns` (frozen contract crate per its CLAUDE.md). Treat both as a minor contract-change PR alongside WU-A, OR reuse existing `CompactionStarted` event by adding only the `CompactionInitiator` variant — pick during WU-A scoping. Recommended: reuse `CompactionStarted` to minimize contract surface; add only the `CapabilityResultOverflow` initiator variant. |
| `crates/ironclaw_agent_loop/src/executor/canonical.rs` | Inside the `CapabilityCalls` match arm (`canonical.rs:166` per Agent 1), after `CapabilityStage.process(...)` returns `TurnCompletedStep`, call `PostCapabilityStage.process(ctx, step).await?` before `.stop.observe(...)`. **Do NOT** add `skip_model` branch logic here — branch lives in `PromptStep` variant (next iteration consumes the flag). Per crate CLAUDE.md: "Put lifecycle mechanics in the owning executor stage instead of adding branch logic directly to `canonical.rs`." |
| `crates/ironclaw_agent_loop/src/executor/prompt.rs` | At entry to `PromptStage.process()`: if `state.skip_model_this_iteration` is set, force-run `PromptCompactionStep` (synthesize `Trigger`), clear `skip_model_this_iteration`, return a NEW `PromptStep::SkipModel(LoopExecutionState)` variant. |
| `crates/ironclaw_agent_loop/src/executor.rs` | Add `PromptStep::SkipModel(LoopExecutionState)` variant. `PromptStep` is today a 2-variant enum (`Prepared(Box<PromptOutput>)`, `Exit(LoopExit)`) at `executor/prompt.rs:51`. |
| `crates/ironclaw_agent_loop/src/executor/canonical.rs` (second touch) | At the `PromptStep` match (`canonical.rs:99-101` — only 2 arms today: `Prepared` extracts, `Exit` returns), add `PromptStep::SkipModel(state) =>` arm. **The arm CANNOT "fall through" to `.stop.observe(...)` directly** — the code between the prompt match and stop.observe runs `CheckpointStage::process(BeforeModel)` (`canonical.rs:105`), `pending_input_ack.ack(...)`, then `ModelStage`. Arm must construct a synthetic `TurnCompletedStep` (kind = `TurnEndKind::SkippedModel` or reuse existing `ReplyOnly`) and `continue 'iteration` past the model+capability section, jumping directly to `.stop.observe(...)` at `canonical.rs:205`. Implementation choice: restructure the iteration body so the `SkipModel` path uses an early `let completed_step = TurnCompletedStep::skipped(state);` then `goto`/`continue` to stop observe. Document in PR. |
| `crates/ironclaw_turns/src/run_profile/compaction.rs` (`CompactionInitiator` enum at `:10`, lives in `ironclaw_turns`) | Add `CompactionInitiator::CapabilityResultOverflow` variant. Existing variants today: `Auto`, `Overflow`, `SubagentScoped`. Reuse existing `LoopProgressEvent::CompactionStarted { initiator, .. }` (at `host.rs:1919`) — no new event variant needed. Marked as minor contract-tweak per turns CLAUDE.md (neutral host/runner protocol). |
| `crates/ironclaw_agent_loop/src/executor/tests.rs` | Caller-level tests through `CanonicalAgentLoopExecutor` (per `.claude/rules/testing.md` "Test Through the Caller"): (1) byte threshold trips → both flags set, `CompactionScheduled` event emitted; (2) under threshold → flags untouched; (3) `PromptStep::SkipModel` route bypasses ModelStage AND runs compaction first; (4) 3-call batch with one oversize → trips once, byte map cleared after; (5) per-cap byte map accumulates across calls within a turn. |

**Build sequence inside WU-A:**

1. Add `pending_capability_bytes` + `skip_model_this_iteration` slots.
2. Land `CompactionForceStrategy` trait + `ByteCapStrategy` impl with unit tests (pure function).
3. Wire `push_completed_result` to update `pending_capability_bytes`.
4. Add `PromptStep::SkipModel` variant; teach `prompt.rs` to emit it; teach `canonical.rs` to route it.
5. Implement `PostCapabilityStage`; wire into `canonical.rs` after Capability match arm.
6. Add 4 executor tests above.
7. `cargo fmt && cargo clippy --all --benches --tests --examples --all-features && cargo test`.

**Deliberately out of scope for WU-A:**
- Removing `ShrinkContext` reactive recovery path. Leave intact; let proactive path land first; deprecate in a follow-up.
- Any change to `LoopResultRef` definition (writer already supplies `byte_len`; ref stays opaque per turns CLAUDE.md).
- Budget-fraction policy (#4311 dep).

**Byte size source — confirmed not a perf concern.** `StagedCapabilityResult.byte_len` (`product_live_adapters.rs:94`) already computed via `serialized_json_len()` at write time (`:239`). Thread through `LoopCapabilityResultWriter` return. Zero re-fetch.

**Estimate:** 1 dev-week (incl. tests + review).

---

### WU-B — Durability schema + parity sub-spec (gate for WU-C)

Goal: written sub-spec before any durable store code lands. Per repo rule (`.claude/rules/database.md` — every persistence feature supports libsql AND postgres). Path: `docs/reborn/2026-06-XX-subagent-durability-spec.md`.

**Must cover:**

| Section | Content |
|---|---|
| Stores in scope | (1) gate resolution (3 denormalized maps, atomic migration); (2) goal store; (3) capability result store (+ introduce `CapabilityResultStore` trait); (4) **tombstone store** (omitted from design doc). |
| Trait introductions | `CapabilityResultStore` — methods, error model. `SubagentRestartReconciler` — concrete trait shape (currently only a stub enum member). |
| Schema per store | Tables, columns, indexes, FK shape for libsql AND postgres. Settlement event log table for `RestartReconciler` replay. Idempotency ledger table keyed on `(run_id, child_run_id, terminal_kind)`. |
| Concurrent settlement | `INSERT OR IGNORE` / `ON CONFLICT DO NOTHING` — match in-memory first-writer-wins. Document explicitly. |
| Migration of in-flight RAM state at deploy | Either accept loss (background mode behind toggle stays off across deploys until E2E green) OR drain + rehydrate. **Recommend accept loss** — feature toggle gates the user impact. |
| Rollback | Empty tables safe to leave; in-memory paths remain fallback; reconciler against empty store is no-op. |
| Reborn-side DB layer convention | Where Reborn-crate persistence code lives (no precedent today). Recommend: new `crates/ironclaw_reborn_persistence/` with `libsql/` and `postgres/` subdirs matching v1 `src/db/` shape. |
| Dual-backend parity test | Shape of the parity test (#4431). |

**Reviewers:** persistence ownership + Reborn ownership. Sub-spec lands as its own PR.

**Estimate:** 3–4 days incl. review iteration.

---

### WU-C — Durable stores impl (blocked by WU-B)

Goal: replace four in-memory stores with libsql + postgres backends. Land `RestartReconciler` + idempotency ledger.

**Files (new) — REVISED per Reviewer 1 V1 + Reviewer 4 G2:**

Do NOT create a new `ironclaw_reborn_persistence` crate. Two compliance issues:

1. **Boundary-test rule (Reviewer 1 V1):** every active Reborn crate needs a `BoundaryRule` entry in `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`. New crate without rule = silent dep escape.
2. **Direction-of-travel (Reviewer 4 G2):** `.claude/rules/database.md` direction is `ScopedFilesystem` for new persistence; legacy `src/db/`-style dual-backend per-crate is reserved for fixing existing code. Pivoting to a new dual-backend crate contradicts that direction.

**Owner:** `crates/ironclaw_reborn_event_store/` — canonical owner of Reborn durable backend selection per `events.md` §2 and `ironclaw_events/CLAUDE.md`. Existing boundary rule covers it.

**Sub-spec (WU-B) must decide:**

- (a) Are the 4 stores best modeled as `ScopedFilesystem`-backed (durable on libsql+postgres root-filesystem backends) per `database.md` direction? OR
- (b) Are they "structured/query-heavy/security/control-plane state" (per `_contract-freeze-index.md` §2 storage model) and thus get typed repositories?

Goal/tombstone/idempotency-ledger likely fit (a) (file-shaped or key-value). Gate resolution (3 denormalized maps) and capability result store (high write rate) likely fit (b). Sub-spec must justify per-store choice + name resulting file locations:

- (a) → `crates/ironclaw_reborn_event_store/src/{gate_resolution_fs.rs, goal_store_fs.rs, ...}` using `RootFilesystem` adapter
- (b) → `crates/ironclaw_reborn_event_store/src/{libsql/, postgres/}` for typed repos that need indexes/atomicity

**Files modified (regardless of choice):**

- `crates/ironclaw_loop_support/src/capability_port.rs` — **do NOT introduce `CapabilityResultStore` trait here** (Reviewer 1 R2: loop_support is adapter glue, not persistence). Introduce in `ironclaw_reborn_event_store` (or its own `ironclaw_loop_capability_store` if needs to be reusable above adapter layer).
- `crates/ironclaw_reborn_composition/src/product_live_adapters.rs` — switch backend choice via config; keep in-memory as `local_dev` fallback only.
- `crates/ironclaw_reborn_composition/src/runtime/local_dev.rs` — keep in-memory for dev; wire production-readiness check to require non-in-memory in prod.
- `crates/ironclaw_reborn/src/composition/production_readiness.rs` — flip `SubagentRestartReconciler` from stub to required impl present.

**Schema requirement (Reviewer 4 B2):** every durable table/file gets explicit `agent_id` column/field (nullable for non-agent runs) per `_contract-freeze-index.md` §2 + §8. Index on `(tenant_id, user_id, agent_id, ...)` for scoped queries.

**Tests:** per-store CRUD + idempotency tests, dual-backend parity test, reconciler replay test (write settlement, drop in-memory state, boot, observe replay).

**Estimate:** 2 dev-weeks.

---

### WU-D — Background mode re-enable (blocked by WU-C)

Goal: model can request background subagents; spawn port accepts them.

**Files:**

| File | Change |
|---|---|
| `crates/ironclaw_loop_support/src/subagent_spawn_port.rs` | Lines ~183-188: remove the two `background_subagents_disabled()` returns. Add field `mode: SpawnSubagentMode` to `SpawnSubagentArgs` (line 152). Carry `mode` from `SpawnSubagentWireArgs` through `TryFrom`. `SubagentDefinition` (line 207): add `background_policy: BackgroundPolicy` (enum: `Denied`, `Allowed { max_depth: u32, max_concurrent: u32 }`). |
| `crates/ironclaw_host_runtime/src/first_party_tools/spawn_subagent.rs` (`spawn_subagent_parameters_schema()` around line 48) | Add `mode: { type: "string", enum: ["blocking", "background"], default: "blocking" }` property. |
| `crates/ironclaw_loop_support/src/subagent_spawn_port.rs` (spawn body, around line 590 depth check) | Add concurrent-background-children counter check against `BackgroundPolicy::Allowed.max_concurrent`. Counter lives on a new per-parent state slot (or computed from goal store on demand). Mailbox admission cap: refuse new background spawns when pending mailbox is full. |
| `crates/ironclaw_loop_support/src/subagent_spawn_port.rs` (`finish_spawn` ~line 863) | For background mode, return `CapabilityOutcome::SpawnedChildRun { child_run_id, result_ref, safe_summary }` (existing variant, not `Completed`) instead of `AwaitDependentRun`. `AwaitDependentRunGateStage` is bypassed automatically. |
| `crates/ironclaw_reborn/src/subagent/completion_observer.rs` | Existing Background branch (line 180-183) is already wired through `mark_child_deliveries`. Confirm it still works with durable result store; add idempotency-ledger write. |
| New config knob | `ironclaw_reborn` config: `subagent.background_enabled: bool` (default false). Gate the wire-args acceptance. |

**Tests:** background spawn happy path; background spawn denied when policy = `Denied`; depth cap honored; concurrent cap honored; mailbox admission cap honored; restart between spawn and settlement → reconciler delivers result.

**Estimate:** 1 dev-week.

---

### WU-E — LoopBackgroundChildPort + drain_settled (Responsibility 2 fill, blocked by WU-C + WU-D)

Goal: parent loop drains settled background children into `state.result_refs`.

**Files:**

- `crates/ironclaw_turns/src/run_profile/host.rs` — add new trait `LoopBackgroundChildPort { fn drain_settled(&self, parent: TurnRunId) -> Vec<SettledChild>; }`. Add as supertrait of `AgentLoopDriverHost`. Total supertrait count goes from 10+Send+Sync to 11+Send+Sync.
- `crates/ironclaw_loop_support/src/background_child_port.rs` (new) — `SettledChild` type, default impl backed by the durable capability result store + settlement log.
- `crates/ironclaw_agent_loop/src/executor/post_capability.rs` (or wherever WU-A landed `PostCapabilityStage`) — fill the no-op stub:
  1. Call `drain_settled(parent_run_id)`.
  2. Apply per-iteration drain cap (e.g. `MAX_DRAIN_PER_ITERATION = 8`); carry overflow to next iteration.
  3. For each `SettledChild`: hydrate `LoopResultRef`, call existing `push_completed_result` chain so byte accounting + compaction trigger work uniformly.
  4. Emit `LoopProgressEvent::BackgroundChildSettled { child_run_id, status }`.
  5. Inject `CapabilityOutcome::Failed` for failures (surfaces via `LoopFailureKind`, #4427).
- **Ordering inside `PostCapabilityStage`:** drain FIRST, then policy check. Already documented in design doc, just enforce in tests.

**Tests:** N settled children → all drained within cap; overflow carries; failure variant surfaces correctly; partial fan-out (3/5 fail) continues parent.

**Estimate:** 1 dev-week.

---

### WU-F — WebUI parent-child nesting (blocked by WU-C projection-ready artifacts)

Goal: WebUI shows runs nested under parent with depth indentation + background status badge.

**Approach decision: projection-time JOIN, NOT event-schema fields.** Reasoning (contract + boundary):
- **Contract:** `docs/reborn/contracts/events.md` §2 publishes `RuntimeEvent` shape as a frozen contract. Adding fields = contract-change request per `_contract-freeze-index.md` §1, not implementation work. JOIN avoids busting freeze.
- **Boundary precedent explicit:** `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs:1996` documents `ironclaw_event_projections` IS permitted to depend on `ironclaw_turns` for `TurnLifecycleEvent`-derived read models (existing example: `PendingGateProjection`). Subagent lineage is the same pattern.
- **Metadata-only rule satisfied:** `parent_run_id, subagent_depth, spawn_tree_root_run_id` are lifecycle metadata on `TurnRunRecord` (`store.rs:189-193`), not raw payload — projection `CLAUDE.md` rule preserved.
- **Source of truth single-owner:** `TurnRunRecord` owns lineage. Schema-fields would duplicate, drift risk.
- **Blast radius minimal:** zero `RuntimeEvent` changes, zero replay/durable-log fixture changes, zero Wire compat burden.

**Files:**

| File | Change |
|---|---|
| `crates/ironclaw_event_projections/src/lib.rs:264` (`RunStatusProjection`) | Add `parent_run_id: Option<TurnRunId>`, `subagent_depth: u32`, `tree_root_run_id: Option<TurnRunId>`. |
| `crates/ironclaw_event_projections/src/runtime_projection.rs:157` (`apply_run_event`) | **Do NOT do the JOIN here.** `apply_run_event` is `fn` (sync), not `async`; calling an async store read violates the deterministic-reducer rule (`events-projections.md §5`). Leave the 3 new projection fields as `None` here. JOIN happens downstream in composition. |
| `crates/ironclaw_product_adapters/src/outbound.rs:638` (`ProductProjectionItem::RunStatus`) | Add 3 fields with `#[serde(skip_serializing_if = "Option::is_none")]`. |
| `crates/ironclaw_reborn_composition/src/projection.rs:967` (`run_status_projection_state`) | Inject `Arc<dyn TurnSpawnTreeStateStore>` (the trait that actually owns `get_run_record(&scope, run_id) -> async Result<Option<TurnRunRecord>>` at `store.rs:95`, NOT `TurnCoordinator` which has only `get_run_state`). At this call site, do `let record = store.get_run_record(&scope, run_id).await.ok().flatten();` and populate the 3 new projection fields from `record.parent_run_id`, `record.subagent_depth`, `record.spawn_tree_root_run_id`. Stale-parent race: `None` is graceful default (FE ignores `None` via `serde skip`). |
| `crates/ironclaw_reborn_composition/src/live_progress.rs:147` (`product_items_for_live_update`) | **Currently has no `RunStatus` arm** — handles `Thinking`, `CapabilityActivity`, `WorkSummary`, `SkillActivation` only. Adding nested-subagent lineage to live updates requires a NEW `RunStatus` arm. Scope decision: defer live-update lineage to a follow-up; WU-F ships only the snapshot/replay path through `run_status_projection_state`. FE renders lineage on full timeline reload, not on every live SSE tick. Reduces WU-F blast radius. |
| `crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/useChatEvents.js:252` | Destructure new fields. Render runs grouped by `tree_root_run_id`, indented by `subagent_depth`. Background runs get distinct status badge (running, settled, failed). |
| FE component (TBD: run-list renderer) | Indent + badge UI. |

**Tests:** projection includes lineage when set; projection omits when null (backward compat); FE renders nested tree; FE handles missing fields gracefully.

**Estimate:** 1 dev-week (BE ~3 days, FE ~2 days).

---

### WU-G — Parity test (#4431 follow-on)

Already opened. Land after WU-C/D/E.

---

### Cross-cutting

- **Feature toggle:** `subagent.background_enabled` defaults to `false` until WU-G passes E2E green. Doc requirement.
- **Tracing:** `LoopProgressEvent::CompactionScheduled` + new `BackgroundChildSettled` cover #4427 trace gap for the new code paths.
- **Backward compat:** All new projection fields optional + `serde(skip_serializing_if = "Option::is_none")`. FE ignores unknown keys.
- **`ShrinkContext` reactive path:** leave intact across all WUs; deprecate in a follow-up PR after Step 1 lands and proactive path proves out in metrics.

### Test matrix (full)

From design doc, with adjustments:

| Test | WU | Notes |
|---|---|---|
| Responsibility 1 compaction-on-overflow (single result + batch with one oversize) | WU-A | Add batch variant — doc missed it. |
| `SkipModel` short-circuit (compact-this-turn) | WU-A | New — doc proposed mechanism wrong. |
| Restart-between-spawn-and-settle (reconciler replay) | WU-C+D | Requires durable store. |
| Duplicate settlement delivery (idempotency ledger) | WU-C | Insert-or-ignore semantics. |
| Responsibility 2 settlement injection | WU-E | |
| Per-iteration drain cap | WU-E | |
| Partial fan-out continues parent | WU-E | 3/5 fail; parent continues. |
| Background spawn denied by policy | WU-D | |
| Depth cap enforced | WU-D | Cap already exists (max_depth=1). |
| Concurrent cap enforced | WU-D | New. |
| E2E background fan-out happy path | WU-G | Feature-toggle on. |
| Dual-backend parity (libsql vs postgres) | WU-G | #4431. |

### Issue mapping

Identical to design doc Issue map. No corrections. Add: WU-B sub-spec lands as a new doc-only PR under #4474.

### Closing criteria

- WU-A merges independently; ships proactive compaction immediately.
- WU-B sub-spec merged before WU-C opens.
- WU-C/D/E/F merged with feature toggle defaulting OFF.
- WU-G E2E + parity test green → toggle flips ON; documented contract under `docs/reborn/`.
- Follow-up PR removes `ShrinkContext` reactive recovery path once proactive path proves out.

---

## Part 3 — Decisions ratified after boundary/ownership check

Verified against `docs/reborn/contracts/_contract-freeze-index.md`, `events.md`, `events-projections.md`, crate-local `CLAUDE.md` for `ironclaw_agent_loop`, `ironclaw_events`, `ironclaw_event_projections`, `ironclaw_turns`, `ironclaw_loop_support`, `ironclaw_product_adapters`, `ironclaw_reborn_composition`, and `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`.

| # | Decision | Verdict | Reason |
|---|---|---|---|
| 1 | `PromptStep::SkipModel` variant (typed branch via `match`, not state-flag check in `canonical.rs`) | LOCKED IN | agent_loop CLAUDE.md: lifecycle mechanics owned by the stage; strategies return outcomes, not by-ref mutation. |
| 2 | WebUI nesting via projection-time JOIN against `TurnRunRecord` (NOT new `RuntimeEvent` fields) | LOCKED IN | Schema-fields = contract change per freeze index §1. JOIN explicitly permitted: `reborn_dependency_boundaries.rs:1996` allows `ironclaw_event_projections → ironclaw_turns`. Precedent: `PendingGateProjection`. |
| 3 | `PostCapabilityStage` single stage, R2 = owner-of-record (NOT "stub") | LOCKED IN | Reframed: R2 is the seam owner for #4474 follow-on, not a placeholder. Avoids "no stub stages" rule. R1 is the active responsibility. |
| 4 | `pending_capability_bytes: BTreeMap<CapabilityId, u64>` | LOCKED IN | agent_loop already depends on `ironclaw_host_api`; `CapabilityId` already used in `state.rs:169`. Counter map = allowed per CLAUDE.md. Future per-cap policies (`BudgetFractionPolicy`) consume it. |
| 5 | Reuse existing `StagedCapabilityResult.byte_len`; thread through `LoopCapabilityResultWriter`. Do NOT modify `LoopResultRef`. | REPLACED | `byte_len` already computed at write time (`product_live_adapters.rs:94,239`). Adding field to `LoopResultRef` muddies "refs vs metadata" boundary (turns CLAUDE.md). |

### Cross-cutting boundary compliance

- **`ironclaw_agent_loop` deps unchanged.** No imports from `ironclaw_reborn`, host_runtime, dispatcher, network, secrets, DB backends. `CompactionForceStrategy` trait + `ByteCapStrategy` impl stay crate-private. New file `executor/post_capability.rs` follows "one decision axis per file" rule.
- **`ironclaw_turns` gains `CompactionInitiator::CapabilityResultOverflow` variant only.** No new ports. Stays neutral host/runner protocol per turns CLAUDE.md.
- **`ironclaw_loop_support` writer return widened to surface existing `byte_len`.** Adapter glue per crate CLAUDE.md; no new ports, no dispatcher bypass.
- **`ironclaw_events`, `ironclaw_event_projections`, `ironclaw_product_adapters`:** zero schema changes from WU-A. WU-F adds 3 optional fields to `RunStatusProjection` + `ProductProjectionItem::RunStatus`, all `#[serde(skip_serializing_if = "Option::is_none")]`. JOIN lookup via `TurnSpawnTreeStateStore::get_run_record(&scope, run_id)` per `events-projections.md` §5 (reducer rules: deterministic, side-effect free, rebuildable).
- **`ironclaw_reborn_composition`:** `run_status_projection_state` (`projection.rs:967`) + `product_items_for_live_update` (`live_progress.rs:147`) gain the JOIN. Stays facade-shaped per composition CLAUDE.md (no new substrate exposure).

### Testing discipline per `.claude/rules/testing.md` "Test Through the Caller"

- WU-A: `CompactionPolicy::should_force_compact` gates a state-mutation side effect (force_compact flag + skip_model flag + progress event). Helper unit tests NOT sufficient. Required: caller-level executor tests via `CanonicalAgentLoopExecutor` covering the byte-trip → flag-set → next-iteration-compaction → SkipModel-bypass full path.
- WU-D: `BackgroundPolicy::Allowed { max_depth, max_concurrent }` gates spawn admission. Required: caller-level test through `SpawnSubagentPort::spawn(...)` not just policy predicate unit tests.
- WU-F: projection JOIN gates UI-visible behavior. Required: caller-level test through `EventStreamManager` resume path with nested run lineage.

### Architecture boundary verification step (every WU)

Run after touching dependencies, public APIs, or facade shape:

```bash
cargo test -p ironclaw_architecture
```

This catches: forbidden deps slipping in, public prelude widening, facade shape drift, projection→backend dep additions.

### Outstanding contract-change requests (REVISED per Reviewer 1 V3)

**WU-A — minor contract tweak:** Adding `CompactionInitiator::CapabilityResultOverflow` variant in `ironclaw_turns` is a contract addition. Treat as same-PR contract note in `docs/reborn/contracts/turns-agent-loop.md` (or `loop-exit.md`). Reuses existing `LoopProgressEvent::CompactionStarted` event — no new event variant.

**WU-D — CONTRACT CHANGE REQUEST, not implementation:** `SubagentDefinition` lives in `ironclaw_loop_support` (`subagent_spawn_port.rs:207`) and is pub-re-exported from `ironclaw_loop_support/src/lib.rs:91` — published surface. Adding `BackgroundPolicy` enum field is a contract change per `_contract-freeze-index.md` §1. **Must land a contract-change request PR BEFORE WU-D opens.** Likely targets: `docs/reborn/contracts/turns-agent-loop.md` (subagent-spawn section) or a new `subagent-lifecycle.md` packet entry.

**WU-B — doc-only PR** (the sub-spec itself).

---

## Part 4 — Reviewer-driven revisions (4 parallel reviewers, 2026-06-07)

### Blockers fixed in this revision

| # | Blocker | Source | Fix applied |
|---|---|---|---|
| WU-A.1 | `LoopProgressEvent::CompactionScheduled` doesn't exist — only `CompactionStarted` | Reviewer 2 B2 | Reuse `CompactionStarted`; add only `CompactionInitiator::CapabilityResultOverflow` |
| WU-A.2 | `PromptStep::SkipModel` cannot "fall through" — `canonical.rs` runs Checkpoint+Ack+Model after the prompt match | Reviewer 2 B3 | Spelled out: arm constructs synthetic `TurnCompletedStep` and jumps to `.stop.observe(...)` at `:205` |
| WU-A.3 | `LoopCapabilityResultWriter::write_capability_result` returns `LoopResultRef` only — widening to `(LoopResultRef, u64)` is breaking trait change | Reviewer 2 B1 | Made explicit; every impl touched (`ProductLiveCapabilityIo` + test fakes) |
| WU-A.4 | `HashMap<CapabilityId, u64>` breaks deterministic replay | Reviewer 2 D2 | Switched to `BTreeMap<CapabilityId, u64>` matching `RecoveryStrategyState.attempts_by_class` precedent |
| WU-A.5 | Per-cap accumulate vs clear timing ambiguous | Reviewer 2 D1 | Accumulate per call in `push_completed_result`; `PostCapabilityStage` clears whole map after deciding to trip (batch finishes before stage fires) |
| WU-A.6 | `ironclaw_agent_loop` boundary rule absent from `boundary_rules()` | Reviewer 1 R3 | Add `BoundaryRule` entry for `ironclaw_agent_loop` to `reborn_dependency_boundaries.rs` as part of WU-A |
| WU-A.7 | `CompactionInitiator` addition IS a contract change in `ironclaw_turns` | Reviewer 2 B2 | Called out in "Outstanding contract-change requests" above |
| WU-C.1 | New `ironclaw_reborn_persistence` crate has no boundary-test rule + contradicts `database.md` direction | Reviewer 1 V1 + Reviewer 4 G2 | Pivoted to `ironclaw_reborn_event_store` (canonical owner per `events.md` §2); WU-B sub-spec decides per-store ScopedFilesystem vs typed-repo per `_contract-freeze-index.md` §2 storage model |
| WU-C.2 | `CapabilityResultStore` trait wrongly placed in `ironclaw_loop_support` (adapter glue, not persistence) | Reviewer 1 R2 | Move trait to `ironclaw_reborn_event_store` |
| WU-D.1 | `BackgroundPolicy` on `SubagentDefinition` = contract change, not impl | Reviewer 1 V3 | Now an explicit prerequisite contract-change PR before WU-D opens |
| WU-F.1 | `TurnCoordinator::get_run_record` does NOT exist — method lives on `TurnSpawnTreeStateStore::get_run_record(&scope, run_id)` | Reviewer 3 finding 1 | Updated plan to inject `Arc<dyn TurnSpawnTreeStateStore>` and call correct sig |
| WU-F.2 | `apply_run_event` is sync — async JOIN there violates deterministic-reducer rule | Reviewer 3 finding 2 | JOIN moved to composition layer (`run_status_projection_state` is already async-capable) |
| WU-F.3 | `live_progress.rs:147` has no `RunStatus` arm today | Reviewer 3 finding 6 | WU-F scoped to snapshot/replay path only; live-update lineage deferred to follow-up |
| WU-C.3 | Missing `agent_id` per-row schema (contract-freeze §2 + §8 scope propagation) | Reviewer 4 B2 | Mandated in WU-C; sub-spec (WU-B) must specify scope columns/fields + indexes |

### Gaps tracked (need addressing during WU scoping, not blockers)

- **Caller-level tests for WU-C + WU-E** (Reviewer 4 B1). WU-C: drive `SubagentRestartReconciler::replay` through the composition boot path. WU-E: drive `drain_settled` through `CanonicalAgentLoopExecutor` with a live `LoopBackgroundChildPort` mock.
- **`cargo doc` missing from verification commands** (Reviewer 4 B3). Add to every WU's evidence list per `_contract-freeze-index.md` §9.8.
- **Feature toggle exact location** (Reviewer 4 G1). Decide during WU-D scoping: env var name + config struct field (likely `RebornCompositionConfig.subagent.background_enabled` or similar).
- **Logging discipline for background paths** (Reviewer 4 G3). `PostCapabilityStage`, `SubagentRestartReconciler::replay`, `completion_observer.rs` background branch must use `debug!` not `info!` per `CLAUDE.md`.
- **Migration rollback note for toggle-flip-back** (Reviewer 4 G4). Document what happens to durable rows written while toggle was ON if toggle flips back OFF (recommend: leave rows in place; in-memory paths re-activate; reconciler runs as no-op).
- **E2E test placement spec** (Reviewer 4 G5). WU-G test goes at `tests/e2e/scenarios/test_subagent_background.py` per `tests/e2e/CLAUDE.md`; `asyncio_mode = "auto"`; ironclaw_server fixture must inject `IRONCLAW_SUBAGENT_BACKGROUND_ENABLED=true` env.
- **Stale-parent race fallback for WU-F JOIN** (Reviewer 3 finding 7). `None` is graceful default; document explicitly. FE handles missing fields via `#[serde(skip)]` + JS optional-chaining.
- **`LoopBackgroundChildPort` as 11th `AgentLoopDriverHost` supertrait** (Reviewer 1 R1). Acceptable boundary risk — port is mailbox-shaped, no strategy-specific payload. Document scoping rationale in WU-E PR.

### Items confirmed CLEAN by reviewers

- Item 1 — `PromptStep::SkipModel` variant approach (typed `match` exhaustiveness)
- Item 2 — JOIN over schema-fields (contract freeze + explicit boundary precedent)
- Item 3 — single `PostCapabilityStage` with owner-of-record R2 framing
- Item 4 — `pending_capability_bytes` map (switched to `BTreeMap` per D2)
- Item 5 — reuse existing `StagedCapabilityResult.byte_len`; no `LoopResultRef` change
- WU-A agent_loop dep boundary unchanged (Reviewer 1)
- `CapabilityResultOverflow` reason field redaction-safe (Reviewer 4)
- `ShrinkContext` preservation correct incremental approach (Reviewer 4)
- All new projection fields wire-additive with `#[serde(skip_serializing_if)]` (Reviewer 3 + Reviewer 4)
- `INSERT OR IGNORE` settlement semantics correctly match first-writer-wins in-memory (Reviewer 4)
- No `unwrap()`/`expect()` patterns, no prompt templates in Rust code (Reviewer 4)
- Tombstone store inclusion (was missing from design doc; plan fixes it) (Reviewer 4)

### Revised WU-A merge gate

Before WU-A opens its PR:
1. Apply trait-widening to `LoopCapabilityResultWriter` (BLOCKER 3).
2. `BTreeMap` not `HashMap` (BLOCKER 4).
3. `canonical.rs` SkipModel arm builds synthetic `TurnCompletedStep` (BLOCKER 2).
4. Add `BoundaryRule` for `ironclaw_agent_loop` in same PR (BLOCKER 6).
5. Same-PR contract note for `CompactionInitiator::CapabilityResultOverflow`.
6. Caller-level tests pass `cargo test -p ironclaw_agent_loop`; boundary test passes `cargo test -p ironclaw_architecture`.

WU-F merge gate: snapshot-path only this round; live-update path follow-up. `TurnSpawnTreeStateStore` injected at composition, not projections crate.

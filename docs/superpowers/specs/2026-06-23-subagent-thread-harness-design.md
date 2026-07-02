# Subagent thread harness — foundational design

Date: 2026-06-23.
Status: design doc, and the **single source of truth** for the durable
subagent delivery/thread design. Supersedes and **replaces** the former
subagent-durability sub-spec (`docs/reborn/2026-06-08-subagent-durability-spec.md`,
removed in this change) and the WU-C implementation it described. Note: the
shipped spawn mechanics (`docs/reborn/subagent-spawn/`), compaction
(`docs/reborn/2026-06-04-subagent-compaction-design.md`), and planner
(`docs/superpowers/specs/2026-06-08-planner-subagent-design.md`) docs remain —
they describe live code this design builds on, not the superseded delivery
layer. Scope: **Reborn only** (no legacy ironclaw /
engine v1 / v2 reuse — every component named here lives under
`crates/ironclaw_reborn*`, `crates/ironclaw_turns`, `crates/ironclaw_threads`,
`crates/ironclaw_filesystem`, `crates/ironclaw_loop_support`,
`crates/ironclaw_host_runtime`).

Closes PR #4656 (`wu-c2-gate-resolution-store`): its durable gate-resolution
stack is replaced wholesale by the design below, not salvaged.

## 1. Why this exists

The prior durability sub-spec modelled a subagent as a *one-shot subordinate
computation*: spawn → settle once → deliver a result to the parent → tombstone
(dead). Four product requirements break that model:

1. A parent agent must be able to inspect a child's work **at any time**, not
   just read terminal metadata.
2. A human must be able to **open a child's thread in the console and continue
   conversing with it** interactively.
3. A parent must be able to **re-activate a finished child with an extension
   task**, keeping the child's prior context (no cold start).
4. We have a clean slate and want a best-in-class subagent harness, informed by
   how comparable systems (Claude Code/Codex subagents, LangGraph, Temporal,
   Devin, OpenAI Assistants) implement these capabilities.

These collectively reframe a subagent from a computation into a **persistent,
addressable, resumable conversation thread** that both the parent agent and a
human can attach to. This document defines that model and the durable delivery
substrate underneath it.

## 2. Decisions ratified

| # | Decision |
|---|----------|
| 1 | A child subagent **is a first-class thread** — same kind and same thread store as a top-level thread, plus a lineage header (parent edge). Reqs 1–3 reduce to ordinary thread operations. |
| 2 | **Terminal applies to a *run*, not a thread.** A thread holds a sequence of runs sharing one transcript. A run reaches terminal status and the thread *parks*; it never dies. *Existing infrastructure:* Reborn already separates `TurnRunId` from `TurnScope.thread_id`, and a thread already holds multiple run records (`TurnSpawnTreeStateStore::children_of`). *New in this design:* the thread-level lifecycle (`Active`/`Parked`/`Archived`), the `Parked` resting state, and the `activate` primitive — none exist today. |
| 3 | **Re-activation is one primitive:** `activate(thread, input, provenance)` starts a new run on the same thread, with the prior transcript as context. Called by the parent agent (`subagent_extend`) or by a human (console message). |
| 4 | **One run at a time per thread.** `activate` on a thread that is not fully `Parked` returns `TurnError::ThreadBusy` (the existing thread-conflict error at the turns layer — not the product-workflow `RejectedBusy`); the caller decides what to do. No queue. (`Active` is inclusive — see §4.) |
| 5 | **Delivery is decoupled from lifecycle.** "Parent run P awaits child run C" is a separate, thin record (the *await-edge*). A run may have zero or one await-edge. No edge → the result simply rests in the thread. |
| 6 | The child's **own durable run record is the single source of truth** for terminal status, read via `get_run_state(GetRunStateRequest { scope, run_id })` → `TurnRunState`. No separate settlement log duplicating it. |
| 7 | The await-edge is stored on **`ScopedFilesystem`** (one file per edge), alongside the goal and tombstone stores. No SQL tables, no DDL, no bespoke migration. |
| 8 | **Idempotent delivery is the await-edge CAS itself**, not inherited from any prior store. Each transition (`open→settled`, `settled→drained`, `open→abandoned`) is a single-winner compare-and-swap on one file; a duplicate attempt reads the current state and no-ops. The old in-memory `record_child_terminal` skip-if-set lived in the store being deleted; this design re-implements the guarantee as the edge CAS. |
| 9 | **Capacity** is a `list_dir` count of open edges per scope against a single cap constant. No sharded counter. |
| 10 | **Safety boundary is an edge, not a time.** Every child→parent-agent content crossing is framed-as-untrusted and byte-capped (cheap tier). The child→human console crossing is raw (the human owns it). |
| 11 | **Approvals always bubble up the lineage to a human.** No parent LLM and no child LLM ever auto-approves a child's sensitive action. This is the catastrophe backstop, extending the never-auto-approve hard floor (PR #4959). |
| 12 | **No semantic injection scanning.** Approval-bubbling (11) is the real defense; framing (10) prevents the parent confusing data for instructions. Expensive content sanitization is explicitly not done. |
| 13 | **Provenance** (`Human` vs `ParentAgent`) is tagged on every input into a child, used for **audit + UI routing only** — not as an authorization decision (11 covers authorization). |
| 14 | **A spawn depth floor is enforced from PR 1.** `spawn`/`activate` is refused when `lineage.depth >= max_depth` (configurable; conservative default). Reuses the existing `TurnRunRecord.subagent_depth` + `SubmitChildRunRequest.spawn_tree_descendant_cap`. Full tool-attenuation at max depth is deferred (§10), but the hard stop is not. |
| 15 | **When a parent run reaches terminal before a child's edge is settled, the edge is closed as `abandoned`** by the resolver. The child run is **not** auto-cancelled — it continues, its result rests in its thread, and a human may still open/extend it (req 2). Auto-cancel-on-parent-death is a future policy, not the default. |
| 16 | **Rollout is a vertical walking skeleton** (Option A, §9), behind a new `subagent.v2_enabled` flag (to be provisioned via the `ironclaw_reborn_composition` config pattern — does not exist yet). |

## 3. Architecture

Two layers. Layer 2 is the spine; Layer 1 is a thin delivery edge.

```
 Layer 2 — Subagent threads (the foundation; the thread lifecycle is NEW code)
   SubagentThread = a normal Turn thread + lineage header
     thread_id (first-class)         parent_run_id, tree_root_run_id, depth
     lifecycle: Active | Parked | Archived(future)        flavor/role
   runs within the thread (terminal is PER-RUN):
     run#1 -> terminal -> park ; run#2 (extend) -> terminal -> park ; ...

 Layer 1 — Await-edge delivery (replaces #4656's whole stack)
   one ScopedFilesystem file per "parent run P awaits child run C"
   resolver: child run record = source of truth
     on child-terminal notify OR on boot -> settle open edges -> parent resumes/drains
```

### Core objects

- **`SubagentThread`** — a Turn thread with a lineage header. Lives in the
  existing thread store. Makes reqs 1–3 native: inspect = read it, human-attach
  = open it, extend = new run on it. The lineage fields (`parent_run_id`,
  `subagent_depth`, `spawn_tree_root_run_id`) already exist on `TurnRunRecord`;
  the `SubagentThread` entity and its lifecycle states are new.
- **Await-edge** — one ScopedFilesystem file identified *by its path*
  (`<parent_run_id>/<child_run_id>`). Payload: `{ child_thread_id, mode, state,
  terminal_kind?, created_at, settled_at?, closed_at? }`. It is the await-record
  and the parent's delivery signal. It carries **no** result pointer — the
  parent reads the child's actual output from the child run record/transcript by
  `child_run_id` on drain. (`gate_ref` and `result_ref` from the old design are
  intentionally gone: the path is the identity, and `child_run_id` is the result
  locator.)
- **Resolver** — the only recovery mechanism. On a child-terminal event (live)
  or at boot (recovery), it settles open edges by reading the child's run
  record. Idempotent via compare-and-swap on one file.

Two modes differ only in *who consumes the settled edge*:
- **blocking** — the parent run is suspended (a `Blocked` `TurnStatus`) until its
  edge(s) settle.
- **background** — the parent continues; it drains the settled edge later at
  `PostCapabilityStage`.

### Blocking wake mechanism

A blocking parent run is parked by the scheduler in
`TurnStatus::BlockedDependentRun` — **reused, not extended.** The blocked-status
model is gate-ref-centric: every `BlockedReason` carries a `gate_ref`, and ~5
sites (`events.rs`, `lifecycle.rs`, `completion_observer.rs`, `memory/mod.rs`,
`request.rs`) map blocked status ↔ gate-ref reason. The await-edge supplies a
**real** `GateRef` that names the edge (e.g. `gate:subagent-await:<child_run_id>`)
— not a synthetic placeholder, but a genuine reference to the awaited edge — so
`BlockedReason::AwaitDependentRun { gate_ref }` is accurate and the entire
existing block/wake machinery is reused unchanged. (An earlier draft proposed a
new gate-ref-less `BlockedAwaitEdge` variant; rejected — it fights the gate-ref
model, ripples into all 5 sites, and a subagent wait is, to the user and model,
just another dependent-run wait. Consistency wins.) When the resolver CAS-settles
an edge, it signals the scheduler via the existing `SchedulerTurnRunWakeNotifier`
(PR #5085) to re-queue the parent run; the resumed parent lists its now-settled
edges and drains them.

**Boot recovery of a blocking parent:** on restart the parent run is still
`Blocked` in durable run state. The resolver (driven by active-run enumeration,
§6) settles any terminal edges; the parent run, when re-queued, finds its edges
already `settled` and proceeds immediately — it does not wait for a live wakeup
that was lost with the crashed process. If an edge is still `open` (child not
terminal), the parent stays `Blocked` and is woken later by the live path.

## 4. Lifecycle & re-activation

```
              spawn
                |
                v
            [ ACTIVE ] --- run terminal + edge drained/closed ---> [ PARKED ]
                ^                                                       |
                |  activate(thread, input, provenance)                 | (retention, future)
                +-------------------------------------------------------+--> [ ARCHIVED ]
                   = NEW run on same thread, prior transcript as context     read-only, not wakeable
```

`ACTIVE` is **inclusive**: it covers "a run is executing" *and* "the run hit
terminal but its edge is not yet drained/closed." `activate` returns `Busy` for
the whole `ACTIVE` span; a thread is only `PARKED` once it has no open edge and
no executing run. This removes any ambiguous transit window (decision 4).

A **parked** thread holds no scheduler slot and **no open await-edge** — every
edge is in a closed state (`drained` or `abandoned`). It keeps its transcript and
lineage (storage only). Thousands of parked children are cheap. Active children
consume scheduler slots, already capped per-user/per-type by `TurnRunScheduler`.

Re-activation rules:
- **One run at a time** (decision 4) — concurrent `activate` → `Busy`.
- **Depth floor** (decision 14) — `activate`/`spawn` refused at `max_depth`.
- **A human may wake a parked child whose parent already ended**, unilaterally
  (the human owns the tree). The new run has no awaiting parent edge; its result
  rests in the thread.
- **A parent waking its own child** opens a fresh await-edge for the new run,
  pathed under the run that called `spawn`/`extend` (which may be a later parent
  run than the original spawner — see §6).
- **Consent to wake:** an **agent** (`subagent_extend`) may wake only its own
  **direct** child, and only from a live run that can issue the call. Once the
  spawning run is terminal — an orphaned child or any deeper orphaned descendant
  — **only a human** (the tree-owner) may wake it. No agent reaches across a dead
  run to wake a grandchild; no sibling/peer wake (no peer-spawn in scope).

The **parent-terminates-first** case (decision 15) preserves the parked
invariant: the resolver closes the orphaned edge as `abandoned`, so the child's
thread reaches `PARKED` with no open edge, and the human can still wake it.

`ARCHIVED` is a future retention state (read-only, not wakeable); it preserves
the LLM-data-never-deleted invariant. Out of scope for the initial build.

## 5. Capability flows & tool surface

**Req 1 — parent inspects child's work anytime** (read-only, no run):
`subagent_inspect(child_run_id)` reads the child run record + transcript-so-far,
frames it as untrusted + caps it, returns `{ status, last_event_age, flavor,
work_so_far }`. Supersedes the old metadata-only restriction.

**Req 2 — human opens child thread, converses** (console; human is trusted):
console opens `thread_id`, renders the raw transcript, and a console message is
`activate(thread, message, provenance=Human)`. Sensitive actions during the run
bubble to the human.

**Req 3 — parent respawns finished child with extension** (no cold start):
`subagent_extend(child_thread, task)` (child must be Parked, else `Busy`) =
`activate(thread, task, provenance=ParentAgent)`; new run, prior transcript is
context, parent opens a fresh await-edge.

Model-visible tools (Layer 2):

| tool | does | provenance |
|------|------|------------|
| `spawn_subagent` | create child thread + run#1 + await-edge | ParentAgent |
| `subagent_status` | cheap metadata snapshot (no content) | — |
| `subagent_inspect` | framed work-so-far + status (req 1) | — |
| `subagent_extend` | activate parked child with new task (req 3) | ParentAgent |
| `subagent_cancel` | cooperative cancel of a running child | ParentAgent |

Console surface (human): open thread, view raw transcript, send message
(= `activate`, `Human`), approve/deny bubbled gates.

This table is the **eventual** surface; tools land per the §9 PR column
(`spawn_subagent` PR 1, `subagent_inspect` PR 3, `subagent_extend` PR 4, console
PR 5, `subagent_cancel` PR 6). A tool is unregistered (no schema) until its PR —
not stubbed-as-unimplemented.

## 6. Delivery & durability (Layer 1)

Storage is `ScopedFilesystem` (`crates/ironclaw_filesystem/src/scoped.rs`:
`put(scope, path, Entry, CasExpectation)`, `get`, `list_dir`), one file per
edge, under the per-scope MountView (same scoping as the goal/tombstone stores —
see the §8 MountView caveat):

```
 /turns/subagent-await-edges/<parent_run_id>/<child_run_id>.json
   { child_thread_id, mode, state, terminal_kind?, created_at, settled_at?, closed_at? }
```

`terminal_kind` reuses the existing `TurnStatus` terminal variants
(`Completed` | `Failed` | `Cancelled`) — no new enum. `state` is the edge state
machine below (`open` | `settled` | `drained` | `abandoned`).

Edge lifecycle — compare-and-swap transitions on one file, no transactions:

```
 spawn/extend          -> put(CasExpectation::Absent)         state=open
 child terminal        -> CAS open->settled (attach terminal_kind)
 parent drains         -> CAS settled->drained
 parent run terminal   -> CAS open->abandoned                 (decision 15)
   while edge still open
```

**CAS mechanics.** The initial write uses `CasExpectation::Absent`. Each
transition `get`s the current `VersionedEntry`, then `put`s with the read version
as the expectation. On `VersionMismatch` the writer re-reads: if the edge is
already in (or past) the target state, it no-ops — this covers both the
concurrent live+boot resolver race and post-crash re-delivery. This single-winner
CAS *is* the idempotency guarantee (decision 8); there is no ledger.

**Path identity & access patterns.** The edge is keyed by
`(parent_run_id, child_run_id)`. Two access patterns, both cheap:
- **Live settle:** a child knows its own `parent_run_id` (lineage), so the edge
  is a direct path lookup — no by-child index.
- **Parent drain:** the parent lists its own directory
  `/await-edges/<parent_run_id>/` (single-level `list_dir`).

`parent_run_id` in the path is always **the run that called `spawn`/`extend`**,
not the thread's first run (relevant when a later parent run extends a child).

**Resolver.**
- *Live:* on a child-terminal event, direct-path the edge → CAS `open→settled` →
  wake the blocking parent (§3) or leave for background drain.
- *Boot:* enumerate active (non-terminal) parent runs; for each, `list_dir` its
  edge directory; for each open edge, `get_run_state(child)`; settle the terminal
  ones, close `abandoned` ones whose parent run is terminal. This is driven by
  active runs — **no global recursive scan**.

**Capacity** (decision 9) is a `list_dir` count of open edges in scope at spawn
admission against a single cap constant. (If listing ever becomes hot, add one
CAS'd count file — not bucket sharding.)

**Result delivery.** On drain the parent reads the child's terminal output from
the child run record/transcript by `child_run_id`, frames it as untrusted
(§7), and feeds it into its context. `terminal_kind` on the edge lets the parent
distinguish completed/failed/cancelled without a fetch.

### What is deleted vs kept from PR #4656

```
 DELETED                                   REPLACED BY
  settlement_log table                      child run record (get_run_state)
  3 gate tables                             1 await-edge file
  bucketed capacity counter (K=16)          list_dir count of open edges
  2-phase idempotency ledger                edge CAS (single-winner per transition)
  gate_ref / result_ref fields              edge path identity + child_run_id locator
  CapabilityResultStore + capability_results child run output (general capability
                                              result store is orthogonal — out of scope)
  phase-batched replay reconciler           resolver re-checks open edges on boot
  all SQL DDL + dual-backend SQL parity      ScopedFilesystem (parity at RootFilesystem layer)

 KEPT
  scope cols + conditional agent-predicate isolation invariant
```

### Migration of the live gate infrastructure

The old gate path is **not** deleted in PR 1 — it is bypassed behind the flag.
These live symbols stay until v2 is the default and has soaked:
`SubagentGateResolutionStore` + `InMemorySubagentGateResolutionStore` and
`SubagentSpawnDeps.gate_store` (`crates/ironclaw_loop_support/src/subagent_spawn_port.rs`),
`AwaitedChildSetRecord.{gate_ref, result_ref}`, and `TurnRunRecord.gate_ref`
(`crates/ironclaw_turns/src/store.rs`).

- `subagent.v2_enabled = false` → the existing in-memory gate path serves
  today's blocking subagents unchanged.
- `subagent.v2_enabled = true` → the await-edge path (this design) serves
  spawn/await; the old `gate_store` is not consulted.

A dedicated **cleanup PR after the rollout soaks** removes the old trait, store,
`gate_store` wiring, and the `gate_ref`/`result_ref` fields. PR 1 adds the new
path alongside; it does not touch the old one. No silent dual-delivery: exactly
one path is live per the flag.

### Implementation notes (existing seams)

- `wrap_untrusted_subagent_text` is a **private** `fn` in
  `crates/ironclaw_reborn/src/subagent/completion_observer.rs`. It must be made
  `pub(crate)` or moved to a shared subagent utilities module before the resolver
  can reuse it.
- `PostCapabilityStage::drain_settled` exists today as a typed **stub**
  (`-> Vec<()>`, returns empty, one call site at `post_capability.rs`) reserved
  for exactly this. PR 2 replaces both its body **and** its signature — target
  shape roughly `async fn drain_settled(&self, scope, parent_run_id) ->
  Result<Vec<SettledChild>, AgentLoopExecutorError>`, where `SettledChild`
  carries `{ child_run_id, terminal_kind }` (the parent fetches output by
  `child_run_id`). Final shape is PR 2's to set; this removes the ambiguity.

## 7. Safety & trust

```
 PRINCIPALS               TRUST
  human (console owner)    trusted — sees child raw; can drive any owned thread
  parent agent (LLM)       semi-trusted — may carry injection; cannot auto-approve
  child agent (LLM)        semi-trusted — its output is untrusted data upstream

 CONTENT edges
  child -> human console   RAW       (human owns it; makes req 2 free)
  child -> parent agent    FRAMED    (wrap-as-untrusted + byte-cap; no semantic scan)
  parent -> child          bounded, provenance-tagged

 APPROVALS (the security guarantee)
  any child sensitive action -> gate -> bubbles up lineage -> first HUMAN decides
  no parent LLM, no child LLM ever auto-approves
```

Three load-bearing claims:
1. Approval-bubbling is the catastrophe backstop, not text scrubbing — an
   injected instruction reaching the parent LLM cannot *act* without a human.
2. Cheap framing still earns its keep (reuses `wrap_untrusted_subagent_text`):
   prevents the parent treating child data as instructions, and caps context.
3. The human console edge is raw and unrestricted — the human's own data.

**Approval-bubbling mechanism.** A child's sensitive action raises a gate that
suspends the child run; the gate escalates along the lineage (`parent_run_id`
chain) until it reaches a human gate surface — the console if a human is attached
to that thread, otherwise the root user's surface, tagged with the source thread.
No intermediate LLM resolves it. The concrete gate-escalation wiring is designed
and built in PR 6 (§9); it extends the existing Reborn approval-gate layer.

## 8. Open items to verify during implementation

- **Postgres `RootFilesystem` `list_dir` prefix index.** Boot-scan (per active
  parent) and capacity-count depend on efficient prefix listing. Verify the index
  exists; add it if missing (the prior spec flagged this in its §2.4).
- **MountView scoping.** `FilesystemRebornIdentityStore` scopes per
  `(tenant, user, agent, project?)`, and `TurnScope::to_resource_scope()` falls
  back to a system-sentinel user when there is no explicit owner. Confirm the
  await-edge MountView resolves to a per-`(tenant, user)` (or finer) partition
  for ownerless child runs; if it is tenant-only, add a `users/<user_id>/` path
  segment before shipping.
- **Approval-sink coverage.** Verify no child code path can take a sensitive
  action (tool, secret, outbound) without bubbling to a human gate. Decision 11
  is only as strong as its weakest un-gated sink.
- **`subagent.v2_enabled` flag** does not exist yet — provision it via the
  `ironclaw_reborn_composition` config pattern before PR 1 ships.

## 9. Rollout — Option A (vertical walking skeleton)

Each item is a small, end-to-end, independently shippable PR behind
`subagent.v2_enabled`.

| PR | Ships | Demoable result |
|----|-------|-----------------|
| 0 | **prerequisites** (gating, no behavior): provision `subagent.v2_enabled` flag (`ironclaw_reborn_composition` config); lift `wrap_untrusted_subagent_text` to `pub(crate)` / shared module. (No new `TurnStatus` variant — blocking reuses `BlockedDependentRun` with a real await-edge `GateRef`, per §3.) | clean entry point for PR 1 |
| 1 | child = thread + `spawn_subagent` + one await-edge file + resolver + boot recovery + **depth floor** (decision 14) (**blocking only**) | spawn → block → result; kill mid-run → reboot → delivered; spawn refused past max depth |
| 2 | background mode + `PostCapabilityStage` drain (replaces the `drain_settled` stub signature) | parent continues, drains later |
| 3 | req 1 — `subagent_inspect` (framed read) | parent reads child work-so-far |
| 4 | req 3 — `subagent_extend` (activate parked child, new run) | extend a finished child with prior context |
| 5 | req 2 — human console: open child thread + converse | human takes over a child |
| 6 | `subagent_cancel` + approval-bubbling escalation wiring | cooperative cancel; gates bubble to human |

PR 1 validates the central bet (filesystem await-edge + resolver + crash
recovery + depth floor). Its resolver is written for the **full** edge state
machine; since background mode does not exist until PR 2, the background-drain
branch is a no-op stub in PR 1 (only blocking edges are settled-and-drained).
Each later PR adds one capability.

## 10. Out of scope / later

Tracked, not built in the initial rollout. Several are "features worth stealing"
from the research survey:

- **Fork-on-extend** — branch a parked thread's transcript into a new thread to
  explore an alternative (LangGraph checkpoint-fork). The default extend
  re-activates the same thread; fork is a separate explicit op.
- **Cross-agent shared memory store** — namespaced KV readable across an agent
  tree (LangGraph `BaseStore`); back it with the existing workspace.
- **Per-child hard token/cost budget** — pre-call enforcement with a hard stop,
  not alerts (the "$47K loop" failure mode). No budget field is stored on the
  thread until this is built.
- **Per-child structured output schema** — parent declares the child's expected
  return type; typed child→parent results instead of free text.
- **Tool attenuation at max depth** — beyond the decision-14 hard stop, disable
  spawning/other tools as depth approaches the cap; cap concurrent children per
  parent.
- **Structured observability** — replay/agent-tree spans and metrics; gated on a
  real metrics/OTel backend existing (Reborn currently has only log/noop).
- **`ARCHIVED` retention state + GC** for long-idle threads.
- **Auto-cancel-on-parent-death** policy (decision 15 keeps the child alive by
  default; a future policy may cancel orphaned background children).
- **General capability-result store** (the old §4) — orthogonal to subagent
  delivery; revisit on its own merits.

## 11. Test plan

- **PR 1 acceptance (the core bet):** spawn → blocking deliver; process-kill
  mid-run → reboot → resolver delivers; double-resolve is a no-op (CAS);
  parent-terminates-first → edge `abandoned`, child still wakeable; spawn refused
  at `max_depth`; a scoped query never returns another `(tenant, user, agent)`'s
  edge (the one isolation test worth keeping from the old §7.3).
- **Per-PR:** drive each capability through its caller (the tool / console
  handler), not just the helper — per `.claude/rules/testing.md`.
- **Filesystem parity:** the await-edge lifecycle behaves identically across the
  `RootFilesystem` backends; parity lives at the filesystem layer (no per-store
  SQL parity matrix).

## References

- (superseded sub-spec `docs/reborn/2026-06-08-subagent-durability-spec.md` — removed; replaced by this doc)
- PR #4656 (closed by this design)
- `crates/ironclaw_filesystem/src/scoped.rs` (`ScopedFilesystem`: `put`/`get`/`list_dir`, `CasExpectation`)
- `crates/ironclaw_turns/src/request.rs` (`GetRunStateRequest`, `SubmitChildRunRequest`), `crates/ironclaw_turns/src/store.rs` (`TurnRunRecord` lineage fields)
- `crates/ironclaw_reborn/src/subagent/` (existing in-memory stores; `record_child_terminal`, `wrap_untrusted_subagent_text`)
- `crates/ironclaw_reborn_identity/src/filesystem_store.rs` (the CAS + ScopedFilesystem pattern this reuses)
- `crates/ironclaw_agent_loop/src/executor/post_capability.rs` (`PostCapabilityStage::drain_settled` stub)
- `crates/ironclaw_host_runtime/src/turn_scheduler.rs` + `crates/ironclaw_reborn_composition/src/runtime/runtime_turn_scheduler.rs` (scheduler caps + wake notifier)

# Subagent Spawn for the Reborn Agent Loop — Design

**Status:** Proposed
**Date:** 2026-05-19
**Branch:** `subagent-spawn-design`
**Scope:** `crates/ironclaw_agent_loop`, `crates/ironclaw_turns`,
`crates/ironclaw_loop_support`, `crates/ironclaw_reborn`

This is the **overarching design doc**. Per-phase implementation docs (detailed,
with pseudo code) live alongside this file:

- [`phase-1-contracts.md`](./phase-1-contracts.md) — contracts & isolated units
- [`phase-2-mechanisms.md`](./phase-2-mechanisms.md) — spawn, prompt, driver, observer
- [`phase-3-integration.md`](./phase-3-integration.md) — wiring & end-to-end tests

---

## 1. Context & motivation

The IronClaw **Reborn** agent loop has no way for a running agent loop to spawn a
child loop. The broader goal is a *system around agent loops* covering four loop
types — **subagents**, long-running **missions**, **cron** jobs, and **trigger**
(event-matched) jobs. This design delivers **subagents first**; the other three
become additive later (each is a new `LoopFamily` plus a new turn *submitter*,
with no rework of what this design ships).

A **subagent** is a child agent loop with a fresh context, an attenuated tool set,
its own model and "direction" (persona), spawned by a parent loop, returning a
result to the parent either **blocking** (parent waits) or **background** (parent
continues, result delivered later).

This design was produced through an iterative review process and hardened by a
four-reviewer pass (design / bugs / conventions / security) against the live
crates. Section 8 (Security model) and Section 9 (Failure & concurrency model)
exist because of that review.

### Legacy / out of scope

`src/agent/`, `src/worker/`, `src/tools/`, and `crates/ironclaw_engine/` are
**legacy** and are not designed against. The Reborn loop is the only target.

## 2. Goals & non-goals

**Goals**

- A running agent loop can spawn one or more child agent loops via a tool call.
- Child loops have a fresh context, an attenuated capability surface, their own
  model, iteration/cost budget, and "direction" (persona prompt).
- Results return to the parent blocking *or* background.
- Parallel spawns: one parent turn may spawn N children that run concurrently.
- The design generalises: missions/cron/triggers slot in as new families +
  submitters without reworking subagents.
- Static over dynamic — no plugin system; the set of subagents is a closed
  compile-time table.

**Non-goals (v1)**

- Mission / cron / trigger loop types (future; seams only).
- Nested parent↔child UI/event linkage (child shows as a normal standalone thread).
- `Fork` seed mode (full parent-context copy) — enum variant reserved, unimplemented.
- Relaying a child's approval gate into the parent conversation.
- File-discovered, user-defined subagent flavors.

## 3. Glossary (Reborn loop terms)

| Term | Meaning |
|---|---|
| **Turn / run** | One unit of work in a thread scope. `TurnCoordinator::submit_turn` queues it; a `TurnRunnerWorker` claims and drives it. |
| **`TurnScope`** | `(tenant_id, agent_id?, project_id?, thread_id)` — the coordination key. Active-run exclusivity is per-scope. |
| **`LoopFamily`** | A sealed, static composition of the nine loop strategies — the *mechanics* of a loop. Built-in only; not a plugin system. |
| **`PlannedDriver`** | Adapts one `LoopFamily` + the executor to the `AgentLoopDriver` contract. A run reaches a family only via run-profile → driver → family. |
| **Run profile** | `ResolvedRunProfile` — per-run config: capability surface, model, budget, driver descriptor. |
| **Loop-host port** | A `LoopXxxPort` trait the executor calls for host I/O (context, model, capability, checkpoint, …). Strategies *decide*; ports *execute*. |
| **Gate** | A capability invocation can return a gate (`Approval`/`Auth`/`Resource`, and — new — `AwaitDependentRun`). The loop checkpoints `BeforeBlock` and returns `LoopExit::Blocked`. |
| **`CapabilityOutcome`** | What a capability invocation yields back to the executor: a result, a gate, a spawned process — and, new, a spawned child run. |

## 4. Design principles

1. **The loop family is the loop-type discriminator.** A subagent run = a run of
   the `subagent` `LoopFamily`. No parallel "origin" enum duplicating that.
2. **Codebase-native mechanism.** Spawn is an ordinary capability; its host port
   impl returns a *new `CapabilityOutcome` variant* — the exact shape the executor
   already handles for process spawn and for gates. No new executor routing.
3. **Static over dynamic.** Subagent flavors are a compile-time table; direction
   prompts are `include_str!`'d `.md` files. No plugin loader.
4. **Respect crate boundaries.** The sealed loop framework stays product-agnostic;
   `ironclaw_turns` owns coordination contracts; `host_runtime` is untouched.
5. **Fail loud, fail closed.** No silent fallbacks on store/IO; security gates
   reject by default.

## 5. Architecture

> Diagrams below are rendered from D2 sources in [`diagrams/`](./diagrams/) —
> edit the `.d2` file and re-run `d2 <name>.d2 <name>.svg`.

### 5.1 Dependency layering (acyclic)

![Crate layering](diagrams/architecture.svg)

### 5.2 The spawn mechanism — corrected

An earlier draft proposed the executor "routing a `SpawnChildRun` effect" to a
dedicated port. **The real executor cannot do this** — it batches all tool calls
to `invoke_capability_batch`, and `CapabilityCallCandidate` carries no effect
field; `EffectKind` is deliberately not propagated to the loop layer.

The mechanism therefore is:

`spawn_subagent` is an **ordinary capability** in the surface. The executor
invokes it through the existing `invoke_capability_batch` path. The host's
capability-port impl (in `ironclaw_loop_support`) recognises the `spawn_subagent`
capability id, performs the spawn, and returns a **new `CapabilityOutcome`
variant**:

- **background** → `CapabilityOutcome::SpawnedChildRun { child_run_id }` — a normal
  capability result, threaded back as the tool result.
- **blocking** → an `AwaitDependentRun` **gate** outcome — the executor already
  turns a gate into `checkpoint BeforeBlock → LoopExit::Blocked`.

No `EffectKind` change, no executor change, no `host_runtime` involvement.

### 5.3 What changes — by layer

```
ironclaw_agent_loop    + `subagent` LoopFamily (static composition)
(sealed framework)     + GateKind::AwaitDependentRun (pub(crate), wire-stable)
                       (executor: unchanged)

ironclaw_turns         + CapabilityOutcome::SpawnedChildRun
(coordination)         + AwaitDependentRun across LoopGateKind / LoopBlockedKind /
                         BlockedReason  and  TurnStatus::BlockedDependentRun
                       ~ TurnRunRecord: + parent_run_id, + subagent_depth
                       + children_of(run_id) store query

ironclaw_loop_support  + spawn handling in the capability-port impl
(host I/O glue)        ~ prompt/context port: direction system msg + user-role goal
                       + attenuation (CapabilityAllowSet) + hard allow_nesting gate

ironclaw_reborn        + `subagent` PlannedDriver + run-profile→driver binding
(composition)          + built-in subagent flavor table + direction .md files
                       + durable, bounded subagent goal store
                       + SubagentCompletionObserver (TurnEventSink)
                       ~ runtime.rs wiring

ironclaw_host_runtime / ironclaw_host_api   — unchanged
```

### 5.4 Static vs dynamic — ownership boundaries

Everything that defines *what subagents exist* is **static** — compiled into the
binary, identical in every deployment, no plugin loader, no runtime swapping. Only
*per-run* state (the goal, run ids, gates, lineage) is **dynamic**, created at
spawn time and keyed by the coordinator-minted `TurnRunId`.

![Static vs dynamic, by owning crate](diagrams/static-vs-dynamic.svg)

## 6. Detailed design — locked decisions

| Area | Decision |
|---|---|
| Execution | In-process child runs, runner-worker driven. |
| Result modes | Blocking and background (`run_in_background` arg on the capability). |
| Spawn surface | `spawn_subagent` capability; host port returns `SpawnedChildRun` (bg) or an `AwaitDependentRun` gate (blocking). |
| Blocking | The `AwaitDependentRun` gate and its awaited child-run **set** are recorded **at spawn time, before `submit_turn`** — durable. The gate awaits a set; the parent resumes once, after the **last** child is terminal. If every child is already terminal when the parent would block, the gate resolves **inline** (no `Blocked`). |
| Resume payload | One synthetic `GateRef`; a host-side gate-resolution store holds all N child results, mapped back to the N pending tool calls. |
| Child failure | Failed/cancelled children produce a typed result entry; the gate waits for **all** children to reach terminal — no early resume, no sibling cancellation. |
| Background delivery | Each child result is `accept_inbound_message`'d into the parent thread (idempotent). The follow-up parent turn is **coalescing** — `submit_turn` only if none pending; `ThreadBusy` means "already pending, message will be consumed". |
| Child authority | The child run starts with an **empty grant/lease set** — no inheritance of parent grants/leases. The capability allowlist is a surface *ceiling*, not authority. The child re-acquires every lease via its own `Approval` gate on its own thread. |
| Nesting | Hard gate: a spawn from a flavor with `allow_nesting=false` is rejected regardless of surface membership. Plus a depth cap (`subagent_depth` field, checked before `submit_turn`), a per-turn fan-out cap, and a per-run-tree descendant cap. |
| Loop family | One static `subagent` `LoopFamily` (`LoopFamilyId "subagent"`); default strategies + tighter `BudgetStrategy`. Bound to a dedicated `subagent` `PlannedDriver`. |
| Flavors | Built-in static table — v1: `general`, `researcher`. Each: direction id, tool allowlist, model, iteration + token/cost budget, `allow_nesting`. |
| Direction prompt | Static `.md` per flavor (`include_str!`, `ironclaw_reborn/src/directions/`), selected by static match. The system message. |
| Goal placement | The parent-injected goal + `Handoff` blob are the child's **first user message**, delimited as task data (`## Task (from parent)` / `## Context from parent`). **Never** the system message — the goal is model-generated and may carry upstream-tainted content. |
| Goal durability | Persisted in a durable, **bounded** goal store keyed by the child `TurnRunId`. `submit_turn` mints the run id, so the goal is staged under a temporary key and **re-keyed** once `submit_turn` returns (see `phase-2-mechanisms.md`). A store miss **fails the child run loudly** — never an empty `## Task`. |
| Lineage | `parent_run_id` + `subagent_depth` fields on `TurnRunRecord` (durable). `children_of` is a store query — no in-memory index as source of truth. |
| Idempotency | Child `submit_turn` key = `(parent_run_id, parent_turn_id, spawn-call ordinal)` — deterministic for replay, unique per spawn call even for identical-argument siblings. |
| Tenancy | The child `TurnScope` copies `tenant_id`/`agent_id`/`project_id` **verbatim**; only `thread_id` differs (fresh). Test-enforced invariant. |
| Child output trust | A child result crossing back to the parent is **untrusted data** — wrapped in a delimited block, channel-edge sanitised, and safety-scanned before it enters the parent thread. |
| Context seed | `Fresh` (goal only), `Handoff(String)` (goal + curated parent blob, re-materialised into the child scope). `Fork` reserved, unimplemented. |

## 7. Flows

### 7.1 Loop execution — pause & exit points

A subagent run is an ordinary agent-loop run. The executor cycles
context → model → capability calls, **checkpointing** at four points (the pause
points) and terminating at one of four `LoopExit` outcomes (the exit points). The
blocking-spawn gate reuses the `BeforeBlock` checkpoint + `LoopExit::Blocked` path
that approvals already use.

![Loop execution — checkpoint and exit points](diagrams/loop-execution.svg)

### 7.2 Spawn flow — background & blocking

`spawn_subagent` is an ordinary capability; the `ironclaw_loop_support` capability
port handles it and returns either `SpawnedChildRun` (background) or an
`AwaitDependentRun` gate (blocking).

![Subagent spawn flow](diagrams/spawn-flow.svg)

### 7.3 Blocking lifecycle — parent suspension & child runs

In blocking mode the parent suspends on the `AwaitDependentRun` gate (releasing its
runner worker, keeping its thread lock) while N children run concurrently, each its
own coordinator-managed run.

![Blocking subagent lifecycle](diagrams/blocking-lifecycle.svg)

### 7.4 Autonomous wake (background)

When a background subagent completes and **the parent is idle / no user is
interacting**, the parent still runs — `SubagentCompletionObserver`'s
`submit_turn(parent_scope)` **is the tick**. Reborn turns are
**coordinator-queued, not user-triggered**; any submitter can queue a turn for a
thread, and the runner-worker pool claims it. The observer is one such submitter
(channels are another). No user presence is required for the parent to wake.

**Result and tick are decoupled.**

| Concept | Where it lives | Lifetime |
|---|---|---|
| Subagent result data | A message in the **parent thread transcript** (provenance-tagged `SubagentResult`) | Durable |
| Wake signal | A **queued parent turn** from `submit_turn(parent_scope)` | Transient — collapsed into the next-claimed turn |

That decoupling is what makes coalescing work: **N child completions stage N
transcript messages but only 1 queued parent turn**, which consumes all N at
once via the normal context-load path. `ThreadBusy` from a second `submit_turn`
is expected ("already pending — message will be consumed"), not an error.

**Cascade risk** — autonomous wake can drive its own follow-up spawns
(parent processes results → spawns more subagents → those complete → wake parent
again → loop). Bounded by `MAX_TREE_DESCENDANTS`, `MAX_SPAWN_PER_TURN`, the
`subagent_depth` cap, and per-flavor `max_iterations` + token/cost budget — all
enforced **before** `submit_turn` (see §8).

### 7.5 Cancellation

```
parent CancelRequested
 └ SubagentCompletionObserver: children_of(parent) via durable parent_run_id
      recursively cancel_run the whole lineage subtree (BFS over parent_run_id)
      a child completing mid-cancel → its result is discarded
 a worker-released Blocked parent is driven to terminal Cancelled via the
   gate-abort path (it has no claiming worker of its own)
```

## 8. Security model

The four-reviewer pass surfaced subagents as a meaningful attack surface. The
mitigations below are **load-bearing**, not optional.

1. **No authority inheritance.** A child starts with an empty grant/lease set.
   `CapabilityAllowSet` filters the *surface* only. A child must re-acquire every
   privileged lease through its own `Approval` gate. A subagent can never exercise
   a lease the parent obtained from a prior user approval.
2. **Fork-bomb containment.** Depth alone is insufficient (N children each
   spawning N → N^depth). Three caps, all enforced **before `submit_turn`**, all
   rejecting without queuing: `MAX_SUBAGENT_DEPTH`, `MAX_SPAWN_PER_TURN`
   (fan-out), `MAX_TREE_DESCENDANTS` (per-run-tree total).
3. **Nesting hard gate.** `spawn_subagent` exclusion is not left to denylist-by-
   omission in each flavor's allowlist. A `spawn_subagent` invocation from a flavor
   with `allow_nesting=false` is rejected outright, regardless of surface membership.
4. **Prompt-injection isolation.** The goal + handoff blob are model-generated and
   may carry upstream-tainted content. They go in the child's **user** message,
   delimited as task data — never the system message. The system message is the
   static, authored direction `.md` only.
5. **Child output is untrusted.** A child's result crossing back to the parent
   (tool result or inbound message) is wrapped in a delimited block, channel-edge
   sanitised (host paths / internal identifiers stripped), and run through the
   inbound `safety_layer` scan before it is stored in the parent thread.
6. **Tenancy invariant.** The child `TurnScope` copies `tenant_id`/`agent_id`/
   `project_id` verbatim; a spawn whose resolved scope deviates is rejected.
7. **Idempotency keys** are derived from `(parent_run_id, parent_turn_id, ordinal)`
   — collision-free across identical-argument sibling spawns, deterministic for
   replay.

## 9. Failure & concurrency model

| Hazard | Handling |
|---|---|
| Child completes before the parent blocks (lost wakeup) | The `AwaitDependentRun` gate + awaited set are recorded **before `submit_turn`**, durably. On entering the gate the parent reconciles against `get_run_state`. |
| All children finish before the parent blocks | Gate resolves **inline** — the parent never emits `Blocked`. |
| One child fails mid-flight | Failed/cancelled child = a typed result entry; the gate still waits for all children; siblings are not cancelled. |
| Two background completions race on the parent thread | Results are `accept_inbound_message`'d (idempotent); the follow-up turn is coalescing; `ThreadBusy` is expected, not an error. |
| Process restart | Goal store is durable; lineage (`parent_run_id`/`subagent_depth`) is durable; `children_of` is a store query. No in-memory source of truth. A goal-store miss fails the child loudly. |
| Identical-argument sibling spawns | Distinct idempotency keys via the per-turn ordinal. |
| Parent cancelled | Recursive subtree cancel; worker-released Blocked parent driven to `Cancelled` via gate-abort. |
| Child completes with no assistant message | Typed "completed, no output" result — the parent always receives N well-formed results. |
| Partial spawn (`submit_turn` fails after thread create) | The awaited-set entry is the source of truth, written first; the half-spawn is reconciled (child absent or marked failed); fail loud. |

## 10. Crate boundary verification

| Crate | Rule | This design | Verdict |
|---|---|---|---|
| `ironclaw_agent_loop` | sealed; product-agnostic; refs not raw prompts | one `subagent` family; `GateKind::AwaitDependentRun` is neutral; executor unchanged | ✅ |
| `ironclaw_turns` | coordination contracts; lifecycle metadata + refs | `CapabilityOutcome` variant, blocked-kind variants, `parent_run_id`/`subagent_depth` | ✅ |
| `ironclaw_loop_support` | host-port adapter glue; no stateful stores | spawn handling in the capability port; goal store lives in `ironclaw_reborn` | ✅ |
| `ironclaw_reborn` | driver integration + composition | family driver, flavors, directions, goal store, observer, wiring | ✅ |
| `ironclaw_host_runtime` / `ironclaw_host_api` | — | untouched | ✅ |

Five wire-stable enums gain an `AwaitDependentRun`/`BlockedDependentRun`/
`SpawnedChildRun` variant: `CapabilityOutcome`, `LoopGateKind`, `LoopBlockedKind`,
`BlockedReason`, `TurnStatus`. Each new variant **matches the existing serde
convention of its enum** — `LoopGateKind`/`LoopBlockedKind`/`CapabilityOutcome`
are snake_case; **`TurnStatus` and `BlockedReason` serialize PascalCase today**
(no `#[serde(rename_all)]`) and their new variants stay PascalCase to avoid
breaking already-persisted records. Each gets a raw-JSON round-trip test;
`#[non_exhaustive]` is added to the observability-carrier enums
(`LoopGateKind`/`LoopBlockedKind`) but **not** to the state-machine-gating enums
(`CapabilityOutcome`/`TurnStatus`), which should force a compile break at every
transition site. `TurnStatus::BlockedDependentRun` is a persisted-enum migration —
grep producers, add a legacy-value deserialization test, and update the two
exhaustive `TurnStatus` match sites in `ironclaw_turns` `memory.rs`
(`resume_turn_once`, `request_cancel_once`). See `phase-1-contracts.md`.

## 11. Implementation phases

The work is a 3-level DAG. Phase 1 and Phase 2 each contain parallel workstreams;
Phase 3 wires everything. Each workstream is an independently reviewable PR.

![Implementation phase DAG](diagrams/phase-dag.svg)

```
PHASE 1 — Contracts & isolated units   (3 parallel workstreams)
  P1.A  ironclaw_turns contract additions
  P1.B  ironclaw_agent_loop: `subagent` family + GateKind::AwaitDependentRun
  P1.C  ironclaw_reborn data: direction .md files, bounded goal store, flavor table
        ── P1.A/B/C share only the agreed variant names (see phase-1 doc); they
           touch different crates and run fully in parallel.

PHASE 2 — Mechanisms                   (4 parallel workstreams; each needs Phase 1)
  P2.A  loop_support: spawn handling in the capability-port impl   [needs P1.A, P1.C]
  P2.B  loop_support: prompt composition + attenuation             [needs P1.A, P1.C]
  P2.C  ironclaw_reborn: `subagent` PlannedDriver + profile binding [needs P1.B]
  P2.D  ironclaw_reborn: SubagentCompletionObserver                [needs P1.A, P1.C]
        ── P2.A and P2.B touch the same crate but different files; coordinate file
           ownership (capability port vs prompt port).

PHASE 3 — Integration                  (single workstream; needs all of Phase 2)
  P3    runtime.rs wiring · spawn_subagent capability surface entry ·
        end-to-end integration tests · quality gate
```

Detailed per-phase docs with pseudo code:
[phase-1](./phase-1-contracts.md) · [phase-2](./phase-2-mechanisms.md) ·
[phase-3](./phase-3-integration.md).

## 12. Verification strategy

- **Unit tests** per crate — see each phase doc.
- **Integration tests** (`crates/ironclaw_reborn/tests/`): background E2E,
  blocking E2E, parallel-blocking E2E, early-completion (all children terminate
  before the parent blocks), child-authority (a child cannot use a lease the
  parent holds), fork-bomb (caps reject), cancellation subtree, no-deadlock
  regression (child `thread_id` ≠ parent).
- **Quality gate:** `cargo fmt`; `cargo clippy --all --benches --tests --examples
  --all-features` (zero warnings); `cargo test`.

## 13. Follow-ups (explicitly deferred)

Mission / cron / trigger loop families + their submitters; parent↔child UI/event
linkage (nested thread display); `Fork` seed mode; child-approval relay into the
parent; file-discovered user-defined flavors; a durable goal-store backend beyond
the bounded in-process store.

## 13a. Open question — goal-store durability vs production readiness

§9 requires the goal store to survive process restart, but §13 defers a durable
store backend. The bounded in-process store is therefore `NonDurable`, which a
strict `Production` readiness check (`ironclaw_reborn/src/production_readiness.rs`)
would block. **Pre-merge decision required:** either (a) ship the in-process store
behind a `LocalDevTest`-degraded readiness flag, or (b) write the goal through to
the durable turn-state DB in v1. `phase-3-integration.md` (Risk 10.1) carries the
detail.

## 14. Appendix — review provenance

This design was hardened by a four-reviewer pass (design, bugs, conventions,
security) against the live crates. Findings folded in: the `EffectKind` /
executor-routing correction (§5.2), the lost-wakeup and early-completion races
(§9), the 5-enum blocked-kind surface (§10), the `PlannedDriver` binding
requirement (§11 P2.C), durability of the goal store and lineage (§9), the
idempotency-key collision fix (§6), and the entire security model (§8).

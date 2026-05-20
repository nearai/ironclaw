# Phase 1 — Contracts & Isolated Units

**Status:** Implementation-ready
**Date:** 2026-05-19
**Parent:** [`README.md`](./README.md) (overarching design)
**Scope:** `crates/ironclaw_turns`, `crates/ironclaw_agent_loop`, `crates/ironclaw_reborn`

This document is the detailed, implementer-facing spec for **Phase 1** of the
subagent-spawn feature. Phase 1 lands the *contracts and isolated units* that
Phase 2 (mechanisms) and Phase 3 (integration) build on. It is three
independently-reviewable PRs, but **P1.B depends on P1.A** if each PR must build
against the whole workspace: P1.B maps the new `ironclaw_turns` gate kinds in
the executor. P1.C is independent. See §0 and §4 for the exact ordering.

Every type definition, field name, and signature in this doc was checked against
the live worktree. Where the overarching design named something inaccurately,
this doc corrects it and flags the correction inline (search for **[CORRECTION]**).

---

## 0. Inter-workstream contract (read first)

P1.A, P1.B, and P1.C touch disjoint crates. They only need to agree on a small
set of **names and wire strings**. Freeze these before any workstream starts —
they are the seam.

### 0.1 Shared variant / identifier names

| Concept | Exact name | Owner | Consumers |
|---|---|---|---|
| New `CapabilityOutcome` variant | `SpawnedChildRun { child_run_id: TurnRunId, result_ref: LoopResultRef, safe_summary: String }` | P1.A | P2.A produces it; executor pushes the result ref |
| New `CapabilityOutcome` variant | `AwaitDependentRun { gate_ref: LoopGateRef, safe_summary: String }` | P1.A | P2.A produces it; executor maps it to `GateKind::AwaitDependentRun` |
| New `LoopGateKind` variant | `AwaitDependentRun` | P1.A | executor `loop_gate_kind` (P3) |
| New `LoopBlockedKind` variant | `AwaitDependentRun` | P1.A | executor `blocked_kind` (P3) |
| New `BlockedReason` variant | `DependentRun { gate_ref: GateRef }` | P1.A | coordinator, runner (P2/P3) |
| New `TurnStatus` variant | `BlockedDependentRun` | P1.A | store, coordinator (P2/P3) |
| New `GateKind` variant (`ironclaw_agent_loop`) | `AwaitDependentRun` | P1.B | executor `handle_gate` (P3) |
| New `LoopFamilyId` value | `"subagent"` (wire string) | P1.B | reborn driver binding (P2.C) |
| New lineage fields on `TurnRunRecord` | `parent_run_id: Option<TurnRunId>`, `subagent_depth: u32`, `spawn_tree_root_run_id: Option<TurnRunId>` | P1.A | reborn submit path (P2.A), observer (P2.D), reservation table (P1.C/P2) |
| New lineage fields on `SubmitTurnRequest` | `requested_run_id: Option<TurnRunId>`, `parent_run_id: Option<TurnRunId>`, `subagent_depth: u32`, `spawn_tree_root_run_id: Option<TurnRunId>` | P1.A | P2.A spawn path; mission/cron/trigger submitters (future) |
| New coordinator trait method | `TurnCoordinator::prepare_turn(scope: TurnScope) -> Result<TurnRunId, TurnError>` | P1.A | P2.A spawn handler — mints child run id **before** any side-effect so goal store and reservation can be keyed by the final id |
| New store query | `children_of(&self, run_id: TurnRunId)` | P1.A | observer cancellation subtree walk (P2.D) |
| New store query | `get_run_record(&self, run_id: TurnRunId)` | P1.A | observer parent lookup for terminal child events (P2.D) |
| New store atomic | `tree_descendant_count_and_reserve(root: TurnRunId, delta: u32) -> Result<u32, TurnError>` | P1.A (trait) / P1.C (`SpawnTreeReservation` row backend) | P2.A admission, before `submit_turn` |
| New store atomic (companion) | `release_tree_descendants(root: TurnRunId, delta: u32) -> Result<(), TurnError>` | P1.A | P2.A partial-spawn rollback |
| New coordinator hook | `DefaultTurnCoordinator::with_event_sink(Arc<dyn TurnEventSink>)` | P1.A | live `SubagentCompletionObserver` notification (P2.D/P3) |
| New spawn-result payload struct | `SpawnedChildRunPayload { child_run_id, child_thread_id, flavor, mode, status, output_available, final_text, failure_summary }` | P1.C (`ironclaw_reborn::subagent`) | P2.A `result_ref` content; parent's model receives it as tool result |
| New tombstone struct | `SubagentResultTombstone { child_run_id, terminal_status, disposition: SubagentResultDisposition }` + enum `SubagentResultDisposition::DiscardedByParentCancel` | P1.C (`ironclaw_reborn::subagent`) | P2.D writes on mid-cancel terminal completes; reconciler reads |

### 0.2 Wire-string contract for the new enum variants

Five wire-stable `ironclaw_turns` enums and the one `ironclaw_agent_loop` enum
gain the additive variants below. `CapabilityOutcome` gains two variants; every
other enum gains one. The serialized wire strings are frozen here:

| Enum | Rust variant | `#[serde]` wire string |
|---|---|---|
| `LoopGateKind` | `AwaitDependentRun` | `"await_dependent_run"` (enum has `rename_all = "snake_case"`) |
| `LoopBlockedKind` | `AwaitDependentRun` | `"await_dependent_run"` (enum has `rename_all = "snake_case"`) |
| `BlockedReason` | `DependentRun { gate_ref }` | `{"DependentRun":{"gate_ref":"…"}}` — **no `rename_all`** on this enum; variant tag is PascalCase, matching the existing `Approval`/`Auth`/`Resource` arms |
| `TurnStatus` | `BlockedDependentRun` | `"BlockedDependentRun"` — **no `rename_all`** on this enum; variant is PascalCase, matching the existing `BlockedApproval` etc. |
| `CapabilityOutcome` | `SpawnedChildRun { child_run_id, result_ref, safe_summary }` | `{"spawned_child_run":{"child_run_id":"…","result_ref":"…","safe_summary":"…"}}` — enum has `rename_all = "snake_case"` |
| `CapabilityOutcome` | `AwaitDependentRun { gate_ref, safe_summary }` | `{"await_dependent_run":{"gate_ref":"…","safe_summary":"…"}}` — enum has `rename_all = "snake_case"` |
| `GateKind` (`agent_loop`) | `AwaitDependentRun` | `"await_dependent_run"` (enum has `rename_all = "snake_case"`) |

> **[CORRECTION]** The overarching doc (§10) lumps `BlockedReason` and
> `TurnStatus` with the snake_case-named enums. They are **not** snake_case —
> neither carries `#[serde(rename_all)]` today (`status.rs` lines 10, 102). To
> preserve the existing wire format the new variants must keep PascalCase. The
> snake_case requirement only applies to `LoopGateKind`, `LoopBlockedKind`,
> `CapabilityOutcome`, and `GateKind`, which already carry `rename_all`.

### 0.3 Parallelism

- P1.C has **no compile-time dependency** on P1.A or P1.B.
- P1.B has a deliberate compile-time dependency on P1.A if the workspace build
  must stay green, because it maps `GateKind::AwaitDependentRun` to P1.A's new
  `LoopGateKind` / `LoopBlockedKind` variants.
- The names in §0.1/§0.2 are the only coupling. They are string/identifier
  constants — agree once, then never touch the other workstream.
- Risk: if a name drifts, Phase 2 fails to compile. Mitigation — land §0 as a
  one-paragraph note in each of the three PR descriptions and cross-link them.

---

## P0 — Pending-gate projection over `TurnLifecycleEvent` (PREREQUISITE)

**Not subagent-specific. Must land before Phase 2's subagent paths are useful in
production.** Can be built in parallel with Phase 1 (no shared files).

### P0.1 — The gap this closes

`block_run()` (`crates/ironclaw_turns/src/memory.rs:688-705`) only:
1. Updates the run's `TurnStatus` to `BlockedApproval` / `BlockedAuth` / `BlockedResource`.
2. Pushes a `TurnLifecycleEvent { kind: Blocked, … }` to the turn event buffer
   scoped to `TurnScope`.

The web UI surfaces approvals by querying the engine-level **`PendingGateStore`**
(`/src/gate/pending.rs`) keyed by `(user_id, thread_id)`. **Nothing today
populates `PendingGateStore` from a turn block.** A blocked turn is therefore
**invisible to the UI** — affects every blocked turn today (system-issued
cancellation turns, future cron/triggers, and subagents the moment this design
ships). Closing this gap is a prerequisite for the design's "child approval
resolves on the child thread" guarantee (README §6 "Approval surfacing").

### P0.2 — Why a projection, not a write-hook bridge

A direct write hook in `block_run()` that *also* inserts into `PendingGateStore`
is the smallest patch but the wrong shape: it is a dual-write between two
authoritative stores. Any code path that blocks without going through the hook
silently orphans the gate; any approval resolved on the engine side without
notifying turns leaves turn state stale. This is the **split-brain shape**
`.claude/rules/gateway-events.md` exists to prevent (incident references in that
rule). The rule's mandate: "every `AppEvent` projects from exactly one typed
source log."

`TurnLifecycleEvent` is already that typed source log. Make `PendingGateStore` a
**derived projection** of it. The UI's query surface is unchanged; the underlying
store is materialised by a projection consumer with a cursor. Replayable;
restart-safe; idempotent. No `block_run()` hook required.

### P0.3 — Contracts to add

- **Read model** — `PendingGateStore` keeps its existing reader API (so the UI
  is unchanged). Its writer API becomes internal to the projection consumer
  (no other writers).
- **Projection consumer** — lives in `crates/ironclaw_event_projections/`
  (the Reborn read-model boundary). Consumes `TurnLifecycleEvent`. Per event:
  - `kind = Blocked` with `status` ∈ {`BlockedApproval`, `BlockedAuth`,
    `BlockedResource`, `BlockedDependentRun` (new — from P1.A)} →
    upsert a `PendingGate { user_id = turn_actor.user_id, thread_id =
    scope.thread_id, run_id, gate_kind, gate_ref, sanitized_reason, blocked_at }`.
  - `kind ∈ {Completed, Failed, Cancelled, Resumed}` for a previously-blocked
    run → remove the `PendingGate` row for that `(user_id, thread_id, run_id)`.
- **Projection cursor** — durable `(consumer_id, last_event_cursor)` row. On
  startup the consumer resumes from `last_event_cursor`; on a fresh deployment
  it replays from the beginning. Reconciliation is purely additive — upserts
  by `(user_id, thread_id, run_id)`, never duplicates.

Pseudo code (Rust-shaped):

```rust
// crates/ironclaw_event_projections/src/pending_gate_projection.rs
pub struct PendingGateProjection {
    store: Arc<dyn PendingGateStore>,        // existing engine store; now write-via-projection-only
    cursor: Arc<dyn ProjectionCursorStore>,
}

#[async_trait]
impl TurnEventSink for PendingGateProjection {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        match event.kind {
            TurnEventKind::Blocked => {
                let row = PendingGate::from_lifecycle_event(&event);
                self.store.upsert(row).await?;
            }
            TurnEventKind::Completed
            | TurnEventKind::Failed
            | TurnEventKind::Cancelled
            | TurnEventKind::Resumed => {
                self.store
                    .remove_for_run(event.scope.user_id_for_owner(), event.run_id)
                    .await?;
            }
            _ => {}
        }
        self.cursor.advance(PROJECTION_ID, event.cursor).await
    }
}
```

(`from_lifecycle_event` pulls `user_id` from `TurnActor`, `gate_ref` from the
event payload, etc. Idempotent because the underlying store upsert is keyed by
`(user_id, thread_id, run_id)`.)

### P0.4 — Wiring (Phase 3 dependency)

Phase 3's `subagent_runtime.rs` registers this projection alongside the
`SubagentCompletionObserver` via the new `DefaultTurnCoordinator::with_event_sink`
(P1.A). At that point any subagent child run blocking on `Approval` (or any
future loop type's blocked turn) appears in `PendingGateStore` and is queryable
by the parent owner via the existing UI surface.

### P0.5 — Tests

- Unit: feed synthetic `TurnLifecycleEvent { Blocked }` → assert `PendingGate`
  row exists with correct keys; feed `{ Resumed | Cancelled | Completed }` →
  assert row gone.
- Unit: replay the same event stream twice → idempotent (one row, same content).
- Unit: cursor advance + crash-mid-batch → assert no row leaks, no row missed.
- Integration: block a real turn (e.g. via an `ApprovalRequired` capability
  outcome), assert `PendingGate` populated, resolve via existing
  `/api/chat/gate/resolve`, assert row removed and turn resumes.

### P0.6 — Migration of existing writers

Grep the worktree for current `PendingGateStore::{insert, upsert, remove}` call
sites outside the projection consumer. Each call site is one of:
- A legacy direct-write that this projection replaces — remove it (the
  projection now covers it because every direct-write call site is paired with
  a turn block that emits a `TurnLifecycleEvent`).
- A consumer we missed — list it and decide per-case whether it migrates to a
  read-only query or feeds the same projection.

Land P0 with the writer surface narrowed to the projection consumer only.

### P0.7 — Owner

Lives in `crates/ironclaw_event_projections/` (new module
`pending_gate_projection.rs`). Reader surface remains in `/src/gate/pending.rs`.

---

## 1. P1.A — `ironclaw_turns` contract additions

**Goal:** add the coordination-layer types the spawn mechanism needs: a new
capability outcome, the `AwaitDependentRun` blocked surface across five enums,
durable lineage fields on `TurnRunRecord`, and a `children_of` store query.

### 1.1 Files to modify / create

| File | Change |
|---|---|
| `crates/ironclaw_turns/src/run_profile/host.rs` | + `CapabilityOutcome::{SpawnedChildRun, AwaitDependentRun}`; + `LoopGateKind::AwaitDependentRun`; update `CapabilityOutcome::is_suspension` |
| `crates/ironclaw_turns/src/status.rs` | + `TurnStatus::BlockedDependentRun`; + `BlockedReason::DependentRun`; update `is_terminal`, `keeps_active_lock` (no behavior change, but re-verify); update `BlockedReason::status` / `gate_ref` |
| `crates/ironclaw_turns/src/loop_exit.rs` | + `LoopBlockedKind::AwaitDependentRun`; update `LoopBlockedKind::to_blocked_reason` |
| `crates/ironclaw_turns/src/request.rs` | + `requested_run_id`, `parent_run_id`, `subagent_depth`, `spawn_tree_root_run_id` on `SubmitTurnRequest` (all `#[serde(default)]`) |
| `crates/ironclaw_turns/src/store.rs` | + `parent_run_id`, `subagent_depth`, `spawn_tree_root_run_id` on `TurnRunRecord`; + `children_of`, `get_run_record`, `tree_descendant_count_and_reserve`, `release_tree_descendants` on `TurnStateStore` trait |
| `crates/ironclaw_turns/src/coordinator.rs` | + `prepare_turn(scope) -> TurnRunId` on `TurnCoordinator` trait + `DefaultTurnCoordinator` impl; + optional `TurnEventSink` on `DefaultTurnCoordinator`; publish submit/resume/cancel lifecycle events best-effort; `submit_turn` must honour `requested_run_id` (bind instead of mint) |
| `crates/ironclaw_turns/src/memory.rs` | + `parent_run_id`/`subagent_depth`/`spawn_tree_root_run_id` on `RunRecord`; thread through `persistence_record`; impl `children_of`, `get_run_record`, `tree_descendant_count_and_reserve`, `release_tree_descendants`; honour `requested_run_id` in `submit_turn`; update blocked-status `match` arms (resume + cancel) |
| `crates/ironclaw_turns/src/run_profile/milestones.rs` | no code change — `LoopHostMilestoneKind::GateBlocked` carries `LoopGateKind` opaquely; verify it still compiles |
| `crates/ironclaw_turns/src/db.rs` | no schema change — `TurnRunRecord` is stored as a JSON payload; verify the round-trip test still passes |
| `crates/ironclaw_turns/tests/…` | new unit + contract tests (§1.9) |

`request.rs` (`SubmitTurnRequest`) **is** modified in Phase 1. Without request
fields, P2.A has no caller-level way to carry lineage into the store, so
`TurnRunRecord.parent_run_id` / `subagent_depth` would always remain top-level
defaults. Phase 1 adds the request fields with `#[serde(default)]` and the memory
store copies them into `RunRecord` during `submit_turn`.

### 1.2 `CapabilityOutcome::{SpawnedChildRun, AwaitDependentRun}`

**Current** (`run_profile/host.rs` lines 1114–1145):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOutcome {
    Completed(CapabilityResultMessage),
    ApprovalRequired { gate_ref: LoopGateRef, safe_summary: String },
    AuthRequired { gate_ref: LoopGateRef, safe_summary: String },
    ResourceBlocked { gate_ref: LoopGateRef, safe_summary: String },
    SpawnedProcess(ProcessHandleSummary),
    Denied(CapabilityDenied),
    Failed(CapabilityFailure),
}

impl CapabilityOutcome {
    pub fn is_suspension(&self) -> bool {
        matches!(
            self,
            Self::ApprovalRequired { .. }
                | Self::AuthRequired { .. }
                | Self::ResourceBlocked { .. }
                | Self::SpawnedProcess(_)
        )
    }
}
```

**After:**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOutcome {
    Completed(CapabilityResultMessage),
    ApprovalRequired { gate_ref: LoopGateRef, safe_summary: String },
    AuthRequired { gate_ref: LoopGateRef, safe_summary: String },
    ResourceBlocked { gate_ref: LoopGateRef, safe_summary: String },
    SpawnedProcess(ProcessHandleSummary),
    /// A `spawn_subagent` capability submitted a *blocking* child run. This is
    /// a suspension gate, mapped by the executor to `GateKind::AwaitDependentRun`.
    AwaitDependentRun { gate_ref: LoopGateRef, safe_summary: String },
    /// A `spawn_subagent` capability completed by submitting a *background*
    /// child run. The executor pushes `result_ref` as the tool result while
    /// retaining `child_run_id` for lineage/observability. Unlike
    /// `SpawnedProcess`, this is a terminal capability result — the loop
    /// continues; it is NOT a suspension.
    SpawnedChildRun {
        child_run_id: TurnRunId,
        result_ref: LoopResultRef,
        safe_summary: String,
    },
    Denied(CapabilityDenied),
    Failed(CapabilityFailure),
}

impl CapabilityOutcome {
    pub fn is_suspension(&self) -> bool {
        // `AwaitDependentRun` is a suspension: a blocking spawn parks the parent
        // on a dependent-run gate.
        // `SpawnedChildRun` is intentionally NOT a suspension: a background
        // spawn returns a normal result and the parent loop keeps running.
        matches!(
            self,
            Self::ApprovalRequired { .. }
                | Self::AuthRequired { .. }
                | Self::ResourceBlocked { .. }
                | Self::AwaitDependentRun { .. }
                | Self::SpawnedProcess(_)
        )
    }
}
```

Notes:
- `TurnRunId` and `LoopResultRef` are already imported in/near `host.rs`;
  confirm and extend the `use crate::{…}` block if needed.
- Background wire form:
  `{"spawned_child_run":{"child_run_id":"<uuid>","result_ref":"result:…","safe_summary":"…"}}`.
  Blocking wire form:
  `{"await_dependent_run":{"gate_ref":"gate:…","safe_summary":"…"}}`.
- The variant is a **struct variant** (named field), consistent with the gate
  variants and ready for Phase 2 to carry more if needed without a tuple→struct
  break.

**Exhaustive `match` sites to update** (grep `CapabilityOutcome::` in non-test
`src`): the executor's `handle_capability_outcome` must gain two arms in the
same workspace-green change:

```rust
CapabilityOutcome::AwaitDependentRun { gate_ref, .. } => {
    self.handle_gate(planner, host, state, GateKind::AwaitDependentRun, gate_ref).await
}
CapabilityOutcome::SpawnedChildRun { result_ref, safe_summary, .. } => {
    push_completed_result(&mut state, CapabilityResultMessage {
        result_ref,
        safe_summary,
        terminate_hint: false,
    });
    Ok(BatchStep::Continue(Box::new(state)))
}
```

The important contract is that the background path pushes a **durable result
ref**. A child id alone is insufficient because the executor only appends result
refs to the loop state. (Alternative considered: mark `CapabilityOutcome`
`#[non_exhaustive]` — rejected, see §1.6.)

### 1.3 `LoopGateKind::AwaitDependentRun`

**Current** (`run_profile/host.rs` lines 1585–1591):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopGateKind {
    Approval,
    Auth,
    ResourceWait,
}
```

**After:**

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopGateKind {
    Approval,
    Auth,
    ResourceWait,
    /// Loop is blocked awaiting a set of dependent (child) runs to reach a
    /// terminal status. Surfaced by the subagent blocking-spawn path.
    AwaitDependentRun,
}
```

Add `#[non_exhaustive]` (it is a wire/observability enum that will keep growing
as mission/cron/trigger families land). Wire string: `"await_dependent_run"`.

**Exhaustive `match` sites** (grep `LoopGateKind`): the only `match` is
`executor.rs::loop_gate_kind` in `ironclaw_agent_loop` — updated in **Phase 3**,
not here. `milestones.rs` only *carries* `LoopGateKind` in
`LoopHostMilestoneKind::GateBlocked` and never matches it. No `ironclaw_turns`
match arms change.

### 1.4 `TurnStatus::BlockedDependentRun` + `BlockedReason::DependentRun`

**Current** (`status.rs` lines 10–32, 102–125):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnStatus {
    Queued,
    Running,
    BlockedApproval,
    BlockedAuth,
    BlockedResource,
    CancelRequested,
    Cancelled,
    Completed,
    Failed,
    RecoveryRequired,
}

impl TurnStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Completed | Self::Failed)
    }
    pub fn keeps_active_lock(self) -> bool {
        !self.is_terminal()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockedReason {
    Approval { gate_ref: GateRef },
    Auth { gate_ref: GateRef },
    Resource { gate_ref: GateRef },
}

impl BlockedReason {
    pub fn status(&self) -> TurnStatus {
        match self {
            Self::Approval { .. } => TurnStatus::BlockedApproval,
            Self::Auth { .. } => TurnStatus::BlockedAuth,
            Self::Resource { .. } => TurnStatus::BlockedResource,
        }
    }
    pub fn gate_ref(&self) -> &GateRef {
        match self {
            Self::Approval { gate_ref } | Self::Auth { gate_ref } | Self::Resource { gate_ref } => {
                gate_ref
            }
        }
    }
}
```

**After:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnStatus {
    Queued,
    Running,
    BlockedApproval,
    BlockedAuth,
    BlockedResource,
    /// Run is blocked awaiting one or more dependent (child) runs. Resumed by
    /// the host once the awaited set is fully terminal. Non-terminal: keeps the
    /// thread's active lock so a sibling turn cannot start.
    BlockedDependentRun,
    CancelRequested,
    Cancelled,
    Completed,
    Failed,
    RecoveryRequired,
}

impl TurnStatus {
    pub fn is_terminal(self) -> bool {
        // BlockedDependentRun is NOT terminal — unchanged set.
        matches!(self, Self::Cancelled | Self::Completed | Self::Failed)
    }
    pub fn keeps_active_lock(self) -> bool {
        // Derived from is_terminal; BlockedDependentRun keeps the lock,
        // matching every other Blocked* variant. No edit needed beyond
        // confirming the derivation still holds.
        !self.is_terminal()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockedReason {
    Approval { gate_ref: GateRef },
    Auth { gate_ref: GateRef },
    Resource { gate_ref: GateRef },
    /// Blocked awaiting a dependent run set. `gate_ref` is the single synthetic
    /// gate ref the host minted at spawn time; the host-side gate-resolution
    /// store maps it to the N child results on resume.
    DependentRun { gate_ref: GateRef },
}

impl BlockedReason {
    pub fn status(&self) -> TurnStatus {
        match self {
            Self::Approval { .. } => TurnStatus::BlockedApproval,
            Self::Auth { .. } => TurnStatus::BlockedAuth,
            Self::Resource { .. } => TurnStatus::BlockedResource,
            Self::DependentRun { .. } => TurnStatus::BlockedDependentRun,
        }
    }
    pub fn gate_ref(&self) -> &GateRef {
        match self {
            Self::Approval { gate_ref }
            | Self::Auth { gate_ref }
            | Self::Resource { gate_ref }
            | Self::DependentRun { gate_ref } => gate_ref,
        }
    }
}
```

`TurnStatus` and `BlockedReason` stay **without** `#[serde(rename_all)]` and
**without** `#[non_exhaustive]` (see §1.6 for the `#[non_exhaustive]` decision).
`TurnStatus` is `Copy`; `BlockedDependentRun` is a unit variant so `Copy` holds.

**Exhaustive `match` sites on `TurnStatus` to update** — grep
`TurnStatus::Blocked` in `crates/ironclaw_turns/src`:

1. **`memory.rs` `resume_turn_once`** (lines 1215–1218) — the resume-eligibility
   guard. **Must add `BlockedDependentRun`:**

   ```rust
   if !matches!(
       record.status,
       TurnStatus::BlockedApproval
           | TurnStatus::BlockedAuth
           | TurnStatus::BlockedResource
           | TurnStatus::BlockedDependentRun,
   ) {
       return Err(TurnError::InvalidTransition { from: record.status, to: TurnStatus::Queued });
   }
   ```

   Without this, `resume_turn` of a dependent-run-blocked parent would be
   rejected as an invalid transition — the blocking-spawn path would deadlock.

2. **`memory.rs` `request_cancel_once`** (lines 1269–1277) — the cancel
   next-status match. **Must add `BlockedDependentRun` to the cancellable arm:**

   ```rust
   let (next_status, event_kind) = match record.status {
       TurnStatus::Queued
       | TurnStatus::BlockedApproval
       | TurnStatus::BlockedAuth
       | TurnStatus::BlockedResource
       | TurnStatus::BlockedDependentRun
       | TurnStatus::RecoveryRequired => (TurnStatus::Cancelled, TurnEventKind::Cancelled),
       TurnStatus::Running | TurnStatus::CancelRequested => {
           (TurnStatus::CancelRequested, TurnEventKind::CancelRequested)
       }
       status => return Ok(CancelRunResponse { /* already_terminal */ }),
   };
   ```

   A dependent-run-blocked parent has no claiming worker, so it must cancel
   **directly to `Cancelled`** (the queued/blocked arm), exactly like the other
   `Blocked*` variants. The catch-all `status =>` arm would otherwise treat it
   as already-terminal — wrong.

`db.rs::status_key` (line 1571) just `serde_json`-serializes `TurnStatus` into a
JSON key string; the new variant works with **no edit** (it is not a closed
match). The `match` inside `TurnError`/`TurnIdempotency*` does not involve
`TurnStatus`. `memory.rs` lines 1037 / 1592 / 1635 match `Running |
CancelRequested` only — no edit. Line 714/734 are literal value lists, not
matches — no edit.

### 1.5 `LoopBlockedKind::AwaitDependentRun`

**Current** (`loop_exit.rs` lines 379–396):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopBlockedKind {
    Approval,
    Auth,
    Resource,
}

impl LoopBlockedKind {
    fn to_blocked_reason(self, gate_ref: LoopGateRef) -> Result<BlockedReason, ()> {
        let gate_ref = GateRef::new(gate_ref.as_str()).map_err(|_| ())?;
        Ok(match self {
            Self::Approval => BlockedReason::Approval { gate_ref },
            Self::Auth => BlockedReason::Auth { gate_ref },
            Self::Resource => BlockedReason::Resource { gate_ref },
        })
    }
}
```

**After:**

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopBlockedKind {
    Approval,
    Auth,
    Resource,
    /// Driver-reported block: the loop is awaiting a dependent (child) run set.
    AwaitDependentRun,
}

impl LoopBlockedKind {
    fn to_blocked_reason(self, gate_ref: LoopGateRef) -> Result<BlockedReason, ()> {
        let gate_ref = GateRef::new(gate_ref.as_str()).map_err(|_| ())?;
        Ok(match self {
            Self::Approval => BlockedReason::Approval { gate_ref },
            Self::Auth => BlockedReason::Auth { gate_ref },
            Self::Resource => BlockedReason::Resource { gate_ref },
            Self::AwaitDependentRun => BlockedReason::DependentRun { gate_ref },
        })
    }
}
```

Add `#[non_exhaustive]` (wire-stable, growing). Wire string:
`"await_dependent_run"`. `to_blocked_reason` is a private fn, but it is an
exhaustive `match` — the new arm above is **required** or the crate will not
compile. `LoopBlocked` (the struct that carries `kind: LoopBlockedKind`) carries
`#[serde(deny_unknown_fields)]` on its *fields*, which is unaffected by the enum
change.

> **[CORRECTION]** The overarching doc §10 says "`LoopBlockedKind` (+ its
> `to_blocked_reason` arm)". Confirmed accurate — `to_blocked_reason` is the
> single private exhaustive match and `loop_exit.rs::LoopExit::validate` calls
> it. No other `LoopBlockedKind` match exists (grep confirms).

### 1.6 `#[non_exhaustive]` decision matrix

Per `.claude/rules/types.md` and the design's "wire-stable enum" requirement:

| Enum | `#[non_exhaustive]`? | Rationale |
|---|---|---|
| `LoopGateKind` | **add it** | observability/wire enum; mission/cron/trigger families will add more gate kinds; external (reborn) test code constructs it but never exhaustively matches it |
| `LoopBlockedKind` | **add it** | same; the only exhaustive match (`to_blocked_reason`) is in-crate and updated atomically |
| `CapabilityOutcome` | **do NOT add it** | the executor in `ironclaw_agent_loop` *must* exhaustively match every outcome — a non-exhaustive `CapabilityOutcome` would force a `_ =>` arm that silently drops a future spawn-like outcome. Keeping it exhaustive means adding a variant is a compile error at every host, which is the desired fail-loud behavior. The P1.B/companion agent-loop change updates the executor match. |
| `TurnStatus` | **do NOT add it** | status drives exhaustive lifecycle matches in `memory.rs`; a `_ =>` arm would silently mishandle a new status (e.g. treat it as terminal). Adding a variant *should* break every match so each is reviewed. The two match sites in §1.4 are updated atomically in this PR. |
| `BlockedReason` | **do NOT add it** | `status()` / `gate_ref()` are exhaustive by design; a new reason must be mapped to a `TurnStatus`, never defaulted. |
| `GateKind` (`agent_loop`) | already `#[non_exhaustive]` | unchanged; see §2.4 |

Rule applied: **`#[non_exhaustive]` for enums whose only matches are in-crate or
that are observability-carriers; keep exhaustive for enums that gate
state-machine transitions**, so the compiler forces every transition site to be
reviewed when a variant is added. This matches the existing codebase — note
`LoopFailureKind` is `#[non_exhaustive]` (its matches are all in-crate
`to_sanitized_failure`-style maps) while `TurnStatus` is not.

Naming: both new snake_case enums get a snake_case wire string
(`await_dependent_run`); `TurnStatus`/`BlockedReason` keep PascalCase to match
existing variants and avoid a silent wire break on already-persisted records.

### 1.7 `TurnRunRecord` lineage fields + `children_of`

**`SubmitTurnRequest` lineage + reserved-id input.** Add four fields to
`request.rs::SubmitTurnRequest` so callers can create child runs with durable
lineage **and** pre-mint the run id via `prepare_turn`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitTurnRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub requested_run_profile: Option<RunProfileRequest>,
    pub idempotency_key: IdempotencyKey,
    pub received_at: TurnTimestamp,
    /// Pre-minted `TurnRunId` from `TurnCoordinator::prepare_turn(scope)`. When
    /// `Some`, the coordinator/store MUST bind this id to the new run rather
    /// than mint a fresh one — used by the spawn handler so the goal store and
    /// `SpawnTreeReservation` row can be keyed by the final child id **before**
    /// `submit_turn`. When `None`, the store mints a fresh id (legacy path).
    /// Mismatch with a coordinator-claimed id is `TurnError::Conflict` (see
    /// §1.10 — load-bearing for replay determinism).
    #[serde(default)]
    pub requested_run_id: Option<TurnRunId>,
    /// Parent run that spawned this run as a subagent child. `None` for
    /// top-level (user-initiated) runs.
    #[serde(default)]
    pub parent_run_id: Option<TurnRunId>,
    /// Depth in the subagent run tree. `0` for top-level; child =
    /// `parent.subagent_depth + 1`.
    #[serde(default)]
    pub subagent_depth: u32,
    /// Spawn-tree root for per-tree atomic accounting. `None` for top-level
    /// runs **and** for the immediate child of a top-level run (the root run
    /// is its own root — represented as `None`, not `Some(self_id)`, because
    /// the root's id is the row key in `SpawnTreeReservation`). For depth ≥ 2,
    /// the child inherits `parent.spawn_tree_root_run_id.unwrap_or(parent.run_id)`.
    #[serde(default)]
    pub spawn_tree_root_run_id: Option<TurnRunId>,
}
```

Top-level submitters leave all four fields at their defaults. P2.A is the first
caller that sets them — it calls `prepare_turn(child_scope)` to obtain
`requested_run_id`, then derives lineage from the parent record.

Back-compat: all four are `#[serde(default)]`, so every persisted/legacy request
JSON without these keys deserialises with `None`/`0` — wire-stable.

**Current** (`store.rs` lines 79–101):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRunRecord {
    pub run_id: TurnRunId,
    pub turn_id: TurnId,
    pub scope: TurnScope,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub status: TurnStatus,
    pub profile: TurnRunProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    pub checkpoint_id: Option<TurnCheckpointId>,
    pub gate_ref: Option<GateRef>,
    pub failure: Option<crate::SanitizedFailure>,
    pub event_cursor: EventCursor,
    pub runner_id: Option<TurnRunnerId>,
    pub lease_token: Option<TurnLeaseToken>,
    pub lease_expires_at: Option<TurnTimestamp>,
    pub last_heartbeat_at: Option<TurnTimestamp>,
    pub claim_count: u64,
    pub received_at: TurnTimestamp,
}
```

**After** — two new fields, both `#[serde(default)]` so every legacy persisted
record (which lacks them) deserializes cleanly:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRunRecord {
    pub run_id: TurnRunId,
    pub turn_id: TurnId,
    pub scope: TurnScope,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub status: TurnStatus,
    pub profile: TurnRunProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    pub checkpoint_id: Option<TurnCheckpointId>,
    pub gate_ref: Option<GateRef>,
    pub failure: Option<crate::SanitizedFailure>,
    pub event_cursor: EventCursor,
    pub runner_id: Option<TurnRunnerId>,
    pub lease_token: Option<TurnLeaseToken>,
    pub lease_expires_at: Option<TurnTimestamp>,
    pub last_heartbeat_at: Option<TurnTimestamp>,
    pub claim_count: u64,
    pub received_at: TurnTimestamp,
    /// Parent run that spawned this run as a subagent child. `None` for
    /// top-level (user-initiated) runs. Durable lineage — `children_of` is a
    /// store query over this field, not an in-memory index.
    #[serde(default)]
    pub parent_run_id: Option<TurnRunId>,
    /// Depth in the subagent run tree. `0` for top-level runs; a child is
    /// `parent.subagent_depth + 1`. Checked against `MAX_SUBAGENT_DEPTH`
    /// before `submit_turn` (Phase 2).
    #[serde(default)]
    pub subagent_depth: u32,
    /// Spawn-tree root for per-tree atomic descendant accounting. The
    /// `SpawnTreeReservation` row is keyed by this id (when `Some`) or by
    /// `run_id` (when `None` — the run is its own root). A top-level run
    /// always has `spawn_tree_root_run_id == None`. P2.A derives this from
    /// the parent record at spawn time and writes it into `SubmitTurnRequest`.
    #[serde(default)]
    pub spawn_tree_root_run_id: Option<TurnRunId>,
}
```

- `Option<TurnRunId>` `#[serde(default)]` → `None`. `u32` `#[serde(default)]` →
  `0`. All three legacy-safe.
- Do **not** add `skip_serializing_if` — always serialize them so a top-level
  run round-trips an explicit `parent_run_id: null, subagent_depth: 0` and a
  forensic read never has to distinguish "absent" from "top-level". (The
  existing `resolved_model_route` uses `skip_serializing_if` because it is large
  and genuinely optional; lineage is small and always meaningful.)

**Mirror fields on the in-memory `RunRecord`** (`memory.rs` lines 109–130). Add:

```rust
struct RunRecord {
    // … existing fields …
    parent_run_id: Option<TurnRunId>,
    subagent_depth: u32,
    spawn_tree_root_run_id: Option<TurnRunId>,
}
```

Thread them through:
- `persistence_record()` (`memory.rs` line 1821) — copy all three fields into
  the emitted `TurnRunRecord`.
- `from_persistence_snapshot` reconstruction — when rebuilding `RunRecord` from a
  `TurnRunRecord`, copy `parent_run_id` / `subagent_depth` /
  `spawn_tree_root_run_id` across.
- `RunRecord` construction in `submit_turn` (`memory.rs` ~line 476/499/512) —
  copy `request.parent_run_id`, `request.subagent_depth`, and
  `request.spawn_tree_root_run_id` into the record. Top-level callers get
  legacy behavior through the request fields' serde/default values
  (`None`/`0`/`None`).
- `state()` (`TurnRunState`, line 1845) — **not modified.** `TurnRunState` does
  not gain lineage fields in Phase 1; the observer reads lineage via
  `children_of` / the record, not via run state. Keeping `TurnRunState` stable
  avoids touching every `get_run_state` consumer.

**`children_of` and `get_run_record` store queries.** Add to the
`TurnStateStore` trait
(`store.rs` lines 15–35):

```rust
#[async_trait]
pub trait TurnStateStore: Send + Sync {
    async fn submit_turn(/* … unchanged … */) -> Result<SubmitTurnResponse, TurnError>;
    async fn resume_turn(/* … */) -> Result<ResumeTurnResponse, TurnError>;
    async fn request_cancel(/* … */) -> Result<CancelRunResponse, TurnError>;
    async fn get_run_state(/* … */) -> Result<TurnRunState, TurnError>;

    /// Return every run whose `parent_run_id == Some(run_id)`.
    ///
    /// Used by the subagent completion observer to walk a run-tree subtree for
    /// recursive cancellation. Order is unspecified. An unknown `run_id`
    /// returns an empty `Vec` (NOT an error) — a parent with no children is a
    /// valid, common state.
    async fn children_of(&self, run_id: TurnRunId)
        -> Result<Vec<TurnRunRecord>, TurnError>;

    /// Return the durable run record for `run_id`, or `Ok(None)` if unknown.
    ///
    /// Used by the completion observer to determine whether a terminal event is
    /// for a child run and, if so, which parent run should receive the result.
    async fn get_run_record(&self, run_id: TurnRunId)
        -> Result<Option<TurnRunRecord>, TurnError>;

    /// Atomically reserve `delta` additional descendant slots in the spawn
    /// tree rooted at `root_run_id` and return the **post-reservation** count
    /// (i.e. the new total descendant count including this reservation).
    ///
    /// Backed by a durable `SpawnTreeReservation` row keyed by
    /// `(tenant_id, spawn_tree_root_run_id)`; the increment is done under a
    /// store-level lock / atomic UPDATE so concurrent admit across subtrees on
    /// different threads cannot over-admit. The caller compares the returned
    /// count against `MAX_TREE_DESCENDANTS` (Phase 2 policy constant); the
    /// store itself does not enforce the cap — admission policy does.
    ///
    /// Runs **before** `submit_turn` in P2.A. On `submit_turn` failure the
    /// caller MUST `release_tree_descendants(root, delta)` to roll back.
    ///
    /// Returns `TurnError::Conflict` if the row is being mutated concurrently
    /// in an incompatible way (store-implementation-specific). NEVER returns a
    /// `WouldExceed` itself — the cap is policy, not store-enforced — but
    /// admission/spawn handler turns the returned count > cap into a typed
    /// rejection.
    async fn tree_descendant_count_and_reserve(
        &self,
        root_run_id: TurnRunId,
        delta: u32,
    ) -> Result<u32, TurnError>;

    /// Companion to `tree_descendant_count_and_reserve` — atomically decrement
    /// the descendant count for partial-spawn rollback. Called by P2.A when
    /// `submit_turn` of a reserved child fails between reservation and queue.
    /// Idempotent at the row level via the same atomic-update pattern; an
    /// underflow saturates at 0 and logs at `debug!` (a release of a slot we
    /// never reserved is a bug, but losing the row would orphan capacity
    /// which is worse).
    async fn release_tree_descendants(
        &self,
        root_run_id: TurnRunId,
        delta: u32,
    ) -> Result<(), TurnError>;
}
```

**`SpawnTreeReservation` row schema** (durable, backend-mirrored). Lives in the
same persistence backend as `TurnRunRecord` (libSQL / Postgres):

| Column | Type | Notes |
|---|---|---|
| `tenant_id` | text (PK part) | from the root run's `TurnScope.tenant_id` — tenant isolation |
| `spawn_tree_root_run_id` | uuid (PK part) | the root `TurnRunId`; equals the run's own id when its `spawn_tree_root_run_id` field is `None` |
| `descendant_count` | bigint (u64 in Rust) | total reserved descendants under this root; atomically incremented by `tree_descendant_count_and_reserve` |
| `created_at` | timestamptz | first reservation time; helps reconciliation age-out orphaned rows |
| `updated_at` | timestamptz | last reservation/release time |

Primary key: `(tenant_id, spawn_tree_root_run_id)`. The atomic increment is a
single `UPDATE … SET descendant_count = descendant_count + $1 RETURNING
descendant_count` (Postgres) or its libSQL equivalent. Row is INSERTed with
count = `delta` when it does not yet exist (`ON CONFLICT … DO UPDATE`). This
keeps the operation atomic without a separate `SELECT` round-trip.

> **[CORRECTION]** The overarching doc says `children_of(run_id)` returns a
> store query result without specifying the element type. It must return
> `Vec<TurnRunRecord>` (not `TurnRunState`) because the caller (P2.D observer)
> needs `parent_run_id`/`status`/`run_id` to BFS the subtree, and only
> `TurnRunRecord` carries `parent_run_id`. `TurnRunState` deliberately does not.
> The same reason requires `get_run_record(run_id)` for single terminal child
> events; `get_run_state` is not enough.

**`InMemoryTurnStateStore` impl** (`memory.rs`, inside `impl TurnStateStore for
InMemoryTurnStateStore`, lines 351+):

```rust
async fn children_of(&self, run_id: TurnRunId)
    -> Result<Vec<TurnRunRecord>, TurnError>
{
    let inner = match self.inner.lock() {
        Ok(inner) => inner,
        Err(poisoned) => poisoned.into_inner(),
    };
    let children = inner
        .records
        .values()
        .filter(|record| record.parent_run_id == Some(run_id))
        .map(RunRecord::persistence_record)
        .collect();
    Ok(children)
}

async fn get_run_record(&self, run_id: TurnRunId)
    -> Result<Option<TurnRunRecord>, TurnError>
{
    let inner = match self.inner.lock() {
        Ok(inner) => inner,
        Err(poisoned) => poisoned.into_inner(),
    };
    Ok(inner.records.get(&run_id).map(RunRecord::persistence_record))
}

async fn tree_descendant_count_and_reserve(
    &self,
    root_run_id: TurnRunId,
    delta: u32,
) -> Result<u32, TurnError> {
    let mut inner = match self.inner.lock() {
        Ok(inner) => inner,
        Err(poisoned) => poisoned.into_inner(),
    };
    // `Inner` gains a `tree_reservations: HashMap<TurnRunId, u32>` field
    // (in-memory mirror of the `SpawnTreeReservation` row keyed by root id;
    // the in-memory store is single-tenant so no tenant key needed).
    let entry = inner.tree_reservations.entry(root_run_id).or_insert(0);
    *entry = entry.saturating_add(delta);
    Ok(*entry)
}

async fn release_tree_descendants(
    &self,
    root_run_id: TurnRunId,
    delta: u32,
) -> Result<(), TurnError> {
    let mut inner = match self.inner.lock() {
        Ok(inner) => inner,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(entry) = inner.tree_reservations.get_mut(&root_run_id) {
        let prev = *entry;
        *entry = entry.saturating_sub(delta);
        if prev < delta {
            tracing::debug!(
                root = %root_run_id,
                attempted = delta,
                available = prev,
                "tree descendant release underflowed; saturated at 0"
            );
        }
        if *entry == 0 {
            inner.tree_reservations.remove(&root_run_id);
        }
    }
    Ok(())
}
```

(`Inner::records` is the `HashMap<TurnRunId, RunRecord>` — confirm field name
when implementing; `take_record` already indexes it. `Inner::tree_reservations`
is the new in-memory mirror of the `SpawnTreeReservation` durable table.)

**`LibSqlTurnStateStore` / `PostgresTurnStateStore`** (`db.rs`). `TurnRunRecord`
is stored as an opaque JSON payload (`libsql_load_payloads::<TurnRunRecord>` /
`postgres_load_payloads::<TurnRunRecord>`, lines 1060 / 1358) — the new fields
serialize into that payload with **no schema migration**. For `children_of` and
`get_run_record`,
the simplest correct Phase 1 implementation is a **load-then-filter** mirroring
the in-memory store (load run payloads, filter by `run_id` or by
`parent_run_id == Some(run_id)`). A dedicated indexed column is a deferred
optimization (README §13 "durable goal-store backend beyond the bounded
store" is the analogous deferral) — note it as a follow-up in the PR; do not
add a migration in Phase 1. If `db.rs` proves to have no whole-table load
helper, the acceptable Phase-1 fallback is a JSON-path `WHERE` filter; either
way it is a contained `db.rs` change.

### 1.8 `TurnCoordinator::prepare_turn` + `SubmitTurnRequest.requested_run_id` binding

The spawn handler (P2.A) needs the **child `TurnRunId` before any side-effect**
so the goal store row and the `SpawnTreeReservation` row can be keyed by the
final id — no staging key, no rekey. README §6 ("Goal durability (DB-backed)")
and §11 design table ("`requested_run_id` / `prepare_turn`") make this the
mechanism. The same shape generalises to missions/cron/triggers (any submitter
that needs to "persist dependent state before submit").

**Add to the `TurnCoordinator` trait:**

```rust
#[async_trait]
pub trait TurnCoordinator: Send + Sync {
    /// Mint a fresh `TurnRunId` for a future `submit_turn(scope, ...)` call,
    /// **without** any store side-effect. The caller passes the returned id
    /// back via `SubmitTurnRequest.requested_run_id`; the coordinator/store
    /// binds it instead of minting a new one.
    ///
    /// Used by the subagent spawn handler so the durable goal store and the
    /// `SpawnTreeReservation` row can be keyed by the final child id before
    /// `submit_turn`. Idempotent in the sense that the same caller calling
    /// twice yields two distinct ids — there is no de-dup; the caller owns
    /// the id from this point.
    async fn prepare_turn(&self, scope: TurnScope) -> Result<TurnRunId, TurnError>;

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError>;

    // … existing methods unchanged …
}
```

**`DefaultTurnCoordinator` impl** (pseudo code):

```rust
#[async_trait]
impl<S> TurnCoordinator for DefaultTurnCoordinator<S>
where
    S: TurnStateStore + ?Sized + 'static,
{
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        // Pure id mint — no store I/O, no admission check, no lock.
        // `_scope` is taken for forward-compat (a future tenant-scoped id
        // policy may want it) but currently unused.
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        // … existing flow, with one new precondition propagated into the
        // store: when `request.requested_run_id == Some(id)`, the store
        // binds `id` to the new run rather than minting a fresh one.
        // If the store has already claimed `id` for a different scope, the
        // store returns `TurnError::Conflict { reason: "requested_run_id
        // already bound to a different run" }`.
        let scope = request.scope.clone();
        let response = self
            .store
            .submit_turn(
                request,
                self.admission_policy.as_ref(),
                self.run_profile_resolver.as_ref(),
            )
            .await?;
        notify_queued_run_best_effort(self.wake_notifier.as_ref(), submit_wake(scope, &response));
        Ok(response)
    }
}

#[async_trait]
impl<C> TurnCoordinator for Arc<C>
where
    C: TurnCoordinator + ?Sized,
{
    async fn prepare_turn(&self, scope: TurnScope) -> Result<TurnRunId, TurnError> {
        self.as_ref().prepare_turn(scope).await
    }
    // … existing forwards unchanged …
}
```

**Store-level binding rule** (`memory.rs`, `db.rs`):

- When `submit_turn` sees `request.requested_run_id == Some(id)`, the new
  `RunRecord` / `TurnRunRecord` has `run_id = id`. The id is NOT validated
  against any prior `prepare_turn` call — `prepare_turn` is pure id-mint, the
  store does not track outstanding mints.
- If `id` collides with an existing `RunRecord`, the store returns
  `TurnError::Conflict { reason: "requested_run_id already bound" }`. Callers
  treat this as fatal (idempotency keys still apply for replay; collision
  means the caller's id-mint is broken).
- When `requested_run_id == None`, legacy behavior: store mints via
  `TurnRunId::new()`.

**Default trait impl pseudo code** — note `prepare_turn` does NOT have a
default impl on the trait (every coordinator must opt in explicitly); the
`Arc<C>` blanket forwards, which keeps existing tests / mock coordinators
working only after they implement it. This is intentional fail-loud: a
coordinator that returns a non-unique id would silently break replay.

**Unit-test stub (P1.A):**

```rust
#[tokio::test]
async fn prepare_turn_mints_unique_run_ids_without_side_effects() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coord = DefaultTurnCoordinator::new(store.clone());
    let scope = test_scope();

    let id_a = coord.prepare_turn(scope.clone()).await.unwrap();
    let id_b = coord.prepare_turn(scope.clone()).await.unwrap();
    assert_ne!(id_a, id_b, "prepare_turn must mint distinct ids");

    // No `RunRecord` was created — prepare_turn is side-effect free.
    let state = store
        .get_run_state(GetRunStateRequest { scope: scope.clone(), run_id: id_a })
        .await;
    assert!(matches!(state, Err(TurnError::ScopeNotFound)));
}

#[tokio::test]
async fn submit_turn_with_requested_run_id_binds_that_id() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coord = DefaultTurnCoordinator::new(store.clone());
    let scope = test_scope();

    let reserved = coord.prepare_turn(scope.clone()).await.unwrap();
    let req = SubmitTurnRequest {
        // … other fields …
        requested_run_id: Some(reserved),
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        scope: scope.clone(),
        // … rest …
    };
    let resp = coord.submit_turn(req).await.unwrap();
    let SubmitTurnResponse::Accepted { run_id, .. } = resp else { panic!() };
    assert_eq!(run_id, reserved, "coordinator must bind requested_run_id");
}

#[tokio::test]
async fn submit_turn_rejects_requested_run_id_collision() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coord = DefaultTurnCoordinator::new(store.clone());
    let scope = test_scope();
    let reserved = coord.prepare_turn(scope.clone()).await.unwrap();

    // First submit binds the id.
    let _ = coord.submit_turn(req_with_id(scope.clone(), reserved)).await.unwrap();
    // Second submit with the SAME id must fail loud.
    let err = coord
        .submit_turn(req_with_id(scope.clone(), reserved))
        .await
        .unwrap_err();
    assert!(matches!(err, TurnError::Conflict { .. }));
}
```

### 1.9 `DefaultTurnCoordinator` event sink

`TurnEventSink` already exists in `ironclaw_turns::events`, and the stores
already persist `TurnLifecycleEvent`s. The missing live seam is that
`DefaultTurnCoordinator` does not publish those events to an injected sink, so a
`SubagentCompletionObserver` would otherwise have no reliable notification path.

Add an optional sink to `DefaultTurnCoordinator`:

```rust
pub struct DefaultTurnCoordinator<S: ?Sized> {
    store: Arc<S>,
    admission_policy: Arc<dyn TurnAdmissionPolicy>,
    run_profile_resolver: Arc<dyn RunProfileResolver>,
    wake_notifier: Arc<dyn TurnRunWakeNotifier>,
    event_sink: Option<Arc<dyn TurnEventSink>>,
}

impl<S> DefaultTurnCoordinator<S>
where
    S: TurnStateStore + ?Sized,
{
    pub fn with_event_sink(mut self, sink: Arc<dyn TurnEventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }
}
```

After successful `submit_turn`, `resume_turn`, and `cancel_run`, construct the
corresponding `TurnLifecycleEvent` from the operation response (`Submitted`,
`Resumed`, `Cancelled` / `CancelRequested` as appropriate) and publish it
best-effort to the sink. Sink failure is logged and does not roll back the
coordinator operation; the store remains the durable source of truth.

### 1.9 Unit tests to add (P1.A)

Place in the existing `crates/ironclaw_turns/tests/` integration tests and/or
`#[cfg(test)] mod tests` blocks. Required:

1. **`CapabilityOutcome::SpawnedChildRun` serde round-trip** — serialize and
   deserialize `SpawnedChildRun { child_run_id, result_ref, safe_summary }`;
   assert the wire JSON is
   `{"spawned_child_run":{"child_run_id":"<uuid>","result_ref":"result:…","safe_summary":"…"}}`;
   assert `is_suspension() == false`.

2. **`CapabilityOutcome::AwaitDependentRun` serde round-trip** — serialize and
   deserialize `AwaitDependentRun { gate_ref, safe_summary }`; assert wire JSON
   is `{"await_dependent_run":{"gate_ref":"gate:…","safe_summary":"…"}}`;
   assert `is_suspension() == true`.

3. **`LoopGateKind::AwaitDependentRun` round-trip** — assert wire string
   `"await_dependent_run"`; round-trips for all four variants.

4. **`LoopBlockedKind::AwaitDependentRun` round-trip + `to_blocked_reason`** —
   assert `"await_dependent_run"`; assert
   `LoopBlockedKind::AwaitDependentRun.to_blocked_reason(gate)` yields
   `BlockedReason::DependentRun { gate_ref }` (use the existing private-fn test
   pattern in `loop_exit/tests/`, or expose via a `LoopBlocked` validate path).

5. **`TurnStatus::BlockedDependentRun` serde round-trip** — assert wire string
   `"BlockedDependentRun"` (PascalCase), `is_terminal() == false`,
   `keeps_active_lock() == true`.

6. **`TurnStatus` legacy-JSON deserialization** — deserialize each *pre-existing*
   variant from its already-persisted wire string
   (`"Queued"`, `"BlockedApproval"`, …) and assert it still decodes — proves the
   new variant did not perturb the wire format of old data.

7. **`BlockedReason::DependentRun` round-trip + mapping** — round-trip the JSON
   `{"DependentRun":{"gate_ref":"…"}}`; assert
   `status() == TurnStatus::BlockedDependentRun` and `gate_ref()` returns the
   ref.

8. **`SubmitTurnRequest` lineage defaults + explicit child lineage** — deserialize
   legacy request JSON with no lineage keys and assert `None`/`0`; construct a
   child request with `Some(parent_run_id)` / `2` and assert the memory store
   persists those values into the emitted `TurnRunRecord`.

9. **`TurnRunRecord` legacy-JSON deserialization** — take a `TurnRunRecord` JSON
   blob with **no** `parent_run_id` / `subagent_depth` keys; assert it
   deserializes with `parent_run_id == None`, `subagent_depth == 0`. Then
   round-trip a child record with `parent_run_id = Some(..)`,
   `subagent_depth = 2` and assert field equality.

10. **`children_of` / `get_run_record` semantics** (in-memory store contract test, in
   `turn_coordinator_contract.rs` style) — submit a parent run; submit two child
   runs using `SubmitTurnRequest.parent_run_id = Some(parent)`. Assert
   `children_of(parent)` returns exactly the two children;
   `children_of(unknown_run_id)` returns `Ok(vec![])`; `children_of(child)`
   returns `Ok(vec![])`; `get_run_record(child)` returns the durable child
   record; `get_run_record(unknown_run_id)` returns `Ok(None)`.

11. **Resume of a `BlockedDependentRun` run** — drive a `RunRecord` to
   `BlockedDependentRun` (via `block_claimed_record` with a
   `BlockedReason::DependentRun`) and assert `resume_turn` succeeds (not
   `InvalidTransition`) — regression guard for the §1.4 match-arm edit.

12. **Cancel of a `BlockedDependentRun` run** — assert `request_cancel` of a
    `BlockedDependentRun` run transitions directly to `Cancelled` (not
    `CancelRequested`, not `already_terminal`) — regression guard for the §1.4
    cancel-match edit.

---

## 2. P1.B — `ironclaw_agent_loop`: `subagent` family + `GateKind::AwaitDependentRun`

**Goal:** add the static `subagent` `LoopFamily` factory and the
`GateKind::AwaitDependentRun` strategy-side variant plus the mechanical executor
maps for the new gate/result outcomes.

### 2.1 Files to create / modify

| File | Change |
|---|---|
| `crates/ironclaw_agent_loop/src/families/subagent.rs` | **new** — `subagent` family factory, fingerprint, digest |
| `crates/ironclaw_agent_loop/src/families/mod.rs` | declare `subagent` module + re-export `subagent::subagent()` and `SUBAGENT_FAMILY_DIGEST` |
| `crates/ironclaw_agent_loop/src/family.rs` | + `LoopFamilyId::SUBAGENT` associated const |
| `crates/ironclaw_agent_loop/src/strategies/gate.rs` | + `GateKind::AwaitDependentRun` |
| `crates/ironclaw_agent_loop/src/strategies/mod.rs` | no change — `GateKind` already re-exported |

> **[CORRECTION]** `families/mod.rs` today is *not* a module-directory hub — it
> contains the `default()` family inline (the directory only holds `mod.rs` +
> `CLAUDE.md`). Adding `families/subagent.rs` means `families/mod.rs` must gain
> `mod subagent;` and a re-export. The `families/CLAUDE.md` explicitly says "Add
> a new family file when a built-in loop family needs a distinct strategy
> composition" — so a new file is the sanctioned shape.

### 2.2 `LoopFamilyId::SUBAGENT`

**Current** (`family.rs` lines 14–15): `LoopFamilyId` has one associated const,
`DEFAULT`. Add a second:

```rust
impl LoopFamilyId {
    pub const DEFAULT: Self = Self(Cow::Borrowed("default"));
    /// The static subagent loop family — a child agent loop with a fresh
    /// context, attenuated surface, tighter budget.
    pub const SUBAGENT: Self = Self(Cow::Borrowed("subagent"));

    pub fn new(/* … unchanged … */) -> Result<Self, String> { /* … */ }
}
```

`"subagent"` passes `validate_loop_family_id` (lowercase ASCII letters only).
No deserialization change — `LoopFamilyId` deserializes from any valid string;
the registry is the authority on whether an id is bound.

### 2.3 The `subagent` family factory (`families/subagent.rs`)

The `subagent` family is a `DefaultPlanner` composition: **all default
strategies except `BudgetStrategy`**, which is a tighter `DefaultBudgetStrategy`
instance. The framework already supports this exactly:
`DefaultPlanner::compose_default()` builds the all-default planner, and
`DefaultPlanner::with_budget(Arc<dyn BudgetStrategy>)` swaps one slot
(`default_planner.rs` lines 122–125). `DefaultBudgetStrategy` is a public struct
with public fields (`budget.rs` lines 30–45) and its doc explicitly says "Loop
families that need shorter or longer budgets construct this struct directly."

> **[CORRECTION]** The overarching doc §6 says the subagent family uses
> "default strategies + a tighter `BudgetStrategy`". Accurate. Note the budget
> *iteration* cap lives in the `BudgetStrategy`; the *token/cost* budget the
> design's flavor table mentions is **not** a `BudgetStrategy` concern —
> `BudgetStrategy` only owns `iteration_limit` + `wall_clock_limit` (see the
> module doc in `budget.rs`: "Model-call and capability-call caps belong to
> `ResolvedRunProfile.resource_budget_policy`"). Token/cost budgets are carried
> by the P1.C flavor table and resolved into the child's `ResolvedRunProfile`
> by P2.C — they are not part of P1.B. P1.B's budget tightening is purely the
> iteration cap (and optionally a wall-clock cap).

```rust
//! `families/subagent.rs`
//!
//! The `subagent` loop family — a child agent loop. Composition is the default
//! nine-strategy set with one override: a tighter `DefaultBudgetStrategy`
//! (lower iteration ceiling than the 32-iteration default).

use std::sync::Arc;
use std::time::Duration;

use crate::default_planner::DefaultPlanner;
use crate::family::{ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId};
use crate::planner::AgentLoopPlanner;
use crate::strategies::DefaultBudgetStrategy;

/// Iteration ceiling for the subagent family. Lower than the default 32:
/// subagents are scoped, single-purpose runs. The per-flavor iteration budget
/// in `ironclaw_reborn` is resolved into the run profile (P1.C / P2.C); this
/// is the family-level hard safety net.
const SUBAGENT_ITERATION_LIMIT: u32 = 16;

/// Optional family-level wall-clock cap. `None` keeps parity with the default
/// family (no wall-clock limit); a future tightening can set this.
const SUBAGENT_WALL_CLOCK_LIMIT: Option<Duration> = None;

#[cfg(test)]
const SUBAGENT_FAMILY_FINGERPRINT: &[u8] = concat!(
    "ironclaw_agent_loop.subagent_family.v1:",
    "family_id=subagent;",
    "identity=component_identity_v1;",
    "planner=DefaultPlanner;",
    "strategies=",
    "context:DefaultContextStrategy(max_messages=16),",
    "capability:DefaultCapabilityStrategy(all),",
    "model:DefaultModelStrategy(primary_or_fallback_index),",
    "batch:DefaultBatchPolicyStrategy(exclusive_sequential),",
    "gate:DefaultGateHandlingStrategy(block),",
    "recovery:DefaultRecoveryStrategy(max_attempts_per_class=2),",
    "stop:DefaultStopConditionStrategy(window=5,repeat=3,failure_run=3),",
    "drain:DefaultInputDrainStrategy(steering=true,followup=true),",
    "budget:DefaultBudgetStrategy(iteration_limit=16,wall_clock_limit=none)"
)
.as_bytes();

/// Stable digest: BLAKE3-256 of `SUBAGENT_FAMILY_FINGERPRINT`.
///
/// Update this digest whenever the subagent family composition changes in a
/// replay-relevant way. The placeholder below MUST be replaced with the real
/// BLAKE3 hash — see the test in §2.6 which derives and asserts it.
pub const SUBAGENT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    // PLACEHOLDER — run `subagent_family_digest_matches_blake3_fingerprint`
    // once; copy the printed bytes here. Mirrors how DEFAULT_FAMILY_DIGEST
    // was produced (families/mod.rs lines 30–33).
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
]);

/// The `subagent` loop family: default composition with a tighter budget.
pub fn subagent() -> LoopFamily {
    let budget = Arc::new(DefaultBudgetStrategy {
        iteration_limit: SUBAGENT_ITERATION_LIMIT,
        wall_clock_limit: SUBAGENT_WALL_CLOCK_LIMIT,
    });

    let planner = DefaultPlanner::compose_default()
        .with_id(LoopFamilyId::SUBAGENT)
        .with_version(ComponentIdentity::from_static(
            "subagent",
            SUBAGENT_FAMILY_DIGEST,
        ))
        .with_budget(budget);

    let id = planner.id().clone();
    let version = planner.version().clone();
    LoopFamily::new(id, version, Arc::new(planner))
}
```

`families/mod.rs` additions:

```rust
mod subagent; // add near the top of mod.rs

pub use subagent::{subagent, SUBAGENT_FAMILY_DIGEST};
```

(`families/mod.rs` currently has no `pub use` block — `default()` is a free fn
in the file itself. Add `mod subagent;` and the `pub use`. Keep `default()`
where it is; do not refactor it in this PR.)

Construction notes — all verified against the real code:
- `DefaultPlanner::compose_default()`, `.with_id`, `.with_version`,
  `.with_budget` are all `pub(crate)` — `families/subagent.rs` is in the same
  crate, so they are callable. ✅
- `DefaultBudgetStrategy` and its fields `iteration_limit` / `wall_clock_limit`
  are fully `pub` (`budget.rs`). ✅
- `LoopFamily::new` is `pub(crate)`. ✅
- `ComponentIdentity::from_static` is `pub const fn`. ✅
- `DefaultPlanner` is `Clone` and `Send + Sync` (`default_planner.rs` test
  `default_planner_is_send_sync_and_clone`), so `Arc::new(planner)` satisfies
  `Arc<dyn AgentLoopPlannerInternal>`. ✅

### 2.4 `GateKind::AwaitDependentRun`

**Current** (`strategies/gate.rs` lines 59–104):

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GateKind {
    Approval,
    Auth,
    Resource,
}
```

**After:**

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GateKind {
    Approval,
    Auth,
    Resource,
    /// The capability returned an `AwaitDependentRun` gate — the loop is to
    /// block awaiting a dependent (child) run set. The executor checkpoints
    /// `BeforeBlock` and returns `LoopExit::Blocked`, exactly as for
    /// `Approval`.
    AwaitDependentRun,
}
```

`GateKind` is already `#[non_exhaustive]` and `pub(crate)`. Wire string:
`"await_dependent_run"`.

**`GateOutcome::validate_for_gate_kind`** (`gate.rs` lines 96–104) — this is a
`match (kind, self)` whose only non-`_` arm is `(Approval, SkipAndContinue)`.
`AwaitDependentRun` falls into the existing `_ => Ok(())` arm. **Decision:** an
`AwaitDependentRun` gate must never be skipped-and-continued (skipping a child
dependency would silently drop the awaited result). Tighten the match:

```rust
pub(crate) fn validate_for_gate_kind(&self, kind: GateKind) -> Result<(), LoopFailureKind> {
    match (kind, self) {
        (GateKind::Approval, GateOutcome::SkipAndContinue { .. })
        | (GateKind::AwaitDependentRun, GateOutcome::SkipAndContinue { .. }) => {
            Err(LoopFailureKind::DriverBug)
        }
        _ => Ok(()),
    }
}
```

The `DefaultGateHandlingStrategy` always returns `Block` for *every* kind
(`gate.rs` lines 38–45 — `_gate` is ignored), so the `subagent` family — which
keeps the default gate strategy — already does the right thing for an
`AwaitDependentRun` gate with **no strategy change**. The `validate_for_gate_kind`
tightening is a defense-in-depth guard against a future custom gate strategy.

> **[CORRECTION]** The overarching doc §11 P1.B says "`GateKind::AwaitDependentRun`
> in `strategies/gate.rs`" — accurate. It does not mention `GateSummary`, but
> `GateSummary { kind: GateKind, gate_ref }` (`gate.rs` lines 51–55) carries
> `GateKind` and gets the new variant for free; its serde round-trip test should
> gain an `AwaitDependentRun` case (§2.6).

### 2.5 Minimal executor maps in Phase 1

`executor.rs` has three relevant exhaustive matches — `handle_gate` dispatch
(produces `GateKind`), `blocked_kind(GateKind) -> LoopBlockedKind` (lines
1404–1409), `loop_gate_kind(GateKind) -> LoopGateKind` (lines 1412–1417). Adding
`GateKind::AwaitDependentRun` makes `blocked_kind` and `loop_gate_kind`
non-exhaustive → **`ironclaw_agent_loop` will not compile** until those arms are
added.

**Phase 1 ordering decision:** the minimal arm additions to `blocked_kind` and
`loop_gate_kind` are part of **P1.B** (they are pure, mechanical 1:1 maps in the
same crate as `GateKind`), even though the *capability path that emits* an
`AwaitDependentRun` gate is Phase 2/3. Land them now so `ironclaw_agent_loop`
stays green:

```rust
// executor.rs — blocked_kind
fn blocked_kind(kind: GateKind) -> LoopBlockedKind {
    match kind {
        GateKind::Approval => LoopBlockedKind::Approval,
        GateKind::Auth => LoopBlockedKind::Auth,
        GateKind::Resource => LoopBlockedKind::Resource,
        GateKind::AwaitDependentRun => LoopBlockedKind::AwaitDependentRun,
    }
}

// executor.rs — loop_gate_kind
fn loop_gate_kind(kind: GateKind) -> LoopGateKind {
    match kind {
        GateKind::Approval => LoopGateKind::Approval,
        GateKind::Auth => LoopGateKind::Auth,
        GateKind::Resource => LoopGateKind::Resource,
        GateKind::AwaitDependentRun => LoopGateKind::AwaitDependentRun,
    }
}
```

This makes **P1.B depend on P1.A's `LoopBlockedKind`/`LoopGateKind` variants at
compile time** — see §4 ordering. The `handle_capability_outcome` match on
`CapabilityOutcome` (§1.2) routing for `AwaitDependentRun` and
`SpawnedChildRun` is also a mechanical executor map. It can land in the same
agent-loop PR as P1.B after P1.A is available, or in a tiny companion PR stacked
on P1.A. Do not leave it until Phase 3 if workspace CI must stay green: adding
the `CapabilityOutcome` variants intentionally breaks the exhaustive match until
these arms exist. See §4.

### 2.6 Unit tests to add (P1.B)

In `families/subagent.rs` `#[cfg(test)] mod tests` (mirror `families/mod.rs`
tests):

1. **`subagent_family_has_subagent_identity`** — `subagent()` family;
   `family.id() == &LoopFamilyId::SUBAGENT`; `family.version().id == "subagent"`;
   `family.version().digest == SUBAGENT_FAMILY_DIGEST`;
   `digest != ComponentDigest([0; 32])`.

2. **`subagent_family_digest_matches_blake3_fingerprint`** — assert
   `SUBAGENT_FAMILY_DIGEST == ComponentDigest::from_blake3(SUBAGENT_FAMILY_FINGERPRINT)`.
   This is the test that *produces* the digest: first run it with the
   placeholder, copy the printed expected bytes into `SUBAGENT_FAMILY_DIGEST`,
   re-run green. (Same procedure as `DEFAULT_FAMILY_DIGEST`.)

3. **`subagent_family_digest_differs_from_default`** — assert
   `SUBAGENT_FAMILY_DIGEST != DEFAULT_FAMILY_DIGEST` (the iteration limit is in
   the fingerprint, so the digests must differ — a same-digest collision would
   break replay disambiguation).

4. **`subagent_family_budget_is_tightened`** — build `subagent()`, reach the
   planner's budget via the crate-internal accessor pattern used in
   `default_planner.rs::crate_private_internal_accessors_are_wired`
   (`planner.budget().iteration_limit(&state)`), assert it returns
   `SUBAGENT_ITERATION_LIMIT` (16), not 32.

5. **`subagent_family_keeps_default_non_budget_strategies`** — assert
   `planner.batch().policy(&state, &[]) == BatchPolicy::Parallel` and
   `planner.capability().filter(&state).await == CapabilityFilter::All` — proves
   only the budget slot was overridden.

In `families/mod.rs` — extend `LoopFamilyId` serde test
(`loop_family_id_default_is_flat_string` analog) to cover `SUBAGENT`
serializing as `"subagent"` and round-tripping.

In `strategies/gate.rs` `#[cfg(test)] mod tests`:

6. **`gate_kind_round_trips_snake_case`** — extend the existing test's table to
   add `(GateKind::AwaitDependentRun, "await_dependent_run")`.

7. **`gate_summary_round_trips`** — add an `AwaitDependentRun` case.

8. **`await_dependent_run_gate_rejects_skip_and_continue`** — assert
   `GateOutcome::SkipAndContinue{..}.validate_for_gate_kind(GateKind::AwaitDependentRun)
   == Err(LoopFailureKind::DriverBug)`.

9. **`default_gate_handling_strategy_blocks_for_await_dependent_run`** — extend
   the existing `default_gate_handling_strategy_blocks_for_every_kind` loop to
   include `GateKind::AwaitDependentRun` and assert it yields `GateOutcome::Block`.

---

## 3. P1.C — `ironclaw_reborn` data: directions, bounded goal store, flavor table

**Goal:** land the *pure data* the subagent feature needs in `ironclaw_reborn` —
direction prompt `.md` files, a durable bounded goal store, and the static
built-in flavor table. No driver, no observer, no wiring (those are Phase 2).

### 3.1 Files to create

| File | Purpose |
|---|---|
| `crates/ironclaw_reborn/src/directions/mod.rs` | `DirectionId` newtype + static `direction_prompt(DirectionId) -> &'static str` |
| `crates/ironclaw_reborn/src/directions/general.md` | direction prompt for the `general` flavor |
| `crates/ironclaw_reborn/src/directions/researcher.md` | direction prompt for the `researcher` flavor |
| `crates/ironclaw_reborn/src/subagent/mod.rs` | module hub: re-exports flavor table + goal store |
| `crates/ironclaw_reborn/src/subagent/flavors.rs` | static built-in subagent flavor table |
| `crates/ironclaw_reborn/src/subagent/goal_store.rs` | durable, bounded subagent goal store |
| `crates/ironclaw_reborn/src/lib.rs` | + `pub mod directions;` + `pub mod subagent;` |

> **[NOTE]** `ironclaw_reborn/src` is currently a *flat* directory of modules
> plus two subdirectories (`loop_exit_applier/`, `turn_runner/`). `directions/`
> and `subagent/` follow that same module-directory pattern. The crate
> `CLAUDE.md` says "Add a new file when adding a new … concern" and "The public
> surface here is intentionally a directory of modules" — so `pub mod` entries
> in `lib.rs` (not a wall of `pub use`) is the sanctioned shape.

`include_str!` paths are relative to the **including `.rs` file**, so
`directions/mod.rs` does `include_str!("general.md")` for a sibling file.

### 3.2 `directions/` — direction prompt files

`general.md` and `researcher.md` are authored static system-prompt text. They
are the **system message** for a child run (README §6 "Direction prompt"). They
must be authored as plain instruction prose with **no template placeholders**
(the goal/handoff is injected as a separate *user* message — README §8.4
prompt-injection isolation). Suggested content shape (the implementer writes the
real prose; this is the contract):

- `general.md` — a general-purpose helpful subagent persona: scope discipline
  ("you are a focused sub-task agent; complete exactly the task given and
  report back"), no speculative side effects, return a concise structured
  result.
- `researcher.md` — a read/gather-oriented persona: emphasises information
  gathering, citing sources, not mutating state, summarising findings.

`directions/mod.rs`:

```rust
//! Static subagent direction prompts.
//!
//! A "direction" is the authored system-prompt persona for a subagent flavor.
//! Prompts are static `.md` files compiled in via `include_str!` — never
//! Rust string constants (project rule: prompt templates live in files), and
//! never built from model-generated content (security: the system message is
//! authored-only; the goal goes in a user message — see design §8.4).

/// Identity of a built-in direction. Closed set — directions are static, not
/// file-discovered (design non-goal: no user-defined flavors in v1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DirectionId {
    General,
    Researcher,
}

impl DirectionId {
    /// Stable wire/identifier string for this direction.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Researcher => "researcher",
        }
    }
}

const GENERAL_DIRECTION: &str = include_str!("general.md");
const RESEARCHER_DIRECTION: &str = include_str!("researcher.md");

/// Return the static system-prompt text for a direction.
///
/// Infallible by construction: every `DirectionId` variant maps to a compiled
/// `include_str!`. A missing file is a compile error, not a runtime miss.
pub fn direction_prompt(id: DirectionId) -> &'static str {
    match id {
        DirectionId::General => GENERAL_DIRECTION,
        DirectionId::Researcher => RESEARCHER_DIRECTION,
    }
}
```

`DirectionId` is intentionally *not* serde-derived in Phase 1 — it is selected
by the flavor table (in-process), not persisted on the wire. If Phase 2 needs it
on a wire contract, add `Serialize`/`Deserialize` then.

### 3.3 `subagent/flavors.rs` — the static built-in flavor table

A **flavor** is a built-in subagent kind. v1 has exactly two: `general` and
`researcher` (README §6 "Flavors"). The table is a compile-time `&[…]` — no
plugin loading, no file discovery (design principle 3, non-goal in §2).

Per the design each flavor carries: direction id, tool allowlist, model,
iteration budget, token budget, cost budget, `allow_nesting`. Modeled as:

```rust
//! `subagent/flavors.rs` — the static built-in subagent flavor table.
//!
//! A flavor is a built-in subagent kind. The table is closed at compile time
//! (design: static over dynamic; no plugin loader). Phase 1 lands the table
//! as pure data; P2.C resolves a flavor into a child `ResolvedRunProfile`.

use crate::directions::DirectionId;

/// Identity of a built-in subagent flavor. Closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubagentFlavorId {
    General,
    Researcher,
}

impl SubagentFlavorId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Researcher => "researcher",
        }
    }
}

/// A capability/tool allowlist entry. The allowlist is a *surface ceiling*, not
/// authority (design §8.1: a child starts with an empty grant/lease set).
/// Entries are capability-id strings matched against the resolved surface by
/// P2.B attenuation; kept as `&'static str` because the table is static.
pub type ToolAllowEntry = &'static str;

/// Per-flavor resource budget. Iteration cap is the loop-family safety net's
/// per-flavor override; token/cost caps are resolved into the child run
/// profile's `resource_budget_policy` by P2.C. All three are hard ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentBudget {
    /// Max loop iterations for a run of this flavor.
    pub iteration_limit: u32,
    /// Max cumulative model input+output tokens. `None` = profile default.
    pub max_total_tokens: Option<u64>,
    /// Max cumulative cost in micro-USD (integer to stay `Eq`/`Hash`-able and
    /// avoid float wire drift). `None` = profile default.
    pub max_cost_micro_usd: Option<u64>,
}

/// One built-in subagent flavor — pure static data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentFlavor {
    pub id: SubagentFlavorId,
    /// The direction (system-prompt persona) for this flavor.
    pub direction: DirectionId,
    /// Capability/tool surface ceiling for child runs of this flavor.
    pub tool_allowlist: &'static [ToolAllowEntry],
    /// Model identifier for the child run. A stable string resolved into a
    /// `ModelProfileId` by P2.C — kept as `&'static str` here so the table is
    /// fully `const`.
    pub model: &'static str,
    /// Hard resource caps.
    pub budget: SubagentBudget,
    /// If `false`, a `spawn_subagent` call from a run of this flavor is
    /// rejected outright (design §8.3 nesting hard gate) — independent of
    /// whether `spawn_subagent` appears in `tool_allowlist`.
    pub allow_nesting: bool,
}

/// `general` flavor surface: deliberately broad-but-safe; explicitly excludes
/// `spawn_subagent` AND sets `allow_nesting = false` (defense in depth).
const GENERAL_TOOLS: &[ToolAllowEntry] = &[
    "file_read",
    "list_dir",
    "web_fetch",
    "shell",
    // NOTE: "spawn_subagent" intentionally absent; allow_nesting=false is the
    // load-bearing gate (design §8.3).
];

/// `researcher` flavor surface: read/gather only — no shell, no writes.
const RESEARCHER_TOOLS: &[ToolAllowEntry] = &[
    "file_read",
    "list_dir",
    "web_fetch",
];

/// The closed built-in flavor table.
pub static BUILTIN_SUBAGENT_FLAVORS: &[SubagentFlavor] = &[
    SubagentFlavor {
        id: SubagentFlavorId::General,
        direction: DirectionId::General,
        tool_allowlist: GENERAL_TOOLS,
        model: "default",
        budget: SubagentBudget {
            iteration_limit: 16,
            max_total_tokens: Some(200_000),
            max_cost_micro_usd: Some(500_000), // $0.50
        },
        allow_nesting: false,
    },
    SubagentFlavor {
        id: SubagentFlavorId::Researcher,
        direction: DirectionId::Researcher,
        tool_allowlist: RESEARCHER_TOOLS,
        model: "default",
        budget: SubagentBudget {
            iteration_limit: 12,
            max_total_tokens: Some(150_000),
            max_cost_micro_usd: Some(300_000), // $0.30
        },
        allow_nesting: false,
    },
];

/// Resolve a flavor by id. Returns `None` for an unknown id — callers must
/// fail loud (design §4: no silent fallbacks).
pub fn lookup_flavor(id: SubagentFlavorId) -> Option<&'static SubagentFlavor> {
    BUILTIN_SUBAGENT_FLAVORS.iter().find(|flavor| flavor.id == id)
}
```

Design notes baked in:
- Concrete budget numbers above are **proposed defaults** — the implementer
  confirms them with the design owner; the *shape* is the contract.
- `max_cost_micro_usd` is an integer (micro-USD) deliberately: floats break
  `Eq`/`Hash` and risk wire drift. README §6 says "token/cost budget" without a
  unit — integer micro-USD is the chosen representation.
- `allow_nesting = false` for **both** v1 flavors — v1 has no nesting use case;
  the field exists so the hard gate (design §8.3) has something to read, and so
  a future flavor can opt in.
- `model: "default"` is a placeholder string; P2.C maps it to a real
  `ModelProfileId`. Keeping it `&'static str` keeps the whole table `const`.

### 3.4 `subagent/goal_store.rs` — durable, bounded goal store

The goal store holds the parent-injected goal (+ optional `Handoff` blob) keyed
by **child `TurnRunId`**. README §6 / §9: durable, **bounded** (hard entry cap +
eviction), a store miss fails the child run loudly.

Phase 1 ships the **bounded in-process store** (README §13 explicitly defers "a
durable goal-store backend beyond the bounded in-process store"). "Durable"
here means: it survives within the process lifetime and is the *single source
of truth* (not a cache that can silently lose entries) — a real disk/DB backend
is a deferred follow-up. The Phase 1 store is in-memory but **fail-loud and
bounded**.

```rust
//! `subagent/goal_store.rs` — bounded subagent goal store.
//!
//! Holds the parent-injected goal for a child run, keyed by the child's
//! `TurnRunId`. Bounded: a hard entry cap with eviction of the oldest entry.
//! A `get` miss is an error, never an empty goal (design §6 goal durability:
//! a miss "fails the child run loudly").

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Mutex;

use ironclaw_turns::TurnRunId;

/// Hard cap on stored goals. Eviction is oldest-first when the cap is reached.
/// Sized well above any plausible concurrent in-flight subagent count; an
/// eviction under normal load indicates a leak and is logged at `debug`.
const MAX_GOAL_ENTRIES: usize = 4096;

/// Maximum byte length of a stored goal payload. A larger payload is rejected
/// at `put` time — the goal is model-generated and must not be unbounded.
const MAX_GOAL_BYTES: usize = 64 * 1024;

/// The parent-injected goal for a child run.
///
/// `task` is the `## Task (from parent)` body; `handoff` is the optional
/// `## Context from parent` blob (design §6 context seed: `Fresh` => `handoff`
/// is `None`, `Handoff(String)` => `Some`). Both are untrusted, model-generated
/// content — they are placed in the child's *user* message by P2.B, never the
/// system message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentGoal {
    pub task: String,
    pub handoff: Option<String>,
}

impl SubagentGoal {
    fn byte_len(&self) -> usize {
        self.task.len() + self.handoff.as_deref().map_or(0, str::len)
    }
}

/// Errors from the goal store. Distinct typed variants — design §4 fail-loud.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubagentGoalStoreError {
    #[error("subagent goal for run {run_id} not found")]
    NotFound { run_id: TurnRunId },
    #[error("subagent goal payload too large: {bytes} bytes (max {max})")]
    PayloadTooLarge { bytes: usize, max: usize },
    #[error("subagent goal for run {run_id} already stored")]
    DuplicateKey { run_id: TurnRunId },
}

/// Bounded, in-process, fail-loud subagent goal store.
///
/// Thread-safe (`Mutex`). The `insertion_order` deque tracks FIFO eviction
/// order so the cap is enforced in O(1) amortized.
pub struct SubagentGoalStore {
    inner: Mutex<GoalStoreInner>,
}

struct GoalStoreInner {
    goals: HashMap<TurnRunId, SubagentGoal>,
    insertion_order: VecDeque<TurnRunId>,
}

impl SubagentGoalStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(GoalStoreInner {
                goals: HashMap::new(),
                insertion_order: VecDeque::new(),
            }),
        }
    }

    /// Store the goal for a child run.
    ///
    /// - Rejects a payload over `MAX_GOAL_BYTES` (`PayloadTooLarge`).
    /// - Rejects a re-insert of an existing key (`DuplicateKey`) — child run
    ///   ids are unique per spawn, so a duplicate is a bug, not a refresh.
    /// - When at `MAX_GOAL_ENTRIES`, evicts the oldest entry first.
    pub fn put(
        &self,
        run_id: TurnRunId,
        goal: SubagentGoal,
    ) -> Result<(), SubagentGoalStoreError> {
        let bytes = goal.byte_len();
        if bytes > MAX_GOAL_BYTES {
            return Err(SubagentGoalStoreError::PayloadTooLarge {
                bytes,
                max: MAX_GOAL_BYTES,
            });
        }
        let mut inner = lock(&self.inner);
        if inner.goals.contains_key(&run_id) {
            return Err(SubagentGoalStoreError::DuplicateKey { run_id });
        }
        if inner.goals.len() >= MAX_GOAL_ENTRIES {
            // Evict oldest. The loop drains stale order entries whose key was
            // already removed by a prior `take`.
            while let Some(oldest) = inner.insertion_order.pop_front() {
                if inner.goals.remove(&oldest).is_some() {
                    tracing::debug!(
                        evicted_run_id = %oldest,
                        "subagent goal store at capacity; evicted oldest goal"
                    );
                    break;
                }
            }
        }
        inner.goals.insert(run_id, goal);
        inner.insertion_order.push_back(run_id);
        Ok(())
    }

    /// Fetch the goal for a child run. A miss is a hard error — the caller
    /// (P2.B prompt composition) must fail the child run, never proceed with an
    /// empty `## Task`.
    pub fn get(&self, run_id: TurnRunId) -> Result<SubagentGoal, SubagentGoalStoreError> {
        let inner = lock(&self.inner);
        inner
            .goals
            .get(&run_id)
            .cloned()
            .ok_or(SubagentGoalStoreError::NotFound { run_id })
    }
}

impl Default for SubagentGoalStore {
    fn default() -> Self {
        Self::new()
    }
}

fn lock(inner: &Mutex<GoalStoreInner>) -> std::sync::MutexGuard<'_, GoalStoreInner> {
    // The store holds no `LLM data` — only an in-flight goal. A poisoned mutex
    // is recoverable here; mirror the `InMemoryTurnStateStore` pattern.
    match inner.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
```

Design decisions:
- **`get` borrows-and-clones, does not remove.** README §6 says `put`/`get`;
  it does not say `get` consumes. A child may need its goal more than once
  (e.g. recovery re-materialisation), and a process restart must be able to
  re-read it. Eviction is by cap, not by read. (If Phase 2 finds it genuinely
  needs a one-shot `take`, add it then — but the contract here is non-consuming
  `get`.)
- **`DuplicateKey` is an error.** Child `TurnRunId`s are freshly minted per
  spawn; a duplicate `put` means a wiring bug — fail loud.
- `tracing::debug!` for eviction, never `info!`/`warn!` (project rule: `info!`
  corrupts the REPL/TUI; background subagent paths must use `debug!`).
- The store is `Send + Sync` (`Mutex` + `Send` contents) so Phase 2 can wrap it
  in `Arc` and share it with the capability port and the prompt port.

### 3.5 `lib.rs` + `subagent/mod.rs` wiring

`subagent/mod.rs`:

```rust
//! Subagent static data: built-in flavor table and the bounded goal store.

pub mod flavors;
pub mod goal_store;
```

`lib.rs` — add to the `pub mod` block (after `pub mod app_loop_family;`,
alphabetical-ish placement is fine):

```rust
pub mod directions;
pub mod subagent;
```

No `pub use` flattening — consistent with the `lib.rs` doc comment ("a directory
of modules, not a shopping list of types"). Downstream Phase 2 code reaches in
by path: `ironclaw_reborn::subagent::flavors::lookup_flavor`,
`ironclaw_reborn::subagent::goal_store::SubagentGoalStore`,
`ironclaw_reborn::directions::direction_prompt`.

`thiserror` must be available to `ironclaw_reborn` for `SubagentGoalStoreError`.
Check `crates/ironclaw_reborn/Cargo.toml` `[dependencies]` — it is **not**
currently listed (the crate uses `serde`, `tracing`, etc.). **Add
`thiserror = "1"`** to `[dependencies]` as part of P1.C (every other crate in
the workspace pins `thiserror = "1"` per `error.rs` convention).

### 3.6 Unit tests to add (P1.C)

In `subagent/goal_store.rs` `#[cfg(test)] mod tests`:

1. **`put_then_get_round_trips`** — `put(run_id, goal)`; `get(run_id)` returns an
   equal `SubagentGoal` (both `Fresh` — `handoff: None` — and `Handoff` —
   `handoff: Some(..)` — cases).

2. **`get_miss_is_not_found_error`** — `get(unknown_run_id)` returns
   `Err(SubagentGoalStoreError::NotFound { .. })`. (Proves the fail-loud
   contract — design §6.)

3. **`put_rejects_oversized_payload`** — `put` a goal whose `byte_len()` exceeds
   `MAX_GOAL_BYTES` → `Err(PayloadTooLarge { .. })`.

4. **`put_rejects_duplicate_key`** — `put` twice for the same `run_id` → second
   call `Err(DuplicateKey { .. })`.

5. **`bounded_store_evicts_oldest`** — insert `MAX_GOAL_ENTRIES + 1` goals;
   assert the very first inserted key is now a `get` miss (`NotFound`), the
   second-inserted and the last-inserted keys are still present. Confirms FIFO
   eviction and the cap.

6. **`bounded_store_stays_at_cap`** — after inserting `MAX_GOAL_ENTRIES + N`
   entries, assert the live entry count never exceeds `MAX_GOAL_ENTRIES` (expose
   a `#[cfg(test)] fn len(&self)` or assert via successful/missed `get`s).

7. **`goal_store_is_send_sync`** — `fn assert_send_sync<T: Send + Sync>(){}`;
   `assert_send_sync::<SubagentGoalStore>()` — Phase 2 shares it via `Arc`.

In `subagent/flavors.rs` `#[cfg(test)] mod tests`:

8. **`builtin_table_has_general_and_researcher`** — `BUILTIN_SUBAGENT_FLAVORS`
   has exactly 2 entries; `lookup_flavor(General)` and `lookup_flavor(Researcher)`
   both `Some`.

9. **`every_flavor_direction_resolves`** — for each flavor in the table,
   `direction_prompt(flavor.direction)` returns a non-empty `&str` (proves the
   `.md` files exist and are wired — a missing file is a compile error, but this
   also catches an empty file).

10. **`v1_flavors_disallow_nesting`** — assert `allow_nesting == false` for every
    v1 flavor (regression guard for the §8.3 hard gate baseline).

11. **`flavor_tool_allowlists_exclude_spawn_subagent`** — assert no flavor's
    `tool_allowlist` contains `"spawn_subagent"` (defense-in-depth check
    complementing `allow_nesting`).

In `directions/mod.rs` `#[cfg(test)] mod tests`:

12. **`direction_prompts_are_non_empty`** — `direction_prompt(General)` and
    `direction_prompt(Researcher)` are both non-empty after `trim()`.

13. **`direction_id_as_str_is_stable`** — `General.as_str() == "general"`,
    `Researcher.as_str() == "researcher"` (these strings are an identity
    contract Phase 2 may key on).

---

## 4. Ordering, parallelism & risks

### 4.1 Build-dependency reality

Although the three workstreams are *logically* parallel, there is one
compile-time edge introduced by §2.5:

- **P1.B's executor arm additions** (`blocked_kind`, `loop_gate_kind`) reference
  `LoopBlockedKind::AwaitDependentRun` and `LoopGateKind::AwaitDependentRun`,
  which are **P1.A's** new variants.

Two ways to keep all three PRs independently green:

- **Option A (recommended): land P1.A first.** P1.A touches only `ironclaw_turns`
  and is fully self-contained. Once merged, P1.B and P1.C rebase onto it and are
  trivially green. P1.C has *zero* dependency on P1.A or P1.B and can land in any
  order. Net order: **P1.A → (P1.B ∥ P1.C)**.
- **Option B: P1.B pins P1.A's branch** as a temporary path/git dependency for
  CI, dropped on merge. More fragile; only use if calendar pressure forbids
  serialising P1.A before P1.B.

P1.C is genuinely independent of both and should be reviewed/merged in parallel
with whichever of A/B is chosen.

### 4.2 The "variant added, downstream crate breaks" hazard

- Adding `CapabilityOutcome::{SpawnedChildRun, AwaitDependentRun}` (P1.A) makes
  `ironclaw_agent_loop::executor::handle_capability_outcome` non-exhaustive.
  This is *intended* (we deliberately keep `CapabilityOutcome` exhaustive —
  §1.6), but a workspace-green branch must include the two executor arms from
  §1.2 / §2.5 in the same stack. **Do not** add a catch-all `_` arm or a stub
  `unreachable!`; that would erase the fail-loud contract for future outcomes.
- Adding `GateKind::AwaitDependentRun` (P1.B) — `GateKind` is already
  `#[non_exhaustive]`, but the *in-crate* `blocked_kind` / `loop_gate_kind`
  matches are exhaustive and **must** be updated in the same P1.B PR (§2.5).
  That keeps `ironclaw_agent_loop` self-consistently green.

### 4.3 Wire-stability risks

- `TurnStatus` / `BlockedReason` keep PascalCase (no `rename_all`). Adding a
  PascalCase variant is purely additive to the wire format — old persisted
  `TurnRunRecord`s never contained the new value, and the legacy-JSON test (§1.7
  test 5) proves old values still decode. **Do not** retrofit `rename_all` onto
  these enums — that would be a breaking wire change for every persisted record.
- `TurnRunRecord`'s two new fields are `#[serde(default)]` → every pre-existing
  persisted record (libSQL/Postgres JSON payload, or an in-memory snapshot)
  deserializes cleanly. Test §1.7-7 is the regression guard.
- `SUBAGENT_FAMILY_DIGEST` must be the real BLAKE3 of the fingerprint and must
  differ from `DEFAULT_FAMILY_DIGEST` — replay/resume disambiguation depends on
  it. Tests §2.6-2 and §2.6-3 enforce this.

### 4.4 Risks specific to P1.C

- `directions/*.md` are authored prose — a reviewer should sanity-check they
  contain **no `{{placeholder}}`-style templating** and read as a static system
  prompt. The goal injection is a *separate user message* (Phase 2); a templated
  direction file would reopen the prompt-injection hole the design closes.
- The goal-store cap (`MAX_GOAL_ENTRIES`) and payload cap (`MAX_GOAL_BYTES`) are
  proposed numbers — confirm with the design owner. Too small a cap silently
  evicts a live in-flight subagent's goal, which P2.B then turns into a loud
  child-run failure (acceptable fail-loud behavior, but undesirable under normal
  load — hence the `debug!` log on eviction as an early-warning signal).
- `thiserror` must be added to `ironclaw_reborn/Cargo.toml` (§3.5) or
  `SubagentGoalStoreError` will not compile.

### 4.5 Phase 1 exit criteria

Phase 1 is done when, per workstream:

- **P1.A** — `ironclaw_turns` compiles; `cargo test -p ironclaw_turns` green
  (incl. `--features integration` for the libSQL/Postgres `children_of` and the
  `TurnRunRecord` round-trip); all §1.9 tests present and passing.
- **P1.B** — `ironclaw_agent_loop` compiles **against the Phase-1 `ironclaw_turns`**
  and `cargo test -p ironclaw_agent_loop` green; `SUBAGENT_FAMILY_DIGEST`
  filled in with the real hash; all §2.6 tests passing.
- **P1.C** — `ironclaw_reborn` compiles; `cargo test -p ironclaw_reborn` green;
  all §3.6 tests passing; `thiserror` added to `Cargo.toml`.
- Workspace-wide `cargo fmt` clean and `cargo clippy --all --benches --tests
  --examples --all-features` zero-warnings **per crate touched** (the
  workspace-wide clippy should be green once the P1.B executor arms from §2.5
  are present — see §4.2).

No new variant is *exercised* end-to-end in Phase 1 — Phase 1 only lands the
types, the family, and the data. The first end-to-end spawn is Phase 3.

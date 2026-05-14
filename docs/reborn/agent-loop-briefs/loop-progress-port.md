# WS-12 — `LoopProgressPort` Wiring

**Workstream:** WS-12 (follow-up; not in the skeleton WS-0..WS-8 set)
**Crates touched:** `ironclaw_turns` (additive enum variants) +
`ironclaw_loop_support` (adapter) + `ironclaw_reborn` (composition)
**Depends on:** WS-7 (`PlannedDriver` adapter), WS-8 (skeleton green)
**Parallel with:** WS-9, WS-10, WS-11, WS-13, WS-15
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §11–§12

---

## 1. Scope

`LoopProgressPort` ([`crates/ironclaw_turns/src/run_profile/host.rs:1151`](../../../crates/ironclaw_turns/src/run_profile/host.rs))
exists with one method, `emit_loop_progress(LoopProgressEvent)`. The
event enum ships exactly one variant today —
`LoopProgressEvent::DriverNote { kind, safe_summary }` at
[`host.rs:1117`](../../../crates/ironclaw_turns/src/run_profile/host.rs)
with kinds `Planning | Waiting | Retrying`. The skeleton composes a
stub that no-ops the emit call; the executor (master doc §8) reserves
emission points but there is no observer-visible behavior.

WS-12 lands two things together:

1. **Richer milestone surface** — additive `LoopProgressEvent`
   variants for the boundaries the canonical tick (§8) actually
   crosses: `IterationStarted`, `PromptBundleBuilt`,
   `CapabilityBatchStarted`, `CapabilityBatchCompleted`, `GateBlocked`,
   `CheckpointWritten`. Strictly additive; the existing `DriverNote`
   variant stays.
2. **Route through the existing milestone substrate** — extend the
   match in `HostManagedLoopProgressPort::emit_loop_progress`
   ([`crates/ironclaw_reborn/src/loop_driver_host.rs:1599`](../../../crates/ironclaw_reborn/src/loop_driver_host.rs))
   to handle each new variant; add matching emitter methods on
   `LoopHostMilestoneEmitter`
   ([`crates/ironclaw_turns/src/run_profile/milestones.rs:161`](../../../crates/ironclaw_turns/src/run_profile/milestones.rs))
   and matching `LoopHostMilestoneKind` variants.

**Why this composition, not a new adapter:** the milestone-sink
chain (`LoopProgressPort` → `LoopHostMilestoneEmitter` →
`LoopHostMilestoneSink`) is the canonical fanout point per the
existing skeleton (the sink ultimately reaches SSE / audit
observers). Introducing a parallel `RuntimeEventLoopProgressPort`
that goes directly to `EventSink` would create a second source of
truth — the same anti-pattern `.claude/rules/gateway-events.md`
exists to prevent. WS-12 extends the canonical chain instead.

End-state composition:

```text
PlannedDriver
  AgentLoopDriverHost
    LoopProgressPort  →  HostManagedLoopProgressPort  (existing; expanded match)
                          └─ LoopHostMilestoneEmitter  (existing; new methods)
                              └─ LoopHostMilestoneSink  (fans out to engine event substrate)
```

Crate ownership (per master doc §12 follow-up rule):

- **Trait surface additions** — `ironclaw_turns`. Additive enum
  variants on `LoopProgressEvent` and `LoopHostMilestoneKind`; new
  methods on `LoopHostMilestoneEmitter`.
- **Match expansion** — `ironclaw_reborn` (`loop_driver_host.rs` —
  the production `HostManagedLoopProgressPort`).

## 2. Files

### NEW
_None._ (Originally drafted a `RuntimeEventLoopProgressPort` in
`ironclaw_loop_support`; dropped because it would create a parallel
fanout path. The milestone-sink substrate is the canonical chain.)

### MODIFIED
- `crates/ironclaw_turns/src/run_profile/host.rs` —
  `LoopProgressEvent` gains six additive variants (§3.2). `kind_name`
  match expands. No existing variant changes shape.
- `crates/ironclaw_turns/src/run_profile/milestones.rs` —
  `LoopHostMilestoneEmitter` ([line 161](../../../crates/ironclaw_turns/src/run_profile/milestones.rs))
  gains async methods `iteration_started`, `prompt_iteration_built`,
  `capability_batch_started`, `capability_batch_completed`,
  `gate_blocked`, `checkpoint_milestone_written` (the existing
  `prompt_bundle_built` and `checkpoint_created` already cover two
  of the six boundaries — see §3.4 for the mapping). `LoopHostMilestoneKind`
  gains matching variants where needed.
- `crates/ironclaw_reborn/src/loop_driver_host.rs` —
  `HostManagedLoopProgressPort::emit_loop_progress` match (line 1601)
  expands from the current single `DriverNote` arm to cover the six
  new variants. Each arm routes to the matching emitter method.
- `crates/ironclaw_reborn/src/milestone_events.rs` —
  `DurableLoopHostMilestoneSink::runtime_event_for_milestone` (line
  167) matches `LoopHostMilestoneKind` exhaustively. Adding new
  variants forces matching arms here even if the projection is
  intentionally a no-op for the new milestones: each new variant
  gets an explicit `Ok(None)` arm (drop-on-floor at the durable-event
  projection layer) or a `RuntimeEvent` projection arm — the brief
  author picks per variant at PR time, but every new variant MUST
  appear in this match.
- `crates/ironclaw_agent_loop/src/canonical_executor.rs` (WS-6 file) —
  emission points described in §3.4 fire the new variants. The
  existing `DriverNote` emissions (Planning at top of loop, Waiting
  on gate block, Retrying on recovery) stay where they are.

### NOT TOUCHED
- `crates/ironclaw_turns/src/run_profile/host.rs` `AgentLoopDriverHost`
  composite trait — `LoopProgressPort` is already a supertrait. No
  trait surface change.
- `crates/ironclaw_events/**` — the milestone-sink substrate is the
  fanout point. Adding a new `RuntimeEventKind` is **not** part of
  WS-12; the existing event substrate already conveys
  `LoopHostMilestoneKind` through its sinks via the substrate the
  host runtime owns. If a future PR wants typed `RuntimeEventKind`
  for loop milestones, that lands separately on the sink side.
- SSE / web transport — sinks already subscribe to whatever the
  milestone substrate fans out. Once the new emitter methods fire,
  the transport delivers them without further wiring (subject to
  whatever projection layer maps `LoopHostMilestoneKind` into the
  gateway's `AppEvent`).

## 3. Specification

### 3.1 Why additive, not breaking

`LoopProgressEvent` is wire-stable per `.claude/rules/types.md`
("Wire-stable enums"). Adding variants is additive; renaming or
removing is breaking. Older consumers running an older event schema
will see `kind_name() = "iteration_started"` etc. via the existing
fallback arm pattern (`Self::DriverNote { .. } => "driver_note"`) —
they pass the event through unmodified. The brief explicitly does
**not** remove `DriverNote`.

### 3.2 New `LoopProgressEvent` variants

```rust
//! crates/ironclaw_turns/src/run_profile/host.rs (delta at line 1117)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopProgressEvent {
    DriverNote {
        kind: LoopDriverNoteKind,
        safe_summary: LoopSafeSummary,
    },

    /// Fires at the top of every iteration, after the iteration cap
    /// check (§8 step 0) and before cancellation observation.
    IterationStarted {
        iteration: u32,
    },

    /// Fires after `host.build_prompt_bundle(...)` returns and before
    /// `BeforeModel` checkpoint.
    PromptBundleBuilt {
        iteration: u32,
        message_count: u32,
        identity_message_count: u32,
        instruction_snippet_count: u32,
        /// `Option` matches `LoopPromptBundle.surface_version` and the
        /// existing `LoopHostMilestoneKind::PromptBundleBuilt` field
        /// shape ([`crates/ironclaw_turns/src/run_profile/milestones.rs:181`](../../../crates/ironclaw_turns/src/run_profile/milestones.rs)).
        /// Profiles that never pin a capability surface (text-only,
        /// pre-WS-9) leave it `None`; making it mandatory would
        /// force callers to fabricate a bogus version.
        surface_version: Option<CapabilitySurfaceVersion>,
    },

    /// Fires after `BeforeSideEffect` checkpoint, before
    /// `invoke_capability_batch`.
    CapabilityBatchStarted {
        iteration: u32,
        call_count: u32,
        policy: BatchPolicyKind,
    },

    /// Fires after the batch invocation returns (any outcomes — Completed,
    /// Denied, ApprovalRequired, etc.) and before per-iteration stop
    /// check.
    CapabilityBatchCompleted {
        iteration: u32,
        result_count: u32,
        denied_count: u32,
        gated_count: u32,
        failed_count: u32,
    },

    /// Fires on a planner-Block gate outcome (§8 ApprovalRequired /
    /// AuthRequired / ResourceBlocked branch) right before
    /// `BeforeBlock` checkpoint.
    GateBlocked {
        iteration: u32,
        gate_kind: LoopGateKind,
    },

    /// Fires immediately after every successful `host.checkpoint(...)`
    /// returns. Mirrors the four checkpoint kinds so observers see one
    /// per kind per iteration (BeforeModel, BeforeSideEffect,
    /// BeforeBlock, Final).
    CheckpointWritten {
        iteration: u32,
        kind: LoopCheckpointKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchPolicyKind { Sequential, Parallel }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopGateKind { Approval, Auth, ResourceWait }
```

Notes:

- All counters are `u32`; per-iteration values fit comfortably.
- `CapabilitySurfaceVersion` already exists at
  [`host.rs:710`](../../../crates/ironclaw_turns/src/run_profile/host.rs).
- `BatchPolicyKind` is a wire-stable shadow of the planner-side
  `BatchPolicy` (which carries policy-specific fields the wire does
  not need); the adapter compresses to the kind.
- Per `error-handling.md`'s channel-edge rule, *no raw prompt
  content, capability args, or model errors* land in these events.
  Snippet counts and result counts only — the actual content stays
  below the host's redaction surface.

`kind_name(&self)` ([`host.rs:1135`](../../../crates/ironclaw_turns/src/run_profile/host.rs))
gains arms for each new variant returning the snake-cased name.

### 3.3 New `LoopHostMilestoneEmitter` methods

Each new `LoopProgressEvent` variant routes to a corresponding
emitter method. Two of the six already exist on the emitter
(`prompt_bundle_built` and `checkpoint_created` at
[`milestones.rs:177,231`](../../../crates/ironclaw_turns/src/run_profile/milestones.rs))
— the `HostManagedLoopProgressPort` match calls them with the same
arguments. Four new methods land in this brief:

```rust
//! crates/ironclaw_turns/src/run_profile/milestones.rs (delta)

impl<S> LoopHostMilestoneEmitter<S>
where
    S: LoopHostMilestoneSink + ?Sized,
{
    // ... existing methods ...

    pub async fn iteration_started(
        &self,
        iteration: u32,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::IterationStarted { iteration }).await
    }

    pub async fn capability_batch_started(
        &self,
        iteration: u32,
        call_count: u32,
        policy: BatchPolicyKind,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::CapabilityBatchStarted {
            iteration, call_count, policy,
        }).await
    }

    pub async fn capability_batch_completed(
        &self,
        iteration: u32,
        result_count: u32,
        denied_count: u32,
        gated_count: u32,
        failed_count: u32,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::CapabilityBatchCompleted {
            iteration, result_count, denied_count, gated_count, failed_count,
        }).await
    }

    pub async fn gate_blocked(
        &self,
        iteration: u32,
        gate_kind: LoopGateKind,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::GateBlocked { iteration, gate_kind }).await
    }
}
```

`LoopHostMilestoneKind` gains matching additive variants. Existing
variants (`PromptBundleBuilt`, `ModelStarted`, `ModelCompleted`,
`ModelFailed`, `CapabilityInvoked`, `CheckpointCreated`,
`AssistantReplyFinalized`, `Blocked`, `Completed`, `Failed`,
`DriverNote`) stay as they are.

### 3.4 Executor emission points (master doc §8)

Each emission is annotated MUST or MAY. MUST emissions are part of
the canonical-tick contract; MAY emissions are reserved for richer
planners.

| Step in §8 | Event | MUST/MAY |
|---|---|---|
| Step 0 (top of loop, after cap check) | `IterationStarted { iteration }` | MUST |
| Step 1 (cancellation observation, when fired) | (WS-13 emits `LoopExit::Cancelled`; no progress event) | — |
| Step 2 (steering drain) | `DriverNote { Planning }` | MAY |
| After `host.build_prompt_bundle` returns | `PromptBundleBuilt { … }` | MUST |
| After `BeforeModel` checkpoint | `CheckpointWritten { BeforeModel }` | MUST |
| Model stream complete (reply case) | (transport-side `ModelCompleted` already fires; no loop event) | — |
| Capability calls path, after `BeforeSideEffect` checkpoint | `CheckpointWritten { BeforeSideEffect }` | MUST |
| Before `invoke_capability_batch` | `CapabilityBatchStarted { call_count, policy }` | MUST |
| After batch outcomes received | `CapabilityBatchCompleted { result_count, denied_count, gated_count, failed_count }` | MUST |
| Recovery `Retry { alter }` taken | `DriverNote { Retrying }` | MUST |
| Gate-block branch, before `BeforeBlock` checkpoint | `GateBlocked { gate_kind }` | MUST |
| After `BeforeBlock` checkpoint | `CheckpointWritten { BeforeBlock }` | MUST |
| After `Final` checkpoint (exit paths) | `CheckpointWritten { Final }` | MUST |

The brief's verification suite asserts the MUST emissions appear in
the expected order against an in-memory sink (§5).

### 3.5 Expanded match in `HostManagedLoopProgressPort`

```rust
//! crates/ironclaw_reborn/src/loop_driver_host.rs (delta at line 1599)

#[async_trait]
impl LoopProgressPort for HostManagedLoopProgressPort {
    async fn emit_loop_progress(
        &self,
        event: LoopProgressEvent,
    ) -> Result<(), AgentLoopHostError> {
        let emitter = LoopHostMilestoneEmitter::new(
            self.run_context.clone(),
            Arc::clone(&self.milestone_sink),
        );
        match event {
            // EXISTING:
            LoopProgressEvent::DriverNote { kind, safe_summary } =>
                emitter.driver_note(kind, safe_summary).await,

            // NEW (WS-12):
            LoopProgressEvent::IterationStarted { iteration } =>
                emitter.iteration_started(iteration).await,
            LoopProgressEvent::PromptBundleBuilt {
                iteration, bundle_ref, mode, surface_version,
                message_count, identity_message_count, instruction_snippet_count,
            } => {
                // Compose `skill_context` from snippet/identity counts;
                // the existing emitter method already accepts the
                // bundle-shape metadata. Drop the iteration field here
                // (already implied by milestone ordering) — or expose
                // it on a thin successor variant if observers need it.
                emitter.prompt_bundle_built(
                    bundle_ref, mode, surface_version,
                    message_count as usize,
                    Vec::new(),  // skill_context: filled at PR time
                ).await
            }
            LoopProgressEvent::CapabilityBatchStarted {
                iteration, call_count, policy,
            } => emitter.capability_batch_started(iteration, call_count, policy).await,
            LoopProgressEvent::CapabilityBatchCompleted {
                iteration, result_count, denied_count, gated_count, failed_count,
            } => emitter
                .capability_batch_completed(
                    iteration, result_count, denied_count, gated_count, failed_count,
                ).await,
            LoopProgressEvent::GateBlocked { iteration, gate_kind } =>
                emitter.gate_blocked(iteration, gate_kind).await,
            LoopProgressEvent::CheckpointWritten { iteration: _, kind } => {
                // CheckpointCreated milestone takes a checkpoint_id; the
                // BeforeBlock/Final flows already emit it from inside
                // HostManagedLoopCheckpointPort::checkpoint at line 1567.
                // The new `CheckpointWritten` LoopProgressEvent variant
                // is therefore advisory only at this layer — the canonical
                // emission for `CheckpointKind` is already wired. Treat
                // this arm as `Ok(())` to avoid double-emission, or
                // promote it to a dedicated emitter method if observers
                // need the iteration counter alongside the kind.
                Ok(())
            }
        }
    }
}
```

Three properties the impl must respect:

1. **Best-effort** — failures from `LoopHostMilestoneSink` already
   return as `AgentLoopHostError` per the existing
   `emitter.publish(...)` contract; per
   [`crates/ironclaw_events/src/sink.rs:14-26`](../../../crates/ironclaw_events/src/sink.rs)
   the sink contract is best-effort. `HostManagedLoopProgressPort`
   already propagates the result of these calls; that does not
   change in WS-12.
2. **No raw content** — payloads are counts + ids + version refs +
   `LoopSafeSummary` only. The executor never hands the adapter a
   prompt body, args, or error message.
3. **Stable cardinality per iteration** — exactly one emission per
   `LoopProgressEvent` (with the `CheckpointWritten` exception in §3.5
   above to prevent double-emission).

### 3.6 No driver-config change

`HostManagedLoopProgressPort` is composed inside `RebornLoopDriverHost`
at [`loop_driver_host.rs:1246`](../../../crates/ironclaw_reborn/src/loop_driver_host.rs).
No new field on `PlannedDriverConfig`; the milestone sink is already
wired through.

## 4. Composition

Already wired. `HostManagedLoopProgressPort` is composed inside
`RebornLoopDriverHost` at
[`loop_driver_host.rs:1246`](../../../crates/ironclaw_reborn/src/loop_driver_host.rs);
the milestone sink is passed in at host-build time. WS-12 is a pure
match-expansion + emitter-method addition on top of that wiring.

## 5. Verification

Unit tests (in `crates/ironclaw_turns` — alongside existing
`LoopHostMilestoneEmitter` tests):

- `milestones::tests::iteration_started_publishes_kind` — emitter
  with an `InMemoryLoopHostMilestoneSink`; call `iteration_started(3)`;
  assert the captured `LoopHostMilestoneKind::IterationStarted { iteration: 3 }`.
- `milestones::tests::capability_batch_started_publishes_counts` —
  assert call_count + policy round-trip into the captured milestone.
- `milestones::tests::capability_batch_completed_publishes_counters` —
  result/denied/gated/failed all reach the sink.
- `milestones::tests::gate_blocked_publishes_kind` — gate_kind round
  trip.

Unit tests (in `crates/ironclaw_reborn` — alongside existing
`HostManagedLoopProgressPort` tests):

- `loop_driver_host::tests::progress_port_routes_iteration_started` —
  emit `LoopProgressEvent::IterationStarted { iteration: 2 }`; assert
  milestone-sink captures `IterationStarted { iteration: 2 }`.
- `loop_driver_host::tests::progress_port_routes_capability_batch_started_and_completed`
  — assert both events route correctly.
- `loop_driver_host::tests::progress_port_routes_gate_blocked` —
  blocking arm fires.
- `loop_driver_host::tests::progress_port_checkpoint_written_no_double_emit`
  — assert `CheckpointWritten` does not double-fire next to the
  existing `CheckpointCreated` milestone (§3.5 contract).
- `loop_driver_host::tests::progress_port_driver_note_unchanged` —
  regression: existing `DriverNote` arm still works after the match
  expansion.
- `loop_driver_host::tests::progress_event_serde_roundtrip_all_variants`
  — round-trip each of the seven `LoopProgressEvent` variants through
  serde; locks the wire-stable contract.

Integration tests (in `crates/ironclaw_reborn`, gated behind
`ironclaw_agent_loop/test-support` from WS-8):

- `planned_driver_emits_iteration_milestones_in_order` — drive one
  iteration that takes the AssistantReply branch; observe in-memory
  sink; assert the ordered sequence:
  `IterationStarted, PromptBundleBuilt, CheckpointWritten(BeforeModel),
  AssistantReplyFinalized, CheckpointWritten(Final), LoopCompleted`.
- `planned_driver_emits_capability_batch_milestones` — drive an
  iteration that takes the CapabilityCalls branch with one batch of
  three calls (two completed + one denied); assert:
  `IterationStarted, PromptBundleBuilt, CheckpointWritten(BeforeModel),
  CheckpointWritten(BeforeSideEffect), CapabilityBatchStarted(call_count=3),
  CapabilityBatchCompleted(result_count=2, denied_count=1, gated_count=0, failed_count=0),
  CheckpointWritten(Final), LoopCompleted`.
- `planned_driver_emits_gate_blocked_on_approval_required` — drive a
  call that returns `ApprovalRequired`; assert
  `GateBlocked(gate_kind=Approval), CheckpointWritten(BeforeBlock),
  LoopBlocked` sequence.

## 6. Out of scope (for this brief)

- **SSE transport.** The gateway already projects `RuntimeEvent` to
  the SSE/WS stream per `.claude/rules/gateway-events.md`. Sink-side
  concern; no work here.
- **Audit envelope emission.** The `AuditEnvelope` substrate
  ([`crates/ironclaw_events/src/sink.rs:39`](../../../crates/ironclaw_events/src/sink.rs))
  is control-plane only. Loop progress is runtime-plane.
- **Metric exporters** (Prometheus, OpenTelemetry). Sink-side; add
  a sink implementation in the metrics PR. Not part of WS-12.
- **Retention / sampling / tiered storage.** Sink-side concern.
- **Loop-family-specific milestones.** Beyond the six this brief
  adds, richer planners can add additional variants in their own
  follow-up brief — the enum is intentionally open.
- **Backpressure on the sink.** Best-effort emit assumes the sink
  has its own buffering / drop policy. If the sink blocks, the loop
  blocks; sinks SHOULD have a bounded-wait policy.

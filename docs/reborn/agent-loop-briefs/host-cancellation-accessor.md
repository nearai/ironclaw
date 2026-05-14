# WS-13 — Cancellation Accessor on `AgentLoopDriverHost`

**Workstream:** WS-13 (follow-up; not in the skeleton WS-0..WS-8 set)
**Crates touched:** `ironclaw_turns` (trait extension) +
`ironclaw_loop_support` (adapter) + `ironclaw_reborn` (composition)
**Depends on:** WS-7 (`PlannedDriver` adapter), WS-8 (skeleton green).
This brief also locks the placeholder WS-6 left at
[`agent-loop-skeleton.md` §8 step 1](../agent-loop-skeleton.md).
**Parallel with:** WS-9, WS-10, WS-11, WS-12, WS-15
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §8, §11–§12

---

## 1. Scope

Master doc §8 step 1 says:

> Cancellation observation — checkpoint + `Ok(LoopExit::Cancelled(...))`
> if fired.

Master doc §11 acknowledges the gap:

> Not a cancellation accessor on the host (WS-13). The executor's
> cancellation-observation point in WS-6 §3.5 is documented but the host
> method it calls doesn't exist yet.

Today's cancellation flow is two-phase
([`crates/ironclaw_turns/CLAUDE.md`](../../../crates/ironclaw_turns/CLAUDE.md)):
public cancel requests move `TurnRunState` to `CancelRequested`
([`crates/ironclaw_turns/src/events.rs:23`](../../../crates/ironclaw_turns/src/events.rs))
and a trusted runner completion via
`TurnRunTransitionPort::cancel_run`
([`crates/ironclaw_turns/src/runner.rs:137`](../../../crates/ironclaw_turns/src/runner.rs))
finalizes the terminal state. Drivers have **no live signal** — only
the per-worker `tokio_util::sync::CancellationToken` exists, at
[`crates/ironclaw_reborn/src/turn_runner.rs:23`](../../../crates/ironclaw_reborn/src/turn_runner.rs),
which is process-level, not per-run.

WS-13 adds the missing per-run accessor:

1. **Trait extension** — new tiny port `LoopCancellationPort` in
   `ironclaw_turns`, added to the `AgentLoopDriverHost` supertrait
   list at [`host.rs:1155`](../../../crates/ironclaw_turns/src/run_profile/host.rs).
2. **Adapter** — `RunStateLoopCancellationPort` in
   `ironclaw_loop_support` backed by a cheap snapshot handle the host
   runtime flips when it observes `CancelRequested`.
3. **Executor wiring (no new code in WS-13, just a pointer)** —
   `CanonicalAgentLoopExecutor` already has the
   `checkpoint_and_exit_if_cancelled()` helper
   from WS-6. This brief documents the exact call it MUST make and
   the exit semantics.

End-state: the executor honors mid-loop cancellation between every
strategy call (master doc §8 property 2 "Cancellation observation —
checked between every strategy call"), returns
`Ok(LoopExit::Cancelled { reason_kind })`, and writes a `Final`
checkpoint before exit so the run lifecycle stays well-formed.

Crate ownership (per master doc §12 follow-up rule):

- **Trait extension** — `ironclaw_turns`. Adds one port; tightens
  the composite supertrait.
- **Adapter** — `ironclaw_loop_support`.
- **Composition** — `ironclaw_reborn` (driver config field + host
  build hook).

## 2. Files

### NEW
- `crates/ironclaw_loop_support/src/cancellation_port.rs` —
  `RunStateLoopCancellationPort`, `RunCancellationHandle` (snapshot
  handle the host runtime flips).

### MODIFIED
- `crates/ironclaw_turns/src/run_profile/host.rs`:
  - New trait `LoopCancellationPort` (§3.1).
  - New value types `LoopCancellationSignal`.
  - `AgentLoopDriverHost` supertrait list at line 1155 gains
    `LoopCancellationPort`.
  - Blanket `impl AgentLoopDriverHost for T` at line 1170 mirrors
    the supertrait list.
- `crates/ironclaw_agent_loop/src/canonical_executor.rs` (WS-6 file)
  — the `checkpoint_and_exit_if_cancelled` helper
  inlines the new port call (§3.3). Pure rewiring of an existing
  placeholder; no logic shift.
- `crates/ironclaw_loop_support/src/lib.rs` — module declaration +
  re-export.
- `crates/ironclaw_reborn/src/planned_driver.rs` (WS-7 file) —
  `PlannedDriverConfig` gains a `RunCancellationFactory`; build hook
  composes the port per run.

### NOT TOUCHED
- `crates/ironclaw_turns/src/runner.rs` — the two-phase cancellation
  surface (`TurnRunState::CancelRequested` + `cancel_run`) is the
  upstream signal. This brief does not change it.
- `crates/ironclaw_reborn/src/turn_runner.rs` — the worker-level
  `CancellationToken` stays; the adapter can optionally tap it.

## 3. Specification

### 3.1 `LoopCancellationPort` trait

```rust
//! crates/ironclaw_turns/src/run_profile/host.rs (new section, near line 1155)

use chrono::{DateTime, Utc};

/// Per-run cancellation observation point. The executor consults
/// this between every strategy call (master doc §8 property 2). The
/// method is intentionally **synchronous and non-blocking** — it is
/// called O(strategies-per-iteration) times and must not introduce
/// an `.await` between strategy calls. Implementations expose a
/// cheap snapshot (typically `Arc<AtomicBool>`).
///
/// Cancellation is a **successful exit**, not an executor error.
/// On a positive observation the executor returns
/// `Ok(LoopExit::Cancelled { reason_kind })` after writing a final
/// checkpoint. `AgentLoopExecutorError::Cancelled` is reserved for
/// the unrecoverable edge case where the executor cannot even
/// produce a `LoopExit::Cancelled`.
pub trait LoopCancellationPort: Send + Sync {
    /// Returns `Some(signal)` if cancellation has been requested
    /// for this run; `None` otherwise.
    ///
    /// Implementations MUST be idempotent across reads — repeated
    /// calls return the same `Some(signal)` once the request fires.
    /// The executor calls this many times per iteration; this
    /// contract lets `recent_call_signatures` etc. continue
    /// observing state between the cancellation flip and the
    /// executor's next opportunity to consult the port.
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopCancellationSignal {
    pub reason_kind: LoopCancelReasonKind,
    pub requested_at: DateTime<Utc>,
}
```

Two design points:

- **Sync, not async.** The observation point fires between every
  strategy call (§8 property 2). Making it async would invite the
  strategy boundary to absorb deadlocks; the existing two-phase
  cancellation upstream is the only place that needs async (which
  already happens, outside the loop).
- **Reused `LoopCancelReasonKind`.** The same enum
  ([`host.rs:691`](../../../crates/ironclaw_turns/src/run_profile/host.rs))
  that `LoopInput::Cancel` already uses (`UserRequested |
  Superseded | Policy`). One source of truth; no parallel taxonomy.
- **Idempotent reads.** The executor's `recent_call_signatures` and
  `recent_failure_kinds` rings (master doc §10) are maintained
  *between* cancellation observations within a single batch. If the
  port flipped on first read and returned `None` on later reads, a
  long batch could observe cancellation, then drift back into "alive"
  state. Idempotent reads close that hole at no cost — an
  `AtomicBool` load is cheap.

### 3.2 `AgentLoopDriverHost` composition

```rust
//! crates/ironclaw_turns/src/run_profile/host.rs (delta at line 1155)

pub trait AgentLoopDriverHost:
    LoopRunInfoPort
    + LoopContextPort
    + LoopPromptPort
    + LoopInputPort
    + LoopModelPort
    + LoopCapabilityPort
    + LoopTranscriptPort
    + LoopCheckpointPort
    + LoopProgressPort
    + LoopCancellationPort           // NEW
    + Send
    + Sync
{
}

impl<T> AgentLoopDriverHost for T where
    T: LoopRunInfoPort
        + LoopContextPort
        + LoopPromptPort
        + LoopInputPort
        + LoopModelPort
        + LoopCapabilityPort
        + LoopTranscriptPort
        + LoopCheckpointPort
        + LoopProgressPort
        + LoopCancellationPort       // NEW
        + Send
        + Sync
{
}
```

Test fixtures and mock hosts that today derive `AgentLoopDriverHost`
via the blanket impl must add a `LoopCancellationPort` impl —
typically a one-liner returning `None` (the "never cancelled" stub).
The brief includes a small `AlwaysAliveLoopCancellationPort`
convenience in `ironclaw_loop_support` for this exact purpose so
test code does not need to spell out the impl.

### 3.3 Executor observation point (WS-6 placeholder → real call)

```rust
//! crates/ironclaw_agent_loop/src/canonical_executor.rs (delta)

async fn checkpoint_and_exit_if_cancelled(
    state: &mut LoopExecutionState,
    host: &(dyn AgentLoopDriverHost + Send + Sync),
) -> Option<Result<LoopExit, AgentLoopExecutorError>> {
    let Some(signal) = host.observe_cancellation() else {
        return None;
    };
    // Write Final checkpoint before exit so the run is well-formed.
    // The validation policy decides what happens on failure:
    // `validate_cancelled_exit` at
    // `crates/ironclaw_turns/src/loop_exit.rs:803` rejects a
    // `LoopExit::Cancelled` without a verified final checkpoint when
    // `checkpoint_policy.require_final_checkpoint` is true (which the
    // builtin `long_running_mission` profile sets at
    // `crates/ironclaw_turns/src/run_profile/resolver.rs:293`). Silently
    // returning Cancelled in that case would surface as a
    // `MissingFinalCheckpoint` violation downstream.
    let checkpoint_id = match host
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::Final,
            state_ref: state.encode_checkpoint_payload(),
        })
        .await
    {
        Ok(id) => {
            state.last_checkpoint = Some(CheckpointMarker {
                kind: LoopCheckpointKind::Final,
                iteration_at_checkpoint: state.iteration,
                checkpoint_id: Some(id),
            });
            Some(id)
        }
        Err(_) if !host.run_context().checkpoint_policy.require_final_checkpoint => {
            // Permissive profile: cancellation still wins, the
            // missing checkpoint is recorded via LoopProgressPort
            // (WS-12). Best-effort.
            None
        }
        Err(host_err) => {
            // Strict profile: a cancel without a final checkpoint
            // would be invalidated by `validate_cancelled_exit`.
            // Escalate to a Failed exit so the runner sees a
            // well-formed exit instead of a contract violation. The
            // user-facing intent (cancellation) is captured in the
            // failure reason summary.
            return Some(Ok(LoopExit::failed(
                LoopFailureKind::CheckpointRejected,
                LoopExitId::new(),
            ).into()));
        }
    };

    Some(Ok(LoopExit::cancelled(signal.reason_kind, checkpoint_id, LoopExitId::new()).into()))
}
```

The function is called after every strategy call in §8: at the top
of the loop, between drain → prompt → checkpoint → model → reply,
inside the capability-batch recovery loop, between gate handling
and stop-condition consultation. WS-6's existing test suite (under
`ironclaw_agent_loop/test-support`) exercises the placeholder; this
brief's verification (§5) expands the suite to cover real
cancellation observation.

### 3.4 Adapter `RunStateLoopCancellationPort`

```rust
//! crates/ironclaw_loop_support/src/cancellation_port.rs

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use chrono::Utc;
use parking_lot::RwLock;
use ironclaw_turns::run_profile::{
    LoopCancellationPort, LoopCancellationSignal, LoopCancelReasonKind,
};

/// Snapshot handle the host runtime owns and flips on cancellation.
/// One handle per run; passed into `RunStateLoopCancellationPort` and
/// also kept by the host runtime so it can fire the flip when its
/// upstream observes `TurnRunState::CancelRequested` for this run.
#[derive(Clone, Default)]
pub struct RunCancellationHandle {
    fired: Arc<AtomicBool>,
    signal: Arc<RwLock<Option<LoopCancellationSignal>>>,
}

impl RunCancellationHandle {
    pub fn request(&self, reason_kind: LoopCancelReasonKind) {
        let signal = LoopCancellationSignal {
            reason_kind,
            requested_at: Utc::now(),
        };
        // Write signal first so any observer that sees `fired=true`
        // can also read the signal payload. Release ordering pairs
        // with the load's Acquire in observe_cancellation.
        *self.signal.write() = Some(signal);
        self.fired.store(true, Ordering::Release);
    }

    pub fn is_requested(&self) -> bool {
        self.fired.load(Ordering::Acquire)
    }
}

pub struct RunStateLoopCancellationPort {
    handle: RunCancellationHandle,
}

impl RunStateLoopCancellationPort {
    pub fn new(handle: RunCancellationHandle) -> Self {
        Self { handle }
    }
}

impl LoopCancellationPort for RunStateLoopCancellationPort {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        if !self.handle.fired.load(Ordering::Acquire) {
            return None;
        }
        self.handle.signal.read().clone()
    }
}
```

Two notes:

- **`parking_lot::RwLock` over `std::sync::Mutex`** — observation is
  hot (per-strategy-call), and contention is single-writer (host
  fires once). `parking_lot` is already in the workspace.
- **Atomic-then-lock pattern** — the common path (no cancellation)
  is one atomic load with no lock acquisition. The slow path
  (cancellation fired) reads the signal once.

The host runtime's plumbing — wiring `RunCancellationHandle.request`
to the actual `TurnRunState::CancelRequested` transition — is a
host-side wiring PR. The brief defines the seam; the substrate is
out of scope (§6).

### 3.5 Test convenience: `AlwaysAliveLoopCancellationPort`

```rust
//! crates/ironclaw_loop_support/src/cancellation_port.rs

/// Always reports "not cancelled". Used in test hosts that don't
/// care about cancellation. Adding a real handle for every mock is
/// noisy; this stub satisfies the supertrait at zero cost.
pub struct AlwaysAliveLoopCancellationPort;

impl LoopCancellationPort for AlwaysAliveLoopCancellationPort {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> { None }
}
```

WS-8's integration suite (the `test-support` feature) imports this
when building mock hosts. The brief explicitly inventories the
fixtures that need the new impl so the suite goes green on WS-13's
landing without extra plumbing.

## 4. Composition in `PlannedDriverConfig`

```rust
//! crates/ironclaw_reborn/src/planned_driver.rs (delta)

pub struct PlannedDriverConfig {
    // ... fields from WS-7, WS-9..WS-12, WS-15 ...
    pub cancellation_factory: Arc<dyn RunCancellationFactory>,
}

/// Host-runtime callback the driver invokes once per claimed run to
/// produce a `RunCancellationHandle` for that run. The host runtime
/// keeps its end of the handle and flips it on
/// `TurnRunState::CancelRequested`.
#[async_trait]
pub trait RunCancellationFactory: Send + Sync {
    async fn handle_for_run(
        &self,
        run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError>;
}

impl PlannedDriver {
    async fn build_cancellation_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCancellationPort>, AgentLoopHostError> {
        let handle = self
            .config
            .cancellation_factory
            .handle_for_run(run_context.run_id)
            .await?;
        Ok(Arc::new(RunStateLoopCancellationPort::new(handle)))
    }
}
```

`RunCancellationFactory` lives in `ironclaw_loop_support`. The host
runtime's implementation taps the existing two-phase cancellation
observer in `TurnCoordinator` and wires the flip into a per-run record
of `RunCancellationHandle`s. Construction is a three-step race-safe
protocol:

1. Read durable run state before returning the handle. If the run is
   already `CancelRequested`, seed the handle as fired.
2. Register/subscribe the handle with the host-runtime cancellation
   broadcaster for future flips.
3. Re-read durable run state after registration and fire the handle if
   cancellation landed between steps 1 and 2.

A fresh clear handle for an already-cancelled run is invalid: resume
could continue into prompt/model/tool side effects after durable
cancellation. The implementation must close the late-subscriber race
with the recheck above.

## 5. Verification

Unit tests (in `crates/ironclaw_loop_support`):

- `cancellation_port::tests::observe_returns_none_when_not_requested` —
  fresh handle; port returns `None` on first observation.
- `cancellation_port::tests::observe_returns_signal_after_flip` —
  flip with `UserRequested`; port returns `Some(signal)` with
  matching `reason_kind`.
- `cancellation_port::tests::observe_idempotent_after_first_read` —
  flip, read three times, assert all three reads return identical
  `Some(signal)`. Guards §3.1 idempotency contract.
- `cancellation_port::tests::observe_payload_includes_requested_at` —
  assert `requested_at` is set within a 5-second window of the flip.
- `cancellation_port::tests::handle_signal_visible_after_atomic_load` —
  concurrent test: thread A flips, thread B busy-loops on
  `observe_cancellation`. When B sees `Some`, the signal payload
  must be readable. Guards the atomic-then-lock memory ordering.
- `cancellation_port::tests::factory_seeds_prefired_cancel_state` —
  durable run state is already `CancelRequested` before
  `handle_for_run`; returned handle observes cancellation immediately.
- `cancellation_port::tests::factory_rechecks_after_subscription` —
  cancellation flips between initial durable read and broadcaster
  registration; returned handle still observes cancellation.
- `cancellation_port::tests::always_alive_port_returns_none` —
  trivial coverage of the test stub.

Integration tests (in `crates/ironclaw_reborn`, gated behind
`ironclaw_agent_loop/test-support` from WS-8):

- `planned_driver_cancels_between_strategy_calls` — start a run with
  a planner that emits capability calls; flip the handle after the
  `BeforeSideEffect` checkpoint (i.e. before
  `invoke_capability_batch`); assert
  `LoopExit::Cancelled { reason_kind: UserRequested }` and that a
  `Final` checkpoint was written (visible via WS-10's load path).
- `planned_driver_cancels_mid_batch_recovery_loop` — flip during a
  recovery `Retry`; assert the recovery loop exits and yields
  `LoopExit::Cancelled` instead of escalating to `LoopFailureKind`.
- `cancelled_run_writes_final_checkpoint_before_exit` — combined
  WS-10 + WS-13: cancel; reload from `Final` checkpoint; assert
  the loaded state matches the state at the cancellation point.
- `cancellation_observed_before_first_iteration` — flip handle
  before `PlannedDriver::run` begins; assert the executor exits on
  step 1 of iteration 0 without doing any work.

## 6. Out of scope (for this brief)

- **Soft-cancel / drain-then-stop.** v1 ships a single signal kind.
  A "stop after the current iteration" semantic can be a future
  variant on `LoopCancelReasonKind`; not in WS-13.
- **Capability call cancellation mid-invocation.** Once
  `invoke_capability_batch` is in flight, cancelling it requires
  `CapabilityHost` cooperation (process kill, RPC abort, …). That
  is a separate concern handled below the loop. WS-13 cancels
  *between* host calls, not *during* them.
- **Whole-process shutdown.** The existing
  `tokio_util::sync::CancellationToken` at `turn_runner.rs:23` keeps
  its role for worker-shutdown propagation. WS-13's adapter can
  optionally tap it (a worker shutdown flips every run's handle),
  but that wiring is host-side.
- **Cancellation precedence with `LoopInput::Cancel`.** Both paths
  produce `Cancelled` exits; the executor takes whichever fires
  first. No tie-break logic needed in v1.
- **Audit trail of cancellation events.** `LoopExit::Cancelled` is
  already a recorded transition. No new audit hook in WS-13.
- **Re-arming after cancellation.** A handle that fired stays fired
  for the run. New runs get new handles via `RunCancellationFactory`.

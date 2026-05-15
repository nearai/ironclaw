# WS-11 — `LoopInputPort` Implementation

**Workstream:** WS-11 (follow-up; not in the skeleton WS-0..WS-8 set)
**Crates touched:** `ironclaw_loop_support` (adapter) + host-runtime PR
(concrete `HostInputQueue` backing — out of scope here)
**Depends on:** WS-7 (`PlannedDriver` adapter), WS-8 (skeleton green)
**Parallel with:** WS-9, WS-10, WS-12, WS-13, WS-15
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §11–§12

---

## 1. Scope

The skeleton today composes a stub `LoopInputPort` (the unnamed
`EmptyLoopInputPort` equivalent — today routed through the
`unsupported_host_method` default at
[`crates/ironclaw_turns/src/run_profile/host.rs:1189`](../../../crates/ironclaw_turns/src/run_profile/host.rs))
into `AgentLoopDriverHost`, so `host.poll_inputs(...)` is unavailable
and the executor's steering-drain step (master doc §8 step 2) is a
no-op. `TextOnlyModelReplyDriver` never polls, so the gap is invisible
for the first slice. `PlannedDriver` (WS-7) does — but until WS-11
lands, every drain returns empty.

WS-11 ships the adapter that bridges the `LoopInputPort` trait to a
host-owned queue substrate:

- A neutral `HostInputQueue` trait owned by `ironclaw_loop_support`,
  modeled on the existing in-memory test fixture at
  [`crates/ironclaw_reborn/tests/loop_driver_host.rs:3360`](../../../crates/ironclaw_reborn/tests/loop_driver_host.rs).
- A `HostQueueLoopInputPort` adapter that implements the
  `LoopInputPort` trait from
  [`host.rs:698`](../../../crates/ironclaw_turns/src/run_profile/host.rs)
  by translating `(LoopInputCursor, limit)` → queue lookup →
  `LoopInputBatch` plus exact ack tokens.

WS-11 does **not** ship the concrete queue backing. The host runtime
already routes user/steering/followup messages through its own
substrate (channels, threads, gateway); plugging that into the
neutral `HostInputQueue` trait is a host-side PR and is intentionally
out of scope. This brief defines only the seam.

Crate ownership (per master doc §12 follow-up rule):

- **Adapter + host queue seam** — `ironclaw_loop_support`. The existing
  adapters there (`ThreadBackedLoopContextPort`,
  `ThreadBackedLoopTranscriptPort`, `ThreadBackedLoopModelPort`,
  `EmptyLoopCapabilityPort` in
  [`crates/ironclaw_loop_support/src/lib.rs`](../../../crates/ironclaw_loop_support/src/lib.rs)) are the templates.
- **Composition wiring** — `ironclaw_reborn` (`RebornLoopDriverHostFactory`
  gains `with_input_queue()` and composes the adapter at host-build time).
- **Contract crate** — `ironclaw_turns` changes `LoopInputPort::ack_inputs`
  from cursor-through ack to exact-token ack and exposes the token type
  the adapter passes through. `LoopInputCursor` remains the read position.

## 2. Files

### NEW
- `crates/ironclaw_loop_support/src/input_port.rs` — `HostQueueLoopInputPort`
  adapter implementing `LoopInputPort`.
- `crates/ironclaw_loop_support/src/input_queue.rs` — `HostInputQueue`
  trait, `HostInputBatch` value type, `HostInputQueueError`.

### MODIFIED
- `crates/ironclaw_turns/src/run_profile/host.rs` —
  `LoopInputAckToken`, exact-token `ack_inputs`, and the matching
  `LoopInputBatch`/envelope metadata needed by the executor.
- `crates/ironclaw_loop_support/src/lib.rs` — module declarations and
  re-exports.
- `crates/ironclaw_reborn/src/loop_driver_host.rs` —
  `RebornLoopDriverHostFactory` gains an `input_queue: Option<Arc<dyn HostInputQueue>>`
  field and the `with_input_queue()` builder method; the factory's host-build step
  composes `HostQueueLoopInputPort` (or the stub `EmptyLoopInputPort` when unset).

### NOT TOUCHED
- `crates/ironclaw_reborn/tests/loop_driver_host.rs` — the in-memory
  fixture stays as a unit-test convenience; new prod code uses the
  adapter from `ironclaw_loop_support`.

## 3. Specification

### 3.1 `HostInputQueue` trait

```rust
//! crates/ironclaw_loop_support/src/input_queue.rs

use async_trait::async_trait;
use ironclaw_turns::run_profile::{LoopInput, LoopInputCursorToken};
use ironclaw_turns::TurnRunId;
use thiserror::Error;

/// Host-owned input queue surface. The host runtime exposes one
/// implementation backed by its actual user-input / steering /
/// followup substrate (channels, threads, gateway). `HostQueueLoopInputPort`
/// adapts this surface to the `LoopInputPort` contract the loop calls.
///
/// Cursor semantics:
///
/// - Tokens are opaque to the loop. Implementations may use a
///   monotonically increasing sequence, a UUID-based generation, or a
///   compound key — the only requirement is that `next_after(run_id,
///   cursor, _)` returns the first input strictly after `cursor` (or
///   an equivalent point if `cursor` was returned by
///   [`origin_input_cursor_token`](../../../crates/ironclaw_turns/src/run_profile/host.rs:635)
///   as the run-start origin).
/// - Cursors are read positions, not ack identities. A batch may contain
///   control inputs before, between, or after user-facing inputs. The
///   executor must be able to ack only the inputs it actually consumed,
///   so acking is by per-input token rather than "through cursor".
/// - `ack_consumed(run_id, tokens)` MUST be at-most-once: acking the
///   same token twice is a no-op. The host queue may drop acked inputs
///   from physical storage, but does not have to.
///
/// Lifetime: implementations are per-host-process. Each adapter binds
/// to one `run_id` at host-build time. Cross-run polls are explicitly
/// not supported — the executor only ever consumes its own run's
/// inputs.
#[async_trait]
pub trait HostInputQueue: Send + Sync {
    async fn next_after(
        &self,
        run_id: TurnRunId,
        after: LoopInputCursorToken,
        limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError>;

    async fn ack_consumed(
        &self,
        run_id: TurnRunId,
        tokens: Vec<LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInputBatch {
    pub inputs: Vec<HostInputEnvelope>,
    pub next_cursor: LoopInputCursorToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInputEnvelope {
    pub input: LoopInput,
    pub cursor: LoopInputCursorToken,
    pub ack_token: LoopInputAckToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoopInputAckToken(String);

#[derive(Debug, Error)]
pub enum HostInputQueueError {
    #[error("input queue unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("cursor invalid for run: {reason}")]
    InvalidCursor { reason: String },
    #[error("input queue internal error")]
    Internal,
}
```

`HostInputBatch` is intentionally richer than the contract crate's
`LoopInputBatch` (at
[`host.rs:665`](../../../crates/ironclaw_turns/src/run_profile/host.rs)).
It carries per-input cursors and ack tokens so the adapter can advance
the executor's read cursor without pretending every earlier input was
consumed. The adapter wraps the returned cursor in a `LoopInputCursor`
bound to the run context.

### 3.2 Adapter `HostQueueLoopInputPort`

```rust
//! crates/ironclaw_loop_support/src/input_port.rs

use std::sync::Arc;
use async_trait::async_trait;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopInputBatch, LoopInputCursor,
    LoopInputPort, LoopRunContext, LoopRunInfoPort,
};
use crate::input_queue::{HostInputQueue, HostInputQueueError, LoopInputAckToken};

pub struct HostQueueLoopInputPort {
    queue: Arc<dyn HostInputQueue>,
    run_context: LoopRunContext,    // frozen at host-build time
}

impl HostQueueLoopInputPort {
    pub fn new(queue: Arc<dyn HostInputQueue>, run_context: LoopRunContext) -> Self {
        Self { queue, run_context }
    }
}

#[async_trait]
impl LoopInputPort for HostQueueLoopInputPort {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        if !after.is_for_run(&self.run_context) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Invalid,
                "cursor scope/run_id does not match port's run".to_string(),
            ));
        }

        let host_batch = self
            .queue
            .next_after(self.run_context.run_id, after.token().clone(), limit)
            .await
            .map_err(host_queue_error_into_host_error)?;

        Ok(LoopInputBatch {
            inputs: host_batch.inputs.into_iter().map(|envelope| envelope.input).collect(),
            next_cursor: LoopInputCursor::from_host_token(
                &self.run_context,
                host_batch.next_cursor,
            ),
        })
    }

    async fn ack_inputs(&self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        self.queue
            .ack_consumed(self.run_context.run_id, tokens)
            .await
            .map_err(host_queue_error_into_host_error)
    }
}

fn host_queue_error_into_host_error(e: HostInputQueueError) -> AgentLoopHostError {
    match e {
        HostInputQueueError::Unavailable { reason } =>
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, reason),
        HostInputQueueError::InvalidCursor { reason } =>
            AgentLoopHostError::new(AgentLoopHostErrorKind::Invalid, reason),
        HostInputQueueError::Internal =>
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "input queue internal error".to_string(),
            ),
    }
}
```

The real implementation should preserve the envelope metadata returned
by `poll_inputs` in a small per-host local cache keyed by input id so
`ack_inputs(tokens)` can be called after the next durable checkpoint.
The contract crate can expose this directly as
`ConsumedInputAck { token, cursor }` if that is cleaner at implementation
time; the required invariant is exact input ack, not cursor-through ack.

The error-mapping helper is the channel-edge boundary required by
`.claude/rules/error-handling.md`: raw queue errors are collapsed
into the typed `AgentLoopHostError` variants the executor knows how
to consult `RecoveryStrategy` against.

### 3.3 Variant routing

The port emits **every** `LoopInput` variant returned by the queue —
including the four control variants:

- `Interrupt { kind }` (UserInterrupt | HostShutdown)
- `Cancel { reason_kind }` (UserRequested | Superseded | Policy)
- `GateResolved { gate_ref }`
- `CapabilitySurfaceChanged { version }`

The executor (master doc §8 step 2) partitions by kind in the steering
and followup drains, but it must **not** cursor-ack past unhandled
control inputs. The drain may consume only a contiguous run of
user-facing inputs before the first control input in the batch. If the
first unprocessed input is `Cancel`, `Interrupt`, `GateResolved`, or
`CapabilitySurfaceChanged`, the drain returns `ControlFirst` and leaves
both `state.input_cursor` and user-facing ack tokens unchanged; the
dedicated control path handles that input before user-drain continues.

The port does **not** filter. Stateless about loop policy keeps the
adapter testable and keeps the policy decision in one place. Exact ack
tokens are what make this safe: a user-facing drain can ack only the
messages it actually appended and can never accidentally consume a
control event with a broad `ack_through(cursor)`.

### 3.3.5 `GateResolved` wakeup path for blocked runs

`GateResolved` is not a steering message. A run that returned
`LoopExit::Blocked` is no longer inside the executor's steering drain,
so filtering the event out of step 2 is only correct if the host has a
separate wakeup path for blocked runs. WS-11 defines that path:

1. Approval/auth/resource subsystems enqueue
   `LoopInput::GateResolved { gate_ref }` for the blocked run when
   the gate clears.
2. The same enqueue operation notifies a host-owned
   `BlockedRunResumer` (name illustrative; the implementation can
   live in the run scheduler / turn coordinator layer).
3. `BlockedRunResumer` claims the blocked run, verifies the cleared
   `gate_ref` matches `LoopExit::Blocked.gate_ref` / the latest
   `BeforeBlock` checkpoint metadata, loads the checkpoint payload via
   WS-10's `load_checkpoint_payload`, and calls `PlannedDriver::resume`.
4. The `GateResolved` ack token is acked only after the resume claim is
   durable. If resume fails before claim, the unacked input is
   redelivered and the resumer retries. If resume starts and later
   returns another `LoopExit::Blocked`, that new blocked exit owns the
   next gate.

This keeps the executor single-purpose: active runs poll inputs during
iteration; blocked runs are re-entered by the host scheduler in
response to a control input. `GateResolved` may still appear in a
normal poll if a gate clears before the executor returns `Blocked`;
the steering drain ignores it because the active capability path has
already observed the capability outcome.

### 3.4 Idempotency contract

Four rules that adapters and queue implementations both honor:

1. **Monotonic cursors per run** — for any sequence of
   `next_after(run_id, c, _)` calls, the returned `next_cursor` is
   either strictly after `c` or equal to `c` if no new inputs are
   available. Never moves backwards.
2. **Polled-but-unacked inputs are redeliverable.** If
   `next_after(run_id, c0, n)` returns `(inputs, c1)` and the caller
   does not ack before crashing, the next call to
   `next_after(run_id, c0, n)` MUST return the same `inputs` (modulo
   later arrivals). Acks are advisory eviction; reads are
   re-runnable. This matches the master doc's resume guarantee.
3. **Ack is exact and at-most-once.** `ack_consumed(run_id, tokens)` on
   tokens already acked is a no-op, not an error. Implementations drop
   only the tokened inputs, never every input before a cursor.
4. **Durable cursor before physical ack.** The executor must persist
   the advanced `LoopExecutionState.input_cursor` in a checkpoint before
   calling `ack_inputs(tokens)`. If the worker crashes before the
   checkpoint, the old cursor redelivers the inputs. If it crashes after
   the checkpoint but before ack, the advanced cursor skips them on
   resume and a later best-effort ack may reclaim storage.

These rules are documented on `HostInputQueue` so substrate
implementations have a contract to satisfy. Brief verification
includes explicit tests for each.

### 3.5 Construction binding

`HostQueueLoopInputPort` is constructed **once per claimed run** by
`PlannedDriver` and embeds the `LoopRunContext` for cursor scope
validation. Reusing a port across runs is a programming error — the
scope/run_id check in `poll_inputs`/`ack_inputs` rejects it
defensively.

## 4. Composition in `RebornLoopDriverHostFactory`

The implementation departs from the originally planned `PlannedDriverConfig` field.
`input_queue` is wired in via the builder method `with_input_queue()` on
`RebornLoopDriverHostFactory`, which is the factory composition point already
used for all other per-host-build dependencies (store, gateway, etc.).

```rust
//! crates/ironclaw_reborn/src/loop_driver_host.rs (delta)

impl<S, G> RebornLoopDriverHostFactory<S, G> {
    /// Attaches a host input queue; the factory composes `HostQueueLoopInputPort`
    /// for each claimed run. If not set, the factory falls back to the stub
    /// `EmptyLoopInputPort`.
    pub fn with_input_queue(mut self, queue: Arc<dyn HostInputQueue>) -> Self {
        self.input_queue = Some(queue);
        self
    }
}
```

At host-build time (inside the `HostFactory::build_host` call path), the factory
constructs a `HostQueueLoopInputPort` from the stored queue and the per-run
`LoopRunContext`:

```rust
let input: Arc<dyn LoopInputPort> = match self.input_queue.as_ref() {
    Some(queue) => Arc::new(HostQueueLoopInputPort::new(queue.clone(), run_context.clone())),
    None => Arc::new(EmptyLoopInputPort),
};
```

The host runtime's queue implementation is constructed at app-startup
time (it's a singleton across the process) and passed into the factory
via `with_input_queue`. Per-run binding happens inside the factory's
host-build step, not at `PlannedDriverConfig` construction time.

## 5. Verification

Unit tests (in `crates/ironclaw_loop_support`):

- `input_port::tests::poll_returns_inputs_in_order` — fake queue with
  three inputs (UserMessage, Steering, FollowUp); poll from origin,
  assert inputs are returned in that order.
- `input_port::tests::poll_after_exact_ack_returns_empty_for_consumed`
  — poll, ack only the returned user-facing tokens, poll from the
  advanced cursor; assert no consumed user-facing input is returned.
- `input_port::tests::polled_unacked_input_is_redelivered` — poll
  without acking; poll again from the same `after` cursor; assert
  the same inputs come back. Guards §3.4 rule 2.
- `input_port::tests::ack_idempotent` — ack twice; assert second call
  returns `Ok(())`. Guards §3.4 rule 3.
- `input_port::tests::exact_ack_does_not_consume_control_before_user`
  — fake queue returns `[Cancel, UserMessage]`; ack the user token;
  assert the cancel token remains unacked and visible to the control
  consumer.
- `input_port::tests::cursor_for_different_run_is_rejected` — build
  port for `run_id_a`, poll with a cursor scoped to `run_id_b`;
  assert `Invalid` error and that the queue is not touched.
- `input_port::tests::control_inputs_pass_through_unfiltered` — fake
  queue returns `[Cancel, CapabilitySurfaceChanged, Interrupt,
  GateResolved]`; assert all four reach the caller; the port does
  not strip them. Locks §3.3.
- `input_port::tests::host_queue_unavailable_maps_to_unavailable_host_error`
  — fake queue returns `HostInputQueueError::Unavailable`; assert
  `AgentLoopHostErrorKind::Unavailable` on the port.

Integration tests (in `crates/ironclaw_reborn`, gated behind
`ironclaw_agent_loop/test-support` from WS-8):

- `planned_driver_consumes_steering_message_mid_loop` — enqueue
  Steering before iteration 2's `plan_context_request`; drive;
  assert the model port receives a `LoopPromptBundle` whose
  `messages` includes the steering message ref.
- `planned_driver_control_before_steering_does_not_ack_steering` —
  enqueue `[Cancel, Steering]`; drive until cancellation exits; assert
  the steering ack token was not acked and no prompt consumed it.
- `planned_driver_crash_between_drain_and_checkpoint_redelivers_input`
  — poll and append a steering message, crash before `BeforeModel`;
  resume from the prior checkpoint and assert the same input is
  redelivered rather than lost.
- `planned_driver_followup_restarts_after_natural_stop` — first
  iteration emits assistant reply; second iteration's followup drain
  picks up a queued FollowUp; assert a second model call happens
  and the run exits `Completed` only after that drain returns empty.
- `planned_driver_gate_resolved_resumes_blocked_run` — first run
  returns `LoopExit::Blocked` with a `gate_ref` and `BeforeBlock`
  checkpoint; enqueue `GateResolved { gate_ref }`; drive the
  `BlockedRunResumer`; assert it loads the WS-10 checkpoint, calls
  `PlannedDriver::resume`, acks the cursor after claiming resume, and
  reaches `LoopExit::Completed` once the capability succeeds.
- `planned_driver_drains_when_iteration_caps` — queue all 33 inputs;
  assert `LoopFailureKind::IterationLimit` exits cleanly, no panic
  on the unconsumed input cursor.

## 6. Out of scope (for this brief)

- **Concrete queue substrate.** The host's actual implementation
  (probably a `tokio::sync::mpsc`-backed per-thread store, or a
  durable queue in `src/agent/`) is a host-runtime PR. The brief
  defines only the seam.
- **Cross-channel fan-in.** Gateway, CLI, HTTP, Telegram, Slack all
  produce inputs; merging them into a single queue is the substrate's
  job, not the adapter's.
- **Durable replay across host restart.** If `HostInputQueue` is
  in-memory only, a host restart loses unacked inputs. Persistent
  queue impls are encouraged but optional — the loop honors at-most-
  once delivery either way.
- **Priority ordering.** Inputs are FIFO per run; no priority lanes.
  If steering should "jump" in some loop family, the substrate
  inserts at head; the adapter is order-preserving.
- **Backpressure / quotas.** The `limit` parameter on `poll_inputs`
  is advisory; queues may return fewer. Capping the producer is
  substrate concern.
- **Cross-run delivery.** Inputs are addressed to one `run_id`; if a
  user re-submits after a run terminates, the runner promotes the
  new submission into a new run via `TurnCoordinator`. Inputs do
  not move between runs.

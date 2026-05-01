# Inline Gate Await (Tier 0 + Tier 1)

**Status:** Implemented — 2026-05-01
**Date:** 2026-05-01
**Owner:** engine v2 / bridge

## Problem

When a CodeAct (Tier 1, Monty) script makes a tool call that requires
approval, the user sees:

```
Error: RuntimeError: execution paused by gate 'approval'
```

instead of an approval prompt. The script aborts, the gate is never
surfaced to the user.

Concretely: in `crates/ironclaw_engine/src/executor/scripting.rs`, the
async tool-resolve path (`resolve_tool_future`, line 1740-1766) catches
`Err(EngineError::GatePaused { .. })` from `EffectExecutor::execute_action`,
emits an `ApprovalRequested` event, and converts the gate to a
`MontyException::new(ExcType::RuntimeError, "execution paused by gate
'approval'")`. Python sees a catchable `RuntimeError`. If user code
doesn't catch, the script crashes with the message above.

The synchronous preflight path (line 841-852) handles this correctly
for policy-level approval gates, but only by aborting the entire
CodeAct turn and returning `need_approval: Some(outcome)` to the
orchestrator. On approval the orchestrator re-runs the LLM step from
scratch, regenerates code, and re-executes CodeAct from the top —
which **double-executes any non-idempotent tool calls that ran before
the gated one** in the same script.

## Goals

1. Eliminate the `RuntimeError` leak. Gates never reach Python as
   exceptions.
2. Eliminate double-execution of side effects on resume. A tool that
   ran successfully before the gate must not run a second time.
3. Stay within the existing `PendingGateStore` / `/api/chat/gate/resolve`
   / SSE machinery. Don't fork a parallel UI surface for CodeAct gates.

## Non-goals

- Monty VM serialization across process restarts. Out of scope.
- Surviving an IronClaw process restart while a CodeAct gate is
  pending. Accepted loss: stranded gates expire after 30 min and the
  user retries.
- Combining multiple parallel gate prompts into one approval card.
  Future work — for now, gates are surfaced one at a time.

## Design

**Keep the Monty VM alive while the gate is pending.** The script's
local state, frame stack, and prior tool results are all in memory.
On approval, deliver the result back via the same `call.resume(...)`
path the VM was already going to use; the script continues from the
exact suspension point. No replay, no restart, no double execution.

### Core mechanism: a `GateController` callback on `ThreadExecutionContext`

```rust
// crates/ironclaw_engine/src/gate/mod.rs

pub struct GatePauseRequest<'a> {
    pub gate_name: &'a str,
    pub action_name: &'a str,
    pub call_id: &'a str,
    pub parameters: &'a serde_json::Value,
    pub resume_kind: ResumeKind,
    pub paused_lease: Option<CapabilityLease>,
    pub resume_output: Option<serde_json::Value>,
}

#[async_trait]
pub trait GateController: Send + Sync {
    /// Pause execution until the gate is resolved by the user or
    /// external system. The implementation is responsible for any
    /// persistence, UI/SSE emission, and channel registration needed
    /// to surface the gate.
    async fn pause(&self, req: GatePauseRequest<'_>) -> GateResolution;
}
```

`ThreadExecutionContext` gets:

```rust
pub gate_controller: Option<Arc<dyn GateController>>,
```

Optional so existing tests and Tier 0 paths that don't construct a
controller continue to work unchanged.

### Bridge implementation: `BridgeGateController`

Wraps the existing `PendingGateStore` and adds an in-memory channel
registry:

```rust
// src/bridge/gate_controller.rs

pub struct BridgeGateController {
    pending: Arc<PendingGateStore>,
    sse: Option<Arc<SseManager>>,
    channels: Arc<ChannelManager>,
    auth_manager: Option<Arc<AuthManager>>,
    extension_manager: Option<Arc<ExtensionManager>>,
    tools: Arc<ToolRegistry>,
    pending_resolutions: Mutex<HashMap<Uuid, oneshot::Sender<GateResolution>>>,
    // … plus the user_id / channel / conversation_id that
    //   construct_pending_gate needs; supplied per controller instance,
    //   one controller per active thread execution.
}

#[async_trait]
impl GateController for BridgeGateController {
    async fn pause(&self, req: GatePauseRequest<'_>) -> GateResolution {
        let request_id = Uuid::new_v4();
        let pending = self.build_pending_gate(request_id, &req);

        // 1. Store in PendingGateStore (DB-backed) — gives us the
        //    existing UI rendering, history rehydration, expiry, and
        //    channel-mismatch protection for free.
        let _ = self.pending.insert(pending.clone()).await;

        // 2. Register an in-memory resolution channel keyed by request_id.
        let (tx, rx) = oneshot::channel();
        self.pending_resolutions.lock().await.insert(request_id, tx);

        // 3. Emit the SSE / channel-native gate prompt (existing
        //    `send_pending_gate_status` flow).
        self.emit_gate_status(&pending).await;

        // 4. Await resolution.
        match rx.await {
            Ok(resolution) => resolution,
            // Sender dropped (process shutting down or channel was
            // displaced) → treat as cancel.
            Err(_) => GateResolution::Cancelled,
        }
    }
}

impl BridgeGateController {
    pub async fn try_deliver(&self, request_id: Uuid, resolution: GateResolution) -> bool {
        if let Some(tx) = self.pending_resolutions.lock().await.remove(&request_id) {
            let _ = tx.send(resolution);
            true
        } else {
            false
        }
    }
}
```

### Gate-resolve endpoint integration

In `src/bridge/router.rs::resolve_pending_gate`, before falling
through to the existing `execute_pending_gate_action` /
`thread_manager.resume_thread` path, **try the in-memory channel
first**:

```rust
// existing: take verified gate from store
let pending = self.pending_gates.take_verified(...).await?;

// NEW: try in-memory delivery
if let Some(controller) = state.bridge_gate_controller_for(&key) {
    if controller.try_deliver(pending.request_id, resolution.clone()).await {
        // The CodeAct VM is alive and waiting; it will continue
        // execution itself. We just need to emit the GateResolved SSE
        // for the UI and return Pending.
        self.emit_gate_resolved_sse(state, message, &pending, &resolution);
        return Ok(BridgeOutcome::Pending);
    }
}

// fall through to legacy path: re-enter the thread via
// execute_pending_gate_action / resume_thread (Tier 0 flow)
```

Result: gates fired from CodeAct keep the VM alive and resolve via the
channel. Gates fired from Tier 0 (no live VM) take the existing
re-entry path.

### `scripting.rs` changes

Replace the two broken sites:

**Sync preflight (line 841-852)** — the early return:

```rust
PreflightResult::GatePaused(outcome) => {
    let resolution = match &context.gate_controller {
        Some(controller) => controller.pause(GatePauseRequest {
            gate_name: &outcome.gate_name,
            action_name: &outcome.action_name,
            call_id: &outcome.call_id,
            parameters: &outcome.parameters,
            resume_kind: outcome.resume_kind.clone(),
            paused_lease: outcome.paused_lease.as_deref().cloned(),
            resume_output: outcome.resume_output.clone(),
        }).await,
        None => {
            // Tests / legacy callers without a controller — preserve
            // the current behavior (return need_approval).
            return Ok(CodeExecutionResult {
                need_approval: Some(outcome),
                /* … */
            });
        }
    };

    match resolution {
        GateResolution::Approved { always } => {
            // Auto-approve registration, lease re-consume, continue
            // through the Approved path.
            …
        }
        GateResolution::Denied { reason } => {
            // Resume Monty with a TYPED exception so user code can
            // catch it but it's not the misleading RuntimeError. Use
            // PermissionError; the message is the deny reason.
            let ext_result = ExtFunctionResult::Error(MontyException::new(
                ExcType::PermissionError,
                Some(reason.unwrap_or_else(|| "denied by user".into())),
            ));
            // resume Monty with the exception, continue loop
        }
        GateResolution::Cancelled | GateResolution::ExternalCallback { .. } => {
            // Same as Denied — script gets PermissionError.
        }
        GateResolution::CredentialProvided { .. } => {
            // Auth gate completed; rebuild lease/credential state and
            // retry the action through the Approved path.
        }
    }
}
```

**Async output gate (line 1740-1766)** — the bug site. Same shape:
on `Err(EngineError::GatePaused { .. })` from `effects.execute_action`,
call `controller.pause(...)`, branch on resolution. On `Approved`,
re-execute the action with the refunded lease (or use `resume_output`
if the gate handed back a held result). On `Denied/Cancelled`, surface
`PermissionError` to the script.

### Resource limit adjustment

Monty's `ResourceLimits::max_duration` is wall-clock from start of
`runner.start(...)`. It ticks during gate awaits. The default stays
at **30 seconds** — the same as before this change.

Why not bump to 30 minutes (which would match `PendingGate.expires_at`
and let humans approve at human latency)? Because the existing
`sandbox_enforces_cpu_limits` test relies on `max_duration` to
terminate `while True: x += 1` (a CPU-bound script with no
allocations to trip the allocation/memory caps). Raising the cap
hangs that test for the new value.

**Tradeoff**: an approval that takes longer than 30 s times out the
script. The user re-prompts and the LLM re-issues the action. Most
approvals come back in seconds; this is acceptable for the common
case.

**Follow-up**: a proper "active CPU vs paused" timer split — only
count CPU time during VM execution, not during gate-await futures.
Either mutate `LimitedTracker::set_max_duration` around each await
to extend the budget, or expose a per-call tracker handle. Out of
scope for this PR.

### Restart behavior

If IronClaw restarts while a CodeAct gate is pending:

- The DB-stored `PendingGate` still exists.
- The in-memory `oneshot::Sender` is gone.
- User clicks approve → `resolve_pending_gate` → `try_deliver` returns
  `false` (no channel) → falls through to legacy `execute_pending_gate_action`.
- `execute_pending_gate_action` re-enters the thread → re-runs LLM →
  CodeAct re-runs from the top. **This is the bug we're trying to
  prevent.**

Cleanup: on startup, iterate `PendingGateStore`, find gates whose
`gate_name == "approval"` and whose source thread was in `Running`
state at shutdown, mark them expired with reason
`"interrupted by restart"`. The user gets a clean error and retries.

This requires a flag on `PendingGate` distinguishing "live-VM gate"
from "Tier 0 re-entry gate", since the latter is genuinely
restart-survivable. Add `requires_live_vm: bool` (default `false`).
CodeAct sets it to `true`. Startup sweep targets only `requires_live_vm`
gates.

### Wall-clock semantics for signals

A `Stop` signal during a gate await must cancel the pause. Use
`tokio::select!` in `BridgeGateController::pause`:

```rust
tokio::select! {
    res = rx => res.unwrap_or(GateResolution::Cancelled),
    _ = stop_signal.notified() => GateResolution::Cancelled,
}
```

`InjectMessage` during a gate await is queued and surfaces only after
the pause resolves — the script is mid-statement, can't accept a new
message inline.

### Concurrency cap

A paused VM holds its frame stack and closed-over `Arc`s in memory —
small (kilobytes per script), but should be capped to prevent a stuck
user from accumulating dozens. Per-user cap (default 8) on
concurrent in-script gate pauses; ninth attempt rejects with a clean
"too many pending approvals" error. Tunable via env var.

## Scope: which gates are unified

| Resume kind | Tier 0 path | Tier 1 path |
|---|---|---|
| `Approval` | `GateController::pause` (NEW, this PR) | `GateController::pause` (NEW, this PR) |
| `Authentication` | Legacy (`execute_pending_gate_action` + `AuthManager`) | Legacy (returns `need_approval`, orchestrator pauses) |
| `External` | Legacy | Legacy |

Auth and External gates stay on the existing re-entry path because:

- Auth completion installs a credential in the secrets store; the
  *new* credential availability is what makes the retried action
  succeed. No live in-flight state to hand back to.
- External callbacks may arrive long after the originating script
  has cleaned up; they're inherently async-via-DB.

Approval is the only resume kind where re-entry causes the
double-execution bug (the user already gave the answer; we just
need to deliver it back to the suspended call).

## Migration / blast radius

- **Tier 0** (structured tool calls): both gate sites in
  `structured.rs` (preflight at line 185, mid-execution at line 457)
  call `GateController::pause` for `Approval` gates. The loop stays
  inside `execute_action_calls` until the gate resolves. On approval,
  the gated action is re-executed (lease re-consumed, credential
  re-injected); on denial, an `ActionFailed` event is emitted and the
  batch continues with that single call marked failed.
- **Tier 1** (CodeAct): same callback used. VM stays alive across
  the gate.
- **Bridge**: `handle_with_engine_inner`'s
  `ThreadOutcome::GatePaused` arm continues to handle `Authentication`
  and `External` resume kinds via the existing path. `Approval` no
  longer flows through this arm because the engine handles it
  inline.
- **Resolve endpoint**: `resolve_pending_gate` checks
  `controller.try_deliver` first. On success (in-flight `Approval`
  gate), short-circuits with a UI event. On miss, falls through to
  the existing `execute_pending_gate_action` path — preserved as a
  fall-through but never hit in normal operation.

## Restart semantics

If IronClaw restarts while an `Approval` gate is pending:

- DB-stored `PendingGate` row still exists.
- In-memory `oneshot::Sender` is gone.
- User clicks approve → `try_deliver` returns false → fall-through
  to `execute_pending_gate_action` re-entry → re-runs LLM step →
  the bug we're trying to prevent recurs.

Mitigation in this PR: on startup, sweep `PendingGateStore` for
`Approval`-kind gates and mark them expired (emit `GateExpired` SSE,
remove from store). User sees "approval expired due to restart" and
retries. Auth/External rows are untouched.

This is simpler than the `requires_live_vm` flag I proposed earlier:
since `Approval` is now always handled inline, **every** unresolved
`Approval` row at startup represents a stranded in-flight gate and
should be invalidated.

## Testing

- **Unit (engine):** `scripting.rs` test that drives a CodeAct
  call where the tool returns `Err(EngineError::GatePaused)` mid-execution,
  with a stub `GateController` that returns `GateResolution::Approved`,
  asserts the script completes and the gated tool's result is
  delivered to Python.
- **Unit (engine):** same shape with `Denied`, asserts a
  `PermissionError` is raised inside the script (catchable by
  `try/except`), and the Python `except` branch runs.
- **Unit (engine):** `Cancelled` resolution also raises
  `PermissionError` — script-level indistinguishable from `Denied`,
  intentional.
- **Integration (bridge):** drive `resolve_pending_gate` with both an
  in-memory channel registered (asserts `try_deliver` succeeds, no
  re-entry happens) and without (asserts legacy
  `execute_pending_gate_action` still runs). Per the testing rule
  (`Test Through the Caller, Not Just the Helper`), this is the
  caller-level coverage that catches a wrapper that silently drops
  the controller.
- **Integration (bridge):** start a thread, send a CodeAct script
  that gates, send `Stop`, assert the pause cancels and the thread
  terminates.

## Build order

1. `GatePauseRequest` / `GateController` plumbing in
   `crates/ironclaw_engine/src/gate/mod.rs` and
   `traits/effect.rs::ThreadExecutionContext`. Restrict to
   `Approval` resume-kind for this PR.
2. `BridgeGateController` in `src/bridge/gate_controller.rs` —
   construction, `pause`, `try_deliver`, `emit_gate_status`.
3. Wire one controller per thread execution from
   `src/bridge/router.rs::handle_with_engine_inner` into
   `ThreadExecutionContext.gate_controller`.
4. **Tier 1 sites:** replace `scripting.rs:841-852` (sync
   preflight) and `scripting.rs:1740-1766` (async output gate) with
   `controller.pause(...).await`. Branch on `GateResolution`.
5. **Tier 0 sites:** replace `structured.rs:185-218` (preflight)
   and `structured.rs:457-496` (mid-execution) with
   `controller.pause(...).await` for `Approval` gates. Other resume
   kinds keep current behavior.
6. Update `resolve_pending_gate` to `try_deliver` first; legacy
   path stays as a fall-through for Auth/External and post-restart
   stragglers.
7. Bump `default_limits().max_duration` to 30 min. Document in
   `crates/ironclaw_engine/CLAUDE.md`.
8. Startup sweep: invalidate all `Approval`-kind `PendingGate` rows
   on boot.
9. Tests for both tiers (sync preflight, async output, denial,
   stop-during-wait, restart cleanup).

## Open questions / follow-ups (not in this PR)

- Active-CPU vs paused-clock split for `max_duration`. Current 30 min
  is generous; a runaway script combined with a never-resolved gate
  could pin a VM for the full duration.
- Multi-gate single-prompt UX (`asyncio.gather` of two gating tools).
- `InjectMessage` during a gate wait — current behavior queues; may
  want to surface a marker in the script so the LLM knows the user
  said something.
- Migration to `GateController` for Tier 0: would let us delete the
  thread re-entry path entirely and unify both tiers on one mechanism.
  Worth considering after this lands.

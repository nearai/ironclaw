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

#[derive(Debug, Clone)]
pub struct GatePauseRequest {
    pub thread_id: ThreadId,
    pub user_id: String,
    pub gate_name: String,
    pub action_name: String,
    pub call_id: String,
    pub parameters: serde_json::Value,
    pub resume_kind: ResumeKind,
}

#[async_trait]
pub trait GateController: Send + Sync {
    /// Pause execution until the gate is resolved by the user or
    /// external system. The implementation is responsible for any
    /// persistence, UI/SSE emission, and channel registration needed
    /// to surface the gate.
    async fn pause(&self, req: GatePauseRequest) -> GateResolution;
}

/// Default impl that immediately cancels every pause request. Use for
/// post-resolution replay paths, mission protected writes, and tests.
pub struct CancellingGateController;

impl CancellingGateController {
    pub fn arc() -> Arc<dyn GateController> { Arc::new(Self) }
}

#[async_trait]
impl GateController for CancellingGateController {
    async fn pause(&self, _: GatePauseRequest) -> GateResolution {
        GateResolution::Cancelled
    }
}
```

`ThreadExecutionContext` gets:

```rust
pub gate_controller: Arc<dyn GateController>,
```

**Required, not optional.** A previous iteration made it optional to
avoid touching test fixtures, but that left a fall-back path in the
executors that re-emitted the original `"execution paused by gate"`
RuntimeError when the field was `None`. Removing the `Option` makes it
a compile error to forget to wire a controller, and `CancellingGateController`
is the explicit drop-in for paths that don't pause — gates surface as
typed denials there, never as the legacy bug message.

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

Test mapping to the as-shipped code:

- **Unit (engine, `scripting.rs`):** `codeact_gate_inline_await_approved_delivers_result`
  — tool returns `Err(EngineError::GatePaused)` mid-execution; a stub
  `GateController` returns `Approved`; asserts the script completes
  with the tool's result.
- **Unit (engine, `scripting.rs`):** `codeact_gate_inline_await_denied_raises_in_script`
  — denial surfaces as a typed `RuntimeError("user denied tool 'X': <reason>")`,
  catchable in Python.
- **Unit (engine, `scripting.rs`):** `codeact_default_controller_cancels_approval_gates`
  — locks in the `CancellingGateController` default. Verifies the
  pre-fix `"execution paused by gate"` message never appears even
  when the controller is the inert default.
- **Unit (engine, `scripting.rs`):** `bridge::gate_controller::tests::*`
  — 4 tests cover `GateResolutions` registry semantics (unknown /
  registered / dropped-receiver / one-shot).
- **Live regression (`tests/engine_v2_gate_integration.rs`):**
  `codeact_inline_gate_await_resumes_user_reproducer` drives the
  user-reported reproducer through `ThreadManager`; asserts no
  RuntimeError leak and the post-approval retry runs.
  `codeact_inline_gate_await_denial_does_not_retry` covers denial.
- **Unit (bridge, `router.rs`):** `invalidate_stranded_approval_gates_evicts_only_approval_kind`
  — boot sweep evicts only `Approval` rows; Auth/External rows
  survive.
- **Open follow-ups:** integration test covering `resolve_gate`'s
  auto-approve install + rollback path (caller-level coverage per
  `.claude/rules/testing.md`); E2E click-through tests for the live
  approve/deny/restart flows in the UI.

`PermissionError` was the originally proposed Python exception kind
but the as-shipped code uses `RuntimeError` with a descriptive message
(`"user denied tool 'X': <reason>"`) — `RuntimeError` is what scripts
already catch, and the message is specific enough to distinguish from
other runtime errors.

## As-shipped shape

The code shipped differs from the original sketch in a few places worth
recording so a future reader doesn't get confused:

- **`gate_controller` is required, not optional.** The original sketch
  used `Option<Arc<dyn GateController>>` and let executors fall back
  to the V1 unwind path when `None`. That fallback re-emitted the bug
  message, so it was removed. `CancellingGateController` is the
  explicit drop-in for paths that don't pause (post-resolution replay,
  mission protected writes, tests).
- **Bounded retry on Approval.** Both `scripting::drive_inline_gate`
  (Tier 1 async) and `structured::execute_with_inline_gate_retry`
  (Tier 0) cap the re-pause loop at `MAX_INLINE_GATE_RETRIES = 3`.
  Well-behaved chains (auto-approve installed before delivery)
  converge in 1–2 iterations; the cap only ever fires on a buggy host
  controller.
- **`denial_reason_for_resolution` helper.** Pulled into
  `executor::scripting` and used by Tier 0 + Tier 1 sites so denial
  messages can't drift between executors.
- **`max_duration` stays at 30 s.** The original plan considered
  bumping it to 30 min so human approvals fit; the existing
  `sandbox_enforces_cpu_limits` test relied on the 30 s cap to
  terminate a CPU-bound script. The active-CPU vs paused-clock split
  is on the follow-up list.

## Build order

1. `GatePauseRequest` / `GateController` / `CancellingGateController`
   in `crates/ironclaw_engine/src/gate/mod.rs`. Required field
   (`Arc<dyn GateController>`, not `Option`) on
   `traits/effect.rs::ThreadExecutionContext`. Restricted to
   `Approval` resume-kind for this PR.
2. `BridgeGateController` in `src/bridge/gate_controller.rs` —
   construction, `pause`, `try_deliver`, `emit_gate_status`,
   per-execution context registry.
3. Wire one controller per thread execution from
   `src/bridge/router.rs::handle_with_engine_inner` into
   `ThreadManager::set_gate_controller`. Subsequent thread spawns pick
   it up via `ExecutionLoop::new`. `ThreadManager` defaults to
   `CancellingGateController` so unwired hosts fail loud rather than
   silently.
4. **Tier 1 sites:** rewrite the sync preflight `PreflightResult::GatePaused`
   arm and the async output `resolve_tool_future` `EngineError::GatePaused`
   arm to call the controller. Async path delegates to
   `drive_inline_gate` (bounded retry).
5. **Tier 0 sites:** rewrite the preflight `RequireApproval` arm in
   `structured::execute_action_calls` to call the controller, and the
   mid-execution loop in `structured::execute_with_inline_gate_retry`
   (also bounded). Authentication/External keep the legacy re-entry
   path unchanged.
6. Update `resolve_pending_gate` to `try_deliver` the controller's
   in-memory channel first; install/rollback auto-approve preference
   around the delivery so chained gates short-circuit. Legacy
   `execute_pending_gate_action` path stays as a fall-through for
   Auth/External and post-restart stragglers.
7. Startup sweep: `invalidate_stranded_approval_gates` evicts every
   `Approval`-kind `PendingGate` row on boot and emits a
   `GateResolved` SSE with `resolution = "expired"` per row.
8. Tests for both tiers (sync preflight, async output, denial,
   approval, fallback-controller default, restart cleanup).
9. Wire `CancellingGateController` into the remaining production paths
   that build `ThreadExecutionContext` directly (mission protected
   writes, post-resolution replay) — required field, no implicit
   default.

## Open questions / follow-ups (not in this PR)

- **Active-CPU vs paused-clock split for `max_duration`.** Currently
  30 s and ticks during gate awaits — long approvals time out. A
  proper split would only count CPU time during VM execution.
- **Multi-gate single-prompt UX** (`asyncio.gather` of two gating
  tools).
- **`InjectMessage` during a gate wait** — current behavior queues;
  may want to surface a marker in the script so the LLM knows the
  user said something.
- **Migration of Auth/External to the controller.** Would let us
  delete the thread re-entry path entirely. Each needs new state
  installed (credential, callback) before the suspended call can
  succeed, so the controller's contract would need to grow.
- **Bridge integration test for `resolve_gate` rollback** — the
  auto-approve install/rollback dance in `resolve_gate` deserves a
  caller-level test per `.claude/rules/testing.md`.
- **Per-user concurrency cap on in-flight gates.** Documented but not
  implemented. A stuck user could accumulate paused VMs.
- **`mission.rs::dispatch_protected_write` controller wiring.** Currently
  uses `CancellingGateController`; if a protected write ever surfaces
  a gate the user gets a silent denial. Decide whether to wire it
  through the live controller or surface as a clearer engine error.
- **Tier 0 parallel-batch mid-execution gates.** `execute_with_inline_gate_retry`
  handles the single-call path; multi-call parallel batches with
  simultaneous gates need the same treatment (rare).

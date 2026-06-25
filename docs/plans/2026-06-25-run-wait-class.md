# Run-wait classification: one exhaustive `TurnStatus::wait_class()` + 3 site fixes

Date: 2026-06-25
Branch: `fix/reborn-run-wait-class` (off `main` @ 4f28febbe)

## Problem

`TurnStatus` (`crates/ironclaw_turns/src/status.rs`, 11 variants) has four
"parked" states — `BlockedApproval`, `BlockedAuth`, `BlockedResource`
(parked **awaiting user**) and `BlockedDependentRun` (parked **awaiting a
dependent run**). A parked run does **not** self-advance: it only leaves the
parked state when the gate is resolved (user submits approval/auth, resource
frees, or the dependent child completes).

The only shared classifier is `is_terminal()` = `Cancelled | Completed |
Failed | RecoveryRequired`. Every wait/poll loop that needs to know "is this
run still going to make progress on its own?" hand-rolls a partial predicate
on top of `is_terminal()`, and three of them mishandle parked runs.

## Verified bugs (Stage 1 — each independently confirmed with file:line, red test required before fix)

### Bug #1 — OpenAI-compat wait spins on a gate-parked run (permanent 503 loop)
`crates/ironclaw_reborn_composition/src/openai_compat_serve.rs:622`
`OpenAiResponsesThreadProjectionReader::wait_for_response_completion` is an
**unbounded** `loop`. Its only exits are: a finalized assistant message, or a
projected `Failed`/`Cancelled` status. The projection `RunStatus` wire string
(`run_status_wire`, projection.rs:1166) only emits running/completed/cancelled/
failed/killed — a parked run emits **no** `RunStatus` update at all, and
`response_status_from_projection_run_status:787` falls through `_ => None`. So a
run parked on a gate produces neither a finalized reply nor a terminal status →
the loop polls forever.

The outer caller `create_response_request`
(`crates/ironclaw_reborn_openai_compat/src/responses_workflow.rs:260`) wraps it
in `tokio::time::timeout(30s)` → so it does **not** hang the process forever,
but a gate-parked run yields a **retryable 503 with no gate info** every 30s,
indefinitely, with no way to surface that the run is waiting on the user.

Production path: `POST /v1/responses` (non-streaming) → `handlers::create_response`
→ workflow → `wait_for_response_completion`.

**Fix:** detect the parked/gate state inside the loop and return promptly with
`OpenAiResponseStatus::Incomplete` (the existing variant, responses.rs:92)
instead of spinning. The reader only has `thread_service` + `projection_stream`
(no `TurnStatus` in scope), so the available signal is the run-scoped `RunStatus`
wire string from the projection. We extend the projection wire + classifier so a
parked run emits a `RunStatus` whose status string maps to `Incomplete`, and the
loop returns on it. This keeps the wire string the canonical signal and adds a
bounded escape for parked runs.

### Bug #2 — runtime `wait_for_terminal` KILLS a gate-parked run after 180s
`crates/ironclaw_reborn_composition/src/runtime.rs:1871`
`wait_for_terminal` polls `is_terminal()` with a 180s bound
(`poll_settings.max_total`, runtime_input.rs:240) and on timeout calls
`cancel_run(SanitizedCancelReason::Timeout)` — which is **destructive**: it
cancels the run and all descendants and writes a `webui_loop_cancelled` event;
the run can no longer be resumed.

A run legitimately parked on `BlockedApproval`/`BlockedAuth`/`BlockedResource`
never satisfies `is_terminal()`, so after 180s it gets **killed** — destroying a
run that was correctly waiting for the user.

Production path: `send_user_message_internal:1576` (reached from
`send_user_message` / `send_user_message_with_cancellation`).

A gate-aware sibling `wait_for_terminal_or_gate:1931` already exists with an
**exhaustive** match that short-circuits on the three user-resolvable gates, but
it is `#[cfg(any(test, feature = "test-support"))]` and unused in production.

**Fix:** promote the gate-aware wait to production by routing
`send_user_message_internal` through it, and re-express its match via the new
`wait_class()` so the exhaustiveness lives in one place. A parked run is
returned to the caller (status carries the `Blocked*` state + `gate_ref`),
**not** cancelled. `BlockedDependentRun` keeps polling (internal wait, not
user-resolvable).

### Bug #3 — subagent completion observer hangs the parent on a parked child
`crates/ironclaw_reborn/src/subagent/completion_observer.rs:699`
`is_subagent_terminal_status` = `status.is_terminal()`. It gates
`observes_state`/`observes_event` (lines 665-671): the
`SubagentCompletionObserver` only fires on terminal child statuses. If a child
subagent parks (e.g. its own tool-approval gate → `BlockedApproval` via
`block_run`), the observer is skipped, `resume_parent` never fires, and the
parent stays in `BlockedDependentRun` **forever** (no timeout anywhere).

Reachability confirmed: subagents run through the **same** approval-gated
`HostRuntime` as the parent; a `coder`/`planner` subagent invoking an
approval/auth-gated capability parks the child run. Observer is wired in
production via `build_default_planned_runtime_inner` → `subscribe_required`.

**Fix:** widen `observes_state`/`observes_event` to also fire on the three
parked-awaiting-user states, and in `observe_committed_*` synthesize a
`Failed`-status lifecycle event (descriptive `sanitized_reason`: "subagent run
parked on a gate it cannot resolve") and route it through the **existing,
unchanged** `handle_terminal`. The terminal-only invariant of
`record_child_terminal` (gate_resolution.rs:81) is preserved because the
synthetic event carries `Failed`. The parent resumes with a failed-child payload
the model can surface, instead of hanging. (`BlockedDependentRun` is excluded —
that is a child that is itself waiting on a grandchild; it will resolve when its
own dependent run completes.)

## Canonical fix — one exhaustive classifier (no over-engineering)

Add to `crates/ironclaw_turns/src/status.rs`, next to `is_terminal()`:

```rust
/// How a wait/poll loop should treat a run in this status. Exhaustive on
/// purpose: a new `TurnStatus` variant must force a compile error here so
/// every waiter is forced to classify it, rather than silently treating it
/// as "still running" or "terminal".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunWaitClass {
    /// Still making progress on its own; keep waiting.
    Running,
    /// Parked awaiting a USER-resolvable gate (approval/auth/resource).
    /// Never self-advances; a waiter must surface it, never kill it.
    ParkedAwaitingUser,
    /// Parked awaiting a dependent (child) run. Never self-advances on its
    /// own, but resolves when the dependent run completes — so a waiter may
    /// keep polling (it is not user-resolvable through a gate facade).
    ParkedAwaitingRun,
    /// Reached a terminal success.
    TerminalSuccess,
    /// Reached a terminal failure / cancellation / recovery-required.
    TerminalFailure,
}

impl TurnStatus {
    pub fn wait_class(self) -> RunWaitClass {
        match self {
            Self::Queued | Self::Running | Self::CancelRequested => RunWaitClass::Running,
            Self::BlockedApproval | Self::BlockedAuth | Self::BlockedResource => {
                RunWaitClass::ParkedAwaitingUser
            }
            Self::BlockedDependentRun => RunWaitClass::ParkedAwaitingRun,
            Self::Completed => RunWaitClass::TerminalSuccess,
            Self::Cancelled | Self::Failed | Self::RecoveryRequired => {
                RunWaitClass::TerminalFailure
            }
        }
    }
}
```

Notes / justification of the classification:
- `CancelRequested` → `Running`: cancellation is in flight; the run will reach a
  terminal state on its own, so a waiter keeps waiting (matches `keeps_active_lock`
  and the existing `wait_for_terminal_or_gate` which treats it as "keep polling").
- The split of terminal into `TerminalSuccess`/`TerminalFailure` is cheap and
  lets callers that care (none currently must) branch without re-matching; both
  satisfy the old `is_terminal()`. `is_terminal()` is **kept unchanged** and is
  equivalent to `matches!(wait_class(), TerminalSuccess | TerminalFailure)` — a
  debug-assert/test pins that equivalence so the two never drift.
- No trait, no generics, no per-site abstraction. One enum + one method.

Per-site use:
- #2 reuses it directly: `wait_for_terminal_or_gate`'s `blocked_on_gate` match
  becomes `matches!(state.status.wait_class(), RunWaitClass::ParkedAwaitingUser)`.
- #3 uses it to decide "is this a parked-awaiting-user child?" before
  synthesizing the failed event.
- #1 lives below `ironclaw_turns` on the projection wire layer; it cannot import
  a `TurnStatus` value (it only sees wire strings). The guardrail there is the
  same *shape* (exhaustive mapping, explicit parked handling) applied to the
  projection `RunProjectionStatus`/wire — see the projection-status change.

## Stage order

1. Add `RunWaitClass` + `wait_class()` + equivalence test in `ironclaw_turns`.
2. Write the 3 red tests (drive the real callers), prove RED on this base.
3. Apply per-site fixes, prove the same tests GREEN.
4. Guardrail doc in the `ironclaw_turns` spec / `.claude/rules/types.md`.
5. fmt / clippy / test gate; code-review + thermo-nuclear; PR.

## Out of scope / follow-ups
- `crates/ironclaw_reborn_composition/src/slack_delivery.rs` migration onto
  `wait_class()` is owned by a concurrent agent — DO NOT touch here. Documented
  follow-up: migrate its hand-rolled status predicate onto `wait_class()`.
- CLI `runtime/mod.rs` and scheduler config defaults are owned by another agent;
  not touched here.

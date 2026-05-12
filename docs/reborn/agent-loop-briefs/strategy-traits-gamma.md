# WS-3 — Strategy Traits γ: Stop / Drain / Budget

**Workstream:** WS-3
**Crate touched:** `ironclaw_agent_loop`
**Depends on:** WS-0 (`LoopExecutionState`, slot types, `LoopFailureKind::NoProgressDetected`)
**Parallel with:** WS-1, WS-2
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §6, §10

---

## 1. Scope

Land the three loop-control strategies:

- `StopConditionStrategy` — async, mutates `control_state`, returns `StopOutcome`. The home of pi's `shouldStopAfterTurn` plus the production-safe no-progress detection (master doc §10).
- `InputDrainStrategy` — async, pure-policy (no mutation). Decides when to drain steering / followup queues from the host.
- `BudgetStrategy` — sync, pure-policy. Returns iteration limit and optional wall-clock cap.

Trait stubs and one outcome enum (`StopOutcome`) only. `Default*` impls land in WS-5.

## 2. Files

### NEW
- `crates/ironclaw_agent_loop/src/strategies/stop.rs`
- `crates/ironclaw_agent_loop/src/strategies/drain.rs`
- `crates/ironclaw_agent_loop/src/strategies/budget.rs`

### EXTEND
- `crates/ironclaw_agent_loop/src/strategies/mod.rs` — add `pub mod` lines + re-exports

## 3. Specification

### 3.1 `StopConditionStrategy`

```rust
//! crates/ironclaw_agent_loop/src/strategies/stop.rs

use async_trait::async_trait;
use ironclaw_turns::{LoopFailureKind, LoopMessageRef, LoopResultRef};

use crate::state::{ControlStrategyState, LoopExecutionState};

/// Decides whether the loop should stop after the current turn finishes.
///
/// Mutates `control_state` (turns-completed counter, terminate-hint counters,
/// etc.). Async because future strategies may consult host state for
/// milestone tracking.
///
/// The default impl provides the production-safe escapes from master doc §10:
/// terminate-hint stop, repetition-detected stop, and repeated-failure stop.
#[async_trait]
pub trait StopConditionStrategy: Send + Sync {
    /// Called after a turn completes. The turn's outcome (whether it ended
    /// in a Reply or after a CapabilityCalls batch) is communicated via the
    /// `just_completed` summary.
    async fn should_stop_after_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopOutcome;
}

/// Loop-side projection of what just happened in the turn the executor is
/// asking about. **Refs only — never raw content.**
///
/// This is intentional: pi-mono's `shouldStopAfterTurn` receives the actual
/// assistant message because pi has no host abstraction. Reborn's framework
/// stores refs only (per `turns-agent-loop.md` §6 — no raw model output in
/// loop state). A strategy that needs to inspect reply content reads it via
/// the host using `assistant_message_ref` (host applies redaction/scope per
/// the trust-boundary contract).
///
/// In other words: the framework intentionally does NOT pass content into
/// strategy decisions, because doing so would create a second source of truth
/// that bypasses host-side content policy. Strategies that need content read
/// it through the host port; the host decides what's safe to expose.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TurnSummary {
    pub kind: TurnEndKind,
    pub assistant_message_ref: Option<LoopMessageRef>,
    pub batch_result_refs: Vec<LoopResultRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEndKind {
    /// The model returned a Reply; no capability batch was executed this turn.
    ReplyOnly,
    /// The model returned CapabilityCalls; the listed result refs are the
    /// finalized batch outcomes for this turn.
    AfterCapabilityBatch,
}

/// Strategy decision plus the new control_state slot value.
///
/// `Stop` carries a `StopKind` distinguishing graceful completion from a
/// safety-net escape (no-progress, etc.). The executor maps `StopKind` to
/// the appropriate `LoopExit` variant (Completed vs Failed).
///
/// Consumers (the executor) **pattern-match on the variants directly**; this
/// type intentionally does not expose accessor methods like `.kind()` or
/// `.control_state()`. Any pseudocode showing such accessors is shorthand —
/// the implementation uses `match` arms.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StopOutcome {
    Continue { control: ControlStrategyState },
    Stop { control: ControlStrategyState, kind: StopKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopKind {
    /// Strategy is satisfied. Executor → LoopExit::Completed { GracefulStop }.
    GracefulStop,
    /// Safety-net escape fired (master doc §10). Executor → LoopExit::Failed
    /// { reason_kind: NoProgressDetected }.
    NoProgressDetected,
    /// Strategy aborts with an explicit failure kind. Executor →
    /// LoopExit::Failed { reason_kind: <provided> }.
    Aborted(LoopFailureKind),
}
```

### 3.2 `InputDrainStrategy`

```rust
//! crates/ironclaw_agent_loop/src/strategies/drain.rs

use async_trait::async_trait;

use crate::state::LoopExecutionState;

/// Decides when to drain the host's steering / followup queues.
///
/// Pure policy (no mutation). Async because future strategies may consult
/// host state for queue-size hints, priority, etc.
///
/// Two independent decisions per tick:
/// - `drain_steering`: drain the steering queue before the next model call
///   (mid-turn injection — pi's `getSteeringMessages`).
/// - `drain_followup`: drain the followup queue when the loop would
///   otherwise stop (post-natural-stop continuation — pi's
///   `getFollowUpMessages`).
#[async_trait]
pub trait InputDrainStrategy: Send + Sync {
    /// Called by the executor at the start of each tick, BEFORE prompt build.
    async fn drain_steering(&self, state: &LoopExecutionState) -> bool;

    /// Called by the executor after the loop would stop, BEFORE returning
    /// LoopExit::Completed. If true, the executor drains followup messages
    /// and (if any were drained) continues the loop instead.
    async fn drain_followup(&self, state: &LoopExecutionState) -> bool;
}
```

### 3.3 `BudgetStrategy`

```rust
//! crates/ironclaw_agent_loop/src/strategies/budget.rs

use std::time::Duration;

use crate::state::LoopExecutionState;

/// Hard caps on loop execution. Sync, pure policy.
///
/// The executor enforces these AFTER each tick:
/// - if `state.iteration >= iteration_limit(&state)` → `LoopExit::Failed { IterationLimit }`
/// - if `wall_clock_limit(&state)` is `Some(d)` and elapsed > d → `LoopExit::Failed { IterationLimit }`
///
/// Strategies cannot mutate state; this is a read-only policy gate.
pub trait BudgetStrategy: Send + Sync {
    /// Maximum number of iterations before the loop is forcibly failed.
    /// `state` is provided in case the strategy varies its limit by what
    /// the loop has already done (e.g. tighter cap in resume scenarios).
    fn iteration_limit(&self, state: &LoopExecutionState) -> u32;

    /// Optional wall-clock cap. `None` means no time limit.
    fn wall_clock_limit(&self, state: &LoopExecutionState) -> Option<Duration>;
}
```

## 4. Acceptance criteria

- [ ] `cargo check -p ironclaw_agent_loop` passes
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings
- [ ] Unit tests per file:
  - [ ] `stop.rs` — `StopOutcome` round-trips through `serde_json`; `StopKind::Aborted(LoopFailureKind::ModelError)` round-trips with the variant tag intact; object-safety check `fn _check(_: &dyn StopConditionStrategy) {}`
  - [ ] `drain.rs` — object-safety check; serde-test of `TurnSummary` (lives in `stop.rs` but is referenced from drain doc examples)
  - [ ] `budget.rs` — object-safety check; trivial impl returning `(32, None)` exercises the trait surface
- [ ] All three modules visible from `strategies::mod.rs` re-exports

## 5. Out of scope

- `DefaultStopConditionStrategy` — including the no-progress detection logic that reads `state.recent_call_signatures` and `state.recent_failure_kinds` — WS-5
- `DefaultInputDrainStrategy` — WS-5
- `DefaultBudgetStrategy` — WS-5
- The executor body that consults these strategies — WS-6
- Per-strategy state slot growth: skeleton uses what WS-0 defined; if a `Default*` impl needs more fields, that growth lands in WS-5

## 6. Verification command sequence

```bash
cargo check -p ironclaw_agent_loop
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_agent_loop
```

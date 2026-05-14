# WS-5 — Default Strategies

**Workstream:** WS-5
**Crate touched:** `ironclaw_agent_loop`
**Depends on:** WS-1, WS-2, WS-3 (all nine traits exist)
**Parallel with:** WS-4
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §6, §10

---

## 1. Scope

Implement the nine `Default*Strategy` types. Each is a concrete struct whose `Default` impl produces the all-pi-mono behavior the framework documents as the reference baseline. These are what `DefaultPlanner::compose_default()` (WS-4, `pub(crate)`) wires up; the family factory `families::default()` (WS-3.5) is the only public path to the resulting `LoopFamily`.

The strategies must, *together*, deliver the production-safe escape from master doc §10:

- iteration cap of 32 (`DefaultBudgetStrategy`)
- per-error retry budget of 2 (`DefaultRecoveryStrategy`)
- no-progress detection on call-signature ring + failure-kind run-length (`DefaultStopConditionStrategy`)

## 2. Files

### EXTEND
- `crates/ironclaw_agent_loop/src/strategies/context.rs` — add `DefaultContextStrategy`
- `crates/ironclaw_agent_loop/src/strategies/capability.rs` — add `DefaultCapabilityStrategy`
- `crates/ironclaw_agent_loop/src/strategies/model.rs` — add `DefaultModelStrategy`
- `crates/ironclaw_agent_loop/src/strategies/batch.rs` — add `DefaultBatchPolicyStrategy`
- `crates/ironclaw_agent_loop/src/strategies/gate.rs` — add `DefaultGateHandlingStrategy`
- `crates/ironclaw_agent_loop/src/strategies/recovery.rs` — add `DefaultRecoveryStrategy`
- `crates/ironclaw_agent_loop/src/strategies/stop.rs` — add `DefaultStopConditionStrategy`
- `crates/ironclaw_agent_loop/src/strategies/drain.rs` — add `DefaultInputDrainStrategy`
- `crates/ironclaw_agent_loop/src/strategies/budget.rs` — add `DefaultBudgetStrategy`

If `DefaultRecoveryStrategy` needs richer per-error counters than the WS-0 skeleton `RecoveryStrategyState { attempts: u32 }`, this brief grows that slot type. Same applies to `StopStrategyState` for `DefaultStopConditionStrategy` and `GateStrategyState` for `DefaultGateHandlingStrategy` (each stop/gate strategy now owns its own slot — no shared `control_state`). Document any growth in `state.rs` doc-comments.

## 3. Specification

### 3.1 `DefaultContextStrategy`

```rust
#[derive(Debug, Clone, Default)]
pub struct DefaultContextStrategy {
    /// Max messages to ask the host to include in the bundle. Default 16.
    pub max_messages: u32,
}

#[async_trait]
impl ContextStrategy for DefaultContextStrategy {
    async fn plan_context_request(&self, _state: &LoopExecutionState) -> LoopPromptBundleRequest {
        LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(self.max_messages.max(1)),
            inline_messages: Vec::new(),  // no nudges by default
            // … any other fields, all defaults
        }
    }
}

impl DefaultContextStrategy {
    const DEFAULT_MAX_MESSAGES: u32 = 16;
}

// `Default` should set max_messages = 16
```

### 3.2 `DefaultCapabilityStrategy`

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultCapabilityStrategy;

#[async_trait]
impl CapabilityStrategy for DefaultCapabilityStrategy {
    async fn filter(&self, _state: &LoopExecutionState) -> VisibleCapabilityFilter {
        VisibleCapabilityFilter::All  // host applies its own scope/grant filters
    }
}
```

### 3.3 `DefaultModelStrategy`

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultModelStrategy;

#[async_trait]
impl ModelStrategy for DefaultModelStrategy {
    async fn preference(&self, state: &LoopExecutionState) -> ModelPreference {
        match state.model_state.fallback_index {
            0 => ModelPreference::Primary,
            i => ModelPreference::Fallback { index: i },
        }
    }
}
```

In the skeleton `state.model_state.fallback_index` is always 0, so this always returns `Primary`. The `Fallback` arm is wired through for the deferred ModelRouteChain follow-up (master doc §9).

### 3.4 `DefaultBatchPolicyStrategy`

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultBatchPolicyStrategy;

impl BatchPolicyStrategy for DefaultBatchPolicyStrategy {
    fn policy(&self, _state: &LoopExecutionState, calls: &[CapabilityCallSummary]) -> BatchPolicy {
        if calls.iter().any(|c| matches!(c.concurrency_hint, ConcurrencyHint::Exclusive)) {
            BatchPolicy::Sequential
        } else {
            BatchPolicy::Parallel
        }
    }
}
```

### 3.5 `DefaultGateHandlingStrategy`

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultGateHandlingStrategy;

#[async_trait]
impl GateHandlingStrategy for DefaultGateHandlingStrategy {
    async fn handle(&self, state: &LoopExecutionState, _gate: &GateSummary) -> GateOutcome {
        // Default behavior is always to block. Loop families that want
        // skip-and-continue or abort semantics swap this strategy.
        GateOutcome::Block { gate: state.gate_state.clone() }
    }
}
```

### 3.6 `DefaultRecoveryStrategy`

```rust
#[derive(Debug, Clone, Copy)]
pub struct DefaultRecoveryStrategy {
    /// Max retries per error class before giving up. Default 2.
    pub max_attempts_per_class: u32,
}

impl Default for DefaultRecoveryStrategy {
    fn default() -> Self { Self { max_attempts_per_class: 2 } }
}

#[async_trait]
impl RecoveryStrategy for DefaultRecoveryStrategy {
    async fn on_capability_error(
        &self,
        state: &LoopExecutionState,
        err: &CapabilityErrorSummary,
    ) -> RecoveryOutcome {
        let next = state.recovery_state.with_incremented_attempts();
        let kind = capability_error_to_failure_kind(err.class);

        match err.class {
            // PolicyDenied — skip the denied call and let the model try
            // something else. A denial means "this tool isn't authorized
            // for this run" (profile filter, hook deny, runtime auth gap);
            // aborting the whole turn on the first denial is harsh. The
            // no-progress safety net (master doc §10) still catches a
            // stuck model repeatedly issuing denied calls. Families that
            // want stricter behavior override DefaultRecoveryStrategy.
            CapabilityErrorClass::PolicyDenied => {
                RecoveryOutcome::SkipResult { recovery: next }
            }
            // Permanent / input — no retry, no graceful skip; abort.
            CapabilityErrorClass::Permanent
            | CapabilityErrorClass::InputInvalid => {
                RecoveryOutcome::Abort { recovery: next, failure_kind: kind }
            }
            // Transient / unavailable / internal — bounded retry.
            CapabilityErrorClass::Transient
            | CapabilityErrorClass::Unavailable
            | CapabilityErrorClass::Internal => {
                if state.recovery_state.attempts >= self.max_attempts_per_class {
                    RecoveryOutcome::Abort { recovery: next, failure_kind: kind }
                } else {
                    RecoveryOutcome::Retry {
                        recovery: next,
                        alter: Some(RetryAlteration::Backoff {
                            delay: backoff_for(state.recovery_state.attempts),
                        }),
                    }
                }
            }
        }
    }

    async fn on_model_error(
        &self,
        state: &LoopExecutionState,
        err: &ModelErrorSummary,
    ) -> RecoveryOutcome { /* analogous shape */ }
}

fn backoff_for(attempt: u32) -> std::time::Duration {
    // Simple exponential: 250ms × 2^attempt, capped at 5s.
    let ms = 250u64.saturating_mul(1u64 << attempt.min(5));
    std::time::Duration::from_millis(ms.min(5_000))
}
```

`with_incremented_attempts()` is a small helper on `RecoveryStrategyState` (this brief adds it). The mapping `capability_error_to_failure_kind` is straightforward and lives in `recovery.rs`.

### 3.7 `DefaultStopConditionStrategy`

This is the biggest of the defaults — it owns the production-safe escape:

```rust
#[derive(Debug, Clone, Copy)]
pub struct DefaultStopConditionStrategy {
    /// Window size for "same call signature ≥ N times" check. Default: 5 most recent.
    pub repetition_window: usize,
    /// Min repeated count within the window to trigger NoProgressDetected. Default: 3.
    pub repetition_threshold: usize,
    /// Min trailing run length of identical failure kinds to trigger NoProgressDetected. Default: 3.
    pub failure_run_threshold: usize,
}

impl Default for DefaultStopConditionStrategy {
    fn default() -> Self {
        Self {
            repetition_window: 5,
            repetition_threshold: 3,
            failure_run_threshold: 3,
        }
    }
}

#[async_trait]
impl StopConditionStrategy for DefaultStopConditionStrategy {
    async fn should_stop_after_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopOutcome {
        let next = StopStrategyState {
            turns_completed: state.stop_state.turns_completed + 1,
            ..state.stop_state.clone()
        };

        // (a) terminate hint: every result in the just-completed batch said terminate.
        if just_completed.kind == TurnEndKind::AfterCapabilityBatch
            && state.stop_state.last_batch_total > 0
            && state.stop_state.terminate_hints_in_last_batch
                == state.stop_state.last_batch_total
        {
            return StopOutcome::Stop { stop: next, kind: StopKind::GracefulStop };
        }

        // (b) repetition: same call signature ≥ threshold in the last window iterations.
        if state.recent_call_signatures.most_common_count_in(self.repetition_window)
            >= self.repetition_threshold
        {
            return StopOutcome::Stop { stop: next, kind: StopKind::NoProgressDetected };
        }

        // (c) failure run-length: same failure kind ≥ threshold in a row.
        if state.recent_failure_kinds.same_run_length() >= self.failure_run_threshold {
            return StopOutcome::Stop { stop: next, kind: StopKind::NoProgressDetected };
        }

        StopOutcome::Continue { stop: next }
    }
}
```

### 3.8 `DefaultInputDrainStrategy`

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultInputDrainStrategy;

#[async_trait]
impl InputDrainStrategy for DefaultInputDrainStrategy {
    async fn drain_steering(&self, _state: &LoopExecutionState) -> bool { true }    // before every model call
    async fn drain_followup(&self, _state: &LoopExecutionState) -> bool { true }    // when otherwise stopping
}
```

### 3.9 `DefaultBudgetStrategy`

```rust
#[derive(Debug, Clone, Copy)]
pub struct DefaultBudgetStrategy {
    pub iteration_limit: u32,
    pub wall_clock_limit: Option<std::time::Duration>,
}

impl Default for DefaultBudgetStrategy {
    fn default() -> Self { Self { iteration_limit: 32, wall_clock_limit: None } }
}

impl BudgetStrategy for DefaultBudgetStrategy {
    fn iteration_limit(&self, _state: &LoopExecutionState) -> u32 { self.iteration_limit }
    fn wall_clock_limit(&self, _state: &LoopExecutionState) -> Option<std::time::Duration> {
        self.wall_clock_limit
    }
}
```

## 4. Acceptance criteria

- [ ] `cargo check -p ironclaw_agent_loop` passes
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings
- [ ] Unit tests per strategy:
  - [ ] `DefaultContextStrategy::default().max_messages == 16`; `plan_context_request` returns `PromptMode::TextOnly` with `max_messages: Some(16)`, no inline messages
  - [ ] `DefaultCapabilityStrategy.filter(...)` always returns `VisibleCapabilityFilter::All`
  - [ ] `DefaultModelStrategy.preference` returns `Primary` when `fallback_index == 0`, `Fallback { index: 2 }` when `fallback_index == 2`
  - [ ] `DefaultBatchPolicyStrategy.policy` returns `Sequential` when any call has `Exclusive` hint, `Parallel` otherwise; empty batch returns `Parallel`
  - [ ] `DefaultGateHandlingStrategy.handle` always returns `Block` for any kind
  - [ ] `DefaultRecoveryStrategy` aborts on `Permanent`/`InputInvalid` immediately; **skips on `PolicyDenied`** (returns `SkipResult` — denied calls are dropped from the batch and the model is free to try another tool; no-progress detector catches stuck-on-denied loops); retries `Transient`/`Unavailable`/`Internal` up to `max_attempts_per_class` then aborts; backoff increases with attempt
  - [ ] `DefaultStopConditionStrategy`:
    - all-results-terminate-hint case → `Stop { GracefulStop }`
    - same call signature pushed 3× into ring → `Stop { NoProgressDetected }`
    - same failure kind pushed 3× in a row → `Stop { NoProgressDetected }`
    - no signal → `Continue` with `turns_completed += 1`
  - [ ] `DefaultInputDrainStrategy` returns `(true, true)`
  - [ ] `DefaultBudgetStrategy::default()` returns `(32, None)`
- [ ] Doc comments on each `Default*` cite `agent-loop-skeleton.md` §6 (and §10 for the safety-net trio)

## 5. Out of scope

- Any non-default strategy (loop-family-specific) — those land per follow-up loop-family PR
- Wiring `DefaultPlanner::compose_default()` to instantiate these — WS-4 (`pub(crate)`); the public path is `families::default()` in WS-3.5
- The executor that calls these strategies — WS-6
- Backoff respect at the executor (this brief defines the alteration; WS-6 honors it)

## 6. Verification command sequence

```bash
cargo check -p ironclaw_agent_loop
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_agent_loop
```

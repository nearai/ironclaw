# WS-2 — Strategy Traits β: Batch / Gate / Recovery

**Workstream:** WS-2
**Crate touched:** `ironclaw_agent_loop`
**Depends on:** WS-0 (`LoopExecutionState`, slot types, `LoopFailureKind::NoProgressDetected`)
**Parallel with:** WS-1, WS-3
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §6, §8, §9

---

## 1. Scope

Land three strategies that govern how the executor handles capability calls and their failures:

- `BatchPolicyStrategy` — pure policy, returns sequential vs parallel execution mode.
- `GateHandlingStrategy` — mutates `gate_state`; returns `GateOutcome` enum.
- `RecoveryStrategy` — mutates `recovery_state`; returns `RecoveryOutcome` enum.

The first is sync (it touches no async surface). The other two are async (they may consult host state in future loop families).

Trait stubs and outcome enums only. `Default*` impls land in WS-5.

## 2. Files

### NEW
- `crates/ironclaw_agent_loop/src/strategies/batch.rs`
- `crates/ironclaw_agent_loop/src/strategies/gate.rs`
- `crates/ironclaw_agent_loop/src/strategies/recovery.rs`

### EXTEND
- `crates/ironclaw_agent_loop/src/strategies/mod.rs` — add `pub(crate) mod` lines + crate-internal re-exports

## 3. Specification

### 3.1 `BatchPolicyStrategy`

```rust
//! crates/ironclaw_agent_loop/src/strategies/batch.rs

use crate::state::LoopExecutionState;

/// Decides whether a capability batch executes sequentially or in parallel.
///
/// Pure policy and synchronous — does not consult the host. Mutates nothing.
///
/// The host's per-capability concurrency hints (from descriptors) are
/// authoritative for "this specific call must run alone"; this strategy
/// decides the BATCH-level default.
pub(crate) trait BatchPolicyStrategy: Send + Sync {
    fn policy(&self, state: &LoopExecutionState, calls: &[CapabilityCallSummary]) -> BatchPolicy;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BatchPolicy {
    Sequential,
    Parallel,
}

/// Loop-side projection of one entry in a CapabilityCalls batch — name
/// + concurrency hint only. The strategy never sees raw args (per
/// turns-agent-loop.md §6).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CapabilityCallSummary {
    pub(crate) name: ironclaw_host_api::CapabilityId,
    pub(crate) concurrency_hint: ironclaw_turns::run_profile::ConcurrencyHint,
}
```

`ConcurrencyHint` is defined in **`ironclaw_turns::run_profile`** (in `host.rs` alongside the descriptor types) — NOT in this crate. `ironclaw_agent_loop` depends on `ironclaw_turns`, not the reverse, so a type that's read as a field on `CapabilityDescriptorView` (per WS-0) must live in `ironclaw_turns`. Variants: `SafeForParallel` and `Exclusive`. Derivation from `CapabilityDescriptor.effects` happens at the adapter boundary in WS-9 (see WS-9 §3.2a for the per-`EffectKind` mapping). WS-2 only consumes the type for the strategy projection.

The capability identifier is the current host API `CapabilityId`; older references to `CapabilityName` are superseded.

### 3.2 `GateHandlingStrategy`

```rust
//! crates/ironclaw_agent_loop/src/strategies/gate.rs

use async_trait::async_trait;
use ironclaw_turns::LoopFailureKind;

use crate::state::{GateStrategyState, LoopExecutionState};

/// Decides what to do when a capability invocation comes back with a
/// gate (Approval, Auth, or Resource).
///
/// Mutates `gate_state` (e.g. record gate fingerprints for resume).
/// Async because future strategies may consult host state for grant-history
/// or auth-flow lookups.
#[async_trait]
pub(crate) trait GateHandlingStrategy: Send + Sync {
    async fn handle(&self, state: &LoopExecutionState, gate: &GateSummary) -> GateOutcome;
}

/// Loop-side projection of a host capability gate — kind + opaque ref only.
/// The strategy never sees raw input/secrets/auth state (per
/// turns-agent-loop.md §6 + lightweight-agent-loop.md §8).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct GateSummary {
    pub(crate) kind: GateKind,
    pub(crate) gate_ref: ironclaw_turns::LoopGateRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GateKind {
    Approval,
    Auth,
    Resource,
}

/// Strategy decision for a gate, plus the new gate_state slot value.
///
/// Variants:
/// - `Block` — the executor checkpoints (BeforeBlock) and returns
///   `LoopExit::Blocked`. The standard production path.
/// - `SkipAndContinue` — drop this call's result entirely and proceed with
///   the rest of the batch. Use sparingly; intended for fire-and-forget
///   tools where a missing approval is non-fatal.
/// - `Abort` — return `LoopExit::Failed { reason_kind: failure_kind }`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum GateOutcome {
    Block { gate: GateStrategyState },
    SkipAndContinue { gate: GateStrategyState },
    Abort { gate: GateStrategyState, failure_kind: LoopFailureKind },
}

impl GateOutcome {
    /// Executor-side contract check before honoring the outcome.
    /// Approval gates must not be silently skipped.
    pub(crate) fn validate_for_gate_kind(&self, kind: GateKind) -> Result<(), LoopFailureKind> {
        match (kind, self) {
            (GateKind::Approval, GateOutcome::SkipAndContinue { .. }) => {
                Err(LoopFailureKind::DriverBug)
            }
            _ => Ok(()),
        }
    }
}
```

### 3.3 `RecoveryStrategy`

```rust
//! crates/ironclaw_agent_loop/src/strategies/recovery.rs

use async_trait::async_trait;
use ironclaw_turns::{LoopDiagnosticRef, LoopFailureKind, run_profile::LoopSafeSummary};

use crate::state::{LoopExecutionState, RecoveryStrategyState};

/// Decides what to do when a capability call OR a model call fails with a
/// (sanitized) error summary.
///
/// Mutates `recovery_state` (attempt counters, fallback advance bookkeeping).
/// Async because future strategies may consult host state for circuit-breaker
/// counters, route health, etc.
#[async_trait]
pub(crate) trait RecoveryStrategy: Send + Sync {
    async fn on_capability_error(
        &self,
        state: &LoopExecutionState,
        err: &CapabilityErrorSummary,
    ) -> RecoveryOutcome;

    async fn on_model_error(
        &self,
        state: &LoopExecutionState,
        err: &ModelErrorSummary,
    ) -> RecoveryOutcome;
}

/// Sanitized capability error — class + safe summary string + opaque diagnostic
/// ref. Strategies never see raw provider errors, host paths, or secrets
/// (sanitization happens at the host port boundary, per master doc §9).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct SanitizedStrategySummary(String);

impl SanitizedStrategySummary {
    pub(crate) fn new(summary: impl Into<String>) -> Result<Self, String> {
        let summary = summary.into();
        LoopSafeSummary::new(summary.clone()).map(|_| Self(summary))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> serde::Deserialize<'de> for SanitizedStrategySummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let summary = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(summary).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CapabilityErrorSummary {
    pub(crate) class: CapabilityErrorClass,
    pub(crate) safe_summary: SanitizedStrategySummary,
    pub(crate) diagnostic_ref: Option<LoopDiagnosticRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityErrorClass {
    Transient,
    Permanent,
    InputInvalid,
    PolicyDenied,
    Unavailable,
    Internal,
}

/// Sanitized model error — class + safe summary + opaque diagnostic ref.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ModelErrorSummary {
    pub(crate) class: ModelErrorClass,
    pub(crate) safe_summary: SanitizedStrategySummary,
    pub(crate) diagnostic_ref: Option<LoopDiagnosticRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelErrorClass {
    Transient,
    ContextOverflow,
    ContentFiltered,
    Unavailable,
    Internal,
}

/// Strategy decision plus the new recovery_state slot value.
///
/// Variants:
/// - `Retry` — re-issue; `scope` states whether the executor should retry
///   the failed call only or the current loop iteration, and `alter` carries
///   the strategy's prompt/model hint.
/// - `SkipResult` — drop this result and continue the batch.
/// - `Abort` — return LoopExit::Failed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum RecoveryOutcome {
    Retry {
        recovery: RecoveryStrategyState,
        scope: RetryScope,
        alter: Option<RetryAlteration>,
    },
    SkipResult { recovery: RecoveryStrategyState },
    Abort { recovery: RecoveryStrategyState, failure_kind: LoopFailureKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RetryScope {
    Call,
    Iteration,
}

/// Strategy hint about WHAT to alter on retry. Skeleton supports prompt-shape
/// alterations only; model-route swap is reserved for the deferred
/// ModelRouteChain follow-up (master doc §9).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum RetryAlteration {
    /// Shrink context for the next attempt (e.g. on context-overflow).
    ShrinkContext { drop_messages: u32 },
    /// Backoff before retry (executor honors as a sleep).
    Backoff { delay_ms: BackoffDelayMs },
    /// Reserved for future ModelRouteChain landing. Skeleton executor MUST
    /// reject this alteration with `LoopFailureKind::DriverBug` until the
    /// chain mechanism lands.
    AdvanceFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BackoffDelayMs(u64);

impl BackoffDelayMs {
    pub(crate) const MAX_DELAY_MS: u64 = 60_000;

    pub(crate) fn new(delay_ms: u64) -> Result<Self, String> {
        if delay_ms <= Self::MAX_DELAY_MS {
            Ok(Self(delay_ms))
        } else {
            Err(format!(
                "backoff delay {delay_ms}ms exceeds max {}ms",
                Self::MAX_DELAY_MS
            ))
        }
    }

    pub(crate) fn as_u64(self) -> u64 {
        self.0
    }
}
```

## 4. Acceptance criteria

- [ ] `cargo check -p ironclaw_agent_loop` passes with the three new modules wired into `strategies/mod.rs`
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings
- [ ] Unit tests per file:
  - [ ] `batch.rs` — `BatchPolicy` round-trips through `serde_json` with snake_case; `ironclaw_turns::run_profile::ConcurrencyHint` round-trip test lives in `ironclaw_turns` (WS-0 owns it; this brief only consumes the type)
  - [ ] `gate.rs` — `GateOutcome` round-trips; approval gates reject `SkipAndContinue` through `validate_for_gate_kind`; object-safety check `fn _check(_: &dyn GateHandlingStrategy) {}`
  - [ ] `recovery.rs` — `RecoveryOutcome` round-trips; `RetryScope`, `SanitizedStrategySummary`, `BackoffDelayMs`, and both `RetryAlteration::ShrinkContext` and `Backoff` round-trip; object-safety check
  - [ ] one test per outcome enum confirms that `Retry`/`SkipResult`/`Abort` carry the new strategy slot value (the field is named correctly and is the right type)
- [ ] Doc comments cite the master doc and the relevant contract section

## 5. Out of scope

- `DefaultBatchPolicyStrategy`, `DefaultGateHandlingStrategy`, `DefaultRecoveryStrategy` — WS-5
- The executor body that calls these strategies and applies outcomes — WS-6
- The actual production of `RetryAlteration::AdvanceFallback` — deferred until `ModelRouteChain` follow-up
- Sanitization at the host port (already in place per the existing host-port surface)

## 6. Verification command sequence

```bash
cargo check -p ironclaw_agent_loop
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_agent_loop
```

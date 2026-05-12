# WS-0 — State and Checkpoints

**Workstream:** WS-0 (foundation — blocks WS-1, WS-2, WS-3, WS-7)
**Crates touched:** `ironclaw_agent_loop` (NEW), `ironclaw_turns`
**Depends on:** —
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §5–§7, §10

---

## 1. Scope

Land the foundation everything else stands on:

- The new crate `ironclaw_agent_loop` with `Cargo.toml`, `lib.rs`, `CLAUDE.md` guardrail, workspace registration.
- `LoopExecutionState` (immutable value type) with all universal fields, executor-observed fields, and per-strategy state slots.
- `BoundedRing<T, N>` and `CapabilityCallSignature` helper types.
- The checkpoint payload schema id `reborn:default-loop-v1` (reserved string constant; producer wiring deferred).
- `CheckpointMarker` aggregate held in state.
- One new variant on `LoopFailureKind` in `ironclaw_turns::loop_exit`: `NoProgressDetected`.

The per-strategy state slot *types* (`ContextStrategyState`, `RecoveryStrategyState`, etc.) land here as empty unit structs (or with whatever skeleton fields are obviously needed: `RecoveryStrategyState { attempts: u32 }`, `ModelStrategyState { fallback_index: u32 }`). Strategy traits and outcome enums that read or update them land in WS-1/2/3.

## 2. Files

### NEW
- `crates/ironclaw_agent_loop/Cargo.toml` — depends on `ironclaw_turns`, `serde`, `serde_json`, `thiserror`, `async-trait`
- `crates/ironclaw_agent_loop/CLAUDE.md` — guardrail (see §6 below)
- `crates/ironclaw_agent_loop/src/lib.rs` — module declarations + crate-level docs pointing at master spec
- `crates/ironclaw_agent_loop/src/state.rs` — everything in §3
- `crates/ironclaw_agent_loop/src/state/bounded_ring.rs` (or inline) — `BoundedRing<T, N>`
- `crates/ironclaw_agent_loop/src/state/signature.rs` (or inline) — `CapabilityCallSignature`
- `crates/ironclaw_agent_loop/src/state/slots.rs` (or inline) — per-strategy state slot types

### EXTEND
- `crates/ironclaw_turns/src/loop_exit.rs` — add `LoopFailureKind::NoProgressDetected` variant
- `crates/ironclaw_turns/src/run_profile/host.rs` — extend `LoopPromptBundleRequest` with `inline_messages: Vec<LoopInlineMessage>` (default empty); extend `LoopCheckpointPort` trait with `load_checkpoint_payload(checkpoint_id: TurnCheckpointId) -> Vec<u8>` method (default impl can return `Err(AgentLoopHostError::Unavailable)` until WS-10 wires a real backing store; existing `TextOnlyModelReplyDriver` is unaffected because it never calls resume)
- `crates/ironclaw_turns/CLAUDE.md` — append amendment paragraph (see §6 below)
- `Cargo.toml` (workspace) — add `crates/ironclaw_agent_loop` to members
- `crates/ironclaw_agent_loop/src/state.rs` re-exports `LoopFailureKind` from `ironclaw_turns` for ergonomics

### NOT TOUCHED in this brief
- Strategy traits — WS-1/2/3
- `DefaultPlanner` — WS-4
- Executor — WS-6
- Driver adapter — WS-7
- `ModelRouteChain` (deferred — see master doc §9)

## 3. Specification

### 3.1 `LoopExecutionState`

```rust
//! crates/ironclaw_agent_loop/src/state.rs

use ironclaw_turns::{
    LoopFailureKind, LoopGateRef, LoopMessageRef, LoopResultRef,
    run_profile::{LoopInputCursor, VisibleSurfaceVersion},
};

/// Immutable execution state threaded through the loop.
///
/// The executor rebinds its local `let mut state` each tick to the next whole
/// state. Strategies receive `&LoopExecutionState` and return outcome enums
/// that carry the new value of their own slot. The executor builds the next
/// whole state by swapping that slot.
///
/// See `docs/reborn/agent-loop-skeleton.md` §5–§7 for the full mutability
/// model and rationale.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LoopExecutionState {
    // executor-universal
    pub iteration: u32,
    pub last_checkpoint: Option<CheckpointMarker>,
    pub assistant_refs: Vec<LoopMessageRef>,
    pub result_refs: Vec<LoopResultRef>,
    pub last_gate: Option<LoopGateRef>,
    pub input_cursor: LoopInputCursor,
    pub surface_version: Option<VisibleSurfaceVersion>,

    // executor-observed (populated by executor; read-only to strategies)
    pub recent_call_signatures: BoundedRing<CapabilityCallSignature, 8>,
    pub recent_failure_kinds: BoundedRing<LoopFailureKind, 8>,

    // strategy slots
    pub context_state: ContextStrategyState,
    pub capability_state: CapabilityStrategyState,
    pub model_state: ModelStrategyState,
    pub recovery_state: RecoveryStrategyState,
    pub control_state: ControlStrategyState,
}

impl LoopExecutionState {
    /// Builds the initial state at the start of a fresh run.
    pub fn initial() -> Self { /* default everything to zero / empty */ }

    /// Rehydrates state from a checkpoint payload. Schema validation lives
    /// here (verify schema_id matches CHECKPOINT_SCHEMA_ID).
    pub fn from_checkpoint_payload(
        payload: &serde_json::Value,
    ) -> Result<Self, CheckpointPayloadError>;
}
```

### 3.2 `CheckpointMarker` and schema constant

```rust
/// Records the most recent checkpoint the executor took, for resume coordination.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckpointMarker {
    pub kind: CheckpointKind,
    pub iteration_at_checkpoint: u32,
}

/// Mirrors the four checkpoint boundaries from the executor (master doc §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointKind {
    BeforeModel,
    BeforeSideEffect,
    BeforeBlock,
    Final,
}

/// Reserved identifier for the default-loop checkpoint payload schema.
/// The producer (executor) and consumer (resume path) both reference this
/// constant. Bumping the version is a breaking checkpoint-format change.
pub const CHECKPOINT_SCHEMA_ID: &str = "reborn:default-loop-v1";
```

### 3.3 `BoundedRing<T, N>`

```rust
/// Fixed-capacity ring buffer. Drops oldest at capacity. Used for
/// repetition / no-progress detection in DefaultStopConditionStrategy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BoundedRing<T, const N: usize> {
    items: Vec<T>,           // length always <= N; oldest at index 0
}

impl<T: Clone + Eq + std::hash::Hash, const N: usize> BoundedRing<T, N> {
    pub fn new() -> Self { Self { items: Vec::with_capacity(N) } }

    pub fn push(&mut self, item: T);

    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn iter(&self) -> impl Iterator<Item = &T>;

    /// Count of the most-frequently-occurring item in the last `window` entries.
    /// Window is clamped to `len()`.
    pub fn most_common_count_in(&self, window: usize) -> usize;

    /// Length of the trailing run of identical items (always >= 1 when non-empty).
    pub fn same_run_length(&self) -> usize;
}

impl<T, const N: usize> Default for BoundedRing<T, N> {
    fn default() -> Self { Self { items: Vec::with_capacity(N) } }
}
```

Note: `N` is a const-generic; tests should cover `N = 1`, `N = 8` (the production size), and capacity rollover.

### 3.4 `CapabilityCallSignature`

```rust
use ironclaw_turns::run_profile::CapabilityName;  // exact import TBD; use the existing newtype

/// Stable identity for a capability call, suitable for repetition detection
/// without retaining raw arguments (per turns-agent-loop.md §6: no raw tool
/// input in loop state).
///
/// Constructed by the executor via `from_call(...)` which canonicalizes
/// the JSON args before hashing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CapabilityCallSignature {
    pub name: CapabilityName,
    pub args_hash: ArgsHash,    // 64-bit blake3 / xxhash; do not expose raw args
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ArgsHash(pub u64);

impl CapabilityCallSignature {
    /// Builds a signature from a capability name and JSON args.
    /// Args are canonicalized (sort object keys, normalize numbers) before hashing.
    pub fn from_call(name: CapabilityName, args: &serde_json::Value) -> Self;
}
```

#### Per-iteration push semantics for `recent_call_signatures`

The repetition-escape heuristic in master doc §10 is phrased in terms of *iterations*, not individual calls. To keep that semantics, the executor pushes signatures into `recent_call_signatures` with one-entry-per-iteration deduplication:

- **Push only on the first occurrence of a signature within an iteration.** If a single batch contains three `file_read` calls with identical args, exactly one signature is pushed for that iteration — not three. This prevents a single batch from spuriously tripping no-progress detection.
- **Always push at least one signature per iteration that issues capability calls.** If a batch contains multiple distinct signatures, each gets pushed once; the order matches the batch source order.
- **Retries do not push.** When `RecoveryStrategy::Retry` re-issues the same call (per WS-6 §3.3), the retried invocation does NOT push a new signature — the original push from the initial batch already represents this iteration's attempt.

Implementation guidance: the executor maintains a small `HashSet<CapabilityCallSignature>` scoped to the current iteration, drained at iteration boundaries. `BoundedRing::push` is called once per `(iteration, signature)` tuple that wasn't already present in the per-iteration set. This keeps the data structure simple (`BoundedRing<CapabilityCallSignature, 8>` stays as-is) while honoring the documented "≥3 in the last 5 *iterations*" semantics rather than "≥3 calls."

Tests in WS-8 explicitly cover both shapes: a single batch with three identical calls must NOT trip the detector; three iterations each issuing the same call once MUST trip.

### 3.5 Per-strategy state slots

Each is a small `#[derive(Default)]` struct. Skeleton fields:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContextStrategyState {
    // skeleton: empty. WS-1 may add fields when ContextStrategy needs them.
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityStrategyState {
    // skeleton: empty.
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelStrategyState {
    /// Index into the (deferred) model route fallback chain.
    /// In the skeleton, always 0. Reserved for the follow-up PR that introduces
    /// ModelRouteChain (see master doc §9).
    pub fallback_index: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryStrategyState {
    /// Per-error-class attempt counter. WS-2 may grow this into a
    /// HashMap<LoopFailureKind, u32> when DefaultRecoveryStrategy needs it.
    pub attempts: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ControlStrategyState {
    /// Number of completed turns the StopConditionStrategy has observed.
    pub turns_completed: u32,
    /// Count of `terminate: true` hints seen in the most recent capability batch.
    /// Reset to 0 at the start of each batch.
    pub terminate_hints_in_last_batch: u32,
    /// Total number of results in the most recent capability batch (denominator
    /// for "all results said terminate").
    pub last_batch_total: u32,
}
```

### 3.6 `LoopFailureKind::NoProgressDetected`

```rust
//! crates/ironclaw_turns/src/loop_exit.rs (extend the existing enum)

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopFailureKind {
    ModelError,
    ContextBuildFailed,
    CapabilityProtocolError,
    IterationLimit,
    InvalidModelOutput,
    CheckpointRejected,
    TranscriptWriteFailed,
    DriverBug,
    InterruptedUnexpectedly,
    /// NEW (WS-0): emitted by DefaultStopConditionStrategy when repetition or
    /// repeated-same-error escapes fire. See agent-loop-skeleton.md §10.
    NoProgressDetected,
}
```

Existing `loop_failure_kind_name` helpers (and any sites that match exhaustively) need the new arm. The pre-existing `text_loop_driver` test for sanitization should continue to pass unchanged.

### 3.7 `CheckpointPayloadError`

```rust
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CheckpointPayloadError {
    #[error("checkpoint payload schema id mismatch: expected `{expected}`, got `{actual}`")]
    SchemaMismatch { expected: String, actual: String },
    #[error("checkpoint payload missing required field `{field}`")]
    MissingField { field: &'static str },
    #[error("checkpoint payload field `{field}` failed validation: {reason}")]
    InvalidField { field: &'static str, reason: String },
}
```

## 4. Acceptance criteria

- [ ] `cargo check -p ironclaw_agent_loop` passes
- [ ] `cargo check -p ironclaw_turns` passes after `LoopFailureKind::NoProgressDetected` lands
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings (workspace standard per `CLAUDE.md`)
- [ ] `cargo test -p ironclaw_agent_loop` — unit tests cover:
  - [ ] `BoundedRing::push` rolls over at capacity
  - [ ] `BoundedRing::most_common_count_in` returns correct counts at window < len, window == len, window > len
  - [ ] `BoundedRing::same_run_length` returns 0 for empty, 1 for distinct trailing items, N for trailing run of N
  - [ ] `CapabilityCallSignature::from_call` is stable under JSON key reordering
  - [ ] `LoopExecutionState::initial()` produces value-equal results across calls
  - [ ] `LoopExecutionState` round-trips through `serde_json` (serialize → deserialize → equal)
  - [ ] `LoopExecutionState::from_checkpoint_payload` rejects mismatched schema ids with `SchemaMismatch`
- [ ] `cargo test -p ironclaw_turns` — existing tests pass; new test asserts `LoopFailureKind::NoProgressDetected` serializes as `"no_progress_detected"`
- [ ] No `unwrap()` / `expect()` / `unwrap_or_default()` on Result types in production code (per `error-handling.md`)
- [ ] No raw provider/secret/host-path strings appear in any state field, error message, or doc

## 5. Out of scope (other briefs handle)

- `ContextStrategy`, `CapabilityStrategy`, `ModelStrategy` traits — WS-1
- `BatchPolicyStrategy`, `GateHandlingStrategy`, `RecoveryStrategy` traits — WS-2
- `StopConditionStrategy`, `InputDrainStrategy`, `BudgetStrategy` traits — WS-3
- `AgentLoopPlanner` facade — WS-4
- `Default*Strategy` impls — WS-5
- `AgentLoopExecutor` body that *populates* `recent_call_signatures` and `recent_failure_kinds` — WS-6
- `PlannedDriver` adapter — WS-7
- `ModelRouteChain` and any storage-layer migration — deferred (see master doc §9)
- Checkpoint payload *backing store* (`LoopCheckpointStore` impls) — out of skeleton scope

## 6. Crate guardrails

### 6.1 `crates/ironclaw_turns/CLAUDE.md` — amendment to append

Append the following paragraph to the existing guardrail file (after its current bullet list):

```markdown
- New loop-framework concerns extend this crate carefully:
  - `LoopFailureKind` gains framework variants (currently: `NoProgressDetected`, added by WS-0).
  - `LoopXxxPort` traits are extended by follow-up workstreams (WS-10 adds
    `load_checkpoint_payload` to `LoopCheckpointPort`; WS-13 adds the cancellation
    accessor to `AgentLoopDriverHost`). Trait extensions live here; impls live in
    `ironclaw_loop_support` (host-runtime adapters) or `ironclaw_reborn` (driver-side
    integration). See `docs/reborn/agent-loop-skeleton.md` §3 + §12.
  - `LoopPromptBundleRequest` gains `inline_messages: Vec<LoopInlineMessage>` to
    support nudge-style mid-loop injections produced by `ContextStrategy`
    implementations in the framework crate.
```

### 6.2 `crates/ironclaw_agent_loop/CLAUDE.md` — new file

Suggested content:

```markdown
# ironclaw_agent_loop guardrails

- Owns "what an agent loop is": strategy traits, the `AgentLoopPlanner` facade,
  the `AgentLoopExecutor` trait + canonical impl, and `LoopExecutionState`.
- Stays one layer above `ironclaw_turns` (which owns runner-facing turn
  contracts). Depends on `ironclaw_turns` for `LoopRunContext`, `LoopExit`,
  `LoopXxxPort` traits, and ref types.
- Does NOT depend on `ironclaw_reborn`. The framework crate has no knowledge
  of `AgentLoopDriver`; that bridge lives in `PlannedDriver` in
  `ironclaw_reborn`.
- Stores refs, cursors, counters, versions, and safe summaries only. Never
  raw prompts, raw model output, raw tool input, secrets, host paths, provider
  errors, or stack traces in `LoopExecutionState` or any strategy slot.
- Strategies are `&self`-only; `LoopExecutionState` is value-immutable. All
  mutation happens by the executor swapping a strategy's returned slot into
  the next whole state. There is no `&mut LoopExecutionState` API.
- New strategies, slots, and outcome enums must land typed (no string keys,
  no `serde_json::Value` interior in long-lived state). Per
  `.claude/rules/types.md`.
- Master spec: `docs/reborn/agent-loop-skeleton.md`. Workstream briefs:
  `docs/reborn/agent-loop-briefs/`.
```

## 7. Verification command sequence

```bash
cargo check -p ironclaw_agent_loop
cargo check -p ironclaw_turns
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_agent_loop
cargo test -p ironclaw_turns
```

All five must succeed.

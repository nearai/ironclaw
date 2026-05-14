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
- Two new variants on `LoopFailureKind` in `ironclaw_turns::loop_exit`: `NoProgressDetected` and `PolicyDenied` (the latter for hook/policy-induced denials per master doc §9.1's hooks-as-middleware design).
- Per-strategy state slots: `ContextStrategyState`, `CapabilityStrategyState`, `ModelStrategyState`, `RecoveryStrategyState`, **`StopStrategyState`**, **`GateStrategyState`**. Stop and Gate each own their own slot — there is no shared `ControlStrategyState`.

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
- `crates/ironclaw_turns/src/loop_exit.rs` — add `LoopFailureKind::NoProgressDetected` + `LoopFailureKind::PolicyDenied` variants (see §3.6)
- `crates/ironclaw_turns/src/run_profile/host.rs`:
  - Extend `LoopPromptBundleRequest` with `inline_messages: Vec<LoopInlineMessage>` (default empty).
  - **Read-side `load_checkpoint_payload` is owned by WS-10**, not WS-0. WS-0 does NOT pre-declare a stub signature here — that would drift from WS-10's actual `(LoadCheckpointPayloadRequest { checkpoint_id, expected_schema_id, expected_schema_version }) -> LoadedCheckpointPayload` shape. WS-10 is the source of truth; see [`checkpoint-store-and-resume.md`](checkpoint-store-and-resume.md) §3.1.
  - **NEW:** Add `stage_checkpoint_payload(StageCheckpointPayloadRequest { schema_id, payload }) -> LoopCheckpointStateRef` method on **`LoopCheckpointPort`** (alongside the write-side `checkpoint(...)` already there). `AgentLoopDriverHost` is a method-less marker trait with a blanket impl over its port supertraits — methods cannot live directly on it; they must live on a port that the host implements. Concrete impl in `ironclaw_loop_support` wraps the existing `CheckpointStateStore::put_checkpoint_state`. The executor's `checkpoint(...)` helper (WS-6 §3.4) calls `host.stage_checkpoint_payload(...)` (resolves through `LoopCheckpointPort` via `AgentLoopDriverHost`'s deref-through-supertrait blanket impl) to obtain a validated `state_ref` before invoking `LoopCheckpointPort::checkpoint(LoopCheckpointRequest { kind, state_ref })`. Two-step write keeps byte storage and metadata-write responsibilities cleanly split. **Sibling note**: WS-10 adds the read-side `load_checkpoint_payload(...)` to the same port — both write-stage and read-load live on `LoopCheckpointPort`.
  - **NEW:** Make `LoopContextMessage.message_ref` `Option<LoopMessageRef>` (was required `LoopMessageRef`). `None` means "summary-only entry; prompt port MUST NOT resolve content — use `safe_summary` verbatim instead." Mirrors the `SkillTrustLevel::Installed` carrying `prompt_content: None` pattern. Nine existing call-sites in `crates/ironclaw_*/src` update to wrap writes in `Some(...)` and pattern-match on reads. See [`prompt-context-assembly.md`](prompt-context-assembly.md) §3.2 for the upstream invariant this enforces.
  - **NEW:** Define `ConcurrencyHint` enum **in `ironclaw_turns`** (in `host.rs` alongside the descriptor types — NOT in `ironclaw_agent_loop` per `ironclaw_turns`'s no-downward-dependency rule). Variants: `SafeForParallel` and `Exclusive`. Add `concurrency_hint: ConcurrencyHint` field to `CapabilityDescriptorView`. WS-2 imports `ConcurrencyHint` from `ironclaw_turns` rather than defining its own. The hint is derived at the adapter boundary in WS-9 (`HostRuntimeLoopCapabilityPort::visible_capabilities`) from the underlying `CapabilityDescriptor.effects` Vec — see WS-9 §3.2a for the per-`EffectKind` mapping table. Lower-layer `CapabilityDescriptor` is NOT modified; `effects` remains the source of truth and the hint is a computed projection. Resolves the missing-method bug in WS-6 §3.3b's `summary_of(...)` call site. **Existing struct-literal constructors of `CapabilityDescriptorView`** (in `crates/ironclaw_loop_support/src/`, in `crates/ironclaw_turns/tests/agent_loop_host_contract.rs`, and in any other call site) update in the same PR — this is a breaking field-add, not a true additive change.
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
    run_profile::{LoopInputCursor, LoopRunContext, VisibleSurfaceVersion},
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

    // strategy slots — one per strategy that mutates state. Stop and Gate
    // each own their own slot (no shared `control_state`) so a family's
    // future growth in either dimension can't accidentally mix concerns
    // through a shared struct.
    pub context_state: ContextStrategyState,
    pub capability_state: CapabilityStrategyState,
    pub model_state: ModelStrategyState,
    pub recovery_state: RecoveryStrategyState,
    pub stop_state: StopStrategyState,
    pub gate_state: GateStrategyState,
}

impl LoopExecutionState {
    /// Builds the initial state at the start of a fresh run.
    pub fn initial(run_context: &LoopRunContext) -> Self {
        Self {
            input_cursor: LoopInputCursor::origin_for_run(run_context),
            /* default everything else to zero / empty */
        }
    }

    /// Rehydrates state from a checkpoint payload's bytes. The bytes come
    /// from `LoopCheckpointPort::load_checkpoint_payload(...)` (defined in
    /// WS-10) — checkpoint storage is byte-oriented, not `Value`-oriented.
    /// Schema validation lives here (verify schema_id matches
    /// `CHECKPOINT_SCHEMA_ID`); the `kind` arg is the
    /// `CheckpointKind` recorded in the loaded payload metadata, used
    /// to authenticate the boundary the checkpoint was taken at.
    pub fn from_checkpoint_payload(
        payload: &[u8],
        kind: CheckpointKind,
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
    /// Args are JCS-canonicalized (RFC 8785) before hashing — see §3.4a.
    pub fn from_call(name: CapabilityName, args: &serde_json::Value) -> Self;
}
```

### 3.4a Canonicalization for `ArgsHash` (JCS RFC 8785)

`CapabilityCallSignature::from_call` hashes args using a canonical JSON byte
sequence so that the same logical args always produce the same `ArgsHash`,
regardless of how the upstream model provider serializes the JSON. **The
canonicalization scheme is [JCS RFC 8785](https://datatracker.ietf.org/doc/html/rfc8785)** — the formal IETF spec for JSON canonicalization.

**Implementation:** the [`jcs`](https://crates.io/crates/jcs) crate. Add as a
dependency in `crates/ironclaw_agent_loop/Cargo.toml`:

```toml
[dependencies]
jcs = "0.x"   # pin a maintained release at PR time
```

**Rules implementers must honor** (spelled out so reading RFC 8785 isn't required):

1. **Sort object keys by UTF-16 code-unit order.** Not byte order, not
   lexicographic ASCII order. The `jcs` crate does this correctly; rolling
   a hand impl is a hazard.
2. **Reject `NaN` and `±Infinity`.** Neither is valid JSON. If one of these
   reaches `from_call`, it's a host-port bug upstream (some provider's tool
   call serializer emitted invalid JSON that `serde_json::Value` happened to
   accept). The implementation MUST return an error from `from_call` rather
   than fabricating a hash that no other implementation could reproduce.
3. **Preserve number representation.** Don't normalize `1.0` to `1` or vice
   versa. `serde_json::Value::Number` already distinguishes integers from
   floats; JCS canonicalization respects that distinction.
4. **Minimal whitespace.** No padding between tokens.

**Cross-model compatibility:** for typical tool-call args (strings, integers,
nested objects without floats), JCS output is byte-identical to the Hermes /
Forge sorted-keys-minimal-whitespace convention used in the open-weights
tool-calling ecosystem (Llama 3.1+, Qwen, DeepSeek, Mistral via `<tool_call>`
ChatML). Replay across model swaps (Claude ↔ Hermes 3 ↔ Llama-tool-call
format) hashes identically for typical args. The float-representation edge
case (rule 3 above) is the only divergence point with Hermes-minimal, and
production tool args essentially never carry floats.

**`ArgsHash` algorithm choice:** the `args_hash: ArgsHash(u64)` field uses a
64-bit hash over the JCS-canonicalized bytes. Implementer can pick `blake3`
truncated to 64 bits, `xxhash3_64`, or another stable 64-bit non-cryptographic
hash. The choice is fixed per release (changing the hash function across
releases invalidates all in-flight checkpoint `recent_call_signatures` —
treat as a checkpoint-schema break and bump `CHECKPOINT_SCHEMA_ID` accordingly).

WS-8's integration suite (see [`e2e-integration-tests.md`](e2e-integration-tests.md))
includes one test asserting `ArgsHash` stability across JCS-equivalent JSON
inputs (pretty-printed, minified, key-reordered).

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

/// Persistent state owned by `StopConditionStrategy`. Split from a previously
/// shared `ControlStrategyState` so Stop and Gate evolve independently — a
/// future family's growth in stop-condition state cannot perturb gate-handler
/// invariants and vice versa.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StopStrategyState {
    /// Number of completed turns the StopConditionStrategy has observed.
    pub turns_completed: u32,
    /// Count of `terminate: true` hints seen in the most recent capability batch.
    /// Reset to 0 at the start of each batch.
    pub terminate_hints_in_last_batch: u32,
    /// Total number of results in the most recent capability batch (denominator
    /// for "all results said terminate").
    pub last_batch_total: u32,
}

/// Persistent state owned by `GateHandlingStrategy`. Empty in the skeleton;
/// future families may track gate fingerprints (for resume correlation),
/// per-gate-kind counters, or other gate-relevant bookkeeping here without
/// touching Stop-strategy state.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GateStrategyState {
    // skeleton: empty. WS-2 may extend when DefaultGateHandlingStrategy needs it.
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
    /// NEW (WS-0): emitted when a `CapabilityOutcome::Denied` reaches the
    /// recovery path with no further retry possible. Distinct from
    /// `CapabilityProtocolError` so the no-progress detector can count
    /// repeated denials without conflating them with transport faults.
    /// Hook-induced denials (via the middleware composition seam — see
    /// master doc §9.1 scenario A) accumulate through this
    /// variant. See agent-loop-skeleton.md §9, §10.
    PolicyDenied,
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
  - [ ] `CapabilityCallSignature::from_call` is **JCS-stable** (RFC 8785) — produces the same `ArgsHash` for any two `serde_json::Value` instances that are JCS-equivalent (reordered object keys, equivalent whitespace, equivalent number representation). Cover at minimum: key reordering, pretty-printed vs minified inputs, nested objects with shuffled keys at multiple depths
  - [ ] `CapabilityCallSignature::from_call` returns an error (does not panic, does not silently hash) on `serde_json::Value::Number` instances that are NaN or Infinity (per §3.4a rule 2)
  - [ ] `LoopExecutionState::initial(&run_context)` seeds
    `input_cursor == LoopInputCursor::origin_for_run(&run_context)` and
    produces value-equal results across calls for the same context
  - [ ] `LoopExecutionState` round-trips through `serde_json` (serialize → deserialize → equal)
  - [ ] `LoopExecutionState::from_checkpoint_payload` rejects mismatched schema ids with `SchemaMismatch`
- [ ] `cargo test -p ironclaw_turns` — existing tests pass; new tests assert `LoopFailureKind::NoProgressDetected` serializes as `"no_progress_detected"` and `LoopFailureKind::PolicyDenied` serializes as `"policy_denied"`
- [ ] `StopStrategyState::default()` and `GateStrategyState::default()` round-trip through `serde_json`; no `control_state` field appears on `LoopExecutionState` (grep test)
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

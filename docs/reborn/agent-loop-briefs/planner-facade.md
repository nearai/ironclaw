# WS-4 — Planner Facade

**Workstream:** WS-4
**Crate touched:** `ironclaw_agent_loop`
**Depends on:** WS-1, WS-2, WS-3 (all nine strategy traits exist)
**Parallel with:** WS-5
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §3, §6

---

## 1. Scope

Land the composition layer that ties the nine strategies into a single thing the executor calls:

- `AgentLoopPlanner` trait — a facade with one `&dyn …Strategy` accessor per strategy.
- `PlannerId` newtype — used in checkpoint payload metadata for resume validation. No richer descriptor.
- `DefaultPlanner` struct — owns nine `Arc<dyn …Strategy>` slots; provides a builder (`with_context`, `with_capability`, …) for the override pattern.
- `impl Default for DefaultPlanner` — wires up nine `Default*Strategy` instances *that have not yet been written* (WS-5). To unblock WS-4 from WS-5, the brief uses *empty placeholder structs* under `cfg(test)` or behind a `cfg(feature = "default-strategies")` gate, so the trait surface compiles and the public API is verifiable. The real `Default*` impls land in WS-5 and replace the placeholders.

## 2. Files

### NEW
- `crates/ironclaw_agent_loop/src/planner.rs` — `AgentLoopPlanner` trait + `PlannerId` newtype
- `crates/ironclaw_agent_loop/src/default_planner.rs` — `DefaultPlanner` struct + builder

### EXTEND
- `crates/ironclaw_agent_loop/src/lib.rs` — export `planner`, `default_planner`

## 3. Specification

### 3.1 `PlannerId`

```rust
//! crates/ironclaw_agent_loop/src/planner.rs

use std::fmt;

/// Stable identifier for a planner composition. Carried in checkpoint payload
/// metadata so that resume can validate the planner being used hasn't drifted
/// from the planner that produced the checkpoint.
///
/// Validation: ASCII printable, no whitespace, max 96 bytes. No `From<String>`
/// (forces explicit `::new` so validation happens). See `.claude/rules/types.md`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct PlannerId(String);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlannerIdError {
    #[error("planner id must be 1..=96 bytes, got {0}")]
    InvalidLength(usize),
    #[error("planner id contains forbidden character at byte {0}")]
    ForbiddenChar(usize),
}

impl PlannerId {
    fn validate(s: &str) -> Result<(), PlannerIdError> { /* ascii printable, no ws, 1..=96 */ }
    pub fn new(raw: impl Into<String>) -> Result<Self, PlannerIdError>;
    pub fn as_str(&self) -> &str { &self.0 }
}

impl<'de> serde::Deserialize<'de> for PlannerId {
    /// Validates on the wire to match construction.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { /* try_from String */ }
}

impl fmt::Display for PlannerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}
```

### 3.2 `AgentLoopPlanner` trait

```rust
use crate::strategies::{
    BatchPolicyStrategy, BudgetStrategy, CapabilityStrategy, ContextStrategy,
    GateHandlingStrategy, InputDrainStrategy, ModelStrategy, RecoveryStrategy,
    StopConditionStrategy,
};

/// A planner is a composition of nine strategies. Each strategy is one
/// swappable decision-procedure consulted by the executor at a specific
/// point in the canonical tick (see master doc §8).
///
/// Implementations should be cheap to clone (typically wrap each strategy
/// in `Arc<dyn …Strategy>`) so the executor can borrow strategies without
/// constraining planner lifetimes.
///
/// The planner has NO `run()` or `tick()` method; loop mechanics live in
/// the `AgentLoopExecutor`. The planner is data — strategies + an id.
pub trait AgentLoopPlanner: Send + Sync {
    fn id(&self) -> &PlannerId;

    fn context(&self) -> &dyn ContextStrategy;
    fn capability(&self) -> &dyn CapabilityStrategy;
    fn model(&self) -> &dyn ModelStrategy;
    fn batch(&self) -> &dyn BatchPolicyStrategy;
    fn gate(&self) -> &dyn GateHandlingStrategy;
    fn recovery(&self) -> &dyn RecoveryStrategy;
    fn stop(&self) -> &dyn StopConditionStrategy;
    fn drain(&self) -> &dyn InputDrainStrategy;
    fn budget(&self) -> &dyn BudgetStrategy;
}
```

### 3.3 `DefaultPlanner`

```rust
//! crates/ironclaw_agent_loop/src/default_planner.rs

use std::sync::Arc;

use crate::planner::{AgentLoopPlanner, PlannerId};
use crate::strategies::*;

/// The reference planner. Composes nine strategies; each can be swapped
/// individually via the builder methods.
///
/// `DefaultPlanner::default()` returns the all-`Default*Strategy` composition
/// that models pi-mono behavior (see WS-5). Loop families build on top:
///
/// ```ignore
/// let coding = DefaultPlanner::default()
///     .with_context(Arc::new(CodingContextStrategy::new()))
///     .with_recovery(Arc::new(CodingRecoveryStrategy::new()));
/// ```
#[derive(Clone)]
pub struct DefaultPlanner {
    id: PlannerId,
    context: Arc<dyn ContextStrategy>,
    capability: Arc<dyn CapabilityStrategy>,
    model: Arc<dyn ModelStrategy>,
    batch: Arc<dyn BatchPolicyStrategy>,
    gate: Arc<dyn GateHandlingStrategy>,
    recovery: Arc<dyn RecoveryStrategy>,
    stop: Arc<dyn StopConditionStrategy>,
    drain: Arc<dyn InputDrainStrategy>,
    budget: Arc<dyn BudgetStrategy>,
}

impl DefaultPlanner {
    /// Override the context strategy. Returns `Self` for chaining.
    pub fn with_context(mut self, s: Arc<dyn ContextStrategy>) -> Self { self.context = s; self }
    pub fn with_capability(mut self, s: Arc<dyn CapabilityStrategy>) -> Self { self.capability = s; self }
    pub fn with_model(mut self, s: Arc<dyn ModelStrategy>) -> Self { self.model = s; self }
    pub fn with_batch(mut self, s: Arc<dyn BatchPolicyStrategy>) -> Self { self.batch = s; self }
    pub fn with_gate(mut self, s: Arc<dyn GateHandlingStrategy>) -> Self { self.gate = s; self }
    pub fn with_recovery(mut self, s: Arc<dyn RecoveryStrategy>) -> Self { self.recovery = s; self }
    pub fn with_stop(mut self, s: Arc<dyn StopConditionStrategy>) -> Self { self.stop = s; self }
    pub fn with_drain(mut self, s: Arc<dyn InputDrainStrategy>) -> Self { self.drain = s; self }
    pub fn with_budget(mut self, s: Arc<dyn BudgetStrategy>) -> Self { self.budget = s; self }

    /// Replace the planner id (loop families set their own to disambiguate
    /// in checkpoint payloads).
    pub fn with_id(mut self, id: PlannerId) -> Self { self.id = id; self }
}

impl AgentLoopPlanner for DefaultPlanner {
    fn id(&self) -> &PlannerId { &self.id }
    fn context(&self) -> &dyn ContextStrategy { &*self.context }
    fn capability(&self) -> &dyn CapabilityStrategy { &*self.capability }
    fn model(&self) -> &dyn ModelStrategy { &*self.model }
    fn batch(&self) -> &dyn BatchPolicyStrategy { &*self.batch }
    fn gate(&self) -> &dyn GateHandlingStrategy { &*self.gate }
    fn recovery(&self) -> &dyn RecoveryStrategy { &*self.recovery }
    fn stop(&self) -> &dyn StopConditionStrategy { &*self.stop }
    fn drain(&self) -> &dyn InputDrainStrategy { &*self.drain }
    fn budget(&self) -> &dyn BudgetStrategy { &*self.budget }
}

impl Default for DefaultPlanner {
    /// Composes nine Default*Strategy instances. Each Default* impl ships in
    /// WS-5; this Default impl is the integration point.
    fn default() -> Self {
        Self {
            id: PlannerId::new("reborn:default-loop").expect("static id is valid"),
            context: Arc::new(DefaultContextStrategy::default()),
            capability: Arc::new(DefaultCapabilityStrategy::default()),
            model: Arc::new(DefaultModelStrategy::default()),
            batch: Arc::new(DefaultBatchPolicyStrategy::default()),
            gate: Arc::new(DefaultGateHandlingStrategy::default()),
            recovery: Arc::new(DefaultRecoveryStrategy::default()),
            stop: Arc::new(DefaultStopConditionStrategy::default()),
            drain: Arc::new(DefaultInputDrainStrategy::default()),
            budget: Arc::new(DefaultBudgetStrategy::default()),
        }
    }
}
```

### 3.4 Coordinating with WS-5

`DefaultPlanner::default()` references nine types that WS-5 ships. To unblock WS-4 from being merged behind WS-5:

- **Option A (preferred): merge order WS-1/2/3 → WS-5 → WS-4.** WS-4 lands last and `default_planner.rs` references real types from the start.
- **Option B (parallel-friendly): WS-4 ships placeholder unit-struct stubs in the same files (`strategies/context.rs` etc.) for each `Default*Strategy`. The placeholder impls satisfy the trait but do nothing useful — `unimplemented!()` in the body. WS-5 then replaces the bodies. This lets WS-4 compile in isolation.

Pick A by default; only fall back to B if WS-5 is genuinely blocked. Note this in the brief's PR description.

## 4. Acceptance criteria

- [ ] `cargo check -p ironclaw_agent_loop` passes (whichever merge order)
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings
- [ ] Unit tests:
  - [ ] `PlannerId::new("reborn:default-loop")` succeeds; `PlannerId::new("")` fails with `InvalidLength`; `PlannerId::new("has space")` fails with `ForbiddenChar` (or whatever validation rejects whitespace)
  - [ ] `PlannerId` round-trips through `serde_json` (validates on deserialize)
  - [ ] `DefaultPlanner::default().id()` returns `"reborn:default-loop"`
  - [ ] `DefaultPlanner::default()` compiles, can be cloned, and is `Send + Sync`
  - [ ] `dyn AgentLoopPlanner` is object-safe: `fn _check(_: &dyn AgentLoopPlanner) {}`
  - [ ] Builder chaining works: `DefaultPlanner::default().with_id(...).with_context(...)` produces a planner whose `id()` and `context()` reflect the overrides

## 5. Out of scope

- The nine `Default*Strategy` impls — WS-5
- `AgentLoopExecutor` — WS-6
- `PlannedDriver` — WS-7
- A `families/default.rs` factory function — there isn't one; `DefaultPlanner::default()` IS the factory
- Loop-family planners (`coding_planner()` etc.) — out of skeleton scope

## 6. Verification command sequence

```bash
cargo check -p ironclaw_agent_loop
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_agent_loop
```

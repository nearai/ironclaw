# WS-4 — Planner Facade

**Workstream:** WS-4
**Crate touched:** `ironclaw_agent_loop`
**Depends on:** WS-1, WS-2, WS-3 (all nine strategy traits exist)
**Parallel with:** WS-5
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §3, §6

---

## 1. Scope

Land the composition layer that ties the nine strategies into a single thing the executor calls:

- `AgentLoopPlanner` trait — `pub` but **sealed** (only types inside `ironclaw_agent_loop` can implement it). Has `id()` / `version()` accessors visible to downstream crates; strategy accessors live on a `pub(crate)` extension trait so extensions cannot reach into a planner's strategies.
- `LoopFamilyId` and `ComponentIdentity` (defined in WS-3.5's `family.rs`) subsume what would otherwise have been a `PlannerId` newtype. There is no separate `PlannerId`.
- `DefaultPlanner` struct — owns nine `Arc<dyn …Strategy>` slots. The constructor is `pub(crate) fn compose_default` (and overloaded `compose` for future family factories); only `families::*` factory functions in this crate can construct one. Downstream crates do not call `DefaultPlanner::default()` or `DefaultPlanner::new(…)`.
- The builder-style overrides (`with_context`, `with_capability`, …) become `pub(crate)` mutator methods used inside `families::*` to construct family-specific compositions. They are not part of the public API.

## 2. Files

### NEW
- `crates/ironclaw_agent_loop/src/planner.rs` — `AgentLoopPlanner` trait (sealed) + `AgentLoopPlannerInternal` (`pub(crate)`). Identity newtypes (`LoopFamilyId`, `ComponentIdentity`) live in `family.rs` per WS-3.5
- `crates/ironclaw_agent_loop/src/default_planner.rs` — `DefaultPlanner` struct + builder

### EXTEND
- `crates/ironclaw_agent_loop/src/lib.rs` — export `planner`, `default_planner`

## 3. Specification

### 3.1 Identity types (now in WS-3.5)

`PlannerId` from the original brief is **subsumed by `LoopFamilyId` + `ComponentIdentity`** (defined in WS-3.5's `crates/ironclaw_agent_loop/src/family.rs`). Rationale per master doc §11 amendment: the top-layer abstraction is `LoopFamily`, not a planner identity newtype.

`AgentLoopPlanner` reports its own identity via:
- `fn id(&self) -> &LoopFamilyId` — which family this planner is the composition for
- `fn version(&self) -> &ComponentIdentity` — content-addressed identity used in checkpoint payload metadata

Both methods are on the public-facing trait surface. The actual strategy-by-slot accessors live on a `pub(crate)` extension trait — see §3.2.

### 3.2 `AgentLoopPlanner` trait — sealed

```rust
//! crates/ironclaw_agent_loop/src/planner.rs

use crate::family::{ComponentIdentity, LoopFamilyId};
use crate::strategies::{
    BatchPolicyStrategy, BudgetStrategy, CapabilityStrategy, ContextStrategy,
    GateHandlingStrategy, InputDrainStrategy, ModelStrategy, RecoveryStrategy,
    StopConditionStrategy,
};

/// Sealed-trait pattern. Downstream crates cannot implement `AgentLoopPlanner`
/// because they cannot implement `Sealed`. This enforces the master doc §9
/// invariant: planners (and their strategy compositions) are Builtin-only.
mod sealed { pub trait Sealed {} }

/// A planner is a composition of nine strategies. Each strategy is one
/// swappable decision-procedure consulted by the executor at a specific
/// point in the canonical tick (see master doc §8).
///
/// The public-facing surface exposes only identity. Strategy access lives on
/// the `pub(crate)` extension trait `AgentLoopPlannerInternal` below — the
/// `CanonicalAgentLoopExecutor` (also in this crate) is the only consumer.
/// Extensions cannot reach into a planner's strategies via this trait.
///
/// Implementations should be cheap to clone (typically wrap each strategy
/// in `Arc<dyn …Strategy>`) so the executor can borrow strategies without
/// constraining planner lifetimes.
///
/// The planner has NO `run()` or `tick()` method; loop mechanics live in
/// the `AgentLoopExecutor`. The planner is data — strategies + identity.
pub trait AgentLoopPlanner: sealed::Sealed + Send + Sync {
    /// The loop family this planner composes for. Stable across versions of
    /// the same family.
    fn id(&self) -> &LoopFamilyId;

    /// Content-addressed identity. Bumping the composition (swapping a
    /// `Default*Strategy` etc.) re-derives the digest and invalidates resume
    /// from older checkpoints. Carried in checkpoint payload metadata.
    fn version(&self) -> &ComponentIdentity;
}

/// Crate-private extension trait. The canonical executor inside
/// `ironclaw_agent_loop` consults strategies through this trait; downstream
/// crates cannot see it.
pub(crate) trait AgentLoopPlannerInternal: AgentLoopPlanner {
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

use crate::family::{ComponentDigest, ComponentIdentity, LoopFamilyId};
use crate::planner::{AgentLoopPlanner, AgentLoopPlannerInternal, sealed};
use crate::strategies::*;

/// The reference planner — a concrete composition of nine strategies. Public
/// for use in trait-object contexts (`Arc<dyn AgentLoopPlanner>` flows out of
/// `families::*` factories) but **not constructible by downstream crates**:
/// the `compose_*` constructors are `pub(crate)`, so only `families::*` in
/// this crate can build one.
///
/// Loop families compose by calling `DefaultPlanner::compose_default()` and
/// applying `with_*` overrides — both crate-private operations. The result
/// is wrapped into a `LoopFamily` via `LoopFamily::new`, also `pub(crate)`.
pub struct DefaultPlanner {
    id: LoopFamilyId,
    version: ComponentIdentity,
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
    /// Crate-private constructor: the all-`Default*Strategy` composition.
    /// Used by `families::default()`.
    pub(crate) fn compose_default() -> Self {
        Self {
            id: LoopFamilyId::DEFAULT,
            version: ComponentIdentity::new("default", families::default_family_digest()),
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

    // Crate-private builder methods. Future family factories use them to
    // produce family-specific compositions (e.g. `families::routine()`
    // overrides `stop` and `drain`). Downstream crates cannot call these.
    pub(crate) fn with_id(mut self, id: LoopFamilyId) -> Self { self.id = id; self }
    pub(crate) fn with_version(mut self, version: ComponentIdentity) -> Self { self.version = version; self }
    pub(crate) fn with_context(mut self, s: Arc<dyn ContextStrategy>) -> Self { self.context = s; self }
    pub(crate) fn with_capability(mut self, s: Arc<dyn CapabilityStrategy>) -> Self { self.capability = s; self }
    pub(crate) fn with_model(mut self, s: Arc<dyn ModelStrategy>) -> Self { self.model = s; self }
    pub(crate) fn with_batch(mut self, s: Arc<dyn BatchPolicyStrategy>) -> Self { self.batch = s; self }
    pub(crate) fn with_gate(mut self, s: Arc<dyn GateHandlingStrategy>) -> Self { self.gate = s; self }
    pub(crate) fn with_recovery(mut self, s: Arc<dyn RecoveryStrategy>) -> Self { self.recovery = s; self }
    pub(crate) fn with_stop(mut self, s: Arc<dyn StopConditionStrategy>) -> Self { self.stop = s; self }
    pub(crate) fn with_drain(mut self, s: Arc<dyn InputDrainStrategy>) -> Self { self.drain = s; self }
    pub(crate) fn with_budget(mut self, s: Arc<dyn BudgetStrategy>) -> Self { self.budget = s; self }
}

impl sealed::Sealed for DefaultPlanner {}

impl AgentLoopPlanner for DefaultPlanner {
    fn id(&self) -> &LoopFamilyId { &self.id }
    fn version(&self) -> &ComponentIdentity { &self.version }
}

impl AgentLoopPlannerInternal for DefaultPlanner {
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
```

No `impl Default for DefaultPlanner` — public `Default::default()` would defeat the seal. Construction goes through `families::default()` only.

### 3.4 Coordinating with WS-5 and WS-3.5

`DefaultPlanner::compose_default()` references nine types that WS-5 ships, and the resulting planner is wrapped into a `LoopFamily` by `families::default()` (defined in WS-3.5). Merge ordering:

- **Preferred: WS-1/2/3 → WS-5 → WS-4 → WS-3.5.** WS-3.5 lands last; `families::default()` references real `DefaultPlanner::compose_default()` and the wired family registry composes cleanly into `ironclaw_reborn`.
- **Parallel-friendly fallback: WS-4 ships placeholder unit-struct stubs in `strategies/context.rs` etc. for each `Default*Strategy`. The placeholder impls satisfy the trait but do nothing useful — `unimplemented!()` in the body. WS-5 then replaces the bodies. WS-3.5 can land alongside.

Pick the preferred order by default; only fall back if WS-5 is genuinely blocked. Note this in the brief's PR description.

## 4. Acceptance criteria

- [ ] `cargo check -p ironclaw_agent_loop` passes (whichever merge order)
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings
- [ ] Unit tests:
  - [ ] `families::default().id() == &LoopFamilyId::DEFAULT`
  - [ ] `families::default().version().id == "default"`
  - [ ] `dyn AgentLoopPlanner` is object-safe: `fn _check(_: &dyn AgentLoopPlanner) {}`
  - [ ] `Arc<dyn AgentLoopPlanner>` is `Send + Sync` (compiles)
  - [ ] Internal builder chaining works inside `families::*`: `DefaultPlanner::compose_default().with_id(...).with_context(...)` produces a planner whose `id()` reflects the override (test lives in the same module so it has crate-private access)
- [ ] Negative tests (manual review checklist):
  - [ ] `DefaultPlanner::default()` does NOT exist as a public function (no `impl Default for DefaultPlanner`)
  - [ ] `DefaultPlanner::compose_default()` is `pub(crate)` — invisible to downstream
  - [ ] All `with_*` mutators are `pub(crate)`
  - [ ] `AgentLoopPlannerInternal` is not exported from the crate

## 5. Out of scope

- The nine `Default*Strategy` impls — WS-5
- `AgentLoopExecutor` — WS-6
- `PlannedDriver` — WS-7
- `LoopFamily`, `LoopFamilyId`, `LoopFamilyRegistry`, `families::default()` factory — WS-3.5
- Loop-family planners (hypothetical `routine`, `mission`, `coding`, `planning` families) — out of skeleton scope; future families add `pub fn` factories in `families::*`

## 6. Verification command sequence

```bash
cargo check -p ironclaw_agent_loop
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_agent_loop
```

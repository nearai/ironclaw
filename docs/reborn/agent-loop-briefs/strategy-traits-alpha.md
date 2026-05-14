# WS-1 — Strategy Traits α: Context / Capability / Model

**Workstream:** WS-1
**Crate touched:** `ironclaw_agent_loop`
**Depends on:** WS-0 (`LoopExecutionState`, slot types)
**Parallel with:** WS-2, WS-3
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §6

---

## 1. Scope

Land three pure-policy strategy traits — they read state and return a request value the executor passes to the host. None of them mutate state slots; therefore none has an outcome enum.

- `ContextStrategy` — picks the prompt-bundle request shape (and optional inline messages — pi's "nudge" role).
- `CapabilityStrategy` — picks the capability filter for the visible surface.
- `ModelStrategy` — picks the model preference for the next stream call.

Trait stubs only; concrete `Default*` impls land in WS-5.

## 2. Files

### NEW
- `crates/ironclaw_agent_loop/src/strategies/mod.rs` — module declarations + re-exports for the three traits in this brief (WS-2, WS-3 add to this same `mod.rs`)
- `crates/ironclaw_agent_loop/src/strategies/context.rs`
- `crates/ironclaw_agent_loop/src/strategies/capability.rs`
- `crates/ironclaw_agent_loop/src/strategies/model.rs`

### NOT TOUCHED in this brief
- `Default*Strategy` impls — WS-5
- `LoopPromptBundleRequest.inline_messages` field — already exists or extends in this brief if needed (see §3.1 note)
- Strategy state slots (already in WS-0)

## 3. Specification

### 3.1 `ContextStrategy`

```rust
//! crates/ironclaw_agent_loop/src/strategies/context.rs

use async_trait::async_trait;
use ironclaw_turns::run_profile::LoopPromptBundleRequest;

use crate::state::LoopExecutionState;
use ironclaw_turns::run_profile::VisibleCapabilityFilter;

/// Decides what context the host should materialize for the next model call.
///
/// Pure policy: returns the request value the executor will pass to
/// `LoopPromptPort::build_prompt_bundle`. Does NOT mutate state.
///
/// Inline messages (the role pi-mono's "nudge" mechanism plays) flow through
/// the `inline_messages` field of `LoopPromptBundleRequest` — there is no
/// separate `NudgeStrategy`. Loop families that want a nudge mechanism extend
/// their `ContextStrategy` to populate this field based on `state`.
///
/// See `docs/reborn/agent-loop-skeleton.md` §6.
#[async_trait]
pub trait ContextStrategy: Send + Sync {
    async fn plan_context_request(
        &self,
        state: &LoopExecutionState,
    ) -> LoopPromptBundleRequest;
}
```

**Note on `inline_messages`:** WS-0 extends `LoopPromptBundleRequest` (in `ironclaw_turns::run_profile::host`) with `inline_messages: Vec<LoopInlineMessage>` (default empty). WS-1 consumes this field; it does not extend the request type. The change is additive: the existing `TextOnlyModelReplyDriver` constructs the request without populating `inline_messages` and continues to compile and pass tests unchanged.

### 3.2 `CapabilityStrategy`

```rust
//! crates/ironclaw_agent_loop/src/strategies/capability.rs

use async_trait::async_trait;

use crate::state::LoopExecutionState;

/// Decides which capabilities are visible to the model this iteration.
///
/// Pure policy: returns a filter the executor passes to the host when
/// requesting the visible capability surface. Does NOT mutate state.
///
/// The host is the source of truth for the catalog and applies its own
/// scope/grant/auth filters AFTER the strategy filter; the strategy can
/// only narrow, never expand.
#[async_trait]
pub trait CapabilityStrategy: Send + Sync {
    async fn filter(&self, state: &LoopExecutionState) -> VisibleCapabilityFilter;
}

/// Strategy-side narrowing of the visible capability surface.
///
/// Variants are mutually exclusive. The host always applies its own
/// scope/grant/auth filters on top — this filter only narrows.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VisibleCapabilityFilter {
    /// Allow everything the host would otherwise expose.
    All,
    /// Only the capabilities whose names appear in the set.
    AllowOnly(Vec<ironclaw_turns::run_profile::CapabilityName>),
    /// Everything except the capabilities whose names appear in the set.
    Deny(Vec<ironclaw_turns::run_profile::CapabilityName>),
}

impl Default for VisibleCapabilityFilter {
    fn default() -> Self { VisibleCapabilityFilter::All }
}
```

`VisibleCapabilityFilter` lives in `ironclaw_turns`, not in
`ironclaw_agent_loop`, because it is carried over the contract-level
`VisibleCapabilityRequest`. The strategy crate imports the contract type;
the contract crate never depends upward on strategies.

If `CapabilityName` does not yet exist as a typed newtype in `ironclaw_turns`, this brief uses whatever name-shape the existing capability descriptor surface uses. Adding a new newtype is *not* in scope for WS-1; track in a follow-up if needed.

### 3.3 `ModelStrategy`

```rust
//! crates/ironclaw_agent_loop/src/strategies/model.rs

use async_trait::async_trait;

use crate::state::LoopExecutionState;

/// Decides which model preference to pass on the next `stream_model` call.
///
/// Pure policy: returns a `ModelPreference` the executor includes in
/// `LoopModelRequest`. Does NOT mutate state.
///
/// The actual model the host calls is bound by `LoopRunContext.resolved_model_route`
/// (or, post-deferral, the resolved model route chain). The strategy's preference
/// is a hint that the host may interpret (e.g. picking among already-resolved
/// fallbacks). Strategies cannot introduce new routes mid-run.
#[async_trait]
pub trait ModelStrategy: Send + Sync {
    async fn preference(&self, state: &LoopExecutionState) -> ModelPreference;
}

/// Strategy hint to the host about which already-resolved route to use.
///
/// In the skeleton (no fallback chain yet), `Primary` is the only meaningful
/// value; `Fallback { index }` is wired through but reserved for the future
/// `ModelRouteChain` follow-up (see master doc §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPreference {
    Primary,
    Fallback { index: u32 },
}

impl Default for ModelPreference {
    fn default() -> Self { ModelPreference::Primary }
}
```

The `state.model_state.fallback_index` (defined in WS-0) feeds the typical default impl: `if state.model_state.fallback_index == 0 { Primary } else { Fallback { index } }`.

## 4. Acceptance criteria

- [ ] `cargo check -p ironclaw_agent_loop` passes with the three new modules
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings
- [ ] Unit tests per file:
  - [ ] `context.rs` — has at least a doctest or compile-time test asserting `dyn ContextStrategy` is object-safe (`fn _check(_: &dyn ContextStrategy) {}`)
- [ ] `capability.rs` — `VisibleCapabilityFilter::default()` returns `All`; `serde_json` round-trip preserves variants
  - [ ] `model.rs` — `ModelPreference::default()` returns `Primary`; `serde` snake-case round-trip
- [ ] After WS-0 added `LoopPromptBundleRequest.inline_messages`: the existing `TextOnlyModelReplyDriver` still compiles and its tests pass unchanged (it never populates the field)

## 5. Out of scope

- `DefaultContextStrategy`, `DefaultCapabilityStrategy`, `DefaultModelStrategy` — WS-5
- `BatchPolicyStrategy`, `GateHandlingStrategy`, `RecoveryStrategy` — WS-2
- `StopConditionStrategy`, `InputDrainStrategy`, `BudgetStrategy` — WS-3
- `AgentLoopPlanner` facade — WS-4
- Anything mutating `state` — these strategies are pure policy; outcome enums live in WS-2 and WS-3 only

## 6. Verification command sequence

```bash
cargo check -p ironclaw_agent_loop
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_agent_loop
cargo test -p ironclaw_reborn   # if LoopPromptBundleRequest extended, ensure TextOnly driver still works
```

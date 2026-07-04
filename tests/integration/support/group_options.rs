//! Runtime-wiring setters for [`RebornIntegrationGroupBuilder`].
//!
//! `group.rs` owns the builder's struct definition and the shared
//! assembly mechanics (`build_base`/`into_group`); this file is a private
//! child module of `group` (declared `#[path = "group_options.rs"] mod
//! group_options;` in `group.rs`, NOT `pub mod` from `mod.rs`, same
//! precedent as `group_constructors.rs`) that catalogs the builder's
//! optional-runtime-wiring setters — `storage`, `safety_context`,
//! `with_turn_event_sink`, `budget_accounting`,
//! `communication_context_provider`, `hook_dispatcher_builder_factory`.
//! Keeping it a child module (rather than a top-level sibling) lets it
//! reach `RebornIntegrationGroupBuilder`'s private fields at plain
//! module-private visibility instead of widening them to `pub(crate)` for
//! the whole test-support crate. New builder setters belong HERE, not back
//! in `group.rs`.

// Shared by all group test binaries; symbols read as dead when a binary does
// not exercise every setter (mirrors the same attribute on `group.rs`/
// `group_constructors.rs`).
#![allow(dead_code)]

use std::sync::Arc;

use ironclaw_reborn::loop_driver_host::HookDispatcherBuilderFactory;
use ironclaw_turns::InMemoryTurnEventSink;
use ironclaw_turns::run_profile::{CommunicationContextProvider, InstructionSafetyContext};

use super::super::builder::StorageMode;
use super::RebornIntegrationGroupBuilder;

impl RebornIntegrationGroupBuilder {
    /// Select the durable storage backend (default: `StorageMode::InMemory`).
    /// Use `StorageMode::LibSql` to exercise on-disk durability across
    /// `assert_reply_persists_after_reopen`.
    pub fn storage(mut self, mode: StorageMode) -> Self {
        self.storage = mode;
        self
    }

    /// Wire a model-visible instruction-safety banner into the group's ONE
    /// shared planned runtime (`DefaultPlannedRuntimeParts::safety_context`).
    /// Rendered verbatim as a `system`-role prompt message ahead of any
    /// per-turn instructions (`push_safety_context`); the only model-visible
    /// artifact of instruction-safety scanning on this tier (T0-SYSPROMPT /
    /// C-SAFETY). Defaults to `None` (no banner, matching today's behavior).
    pub fn safety_context(mut self, ctx: InstructionSafetyContext) -> Self {
        self.safety_context = Some(ctx);
        self
    }

    /// Install an in-memory `TurnEventSink` (`ironclaw_turns::InMemoryTurnEventSink`,
    /// a real, already-shipped production type with zero callers today — this is the
    /// seam production wires via `subscribe_best_effort` in `build_default_planned_runtime_inner`,
    /// `crates/ironclaw_reborn/src/runtime.rs:613-619`) into the group's ONE planned
    /// runtime (C-TRACECAP). Read the recorded events back with
    /// [`RebornIntegrationHarness::recorded_turn_events`] — the ONLY read path;
    /// it slices `[baseline_turn_event_count..]` so a group thread never sees a
    /// sibling thread's events. Deliberately no raw group-level sink accessor:
    /// one would bypass that slicing and reintroduce cross-thread bleed.
    pub fn with_turn_event_sink(mut self) -> Self {
        self.turn_event_sink = Some(Arc::new(InMemoryTurnEventSink::default()));
        self
    }

    /// Wire the production `build_default_budget_accountant` (over in-memory
    /// governor, gate store, zero-cost table, and compiled-default seeding) into
    /// the group's ONE shared planned runtime (`DefaultPlannedRuntimeParts::model_budget_accountant`),
    /// and retain the governor for read-back. This is the C-BUDGET liveness seam:
    /// on the first model call of any turn the accountant seeds the run owner's
    /// daily USD cap into the governor, which
    /// `RebornIntegrationHarness::assert_budget_user_cap_seeded` reads back.
    /// Budget SEMANTICS (thresholds, gates, `BudgetEvent` cascade) are covered at
    /// crate tier (`budget_e2e.rs`); this only proves the harness bypass path
    /// (`build_default_planned_runtime`) wires the accountant live. Defaults off.
    pub fn budget_accounting(mut self) -> Self {
        self.budget = true;
        self
    }

    /// Wire a `CommunicationContextProvider` into the group's ONE shared planned
    /// runtime (`DefaultPlannedRuntimeParts::communication_context_provider`), so
    /// the delivery-preference / connected-channel slice it resolves renders into
    /// the model request. This is the C-COMMCTX seam — distinct from the outbound
    /// delivery **sink** (E-OUTBOUND): this is prompt **context**. Defaults `None`.
    pub fn communication_context_provider(
        mut self,
        provider: Arc<dyn CommunicationContextProvider>,
    ) -> Self {
        self.communication_context_provider = Some(provider);
        self
    }

    /// Wire a per-run `HookDispatcherBuilderFactory` into the group's ONE shared
    /// planned runtime (`DefaultPlannedRuntimeParts::hook_dispatcher_builder_factory`),
    /// so hooks fire at their lifecycle points on a coordinator-path turn. This is
    /// the E-HOOK-INFRA / C-HOOKS seam. Defaults `None` (hook framework dormant).
    pub fn hook_dispatcher_builder_factory(
        mut self,
        factory: HookDispatcherBuilderFactory,
    ) -> Self {
        self.hook_dispatcher_builder_factory = Some(factory);
        self
    }
}

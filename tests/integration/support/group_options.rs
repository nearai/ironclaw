//! Runtime-wiring setters for [`RebornIntegrationGroupBuilder`] — `storage`,
//! `safety_context`, `with_turn_event_sink`, `with_trace_capture`,
//! `with_tool_disclosure_bridged`, `with_tool_disclosure_off`, `budget_accounting`,
//! `communication_context_provider`, `hook_dispatcher_builder_factory`.
//! Private child module of `group.rs` (owns the struct + `build_base`/
//! `into_group`), so it reaches the builder's private fields at module-
//! private visibility instead of widening them to `pub(crate)`. New builder
//! setters belong HERE.

// Shared by all group test binaries; symbols read as dead when a binary does
// not exercise every setter (mirrors the same attribute on `group.rs`/
// `group_constructors.rs`).
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use ironclaw_reborn::loop_driver_host::HookDispatcherBuilderFactory;
use ironclaw_reborn::runtime::ToolDisclosureMode;
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

    /// Install an in-memory `InMemoryTurnEventSink` into the group's ONE
    /// planned runtime via production's `subscribe_best_effort` seam
    /// (C-TRACECAP). Read back via
    /// [`RebornIntegrationHarness::recorded_turn_events`] — the ONLY read
    /// path; it slices `[baseline_turn_event_count..]` so threads don't see
    /// siblings' events. No raw sink accessor, deliberately.
    pub fn with_turn_event_sink(mut self) -> Self {
        self.turn_event_sink = Some(Arc::new(InMemoryTurnEventSink::default()));
        self
    }

    /// Wire the PRODUCTION `TraceCaptureTurnEventSink` into the group's ONE
    /// planned runtime (enabler (c), C-TRACECAP), via composition's
    /// `trace_capture_turn_event_sink_for_test` — the same sink + scope-seed
    /// recipe `build_reborn_runtime` composes. Read the seeded scope back with
    /// `RebornIntegrationGroup::trace_capture_scope()`. Composes with
    /// `.with_turn_event_sink()` through the group's fan-out. NOTE: capture
    /// resolves policy/queue paths through `IRONCLAW_BASE_DIR` (a process-wide
    /// `LazyLock`) — the consuming test binary must point it at a tempdir as
    /// its FIRST action (see `tests/integration/trace_capture.rs`).
    pub fn with_trace_capture(mut self) -> Self {
        self.trace_capture = true;
        self
    }

    /// Force `ToolDisclosureMode::Bridged` into the group's ONE planned
    /// runtime config (enabler (b)), regardless of `REBORN_TOOL_DISCLOSURE` —
    /// avoids the shared-process env-var race `apply_hermetic_env()` already
    /// guards against (see `ToolDisclosureMode::from_env`). Defaults `None`
    /// (resolves via `from_env()`, matching today's behavior).
    pub fn with_tool_disclosure_bridged(mut self) -> Self {
        self.tool_disclosure = Some(ToolDisclosureMode::Bridged);
        self
    }

    /// Force `ToolDisclosureMode::Off` into the group's ONE planned runtime
    /// config, regardless of `REBORN_TOOL_DISCLOSURE`. Used by the disclosure
    /// mode's negative-control test to pin Off-mode behavior explicitly:
    /// leaving it on the `from_env()` default-resolution path would let an
    /// ambient `REBORN_TOOL_DISCLOSURE=Bridged` (e.g. from a developer's
    /// shell or a differently-configured CI runner) silently flip the control
    /// into the very mode it's meant to disprove. `apply_hermetic_env()` also
    /// scrubs the var for defense in depth, but this explicit opt-in is what
    /// actually makes the control's assertion mode-specific rather than
    /// env-dependent.
    pub fn with_tool_disclosure_off(mut self) -> Self {
        self.tool_disclosure = Some(ToolDisclosureMode::Off);
        self
    }

    /// Wire the production `build_default_budget_accountant` into the group's
    /// ONE planned runtime and retain the governor for read-back (C-BUDGET
    /// liveness seam: the accountant seeds the run owner's daily cap on the
    /// first model call; read back via `assert_budget_user_cap_seeded`).
    /// Budget semantics are covered at crate tier (`budget_e2e.rs`); this only
    /// proves the harness wires the accountant live. Defaults off.
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

    /// Shorten the group's turn-state store lease TTL (default 90s,
    /// `InMemoryTurnStateStoreLimits::default()`) for lease-expiry-under-a-
    /// wedged-tool coverage (see `tests/integration/lease_wedge.rs`).
    /// `None` (default) leaves today's behavior byte-identical.
    pub fn with_runner_lease_ttl_for_test(mut self, ttl: chrono::Duration) -> Self {
        self.runner_lease_ttl_override = Some(ttl);
        self
    }

    /// Shorten the group's scheduler lease-recovery sweep interval (default
    /// 10s, `TurnRunSchedulerConfig::lease_recovery_interval`) so a wedged run
    /// is reaped without waiting on the production tick (see
    /// `tests/integration/lease_wedge.rs`). `None` (default) leaves today's
    /// behavior byte-identical.
    pub fn with_lease_recovery_interval_for_test(mut self, interval: Duration) -> Self {
        self.lease_recovery_interval_override = Some(interval);
        self
    }

    /// Wire the REAL approval/auth interaction services (via the group's
    /// `HostRuntimeCapabilityHarness`'s retained `RebornServices`, over the
    /// group's own shared turn-state store) into every thread's
    /// `DefaultProductWorkflow`, so `submit_inbound(ApprovalResolution/
    /// AuthResolution)` dispatches through the SAME arms a real adapter reply
    /// hits, instead of every workflow's default `Rejecting*InteractionService`
    /// stubs. Requires a `HostRuntime` capability backend built via
    /// `new_with_options` (e.g. `live_approvals`, `live_auth_and_approval`) —
    /// `RebornThreadBuilder::build()` errors otherwise. Defaults off (every
    /// other group keeps today's Rejecting-stub behavior).
    pub fn with_real_gate_dispatch_services(mut self) -> Self {
        self.real_gate_dispatch_services = true;
        self
    }
}

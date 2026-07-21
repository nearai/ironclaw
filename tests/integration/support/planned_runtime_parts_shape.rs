//! W5-WIRING-PARITY: test-tree-only Some/None shape extraction for
//! `DefaultPlannedRuntimeParts`.
//!
//! Lives in the shared support tree (not `tests/integration/wiring_parity.rs`
//! itself) because the exhaustive destructure must run at the harness's OWN
//! construction site (`group.rs`'s `into_group`, right after the struct
//! literal is bound to `parts` and before `build_default_planned_runtime`
//! consumes it by value) — that is the only place the full, real value
//! exists. `wiring_parity.rs` only reads the already-computed shape back off
//! `GroupSharedStorage`/`RebornIntegrationHarness`.
//!
//! Zero production-crate changes: `DefaultPlannedRuntimeParts` is already
//! `pub` with `pub` fields and no `#[non_exhaustive]`
//! (`crates/ironclaw_runner/src/runtime.rs:260-326`), so this file only reads
//! it from test-tree code.

use ironclaw_loop_host::HostManagedModelGateway;
use ironclaw_runner::runtime::DefaultPlannedRuntimeParts;

/// Some/None shape of `DefaultPlannedRuntimeParts`'s 14 `Option`-typed
/// fields. Field VALUES are out of scope by design (see
/// `tests/integration/wiring_parity.rs`'s module doc) — only whether each
/// optional wiring seam is populated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultPlannedRuntimePartsShape {
    pub model_route_resolver: bool,
    pub cancellation_factory: bool,
    pub skill_context_source: bool,
    pub attachment_read_port: bool,
    pub gate_record_store: bool,
    pub input_queue: bool,
    pub model_policy_guard: bool,
    pub model_budget_accountant: bool,
    pub safety_context: bool,
    pub hook_security_audit_sink: bool,
    pub turn_event_sink: bool,
    pub hook_dispatcher_builder_factory: bool,
    pub communication_context_provider: bool,
    pub scheduler_wake_wiring: bool,
}

/// Exhaustive, no-`..` destructure of `parts` into its Option-field shape.
///
/// Every one of the 32 fields is named explicitly here (the 19 required
/// fields bound to `_`), so this function FAILS TO COMPILE the moment a
/// field is added to or removed from `DefaultPlannedRuntimeParts` — the
/// tripwire `wiring_parity.rs` relies on. Match ergonomics on `&parts` bind
/// every field by reference, so nothing is moved out of the caller's value.
pub fn harness_planned_runtime_parts_shape<G>(
    parts: &DefaultPlannedRuntimeParts<G>,
) -> DefaultPlannedRuntimePartsShape
where
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    let DefaultPlannedRuntimeParts {
        turn_state: _,
        thread_service: _,
        thread_scope: _,
        model_gateway: _,
        checkpoint_state_store: _,
        loop_checkpoint_store: _,
        milestone_sink: _,
        capability_factory: _,
        capability_surface_resolver: _,
        capability_result_writer: _,
        subagent_goal_store: _,
        subagent_await_edge_writer: _,
        subagent_await_edge_settler: _,
        subagent_await_edge_evidence: _,
        subagent_definition_resolver: _,
        subagent_spawn_input_codec: _,
        subagent_spawn_limits: _,
        loop_exit_evidence: _,
        config: _,
        model_route_resolver,
        cancellation_factory,
        skill_context_source,
        attachment_read_port,
        gate_record_store,
        input_queue,
        identity_context_source: _,
        user_profile_source: _,
        model_policy_guard,
        model_budget_accountant,
        safety_context,
        hook_security_audit_sink,
        turn_event_sink,
        hook_dispatcher_builder_factory,
        communication_context_provider,
        scheduler_wake_wiring,
    } = parts;
    DefaultPlannedRuntimePartsShape {
        model_route_resolver: model_route_resolver.is_some(),
        cancellation_factory: cancellation_factory.is_some(),
        skill_context_source: skill_context_source.is_some(),
        attachment_read_port: attachment_read_port.is_some(),
        gate_record_store: gate_record_store.is_some(),
        input_queue: input_queue.is_some(),
        model_policy_guard: model_policy_guard.is_some(),
        model_budget_accountant: model_budget_accountant.is_some(),
        safety_context: safety_context.is_some(),
        hook_security_audit_sink: hook_security_audit_sink.is_some(),
        turn_event_sink: turn_event_sink.is_some(),
        hook_dispatcher_builder_factory: hook_dispatcher_builder_factory.is_some(),
        communication_context_provider: communication_context_provider.is_some(),
        scheduler_wake_wiring: scheduler_wake_wiring.is_some(),
    }
}

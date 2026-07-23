//! Trace-capture `TurnEventSink` test support (C-TRACECAP seam).

/// Build the SAME `TraceCaptureTurnEventSink` production fans into
/// `DefaultPlannedRuntimeParts.turn_event_sink` inside `build_ironclaw_runtime`
/// (`runtime.rs`, "Autonomous Trace Commons capture"), seeded with the
/// runtime owner's tenant-scoped trace scope key — the identical
/// `trace_scope_key(tenant, owner)` recipe, so the test path never drifts
/// from production wiring. Capture stays policy-gated per scope (inert until
/// the scope enrolls), exactly as in production.
///
/// Returns the sink alongside the EXACT scope key it was seeded with, so the
/// caller has a single source of truth instead of recomputing
/// `trace_scope_key(tenant_id, actor_user_id)` a second time (which risks the
/// two call sites drifting if either recipe ever changes independently).
#[cfg(feature = "test-support")]
pub fn trace_capture_turn_event_sink_for_test(
    thread_service: std::sync::Arc<dyn ironclaw_threads::SessionThreadService>,
    tenant_id: &str,
    actor_user_id: &str,
) -> (std::sync::Arc<dyn ironclaw_turns::TurnEventSink>, String) {
    let scope = ironclaw_traces::contribution::trace_scope_key(tenant_id, actor_user_id);
    let observed_scopes: crate::observability::trace_capture::ObservedTraceScopes =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::from([
            scope.clone(),
        ])));
    let sink = std::sync::Arc::new(
        crate::observability::trace_capture::TraceCaptureTurnEventSink::new(
            thread_service,
            observed_scopes,
        ),
    );
    (sink, scope)
}

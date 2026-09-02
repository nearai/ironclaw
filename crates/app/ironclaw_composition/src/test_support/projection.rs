//! NARROWER than production's chain in `runtime.rs`: only `.with_turn_events`
//! (no approval/display-preview/auth enrichment) — enough for the SSE
//! turn-lifecycle scenario (Enabler A). `wiring_parity` guard (#5642) tracks
//! zero fields here; follow-up: add a projection-assembly field to it.

use std::sync::Arc;

use ironclaw_event_log::DurableEventLog;
use ironclaw_product_contracts::projection::ProjectionStream;
use ironclaw_turns::{ReplyTargetBindingRef, TurnCoordinator, TurnEventProjectionSource};

/// Build a turn-lifecycle-only `ProjectionStream` for
/// `RebornServices::with_event_stream` test wiring; see module doc for the
/// narrowing vs. production's assembly.
#[cfg(feature = "test-support")]
pub fn build_product_event_stream_for_test(
    event_log: Arc<dyn DurableEventLog>,
    turn_event_source: Arc<dyn TurnEventProjectionSource>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    reply_target_binding_ref: ReplyTargetBindingRef,
) -> Arc<dyn ProjectionStream> {
    ironclaw_assistant::projection::build_reborn_projection_services(
        event_log,
        reply_target_binding_ref,
    )
    .with_turn_events(turn_event_source, turn_coordinator)
    .product_event_stream()
}

/// Like [`build_product_event_stream_for_test`], additionally wiring the
/// session thread service exactly as production's
/// `build_reborn_projection_services(...).with_thread_service(...)` does
/// (`crates/app/ironclaw_composition/src/runtime.rs`), so the completed-turn
/// projection can resolve and embed the finalized assistant reply
/// (`ProductProjectionItem::Text { finalized: true }`). Use this variant for
/// tests asserting durable-final-reply parity across a stream transport.
#[cfg(feature = "test-support")]
pub fn build_product_event_stream_with_thread_service_for_test(
    event_log: Arc<dyn DurableEventLog>,
    turn_event_source: Arc<dyn TurnEventProjectionSource>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    reply_target_binding_ref: ReplyTargetBindingRef,
    thread_service: Arc<dyn ironclaw_threads::SessionThreadService>,
) -> Arc<dyn ProjectionStream> {
    ironclaw_assistant::projection::build_reborn_projection_services(
        event_log,
        reply_target_binding_ref,
    )
    .with_turn_events(turn_event_source, turn_coordinator)
    .with_thread_service(thread_service)
    .product_event_stream()
}

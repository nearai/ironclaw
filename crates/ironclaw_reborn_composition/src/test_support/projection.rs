//! NARROWER than production's chain in `runtime.rs`: only `.with_turn_events`
//! (no approval/display-preview/auth enrichment) — enough for the SSE
//! turn-lifecycle scenario (Enabler A). `wiring_parity` guard (#5642) tracks
//! zero fields here; follow-up: add a projection-assembly field to it.

use std::sync::Arc;

use ironclaw_events::DurableEventLog;
use ironclaw_product_adapters::ProjectionStream;
use ironclaw_turns::{ReplyTargetBindingRef, TurnCoordinator, TurnEventProjectionSource};

/// Build a turn-lifecycle-only `ProjectionStream` for
/// `RebornServices::with_event_stream` test wiring; see module doc for the
/// narrowing vs. production's assembly.
#[cfg(feature = "test-support")]
pub fn build_webui_event_stream_for_test(
    event_log: Arc<dyn DurableEventLog>,
    turn_event_source: Arc<dyn TurnEventProjectionSource>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    reply_target_binding_ref: ReplyTargetBindingRef,
) -> Arc<dyn ProjectionStream> {
    crate::projection::build_reborn_projection_services(event_log, reply_target_binding_ref)
        .with_turn_events(turn_event_source, turn_coordinator)
        .webui_event_stream()
}

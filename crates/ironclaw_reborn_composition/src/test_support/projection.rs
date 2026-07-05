//! Test-support constructor for the WebUI event-stream projection
//! (W5-WEBUI-API-1 Enabler A, SSE-activity-stream scenario).
//!
//! `build_reborn_projection_services` / `.with_turn_events` /
//! `.webui_event_stream()` are `pub(crate)` inside this crate. Production's
//! real call site (`runtime.rs`, inside `build_reborn_runtime`) always chains
//! `.with_approval_requests(...)` whenever `local_runtime` is `Some` (which it
//! always is in a local-dev-shaped harness), plus a live-progress-milestone-
//! sink chain, `.with_model_failure_explainer_factory`, and conditionally
//! `.with_display_previews`/`.with_auth_challenges`.
//!
//! **This constructor is a deliberately NARROWER assembly than production's**:
//! it wires ONLY `.with_turn_events(...)` before calling
//! `.webui_event_stream()` — no approval-request re-hydration, no
//! display-preview/auth-challenge enrichment, no live-progress milestone
//! sink. That is sufficient for W5-WEBUI-API-1's SSE scenario, which only
//! asserts the turn-lifecycle half of the stream (`TurnEventBridge`); it is
//! NOT a general-purpose production stand-in for approval/auth/display-preview
//! projection coverage.
//!
//! `tests/integration/wiring_parity.rs`'s `DefaultPlannedRuntimePartsShape`/
//! `EXPECTED_PRODUCTION_SHAPE` guard (#5642) tracks zero fields for this
//! projection/event-stream assembly, so a future production change to the
//! real chain (e.g. an unconditional `.with_display_previews`) will NOT make
//! this test — or that guard — go red. This narrowing is intentionally
//! documented here rather than fixed in this lane; a follow-up should extend
//! the wiring-parity shape with a projection-assembly field.

use std::sync::Arc;

use ironclaw_events::DurableEventLog;
use ironclaw_product_adapters::ProjectionStream;
use ironclaw_turns::{ReplyTargetBindingRef, TurnCoordinator, TurnEventProjectionSource};

/// Build a `ProjectionStream` wired ONLY for turn-lifecycle events, for
/// `RebornServices::with_event_stream` test wiring (the WebUI-facing
/// `ironclaw_product_workflow::RebornServices` facade, not this crate's
/// composition-level type of the same name). See the module doc for the
/// narrowing vs. production's real assembly.
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

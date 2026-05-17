//! Convenience constructor for an axum [`Router`] wired to the
//! WebChat v2 handlers.
//!
//! Host composition is free to ignore this and mount each handler directly
//! against its own router; the descriptors in [`crate::descriptors`] are
//! the canonical contract. This module exists so handler-level tests can
//! drive the full route table without re-stating the path/method table.

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use ironclaw_product_workflow::RebornServicesApi;

use crate::descriptors::{
    WEBUI_V2_PATTERN_CANCEL_RUN, WEBUI_V2_PATTERN_CREATE_THREAD, WEBUI_V2_PATTERN_GET_TIMELINE,
    WEBUI_V2_PATTERN_RESOLVE_GATE, WEBUI_V2_PATTERN_SEND_MESSAGE, WEBUI_V2_PATTERN_STREAM_EVENTS,
};
use crate::handlers;

/// Shared state injected into every WebChat v2 handler.
///
/// Handlers receive a single facade so they can never reach into the
/// dispatcher, run-state, or any runtime lane directly.
#[derive(Clone)]
pub struct WebUiV2State {
    services: Arc<dyn RebornServicesApi>,
}

impl WebUiV2State {
    pub fn new(services: Arc<dyn RebornServicesApi>) -> Self {
        Self { services }
    }

    pub fn services(&self) -> &Arc<dyn RebornServicesApi> {
        &self.services
    }
}

/// Build a [`Router`] mounting the six WebChat v2 routes against the
/// supplied facade. Path patterns match
/// [`crate::descriptors::webui_v2_routes`] exactly; host composition is
/// expected to apply its own auth / CORS / body-limit middleware in front
/// of this router.
pub fn webui_v2_router(state: WebUiV2State) -> Router {
    Router::new()
        .route(
            WEBUI_V2_PATTERN_CREATE_THREAD,
            post(handlers::create_thread),
        )
        .route(WEBUI_V2_PATTERN_SEND_MESSAGE, post(handlers::send_message))
        .route(WEBUI_V2_PATTERN_GET_TIMELINE, get(handlers::get_timeline))
        .route(WEBUI_V2_PATTERN_STREAM_EVENTS, get(handlers::stream_events))
        .route(WEBUI_V2_PATTERN_CANCEL_RUN, post(handlers::cancel_run))
        .route(WEBUI_V2_PATTERN_RESOLVE_GATE, post(handlers::resolve_gate))
        .with_state(state)
}

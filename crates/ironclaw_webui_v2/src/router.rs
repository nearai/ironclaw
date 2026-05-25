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
    WEBUI_V2_PATTERN_RESOLVE_GATE, WEBUI_V2_PATTERN_SEND_MESSAGE, WEBUI_V2_PATTERN_SETUP_EXTENSION,
    WEBUI_V2_PATTERN_STREAM_EVENTS, WEBUI_V2_PATTERN_STREAM_EVENTS_WS,
};
use crate::handlers;
use crate::sse_capacity::{DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, SseCapacity};

/// Shared state injected into every WebChat v2 handler.
///
/// Handlers receive a single facade so they can never reach into the
/// dispatcher, run-state, or any runtime lane directly. The state also
/// owns the [`SseCapacity`] gate that bounds concurrent SSE streams per
/// `(tenant, user)`; cloning the state shares the same gate so all
/// handler invocations enforce one cap process-wide.
#[derive(Clone)]
pub struct WebUiV2State {
    services: Arc<dyn RebornServicesApi>,
    sse_capacity: Arc<SseCapacity>,
}

impl WebUiV2State {
    /// Build state with the default per-caller SSE concurrency cap
    /// ([`DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER`]).
    pub fn new(services: Arc<dyn RebornServicesApi>) -> Self {
        Self::with_sse_concurrency_limit(services, DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER)
    }

    /// Build state with a custom per-caller SSE concurrency cap. Use
    /// from host composition or tests that want to tune the ceiling.
    pub fn with_sse_concurrency_limit(
        services: Arc<dyn RebornServicesApi>,
        max_concurrent_streams_per_caller: usize,
    ) -> Self {
        Self {
            services,
            sse_capacity: Arc::new(SseCapacity::new(max_concurrent_streams_per_caller)),
        }
    }

    pub fn services(&self) -> &Arc<dyn RebornServicesApi> {
        &self.services
    }

    pub(crate) fn sse_capacity(&self) -> &Arc<SseCapacity> {
        &self.sse_capacity
    }
}

/// Build a [`Router`] mounting the six WebChat v2 routes against the
/// supplied facade. Path patterns match
/// [`crate::descriptors::webui_v2_routes`] exactly; host composition is
/// expected to apply its own auth / CORS / body-limit middleware in front
/// of this router.
pub fn webui_v2_router(state: WebUiV2State) -> Router {
    Router::new()
        // GET and POST share the `/api/webchat/v2/threads` path
        // (`WEBUI_V2_PATTERN_CREATE_THREAD == WEBUI_V2_PATTERN_LIST_THREADS`);
        // mount both verbs in one `.route()` so axum's matcher
        // dispatches by method.
        .route(
            WEBUI_V2_PATTERN_CREATE_THREAD,
            post(handlers::create_thread).get(handlers::list_threads),
        )
        .route(WEBUI_V2_PATTERN_SEND_MESSAGE, post(handlers::send_message))
        .route(WEBUI_V2_PATTERN_GET_TIMELINE, get(handlers::get_timeline))
        .route(WEBUI_V2_PATTERN_STREAM_EVENTS, get(handlers::stream_events))
        .route(
            WEBUI_V2_PATTERN_STREAM_EVENTS_WS,
            get(handlers::stream_events_ws),
        )
        .route(WEBUI_V2_PATTERN_CANCEL_RUN, post(handlers::cancel_run))
        .route(WEBUI_V2_PATTERN_RESOLVE_GATE, post(handlers::resolve_gate))
        .route(
            WEBUI_V2_PATTERN_SETUP_EXTENSION,
            post(handlers::setup_extension),
        )
        .with_state(state)
}

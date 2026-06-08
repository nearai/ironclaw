//! Convenience constructor for an axum [`Router`] wired to the
//! WebChat v2 handlers.
//!
//! Host composition is free to ignore this and mount each handler directly
//! against its own router; the descriptors in [`crate::descriptors`] are
//! the canonical contract. This module exists so handler-level tests can
//! drive the full route table without re-stating the path/method table.

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post};
use ironclaw_product_workflow::RebornServicesApi;
use serde::Serialize;

use crate::descriptors::{
    WEBUI_V2_PATTERN_ACTIVATE_EXTENSION, WEBUI_V2_PATTERN_CANCEL_RUN,
    WEBUI_V2_PATTERN_COMPLETE_NEARAI_WALLET_LOGIN, WEBUI_V2_PATTERN_CREATE_THREAD,
    WEBUI_V2_PATTERN_DELETE_LLM_PROVIDER, WEBUI_V2_PATTERN_DELETE_THREAD,
    WEBUI_V2_PATTERN_GET_LLM_CONFIG, WEBUI_V2_PATTERN_GET_SESSION, WEBUI_V2_PATTERN_GET_TIMELINE,
    WEBUI_V2_PATTERN_INSTALL_EXTENSION, WEBUI_V2_PATTERN_LIST_AUTOMATIONS,
    WEBUI_V2_PATTERN_LIST_CONNECTABLE_CHANNELS, WEBUI_V2_PATTERN_LIST_EXTENSION_REGISTRY,
    WEBUI_V2_PATTERN_LIST_EXTENSIONS, WEBUI_V2_PATTERN_LIST_LLM_MODELS,
    WEBUI_V2_PATTERN_REMOVE_EXTENSION, WEBUI_V2_PATTERN_RESOLVE_GATE,
    WEBUI_V2_PATTERN_SEND_MESSAGE, WEBUI_V2_PATTERN_SET_ACTIVE_LLM,
    WEBUI_V2_PATTERN_SETUP_EXTENSION, WEBUI_V2_PATTERN_START_CODEX_LOGIN,
    WEBUI_V2_PATTERN_START_NEARAI_LOGIN, WEBUI_V2_PATTERN_STREAM_EVENTS,
    WEBUI_V2_PATTERN_STREAM_EVENTS_WS, WEBUI_V2_PATTERN_TEST_LLM_CONNECTION,
};
use crate::handlers;
use crate::sse_capacity::SseCapacity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebUiV2RouteOptions {
    pub mount_llm_config_routes: bool,
}

impl WebUiV2RouteOptions {
    pub const fn all() -> Self {
        Self {
            mount_llm_config_routes: true,
        }
    }

    pub const fn without_llm_config_routes() -> Self {
        Self {
            mount_llm_config_routes: false,
        }
    }
}

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
    capabilities: WebUiV2Capabilities,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct WebUiV2Capabilities {
    pub operator_webui_config: bool,
}

impl WebUiV2State {
    pub fn new(
        services: Arc<dyn RebornServicesApi>,
        capabilities: WebUiV2Capabilities,
        max_concurrent_streams_per_caller: usize,
    ) -> Self {
        Self {
            services,
            sse_capacity: Arc::new(SseCapacity::new(max_concurrent_streams_per_caller)),
            capabilities,
        }
    }

    pub fn services(&self) -> &Arc<dyn RebornServicesApi> {
        &self.services
    }

    pub(crate) fn sse_capacity(&self) -> &Arc<SseCapacity> {
        &self.sse_capacity
    }

    pub(crate) fn capabilities(&self) -> WebUiV2Capabilities {
        self.capabilities
    }
}

/// Build a [`Router`] mounting the WebChat v2 routes against the supplied
/// facade. Path patterns match
/// [`crate::descriptors::webui_v2_routes`] exactly; host composition is
/// expected to apply its own auth / CORS / body-limit middleware in front
/// of this router.
pub fn webui_v2_router(state: WebUiV2State) -> Router {
    webui_v2_router_with_options(state, WebUiV2RouteOptions::all())
}

pub fn webui_v2_router_with_options(state: WebUiV2State, options: WebUiV2RouteOptions) -> Router {
    let mut router = Router::new()
        // GET and POST share the `/api/webchat/v2/threads` path
        // (`WEBUI_V2_PATTERN_CREATE_THREAD == WEBUI_V2_PATTERN_LIST_THREADS`);
        // mount both verbs in one `.route()` so axum's matcher
        // dispatches by method.
        .route(
            WEBUI_V2_PATTERN_CREATE_THREAD,
            post(handlers::create_thread).get(handlers::list_threads),
        )
        .route(
            WEBUI_V2_PATTERN_DELETE_THREAD,
            delete(handlers::delete_thread),
        )
        .route(WEBUI_V2_PATTERN_GET_SESSION, get(handlers::get_session))
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
            WEBUI_V2_PATTERN_LIST_AUTOMATIONS,
            get(handlers::list_automations),
        )
        .route(
            WEBUI_V2_PATTERN_LIST_CONNECTABLE_CHANNELS,
            get(handlers::list_connectable_channels),
        )
        .route(
            WEBUI_V2_PATTERN_LIST_EXTENSIONS,
            get(handlers::list_extensions),
        )
        .route(
            WEBUI_V2_PATTERN_LIST_EXTENSION_REGISTRY,
            get(handlers::list_extension_registry),
        )
        .route(
            WEBUI_V2_PATTERN_INSTALL_EXTENSION,
            post(handlers::install_extension),
        )
        .route(
            WEBUI_V2_PATTERN_ACTIVATE_EXTENSION,
            post(handlers::activate_extension),
        )
        .route(
            WEBUI_V2_PATTERN_REMOVE_EXTENSION,
            post(handlers::remove_extension),
        )
        .route(
            WEBUI_V2_PATTERN_SETUP_EXTENSION,
            get(handlers::get_extension_setup).post(handlers::setup_extension),
        );
    if options.mount_llm_config_routes {
        router = router
            // `WEBUI_V2_PATTERN_GET_LLM_CONFIG == WEBUI_V2_PATTERN_UPSERT_LLM_PROVIDER`
            // (`/llm/providers`); mount GET + POST in one `.route()`.
            .route(
                WEBUI_V2_PATTERN_GET_LLM_CONFIG,
                get(handlers::get_llm_config).post(handlers::upsert_llm_provider),
            )
            .route(
                WEBUI_V2_PATTERN_DELETE_LLM_PROVIDER,
                post(handlers::delete_llm_provider),
            )
            .route(
                WEBUI_V2_PATTERN_SET_ACTIVE_LLM,
                post(handlers::set_active_llm),
            )
            .route(
                WEBUI_V2_PATTERN_TEST_LLM_CONNECTION,
                post(handlers::test_llm_connection),
            )
            .route(
                WEBUI_V2_PATTERN_LIST_LLM_MODELS,
                post(handlers::list_llm_models),
            )
            .route(
                WEBUI_V2_PATTERN_START_NEARAI_LOGIN,
                post(handlers::start_nearai_login),
            )
            .route(
                WEBUI_V2_PATTERN_COMPLETE_NEARAI_WALLET_LOGIN,
                post(handlers::complete_nearai_wallet_login),
            )
            .route(
                WEBUI_V2_PATTERN_START_CODEX_LOGIN,
                post(handlers::start_codex_login),
            );
    }
    router.with_state(state)
}

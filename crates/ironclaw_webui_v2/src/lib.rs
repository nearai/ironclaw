//! Reborn WebChat v2 HTTP route surface.
//!
//! This crate ships the minimal native WebUI v2 route set on top of the
//! [`ironclaw_product_workflow::RebornServicesApi`] facade. It is off by
//! default — enable the `webui-v2-beta` Cargo feature to compile it in.
//!
//! ## Boundaries
//!
//! - Handlers consume only [`RebornServicesApi`] for chat, run/gate,
//!   extension, and automation reads. They never reach into the dispatcher,
//!   `HostRuntime`, run-state, DB stores, or any runtime lane.
//! - Auth and CORS are **not** enforced here. Host composition runs the
//!   bearer-token middleware that builds a [`WebUiAuthenticatedCaller`] and
//!   injects it as an `Extension` before traffic reaches these handlers.
//! - The [`IngressRouteDescriptor`] set returned by [`webui_v2_routes`] is
//!   the canonical contract the host composes against: mount path, method,
//!   auth scheme, body / rate limit, streaming mode, audit class, and the
//!   allowed effect path. Adding a new route here requires a matching
//!   descriptor.
//!
//! ## Streaming
//!
//! `stream_events` is exposed as SSE. The current
//! [`RebornServicesApi::stream_events`] is drain-only, so the handler
//! drains once, renders each product envelope into a
//! [`WebChatV2EventFrame`] SSE message with the projection cursor as the
//! SSE id, then polls at a low cadence for newly-arrived events. When the
//! facade gains a real subscription API the handler can migrate without
//! changing the descriptor or browser-visible event schema.
//!
//! Beyond the route descriptor's per-caller request rate limit, the
//! handler caps the number of *concurrent* SSE streams a single
//! `(tenant, user)` may hold and closes any single stream after a fixed
//! maximum lifetime so leaked guards or stuck pollers cannot wedge a
//! caller's slot indefinitely.
//!
//! [`RebornServicesApi`]: ironclaw_product_workflow::RebornServicesApi
//! [`WebChatV2EventFrame`]: crate::WebChatV2EventFrame
//! [`WebUiAuthenticatedCaller`]: ironclaw_product_workflow::WebUiAuthenticatedCaller
//! [`IngressRouteDescriptor`]: ironclaw_host_api::ingress::IngressRouteDescriptor

#![forbid(unsafe_code)]

#[cfg(feature = "webui-v2-beta")]
mod descriptors;
#[cfg(feature = "webui-v2-beta")]
mod error;
#[cfg(feature = "webui-v2-beta")]
mod handlers;
#[cfg(feature = "webui-v2-beta")]
mod router;
#[cfg(feature = "webui-v2-beta")]
mod schema;
#[cfg(feature = "webui-v2-beta")]
mod sse_capacity;

#[cfg(feature = "webui-v2-beta")]
pub use descriptors::{
    WEBUI_V2_ROUTE_ACTIVATE_EXTENSION, WEBUI_V2_ROUTE_CANCEL_RUN, WEBUI_V2_ROUTE_CREATE_THREAD,
    WEBUI_V2_ROUTE_DELETE_LLM_PROVIDER, WEBUI_V2_ROUTE_GET_EXTENSION_SETUP,
    WEBUI_V2_ROUTE_GET_LLM_CONFIG, WEBUI_V2_ROUTE_GET_TIMELINE, WEBUI_V2_ROUTE_INSTALL_EXTENSION,
    WEBUI_V2_ROUTE_LIST_AUTOMATIONS, WEBUI_V2_ROUTE_LIST_CONNECTABLE_CHANNELS,
    WEBUI_V2_ROUTE_LIST_EXTENSION_REGISTRY, WEBUI_V2_ROUTE_LIST_EXTENSIONS,
    WEBUI_V2_ROUTE_LIST_LLM_MODELS, WEBUI_V2_ROUTE_LIST_THREADS, WEBUI_V2_ROUTE_REMOVE_EXTENSION,
    WEBUI_V2_ROUTE_RESOLVE_GATE, WEBUI_V2_ROUTE_SEND_MESSAGE, WEBUI_V2_ROUTE_SET_ACTIVE_LLM,
    WEBUI_V2_ROUTE_SETUP_EXTENSION, WEBUI_V2_ROUTE_STREAM_EVENTS, WEBUI_V2_ROUTE_STREAM_EVENTS_WS,
    WEBUI_V2_ROUTE_TEST_LLM_CONNECTION, WEBUI_V2_ROUTE_UPSERT_LLM_PROVIDER,
    is_webui_v2_llm_config_route_id, webui_v2_routes,
};
#[cfg(feature = "webui-v2-beta")]
pub use error::{WebUiV2HttpError, WebUiV2HttpErrorBody};
#[cfg(feature = "webui-v2-beta")]
pub use handlers::{
    activate_extension, cancel_run, create_thread, delete_llm_provider, get_extension_setup,
    get_llm_config, get_timeline, install_extension, list_automations, list_connectable_channels,
    list_extension_registry, list_extensions, list_llm_models, list_threads, remove_extension,
    resolve_gate, send_message, set_active_llm, setup_extension, stream_events, stream_events_ws,
    test_llm_connection, upsert_llm_provider,
};
#[cfg(feature = "webui-v2-beta")]
pub use router::{
    WebUiV2RouteOptions, WebUiV2State, webui_v2_router, webui_v2_router_with_options,
};
#[cfg(feature = "webui-v2-beta")]
pub use schema::{WebChatV2Event, WebChatV2EventFrame};

//! Reborn WebChat v2 HTTP route surface (#3611).
//!
//! This crate ships the minimal native WebUI v2 route set on top of the
//! [`ironclaw_product_workflow::RebornServicesApi`] facade. It is off by
//! default — enable the `webui-v2-beta` Cargo feature to compile it in.
//!
//! ## Boundaries
//!
//! - Handlers consume only [`RebornServicesApi`] for all six commands
//!   (`create_thread`, `submit_turn`, `get_timeline`, `stream_events`,
//!   `cancel_run`, `resolve_gate`). They never reach into the dispatcher,
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
//! drains once, emits each event as an SSE message with the projection
//! cursor as the SSE id, then polls at a low cadence for newly-arrived
//! events. When the facade gains a real subscription API the handler can
//! migrate without changing the descriptor.
//!
//! Beyond the route descriptor's per-caller request rate limit, the
//! handler caps the number of *concurrent* SSE streams a single
//! `(tenant, user)` may hold and closes any single stream after a fixed
//! maximum lifetime so leaked guards or stuck pollers cannot wedge a
//! caller's slot indefinitely.
//!
//! [`RebornServicesApi`]: ironclaw_product_workflow::RebornServicesApi
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
mod sse_capacity;

#[cfg(feature = "webui-v2-beta")]
pub use descriptors::{
    WEBUI_V2_ROUTE_CANCEL_RUN, WEBUI_V2_ROUTE_CREATE_THREAD, WEBUI_V2_ROUTE_GET_TIMELINE,
    WEBUI_V2_ROUTE_RESOLVE_GATE, WEBUI_V2_ROUTE_SEND_MESSAGE, WEBUI_V2_ROUTE_STREAM_EVENTS,
    webui_v2_routes,
};
#[cfg(feature = "webui-v2-beta")]
pub use error::{WebUiV2HttpError, WebUiV2HttpErrorBody};
#[cfg(feature = "webui-v2-beta")]
pub use handlers::{
    cancel_run, create_thread, get_timeline, resolve_gate, send_message, stream_events,
};
#[cfg(feature = "webui-v2-beta")]
pub use router::{WebUiV2State, webui_v2_router};

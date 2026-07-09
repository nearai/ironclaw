//! Reborn WebUI cluster — the WebUI services facade plus the beta HTTP
//! serve/middleware surface. Grouped behind one internal module; the crate
//! root re-exports the same public items so the public API is unchanged.

pub(crate) mod facade;
#[cfg(feature = "webui-v2-beta")]
pub(crate) mod webui_body_limit;
#[cfg(feature = "webui-v2-beta")]
pub(crate) mod webui_operator_auth;
#[cfg(feature = "webui-v2-beta")]
pub(crate) mod webui_rate_limit;
#[cfg(feature = "webui-v2-beta")]
pub(crate) mod webui_route_match;
#[cfg(feature = "webui-v2-beta")]
pub(crate) mod webui_serve;
#[cfg(feature = "webui-v2-beta")]
pub(crate) mod webui_ws_origin;

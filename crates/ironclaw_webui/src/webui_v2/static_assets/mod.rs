//! Reborn WebChat v2 static asset bundle.
//!
//! Ships the browser-side SPA that drives the JSON route surface in this crate.
//! The SPA targets only the `/api/webchat/v2/*` endpoints — no v1 gateway
//! routes, no engine APIs. See `crates/ironclaw_webui/frontend/` for the
//! TypeScript/Vite project and the issue #3886 plan for the hard non-goals.
//!
//! Folded in from the former `ironclaw_webui_v2_static` crate: the SPA bytes and
//! the JSON route surface now ship from one crate behind the single
//! `webui-v2-beta` feature. Bearer auth, CORS, body limits, and rate limits stay
//! in the composition middleware stack — this module only emits asset bytes and
//! substitutes a per-request CSP nonce into the SPA shell.
//!
//! The whole module is gated behind `webui-v2-beta` (see the parent `lib.rs`),
//! so a default build carries no asset table and no `axum`/`rand` code.

mod assets;
mod router;

pub use router::{
    StaticRouterConfig, StaticRouterConfigError, serve_root, serve_wildcard, static_router,
    static_router_with_config,
};

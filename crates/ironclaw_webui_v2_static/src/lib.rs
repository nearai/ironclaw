//! Reborn WebChat v2 static asset bundle.
//!
//! This crate ships the browser-side SPA that drives the JSON route
//! surface in [`ironclaw_webui_v2`]. The SPA targets only the nine
//! `/api/webchat/v2/*` endpoints — no v1 gateway routes, no engine
//! APIs. See `crates/ironclaw_webui_v2_static/static/` for the
//! bundle and the issue #3886 plan for the hard non-goals.
//!
//! ## Boundary
//!
//! The crate is intentionally self-contained: it depends on no other
//! workspace crates and exposes a single factory,
//! [`static_router`], that the host composition layer mounts under
//! `/v2`. Bearer auth, CORS, body limits, and rate limits stay in the
//! composition middleware stack — this crate only emits asset bytes
//! and substitutes a per-request CSP nonce into the SPA shell.
//!
//! Off by default; compile in with the `webui-v2-beta` feature.

#![cfg_attr(not(feature = "webui-v2-beta"), allow(dead_code))]

#[cfg(feature = "webui-v2-beta")]
mod assets;

#[cfg(feature = "webui-v2-beta")]
mod router;

#[cfg(feature = "webui-v2-beta")]
pub use router::{mount_at_prefix, serve_root, serve_wildcard, static_router};

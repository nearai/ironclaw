//! Platform layer for the web gateway.
//!
//! This submodule holds the gateway's transport and framing concerns: shared
//! state, the Axum route composition, static asset serving, and (in later
//! stages of ironclaw#2599) auth / SSE / WS. Feature-specific handlers live
//! alongside their domain (`handlers/` today, `features/<slice>/` in later
//! stages) and depend on the platform layer, not the other way around.
//!
//! See `src/channels/web/CLAUDE.md` for the staged migration plan.

pub mod router;
pub mod state;
pub mod static_files;

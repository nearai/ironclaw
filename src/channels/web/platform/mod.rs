//! Platform layer for the web gateway.
//!
//! This submodule holds the gateway's transport and framing concerns:
//! shared state, the Axum route composition, static asset serving, bearer
//! / OIDC auth, and the SSE / WebSocket broadcast fan-out.
//!
//! **Dependency direction.** Feature handlers (under `handlers/` today,
//! `features/<slice>/` later) depend on platform types (`GatewayState`,
//! rate limiters, auth extractors, `SseManager`, `WsConnectionTracker`).
//! Platform *submodules* do **not** reach back into feature handlers —
//! with the single, intentional exception of [`router`], which is the
//! composition point. The router imports every feature handler it
//! registers; that is its job. The "no back-edges" rule enforced by
//! future CI (ironclaw#2599 stage 5) applies to every platform module
//! except `router`.
//!
//! See `src/channels/web/CLAUDE.md` for the staged migration plan.

pub mod auth;
pub mod engine_dispatch;
pub mod legacy_auth;
pub mod router;
pub mod sse;
pub mod state;
pub mod static_files;
pub mod ws;

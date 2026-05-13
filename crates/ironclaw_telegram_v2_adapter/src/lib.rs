//! Telegram WASM v2 ProductAdapter (issue #3285 tracer-bullet).
//!
//! This crate implements the Telegram side of the Reborn ProductAdapter
//! contract defined in `ironclaw_product_adapters`. It is a clean rewrite
//! that does **not** depend on legacy v1 channel types.
//!
//! Layering:
//!
//! * [`payload`] — Telegram Bot API payload normalization (private/group
//!   gating, attachment descriptors, idempotency from `update_id`).
//! * [`adapter`] — `ProductAdapter` impl (`parse_inbound`, `render_outbound`).
//! * [`render`] — `FinalReplyView` -> `sendMessage` body shaping.
//!
//! The crate ships as a native Rust ProductAdapter so the contract can be
//! exercised end-to-end against fakes today; the wasmtime component-model
//! binary build lands in a follow-up landing alongside the
//! `crates/ironclaw_wasm_product_adapters` runtime glue.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod payload;
pub mod render;

pub use adapter::{
    TelegramV2Adapter, TelegramV2AdapterConfig, telegram_declared_egress_hosts,
    telegram_default_capabilities,
};
pub use payload::{
    GroupTriggerPolicy, PayloadParseError, TELEGRAM_API_HOST, TELEGRAM_FILE_API_HOST,
    TELEGRAM_USER_ACTOR_KIND, parse_telegram_update,
};
pub use render::{TelegramRenderError, render_final_reply, render_progress_typing};

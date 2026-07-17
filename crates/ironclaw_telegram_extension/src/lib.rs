//! Telegram channel extension for Reborn (issue #3285).
//!
//! This crate implements the Telegram side of the Reborn generic-ingress
//! `ChannelAdapter` contract defined in `ironclaw_product_adapters`. It is a
//! clean rewrite that does **not** depend on legacy v1 channel types.
//!
//! Layering:
//!
//! * [`payload`] — Telegram Bot API payload normalization (private/group
//!   gating, attachment descriptors, idempotency from `update_id`).
//! * [`channel`] — the generic-ingress `ChannelAdapter` (live inbound/outbound
//!   + webhook registration hooks, extension-runtime P4).
//! * [`render`] — `FinalReplyView` -> `sendMessage` body shaping.

#![forbid(unsafe_code)]

mod channel;
mod payload;
mod render;

pub use channel::{
    TELEGRAM_BOT_TOKEN_HANDLE, TELEGRAM_WEBHOOK_SECRET_HANDLE, TELEGRAM_WEBHOOK_URL_CONFIG,
    TelegramChannelAdapter,
};
pub use payload::{
    GroupTriggerPolicy, PayloadParseError, TELEGRAM_API_HOST, TELEGRAM_FILE_API_HOST,
    TELEGRAM_USER_ACTOR_KIND, TelegramInboundEvent,
    normalize_telegram_update, parse_telegram_update,
};
pub use render::{TelegramRenderError, render_final_reply, render_progress_typing};

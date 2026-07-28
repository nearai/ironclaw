//! Telegram Bot API protocol engine (issue #3285).
//!
//! Pure protocol work with no I/O and no secrets: payload
//! parsing/normalization and outbound request rendering. The Telegram
//! channel extension (`ironclaw_telegram_extension`) layers the generic
//! `ChannelAdapter` on top of this crate; hosts run manifest-declared
//! verification/egress around it.
//!
//! Layering:
//!
//! * [`payload`] — Bot API payload normalization (private/group gating,
//!   attachment descriptors, idempotency from `update_id`, channel-normalized
//!   [`TelegramInboundEvent`]).
//! * [`render`] — `FinalReplyView` -> `sendMessage` body shaping.

#![forbid(unsafe_code)]

mod payload;
mod render;

pub use payload::{
    GroupTriggerPolicy, PayloadParseError, TELEGRAM_API_HOST, TELEGRAM_FILE_API_HOST,
    TELEGRAM_USER_ACTOR_KIND, TelegramInboundEvent, normalize_telegram_update,
    parse_telegram_update,
};
pub use render::{
    TelegramRenderError, TelegramReplyTarget, build_reply_target_binding, parse_reply_target,
    render_auth_prompt, render_final_reply, render_gate_prompt, render_progress_typing,
    resolve_reply_target,
};

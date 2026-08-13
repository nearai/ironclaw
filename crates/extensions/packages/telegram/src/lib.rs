//! Telegram channel extension for Reborn (issue #3285).
//!
//! The Telegram side of the Reborn channel capability contracts defined in
//! `ironclaw_extension_contracts`. The package owns both halves of the
//! extension:
//!
//! * the pure Bot API protocol engine — no I/O and no secrets — in
//!   [`payload`] (payload normalization: private/group gating, attachment
//!   descriptors, idempotency from `update_id`, and the channel-normalized
//!   [`TelegramInboundEvent`]) plus [`render`] (opaque reply-target encoding
//!   and Telegram UTF-16 message chunking);
//! * the channel capabilities — complete inbound (including the two-hop file
//!   exchange), reply, and delivery — which stay free of raw token bytes:
//!   hosts run the manifest-declared `shared_secret_header` verification and
//!   inject credentials on mediated egress. Webhook registration is manifest
//!   data executed by the generic host.
//!
//! The protocol engine shipped as its own crate
//! (`ironclaw_telegram_v2_adapter`) until the Wave 2 package colocation
//! merged it here, giving Telegram the same one-crate-per-package shape Slack
//! already had (PROPOSAL §5, CHECKLIST WS2).
//!
#![forbid(unsafe_code)]

mod attachment_transfer;
mod channel;
mod payload;
mod preference_targets;
mod render;

pub use channel::{
    TELEGRAM_BOT_TOKEN_HANDLE, TELEGRAM_BOT_USERNAME_CONFIG, TELEGRAM_WEBHOOK_SECRET_HANDLE,
    TELEGRAM_WEBHOOK_URL_CONFIG, TelegramChannelAdapter,
};
pub use payload::{
    GroupTriggerPolicy, PayloadParseError, TELEGRAM_API_HOST, TELEGRAM_FILE_API_HOST,
    TELEGRAM_USER_ACTOR_KIND, TelegramInboundEvent, normalize_telegram_update,
};
pub use preference_targets::TelegramPreferenceTargetCodec;
pub use render::{
    TelegramRenderError, TelegramReplyTarget, build_reply_target_binding, parse_reply_target,
};

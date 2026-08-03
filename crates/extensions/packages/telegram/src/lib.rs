//! Telegram channel extension for Reborn (issue #3285).
//!
//! The Telegram side of the Reborn generic-ingress [`ChannelAdapter`]
//! contract defined in `ironclaw_extension_contracts`. The package owns both
//! halves of the extension:
//!
//! * the pure Bot API protocol engine — no I/O and no secrets — in
//!   [`payload`] (payload normalization: private/group gating, attachment
//!   descriptors, idempotency from `update_id`, the channel-normalized
//!   [`TelegramInboundEvent`]) and [`render`] (`FinalReplyView` ->
//!   `sendMessage` body shaping);
//! * the adapter itself — live inbound/outbound plus the webhook
//!   registration hooks (extension-runtime P4) — which stays free of raw
//!   token bytes: hosts run the manifest-declared `shared_secret_header`
//!   verification and inject credentials on mediated egress.
//!
//! The protocol engine shipped as its own crate
//! (`ironclaw_telegram_v2_adapter`) until the Wave 2 package colocation
//! merged it here, giving Telegram the same one-crate-per-package shape Slack
//! already had (PROPOSAL §5, CHECKLIST WS2).
//!
//! [`ChannelAdapter`]: ironclaw_extension_contracts::ChannelAdapter

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
    parse_telegram_update,
};
pub use preference_targets::TelegramPreferenceTargetCodec;
pub use render::{
    TelegramRenderError, TelegramReplyTarget, build_reply_target_binding, parse_reply_target,
    render_auth_prompt, render_final_reply, render_gate_prompt, render_progress_typing,
    resolve_reply_target,
};

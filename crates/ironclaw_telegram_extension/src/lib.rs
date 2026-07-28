//! Telegram channel extension for Reborn (issue #3285).
//!
//! The Telegram side of the Reborn generic-ingress [`ChannelAdapter`]
//! contract defined in `ironclaw_product`. Pure Bot API protocol
//! work (payload normalization, outbound rendering) lives in
//! `ironclaw_telegram_v2_adapter`; this crate owns the adapter itself —
//! live inbound/outbound plus the webhook registration hooks
//! (extension-runtime P4) — and stays free of raw token bytes: hosts run
//! the manifest-declared `shared_secret_header` verification and inject
//! credentials on mediated egress.
//!
//! [`ChannelAdapter`]: ironclaw_host_api::product_adapter::ChannelAdapter

#![forbid(unsafe_code)]

mod channel;
mod preference_targets;

pub use channel::{
    TELEGRAM_BOT_TOKEN_HANDLE, TELEGRAM_WEBHOOK_SECRET_HANDLE, TELEGRAM_WEBHOOK_URL_CONFIG,
    TelegramChannelAdapter,
};
pub use preference_targets::TelegramPreferenceTargetCodec;

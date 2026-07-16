//! Reborn Telegram channel host (single `telegram` extension).
//!
//! Mirrors the Slack host-beta module shape: operator-managed bot setup,
//! manifest-projected webhook ingress, per-user pairing (WebGeneratedCode)
//! binding Telegram accounts to Reborn users, and outbound DM delivery
//! targets. Everything is gated on the `telegram-v2-host-beta` feature.

#[cfg(feature = "telegram-v2-host-beta")]
pub(crate) mod telegram_actor_identity;
#[cfg(feature = "telegram-v2-host-beta")]
pub(crate) mod telegram_bot_api;
#[cfg(feature = "telegram-v2-host-beta")]
pub(crate) mod telegram_host_state;
#[cfg(feature = "telegram-v2-host-beta")]
pub(crate) mod telegram_pairing;
#[cfg(feature = "telegram-v2-host-beta")]
pub(crate) mod telegram_setup;

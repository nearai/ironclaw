//! Telegram channel host for IronClaw Reborn (the single `telegram`
//! extension).
//!
//! Owns the Telegram host domain: operator bot setup (+ Bot API client),
//! durable filesystem host state, WebGeneratedCode pairing, the DM-only
//! pairing-aware dispatch pre-router, the manifest-projected webhook serve
//! fragment with its dynamic per-setup-revision installation resolver, actor
//! identity, admin/pairing channel routes, connectable/connection facades,
//! the outbound DM target provider, and `TelegramDeliveryProtocol`.
//!
//! Composition (`ironclaw_reborn_composition::telegram::telegram_host_beta`)
//! keeps only the thin wiring layer: it builds these services from
//! `RebornRuntime` parts, wraps the route fragments into its mount shapes,
//! and registers the delivery hooks. Behavior contract:
//! `docs/reborn/contracts/telegram-v2.md`. At the #6116 fold the crate
//! boundary survives while the internals swap onto the generic extension
//! runtime.

#![warn(unreachable_pub)]

pub mod state;
pub mod telegram_account_setup;
pub mod telegram_actor_identity;
pub mod telegram_adapter;
pub mod telegram_bot_api;
pub mod telegram_channel_routes;
pub mod telegram_connectable_channel;
pub mod telegram_dispatch;
pub mod telegram_egress;
pub mod telegram_manifest;
pub mod telegram_outbound_targets;
pub mod telegram_pairing;
pub mod telegram_serve;
pub mod telegram_setup;
#[cfg(test)]
mod test_support;

pub use telegram_account_setup::{
    TELEGRAM_EXTENSION_ID, TelegramHostBuildError, telegram_account_setup_descriptor,
};

//! Reborn product-layer runtime (Telegram v2).
//!
//! This module wires the `ironclaw_product_workflow` /
//! `ironclaw_product_adapters` stack into the running binary when
//! `REBORN_TELEGRAM_V2_ENABLED=true`. The v1 WASM Telegram channel
//! (under `crate::channels::wasm::*`) continues to coexist; the
//! exclusivity guard at `src/config/channels.rs:517` prevents both
//! paths from being active for the same install.

pub mod boot;
pub mod composition;
pub mod product_channel;
pub mod registry;
pub mod v2_inbound_turn;
pub mod v2_router;

pub use boot::{TELEGRAM_V2_CHANNEL_NAME, TelegramV2Bootstrap, bootstrap_telegram_v2};
pub use composition::{
    RebornProductRuntime, RebornProductRuntimeConfig, build_reborn_product_runtime,
};
pub use product_channel::{ProductChannel, ProductChannelConfig};
pub use registry::{RebornChannelWiringInputs, register_reborn_channels};
pub use v2_inbound_turn::V2InboundTurnService;
pub use v2_router::{TelegramV2RouterState, telegram_v2_routes};

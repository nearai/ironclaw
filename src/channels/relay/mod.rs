//! Channel-relay integration for connecting to external messaging platforms
//! (Slack) via the channel-relay service.
//!
//! The relay service handles OAuth, credential storage, and webhook ingestion.
//! IronClaw receives events via webhook callbacks and sends messages via the
//! relay's proxy API.

pub mod channel;
pub mod client;
pub mod webhook;

pub use channel::{DEFAULT_RELAY_NAME, RelayChannel};
pub use client::RelayClient;

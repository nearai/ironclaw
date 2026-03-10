//! Channel-relay integration for connecting to external messaging platforms
//! (Slack, Telegram, etc.) via the channel-relay service.
//!
//! The relay service handles OAuth, credential storage, webhook ingestion,
//! and SSE event streaming. IronClaw consumes the SSE stream and sends
//! messages via the relay's proxy API.

pub mod channel;
pub mod client;

pub use channel::{DEFAULT_RELAY_NAME, RelayChannel};
pub use client::RelayClient;

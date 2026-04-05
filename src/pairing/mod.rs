//! DM pairing for channels.
//!
//! Gates DMs from unknown senders. Only approved senders can message the agent.
//! Unknown senders receive a pairing code and can be claimed in the web UI or
//! approved via `ironclaw pairing approve`.
//!
//! OpenClaw reference: src/pairing/pairing-store.ts

mod code;
mod store;

pub use code::PairingCodeChallenge;
pub use store::PairingStore;

/// Canonical channel identifier used for pairing persistence and cache keys.
///
/// Channel names are internal ASCII-ish identifiers (`telegram`, `slack`,
/// etc.), so lowercasing keeps storage, cache, and lookup semantics aligned.
pub(crate) fn normalize_channel_name(channel: &str) -> String {
    channel.to_ascii_lowercase()
}

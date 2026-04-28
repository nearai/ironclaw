//! DM pairing for channels.
//!
//! Gates DMs from unknown senders. Only approved senders can message the agent.
//! Unknown senders receive a pairing code and can be claimed in the web UI or
//! approved via `ironclaw pairing approve`.
//!
//! OpenClaw reference: src/pairing/pairing-store.ts

pub mod approval;
mod code;
mod store;

pub use code::PairingCodeChallenge;
pub use store::PairingStore;

/// Typed wrapper for the external actor ID returned by pairing approval.
///
/// Represents a channel-specific external identifier (e.g. a Telegram user ID,
/// Discord user ID, Slack member ID). Prevents bare `String` confusion with
/// other string-typed identifiers like `owner_id` or `user_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalId(String);

impl ExternalId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ExternalId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::fmt::Display for ExternalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Canonical channel identifier used for pairing persistence and cache keys.
///
/// Channel names are internal ASCII-ish identifiers (`telegram`, `slack`,
/// etc.), so lowercasing keeps storage, cache, and lookup semantics aligned.
pub(crate) fn normalize_channel_name(channel: &str) -> String {
    channel.to_ascii_lowercase()
}

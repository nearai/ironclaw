//! Types for the Nostr tool.
//!
//! `NostrAction` is the tagged enum used for dispatch. `JsonSchema` is derived
//! so the advertised schema stays in sync with serde.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Input parameters for the Nostr tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum NostrAction {
    /// Publish a text note (kind 1).
    PublishNote {
        /// The note content (plaintext).
        content: String,
        /// Relays to publish to (defaults to a built-in list).
        #[serde(default)]
        relays: Vec<String>,
    },

    /// Get the profile metadata (kind 0) for a pubkey.
    GetProfile {
        /// Public key in hex or npub1... format.
        pubkey: String,
    },

    /// Update your own profile metadata (kind 0).
    SetProfile {
        /// Display name.
        #[serde(default)]
        name: Option<String>,
        /// About text (biography).
        #[serde(default)]
        about: Option<String>,
        /// Profile picture URL.
        #[serde(default)]
        picture: Option<String>,
        /// NIP-05 identifier (e.g. "user@domain.com").
        #[serde(default)]
        nip05: Option<String>,
        /// Website URL.
        #[serde(default)]
        website: Option<String>,
        /// Relays to publish to.
        #[serde(default)]
        relays: Vec<String>,
    },

    /// Search for notes by text query (uses nostr.band API).
    SearchNotes {
        /// Search query.
        query: String,
        /// Maximum results to return (default: 20).
        #[serde(default = "default_limit")]
        limit: u32,
    },

    /// Fetch recent notes from specific pubkeys.
    GetNotes {
        /// Pubkeys to fetch notes from (hex or npub1...).
        authors: Vec<String>,
        /// Maximum number of notes (default: 20).
        #[serde(default = "default_limit")]
        limit: u32,
    },

    /// Send an encrypted direct message (kind 4, NIP-04).
    SendDm {
        /// Recipient public key (hex or npub1...).
        recipient: String,
        /// Message content.
        content: String,
        /// Relays to publish to.
        #[serde(default)]
        relays: Vec<String>,
    },

    /// React to a note (kind 7).
    React {
        /// Event ID to react to.
        event_id: String,
        /// Author pubkey of the event being reacted to (for p-tag).
        event_pubkey: String,
        /// Reaction emoji (default: "+").
        #[serde(default = "default_reaction")]
        reaction: String,
        /// Relays to publish to.
        #[serde(default)]
        relays: Vec<String>,
    },

    /// Repost a note (kind 6).
    Repost {
        /// Event ID to repost.
        event_id: String,
        /// Author pubkey of the original event.
        event_pubkey: String,
        /// Relays to publish to.
        #[serde(default)]
        relays: Vec<String>,
    },

    /// Get your own public key.
    GetPubkey,

    /// Configure which relays to use by default (stored in workspace).
    SetRelays {
        /// List of relay URLs (e.g. "https://relay.damus.io").
        relays: Vec<String>,
    },

    /// Get current relay configuration.
    GetRelays,
}

fn default_limit() -> u32 {
    20
}
fn default_reaction() -> String {
    "+".to_string()
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// Result from publishing an event.
#[derive(Debug, Serialize)]
pub struct PublishResult {
    pub event_id: String,
    pub kind: u64,
    pub relay_response: String,
}

/// Profile metadata (kind 0 content).
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ProfileMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nip05: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
}

/// A fetched note.
#[derive(Debug, Serialize)]
pub struct NoteInfo {
    pub event_id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Vec<String>>,
}

/// Result from search.
#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub notes: Vec<NoteInfo>,
    pub count: usize,
}

/// Relay configuration.
#[derive(Debug, Serialize)]
pub struct RelayConfig {
    pub relays: Vec<String>,
}

/// Public key info.
#[derive(Debug, Serialize)]
pub struct PubkeyInfo {
    pub hex: String,
    pub npub: String,
}

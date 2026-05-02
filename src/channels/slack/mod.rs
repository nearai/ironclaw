//! Slack channel install + OAuth helpers.
//!
//! This module is the *control plane* for the Slack WASM channel
//! (`channels-src/slack/`). The WASM channel handles inbound Slack events
//! and outbound `chat.postMessage` calls; this module handles the one-time
//! workspace install dance — generating the Slack app manifest, parsing
//! the `oauth.v2.access` response that yields the bot token, and persisting
//! the resulting workspace identity.
//!
//! Subsequent commits on this branch land:
//!   * `/api/channels/slack/slash` — slash-command receiver
//!   * `channel_audit_log` — compliance trail for in/out messages
//!   * Per-user pairing on first DM (reuses [`crate::pairing`])
//!
//! Driven by NEAR Foundation pilot demand:
//! "slack is really would be the channel number one to be used (compliance)."
//! — Tobias Holenstein, 2026-05-01 NEAR AI all-hands.

pub mod manifest;
pub mod oauth;
pub mod sig;

pub use manifest::{MINIMAL_BOT_SCOPES, SlackManifest, generate_manifest};
pub use oauth::{OAuthV2AccessResponse, parse_oauth_v2_access};
pub use sig::{MAX_TIMESTAMP_SKEW_SECS, SignatureError, VerifyInputs, verify};

//! Adapter-generic machinery shared by IronClaw Reborn channel hosts.
//!
//! Every module here started life inside `ironclaw_reborn_composition`'s
//! Slack host and was made vendor-neutral when the Telegram host landed. It
//! now lives below composition so a channel host crate (e.g.
//! `ironclaw_telegram_extension`) and composition's own channel modules can
//! share one definition without a dependency cycle. Nothing in this crate
//! keys on a concrete channel; each host injects its own protocol details.
//!
//! At the #6116 fold this crate is absorbed by the generic extension runtime
//! alongside the per-channel host crates it serves.

#![warn(unreachable_pub)]

pub mod auth_continuation;
pub mod delivery_protocol;
pub mod host_ingress;
pub mod host_state_records;
pub mod identity;
pub mod outbound_targets;

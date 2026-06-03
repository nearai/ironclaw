//! Retired trusted ingress authority-token crate for IronClaw Reborn.
//!
//! Trusted trigger ingress is now owned inside `ironclaw_conversations`, which
//! keeps the trusted request constructor private. This crate intentionally
//! exposes no public authority token and must not regain production dependents.
#![warn(unreachable_pub)]

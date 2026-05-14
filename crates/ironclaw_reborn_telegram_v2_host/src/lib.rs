//! Standalone Reborn Telegram v2 webhook host.
//!
//! This crate is intentionally not linked into the v1 agent binary. It runs
//! as its own process via the `ironclaw-reborn-telegram-host` binary; the v1
//! agent has zero awareness it exists. See
//! `crates/ironclaw_reborn_telegram_v2_host/CLAUDE.md` for the architectural
//! rationale.
//!
//! Today this binary terminates inbound at the durable ledger / binding write
//! and acks 200 to Telegram. The reply path is intentionally stubbed: there
//! is no Reborn agent loop in `src/` yet (PRs #3544 / #3550 / #3586 open). The
//! tracer's purpose here is to lock down the inbound contract — webhook auth,
//! parse, idempotency, binding persistence, ledger settlement — so the swap-in
//! once the Reborn loop ships is a one-line change in `boot.rs`.

pub mod boot;
pub mod composition;
pub mod config;
pub mod error;
pub mod inbound_turn;
pub mod migrations;
pub mod router;

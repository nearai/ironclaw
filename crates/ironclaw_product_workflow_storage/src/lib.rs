//! Durable storage implementations for the Reborn product workflow.
//!
//! Backs the `IdempotencyLedger` port with libSQL and Postgres-native
//! implementations. The conversation-binding port is owned by the shared
//! `ProductConversationBindingService` in `ironclaw_product_workflow` (PR
//! #3727); its durable state lives in `ironclaw_conversations`'s filesystem
//! store over the unified-FS dispatch fabric (PR #3679).
//!
//! Schema migrations live in the consumer host crate
//! (`crates/ironclaw_reborn_telegram_v2_host/src/migrations.rs`).
//!
//! This crate intentionally does not own:
//! - Workflow orchestration (lives in `ironclaw_product_workflow`).
//! - Conversation binding / external-actor pairing (lives in
//!   `ironclaw_conversations` + the shared `ProductConversationBindingService`).
//! - Adapter contracts (live in `ironclaw_product_adapters`).
//! - Outbound delivery state (lives in `ironclaw_outbound`).
//!
//! Its single responsibility is mapping the workflow ledger port onto durable
//! rows plus the Telegram HTTP egress shim used by the runner.

#![forbid(unsafe_code)]

mod error;
#[cfg(any(feature = "libsql", feature = "postgres"))]
mod phase;
#[cfg(any(feature = "libsql", feature = "postgres"))]
mod recovery;

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub use recovery::DEFAULT_RECOVERY_LEASE;

mod egress;
#[cfg(feature = "libsql")]
mod ledger_libsql;
#[cfg(feature = "postgres")]
mod ledger_postgres;
mod outbound_sink;

pub use egress::{EgressCredentialResolver, StaticCredentialResolver, TelegramHttpEgress};
#[cfg(feature = "libsql")]
pub use ledger_libsql::LibSqlProductIdempotencyLedger;
#[cfg(feature = "postgres")]
pub use ledger_postgres::PostgresProductIdempotencyLedger;
pub use outbound_sink::OutboundStateStoreDeliverySink;

//! Durable storage implementations for the Reborn product workflow.
//!
//! Backs the `IdempotencyLedger` and `ConversationBindingService` ports with
//! libSQL and Postgres-native implementations. Schema migrations live in the
//! host crate (`src/db/libsql_migrations.rs` V26 and `migrations/V28_*.sql`).
//!
//! This crate intentionally does not own:
//! - Workflow orchestration (lives in `ironclaw_product_workflow`).
//! - Adapter contracts (live in `ironclaw_product_adapters`).
//! - Outbound delivery state (lives in `ironclaw_outbound`).
//!
//! Its single responsibility is mapping the workflow ports onto durable rows.

#![forbid(unsafe_code)]

mod error;
mod identifiers;

#[cfg(feature = "libsql")]
mod binding_libsql;
#[cfg(feature = "postgres")]
mod binding_postgres;
mod egress;
#[cfg(feature = "libsql")]
mod ledger_libsql;
#[cfg(feature = "postgres")]
mod ledger_postgres;
mod outbound_sink;

#[cfg(feature = "libsql")]
pub use binding_libsql::LibSqlConversationBindingService;
#[cfg(feature = "postgres")]
pub use binding_postgres::PostgresConversationBindingService;
pub use egress::{EgressCredentialResolver, StaticCredentialResolver, TelegramHttpEgress};
#[cfg(feature = "libsql")]
pub use ledger_libsql::LibSqlProductIdempotencyLedger;
#[cfg(feature = "postgres")]
pub use ledger_postgres::PostgresProductIdempotencyLedger;
pub use outbound_sink::OutboundStateStoreDeliverySink;

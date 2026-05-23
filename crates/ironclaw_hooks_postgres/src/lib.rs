//! Durable PostgreSQL-backed [`PredicateStateBackend`].
//!
//! This is durable-backend PR 2/4 in the predicate-state split. It
//! implements the *exact same* trait contract as the in-memory backend
//! ([`ironclaw_hooks::predicate_state::InMemoryPredicateStateBackend`])
//! and is proven against the shared contract harness (see
//! `tests/predicate_state_postgres_contract.rs`). All eight contract
//! functions plus adversarial multi-host tests run against this impl.
//!
//! # Why a separate crate
//!
//! The trait was widened to `pub` in PR 1/4 specifically so durable
//! backends live *out of crate* and depend on `ironclaw_hooks` with the
//! `contract-tests` feature — exactly the way
//! [`ironclaw_reborn_event_store`] is a separate per-domain durable crate
//! rather than living in `ironclaw_events`. Keeping the Postgres
//! dependency surface out of `ironclaw_hooks` keeps the hook framework
//! itself DB-free.
//!
//! [`PredicateStateBackend`]:
//!     ironclaw_hooks::predicate_state::PredicateStateBackend
//! [`ironclaw_reborn_event_store`]: https://docs.rs/ironclaw_reborn_event_store

#[cfg(feature = "postgres")]
mod backend;
#[cfg(feature = "postgres")]
mod hashing;
#[cfg(feature = "postgres")]
mod schema;

#[cfg(feature = "postgres")]
pub use backend::PostgresPredicateStateBackend;
#[cfg(feature = "postgres")]
pub use schema::POSTGRES_PREDICATE_SCHEMA;

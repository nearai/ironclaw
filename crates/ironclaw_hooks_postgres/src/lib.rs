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
//! # Dual-backend compliance (libSQL counterpart)
//!
//! The repo rule "new persistence must support both PostgreSQL and
//! libSQL" is satisfied across the staged durable-backend series, not in
//! this crate alone. This crate is the **Postgres** half; the **libSQL**
//! counterpart lives in the sibling crate `ironclaw_hooks_libsql` (PR
//! #3936), and behavioral interchangeability between the two is enforced
//! by the cross-backend parity suite `ironclaw_hooks_parity` (PR #3937),
//! which runs the identical contract assertions against both backends.
//! Merge ordering: this crate (PR 2/4, #3933) lands first, then the
//! libSQL crate (#3936), then the parity suite (#3937) gates that the two
//! durable backends are drop-in equivalent. Both backends share the one
//! logical two-table typed schema (see `migrations/V1__predicate_state.sql`);
//! only the storage column types differ (Postgres TIMESTAMPTZ + NUMERIC
//! vs. libSQL epoch-ms INTEGER + TEXT).
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

/// Test-only accessors for the crate-internal bucket hashing, used by the
/// adversarial integration tests to compute the `key_hash` / `scope_hash`
/// bytes a bucket maps to so a test can query rows directly. Not part of the
/// public API surface (this crate is `publish = false`); kept out of the
/// rendered docs. Production code must never depend on this module.
#[cfg(feature = "postgres")]
#[doc(hidden)]
pub mod test_support {
    use ironclaw_hooks::predicate_state::InvocationKey;

    /// `key_hash` bytes for an invocation bucket — see
    /// `crate::hashing::invocation_key_hash`.
    pub fn invocation_key_hash_bytes(key: &InvocationKey) -> [u8; 32] {
        crate::hashing::invocation_key_hash(key)
    }

    /// `scope_hash` bytes for a tenant — see `crate::hashing::scope_hash`.
    pub fn scope_hash_bytes(tenant_id: &str) -> [u8; 32] {
        crate::hashing::scope_hash(tenant_id)
    }
}

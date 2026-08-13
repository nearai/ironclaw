//! Durable libSQL-backed [`PredicateStateBackend`].
//!
//! This is the libSQL sibling of the in-memory backend that ships in the same
//! crate. `ironclaw_hooks` owns the public [`PredicateStateBackend`] trait and
//! its supporting types; this module provides a concrete durable
//! implementation. It — and the `libsql` dependency it pulls in — is gated
//! behind the crate's `libsql` feature so the framework stays DB-free by
//! default. (It previously lived in a standalone `ironclaw_hooks_libsql` crate;
//! the native-SQL atomic counter is unchanged by the fold-in.)
//!
//! [`PredicateStateBackend`]: crate::predicate_state::PredicateStateBackend
//!
//! # Public surface
//!
//! - [`LibSqlPredicateStateBackend`] — the durable backend impl.
//!
//! Everything else (schema, scope hashing, transaction helpers) is internal.
//!
//! See [`backend`] for the implementation notes (schema, atomicity,
//! fail-closed cap enforcement, clock basis).

mod backend;
mod hashing;
mod schema;

pub use backend::LibSqlPredicateStateBackend;

/// Test-only accessors for the crate-internal bucket hashing, used by the
/// contract/parity tests to compute the `scope_hash` a tenant maps to so a
/// test can query rows directly by the canonical tenant grain (the raw
/// `tenant_id` is no longer a column). Not part of the public API surface
/// (this crate is `publish = false`); kept out of the rendered docs.
/// Production code must never depend on this module.
#[doc(hidden)]
pub mod test_support {
    use crate::predicate_hash::{self, Digest};
    use crate::predicate_state::InvocationKey;

    /// `scope_hash` (tenant digest) bytes — see
    /// [`crate::predicate_hash::scope_hash`]. Returns the canonical
    /// fixed-width [`Digest`] (`[u8; 32]`) so the accessor is byte-for-byte
    /// symmetric with the Postgres sibling's `scope_hash_bytes`; the libSQL
    /// `BLOB` param binding accepts a `&[u8]` slice of it without a `.to_vec()`
    /// asymmetry at the call sites (henrypark133 LOW on PR #3937). Delegates
    /// directly to the canonical shared hash rather than the crate-internal
    /// `Vec<u8>` wrapper.
    pub fn scope_hash_bytes(tenant_id: &str) -> Digest {
        predicate_hash::scope_hash(tenant_id)
    }

    /// `key_hash` bytes for an invocation bucket — see
    /// [`crate::predicate_hash::invocation_key_hash`]. Mirrors the
    /// Postgres sibling's `invocation_key_hash_bytes` so a parity test querying
    /// by key hash has a parallel accessor on both backends.
    pub fn invocation_key_hash_bytes(key: &InvocationKey) -> Digest {
        predicate_hash::invocation_key_hash(key)
    }
}

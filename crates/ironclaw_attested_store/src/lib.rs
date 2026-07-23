//! Durable PostgreSQL + libSQL backends for the attested-signing stores.
//!
//! This is **PR12 of the attested-signing stack** (see
//! `docs/plans/2026-05-23-attested-signing-substrate.md`). The substrate's
//! authorization, anti-replay, and broadcast-idempotency guards live in three
//! stores defined one layer down:
//!
//! * [`ironclaw_attestation::SealedGrantStore`] — one-shot signing grant; the
//!   anti-double-sign guard.
//! * [`ironclaw_attestation::SigningLedger`] — per-`gate_ref` broadcast
//!   idempotency state machine.
//! * [`ironclaw_attested_runtime::AttestedGateBindingStore`] — the
//!   authoritative `(context, hash, decoded tx, schema)` binding the resume
//!   path verifies against.
//!
//! PR1–PR11 ran them in-memory. This crate makes them production-durable on
//! BOTH backends and — crucially — enforces every load-bearing invariant **at
//! the database level**, not via application-side read-modify-write:
//!
//! * **Sealed-grant one-shot claim** is a single conditional
//!   `UPDATE ... WHERE status = 'sealed'`; the row count decides the winner, so
//!   concurrent claims (and a `Stuck -> InProgress` job-recovery double-claim)
//!   resolve to exactly one success at the DB.
//! * **Ledger create** is an `INSERT` against a `gate_ref` primary key — a
//!   duplicate is a unique-violation, i.e. one-shot create.
//! * **Ledger advance** is a conditional `UPDATE ... WHERE state = <from>` after
//!   the in-memory transition check, so the broadcast-idempotency guard
//!   (`BroadcastSubmitted` only moves to a terminal) holds even under a
//!   concurrent recovery attempt.
//!
//! Per the workspace "LLM data is never deleted" rule, no store ever `DELETE`s
//! a row — claims and transitions are marked with timestamps in place.
//!
//! ## Dual-backend invariant
//!
//! Every store has a `Postgres*` impl (behind `feature = "postgres"`) and a
//! `LibSql*` impl (behind `feature = "libsql"`), with byte-identical schemas
//! and identical observable semantics. Both are driven through the SAME
//! `*_contract_cases!` suites from `ironclaw_attestation` (behind
//! `feature = "contract-suite"` + `feature = "integration"`). Never add a
//! single-backend persistence path.

#![warn(unreachable_pub)]
#![forbid(unsafe_code)]

mod binding;
#[cfg(feature = "broadcast-http")]
mod broadcaster;
mod error;
mod grant;
mod ledger;

pub use error::StoreError;

#[cfg(feature = "broadcast-http")]
pub use broadcaster::{ChainRpcEndpoints, MultiChainBroadcaster};

#[cfg(feature = "postgres")]
pub use binding::PostgresAttestedGateBindingStore;
#[cfg(feature = "postgres")]
pub use grant::PostgresSealedGrantStore;
#[cfg(feature = "postgres")]
pub use ledger::PostgresSigningLedger;

#[cfg(feature = "libsql")]
pub use binding::LibSqlAttestedGateBindingStore;
#[cfg(feature = "libsql")]
pub use grant::LibSqlSealedGrantStore;
#[cfg(feature = "libsql")]
pub use ledger::LibSqlSigningLedger;

/// Stable composite primary-key hash for a [`ironclaw_attestation::GrantKey`].
///
/// The seven key components are stored individually for audit/forensics, but
/// the primary key is the lowercase-hex SHA-256 of a domain-separated,
/// length-prefixed concatenation of those components. Length-prefixing each
/// component prevents boundary-collision smuggling (e.g. `a||bc` vs `ab||c`).
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn grant_key_hash(key: &ironclaw_attestation::GrantKey) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"ironclaw.attested_store.grant_key.v1");
    let mut field = |bytes: &[u8]| {
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    };
    field(key.tenant.as_str().as_bytes());
    field(key.user.as_str().as_bytes());
    field(key.run_id.as_str().as_bytes());
    field(key.gate_ref.as_str().as_bytes());
    field(key.approved_tx_hash.as_bytes());
    field(key.key_or_account_id.as_str().as_bytes());
    field(key.chain_id.as_str().as_bytes());
    hex::encode(hasher.finalize())
}

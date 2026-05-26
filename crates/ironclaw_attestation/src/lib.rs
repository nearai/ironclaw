//! Canonical signing-bytes + [`ApprovedTxHash`] core for the IronClaw
//! attested-signing substrate.
//!
//! This is **PR2 of a 10-PR stack** (see
//! `docs/plans/2026-05-23-attested-signing-substrate.md`). It defines the
//! value-binding core: the chain-tagged, chain-SDK-FREE
//! [`DecodedTransaction`] model, the [`render`] function that derives the
//! human-facing view, the [`canonical_signing_bytes`] encoder, and
//! [`approved_tx_hash_for`] which binds them — together with an explicit,
//! trusted signer/account — into the [`ApprovedTxHash`] from
//! `ironclaw_signing_provider`.
//!
//! ## Purity invariant
//!
//! This crate depends ONLY on `ironclaw_signing_provider`, `serde`,
//! `thiserror`, and `sha2`. It carries **no chain SDK** (no `solana-sdk`,
//! `near-*`, `alloy`), **no secrets**, and **no webauthn** — those land in
//! PR4/PR6. The architecture boundary test
//! (`crates/ironclaw_architecture/tests/attested_signing_boundaries.rs`)
//! enforces this.
//!
//! ## Anti-field-smuggling guarantee
//!
//! The renderer and the canonical encoder both derive from the single
//! [`crate::fields::project`] projection, so the human-approved view and the
//! signed bytes can never diverge. [`approved_tx_hash_for`] then binds
//! render ∥ canonical bytes ∥ signer/account ∥ chain/network ∥ tx-type ∥
//! schema-version: changing ANY component changes the hash. Because it derives
//! both the render and the canonical bytes from the SAME decoded transaction, a
//! caller cannot mismatch a render of tx A with the canonical bytes of tx B.
#![warn(unreachable_pub)]
#![forbid(unsafe_code)]

mod approved_tx_hash;
mod canonical;
mod decoded_tx;
mod fields;
mod rendered;

mod wire;

pub use approved_tx_hash::approved_tx_hash_for;
pub use canonical::canonical_signing_bytes;
pub use decoded_tx::{
    Bytes32, DecodedTransaction, EvmAccessListEntry, EvmAddress, EvmTransaction, NearAccessKey,
    NearAccessKeyPermission, NearAction, NearPublicKey, NearTransaction, RenderingSchemaVersion,
    SolanaAddressTableLookup, SolanaCompiledInstruction, SolanaMessageHeader, SolanaMessageVersion,
    SolanaTransaction,
};
pub use rendered::{RenderedField, RenderedTx, render};

/// Test-only re-export of the low-level component hasher.
///
/// Production code must use [`approved_tx_hash_for`] (which derives render and
/// canonical bytes from the same decoded transaction). The low-level
/// component-wise hasher is exposed ONLY under the internal `test-internals`
/// feature so the binding test-suite can exercise per-component tampering
/// (e.g. "same render, different canonical bytes ⇒ different hash"). It is not
/// part of the public API and must never be enabled by production dependents.
#[cfg(feature = "test-internals")]
pub use approved_tx_hash::compute_approved_tx_hash;

// Re-export the binding hash type so downstream PRs import it from the
// attestation crate alongside the functions that produce it.
pub use ironclaw_signing_provider::{APPROVED_TX_HASH_LEN, ApprovedTxHash};

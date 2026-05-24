//! Canonical signing-bytes + [`ApprovedTxHash`] core for the IronClaw
//! attested-signing substrate.
//!
//! This is **PR2 of a 10-PR stack** (see
//! `docs/plans/2026-05-23-attested-signing-substrate.md`). It defines the
//! value-binding core: the chain-tagged, chain-SDK-FREE
//! [`DecodedTransaction`] model, the [`render`] function that derives the
//! human-facing view, the [`canonical_signing_bytes`] encoder, and
//! [`compute_approved_tx_hash`] which binds them into the
//! [`ApprovedTxHash`] from `ironclaw_signing_provider`.
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
//! signed bytes can never diverge. [`compute_approved_tx_hash`] then binds
//! render ∥ canonical bytes ∥ signer/account ∥ chain/network ∥ tx-type ∥
//! schema-version: changing ANY component changes the hash.
#![warn(unreachable_pub)]
#![forbid(unsafe_code)]

mod approved_tx_hash;
mod canonical;
mod decoded_tx;
mod fields;
mod rendered;

pub use approved_tx_hash::compute_approved_tx_hash;
pub use canonical::canonical_signing_bytes;
pub use decoded_tx::{
    Bytes32, DecodedTransaction, EvmAccessListEntry, EvmAddress, EvmTransaction, NearAction,
    NearTransaction, RenderingSchemaVersion, SolanaInstruction, SolanaTransaction,
};
pub use rendered::{RenderedField, RenderedTx, render};

// Re-export the binding hash type so downstream PRs import it from the
// attestation crate alongside the functions that produce it.
pub use ironclaw_signing_provider::{APPROVED_TX_HASH_LEN, ApprovedTxHash};

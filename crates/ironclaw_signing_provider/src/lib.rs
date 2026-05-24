//! Provider-agnostic signing abstraction for the IronClaw attested-signing
//! substrate.
//!
//! This is **PR1 of a 10-PR stack** (see
//! `docs/plans/2026-05-23-attested-signing-substrate.md`). It pins the binding
//! model that every downstream crate depends on: the [`SigningProvider`] trait
//! and the value types it names.
//!
//! ## Purity invariant
//!
//! This crate carries **zero chain or crypto dependencies**. It must never
//! depend on `solana-sdk`, `near-*`, `alloy*`, `k256`, `sha3`, `webauthn-rs`,
//! `ironclaw_secrets`, `ironclaw_chain_signing`, or `ironclaw_attestation`.
//! The concrete implementations of the opaque types declared here
//! ([`DecodedTransaction`], [`RenderedTx`], [`ApprovedTxHash`], and the
//! [`SigningProof`] payloads) land in `ironclaw_attestation` (PR2) and the
//! chain crates. At this layer they are forward-declared markers / opaque byte
//! payloads so the trait can name them without pulling chain code.
//!
//! The architecture dependency-boundary test
//! (`crates/ironclaw_architecture/tests/`) enforces this purity.
#![warn(unreachable_pub)]
#![forbid(unsafe_code)]

mod context;
mod error;
mod proof;
mod provider;
mod transaction;

pub use context::{
    ActorId, ChainId, GateRef, KeyOrAccountId, RunId, ScopeId, SigningContext, TenantId, UserId,
};
pub use error::SigningProviderError;
pub use proof::{SigningProof, VerifiedProof};
pub use provider::{InitiationOutcome, ProviderId, SigningProvider, TrustModel};
pub use transaction::{APPROVED_TX_HASH_LEN, ApprovedTxHash, DecodedTransaction, RenderedTx};

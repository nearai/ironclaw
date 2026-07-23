//! Custodial multi-chain signing for the IronClaw attested-signing substrate.
//!
//! This is **PR6 of the 10-PR attested-signing stack** (see
//! `docs/plans/2026-05-23-attested-signing-substrate.md`). It turns a resolved
//! attestation + a persisted [`ironclaw_attestation::DecodedTransaction`] into a
//! signed, broadcast transaction, behind two independent enforcement points:
//!
//! 1. **Grant claim** — the signer refuses to act without claiming the sealed
//!    one-shot [`ironclaw_attestation::AttestedSigningGrant`] (PR3); a replayed
//!    approval cannot be turned into a second signature.
//! 2. **Sign-time approved-tx-hash re-check** — the signer recomputes the
//!    [`ironclaw_signing_provider::ApprovedTxHash`] *from the persisted decoded
//!    transaction* and refuses (before any key access) if it diverges from the
//!    approved hash.
//!
//! The [`ironclaw_attestation::SigningLedger`] (PR3) provides broadcast
//! idempotency: a gate_ref past `BroadcastSubmitted` can never re-enter signing.
//!
//! ## Custody, the KMS sign-digest path & the ship-gate
//!
//! Hot (in-process) chain private keys are SECRETS, encrypted with
//! [`ironclaw_secrets::SecretsCrypto`] under the
//! [`ironclaw_secrets::chain_key_aad`] domain (added in this PR) and used ONLY
//! for testnet/dev. Real-value / mainnet signing is routed through a sign-only
//! [`kms::KmsSigner`] in which **no private-key bytes ever enter this process**:
//! the key lives in the KMS/HSM and only an opaque key reference + the digest
//! cross the boundary. The [`kms::ShipGate`] refuses mainnet signing unless such
//! a secure-custody backend is wired, and returns the REQUIRED
//! [`kms::SigningPath`] so a hot key can never service a mainnet request
//! (compromised-host hot-key threat #18). [`kms::LocalKmsSigner`] is an in-tree
//! software-HSM reference backend (secp256k1 + ed25519) that exercises and tests
//! the full key-ref path without a cloud account; a concrete cloud backend (AWS
//! KMS / GCP KMS / YubiHSM — note AWS KMS supports secp256k1 but NOT ed25519) is
//! a separately-approved follow-up. Durable PG/libSQL keystore/grant/ledger
//! backends are likewise deferred.
//!
//! ## Per-chain layout
//!
//! [`evm`], [`solana`], and [`near`] each carry `decode` / `render` / `sign` /
//! `broadcast` / `policy`. `render` delegates to PR2's shared field projection
//! so the human-approved view and the signed bytes cannot diverge.
//!
//! All three chains sign bytes derived SOLELY from the persisted decoded
//! transaction the approved hash was computed over (review findings #1 / #4),
//! never a separate caller-supplied payload. EVM signs the keccak signing hash
//! of the transaction RECONSTRUCTED from the decoded projection
//! ([`evm::decode`]'s `rebuild_signable`) with a mandatory ecrecover binding
//! check. Solana and NEAR sign a 32-byte sha256 commitment over PR2's shared
//! [`ironclaw_attestation::canonical_signing_bytes`] (the single source of truth
//! the approved hash binds), using the vendored `ed25519-dalek` so the heavy
//! `solana-sdk` / `near-primitives` SDKs are not pulled; producing directly
//! broadcastable on-wire bytes (Solana `VersionedMessage` with ALT resolution,
//! NEAR `Transaction` borsh) is the immediate next slice — flagged here and in
//! the PR body. The raw key-consumption and signing primitives are `pub(crate)`
//! (review finding #5): only the grant/hash/ledger/ship-gate-enforcing
//! [`CustodialSigner`] facade is public.
//!
//! ## Open questions (injectable, deny-first)
//!
//! First-key bootstrap trust anchor, key rotation, and custody recovery/backup
//! are open governance questions; they are surfaced as injectable
//! [`policy::BootstrapPolicy`] / [`policy::KeyCustodyPolicy`] hooks with
//! conservative deny-first defaults rather than hardcoded answers.
#![warn(unreachable_pub)]
#![forbid(unsafe_code)]

mod chain;
mod custodial;
mod error;
mod keystore;
mod kms;
mod policy;

/// SHA-256 over a byte slice. Used to form the 32-byte ed25519 signing digest
/// (a commitment over the shared canonical signing bytes) for Solana/NEAR, so
/// the same digest is signed on both the hot-key and the digest-oriented KMS
/// path.
pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

pub mod evm;
pub mod near;
pub mod solana;

pub use chain::{ChainFamily, ChainKeyId};
pub use custodial::{
    CustodialSignOutcome, CustodialSignRequest, CustodialSigner, recompute_approved_hash,
};
pub use error::{ChainSigningError, Result};
pub use keystore::{ChainKeyBinding, ConsumedChainKey, KeyStore, KeyStoreError, SecretsKeyStore};
pub use kms::{
    HsmKmsBackend, KmsSigner, LocalKmsSigner, ShipGate, SignatureAlg, SigningPath, ValueClass,
};
pub use policy::{
    AllowBootstrapPolicy, BootstrapPolicy, CustodyDecision, DenyFirstBootstrapPolicy,
    DenyFirstCustodyPolicy, KeyCustodyPolicy,
};

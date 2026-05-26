//! External-wallet signing providers for the IronClaw attested-signing
//! substrate.
//!
//! This crate spans **PR7 and PR8 of a 10-PR stack** (see
//! `docs/plans/2026-05-23-attested-signing-substrate.md`). PR7 implements the
//! **browser injected provider** backend — `window.ethereum` (EVM) and
//! `window.solana` (Solana). PR8 adds the **NEAR browser-wallet redirect
//! provider** ([`NearRedirectSigningProvider`]): the user is redirected to a
//! NEAR wallet that signs the bound approved-tx hash with the account's ed25519
//! access key and redirects back with the signature. In every case the wallet
//! holds the keys and renders + signs natively (true wallet-side WYSIWYS);
//! IronClaw never has custody.
//!
//! ## Trust model
//!
//! [`InjectedSigningProvider`] reports
//! [`TrustModel::ExternalWallet`](ironclaw_signing_provider::TrustModel::ExternalWallet)
//! and [`ProviderId::Injected`](ironclaw_signing_provider::ProviderId::Injected).
//! It holds no key material.
//!
//! ## Security core: [`InjectedSigningProvider::verify_resume`]
//!
//! The injected wallet attests to the *bound* [`ApprovedTxHash`] — the
//! WYSIWYS digest IronClaw rendered and the wallet's UI mirrors — by signing
//! over its raw 32 bytes. `verify_resume` enforces, fail-closed, in order:
//!
//! 1. **Hash binding (threat #3):** the hash carried in the proof must equal
//!    the bound [`ApprovedTxHash`] the gate persisted. A caller cannot smuggle
//!    a different approved hash.
//! 2. **Signer binding (threat #5):** the signer recovered (EVM ecrecover via
//!    `k256`) / verified (Solana ed25519 via `ed25519-dalek`) from the
//!    signature over the bound hash must equal the account bound into the
//!    [`SigningContext`]. A mismatch is
//!    [`SigningProviderError::SignerMismatch`].
//! 3. **One-shot grant (threat #1):** the sealed [`AttestedSigningGrant`] is
//!    claimed via the atomic CAS
//!    ([`SealedGrantStore::claim`](ironclaw_attestation::SealedGrantStore::claim)).
//!    A replay — a second resume of an already-claimed grant — fails closed.
//!
//! Only when all three pass does it return
//! [`VerifiedProof`](ironclaw_signing_provider::VerifiedProof).
//!
//! ## Scope boundary (PR7 vs PR10)
//!
//! `verify_resume` stops at the verified-proof boundary. Broadcasting the
//! wallet-signed transaction (through `ironclaw_chain_signing`, PR6) and the
//! full deterministic-continuation composition land in PR10. This crate has no
//! `ironclaw_chain_signing` dependency.
//!
//! ## Dependency boundary
//!
//! May depend on `k256` / `ed25519-dalek` / `sha3` / `sha2` / `base64` /
//! `ironclaw_signing_provider` / `ironclaw_attestation` — but NOT on
//! `solana-sdk`, `near-primitives`, or `ironclaw_secrets` (it holds no keys).
//! The architecture boundary test
//! (`crates/ironclaw_architecture/tests/attested_signing_boundaries.rs`)
//! enforces this.
#![warn(unreachable_pub)]
#![forbid(unsafe_code)]

mod injected;
mod near_redirect;

pub use injected::{
    InjectedProofPayload, InjectedScheme, InjectedSigningProvider, decode_injected_proof,
    encode_injected_proof,
};
pub use near_redirect::{
    NearAccessKeyScope, NearBoundOperation, NearRedirectProofPayload, NearRedirectSigningProvider,
    NearRedirectState, decode_near_redirect_proof, decode_state, derive_state,
    encode_near_redirect_proof, encode_state, verify_state,
};

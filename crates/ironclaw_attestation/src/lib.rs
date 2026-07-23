//! Canonical signing-bytes + [`ApprovedTxHash`] core for the IronClaw
//! attested-signing substrate.
//!
//! This is **PR3 of a 10-PR stack** (see
//! `docs/plans/2026-05-23-attested-signing-substrate.md`). It builds on the
//! PR2 value-binding core: the chain-tagged, chain-SDK-FREE
//! [`DecodedTransaction`] model, the [`render`] function that derives the
//! human-facing view, the [`canonical_signing_bytes`] encoder, and
//! [`approved_tx_hash_for`] which binds them — together with an explicit,
//! trusted signer/account — into the [`ApprovedTxHash`] from
//! `ironclaw_signing_provider`.
//!
//! ## Layering invariant
//!
//! This crate depends on `ironclaw_signing_provider`, `serde`/`serde_json`,
//! `thiserror`, `sha2`, `async-trait`, and — as of PR4 — the pure-Rust,
//! openssl-free WebAuthn crypto trio (`coset` for COSE_Key CBOR, `p256` for
//! ES256, `ed25519-dalek` for EdDSA; NOT `webauthn-rs-core`, which would link
//! `openssl`). It still carries **no chain SDK** (no `solana-sdk`,
//! `near-*`, `alloy`), **no EVM crypto primitives** (`k256`/`sha3`), and **no
//! key custody** (`ironclaw_secrets` / `ironclaw_chain_signing`) — the custody
//! keys and per-chain decode/sign/broadcast land in PR6. The architecture
//! boundary test
//! (`crates/ironclaw_architecture/tests/attested_signing_boundaries.rs`)
//! enforces this.
//!
//! ## PR4 additions
//!
//! - [`challenge`]: the durable one-shot [`ChallengeStore`] + the
//!   [`ChallengePreimage`] that binds a challenge to the exact operation.
//! - [`webauthn`]: the [`WebAuthnCredentialRegistry`] and
//!   [`verify_assertion`] full RP-validation verifier (UV-required,
//!   challenge-echo, rpIdHash, origin, signCount-regression, BE/BS).
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
mod challenge;
mod decoded_tx;
mod error;
mod fields;
mod rendered;
mod webauthn;

mod wire;

// `grant` and `ledger` are private by default — their public types are
// re-exported below. Under the `contract-tests` feature they are made public
// so out-of-crate durable-backend crates can reach the canonical contract
// suites at `ironclaw_attestation::grant::contract` /
// `ironclaw_attestation::ledger::contract` (the `#[macro_export]`ed
// `*_contract_cases!` macros expand to `$crate::grant::contract::...` paths).
#[cfg(not(feature = "contract-tests"))]
mod grant;
#[cfg(feature = "contract-tests")]
pub mod grant;
#[cfg(not(feature = "contract-tests"))]
mod ledger;
#[cfg(feature = "contract-tests")]
pub mod ledger;

pub use approved_tx_hash::approved_tx_hash_for;
pub use canonical::canonical_signing_bytes;
pub use challenge::{
    ChallengeCommitment, ChallengeError, ChallengeId, ChallengePreimage, ChallengeStore,
    ConsumedChallenge, CredentialId, DeliveryAttemptId, InMemoryChallengeStore, IssuedChallenge,
};
pub use decoded_tx::{
    Bytes32, DecodedTransaction, EvmAccessListEntry, EvmAddress, EvmTransaction, NearAccessKey,
    NearAccessKeyPermission, NearAction, NearPublicKey, NearTransaction, RenderingSchemaVersion,
    SolanaAddressTableLookup, SolanaCompiledInstruction, SolanaMessageHeader, SolanaMessageVersion,
    SolanaTransaction,
};
pub use error::AttestationError;
pub use grant::{
    AttestedSigningGrant, ClaimedGrant, GrantError, GrantKey, GrantStatus,
    InMemorySealedGrantStore, SealedGrantStore,
};
pub use ledger::{
    InMemorySigningLedger, LedgerError, LedgerKey, SigningLedger, SigningLedgerState,
};
pub use rendered::{RenderedField, RenderedTx, render};
pub use webauthn::{
    Aaguid, AssertionInput, AttestationPolicy, BackupFlagPolicy, BootstrapPolicy, CoseError,
    CosePublicKey, InMemoryWebAuthnCredentialRegistry, OriginContext, OriginPolicy,
    RegisteredCredential, RegistrationError, RegistrationRequest, SignCountPolicy,
    StandardOriginPolicy, VerificationError, VerifiedAssertion, WebAuthnCredentialRegistry,
    verify_assertion,
};

/// Test-only re-export of the low-level component hasher.
///
/// Production code must use [`approved_tx_hash_for`] (which derives render and
/// canonical bytes from the same decoded transaction). The low-level
/// component-wise hasher is exposed ONLY under the internal `test-support`
/// feature so the binding test-suite can exercise per-component tampering
/// (e.g. "same render, different canonical bytes ⇒ different hash"). It is not
/// part of the public API and must never be enabled by production dependents.
#[cfg(feature = "test-support")]
pub use approved_tx_hash::compute_approved_tx_hash;

// Re-export the binding hash type so downstream PRs import it from the
// attestation crate alongside the functions that produce it.
pub use ironclaw_signing_provider::{APPROVED_TX_HASH_LEN, ApprovedTxHash};

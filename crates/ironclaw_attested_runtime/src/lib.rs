//! Composition-layer runtime glue for the IronClaw attested-signing substrate.
//!
//! This is **PR10 of the 10-PR attested-signing stack** (see
//! `docs/plans/2026-05-23-attested-signing-substrate.md`). It is the
//! *composition glue* the binary-boundary rule requires to live outside `src/`:
//! it is the single place where the crypto-free turn store
//! ([`ironclaw_turns`]), the provider-agnostic trait
//! ([`ironclaw_signing_provider`]), the external-wallet providers
//! ([`ironclaw_wallet_external`]), and the custodial chain signer
//! ([`ironclaw_chain_signing`]) are wired together.
//!
//! It ships three deliverables:
//!
//! 1. [`RuntimeAttestedResumePort`] — the production
//!    [`ironclaw_turns::AttestedResumePort`] implementation. It runs inside the
//!    turn store's synchronous resume critical section, so it is strictly
//!    non-blocking: it re-checks the persisted gate binding against the
//!    `expected_tx_hash` the gate was raised with and claims a synchronous
//!    one-shot **resume guard** (threats #1 / #16 at the resume boundary). The
//!    heavyweight async work (provider `verify_resume`, the authoritative
//!    sealed-grant CAS, and the chain sign + broadcast) happens *after* the
//!    store transitions `BlockedAttested -> AttestedResolved`, in the
//!    [`AttestedSignerContinuationDriver`].
//!
//! 2. [`AttestedSignerContinuationDriver`] — drives the deterministic
//!    post-approval continuation once the turn reaches
//!    [`ironclaw_turns::TurnStatus::AttestedResolved`]: routes to the correct
//!    [`ironclaw_signing_provider::SigningProvider`] (or the custodial chain
//!    signer) to verify the proof + claim the sealed grant, then performs the
//!    real sign + broadcast honoring the broadcast-idempotency
//!    [`ironclaw_attestation::SigningLedger`]. It NEVER re-enters the agent loop
//!    (threat #16) and NEVER re-broadcasts a `gate_ref` already past
//!    `BroadcastSubmitted` (threats #6 / #7).
//!
//! 3. [`CustodialMainnetShipGate`] — the `CUSTODIAL_MAINNET_ENABLED` env gate
//!    (mirroring the `HOOKS_THIRD_PARTY_ENABLED` ship-gate pattern). It builds
//!    the chain-signing [`ironclaw_chain_signing::ShipGate`] from the operator
//!    opt-in and an optionally-wired KMS backend, refusing real-value /
//!    mainnet custodial signing unless secure custody is wired (threat #18).
//!
//! ## Boundary invariants
//!
//! * `ironclaw_turns` stays crypto-free: this crate depends on `ironclaw_turns`
//!   but never the reverse. All chain/crypto convergence happens *here*, at the
//!   composition layer, which is the legitimate place for it.
//! * This crate is a library outside `src/`; it carries no dependency on the
//!   binary.
#![warn(unreachable_pub)]
#![forbid(unsafe_code)]

mod binding;
mod driver;
mod port;
mod ship_gate;
mod trust;

pub use trust::{
    AlwaysTrustNearAccessKeyVerifier, BindingKey, BindingStatus, EnrollmentState,
    InMemoryTrustStore, NearAccessKeyVerifier, NonceSource, SignedChallenge, TrustChallenge,
    TrustEnrollment, TrustError, TrustKind, TrustRegistrar, TrustStore, TrustedSignerBinding,
    VerifiedControl,
};

pub use binding::{
    AttestedGateBinding, AttestedGateBindingStore, BindingError, InMemoryAttestedGateBindingStore,
    SyncBindingRead, validate_binding,
};
pub use driver::{
    AttestedSignerContinuationDriver, BroadcastDisposition, BroadcastOutcome, Broadcaster,
    ContinuationError, CustodialSignerLike, EvmSignable, ProviderRegistry, RebuildError,
    SignerContinuationOutcome, VerifiedContinuation,
};
pub use port::{
    InMemoryResumeGuard, ResumeGuard, RuntimeAttestedResumePort, approved_tx_hash_ref_hex,
};
pub use ship_gate::{CUSTODIAL_MAINNET_ENABLED_ENV, CustodialMainnetShipGate};

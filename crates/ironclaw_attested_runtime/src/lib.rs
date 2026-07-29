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

mod alpaca;
mod alpaca_supervisor;
mod alpaca_uds;
mod binding;
mod clear_signing;
#[cfg(feature = "clear-signing-http")]
mod clear_signing_http;
mod device_signature;
mod driver;
mod intent_signer;
mod port;
mod ship_gate;
mod trust;

#[cfg(any(test, feature = "unsafe-always-trust-near"))]
pub use trust::AlwaysTrustNearAccessKeyVerifier;
pub use trust::{
    BindingKey as TrustBindingKey, BindingStatus, CsprngNonceSource, EnrollmentState,
    InMemoryTrustStore, NearAccessKeyVerifier, NonceSource, SignedChallenge, TrustChallenge,
    TrustEnrollment, TrustError, TrustKind, TrustRegistrar, TrustStore, TrustedSignerBinding,
    VerifiedControl,
};

pub use alpaca::{
    AlpacaError, AlpacaPort, BroadcastRequest, CombineRequest, CraftRequest, CurrencyId,
    RecordingAlpacaPort, SharedAlpacaPort, UnconfiguredAlpacaPort,
};
pub use alpaca_supervisor::{
    AlpacaConfigError, AlpacaDeployment, AlpacaSupervisor, RestartBackoff, SOCKET_PATH_MAX,
    SidecarSpawnSpec, mint_sidecar_token, port_for,
};
pub use alpaca_uds::UdsAlpacaPort;
pub use binding::{
    AttestedGateBinding, AttestedGateBindingStore, BindingError, BindingKey,
    InMemoryAttestedGateBindingStore, SyncBindingRead, validate_binding, validate_binding_key,
};
pub use clear_signing::{
    DescriptorKey, DescriptorLookup, DescriptorSource, TtlDescriptorCache,
    UnconfiguredDescriptorSource,
};
#[cfg(feature = "clear-signing-http")]
pub use clear_signing_http::{
    ALLOWED_UPSTREAM_HOSTS, CLEAR_SIGNING_UPSTREAM_ENV, HttpDescriptorSource, LEDGER_CAL_BASE_URL,
    UpstreamConfigError, validate_upstream,
};
pub use device_signature::{DeviceSignatureError, signable_digest, verify_device_signature};
pub use driver::{
    AttestedSignerContinuationDriver, BindingOwner, BroadcastDisposition, BroadcastOutcome,
    Broadcaster, ContinuationError, CustodialSignerLike, EvmSignable, ProviderRegistry,
    RebuildError, SignerContinuationOutcome, VerifiedContinuation,
};
pub use intent_signer::{
    InMemorySealedAgentKeyStore, SealedAgentKey, SealedAgentKeyStore, SecretsIntentSigner,
};
pub use port::{
    InMemoryResumeGuard, ResumeGuard, RuntimeAttestedResumePort, approved_tx_hash_ref_hex,
};
pub use ship_gate::{CUSTODIAL_MAINNET_ENABLED_ENV, CustodialMainnetShipGate};

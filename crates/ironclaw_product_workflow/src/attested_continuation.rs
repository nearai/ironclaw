//! Crypto-free attested-signing continuation port for the WebUI facade
//! (attested-signing PR11).
//!
//! `ironclaw_product_workflow` is a product-facing facade crate that must stay
//! crypto-free: it never names a chain SDK, a signing provider, a sealed grant,
//! or a broadcast ledger. But the WebUI `resolve_gate` path needs to drive the
//! deterministic sign + broadcast continuation once a `BlockedAttested` gate has
//! been resolved to `AttestedResolved`.
//!
//! The bridge is this injected port. The facade (atomic verify-before-resume,
//! PR11 item B):
//!
//! 1. Translates the browser-supplied attested-proof resolution into an opaque
//!    [`AttestedProofClaim`] (all fields are strings / JSON — no crypto types).
//! 2. Calls [`AttestedGateContinuationPort::verify_and_claim`] BEFORE touching
//!    the turn store. This runs the FULL cryptographic verification (real
//!    signature recovery / WebAuthn assertion) AND claims the one-shot sealed
//!    grant. On ANY failure the turn is left `BlockedAttested` with zero
//!    state-machine mutation — the facade never calls `resume_turn`.
//! 3. Only on success, builds a `ResumeTurnRequest { attestation: Some(..) }`
//!    whose [`ironclaw_turns::AttestationClaimRef`] is the proof's bound-hash
//!    claim, and calls `resume_turn`. The injected `AttestedResumePort` (wired
//!    in the composition layer, outside `src/`) runs the synchronous binding
//!    re-check + one-shot resume guard (defense in depth — it does NOT re-claim)
//!    and transitions the turn to `AttestedResolved`.
//! 4. Calls [`AttestedGateContinuationPort::broadcast_resolved`] with the
//!    [`VerifiedAttestedContinuation`] handle from step 2 to drive the
//!    sign-output broadcast. No re-verification, no re-claim.
//!
//! The production implementation lives in `ironclaw_reborn_composition` over
//! `ironclaw_attested_runtime`'s driver; this crate declares only the
//! crypto-free contract and the opaque DTOs/handle. Mirrors how the turn store
//! already takes an injected `AttestedResumePort`.

use std::any::Any;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use ironclaw_turns::{GateRef, TurnRunId, TurnScope};

/// The proof family carried on an attested gate resolution. Mirrors the legacy
/// monolith `GateResolutionPayload` variants for wire compatibility; the
/// composition-layer port maps each kind onto the matching
/// `ironclaw_signing_provider::SigningProof` it knows how to verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttestedProofKind {
    /// Browser injected wallet (`window.ethereum` / `window.solana`).
    InjectedWallet,
    /// NEAR wallet redirect callback proof.
    NearRedirect,
    /// WalletConnect v2 session proof.
    WalletConnect,
}

impl AttestedProofKind {
    /// Sanitized, snake_case category for diagnostics and error mapping.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InjectedWallet => "injected_wallet",
            Self::NearRedirect => "near_redirect",
            Self::WalletConnect => "wallet_connect",
        }
    }
}

/// The opaque attested-proof claim the facade forwards to the continuation port.
///
/// Every field is a string or JSON value: this crate confers no trust and holds
/// no crypto type. The composition-layer port re-decodes `proof_json` into the
/// concrete provider proof and verifies it against the authoritative gate
/// binding (which it persisted when the gate was raised — never trusting these
/// caller-supplied fields to *define* the binding, only to attest to it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedProofClaim {
    /// Which proof family this claim belongs to.
    pub kind: AttestedProofKind,
    /// Lowercase-hex of the approved-tx hash the wallet attests to. This becomes
    /// the `AttestationClaimRef` on the resume request, so the synchronous
    /// resume-port binding re-check can reject a claim that does not even name
    /// the bound hash before any async verification runs.
    pub approved_tx_hash_hex: String,
    /// The opaque, provider-specific proof payload (signature, signer, scheme,
    /// public key, scope, state echo, …). Re-decoded by the port; never
    /// interpreted here.
    pub proof_json: serde_json::Value,
}

/// Sanitized outcome of a continuation. Carries no chain, signer, or ledger
/// internals beyond the public broadcast attribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedContinuationOutcome {
    /// Public signer/account the broadcast was attributed to.
    pub signer: String,
}

/// Sanitized rejection taxonomy for an attested continuation. Mirrors the
/// crypto-free spirit of [`ironclaw_turns::AttestedResumeRejection`]: categories
/// only, no ceremony detail.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttestedContinuationRejection {
    /// No authoritative binding exists for the resolved gate (it was never
    /// raised, or the binding store lost it).
    MissingBinding,
    /// The proof family or its provider did not match the bound provider.
    ProviderMismatch,
    /// The provider rejected the proof (signer/hash mismatch, grant-claim
    /// failure, scope violation), or the custodial signer failed.
    ProofRejected,
    /// A broadcast-idempotency / ledger guard refused the transition (e.g. the
    /// gate was already broadcast).
    LedgerGuard,
    /// The proof payload was malformed and could not be decoded.
    MalformedProof,
    /// The continuation port is not wired on this deployment.
    Unavailable,
    /// An infrastructure/runtime backend failed (chain-signing backend error,
    /// broadcast/RPC outage). This is a service-health failure, not a client
    /// input failure, so it must surface as a retryable 503 rather than a 400.
    BackendUnavailable,
}

impl AttestedContinuationRejection {
    /// Sanitized, snake_case category for diagnostics and error mapping.
    pub fn category(&self) -> &'static str {
        match self {
            Self::MissingBinding => "attested_missing_binding",
            Self::ProviderMismatch => "attested_provider_mismatch",
            Self::ProofRejected => "attested_proof_rejected",
            Self::LedgerGuard => "attested_ledger_guard",
            Self::MalformedProof => "attested_malformed_proof",
            Self::Unavailable => "attested_unavailable",
            Self::BackendUnavailable => "attested_backend_unavailable",
        }
    }
}

/// Opaque, crypto-free handle proving that [`AttestedGateContinuationPort::verify_and_claim`]
/// ran successfully: the proof's full signature was verified and the one-shot
/// sealed grant was claimed. The facade holds it between `verify_and_claim` and
/// [`AttestedGateContinuationPort::broadcast_resolved`] without inspecting it —
/// the composition-layer implementation downcasts it back to its concrete
/// verified-continuation type. This crate confers no trust and names no crypto
/// type; the handle is just an opaque token.
pub struct VerifiedAttestedContinuation {
    inner: Box<dyn Any + Send>,
}

impl VerifiedAttestedContinuation {
    /// Wrap a composition-layer verified continuation as an opaque handle.
    pub fn new<T: Any + Send>(inner: T) -> Self {
        Self {
            inner: Box::new(inner),
        }
    }

    /// Recover the concrete verified continuation. Returns the boxed value back
    /// on a type mismatch so the caller can fail closed rather than panic.
    pub fn downcast<T: Any + Send>(self) -> Result<Box<T>, VerifiedAttestedContinuation> {
        match self.inner.downcast::<T>() {
            Ok(value) => Ok(value),
            Err(inner) => Err(Self { inner }),
        }
    }
}

impl std::fmt::Debug for VerifiedAttestedContinuation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedAttestedContinuation").finish()
    }
}

/// Injected, crypto-free continuation port for attested-signing gate resolution.
///
/// Implementations live outside this crate (composition / reborn layer). The
/// facade (atomic verify-before-resume, PR11 item B) calls
/// [`Self::verify_and_claim`] BEFORE `resume_turn` and
/// [`Self::broadcast_resolved`] AFTER. The full cryptographic verification and
/// the one-shot sealed-grant claim run in `verify_and_claim`, so they gate the
/// `BlockedAttested -> AttestedResolved` transition; the broadcast half never
/// re-verifies or re-claims.
#[async_trait]
pub trait AttestedGateContinuationPort: Send + Sync {
    /// Run the FULL cryptographic verification (real signature recovery /
    /// WebAuthn assertion) AND claim the one-shot sealed grant for the resolved
    /// gate — all BEFORE the turn transitions. On success returns an opaque
    /// [`VerifiedAttestedContinuation`] handle the facade passes back to
    /// [`Self::broadcast_resolved`] after `resume_turn`.
    ///
    /// On ANY failure (malformed/forged proof, signer/hash mismatch, provider
    /// mismatch, grant already claimed, missing binding) it returns a sanitized
    /// rejection and MUST leave the turn `BlockedAttested` with NO
    /// run/mission/gate state-machine mutation: the facade never calls
    /// `resume_turn` for a claim that fails here. (A failed claim may have
    /// advanced the implementation's own ledger/grant fail-closed state; that is
    /// internal one-shot bookkeeping, never a turn-state transition, and means a
    /// retry of the same gate is refused rather than double-driven.)
    async fn verify_and_claim(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: &GateRef,
        claim: &AttestedProofClaim,
    ) -> Result<VerifiedAttestedContinuation, AttestedContinuationRejection>;

    /// Drive the sign-output broadcast for a gate whose proof was already
    /// verified + grant-claimed in [`Self::verify_and_claim`]. Consumes the
    /// opaque handle; performs NO re-verification and NO re-claim.
    async fn broadcast_resolved(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: &GateRef,
        verified: VerifiedAttestedContinuation,
    ) -> Result<AttestedContinuationOutcome, AttestedContinuationRejection>;
}

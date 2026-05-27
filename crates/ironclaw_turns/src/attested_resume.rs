//! Injected verification port for resuming an attested-signing gate.
//!
//! `ironclaw_turns` is the crypto-free turn-coordination contract crate. It
//! never verifies attestation claims, performs chain I/O, or links the signer.
//! When a `BlockedAttested` gate is resumed, the store runs its flat
//! same-thread checks (scope, status, actor, gate-ref match), requires the
//! untrusted attestation claim to be present, and then delegates the actual
//! verification to an [`AttestedResumePort`] supplied at store construction.
//!
//! The production implementation lives outside this crate (composition /
//! reborn layer), mirroring how the store already takes an injected admission
//! limit provider. This module declares only the crypto-free signature and a
//! rejection taxonomy; it deliberately ships no verifying implementation.

use crate::{ApprovedTxHashRef, AttestationClaimRef, GateRef};

/// Inputs handed to the port when validating an attested-signing resume.
///
/// All three values are opaque to turns: the gate reference and the expected
/// transaction-hash binding were persisted when the gate was raised, and the
/// attestation claim is the untrusted wire value carried on the resume request.
#[derive(Debug, Clone, Copy)]
pub struct AttestedResumeRequest<'a> {
    /// The gate reference that the resume request must match.
    pub gate_ref: &'a GateRef,
    /// The untrusted attestation claim carried on the resume request.
    pub attestation: &'a AttestationClaimRef,
    /// The opaque expected-transaction-hash binding persisted when the gate was
    /// raised. The port is responsible for binding `attestation` to this value.
    pub expected_tx_hash: &'a ApprovedTxHashRef,
}

/// Injected, crypto-free verification port for attested-signing resumes.
///
/// Implementations live outside `ironclaw_turns`. The store calls this exactly
/// once per `BlockedAttested` resume, after its flat checks pass and after it
/// has confirmed the attestation claim is present. A returned `Ok(())` means
/// the run may transition to [`crate::TurnStatus::AttestedResolved`]; any
/// [`AttestedResumeRejection`] leaves the run blocked.
pub trait AttestedResumePort: Send + Sync {
    /// Verify that `request.attestation` validly binds to
    /// `request.expected_tx_hash` for `request.gate_ref`.
    fn verify_attested_resume(
        &self,
        request: AttestedResumeRequest<'_>,
    ) -> Result<(), AttestedResumeRejection>;
}

/// Why an attested-signing resume was rejected by the port.
///
/// Variants are sanitized categories: they carry no chain, WebAuthn, or secret
/// detail, only enough to map to a turn error without leaking ceremony
/// internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttestedResumeRejection {
    /// The attestation claim did not bind to the expected transaction hash.
    BindingMismatch,
    /// The attestation claim was malformed or otherwise unverifiable.
    InvalidClaim,
    /// The attestation evidence was rejected by the verifier (e.g. expired,
    /// replayed, or untrusted issuer).
    EvidenceRejected,
}

impl AttestedResumeRejection {
    /// Sanitized, snake_case category for diagnostics and error mapping.
    pub fn category(self) -> &'static str {
        match self {
            Self::BindingMismatch => "attested_binding_mismatch",
            Self::InvalidClaim => "attested_invalid_claim",
            Self::EvidenceRejected => "attested_evidence_rejected",
        }
    }
}

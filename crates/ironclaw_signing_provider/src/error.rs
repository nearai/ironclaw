//! The error type returned by [`crate::SigningProvider`] operations.

use thiserror::Error;

/// Errors a signing provider can surface during initiation or resume.
///
/// These are the fail-closed outcomes named by the threat matrix in
/// `docs/plans/2026-05-23-attested-signing-substrate.md`: a mismatched signer,
/// a grant that could not be claimed one-shot, an invalid proof, or a scope
/// violation. Providers map their own internal failures onto these variants so
/// the resume path can reason about them uniformly.
#[derive(Debug, Error)]
pub enum SigningProviderError {
    /// The signer / account recovered from the proof did not match the account
    /// bound into the request (e.g. EVM `from` recovered via ecrecover differs
    /// from the bound account).
    #[error("signer mismatch: recovered signer does not match the bound account")]
    SignerMismatch,

    /// The sealed one-shot signing grant could not be claimed (already
    /// consumed, missing, or lost the atomic CAS race).
    #[error("grant claim failed: the one-shot signing grant could not be claimed")]
    GrantClaimFailed,

    /// The supplied proof failed verification (bad signature, failed WebAuthn
    /// RP checks, malformed payload).
    #[error("proof invalid: {reason}")]
    ProofInvalid {
        /// Human-readable reason the proof was rejected.
        reason: String,
    },

    /// The requested operation exceeds the approved / granted scope (e.g. WC
    /// scope escalation, NEAR access-key scope mismatch).
    #[error("scope violation: {reason}")]
    ScopeViolation {
        /// Human-readable description of the scope that was violated.
        reason: String,
    },

    /// A provider-internal failure that does not map onto a more specific
    /// variant. Carries an opaque description.
    #[error("provider error: {reason}")]
    Provider {
        /// Human-readable description of the provider-internal failure.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_variants_construct_and_render() {
        assert!(
            SigningProviderError::SignerMismatch
                .to_string()
                .contains("signer mismatch")
        );
        assert!(
            SigningProviderError::GrantClaimFailed
                .to_string()
                .contains("grant claim failed")
        );
        assert!(
            SigningProviderError::ProofInvalid {
                reason: "bad sig".to_string(),
            }
            .to_string()
            .contains("bad sig")
        );
        assert!(
            SigningProviderError::ScopeViolation {
                reason: "scope too broad".to_string(),
            }
            .to_string()
            .contains("scope too broad")
        );
        assert!(
            SigningProviderError::Provider {
                reason: "boom".to_string(),
            }
            .to_string()
            .contains("boom")
        );
    }
}

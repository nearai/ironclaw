//! Per-provider proof shapes and the verified-proof marker.
//!
//! At this layer proof bodies are **opaque byte payloads** — the trait crate
//! does not interpret them. The verifiers that turn a [`SigningProof`] into a
//! [`VerifiedProof`] live with each provider implementation (PR7–PR9) and the
//! WebAuthn verifier in `ironclaw_attestation` (PR4).

use serde::{Deserialize, Serialize};

/// A provider-specific signing proof, carried back from the wallet / authn
/// ceremony to the resume path.
///
/// Each variant wraps an opaque payload whose concrete shape is owned by the
/// provider that produces it. Keeping the bytes opaque here means the trait
/// crate stays chain/crypto-free while still typing *which kind* of proof a
/// resume carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum SigningProof {
    /// Proof from a WalletConnect v2 session (PR9).
    WalletConnectProof(Vec<u8>),
    /// Proof from a browser injected provider (`window.ethereum` /
    /// `window.solana`, PR7).
    InjectedProof(Vec<u8>),
    /// Proof from the NEAR browser-wallet redirect protocol (PR8).
    NearRedirectProof(Vec<u8>),
    /// A WebAuthn assertion authorizing a custodial signing (PR4).
    WebAuthnAssertionProof(Vec<u8>),
}

impl SigningProof {
    /// Borrow the opaque proof payload regardless of variant.
    pub fn payload(&self) -> &[u8] {
        match self {
            Self::WalletConnectProof(bytes)
            | Self::InjectedProof(bytes)
            | Self::NearRedirectProof(bytes)
            | Self::WebAuthnAssertionProof(bytes) => bytes,
        }
    }
}

/// A [`SigningProof`] that a provider's verifier has validated.
///
/// Construction is intentionally gated through [`VerifiedProof::new`] so that a
/// `VerifiedProof` value is evidence that verification *ran*. The actual
/// verification logic (signature recovery, WebAuthn RP checks, scope checks)
/// lives in the provider / attestation crates downstream; this type is the
/// trait-level token they return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedProof {
    proof: SigningProof,
}

impl VerifiedProof {
    /// Wrap a proof that a downstream verifier has accepted.
    pub fn new(proof: SigningProof) -> Self {
        Self { proof }
    }

    /// Borrow the underlying verified proof.
    pub fn proof(&self) -> &SigningProof {
        &self.proof
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_proof_uses_snake_case_wire_tags() {
        let cases = [
            (
                SigningProof::WalletConnectProof(vec![1]),
                "wallet_connect_proof",
            ),
            (SigningProof::InjectedProof(vec![2]), "injected_proof"),
            (
                SigningProof::NearRedirectProof(vec![3]),
                "near_redirect_proof",
            ),
            (
                SigningProof::WebAuthnAssertionProof(vec![4]),
                "web_authn_assertion_proof",
            ),
        ];
        for (proof, expected_tag) in cases {
            let json = serde_json::to_string(&proof).expect("serialize");
            assert!(
                json.contains(&format!("\"kind\":\"{expected_tag}\"")),
                "expected snake_case tag `{expected_tag}` in {json}"
            );
            let back: SigningProof = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, proof);
        }
    }

    #[test]
    fn payload_accessor_returns_inner_bytes_for_every_variant() {
        assert_eq!(SigningProof::InjectedProof(vec![9, 9]).payload(), &[9, 9]);
        assert_eq!(
            SigningProof::WebAuthnAssertionProof(vec![1]).payload(),
            &[1]
        );
    }

    #[test]
    fn verified_proof_wraps_and_round_trips() {
        let verified = VerifiedProof::new(SigningProof::InjectedProof(vec![5, 6]));
        assert_eq!(verified.proof().payload(), &[5, 6]);
        let json = serde_json::to_string(&verified).expect("serialize");
        let back: VerifiedProof = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, verified);
    }
}

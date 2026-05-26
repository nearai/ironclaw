//! Per-provider proof shapes and the verified-proof marker.
//!
//! At this layer proof bodies are **opaque byte payloads** — the trait crate
//! does not interpret them. The verifiers that turn a [`SigningProof`] into a
//! [`VerifiedProof`] live with each provider implementation (PR7–PR9) and the
//! WebAuthn verifier in `ironclaw_attestation` (PR4).

use serde::{Deserialize, Serialize};

use crate::{ApprovedTxHash, ProviderId};

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
/// A `VerifiedProof` is a *trust token*: its mere existence is evidence that a
/// provider's verifier ran and accepted the proof against a specific binding.
/// To keep that guarantee meaningful, the type is deliberately
/// **un-deserializable** — it derives [`Serialize`] but not [`Deserialize`], so
/// an untrusted payload can never be rehydrated into a "verified" value off the
/// wire. The only way to obtain one is [`VerifiedProof::new`], called by a
/// provider's verifier after [`crate::SigningProvider::verify_resume`] succeeds.
///
/// It also binds the [`ProviderId`] and [`ApprovedTxHash`] that were checked, so
/// a verified proof cannot be silently re-pointed at a different provider or
/// transaction. The actual verification logic (signature recovery, WebAuthn RP
/// checks, scope checks) lives in the provider / attestation crates downstream;
/// this type is the trait-level token they return.
///
/// The following must not compile — `VerifiedProof` is intentionally not
/// `Deserialize`, so it cannot be forged from an untrusted payload:
///
/// ```compile_fail
/// use ironclaw_signing_provider::VerifiedProof;
///
/// fn requires_deserialize<'de, T: serde::Deserialize<'de>>() {}
/// requires_deserialize::<VerifiedProof>();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerifiedProof {
    provider_id: ProviderId,
    approved_tx_hash: ApprovedTxHash,
    proof: SigningProof,
}

impl VerifiedProof {
    /// Wrap a proof that a downstream verifier has accepted.
    ///
    /// Call this only from a provider's verifier *after* it has cryptographically
    /// validated `proof` against `approved_tx_hash` and the signing context.
    /// Constructing a `VerifiedProof` asserts that verification succeeded.
    pub fn new(
        provider_id: ProviderId,
        approved_tx_hash: ApprovedTxHash,
        proof: SigningProof,
    ) -> Self {
        Self {
            provider_id,
            approved_tx_hash,
            proof,
        }
    }

    /// The provider identity whose verifier accepted the proof.
    pub fn provider_id(&self) -> ProviderId {
        self.provider_id
    }

    /// The approved transaction hash the proof was checked against.
    pub fn approved_tx_hash(&self) -> &ApprovedTxHash {
        &self.approved_tx_hash
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
        assert_eq!(
            SigningProof::WalletConnectProof(vec![7, 7]).payload(),
            &[7, 7]
        );
        assert_eq!(SigningProof::InjectedProof(vec![9, 9]).payload(), &[9, 9]);
        assert_eq!(
            SigningProof::NearRedirectProof(vec![3, 3]).payload(),
            &[3, 3]
        );
        assert_eq!(
            SigningProof::WebAuthnAssertionProof(vec![1]).payload(),
            &[1]
        );
    }

    #[test]
    fn verified_proof_serializes_binding_but_is_not_deserializable() {
        let approved_tx_hash = ApprovedTxHash::from_bytes([9u8; 32]);
        let proof = SigningProof::InjectedProof(vec![5, 6]);
        let verified = VerifiedProof::new(ProviderId::Injected, approved_tx_hash, proof.clone());

        // Accessors expose the bound identity, hash, and proof.
        assert_eq!(verified.provider_id(), ProviderId::Injected);
        assert_eq!(verified.approved_tx_hash(), &approved_tx_hash);
        assert_eq!(verified.proof(), &proof);

        // It serializes (audit / observability), binding provider + hash + proof.
        let json = serde_json::to_value(&verified).expect("serialize");
        assert_eq!(json["provider_id"], "injected");
        assert_eq!(
            json["approved_tx_hash"],
            serde_json::to_value(approved_tx_hash).expect("serialize hash")
        );
        assert_eq!(json["proof"]["kind"], "injected_proof");

        // It is NOT deserializable — the `compile_fail` doctest on the type locks
        // in that a `VerifiedProof` can never be rehydrated from an untrusted
        // payload, so the only way to obtain one is through a verifier calling
        // `VerifiedProof::new`.
    }
}

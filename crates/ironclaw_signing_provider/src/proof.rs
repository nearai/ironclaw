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
/// # What this type does and does not guarantee
///
/// A `VerifiedProof` is a *provenance* token, not a proof of correctness. Be
/// precise about the difference, because the gap is where a false sense of
/// safety would live:
///
/// * **Guaranteed:** the value was minted by code holding a
///   [`SigningProvider`], and its [`ProviderId`] is that provider's own
///   identity. It cannot be rehydrated from the wire — the type derives
///   [`Serialize`] but deliberately not [`Deserialize`] — and it cannot be
///   minted by the driver, composition, or product layers, which hold no
///   provider.
/// * **NOT guaranteed:** that any cryptographic check actually ran. The
///   [`SigningProvider`] trait *requires* implementors to produce this type, so
///   a buggy or hostile provider can return one having checked nothing. No type
///   signature can close that: the trust boundary for provider *conduct* is
///   registration (which providers the registry admits), not construction.
///
/// So: treat a `VerifiedProof` as "provider P asserts this proof is good for
/// this hash", never as "this proof is good". The driver relies on the former
/// and re-reads the authoritative binding for everything else.
///
/// The identity is taken from the verifier rather than passed beside it, so a
/// provider cannot stamp another provider's [`ProviderId`] onto a proof it
/// minted — the mismatch is unrepresentable rather than merely discouraged.
///
/// The actual verification logic (signature recovery, WebAuthn RP checks, scope
/// checks) lives in the provider / attestation crates downstream; this type is
/// the trait-level token they return.
///
/// `VerifiedProof` is intentionally not `Deserialize`, so it cannot be forged
/// from an untrusted payload:
///
/// ```compile_fail
/// use ironclaw_signing_provider::VerifiedProof;
///
/// fn requires_deserialize<'de, T: serde::Deserialize<'de>>() {}
/// requires_deserialize::<VerifiedProof>();
/// ```
///
/// Nor can it be minted without a provider in hand, which keeps every layer
/// downstream of the registry from manufacturing its own trust token:
///
/// ```compile_fail
/// use ironclaw_signing_provider::{ApprovedTxHash, ProviderId, SigningProof, VerifiedProof};
///
/// // No `SigningProvider` here — there is no constructor to reach.
/// let _ = VerifiedProof::new(
///     ProviderId::Injected,
///     ApprovedTxHash::from_bytes([0u8; 32]),
///     SigningProof::InjectedProof(vec![]),
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerifiedProof {
    provider_id: ProviderId,
    approved_tx_hash: ApprovedTxHash,
    proof: SigningProof,
}

impl VerifiedProof {
    /// Wrap a proof that `verifier` has accepted.
    ///
    /// Call this only from a provider's own
    /// [`verify_resume`](crate::SigningProvider::verify_resume), *after* it has
    /// cryptographically validated `proof` against `approved_tx_hash` and the
    /// signing context — pass `self` as `verifier`.
    ///
    /// Taking the provider by reference is what keeps non-provider code from
    /// minting a trust token, and taking the [`ProviderId`] from
    /// [`SigningProvider::provider_id`] rather than as a separate argument is
    /// what makes a mis-stamped identity unrepresentable.
    pub fn new<P>(verifier: &P, approved_tx_hash: ApprovedTxHash, proof: SigningProof) -> Self
    where
        P: crate::SigningProvider + ?Sized,
    {
        Self {
            provider_id: verifier.provider_id(),
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

    /// A verifier that reports one identity. Used to prove the minted proof
    /// takes its `provider_id` from the verifier itself.
    struct StubVerifier {
        id: ProviderId,
    }

    #[async_trait::async_trait]
    impl crate::SigningProvider for StubVerifier {
        fn provider_id(&self) -> ProviderId {
            self.id
        }
        fn trust_model(&self) -> crate::TrustModel {
            crate::TrustModel::ExternalWallet
        }
        async fn initiate(
            &self,
            _context: &crate::SigningContext,
            _decoded: &crate::transaction::DecodedTransaction,
            _rendered: &crate::transaction::RenderedTx,
            _approved_tx_hash: &ApprovedTxHash,
        ) -> Result<crate::InitiationOutcome, crate::SigningProviderError> {
            Ok(crate::InitiationOutcome::ReadyForProof)
        }
        async fn verify_resume(
            &self,
            _context: &crate::SigningContext,
            approved_tx_hash: &ApprovedTxHash,
            proof: &SigningProof,
        ) -> Result<VerifiedProof, crate::SigningProviderError> {
            Ok(VerifiedProof::new(self, *approved_tx_hash, proof.clone()))
        }
    }

    #[test]
    fn verified_proof_serializes_binding_but_is_not_deserializable() {
        let approved_tx_hash = ApprovedTxHash::from_bytes([9u8; 32]);
        let proof = SigningProof::InjectedProof(vec![5, 6]);
        let verifier = StubVerifier {
            id: ProviderId::Injected,
        };
        let verified = VerifiedProof::new(&verifier, approved_tx_hash, proof.clone());

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

        // It is NOT deserializable, and cannot be minted without a provider —
        // both locked in by the `compile_fail` doctests on the type.
    }

    /// The minted identity comes from the verifier, never from a caller-supplied
    /// argument. This is what makes "provider A mints a proof stamped provider
    /// B" unrepresentable rather than merely discouraged: there is no argument
    /// left to disagree with `provider_id()`.
    #[test]
    fn verified_proof_identity_is_taken_from_the_minting_verifier() {
        let approved_tx_hash = ApprovedTxHash::from_bytes([4u8; 32]);
        let proof = SigningProof::WalletConnectProof(vec![1]);

        for id in [
            ProviderId::Injected,
            ProviderId::NearRedirect,
            ProviderId::WalletConnect,
            ProviderId::Custodial,
        ] {
            let verified =
                VerifiedProof::new(&StubVerifier { id }, approved_tx_hash, proof.clone());
            assert_eq!(
                verified.provider_id(),
                id,
                "the minted proof must carry the minting verifier's own identity"
            );
        }
    }
}

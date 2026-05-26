//! NEAR redirect-proof ed25519 signature verification.
//!
//! A NEAR wallet attests to the bound [`ApprovedTxHash`] by signing over the raw
//! 32 hash bytes with the account's ed25519 access key, producing a 64-byte
//! signature. We verify it with the vendored `ed25519-dalek` (no
//! `near-primitives` / `near-crypto`). Binding the verifying key to the bound
//! NEAR account happens in the caller (the account id is the bound identity;
//! the public key authenticates the signature under it).

use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use ironclaw_signing_provider::SigningProviderError;

/// Verify a 64-byte ed25519 `signature` over the 32 `hash_bytes` against the
/// 32-byte ed25519 `public_key`. Fails closed on any malformed input or
/// verification failure.
pub(super) fn verify_signature_over_hash(
    hash_bytes: &[u8; 32],
    signature: &[u8],
    public_key: &[u8],
) -> Result<(), SigningProviderError> {
    let pk_bytes: [u8; 32] =
        public_key
            .try_into()
            .map_err(|_| SigningProviderError::ProofInvalid {
                reason: format!("near public key must be 32 bytes, got {}", public_key.len()),
            })?;
    let sig_bytes: [u8; 64] =
        signature
            .try_into()
            .map_err(|_| SigningProviderError::ProofInvalid {
                reason: format!("near signature must be 64 bytes, got {}", signature.len()),
            })?;

    let verifying_key =
        VerifyingKey::from_bytes(&pk_bytes).map_err(|e| SigningProviderError::ProofInvalid {
            reason: format!("invalid near ed25519 public key: {e}"),
        })?;
    let sig = Signature::from_bytes(&sig_bytes);

    verifying_key
        .verify(hash_bytes, &sig)
        .map_err(|e| SigningProviderError::ProofInvalid {
            reason: format!("near ed25519 verification failed: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn valid_signature_verifies() {
        let key = SigningKey::from_bytes(&[0x44u8; 32]);
        let pk = key.verifying_key().to_bytes();
        let hash = [9u8; 32];
        let sig = key.sign(&hash).to_bytes();
        assert!(verify_signature_over_hash(&hash, &sig, &pk).is_ok());
    }

    #[test]
    fn signature_over_other_message_fails() {
        let key = SigningKey::from_bytes(&[0x44u8; 32]);
        let pk = key.verifying_key().to_bytes();
        let sig = key.sign(&[0u8; 32]).to_bytes();
        let err =
            verify_signature_over_hash(&[9u8; 32], &sig, &pk).expect_err("wrong message must fail");
        assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
    }

    #[test]
    fn wrong_length_public_key_fails() {
        let err = verify_signature_over_hash(&[0u8; 32], &[0u8; 64], &[0u8; 31])
            .expect_err("short pubkey");
        assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
    }

    #[test]
    fn wrong_length_signature_fails() {
        let err =
            verify_signature_over_hash(&[0u8; 32], &[0u8; 63], &[0u8; 32]).expect_err("short sig");
        assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
    }
}

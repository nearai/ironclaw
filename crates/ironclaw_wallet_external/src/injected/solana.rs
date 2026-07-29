//! Solana (`window.solana`) injected-proof signature verification.
//!
//! An injected Solana wallet attests to the bound [`ApprovedTxHash`] via
//! `signMessage` over the raw 32 hash bytes, producing a 64-byte ed25519
//! signature. We verify the signature against the connected wallet's ed25519
//! public key and require that public key to equal the bound account (threat
//! #5) via the shared [`crate::verify`] kernel — no `solana-sdk`.

use ironclaw_signing_provider::SigningProviderError;

use crate::verify::verify_ed25519_signer_over_digest;

/// Verify `signature` over `hash_bytes` against `public_key`, and require
/// `public_key` to equal `bound_account`.
///
/// `bound_account` is the lowercase hex of the 32-byte ed25519 public key.
pub(super) fn verify_signer_over_hash(
    hash_bytes: &[u8; 32],
    signature: &[u8],
    public_key: &[u8],
    bound_account: &str,
) -> Result<(), SigningProviderError> {
    verify_ed25519_signer_over_digest(hash_bytes, signature, public_key, bound_account)
}

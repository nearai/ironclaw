//! EVM (`window.ethereum`) injected-proof signer recovery.
//!
//! An injected EVM wallet attests to the bound [`ApprovedTxHash`] via
//! `personal_sign` over the raw 32 hash bytes. `personal_sign` applies the
//! EIP-191 personal-message prefix and keccak256, so the recoverable digest is
//! `keccak256("\x19Ethereum Signed Message:\n32" ∥ hash)`. We compute that
//! digest here, then delegate the k256 ecrecover + 20-byte-address match to the
//! shared [`crate::verify`] kernel (threat #5).

use sha3::{Digest, Keccak256};

use ironclaw_signing_provider::SigningProviderError;

use crate::verify::verify_evm_signer_over_digest;

/// EIP-191 personal-message prefix for a 32-byte message.
const PERSONAL_PREFIX_32: &[u8] = b"\x19Ethereum Signed Message:\n32";

/// Recover the signer from `signature` over the EIP-191 personal-sign digest of
/// `hash_bytes`, and require it to equal `bound_account`.
///
/// `bound_account` is a `0x`-prefixed (case-insensitive) hex EVM address.
pub(super) fn verify_signer_over_hash(
    hash_bytes: &[u8; 32],
    signature: &[u8],
    bound_account: &str,
) -> Result<(), SigningProviderError> {
    // EIP-191 personal-sign digest over the 32-byte approved hash.
    let mut hasher = Keccak256::new();
    hasher.update(PERSONAL_PREFIX_32);
    hasher.update(hash_bytes);
    let digest: [u8; 32] = hasher.finalize().into();

    verify_evm_signer_over_digest(&digest, signature, bound_account)
}

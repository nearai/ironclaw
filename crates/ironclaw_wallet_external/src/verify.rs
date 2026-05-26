//! Reusable, chain-specific signature-verification kernels.
//!
//! These are the *single source of truth* for the two low-level signature
//! checks the external-wallet substrate performs, decoupled from any
//! particular message/digest semantics:
//!
//! * [`verify_evm_signer_over_digest`] — k256 secp256k1 ecrecover of a 65-byte
//!   `(r ∥ s ∥ v)` signature over a 32-byte digest, reduced to the 20-byte EVM
//!   address, required to equal the bound account.
//! * [`verify_ed25519_signer_over_digest`] — ed25519 verification of a 64-byte
//!   signature over a 32-byte digest against a 32-byte public key, with the
//!   public key required to equal the bound account.
//!
//! The injected (`injected/evm.rs`, `injected/solana.rs`) and WalletConnect
//! (`walletconnect/signer.rs`) providers, and the trust-registration ceremony
//! in `ironclaw_attested_runtime`, all delegate here so the keccak/ecrecover
//! and ed25519 logic exists exactly once. Callers own *what* 32-byte digest is
//! signed (raw approved-tx hash, EIP-191 personal-sign digest, a
//! domain-separated attestation digest, or a trust challenge); this module owns
//! *how* the signer is recovered/verified and matched to the bound account.

use ed25519_dalek::{Signature as EdSignature, Verifier, VerifyingKey as EdVerifyingKey};
use k256::ecdsa::{RecoveryId, Signature as EcSignature, VerifyingKey};
use sha3::{Digest, Keccak256};

use ironclaw_signing_provider::SigningProviderError;

/// Recover the EVM signer from a 65-byte `(r ∥ s ∥ v)` `signature` over the
/// 32-byte `digest`, derive its 20-byte address, and require it to equal
/// `bound_account`.
///
/// `bound_account` is a `0x`-prefixed (case-insensitive) hex EVM address.
/// `digest` is the prehash the signature is over — the caller is responsible
/// for any EIP-191 / EIP-712 / domain-separation framing.
pub fn verify_evm_signer_over_digest(
    digest: &[u8; 32],
    signature: &[u8],
    bound_account: &str,
) -> Result<(), SigningProviderError> {
    if signature.len() != 65 {
        return Err(SigningProviderError::ProofInvalid {
            reason: format!("evm signature must be 65 bytes, got {}", signature.len()),
        });
    }
    let sig = EcSignature::from_slice(&signature[..64]).map_err(|e| {
        SigningProviderError::ProofInvalid {
            reason: format!("invalid evm signature scalars: {e}"),
        }
    })?;
    let rec_id = recovery_id_from_v(signature[64])?;
    let recovered =
        VerifyingKey::recover_from_prehash(digest.as_slice(), &sig, rec_id).map_err(|e| {
            SigningProviderError::ProofInvalid {
                reason: format!("evm signer recovery failed: {e}"),
            }
        })?;
    let recovered_address = address_from_verifying_key(&recovered);
    let bound = parse_evm_address(bound_account)?;
    if recovered_address != bound {
        return Err(SigningProviderError::SignerMismatch);
    }
    Ok(())
}

/// Verify a 64-byte ed25519 `signature` over the 32-byte `digest` against
/// `public_key`, and require `public_key` to equal `bound_account`.
///
/// `bound_account` is the lowercase (optionally `0x`-prefixed) hex of the
/// 32-byte ed25519 public key. Used by Solana and NEAR, whose signer is the
/// connected ed25519 account.
pub fn verify_ed25519_signer_over_digest(
    digest: &[u8; 32],
    signature: &[u8],
    public_key: &[u8],
    bound_account: &str,
) -> Result<(), SigningProviderError> {
    if signature.len() != 64 {
        return Err(SigningProviderError::ProofInvalid {
            reason: format!(
                "ed25519 signature must be 64 bytes, got {}",
                signature.len()
            ),
        });
    }
    let pk_bytes: [u8; 32] =
        public_key
            .try_into()
            .map_err(|_| SigningProviderError::ProofInvalid {
                reason: format!(
                    "ed25519 public key must be 32 bytes, got {}",
                    public_key.len()
                ),
            })?;
    // Signer binding: the verifying key must equal the bound account before we
    // trust any signature it produced.
    let bound = parse_ed25519_pubkey(bound_account)?;
    if pk_bytes != bound {
        return Err(SigningProviderError::SignerMismatch);
    }
    let verifying_key =
        EdVerifyingKey::from_bytes(&pk_bytes).map_err(|e| SigningProviderError::ProofInvalid {
            reason: format!("invalid ed25519 public key: {e}"),
        })?;
    let sig_bytes: [u8; 64] =
        signature
            .try_into()
            .map_err(|_| SigningProviderError::ProofInvalid {
                reason: "ed25519 signature length mismatch".to_string(),
            })?;
    let sig = EdSignature::from_bytes(&sig_bytes);
    verifying_key
        .verify(digest, &sig)
        .map_err(|e| SigningProviderError::ProofInvalid {
            reason: format!("ed25519 verification failed: {e}"),
        })?;
    Ok(())
}

/// Normalize the signature `v` byte to a `k256` [`RecoveryId`].
///
/// Accepts the raw 0/1 form, the legacy 27/28 form, and EIP-155 `v`
/// (`35 + 2*chain_id + parity`) by reducing to parity.
pub(crate) fn recovery_id_from_v(v: u8) -> Result<RecoveryId, SigningProviderError> {
    let parity = match v {
        0 | 1 => v,
        27 | 28 => v - 27,
        v if v >= 35 => (v - 35) & 1,
        other => {
            return Err(SigningProviderError::ProofInvalid {
                reason: format!("invalid evm recovery id v={other}"),
            });
        }
    };
    RecoveryId::from_byte(parity).ok_or(SigningProviderError::ProofInvalid {
        reason: "invalid evm recovery id parity".to_string(),
    })
}

/// Derive the 20-byte EVM address from a recovered secp256k1 public key:
/// `keccak256(uncompressed_pubkey[1..])[12..]`.
pub(crate) fn address_from_verifying_key(key: &VerifyingKey) -> [u8; 20] {
    let encoded = key.to_encoded_point(false);
    // Skip the 0x04 prefix byte of the uncompressed SEC1 point.
    let hash = Keccak256::digest(&encoded.as_bytes()[1..]);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]);
    addr
}

/// Parse a `0x`-prefixed (case-insensitive) hex EVM address into 20 bytes.
pub(crate) fn parse_evm_address(s: &str) -> Result<[u8; 20], SigningProviderError> {
    // Decode over raw bytes: the bound account is untrusted input and may carry
    // multi-byte UTF-8 of even byte length, so `&str` byte-range slicing would
    // panic on a non-char-boundary. `&[u8]` indexing is panic-free and any
    // non-ASCII byte is rejected cleanly by `hex_digit`.
    let stripped = s.strip_prefix("0x").unwrap_or(s).as_bytes();
    if stripped.len() != 40 {
        return Err(SigningProviderError::ProofInvalid {
            reason: format!("bound account is not a 20-byte evm address: {s}"),
        });
    }
    let mut out = [0u8; 20];
    for (byte, pair) in out.iter_mut().zip(stripped.chunks_exact(2)) {
        *byte = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
    }
    Ok(out)
}

/// Parse a lowercase-hex (optionally `0x`-prefixed) 32-byte ed25519 public key
/// into bytes.
pub(crate) fn parse_ed25519_pubkey(s: &str) -> Result<[u8; 32], SigningProviderError> {
    // See `parse_evm_address`: decode over raw bytes to stay panic-free on
    // untrusted multi-byte UTF-8 input.
    let stripped = s.strip_prefix("0x").unwrap_or(s).as_bytes();
    if stripped.len() != 64 {
        return Err(SigningProviderError::ProofInvalid {
            reason: format!("bound account is not a 32-byte ed25519 key: {s}"),
        });
    }
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(stripped.chunks_exact(2)) {
        *byte = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
    }
    Ok(out)
}

/// Decode a single ASCII hex digit byte to its 0–15 value, rejecting any
/// non-hex (including non-ASCII) byte without panicking.
fn hex_digit(b: u8) -> Result<u8, SigningProviderError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        other => Err(SigningProviderError::ProofInvalid {
            reason: format!("bound account hex invalid digit: {other:#04x}"),
        }),
    }
}

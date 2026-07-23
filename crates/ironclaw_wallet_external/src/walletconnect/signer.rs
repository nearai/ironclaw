//! Signer recovery / verification for WalletConnect v2 chain signatures.
//!
//! A WalletConnect wallet does **not** sign a synthetic attestation digest. It
//! signs the **real transaction** via `eth_signTransaction` /
//! `solana_signTransaction`. The proof therefore carries the exact bytes that
//! chain signature covers ([`WalletConnectProofPayload::signed_payload`](super::proof::WalletConnectProofPayload::signed_payload)):
//!
//! * For EVM the `signed_payload` is the 32-byte secp256k1 sighash (the
//!   keccak256 of the EIP-2718/RLP signing pre-image); the signer is recovered
//!   from the 65-byte signature via `k256` ecrecover and reduced to its 20-byte
//!   address.
//! * For Solana/NEAR the `signed_payload` is the ed25519 message bytes; the
//!   signature is verified against the connected ed25519 public key with the
//!   vendored `ed25519-dalek`.
//!
//! In every case the resolved signer must equal the bound account
//! ([`SignerMismatch`](ironclaw_signing_provider::SigningProviderError::SignerMismatch)).
//! The binding from `signed_payload` back to the human-approved transaction is
//! enforced by the caller ([`super::WalletConnectSigningProvider::verify_resume`]),
//! which requires `signed_payload == binding.expected_signing_payload` (both
//! derived from the same decoded tx). The session-topic + nonce binding is an
//! additional anti-replay layer enforced by the caller, not a replacement for
//! this real chain-signature check.
//!
//! The relay transport / Sign envelope crypto comes from the fork — it is never
//! reimplemented here.

use k256::ecdsa::{RecoveryId, Signature as EcSignature, VerifyingKey};
use sha3::{Digest, Keccak256};

use ed25519_dalek::{Signature as EdSignature, Verifier, VerifyingKey as EdVerifyingKey};

use ironclaw_signing_provider::SigningProviderError;

use super::namespace::ChainFamily;

/// Verify the wallet's **real chain signature** over `signed_payload` and
/// require the resolved signer to equal `bound_account`.
///
/// * `family` selects the recovery/verification scheme.
/// * `signed_payload` is the exact bytes the chain signature covers: the
///   32-byte EVM secp256k1 sighash, or the Solana ed25519 message bytes.
/// * `signature` is 65 bytes (r ∥ s ∥ v) for EVM, 64 bytes for ed25519 families.
/// * `public_key` is required (32 bytes) for the ed25519 families and ignored
///   for EVM (the address is recovered from the signature).
pub(super) fn verify_chain_signature(
    family: ChainFamily,
    signed_payload: &[u8],
    signature: &[u8],
    public_key: Option<&[u8]>,
    bound_account: &str,
) -> Result<(), SigningProviderError> {
    match family {
        ChainFamily::Evm => verify_evm(signed_payload, signature, bound_account),
        ChainFamily::Solana => {
            let pk = public_key.ok_or(SigningProviderError::ProofInvalid {
                reason: "ed25519 walletconnect proof missing public_key".to_string(),
            })?;
            verify_ed25519(signed_payload, signature, pk, bound_account)
        }
    }
}

/// Recover the EVM signer from a 65-byte signature over the 32-byte secp256k1
/// sighash `signed_payload` and require it to equal `bound_account`
/// (`0x`-prefixed, case-insensitive 20-byte hex).
fn verify_evm(
    signed_payload: &[u8],
    signature: &[u8],
    bound_account: &str,
) -> Result<(), SigningProviderError> {
    // The EVM chain signature is over a 32-byte prehash (the EIP-2718/RLP
    // sighash). Anything else cannot be a valid eth_signTransaction sighash and
    // fails closed.
    let prehash: [u8; 32] =
        signed_payload
            .try_into()
            .map_err(|_| SigningProviderError::ProofInvalid {
                reason: format!(
                    "evm signed payload must be a 32-byte sighash, got {} bytes",
                    signed_payload.len()
                ),
            })?;
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
        VerifyingKey::recover_from_prehash(prehash.as_slice(), &sig, rec_id).map_err(|e| {
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

/// Verify a 64-byte ed25519 signature over the message `signed_payload` against
/// `public_key`, and require `public_key` to equal `bound_account` (lowercase
/// 32-byte hex).
fn verify_ed25519(
    signed_payload: &[u8],
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
    // Signer binding (T17): the verifying key must equal the bound account
    // before we trust any signature it produced.
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
        .verify(signed_payload, &sig)
        .map_err(|e| SigningProviderError::ProofInvalid {
            reason: format!("ed25519 verification failed: {e}"),
        })?;
    Ok(())
}

/// Normalize the signature `v` byte to a `k256` [`RecoveryId`] (0/1, 27/28, or
/// EIP-155 reduced to parity).
fn recovery_id_from_v(v: u8) -> Result<RecoveryId, SigningProviderError> {
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

/// `keccak256(uncompressed_pubkey[1..])[12..]`.
fn address_from_verifying_key(key: &VerifyingKey) -> [u8; 20] {
    let encoded = key.to_encoded_point(false);
    let hash = Keccak256::digest(&encoded.as_bytes()[1..]);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]);
    addr
}

/// Parse a `0x`-prefixed (case-insensitive) hex EVM address into 20 bytes.
fn parse_evm_address(s: &str) -> Result<[u8; 20], SigningProviderError> {
    decode_hex_fixed::<20>(s).map_err(|_| SigningProviderError::ProofInvalid {
        reason: format!("bound account is not a 20-byte evm address: {s}"),
    })
}

/// Parse a lowercase-hex 32-byte ed25519 public key into bytes.
fn parse_ed25519_pubkey(s: &str) -> Result<[u8; 32], SigningProviderError> {
    decode_hex_fixed::<32>(s).map_err(|_| SigningProviderError::ProofInvalid {
        reason: format!("bound account is not a 32-byte ed25519 key: {s}"),
    })
}

/// Decode `0x`-prefixed (case-insensitive) hex into a fixed-size `[u8; N]`,
/// panic-free on any input.
///
/// Delegates the byte-based, panic-free nibble decoding to the shared
/// [`super::hex_bytes::hex_decode`] (which operates on **bytes**, never on
/// `&str` byte-offset slices, so non-ASCII even-byte input fails closed instead
/// of panicking — #3) and then enforces the fixed length `N`.
fn decode_hex_fixed<const N: usize>(s: &str) -> Result<[u8; N], ()> {
    let bytes = super::hex_bytes::hex_decode(s).map_err(|_| ())?;
    bytes.try_into().map_err(|_| ())
}

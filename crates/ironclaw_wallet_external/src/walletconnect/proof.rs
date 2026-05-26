//! Wire shape of the WalletConnect v2 signing proof.
//!
//! Serialized into the opaque [`SigningProof::WalletConnectProof`](ironclaw_signing_provider::SigningProof::WalletConnectProof)
//! byte body. The payload echoes everything the verifier re-checks against the
//! recorded [`SessionBinding`](super::session::SessionBinding) and the bound
//! account — none of it is trusted on its own.
//!
//! The security-critical field is [`WalletConnectProofPayload::signed_payload`]:
//! the **exact bytes the wallet's chain signature covers** (the EVM secp256k1
//! sighash / the Solana ed25519 message), as returned by
//! `eth_signTransaction` / `solana_signTransaction`. The verifier checks the
//! real chain signature over *those* bytes and requires them to equal the
//! `expected_signing_payload` recorded at `initiate` from the same decoded
//! transaction that produced the approved hash — see
//! [`super::WalletConnectSigningProvider::verify_resume`]. A signature over a
//! synthetic digest is never accepted.

use serde::{Deserialize, Serialize};

use ironclaw_signing_provider::{ApprovedTxHash, SigningProviderError};

use super::hex_bytes;

/// The structured payload a WalletConnect wallet carries back from the v2
/// signing ceremony.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletConnectProofPayload {
    /// The WalletConnect session topic this proof was minted under. Must equal
    /// the recorded session binding (T18).
    pub session_topic: String,
    /// The approved-tx hash the wallet attests to. Must equal the bound hash
    /// (T20); re-checked before any signature work.
    pub approved_tx_hash: ApprovedTxHash,
    /// The account the wallet claims signed. Re-derived from the signature
    /// (EVM) / checked against the public key (ed25519) and compared to the
    /// bound account; never trusted on its own (T17).
    pub claimed_signer: String,
    /// Per-request nonce the wallet committed to. Must equal the recorded
    /// binding nonce (T18).
    #[serde(with = "hex_bytes")]
    pub nonce: Vec<u8>,
    /// The **exact bytes the wallet's chain signature covers** — the EVM
    /// secp256k1 sighash / the Solana ed25519 message returned by
    /// `eth_signTransaction` / `solana_signTransaction`. The verifier checks the
    /// chain signature over *these* bytes and requires them to equal the
    /// `expected_signing_payload` recorded at `initiate` from the same decoded
    /// transaction that produced the approved hash (the binding back to what the
    /// human approved). Never trusted on its own — a payload that does not match
    /// the recorded expectation is rejected before any signature work (#1).
    #[serde(with = "hex_bytes")]
    pub signed_payload: Vec<u8>,
    /// The chain signature over [`Self::signed_payload`]. 65 bytes (r ∥ s ∥ v)
    /// for EVM secp256k1, 64 bytes for ed25519 families.
    #[serde(with = "hex_bytes")]
    pub signature: Vec<u8>,
    /// For the ed25519 families (Solana / NEAR), the 32-byte public key the
    /// signature verifies against (lowercase hex). Unused for EVM.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "opt_hex_bytes"
    )]
    pub public_key: Option<Vec<u8>>,
}

/// Serialize a [`WalletConnectProofPayload`] into opaque proof bytes.
///
/// Returns [`SigningProviderError::ProofInvalid`] if serialization fails rather
/// than silently emitting an empty body — an empty `Vec` would later
/// decode-fail with a confusing "malformed payload" error far from the real
/// cause, so the error is surfaced at the encode site.
pub fn encode_walletconnect_proof(
    payload: &WalletConnectProofPayload,
) -> Result<Vec<u8>, SigningProviderError> {
    serde_json::to_vec(payload).map_err(|e| SigningProviderError::ProofInvalid {
        reason: format!("failed to serialize walletconnect proof payload: {e}"),
    })
}

/// Decode opaque proof bytes into a structured [`WalletConnectProofPayload`].
pub fn decode_walletconnect_proof(
    bytes: &[u8],
) -> Result<WalletConnectProofPayload, SigningProviderError> {
    serde_json::from_slice(bytes).map_err(|e| SigningProviderError::ProofInvalid {
        reason: format!("malformed walletconnect proof payload: {e}"),
    })
}

/// Hex (de)serialization helper for `Option<Vec<u8>>` proof fields.
mod opt_hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        bytes: &Option<Vec<u8>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(b) => s.serialize_str(&super::hex_bytes::hex_encode(b)),
            None => s.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        let opt = Option::<String>::deserialize(d)?;
        match opt {
            Some(s) => super::hex_bytes::hex_decode(&s)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

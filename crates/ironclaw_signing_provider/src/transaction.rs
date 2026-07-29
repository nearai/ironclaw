//! Opaque, forward-declared transaction value types.
//!
//! The concrete decode / render / canonical-hash implementations land in
//! `ironclaw_attestation` (PR2). At this layer the types are opaque so the
//! [`crate::SigningProvider`] trait can name them without any chain or crypto
//! dependency.

use serde::{Deserialize, Serialize};

/// Length in bytes of an [`ApprovedTxHash`].
pub const APPROVED_TX_HASH_LEN: usize = 32;

/// A server-decoded transaction.
///
/// **Opaque at this layer.** The real type in PR2 carries the per-chain
/// decoded fields used to build the canonical signing bytes and the rendered
/// view. Here it is an opaque, chain-free handle so the trait can pass it
/// around. Treat the inner bytes as already-canonicalized decode output; this
/// crate never interprets them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodedTransaction {
    /// Opaque payload produced by the chain-specific decoder in PR2.
    payload: Vec<u8>,
}

impl DecodedTransaction {
    /// Wrap an opaque decoded payload.
    pub fn from_opaque(payload: impl Into<Vec<u8>>) -> Self {
        Self {
            payload: payload.into(),
        }
    }

    /// Borrow the opaque payload bytes.
    pub fn as_opaque(&self) -> &[u8] {
        &self.payload
    }
}

/// A human-facing rendering of a transaction (the WYSIWYS view shown at the
/// gate).
///
/// **Opaque at this layer.** PR2 defines the structured render schema (with a
/// rendering-schema-version that participates in the approved-tx hash). Here it
/// is an opaque handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedTx {
    /// Opaque rendered payload produced by the renderer in PR2.
    payload: Vec<u8>,
}

impl RenderedTx {
    /// Wrap an opaque rendered payload.
    pub fn from_opaque(payload: impl Into<Vec<u8>>) -> Self {
        Self {
            payload: payload.into(),
        }
    }

    /// Borrow the opaque rendered bytes.
    pub fn as_opaque(&self) -> &[u8] {
        &self.payload
    }
}

/// The domain-separated, fixed-width hash that binds an approved transaction.
///
/// In PR2 this is computed over `render ∥ canonical signing bytes ∥
/// signer/account ∥ chain/network ∥ tx-type ∥ rendering-schema-version`
/// (domain-separated CBOR). It is a **binding**, not an authorization: the
/// sealed one-shot grant (PR3) is what authorizes a single signing. At this
/// layer it is a fixed 32-byte newtype so the trait can name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApprovedTxHash([u8; APPROVED_TX_HASH_LEN]);

impl ApprovedTxHash {
    /// Construct from raw 32 bytes. The bytes are assumed to already be the
    /// domain-separated digest computed by `ironclaw_attestation` (PR2); this
    /// crate performs no hashing.
    pub fn from_bytes(bytes: [u8; APPROVED_TX_HASH_LEN]) -> Self {
        Self(bytes)
    }

    /// Borrow the raw hash bytes.
    pub fn as_bytes(&self) -> &[u8; APPROVED_TX_HASH_LEN] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_tx_hash_is_thirty_two_bytes() {
        assert_eq!(APPROVED_TX_HASH_LEN, 32);
        let hash = ApprovedTxHash::from_bytes([7u8; 32]);
        assert_eq!(hash.as_bytes(), &[7u8; 32]);
    }

    #[test]
    fn approved_tx_hash_round_trips() {
        let hash = ApprovedTxHash::from_bytes([1u8; 32]);
        let json = serde_json::to_string(&hash).expect("serialize");
        let back: ApprovedTxHash = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, hash);
    }

    #[test]
    fn opaque_transaction_handles_round_trip() {
        let decoded = DecodedTransaction::from_opaque(vec![1, 2, 3]);
        assert_eq!(decoded.as_opaque(), &[1, 2, 3]);
        let rendered = RenderedTx::from_opaque(vec![4, 5]);
        assert_eq!(rendered.as_opaque(), &[4, 5]);

        let back: DecodedTransaction =
            serde_json::from_str(&serde_json::to_string(&decoded).expect("ser")).expect("de");
        assert_eq!(back, decoded);
    }
}

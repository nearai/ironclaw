//! NEAR decode -> PR2 [`DecodedTransaction`].
//!
//! Accepts an already-projected [`NearTransaction`]. A full
//! `near-primitives::Transaction` wire decoder is the next slice (module docs).

use ironclaw_attestation::{DecodedTransaction, NearTransaction};

use crate::error::ChainSigningError;

/// Wrap a projected NEAR transaction as a chain-tagged [`DecodedTransaction`],
/// validating the basic shape.
pub fn decode_projected(tx: NearTransaction) -> Result<DecodedTransaction, ChainSigningError> {
    if tx.signer_id.is_empty() {
        return Err(ChainSigningError::Decode {
            chain: "near",
            reason: "empty signer_id".to_string(),
        });
    }
    if tx.receiver_id.is_empty() {
        return Err(ChainSigningError::Decode {
            chain: "near",
            reason: "empty receiver_id".to_string(),
        });
    }
    if tx.actions.is_empty() {
        return Err(ChainSigningError::Decode {
            chain: "near",
            reason: "transaction has no actions".to_string(),
        });
    }
    Ok(DecodedTransaction::Near(tx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_attestation::{Bytes32, NearAction, NearPublicKey};

    fn tx() -> NearTransaction {
        NearTransaction {
            network: "mainnet".into(),
            signer_id: "alice.near".into(),
            public_key: NearPublicKey {
                key_type: 0,
                data: vec![7u8; 32],
            },
            receiver_id: "bob.near".into(),
            nonce: 1,
            block_hash: Bytes32([3u8; 32]),
            actions: vec![NearAction::Transfer { deposit: vec![1] }],
        }
    }

    #[test]
    fn valid_projection_decodes() {
        assert!(decode_projected(tx()).is_ok());
    }

    #[test]
    fn missing_fields_rejected() {
        let mut t = tx();
        t.signer_id = String::new();
        assert!(decode_projected(t).is_err());
        let mut t = tx();
        t.actions.clear();
        assert!(decode_projected(t).is_err());
    }
}

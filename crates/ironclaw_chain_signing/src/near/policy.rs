//! NEAR untrusted-metadata policy: network identity verification.
//!
//! The transaction's signer access-key nonce, block hash, actions, deposit, and
//! gas are all surfaced by the PR2 render projection; this module adds the
//! network cross-check (mainnet vs testnet) that the raw field list cannot
//! express as a policy decision.

use ironclaw_attestation::NearTransaction;

use crate::error::ChainSigningError;

/// Verify the transaction's NEAR network against the expected network.
///
/// A mismatch (a testnet tx presented as mainnet, or vice versa) is fatal — it
/// can move real value on an unintended network.
pub fn check_network(
    tx: &NearTransaction,
    expected_network: &str,
) -> Result<(), ChainSigningError> {
    if tx.network != expected_network {
        return Err(ChainSigningError::MetadataPolicy {
            chain: "near",
            reason: format!(
                "tx network {:?} does not match expected network {expected_network:?}",
                tx.network
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_attestation::{Bytes32, NearAction, NearPublicKey};

    fn tx(network: &str) -> NearTransaction {
        NearTransaction {
            network: network.into(),
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
    fn wrong_network_rejected() {
        assert!(check_network(&tx("testnet"), "mainnet").is_err());
        assert!(check_network(&tx("mainnet"), "mainnet").is_ok());
    }
}

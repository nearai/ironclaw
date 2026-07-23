//! EVM untrusted-metadata policy checks.
//!
//! RPC endpoints and token lists are untrusted. Before a transaction is shown
//! for approval, the decoded tx is checked against an **expected** chain id and
//! the hidden / easily-overlooked fields (tx type, access lists, blob commitments,
//! fee caps) are surfaced for the render. The render itself already lists every
//! field (PR2 projection); this module adds the cross-checks that a raw field
//! list cannot express — e.g. "the chain id the RPC claims matches the chain id
//! the user is operating on", and token-metadata source disclosure.

use ironclaw_attestation::{DecodedTransaction, EvmTransaction};

use crate::error::ChainSigningError;

/// Untrusted token metadata that a UI might render alongside an ERC-20 transfer.
///
/// The policy requires the metadata to disclose its SOURCE (which list / which
/// RPC), not just a symbol, so a spoofed token symbol cannot impersonate a
/// well-known asset without the user seeing where the claim came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmTokenMetadata {
    /// Token contract address, lowercase hex (no `0x`).
    pub contract_hex: String,
    /// Decimals claimed by the source.
    pub decimals: u8,
    /// Symbol claimed by the source.
    pub symbol: String,
    /// Where this metadata came from (token list URL, RPC, on-chain read).
    /// Required: empty source is rejected.
    pub source: String,
}

/// Verify the decoded EVM transaction's chain id against the expected chain id.
///
/// Mismatch is fatal: an RPC that reports a different chain id than the user is
/// operating on could be replaying a tx onto an unintended network.
pub fn check_chain_id(
    tx: &EvmTransaction,
    expected_chain_id: u64,
) -> Result<(), ChainSigningError> {
    if tx.chain_id != expected_chain_id {
        return Err(ChainSigningError::MetadataPolicy {
            chain: "evm",
            reason: format!(
                "tx chain id {} does not match expected chain id {expected_chain_id}",
                tx.chain_id
            ),
        });
    }
    // A legacy tx with chain_id 0 is replay-unprotected (pre-EIP-155). Refuse
    // it unless the expected chain is also explicitly 0 (never, for mainnets).
    if tx.chain_id == 0 {
        return Err(ChainSigningError::MetadataPolicy {
            chain: "evm",
            reason: "replay-unprotected legacy transaction (chain id 0) refused".to_string(),
        });
    }
    Ok(())
}

/// Validate untrusted token metadata: the source MUST be disclosed.
pub fn check_token_metadata(meta: &EvmTokenMetadata) -> Result<(), ChainSigningError> {
    if meta.source.trim().is_empty() {
        return Err(ChainSigningError::MetadataPolicy {
            chain: "evm",
            reason: format!(
                "token {} ({}) has no metadata source; refusing to render symbol without provenance",
                meta.contract_hex, meta.symbol
            ),
        });
    }
    Ok(())
}

/// Summarize the hidden / easily-overlooked signing-relevant fields so the
/// caller can surface them in the render (they are also in the PR2 field list,
/// but this gives the UI an explicit "advanced fields present" signal).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EvmHiddenFields {
    /// EIP-2718 type byte.
    pub tx_type: u8,
    /// Whether an access list is present.
    pub has_access_list: bool,
    /// Whether blob commitments are present (EIP-4844).
    pub has_blobs: bool,
    /// Whether a blob fee cap is present.
    pub has_blob_fee_cap: bool,
}

/// Extract the hidden-field summary from a decoded transaction.
pub fn hidden_fields(tx: &DecodedTransaction) -> Option<EvmHiddenFields> {
    let DecodedTransaction::Evm(evm) = tx else {
        return None;
    };
    Some(EvmHiddenFields {
        tx_type: evm.tx_type,
        has_access_list: !evm.access_list.is_empty(),
        has_blobs: !evm.blob_versioned_hashes.is_empty(),
        has_blob_fee_cap: evm.max_fee_per_blob_gas.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_attestation::{Bytes32, EvmAddress};

    fn evm(chain_id: u64) -> EvmTransaction {
        EvmTransaction {
            chain_id,
            nonce: 0,
            tx_type: 2,
            to: Some(EvmAddress([1u8; 20])),
            value: vec![],
            data: vec![],
            gas_limit: 21000,
            gas_price: None,
            max_fee_per_gas: Some(vec![1]),
            max_priority_fee_per_gas: Some(vec![1]),
            access_list: vec![],
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
        }
    }

    #[test]
    fn wrong_chain_id_rejected() {
        let err = check_chain_id(&evm(10), 1).unwrap_err();
        assert!(matches!(err, ChainSigningError::MetadataPolicy { .. }));
        assert!(check_chain_id(&evm(1), 1).is_ok());
    }

    #[test]
    fn chain_id_zero_rejected_as_replay_unprotected() {
        assert!(check_chain_id(&evm(0), 0).is_err());
    }

    #[test]
    fn token_metadata_requires_source() {
        let mut meta = EvmTokenMetadata {
            contract_hex: "aa".into(),
            decimals: 18,
            symbol: "USDC".into(),
            source: "".into(),
        };
        assert!(check_token_metadata(&meta).is_err());
        meta.source = "https://tokenlists.org/uniswap".into();
        assert!(check_token_metadata(&meta).is_ok());
    }

    #[test]
    fn hidden_fields_flag_blobs_and_access_lists() {
        let mut tx = evm(1);
        tx.blob_versioned_hashes = vec![Bytes32([2u8; 32])];
        tx.max_fee_per_blob_gas = Some(vec![5]);
        let h = hidden_fields(&DecodedTransaction::Evm(tx)).unwrap();
        assert!(h.has_blobs);
        assert!(h.has_blob_fee_cap);
        assert_eq!(h.tx_type, 2);
    }
}

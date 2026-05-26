//! Solana untrusted-metadata policy: cluster/genesis verification and SPL token
//! mint metadata source disclosure.

use ironclaw_attestation::SolanaTransaction;

use crate::error::ChainSigningError;

/// Untrusted SPL-token metadata for a mint referenced by the transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolanaTokenMetadata {
    /// Mint address, base58 or hex — opaque string here.
    pub mint: String,
    /// Decimals claimed by the source.
    pub decimals: u8,
    /// Symbol claimed by the source.
    pub symbol: String,
    /// Provenance of this metadata (registry URL, on-chain metaplex read, …).
    pub source: String,
}

/// Verify the transaction's cluster against the expected cluster identity.
///
/// A wrong cluster (e.g. a devnet message presented as mainnet, or vice versa)
/// is fatal — it can move real value on an unintended network.
pub fn check_cluster(
    tx: &SolanaTransaction,
    expected_cluster: &str,
) -> Result<(), ChainSigningError> {
    if tx.cluster != expected_cluster {
        return Err(ChainSigningError::MetadataPolicy {
            chain: "solana",
            reason: format!(
                "tx cluster {:?} does not match expected cluster {expected_cluster:?}",
                tx.cluster
            ),
        });
    }
    Ok(())
}

/// Validate untrusted SPL token metadata: the source MUST be disclosed and the
/// mint must render with its source, not just a symbol.
pub fn check_token_metadata(meta: &SolanaTokenMetadata) -> Result<(), ChainSigningError> {
    if meta.source.trim().is_empty() {
        return Err(ChainSigningError::MetadataPolicy {
            chain: "solana",
            reason: format!(
                "SPL token mint {} ({}) has no metadata source; refusing to render symbol \
                 without provenance",
                meta.mint, meta.symbol
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_attestation::{
        Bytes32, SolanaCompiledInstruction, SolanaMessageHeader, SolanaMessageVersion,
    };

    fn tx(cluster: &str) -> SolanaTransaction {
        let program = Bytes32([9u8; 32]);
        SolanaTransaction {
            cluster: cluster.into(),
            version: SolanaMessageVersion::Legacy,
            header: SolanaMessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 1,
            },
            static_account_keys: vec![Bytes32([1u8; 32]), program],
            recent_blockhash: Bytes32([2u8; 32]),
            instructions: vec![SolanaCompiledInstruction {
                program_id_index: 1,
                account_indices: vec![],
                data: vec![],
            }],
            address_table_lookups: vec![],
        }
    }

    #[test]
    fn wrong_cluster_rejected() {
        assert!(check_cluster(&tx("devnet"), "mainnet-beta").is_err());
        assert!(check_cluster(&tx("mainnet-beta"), "mainnet-beta").is_ok());
    }

    #[test]
    fn token_metadata_requires_source() {
        let mut meta = SolanaTokenMetadata {
            mint: "So111...".into(),
            decimals: 9,
            symbol: "SOL".into(),
            source: "".into(),
        };
        assert!(check_token_metadata(&meta).is_err());
        meta.source = "https://github.com/solana-labs/token-list".into();
        assert!(check_token_metadata(&meta).is_ok());
    }
}

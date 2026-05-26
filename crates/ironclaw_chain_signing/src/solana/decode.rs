//! Solana decode -> PR2 [`DecodedTransaction`].
//!
//! Accepts an already-projected [`SolanaTransaction`] (the PR2 model, with
//! address-lookup-table references already resolved to absolute pubkeys). A
//! `solana-sdk` `VersionedMessage` wire decoder that performs on-chain ALT
//! resolution is the immediate next slice (see module docs).

use ironclaw_attestation::{DecodedTransaction, SolanaTransaction};

use crate::error::ChainSigningError;

/// Wrap a projected Solana transaction as a chain-tagged [`DecodedTransaction`].
///
/// Validates the basic shape (non-empty account keys) and rejects obvious
/// inconsistencies so a malformed projection can't reach the signer.
pub fn decode_projected(tx: SolanaTransaction) -> Result<DecodedTransaction, ChainSigningError> {
    if tx.static_account_keys.is_empty() {
        return Err(ChainSigningError::Decode {
            chain: "solana",
            reason: "message has no account keys".to_string(),
        });
    }
    // Every instruction's program id is referenced by index into the static
    // account-key list (Solana requires program ids to be in the account-key
    // list). Reject any instruction whose program/account index is out of
    // bounds so a malformed projection can't reach the signer.
    let key_count = tx.static_account_keys.len();
    for (i, ix) in tx.instructions.iter().enumerate() {
        if usize::from(ix.program_id_index) >= key_count {
            return Err(ChainSigningError::Decode {
                chain: "solana",
                reason: format!("instruction {i} program id index out of bounds"),
            });
        }
        if let Some(bad) = ix
            .account_indices
            .iter()
            .find(|idx| usize::from(**idx) >= key_count)
        {
            return Err(ChainSigningError::Decode {
                chain: "solana",
                reason: format!("instruction {i} account index {bad} out of bounds"),
            });
        }
    }
    Ok(DecodedTransaction::Solana(tx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_attestation::{
        Bytes32, SolanaCompiledInstruction, SolanaMessageHeader, SolanaMessageVersion,
    };

    fn tx() -> SolanaTransaction {
        let program = Bytes32([9u8; 32]);
        SolanaTransaction {
            cluster: "mainnet-beta".into(),
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
                account_indices: vec![0],
                data: vec![1, 2, 3],
            }],
            address_table_lookups: vec![],
        }
    }

    #[test]
    fn valid_projection_decodes() {
        assert!(decode_projected(tx()).is_ok());
    }

    #[test]
    fn empty_account_keys_rejected() {
        let mut t = tx();
        t.static_account_keys.clear();
        assert!(decode_projected(t).is_err());
    }

    #[test]
    fn unknown_program_id_rejected() {
        let mut t = tx();
        // Program index past the end of the static account-key list.
        t.instructions[0].program_id_index = 42;
        assert!(decode_projected(t).is_err());
    }
}

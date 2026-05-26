//! Solana human render — delegates to the PR2 shared projection so the view and
//! the canonical bytes cannot diverge.

use ironclaw_attestation::{
    AttestationError, DecodedTransaction, RenderedTx, RenderingSchemaVersion, render,
};

/// Render a decoded Solana transaction for human approval.
pub fn render_solana(
    tx: &DecodedTransaction,
    schema: RenderingSchemaVersion,
) -> Result<RenderedTx, AttestationError> {
    render(tx, schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solana::decode::decode_projected;
    use ironclaw_attestation::{
        Bytes32, SolanaCompiledInstruction, SolanaMessageHeader, SolanaMessageVersion,
        SolanaTransaction,
    };

    #[test]
    fn render_surfaces_cluster_blockhash_and_message_header() {
        let program = Bytes32([9u8; 32]);
        let tx = SolanaTransaction {
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
                data: vec![1],
            }],
            address_table_lookups: vec![],
        };
        let decoded = decode_projected(tx).unwrap();
        let r = render_solana(&decoded, RenderingSchemaVersion::CURRENT).unwrap();
        assert!(r.has_label("Cluster"));
        assert!(r.has_label("Recent Blockhash"));
        assert!(r.has_label("Message Version"));
        assert!(r.has_label("Required Signatures"));
    }
}

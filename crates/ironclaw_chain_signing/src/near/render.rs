//! NEAR human render — delegates to the PR2 shared projection.

use ironclaw_attestation::{
    AttestationError, DecodedTransaction, RenderedTx, RenderingSchemaVersion, render,
};

/// Render a decoded NEAR transaction for human approval.
pub fn render_near(
    tx: &DecodedTransaction,
    schema: RenderingSchemaVersion,
) -> Result<RenderedTx, AttestationError> {
    render(tx, schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::near::decode::decode_projected;
    use ironclaw_attestation::{Bytes32, NearAction, NearPublicKey, NearTransaction};

    #[test]
    fn render_surfaces_signer_receiver_nonce_block_hash_and_actions() {
        let tx = NearTransaction {
            network: "mainnet".into(),
            signer_id: "alice.near".into(),
            public_key: NearPublicKey {
                key_type: 0,
                data: vec![7u8; 32],
            },
            receiver_id: "bob.near".into(),
            nonce: 42,
            block_hash: Bytes32([3u8; 32]),
            actions: vec![NearAction::FunctionCall {
                method_name: "ft_transfer".into(),
                args: vec![1, 2, 3],
                gas: 30_000_000_000_000,
                deposit: vec![1],
            }],
        };
        let decoded = decode_projected(tx).unwrap();
        let r = render_near(&decoded, RenderingSchemaVersion::CURRENT).unwrap();
        for label in [
            "Network",
            "Signer",
            "Receiver",
            "Access-Key Nonce",
            "Block Hash",
            "Action Kind",
            "Method",
            "Deposit (yocto)",
            "Gas",
        ] {
            assert!(r.has_label(label), "missing {label}");
        }
    }
}

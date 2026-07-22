//! EVM human render.
//!
//! Delegates to the PR2 [`ironclaw_attestation::render`] so the displayed view
//! is derived from the SAME field projection the canonical signing bytes
//! consume — there is no separate EVM render walk that could diverge from the
//! signed bytes (the anti "approve view A, sign bytes B" property is inherited
//! from PR2's shared `fields::project`).

use ironclaw_attestation::{
    AttestationError, DecodedTransaction, RenderedTx, RenderingSchemaVersion, render,
};

/// Render an EVM (or any) decoded transaction for human approval.
pub fn render_evm(
    tx: &DecodedTransaction,
    schema: RenderingSchemaVersion,
) -> Result<RenderedTx, AttestationError> {
    render(tx, schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evm::decode::decode_eip1559;
    use alloy_consensus::TxEip1559;
    use alloy_primitives::{Bytes, TxKind, U256, address};

    #[test]
    fn render_surfaces_every_signing_field() {
        let tx = TxEip1559 {
            chain_id: 1,
            nonce: 7,
            gas_limit: 21000,
            max_fee_per_gas: 100,
            max_priority_fee_per_gas: 2,
            to: TxKind::Call(address!("00000000000000000000000000000000000000aa")),
            value: U256::from(1000u64),
            access_list: Default::default(),
            input: Bytes::from(vec![0xde, 0xad]),
        };
        let decoded = decode_eip1559(&tx);
        let rendered = render_evm(&decoded, RenderingSchemaVersion::CURRENT).unwrap();
        // Every signing-relevant field must be present in the human view.
        for label in [
            "Chain ID",
            "Nonce",
            "Tx Type",
            "To",
            "Value (wei)",
            "Data",
            "Gas Limit",
            "Max Fee/Gas",
            "Max Priority Fee/Gas",
        ] {
            assert!(rendered.has_label(label), "missing label {label}");
        }
    }
}

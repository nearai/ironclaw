//! The single shared projection of a [`DecodedTransaction`] into an ordered
//! list of signing-relevant fields.
//!
//! ## Why this exists (the anti-field-smuggling guarantee)
//!
//! The whole security point of the attested-signing core is that the human
//! approves the *same* set of fields the signer will later commit to bytes —
//! "what you see is what you sign". If the renderer and the canonical encoder
//! each walked the [`DecodedTransaction`] independently, a field could be shown
//! but not signed (or signed but not shown). To make that structurally
//! impossible, **both** [`crate::rendered::render`] and
//! [`crate::canonical::canonical_signing_bytes`] are derived from *this one*
//! function. A new signing-relevant field is added in exactly one place; it
//! then automatically appears in the render and the canonical bytes (and the
//! render-coverage test asserts the two stay in lockstep).

use crate::decoded_tx::{DecodedTransaction, hex_lower};

/// One signing-relevant field, projected into a stable wire form.
///
/// `tag` is a stable per-field domain label (canonical encoding hashes it);
/// `label` / `value` are the human-readable projection (the renderer shows
/// them). `canonical_bytes` is the exact byte commitment for the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    /// Stable field tag, unique within a chain variant.
    pub tag: &'static str,
    /// Human-readable field label.
    pub label: &'static str,
    /// Human-readable value.
    pub value: String,
    /// Canonical byte commitment for this field.
    pub canonical_bytes: Vec<u8>,
}

impl Field {
    fn text(tag: &'static str, label: &'static str, value: impl Into<String>) -> Self {
        let value = value.into();
        let canonical_bytes = value.as_bytes().to_vec();
        Self {
            tag,
            label,
            value,
            canonical_bytes,
        }
    }

    fn num(tag: &'static str, label: &'static str, n: u64) -> Self {
        Self {
            tag,
            label,
            value: n.to_string(),
            canonical_bytes: n.to_be_bytes().to_vec(),
        }
    }

    fn bytes(tag: &'static str, label: &'static str, bytes: &[u8]) -> Self {
        Self {
            tag,
            label,
            value: format!("0x{}", hex_lower(bytes)),
            canonical_bytes: bytes.to_vec(),
        }
    }
}

/// Project a decoded transaction into the ordered, signing-relevant field list
/// shared by the renderer and the canonical encoder.
pub(crate) fn project(tx: &DecodedTransaction) -> Vec<Field> {
    match tx {
        DecodedTransaction::Evm(evm) => {
            let mut fields = vec![
                Field::num("chain_id", "Chain ID", evm.chain_id),
                Field::num("nonce", "Nonce", evm.nonce),
                Field::num("tx_type", "Tx Type", u64::from(evm.tx_type)),
                Field::text(
                    "to",
                    "To",
                    match evm.to {
                        Some(addr) => format!("0x{}", hex_lower(&addr.0)),
                        None => "(contract creation)".to_string(),
                    },
                ),
                Field::bytes("value", "Value (wei)", &evm.value),
                Field::bytes("data", "Data", &evm.data),
                Field::num("gas_limit", "Gas Limit", evm.gas_limit),
            ];
            if let Some(gp) = &evm.gas_price {
                fields.push(Field::bytes("gas_price", "Gas Price", gp));
            }
            if let Some(mf) = &evm.max_fee_per_gas {
                fields.push(Field::bytes("max_fee_per_gas", "Max Fee/Gas", mf));
            }
            if let Some(mp) = &evm.max_priority_fee_per_gas {
                fields.push(Field::bytes(
                    "max_priority_fee_per_gas",
                    "Max Priority Fee/Gas",
                    mp,
                ));
            }
            for (i, entry) in evm.access_list.iter().enumerate() {
                fields.push(Field::bytes(
                    "access_list.address",
                    "Access List Address",
                    &entry.address.0,
                ));
                for key in &entry.storage_keys {
                    fields.push(Field::bytes(
                        "access_list.storage_key",
                        "Access List Storage Key",
                        &key.0,
                    ));
                }
                // Bind the entry index so reordering changes the hash.
                fields.push(Field::num(
                    "access_list.index",
                    "Access List Index",
                    i as u64,
                ));
            }
            if let Some(mb) = &evm.max_fee_per_blob_gas {
                fields.push(Field::bytes("max_fee_per_blob_gas", "Max Fee/Blob Gas", mb));
            }
            for hash in &evm.blob_versioned_hashes {
                fields.push(Field::bytes(
                    "blob_versioned_hash",
                    "Blob Versioned Hash",
                    &hash.0,
                ));
            }
            fields
        }
        DecodedTransaction::Solana(sol) => {
            let mut fields = vec![
                Field::text("cluster", "Cluster", sol.cluster.clone()),
                Field::bytes(
                    "recent_blockhash",
                    "Recent Blockhash",
                    &sol.recent_blockhash.0,
                ),
            ];
            for (i, key) in sol.account_keys.iter().enumerate() {
                fields.push(Field::bytes("account_key", "Account Key", &key.0));
                fields.push(Field::num(
                    "account_key.index",
                    "Account Key Index",
                    i as u64,
                ));
            }
            for (i, ix) in sol.instructions.iter().enumerate() {
                fields.push(Field::num(
                    "instruction.index",
                    "Instruction Index",
                    i as u64,
                ));
                fields.push(Field::bytes(
                    "instruction.program_id",
                    "Instruction Program",
                    &ix.program_id.0,
                ));
                for acct in &ix.accounts {
                    fields.push(Field::bytes(
                        "instruction.account",
                        "Instruction Account",
                        &acct.0,
                    ));
                }
                fields.push(Field::bytes(
                    "instruction.data",
                    "Instruction Data",
                    &ix.data,
                ));
            }
            if let Some(limit) = sol.compute_unit_limit {
                fields.push(Field::num(
                    "compute_unit_limit",
                    "Compute Unit Limit",
                    u64::from(limit),
                ));
            }
            if let Some(price) = sol.compute_unit_price {
                fields.push(Field::num(
                    "compute_unit_price",
                    "Compute Unit Price (micro-lamports)",
                    price,
                ));
            }
            fields
        }
        DecodedTransaction::Near(near) => {
            let mut fields = vec![
                Field::text("network", "Network", near.network.clone()),
                Field::text("signer_id", "Signer", near.signer_id.clone()),
                Field::text("receiver_id", "Receiver", near.receiver_id.clone()),
                Field::num("nonce", "Access-Key Nonce", near.nonce),
                Field::bytes("block_hash", "Block Hash", &near.block_hash.0),
            ];
            for (i, action) in near.actions.iter().enumerate() {
                fields.push(Field::num("action.index", "Action Index", i as u64));
                fields.push(Field::text(
                    "action.kind",
                    "Action Kind",
                    action.kind.clone(),
                ));
                if !action.method_name.is_empty() {
                    fields.push(Field::text(
                        "action.method_name",
                        "Method",
                        action.method_name.clone(),
                    ));
                }
                if !action.args.is_empty() {
                    fields.push(Field::bytes("action.args", "Args", &action.args));
                }
                fields.push(Field::bytes(
                    "action.deposit",
                    "Deposit (yocto)",
                    &action.deposit,
                ));
                fields.push(Field::num("action.gas", "Gas", action.gas));
            }
            fields
        }
    }
}

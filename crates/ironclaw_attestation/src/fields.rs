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
//!
//! ## Full-fidelity binding
//!
//! For Solana and NEAR the projection additionally binds the EXACT signed wire
//! bytes (via [`crate::wire`]) as a dedicated field, so the canonical bytes
//! commit to the precise message a wallet/HSM signs — every header count,
//! account index, ALT descriptor, and action field. The per-field human
//! projection remains for the render; the wire field guarantees injectivity.

use crate::decoded_tx::{
    DecodedTransaction, NearAccessKeyPermission, NearAction, NearPublicKey, SolanaMessageVersion,
    hex_lower,
};
use crate::wire::{near_transaction_bytes, solana_message_bytes};

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
            let version_label = match sol.version {
                SolanaMessageVersion::Legacy => "legacy",
                SolanaMessageVersion::V0 => "v0",
            };
            let mut fields = vec![
                Field::text("cluster", "Cluster", sol.cluster.clone()),
                Field::text("version", "Message Version", version_label),
                Field::num(
                    "header.num_required_signatures",
                    "Required Signatures",
                    u64::from(sol.header.num_required_signatures),
                ),
                Field::num(
                    "header.num_readonly_signed",
                    "Readonly Signed Accounts",
                    u64::from(sol.header.num_readonly_signed_accounts),
                ),
                Field::num(
                    "header.num_readonly_unsigned",
                    "Readonly Unsigned Accounts",
                    u64::from(sol.header.num_readonly_unsigned_accounts),
                ),
                Field::bytes(
                    "recent_blockhash",
                    "Recent Blockhash",
                    &sol.recent_blockhash.0,
                ),
            ];
            for (i, key) in sol.static_account_keys.iter().enumerate() {
                fields.push(Field::bytes(
                    "static_account_key",
                    "Static Account Key",
                    &key.0,
                ));
                fields.push(Field::num(
                    "static_account_key.index",
                    "Static Account Key Index",
                    i as u64,
                ));
            }
            for (i, ix) in sol.instructions.iter().enumerate() {
                fields.push(Field::num(
                    "instruction.index",
                    "Instruction Index",
                    i as u64,
                ));
                fields.push(Field::num(
                    "instruction.program_id_index",
                    "Instruction Program Index",
                    u64::from(ix.program_id_index),
                ));
                fields.push(Field::bytes(
                    "instruction.account_indices",
                    "Instruction Account Indices",
                    &ix.account_indices,
                ));
                fields.push(Field::bytes(
                    "instruction.data",
                    "Instruction Data",
                    &ix.data,
                ));
            }
            for (i, lookup) in sol.address_table_lookups.iter().enumerate() {
                fields.push(Field::num(
                    "address_table_lookup.index",
                    "Address Table Lookup Index",
                    i as u64,
                ));
                fields.push(Field::bytes(
                    "address_table_lookup.account_key",
                    "Lookup Table Account",
                    &lookup.account_key.0,
                ));
                fields.push(Field::bytes(
                    "address_table_lookup.writable_indexes",
                    "Lookup Writable Indexes",
                    &lookup.writable_indexes,
                ));
                fields.push(Field::bytes(
                    "address_table_lookup.readonly_indexes",
                    "Lookup Readonly Indexes",
                    &lookup.readonly_indexes,
                ));
            }
            // Bind the EXACT signed message bytes so two distinct messages can
            // never collapse to the same projection.
            fields.push(Field {
                tag: "solana.message_bytes",
                label: "Signed Message Bytes",
                value: format!("0x{}", hex_lower(&solana_message_bytes(sol))),
                canonical_bytes: solana_message_bytes(sol),
            });
            fields
        }
        DecodedTransaction::Near(near) => {
            let mut fields = vec![
                Field::text("network", "Network", near.network.clone()),
                Field::text("signer_id", "Signer", near.signer_id.clone()),
                public_key_field("public_key", "Public Key", &near.public_key),
                Field::text("receiver_id", "Receiver", near.receiver_id.clone()),
                Field::num("nonce", "Access-Key Nonce", near.nonce),
                Field::bytes("block_hash", "Block Hash", &near.block_hash.0),
            ];
            for (i, action) in near.actions.iter().enumerate() {
                fields.push(Field::num("action.index", "Action Index", i as u64));
                fields.push(Field::text(
                    "action.kind",
                    "Action Kind",
                    action.kind_label(),
                ));
                project_near_action(&mut fields, action);
            }
            // Bind the EXACT borsh-signed transaction bytes.
            fields.push(Field {
                tag: "near.transaction_bytes",
                label: "Signed Transaction Bytes",
                value: format!("0x{}", hex_lower(&near_transaction_bytes(near))),
                canonical_bytes: near_transaction_bytes(near),
            });
            fields
        }
    }
}

/// Project a NEAR public key into a single human-readable + canonical field.
fn public_key_field(tag: &'static str, label: &'static str, pk: &NearPublicKey) -> Field {
    let mut canonical = Vec::with_capacity(1 + pk.data.len());
    canonical.push(pk.key_type);
    canonical.extend_from_slice(&pk.data);
    Field {
        tag,
        label,
        value: format!("{}:0x{}", pk.key_type, hex_lower(&pk.data)),
        canonical_bytes: canonical,
    }
}

/// Expand a NEAR action's per-variant fields into the projection.
fn project_near_action(fields: &mut Vec<Field>, action: &NearAction) {
    match action {
        NearAction::CreateAccount => {}
        NearAction::DeployContract { code } => {
            fields.push(Field::bytes("action.code", "Contract Code", code));
        }
        NearAction::FunctionCall {
            method_name,
            args,
            gas,
            deposit,
        } => {
            fields.push(Field::text(
                "action.method_name",
                "Method",
                method_name.clone(),
            ));
            fields.push(Field::bytes("action.args", "Args", args));
            fields.push(Field::num("action.gas", "Gas", *gas));
            fields.push(Field::bytes("action.deposit", "Deposit (yocto)", deposit));
        }
        NearAction::Transfer { deposit } => {
            fields.push(Field::bytes("action.deposit", "Deposit (yocto)", deposit));
        }
        NearAction::Stake { stake, public_key } => {
            fields.push(Field::bytes("action.stake", "Stake (yocto)", stake));
            fields.push(public_key_field(
                "action.public_key",
                "Validator Public Key",
                public_key,
            ));
        }
        NearAction::AddKey {
            public_key,
            access_key,
        } => {
            fields.push(public_key_field(
                "action.public_key",
                "New Public Key",
                public_key,
            ));
            fields.push(Field::num(
                "action.access_key.nonce",
                "Access Key Nonce",
                access_key.nonce,
            ));
            match &access_key.permission {
                NearAccessKeyPermission::FunctionCall {
                    allowance,
                    receiver_id,
                    method_names,
                } => {
                    fields.push(Field::text(
                        "action.access_key.permission",
                        "Access Key Permission",
                        "FunctionCall",
                    ));
                    match allowance {
                        Some(a) => {
                            fields.push(Field::bytes(
                                "action.access_key.allowance",
                                "Allowance (yocto)",
                                a,
                            ));
                        }
                        None => {
                            fields.push(Field::text(
                                "action.access_key.allowance",
                                "Allowance (yocto)",
                                "(unlimited)",
                            ));
                        }
                    }
                    fields.push(Field::text(
                        "action.access_key.receiver_id",
                        "Allowed Contract",
                        receiver_id.clone(),
                    ));
                    for (i, name) in method_names.iter().enumerate() {
                        fields.push(Field::num(
                            "action.access_key.method_name.index",
                            "Allowed Method Index",
                            i as u64,
                        ));
                        fields.push(Field::text(
                            "action.access_key.method_name",
                            "Allowed Method",
                            name.clone(),
                        ));
                    }
                }
                NearAccessKeyPermission::FullAccess => {
                    fields.push(Field::text(
                        "action.access_key.permission",
                        "Access Key Permission",
                        "FullAccess",
                    ));
                }
            }
        }
        NearAction::DeleteKey { public_key } => {
            fields.push(public_key_field(
                "action.public_key",
                "Deleted Public Key",
                public_key,
            ));
        }
        NearAction::DeleteAccount { beneficiary_id } => {
            fields.push(Field::text(
                "action.beneficiary_id",
                "Beneficiary",
                beneficiary_id.clone(),
            ));
        }
        NearAction::Delegate {
            sender_id,
            receiver_id,
            nonce,
            max_block_height,
            public_key,
        } => {
            fields.push(Field::text(
                "action.delegate.sender_id",
                "Delegate Sender",
                sender_id.clone(),
            ));
            fields.push(Field::text(
                "action.delegate.receiver_id",
                "Delegate Receiver",
                receiver_id.clone(),
            ));
            fields.push(Field::num(
                "action.delegate.nonce",
                "Delegate Nonce",
                *nonce,
            ));
            fields.push(Field::num(
                "action.delegate.max_block_height",
                "Delegate Max Block Height",
                *max_block_height,
            ));
            fields.push(public_key_field(
                "action.delegate.public_key",
                "Delegate Public Key",
                public_key,
            ));
        }
    }
}

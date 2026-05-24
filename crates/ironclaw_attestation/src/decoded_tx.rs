//! The chain-TAGGED, chain-SDK-FREE intermediate representation of a decoded
//! transaction.
//!
//! PR6's per-chain decoders populate these structs from real chain SDK types;
//! this crate only ever sees the plain serde-friendly projection (`Vec<u8>`,
//! `String`, `u64`, typed newtypes). Keeping the model SDK-free is what lets
//! the attestation crate stay at the base of the dependency graph (see the
//! architecture boundary test).
//!
//! Every field carried here is **signing-relevant**: it either changes the
//! canonical signing bytes a wallet/HSM would produce, or it is part of the
//! human-facing render. The shared field-projection in
//! [`crate::fields`] is what guarantees the renderer and the canonical encoder
//! consume exactly the same set (the anti "approve view A, sign bytes B"
//! property).

use serde::{Deserialize, Serialize};

/// Version of the rendering / canonical-encoding schema.
///
/// Participates in the [`crate::ApprovedTxHash`]: bumping the schema version
/// changes the bound hash even if every transaction field is identical, so an
/// approval rendered under one schema can never be replayed under another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RenderingSchemaVersion(pub u16);

impl RenderingSchemaVersion {
    /// The current schema version emitted by this crate.
    pub const CURRENT: RenderingSchemaVersion = RenderingSchemaVersion(1);

    /// Borrow the raw version number.
    pub fn get(self) -> u16 {
        self.0
    }
}

/// A 20-byte EVM address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvmAddress(pub [u8; 20]);

/// A 32-byte Solana / NEAR-style public key or hash, stored raw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bytes32(pub [u8; 32]);

/// An EVM access-list entry (EIP-2930 / EIP-1559).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmAccessListEntry {
    /// Account the storage keys belong to.
    pub address: EvmAddress,
    /// Pre-warmed storage slots.
    pub storage_keys: Vec<Bytes32>,
}

/// An EVM transaction projected to signing-relevant fields only.
///
/// Covers legacy, EIP-2930, EIP-1559, and EIP-4844 (blob) shapes; fields that
/// do not apply to a given `tx_type` are left `None` / empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmTransaction {
    /// EIP-155 chain id.
    pub chain_id: u64,
    /// Account nonce.
    pub nonce: u64,
    /// EIP-2718 transaction type byte (0 legacy, 1 access-list, 2 fee-market,
    /// 3 blob).
    pub tx_type: u8,
    /// Recipient. `None` for contract creation.
    pub to: Option<EvmAddress>,
    /// Value in wei, big-endian minimal bytes.
    pub value: Vec<u8>,
    /// Call data.
    pub data: Vec<u8>,
    /// Gas limit.
    pub gas_limit: u64,
    /// Legacy gas price (big-endian wei). Set for legacy / 2930 only.
    pub gas_price: Option<Vec<u8>>,
    /// EIP-1559 max fee per gas (big-endian wei).
    pub max_fee_per_gas: Option<Vec<u8>>,
    /// EIP-1559 max priority fee per gas (big-endian wei).
    pub max_priority_fee_per_gas: Option<Vec<u8>>,
    /// EIP-2930 / EIP-1559 access list.
    pub access_list: Vec<EvmAccessListEntry>,
    /// EIP-4844 max fee per blob gas (big-endian wei).
    pub max_fee_per_blob_gas: Option<Vec<u8>>,
    /// EIP-4844 blob versioned hashes.
    pub blob_versioned_hashes: Vec<Bytes32>,
}

/// A single Solana instruction projected to signing-relevant fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaInstruction {
    /// Program account that will execute this instruction.
    pub program_id: Bytes32,
    /// Ordered account references (already ALT-resolved to absolute pubkeys).
    pub accounts: Vec<Bytes32>,
    /// Opaque instruction data.
    pub data: Vec<u8>,
}

/// A Solana transaction message projected to signing-relevant fields.
///
/// Address-lookup-table references are assumed already resolved to absolute
/// account keys by the PR6 decoder — this crate never resolves ALTs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaTransaction {
    /// Cluster identity (e.g. `mainnet-beta`, `devnet`).
    pub cluster: String,
    /// Fully-resolved ordered account keys for the message.
    pub account_keys: Vec<Bytes32>,
    /// Recent blockhash the message is bound to.
    pub recent_blockhash: Bytes32,
    /// Ordered instructions.
    pub instructions: Vec<SolanaInstruction>,
    /// Compute-unit limit (`None` if no compute-budget instruction).
    pub compute_unit_limit: Option<u32>,
    /// Compute-unit price in micro-lamports (`None` if unset).
    pub compute_unit_price: Option<u64>,
}

/// A single NEAR action projected to signing-relevant fields.
///
/// Kept as a tagged value carrying the action discriminant plus the
/// quantity-bearing fields (`deposit`, `gas`); per-action argument bytes ride
/// in `args`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearAction {
    /// Action kind label (e.g. `Transfer`, `FunctionCall`, `AddKey`).
    pub kind: String,
    /// Method name for `FunctionCall`, else empty.
    pub method_name: String,
    /// Argument bytes (function-call args, etc.).
    pub args: Vec<u8>,
    /// Attached deposit in yoctoNEAR, big-endian minimal bytes.
    pub deposit: Vec<u8>,
    /// Prepaid gas for `FunctionCall` (0 otherwise).
    pub gas: u64,
}

/// A NEAR transaction projected to signing-relevant fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearTransaction {
    /// Network identity (e.g. `mainnet`, `testnet`).
    pub network: String,
    /// Account authorizing the transaction.
    pub signer_id: String,
    /// Target account.
    pub receiver_id: String,
    /// Access-key nonce.
    pub nonce: u64,
    /// Block hash the transaction is anchored to.
    pub block_hash: Bytes32,
    /// Ordered actions.
    pub actions: Vec<NearAction>,
}

/// A server-decoded transaction in a chain-tagged, SDK-free form.
///
/// This is the concrete type the attestation layer renders and hashes. (The
/// `ironclaw_signing_provider` crate exposes an opaque forward-declared
/// `DecodedTransaction` so the trait can name decode output without a chain
/// dependency; this is the real, populated model that lives one layer up.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "chain", rename_all = "lowercase")]
pub enum DecodedTransaction {
    /// An EVM transaction.
    Evm(EvmTransaction),
    /// A Solana transaction.
    Solana(SolanaTransaction),
    /// A NEAR transaction.
    Near(NearTransaction),
}

impl DecodedTransaction {
    /// Stable, domain-separated chain tag used in both the canonical encoding
    /// and the rendered view. This is the chain/network domain separator that
    /// makes coincidentally-similar bytes on different chains hash differently.
    pub fn chain_tag(&self) -> &'static str {
        match self {
            DecodedTransaction::Evm(_) => "evm",
            DecodedTransaction::Solana(_) => "solana",
            DecodedTransaction::Near(_) => "near",
        }
    }

    /// Human-readable transaction-type label bound into the hash.
    pub fn tx_type_label(&self) -> String {
        match self {
            DecodedTransaction::Evm(tx) => format!("evm-type-{}", tx.tx_type),
            DecodedTransaction::Solana(_) => "solana-message".to_string(),
            DecodedTransaction::Near(tx) => {
                let kinds: Vec<&str> = tx.actions.iter().map(|a| a.kind.as_str()).collect();
                format!("near-actions[{}]", kinds.join(","))
            }
        }
    }

    /// Chain/network identity bound into the hash (the network domain
    /// separator). EVM uses the numeric chain id; Solana/NEAR carry a string
    /// cluster/network.
    pub fn chain_network(&self) -> String {
        match self {
            DecodedTransaction::Evm(tx) => format!("eip155:{}", tx.chain_id),
            DecodedTransaction::Solana(tx) => format!("solana:{}", tx.cluster),
            DecodedTransaction::Near(tx) => format!("near:{}", tx.network),
        }
    }

    /// Signer / account identity bound into the hash.
    pub fn signer_account(&self) -> String {
        match self {
            DecodedTransaction::Evm(tx) => match tx.to {
                // The EVM `from` is recovered from the signature at sign time
                // (PR6, threat #5); the decode model has no `from`. The
                // signer/account bound into the hash is supplied by the caller
                // context, so here we surface the recipient for the render and
                // leave signer binding to the explicit hash argument.
                Some(addr) => format!("0x{}", hex_lower(&addr.0)),
                None => "contract-creation".to_string(),
            },
            DecodedTransaction::Solana(tx) => tx
                .account_keys
                .first()
                .map(|k| hex_lower(&k.0))
                .unwrap_or_default(),
            DecodedTransaction::Near(tx) => tx.signer_id.clone(),
        }
    }
}

/// Lowercase hex without allocation churn beyond the result string.
pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

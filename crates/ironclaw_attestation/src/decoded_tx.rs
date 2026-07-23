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
//!
//! ## Full-fidelity message modeling
//!
//! Solana and NEAR are modeled at *full signing fidelity*: the Solana variant
//! carries the versioned-message header, static account keys, address-table
//! lookups, and compiled instructions (program/account *indices*, not resolved
//! pubkeys); the NEAR variant carries the transaction `public_key` and a typed
//! action enum covering every NEAR action. This is what lets
//! [`crate::canonical::canonical_signing_bytes`] reproduce the EXACT bytes the
//! wallet/HSM signs over, so two distinct signed messages can never collapse to
//! the same projection (the non-injective-projection class of bugs).
//!
//! ## Unknown-field rejection
//!
//! Every struct here derives `#[serde(deny_unknown_fields)]`. A decoded
//! transaction that carries an extra/unknown field (threats #8 field-smuggling,
//! #9 hidden-field) is rejected at deserialize time rather than silently dropped
//! before canonicalization.

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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

/// The Solana message version. Bound into the canonical bytes; a legacy message
/// and a v0 message with otherwise-identical contents serialize differently and
/// so can never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SolanaMessageVersion {
    /// Legacy (pre-versioned) message. No address-table lookups permitted.
    Legacy,
    /// Versioned message, version 0. May carry address-table lookups.
    V0,
}

/// The Solana message header (`MessageHeader`): the three signer/readonly
/// counts that prefix every Solana message and determine which accounts must
/// sign and which are read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolanaMessageHeader {
    /// Number of accounts (from the front of the static key list) that must
    /// sign the transaction.
    pub num_required_signatures: u8,
    /// Of the required signers, how many are read-only.
    pub num_readonly_signed_accounts: u8,
    /// Of the non-signer static keys, how many are read-only.
    pub num_readonly_unsigned_accounts: u8,
}

/// A single compiled Solana instruction, referencing accounts by *index* into
/// the combined account list (static keys ∥ writable ALT ∥ readonly ALT) — the
/// exact form carried in the signed message. The crate never resolves these to
/// absolute pubkeys (that would require chain SDK / RPC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolanaCompiledInstruction {
    /// Index into the account list of the program that executes this
    /// instruction.
    pub program_id_index: u8,
    /// Ordered indices into the account list for this instruction's accounts.
    pub account_indices: Vec<u8>,
    /// Opaque instruction data.
    pub data: Vec<u8>,
}

/// An address-table-lookup descriptor (v0 messages): the lookup-table account
/// plus the writable / readonly index lists used to extend the account list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolanaAddressTableLookup {
    /// The address-lookup-table account key.
    pub account_key: Bytes32,
    /// Indices into the table for writable accounts.
    pub writable_indexes: Vec<u8>,
    /// Indices into the table for readonly accounts.
    pub readonly_indexes: Vec<u8>,
}

/// A Solana transaction *message* modeled at full signing fidelity.
///
/// This mirrors the on-chain `Message` / `MessageV0` wire layout exactly: the
/// version, header, static account keys, recent blockhash, compiled
/// instructions (account *indices*), and (v0 only) address-table lookups. The
/// canonical encoder reproduces the precise bytes the wallet signs over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolanaTransaction {
    /// Cluster identity (e.g. `mainnet-beta`, `devnet`). Domain separator; not
    /// part of the on-wire signed bytes but bound into the hash.
    pub cluster: String,
    /// Message version (legacy vs v0).
    pub version: SolanaMessageVersion,
    /// Message header (signer / readonly counts).
    pub header: SolanaMessageHeader,
    /// Static account keys carried directly in the message.
    pub static_account_keys: Vec<Bytes32>,
    /// Recent blockhash the message is bound to.
    pub recent_blockhash: Bytes32,
    /// Ordered compiled instructions (account references by index).
    pub instructions: Vec<SolanaCompiledInstruction>,
    /// Address-table lookups (v0 only; MUST be empty for legacy messages).
    pub address_table_lookups: Vec<SolanaAddressTableLookup>,
}

/// A NEAR public key (ed25519 or secp256k1), carried at full fidelity so the
/// transaction `public_key` and key-bearing actions serialize byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NearPublicKey {
    /// Borsh key-type discriminant (0 = ED25519, 1 = SECP256K1).
    pub key_type: u8,
    /// Raw key bytes (32 for ed25519, 64 for secp256k1).
    pub data: Vec<u8>,
}

/// The access-key permission for a NEAR `AddKey` action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum NearAccessKeyPermission {
    /// Function-call access key: optional allowance, a receiver contract, and
    /// an allowed method-name list (empty = any method).
    FunctionCall {
        /// Allowance in yoctoNEAR, big-endian minimal bytes; `None` =
        /// unlimited.
        allowance: Option<Vec<u8>>,
        /// Contract the key may call.
        receiver_id: String,
        /// Allowed method names (empty = all methods).
        method_names: Vec<String>,
    },
    /// Full-access key.
    FullAccess,
}

/// A NEAR access key (nonce + permission) for `AddKey`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NearAccessKey {
    /// Access-key nonce.
    pub nonce: u64,
    /// Permission granted to the key.
    pub permission: NearAccessKeyPermission,
}

/// A NEAR action, typed across every action variant, carrying every serialized
/// field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum NearAction {
    /// Create the receiver account.
    CreateAccount,
    /// Deploy contract code to the receiver account.
    DeployContract {
        /// WASM contract code.
        code: Vec<u8>,
    },
    /// Call a contract method.
    FunctionCall {
        /// Method name.
        method_name: String,
        /// Argument bytes.
        args: Vec<u8>,
        /// Prepaid gas.
        gas: u64,
        /// Attached deposit in yoctoNEAR, big-endian minimal bytes.
        deposit: Vec<u8>,
    },
    /// Transfer NEAR.
    Transfer {
        /// Amount in yoctoNEAR, big-endian minimal bytes.
        deposit: Vec<u8>,
    },
    /// Stake.
    Stake {
        /// Stake amount in yoctoNEAR, big-endian minimal bytes.
        stake: Vec<u8>,
        /// Validator public key.
        public_key: NearPublicKey,
    },
    /// Add an access key.
    AddKey {
        /// Public key to add.
        public_key: NearPublicKey,
        /// Access key (nonce + permission).
        access_key: NearAccessKey,
    },
    /// Remove an access key.
    DeleteKey {
        /// Public key to remove.
        public_key: NearPublicKey,
    },
    /// Delete the account, sweeping balance to the beneficiary.
    DeleteAccount {
        /// Beneficiary account id.
        beneficiary_id: String,
    },
    /// Meta-transaction delegate action (NEP-366). Carries the full inner
    /// `DelegateAction`, including its inner actions, so the signed commitment
    /// is injective over them.
    Delegate {
        /// The account whose key signs the delegate action.
        sender_id: String,
        /// The receiver of the delegated actions.
        receiver_id: String,
        /// The inner actions the delegate authorizes. NEP-366 types these as
        /// `NonDelegateAction`: a delegate may not nest another delegate. A
        /// nested `Delegate` here is unrepresentable on-chain and is rejected
        /// (fail closed) at canonicalization time rather than serialized.
        actions: Vec<NearAction>,
        /// Nonce for the delegate action.
        nonce: u64,
        /// Block height past which the delegate action is invalid.
        max_block_height: u64,
        /// Public key authorizing the delegate action.
        public_key: NearPublicKey,
    },
}

impl NearAction {
    /// Stable label for the action variant (used in the tx-type render/binding).
    pub(crate) fn kind_label(&self) -> &'static str {
        match self {
            NearAction::CreateAccount => "CreateAccount",
            NearAction::DeployContract { .. } => "DeployContract",
            NearAction::FunctionCall { .. } => "FunctionCall",
            NearAction::Transfer { .. } => "Transfer",
            NearAction::Stake { .. } => "Stake",
            NearAction::AddKey { .. } => "AddKey",
            NearAction::DeleteKey { .. } => "DeleteKey",
            NearAction::DeleteAccount { .. } => "DeleteAccount",
            NearAction::Delegate { .. } => "Delegate",
        }
    }
}

/// A NEAR transaction projected to signing-relevant fields, including the
/// transaction `public_key` (omitting it previously made the projection
/// non-injective across signing keys).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NearTransaction {
    /// Network identity (e.g. `mainnet`, `testnet`).
    pub network: String,
    /// Account authorizing the transaction.
    pub signer_id: String,
    /// Public key of the signer's access key.
    pub public_key: NearPublicKey,
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
#[serde(tag = "chain", rename_all = "lowercase", deny_unknown_fields)]
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
            DecodedTransaction::Solana(tx) => match tx.version {
                SolanaMessageVersion::Legacy => "solana-message-legacy".to_string(),
                SolanaMessageVersion::V0 => "solana-message-v0".to_string(),
            },
            DecodedTransaction::Near(tx) => {
                let kinds: Vec<&str> = tx.actions.iter().map(NearAction::kind_label).collect();
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

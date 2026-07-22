//! EVM native transaction -> PR2 [`DecodedTransaction`].
//!
//! Populates every signing-relevant field the canonical encoder consumes from
//! an alloy typed request, so the render and the canonical bytes cover exactly
//! what will be signed.

use alloy_consensus::{SignableTransaction, TxEip1559, TxEip2930, TxLegacy};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256};

use ironclaw_attestation::{
    Bytes32, DecodedTransaction, EvmAccessListEntry, EvmAddress, EvmTransaction,
};

use crate::error::ChainSigningError;

/// Big-endian minimal-byte encoding of a `u128` wei value (matching PR2's
/// `Vec<u8>` value fields). Trims leading zero bytes; an all-zero value encodes
/// as an empty vector, exactly like RLP integer minimality.
fn be_minimal_u128(value: u128) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len());
    bytes[first..].to_vec()
}

/// Big-endian minimal-byte encoding of a `U256` (for the `value` field).
fn be_minimal_u256(value: alloy_primitives::U256) -> Vec<u8> {
    let bytes: [u8; 32] = value.to_be_bytes();
    let first = bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len());
    bytes[first..].to_vec()
}

fn to_field(kind: TxKind) -> Option<EvmAddress> {
    match kind {
        TxKind::Create => None,
        TxKind::Call(addr) => Some(EvmAddress(addr.into_array())),
    }
}

fn access_list_entries(access_list: &alloy_eips::eip2930::AccessList) -> Vec<EvmAccessListEntry> {
    access_list
        .0
        .iter()
        .map(|item| EvmAccessListEntry {
            address: EvmAddress(item.address.into_array()),
            storage_keys: item.storage_keys.iter().map(|k| Bytes32(k.0)).collect(),
        })
        .collect()
}

/// Decode an EIP-1559 (fee-market) transaction.
pub fn decode_eip1559(tx: &TxEip1559) -> DecodedTransaction {
    DecodedTransaction::Evm(EvmTransaction {
        chain_id: tx.chain_id,
        nonce: tx.nonce,
        tx_type: 2,
        to: to_field(tx.to),
        value: be_minimal_u256(tx.value),
        data: tx.input.to_vec(),
        gas_limit: tx.gas_limit,
        gas_price: None,
        max_fee_per_gas: Some(be_minimal_u128(tx.max_fee_per_gas)),
        max_priority_fee_per_gas: Some(be_minimal_u128(tx.max_priority_fee_per_gas)),
        access_list: access_list_entries(&tx.access_list),
        max_fee_per_blob_gas: None,
        blob_versioned_hashes: Vec::new(),
    })
}

/// Decode a legacy transaction.
pub fn decode_legacy(tx: &TxLegacy) -> DecodedTransaction {
    DecodedTransaction::Evm(EvmTransaction {
        // Legacy txs carry an optional chain id (EIP-155). `None` (pre-155)
        // maps to 0, which the policy layer treats as a replay-unprotected tx.
        chain_id: tx.chain_id.unwrap_or(0),
        nonce: tx.nonce,
        tx_type: 0,
        to: to_field(tx.to),
        value: be_minimal_u256(tx.value),
        data: tx.input.to_vec(),
        gas_limit: tx.gas_limit,
        gas_price: Some(be_minimal_u128(tx.gas_price)),
        max_fee_per_gas: None,
        max_priority_fee_per_gas: None,
        access_list: Vec::new(),
        max_fee_per_blob_gas: None,
        blob_versioned_hashes: Vec::new(),
    })
}

/// Decode an EIP-2930 (access-list) transaction.
pub fn decode_eip2930(tx: &TxEip2930) -> DecodedTransaction {
    DecodedTransaction::Evm(EvmTransaction {
        chain_id: tx.chain_id,
        nonce: tx.nonce,
        tx_type: 1,
        to: to_field(tx.to),
        value: be_minimal_u256(tx.value),
        data: tx.input.to_vec(),
        gas_limit: tx.gas_limit,
        gas_price: Some(be_minimal_u128(tx.gas_price)),
        max_fee_per_gas: None,
        max_priority_fee_per_gas: None,
        access_list: access_list_entries(&tx.access_list),
        max_fee_per_blob_gas: None,
        blob_versioned_hashes: Vec::new(),
    })
}

/// Parse big-endian minimal bytes back into a `u128` (fees / gas price).
fn u128_from_be_minimal(bytes: &[u8]) -> Result<u128, ChainSigningError> {
    if bytes.len() > 16 {
        return Err(decode_err("u128 field exceeds 16 bytes"));
    }
    let mut buf = [0u8; 16];
    buf[16 - bytes.len()..].copy_from_slice(bytes);
    Ok(u128::from_be_bytes(buf))
}

/// Parse big-endian minimal bytes back into a `U256` (value).
fn u256_from_be_minimal(bytes: &[u8]) -> Result<U256, ChainSigningError> {
    if bytes.len() > 32 {
        return Err(decode_err("u256 field exceeds 32 bytes"));
    }
    Ok(U256::from_be_slice(bytes))
}

fn decode_err(reason: &str) -> ChainSigningError {
    ChainSigningError::Decode {
        chain: "evm",
        reason: reason.to_string(),
    }
}

fn to_kind(addr: Option<EvmAddress>) -> TxKind {
    match addr {
        Some(a) => TxKind::Call(Address::from(a.0)),
        None => TxKind::Create,
    }
}

fn rebuild_access_list(entries: &[EvmAccessListEntry]) -> alloy_eips::eip2930::AccessList {
    alloy_eips::eip2930::AccessList(
        entries
            .iter()
            .map(|e| alloy_eips::eip2930::AccessListItem {
                address: Address::from(e.address.0),
                storage_keys: e.storage_keys.iter().map(|k| B256::from(k.0)).collect(),
            })
            .collect(),
    )
}

/// The reconstructed, signable alloy transaction for an [`EvmTransaction`]
/// projection, tagged by EIP-2718 type.
///
/// This is the REVERSE of [`decode_eip1559`] / [`decode_legacy`] /
/// [`decode_eip2930`]: it rebuilds the exact alloy typed transaction from the
/// PR2 [`EvmTransaction`] projection. Signing operates on the digest of THIS
/// reconstruction, so the bytes signed are derived from the same decoded
/// transaction whose [`ironclaw_attestation::canonical_signing_bytes`] produced
/// the approved hash — there is no separate, caller-supplied signable tx that
/// could drift from the approved one (review finding #1).
pub(crate) enum RebuiltEvmTx {
    /// Legacy (type 0).
    Legacy(Box<TxLegacy>),
    /// EIP-2930 access-list (type 1).
    Eip2930(Box<TxEip2930>),
    /// EIP-1559 fee-market (type 2).
    Eip1559(Box<TxEip1559>),
}

impl RebuiltEvmTx {
    /// The signing digest (keccak256 over the EIP-2718 unsigned payload) for
    /// the reconstructed transaction, exactly as the wallet/HSM would compute.
    pub(crate) fn signature_hash(&self) -> B256 {
        match self {
            RebuiltEvmTx::Legacy(tx) => tx.signature_hash(),
            RebuiltEvmTx::Eip2930(tx) => tx.signature_hash(),
            RebuiltEvmTx::Eip1559(tx) => tx.signature_hash(),
        }
    }
}

/// Reconstruct the signable alloy transaction from the PR2 EVM projection.
///
/// Fails closed if the projection is internally inconsistent (e.g. a type-2 tx
/// missing its fee fields, an over-long integer field). The reconstruction is
/// deterministic: the same projection always rebuilds to the same bytes.
pub(crate) fn rebuild_signable(tx: &EvmTransaction) -> Result<RebuiltEvmTx, ChainSigningError> {
    let value = u256_from_be_minimal(&tx.value)?;
    let to = to_kind(tx.to);
    let input = Bytes::from(tx.data.clone());
    match tx.tx_type {
        0 => {
            if tx.gas_price.is_none() {
                return Err(decode_err("legacy tx missing gas_price"));
            }
            let gas_price = u128_from_be_minimal(tx.gas_price.as_deref().unwrap_or_default())?;
            // chain_id 0 in the projection means pre-EIP-155 (no replay
            // protection); the policy layer refuses it, but rebuild faithfully.
            let chain_id = if tx.chain_id == 0 {
                None
            } else {
                Some(tx.chain_id)
            };
            Ok(RebuiltEvmTx::Legacy(Box::new(TxLegacy {
                chain_id,
                nonce: tx.nonce,
                gas_price,
                gas_limit: tx.gas_limit,
                to,
                value,
                input,
            })))
        }
        1 => {
            let gas_price = u128_from_be_minimal(
                tx.gas_price
                    .as_deref()
                    .ok_or_else(|| decode_err("2930 tx missing gas_price"))?,
            )?;
            Ok(RebuiltEvmTx::Eip2930(Box::new(TxEip2930 {
                chain_id: tx.chain_id,
                nonce: tx.nonce,
                gas_price,
                gas_limit: tx.gas_limit,
                to,
                value,
                access_list: rebuild_access_list(&tx.access_list),
                input,
            })))
        }
        2 => {
            let max_fee = u128_from_be_minimal(
                tx.max_fee_per_gas
                    .as_deref()
                    .ok_or_else(|| decode_err("1559 tx missing max_fee_per_gas"))?,
            )?;
            let max_priority = u128_from_be_minimal(
                tx.max_priority_fee_per_gas
                    .as_deref()
                    .ok_or_else(|| decode_err("1559 tx missing max_priority_fee_per_gas"))?,
            )?;
            Ok(RebuiltEvmTx::Eip1559(Box::new(TxEip1559 {
                chain_id: tx.chain_id,
                nonce: tx.nonce,
                gas_limit: tx.gas_limit,
                max_fee_per_gas: max_fee,
                max_priority_fee_per_gas: max_priority,
                to,
                value,
                access_list: rebuild_access_list(&tx.access_list),
                input,
            })))
        }
        other => Err(decode_err(&format!("unsupported EVM tx type {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn eip1559_decode_populates_signing_fields() {
        let to: Address = address!("00000000000000000000000000000000000000aa");
        let tx = TxEip1559 {
            chain_id: 1,
            nonce: 7,
            gas_limit: 21000,
            max_fee_per_gas: 100,
            max_priority_fee_per_gas: 2,
            to: TxKind::Call(to),
            value: U256::from(1000u64),
            access_list: Default::default(),
            input: Bytes::from(vec![0xde, 0xad]),
        };
        let decoded = decode_eip1559(&tx);
        let DecodedTransaction::Evm(evm) = &decoded else {
            panic!("expected evm");
        };
        assert_eq!(evm.chain_id, 1);
        assert_eq!(evm.nonce, 7);
        assert_eq!(evm.tx_type, 2);
        assert_eq!(evm.to.unwrap().0, to.into_array());
        assert_eq!(evm.value, vec![0x03, 0xe8]); // 1000 = 0x3e8
        assert_eq!(evm.data, vec![0xde, 0xad]);
        assert_eq!(evm.max_fee_per_gas, Some(vec![100]));
    }

    #[test]
    fn rebuild_signable_rejects_unsupported_tx_type() {
        // Build a valid 1559 projection, then poison the tx_type to an
        // unsupported value. rebuild_signable must fail closed with a Decode
        // error rather than silently rebuilding the wrong envelope.
        let tx = TxEip1559 {
            chain_id: 1,
            nonce: 0,
            gas_limit: 21000,
            max_fee_per_gas: 100,
            max_priority_fee_per_gas: 2,
            to: TxKind::Call(address!("00000000000000000000000000000000000000aa")),
            value: U256::from(1u64),
            access_list: Default::default(),
            input: Bytes::new(),
        };
        let DecodedTransaction::Evm(mut evm) = decode_eip1559(&tx) else {
            panic!("expected evm");
        };
        evm.tx_type = 99;
        assert!(matches!(
            rebuild_signable(&evm),
            Err(ChainSigningError::Decode { .. })
        ));
    }

    #[test]
    fn be_minimal_trims_leading_zeros() {
        assert_eq!(be_minimal_u128(0), Vec::<u8>::new());
        assert_eq!(be_minimal_u128(1), vec![1]);
        assert_eq!(be_minimal_u128(256), vec![1, 0]);
    }

    /// Property for review finding #1: the digest of the transaction rebuilt
    /// from the decoded projection is byte-identical to the digest of the
    /// original native transaction. Signing the rebuilt digest therefore signs
    /// exactly the bytes the approved hash was computed over.
    #[test]
    fn rebuilt_eip1559_signature_hash_matches_native() {
        let native = TxEip1559 {
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
        let decoded = decode_eip1559(&native);
        let DecodedTransaction::Evm(evm) = &decoded else {
            panic!("evm");
        };
        let rebuilt = rebuild_signable(evm).expect("rebuild");
        assert_eq!(rebuilt.signature_hash(), native.signature_hash());
    }

    #[test]
    fn rebuilt_legacy_signature_hash_matches_native() {
        let native = TxLegacy {
            chain_id: Some(1),
            nonce: 4,
            gas_price: 50,
            gas_limit: 21000,
            to: TxKind::Call(address!("00000000000000000000000000000000000000bb")),
            value: U256::from(5u64),
            input: Bytes::from(vec![0x01]),
        };
        let decoded = decode_legacy(&native);
        let DecodedTransaction::Evm(evm) = &decoded else {
            panic!("evm");
        };
        let rebuilt = rebuild_signable(evm).expect("rebuild");
        assert_eq!(rebuilt.signature_hash(), native.signature_hash());
    }

    #[test]
    fn rebuilt_eip2930_signature_hash_matches_native() {
        let native = TxEip2930 {
            chain_id: 1,
            nonce: 9,
            gas_price: 77,
            gas_limit: 30000,
            to: TxKind::Call(address!("00000000000000000000000000000000000000cc")),
            value: U256::from(42u64),
            access_list: Default::default(),
            input: Bytes::new(),
        };
        let decoded = decode_eip2930(&native);
        let DecodedTransaction::Evm(evm) = &decoded else {
            panic!("evm");
        };
        let rebuilt = rebuild_signable(evm).expect("rebuild");
        assert_eq!(rebuilt.signature_hash(), native.signature_hash());
    }
}

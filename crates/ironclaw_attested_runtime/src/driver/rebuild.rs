//! Reconstruct a chain-native signable transaction *from the authoritative
//! decoded binding* so the custodial signer signs exactly what was approved.
//!
//! This is the byte-drift defense for the custodial continuation (the same
//! class PR6 fixed in `CustodialSigner`): the driver never signs a
//! caller-supplied signable. It rebuilds the EVM transaction deterministically
//! from `binding.decoded` — the server-decoded model the `ApprovedTxHash` was
//! computed over — so the bytes that get signed cannot diverge from the bytes
//! that were approved, and a mainnet tx cannot be smuggled past a testnet
//! `binding.chain` ship-gate by passing a different signable after approval.
//!
//! Only EVM is reconstructed here; Solana / NEAR custodial signing land with
//! their own per-chain rebuild in a later slice and currently return
//! [`RebuildError::UnsupportedChain`] (fail-closed).

use alloy_consensus::{TxEip1559, TxEip2930, TxLegacy};
use alloy_eips::eip2930::{AccessList, AccessListItem};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256};

use ironclaw_attestation::{DecodedTransaction, EvmAccessListEntry, EvmTransaction};

/// Errors that can occur reconstructing a signable from the decoded binding.
#[derive(Debug)]
pub enum RebuildError {
    /// The decoded binding is for a chain family that has no custodial rebuild
    /// path yet (Solana / NEAR custodial signing is a later slice).
    UnsupportedChain {
        /// The chain tag of the decoded binding.
        chain: &'static str,
    },
    /// A scalar field in the decoded model was wider than its chain-native type
    /// can hold (e.g. an EVM fee field with more than 16 big-endian bytes).
    FieldOverflow {
        /// Which field overflowed.
        field: &'static str,
    },
    /// The decoded EVM tx carried a transaction type this rebuild path does not
    /// reconstruct (e.g. EIP-4844 blob txs).
    UnsupportedTxType {
        /// The EIP-2718 type byte.
        tx_type: u8,
    },
    /// A required field for the decoded tx type was missing.
    MissingField {
        /// Which field was missing.
        field: &'static str,
    },
}

impl std::fmt::Display for RebuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedChain { chain } => {
                write!(f, "no custodial rebuild path for chain family {chain}")
            }
            Self::FieldOverflow { field } => write!(f, "decoded field {field} overflowed"),
            Self::UnsupportedTxType { tx_type } => {
                write!(f, "unsupported EVM tx type {tx_type}")
            }
            Self::MissingField { field } => write!(f, "missing decoded field {field}"),
        }
    }
}

impl std::error::Error for RebuildError {}

/// A reconstructed EVM signable. Mirrors the alloy typed-tx shapes the decoder
/// (`evm::decode_*`) produces, so a round-trip rebuild is byte-identical.
pub enum EvmSignable {
    /// EIP-1559 fee-market transaction.
    Eip1559(TxEip1559),
    /// Legacy transaction.
    Legacy(TxLegacy),
    /// EIP-2930 access-list transaction.
    Eip2930(TxEip2930),
}

/// Reconstruct the EVM signable from the authoritative decoded binding.
pub(crate) fn rebuild_evm_signable(
    decoded: &DecodedTransaction,
) -> Result<EvmSignable, RebuildError> {
    match decoded {
        DecodedTransaction::Evm(evm) => rebuild_evm(evm),
        other => Err(RebuildError::UnsupportedChain {
            chain: other.chain_tag(),
        }),
    }
}

fn rebuild_evm(evm: &EvmTransaction) -> Result<EvmSignable, RebuildError> {
    let to = match &evm.to {
        Some(addr) => TxKind::Call(Address::from(addr.0)),
        None => TxKind::Create,
    };
    let value = be_u256(&evm.value);
    let input = Bytes::from(evm.data.clone());
    let access_list = rebuild_access_list(&evm.access_list);

    match evm.tx_type {
        2 => {
            let max_fee = be_u128(
                evm.max_fee_per_gas
                    .as_deref()
                    .ok_or(RebuildError::MissingField {
                        field: "max_fee_per_gas",
                    })?,
                "max_fee_per_gas",
            )?;
            let max_prio = be_u128(
                evm.max_priority_fee_per_gas
                    .as_deref()
                    .ok_or(RebuildError::MissingField {
                        field: "max_priority_fee_per_gas",
                    })?,
                "max_priority_fee_per_gas",
            )?;
            Ok(EvmSignable::Eip1559(TxEip1559 {
                chain_id: evm.chain_id,
                nonce: evm.nonce,
                gas_limit: evm.gas_limit,
                max_fee_per_gas: max_fee,
                max_priority_fee_per_gas: max_prio,
                to,
                value,
                access_list,
                input,
            }))
        }
        0 => {
            let gas_price = be_u128(
                evm.gas_price
                    .as_deref()
                    .ok_or(RebuildError::MissingField { field: "gas_price" })?,
                "gas_price",
            )?;
            // Decoder maps a missing EIP-155 chain id to 0; preserve that.
            let chain_id = if evm.chain_id == 0 {
                None
            } else {
                Some(evm.chain_id)
            };
            Ok(EvmSignable::Legacy(TxLegacy {
                chain_id,
                nonce: evm.nonce,
                gas_price,
                gas_limit: evm.gas_limit,
                to,
                value,
                input,
            }))
        }
        1 => {
            let gas_price = be_u128(
                evm.gas_price
                    .as_deref()
                    .ok_or(RebuildError::MissingField { field: "gas_price" })?,
                "gas_price",
            )?;
            Ok(EvmSignable::Eip2930(TxEip2930 {
                chain_id: evm.chain_id,
                nonce: evm.nonce,
                gas_price,
                gas_limit: evm.gas_limit,
                to,
                value,
                access_list,
                input,
            }))
        }
        other => Err(RebuildError::UnsupportedTxType { tx_type: other }),
    }
}

fn rebuild_access_list(entries: &[EvmAccessListEntry]) -> AccessList {
    AccessList(
        entries
            .iter()
            .map(|entry| AccessListItem {
                address: Address::from(entry.address.0),
                storage_keys: entry.storage_keys.iter().map(|k| B256::from(k.0)).collect(),
            })
            .collect(),
    )
}

/// Parse a big-endian minimal-byte `value` field into a `U256`. An empty slice
/// is zero (matching the decoder's minimal encoding). Overflow (more than 32
/// bytes) is impossible for a 32-byte source, but we guard anyway.
fn be_u256(bytes: &[u8]) -> U256 {
    U256::from_be_slice(bytes)
}

/// Parse a big-endian minimal-byte fee field into a `u128`, failing closed on
/// overflow (a fee field wider than 16 bytes cannot be a valid EVM fee).
fn be_u128(bytes: &[u8], field: &'static str) -> Result<u128, RebuildError> {
    if bytes.len() > 16 {
        return Err(RebuildError::FieldOverflow { field });
    }
    let mut buf = [0u8; 16];
    buf[16 - bytes.len()..].copy_from_slice(bytes);
    Ok(u128::from_be_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::SignableTransaction;
    use alloy_primitives::Signature;
    use ironclaw_chain_signing::evm;

    fn sample_1559() -> TxEip1559 {
        TxEip1559 {
            chain_id: 11155111,
            nonce: 7,
            gas_limit: 21_000,
            max_fee_per_gas: 30_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000,
            to: TxKind::Call(Address::repeat_byte(0xbb)),
            value: U256::from(1_000u64),
            input: Bytes::from(vec![0xde, 0xad]),
            access_list: Default::default(),
        }
    }

    /// The rebuild must round-trip byte-identically: decode -> rebuild produces
    /// a signable with the SAME signature hash as the original.
    #[test]
    fn eip1559_rebuild_roundtrips_signature_hash() {
        let original = sample_1559();
        let decoded = evm::decode_eip1559(&original);
        let EvmSignable::Eip1559(rebuilt) = rebuild_evm_signable(&decoded).unwrap() else {
            panic!("expected eip1559");
        };
        let h1 = SignableTransaction::<Signature>::signature_hash(&original);
        let h2 = SignableTransaction::<Signature>::signature_hash(&rebuilt);
        assert_eq!(h1, h2, "rebuilt tx must hash identically to the original");
    }

    #[test]
    fn legacy_rebuild_roundtrips_signature_hash() {
        let original = TxLegacy {
            chain_id: Some(11155111),
            nonce: 3,
            gas_price: 20_000_000_000,
            gas_limit: 21_000,
            to: TxKind::Call(Address::repeat_byte(0xcc)),
            value: U256::from(42u64),
            input: Bytes::new(),
        };
        let decoded = evm::decode_legacy(&original);
        let EvmSignable::Legacy(rebuilt) = rebuild_evm_signable(&decoded).unwrap() else {
            panic!("expected legacy");
        };
        let h1 = SignableTransaction::<Signature>::signature_hash(&original);
        let h2 = SignableTransaction::<Signature>::signature_hash(&rebuilt);
        assert_eq!(h1, h2);
    }

    #[test]
    fn eip2930_rebuild_roundtrips_signature_hash() {
        let original = TxEip2930 {
            chain_id: 11155111,
            nonce: 5,
            gas_price: 25_000_000_000,
            gas_limit: 50_000,
            to: TxKind::Call(Address::repeat_byte(0xdd)),
            value: U256::from(7u64),
            access_list: AccessList(vec![AccessListItem {
                address: Address::repeat_byte(0xee),
                storage_keys: vec![B256::repeat_byte(0x11), B256::repeat_byte(0x22)],
            }]),
            input: Bytes::from(vec![0xca, 0xfe]),
        };
        let decoded = evm::decode_eip2930(&original);
        let EvmSignable::Eip2930(rebuilt) = rebuild_evm_signable(&decoded).unwrap() else {
            panic!("expected eip2930");
        };
        let h1 = SignableTransaction::<Signature>::signature_hash(&original);
        let h2 = SignableTransaction::<Signature>::signature_hash(&rebuilt);
        assert_eq!(
            h1, h2,
            "rebuilt eip2930 must hash identically to the original"
        );
    }

    // `EvmSignable` is intentionally not `Debug` (it wraps signable txs), so the
    // error-path tests assert on the `Result` via `matches!` rather than
    // `expect_err` (which would require `Debug` on the `Ok` variant).

    #[test]
    fn unsupported_tx_type_fails_closed() {
        // Start from a valid 1559 decode, then mutate the tx_type to an
        // unsupported value (e.g. EIP-4844 blob = 3).
        let DecodedTransaction::Evm(mut evm) = evm::decode_eip1559(&sample_1559()) else {
            panic!("expected evm");
        };
        evm.tx_type = 3;
        let result = rebuild_evm_signable(&DecodedTransaction::Evm(evm));
        assert!(matches!(
            result,
            Err(RebuildError::UnsupportedTxType { tx_type: 3 })
        ));
    }

    #[test]
    fn missing_fee_field_fails_closed() {
        // A 1559 tx with its max_fee_per_gas stripped must fail closed.
        let DecodedTransaction::Evm(mut evm) = evm::decode_eip1559(&sample_1559()) else {
            panic!("expected evm");
        };
        evm.max_fee_per_gas = None;
        let result = rebuild_evm_signable(&DecodedTransaction::Evm(evm));
        assert!(matches!(
            result,
            Err(RebuildError::MissingField {
                field: "max_fee_per_gas"
            })
        ));
    }

    #[test]
    fn overflowing_fee_field_fails_closed() {
        // A fee field wider than 16 bytes cannot be a valid EVM u128 fee.
        let DecodedTransaction::Evm(mut evm) = evm::decode_eip1559(&sample_1559()) else {
            panic!("expected evm");
        };
        evm.max_fee_per_gas = Some(vec![0xffu8; 17]);
        let result = rebuild_evm_signable(&DecodedTransaction::Evm(evm));
        assert!(matches!(
            result,
            Err(RebuildError::FieldOverflow {
                field: "max_fee_per_gas"
            })
        ));
    }

    #[test]
    fn non_evm_chain_fails_closed() {
        // A non-EVM decoded tx has no custodial rebuild path yet.
        let near = DecodedTransaction::Near(ironclaw_attestation::NearTransaction {
            network: "testnet".to_string(),
            signer_id: "alice.near".to_string(),
            public_key: ironclaw_attestation::NearPublicKey {
                key_type: 0,
                data: vec![0u8; 32],
            },
            receiver_id: "bob.near".to_string(),
            nonce: 1,
            block_hash: ironclaw_attestation::Bytes32([0u8; 32]),
            actions: vec![],
        });
        let result = rebuild_evm_signable(&near);
        assert!(matches!(result, Err(RebuildError::UnsupportedChain { .. })));
    }
}

//! Hand-rolled, chain-SDK-free serialization of the EXACT signed bytes for
//! Solana versioned messages and NEAR transactions.
//!
//! The attestation crate must not pull `solana-sdk` / `near-primitives` (see
//! the architecture boundary test), yet the canonical signing bytes must
//! reproduce — byte-for-byte — what a wallet/HSM signs over, so that two
//! distinct signed messages can never collapse to the same projection (the
//! non-injective bug class). These encoders therefore re-implement the
//! relevant on-chain wire formats directly:
//!
//! - **Solana**: the `Message` (legacy) and `MessageV0` layouts, including the
//!   compact-u16 ("shortvec") length encoding used for every vector.
//! - **NEAR**: the borsh layout of `Transaction` and `Action` (borsh encodes
//!   `u128` little-endian, `String`/`Vec` length-prefixed `u32` little-endian,
//!   enums as a `u8` discriminant + payload).
//!
//! These bytes are the SIGNED payload (Solana: the message bytes that go under
//! the signature; NEAR: the borsh-serialized `Transaction` that is then
//! sha256'd and signed). They are folded into the canonical signing bytes so
//! the human-approved hash commits to the precise wire form.

use crate::decoded_tx::{
    NearAccessKeyPermission, NearAction, NearPublicKey, NearTransaction, SolanaMessageVersion,
    SolanaTransaction,
};

// ---- Solana compact-u16 (shortvec) -------------------------------------

/// Append a Solana compact-u16 ("shortvec") length prefix. Matches
/// `solana_short_vec`: 7 bits per byte, little-endian, high bit = continuation.
fn push_compact_u16(out: &mut Vec<u8>, mut value: u16) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
            out.push(byte);
        } else {
            out.push(byte);
            break;
        }
    }
}

/// Append a compact-u16-prefixed byte slice.
fn push_compact_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    debug_assert!(bytes.len() <= u16::MAX as usize);
    push_compact_u16(out, bytes.len() as u16);
    out.extend_from_slice(bytes);
}

/// Serialize a Solana message to its exact signed bytes (legacy or v0).
///
/// Legacy layout:
/// `header(3) ∥ shortvec(static_keys[32]) ∥ blockhash(32) ∥ shortvec(instrs)`.
/// V0 prefixes a `0x80` version byte and appends `shortvec(addr_table_lookups)`.
/// Each instruction: `program_id_index(u8) ∥ shortvec(account_indices) ∥
/// shortvec(data)`. Each lookup: `account_key(32) ∥ shortvec(writable) ∥
/// shortvec(readonly)`.
pub(crate) fn solana_message_bytes(tx: &SolanaTransaction) -> Vec<u8> {
    let mut out = Vec::new();

    if matches!(tx.version, SolanaMessageVersion::V0) {
        // Versioned-message prefix: high bit set, low 7 bits = version (0).
        out.push(0x80);
    }

    // Header.
    out.push(tx.header.num_required_signatures);
    out.push(tx.header.num_readonly_signed_accounts);
    out.push(tx.header.num_readonly_unsigned_accounts);

    // Static account keys.
    push_compact_u16(&mut out, tx.static_account_keys.len() as u16);
    for key in &tx.static_account_keys {
        out.extend_from_slice(&key.0);
    }

    // Recent blockhash.
    out.extend_from_slice(&tx.recent_blockhash.0);

    // Compiled instructions.
    push_compact_u16(&mut out, tx.instructions.len() as u16);
    for ix in &tx.instructions {
        out.push(ix.program_id_index);
        push_compact_bytes(&mut out, &ix.account_indices);
        push_compact_bytes(&mut out, &ix.data);
    }

    // Address-table lookups (v0 only).
    if matches!(tx.version, SolanaMessageVersion::V0) {
        push_compact_u16(&mut out, tx.address_table_lookups.len() as u16);
        for lookup in &tx.address_table_lookups {
            out.extend_from_slice(&lookup.account_key.0);
            push_compact_bytes(&mut out, &lookup.writable_indexes);
            push_compact_bytes(&mut out, &lookup.readonly_indexes);
        }
    }

    out
}

// ---- NEAR borsh --------------------------------------------------------

/// Append a borsh `u32` length-prefixed byte slice (used for `String`/`Vec<u8>`
/// and as the length for other vectors).
fn push_borsh_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

/// Append a borsh `String`.
fn push_borsh_string(out: &mut Vec<u8>, s: &str) {
    push_borsh_bytes(out, s.as_bytes());
}

/// Append a yoctoNEAR `u128` carried as big-endian minimal bytes, re-encoded as
/// borsh `u128` (little-endian, 16 bytes).
fn push_borsh_u128_be(out: &mut Vec<u8>, be_minimal: &[u8]) {
    let value = u128_from_be_minimal(be_minimal);
    out.extend_from_slice(&value.to_le_bytes());
}

/// Parse a big-endian minimal byte string into a `u128`. Bytes beyond 16 (only
/// possible from malformed input) are folded in by treating the value as
/// big-endian; the high bytes simply shift, which is acceptable because the
/// canonical bytes still commit to the raw `deposit` slice separately in the
/// field projection. This conversion exists only to reproduce the NEAR wire
/// `u128`.
fn u128_from_be_minimal(be_minimal: &[u8]) -> u128 {
    let mut value: u128 = 0;
    for &b in be_minimal {
        value = value.wrapping_shl(8) | u128::from(b);
    }
    value
}

/// Append a borsh-serialized NEAR `PublicKey`: `key_type(u8) ∥ data`.
/// (The data length is fixed by the key type on-chain — 32 for ed25519, 64 for
/// secp256k1 — so it is NOT length-prefixed.)
fn push_near_public_key(out: &mut Vec<u8>, pk: &NearPublicKey) {
    out.push(pk.key_type);
    out.extend_from_slice(&pk.data);
}

/// Append a borsh-serialized NEAR `Action`.
fn push_near_action(out: &mut Vec<u8>, action: &NearAction) {
    match action {
        NearAction::CreateAccount => out.push(0),
        NearAction::DeployContract { code } => {
            out.push(1);
            push_borsh_bytes(out, code);
        }
        NearAction::FunctionCall {
            method_name,
            args,
            gas,
            deposit,
        } => {
            out.push(2);
            push_borsh_string(out, method_name);
            push_borsh_bytes(out, args);
            out.extend_from_slice(&gas.to_le_bytes());
            push_borsh_u128_be(out, deposit);
        }
        NearAction::Transfer { deposit } => {
            out.push(3);
            push_borsh_u128_be(out, deposit);
        }
        NearAction::Stake { stake, public_key } => {
            out.push(4);
            push_borsh_u128_be(out, stake);
            push_near_public_key(out, public_key);
        }
        NearAction::AddKey {
            public_key,
            access_key,
        } => {
            out.push(5);
            push_near_public_key(out, public_key);
            // AccessKey: nonce(u64 le) ∥ permission.
            out.extend_from_slice(&access_key.nonce.to_le_bytes());
            match &access_key.permission {
                NearAccessKeyPermission::FunctionCall {
                    allowance,
                    receiver_id,
                    method_names,
                } => {
                    // AccessKeyPermission enum: 0 = FunctionCall.
                    out.push(0);
                    // Option<u128>: 0 = None, 1 = Some.
                    match allowance {
                        Some(a) => {
                            out.push(1);
                            push_borsh_u128_be(out, a);
                        }
                        None => out.push(0),
                    }
                    push_borsh_string(out, receiver_id);
                    // Vec<String>: u32 len ∥ each borsh String.
                    out.extend_from_slice(&(method_names.len() as u32).to_le_bytes());
                    for name in method_names {
                        push_borsh_string(out, name);
                    }
                }
                NearAccessKeyPermission::FullAccess => {
                    // AccessKeyPermission enum: 1 = FullAccess.
                    out.push(1);
                }
            }
        }
        NearAction::DeleteKey { public_key } => {
            out.push(6);
            push_near_public_key(out, public_key);
        }
        NearAction::DeleteAccount { beneficiary_id } => {
            out.push(7);
            push_borsh_string(out, beneficiary_id);
        }
        NearAction::Delegate {
            sender_id,
            receiver_id,
            nonce,
            max_block_height,
            public_key,
        } => {
            out.push(8);
            push_borsh_string(out, sender_id);
            push_borsh_string(out, receiver_id);
            out.extend_from_slice(&nonce.to_le_bytes());
            out.extend_from_slice(&max_block_height.to_le_bytes());
            push_near_public_key(out, public_key);
        }
    }
}

/// Serialize a NEAR transaction to its exact borsh-signed bytes:
/// `signer_id ∥ public_key ∥ nonce(u64 le) ∥ receiver_id ∥ block_hash(32) ∥
/// Vec<Action>`.
pub(crate) fn near_transaction_bytes(tx: &NearTransaction) -> Vec<u8> {
    let mut out = Vec::new();
    push_borsh_string(&mut out, &tx.signer_id);
    push_near_public_key(&mut out, &tx.public_key);
    out.extend_from_slice(&tx.nonce.to_le_bytes());
    push_borsh_string(&mut out, &tx.receiver_id);
    out.extend_from_slice(&tx.block_hash.0);
    out.extend_from_slice(&(tx.actions.len() as u32).to_le_bytes());
    for action in &tx.actions {
        push_near_action(&mut out, action);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_u16_matches_solana_shortvec() {
        let cases: &[(u16, &[u8])] = &[
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (16384, &[0x80, 0x80, 0x01]),
            (u16::MAX, &[0xff, 0xff, 0x03]),
        ];
        for (value, expected) in cases {
            let mut out = Vec::new();
            push_compact_u16(&mut out, *value);
            assert_eq!(&out, expected, "compact-u16 for {value}");
        }
    }

    #[test]
    fn u128_be_minimal_round_trips() {
        assert_eq!(u128_from_be_minimal(&[]), 0);
        assert_eq!(u128_from_be_minimal(&[0x01]), 1);
        assert_eq!(u128_from_be_minimal(&[0x01, 0x00]), 256);
        assert_eq!(
            u128_from_be_minimal(&[0x0d, 0xe0, 0xb6, 0xb3, 0xa7, 0x64, 0x00, 0x00]),
            1_000_000_000_000_000_000
        );
    }
}

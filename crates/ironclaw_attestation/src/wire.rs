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
//!
//! ## Fail-closed length handling
//!
//! Every length prefix here is derived from a `usize` slice length. A length
//! that does not fit the on-wire prefix width (compact-u16 for Solana, `u32`
//! for borsh) cannot be serialized faithfully. We therefore **reject** such
//! inputs (returning [`AttestationError`]) rather than casting with silent
//! truncation: a truncated prefix would desync the length from the payload,
//! letting trailing bytes be reinterpreted as extra instructions/accounts and
//! breaking the what-you-see-is-what-you-sign binding.

use crate::decoded_tx::{
    NearAccessKeyPermission, NearAction, NearPublicKey, NearTransaction, SolanaMessageVersion,
    SolanaTransaction,
};
use crate::error::AttestationError;

// ---- Solana compact-u16 (shortvec) -------------------------------------

/// Convert a `usize` length into a Solana compact-u16, rejecting any value that
/// exceeds `u16::MAX`. The Solana wire format encodes every vector length as a
/// compact-u16; a longer vector simply cannot be represented, so we fail closed
/// rather than truncate (which would desync the length from the payload and
/// allow instruction/account smuggling past the WYSIWYS binding).
fn checked_short_vec_len(len: usize, what: &'static str) -> Result<u16, AttestationError> {
    u16::try_from(len).map_err(|_| AttestationError::SolanaShortVecOverflow {
        what,
        len,
        max: u16::MAX as usize,
    })
}

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

/// Append a compact-u16-prefixed byte slice, rejecting slices longer than
/// `u16::MAX` instead of silently truncating the length prefix.
fn push_compact_bytes(
    out: &mut Vec<u8>,
    bytes: &[u8],
    what: &'static str,
) -> Result<(), AttestationError> {
    push_compact_u16(out, checked_short_vec_len(bytes.len(), what)?);
    out.extend_from_slice(bytes);
    Ok(())
}

/// Serialize a Solana message to its exact signed bytes (legacy or v0).
///
/// Legacy layout:
/// `header(3) ∥ shortvec(static_keys[32]) ∥ blockhash(32) ∥ shortvec(instrs)`.
/// V0 prefixes a `0x80` version byte and appends `shortvec(addr_table_lookups)`.
/// Each instruction: `program_id_index(u8) ∥ shortvec(account_indices) ∥
/// shortvec(data)`. Each lookup: `account_key(32) ∥ shortvec(writable) ∥
/// shortvec(readonly)`.
pub(crate) fn solana_message_bytes(tx: &SolanaTransaction) -> Result<Vec<u8>, AttestationError> {
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
    push_compact_u16(
        &mut out,
        checked_short_vec_len(tx.static_account_keys.len(), "static_account_keys")?,
    );
    for key in &tx.static_account_keys {
        out.extend_from_slice(&key.0);
    }

    // Recent blockhash.
    out.extend_from_slice(&tx.recent_blockhash.0);

    // Compiled instructions.
    push_compact_u16(
        &mut out,
        checked_short_vec_len(tx.instructions.len(), "instructions")?,
    );
    for ix in &tx.instructions {
        out.push(ix.program_id_index);
        push_compact_bytes(&mut out, &ix.account_indices, "instruction.account_indices")?;
        push_compact_bytes(&mut out, &ix.data, "instruction.data")?;
    }

    // Address-table lookups (v0 only).
    if matches!(tx.version, SolanaMessageVersion::V0) {
        push_compact_u16(
            &mut out,
            checked_short_vec_len(tx.address_table_lookups.len(), "address_table_lookups")?,
        );
        for lookup in &tx.address_table_lookups {
            out.extend_from_slice(&lookup.account_key.0);
            push_compact_bytes(
                &mut out,
                &lookup.writable_indexes,
                "lookup.writable_indexes",
            )?;
            push_compact_bytes(
                &mut out,
                &lookup.readonly_indexes,
                "lookup.readonly_indexes",
            )?;
        }
    }

    Ok(out)
}

// ---- NEAR borsh --------------------------------------------------------

/// Borsh `u32` length prefix for a slice, rejecting lengths beyond `u32::MAX`.
/// Borsh encodes `String`/`Vec` lengths as `u32` little-endian; a longer slice
/// cannot be represented, so we fail closed rather than truncate the prefix.
///
/// Target-width note: the overflow branch (`NearBorshLengthOverflow`) is
/// reachable only on targets where `usize > u32` — i.e. 64-bit. On 32-bit
/// targets `usize == u32`, so `u32::try_from` is infallible and the branch (and
/// the `borsh_bytes_rejects_overlong_length` test exercising it) cannot fire;
/// that is expected, not dead code.
fn borsh_len_le(len: usize, what: &'static str) -> Result<[u8; 4], AttestationError> {
    u32::try_from(len).map(u32::to_le_bytes).map_err(|_| {
        AttestationError::NearBorshLengthOverflow {
            what,
            len,
            max: u32::MAX as usize,
        }
    })
}

/// Append a borsh `u32` length-prefixed byte slice (used for `String`/`Vec<u8>`
/// and as the length for other vectors).
fn push_borsh_bytes(
    out: &mut Vec<u8>,
    bytes: &[u8],
    what: &'static str,
) -> Result<(), AttestationError> {
    out.extend_from_slice(&borsh_len_le(bytes.len(), what)?);
    out.extend_from_slice(bytes);
    Ok(())
}

/// Append a borsh `String`.
fn push_borsh_string(out: &mut Vec<u8>, s: &str) -> Result<(), AttestationError> {
    push_borsh_bytes(out, s.as_bytes(), "string")
}

/// Append a yoctoNEAR `u128` carried as big-endian minimal bytes, re-encoded as
/// borsh `u128` (little-endian, 16 bytes). Rejects inputs longer than 16 bytes:
/// such a value does not fit a `u128`, so the human-rendered amount would
/// diverge from the (wrapped) signed amount — an approve-vs-sign mismatch.
/// `what` names the offending amount (`deposit` / `stake` / `allowance`) so a
/// rejection can identify which field overflowed.
fn push_borsh_u128_be(
    out: &mut Vec<u8>,
    be_minimal: &[u8],
    what: &'static str,
) -> Result<(), AttestationError> {
    let value = u128_from_be_minimal(be_minimal, what)?;
    out.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Parse a big-endian minimal byte string into a `u128`, rejecting inputs that
/// exceed 16 bytes (which cannot fit a `u128`). Leading-zero padding within 16
/// bytes is tolerated; only a genuine length overflow is rejected, so the
/// canonical wire `u128` always equals the value the human approved.
fn u128_from_be_minimal(be_minimal: &[u8], what: &'static str) -> Result<u128, AttestationError> {
    if be_minimal.len() > 16 {
        return Err(AttestationError::NearU128Overflow {
            what,
            len: be_minimal.len(),
        });
    }
    let mut value: u128 = 0;
    for &b in be_minimal {
        value = (value << 8) | u128::from(b);
    }
    Ok(value)
}

/// Append a borsh-serialized NEAR `PublicKey`: `key_type(u8) ∥ data`.
/// (The data length is fixed by the key type on-chain — 32 for ed25519, 64 for
/// secp256k1 — so it is NOT length-prefixed.)
fn push_near_public_key(out: &mut Vec<u8>, pk: &NearPublicKey) {
    out.push(pk.key_type);
    out.extend_from_slice(&pk.data);
}

/// Append a borsh-serialized NEAR `Action`.
fn push_near_action(out: &mut Vec<u8>, action: &NearAction) -> Result<(), AttestationError> {
    match action {
        NearAction::CreateAccount => out.push(0),
        NearAction::DeployContract { code } => {
            out.push(1);
            push_borsh_bytes(out, code, "deploy_contract.code")?;
        }
        NearAction::FunctionCall {
            method_name,
            args,
            gas,
            deposit,
        } => {
            out.push(2);
            push_borsh_string(out, method_name)?;
            push_borsh_bytes(out, args, "function_call.args")?;
            out.extend_from_slice(&gas.to_le_bytes());
            push_borsh_u128_be(out, deposit, "function_call.deposit")?;
        }
        NearAction::Transfer { deposit } => {
            out.push(3);
            push_borsh_u128_be(out, deposit, "transfer.deposit")?;
        }
        NearAction::Stake { stake, public_key } => {
            out.push(4);
            push_borsh_u128_be(out, stake, "stake.stake")?;
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
                            push_borsh_u128_be(out, a, "add_key.allowance")?;
                        }
                        None => out.push(0),
                    }
                    push_borsh_string(out, receiver_id)?;
                    // Vec<String>: u32 len ∥ each borsh String.
                    out.extend_from_slice(&borsh_len_le(method_names.len(), "method_names")?);
                    for name in method_names {
                        push_borsh_string(out, name)?;
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
            push_borsh_string(out, beneficiary_id)?;
        }
        NearAction::Delegate {
            sender_id,
            receiver_id,
            actions,
            nonce,
            max_block_height,
            public_key,
        } => {
            out.push(8);
            // NEP-366 DelegateAction borsh layout:
            // `sender_id ∥ receiver_id ∥ Vec<NonDelegateAction> ∥ nonce(u64 le)
            //  ∥ max_block_height(u64 le) ∥ public_key`. The inner actions are
            // carried at full fidelity so two delegates differing only in their
            // inner actions produce distinct bytes (injectivity). Inner actions
            // are `NonDelegateAction`: a nested `Delegate` is unrepresentable
            // on-chain, so it is rejected (fail closed) rather than serialized.
            push_borsh_string(out, sender_id)?;
            push_borsh_string(out, receiver_id)?;
            out.extend_from_slice(&borsh_len_le(actions.len(), "delegate.actions")?);
            for inner in actions {
                if matches!(inner, NearAction::Delegate { .. }) {
                    return Err(AttestationError::NearNestedDelegate);
                }
                push_near_action(out, inner)?;
            }
            out.extend_from_slice(&nonce.to_le_bytes());
            out.extend_from_slice(&max_block_height.to_le_bytes());
            push_near_public_key(out, public_key);
        }
    }
    Ok(())
}

/// Serialize a NEAR transaction to its exact borsh-signed bytes:
/// `signer_id ∥ public_key ∥ nonce(u64 le) ∥ receiver_id ∥ block_hash(32) ∥
/// Vec<Action>`.
pub(crate) fn near_transaction_bytes(tx: &NearTransaction) -> Result<Vec<u8>, AttestationError> {
    let mut out = Vec::new();
    push_borsh_string(&mut out, &tx.signer_id)?;
    push_near_public_key(&mut out, &tx.public_key);
    out.extend_from_slice(&tx.nonce.to_le_bytes());
    push_borsh_string(&mut out, &tx.receiver_id)?;
    out.extend_from_slice(&tx.block_hash.0);
    out.extend_from_slice(&borsh_len_le(tx.actions.len(), "actions")?);
    for action in &tx.actions {
        push_near_action(&mut out, action)?;
    }
    Ok(out)
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
        assert_eq!(u128_from_be_minimal(&[], "deposit"), Ok(0));
        assert_eq!(u128_from_be_minimal(&[0x01], "deposit"), Ok(1));
        assert_eq!(u128_from_be_minimal(&[0x01, 0x00], "deposit"), Ok(256));
        assert_eq!(
            u128_from_be_minimal(&[0x0d, 0xe0, 0xb6, 0xb3, 0xa7, 0x64, 0x00, 0x00], "deposit"),
            Ok(1_000_000_000_000_000_000)
        );
        // Exactly 16 bytes (u128::MAX) is the boundary and must be accepted.
        assert_eq!(u128_from_be_minimal(&[0xff; 16], "deposit"), Ok(u128::MAX));
    }

    #[test]
    fn u128_be_minimal_oversized_bytes() {
        // 17 bytes cannot fit a u128. Rather than silently wrapping/discarding
        // the high bytes (which would make the signed value diverge from the
        // human-approved one), the parse must fail closed.
        let err =
            u128_from_be_minimal(&[0x01; 17], "stake").expect_err("17 bytes must be rejected");
        assert_eq!(
            err,
            AttestationError::NearU128Overflow {
                what: "stake",
                len: 17
            }
        );
        // A leading-zero-padded value within 16 bytes is still fine.
        assert_eq!(u128_from_be_minimal(&[0x00, 0x00, 0x01], "stake"), Ok(1));
    }

    #[test]
    fn compact_bytes_rejects_overlong_slice() {
        // A byte slice longer than u16::MAX cannot be length-prefixed by a
        // compact-u16. Truncating the prefix would let the trailing bytes be
        // reinterpreted as separate instructions/accounts (field smuggling), so
        // the encoder must fail closed.
        let oversized = vec![0u8; u16::MAX as usize + 1];
        let mut out = Vec::new();
        let err = push_compact_bytes(&mut out, &oversized, "instruction.data")
            .expect_err("oversized compact slice must be rejected");
        assert_eq!(
            err,
            AttestationError::SolanaShortVecOverflow {
                what: "instruction.data",
                len: u16::MAX as usize + 1,
                max: u16::MAX as usize,
            }
        );
        // Exactly u16::MAX is the boundary and must encode.
        let mut ok = Vec::new();
        assert!(push_compact_bytes(&mut ok, &vec![0u8; u16::MAX as usize], "d").is_ok());
    }

    #[test]
    fn borsh_bytes_rejects_overlong_length() {
        // borsh_len_le is the choke point for every borsh u32 length prefix.
        // usize values beyond u32::MAX must be rejected, not truncated.
        assert!(borsh_len_le(u32::MAX as usize, "bytes").is_ok());
        let err = borsh_len_le(u32::MAX as usize + 1, "bytes")
            .expect_err("u32 overflow must be rejected");
        assert_eq!(
            err,
            AttestationError::NearBorshLengthOverflow {
                what: "bytes",
                len: u32::MAX as usize + 1,
                max: u32::MAX as usize,
            }
        );
    }
}

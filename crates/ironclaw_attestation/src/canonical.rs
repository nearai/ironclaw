//! Deterministic, domain-separated, length-prefixed canonical signing-bytes
//! encoding.
//!
//! ## Encoding (hand-rolled; no CBOR dependency)
//!
//! Per the conservative-deps rule, this is a small hand-rolled length-prefixed
//! encoder rather than a CBOR dependency (the plan's "domain-separated CBOR"
//! wording predates the deps review; the binding property only needs an
//! injective, domain-separated, deterministic encoding, which length-prefixing
//! gives us in far less than 30 lines):
//!
//! ```text
//! DOMAIN_TAG
//! lp(chain_tag)
//! lp(chain_network)
//! lp(tx_type_label)
//! lp(schema_version big-endian u16)
//! u64_be(field_count)
//! for each field, in canonical order:
//!     lp(field.tag)
//!     lp(field.canonical_bytes)
//! ```
//!
//! where `lp(x)` = `u64_be(x.len()) ∥ x` (lengths in bytes). Length-prefixing every component makes
//! the encoding injective: no concatenation of two distinct field sets can
//! collide, so a hidden / reordered / extra field always changes the bytes.

use crate::decoded_tx::{DecodedTransaction, RenderingSchemaVersion};
use crate::fields::project;

/// Domain separator for the canonical signing-bytes encoding. Distinct from the
/// [`crate::approved_tx_hash`] domain so the two pre-images can never be
/// confused.
const CANONICAL_DOMAIN: &[u8] = b"ironclaw.attestation.canonical.v1";

/// Append `len(bytes) ∥ bytes` to `out`.
fn push_lp(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Compute the deterministic, domain-separated canonical signing bytes for a
/// decoded transaction.
///
/// Determinism: derived solely from the [`crate::fields::project`] field list
/// plus the chain/network/type/schema headers, all in fixed order — identical
/// input yields identical output across calls and serde round-trips.
pub fn canonical_signing_bytes(
    tx: &DecodedTransaction,
    schema_version: RenderingSchemaVersion,
) -> Vec<u8> {
    let fields = project(tx);
    let mut out = Vec::new();
    out.extend_from_slice(CANONICAL_DOMAIN);
    push_lp(&mut out, tx.chain_tag().as_bytes());
    push_lp(&mut out, tx.chain_network().as_bytes());
    push_lp(&mut out, tx.tx_type_label().as_bytes());
    push_lp(&mut out, &schema_version.get().to_be_bytes());
    out.extend_from_slice(&(fields.len() as u64).to_be_bytes());
    for field in &fields {
        push_lp(&mut out, field.tag.as_bytes());
        push_lp(&mut out, &field.canonical_bytes);
    }
    out
}

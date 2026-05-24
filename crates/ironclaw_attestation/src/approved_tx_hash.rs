//! Computation of the binding [`ApprovedTxHash`].
//!
//! The hash is a domain-separated SHA-256 over
//! `render ∥ canonical signing bytes ∥ signer/account ∥ chain/network ∥
//! tx-type ∥ rendering-schema-version`, each component length-prefixed. Binding
//! all six components is the anti-field-smuggling guarantee: changing any one
//! changes the hash, so an approval of one view can never authorize signing of
//! different bytes, a different account, chain, type, or schema.

use ironclaw_signing_provider::ApprovedTxHash;
use sha2::{Digest, Sha256};

use crate::decoded_tx::RenderingSchemaVersion;
use crate::rendered::RenderedTx;

/// Domain separator for the approved-tx hash pre-image. Distinct from the
/// canonical-bytes domain so the two digests can never share a pre-image.
const APPROVED_TX_DOMAIN: &[u8] = b"ironclaw.attestation.approved_tx_hash.v1";

/// Append `len(bytes) ∥ bytes` to the hasher.
fn update_lp(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u32).to_be_bytes());
    hasher.update(bytes);
}

/// Length-prefix the rendered view into a deterministic pre-image.
///
/// Walking the struct fields explicitly (rather than via a serializer) keeps
/// the encoding injective and dependency-free: every label/value pair is
/// length-prefixed so no two distinct renders can produce the same bytes.
fn render_bytes(rendered: &RenderedTx) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&rendered.schema_version.get().to_be_bytes());
    push_lp(&mut out, rendered.chain.as_bytes());
    push_lp(&mut out, rendered.chain_network.as_bytes());
    push_lp(&mut out, rendered.tx_type.as_bytes());
    out.extend_from_slice(&(rendered.fields.len() as u32).to_be_bytes());
    for field in &rendered.fields {
        push_lp(&mut out, field.label.as_bytes());
        push_lp(&mut out, field.value.as_bytes());
    }
    out
}

/// Append `len(bytes) ∥ bytes` to `out`.
fn push_lp(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Compute the binding [`ApprovedTxHash`].
///
/// All six components are length-prefixed and folded under a single domain tag.
#[allow(clippy::too_many_arguments)]
pub fn compute_approved_tx_hash(
    rendered: &RenderedTx,
    canonical_bytes: &[u8],
    signer_account: &str,
    chain_network: &str,
    tx_type: &str,
    schema_version: RenderingSchemaVersion,
) -> ApprovedTxHash {
    let mut hasher = Sha256::new();
    hasher.update(APPROVED_TX_DOMAIN);
    update_lp(&mut hasher, &render_bytes(rendered));
    update_lp(&mut hasher, canonical_bytes);
    update_lp(&mut hasher, signer_account.as_bytes());
    update_lp(&mut hasher, chain_network.as_bytes());
    update_lp(&mut hasher, tx_type.as_bytes());
    update_lp(&mut hasher, &schema_version.get().to_be_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    ApprovedTxHash::from_bytes(digest)
}

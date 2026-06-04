//! Computation of the binding [`ApprovedTxHash`].
//!
//! The hash is a domain-separated SHA-256 over
//! `render ∥ canonical signing bytes ∥ signer/account ∥ chain/network ∥
//! tx-type ∥ rendering-schema-version`, each component length-prefixed. Binding
//! all six components is the anti-field-smuggling guarantee: changing any one
//! changes the hash, so an approval of one view can never authorize signing of
//! different bytes, a different account, chain, type, or schema.
//!
//! ## Safe vs low-level API
//!
//! Callers should use [`approved_tx_hash_for`], which derives BOTH the render
//! and the canonical signing bytes from the *same* decoded transaction and the
//! transaction's own chain/network/type — so the render input and the canonical
//! input can never be mismatched by a caller. The low-level
//! [`compute_approved_tx_hash`] (which takes already-derived components) is kept
//! crate-private/test-only precisely so production callers cannot accidentally
//! feed it a render of tx A with the canonical bytes of tx B.
//!
//! The signer/account is the ONE component that is NOT derived from the decoded
//! transaction: it is the explicit, trusted account from the signing context
//! (`SigningContext.key_or_account_id`), bound here so changing the bound signer
//! changes the hash even when `to` / message contents stay fixed (threats #4/#5
//! — the approved hash commits to *who* signs, not a heuristic recovered from
//! the tx body).

use ironclaw_signing_provider::ApprovedTxHash;
use sha2::{Digest, Sha256};

use crate::canonical::canonical_signing_bytes;
use crate::decoded_tx::{DecodedTransaction, RenderingSchemaVersion};
use crate::error::AttestationError;
use crate::rendered::{RenderedTx, render};

/// Domain separator for the approved-tx hash pre-image. Distinct from the
/// canonical-bytes domain so the two digests can never share a pre-image.
const APPROVED_TX_DOMAIN: &[u8] = b"ironclaw.attestation.approved_tx_hash.v1";

/// Append `len(bytes) ∥ bytes` to the hasher.
fn update_lp(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
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
    out.extend_from_slice(&(rendered.fields.len() as u64).to_be_bytes());
    for field in &rendered.fields {
        push_lp(&mut out, field.label.as_bytes());
        push_lp(&mut out, field.value.as_bytes());
    }
    out
}

/// Append `len(bytes) ∥ bytes` to `out`.
fn push_lp(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Compute the binding [`ApprovedTxHash`] for a decoded transaction and an
/// explicit, trusted signer/account.
///
/// This is the **safe public API**: the render and canonical signing bytes are
/// both derived here from the SAME `tx`, and chain/network/tx-type are read off
/// `tx`, so a caller cannot mismatch a render of one transaction with the
/// canonical bytes of another. `signer_account` is the explicit account the
/// signing context binds (`SigningContext.key_or_account_id`) — it is NOT
/// derived from the transaction body.
pub fn approved_tx_hash_for(
    tx: &DecodedTransaction,
    signer_account: &str,
    schema_version: RenderingSchemaVersion,
) -> Result<ApprovedTxHash, AttestationError> {
    let rendered = render(tx, schema_version)?;
    let canonical = canonical_signing_bytes(tx, schema_version)?;
    Ok(compute_approved_tx_hash_inner(
        &rendered,
        &canonical,
        signer_account,
        &tx.chain_network(),
        &tx.tx_type_label(),
        schema_version,
    ))
}

/// Compute the binding [`ApprovedTxHash`] from already-derived components.
///
/// All six components are length-prefixed and folded under a single domain tag.
/// **Crate-private**: production code must call [`approved_tx_hash_for`] so the
/// render and canonical inputs are guaranteed to describe the same transaction.
#[allow(clippy::too_many_arguments)]
fn compute_approved_tx_hash_inner(
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

/// Test-only escape hatch exposing the low-level component hasher so the
/// binding suite can drive per-component tampering ("same render, different
/// canonical bytes ⇒ different hash"). Gated behind the internal
/// `test-internals` feature; never enabled by production dependents.
#[cfg(feature = "test-internals")]
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn compute_approved_tx_hash(
    rendered: &RenderedTx,
    canonical_bytes: &[u8],
    signer_account: &str,
    chain_network: &str,
    tx_type: &str,
    schema_version: RenderingSchemaVersion,
) -> ApprovedTxHash {
    compute_approved_tx_hash_inner(
        rendered,
        canonical_bytes,
        signer_account,
        chain_network,
        tx_type,
        schema_version,
    )
}

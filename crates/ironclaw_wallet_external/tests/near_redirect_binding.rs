//! Edge-case coverage for the canonical NEAR signer-binding format
//! (`account_id:<64-char lowercase-hex 32-byte ed25519 pubkey>`) parsed from
//! [`SigningContext::key_or_account_id`] in `verify_resume`.
//!
//! `BoundNearIdentity::parse` is a crate-private helper, so these drive it
//! through the public `verify_resume` surface: a malformed bound identity
//! string must fail closed as `ProofInvalid` before any signature is trusted.
//! These supplement the byte-identical `tests/near_redirect.rs` (which mirrors
//! `attested-signing-10-reborn-runtime`) with the parse failure modes, and are
//! kept in a separate file so the shared test file stays identical across the
//! stack for clean rebase-cascade dedup.

use std::sync::Arc;

use ironclaw_attestation::{
    ApprovedTxHash, AttestedSigningGrant, GrantKey, InMemorySealedGrantStore, SealedGrantStore,
};
use ironclaw_signing_provider::{
    ActorId, ChainId, GateRef, KeyOrAccountId, RunId, ScopeId, SigningContext, SigningProof,
    SigningProvider, SigningProviderError, TenantId, UserId,
};
use ironclaw_wallet_external::{
    NearAccessKeyScope, NearRedirectProofPayload, NearRedirectSigningProvider, derive_state,
    encode_near_redirect_proof,
};

use ed25519_dalek::{Signer as _, SigningKey as EdSigningKey};

const WALLET_URL: &str = "https://wallet.near.org/sign";
const CALLBACK_URL: &str = "https://ironclaw.example/api/chat/gate/resolve";
const STATE_SECRET: &[u8] = b"server-side-state-secret";

fn near_key() -> EdSigningKey {
    EdSigningKey::from_bytes(&[0x55u8; 32])
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

/// Build a context whose `key_or_account_id` is set to an arbitrary (possibly
/// malformed) bound-identity string, so we can exercise `BoundNearIdentity::parse`.
fn ctx_with_bound(bound: &str) -> SigningContext {
    SigningContext {
        tenant: TenantId::new("tenant-a"),
        user: UserId::new("user-1"),
        scope: ScopeId::new("scope-x"),
        actor: ActorId::new("actor-7"),
        run_id: RunId::new("run-42"),
        gate_ref: GateRef::new("gate:near-1"),
        chain_id: ChainId::new("near:mainnet"),
        key_or_account_id: KeyOrAccountId::new(bound),
    }
}

async fn seal_grant(store: &InMemorySealedGrantStore, ctx: &SigningContext, hash: ApprovedTxHash) {
    let key = GrantKey::from_context(ctx, hash);
    store
        .seal(AttestedSigningGrant::seal(key, 1_000, None))
        .await
        .expect("seal");
}

/// Build an otherwise-valid proof (good hash, good state, real signature) so the
/// ONLY thing that can fail is parsing the bound identity from the context.
fn valid_proof(
    key: &EdSigningKey,
    account: &str,
    ctx: &SigningContext,
    hash: ApprovedTxHash,
) -> SigningProof {
    let sig = key.sign(hash.as_bytes());
    let payload = NearRedirectProofPayload {
        approved_tx_hash: hash,
        account_id: account.to_string(),
        public_key: key.verifying_key().to_bytes().to_vec(),
        signature: sig.to_bytes().to_vec(),
        access_key_scope: NearAccessKeyScope::FullAccess,
        state: derive_state(STATE_SECRET, ctx, &hash),
    };
    SigningProof::NearRedirectProof(encode_near_redirect_proof(&payload).expect("encode"))
}

/// Drive `verify_resume` with a malformed bound identity and assert it fails
/// closed as `ProofInvalid` (the `parse` error class).
async fn assert_bound_identity_rejected(bound: &str) {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider =
        NearRedirectSigningProvider::new(WALLET_URL, CALLBACK_URL, STATE_SECRET, store.clone());
    let key = near_key();
    let ctx = ctx_with_bound(bound);
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    let proof = valid_proof(&key, "alice.near", &ctx, hash);
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("malformed bound identity must fail closed");
    assert!(
        matches!(err, SigningProviderError::ProofInvalid { .. }),
        "expected ProofInvalid for bound `{bound}`, got {err:?}"
    );
}

#[tokio::test]
async fn bound_identity_without_colon_is_rejected() {
    // No `:` separator at all — cannot split account from key.
    assert_bound_identity_rejected("alice.near").await;
}

#[tokio::test]
async fn bound_identity_with_empty_account_is_rejected() {
    let key = near_key();
    let bound = format!(":{}", lower_hex(&key.verifying_key().to_bytes()));
    assert_bound_identity_rejected(&bound).await;
}

#[tokio::test]
async fn bound_identity_with_non_hex_pubkey_is_rejected() {
    // 64 chars but not valid hex (`z` is not a hex digit).
    let bound = format!("alice.near:{}", "z".repeat(64));
    assert_bound_identity_rejected(&bound).await;
}

#[tokio::test]
async fn bound_identity_with_wrong_length_pubkey_is_rejected() {
    // Valid hex, but only 16 bytes (32 hex chars) instead of the required 32.
    let bound = format!("alice.near:{}", "ab".repeat(16));
    assert_bound_identity_rejected(&bound).await;
}

#[tokio::test]
async fn bound_identity_account_with_colon_keeps_key_as_final_field() {
    // `rsplit_once(':')` splits on the LAST colon, so an account id that itself
    // contains a colon keeps the trailing 64-hex field as the key. A correctly
    // formed identity (real key bytes) with such an account verifies cleanly.
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider =
        NearRedirectSigningProvider::new(WALLET_URL, CALLBACK_URL, STATE_SECRET, store.clone());
    let key = near_key();
    let account = "weird:account.near";
    let bound = format!("{account}:{}", lower_hex(&key.verifying_key().to_bytes()));
    let ctx = ctx_with_bound(&bound);
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    // The proof's account_id must echo the full account (everything before the
    // final colon), and the key must be the bound key.
    let proof = valid_proof(&key, account, &ctx, hash);
    provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect("colon-containing account with a valid final key field verifies");
}

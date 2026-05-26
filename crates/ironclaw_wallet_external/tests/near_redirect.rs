//! End-to-end verification tests for the NEAR redirect
//! [`NearRedirectSigningProvider`] (attested-signing PR8).
//!
//! These drive the provider behind `Arc<dyn SigningProvider>` (object-safety)
//! and through the sealed-grant store, exercising both halves of the contract:
//!
//! * `initiate` builds an `AwaitingUserAction` redirect directive embedding the
//!   base64 transaction, the callback URL, and the gate-bound `state`.
//! * `verify_resume` is fail-closed: a valid signature from the bound account
//!   with a matching state + covering scope succeeds; a wrong account is
//!   `SignerMismatch`; a tampered hash, a bad signature, or a mismatched state
//!   is `ProofInvalid`; a function-call key with an empty receiver is a
//!   `ScopeViolation`; a replayed (already-claimed) grant fails closed.

use std::sync::Arc;

use ironclaw_attestation::{
    ApprovedTxHash, AttestedSigningGrant, GrantKey, InMemorySealedGrantStore, SealedGrantStore,
};
use ironclaw_signing_provider::{
    ActorId, ChainId, DecodedTransaction, GateRef, InitiationOutcome, KeyOrAccountId, RenderedTx,
    RunId, ScopeId, SigningContext, SigningProof, SigningProvider, SigningProviderError, TenantId,
    UserId,
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
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    out
}

/// The gate-bound NEAR identity is `account_id:<64-hex ed25519 pubkey>`. The
/// access-key public key is bound at gate-raise (from Wallet Selector), exactly
/// as the Solana injected provider binds the signer pubkey, so a callback that
/// carries an attacker's own key cannot pass verification (threat #4).
fn bound_identity(account: &str, key: &EdSigningKey) -> String {
    format!("{account}:{}", lower_hex(&key.verifying_key().to_bytes()))
}

fn ctx_for(account: &str) -> SigningContext {
    SigningContext {
        tenant: TenantId::new("tenant-a"),
        user: UserId::new("user-1"),
        scope: ScopeId::new("scope-x"),
        actor: ActorId::new("actor-7"),
        run_id: RunId::new("run-42"),
        gate_ref: GateRef::new("gate:near-1"),
        chain_id: ChainId::new("near:mainnet"),
        key_or_account_id: KeyOrAccountId::new(account),
    }
}

fn provider(store: Arc<InMemorySealedGrantStore>) -> NearRedirectSigningProvider {
    NearRedirectSigningProvider::new(WALLET_URL, CALLBACK_URL, STATE_SECRET, store)
}

async fn seal_grant(store: &InMemorySealedGrantStore, ctx: &SigningContext, hash: ApprovedTxHash) {
    let key = GrantKey::from_context(ctx, hash);
    store
        .seal(AttestedSigningGrant::seal(key, 1_000, None))
        .await
        .expect("seal");
}

/// Build a valid proof: the key signs the bound hash, the state is gate-derived.
fn valid_proof(
    key: &EdSigningKey,
    account: &str,
    ctx: &SigningContext,
    hash: ApprovedTxHash,
    scope: NearAccessKeyScope,
) -> SigningProof {
    let sig = key.sign(hash.as_bytes());
    let payload = NearRedirectProofPayload {
        approved_tx_hash: hash,
        account_id: account.to_string(),
        public_key: key.verifying_key().to_bytes().to_vec(),
        signature: sig.to_bytes().to_vec(),
        access_key_scope: scope,
        state: derive_state(STATE_SECRET, ctx, &hash),
    };
    SigningProof::NearRedirectProof(encode_near_redirect_proof(&payload).expect("encode"))
}

#[tokio::test]
async fn initiate_returns_redirect_directive_with_state_and_callback() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = provider(store.clone());
    let ctx = ctx_for("alice.near");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let decoded = DecodedTransaction::from_opaque(vec![0xab, 0xcd, 0xef]);
    let rendered = RenderedTx::from_opaque(vec![1]);

    let outcome = provider
        .initiate(&ctx, &decoded, &rendered, &hash)
        .await
        .expect("initiate");
    let InitiationOutcome::AwaitingUserAction { directive } = outcome else {
        panic!("near redirect must require a user redirect");
    };
    let url = String::from_utf8(directive).expect("utf8 url");
    assert!(url.starts_with(WALLET_URL), "url: {url}");
    assert!(url.contains("transactions="), "url: {url}");
    assert!(url.contains("callbackUrl="), "url: {url}");
    // The gate-bound state must be embedded so the callback can be matched.
    let state = derive_state(STATE_SECRET, &ctx, &hash);
    assert!(url.contains(&format!("state={state}")), "url: {url}");
}

#[tokio::test]
async fn valid_signature_from_bound_account_verifies() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let p: Arc<dyn SigningProvider> = Arc::new(provider(store.clone()));
    let key = near_key();
    let account = "alice.near";
    let ctx = ctx_for(&bound_identity(account, &key));
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    let proof = valid_proof(&key, account, &ctx, hash, NearAccessKeyScope::FullAccess);
    let verified = p
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect("valid near proof must verify");
    assert_eq!(verified.proof(), &proof);
}

/// Threat #4 (signer/key binding): an attacker who knows the bound account but
/// NOT its bound access key supplies their own keypair in the callback. The
/// claimed `account_id` matches the bound account, and they can sign over the
/// bound hash with their own key, but the signature must be verified against the
/// GATE-BOUND access key — not the callback-supplied one — so it fails closed.
#[tokio::test]
async fn attacker_supplied_key_is_signer_mismatch() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let p: Arc<dyn SigningProvider> = Arc::new(provider(store.clone()));
    let legit_key = near_key();
    let attacker_key = EdSigningKey::from_bytes(&[0x99u8; 32]);
    let account = "alice.near";
    // The gate binds alice.near's LEGITIMATE access key.
    let ctx = ctx_for(&bound_identity(account, &legit_key));
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    // Attacker forges a proof: correct account, correct hash, valid state (state
    // only binds the account), a self-signed signature, and THEIR OWN pubkey.
    let sig = attacker_key.sign(hash.as_bytes());
    let payload = NearRedirectProofPayload {
        approved_tx_hash: hash,
        account_id: account.to_string(),
        public_key: attacker_key.verifying_key().to_bytes().to_vec(),
        signature: sig.to_bytes().to_vec(),
        access_key_scope: NearAccessKeyScope::FullAccess,
        state: derive_state(STATE_SECRET, &ctx, &hash),
    };
    let proof =
        SigningProof::NearRedirectProof(encode_near_redirect_proof(&payload).expect("encode"));
    let err = p
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("attacker-supplied key must fail closed");
    assert!(matches!(err, SigningProviderError::SignerMismatch));
}

#[tokio::test]
async fn wrong_account_is_signer_mismatch() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let p = provider(store.clone());
    let key = near_key();
    // Bind a different account than the proof claims.
    let ctx = ctx_for(&bound_identity("bob.near", &key));
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    // The proof claims `alice.near` (state derived for the bound bob.near ctx
    // would mismatch, so derive the proof for its own claimed account but bind
    // bob.near) — account binding check fires first via the bound ctx account.
    let proof = valid_proof(
        &key,
        "alice.near",
        &ctx,
        hash,
        NearAccessKeyScope::FullAccess,
    );
    let err = p
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("mismatched account must reject");
    assert!(matches!(err, SigningProviderError::SignerMismatch));
}

#[tokio::test]
async fn tampered_hash_is_proof_invalid() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let p = provider(store.clone());
    let key = near_key();
    let account = "alice.near";
    let ctx = ctx_for(&bound_identity(account, &key));
    let bound_hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, bound_hash).await;

    // Wallet attests to a DIFFERENT hash than the gate bound.
    let attested = ApprovedTxHash::from_bytes([9u8; 32]);
    let proof = valid_proof(
        &key,
        account,
        &ctx,
        attested,
        NearAccessKeyScope::FullAccess,
    );
    let err = p
        .verify_resume(&ctx, &bound_hash, &proof)
        .await
        .expect_err("tampered hash must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn mismatched_state_is_proof_invalid() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let p = provider(store.clone());
    let key = near_key();
    let account = "alice.near";
    let ctx = ctx_for(&bound_identity(account, &key));
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    let sig = key.sign(hash.as_bytes());
    let payload = NearRedirectProofPayload {
        approved_tx_hash: hash,
        account_id: account.to_string(),
        public_key: key.verifying_key().to_bytes().to_vec(),
        signature: sig.to_bytes().to_vec(),
        access_key_scope: NearAccessKeyScope::FullAccess,
        // Forged / intercepted state that was not derived for this gate.
        state: "deadbeef".to_string(),
    };
    let proof =
        SigningProof::NearRedirectProof(encode_near_redirect_proof(&payload).expect("encode"));
    let err = p
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("bad state must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn bad_signature_is_proof_invalid() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let p = provider(store.clone());
    let key = near_key();
    let account = "alice.near";
    let ctx = ctx_for(&bound_identity(account, &key));
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    // Signature over a different message.
    let sig = key.sign(&[0u8; 32]);
    let payload = NearRedirectProofPayload {
        approved_tx_hash: hash,
        account_id: account.to_string(),
        public_key: key.verifying_key().to_bytes().to_vec(),
        signature: sig.to_bytes().to_vec(),
        access_key_scope: NearAccessKeyScope::FullAccess,
        state: derive_state(STATE_SECRET, &ctx, &hash),
    };
    let proof =
        SigningProof::NearRedirectProof(encode_near_redirect_proof(&payload).expect("encode"));
    let err = p
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("bad signature must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn empty_receiver_function_call_scope_is_scope_violation() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let p = provider(store.clone());
    let key = near_key();
    let account = "alice.near";
    let ctx = ctx_for(&bound_identity(account, &key));
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    let scope = NearAccessKeyScope::FunctionCall {
        receiver_id: String::new(),
        method_names: vec![],
    };
    let proof = valid_proof(&key, account, &ctx, hash, scope);
    let err = p
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("empty receiver must be a scope violation");
    assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
}

#[tokio::test]
async fn replay_after_claim_fails_closed() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let p = provider(store.clone());
    let key = near_key();
    let account = "alice.near";
    let ctx = ctx_for(&bound_identity(account, &key));
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    let proof = valid_proof(&key, account, &ctx, hash, NearAccessKeyScope::FullAccess);
    p.verify_resume(&ctx, &hash, &proof)
        .await
        .expect("first resume succeeds");
    let err = p
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("replay must fail closed");
    assert!(matches!(err, SigningProviderError::GrantClaimFailed));
}

#[tokio::test]
async fn unsealed_grant_fails_closed() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let p = provider(store.clone());
    let key = near_key();
    let account = "alice.near";
    let ctx = ctx_for(&bound_identity(account, &key));
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    // No grant sealed.

    let proof = valid_proof(&key, account, &ctx, hash, NearAccessKeyScope::FullAccess);
    let err = p
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("no grant must fail closed");
    assert!(matches!(err, SigningProviderError::GrantClaimFailed));
}

#[tokio::test]
async fn non_near_redirect_proof_is_rejected() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let p = provider(store);
    let ctx = ctx_for("alice.near");
    let hash = ApprovedTxHash::from_bytes([1u8; 32]);

    let err = p
        .verify_resume(&ctx, &hash, &SigningProof::InjectedProof(vec![1, 2, 3]))
        .await
        .expect_err("non-near-redirect proof must be rejected");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn provider_reports_near_redirect_identity() {
    use ironclaw_signing_provider::{ProviderId, TrustModel};
    let store = Arc::new(InMemorySealedGrantStore::new());
    let p = provider(store);
    assert_eq!(p.provider_id(), ProviderId::NearRedirect);
    assert_eq!(p.trust_model(), TrustModel::ExternalWallet);
}

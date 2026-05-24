//! End-to-end verification tests for the injected-wallet
//! [`InjectedSigningProvider::verify_resume`] security core (attested-signing
//! PR7).
//!
//! These drive the provider behind `Arc<dyn SigningProvider>` (object-safety)
//! and through the sealed-grant store, exercising the full fail-closed contract:
//! valid signer over the bound hash succeeds; a wrong signer is
//! `SignerMismatch`; a tampered hash is `ProofInvalid`; a replayed (already
//! claimed) grant fails closed.

use std::sync::Arc;

use ironclaw_attestation::{
    ApprovedTxHash, AttestedSigningGrant, GrantKey, InMemorySealedGrantStore, SealedGrantStore,
};
use ironclaw_signing_provider::{
    ActorId, ChainId, GateRef, KeyOrAccountId, RunId, ScopeId, SigningContext, SigningProof,
    SigningProvider, SigningProviderError, TenantId, UserId,
};
use ironclaw_wallet_external::{
    InjectedProofPayload, InjectedScheme, InjectedSigningProvider, encode_injected_proof,
};

use ed25519_dalek::{Signer as _, SigningKey as EdSigningKey};
use k256::ecdsa::{SigningKey as EcSigningKey, signature::hazmat::PrehashSigner};
use sha3::{Digest, Keccak256};

const PERSONAL_PREFIX_32: &[u8] = b"\x19Ethereum Signed Message:\n32";

fn lower_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    out
}

// ── EVM key + signing helpers (mirror the provider's verification digest) ──

fn evm_key() -> EcSigningKey {
    EcSigningKey::from_slice(&[0x11u8; 32]).expect("valid secp256k1 key")
}

fn evm_address(key: &EcSigningKey) -> [u8; 20] {
    let vk = key.verifying_key();
    let encoded = vk.to_encoded_point(false);
    let hash = Keccak256::digest(&encoded.as_bytes()[1..]);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]);
    addr
}

/// Produce a 65-byte (r ∥ s ∥ v) personal_sign signature over the 32-byte hash.
fn evm_personal_sign(key: &EcSigningKey, hash: &[u8; 32]) -> Vec<u8> {
    let mut hasher = Keccak256::new();
    hasher.update(PERSONAL_PREFIX_32);
    hasher.update(hash);
    let digest = hasher.finalize();
    let (sig, recid): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) =
        key.sign_prehash(digest.as_slice()).expect("sign");
    let mut out = sig.to_bytes().to_vec();
    out.push(recid.to_byte());
    out
}

// ── Solana key + signing helpers ──

fn solana_key() -> EdSigningKey {
    EdSigningKey::from_bytes(&[0x22u8; 32])
}

fn solana_pubkey(key: &EdSigningKey) -> [u8; 32] {
    key.verifying_key().to_bytes()
}

// ── Context + grant fixtures ──

fn ctx_for(account: &str, chain: &str) -> SigningContext {
    SigningContext {
        tenant: TenantId::new("tenant-a"),
        user: UserId::new("user-1"),
        scope: ScopeId::new("scope-x"),
        actor: ActorId::new("actor-7"),
        run_id: RunId::new("run-42"),
        gate_ref: GateRef::new("gate:abc"),
        chain_id: ChainId::new(chain),
        key_or_account_id: KeyOrAccountId::new(account),
    }
}

async fn seal_grant(store: &InMemorySealedGrantStore, ctx: &SigningContext, hash: ApprovedTxHash) {
    let key = GrantKey::from_context(ctx, hash);
    store
        .seal(AttestedSigningGrant::seal(key, 1_000, None))
        .await
        .expect("seal");
}

#[tokio::test]
async fn evm_valid_signature_from_bound_account_verifies() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider: Arc<dyn SigningProvider> = Arc::new(InjectedSigningProvider::new(store.clone()));

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    let payload = InjectedProofPayload {
        scheme: InjectedScheme::Evm,
        approved_tx_hash: hash,
        claimed_signer: account.clone(),
        signature: evm_personal_sign(&key, hash.as_bytes()),
        public_key: None,
    };
    let proof = SigningProof::InjectedProof(encode_injected_proof(&payload));

    let verified = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect("valid evm proof must verify");
    assert_eq!(verified.proof(), &proof);
}

#[tokio::test]
async fn evm_signer_not_bound_account_is_signer_mismatch() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = InjectedSigningProvider::new(store.clone());

    let key = evm_key();
    // Bind a *different* account than the key recovers to.
    let wrong_account = "0x00000000000000000000000000000000000000bb";
    let ctx = ctx_for(wrong_account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    let payload = InjectedProofPayload {
        scheme: InjectedScheme::Evm,
        approved_tx_hash: hash,
        claimed_signer: wrong_account.to_string(),
        signature: evm_personal_sign(&key, hash.as_bytes()),
        public_key: None,
    };
    let proof = SigningProof::InjectedProof(encode_injected_proof(&payload));

    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("must reject mismatched signer");
    assert!(matches!(err, SigningProviderError::SignerMismatch));
}

#[tokio::test]
async fn evm_tampered_hash_is_proof_invalid() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = InjectedSigningProvider::new(store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let bound_hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, bound_hash).await;

    // Wallet attests to a DIFFERENT hash than the gate bound.
    let attested_hash = ApprovedTxHash::from_bytes([9u8; 32]);
    let payload = InjectedProofPayload {
        scheme: InjectedScheme::Evm,
        approved_tx_hash: attested_hash,
        claimed_signer: account.clone(),
        signature: evm_personal_sign(&key, attested_hash.as_bytes()),
        public_key: None,
    };
    let proof = SigningProof::InjectedProof(encode_injected_proof(&payload));

    let err = provider
        .verify_resume(&ctx, &bound_hash, &proof)
        .await
        .expect_err("tampered hash must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn evm_replay_after_claim_fails_closed() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = InjectedSigningProvider::new(store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    let payload = InjectedProofPayload {
        scheme: InjectedScheme::Evm,
        approved_tx_hash: hash,
        claimed_signer: account.clone(),
        signature: evm_personal_sign(&key, hash.as_bytes()),
        public_key: None,
    };
    let proof = SigningProof::InjectedProof(encode_injected_proof(&payload));

    provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect("first resume succeeds");
    // Replay: the grant is already claimed, so the one-shot CAS fails closed.
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("replay must fail closed");
    assert!(matches!(err, SigningProviderError::GrantClaimFailed));
}

#[tokio::test]
async fn evm_unsealed_grant_fails_closed() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = InjectedSigningProvider::new(store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    // No grant sealed.

    let payload = InjectedProofPayload {
        scheme: InjectedScheme::Evm,
        approved_tx_hash: hash,
        claimed_signer: account.clone(),
        signature: evm_personal_sign(&key, hash.as_bytes()),
        public_key: None,
    };
    let proof = SigningProof::InjectedProof(encode_injected_proof(&payload));

    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("no grant must fail closed");
    assert!(matches!(err, SigningProviderError::GrantClaimFailed));
}

#[tokio::test]
async fn solana_valid_signature_from_bound_account_verifies() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider: Arc<dyn SigningProvider> = Arc::new(InjectedSigningProvider::new(store.clone()));

    let key = solana_key();
    let pubkey = solana_pubkey(&key);
    let account = lower_hex(&pubkey);
    let ctx = ctx_for(&account, "solana:mainnet");
    let hash = ApprovedTxHash::from_bytes([5u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    let sig = key.sign(hash.as_bytes());
    let payload = InjectedProofPayload {
        scheme: InjectedScheme::Solana,
        approved_tx_hash: hash,
        claimed_signer: account.clone(),
        signature: sig.to_bytes().to_vec(),
        public_key: Some(pubkey.to_vec()),
    };
    let proof = SigningProof::InjectedProof(encode_injected_proof(&payload));

    provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect("valid solana proof must verify");
}

#[tokio::test]
async fn solana_wrong_signer_is_signer_mismatch() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = InjectedSigningProvider::new(store.clone());

    let key = solana_key();
    let pubkey = solana_pubkey(&key);
    // Bind a different pubkey than the one in the proof.
    let other = [0x33u8; 32];
    let bound_account = lower_hex(&other);
    let ctx = ctx_for(&bound_account, "solana:mainnet");
    let hash = ApprovedTxHash::from_bytes([5u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    let sig = key.sign(hash.as_bytes());
    let payload = InjectedProofPayload {
        scheme: InjectedScheme::Solana,
        approved_tx_hash: hash,
        claimed_signer: bound_account.clone(),
        signature: sig.to_bytes().to_vec(),
        public_key: Some(pubkey.to_vec()),
    };
    let proof = SigningProof::InjectedProof(encode_injected_proof(&payload));

    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("mismatched solana signer must reject");
    assert!(matches!(err, SigningProviderError::SignerMismatch));
}

#[tokio::test]
async fn solana_bad_signature_bytes_is_proof_invalid() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = InjectedSigningProvider::new(store.clone());

    let key = solana_key();
    let pubkey = solana_pubkey(&key);
    let account = lower_hex(&pubkey);
    let ctx = ctx_for(&account, "solana:mainnet");
    let hash = ApprovedTxHash::from_bytes([5u8; 32]);
    seal_grant(&store, &ctx, hash).await;

    // Signature over a DIFFERENT message → verification fails.
    let sig = key.sign(&[0u8; 32]);
    let payload = InjectedProofPayload {
        scheme: InjectedScheme::Solana,
        approved_tx_hash: hash,
        claimed_signer: account.clone(),
        signature: sig.to_bytes().to_vec(),
        public_key: Some(pubkey.to_vec()),
    };
    let proof = SigningProof::InjectedProof(encode_injected_proof(&payload));

    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("bad solana signature must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn non_injected_proof_is_rejected() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = InjectedSigningProvider::new(store);
    let ctx = ctx_for("0x00000000000000000000000000000000000000aa", "eip155:1");
    let hash = ApprovedTxHash::from_bytes([1u8; 32]);

    let err = provider
        .verify_resume(
            &ctx,
            &hash,
            &SigningProof::WebAuthnAssertionProof(vec![1, 2, 3]),
        )
        .await
        .expect_err("non-injected proof must be rejected");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn proof_payload_round_trips_through_opaque_bytes() {
    let hash = ApprovedTxHash::from_bytes([4u8; 32]);
    let payload = InjectedProofPayload {
        scheme: InjectedScheme::Solana,
        approved_tx_hash: hash,
        claimed_signer: "deadbeef".repeat(8),
        signature: vec![9u8; 64],
        public_key: Some(vec![3u8; 32]),
    };
    let bytes = encode_injected_proof(&payload);
    let back = ironclaw_wallet_external::decode_injected_proof(&bytes).expect("decode");
    assert_eq!(back, payload);
}

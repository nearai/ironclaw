//! End-to-end verification tests for the WalletConnect v2
//! [`WalletConnectSigningProvider`] security cores (attested-signing PR9).
//!
//! Drives the provider behind `Arc<dyn SigningProvider>` (object-safety) and
//! through the sealed-grant store + recorded session binding, exercising the
//! full fail-closed contract:
//!
//! * namespace pinning rejects scope broadening + wrong-chain CAIP-10 accounts
//!   + duplicate scope arrays (T17/T19, #2);
//! * a valid REAL chain signature over the REAL signed-tx payload from the
//!   session-bound account, where that payload equals the
//!   `expected_signing_payload` recorded at initiate → `VerifiedProof` (#1);
//! * a tampered hash, a wrong session topic / nonce (T18), a mismatched signer
//!   (T17), a payload that differs from the approved bytes (#1), a malformed
//!   Unicode-hex account (#3), and a replayed / already-claimed grant (T20) all
//!   fail closed.
//!
//! The relay is never contacted: the session binding `initiate` would record
//! over the relay (PR10) — including the `expected_signing_payload` PR10/PR11's
//! chain encoder will derive — is installed directly via `record_session_binding`,
//! and the wallet signature is minted in-test over the exact `signed_payload`
//! fixture the verifier checks. No synthetic attestation digest exists anymore.

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
    PinnedScope, ProjectId, ProposedScope, SessionBinding, WalletConnectProofPayload,
    WalletConnectSigningProvider, encode_walletconnect_proof, enforce_pinned_scope,
};

use ed25519_dalek::{Signer as _, SigningKey as EdSigningKey};
use k256::ecdsa::{SigningKey as EcSigningKey, signature::hazmat::PrehashSigner};
use sha3::{Digest, Keccak256};

const SESSION_TOPIC: &str = "a3f1c0de";
const NONCE: &[u8] = b"nonce-001";

fn project() -> ProjectId {
    // Publishable, API-key class; injected, never hardcoded in production.
    ProjectId::from("00000000000000000000000000000000")
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    out
}

/// A stand-in for the EVM secp256k1 sighash a real `eth_signTransaction` would
/// cover. In production PR10/PR11 derives this from the decoded tx via a real
/// EIP-2718/RLP encoder; here it is a fixed 32-byte fixture the wallet signs and
/// the binding records as `expected_signing_payload`.
fn evm_sighash_fixture() -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(b"real-evm-signing-payload-fixture");
    let out = h.finalize();
    let mut sighash = [0u8; 32];
    sighash.copy_from_slice(&out);
    sighash
}

/// A stand-in for the Solana ed25519 message bytes a real
/// `solana_signTransaction` would cover.
fn solana_message_fixture() -> Vec<u8> {
    b"real-solana-signing-message-fixture-bytes".to_vec()
}

// ── EVM helpers ──

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

/// 65-byte (r ∥ s ∥ v) signature over the 32-byte sighash payload.
fn evm_sign_payload(key: &EcSigningKey, payload: &[u8; 32]) -> Vec<u8> {
    let (sig, recid): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) =
        key.sign_prehash(payload.as_slice()).expect("sign");
    let mut out = sig.to_bytes().to_vec();
    out.push(recid.to_byte());
    out
}

// ── Solana / ed25519 helpers ──

fn ed_key() -> EdSigningKey {
    EdSigningKey::from_bytes(&[0x22u8; 32])
}

// ── Fixtures ──

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

/// Build a binding whose `expected_signing_payload` is the real bytes the
/// wallet will sign, recorded at initiate from the same decoded tx as `hash`.
fn binding_for(
    account: &str,
    chain: &str,
    hash: ApprovedTxHash,
    expected_signing_payload: Vec<u8>,
) -> SessionBinding {
    SessionBinding {
        session_topic: SESSION_TOPIC.to_string(),
        account: account.to_string(),
        nonce: NONCE.to_vec(),
        pinned: PinnedScope::from_chain_id(&ChainId::new(chain)).expect("pinned"),
        approved_tx_hash: hash,
        expected_signing_payload,
    }
}

// ── Namespace pinning (T17/T19, #2) ──

#[test]
fn pinning_accepts_exact_scope_and_rejects_broadening() {
    let pinned = PinnedScope::from_chain_id(&ChainId::new("eip155:1")).expect("evm");
    enforce_pinned_scope(
        &pinned,
        &ProposedScope {
            chains: vec!["eip155:1".to_string()],
            methods: vec!["eth_signTransaction".to_string()],
            accounts: vec!["eip155:1:0x00000000000000000000000000000000000000aa".to_string()],
        },
    )
    .expect("exact scope accepted");

    // Extra chain (T19).
    assert!(matches!(
        enforce_pinned_scope(
            &pinned,
            &ProposedScope {
                chains: vec!["eip155:1".to_string(), "eip155:10".to_string()],
                methods: vec!["eth_signTransaction".to_string()],
                accounts: vec![],
            },
        )
        .expect_err("broader chains rejected"),
        SigningProviderError::ScopeViolation { .. }
    ));

    // Extra method (T17).
    assert!(matches!(
        enforce_pinned_scope(
            &pinned,
            &ProposedScope {
                chains: vec!["eip155:1".to_string()],
                methods: vec![
                    "eth_signTransaction".to_string(),
                    "eth_sendTransaction".to_string(),
                ],
                accounts: vec![],
            },
        )
        .expect_err("broader methods rejected"),
        SigningProviderError::ScopeViolation { .. }
    ));
}

#[test]
fn pinning_rejects_wrong_chain_caip10_account() {
    let pinned = PinnedScope::from_chain_id(&ChainId::new("eip155:1")).expect("evm");
    // Same address, different chain — must be rejected (#2).
    let err = enforce_pinned_scope(
        &pinned,
        &ProposedScope {
            chains: vec!["eip155:1".to_string()],
            methods: vec!["eth_signTransaction".to_string()],
            accounts: vec!["eip155:137:0x00000000000000000000000000000000000000aa".to_string()],
        },
    )
    .expect_err("wrong-chain account rejected");
    assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
}

#[test]
fn pinning_rejects_duplicate_scope_arrays() {
    let pinned = PinnedScope::from_chain_id(&ChainId::new("eip155:1")).expect("evm");
    let dup_chain = enforce_pinned_scope(
        &pinned,
        &ProposedScope {
            chains: vec!["eip155:1".to_string(), "eip155:1".to_string()],
            methods: vec!["eth_signTransaction".to_string()],
            accounts: vec![],
        },
    )
    .expect_err("duplicate chain rejected");
    assert!(matches!(
        dup_chain,
        SigningProviderError::ScopeViolation { .. }
    ));

    let dup_method = enforce_pinned_scope(
        &pinned,
        &ProposedScope {
            chains: vec!["eip155:1".to_string()],
            methods: vec![
                "eth_signTransaction".to_string(),
                "eth_signTransaction".to_string(),
            ],
            accounts: vec![],
        },
    )
    .expect_err("duplicate method rejected");
    assert!(matches!(
        dup_method,
        SigningProviderError::ScopeViolation { .. }
    ));
}

// ── verify_resume: EVM (real signed-tx payload, #1) ──

fn evm_proof(
    key: &EcSigningKey,
    account: &str,
    hash: ApprovedTxHash,
    topic: &str,
    nonce: &[u8],
    signed_payload: Vec<u8>,
) -> SigningProof {
    // Sign the REAL payload bytes (the secp256k1 sighash), exactly as
    // eth_signTransaction would. There is no synthetic attestation digest.
    let sighash: [u8; 32] = signed_payload
        .clone()
        .try_into()
        .expect("evm signed payload is a 32-byte sighash");
    let payload = WalletConnectProofPayload {
        session_topic: topic.to_string(),
        approved_tx_hash: hash,
        claimed_signer: account.to_string(),
        nonce: nonce.to_vec(),
        signed_payload,
        signature: evm_sign_payload(key, &sighash),
        public_key: None,
    };
    SigningProof::WalletConnectProof(encode_walletconnect_proof(&payload).expect("encode proof"))
}

#[tokio::test]
async fn evm_valid_proof_over_real_signed_tx_verifies() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    seal_grant(&store, &ctx, hash).await;
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "eip155:1", hash, signed.to_vec()),
    );

    let proof = evm_proof(&key, &account, hash, SESSION_TOPIC, NONCE, signed.to_vec());
    let provider: Arc<dyn SigningProvider> = Arc::new(provider);
    let verified = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect("valid evm walletconnect proof over the real signed tx must verify");
    assert_eq!(verified.proof(), &proof);
}

#[tokio::test]
async fn evm_session_account_casing_mismatch_still_verifies() {
    // EVM addresses are case-insensitive hex; the WC session may settle the
    // account in EIP-55 mixed case while the gate stores it lowercase. The
    // defense-in-depth account string compare must be case-insensitive so a
    // pure-casing difference does not spuriously reject. The authoritative
    // signer binding remains the byte-exact chain-signature recovery.
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    seal_grant(&store, &ctx, hash).await;
    // Binding records the SAME address but UPPER-cased (different casing only).
    let upper_account = account.to_ascii_uppercase();
    assert_ne!(upper_account, account);
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&upper_account, "eip155:1", hash, signed.to_vec()),
    );

    let proof = evm_proof(&key, &account, hash, SESSION_TOPIC, NONCE, signed.to_vec());
    let provider: Arc<dyn SigningProvider> = Arc::new(provider);
    let verified = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect("case-only account difference must not reject a valid proof");
    assert_eq!(verified.proof(), &proof);
}

#[tokio::test]
async fn evm_payload_not_matching_approved_bytes_is_rejected() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let approved = evm_sighash_fixture();
    seal_grant(&store, &ctx, hash).await;
    // Binding records the APPROVED payload.
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "eip155:1", hash, approved.to_vec()),
    );

    // Wallet signs a DIFFERENT transaction than the approved one (the core #1
    // attack: a validly-signed but unapproved tx). The signature is valid for
    // the bound account over `other`, but `other != expected_signing_payload`.
    let mut other = approved;
    other[0] ^= 0xff;
    let proof = evm_proof(&key, &account, hash, SESSION_TOPIC, NONCE, other.to_vec());
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("signed payload != approved bytes must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn evm_signer_not_bound_account_is_signer_mismatch() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    // Bind a different account than the key recovers to, but record the binding
    // on the REAL signing account so the account-binding check passes and we
    // reach the signer recovery (which must mismatch the gate's bound account).
    let wrong = "0x00000000000000000000000000000000000000bb";
    let ctx = ctx_for(wrong, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    seal_grant(&store, &ctx, hash).await;
    let real_account = format!("0x{}", lower_hex(&evm_address(&key)));
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&real_account, "eip155:1", hash, signed.to_vec()),
    );

    let proof = evm_proof(
        &key,
        &real_account,
        hash,
        SESSION_TOPIC,
        NONCE,
        signed.to_vec(),
    );
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("mismatched signer must reject");
    assert!(matches!(err, SigningProviderError::SignerMismatch));
}

#[tokio::test]
async fn evm_tampered_hash_is_proof_invalid() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let bound_hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    seal_grant(&store, &ctx, bound_hash).await;
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "eip155:1", bound_hash, signed.to_vec()),
    );

    // Wallet attests to a DIFFERENT hash than the gate bound.
    let attested = ApprovedTxHash::from_bytes([9u8; 32]);
    let proof = evm_proof(
        &key,
        &account,
        attested,
        SESSION_TOPIC,
        NONCE,
        signed.to_vec(),
    );
    let err = provider
        .verify_resume(&ctx, &bound_hash, &proof)
        .await
        .expect_err("tampered hash must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn evm_wrong_session_topic_is_rejected_t18() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    seal_grant(&store, &ctx, hash).await;
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "eip155:1", hash, signed.to_vec()),
    );

    // Proof minted under a DIFFERENT session topic (relay/session compromise).
    let proof = evm_proof(&key, &account, hash, "deadbeef", NONCE, signed.to_vec());
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("wrong session topic must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn evm_wrong_nonce_is_rejected_t18() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    seal_grant(&store, &ctx, hash).await;
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "eip155:1", hash, signed.to_vec()),
    );

    // Proof carries a stale / forged nonce (replay defense).
    let proof = evm_proof(
        &key,
        &account,
        hash,
        SESSION_TOPIC,
        b"other-nonce",
        signed.to_vec(),
    );
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("wrong nonce must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn evm_missing_session_binding_is_rejected() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    seal_grant(&store, &ctx, hash).await;
    // No session binding recorded.

    let proof = evm_proof(&key, &account, hash, SESSION_TOPIC, NONCE, signed.to_vec());
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("missing binding must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn evm_malformed_response_does_not_burn_binding() {
    // A malformed relay/wallet response (wrong nonce) must NOT consume the
    // recorded binding — a subsequent valid proof must still verify (the
    // "validate first, consume only on success" recommendation).
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    seal_grant(&store, &ctx, hash).await;
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "eip155:1", hash, signed.to_vec()),
    );

    // First: malformed (wrong nonce) — must fail but NOT burn the binding.
    let bad = evm_proof(
        &key,
        &account,
        hash,
        SESSION_TOPIC,
        b"forged-nonce",
        signed.to_vec(),
    );
    assert!(matches!(
        provider
            .verify_resume(&ctx, &hash, &bad)
            .await
            .expect_err("malformed must fail"),
        SigningProviderError::ProofInvalid { .. }
    ));

    // Then: a valid proof against the still-present binding must verify.
    let good = evm_proof(&key, &account, hash, SESSION_TOPIC, NONCE, signed.to_vec());
    provider
        .verify_resume(&ctx, &hash, &good)
        .await
        .expect("valid proof after a malformed attempt must still verify");
}

#[tokio::test]
async fn evm_replay_after_claim_fails_closed_t20() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = Arc::new(WalletConnectSigningProvider::new(project(), store.clone()));

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    seal_grant(&store, &ctx, hash).await;
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "eip155:1", hash, signed.to_vec()),
    );

    let proof = evm_proof(&key, &account, hash, SESSION_TOPIC, NONCE, signed.to_vec());
    provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect("first resume succeeds");

    // Re-record the binding so the replay reaches the grant CAS (the binding is
    // consumed on first success). The one-shot grant must still reject the replay.
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "eip155:1", hash, signed.to_vec()),
    );
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("replay must fail closed");
    assert!(matches!(err, SigningProviderError::GrantClaimFailed));
}

#[tokio::test]
async fn evm_unsealed_grant_fails_closed() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    // No grant sealed.
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "eip155:1", hash, signed.to_vec()),
    );

    let proof = evm_proof(&key, &account, hash, SESSION_TOPIC, NONCE, signed.to_vec());
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("no grant must fail closed");
    assert!(matches!(err, SigningProviderError::GrantClaimFailed));
}

#[tokio::test]
async fn evm_malformed_unicode_hex_account_fails_closed() {
    // A bound account carrying a non-ASCII even-byte string used to panic when
    // the hex parser sliced &str by byte offset (#3). It must now fail closed.
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    // 40-byte (20 char-pairs) string of a 2-byte multibyte char: ASCII byte
    // length is even but slicing at odd byte offsets crosses a char boundary.
    let bad_account = "é".repeat(20); // 40 bytes, all non-ASCII
    assert_eq!(bad_account.len(), 40);
    let ctx = ctx_for(&bad_account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    seal_grant(&store, &ctx, hash).await;
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&bad_account, "eip155:1", hash, signed.to_vec()),
    );

    let proof = evm_proof(
        &key,
        &bad_account,
        hash,
        SESSION_TOPIC,
        NONCE,
        signed.to_vec(),
    );
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("malformed unicode hex account must fail closed (not panic)");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn malformed_unicode_hex_in_proof_field_fails_closed() {
    // A relay-supplied proof whose hex-encoded fields contain non-ASCII even-byte
    // content must decode-fail (ProofInvalid), never panic (#3).
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store);
    let ctx = ctx_for("0x00000000000000000000000000000000000000aa", "eip155:1");
    let hash = ApprovedTxHash::from_bytes([1u8; 32]);

    // Hand-craft JSON with a malformed-unicode hex `signature` field.
    let json = format!(
        r#"{{"session_topic":"{SESSION_TOPIC}","approved_tx_hash":{:?},"claimed_signer":"0x00000000000000000000000000000000000000aa","nonce":"6e6f6e6365","signed_payload":"00","signature":"éé"}}"#,
        hash.as_bytes()
    );
    let proof = SigningProof::WalletConnectProof(json.into_bytes());
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("malformed unicode hex in proof must fail closed (not panic)");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

// ── verify_resume: Solana (ed25519, real signed message) ──

#[tokio::test]
async fn solana_valid_proof_over_real_message_verifies() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider: Arc<dyn SigningProvider> =
        Arc::new(WalletConnectSigningProvider::new(project(), store.clone()));

    let key = ed_key();
    let pubkey = key.verifying_key().to_bytes();
    let account = lower_hex(&pubkey);
    let ctx = ctx_for(&account, "solana:mainnet");
    let hash = ApprovedTxHash::from_bytes([5u8; 32]);
    let message = solana_message_fixture();
    seal_grant(&store, &ctx, hash).await;

    // Record binding via a concrete provider, then drive via the dyn handle.
    let inner = WalletConnectSigningProvider::new(project(), store.clone());
    inner.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "solana:mainnet", hash, message.clone()),
    );

    // ed25519 signs the REAL message bytes, as solana_signTransaction would.
    let sig = key.sign(&message);
    let payload = WalletConnectProofPayload {
        session_topic: SESSION_TOPIC.to_string(),
        approved_tx_hash: hash,
        claimed_signer: account.clone(),
        nonce: NONCE.to_vec(),
        signed_payload: message,
        signature: sig.to_bytes().to_vec(),
        public_key: Some(pubkey.to_vec()),
    };
    let proof = SigningProof::WalletConnectProof(
        encode_walletconnect_proof(&payload).expect("encode proof"),
    );

    inner
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect("valid solana walletconnect proof over the real message must verify");

    // Object-safety sanity: the dyn handle is usable.
    let _ = provider.provider_id();
}

#[tokio::test]
async fn solana_wrong_signer_is_signer_mismatch() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = ed_key();
    let pubkey = key.verifying_key().to_bytes();
    // Bind a different pubkey than the proof's.
    let other = [0x33u8; 32];
    let bound_account = lower_hex(&other);
    let ctx = ctx_for(&bound_account, "solana:mainnet");
    let hash = ApprovedTxHash::from_bytes([5u8; 32]);
    let message = solana_message_fixture();
    seal_grant(&store, &ctx, hash).await;
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&bound_account, "solana:mainnet", hash, message.clone()),
    );

    let sig = key.sign(&message);
    let payload = WalletConnectProofPayload {
        session_topic: SESSION_TOPIC.to_string(),
        approved_tx_hash: hash,
        claimed_signer: bound_account.clone(),
        nonce: NONCE.to_vec(),
        signed_payload: message,
        signature: sig.to_bytes().to_vec(),
        public_key: Some(pubkey.to_vec()),
    };
    let proof = SigningProof::WalletConnectProof(
        encode_walletconnect_proof(&payload).expect("encode proof"),
    );

    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("mismatched solana signer must reject");
    assert!(matches!(err, SigningProviderError::SignerMismatch));
}

#[tokio::test]
async fn non_walletconnect_proof_is_rejected() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store);
    let ctx = ctx_for("0x00000000000000000000000000000000000000aa", "eip155:1");
    let hash = ApprovedTxHash::from_bytes([1u8; 32]);

    let err = provider
        .verify_resume(&ctx, &hash, &SigningProof::InjectedProof(vec![1, 2, 3]))
        .await
        .expect_err("non-walletconnect proof must be rejected");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn near_chain_is_unsupported_fail_closed() {
    // NEAR is explicitly unsupported on the WC provider until a real verifier
    // exists — initiate/verify must fail closed at scope resolution.
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store);
    let account = "0011223344556677889900112233445566778899001122334455667788990011";
    let ctx = ctx_for(account, "near:mainnet");
    let hash = ApprovedTxHash::from_bytes([1u8; 32]);
    // A well-formed (decodable) proof so resolution reaches scope pinning, which
    // is where NEAR is rejected.
    let payload = WalletConnectProofPayload {
        session_topic: SESSION_TOPIC.to_string(),
        approved_tx_hash: hash,
        claimed_signer: account.to_string(),
        nonce: NONCE.to_vec(),
        signed_payload: vec![0u8; 32],
        signature: vec![0u8; 64],
        public_key: Some(vec![0u8; 32]),
    };
    let proof = SigningProof::WalletConnectProof(
        encode_walletconnect_proof(&payload).expect("encode proof"),
    );
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("near must fail closed");
    assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
}

// ── Crypto-field error paths (henrypark review: explicit fail-closed coverage) ──

#[tokio::test]
async fn evm_signed_payload_wrong_length_is_proof_invalid() {
    // verify_evm requires the signed_payload to be exactly the 32-byte sighash.
    // A wrong-length payload (here 31 bytes) must fail closed as ProofInvalid.
    // The binding records the SAME wrong-length payload so the earlier
    // payload-match check passes and we actually reach verify_evm's length gate.
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let short_payload = vec![0xabu8; 31]; // not 32 bytes
    seal_grant(&store, &ctx, hash).await;
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "eip155:1", hash, short_payload.clone()),
    );

    // A syntactically-valid 65-byte signature; verify_evm rejects on the
    // payload length before signature recovery is attempted.
    let payload = WalletConnectProofPayload {
        session_topic: SESSION_TOPIC.to_string(),
        approved_tx_hash: hash,
        claimed_signer: account.clone(),
        nonce: NONCE.to_vec(),
        signed_payload: short_payload,
        signature: vec![0u8; 65],
        public_key: None,
    };
    let proof =
        SigningProof::WalletConnectProof(encode_walletconnect_proof(&payload).expect("encode"));
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("non-32-byte evm signed payload must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn evm_invalid_recovery_id_v_is_proof_invalid() {
    // recovery_id_from_v rejects v bytes outside {0,1,27,28,>=35}. A 65-byte
    // signature whose v byte is, e.g., 5 must fail closed as ProofInvalid.
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = evm_key();
    let account = format!("0x{}", lower_hex(&evm_address(&key)));
    let ctx = ctx_for(&account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let signed = evm_sighash_fixture();
    seal_grant(&store, &ctx, hash).await;
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "eip155:1", hash, signed.to_vec()),
    );

    // Valid r∥s scalars, but an out-of-range v byte.
    let mut signature = evm_sign_payload(&key, &signed);
    signature[64] = 5; // invalid recovery id
    let payload = WalletConnectProofPayload {
        session_topic: SESSION_TOPIC.to_string(),
        approved_tx_hash: hash,
        claimed_signer: account.clone(),
        nonce: NONCE.to_vec(),
        signed_payload: signed.to_vec(),
        signature,
        public_key: None,
    };
    let proof =
        SigningProof::WalletConnectProof(encode_walletconnect_proof(&payload).expect("encode"));
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("invalid evm recovery id v must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn solana_missing_public_key_is_proof_invalid() {
    // verify_chain_signature for the ed25519 family requires public_key; a
    // Solana proof with public_key=None must fail closed as ProofInvalid.
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = ed_key();
    let pubkey = key.verifying_key().to_bytes();
    let account = lower_hex(&pubkey);
    let ctx = ctx_for(&account, "solana:mainnet");
    let hash = ApprovedTxHash::from_bytes([5u8; 32]);
    let message = solana_message_fixture();
    seal_grant(&store, &ctx, hash).await;
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "solana:mainnet", hash, message.clone()),
    );

    let sig = key.sign(&message);
    let payload = WalletConnectProofPayload {
        session_topic: SESSION_TOPIC.to_string(),
        approved_tx_hash: hash,
        claimed_signer: account.clone(),
        nonce: NONCE.to_vec(),
        signed_payload: message,
        signature: sig.to_bytes().to_vec(),
        public_key: None, // missing — must fail closed
    };
    let proof =
        SigningProof::WalletConnectProof(encode_walletconnect_proof(&payload).expect("encode"));
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("solana proof missing public_key must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn solana_wrong_length_crypto_fields_is_proof_invalid() {
    // verify_ed25519 rejects non-64-byte signatures and non-32-byte public keys.
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store.clone());

    let key = ed_key();
    let pubkey = key.verifying_key().to_bytes();
    let account = lower_hex(&pubkey);
    let ctx = ctx_for(&account, "solana:mainnet");
    let hash = ApprovedTxHash::from_bytes([5u8; 32]);
    let message = solana_message_fixture();
    seal_grant(&store, &ctx, hash).await;

    // Case 1: wrong-length signature (63 bytes).
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "solana:mainnet", hash, message.clone()),
    );
    let short_sig = WalletConnectProofPayload {
        session_topic: SESSION_TOPIC.to_string(),
        approved_tx_hash: hash,
        claimed_signer: account.clone(),
        nonce: NONCE.to_vec(),
        signed_payload: message.clone(),
        signature: vec![0u8; 63],
        public_key: Some(pubkey.to_vec()),
    };
    let proof =
        SigningProof::WalletConnectProof(encode_walletconnect_proof(&short_sig).expect("encode"));
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("non-64-byte ed25519 signature must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));

    // Case 2: wrong-length public key (31 bytes). The bound account is the real
    // pubkey hex so we reach verify_ed25519, where the 32-byte check fails.
    provider.record_session_binding(
        &ctx.gate_ref,
        binding_for(&account, "solana:mainnet", hash, message.clone()),
    );
    let sig = key.sign(&message);
    let short_pk = WalletConnectProofPayload {
        session_topic: SESSION_TOPIC.to_string(),
        approved_tx_hash: hash,
        claimed_signer: account.clone(),
        nonce: NONCE.to_vec(),
        signed_payload: message,
        signature: sig.to_bytes().to_vec(),
        public_key: Some(vec![0u8; 31]),
    };
    let proof =
        SigningProof::WalletConnectProof(encode_walletconnect_proof(&short_pk).expect("encode"));
    let err = provider
        .verify_resume(&ctx, &hash, &proof)
        .await
        .expect_err("non-32-byte ed25519 public key must fail closed");
    assert!(matches!(err, SigningProviderError::ProofInvalid { .. }));
}

#[tokio::test]
async fn initiate_returns_awaiting_user_action() {
    // initiate pins the scope (no relay traffic in PR9) and signals the ceremony
    // proceeds out of band via AwaitingUserAction.
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store);

    let account = "0x00000000000000000000000000000000000000aa";
    let ctx = ctx_for(account, "eip155:1");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let decoded = DecodedTransaction::from_opaque(vec![1u8, 2, 3]);
    let rendered = RenderedTx::from_opaque(vec![4u8, 5, 6]);

    let outcome = provider
        .initiate(&ctx, &decoded, &rendered, &hash)
        .await
        .expect("initiate over a pinnable chain must succeed");
    assert!(matches!(
        outcome,
        InitiationOutcome::AwaitingUserAction { .. }
    ));
}

#[tokio::test]
async fn initiate_unsupported_chain_fails_closed() {
    // initiate must fail closed (ScopeViolation) for a chain it cannot pin
    // (NEAR is unsupported on the WC provider in PR9).
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider = WalletConnectSigningProvider::new(project(), store);

    let ctx = ctx_for("near-account.near", "near:mainnet");
    let hash = ApprovedTxHash::from_bytes([7u8; 32]);
    let decoded = DecodedTransaction::from_opaque(vec![1u8]);
    let rendered = RenderedTx::from_opaque(vec![2u8]);

    let err = provider
        .initiate(&ctx, &decoded, &rendered, &hash)
        .await
        .expect_err("unsupported chain must fail closed at scope pinning");
    assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
}

#[tokio::test]
async fn object_safe_behind_dyn_arc() {
    let store = Arc::new(InMemorySealedGrantStore::new());
    let provider: Arc<dyn SigningProvider> =
        Arc::new(WalletConnectSigningProvider::new(project(), store));
    assert_eq!(
        provider.provider_id(),
        ironclaw_signing_provider::ProviderId::WalletConnect
    );
    assert_eq!(
        provider.trust_model(),
        ironclaw_signing_provider::TrustModel::ExternalWallet
    );
}

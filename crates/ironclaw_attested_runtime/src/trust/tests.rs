//! Tests for the trust-registration ceremony.
//!
//! Covers EVM/Solana/NEAR happy paths, wrong-signer / forged-challenge /
//! expired / replayed rejection, idempotent re-initiate, revoke, the
//! `lookup_active_binding` seam (active vs none/expired/revoked), and
//! cross-tenant lookup isolation.

use ed25519_dalek::{Signer as _, SigningKey};
use k256::ecdsa::{RecoveryId, Signature as EcSignature, SigningKey as EcSigningKey};
use sha3::{Digest, Keccak256};

use ironclaw_signing_provider::{ActorId, ChainId, TenantId, UserId};

use super::*;

const TTL_MS: u64 = 60_000;
const T0: u64 = 1_000_000;

/// Deterministic, monotonic nonce source for tests.
struct SeqNonce {
    counter: std::sync::atomic::AtomicU64,
}
impl SeqNonce {
    fn new() -> Self {
        Self {
            counter: std::sync::atomic::AtomicU64::new(1),
        }
    }
}
impl NonceSource for SeqNonce {
    fn next_nonce_hex(&self) -> Result<String, TrustError> {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(format!("{n:032x}"))
    }
}

fn registrar() -> TrustRegistrar<InMemoryTrustStore, AlwaysTrustNearAccessKeyVerifier> {
    TrustRegistrar::new(
        InMemoryTrustStore::new(),
        Box::new(SeqNonce::new()),
        AlwaysTrustNearAccessKeyVerifier,
        TTL_MS,
    )
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// ---- signing helpers (test-side wallet) ----

/// secp256k1 keypair → (signing key, lowercase 0x address).
fn evm_keypair() -> (EcSigningKey, String) {
    let sk = EcSigningKey::from_bytes(&[7u8; 32].into()).expect("k256 key");
    let vk = sk.verifying_key();
    let encoded = vk.to_encoded_point(false);
    let hash = Keccak256::digest(&encoded.as_bytes()[1..]);
    let addr = format!("0x{}", hex_lower(&hash[12..]));
    (sk, addr)
}

/// EIP-191 personal_sign over the challenge digest → 65-byte (r∥s∥v).
fn evm_sign(sk: &EcSigningKey, challenge: &TrustChallenge) -> Vec<u8> {
    let mut hasher = Keccak256::new();
    hasher.update(b"\x19Ethereum Signed Message:\n32");
    hasher.update(challenge.digest());
    let eip191: [u8; 32] = hasher.finalize().into();
    let (sig, rec): (EcSignature, RecoveryId) = sk.sign_prehash_recoverable(&eip191).expect("sign");
    let mut out = sig.to_bytes().to_vec();
    out.push(rec.to_byte());
    out
}

/// ed25519 keypair → (signing key, lowercase hex pubkey).
fn ed_keypair(seed: u8) -> (SigningKey, String) {
    let sk = SigningKey::from_bytes(&[seed; 32]);
    let pk = hex_lower(sk.verifying_key().as_bytes());
    (sk, pk)
}

fn ed_sign(sk: &SigningKey, challenge: &TrustChallenge) -> Vec<u8> {
    sk.sign(&challenge.digest()).to_bytes().to_vec()
}

// ---- EVM ----

#[tokio::test]
async fn evm_registration_happy_path_and_lookup() {
    let reg = registrar();
    let (sk, addr) = evm_keypair();
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr.clone(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .expect("initiate");
    assert_eq!(enr.state, EnrollmentState::Challenged);

    let sig = evm_sign(&sk, &chal);
    let binding = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: chal,
                signature: sig,
                public_key_hex: None,
            },
            T0 + 1,
        )
        .await
        .expect("complete");
    assert_eq!(binding.status, BindingStatus::Active);
    assert_eq!(binding.account_or_key, addr);

    let found = reg
        .lookup_active_binding(
            &TenantId::new("t1"),
            &UserId::new("u1"),
            &ChainId::new("eip155:1"),
            "mainnet",
            T0 + 2,
        )
        .await
        .expect("active binding");
    assert_eq!(found.account_or_key, addr);
}

#[tokio::test]
async fn evm_wrong_signer_rejected() {
    let reg = registrar();
    let (_sk, addr) = evm_keypair();
    // A *different* key signs, but the claim is `addr`.
    let other = EcSigningKey::from_bytes(&[9u8; 32].into()).unwrap();
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr,
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = evm_sign(&other, &chal);
    let err = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: chal,
                signature: sig,
                public_key_hex: None,
            },
            T0 + 1,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, TrustError::Verification(_)));
    // No binding minted.
    assert!(
        reg.lookup_active_binding(
            &TenantId::new("t1"),
            &UserId::new("u1"),
            &ChainId::new("eip155:1"),
            "mainnet",
            T0 + 2
        )
        .await
        .is_none()
    );
}

#[tokio::test]
async fn forged_challenge_rejected() {
    let reg = registrar();
    let (sk, addr) = evm_keypair();
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr,
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    // Tamper: change the network in the submitted challenge (and re-sign it so
    // the signature is valid over the forged challenge). The digest no longer
    // matches the issued challenge_hash → ChallengeMismatch.
    let mut forged = chal.clone();
    forged.network = "testnet".into();
    let sig = evm_sign(&sk, &forged);
    let err = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: forged,
                signature: sig,
                public_key_hex: None,
            },
            T0 + 1,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, TrustError::ChallengeMismatch));
}

#[tokio::test]
async fn expired_challenge_rejected() {
    let reg = registrar();
    let (sk, addr) = evm_keypair();
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr,
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = evm_sign(&sk, &chal);
    let err = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: chal,
                signature: sig,
                public_key_hex: None,
            },
            T0 + TTL_MS + 1, // past expiry
        )
        .await
        .unwrap_err();
    assert!(matches!(err, TrustError::ChallengeExpired(_)));
}

#[tokio::test]
async fn replayed_challenge_rejected() {
    let reg = registrar();
    let (sk, addr) = evm_keypair();
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr,
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = evm_sign(&sk, &chal);
    let submit = SignedChallenge {
        challenge: chal,
        signature: sig,
        public_key_hex: None,
    };
    reg.complete_registration(&enr.enrollment_id, submit.clone(), T0 + 1)
        .await
        .expect("first completes");
    // Second use of the same challenge: enrollment is now Active, not Challenged.
    let err = reg
        .complete_registration(&enr.enrollment_id, submit, T0 + 2)
        .await
        .unwrap_err();
    assert!(matches!(err, TrustError::NotChallengeable { .. }));
}

#[tokio::test]
async fn idempotent_reinitiate_resumes_same_challenge() {
    let reg = registrar();
    let (_sk, addr) = evm_keypair();
    let (enr1, chal1) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr.clone(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let (enr2, chal2) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr,
            ActorId::new("a1"),
            T0 + 5,
        )
        .await
        .unwrap();
    assert_eq!(enr1.enrollment_id, enr2.enrollment_id);
    assert_eq!(chal1.digest(), chal2.digest());
}

// ---- Solana ----

#[tokio::test]
async fn solana_registration_happy_path() {
    let reg = registrar();
    let (sk, pk) = ed_keypair(3);
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("solana:mainnet"),
            "mainnet".into(),
            pk.clone(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = ed_sign(&sk, &chal);
    let binding = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: chal,
                signature: sig,
                public_key_hex: Some(pk.clone()),
            },
            T0 + 1,
        )
        .await
        .expect("complete");
    assert_eq!(binding.account_or_key, pk);
    assert!(binding.access_key.is_none());
}

// ---- NEAR ----

#[tokio::test]
async fn near_registration_records_account_and_key() {
    let reg = registrar();
    let (sk, pk) = ed_keypair(5);
    let account = "alice.near".to_string();
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("near:mainnet"),
            "mainnet".into(),
            account.clone(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = ed_sign(&sk, &chal);
    let binding = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: chal,
                signature: sig,
                public_key_hex: Some(pk.clone()),
            },
            T0 + 1,
        )
        .await
        .expect("complete");
    assert_eq!(binding.account_or_key, account);
    assert_eq!(binding.access_key.as_deref(), Some(pk.as_str()));
}

/// A NEAR verifier that fails closed — registration must reject.
struct RejectingNearVerifier;
impl NearAccessKeyVerifier for RejectingNearVerifier {
    fn verify_access_key(
        &self,
        _account_id: &str,
        _public_key_hex: &str,
    ) -> Result<(), ironclaw_signing_provider::SigningProviderError> {
        Err(
            ironclaw_signing_provider::SigningProviderError::ProofInvalid {
                reason: "access key not on chain".into(),
            },
        )
    }
}

#[tokio::test]
async fn near_fails_closed_when_verifier_rejects() {
    let reg = TrustRegistrar::new(
        InMemoryTrustStore::new(),
        Box::new(SeqNonce::new()),
        RejectingNearVerifier,
        TTL_MS,
    );
    let (sk, pk) = ed_keypair(6);
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("near:mainnet"),
            "mainnet".into(),
            "bob.near".into(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = ed_sign(&sk, &chal);
    let err = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: chal,
                signature: sig,
                public_key_hex: Some(pk),
            },
            T0 + 1,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, TrustError::Verification(_)));
}

// ---- revoke / expiry / isolation ----

async fn make_active_evm_binding(
    reg: &TrustRegistrar<InMemoryTrustStore, AlwaysTrustNearAccessKeyVerifier>,
    tenant: &str,
) {
    let (sk, addr) = evm_keypair();
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new(tenant),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr,
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = evm_sign(&sk, &chal);
    reg.complete_registration(
        &enr.enrollment_id,
        SignedChallenge {
            challenge: chal,
            signature: sig,
            public_key_hex: None,
        },
        T0 + 1,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn revoke_makes_binding_unresolvable() {
    let reg = registrar();
    make_active_evm_binding(&reg, "t1").await;
    assert!(
        reg.lookup_active_binding(
            &TenantId::new("t1"),
            &UserId::new("u1"),
            &ChainId::new("eip155:1"),
            "mainnet",
            T0 + 2
        )
        .await
        .is_some()
    );
    let revoked = reg
        .revoke_binding(
            &TenantId::new("t1"),
            &UserId::new("u1"),
            &ChainId::new("eip155:1"),
            "mainnet",
            T0 + 3,
        )
        .await;
    assert!(revoked);
    assert!(
        reg.lookup_active_binding(
            &TenantId::new("t1"),
            &UserId::new("u1"),
            &ChainId::new("eip155:1"),
            "mainnet",
            T0 + 4
        )
        .await
        .is_none()
    );
}

#[tokio::test]
async fn expired_binding_does_not_resolve() {
    // Build a binding manually with an expiry, then look it up past expiry.
    let store = InMemoryTrustStore::new();
    let binding = TrustedSignerBinding {
        tenant_id: TenantId::new("t1"),
        user_id: UserId::new("u1"),
        chain_id: ChainId::new("eip155:1"),
        network: "mainnet".into(),
        account_or_key: "0xabc".into(),
        access_key: None,
        evidence_hash: "deadbeef".into(),
        status: BindingStatus::Active,
        created_at_unix_ms: T0,
        expires_at_unix_ms: Some(T0 + 100),
        revoked_at_unix_ms: None,
    };
    store.put_binding(binding).await;
    assert!(
        store
            .lookup_active_binding(
                &TenantId::new("t1"),
                &UserId::new("u1"),
                &ChainId::new("eip155:1"),
                "mainnet",
                T0 + 50
            )
            .await
            .is_some()
    );
    assert!(
        store
            .lookup_active_binding(
                &TenantId::new("t1"),
                &UserId::new("u1"),
                &ChainId::new("eip155:1"),
                "mainnet",
                T0 + 200
            )
            .await
            .is_none()
    );
}

#[tokio::test]
async fn cross_tenant_lookup_is_isolated() {
    let reg = registrar();
    make_active_evm_binding(&reg, "tenant-a").await;
    // Same user/chain/network but a different tenant must not resolve.
    assert!(
        reg.lookup_active_binding(
            &TenantId::new("tenant-b"),
            &UserId::new("u1"),
            &ChainId::new("eip155:1"),
            "mainnet",
            T0 + 2
        )
        .await
        .is_none()
    );
    // Tenant A still resolves.
    assert!(
        reg.lookup_active_binding(
            &TenantId::new("tenant-a"),
            &UserId::new("u1"),
            &ChainId::new("eip155:1"),
            "mainnet",
            T0 + 2
        )
        .await
        .is_some()
    );
}

// ---- atomic race-safety primitives (#4055 review) ----

/// `put_enrollment_if_absent` is an atomic get-or-insert: the first candidate
/// for an idempotency key wins and is returned with `inserted = true`; a second
/// candidate for the same key does NOT overwrite it and is told `inserted =
/// false`, receiving the stored (winning) enrollment instead. This is the fix
/// for the initiate double-submit race where the later write would otherwise
/// clobber the earlier challenge, leaving one client a non-completable handle.
#[tokio::test]
async fn put_enrollment_if_absent_is_get_or_insert() {
    let store = InMemoryTrustStore::new();
    let mk = |id: &str| {
        let mut e = TrustEnrollment::pending(
            id.to_string(),
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            "0xabc".into(),
            "idem-key".into(),
            ActorId::new("a1"),
            T0,
        );
        e.mark_challenged("hash".into(), "nonce".into(), T0 + TTL_MS, T0);
        e
    };

    let (won, inserted) = store.put_enrollment_if_absent(mk("first")).await;
    assert!(inserted, "first candidate must win the slot");
    assert_eq!(won.enrollment_id, "first");

    let (returned, inserted2) = store.put_enrollment_if_absent(mk("second")).await;
    assert!(!inserted2, "second candidate for same key must not insert");
    assert_eq!(
        returned.enrollment_id, "first",
        "loser must receive the winner's enrollment, not its own"
    );
    // The stored slot is still the winner.
    assert_eq!(
        store
            .get_enrollment("idem-key")
            .await
            .expect("stored")
            .enrollment_id,
        "first"
    );
}

/// `compare_and_swap_enrollment_state` only transitions when the current state
/// matches `expected`. This is the single-use guard for completion: the second
/// completer's CAS from `Challenged` fails because the first already moved the
/// state to `Active`.
#[tokio::test]
async fn cas_enrollment_state_enforces_expected_state() {
    let store = InMemoryTrustStore::new();
    let mut e = TrustEnrollment::pending(
        "e1".into(),
        TenantId::new("t1"),
        UserId::new("u1"),
        ChainId::new("eip155:1"),
        "mainnet".into(),
        "0xabc".into(),
        "idem-key".into(),
        ActorId::new("a1"),
        T0,
    );
    e.mark_challenged("hash".into(), "nonce".into(), T0 + TTL_MS, T0);
    store.put_enrollment(e.clone()).await;

    let mut active = e.clone();
    active.mark_active("evidence".into(), T0 + 1);
    // First CAS (Challenged -> Active) wins.
    assert!(
        store
            .compare_and_swap_enrollment_state("e1", EnrollmentState::Challenged, active.clone())
            .await
    );
    // Second CAS from Challenged loses (state is now Active).
    assert!(
        !store
            .compare_and_swap_enrollment_state("e1", EnrollmentState::Challenged, active)
            .await,
        "a CAS from Challenged must fail once the state has moved to Active"
    );
    // Unknown id never swaps.
    assert!(
        !store
            .compare_and_swap_enrollment_state("nope", EnrollmentState::Challenged, e)
            .await
    );
}

/// Sequential double-completion of the same challenge yields exactly one Active
/// binding and the second attempt fails closed (`NotChallengeable`) — the
/// single-use invariant the completion CAS protects.
#[tokio::test]
async fn second_completion_is_not_challengeable_single_binding() {
    let reg = registrar();
    let (sk, addr) = evm_keypair();
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr.clone(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .expect("initiate");
    let sig = evm_sign(&sk, &chal);
    let submit = SignedChallenge {
        challenge: chal,
        signature: sig,
        public_key_hex: None,
    };
    let first = reg
        .complete_registration(&enr.enrollment_id, submit.clone(), T0 + 1)
        .await
        .expect("first completion wins");
    assert_eq!(first.status, BindingStatus::Active);

    let err = reg
        .complete_registration(&enr.enrollment_id, submit, T0 + 2)
        .await
        .expect_err("second completion must fail closed");
    assert!(matches!(err, TrustError::NotChallengeable { .. }));
}

// ---- Normalization & idempotency (henrypark133 review: #1, #2) ----

/// EVM addresses differing only in case are the SAME physical account: they
/// must resume one ceremony (same idempotency key, same enrollment id, same
/// challenge digest) and produce one binding with a deterministic lowercase
/// `account_or_key` — not two parallel enrollment slots.
#[tokio::test]
async fn evm_address_case_is_normalized_to_one_ceremony_and_binding() {
    let reg = registrar();
    let (sk, addr) = evm_keypair(); // lowercase 0x form
    let upper = format!("0x{}", addr.trim_start_matches("0x").to_ascii_uppercase());
    assert_ne!(addr, upper, "test precondition: forms differ by case");

    let (enr1, chal1) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            upper.clone(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .expect("initiate upper");
    // The stored claim and challenge are canonicalized to lowercase.
    assert_eq!(enr1.claimed_account, addr);
    assert_eq!(chal1.claimed_account, addr);

    let (enr2, chal2) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr.clone(),
            ActorId::new("a1"),
            T0 + 5,
        )
        .await
        .expect("initiate lower");
    // Same physical account → same ceremony resumed, byte-identical challenge.
    assert_eq!(enr1.enrollment_id, enr2.enrollment_id);
    assert_eq!(chal1.digest(), chal2.digest());

    let sig = evm_sign(&sk, &chal2);
    let binding = reg
        .complete_registration(
            &enr2.enrollment_id,
            SignedChallenge {
                challenge: chal2,
                signature: sig,
                public_key_hex: None,
            },
            T0 + 6,
        )
        .await
        .expect("complete");
    assert_eq!(
        binding.account_or_key, addr,
        "binding is canonical lowercase"
    );
}

/// A real Solana wallet presents its pubkey as base58. Registration must accept
/// it (normalizing to canonical lowercase hex) and verify against the same key.
#[tokio::test]
async fn solana_base58_account_is_accepted_and_normalized() {
    let reg = registrar();
    let (sk, pk_hex) = ed_keypair(11);
    // What a Phantom/Solflare-style wallet would submit: base58 of the 32-byte key.
    let pk_bytes = sk.verifying_key().to_bytes();
    let base58 = bs58::encode(pk_bytes).into_string();
    assert_ne!(base58, pk_hex, "base58 and hex forms differ");

    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("solana:mainnet"),
            "mainnet".into(),
            base58.clone(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .expect("initiate base58");
    // Canonicalized to lowercase hex for storage / challenge / signer match.
    assert_eq!(enr.claimed_account, pk_hex);
    assert_eq!(chal.claimed_account, pk_hex);

    let sig = ed_sign(&sk, &chal);
    let binding = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: chal,
                signature: sig,
                public_key_hex: Some(pk_hex.clone()),
            },
            T0 + 1,
        )
        .await
        .expect("complete base58 solana");
    assert_eq!(binding.account_or_key, pk_hex);
}

/// base58 and hex submissions of the same Solana key resume one ceremony.
#[tokio::test]
async fn solana_base58_and_hex_share_one_idempotency_slot() {
    let reg = registrar();
    let (sk, pk_hex) = ed_keypair(12);
    let base58 = bs58::encode(sk.verifying_key().to_bytes()).into_string();

    let (enr_b58, chal_b58) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("solana:mainnet"),
            "mainnet".into(),
            base58,
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let (enr_hex, chal_hex) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("solana:mainnet"),
            "mainnet".into(),
            pk_hex,
            ActorId::new("a1"),
            T0 + 5,
        )
        .await
        .unwrap();
    assert_eq!(enr_b58.enrollment_id, enr_hex.enrollment_id);
    assert_eq!(chal_b58.digest(), chal_hex.digest());
}

/// Garbage Solana accounts (neither 32-byte hex nor 32-byte base58) fail closed.
#[tokio::test]
async fn solana_invalid_account_fails_closed() {
    let reg = registrar();
    let err = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("solana:mainnet"),
            "mainnet".into(),
            "not-a-key".into(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .expect_err("must reject garbage account");
    assert!(matches!(err, TrustError::Verification(_)));
}

/// The production CSPRNG nonce source yields fresh, well-formed (64-hex)
/// nonces; two draws differ with overwhelming probability.
#[test]
fn csprng_nonce_source_is_fresh_and_well_formed() {
    let src = CsprngNonceSource::new();
    let a = src.next_nonce_hex().expect("OS CSPRNG available in tests");
    let b = src.next_nonce_hex().expect("OS CSPRNG available in tests");
    assert_eq!(a.len(), 64, "32 bytes → 64 hex chars");
    assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b, "distinct draws");
}

// ---- #4055 review fixes: panic-safety, fail-closed input guards, coverage ----

/// A multi-byte UTF-8 `public_key_hex` of even byte length must be rejected
/// cleanly (`Verification`/`ProofInvalid`), not panic on a non-char-boundary
/// `&str` slice. The value is attacker-controlled and NOT committed in the
/// challenge digest, so the hash compare does not constrain it: it reaches the
/// hex decoder unchanged.
#[tokio::test]
async fn solana_completion_with_multibyte_public_key_does_not_panic() {
    let reg = registrar();
    let (sk, pk) = ed_keypair(21);
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("solana:mainnet"),
            "mainnet".into(),
            pk,
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = ed_sign(&sk, &chal);
    // "a\u{e9}b" is 4 bytes (even length, passes the byte-length gate) but the
    // char boundary falls inside the first 2-byte window.
    let err = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: chal,
                signature: sig,
                public_key_hex: Some("a\u{e9}b".to_string()),
            },
            T0 + 1,
        )
        .await
        .expect_err("multi-byte public_key_hex must fail closed, not panic");
    assert!(matches!(err, TrustError::Verification(_)));
}

/// NEAR analogue of the multi-byte panic guard: `decode_hex32` must not slice a
/// `&str` on a non-char-boundary. A 64-byte multi-byte value passes the
/// byte-length gate but splits a char.
#[tokio::test]
async fn near_completion_with_multibyte_public_key_does_not_panic() {
    let reg = registrar();
    let (sk, _pk) = ed_keypair(22);
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("near:mainnet"),
            "mainnet".into(),
            "carol.near".into(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = ed_sign(&sk, &chal);
    // "a" + 21 * "\u{20ac}" (3 bytes each) = 1 + 63 = 64 bytes: passes
    // `decode_hex32`'s length gate, but the 3-byte chars misalign the 2-byte
    // windows so a slice boundary falls inside a char.
    let multibyte = format!("a{}", "\u{20ac}".repeat(21));
    assert_eq!(multibyte.len(), 64, "test precondition: 64 bytes");
    let err = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: chal,
                signature: sig,
                public_key_hex: Some(multibyte),
            },
            T0 + 1,
        )
        .await
        .expect_err("multi-byte public_key_hex must fail closed, not panic");
    assert!(matches!(err, TrustError::Verification(_)));
}

/// Solana completion submitted with no `public_key_hex` hits the fail-closed
/// input guard (`verify_control` requires it for ed25519 families).
#[tokio::test]
async fn solana_completion_without_public_key_fails_closed() {
    let reg = registrar();
    let (sk, pk) = ed_keypair(23);
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("solana:mainnet"),
            "mainnet".into(),
            pk,
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = ed_sign(&sk, &chal);
    let err = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: chal,
                signature: sig,
                public_key_hex: None,
            },
            T0 + 1,
        )
        .await
        .expect_err("solana completion needs a public key");
    assert!(matches!(err, TrustError::Verification(_)));
}

/// NEAR analogue of the missing-public-key fail-closed guard.
#[tokio::test]
async fn near_completion_without_public_key_fails_closed() {
    let reg = registrar();
    let (sk, _pk) = ed_keypair(24);
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("near:mainnet"),
            "mainnet".into(),
            "dave.near".into(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = ed_sign(&sk, &chal);
    let err = reg
        .complete_registration(
            &enr.enrollment_id,
            SignedChallenge {
                challenge: chal,
                signature: sig,
                public_key_hex: None,
            },
            T0 + 1,
        )
        .await
        .expect_err("near completion needs an access-key public key");
    assert!(matches!(err, TrustError::Verification(_)));
}

/// An unsupported CAIP-2 namespace must fail closed at `initiate_registration`.
#[tokio::test]
async fn unsupported_chain_fails_closed_on_initiate() {
    let reg = registrar();
    let err = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("cosmos:cosmoshub-4"),
            "mainnet".into(),
            "cosmos1abc".into(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .expect_err("unsupported chain must fail closed");
    assert!(matches!(err, TrustError::UnsupportedChain(_)));
}

/// An unsupported namespace on the submitted challenge must also fail closed at
/// `complete_registration`. We build a `Challenged` enrollment for a supported
/// chain, then submit a challenge whose `chain_id` is unsupported. The hash
/// compare runs first, so the submitted challenge must match the stored hash;
/// we therefore stage a stored enrollment whose `challenge_hash` equals the
/// unsupported-chain challenge's digest.
#[tokio::test]
async fn unsupported_chain_fails_closed_on_complete() {
    let store = InMemoryTrustStore::new();
    let unsupported = TrustChallenge {
        tenant_id: TenantId::new("t1"),
        user_id: UserId::new("u1"),
        chain_id: ChainId::new("cosmos:cosmoshub-4"),
        network: "mainnet".into(),
        claimed_account: "cosmos1abc".into(),
        nonce_hex: format!("{:032x}", 1),
        expires_at_unix_ms: T0 + TTL_MS,
    };
    let mut enrollment = TrustEnrollment::pending(
        "e-unsupported".into(),
        TenantId::new("t1"),
        UserId::new("u1"),
        ChainId::new("cosmos:cosmoshub-4"),
        "mainnet".into(),
        "cosmos1abc".into(),
        "idem-unsupported".into(),
        ActorId::new("a1"),
        T0,
    );
    enrollment.mark_challenged(
        hex_lower(&unsupported.digest()),
        unsupported.nonce_hex.clone(),
        T0 + TTL_MS,
        T0,
    );
    store.put_enrollment(enrollment).await;
    let reg = TrustRegistrar::new(
        store,
        Box::new(SeqNonce::new()),
        AlwaysTrustNearAccessKeyVerifier,
        TTL_MS,
    );
    let err = reg
        .complete_registration(
            "e-unsupported",
            SignedChallenge {
                challenge: unsupported,
                signature: vec![0u8; 65],
                public_key_hex: None,
            },
            T0 + 1,
        )
        .await
        .expect_err("unsupported chain must fail closed on complete");
    assert!(matches!(err, TrustError::UnsupportedChain(_)));
}

/// Re-initiating past the challenge TTL must mint a FRESH ceremony (new
/// enrollment id, new nonce/digest) over the dead slot, not resume the expired
/// one — otherwise an expired challenge would silently extend its replay window.
#[tokio::test]
async fn reinitiate_after_expiry_mints_fresh_challenge() {
    let reg = registrar();
    let (_sk, addr) = evm_keypair();
    let (enr1, chal1) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr.clone(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    // Past the first challenge's TTL: the dead slot must be replaced, not resumed.
    let (enr2, chal2) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr,
            ActorId::new("a1"),
            T0 + TTL_MS + 1,
        )
        .await
        .unwrap();
    assert_ne!(
        enr1.enrollment_id, enr2.enrollment_id,
        "expired ceremony must mint a fresh enrollment"
    );
    assert_ne!(
        chal1.digest(),
        chal2.digest(),
        "fresh challenge must use a new nonce"
    );
    assert_eq!(enr2.state, EnrollmentState::Challenged);
    assert_eq!(enr2.expires_at_unix_ms, Some(T0 + TTL_MS + 1 + TTL_MS));
}

/// Re-initiating after the ceremony reached `Active` must mint a fresh
/// `Challenged` ceremony over the terminal slot, not hand back the Active one.
#[tokio::test]
async fn reinitiate_after_active_mints_fresh_ceremony() {
    let reg = registrar();
    let (sk, addr) = evm_keypair();
    let (enr1, chal1) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr.clone(),
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = evm_sign(&sk, &chal1);
    reg.complete_registration(
        &enr1.enrollment_id,
        SignedChallenge {
            challenge: chal1.clone(),
            signature: sig,
            public_key_hex: None,
        },
        T0 + 1,
    )
    .await
    .expect("first completes to Active");

    let (enr2, chal2) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr,
            ActorId::new("a1"),
            T0 + 2,
        )
        .await
        .unwrap();
    assert_ne!(
        enr1.enrollment_id, enr2.enrollment_id,
        "an Active ceremony must not be resumed; a fresh one is minted"
    );
    assert_ne!(chal1.digest(), chal2.digest());
    assert_eq!(enr2.state, EnrollmentState::Challenged);
}

/// Same-tenant cross-user isolation: a binding for user `u1` must NOT resolve
/// for user `u2` in the same tenant/chain/network (the per-user axis the threat
/// model calls out for idempotency-key hijack).
#[tokio::test]
async fn cross_user_lookup_is_isolated() {
    let reg = registrar();
    make_active_evm_binding(&reg, "t1").await; // binds user "u1"
    // Same tenant/chain/network, different user must NOT resolve.
    assert!(
        reg.lookup_active_binding(
            &TenantId::new("t1"),
            &UserId::new("u2"),
            &ChainId::new("eip155:1"),
            "mainnet",
            T0 + 2,
        )
        .await
        .is_none()
    );
    // The bound user still resolves.
    assert!(
        reg.lookup_active_binding(
            &TenantId::new("t1"),
            &UserId::new("u1"),
            &ChainId::new("eip155:1"),
            "mainnet",
            T0 + 2,
        )
        .await
        .is_some()
    );
}

/// A rejected completion must consume the ceremony fail-closed: the enrollment
/// transitions `Challenged -> Failed` via CAS, not left `Challenged`
/// (retryable). Asserted for both the expired-challenge and wrong-signer paths.
#[tokio::test]
async fn expired_completion_marks_enrollment_failed() {
    let reg = registrar();
    let (sk, addr) = evm_keypair();
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr,
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = evm_sign(&sk, &chal);
    reg.complete_registration(
        &enr.enrollment_id,
        SignedChallenge {
            challenge: chal,
            signature: sig,
            public_key_hex: None,
        },
        T0 + TTL_MS + 1, // past expiry
    )
    .await
    .expect_err("expired challenge rejected");
    let after = reg
        .store()
        .get_enrollment_by_id(&enr.enrollment_id)
        .await
        .expect("enrollment present");
    assert_eq!(
        after.state,
        EnrollmentState::Failed,
        "an expired completion must consume the ceremony (Failed), not leave it Challenged"
    );
}

#[tokio::test]
async fn wrong_signer_completion_marks_enrollment_failed() {
    let reg = registrar();
    let (_sk, addr) = evm_keypair();
    let other = EcSigningKey::from_bytes(&[9u8; 32].into()).unwrap();
    let (enr, chal) = reg
        .initiate_registration(
            TenantId::new("t1"),
            UserId::new("u1"),
            ChainId::new("eip155:1"),
            "mainnet".into(),
            addr,
            ActorId::new("a1"),
            T0,
        )
        .await
        .unwrap();
    let sig = evm_sign(&other, &chal);
    reg.complete_registration(
        &enr.enrollment_id,
        SignedChallenge {
            challenge: chal,
            signature: sig,
            public_key_hex: None,
        },
        T0 + 1,
    )
    .await
    .expect_err("wrong signer rejected");
    let after = reg
        .store()
        .get_enrollment_by_id(&enr.enrollment_id)
        .await
        .expect("enrollment present");
    assert_eq!(
        after.state,
        EnrollmentState::Failed,
        "a wrong-signer completion must consume the ceremony (Failed), not leave it Challenged"
    );
}

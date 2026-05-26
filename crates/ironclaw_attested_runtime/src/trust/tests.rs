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
    fn next_nonce_hex(&self) -> String {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("{n:032x}")
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
    active.mark_active(T0 + 1);
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

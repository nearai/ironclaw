//! Threat-matrix integration tests for the attested-signing reborn runtime
//! (PR10).
//!
//! These drive the REAL composition pieces — the [`RuntimeAttestedResumePort`]
//! through the actual `ironclaw_turns` resume path, and the
//! [`AttestedSignerContinuationDriver`] through the real `ironclaw_chain_signing`
//! custodial signer / `ironclaw_wallet_external` provider and the
//! `ironclaw_attestation` sealed-grant + ledger stores — rather than testing a
//! helper in isolation (CLAUDE.md "Test Through the Caller").
//!
//! Coverage maps to the threat matrix in
//! `docs/plans/2026-05-23-attested-signing-substrate.md`:
//!
//! * #1  sealed-grant replay rejected
//! * #3  caller-supplied hash rejected
//! * #5  EVM `from` spoof caught via ecrecover
//! * #6  broadcast retry blocked by the ledger
//! * #7  `Stuck -> InProgress` double-broadcast blocked (ledger-state-keyed)
//! * #16 LLM-loop never re-entered on resume (resume yields AttestedResolved,
//!   never Queued)
//! * #18 ship-gate refuses custodial mainnet without KMS

use std::sync::Arc;

use alloy_consensus::TxEip1559;
use alloy_primitives::{Address, Bytes, TxKind, U256};

use ironclaw_attestation::{
    AttestedSigningGrant, DecodedTransaction, GrantKey, InMemorySealedGrantStore,
    InMemorySigningLedger, RenderingSchemaVersion, SealedGrantStore, SigningLedger,
    SigningLedgerState,
};
use ironclaw_attested_runtime::{
    AttestedGateBinding, AttestedGateBindingStore, AttestedSignerContinuationDriver,
    ContinuationError, CustodialMainnetShipGate, InMemoryAttestedGateBindingStore,
    InMemoryResumeGuard, ProviderRegistry, ResumeGuard, RuntimeAttestedResumePort, SyncBindingRead,
    approved_tx_hash_ref_hex,
};
use ironclaw_chain_signing::{
    ChainKeyBinding, ChainKeyId, ChainSigningError, CustodialSigner, DenyFirstCustodyPolicy,
    KeyStore, SecretsKeyStore, evm,
};
use ironclaw_host_api::{InvocationId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_secrets::SecretsCrypto;
use ironclaw_signing_provider::{
    ActorId, ApprovedTxHash, ChainId, GateRef as SigningGateRef, KeyOrAccountId, ProviderId, RunId,
    ScopeId, SigningContext, SigningProof, TenantId as SigningTenantId, UserId as SigningUserId,
};
use ironclaw_turns::{
    ApprovedTxHashRef, AttestationClaimRef, AttestedResumePort, AttestedResumeRejection,
    AttestedResumeRequest, GateRef as TurnsGateRef,
};
use secrecy::SecretString;

// ── shared fixtures ──────────────────────────────────────────────────────

const GATE: &str = "gate:threat-matrix";
const DEV_TESTNET_CHAIN: &str = "eip155:11155111"; // sepolia (testnet)
const MASTER_KEY: &str = "0123456789abcdef0123456789ABCDEF";

fn owner_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("default").unwrap(),
        user_id: UserId::new("alice").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("bootstrap").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn signing_context(account_hex_no_prefix: &str) -> SigningContext {
    SigningContext {
        tenant: SigningTenantId::new("default"),
        user: SigningUserId::new("alice"),
        scope: ScopeId::new("scope-x"),
        actor: ActorId::new("actor-7"),
        run_id: RunId::new("run-42"),
        gate_ref: SigningGateRef::new(GATE),
        chain_id: ChainId::new(DEV_TESTNET_CHAIN),
        key_or_account_id: KeyOrAccountId::new(account_hex_no_prefix),
    }
}

/// Build a sample EIP-1559 transaction + its decoded form + the binding hash.
///
/// The approved hash is computed with the GATE-BOUND signer (`signer`) folded
/// in explicitly — the same account the test's `SigningContext.key_or_account_id`
/// carries — never the tx body. This mirrors the production WYSIWYS binding so
/// the driver's resume-time recompute (which uses `binding.context
/// .key_or_account_id`) reproduces the bound hash.
fn sample_evm(signer: &str) -> (TxEip1559, DecodedTransaction, ApprovedTxHash) {
    let tx = TxEip1559 {
        chain_id: 11155111,
        nonce: 7,
        gas_limit: 21_000,
        max_fee_per_gas: 30_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(Address::repeat_byte(0xbb)),
        value: U256::from(1_000u64),
        input: Bytes::new(),
        access_list: Default::default(),
    };
    let decoded = evm::decode_eip1559(&tx);
    let hash = ironclaw_chain_signing::recompute_approved_hash(
        &decoded,
        signer,
        RenderingSchemaVersion::CURRENT,
    )
    .expect("recompute approved hash in test");
    (tx, decoded, hash)
}

/// An EVM keystore bound to the address derived from `priv_bytes`, plus the
/// lowercase-hex (no `0x`) bound account string.
async fn keystore_with_evm_key(priv_bytes: &[u8; 32]) -> (Arc<SecretsKeyStore>, String) {
    let crypto = SecretsCrypto::new(SecretString::from(MASTER_KEY.to_string())).unwrap();
    let keystore = Arc::new(SecretsKeyStore::new(crypto));
    let key = k256::ecdsa::SigningKey::from_slice(priv_bytes).unwrap();
    let address = evm::address_of(&key);
    let addr_hex = hex::encode(address.as_slice());
    let binding = ChainKeyBinding {
        chain: ChainKeyId::new(DEV_TESTNET_CHAIN).expect("valid chain id in test"),
        public_address_hex: addr_hex.clone(),
        evm_chain_id: Some(11155111),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        // Hot-key custody: no KMS handle. The testnet ship-gate permits this;
        // mainnet would require `Some(..)`.
        kms_key_ref: None,
    };
    keystore
        .bind(&owner_scope(), binding, priv_bytes.to_vec())
        .await
        .unwrap();
    (keystore, addr_hex)
}

/// The concrete custodial driver type assembled by [`custodial_driver`].
type TestCustodialDriver = AttestedSignerContinuationDriver<
    ironclaw_reborn_noop::NoopBroadcaster,
    InMemorySigningLedger,
    CustodialSigner<SecretsKeyStore, InMemorySealedGrantStore, InMemorySigningLedger>,
>;

/// Assemble a custodial driver with shared grant + ledger stores. Returns the
/// driver, the shared grant store, the shared ledger, and the binding store.
fn custodial_driver(
    keystore: Arc<SecretsKeyStore>,
    bindings: Arc<InMemoryAttestedGateBindingStore>,
) -> (
    TestCustodialDriver,
    Arc<InMemorySealedGrantStore>,
    Arc<InMemorySigningLedger>,
) {
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    // Testnet chain => ship-gate permits hot-key custodial without a KMS.
    let ship_gate = CustodialMainnetShipGate::new(false).build_chain_ship_gate(None);
    let signer = Arc::new(CustodialSigner::new(
        Arc::clone(&keystore),
        Arc::clone(&grants),
        Arc::clone(&ledger),
        ship_gate,
        Arc::new(DenyFirstCustodyPolicy),
    ));
    let driver = AttestedSignerContinuationDriver::new(
        Arc::clone(&bindings) as Arc<dyn AttestedGateBindingStore>,
        ProviderRegistry::new(),
        signer,
        Arc::clone(&ledger),
        Arc::new(ironclaw_reborn_noop::NoopBroadcaster),
    );
    (driver, grants, ledger)
}

/// A local no-op broadcaster mirroring the composition crate's
/// `NoopBroadcaster` so the driver's ledger guard is exercised without network
/// I/O.
mod ironclaw_reborn_noop {
    use super::*;

    #[derive(Default)]
    pub struct NoopBroadcaster;

    #[async_trait::async_trait]
    impl ironclaw_attested_runtime::Broadcaster for NoopBroadcaster {
        fn submits(&self) -> bool {
            false
        }

        async fn broadcast(
            &self,
            _context: &SigningContext,
            _signed: &[u8],
        ) -> Result<ironclaw_attested_runtime::BroadcastOutcome, ContinuationError> {
            Ok(ironclaw_attested_runtime::BroadcastOutcome::NotBroadcast {
                reason: "test noop".to_string(),
            })
        }
    }
}

/// A broadcaster that REALLY submits (reports `submits() == true`) and counts
/// every submit, so a test can assert true broadcast idempotency: a replayed
/// continuation must produce ZERO additional submit calls, not merely a
/// returned ledger error.
mod recording {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    pub struct RecordingBroadcaster {
        pub calls: AtomicUsize,
    }

    impl RecordingBroadcaster {
        pub fn count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl ironclaw_attested_runtime::Broadcaster for RecordingBroadcaster {
        fn submits(&self) -> bool {
            true
        }

        async fn broadcast(
            &self,
            _context: &SigningContext,
            _signed: &[u8],
        ) -> Result<ironclaw_attested_runtime::BroadcastOutcome, ContinuationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ironclaw_attested_runtime::BroadcastOutcome::Submitted {
                tx_id: "recorded-tx".to_string(),
            })
        }
    }
}

/// A broadcaster that REALLY submits (reports `submits() == true`) and counts
/// every submit, but always fails the submit with a broadcast error (e.g. an
/// RPC timeout). Used to exercise broadcast-failure recovery (item C): the
/// ledger must move to a terminal recovery state and a retry must not
/// double-broadcast.
mod failing {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    pub struct FailingBroadcaster {
        pub calls: AtomicUsize,
    }

    impl FailingBroadcaster {
        pub fn count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl ironclaw_attested_runtime::Broadcaster for FailingBroadcaster {
        fn submits(&self) -> bool {
            true
        }

        async fn broadcast(
            &self,
            _context: &SigningContext,
            _signed: &[u8],
        ) -> Result<ironclaw_attested_runtime::BroadcastOutcome, ContinuationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(ContinuationError::Broadcast {
                reason: "rpc timeout after send".to_string(),
            })
        }
    }
}

/// The concrete custodial driver type backed by a [`recording::RecordingBroadcaster`].
type RecordingCustodialDriver = AttestedSignerContinuationDriver<
    recording::RecordingBroadcaster,
    InMemorySigningLedger,
    CustodialSigner<SecretsKeyStore, InMemorySealedGrantStore, InMemorySigningLedger>,
>;

/// The concrete custodial driver type backed by a [`failing::FailingBroadcaster`].
type FailingCustodialDriver = AttestedSignerContinuationDriver<
    failing::FailingBroadcaster,
    InMemorySigningLedger,
    CustodialSigner<SecretsKeyStore, InMemorySealedGrantStore, InMemorySigningLedger>,
>;

/// Assemble a custodial driver wired to a shared failing broadcaster so a test
/// can assert broadcast-failure recovery and that a retry does not
/// double-broadcast.
fn custodial_driver_failing(
    keystore: Arc<SecretsKeyStore>,
    bindings: Arc<InMemoryAttestedGateBindingStore>,
    broadcaster: Arc<failing::FailingBroadcaster>,
) -> (
    FailingCustodialDriver,
    Arc<InMemorySealedGrantStore>,
    Arc<InMemorySigningLedger>,
) {
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let ship_gate = CustodialMainnetShipGate::new(false).build_chain_ship_gate(None);
    let signer = Arc::new(CustodialSigner::new(
        Arc::clone(&keystore),
        Arc::clone(&grants),
        Arc::clone(&ledger),
        ship_gate,
        Arc::new(DenyFirstCustodyPolicy),
    ));
    let driver = AttestedSignerContinuationDriver::new(
        Arc::clone(&bindings) as Arc<dyn AttestedGateBindingStore>,
        ProviderRegistry::new(),
        signer,
        Arc::clone(&ledger),
        broadcaster,
    );
    (driver, grants, ledger)
}

/// Assemble a custodial driver wired to a shared recording broadcaster so a
/// test can assert how many real submits happened.
fn custodial_driver_recording(
    keystore: Arc<SecretsKeyStore>,
    bindings: Arc<InMemoryAttestedGateBindingStore>,
    broadcaster: Arc<recording::RecordingBroadcaster>,
) -> (
    RecordingCustodialDriver,
    Arc<InMemorySealedGrantStore>,
    Arc<InMemorySigningLedger>,
) {
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let ship_gate = CustodialMainnetShipGate::new(false).build_chain_ship_gate(None);
    let signer = Arc::new(CustodialSigner::new(
        Arc::clone(&keystore),
        Arc::clone(&grants),
        Arc::clone(&ledger),
        ship_gate,
        Arc::new(DenyFirstCustodyPolicy),
    ));
    let driver = AttestedSignerContinuationDriver::new(
        Arc::clone(&bindings) as Arc<dyn AttestedGateBindingStore>,
        ProviderRegistry::new(),
        signer,
        Arc::clone(&ledger),
        broadcaster,
    );
    (driver, grants, ledger)
}

async fn seal_grant(grants: &InMemorySealedGrantStore, ctx: &SigningContext, hash: ApprovedTxHash) {
    let key = GrantKey::from_context(ctx, hash);
    grants
        .seal(AttestedSigningGrant::seal(key, 0, None))
        .await
        .expect("seal");
}

async fn put_binding(
    bindings: &InMemoryAttestedGateBindingStore,
    ctx: &SigningContext,
    decoded: DecodedTransaction,
    hash: ApprovedTxHash,
) {
    put_binding_res(bindings, ctx, decoded, hash)
        .await
        .expect("binding insert succeeds");
}

async fn put_binding_res(
    bindings: &InMemoryAttestedGateBindingStore,
    ctx: &SigningContext,
    decoded: DecodedTransaction,
    hash: ApprovedTxHash,
) -> Result<(), ironclaw_attested_runtime::BindingError> {
    bindings
        .put(
            SigningGateRef::new(GATE),
            AttestedGateBinding {
                provider_id: ProviderId::Custodial,
                context: ctx.clone(),
                approved_tx_hash: hash,
                decoded,
                chain: ChainKeyId::new(DEV_TESTNET_CHAIN).expect("valid chain id in test"),
                scope: owner_scope(),
                schema_version: RenderingSchemaVersion::CURRENT,
            },
        )
        .await
}

// ── Threat #18: ship-gate refuses custodial mainnet without KMS ───────────

#[test]
fn threat_18_ship_gate_refuses_custodial_mainnet_without_kms() {
    // Opt-in TRUE but no KMS backend: mainnet must still be refused.
    let gate = CustodialMainnetShipGate::new(true).build_chain_ship_gate(None);
    let err = gate
        .authorize_chain("eip155:1")
        .expect_err("mainnet custodial must be refused without secure custody");
    assert!(matches!(err, ChainSigningError::ShipGateRefused { .. }));

    // Testnet is always allowed (hot-key dev signing).
    assert!(gate.authorize_chain(DEV_TESTNET_CHAIN).is_ok());

    // Opt-out (default) also refuses mainnet.
    let gate_off = CustodialMainnetShipGate::new(false).build_chain_ship_gate(None);
    assert!(gate_off.authorize_chain("eip155:1").is_err());
}

// ── Threat #1 + #6: grant replay & broadcast retry both fail closed ───────

#[tokio::test]
async fn threat_1_and_6_custodial_replay_and_broadcast_retry_blocked() {
    let priv_bytes = [0x11u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, hash) = sample_evm(&account);

    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let broadcaster = Arc::new(recording::RecordingBroadcaster::default());
    let (driver, grants, _ledger) = custodial_driver_recording(
        Arc::clone(&keystore),
        Arc::clone(&bindings),
        Arc::clone(&broadcaster),
    );
    seal_grant(&grants, &ctx, hash).await;
    put_binding(&bindings, &ctx, decoded, hash).await;

    let gate = SigningGateRef::new(GATE);
    let proof = SigningProof::WebAuthnAssertionProof(vec![]);

    // First continuation: signs the tx REBUILT FROM THE BINDING + really
    // broadcasts (one submit), advancing the ledger to BroadcastSubmitted.
    let outcome = driver
        .continue_after_resolved(&gate, &proof)
        .await
        .expect("first continuation succeeds");
    assert_eq!(outcome.ledger_state, SigningLedgerState::BroadcastSubmitted);
    assert_eq!(broadcaster.count(), 1, "exactly one real broadcast");

    // Second continuation of the SAME gate: the ledger row already exists and
    // is past Signed, so the deterministic continuation is refused (threats
    // #6/#7). The sealed grant was also already claimed (threat #1) — either
    // guard alone fails the replay closed.
    let err = driver
        .continue_after_resolved(&gate, &proof)
        .await
        .expect_err("replay/broadcast-retry must fail closed");
    assert!(
        matches!(
            err,
            ContinuationError::Ledger(_) | ContinuationError::LedgerRowExists { .. }
        ),
        "expected ledger guard rejection, got {err:?}"
    );
    // TRUE idempotency: the replay produced ZERO additional broadcast calls.
    assert_eq!(
        broadcaster.count(),
        1,
        "replay must not produce a second broadcast"
    );
}

// ── Threat #1 directly: a second grant claim is AlreadyClaimed ────────────

#[tokio::test]
async fn threat_1_sealed_grant_one_shot_claim() {
    let priv_bytes = [0x12u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, _decoded, hash) = sample_evm(&account);

    let grants = InMemorySealedGrantStore::new();
    seal_grant(&grants, &ctx, hash).await;
    let key = GrantKey::from_context(&ctx, hash);
    grants.claim(&key).await.expect("first claim wins");
    let err = grants.claim(&key).await.expect_err("second claim fails");
    assert_eq!(err, ironclaw_attestation::GrantError::AlreadyClaimed);
    let _ = keystore; // keep the bound key alive for parity with other cases
}

// ── Threat #3: caller-supplied hash rejected ──────────────────────────────
//
// Two layers, both fail-closed:
//   (a) the binding store refuses to PERSIST a binding whose approved hash does
//       not match its own decoded tx (immutable, validated write — fix #2); and
//   (b) the driver RE-CHECKS the hash from the decoded tx at sign time (defense
//       in depth, so a durable backend that somehow holds an inconsistent row
//       still fails closed — exercised in `driver_rechecks_inconsistent_binding`).

#[tokio::test]
async fn threat_3_inconsistent_binding_write_rejected() {
    let priv_bytes = [0x13u8; 32];
    let (_keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, real_hash) = sample_evm(&account);

    // A caller-asserted hash that does not match the decoded tx.
    let bogus_hash = ApprovedTxHash::from_bytes([0x99u8; 32]);
    assert_ne!(bogus_hash, real_hash);

    let bindings = InMemoryAttestedGateBindingStore::new();
    let err = put_binding_res(&bindings, &ctx, decoded, bogus_hash)
        .await
        .expect_err("inconsistent binding write must be rejected");
    assert_eq!(
        err,
        ironclaw_attested_runtime::BindingError::ApprovedHashMismatch
    );
}

/// Driver-level defense in depth (fix #1 / #3): even if a backend somehow holds
/// a binding whose stored hash diverges from its decoded tx, the driver
/// re-checks at sign time and fails closed BEFORE any key use. Uses a tiny
/// store mock that returns an inconsistent binding (the validated in-memory
/// store would never persist one).
#[tokio::test]
async fn driver_rechecks_inconsistent_binding() {
    use async_trait::async_trait;
    use ironclaw_attested_runtime::AttestedGateBinding;

    struct InconsistentStore(AttestedGateBinding);
    impl SyncBindingRead for InconsistentStore {
        fn get_sync(&self, _gate_ref: &SigningGateRef) -> Option<AttestedGateBinding> {
            Some(self.0.clone())
        }
    }
    #[async_trait]
    impl AttestedGateBindingStore for InconsistentStore {
        async fn put(
            &self,
            _gate_ref: SigningGateRef,
            _binding: AttestedGateBinding,
        ) -> Result<(), ironclaw_attested_runtime::BindingError> {
            Ok(())
        }
        async fn get(&self, _gate_ref: &SigningGateRef) -> Option<AttestedGateBinding> {
            Some(self.0.clone())
        }
    }

    let priv_bytes = [0x13u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, real_hash) = sample_evm(&account);
    let bogus_hash = ApprovedTxHash::from_bytes([0x99u8; 32]);
    assert_ne!(bogus_hash, real_hash);

    let inconsistent = AttestedGateBinding {
        provider_id: ProviderId::Custodial,
        context: ctx.clone(),
        approved_tx_hash: bogus_hash, // diverges from `decoded`
        decoded,
        chain: ChainKeyId::new(DEV_TESTNET_CHAIN).expect("valid chain id in test"),
        scope: owner_scope(),
        schema_version: RenderingSchemaVersion::CURRENT,
    };
    let store: Arc<dyn AttestedGateBindingStore> = Arc::new(InconsistentStore(inconsistent));

    let grants = Arc::new(InMemorySealedGrantStore::new());
    seal_grant(&grants, &ctx, bogus_hash).await;
    let ledger = Arc::new(InMemorySigningLedger::new());
    let ship_gate = CustodialMainnetShipGate::new(false).build_chain_ship_gate(None);
    let signer = Arc::new(CustodialSigner::new(
        Arc::clone(&keystore),
        Arc::clone(&grants),
        Arc::clone(&ledger),
        ship_gate,
        Arc::new(DenyFirstCustodyPolicy),
    ));
    let driver = AttestedSignerContinuationDriver::new(
        store,
        ProviderRegistry::new(),
        signer,
        ledger,
        Arc::new(ironclaw_reborn_noop::NoopBroadcaster),
    );

    let gate = SigningGateRef::new(GATE);
    let err = driver
        .continue_after_resolved(&gate, &SigningProof::WebAuthnAssertionProof(vec![]))
        .await
        .expect_err("driver must re-check and reject inconsistent binding");
    assert!(
        matches!(err, ContinuationError::ApprovedHashMismatch),
        "expected approved-hash mismatch, got {err:?}"
    );
}

// Note: the "approve-A / sign-B" caller-tx WYSIWYS threat is now structurally
// impossible and so has no dedicated test here: the driver NEVER accepts a
// caller-supplied signable transaction — it reconstructs the signable purely
// from the authoritative `binding.decoded` (the same decoded tx the approved
// hash was computed over). The equivalent attack surface (a tampered binding)
// is covered by `threat_3_inconsistent_binding_write_rejected` and
// `driver_rechecks_inconsistent_binding`.

// ── Threat #5: EVM `from` spoof caught via ecrecover binding ──────────────

#[tokio::test]
async fn threat_5_evm_from_spoof_caught_via_ecrecover() {
    // Bind the keystore account to an address that is NOT the address of the
    // private key actually stored. The custodial signer recovers the signer
    // from the signature (ecrecover) and compares it to the bound address; a
    // mismatch fails closed. We construct this by binding a wrong public
    // address against the real private key.
    let priv_bytes = [0x14u8; 32];
    let crypto = SecretsCrypto::new(SecretString::from(MASTER_KEY.to_string())).unwrap();
    let keystore = Arc::new(SecretsKeyStore::new(crypto));
    // Wrong bound address (all 0xCD), not the address of priv_bytes.
    let wrong_addr_hex = hex::encode([0xCDu8; 20]);
    let binding = ChainKeyBinding {
        chain: ChainKeyId::new(DEV_TESTNET_CHAIN).expect("valid chain id in test"),
        public_address_hex: wrong_addr_hex.clone(),
        evm_chain_id: Some(11155111),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        kms_key_ref: None,
    };
    keystore
        .bind(&owner_scope(), binding, priv_bytes.to_vec())
        .await
        .unwrap();

    let ctx = signing_context(&wrong_addr_hex);
    let (_tx, decoded, hash) = sample_evm(&wrong_addr_hex);
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let (driver, grants, _ledger) = custodial_driver(Arc::clone(&keystore), Arc::clone(&bindings));
    seal_grant(&grants, &ctx, hash).await;
    put_binding(&bindings, &ctx, decoded, hash).await;

    let gate = SigningGateRef::new(GATE);
    let err = driver
        .continue_after_resolved(&gate, &SigningProof::WebAuthnAssertionProof(vec![]))
        .await
        .expect_err("ecrecover binding mismatch must fail closed");
    assert!(
        matches!(
            err,
            ContinuationError::ChainSigning(ChainSigningError::SignerMismatch)
        ),
        "expected ecrecover SignerMismatch, got {err:?}"
    );
}

// ── Threat #7: Stuck->InProgress double-broadcast blocked (ledger-keyed) ──

#[tokio::test]
async fn threat_7_double_broadcast_blocked_by_ledger_state() {
    // Simulate a job that already broadcast: the ledger row for this gate_ref
    // is at BroadcastSubmitted. A recovery worker re-driving the continuation
    // must be refused — the guard is keyed on LEDGER state, not job state.
    let priv_bytes = [0x15u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, hash) = sample_evm(&account);

    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let (driver, grants, ledger) = custodial_driver(Arc::clone(&keystore), Arc::clone(&bindings));
    seal_grant(&grants, &ctx, hash).await;
    put_binding(&bindings, &ctx, decoded, hash).await;

    let gate = SigningGateRef::new(GATE);
    // Pre-advance the ledger to BroadcastSubmitted, as if a prior attempt
    // already broadcast.
    ledger.create(&gate).await.unwrap();
    ledger
        .advance(&gate, SigningLedgerState::Signing)
        .await
        .unwrap();
    ledger
        .advance(&gate, SigningLedgerState::Signed)
        .await
        .unwrap();
    ledger
        .advance(&gate, SigningLedgerState::BroadcastSubmitted)
        .await
        .unwrap();

    // The recovery re-drive must be refused (the create fails AlreadyExists and
    // the row is already broadcast).
    let err = driver
        .continue_after_resolved(&gate, &SigningProof::WebAuthnAssertionProof(vec![]))
        .await
        .expect_err("double-broadcast after recovery must fail closed");
    assert!(
        matches!(
            err,
            ContinuationError::Ledger(_) | ContinuationError::LedgerRowExists { .. }
        ),
        "expected ledger guard rejection, got {err:?}"
    );
    // Ledger never regressed out of BroadcastSubmitted.
    assert_eq!(
        ledger.state(&gate).await.unwrap(),
        SigningLedgerState::BroadcastSubmitted
    );
}

// ── Item C: broadcast-failure recovery (ledger -> Unknown, no double-cast) ──

/// A broadcast error must NOT leave the ledger stuck. The row moves to the
/// `Unknown` terminal (genuinely unknown whether the tx landed) and the driver
/// surfaces a `Broadcast` error carrying the evidence.
#[tokio::test]
async fn broadcast_error_moves_ledger_to_unknown_not_stuck() {
    let priv_bytes = [0x21u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, hash) = sample_evm(&account);

    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let broadcaster = Arc::new(failing::FailingBroadcaster::default());
    let (driver, grants, ledger) = custodial_driver_failing(
        Arc::clone(&keystore),
        Arc::clone(&bindings),
        Arc::clone(&broadcaster),
    );
    seal_grant(&grants, &ctx, hash).await;
    put_binding(&bindings, &ctx, decoded, hash).await;

    let gate = SigningGateRef::new(GATE);
    let proof = SigningProof::WebAuthnAssertionProof(vec![]);

    let err = driver
        .continue_after_resolved(&gate, &proof)
        .await
        .expect_err("a broadcast error must surface, not silently succeed");
    assert!(
        matches!(err, ContinuationError::Broadcast { .. }),
        "expected a Broadcast error, got {err:?}"
    );
    // Exactly one submit attempt was made.
    assert_eq!(broadcaster.count(), 1, "exactly one submit attempt");
    // The ledger is at the Unknown terminal — NOT stuck at Signing/Signed/
    // BroadcastSubmitted.
    let state = ledger.state(&gate).await.unwrap();
    assert_eq!(
        state,
        SigningLedgerState::Unknown,
        "failed broadcast must move the ledger to the Unknown terminal"
    );
    assert!(state.is_terminal(), "Unknown is terminal");
}

/// A retried continuation after a broadcast failure must NOT double-broadcast.
/// The ledger row already exists (now at the `Unknown` terminal), so the
/// one-shot `create` fails closed and the broadcaster is never called again.
#[tokio::test]
async fn retry_after_broadcast_failure_does_not_double_broadcast() {
    let priv_bytes = [0x22u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, hash) = sample_evm(&account);

    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let broadcaster = Arc::new(failing::FailingBroadcaster::default());
    let (driver, grants, ledger) = custodial_driver_failing(
        Arc::clone(&keystore),
        Arc::clone(&bindings),
        Arc::clone(&broadcaster),
    );
    seal_grant(&grants, &ctx, hash).await;
    put_binding(&bindings, &ctx, decoded, hash).await;

    let gate = SigningGateRef::new(GATE);
    let proof = SigningProof::WebAuthnAssertionProof(vec![]);

    // First attempt: broadcast fails -> ledger moves to Unknown.
    let _ = driver
        .continue_after_resolved(&gate, &proof)
        .await
        .expect_err("first attempt fails the broadcast");
    assert_eq!(broadcaster.count(), 1, "one submit attempt so far");
    assert_eq!(
        ledger.state(&gate).await.unwrap(),
        SigningLedgerState::Unknown
    );

    // Retry / recovery re-drive: the one-shot ledger row already exists, so the
    // continuation is refused fail-closed and the broadcaster is NOT called
    // again. Recovery from `Unknown` requires explicit out-of-band re-approval,
    // never an automatic re-broadcast.
    let err = driver
        .continue_after_resolved(&gate, &proof)
        .await
        .expect_err("retry after a failed broadcast must fail closed");
    assert!(
        matches!(
            err,
            ContinuationError::Ledger(_) | ContinuationError::LedgerRowExists { .. }
        ),
        "expected ledger guard rejection on retry, got {err:?}"
    );
    assert_eq!(
        broadcaster.count(),
        1,
        "retry must NOT produce a second broadcast"
    );
    // The terminal state is preserved (no regression out of Unknown).
    assert_eq!(
        ledger.state(&gate).await.unwrap(),
        SigningLedgerState::Unknown
    );
}

/// A confirmed submission advances the ledger to `BroadcastSubmitted` and
/// reports a `Submitted` disposition with the real tx id (success contract).
#[tokio::test]
async fn confirmed_submission_reaches_broadcast_submitted() {
    let priv_bytes = [0x23u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, hash) = sample_evm(&account);

    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let broadcaster = Arc::new(recording::RecordingBroadcaster::default());
    let (driver, grants, ledger) = custodial_driver_recording(
        Arc::clone(&keystore),
        Arc::clone(&bindings),
        Arc::clone(&broadcaster),
    );
    seal_grant(&grants, &ctx, hash).await;
    put_binding(&bindings, &ctx, decoded, hash).await;

    let gate = SigningGateRef::new(GATE);
    let proof = SigningProof::WebAuthnAssertionProof(vec![]);

    let outcome = driver
        .continue_after_resolved(&gate, &proof)
        .await
        .expect("confirmed submission succeeds");
    assert_eq!(outcome.ledger_state, SigningLedgerState::BroadcastSubmitted);
    assert_eq!(
        outcome.broadcast,
        ironclaw_attested_runtime::BroadcastDisposition::Submitted {
            tx_id: "recorded-tx".to_string()
        }
    );
    assert_eq!(broadcaster.count(), 1);
    assert_eq!(
        ledger.state(&gate).await.unwrap(),
        SigningLedgerState::BroadcastSubmitted
    );
}

// ── Threats #1/#16 via the resume PORT (drives the turns resume boundary) ──

#[test]
fn threat_16_resume_port_validates_then_one_shot_no_loop_reentry() {
    // The port runs synchronously inside the turn store's resume critical
    // section: it re-checks the bound hash and claims a one-shot resume guard.
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let resume_guard: Arc<dyn ResumeGuard> = Arc::new(InMemoryResumeGuard::new());
    let port = RuntimeAttestedResumePort::new(
        Arc::clone(&bindings) as Arc<dyn SyncBindingRead>,
        Arc::clone(&resume_guard),
    );

    let ctx = signing_context(&hex::encode([0xAAu8; 20]));
    let (_tx, decoded, hash) = sample_evm(&hex::encode([0xAAu8; 20]));
    // Persist the authoritative binding (as PR11 ingress would on raising).
    bindings.get_sync(&SigningGateRef::new(GATE)); // no-op read
    // SAFETY: synchronous put via the sync helper-equivalent (use blocking put
    // through a tiny runtime since put is async).
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(put_binding(&bindings, &ctx, decoded, hash));

    let hash_ref = approved_tx_hash_ref_hex(hash.as_bytes());
    let gate = TurnsGateRef::new(GATE).unwrap();
    let attestation = AttestationClaimRef::new(hash_ref.clone()).unwrap();
    let expected = ApprovedTxHashRef::new(hash_ref).unwrap();

    // First resume verifies (binding matches; guard claimed).
    port.verify_attested_resume(AttestedResumeRequest {
        gate_ref: &gate,
        attestation: &attestation,
        expected_tx_hash: &expected,
    })
    .expect("first attested resume verifies");

    // Replay of the same gate is refused one-shot (threat #1 at the resume
    // boundary). The turn would already be AttestedResolved, never re-queued
    // onto the agent loop (threat #16) — the port only ever returns Ok once.
    let err = port
        .verify_attested_resume(AttestedResumeRequest {
            gate_ref: &gate,
            attestation: &attestation,
            expected_tx_hash: &expected,
        })
        .expect_err("replayed resume must fail closed");
    assert_eq!(err, AttestedResumeRejection::EvidenceRejected);
}

// ── Threat #3 at the resume boundary: caller-supplied hash on resume ──────

#[test]
fn threat_3_resume_port_rejects_mismatched_expected_hash() {
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let resume_guard: Arc<dyn ResumeGuard> = Arc::new(InMemoryResumeGuard::new());
    let port = RuntimeAttestedResumePort::new(
        Arc::clone(&bindings) as Arc<dyn SyncBindingRead>,
        resume_guard,
    );

    let ctx = signing_context(&hex::encode([0xABu8; 20]));
    let (_tx, decoded, hash) = sample_evm(&hex::encode([0xABu8; 20]));
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(put_binding(&bindings, &ctx, decoded, hash));

    let gate = TurnsGateRef::new(GATE).unwrap();
    // A caller-supplied expected hash that does NOT match the bound hash.
    let bogus = ApprovedTxHashRef::new("00".repeat(32)).unwrap();
    let attestation = AttestationClaimRef::new(approved_tx_hash_ref_hex(hash.as_bytes())).unwrap();
    let err = port
        .verify_attested_resume(AttestedResumeRequest {
            gate_ref: &gate,
            attestation: &attestation,
            expected_tx_hash: &bogus,
        })
        .expect_err("mismatched expected hash must be rejected");
    assert_eq!(err, AttestedResumeRejection::BindingMismatch);
}

// ── Fix #2: authoritative binding writes are immutable + validated ────────

#[tokio::test]
async fn binding_write_is_insert_only_duplicate_rejected() {
    let priv_bytes = [0x21u8; 32];
    let (_keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, hash) = sample_evm(&account);

    let bindings = InMemoryAttestedGateBindingStore::new();
    put_binding_res(&bindings, &ctx, decoded.clone(), hash)
        .await
        .expect("first write wins");
    // A second write for the same gate_ref must fail closed — bindings are
    // immutable, so a later write can never mutate the trusted binding.
    let err = put_binding_res(&bindings, &ctx, decoded, hash)
        .await
        .expect_err("duplicate write must be rejected");
    assert_eq!(err, ironclaw_attested_runtime::BindingError::AlreadyExists);
}

#[tokio::test]
async fn binding_write_rejects_key_context_gate_ref_mismatch() {
    use ironclaw_attested_runtime::{AttestedGateBinding, validate_binding};

    let priv_bytes = [0x22u8; 32];
    let (_keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    // Context's gate_ref is GATE, but we will key the binding under a DIFFERENT
    // gate_ref. The store must reject it.
    let ctx = signing_context(&account);
    let (_tx, decoded, hash) = sample_evm(&account);
    let binding = AttestedGateBinding {
        provider_id: ProviderId::Custodial,
        context: ctx,
        approved_tx_hash: hash,
        decoded,
        chain: ChainKeyId::new(DEV_TESTNET_CHAIN).expect("valid chain id in test"),
        scope: owner_scope(),
        schema_version: RenderingSchemaVersion::CURRENT,
    };
    let wrong_key = SigningGateRef::new("gate:some-other-gate");
    let err = validate_binding(&wrong_key, &binding)
        .expect_err("key/context gate_ref mismatch must be rejected");
    assert_eq!(
        err,
        ironclaw_attested_runtime::BindingError::GateRefMismatch
    );
}

#[tokio::test]
async fn binding_write_rejects_chain_mismatch() {
    use ironclaw_attested_runtime::AttestedGateBinding;

    let priv_bytes = [0x23u8; 32];
    let (_keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, hash) = sample_evm(&account);

    // The decoded tx is on eip155:11155111 (testnet); bind a MAINNET chain. The
    // store must reject this so a testnet/mainnet smuggle never persists.
    let binding = AttestedGateBinding {
        provider_id: ProviderId::Custodial,
        context: ctx,
        approved_tx_hash: hash,
        decoded,
        chain: ChainKeyId::new("eip155:1").expect("valid chain id in test"),
        scope: owner_scope(),
        schema_version: RenderingSchemaVersion::CURRENT,
    };
    let bindings = InMemoryAttestedGateBindingStore::new();
    let err = bindings
        .put(SigningGateRef::new(GATE), binding)
        .await
        .expect_err("chain mismatch must be rejected");
    assert_eq!(err, ironclaw_attested_runtime::BindingError::ChainMismatch);
}

// ── External-wallet continuation path (drives `continue_external_wallet`) ──

/// A mock external-wallet [`SigningProvider`] whose `verify_resume` outcome is
/// configurable, so the driver's external-wallet continuation path can be
/// exercised without a real wallet ceremony.
mod ext_provider {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_signing_provider::{
        DecodedTransaction, InitiationOutcome, RenderedTx, SigningProvider, SigningProviderError,
        TrustModel, VerifiedProof,
    };

    pub struct MockExternalProvider {
        pub id: ProviderId,
        /// When `true`, `verify_resume` returns `Ok`; when `false` it rejects.
        pub verifies: bool,
    }

    #[async_trait]
    impl SigningProvider for MockExternalProvider {
        fn provider_id(&self) -> ProviderId {
            self.id
        }
        fn trust_model(&self) -> TrustModel {
            TrustModel::ExternalWallet
        }
        async fn initiate(
            &self,
            _context: &SigningContext,
            _decoded: &DecodedTransaction,
            _rendered: &RenderedTx,
            _approved_tx_hash: &ApprovedTxHash,
        ) -> Result<InitiationOutcome, SigningProviderError> {
            Ok(InitiationOutcome::AwaitingUserAction { directive: vec![] })
        }
        async fn verify_resume(
            &self,
            _context: &SigningContext,
            approved_tx_hash: &ApprovedTxHash,
            proof: &SigningProof,
        ) -> Result<VerifiedProof, SigningProviderError> {
            if self.verifies {
                Ok(VerifiedProof::new(
                    self.id,
                    *approved_tx_hash,
                    proof.clone(),
                ))
            } else {
                Err(SigningProviderError::ProofInvalid {
                    reason: "mock external provider rejects".to_string(),
                })
            }
        }
    }
}

/// Build an external-wallet driver bound to `provider_id`, with a mock provider
/// whose verification outcome is `verifies`. Returns the driver, the shared
/// grant store, the ledger, and the recording broadcaster.
async fn external_wallet_driver(
    provider_id: ProviderId,
    verifies: bool,
) -> (
    RecordingExternalDriver,
    Arc<InMemoryAttestedGateBindingStore>,
    Arc<InMemorySealedGrantStore>,
    Arc<InMemorySigningLedger>,
    Arc<recording::RecordingBroadcaster>,
) {
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let broadcaster = Arc::new(recording::RecordingBroadcaster::default());

    let registry =
        ProviderRegistry::new().with_provider(Arc::new(ext_provider::MockExternalProvider {
            id: provider_id,
            verifies,
        }));

    // A custodial signer is still required by the driver constructor even though
    // the external-wallet path never reaches it.
    let crypto = SecretsCrypto::new(SecretString::from(MASTER_KEY.to_string())).unwrap();
    let keystore = Arc::new(SecretsKeyStore::new(crypto));
    let ship_gate = CustodialMainnetShipGate::new(false).build_chain_ship_gate(None);
    let signer = Arc::new(CustodialSigner::new(
        Arc::clone(&keystore),
        Arc::clone(&grants),
        Arc::clone(&ledger),
        ship_gate,
        Arc::new(DenyFirstCustodyPolicy),
    ));
    let driver = AttestedSignerContinuationDriver::new(
        Arc::clone(&bindings) as Arc<dyn AttestedGateBindingStore>,
        registry,
        signer,
        Arc::clone(&ledger),
        Arc::clone(&broadcaster),
    );
    (driver, bindings, grants, ledger, broadcaster)
}

type RecordingExternalDriver = AttestedSignerContinuationDriver<
    recording::RecordingBroadcaster,
    InMemorySigningLedger,
    CustodialSigner<SecretsKeyStore, InMemorySealedGrantStore, InMemorySigningLedger>,
>;

async fn put_external_binding(
    bindings: &InMemoryAttestedGateBindingStore,
    provider_id: ProviderId,
    ctx: &SigningContext,
    decoded: DecodedTransaction,
    hash: ApprovedTxHash,
) {
    bindings
        .put(
            SigningGateRef::new(GATE),
            AttestedGateBinding {
                provider_id,
                context: ctx.clone(),
                approved_tx_hash: hash,
                decoded,
                chain: ChainKeyId::new(DEV_TESTNET_CHAIN).expect("valid chain id in test"),
                scope: owner_scope(),
                schema_version: RenderingSchemaVersion::CURRENT,
            },
        )
        .await
        .expect("external binding insert succeeds");
}

/// Verify-before-resume: when the external-wallet provider REJECTS the proof,
/// the ledger must NOT be stranded at `Signing`. Because the continuation is
/// one-shot per gate_ref, a row left at the in-flight `Signing` state could
/// never be re-entered and would be stuck permanently. The split driver
/// verifies + claims the grant BEFORE creating the ledger row, so a rejected
/// proof leaves NO ledger row at all (cleaner than a row stranded at any
/// non-terminal state), and the broadcaster is never called. A follow-up VALID
/// proof for the same gate is therefore still drivable.
#[tokio::test]
async fn external_wallet_verify_failure_does_not_strand_ledger_at_signing() {
    let ctx = signing_context(&hex::encode([0x31u8; 20]));
    let (_tx, decoded, hash) = sample_evm(&hex::encode([0x31u8; 20]));
    let (driver, bindings, grants, ledger, broadcaster) =
        external_wallet_driver(ProviderId::Injected, /* verifies */ false).await;
    seal_grant(&grants, &ctx, hash).await;
    put_external_binding(&bindings, ProviderId::Injected, &ctx, decoded, hash).await;

    let gate = SigningGateRef::new(GATE);
    let proof = SigningProof::InjectedProof(vec![1, 2, 3]);
    let err = driver
        .continue_after_resolved(&gate, &proof)
        .await
        .expect_err("a rejected external-wallet proof must fail closed");
    assert!(
        matches!(err, ContinuationError::ProofRejected(_)),
        "expected ProofRejected, got {err:?}"
    );
    // Verify-before-resume: a rejected proof must leave NO ledger row at all
    // (the row is only created after verify + grant claim succeed), so it can
    // never be stranded at an in-flight state.
    assert_eq!(
        ledger.state(&gate).await,
        Err(ironclaw_attestation::LedgerError::NotFound),
        "rejected proof must not create a ledger row at all"
    );
    assert_eq!(broadcaster.count(), 0, "broadcaster must not be called");
}

/// The happy external-wallet path: a verifying provider advances the ledger
/// through `Signing -> Signed` and broadcasts the wallet-signed bytes.
#[tokio::test]
async fn external_wallet_verify_success_signs_and_broadcasts() {
    let ctx = signing_context(&hex::encode([0x32u8; 20]));
    let (_tx, decoded, hash) = sample_evm(&hex::encode([0x32u8; 20]));
    let (driver, bindings, grants, ledger, broadcaster) =
        external_wallet_driver(ProviderId::Injected, /* verifies */ true).await;
    seal_grant(&grants, &ctx, hash).await;
    put_external_binding(&bindings, ProviderId::Injected, &ctx, decoded, hash).await;

    let gate = SigningGateRef::new(GATE);
    let proof = SigningProof::InjectedProof(vec![9, 9, 9]);
    let outcome = driver
        .continue_after_resolved(&gate, &proof)
        .await
        .expect("verifying external-wallet proof must continue");
    assert_eq!(outcome.ledger_state, SigningLedgerState::BroadcastSubmitted);
    assert_eq!(broadcaster.count(), 1);
    assert_eq!(
        ledger.state(&gate).await.unwrap(),
        SigningLedgerState::BroadcastSubmitted
    );
}

/// A resolved gate bound to a provider that is NOT registered fails closed with
/// `ProviderMismatch` BEFORE the ledger advances past `Approved`.
#[tokio::test]
async fn external_wallet_unregistered_provider_is_provider_mismatch() {
    let ctx = signing_context(&hex::encode([0x33u8; 20]));
    let (_tx, decoded, hash) = sample_evm(&hex::encode([0x33u8; 20]));
    // Register Injected, but bind WalletConnect -> mismatch.
    let (driver, bindings, grants, ledger, broadcaster) =
        external_wallet_driver(ProviderId::Injected, true).await;
    seal_grant(&grants, &ctx, hash).await;
    put_external_binding(&bindings, ProviderId::WalletConnect, &ctx, decoded, hash).await;

    let gate = SigningGateRef::new(GATE);
    let err = driver
        .continue_after_resolved(&gate, &SigningProof::InjectedProof(vec![]))
        .await
        .expect_err("unregistered provider must fail closed");
    assert!(
        matches!(err, ContinuationError::ProviderMismatch { .. }),
        "expected ProviderMismatch, got {err:?}"
    );
    // Verify-before-resume: a provider mismatch is detected before any ledger
    // row is created, so no row exists.
    assert_eq!(
        ledger.state(&gate).await,
        Err(ironclaw_attestation::LedgerError::NotFound),
        "provider mismatch must not create a ledger row"
    );
    assert_eq!(broadcaster.count(), 0);
}

// ── Tests/Medium: driver-level BindingChainMismatch re-check ──────────────

/// The custodial continuation re-checks that the binding's authoritative chain
/// matches the chain its OWN decoded tx encodes (testnet-chain / mainnet-tx
/// smuggle defense). A divergence fails closed with `BindingChainMismatch`. We
/// drive this through a store mock that returns a binding whose `chain` differs
/// from its decoded tx's network (the validated store would never persist one).
#[tokio::test]
async fn driver_rejects_binding_chain_mismatch() {
    use async_trait::async_trait;

    struct ChainMismatchStore(AttestedGateBinding);
    impl SyncBindingRead for ChainMismatchStore {
        fn get_sync(&self, _gate_ref: &SigningGateRef) -> Option<AttestedGateBinding> {
            Some(self.0.clone())
        }
    }
    #[async_trait]
    impl AttestedGateBindingStore for ChainMismatchStore {
        async fn put(
            &self,
            _gate_ref: SigningGateRef,
            _binding: AttestedGateBinding,
        ) -> Result<(), ironclaw_attested_runtime::BindingError> {
            Ok(())
        }
        async fn get(&self, _gate_ref: &SigningGateRef) -> Option<AttestedGateBinding> {
            Some(self.0.clone())
        }
    }

    let priv_bytes = [0x41u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, hash) = sample_evm(&account); // decoded encodes eip155:11155111

    // Binding's authoritative chain is MAINNET while the decoded tx is testnet:
    // the driver's pre-flight network identity check must reject this. The hash
    // is recomputed from `decoded`, so we keep it consistent with `decoded`.
    let mismatched = AttestedGateBinding {
        provider_id: ProviderId::Custodial,
        context: ctx.clone(),
        approved_tx_hash: hash,
        decoded,
        chain: ChainKeyId::new("eip155:1").expect("valid chain id in test"),
        scope: owner_scope(),
        schema_version: RenderingSchemaVersion::CURRENT,
    };
    let store: Arc<dyn AttestedGateBindingStore> = Arc::new(ChainMismatchStore(mismatched));

    let grants = Arc::new(InMemorySealedGrantStore::new());
    seal_grant(&grants, &ctx, hash).await;
    let ledger = Arc::new(InMemorySigningLedger::new());
    let ship_gate = CustodialMainnetShipGate::new(false).build_chain_ship_gate(None);
    let signer = Arc::new(CustodialSigner::new(
        Arc::clone(&keystore),
        Arc::clone(&grants),
        Arc::clone(&ledger),
        ship_gate,
        Arc::new(DenyFirstCustodyPolicy),
    ));
    let driver = AttestedSignerContinuationDriver::new(
        store,
        ProviderRegistry::new(),
        signer,
        ledger,
        Arc::new(ironclaw_reborn_noop::NoopBroadcaster),
    );

    let gate = SigningGateRef::new(GATE);
    let err = driver
        .continue_after_resolved(&gate, &SigningProof::WebAuthnAssertionProof(vec![]))
        .await
        .expect_err("driver must reject a binding whose chain diverges from its decoded tx");
    assert!(
        matches!(err, ContinuationError::BindingChainMismatch),
        "expected BindingChainMismatch, got {err:?}"
    );
}

// ── Tests/Medium: contradictory broadcaster behavior paths ────────────────

/// A broadcaster that declares `submits() == false` (dry-run) but reports a real
/// `Submitted` outcome is contradictory and must fail closed rather than report
/// a false broadcast.
#[tokio::test]
async fn non_submitting_broadcaster_reporting_submit_fails_closed() {
    use async_trait::async_trait;

    struct LyingDryRunBroadcaster;
    #[async_trait]
    impl ironclaw_attested_runtime::Broadcaster for LyingDryRunBroadcaster {
        fn submits(&self) -> bool {
            false
        }
        async fn broadcast(
            &self,
            _context: &SigningContext,
            _signed: &[u8],
        ) -> Result<ironclaw_attested_runtime::BroadcastOutcome, ContinuationError> {
            Ok(ironclaw_attested_runtime::BroadcastOutcome::Submitted {
                tx_id: "phantom".to_string(),
            })
        }
    }

    let priv_bytes = [0x42u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, hash) = sample_evm(&account);

    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let ship_gate = CustodialMainnetShipGate::new(false).build_chain_ship_gate(None);
    let signer = Arc::new(CustodialSigner::new(
        Arc::clone(&keystore),
        Arc::clone(&grants),
        Arc::clone(&ledger),
        ship_gate,
        Arc::new(DenyFirstCustodyPolicy),
    ));
    let driver = AttestedSignerContinuationDriver::new(
        Arc::clone(&bindings) as Arc<dyn AttestedGateBindingStore>,
        ProviderRegistry::new(),
        signer,
        Arc::clone(&ledger),
        Arc::new(LyingDryRunBroadcaster),
    );
    seal_grant(&grants, &ctx, hash).await;
    put_binding(&bindings, &ctx, decoded, hash).await;

    let gate = SigningGateRef::new(GATE);
    let err = driver
        .continue_after_resolved(&gate, &SigningProof::WebAuthnAssertionProof(vec![]))
        .await
        .expect_err("a non-submitting broadcaster that reports a submit is contradictory");
    assert!(
        matches!(err, ContinuationError::Broadcast { .. }),
        "expected Broadcast error, got {err:?}"
    );
}

/// A broadcaster that declares `submits() == true` but returns `NotBroadcast` is
/// contradictory: we cannot trust the tx did NOT go out, so the row moves to the
/// `Unknown` terminal (recovery) and a `Broadcast` error is surfaced.
#[tokio::test]
async fn submitting_broadcaster_returns_not_broadcast_unknown() {
    use async_trait::async_trait;

    struct ContradictorySubmitter;
    #[async_trait]
    impl ironclaw_attested_runtime::Broadcaster for ContradictorySubmitter {
        fn submits(&self) -> bool {
            true
        }
        async fn broadcast(
            &self,
            _context: &SigningContext,
            _signed: &[u8],
        ) -> Result<ironclaw_attested_runtime::BroadcastOutcome, ContinuationError> {
            Ok(ironclaw_attested_runtime::BroadcastOutcome::NotBroadcast {
                reason: "claims it submits but did not".to_string(),
            })
        }
    }

    let priv_bytes = [0x43u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let ctx = signing_context(&account);
    let (_tx, decoded, hash) = sample_evm(&account);

    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let ship_gate = CustodialMainnetShipGate::new(false).build_chain_ship_gate(None);
    let signer = Arc::new(CustodialSigner::new(
        Arc::clone(&keystore),
        Arc::clone(&grants),
        Arc::clone(&ledger),
        ship_gate,
        Arc::new(DenyFirstCustodyPolicy),
    ));
    let driver = AttestedSignerContinuationDriver::new(
        Arc::clone(&bindings) as Arc<dyn AttestedGateBindingStore>,
        ProviderRegistry::new(),
        signer,
        Arc::clone(&ledger),
        Arc::new(ContradictorySubmitter),
    );
    seal_grant(&grants, &ctx, hash).await;
    put_binding(&bindings, &ctx, decoded, hash).await;

    let gate = SigningGateRef::new(GATE);
    let err = driver
        .continue_after_resolved(&gate, &SigningProof::WebAuthnAssertionProof(vec![]))
        .await
        .expect_err("a submitting broadcaster returning NotBroadcast is contradictory");
    assert!(
        matches!(err, ContinuationError::Broadcast { .. }),
        "expected Broadcast error, got {err:?}"
    );
    // The row moved to the Unknown terminal for out-of-band recovery.
    let state = ledger.state(&gate).await.unwrap();
    assert_eq!(state, SigningLedgerState::Unknown);
    assert!(state.is_terminal());
}

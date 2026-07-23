//! End-to-end test for the PR11 reborn WebUI attested gate/resolve ingress.
//!
//! Drives the CALLER (`RebornServices::resolve_gate`, the same facade the
//! `ironclaw_webui_v2` `resolve_gate` HTTP handler calls) through the full
//! attested-signing lifecycle, per CLAUDE.md "Test Through the Caller":
//!
//!   raise (persist authoritative binding + seal one-shot grant)
//!     -> block the turn `BlockedAttested`
//!     -> POST an attested injected-wallet proof through `resolve_gate`
//!     -> `RuntimeAttestedResumePort` re-checks the binding + claims the resume
//!        guard -> turn transitions to `AttestedResolved`
//!     -> `AttestedSignerContinuationDriver` verifies the proof through the bound
//!        provider, claims the sealed grant, and broadcasts under the ledger.
//!
//! It also asserts the security envelope PR11 must preserve: a replayed resolve
//! fails closed (one-shot resume guard + sealed grant), and an attested resolve
//! with no continuation port wired fails closed.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ed25519_dalek::{Signer as _, SigningKey as EdSigningKey};

use ironclaw_attestation::{
    Bytes32, DecodedTransaction, RenderingSchemaVersion, SolanaCompiledInstruction,
    SolanaMessageHeader, SolanaMessageVersion, SolanaTransaction,
};
use ironclaw_attested_runtime::{
    AttestedGateBinding, InMemoryAttestedGateBindingStore, InMemoryResumeGuard, ResumeGuard,
    RuntimeAttestedResumePort, approved_tx_hash_ref_hex,
};
use ironclaw_chain_signing::{ChainKeyId, SecretsKeyStore, recompute_approved_hash};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_product_workflow::{
    AttestedContinuationOutcome, AttestedContinuationRejection, AttestedGateContinuationPort,
    AttestedProofClaim, RebornServices, RebornServicesApi, WebUiAuthenticatedCaller,
    WebUiResolveGateRequest,
};
use ironclaw_reborn_composition::{
    RebornAttestedComposition, RebornAttestedContinuation, RegisterAttestedGateError,
};
use ironclaw_secrets::SecretsCrypto;
use ironclaw_signing_provider::{
    ActorId, ApprovedTxHash, ChainId, GateRef as SigningGateRef, KeyOrAccountId, ProviderId, RunId,
    ScopeId, SigningContext, TenantId as SigningTenantId, UserId as SigningUserId,
};
use ironclaw_threads::{
    EnsureThreadRequest, InMemorySessionThreadService, SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, ApprovedTxHashRef, AttestedResumePort, BlockedReason,
    DefaultTurnCoordinator, GateRef, IdempotencyKey, InMemoryTurnStateStore,
    LoopCheckpointStateRef, ReplyTargetBindingRef, RunProfileRequest, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCheckpointId, TurnCoordinator,
    TurnLeaseToken, TurnRunId, TurnRunnerId, TurnScope,
    runner::{BlockRunRequest, ClaimRunRequest, TurnRunTransitionPort},
};

use secrecy::SecretString;
use serde_json::json;

const GATE: &str = "gate:pr11-attested-ingress";
const TENANT: &str = "tenant1";
const AGENT: &str = "agent1";
const PROJECT: &str = "project1";
const USER: &str = "user1";
const THREAD: &str = "thread-pr11";

/// The authoritative decoded transaction the binding is approved over. A Solana
/// (`solana:mainnet`) message so its `chain_network()` matches the binding's
/// `chain` / context `chain_id` and the injected-wallet (ed25519/Solana) proof
/// the resolve requests carry.
fn placeholder_decoded() -> DecodedTransaction {
    DecodedTransaction::Solana(SolanaTransaction {
        cluster: "mainnet".to_string(),
        version: SolanaMessageVersion::Legacy,
        header: SolanaMessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 1,
        },
        static_account_keys: vec![Bytes32([0x44; 32]), Bytes32([0x55; 32])],
        recent_blockhash: Bytes32([0x66; 32]),
        instructions: vec![SolanaCompiledInstruction {
            program_id_index: 1,
            account_indices: vec![0],
            data: vec![1, 2, 3],
        }],
        address_table_lookups: vec![],
    })
}

/// The 32-byte approved-tx hash the wallet attests to, recomputed from the
/// authoritative decoded tx folded with the GATE-BOUND signer (`account_hex`),
/// so a validating insert-only binding store accepts it (WYSIWYS
/// self-consistency).
fn bound_hash(account_hex: &str) -> ApprovedTxHash {
    recompute_approved_hash(
        &placeholder_decoded(),
        account_hex,
        RenderingSchemaVersion::CURRENT,
    )
    .expect("recompute approved hash in test")
}

fn signing_ctx(account_hex: &str) -> SigningContext {
    SigningContext {
        tenant: SigningTenantId::new(TENANT),
        user: SigningUserId::new(USER),
        scope: ScopeId::new("scope"),
        actor: ActorId::new("actor"),
        run_id: RunId::new("run"),
        gate_ref: SigningGateRef::new(GATE),
        chain_id: ChainId::new("solana:mainnet"),
        key_or_account_id: KeyOrAccountId::new(account_hex),
    }
}

fn turn_scope() -> TurnScope {
    TurnScope::new(
        TenantId::new(TENANT).unwrap(),
        Some(AgentId::new(AGENT).unwrap()),
        Some(ProjectId::new(PROJECT).unwrap()),
        ThreadId::new(THREAD).unwrap(),
    )
}

fn caller() -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new(TENANT).unwrap(),
        UserId::new(USER).unwrap(),
        Some(AgentId::new(AGENT).unwrap()),
        Some(ProjectId::new(PROJECT).unwrap()),
    )
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    out
}

/// Build the local-dev attested composition (the same wiring the reborn runtime
/// assembles), exposed here so the test can register a gate and read the driver.
fn build_composition(bindings: Arc<InMemoryAttestedGateBindingStore>) -> RebornAttestedComposition {
    use ironclaw_attestation::InMemorySealedGrantStore;
    use ironclaw_attested_runtime::{CustodialMainnetShipGate, ProviderRegistry};
    use ironclaw_wallet_external::InjectedSigningProvider;

    let crypto = SecretsCrypto::new(SecretString::from(
        "0123456789abcdef0123456789ABCDEF".to_string(),
    ))
    .expect("valid local-dev master key");
    let keystore = Arc::new(SecretsKeyStore::new(crypto));
    let ship_gate = CustodialMainnetShipGate::from_env().build_chain_ship_gate(None);
    let grants = Arc::new(InMemorySealedGrantStore::new());
    RebornAttestedComposition::new(bindings, keystore, ship_gate, grants, |grants| {
        ProviderRegistry::new()
            .with_provider(Arc::new(InjectedSigningProvider::new(
                Arc::clone(grants) as Arc<dyn ironclaw_attestation::SealedGrantStore>
            )))
    })
}

/// Submit a turn and block it `BlockedAttested` on `GATE`.
async fn block_attested(
    store: &Arc<InMemoryTurnStateStore>,
    expected_tx_hash_ref: &str,
) -> TurnRunId {
    let scope = turn_scope();
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let SubmitTurnResponse::Accepted { run_id, .. } = coordinator
        .submit_turn(SubmitTurnRequest {
            scope: scope.clone(),
            actor: TurnActor::new(UserId::new(USER).unwrap()),
            accepted_message_ref: AcceptedMessageRef::new("msg-pr11").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
            requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
            idempotency_key: IdempotencyKey::new("idem-pr11").unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap(),
        })
        .await
        .unwrap();
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();
    store
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id: TurnCheckpointId::new(),
            state_ref: LoopCheckpointStateRef::new("checkpoint:block").unwrap(),
            reason: BlockedReason::Attested {
                gate_ref: GateRef::new(GATE).unwrap(),
                expected_tx_hash: ApprovedTxHashRef::new(expected_tx_hash_ref).unwrap(),
            },
        })
        .await
        .unwrap();
    run_id
}

async fn ensure_thread(thread_service: &Arc<InMemorySessionThreadService>) {
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: ThreadScope {
                tenant_id: TenantId::new(TENANT).unwrap(),
                agent_id: AgentId::new(AGENT).unwrap(),
                project_id: Some(ProjectId::new(PROJECT).unwrap()),
                owner_user_id: Some(UserId::new(USER).unwrap()),
                mission_id: None,
            },
            thread_id: Some(ThreadId::new(THREAD).unwrap()),
            created_by_actor_id: USER.to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure thread");
}

/// Build an `attested` injected-wallet (Solana) resolve request whose proof
/// signs the bound hash with `key`.
fn attested_request(
    run_id: TurnRunId,
    key: &EdSigningKey,
    hash: &ApprovedTxHash,
    account_hex: &str,
    client_action_id: &str,
) -> WebUiResolveGateRequest {
    let signature = key.sign(hash.as_bytes());
    WebUiResolveGateRequest {
        client_action_id: Some(client_action_id.to_string()),
        thread_id: Some(THREAD.to_string()),
        run_id: Some(run_id.to_string()),
        gate_ref: Some(GATE.to_string()),
        resolution: Some("attested".to_string()),
        always: None,
        credential_ref: None,
        attested_proof_kind: Some("injected_wallet".to_string()),
        attested_approved_tx_hash: Some(approved_tx_hash_ref_hex(hash.as_bytes())),
        attested_proof: Some(json!({
            "scheme": "solana",
            "approved_tx_hash": lower_hex(hash.as_bytes()),
            "claimed_signer": account_hex,
            "signature": lower_hex(&signature.to_bytes()),
            "public_key": account_hex,
        })),
    }
}

fn binding(account_hex: &str, hash: ApprovedTxHash) -> AttestedGateBinding {
    AttestedGateBinding {
        provider_id: ProviderId::Injected,
        context: signing_ctx(account_hex),
        approved_tx_hash: hash,
        decoded: placeholder_decoded(),
        chain: ChainKeyId::new("solana:mainnet").expect("valid chain id in test"),
        scope: ResourceScope {
            tenant_id: TenantId::new(TENANT).unwrap(),
            user_id: UserId::new(USER).unwrap(),
            agent_id: Some(AgentId::new(AGENT).unwrap()),
            project_id: Some(ProjectId::new(PROJECT).unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        schema_version: RenderingSchemaVersion::CURRENT,
    }
}

#[tokio::test]
async fn resolve_gate_attested_drives_resume_and_continuation() {
    let key = EdSigningKey::from_bytes(&[0x22u8; 32]);
    let account_hex = lower_hex(&key.verifying_key().to_bytes());

    let hash = bound_hash(&account_hex);
    let hash_ref = approved_tx_hash_ref_hex(hash.as_bytes());

    // Wire the resume port over the shared binding store exactly as the reborn
    // factory does, then build the turn store with it injected.
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let resume_guard: Arc<dyn ResumeGuard> = Arc::new(InMemoryResumeGuard::new());
    let port: Arc<dyn AttestedResumePort> = Arc::new(RuntimeAttestedResumePort::new(
        Arc::clone(&bindings),
        Arc::clone(&resume_guard),
    ));
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port));

    // Build the attested composition over the SAME binding store, and wire the
    // continuation port into the facade.
    let composition = build_composition(Arc::clone(&bindings));

    // Raise side (PR11): persist the authoritative binding + seal the one-shot
    // grant.
    composition
        .register_attested_gate(
            SigningGateRef::new(GATE),
            binding(&account_hex, hash),
            0,
            None,
        )
        .await
        .expect("register attested gate");

    let thread_service = Arc::new(InMemorySessionThreadService::default());
    ensure_thread(&thread_service).await;

    let coordinator: Arc<dyn TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(store.clone()));
    let services = RebornServices::new(thread_service.clone(), coordinator)
        .with_attested_continuation(Arc::new(RebornAttestedContinuation::new(&composition)));

    let run_id = block_attested(&store, &hash_ref).await;

    // POST the attested proof through the facade (the resolve_gate caller).
    let response = services
        .resolve_gate(
            caller(),
            attested_request(run_id, &key, &hash, &account_hex, "action-1"),
        )
        .await
        .expect("attested resolve succeeds end-to-end");

    match response {
        ironclaw_product_workflow::RebornResolveGateResponse::Resumed(resumed) => {
            assert_eq!(
                resumed.status,
                ironclaw_turns::TurnStatus::AttestedResolved,
                "resume must transition the turn to AttestedResolved"
            );
        }
        other => panic!("expected Resumed, got {other:?}"),
    }

    // Replay: a second resolve of the same gate fails closed. The turn is no
    // longer BlockedAttested and the one-shot resume guard / sealed grant
    // refuse it.
    let replay = services
        .resolve_gate(
            caller(),
            attested_request(run_id, &key, &hash, &account_hex, "action-2"),
        )
        .await;
    assert!(
        replay.is_err(),
        "replayed attested resolve must fail closed"
    );
}

#[tokio::test]
async fn resolve_gate_attested_without_continuation_port_fails_closed() {
    let key = EdSigningKey::from_bytes(&[0x33u8; 32]);
    let account_hex = lower_hex(&key.verifying_key().to_bytes());
    let hash = bound_hash(&account_hex);
    let hash_ref = approved_tx_hash_ref_hex(hash.as_bytes());

    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let resume_guard: Arc<dyn ResumeGuard> = Arc::new(InMemoryResumeGuard::new());
    let port: Arc<dyn AttestedResumePort> = Arc::new(RuntimeAttestedResumePort::new(
        Arc::clone(&bindings),
        Arc::clone(&resume_guard),
    ));
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port));
    let composition = build_composition(Arc::clone(&bindings));
    composition
        .register_attested_gate(
            SigningGateRef::new(GATE),
            binding(&account_hex, hash),
            0,
            None,
        )
        .await
        .expect("register attested gate");

    let thread_service = Arc::new(InMemorySessionThreadService::default());
    ensure_thread(&thread_service).await;
    let coordinator: Arc<dyn TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(store.clone()));
    // No `.with_attested_continuation(...)`.
    let services = RebornServices::new(thread_service.clone(), coordinator);

    let run_id = block_attested(&store, &hash_ref).await;
    let result = services
        .resolve_gate(
            caller(),
            attested_request(run_id, &key, &hash, &account_hex, "action-1"),
        )
        .await;
    assert!(
        result.is_err(),
        "attested resolve with no continuation port wired must fail closed"
    );
}

/// A continuation port wrapper that counts how many times the full verify+claim
/// (`verify_and_claim`) runs and how many times the broadcast half
/// (`broadcast_resolved`) is driven, delegating both to the real composition
/// port. Lets the tests assert the verify-before-resume ordering and the
/// single-drive invariant (PR11 item B).
struct CountingContinuation {
    inner: RebornAttestedContinuation,
    verify_calls: Arc<std::sync::atomic::AtomicUsize>,
    drive_calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl AttestedGateContinuationPort for CountingContinuation {
    async fn verify_and_claim(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: &GateRef,
        claim: &AttestedProofClaim,
    ) -> Result<
        ironclaw_product_workflow::VerifiedAttestedContinuation,
        AttestedContinuationRejection,
    > {
        self.verify_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner
            .verify_and_claim(scope, run_id, gate_ref, claim)
            .await
    }

    async fn broadcast_resolved(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: &GateRef,
        verified: ironclaw_product_workflow::VerifiedAttestedContinuation,
    ) -> Result<AttestedContinuationOutcome, AttestedContinuationRejection> {
        self.drive_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner
            .broadcast_resolved(scope, run_id, gate_ref, verified)
            .await
    }
}

/// Build a fully-wired services + turn store + composition over a shared binding
/// store, with a `CountingContinuation` so a test can observe drive/verify counts.
async fn wired_services_with_counting(
    bytes_seed: u8,
) -> (
    RebornServices,
    Arc<InMemoryTurnStateStore>,
    String,
    EdSigningKey,
    ApprovedTxHash,
    String,
    Arc<std::sync::atomic::AtomicUsize>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    let key = EdSigningKey::from_bytes(&[bytes_seed; 32]);
    let account_hex = lower_hex(&key.verifying_key().to_bytes());
    let hash = bound_hash(&account_hex);
    let hash_ref = approved_tx_hash_ref_hex(hash.as_bytes());

    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let resume_guard: Arc<dyn ResumeGuard> = Arc::new(InMemoryResumeGuard::new());
    let port: Arc<dyn AttestedResumePort> = Arc::new(RuntimeAttestedResumePort::new(
        Arc::clone(&bindings),
        Arc::clone(&resume_guard),
    ));
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port));
    let composition = build_composition(Arc::clone(&bindings));
    composition
        .register_attested_gate(
            SigningGateRef::new(GATE),
            binding(&account_hex, hash),
            0,
            None,
        )
        .await
        .expect("register attested gate");

    let thread_service = Arc::new(InMemorySessionThreadService::default());
    ensure_thread(&thread_service).await;
    let coordinator: Arc<dyn TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(store.clone()));
    let verify_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let drive_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counting = CountingContinuation {
        inner: RebornAttestedContinuation::new(&composition),
        verify_calls: Arc::clone(&verify_calls),
        drive_calls: Arc::clone(&drive_calls),
    };
    let services = RebornServices::new(thread_service, coordinator)
        .with_attested_continuation(Arc::new(counting));
    (
        services,
        store,
        hash_ref,
        key,
        hash,
        account_hex,
        verify_calls,
        drive_calls,
    )
}

/// A same-key retry is single-drive via the grant/ledger CAS: the first resolve
/// claims the grant + broadcasts once; the retry's `verify_and_claim` re-enters
/// the driver, the one-shot ledger CAS rejects the second claim (clean replay
/// error), and the broadcast half is never reached again — no double
/// sign/broadcast.
#[tokio::test]
async fn resolve_gate_attested_retry_is_single_drive_via_grant_cas() {
    let (services, store, hash_ref, key, hash, account_hex, verify, drive) =
        wired_services_with_counting(0x44).await;
    let run_id = block_attested(&store, &hash_ref).await;

    let req = || attested_request(run_id, &key, &hash, &account_hex, "action-retry");

    // First (fresh) resolve: verify+claim runs once, broadcast drives once.
    services
        .resolve_gate(caller(), req())
        .await
        .expect("fresh attested resolve succeeds");
    assert_eq!(
        verify.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "fresh resolve runs verify+claim exactly once"
    );
    assert_eq!(
        drive.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "fresh resolve drives the broadcast exactly once"
    );

    // Retry: verify+claim re-enters the driver, the one-shot grant/ledger CAS
    // rejects the second claim. The retry fails closed (clean replay error) and
    // the broadcast half is NOT reached again.
    let replay = services.resolve_gate(caller(), req()).await;
    assert!(
        replay.is_err(),
        "same-key retry must fail closed via the grant/ledger CAS"
    );
    assert_eq!(
        verify.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "retry re-enters verify+claim (where the CAS rejects it)"
    );
    assert_eq!(
        drive.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "retry must NOT double-drive the broadcast"
    );
}

/// A malformed proof must be rejected by `verify_and_claim` BEFORE `resume_turn`
/// (it fails at decode, before any grant claim or ledger advance), leaving the
/// turn `BlockedAttested` and driving NO broadcast — so a follow-up VALID resolve
/// still succeeds.
#[tokio::test]
async fn resolve_gate_attested_malformed_proof_fails_before_resume() {
    let (services, store, hash_ref, key, hash, account_hex, verify, drive) =
        wired_services_with_counting(0x55).await;
    let run_id = block_attested(&store, &hash_ref).await;

    // Corrupt the proof so decode fails: a multibyte-Unicode signature field
    // (also exercises the panic-free hex path).
    let mut req = attested_request(run_id, &key, &hash, &account_hex, "action-bad");
    req.attested_proof = Some(json!({
        "scheme": "solana",
        "approved_tx_hash": lower_hex(hash.as_bytes()),
        "claimed_signer": account_hex,
        "signature": "déadbeef",
        "public_key": account_hex,
    }));

    let result = services.resolve_gate(caller(), req).await;
    assert!(result.is_err(), "malformed proof must fail closed");
    assert_eq!(
        verify.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "verify+claim ran (and failed at decode)"
    );
    assert_eq!(
        drive.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "broadcast must NOT be driven for a malformed proof"
    );

    // The turn must remain BlockedAttested (no state mutated): a follow-up VALID
    // resolve with a fresh client_action_id still succeeds and drives once.
    let ok = services
        .resolve_gate(
            caller(),
            attested_request(run_id, &key, &hash, &account_hex, "action-good"),
        )
        .await;
    assert!(
        ok.is_ok(),
        "turn stayed BlockedAttested after the malformed-proof rejection: {ok:?}"
    );
    assert_eq!(
        drive.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the valid follow-up drives the continuation exactly once"
    );
}

/// When NO authoritative binding was ever registered for the blocked gate, the
/// resolve must fail closed: the resume port cannot validate the attested claim
/// against an absent binding, so the turn stays `BlockedAttested` and the
/// continuation is never driven. Exercises the missing-binding path that every
/// other e2e test bypasses by registering a binding first.
#[tokio::test]
async fn resolve_gate_attested_with_no_binding_fails_closed() {
    let key = EdSigningKey::from_bytes(&[0x99u8; 32]);
    let account_hex = lower_hex(&key.verifying_key().to_bytes());
    let hash = bound_hash(&account_hex);
    let hash_ref = approved_tx_hash_ref_hex(hash.as_bytes());

    // Wire the resume port + continuation over a binding store, but DO NOT
    // register any binding for the gate.
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let resume_guard: Arc<dyn ResumeGuard> = Arc::new(InMemoryResumeGuard::new());
    let port: Arc<dyn AttestedResumePort> = Arc::new(RuntimeAttestedResumePort::new(
        Arc::clone(&bindings),
        Arc::clone(&resume_guard),
    ));
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port));
    let composition = build_composition(Arc::clone(&bindings));

    let thread_service = Arc::new(InMemorySessionThreadService::default());
    ensure_thread(&thread_service).await;
    let coordinator: Arc<dyn TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(store.clone()));
    let verify_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let drive_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counting = CountingContinuation {
        inner: RebornAttestedContinuation::new(&composition),
        verify_calls: Arc::clone(&verify_calls),
        drive_calls: Arc::clone(&drive_calls),
    };
    let services = RebornServices::new(thread_service, coordinator)
        .with_attested_continuation(Arc::new(counting));

    let run_id = block_attested(&store, &hash_ref).await;

    let result = services
        .resolve_gate(
            caller(),
            attested_request(run_id, &key, &hash, &account_hex, "action-no-binding"),
        )
        .await;
    assert!(
        result.is_err(),
        "resolve with no registered binding must fail closed, got {result:?}"
    );
    assert_eq!(
        drive_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "continuation must NOT be driven when the binding is absent"
    );
}

/// A FORGED proof (well-formed, but signed by the WRONG key) must be rejected by
/// the FULL cryptographic verification inside `verify_and_claim` BEFORE
/// `resume_turn`: the turn stays `BlockedAttested`, no broadcast is driven, and
/// the sealed grant is NOT consumed — so a follow-up VALID resolve still
/// succeeds. This is the core item-B guarantee: signature verification gates the
/// transition.
#[tokio::test]
async fn resolve_gate_attested_forged_signature_fails_before_resume() {
    let (services, store, hash_ref, key, hash, account_hex, verify, drive) =
        wired_services_with_counting(0x77).await;
    let run_id = block_attested(&store, &hash_ref).await;

    // Forge: keep the bound signer/account, but sign with a DIFFERENT key so the
    // recovered signer will not match. The proof is structurally valid (decodes
    // fine) but cryptographically wrong.
    let wrong_key = EdSigningKey::from_bytes(&[0x88u8; 32]);
    let forged_signature = wrong_key.sign(hash.as_bytes());
    let mut req = attested_request(run_id, &key, &hash, &account_hex, "action-forged");
    req.attested_proof = Some(json!({
        "scheme": "solana",
        "approved_tx_hash": lower_hex(hash.as_bytes()),
        "claimed_signer": account_hex,
        "signature": lower_hex(&forged_signature.to_bytes()),
        "public_key": account_hex,
    }));

    let result = services.resolve_gate(caller(), req).await;
    assert!(
        result.is_err(),
        "forged signature must fail closed before resume"
    );
    assert_eq!(
        verify.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "verify+claim ran (and the signature check rejected the forgery)"
    );
    assert_eq!(
        drive.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "broadcast must NOT be driven for a forged proof"
    );

    // The turn stayed BlockedAttested and the grant was NOT consumed: a valid
    // follow-up still succeeds and drives the broadcast exactly once.
    let ok = services
        .resolve_gate(
            caller(),
            attested_request(run_id, &key, &hash, &account_hex, "action-good"),
        )
        .await;
    assert!(
        ok.is_ok(),
        "turn stayed BlockedAttested + grant unclaimed after forgery: {ok:?}"
    );
    assert_eq!(
        drive.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the valid follow-up drives the broadcast exactly once"
    );
}

/// `register_attested_gate` rejects a gate_ref that mismatches
/// `binding.context.gate_ref` and refuses to overwrite an existing binding.
#[tokio::test]
async fn register_attested_gate_rejects_mismatch_and_is_insert_only() {
    let key = EdSigningKey::from_bytes(&[0x66u8; 32]);
    let account_hex = lower_hex(&key.verifying_key().to_bytes());
    let hash = bound_hash(&account_hex);
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let composition = build_composition(Arc::clone(&bindings));

    // gate_ref mismatch: binding.context.gate_ref is GATE, register under a
    // different gate_ref => GateRefMismatch.
    let mismatch = composition
        .register_attested_gate(
            SigningGateRef::new("gate:other"),
            binding(&account_hex, hash),
            0,
            None,
        )
        .await;
    assert!(
        matches!(mismatch, Err(RegisterAttestedGateError::GateRefMismatch)),
        "gate_ref/binding mismatch must be rejected, got {mismatch:?}"
    );

    // First valid raise succeeds.
    composition
        .register_attested_gate(
            SigningGateRef::new(GATE),
            binding(&account_hex, hash),
            0,
            None,
        )
        .await
        .expect("first raise succeeds");

    // Second raise for the same gate is refused (insert-only).
    let dup = composition
        .register_attested_gate(
            SigningGateRef::new(GATE),
            binding(&account_hex, hash),
            0,
            None,
        )
        .await;
    assert!(
        matches!(dup, Err(RegisterAttestedGateError::DuplicateBinding)),
        "a second raise for the same gate must be refused, got {dup:?}"
    );
}

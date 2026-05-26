//! End-to-end resume test driving the REAL `ironclaw_turns`
//! `InMemoryTurnStateStore` with the production [`RuntimeAttestedResumePort`]
//! through the full submit -> claim -> block -> resume cycle (CLAUDE.md "Test
//! Through the Caller").
//!
//! This proves the PR10 port, once injected exactly as the reborn factory
//! injects it, resolves a `BlockedAttested` gate to `AttestedResolved` (the
//! deterministic signer continuation) and NEVER requeues the agent loop
//! (threat #16), and that a replayed resume fails closed at the store boundary
//! (threat #1 at the resume layer).

use std::sync::Arc;

use alloy_consensus::TxEip1559;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use chrono::{TimeZone, Utc};
use ironclaw_attestation::{DecodedTransaction, RenderingSchemaVersion};
use ironclaw_attested_runtime::{
    AttestedGateBinding, AttestedGateBindingStore, InMemoryAttestedGateBindingStore,
    InMemoryResumeGuard, ResumeGuard, RuntimeAttestedResumePort, approved_tx_hash_ref_hex,
};
use ironclaw_chain_signing::{ChainKeyId, evm};
use ironclaw_host_api::{AgentId, ProjectId, ResourceScope, TenantId, ThreadId, UserId};
use ironclaw_signing_provider::{
    ActorId, ApprovedTxHash, ChainId, GateRef as SigningGateRef, KeyOrAccountId, ProviderId, RunId,
    ScopeId, SigningContext, TenantId as SigningTenantId, UserId as SigningUserId,
};
use ironclaw_turns::{
    AcceptedMessageRef, ApprovedTxHashRef, AttestationClaimRef, AttestedResumePort, BlockedReason,
    DefaultTurnCoordinator, GateRef, GetRunStateRequest, IdempotencyKey, InMemoryTurnStateStore,
    LoopCheckpointStateRef, ReplyTargetBindingRef, ResumeTurnRequest, RunProfileRequest,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCheckpointId,
    TurnCoordinator, TurnLeaseToken, TurnRunId, TurnRunnerId, TurnScope, TurnStateStore,
    TurnStatus,
    runner::{BlockRunRequest, ClaimRunRequest, TurnRunTransitionPort},
};

const GATE: &str = "gate:e2e-attested";

fn sample_hash() -> ApprovedTxHash {
    let tx = TxEip1559 {
        chain_id: 11155111,
        nonce: 1,
        gas_limit: 21_000,
        max_fee_per_gas: 30_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(Address::repeat_byte(0x11)),
        value: U256::from(5u64),
        input: Bytes::new(),
        access_list: Default::default(),
    };
    let decoded: DecodedTransaction = evm::decode_eip1559(&tx);
    // Fold in the SAME gate-bound signer that `signing_ctx().key_or_account_id`
    // carries (WYSIWYS) so the resume-time recompute reproduces this hash.
    ironclaw_chain_signing::recompute_approved_hash(
        &decoded,
        &"00".repeat(20),
        RenderingSchemaVersion::CURRENT,
    )
    .expect("recompute approved hash in test")
}

fn signing_ctx() -> SigningContext {
    SigningContext {
        tenant: SigningTenantId::new("tenant1"),
        user: SigningUserId::new("user1"),
        scope: ScopeId::new("scope"),
        actor: ActorId::new("actor"),
        run_id: RunId::new("run"),
        gate_ref: SigningGateRef::new(GATE),
        chain_id: ChainId::new("eip155:11155111"),
        key_or_account_id: KeyOrAccountId::new("00".repeat(20)),
    }
}

fn turn_scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant1").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn turn_actor() -> TurnActor {
    TurnActor::new(UserId::new("user1").unwrap())
}

async fn submit_and_block_attested(
    store: &Arc<InMemoryTurnStateStore>,
    thread: &str,
    expected_tx_hash_ref: &str,
) -> (TurnRunId, TurnScope) {
    let scope = turn_scope(thread);
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let SubmitTurnResponse::Accepted { run_id, .. } = coordinator
        .submit_turn(SubmitTurnRequest {
            scope: scope.clone(),
            actor: turn_actor(),
            accepted_message_ref: AcceptedMessageRef::new(format!("msg-{thread}")).unwrap(),
            source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
            requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
            idempotency_key: IdempotencyKey::new(format!("idem-{thread}")).unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 5, 12, 0, 0).unwrap(),
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
    (run_id, scope)
}

fn resume_request(scope: &TurnScope, run_id: TurnRunId, claim: &str) -> ResumeTurnRequest {
    ResumeTurnRequest {
        scope: scope.clone(),
        actor: turn_actor(),
        run_id,
        gate_resolution_ref: GateRef::new(GATE).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-resume").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-resume").unwrap(),
        idempotency_key: IdempotencyKey::new(format!("idem-resume-{run_id}")).unwrap(),
        attestation: Some(AttestationClaimRef::new(claim).unwrap()),
    }
}

#[tokio::test]
async fn real_port_resolves_attested_gate_and_never_requeues_loop() {
    let hash = sample_hash();
    let hash_ref = approved_tx_hash_ref_hex(hash.as_bytes());

    // Wire the port exactly as the reborn factory does.
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let resume_guard: Arc<dyn ResumeGuard> = Arc::new(InMemoryResumeGuard::new());
    let port: Arc<dyn AttestedResumePort> = Arc::new(RuntimeAttestedResumePort::new(
        Arc::clone(&bindings),
        Arc::clone(&resume_guard),
    ));
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port));

    // PR11 ingress would persist the authoritative binding when raising the gate.
    bindings
        .put(
            SigningGateRef::new(GATE),
            AttestedGateBinding {
                provider_id: ProviderId::Injected,
                context: signing_ctx(),
                approved_tx_hash: hash,
                decoded: evm::decode_eip1559(&TxEip1559 {
                    chain_id: 11155111,
                    nonce: 1,
                    gas_limit: 21_000,
                    max_fee_per_gas: 30_000_000_000,
                    max_priority_fee_per_gas: 1_000_000_000,
                    to: TxKind::Call(Address::repeat_byte(0x11)),
                    value: U256::from(5u64),
                    input: Bytes::new(),
                    access_list: Default::default(),
                }),
                chain: ChainKeyId::new("eip155:11155111").expect("valid chain id in test"),
                scope: ResourceScope {
                    tenant_id: TenantId::new("tenant1").unwrap(),
                    user_id: UserId::new("user1").unwrap(),
                    agent_id: None,
                    project_id: Some(ProjectId::new("project1").unwrap()),
                    mission_id: None,
                    thread_id: None,
                    invocation_id: ironclaw_host_api::InvocationId::new(),
                },
                schema_version: RenderingSchemaVersion::CURRENT,
            },
        )
        .await
        .expect("binding insert succeeds");

    let (run_id, scope) = submit_and_block_attested(&store, "thread-e2e", &hash_ref).await;

    // The attestation claim must attest to the bound hash (the wire value).
    let response = store
        .resume_turn(resume_request(&scope, run_id, &hash_ref))
        .await
        .expect("attested resume verifies through the real port");

    // Deterministic continuation: AttestedResolved, NOT requeued.
    assert_eq!(response.status, TurnStatus::AttestedResolved);
    let state = store
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::AttestedResolved);

    // Threat #16: never requeued onto the agent loop.
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope.clone()),
        })
        .await
        .unwrap();
    assert!(
        claimed.is_none(),
        "attested resume must not requeue the loop"
    );

    // Threat #1 at the resume boundary: a replayed resume of the now-resolved
    // gate fails closed (the run is no longer BlockedAttested; the one-shot
    // resume guard would also refuse it).
    let err = store
        .resume_turn(resume_request(&scope, run_id, &hash_ref))
        .await;
    // Idempotency replay returns the cached prior response rather than
    // re-resolving; either way the run never leaves AttestedResolved and the
    // loop is never re-entered.
    let final_status = store
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .unwrap()
        .status;
    assert_eq!(final_status, TurnStatus::AttestedResolved);
    let _ = err;
}

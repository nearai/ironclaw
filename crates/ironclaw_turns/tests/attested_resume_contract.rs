//! Contract tests for the attested-signing resume path (PR5).
//!
//! These tests exercise `TurnStatus::BlockedAttested`, the
//! `BlockedReason::Attested` serde shape, the injected `AttestedResumePort`,
//! and the deterministic resume-dispatch split: an attested resume validates
//! the untrusted claim and transitions to `AttestedResolved` (the signer
//! continuation), never back onto the agent-loop queue, while the existing
//! `Approval`/`Auth`/`Resource` resume paths are unchanged.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, ApprovedTxHashRef, AttestationClaimRef, AttestedResumePort,
    AttestedResumeRejection, AttestedResumeRequest, BlockedReason, DefaultTurnCoordinator, GateRef,
    IdempotencyKey, InMemoryTurnStateStore, LoopCheckpointStateRef, ReplyTargetBindingRef,
    ResumeTurnRequest, RunProfileRequest, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnCheckpointId, TurnCoordinator, TurnLeaseToken, TurnRunId, TurnRunnerId,
    TurnScope, TurnStateStore, TurnStatus,
    runner::{BlockRunRequest, ClaimRunRequest, TurnRunTransitionPort},
};

// ---- mock port -------------------------------------------------------------

/// Records the inputs it was called with and returns a configured verdict.
struct MockAttestedResumePort {
    verdict: Result<(), AttestedResumeRejection>,
    calls: Mutex<Vec<(String, String, String)>>,
}

impl MockAttestedResumePort {
    fn accepting() -> Self {
        Self {
            verdict: Ok(()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn rejecting(rejection: AttestedResumeRejection) -> Self {
        Self {
            verdict: Err(rejection),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, String, String)> {
        self.calls.lock().unwrap().clone()
    }
}

impl AttestedResumePort for MockAttestedResumePort {
    fn verify_attested_resume(
        &self,
        request: AttestedResumeRequest<'_>,
    ) -> Result<(), AttestedResumeRejection> {
        self.calls.lock().unwrap().push((
            request.gate_ref.as_str().to_string(),
            request.attestation.as_str().to_string(),
            request.expected_tx_hash.as_str().to_string(),
        ));
        self.verdict
    }
}

// ---- serde round-trips -----------------------------------------------------

#[test]
fn blocked_attested_status_serde_round_trips() {
    let json = serde_json::to_string(&TurnStatus::BlockedAttested).unwrap();
    assert_eq!(json, "\"BlockedAttested\"");
    let back: TurnStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(back, TurnStatus::BlockedAttested);

    let json = serde_json::to_string(&TurnStatus::AttestedResolved).unwrap();
    assert_eq!(json, "\"AttestedResolved\"");
    let back: TurnStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(back, TurnStatus::AttestedResolved);
}

#[test]
fn blocked_reason_attested_serde_round_trips() {
    let reason = BlockedReason::Attested {
        gate_ref: GateRef::new("gate-attested").unwrap(),
        expected_tx_hash: ApprovedTxHashRef::new("approved-hash-1").unwrap(),
    };
    let json = serde_json::to_value(&reason).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "Attested": {
                "gate_ref": "gate-attested",
                "expected_tx_hash": "approved-hash-1"
            }
        })
    );
    let back: BlockedReason = serde_json::from_value(json).unwrap();
    assert_eq!(back, reason);
    assert_eq!(back.status(), TurnStatus::BlockedAttested);
    assert_eq!(
        back.expected_tx_hash().map(|h| h.as_str()),
        Some("approved-hash-1")
    );
}

// ---- resume behavior -------------------------------------------------------

#[tokio::test]
async fn attested_resume_with_valid_port_transitions_to_signer_continuation() {
    let port = Arc::new(MockAttestedResumePort::accepting());
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port.clone()));

    let (run_id, scope) = submit_and_block_attested(&store, "thread-attested-ok").await;

    let response = store
        .resume_turn(attested_resume(&scope, run_id, Some("claim-good")))
        .await
        .unwrap();

    // Deterministic continuation: AttestedResolved, NOT Queued/requeue.
    assert_eq!(response.status, TurnStatus::AttestedResolved);
    let state = store
        .get_run_state(ironclaw_turns::GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::AttestedResolved);
    // Binding retained for the signer continuation; gate cleared.
    assert_eq!(
        state.expected_tx_hash.as_ref().map(|h| h.as_str()),
        Some("approved-tx-hash")
    );
    assert!(state.gate_ref.is_none());

    // The injected port was consulted with the persisted binding + claim.
    assert_eq!(
        port.calls(),
        vec![(
            "gate-attested".to_string(),
            "claim-good".to_string(),
            "approved-tx-hash".to_string()
        )]
    );

    // It must NOT be requeued for the agent loop: claiming the next run yields
    // nothing for this scope.
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
        "attested resume must not requeue the agent loop"
    );
}

#[tokio::test]
async fn attested_resume_without_claim_fails_closed() {
    let port = Arc::new(MockAttestedResumePort::accepting());
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port.clone()));
    let (run_id, scope) = submit_and_block_attested(&store, "thread-attested-no-claim").await;

    let err = store
        .resume_turn(attested_resume(&scope, run_id, None))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ironclaw_turns::TurnError::InvalidRequest { .. }
    ));

    // Port never consulted, run stays blocked.
    assert!(port.calls().is_empty());
    assert_eq!(
        run_status(&store, &scope, run_id).await,
        TurnStatus::BlockedAttested
    );
}

#[tokio::test]
async fn attested_resume_with_rejecting_port_stays_blocked_and_errors() {
    let port = Arc::new(MockAttestedResumePort::rejecting(
        AttestedResumeRejection::BindingMismatch,
    ));
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port.clone()));
    let (run_id, scope) = submit_and_block_attested(&store, "thread-attested-reject").await;

    let err = store
        .resume_turn(attested_resume(&scope, run_id, Some("claim-bad")))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ironclaw_turns::TurnError::InvalidRequest { .. }
    ));
    assert_eq!(port.calls().len(), 1);
    assert_eq!(
        run_status(&store, &scope, run_id).await,
        TurnStatus::BlockedAttested
    );
}

#[tokio::test]
async fn attested_resume_without_configured_port_fails_closed() {
    // No port injected.
    let store = Arc::new(InMemoryTurnStateStore::default());
    let (run_id, scope) = submit_and_block_attested(&store, "thread-attested-no-port").await;

    let err = store
        .resume_turn(attested_resume(&scope, run_id, Some("claim-good")))
        .await
        .unwrap_err();
    assert!(matches!(err, ironclaw_turns::TurnError::Unavailable { .. }));
    assert_eq!(
        run_status(&store, &scope, run_id).await,
        TurnStatus::BlockedAttested
    );
}

// ---- replay safety: signer continuation fires at most once -----------------

#[tokio::test]
async fn attested_resume_same_key_retry_is_marked_replayed() {
    // A duplicate resume with the SAME idempotency key must return the cached
    // success flagged `replayed = true`, so the reborn signer-continuation
    // layer can tell it apart from the original fresh transition and never fire
    // the external signer twice for one gate.
    let port = Arc::new(MockAttestedResumePort::accepting());
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port.clone()));
    let (run_id, scope) = submit_and_block_attested(&store, "thread-attested-retry").await;

    let request = attested_resume(&scope, run_id, Some("claim-good"));

    let first = store.resume_turn(request.clone()).await.unwrap();
    assert_eq!(first.status, TurnStatus::AttestedResolved);
    assert!(!first.replayed, "first resume is a fresh transition");

    // Same idempotency key → cached replay.
    let second = store.resume_turn(request).await.unwrap();
    assert_eq!(second.status, TurnStatus::AttestedResolved);
    assert!(second.replayed, "same-key retry must be marked replayed");
    assert_eq!(second.run_id, first.run_id);
    assert_eq!(second.event_cursor, first.event_cursor);

    // The verifier port was consulted exactly once (the fresh transition); the
    // replay short-circuits before any verification.
    assert_eq!(
        port.calls().len(),
        1,
        "replay must not re-invoke the verifier"
    );
}

#[tokio::test]
async fn attested_resume_new_key_double_resume_is_rejected() {
    // A second resume with a DIFFERENT idempotency key is a fresh transition
    // attempt against an already-resolved gate; the run is `AttestedResolved`,
    // which is non-resumable, so it fails closed without touching the port.
    let port = Arc::new(MockAttestedResumePort::accepting());
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port.clone()));
    let (run_id, scope) = submit_and_block_attested(&store, "thread-attested-double").await;

    let first = store
        .resume_turn(attested_resume(&scope, run_id, Some("claim-good")))
        .await
        .unwrap();
    assert_eq!(first.status, TurnStatus::AttestedResolved);
    assert!(!first.replayed);

    // Fresh key → not an idempotency replay; runs flat checks and is rejected
    // because AttestedResolved is non-resumable.
    let mut second_request = attested_resume(&scope, run_id, Some("claim-good"));
    second_request.idempotency_key = IdempotencyKey::new("idem-resume-attested-double-2").unwrap();
    let err = store.resume_turn(second_request).await.unwrap_err();
    assert!(matches!(
        err,
        ironclaw_turns::TurnError::InvalidTransition { .. }
    ));

    // Port consulted exactly once total; the second attempt never reached it.
    assert_eq!(port.calls().len(), 1);
    assert_eq!(
        run_status(&store, &scope, run_id).await,
        TurnStatus::AttestedResolved
    );
}

#[tokio::test]
async fn attested_resume_missing_persisted_hash_fails_closed() {
    // A run in `BlockedAttested` whose persisted `expected_tx_hash` binding is
    // absent is a store-invariant violation (e.g. a corrupted snapshot). Resume
    // must fail closed (Conflict) without consulting the port, leaving the run
    // blocked rather than resolving it.
    let port = Arc::new(MockAttestedResumePort::accepting());
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port.clone()));
    let (run_id, scope) = submit_and_block_attested(&store, "thread-attested-missing-hash").await;

    // Drop the persisted binding via a snapshot round-trip, then rebuild a store
    // carrying the port.
    let mut snapshot = store.persistence_snapshot();
    for run in &mut snapshot.runs {
        if run.run_id == run_id {
            assert_eq!(run.status, TurnStatus::BlockedAttested);
            run.expected_tx_hash = None;
        }
    }
    let store = Arc::new(
        InMemoryTurnStateStore::from_persistence_snapshot(
            snapshot,
            ironclaw_turns::InMemoryTurnStateStoreLimits::default(),
        )
        .unwrap()
        .with_attested_resume_port(port.clone()),
    );

    let err = store
        .resume_turn(attested_resume(&scope, run_id, Some("claim-good")))
        .await
        .unwrap_err();
    assert!(matches!(err, ironclaw_turns::TurnError::Conflict { .. }));
    assert!(
        port.calls().is_empty(),
        "missing binding must not reach the port"
    );
    assert_eq!(
        run_status(&store, &scope, run_id).await,
        TurnStatus::BlockedAttested
    );
}

// ---- recommended: wrong actor/scope/gate must NOT call the port ------------

#[tokio::test]
async fn attested_resume_wrong_actor_does_not_call_port() {
    let port = Arc::new(MockAttestedResumePort::accepting());
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port.clone()));
    let (run_id, scope) = submit_and_block_attested(&store, "thread-attested-wrong-actor").await;

    let mut request = attested_resume(&scope, run_id, Some("claim-good"));
    request.actor = TurnActor::new(UserId::new("intruder").unwrap());
    let err = store.resume_turn(request).await.unwrap_err();
    assert!(matches!(err, ironclaw_turns::TurnError::Unauthorized));
    assert!(port.calls().is_empty(), "wrong actor must not reach port");
    assert_eq!(
        run_status(&store, &scope, run_id).await,
        TurnStatus::BlockedAttested
    );
}

#[tokio::test]
async fn attested_resume_wrong_scope_does_not_call_port() {
    let port = Arc::new(MockAttestedResumePort::accepting());
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port.clone()));
    let (run_id, scope) = submit_and_block_attested(&store, "thread-attested-wrong-scope").await;

    let mut request = attested_resume(&scope, run_id, Some("claim-good"));
    request.scope = scope_other("thread-attested-wrong-scope-other");
    let err = store.resume_turn(request).await.unwrap_err();
    assert!(matches!(err, ironclaw_turns::TurnError::ScopeNotFound));
    assert!(port.calls().is_empty(), "wrong scope must not reach port");
    assert_eq!(
        run_status(&store, &scope, run_id).await,
        TurnStatus::BlockedAttested
    );
}

#[tokio::test]
async fn attested_resume_wrong_gate_does_not_call_port() {
    let port = Arc::new(MockAttestedResumePort::accepting());
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port.clone()));
    let (run_id, scope) = submit_and_block_attested(&store, "thread-attested-wrong-gate").await;

    let mut request = attested_resume(&scope, run_id, Some("claim-good"));
    request.gate_resolution_ref = GateRef::new("gate-wrong").unwrap();
    let err = store.resume_turn(request).await.unwrap_err();
    assert!(matches!(
        err,
        ironclaw_turns::TurnError::InvalidRequest { .. }
    ));
    assert!(port.calls().is_empty(), "gate mismatch must not reach port");
    assert_eq!(
        run_status(&store, &scope, run_id).await,
        TurnStatus::BlockedAttested
    );
}

#[tokio::test]
async fn standard_resume_carrying_attestation_does_not_enter_attested_path() {
    // A non-`BlockedAttested` resume that happens to carry an attestation claim
    // must take the standard requeue path and never consult the attested port.
    let port = Arc::new(MockAttestedResumePort::accepting());
    let store = Arc::new(InMemoryTurnStateStore::default().with_attested_resume_port(port.clone()));
    let (run_id, scope) = submit_and_block_standard(
        &store,
        "thread-approval-with-attestation",
        BlockedReason::Approval {
            gate_ref: GateRef::new("gate-approval-attn").unwrap(),
        },
    )
    .await;

    let mut request = standard_resume(&scope, run_id, "gate-approval-attn");
    request.attestation = Some(AttestationClaimRef::new("stray-claim").unwrap());
    let response = store.resume_turn(request).await.unwrap();
    assert_eq!(response.status, TurnStatus::Queued);
    assert!(!response.replayed);
    assert!(
        port.calls().is_empty(),
        "standard resume must not enter the attested path"
    );
}

// ---- regression: existing blocked reasons unchanged ------------------------

#[tokio::test]
async fn approval_resume_still_requeues_agent_loop() {
    // No attested port needed; approval resume must behave exactly as before.
    let store = Arc::new(InMemoryTurnStateStore::default());
    let (run_id, scope) = submit_and_block_standard(
        &store,
        "thread-approval",
        BlockedReason::Approval {
            gate_ref: GateRef::new("gate-approval").unwrap(),
        },
    )
    .await;

    let response = store
        .resume_turn(standard_resume(&scope, run_id, "gate-approval"))
        .await
        .unwrap();
    assert_eq!(response.status, TurnStatus::Queued);

    // Requeued: the run is claimable again.
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope.clone()),
        })
        .await
        .unwrap();
    assert!(
        claimed.is_some(),
        "approval resume must requeue the agent loop"
    );
}

#[tokio::test]
async fn auth_and_resource_resume_still_requeue_agent_loop() {
    for (thread, gate, reason) in [
        (
            "thread-auth",
            "gate-auth",
            BlockedReason::Auth {
                gate_ref: GateRef::new("gate-auth").unwrap(),
            },
        ),
        (
            "thread-resource",
            "gate-resource",
            BlockedReason::Resource {
                gate_ref: GateRef::new("gate-resource").unwrap(),
            },
        ),
    ] {
        let store = Arc::new(InMemoryTurnStateStore::default());
        let (run_id, scope) = submit_and_block_standard(&store, thread, reason).await;
        let response = store
            .resume_turn(standard_resume(&scope, run_id, gate))
            .await
            .unwrap();
        assert_eq!(response.status, TurnStatus::Queued);
        let claimed = store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: Some(scope.clone()),
            })
            .await
            .unwrap();
        assert!(claimed.is_some());
    }
}

// ---- helpers ---------------------------------------------------------------

async fn submit_and_block_attested(
    store: &Arc<InMemoryTurnStateStore>,
    thread: &str,
) -> (TurnRunId, TurnScope) {
    submit_and_block_standard(
        store,
        thread,
        BlockedReason::Attested {
            gate_ref: GateRef::new("gate-attested").unwrap(),
            expected_tx_hash: ApprovedTxHashRef::new("approved-tx-hash").unwrap(),
        },
    )
    .await
}

async fn submit_and_block_standard(
    store: &Arc<InMemoryTurnStateStore>,
    thread: &str,
    reason: BlockedReason,
) -> (TurnRunId, TurnScope) {
    let scope = scope(thread);
    let submit = SubmitTurnRequest {
        scope: scope.clone(),
        actor: actor(),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{thread}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(format!("idem-{thread}-submit")).unwrap(),
        received_at: received_at(),
    };
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let SubmitTurnResponse::Accepted { run_id, .. } =
        coordinator.submit_turn(submit).await.unwrap();
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
            state_ref: LoopCheckpointStateRef::new("checkpoint:block-state").unwrap(),
            reason,
        })
        .await
        .unwrap();
    (run_id, scope)
}

fn attested_resume(scope: &TurnScope, run_id: TurnRunId, claim: Option<&str>) -> ResumeTurnRequest {
    ResumeTurnRequest {
        scope: scope.clone(),
        actor: actor(),
        run_id,
        gate_resolution_ref: GateRef::new("gate-attested").unwrap(),
        source_binding_ref: SourceBindingRef::new("source-resume").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-resume").unwrap(),
        idempotency_key: IdempotencyKey::new(format!("idem-resume-{run_id}")).unwrap(),
        attestation: claim.map(|c| AttestationClaimRef::new(c).unwrap()),
    }
}

fn standard_resume(scope: &TurnScope, run_id: TurnRunId, gate: &str) -> ResumeTurnRequest {
    ResumeTurnRequest {
        scope: scope.clone(),
        actor: actor(),
        run_id,
        gate_resolution_ref: GateRef::new(gate).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-resume").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-resume").unwrap(),
        idempotency_key: IdempotencyKey::new(format!("idem-resume-{run_id}")).unwrap(),
        attestation: None,
    }
}

async fn run_status(
    store: &Arc<InMemoryTurnStateStore>,
    scope: &TurnScope,
    run_id: TurnRunId,
) -> TurnStatus {
    store
        .get_run_state(ironclaw_turns::GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .unwrap()
        .status
}

fn received_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 5, 12, 0, 0).unwrap()
}

fn scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant1").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

/// A scope in a different tenant, for wrong-scope rejection tests.
fn scope_other(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-other").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn actor() -> TurnActor {
    TurnActor::new(UserId::new("user1").unwrap())
}

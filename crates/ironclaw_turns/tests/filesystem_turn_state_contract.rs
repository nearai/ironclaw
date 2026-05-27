//! Contract tests for [`FilesystemTurnStateStore`] against a
//! [`ScopedFilesystem`] over [`LocalFilesystem`]. The persistent shape is a
//! single `/turns/state.json` snapshot keyed by the [`MountView`] target.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, HostPath, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId,
    ThreadId, UserId, VirtualPath,
};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, ApprovedTxHashRef, AttestationClaimRef,
    AttestedResumePort, AttestedResumeRejection, AttestedResumeRequest, BlockedReason,
    FilesystemTurnStateStore, GateRef, GetRunStateRequest, IdempotencyKey,
    InMemoryRunProfileResolver, LoopCheckpointStateRef, ReplyTargetBindingRef, ResumeTurnRequest,
    RunProfileRequest, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnCheckpointId, TurnLeaseToken, TurnRunId, TurnRunnerId, TurnScope, TurnStateStore,
    TurnStatus,
    runner::{BlockRunRequest, ClaimRunRequest, TurnRunTransitionPort},
};
use std::sync::Mutex;

/// Build a [`LocalFilesystem`] with `/engine` mounted to a tempdir; the
/// `/turns` alias on the outer [`ScopedFilesystem`] resolves under
/// `/engine/...` per the test convention used by the run-state contract.
fn engine_filesystem() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

/// Wrap a [`RootFilesystem`] in a [`ScopedFilesystem`] that exposes the
/// `/turns` mount alias under a tenant/user-scoped subtree of the underlying
/// mount target.
fn scoped_turns_fs_at<F>(backend: Arc<F>, tenant: &str, user: &str) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let tenant_user_prefix = format!("/engine/tenants/{tenant}/users/{user}");
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("alias"),
        VirtualPath::new(format!("{tenant_user_prefix}/turns")).expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

fn scoped_turns_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    scoped_turns_fs_at(backend, "test-tenant", "test-user")
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

fn submit_request_for(scope: TurnScope, idempotency_key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        scope,
        actor: turn_actor(),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{idempotency_key}"))
            .unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap(),
    }
}

fn accepted_run_id(response: &SubmitTurnResponse) -> TurnRunId {
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    *run_id
}

// ---- attested resume through the filesystem store --------------------------

/// Records calls and returns a configured verdict, like the in-memory contract.
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

/// Submit a turn, claim it, then block it on a `BlockedAttested` gate, all
/// through the supplied filesystem store. Returns the run id and scope.
async fn fs_submit_and_block_attested<F>(
    store: &FilesystemTurnStateStore<F>,
    thread: &str,
) -> (TurnRunId, TurnScope)
where
    F: RootFilesystem,
{
    let scope = turn_scope(thread);
    let resolver = InMemoryRunProfileResolver::default();
    let response = store
        .submit_turn(
            submit_request_for(scope.clone(), &format!("idem-{thread}-submit")),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
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
            state_ref: LoopCheckpointStateRef::new("checkpoint:fs-block-state").unwrap(),
            reason: BlockedReason::Attested {
                gate_ref: GateRef::new("gate-attested").unwrap(),
                expected_tx_hash: ApprovedTxHashRef::new("approved-tx-hash").unwrap(),
            },
        })
        .await
        .unwrap();
    (run_id, scope)
}

fn fs_attested_resume(
    scope: &TurnScope,
    run_id: TurnRunId,
    claim: Option<&str>,
    idem: &str,
) -> ResumeTurnRequest {
    ResumeTurnRequest {
        scope: scope.clone(),
        actor: turn_actor(),
        run_id,
        gate_resolution_ref: GateRef::new("gate-attested").unwrap(),
        source_binding_ref: SourceBindingRef::new("source-resume").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-resume").unwrap(),
        idempotency_key: IdempotencyKey::new(idem).unwrap(),
        attestation: claim.map(|c| AttestationClaimRef::new(c).unwrap()),
    }
}

async fn fs_run_status<F>(
    store: &FilesystemTurnStateStore<F>,
    scope: &TurnScope,
    run_id: TurnRunId,
) -> TurnStatus
where
    F: RootFilesystem,
{
    store
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .unwrap()
        .status
}

#[tokio::test]
async fn fs_attested_resume_with_injected_port_transitions_to_signer_continuation() {
    // Regression: the filesystem store must carry the injected port into the
    // transient in-memory store it builds per mutation. Without the fix the
    // port was dropped and this resume failed closed with "port not
    // configured".
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let port = Arc::new(MockAttestedResumePort::accepting());
    let store = FilesystemTurnStateStore::new(scoped).with_attested_resume_port(port.clone());

    let (run_id, scope) = fs_submit_and_block_attested(&store, "thread-fs-attested-ok").await;

    let response = store
        .resume_turn(fs_attested_resume(
            &scope,
            run_id,
            Some("claim-good"),
            "idem-fs-attested-ok-resume",
        ))
        .await
        .unwrap();
    assert_eq!(response.status, TurnStatus::AttestedResolved);
    assert!(!response.replayed);
    assert_eq!(
        port.calls(),
        vec![(
            "gate-attested".to_string(),
            "claim-good".to_string(),
            "approved-tx-hash".to_string()
        )],
        "the filesystem store must thread the port into the transient store"
    );
    assert_eq!(
        fs_run_status(&store, &scope, run_id).await,
        TurnStatus::AttestedResolved
    );
}

#[tokio::test]
async fn fs_attested_resume_without_configured_port_fails_closed() {
    // No port injected on the filesystem store → attested resume fails closed,
    // run stays blocked.
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(scoped);

    let (run_id, scope) = fs_submit_and_block_attested(&store, "thread-fs-attested-no-port").await;

    let err = store
        .resume_turn(fs_attested_resume(
            &scope,
            run_id,
            Some("claim-good"),
            "idem-fs-attested-no-port-resume",
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, ironclaw_turns::TurnError::Unavailable { .. }));
    assert_eq!(
        fs_run_status(&store, &scope, run_id).await,
        TurnStatus::BlockedAttested
    );
}

#[tokio::test]
async fn fs_attested_resume_same_key_retry_is_marked_replayed() {
    // Replay safety holds across the filesystem store's persistent idempotency
    // map: a same-key retry returns the cached success flagged `replayed`, and
    // the verifier is consulted only on the fresh transition.
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let port = Arc::new(MockAttestedResumePort::accepting());
    let store = FilesystemTurnStateStore::new(scoped).with_attested_resume_port(port.clone());

    let (run_id, scope) = fs_submit_and_block_attested(&store, "thread-fs-attested-retry").await;
    let request = fs_attested_resume(&scope, run_id, Some("claim-good"), "idem-fs-attested-retry");

    let first = store.resume_turn(request.clone()).await.unwrap();
    assert_eq!(first.status, TurnStatus::AttestedResolved);
    assert!(!first.replayed);

    let second = store.resume_turn(request).await.unwrap();
    assert_eq!(second.status, TurnStatus::AttestedResolved);
    assert!(
        second.replayed,
        "same-key retry through the filesystem store must be marked replayed"
    );
    assert_eq!(
        port.calls().len(),
        1,
        "replay must not re-invoke the verifier"
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_persists_submit_and_reopens() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(turn_scope("thread-fs-persist"), "idem-fs-persist");
    let response = store
        .submit_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);

    // Re-construct the store over the same scoped filesystem; the on-disk
    // snapshot must rehydrate the queued run.
    let reopened = FilesystemTurnStateStore::new(scoped);
    let state = reopened
        .get_run_state(GetRunStateRequest {
            scope: request.scope,
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.run_id, run_id);
    assert_eq!(state.status, TurnStatus::Queued);
}

#[tokio::test]
async fn filesystem_turn_state_store_hides_records_from_other_tenants_via_mount_view() {
    // Regression for the ScopedFilesystem migration: two stores share one
    // underlying RootFilesystem but each is constructed with a MountView
    // whose `/turns` alias resolves to a different tenant-scoped VirtualPath
    // subtree. Writing the same (thread, idempotency_key) on tenant A's
    // store must NOT make the snapshot visible from tenant B's store. The
    // structural fix routes every op through ScopedFilesystem; two
    // MountViews over the same backend cannot see each other's snapshots.
    let backend = Arc::new(engine_filesystem());
    let scoped_a = scoped_turns_fs_at(Arc::clone(&backend), "tenant-a", "alice");
    let scoped_b = scoped_turns_fs_at(Arc::clone(&backend), "tenant-b", "alice");

    let store_a = FilesystemTurnStateStore::new(Arc::clone(&scoped_a));
    let store_b = FilesystemTurnStateStore::new(Arc::clone(&scoped_b));
    let resolver = InMemoryRunProfileResolver::default();

    let scope_a = TurnScope::new(
        TenantId::new("tenant-a").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("thread-cross-tenant").unwrap(),
    );
    let scope_b = TurnScope::new(
        TenantId::new("tenant-b").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("thread-cross-tenant").unwrap(),
    );

    let response_a = store_a
        .submit_turn(
            submit_request_for(scope_a.clone(), "idem-cross-tenant"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id_a = accepted_run_id(&response_a);

    // Tenant A sees its own run.
    let state_a = store_a
        .get_run_state(GetRunStateRequest {
            scope: scope_a.clone(),
            run_id: run_id_a,
        })
        .await
        .unwrap();
    assert_eq!(state_a.run_id, run_id_a);

    // Tenant B does NOT see tenant A's run id, despite the identical
    // (thread, idempotency_key). The mount target prefix in tenant B's
    // ScopedFilesystem resolves to a disjoint VirtualPath, so the snapshot
    // is absent and `get_run_state` reports `ScopeNotFound`.
    let err = store_b
        .get_run_state(GetRunStateRequest {
            scope: scope_b.clone(),
            run_id: run_id_a,
        })
        .await
        .expect_err("tenant B must NOT see tenant A's run (cross-tenant snapshot leak)");
    assert!(matches!(err, ironclaw_turns::TurnError::ScopeNotFound));

    // Tenant B can independently submit with the same idempotency_key and
    // observe its own run id, distinct from tenant A's.
    let response_b = store_b
        .submit_turn(
            submit_request_for(scope_b.clone(), "idem-cross-tenant"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id_b = accepted_run_id(&response_b);
    assert_ne!(
        run_id_a, run_id_b,
        "each tenant snapshot must mint its own run id; collision implies leakage"
    );
}

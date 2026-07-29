use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use chrono::{Duration, TimeZone, Utc};
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, BlockedReason, CancelRunRequest, GateRef,
    IdempotencyKey, InMemoryRunProfileResolver, ReplyTargetBindingRef, ResumeTurnPrecondition,
    ResumeTurnRequest, RunProfileRequest, SanitizedCancelReason, SanitizedFailure,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCheckpointId,
    TurnError, TurnLeaseToken, TurnPersistenceSnapshot, TurnRunId, TurnRunnerId, TurnScope,
    TurnStateRowStore, TurnStateStore, TurnStatus,
    run_profile::LoopCheckpointStateRef,
    runner::{
        BlockRunRequest, ClaimRunRequest, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecoverExpiredLeasesRequest, TurnRunTransitionPort,
    },
};
use rand::{RngExt, SeedableRng, rngs::StdRng};

type TurnStore = TurnStateRowStore<InMemoryBackend>;

const MAX_CRASH_RECOVERY_RECLAIMS: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedRun {
    scope_idx: usize,
    status: TurnStatus,
    runner_id: Option<TurnRunnerId>,
    lease_token: Option<TurnLeaseToken>,
    checkpoint_id: Option<TurnCheckpointId>,
    gate_ref: Option<GateRef>,
    failure_present: bool,
    claim_count: u64,
}

#[derive(Debug, Default)]
struct ReferenceModel {
    runs: BTreeMap<TurnRunId, ExpectedRun>,
    queue: VecDeque<TurnRunId>,
}

impl ReferenceModel {
    fn submit(&mut self, scope_idx: usize, run_id: TurnRunId) {
        self.runs.insert(
            run_id,
            ExpectedRun {
                scope_idx,
                status: TurnStatus::Queued,
                runner_id: None,
                lease_token: None,
                checkpoint_id: None,
                gate_ref: None,
                failure_present: false,
                claim_count: 0,
            },
        );
        self.queue.push_back(run_id);
    }

    fn claim(&mut self, scope_idx: usize, runner_id: TurnRunnerId, lease_token: TurnLeaseToken) {
        let run_id = self.pop_queued_run_for_scope(scope_idx);
        let run = self
            .runs
            .get_mut(&run_id)
            .expect("claimed run must exist in model");
        run.status = TurnStatus::Running;
        run.runner_id = Some(runner_id);
        run.lease_token = Some(lease_token);
        run.claim_count = run.claim_count.saturating_add(1);
    }

    fn heartbeat(&mut self, run_id: TurnRunId) {
        let run = self
            .runs
            .get(&run_id)
            .expect("heartbeat run must exist in model");
        assert_eq!(run.status, TurnStatus::Running);
    }

    fn block(
        &mut self,
        run_id: TurnRunId,
        status: TurnStatus,
        checkpoint_id: TurnCheckpointId,
        gate_ref: GateRef,
    ) {
        let run = self
            .runs
            .get_mut(&run_id)
            .expect("blocked run must exist in model");
        assert_eq!(run.status, TurnStatus::Running);
        run.status = status;
        run.runner_id = None;
        run.lease_token = None;
        run.checkpoint_id = Some(checkpoint_id);
        run.gate_ref = Some(gate_ref);
    }

    fn resume(&mut self, run_id: TurnRunId) {
        let run = self
            .runs
            .get_mut(&run_id)
            .expect("resumed run must exist in model");
        assert!(matches!(
            run.status,
            TurnStatus::BlockedApproval | TurnStatus::BlockedAuth | TurnStatus::BlockedResource
        ));
        run.status = TurnStatus::Queued;
        run.gate_ref = None;
        self.queue.push_back(run_id);
    }

    fn request_cancel(&mut self, run_id: TurnRunId) {
        let run = self
            .runs
            .get_mut(&run_id)
            .expect("cancelled run must exist in model");
        match run.status {
            TurnStatus::Queued
            | TurnStatus::BlockedApproval
            | TurnStatus::BlockedAuth
            | TurnStatus::BlockedResource
            | TurnStatus::BlockedDependentRun => {
                run.status = TurnStatus::Cancelled;
                run.failure_present = false;
                Self::remove_queued(&mut self.queue, run_id);
            }
            TurnStatus::Running | TurnStatus::CancelRequested => {
                run.status = TurnStatus::CancelRequested;
            }
            status => assert!(status.is_terminal()),
        }
    }

    fn complete(&mut self, run_id: TurnRunId) {
        self.terminal(run_id, TurnStatus::Completed, false);
    }

    fn fail(&mut self, run_id: TurnRunId) {
        self.terminal(run_id, TurnStatus::Failed, true);
        self.runs
            .get_mut(&run_id)
            .expect("failed run must exist in model")
            .checkpoint_id = None;
    }

    fn recover_expired_running(&mut self, run_id: TurnRunId) {
        let run = self
            .runs
            .get_mut(&run_id)
            .expect("recovered run must exist in model");
        assert_eq!(run.status, TurnStatus::Running);
        run.runner_id = None;
        run.lease_token = None;
        if run.claim_count >= MAX_CRASH_RECOVERY_RECLAIMS {
            run.status = TurnStatus::Failed;
            run.failure_present = true;
        } else {
            run.status = TurnStatus::Queued;
            self.queue.push_back(run_id);
        }
    }

    fn recover_expired_cancel_requested(&mut self, run_id: TurnRunId) {
        let run = self
            .runs
            .get_mut(&run_id)
            .expect("cancel-recovered run must exist in model");
        assert_eq!(run.status, TurnStatus::CancelRequested);
        run.status = TurnStatus::Cancelled;
        run.runner_id = None;
        run.lease_token = None;
        run.failure_present = false;
    }

    fn recover_expired_scope(&mut self, scope_idx: usize) -> Vec<TurnRunId> {
        let recoverable = self
            .runs
            .iter()
            .filter_map(|(run_id, run)| {
                (run.scope_idx == scope_idx
                    && matches!(
                        run.status,
                        TurnStatus::Running | TurnStatus::CancelRequested
                    ))
                .then_some(*run_id)
            })
            .collect::<Vec<_>>();
        for run_id in &recoverable {
            match self
                .runs
                .get(run_id)
                .expect("recovered run must exist in model")
                .status
            {
                TurnStatus::Running => self.recover_expired_running(*run_id),
                TurnStatus::CancelRequested => self.recover_expired_cancel_requested(*run_id),
                status => panic!("unexpected recoverable status in model: {status:?}"),
            }
        }
        recoverable
    }

    fn has_active_run_in_scope(&self, scope_idx: usize) -> bool {
        self.runs
            .values()
            .any(|run| run.scope_idx == scope_idx && !run.status.is_terminal())
    }

    fn queued_scope_indices(&self) -> Vec<usize> {
        self.queue
            .iter()
            .filter_map(|run_id| {
                self.runs
                    .get(run_id)
                    .filter(|run| run.status == TurnStatus::Queued)
                    .map(|run| run.scope_idx)
            })
            .collect()
    }

    fn run_ids_with_status(&self, status: TurnStatus) -> Vec<TurnRunId> {
        self.runs
            .iter()
            .filter_map(|(run_id, run)| (run.status == status).then_some(*run_id))
            .collect()
    }

    fn running_run_ids(&self) -> Vec<TurnRunId> {
        self.run_ids_with_status(TurnStatus::Running)
    }

    fn blocked_run_ids(&self) -> Vec<TurnRunId> {
        self.runs
            .iter()
            .filter_map(|(run_id, run)| {
                matches!(
                    run.status,
                    TurnStatus::BlockedApproval
                        | TurnStatus::BlockedAuth
                        | TurnStatus::BlockedResource
                )
                .then_some(*run_id)
            })
            .collect()
    }

    fn recoverable_scope_indices(&self) -> Vec<usize> {
        self.runs
            .values()
            .filter_map(|run| {
                matches!(
                    run.status,
                    TurnStatus::Running | TurnStatus::CancelRequested
                )
                .then_some(run.scope_idx)
            })
            .collect()
    }

    fn run(&self, run_id: TurnRunId) -> &ExpectedRun {
        self.runs.get(&run_id).expect("model run should exist")
    }

    fn terminal(&mut self, run_id: TurnRunId, status: TurnStatus, failure_present: bool) {
        let run = self
            .runs
            .get_mut(&run_id)
            .expect("terminal run must exist in model");
        assert_eq!(run.status, TurnStatus::Running);
        run.status = status;
        run.runner_id = None;
        run.lease_token = None;
        run.failure_present = failure_present;
        Self::remove_queued(&mut self.queue, run_id);
    }

    fn pop_queued_run_for_scope(&mut self, scope_idx: usize) -> TurnRunId {
        let index = self
            .queue
            .iter()
            .position(|run_id| {
                self.runs.get(run_id).is_some_and(|run| {
                    run.scope_idx == scope_idx && run.status == TurnStatus::Queued
                })
            })
            .expect("model should have a queued run for scope");
        self.queue
            .remove(index)
            .expect("queued run index should exist")
    }

    fn remove_queued(queue: &mut VecDeque<TurnRunId>, run_id: TurnRunId) {
        queue.retain(|queued| *queued != run_id);
    }
}

fn turn_scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-reference-model").unwrap(),
        Some(AgentId::new("agent-reference-model").unwrap()),
        Some(ProjectId::new("project-reference-model").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn scopes() -> Vec<TurnScope> {
    vec![
        turn_scope("thread-a"),
        turn_scope("thread-b"),
        turn_scope("thread-c"),
    ]
}

fn turn_actor() -> TurnActor {
    TurnActor::new(UserId::new("user-reference-model").unwrap())
}

fn submit_request(scope: TurnScope, run_id: TurnRunId, idem: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        requested_model: None,
        scope,
        actor: turn_actor(),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{idem}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-reference-model").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-reference-model").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idem).unwrap(),
        received_at: Utc.with_ymd_and_hms(2026, 7, 17, 12, 0, 0).unwrap(),
        requested_run_id: Some(run_id),
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    }
}

fn resume_request(
    scope: TurnScope,
    run_id: TurnRunId,
    gate_ref: GateRef,
    idem: &str,
) -> ResumeTurnRequest {
    ResumeTurnRequest {
        scope,
        actor: turn_actor(),
        run_id,
        gate_resolution_ref: gate_ref,
        source_binding_ref: SourceBindingRef::new("source-reference-model-resume").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-reference-model-resume")
            .unwrap(),
        idempotency_key: IdempotencyKey::new(idem).unwrap(),
        precondition: ResumeTurnPrecondition::default(),
        resume_disposition: None,
    }
}

fn cancel_request(scope: TurnScope, run_id: TurnRunId, idem: &str) -> CancelRunRequest {
    CancelRunRequest {
        scope,
        actor: turn_actor(),
        run_id,
        reason: SanitizedCancelReason::UserRequested,
        idempotency_key: IdempotencyKey::new(idem).unwrap(),
    }
}

fn gate_ref(tag: &str) -> GateRef {
    GateRef::new(format!("gate-reference-model-{tag}")).unwrap()
}

fn state_ref(tag: &str) -> LoopCheckpointStateRef {
    LoopCheckpointStateRef::new(format!("checkpoint:reference-model-{tag}")).unwrap()
}

fn scope_key(scope: &TurnScope) -> String {
    serde_json::to_string(scope).unwrap()
}

fn run_projection(snapshot: TurnPersistenceSnapshot) -> BTreeMap<TurnRunId, ExpectedRun> {
    snapshot
        .runs
        .into_iter()
        .map(|run| {
            let scope_idx = match run.scope.thread_id.as_str() {
                "thread-a" => 0,
                "thread-b" => 1,
                "thread-c" => 2,
                other => panic!("unexpected thread in snapshot: {other}"),
            };
            (
                run.run_id,
                ExpectedRun {
                    scope_idx,
                    status: run.status,
                    runner_id: run.runner_id,
                    lease_token: run.lease_token,
                    checkpoint_id: run.checkpoint_id,
                    gate_ref: run.gate_ref,
                    failure_present: run.failure.is_some(),
                    claim_count: run.claim_count,
                },
            )
        })
        .collect()
}

async fn assert_store_matches_model(
    store: &TurnStore,
    model: &ReferenceModel,
    scopes: &[TurnScope],
    label: &str,
) {
    let snapshot = store.persistence_snapshot().await.unwrap();
    assert_eq!(
        run_projection(snapshot.clone()),
        model.runs,
        "{label}: runs"
    );

    let expected_active = model
        .runs
        .iter()
        .filter(|(_, run)| !run.status.is_terminal())
        .map(|(run_id, run)| (scope_key(&scopes[run.scope_idx]), (*run_id, run.status)))
        .collect::<BTreeMap<_, _>>();
    let actual_active = snapshot
        .active_locks
        .iter()
        .map(|lock| (scope_key(&lock.key.scope), (lock.run_id, lock.status)))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(actual_active, expected_active, "{label}: active locks");

    for run in snapshot.runs {
        if run.status.is_terminal() {
            assert_eq!(run.runner_id, None, "{label}: terminal runner lease");
            assert_eq!(run.lease_token, None, "{label}: terminal lease token");
        }
    }
}

fn snapshot_projection(
    snapshot: TurnPersistenceSnapshot,
) -> (
    BTreeMap<TurnRunId, ExpectedRun>,
    BTreeMap<String, (TurnRunId, TurnStatus)>,
) {
    let active_locks = snapshot
        .active_locks
        .iter()
        .map(|lock| (scope_key(&lock.key.scope), (lock.run_id, lock.status)))
        .collect();
    (run_projection(snapshot), active_locks)
}

async fn assert_projection_unchanged_after<Fut>(
    store: &TurnStore,
    label: &str,
    operation: impl FnOnce() -> Fut,
) where
    Fut: std::future::Future<Output = ()>,
{
    let before = snapshot_projection(store.persistence_snapshot().await.unwrap());
    operation().await;
    let after = snapshot_projection(store.persistence_snapshot().await.unwrap());
    assert_eq!(after, before, "{label}");
}

async fn submit_ok(
    store: &TurnStore,
    model: &mut ReferenceModel,
    scopes: &[TurnScope],
    scope_idx: usize,
    run_id: TurnRunId,
    idem: &str,
) {
    let response = store
        .submit_turn(
            submit_request(scopes[scope_idx].clone(), run_id, idem),
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .unwrap();
    assert!(matches!(
        response,
        SubmitTurnResponse::Accepted {
            run_id: accepted,
            status: TurnStatus::Queued,
            ..
        } if accepted == run_id
    ));
    model.submit(scope_idx, run_id);
    assert_store_matches_model(store, model, scopes, idem).await;
}

async fn claim_ok(
    store: &TurnStore,
    model: &mut ReferenceModel,
    scopes: &[TurnScope],
    scope_idx: usize,
    runner_id: TurnRunnerId,
    lease_token: TurnLeaseToken,
    label: &str,
) -> TurnRunId {
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scopes[scope_idx].clone()),
        })
        .await
        .unwrap()
        .expect("expected queued run to be claimable");
    model.claim(scope_idx, runner_id, lease_token);
    assert_store_matches_model(store, model, scopes, label).await;
    claimed.state.run_id
}

struct BuiltRun {
    store: TurnStore,
    scopes: Vec<TurnScope>,
    run_id: TurnRunId,
    runner_id: TurnRunnerId,
    lease_token: TurnLeaseToken,
    gate_ref: GateRef,
}

async fn build_run_in_status(status: TurnStatus) -> BuiltRun {
    let store = ironclaw_turns::test_support::in_memory_turn_state_store();
    let scopes = scopes();
    let mut model = ReferenceModel::default();
    let run_id = TurnRunId::new();
    submit_ok(&store, &mut model, &scopes, 0, run_id, "state-op-submit").await;

    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    let gate_ref = gate_ref("state-op");
    if status == TurnStatus::Cancelled {
        store
            .request_cancel(cancel_request(scopes[0].clone(), run_id, "state-op-cancel"))
            .await
            .unwrap();
        model.request_cancel(run_id);
        assert_store_matches_model(&store, &model, &scopes, "state-op-built-cancelled").await;
        return BuiltRun {
            store,
            scopes,
            run_id,
            runner_id,
            lease_token,
            gate_ref,
        };
    }
    if status == TurnStatus::Queued {
        return BuiltRun {
            store,
            scopes,
            run_id,
            runner_id,
            lease_token,
            gate_ref,
        };
    }

    claim_ok(
        &store,
        &mut model,
        &scopes,
        0,
        runner_id,
        lease_token,
        "state-op-claim",
    )
    .await;
    if status == TurnStatus::Running {
        return BuiltRun {
            store,
            scopes,
            run_id,
            runner_id,
            lease_token,
            gate_ref,
        };
    }

    match status {
        TurnStatus::BlockedApproval => {
            let checkpoint_id = TurnCheckpointId::new();
            store
                .block_run(BlockRunRequest {
                    run_id,
                    runner_id,
                    lease_token,
                    checkpoint_id,
                    state_ref: state_ref("state-op-block"),
                    reason: BlockedReason::Approval {
                        gate_ref: gate_ref.clone(),
                    },
                })
                .await
                .unwrap();
            model.block(
                run_id,
                TurnStatus::BlockedApproval,
                checkpoint_id,
                gate_ref.clone(),
            );
        }
        TurnStatus::CancelRequested => {
            store
                .request_cancel(cancel_request(
                    scopes[0].clone(),
                    run_id,
                    "state-op-cancel-running",
                ))
                .await
                .unwrap();
            model.request_cancel(run_id);
        }
        TurnStatus::Completed => {
            store
                .complete_run(CompleteRunRequest {
                    run_id,
                    runner_id,
                    lease_token,
                })
                .await
                .unwrap();
            model.complete(run_id);
        }
        TurnStatus::Failed => {
            store
                .fail_run(FailRunRequest {
                    run_id,
                    runner_id,
                    lease_token,
                    failure: SanitizedFailure::new("state_op_failure").unwrap(),
                })
                .await
                .unwrap();
            model.fail(run_id);
        }
        TurnStatus::BlockedAuth
        | TurnStatus::BlockedResource
        | TurnStatus::BlockedDependentRun
        | TurnStatus::BlockedExternalTool
        | TurnStatus::RecoveryRequired
        | TurnStatus::Cancelled
        | TurnStatus::Queued
        | TurnStatus::Running => {
            panic!("state-op fixture does not build {status:?}");
        }
    }
    assert_store_matches_model(&store, &model, &scopes, "state-op-built").await;
    BuiltRun {
        store,
        scopes,
        run_id,
        runner_id,
        lease_token,
        gate_ref,
    }
}

#[derive(Debug, Clone, Copy)]
enum InvalidTurnOp {
    ClaimNoQueued,
    Heartbeat,
    Block,
    Resume,
    Complete,
    Fail,
    SubmitBusyScope,
    ExpireLeaseNoop,
}

async fn apply_invalid_turn_op(fixture: &BuiltRun, op: InvalidTurnOp, label: &str) {
    match op {
        InvalidTurnOp::ClaimNoQueued => {
            let claimed = fixture
                .store
                .claim_next_run(ClaimRunRequest {
                    runner_id: TurnRunnerId::new(),
                    lease_token: TurnLeaseToken::new(),
                    scope_filter: Some(fixture.scopes[0].clone()),
                })
                .await
                .unwrap();
            assert!(claimed.is_none());
        }
        InvalidTurnOp::Heartbeat => {
            assert!(
                fixture
                    .store
                    .heartbeat(HeartbeatRequest {
                        run_id: fixture.run_id,
                        runner_id: fixture.runner_id,
                        lease_token: fixture.lease_token,
                    })
                    .await
                    .is_err()
            );
        }
        InvalidTurnOp::Block => {
            assert!(
                fixture
                    .store
                    .block_run(BlockRunRequest {
                        run_id: fixture.run_id,
                        runner_id: fixture.runner_id,
                        lease_token: fixture.lease_token,
                        checkpoint_id: TurnCheckpointId::new(),
                        state_ref: state_ref("invalid-block"),
                        reason: BlockedReason::Approval {
                            gate_ref: gate_ref("invalid-block"),
                        },
                    })
                    .await
                    .is_err()
            );
        }
        InvalidTurnOp::Resume => {
            assert!(
                fixture
                    .store
                    .resume_turn(resume_request(
                        fixture.scopes[0].clone(),
                        fixture.run_id,
                        fixture.gate_ref.clone(),
                        "invalid-resume",
                    ))
                    .await
                    .is_err()
            );
        }
        InvalidTurnOp::Complete => {
            assert!(
                fixture
                    .store
                    .complete_run(CompleteRunRequest {
                        run_id: fixture.run_id,
                        runner_id: fixture.runner_id,
                        lease_token: fixture.lease_token,
                    })
                    .await
                    .is_err()
            );
        }
        InvalidTurnOp::Fail => {
            assert!(
                fixture
                    .store
                    .fail_run(FailRunRequest {
                        run_id: fixture.run_id,
                        runner_id: fixture.runner_id,
                        lease_token: fixture.lease_token,
                        failure: SanitizedFailure::new("invalid_state_failure").unwrap(),
                    })
                    .await
                    .is_err()
            );
        }
        InvalidTurnOp::SubmitBusyScope => {
            assert!(matches!(
                fixture
                    .store
                    .submit_turn(
                        submit_request(
                            fixture.scopes[0].clone(),
                            TurnRunId::new(),
                            "invalid-submit-busy"
                        ),
                        &AllowAllTurnAdmissionPolicy,
                        &InMemoryRunProfileResolver::default(),
                    )
                    .await,
                Err(TurnError::ThreadBusy(_))
            ));
        }
        InvalidTurnOp::ExpireLeaseNoop => {
            let recovered = fixture
                .store
                .recover_expired_leases(RecoverExpiredLeasesRequest {
                    now: Utc::now() + Duration::minutes(10),
                    scope_filter: Some(fixture.scopes[0].clone()),
                })
                .await
                .unwrap();
            assert!(recovered.recovered.is_empty(), "{label}: {recovered:?}");
        }
    }
}

#[tokio::test]
async fn unsupported_turn_state_operations_do_not_mutate_projection() {
    let cases = [
        (
            TurnStatus::Queued,
            &[
                InvalidTurnOp::Heartbeat,
                InvalidTurnOp::Block,
                InvalidTurnOp::Resume,
                InvalidTurnOp::Complete,
                InvalidTurnOp::Fail,
            ][..],
        ),
        (
            TurnStatus::Running,
            &[InvalidTurnOp::Resume, InvalidTurnOp::SubmitBusyScope][..],
        ),
        (
            TurnStatus::BlockedApproval,
            &[
                InvalidTurnOp::ClaimNoQueued,
                InvalidTurnOp::Heartbeat,
                InvalidTurnOp::Block,
                InvalidTurnOp::Complete,
                InvalidTurnOp::Fail,
                InvalidTurnOp::SubmitBusyScope,
            ][..],
        ),
        (
            TurnStatus::CancelRequested,
            &[
                InvalidTurnOp::ClaimNoQueued,
                InvalidTurnOp::Heartbeat,
                InvalidTurnOp::Block,
                InvalidTurnOp::Resume,
                InvalidTurnOp::Complete,
                InvalidTurnOp::Fail,
                InvalidTurnOp::SubmitBusyScope,
            ][..],
        ),
        (
            TurnStatus::Completed,
            &[
                InvalidTurnOp::ClaimNoQueued,
                InvalidTurnOp::Heartbeat,
                InvalidTurnOp::Block,
                InvalidTurnOp::Resume,
                InvalidTurnOp::Complete,
                InvalidTurnOp::Fail,
                InvalidTurnOp::ExpireLeaseNoop,
            ][..],
        ),
        (
            TurnStatus::Failed,
            &[
                InvalidTurnOp::ClaimNoQueued,
                InvalidTurnOp::Heartbeat,
                InvalidTurnOp::Block,
                InvalidTurnOp::Resume,
                InvalidTurnOp::Complete,
                InvalidTurnOp::Fail,
                InvalidTurnOp::ExpireLeaseNoop,
            ][..],
        ),
        (
            TurnStatus::Cancelled,
            &[
                InvalidTurnOp::ClaimNoQueued,
                InvalidTurnOp::Heartbeat,
                InvalidTurnOp::Block,
                InvalidTurnOp::Resume,
                InvalidTurnOp::Complete,
                InvalidTurnOp::Fail,
                InvalidTurnOp::ExpireLeaseNoop,
            ][..],
        ),
    ];

    for (status, ops) in cases {
        for op in ops {
            let fixture = build_run_in_status(status).await;
            let label = format!("status={status:?} op={op:?}");
            assert_projection_unchanged_after(&fixture.store, &label, || async {
                apply_invalid_turn_op(&fixture, *op, &label).await;
            })
            .await;
        }
    }
}

fn turn_store_with_shared_fs(fs: Arc<ScopedFilesystem<InMemoryBackend>>) -> TurnStore {
    TurnStateRowStore::new(fs)
}

#[derive(Debug, Clone)]
enum GeneratedOp {
    Submit { scope_idx: usize },
    Claim { scope_idx: usize },
    RaceClaim { scope_idx: usize },
    Heartbeat { run_id: TurnRunId },
    Block { run_id: TurnRunId, auth: bool },
    Resume { run_id: TurnRunId },
    Cancel { run_id: TurnRunId },
    Complete { run_id: TurnRunId },
    Fail { run_id: TurnRunId },
    ExpireLease { scope_idx: usize },
    CrashAndRecover,
}

struct GeneratedTurnWorld {
    fs: Arc<ScopedFilesystem<InMemoryBackend>>,
    store: TurnStore,
    scopes: Vec<TurnScope>,
    model: ReferenceModel,
    next_idem: u64,
}

impl GeneratedTurnWorld {
    fn new() -> Self {
        let fs = ironclaw_turns::test_support::in_memory_turns_filesystem();
        Self {
            store: turn_store_with_shared_fs(Arc::clone(&fs)),
            fs,
            scopes: scopes(),
            model: ReferenceModel::default(),
            next_idem: 0,
        }
    }

    fn next_label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}-{}", self.next_idem);
        self.next_idem = self.next_idem.saturating_add(1);
        label
    }

    fn generated_ops(&self) -> Vec<GeneratedOp> {
        let mut ops = Vec::new();
        for scope_idx in 0..self.scopes.len() {
            if !self.model.has_active_run_in_scope(scope_idx) {
                ops.push(GeneratedOp::Submit { scope_idx });
            }
        }
        for scope_idx in self.model.queued_scope_indices() {
            ops.push(GeneratedOp::Claim { scope_idx });
            ops.push(GeneratedOp::RaceClaim { scope_idx });
        }
        for run_id in self.model.running_run_ids() {
            ops.push(GeneratedOp::Heartbeat { run_id });
            ops.push(GeneratedOp::Block {
                run_id,
                auth: false,
            });
            ops.push(GeneratedOp::Block { run_id, auth: true });
            ops.push(GeneratedOp::Cancel { run_id });
            ops.push(GeneratedOp::Complete { run_id });
            ops.push(GeneratedOp::Fail { run_id });
        }
        for run_id in self.model.blocked_run_ids() {
            ops.push(GeneratedOp::Resume { run_id });
            ops.push(GeneratedOp::Cancel { run_id });
        }
        for scope_idx in self.model.recoverable_scope_indices() {
            ops.push(GeneratedOp::ExpireLease { scope_idx });
        }
        ops.push(GeneratedOp::CrashAndRecover);
        ops
    }

    async fn apply(&mut self, op: GeneratedOp, label: &str) {
        match op {
            GeneratedOp::Submit { scope_idx } => {
                let run_id = TurnRunId::new();
                let idem = self.next_label("generated-submit");
                submit_ok(
                    &self.store,
                    &mut self.model,
                    &self.scopes,
                    scope_idx,
                    run_id,
                    &idem,
                )
                .await;
            }
            GeneratedOp::Claim { scope_idx } => {
                let runner_id = TurnRunnerId::new();
                let lease_token = TurnLeaseToken::new();
                claim_ok(
                    &self.store,
                    &mut self.model,
                    &self.scopes,
                    scope_idx,
                    runner_id,
                    lease_token,
                    label,
                )
                .await;
            }
            GeneratedOp::RaceClaim { scope_idx } => {
                let runner_a = TurnRunnerId::new();
                let runner_b = TurnRunnerId::new();
                let lease_a = TurnLeaseToken::new();
                let lease_b = TurnLeaseToken::new();
                let request_a = ClaimRunRequest {
                    runner_id: runner_a,
                    lease_token: lease_a,
                    scope_filter: Some(self.scopes[scope_idx].clone()),
                };
                let request_b = ClaimRunRequest {
                    runner_id: runner_b,
                    lease_token: lease_b,
                    scope_filter: Some(self.scopes[scope_idx].clone()),
                };
                let (claimed_a, claimed_b) = tokio::join!(
                    self.store.claim_next_run(request_a),
                    self.store.claim_next_run(request_b)
                );
                let claimed_a = claimed_a.unwrap();
                let claimed_b = claimed_b.unwrap();
                assert!(
                    claimed_a.is_some() ^ claimed_b.is_some(),
                    "{label}: exactly one race claim should win"
                );
                if claimed_a.is_some() {
                    self.model.claim(scope_idx, runner_a, lease_a);
                } else {
                    self.model.claim(scope_idx, runner_b, lease_b);
                }
                assert_store_matches_model(&self.store, &self.model, &self.scopes, label).await;
            }
            GeneratedOp::Heartbeat { run_id } => {
                let run = self.model.run(run_id).clone();
                self.store
                    .heartbeat(HeartbeatRequest {
                        run_id,
                        runner_id: run.runner_id.unwrap(),
                        lease_token: run.lease_token.unwrap(),
                    })
                    .await
                    .unwrap();
                self.model.heartbeat(run_id);
                assert_store_matches_model(&self.store, &self.model, &self.scopes, label).await;
            }
            GeneratedOp::Block { run_id, auth } => {
                let run = self.model.run(run_id).clone();
                let checkpoint_id = TurnCheckpointId::new();
                let gate_ref = gate_ref(&self.next_label("generated-gate"));
                let reason = if auth {
                    BlockedReason::Auth {
                        gate_ref: gate_ref.clone(),
                        credential_requirements: Vec::new(),
                    }
                } else {
                    BlockedReason::Approval {
                        gate_ref: gate_ref.clone(),
                    }
                };
                let state_ref = state_ref(&self.next_label("generated-block"));
                self.store
                    .block_run(BlockRunRequest {
                        run_id,
                        runner_id: run.runner_id.unwrap(),
                        lease_token: run.lease_token.unwrap(),
                        checkpoint_id,
                        state_ref,
                        reason,
                    })
                    .await
                    .unwrap();
                self.model.block(
                    run_id,
                    if auth {
                        TurnStatus::BlockedAuth
                    } else {
                        TurnStatus::BlockedApproval
                    },
                    checkpoint_id,
                    gate_ref,
                );
                assert_store_matches_model(&self.store, &self.model, &self.scopes, label).await;
            }
            GeneratedOp::Resume { run_id } => {
                let run = self.model.run(run_id).clone();
                let idem = self.next_label("generated-resume");
                self.store
                    .resume_turn(resume_request(
                        self.scopes[run.scope_idx].clone(),
                        run_id,
                        run.gate_ref.clone().unwrap(),
                        &idem,
                    ))
                    .await
                    .unwrap();
                self.model.resume(run_id);
                assert_store_matches_model(&self.store, &self.model, &self.scopes, label).await;
            }
            GeneratedOp::Cancel { run_id } => {
                let run = self.model.run(run_id).clone();
                let idem = self.next_label("generated-cancel");
                self.store
                    .request_cancel(cancel_request(
                        self.scopes[run.scope_idx].clone(),
                        run_id,
                        &idem,
                    ))
                    .await
                    .unwrap();
                self.model.request_cancel(run_id);
                assert_store_matches_model(&self.store, &self.model, &self.scopes, label).await;
            }
            GeneratedOp::Complete { run_id } => {
                let run = self.model.run(run_id).clone();
                self.store
                    .complete_run(CompleteRunRequest {
                        run_id,
                        runner_id: run.runner_id.unwrap(),
                        lease_token: run.lease_token.unwrap(),
                    })
                    .await
                    .unwrap();
                self.model.complete(run_id);
                assert_store_matches_model(&self.store, &self.model, &self.scopes, label).await;
            }
            GeneratedOp::Fail { run_id } => {
                let run = self.model.run(run_id).clone();
                self.store
                    .fail_run(FailRunRequest {
                        run_id,
                        runner_id: run.runner_id.unwrap(),
                        lease_token: run.lease_token.unwrap(),
                        failure: SanitizedFailure::new("generated_reference_model_failure")
                            .unwrap(),
                    })
                    .await
                    .unwrap();
                self.model.fail(run_id);
                assert_store_matches_model(&self.store, &self.model, &self.scopes, label).await;
            }
            GeneratedOp::ExpireLease { scope_idx } => {
                let recovered = self
                    .store
                    .recover_expired_leases(RecoverExpiredLeasesRequest {
                        now: Utc::now() + Duration::minutes(10),
                        scope_filter: Some(self.scopes[scope_idx].clone()),
                    })
                    .await
                    .unwrap();
                let expected = self.model.recover_expired_scope(scope_idx);
                assert_eq!(recovered.recovered.len(), expected.len(), "{label}");
                assert_store_matches_model(&self.store, &self.model, &self.scopes, label).await;
            }
            GeneratedOp::CrashAndRecover => {
                self.store.drain().await.unwrap();
                self.store = turn_store_with_shared_fs(Arc::clone(&self.fs));
                assert_store_matches_model(&self.store, &self.model, &self.scopes, label).await;
            }
        }
    }
}

#[tokio::test]
async fn generated_turn_operations_match_reference_model() {
    for seed in 0x510ce_u64..0x510ce + 8 {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut world = GeneratedTurnWorld::new();
        for step in 0..96 {
            let ops = world.generated_ops();
            let op = ops[rng.random_range(0..ops.len())].clone();
            let label = format!("seed={seed} step={step} op={op:?}");
            world.apply(op, &label).await;
        }
        world.store.drain().await.unwrap();
    }
}

#[tokio::test]
async fn turn_lifecycle_matches_reference_model() {
    let store = ironclaw_turns::test_support::in_memory_turn_state_store();
    let scopes = scopes();
    let mut model = ReferenceModel::default();

    let run_a = TurnRunId::new();
    submit_ok(&store, &mut model, &scopes, 0, run_a, "submit-a").await;

    let duplicate = store
        .submit_turn(
            submit_request(scopes[0].clone(), run_a, "submit-a"),
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .unwrap();
    assert!(matches!(
        duplicate,
        SubmitTurnResponse::Accepted {
            run_id,
            status: TurnStatus::Queued,
            ..
        } if run_id == run_a
    ));
    assert_store_matches_model(&store, &model, &scopes, "duplicate-submit-a").await;

    let busy = store
        .submit_turn(
            submit_request(scopes[0].clone(), TurnRunId::new(), "busy-submit-a"),
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await;
    assert!(matches!(busy, Err(TurnError::ThreadBusy(_))));
    assert_store_matches_model(&store, &model, &scopes, "busy-submit-a").await;

    let runner_a1 = TurnRunnerId::new();
    let lease_a1 = TurnLeaseToken::new();
    let claimed_run = claim_ok(
        &store, &mut model, &scopes, 0, runner_a1, lease_a1, "claim-a1",
    )
    .await;
    assert_eq!(claimed_run, run_a);

    store
        .heartbeat(HeartbeatRequest {
            run_id: run_a,
            runner_id: runner_a1,
            lease_token: lease_a1,
        })
        .await
        .unwrap();
    model.heartbeat(run_a);
    assert_store_matches_model(&store, &model, &scopes, "heartbeat-a").await;

    let approval_gate = gate_ref("approval-a");
    let approval_checkpoint = TurnCheckpointId::new();
    store
        .block_run(BlockRunRequest {
            run_id: run_a,
            runner_id: runner_a1,
            lease_token: lease_a1,
            checkpoint_id: approval_checkpoint,
            state_ref: state_ref("approval-a"),
            reason: BlockedReason::Approval {
                gate_ref: approval_gate.clone(),
            },
        })
        .await
        .unwrap();
    model.block(
        run_a,
        TurnStatus::BlockedApproval,
        approval_checkpoint,
        approval_gate.clone(),
    );
    assert_store_matches_model(&store, &model, &scopes, "block-approval-a").await;

    let resume = store
        .resume_turn(resume_request(
            scopes[0].clone(),
            run_a,
            approval_gate.clone(),
            "resume-a",
        ))
        .await
        .unwrap();
    assert_eq!(resume.status, TurnStatus::Queued);
    model.resume(run_a);
    assert_store_matches_model(&store, &model, &scopes, "resume-a").await;

    let replayed_resume = store
        .resume_turn(resume_request(
            scopes[0].clone(),
            run_a,
            approval_gate,
            "resume-a",
        ))
        .await
        .unwrap();
    assert_eq!(replayed_resume.status, TurnStatus::Queued);
    assert_store_matches_model(&store, &model, &scopes, "resume-a-replay").await;

    let runner_a2 = TurnRunnerId::new();
    let lease_a2 = TurnLeaseToken::new();
    claim_ok(
        &store, &mut model, &scopes, 0, runner_a2, lease_a2, "claim-a2",
    )
    .await;

    store
        .complete_run(CompleteRunRequest {
            run_id: run_a,
            runner_id: runner_a2,
            lease_token: lease_a2,
        })
        .await
        .unwrap();
    model.complete(run_a);
    assert_store_matches_model(&store, &model, &scopes, "complete-a").await;

    let run_b = TurnRunId::new();
    submit_ok(&store, &mut model, &scopes, 0, run_b, "submit-b").await;
    let runner_b = TurnRunnerId::new();
    let lease_b = TurnLeaseToken::new();
    claim_ok(&store, &mut model, &scopes, 0, runner_b, lease_b, "claim-b").await;

    let cancel_b = store
        .request_cancel(cancel_request(scopes[0].clone(), run_b, "cancel-b"))
        .await
        .unwrap();
    assert_eq!(cancel_b.status, TurnStatus::CancelRequested);
    model.request_cancel(run_b);
    assert_store_matches_model(&store, &model, &scopes, "cancel-running-b").await;

    let recovered = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc::now() + Duration::minutes(10),
            scope_filter: Some(scopes[0].clone()),
        })
        .await
        .unwrap();
    assert_eq!(recovered.recovered.len(), 1);
    assert_eq!(recovered.recovered[0].status, TurnStatus::Cancelled);
    model.recover_expired_cancel_requested(run_b);
    assert_store_matches_model(&store, &model, &scopes, "recover-cancel-b").await;

    let replayed_cancel = store
        .request_cancel(cancel_request(scopes[0].clone(), run_b, "cancel-b"))
        .await
        .unwrap();
    assert_eq!(replayed_cancel.status, TurnStatus::CancelRequested);
    assert_store_matches_model(&store, &model, &scopes, "cancel-b-replay").await;

    let run_c = TurnRunId::new();
    submit_ok(&store, &mut model, &scopes, 1, run_c, "submit-c").await;
    let cancel_c = store
        .request_cancel(cancel_request(scopes[1].clone(), run_c, "cancel-c"))
        .await
        .unwrap();
    assert_eq!(cancel_c.status, TurnStatus::Cancelled);
    model.request_cancel(run_c);
    assert_store_matches_model(&store, &model, &scopes, "cancel-queued-c").await;

    let run_d = TurnRunId::new();
    submit_ok(&store, &mut model, &scopes, 2, run_d, "submit-d").await;
    let runner_d = TurnRunnerId::new();
    let lease_d = TurnLeaseToken::new();
    claim_ok(&store, &mut model, &scopes, 2, runner_d, lease_d, "claim-d").await;
    let recovered = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc::now() + Duration::minutes(10),
            scope_filter: Some(scopes[2].clone()),
        })
        .await
        .unwrap();
    assert_eq!(recovered.recovered.len(), 1);
    assert_eq!(recovered.recovered[0].status, TurnStatus::Queued);
    model.recover_expired_running(run_d);
    assert_store_matches_model(&store, &model, &scopes, "recover-checkpointless-d").await;

    let runner_d2 = TurnRunnerId::new();
    let lease_d2 = TurnLeaseToken::new();
    claim_ok(
        &store, &mut model, &scopes, 2, runner_d2, lease_d2, "claim-d2",
    )
    .await;
    store
        .fail_run(FailRunRequest {
            run_id: run_d,
            runner_id: runner_d2,
            lease_token: lease_d2,
            failure: SanitizedFailure::new("reference_model_failure").unwrap(),
        })
        .await
        .unwrap();
    model.fail(run_d);
    assert_store_matches_model(&store, &model, &scopes, "fail-d").await;

    store.drain().await.unwrap();
}

/// Tests for the per-user running-run counter and concurrency cap.
///
/// Covers:
///  - A2: counter tracks per-user across the full lifecycle (complete, fail, block→resume,
///    cancel, lease-expiry, relinquish, apply_validated_loop_exit).
///  - A2 Step 6: snapshot rebuild restores the counter.
///  - A3: claim skips a user at the cap and proceeds with another user / same user after
///    the first run finishes.
use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::test_support::{in_memory_turn_state_store, in_memory_turns_filesystem};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, BlockedReason, GateRef, GetRunStateRequest,
    IdempotencyKey, InMemoryRunProfileResolver, LoopExitMapping, ReplyTargetBindingRef,
    ResumeTurnPrecondition, ResumeTurnRequest, RunProfileRequest, SanitizedCancelReason,
    SanitizedFailure, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnCheckpointId, TurnLeaseToken, TurnRunnerId, TurnScope, TurnStateStore, TurnStatus,
    run_profile::LoopCheckpointStateRef,
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, CompleteRunRequest, FailRunRequest, RecoverExpiredLeasesRequest,
        RelinquishRunRequest, TurnRunTransitionPort, TurnRunnerOutcome,
    },
};
use ironclaw_turns::{FilesystemTurnStateRowStore, TurnStateStoreLimits};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tenant() -> TenantId {
    TenantId::new("tenant-cap-tests").unwrap()
}

fn user_u() -> UserId {
    UserId::new("user-u").unwrap()
}

fn user_v() -> UserId {
    UserId::new("user-v").unwrap()
}

/// Scope owned by `user` on a given thread.
fn owned_scope(thread: &str, owner: &UserId) -> TurnScope {
    TurnScope::new_with_owner(
        tenant(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
        Some(owner.clone()),
    )
}

fn actor_for(user: &UserId) -> TurnActor {
    TurnActor::new(user.clone())
}

fn submit_request_for(scope: TurnScope, key: &str) -> SubmitTurnRequest {
    let actor = actor_for(scope.explicit_owner_user_id().unwrap());
    SubmitTurnRequest {
        requested_model: None,
        actor,
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{key}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(key).unwrap(),
        received_at: Utc.with_ymd_and_hms(2026, 6, 18, 0, 0, 0).unwrap(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
        scope,
    }
}

fn accepted_run_id(resp: &SubmitTurnResponse) -> ironclaw_turns::TurnRunId {
    let SubmitTurnResponse::Accepted { run_id, .. } = resp;
    *run_id
}

fn block_state_ref() -> LoopCheckpointStateRef {
    LoopCheckpointStateRef::new("checkpoint:cap-test-block").unwrap()
}

fn gate_ref_val(s: &str) -> GateRef {
    GateRef::new(s).unwrap()
}

/// The in-memory turn-state store double: a `FilesystemTurnStateRowStore` over
/// a volatile `InMemoryBackend` at the synchronous WriteThrough default, which
/// is behaviorally identical to the former in-memory authority for these tests.
type TurnStore = FilesystemTurnStateRowStore<InMemoryBackend>;

fn make_store() -> TurnStore {
    // A high, effectively-unlimited per-user cap. Unlike the former live
    // in-memory authority, the row store maintains no incremental running
    // counter: `running_count_for_user` rebuilds a transient engine per read
    // and that rebuild is SKIPPED when no cap is enabled (the counter is only
    // consulted for admission). Enabling a cap keeps the counter faithfully
    // populated across the lifecycle without ever gating (1024 ≫ any test's run
    // count); the uncapped-skip behavior is still asserted by tests that build
    // an explicitly uncapped store.
    in_memory_turn_state_store().with_limits(
        TurnStateStoreLimits::default().set_max_concurrent_runs_per_user(
            std::num::NonZeroU32::new(1024).expect("nonzero cap"),
        ),
    )
}

fn make_capped_store(cap: u32) -> TurnStore {
    in_memory_turn_state_store().with_limits(
        TurnStateStoreLimits::default()
            .set_max_concurrent_runs_per_user(std::num::NonZeroU32::new(cap).expect("nonzero cap")),
    )
}

fn resolver() -> InMemoryRunProfileResolver {
    InMemoryRunProfileResolver::default()
}

async fn submit(store: &TurnStore, scope: TurnScope, key: &str) -> ironclaw_turns::TurnRunId {
    let resp = store
        .submit_turn(
            submit_request_for(scope, key),
            &AllowAllTurnAdmissionPolicy,
            &resolver(),
        )
        .await
        .unwrap();
    accepted_run_id(&resp)
}

async fn claim(store: &TurnStore) -> (TurnRunnerId, TurnLeaseToken) {
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    (runner_id, lease_token)
}

// ---------------------------------------------------------------------------
// Task A2 — counter tracks per-user across the full lifecycle
// ---------------------------------------------------------------------------

/// Basic submit + claim increments; complete (terminal) decrements → 0.
#[tokio::test]
async fn running_counter_tracks_per_user_across_lifecycle() {
    let store = make_store();
    let scope = owned_scope("cap-lifecycle-basic", &user_u());

    let run_id = submit(&store, scope.clone(), "cap-lifecycle-basic").await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );

    let (runner_id, lease_token) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
}

/// Fail path decrements.
#[tokio::test]
async fn running_counter_decrements_on_fail() {
    let store = make_store();
    let scope = owned_scope("cap-fail", &user_u());
    let run_id = submit(&store, scope, "cap-fail").await;

    let (runner_id, lease_token) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    store
        .fail_run(FailRunRequest {
            run_id,
            runner_id,
            lease_token,
            failure: SanitizedFailure::new("test_failure").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
}

/// Block (Running → Blocked) decrements; resume re-queues; re-claim re-increments; complete
/// brings it back to 0.
#[tokio::test]
async fn running_counter_decrements_on_block_and_resets_on_resume() {
    let store = make_store();
    let scope = owned_scope("cap-block-resume", &user_u());
    let run_id = submit(&store, scope.clone(), "cap-block-resume").await;

    let (runner_id, lease_token) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    let gate = gate_ref_val("gate:block-resume");
    store
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id: TurnCheckpointId::new(),
            state_ref: block_state_ref(),
            reason: BlockedReason::Approval {
                gate_ref: gate.clone(),
            },
        })
        .await
        .unwrap();
    // After block, counter drops to 0.
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );

    // Resume re-queues the run.
    store
        .resume_turn(ResumeTurnRequest {
            scope: scope.clone(),
            actor: actor_for(&user_u()),
            run_id,
            gate_resolution_ref: gate,
            source_binding_ref: SourceBindingRef::new("source-web-resumed").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web-resumed").unwrap(),
            idempotency_key: IdempotencyKey::new("cap-block-resume-res").unwrap(),
            precondition: ResumeTurnPrecondition::AnyBlockedGate,
            resume_disposition: None,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );

    // Re-claim increments again.
    let (runner_id2, lease_token2) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id: runner_id2,
            lease_token: lease_token2,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
}

/// Running → CancelRequested → Cancelled (runner-held cancel completion) decrements once.
#[tokio::test]
async fn running_counter_decrements_on_cancel_completion() {
    let store = make_store();
    let scope = owned_scope("cap-cancel-complete", &user_u());
    let run_id = submit(&store, scope.clone(), "cap-cancel-complete").await;

    let (runner_id, lease_token) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    // Request cancel: Running → CancelRequested. Runner still holds the lease.
    store
        .request_cancel(ironclaw_turns::CancelRunRequest {
            scope: scope.clone(),
            actor: actor_for(&user_u()),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cap-cancel-complete-req").unwrap(),
        })
        .await
        .unwrap();
    // Counter stays at 1: the runner still holds the run.
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    // Runner completes the cancellation (CancelRequested → Cancelled).
    store
        .cancel_run(CancelRunCompletionRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    // Fully cancelled: counter drops to 0.
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
}

/// Lease expiry of a checkpoint-less run: Running → Queued (re-drivable, #6284).
/// Leaving `Running` releases the concurrency slot, so the counter drops to 0.
#[tokio::test]
async fn running_counter_decrements_on_lease_expiry() {
    let store = make_store();
    let scope = owned_scope("cap-lease-expiry", &user_u());
    submit(&store, scope.clone(), "cap-lease-expiry").await;

    let (_runner_id, _lease_token) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    // Expire the lease by advancing time far into the future.
    store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc::now() + ChronoDuration::seconds(300),
            scope_filter: None,
        })
        .await
        .unwrap();

    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
}

/// Relinquish: Running → Queued. Counter decrements; re-claim re-increments; complete → 0.
#[tokio::test]
async fn running_counter_decrements_on_relinquish() {
    let store = make_store();
    let scope = owned_scope("cap-relinquish", &user_u());
    let run_id = submit(&store, scope.clone(), "cap-relinquish").await;

    let (runner_id, lease_token) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    store
        .relinquish_run(RelinquishRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    // After relinquish (Running → Queued), counter drops to 0.
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );

    // Run is re-queued; claiming it again should bring count back to 1.
    let (runner_id2, lease_token2) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id: runner_id2,
            lease_token: lease_token2,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
}

/// apply_validated_loop_exit → Completed path decrements.
#[tokio::test]
async fn running_counter_decrements_via_apply_validated_loop_exit_completed() {
    let store = make_store();
    let scope = owned_scope("cap-loop-exit-complete", &user_u());
    let run_id = submit(&store, scope, "cap-loop-exit-complete").await;

    let (runner_id, lease_token) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    store
        .apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
            model_usage: None,
            run_id,
            runner_id,
            lease_token,
            mapping: LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed),
        })
        .await
        .unwrap();

    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
}

/// apply_validated_loop_exit → Cancelled path (via CancelRequested) decrements.
#[tokio::test]
async fn running_counter_decrements_via_apply_validated_loop_exit_cancelled() {
    let store = make_store();
    let scope = owned_scope("cap-loop-exit-cancel", &user_u());
    let run_id = submit(&store, scope.clone(), "cap-loop-exit-cancel").await;

    let (runner_id, lease_token) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    // Running → CancelRequested (runner still holds slot).
    store
        .request_cancel(ironclaw_turns::CancelRunRequest {
            scope: scope.clone(),
            actor: actor_for(&user_u()),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cap-loop-exit-cancel-req").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    // apply_validated_loop_exit with Cancelled routes through cancel_or_fail_claimed_record
    // which calls cancel_claimed_record (CancelRequested → Cancelled).
    store
        .apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
            model_usage: None,
            run_id,
            runner_id,
            lease_token,
            mapping: LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Cancelled),
        })
        .await
        .unwrap();

    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
}

// ---------------------------------------------------------------------------
// Task A2 Step 6 — snapshot rebuild restores running_by_user
// ---------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_rebuild_restores_running_counter() {
    // Reopen over the same durable rows (WriteThrough rehydration replaced the
    // old `from_persistence_snapshot` reload): `store` persisted the Queued run,
    // and a second store over the same backend rehydrates it. Both enable a cap
    // so the row store's per-read counter rebuild is populated.
    let cap = || {
        TurnStateStoreLimits::default()
            .set_max_concurrent_runs_per_user(std::num::NonZeroU32::new(1024).expect("nonzero cap"))
    };
    let scoped = in_memory_turns_filesystem();
    let store = FilesystemTurnStateRowStore::new(scoped.clone()).with_limits(cap());
    let scope = owned_scope("cap-snapshot", &user_u());
    let run_id = submit(&store, scope.clone(), "cap-snapshot").await;

    // Reopen BEFORE claiming (run is still Queued).
    let restored = FilesystemTurnStateRowStore::new(scoped.clone()).with_limits(cap());

    // Counter is 0 before claim.
    assert_eq!(
        restored
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );

    // Claim in the restored store → counter goes to 1.
    let (runner_id, lease_token) = claim(&restored).await;
    assert_eq!(
        restored
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    // Complete → counter drops to 0.
    restored
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    assert_eq!(
        restored
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
}

// ---------------------------------------------------------------------------
// Task A3 — cap enforcement in claim selection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn claim_skips_user_at_concurrency_cap() {
    // cap = 1 for user U
    let store = make_capped_store(1);

    // Submit two runs for user U on different threads.
    let scope1 = owned_scope("cap-skip-u-thread1", &user_u());
    let scope2 = owned_scope("cap-skip-u-thread2", &user_u());
    // Submit a run for user V.
    let scope_v = owned_scope("cap-skip-v-thread1", &user_v());

    let run1 = submit(&store, scope1.clone(), "cap-skip-u1").await;
    let run2 = submit(&store, scope2.clone(), "cap-skip-u2").await;
    let run_v = submit(&store, scope_v.clone(), "cap-skip-v1").await;

    // First claim → run1 (U, thread1). U now at cap 1.
    let runner1 = TurnRunnerId::new();
    let lease1 = TurnLeaseToken::new();
    let claimed1 = store
        .claim_next_run(ClaimRunRequest {
            runner_id: runner1,
            lease_token: lease1,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed1.state.run_id, run1);
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    // Second claim with no scope filter → should skip U's run2 (U at cap) and return V's run.
    let runner2 = TurnRunnerId::new();
    let lease2 = TurnLeaseToken::new();
    let claimed2 = store
        .claim_next_run(ClaimRunRequest {
            runner_id: runner2,
            lease_token: lease2,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed2.state.run_id, run_v);
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_v())
            .await
            .unwrap(),
        1
    );

    // Third claim → should still skip U's run2 (still capped), nothing left for V → None.
    let runner3 = TurnRunnerId::new();
    let lease3 = TurnLeaseToken::new();
    let claimed3 = store
        .claim_next_run(ClaimRunRequest {
            runner_id: runner3,
            lease_token: lease3,
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(claimed3.is_none(), "U is capped, V has no more queued runs");

    // run2 is still Queued.
    let state2 = store
        .get_run_state(GetRunStateRequest {
            run_id: run2,
            scope: scope2.clone(),
        })
        .await
        .unwrap();
    assert_eq!(state2.status, TurnStatus::Queued);

    // Complete run1 → U is below cap again.
    store
        .complete_run(CompleteRunRequest {
            run_id: run1,
            runner_id: runner1,
            lease_token: lease1,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );

    // Now claim again → should get run2 (U no longer capped).
    let runner4 = TurnRunnerId::new();
    let lease4 = TurnLeaseToken::new();
    let claimed4 = store
        .claim_next_run(ClaimRunRequest {
            runner_id: runner4,
            lease_token: lease4,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed4.state.run_id, run2);
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    // Clean up.
    store
        .complete_run(CompleteRunRequest {
            run_id: run2,
            runner_id: runner4,
            lease_token: lease4,
        })
        .await
        .unwrap();
    store
        .complete_run(CompleteRunRequest {
            run_id: run_v,
            runner_id: runner2,
            lease_token: lease2,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_v())
            .await
            .unwrap(),
        0
    );
}

/// Truly ownerless (`TurnThreadOwner::Ownerless`) runs are never counted against any cap.
#[tokio::test]
async fn ownerless_runs_are_not_counted_against_cap() {
    let store = make_capped_store(1);
    let actor = TurnActor::new(user_u());

    // Two ownerless scopes (Ownerless — owner_user_id = None).
    let ownerless_scope1 = TurnScope::new_with_owner(
        tenant(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("cap-ownerless-thread1").unwrap(),
        None, // Ownerless
    );
    let ownerless_scope2 = TurnScope::new_with_owner(
        tenant(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("cap-ownerless-thread2").unwrap(),
        None, // Ownerless
    );

    let make_req = |scope: TurnScope, key: &'static str| SubmitTurnRequest {
        requested_model: None,
        scope,
        actor: actor.clone(),
        accepted_message_ref: AcceptedMessageRef::new(format!("msg-{key}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("src").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("rply").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(key).unwrap(),
        received_at: Utc.with_ymd_and_hms(2026, 6, 18, 0, 0, 0).unwrap(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    };

    let resp1 = store
        .submit_turn(
            make_req(ownerless_scope1, "ownerless-1"),
            &AllowAllTurnAdmissionPolicy,
            &resolver(),
        )
        .await
        .unwrap();
    let _run1 = accepted_run_id(&resp1);
    let resp2 = store
        .submit_turn(
            make_req(ownerless_scope2, "ownerless-2"),
            &AllowAllTurnAdmissionPolicy,
            &resolver(),
        )
        .await
        .unwrap();
    let run2 = accepted_run_id(&resp2);

    // Claim first (ownerless — not counted against any user cap).
    let (runner1, lease1) = claim(&store).await;

    // Even though cap = 1 and one ownerless run is active, claiming again should succeed
    // because ownerless runs are never capped.
    let runner2 = TurnRunnerId::new();
    let lease2 = TurnLeaseToken::new();
    let claimed2 = store
        .claim_next_run(ClaimRunRequest {
            runner_id: runner2,
            lease_token: lease2,
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(
        claimed2.is_some(),
        "ownerless runs should not be capped even when cap=1"
    );
    let claimed2 = claimed2.unwrap();
    assert_eq!(claimed2.state.run_id, run2);

    // Clean up.
    store
        .complete_run(CompleteRunRequest {
            run_id: claimed2.state.run_id,
            runner_id: runner2,
            lease_token: lease2,
        })
        .await
        .unwrap();
    let _ = (runner1, lease1); // suppress unused warnings
}

/// Actor-fallback runs ARE capped under the submitting actor's user_id.
/// With cap = 1: after run1 is claimed (actor = user_u), claiming run2
/// (also actor = user_u) must be blocked — even though neither scope has
/// an explicit `ExplicitUser` owner.
#[tokio::test]
async fn actor_fallback_runs_are_capped_under_actor_user_id() {
    let store = make_capped_store(1);
    let actor_u = TurnActor::new(user_u());
    let actor_v = TurnActor::new(user_v());

    // Two actor-fallback scopes for user_u (TurnScope::new gives ActorFallback).
    let scope1 = TurnScope::new(
        tenant(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("cap-actor-fallback-thread1").unwrap(),
    );
    let scope2 = TurnScope::new(
        tenant(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("cap-actor-fallback-thread2").unwrap(),
    );
    // One actor-fallback scope for user_v (different actor).
    let scope_v = TurnScope::new(
        tenant(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("cap-actor-fallback-thread-v").unwrap(),
    );

    let make_req = |scope: TurnScope, actor: TurnActor, key: &'static str| SubmitTurnRequest {
        requested_model: None,
        scope,
        actor,
        accepted_message_ref: AcceptedMessageRef::new(format!("msg-{key}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("src").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("rply").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(key).unwrap(),
        received_at: Utc.with_ymd_and_hms(2026, 6, 18, 0, 0, 0).unwrap(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    };

    let resp1 = store
        .submit_turn(
            make_req(scope1, actor_u.clone(), "actor-fallback-u1"),
            &AllowAllTurnAdmissionPolicy,
            &resolver(),
        )
        .await
        .unwrap();
    let run1 = accepted_run_id(&resp1);

    let resp2 = store
        .submit_turn(
            make_req(scope2, actor_u.clone(), "actor-fallback-u2"),
            &AllowAllTurnAdmissionPolicy,
            &resolver(),
        )
        .await
        .unwrap();
    let run2 = accepted_run_id(&resp2);

    let resp_v = store
        .submit_turn(
            make_req(scope_v, actor_v, "actor-fallback-v1"),
            &AllowAllTurnAdmissionPolicy,
            &resolver(),
        )
        .await
        .unwrap();
    let run_v = accepted_run_id(&resp_v);

    // Claim first run for user_u → counter = 1 for user_u.
    let runner1 = TurnRunnerId::new();
    let lease1 = TurnLeaseToken::new();
    let claimed1 = store
        .claim_next_run(ClaimRunRequest {
            runner_id: runner1,
            lease_token: lease1,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed1.state.run_id, run1);
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1,
        "actor-fallback run should be counted under actor's user_id"
    );

    // Second claim — user_u is at cap (1), so run2 is skipped; run_v (user_v) should be claimed.
    let runner2 = TurnRunnerId::new();
    let lease2 = TurnLeaseToken::new();
    let claimed2 = store
        .claim_next_run(ClaimRunRequest {
            runner_id: runner2,
            lease_token: lease2,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        claimed2.state.run_id, run_v,
        "run2 for user_u must be skipped (at cap); run_v for user_v must be claimed"
    );

    // run2 is still queued.
    let state2 = store
        .get_run_state(GetRunStateRequest {
            run_id: run2,
            scope: TurnScope::new(
                tenant(),
                Some(AgentId::new("agent1").unwrap()),
                Some(ProjectId::new("project1").unwrap()),
                ThreadId::new("cap-actor-fallback-thread2").unwrap(),
            ),
        })
        .await
        .unwrap();
    assert_eq!(state2.status, TurnStatus::Queued);

    // Complete run1 → user_u below cap again.
    store
        .complete_run(CompleteRunRequest {
            run_id: run1,
            runner_id: runner1,
            lease_token: lease1,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );

    // Now run2 for user_u can be claimed.
    let runner3 = TurnRunnerId::new();
    let lease3 = TurnLeaseToken::new();
    let claimed3 = store
        .claim_next_run(ClaimRunRequest {
            runner_id: runner3,
            lease_token: lease3,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed3.state.run_id, run2);

    // Clean up.
    store
        .complete_run(CompleteRunRequest {
            run_id: run2,
            runner_id: runner3,
            lease_token: lease3,
        })
        .await
        .unwrap();
    store
        .complete_run(CompleteRunRequest {
            run_id: run_v,
            runner_id: runner2,
            lease_token: lease2,
        })
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Task A2 Step 6b — snapshot rebuild restores non-zero running counter
// ---------------------------------------------------------------------------

/// Snapshot taken WHILE a run is Running → restored store has counter = 1
/// immediately, without needing to claim again.  The restored store is
/// configured WITH a per-user cap so the rebuild loop runs.
#[tokio::test]
async fn snapshot_rebuild_restores_nonzero_running_counter() {
    let scoped = in_memory_turns_filesystem();
    let store = FilesystemTurnStateRowStore::new(scoped.clone()).with_limits(
        TurnStateStoreLimits::default()
            .set_max_concurrent_runs_per_user(std::num::NonZeroU32::new(10).expect("nonzero cap")),
    );
    let scope = owned_scope("cap-snapshot-running", &user_u());
    let run_id = submit(&store, scope.clone(), "cap-snapshot-running").await;

    // Claim → run is now Running, counter = 1.
    let (runner_id, lease_token) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );
    // The claim (Queued -> Running) is non-critical claim churn — drain it so
    // its journal append has landed before the reopen below.
    store.drain().await.expect("drain claim");

    // Reopen WHILE the run is Running, with a cap enabled so the rebuild loop
    // executes — counter must already be 1 without claiming again.
    let restored = FilesystemTurnStateRowStore::new(scoped.clone()).with_limits(
        TurnStateStoreLimits::default()
            .set_max_concurrent_runs_per_user(std::num::NonZeroU32::new(10).expect("nonzero cap")),
    );
    assert_eq!(
        restored
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1,
        "snapshot rebuild must restore non-zero running counter when cap is enabled"
    );

    // Complete in the ORIGINAL store (restored has a detached copy of the lease).
    store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
}

/// When ALL concurrency caps are `None` (the default), restoring from a
/// snapshot containing a Running run must NOT rebuild the per-user / per-origin
/// counters — the rebuild loop is O(records) and is skipped for the uncapped
/// hot path.  The counter stays at 0 because it is never consulted when caps
/// are disabled.
#[tokio::test]
async fn snapshot_rebuild_skips_counter_when_caps_disabled() {
    // Capped `store` (so its counter is populated → 1); a second store reopened
    // over the same backend with ALL caps None exercises the rebuild-skip.
    let scoped = in_memory_turns_filesystem();
    let store = FilesystemTurnStateRowStore::new(scoped.clone()).with_limits(
        TurnStateStoreLimits::default().set_max_concurrent_runs_per_user(
            std::num::NonZeroU32::new(1024).expect("nonzero cap"),
        ),
    );
    let scope = owned_scope("cap-snapshot-nocap", &user_u());
    let run_id = submit(&store, scope.clone(), "cap-snapshot-nocap").await;

    // Claim → run is now Running, live counter = 1.
    let (runner_id, lease_token) = claim(&store).await;
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        1
    );

    // Reopen WHILE the run is Running with ALL caps None — the per-read rebuild
    // must be skipped, so the uncapped counter reads 0.
    let restored = FilesystemTurnStateRowStore::new(scoped.clone());
    assert_eq!(
        restored
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0,
        "counter rebuild must be skipped when all concurrency caps are None"
    );

    // Complete in the original store.
    store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .running_count_for_user(&tenant(), &user_u())
            .await
            .unwrap(),
        0
    );
}

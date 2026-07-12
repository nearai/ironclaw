/// Tests for the per-origin-class running-run counter and concurrency cap.
///
/// Covers:
///  - B1: trigger counter tracks across the full lifecycle (complete, fail, block→resume,
///    cancel, lease-expiry, relinquish, apply_validated_loop_exit).
///  - B2: origin counter returns to 0 after complete AND after block→resume (proving the
///    funnel covers block + requeue, not just terminal transitions).
///  - B3: claim skips trigger runs at the trigger cap but proceeds with conversation runs.
///  - B4: claim skips conversation runs at the conversation cap but proceeds with trigger runs.
///  - B5: runs without product_context are never counted by origin class.
///  - B6: snapshot rebuild restores running_by_origin_class.
use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, BlockedReason, GateRef, GetRunStateRequest,
    IdempotencyKey, InMemoryRunProfileResolver, InMemoryTurnStateStore,
    InMemoryTurnStateStoreLimits, LoopExitMapping, ProductTurnContext, ReplyTargetBindingRef,
    ResumeTurnPrecondition, ResumeTurnRequest, RunProfileRequest, SanitizedCancelReason,
    SanitizedFailure, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnCheckpointId, TurnLeaseToken, TurnOriginKind, TurnOwner, TurnRunnerId, TurnScope,
    TurnStateStore, TurnStatus,
    run_profile::LoopCheckpointStateRef,
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, CompleteRunRequest, FailRunRequest, RecoverExpiredLeasesRequest,
        RelinquishRunRequest, TurnRunTransitionPort, TurnRunnerOutcome,
    },
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tenant() -> TenantId {
    TenantId::new("tenant-origin-cap-tests").unwrap()
}

fn tenant2() -> TenantId {
    TenantId::new("tenant-origin-cap-tests-t2").unwrap()
}

fn user_u() -> UserId {
    UserId::new("user-u-origin").unwrap()
}

fn user_v() -> UserId {
    UserId::new("user-v-origin").unwrap()
}

fn scope(thread: &str, owner: &UserId) -> TurnScope {
    TurnScope::new_with_owner(
        tenant(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
        Some(owner.clone()),
    )
}

fn scope_for_tenant(tenant_id: TenantId, thread: &str, owner: &UserId) -> TurnScope {
    TurnScope::new_with_owner(
        tenant_id,
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
        Some(owner.clone()),
    )
}

fn actor_for(user: &UserId) -> TurnActor {
    TurnActor::new(user.clone())
}

fn trigger_context(user: &UserId) -> ProductTurnContext {
    ProductTurnContext::new(
        TurnOriginKind::ScheduledTrigger,
        None,
        None,
        TurnOwner::Personal { user: user.clone() },
    )
}

fn conversation_context(user: &UserId) -> ProductTurnContext {
    ProductTurnContext::new(
        TurnOriginKind::Inbound,
        None,
        None,
        TurnOwner::Personal { user: user.clone() },
    )
}

fn submit_request(
    scope: TurnScope,
    key: &str,
    product_context: Option<ProductTurnContext>,
) -> SubmitTurnRequest {
    let owner = scope.explicit_owner_user_id().unwrap().clone();
    SubmitTurnRequest {
        actor: actor_for(&owner),
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
        product_context,
        scope,
    }
}

fn resolver() -> InMemoryRunProfileResolver {
    InMemoryRunProfileResolver::default()
}

fn make_trigger_capped_store(cap: u32) -> InMemoryTurnStateStore {
    InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits::default()
            .set_max_concurrent_trigger_runs(std::num::NonZeroU32::new(cap).expect("nonzero cap")),
    )
}

fn make_conversation_capped_store(cap: u32) -> InMemoryTurnStateStore {
    InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits::default().set_max_concurrent_conversation_runs(
            std::num::NonZeroU32::new(cap).expect("nonzero cap"),
        ),
    )
}

fn accepted_run_id(resp: &SubmitTurnResponse) -> ironclaw_turns::TurnRunId {
    let SubmitTurnResponse::Accepted { run_id, .. } = resp;
    *run_id
}

fn block_state_ref() -> LoopCheckpointStateRef {
    LoopCheckpointStateRef::new("checkpoint:origin-cap-test-block").unwrap()
}

fn gate_ref_val(s: &str) -> GateRef {
    GateRef::new(s).unwrap()
}

async fn submit_with_context(
    store: &InMemoryTurnStateStore,
    scope: TurnScope,
    key: &str,
    ctx: Option<ProductTurnContext>,
) -> ironclaw_turns::TurnRunId {
    let resp = store
        .submit_turn(
            submit_request(scope, key, ctx),
            &AllowAllTurnAdmissionPolicy,
            &resolver(),
        )
        .await
        .unwrap();
    accepted_run_id(&resp)
}

async fn submit_trigger(
    store: &InMemoryTurnStateStore,
    thread: &str,
    key: &str,
) -> ironclaw_turns::TurnRunId {
    let s = scope(thread, &user_u());
    let ctx = trigger_context(&user_u());
    submit_with_context(store, s, key, Some(ctx)).await
}

async fn submit_conversation(
    store: &InMemoryTurnStateStore,
    thread: &str,
    user: &UserId,
    key: &str,
) -> ironclaw_turns::TurnRunId {
    let s = scope(thread, user);
    let ctx = conversation_context(user);
    submit_with_context(store, s, key, Some(ctx)).await
}

async fn claim(
    store: &InMemoryTurnStateStore,
) -> (TurnRunnerId, TurnLeaseToken, ironclaw_turns::TurnRunId) {
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    (runner_id, lease_token, claimed.state.run_id)
}

// ---------------------------------------------------------------------------
// B1 / B2 — trigger counter tracks across the full lifecycle
// ---------------------------------------------------------------------------

/// Trigger counter increments on claim, decrements on complete.
#[tokio::test]
async fn trigger_counter_tracks_complete() {
    let store = InMemoryTurnStateStore::default();
    let run_id = submit_trigger(&store, "orig-complete", "orig-complete").await;

    assert_eq!(store.running_trigger_count(&tenant()), 0);
    let (runner_id, lease_token, _) = claim(&store).await;
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    assert_eq!(store.running_trigger_count(&tenant()), 0);
}

/// Trigger counter decrements on fail.
#[tokio::test]
async fn trigger_counter_tracks_fail() {
    let store = InMemoryTurnStateStore::default();
    let run_id = submit_trigger(&store, "orig-fail", "orig-fail").await;

    let (runner_id, lease_token, _) = claim(&store).await;
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    store
        .fail_run(FailRunRequest {
            run_id,
            runner_id,
            lease_token,
            failure: SanitizedFailure::new("test_failure").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(store.running_trigger_count(&tenant()), 0);
}

/// Trigger counter decrements on block (Running → Blocked) and returns to 0.
/// After resume + re-claim it increments back to 1. Proves the funnel covers block.
#[tokio::test]
async fn trigger_counter_decrements_on_block_and_resets_on_resume() {
    let store = InMemoryTurnStateStore::default();
    let s = scope("orig-block-resume", &user_u());
    let run_id = submit_with_context(
        &store,
        s.clone(),
        "orig-block-resume",
        Some(trigger_context(&user_u())),
    )
    .await;

    let (runner_id, lease_token, _) = claim(&store).await;
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    let gate = gate_ref_val("gate:orig-block-resume");
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
    // Counter drops to 0 after block.
    assert_eq!(store.running_trigger_count(&tenant()), 0);

    // Resume re-queues.
    store
        .resume_turn(ResumeTurnRequest {
            scope: s.clone(),
            actor: actor_for(&user_u()),
            run_id,
            gate_resolution_ref: gate,
            source_binding_ref: SourceBindingRef::new("src-resumed").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("rply-resumed").unwrap(),
            idempotency_key: IdempotencyKey::new("orig-block-resume-res").unwrap(),
            precondition: ResumeTurnPrecondition::AnyBlockedGate,
            resume_disposition: None,
        })
        .await
        .unwrap();
    assert_eq!(store.running_trigger_count(&tenant()), 0);

    // Re-claim increments back.
    let (runner_id2, lease_token2, _) = claim(&store).await;
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id: runner_id2,
            lease_token: lease_token2,
        })
        .await
        .unwrap();
    assert_eq!(store.running_trigger_count(&tenant()), 0);
}

/// Trigger counter decrements on cancel completion (CancelRequested → Cancelled).
#[tokio::test]
async fn trigger_counter_decrements_on_cancel_completion() {
    let store = InMemoryTurnStateStore::default();
    let s = scope("orig-cancel", &user_u());
    let run_id = submit_with_context(
        &store,
        s.clone(),
        "orig-cancel",
        Some(trigger_context(&user_u())),
    )
    .await;

    let (runner_id, lease_token, _) = claim(&store).await;
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    // Running → CancelRequested. Runner still holds the slot.
    store
        .request_cancel(ironclaw_turns::CancelRunRequest {
            scope: s.clone(),
            actor: actor_for(&user_u()),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("orig-cancel-req").unwrap(),
        })
        .await
        .unwrap();
    // Slot still held (Running → CancelRequested is Unchanged).
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    // Runner completes cancellation: CancelRequested → Cancelled.
    store
        .cancel_run(CancelRunCompletionRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    assert_eq!(store.running_trigger_count(&tenant()), 0);
}

/// Trigger counter decrements on lease expiry.
#[tokio::test]
async fn trigger_counter_decrements_on_lease_expiry() {
    let store = InMemoryTurnStateStore::default();
    submit_trigger(&store, "orig-lease-expiry", "orig-lease-expiry").await;

    let _ = claim(&store).await;
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc::now() + ChronoDuration::seconds(300),
            scope_filter: None,
        })
        .await
        .unwrap();

    assert_eq!(store.running_trigger_count(&tenant()), 0);
}

/// Trigger counter decrements on relinquish (Running → Queued), re-increments on re-claim.
#[tokio::test]
async fn trigger_counter_decrements_on_relinquish() {
    let store = InMemoryTurnStateStore::default();
    let run_id = submit_trigger(&store, "orig-relinquish", "orig-relinquish").await;

    let (runner_id, lease_token, _) = claim(&store).await;
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    store
        .relinquish_run(RelinquishRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    assert_eq!(store.running_trigger_count(&tenant()), 0);

    // Re-queue claim.
    let (runner_id2, lease_token2, _) = claim(&store).await;
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id: runner_id2,
            lease_token: lease_token2,
        })
        .await
        .unwrap();
    assert_eq!(store.running_trigger_count(&tenant()), 0);
}

/// apply_validated_loop_exit → Completed decrements trigger counter.
#[tokio::test]
async fn trigger_counter_decrements_via_apply_validated_loop_exit() {
    let store = InMemoryTurnStateStore::default();
    let run_id = submit_trigger(&store, "orig-loop-exit", "orig-loop-exit").await;

    let (runner_id, lease_token, _) = claim(&store).await;
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    store
        .apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
            run_id,
            runner_id,
            lease_token,
            mapping: LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed),
        })
        .await
        .unwrap();

    assert_eq!(store.running_trigger_count(&tenant()), 0);
}

/// apply_validated_loop_exit → Cancelled path (via CancelRequested) decrements trigger counter.
#[tokio::test]
async fn trigger_counter_decrements_via_apply_validated_loop_exit_cancelled() {
    let store = InMemoryTurnStateStore::default();
    let s = scope("orig-loop-exit-cancel", &user_u());
    let run_id = submit_with_context(
        &store,
        s.clone(),
        "orig-loop-exit-cancel",
        Some(trigger_context(&user_u())),
    )
    .await;

    let (runner_id, lease_token, _) = claim(&store).await;
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    // Running → CancelRequested.
    store
        .request_cancel(ironclaw_turns::CancelRunRequest {
            scope: s.clone(),
            actor: actor_for(&user_u()),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("orig-loop-exit-cancel-req").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    // apply_validated_loop_exit Cancelled → cancel_or_fail_claimed_record → cancel_claimed_record.
    store
        .apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
            run_id,
            runner_id,
            lease_token,
            mapping: LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Cancelled),
        })
        .await
        .unwrap();

    assert_eq!(store.running_trigger_count(&tenant()), 0);
}

// ---------------------------------------------------------------------------
// B3 — trigger cap enforcement: trigger runs blocked, conversation proceeds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn trigger_runs_capped_while_conversation_proceeds() {
    let store = make_trigger_capped_store(1);

    // Submit two trigger runs and one conversation run.
    let trigger1 = submit_trigger(&store, "tri-cap-t1", "tri-cap-t1").await;
    let trigger2 = submit_trigger(&store, "tri-cap-t2", "tri-cap-t2").await;
    let conv = submit_conversation(&store, "tri-cap-cv", &user_v(), "tri-cap-cv").await;

    // Claim first → trigger1. Trigger counter = 1 = cap.
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
    assert_eq!(claimed1.state.run_id, trigger1);
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    // Second claim → trigger2 is skipped (cap hit), conversation is returned.
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
    assert_eq!(claimed2.state.run_id, conv);
    assert_eq!(store.running_trigger_count(&tenant()), 1);
    assert_eq!(store.running_conversation_count(&tenant()), 1);

    // Third claim → trigger2 still blocked, conv exhausted → None.
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
    assert!(claimed3.is_none());

    // trigger2 is still Queued.
    let scope2 = scope("tri-cap-t2", &user_u());
    let state2 = store
        .get_run_state(GetRunStateRequest {
            run_id: trigger2,
            scope: scope2,
        })
        .await
        .unwrap();
    assert_eq!(state2.status, TurnStatus::Queued);

    // Complete trigger1 → trigger counter drops below cap.
    store
        .complete_run(CompleteRunRequest {
            run_id: trigger1,
            runner_id: runner1,
            lease_token: lease1,
        })
        .await
        .unwrap();
    assert_eq!(store.running_trigger_count(&tenant()), 0);

    // Now claim succeeds for trigger2.
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
    assert_eq!(claimed4.state.run_id, trigger2);
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    // Clean up.
    store
        .complete_run(CompleteRunRequest {
            run_id: trigger2,
            runner_id: runner4,
            lease_token: lease4,
        })
        .await
        .unwrap();
    store
        .complete_run(CompleteRunRequest {
            run_id: conv,
            runner_id: runner2,
            lease_token: lease2,
        })
        .await
        .unwrap();
    assert_eq!(store.running_trigger_count(&tenant()), 0);
    assert_eq!(store.running_conversation_count(&tenant()), 0);
}

// ---------------------------------------------------------------------------
// B4 — conversation cap: conversation runs blocked, trigger proceeds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conversation_runs_capped_while_triggers_proceed() {
    let store = make_conversation_capped_store(1);

    // Submit two conversation runs and one trigger run.
    let conv1 = submit_conversation(&store, "conv-cap-c1", &user_u(), "conv-cap-c1").await;
    let conv2 = submit_conversation(&store, "conv-cap-c2", &user_u(), "conv-cap-c2").await;
    let trig = submit_trigger(&store, "conv-cap-t1", "conv-cap-t1").await;

    // First claim → conv1. Conversation counter = 1 = cap.
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
    assert_eq!(claimed1.state.run_id, conv1);
    assert_eq!(store.running_conversation_count(&tenant()), 1);

    // Second claim → conv2 is skipped (cap hit), trigger is returned.
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
    assert_eq!(claimed2.state.run_id, trig);
    assert_eq!(store.running_conversation_count(&tenant()), 1);
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    // Third claim → conv2 still blocked, trig exhausted → None.
    let claimed3 = store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(claimed3.is_none());

    // Complete conv1 → conversation drops below cap.
    store
        .complete_run(CompleteRunRequest {
            run_id: conv1,
            runner_id: runner1,
            lease_token: lease1,
        })
        .await
        .unwrap();
    assert_eq!(store.running_conversation_count(&tenant()), 0);

    // Now conv2 can be claimed.
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
    assert_eq!(claimed4.state.run_id, conv2);
    assert_eq!(store.running_conversation_count(&tenant()), 1);

    // Clean up.
    store
        .complete_run(CompleteRunRequest {
            run_id: conv2,
            runner_id: runner4,
            lease_token: lease4,
        })
        .await
        .unwrap();
    store
        .complete_run(CompleteRunRequest {
            run_id: trig,
            runner_id: runner2,
            lease_token: lease2,
        })
        .await
        .unwrap();
    assert_eq!(store.running_trigger_count(&tenant()), 0);
    assert_eq!(store.running_conversation_count(&tenant()), 0);
}

// ---------------------------------------------------------------------------
// B5 — runs without product_context are not capped by origin class
// ---------------------------------------------------------------------------

#[tokio::test]
async fn runs_without_product_context_are_not_capped_by_origin() {
    let store = make_trigger_capped_store(1);

    // Two ownerless / no-product-context runs.
    let plain_scope1 = scope("orig-no-ctx-1", &user_u());
    let plain_scope2 = scope("orig-no-ctx-2", &user_u());
    let run1 = submit_with_context(&store, plain_scope1, "orig-no-ctx-1", None).await;
    let run2 = submit_with_context(&store, plain_scope2, "orig-no-ctx-2", None).await;

    // Claim first — should not affect trigger counter.
    let (runner1, lease1, claimed_run1) = claim(&store).await;
    assert_eq!(claimed_run1, run1);
    assert_eq!(store.running_trigger_count(&tenant()), 0);

    // Even with cap=1, second no-context run should also be claimable (not counted as trigger).
    let (runner2, lease2, claimed_run2) = claim(&store).await;
    assert_eq!(claimed_run2, run2);
    assert_eq!(store.running_trigger_count(&tenant()), 0);

    // Clean up.
    store
        .complete_run(CompleteRunRequest {
            run_id: run1,
            runner_id: runner1,
            lease_token: lease1,
        })
        .await
        .unwrap();
    store
        .complete_run(CompleteRunRequest {
            run_id: run2,
            runner_id: runner2,
            lease_token: lease2,
        })
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// B6 — snapshot rebuild restores running_by_origin_class
// ---------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_rebuild_restores_origin_class_counter() {
    let store = InMemoryTurnStateStore::default();
    let run_id = submit_trigger(&store, "orig-snapshot", "orig-snapshot").await;

    // Snapshot BEFORE claiming (run is still Queued).
    let snapshot = store.persistence_snapshot();

    // Restore from snapshot.
    let restored = InMemoryTurnStateStore::from_persistence_snapshot(
        snapshot,
        InMemoryTurnStateStoreLimits::default(),
    )
    .unwrap();

    // Counter is 0 before claim.
    assert_eq!(restored.running_trigger_count(&tenant()), 0);
    assert_eq!(restored.running_conversation_count(&tenant()), 0);

    // Claim in the restored store → counter goes to 1.
    let (runner_id, lease_token, claimed_run_id) = claim(&restored).await;
    assert_eq!(claimed_run_id, run_id);
    assert_eq!(restored.running_trigger_count(&tenant()), 1);

    // Complete → counter drops to 0.
    restored
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    assert_eq!(restored.running_trigger_count(&tenant()), 0);
}

// ---------------------------------------------------------------------------
// B7 — tenant isolation: T1 at trigger cap does NOT block T2's trigger run
// ---------------------------------------------------------------------------

/// Tenant T1 is at the trigger cap (1). Tenant T2 should still be able to
/// claim and run its own trigger run — the cap is per-tenant, not global.
#[tokio::test]
async fn trigger_cap_is_per_tenant_not_global() {
    let store = make_trigger_capped_store(1);

    let t1 = tenant();
    let t2 = tenant2();
    let user = user_u();

    // Submit one trigger run for T1 and one for T2.
    let t1_scope = scope_for_tenant(t1.clone(), "iso-t1-thread", &user);
    let t2_scope = scope_for_tenant(t2.clone(), "iso-t2-thread", &user);

    let t1_run =
        submit_with_context(&store, t1_scope, "iso-t1-key", Some(trigger_context(&user))).await;
    let t2_run =
        submit_with_context(&store, t2_scope, "iso-t2-key", Some(trigger_context(&user))).await;

    // Claim T1's trigger run → T1 counter = 1 = cap.
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
    assert_eq!(claimed1.state.run_id, t1_run);
    assert_eq!(store.running_trigger_count(&t1), 1);
    assert_eq!(store.running_trigger_count(&t2), 0);

    // T2's trigger run must still be claimable even though T1 is at cap.
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
    assert_eq!(claimed2.state.run_id, t2_run);
    assert_eq!(store.running_trigger_count(&t1), 1);
    assert_eq!(store.running_trigger_count(&t2), 1);

    // Clean up.
    store
        .complete_run(CompleteRunRequest {
            run_id: t1_run,
            runner_id: runner1,
            lease_token: lease1,
        })
        .await
        .unwrap();
    store
        .complete_run(CompleteRunRequest {
            run_id: t2_run,
            runner_id: runner2,
            lease_token: lease2,
        })
        .await
        .unwrap();
    assert_eq!(store.running_trigger_count(&t1), 0);
    assert_eq!(store.running_trigger_count(&t2), 0);
}

// ---------------------------------------------------------------------------
// B6b — snapshot rebuild restores non-zero origin-class counter
// ---------------------------------------------------------------------------

/// Snapshot taken WHILE a trigger run is Running → restored store sees counter = 1
/// immediately, exercising the non-zero rebuild branch in `from_persistence_snapshot`.
/// The restored store is configured WITH a trigger cap so the rebuild loop runs.
#[tokio::test]
async fn snapshot_rebuild_restores_nonzero_origin_class_counter() {
    let store = InMemoryTurnStateStore::default();
    let run_id = submit_trigger(&store, "orig-snapshot-running", "orig-snapshot-running").await;

    // Claim → run is now Running, trigger counter = 1.
    let (runner_id, lease_token, _) = claim(&store).await;
    assert_eq!(store.running_trigger_count(&tenant()), 1);

    // Snapshot WHILE the run is Running.
    let snapshot = store.persistence_snapshot();

    // Restore with a trigger cap enabled so the rebuild loop executes — counter
    // must already be 1 without claiming again.
    let restored = InMemoryTurnStateStore::from_persistence_snapshot(
        snapshot,
        InMemoryTurnStateStoreLimits::default()
            .set_max_concurrent_trigger_runs(std::num::NonZeroU32::new(10).expect("nonzero cap")),
    )
    .unwrap();
    assert_eq!(
        restored.running_trigger_count(&tenant()),
        1,
        "snapshot rebuild must restore non-zero trigger counter when cap is enabled"
    );
    assert_eq!(restored.running_conversation_count(&tenant()), 0);

    // Complete in the original store.
    store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    assert_eq!(store.running_trigger_count(&tenant()), 0);
}

use std::sync::Arc;

use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, EnsureThreadRequest,
    InMemorySessionThreadService, MessageContent, MessageStatus, SessionThreadService,
    ThreadHistoryRequest, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, DefaultTurnCoordinator,
    DefaultTurnLifecycleEventBus, EventCursor, GetRunStateRequest, IdempotencyKey,
    InMemoryTurnStateStore, LifecyclePublishingTurnStateStore, ReplyTargetBindingRef,
    ResumeTurnRequest, ResumeTurnResponse, SanitizedCancelReason, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCommittedEventObserver, TurnCoordinator,
    TurnError, TurnEventKind, TurnLeaseToken, TurnLifecycleEvent, TurnLifecycleEventBus, TurnRunId,
    TurnRunState, TurnRunnerId, TurnScope, TurnStatus,
    runner::{ClaimRunRequest, CompleteRunRequest, TurnRunTransitionPort},
};

use super::DeferredBusyDrainObserver;

// -----------------------------------------------------------------------
// Test harness helpers
// -----------------------------------------------------------------------

fn tenant() -> TenantId {
    TenantId::new("tenant-drain-test").unwrap()
}

fn agent() -> AgentId {
    AgentId::new("agent-drain-test").unwrap()
}

fn actor() -> UserId {
    UserId::new("user-drain-actor").unwrap()
}

fn owner() -> UserId {
    UserId::new("user-drain-owner").unwrap()
}

fn thread_id() -> ThreadId {
    ThreadId::new("thread-drain-test").unwrap()
}

fn thread_scope() -> ThreadScope {
    ThreadScope {
        tenant_id: tenant(),
        agent_id: agent(),
        project_id: None,
        owner_user_id: Some(owner()),
        mission_id: None,
    }
}

fn turn_scope() -> TurnScope {
    TurnScope::new_with_owner(tenant(), Some(agent()), None, thread_id(), Some(owner()))
}

/// Build a reusable coordinator + lifecycle bus + drain observer harness.
///
/// Returns `(coordinator, thread_service, publishing_store)` ready for
/// test assertions. The drain observer is already subscribed and bound.
/// `publishing_store` exposes the runner-transition port for tests that
/// need to drive `claim_next_run` / `complete_run` directly (Scenario C).
async fn build_harness() -> (
    Arc<dyn TurnCoordinator>,
    Arc<InMemorySessionThreadService>,
    Arc<LifecyclePublishingTurnStateStore<InMemoryTurnStateStore>>,
) {
    let thread_service = Arc::new(InMemorySessionThreadService::default());

    // Ensure the test thread exists.
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope(),
            thread_id: Some(thread_id()),
            created_by_actor_id: actor().as_str().to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure thread");

    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let lifecycle_bus = Arc::new(DefaultTurnLifecycleEventBus::new());

    let drain_observer_for_bind = Arc::new(DeferredBusyDrainObserver::new_unbound(Arc::clone(
        &thread_service,
    )
        as Arc<dyn ironclaw_threads::SessionThreadService>));
    let drain_observer: Arc<dyn TurnCommittedEventObserver> =
        Arc::clone(&drain_observer_for_bind) as Arc<dyn TurnCommittedEventObserver>;
    lifecycle_bus
        .subscribe_required(drain_observer)
        .expect("subscribe drain observer");

    let publishing_store = Arc::new(LifecyclePublishingTurnStateStore::new(
        Arc::clone(&turn_store),
        lifecycle_bus,
    ));

    let coordinator: Arc<dyn TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(Arc::clone(&publishing_store)));

    drain_observer_for_bind
        .bind_coordinator(Arc::clone(&coordinator))
        .expect("bind drain coordinator");

    (coordinator, thread_service, publishing_store)
}

/// Submit a turn to the coordinator and return the run id.
async fn submit_run(
    coordinator: &dyn TurnCoordinator,
    idempotency_suffix: &str,
    accepted_message_ref: AcceptedMessageRef,
) -> TurnRunId {
    let response = coordinator
        .submit_turn(SubmitTurnRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            accepted_message_ref,
            source_binding_ref: SourceBindingRef::new("src:binding-drain").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new(format!("turn:drain-test-{idempotency_suffix}"))
                .unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
        })
        .await
        .expect("submit_turn should succeed");
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    run_id
}

/// Accept a user message and return the `AcceptedInboundMessage`.
///
/// Stores `"binding-drain"` as both `source_binding_id` and
/// `reply_target_binding_id`.  Callers that need to defer the message must
/// separately call `mark_message_deferred_busy` with the canonical refs
/// (e.g. `"src:binding-drain"` / `"reply:binding-drain"`).
async fn accept_message(
    thread_service: &InMemorySessionThreadService,
    text: &str,
    external_event_id: &str,
) -> AcceptedInboundMessage {
    thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
            actor_id: actor().as_str().to_string(),
            source_binding_id: Some("binding-drain".to_string()),
            reply_target_binding_id: Some("binding-drain".to_string()),
            external_event_id: Some(external_event_id.to_string()),
            content: MessageContent::text(text),
        })
        .await
        .expect("accept_inbound_message")
}

// -----------------------------------------------------------------------
// Scenario A: deferred message drained on terminal event (coordinator path)
// -----------------------------------------------------------------------

#[tokio::test]
async fn deferred_message_submitted_after_blocking_run_is_cancelled() {
    let (coordinator, thread_service, _) = build_harness().await;

    // Step 1: Accept and submit message A — thread lock acquired.
    let msg_a = accept_message(&thread_service, "message A", "ext-event-a").await;
    let msg_a_ref = AcceptedMessageRef::new(format!("msg:{}", msg_a.message_id)).unwrap();
    let run_a = submit_run(coordinator.as_ref(), "a", msg_a_ref).await;

    // Step 2: Accept message B — coordinator returns ThreadBusy.
    let msg_b = accept_message(&thread_service, "message B", "ext-event-b").await;
    let msg_b_ref = AcceptedMessageRef::new(format!("msg:{}", msg_b.message_id)).unwrap();
    match coordinator
        .submit_turn(SubmitTurnRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            accepted_message_ref: msg_b_ref,
            source_binding_ref: SourceBindingRef::new("src:binding-drain").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("turn:drain-test-b").unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
        })
        .await
    {
        Err(ironclaw_turns::TurnError::ThreadBusy(_)) => {}
        other => panic!("expected ThreadBusy, got {other:?}"),
    }
    thread_service
        .mark_message_deferred_busy(
            &thread_scope(),
            &thread_id(),
            msg_b.message_id,
            Some("src:binding-drain".to_string()),
            Some("reply:binding-drain".to_string()),
        )
        .await
        .expect("mark deferred busy");

    // Verify B is deferred before the drain.
    let history_before = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap();
    let b_before = history_before
        .messages
        .iter()
        .find(|m| m.message_id == msg_b.message_id)
        .expect("message B in history");
    assert_eq!(b_before.status, MessageStatus::DeferredBusy);

    // Step 3: Cancel run A → terminal event → drain fires → B resubmitted.
    coordinator
        .cancel_run(CancelRunRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            run_id: run_a,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cancel:run-a-drain-test").unwrap(),
        })
        .await
        .expect("cancel run A");

    // Step 4: Assert message B is now Submitted.
    let history_after = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap();
    let b_after = history_after
        .messages
        .iter()
        .find(|m| m.message_id == msg_b.message_id)
        .expect("message B in history after drain");
    assert_eq!(
        b_after.status,
        MessageStatus::Submitted,
        "DeferredBusy message must be Submitted after blocking run terminates"
    );
}

// -----------------------------------------------------------------------
// Scenario B: idempotency — drain fired twice → message submitted once
// -----------------------------------------------------------------------

#[tokio::test]
async fn drain_idempotency_second_terminal_event_does_not_double_submit() {
    let (coordinator, thread_service, _) = build_harness().await;

    // Step 1: Accept and submit message A — thread lock acquired.
    let msg_a = accept_message(&thread_service, "message A-idem", "ext-event-a-idem").await;
    let msg_a_ref = AcceptedMessageRef::new(format!("msg:{}", msg_a.message_id)).unwrap();
    let run_a = submit_run(coordinator.as_ref(), "a-idem", msg_a_ref).await;

    // Step 2: Accept B and defer.
    let msg_b = accept_message(&thread_service, "message B-idem", "ext-event-b-idem").await;
    let msg_b_ref = AcceptedMessageRef::new(format!("msg:{}", msg_b.message_id)).unwrap();
    match coordinator
        .submit_turn(SubmitTurnRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            accepted_message_ref: msg_b_ref,
            source_binding_ref: SourceBindingRef::new("src:binding-drain").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("turn:drain-test-b-idem").unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
        })
        .await
    {
        Err(ironclaw_turns::TurnError::ThreadBusy(_)) => {}
        other => panic!("expected ThreadBusy, got {other:?}"),
    }
    thread_service
        .mark_message_deferred_busy(
            &thread_scope(),
            &thread_id(),
            msg_b.message_id,
            Some("src:binding-drain".to_string()),
            Some("reply:binding-drain".to_string()),
        )
        .await
        .expect("mark deferred busy");

    // Step 3: First cancel (fires drain, B → Submitted, new run B_run acquired).
    coordinator
        .cancel_run(CancelRunRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            run_id: run_a,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cancel:run-a-idem-first").unwrap(),
        })
        .await
        .expect("cancel run A (first)");

    let history_mid = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap();
    let b_mid = history_mid
        .messages
        .iter()
        .find(|m| m.message_id == msg_b.message_id)
        .expect("message B in mid history");
    assert_eq!(
        b_mid.status,
        MessageStatus::Submitted,
        "B must be Submitted after first drain"
    );
    let b_run_id_str = b_mid
        .turn_run_id
        .clone()
        .expect("B must have a run id after submission");

    // Step 4: Cancel run B (the submitted run) — fires second drain but B is no
    // longer DeferredBusy so drain returns early (empty list).
    let b_run_id = TurnRunId::from_uuid(uuid::Uuid::parse_str(&b_run_id_str).expect("valid uuid"));
    coordinator
        .cancel_run(CancelRunRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            run_id: b_run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cancel:run-b-idem-second").unwrap(),
        })
        .await
        .expect("cancel run B");

    // B's status must still be Submitted (drain saw empty DeferredBusy list).
    let history_after = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap();
    let b_after = history_after
        .messages
        .iter()
        .find(|m| m.message_id == msg_b.message_id)
        .expect("message B in final history");
    assert_eq!(
        b_after.status,
        MessageStatus::Submitted,
        "B must remain Submitted after second drain (idempotency)"
    );
    assert_eq!(
        b_after.turn_run_id.as_deref(),
        Some(b_run_id_str.as_str()),
        "B's run_id must not change on second drain"
    );
}

// -----------------------------------------------------------------------
// Scenario C: drain fires via publish_state (runner-origin complete_run)
//
// This is the PRODUCTION approval-flow path: the runner calls complete_run
// which goes through publish_state → observe_committed_state (not the
// coordinator cancel_run path that tests A/B use).
// -----------------------------------------------------------------------

#[tokio::test]
async fn deferred_message_submitted_after_blocking_run_completes_via_publish_state() {
    let (coordinator, thread_service, publishing_store) = build_harness().await;

    // Step 1: Accept and submit message A — thread lock acquired.
    let msg_a = accept_message(&thread_service, "message A-state", "ext-event-a-state").await;
    let msg_a_ref = AcceptedMessageRef::new(format!("msg:{}", msg_a.message_id)).unwrap();
    let run_a = submit_run(coordinator.as_ref(), "a-state", msg_a_ref).await;

    // Step 2: Accept message B — mark deferred.
    let msg_b = accept_message(&thread_service, "message B-state", "ext-event-b-state").await;
    let msg_b_ref = AcceptedMessageRef::new(format!("msg:{}", msg_b.message_id)).unwrap();
    match coordinator
        .submit_turn(SubmitTurnRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            accepted_message_ref: msg_b_ref,
            source_binding_ref: SourceBindingRef::new("src:binding-drain").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("turn:drain-test-b-state").unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
        })
        .await
    {
        Err(ironclaw_turns::TurnError::ThreadBusy(_)) => {}
        other => panic!("expected ThreadBusy, got {other:?}"),
    }
    thread_service
        .mark_message_deferred_busy(
            &thread_scope(),
            &thread_id(),
            msg_b.message_id,
            Some("src:binding-drain".to_string()),
            Some("reply:binding-drain".to_string()),
        )
        .await
        .expect("mark deferred busy");

    // Step 3: Claim run A (runner takes ownership).
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    let claimed = TurnRunTransitionPort::claim_next_run(
        publishing_store.as_ref(),
        ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        },
    )
    .await
    .expect("claim run")
    .expect("claim returns Some");
    assert_eq!(claimed.state.run_id, run_a);

    // Step 4: Runner completes run A via publish_state path.
    // This exercises observe_committed_state (NOT observe_committed_event).
    TurnRunTransitionPort::complete_run(
        publishing_store.as_ref(),
        CompleteRunRequest {
            run_id: run_a,
            runner_id,
            lease_token: claimed.lease_token,
        },
    )
    .await
    .expect("complete run A");

    // Step 5: Assert message B is now Submitted.
    let history_after = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap();
    let b_after = history_after
        .messages
        .iter()
        .find(|m| m.message_id == msg_b.message_id)
        .expect("message B in history after state drain");
    assert_eq!(
        b_after.status,
        MessageStatus::Submitted,
        "DeferredBusy message must be Submitted after blocking run completes via publish_state"
    );
}

// -----------------------------------------------------------------------
// Scenario D: head-of-line blocking — invalid first message is skipped
// -----------------------------------------------------------------------

#[tokio::test]
async fn drain_skips_invalid_message_and_submits_next() {
    let (coordinator, thread_service, _) = build_harness().await;

    // Step 1: Accept and submit message A — thread lock acquired.
    let msg_a = accept_message(&thread_service, "message A-hol", "ext-event-a-hol").await;
    let msg_a_ref = AcceptedMessageRef::new(format!("msg:{}", msg_a.message_id)).unwrap();
    let run_a = submit_run(coordinator.as_ref(), "a-hol", msg_a_ref).await;

    // Step 2: Accept message B — mark deferred but inject an invalid
    // actor_id so the drain will fail to resolve it and skip to C.
    let msg_b_id = {
        // Accept without actor initially, then get the raw id
        let accepted = thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope(),
                thread_id: thread_id(),
                actor_id: "user-drain-actor".to_string(),
                source_binding_id: None, // missing — will be skipped for missing binding
                reply_target_binding_id: Some("binding-drain".to_string()),
                external_event_id: Some("ext-event-b-hol".to_string()),
                content: MessageContent::text("message B-hol"),
            })
            .await
            .expect("accept message B");
        thread_service
            .mark_message_deferred_busy(
                &thread_scope(),
                &thread_id(),
                accepted.message_id,
                None, // No canonical refs — simulates legacy/invalid entry that drain skips
                None,
            )
            .await
            .expect("mark B deferred busy");
        accepted.message_id
    };

    // Step 3: Accept message C — valid, should be drained after B is skipped.
    let msg_c = accept_message(&thread_service, "message C-hol", "ext-event-c-hol").await;
    let msg_c_ref = AcceptedMessageRef::new(format!("msg:{}", msg_c.message_id)).unwrap();
    // C also gets ThreadBusy because A still holds the lock.
    match coordinator
        .submit_turn(SubmitTurnRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            accepted_message_ref: msg_c_ref,
            source_binding_ref: SourceBindingRef::new("src:binding-drain").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("turn:drain-test-c-hol").unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
        })
        .await
    {
        Err(ironclaw_turns::TurnError::ThreadBusy(_)) => {}
        other => panic!("expected ThreadBusy for C, got {other:?}"),
    }
    thread_service
        .mark_message_deferred_busy(
            &thread_scope(),
            &thread_id(),
            msg_c.message_id,
            Some("src:binding-drain".to_string()),
            Some("reply:binding-drain".to_string()),
        )
        .await
        .expect("mark C deferred busy");

    // Step 4: Cancel run A → drain fires → B skipped (missing source_binding_id) → C submitted.
    coordinator
        .cancel_run(CancelRunRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            run_id: run_a,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cancel:run-a-hol-test").unwrap(),
        })
        .await
        .expect("cancel run A");

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap();

    // B must still be DeferredBusy (skipped due to missing binding).
    let b_rec = history
        .messages
        .iter()
        .find(|m| m.message_id == msg_b_id)
        .expect("message B in history");
    assert_eq!(
        b_rec.status,
        MessageStatus::DeferredBusy,
        "invalid message B must remain DeferredBusy"
    );

    // C must now be Submitted.
    let c_rec = history
        .messages
        .iter()
        .find(|m| m.message_id == msg_c.message_id)
        .expect("message C in history");
    assert_eq!(
        c_rec.status,
        MessageStatus::Submitted,
        "valid message C must be Submitted after B is skipped"
    );
}

// -----------------------------------------------------------------------
// Scenario E: canonical refs persisted at defer time are replayed verbatim
// -----------------------------------------------------------------------

#[tokio::test]
async fn drain_submits_using_canonical_refs_persisted_at_defer_time() {
    let (coordinator, thread_service, _) = build_harness().await;

    // Step 1: Accept and submit message A — thread lock acquired.
    let msg_a = accept_message(
        &thread_service,
        "message A-canonical",
        "ext-event-a-canonical",
    )
    .await;
    let msg_a_ref = AcceptedMessageRef::new(format!("msg:{}", msg_a.message_id)).unwrap();
    let run_a = submit_run(coordinator.as_ref(), "a-canonical", msg_a_ref).await;

    // Step 2: Accept message B and defer it with explicitly provided canonical
    // refs (the inbound path would compute these before calling the service).
    // We use non-standard prefixes to verify the drain uses exactly what was
    // stored rather than re-deriving with "src:"/"reply:".
    let canonical_src = "webui-src:some-webui-binding-id";
    let canonical_reply = "webui-reply:some-webui-binding-id";
    let msg_b = accept_message(
        &thread_service,
        "message B-canonical",
        "ext-event-b-canonical",
    )
    .await;
    match coordinator
        .submit_turn(SubmitTurnRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            accepted_message_ref: AcceptedMessageRef::new(format!("msg:{}", msg_b.message_id))
                .unwrap(),
            source_binding_ref: SourceBindingRef::new(canonical_src).unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new(canonical_reply).unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("turn:drain-test-b-canonical").unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
        })
        .await
    {
        Err(ironclaw_turns::TurnError::ThreadBusy(_)) => {}
        other => panic!("expected ThreadBusy, got {other:?}"),
    }
    thread_service
        .mark_message_deferred_busy(
            &thread_scope(),
            &thread_id(),
            msg_b.message_id,
            Some(canonical_src.to_string()),
            Some(canonical_reply.to_string()),
        )
        .await
        .expect("mark B deferred busy");

    // Step 3: Cancel run A → drain fires → replays canonical refs verbatim.
    coordinator
        .cancel_run(CancelRunRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            run_id: run_a,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cancel:run-a-oversize-test").unwrap(),
        })
        .await
        .expect("cancel run A");

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap();
    let b_rec = history
        .messages
        .iter()
        .find(|m| m.message_id == msg_b.message_id)
        .expect("message B in history");
    assert_eq!(
        b_rec.status,
        MessageStatus::Submitted,
        "deferred message must be Submitted after drain replays canonical refs verbatim"
    );
}

// -----------------------------------------------------------------------
// Scenario F: two valid deferred messages drained one per terminal event
// -----------------------------------------------------------------------

#[tokio::test]
async fn drain_submits_one_valid_message_per_terminal_event_cascade() {
    let (coordinator, thread_service, publishing_store) = build_harness().await;

    let msg_a = accept_message(&thread_service, "cascade-a", "ev-casc-a").await;
    let run_a = submit_run(
        coordinator.as_ref(),
        "casc-a",
        AcceptedMessageRef::new(format!("msg:{}", msg_a.message_id)).unwrap(),
    )
    .await;

    // Defer B and C in order.
    let msg_b = accept_message(&thread_service, "cascade-b", "ev-casc-b").await;
    thread_service
        .mark_message_deferred_busy(
            &thread_scope(),
            &thread_id(),
            msg_b.message_id,
            Some("src:binding-drain".to_string()),
            Some("reply:binding-drain".to_string()),
        )
        .await
        .expect("defer B");

    let msg_c = accept_message(&thread_service, "cascade-c", "ev-casc-c").await;
    thread_service
        .mark_message_deferred_busy(
            &thread_scope(),
            &thread_id(),
            msg_c.message_id,
            Some("src:binding-drain".to_string()),
            Some("reply:binding-drain".to_string()),
        )
        .await
        .expect("defer C");

    // Complete run A via runner path — drain fires, submits B (oldest).
    let claimed_a = TurnRunTransitionPort::claim_next_run(
        publishing_store.as_ref(),
        ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        },
    )
    .await
    .expect("claim A")
    .expect("A must be claimable");
    assert_eq!(claimed_a.state.run_id, run_a);

    TurnRunTransitionPort::complete_run(
        publishing_store.as_ref(),
        CompleteRunRequest {
            run_id: run_a,
            runner_id: claimed_a.runner_id,
            lease_token: claimed_a.lease_token,
        },
    )
    .await
    .expect("complete A");

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap();
    let b_status = history
        .messages
        .iter()
        .find(|m| m.message_id == msg_b.message_id)
        .map(|m| m.status)
        .expect("msg B");
    let c_status = history
        .messages
        .iter()
        .find(|m| m.message_id == msg_c.message_id)
        .map(|m| m.status)
        .expect("msg C");
    assert_eq!(
        b_status,
        MessageStatus::Submitted,
        "B must be Submitted after A completes"
    );
    assert_eq!(
        c_status,
        MessageStatus::DeferredBusy,
        "C must still be DeferredBusy — drain submits one per terminal event"
    );

    // Complete B's run — drain fires, submits C.
    let claimed_b = TurnRunTransitionPort::claim_next_run(
        publishing_store.as_ref(),
        ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        },
    )
    .await
    .expect("claim B")
    .expect("B's run must be claimable");

    TurnRunTransitionPort::complete_run(
        publishing_store.as_ref(),
        CompleteRunRequest {
            run_id: claimed_b.state.run_id,
            runner_id: claimed_b.runner_id,
            lease_token: claimed_b.lease_token,
        },
    )
    .await
    .expect("complete B's run");

    let history2 = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap();
    let c_status2 = history2
        .messages
        .iter()
        .find(|m| m.message_id == msg_c.message_id)
        .map(|m| m.status)
        .expect("msg C after B");
    assert_eq!(
        c_status2,
        MessageStatus::Submitted,
        "C must be Submitted after B's run completes"
    );
}

// -----------------------------------------------------------------------
// Scenario G: list_deferred_busy_messages error — observe methods stay Ok
// -----------------------------------------------------------------------

/// Minimal `SessionThreadService` that panics on any call except
/// `list_deferred_busy_messages`, which always returns a backend error.
struct FailingListService;

#[async_trait::async_trait]
impl ironclaw_threads::SessionThreadService for FailingListService {
    // ── Required methods — all unreachable; drain only calls list_deferred_busy_messages ──

    async fn ensure_thread(
        &self,
        _: ironclaw_threads::EnsureThreadRequest,
    ) -> Result<ironclaw_threads::SessionThreadRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: ensure_thread")
    }
    async fn accept_inbound_message(
        &self,
        _: ironclaw_threads::AcceptInboundMessageRequest,
    ) -> Result<ironclaw_threads::AcceptedInboundMessage, ironclaw_threads::SessionThreadError>
    {
        unreachable!("FailingListService: accept_inbound_message")
    }
    async fn replay_accepted_inbound_message(
        &self,
        _: ironclaw_threads::ReplayAcceptedInboundMessageRequest,
    ) -> Result<
        Option<ironclaw_threads::AcceptedInboundMessageReplay>,
        ironclaw_threads::SessionThreadError,
    > {
        unreachable!("FailingListService: replay_accepted_inbound_message")
    }
    async fn mark_message_submitted(
        &self,
        _: &ThreadScope,
        _: &ironclaw_host_api::ThreadId,
        _: ironclaw_threads::ThreadMessageId,
        _: String,
        _: String,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: mark_message_submitted")
    }
    async fn mark_message_deferred_busy(
        &self,
        _: &ThreadScope,
        _: &ironclaw_host_api::ThreadId,
        _: ironclaw_threads::ThreadMessageId,
        _: Option<String>,
        _: Option<String>,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: mark_message_deferred_busy")
    }
    /// Always fails — exercises the drain's list-error handling path.
    async fn list_deferred_busy_messages(
        &self,
        _: ironclaw_threads::ListDeferredBusyMessagesRequest,
    ) -> Result<Vec<ironclaw_threads::ThreadMessageRecord>, ironclaw_threads::SessionThreadError>
    {
        Err(ironclaw_threads::SessionThreadError::Backend(
            "injected list failure".to_string(),
        ))
    }
    async fn append_assistant_draft(
        &self,
        _: ironclaw_threads::AppendAssistantDraftRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: append_assistant_draft")
    }
    async fn append_tool_result_reference(
        &self,
        _: ironclaw_threads::AppendToolResultReferenceRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: append_tool_result_reference")
    }
    async fn append_capability_display_preview(
        &self,
        _: ironclaw_threads::AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: append_capability_display_preview")
    }
    async fn update_tool_result_reference(
        &self,
        _: ironclaw_threads::UpdateToolResultReferenceRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: update_tool_result_reference")
    }
    async fn update_assistant_draft(
        &self,
        _: ironclaw_threads::UpdateAssistantDraftRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: update_assistant_draft")
    }
    async fn finalize_assistant_message(
        &self,
        _: &ThreadScope,
        _: &ironclaw_host_api::ThreadId,
        _: ironclaw_threads::ThreadMessageId,
        _: ironclaw_threads::MessageContent,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: finalize_assistant_message")
    }
    async fn redact_message(
        &self,
        _: ironclaw_threads::RedactMessageRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: redact_message")
    }
    async fn load_context_window(
        &self,
        _: ironclaw_threads::LoadContextWindowRequest,
    ) -> Result<ironclaw_threads::ContextWindow, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: load_context_window")
    }
    async fn load_context_messages(
        &self,
        _: ironclaw_threads::LoadContextMessagesRequest,
    ) -> Result<ironclaw_threads::ContextMessages, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: load_context_messages")
    }
    async fn list_thread_history(
        &self,
        _: ironclaw_threads::ThreadHistoryRequest,
    ) -> Result<ironclaw_threads::ThreadHistory, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: list_thread_history")
    }
    async fn create_summary_artifact(
        &self,
        _: ironclaw_threads::CreateSummaryArtifactRequest,
    ) -> Result<ironclaw_threads::SummaryArtifact, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingListService: create_summary_artifact")
    }
    // Methods with default impls (read_thread, delete_thread, latest_thread_message,
    // finalized_assistant_message_by_run, list_thread_messages_range, update_thread_goal,
    // read_thread_by_id, list_threads_for_scope) are inherited as-is.
}

#[tokio::test]
async fn drain_list_error_returns_ok_and_leaves_deferred() {
    let failing_service: Arc<dyn ironclaw_threads::SessionThreadService> =
        Arc::new(FailingListService);

    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let lifecycle_bus = Arc::new(DefaultTurnLifecycleEventBus::new());

    let drain = Arc::new(DeferredBusyDrainObserver::new_unbound(Arc::clone(
        &failing_service,
    )));
    let drain_observer: Arc<dyn TurnCommittedEventObserver> =
        Arc::clone(&drain) as Arc<dyn TurnCommittedEventObserver>;
    lifecycle_bus
        .subscribe_required(drain_observer)
        .expect("subscribe drain");

    let publishing_store = Arc::new(LifecyclePublishingTurnStateStore::new(
        Arc::clone(&turn_store),
        lifecycle_bus,
    ));
    let coordinator: Arc<dyn TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(Arc::clone(&publishing_store)));
    drain
        .bind_coordinator(Arc::clone(&coordinator))
        .expect("bind coordinator");

    let run_response = coordinator
        .submit_turn(SubmitTurnRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            accepted_message_ref: AcceptedMessageRef::new("msg:fail-list-a").unwrap(),
            source_binding_ref: SourceBindingRef::new("src:fail-list").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:fail-list").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("turn:fail-list-a").unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
        })
        .await
        .expect("submit turn");
    let SubmitTurnResponse::Accepted { run_id, .. } = run_response;

    // Cancel triggers observe_committed_event via the lifecycle bus.
    // The drain calls list_deferred_busy_messages on FailingListService → error →
    // drain logs a warn and returns Ok.  The lifecycle bus propagates Ok.
    let cancel_result = coordinator
        .cancel_run(CancelRunRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cancel:fail-list-a").unwrap(),
        })
        .await;
    assert!(
        cancel_result.is_ok(),
        "cancel_run must not fail due to a drain list error: {cancel_result:?}"
    );

    // The cancel_run path above already exercised observe_committed_event via the lifecycle
    // bus.  observe_committed_state (the non-terminal path) doesn't call the list service
    // at all, so no extra assertion is needed here.
}

// -----------------------------------------------------------------------
// Scenario I: actor_id = None — drain skips the record, never submits
// -----------------------------------------------------------------------

/// Mock that returns one `DeferredBusy` message with `actor_id: None` on
/// the first call and an empty list on subsequent calls (after_sequence
/// will be `Some` once the drain advances past the skipped window).
/// Panics on `mark_message_submitted` so any submission attempt fails
/// the test loudly.
struct NoActorListService {
    message_id: ironclaw_threads::ThreadMessageId,
}

#[async_trait::async_trait]
impl ironclaw_threads::SessionThreadService for NoActorListService {
    async fn ensure_thread(
        &self,
        _: ironclaw_threads::EnsureThreadRequest,
    ) -> Result<ironclaw_threads::SessionThreadRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: ensure_thread")
    }
    async fn accept_inbound_message(
        &self,
        _: ironclaw_threads::AcceptInboundMessageRequest,
    ) -> Result<ironclaw_threads::AcceptedInboundMessage, ironclaw_threads::SessionThreadError>
    {
        unreachable!("NoActorListService: accept_inbound_message")
    }
    async fn replay_accepted_inbound_message(
        &self,
        _: ironclaw_threads::ReplayAcceptedInboundMessageRequest,
    ) -> Result<
        Option<ironclaw_threads::AcceptedInboundMessageReplay>,
        ironclaw_threads::SessionThreadError,
    > {
        unreachable!("NoActorListService: replay_accepted_inbound_message")
    }
    /// Must never be called — panics if drain attempts to submit the no-actor record.
    async fn mark_message_submitted(
        &self,
        _: &ThreadScope,
        _: &ironclaw_host_api::ThreadId,
        _: ironclaw_threads::ThreadMessageId,
        _: String,
        _: String,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!(
            "NoActorListService: mark_message_submitted — drain must not submit a no-actor message"
        )
    }
    async fn mark_message_deferred_busy(
        &self,
        _: &ThreadScope,
        _: &ironclaw_host_api::ThreadId,
        _: ironclaw_threads::ThreadMessageId,
        _: Option<String>,
        _: Option<String>,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: mark_message_deferred_busy")
    }
    /// Returns the single no-actor record on the first call (after_sequence = None).
    /// Returns an empty list on subsequent calls (after_sequence = Some) so the
    /// drain exits cleanly via the empty-window path rather than looping to cap.
    async fn list_deferred_busy_messages(
        &self,
        request: ironclaw_threads::ListDeferredBusyMessagesRequest,
    ) -> Result<Vec<ironclaw_threads::ThreadMessageRecord>, ironclaw_threads::SessionThreadError>
    {
        if request.after_sequence.is_some() {
            return Ok(vec![]);
        }
        Ok(vec![ironclaw_threads::ThreadMessageRecord {
            message_id: self.message_id,
            thread_id: thread_id(),
            sequence: 1,
            kind: ironclaw_threads::MessageKind::User,
            status: ironclaw_threads::MessageStatus::DeferredBusy,
            actor_id: None, // <- the branch under test
            source_binding_id: Some("binding-drain".to_string()),
            reply_target_binding_id: Some("binding-drain".to_string()),
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some("no-actor message".to_string()),
            redaction_ref: None,
            turn_source_binding_ref: Some("src:binding-drain".to_string()),
            turn_reply_target_binding_ref: Some("reply:binding-drain".to_string()),
        }])
    }
    async fn append_assistant_draft(
        &self,
        _: ironclaw_threads::AppendAssistantDraftRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: append_assistant_draft")
    }
    async fn append_tool_result_reference(
        &self,
        _: ironclaw_threads::AppendToolResultReferenceRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: append_tool_result_reference")
    }
    async fn append_capability_display_preview(
        &self,
        _: ironclaw_threads::AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: append_capability_display_preview")
    }
    async fn update_tool_result_reference(
        &self,
        _: ironclaw_threads::UpdateToolResultReferenceRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: update_tool_result_reference")
    }
    async fn update_assistant_draft(
        &self,
        _: ironclaw_threads::UpdateAssistantDraftRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: update_assistant_draft")
    }
    async fn finalize_assistant_message(
        &self,
        _: &ThreadScope,
        _: &ironclaw_host_api::ThreadId,
        _: ironclaw_threads::ThreadMessageId,
        _: ironclaw_threads::MessageContent,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: finalize_assistant_message")
    }
    async fn redact_message(
        &self,
        _: ironclaw_threads::RedactMessageRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: redact_message")
    }
    async fn load_context_window(
        &self,
        _: ironclaw_threads::LoadContextWindowRequest,
    ) -> Result<ironclaw_threads::ContextWindow, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: load_context_window")
    }
    async fn load_context_messages(
        &self,
        _: ironclaw_threads::LoadContextMessagesRequest,
    ) -> Result<ironclaw_threads::ContextMessages, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: load_context_messages")
    }
    async fn list_thread_history(
        &self,
        _: ironclaw_threads::ThreadHistoryRequest,
    ) -> Result<ironclaw_threads::ThreadHistory, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: list_thread_history")
    }
    async fn create_summary_artifact(
        &self,
        _: ironclaw_threads::CreateSummaryArtifactRequest,
    ) -> Result<ironclaw_threads::SummaryArtifact, ironclaw_threads::SessionThreadError> {
        unreachable!("NoActorListService: create_summary_artifact")
    }
    // Methods with default impls (read_thread, delete_thread, latest_thread_message,
    // finalized_assistant_message_by_run, list_thread_messages_range, update_thread_goal,
    // read_thread_by_id, list_threads_for_scope) are inherited as-is.
}

#[tokio::test]
async fn drain_skips_deferred_message_with_no_actor_id_and_leaves_it_deferred_busy() {
    let message_id = ironclaw_threads::ThreadMessageId::new();
    let no_actor_service: Arc<dyn ironclaw_threads::SessionThreadService> =
        Arc::new(NoActorListService { message_id });

    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let lifecycle_bus = Arc::new(DefaultTurnLifecycleEventBus::new());

    let drain = Arc::new(DeferredBusyDrainObserver::new_unbound(Arc::clone(
        &no_actor_service,
    )));
    let drain_observer: Arc<dyn TurnCommittedEventObserver> =
        Arc::clone(&drain) as Arc<dyn TurnCommittedEventObserver>;
    lifecycle_bus
        .subscribe_required(drain_observer)
        .expect("subscribe drain");

    let publishing_store = Arc::new(LifecyclePublishingTurnStateStore::new(
        Arc::clone(&turn_store),
        lifecycle_bus,
    ));
    let coordinator: Arc<dyn TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(Arc::clone(&publishing_store)));
    drain
        .bind_coordinator(Arc::clone(&coordinator))
        .expect("bind coordinator");

    // Submit a run so we have a run_id to cancel.
    let run_response = coordinator
        .submit_turn(SubmitTurnRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            accepted_message_ref: AcceptedMessageRef::new("msg:no-actor-a").unwrap(),
            source_binding_ref: SourceBindingRef::new("src:binding-drain").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("turn:no-actor-test-a").unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
        })
        .await
        .expect("submit turn");
    let SubmitTurnResponse::Accepted { run_id, .. } = run_response;

    // Cancel fires drain → drain sees actor_id = None → skips → no submit.
    // If mark_message_submitted were called, NoActorListService would panic.
    let cancel_result = coordinator
        .cancel_run(CancelRunRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cancel:no-actor-a").unwrap(),
        })
        .await;
    assert!(
        cancel_result.is_ok(),
        "cancel_run must not fail even when drain skips a no-actor message: {cancel_result:?}"
    );
}

// -----------------------------------------------------------------------
// Scenario H: ThreadBusy during drain — message stays DeferredBusy
// -----------------------------------------------------------------------

#[tokio::test]
async fn drain_leaves_deferred_when_resubmit_hits_thread_busy() {
    // Build harness normally.
    let (coordinator, thread_service, publishing_store) = build_harness().await;

    // Submit run A — thread locked.
    let msg_a = accept_message(&thread_service, "busy-h-a", "ev-busy-h-a").await;
    let run_a = submit_run(
        coordinator.as_ref(),
        "busy-h-a",
        AcceptedMessageRef::new(format!("msg:{}", msg_a.message_id)).unwrap(),
    )
    .await;

    // Defer B.
    let msg_b = accept_message(&thread_service, "busy-h-b", "ev-busy-h-b").await;
    thread_service
        .mark_message_deferred_busy(
            &thread_scope(),
            &thread_id(),
            msg_b.message_id,
            Some("src:binding-drain".to_string()),
            Some("reply:binding-drain".to_string()),
        )
        .await
        .expect("defer B");

    // Cancel A — drain fires, submits B (B becomes Submitted, thread busy again).
    coordinator
        .cancel_run(CancelRunRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            run_id: run_a,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cancel:busy-h-a").unwrap(),
        })
        .await
        .expect("cancel A");

    let hist1 = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap();
    assert_eq!(
        hist1
            .messages
            .iter()
            .find(|m| m.message_id == msg_b.message_id)
            .unwrap()
            .status,
        MessageStatus::Submitted,
        "drain must submit B after A terminates"
    );

    // Thread now has B's run InProgress.  Accept C, try to submit C → ThreadBusy.
    // Defer C manually.  Accept D, submit D → ThreadBusy (B still active).
    let msg_c = accept_message(&thread_service, "busy-h-c", "ev-busy-h-c").await;
    let submit_c_err = coordinator
        .submit_turn(SubmitTurnRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            accepted_message_ref: AcceptedMessageRef::new(format!("msg:{}", msg_c.message_id))
                .unwrap(),
            source_binding_ref: SourceBindingRef::new("src:binding-drain").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("turn:busy-h-c").unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
        })
        .await;
    assert!(
        matches!(submit_c_err, Err(ironclaw_turns::TurnError::ThreadBusy(_))),
        "C must hit ThreadBusy while B runs: {submit_c_err:?}"
    );
    thread_service
        .mark_message_deferred_busy(
            &thread_scope(),
            &thread_id(),
            msg_c.message_id,
            Some("src:binding-drain".to_string()),
            Some("reply:binding-drain".to_string()),
        )
        .await
        .expect("defer C");

    // Claim and complete B's run — drain fires, submits C.
    let claimed_b = TurnRunTransitionPort::claim_next_run(
        publishing_store.as_ref(),
        ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        },
    )
    .await
    .expect("claim B")
    .expect("B must be claimable");

    TurnRunTransitionPort::complete_run(
        publishing_store.as_ref(),
        CompleteRunRequest {
            run_id: claimed_b.state.run_id,
            runner_id: claimed_b.runner_id,
            lease_token: claimed_b.lease_token,
        },
    )
    .await
    .expect("complete B's run");

    // C must now be Submitted.
    let hist2 = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap();
    assert_eq!(
        hist2
            .messages
            .iter()
            .find(|m| m.message_id == msg_c.message_id)
            .unwrap()
            .status,
        MessageStatus::Submitted,
        "C must be Submitted after B's run terminates"
    );
}

// -----------------------------------------------------------------------
// Scenario J: submit_turn returns a non-ThreadBusy error during drain
//
// The observer must return Ok so the terminal path is not poisoned.
// The deferred message must NOT be marked submitted.
// -----------------------------------------------------------------------

/// Coordinator mock whose `submit_turn` always returns a non-ThreadBusy
/// error (Unavailable).  All other methods are unreachable.
struct FailingSubmitCoordinator;

#[async_trait::async_trait]
impl TurnCoordinator for FailingSubmitCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        unreachable!("FailingSubmitCoordinator: prepare_turn")
    }

    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "injected submit failure".to_string(),
        })
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unreachable!("FailingSubmitCoordinator: resume_turn")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        unreachable!("FailingSubmitCoordinator: cancel_run")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        unreachable!("FailingSubmitCoordinator: get_run_state")
    }
}

#[tokio::test]
async fn drain_submit_error_returns_ok_and_leaves_deferred_busy() {
    let message_id = ironclaw_threads::ThreadMessageId::new();

    // Use NoActorListService shape but with valid actor_id so the drain
    // actually reaches submit_turn.  Build a bespoke service inline.
    struct ValidRecordListService {
        message_id: ironclaw_threads::ThreadMessageId,
    }

    #[async_trait::async_trait]
    impl ironclaw_threads::SessionThreadService for ValidRecordListService {
        async fn ensure_thread(
            &self,
            _: ironclaw_threads::EnsureThreadRequest,
        ) -> Result<ironclaw_threads::SessionThreadRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: ensure_thread")
        }
        async fn accept_inbound_message(
            &self,
            _: ironclaw_threads::AcceptInboundMessageRequest,
        ) -> Result<ironclaw_threads::AcceptedInboundMessage, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: accept_inbound_message")
        }
        async fn replay_accepted_inbound_message(
            &self,
            _: ironclaw_threads::ReplayAcceptedInboundMessageRequest,
        ) -> Result<
            Option<ironclaw_threads::AcceptedInboundMessageReplay>,
            ironclaw_threads::SessionThreadError,
        > {
            unreachable!("ValidRecordListService: replay_accepted_inbound_message")
        }
        /// Must not be called — panics if drain attempts to mark submitted
        /// after a coordinator error.
        async fn mark_message_submitted(
            &self,
            _: &ThreadScope,
            _: &ironclaw_host_api::ThreadId,
            _: ironclaw_threads::ThreadMessageId,
            _: String,
            _: String,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!(
                "ValidRecordListService: mark_message_submitted — must not be called after submit error"
            )
        }
        async fn mark_message_deferred_busy(
            &self,
            _: &ThreadScope,
            _: &ironclaw_host_api::ThreadId,
            _: ironclaw_threads::ThreadMessageId,
            _: Option<String>,
            _: Option<String>,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: mark_message_deferred_busy")
        }
        /// Returns one valid DeferredBusy record on first call, empty on
        /// subsequent calls (after_sequence is Some once the drain advances).
        async fn list_deferred_busy_messages(
            &self,
            request: ironclaw_threads::ListDeferredBusyMessagesRequest,
        ) -> Result<Vec<ironclaw_threads::ThreadMessageRecord>, ironclaw_threads::SessionThreadError>
        {
            if request.after_sequence.is_some() {
                return Ok(vec![]);
            }
            Ok(vec![ironclaw_threads::ThreadMessageRecord {
                message_id: self.message_id,
                thread_id: thread_id(),
                sequence: 1,
                kind: ironclaw_threads::MessageKind::User,
                status: ironclaw_threads::MessageStatus::DeferredBusy,
                actor_id: Some(actor().as_str().to_string()),
                source_binding_id: Some("binding-drain".to_string()),
                reply_target_binding_id: Some("binding-drain".to_string()),
                turn_id: None,
                turn_run_id: None,
                tool_result_ref: None,
                tool_result_provider_call: None,
                content: Some("submit-error test message".to_string()),
                redaction_ref: None,
                turn_source_binding_ref: Some("src:binding-drain".to_string()),
                turn_reply_target_binding_ref: Some("reply:binding-drain".to_string()),
            }])
        }
        async fn append_assistant_draft(
            &self,
            _: ironclaw_threads::AppendAssistantDraftRequest,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: append_assistant_draft")
        }
        async fn append_tool_result_reference(
            &self,
            _: ironclaw_threads::AppendToolResultReferenceRequest,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: append_tool_result_reference")
        }
        async fn append_capability_display_preview(
            &self,
            _: ironclaw_threads::AppendCapabilityDisplayPreviewRequest,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: append_capability_display_preview")
        }
        async fn update_tool_result_reference(
            &self,
            _: ironclaw_threads::UpdateToolResultReferenceRequest,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: update_tool_result_reference")
        }
        async fn update_assistant_draft(
            &self,
            _: ironclaw_threads::UpdateAssistantDraftRequest,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: update_assistant_draft")
        }
        async fn finalize_assistant_message(
            &self,
            _: &ThreadScope,
            _: &ironclaw_host_api::ThreadId,
            _: ironclaw_threads::ThreadMessageId,
            _: ironclaw_threads::MessageContent,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: finalize_assistant_message")
        }
        async fn redact_message(
            &self,
            _: ironclaw_threads::RedactMessageRequest,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: redact_message")
        }
        async fn load_context_window(
            &self,
            _: ironclaw_threads::LoadContextWindowRequest,
        ) -> Result<ironclaw_threads::ContextWindow, ironclaw_threads::SessionThreadError> {
            unreachable!("ValidRecordListService: load_context_window")
        }
        async fn load_context_messages(
            &self,
            _: ironclaw_threads::LoadContextMessagesRequest,
        ) -> Result<ironclaw_threads::ContextMessages, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: load_context_messages")
        }
        async fn list_thread_history(
            &self,
            _: ironclaw_threads::ThreadHistoryRequest,
        ) -> Result<ironclaw_threads::ThreadHistory, ironclaw_threads::SessionThreadError> {
            unreachable!("ValidRecordListService: list_thread_history")
        }
        async fn create_summary_artifact(
            &self,
            _: ironclaw_threads::CreateSummaryArtifactRequest,
        ) -> Result<ironclaw_threads::SummaryArtifact, ironclaw_threads::SessionThreadError>
        {
            unreachable!("ValidRecordListService: create_summary_artifact")
        }
        // Methods with default impls are inherited.
    }

    let svc: Arc<dyn ironclaw_threads::SessionThreadService> =
        Arc::new(ValidRecordListService { message_id });
    let drain = Arc::new(DeferredBusyDrainObserver::new_unbound(Arc::clone(&svc)));

    // Bind a coordinator that always fails submit_turn with a non-ThreadBusy error.
    let failing_coordinator: Arc<dyn TurnCoordinator> = Arc::new(FailingSubmitCoordinator);
    drain
        .bind_coordinator(Arc::clone(&failing_coordinator))
        .expect("bind coordinator");

    // Manually invoke observe_committed_event with a synthetic terminal event.
    // The event scope must carry an agent_id so thread-scope derivation succeeds.
    let event = TurnLifecycleEvent {
        cursor: EventCursor(1),
        scope: turn_scope(),
        occurred_at: None,
        owner_user_id: Some(owner()),
        run_id: TurnRunId::new(),
        status: TurnStatus::Cancelled,
        kind: TurnEventKind::Cancelled,
        blocked_gate: None,
        sanitized_reason: None,
    };

    let result = drain.observe_committed_event(event).await;
    assert!(
        result.is_ok(),
        "observe_committed_event must return Ok even when submit_turn fails: {result:?}"
    );
}

// -----------------------------------------------------------------------
// Scenario K: submit returns Accepted but mark_message_submitted fails
//
// The observer must still return Ok — mark failure is non-fatal and logged
// at warn.  The drain path is complete from the coordinator's perspective.
// -----------------------------------------------------------------------

/// Service mock: `list_deferred_busy_messages` returns one valid record on
/// the first call; `mark_message_submitted` always returns an error.
struct FailingMarkSubmittedService {
    message_id: ironclaw_threads::ThreadMessageId,
}

#[async_trait::async_trait]
impl ironclaw_threads::SessionThreadService for FailingMarkSubmittedService {
    async fn ensure_thread(
        &self,
        _: ironclaw_threads::EnsureThreadRequest,
    ) -> Result<ironclaw_threads::SessionThreadRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: ensure_thread")
    }
    async fn accept_inbound_message(
        &self,
        _: ironclaw_threads::AcceptInboundMessageRequest,
    ) -> Result<ironclaw_threads::AcceptedInboundMessage, ironclaw_threads::SessionThreadError>
    {
        unreachable!("FailingMarkSubmittedService: accept_inbound_message")
    }
    async fn replay_accepted_inbound_message(
        &self,
        _: ironclaw_threads::ReplayAcceptedInboundMessageRequest,
    ) -> Result<
        Option<ironclaw_threads::AcceptedInboundMessageReplay>,
        ironclaw_threads::SessionThreadError,
    > {
        unreachable!("FailingMarkSubmittedService: replay_accepted_inbound_message")
    }
    /// Always fails — exercises the drain's mark-error handling path.
    async fn mark_message_submitted(
        &self,
        _: &ThreadScope,
        _: &ironclaw_host_api::ThreadId,
        _: ironclaw_threads::ThreadMessageId,
        _: String,
        _: String,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        Err(ironclaw_threads::SessionThreadError::Backend(
            "injected mark_message_submitted failure".to_string(),
        ))
    }
    async fn mark_message_deferred_busy(
        &self,
        _: &ThreadScope,
        _: &ironclaw_host_api::ThreadId,
        _: ironclaw_threads::ThreadMessageId,
        _: Option<String>,
        _: Option<String>,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: mark_message_deferred_busy")
    }
    /// Returns one valid DeferredBusy record on first call, empty on
    /// subsequent calls (after the drain succeeds with the coordinator and
    /// continues past the mark failure).
    async fn list_deferred_busy_messages(
        &self,
        request: ironclaw_threads::ListDeferredBusyMessagesRequest,
    ) -> Result<Vec<ironclaw_threads::ThreadMessageRecord>, ironclaw_threads::SessionThreadError>
    {
        if request.after_sequence.is_some() {
            return Ok(vec![]);
        }
        Ok(vec![ironclaw_threads::ThreadMessageRecord {
            message_id: self.message_id,
            thread_id: thread_id(),
            sequence: 1,
            kind: ironclaw_threads::MessageKind::User,
            status: ironclaw_threads::MessageStatus::DeferredBusy,
            actor_id: Some(actor().as_str().to_string()),
            source_binding_id: Some("binding-drain".to_string()),
            reply_target_binding_id: Some("binding-drain".to_string()),
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some("mark-fail test message".to_string()),
            redaction_ref: None,
            turn_source_binding_ref: Some("src:binding-drain".to_string()),
            turn_reply_target_binding_ref: Some("reply:binding-drain".to_string()),
        }])
    }
    async fn append_assistant_draft(
        &self,
        _: ironclaw_threads::AppendAssistantDraftRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: append_assistant_draft")
    }
    async fn append_tool_result_reference(
        &self,
        _: ironclaw_threads::AppendToolResultReferenceRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: append_tool_result_reference")
    }
    async fn append_capability_display_preview(
        &self,
        _: ironclaw_threads::AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: append_capability_display_preview")
    }
    async fn update_tool_result_reference(
        &self,
        _: ironclaw_threads::UpdateToolResultReferenceRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: update_tool_result_reference")
    }
    async fn update_assistant_draft(
        &self,
        _: ironclaw_threads::UpdateAssistantDraftRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: update_assistant_draft")
    }
    async fn finalize_assistant_message(
        &self,
        _: &ThreadScope,
        _: &ironclaw_host_api::ThreadId,
        _: ironclaw_threads::ThreadMessageId,
        _: ironclaw_threads::MessageContent,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: finalize_assistant_message")
    }
    async fn redact_message(
        &self,
        _: ironclaw_threads::RedactMessageRequest,
    ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: redact_message")
    }
    async fn load_context_window(
        &self,
        _: ironclaw_threads::LoadContextWindowRequest,
    ) -> Result<ironclaw_threads::ContextWindow, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: load_context_window")
    }
    async fn load_context_messages(
        &self,
        _: ironclaw_threads::LoadContextMessagesRequest,
    ) -> Result<ironclaw_threads::ContextMessages, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: load_context_messages")
    }
    async fn list_thread_history(
        &self,
        _: ironclaw_threads::ThreadHistoryRequest,
    ) -> Result<ironclaw_threads::ThreadHistory, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: list_thread_history")
    }
    async fn create_summary_artifact(
        &self,
        _: ironclaw_threads::CreateSummaryArtifactRequest,
    ) -> Result<ironclaw_threads::SummaryArtifact, ironclaw_threads::SessionThreadError> {
        unreachable!("FailingMarkSubmittedService: create_summary_artifact")
    }
    // Methods with default impls are inherited.
}

#[tokio::test]
async fn drain_mark_submitted_error_returns_ok_after_accept() {
    let message_id = ironclaw_threads::ThreadMessageId::new();
    let svc: Arc<dyn ironclaw_threads::SessionThreadService> =
        Arc::new(FailingMarkSubmittedService { message_id });

    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let lifecycle_bus = Arc::new(DefaultTurnLifecycleEventBus::new());

    let drain = Arc::new(DeferredBusyDrainObserver::new_unbound(Arc::clone(&svc)));
    let drain_observer: Arc<dyn TurnCommittedEventObserver> =
        Arc::clone(&drain) as Arc<dyn TurnCommittedEventObserver>;
    lifecycle_bus
        .subscribe_required(drain_observer)
        .expect("subscribe drain");

    let publishing_store = Arc::new(LifecyclePublishingTurnStateStore::new(
        Arc::clone(&turn_store),
        lifecycle_bus,
    ));
    // DefaultTurnCoordinator will succeed on submit_turn; the service mock
    // only fails on mark_message_submitted.
    let coordinator: Arc<dyn TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(Arc::clone(&publishing_store)));
    drain
        .bind_coordinator(Arc::clone(&coordinator))
        .expect("bind coordinator");

    // Submit a run to acquire the thread lock, then cancel it to fire the
    // drain.  The drain will: list → valid record → submit (Accepted) →
    // mark_message_submitted fails → warn, return Ok.
    let run_response = coordinator
        .submit_turn(SubmitTurnRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            accepted_message_ref: AcceptedMessageRef::new("msg:mark-fail-a").unwrap(),
            source_binding_ref: SourceBindingRef::new("src:binding-drain").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("turn:mark-fail-a").unwrap(),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
        })
        .await
        .expect("submit first turn");
    let SubmitTurnResponse::Accepted { run_id, .. } = run_response;

    // Cancel fires drain → list → valid record → submit Accepted →
    // mark_message_submitted returns error → drain logs warn → returns Ok.
    // The cancel itself must also return Ok (terminal path not poisoned).
    let cancel_result = coordinator
        .cancel_run(CancelRunRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cancel:mark-fail-a").unwrap(),
        })
        .await;
    assert!(
        cancel_result.is_ok(),
        "cancel_run must not fail due to a mark_message_submitted error: {cancel_result:?}"
    );
}

// -----------------------------------------------------------------------
// Scenario L: drain replays persisted canonical refs verbatim (submit capture)
//
// Unlike Scenario E (which only checks the thread status after drain),
// this test captures the exact SubmitTurnRequest the drain passes to the
// coordinator and asserts every ref field matches the persisted values.
// A drain that re-derives refs with a different prefix would pass
// Scenario E but fail here.
// -----------------------------------------------------------------------

/// Coordinator wrapper that records the first `SubmitTurnRequest` it
/// receives and forwards the call to the inner coordinator.
struct CapturingCoordinator {
    inner: Arc<dyn TurnCoordinator>,
    captured: tokio::sync::Mutex<Option<SubmitTurnRequest>>,
}

impl CapturingCoordinator {
    fn new(inner: Arc<dyn TurnCoordinator>) -> Self {
        Self {
            inner,
            captured: tokio::sync::Mutex::new(None),
        }
    }

    async fn take_captured(&self) -> Option<SubmitTurnRequest> {
        self.captured.lock().await.take()
    }
}

#[async_trait::async_trait]
impl TurnCoordinator for CapturingCoordinator {
    async fn prepare_turn(&self, scope: TurnScope) -> Result<TurnRunId, TurnError> {
        self.inner.prepare_turn(scope).await
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        // Store a clone of the request before forwarding.
        *self.captured.lock().await = Some(request.clone());
        self.inner.submit_turn(request).await
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.inner.resume_turn(request).await
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        self.inner.cancel_run(request).await
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.inner.get_run_state(request).await
    }
}

#[tokio::test]
async fn drain_replays_persisted_refs_verbatim_in_submit_turn_request() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope(),
            thread_id: Some(thread_id()),
            created_by_actor_id: actor().as_str().to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure thread");

    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let lifecycle_bus = Arc::new(DefaultTurnLifecycleEventBus::new());

    let drain_observer_for_bind = Arc::new(DeferredBusyDrainObserver::new_unbound(Arc::clone(
        &thread_service,
    )
        as Arc<dyn ironclaw_threads::SessionThreadService>));
    let drain_observer: Arc<dyn TurnCommittedEventObserver> =
        Arc::clone(&drain_observer_for_bind) as Arc<dyn TurnCommittedEventObserver>;
    lifecycle_bus
        .subscribe_required(drain_observer)
        .expect("subscribe drain observer");

    let publishing_store = Arc::new(LifecyclePublishingTurnStateStore::new(
        Arc::clone(&turn_store),
        lifecycle_bus,
    ));

    // Wrap the real coordinator in the capturing wrapper so we intercept
    // the drain's submit_turn call (the first submit_turn comes from the
    // test setup for run A; the second is the drain's call for message B).
    let inner_coordinator: Arc<dyn TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(Arc::clone(&publishing_store)));
    let capturing_coordinator = Arc::new(CapturingCoordinator::new(Arc::clone(&inner_coordinator)));
    let coordinator: Arc<dyn TurnCoordinator> =
        Arc::clone(&capturing_coordinator) as Arc<dyn TurnCoordinator>;

    drain_observer_for_bind
        .bind_coordinator(Arc::clone(&coordinator))
        .expect("bind drain coordinator");

    // Step 1: Submit run A directly through the inner coordinator to avoid
    // recording it in the capturing wrapper (we only want to capture the
    // drain's submit for message B).
    let msg_a = accept_message(&thread_service, "capture-a", "ev-cap-a").await;
    let run_a = submit_run(
        inner_coordinator.as_ref(),
        "capture-a",
        AcceptedMessageRef::new(format!("msg:{}", msg_a.message_id)).unwrap(),
    )
    .await;

    // Step 2: Accept B with distinctive canonical refs and defer it.
    let canonical_src = "webui-src:capture-me";
    let canonical_reply = "webui-reply:capture-me";
    let msg_b = accept_message(&thread_service, "capture-b", "ev-cap-b").await;
    let expected_accepted_ref = format!("msg:{}", msg_b.message_id);

    // Clear any leftover capture from build_harness setup before deferring.
    let _ = capturing_coordinator.take_captured().await;

    thread_service
        .mark_message_deferred_busy(
            &thread_scope(),
            &thread_id(),
            msg_b.message_id,
            Some(canonical_src.to_string()),
            Some(canonical_reply.to_string()),
        )
        .await
        .expect("mark B deferred busy");

    // Step 3: Cancel run A → drain fires → intercept the SubmitTurnRequest.
    coordinator
        .cancel_run(CancelRunRequest {
            scope: turn_scope(),
            actor: TurnActor::new(actor()),
            run_id: run_a,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cancel:run-a-capture-test").unwrap(),
        })
        .await
        .expect("cancel run A");

    // Step 4: Retrieve the captured request and assert all three ref fields.
    let captured = capturing_coordinator
        .take_captured()
        .await
        .expect("drain must have called submit_turn for deferred message B");

    assert_eq!(
        captured.source_binding_ref.as_str(),
        canonical_src,
        "source_binding_ref must equal the persisted canonical ref verbatim"
    );
    assert_eq!(
        captured.reply_target_binding_ref.as_str(),
        canonical_reply,
        "reply_target_binding_ref must equal the persisted canonical ref verbatim"
    );
    assert_eq!(
        captured.accepted_message_ref.as_str(),
        expected_accepted_ref,
        "accepted_message_ref must point at the deferred message"
    );
}

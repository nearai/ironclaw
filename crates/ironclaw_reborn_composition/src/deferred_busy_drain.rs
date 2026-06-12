/// Drains [`MessageStatus::DeferredBusy`] messages when a blocking run reaches
/// terminal state.
///
/// When a run terminates the thread's active lock is released.  Any messages
/// that were parked with `DeferredBusy` because the thread was busy may now be
/// submitted.  This observer fires once per terminal event, picks up the
/// *oldest* deferred message for the affected thread, and resubmits it through
/// the coordinator.  One-at-a-time cascade semantics follow naturally: when
/// that resubmitted run terminates, this observer fires again and picks up the
/// next deferred message.
///
/// # Failure contract
///
/// Drain failures must **never** poison the terminal-event path.  The observer
/// logs a `warn!` and returns `Ok(())` so the run's own terminal state is
/// always committed even when the drain step fails.
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ironclaw_host_api::UserId;
use ironclaw_product_workflow::{
    DEFAULT_BINDING_REF_RAW_MAX_BYTES, bounded_reply_target_binding_ref, bounded_source_binding_ref,
};
use ironclaw_threads::{ListDeferredBusyMessagesRequest, SessionThreadService, ThreadScope};
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnCommittedEventObserver, TurnCoordinator, TurnError, TurnLifecycleEvent, TurnRunState,
    TurnScope,
};

/// Maximum number of `DeferredBusy` records loaded per drain attempt.
///
/// This caps the full-thread history scan to a bounded window. The value must
/// be large enough that the skip-invalid loop (Fix 3) can make progress past
/// a few bad entries before stopping and leaving the rest for the next drain.
const DRAIN_LIST_LIMIT: usize = 8;

/// Observer that drains `DeferredBusy` thread messages when a run terminates.
///
/// Bind the coordinator before subscribing — use `new_unbound` + `bind_coordinator`
/// mirroring the [`SubagentCompletionObserver`] pattern.
#[derive(Clone)]
pub(crate) struct DeferredBusyDrainObserver<S: SessionThreadService + ?Sized> {
    thread_service: Arc<S>,
    coordinator: Arc<OnceLock<Arc<dyn TurnCoordinator>>>,
}

impl<S> DeferredBusyDrainObserver<S>
where
    S: SessionThreadService + ?Sized,
{
    /// Create an unbound observer.  Call [`bind_coordinator`] before
    /// subscribing to the lifecycle bus.
    pub(crate) fn new_unbound(thread_service: Arc<S>) -> Self {
        Self {
            thread_service,
            coordinator: Arc::new(OnceLock::new()),
        }
    }

    /// Bind the [`TurnCoordinator`] back-reference so the drain can submit
    /// runs.  Returns `TurnError::InvalidRequest` if already bound.
    pub(crate) fn bind_coordinator(
        &self,
        coordinator: Arc<dyn TurnCoordinator>,
    ) -> Result<(), TurnError> {
        self.coordinator
            .set(coordinator)
            .map_err(|_| TurnError::InvalidRequest {
                reason: "deferred busy drain observer coordinator already bound".to_string(),
            })
    }

    /// Core drain logic: lists deferred messages for `thread_id` within
    /// `thread_scope` and submits the first valid one through `coordinator`.
    ///
    /// Iterates in sequence order (oldest first).  A message that fails
    /// validation (bad actor, missing binding refs, etc.) is logged at
    /// `warn!` and **skipped** — the loop continues to the next entry.
    /// This prevents a single corrupt entry from head-of-line blocking the
    /// entire queue.  On a successful submit the loop stops (cascade
    /// semantics take over when that run terminates).  On `ThreadBusy` the
    /// loop stops and all remaining messages stay deferred.
    ///
    /// Failed messages are NOT mutated — LLM-data retention rule applies.
    /// They will be re-examined on each subsequent drain call and skipped
    /// again, which is acceptable.
    async fn drain_for_scope(
        &self,
        run_id: &ironclaw_turns::TurnRunId,
        scope: &TurnScope,
        thread_scope: &ThreadScope,
        coordinator: &Arc<dyn TurnCoordinator>,
    ) -> Result<(), TurnError> {
        let deferred = match self
            .thread_service
            .list_deferred_busy_messages(ListDeferredBusyMessagesRequest {
                scope: thread_scope.clone(),
                thread_id: scope.thread_id.clone(),
                limit: Some(DRAIN_LIST_LIMIT),
            })
            .await
        {
            Ok(messages) => messages,
            Err(error) => {
                warn!(
                    run_id = %run_id,
                    error = %error,
                    "DeferredBusyDrainObserver: failed to list deferred messages, skipping drain"
                );
                return Ok(());
            }
        };

        for message in deferred {
            // Build the coordinator submission from the thread record fields.
            // On any validation failure for this message, log + skip to next.

            let actor_user_id = match resolve_actor_user_id(&message, thread_scope) {
                Ok(id) => id,
                Err(reason) => {
                    warn!(
                        run_id = %run_id,
                        message_id = %message.message_id,
                        reason,
                        "DeferredBusyDrainObserver: cannot resolve actor for deferred message, skipping"
                    );
                    continue;
                }
            };

            let source_binding_id_raw = match message
                .source_binding_id
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                Some(raw) => raw,
                None => {
                    warn!(
                        run_id = %run_id,
                        message_id = %message.message_id,
                        "DeferredBusyDrainObserver: deferred message missing source_binding_id, skipping"
                    );
                    continue;
                }
            };
            // Use the same bounded conversion that the inbound turn path used
            // when originally persisting the binding ids, so the ref matches
            // what the original product submission produced.
            let source_binding_ref = match bounded_source_binding_ref(
                "src",
                source_binding_id_raw,
                DEFAULT_BINDING_REF_RAW_MAX_BYTES,
            ) {
                Ok(r) => r,
                Err(reason) => {
                    warn!(
                        run_id = %run_id,
                        message_id = %message.message_id,
                        reason,
                        "DeferredBusyDrainObserver: invalid source_binding_id, skipping"
                    );
                    continue;
                }
            };

            let reply_target_binding_id_raw = match message
                .reply_target_binding_id
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                Some(raw) => raw,
                None => {
                    warn!(
                        run_id = %run_id,
                        message_id = %message.message_id,
                        "DeferredBusyDrainObserver: deferred message missing reply_target_binding_id, skipping"
                    );
                    continue;
                }
            };
            let reply_target_binding_ref = match bounded_reply_target_binding_ref(
                "reply",
                reply_target_binding_id_raw,
                DEFAULT_BINDING_REF_RAW_MAX_BYTES,
            ) {
                Ok(r) => r,
                Err(reason) => {
                    warn!(
                        run_id = %run_id,
                        message_id = %message.message_id,
                        reason,
                        "DeferredBusyDrainObserver: invalid reply_target_binding_id, skipping"
                    );
                    continue;
                }
            };

            let accepted_message_ref =
                match AcceptedMessageRef::new(format!("msg:{}", message.message_id)) {
                    Ok(r) => r,
                    Err(reason) => {
                        warn!(
                            run_id = %run_id,
                            message_id = %message.message_id,
                            reason,
                            "DeferredBusyDrainObserver: cannot build accepted_message_ref, skipping"
                        );
                        continue;
                    }
                };

            // Use the message_id as the idempotency key so a duplicate drain
            // fire produces the same run rather than a second submission.
            let idempotency_key = match IdempotencyKey::new(format!("drain:{}", message.message_id))
            {
                Ok(k) => k,
                Err(reason) => {
                    warn!(
                        run_id = %run_id,
                        message_id = %message.message_id,
                        reason,
                        "DeferredBusyDrainObserver: cannot build idempotency key, skipping"
                    );
                    continue;
                }
            };

            let agent_id = match scope.agent_id.clone() {
                Some(id) => id,
                None => {
                    debug!(
                        run_id = %run_id,
                        message_id = %message.message_id,
                        "DeferredBusyDrainObserver: agentless scope, skipping drain"
                    );
                    // Agentless scope is a structural issue — no point
                    // iterating further since all messages share the same scope.
                    return Ok(());
                }
            };

            let turn_scope = TurnScope::new_with_owner(
                scope.tenant_id.clone(),
                Some(agent_id),
                scope.project_id.clone(),
                scope.thread_id.clone(),
                thread_scope.owner_user_id.clone(),
            );

            let request = SubmitTurnRequest {
                scope: turn_scope,
                actor: TurnActor::new(actor_user_id),
                accepted_message_ref: accepted_message_ref.clone(),
                source_binding_ref,
                reply_target_binding_ref,
                requested_run_profile: None,
                idempotency_key,
                received_at: chrono::Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
            };

            match coordinator.submit_turn(request).await {
                Ok(SubmitTurnResponse::Accepted {
                    turn_id,
                    run_id: submitted_run_id,
                    ..
                }) => {
                    debug!(
                        drained_message_id = %message.message_id,
                        submitted_turn_id = %turn_id,
                        submitted_run_id = %submitted_run_id,
                        triggering_run_id = %run_id,
                        "DeferredBusyDrainObserver: deferred message drained and submitted"
                    );
                    if let Err(error) = self
                        .thread_service
                        .mark_message_submitted(
                            thread_scope,
                            &scope.thread_id,
                            message.message_id,
                            turn_id.to_string(),
                            submitted_run_id.to_string(),
                        )
                        .await
                    {
                        warn!(
                            error = %error,
                            drained_message_id = %message.message_id,
                            "DeferredBusyDrainObserver: submitted to coordinator but failed to mark message as submitted"
                        );
                    }
                    // Stop after the first successful submit — the cascade
                    // will handle subsequent messages when this run terminates.
                    return Ok(());
                }
                Err(TurnError::ThreadBusy(busy)) => {
                    // A new run is already holding the lock — leave all
                    // deferred messages.  The drain will fire again when that
                    // run terminates.
                    debug!(
                        active_run_id = ?busy.active_run_id,
                        drained_message_id = %message.message_id,
                        triggering_run_id = %run_id,
                        "DeferredBusyDrainObserver: thread still busy after terminal event, leaving deferred"
                    );
                    return Ok(());
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        drained_message_id = %message.message_id,
                        triggering_run_id = %run_id,
                        "DeferredBusyDrainObserver: coordinator submit failed, leaving deferred"
                    );
                    return Ok(());
                }
            }
        }

        Ok(())
    }

    async fn drain_for_terminal_event(&self, event: &TurnLifecycleEvent) -> Result<(), TurnError> {
        let coordinator = match self.coordinator.get() {
            Some(c) => Arc::clone(c),
            None => {
                warn!(
                    run_id = %event.run_id,
                    "DeferredBusyDrainObserver: coordinator not bound, skipping drain"
                );
                return Ok(());
            }
        };

        let thread_scope = match thread_scope_from_event(event) {
            Ok(scope) => scope,
            Err(reason) => {
                debug!(
                    run_id = %event.run_id,
                    reason,
                    "DeferredBusyDrainObserver: cannot derive thread scope, skipping drain"
                );
                return Ok(());
            }
        };

        self.drain_for_scope(&event.run_id, &event.scope, &thread_scope, &coordinator)
            .await
    }

    async fn drain_for_terminal_state(&self, state: &TurnRunState) -> Result<(), TurnError> {
        let coordinator = match self.coordinator.get() {
            Some(c) => Arc::clone(c),
            None => {
                warn!(
                    run_id = %state.run_id,
                    "DeferredBusyDrainObserver: coordinator not bound, skipping drain"
                );
                return Ok(());
            }
        };

        let thread_scope = match thread_scope_from_state(state) {
            Ok(scope) => scope,
            Err(reason) => {
                debug!(
                    run_id = %state.run_id,
                    reason,
                    "DeferredBusyDrainObserver: cannot derive thread scope from state, skipping drain"
                );
                return Ok(());
            }
        };

        self.drain_for_scope(&state.run_id, &state.scope, &thread_scope, &coordinator)
            .await
    }
}

#[async_trait]
impl<S> TurnCommittedEventObserver for DeferredBusyDrainObserver<S>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
{
    /// Observe terminal state publications (runner-origin transitions: complete,
    /// fail, cancel via runner, recovery, validated-loop-exit).
    ///
    /// This covers the normal production approval-run-completes path where
    /// `complete_run` goes through `publish_state`, not `publish_event`.
    fn observes_state(&self, state: &TurnRunState) -> bool {
        state.status.is_terminal()
    }

    /// Only observe terminal events — those are the ones that release the
    /// thread's active lock.
    fn observes_event(&self, event: &TurnLifecycleEvent) -> bool {
        event.status.is_terminal()
    }

    /// Handles runner-origin terminal transitions (complete_run, fail_run,
    /// cancel_run via runner, etc.) — the production path for approval runs.
    ///
    /// The idempotency key (`drain:<message_id>`) ensures a double-drain
    /// (one from state publication, one from any subsequent event publication
    /// for the same terminal transition) produces `AlreadySubmitted` on the
    /// second call rather than a duplicate run.
    async fn observe_committed_state(&self, state: TurnRunState) -> Result<(), TurnError> {
        if let Err(error) = self.drain_for_terminal_state(&state).await {
            warn!(
                run_id = %state.run_id,
                error = %error,
                "DeferredBusyDrainObserver: drain step returned error on state, continuing"
            );
        }
        Ok(())
    }

    async fn observe_committed_event(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        if let Err(error) = self.drain_for_terminal_event(&event).await {
            // Must not surface errors up — the terminal event path must always
            // succeed even when the drain step fails.
            warn!(
                run_id = %event.run_id,
                error = %error,
                "DeferredBusyDrainObserver: drain step returned error, continuing"
            );
        }
        Ok(())
    }
}

/// Derive a [`ThreadScope`] from a [`TurnLifecycleEvent`].
///
/// Returns `Err` with a human-readable reason when the scope cannot be derived
/// (e.g. agentless turn, missing owner).  The caller handles this as a
/// non-fatal skip.
fn thread_scope_from_event(event: &TurnLifecycleEvent) -> Result<ThreadScope, &'static str> {
    let Some(agent_id) = event.scope.agent_id.clone() else {
        return Err("agentless turn scope — no ThreadScope");
    };
    let owner_user_id = event
        .scope
        .thread_owner
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| event.owner_user_id.clone());
    Ok(ThreadScope {
        tenant_id: event.scope.tenant_id.clone(),
        agent_id,
        project_id: event.scope.project_id.clone(),
        owner_user_id,
        mission_id: None,
    })
}

/// Derive a [`ThreadScope`] from a [`TurnRunState`].
fn thread_scope_from_state(state: &TurnRunState) -> Result<ThreadScope, &'static str> {
    let Some(agent_id) = state.scope.agent_id.clone() else {
        return Err("agentless turn scope — no ThreadScope");
    };
    let owner_user_id = state
        .scope
        .thread_owner
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| state.actor.as_ref().map(|a| a.user_id.clone()));
    Ok(ThreadScope {
        tenant_id: state.scope.tenant_id.clone(),
        agent_id,
        project_id: state.scope.project_id.clone(),
        owner_user_id,
        mission_id: None,
    })
}

/// Resolve the actor `UserId` for the turn submission.
///
/// The drained message must resubmit as its ORIGINAL sender. The inbound
/// path always records `actor_id` for user messages; a record without one
/// is left deferred rather than misattributed to the thread owner.
fn resolve_actor_user_id(
    message: &ironclaw_threads::ThreadMessageRecord,
    _thread_scope: &ThreadScope,
) -> Result<UserId, String> {
    match message.actor_id.as_deref() {
        Some(actor_id) => UserId::new(actor_id).map_err(|e| format!("invalid actor_id: {e}")),
        None => Err(
            "deferred message has no actor_id; refusing to resubmit as thread owner".to_string(),
        ),
    }
}

// Tracing macros used in this module come from the `tracing` crate which is
// already a dependency of `ironclaw_reborn_composition` transitively via the
// `ironclaw_turns` and `ironclaw_threads` crate graph.
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
    use ironclaw_threads::{
        AcceptInboundMessageRequest, AcceptedInboundMessage, EnsureThreadRequest,
        InMemorySessionThreadService, MessageContent, MessageStatus, SessionThreadService,
        ThreadHistoryRequest, ThreadScope,
    };
    use ironclaw_turns::{
        AcceptedMessageRef, CancelRunRequest, DefaultTurnCoordinator, DefaultTurnLifecycleEventBus,
        IdempotencyKey, InMemoryTurnStateStore, LifecyclePublishingTurnStateStore,
        ReplyTargetBindingRef, SanitizedCancelReason, SourceBindingRef, SubmitTurnRequest,
        SubmitTurnResponse, TurnActor, TurnCommittedEventObserver, TurnCoordinator, TurnLeaseToken,
        TurnLifecycleEventBus, TurnRunId, TurnRunnerId, TurnScope,
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
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain")
                    .unwrap(),
                requested_run_profile: None,
                idempotency_key: IdempotencyKey::new(format!(
                    "turn:drain-test-{idempotency_suffix}"
                ))
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
    /// `source_binding_id` / `reply_target_binding_id` are the RAW values
    /// (without the "src:" / "reply:" prefix) as stored by the inbound turn
    /// path before `bounded_source_binding_ref` adds the prefix.
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
                // Raw binding ids — the drain will add the "src:"/"reply:"
                // prefix via bounded_source/reply_target_binding_ref.
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
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain")
                    .unwrap(),
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
            .mark_message_deferred_busy(&thread_scope(), &thread_id(), msg_b.message_id)
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
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain")
                    .unwrap(),
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
            .mark_message_deferred_busy(&thread_scope(), &thread_id(), msg_b.message_id)
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
        let b_run_id =
            TurnRunId::from_uuid(uuid::Uuid::parse_str(&b_run_id_str).expect("valid uuid"));
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
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain")
                    .unwrap(),
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
            .mark_message_deferred_busy(&thread_scope(), &thread_id(), msg_b.message_id)
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
                .mark_message_deferred_busy(&thread_scope(), &thread_id(), accepted.message_id)
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
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain")
                    .unwrap(),
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
            .mark_message_deferred_busy(&thread_scope(), &thread_id(), msg_c.message_id)
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
    // Scenario E: oversized binding id round-trip through defer → drain
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn drain_handles_oversized_binding_id_via_uuid_fallback() {
        let (coordinator, thread_service, _) = build_harness().await;

        // Step 1: Accept and submit message A — thread lock acquired.
        let msg_a = accept_message(
            &thread_service,
            "message A-oversize",
            "ext-event-a-oversize",
        )
        .await;
        let msg_a_ref = AcceptedMessageRef::new(format!("msg:{}", msg_a.message_id)).unwrap();
        let run_a = submit_run(coordinator.as_ref(), "a-oversize", msg_a_ref).await;

        // Step 2: Accept message B with a >256-byte raw binding id.
        // The drain should apply bounded_source_binding_ref and UUID-hash it
        // instead of failing with an invalid-length error.
        let oversized_binding_id = "x".repeat(300); // 300 bytes > 256 limit
        let msg_b = thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope(),
                thread_id: thread_id(),
                actor_id: actor().as_str().to_string(),
                source_binding_id: Some(oversized_binding_id.clone()),
                reply_target_binding_id: Some(oversized_binding_id.clone()),
                external_event_id: Some("ext-event-b-oversize".to_string()),
                content: MessageContent::text("message B-oversize"),
            })
            .await
            .expect("accept message B with oversized binding");
        match coordinator
            .submit_turn(SubmitTurnRequest {
                scope: turn_scope(),
                actor: TurnActor::new(actor()),
                accepted_message_ref: AcceptedMessageRef::new(format!("msg:{}", msg_b.message_id))
                    .unwrap(),
                source_binding_ref: SourceBindingRef::new("src:binding-drain").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:binding-drain")
                    .unwrap(),
                requested_run_profile: None,
                idempotency_key: IdempotencyKey::new("turn:drain-test-b-oversize").unwrap(),
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
            .mark_message_deferred_busy(&thread_scope(), &thread_id(), msg_b.message_id)
            .await
            .expect("mark B deferred busy");

        // Step 3: Cancel run A → drain fires with oversized binding id.
        // Should NOT leave B stuck — must submit successfully via UUID hash.
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
            "message with oversized binding id must be Submitted after drain (UUID fallback)"
        );
    }
}

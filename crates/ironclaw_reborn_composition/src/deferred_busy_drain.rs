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
use ironclaw_threads::{ListDeferredBusyMessagesRequest, SessionThreadService, ThreadScope};
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, ReplyTargetBindingRef, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCommittedEventObserver, TurnCoordinator, TurnError,
    TurnLifecycleEvent, TurnRunState, TurnScope,
};

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

        let deferred = match self
            .thread_service
            .list_deferred_busy_messages(ListDeferredBusyMessagesRequest {
                scope: thread_scope.clone(),
                thread_id: event.scope.thread_id.clone(),
            })
            .await
        {
            Ok(messages) => messages,
            Err(error) => {
                warn!(
                    run_id = %event.run_id,
                    error = %error,
                    "DeferredBusyDrainObserver: failed to list deferred messages, skipping drain"
                );
                return Ok(());
            }
        };

        let oldest = match deferred.into_iter().next() {
            Some(m) => m,
            None => return Ok(()),
        };

        // Build the coordinator submission from the thread record fields.
        let actor_user_id = match resolve_actor_user_id(&oldest, &thread_scope) {
            Ok(id) => id,
            Err(reason) => {
                warn!(
                    run_id = %event.run_id,
                    message_id = %oldest.message_id,
                    reason,
                    "DeferredBusyDrainObserver: cannot resolve actor for deferred message, leaving deferred"
                );
                return Ok(());
            }
        };
        let source_binding_ref = match oldest
            .source_binding_id
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            Some(raw) => match SourceBindingRef::new(raw) {
                Ok(r) => r,
                Err(reason) => {
                    warn!(
                        run_id = %event.run_id,
                        message_id = %oldest.message_id,
                        reason,
                        "DeferredBusyDrainObserver: invalid source_binding_id, leaving deferred"
                    );
                    return Ok(());
                }
            },
            None => {
                warn!(
                    run_id = %event.run_id,
                    message_id = %oldest.message_id,
                    "DeferredBusyDrainObserver: deferred message missing source_binding_id, leaving deferred"
                );
                return Ok(());
            }
        };
        let reply_target_binding_ref = match oldest
            .reply_target_binding_id
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            Some(raw) => match ReplyTargetBindingRef::new(raw) {
                Ok(r) => r,
                Err(reason) => {
                    warn!(
                        run_id = %event.run_id,
                        message_id = %oldest.message_id,
                        reason,
                        "DeferredBusyDrainObserver: invalid reply_target_binding_id, leaving deferred"
                    );
                    return Ok(());
                }
            },
            None => {
                warn!(
                    run_id = %event.run_id,
                    message_id = %oldest.message_id,
                    "DeferredBusyDrainObserver: deferred message missing reply_target_binding_id, leaving deferred"
                );
                return Ok(());
            }
        };
        let accepted_message_ref = match AcceptedMessageRef::new(format!(
            "msg:{}",
            oldest.message_id
        )) {
            Ok(r) => r,
            Err(reason) => {
                warn!(
                    run_id = %event.run_id,
                    message_id = %oldest.message_id,
                    reason,
                    "DeferredBusyDrainObserver: cannot build accepted_message_ref, leaving deferred"
                );
                return Ok(());
            }
        };
        // Use the message_id as the idempotency key so a duplicate drain
        // fire produces the same run rather than a second submission.
        let idempotency_key = match IdempotencyKey::new(format!("drain:{}", oldest.message_id)) {
            Ok(k) => k,
            Err(reason) => {
                warn!(
                    run_id = %event.run_id,
                    message_id = %oldest.message_id,
                    reason,
                    "DeferredBusyDrainObserver: cannot build idempotency key, leaving deferred"
                );
                return Ok(());
            }
        };
        let agent_id = match event.scope.agent_id.clone() {
            Some(id) => id,
            None => {
                debug!(
                    run_id = %event.run_id,
                    message_id = %oldest.message_id,
                    "DeferredBusyDrainObserver: agentless scope, skipping drain"
                );
                return Ok(());
            }
        };
        let turn_scope = TurnScope::new_with_owner(
            event.scope.tenant_id.clone(),
            Some(agent_id),
            event.scope.project_id.clone(),
            event.scope.thread_id.clone(),
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
                turn_id, run_id, ..
            }) => {
                debug!(
                    drained_message_id = %oldest.message_id,
                    submitted_turn_id = %turn_id,
                    submitted_run_id = %run_id,
                    triggering_run_id = %event.run_id,
                    "DeferredBusyDrainObserver: deferred message drained and submitted"
                );
                if let Err(error) = self
                    .thread_service
                    .mark_message_submitted(
                        &thread_scope,
                        &event.scope.thread_id,
                        oldest.message_id,
                        turn_id.to_string(),
                        run_id.to_string(),
                    )
                    .await
                {
                    warn!(
                        error = %error,
                        drained_message_id = %oldest.message_id,
                        "DeferredBusyDrainObserver: submitted to coordinator but failed to mark message as submitted"
                    );
                }
            }
            Err(TurnError::ThreadBusy(busy)) => {
                // A new run is already holding the lock — leave the message
                // deferred.  The drain will fire again when that run terminates.
                debug!(
                    active_run_id = ?busy.active_run_id,
                    drained_message_id = %oldest.message_id,
                    triggering_run_id = %event.run_id,
                    "DeferredBusyDrainObserver: thread still busy after terminal event, leaving deferred"
                );
            }
            Err(error) => {
                warn!(
                    error = %error,
                    drained_message_id = %oldest.message_id,
                    triggering_run_id = %event.run_id,
                    "DeferredBusyDrainObserver: coordinator submit failed, leaving deferred"
                );
            }
        }

        Ok(())
    }
}

#[async_trait]
impl<S> TurnCommittedEventObserver for DeferredBusyDrainObserver<S>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
{
    /// Only observe terminal events — those are the ones that release the
    /// thread's active lock.
    fn observes_event(&self, event: &TurnLifecycleEvent) -> bool {
        event.status.is_terminal()
    }

    /// State-based observation is not needed; drain is driven by committed
    /// events only.
    fn observes_state(&self, _state: &TurnRunState) -> bool {
        false
    }

    async fn observe_committed_state(&self, _state: TurnRunState) -> Result<(), TurnError> {
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

/// Resolve the actor `UserId` for the turn submission.
///
/// Tries `actor_id` from the message record first; falls back to
/// `thread_scope.owner_user_id`.
fn resolve_actor_user_id(
    message: &ironclaw_threads::ThreadMessageRecord,
    thread_scope: &ThreadScope,
) -> Result<UserId, String> {
    if let Some(actor_id) = message.actor_id.as_deref() {
        return UserId::new(actor_id).map_err(|e| format!("invalid actor_id: {e}"));
    }
    thread_scope
        .owner_user_id
        .clone()
        .ok_or_else(|| "deferred message has no actor_id and thread scope has no owner".to_string())
}

// Tracing macros used in this module come from the `tracing` crate which is
// already a dependency of `ironclaw_reborn_composition` transitively via the
// `ironclaw_turns` and `ironclaw_threads` crate graph.
use tracing::{debug, warn};

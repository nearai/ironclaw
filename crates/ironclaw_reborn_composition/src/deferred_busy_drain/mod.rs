//! Drains [`MessageStatus::DeferredBusy`] messages when a blocking run reaches
//! terminal state.
//!
//! When a run terminates the thread's active lock is released.  Any messages
//! that were parked with `DeferredBusy` because the thread was busy may now be
//! submitted.  This observer fires once per terminal event, picks up the
//! *oldest* deferred message for the affected thread, and resubmits it through
//! the coordinator.  One-at-a-time cascade semantics follow naturally: when
//! that resubmitted run terminates, this observer fires again and picks up the
//! next deferred message.
//!
//! # Failure contract
//!
//! Drain failures must **never** poison the terminal-event path.  The observer
//! logs a `warn!` and returns `Ok(())` so the run's own terminal state is
//! always committed even when the drain step fails.
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ironclaw_host_api::UserId;
use ironclaw_reborn::thread_scope::ThreadScopeResolver;
use ironclaw_threads::{ListDeferredBusyMessagesRequest, SessionThreadService, ThreadScope};
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, ReplyTargetBindingRef, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCommittedEventObserver, TurnCoordinator, TurnError,
    TurnLifecycleEvent, TurnRunState, TurnScope,
};
use tracing::{debug, warn};

/// Maximum number of `DeferredBusy` records loaded per drain window.
///
/// Each drain attempt pages through windows of this size, advancing past
/// entirely-invalid windows until a valid message is found or the total cap
/// is hit.  Must be ≤ `DRAIN_TOTAL_CAP` and large enough to make progress
/// past a few bad entries in a single window.
const DRAIN_LIST_LIMIT: usize = 8;

/// Hard cap on total deferred records examined per drain invocation.
///
/// Prevents a pathological sequence of 64+ invalid records from causing
/// unbounded service calls.  When this cap is hit any remaining deferred
/// messages are left for the next drain invocation.
const DRAIN_TOTAL_CAP: usize = 64;

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
    /// Iterates in sequence order (oldest first) in windows of `DRAIN_LIST_LIMIT`.
    /// A message that fails validation (bad actor, missing canonical refs, etc.)
    /// is logged at `warn!` and **skipped** — the loop continues to the next
    /// entry.  When an entire window is exhausted without finding a valid message,
    /// the next window is fetched starting after the last examined sequence.
    /// The total number of records examined across all windows is capped at
    /// `DRAIN_TOTAL_CAP`.
    ///
    /// On a successful submit the loop stops (cascade semantics take over when
    /// that run terminates).  On `ThreadBusy` the loop stops and all remaining
    /// messages stay deferred.
    ///
    /// Failed messages are NOT mutated — LLM-data retention rule applies.
    /// They will be re-examined on each subsequent drain call and skipped again,
    /// which is acceptable.
    async fn drain_for_scope(
        &self,
        run_id: &ironclaw_turns::TurnRunId,
        scope: &TurnScope,
        thread_scope: &ThreadScope,
        coordinator: &Arc<dyn TurnCoordinator>,
    ) -> Result<(), TurnError> {
        let mut after_sequence: Option<u64> = None;
        let mut total_examined: usize = 0;

        loop {
            if total_examined >= DRAIN_TOTAL_CAP {
                debug!(
                    run_id = %run_id,
                    total_examined,
                    "DeferredBusyDrainObserver: total examined cap reached, leaving rest for next drain"
                );
                return Ok(());
            }

            let window = match self
                .thread_service
                .list_deferred_busy_messages(ListDeferredBusyMessagesRequest {
                    scope: thread_scope.clone(),
                    thread_id: scope.thread_id.clone(),
                    limit: Some(DRAIN_LIST_LIMIT),
                    after_sequence,
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

            if window.is_empty() {
                // No more deferred messages — nothing to drain.
                return Ok(());
            }

            let window_last_sequence = window.last().map(|m| m.sequence).unwrap_or(0);

            for message in window {
                if total_examined >= DRAIN_TOTAL_CAP {
                    debug!(
                        run_id = %run_id,
                        total_examined,
                        "DeferredBusyDrainObserver: total examined cap reached, leaving rest for next drain"
                    );
                    return Ok(());
                }
                total_examined += 1;

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

                // Use the canonical refs persisted at defer time.  Records
                // persisted before canonical binding refs existed have `None`
                // here and are skipped.
                let source_binding_ref = match message.turn_source_binding_ref.as_deref() {
                    Some(canonical) => match SourceBindingRef::new(canonical) {
                        Ok(r) => r,
                        Err(reason) => {
                            warn!(
                                run_id = %run_id,
                                message_id = %message.message_id,
                                reason,
                                "DeferredBusyDrainObserver: invalid persisted turn_source_binding_ref, skipping"
                            );
                            continue;
                        }
                    },
                    None => {
                        warn!(
                            run_id = %run_id,
                            message_id = %message.message_id,
                            "DeferredBusyDrainObserver: deferred message missing turn_source_binding_ref (legacy record), skipping"
                        );
                        continue;
                    }
                };

                let reply_target_binding_ref = match message
                    .turn_reply_target_binding_ref
                    .as_deref()
                {
                    Some(canonical) => match ReplyTargetBindingRef::new(canonical) {
                        Ok(r) => r,
                        Err(reason) => {
                            warn!(
                                run_id = %run_id,
                                message_id = %message.message_id,
                                reason,
                                "DeferredBusyDrainObserver: invalid persisted turn_reply_target_binding_ref, skipping"
                            );
                            continue;
                        }
                    },
                    None => {
                        warn!(
                            run_id = %run_id,
                            message_id = %message.message_id,
                            "DeferredBusyDrainObserver: deferred message missing turn_reply_target_binding_ref (legacy record), skipping"
                        );
                        continue;
                    }
                };

                let accepted_message_ref = match AcceptedMessageRef::new(format!(
                    "msg:{}",
                    message.message_id
                )) {
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
                let idempotency_key =
                    match IdempotencyKey::new(format!("drain:{}", message.message_id)) {
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
                // Reached only via the loop-continue paths above (validation skips);
                // every submit arm returns before reaching here.
            }

            // Entire window was skipped (all validation failures).
            // Advance past this window and fetch the next one.
            debug!(
                run_id = %run_id,
                window_last_sequence,
                total_examined,
                "DeferredBusyDrainObserver: full window skipped, advancing past sequence {window_last_sequence}"
            );
            after_sequence = Some(window_last_sequence);
        }
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

        let thread_scope = match ThreadScopeResolver::derive_for_terminal_event(event) {
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

        let thread_scope = match ThreadScopeResolver::derive_for_terminal_state(state) {
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

#[cfg(test)]
mod tests;

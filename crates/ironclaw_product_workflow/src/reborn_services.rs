//! WebUI-facing Reborn service facade.
//!
//! This module is the stable high-level API beta WebUI route handlers use
//! instead of reaching into turn coordination, thread stores, runtime lanes, DB
//! stores, dispatchers, or capability hosts directly.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, ThreadId};
use ironclaw_product_adapters::{
    ProductAdapterError, ProductWorkflowRejectionKind, ProjectionStream,
    ProjectionSubscriptionRequest,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessageReplay, EnsureThreadRequest, MessageContent,
    MessageStatus, ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadService,
    ThreadHistoryRequest, ThreadMessageId, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, GateRef, GetRunStateRequest, IdempotencyKey, ReplyTargetBindingRef,
    ResumeTurnRequest, SanitizedCancelReason, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnRunId, TurnScope,
};
use uuid::Uuid;

use crate::{
    WebUiAuthenticatedCaller, WebUiCancelRunRequest, WebUiCreateThreadRequest, WebUiGateResolution,
    WebUiInboundCommand, WebUiInboundValidationCode, WebUiInboundValidationError,
    WebUiResolveGateRequest, WebUiSendMessageRequest,
};

mod error;
mod types;

pub use error::{RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind};
pub use types::{
    RebornCancelRunResponse, RebornCreateThreadResponse, RebornGetRunStateRequest,
    RebornGetRunStateResponse, RebornResolveGateResponse, RebornResumeGateResponse,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornSubmitTurnResponse,
    RebornTimelineRequest, RebornTimelineResponse,
};

/// Stable WebUI-facing facade surface for beta Reborn routes.
#[async_trait]
pub trait RebornServicesApi: Send + Sync {
    async fn create_thread(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError>;

    async fn submit_turn(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError>;

    async fn get_timeline(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError>;

    async fn stream_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError>;

    async fn cancel_run(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError>;

    async fn resolve_gate(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError>;

    async fn get_run_state(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError>;
}

/// Default facade implementation composed at the WebUI boundary.
#[derive(Clone)]
pub struct RebornServices {
    thread_service: Arc<dyn SessionThreadService>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    event_stream: Option<Arc<dyn ProjectionStream>>,
}

impl RebornServices {
    pub fn new(
        thread_service: Arc<dyn SessionThreadService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self {
            thread_service,
            turn_coordinator,
            event_stream: None,
        }
    }

    pub fn with_event_stream(mut self, event_stream: Arc<dyn ProjectionStream>) -> Self {
        self.event_stream = Some(event_stream);
        self
    }
}

#[async_trait]
impl RebornServicesApi for RebornServices {
    /// `requested_thread_id` makes the caller's choice authoritative.
    /// Without it, `client_action_id` deterministically derives the thread id
    /// so a retry of the same create maps back to the same thread.
    ///
    /// When the caller supplies an explicit `requested_thread_id`, an
    /// `ensure_thread` collision with a thread owned by another user is
    /// remapped to `NotFound` rather than the underlying `409 Conflict`.
    /// Otherwise the 400/409 distinction would be an existence oracle:
    /// callers sharing the same (tenant, agent, project) scope could probe
    /// for thread ids they did not create.
    async fn create_thread(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
        let command = request.into_command(caller)?;
        let WebUiInboundCommand::CreateThread {
            caller,
            client_action_id,
            requested_thread_id,
        } = command
        else {
            return Err(RebornServicesError::internal_invariant());
        };
        let caller_supplied_id = requested_thread_id.is_some();
        let thread_id =
            requested_thread_id.unwrap_or_else(|| generated_thread_id(&caller, &client_action_id));
        let scope = caller.turn_scope(thread_id.clone());
        let thread_scope = thread_scope_from_turn_scope(&scope, Some(caller.user_id.clone()))?;
        let thread = self
            .thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope,
                thread_id: Some(thread_id),
                created_by_actor_id: caller.user_id.as_str().to_string(),
                title: None,
                metadata_json: Some(create_thread_metadata_json(&client_action_id)?),
            })
            .await
            .map_err(|error| {
                if caller_supplied_id {
                    map_ownership_probe_error(error)
                } else {
                    // Deterministic generated ids derive from caller scope so
                    // a cross-user collision implies a UUIDv5 hash collision,
                    // which is not an oracle the caller can usefully probe.
                    // Preserve the underlying mapping for diagnosability.
                    map_thread_error(error)
                }
            })?;
        Ok(RebornCreateThreadResponse { thread })
    }

    async fn submit_turn(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
        let command = request.into_command(caller)?;
        let WebUiInboundCommand::SendMessage {
            scope,
            actor,
            client_action_id,
            content,
        } = command
        else {
            return Err(RebornServicesError::internal_invariant());
        };

        let (scope, thread_scope) = self.resolve_webui_thread_metadata(scope, &actor).await?;
        let source_binding_id = webui_source_binding_id(&scope, &actor);
        let external_event_id = client_action_id.as_str().to_string();

        let handoff = if let Some((replay, replay_source_binding_id)) = replay_webui_send_message(
            &*self.thread_service,
            &thread_scope,
            &scope,
            &actor,
            &external_event_id,
        )
        .await?
        {
            if replay.thread_id != scope.thread_id {
                return Err(RebornServicesError::from_status_kind(
                    RebornServicesErrorCode::Conflict,
                    RebornServicesErrorKind::Duplicate,
                    409,
                    false,
                ));
            }
            match replay.status {
                MessageStatus::Submitted => {
                    let run_id = parse_replay_run_id(replay.turn_run_id)?;
                    let state = self
                        .turn_coordinator
                        .get_run_state(GetRunStateRequest {
                            scope: scope.clone(),
                            run_id,
                        })
                        .await
                        .map_err(map_turn_error)?;
                    return Ok(RebornSubmitTurnResponse::AlreadySubmitted {
                        thread_id: replay.thread_id,
                        accepted_message_ref: accepted_message_ref(replay.message_id.to_string())?,
                        run_id,
                        status: state.status,
                        event_cursor: state.event_cursor,
                    });
                }
                MessageStatus::Accepted | MessageStatus::DeferredBusy => AcceptedWebUiMessage {
                    thread_id: replay.thread_id,
                    message_id: replay.message_id,
                    actor_id: actor.user_id.as_str().to_string(),
                    source_binding_id: replay
                        .source_binding_id
                        .unwrap_or_else(|| replay_source_binding_id.clone()),
                    reply_target_binding_id: replay
                        .reply_target_binding_id
                        .unwrap_or(replay_source_binding_id),
                },
                _ => {
                    return Err(RebornServicesError::from_status(
                        RebornServicesErrorCode::Conflict,
                        409,
                        false,
                    ));
                }
            }
        } else {
            let accepted = self
                .thread_service
                .accept_inbound_message(AcceptInboundMessageRequest {
                    scope: thread_scope.clone(),
                    thread_id: scope.thread_id.clone(),
                    actor_id: actor.user_id.as_str().to_string(),
                    source_binding_id: Some(source_binding_id.clone()),
                    reply_target_binding_id: Some(source_binding_id.clone()),
                    external_event_id: Some(external_event_id),
                    content: MessageContent::text(content),
                })
                .await
                .map_err(map_thread_error)?;
            AcceptedWebUiMessage {
                thread_id: accepted.thread_id,
                message_id: accepted.message_id,
                actor_id: actor.user_id.as_str().to_string(),
                source_binding_id: source_binding_id.clone(),
                reply_target_binding_id: source_binding_id.clone(),
            }
        };

        let accepted_message_ref = accepted_message_ref(handoff.message_id.to_string())?;
        let source_binding_ref =
            bounded_ref::<SourceBindingRef>("webui-src", &handoff.source_binding_id)?;
        let reply_target_binding_ref =
            bounded_ref::<ReplyTargetBindingRef>("webui-reply", &handoff.reply_target_binding_id)?;

        let submit = SubmitTurnRequest {
            scope,
            actor,
            accepted_message_ref: accepted_message_ref.clone(),
            source_binding_ref,
            reply_target_binding_ref,
            requested_run_profile: None,
            idempotency_key: client_action_id.clone(),
            received_at: Utc::now(),
        };

        match self.turn_coordinator.submit_turn(submit).await {
            Ok(SubmitTurnResponse::Accepted {
                turn_id,
                run_id,
                status,
                resolved_run_profile_id,
                resolved_run_profile_version,
                event_cursor,
                ..
            }) => {
                mark_message_submitted_or_replay(
                    &*self.thread_service,
                    &thread_scope,
                    &handoff,
                    &client_action_id,
                    turn_id.to_string(),
                    run_id.to_string(),
                )
                .await?;

                Ok(RebornSubmitTurnResponse::Submitted {
                    thread_id: handoff.thread_id,
                    accepted_message_ref,
                    turn_id: turn_id.to_string(),
                    run_id,
                    status,
                    resolved_run_profile_id: resolved_run_profile_id.as_str().to_string(),
                    resolved_run_profile_version: resolved_run_profile_version.as_u64(),
                    event_cursor,
                })
            }
            Err(TurnError::ThreadBusy(busy)) => {
                mark_message_deferred_busy_or_replay(
                    &*self.thread_service,
                    &thread_scope,
                    &handoff,
                    &client_action_id,
                )
                .await?;

                Ok(RebornSubmitTurnResponse::DeferredBusy {
                    thread_id: handoff.thread_id,
                    accepted_message_ref,
                    active_run_id: busy.active_run_id,
                    status: busy.status,
                    event_cursor: busy.event_cursor,
                })
            }
            Err(error) => Err(map_turn_error(error)),
        }
    }

    async fn get_timeline(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let actor = caller.actor();
        let limit = clamp_timeline_limit(request.limit);
        let cursor = parse_timeline_cursor(request.cursor.as_deref())?;
        let scope = caller.turn_scope(thread_id);
        let thread_scope = thread_scope_from_turn_scope(&scope, Some(actor.user_id.clone()))?;
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope,
                thread_id: scope.thread_id.clone(),
            })
            .await
            .map_err(map_timeline_probe_error)?;

        let (messages, next_cursor) = paginate_timeline_messages(history.messages, limit, cursor);
        let summary_artifacts = cap_summary_artifacts(history.summary_artifacts);

        Ok(RebornTimelineResponse {
            thread: history.thread,
            messages,
            summary_artifacts,
            next_cursor,
        })
    }

    async fn stream_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let actor = caller.actor();
        // Metadata-only ownership probe: the SSE handler calls
        // stream_events once per poll, and using list_thread_history here
        // would load the full message transcript + summary artifacts per
        // call — for an active stream that is hundreds of rows per second
        // per caller. resolve_webui_thread_metadata uses the cheap
        // read_thread probe; without it a caller sharing
        // (tenant, agent, project) could still read another user's
        // projection feed by guessing thread_id, so the probe itself
        // stays.
        let (scope, _thread_scope) = self
            .resolve_webui_thread_metadata(caller.turn_scope(thread_id), &actor)
            .await?;
        let Some(event_stream) = &self.event_stream else {
            return Err(RebornServicesError::from_status_kind(
                RebornServicesErrorCode::Unavailable,
                RebornServicesErrorKind::ReplayUnavailable,
                503,
                false,
            ));
        };
        let events = event_stream
            .drain(ProjectionSubscriptionRequest {
                actor,
                scope,
                after_cursor: request.after_cursor,
            })
            .await
            .map_err(map_projection_error)?;
        Ok(RebornStreamEventsResponse { events })
    }

    async fn cancel_run(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError> {
        let command = request.into_command(caller)?;
        let WebUiInboundCommand::CancelRun { request } = command else {
            return Err(RebornServicesError::internal_invariant());
        };
        // Metadata-only ownership probe — cancel_run has no use for the
        // message transcript and the load would be wasted work.
        self.resolve_webui_thread_metadata(request.scope.clone(), &request.actor)
            .await?;
        let response = self
            .turn_coordinator
            .cancel_run(request)
            .await
            .map_err(map_turn_error)?;
        Ok(response.into())
    }

    async fn resolve_gate(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        let command = request.into_command(caller)?;
        let WebUiInboundCommand::ResolveGate {
            scope,
            actor,
            run_id,
            gate_ref,
            client_action_id,
            resolution,
        } = command
        else {
            return Err(RebornServicesError::internal_invariant());
        };

        // Metadata-only ownership probe — resolve_gate has no use for
        // the message transcript and the load would be wasted work.
        self.resolve_webui_thread_metadata(scope.clone(), &actor)
            .await?;
        match resolution {
            WebUiGateResolution::Approved { always } => {
                // `always: true` requests a *persistent* approval but this
                // facade has only one-shot `resume_turn` and no approval-policy
                // port. Fail loud rather than silently downgrade.
                if always {
                    return Err(RebornServicesError::from_status_kind(
                        RebornServicesErrorCode::Unavailable,
                        RebornServicesErrorKind::BlockedApproval,
                        503,
                        false,
                    ));
                }
                let binding_id = webui_gate_binding_id(&scope, &gate_ref_string(&gate_ref));
                let response = self
                    .turn_coordinator
                    .resume_turn(ResumeTurnRequest {
                        scope,
                        actor,
                        run_id,
                        gate_resolution_ref: gate_ref,
                        source_binding_ref: bounded_ref::<SourceBindingRef>(
                            "webui-gate-src",
                            &binding_id,
                        )?,
                        reply_target_binding_ref: bounded_ref::<ReplyTargetBindingRef>(
                            "webui-gate-reply",
                            &binding_id,
                        )?,
                        idempotency_key: client_action_id,
                    })
                    .await
                    .map_err(map_turn_error)?;
                Ok(RebornResolveGateResponse::Resumed(response.into()))
            }
            WebUiGateResolution::CredentialProvided { .. } => {
                Err(RebornServicesError::from_status_kind(
                    RebornServicesErrorCode::Unavailable,
                    RebornServicesErrorKind::BlockedAuthentication,
                    503,
                    false,
                ))
            }
            WebUiGateResolution::Denied | WebUiGateResolution::Cancelled => {
                // `cancel_run` is not gate-aware, so without this check a
                // denied/cancelled resolution for a stale or attacker-supplied
                // gate_ref would terminate any non-terminal run sharing run_id.
                assert_run_parked_on_gate(
                    self.turn_coordinator.as_ref(),
                    &scope,
                    run_id,
                    &gate_ref,
                )
                .await?;
                let response = self
                    .turn_coordinator
                    .cancel_run(ironclaw_turns::CancelRunRequest {
                        scope,
                        actor,
                        run_id,
                        reason: SanitizedCancelReason::UserRequested,
                        idempotency_key: client_action_id,
                    })
                    .await
                    .map_err(map_turn_error)?;
                Ok(RebornResolveGateResponse::Cancelled(response.into()))
            }
        }
    }

    async fn get_run_state(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let run_id = parse_run_id_field("run_id", request.run_id)?;
        let scope = caller.turn_scope(thread_id);
        let actor = caller.actor();
        // TurnScope has no owner_user_id, so without this gate any caller
        // sharing the (tenant, agent, project) scope could read another user's
        // run state by guessing thread_id and run_id. Mirrors the ownership
        // probe `cancel_run` and `resolve_gate` already perform.
        // Metadata-only — get_run_state has no use for the transcript.
        self.resolve_webui_thread_metadata(scope.clone(), &actor)
            .await?;
        let state = self
            .turn_coordinator
            .get_run_state(GetRunStateRequest { scope, run_id })
            .await
            .map_err(map_turn_error)?;
        Ok(state.into())
    }
}

struct AcceptedWebUiMessage {
    thread_id: ThreadId,
    message_id: ThreadMessageId,
    actor_id: String,
    source_binding_id: String,
    reply_target_binding_id: String,
}

async fn mark_message_submitted_or_replay(
    thread_service: &dyn SessionThreadService,
    thread_scope: &ThreadScope,
    handoff: &AcceptedWebUiMessage,
    client_action_id: &IdempotencyKey,
    turn_id: String,
    run_id: String,
) -> Result<(), RebornServicesError> {
    match thread_service
        .mark_message_submitted(
            thread_scope,
            &handoff.thread_id,
            handoff.message_id,
            turn_id,
            run_id.clone(),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            reconcile_terminal_duplicate(
                thread_service,
                thread_scope,
                handoff,
                client_action_id,
                |replay| {
                    replay.status == MessageStatus::Submitted && replay.turn_run_id == Some(run_id)
                },
                error,
            )
            .await
        }
    }
}

async fn mark_message_deferred_busy_or_replay(
    thread_service: &dyn SessionThreadService,
    thread_scope: &ThreadScope,
    handoff: &AcceptedWebUiMessage,
    client_action_id: &IdempotencyKey,
) -> Result<(), RebornServicesError> {
    match thread_service
        .mark_message_deferred_busy(thread_scope, &handoff.thread_id, handoff.message_id)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            reconcile_terminal_duplicate(
                thread_service,
                thread_scope,
                handoff,
                client_action_id,
                |replay| replay.status == MessageStatus::DeferredBusy,
                error,
            )
            .await
        }
    }
}

async fn reconcile_terminal_duplicate(
    thread_service: &dyn SessionThreadService,
    thread_scope: &ThreadScope,
    handoff: &AcceptedWebUiMessage,
    client_action_id: &IdempotencyKey,
    matches_replay: impl FnOnce(&AcceptedInboundMessageReplay) -> bool,
    original_error: SessionThreadError,
) -> Result<(), RebornServicesError> {
    let replay = thread_service
        .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
            scope: thread_scope.clone(),
            actor_id: handoff.actor_id.clone(),
            source_binding_id: handoff.source_binding_id.clone(),
            external_event_id: client_action_id.as_str().to_string(),
        })
        .await
        .map_err(map_thread_error)?;
    match replay {
        Some(replay)
            if replay.thread_id == handoff.thread_id
                && replay.message_id == handoff.message_id
                && matches_replay(&replay) =>
        {
            Ok(())
        }
        _ => Err(map_thread_error(original_error)),
    }
}

async fn replay_webui_send_message(
    thread_service: &dyn SessionThreadService,
    thread_scope: &ThreadScope,
    scope: &TurnScope,
    actor: &TurnActor,
    external_event_id: &str,
) -> Result<Option<(AcceptedInboundMessageReplay, String)>, RebornServicesError> {
    let source_binding_id = webui_source_binding_id(scope, actor);
    if let Some(replay) = replay_accepted_message(
        thread_service,
        thread_scope,
        actor,
        &source_binding_id,
        external_event_id,
    )
    .await?
    {
        return Ok(Some((replay, source_binding_id)));
    }

    let legacy_source_binding_id = legacy_webui_source_binding_id(scope, actor);
    replay_accepted_message(
        thread_service,
        thread_scope,
        actor,
        &legacy_source_binding_id,
        external_event_id,
    )
    .await
    .map(|replay| replay.map(|replay| (replay, legacy_source_binding_id)))
}

async fn replay_accepted_message(
    thread_service: &dyn SessionThreadService,
    thread_scope: &ThreadScope,
    actor: &TurnActor,
    source_binding_id: &str,
    external_event_id: &str,
) -> Result<Option<AcceptedInboundMessageReplay>, RebornServicesError> {
    thread_service
        .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
            scope: thread_scope.clone(),
            actor_id: actor.user_id.as_str().to_string(),
            source_binding_id: source_binding_id.to_string(),
            external_event_id: external_event_id.to_string(),
        })
        .await
        .map_err(map_thread_error)
}

// Owner-bound thread resolution shared by the WebUI-facing methods that
// only need to prove a browser thread id belongs to the authenticated actor.
// The actor is pinned as `owner_user_id` so a caller sharing (tenant, agent,
// project) cannot act on a thread it does not own; `map_ownership_probe_error`
// collapses both UnknownThread and ThreadScopeMismatch into NotFound so the
// response cannot be used as an existence oracle.
impl RebornServices {
    async fn resolve_webui_thread_metadata(
        &self,
        scope: TurnScope,
        actor: &TurnActor,
    ) -> Result<(TurnScope, ThreadScope), RebornServicesError> {
        let thread_scope = thread_scope_from_turn_scope(&scope, Some(actor.user_id.clone()))?;
        // `read_thread` is the metadata-only probe; production backends
        // override it to skip the message/summary load entirely. The
        // ownership semantics (UnknownThread / ThreadScopeMismatch
        // collapse to NotFound) must match `list_thread_history`'s
        // path, which `map_ownership_probe_error` guarantees.
        self.thread_service
            .read_thread(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: scope.thread_id.clone(),
            })
            .await
            .map_err(map_ownership_probe_error)?;
        Ok((scope, thread_scope))
    }
}

/// Ownership probes must collapse "thread does not exist" and "thread exists
/// but is owned by another caller" into NotFound so that a caller sharing the
/// (tenant, agent, project) scope cannot tell whether the supplied `thread_id`
/// matches a real thread under a different owner. The current backends return
/// `UnknownThread` for both cases on `list_thread_history`, but the contract
/// also permits `ThreadScopeMismatch`; remap it explicitly so a future backend
/// change cannot silently reintroduce an existence-leak.
fn map_ownership_probe_error(error: SessionThreadError) -> RebornServicesError {
    match &error {
        SessionThreadError::ThreadScopeMismatch { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
        }
        _ => map_thread_error(error),
    }
}

/// Reject denied/cancelled gate resolutions whose `gate_ref` does not match the
/// gate the run is actually parked on. `cancel_run` is not gate-aware, so
/// without this guard a stale or attacker-supplied `gate_ref` would cancel any
/// non-terminal run sharing the same `run_id`.
async fn assert_run_parked_on_gate(
    turn_coordinator: &dyn TurnCoordinator,
    scope: &TurnScope,
    run_id: TurnRunId,
    expected_gate_ref: &GateRef,
) -> Result<(), RebornServicesError> {
    let state = turn_coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .map_err(map_turn_error)?;
    match state.gate_ref.as_ref() {
        Some(parked) if parked == expected_gate_ref => Ok(()),
        _ => Err(RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::BlockedApproval,
            409,
            false,
        )),
    }
}

fn thread_scope_from_turn_scope(
    scope: &TurnScope,
    owner_user_id: Option<ironclaw_host_api::UserId>,
) -> Result<ThreadScope, RebornServicesError> {
    let Some(agent_id) = scope.agent_id.clone() else {
        return Err(RebornServicesError::from_status(
            RebornServicesErrorCode::InvalidRequest,
            400,
            false,
        ));
    };
    Ok(ThreadScope {
        tenant_id: scope.tenant_id.clone(),
        agent_id,
        project_id: scope.project_id.clone(),
        owner_user_id,
        mission_id: None,
    })
}

fn parse_thread_id_field(
    field: &'static str,
    value: String,
) -> Result<ThreadId, RebornServicesError> {
    ThreadId::new(value).map_err(|_| {
        RebornServicesError::validation(WebUiInboundValidationError::new(
            field,
            WebUiInboundValidationCode::InvalidId,
        ))
    })
}

fn parse_run_id_field(
    field: &'static str,
    value: String,
) -> Result<TurnRunId, RebornServicesError> {
    Uuid::parse_str(&value)
        .map(TurnRunId::from_uuid)
        .map_err(|_| {
            RebornServicesError::validation(WebUiInboundValidationError::new(
                field,
                WebUiInboundValidationCode::InvalidId,
            ))
        })
}

fn accepted_message_ref(message_id: String) -> Result<AcceptedMessageRef, RebornServicesError> {
    AcceptedMessageRef::new(format!("msg:{message_id}")).map_err(|_| {
        RebornServicesError::from_status(RebornServicesErrorCode::Internal, 500, false)
    })
}

fn parse_replay_run_id(value: Option<String>) -> Result<TurnRunId, RebornServicesError> {
    let Some(value) = value else {
        return Err(RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::ReplayUnavailable,
            409,
            false,
        ));
    };
    Uuid::parse_str(&value)
        .map(TurnRunId::from_uuid)
        .map_err(|_| {
            RebornServicesError::from_status_kind(
                RebornServicesErrorCode::Conflict,
                RebornServicesErrorKind::ReplayUnavailable,
                409,
                false,
            )
        })
}

trait RefFactory: Sized {
    fn build(value: String) -> Result<Self, String>;
}

impl RefFactory for SourceBindingRef {
    fn build(value: String) -> Result<Self, String> {
        Self::new(value)
    }
}

impl RefFactory for ReplyTargetBindingRef {
    fn build(value: String) -> Result<Self, String> {
        Self::new(value)
    }
}

fn bounded_ref<T: RefFactory>(prefix: &str, raw: &str) -> Result<T, RebornServicesError> {
    let value = if raw.len() <= 240 && !raw.chars().any(|c| c == '\0' || c.is_control()) {
        format!("{prefix}:{raw}")
    } else {
        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, raw.as_bytes());
        format!("{prefix}:{id}")
    };
    T::build(value).map_err(|_| {
        RebornServicesError::from_status(RebornServicesErrorCode::Internal, 500, false)
    })
}

fn webui_source_binding_id(scope: &TurnScope, actor: &TurnActor) -> String {
    // WebUI retries are scoped to the authenticated caller context, not the
    // thread id. When the caller is not project-bound, we encode that
    // explicitly rather than collapsing onto an empty string.
    format!(
        "{}{}{}{}{}{}",
        segment("surface", "webui"),
        segment("tenant", scope.tenant_id.as_str()),
        segment(
            "agent",
            scope.agent_id.as_ref().map(AgentId::as_str).unwrap_or("")
        ),
        segment(
            "project_scope",
            if scope.project_id.is_some() {
                "bound"
            } else {
                "none"
            }
        ),
        scope
            .project_id
            .as_ref()
            .map(|project_id| segment("project", project_id.as_str()))
            .unwrap_or_default(),
        segment("actor", actor.user_id.as_str())
    )
}

fn legacy_webui_source_binding_id(scope: &TurnScope, actor: &TurnActor) -> String {
    format!(
        "{}{}{}{}{}",
        segment("surface", "webui"),
        segment("tenant", scope.tenant_id.as_str()),
        segment(
            "agent",
            scope.agent_id.as_ref().map(AgentId::as_str).unwrap_or("")
        ),
        segment("thread", scope.thread_id.as_str()),
        segment("actor", actor.user_id.as_str())
    )
}

/// Default page size for [`RebornServicesApi::get_timeline`] when the
/// caller does not supply one. Sized to cover a typical chat history
/// without forcing a multi-megabyte JSON response on first load.
pub(crate) const TIMELINE_DEFAULT_PAGE_SIZE: u32 = 100;

/// Hard ceiling on the number of messages a single timeline response can
/// carry. Callers asking for more get the cap. Without this, a large
/// thread would let the per-route rate limit be the only thing bounding
/// per-request response size, which was the original Medium review
/// issue.
pub(crate) const TIMELINE_MAX_PAGE_SIZE: u32 = 200;

/// Hard ceiling on summary artifacts returned per response. Summary
/// artifacts are typically much smaller than the message transcript so
/// this cap is generous; it exists to bound the worst case where a
/// thread accumulates an unusual number of summaries.
const TIMELINE_MAX_SUMMARY_ARTIFACTS: usize = 200;

fn clamp_timeline_limit(requested: Option<u32>) -> usize {
    let raw = requested.unwrap_or(TIMELINE_DEFAULT_PAGE_SIZE);
    let clamped = raw.clamp(1, TIMELINE_MAX_PAGE_SIZE);
    clamped as usize
}

/// Wire shape of the opaque timeline cursor. The browser does not need
/// to interpret this; it just echoes the previous response's
/// `next_cursor` back as the next request's `cursor`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TimelineCursor {
    /// Only return messages whose `sequence` is strictly less than this
    /// value. Naming is deliberate: `before_*` makes the directional
    /// semantics (page backward through history) obvious at call sites.
    before_message_sequence: u64,
}

fn parse_timeline_cursor(raw: Option<&str>) -> Result<Option<TimelineCursor>, RebornServicesError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.is_empty() {
        return Ok(None);
    }
    let cursor: TimelineCursor = serde_json::from_str(raw).map_err(|_| {
        RebornServicesError::validation(WebUiInboundValidationError::new(
            "cursor",
            WebUiInboundValidationCode::InvalidValue,
        ))
    })?;
    Ok(Some(cursor))
}

fn serialize_timeline_cursor(cursor: &TimelineCursor) -> Option<String> {
    // Serialization of a tiny tagged struct is total in practice, but
    // returning Option keeps the call site honest without an unwrap.
    serde_json::to_string(cursor).ok()
}

/// Slice the message transcript to the most recent `limit` messages
/// strictly older than `cursor.before_message_sequence` (or the most
/// recent `limit` overall when no cursor is supplied), returning the
/// page plus the cursor the caller should pass back to load the page
/// preceding this one. `None` for `next_cursor` means there is nothing
/// older — the caller has reached the start of the thread.
///
/// Messages are sorted by `sequence` ascending before slicing so the
/// returned page is in monotonic order regardless of the input order
/// the underlying store happens to produce.
fn paginate_timeline_messages(
    mut messages: Vec<ironclaw_threads::ThreadMessageRecord>,
    limit: usize,
    cursor: Option<TimelineCursor>,
) -> (Vec<ironclaw_threads::ThreadMessageRecord>, Option<String>) {
    messages.sort_by_key(|message| message.sequence);
    if let Some(cursor) = cursor.as_ref() {
        messages.retain(|message| message.sequence < cursor.before_message_sequence);
    }
    let total = messages.len();
    let start = total.saturating_sub(limit);
    let next_cursor = if start > 0 {
        // The next page is older than the oldest message in *this* page.
        // We take the sequence of the page's first (oldest) message and
        // use it as `before_message_sequence` for the follow-up: that
        // request returns messages with sequence < this one, i.e. the
        // page strictly preceding the current one.
        messages.get(start).and_then(|message| {
            serialize_timeline_cursor(&TimelineCursor {
                before_message_sequence: message.sequence,
            })
        })
    } else {
        None
    };
    let page: Vec<_> = messages.into_iter().skip(start).collect();
    (page, next_cursor)
}

fn cap_summary_artifacts(
    mut artifacts: Vec<ironclaw_threads::SummaryArtifact>,
) -> Vec<ironclaw_threads::SummaryArtifact> {
    if artifacts.len() > TIMELINE_MAX_SUMMARY_ARTIFACTS {
        artifacts.truncate(TIMELINE_MAX_SUMMARY_ARTIFACTS);
    }
    artifacts
}

fn webui_gate_binding_id(scope: &TurnScope, gate_ref: &str) -> String {
    format!(
        "{}{}{}{}",
        segment("surface", "webui"),
        segment("tenant", scope.tenant_id.as_str()),
        segment("thread", scope.thread_id.as_str()),
        segment("gate", gate_ref)
    )
}

fn gate_ref_string(gate_ref: &ironclaw_turns::GateRef) -> String {
    gate_ref.as_str().to_string()
}

fn segment(name: &str, value: &str) -> String {
    format!("{name}:{}:{value};", value.len())
}

fn map_timeline_probe_error(error: SessionThreadError) -> RebornServicesError {
    match error {
        SessionThreadError::Serialization(_)
        | SessionThreadError::Deserialization(_)
        | SessionThreadError::Backend(_) => RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::TimelineUnavailable,
            503,
            true,
        ),
        _ => map_ownership_probe_error(error),
    }
}

fn map_thread_error(error: SessionThreadError) -> RebornServicesError {
    match error {
        SessionThreadError::UnknownThread { .. } | SessionThreadError::UnknownMessage { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
        }
        SessionThreadError::IdempotentReplayThreadMismatch { .. } => {
            RebornServicesError::from_status_kind(
                RebornServicesErrorCode::Conflict,
                RebornServicesErrorKind::Duplicate,
                409,
                false,
            )
        }
        SessionThreadError::ThreadScopeMismatch { .. }
        | SessionThreadError::IdempotentReplayActorMismatch { .. }
        | SessionThreadError::InvalidMessageTransition { .. }
        | SessionThreadError::MessageNotDraft { .. }
        | SessionThreadError::InvalidSummaryRange { .. }
        | SessionThreadError::OverlappingSummaryRange { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::Conflict, 409, false)
        }
        SessionThreadError::GeneratedThreadId(_)
        | SessionThreadError::Serialization(_)
        | SessionThreadError::Deserialization(_)
        | SessionThreadError::Backend(_) => RebornServicesError::service_unavailable(true),
    }
}

fn map_turn_error(error: TurnError) -> RebornServicesError {
    let (code, kind, status_code, retryable) = match error.category() {
        ironclaw_turns::TurnErrorCategory::ThreadBusy => (
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::Busy,
            409,
            false,
        ),
        ironclaw_turns::TurnErrorCategory::Conflict => (
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::Conflict,
            409,
            false,
        ),
        ironclaw_turns::TurnErrorCategory::AdmissionRejected => (
            RebornServicesErrorCode::RateLimited,
            RebornServicesErrorKind::Busy,
            429,
            true,
        ),
        ironclaw_turns::TurnErrorCategory::ScopeNotFound => (
            RebornServicesErrorCode::NotFound,
            RebornServicesErrorKind::NotFound,
            404,
            false,
        ),
        ironclaw_turns::TurnErrorCategory::Unauthorized => (
            RebornServicesErrorCode::Forbidden,
            RebornServicesErrorKind::ParticipantDenied,
            403,
            false,
        ),
        ironclaw_turns::TurnErrorCategory::InvalidRequest => (
            RebornServicesErrorCode::InvalidRequest,
            RebornServicesErrorKind::Validation,
            400,
            false,
        ),
        ironclaw_turns::TurnErrorCategory::Unavailable => (
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ServiceUnavailable,
            503,
            true,
        ),
    };
    RebornServicesError::from_status_kind(code, kind, status_code, retryable)
}

fn map_adapter_error(error: ProductAdapterError) -> RebornServicesError {
    match error {
        ProductAdapterError::WorkflowRejected {
            kind,
            status_code,
            retryable,
            ..
        } => RebornServicesError::from_status_kind(
            code_for_status(status_code),
            kind_for_workflow_rejection(kind),
            status_code,
            retryable,
        ),
        ProductAdapterError::WorkflowTransient { .. }
        | ProductAdapterError::EgressTransient { .. } => {
            RebornServicesError::service_unavailable(true)
        }
        ProductAdapterError::Authentication(_) => {
            RebornServicesError::from_status(RebornServicesErrorCode::Unauthenticated, 401, false)
        }
        ProductAdapterError::MalformedInboundPayload { .. }
        | ProductAdapterError::InvalidIdentifier { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::InvalidRequest, 400, false)
        }
        ProductAdapterError::EgressDenied { .. }
        | ProductAdapterError::EgressUndeclaredHost { .. } => {
            RebornServicesError::from_status_kind(
                RebornServicesErrorCode::Forbidden,
                RebornServicesErrorKind::BlockedResource,
                403,
                false,
            )
        }
        ProductAdapterError::Internal { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::Internal, 500, false)
        }
    }
}

fn map_projection_error(error: ProductAdapterError) -> RebornServicesError {
    match error {
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unavailable,
            status_code,
            retryable,
            ..
        } => RebornServicesError::from_status_kind(
            code_for_status(status_code),
            RebornServicesErrorKind::ReplayUnavailable,
            status_code,
            retryable,
        ),
        ProductAdapterError::WorkflowTransient { .. }
        | ProductAdapterError::EgressTransient { .. } => RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ReplayUnavailable,
            503,
            true,
        ),
        _ => map_adapter_error(error),
    }
}

fn code_for_status(status_code: u16) -> RebornServicesErrorCode {
    match status_code {
        400 => RebornServicesErrorCode::InvalidRequest,
        401 => RebornServicesErrorCode::Unauthenticated,
        403 => RebornServicesErrorCode::Forbidden,
        404 => RebornServicesErrorCode::NotFound,
        409 => RebornServicesErrorCode::Conflict,
        429 => RebornServicesErrorCode::RateLimited,
        503 => RebornServicesErrorCode::Unavailable,
        _ => RebornServicesErrorCode::Internal,
    }
}

fn kind_for_workflow_rejection(kind: ProductWorkflowRejectionKind) -> RebornServicesErrorKind {
    match kind {
        ProductWorkflowRejectionKind::ThreadBusy
        | ProductWorkflowRejectionKind::AdmissionRejected => RebornServicesErrorKind::Busy,
        ProductWorkflowRejectionKind::ScopeNotFound => RebornServicesErrorKind::NotFound,
        ProductWorkflowRejectionKind::Unauthorized => RebornServicesErrorKind::ParticipantDenied,
        ProductWorkflowRejectionKind::InvalidRequest => RebornServicesErrorKind::Validation,
        ProductWorkflowRejectionKind::Unavailable => RebornServicesErrorKind::ServiceUnavailable,
        ProductWorkflowRejectionKind::Conflict => RebornServicesErrorKind::Conflict,
    }
}

fn create_thread_metadata_json(
    client_action_id: &ironclaw_turns::IdempotencyKey,
) -> Result<String, RebornServicesError> {
    serde_json::to_string(&serde_json::json!({
        "client_action_id": client_action_id.as_str(),
    }))
    .map_err(|_| RebornServicesError::internal_invariant())
}

fn generated_thread_id(
    caller: &WebUiAuthenticatedCaller,
    client_action_id: &ironclaw_turns::IdempotencyKey,
) -> ThreadId {
    let seed = format!(
        "{}{}{}{}{}{}",
        segment("surface", "webui-create-thread"),
        segment("tenant", caller.tenant_id.as_str()),
        segment("user", caller.user_id.as_str()),
        segment(
            "agent",
            caller.agent_id.as_ref().map(AgentId::as_str).unwrap_or("")
        ),
        segment(
            "project",
            caller
                .project_id
                .as_ref()
                .map(ironclaw_host_api::ProjectId::as_str)
                .unwrap_or("")
        ),
        segment("action", client_action_id.as_str())
    );
    let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, seed.as_bytes());
    // UUID text contains no path separators/control characters and is accepted by ThreadId.
    match ThreadId::new(id.to_string()) {
        Ok(thread_id) => thread_id,
        Err(error) => {
            debug_assert!(false, "generated UUID thread id should be valid: {error}");
            // Fallback remains valid under ThreadId validation rules.
            ThreadId::new("generated-thread-fallback").unwrap_or_else(|_| unreachable!())
        }
    }
}

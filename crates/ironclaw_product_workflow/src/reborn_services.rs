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
    AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent, MessageStatus,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadService,
    ThreadHistoryRequest, ThreadMessageId, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, GateRef, GetRunStateRequest, ReplyTargetBindingRef, ResumeTurnRequest,
    SanitizedCancelReason, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnCoordinator, TurnError, TurnRunId, TurnScope,
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
    RebornCancelRunResponse, RebornCreateThreadResponse, RebornResolveGateResponse,
    RebornResumeGateResponse, RebornStreamEventsRequest, RebornStreamEventsResponse,
    RebornSubmitTurnResponse, RebornTimelineRequest, RebornTimelineResponse,
};

/// Stable WebUI-facing facade surface for beta Reborn routes.
#[async_trait]
pub trait RebornServicesApi: Send + Sync {
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

        let thread_scope = thread_scope_from_turn_scope(&scope, Some(actor.user_id.clone()))?;
        let source_binding_id = webui_source_binding_id(&scope, &actor);
        let external_event_id = client_action_id.as_str().to_string();

        let handoff = if let Some(replay) = self
            .thread_service
            .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
                source_binding_id: source_binding_id.clone(),
                external_event_id: external_event_id.clone(),
            })
            .await
            .map_err(map_thread_error)?
        {
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
                    reply_target_binding_id: replay
                        .reply_target_binding_id
                        .unwrap_or_else(|| source_binding_id.clone()),
                },
                _ => {
                    return Err(RebornServicesError::from_status_kind(
                        RebornServicesErrorCode::Conflict,
                        RebornServicesErrorKind::Duplicate,
                        409,
                        false,
                    ));
                }
            }
        } else {
            self.thread_service
                .ensure_thread(EnsureThreadRequest {
                    scope: thread_scope.clone(),
                    thread_id: Some(scope.thread_id.clone()),
                    created_by_actor_id: actor.user_id.as_str().to_string(),
                    title: None,
                    metadata_json: None,
                })
                .await
                .map_err(map_thread_error)?;

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
                reply_target_binding_id: source_binding_id.clone(),
            }
        };

        let accepted_message_ref = accepted_message_ref(handoff.message_id.to_string())?;
        let source_binding_ref = bounded_ref::<SourceBindingRef>("webui-src", &source_binding_id)?;
        let reply_target_binding_ref =
            bounded_ref::<ReplyTargetBindingRef>("webui-reply", &handoff.reply_target_binding_id)?;

        let submit = SubmitTurnRequest {
            scope,
            actor,
            accepted_message_ref: accepted_message_ref.clone(),
            source_binding_ref,
            reply_target_binding_ref,
            requested_run_profile: None,
            idempotency_key: client_action_id,
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
                self.thread_service
                    .mark_message_submitted(
                        &thread_scope,
                        &handoff.thread_id,
                        handoff.message_id,
                        turn_id.to_string(),
                        run_id.to_string(),
                    )
                    .await
                    .map_err(map_thread_error)?;

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
                self.thread_service
                    .mark_message_deferred_busy(
                        &thread_scope,
                        &handoff.thread_id,
                        handoff.message_id,
                    )
                    .await
                    .map_err(map_thread_error)?;

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
        let scope = caller.turn_scope(thread_id.clone());
        let thread_scope = thread_scope_from_turn_scope(&scope, Some(caller.user_id.clone()))?;
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope,
                thread_id,
            })
            .await
            .map_err(map_timeline_error)?;

        Ok(RebornTimelineResponse {
            thread: history.thread,
            messages: history.messages,
            summary_artifacts: history.summary_artifacts,
        })
    }

    async fn stream_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
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
                actor: caller.actor(),
                scope: caller.turn_scope(thread_id),
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
        // TurnScope has no owner_user_id and the coordinator has no per-user
        // authority check, so without this gate any caller sharing the agent
        // scope could cancel another user's run by guessing run_id.
        assert_thread_owned_by(self.thread_service.as_ref(), &request.scope, &request.actor)
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
                assert_thread_owned_by(self.thread_service.as_ref(), &scope, &actor).await?;
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
                assert_thread_owned_by(self.thread_service.as_ref(), &scope, &actor).await?;
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
}

struct AcceptedWebUiMessage {
    thread_id: ThreadId,
    message_id: ThreadMessageId,
    reply_target_binding_id: String,
}

/// Optional create-thread helper for routes that want an explicit allocation
/// before first turn submission.
///
/// When `requested_thread_id` is omitted, `client_action_id` deterministically
/// replays the generated thread id. When callers provide an explicit thread id,
/// that id is the thread identity; conflicting explicit ids for the same client
/// action are not reconciled at this layer.
impl RebornServices {
    pub async fn create_thread(
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
            .map_err(map_thread_error)?;
        Ok(RebornCreateThreadResponse { thread })
    }
}

/// Verify the actor owns the thread referenced by `scope` through the thread
/// service. Used by `cancel_run` / `resolve_gate`, whose inputs carry a
/// `TurnScope` (no `owner_user_id` field) — without this check any caller
/// sharing the (tenant, agent, project) scope could affect another user's run
/// by guessing `thread_id` and `run_id`.
async fn assert_thread_owned_by(
    thread_service: &dyn SessionThreadService,
    scope: &TurnScope,
    actor: &TurnActor,
) -> Result<(), RebornServicesError> {
    let thread_scope = thread_scope_from_turn_scope(scope, Some(actor.user_id.clone()))?;
    thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope,
            thread_id: scope.thread_id.clone(),
        })
        .await
        .map_err(map_ownership_probe_error)?;
    Ok(())
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

fn map_thread_error(error: SessionThreadError) -> RebornServicesError {
    match error {
        SessionThreadError::UnknownThread { .. } | SessionThreadError::UnknownMessage { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::NotFound, 404, false)
        }
        SessionThreadError::ThreadScopeMismatch { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::Conflict, 409, false)
        }
        SessionThreadError::IdempotentReplayThreadMismatch { .. } => {
            RebornServicesError::from_status_kind(
                RebornServicesErrorCode::Conflict,
                RebornServicesErrorKind::Duplicate,
                409,
                false,
            )
        }
        SessionThreadError::InvalidMessageTransition { .. }
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

fn map_timeline_error(error: SessionThreadError) -> RebornServicesError {
    match error {
        SessionThreadError::Serialization(_)
        | SessionThreadError::Deserialization(_)
        | SessionThreadError::Backend(_) => RebornServicesError::from_status_kind(
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::TimelineUnavailable,
            503,
            true,
        ),
        _ => map_thread_error(error),
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

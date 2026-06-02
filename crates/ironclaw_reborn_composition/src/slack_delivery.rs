//! Slack final-reply delivery for immediate-ACK Reborn webhooks.
//!
//! Slack Events API requires the HTTP handler to return 2xx quickly. This
//! observer runs after the workflow accepts an inbound Slack message, waits for
//! the submitted run to finish, reads the finalized assistant reply, and sends it
//! through the host-mediated product outbound delivery seam.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    CommunicationPreferenceRepository, OutboundError, OutboundPolicyService, OutboundStateStore,
    ProjectionUpdateRef, ReplyTargetBindingClaim, ReplyTargetBindingValidator,
    ReplyTargetValidationRequest, RunNotificationContext, RunNotificationEventKind,
    RunNotificationOrigin, SourceRouteContext, ValidatedReplyTargetBinding,
};
use ironclaw_product_adapters::{
    ExternalActorRef, ExternalConversationRef, FinalReplyView, OutboundDeliverySink,
    ProductAdapter, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductOutboundPayload, ProductTriggerReason, ProtocolHttpEgress,
};
use ironclaw_product_workflow::{
    ConversationBindingService, ProductConversationRouteKind, ProductOutboundDeliveryRequest,
    ProductOutboundTargetResolver, ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
    VerifiedProductOutboundTargetMetadata, prepare_and_render_product_outbound,
};
use ironclaw_threads::{
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use ironclaw_turns::{
    GetRunStateRequest, ReplyTargetBindingRef, TurnActor, TurnCoordinator, TurnRunId, TurnScope,
    TurnStatus,
};
use ironclaw_wasm_product_adapters::ImmediateAckWorkflowObserver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlackFinalReplyDeliverySettings {
    pub poll_interval: Duration,
    pub max_wait: Duration,
}

impl Default for SlackFinalReplyDeliverySettings {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(250),
            max_wait: Duration::from_secs(120),
        }
    }
}

pub struct SlackFinalReplyDeliveryObserver {
    binding_service: Arc<dyn ConversationBindingService>,
    thread_service: Arc<dyn SessionThreadService>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    outbound_store: Arc<dyn OutboundStateStore>,
    communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    adapter: Arc<dyn ProductAdapter>,
    egress: Arc<dyn ProtocolHttpEgress>,
    delivery_sink: Arc<dyn OutboundDeliverySink>,
    settings: SlackFinalReplyDeliverySettings,
}

impl SlackFinalReplyDeliveryObserver {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        binding_service: Arc<dyn ConversationBindingService>,
        thread_service: Arc<dyn SessionThreadService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
        outbound_store: Arc<dyn OutboundStateStore>,
        communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
        adapter: Arc<dyn ProductAdapter>,
        egress: Arc<dyn ProtocolHttpEgress>,
        delivery_sink: Arc<dyn OutboundDeliverySink>,
    ) -> Self {
        Self::with_settings(
            binding_service,
            thread_service,
            turn_coordinator,
            outbound_store,
            communication_preferences,
            adapter,
            egress,
            delivery_sink,
            SlackFinalReplyDeliverySettings::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_settings(
        binding_service: Arc<dyn ConversationBindingService>,
        thread_service: Arc<dyn SessionThreadService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
        outbound_store: Arc<dyn OutboundStateStore>,
        communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
        adapter: Arc<dyn ProductAdapter>,
        egress: Arc<dyn ProtocolHttpEgress>,
        delivery_sink: Arc<dyn OutboundDeliverySink>,
        settings: SlackFinalReplyDeliverySettings,
    ) -> Self {
        Self {
            binding_service,
            thread_service,
            turn_coordinator,
            outbound_store,
            communication_preferences,
            adapter,
            egress,
            delivery_sink,
            settings,
        }
    }

    async fn deliver_final_reply(
        &self,
        envelope: ProductInboundEnvelope,
        ack: ProductInboundAck,
    ) -> Result<(), SlackFinalReplyDeliveryError> {
        let Some(run_id) = submitted_run_id(&ack) else {
            return Ok(());
        };
        let route_kind = route_kind_for_envelope(&envelope);
        let binding = self
            .binding_service
            .lookup_binding(resolve_binding_request(&envelope, route_kind))
            .await?;
        let scope = turn_scope_from_binding(&binding)?;
        let actor = TurnActor::new(binding.user_id.clone());
        let terminal_state = self.wait_for_terminal(&scope, run_id).await?;
        if terminal_state.status != TurnStatus::Completed {
            return Ok(());
        }
        let thread_scope = thread_scope_from_binding(&binding, route_kind)?;
        let Some(text) = self
            .read_latest_assistant_text(&thread_scope, &binding, run_id)
            .await?
        else {
            return Ok(());
        };
        let reply_target = terminal_state.reply_target_binding_ref.clone();
        let target_authority = ObservedSlackReplyTargetAuthority {
            scope: scope.clone(),
            actor: actor.clone(),
            expected_target: reply_target.clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            external_actor_ref: Some(envelope.external_actor_ref().clone()),
        };
        let projection_access_policy = AllowNoProjectionAccess;
        let outbound_policy = OutboundPolicyService::new(
            self.outbound_store.as_ref(),
            &projection_access_policy,
            &target_authority,
        );
        let projection_ref = ProjectionUpdateRef::new(format!("slack-final-reply:{run_id}"))
            .map_err(|reason| SlackFinalReplyDeliveryError::InvalidProjectionRef { reason })?;
        let delivery = ironclaw_outbound::PrepareCommunicationDeliveryRequest {
            resolution_request: CommunicationDeliveryResolutionRequest {
                scope,
                actor,
                modality: CommunicationModality::Text,
                intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    origin: RunNotificationOrigin::LiveSourceRoute {
                        source_route: SourceRouteContext {
                            reply_target_binding_ref: reply_target,
                        },
                    },
                }),
            },
            turn_run_id: Some(run_id),
            projection_ref,
            attempted_at: Utc::now(),
        };
        let payload = ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: run_id,
            text,
            generated_at: Utc::now(),
        });
        let _outcome = prepare_and_render_product_outbound(
            &outbound_policy,
            self.communication_preferences.as_ref(),
            &target_authority,
            ProductOutboundDeliveryRequest {
                delivery,
                payload,
                projection_cursor: ironclaw_product_adapters::ProjectionCursor::new(format!(
                    "slack-final-reply:{run_id}"
                ))
                .map_err(|error| {
                    SlackFinalReplyDeliveryError::InvalidProjectionRef {
                        reason: error.to_string(),
                    }
                })?,
                adapter: self.adapter.as_ref(),
                egress: self.egress.as_ref(),
                delivery_sink: self.delivery_sink.as_ref(),
            },
        )
        .await?;
        Ok(())
    }

    async fn wait_for_terminal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<ironclaw_turns::TurnRunState, SlackFinalReplyDeliveryError> {
        let start = Instant::now();
        loop {
            let state = self
                .turn_coordinator
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await?;
            if state.status.is_terminal() {
                return Ok(state);
            }
            if start.elapsed() >= self.settings.max_wait {
                return Err(SlackFinalReplyDeliveryError::RunWaitTimedOut { run_id });
            }
            tokio::time::sleep(self.settings.poll_interval).await;
        }
    }

    async fn read_latest_assistant_text(
        &self,
        thread_scope: &ThreadScope,
        binding: &ResolvedBinding,
        run_id: TurnRunId,
    ) -> Result<Option<String>, SlackFinalReplyDeliveryError> {
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: binding.thread_id.clone(),
            })
            .await?;
        let run_id = run_id.to_string();
        Ok(history
            .messages
            .into_iter()
            .rev()
            .find(|message| {
                matches!(message.kind, MessageKind::Assistant)
                    && matches!(message.status, MessageStatus::Finalized)
                    && message.turn_run_id.as_deref() == Some(run_id.as_str())
            })
            .and_then(|message| message.content))
    }
}

#[async_trait]
impl ImmediateAckWorkflowObserver for SlackFinalReplyDeliveryObserver {
    async fn observe_workflow_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck) {
        if let Err(error) = self.deliver_final_reply(envelope, ack).await {
            tracing::debug!(
                target = "ironclaw::reborn::slack_delivery",
                error = %error,
                "Slack final reply delivery skipped or failed after immediate ACK"
            );
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum SlackFinalReplyDeliveryError {
    #[error("workflow binding failed: {0}")]
    Workflow(#[from] ProductWorkflowError),
    #[error("turn coordinator failed: {0}")]
    Turn(#[from] ironclaw_turns::TurnError),
    #[error("thread service failed: {0}")]
    Thread(#[from] ironclaw_threads::SessionThreadError),
    #[error("outbound delivery failed: {0}")]
    Outbound(#[from] ironclaw_product_workflow::ProductOutboundDeliveryError),
    #[error("outbound policy failed: {0}")]
    OutboundPolicy(#[from] OutboundError),
    #[error("run {run_id} did not finish before Slack delivery timeout")]
    RunWaitTimedOut { run_id: TurnRunId },
    #[error("invalid projection ref: {reason}")]
    InvalidProjectionRef { reason: String },
}

struct ObservedSlackReplyTargetAuthority {
    scope: TurnScope,
    actor: TurnActor,
    expected_target: ReplyTargetBindingRef,
    external_conversation_ref: ExternalConversationRef,
    external_actor_ref: Option<ExternalActorRef>,
}

#[async_trait]
impl ReplyTargetBindingValidator for ObservedSlackReplyTargetAuthority {
    async fn validate_reply_target(
        &self,
        request: ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError> {
        if request.scope != self.scope
            || request.actor != self.actor
            || request.candidate.target != self.expected_target
        {
            return Err(OutboundError::AccessDenied);
        }
        Ok(ReplyTargetBindingClaim::new(request.candidate.target))
    }
}

#[async_trait]
impl ProductOutboundTargetResolver for ObservedSlackReplyTargetAuthority {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ValidatedReplyTargetBinding,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        if target.target() != &self.expected_target {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        Ok(VerifiedProductOutboundTargetMetadata {
            external_conversation_ref: self.external_conversation_ref.clone(),
            external_actor_ref: self.external_actor_ref.clone(),
        })
    }
}

struct AllowNoProjectionAccess;

#[async_trait]
impl ironclaw_outbound::ThreadProjectionAccessPolicy for AllowNoProjectionAccess {
    async fn authorize_projection_access(
        &self,
        _request: ironclaw_outbound::ThreadProjectionAccessRequest,
    ) -> Result<ironclaw_outbound::ThreadProjectionAccessClaim, OutboundError> {
        Err(OutboundError::AccessDenied)
    }
}

fn submitted_run_id(ack: &ProductInboundAck) -> Option<TurnRunId> {
    match ack {
        ProductInboundAck::Accepted {
            submitted_run_id, ..
        } => Some(*submitted_run_id),
        ProductInboundAck::Duplicate { prior } => submitted_run_id(prior),
        ProductInboundAck::DeferredBusy { .. }
        | ProductInboundAck::Rejected(_)
        | ProductInboundAck::CommandResult { .. }
        | ProductInboundAck::NoOp => None,
    }
}

fn resolve_binding_request(
    envelope: &ProductInboundEnvelope,
    route_kind: ProductConversationRouteKind,
) -> ResolveBindingRequest {
    ResolveBindingRequest {
        adapter_id: envelope.adapter_id().clone(),
        installation_id: envelope.installation_id().clone(),
        external_actor_ref: envelope.external_actor_ref().clone(),
        external_conversation_ref: envelope.external_conversation_ref().clone(),
        external_event_id: envelope.external_event_id().clone(),
        route_kind,
        auth_claim: envelope.auth_claim().clone(),
    }
}

fn route_kind_for_envelope(envelope: &ProductInboundEnvelope) -> ProductConversationRouteKind {
    match envelope.payload() {
        ProductInboundPayload::UserMessage(message) => match message.trigger {
            ProductTriggerReason::DirectChat => ProductConversationRouteKind::Direct,
            ProductTriggerReason::BotMention
            | ProductTriggerReason::ReplyToBot
            | ProductTriggerReason::BotCommand
            | ProductTriggerReason::LinkedThreadAction => ProductConversationRouteKind::Shared,
        },
        _ => ProductConversationRouteKind::Direct,
    }
}

fn turn_scope_from_binding(binding: &ResolvedBinding) -> Result<TurnScope, ProductWorkflowError> {
    Ok(TurnScope::new(
        binding.tenant_id.clone(),
        binding.agent_id.clone(),
        binding.project_id.clone(),
        binding.thread_id.clone(),
    ))
}

fn thread_scope_from_binding(
    binding: &ResolvedBinding,
    route_kind: ProductConversationRouteKind,
) -> Result<ThreadScope, ProductWorkflowError> {
    let Some(agent_id) = binding.agent_id.clone() else {
        return Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "resolved binding missing agent_id required for thread scope".to_string(),
        });
    };
    let owner_user_id = match route_kind {
        ProductConversationRouteKind::Direct => Some(binding.user_id.clone()),
        ProductConversationRouteKind::Shared => None,
    };
    Ok(ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id,
        project_id: binding.project_id.clone(),
        owner_user_id,
        mission_id: None,
    })
}

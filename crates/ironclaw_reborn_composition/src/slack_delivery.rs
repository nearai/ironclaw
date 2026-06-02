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
    CommunicationPreferenceKey, CommunicationPreferenceRecord, CommunicationPreferenceRepository,
    OutboundError, OutboundPolicyService, OutboundStateStore, ProjectionUpdateRef,
    ReplyTargetBindingClaim, ReplyTargetBindingValidator, ReplyTargetValidationRequest,
    RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin, SourceRouteContext,
    ValidatedReplyTargetBinding,
};
use ironclaw_product_adapters::{
    AuthPromptView, ExternalActorRef, ExternalConversationRef, FinalReplyView, GatePromptView,
    OutboundDeliverySink, ProductAdapter, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductOutboundPayload, ProductTriggerReason, ProtocolHttpEgress,
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
        let thread_scope = thread_scope_from_binding(&binding, route_kind)?;
        let mut actionable_state = self.wait_for_actionable(&scope, run_id).await?;
        loop {
            let (event_kind, payload, ignored_block) = match actionable_state.status {
                TurnStatus::Completed => {
                    let Some(text) = self
                        .read_latest_assistant_text(&thread_scope, &binding, run_id)
                        .await?
                    else {
                        return Ok(());
                    };
                    (
                        RunNotificationEventKind::FinalReplyReady,
                        ProductOutboundPayload::FinalReply(FinalReplyView {
                            turn_run_id: run_id,
                            text,
                            generated_at: Utc::now(),
                        }),
                        None,
                    )
                }
                TurnStatus::BlockedApproval => {
                    let Some(gate_ref) = actionable_state.gate_ref.as_ref() else {
                        return Ok(());
                    };
                    let gate_ref = gate_ref.as_str().to_string();
                    (
                        RunNotificationEventKind::ApprovalNeeded,
                        ProductOutboundPayload::GatePrompt(GatePromptView {
                            turn_run_id: run_id,
                            gate_ref: gate_ref.clone(),
                            headline: "Approval needed".to_string(),
                            body: "Reply in this Slack thread with `approve <gate_ref>` or `deny <gate_ref>`.".to_string(),
                        }),
                        Some(BlockedRunKey::Approval(gate_ref)),
                    )
                }
                TurnStatus::BlockedAuth => {
                    let Some(gate_ref) = actionable_state.gate_ref.as_ref() else {
                        return Ok(());
                    };
                    let auth_request_ref = gate_ref.as_str().to_string();
                    (
                        RunNotificationEventKind::AuthRequired,
                        ProductOutboundPayload::AuthPrompt(AuthPromptView {
                            turn_run_id: run_id,
                            auth_request_ref: auth_request_ref.clone(),
                            headline: "Authentication required".to_string(),
                            body: "Use WebUI setup to connect the missing account, or reply `auth deny <auth_request_ref>` to cancel this blocked run.".to_string(),
                            challenge_kind: None,
                            provider: None,
                            account_label: None,
                            authorization_url: None,
                            expires_at: None,
                        }),
                        Some(BlockedRunKey::Auth(auth_request_ref)),
                    )
                }
                _ => return Ok(()),
            };
            let reply_target = actionable_state.reply_target_binding_ref.clone();
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
            let projection_id = format!("slack-final-reply:{run_id}");
            let projection_ref = ProjectionUpdateRef::new(projection_id.clone())
                .map_err(|reason| SlackFinalReplyDeliveryError::InvalidProjectionRef { reason })?;
            let source_route_preferences = SourceRoutePromptPreferenceRepository::new(
                self.communication_preferences.as_ref(),
                CommunicationPreferenceKey::new(scope.tenant_id.clone(), actor.user_id.clone()),
                reply_target.clone(),
            );
            let delivery = ironclaw_outbound::PrepareCommunicationDeliveryRequest {
                resolution_request: CommunicationDeliveryResolutionRequest {
                    scope: scope.clone(),
                    actor: actor.clone(),
                    modality: CommunicationModality::Text,
                    intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                        event_kind,
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
            let _outcome = prepare_and_render_product_outbound(
                &outbound_policy,
                &source_route_preferences,
                &target_authority,
                ProductOutboundDeliveryRequest {
                    delivery,
                    payload,
                    projection_cursor: ironclaw_product_adapters::ProjectionCursor::new(
                        projection_id,
                    )
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

            let Some(ignored_block) = ignored_block else {
                return Ok(());
            };
            actionable_state = self
                .wait_for_actionable_after_block(&scope, run_id, ignored_block)
                .await?;
        }
    }

    async fn wait_for_actionable(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<ironclaw_turns::TurnRunState, SlackFinalReplyDeliveryError> {
        self.wait_for_actionable_matching(scope, run_id, |_| true)
            .await
    }

    async fn wait_for_actionable_after_block(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        ignored_block: BlockedRunKey,
    ) -> Result<ironclaw_turns::TurnRunState, SlackFinalReplyDeliveryError> {
        self.wait_for_actionable_matching(scope, run_id, |state| {
            blocked_run_key(state).as_ref() != Some(&ignored_block)
        })
        .await
    }

    async fn wait_for_actionable_matching(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        mut accept_state: impl FnMut(&ironclaw_turns::TurnRunState) -> bool,
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
            if (state.status.is_terminal()
                || matches!(
                    state.status,
                    TurnStatus::BlockedApproval | TurnStatus::BlockedAuth
                ))
                && accept_state(&state)
            {
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum BlockedRunKey {
    Approval(String),
    Auth(String),
}

fn blocked_run_key(state: &ironclaw_turns::TurnRunState) -> Option<BlockedRunKey> {
    match state.status {
        TurnStatus::BlockedApproval => state
            .gate_ref
            .as_ref()
            .map(|gate_ref| BlockedRunKey::Approval(gate_ref.as_str().to_string())),
        TurnStatus::BlockedAuth => state
            .gate_ref
            .as_ref()
            .map(|gate_ref| BlockedRunKey::Auth(gate_ref.as_str().to_string())),
        _ => None,
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

struct SourceRoutePromptPreferenceRepository<'a> {
    inner: &'a dyn CommunicationPreferenceRepository,
    key: CommunicationPreferenceKey,
    source_target: ReplyTargetBindingRef,
}

impl<'a> SourceRoutePromptPreferenceRepository<'a> {
    fn new(
        inner: &'a dyn CommunicationPreferenceRepository,
        key: CommunicationPreferenceKey,
        source_target: ReplyTargetBindingRef,
    ) -> Self {
        Self {
            inner,
            key,
            source_target,
        }
    }
}

#[async_trait]
impl CommunicationPreferenceRepository for SourceRoutePromptPreferenceRepository<'_> {
    async fn put_communication_preference(
        &self,
        record: CommunicationPreferenceRecord,
    ) -> Result<(), OutboundError> {
        self.inner.put_communication_preference(record).await
    }

    async fn load_communication_preference(
        &self,
        key: CommunicationPreferenceKey,
    ) -> Result<Option<CommunicationPreferenceRecord>, OutboundError> {
        let Some(mut record) = self
            .inner
            .load_communication_preference(key.clone())
            .await?
        else {
            if key != self.key {
                return Ok(None);
            }
            return Ok(Some(CommunicationPreferenceRecord {
                tenant_id: key.tenant_id.clone(),
                user_id: key.user_id.clone(),
                final_reply_target: None,
                progress_target: None,
                approval_prompt_target: Some(self.source_target.clone()),
                auth_prompt_target: Some(self.source_target.clone()),
                default_modality: Some(CommunicationModality::Text),
                updated_at: Utc::now(),
                updated_by: key.user_id.clone(),
            }));
        };
        if key == self.key {
            record.approval_prompt_target = Some(self.source_target.clone());
            record.auth_prompt_target = Some(self.source_target.clone());
        }
        Ok(Some(record))
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
        ProductInboundAck::Duplicate { .. } => None,
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
        ProductInboundPayload::UserMessage(message) => route_kind_for_trigger(message.trigger),
        ProductInboundPayload::Command(command) => route_kind_for_trigger(command.trigger),
        ProductInboundPayload::ApprovalResolution(resolution) => resolution
            .source_trigger
            .map(route_kind_for_trigger)
            .unwrap_or(ProductConversationRouteKind::Direct),
        ProductInboundPayload::AuthResolution(resolution) => resolution
            .source_trigger
            .map(route_kind_for_trigger)
            .unwrap_or(ProductConversationRouteKind::Direct),
        ProductInboundPayload::SubscriptionRequest(_)
        | ProductInboundPayload::LinkedThreadAction(_)
        | ProductInboundPayload::NoOp => ProductConversationRouteKind::Direct,
    }
}

fn route_kind_for_trigger(trigger: ProductTriggerReason) -> ProductConversationRouteKind {
    match trigger {
        ProductTriggerReason::DirectChat => ProductConversationRouteKind::Direct,
        ProductTriggerReason::BotMention
        | ProductTriggerReason::ReplyToBot
        | ProductTriggerReason::BotCommand
        | ProductTriggerReason::LinkedThreadAction => ProductConversationRouteKind::Shared,
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

//! Slack final-reply delivery for immediate-ACK Reborn webhooks.
//!
//! Slack Events API requires the HTTP handler to return 2xx quickly. This
//! observer runs after the workflow accepts an inbound Slack message, waits for
//! the submitted run to finish, reads the finalized assistant reply, and sends it
//! through the host-mediated product outbound delivery seam.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
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
    ExternalActorRef, ExternalConversationRef, FinalReplyView, GatePromptView,
    OutboundDeliverySink, ProductAdapter, ProductAdapterError, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductOutboundPayload, ProductTriggerReason,
    ProtocolHttpEgress,
};
use ironclaw_product_workflow::{
    ConversationBindingService, ProductOutboundDeliveryRequest, ProductOutboundTargetResolver,
    ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
    VerifiedProductOutboundTargetMetadata, prepare_and_render_product_outbound,
};
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, SessionThreadService, ThreadScope};
use ironclaw_turns::{
    GetRunStateRequest, ReplyTargetBindingRef, TurnActor, TurnCoordinator, TurnRunId, TurnScope,
    TurnStatus,
};
use ironclaw_wasm_product_adapters::ImmediateAckWorkflowObserver;
use tokio::sync::Semaphore;

use crate::AuthChallengeProvider;
use crate::auth_prompt::auth_prompt_view_for_blocked_auth;

const MAX_SLACK_RUN_POLL_INTERVAL: Duration = Duration::from_secs(5);
const SLACK_RUN_POLL_JITTER_BUCKETS: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlackFinalReplyDeliverySettings {
    pub poll_interval: Duration,
    pub max_wait: Duration,
    pub max_concurrent_deliveries: NonZeroUsize,
}

impl Default for SlackFinalReplyDeliverySettings {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(250),
            max_wait: Duration::from_secs(120),
            max_concurrent_deliveries: NonZeroUsize::new(64).expect("non-zero literal"), // safety: static default literal is non-zero.
        }
    }
}

pub struct SlackFinalReplyDeliveryServices {
    pub binding_service: Arc<dyn ConversationBindingService>,
    pub thread_service: Arc<dyn SessionThreadService>,
    pub turn_coordinator: Arc<dyn TurnCoordinator>,
    pub outbound_store: Arc<dyn OutboundStateStore>,
    pub communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    pub adapter: Arc<dyn ProductAdapter>,
    pub egress: Arc<dyn ProtocolHttpEgress>,
    pub delivery_sink: Arc<dyn OutboundDeliverySink>,
    pub auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
}

pub struct SlackFinalReplyDeliveryObserver {
    services: SlackFinalReplyDeliveryServices,
    settings: SlackFinalReplyDeliverySettings,
    delivery_permits: Arc<Semaphore>,
}

impl SlackFinalReplyDeliveryObserver {
    pub fn new(services: SlackFinalReplyDeliveryServices) -> Self {
        Self::with_settings(services, SlackFinalReplyDeliverySettings::default())
    }

    pub fn with_settings(
        services: SlackFinalReplyDeliveryServices,
        settings: SlackFinalReplyDeliverySettings,
    ) -> Self {
        Self {
            services,
            settings,
            delivery_permits: Arc::new(Semaphore::new(settings.max_concurrent_deliveries.get())),
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
        let binding = self
            .services
            .binding_service
            .lookup_binding(ResolveBindingRequest::from_envelope(&envelope))
            .await?;
        let actor = TurnActor::new(binding.actor_user_id.clone());
        let thread_scope = thread_scope_from_binding(&binding)?;
        let scope = turn_scope_from_thread_scope(&binding, &thread_scope)?;
        let actionable_state = self.wait_for_actionable(&scope, run_id).await?;
        let (event_kind, payload) = match actionable_state.status {
            TurnStatus::Completed => {
                let Some(text) = self
                    .read_latest_assistant_text(&thread_scope, &binding, run_id)
                    .await?
                else {
                    tracing::warn!(
                        %run_id,
                        "completed Slack run has no finalized assistant message; skipping final reply delivery"
                    );
                    return Ok(());
                };
                (
                    RunNotificationEventKind::FinalReplyReady,
                    ProductOutboundPayload::FinalReply(FinalReplyView {
                        turn_run_id: run_id,
                        text,
                        generated_at: Utc::now(),
                    }),
                )
            }
            TurnStatus::BlockedApproval => {
                let Some(gate_ref) = actionable_state.gate_ref.as_ref() else {
                    tracing::warn!(
                        %run_id,
                        "Slack run is blocked on approval without a gate ref; skipping approval prompt delivery"
                    );
                    return Ok(());
                };
                (
                    RunNotificationEventKind::ApprovalNeeded,
                    ProductOutboundPayload::GatePrompt(GatePromptView {
                        turn_run_id: run_id,
                        gate_ref: gate_ref.as_str().to_string(),
                        headline: "Approval needed".to_string(),
                        body: "A step in the workflow requires your approval to proceed."
                            .to_string(),
                    }),
                )
            }
            TurnStatus::BlockedAuth => {
                let Some(gate_ref) = actionable_state.gate_ref.as_ref() else {
                    tracing::warn!(
                        %run_id,
                        "Slack run is blocked on auth without a gate ref; skipping auth prompt delivery"
                    );
                    return Ok(());
                };
                let view = slack_auth_prompt_view(
                    &envelope,
                    auth_prompt_view_for_blocked_auth(
                        &binding.actor_user_id,
                        &scope,
                        run_id,
                        gate_ref.as_str(),
                        "Authenticate to continue this run.".to_string(),
                        &actionable_state.credential_requirements,
                        self.services.auth_challenges.as_deref(),
                    )
                    .await?,
                );
                (
                    RunNotificationEventKind::AuthRequired,
                    ProductOutboundPayload::AuthPrompt(view),
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
            self.services.outbound_store.as_ref(),
            &projection_access_policy,
            &target_authority,
        );
        let projection_id = format!("slack-final-reply:{run_id}");
        let projection_ref = ProjectionUpdateRef::new(projection_id.clone())
            .map_err(|reason| SlackFinalReplyDeliveryError::InvalidProjectionRef { reason })?;
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
            self.services.communication_preferences.as_ref(),
            &target_authority,
            ProductOutboundDeliveryRequest {
                delivery,
                payload,
                projection_cursor: ironclaw_product_adapters::ProjectionCursor::new(projection_id)
                    .map_err(|error| SlackFinalReplyDeliveryError::InvalidProjectionRef {
                        reason: error.to_string(),
                    })?,
                adapter: self.services.adapter.as_ref(),
                egress: self.services.egress.as_ref(),
                delivery_sink: self.services.delivery_sink.as_ref(),
            },
        )
        .await?;
        Ok(())
    }

    async fn wait_for_actionable(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<ironclaw_turns::TurnRunState, SlackFinalReplyDeliveryError> {
        let start = Instant::now();
        let mut poll_interval = self.settings.poll_interval;
        loop {
            let state = self
                .services
                .turn_coordinator
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await?;
            if state.status.is_terminal()
                || matches!(
                    state.status,
                    TurnStatus::BlockedApproval | TurnStatus::BlockedAuth
                )
            {
                return Ok(state);
            }
            if start.elapsed() >= self.settings.max_wait {
                return Err(SlackFinalReplyDeliveryError::RunWaitTimedOut { run_id });
            }
            tokio::time::sleep(jittered_poll_interval(poll_interval, &run_id)).await;
            poll_interval = poll_interval
                .saturating_mul(2)
                .min(MAX_SLACK_RUN_POLL_INTERVAL);
        }
    }

    async fn read_latest_assistant_text(
        &self,
        thread_scope: &ThreadScope,
        binding: &ResolvedBinding,
        run_id: TurnRunId,
    ) -> Result<Option<String>, SlackFinalReplyDeliveryError> {
        Ok(self
            .services
            .thread_service
            .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                scope: thread_scope.clone(),
                thread_id: binding.thread_id.clone(),
                turn_run_id: run_id.to_string(),
            })
            .await?
            .and_then(|message| message.content))
    }
}

fn slack_auth_prompt_view(
    envelope: &ProductInboundEnvelope,
    mut view: ironclaw_product_adapters::AuthPromptView,
) -> ironclaw_product_adapters::AuthPromptView {
    if !slack_auth_setup_link_is_private(envelope) {
        view.authorization_url = None;
    }
    view
}

fn slack_auth_setup_link_is_private(envelope: &ProductInboundEnvelope) -> bool {
    matches!(
        envelope.payload(),
        ProductInboundPayload::UserMessage(payload)
            if payload.trigger == ProductTriggerReason::DirectChat
    )
}

fn jittered_poll_interval(base: Duration, run_id: &TurnRunId) -> Duration {
    if base.is_zero() {
        return base;
    }
    let mut hasher = DefaultHasher::new();
    run_id.to_string().hash(&mut hasher);
    let bucket = hasher.finish() as u32 % SLACK_RUN_POLL_JITTER_BUCKETS;
    (base + base / SLACK_RUN_POLL_JITTER_BUCKETS * bucket).min(MAX_SLACK_RUN_POLL_INTERVAL)
}

#[async_trait]
impl ImmediateAckWorkflowObserver for SlackFinalReplyDeliveryObserver {
    async fn observe_workflow_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck) {
        let Ok(_permit) = self.delivery_permits.clone().acquire_owned().await else {
            tracing::warn!(
                target = "ironclaw::reborn::slack_delivery",
                "Slack final reply delivery skipped because delivery semaphore was closed"
            );
            return;
        };
        if let Err(error) = self.deliver_final_reply(envelope, ack).await {
            tracing::warn!(
                target = "ironclaw::reborn::slack_delivery",
                error = %error,
                "Slack final reply delivery failed after immediate ACK"
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
    #[error("adapter failed: {0}")]
    Adapter(#[from] ProductAdapterError),
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
        ProductInboundAck::Duplicate { .. } => None,
        ProductInboundAck::DeferredBusy { .. }
        | ProductInboundAck::Rejected(_)
        | ProductInboundAck::CommandResult { .. }
        | ProductInboundAck::NoOp => None,
    }
}

fn turn_scope_from_thread_scope(
    binding: &ResolvedBinding,
    thread_scope: &ThreadScope,
) -> Result<TurnScope, ProductWorkflowError> {
    let Some(agent_id) = binding.agent_id.clone() else {
        return Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "resolved binding missing agent_id required for turn scope".to_string(),
        });
    };
    Ok(TurnScope::new_with_owner(
        binding.tenant_id.clone(),
        Some(agent_id),
        binding.project_id.clone(),
        binding.thread_id.clone(),
        thread_scope.owner_user_id.clone(),
    ))
}

fn thread_scope_from_binding(
    binding: &ResolvedBinding,
) -> Result<ThreadScope, ProductWorkflowError> {
    let Some(agent_id) = binding.agent_id.clone() else {
        return Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "resolved binding missing agent_id required for thread scope".to_string(),
        });
    };
    Ok(ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id,
        project_id: binding.project_id.clone(),
        owner_user_id: binding.subject_user_id.clone(),
        mission_id: None,
    })
}

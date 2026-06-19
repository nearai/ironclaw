//! Telegram final-reply delivery for immediate-ACK Reborn webhooks.
//!
//! Telegram (like Slack) requires the webhook HTTP handler to ACK quickly. This
//! observer runs after the workflow accepts an inbound update, waits for the
//! submitted run to finish, reads the finalized assistant reply, and sends it
//! through the host-mediated product outbound seam (`prepare_and_render_product_
//! outbound`) using the Telegram adapter + host-mediated egress.
//!
//! Scope: this is the happy-path final-reply deliverer. Blocked-approval /
//! blocked-auth / busy-thread hinting (the elaborate Slack delivery surface) is
//! intentionally out of scope for the first Telegram slice; such runs are left
//! for the WebUI to surface and are skipped here rather than mis-delivered.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    InMemoryOutboundStateStore, OutboundError, OutboundPolicyService,
    PrepareCommunicationDeliveryRequest, ProjectionUpdateRef, ReplyTargetBindingClaim,
    ReplyTargetBindingValidator, ReplyTargetValidationRequest, RunNotificationContext,
    RunNotificationEventKind, RunNotificationOrigin, SourceRouteContext,
    ThreadProjectionAccessClaim, ThreadProjectionAccessPolicy, ThreadProjectionAccessRequest,
    ValidatedReplyTargetBinding,
};
use ironclaw_product_adapters::{
    ExternalActorRef, ExternalConversationRef, FinalReplyView, OutboundDeliverySink,
    ProductAdapter, ProductInboundAck, ProductInboundEnvelope, ProductOutboundPayload,
    ProjectionCursor, ProtocolHttpEgress,
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

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(200);
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_MAX_WAIT: Duration = Duration::from_secs(120);

/// Collaborators the Telegram delivery observer needs. `adapter` and `egress`
/// are the same instances the ingress runner uses (cloned `Arc`s); the
/// `binding_service` resolves the inbound conversation binding so the final
/// reply is routed back to the originating chat.
pub(crate) struct TelegramFinalReplyDeliveryServices {
    pub(crate) binding_service: Arc<dyn ConversationBindingService>,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) turn_coordinator: Arc<dyn TurnCoordinator>,
    pub(crate) adapter: Arc<dyn ProductAdapter>,
    pub(crate) egress: Arc<dyn ProtocolHttpEgress>,
    pub(crate) delivery_sink: Arc<dyn OutboundDeliverySink>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TelegramFinalReplyDeliverySettings {
    pub(crate) poll_interval: Duration,
    pub(crate) max_wait: Duration,
}

impl Default for TelegramFinalReplyDeliverySettings {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
            max_wait: DEFAULT_MAX_WAIT,
        }
    }
}

pub(crate) struct TelegramFinalReplyDeliveryObserver {
    services: TelegramFinalReplyDeliveryServices,
    settings: TelegramFinalReplyDeliverySettings,
}

impl TelegramFinalReplyDeliveryObserver {
    pub(crate) fn new(services: TelegramFinalReplyDeliveryServices) -> Self {
        Self::with_settings(services, TelegramFinalReplyDeliverySettings::default())
    }

    pub(crate) fn with_settings(
        services: TelegramFinalReplyDeliveryServices,
        settings: TelegramFinalReplyDeliverySettings,
    ) -> Self {
        Self { services, settings }
    }

    async fn deliver_final_reply(
        &self,
        envelope: ProductInboundEnvelope,
        ack: ProductInboundAck,
    ) -> Result<(), TelegramDeliveryError> {
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

        let state = self.wait_for_completion(&scope, run_id).await?;
        if state.status != TurnStatus::Completed {
            // Blocked / failed / cancelled runs are surfaced by the WebUI; the
            // first Telegram slice does not post blocked-state hints.
            tracing::debug!(
                target = "ironclaw::reborn::telegram_delivery",
                %run_id,
                status = ?state.status,
                "telegram run did not complete cleanly; skipping final-reply delivery"
            );
            return Ok(());
        }

        let Some(text) = self
            .read_latest_assistant_text(&thread_scope, &binding, run_id)
            .await?
        else {
            tracing::warn!(
                target = "ironclaw::reborn::telegram_delivery",
                %run_id,
                "completed telegram run has no finalized assistant message; skipping delivery"
            );
            return Ok(());
        };

        let reply_target = state.reply_target_binding_ref.clone();
        let authority = TelegramReplyTargetAuthority {
            scope: scope.clone(),
            actor: actor.clone(),
            expected_target: reply_target.clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            external_actor_ref: Some(envelope.external_actor_ref().clone()),
        };
        let projection_access_policy = DenyProjectionAccess;
        let store = InMemoryOutboundStateStore::default();
        let outbound_policy =
            OutboundPolicyService::new(&store, &projection_access_policy, &authority);

        let projection_id = format!("telegram-run-notification:final:{run_id}");
        let projection_ref = ProjectionUpdateRef::new(projection_id.clone())
            .map_err(|reason| TelegramDeliveryError::InvalidProjectionRef { reason })?;
        let delivery = PrepareCommunicationDeliveryRequest {
            resolution_request: CommunicationDeliveryResolutionRequest {
                scope: scope.clone(),
                actor: actor.clone(),
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
        let projection_cursor = ProjectionCursor::new(projection_id).map_err(|error| {
            TelegramDeliveryError::InvalidProjectionRef {
                reason: error.to_string(),
            }
        })?;

        prepare_and_render_product_outbound(
            &outbound_policy,
            &store,
            &authority,
            ProductOutboundDeliveryRequest {
                delivery,
                payload,
                projection_cursor,
                adapter: self.services.adapter.as_ref(),
                egress: self.services.egress.as_ref(),
                delivery_sink: self.services.delivery_sink.as_ref(),
                require_direct_message_target: false,
            },
        )
        .await
        .map_err(|error| TelegramDeliveryError::Delivery {
            reason: error.to_string(),
        })?;
        Ok(())
    }

    async fn wait_for_completion(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<ironclaw_turns::TurnRunState, TelegramDeliveryError> {
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
            if state.status.is_terminal() {
                return Ok(state);
            }
            if start.elapsed() >= self.settings.max_wait {
                return Err(TelegramDeliveryError::RunWaitTimedOut { run_id });
            }
            tokio::time::sleep(poll_interval).await;
            poll_interval = poll_interval.saturating_mul(2).min(MAX_POLL_INTERVAL);
        }
    }

    async fn read_latest_assistant_text(
        &self,
        thread_scope: &ThreadScope,
        binding: &ResolvedBinding,
        run_id: TurnRunId,
    ) -> Result<Option<String>, TelegramDeliveryError> {
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

#[async_trait]
impl ImmediateAckWorkflowObserver for TelegramFinalReplyDeliveryObserver {
    async fn observe_workflow_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck) {
        if let Err(error) = self.deliver_final_reply(envelope, ack).await {
            // Delivery is best-effort after the protocol ACK: the transport
            // cannot retry this event, so failures are logged, never panicked.
            tracing::debug!(
                target = "ironclaw::reborn::telegram_delivery",
                %error,
                "telegram final-reply delivery did not complete"
            );
        }
    }
}

/// Live-path reply-target authority: validates that the outbound target matches
/// the inbound-sealed target and projects the trusted conversation/actor refs.
/// Protocol-agnostic; the Slack equivalent does the same.
struct TelegramReplyTargetAuthority {
    scope: TurnScope,
    actor: TurnActor,
    expected_target: ReplyTargetBindingRef,
    external_conversation_ref: ExternalConversationRef,
    external_actor_ref: Option<ExternalActorRef>,
}

#[async_trait]
impl ReplyTargetBindingValidator for TelegramReplyTargetAuthority {
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
impl ProductOutboundTargetResolver for TelegramReplyTargetAuthority {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ValidatedReplyTargetBinding,
        _require_direct_message: bool,
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

/// Telegram delivery never reads thread projections, so projection access is
/// denied; the live path resolves targets purely from the sealed binding.
struct DenyProjectionAccess;

#[async_trait]
impl ThreadProjectionAccessPolicy for DenyProjectionAccess {
    async fn authorize_projection_access(
        &self,
        _request: ThreadProjectionAccessRequest,
    ) -> Result<ThreadProjectionAccessClaim, OutboundError> {
        Err(OutboundError::AccessDenied)
    }
}

#[derive(Debug, thiserror::Error)]
enum TelegramDeliveryError {
    #[error("telegram delivery binding/workflow error: {0}")]
    Workflow(#[from] ProductWorkflowError),
    #[error("telegram delivery turn error: {0}")]
    Turn(#[from] ironclaw_turns::TurnError),
    #[error("telegram delivery thread error: {0}")]
    Thread(#[from] ironclaw_threads::SessionThreadError),
    #[error("telegram run {run_id} did not finish before the delivery deadline")]
    RunWaitTimedOut { run_id: TurnRunId },
    #[error("invalid telegram projection ref: {reason}")]
    InvalidProjectionRef { reason: String },
    #[error("telegram outbound delivery failed: {reason}")]
    Delivery { reason: String },
}

fn submitted_run_id(ack: &ProductInboundAck) -> Option<TurnRunId> {
    match ack {
        ProductInboundAck::Accepted {
            submitted_run_id, ..
        } => Some(*submitted_run_id),
        _ => None,
    }
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

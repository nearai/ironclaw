//! The proactive half of run delivery: register a trigger-submitted run with
//! the lifecycle-event router and deliver its actionable outputs through the
//! [`DeliveryCoordinator`].

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::OutboundPart;
use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, ResourceScope, UserId};
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    OutboundDeliveryTargetId, OutboundError, OutboundPolicyService,
    PrepareCommunicationDeliveryRequest, ProjectionUpdateRef, ReplyTargetBindingClaim,
    ReplyTargetBindingValidator, ReplyTargetValidationRequest, RunFinalReplyDestination,
    RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin,
    TriggerCommunicationContext, TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryRecord,
    TriggeredRunDeliveryStore, ValidatedReplyTargetBinding,
};
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, ThreadScope};
use ironclaw_turns::{
    GetRunStateRequest, TurnActor, TurnEventKind, TurnEventSink, TurnLifecycleEvent,
    TurnOriginKind, TurnRunId, TurnRunState, TurnScope, TurnStatus,
};

use super::RunDeliveryEventRouter;
use super::lifecycle_events::AllowNoProjectionAccess;
use super::prompts;
use super::{
    BlockedAuthPromptRequest, DeliveredChannelMessage, RunDeliveryError, RunDeliveryServices,
    cancel_auth_blocked_run, delivered_messages_from_outcome,
    gate_routes::record_gate_route_if_needed,
};
use crate::delivery_coordinator::{
    CoordinatedDeliveryError, CoordinatedDeliveryOutcome, CoordinatedDeliveryRequest,
    DeliveryIntent,
};
use crate::{ProductOutboundTargetResolver, ProductWorkflowError};

// The codec contract lives in `ironclaw_product` (the vendor half
// is implemented by channel extension crates, which never depend on this
// crate); re-exported here so the triggered-delivery consumers keep one
// import surface.

/// Send-time authority boundary for a host-selected delivery target.
///
/// Both live per-run routing and scheduled delivery use this same port, so a
/// removed installation or revoked pairing invalidates either path uniformly.
/// Implementations must re-resolve caller ownership and current channel
/// readiness; decoding a stale opaque binding is not authorization. This is a
/// `dyn` seam because channel target providers are runtime-registered and this
/// boundary returns their current authority decision to product workflow.
#[async_trait]
pub trait CurrentDeliveryTargetResolver: Send + Sync {
    async fn resolve_current_target(
        &self,
        scope: &TurnScope,
        actor: &TurnActor,
        target: &ironclaw_turns::ReplyTargetBindingRef,
    ) -> Result<Option<CurrentDeliveryTarget>, ProductWorkflowError>;

    /// Resolve an opaque registry id through current caller authority.
    ///
    /// The returned destination is the canonical host-sealed routing value;
    /// implementations must not decode provider structure from `target_id`.
    async fn resolve_current_destination(
        &self,
        scope: &ResourceScope,
        target_id: &OutboundDeliveryTargetId,
    ) -> Result<Option<RunFinalReplyDestination>, ProductWorkflowError>;

    /// Resolve a currently-authorized binding back to its opaque registry id.
    async fn resolve_current_target_id(
        &self,
        scope: &ResourceScope,
        target: &ironclaw_turns::ReplyTargetBindingRef,
    ) -> Result<Option<OutboundDeliveryTargetId>, ProductWorkflowError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentDeliveryTarget {
    /// Extension/provider registration that owns this target. Delivery
    /// handlers must match it against their own extension id before sending.
    pub extension_id: String,
    pub external_conversation_ref: crate::ExternalConversationRef,
    pub personal_direct_message: bool,
}

/// Product-owned routing plan for a trigger's authoritative final-reply
/// destination.
///
/// The product-owned trigger router resolves the opaque registry id through
/// [`CurrentDeliveryTargetResolver`] and delegates the typed destination here.
/// This is the one place that defines WebApp as history-only: it must never be
/// collapsed into an absent external target, because absence means the
/// creator's communication preference should be consulted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggeredRunExternalDeliveryTarget {
    UseCommunicationPreference,
    Explicit {
        reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef,
    },
}

impl TriggeredRunExternalDeliveryTarget {
    /// Normalize the authoritative destination into the channel-egress lane.
    /// `None` means the result remains in host-owned WebApp history and the
    /// caller must not select or invoke a channel driver.
    pub fn from_destination(destination: Option<RunFinalReplyDestination>) -> Option<Self> {
        match destination {
            None => Some(Self::UseCommunicationPreference),
            Some(RunFinalReplyDestination::External {
                reply_target_binding_ref,
            }) => Some(Self::Explicit {
                reply_target_binding_ref,
            }),
            Some(RunFinalReplyDestination::WebApp) => None,
        }
    }
}

/// One trigger-submitted run to register for lifecycle-event delivery, in
/// generic vocabulary.
/// The composition's post-submit hook translates its trigger-fire type into
/// this.
#[derive(Debug, Clone)]
pub struct TriggeredRunDeliveryRequest {
    pub run_id: TurnRunId,
    pub scope: TurnScope,
    /// The trigger creator; delivery goes to their personal preference
    /// target.
    pub creator_user_id: UserId,
    /// Fail closed for non-personal triggers: a project-scoped trigger is
    /// never delivered to a personal channel.
    pub project_scoped: bool,
    /// The trigger prompt; its first line becomes the short footer label.
    pub prompt: String,
    /// Optional per-trigger target resolved from the creator-scoped outbound
    /// target registry. When present, ordinary results route here instead of
    /// consulting the user's mutable global default.
    pub delivery_target: Option<ironclaw_turns::ReplyTargetBindingRef>,
    pub trigger_context: TriggerCommunicationContext,
}

/// Notification content for one actionable triggered-run state.
struct TriggeredNotification {
    event_kind: RunNotificationEventKind,
    intent: DeliveryIntent,
    part: OutboundPart,
    gate_ref_for_routing: Option<String>,
    /// AuthPrompt payloads carrying an OAuth URL must only land in a
    /// personal DM; enforced by the resolver at send time.
    require_direct_message_target: bool,
}

/// Stable run and routing inputs shared by each notification attempt for one
/// triggered run.
struct TriggeredNotificationContext<'a> {
    scope: &'a TurnScope,
    actor: &'a TurnActor,
    run_id: TurnRunId,
    trigger_context: &'a TriggerCommunicationContext,
    delivery_target: Option<&'a ironclaw_turns::ReplyTargetBindingRef>,
    authority: &'a TriggeredReplyTargetAuthority<'a>,
    event_cursor: u64,
}

/// Typed failure classification for a single triggered-run notification
/// delivery attempt.
enum TriggeredNotificationFailure {
    /// The creator has no personal delivery target configured.
    NoDefaultConfigured,
    /// The resolved target is inaccessible or rejected the delivery.
    Denied,
    /// The payload carries an OAuth `authorization_url` but the send-time
    /// binding resolved to a non-personal-DM target. Handled by cancelling
    /// the run and posting the auth-unavailable notice.
    OAuthTargetNotDm,
    /// Any other delivery or transport failure.
    Other(String),
}

impl std::fmt::Display for TriggeredNotificationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDefaultConfigured => write!(f, "no default delivery target configured"),
            Self::Denied => write!(f, "delivery target access denied"),
            Self::OAuthTargetNotDm => write!(
                f,
                "OAuth authorization_url suppressed: send-time target is not a personal DM"
            ),
            Self::Other(reason) => write!(f, "{reason}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TriggeredDeliveryStage {
    Approval(String),
    Auth(String),
    Final,
}

#[derive(Default)]
struct TriggeredEventLedger {
    active: HashSet<TriggeredDeliveryStage>,
    delivered: HashSet<TriggeredDeliveryStage>,
    cleanup: Vec<PendingTriggeredCleanup>,
}

struct PendingTriggeredCleanup {
    message: DeliveredChannelMessage,
    attempt_ordinal: u32,
}

/// Event-driven owner for one settled trigger fire. It retains routing
/// context only while the durable run is live; each lifecycle event fetches
/// canonical state and independently attempts the corresponding delivery.
pub(crate) struct TriggeredRunDeliveryEventHandler {
    services: RunDeliveryServices,
    request: TriggeredRunDeliveryRequest,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    current_target_resolver: Arc<dyn CurrentDeliveryTargetResolver>,
    fallback_agent_id: AgentId,
    ledger: Mutex<TriggeredEventLedger>,
}

impl TriggeredRunDeliveryEventHandler {
    fn new(
        services: RunDeliveryServices,
        request: TriggeredRunDeliveryRequest,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        current_target_resolver: Arc<dyn CurrentDeliveryTargetResolver>,
        fallback_agent_id: AgentId,
    ) -> Self {
        Self {
            services,
            request,
            delivery_store,
            current_target_resolver,
            fallback_agent_id,
            ledger: Mutex::new(TriggeredEventLedger::default()),
        }
    }

    pub(crate) async fn handle_event(
        &self,
        event: &TurnLifecycleEvent,
    ) -> Result<bool, RunDeliveryError> {
        match self.handle_event_inner(event).await {
            Err(error)
                if matches!(
                    event.kind,
                    TurnEventKind::Completed | TurnEventKind::Failed | TurnEventKind::Cancelled
                ) =>
            {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    run_id = %event.run_id,
                    %error,
                    "triggered terminal delivery failed; recording a terminal failed outcome"
                );
                record_triggered_run_outcome_strict(
                    self.delivery_store.as_ref(),
                    event.run_id,
                    TriggeredRunDeliveryOutcomeKind::Failed,
                )
                .await?;
                Ok(self.retract_cleanup(&event.scope, event.run_id).await)
            }
            result => result,
        }
    }

    async fn handle_event_inner(
        &self,
        event: &TurnLifecycleEvent,
    ) -> Result<bool, RunDeliveryError> {
        if event.run_id != self.request.run_id || event.scope != self.request.scope {
            return Ok(false);
        }
        let state = self
            .services
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: event.scope.clone(),
                run_id: event.run_id,
            })
            .await?;
        if state
            .product_context
            .as_ref()
            .is_some_and(|context| context.origin != TurnOriginKind::ScheduledTrigger)
        {
            return Ok(false);
        }
        if matches!(state.status, TurnStatus::Failed | TurnStatus::Cancelled) {
            let cleanup_settled = self.retract_cleanup(&state.scope, state.run_id).await;
            record_triggered_run_outcome(
                self.delivery_store.as_ref(),
                state.run_id,
                TriggeredRunDeliveryOutcomeKind::Failed,
            )
            .await;
            return Ok(cleanup_settled);
        }
        if !matches!(
            state.status,
            TurnStatus::BlockedApproval | TurnStatus::BlockedAuth | TurnStatus::Completed
        ) {
            return Ok(false);
        }

        let stage = match state.status {
            TurnStatus::BlockedApproval => TriggeredDeliveryStage::Approval(
                state
                    .gate_ref
                    .as_ref()
                    .map(|gate| gate.as_str().to_string())
                    .unwrap_or_default(),
            ),
            TurnStatus::BlockedAuth => TriggeredDeliveryStage::Auth(
                state
                    .gate_ref
                    .as_ref()
                    .map(|gate| gate.as_str().to_string())
                    .unwrap_or_default(),
            ),
            TurnStatus::Completed => TriggeredDeliveryStage::Final,
            _ => return Ok(false),
        };
        if stage == TriggeredDeliveryStage::Final && self.stage_was_delivered(&stage) {
            return Ok(self.retract_cleanup(&state.scope, state.run_id).await);
        }
        if !self.claim(&stage) {
            return Ok(false);
        }

        let actor = TurnActor::new(self.request.creator_user_id.clone());
        let thread_scope = ThreadScope {
            tenant_id: state.scope.tenant_id.clone(),
            agent_id: state
                .scope
                .agent_id
                .clone()
                .unwrap_or_else(|| self.fallback_agent_id.clone()),
            project_id: state.scope.project_id.clone(),
            owner_user_id: state.scope.explicit_owner_user_id().cloned(),
            mission_id: None,
        };
        let trigger_label = prompts::triggered_label_from_prompt(&self.request.prompt);
        let notification = match triggered_notification_for_state(
            &self.services,
            &state.scope,
            &thread_scope,
            &actor,
            &state,
            state.run_id,
            &trigger_label,
        )
        .await
        {
            Ok(Some(notification)) => notification,
            Ok(None) => {
                self.finish_claim(stage, false);
                if state.status == TurnStatus::Completed {
                    record_triggered_run_outcome(
                        self.delivery_store.as_ref(),
                        state.run_id,
                        TriggeredRunDeliveryOutcomeKind::Skipped,
                    )
                    .await;
                    return Ok(self.retract_cleanup(&state.scope, state.run_id).await);
                }
                return Ok(false);
            }
            Err(error) => {
                self.finish_claim(stage, false);
                return Err(error);
            }
        };
        let notification_kind = notification.event_kind;
        let gate_ref = notification.gate_ref_for_routing.clone();
        let authority = TriggeredReplyTargetAuthority {
            scope: state.scope.clone(),
            actor: actor.clone(),
            resolver: self.current_target_resolver.as_ref(),
        };
        let context = TriggeredNotificationContext {
            scope: &state.scope,
            actor: &actor,
            run_id: state.run_id,
            trigger_context: &self.request.trigger_context,
            delivery_target: self.request.delivery_target.as_ref(),
            authority: &authority,
            event_cursor: state.event_cursor.0,
        };

        match deliver_triggered_notification(&self.services, &context, notification).await {
            Ok(delivered) => {
                if let Some(gate_ref) = gate_ref.as_deref() {
                    record_gate_route_if_needed(
                        self.services.route_store.as_ref(),
                        state.run_id,
                        &state.scope.tenant_id,
                        &actor.user_id,
                        gate_ref,
                        &state.scope,
                        &delivered,
                        None,
                    )
                    .await;
                }
                self.finish_claim(stage, true);
                if matches!(
                    notification_kind,
                    RunNotificationEventKind::ApprovalNeeded
                        | RunNotificationEventKind::AuthRequired
                ) {
                    self.replace_cleanup(&state.scope, state.run_id, delivered)
                        .await;
                    record_triggered_run_outcome(
                        self.delivery_store.as_ref(),
                        state.run_id,
                        TriggeredRunDeliveryOutcomeKind::Delivered,
                    )
                    .await;
                    return Ok(false);
                }
                let cleanup_settled = self.retract_cleanup(&state.scope, state.run_id).await;
                record_triggered_run_outcome(
                    self.delivery_store.as_ref(),
                    state.run_id,
                    TriggeredRunDeliveryOutcomeKind::Delivered,
                )
                .await;
                Ok(cleanup_settled)
            }
            Err(TriggeredNotificationFailure::OAuthTargetNotDm) => {
                let (outcome, cleanup_settled) = self
                    .cancel_and_deliver_auth_unavailable(&state, &actor, &context, &trigger_label)
                    .await;
                self.finish_claim(stage, true);
                record_triggered_run_outcome(self.delivery_store.as_ref(), state.run_id, outcome)
                    .await;
                Ok(cleanup_settled)
            }
            Err(failure) => {
                self.finish_claim(stage, true);
                let outcome = triggered_failure_outcome(&failure);
                record_triggered_run_outcome(self.delivery_store.as_ref(), state.run_id, outcome)
                    .await;
                Ok(self.retract_cleanup(&state.scope, state.run_id).await)
            }
        }
    }

    fn claim(&self, stage: &TriggeredDeliveryStage) -> bool {
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !ledger.delivered.contains(stage) && ledger.active.insert(stage.clone())
    }

    fn finish_claim(&self, stage: TriggeredDeliveryStage, delivered: bool) {
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ledger.active.remove(&stage);
        if delivered {
            ledger.delivered.insert(stage);
        }
    }

    fn stage_was_delivered(&self, stage: &TriggeredDeliveryStage) -> bool {
        self.ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .delivered
            .contains(stage)
    }

    async fn replace_cleanup(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        delivered: Vec<DeliveredChannelMessage>,
    ) {
        let previous = {
            let mut ledger = self
                .ledger
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::replace(
                &mut ledger.cleanup,
                delivered
                    .into_iter()
                    .map(|message| PendingTriggeredCleanup {
                        message,
                        attempt_ordinal: 0,
                    })
                    .collect(),
            )
        };
        let mut retry = Vec::new();
        for message in previous {
            if let Some(message) = self.retract_if_current(scope, run_id, message).await {
                retry.push(message);
            }
        }
        if !retry.is_empty() {
            self.ledger
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .cleanup
                .extend(retry);
        }
    }

    async fn retract_cleanup(&self, scope: &TurnScope, run_id: TurnRunId) -> bool {
        let cleanup = {
            let mut ledger = self
                .ledger
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut ledger.cleanup)
        };
        let mut retry = Vec::new();
        for message in cleanup {
            if let Some(message) = self.retract_if_current(scope, run_id, message).await {
                retry.push(message);
            }
        }
        let settled = retry.is_empty();
        if !settled {
            self.ledger
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .cleanup
                .extend(retry);
        }
        settled
    }

    async fn retract_if_current(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        pending: PendingTriggeredCleanup,
    ) -> Option<PendingTriggeredCleanup> {
        let message = &pending.message;
        let actor = TurnActor::new(self.request.creator_user_id.clone());
        let target = match self
            .current_target_resolver
            .resolve_current_target(scope, &actor, &message.reply_target_binding_ref)
            .await
        {
            Ok(Some(target)) => target,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    run_id = %run_id,
                    %error,
                    "triggered cleanup target resolution failed; retaining cleanup responsibility"
                );
                return Some(pending);
            }
        };
        if target.external_conversation_ref != message.conversation {
            return None;
        }
        match self
            .services
            .retract_message_outcome(
                scope.clone(),
                Some(run_id),
                pending.message.clone(),
                pending.attempt_ordinal,
            )
            .await
        {
            Ok(CoordinatedDeliveryOutcome::Delivered { .. }) => None,
            Ok(
                CoordinatedDeliveryOutcome::NoDelivery
                | CoordinatedDeliveryOutcome::Rejected { .. }
                | CoordinatedDeliveryOutcome::DuplicateSuppressed { .. }
                | CoordinatedDeliveryOutcome::Failed { .. },
            ) => Some(PendingTriggeredCleanup {
                message: pending.message,
                attempt_ordinal: pending.attempt_ordinal.saturating_add(1),
            }),
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    run_id = %run_id,
                    %error,
                    "triggered cleanup delivery failed; retaining cleanup responsibility"
                );
                Some(PendingTriggeredCleanup {
                    message: pending.message,
                    attempt_ordinal: pending.attempt_ordinal.saturating_add(1),
                })
            }
        }
    }

    async fn cancel_and_deliver_auth_unavailable(
        &self,
        state: &TurnRunState,
        actor: &TurnActor,
        context: &TriggeredNotificationContext<'_>,
        trigger_label: &str,
    ) -> (TriggeredRunDeliveryOutcomeKind, bool) {
        if cancel_auth_blocked_run(
            self.services.turn_coordinator.as_ref(),
            self.services.auth_flow_cancel.as_deref(),
            &state.scope,
            actor.clone(),
            state.run_id,
            state.gate_ref.as_ref().map(|gate| gate.as_str()),
        )
        .await
        .is_err()
        {
            let cleanup_settled = self.retract_cleanup(&state.scope, state.run_id).await;
            return (TriggeredRunDeliveryOutcomeKind::Failed, cleanup_settled);
        }
        let notice = TriggeredNotification {
            event_kind: RunNotificationEventKind::FinalReplyReady,
            intent: DeliveryIntent::FinalReply,
            part: OutboundPart::Text(format!(
                "{}{}",
                prompts::AUTH_UNAVAILABLE_MESSAGE,
                prompts::triggered_update_footer(trigger_label)
            )),
            gate_ref_for_routing: None,
            require_direct_message_target: false,
        };
        let outcome = match deliver_triggered_notification(&self.services, context, notice).await {
            Ok(_) => TriggeredRunDeliveryOutcomeKind::Delivered,
            Err(failure) => triggered_failure_outcome(&failure),
        };
        let cleanup_settled = self.retract_cleanup(&state.scope, state.run_id).await;
        (outcome, cleanup_settled)
    }
}

fn triggered_failure_outcome(
    failure: &TriggeredNotificationFailure,
) -> TriggeredRunDeliveryOutcomeKind {
    match failure {
        TriggeredNotificationFailure::NoDefaultConfigured => {
            TriggeredRunDeliveryOutcomeKind::NoDefaultConfigured
        }
        TriggeredNotificationFailure::Denied => TriggeredRunDeliveryOutcomeKind::Denied,
        TriggeredNotificationFailure::OAuthTargetNotDm | TriggeredNotificationFailure::Other(_) => {
            TriggeredRunDeliveryOutcomeKind::Failed
        }
    }
}

/// Registers trigger fires with the shared lifecycle-event delivery router.
pub struct TriggeredRunDeliveryDriver {
    services: RunDeliveryServices,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    current_target_resolver: Arc<dyn CurrentDeliveryTargetResolver>,
    /// Fallback agent id used when the submitted TurnScope has no agent.
    fallback_agent_id: AgentId,
    event_router: Arc<RunDeliveryEventRouter>,
}

impl TriggeredRunDeliveryDriver {
    /// Build the event-driven driver. The shared router is the only
    /// long-lived lifecycle owner; no task, timeout, or concurrency permit
    /// is held while a run waits on auth or approval.
    pub fn with_event_router(
        services: RunDeliveryServices,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        current_target_resolver: Arc<dyn CurrentDeliveryTargetResolver>,
        fallback_agent_id: AgentId,
        event_router: Arc<RunDeliveryEventRouter>,
    ) -> Self {
        Self {
            services,
            delivery_store,
            current_target_resolver,
            fallback_agent_id,
            event_router,
        }
    }

    /// The preference repository this driver resolves targets from.
    /// Production wiring must hand the SAME store the WebUI writes, so
    /// user-set preferences are visible here; tests assert pointer equality.
    pub fn communication_preferences(
        &self,
    ) -> Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository> {
        Arc::clone(&self.services.communication_preferences)
    }

    /// Register one submitted trigger and reconcile its already-durable state.
    pub async fn on_trigger_submitted(&self, request: TriggeredRunDeliveryRequest) {
        if request.project_scoped {
            record_triggered_run_outcome(
                self.delivery_store.as_ref(),
                request.run_id,
                TriggeredRunDeliveryOutcomeKind::Denied,
            )
            .await;
            return;
        }

        let event_router = Arc::clone(&self.event_router);
        let run_id = request.run_id;
        let scope = request.scope.clone();
        let handler = Arc::new(TriggeredRunDeliveryEventHandler::new(
            self.services.clone(),
            request,
            Arc::clone(&self.delivery_store),
            Arc::clone(&self.current_target_resolver),
            self.fallback_agent_id.clone(),
        ));
        event_router.register_triggered(run_id, handler);
        let state = match self
            .services
            .turn_coordinator
            .get_run_state(GetRunStateRequest { scope, run_id })
            .await
        {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    %run_id,
                    %error,
                    "triggered run delivery could not reconcile submitted state"
                );
                event_router.remove_triggered(run_id);
                record_triggered_run_outcome(
                    self.delivery_store.as_ref(),
                    run_id,
                    TriggeredRunDeliveryOutcomeKind::Failed,
                )
                .await;
                return;
            }
        };
        let event_kind = match state.status {
            TurnStatus::BlockedApproval | TurnStatus::BlockedAuth => TurnEventKind::Blocked,
            TurnStatus::Completed => TurnEventKind::Completed,
            TurnStatus::Failed => TurnEventKind::Failed,
            TurnStatus::Cancelled => TurnEventKind::Cancelled,
            _ => TurnEventKind::Submitted,
        };
        let event = TurnLifecycleEvent::from_run_state(&state, event_kind, None);
        if let Err(error) = event_router.publish(event).await {
            tracing::warn!(
                target = "ironclaw::reborn::run_delivery",
                %run_id,
                %error,
                "triggered run initial lifecycle reconciliation failed"
            );
        }
    }
}

/// Builds the notification for a triggered run's actionable state.
///
/// ## Triggered channel surface contract
///
/// A triggered run is **output-only over the channel, plus gate-resolution
/// input** — it is NOT a conversational surface. Only three outputs are
/// minted here:
///
/// - `BlockedApproval` → gate prompt (approve/deny)
/// - `BlockedAuth`     → auth prompt (OAuth link) or, for non-OAuth, a
///   cancel + final-reply carrying the auth-unavailable notice
/// - `Completed`       → final reply
///
/// Anything else yields `None` — triggered delivery deliberately does not
/// stream progress; that belongs to the live WebUI surface. Preserve that
/// boundary when extending this function.
async fn triggered_notification_for_state(
    services: &RunDeliveryServices,
    scope: &TurnScope,
    thread_scope: &ThreadScope,
    actor: &TurnActor,
    state: &TurnRunState,
    run_id: TurnRunId,
    trigger_label: &str,
) -> Result<Option<TriggeredNotification>, RunDeliveryError> {
    match state.status {
        TurnStatus::Completed => {
            let Some(text) = services
                .thread_service
                .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                    scope: thread_scope.clone(),
                    thread_id: scope.thread_id.clone(),
                    turn_run_id: run_id.to_string(),
                })
                .await?
                .and_then(|message| message.content)
            else {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    %run_id,
                    "completed triggered run has no finalized assistant message; skipping delivery"
                );
                return Ok(None);
            };
            Ok(Some(TriggeredNotification {
                event_kind: RunNotificationEventKind::FinalReplyReady,
                intent: DeliveryIntent::TriggeredDelivery,
                part: OutboundPart::Text(format!(
                    "{text}{}",
                    prompts::triggered_update_footer(trigger_label)
                )),
                gate_ref_for_routing: None,
                require_direct_message_target: false,
            }))
        }
        TurnStatus::BlockedApproval => {
            let Some(gate_ref) = state.gate_ref.as_ref() else {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    %run_id,
                    "triggered run blocked on approval without gate ref; skipping"
                );
                return Ok(None);
            };
            let context = match &services.approval_context {
                Some(source) => {
                    source
                        .approval_prompt_context(gate_ref, &actor.user_id, scope)
                        .await
                }
                None => None,
            };
            let mut view = prompts::approval_gate_prompt_view(run_id, gate_ref, context.as_ref());
            view.body
                .push_str(&prompts::triggered_gate_footer(trigger_label));
            Ok(Some(TriggeredNotification {
                event_kind: RunNotificationEventKind::ApprovalNeeded,
                intent: DeliveryIntent::GatePrompt,
                // Preference targets are personal DMs or picked shared
                // channels; the DM reply instruction applies to the personal
                // target this delivery resolves to.
                part: OutboundPart::Text(prompts::gate_prompt_text(&view, true)),
                gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
                require_direct_message_target: false,
            }))
        }
        TurnStatus::BlockedAuth => {
            let Some(gate_ref) = state.gate_ref.as_ref() else {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    %run_id,
                    "triggered run blocked on auth without gate ref; skipping"
                );
                return Ok(None);
            };
            let view = match &services.blocked_auth_prompts {
                Some(source) => Some(
                    source
                        .auth_prompt_for_blocked_run(BlockedAuthPromptRequest {
                            fallback_owner_user_id: &actor.user_id,
                            scope,
                            run_id,
                            gate_ref: gate_ref.as_str(),
                            invocation_id: None,
                            body: "Authentication required to continue this automation."
                                .to_string(),
                            credential_requirements: &state.credential_requirements,
                        })
                        .await?,
                ),
                None => None,
            };
            let unavailable_message = prompts::unserviceable_auth_prompt_message(view.as_ref());
            match view.filter(prompts::auth_prompt_is_serviceable) {
                Some(mut view) => {
                    view.body = prompts::actionable_auth_prompt_body(&view);
                    view.body
                        .push_str(&prompts::triggered_gate_footer(trigger_label));
                    let require_direct_message_target =
                        view.authorization_url.is_some() || view.pairing.is_some();
                    // The DM requirement is enforced by the resolver at send
                    // time (closing the snapshot-vs-send race); no pre-check
                    // here.
                    Ok(Some(TriggeredNotification {
                        event_kind: RunNotificationEventKind::AuthRequired,
                        intent: DeliveryIntent::AuthPrompt,
                        part: OutboundPart::AuthPrompt {
                            view: Box::new(view),
                            direct_message: true,
                        },
                        gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
                        require_direct_message_target,
                    }))
                }
                _ => {
                    // Missing/retired challenge metadata, secret entry, and a
                    // pairing kind without materialized host challenge data
                    // are not actionable in a channel. Cancel the parked run
                    // and deliver one terminal-safe WebUI notice.
                    cancel_auth_blocked_run(
                        services.turn_coordinator.as_ref(),
                        services.auth_flow_cancel.as_deref(),
                        scope,
                        actor.clone(),
                        run_id,
                        Some(gate_ref.as_str()),
                    )
                    .await?;
                    Ok(Some(TriggeredNotification {
                        event_kind: RunNotificationEventKind::FinalReplyReady,
                        intent: DeliveryIntent::TriggeredDelivery,
                        part: OutboundPart::Text(format!(
                            "{}{}",
                            unavailable_message,
                            prompts::triggered_update_footer(trigger_label)
                        )),
                        gate_ref_for_routing: None,
                        require_direct_message_target: false,
                    }))
                }
            }
        }
        _ => Ok(None),
    }
}

/// Deliver one triggered-run notification through the coordinator,
/// returning the delivered channel messages.
async fn deliver_triggered_notification(
    services: &RunDeliveryServices,
    context: &TriggeredNotificationContext<'_>,
    notification: TriggeredNotification,
) -> Result<Vec<DeliveredChannelMessage>, TriggeredNotificationFailure> {
    let projection_access_policy = AllowNoProjectionAccess;
    let outbound_policy = OutboundPolicyService::new(
        services.outbound_store.as_ref(),
        &projection_access_policy,
        context.authority,
    );
    let projection_id = format!(
        "{}:{}",
        prompts::run_notification_projection_id(context.run_id, notification.event_kind),
        context.event_cursor,
    );
    let projection_ref = ProjectionUpdateRef::new(projection_id).map_err(|reason| {
        TriggeredNotificationFailure::Other(format!("invalid_projection_ref: {reason}"))
    })?;
    let delivery = PrepareCommunicationDeliveryRequest {
        resolution_request: CommunicationDeliveryResolutionRequest {
            scope: context.scope.clone(),
            actor: context.actor.clone(),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind: notification.event_kind,
                origin: match context.delivery_target {
                    Some(target) => RunNotificationOrigin::TriggeredWithTarget {
                        trigger: context.trigger_context.clone(),
                        target: target.clone(),
                    },
                    None => RunNotificationOrigin::Triggered {
                        trigger: context.trigger_context.clone(),
                    },
                },
            }),
        },
        turn_run_id: Some(context.run_id),
        projection_ref,
        attempted_at: Utc::now(),
    };

    let outcome = services
        .coordinator
        .deliver(
            &outbound_policy,
            services.communication_preferences.as_ref(),
            context.authority,
            CoordinatedDeliveryRequest {
                intent: notification.intent,
                delivery,
                parts: vec![notification.part],
                thread_anchor: None,
                require_direct_message_target: notification.require_direct_message_target,
                extension_id: &services.extension_id,
            },
        )
        .await
        .map_err(classify_delivery_error)?;
    match outcome {
        CoordinatedDeliveryOutcome::NoDelivery => {
            Err(TriggeredNotificationFailure::NoDefaultConfigured)
        }
        CoordinatedDeliveryOutcome::Rejected { .. } => Err(TriggeredNotificationFailure::Denied),
        CoordinatedDeliveryOutcome::Failed { failure_kind, .. } => Err(
            TriggeredNotificationFailure::Other(format!("delivery failed: {failure_kind:?}")),
        ),
        outcome => Ok(delivered_messages_from_outcome(&outcome)),
    }
}

/// Classify a [`CoordinatedDeliveryError`] into the typed failure variants
/// used for outcome recording.
fn classify_delivery_error(error: CoordinatedDeliveryError) -> TriggeredNotificationFailure {
    match &error {
        CoordinatedDeliveryError::Workflow(
            ProductWorkflowError::OutboundTargetNotDirectMessage,
        ) => TriggeredNotificationFailure::OAuthTargetNotDm,
        CoordinatedDeliveryError::Outbound(OutboundError::PreferenceTargetMissing { .. }) => {
            TriggeredNotificationFailure::NoDefaultConfigured
        }
        CoordinatedDeliveryError::Outbound(OutboundError::AccessDenied) => {
            TriggeredNotificationFailure::Denied
        }
        _ => TriggeredNotificationFailure::Other(error.to_string()),
    }
}

async fn record_triggered_run_outcome(
    store: &dyn TriggeredRunDeliveryStore,
    run_id: TurnRunId,
    outcome: TriggeredRunDeliveryOutcomeKind,
) {
    if let Err(error) = record_triggered_run_outcome_strict(store, run_id, outcome).await {
        tracing::warn!(
            target = "ironclaw::reborn::run_delivery",
            %run_id,
            error = %error,
            "failed to record triggered run delivery outcome (best-effort)"
        );
    }
}

async fn record_triggered_run_outcome_strict(
    store: &dyn TriggeredRunDeliveryStore,
    run_id: TurnRunId,
    outcome: TriggeredRunDeliveryOutcomeKind,
) -> Result<(), RunDeliveryError> {
    let record = TriggeredRunDeliveryRecord {
        run_id,
        outcome,
        recorded_at: Utc::now(),
    };
    store
        .record_triggered_run_delivery(record)
        .await
        .map_err(|reason| ProductWorkflowError::Transient { reason }.into())
}

/// Reply-target authority for triggered-run delivery: trusts the target the
/// resolution engine chose from the creator's personal preference. The
/// current-target resolver rechecks ownership and channel readiness both at
/// policy validation and immediately before adapter resolution.
struct TriggeredReplyTargetAuthority<'a> {
    scope: TurnScope,
    actor: TurnActor,
    resolver: &'a dyn CurrentDeliveryTargetResolver,
}

#[async_trait]
impl ReplyTargetBindingValidator for TriggeredReplyTargetAuthority<'_> {
    async fn validate_reply_target(
        &self,
        request: ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError> {
        if request.scope != self.scope || request.actor != self.actor {
            return Err(OutboundError::AccessDenied);
        }
        let resolved = self
            .resolver
            .resolve_current_target(&self.scope, &self.actor, &request.candidate.target)
            .await
            .map_err(|error| match error {
                ProductWorkflowError::Transient { .. } => OutboundError::Backend,
                _ => OutboundError::AccessDenied,
            })?;
        if resolved.is_none() {
            return Err(OutboundError::AccessDenied);
        }
        Ok(ReplyTargetBindingClaim::new(request.candidate.target))
    }
}

#[async_trait]
impl ProductOutboundTargetResolver for TriggeredReplyTargetAuthority<'_> {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ValidatedReplyTargetBinding,
        require_direct_message: bool,
    ) -> Result<crate::VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        let resolved = self
            .resolver
            .resolve_current_target(&self.scope, &self.actor, target.target())
            .await?
            .ok_or(ProductWorkflowError::BindingAccessDenied)?;
        if require_direct_message && !resolved.personal_direct_message {
            return Err(ProductWorkflowError::OutboundTargetNotDirectMessage);
        }
        Ok(crate::VerifiedProductOutboundTargetMetadata {
            external_conversation_ref: resolved.external_conversation_ref,
            external_actor_ref: None,
        })
    }
}

//! The live source-route half of run delivery: watch the run an inbound
//! channel message submitted and deliver its outputs back to the
//! originating conversation, entirely through the [`DeliveryCoordinator`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    OutboundError, OutboundPolicyService, PrepareCommunicationDeliveryRequest, ProjectionUpdateRef,
    ReplyTargetBindingClaim, ReplyTargetBindingValidator, ReplyTargetValidationRequest,
    RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin, SourceRouteContext,
    ThreadProjectionAccessClaim, ThreadProjectionAccessPolicy, ThreadProjectionAccessRequest,
};
use ironclaw_product_adapters::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, OutboundPart, ProductAdapterError,
    ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload, ProductRejection,
    ProductRejectionKind, ProductTriggerReason, ProductWorkflowRejectionKind,
};
use ironclaw_threads::FinalizedAssistantMessageByRunRequest;
use ironclaw_turns::{
    GetRunStateRequest, ReplyTargetBindingRef, TurnActor, TurnErrorCategory, TurnRunId,
    TurnRunState, TurnScope, TurnStatus,
};
use tokio::sync::Semaphore;

use super::prompts;
use super::{
    BlockedActionableMarker, BlockedAuthPromptRequest, DeliveredChannelMessage, HINT_SEEN_CAP,
    HintSeenSet, RunDeliveryError, RunDeliveryServices, RunDeliverySettings,
    blocked_actionable_marker, cancel_auth_blocked_run, delivered_messages_from_outcome,
    gate_routes::record_gate_route_if_needed, thread_scope_from_binding,
    turn_scope_from_thread_scope,
};
use crate::delivery_coordinator::{
    CoordinatedDeliveryOutcome, CoordinatedDeliveryRequest, DeliveryIntent,
};
use crate::{
    ChannelConnectionNoticePolicy, ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
};

const CONNECT_NOTICE_THROTTLE_WINDOW: std::time::Duration = std::time::Duration::from_secs(30);

/// One actionable run state reduced to a semantic notification: the intent,
/// its channel-neutral text, and the routing bookkeeping it needs.
struct ActionableNotification {
    event_kind: RunNotificationEventKind,
    intent: DeliveryIntent,
    text: String,
    /// Gate ref recorded into the delivered-gate route store for approval
    /// and auth prompts, so a bare reply next to the prompt resolves the
    /// gate. `None` for other kinds.
    gate_ref_for_routing: Option<String>,
}

/// Bound on the delivered-run memory. Evicted oldest-first; an evicted entry
/// only weakens dedup for a run whose final reply was delivered more than
/// `DELIVERED_RUNS_CAP` deliveries ago — duplicate acks arrive within the
/// same inbound exchange, not that far apart.
const DELIVERED_RUNS_CAP: usize = 1024;
const CONNECT_NUDGE_RESERVATION_CAP: usize = 1024;

/// Single-mutex ledger behind the per-run delivery dedup. `active` is the
/// single-flight set (at most one live delivery loop per run); `delivered` is
/// a bounded memory of runs whose FINAL reply was already posted, so a
/// gate-resolution ack that lands after the original loop delivered and
/// exited does not start a second loop and re-post the final reply. One
/// mutex, one atomic skip/proceed decision — two separate locks would
/// reintroduce the sequential-redelivery TOCTOU this exists to close.
#[derive(Default)]
struct DeliveryRunLedger {
    active: HashSet<TurnRunId>,
    delivered: HashSet<TurnRunId>,
    delivered_order: std::collections::VecDeque<TurnRunId>,
}

impl DeliveryRunLedger {
    /// Atomically decide whether a new delivery loop may start for `run_id`.
    fn try_claim(&mut self, run_id: TurnRunId) -> DeliveryClaim {
        if self.delivered.contains(&run_id) {
            return DeliveryClaim::AlreadyDelivered;
        }
        if !self.active.insert(run_id) {
            return DeliveryClaim::AlreadyActive;
        }
        DeliveryClaim::Claimed
    }

    fn record_delivered(&mut self, run_id: TurnRunId) {
        self.active.remove(&run_id);
        if self.delivered.insert(run_id) {
            self.delivered_order.push_back(run_id);
            while self.delivered_order.len() > DELIVERED_RUNS_CAP {
                if let Some(evicted) = self.delivered_order.pop_front() {
                    self.delivered.remove(&evicted);
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum DeliveryClaim {
    Claimed,
    AlreadyActive,
    AlreadyDelivered,
}

/// RAII guard that removes a `run_id` from the ledger's active set on drop
/// (failure/cancel path — success is recorded through
/// [`DeliveryRunLedger::record_delivered`], after which this drop is a
/// no-op). Acquired before the delivery semaphore permit so a concurrent ack
/// for the same run is rejected immediately, without a TOCTOU window on the
/// permit. Panic-safe: `Drop` tolerates a poisoned mutex.
struct RunDeliveryGuard<'a> {
    ledger: &'a Mutex<DeliveryRunLedger>,
    run_id: TurnRunId,
}

impl Drop for RunDeliveryGuard<'_> {
    fn drop(&mut self) {
        self.ledger
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .remove(&self.run_id);
    }
}

/// Generic immediate-ACK run watcher: observes the ack the workflow
/// produced for an inbound channel message, polls the submitted run, and
/// emits every user-visible output as a [`DeliveryIntent`] through the
/// coordinator. Channel-agnostic by construction — the vendor is reachable
/// only through the coordinator's resolved adapter.
pub struct RunDeliveryObserver {
    services: RunDeliveryServices,
    settings: RunDeliverySettings,
    connection_notices: ChannelConnectionNoticePolicy,
    delivery_permits: Arc<Semaphore>,
    /// Per-observer, per-conversation connect-nudge reservations. Reserving
    /// before delivery prevents concurrent unbound events from racing.
    connect_nudge_reservations: Mutex<HashMap<String, Instant>>,
    /// Per-observer throttle: at most one busy-thread hint per
    /// (conversation fingerprint, external_event_id) pair.
    hint_seen: HintSeenSet,
    /// Per-run delivery dedup: at most one live delivery loop per run id
    /// (single-flight), plus a bounded memory of runs whose final reply was
    /// already delivered. A gate-resolution ack carries the same submitted
    /// run id as the original user-message ack (it resumes the run); whether
    /// it lands while the original loop is still polling or just after that
    /// loop delivered and exited, it must not re-post the final reply.
    delivery_runs: Mutex<DeliveryRunLedger>,
}

impl RunDeliveryObserver {
    pub fn new(services: RunDeliveryServices) -> Self {
        Self::with_settings_and_connection_notices(
            services,
            RunDeliverySettings::default(),
            ChannelConnectionNoticePolicy::generic("this channel"),
        )
    }

    pub fn with_settings(services: RunDeliveryServices, settings: RunDeliverySettings) -> Self {
        Self::with_settings_and_connection_notices(
            services,
            settings,
            ChannelConnectionNoticePolicy::generic("this channel"),
        )
    }

    pub fn with_settings_and_connection_notices(
        services: RunDeliveryServices,
        settings: RunDeliverySettings,
        connection_notices: ChannelConnectionNoticePolicy,
    ) -> Self {
        Self {
            services,
            settings,
            connection_notices,
            delivery_permits: Arc::new(Semaphore::new(settings.max_concurrent_deliveries.get())),
            connect_nudge_reservations: Mutex::new(HashMap::new()),
            hint_seen: Mutex::new((std::collections::VecDeque::new(), HashSet::new())),
            delivery_runs: Mutex::new(DeliveryRunLedger::default()),
        }
    }

    pub async fn post_connection_status_notice(
        &self,
        conversation: &ExternalConversationRef,
        event_id: &ExternalEventId,
        text: &str,
    ) {
        self.services
            .post_notice(
                DeliveryIntent::ConnectionStatus,
                self.services.fallback_notice_scope.clone(),
                None,
                conversation,
                text,
                format!("connection-status:{}", event_id.as_str()),
            )
            .await;
    }

    /// Observe one workflow ack for an inbound envelope. This is the
    /// entry point the composition's post-admission observer seam calls.
    pub async fn observe_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck) {
        self.close_connect_nudge_epoch_after_accepted_user_message(&envelope, &ack);
        // Rejected approval/auth feedback is a single best-effort post, not
        // a long-running delivery — handle before taking the semaphore.
        if self
            .post_rejection_hint_if_authorized(&envelope, &ack)
            .await
        {
            return;
        }
        if self
            .post_connect_nudge_if_unbound_user_message(&envelope, &ack)
            .await
        {
            return;
        }
        // Busy-thread hint: the user's message was dropped because a run is
        // busy. Post a one-shot state-aware hint (approve/deny/wait) rather
        // than leaving the user in silence.
        if let Some(active_run_id) = busy_hint_user_message_run_id(&envelope, &ack) {
            self.post_busy_hint(&envelope, active_run_id).await;
            return;
        }
        let _delivery_guard = if let Some(run_id) = submitted_run_id(&ack) {
            let claim = self
                .delivery_runs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .try_claim(run_id);
            match claim {
                DeliveryClaim::AlreadyActive => {
                    tracing::debug!(
                        target = "ironclaw::reborn::run_delivery",
                        %run_id,
                        "skipping redundant delivery loop: a loop is already watching this run"
                    );
                    return;
                }
                DeliveryClaim::AlreadyDelivered => {
                    tracing::debug!(
                        target = "ironclaw::reborn::run_delivery",
                        %run_id,
                        "skipping redundant delivery loop: this run's final reply was already delivered"
                    );
                    return;
                }
                DeliveryClaim::Claimed => {}
            }
            Some(RunDeliveryGuard {
                ledger: &self.delivery_runs,
                run_id,
            })
        } else {
            None
        };
        let Ok(_permit) = self.delivery_permits.clone().acquire_owned().await else {
            tracing::warn!(
                target = "ironclaw::reborn::run_delivery",
                "final reply delivery skipped because the delivery semaphore was closed"
            );
            return;
        };
        let delivery_result = self.deliver_final_reply(envelope.clone(), ack).await;
        drop(_delivery_guard);
        if let Err(error) = delivery_result {
            tracing::warn!(
                target = "ironclaw::reborn::run_delivery",
                error = %error,
                "final reply delivery failed after immediate ACK"
            );
            // Best-effort feedback so the user is not left in silence. Skip
            // if a blocked-state notification was already delivered.
            let feedback = match &error {
                RunDeliveryError::RunWaitTimedOut { .. } => Some(prompts::DELIVERY_TIMEOUT_MESSAGE),
                RunDeliveryError::RunWaitTimedOutAfterNotification { .. } => None,
                _ => Some(prompts::DELIVERY_ERROR_MESSAGE),
            };
            if let Some(feedback) = feedback {
                let scope = self.notice_scope(&envelope).await;
                self.services
                    .post_notice(
                        DeliveryIntent::FailureNotice,
                        scope,
                        submitted_run_id_for_feedback(&error),
                        envelope.external_conversation_ref(),
                        feedback,
                        format!(
                            "delivery-feedback:{}",
                            envelope.external_event_id().as_str()
                        ),
                    )
                    .await;
            }
        }
    }

    /// Observe a workflow error for an inbound envelope (the error-path
    /// mirror of `observe_ack`).
    pub async fn observe_error(
        &self,
        envelope: ProductInboundEnvelope,
        error: ProductAdapterError,
    ) {
        let Some(ack) = rejection_ack_for_workflow_error(&error) else {
            return;
        };
        // An unbound user's first inbound resolves as a `BindingRequired`
        // workflow *error* — the rejection-hint path is a guaranteed no-op
        // for them (binding lookup fails by definition), so without the
        // connect nudge an unbound 1:1 DM gets total silence.
        if self
            .post_rejection_hint_if_authorized(&envelope, &ack)
            .await
        {
            return;
        }
        self.post_connect_nudge_if_unbound_user_message(&envelope, &ack)
            .await;
    }

    async fn deliver_final_reply(
        &self,
        envelope: ProductInboundEnvelope,
        ack: ProductInboundAck,
    ) -> Result<(), RunDeliveryError> {
        if is_accepted_auth_denial(&envelope, &ack) {
            let scope = self.notice_scope(&envelope).await;
            self.services
                .post_notice(
                    DeliveryIntent::FailureNotice,
                    scope,
                    submitted_run_id(&ack),
                    envelope.external_conversation_ref(),
                    prompts::AUTH_CANCELED_MESSAGE,
                    format!("auth-canceled:{}", envelope.external_event_id().as_str()),
                )
                .await;
            return Ok(());
        }
        if !should_deliver_after_ack(&envelope, &ack) {
            return Ok(());
        }
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
        // Foreign-run guard: a resolution bridged to a triggered run (the
        // delivered-gate-route rewrite) resumes a run that lives in the
        // trigger's own scope, not this conversation's scope. That run is
        // delivered by its own triggered-delivery loop, so the live observer
        // must not also poll it here — the run isn't found in this scope,
        // which would otherwise surface as a spurious "something went wrong"
        // error. The skip applies ONLY to bridged gate/auth resolution
        // payloads; a normal user message must never be silently dropped.
        let payload_can_bridge_to_foreign_run = matches!(
            envelope.payload(),
            ProductInboundPayload::ApprovalResolution(_)
                | ProductInboundPayload::ScopedApprovalResolution(_)
                | ProductInboundPayload::AuthResolution(_)
        );
        match self
            .services
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
        {
            Ok(_) => {}
            Err(error)
                if payload_can_bridge_to_foreign_run
                    && matches!(error.category(), TurnErrorCategory::ScopeNotFound) =>
            {
                tracing::debug!(
                    target = "ironclaw::reborn::run_delivery",
                    %run_id,
                    "skipping live delivery: run is not in this conversation scope (triggered/foreign run); its own delivery loop owns continuation"
                );
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        }
        let mut delivered_blocked_marker: Option<BlockedActionableMarker> = None;
        let mut working_message: Option<DeliveredChannelMessage> = None;
        let mut messages_to_delete_after_final: Vec<DeliveredChannelMessage> = Vec::new();
        loop {
            let actionable_state = {
                self.wait_for_actionable(
                    &envelope,
                    &scope,
                    run_id,
                    delivered_blocked_marker.as_ref(),
                    &mut working_message,
                )
                .await
                .map_err(|err| {
                    // If a blocked-state notification was already delivered,
                    // a timeout does not leave the user in silence — convert
                    // to the quieter variant so feedback does not double-post.
                    if matches!(err, RunDeliveryError::RunWaitTimedOut { .. })
                        && delivered_blocked_marker.is_some()
                    {
                        RunDeliveryError::RunWaitTimedOutAfterNotification { run_id }
                    } else {
                        err
                    }
                })?
            };
            if matches!(
                actionable_state.status,
                TurnStatus::BlockedApproval | TurnStatus::BlockedAuth
            ) && let Some(message) = working_message.take()
            {
                self.services
                    .retract_message(scope.clone(), Some(run_id), message)
                    .await;
            }
            let Some(notification) = self
                .notification_for_actionable_state(
                    &envelope,
                    &binding,
                    &thread_scope,
                    &scope,
                    run_id,
                    &actionable_state,
                )
                .await?
            else {
                return Ok(());
            };
            let next_blocked_marker = blocked_actionable_marker(&actionable_state);
            let event_kind = notification.event_kind;
            let gate_ref_for_routing = notification.gate_ref_for_routing.clone();
            let delivered_messages = self
                .deliver_run_notification(
                    &envelope,
                    &scope,
                    &actor,
                    run_id,
                    &actionable_state,
                    notification,
                )
                .await?;
            if (event_kind == RunNotificationEventKind::ApprovalNeeded
                || event_kind == RunNotificationEventKind::AuthRequired)
                && let Some(gate_ref_str) = gate_ref_for_routing.as_deref()
            {
                record_gate_route_if_needed(
                    self.services.route_store.as_ref(),
                    run_id,
                    &scope.tenant_id,
                    &binding.actor_user_id,
                    gate_ref_str,
                    &scope,
                    &delivered_messages,
                    Some(envelope.external_conversation_ref()),
                )
                .await;
            }

            let Some(marker) = next_blocked_marker else {
                // Terminal notification delivered — record it so a late
                // duplicate ack for this run (a gate-resolution ack racing
                // this loop's exit) skips instead of re-posting the final
                // reply.
                self.delivery_runs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_delivered(run_id);
                if let Some(message) = working_message.take() {
                    self.services
                        .retract_message(scope.clone(), Some(run_id), message)
                        .await;
                }
                for message in messages_to_delete_after_final {
                    self.services
                        .retract_message(scope.clone(), Some(run_id), message)
                        .await;
                }
                return Ok(());
            };
            if event_kind == RunNotificationEventKind::AuthRequired {
                messages_to_delete_after_final.extend(delivered_messages);
            }
            delivered_blocked_marker = Some(marker);
        }
    }

    /// The live-path poll loop: waits for a terminal or newly-blocked state,
    /// raising the working indicator once while the run is quietly running.
    /// (Mirror of `wait_for_actionable_state`; kept separate so the indicator
    /// side effect stays on the live path only.)
    async fn wait_for_actionable(
        &self,
        envelope: &ProductInboundEnvelope,
        scope: &TurnScope,
        run_id: TurnRunId,
        delivered_blocked_marker: Option<&BlockedActionableMarker>,
        working_message: &mut Option<DeliveredChannelMessage>,
    ) -> Result<TurnRunState, RunDeliveryError> {
        let start = std::time::Instant::now();
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
            if let Some(marker) = blocked_actionable_marker(&state)
                && Some(&marker) != delivered_blocked_marker
            {
                return Ok(state);
            }
            if start.elapsed() >= self.settings.max_wait {
                return Err(RunDeliveryError::RunWaitTimedOut { run_id });
            }
            if working_message.is_none() && blocked_actionable_marker(&state).is_none() {
                *working_message = self
                    .services
                    .post_notice(
                        DeliveryIntent::Working,
                        scope.clone(),
                        Some(run_id),
                        envelope.external_conversation_ref(),
                        prompts::WORKING_MESSAGE,
                        format!("working:{run_id}"),
                    )
                    .await;
            }
            tokio::time::sleep(super::jittered_poll_interval(poll_interval, &run_id)).await;
            poll_interval = poll_interval
                .saturating_mul(2)
                .min(std::time::Duration::from_secs(5));
        }
    }

    async fn notification_for_actionable_state(
        &self,
        envelope: &ProductInboundEnvelope,
        binding: &ResolvedBinding,
        thread_scope: &ironclaw_threads::ThreadScope,
        scope: &TurnScope,
        run_id: TurnRunId,
        state: &TurnRunState,
    ) -> Result<Option<ActionableNotification>, RunDeliveryError> {
        let direct_message = envelope_is_direct_chat(envelope);
        let notification = match state.status {
            TurnStatus::Completed => {
                let Some(text) = self
                    .read_latest_assistant_text(thread_scope, binding, run_id)
                    .await?
                else {
                    tracing::warn!(
                        %run_id,
                        "completed run has no finalized assistant message; skipping final reply delivery"
                    );
                    return Ok(None);
                };
                ActionableNotification {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    intent: DeliveryIntent::FinalReply,
                    text,
                    gate_ref_for_routing: None,
                }
            }
            TurnStatus::BlockedApproval => {
                let Some(gate_ref) = state.gate_ref.as_ref() else {
                    tracing::warn!(
                        %run_id,
                        "run is blocked on approval without a gate ref; skipping approval prompt delivery"
                    );
                    return Ok(None);
                };
                let approval_context = match &self.services.approval_context {
                    Some(source) => {
                        source
                            .approval_prompt_context(gate_ref, &binding.actor_user_id, scope)
                            .await
                    }
                    None => None,
                };
                let view =
                    prompts::approval_gate_prompt_view(run_id, gate_ref, approval_context.as_ref());
                ActionableNotification {
                    event_kind: RunNotificationEventKind::ApprovalNeeded,
                    intent: DeliveryIntent::GatePrompt,
                    text: prompts::gate_prompt_text(&view, direct_message),
                    gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
                }
            }
            TurnStatus::BlockedAuth => {
                let Some(gate_ref) = state.gate_ref.as_ref() else {
                    tracing::warn!(
                        %run_id,
                        "run is blocked on auth without a gate ref; skipping auth handling"
                    );
                    return Ok(None);
                };
                let view = match &self.services.blocked_auth_prompts {
                    Some(source) => Some(
                        source
                            .auth_prompt_for_blocked_run(BlockedAuthPromptRequest {
                                fallback_owner_user_id: &binding.actor_user_id,
                                scope,
                                run_id,
                                gate_ref: gate_ref.as_str(),
                                invocation_id: None,
                                body: "Authenticate to continue this run.".to_string(),
                                credential_requirements: &state.credential_requirements,
                            })
                            .await?,
                    ),
                    None => None,
                };
                // Only link-based OAuth is allowed over chat surfaces: the
                // user authenticates on the provider's site and the callback
                // stores the credential server-side. Any other challenge
                // would have the user paste a credential into chat, so deny
                // it: cancel the run (same outcome as `auth deny`) and
                // redirect to the web app.
                match view {
                    Some(view) if view.authorization_url.is_some() => {
                        let mut view = view;
                        // OAuth setup links are only safe in a private DM;
                        // strip the URL for any other origin.
                        if !auth_setup_link_is_private(envelope) {
                            view.authorization_url = None;
                        }
                        ActionableNotification {
                            event_kind: RunNotificationEventKind::AuthRequired,
                            intent: DeliveryIntent::AuthPrompt,
                            text: prompts::auth_prompt_text(&view, direct_message),
                            gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
                        }
                    }
                    _ => {
                        cancel_auth_blocked_run(
                            self.services.turn_coordinator.as_ref(),
                            self.services.auth_flow_cancel.as_deref(),
                            scope,
                            TurnActor::new(binding.actor_user_id.clone()),
                            run_id,
                            Some(gate_ref.as_str()),
                        )
                        .await?;
                        self.services
                            .post_notice(
                                DeliveryIntent::FailureNotice,
                                scope.clone(),
                                Some(run_id),
                                envelope.external_conversation_ref(),
                                prompts::AUTH_UNAVAILABLE_MESSAGE,
                                format!("auth-unavailable:{run_id}"),
                            )
                            .await;
                        return Ok(None);
                    }
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(notification))
    }

    async fn deliver_run_notification(
        &self,
        envelope: &ProductInboundEnvelope,
        scope: &TurnScope,
        actor: &TurnActor,
        run_id: TurnRunId,
        state: &TurnRunState,
        notification: ActionableNotification,
    ) -> Result<Vec<DeliveredChannelMessage>, RunDeliveryError> {
        let reply_target = state.reply_target_binding_ref.clone();
        let target_authority = ObservedReplyTargetAuthority {
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
        let projection_id =
            prompts::run_notification_projection_id(run_id, notification.event_kind);
        let projection_ref = ProjectionUpdateRef::new(projection_id)
            .map_err(|reason| RunDeliveryError::InvalidProjectionRef { reason })?;
        let delivery = PrepareCommunicationDeliveryRequest {
            resolution_request: CommunicationDeliveryResolutionRequest {
                scope: scope.clone(),
                actor: actor.clone(),
                modality: CommunicationModality::Text,
                intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                    event_kind: notification.event_kind,
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
        let outcome = self
            .services
            .coordinator
            .deliver(
                &outbound_policy,
                self.services.communication_preferences.as_ref(),
                &target_authority,
                CoordinatedDeliveryRequest {
                    intent: notification.intent,
                    delivery,
                    parts: vec![OutboundPart::Text(notification.text)],
                    thread_anchor: None,
                    require_direct_message_target: false,
                    extension_id: &self.services.extension_id,
                },
            )
            .await?;
        match outcome {
            CoordinatedDeliveryOutcome::Failed { failure_kind, .. } => {
                Err(RunDeliveryError::DeliveryFailed { failure_kind })
            }
            outcome => Ok(delivered_messages_from_outcome(&outcome)),
        }
    }

    async fn read_latest_assistant_text(
        &self,
        thread_scope: &ironclaw_threads::ThreadScope,
        binding: &ResolvedBinding,
        run_id: TurnRunId,
    ) -> Result<Option<String>, RunDeliveryError> {
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

    /// Scope for notices raised outside a resolved delivery loop: the
    /// conversation's binding scope when it resolves, else the host's
    /// fallback notice scope (never silent, always attributed).
    async fn notice_scope(&self, envelope: &ProductInboundEnvelope) -> TurnScope {
        match self
            .services
            .binding_service
            .lookup_binding(ResolveBindingRequest::from_envelope(envelope))
            .await
        {
            Ok(binding) => thread_scope_from_binding(&binding)
                .and_then(|thread_scope| turn_scope_from_thread_scope(&binding, &thread_scope))
                .unwrap_or_else(|_| self.services.fallback_notice_scope.clone()),
            Err(_) => self.services.fallback_notice_scope.clone(),
        }
    }

    async fn post_rejection_hint_if_authorized(
        &self,
        envelope: &ProductInboundEnvelope,
        ack: &ProductInboundAck,
    ) -> bool {
        let Some(hint) = rejection_hint_for_resolution(envelope, ack) else {
            return false;
        };
        let binding = match self
            .services
            .binding_service
            .lookup_binding(ResolveBindingRequest::from_envelope(envelope))
            .await
        {
            Ok(binding) => binding,
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::reborn::run_delivery",
                    error = %error,
                    "skipped rejection hint because the originating conversation was not authorized"
                );
                return true;
            }
        };
        let scope = thread_scope_from_binding(&binding)
            .and_then(|thread_scope| turn_scope_from_thread_scope(&binding, &thread_scope))
            .unwrap_or_else(|_| self.services.fallback_notice_scope.clone());
        self.services
            .post_notice(
                DeliveryIntent::FailureNotice,
                scope,
                None,
                envelope.external_conversation_ref(),
                hint,
                format!("rejection-hint:{}", envelope.external_event_id().as_str()),
            )
            .await;
        true
    }

    /// A first-contact DM from a user with no identity binding is rejected
    /// with `BindingRequired`. Instead of silently dropping it, greet them
    /// with a connect nudge — but ONLY in a 1:1 direct chat: the nudge is
    /// addressed to one person and must never land in a shared conversation.
    /// The gate is the adapter's own trigger classification
    /// (`ProductTriggerReason::DirectChat` — the same signal the OAuth-link
    /// privacy rule trusts). Deliberately performs NO binding lookup (the
    /// sender is unbound by definition) and posts only fixed, host-authored
    /// text. Transport retries arrive as `Duplicate`, so this fires at most
    /// once per inbound event.
    async fn post_connect_nudge_if_unbound_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
        ack: &ProductInboundAck,
    ) -> bool {
        let ProductInboundAck::Rejected(rejection) = ack else {
            return false;
        };
        if !matches!(rejection.kind, ProductRejectionKind::BindingRequired) {
            return false;
        }
        if !envelope_is_direct_chat(envelope) {
            return false;
        }
        let conversation_key = envelope
            .external_conversation_ref()
            .conversation_fingerprint();
        let Some(reserved_at) = self.reserve_connect_nudge(conversation_key.clone()) else {
            return true;
        };
        let delivered = self
            .services
            .post_notice(
                DeliveryIntent::ConnectRequired,
                self.services.fallback_notice_scope.clone(),
                None,
                envelope.external_conversation_ref(),
                &self.connection_notices.connect_required,
                format!("connect-nudge:{}", envelope.external_event_id().as_str()),
            )
            .await;
        if delivered.is_none() {
            self.release_connect_nudge(&conversation_key, reserved_at);
        }
        true
    }

    fn reserve_connect_nudge(&self, conversation_key: String) -> Option<Instant> {
        let now = Instant::now();
        let mut reservations = self
            .connect_nudge_reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reservations.retain(|_, reserved_at| {
            now.duration_since(*reserved_at) < CONNECT_NOTICE_THROTTLE_WINDOW
        });
        if reservations.contains_key(&conversation_key) {
            return None;
        }
        if reservations.len() >= CONNECT_NUDGE_RESERVATION_CAP {
            // Fail closed at saturation. Entries can represent deliveries
            // currently awaiting vendor evidence, so evicting an unexpired
            // reservation would reopen that conversation to a concurrent
            // duplicate nudge.
            return None;
        }
        reservations.insert(conversation_key, now);
        Some(now)
    }

    fn release_connect_nudge(&self, conversation_key: &str, reserved_at: Instant) {
        let mut reservations = self
            .connect_nudge_reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if reservations.get(conversation_key) == Some(&reserved_at) {
            reservations.remove(conversation_key);
        }
    }

    fn close_connect_nudge_epoch_after_accepted_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
        ack: &ProductInboundAck,
    ) {
        if !matches!(envelope.payload(), ProductInboundPayload::UserMessage(_))
            || !matches!(ack, ProductInboundAck::Accepted { .. })
        {
            return;
        }
        let conversation_key = envelope
            .external_conversation_ref()
            .conversation_fingerprint();
        self.connect_nudge_reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&conversation_key);
    }

    async fn post_busy_hint(&self, envelope: &ProductInboundEnvelope, active_run_id: TurnRunId) {
        // Throttle: at most one hint per (conversation, external_event_id)
        // pair. Transport retries carry the same event id → deduplicated;
        // each new human message gets a fresh hint. Checked before any
        // round-trips.
        let conv_key = envelope
            .external_conversation_ref()
            .conversation_fingerprint();
        let throttle_key = (conv_key, envelope.external_event_id().clone());
        let already_seen = {
            let mut guard = self.hint_seen.lock().unwrap_or_else(|e| e.into_inner());
            let (queue, set) = &mut *guard;
            if set.contains(&throttle_key) {
                true
            } else {
                if set.len() >= HINT_SEEN_CAP
                    && let Some(oldest) = queue.pop_front()
                {
                    set.remove(&oldest);
                }
                set.insert(throttle_key.clone());
                queue.push_back(throttle_key);
                false
            }
        };
        if already_seen {
            tracing::debug!(
                target = "ironclaw::reborn::run_delivery",
                "busy-thread hint suppressed: already posted for this (conversation, event_id) pair (transport retry)"
            );
            return;
        }
        // Derive the scope for the run-state lookup so the hint can be
        // state-specific. When the conversation has no resolvable binding,
        // fall back to the generic copy rather than going silent — the hint
        // replies to the sender's own conversation and leaks nothing.
        let (hint, scope) = match self
            .services
            .binding_service
            .lookup_binding(ResolveBindingRequest::from_envelope(envelope))
            .await
        {
            Ok(binding) => {
                let hint = self.busy_hint_from_run_state(&binding, active_run_id).await;
                let scope = thread_scope_from_binding(&binding)
                    .and_then(|thread_scope| turn_scope_from_thread_scope(&binding, &thread_scope))
                    .unwrap_or_else(|_| self.services.fallback_notice_scope.clone());
                (hint, scope)
            }
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::reborn::run_delivery",
                    error = %error,
                    "busy-thread hint falling back to generic copy because the conversation binding was not resolved"
                );
                (
                    prompts::BUSY_GENERIC_MESSAGE.to_string(),
                    self.services.fallback_notice_scope.clone(),
                )
            }
        };
        self.services
            .post_notice(
                DeliveryIntent::FailureNotice,
                scope,
                Some(active_run_id),
                envelope.external_conversation_ref(),
                &hint,
                format!("busy-hint:{}", envelope.external_event_id().as_str()),
            )
            .await;
    }

    /// Looks up the blocking run's state and returns the appropriate
    /// busy-thread hint copy. Never errors — lookup failures degrade to the
    /// generic copy.
    async fn busy_hint_from_run_state(
        &self,
        binding: &ResolvedBinding,
        active_run_id: TurnRunId,
    ) -> String {
        let scope = match thread_scope_from_binding(binding)
            .and_then(|thread_scope| turn_scope_from_thread_scope(binding, &thread_scope))
        {
            Ok(scope) => scope,
            Err(err) => {
                tracing::debug!(
                    target = "ironclaw::reborn::run_delivery",
                    error = %err,
                    "busy-thread hint scope derivation failed; using generic copy"
                );
                return prompts::BUSY_GENERIC_MESSAGE.to_string();
            }
        };
        match self
            .services
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id: active_run_id,
            })
            .await
        {
            Ok(state) => match state.status {
                TurnStatus::BlockedApproval => match state.gate_ref.as_ref() {
                    // Name both the blocking gate ref AND what it would
                    // approve, so the user sees exactly what is holding the
                    // conversation.
                    Some(gate_ref) => {
                        let what = match &self.services.approval_context {
                            Some(source) => source
                                .approval_prompt_context(gate_ref, &binding.actor_user_id, &scope)
                                .await
                                .map(|ctx| ctx.tool_name),
                            None => None,
                        };
                        match what {
                            Some(tool) => format!(
                                "Ironclaw is waiting on your approval for `{tool}` before taking new \
                                 messages — reply `approve {ref}` to authorize it or `deny {ref}` to \
                                 decline.",
                                ref = gate_ref.as_str()
                            ),
                            None => format!(
                                "Ironclaw is waiting on a pending approval (`{ref}`) before taking new \
                                 messages — reply `approve {ref}` or `deny {ref}` to respond.",
                                ref = gate_ref.as_str()
                            ),
                        }
                    }
                    None => prompts::BUSY_APPROVAL_MESSAGE.to_string(),
                },
                // Auth gates can't be completed over chat (credential
                // sharing is a security risk), but still name the blocking
                // ref so the user can decline it here.
                TurnStatus::BlockedAuth => match state.gate_ref.as_ref() {
                    Some(gate_ref) => format!(
                        "Ironclaw is waiting on authentication before taking new messages. Reply \
                         `auth deny {ref}` to decline it here, or complete the connection in the \
                         Ironclaw web app to resume.",
                        ref = gate_ref.as_str()
                    ),
                    None => prompts::BUSY_GENERIC_MESSAGE.to_string(),
                },
                _ => prompts::BUSY_GENERIC_MESSAGE.to_string(),
            },
            Err(err) => {
                tracing::debug!(
                    target = "ironclaw::reborn::run_delivery",
                    error = %err,
                    "busy-thread hint run-state lookup failed; using generic copy"
                );
                prompts::BUSY_GENERIC_MESSAGE.to_string()
            }
        }
    }
}

/// The live-path reply-target authority: validates that the outbound target
/// the policy engine chose is exactly the run's own reply-target binding,
/// and resolves it to the originating conversation. Fully generic — the
/// envelope supplies the trusted conversation metadata.
pub(crate) struct ObservedReplyTargetAuthority {
    pub(crate) scope: TurnScope,
    pub(crate) actor: TurnActor,
    pub(crate) expected_target: ReplyTargetBindingRef,
    pub(crate) external_conversation_ref: ExternalConversationRef,
    pub(crate) external_actor_ref: Option<ExternalActorRef>,
}

#[async_trait]
impl ReplyTargetBindingValidator for ObservedReplyTargetAuthority {
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
impl crate::ProductOutboundTargetResolver for ObservedReplyTargetAuthority {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ironclaw_outbound::ValidatedReplyTargetBinding,
        require_direct_message: bool,
    ) -> Result<crate::VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        if target.target() != &self.expected_target {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        // The live path carries no DM classification for the raw source
        // conversation; DM-only payloads never set the flag here (the OAuth
        // URL is stripped upstream instead).
        if require_direct_message {
            return Err(ProductWorkflowError::OutboundTargetNotDirectMessage);
        }
        Ok(crate::VerifiedProductOutboundTargetMetadata {
            external_conversation_ref: self.external_conversation_ref.clone(),
            external_actor_ref: self.external_actor_ref.clone(),
        })
    }
}

pub(crate) struct AllowNoProjectionAccess;

#[async_trait]
impl ThreadProjectionAccessPolicy for AllowNoProjectionAccess {
    async fn authorize_projection_access(
        &self,
        _request: ThreadProjectionAccessRequest,
    ) -> Result<ThreadProjectionAccessClaim, OutboundError> {
        Err(OutboundError::AccessDenied)
    }
}

pub(crate) fn submitted_run_id(ack: &ProductInboundAck) -> Option<TurnRunId> {
    match ack {
        ProductInboundAck::Accepted {
            submitted_run_id, ..
        } => Some(*submitted_run_id),
        ProductInboundAck::Duplicate { .. } => None,
        ProductInboundAck::DeferredBusy { .. }
        | ProductInboundAck::RejectedBusy { .. }
        | ProductInboundAck::Rejected(_)
        | ProductInboundAck::CommandResult { .. }
        | ProductInboundAck::NoOp => None,
    }
}

fn submitted_run_id_for_feedback(_error: &RunDeliveryError) -> Option<TurnRunId> {
    None
}

fn should_deliver_after_ack(envelope: &ProductInboundEnvelope, ack: &ProductInboundAck) -> bool {
    if submitted_run_id(ack).is_none() {
        return false;
    }
    !matches!(
        envelope.payload(),
        ProductInboundPayload::AuthResolution(payload)
            if matches!(
                &payload.result,
                ironclaw_product_adapters::AuthResolutionResult::Denied
            )
    ) && !matches!(
        envelope.payload(),
        ProductInboundPayload::ApprovalResolution(payload)
            if payload.decision == ironclaw_product_adapters::ApprovalDecision::Deny
    ) && !matches!(
        envelope.payload(),
        ProductInboundPayload::ScopedApprovalResolution(payload)
            if payload.decision == ironclaw_product_adapters::ApprovalDecision::Deny
    )
}

/// The user-facing hint to post when a resolution attempt (approval or
/// auth) is rejected. `None` for non-resolution payloads or any `Duplicate`
/// ack — transport retries must not repeat side effects.
fn rejection_hint_for_resolution(
    envelope: &ProductInboundEnvelope,
    ack: &ProductInboundAck,
) -> Option<&'static str> {
    let ProductInboundAck::Rejected(effective_rejection) = ack else {
        return None;
    };
    let is_resolution = matches!(
        envelope.payload(),
        ProductInboundPayload::ApprovalResolution(_)
            | ProductInboundPayload::ScopedApprovalResolution(_)
            | ProductInboundPayload::AuthResolution(_)
    );
    if !is_resolution {
        return None;
    }
    let hint = match envelope.payload() {
        ProductInboundPayload::AuthResolution(_) => {
            effective_rejection.kind.user_facing_auth_hint()
        }
        _ => effective_rejection.kind.user_facing_hint(),
    };
    Some(hint)
}

/// `Some(active_run_id)` when the ack + payload combination should trigger
/// the busy-thread hint flow: a `DeferredBusy` (legacy) or `RejectedBusy`
/// ack on a `UserMessage` payload. `Duplicate` unwraps to the prior ack —
/// a settled `RejectedBusy` retried by the transport still carries the
/// blocking run id (the per-event throttle prevents double posts).
fn busy_hint_user_message_run_id(
    envelope: &ProductInboundEnvelope,
    ack: &ProductInboundAck,
) -> Option<TurnRunId> {
    if !matches!(envelope.payload(), ProductInboundPayload::UserMessage(_)) {
        return None;
    }
    match ack {
        ProductInboundAck::DeferredBusy { active_run_id, .. } => Some(*active_run_id),
        ProductInboundAck::RejectedBusy {
            active_run_id: Some(run_id),
            ..
        } => Some(*run_id),
        ProductInboundAck::RejectedBusy {
            active_run_id: None,
            ..
        } => None,
        ProductInboundAck::Duplicate { prior } => busy_hint_user_message_run_id(envelope, prior),
        _ => None,
    }
}

fn rejection_ack_for_workflow_error(error: &ProductAdapterError) -> Option<ProductInboundAck> {
    match error {
        ProductAdapterError::WorkflowRejected {
            kind,
            retryable: false,
            ..
        } => Some(ProductInboundAck::Rejected(ProductRejection::permanent(
            product_rejection_kind_for_workflow_rejection(*kind),
            "workflow rejected resolution",
        ))),
        _ => None,
    }
}

fn product_rejection_kind_for_workflow_rejection(
    kind: ProductWorkflowRejectionKind,
) -> ProductRejectionKind {
    match kind {
        ProductWorkflowRejectionKind::ScopeNotFound => ProductRejectionKind::BindingRequired,
        ProductWorkflowRejectionKind::Unauthorized => ProductRejectionKind::AccessDenied,
        ProductWorkflowRejectionKind::InvalidRequest => ProductRejectionKind::InvalidRequest,
        ProductWorkflowRejectionKind::Ambiguous => ProductRejectionKind::AmbiguousResolution,
        ProductWorkflowRejectionKind::ThreadBusy
        | ProductWorkflowRejectionKind::AdmissionRejected
        | ProductWorkflowRejectionKind::Unavailable
        | ProductWorkflowRejectionKind::Conflict => ProductRejectionKind::PolicyDenied,
    }
}

fn is_accepted_auth_denial(envelope: &ProductInboundEnvelope, ack: &ProductInboundAck) -> bool {
    submitted_run_id(ack).is_some()
        && matches!(
            envelope.payload(),
            ProductInboundPayload::AuthResolution(payload)
                if matches!(
                    &payload.result,
                    ironclaw_product_adapters::AuthResolutionResult::Denied
                )
        )
}

/// A payload is a private 1:1 direct chat iff the adapter classified the
/// originating user message as `DirectChat`. This is the same signal that
/// gates OAuth setup-link privacy.
pub(crate) fn envelope_is_direct_chat(envelope: &ProductInboundEnvelope) -> bool {
    matches!(
        envelope.payload(),
        ProductInboundPayload::UserMessage(payload)
            if payload.trigger == ProductTriggerReason::DirectChat
    )
}

/// OAuth setup links are only safe in a private DM.
fn auth_setup_link_is_private(envelope: &ProductInboundEnvelope) -> bool {
    envelope_is_direct_chat(envelope)
}

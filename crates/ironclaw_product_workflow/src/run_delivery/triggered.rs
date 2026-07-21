//! The proactive half of run delivery: watch a trigger-submitted run and
//! deliver its outputs to the creator's personal preference target, through
//! the [`DeliveryCoordinator`].

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, UserId};
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    OutboundError, OutboundPolicyService, PrepareCommunicationDeliveryRequest, ProjectionUpdateRef,
    ReplyTargetBindingClaim, ReplyTargetBindingValidator, ReplyTargetValidationRequest,
    RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin,
    TriggerCommunicationContext, TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryRecord,
    TriggeredRunDeliveryStore, ValidatedReplyTargetBinding,
};
use ironclaw_product_adapters::OutboundPart;
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, ThreadScope};
use ironclaw_turns::{TurnActor, TurnRunId, TurnRunState, TurnScope, TurnStatus};
use tokio::sync::Semaphore;

use super::observer::AllowNoProjectionAccess;
use super::prompts;
use super::{
    BlockedActionableMarker, BlockedAuthPromptRequest, DeliveredChannelMessage, RunDeliveryError,
    RunDeliveryServices, RunDeliverySettings, blocked_actionable_marker, cancel_auth_blocked_run,
    delivered_messages_from_outcome, gate_routes::record_gate_route_if_needed,
    triggered_run_delivery_settings, wait_for_actionable_state,
};
use crate::delivery_coordinator::{
    CoordinatedDeliveryError, CoordinatedDeliveryOutcome, CoordinatedDeliveryRequest,
    DeliveryIntent,
};
use crate::{ProductOutboundTargetResolver, ProductWorkflowError};

// The codec contract lives in `ironclaw_product_adapters` (the vendor half
// is implemented by channel extension crates, which never depend on this
// crate); re-exported here so the triggered-delivery consumers keep one
// import surface.
pub use ironclaw_product_adapters::{PreferenceTargetCodec, PreferenceTargetEncodeRequest};

/// One trigger-submitted run to watch and deliver, in generic vocabulary.
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
    pub trigger_context: TriggerCommunicationContext,
}

/// Notification content for one actionable triggered-run state.
struct TriggeredNotification {
    event_kind: RunNotificationEventKind,
    intent: DeliveryIntent,
    text: String,
    gate_ref_for_routing: Option<String>,
    /// AuthPrompt payloads carrying an OAuth URL must only land in a
    /// personal DM; enforced by the resolver at send time.
    require_direct_message_target: bool,
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

/// Drives triggered-run delivery: one background watcher per submitted run,
/// bounded by delivery and pending-admission semaphores, recording every
/// outcome in the [`TriggeredRunDeliveryStore`].
pub struct TriggeredRunDeliveryDriver {
    services: RunDeliveryServices,
    settings: RunDeliverySettings,
    delivery_permits: Arc<Semaphore>,
    /// Bounds the total number of spawned delivery tasks (active + waiting).
    /// Overflow is recorded as `Skipped` without spawning.
    pending_permits: Arc<Semaphore>,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    target_codec: Arc<dyn PreferenceTargetCodec>,
    /// Fallback agent id used when the submitted `TurnScope::agent_id` is
    /// `None`. Must match the default agent id the trigger prompt was
    /// recorded under so the thread-scope key aligns with the stored run.
    fallback_agent_id: AgentId,
}

impl TriggeredRunDeliveryDriver {
    pub fn new(
        services: RunDeliveryServices,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        target_codec: Arc<dyn PreferenceTargetCodec>,
        fallback_agent_id: AgentId,
    ) -> Self {
        Self::with_settings(
            services,
            triggered_run_delivery_settings(),
            delivery_store,
            target_codec,
            fallback_agent_id,
        )
    }

    pub fn with_settings(
        services: RunDeliveryServices,
        settings: RunDeliverySettings,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        target_codec: Arc<dyn PreferenceTargetCodec>,
        fallback_agent_id: AgentId,
    ) -> Self {
        let delivery_permits = Arc::new(Semaphore::new(settings.max_concurrent_deliveries.get()));
        let pending_permits = Arc::new(Semaphore::new(settings.max_pending_deliveries.get()));
        Self {
            services,
            settings,
            delivery_permits,
            pending_permits,
            delivery_store,
            target_codec,
            fallback_agent_id,
        }
    }

    /// Acquire a permit from the pending-delivery semaphore for testing:
    /// lets tests hold the pending slot without spawning a real delivery
    /// task, so `Skipped` outcomes are assertable.
    #[cfg(any(test, feature = "test-support"))]
    pub fn try_acquire_pending_permit(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        Arc::clone(&self.pending_permits).try_acquire_owned().ok()
    }

    /// The preference repository this driver resolves targets from.
    /// Production wiring must hand the SAME store the WebUI writes, so
    /// user-set preferences are visible here; tests assert pointer equality.
    pub fn communication_preferences(
        &self,
    ) -> Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository> {
        Arc::clone(&self.services.communication_preferences)
    }

    /// Watch one submitted triggered run and deliver its outputs. Spawns a
    /// bounded background task; the call returns once admission is decided.
    pub async fn on_trigger_submitted(&self, request: TriggeredRunDeliveryRequest) {
        // Fail closed for non-personal triggers.
        if request.project_scoped {
            tracing::debug!(
                run_id = %request.run_id,
                "triggered run delivery denied: project-scoped trigger is not personal scope"
            );
            record_triggered_run_outcome(
                &*self.delivery_store,
                request.run_id,
                TriggeredRunDeliveryOutcomeKind::Denied,
            )
            .await;
            return;
        }

        // Guard against unbounded task accumulation: if the pending queue is
        // full, record Skipped immediately without spawning.
        let Ok(pending_permit) = Arc::clone(&self.pending_permits).try_acquire_owned() else {
            tracing::warn!(
                target: "ironclaw::reborn::run_delivery",
                run_id = %request.run_id,
                "triggered run delivery skipped: pending delivery queue full"
            );
            record_triggered_run_outcome(
                &*self.delivery_store,
                request.run_id,
                TriggeredRunDeliveryOutcomeKind::Skipped,
            )
            .await;
            return;
        };

        let permits = Arc::clone(&self.delivery_permits);
        let services = self.services.clone();
        let settings = self.settings;
        let delivery_store = Arc::clone(&self.delivery_store);
        let target_codec = Arc::clone(&self.target_codec);
        let fallback_agent_id = self.fallback_agent_id.clone();

        tokio::spawn(async move {
            // Hold the pending permit for the full task lifetime so it
            // counts against the cap until delivery completes.
            let _pending_permit = pending_permit;

            let Ok(_permit) = permits.clone().acquire_owned().await else {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    run_id = %request.run_id,
                    "triggered run delivery skipped: delivery semaphore closed"
                );
                record_triggered_run_outcome(
                    &*delivery_store,
                    request.run_id,
                    TriggeredRunDeliveryOutcomeKind::Skipped,
                )
                .await;
                return;
            };

            let run_id = request.run_id;
            let outcome = deliver_triggered_run(
                &services,
                &settings,
                request,
                &*delivery_store,
                target_codec.as_ref(),
                &fallback_agent_id,
            )
            .await;
            tracing::debug!(
                target = "ironclaw::reborn::run_delivery",
                %run_id,
                ?outcome,
                "triggered run delivery completed"
            );
        });
    }
}

/// Inner delivery coroutine for a single triggered run.
///
/// ## Invariant: a parked-awaiting-user run is terminal-for-delivery
///
/// After the actionable gate/auth prompt for a blocked run has been
/// delivered, the run typically *stays* blocked until the user acts — the
/// common case, not a failure. If the re-wait hits the `max_wait` backstop,
/// the run is parked awaiting the user: that is a successful,
/// terminal-for-delivery outcome (`Delivered`) — never record `Failed` for
/// it. The backstop is the failure signal ONLY for runs that never reached
/// an actionable state at all, distinguished by `delivered_blocked_marker`.
async fn deliver_triggered_run(
    services: &RunDeliveryServices,
    settings: &RunDeliverySettings,
    request: TriggeredRunDeliveryRequest,
    delivery_store: &dyn TriggeredRunDeliveryStore,
    target_codec: &dyn PreferenceTargetCodec,
    fallback_agent_id: &AgentId,
) -> TriggeredRunDeliveryOutcomeKind {
    let TriggeredRunDeliveryRequest {
        run_id,
        scope,
        creator_user_id,
        project_scoped: _,
        prompt,
        trigger_context,
    } = request;
    let actor = TurnActor::new(creator_user_id);

    // Thread scope for reading the finalized assistant message: the turn
    // scope's thread is the canonical trigger-session thread; the fallback
    // agent id matches how the trigger prompt was recorded.
    let thread_scope = ThreadScope {
        tenant_id: scope.tenant_id.clone(),
        agent_id: scope
            .agent_id
            .clone()
            .unwrap_or_else(|| fallback_agent_id.clone()),
        project_id: scope.project_id.clone(),
        owner_user_id: scope.explicit_owner_user_id().cloned(),
        mission_id: None,
    };

    let authority = TriggeredReplyTargetAuthority {
        scope: scope.clone(),
        actor: actor.clone(),
        codec: target_codec,
    };

    let mut delivered_blocked_marker: Option<BlockedActionableMarker> = None;
    let mut messages_to_delete_after_final: Vec<DeliveredChannelMessage> = Vec::new();

    loop {
        let state = match wait_for_actionable_state(
            services.turn_coordinator.as_ref(),
            &scope,
            run_id,
            settings,
            delivered_blocked_marker.as_ref(),
        )
        .await
        {
            Ok(state) => state,
            Err(RunDeliveryError::RunWaitTimedOut { .. }) if delivered_blocked_marker.is_some() => {
                // Parked awaiting the user after its prompt went out — a
                // successful, terminal-for-delivery outcome. The prompt must
                // stay actionable, so stale-prompt cleanup deliberately does
                // NOT run here.
                tracing::debug!(
                    target = "ironclaw::reborn::run_delivery",
                    %run_id,
                    "triggered run parked awaiting user after delivering blocked prompt; recording Delivered"
                );
                let outcome = TriggeredRunDeliveryOutcomeKind::Delivered;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
            Err(err) => {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    %run_id,
                    error = %err,
                    "triggered run wait failed"
                );
                let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
        };

        let trigger_label = prompts::triggered_label_from_prompt(&prompt);
        let notification = match triggered_notification_for_state(
            services,
            &scope,
            &thread_scope,
            &actor,
            &state,
            run_id,
            &trigger_label,
        )
        .await
        {
            Ok(Some(notification)) => notification,
            Ok(None) => {
                // Run completed with no assistant message — a normal
                // "skipped" outcome.
                let outcome = TriggeredRunDeliveryOutcomeKind::Skipped;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
            Err(err) => {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    %run_id,
                    error = %err,
                    "triggered run notification build failed"
                );
                let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
        };

        let next_blocked_marker = blocked_actionable_marker(&state);
        let event_kind = notification.event_kind;
        let gate_ref_for_routing = notification.gate_ref_for_routing.clone();

        let delivery_result = deliver_triggered_notification(
            services,
            &scope,
            &actor,
            run_id,
            &trigger_context,
            &authority,
            notification,
        )
        .await;

        match delivery_result {
            Ok(delivered_messages) => {
                if (event_kind == RunNotificationEventKind::ApprovalNeeded
                    || event_kind == RunNotificationEventKind::AuthRequired)
                    && let Some(gate_ref) = gate_ref_for_routing.as_deref()
                {
                    record_gate_route_if_needed(
                        services.route_store.as_ref(),
                        run_id,
                        &scope.tenant_id,
                        &actor.user_id,
                        gate_ref,
                        &scope,
                        &delivered_messages,
                        None,
                    )
                    .await;
                }
                if let Some(marker) = next_blocked_marker
                    && matches!(
                        event_kind,
                        RunNotificationEventKind::ApprovalNeeded
                            | RunNotificationEventKind::AuthRequired
                    )
                {
                    if event_kind == RunNotificationEventKind::AuthRequired {
                        messages_to_delete_after_final.extend(delivered_messages);
                    }
                    delivered_blocked_marker = Some(marker);
                    continue;
                }
                // Terminal delivery — clean up auth prompts that should not
                // persist.
                for message in messages_to_delete_after_final {
                    services
                        .retract_message(scope.clone(), Some(run_id), message)
                        .await;
                }
                let outcome = TriggeredRunDeliveryOutcomeKind::Delivered;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
            Err(TriggeredNotificationFailure::OAuthTargetNotDm) => {
                // Send-time backstop tripped: the payload carried an OAuth
                // authorization_url but the binding was not a personal DM.
                // Cancel the blocked run FIRST — a transient cancel failure
                // must leave the existing prompt in place.
                tracing::debug!(
                    target = "ironclaw::reborn::run_delivery",
                    %run_id,
                    "triggered run OAuth URL suppressed by send-time backstop: resolved \
                     target is not a personal DM; cancelling run"
                );
                if let Err(err) = cancel_auth_blocked_run(
                    services.turn_coordinator.as_ref(),
                    services.auth_flow_cancel.as_deref(),
                    &scope,
                    actor.clone(),
                    run_id,
                    state.gate_ref.as_ref().map(|gate_ref| gate_ref.as_str()),
                )
                .await
                {
                    tracing::debug!(
                        target = "ironclaw::reborn::run_delivery",
                        %run_id,
                        error = %err,
                        "triggered run OAuth backstop: cancel_auth_blocked_run failed"
                    );
                    let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
                    record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                    return outcome;
                }
                // Post the auth-unavailable notice as a terminal FinalReply.
                // No DM restriction applies: plain text, no OAuth URL.
                let notice = TriggeredNotification {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    intent: DeliveryIntent::FinalReply,
                    text: format!(
                        "{}{}",
                        prompts::AUTH_UNAVAILABLE_MESSAGE,
                        prompts::triggered_update_footer(&trigger_label)
                    ),
                    gate_ref_for_routing: None,
                    require_direct_message_target: false,
                };
                let outcome = match deliver_triggered_notification(
                    services,
                    &scope,
                    &actor,
                    run_id,
                    &trigger_context,
                    &authority,
                    notice,
                )
                .await
                {
                    Ok(_) => TriggeredRunDeliveryOutcomeKind::Delivered,
                    Err(TriggeredNotificationFailure::NoDefaultConfigured) => {
                        TriggeredRunDeliveryOutcomeKind::NoDefaultConfigured
                    }
                    Err(TriggeredNotificationFailure::Denied) => {
                        TriggeredRunDeliveryOutcomeKind::Denied
                    }
                    Err(TriggeredNotificationFailure::OAuthTargetNotDm)
                    | Err(TriggeredNotificationFailure::Other(_)) => {
                        TriggeredRunDeliveryOutcomeKind::Failed
                    }
                };
                // Only after a successful cancel and the replacement notice:
                // remove the now-stale OAuth prompts.
                for message in messages_to_delete_after_final.drain(..) {
                    services
                        .retract_message(scope.clone(), Some(run_id), message)
                        .await;
                }
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
            Err(failure) => {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    %run_id,
                    reason = %failure,
                    "triggered run delivery failed"
                );
                let outcome = match failure {
                    TriggeredNotificationFailure::NoDefaultConfigured => {
                        TriggeredRunDeliveryOutcomeKind::NoDefaultConfigured
                    }
                    TriggeredNotificationFailure::Denied => TriggeredRunDeliveryOutcomeKind::Denied,
                    TriggeredNotificationFailure::OAuthTargetNotDm => {
                        unreachable!("OAuthTargetNotDm is handled by the dedicated arm above")
                    }
                    TriggeredNotificationFailure::Other(_) => {
                        TriggeredRunDeliveryOutcomeKind::Failed
                    }
                };
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
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
                text: format!("{text}{}", prompts::triggered_update_footer(trigger_label)),
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
                text: prompts::gate_prompt_text(&view, true),
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
            match view {
                Some(mut view) if view.authorization_url.is_some() => {
                    view.body
                        .push_str(&prompts::triggered_gate_footer(trigger_label));
                    // The DM requirement is enforced by the resolver at send
                    // time (closing the snapshot-vs-send race); no pre-check
                    // here.
                    Ok(Some(TriggeredNotification {
                        event_kind: RunNotificationEventKind::AuthRequired,
                        intent: DeliveryIntent::AuthPrompt,
                        text: prompts::auth_prompt_text(&view, true),
                        gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
                        require_direct_message_target: true,
                    }))
                }
                _ => {
                    // Non-OAuth challenge (manual credential entry). Deny:
                    // cancel the parked run and deliver the auth-unavailable
                    // notice as the terminal reply.
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
                        text: format!(
                            "{}{}",
                            prompts::AUTH_UNAVAILABLE_MESSAGE,
                            prompts::triggered_update_footer(trigger_label)
                        ),
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
    scope: &TurnScope,
    actor: &TurnActor,
    run_id: TurnRunId,
    trigger_context: &TriggerCommunicationContext,
    authority: &TriggeredReplyTargetAuthority<'_>,
    notification: TriggeredNotification,
) -> Result<Vec<DeliveredChannelMessage>, TriggeredNotificationFailure> {
    let projection_access_policy = AllowNoProjectionAccess;
    let outbound_policy = OutboundPolicyService::new(
        services.outbound_store.as_ref(),
        &projection_access_policy,
        authority,
    );
    let projection_id = prompts::run_notification_projection_id(run_id, notification.event_kind);
    let projection_ref = ProjectionUpdateRef::new(projection_id).map_err(|reason| {
        TriggeredNotificationFailure::Other(format!("invalid_projection_ref: {reason}"))
    })?;
    let delivery = PrepareCommunicationDeliveryRequest {
        resolution_request: CommunicationDeliveryResolutionRequest {
            scope: scope.clone(),
            actor: actor.clone(),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind: notification.event_kind,
                origin: RunNotificationOrigin::Triggered {
                    trigger: trigger_context.clone(),
                },
            }),
        },
        turn_run_id: Some(run_id),
        projection_ref,
        attempted_at: Utc::now(),
    };

    let outcome = services
        .coordinator
        .deliver(
            &outbound_policy,
            services.communication_preferences.as_ref(),
            authority,
            CoordinatedDeliveryRequest {
                intent: notification.intent,
                delivery,
                parts: vec![OutboundPart::Text(notification.text)],
                thread_anchor: None,
                require_direct_message_target: notification.require_direct_message_target,
                extension_id: &services.extension_id,
            },
        )
        .await
        .map_err(classify_delivery_error)?;
    match outcome {
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
    let record = TriggeredRunDeliveryRecord {
        run_id,
        outcome,
        recorded_at: Utc::now(),
    };
    if let Err(error) = store.record_triggered_run_delivery(record).await {
        tracing::warn!(
            target = "ironclaw::reborn::run_delivery",
            %run_id,
            error = %error,
            "failed to record triggered run delivery outcome (best-effort)"
        );
    }
}

/// Reply-target authority for triggered-run delivery: trusts the target the
/// resolution engine chose from the creator's personal preference (scope
/// and actor must match), decodes it through the vendor codec port, and
/// enforces the DM requirement against the send-time binding.
struct TriggeredReplyTargetAuthority<'a> {
    scope: TurnScope,
    actor: TurnActor,
    codec: &'a dyn PreferenceTargetCodec,
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
        // Single enforcement point for the OAuth DM rule, checked against
        // the binding resolved NOW (at send time) — race-free against a
        // stale preference snapshot.
        if require_direct_message && !self.codec.is_personal_direct_message(target.target()) {
            return Err(ProductWorkflowError::OutboundTargetNotDirectMessage);
        }
        let external_conversation_ref = self
            .codec
            .conversation_for_target(target.target())
            .ok_or_else(|| ProductWorkflowError::BindingResolutionFailed {
                reason: format!(
                    "triggered delivery: cannot decode conversation from binding ref '{}'",
                    target.target().as_str()
                ),
            })?;
        Ok(crate::VerifiedProductOutboundTargetMetadata {
            external_conversation_ref,
            external_actor_ref: None,
        })
    }
}

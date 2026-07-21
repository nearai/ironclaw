use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_channel_host::outbound_targets::OutboundDeliveryTargetProvider;
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    DeliveredGateRouteStore, OutboundError, OutboundPolicyService, ProjectionUpdateRef,
    ReplyTargetBindingClaim, ReplyTargetBindingValidator, ReplyTargetValidationRequest,
    RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin, SourceRouteContext,
    TriggerCommunicationContext, TriggerFireSlot, TriggerOriginRef, TriggerSourceKind,
    TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryRecord, TriggeredRunDeliveryStore,
    ValidatedReplyTargetBinding,
};
use ironclaw_product_adapters::{
    AuthPromptView, ExternalConversationRef, FinalReplyView, ProductOutboundPayload,
};
use ironclaw_product_workflow::{
    ProductOutboundDeliveryRequest, ProductOutboundTargetResolver, ProductWorkflowError,
    ResolvedBinding, VerifiedProductOutboundTargetMetadata, approval_prompt_context_view,
    enrich_auth_prompt_view, prepare_and_render_product_outbound,
};
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, SessionThreadService, ThreadScope};
use ironclaw_triggers::TriggerFire;
use ironclaw_turns::{
    GetRunStateRequest, ReplyTargetBindingRef, TurnActor, TurnCoordinator, TurnRunId, TurnRunState,
    TurnScope, TurnStatus,
};
use tokio::sync::Semaphore;

use crate::actionable::{
    AllowNoProjectionAccess, blocked_actionable_marker, cancel_auth_blocked_run,
    channel_approval_gate_prompt_view, channel_run_notification_projection_id,
    enforce_direct_message_if_required, jittered_poll_interval,
};
use crate::hooks::{PostSubmitDeliveryError, PostSubmitDeliveryHook};
use crate::routing::{TrackingPostEgress, record_gate_route_if_needed};
use crate::services::*;

/// Drives triggered-run delivery for a single submitted run.
///
/// Polls the run to completion (or gate) and delivers the result to the
/// configured channel target inside the trigger poller's managed task.
/// Project-scoped fires are denied by the existing product policy.
pub struct TriggeredRunDeliveryDriver {
    services: FinalReplyDeliveryServices,
    pub(crate) settings: FinalReplyDeliverySettings,
    delivery_permits: Arc<Semaphore>,
    /// Bounds the total number of admitted delivery calls (active + waiting).
    /// Acquired via `try_acquire_owned`; overflow is recorded as `Skipped`.
    pending_permits: Arc<Semaphore>,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    route_store: Arc<dyn DeliveredGateRouteStore>,
    /// Fallback agent id used when the submitted `TurnScope::agent_id` is
    /// `None`. Must match the `default_agent_id` that
    /// `ConversationContentRefMaterializer` (and `record_trigger_prompt`)
    /// uses so the thread-scope key aligns with where the run was stored.
    fallback_agent_id: ironclaw_host_api::AgentId,
    /// Resolves per-trigger delivery targets (`TriggerFire::delivery_target`)
    /// into reply-target bindings. Consulted only for fires that carry a
    /// target; fires without one use the creator's preference as before.
    /// Production wiring (`build_triggered_run_delivery_hook`) always supplies
    /// it; when absent (reduced test drivers), a fire carrying a target fails
    /// closed as `TargetUnavailable` — see
    /// `driver_fire_with_unresolvable_delivery_target_records_target_unavailable`.
    // arch-exempt: optional_arc, reduced test drivers omit the cross-owner target strategy while production wiring supplies it and targeted fires fail closed without it, plan #6159
    outbound_target_provider: Option<Arc<dyn OutboundDeliveryTargetProvider>>,
}

impl TriggeredRunDeliveryDriver {
    /// Test-support wrapper for the existing direct behavior corpus. Runtime
    /// delivery goes through [`PostSubmitDeliveryHook`], which returns
    /// authoritative persistence failures to its managed task owner.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn on_trigger_submitted(
        &self,
        fire: TriggerFire,
        run_id: TurnRunId,
        scope: TurnScope,
    ) {
        if let Err(error) = self.run_post_submit_delivery(fire, run_id, scope).await {
            panic!("test delivery must persist its terminal outcome for run {run_id}: {error}");
        }
    }

    pub fn new(
        services: FinalReplyDeliveryServices,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        route_store: Arc<dyn DeliveredGateRouteStore>,
        fallback_agent_id: ironclaw_host_api::AgentId,
    ) -> Self {
        Self::with_settings(
            services,
            FinalReplyDeliverySettings {
                max_wait: DEFAULT_TRIGGERED_RUN_DELIVERY_MAX_WAIT,
                ..FinalReplyDeliverySettings::default()
            },
            delivery_store,
            route_store,
            fallback_agent_id,
        )
    }

    pub fn with_settings(
        services: FinalReplyDeliveryServices,
        settings: FinalReplyDeliverySettings,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        route_store: Arc<dyn DeliveredGateRouteStore>,
        fallback_agent_id: ironclaw_host_api::AgentId,
    ) -> Self {
        let delivery_permits = Arc::new(Semaphore::new(settings.max_concurrent_deliveries.get()));
        let pending_permits = Arc::new(Semaphore::new(settings.max_pending_deliveries.get()));
        Self {
            services,
            settings,
            delivery_permits,
            pending_permits,
            delivery_store,
            route_store,
            fallback_agent_id,
            outbound_target_provider: None,
        }
    }

    /// Wire the outbound delivery target provider used to resolve per-trigger
    /// delivery targets. Production wiring must call this; without it, fires
    /// carrying a `delivery_target` fail closed as `TargetUnavailable`.
    pub fn with_outbound_target_provider(
        mut self,
        provider: Arc<dyn OutboundDeliveryTargetProvider>,
    ) -> Self {
        self.outbound_target_provider = Some(provider);
        self
    }

    /// Acquire a permit from the pending-delivery semaphore for testing.
    ///
    /// Allows tests to hold the pending slot without spawning a real delivery
    /// task, making it straightforward to assert `Skipped` outcomes when the
    /// queue is full.
    #[cfg(test)]
    pub fn try_acquire_pending_permit(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        Arc::clone(&self.pending_permits).try_acquire_owned().ok()
    }

    /// Returns the `CommunicationPreferenceRepository` wired into this driver's
    /// `FinalReplyDeliveryServices.communication_preferences`.
    ///
    /// Production call site: `build_triggered_run_delivery_hook` in
    /// `slack_host_beta.rs` — the store it passes here must be pointer-equal to
    /// `local_runtime.outbound_preferences` so WebUI-written preferences are
    /// visible to Slack delivery.  Use `Arc::ptr_eq` in tests to assert this.
    /// This accessor is for tests only and compiles to nothing in production binaries.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn communication_preferences_for_test(
        &self,
    ) -> Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository> {
        Arc::clone(&self.services.communication_preferences)
    }
}

#[async_trait]
impl PostSubmitDeliveryHook for TriggeredRunDeliveryDriver {
    async fn on_trigger_submitted(
        &self,
        fire: TriggerFire,
        run_id: TurnRunId,
        scope: TurnScope,
    ) -> Result<(), PostSubmitDeliveryError> {
        self.run_post_submit_delivery(fire, run_id, scope).await
    }
}

impl TriggeredRunDeliveryDriver {
    async fn run_post_submit_delivery(
        &self,
        fire: TriggerFire,
        run_id: TurnRunId,
        scope: TurnScope,
    ) -> Result<(), PostSubmitDeliveryError> {
        // Fail closed for non-personal triggers (project_id set means shared/project scope).
        if fire.project_id.is_some() {
            tracing::debug!(
                %run_id,
                "triggered run delivery denied: project-scoped trigger is not personal scope"
            );
            self.record_outcome(run_id, TriggeredRunDeliveryOutcomeKind::Denied)
                .await
                .map_err(outcome_persistence_error)?;
            return Ok(());
        }

        // Guard against unbounded delivery accumulation: if the pending queue
        // is full, record Skipped immediately.
        let Ok(pending_permit) = Arc::clone(&self.pending_permits).try_acquire_owned() else {
            tracing::debug!(
                target: "ironclaw::reborn::channel_delivery",
                %run_id,
                "triggered run delivery skipped: pending delivery queue full"
            );
            self.record_outcome(run_id, TriggeredRunDeliveryOutcomeKind::Skipped)
                .await
                .map_err(outcome_persistence_error)?;
            return Ok(());
        };

        // Clone the retained ports used for this managed delivery call.
        let permits = Arc::clone(&self.delivery_permits);
        let services = FinalReplyDeliveryServices {
            channel_protocol: Arc::clone(&self.services.channel_protocol),
            binding_service: Arc::clone(&self.services.binding_service),
            thread_service: Arc::clone(&self.services.thread_service),
            turn_coordinator: Arc::clone(&self.services.turn_coordinator),
            outbound_store: Arc::clone(&self.services.outbound_store),
            route_store: Arc::clone(&self.route_store),
            communication_preferences: Arc::clone(&self.services.communication_preferences),
            adapter: Arc::clone(&self.services.adapter),
            egress: Arc::clone(&self.services.egress),
            delivery_sink: Arc::clone(&self.services.delivery_sink),
            auth_challenges: self.services.auth_challenges.clone(),
            auth_flow_canceller: self.services.auth_flow_canceller.clone(),
            approval_requests: self.services.approval_requests.clone(),
        };
        let settings = self.settings;
        let delivery_store = Arc::clone(&self.delivery_store);
        let fallback_agent_id = self.fallback_agent_id.clone();
        let outbound_target_provider = self.outbound_target_provider.clone();

        // The trigger poller invokes this hook from its bounded lifecycle
        // owner. Await the actual delivery here so shutdown can drain it.
        let _pending_permit = pending_permit;

        let Ok(_permit) = permits.clone().acquire_owned().await else {
            tracing::debug!(
                target = "ironclaw::reborn::channel_delivery",
                %run_id,
                "triggered run delivery skipped: delivery semaphore closed"
            );
            record_triggered_run_outcome(
                &*delivery_store,
                run_id,
                TriggeredRunDeliveryOutcomeKind::Skipped,
            )
            .await
            .map_err(outcome_persistence_error)?;
            return Ok(());
        };

        let outcome = deliver_triggered_run(
            &services,
            &settings,
            &fire,
            run_id,
            scope,
            &*delivery_store,
            &fallback_agent_id,
            outbound_target_provider.as_deref(),
        )
        .await
        .map_err(outcome_persistence_error)?;
        tracing::debug!(
            target = "ironclaw::reborn::channel_delivery",
            %run_id,
            ?outcome,
            "triggered run delivery completed"
        );
        Ok(())
    }
}

fn outcome_persistence_error(reason: String) -> PostSubmitDeliveryError {
    PostSubmitDeliveryError::new(format!(
        "authoritative triggered-delivery outcome persistence failed: {reason}"
    ))
}

impl TriggeredRunDeliveryDriver {
    async fn record_outcome(
        &self,
        run_id: TurnRunId,
        outcome: TriggeredRunDeliveryOutcomeKind,
    ) -> Result<(), String> {
        record_triggered_run_outcome(&*self.delivery_store, run_id, outcome).await
    }
}

/// Inner delivery coroutine for a single triggered run.
///
/// ## Invariant: a parked-awaiting-user run is terminal-for-delivery
///
/// After the actionable gate/auth prompt for a `BlockedApproval` / `BlockedAuth`
/// run has been delivered, the run typically *stays* blocked until the user acts
/// (approve / re-auth) — which is the common case, not a failure. The wait loop
/// re-enters `wait_for_actionable_triggered` to handle the eventual transition to
/// `Completed` (deliver the final reply, delete the stale OAuth prompt). If that
/// re-wait hits the `max_wait` backstop, the run is parked awaiting the user: that
/// is a successful, terminal-for-delivery outcome (`Delivered`) — NEVER record
/// `Failed` for it, and never poll it for `Completed`. The `max_wait` backstop is
/// the failure signal ONLY for runs that never reached an actionable state at all
/// (still running / stuck), distinguished here by `delivered_blocked_marker`. See
/// `docs/plans/2026-06-25-slack-delivery-blocked-terminal.md` for the production
/// incident (23× spurious `Failed` after 30-min polls) this guards against.
// arch-exempt: too_many_args, the delivery algorithm receives the shared services bundle plus typed fire/run evidence and the retained cross-owner target strategy, plan #6159
#[allow(clippy::too_many_arguments)]
async fn deliver_triggered_run(
    services: &FinalReplyDeliveryServices,
    settings: &FinalReplyDeliverySettings,
    fire: &TriggerFire,
    run_id: TurnRunId,
    scope: TurnScope,
    delivery_store: &dyn TriggeredRunDeliveryStore,
    fallback_agent_id: &ironclaw_host_api::AgentId,
    outbound_target_provider: Option<&dyn OutboundDeliveryTargetProvider>,
) -> Result<TriggeredRunDeliveryOutcomeKind, String> {
    // The actor is the trigger creator.
    let actor = TurnActor::new(fire.creator_user_id.clone());

    // Resolve the per-trigger delivery target (when the fire carries one) into
    // a reply-target binding BEFORE any delivery work. The resolution engine
    // then prefers this source route over the creator's user-global preference
    // (`RunNotificationOrigin::TriggeredFromSourceRoute`). Resolution failures
    // fail closed — a stale or foreign target must never silently fall back to
    // another conversation.
    let per_trigger_source_route = match &fire.delivery_target {
        None => None,
        Some(target) => {
            let resolved =
                resolve_per_trigger_delivery_route(outbound_target_provider, fire, &scope, target)
                    .await;
            match resolved {
                Ok(route) => Some(route),
                Err(outcome) => {
                    tracing::warn!(
                        target = "ironclaw::reborn::channel_delivery",
                        %run_id,
                        ?outcome,
                        "triggered run delivery stopped: per-trigger delivery target did not resolve"
                    );
                    record_triggered_run_outcome(delivery_store, run_id, outcome).await?;
                    return Ok(outcome);
                }
            }
        }
    };

    // Build a thread scope for reading the finalized assistant message.
    // The turn scope's thread_id is the canonical thread for this trigger session.
    // Use the scope's agent_id when present; otherwise fall back to the configured
    // fallback_agent_id — the same value record_trigger_prompt uses — so the key
    // matches the thread that was stored at submit time.
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

    // Build the reply-target authority: resolves from the per-trigger source
    // route when present, otherwise from the creator's personal preference.
    let authority = match TriggeredChannelReplyTargetAuthority::from_fire(
        Arc::clone(&services.channel_protocol),
        scope.clone(),
        actor.clone(),
        fire,
        per_trigger_source_route,
    ) {
        Ok(authority) => authority,
        Err(reason) => {
            tracing::warn!(
                target = "ironclaw::reborn::channel_delivery",
                %run_id,
                %reason,
                "triggered run delivery skipped: cannot build trigger context"
            );
            let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
            record_triggered_run_outcome(delivery_store, run_id, outcome).await?;
            return Ok(outcome);
        }
    };

    let mut delivered_blocked_marker: Option<BlockedActionableMarker> = None;
    let mut messages_to_delete_after_final = Vec::new();

    loop {
        // Poll until the run reaches an actionable state.
        let state = match wait_for_actionable_triggered(
            services,
            &scope,
            run_id,
            settings,
            &delivered_blocked_marker,
        )
        .await
        {
            Ok(s) => s,
            Err(FinalReplyDeliveryError::RunWaitTimedOut { .. })
                if delivered_blocked_marker.is_some() =>
            {
                // The run is parked in a Blocked* state (awaiting the user's
                // approval or re-auth) AFTER we already delivered its actionable
                // gate/auth prompt. This is the common, expected case — the user
                // simply has not acted within the wait backstop. The prompt is
                // out; the user's resolution arrives later as a separate inbound
                // event and is bridged back via the delivered-gate route. Treat
                // this as a successful, terminal-for-delivery outcome instead of
                // polling to the backstop and recording a generic `Failed` (which
                // also clobbers the `Delivered` we already earned). Mirrors the
                // live-run path's `RunWaitTimedOutAfterNotification` "quiet
                // success" semantics. The auth prompt must remain actionable, so
                // we intentionally do NOT delete `messages_to_delete_after_final`
                // here — they are cleaned up only on a real terminal final reply.
                tracing::debug!(
                    target = "ironclaw::reborn::channel_delivery",
                    %run_id,
                    "triggered run parked awaiting user after delivering blocked prompt; recording Delivered"
                );
                let outcome = TriggeredRunDeliveryOutcomeKind::Delivered;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await?;
                return Ok(outcome);
            }
            Err(err) => {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_delivery",
                    %run_id,
                    error = %err,
                    "triggered run wait failed"
                );
                let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await?;
                return Ok(outcome);
            }
        };

        // Build the notification payload. The trigger prompt becomes the short
        // routine label in the footer appended to every triggered Slack message.
        let trigger_label = triggered_label_from_prompt(&fire.prompt);
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
            Ok(Some(n)) => n,
            Ok(None) => {
                // Run completed with no assistant message — normal "skipped" outcome.
                let outcome = TriggeredRunDeliveryOutcomeKind::Skipped;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await?;
                return Ok(outcome);
            }
            Err(err) => {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_delivery",
                    %run_id,
                    error = %err,
                    "triggered run notification build failed"
                );
                let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await?;
                return Ok(outcome);
            }
        };

        let next_blocked_marker = blocked_actionable_marker(&state);
        let event_kind = notification.event_kind;
        let gate_ref_for_routing = notification.gate_ref_for_routing.clone();

        // Compute the DM requirement from the payload BEFORE it is moved into the call.
        // AuthPrompt payloads with an authorization_url must only be delivered to a
        // personal DM; pass this requirement through the delivery request so the
        // resolver enforces it at send time (closing the snapshot-vs-send race).
        let require_direct_message_target = matches!(
            &notification.payload,
            ProductOutboundPayload::AuthPrompt(view)
                if view.authorization_url.is_some()
        );

        // Build the delivery request and deliver.
        let delivery_result = deliver_triggered_notification(
            services,
            &scope,
            &actor,
            run_id,
            &state,
            &authority,
            notification,
            require_direct_message_target,
        )
        .await;

        match delivery_result {
            Ok(posted_messages) => {
                if (event_kind == RunNotificationEventKind::ApprovalNeeded
                    || event_kind == RunNotificationEventKind::AuthRequired)
                    && let Some(gate_ref) = gate_ref_for_routing.as_deref()
                {
                    // Read the space id that was captured during target resolution.
                    let space_id = {
                        let space_id_guard = authority
                            .resolved_space_id
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        space_id_guard.clone()
                    };
                    record_gate_route_if_needed(
                        services.route_store.as_ref(),
                        run_id,
                        &scope.tenant_id,
                        &fire.creator_user_id,
                        gate_ref,
                        &scope,
                        &posted_messages,
                        None,
                        space_id.as_deref(),
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
                        messages_to_delete_after_final.extend(posted_messages);
                    }
                    delivered_blocked_marker = Some(marker);
                    // Loop again to wait for the next actionable state.
                    continue;
                }
                // Terminal delivery — clean up auth messages that should not persist.
                for message in messages_to_delete_after_final {
                    delete_triggered_channel_message(services, message).await;
                }
                let outcome = TriggeredRunDeliveryOutcomeKind::Delivered;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await?;
                return Ok(outcome);
            }
            Err(TriggeredNotificationFailure::OAuthTargetNotDm) => {
                // Authority backstop tripped: the payload carried an OAuth
                // authorization_url but the send-time binding was not a personal DM.
                // Suppress the URL (fail closed), cancel the blocked run, then post
                // the auth-unavailable notice as a terminal FinalReply — mirrors the
                // non-OAuth deny branch in `triggered_notification_for_state`.
                tracing::debug!(
                    target = "ironclaw::reborn::channel_delivery",
                    %run_id,
                    "triggered run OAuth URL suppressed by send-time backstop: resolved \
                     target is not a personal DM; cancelling run"
                );
                // Cancel the blocked run FIRST. Do NOT remove the existing auth
                // prompt until the run is actually canceled: a transient cancel
                // failure must leave the prompt in place (the user may still be able
                // to finish), so on failure we record `Failed` and return without
                // deleting anything.
                if let Err(err) = cancel_auth_blocked_run(
                    services.turn_coordinator.as_ref(),
                    services.auth_flow_canceller.as_deref(),
                    &scope,
                    actor.clone(),
                    run_id,
                    state.gate_ref.as_ref().map(|gate_ref| gate_ref.as_str()),
                )
                .await
                {
                    tracing::debug!(
                        target = "ironclaw::reborn::channel_delivery",
                        %run_id,
                        error = %err,
                        "triggered run OAuth backstop: cancel_auth_blocked_run failed"
                    );
                    let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
                    record_triggered_run_outcome(delivery_store, run_id, outcome).await?;
                    return Ok(outcome);
                }
                // Post the auth-unavailable notice as a terminal FinalReply.
                // require_direct_message_target is false: the notice is plain text
                // with no OAuth URL, so no DM restriction applies.
                let notice = ChannelActionableNotification {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    payload: ProductOutboundPayload::FinalReply(FinalReplyView {
                        turn_run_id: run_id,
                        text: format!(
                            "{CHANNEL_AUTH_UNAVAILABLE_MESSAGE}{}",
                            triggered_update_footer(&trigger_label)
                        ),
                        generated_at: Utc::now(),
                    }),
                    gate_ref_for_routing: None,
                };
                let outcome = match deliver_triggered_notification(
                    services, &scope, &actor, run_id, &state, &authority, notice, false,
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
                // The run is now canceled, so any OAuth auth-prompt messages posted
                // to a DM in earlier iterations are stale — remove them. This runs
                // only after a successful cancel and after the replacement notice has
                // been attempted, so we never strip the prompt while the run is still
                // live.
                for message in messages_to_delete_after_final.drain(..) {
                    delete_triggered_channel_message(services, message).await;
                }
                record_triggered_run_outcome(delivery_store, run_id, outcome).await?;
                return Ok(outcome);
            }
            Err(failure) => {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_delivery",
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
                record_triggered_run_outcome(delivery_store, run_id, outcome).await?;
                return Ok(outcome);
            }
        }
    }
}

/// Waits for the given run to reach an actionable state (Completed, BlockedApproval, BlockedAuth).
async fn wait_for_actionable_triggered(
    services: &FinalReplyDeliveryServices,
    scope: &TurnScope,
    run_id: TurnRunId,
    settings: &FinalReplyDeliverySettings,
    delivered_blocked_marker: &Option<BlockedActionableMarker>,
) -> Result<TurnRunState, FinalReplyDeliveryError> {
    let start = std::time::Instant::now();
    let mut poll_interval = settings.poll_interval;
    loop {
        let state = services
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
            && Some(&marker) != delivered_blocked_marker.as_ref()
        {
            return Ok(state);
        }
        if start.elapsed() >= settings.max_wait {
            return Err(FinalReplyDeliveryError::RunWaitTimedOut { run_id });
        }
        tokio::time::sleep(jittered_poll_interval(poll_interval, &run_id)).await;
        poll_interval = poll_interval.saturating_mul(2).min(MAX_RUN_POLL_INTERVAL);
    }
}

/// Footer for triggered **gate** prompts (approval / OAuth auth). The user can
/// act on this specific request in Slack, but cannot otherwise drive the run.
/// `label` is a short trigger identifier (truncated prompt); omitted when empty.
fn triggered_gate_footer(label: &str) -> String {
    let label = label.trim();
    let lead = if label.is_empty() {
        "From a triggered event.".to_string()
    } else {
        format!("From a triggered event: “{label}”.")
    };
    format!(
        "\n\n_{lead} You can respond to this request here — to otherwise interact \
         with this run, open the Ironclaw web app._"
    )
}

/// Footer for triggered **updates / final replies**. These are output only —
/// there is nothing to act on in Slack, so it points the user to the web app.
fn triggered_update_footer(label: &str) -> String {
    let label = label.trim();
    let lead = if label.is_empty() {
        "From a triggered event.".to_string()
    } else {
        format!("From a triggered event: “{label}”.")
    };
    format!(
        "\n\n_{lead} You can't interact with triggered events here — open the \
         Ironclaw web app to interact with this run._"
    )
}

/// Truncate a trigger prompt to a short single-line label for the footer.
fn triggered_label_from_prompt(prompt: &str) -> String {
    const MAX_LABEL_CHARS: usize = 60;
    let first_line = prompt.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= MAX_LABEL_CHARS {
        first_line.to_string()
    } else {
        let truncated: String = first_line.chars().take(MAX_LABEL_CHARS).collect();
        format!("{truncated}…")
    }
}

/// Builds the notification payload for a triggered run's actionable state.
///
/// ## Triggered Slack surface contract
///
/// A triggered run is **output-only over Slack, plus gate-resolution input** —
/// it is NOT a conversational channel. This function is the single place those
/// outputs are minted, and it only ever produces three of the nine
/// [`ProductOutboundPayload`] variants:
///
/// - `BlockedApproval` → `GatePrompt` (approve/deny)
/// - `BlockedAuth`     → `AuthPrompt` (OAuth link) or, for non-OAuth, a cancel +
///   `FinalReply` carrying the auth-unavailable notice
/// - `Completed`       → `FinalReply` (the result)
///
/// Anything else (`Running`, etc.) yields `None` — triggered Slack deliberately
/// does NOT stream `Progress` / `CapabilityActivity` / projection payloads; those
/// belong to the live WebUI channel.
///
/// On the inbound side the triggered run only consumes gate **resolutions**
/// (`ApprovalResolution` / `ScopedApprovalResolution` / `AuthResolution`), bridged
/// back into the trigger's scope via the delivered-gate-route fingerprint. A
/// free-text Slack reply parses as a `UserMessage` and starts a *separate* run in
/// the DM's own scope — it never reaches the triggered run. If you extend this
/// function, preserve that boundary: do not mint conversational/streaming payloads
/// here, and do not assume inbound free-text can address a triggered run.
async fn triggered_notification_for_state(
    services: &FinalReplyDeliveryServices,
    scope: &TurnScope,
    thread_scope: &ThreadScope,
    actor: &TurnActor,
    state: &TurnRunState,
    run_id: TurnRunId,
    trigger_label: &str,
) -> Result<Option<ChannelActionableNotification>, FinalReplyDeliveryError> {
    match state.status {
        TurnStatus::Completed => {
            // Read finalized assistant message.
            let Some(text) = services
                .thread_service
                .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                    scope: thread_scope.clone(),
                    thread_id: scope.thread_id.clone(),
                    turn_run_id: run_id.to_string(),
                })
                .await?
                .and_then(|m| m.content)
            else {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_delivery",
                    %run_id,
                    "completed triggered run has no finalized assistant message; skipping delivery"
                );
                return Ok(None);
            };
            Ok(Some(ChannelActionableNotification {
                event_kind: RunNotificationEventKind::FinalReplyReady,
                payload: ProductOutboundPayload::FinalReply(FinalReplyView {
                    turn_run_id: run_id,
                    text: format!("{text}{}", triggered_update_footer(trigger_label)),
                    generated_at: Utc::now(),
                }),
                gate_ref_for_routing: None,
            }))
        }
        TurnStatus::BlockedApproval => {
            let Some(gate_ref) = state.gate_ref.as_ref() else {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_delivery",
                    %run_id,
                    "triggered run blocked on approval without gate ref; skipping"
                );
                return Ok(None);
            };
            // Render the triggered approval prompt exactly like the regular
            // inbound flow: What/Why context (the tool + reason) via the shared
            // `channel_approval_gate_prompt_view`, with the channel-specific reply
            // instruction appended once by the adapter. The approval request is
            // stored under this triggered run's scope, so the context resolves
            // here.
            let context = approval_prompt_context_view(
                services.approval_requests.as_deref(),
                gate_ref,
                &actor.user_id,
                scope,
            )
            .await?;
            let mut prompt = channel_approval_gate_prompt_view(run_id, gate_ref, context.as_ref());
            prompt.body.push_str(&triggered_gate_footer(trigger_label));
            Ok(Some(ChannelActionableNotification {
                event_kind: RunNotificationEventKind::ApprovalNeeded,
                payload: ProductOutboundPayload::GatePrompt(prompt),
                gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
            }))
        }
        TurnStatus::BlockedAuth => {
            let Some(gate_ref) = state.gate_ref.as_ref() else {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_delivery",
                    %run_id,
                    "triggered run blocked on auth without gate ref; skipping"
                );
                return Ok(None);
            };
            let mut view = enrich_auth_prompt_view(
                AuthPromptView {
                    turn_run_id: run_id,
                    auth_request_ref: gate_ref.as_str().to_string(),
                    invocation_id: None,
                    headline: "Authentication required".to_string(),
                    body: "Authentication required to continue this automation.".to_string(),
                    challenge_kind: None,
                    provider: None,
                    account_label: None,
                    authorization_url: None,
                    expires_at: None,
                    connection: None,
                },
                &actor.user_id,
                scope,
                &state.credential_requirements,
                services.auth_challenges.as_deref(),
            )
            .await?;
            view.body.push_str(&triggered_gate_footer(trigger_label));
            // Only link-based OAuth is allowed over Slack. The `require_direct_message_target`
            // flag is set on the `ProductOutboundDeliveryRequest` when the payload carries
            // an `authorization_url`, and the resolver enforces the DM constraint at send
            // time — it returns `OutboundTargetNotDirectMessage` if the resolved binding is
            // not a personal DM, which `classify_delivery_error` maps to `OAuthTargetNotDm`,
            // causing `deliver_triggered_run` to cancel the run and post the auth-unavailable
            // notice. We do not need to pre-check the DM status here.
            if view.authorization_url.is_some() {
                Ok(Some(ChannelActionableNotification {
                    event_kind: RunNotificationEventKind::AuthRequired,
                    payload: ProductOutboundPayload::AuthPrompt(view),
                    gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
                }))
            } else {
                // Non-OAuth challenge (manual token / API-key entry). Deny: cancel the
                // parked run and post the auth-unavailable notice directly.
                cancel_auth_blocked_run(
                    services.turn_coordinator.as_ref(),
                    services.auth_flow_canceller.as_deref(),
                    scope,
                    actor.clone(),
                    run_id,
                    Some(gate_ref.as_str()),
                )
                .await?;
                Ok(Some(ChannelActionableNotification {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    payload: ProductOutboundPayload::FinalReply(FinalReplyView {
                        turn_run_id: run_id,
                        text: format!(
                            "{CHANNEL_AUTH_UNAVAILABLE_MESSAGE}{}",
                            triggered_update_footer(trigger_label)
                        ),
                        generated_at: Utc::now(),
                    }),
                    gate_ref_for_routing: None,
                }))
            }
        }
        _ => Ok(None),
    }
}

/// Typed failure classification for a single triggered-run notification delivery
/// attempt. Avoids string-contains pattern matching on error messages.
enum TriggeredNotificationFailure {
    /// The creator has no personal delivery target configured.
    NoDefaultConfigured,
    /// The resolved target is inaccessible or rejected the delivery.
    Denied,
    /// The payload carries an OAuth `authorization_url` but the send-time
    /// binding resolved to a non-personal-DM target. Posting the OAuth URL
    /// to a shared channel would leak it to every member. The resolver returns
    /// [`ProductWorkflowError::OutboundTargetNotDirectMessage`] when
    /// `require_direct_message_target` is true and the binding is not a DM;
    /// `classify_delivery_error` maps that to this variant. `deliver_triggered_run`
    /// handles it by cancelling the run and posting the auth-unavailable notice.
    OAuthTargetNotDm,
    /// Any other delivery or transport failure.
    Other(String),
}

impl std::fmt::Display for TriggeredNotificationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDefaultConfigured => write!(f, "no default delivery target configured"),
            Self::Denied => write!(f, "delivery target access denied"),
            Self::OAuthTargetNotDm => {
                write!(
                    f,
                    "OAuth authorization_url suppressed: send-time target is not a personal DM"
                )
            }
            Self::Other(reason) => write!(f, "{reason}"),
        }
    }
}

/// Delivers a triggered-run notification, returning the list of posted Slack messages.
// arch-exempt: too_many_args, needs a delivery-request bundle (services + scope + actor + state + authority + notification), plan #4953
#[allow(clippy::too_many_arguments)]
async fn deliver_triggered_notification(
    services: &FinalReplyDeliveryServices,
    scope: &TurnScope,
    actor: &TurnActor,
    run_id: TurnRunId,
    state: &TurnRunState,
    authority: &TriggeredChannelReplyTargetAuthority,
    notification: ChannelActionableNotification,
    require_direct_message_target: bool,
) -> Result<Vec<PostedChannelMessage>, TriggeredNotificationFailure> {
    let ChannelActionableNotification {
        event_kind,
        payload,
        // The caller extracts gate_ref_for_routing before this call and records
        // the delivered-gate route record on success; it is not needed here.
        gate_ref_for_routing: _,
    } = notification;

    let _reply_target = state.reply_target_binding_ref.clone();
    let projection_access_policy = AllowNoProjectionAccess;
    let outbound_policy = OutboundPolicyService::new(
        services.outbound_store.as_ref(),
        &projection_access_policy,
        authority,
    );
    let projection_id = channel_run_notification_projection_id(
        services.channel_protocol.as_ref(),
        run_id,
        event_kind,
    );
    let projection_ref = ProjectionUpdateRef::new(projection_id.clone()).map_err(|reason| {
        TriggeredNotificationFailure::Other(format!("invalid_projection_ref: {reason}"))
    })?;
    // A fire-resolved per-trigger route becomes the source route the engine
    // prefers for the final reply; without one, the creator's user-global
    // preference resolves as before.
    let origin = match &authority.per_trigger_source_route {
        Some(source_route) => RunNotificationOrigin::TriggeredFromSourceRoute {
            trigger: authority.trigger_context.clone(),
            source_route: SourceRouteContext {
                reply_target_binding_ref: source_route.clone(),
            },
        },
        None => RunNotificationOrigin::Triggered {
            trigger: authority.trigger_context.clone(),
        },
    };
    let delivery = ironclaw_outbound::PrepareCommunicationDeliveryRequest {
        resolution_request: CommunicationDeliveryResolutionRequest {
            scope: scope.clone(),
            actor: actor.clone(),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind,
                origin,
            }),
        },
        turn_run_id: Some(run_id),
        projection_ref,
        attempted_at: Utc::now(),
    };

    let tracked_egress = TrackingPostEgress::new(
        Arc::clone(&services.egress),
        Arc::clone(&services.channel_protocol),
    );
    let render_result = prepare_and_render_product_outbound(
        &outbound_policy,
        services.communication_preferences.as_ref(),
        authority,
        ProductOutboundDeliveryRequest {
            delivery,
            payload,
            projection_cursor: ironclaw_product_adapters::ProjectionCursor::new(projection_id)
                .map_err(|e| {
                    TriggeredNotificationFailure::Other(format!("invalid_projection_cursor: {e}"))
                })?,
            adapter: services.adapter.as_ref(),
            egress: &tracked_egress,
            delivery_sink: services.delivery_sink.as_ref(),
            require_direct_message_target,
        },
    )
    .await;

    if let Err(error) = render_result {
        return Err(classify_delivery_error(error));
    }

    let posted_messages = tracked_egress.take_posted_messages();
    if posted_messages.is_empty() {
        return Err(TriggeredNotificationFailure::Other(
            "delivery_evidence_missing".to_string(),
        ));
    }
    Ok(posted_messages)
}

/// Classify a [`ProductOutboundDeliveryError`] into the typed
/// [`TriggeredNotificationFailure`] variants used for outcome recording.
fn classify_delivery_error(
    error: ironclaw_product_workflow::ProductOutboundDeliveryError,
) -> TriggeredNotificationFailure {
    use ironclaw_outbound::OutboundError;
    use ironclaw_product_workflow::ProductOutboundDeliveryError;
    match &error {
        ProductOutboundDeliveryError::Workflow {
            source: ProductWorkflowError::OutboundTargetNotDirectMessage,
            ..
        } => TriggeredNotificationFailure::OAuthTargetNotDm,
        ProductOutboundDeliveryError::Outbound(OutboundError::PreferenceTargetMissing {
            ..
        }) => TriggeredNotificationFailure::NoDefaultConfigured,
        ProductOutboundDeliveryError::Outbound(OutboundError::AccessDenied) => {
            TriggeredNotificationFailure::Denied
        }
        _ => TriggeredNotificationFailure::Other(error.to_string()),
    }
}

async fn delete_triggered_channel_message(
    services: &FinalReplyDeliveryServices,
    message: PostedChannelMessage,
) {
    if let Err(error) = services
        .channel_protocol
        .delete_status_message(services.egress.as_ref(), &message)
        .await
    {
        tracing::warn!(
            target = "ironclaw::reborn::channel_delivery",
            error = %error,
            "failed to delete triggered delivery auth message"
        );
    }
}

async fn record_triggered_run_outcome(
    store: &dyn TriggeredRunDeliveryStore,
    run_id: TurnRunId,
    outcome: TriggeredRunDeliveryOutcomeKind,
) -> Result<(), String> {
    let record = TriggeredRunDeliveryRecord {
        run_id,
        outcome,
        recorded_at: Utc::now(),
    };
    store.record_triggered_run_delivery(record).await
}

/// Resolve a fire's per-trigger delivery target id into the reply-target
/// binding the resolution engine should prefer over the user preference.
///
/// Fail-closed contract: any missing provider, malformed id, foreign/stale
/// target (`Ok(None)`), or provider backend error yields a terminal outcome
/// instead of a route — delivery never guesses a substitute conversation.
async fn resolve_per_trigger_delivery_route(
    outbound_target_provider: Option<&dyn OutboundDeliveryTargetProvider>,
    fire: &TriggerFire,
    scope: &TurnScope,
    target: &ironclaw_triggers::TriggerDeliveryTargetId,
) -> Result<ReplyTargetBindingRef, TriggeredRunDeliveryOutcomeKind> {
    let Some(provider) = outbound_target_provider else {
        tracing::warn!(
            target = "ironclaw::reborn::channel_delivery",
            "per-trigger delivery target present but no outbound target provider is wired"
        );
        return Err(TriggeredRunDeliveryOutcomeKind::TargetUnavailable);
    };
    let target_id = ironclaw_product_workflow::RebornOutboundDeliveryTargetId::new(target.as_str())
        .map_err(|error| {
            tracing::warn!(
                target = "ironclaw::reborn::channel_delivery",
                %error,
                "per-trigger delivery target id failed outbound target id validation"
            );
            TriggeredRunDeliveryOutcomeKind::TargetUnavailable
        })?;
    // The caller for target ownership checks is the trigger creator in the
    // fire's scope — the same identity that selected the target at creation.
    let caller = ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
        scope.tenant_id.clone(),
        fire.creator_user_id.clone(),
        scope.agent_id.clone(),
        scope.project_id.clone(),
    );
    match provider
        .resolve_outbound_delivery_target(&caller, &target_id)
        .await
    {
        Ok(Some(entry)) => Ok(entry.reply_target_binding_ref),
        Ok(None) => {
            tracing::warn!(
                target = "ironclaw::reborn::channel_delivery",
                "per-trigger delivery target did not resolve for the trigger creator (stale, foreign, or disconnected)"
            );
            Err(TriggeredRunDeliveryOutcomeKind::TargetUnavailable)
        }
        Err(error) => {
            tracing::warn!(
                target = "ironclaw::reborn::channel_delivery",
                %error,
                "outbound delivery target lookup failed during triggered delivery"
            );
            Err(TriggeredRunDeliveryOutcomeKind::Failed)
        }
    }
}

/// Reply-target authority for triggered-run delivery.
///
/// Resolves the delivery target from the creator's personal communication
/// preference (via `CommunicationPreferenceRepository`). Validates that the
/// reply target is the one the resolution engine chose (no substitution).
pub(crate) struct TriggeredChannelReplyTargetAuthority {
    channel_protocol: Arc<dyn ChannelDeliveryProtocol>,
    scope: TurnScope,
    actor: TurnActor,
    trigger_context: TriggerCommunicationContext,
    /// Reply-target binding resolved from the trigger's own `delivery_target`
    /// at fire time. When present, delivery uses
    /// `RunNotificationOrigin::TriggeredFromSourceRoute` so the resolution
    /// engine prefers it over the creator's user-global preference.
    per_trigger_source_route: Option<ReplyTargetBindingRef>,
    /// Space id (Slack team id) captured during
    /// `resolve_product_outbound_target_metadata`. Updated on every resolution.
    /// Used after delivery to attach the team id to posted-message gate-route
    /// refs so inbound replies (which carry team_id as space_id)
    /// fingerprint-match the recorded ref.
    resolved_space_id: std::sync::Mutex<Option<String>>,
}

impl TriggeredChannelReplyTargetAuthority {
    pub(crate) fn from_fire(
        channel_protocol: Arc<dyn ChannelDeliveryProtocol>,
        scope: TurnScope,
        actor: TurnActor,
        fire: &TriggerFire,
        per_trigger_source_route: Option<ReplyTargetBindingRef>,
    ) -> Result<Self, String> {
        let trigger_origin_ref = TriggerOriginRef::new(fire.identity.trigger_id().to_string())
            .map_err(|error| format!("invalid trigger origin ref: {error}"))?;
        let fire_slot = TriggerFireSlot::new(fire.identity.fire_slot().to_rfc3339())
            .map_err(|error| format!("invalid fire slot: {error}"))?;
        Ok(Self {
            channel_protocol,
            scope,
            actor,
            trigger_context: TriggerCommunicationContext {
                trigger_origin_ref,
                trigger_source_kind: TriggerSourceKind::Schedule,
                fire_slot,
            },
            per_trigger_source_route,
            resolved_space_id: std::sync::Mutex::new(None),
        })
    }
}

#[async_trait]
impl ReplyTargetBindingValidator for TriggeredChannelReplyTargetAuthority {
    async fn validate_reply_target(
        &self,
        request: ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError> {
        // Scope and actor remain necessary, but a trigger-selected route is
        // also sealed authority: the resolution engine may not substitute a
        // different candidate from a later user-global preference.
        if request.scope != self.scope || request.actor != self.actor {
            return Err(OutboundError::AccessDenied);
        }
        if self
            .per_trigger_source_route
            .as_ref()
            .is_some_and(|sealed| sealed != &request.candidate.target)
        {
            return Err(OutboundError::AccessDenied);
        }
        Ok(ReplyTargetBindingClaim::new(request.candidate.target))
    }
}

#[async_trait]
impl ProductOutboundTargetResolver for TriggeredChannelReplyTargetAuthority {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ValidatedReplyTargetBinding,
        require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        // Single enforcement point for the OAuth DM rule: when the delivery request
        // requires a direct-message target (i.e. the payload carries an OAuth
        // authorization_url), enforce that the EXACT send-time binding is a personal
        // DM. Checked against the binding resolved NOW (at send time), making it
        // race-free against the pre-loop preference snapshot going stale.
        enforce_direct_message_if_required(
            self.channel_protocol.as_ref(),
            target.target(),
            require_direct_message,
        )?;

        // Decode the conversation from the binding ref. Slack refs were built
        // by `slack_personal_dm_reply_target_binding_ref` /
        // `slack_shared_channel_reply_target_binding_ref` and encode space +
        // conversation in length-prefixed segments; Telegram refs use the
        // adapter's `tg:` encoding. We extract only what we need
        // (conversation id + optional space/team id) to reconstruct the
        // `ExternalConversationRef` for the adapter.
        let (conversation_id, space_id) = self
            .channel_protocol
            .conversation_id_from_reply_target_binding_ref(target.target())
            .ok_or_else(|| ProductWorkflowError::BindingResolutionFailed {
            reason: format!(
                "triggered delivery: cannot extract a channel conversation from binding ref '{}'",
                target.target().as_str()
            ),
        })?;
        // Store the resolved space id so that, after deliver_triggered_notification
        // returns posted messages, we can attach the team id to gate-route refs.
        *self
            .resolved_space_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = space_id.clone();
        let external_conversation_ref =
            ExternalConversationRef::new(space_id.as_deref(), &conversation_id, None, None)
                .map_err(|e| ProductWorkflowError::BindingResolutionFailed {
                    reason: format!("triggered delivery conversation ref: {e}"),
                })?;
        Ok(VerifiedProductOutboundTargetMetadata {
            external_conversation_ref,
            external_actor_ref: None,
        })
    }
}

pub(crate) fn turn_scope_from_thread_scope(
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

pub(crate) fn thread_scope_from_binding(
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

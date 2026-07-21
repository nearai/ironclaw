use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::Utc;
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    OutboundPolicyService, ProjectionUpdateRef, RunNotificationContext, RunNotificationEventKind,
    RunNotificationOrigin, SourceRouteContext,
};
use ironclaw_product_adapters::{
    AuthPromptView, FinalReplyView, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductOutboundPayload, ProductRejectionKind,
};
use ironclaw_product_workflow::{
    ConversationBindingService, ProductOutboundDeliveryRequest, ResolveBindingRequest,
    ResolvedBinding, approval_prompt_context_view, enrich_auth_prompt_view,
    prepare_and_render_product_outbound,
};
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, SessionThreadService, ThreadScope};
use ironclaw_turns::{
    GetRunStateRequest, TurnActor, TurnCoordinator, TurnErrorCategory, TurnRunId, TurnRunState,
    TurnScope, TurnStatus,
};
use tokio::sync::Semaphore;

use crate::actionable::{
    AllowNoProjectionAccess, ObservedChannelReplyTargetAuthority, blocked_actionable_marker,
    cancel_auth_blocked_run, channel_approval_gate_prompt_view, channel_auth_prompt_view,
    channel_run_notification_projection_id, is_accepted_auth_denial, jittered_poll_interval,
    rejection_hint_for_resolution, should_deliver_after_ack, submitted_run_id,
};
use crate::routing::{
    TrackingPostEgress, conversations_ref_from_product_ref, record_gate_route_if_needed,
};
use crate::services::*;
use crate::triggered::{thread_scope_from_binding, turn_scope_from_thread_scope};

pub struct FinalReplyDeliveryObserver {
    pub(crate) services: FinalReplyDeliveryServices,
    pub(crate) settings: FinalReplyDeliverySettings,
    pub(crate) delivery_permits: Arc<Semaphore>,
    /// Per-observer throttle: at most one busy-thread hint per
    /// (conversation fingerprint, external_event_id) pair.
    /// Transport retries of the same Slack event share the same external_event_id, so
    /// they are deduplicated here. Each distinct new human message gets a fresh hint
    /// even if the same blocking run is still active.
    /// Bounded FIFO eviction keeps memory O(1); a false-negative after eviction just
    /// means one extra hint, harmless.
    pub(crate) hint_seen: HintSeenSet,
    /// Single-flight guard: at most one live `deliver_final_reply` loop per run_id.
    ///
    /// A gate-resolution ack (`ApprovalResolution(Allow)` / `AuthResolution(Allowed)`)
    /// carries the same `submitted_run_id` as the original user-message ack because it
    /// resumes the pre-existing run rather than creating a new one. Without this guard,
    /// each resolution ack would spawn a second delivery loop for the same run while the
    /// original loop is still watching — N resolutions ⇒ N+1 concurrent loops ⇒ gate N
    /// posted N times. The original loop detects the unblock and posts the next gate
    /// exactly once, so resolution-ack loops are always redundant duplicates.
    pub(crate) active_delivery_run_ids: Mutex<HashSet<TurnRunId>>,
}

/// RAII guard that removes a `run_id` from `active_delivery_run_ids` on drop.
///
/// Acquired before the delivery semaphore permit so that a concurrent ack for
/// the same run_id is rejected immediately — without competing for a permit and
/// without the TOCTOU window that existed when the permit was acquired first.
///
/// Panic-safe: `Drop` uses `unwrap_or_else(|e| e.into_inner())` to tolerate a
/// poisoned mutex, so the run_id is always removed even if `deliver_final_reply`
/// panics.
pub(crate) struct RunDeliveryGuard<'a> {
    pub(crate) set: &'a Mutex<HashSet<TurnRunId>>,
    pub(crate) run_id: TurnRunId,
}

impl Drop for RunDeliveryGuard<'_> {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.run_id);
    }
}

impl FinalReplyDeliveryObserver {
    pub fn new(services: FinalReplyDeliveryServices) -> Self {
        Self::with_settings(services, FinalReplyDeliverySettings::default())
    }

    pub fn with_settings(
        services: FinalReplyDeliveryServices,
        settings: FinalReplyDeliverySettings,
    ) -> Self {
        Self {
            services,
            settings,
            delivery_permits: Arc::new(Semaphore::new(settings.max_concurrent_deliveries.get())),
            hint_seen: Mutex::new((VecDeque::new(), HashSet::new())),
            active_delivery_run_ids: Mutex::new(HashSet::new()),
        }
    }

    pub(crate) async fn deliver_final_reply(
        &self,
        envelope: ProductInboundEnvelope,
        ack: ProductInboundAck,
    ) -> Result<(), FinalReplyDeliveryError> {
        if is_accepted_auth_denial(&envelope, &ack) {
            self.services
                .channel_protocol
                .post_status_message(
                    self.services.egress.as_ref(),
                    envelope.external_conversation_ref(),
                    CHANNEL_AUTH_CANCELED_MESSAGE,
                )
                .await?;
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
        // trigger's own scope, not this Slack conversation's scope. That run is
        // delivered by its own triggered-delivery loop (`deliver_triggered_run`),
        // so the live observer must not also poll it here under the conversation
        // scope — the run isn't found there, which would otherwise surface as a
        // spurious "something went wrong" delivery error. Skip cleanly and let
        // the triggered loop own continuation, matching the regular inbound flow.
        //
        // The skip only applies to bridged gate/auth resolution payloads. A
        // normal UserMessage (or other non-resolution payload) must never be
        // silently dropped here — surface the error instead.
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
                    target = "ironclaw::reborn::channel_delivery",
                    %run_id,
                    "skipping live Slack delivery: run is not in this conversation scope (triggered/foreign run); its own delivery loop owns continuation"
                );
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        }
        let mut delivered_blocked_marker = None;
        let mut working_message = None;
        let mut messages_to_delete_after_final = Vec::new();
        loop {
            let actionable_state = self
                .wait_for_actionable(
                    &scope,
                    run_id,
                    delivered_blocked_marker.as_ref(),
                    &envelope,
                    &mut working_message,
                )
                .await
                .map_err(|err| {
                    // If we already delivered a blocked-state notification
                    // (approval/auth prompt), a timeout does not leave the user
                    // in silence — convert to the quieter variant so A3 does
                    // not double-post.
                    if matches!(err, FinalReplyDeliveryError::RunWaitTimedOut { .. })
                        && delivered_blocked_marker.is_some()
                    {
                        FinalReplyDeliveryError::RunWaitTimedOutAfterNotification { run_id }
                    } else {
                        err
                    }
                })?;
            if matches!(
                actionable_state.status,
                TurnStatus::BlockedApproval | TurnStatus::BlockedAuth
            ) {
                self.delete_status_message_if_present(working_message.take())
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
            let posted_messages = self
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
                // Derive the space id from the envelope's conversation ref so that
                // posted-message refs carry the Slack team id (space_id). Inbound
                // events set space_id = team_id, so without this the fingerprints
                // would differ and a reply in the prompt thread would not match.
                let envelope_space_id =
                    conversations_ref_from_product_ref(envelope.external_conversation_ref())
                        .ok()
                        .and_then(|r| r.space_id().map(str::to_string));
                record_gate_route_if_needed(
                    self.services.route_store.as_ref(),
                    run_id,
                    &scope.tenant_id,
                    &binding.actor_user_id,
                    gate_ref_str,
                    &scope,
                    &posted_messages,
                    Some(envelope.external_conversation_ref()),
                    envelope_space_id.as_deref(),
                )
                .await;
            }

            let Some(marker) = next_blocked_marker else {
                self.delete_status_message_if_present(working_message.take())
                    .await;
                for message in messages_to_delete_after_final {
                    self.delete_posted_status_message(message).await;
                }
                return Ok(());
            };
            if posted_messages.is_empty() {
                // The marker is still recorded below so the wait loop doesn't
                // hot-loop re-rendering the same prompt, but a blocked-state
                // notification that posted nothing means the user was NOT
                // told why the run stopped — exactly how the Telegram
                // AuthPrompt defer stub turned an auth gate into silence
                // (2026-07-17). Keep this loud.
                tracing::warn!(
                    target = "ironclaw::reborn::channel_delivery",
                    %run_id,
                    ?event_kind,
                    "blocked-state notification rendered no channel messages; the user was not notified (adapter deferred or outbound policy suppressed the prompt)"
                );
            }
            if event_kind == RunNotificationEventKind::AuthRequired {
                messages_to_delete_after_final.extend(posted_messages);
            }
            delivered_blocked_marker = Some(marker);
        }
    }

    async fn notification_for_actionable_state(
        &self,
        envelope: &ProductInboundEnvelope,
        binding: &ResolvedBinding,
        thread_scope: &ThreadScope,
        scope: &TurnScope,
        run_id: TurnRunId,
        state: &TurnRunState,
    ) -> Result<Option<ChannelActionableNotification>, FinalReplyDeliveryError> {
        let notification = match state.status {
            TurnStatus::Completed => {
                let Some(text) = self
                    .read_latest_assistant_text(thread_scope, binding, run_id)
                    .await?
                else {
                    tracing::warn!(
                        %run_id,
                        "completed channel run has no finalized assistant message; skipping final reply delivery"
                    );
                    return Ok(None);
                };
                ChannelActionableNotification {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    payload: ProductOutboundPayload::FinalReply(FinalReplyView {
                        turn_run_id: run_id,
                        text,
                        generated_at: Utc::now(),
                    }),
                    gate_ref_for_routing: None,
                }
            }
            TurnStatus::BlockedApproval => {
                let Some(gate_ref) = state.gate_ref.as_ref() else {
                    tracing::warn!(
                        %run_id,
                        "channel run is blocked on approval without a gate ref; skipping approval prompt delivery"
                    );
                    return Ok(None);
                };
                // Look up WHAT is being approved from the ApprovalRequestStore by
                // gate ref — the same source the WebUI gate projection uses — so
                // the prompt names the capability/reason instead of a generic step.
                let approval_context = approval_prompt_context_view(
                    self.services.approval_requests.as_deref(),
                    gate_ref,
                    &binding.actor_user_id,
                    scope,
                )
                .await?;
                ChannelActionableNotification {
                    event_kind: RunNotificationEventKind::ApprovalNeeded,
                    payload: ProductOutboundPayload::GatePrompt(channel_approval_gate_prompt_view(
                        run_id,
                        gate_ref,
                        approval_context.as_ref(),
                    )),
                    gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
                }
            }
            TurnStatus::BlockedAuth => {
                let Some(gate_ref) = state.gate_ref.as_ref() else {
                    tracing::warn!(
                        %run_id,
                        "channel run is blocked on auth without a gate ref; skipping auth handling"
                    );
                    return Ok(None);
                };
                let view = enrich_auth_prompt_view(
                    AuthPromptView {
                        turn_run_id: run_id,
                        auth_request_ref: gate_ref.as_str().to_string(),
                        invocation_id: None,
                        headline: "Authentication required".to_string(),
                        body: "Authenticate to continue this run.".to_string(),
                        challenge_kind: None,
                        provider: None,
                        account_label: None,
                        authorization_url: None,
                        expires_at: None,
                        connection: None,
                    },
                    &binding.actor_user_id,
                    scope,
                    &state.credential_requirements,
                    self.services.auth_challenges.as_deref(),
                )
                .await?;
                // Only link-based OAuth is allowed over Slack: the user
                // authenticates on the provider's site via `authorization_url` and
                // the callback stores the credential server-side — nothing secret
                // is entered into the chat surface. Any other challenge (manual
                // token / API-key entry, etc.) would have the user paste a
                // credential into Slack, so deny it: cancel the run (same outcome
                // as `auth deny`) and redirect them to the web app.
                if view.authorization_url.is_some() {
                    ChannelActionableNotification {
                        event_kind: RunNotificationEventKind::AuthRequired,
                        payload: ProductOutboundPayload::AuthPrompt(channel_auth_prompt_view(
                            envelope, view,
                        )),
                        gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
                    }
                } else {
                    // Deny: cancel the parked run (a backend `cancel_run`, same
                    // outcome as `auth deny`) and post the denial directly. We
                    // post directly — like the busy-thread hint — rather than as a
                    // RunNotification FinalReply, because the outbound-policy /
                    // communication-preference machinery is for agent replies, not
                    // system notices, and gates the synthetic reply. Terminal: no
                    // notification, so the delivery loop ends here.
                    self.cancel_channel_auth_blocked_run(
                        scope,
                        TurnActor::new(binding.actor_user_id.clone()),
                        run_id,
                        gate_ref.as_str(),
                    )
                    .await?;
                    if let Err(error) = self
                        .services
                        .channel_protocol
                        .post_status_message(
                            self.services.egress.as_ref(),
                            envelope.external_conversation_ref(),
                            CHANNEL_AUTH_UNAVAILABLE_MESSAGE,
                        )
                        .await
                    {
                        tracing::debug!(
                            target = "ironclaw::reborn::channel_delivery",
                            %error,
                            "failed to post Slack auth-unavailable notice (best-effort)"
                        );
                    }
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(notification))
    }

    /// Auto-deny a Slack run that blocked on interactive auth (disabled on this
    /// channel). Thin wrapper over the shared [`cancel_auth_blocked_run`] so the
    /// live observer and the triggered delivery path cancel identically.
    async fn cancel_channel_auth_blocked_run(
        &self,
        scope: &TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        gate_ref: &str,
    ) -> Result<(), FinalReplyDeliveryError> {
        cancel_auth_blocked_run(
            self.services.turn_coordinator.as_ref(),
            self.services.auth_flow_canceller.as_deref(),
            scope,
            actor,
            run_id,
            Some(gate_ref),
        )
        .await
    }

    async fn deliver_run_notification(
        &self,
        envelope: &ProductInboundEnvelope,
        scope: &TurnScope,
        actor: &TurnActor,
        run_id: TurnRunId,
        state: &TurnRunState,
        notification: ChannelActionableNotification,
    ) -> Result<Vec<PostedChannelMessage>, FinalReplyDeliveryError> {
        let ChannelActionableNotification {
            event_kind,
            payload,
            gate_ref_for_routing: _,
        } = notification;
        let reply_target = state.reply_target_binding_ref.clone();
        let target_authority = ObservedChannelReplyTargetAuthority {
            channel_protocol: Arc::clone(&self.services.channel_protocol),
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
        let projection_id = channel_run_notification_projection_id(
            self.services.channel_protocol.as_ref(),
            run_id,
            event_kind,
        );
        let projection_ref = ProjectionUpdateRef::new(projection_id.clone())
            .map_err(|reason| FinalReplyDeliveryError::InvalidProjectionRef { reason })?;
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
        let tracked_egress = TrackingPostEgress::new(
            self.services.egress.clone(),
            Arc::clone(&self.services.channel_protocol),
        );
        let _outcome = prepare_and_render_product_outbound(
            &outbound_policy,
            self.services.communication_preferences.as_ref(),
            &target_authority,
            ProductOutboundDeliveryRequest {
                delivery,
                payload,
                projection_cursor: ironclaw_product_adapters::ProjectionCursor::new(projection_id)
                    .map_err(|error| FinalReplyDeliveryError::InvalidProjectionRef {
                        reason: error.to_string(),
                    })?,
                adapter: self.services.adapter.as_ref(),
                egress: &tracked_egress,
                delivery_sink: self.services.delivery_sink.as_ref(),
                require_direct_message_target: false,
            },
        )
        .await?;
        let posted_messages = tracked_egress.take_posted_messages();
        if posted_messages.is_empty() {
            return Err(FinalReplyDeliveryError::DeliveryEvidenceMissing { run_id });
        }
        Ok(posted_messages)
    }

    async fn wait_for_actionable(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        delivered_blocked_marker: Option<&BlockedActionableMarker>,
        envelope: &ProductInboundEnvelope,
        working_message: &mut Option<PostedChannelMessage>,
    ) -> Result<TurnRunState, FinalReplyDeliveryError> {
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
            if let Some(marker) = blocked_actionable_marker(&state)
                && Some(&marker) != delivered_blocked_marker
            {
                return Ok(state);
            }
            if start.elapsed() >= self.settings.max_wait {
                return Err(FinalReplyDeliveryError::RunWaitTimedOut { run_id });
            }
            if working_message.is_none() && blocked_actionable_marker(&state).is_none() {
                *working_message = self.post_working_message(envelope).await;
            }
            tokio::time::sleep(jittered_poll_interval(poll_interval, &run_id)).await;
            poll_interval = poll_interval.saturating_mul(2).min(MAX_RUN_POLL_INTERVAL);
        }
    }

    async fn read_latest_assistant_text(
        &self,
        thread_scope: &ThreadScope,
        binding: &ResolvedBinding,
        run_id: TurnRunId,
    ) -> Result<Option<String>, FinalReplyDeliveryError> {
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

    async fn post_working_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Option<PostedChannelMessage> {
        match self
            .services
            .channel_protocol
            .post_status_message(
                self.services.egress.as_ref(),
                envelope.external_conversation_ref(),
                CHANNEL_WORKING_MESSAGE,
            )
            .await
        {
            Ok(message) => Some(message),
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::reborn::channel_delivery",
                    error = %error,
                    "failed to post Slack working indicator"
                );
                None
            }
        }
    }

    async fn delete_status_message_if_present(&self, message: Option<PostedChannelMessage>) {
        if let Some(message) = message {
            self.delete_posted_status_message(message).await;
        }
    }

    async fn delete_posted_status_message(&self, message: PostedChannelMessage) {
        if let Err(error) = self
            .services
            .channel_protocol
            .delete_status_message(self.services.egress.as_ref(), &message)
            .await
        {
            tracing::warn!(
                target = "ironclaw::reborn::channel_delivery",
                error = %error,
                "failed to delete Slack prompt/status message"
            );
        }
    }

    pub(crate) async fn post_rejection_hint_if_authorized(
        &self,
        envelope: &ProductInboundEnvelope,
        ack: &ProductInboundAck,
    ) -> bool {
        let Some(hint) = rejection_hint_for_resolution(envelope, ack) else {
            return false;
        };
        if let Err(error) = self
            .services
            .binding_service
            .lookup_binding(ResolveBindingRequest::from_envelope(envelope))
            .await
        {
            tracing::debug!(
                target = "ironclaw::reborn::channel_delivery",
                error = %error,
                "skipped Slack rejection hint because the originating conversation was not authorized"
            );
            return true;
        }
        if let Err(error) = self
            .services
            .channel_protocol
            .post_status_message(
                self.services.egress.as_ref(),
                envelope.external_conversation_ref(),
                hint,
            )
            .await
        {
            tracing::debug!(
                target = "ironclaw::reborn::channel_delivery",
                error = %error,
                "failed to post rejection hint to Slack (best-effort)"
            );
        }
        true
    }

    /// Model B: a first-contact DM from a Slack user with no identity binding is
    /// rejected with `BindingRequired`. Instead of silently dropping it, greet
    /// them with a connect nudge — but ONLY in a 1:1 DM. An unbound user's
    /// app-mention in a shared channel also rejects with `BindingRequired`, and
    /// the host nudge must never be posted into a shared channel where everyone
    /// sees a message addressed to one person, so this gates on the DM channel
    /// id prefix ('D') and skips shared/group conversations. Deliberately
    /// performs NO binding lookup (the sender is unbound by definition) and
    /// posts only a fixed, host-authored message — no agent turn runs, no tools
    /// execute, no data is read. Slack transport retries arrive as `Duplicate`
    /// (not `Rejected`), so this fires at most once per inbound event.
    pub(crate) async fn post_connect_nudge_if_unbound_user_message(
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
        if !matches!(envelope.payload(), ProductInboundPayload::UserMessage(_)) {
            return false;
        }
        // Only nudge in a 1:1 DM. An unbound user's app-mention in a SHARED
        // channel also rejects with `BindingRequired`; posting the host connect
        // nudge there would drop a message addressed to one user into a channel
        // everyone can see. Slack DM (im) channel ids start with 'D'; shared
        // channels ('C') and multi-person/group DMs ('G') are excluded, and a
        // missing/blank conversation ref fails closed (no post).
        if !self
            .services
            .channel_protocol
            .is_direct_message_conversation(envelope.external_conversation_ref().conversation_id())
        {
            return false;
        }
        if let Err(error) = self
            .services
            .channel_protocol
            .post_status_message(
                self.services.egress.as_ref(),
                envelope.external_conversation_ref(),
                self.services.channel_protocol.connect_nudge_message(),
            )
            .await
        {
            tracing::debug!(
                target = "ironclaw::reborn::channel_delivery",
                error = %error,
                "failed to post Slack connect nudge (best-effort)"
            );
        }
        true
    }
}

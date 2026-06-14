//! Slack final-reply delivery for immediate-ACK Reborn webhooks.
//!
//! Slack Events API requires the HTTP handler to return 2xx quickly. This
//! observer runs after the workflow accepts an inbound Slack message, waits for
//! the submitted run to finish, reads the finalized assistant reply, and sends it
//! through the host-mediated product outbound delivery seam.
// arch-exempt: large_file, deferred-busy hint logic stays here to share observer/test fixtures with final-reply delivery; decomposition tracked in #4818.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    CommunicationPreferenceRepository, DeliveredGateRouteRecord, DeliveredGateRouteStore,
    OutboundError, OutboundPolicyService, OutboundStateStore, ProjectionUpdateRef,
    ReplyTargetBindingClaim, ReplyTargetBindingValidator, ReplyTargetValidationRequest,
    RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin, SourceRouteContext,
    TriggerCommunicationContext, TriggerFireSlot, TriggerOriginRef, TriggerSourceKind,
    TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryRecord, TriggeredRunDeliveryStore,
    ValidatedReplyTargetBinding,
};
use ironclaw_product_adapters::{
    DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath,
    EgressRequest, EgressResponse, ExternalActorRef, ExternalConversationRef, FinalReplyView,
    GatePromptView, OutboundDeliverySink, ProductAdapter, ProductAdapterError, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductOutboundPayload, ProductRejection,
    ProductRejectionKind, ProductTriggerReason, ProductWorkflowRejectionKind, ProtocolHttpEgress,
    ProtocolHttpEgressError,
};
use ironclaw_product_workflow::{
    ConversationBindingService, ProductOutboundDeliveryRequest, ProductOutboundTargetResolver,
    ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
    VerifiedProductOutboundTargetMetadata, is_approval_gate_ref,
    prepare_and_render_product_outbound,
};
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, SessionThreadService, ThreadScope};
use ironclaw_triggers::TriggerFire;
use ironclaw_turns::{
    GateRef, GetRunStateRequest, ReplyTargetBindingRef, TurnActor, TurnCoordinator, TurnRunId,
    TurnRunState, TurnScope, TurnStatus,
};
use ironclaw_wasm_product_adapters::ImmediateAckWorkflowObserver;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use tokio::sync::Semaphore;

use crate::AuthChallengeProvider;
use crate::auth_prompt::auth_prompt_view_for_blocked_auth;
use crate::slack_outbound_targets::slack_conversation_id_from_reply_target_binding_ref;

const MAX_SLACK_RUN_POLL_INTERVAL: Duration = Duration::from_secs(5);
const SLACK_RUN_POLL_JITTER_BUCKETS: u32 = 5;
const SLACK_API_HOST: &str = "slack.com";
const SLACK_BOT_TOKEN_HANDLE: &str = "slack_bot_token";
const SLACK_WORKING_MESSAGE: &str = "Ironclaw is thinking...";
const SLACK_AUTH_CANCELED_MESSAGE: &str = "Authentication canceled.";
const SLACK_DELIVERY_TIMEOUT_MESSAGE: &str =
    "This is taking longer than expected — check the WebUI for the result.";
const SLACK_DELIVERY_ERROR_MESSAGE: &str =
    "Something went wrong delivering the result here. Check the WebUI.";
/// Posted when the blocking run is `BlockedApproval` and no gate_ref is available.
const SLACK_DEFERRED_BUSY_APPROVAL_MESSAGE: &str = "Ironclaw is waiting on a pending approval before taking new messages — reply `approve` or `deny` (or `approve gate:<ref>`) to resume.";
/// Posted for any other non-terminal blocking state, or when the state lookup fails.
///
/// Honest copy: no queue yet — deferred-message drain ships in a separate PR (#4812).
/// Revisit this wording once #4812 lands and automatic retry is available.
const SLACK_DEFERRED_BUSY_GENERIC_MESSAGE: &str = "Ironclaw is still working on a previous message and can't take this one yet — please resend it once the current task finishes.";

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockedActionableMarker {
    status: TurnStatus,
    gate_ref: Option<String>,
}

struct SlackActionableNotification {
    event_kind: RunNotificationEventKind,
    payload: ProductOutboundPayload,
    /// Gate ref for approval prompts on triggered runs; consumed by the
    /// delivered-gate route record so a DM reply can resolve the gate on the
    /// triggered run's thread. `None` for live-run notifications (same-thread
    /// replies need no routing) and non-approval and non-auth kinds.
    gate_ref_for_routing: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlackFinalReplyDeliverySettings {
    pub poll_interval: Duration,
    pub max_wait: Duration,
    pub max_concurrent_deliveries: NonZeroUsize,
    /// Bounds the total number of spawned delivery tasks (active + waiting for a
    /// delivery permit). When this limit is reached, new trigger fires are
    /// recorded as `Skipped` rather than spawning an unbounded waiting task.
    pub max_pending_deliveries: NonZeroUsize,
}

impl Default for SlackFinalReplyDeliverySettings {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(250),
            max_wait: Duration::from_secs(120),
            max_concurrent_deliveries: NonZeroUsize::new(64).expect("non-zero literal"), // safety: static default literal is non-zero.
            max_pending_deliveries: NonZeroUsize::new(256).expect("non-zero literal"), // safety: static default literal is non-zero.
        }
    }
}

pub struct SlackFinalReplyDeliveryServices {
    pub binding_service: Arc<dyn ConversationBindingService>,
    pub thread_service: Arc<dyn SessionThreadService>,
    pub turn_coordinator: Arc<dyn TurnCoordinator>,
    pub outbound_store: Arc<dyn OutboundStateStore>,
    pub route_store: Arc<dyn DeliveredGateRouteStore>,
    pub communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    pub adapter: Arc<dyn ProductAdapter>,
    pub egress: Arc<dyn ProtocolHttpEgress>,
    pub delivery_sink: Arc<dyn OutboundDeliverySink>,
    pub auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
}

/// Maximum number of (conversation, run_id) pairs remembered for hint dedup.
/// FIFO eviction beyond this cap keeps memory O(1); a false-negative after
/// eviction just means one extra hint, which is harmless.
const HINT_SEEN_CAP: usize = 256;

type HintSeenKey = (String, TurnRunId);
type HintSeenSet = Mutex<(VecDeque<HintSeenKey>, HashSet<HintSeenKey>)>;

pub struct SlackFinalReplyDeliveryObserver {
    services: SlackFinalReplyDeliveryServices,
    settings: SlackFinalReplyDeliverySettings,
    delivery_permits: Arc<Semaphore>,
    /// Per-observer throttle: at most one deferred-busy hint per
    /// (conversation fingerprint, active_run_id) pair.
    /// Caps Slack API usage per blocked run; bounded FIFO eviction keeps memory O(1);
    /// a false-negative after eviction just means one extra hint, harmless.
    hint_seen: HintSeenSet,
    /// Single-flight guard: at most one live `deliver_final_reply` loop per run_id.
    ///
    /// A gate-resolution ack (`ApprovalResolution(Allow)` / `AuthResolution(Allowed)`)
    /// carries the same `submitted_run_id` as the original user-message ack because it
    /// resumes the pre-existing run rather than creating a new one. Without this guard,
    /// each resolution ack would spawn a second delivery loop for the same run while the
    /// original loop is still watching — N resolutions ⇒ N+1 concurrent loops ⇒ gate N
    /// posted N times. The original loop detects the unblock and posts the next gate
    /// exactly once, so resolution-ack loops are always redundant duplicates.
    active_delivery_run_ids: Mutex<HashSet<TurnRunId>>,
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
struct RunDeliveryGuard<'a> {
    set: &'a Mutex<HashSet<TurnRunId>>,
    run_id: TurnRunId,
}

impl Drop for RunDeliveryGuard<'_> {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.run_id);
    }
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
            hint_seen: Mutex::new((VecDeque::new(), HashSet::new())),
            active_delivery_run_ids: Mutex::new(HashSet::new()),
        }
    }

    async fn deliver_final_reply(
        &self,
        envelope: ProductInboundEnvelope,
        ack: ProductInboundAck,
    ) -> Result<(), SlackFinalReplyDeliveryError> {
        if is_accepted_auth_denial(&envelope, &ack) {
            post_slack_message(
                self.services.egress.as_ref(),
                envelope.external_conversation_ref(),
                SLACK_AUTH_CANCELED_MESSAGE,
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
                    if matches!(err, SlackFinalReplyDeliveryError::RunWaitTimedOut { .. })
                        && delivered_blocked_marker.is_some()
                    {
                        SlackFinalReplyDeliveryError::RunWaitTimedOutAfterNotification { run_id }
                    } else {
                        err
                    }
                })?;
            if matches!(
                actionable_state.status,
                TurnStatus::BlockedApproval | TurnStatus::BlockedAuth
            ) {
                self.delete_slack_message_if_present(working_message.take())
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
                self.delete_slack_message_if_present(working_message.take())
                    .await;
                for message in messages_to_delete_after_final {
                    self.delete_slack_message(message).await;
                }
                return Ok(());
            };
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
    ) -> Result<Option<SlackActionableNotification>, SlackFinalReplyDeliveryError> {
        let notification = match state.status {
            TurnStatus::Completed => {
                let Some(text) = self
                    .read_latest_assistant_text(thread_scope, binding, run_id)
                    .await?
                else {
                    tracing::warn!(
                        %run_id,
                        "completed Slack run has no finalized assistant message; skipping final reply delivery"
                    );
                    return Ok(None);
                };
                SlackActionableNotification {
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
                        "Slack run is blocked on approval without a gate ref; skipping approval prompt delivery"
                    );
                    return Ok(None);
                };
                SlackActionableNotification {
                    event_kind: RunNotificationEventKind::ApprovalNeeded,
                    payload: ProductOutboundPayload::GatePrompt(slack_approval_gate_prompt_view(
                        run_id, gate_ref,
                    )),
                    gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
                }
            }
            TurnStatus::BlockedAuth => {
                let Some(gate_ref) = state.gate_ref.as_ref() else {
                    tracing::warn!(
                        %run_id,
                        "Slack run is blocked on auth without a gate ref; skipping auth prompt delivery"
                    );
                    return Ok(None);
                };
                let view = slack_auth_prompt_view(
                    envelope,
                    auth_prompt_view_for_blocked_auth(
                        &binding.actor_user_id,
                        scope,
                        run_id,
                        gate_ref.as_str(),
                        "Authenticate to continue this run.".to_string(),
                        &state.credential_requirements,
                        self.services.auth_challenges.as_deref(),
                    )
                    .await?,
                );
                SlackActionableNotification {
                    event_kind: RunNotificationEventKind::AuthRequired,
                    payload: ProductOutboundPayload::AuthPrompt(view),
                    gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
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
        notification: SlackActionableNotification,
    ) -> Result<Vec<PostedSlackMessage>, SlackFinalReplyDeliveryError> {
        let SlackActionableNotification {
            event_kind,
            payload,
            gate_ref_for_routing: _,
        } = notification;
        let reply_target = state.reply_target_binding_ref.clone();
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
        let projection_id = slack_run_notification_projection_id(run_id, event_kind);
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
        let tracked_egress = TrackingSlackPostEgress::new(self.services.egress.clone());
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
                egress: &tracked_egress,
                delivery_sink: self.services.delivery_sink.as_ref(),
            },
        )
        .await?;
        Ok(tracked_egress.take_posted_messages())
    }

    async fn wait_for_actionable(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        delivered_blocked_marker: Option<&BlockedActionableMarker>,
        envelope: &ProductInboundEnvelope,
        working_message: &mut Option<PostedSlackMessage>,
    ) -> Result<TurnRunState, SlackFinalReplyDeliveryError> {
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
                return Err(SlackFinalReplyDeliveryError::RunWaitTimedOut { run_id });
            }
            if working_message.is_none() && blocked_actionable_marker(&state).is_none() {
                *working_message = self.post_slack_working_message(envelope).await;
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

    async fn post_slack_working_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Option<PostedSlackMessage> {
        match post_slack_message(
            self.services.egress.as_ref(),
            envelope.external_conversation_ref(),
            SLACK_WORKING_MESSAGE,
        )
        .await
        {
            Ok(message) => Some(message),
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::reborn::slack_delivery",
                    error = %error,
                    "failed to post Slack working indicator"
                );
                None
            }
        }
    }

    async fn delete_slack_message_if_present(&self, message: Option<PostedSlackMessage>) {
        if let Some(message) = message {
            self.delete_slack_message(message).await;
        }
    }

    async fn delete_slack_message(&self, message: PostedSlackMessage) {
        if let Err(error) = delete_slack_message(self.services.egress.as_ref(), &message).await {
            tracing::warn!(
                target = "ironclaw::reborn::slack_delivery",
                error = %error,
                "failed to delete Slack prompt/status message"
            );
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
        if let Err(error) = self
            .services
            .binding_service
            .lookup_binding(ResolveBindingRequest::from_envelope(envelope))
            .await
        {
            tracing::debug!(
                target = "ironclaw::reborn::slack_delivery",
                error = %error,
                "skipped Slack rejection hint because the originating conversation was not authorized"
            );
            return true;
        }
        if let Err(error) = post_slack_message(
            self.services.egress.as_ref(),
            envelope.external_conversation_ref(),
            hint,
        )
        .await
        {
            tracing::debug!(
                target = "ironclaw::reborn::slack_delivery",
                error = %error,
                "failed to post rejection hint to Slack (best-effort)"
            );
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PostedSlackMessage {
    channel: String,
    ts: String,
}

// arch-exempt: too_many_args, needs a GateRouteRecordingContext bundle (store + scope identity + posted refs), plan docs/plans/2026-06-10-slack-gate-feedback-and-routing.md Phase C
#[allow(clippy::too_many_arguments)]
async fn record_gate_route_if_needed(
    route_store: &dyn DeliveredGateRouteStore,
    run_id: TurnRunId,
    tenant_id: &ironclaw_host_api::TenantId,
    user_id: &ironclaw_host_api::UserId,
    gate_ref: &str,
    scope: &TurnScope,
    posted_messages: &[PostedSlackMessage],
    envelope_conv_ref: Option<&ExternalConversationRef>,
    // Slack team id to attach to each posted-message conversation ref so that
    // inbound replies (which carry team_id as space_id) match the fingerprint.
    // A no-space fallback variant is always recorded too, for events that omit
    // team_id; the fingerprint set deduplicates when the two are identical
    // (i.e. when `posted_space_id` is None).
    posted_space_id: Option<&str>,
) {
    let mut conversation_fingerprints = std::collections::BTreeSet::new();

    for msg in posted_messages {
        // Record a space-qualified ref (matches inbound Slack events that carry team_id).
        if let Some(space) = posted_space_id
            && let Ok(conv_ref) = ironclaw_conversations::ExternalConversationRef::new(
                Some(space),
                &msg.channel,
                Some(&msg.ts),
                None,
            )
        {
            conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
        }
        // Also record the root conversation for bare replies sent directly in
        // the DM/channel instead of as a threaded reply to the prompt.
        if let Some(space) = posted_space_id
            && let Ok(conv_ref) = ironclaw_conversations::ExternalConversationRef::new(
                Some(space),
                &msg.channel,
                None,
                None,
            )
        {
            conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
        }
        // Always record a no-space fallback ref for events that omit team_id.
        if let Ok(conv_ref) = ironclaw_conversations::ExternalConversationRef::new(
            None,
            &msg.channel,
            Some(&msg.ts),
            None,
        ) {
            conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
        }
        if let Ok(conv_ref) =
            ironclaw_conversations::ExternalConversationRef::new(None, &msg.channel, None, None)
        {
            conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
        }
    }

    if let Some(env_ref) = envelope_conv_ref
        && let Ok(env_conv_ref) = conversations_ref_from_product_ref(env_ref)
    {
        let env_no_msg = env_conv_ref.without_message_id();
        conversation_fingerprints.insert(env_no_msg.conversation_fingerprint());
    }

    if conversation_fingerprints.is_empty() {
        return;
    }

    let record = DeliveredGateRouteRecord {
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        gate_ref: gate_ref.to_string(),
        run_id,
        scope: scope.clone(),
        recorded_at: Utc::now(),
        delivered_conversation_fingerprints: conversation_fingerprints.into_iter().collect(),
    };

    if let Err(error) = route_store.record_delivered_gate_route(record).await {
        // silent-ok: route recording is best-effort; resolution falls back to
        // explicit gate refs and the hint path, so a write failure never
        // aborts delivery.
        tracing::debug!(
            target = "ironclaw::reborn::slack_delivery",
            %run_id,
            error = %error,
            "failed to record delivered gate route"
        );
        return;
    }

    if let Err(sweep_err) = route_store
        .sweep_expired_delivered_gate_routes(Utc::now())
        .await
    {
        // silent-ok: sweep is opportunistic; expired routes are filtered at
        // lookup time, so a failed sweep never affects correctness.
        tracing::debug!(
            target = "ironclaw::reborn::slack_delivery",
            %run_id,
            error = %sweep_err,
            "delivered gate route sweep failed"
        );
    }
}

fn conversations_ref_from_product_ref(
    conv_ref: &ExternalConversationRef,
) -> Result<ironclaw_conversations::ExternalConversationRef, ironclaw_conversations::InboundTurnError>
{
    ironclaw_conversations::ExternalConversationRef::new(
        conv_ref.space_id(),
        conv_ref.conversation_id(),
        conv_ref.topic_id(),
        conv_ref.reply_target_message_id(),
    )
}

#[derive(Debug, Serialize)]
struct ChatPostMessageRequest<'a> {
    channel: &'a str,
    text: &'a str,
    mrkdwn: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_ts: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct ChatDeleteRequest<'a> {
    channel: &'a str,
    ts: &'a str,
}

#[derive(Debug, Deserialize)]
struct SlackMessageResponse {
    ok: bool,
    channel: Option<String>,
    ts: Option<String>,
    error: Option<String>,
}

struct TrackingSlackPostEgress {
    inner: Arc<dyn ProtocolHttpEgress>,
    posted_messages: Arc<Mutex<Vec<PostedSlackMessage>>>,
}

impl TrackingSlackPostEgress {
    fn new(inner: Arc<dyn ProtocolHttpEgress>) -> Self {
        Self {
            inner,
            posted_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn take_posted_messages(&self) -> Vec<PostedSlackMessage> {
        std::mem::take(
            &mut *self
                .posted_messages
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

#[async_trait]
impl ProtocolHttpEgress for TrackingSlackPostEgress {
    async fn send(
        &self,
        request: EgressRequest,
    ) -> Result<EgressResponse, ProtocolHttpEgressError> {
        let captures_posted_message = request.path().as_str() == "/api/chat.postMessage";
        let response = self.inner.send(request).await?;
        if captures_posted_message
            && let Some(message) = posted_slack_message_from_response(response.body())
        {
            self.posted_messages
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(message);
        }
        Ok(response)
    }
}

async fn post_slack_message(
    egress: &dyn ProtocolHttpEgress,
    conversation: &ExternalConversationRef,
    text: &str,
) -> Result<PostedSlackMessage, SlackFinalReplyDeliveryError> {
    let body = ChatPostMessageRequest {
        channel: conversation.conversation_id(),
        text,
        mrkdwn: false,
        thread_ts: conversation.topic_id(),
    };
    let response = egress
        .send(slack_web_api_request(
            "/api/chat.postMessage",
            serde_json::to_vec(&body).map_err(|error| {
                SlackFinalReplyDeliveryError::SlackWebApi {
                    reason: error.to_string(),
                }
            })?,
        )?)
        .await
        .map_err(|error| SlackFinalReplyDeliveryError::SlackWebApi {
            reason: error.to_string(),
        })?;
    if !(200..300).contains(&response.status()) {
        return Err(SlackFinalReplyDeliveryError::SlackWebApi {
            reason: format!("Slack chat.postMessage returned HTTP {}", response.status()),
        });
    }
    let parsed: SlackMessageResponse =
        serde_json::from_slice(response.body()).map_err(|error| {
            SlackFinalReplyDeliveryError::SlackWebApi {
                reason: format!("Slack chat.postMessage response was not JSON: {error}"),
            }
        })?;
    if !parsed.ok {
        return Err(SlackFinalReplyDeliveryError::SlackWebApi {
            reason: format!(
                "Slack chat.postMessage failed: {}",
                parsed.error.unwrap_or_else(|| "unknown_error".to_string())
            ),
        });
    }
    let Some(channel) = parsed.channel else {
        return Err(SlackFinalReplyDeliveryError::SlackWebApi {
            reason: "Slack chat.postMessage response missing channel".to_string(),
        });
    };
    let Some(ts) = parsed.ts else {
        return Err(SlackFinalReplyDeliveryError::SlackWebApi {
            reason: "Slack chat.postMessage response missing ts".to_string(),
        });
    };
    Ok(PostedSlackMessage { channel, ts })
}

async fn delete_slack_message(
    egress: &dyn ProtocolHttpEgress,
    message: &PostedSlackMessage,
) -> Result<(), SlackFinalReplyDeliveryError> {
    let body = ChatDeleteRequest {
        channel: &message.channel,
        ts: &message.ts,
    };
    let response = egress
        .send(slack_web_api_request(
            "/api/chat.delete",
            serde_json::to_vec(&body).map_err(|error| {
                SlackFinalReplyDeliveryError::SlackWebApi {
                    reason: error.to_string(),
                }
            })?,
        )?)
        .await
        .map_err(|error| SlackFinalReplyDeliveryError::SlackWebApi {
            reason: error.to_string(),
        })?;
    if !(200..300).contains(&response.status()) {
        return Err(SlackFinalReplyDeliveryError::SlackWebApi {
            reason: format!("Slack chat.delete returned HTTP {}", response.status()),
        });
    }
    let parsed: SlackMessageResponse =
        serde_json::from_slice(response.body()).map_err(|error| {
            SlackFinalReplyDeliveryError::SlackWebApi {
                reason: format!("Slack chat.delete response was not JSON: {error}"),
            }
        })?;
    if !parsed.ok {
        return Err(SlackFinalReplyDeliveryError::SlackWebApi {
            reason: format!(
                "Slack chat.delete failed: {}",
                parsed.error.unwrap_or_else(|| "unknown_error".to_string())
            ),
        });
    }
    Ok(())
}

fn slack_web_api_request(
    path: &'static str,
    body: Vec<u8>,
) -> Result<EgressRequest, ProductAdapterError> {
    Ok(EgressRequest::new(
        DeclaredEgressHost::new(SLACK_API_HOST)?,
        EgressMethod::post(),
        EgressPath::new(path)?,
    )
    .with_header(EgressHeader::new("content-type", "application/json")?)
    .with_body(body)
    .with_credential_handle(Some(EgressCredentialHandle::new(SLACK_BOT_TOKEN_HANDLE)?)))
}

fn posted_slack_message_from_response(body: &[u8]) -> Option<PostedSlackMessage> {
    let parsed: SlackMessageResponse = serde_json::from_slice(body).ok()?;
    if !parsed.ok {
        return None;
    }
    Some(PostedSlackMessage {
        channel: parsed.channel?,
        ts: parsed.ts?,
    })
}

fn blocked_actionable_marker(state: &TurnRunState) -> Option<BlockedActionableMarker> {
    match state.status {
        TurnStatus::BlockedApproval | TurnStatus::BlockedAuth => Some(BlockedActionableMarker {
            status: state.status,
            gate_ref: state
                .gate_ref
                .as_ref()
                .map(|gate| gate.as_str().to_string()),
        }),
        _ => None,
    }
}

fn slack_run_notification_projection_id(
    run_id: TurnRunId,
    event_kind: RunNotificationEventKind,
) -> String {
    let suffix = match event_kind {
        RunNotificationEventKind::FinalReplyReady => "final",
        RunNotificationEventKind::ProgressUpdate => "progress",
        RunNotificationEventKind::ApprovalNeeded => "approval",
        RunNotificationEventKind::AuthRequired => "auth",
        RunNotificationEventKind::RunBlocked => "blocked",
        RunNotificationEventKind::DeliveryStatus => "delivery-status",
    };
    format!("slack-run-notification:{suffix}:{run_id}")
}

fn slack_approval_gate_prompt_view(run_id: TurnRunId, gate_ref: &GateRef) -> GatePromptView {
    GatePromptView {
        turn_run_id: run_id,
        gate_ref: gate_ref.as_str().to_string(),
        headline: "Approval needed".to_string(),
        body: format!(
            "A step in the workflow requires your approval to resume.\nReply `approve` or `deny` in this thread, or use `approve {}` from anywhere.",
            gate_ref.as_str()
        ),
        allow_always: is_approval_gate_ref(gate_ref),
        approval_context: None,
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
        // A2: rejected approval/auth feedback is a single best-effort post, not a
        // long-running final delivery. Handle it before taking the shared delivery
        // semaphore so it cannot queue behind runs that may poll until max_wait.
        if self
            .post_rejection_hint_if_authorized(&envelope, &ack)
            .await
        {
            return;
        }
        // A2b: DeferredBusy feedback — the user's message was silently dropped
        // because a run is blocked on a pending gate. Post a one-shot state-aware
        // hint so the user knows to approve/deny/wait rather than being left in
        // silence. Same best-effort semantics as A2: post failure → debug! only.
        //
        // Authorization: only post if the binding lookup succeeds, matching the
        // same guard used by `post_rejection_hint_if_authorized`.
        //
        // Inline await is safe: the protocol ACK already returned before this
        // observer runs, and the runner's admission permit in runner_immediate_ack.rs
        // bounds the lifetime of this entire post-ACK task. A detached spawn would
        // escape `drain_immediate_ack_tasks` shutdown/drain without adding any
        // backpressure benefit.
        if let Some(active_run_id) = deferred_busy_user_message_run_id(&envelope, &ack) {
            // Throttle: at most one hint per (conversation, active_run_id) pair.
            // Check before the coordinator call to avoid a round-trip on repeats.
            let conv_key = envelope
                .external_conversation_ref()
                .conversation_fingerprint();
            let throttle_key = (conv_key, active_run_id);
            let already_seen = {
                let mut guard = self.hint_seen.lock().unwrap_or_else(|e| e.into_inner());
                let (queue, set) = &mut *guard;
                if set.contains(&throttle_key) {
                    true
                } else {
                    // FIFO eviction to keep the set bounded at O(1) memory.
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
                    target = "ironclaw::reborn::slack_delivery",
                    "deferred-busy hint suppressed: already posted for this (conversation, run_id) pair"
                );
                return;
            }
            // Authorization: verify the conversation has a valid binding before
            // posting. Skip silently (debug!) when unauthorized.
            let binding = match self
                .services
                .binding_service
                .lookup_binding(ResolveBindingRequest::from_envelope(&envelope))
                .await
            {
                Ok(b) => b,
                Err(error) => {
                    tracing::debug!(
                        target = "ironclaw::reborn::slack_delivery",
                        error = %error,
                        "skipped deferred-busy hint because the originating conversation was not authorized"
                    );
                    return;
                }
            };
            // Derive the scope for the active run state lookup.
            // Falls back to generic copy on any failure — never skips the hint.
            let hint = deferred_busy_hint_from_run_state(
                self.services.turn_coordinator.as_ref(),
                &binding,
                active_run_id,
            )
            .await;
            if let Err(post_err) = post_slack_message(
                self.services.egress.as_ref(),
                envelope.external_conversation_ref(),
                &hint,
            )
            .await
            {
                tracing::debug!(
                    target = "ironclaw::reborn::slack_delivery",
                    error = %post_err,
                    "failed to post deferred-busy hint to Slack (best-effort)"
                );
            }
            return;
        }
        // Single-flight guard: at most one live delivery loop per run_id.
        //
        // A gate-resolution ack (ApprovalResolution(Allow) / AuthResolution(Allowed))
        // carries the same submitted_run_id as the original user-message ack because
        // it resumes the pre-existing run. The original loop is still alive and will
        // observe the unblock on its next poll, posting the next gate or final reply
        // exactly once. Spawning a second loop for the same run_id would produce
        // duplicate posts (N resolutions ⇒ N+1 loops ⇒ gate N posted N times).
        //
        // `should_deliver_after_ack` only filters Deny resolutions; Allow resolutions
        // pass through here. We guard by run_id rather than by ack type so the fix
        // is robust to future ack variants that may also target an existing run.
        //
        // IMPORTANT: the guard is checked and inserted BEFORE acquiring the delivery
        // semaphore permit. Without this ordering, a second ack (L2) for the same
        // run_id could block on the permit while L1 is delivering; when L1 releases
        // the permit and removes the run_id, L2 would wake and pass a now-empty guard
        // set — the exact TOCTOU race this ordering closes.
        //
        // The `RunDeliveryGuard` RAII type ensures the run_id is removed on drop even
        // if `deliver_final_reply` panics, preventing a permanent delivery block.
        let _delivery_guard = if let Some(run_id) = submitted_run_id(&ack) {
            let already_delivering = {
                let mut guard = self
                    .active_delivery_run_ids
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if guard.contains(&run_id) {
                    true
                } else {
                    guard.insert(run_id);
                    false
                }
            };
            if already_delivering {
                tracing::debug!(
                    target = "ironclaw::reborn::slack_delivery",
                    %run_id,
                    "skipping redundant delivery loop: a loop is already watching this run"
                );
                return;
            }
            Some(RunDeliveryGuard {
                set: &self.active_delivery_run_ids,
                run_id,
            })
        } else {
            None
        };
        let Ok(_permit) = self.delivery_permits.clone().acquire_owned().await else {
            tracing::warn!(
                target = "ironclaw::reborn::slack_delivery",
                "Slack final reply delivery skipped because delivery semaphore was closed"
            );
            return;
        };
        let delivery_result = self.deliver_final_reply(envelope.clone(), ack).await;
        // `_delivery_guard` is dropped here automatically, removing the run_id from
        // `active_delivery_run_ids` even if `deliver_final_reply` returned an error.
        // Explicit drop makes the cleanup point visible at the call site.
        drop(_delivery_guard);
        if let Err(error) = delivery_result {
            tracing::warn!(
                target = "ironclaw::reborn::slack_delivery",
                error = %error,
                "Slack final reply delivery failed after immediate ACK"
            );
            // A3: Best-effort feedback post so the user is not left in silence.
            // Skip if a blocked-state notification was already delivered — the
            // user already saw an approval/auth prompt and is not in silence.
            let feedback = match &error {
                SlackFinalReplyDeliveryError::RunWaitTimedOut { .. } => {
                    Some(SLACK_DELIVERY_TIMEOUT_MESSAGE)
                }
                SlackFinalReplyDeliveryError::RunWaitTimedOutAfterNotification { .. } => None,
                _ => Some(SLACK_DELIVERY_ERROR_MESSAGE),
            };
            if let Some(feedback) = feedback
                && let Err(post_err) = post_slack_message(
                    self.services.egress.as_ref(),
                    envelope.external_conversation_ref(),
                    feedback,
                )
                .await
            {
                tracing::debug!(
                    target = "ironclaw::reborn::slack_delivery",
                    error = %post_err,
                    "failed to post delivery-error feedback to Slack (best-effort)"
                );
            }
        }
    }

    async fn observe_workflow_error(
        &self,
        envelope: ProductInboundEnvelope,
        error: ProductAdapterError,
    ) {
        let Some(ack) = rejection_ack_for_workflow_error(&error) else {
            return;
        };
        self.post_rejection_hint_if_authorized(&envelope, &ack)
            .await;
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
    #[error("Slack Web API helper failed: {reason}")]
    SlackWebApi { reason: String },
    #[error("outbound policy failed: {0}")]
    OutboundPolicy(#[from] OutboundError),
    #[error("run {run_id} did not finish before Slack delivery timeout")]
    RunWaitTimedOut { run_id: TurnRunId },
    /// Timeout after at least one blocked-state notification (approval/auth
    /// prompt) was already delivered. The user is not in silence, so no
    /// additional feedback message is needed.
    #[error("run {run_id} did not reach a terminal state after delivering a blocked notification")]
    RunWaitTimedOutAfterNotification { run_id: TurnRunId },
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

/// Returns the user-facing hint to post when a resolution attempt (approval or
/// auth) is rejected. Returns `None` for non-resolution payloads (e.g. user
/// messages) or for any `Duplicate` ack regardless of the prior ack inside it.
fn rejection_hint_for_resolution(
    envelope: &ProductInboundEnvelope,
    ack: &ProductInboundAck,
) -> Option<&'static str> {
    // `Duplicate` is keyed on the external event id (see `ActionFingerprintKey`
    // in ironclaw_product_workflow): Slack transport retries reuse the same
    // event id, so the same event arriving N times produces Duplicate{original}
    // on the second through Nth delivery. A user re-typing "approve" produces a
    // new event id and therefore a fresh `Rejected` ack, never `Duplicate`.
    // Posting a hint on `Duplicate{Rejected}` would repeat the side effect N
    // times on transport retries while suppressing it loses nothing — the
    // original processing already posted the hint.
    let ProductInboundAck::Rejected(effective_rejection) = ack else {
        return None;
    };
    // Only post feedback for resolution-type payloads; user messages and other
    // payloads that happen to be rejected produce no channel noise.
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

/// Returns `Some(active_run_id)` when the ack + payload combination should trigger
/// the deferred-busy hint flow: a plain `DeferredBusy` ack on a `UserMessage` payload.
///
/// Returns `None` for `Duplicate` acks (the idempotency ledger never settles
/// `DeferredBusy`, so Slack transport retries re-process as a fresh plain
/// `DeferredBusy`, never `Duplicate{DeferredBusy}`; suppressing all `Duplicate`
/// matches the invariant in `rejection_hint_for_resolution`) and for all non-user-
/// message payloads (resolution/control payloads must stay silent).
fn deferred_busy_user_message_run_id(
    envelope: &ProductInboundEnvelope,
    ack: &ProductInboundAck,
) -> Option<TurnRunId> {
    // All Duplicate acks are suppressed — same pattern as rejection_hint_for_resolution.
    if matches!(ack, ProductInboundAck::Duplicate { .. }) {
        return None;
    }
    // Only reply to user messages — resolution/control/noop payloads must stay silent.
    if !matches!(envelope.payload(), ProductInboundPayload::UserMessage(_)) {
        return None;
    }
    match ack {
        ProductInboundAck::DeferredBusy { active_run_id, .. } => Some(*active_run_id),
        _ => None,
    }
}

/// Looks up the blocking run's state and returns the appropriate deferred-busy
/// hint copy.
///
/// - `BlockedApproval` with `Some(gate_ref)` → approval wording with concrete `approve {ref}` command
/// - `BlockedApproval` with `None` gate_ref  → approval wording without a specific gate command
/// - `BlockedAuth` with `Some(gate_ref)`     → auth wording with concrete `auth deny {ref}` command
/// - `BlockedAuth` with `None` gate_ref      → auth wording without the deny command
/// - anything else / lookup failure           → generic wording
///
/// Never returns an error — lookup failures degrade to the generic copy.
async fn deferred_busy_hint_from_run_state(
    coordinator: &dyn TurnCoordinator,
    binding: &ResolvedBinding,
    active_run_id: TurnRunId,
) -> String {
    let scope = match (|| -> Result<TurnScope, ProductWorkflowError> {
        let thread_scope = thread_scope_from_binding(binding)?;
        turn_scope_from_thread_scope(binding, &thread_scope)
    })() {
        Ok(s) => s,
        Err(err) => {
            tracing::debug!(
                target = "ironclaw::reborn::slack_delivery",
                error = %err,
                "deferred-busy scope derivation failed; using generic copy"
            );
            return SLACK_DEFERRED_BUSY_GENERIC_MESSAGE.to_string();
        }
    };
    match coordinator
        .get_run_state(GetRunStateRequest {
            scope,
            run_id: active_run_id,
        })
        .await
    {
        Ok(state) => match state.status {
            TurnStatus::BlockedApproval => match state.gate_ref.as_ref() {
                Some(gate_ref) => format!(
                    "Ironclaw is waiting on a pending approval before taking new messages \
                     — reply `approve {ref}` or `deny` to resume.",
                    ref = gate_ref.as_str()
                ),
                None => SLACK_DEFERRED_BUSY_APPROVAL_MESSAGE.to_string(),
            },
            TurnStatus::BlockedAuth => match state.gate_ref.as_ref() {
                Some(gate_ref) => format!(
                    "Ironclaw is waiting on an authentication step before taking new messages \
                     — complete the authentication prompt (or reply `auth deny {ref}` to decline).",
                    ref = gate_ref.as_str()
                ),
                None => "Ironclaw is waiting on an authentication step before taking new messages \
                         — complete the authentication prompt to continue."
                    .to_string(),
            },
            _ => SLACK_DEFERRED_BUSY_GENERIC_MESSAGE.to_string(),
        },
        Err(err) => {
            tracing::debug!(
                target = "ironclaw::reborn::slack_delivery",
                error = %err,
                "deferred-busy run-state lookup failed; using generic copy"
            );
            SLACK_DEFERRED_BUSY_GENERIC_MESSAGE.to_string()
        }
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

// ── Triggered-run delivery ──────────────────────────────────────────────────
//
// When a trigger fires and the poller submits a run, this driver watches the
// run to completion and delivers the result to the creator's personal Slack DM
// using the same polling machinery as the observer path (above).

/// Composition-owned hook invoked by the trigger poller after a successful fire
/// submission. The composition root wires a real implementation (behind the
/// `slack-v2-host-beta` feature) or a no-op.
#[async_trait]
pub trait PostSubmitDeliveryHook: Send + Sync {
    /// Called with the original trigger fire, the submitted run id, and the
    /// turn scope the run was submitted under. Must not block the poller —
    /// implementations must spawn their own tasks.
    async fn on_trigger_submitted(&self, fire: TriggerFire, run_id: TurnRunId, scope: TurnScope);
}

/// No-op hook used when the Slack host-beta feature is not active.
pub struct NoopPostSubmitDeliveryHook;

#[async_trait]
impl PostSubmitDeliveryHook for NoopPostSubmitDeliveryHook {
    async fn on_trigger_submitted(
        &self,
        _fire: TriggerFire,
        _run_id: TurnRunId,
        _scope: TurnScope,
    ) {
    }
}

/// Drives triggered-run delivery for a single submitted run.
///
/// Spawns a background task that polls the run to completion (or gate) and
/// delivers the result to the creator's personal Slack DM. Personal scope only:
/// if the trigger has a `project_id` the delivery is skipped with `Denied`.
pub struct TriggeredRunDeliveryDriver {
    services: SlackFinalReplyDeliveryServices,
    settings: SlackFinalReplyDeliverySettings,
    delivery_permits: Arc<Semaphore>,
    /// Bounds the total number of spawned delivery tasks (active + waiting).
    /// Acquired via `try_acquire_owned` before spawning; released when the task
    /// exits. Overflow is recorded as `Skipped` without spawning.
    pending_permits: Arc<Semaphore>,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    route_store: Arc<dyn DeliveredGateRouteStore>,
    /// Fallback agent id used when the submitted `TurnScope::agent_id` is
    /// `None`. Must match the `default_agent_id` that
    /// `ConversationContentRefMaterializer` (and `record_trigger_prompt`)
    /// uses so the thread-scope key aligns with where the run was stored.
    fallback_agent_id: ironclaw_host_api::AgentId,
}

impl TriggeredRunDeliveryDriver {
    pub fn new(
        services: SlackFinalReplyDeliveryServices,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        route_store: Arc<dyn DeliveredGateRouteStore>,
        fallback_agent_id: ironclaw_host_api::AgentId,
    ) -> Self {
        Self::with_settings(
            services,
            SlackFinalReplyDeliverySettings::default(),
            delivery_store,
            route_store,
            fallback_agent_id,
        )
    }

    pub fn with_settings(
        services: SlackFinalReplyDeliveryServices,
        settings: SlackFinalReplyDeliverySettings,
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
        }
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
    /// `SlackFinalReplyDeliveryServices.communication_preferences`.
    ///
    /// Production call site: `build_triggered_run_delivery_hook` in
    /// `slack_host_beta.rs` — the store it passes here must be pointer-equal to
    /// `local_runtime.outbound_preferences` so WebUI-written preferences are
    /// visible to Slack delivery.  Use `Arc::ptr_eq` in tests to assert this.
    /// This accessor is for tests only and compiles to nothing in production binaries.
    #[cfg(test)]
    pub(crate) fn communication_preferences_for_test(
        &self,
    ) -> Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository> {
        Arc::clone(&self.services.communication_preferences)
    }
}

#[async_trait]
impl PostSubmitDeliveryHook for TriggeredRunDeliveryDriver {
    async fn on_trigger_submitted(&self, fire: TriggerFire, run_id: TurnRunId, scope: TurnScope) {
        // Fail closed for non-personal triggers (project_id set means shared/project scope).
        if fire.project_id.is_some() {
            tracing::debug!(
                %run_id,
                "triggered run delivery denied: project-scoped trigger is not personal scope"
            );
            self.record_outcome(run_id, TriggeredRunDeliveryOutcomeKind::Denied)
                .await;
            return;
        }

        // Guard against unbounded task accumulation: if the pending-delivery
        // queue is full, record Skipped immediately without spawning.
        let Ok(pending_permit) = Arc::clone(&self.pending_permits).try_acquire_owned() else {
            tracing::warn!(
                target: "ironclaw::reborn::slack_delivery",
                %run_id,
                "triggered run delivery skipped: pending delivery queue full"
            );
            self.record_outcome(run_id, TriggeredRunDeliveryOutcomeKind::Skipped)
                .await;
            return;
        };

        // Clone the Arcs we need into the spawned task.
        let permits = Arc::clone(&self.delivery_permits);
        let services = SlackFinalReplyDeliveryServices {
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
        };
        let settings = self.settings;
        let delivery_store = Arc::clone(&self.delivery_store);
        let fallback_agent_id = self.fallback_agent_id.clone();

        tokio::spawn(async move {
            // Hold the pending permit for the full lifetime of this task so it
            // counts against the pending-delivery cap until delivery completes.
            let _pending_permit = pending_permit;

            let Ok(_permit) = permits.clone().acquire_owned().await else {
                tracing::warn!(
                    target = "ironclaw::reborn::slack_delivery",
                    %run_id,
                    "triggered run delivery skipped: delivery semaphore closed"
                );
                record_triggered_run_outcome(
                    &*delivery_store,
                    run_id,
                    TriggeredRunDeliveryOutcomeKind::Skipped,
                )
                .await;
                return;
            };

            let outcome = deliver_triggered_run(
                &services,
                &settings,
                &fire,
                run_id,
                scope,
                &*delivery_store,
                &fallback_agent_id,
            )
            .await;
            tracing::debug!(
                target = "ironclaw::reborn::slack_delivery",
                %run_id,
                ?outcome,
                "triggered run delivery completed"
            );
        });
    }
}

impl TriggeredRunDeliveryDriver {
    async fn record_outcome(&self, run_id: TurnRunId, outcome: TriggeredRunDeliveryOutcomeKind) {
        record_triggered_run_outcome(&*self.delivery_store, run_id, outcome).await;
    }
}

/// Inner delivery coroutine for a single triggered run.
#[allow(clippy::too_many_arguments)]
async fn deliver_triggered_run(
    services: &SlackFinalReplyDeliveryServices,
    settings: &SlackFinalReplyDeliverySettings,
    fire: &TriggerFire,
    run_id: TurnRunId,
    scope: TurnScope,
    delivery_store: &dyn TriggeredRunDeliveryStore,
    fallback_agent_id: &ironclaw_host_api::AgentId,
) -> TriggeredRunDeliveryOutcomeKind {
    // The actor is the trigger creator.
    let actor = TurnActor::new(fire.creator_user_id.clone());

    // Derive the TriggerCommunicationContext for the outbound origin.
    let trigger_context = match triggered_communication_context(fire) {
        Ok(ctx) => ctx,
        Err(reason) => {
            tracing::warn!(
                target = "ironclaw::reborn::slack_delivery",
                %run_id,
                %reason,
                "triggered run delivery skipped: cannot build trigger context"
            );
            let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
            record_triggered_run_outcome(delivery_store, run_id, outcome).await;
            return outcome;
        }
    };

    // Build a thread scope for reading the finalized assistant message.
    // The turn scope's thread_id is the canonical thread for this trigger session.
    // Use the scope's agent_id when present; otherwise fall back to the configured
    // fallback_agent_id — the same value record_trigger_prompt uses — so the key
    // matches the thread that was stored at submit time.
    let _thread_id = scope.thread_id.clone();
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

    // Build the reply-target authority: resolves from the creator's personal preference.
    let authority = TriggeredSlackReplyTargetAuthority {
        scope: scope.clone(),
        actor: actor.clone(),
        trigger_context: trigger_context.clone(),
        resolved_space_id: std::sync::Mutex::new(None),
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
            Err(err) => {
                tracing::warn!(
                    target = "ironclaw::reborn::slack_delivery",
                    %run_id,
                    error = %err,
                    "triggered run wait failed"
                );
                let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
        };

        // Build the notification payload.
        let notification = match triggered_notification_for_state(
            services,
            &scope,
            &thread_scope,
            &actor,
            &state,
            run_id,
        )
        .await
        {
            Ok(Some(n)) => n,
            Ok(None) => {
                // Run completed with no assistant message — normal "skipped" outcome.
                let outcome = TriggeredRunDeliveryOutcomeKind::Skipped;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
            Err(err) => {
                tracing::warn!(
                    target = "ironclaw::reborn::slack_delivery",
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

        // Build the delivery request and deliver.
        let delivery_result = deliver_triggered_notification(
            services,
            &scope,
            &actor,
            run_id,
            &state,
            &authority,
            notification,
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
                if let Some(marker) = next_blocked_marker {
                    if event_kind == RunNotificationEventKind::AuthRequired {
                        messages_to_delete_after_final.extend(posted_messages);
                    }
                    delivered_blocked_marker = Some(marker);
                    // Loop again to wait for the next actionable state.
                    continue;
                }
                // Terminal delivery — clean up auth messages that should not persist.
                for message in messages_to_delete_after_final {
                    delete_triggered_slack_message(services, message).await;
                }
                let outcome = TriggeredRunDeliveryOutcomeKind::Delivered;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
            Err(failure) => {
                tracing::warn!(
                    target = "ironclaw::reborn::slack_delivery",
                    %run_id,
                    reason = %failure,
                    "triggered run delivery failed"
                );
                let outcome = match failure {
                    TriggeredNotificationFailure::NoDefaultConfigured => {
                        TriggeredRunDeliveryOutcomeKind::NoDefaultConfigured
                    }
                    TriggeredNotificationFailure::Denied => TriggeredRunDeliveryOutcomeKind::Denied,
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

/// Waits for the given run to reach an actionable state (Completed, BlockedApproval, BlockedAuth).
async fn wait_for_actionable_triggered(
    services: &SlackFinalReplyDeliveryServices,
    scope: &TurnScope,
    run_id: TurnRunId,
    settings: &SlackFinalReplyDeliverySettings,
    delivered_blocked_marker: &Option<BlockedActionableMarker>,
) -> Result<TurnRunState, SlackFinalReplyDeliveryError> {
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
            return Err(SlackFinalReplyDeliveryError::RunWaitTimedOut { run_id });
        }
        tokio::time::sleep(jittered_poll_interval(poll_interval, &run_id)).await;
        poll_interval = poll_interval
            .saturating_mul(2)
            .min(MAX_SLACK_RUN_POLL_INTERVAL);
    }
}

/// Builds the notification payload for a triggered run's actionable state.
async fn triggered_notification_for_state(
    services: &SlackFinalReplyDeliveryServices,
    scope: &TurnScope,
    thread_scope: &ThreadScope,
    actor: &TurnActor,
    state: &TurnRunState,
    run_id: TurnRunId,
) -> Result<Option<SlackActionableNotification>, SlackFinalReplyDeliveryError> {
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
                    %run_id,
                    "completed triggered run has no finalized assistant message; skipping delivery"
                );
                return Ok(None);
            };
            Ok(Some(SlackActionableNotification {
                event_kind: RunNotificationEventKind::FinalReplyReady,
                payload: ProductOutboundPayload::FinalReply(FinalReplyView {
                    turn_run_id: run_id,
                    text,
                    generated_at: Utc::now(),
                }),
                gate_ref_for_routing: None,
            }))
        }
        TurnStatus::BlockedApproval => {
            let Some(gate_ref) = state.gate_ref.as_ref() else {
                tracing::warn!(
                    %run_id,
                    "triggered run blocked on approval without gate ref; skipping"
                );
                return Ok(None);
            };
            // Approval-prompt copy for triggered runs: "Reply approve <gate_ref>"
            let gate_ref_str = gate_ref.as_str().to_string();
            Ok(Some(SlackActionableNotification {
                event_kind: RunNotificationEventKind::ApprovalNeeded,
                payload: ProductOutboundPayload::GatePrompt(GatePromptView {
                    turn_run_id: run_id,
                    gate_ref: gate_ref_str.clone(),
                    headline: "Approval needed".to_string(),
                    body: format!("Reply `approve {gate_ref_str}` to continue."),
                    allow_always: is_approval_gate_ref(gate_ref),
                    approval_context: None,
                }),
                gate_ref_for_routing: Some(gate_ref_str),
            }))
        }
        TurnStatus::BlockedAuth => {
            let Some(gate_ref) = state.gate_ref.as_ref() else {
                tracing::warn!(
                    %run_id,
                    "triggered run blocked on auth without gate ref; skipping"
                );
                return Ok(None);
            };
            // Auth notifications for triggered runs: strip authorization_url (no secrets in channel).
            // Use the trigger creator as the actor. Fall back to the thread scope owner if set.
            let thread_scope_owner = thread_scope
                .owner_user_id
                .clone()
                .unwrap_or_else(|| actor.user_id.clone());
            let turn_scope_for_auth = TurnScope::new_with_owner(
                scope.tenant_id.clone(),
                scope.agent_id.clone(),
                scope.project_id.clone(),
                scope.thread_id.clone(),
                Some(thread_scope_owner.clone()),
            );
            let actor_for_auth = TurnActor::new(thread_scope_owner);
            let mut view = crate::auth_prompt::auth_prompt_view_for_blocked_auth(
                &actor_for_auth.user_id,
                &turn_scope_for_auth,
                run_id,
                gate_ref.as_str(),
                "Authentication required to continue this automation.".to_string(),
                &state.credential_requirements,
                services.auth_challenges.as_deref(),
            )
            .await?;
            // Strip auth URL — secrets must not appear in the channel.
            view.authorization_url = None;
            Ok(Some(SlackActionableNotification {
                event_kind: RunNotificationEventKind::AuthRequired,
                payload: ProductOutboundPayload::AuthPrompt(view),
                gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
            }))
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
    /// Any other delivery or transport failure.
    Other(String),
}

impl std::fmt::Display for TriggeredNotificationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDefaultConfigured => write!(f, "no default delivery target configured"),
            Self::Denied => write!(f, "delivery target access denied"),
            Self::Other(reason) => write!(f, "{reason}"),
        }
    }
}

/// Delivers a triggered-run notification, returning the list of posted Slack messages.
async fn deliver_triggered_notification(
    services: &SlackFinalReplyDeliveryServices,
    scope: &TurnScope,
    actor: &TurnActor,
    run_id: TurnRunId,
    state: &TurnRunState,
    authority: &TriggeredSlackReplyTargetAuthority,
    notification: SlackActionableNotification,
) -> Result<Vec<PostedSlackMessage>, TriggeredNotificationFailure> {
    let SlackActionableNotification {
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
    let projection_id = slack_run_notification_projection_id(run_id, event_kind);
    let projection_ref = ProjectionUpdateRef::new(projection_id.clone()).map_err(|reason| {
        TriggeredNotificationFailure::Other(format!("invalid_projection_ref: {reason}"))
    })?;
    let delivery = ironclaw_outbound::PrepareCommunicationDeliveryRequest {
        resolution_request: CommunicationDeliveryResolutionRequest {
            scope: scope.clone(),
            actor: actor.clone(),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind,
                origin: RunNotificationOrigin::Triggered {
                    trigger: authority.trigger_context.clone(),
                },
            }),
        },
        turn_run_id: Some(run_id),
        projection_ref,
        attempted_at: Utc::now(),
    };

    let tracked_egress = TrackingSlackPostEgress::new(Arc::clone(&services.egress));
    prepare_and_render_product_outbound(
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
        },
    )
    .await
    .map_err(classify_delivery_error)?;

    Ok(tracked_egress.take_posted_messages())
}

/// Classify a [`ProductOutboundDeliveryError`] into the typed
/// [`TriggeredNotificationFailure`] variants used for outcome recording.
fn classify_delivery_error(
    error: ironclaw_product_workflow::ProductOutboundDeliveryError,
) -> TriggeredNotificationFailure {
    use ironclaw_outbound::OutboundError;
    use ironclaw_product_workflow::ProductOutboundDeliveryError;
    match &error {
        ProductOutboundDeliveryError::Outbound(OutboundError::PreferenceTargetMissing {
            ..
        }) => TriggeredNotificationFailure::NoDefaultConfigured,
        ProductOutboundDeliveryError::Outbound(OutboundError::AccessDenied) => {
            TriggeredNotificationFailure::Denied
        }
        _ => TriggeredNotificationFailure::Other(error.to_string()),
    }
}

async fn delete_triggered_slack_message(
    services: &SlackFinalReplyDeliveryServices,
    message: PostedSlackMessage,
) {
    if let Err(error) = delete_slack_message(services.egress.as_ref(), &message).await {
        tracing::warn!(
            target = "ironclaw::reborn::slack_delivery",
            error = %error,
            "failed to delete triggered delivery auth message"
        );
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
            target = "ironclaw::reborn::slack_delivery",
            %run_id,
            error = %error,
            "failed to record triggered run delivery outcome (best-effort)"
        );
    }
}

/// Build a `TriggerCommunicationContext` from the fire's identity.
fn triggered_communication_context(
    fire: &TriggerFire,
) -> Result<TriggerCommunicationContext, String> {
    let trigger_origin_ref = TriggerOriginRef::new(fire.identity.trigger_id().to_string())
        .map_err(|e| format!("invalid trigger origin ref: {e}"))?;
    let fire_slot = TriggerFireSlot::new(fire.identity.fire_slot().to_rfc3339())
        .map_err(|e| format!("invalid fire slot: {e}"))?;
    Ok(TriggerCommunicationContext {
        trigger_origin_ref,
        trigger_source_kind: TriggerSourceKind::Schedule,
        fire_slot,
    })
}

/// Reply-target authority for triggered-run delivery.
///
/// Resolves the delivery target from the creator's personal communication
/// preference (via `CommunicationPreferenceRepository`). Validates that the
/// reply target is the one the resolution engine chose (no substitution).
struct TriggeredSlackReplyTargetAuthority {
    scope: TurnScope,
    actor: TurnActor,
    trigger_context: TriggerCommunicationContext,
    /// Space id (Slack team id) captured during
    /// `resolve_product_outbound_target_metadata`. Updated on every resolution.
    /// Used after delivery to attach the team id to posted-message gate-route
    /// refs so inbound replies (which carry team_id as space_id)
    /// fingerprint-match the recorded ref.
    resolved_space_id: std::sync::Mutex<Option<String>>,
}

#[async_trait]
impl ReplyTargetBindingValidator for TriggeredSlackReplyTargetAuthority {
    async fn validate_reply_target(
        &self,
        request: ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError> {
        // The resolution engine chose this target from the creator's personal preference.
        // We trust it as long as the scope and actor match.
        if request.scope != self.scope || request.actor != self.actor {
            return Err(OutboundError::AccessDenied);
        }
        Ok(ReplyTargetBindingClaim::new(request.candidate.target))
    }
}

#[async_trait]
impl ProductOutboundTargetResolver for TriggeredSlackReplyTargetAuthority {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ValidatedReplyTargetBinding,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        // Decode the DM channel ID from the binding ref. The ref was built by
        // `slack_personal_dm_reply_target_binding_ref` / `slack_shared_channel_reply_target_binding_ref`
        // and encodes space + conversation in length-prefixed segments. We extract
        // only what we need (channel id + optional team id) to reconstruct the
        // `ExternalConversationRef` for the Slack adapter.
        let (conversation_id, space_id) = slack_conversation_id_from_reply_target_binding_ref(
            target.target(),
        )
        .ok_or_else(|| ProductWorkflowError::BindingResolutionFailed {
            reason: format!(
                "triggered delivery: cannot extract Slack channel from binding ref '{}'",
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

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_product_adapters::{
        AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
        ExternalEventId, ParsedProductInbound, ProtocolAuthEvidence, TrustedInboundContext,
    };
    use ironclaw_turns::AcceptedMessageRef;

    fn accepted_ack() -> ProductInboundAck {
        ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("slack:test-message")
                .expect("accepted message ref"),
            submitted_run_id: TurnRunId::new(),
        }
    }

    fn envelope(payload: ProductInboundPayload) -> ProductInboundEnvelope {
        let adapter_id =
            ironclaw_product_adapters::ProductAdapterId::new("slack_v2").expect("adapter");
        let installation_id = AdapterInstallationId::new("install_alpha").expect("installation");
        let evidence = ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Slack-Signature".to_string(),
            },
            installation_id.as_str(),
        );
        let context = TrustedInboundContext::from_verified_evidence(
            adapter_id,
            installation_id,
            Utc::now(),
            &evidence,
        )
        .expect("trusted context");
        let parsed = ParsedProductInbound::new(
            ExternalEventId::new("evt:test").expect("event"),
            ExternalActorRef::new("slack_user", "U123", None::<String>).expect("actor"),
            ExternalConversationRef::new(Some("T123"), "D123", None, None).expect("conversation"),
            payload,
        )
        .expect("parsed inbound");
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
    }

    #[test]
    fn auth_denial_ack_does_not_enter_slack_delivery_loop() {
        let payload = ProductInboundPayload::AuthResolution(
            ironclaw_product_adapters::AuthResolutionPayload::new(
                "gate:auth-test",
                ironclaw_product_adapters::AuthResolutionResult::Denied,
            )
            .expect("auth resolution"),
        );

        assert!(!should_deliver_after_ack(
            &envelope(payload),
            &accepted_ack()
        ));
    }

    #[test]
    fn auth_completion_ack_still_enters_slack_delivery_loop() {
        let payload = ProductInboundPayload::AuthResolution(
            ironclaw_product_adapters::AuthResolutionPayload::new(
                "gate:auth-test",
                ironclaw_product_adapters::AuthResolutionResult::CallbackCompleted {
                    callback_ref: ironclaw_auth::AuthFlowId::new().to_string(),
                },
            )
            .expect("auth resolution"),
        );

        assert!(should_deliver_after_ack(
            &envelope(payload),
            &accepted_ack()
        ));
    }

    // ── Driver-level tests ─────────────────────────────────────────────────────
    //
    // These tests drive `TriggeredRunDeliveryDriver::on_trigger_submitted` and
    // `deliver_triggered_run` directly using lightweight in-process fakes for
    // all I/O surfaces. They are intentionally NOT full-runtime e2e tests —
    // the plan explicitly forbids exposing `host_state_filesystem` from
    // `RebornRuntime` for that purpose.

    use std::sync::OnceLock;

    use ironclaw_outbound::{
        CommunicationPreferenceRecord, DeliveryDefaultScope, InMemoryDeliveredGateRouteStore,
        InMemoryOutboundStateStore, InMemoryTriggeredRunDeliveryStore,
        WriteCommunicationPreferenceRequest,
    };
    use ironclaw_product_adapters::{
        EgressResponse, FakeOutboundDeliverySink, FakeProtocolHttpEgress, ProductAdapterId,
        ProtocolHttpEgressError,
    };
    use ironclaw_slack_v2_adapter::{
        SlackV2Adapter, SlackV2AdapterConfig, slack_request_signature_auth_requirement,
    };
    use ironclaw_threads::{
        AppendAssistantDraftRequest, EnsureThreadRequest, InMemorySessionThreadService,
        MessageContent, SessionThreadService,
    };
    use ironclaw_triggers::{TriggerFire, TriggerFireIdentity, TriggerId};
    use ironclaw_turns::{
        EventCursor, GateRef, GetRunStateRequest, ReplyTargetBindingRef, ResumeTurnRequest,
        ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse, RunProfileId, RunProfileVersion,
        SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator, TurnError,
        TurnId, TurnRunId, TurnRunState, TurnScope, TurnStatus,
    };

    // --- Minimal inline fakes ------------------------------------------------

    /// Scripted run-state entry: status + optional approval/auth gate ref.
    #[derive(Clone)]
    struct ScriptedRunState {
        status: TurnStatus,
        gate_ref: Option<GateRef>,
    }

    struct ScriptedTurnCoordinator {
        /// Run states returned in order by `get_run_state`. Wraps around.
        states: Vec<ScriptedRunState>,
        calls: Mutex<usize>,
    }

    impl ScriptedTurnCoordinator {
        fn with_states(states: Vec<ScriptedRunState>) -> Self {
            assert!(!states.is_empty(), "must provide at least one state");
            Self {
                states,
                calls: Mutex::new(0),
            }
        }

        fn with_single_status(status: TurnStatus) -> Self {
            Self::with_states(vec![ScriptedRunState {
                status,
                gate_ref: None,
            }])
        }
    }

    struct TestNoopConversationBindingService;

    #[async_trait]
    impl ConversationBindingService for TestNoopConversationBindingService {
        async fn resolve_binding(
            &self,
            _request: ResolveBindingRequest,
        ) -> Result<ResolvedBinding, ProductWorkflowError> {
            Err(ProductWorkflowError::BindingResolutionFailed {
                reason: "not used in triggered delivery tests".to_string(),
            })
        }

        async fn lookup_binding(
            &self,
            _request: ResolveBindingRequest,
        ) -> Result<ResolvedBinding, ProductWorkflowError> {
            Err(ProductWorkflowError::BindingResolutionFailed {
                reason: "not used in triggered delivery tests".to_string(),
            })
        }
    }

    #[async_trait]
    impl TurnCoordinator for ScriptedTurnCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            Ok(TurnRunId::new())
        }

        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "ScriptedTurnCoordinator does not support submit_turn".to_string(),
            })
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "ScriptedTurnCoordinator does not support resume_turn".to_string(),
            })
        }

        async fn retry_turn(
            &self,
            _request: ironclaw_turns::RetryTurnRequest,
        ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "ScriptedTurnCoordinator does not support retry_turn".to_string(),
            })
        }

        async fn get_run_state(
            &self,
            request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            let mut calls = self.calls.lock().expect("calls lock");
            let idx = *calls % self.states.len();
            *calls += 1;
            let scripted = self.states[idx].clone();
            // Build a minimal-but-valid TurnRunState from the scripted status + gate_ref.
            Ok(TurnRunState {
                scope: request.scope.clone(),
                actor: None,
                turn_id: TurnId::new(),
                run_id: request.run_id,
                status: scripted.status,
                accepted_message_ref: AcceptedMessageRef::new("msg:scripted").expect("valid ref"),
                source_binding_ref: SourceBindingRef::new("src:scripted").expect("valid ref"),
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:test:scripted")
                    .expect("valid ref"),
                resolved_run_profile_id: RunProfileId::default_profile(),
                resolved_run_profile_version: RunProfileVersion::new(1),
                resolved_model_route: None,
                received_at: Utc::now(),
                checkpoint_id: None,
                gate_ref: scripted.gate_ref,
                credential_requirements: Vec::new(),
                failure: None,
                event_cursor: EventCursor(1),
            })
        }

        async fn cancel_run(
            &self,
            _request: ironclaw_turns::CancelRunRequest,
        ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "ScriptedTurnCoordinator does not support cancel_run".to_string(),
            })
        }
    }

    // --- Helpers --------------------------------------------------------------

    fn scripted_state(status: TurnStatus, gate_ref: Option<&str>) -> ScriptedRunState {
        ScriptedRunState {
            status,
            gate_ref: gate_ref.map(|s| GateRef::new(s).expect("gate ref")),
        }
    }

    fn minimal_trigger_fire(project_id: Option<ironclaw_host_api::ProjectId>) -> TriggerFire {
        let tenant_id = ironclaw_host_api::TenantId::new("test-tenant").expect("tenant");
        let trigger_id = TriggerId::new();
        let fire_slot = Utc::now();
        let identity = TriggerFireIdentity::new(tenant_id, trigger_id, fire_slot);
        TriggerFire {
            identity,
            creator_user_id: ironclaw_host_api::UserId::new("creator-user").expect("user id"),
            agent_id: None,
            project_id,
            prompt: "test prompt".to_string(),
        }
    }

    fn personal_turn_scope() -> TurnScope {
        let tenant = ironclaw_host_api::TenantId::new("test-tenant").expect("tenant");
        let agent = ironclaw_host_api::AgentId::new("test-agent").expect("agent");
        let thread = ironclaw_host_api::ThreadId::new("test-thread").expect("thread");
        let owner = ironclaw_host_api::UserId::new("creator-user").expect("owner");
        TurnScope::new_with_owner(tenant, Some(agent), None, thread, Some(owner))
    }

    /// Build a `SlackMessageResponse` JSON that looks like a successful post.
    fn slack_post_ok_json(channel: &str, ts: &str) -> Vec<u8> {
        serde_json::json!({
            "ok": true,
            "channel": channel,
            "ts": ts
        })
        .to_string()
        .into_bytes()
    }

    /// Build a valid Slack reply-target binding ref that
    /// `slack_conversation_id_from_reply_target_binding_ref` can decode.
    ///
    /// Mirrors the segment format produced by `slack_personal_dm_reply_target_binding_ref`.
    fn test_slack_binding_ref(installation_id: &str, agent_id: &str) -> ReplyTargetBindingRef {
        fn seg(name: &str, value: &str) -> String {
            format!("{}:{}:{};", name, value.len(), value)
        }
        let raw = format!(
            "{}{}{}{}{}{}{}{}{}",
            seg("adapter", "slack_v2"),
            seg("installation", installation_id),
            seg("agent", agent_id),
            seg("project", ""),
            seg("space", "T123"),
            seg("conversation", "D456"),
            seg("topic", ""),
            seg("actor_kind", "slack_user"),
            seg("actor", "U123"),
        );
        crate::slack_outbound_targets::slack_reply_target_binding_ref_from_raw(raw)
            .expect("test binding ref")
    }

    /// Seed a personal communication preference pointing at a Slack DM channel
    /// with a correctly encoded binding ref.
    async fn seed_personal_preference(
        store: &InMemoryOutboundStateStore,
        scope: &TurnScope,
        binding_ref: ReplyTargetBindingRef,
    ) {
        let tenant = scope.tenant_id.clone();
        let user = scope
            .explicit_owner_user_id()
            .cloned()
            .expect("owner user id for preference seed");
        let updated_by = user.clone();
        let record = CommunicationPreferenceRecord {
            scope: DeliveryDefaultScope::personal(tenant, user),
            final_reply_target: Some(binding_ref.clone()),
            progress_target: None,
            approval_prompt_target: Some(binding_ref),
            auth_prompt_target: None,
            default_modality: None,
            updated_at: Utc::now(),
            updated_by,
        };
        store
            .write_communication_preference(WriteCommunicationPreferenceRequest {
                record,
                expected_version: None,
            })
            .await
            .expect("seed preference");
    }

    /// Build a `SlackV2Adapter` with the given installation_id.
    fn test_adapter(installation_id: &str) -> Arc<SlackV2Adapter> {
        Arc::new(SlackV2Adapter::new(SlackV2AdapterConfig {
            adapter_id: ProductAdapterId::new("slack_v2").expect("adapter id"),
            installation_id: AdapterInstallationId::new(installation_id).expect("installation id"),
            egress_credential_handle: EgressCredentialHandle::new("slack_bot_token")
                .expect("credential handle"),
            auth_requirement: slack_request_signature_auth_requirement(),
        }))
    }

    fn make_services(
        coordinator: Arc<dyn TurnCoordinator>,
        thread_service: Arc<dyn ironclaw_threads::SessionThreadService>,
        egress: Arc<FakeProtocolHttpEgress>,
        outbound: Arc<InMemoryOutboundStateStore>,
        installation_id: &str,
    ) -> SlackFinalReplyDeliveryServices {
        SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(TestNoopConversationBindingService),
            thread_service,
            turn_coordinator: coordinator,
            outbound_store: outbound.clone(),
            route_store: Arc::new(InMemoryDeliveredGateRouteStore::default()),
            communication_preferences: outbound,
            adapter: test_adapter(installation_id),
            egress,
            delivery_sink: Arc::new(FakeOutboundDeliverySink::default()),
            auth_challenges: None,
        }
    }

    /// Seed a finalized assistant message for the given run_id on the thread
    /// that `deliver_triggered_run` will look up.
    async fn seed_finalized_assistant_message(
        thread_service: &InMemorySessionThreadService,
        scope: &TurnScope,
        run_id: TurnRunId,
        text: &str,
    ) {
        let thread_scope = ironclaw_threads::ThreadScope {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope.agent_id.clone().expect("agent"),
            project_id: scope.project_id.clone(),
            owner_user_id: scope.explicit_owner_user_id().cloned(),
            mission_id: None,
        };
        // Ensure the thread exists first.
        let thread = thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(scope.thread_id.clone()),
                created_by_actor_id: "test-actor".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("ensure thread");
        // Append a draft then finalize it with the test text.
        let draft = thread_service
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: thread_scope.clone(),
                thread_id: thread.thread_id.clone(),
                turn_run_id: run_id.to_string(),
                content: MessageContent::text(text.to_string()),
            })
            .await
            .expect("append draft");
        thread_service
            .finalize_assistant_message(
                &thread_scope,
                &thread.thread_id,
                draft.message_id,
                MessageContent::text(text.to_string()),
            )
            .await
            .expect("finalize message");
    }

    /// Poll `delivery_store` until a record for `run_id` exists, then return it.
    ///
    /// The record is written as the very last step of every delivery path, so
    /// once it is present the spawned task has fully completed. Times out after
    /// 5 s to prevent hangs in broken test scenarios.
    async fn wait_for_delivery_record(
        delivery_store: &InMemoryTriggeredRunDeliveryStore,
        run_id: TurnRunId,
    ) -> TriggeredRunDeliveryRecord {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(record) = delivery_store
                    .load_triggered_run_delivery(run_id)
                    .await
                    .expect("load record")
                {
                    return record;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("delivery record appeared within 5 s")
    }

    // --- Tests ----------------------------------------------------------------

    #[tokio::test]
    async fn driver_happy_path_completed_run_with_preference_delivers_and_records_delivered() {
        let install = "test-install";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Completed,
        ));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_finalized_assistant_message(&thread_service, &scope, run_id, "Hello from Ironclaw")
            .await;

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "1234.5678"),
            )),
        );

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_secs(5),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let driver = TriggeredRunDeliveryDriver::with_settings(
            services,
            settings,
            delivery_store.clone(),
            route_store.clone(),
            scope.agent_id.clone().expect("test scope has agent"),
        );

        let fire = minimal_trigger_fire(None);
        driver.on_trigger_submitted(fire, run_id, scope).await;

        // Poll until the spawned delivery task writes its outcome record.
        let record = wait_for_delivery_record(&delivery_store, run_id).await;

        // Egress should have been called for chat.postMessage.
        assert!(
            egress
                .calls()
                .iter()
                .any(|c| c.path == "/api/chat.postMessage"),
            "expected chat.postMessage egress call"
        );

        // Outcome should be Delivered.
        assert_eq!(record.outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    }

    #[tokio::test]
    async fn driver_no_preference_records_no_default_configured_without_egress() {
        let install = "test-install";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();

        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Completed,
        ));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        // Seed a finalized message so the delivery proceeds to the preference lookup.
        // Without it, the thread lookup fails first and the outcome would be Failed.
        seed_finalized_assistant_message(
            &thread_service,
            &scope,
            run_id,
            "Test completion message",
        )
        .await;
        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // No preference seeded → resolution engine returns PreferenceTargetMissing.

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_secs(5),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let driver = TriggeredRunDeliveryDriver::with_settings(
            services,
            settings,
            delivery_store.clone(),
            route_store.clone(),
            scope.agent_id.clone().expect("test scope has agent"),
        );

        let fire = minimal_trigger_fire(None);
        driver.on_trigger_submitted(fire, run_id, scope).await;

        // Poll until the spawned delivery task writes its outcome record.
        let record = wait_for_delivery_record(&delivery_store, run_id).await;

        // No chat.postMessage expected.
        assert!(
            !egress
                .calls()
                .iter()
                .any(|c| c.path == "/api/chat.postMessage"),
            "expected no chat.postMessage call"
        );

        assert_eq!(
            record.outcome,
            TriggeredRunDeliveryOutcomeKind::NoDefaultConfigured
        );
    }

    #[tokio::test]
    async fn driver_approval_gate_body_contains_approve_keyword_without_http_url() {
        let install = "test-install";
        let gate_ref_str = "gate:approval-test-001";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        // First poll → BlockedApproval; second poll → Completed.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![
            scripted_state(TurnStatus::BlockedApproval, Some(gate_ref_str)),
            scripted_state(TurnStatus::Completed, None),
        ]));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_finalized_assistant_message(
            &thread_service,
            &scope,
            run_id,
            "Run complete after approval.",
        )
        .await;

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Approval prompt response.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "1111.2222"),
            )),
        );
        // Final reply response.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "3333.4444"),
            )),
        );

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_secs(5),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let driver = TriggeredRunDeliveryDriver::with_settings(
            services,
            settings,
            delivery_store.clone(),
            route_store.clone(),
            scope.agent_id.clone().expect("test scope has agent"),
        );

        let fire = minimal_trigger_fire(None);
        driver.on_trigger_submitted(fire, run_id, scope).await;

        // Poll until the spawned delivery task writes its outcome record (record
        // is written last, so its presence implies delivery is fully finished).
        let record = wait_for_delivery_record(&delivery_store, run_id).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert!(
            !post_calls.is_empty(),
            "expected at least one chat.postMessage call"
        );

        // Approval-prompt body must contain "approve <gate_ref>".
        let first_body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            first_body.contains("approve") && first_body.contains(gate_ref_str),
            "approval prompt body must contain 'approve {gate_ref_str}'"
        );
        // Must not contain an http(s) URL (no secrets in trigger channel).
        assert!(
            !first_body.contains("http://") && !first_body.contains("https://"),
            "approval prompt must not contain http(s) URL"
        );

        assert_eq!(record.outcome, TriggeredRunDeliveryOutcomeKind::Delivered);

        // The delivered approval prompt must record a gate route keyed by the
        // trigger creator so a DM reply can resolve the gate on the triggered
        // run's thread — even when the run scope has no explicit owner.
        let scope = personal_turn_scope();
        let creator = ironclaw_host_api::UserId::new("creator-user").expect("user id");
        let route = route_store
            .load_delivered_gate_route(&scope.tenant_id, &creator, gate_ref_str)
            .await
            .expect("load gate route")
            .expect("gate route recorded");
        assert_eq!(route.run_id, run_id);
        assert_eq!(route.scope.thread_id, scope.thread_id);
    }

    #[tokio::test]
    async fn driver_project_scoped_trigger_records_denied_without_egress() {
        let install = "test-install";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();

        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Completed,
        ));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        let driver = TriggeredRunDeliveryDriver::new(
            services,
            delivery_store.clone(),
            route_store,
            scope.agent_id.clone().expect("test scope has agent"),
        );

        // project_id is set → non-personal scope → denied immediately (no spawn).
        let project_id = ironclaw_host_api::ProjectId::new("some-project").expect("project");
        let fire = minimal_trigger_fire(Some(project_id));
        driver.on_trigger_submitted(fire, run_id, scope).await;

        // Record is written synchronously before any spawn.
        let record = delivery_store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("load record")
            .expect("record exists");
        assert_eq!(
            record.outcome,
            TriggeredRunDeliveryOutcomeKind::Denied,
            "project-scoped trigger must record Denied"
        );
        assert!(
            !egress
                .calls()
                .iter()
                .any(|c| c.path == "/api/chat.postMessage"),
            "no egress expected for denied trigger"
        );
    }

    // --- BlockedAuth / timeout driver tests ------------------------------------

    /// BlockedAuth state: driver sends an auth-prompt notification (no http/https URL),
    /// then continues polling, eventually receives Completed, and records Delivered.
    #[tokio::test]
    async fn driver_blocked_auth_prompt_body_contains_no_http_url_outcome_delivered() {
        let install = "test-install";
        let gate_ref_str = "gate:auth-test-001";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        // First poll → BlockedAuth with gate_ref; second poll → Completed.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![
            scripted_state(TurnStatus::BlockedAuth, Some(gate_ref_str)),
            scripted_state(TurnStatus::Completed, None),
        ]));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_finalized_assistant_message(
            &thread_service,
            &scope,
            run_id,
            "Run complete after auth.",
        )
        .await;

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Auth-prompt delivery response.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "1111.3333"),
            )),
        );
        // Final reply response (after Completed).
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "2222.4444"),
            )),
        );

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_secs(5),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let driver = TriggeredRunDeliveryDriver::with_settings(
            services,
            settings,
            delivery_store.clone(),
            route_store.clone(),
            scope.agent_id.clone().expect("test scope has agent"),
        );

        let fire = minimal_trigger_fire(None);
        driver.on_trigger_submitted(fire, run_id, scope).await;

        let record = wait_for_delivery_record(&delivery_store, run_id).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert!(
            !post_calls.is_empty(),
            "expected at least one chat.postMessage egress call"
        );

        // Auth-prompt body must NOT contain an http/https URL.
        let first_body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            !first_body.contains("http://") && !first_body.contains("https://"),
            "auth-prompt body must not contain an http/https URL (got: {first_body})"
        );

        assert_eq!(
            record.outcome,
            TriggeredRunDeliveryOutcomeKind::Delivered,
            "terminal outcome must be Delivered"
        );
    }

    /// Timeout: coordinator always returns a non-terminal, non-blocked status.
    /// With max_wait=1ms and poll_interval=0, the driver must time out and record Failed
    /// without making any chat.postMessage egress calls.
    #[tokio::test]
    async fn driver_wait_timeout_records_failed_without_egress() {
        let install = "test-install";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();

        // Always Running — never terminal or blocked.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_millis(1),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let driver = TriggeredRunDeliveryDriver::with_settings(
            services,
            settings,
            delivery_store.clone(),
            route_store,
            scope.agent_id.clone().expect("test scope has agent"),
        );

        let fire = minimal_trigger_fire(None);
        driver.on_trigger_submitted(fire, run_id, scope).await;

        let record = wait_for_delivery_record(&delivery_store, run_id).await;

        assert_eq!(
            record.outcome,
            TriggeredRunDeliveryOutcomeKind::Failed,
            "timed-out delivery must record Failed"
        );
        assert!(
            !egress
                .calls()
                .iter()
                .any(|c| c.path == "/api/chat.postMessage"),
            "no chat.postMessage egress expected for timed-out run"
        );
    }

    // --- Pending-delivery queue cap tests -------------------------------------

    /// When `max_pending_deliveries = 1` and the single pending slot is already
    /// held, a second `on_trigger_submitted` call must record `Skipped` without
    /// spawning a delivery task.
    #[tokio::test]
    async fn driver_pending_queue_full_records_skipped() {
        let install = "test-install";
        let scope = personal_turn_scope();
        let run_id_blocked = TurnRunId::new();
        let run_id_overflow = TurnRunId::new();

        // The coordinator will always return Completed, but since we hold the
        // pending permit the spawned task for run_id_blocked never proceeds.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Completed,
        ));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(coordinator, thread_service, egress, outbound, install);
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_secs(5),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(1).unwrap(),
        };
        let driver = TriggeredRunDeliveryDriver::with_settings(
            services,
            settings,
            delivery_store.clone(),
            route_store.clone(),
            scope.agent_id.clone().expect("test scope has agent"),
        );

        // Occupy the single pending slot directly so no real delivery task
        // consumes it.
        let _held = driver
            .try_acquire_pending_permit()
            .expect("pending slot must be available");

        // Now submit a trigger fire — the pending queue is full so it must skip.
        let fire = minimal_trigger_fire(None);
        driver
            .on_trigger_submitted(fire, run_id_overflow, scope.clone())
            .await;

        // The overflow run must be recorded as Skipped synchronously (no spawn).
        let record = wait_for_delivery_record(&delivery_store, run_id_overflow).await;
        assert_eq!(
            record.outcome,
            TriggeredRunDeliveryOutcomeKind::Skipped,
            "overflow submission must record Skipped when pending queue is full"
        );

        // The held permit keeps the slot occupied for the duration of this test;
        // drop it explicitly to document the intent.
        drop(_held);

        // The first run (run_id_blocked) was never submitted, so no record for it.
        assert!(
            delivery_store
                .load_triggered_run_delivery(run_id_blocked)
                .await
                .expect("load record")
                .is_none(),
            "run_id_blocked was never submitted so must have no delivery record"
        );
    }

    // ── Phase A: ack-feedback and delivery-error feedback tests ───────────────

    /// Build a minimal `SlackFinalReplyDeliveryObserver` for observer-path tests.
    fn make_observer(
        coordinator: Arc<dyn TurnCoordinator>,
        egress: Arc<FakeProtocolHttpEgress>,
        outbound: Arc<InMemoryOutboundStateStore>,
        installation_id: &str,
    ) -> SlackFinalReplyDeliveryObserver {
        use ironclaw_product_workflow::FakeConversationBindingService;

        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let services = SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(FakeConversationBindingService::new()),
            thread_service,
            turn_coordinator: coordinator,
            outbound_store: outbound.clone(),
            route_store: Arc::new(InMemoryDeliveredGateRouteStore::default()),
            communication_preferences: outbound,
            adapter: test_adapter(installation_id),
            egress,
            delivery_sink: Arc::new(FakeOutboundDeliverySink::default()),
            auth_challenges: None,
        };
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_millis(1),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        SlackFinalReplyDeliveryObserver::with_settings(services, settings)
    }

    fn rejected_ack(kind: ironclaw_product_adapters::ProductRejectionKind) -> ProductInboundAck {
        ProductInboundAck::Rejected(ironclaw_product_adapters::ProductRejection::permanent(
            kind,
            "internal reason",
        ))
    }

    fn scoped_approval_resolution_payload() -> ProductInboundPayload {
        ProductInboundPayload::ScopedApprovalResolution(
            ironclaw_product_adapters::ScopedApprovalResolutionPayload::new(
                ironclaw_product_adapters::ApprovalDecision::ApproveOnce,
            )
            .expect("scoped approval resolution"),
        )
    }

    fn approval_resolution_payload() -> ProductInboundPayload {
        ProductInboundPayload::ApprovalResolution(
            ironclaw_product_adapters::ApprovalResolutionPayload::new(
                "gate:approval-hint-test",
                ironclaw_product_adapters::ApprovalDecision::ApproveOnce,
            )
            .expect("approval resolution"),
        )
    }

    fn user_message_payload() -> ProductInboundPayload {
        ProductInboundPayload::UserMessage(
            ironclaw_product_adapters::UserMessagePayload::new(
                "hello",
                vec![],
                ironclaw_product_adapters::ProductTriggerReason::DirectChat,
            )
            .expect("user message"),
        )
    }

    /// Rejected scoped-approval ack → hint posted to the envelope conversation.
    #[tokio::test]
    async fn rejected_scoped_approval_ack_posts_hint_to_conversation() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Program a success response for the hint post.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "1000.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(scoped_approval_resolution_payload());
        let ack = rejected_ack(ironclaw_product_adapters::ProductRejectionKind::BindingRequired);

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert!(
            !post_calls.is_empty(),
            "expected hint chat.postMessage call"
        );

        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        // Hint text must contain "approve gate:" from BindingRequired hint.
        assert!(
            body.contains("approve gate:"),
            "rejection hint body must contain 'approve gate:', got: {body}"
        );
    }

    /// Rejected unscoped approval ack → hint posted to the envelope conversation.
    #[tokio::test]
    async fn rejected_approval_resolution_ack_posts_hint_to_conversation() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "1000.2"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(approval_resolution_payload());
        let ack = rejected_ack(ironclaw_product_adapters::ProductRejectionKind::BindingRequired);

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one hint chat.postMessage call"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains("approve gate:"),
            "rejection hint body must contain approval guidance, got: {body}"
        );
    }

    /// Rejected user-message payload → nothing posted.
    #[tokio::test]
    async fn rejected_user_message_ack_posts_nothing() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(user_message_payload());
        let ack = rejected_ack(ironclaw_product_adapters::ProductRejectionKind::BindingRequired);

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        assert!(
            !calls.iter().any(|c| c.path == "/api/chat.postMessage"),
            "no chat.postMessage expected for rejected user-message payload"
        );
    }

    // ── DeferredBusy ack feedback tests ───────────────────────────────────────

    fn deferred_busy_ack() -> ProductInboundAck {
        ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("slack:deferred").expect("ref"),
            active_run_id: TurnRunId::new(),
        }
    }

    /// DeferredBusy ack + UserMessage payload + BlockedApproval state with gate_ref →
    /// exactly one Slack post containing the concrete `approve gate:<ref>` command.
    ///
    /// The hint post is awaited inline; no yield needed before inspecting the egress capture.
    #[tokio::test]
    async fn deferred_busy_ack_with_user_message_posts_hint() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "2000.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // BlockedApproval with concrete gate_ref → hint embeds the actionable command.
        let gate_ref_str = "gate:approval-abc123";
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some(gate_ref_str),
        )]));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(user_message_payload());
        let ack = deferred_busy_ack();

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage for DeferredBusy + UserMessage"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains("waiting on a pending approval"),
            "deferred-busy hint must mention 'waiting on a pending approval', got: {body}"
        );
        assert!(
            body.contains(gate_ref_str),
            "deferred-busy approval hint must embed the concrete gate ref '{gate_ref_str}', got: {body}"
        );
    }

    /// DeferredBusy ack + non-UserMessage payload → no post (resolution payloads
    /// already have their own feedback path and must stay silent here).
    #[tokio::test]
    async fn deferred_busy_ack_with_resolution_payload_posts_nothing() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(scoped_approval_resolution_payload());
        let ack = deferred_busy_ack();

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        assert!(
            !calls.iter().any(|c| c.path == "/api/chat.postMessage"),
            "no chat.postMessage expected for DeferredBusy with non-UserMessage payload"
        );
    }

    /// Duplicate { prior: DeferredBusy } + UserMessage → nothing posted.
    ///
    /// `should_settle_ack` returns false for DeferredBusy, so the idempotency
    /// ledger never settles it. Slack transport retries re-process as a fresh
    /// plain DeferredBusy (never Duplicate{DeferredBusy}), making this case
    /// unreachable in practice. We still enforce None-for-all-Duplicate for
    /// safety, matching the invariant in `rejection_hint_for_resolution`.
    #[tokio::test]
    async fn duplicate_deferred_busy_with_user_message_posts_nothing() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(user_message_payload());
        let ack = ProductInboundAck::Duplicate {
            prior: Box::new(deferred_busy_ack()),
        };

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        assert!(
            !calls.iter().any(|c| c.path == "/api/chat.postMessage"),
            "Duplicate{{DeferredBusy}} must NOT post a hint (None-for-all-Duplicate invariant)"
        );
    }

    /// Two distinct plain DeferredBusy + UserMessage envelopes with different active_run_ids
    /// → two posts (throttle is per (conversation, run_id) pair).
    ///
    /// Each `deferred_busy_ack()` call creates a fresh `TurnRunId`, so the two acks have
    /// distinct throttle keys and each posts exactly one hint.
    #[tokio::test]
    async fn two_distinct_deferred_busy_user_messages_post_two_hints() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "2000.1"),
            )),
        );
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "2000.2"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // BlockedApproval so the state-aware lookup returns the approval copy for both.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);

        // First distinct user message while blocked (fresh active_run_id in each ack).
        observer
            .observe_workflow_ack(envelope(user_message_payload()), deferred_busy_ack())
            .await;
        // Second distinct user message while blocked (different active_run_id → different key).
        observer
            .observe_workflow_ack(envelope(user_message_payload()), deferred_busy_ack())
            .await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            2,
            "two distinct DeferredBusy + UserMessage deliveries must each post a hint"
        );
    }

    /// DeferredBusy + UserMessage + BlockedAuth state with gate_ref → auth copy with
    /// concrete `auth deny <ref>` command posted.
    #[tokio::test]
    async fn deferred_busy_blocked_auth_state_posts_auth_hint() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "6000.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // BlockedAuth with concrete gate_ref → hint embeds `auth deny <ref>`.
        let gate_ref_str = "gate:auth-slack-hint";
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some(gate_ref_str),
        )]));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(user_message_payload());
        let ack = deferred_busy_ack();

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage for DeferredBusy + BlockedAuth"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains("authentication step"),
            "deferred-busy hint for BlockedAuth must mention 'authentication step', got: {body}"
        );
        assert!(
            body.contains(gate_ref_str),
            "deferred-busy auth hint must embed the concrete gate ref '{gate_ref_str}', got: {body}"
        );
        // Must not contain approval command wording.
        assert!(
            !body.contains("approve") || body.contains("auth deny"),
            "auth hint must not contain approval-only wording, got: {body}"
        );
    }

    /// DeferredBusy + UserMessage + BlockedApproval with no gate_ref → fallback wording
    /// without a specific gate command.
    #[tokio::test]
    async fn deferred_busy_blocked_approval_no_gate_ref_posts_fallback_hint() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "6001.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // BlockedApproval with no gate_ref → static fallback.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedApproval,
            None,
        )]));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(user_message_payload());
        let ack = deferred_busy_ack();

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected one post for fallback approval hint"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains("waiting on a pending approval"),
            "fallback approval hint must still mention pending approval, got: {body}"
        );
    }

    /// DeferredBusy + UserMessage + Running state (non-blocked) → generic copy posted.
    #[tokio::test]
    async fn deferred_busy_running_state_posts_generic_hint() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "6000.2"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // Running → state-aware lookup returns generic wording.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(user_message_payload());
        let ack = deferred_busy_ack();

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage for DeferredBusy + Running"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains("still working on a previous message"),
            "deferred-busy hint for Running state must contain generic copy, got: {body}"
        );
    }

    /// DeferredBusy + UserMessage + unauthorized binding → no post, no panic.
    ///
    /// Uses `TestNoopConversationBindingService` (always fails lookup_binding) to
    /// simulate an unauthorized conversation: the observer must skip the hint post
    /// silently rather than posting to an arbitrary channel.
    #[tokio::test]
    async fn deferred_busy_unauthorized_binding_posts_nothing() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));
        // Override the default `make_observer` so we can inject the no-binding service.
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let services = SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(TestNoopConversationBindingService),
            thread_service,
            turn_coordinator: coordinator,
            outbound_store: outbound.clone(),
            route_store: Arc::new(InMemoryDeliveredGateRouteStore::default()),
            communication_preferences: outbound,
            adapter: test_adapter(install),
            egress: egress.clone(),
            delivery_sink: Arc::new(FakeOutboundDeliverySink::default()),
            auth_challenges: None,
        };
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_millis(1),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let observer = SlackFinalReplyDeliveryObserver::with_settings(services, settings);

        let env = envelope(user_message_payload());
        let ack = deferred_busy_ack();

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        assert!(
            !calls.iter().any(|c| c.path == "/api/chat.postMessage"),
            "no chat.postMessage expected when binding authorization fails"
        );
    }

    /// Duplicate { prior: Accepted } → nothing posted (already succeeded).
    #[tokio::test]
    async fn duplicate_accepted_ack_posts_nothing() {
        let install = "test-install";
        let run_id = TurnRunId::new();
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // The coordinator is a no-op because Duplicate{Accepted} has no submitted_run_id.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(scoped_approval_resolution_payload());
        let prior = ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("slack:prior").expect("ref"),
            submitted_run_id: run_id,
        };
        let ack = ProductInboundAck::Duplicate {
            prior: Box::new(prior),
        };

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        assert!(
            !calls.iter().any(|c| c.path == "/api/chat.postMessage"),
            "no chat.postMessage expected for Duplicate{{Accepted}}"
        );
    }

    /// Duplicate { prior: Rejected } → nothing posted at observer level.
    #[tokio::test]
    async fn duplicate_rejected_ack_posts_nothing() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(scoped_approval_resolution_payload());
        let ack = ProductInboundAck::Duplicate {
            prior: Box::new(rejected_ack(
                ironclaw_product_adapters::ProductRejectionKind::BindingRequired,
            )),
        };

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        assert!(
            !calls.iter().any(|c| c.path == "/api/chat.postMessage"),
            "no chat.postMessage expected for Duplicate{{Rejected}}"
        );
    }

    /// All `Duplicate` acks suppress the hint, regardless of the prior inside.
    ///
    /// `Duplicate` is keyed on the external event id: transport retries of the
    /// same Slack event land as `Duplicate{original}`. The original processing
    /// already posted any hint, so replays must not repeat the side effect.
    /// A user re-typing "approve" produces a new event id → a fresh `Rejected`,
    /// never a `Duplicate`, so suppressing `Duplicate{Rejected}` loses nothing.
    #[test]
    fn duplicate_acks_produce_no_hint() {
        let env = envelope(scoped_approval_resolution_payload());

        let duplicate_rejected = ProductInboundAck::Duplicate {
            prior: Box::new(rejected_ack(
                ironclaw_product_adapters::ProductRejectionKind::BindingRequired,
            )),
        };
        assert!(
            rejection_hint_for_resolution(&env, &duplicate_rejected).is_none(),
            "Duplicate{{Rejected}} must NOT produce a hint (transport replay)"
        );

        let duplicate_accepted = ProductInboundAck::Duplicate {
            prior: Box::new(ProductInboundAck::Accepted {
                accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("slack:prior")
                    .expect("ref"),
                submitted_run_id: ironclaw_turns::TurnRunId::new(),
            }),
        };
        assert!(
            rejection_hint_for_resolution(&env, &duplicate_accepted).is_none(),
            "Duplicate{{Accepted}} must not produce a hint"
        );
    }

    /// A failed best-effort rejection-hint post must not fall through into generic
    /// delivery-error feedback.
    #[tokio::test]
    async fn rejected_resolution_hint_post_failure_is_best_effort() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(scoped_approval_resolution_payload());
        let ack = rejected_ack(ironclaw_product_adapters::ProductRejectionKind::BindingRequired);

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one failed best-effort hint post"
        );
    }

    /// Delivery error (RunWaitTimedOut) → timeout notice posted to conversation.
    ///
    /// Uses `FakeConversationBindingService` so the binding lookup succeeds and
    /// delivery enters the polling loop, which then times out because the
    /// coordinator always returns `Running`.
    #[tokio::test]
    async fn delivery_timeout_posts_timeout_notice_to_conversation() {
        use ironclaw_product_workflow::FakeConversationBindingService;

        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Two slots: one for any working-message post, one for the timeout notice.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "2000.1"),
            )),
        );
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "2000.2"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // Always Running → wait_for_actionable times out after max_wait=1ms.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        // Use FakeConversationBindingService so the binding lookup succeeds and
        // the delivery loop can actually reach the timeout.
        let binding_service = Arc::new(FakeConversationBindingService::new());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let services = SlackFinalReplyDeliveryServices {
            binding_service,
            thread_service,
            turn_coordinator: coordinator,
            outbound_store: outbound.clone(),
            route_store: Arc::new(InMemoryDeliveredGateRouteStore::default()),
            communication_preferences: outbound,
            adapter: test_adapter(install),
            egress: egress.clone(),
            delivery_sink: Arc::new(FakeOutboundDeliverySink::default()),
            auth_challenges: None,
        };
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_millis(1),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let observer = SlackFinalReplyDeliveryObserver::with_settings(services, settings);

        // Accepted ack for a user message so deliver_final_reply enters the
        // polling loop and hits the timeout.
        let run_id = TurnRunId::new();
        let env = envelope(user_message_payload());
        let ack = ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("slack:timeout-test").expect("ref"),
            submitted_run_id: run_id,
        };

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert!(
            !post_calls.is_empty(),
            "expected at least one chat.postMessage for timeout notice"
        );

        // The timeout notice must be the final post — a working message may
        // precede it, but nothing should be posted after the timeout notice.
        let last_body = std::str::from_utf8(&post_calls[post_calls.len() - 1].body).unwrap_or("");
        assert!(
            last_body.contains("longer than expected"),
            "last chat.postMessage must contain timeout notice text, bodies: {:?}",
            post_calls
                .iter()
                .map(|c| std::str::from_utf8(&c.body).unwrap_or("?"))
                .collect::<Vec<_>>()
        );
    }

    /// Accepted ack, then binding lookup fails → generic delivery-error notice
    /// posted to the conversation (A3). Drives the observer (the caller), not
    /// just `deliver_final_reply`, so the error→feedback mapping in
    /// `observe_workflow_ack` is covered.
    #[tokio::test]
    async fn accepted_ack_then_binding_error_posts_delivery_error_notice() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "3000.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let services = SlackFinalReplyDeliveryServices {
            // Errors on lookup_binding, so delivery fails after the Accepted
            // ack and before any polling.
            binding_service: Arc::new(TestNoopConversationBindingService),
            thread_service: Arc::new(InMemorySessionThreadService::default()),
            turn_coordinator: coordinator,
            outbound_store: outbound.clone(),
            route_store: Arc::new(InMemoryDeliveredGateRouteStore::default()),
            communication_preferences: outbound,
            adapter: test_adapter(install),
            egress: egress.clone(),
            delivery_sink: Arc::new(FakeOutboundDeliverySink::default()),
            auth_challenges: None,
        };
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_millis(1),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let observer = SlackFinalReplyDeliveryObserver::with_settings(services, settings);

        let env = envelope(user_message_payload());
        let ack = ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("slack:binding-error-test").expect("ref"),
            submitted_run_id: TurnRunId::new(),
        };

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage (the delivery-error notice), bodies: {:?}",
            post_calls
                .iter()
                .map(|c| std::str::from_utf8(&c.body).unwrap_or("?"))
                .collect::<Vec<_>>()
        );
        let body = std::str::from_utf8(&post_calls[0].body).unwrap_or("");
        assert!(
            body.contains("Something went wrong delivering the result"),
            "post must contain the generic delivery-error notice, body: {body}"
        );
    }

    /// Rejected AuthResolution ack → auth-flavored hint posted; approval
    /// command text and internal rejection reason must not appear.
    ///
    /// This is the caller-level regression for `rejection_hint_for_resolution`
    /// covering the `ProductInboundPayload::AuthResolution(_)` branch: the hint
    /// must come from `user_facing_auth_hint()` (which references `auth deny
    /// <auth-request-ref>`), not from `user_facing_hint()` (which references
    /// approval commands), and not from the raw internal reason.
    #[tokio::test]
    async fn rejected_auth_resolution_ack_posts_static_hint_not_internal_reason() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Program a success response for the hint post.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "4000.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);

        // Build an AuthResolution envelope — this payload kind is included in
        // `rejection_hint_for_resolution`'s `is_resolution` match but had no
        // caller-level test before this one.
        let auth_resolution_payload = ProductInboundPayload::AuthResolution(
            ironclaw_product_adapters::AuthResolutionPayload::new(
                "gate:auth-hint-test",
                ironclaw_product_adapters::AuthResolutionResult::Denied,
            )
            .expect("auth resolution payload"),
        );
        let env = envelope(auth_resolution_payload);

        // Use a rejection with a distinctive internal reason that must NOT appear
        // in the posted message.
        let internal_marker = "internal-secret-reason-marker";
        let ack =
            ProductInboundAck::Rejected(ironclaw_product_adapters::ProductRejection::permanent(
                ironclaw_product_adapters::ProductRejectionKind::BindingRequired,
                internal_marker,
            ));

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage (the hint), bodies: {:?}",
            post_calls
                .iter()
                .map(|c| std::str::from_utf8(&c.body).unwrap_or("?"))
                .collect::<Vec<_>>()
        );

        let body = std::str::from_utf8(&post_calls[0].body).unwrap_or("");

        // The posted text must contain the auth-specific hint for BindingRequired,
        // not the approval-command variant.
        let expected_hint = ironclaw_product_adapters::ProductRejectionKind::BindingRequired
            .user_facing_auth_hint();
        assert!(
            body.contains(expected_hint),
            "post must contain the auth-flavored hint '{expected_hint}', body: {body}"
        );

        // The approval command must NOT appear in an auth-resolution hint.
        assert!(
            !body.contains("approve gate:"),
            "post must not contain approval command 'approve gate:', body: {body}"
        );

        // The internal rejection reason must NOT appear in the post.
        assert!(
            !body.contains(internal_marker),
            "post must not contain the internal rejection reason '{internal_marker}', body: {body}"
        );
    }

    /// WorkflowRejected errors after protocol ACK still post resolution hints when
    /// the originating conversation is authorized.
    #[tokio::test]
    async fn workflow_rejected_resolution_error_posts_authorized_hint() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "4500.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(scoped_approval_resolution_payload());
        let error = ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            status_code: 404,
            retryable: false,
            reason: ironclaw_product_adapters::RedactedString::new("missing gate"),
        };

        observer.observe_workflow_error(env, error).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one hint chat.postMessage call"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains("approve gate:"),
            "workflow rejection hint body must contain approval guidance, got: {body}"
        );
        assert!(
            !body.contains("missing gate"),
            "workflow rejection hint must not expose redacted reason, got: {body}"
        );
    }

    /// If route/binding authorization fails, rejected-resolution feedback is
    /// suppressed instead of posting to an arbitrary shared Slack conversation.
    #[tokio::test]
    async fn workflow_rejected_resolution_error_without_binding_posts_nothing() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_millis(1),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let observer = SlackFinalReplyDeliveryObserver::with_settings(services, settings);
        let env = envelope(scoped_approval_resolution_payload());
        let error = ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            status_code: 404,
            retryable: false,
            reason: ironclaw_product_adapters::RedactedString::new("missing gate"),
        };

        observer.observe_workflow_error(env, error).await;

        let calls = egress.calls();
        assert!(
            !calls.iter().any(|c| c.path == "/api/chat.postMessage"),
            "no chat.postMessage expected when binding authorization fails"
        );
    }

    /// When a blocked-state notification (approval prompt) was delivered and
    /// the subsequent wait times out, no additional timeout notice must be
    /// posted to Slack.
    ///
    /// This is the caller-level regression for the
    /// `RunWaitTimedOutAfterNotification` error variant: `observe_workflow_ack`
    /// maps this variant to `feedback = None` so the user is not double-notified
    /// after already seeing the approval prompt.
    #[tokio::test]
    async fn timeout_after_blocked_notification_suppresses_timeout_message() {
        use ironclaw_product_workflow::FakeConversationBindingService;

        let install = "test-install";
        let gate_ref_str = "gate:approval-timeout-test";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // One programmed response for the approval-prompt postMessage.
        // No second response — if the timeout notice were posted, the test
        // would fail because `FakeProtocolHttpEgress` returns an error on
        // an empty queue.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "5000.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // Always BlockedApproval with the same gate_ref: the first poll exits
        // `wait_for_actionable` immediately (new blocked state, different from
        // `delivered_blocked_marker=None`), delivering the approval prompt.
        // Subsequent polls (second call to `wait_for_actionable`) return the same
        // marker as `delivered_blocked_marker`, so the loop does not exit — it
        // times out after `max_wait=1ms` and the error is converted to
        // `RunWaitTimedOutAfterNotification`.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some(gate_ref_str),
        )]));

        let binding_service = Arc::new(FakeConversationBindingService::new());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let services = SlackFinalReplyDeliveryServices {
            binding_service,
            thread_service,
            turn_coordinator: coordinator,
            outbound_store: outbound.clone(),
            route_store: Arc::new(InMemoryDeliveredGateRouteStore::default()),
            communication_preferences: outbound,
            adapter: test_adapter(install),
            egress: egress.clone(),
            delivery_sink: Arc::new(FakeOutboundDeliverySink::default()),
            auth_challenges: None,
        };
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_millis(1),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let observer = SlackFinalReplyDeliveryObserver::with_settings(services, settings);

        let run_id = TurnRunId::new();
        let env = envelope(user_message_payload());
        let ack = ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("slack:blocked-timeout-test")
                .expect("ref"),
            submitted_run_id: run_id,
        };

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();

        // Exactly one postMessage: the approval prompt notification.
        // No timeout notice must have been posted.
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage (the approval prompt), bodies: {:?}",
            post_calls
                .iter()
                .map(|c| std::str::from_utf8(&c.body).unwrap_or("?"))
                .collect::<Vec<_>>()
        );

        let body = std::str::from_utf8(&post_calls[0].body).unwrap_or("");

        // The one message must be the approval prompt, not a timeout notice.
        assert!(
            !body.contains("longer than expected"),
            "timeout notice must not be posted after blocked-notification timeout, body: {body}"
        );
        // The approval prompt must reference the gate ref.
        assert!(
            body.contains(gate_ref_str),
            "approval prompt must reference the gate ref, body: {body}"
        );
    }

    /// Same suppression as the approval-prompt case, but through the auth prompt
    /// payload path.
    #[tokio::test]
    async fn timeout_after_auth_blocked_notification_suppresses_timeout_message() {
        use ironclaw_product_workflow::FakeConversationBindingService;

        let install = "test-install";
        let gate_ref_str = "gate:auth-timeout-test";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "5000.2"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some(gate_ref_str),
        )]));

        let binding_service = Arc::new(FakeConversationBindingService::new());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let services = SlackFinalReplyDeliveryServices {
            binding_service,
            thread_service,
            turn_coordinator: coordinator,
            outbound_store: outbound.clone(),
            route_store: Arc::new(InMemoryDeliveredGateRouteStore::default()),
            communication_preferences: outbound,
            adapter: test_adapter(install),
            egress: egress.clone(),
            delivery_sink: Arc::new(FakeOutboundDeliverySink::default()),
            auth_challenges: None,
        };
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_millis(1),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let observer = SlackFinalReplyDeliveryObserver::with_settings(services, settings);

        let env = envelope(user_message_payload());
        let ack = ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("slack:auth-blocked-timeout-test")
                .expect("ref"),
            submitted_run_id: TurnRunId::new(),
        };

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage (the auth prompt), bodies: {:?}",
            post_calls
                .iter()
                .map(|c| std::str::from_utf8(&c.body).unwrap_or("?"))
                .collect::<Vec<_>>()
        );
        let body = std::str::from_utf8(&post_calls[0].body).unwrap_or("");
        assert!(
            !body.contains("longer than expected"),
            "timeout notice must not be posted after auth notification timeout, body: {body}"
        );
        assert!(
            body.contains(gate_ref_str),
            "auth prompt must reference the gate ref, body: {body}"
        );
    }

    // --- OnceLock slot behaviour tests ----------------------------------------

    #[tokio::test]
    async fn post_submit_hook_slot_empty_hook_does_not_fire() {
        // Verify the contract that `PostSubmitHookWrappedSubmitter` relies on:
        // when the OnceLock slot is empty, reading it returns None and no hook fires.
        let slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>> = Arc::new(OnceLock::new());

        // Slot is empty — `get()` returns None, so no hook fires.
        assert!(slot.get().is_none(), "empty slot must return None");

        // When the slot IS occupied the hook IS reachable.
        let fired = Arc::new(Mutex::new(false));
        let fired_clone = Arc::clone(&fired);
        struct FlagHook(Arc<Mutex<bool>>);
        #[async_trait]
        impl PostSubmitDeliveryHook for FlagHook {
            async fn on_trigger_submitted(
                &self,
                _fire: TriggerFire,
                _run_id: TurnRunId,
                _scope: TurnScope,
            ) {
                *self.0.lock().expect("flag") = true;
            }
        }
        slot.set(Arc::new(FlagHook(fired_clone)))
            .unwrap_or_else(|_| panic!("first slot set should succeed"));
        if let Some(hook) = slot.get() {
            let fire = minimal_trigger_fire(None);
            hook.on_trigger_submitted(fire, TurnRunId::new(), personal_turn_scope())
                .await;
        }
        assert!(
            *fired.lock().expect("flag"),
            "hook must fire after slot is set"
        );
    }

    #[test]
    fn post_submit_hook_slot_second_set_is_noop() {
        // OnceLock semantics: first set succeeds, second set returns the value.
        // This is the contract behind `set_trigger_post_submit_hook` returning false
        // on duplicate calls.
        let slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>> = Arc::new(OnceLock::new());
        let hook_a: Arc<dyn PostSubmitDeliveryHook> = Arc::new(NoopPostSubmitDeliveryHook);
        let hook_b: Arc<dyn PostSubmitDeliveryHook> = Arc::new(NoopPostSubmitDeliveryHook);

        assert!(slot.set(hook_a).is_ok(), "first set should succeed");
        assert!(
            slot.set(hook_b).is_err(),
            "second set should fail (slot already occupied)"
        );
        assert!(
            slot.get().is_some(),
            "slot still occupied after duplicate set"
        );
    }

    #[test]
    fn slack_approval_prompt_offers_always_for_typed_approval_gate() {
        let gate_ref = GateRef::new(format!(
            "gate:approval-{}",
            ironclaw_host_api::ApprovalRequestId::new()
        ))
        .expect("gate ref");

        let prompt = slack_approval_gate_prompt_view(TurnRunId::new(), &gate_ref);

        assert_eq!(prompt.gate_ref, gate_ref.as_str());
        assert!(prompt.allow_always);
    }

    #[test]
    fn slack_approval_prompt_does_not_offer_always_for_generic_gate() {
        let gate_ref = GateRef::new("gate:approve-slack").expect("gate ref");

        let prompt = slack_approval_gate_prompt_view(TurnRunId::new(), &gate_ref);

        assert_eq!(prompt.gate_ref, gate_ref.as_str());
        assert!(!prompt.allow_always);
    }

    // --- Bug-fix regression tests: gate-route refs carry team id (space_id) ----

    /// Test A: triggered approval delivery records a gate-route that includes a
    /// posted-message ref whose fingerprint matches an inbound-style ref carrying
    /// the Slack team id (space_id = "T123").
    ///
    /// The test_slack_binding_ref helper encodes space = "T123", conversation = "D456".
    /// After delivery the authority's `resolved_space_id` must be Some("T123"),
    /// and the recorded route must contain a ref with space_id = Some("T123"),
    /// conversation_id = the channel returned by Slack ("D456"), and thread_id =
    /// the ts of the posted message ("1111.2222").
    #[tokio::test]
    async fn triggered_approval_route_ref_carries_resolved_space_id() {
        let install = "test-install";
        let gate_ref_str = "gate:approval-space-test";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        // First poll → BlockedApproval; second poll → Completed.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![
            scripted_state(TurnStatus::BlockedApproval, Some(gate_ref_str)),
            scripted_state(TurnStatus::Completed, None),
        ]));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_finalized_assistant_message(
            &thread_service,
            &scope,
            run_id,
            "Run complete after approval.",
        )
        .await;

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Approval-prompt response: channel D456, ts 1111.2222.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "1111.2222"),
            )),
        );
        // Final-reply response.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "3333.4444"),
            )),
        );

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_secs(5),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let driver = TriggeredRunDeliveryDriver::with_settings(
            services,
            settings,
            delivery_store.clone(),
            route_store.clone(),
            scope.agent_id.clone().expect("test scope has agent"),
        );

        let fire = minimal_trigger_fire(None);
        driver
            .on_trigger_submitted(fire, run_id, scope.clone())
            .await;
        wait_for_delivery_record(&delivery_store, run_id).await;

        let creator = ironclaw_host_api::UserId::new("creator-user").expect("user id");
        let route = route_store
            .load_delivered_gate_route(&scope.tenant_id, &creator, gate_ref_str)
            .await
            .expect("load route")
            .expect("gate route was recorded");

        // The binding ref encodes space = "T123", so the recorded route must
        // contain a ref that fingerprint-matches an inbound-style ref with
        // space_id = Some("T123"), conversation_id = "D456", thread_id = "1111.2222".
        let expected_inbound_ref = ironclaw_conversations::ExternalConversationRef::new(
            Some("T123"),
            "D456",
            Some("1111.2222"),
            None,
        )
        .expect("expected inbound ref");
        let expected_fingerprint = expected_inbound_ref.conversation_fingerprint();

        assert!(
            route
                .delivered_conversation_fingerprints
                .iter()
                .any(|fingerprint| fingerprint == &expected_fingerprint),
            "recorded route must include space_id=T123 fingerprint; fingerprints={:?}",
            route.delivered_conversation_fingerprints,
        );
    }

    /// Test B: triggered auth (BlockedAuth) delivery records a gate-route keyed
    /// by the auth gate_ref. This is the Bug-2 regression: previously
    /// `gate_ref_for_routing` was `None` for BlockedAuth so no route was
    /// recorded, causing a `MissingGate` when the user replied "approve".
    #[tokio::test]
    async fn triggered_auth_delivery_records_gate_route() {
        let install = "test-install";
        let gate_ref_str = "gate:auth-route-regression";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        // First poll → BlockedAuth with gate_ref; second poll → Completed.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![
            scripted_state(TurnStatus::BlockedAuth, Some(gate_ref_str)),
            scripted_state(TurnStatus::Completed, None),
        ]));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_finalized_assistant_message(
            &thread_service,
            &scope,
            run_id,
            "Run complete after auth.",
        )
        .await;

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Auth-prompt delivery response.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "9999.1111"),
            )),
        );
        // Final-reply response (after Completed).
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "9999.2222"),
            )),
        );
        // Auth message is deleted after final; need a delete response.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                serde_json::json!({"ok": true}).to_string().into_bytes(),
            )),
        );

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_secs(5),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let driver = TriggeredRunDeliveryDriver::with_settings(
            services,
            settings,
            delivery_store.clone(),
            route_store.clone(),
            scope.agent_id.clone().expect("test scope has agent"),
        );

        let fire = minimal_trigger_fire(None);
        driver
            .on_trigger_submitted(fire, run_id, scope.clone())
            .await;
        wait_for_delivery_record(&delivery_store, run_id).await;

        let creator = ironclaw_host_api::UserId::new("creator-user").expect("user id");
        // A gate route for the auth gate_ref must have been recorded.
        let route = route_store
            .load_delivered_gate_route(&scope.tenant_id, &creator, gate_ref_str)
            .await
            .expect("load route")
            .expect("auth gate route must be recorded (Bug 2 regression)");

        assert_eq!(
            route.gate_ref, gate_ref_str,
            "recorded route must reference the auth gate_ref"
        );
        assert_eq!(route.run_id, run_id);
    }

    /// Test C: the `record_gate_route_if_needed` helper, called from the
    /// observer path, stores a posted-message ref whose fingerprint matches an
    /// inbound-style ref that carries the envelope's team id (space_id).
    ///
    /// This tests the observer call site directly (without driving the full
    /// observer loop) to verify that `envelope_space_id` is extracted and
    /// passed through.
    #[tokio::test]
    async fn observer_approval_route_ref_carries_envelope_space_id() {
        let tenant_id = ironclaw_host_api::TenantId::new("test-tenant").expect("tenant");
        let user_id = ironclaw_host_api::UserId::new("user-obs").expect("user");
        let run_id = TurnRunId::new();
        let gate_ref_str = "gate:observer-space-test";
        let agent = ironclaw_host_api::AgentId::new("obs-agent").expect("agent");
        let thread = ironclaw_host_api::ThreadId::new("obs-thread").expect("thread");
        let scope = TurnScope::new_with_owner(tenant_id.clone(), Some(agent), None, thread, None);

        // Simulate a posted message: channel D789, ts 5555.6666.
        let posted = vec![PostedSlackMessage {
            channel: "D789".to_string(),
            ts: "5555.6666".to_string(),
        }];

        // Envelope conv ref carries space_id = "T999" (the team id).
        let envelope_conv_ref =
            ExternalConversationRef::new(Some("T999"), "D789", Some("5555.6666"), None)
                .expect("envelope conv ref");

        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());

        // Derive space_id from the envelope ref — mirrors the observer call site.
        let envelope_space_id = conversations_ref_from_product_ref(&envelope_conv_ref)
            .ok()
            .and_then(|r| r.space_id().map(str::to_string));
        assert_eq!(
            envelope_space_id.as_deref(),
            Some("T999"),
            "space_id must be extracted from envelope ref"
        );

        record_gate_route_if_needed(
            route_store.as_ref(),
            run_id,
            &tenant_id,
            &user_id,
            gate_ref_str,
            &scope,
            &posted,
            Some(&envelope_conv_ref),
            envelope_space_id.as_deref(),
        )
        .await;

        let route = route_store
            .load_delivered_gate_route(&tenant_id, &user_id, gate_ref_str)
            .await
            .expect("load route")
            .expect("route was recorded");

        // Must contain a ref with space_id = "T999" matching the inbound fingerprint.
        let expected_inbound_ref = ironclaw_conversations::ExternalConversationRef::new(
            Some("T999"),
            "D789",
            Some("5555.6666"),
            None,
        )
        .expect("inbound ref");
        let expected_fingerprint = expected_inbound_ref.conversation_fingerprint();

        assert!(
            route
                .delivered_conversation_fingerprints
                .iter()
                .any(|fingerprint| fingerprint == &expected_fingerprint),
            "recorded route must include space_id=T999 fingerprint; fingerprints={:?}",
            route.delivered_conversation_fingerprints,
        );

        // Also verify that the no-space fallback variant is present (inbound
        // events without team_id must still match).
        let no_space_ref = ironclaw_conversations::ExternalConversationRef::new(
            None,
            "D789",
            Some("5555.6666"),
            None,
        )
        .expect("no-space ref");
        let no_space_fingerprint = no_space_ref.conversation_fingerprint();
        assert!(
            route
                .delivered_conversation_fingerprints
                .iter()
                .any(|fingerprint| fingerprint == &no_space_fingerprint),
            "recorded route must include the no-space fallback fingerprint; fingerprints={:?}",
            route.delivered_conversation_fingerprints,
        );

        let channel_root_ref =
            ironclaw_conversations::ExternalConversationRef::new(Some("T999"), "D789", None, None)
                .expect("channel root ref");
        let channel_root_fingerprint = channel_root_ref.conversation_fingerprint();
        assert!(
            route
                .delivered_conversation_fingerprints
                .iter()
                .any(|fingerprint| fingerprint == &channel_root_fingerprint),
            "recorded route must include the space-qualified channel-root fingerprint for bare replies; fingerprints={:?}",
            route.delivered_conversation_fingerprints,
        );

        let no_space_channel_root_ref =
            ironclaw_conversations::ExternalConversationRef::new(None, "D789", None, None)
                .expect("no-space channel root ref");
        let no_space_channel_root_fingerprint =
            no_space_channel_root_ref.conversation_fingerprint();
        assert!(
            route
                .delivered_conversation_fingerprints
                .iter()
                .any(|fingerprint| fingerprint == &no_space_channel_root_fingerprint),
            "recorded route must include the no-space channel-root fingerprint for bare replies; fingerprints={:?}",
            route.delivered_conversation_fingerprints,
        );
    }

    // ── Extra DeferredBusy coverage tests ─────────────────────────────────────

    /// Binding service that always returns a binding with `agent_id = None`.
    ///
    /// Used to exercise the scope-derivation fallback in
    /// `deferred_busy_hint_from_run_state`: when `thread_scope_from_binding` fails
    /// because `agent_id` is missing, the hint must still be posted using the
    /// generic copy rather than being silently dropped.
    struct NoAgentConversationBindingService;

    #[async_trait]
    impl ConversationBindingService for NoAgentConversationBindingService {
        async fn resolve_binding(
            &self,
            request: ResolveBindingRequest,
        ) -> Result<ResolvedBinding, ProductWorkflowError> {
            Ok(ResolvedBinding {
                tenant_id: ironclaw_host_api::TenantId::new("tenant:test").expect("tenant"),
                actor_user_id: ironclaw_host_api::UserId::new(format!(
                    "user:{}",
                    request.external_actor_ref.id()
                ))
                .expect("user"),
                subject_user_id: None,
                thread_id: ironclaw_host_api::ThreadId::new("thread:test").expect("thread"),
                agent_id: None, // deliberately no agent — triggers scope derivation failure
                project_id: None,
            })
        }

        async fn lookup_binding(
            &self,
            request: ResolveBindingRequest,
        ) -> Result<ResolvedBinding, ProductWorkflowError> {
            self.resolve_binding(request).await
        }
    }

    /// A `TurnCoordinator` double whose `get_run_state` always returns `Err`.
    struct ErroringTurnCoordinator;

    #[async_trait]
    impl TurnCoordinator for ErroringTurnCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            Err(TurnError::Unavailable {
                reason: "ErroringTurnCoordinator".to_string(),
            })
        }

        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "ErroringTurnCoordinator".to_string(),
            })
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "ErroringTurnCoordinator".to_string(),
            })
        }

        async fn retry_turn(
            &self,
            _request: RetryTurnRequest,
        ) -> Result<RetryTurnResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "ErroringTurnCoordinator".to_string(),
            })
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "simulated run-state lookup failure".to_string(),
            })
        }

        async fn cancel_run(
            &self,
            _request: ironclaw_turns::CancelRunRequest,
        ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "ErroringTurnCoordinator".to_string(),
            })
        }
    }

    /// Binding with no `agent_id` → scope derivation fails → generic copy posted.
    ///
    /// `deferred_busy_hint_from_run_state` calls `thread_scope_from_binding` which
    /// returns `Err` when `agent_id` is `None`. The code must fall back to
    /// `SLACK_DEFERRED_BUSY_GENERIC_MESSAGE` and still post the hint.
    #[tokio::test]
    async fn deferred_busy_missing_agent_binding_posts_generic_hint() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "7000.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));

        // Wire the no-agent binding service directly.
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let services = SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(NoAgentConversationBindingService),
            thread_service,
            turn_coordinator: coordinator,
            outbound_store: outbound.clone(),
            route_store: Arc::new(InMemoryDeliveredGateRouteStore::default()),
            communication_preferences: outbound,
            adapter: test_adapter(install),
            egress: egress.clone(),
            delivery_sink: Arc::new(FakeOutboundDeliverySink::default()),
            auth_challenges: None,
        };
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_millis(1),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let observer = SlackFinalReplyDeliveryObserver::with_settings(services, settings);

        let env = envelope(user_message_payload());
        let ack = deferred_busy_ack();

        observer.observe_workflow_ack(env, ack).await;
        tokio::task::yield_now().await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage even when agent_id is missing"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains("still working on a previous message"),
            "hint must fall back to generic copy when scope derivation fails, got: {body}"
        );
    }

    /// Run-state lookup returns `Err` → generic copy posted.
    ///
    /// `deferred_busy_hint_from_run_state` swallows `TurnError` from
    /// `get_run_state` and degrades to `SLACK_DEFERRED_BUSY_GENERIC_MESSAGE`.
    #[tokio::test]
    async fn deferred_busy_run_state_lookup_error_posts_generic_hint() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "7000.2"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());

        use ironclaw_product_workflow::FakeConversationBindingService;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let services = SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(FakeConversationBindingService::new()),
            thread_service,
            // ErroringTurnCoordinator: get_run_state always returns Err.
            turn_coordinator: Arc::new(ErroringTurnCoordinator),
            outbound_store: outbound.clone(),
            route_store: Arc::new(InMemoryDeliveredGateRouteStore::default()),
            communication_preferences: outbound,
            adapter: test_adapter(install),
            egress: egress.clone(),
            delivery_sink: Arc::new(FakeOutboundDeliverySink::default()),
            auth_challenges: None,
        };
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_millis(1),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let observer = SlackFinalReplyDeliveryObserver::with_settings(services, settings);

        let env = envelope(user_message_payload());
        let ack = deferred_busy_ack();

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage even when run-state lookup fails"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains("still working on a previous message"),
            "hint must fall back to generic copy when run-state lookup fails, got: {body}"
        );
    }

    /// Slack post failure → no panic, ack path unaffected.
    ///
    /// The post is best-effort; a transport error must be swallowed with debug!
    /// and the observer must return normally.
    #[tokio::test]
    async fn deferred_busy_post_failure_is_best_effort() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Program a transport failure for the hint post.
        egress.program_response("slack.com", Err(ProtocolHttpEgressError::Timeout));

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);

        let env = envelope(user_message_payload());
        let ack = deferred_busy_ack();

        // Must not panic regardless of the egress failure.
        observer.observe_workflow_ack(env, ack).await;

        // The call was recorded even though the programmed result was an error.
        let calls = egress.calls();
        assert!(
            calls.iter().any(|c| c.path == "/api/chat.postMessage"),
            "egress must have been called even when the hint post fails (best-effort)"
        );
    }

    /// Two DeferredBusy acks with the same conversation + same active_run_id
    /// → exactly one post (throttle suppresses the second).
    #[tokio::test]
    async fn deferred_busy_same_conversation_same_run_id_posts_once() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "9001.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let run_id = TurnRunId::new();
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);

        let make_ack = || ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("slack:deferred-same").expect("ref"),
            active_run_id: run_id,
        };

        // First delivery: posts.
        observer
            .observe_workflow_ack(envelope(user_message_payload()), make_ack())
            .await;
        // Second delivery: same (conversation, run_id) pair → throttled, no second post.
        observer
            .observe_workflow_ack(envelope(user_message_payload()), make_ack())
            .await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "throttle must suppress the second hint for the same (conversation, run_id)"
        );
    }

    /// Two DeferredBusy acks with the same conversation but different active_run_ids
    /// → two posts (distinct throttle keys).
    #[tokio::test]
    async fn deferred_busy_same_conversation_different_run_id_posts_twice() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "9002.1"),
            )),
        );
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "9002.2"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);

        let make_ack = |run_id: TurnRunId| ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("slack:deferred-diff").expect("ref"),
            active_run_id: run_id,
        };

        // First delivery: run_id A.
        observer
            .observe_workflow_ack(envelope(user_message_payload()), make_ack(TurnRunId::new()))
            .await;
        // Second delivery: run_id B (different key) → separate hint posted.
        observer
            .observe_workflow_ack(envelope(user_message_payload()), make_ack(TurnRunId::new()))
            .await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            2,
            "different run_ids must produce separate hints even for the same conversation"
        );
    }

    /// deferred_busy_uses_ack_active_run_id_and_binding_scope_for_state_lookup:
    /// the GetRunStateRequest forwarded to the coordinator must carry the
    /// active_run_id from the DeferredBusy ack and the TurnScope derived from the
    /// conversation binding.
    #[tokio::test]
    async fn deferred_busy_uses_ack_active_run_id_and_binding_scope_for_state_lookup() {
        use std::sync::Mutex as StdMutex;

        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "9003.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());

        // Recording coordinator: captures every GetRunStateRequest it receives.
        struct RecordingTurnCoordinator {
            inner: ScriptedTurnCoordinator,
            recorded: StdMutex<Vec<GetRunStateRequest>>,
        }
        #[async_trait]
        impl TurnCoordinator for RecordingTurnCoordinator {
            async fn prepare_turn(&self, scope: TurnScope) -> Result<TurnRunId, TurnError> {
                self.inner.prepare_turn(scope).await
            }
            async fn submit_turn(
                &self,
                req: SubmitTurnRequest,
            ) -> Result<SubmitTurnResponse, TurnError> {
                self.inner.submit_turn(req).await
            }
            async fn resume_turn(
                &self,
                req: ResumeTurnRequest,
            ) -> Result<ResumeTurnResponse, TurnError> {
                self.inner.resume_turn(req).await
            }

            async fn retry_turn(
                &self,
                req: RetryTurnRequest,
            ) -> Result<RetryTurnResponse, TurnError> {
                self.inner.retry_turn(req).await
            }
            async fn get_run_state(
                &self,
                request: GetRunStateRequest,
            ) -> Result<TurnRunState, TurnError> {
                self.recorded
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(request.clone());
                self.inner.get_run_state(request).await
            }
            async fn cancel_run(
                &self,
                req: ironclaw_turns::CancelRunRequest,
            ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
                self.inner.cancel_run(req).await
            }
        }

        let active_run_id = TurnRunId::new();
        let recording_coordinator = Arc::new(RecordingTurnCoordinator {
            inner: ScriptedTurnCoordinator::with_single_status(TurnStatus::BlockedApproval),
            recorded: StdMutex::new(Vec::new()),
        });

        let observer = make_observer(
            recording_coordinator.clone(),
            egress.clone(),
            outbound,
            install,
        );

        let ack = ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("slack:scope-check").expect("ref"),
            active_run_id,
        };
        let env = envelope(user_message_payload());

        observer.observe_workflow_ack(env.clone(), ack).await;

        let recorded = recording_coordinator
            .recorded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        assert_eq!(recorded.len(), 1, "expected exactly one GetRunStateRequest");
        assert_eq!(
            recorded[0].run_id, active_run_id,
            "run_id in GetRunStateRequest must equal the DeferredBusy ack's active_run_id"
        );

        // Derive the expected scope from the same binding the observer resolves
        // (FakeConversationBindingService is deterministic), through the same
        // production helpers, and require an exact match.
        let binding = ironclaw_product_workflow::FakeConversationBindingService::new()
            .lookup_binding(ResolveBindingRequest::from_envelope(&env))
            .await
            .expect("fake binding service resolves test envelope");
        let thread_scope =
            thread_scope_from_binding(&binding).expect("test binding derives thread scope");
        let expected_scope = turn_scope_from_thread_scope(&binding, &thread_scope)
            .expect("test binding derives turn scope");
        assert_eq!(
            recorded[0].scope, expected_scope,
            "GetRunStateRequest scope must be derived from the authorized binding"
        );
    }

    /// A delivery that errors or times out must NOT leave the run_id in
    /// `active_delivery_run_ids` permanently. A subsequent `observe_workflow_ack`
    /// for the same run_id must proceed to delivery instead of being rejected by
    /// the guard.
    ///
    /// Test setup: the coordinator always returns `Running`; `max_wait = 1 ms`
    /// forces a timeout on every attempt. After the first timeout the RAII guard
    /// drops the run_id, so the second attempt reaches `wait_for_actionable` and
    /// polls `get_run_state` at least once more.
    ///
    /// If the guard were NOT released after an error, the second call would return
    /// early without ever calling `get_run_state`, and the total call count would
    /// equal the first attempt's count.
    #[tokio::test]
    async fn guard_is_released_after_delivery_error_so_subsequent_ack_proceeds() {
        let install = "test-install";
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        let outbound = Arc::new(InMemoryOutboundStateStore::default());

        // Build an observer with a very short max_wait so delivery times out quickly.
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let services = SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(
                ironclaw_product_workflow::FakeConversationBindingService::new(),
            ),
            thread_service,
            turn_coordinator: coordinator.clone(),
            outbound_store: outbound.clone(),
            route_store: Arc::new(InMemoryDeliveredGateRouteStore::default()),
            communication_preferences: outbound,
            adapter: test_adapter(install),
            egress: egress.clone(),
            delivery_sink: Arc::new(FakeOutboundDeliverySink::default()),
            auth_challenges: None,
        };
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::ZERO,
            max_wait: std::time::Duration::from_millis(1),
            max_concurrent_deliveries: NonZeroUsize::new(4).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let observer = SlackFinalReplyDeliveryObserver::with_settings(services, settings);

        let run_id = TurnRunId::new();
        let ack = ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("msg:guard-release-test")
                .expect("accepted message ref"), // safety: static test ref is valid.
            submitted_run_id: run_id,
        };
        let env = envelope(user_message_payload());

        // First delivery: times out; guard must be released on return.
        observer
            .observe_workflow_ack(env.clone(), ack.clone())
            .await;
        let calls_after_first = {
            let c = coordinator.calls.lock().expect("coordinator calls lock");
            *c
        };
        assert!(
            calls_after_first >= 1,
            "first delivery attempt must poll get_run_state at least once; got {calls_after_first}"
        );

        // Second delivery for the same run_id: if the guard were not released the
        // observer would return early and get_run_state would not be called again.
        observer.observe_workflow_ack(env, ack).await;
        let calls_after_second = {
            let c = coordinator.calls.lock().expect("coordinator calls lock");
            *c
        };
        assert!(
            calls_after_second > calls_after_first,
            "second delivery attempt must reach get_run_state (guard was not released after the first error); \
             calls after first={calls_after_first}, calls after second={calls_after_second}"
        );
    }

    /// Single-flight fanout regression: while one delivery loop is in flight for a
    /// run_id, a second ack carrying the SAME run_id must be rejected by the guard
    /// WITHOUT competing for the delivery semaphore permit.
    ///
    /// Real-world case: an `AuthResolution(Allowed)` / `ApprovalResolution(Allow)`
    /// resolution resumes the pre-existing run and is ack'd with the original
    /// `submitted_run_id`. The original loop is still watching, so a second loop
    /// would post gate N a second time (N resolutions ⇒ N+1 loops).
    ///
    /// This locks the TOCTOU ordering specifically: with `max_concurrent_deliveries
    /// = 1`, the first (blocked) delivery holds the only permit. If the guard were
    /// checked AFTER acquiring the permit, the second call would block on the
    /// semaphore and the `timeout` below would elapse. Because the guard is checked
    /// and inserted BEFORE the permit, the second call returns immediately.
    #[tokio::test]
    async fn concurrent_ack_for_same_run_id_is_rejected_before_acquiring_permit() {
        let install = "test-install";
        // Always-Running coordinator + large max_wait ⇒ the first delivery blocks in
        // wait_for_actionable, holding the single delivery permit for the test's life.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let services = SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(
                ironclaw_product_workflow::FakeConversationBindingService::new(),
            ),
            thread_service,
            turn_coordinator: coordinator.clone(),
            outbound_store: outbound.clone(),
            route_store: Arc::new(InMemoryDeliveredGateRouteStore::default()),
            communication_preferences: outbound,
            adapter: test_adapter(install),
            egress: egress.clone(),
            delivery_sink: Arc::new(FakeOutboundDeliverySink::default()),
            auth_challenges: None,
        };
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::from_millis(1),
            max_wait: std::time::Duration::from_secs(60),
            max_concurrent_deliveries: NonZeroUsize::new(1).unwrap(),
            max_pending_deliveries: NonZeroUsize::new(8).unwrap(),
        };
        let observer = Arc::new(SlackFinalReplyDeliveryObserver::with_settings(
            services, settings,
        ));

        let run_id = TurnRunId::new();
        let make_ack = |slug: &str| ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new(slug).expect("accepted message ref"),
            submitted_run_id: run_id,
        };
        let env = envelope(user_message_payload());

        // First delivery: acquires the guard + the only permit, then blocks.
        let first = {
            let observer = observer.clone();
            let env = env.clone();
            let ack = make_ack("msg:first");
            tokio::spawn(async move { observer.observe_workflow_ack(env, ack).await })
        };

        // Wait until the first loop registered the run_id in the single-flight set.
        loop {
            let registered = observer
                .active_delivery_run_ids
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&run_id);
            if registered {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }

        // Second ack for the SAME run_id while the first still holds the permit.
        // Must return promptly via the guard skip, NOT block on the semaphore.
        let second = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            observer.observe_workflow_ack(env, make_ack("msg:second")),
        )
        .await;
        assert!(
            second.is_ok(),
            "second ack for an in-flight run_id must be rejected by the single-flight guard \
             before acquiring the delivery permit; it blocked on the semaphore instead"
        );

        first.abort();
    }
}

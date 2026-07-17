//! Slack final-reply delivery for immediate-ACK Reborn webhooks.
//!
//! Slack Events API requires the HTTP handler to return 2xx quickly. This
//! observer runs after the workflow accepts an inbound Slack message, waits for
//! the submitted run to finish, reads the finalized assistant reply, and sends it
//! through the host-mediated product outbound delivery seam.
// arch-exempt: large_file, busy-thread / RejectedBusy hint logic stays here to share observer/test fixtures with final-reply delivery; decomposition tracked in #4818.

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
    ApprovalPromptContextView, DeclaredEgressHost, EgressCredentialHandle, EgressHeader,
    EgressMethod, EgressPath, EgressRequest, EgressResponse, ExternalActorRef,
    ExternalConversationRef, ExternalEventId, FinalReplyView, GatePromptView, OutboundDeliverySink,
    ProductAdapter, ProductAdapterError, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductOutboundPayload, ProductRejection, ProductRejectionKind,
    ProductWorkflowRejectionKind, ProtocolHttpEgress, ProtocolHttpEgressError,
};
use ironclaw_product_workflow::{
    ConversationBindingService, ProductOutboundDeliveryRequest, ProductOutboundTargetResolver,
    ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
    VerifiedProductOutboundTargetMetadata, is_approval_gate_ref,
    prepare_and_render_product_outbound,
};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, SessionThreadService, ThreadScope};
use ironclaw_triggers::TriggerFire;
use ironclaw_turns::{
    GateRef, GetRunStateRequest, ReplyTargetBindingRef, TurnActor, TurnCoordinator,
    TurnErrorCategory, TurnRunId, TurnRunState, TurnScope, TurnStatus,
};
use ironclaw_wasm_product_adapters::ImmediateAckWorkflowObserver;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use tokio::sync::Semaphore;

use crate::product_auth::api::auth_prompt::{
    BlockedAuthPromptRequest, auth_prompt_view_for_blocked_auth,
};
use crate::slack::slack_outbound_targets::{
    slack_conversation_id_from_reply_target_binding_ref, slack_reply_target_is_personal_dm,
};
use crate::{AuthChallengeProvider, BlockedAuthFlowCanceller};

const MAX_SLACK_RUN_POLL_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_TRIGGERED_RUN_DELIVERY_MAX_WAIT: Duration = Duration::from_secs(30 * 60);
const SLACK_RUN_POLL_JITTER_BUCKETS: u32 = 5;
const SLACK_API_HOST: &str = "slack.com";
const SLACK_BOT_TOKEN_HANDLE: &str = "slack_bot_token";
const SLACK_WORKING_MESSAGE: &str = "Ironclaw is thinking...";
const SLACK_AUTH_CANCELED_MESSAGE: &str = "Authentication canceled.";
/// Posted when a run blocks on a credential-entry (non-OAuth) auth challenge:
/// entering a secret in chat is a security risk, so it must be done in the web app.
const SLACK_AUTH_UNAVAILABLE_MESSAGE: &str = "Setting this up needs a credential (an API key or token). Sharing one here is a security risk — anything entered in chat is stored in the conversation — so credential-based connections can only be set up in the Ironclaw web app. Connect it there, then ask me again here.";
// Model B: greeting for a first-contact DM from a Slack user who has not yet
// connected their account. Fixed, host-authored text only — no agent runs.
const SLACK_CONNECT_NUDGE_MESSAGE: &str = "\u{1F44B} To use me, connect your Slack account in the Ironclaw web app: install the Slack extension and finish the connect step, then message me here again.";
const SLACK_DELIVERY_TIMEOUT_MESSAGE: &str =
    "This is taking longer than expected — check the WebUI for the result.";
const SLACK_DELIVERY_ERROR_MESSAGE: &str =
    "Something went wrong delivering the result here. Check the WebUI.";
/// Posted when the blocking run is `BlockedApproval` and no gate_ref is available.
const SLACK_BUSY_APPROVAL_MESSAGE: &str = "Ironclaw is waiting on a pending approval before taking new messages — reply `approve` or `deny` (or `approve gate:<ref>`) to resume.";
/// Posted for any other non-terminal blocking state, or when the state lookup fails.
const SLACK_BUSY_GENERIC_MESSAGE: &str = "Ironclaw is still working on a previous message and can't take this one yet — please resend it once the current task finishes.";

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
    /// Resolves auth challenges for `BlockedAuth` runs. Only link-based OAuth
    /// challenges are surfaced in Slack; other challenge kinds are denied (see the
    /// `BlockedAuth` arm of `notification_for_actionable_state`).
    pub auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
    /// Cancels the durable `AuthFlow` record whenever a `BlockedAuth` run is
    /// auto-cancelled by the Slack delivery path. Threaded through the shared
    /// `cancel_auth_blocked_run` helper, so it covers every caller that cancels a
    /// blocked-auth run: the live observer non-OAuth deny arm, the triggered
    /// non-OAuth deny arm, and the OAuth send-time DM backstop. The Slack path
    /// cancels the run directly via `TurnCoordinator` (it does not go through the
    /// canonical `AuthInteractionService` deny path), which would otherwise leave
    /// the flow record non-terminal (#4952); this cancels the flow alongside the
    /// run, after the run cancel succeeds. `None` (e.g. no `flow_record_source`
    /// wired in) skips the flow cancel and still cancels the run — backward-compatible.
    pub auth_flow_canceller: Option<Arc<dyn BlockedAuthFlowCanceller>>,
    /// Store used to resolve an approval gate's request details (tool/action/reason)
    /// so the Slack approval prompt can say WHAT is being approved — the same
    /// source the WebUI projection reads. `None` disables the enrichment.
    pub approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
}

/// Maximum number of (conversation, external_event_id) pairs remembered for hint dedup.
/// FIFO eviction beyond this cap keeps memory O(1); a false-negative after
/// eviction just means one extra hint, which is harmless.
const HINT_SEEN_CAP: usize = 256;

/// Throttle key for the busy-thread hint: one hint per (conversation fingerprint, external event id).
///
/// Using `ExternalEventId` instead of `TurnRunId` means:
/// - Transport retries of the **same** Slack event share the same `external_event_id`, so
///   they are deduplicated here — no duplicate hints on retries.
/// - Each **new** human message has a distinct `external_event_id`, so each new message
///   gets a fresh hint even if the same blocking run is still active.
type HintSeenKey = (String, ExternalEventId);
type HintSeenSet = Mutex<(VecDeque<HintSeenKey>, HashSet<HintSeenKey>)>;

pub struct SlackFinalReplyDeliveryObserver {
    services: SlackFinalReplyDeliveryServices,
    settings: SlackFinalReplyDeliverySettings,
    delivery_permits: Arc<Semaphore>,
    /// Per-observer throttle: at most one busy-thread hint per
    /// (conversation fingerprint, external_event_id) pair.
    /// Transport retries of the same Slack event share the same external_event_id, so
    /// they are deduplicated here. Each distinct new human message gets a fresh hint
    /// even if the same blocking run is still active.
    /// Bounded FIFO eviction keeps memory O(1); a false-negative after eviction just
    /// means one extra hint, harmless.
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
                    target = "ironclaw::reborn::slack_delivery",
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
                // Look up WHAT is being approved from the ApprovalRequestStore by
                // gate ref — the same source the WebUI gate projection uses — so
                // the prompt names the capability/reason instead of a generic step.
                let approval_context = crate::projection::approval_prompt_context_view(
                    self.services.approval_requests.as_deref(),
                    gate_ref,
                    &binding.actor_user_id,
                    scope,
                )
                .await;
                SlackActionableNotification {
                    event_kind: RunNotificationEventKind::ApprovalNeeded,
                    payload: ProductOutboundPayload::GatePrompt(slack_approval_gate_prompt_view(
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
                        "Slack run is blocked on auth without a gate ref; skipping auth handling"
                    );
                    return Ok(None);
                };
                let view = auth_prompt_view_for_blocked_auth(BlockedAuthPromptRequest {
                    fallback_owner_user_id: &binding.actor_user_id,
                    scope,
                    run_id,
                    gate_ref: gate_ref.as_str(),
                    invocation_id: None,
                    body: "Authenticate to continue this run.".to_string(),
                    credential_requirements: &state.credential_requirements,
                    auth_challenges: self.services.auth_challenges.as_deref(),
                })
                .await?;
                // Only link-based OAuth is allowed over Slack: the user
                // authenticates on the provider's site via `authorization_url` and
                // the callback stores the credential server-side — nothing secret
                // is entered into the chat surface. Any other challenge (manual
                // token / API-key entry, etc.) would have the user paste a
                // credential into Slack, so deny it: cancel the run (same outcome
                // as `auth deny`) and redirect them to the web app.
                if view.authorization_url.is_some() {
                    SlackActionableNotification {
                        event_kind: RunNotificationEventKind::AuthRequired,
                        payload: ProductOutboundPayload::AuthPrompt(slack_auth_prompt_view(
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
                    self.cancel_slack_auth_blocked_run(
                        scope,
                        TurnActor::new(binding.actor_user_id.clone()),
                        run_id,
                        gate_ref.as_str(),
                    )
                    .await?;
                    if let Err(error) = post_slack_message(
                        self.services.egress.as_ref(),
                        envelope.external_conversation_ref(),
                        SLACK_AUTH_UNAVAILABLE_MESSAGE,
                    )
                    .await
                    {
                        tracing::debug!(
                            target = "ironclaw::reborn::slack_delivery",
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
    async fn cancel_slack_auth_blocked_run(
        &self,
        scope: &TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        gate_ref: &str,
    ) -> Result<(), SlackFinalReplyDeliveryError> {
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
                require_direct_message_target: false,
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
        if !matches!(envelope.payload(), ProductInboundPayload::UserMessage(_)) {
            return false;
        }
        // Only nudge in a 1:1 DM. An unbound user's app-mention in a SHARED
        // channel also rejects with `BindingRequired`; posting the host connect
        // nudge there would drop a message addressed to one user into a channel
        // everyone can see. Slack DM (im) channel ids start with 'D'; shared
        // channels ('C') and multi-person/group DMs ('G') are excluded, and a
        // missing/blank conversation ref fails closed (no post).
        if !envelope
            .external_conversation_ref()
            .conversation_id()
            .starts_with('D')
        {
            return false;
        }
        if let Err(error) = post_slack_message(
            self.services.egress.as_ref(),
            envelope.external_conversation_ref(),
            SLACK_CONNECT_NUDGE_MESSAGE,
        )
        .await
        {
            tracing::debug!(
                target = "ironclaw::reborn::slack_delivery",
                error = %error,
                "failed to post Slack connect nudge (best-effort)"
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

/// Adapts a resolved auth-prompt view for Slack delivery. OAuth setup links are
/// only safe to post in a private DM, so the `authorization_url` is stripped for
/// any non-DM (channel) target.
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
            if payload.trigger == ironclaw_product_adapters::ProductTriggerReason::DirectChat
    )
}

fn slack_approval_gate_prompt_view(
    run_id: TurnRunId,
    gate_ref: &GateRef,
    context: Option<&ApprovalPromptContextView>,
) -> GatePromptView {
    let gate_ref_str = gate_ref.as_str();

    // Body carries only the semantic *What/Why* of the gate. The channel-specific
    // *how to reply* (which differs for a DM vs a channel thread, and is the same
    // for every gate) is appended once by the Slack adapter's
    // `gate_prompt_reply_instruction` — keeping the two from duplicating the
    // reply instructions and keeping the message short.
    let body = match context {
        Some(ctx) => {
            let mut body = format!("*What:* {}", ctx.tool_name);
            if let Some(reason) = ctx.reason.as_deref() {
                body.push_str(&format!("\n*Why:* {reason}"));
            }
            body
        }
        None => "A step in this workflow needs your approval to continue.".to_string(),
    };

    GatePromptView {
        turn_run_id: run_id,
        gate_ref: gate_ref_str.to_string(),
        invocation_id: None,
        headline: "Approval needed".to_string(),
        body,
        allow_always: is_approval_gate_ref(gate_ref_str),
        approval_context: context.cloned(),
    }
}

/// Cancel a run parked on an interactive-auth gate with a `Policy` reason — the
/// same `cancel_run` the auth-deny resolution uses. Idempotent per run
/// (`slack-auth-block:{run_id}`) so repeated observer/delivery passes are safe.
/// Shared by the live observer path ([`SlackFinalReplyDeliveryObserver::cancel_slack_auth_blocked_run`])
/// and the triggered delivery path ([`triggered_notification_for_state`]) so the
/// cancellation contract cannot drift between them.
async fn cancel_auth_blocked_run(
    coordinator: &dyn TurnCoordinator,
    auth_flow_canceller: Option<&dyn BlockedAuthFlowCanceller>,
    scope: &TurnScope,
    actor: TurnActor,
    run_id: TurnRunId,
    gate_ref: Option<&str>,
) -> Result<(), SlackFinalReplyDeliveryError> {
    // Resolve the flow-cancel target BEFORE `cancel_run` consumes `actor`. Owner
    // resolution mirrors `auth_prompt_view_for_blocked_auth`: an explicit turn owner
    // (shared/team subject) wins, else the acting user. When `gate_ref` is absent
    // there is no flow to resolve, so the flow cancel is skipped entirely (not
    // encoded as an empty ref).
    let flow_cancel_target = match (auth_flow_canceller, gate_ref) {
        (Some(canceller), Some(gate_ref)) => {
            let owner_user_id = scope
                .explicit_owner_user_id()
                .unwrap_or(&actor.user_id)
                .clone();
            Some((canceller, owner_user_id, gate_ref))
        }
        _ => None,
    };

    let idempotency_key = ironclaw_turns::IdempotencyKey::new(format!("slack-auth-block:{run_id}"))
        .map_err(|err| SlackFinalReplyDeliveryError::SlackWebApi {
            reason: format!("invalid idempotency key for slack auth block: {err}"),
        })?;
    // Cancel the run FIRST — it is the user-visible terminal action. `cancel_run` is
    // idempotent (`slack-auth-block:{run_id}`), so repeated passes are safe. If it
    // fails we return here and leave the durable `AuthFlow` (and the still-usable
    // auth prompt) intact: marking the flow terminal while the run is still
    // `BlockedAuth` would be the inverse state drift this fix is meant to prevent,
    // and the OAuth backstop relies on a failed cancel leaving the prompt usable.
    coordinator
        .cancel_run(ironclaw_turns::CancelRunRequest {
            scope: scope.clone(),
            actor,
            run_id,
            reason: ironclaw_turns::SanitizedCancelReason::Policy,
            idempotency_key,
        })
        .await?;

    // Run is now terminal — cancel the stale `AuthFlow` record alongside it (#4952).
    // Best-effort cleanliness: a flow-cancel failure does not surface, since the
    // run (the user-visible action) has already been cancelled.
    if let Some((canceller, owner_user_id, gate_ref)) = flow_cancel_target
        && let Err(error) = canceller
            .cancel_blocked_auth_flow(scope, &owner_user_id, run_id, gate_ref)
            .await
    {
        tracing::debug!(
            target = "ironclaw::reborn::slack_delivery",
            %run_id,
            %error,
            "failed to cancel stale auth flow on Slack auth auto-deny (best-effort)"
        );
    }
    Ok(())
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
        if self
            .post_connect_nudge_if_unbound_user_message(&envelope, &ack)
            .await
        {
            return;
        }
        // A2b: Busy-thread hint — the user's message was silently dropped
        // because a run is busy (pending gate or generic RejectedBusy). Post a
        // one-shot state-aware hint so the user knows to approve/deny/wait (gate
        // cases) or simply retry later (running-state cases) rather than being
        // left in silence. Same best-effort semantics as A2: post failure → debug! only.
        //
        // Authorization: only post if the binding lookup succeeds, matching the
        // same guard used by `post_rejection_hint_if_authorized`.
        //
        // Inline await is safe: the protocol ACK already returned before this
        // observer runs, and the runner's admission permit in runner_immediate_ack.rs
        // bounds the lifetime of this entire post-ACK task. A detached spawn would
        // escape `drain_immediate_ack_tasks` shutdown/drain without adding any
        // backpressure benefit.
        if let Some(active_run_id) = busy_hint_user_message_run_id(&envelope, &ack) {
            // Throttle: at most one hint per (conversation, external_event_id) pair.
            // Slack transport retries carry the same event id → deduplicated without
            // duplicate posts. Each new human message has a distinct event id → each
            // gets a fresh hint, even if the same blocking run is still active.
            // Check before the coordinator call to avoid a round-trip on repeats.
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
                    "busy-thread hint suppressed: already posted for this (conversation, event_id) pair (transport retry)"
                );
                return;
            }
            // Derive the scope for the active run state lookup so the hint can be
            // state-specific (pending gate vs generic busy). When the conversation
            // has no resolvable binding — e.g. a gate delivered into a fresh DM
            // that never carried a prior user message — fall back to the generic
            // busy copy rather than going silent. Posting a generic "I'm waiting
            // on approval" back to the conversation that just messaged us leaks no
            // data: it is a reply to the sender's own conversation. The user's
            // choice here is to never be left without feedback while a gate is open.
            let hint = match self
                .services
                .binding_service
                .lookup_binding(ResolveBindingRequest::from_envelope(&envelope))
                .await
            {
                Ok(binding) => {
                    busy_hint_from_run_state(
                        self.services.turn_coordinator.as_ref(),
                        self.services.approval_requests.as_deref(),
                        &binding,
                        active_run_id,
                    )
                    .await
                }
                Err(error) => {
                    tracing::debug!(
                        target = "ironclaw::reborn::slack_delivery",
                        error = %error,
                        "busy-thread hint falling back to generic copy because the conversation binding was not resolved"
                    );
                    SLACK_BUSY_GENERIC_MESSAGE.to_string()
                }
            };
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
                    "failed to post busy-thread hint to Slack (best-effort)"
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
        // An unbound user's first inbound resolves as a `BindingRequired`
        // workflow *error*, surfaced here — NOT as an `Ok(Rejected)` ack in
        // `observe_workflow_ack`. `post_rejection_hint_if_authorized` is a
        // guaranteed no-op for them (its binding lookup fails by definition),
        // so without the connect nudge an unbound 1:1 DM gets total silence.
        // Mirror the ack-path ordering: authorized rejection hint first, then
        // the connect nudge for unbound DMs.
        if self
            .post_rejection_hint_if_authorized(&envelope, &ack)
            .await
        {
            return;
        }
        self.post_connect_nudge_if_unbound_user_message(&envelope, &ack)
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

/// Fail closed when a delivery that must reach a personal DM (e.g. carries an
/// OAuth authorization_url) resolves to a non-DM target.
fn enforce_direct_message_if_required(
    target: &ReplyTargetBindingRef,
    require_direct_message: bool,
) -> Result<(), ProductWorkflowError> {
    if require_direct_message && !slack_reply_target_is_personal_dm(target) {
        return Err(ProductWorkflowError::OutboundTargetNotDirectMessage);
    }
    Ok(())
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
        require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        if target.target() != &self.expected_target {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        // Defense in depth: honor the DM requirement even on the live-path resolver.
        enforce_direct_message_if_required(target.target(), require_direct_message)?;
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
        | ProductInboundAck::RejectedBusy { .. }
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
/// the busy-thread hint flow: a `DeferredBusy` (legacy) or `RejectedBusy` ack on a
/// `UserMessage` payload.
///
/// `RejectedBusy { active_run_id: Some(run_id) }` carries a live blocking run whose
/// state can be fetched to produce a gate-aware hint.  When `active_run_id` is `None`
/// (e.g. a replay with no live run) we return `None` — there is no run state to
/// inspect so no hint is appropriate.
///
/// `Duplicate { prior }` — `RejectedBusy` is a settled outcome, so a Slack transport
/// retry of the same external event arrives as `Duplicate { prior: RejectedBusy { .. } }`.
/// We unwrap the prior and re-apply the same extraction so that `Duplicate { prior:
/// RejectedBusy { active_run_id: Some(run) } }` still yields the blocking run id.
/// The per-(conversation, event_id) throttle prevents a double-post when the first
/// delivery already succeeded — the retry only posts if the original hint was lost.
/// `Duplicate { prior: DeferredBusy { active_run_id, .. } }` yields `Some(active_run_id)` —
/// the recursive call re-applies the same extraction on the prior ack.  DeferredBusy is never
/// settled upstream (so this wrapping is unreachable in practice), but when it does occur the
/// run id is surfaced rather than silently dropped.
///
/// Returns `None` for all non-user-message payloads (resolution/control payloads must
/// stay silent).
fn busy_hint_user_message_run_id(
    envelope: &ProductInboundEnvelope,
    ack: &ProductInboundAck,
) -> Option<TurnRunId> {
    // Only reply to user messages — resolution/control/noop payloads must stay silent.
    if !matches!(envelope.payload(), ProductInboundPayload::UserMessage(_)) {
        return None;
    }
    match ack {
        ProductInboundAck::DeferredBusy { active_run_id, .. } => Some(*active_run_id),
        // RejectedBusy with a live blocking run → hint is gated on the run state.
        // RejectedBusy with no run (replay / no live run) → no hint.
        ProductInboundAck::RejectedBusy {
            active_run_id: Some(run_id),
            ..
        } => Some(*run_id),
        ProductInboundAck::RejectedBusy {
            active_run_id: None,
            ..
        } => None,
        // Unwrap Duplicate and re-apply extraction on the prior ack.
        // RejectedBusy is a settled outcome, so transport retries arrive as
        // Duplicate{RejectedBusy{..}} — the prior still carries the blocking run id.
        // DeferredBusy is never settled upstream, so Duplicate{DeferredBusy} is
        // unreachable in practice; but when it occurs the recursive call yields
        // Some(active_run_id) from the prior — the run id is not silently dropped.
        ProductInboundAck::Duplicate { prior } => busy_hint_user_message_run_id(envelope, prior),
        _ => None,
    }
}

/// Looks up the blocking run's state and returns the appropriate busy-thread hint
/// copy.
///
/// - `BlockedApproval` with `Some(gate_ref)` → approval wording with concrete `approve {ref}` command
/// - `BlockedApproval` with `None` gate_ref  → approval wording without a specific gate command
/// - `BlockedAuth` with `Some(gate_ref)`     → auth wording with concrete `auth deny {ref}` command
/// - `BlockedAuth` with `None` gate_ref      → auth wording without the deny command
/// - anything else / lookup failure           → generic wording
///
/// Never returns an error — lookup failures degrade to the generic copy.
async fn busy_hint_from_run_state(
    coordinator: &dyn TurnCoordinator,
    approval_requests: Option<&dyn ApprovalRequestStore>,
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
                "busy-thread hint scope derivation failed; using generic copy"
            );
            return SLACK_BUSY_GENERIC_MESSAGE.to_string();
        }
    };
    match coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id: active_run_id,
        })
        .await
    {
        Ok(state) => match state.status {
            TurnStatus::BlockedApproval => match state.gate_ref.as_ref() {
                // Name both the blocking gate ref AND what it would approve (the
                // tool/capability), so the user sees exactly what is holding the
                // conversation and what `approve` would authorize. The blocking
                // run is in this thread's scope (that is why the thread is busy),
                // so the approval request resolves under the derived scope.
                Some(gate_ref) => {
                    let what = crate::projection::approval_prompt_context_view(
                        approval_requests,
                        gate_ref,
                        &binding.actor_user_id,
                        &scope,
                    )
                    .await
                    .map(|ctx| ctx.tool_name);
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
                None => SLACK_BUSY_APPROVAL_MESSAGE.to_string(),
            },
            // Auth gates can't be completed in Slack (credential sharing is a
            // security risk), but still name the blocking ref so the user can
            // decline it here and knows what is holding the thread.
            TurnStatus::BlockedAuth => match state.gate_ref.as_ref() {
                Some(gate_ref) => format!(
                    "Ironclaw is waiting on authentication before taking new messages. Reply \
                     `auth deny {ref}` to decline it here, or complete the connection in the \
                     Ironclaw web app to resume.",
                    ref = gate_ref.as_str()
                ),
                None => SLACK_BUSY_GENERIC_MESSAGE.to_string(),
            },
            _ => SLACK_BUSY_GENERIC_MESSAGE.to_string(),
        },
        Err(err) => {
            tracing::debug!(
                target = "ironclaw::reborn::slack_delivery",
                error = %err,
                "busy-thread hint run-state lookup failed; using generic copy"
            );
            SLACK_BUSY_GENERIC_MESSAGE.to_string()
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
/// submission has been durably settled in trigger storage. The composition root
/// wires a real implementation (behind the `slack-v2-host-beta` feature) or a
/// no-op.
#[async_trait]
pub trait PostSubmitDeliveryHook: Send + Sync {
    /// Called with the original trigger fire, the submitted run id, and the
    /// turn scope the run was submitted under. The trigger poller invokes this
    /// hook from a detached task after the accepted fire appears as settled, so
    /// hook latency cannot delay settlement and delivery cannot precede the
    /// persisted run/thread mapping. Implementations may still spawn their own
    /// longer-lived delivery tasks when they need bounded admission or shutdown
    /// tracking.
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
    /// Resolves per-trigger delivery targets (`TriggerFire::delivery_target`)
    /// into reply-target bindings. Consulted only for fires that carry a
    /// target; fires without one use the creator's preference as before.
    /// Production wiring (`build_triggered_run_delivery_hook`) always supplies
    /// it; when absent (reduced test drivers), a fire carrying a target fails
    /// closed as `TargetUnavailable` — see
    /// `driver_fire_with_unresolvable_delivery_target_records_target_unavailable`.
    outbound_target_provider: Option<Arc<dyn crate::outbound::OutboundDeliveryTargetProvider>>,
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
            SlackFinalReplyDeliverySettings {
                max_wait: DEFAULT_TRIGGERED_RUN_DELIVERY_MAX_WAIT,
                ..SlackFinalReplyDeliverySettings::default()
            },
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
            outbound_target_provider: None,
        }
    }

    /// Wire the outbound delivery target provider used to resolve per-trigger
    /// delivery targets. Production wiring must call this; without it, fires
    /// carrying a `delivery_target` fail closed as `TargetUnavailable`.
    pub(crate) fn with_outbound_target_provider(
        mut self,
        provider: Arc<dyn crate::outbound::OutboundDeliveryTargetProvider>,
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
            auth_flow_canceller: self.services.auth_flow_canceller.clone(),
            approval_requests: self.services.approval_requests.clone(),
        };
        let settings = self.settings;
        let delivery_store = Arc::clone(&self.delivery_store);
        let fallback_agent_id = self.fallback_agent_id.clone();
        let outbound_target_provider = self.outbound_target_provider.clone();

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
                outbound_target_provider.as_deref(),
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
#[allow(clippy::too_many_arguments)]
async fn deliver_triggered_run(
    services: &SlackFinalReplyDeliveryServices,
    settings: &SlackFinalReplyDeliverySettings,
    fire: &TriggerFire,
    run_id: TurnRunId,
    scope: TurnScope,
    delivery_store: &dyn TriggeredRunDeliveryStore,
    fallback_agent_id: &ironclaw_host_api::AgentId,
    outbound_target_provider: Option<&dyn crate::outbound::OutboundDeliveryTargetProvider>,
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
                        target = "ironclaw::reborn::slack_delivery",
                        %run_id,
                        ?outcome,
                        "triggered run delivery stopped: per-trigger delivery target did not resolve"
                    );
                    record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                    return outcome;
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
    let authority = TriggeredSlackReplyTargetAuthority {
        scope: scope.clone(),
        actor: actor.clone(),
        trigger_context: trigger_context.clone(),
        per_trigger_source_route,
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
            Err(SlackFinalReplyDeliveryError::RunWaitTimedOut { .. })
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
                    target = "ironclaw::reborn::slack_delivery",
                    %run_id,
                    "triggered run parked awaiting user after delivering blocked prompt; recording Delivered"
                );
                let outcome = TriggeredRunDeliveryOutcomeKind::Delivered;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
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
                    delete_triggered_slack_message(services, message).await;
                }
                let outcome = TriggeredRunDeliveryOutcomeKind::Delivered;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
            Err(TriggeredNotificationFailure::OAuthTargetNotDm) => {
                // Authority backstop tripped: the payload carried an OAuth
                // authorization_url but the send-time binding was not a personal DM.
                // Suppress the URL (fail closed), cancel the blocked run, then post
                // the auth-unavailable notice as a terminal FinalReply — mirrors the
                // non-OAuth deny branch in `triggered_notification_for_state`.
                tracing::debug!(
                    target = "ironclaw::reborn::slack_delivery",
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
                        target = "ironclaw::reborn::slack_delivery",
                        %run_id,
                        error = %err,
                        "triggered run OAuth backstop: cancel_auth_blocked_run failed"
                    );
                    let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
                    record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                    return outcome;
                }
                // Post the auth-unavailable notice as a terminal FinalReply.
                // require_direct_message_target is false: the notice is plain text
                // with no OAuth URL, so no DM restriction applies.
                let notice = SlackActionableNotification {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    payload: ProductOutboundPayload::FinalReply(FinalReplyView {
                        turn_run_id: run_id,
                        text: format!(
                            "{SLACK_AUTH_UNAVAILABLE_MESSAGE}{}",
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
                    delete_triggered_slack_message(services, message).await;
                }
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
    services: &SlackFinalReplyDeliveryServices,
    scope: &TurnScope,
    thread_scope: &ThreadScope,
    actor: &TurnActor,
    state: &TurnRunState,
    run_id: TurnRunId,
    trigger_label: &str,
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
                    target = "ironclaw::reborn::slack_delivery",
                    %run_id,
                    "completed triggered run has no finalized assistant message; skipping delivery"
                );
                return Ok(None);
            };
            Ok(Some(SlackActionableNotification {
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
                    target = "ironclaw::reborn::slack_delivery",
                    %run_id,
                    "triggered run blocked on approval without gate ref; skipping"
                );
                return Ok(None);
            };
            // Render the triggered approval prompt exactly like the regular
            // inbound flow: What/Why context (the tool + reason) via the shared
            // `slack_approval_gate_prompt_view`, with the channel-specific reply
            // instruction appended once by the adapter. The approval request is
            // stored under this triggered run's scope, so the context resolves
            // here.
            let context = crate::projection::approval_prompt_context_view(
                services.approval_requests.as_deref(),
                gate_ref,
                &actor.user_id,
                scope,
            )
            .await;
            let mut prompt = slack_approval_gate_prompt_view(run_id, gate_ref, context.as_ref());
            prompt.body.push_str(&triggered_gate_footer(trigger_label));
            Ok(Some(SlackActionableNotification {
                event_kind: RunNotificationEventKind::ApprovalNeeded,
                payload: ProductOutboundPayload::GatePrompt(prompt),
                gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
            }))
        }
        TurnStatus::BlockedAuth => {
            let Some(gate_ref) = state.gate_ref.as_ref() else {
                tracing::warn!(
                    target = "ironclaw::reborn::slack_delivery",
                    %run_id,
                    "triggered run blocked on auth without gate ref; skipping"
                );
                return Ok(None);
            };
            let mut view = auth_prompt_view_for_blocked_auth(BlockedAuthPromptRequest {
                fallback_owner_user_id: &actor.user_id,
                scope,
                run_id,
                gate_ref: gate_ref.as_str(),
                invocation_id: None,
                body: "Authentication required to continue this automation.".to_string(),
                credential_requirements: &state.credential_requirements,
                auth_challenges: services.auth_challenges.as_deref(),
            })
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
                Ok(Some(SlackActionableNotification {
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
                Ok(Some(SlackActionableNotification {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    payload: ProductOutboundPayload::FinalReply(FinalReplyView {
                        turn_run_id: run_id,
                        text: format!(
                            "{SLACK_AUTH_UNAVAILABLE_MESSAGE}{}",
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
    services: &SlackFinalReplyDeliveryServices,
    scope: &TurnScope,
    actor: &TurnActor,
    run_id: TurnRunId,
    state: &TurnRunState,
    authority: &TriggeredSlackReplyTargetAuthority,
    notification: SlackActionableNotification,
    require_direct_message_target: bool,
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

    let tracked_egress = TrackingSlackPostEgress::new(Arc::clone(&services.egress));
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

/// Resolve a fire's per-trigger delivery target id into the reply-target
/// binding the resolution engine should prefer over the user preference.
///
/// Fail-closed contract: any missing provider, malformed id, foreign/stale
/// target (`Ok(None)`), or provider backend error yields a terminal outcome
/// instead of a route — delivery never guesses a substitute conversation.
async fn resolve_per_trigger_delivery_route(
    outbound_target_provider: Option<&dyn crate::outbound::OutboundDeliveryTargetProvider>,
    fire: &TriggerFire,
    scope: &TurnScope,
    target: &ironclaw_triggers::TriggerDeliveryTargetId,
) -> Result<ReplyTargetBindingRef, TriggeredRunDeliveryOutcomeKind> {
    let Some(provider) = outbound_target_provider else {
        tracing::warn!(
            target = "ironclaw::reborn::slack_delivery",
            "per-trigger delivery target present but no outbound target provider is wired"
        );
        return Err(TriggeredRunDeliveryOutcomeKind::TargetUnavailable);
    };
    let target_id = ironclaw_product_workflow::RebornOutboundDeliveryTargetId::new(target.as_str())
        .map_err(|error| {
            tracing::warn!(
                target = "ironclaw::reborn::slack_delivery",
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
                target = "ironclaw::reborn::slack_delivery",
                "per-trigger delivery target did not resolve for the trigger creator (stale, foreign, or disconnected)"
            );
            Err(TriggeredRunDeliveryOutcomeKind::TargetUnavailable)
        }
        Err(error) => {
            tracing::warn!(
                target = "ironclaw::reborn::slack_delivery",
                %error,
                "outbound delivery target lookup failed during triggered delivery"
            );
            Err(TriggeredRunDeliveryOutcomeKind::Failed)
        }
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
        require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        // Single enforcement point for the OAuth DM rule: when the delivery request
        // requires a direct-message target (i.e. the payload carries an OAuth
        // authorization_url), enforce that the EXACT send-time binding is a personal
        // DM. Checked against the binding resolved NOW (at send time), making it
        // race-free against the pre-loop preference snapshot going stale.
        enforce_direct_message_if_required(target.target(), require_direct_message)?;

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
        envelope_with_event_id("evt:test", payload)
    }

    /// Like `envelope` but with a caller-specified event id.  Use this in tests
    /// that need distinct event ids to exercise the per-(conversation, event_id)
    /// throttle — e.g. two separate human messages vs. a transport retry.
    fn envelope_with_event_id(
        event_id: &str,
        payload: ProductInboundPayload,
    ) -> ProductInboundEnvelope {
        build_test_envelope(event_id, "D123", payload)
    }

    /// Like `envelope` but with a caller-specified conversation id, so tests can
    /// distinguish a 1:1 DM ('D...') from a shared channel ('C...').
    fn envelope_in_conversation(
        conversation_id: &str,
        payload: ProductInboundPayload,
    ) -> ProductInboundEnvelope {
        build_test_envelope("evt:test", conversation_id, payload)
    }

    fn build_test_envelope(
        event_id: &str,
        conversation_id: &str,
        payload: ProductInboundPayload,
    ) -> ProductInboundEnvelope {
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
            ExternalEventId::new(event_id).expect("event"),
            ExternalActorRef::new("slack_user", "U123", None::<String>).expect("actor"),
            ExternalConversationRef::new(Some("T123"), conversation_id, None, None)
                .expect("conversation"),
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
        /// When set, `get_run_state` returns `ScopeNotFound` — simulating a run
        /// that does not live in the queried scope (a triggered/foreign run).
        scope_not_found: bool,
        calls: Mutex<usize>,
        cancel_calls: Mutex<Vec<TurnRunId>>,
        /// When set, `cancel_run` returns `Err(TurnError::Unavailable)` instead of
        /// the normal success response. Used to test the OAuth backstop cancel-failure path.
        cancel_should_fail: std::sync::atomic::AtomicBool,
        /// When `true`, `get_run_state` clamps at the LAST scripted state once
        /// exhausted (sticky) instead of cycling with wraparound. Models a run
        /// that transitions through a prefix of states and then stays in its
        /// final state forever (e.g. Running → BlockedApproval and never resolves).
        clamp_at_last: bool,
    }

    impl ScriptedTurnCoordinator {
        fn with_states(states: Vec<ScriptedRunState>) -> Self {
            assert!(!states.is_empty(), "must provide at least one state");
            Self {
                states,
                scope_not_found: false,
                calls: Mutex::new(0),
                cancel_calls: Mutex::new(Vec::new()),
                cancel_should_fail: std::sync::atomic::AtomicBool::new(false),
                clamp_at_last: false,
            }
        }

        /// Like [`with_states`] but the final state is sticky: once the script is
        /// exhausted, `get_run_state` keeps returning the last entry instead of
        /// wrapping around. Use to model a run parked in its terminal-ish state.
        fn with_states_clamped(states: Vec<ScriptedRunState>) -> Self {
            Self {
                clamp_at_last: true,
                ..Self::with_states(states)
            }
        }

        fn with_single_status(status: TurnStatus) -> Self {
            Self::with_states(vec![ScriptedRunState {
                status,
                gate_ref: None,
            }])
        }

        /// A coordinator whose `get_run_state` always reports `ScopeNotFound` —
        /// the run is not in the queried (conversation) scope.
        fn scope_not_found() -> Self {
            let mut coordinator = Self::with_single_status(TurnStatus::Running);
            coordinator.scope_not_found = true;
            coordinator
        }

        fn cancel_call_count(&self) -> usize {
            self.cancel_calls.lock().expect("cancel calls lock").len()
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
            if self.scope_not_found {
                return Err(TurnError::ScopeNotFound);
            }
            let mut calls = self.calls.lock().expect("calls lock");
            let idx = if self.clamp_at_last {
                (*calls).min(self.states.len() - 1)
            } else {
                *calls % self.states.len()
            };
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
                model_usage: None,
                received_at: Utc::now(),
                checkpoint_id: None,
                gate_ref: scripted.gate_ref,
                blocked_activity_id: None,
                credential_requirements: Vec::new(),
                failure: None,
                event_cursor: EventCursor(1),
                product_context: None,
                resume_disposition: None,
            })
        }

        async fn cancel_run(
            &self,
            request: ironclaw_turns::CancelRunRequest,
        ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
            self.cancel_calls
                .lock()
                .expect("cancel calls lock")
                .push(request.run_id);
            if self
                .cancel_should_fail
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return Err(TurnError::Unavailable {
                    reason: "ScriptedTurnCoordinator: cancel_should_fail is set".to_string(),
                });
            }
            Ok(ironclaw_turns::CancelRunResponse {
                run_id: request.run_id,
                status: TurnStatus::Cancelled,
                event_cursor: ironclaw_turns::EventCursor::default(),
                already_terminal: false,
                actor: None,
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
            delivery_target: None,
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
        crate::slack::slack_outbound_targets::slack_reply_target_binding_ref_from_raw(raw)
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

    /// Seed a personal preference with distinct `auth_prompt_target` and
    /// `final_reply_target` binding refs. Used to prove the OAuth DM gate keys on
    /// the EFFECTIVE auth target (`auth_prompt_target.or(final_reply_target)`),
    /// not "any stored target".
    async fn seed_personal_preference_with_auth_target(
        store: &InMemoryOutboundStateStore,
        scope: &TurnScope,
        auth_prompt_target: ReplyTargetBindingRef,
        final_reply_target: ReplyTargetBindingRef,
    ) {
        let tenant = scope.tenant_id.clone();
        let user = scope
            .explicit_owner_user_id()
            .cloned()
            .expect("owner user id for preference seed");
        let updated_by = user.clone();
        let record = CommunicationPreferenceRecord {
            scope: DeliveryDefaultScope::personal(tenant, user),
            final_reply_target: Some(final_reply_target),
            progress_target: None,
            approval_prompt_target: None,
            auth_prompt_target: Some(auth_prompt_target),
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
        make_services_with_canceller(
            coordinator,
            thread_service,
            egress,
            outbound,
            installation_id,
            None,
        )
    }

    /// Like [`make_services`] but threads in an explicit `auth_flow_canceller`.
    /// Used by triggered-path tests that need to assert `BlockedAuthFlowCanceller`
    /// is called (or not called) when the triggered delivery hits a `BlockedAuth` state.
    fn make_services_with_canceller(
        coordinator: Arc<dyn TurnCoordinator>,
        thread_service: Arc<dyn ironclaw_threads::SessionThreadService>,
        egress: Arc<FakeProtocolHttpEgress>,
        outbound: Arc<InMemoryOutboundStateStore>,
        installation_id: &str,
        auth_flow_canceller: Option<Arc<dyn BlockedAuthFlowCanceller>>,
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
            auth_flow_canceller,
            approval_requests: None,
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

    /// Shared arrangement for the T1 delivery-honesty pair (#6105): a
    /// `Completed` run with a finalized reply and a saved personal DM
    /// preference, driven through the real `TriggeredRunDeliveryDriver`
    /// against a programmed `chat.postMessage` response body. The two tests
    /// differ ONLY in that body and in what they assert — one arrangement
    /// keeps the Delivered and Failed arms from drifting apart.
    async fn deliver_completed_run_with_programmed_post_response(
        post_response_body: Vec<u8>,
    ) -> (TriggeredRunDeliveryRecord, Arc<FakeProtocolHttpEgress>) {
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
            Ok(EgressResponse::new(200, post_response_body)),
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
        (record, egress)
    }

    #[tokio::test]
    async fn driver_happy_path_completed_run_with_preference_delivers_and_records_delivered() {
        let (record, egress) = deliver_completed_run_with_programmed_post_response(
            slack_post_ok_json("D456", "1234.5678"),
        )
        .await;

        // Egress should have been called for chat.postMessage.
        let post = egress
            .calls()
            .into_iter()
            .find(|c| c.path == "/api/chat.postMessage")
            .expect("expected chat.postMessage egress call");

        // Right-target pin (#5943/#5877 shape, T1 of #6105): the posted body
        // must carry the DM conversation id decoded from the saved
        // preference's binding ref — not the run's current channel or another
        // user's target.
        let body: serde_json::Value =
            serde_json::from_slice(&post.body).expect("postMessage body is valid JSON");
        assert_eq!(
            body.get("channel").and_then(|channel| channel.as_str()),
            Some("D456"),
            "chat.postMessage must target the preference binding's DM conversation; body: {body}"
        );

        // Outcome should be Delivered.
        assert_eq!(record.outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    }

    /// Delivery-honesty pin (#5944 shape, T1 of #6105): the RUN completed, but
    /// Slack rejected the post (`200 {"ok":false,...}`). The delivery record
    /// must say `Failed` — a `Delivered` here is exactly the "delivery
    /// silently fails but run reports success" bug: run status alone cannot
    /// distinguish the two, only this record can.
    #[tokio::test]
    async fn driver_slack_api_rejection_records_failed_not_delivered() {
        // Slack accepts the HTTP call but rejects the post — the classic
        // silent-failure shape (revoked channel, kicked bot, archived DM).
        let (record, egress) = deliver_completed_run_with_programmed_post_response(
            serde_json::json!({"ok": false, "error": "channel_not_found"})
                .to_string()
                .into_bytes(),
        )
        .await;

        // The post WAS attempted (this is not a skip/no-target case)…
        assert!(
            egress
                .calls()
                .iter()
                .any(|c| c.path == "/api/chat.postMessage"),
            "expected the rejected chat.postMessage egress call"
        );
        // …and the rejection must be recorded as Failed, never Delivered.
        assert_eq!(
            record.outcome,
            TriggeredRunDeliveryOutcomeKind::Failed,
            "a Slack-rejected post on a Completed run must record Failed (#5944 honesty pair)"
        );
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

    /// Build the real Slack outbound-target provider over in-memory stores with
    /// one shared-channel route (`channel_id` owned by the creator user) so
    /// per-trigger delivery target resolution runs the production path.
    fn shared_channel_target_provider(
        install: &str,
        scope: &TurnScope,
        channel_id: &str,
    ) -> Arc<crate::slack::slack_outbound_targets::SlackHostBetaOutboundTargetProvider> {
        use crate::slack::slack_channel_routes::InMemorySlackChannelRouteStore;
        use crate::slack::slack_outbound_targets::{
            InMemorySlackPersonalDmTargetStore, SlackConfiguredChannelRoute,
            SlackHostBetaOutboundTargetProvider, SlackOutboundTargetProviderConfig,
        };
        Arc::new(SlackHostBetaOutboundTargetProvider::new(
            SlackOutboundTargetProviderConfig {
                tenant_id: scope.tenant_id.clone(),
                agent_id: scope.agent_id.clone().expect("test scope has agent"),
                project_id: scope.project_id.clone(),
                installation_id: AdapterInstallationId::new(install).expect("installation id"),
                team_id: crate::slack::slack_serve::SlackTeamId::new("T123"),
                configured_channel_routes: vec![SlackConfiguredChannelRoute::new(
                    channel_id.to_string(),
                    scope
                        .explicit_owner_user_id()
                        .cloned()
                        .expect("test scope has owner"),
                )],
            },
            Arc::new(InMemorySlackChannelRouteStore::new()),
            Arc::new(InMemorySlackPersonalDmTargetStore::new()),
        ))
    }

    /// A fire carrying a per-trigger delivery target must deliver to THAT
    /// target, not to the creator's user-global preference — the preference
    /// clobbering across automations is exactly the wrong-channel bug.
    #[tokio::test]
    async fn driver_fire_with_delivery_target_routes_to_it_over_the_preference() {
        let install = "test-install";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        // User-global preference points at DM D456.
        let preference_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Completed,
        ));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_finalized_assistant_message(&thread_service, &scope, run_id, "Routed result").await;

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, preference_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("C789", "1234.5678"),
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
        )
        .with_outbound_target_provider(shared_channel_target_provider(install, &scope, "C789"));

        let mut fire = minimal_trigger_fire(None);
        fire.delivery_target = Some(
            ironclaw_triggers::TriggerDeliveryTargetId::new("slack:shared-channel:T123:C789")
                .expect("delivery target id"),
        );
        driver.on_trigger_submitted(fire, run_id, scope).await;

        let record = wait_for_delivery_record(&delivery_store, run_id).await;
        assert_eq!(record.outcome, TriggeredRunDeliveryOutcomeKind::Delivered);

        let posts: Vec<String> = egress
            .calls()
            .iter()
            .filter(|call| call.path == "/api/chat.postMessage")
            .map(|call| String::from_utf8_lossy(&call.body).to_string())
            .collect();
        assert_eq!(posts.len(), 1, "exactly one delivery post expected");
        assert!(
            posts[0].contains("C789"),
            "delivery must go to the per-trigger target channel: {}",
            posts[0]
        );
        assert!(
            !posts[0].contains("D456"),
            "delivery must not fall back to the user-global preference DM: {}",
            posts[0]
        );
    }

    /// A per-trigger delivery target works with NO user-global preference at
    /// all — previously this fire could only record NoDefaultConfigured.
    #[tokio::test]
    async fn driver_fire_with_delivery_target_delivers_without_any_preference() {
        let install = "test-install";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();

        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Completed,
        ));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_finalized_assistant_message(&thread_service, &scope, run_id, "Routed result").await;
        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // No preference seeded.

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("C789", "1234.5678"),
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
        )
        .with_outbound_target_provider(shared_channel_target_provider(install, &scope, "C789"));

        let mut fire = minimal_trigger_fire(None);
        fire.delivery_target = Some(
            ironclaw_triggers::TriggerDeliveryTargetId::new("slack:shared-channel:T123:C789")
                .expect("delivery target id"),
        );
        driver.on_trigger_submitted(fire, run_id, scope).await;

        let record = wait_for_delivery_record(&delivery_store, run_id).await;
        assert_eq!(record.outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
        assert!(
            egress
                .calls()
                .iter()
                .any(|call| call.path == "/api/chat.postMessage"),
            "expected chat.postMessage egress call"
        );
    }

    /// A per-trigger target that no longer resolves (stale, foreign, or the
    /// product is disconnected) fails closed: no egress, TargetUnavailable —
    /// never a silent fallback to some other conversation.
    #[tokio::test]
    async fn driver_fire_with_unresolvable_delivery_target_records_target_unavailable() {
        let install = "test-install";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();

        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Completed,
        ));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_finalized_assistant_message(&thread_service, &scope, run_id, "Routed result").await;
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
        )
        .with_outbound_target_provider(shared_channel_target_provider(install, &scope, "C789"));

        let mut fire = minimal_trigger_fire(None);
        fire.delivery_target = Some(
            ironclaw_triggers::TriggerDeliveryTargetId::new("slack:shared-channel:T123:C_UNKNOWN")
                .expect("delivery target id"),
        );
        driver.on_trigger_submitted(fire, run_id, scope).await;

        let record = wait_for_delivery_record(&delivery_store, run_id).await;
        assert_eq!(
            record.outcome,
            TriggeredRunDeliveryOutcomeKind::TargetUnavailable
        );
        assert!(
            !egress
                .calls()
                .iter()
                .any(|call| call.path == "/api/chat.postMessage"),
            "unresolvable target must not produce any egress"
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

    #[test]
    fn triggered_driver_default_wait_budget_is_longer_than_live_slack_reply_wait() {
        let install = "test-install";
        let scope = personal_turn_scope();
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Completed,
        ));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(coordinator, thread_service, egress, outbound, install);

        let driver = TriggeredRunDeliveryDriver::new(
            services,
            delivery_store,
            route_store,
            scope.agent_id.clone().expect("test scope has agent"),
        );

        assert_eq!(
            driver.settings.max_wait,
            DEFAULT_TRIGGERED_RUN_DELIVERY_MAX_WAIT
        );
        assert!(driver.settings.max_wait > SlackFinalReplyDeliverySettings::default().max_wait);
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

    /// `RecoveryRequired` is terminal (`TurnStatus::is_terminal`) but has no explicit
    /// arm in `triggered_notification_for_state`, so it falls through the catch-all
    /// `_ => Ok(None)` (#5713: intentionally kept, not a bug). The driver must record
    /// `Skipped` and never build or send a Slack message.
    #[tokio::test]
    async fn driver_terminal_recovery_required_run_records_skipped_without_egress() {
        let install = "test-install";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();

        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::RecoveryRequired,
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
            max_wait: std::time::Duration::from_secs(5),
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
            TriggeredRunDeliveryOutcomeKind::Skipped,
            "terminal RecoveryRequired run must be recorded as Skipped, not Failed"
        );
        assert!(
            !egress
                .calls()
                .iter()
                .any(|c| c.path == "/api/chat.postMessage"),
            "no chat.postMessage egress expected for a skipped delivery"
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
        make_observer_with_canceller(coordinator, egress, outbound, installation_id, None)
    }

    fn make_observer_with_canceller(
        coordinator: Arc<dyn TurnCoordinator>,
        egress: Arc<FakeProtocolHttpEgress>,
        outbound: Arc<InMemoryOutboundStateStore>,
        installation_id: &str,
        auth_flow_canceller: Option<Arc<dyn BlockedAuthFlowCanceller>>,
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
            auth_flow_canceller,
            approval_requests: None,
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

    /// Foreign-run guard: an Accepted resolution ack whose run lives in another
    /// scope (a triggered run bridged via the delivered-gate-route rewrite) must
    /// NOT produce a spurious delivery-error post. The live observer skips
    /// delivery (the triggered loop owns continuation) when `get_run_state`
    /// reports the run is not in this conversation scope.
    #[tokio::test]
    async fn accepted_resolution_for_foreign_scope_run_skips_delivery_without_error() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // The resolved run is not in this conversation scope — it lives in the
        // trigger's scope, delivered by its own triggered-delivery loop.
        let coordinator = Arc::new(ScriptedTurnCoordinator::scope_not_found());
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(scoped_approval_resolution_payload());
        let ack = accepted_ack();

        observer.observe_workflow_ack(env, ack).await;

        let post_count = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .count();
        assert_eq!(
            post_count, 0,
            "foreign-scope run must skip live delivery silently — no spurious \
             error post expected, got {post_count} post(s)"
        );
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

    /// Model B: a rejected first-contact UserMessage from an unbound Slack user
    /// (BindingRequired) is greeted with a connect nudge — no binding lookup, no
    /// agent turn. Previously this was silently dropped.
    #[tokio::test]
    async fn rejected_unbound_user_message_posts_connect_nudge() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
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
        let env = envelope(user_message_payload());
        let ack = rejected_ack(ironclaw_product_adapters::ProductRejectionKind::BindingRequired);

        observer.observe_workflow_ack(env, ack).await;

        let posted: Vec<String> = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .collect();
        assert_eq!(
            posted.len(),
            1,
            "an unbound first-contact DM should get exactly one connect nudge"
        );
        assert!(
            posted[0].contains("connect your Slack account"),
            "the connect nudge must tell the user to connect, got: {}",
            posted[0]
        );
    }

    /// Regression: an unbound user's app-mention in a SHARED channel also
    /// rejects with `BindingRequired`, but the host connect-nudge must NOT be
    /// posted into the shared channel — only a 1:1 DM gets it. Same shape as
    /// `rejected_unbound_user_message_posts_connect_nudge` but with a shared
    /// channel ('C0SHARED') conversation, asserting zero posts.
    #[tokio::test]
    async fn rejected_unbound_user_message_in_shared_channel_posts_no_connect_nudge() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("C0SHARED", "1000.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope_in_conversation("C0SHARED", user_message_payload());
        let ack = rejected_ack(ironclaw_product_adapters::ProductRejectionKind::BindingRequired);

        observer.observe_workflow_ack(env, ack).await;

        let posted: Vec<String> = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .collect();
        assert!(
            posted.is_empty(),
            "an unbound app-mention in a shared channel must NOT get a connect nudge posted into the channel, got: {posted:?}"
        );
    }

    /// Regression (connect-nudge wiring): the coverage that was missing. In
    /// production an unbound user's first-contact DM does NOT arrive at
    /// `observe_workflow_ack` as an `Ok(Rejected)` ack — the workflow returns
    /// `BindingRequired` as an ERROR (`ScopeNotFound`, status 404), so the
    /// runner routes it to `observe_workflow_error`. The connect nudge must
    /// fire on THAT path too. Before the fix this posted nothing (the nudge was
    /// wired only into `observe_workflow_ack`); the ack-path unit test above
    /// masked the gap by calling the ack method directly with a synthetic ack
    /// instead of driving the real error path a live unbound DM takes.
    #[tokio::test]
    async fn unbound_user_message_via_workflow_error_posts_connect_nudge() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
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
        let env = envelope(user_message_payload());
        // The production shape: an unbound scope resolves as a BindingRequired
        // workflow rejection ERROR (ScopeNotFound -> BindingRequired), surfaced
        // through `observe_workflow_error`, NOT an `Ok(Rejected)` ack.
        let error = ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            status_code: 404,
            retryable: false,
            reason: ironclaw_product_adapters::RedactedString::new("scope not found"),
        };

        observer.observe_workflow_error(env, error).await;

        let posted: Vec<String> = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .collect();
        assert_eq!(
            posted.len(),
            1,
            "an unbound first-contact DM must get the connect nudge via the ERROR observer path (the real production path), got: {posted:?}"
        );
        assert!(
            posted[0].contains("connect your Slack account"),
            "the connect nudge must tell the user to connect, got: {}",
            posted[0]
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

    /// Duplicate { prior: DeferredBusy } + UserMessage → hint posted (run id extracted
    /// from the prior, same as for a plain DeferredBusy).
    ///
    /// `should_settle_ack` returns false for DeferredBusy, so the idempotency
    /// ledger never settles it and this case is unreachable in practice. However,
    /// the Duplicate unwrap arm delegates to the prior ack's extraction, so
    /// DeferredBusy inside a Duplicate consistently yields the run id rather than
    /// silently dropping it.
    #[tokio::test]
    async fn duplicate_deferred_busy_with_user_message_posts_hint() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "8001.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(user_message_payload());
        let ack = ProductInboundAck::Duplicate {
            prior: Box::new(deferred_busy_ack()),
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
            "Duplicate{{DeferredBusy}} must post a hint — run id extracted from prior"
        );
    }

    /// Two distinct plain DeferredBusy + UserMessage envelopes with different external_event_ids
    /// → two posts (throttle is per (conversation, external_event_id) pair).
    ///
    /// Each envelope is built with a distinct event id so the two messages have
    /// distinct throttle keys and each posts exactly one hint.  The active_run_id in
    /// the acks is the same here to demonstrate that run_id no longer drives dedup.
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

        // Use a shared active_run_id across both acks to prove it's the event_id that
        // gates dedup, not the run_id.
        let shared_run_id = TurnRunId::new();
        let make_ack = || ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("slack:deferred-two-events")
                .expect("ref"),
            active_run_id: shared_run_id,
        };

        // First new user message (event id "evt:msg-1") — must post a hint.
        observer
            .observe_workflow_ack(
                envelope_with_event_id("evt:msg-1", user_message_payload()),
                make_ack(),
            )
            .await;
        // Second new user message (event id "evt:msg-2") — distinct event → must also post.
        observer
            .observe_workflow_ack(
                envelope_with_event_id("evt:msg-2", user_message_payload()),
                make_ack(),
            )
            .await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            2,
            "two distinct event ids must each post a hint even with the same active_run_id"
        );
    }

    /// DeferredBusy + UserMessage + BlockedAuth state → generic busy hint posted
    /// (auth-specific wording removed; BlockedAuth now maps to the generic fallback).
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
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some("gate:auth-slack-hint"),
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
            body.contains("waiting on authentication") && body.contains("auth deny"),
            "deferred-busy hint for BlockedAuth must name the blocking auth gate, got: {body}"
        );
        // Must not contain the old auth-prompt wording.
        assert!(
            !body.contains("authentication step"),
            "deferred-busy hint for BlockedAuth must not mention 'authentication step', got: {body}"
        );
    }

    /// Accepted ack + BlockedAuth state → cancel_run is called and SLACK_AUTH_UNAVAILABLE_MESSAGE
    /// is posted; no "Authentication required" text appears.
    #[tokio::test]
    async fn blocked_auth_cancels_run_and_posts_unavailable_message() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "6005.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some("gate:auth-cancel-test"),
        )]));
        let observer = make_observer(
            Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
            egress.clone(),
            outbound,
            install,
        );
        let env = envelope(user_message_payload());
        let ack = ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("slack:blocked-auth-cancel-test")
                .expect("ref"),
            submitted_run_id: TurnRunId::new(),
        };

        observer.observe_workflow_ack(env, ack).await;

        assert_eq!(
            coordinator.cancel_call_count(),
            1,
            "BlockedAuth must cancel the run exactly once"
        );

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage for BlockedAuth cancel"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains(SLACK_AUTH_UNAVAILABLE_MESSAGE),
            "body must contain SLACK_AUTH_UNAVAILABLE_MESSAGE text, got: {body}"
        );
        assert!(
            !body.contains("Authentication required"),
            "body must not contain old auth-prompt text, got: {body}"
        );
    }

    /// Records every `cancel_blocked_auth_flow` call so tests can assert the Slack
    /// auto-deny path cancels the durable auth-flow record alongside the run (#4952).
    ///
    /// Captures all four arguments of `cancel_blocked_auth_flow` so tests can assert
    /// that both the wiring (run_id/gate_ref) and the owner-resolution logic
    /// (scope/owner_user_id) are correct. Asserting against concrete fixture values
    /// catches a wrong-owner regression at production line 1167 that a tuple of
    /// `(TurnRunId, String)` would silently miss.
    #[derive(Clone)]
    struct RecordedFlowCancel {
        scope: TurnScope,
        owner_user_id: ironclaw_host_api::UserId,
        run_id: TurnRunId,
        gate_ref: String,
    }

    #[derive(Default)]
    struct RecordingBlockedAuthFlowCanceller {
        calls: std::sync::Mutex<Vec<RecordedFlowCancel>>,
    }

    #[async_trait]
    impl BlockedAuthFlowCanceller for RecordingBlockedAuthFlowCanceller {
        async fn cancel_blocked_auth_flow(
            &self,
            scope: &TurnScope,
            owner_user_id: &ironclaw_host_api::UserId,
            run_id: TurnRunId,
            gate_ref: &str,
        ) -> Result<(), ironclaw_auth::AuthProductError> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(RecordedFlowCancel {
                    scope: scope.clone(),
                    owner_user_id: owner_user_id.clone(),
                    run_id,
                    gate_ref: gate_ref.to_string(),
                });
            Ok(())
        }
    }

    /// Accepted ack + BlockedAuth (non-OAuth) → the auto-deny cancels the stale
    /// auth-flow record (via `BlockedAuthFlowCanceller`) for the blocked gate, not
    /// just the run. Drives the live observer caller so a wiring regression — the
    /// canceller no longer threaded into `cancel_auth_blocked_run` — is caught.
    #[tokio::test]
    async fn blocked_auth_cancels_stale_auth_flow() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "6006.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some("gate:auth-cancel-test"),
        )]));
        let recorder = Arc::new(RecordingBlockedAuthFlowCanceller::default());
        let observer = make_observer_with_canceller(
            Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
            egress.clone(),
            outbound,
            install,
            Some(Arc::clone(&recorder) as Arc<dyn BlockedAuthFlowCanceller>),
        );
        let env = envelope(user_message_payload());
        let submitted_run_id = TurnRunId::new();
        let ack = ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("slack:blocked-auth-flow-cancel-test")
                .expect("ref"),
            submitted_run_id,
        };

        observer.observe_workflow_ack(env, ack).await;

        // Run is still cancelled exactly once...
        assert_eq!(
            coordinator.cancel_call_count(),
            1,
            "BlockedAuth must cancel the run exactly once"
        );
        // ...and the stale auth flow is cancelled for the same blocked gate.
        let calls = recorder
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            calls.len(),
            1,
            "auto-deny must cancel the stale auth flow exactly once"
        );
        assert_eq!(
            calls[0].run_id, submitted_run_id,
            "canceller must receive the same run_id as the submitted ack"
        );
        assert_eq!(
            calls[0].gate_ref, "gate:auth-cancel-test",
            "must cancel the auth flow for the blocked gate"
        );
        // FIX 2: Assert the resolved owner_user_id and scope match the fixture values.
        //
        // In the live-observer path, `FakeConversationBindingService` derives:
        //   actor_user_id    = "user:{external_actor_ref.id()}" = "user:U123"
        //   subject_user_id  = Some("user:U123")
        // That subject_user_id becomes thread_scope.owner_user_id → passed as the
        // explicit owner to `TurnScope::new_with_owner`, so
        // `scope.explicit_owner_user_id() = Some("user:U123")` which wins over
        // actor.user_id in `cancel_auth_blocked_run` (production line 1167).
        let expected_owner =
            ironclaw_host_api::UserId::new("user:U123").expect("expected owner fixture");
        assert_eq!(
            calls[0].owner_user_id, expected_owner,
            "owner_user_id must be the subject user derived from the external actor ref (U123)"
        );
        // Scope tenant must match what FakeConversationBindingService builds from
        // installation_id "install_alpha".
        let expected_tenant =
            ironclaw_host_api::TenantId::new("tenant:install_alpha").expect("expected tenant");
        assert_eq!(
            calls[0].scope.tenant_id, expected_tenant,
            "scope.tenant_id must match the tenant derived from the installation"
        );
    }

    /// FIX 3: A failed `cancel_run` must leave the `AuthFlow` record intact.
    ///
    /// `cancel_auth_blocked_run` was reordered so the run is cancelled FIRST and
    /// the durable `AuthFlow` is only marked terminal AFTER a successful cancel.
    /// This test proves the invariant: when `cancel_run` returns `Err`, the
    /// `BlockedAuthFlowCanceller` is NOT invoked — preventing inverse state drift
    /// (a terminal `AuthFlow` whose corresponding run is still `BlockedAuth`).
    ///
    /// Drives the live-observer path (`SlackFinalReplyDeliveryObserver`) with a
    /// `ScriptedTurnCoordinator` whose `cancel_should_fail` flag is set, mirroring
    /// the mechanism used in `triggered_oauth_auth_backstop_cancel_failure_records_failed`.
    #[tokio::test]
    async fn blocked_auth_cancel_run_failure_leaves_auth_flow_intact() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // No HTTP response programmed: the cancel fails before any post is made.

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some("gate:cancel-fail-intact"),
        )]));
        // Make cancel_run fail — mirrors the mechanism in
        // `triggered_oauth_auth_backstop_cancel_failure_records_failed`.
        coordinator
            .cancel_should_fail
            .store(true, std::sync::atomic::Ordering::Release);

        let recorder = Arc::new(RecordingBlockedAuthFlowCanceller::default());
        let observer = make_observer_with_canceller(
            Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
            egress.clone(),
            outbound,
            install,
            Some(Arc::clone(&recorder) as Arc<dyn BlockedAuthFlowCanceller>),
        );
        let env = envelope(user_message_payload());
        let submitted_run_id = TurnRunId::new();
        let ack = ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("slack:cancel-fail-intact-test")
                .expect("ref"),
            submitted_run_id,
        };

        observer.observe_workflow_ack(env, ack).await;

        // cancel_run was attempted (it just failed).
        assert_eq!(
            coordinator.cancel_call_count(),
            1,
            "cancel_run must be attempted exactly once even when it fails"
        );
        // The flow canceller must NOT have been called: a failed run-cancel must
        // leave the durable AuthFlow record intact so the auth prompt remains usable.
        let calls = recorder
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            calls.is_empty(),
            "BlockedAuthFlowCanceller must NOT be called when cancel_run fails; got {} call(s)",
            calls.len()
        );
    }

    /// A `BlockedAuthFlowCanceller` that always returns `Err(BackendUnavailable)`.
    /// Used to assert that a flow-cancel error is swallowed and does not break
    /// Slack auto-denial delivery.
    ///
    /// `call_count` is incremented atomically on every `cancel_blocked_auth_flow`
    /// invocation so tests can assert the canceller was actually wired and called.
    struct FailingBlockedAuthFlowCanceller {
        call_count: std::sync::atomic::AtomicUsize,
    }

    impl FailingBlockedAuthFlowCanceller {
        fn new() -> Self {
            Self {
                call_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.call_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl BlockedAuthFlowCanceller for FailingBlockedAuthFlowCanceller {
        async fn cancel_blocked_auth_flow(
            &self,
            _scope: &ironclaw_turns::TurnScope,
            _owner_user_id: &ironclaw_host_api::UserId,
            _run_id: TurnRunId,
            _gate_ref: &str,
        ) -> Result<(), ironclaw_auth::AuthProductError> {
            self.call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(ironclaw_auth::AuthProductError::BackendUnavailable)
        }
    }

    /// A flow-cancel failure must be swallowed: a failing `BlockedAuthFlowCanceller`
    /// must not break Slack auto-denial.
    ///
    /// After `cancel_run` succeeds, `cancel_auth_blocked_run` attempts a best-effort
    /// `cancel_blocked_auth_flow`.  When that returns `Err`, the error is debug-logged
    /// and the function still returns `Ok(())` — so the `SLACK_AUTH_UNAVAILABLE_MESSAGE`
    /// post still goes out and the coordinator cancel count is still 1.
    #[tokio::test]
    async fn blocked_auth_canceller_failure_is_swallowed() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "6007.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // cancel_run SUCCEEDS (cancel_should_fail is NOT set, matching the default).
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some("gate:auth-cancel-test"),
        )]));
        // Wire in a canceller that always fails — swallow path under test.
        // Hold a clone of the Arc so we can inspect call_count after the observer runs.
        let failing_canceller = Arc::new(FailingBlockedAuthFlowCanceller::new());
        let observer = make_observer_with_canceller(
            Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
            egress.clone(),
            outbound,
            install,
            Some(Arc::clone(&failing_canceller) as Arc<dyn BlockedAuthFlowCanceller>),
        );
        let env = envelope(user_message_payload());
        let ack = ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("slack:canceller-fail-swallowed-test")
                .expect("ref"),
            submitted_run_id: TurnRunId::new(),
        };

        observer.observe_workflow_ack(env, ack).await;

        // The canceller must have been invoked exactly once — proving it is wired up.
        assert_eq!(
            failing_canceller.call_count(),
            1,
            "cancel_blocked_auth_flow must be called exactly once on the failing canceller"
        );

        // The run was still cancelled exactly once — flow-cancel failure does not
        // prevent run cancellation or the auto-denial post.
        assert_eq!(
            coordinator.cancel_call_count(),
            1,
            "cancel_run must be called exactly once even when flow-cancel fails"
        );

        // The SLACK_AUTH_UNAVAILABLE_MESSAGE post must still go out.
        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage despite flow-cancel failure"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains(SLACK_AUTH_UNAVAILABLE_MESSAGE),
            "body must contain SLACK_AUTH_UNAVAILABLE_MESSAGE text, got: {body}"
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

    /// DeferredBusy + UserMessage + unresolved binding → generic busy hint posted.
    ///
    /// Uses `TestNoopConversationBindingService` (always fails lookup_binding) to
    /// simulate a conversation with no resolvable binding (e.g. a gate delivered
    /// into a fresh DM). The observer must still post the generic busy copy to the
    /// originating conversation rather than leaving the user in silence — replying
    /// a generic "waiting on approval" notice to the sender's own conversation
    /// leaks no data.
    #[tokio::test]
    async fn deferred_busy_unresolved_binding_posts_generic_hint() {
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
            auth_flow_canceller: None,
            approval_requests: None,
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
            "the generic busy hint must be posted even when the binding does not resolve"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains(SLACK_BUSY_GENERIC_MESSAGE),
            "fallback hint must be the generic busy copy, got: {body}"
        );
    }

    // ── RejectedBusy ack feedback tests ───────────────────────────────────────
    //
    // PR #4838 replaced `DeferredBusy` with `RejectedBusy` for busy user-message
    // outcomes.  The hint path must recognise the new variant and produce the same
    // gate-aware (BlockedApproval/BlockedAuth) or generic copy as it does for the
    // legacy `DeferredBusy` variant.

    fn rejected_busy_ack_with_run_id() -> ProductInboundAck {
        ProductInboundAck::RejectedBusy {
            accepted_message_ref: AcceptedMessageRef::new("slack:rejected-busy").expect("ref"),
            active_run_id: Some(TurnRunId::new()),
        }
    }

    fn rejected_busy_ack_no_run_id() -> ProductInboundAck {
        ProductInboundAck::RejectedBusy {
            accepted_message_ref: AcceptedMessageRef::new("slack:rejected-busy-none").expect("ref"),
            active_run_id: None,
        }
    }

    /// RejectedBusy { active_run_id: Some(..) } + UserMessage + BlockedApproval with
    /// gate_ref → exactly one Slack post containing the concrete `approve {ref}` command.
    #[tokio::test]
    async fn rejected_busy_ack_with_run_id_posts_approval_hint() {
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
        let gate_ref_str = "gate:approval-rb123";
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some(gate_ref_str),
        )]));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(user_message_payload());
        let ack = rejected_busy_ack_with_run_id();

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage for RejectedBusy(Some) + BlockedApproval"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains("waiting on a pending approval"),
            "RejectedBusy hint must mention 'waiting on a pending approval', got: {body}"
        );
        assert!(
            body.contains(gate_ref_str),
            "RejectedBusy approval hint must embed the concrete gate ref '{gate_ref_str}', got: {body}"
        );
    }

    /// RejectedBusy { active_run_id: Some(..) } + UserMessage + BlockedAuth state →
    /// generic busy hint posted (BlockedAuth now maps to the generic fallback).
    #[tokio::test]
    async fn rejected_busy_ack_with_run_id_posts_auth_hint() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "7001.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some("gate:auth-rb456"),
        )]));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(user_message_payload());
        let ack = rejected_busy_ack_with_run_id();

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "expected exactly one chat.postMessage for RejectedBusy(Some) + BlockedAuth"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains("waiting on authentication") && body.contains("auth deny"),
            "RejectedBusy auth hint must name the blocking auth gate, got: {body}"
        );
        assert!(
            !body.contains("authentication step"),
            "RejectedBusy auth hint must not mention 'authentication step', got: {body}"
        );
    }

    /// RejectedBusy { active_run_id: None } + UserMessage → no hint posted.
    ///
    /// When there is no live blocking run there is no run state to inspect, so
    /// the hint flow is skipped entirely.
    #[tokio::test]
    async fn rejected_busy_ack_with_no_run_id_posts_nothing() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(user_message_payload());
        let ack = rejected_busy_ack_no_run_id();

        observer.observe_workflow_ack(env, ack).await;

        let calls = egress.calls();
        assert!(
            !calls.iter().any(|c| c.path == "/api/chat.postMessage"),
            "no chat.postMessage expected for RejectedBusy(None) — no live run to inspect"
        );
    }

    /// Duplicate { prior: RejectedBusy { active_run_id: Some(..) } } + UserMessage +
    /// BlockedApproval state → hint posted (gate-aware approval copy).
    ///
    /// `RejectedBusy` is a settled outcome, so a Slack transport retry of the same
    /// external event arrives as `Duplicate { prior: RejectedBusy { .. } }`.  The
    /// busy-hint helper must unwrap the prior and extract the blocking run id so the
    /// retry can still post the hint if the original was lost.
    #[tokio::test]
    async fn duplicate_rejected_busy_with_run_id_posts_approval_hint() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "8100.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let gate_ref_str = "gate:approval-dup-rb001";
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some(gate_ref_str),
        )]));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);
        let env = envelope(user_message_payload());
        let ack = ProductInboundAck::Duplicate {
            prior: Box::new(rejected_busy_ack_with_run_id()),
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
            "Duplicate{{RejectedBusy(Some)}} + UserMessage must post exactly one hint"
        );
        let body = std::str::from_utf8(&post_calls[0].body).expect("utf8 body");
        assert!(
            body.contains("waiting on a pending approval"),
            "Duplicate{{RejectedBusy}} hint must mention 'waiting on a pending approval', got: {body}"
        );
        assert!(
            body.contains(gate_ref_str),
            "Duplicate{{RejectedBusy}} approval hint must embed gate ref '{gate_ref_str}', got: {body}"
        );
    }

    /// Duplicate { prior: RejectedBusy { active_run_id: Some(..) } } delivered twice
    /// with the same (conversation, event_id) → exactly one post (throttle suppresses
    /// the second).
    ///
    /// Both deliveries use `envelope()` which has a fixed event id "evt:test", so
    /// they share the same (conversation, event_id) throttle key.  This models a
    /// Slack transport retry of the exact same external event — the throttle prevents
    /// double-posting: the first delivery inserts the key and the second is suppressed.
    #[tokio::test]
    async fn duplicate_rejected_busy_throttle_suppresses_second_delivery() {
        let install = "test-install";
        let run_id = TurnRunId::new();
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Only one response slot — if two posts were attempted the second would
        // error, making the assertion below a double-check.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "8101.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);

        let make_dup_ack = || ProductInboundAck::Duplicate {
            prior: Box::new(ProductInboundAck::RejectedBusy {
                accepted_message_ref: AcceptedMessageRef::new("slack:dup-rb-throttle")
                    .expect("ref"),
                active_run_id: Some(run_id),
            }),
        };

        // First delivery: same event id "evt:test" → inserts throttle key, posts hint.
        observer
            .observe_workflow_ack(envelope(user_message_payload()), make_dup_ack())
            .await;
        // Second delivery: same event id "evt:test" → throttle key already present → suppressed.
        observer
            .observe_workflow_ack(envelope(user_message_payload()), make_dup_ack())
            .await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "throttle must suppress the second Duplicate{{RejectedBusy}} hint for the same (conversation, event_id)"
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
            auth_flow_canceller: None,
            approval_requests: None,
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
            auth_flow_canceller: None,
            approval_requests: None,
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
            auth_flow_canceller: None,
            approval_requests: None,
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

    #[test]
    fn slack_approval_prompt_offers_always_for_typed_approval_gate() {
        let gate_ref = GateRef::new(format!(
            "gate:approval-{}",
            ironclaw_host_api::ApprovalRequestId::new()
        ))
        .expect("gate ref");

        let prompt = slack_approval_gate_prompt_view(TurnRunId::new(), &gate_ref, None);

        assert_eq!(prompt.gate_ref, gate_ref.as_str());
        assert!(prompt.allow_always);
    }

    #[test]
    fn slack_approval_prompt_does_not_offer_always_for_generic_gate() {
        let gate_ref = GateRef::new("gate:approve-slack").expect("gate ref");

        let prompt = slack_approval_gate_prompt_view(TurnRunId::new(), &gate_ref, None);

        assert_eq!(prompt.gate_ref, gate_ref.as_str());
        assert!(!prompt.allow_always);
    }

    /// BUG-1 regression: the composition body carries only the semantic What/Why —
    /// the channel-specific "how to reply" (and the gate ref) is appended once by
    /// the Slack adapter's `gate_prompt_reply_instruction`, so the body must NOT
    /// duplicate reply instructions or the gate ref (that caused the bloated,
    /// confusing, double-instruction message).
    #[test]
    fn slack_approval_prompt_body_carries_only_what_why_not_reply_instructions() {
        let gate_ref = GateRef::new("gate:approve-body-test").expect("gate ref");
        let prompt = slack_approval_gate_prompt_view(TurnRunId::new(), &gate_ref, None);
        let body = &prompt.body;

        // No reply instructions in the body — that is the adapter footer's job.
        assert!(
            !body.contains("approve") && !body.contains("deny"),
            "body must not contain reply instructions; got: {body}"
        );
        // No gate ref in the body (the footer renders it).
        assert!(
            !body.contains("gate:approve-body-test"),
            "body must not contain the gate ref; got: {body}"
        );
        // No legacy misleading copy.
        assert!(
            !body.contains("from anywhere"),
            "body must not claim bare `approve` works from anywhere; got: {body}"
        );
    }

    /// BUG-2 regression: when approval context is provided, the prompt body must
    /// include the action and reason, and approval_context must be Some.
    #[test]
    fn slack_approval_prompt_body_includes_context_when_provided() {
        let gate_ref = GateRef::new("gate:approve-ctx-test").expect("gate ref");
        let context = ApprovalPromptContextView::new(
            "Send email via Gmail",
            ironclaw_product_adapters::ApprovalPromptActionView::new("Send email via Gmail", None)
                .expect("action view"),
            ironclaw_product_adapters::ApprovalPromptScopeView::new("once", false)
                .expect("scope view"),
            Some("Automation step needs to notify the team".to_string()),
            None,
            vec![],
        )
        .expect("context view");
        let prompt = slack_approval_gate_prompt_view(TurnRunId::new(), &gate_ref, Some(&context));
        let body = &prompt.body;

        assert!(
            body.contains("Send email via Gmail"),
            "body must include tool name from context; got: {body}"
        );
        assert!(
            body.contains("Automation step needs to notify the team"),
            "body must include reason from context; got: {body}"
        );
        assert!(
            prompt.approval_context.is_some(),
            "approval_context must be Some when action is available"
        );
    }

    /// BUG-2: when context is None, body falls back to generic text and
    /// approval_context is None.
    #[test]
    fn slack_approval_prompt_body_generic_when_no_context() {
        let gate_ref = GateRef::new("gate:approve-no-ctx").expect("gate ref");
        let prompt = slack_approval_gate_prompt_view(TurnRunId::new(), &gate_ref, None);
        assert!(
            prompt.approval_context.is_none(),
            "approval_context must be None when context is absent"
        );
        // Generic body is a short fallback sentence — reply instructions live in
        // the adapter footer, not the body.
        assert!(
            prompt.body.contains("needs your approval"),
            "body must be the generic approval sentence; got: {}",
            prompt.body
        );
        assert!(
            !prompt.body.contains("approve") && !prompt.body.contains("deny"),
            "generic body must not contain reply instructions; got: {}",
            prompt.body
        );
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

        // Regression: the triggered approval prompt now renders through the same
        // shared `slack_approval_gate_prompt_view` as the regular inbound flow.
        // With no approval context wired (approval_requests: None) the body is
        // the shared generic fallback — NOT the old inline
        // "Reply `approve <gate>` to continue." body that had drifted from live.
        let approval_post_body = egress
            .calls()
            .iter()
            .find(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .expect("approval prompt must be posted");
        assert!(
            approval_post_body.contains("A step in this workflow needs your approval to continue"),
            "triggered approval prompt must use the shared gate-prompt render; got: {approval_post_body}"
        );
        // Every triggered Slack message carries the triggered-event footer
        // naming the surface contract (act here; interact via the web app).
        assert!(
            approval_post_body.contains("From a triggered event")
                && approval_post_body.contains("Ironclaw web app"),
            "triggered message must carry the triggered-event/web-app footer; got: {approval_post_body}"
        );

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
    async fn triggered_non_oauth_auth_is_denied_without_gate_route() {
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
        // Non-OAuth auth (no `authorization_url`) is DENIED over Slack: the run is
        // cancelled and an "auth unavailable" notice is posted instead of an auth
        // prompt, so NO gate route is recorded (there is nothing to resolve
        // in-thread). OAuth auth (which carries a URL) is what records a route.
        let route = route_store
            .load_delivered_gate_route(&scope.tenant_id, &creator, gate_ref_str)
            .await
            .expect("load route");
        assert!(
            route.is_none(),
            "non-OAuth auth must NOT record a gate route on a triggered run"
        );

        let posted: Vec<String> = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .collect();
        assert!(
            posted
                .iter()
                .any(|b| b.contains(SLACK_AUTH_UNAVAILABLE_MESSAGE)),
            "expected the auth-unavailable notice to be posted; got: {posted:?}"
        );
    }

    // ── BUG1 regression + OAuth backstop cancel-failure tests ─────────────────
    //
    // BUG1: when a triggered run reaches BlockedAuth with a non-OAuth challenge,
    // `triggered_notification_for_state` cancels the run inline and returns a
    // terminal FinalReplyReady notification. The delivery loop previously treated
    // any Some(next_blocked_marker) as "still waiting", causing the loop to
    // continue after a successful terminal delivery, read the now-Cancelled run
    // state, hit Ok(None), and record Skipped instead of Delivered.

    /// BUG1 regression: a triggered run that hits BlockedAuth with a non-OAuth
    /// challenge (no authorization_url) must record `Delivered` — NOT `Skipped`.
    ///
    /// The non-OAuth deny branch in `triggered_notification_for_state` cancels the
    /// run inline and returns a terminal `FinalReplyReady` notification. After the
    /// notice is successfully posted, the delivery loop must fall through to the
    /// terminal `Delivered` path rather than looping back and seeing the now-Cancelled
    /// run as `Ok(None)` → `Skipped`.
    #[tokio::test]
    async fn triggered_non_oauth_auth_denial_records_delivered() {
        let install = "test-install";
        let gate_ref_str = "gate:non-oauth-denial-delivered";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        // First poll → BlockedAuth (non-OAuth: no auth_challenges wired, so no
        // authorization_url → deny branch fires inline cancel + FinalReplyReady).
        // Second poll → Cancelled (terminal, no finalized message → Ok(None)).
        // Without the BUG1 fix the loop continues to the second poll and records
        // Skipped. With the fix, after the FinalReplyReady delivery the loop
        // falls through to Delivered without polling again.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![
            scripted_state(TurnStatus::BlockedAuth, Some(gate_ref_str)),
            scripted_state(TurnStatus::Cancelled, None),
        ]));
        // No finalized assistant message needed: the terminal delivery is the
        // auth-unavailable notice, not a Completed assistant reply.

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // One postMessage for the auth-unavailable notice.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "bug1.1"),
            )),
        );

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(
            coordinator.clone(),
            Arc::new(InMemorySessionThreadService::default()),
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
        let record = wait_for_delivery_record(&delivery_store, run_id).await;

        // BUG1 regression: outcome must be Delivered, not Skipped.
        assert_eq!(
            record.outcome,
            TriggeredRunDeliveryOutcomeKind::Delivered,
            "non-OAuth auth denial must record Delivered (not Skipped); got: {:?}",
            record.outcome
        );

        // The auth-unavailable notice must have been posted.
        let posted: Vec<String> = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .collect();
        assert!(
            posted
                .iter()
                .any(|b| b.contains(SLACK_AUTH_UNAVAILABLE_MESSAGE)),
            "auth-unavailable notice must be posted; got: {posted:?}"
        );

        // cancel_run was called exactly once (inline by triggered_notification_for_state).
        assert_eq!(
            coordinator.cancel_call_count(),
            1,
            "cancel_run must be called exactly once for non-OAuth auth denial"
        );
    }

    /// Triggered non-OAuth `BlockedAuth` → `cancel_auth_blocked_run` invokes the
    /// `BlockedAuthFlowCanceller` for the blocked gate (#4952).
    ///
    /// Drives the same `triggered_notification_for_state` non-OAuth branch as
    /// `triggered_non_oauth_auth_denial_records_delivered`, but this time a
    /// `RecordingBlockedAuthFlowCanceller` is wired so we can assert the stale
    /// auth-flow record is cancelled.
    #[tokio::test]
    async fn triggered_non_oauth_auth_cancels_stale_auth_flow() {
        let install = "test-install";
        let gate_ref_str = "gate:triggered-non-oauth-stale-flow";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        // First poll → BlockedAuth (non-OAuth: no auth_challenges, so no
        // authorization_url → deny branch in triggered_notification_for_state).
        // Second poll → Cancelled (terminal, no message → Ok(None)).
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![
            scripted_state(TurnStatus::BlockedAuth, Some(gate_ref_str)),
            scripted_state(TurnStatus::Cancelled, None),
        ]));

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // One postMessage for the auth-unavailable notice.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "noa1.1"),
            )),
        );

        let recorder = Arc::new(RecordingBlockedAuthFlowCanceller::default());
        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services_with_canceller(
            coordinator.clone(),
            Arc::new(InMemorySessionThreadService::default()),
            egress.clone(),
            outbound,
            install,
            Some(Arc::clone(&recorder) as Arc<dyn BlockedAuthFlowCanceller>),
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

        // The stale auth flow must have been cancelled exactly once.
        let calls = recorder
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            calls.len(),
            1,
            "triggered non-OAuth auth deny must cancel the stale auth flow exactly once; got {} calls",
            calls.len()
        );
        assert_eq!(
            calls[0].run_id, run_id,
            "canceller must receive the triggered run's run_id"
        );
        assert_eq!(
            calls[0].gate_ref, gate_ref_str,
            "canceller must receive the blocked gate_ref"
        );
        // FIX 2: Assert the resolved owner_user_id and scope match the fixture values.
        //
        // In the triggered path, `deliver_triggered_run` builds:
        //   actor = TurnActor::new(fire.creator_user_id) = "creator-user"
        // `personal_turn_scope()` sets explicit owner = "creator-user", so
        // `scope.explicit_owner_user_id() = Some("creator-user")` wins at
        // production line 1167 (`cancel_auth_blocked_run` owner resolution).
        let expected_owner =
            ironclaw_host_api::UserId::new("creator-user").expect("expected owner fixture");
        assert_eq!(
            calls[0].owner_user_id, expected_owner,
            "owner_user_id must be the scope's explicit owner (creator-user from personal_turn_scope)"
        );
        // Scope tenant must match personal_turn_scope().
        assert_eq!(
            calls[0].scope.tenant_id, scope.tenant_id,
            "scope.tenant_id must match the personal_turn_scope tenant"
        );
    }

    /// OAuth backstop cancel-failure path: when `cancel_auth_blocked_run` fails in
    /// the `OAuthTargetNotDm` error arm, the outcome must be `Failed` and NO
    /// `/api/chat.delete` calls must be made (we must not strip the auth prompt
    /// while the run may still be live).
    #[tokio::test]
    async fn triggered_oauth_auth_backstop_cancel_failure_records_failed() {
        let install = "test-install";
        let gate_ref_str = "gate:oauth-backstop-cancel-fail";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();

        // Use a SHARED CHANNEL binding ref so the OAuth backstop trips.
        let binding_ref = test_slack_shared_channel_binding_ref(
            install,
            scope.agent_id.as_ref().expect("agent").as_str(),
        );

        // First poll → BlockedAuth; second poll is never reached because the
        // cancel fails and we return Failed immediately.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some(gate_ref_str),
        )]));
        // Make cancel_run fail so the OAuthTargetNotDm backstop arm returns Failed.
        coordinator
            .cancel_should_fail
            .store(true, std::sync::atomic::Ordering::Release);

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // Seed preference with the shared-channel binding so the OAuth guard trips.
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // The initial auth-prompt delivery to the shared channel returns success
        // (the backstop fires only after the delivery attempt when the authority
        // detects the non-DM binding).  We do NOT program a postMessage response
        // because the backstop intercepts BEFORE delivery via the
        // `require_personal_dm_for_oauth` guard — no actual HTTP call is made.
        // (The guard is checked inside `deliver_triggered_notification`; it
        // returns `OAuthTargetNotDm` without posting.)

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let mut services = make_services(
            coordinator.clone(),
            Arc::new(InMemorySessionThreadService::default()),
            egress.clone(),
            outbound,
            install,
        );
        // Wire up an OAuth challenge provider so the BlockedAuth state generates
        // an authorization_url, triggering the DM-only guard.
        services.auth_challenges = Some(Arc::new(OAuthAuthChallengeProvider {
            url: "https://provider.example/oauth-cancel-fail".to_string(),
        }));

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
        let record = wait_for_delivery_record(&delivery_store, run_id).await;

        // The cancel failed → outcome must be Failed.
        assert_eq!(
            record.outcome,
            TriggeredRunDeliveryOutcomeKind::Failed,
            "OAuth backstop cancel failure must record Failed; got: {:?}",
            record.outcome
        );

        // No chat.delete calls: the auth prompt must not be removed when the
        // cancel failed (the run may still be live).
        let delete_call_count = egress
            .calls()
            .into_iter()
            .filter(|c| c.path == "/api/chat.delete")
            .count();
        assert_eq!(
            delete_call_count, 0,
            "no chat.delete must be issued when backstop cancel fails; got {delete_call_count} calls"
        );

        // cancel_run was attempted exactly once.
        assert_eq!(
            coordinator.cancel_call_count(),
            1,
            "cancel_run must be attempted exactly once in the backstop arm"
        );
    }

    /// OAuth `OAuthTargetNotDm` backstop → `BlockedAuthFlowCanceller` is invoked
    /// to cancel the stale auth-flow record alongside the run (#4952).
    ///
    /// Models on `triggered_oauth_auth_backstop_cancel_failure_records_failed` but
    /// wires a `RecordingBlockedAuthFlowCanceller` (no cancel_run failure) so the
    /// backstop succeeds and we can assert the canceller was called for the correct
    /// gate_ref.
    #[tokio::test]
    async fn triggered_oauth_backstop_cancels_stale_auth_flow() {
        let install = "test-install";
        let gate_ref_str = "gate:oauth-backstop-stale-flow";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();

        // Use a SHARED CHANNEL binding ref so the OAuth backstop trips
        // (OAuthTargetNotDm is returned before any HTTP post is made).
        let binding_ref = test_slack_shared_channel_binding_ref(
            install,
            scope.agent_id.as_ref().expect("agent").as_str(),
        );

        // First poll → BlockedAuth; second poll → Cancelled (after cancel_run).
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![
            scripted_state(TurnStatus::BlockedAuth, Some(gate_ref_str)),
            scripted_state(TurnStatus::Cancelled, None),
        ]));
        // cancel_run must succeed so the backstop posts the unavailable notice.

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        // Seed preference with the shared-channel binding so the OAuth guard trips.
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // One postMessage for the auth-unavailable notice (the backstop posts
        // after a successful cancel_run).
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("C0SHARED", "obs2.1"),
            )),
        );

        let recorder = Arc::new(RecordingBlockedAuthFlowCanceller::default());
        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let mut services = make_services_with_canceller(
            coordinator.clone(),
            Arc::new(InMemorySessionThreadService::default()),
            egress.clone(),
            outbound,
            install,
            Some(Arc::clone(&recorder) as Arc<dyn BlockedAuthFlowCanceller>),
        );
        // Wire up an OAuth challenge provider so the BlockedAuth state generates
        // an authorization_url, triggering the DM-only guard.
        services.auth_challenges = Some(Arc::new(OAuthAuthChallengeProvider {
            url: "https://provider.example/oauth-backstop-stale".to_string(),
        }));

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

        // The stale auth flow must have been cancelled exactly once.
        let calls = recorder
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            calls.len(),
            1,
            "OAuth backstop must cancel the stale auth flow exactly once; got {} calls",
            calls.len()
        );
        assert_eq!(
            calls[0].run_id, run_id,
            "canceller must receive the triggered run's run_id"
        );
        assert_eq!(
            calls[0].gate_ref, gate_ref_str,
            "canceller must receive the blocked gate_ref"
        );
    }

    // ── DM-gate security tests ─────────────────────────────────────────────────
    //
    // These tests verify the fail-closed gate that prevents OAuth
    // `authorization_url` values from leaking onto shared Slack channels.

    /// Fake `AuthChallengeProvider` that always returns an OAuth challenge
    /// with the given `authorization_url`.
    struct OAuthAuthChallengeProvider {
        url: String,
    }

    #[async_trait]
    impl AuthChallengeProvider for OAuthAuthChallengeProvider {
        async fn challenge_for_gate(
            &self,
            _scope: &TurnScope,
            _owner_user_id: &ironclaw_host_api::UserId,
            _run_id: TurnRunId,
            _gate_ref: &str,
            _credential_requirements: &[ironclaw_host_api::RuntimeCredentialAuthRequirement],
        ) -> Result<
            Option<crate::product_auth::api::auth_prompt::AuthChallengeView>,
            ironclaw_auth::AuthProductError,
        > {
            Ok(Some(
                crate::product_auth::api::auth_prompt::AuthChallengeView {
                    kind: ironclaw_product_adapters::AuthPromptChallengeKind::OAuthUrl,
                    provider: ironclaw_auth::AuthProviderId::new("test-provider")
                        .expect("provider"),
                    account_label: None,
                    authorization_url: Some(
                        ironclaw_auth::OAuthAuthorizationUrl::new(self.url.clone()).expect("url"),
                    ),
                    expires_at: None,
                },
            ))
        }
    }

    /// Build a shared-channel binding ref for use in tests (no actor segments).
    fn test_slack_shared_channel_binding_ref(
        installation_id: &str,
        agent_id: &str,
    ) -> ReplyTargetBindingRef {
        fn seg(name: &str, value: &str) -> String {
            format!("{}:{}:{};", name, value.len(), value)
        }
        let raw = format!(
            "{}{}{}{}{}{}{}",
            seg("adapter", "slack_v2"),
            seg("installation", installation_id),
            seg("agent", agent_id),
            seg("project", ""),
            seg("space", "T123"),
            seg("conversation", "C0SHARED"),
            seg("topic", ""),
        );
        crate::slack::slack_outbound_targets::slack_reply_target_binding_ref_from_raw(raw)
            .expect("test shared-channel binding ref")
    }

    /// Security regression: triggered OAuth auth whose delivery target is a SHARED
    /// CHANNEL must NOT post the `authorization_url`. The run must be cancelled and
    /// the auth-unavailable notice must be posted instead.
    ///
    /// This is the "fail closed" path: if the binding ref does not parse as a
    /// personal DM, the OAuth URL is suppressed regardless of `authorization_url`
    /// being set.
    #[tokio::test]
    async fn triggered_oauth_auth_to_shared_channel_suppresses_authorization_url() {
        let install = "test-install";
        let gate_ref_str = "gate:oauth-shared-channel-leak";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();

        // Use a shared-channel binding ref (no actor_kind / actor segments).
        let binding_ref = test_slack_shared_channel_binding_ref(
            install,
            scope.agent_id.as_ref().expect("agent").as_str(),
        );

        // First poll → BlockedAuth; second poll → Completed (after cancel the run
        // reaches Completed or the test exits — we only care about what is posted).
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
        // Seed the preference with a SHARED CHANNEL binding ref (not a DM).
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Expect one chat.postMessage call for the auth-unavailable notice.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("C0SHARED", "7001.1"),
            )),
        );
        // And a second for the final reply after Completed.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("C0SHARED", "7001.2"),
            )),
        );

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let mut services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        // Wire up an OAuth challenge provider so the BlockedAuth state WOULD
        // produce an authorization_url — the gate must suppress it.
        services.auth_challenges = Some(Arc::new(OAuthAuthChallengeProvider {
            url: "https://provider.example/oauth-auth".to_string(),
        }));

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

        let posted: Vec<String> = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .collect();

        // The OAuth URL must NOT appear in any posted message.
        for body in &posted {
            assert!(
                !body.contains("https://provider.example/oauth-auth"),
                "authorization_url must NOT be posted to a shared channel; got: {body}"
            );
        }

        // The auth-unavailable notice must appear instead.
        assert!(
            posted
                .iter()
                .any(|b| b.contains(SLACK_AUTH_UNAVAILABLE_MESSAGE)),
            "auth-unavailable notice must be posted when OAuth is suppressed for non-DM target; \
             got: {posted:?}"
        );

        // No gate route must be recorded (the auth was cancelled).
        let creator = ironclaw_host_api::UserId::new("creator-user").expect("user id");
        let route = route_store
            .load_delivered_gate_route(&scope.tenant_id, &creator, gate_ref_str)
            .await
            .expect("load route");
        assert!(
            route.is_none(),
            "no gate route must be recorded when OAuth is suppressed for non-DM target"
        );
    }

    /// Positive case: triggered OAuth auth whose delivery target IS a personal DM
    /// must post the `authorization_url` (unchanged from pre-fix behavior).
    #[tokio::test]
    async fn triggered_oauth_auth_to_personal_dm_posts_authorization_url() {
        let install = "test-install";
        let gate_ref_str = "gate:oauth-dm-allowed";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        // Use the personal-DM binding ref (has actor_kind / actor segments, D-prefixed channel).
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        // First poll → BlockedAuth; second → Completed.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![
            scripted_state(TurnStatus::BlockedAuth, Some(gate_ref_str)),
            scripted_state(TurnStatus::Completed, None),
        ]));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_finalized_assistant_message(
            &thread_service,
            &scope,
            run_id,
            "Run complete after OAuth.",
        )
        .await;

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // OAuth auth-prompt response.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "8001.1"),
            )),
        );
        // Final-reply response.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D456", "8001.2"),
            )),
        );
        // Auth message deleted after final reply.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                serde_json::json!({"ok": true}).to_string().into_bytes(),
            )),
        );

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let mut services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        // Wire up an OAuth challenge provider so authorization_url is set.
        services.auth_challenges = Some(Arc::new(OAuthAuthChallengeProvider {
            url: "https://provider.example/oauth-auth-dm".to_string(),
        }));

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

        let posted: Vec<String> = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .collect();

        // The OAuth URL MUST appear in the auth-prompt message sent to the DM.
        assert!(
            posted
                .iter()
                .any(|b| b.contains("https://provider.example/oauth-auth-dm")),
            "authorization_url must be posted to a verified personal DM; got: {posted:?}"
        );

        // The auth-unavailable notice must NOT appear.
        assert!(
            !posted
                .iter()
                .any(|b| b.contains(SLACK_AUTH_UNAVAILABLE_MESSAGE)),
            "auth-unavailable notice must NOT appear when OAuth is sent to a personal DM; \
             got: {posted:?}"
        );
    }

    /// Precedence guard: when `auth_prompt_target` is a SHARED CHANNEL but
    /// `final_reply_target` is a personal DM, the OAuth gate must key on the
    /// EFFECTIVE auth target (`auth_prompt_target.or(final_reply_target)` — see
    /// `resolution_engine.rs` `PreferenceTargetKind::AuthPrompt`), i.e. the shared
    /// channel. The URL must be SUPPRESSED. A naive "any stored target is a DM"
    /// check would wrongly pass here and leak the OAuth URL to the channel.
    #[tokio::test]
    async fn triggered_oauth_auth_prefers_auth_target_over_dm_fallback() {
        let install = "test-install";
        let gate_ref_str = "gate:oauth-auth-target-shared";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let agent = scope.agent_id.as_ref().expect("agent").as_str();
        // auth_prompt_target → shared channel (the effective auth target);
        // final_reply_target → personal DM (must NOT rescue the OAuth post).
        let auth_target = test_slack_shared_channel_binding_ref(install, agent);
        let final_target = test_slack_binding_ref(install, agent);

        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![
            scripted_state(TurnStatus::BlockedAuth, Some(gate_ref_str)),
            scripted_state(TurnStatus::Completed, None),
        ]));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_finalized_assistant_message(&thread_service, &scope, run_id, "Run complete.").await;

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference_with_auth_target(&outbound, &scope, auth_target, final_target)
            .await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // auth-unavailable notice, then final reply.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("C0SHARED", "9001.1"),
            )),
        );
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("C0SHARED", "9001.2"),
            )),
        );

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let mut services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        services.auth_challenges = Some(Arc::new(OAuthAuthChallengeProvider {
            url: "https://provider.example/oauth-auth-target".to_string(),
        }));

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

        driver
            .on_trigger_submitted(minimal_trigger_fire(None), run_id, scope.clone())
            .await;
        wait_for_delivery_record(&delivery_store, run_id).await;

        let posted: Vec<String> = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .collect();

        for body in &posted {
            assert!(
                !body.contains("https://provider.example/oauth-auth-target"),
                "authorization_url must NOT post when the effective auth target is a shared \
                 channel, even if final_reply_target is a DM; got: {body}"
            );
        }
        assert!(
            posted
                .iter()
                .any(|b| b.contains(SLACK_AUTH_UNAVAILABLE_MESSAGE)),
            "auth-unavailable notice must be posted when OAuth is suppressed; got: {posted:?}"
        );
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
    /// `busy_hint_from_run_state`: when `thread_scope_from_binding` fails
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
    /// `busy_hint_from_run_state` calls `thread_scope_from_binding` which
    /// returns `Err` when `agent_id` is `None`. The code must fall back to
    /// `SLACK_BUSY_GENERIC_MESSAGE` and still post the hint.
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
            auth_flow_canceller: None,
            approval_requests: None,
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
    /// `busy_hint_from_run_state` swallows `TurnError` from
    /// `get_run_state` and degrades to `SLACK_BUSY_GENERIC_MESSAGE`.
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
            auth_flow_canceller: None,
            approval_requests: None,
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

    /// Two DeferredBusy acks for the same conversation and the same external_event_id
    /// (simulating a Slack transport retry) → exactly one post (throttle suppresses the retry).
    ///
    /// Both envelopes use event id "evt:test" (the default in `envelope()`), so they share
    /// the same (conversation, event_id) throttle key. The active_run_id is irrelevant to
    /// dedup with the new event-id-based throttle.
    #[tokio::test]
    async fn deferred_busy_same_conversation_same_event_id_posts_once() {
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
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);

        // Both acks carry a fresh run_id — the throttle must still suppress the
        // second post because the event_id ("evt:test") is identical.
        let make_ack = || ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("slack:deferred-same-evt").expect("ref"),
            active_run_id: TurnRunId::new(),
        };

        // First delivery (event "evt:test"): posts.
        observer
            .observe_workflow_ack(envelope(user_message_payload()), make_ack())
            .await;
        // Second delivery (same event "evt:test", different run_id): throttled, no second post.
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
            "throttle must suppress the second hint for the same (conversation, event_id)"
        );
    }

    /// Two DeferredBusy acks for the same conversation but different external_event_ids
    /// → two posts (distinct throttle keys: dedup is per event, not per run).
    ///
    /// The active_run_id is the same across both calls to prove that distinct run_ids
    /// are no longer the gate for separate hints — distinct event_ids are.
    #[tokio::test]
    async fn deferred_busy_same_conversation_different_event_id_posts_twice() {
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

        let shared_run = TurnRunId::new();
        let make_ack = || ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("slack:deferred-diff-evt").expect("ref"),
            active_run_id: shared_run,
        };

        // First new user message (event "evt:diff-1") → hint posted.
        observer
            .observe_workflow_ack(
                envelope_with_event_id("evt:diff-1", user_message_payload()),
                make_ack(),
            )
            .await;
        // Second new user message (event "evt:diff-2") → distinct event id → separate hint.
        observer
            .observe_workflow_ack(
                envelope_with_event_id("evt:diff-2", user_message_payload()),
                make_ack(),
            )
            .await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            2,
            "different event_ids must produce separate hints even for the same conversation and run_id"
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
            auth_flow_canceller: None,
            approval_requests: None,
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
            auth_flow_canceller: None,
            approval_requests: None,
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

    // ── BUG-3 regression: StaleGate produces a distinct hint ──────────────────

    /// `ProductRejectionKind::StaleGate` on an approval resolution must produce the
    /// "no longer pending" copy, NOT the generic "declined by policy" wording.
    #[test]
    fn stale_gate_rejection_hint_is_distinct_from_policy_denied() {
        // Approval resolution payload + Rejected(StaleGate) → the stale-gate hint.
        let payload = approval_resolution_payload();
        let env = envelope(payload);
        let ack = rejected_ack(ProductRejectionKind::StaleGate);

        let hint = rejection_hint_for_resolution(&env, &ack);
        assert!(hint.is_some(), "StaleGate rejection must produce a hint");
        let hint = hint.unwrap();

        assert!(
            hint.contains("no longer pending"),
            "StaleGate hint must mention 'no longer pending'; got: {hint}"
        );
        assert!(
            !hint.contains("policy"),
            "StaleGate hint must NOT fall through to 'policy' wording; got: {hint}"
        );
    }

    /// `ProductRejectionKind::StaleGate` on a scoped-approval resolution must also
    /// produce the distinct hint (not policy wording).
    #[test]
    fn stale_gate_scoped_approval_rejection_hint_is_distinct() {
        let payload = scoped_approval_resolution_payload();
        let env = envelope(payload);
        let ack = rejected_ack(ProductRejectionKind::StaleGate);

        let hint = rejection_hint_for_resolution(&env, &ack);
        let hint = hint.expect("StaleGate on scoped-approval must produce a hint");

        assert!(
            hint.contains("no longer pending"),
            "StaleGate scoped-approval hint must mention 'no longer pending'; got: {hint}"
        );
        assert!(
            !hint.contains("policy"),
            "StaleGate scoped-approval hint must NOT say 'policy'; got: {hint}"
        );
    }

    // ── BUG-4/5 regression: per-event dedup — new message always gets a hint ──

    /// Each distinct Slack event (new human message) gets a fresh hint even while the
    /// same run is blocking.  This is the core BUG-4/5 fix: the throttle now keys on
    /// external_event_id, not active_run_id.
    ///
    /// Three messages with distinct event ids → three hints, despite the same conversation
    /// and the same blocking run.
    #[tokio::test]
    async fn each_new_human_message_gets_its_own_hint_for_same_blocking_run() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        for i in 1u32..=3 {
            egress.program_response(
                "slack.com",
                Ok(EgressResponse::new(
                    200,
                    slack_post_ok_json("D123", &format!("evt-bug45-{i}.0")),
                )),
            );
        }

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let blocking_run_id = TurnRunId::new();
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);

        // Shared active_run_id — the same run is blocking for all three messages.
        let make_ack = || ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("slack:bug45-hint").expect("ref"),
            active_run_id: blocking_run_id,
        };

        // Three distinct new human messages while the run is blocked.
        for i in 1u32..=3 {
            observer
                .observe_workflow_ack(
                    envelope_with_event_id(&format!("evt:bug45-msg-{i}"), user_message_payload()),
                    make_ack(),
                )
                .await;
        }

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            3,
            "each distinct new human message must produce its own hint (BUG-4/5 fix); got {} posts",
            post_calls.len()
        );
    }

    /// Transport retry of the SAME event must still be deduplicated by the
    /// (conversation, event_id) key — no double-post.
    #[tokio::test]
    async fn transport_retry_of_same_event_is_deduplicated() {
        let install = "test-install";
        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Only one response slot — a second post attempt would fail.
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("D123", "retry-dedup.1"),
            )),
        );

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        let blocking_run_id = TurnRunId::new();
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::BlockedApproval,
        ));
        let observer = make_observer(coordinator, egress.clone(), outbound, install);

        let make_ack = || ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("slack:retry-dedup").expect("ref"),
            active_run_id: blocking_run_id,
        };

        // First delivery (original).
        observer
            .observe_workflow_ack(
                envelope_with_event_id("evt:retry-event-X", user_message_payload()),
                make_ack(),
            )
            .await;
        // Second delivery (Slack transport retry) — same event id → must be suppressed.
        observer
            .observe_workflow_ack(
                envelope_with_event_id("evt:retry-event-X", user_message_payload()),
                make_ack(),
            )
            .await;

        let calls = egress.calls();
        let post_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .collect();
        assert_eq!(
            post_calls.len(),
            1,
            "transport retry of the same event must be deduplicated (only one hint posted)"
        );
    }

    // ── Authority backstop tests ──────────────────────────────────────────────
    //
    // These tests cover the `require_personal_dm_for_oauth` backstop in
    // `TriggeredSlackReplyTargetAuthority::resolve_product_outbound_target_metadata`,
    // which is now the single enforcement point ensuring OAuth authorization_urls
    // only reach personal DMs. The pre-loop snapshot was removed; the backstop is
    // authoritative.

    /// Backstop regression: when the send-time binding resolves to a shared
    /// channel, the `require_personal_dm_for_oauth` backstop must catch it and
    /// suppress the OAuth URL. The run must be cancelled and the auth-unavailable
    /// notice must be posted. No gate route must be recorded.
    ///
    /// Previously named `triggered_oauth_auth_dm_snapshot_but_channel_at_send_suppresses_url`
    /// (tested the snapshot-vs-send race); simplified now that the backstop is the
    /// only enforcement point and no pre-loop snapshot exists.
    #[tokio::test]
    async fn triggered_oauth_auth_dm_snapshot_but_channel_at_send_suppresses_url() {
        let install = "test-install";
        let gate_ref_str = "gate:oauth-race-snapshot-dm-send-channel";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let agent = scope.agent_id.as_ref().expect("agent").as_str();

        // Shared-channel binding: the backstop must catch this at send time.
        let shared_binding = test_slack_shared_channel_binding_ref(install, agent);

        // First poll → BlockedAuth with OAuth gate; second poll → Completed.
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
        seed_personal_preference(&outbound, &scope, shared_binding).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // auth-unavailable notice (after backstop trip).
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("C0SHARED", "race-test.1"),
            )),
        );
        // Second response (available if loop re-runs; not required to be consumed).
        egress.program_response(
            "slack.com",
            Ok(EgressResponse::new(
                200,
                slack_post_ok_json("C0SHARED", "race-test.2"),
            )),
        );

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let mut services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        // Wire up an OAuth challenge provider so authorization_url would be set
        // if the backstop were absent.
        services.auth_challenges = Some(Arc::new(OAuthAuthChallengeProvider {
            url: "https://provider.example/oauth-race".to_string(),
        }));

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

        driver
            .on_trigger_submitted(minimal_trigger_fire(None), run_id, scope.clone())
            .await;
        wait_for_delivery_record(&delivery_store, run_id).await;

        let posted: Vec<String> = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .collect();

        // The OAuth URL must NOT appear in any posted message.
        for body in &posted {
            assert!(
                !body.contains("https://provider.example/oauth-race"),
                "authorization_url must NOT be posted when send-time target is a shared channel \
                 (backstop); got: {body}"
            );
        }

        // The auth-unavailable notice must appear (backstop tripped).
        assert!(
            posted
                .iter()
                .any(|b| b.contains(SLACK_AUTH_UNAVAILABLE_MESSAGE)),
            "auth-unavailable notice must be posted when backstop suppresses OAuth URL; \
             got: {posted:?}"
        );

        // No gate route must be recorded (the auth was cancelled).
        let creator = ironclaw_host_api::UserId::new("creator-user").expect("user id");
        let route = route_store
            .load_delivered_gate_route(&scope.tenant_id, &creator, gate_ref_str)
            .await
            .expect("load route");
        assert!(
            route.is_none(),
            "no gate route must be recorded when backstop cancels OAuth delivery"
        );
    }

    // Removed: triggered_oauth_auth_preference_read_error_suppresses_authorization_url
    // — tested the pre-loop snapshot fail-closed behavior on preference-read error;
    // redundant now that the snapshot was removed and the backstop is the sole
    // enforcement point (shared-channel delivery is already covered by
    // `triggered_oauth_auth_to_shared_channel_suppresses_authorization_url`).

    // Removed: triggered_oauth_auth_no_preference_suppresses_authorization_url
    // — tested the pre-loop snapshot fail-closed behavior for an absent preference
    // record; redundant after snapshot removal for the same reason as above.

    // ── enforce_direct_message_if_required ────────────────────────────────────
    //
    // Direct unit tests for the shared helper that both ObservedSlackReplyTargetAuthority
    // and TriggeredSlackReplyTargetAuthority delegate to (Fix 3 / Fix 6).
    //
    // The helper takes `&ReplyTargetBindingRef` so no ValidatedReplyTargetBinding
    // scaffolding is required — we test the guard logic directly.

    #[test]
    fn enforce_direct_message_shared_channel_require_true_returns_err() {
        let install = "test-install";
        let agent = "test-agent";
        let binding_ref = test_slack_shared_channel_binding_ref(install, agent);
        let result = enforce_direct_message_if_required(&binding_ref, true);
        assert!(
            matches!(
                result,
                Err(ProductWorkflowError::OutboundTargetNotDirectMessage)
            ),
            "shared channel + require=true must return OutboundTargetNotDirectMessage"
        );
    }

    #[test]
    fn enforce_direct_message_shared_channel_require_false_returns_ok() {
        let install = "test-install";
        let agent = "test-agent";
        let binding_ref = test_slack_shared_channel_binding_ref(install, agent);
        let result = enforce_direct_message_if_required(&binding_ref, false);
        assert!(
            result.is_ok(),
            "shared channel + require=false must not be rejected"
        );
    }

    #[test]
    fn enforce_direct_message_dm_binding_require_true_returns_ok() {
        let install = "test-install";
        let agent = "test-agent";
        let binding_ref = test_slack_binding_ref(install, agent);
        let result = enforce_direct_message_if_required(&binding_ref, true);
        assert!(
            result.is_ok(),
            "personal DM binding + require=true must not be rejected"
        );
    }

    // ── Bug reproduction: persistent BlockedApproval never resolves ────────────

    /// Regression for the Slack triggered-run "blocked-forever → Failed" bug.
    ///
    /// A triggered run enters `BlockedApproval` and STAYS there (the user never
    /// approves — the common case). The delivery loop posts the approval prompt
    /// once, then re-waits for the run to leave the blocked state. Before the fix,
    /// that re-wait polled to `max_wait` (30 min in production) and recorded
    /// `Failed`, clobbering the `Delivered` it had already earned — see the 23×
    /// "did not finish before Slack delivery timeout" → `outcome=Failed` in
    /// production logs (logs.1782348290172).
    ///
    /// Desired (post-fix) behavior: the approval prompt is posted at least once
    /// and the outcome is `Delivered` (the run is parked awaiting the user; its
    /// resolution arrives via a separate inbound event). The wait timeout after a
    /// blocked prompt was delivered must NOT record `Failed`.
    ///
    /// `max_wait` is set to 200ms so the test completes in milliseconds. With the
    /// bug present this test fails (`final_outcome == Failed`); with the fix it
    /// passes — verified red→green.
    #[tokio::test]
    async fn triggered_persistent_blocked_approval_records_delivered_not_failed() {
        let install = "test-install";
        let gate_ref_str = "gate:approval-stuck";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        // Always returns BlockedApproval — the user never approves.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some(gate_ref_str),
        )]));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        // No finalized assistant message: the run never completes.

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Program several OK responses so the approval prompt can be posted
        // (and any re-posts if the bug causes repeated delivery).
        for _ in 0..5 {
            egress.program_response(
                "slack.com",
                Ok(EgressResponse::new(
                    200,
                    slack_post_ok_json("D456", "8001.1"),
                )),
            );
        }

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        // Short max_wait so the test finishes in milliseconds instead of 30 min.
        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::from_millis(1),
            max_wait: std::time::Duration::from_millis(200),
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

        // Count how many chat.postMessage calls were made.
        let post_count = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .count();

        // Read the final recorded outcome.
        let record = delivery_store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("load record")
            .expect("record must exist after wait_for_delivery_record");
        let final_outcome = record.outcome;

        // The approval prompt is posted at least once AND the delivery is recorded
        // as Delivered (not Failed). With the bug present, post_count >= 1 but
        // final_outcome == Failed — the assertion below catches that.
        assert!(
            post_count >= 1,
            "approval prompt must be posted at least once; got post_count={post_count}"
        );
        assert_eq!(
            final_outcome,
            TriggeredRunDeliveryOutcomeKind::Delivered,
            "delivery must be recorded as Delivered when approval prompt was posted; \
             got {final_outcome:?} (post_count={post_count})"
        );
    }

    /// Production-faithful variant of the blocked-forever regression: the run is
    /// `Running` for the first 2 polls, THEN enters `BlockedApproval` and stays
    /// there forever — matching the production timeline (run executes briefly,
    /// then blocks on approval, then the wait backstop fires). The first wait that
    /// observes the block must post the approval prompt and the outcome must be
    /// `Delivered`, never `Failed`. Fails on the old behavior, passes after the fix.
    #[tokio::test]
    async fn triggered_running_then_blocked_approval_records_delivered_not_failed() {
        let install = "test-install";
        let gate_ref_str = "gate:approval-stuck-after-running";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        // Running for the first 2 polls, then sticky BlockedApproval forever
        // (clamped script: the last entry is returned indefinitely).
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states_clamped(vec![
            scripted_state(TurnStatus::Running, None),
            scripted_state(TurnStatus::Running, None),
            scripted_state(TurnStatus::BlockedApproval, Some(gate_ref_str)),
        ]));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        // No finalized assistant message: the run never completes.

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        for _ in 0..5 {
            egress.program_response(
                "slack.com",
                Ok(EgressResponse::new(
                    200,
                    slack_post_ok_json("D456", "8001.1"),
                )),
            );
        }

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
            poll_interval: std::time::Duration::from_millis(1),
            max_wait: std::time::Duration::from_millis(200),
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

        let posts: Vec<String> = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .collect();
        let post_count = posts.len();
        let approval_posted = posts.iter().any(|b| b.contains("needs your approval"));

        let record = delivery_store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("load record")
            .expect("record must exist after wait_for_delivery_record");
        let final_outcome = record.outcome;

        assert!(
            post_count >= 1,
            "approval prompt must be posted at least once even when run starts Running; \
             got post_count={post_count}"
        );
        assert!(
            approval_posted,
            "the posted message must be the approval prompt; got posts: {posts:?}"
        );
        assert_eq!(
            final_outcome,
            TriggeredRunDeliveryOutcomeKind::Delivered,
            "delivery must be recorded as Delivered when approval prompt was posted; \
             got {final_outcome:?} (post_count={post_count})"
        );
    }

    /// Backstop-preserved guard: a triggered run that is `Running` and never
    /// reaches an actionable (terminal or Blocked*) state must still time out and
    /// record `Failed` with ZERO posts. This proves the fix only flips the
    /// *post-actionable* wait timeout to `Delivered` — the genuine
    /// never-actionable backstop (the real failure signal) is unchanged.
    #[tokio::test]
    async fn triggered_never_actionable_run_times_out_failed() {
        let install = "test-install";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        // Always Running, no gate ref: the run never becomes actionable.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_single_status(
            TurnStatus::Running,
        ));
        let thread_service = Arc::new(InMemorySessionThreadService::default());

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

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
            poll_interval: std::time::Duration::from_millis(1),
            max_wait: std::time::Duration::from_millis(50),
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

        let post_count = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .count();
        let final_outcome = delivery_store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("load record")
            .expect("record must exist")
            .outcome;

        assert_eq!(
            post_count, 0,
            "a never-actionable run must not post anything; got post_count={post_count}"
        );
        assert_eq!(
            final_outcome,
            TriggeredRunDeliveryOutcomeKind::Failed,
            "a run that never reaches an actionable state must time out as Failed; \
             got {final_outcome:?}"
        );
    }

    /// Auth/OAuth variant of the blocked-forever regression. The new
    /// `RunWaitTimedOut + delivered_blocked_marker.is_some()` arm fires for BOTH
    /// `BlockedApproval` and `BlockedAuth` (line 2230 sets the marker for the
    /// `AuthRequired` event kind too). A triggered run that posts its OAuth
    /// auth prompt to a personal DM and then stays `BlockedAuth` forever (the user
    /// never re-authenticates) must record `Delivered`, not `Failed` — same parked
    /// invariant as the approval case. Guards against a future change that scopes
    /// the guard to `ApprovalNeeded` only. Fails on the old behavior, passes after
    /// the fix.
    #[tokio::test]
    async fn triggered_persistent_blocked_oauth_auth_records_delivered_not_failed() {
        let install = "test-install";
        let gate_ref_str = "gate:oauth-stuck";
        let scope = personal_turn_scope();
        let run_id = TurnRunId::new();
        // Personal-DM binding so the OAuth DM-only send-time guard does NOT trip.
        let binding_ref =
            test_slack_binding_ref(install, scope.agent_id.as_ref().expect("agent").as_str());

        // Always BlockedAuth with a gate ref: the user never re-authenticates.
        let coordinator = Arc::new(ScriptedTurnCoordinator::with_states(vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some(gate_ref_str),
        )]));
        let thread_service = Arc::new(InMemorySessionThreadService::default());

        let outbound = Arc::new(InMemoryOutboundStateStore::default());
        seed_personal_preference(&outbound, &scope, binding_ref).await;

        let egress = Arc::new(FakeProtocolHttpEgress::new(vec!["slack.com".to_string()]));
        egress.allow_credential_handle("slack_bot_token");
        // Program OK responses so the OAuth auth prompt can be posted.
        for _ in 0..5 {
            egress.program_response(
                "slack.com",
                Ok(EgressResponse::new(
                    200,
                    slack_post_ok_json("D456", "8001.1"),
                )),
            );
        }

        let delivery_store = Arc::new(InMemoryTriggeredRunDeliveryStore::default());
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let mut services = make_services(
            coordinator,
            thread_service,
            egress.clone(),
            outbound,
            install,
        );
        // OAuth challenge provider so the auth prompt carries an authorization_url
        // (the OAuth branch that sets `delivered_blocked_marker`).
        services.auth_challenges = Some(Arc::new(OAuthAuthChallengeProvider {
            url: "https://provider.example/oauth-stuck".to_string(),
        }));

        let settings = SlackFinalReplyDeliverySettings {
            poll_interval: std::time::Duration::from_millis(1),
            max_wait: std::time::Duration::from_millis(200),
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

        let posts: Vec<String> = egress
            .calls()
            .iter()
            .filter(|c| c.path == "/api/chat.postMessage")
            .map(|c| String::from_utf8_lossy(&c.body).to_string())
            .collect();
        let final_outcome = delivery_store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("load record")
            .expect("record must exist")
            .outcome;

        assert!(
            posts
                .iter()
                .any(|b| b.contains("https://provider.example/oauth-stuck")),
            "the OAuth authorization_url must be posted to the personal DM; got: {posts:?}"
        );
        assert_eq!(
            final_outcome,
            TriggeredRunDeliveryOutcomeKind::Delivered,
            "a run parked in BlockedAuth after its OAuth prompt was delivered must be \
             recorded as Delivered, not Failed; got {final_outcome:?}"
        );
    }
}

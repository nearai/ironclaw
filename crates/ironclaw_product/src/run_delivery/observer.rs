//! Immediate channel feedback for inbound messages.
//!
//! Long-lived run delivery is owned by the lifecycle-event router. This
//! observer handles only admission-time outcomes that have no durable run
//! lifecycle event: connection nudges, rejected resolutions, and busy hints.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tokio::time::Instant;

use crate::{
    AuthPromptChallengeKind, AuthResolutionResult, ExternalConversationRef, ExternalEventId,
    ProductAdapterError, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductRejection, ProductRejectionKind, ProductTriggerReason, ProductWorkflowRejectionKind,
    render_channel_auth_prompt,
};
use ironclaw_turns::{GetRunStateRequest, TurnRunId, TurnStatus};

use super::prompts;
use super::{
    HINT_SEEN_CAP, HintSeenSet, RunDeliveryServices, thread_scope_from_binding,
    turn_scope_from_thread_scope,
};
use crate::delivery_coordinator::DeliveryIntent;
use crate::{
    ChannelConnectionNoticePolicy, ProductConversationRouteKind, ResolveBindingRequest,
    ResolvedBinding,
};

const CONNECT_NOTICE_THROTTLE_WINDOW: std::time::Duration = std::time::Duration::from_secs(30);

const CONNECT_NUDGE_RESERVATION_CAP: usize = 1024;

/// Observes the immediate workflow acknowledgement for an inbound channel
/// message and posts admission-time feedback through the coordinator.
pub struct RunDeliveryObserver {
    services: RunDeliveryServices,
    connection_notices: ChannelConnectionNoticePolicy,
    /// Per-observer, per-conversation connect-nudge reservations. Reserving
    /// before delivery prevents concurrent unbound events from racing.
    connect_nudge_reservations: Mutex<HashMap<String, Instant>>,
    /// Per-observer throttle: at most one busy-thread hint per
    /// (conversation fingerprint, external_event_id) pair.
    hint_seen: HintSeenSet,
}

impl RunDeliveryObserver {
    pub fn new(services: RunDeliveryServices) -> Self {
        Self::with_connection_notices(
            services,
            ChannelConnectionNoticePolicy::generic("this channel"),
        )
    }

    pub fn with_connection_notices(
        services: RunDeliveryServices,
        connection_notices: ChannelConnectionNoticePolicy,
    ) -> Self {
        Self {
            services,
            connection_notices,
            connect_nudge_reservations: Mutex::new(HashMap::new()),
            hint_seen: Mutex::new((std::collections::VecDeque::new(), HashSet::new())),
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
        // Accepted auth denials do not produce an assistant reply. Acknowledge
        // the exact denial message here, while its source conversation and
        // thread are still authoritative. Duplicate transport deliveries must
        // not repeat the notice.
        if self.post_accepted_auth_denial_notice(&envelope, &ack).await {
            return;
        }
        // Rejected approval/auth feedback is a single best-effort post.
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
        }
    }

    async fn post_accepted_auth_denial_notice(
        &self,
        envelope: &ProductInboundEnvelope,
        ack: &ProductInboundAck,
    ) -> bool {
        let ProductInboundAck::Accepted {
            submitted_run_id, ..
        } = ack
        else {
            return false;
        };
        if !matches!(
            envelope.payload(),
            ProductInboundPayload::AuthResolution(payload)
                if matches!(&payload.result, AuthResolutionResult::Denied)
        ) {
            return false;
        }

        let binding = match crate::workflow::lookup_interaction_binding(
            envelope,
            self.services.binding_service.as_ref(),
        )
        .await
        {
            Ok(binding) => binding,
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    %submitted_run_id,
                    external_event_id = envelope.external_event_id().as_str(),
                    error = %error,
                    "skipped accepted auth-denial notice because the originating conversation binding could not be revalidated"
                );
                return true;
            }
        };
        let scope = match thread_scope_from_binding(&binding)
            .and_then(|thread_scope| turn_scope_from_thread_scope(&binding, &thread_scope))
        {
            Ok(scope) => scope,
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::reborn::run_delivery",
                    %submitted_run_id,
                    external_event_id = envelope.external_event_id().as_str(),
                    error = %error,
                    "skipped accepted auth-denial notice because its verified binding did not yield an authoritative turn scope"
                );
                return true;
            }
        };
        self.services
            .post_notice(
                DeliveryIntent::FailureNotice,
                scope,
                Some(*submitted_run_id),
                envelope.external_conversation_ref(),
                prompts::AUTH_CANCELED_MESSAGE,
                format!("auth-canceled:{}", envelope.external_event_id().as_str()),
            )
            .await;
        true
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
        let binding_request = ResolveBindingRequest::from_envelope(envelope);
        let direct = binding_request.route_kind == ProductConversationRouteKind::Direct;
        let (hint, scope) = match self
            .services
            .binding_service
            .lookup_binding(binding_request)
            .await
        {
            Ok(binding) => {
                let hint = self
                    .busy_hint_from_run_state(&binding, active_run_id, direct)
                    .await;
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
        direct: bool,
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
                TurnStatus::BlockedAuth => match state.gate_ref.as_ref() {
                    Some(gate_ref) => {
                        self.busy_auth_hint(
                            &binding.actor_user_id,
                            &state,
                            gate_ref.as_str(),
                            direct,
                        )
                        .await
                    }
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

    /// Re-project the same typed challenge as the original auth event when a
    /// user sends another message while the run is parked. This must not
    /// guess from provider names: the prompt source has already resolved the
    /// manifest auth recipe and materialized any safe channel challenge.
    async fn busy_auth_hint(
        &self,
        owner_user_id: &ironclaw_host_api::UserId,
        state: &ironclaw_turns::TurnRunState,
        gate_ref: &str,
        direct: bool,
    ) -> String {
        let Some(source) = &self.services.blocked_auth_prompts else {
            return prompts::AUTH_UNAVAILABLE_MESSAGE.to_string();
        };
        let view = source
            .auth_prompt_for_blocked_run(crate::auth_prompt::BlockedAuthPromptRequest {
                fallback_owner_user_id: owner_user_id,
                scope: &state.scope,
                run_id: state.run_id,
                gate_ref,
                invocation_id: None,
                body: "Authenticate to continue this run.".to_string(),
                credential_requirements: &state.credential_requirements,
            })
            .await;
        let Ok(mut view) = view else {
            tracing::debug!(
                target = "ironclaw::reborn::run_delivery",
                run_id = %state.run_id,
                "busy-thread auth hint challenge lookup failed; using safe WebUI copy"
            );
            return prompts::AUTH_UNAVAILABLE_MESSAGE.to_string();
        };

        if !prompts::auth_prompt_is_serviceable(&view) {
            return prompts::unserviceable_auth_prompt_message(Some(&view)).to_string();
        }

        view.body = prompts::actionable_auth_prompt_body(&view);
        if !direct {
            view.authorization_url = None;
            view.pairing = None;
            view.body = match view.challenge_kind {
                Some(AuthPromptChallengeKind::Pairing) => {
                    prompts::PAIRING_PRIVATE_SETUP_MESSAGE.to_string()
                }
                Some(AuthPromptChallengeKind::OAuthUrl) => {
                    prompts::OAUTH_PRIVATE_SETUP_MESSAGE.to_string()
                }
                _ => prompts::AUTH_UNAVAILABLE_MESSAGE.to_string(),
            };
        }
        render_channel_auth_prompt(&view, direct)
    }
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

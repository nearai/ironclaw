use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_outbound::{
    OutboundError, ReplyTargetBindingClaim, ReplyTargetBindingValidator,
    ReplyTargetValidationRequest, RunNotificationEventKind, ValidatedReplyTargetBinding,
};
use ironclaw_product_adapters::{
    ApprovalPromptContextView, ExternalActorRef, ExternalConversationRef, GatePromptView,
    ProductAdapterError, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductRejection, ProductRejectionKind, ProductWorkflowRejectionKind,
};
use ironclaw_product_workflow::{
    BlockedAuthFlowCanceller, ConversationBindingService, ProductOutboundTargetResolver,
    ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
    VerifiedProductOutboundTargetMetadata, approval_prompt_context_view, is_approval_gate_ref,
};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_turns::{
    GateRef, GetRunStateRequest, ReplyTargetBindingRef, TurnActor, TurnCoordinator, TurnRunId,
    TurnRunState, TurnScope, TurnStatus,
};
use ironclaw_wasm_product_adapters::ImmediateAckWorkflowObserver;

use crate::observer::{FinalReplyDeliveryObserver, RunDeliveryGuard};
use crate::services::*;
use crate::triggered::{thread_scope_from_binding, turn_scope_from_thread_scope};

pub(crate) fn blocked_actionable_marker(state: &TurnRunState) -> Option<BlockedActionableMarker> {
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

pub(crate) fn channel_run_notification_projection_id(
    channel_protocol: &dyn ChannelDeliveryProtocol,
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
    format!(
        "{}-run-notification:{suffix}:{run_id}",
        channel_protocol.run_notification_projection_prefix()
    )
}

/// Adapts a resolved auth-prompt view for Slack delivery. OAuth setup links are
/// only safe to post in a private DM, so the `authorization_url` is stripped for
/// any non-DM (channel) target.
pub(crate) fn channel_auth_prompt_view(
    envelope: &ProductInboundEnvelope,
    mut view: ironclaw_product_adapters::AuthPromptView,
) -> ironclaw_product_adapters::AuthPromptView {
    if !auth_setup_link_is_private(envelope) {
        view.authorization_url = None;
    }
    view
}

pub(crate) fn auth_setup_link_is_private(envelope: &ProductInboundEnvelope) -> bool {
    matches!(
        envelope.payload(),
        ProductInboundPayload::UserMessage(payload)
            if payload.trigger == ironclaw_product_adapters::ProductTriggerReason::DirectChat
    )
}

pub(crate) fn channel_approval_gate_prompt_view(
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
/// Shared by the live observer path ([`FinalReplyDeliveryObserver::cancel_channel_auth_blocked_run`])
/// and the triggered delivery path ([`triggered_notification_for_state`]) so the
/// cancellation contract cannot drift between them.
pub(crate) async fn cancel_auth_blocked_run(
    coordinator: &dyn TurnCoordinator,
    auth_flow_canceller: Option<&dyn BlockedAuthFlowCanceller>,
    scope: &TurnScope,
    actor: TurnActor,
    run_id: TurnRunId,
    gate_ref: Option<&str>,
) -> Result<(), FinalReplyDeliveryError> {
    // Resolve the flow-cancel target BEFORE `cancel_run` consumes `actor`. Owner
    // Resolution mirrors `enrich_auth_prompt_view`: an explicit turn owner
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
        .map_err(|err| FinalReplyDeliveryError::StatusMessage {
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
            target = "ironclaw::reborn::channel_delivery",
            %run_id,
            %error,
            "failed to cancel stale auth flow on Slack auth auto-deny (best-effort)"
        );
    }
    Ok(())
}

pub(crate) fn jittered_poll_interval(base: Duration, run_id: &TurnRunId) -> Duration {
    if base.is_zero() {
        return base;
    }
    let mut hasher = DefaultHasher::new();
    run_id.to_string().hash(&mut hasher);
    let bucket = hasher.finish() as u32 % RUN_POLL_JITTER_BUCKETS;
    (base + base / RUN_POLL_JITTER_BUCKETS * bucket).min(MAX_RUN_POLL_INTERVAL)
}

#[async_trait]
impl ImmediateAckWorkflowObserver for FinalReplyDeliveryObserver {
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
                    queue.push_back(throttle_key.clone());
                    false
                }
            };
            if already_seen {
                tracing::debug!(
                    target = "ironclaw::reborn::channel_delivery",
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
                        target = "ironclaw::reborn::channel_delivery",
                        error = %error,
                        "busy-thread hint falling back to generic copy because the conversation binding was not resolved"
                    );
                    CHANNEL_BUSY_GENERIC_MESSAGE.to_string()
                }
            };
            if let Err(post_err) = self
                .services
                .channel_protocol
                .post_status_message(
                    self.services.egress.as_ref(),
                    envelope.external_conversation_ref(),
                    &hint,
                )
                .await
            {
                let mut guard = self.hint_seen.lock().unwrap_or_else(|e| e.into_inner());
                let (queue, set) = &mut *guard;
                set.remove(&throttle_key);
                queue.retain(|key| key != &throttle_key);
                tracing::debug!(
                    target = "ironclaw::reborn::channel_delivery",
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
                    target = "ironclaw::reborn::channel_delivery",
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
            tracing::debug!(
                target = "ironclaw::reborn::channel_delivery",
                "channel final reply delivery skipped because delivery semaphore was closed"
            );
            return;
        };
        let delivery_result = self.deliver_final_reply(envelope.clone(), ack).await;
        // `_delivery_guard` is dropped here automatically, removing the run_id from
        // `active_delivery_run_ids` even if `deliver_final_reply` returned an error.
        // Explicit drop makes the cleanup point visible at the call site.
        drop(_delivery_guard);
        if let Err(error) = delivery_result {
            tracing::debug!(
                target = "ironclaw::reborn::channel_delivery",
                error = %error,
                "channel final reply delivery failed after immediate ACK"
            );
            // A3: Best-effort feedback post so the user is not left in silence.
            // Skip if a blocked-state notification was already delivered — the
            // user already saw an approval/auth prompt and is not in silence.
            let feedback = match &error {
                FinalReplyDeliveryError::RunWaitTimedOut { .. } => {
                    Some(CHANNEL_DELIVERY_TIMEOUT_MESSAGE)
                }
                FinalReplyDeliveryError::RunWaitTimedOutAfterNotification { .. } => None,
                _ => Some(CHANNEL_DELIVERY_ERROR_MESSAGE),
            };
            if let Some(feedback) = feedback
                && let Err(post_err) = self
                    .services
                    .channel_protocol
                    .post_status_message(
                        self.services.egress.as_ref(),
                        envelope.external_conversation_ref(),
                        feedback,
                    )
                    .await
            {
                tracing::debug!(
                    target = "ironclaw::reborn::channel_delivery",
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

/// Fail closed when a delivery that must reach a personal DM (e.g. carries an
/// OAuth authorization_url) resolves to a non-DM target.
pub(crate) fn enforce_direct_message_if_required(
    protocol: &dyn ChannelDeliveryProtocol,
    target: &ReplyTargetBindingRef,
    require_direct_message: bool,
) -> Result<(), ProductWorkflowError> {
    if require_direct_message && !protocol.reply_target_is_personal_dm(target) {
        return Err(ProductWorkflowError::OutboundTargetNotDirectMessage);
    }
    Ok(())
}

pub(crate) struct ObservedChannelReplyTargetAuthority {
    pub(crate) channel_protocol: Arc<dyn ChannelDeliveryProtocol>,
    pub(crate) scope: TurnScope,
    pub(crate) actor: TurnActor,
    pub(crate) expected_target: ReplyTargetBindingRef,
    pub(crate) external_conversation_ref: ExternalConversationRef,
    pub(crate) external_actor_ref: Option<ExternalActorRef>,
}

#[async_trait]
impl ReplyTargetBindingValidator for ObservedChannelReplyTargetAuthority {
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
impl ProductOutboundTargetResolver for ObservedChannelReplyTargetAuthority {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ValidatedReplyTargetBinding,
        require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        if target.target() != &self.expected_target {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        // Defense in depth: honor the DM requirement even on the live-path resolver.
        enforce_direct_message_if_required(
            self.channel_protocol.as_ref(),
            target.target(),
            require_direct_message,
        )?;
        Ok(VerifiedProductOutboundTargetMetadata {
            external_conversation_ref: self.external_conversation_ref.clone(),
            external_actor_ref: self.external_actor_ref.clone(),
        })
    }
}

pub(crate) struct AllowNoProjectionAccess;

#[async_trait]
impl ironclaw_outbound::ThreadProjectionAccessPolicy for AllowNoProjectionAccess {
    async fn authorize_projection_access(
        &self,
        _request: ironclaw_outbound::ThreadProjectionAccessRequest,
    ) -> Result<ironclaw_outbound::ThreadProjectionAccessClaim, OutboundError> {
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

pub(crate) fn should_deliver_after_ack(
    envelope: &ProductInboundEnvelope,
    ack: &ProductInboundAck,
) -> bool {
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
pub(crate) fn rejection_hint_for_resolution(
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
pub(crate) fn busy_hint_user_message_run_id(
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
pub(crate) async fn busy_hint_from_run_state(
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
                target = "ironclaw::reborn::channel_delivery",
                error = %err,
                "busy-thread hint scope derivation failed; using generic copy"
            );
            return CHANNEL_BUSY_GENERIC_MESSAGE.to_string();
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
                    let what = match approval_prompt_context_view(
                        approval_requests,
                        gate_ref,
                        &binding.actor_user_id,
                        &scope,
                    )
                    .await
                    {
                        Ok(context) => context.map(|ctx| ctx.tool_name),
                        Err(error) => {
                            tracing::debug!(
                                target = "ironclaw::reborn::channel_delivery",
                                %error,
                                "busy-thread approval context lookup failed; withholding actionable approval copy"
                            );
                            return "Ironclaw is waiting on an approval, but its details are temporarily unavailable here. Check the Ironclaw web app before responding."
                                .to_string();
                        }
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
                None => CHANNEL_BUSY_APPROVAL_MESSAGE.to_string(),
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
                None => CHANNEL_BUSY_GENERIC_MESSAGE.to_string(),
            },
            _ => CHANNEL_BUSY_GENERIC_MESSAGE.to_string(),
        },
        Err(err) => {
            tracing::debug!(
                target = "ironclaw::reborn::channel_delivery",
                error = %err,
                "busy-thread hint run-state lookup failed; using generic copy"
            );
            CHANNEL_BUSY_GENERIC_MESSAGE.to_string()
        }
    }
}

pub(crate) fn rejection_ack_for_workflow_error(
    error: &ProductAdapterError,
) -> Option<ProductInboundAck> {
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

pub(crate) fn product_rejection_kind_for_workflow_rejection(
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

pub(crate) fn is_accepted_auth_denial(
    envelope: &ProductInboundEnvelope,
    ack: &ProductInboundAck,
) -> bool {
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

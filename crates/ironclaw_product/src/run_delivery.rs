//! Generic run-delivery orchestration for channel extensions (§5.4).
//!
//! After the workflow accepts an inbound channel message (immediate-ACK
//! webhooks), durable lifecycle events deliver its user-visible outputs — the
//! final reply, approval/auth prompts, working indicators, busy hints, and
//! failure notices — back to the channel. Those semantics are identical for
//! every channel, so they live here once and speak only in
//! [`DeliveryIntent`]s through the [`DeliveryCoordinator`]. Vendor mechanics
//! (rendering, splitting, API selection) stay behind each extension's
//! `ChannelAdapter::deliver`.
//!
//! Three components:
//! - [`RunDeliveryObserver`] — admission-time feedback without a run event.
//! - [`RunDeliveryEventRouter`] — live source-route and triggered-run delivery
//!   from committed lifecycle facts.
//! - [`TriggeredRunDeliveryDriver`] — registers proactive routing context.
//!
//! Vendor-specific residue enters ONLY through the small ports below
//! (approval/auth prompt enrichment, preference-target decoding); their
//! implementations live with the vendor integration, not here.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use crate::{
    ApprovalPromptContextView, AuthPromptView, ExternalConversationRef, ExternalEventId,
    OutboundPart, ProductAdapterError,
};
use async_trait::async_trait;
use ironclaw_host_api::UserId;
use ironclaw_outbound::{
    CommunicationPreferenceRepository, DeliveredGateRouteStore, OutboundError, OutboundStateStore,
};
use ironclaw_turns::{GateRef, TurnCoordinator, TurnRunId, TurnScope};

use crate::auth_prompt::{BlockedAuthFlowCanceller, BlockedAuthPromptRequest};

use crate::delivery_coordinator::{
    CoordinatedDeliveryError, CoordinatedDeliveryOutcome, DeliveryCoordinator, DeliveryIntent,
    NoticeDeliveryRequest,
};
use crate::{ConversationBindingService, ProductWorkflowError, ResolvedBinding};

mod gate_routes;
mod lifecycle_events;
mod observer;
pub(crate) mod prompts;
mod trigger_router;
mod triggered;

pub use lifecycle_events::{RunDeliveryEventHandler, RunDeliveryEventRouter};
pub use observer::RunDeliveryObserver;
pub use trigger_router::{TriggeredRunDeliveryChannel, TriggeredRunDeliveryRouter};
pub use triggered::{
    CurrentDeliveryTarget, CurrentDeliveryTargetResolver, TriggeredRunDeliveryDriver,
    TriggeredRunDeliveryRequest, TriggeredRunExternalDeliveryTarget,
};

/// Maximum number of (conversation, external_event_id) pairs remembered for
/// hint dedup. FIFO eviction beyond this cap keeps memory O(1); a
/// false-negative after eviction just means one extra hint, which is
/// harmless.
const HINT_SEEN_CAP: usize = 256;

/// Throttle key for the busy-thread hint: one hint per (conversation
/// fingerprint, external event id). Transport retries of the same vendor
/// event share the event id, so they deduplicate; each new human message has
/// a distinct event id and gets a fresh hint.
pub(crate) type HintSeenKey = (String, ExternalEventId);
pub(crate) type HintSeenSet = Mutex<(VecDeque<HintSeenKey>, HashSet<HintSeenKey>)>;

/// Approval-gate context enrichment: resolves WHAT is being approved
/// (tool/action/reason) for a gate ref — the same source the WebUI gate
/// projection reads. Implemented by the composition over its approval
/// request store; `None` results degrade prompts to generic wording.
#[async_trait]
pub trait ApprovalPromptContextSource: Send + Sync {
    async fn approval_prompt_context(
        &self,
        gate_ref: &GateRef,
        owner_user_id: &UserId,
        scope: &TurnScope,
    ) -> Option<ApprovalPromptContextView>;
}

/// Auth-prompt enrichment: resolves the challenge (OAuth authorization URL
/// vs manual credential entry) for a run blocked on auth. Implemented by the
/// composition over the auth engine.
#[async_trait]
pub trait BlockedAuthPromptSource: Send + Sync {
    async fn auth_prompt_for_blocked_run(
        &self,
        request: BlockedAuthPromptRequest<'_>,
    ) -> Result<AuthPromptView, ProductAdapterError>;
}

/// Everything the generic run-delivery components need. All handles are
/// `Arc`s; cloning shares them.
#[derive(Clone)]
pub struct RunDeliveryServices {
    pub binding_service: Arc<dyn ConversationBindingService>,
    pub thread_service: Arc<dyn ironclaw_threads::SessionThreadService>,
    pub turn_coordinator: Arc<dyn TurnCoordinator>,
    pub outbound_store: Arc<dyn OutboundStateStore>,
    pub route_store: Arc<dyn DeliveredGateRouteStore>,
    pub communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    /// The coordinator every send goes through (OUT-1: none bypasses).
    pub coordinator: Arc<DeliveryCoordinator>,
    /// The channel extension whose surface these components serve (the
    /// coordinator resolves the adapter + egress from the active snapshot by
    /// this id). Configured, not derived from envelopes: the envelope's
    /// adapter id is a protocol identity, not the extension id.
    pub extension_id: String,
    /// Attribution scope for notices whose source has no resolvable binding
    /// (e.g. the connect nudge greeting an unbound user). Attempts must land
    /// under a defined scope; this is the host's channel-notice ledger.
    pub fallback_notice_scope: TurnScope,
    pub approval_context: Option<Arc<dyn ApprovalPromptContextSource>>,
    pub blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
    pub auth_flow_cancel: Option<Arc<dyn BlockedAuthFlowCanceller>>,
}

/// One message a channel accepted, in generic vocabulary: the conversation
/// it landed in plus the vendor's reference for it. Replaces vendor-side
/// response sniffing — the refs come from the coordinator's
/// [`CoordinatedDeliveryOutcome::Delivered`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveredChannelMessage {
    pub reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef,
    pub conversation: ExternalConversationRef,
    pub vendor_message_ref: String,
}

pub(crate) fn delivered_messages_from_outcome(
    outcome: &CoordinatedDeliveryOutcome,
) -> Vec<DeliveredChannelMessage> {
    match outcome {
        CoordinatedDeliveryOutcome::Delivered {
            attempt,
            conversation,
            vendor_message_refs,
        } => vendor_message_refs
            .iter()
            .map(|reference| DeliveredChannelMessage {
                reply_target_binding_ref: attempt.candidate.target.clone(),
                conversation: conversation.clone(),
                vendor_message_ref: reference.clone(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Failures raised while reacting to a run lifecycle event and delivering its outputs.
#[derive(Debug, thiserror::Error)]
pub enum RunDeliveryError {
    #[error("workflow binding failed: {0}")]
    Workflow(#[from] ProductWorkflowError),
    #[error("turn coordinator failed: {0}")]
    Turn(#[from] ironclaw_turns::TurnError),
    #[error("thread service failed: {0}")]
    Thread(#[from] ironclaw_threads::SessionThreadError),
    #[error("adapter failed: {0}")]
    Adapter(#[from] ProductAdapterError),
    #[error("outbound policy failed: {0}")]
    Outbound(#[from] OutboundError),
    #[error("coordinated delivery failed: {0}")]
    Delivery(#[from] CoordinatedDeliveryError),
    #[error("delivery reported terminal failure: {failure_kind:?}")]
    DeliveryFailed {
        failure_kind: ironclaw_outbound::DeliveryFailureKind,
    },
    #[error("cleanup retraction was not delivered")]
    CleanupNotDelivered,
    #[error("invalid projection ref: {reason}")]
    InvalidProjectionRef { reason: String },
}

/// Cancel a run parked on an interactive-auth gate with a `Policy` reason —
/// the same `cancel_run` the auth-deny resolution uses. Idempotent per run
/// (`channel-auth-block:{run_id}`) so replayed lifecycle events are safe.
/// Shared by the live and triggered paths so the cancellation
/// contract cannot drift between them. After a successful run cancel the
/// durable auth-flow record is cancelled alongside it (best-effort).
pub(crate) async fn cancel_auth_blocked_run(
    coordinator: &dyn TurnCoordinator,
    auth_flow_cancel: Option<&dyn BlockedAuthFlowCanceller>,
    scope: &TurnScope,
    actor: ironclaw_turns::TurnActor,
    run_id: TurnRunId,
    gate_ref: Option<&str>,
) -> Result<(), RunDeliveryError> {
    // Resolve the flow-cancel target BEFORE `cancel_run` consumes `actor`.
    // Owner resolution: an explicit turn owner (shared/team subject) wins,
    // else the acting user. Without a gate ref there is no flow to resolve.
    let flow_cancel_target = match (auth_flow_cancel, gate_ref) {
        (Some(canceller), Some(gate_ref)) => {
            let owner_user_id = scope
                .explicit_owner_user_id()
                .unwrap_or(&actor.user_id)
                .clone();
            Some((canceller, owner_user_id, gate_ref))
        }
        _ => None,
    };

    let idempotency_key = ironclaw_turns::IdempotencyKey::new(format!(
        "channel-auth-block:{run_id}"
    ))
    .map_err(|err| RunDeliveryError::InvalidProjectionRef {
        reason: format!("invalid idempotency key for auth block: {err}"),
    })?;
    // Cancel the run FIRST — it is the user-visible terminal action. If it
    // fails we return here and leave the durable auth flow (and the still
    // usable auth prompt) intact: marking the flow terminal while the run is
    // still `BlockedAuth` would be inverse state drift, and the OAuth
    // backstop relies on a failed cancel leaving the prompt usable.
    coordinator
        .cancel_run(ironclaw_turns::CancelRunRequest {
            scope: scope.clone(),
            actor,
            run_id,
            reason: ironclaw_turns::SanitizedCancelReason::Policy,
            idempotency_key,
        })
        .await?;

    if let Some((canceller, owner_user_id, gate_ref)) = flow_cancel_target
        && let Err(error) = canceller
            .cancel_blocked_auth_flow(scope, &owner_user_id, run_id, gate_ref)
            .await
    {
        tracing::debug!(
            target = "ironclaw::reborn::run_delivery",
            %run_id,
            %error,
            "failed to cancel stale auth flow on channel auth auto-deny (best-effort)"
        );
    }
    Ok(())
}

pub(crate) fn thread_scope_from_binding(
    binding: &ResolvedBinding,
) -> Result<ironclaw_threads::ThreadScope, ProductWorkflowError> {
    let Some(agent_id) = binding.agent_id.clone() else {
        return Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "resolved binding missing agent_id required for thread scope".to_string(),
        });
    };
    Ok(ironclaw_threads::ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id,
        project_id: binding.project_id.clone(),
        owner_user_id: binding.subject_user_id.clone(),
        mission_id: None,
    })
}

pub(crate) fn turn_scope_from_thread_scope(
    binding: &ResolvedBinding,
    thread_scope: &ironclaw_threads::ThreadScope,
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

impl RunDeliveryServices {
    /// Best-effort source-routed system notice on `conversation`. Failures
    /// are logged, never propagated — a notice must not break the flow that
    /// raised it.
    pub(crate) async fn post_notice(
        &self,
        intent: DeliveryIntent,
        scope: TurnScope,
        run_id: Option<TurnRunId>,
        conversation: &ExternalConversationRef,
        text: &str,
        notice_ref: String,
    ) -> Option<DeliveredChannelMessage> {
        match self
            .coordinator
            .deliver_notice(NoticeDeliveryRequest {
                intent,
                scope,
                turn_run_id: run_id,
                conversation: conversation.clone(),
                thread_anchor: None,
                parts: vec![OutboundPart::Text(text.to_string())],
                extension_id: &self.extension_id,
                notice_ref,
            })
            .await
        {
            Ok(outcome) => delivered_messages_from_outcome(&outcome).into_iter().next(),
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::reborn::run_delivery",
                    %error,
                    "channel notice delivery failed (best-effort)"
                );
                None
            }
        }
    }

    /// Attempt cleanup of an earlier delivery (`Cleanup` intent with a
    /// `Retract` part), returning `true` only when the provider reports the
    /// retraction delivered.
    pub(crate) async fn retract_message(
        &self,
        scope: TurnScope,
        run_id: Option<TurnRunId>,
        message: DeliveredChannelMessage,
    ) -> Result<bool, CoordinatedDeliveryError> {
        Ok(matches!(
            self.retract_message_outcome(scope, run_id, message, 0)
                .await?,
            CoordinatedDeliveryOutcome::Delivered { .. }
        ))
    }

    pub(crate) async fn retract_message_outcome(
        &self,
        scope: TurnScope,
        run_id: Option<TurnRunId>,
        message: DeliveredChannelMessage,
        attempt_ordinal: u32,
    ) -> Result<CoordinatedDeliveryOutcome, CoordinatedDeliveryError> {
        let notice_ref = format!(
            "retract-{}-attempt-{attempt_ordinal}",
            message
                .vendor_message_ref
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
                .collect::<String>()
        );
        self.coordinator
            .deliver_notice(NoticeDeliveryRequest {
                intent: DeliveryIntent::Cleanup,
                scope,
                turn_run_id: run_id,
                conversation: message.conversation.clone(),
                thread_anchor: None,
                parts: vec![OutboundPart::Retract {
                    vendor_message_ref: message.vendor_message_ref,
                }],
                extension_id: &self.extension_id,
                notice_ref,
            })
            .await
    }
}

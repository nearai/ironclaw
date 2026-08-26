//! Generic run-delivery orchestration for channel extensions (§5.4).
//!
//! After the workflow accepts an inbound channel message (immediate-ACK
//! webhooks), something must watch the submitted run and deliver its
//! user-visible outputs — the final reply, approval/auth prompts, working
//! indicators, busy hints, failure notices — back to the channel. That
//! watching-and-emitting logic is pure delivery *semantics*: it is identical
//! for every channel, so it lives here, once, and speaks only in
//! [`DeliveryIntent`]s through the [`DeliveryCoordinator`]. Vendor mechanics
//! (rendering, splitting, API selection) stay behind each extension's
//! [`ChannelDelivery::deliver`](ironclaw_extension_contracts::channel_adapter::ChannelDelivery::deliver).
//!
//! Two components:
//! - [`RunDeliveryObserver`] — the live source-route path: watches the run an
//!   inbound message submitted and replies on the originating conversation.
//! - [`TriggeredRunDeliveryDriver`] — the proactive path: watches a
//!   trigger-submitted run and delivers to the creator's preference target.
//!
//! Vendor-specific residue enters ONLY through the small ports below
//! (approval/auth prompt enrichment, preference-target decoding); their
//! implementations live with the vendor integration, not here.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironclaw_extension_contracts::channel_adapter::{
    OutboundPart, OutboundVisibility, ReactionAction, RunReaction,
};
use ironclaw_extension_contracts::external::{ExternalConversationRef, ExternalEventId};
use ironclaw_host_api::product_adapter::ProductAdapterError;
use ironclaw_host_api::turn::{TurnRunId, TurnScope, TurnStatus};
use ironclaw_notifications::{
    LifecycleRef, NotificationAction, NotificationId, NotificationInboxError,
    NotificationInboxStorePort, NotificationInitialState, NotificationKind,
    NotificationMutationRequest, NotificationRecipient, NotificationSeverity, NotificationSource,
    PublishNotificationRequest,
};
use ironclaw_outbound::{
    CommunicationPreferenceRepository, DeliveredGateRouteStore, OutboundDeliveryTargetProvider,
    OutboundError, OutboundStateStorePort,
};
use ironclaw_turns::{GetRunStateRequest, TurnCoordinator, TurnRunState};

use ironclaw_auth::product_prompt::BlockedAuthFlowCanceller;
use ironclaw_product_contracts::prompt_source::{
    ApprovalPromptContextSource, BlockedAuthPromptSource,
};

use crate::ProductSurfaceFailure;
use crate::ProjectFilesystemReader;
use crate::delivery_coordinator::{
    CoordinatedDeliveryError, CoordinatedDeliveryOutcome, DeliveryCoordinator, DeliveryIntent,
    NoticeDeliveryRequest,
};
use ironclaw_product_contracts::binding::ProductBindingResolver;
use ironclaw_product_contracts::binding::ResolvedBinding;

mod gate_routes;
mod inbox_gate_observer;
pub mod notifications;
mod observer;
pub(crate) mod prompts;
mod triggered;

pub use observer::RunDeliveryObserver;
pub use triggered::TriggeredRunDeliveryDriver;

const MAX_RUN_POLL_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_RUN_DELIVERY_MAX_WAIT: Duration = Duration::from_secs(30 * 60);
const RUN_POLL_JITTER_BUCKETS: u32 = 5;

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

/// Delivery pacing and admission bounds for run watchers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunDeliverySettings {
    pub poll_interval: Duration,
    pub max_wait: Duration,
    pub max_concurrent_deliveries: NonZeroUsize,
    /// Bounds the total number of spawned delivery tasks (active + waiting
    /// for a delivery permit). When this limit is reached, new trigger fires
    /// are recorded as `Skipped` rather than spawning an unbounded waiting
    /// task.
    pub max_pending_deliveries: NonZeroUsize,
    /// How long a run may run before the working indicator is refreshed to a
    /// "still working" nudge, so the user knows it hasn't stalled.
    pub first_nudge_after: Duration,
    /// Gap before the SECOND nudge; each subsequent gap doubles (e.g. 30s, then
    /// +1m, +2m, +4m …) so a very long run backs off instead of spamming.
    pub renudge_interval: Duration,
}

impl Default for RunDeliverySettings {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(250),
            max_wait: DEFAULT_RUN_DELIVERY_MAX_WAIT,
            max_concurrent_deliveries: NonZeroUsize::new(64).expect("non-zero literal"), // safety: static default literal is non-zero.
            max_pending_deliveries: NonZeroUsize::new(256).expect("non-zero literal"), // safety: static default literal is non-zero.
            first_nudge_after: Duration::from_secs(30),
            renudge_interval: Duration::from_secs(60),
        }
    }
}

/// Compatibility constructor for the triggered path. Live and proactive runs
/// share the same long-running watcher budget because either may legitimately
/// exceed the former two-minute cutoff before parking or completing.
pub fn triggered_run_delivery_settings() -> RunDeliverySettings {
    RunDeliverySettings::default()
}

/// Everything the generic run-delivery components need. All handles are
/// `Arc`s; cloning shares them.
#[derive(Clone)]
pub struct RunDeliveryServices {
    pub binding_service: Arc<dyn ProductBindingResolver>,
    pub thread_service: Arc<dyn ironclaw_threads::SessionThreadService>,
    pub turn_coordinator: Arc<dyn TurnCoordinator>,
    pub outbound_store: Arc<dyn OutboundStateStorePort>,
    pub route_store: Arc<dyn DeliveredGateRouteStore>,
    pub communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    /// Durable product-owned Inbox destination. `None` is allowed only for
    /// ingress-only/test graphs; production composition always supplies it.
    pub notification_inbox: Option<Arc<dyn NotificationInboxStorePort>>,
    /// Canonical project-scoped reader used to materialize assistant-authored
    /// `/workspace/...` references only after outbound policy approves the
    /// delivery.
    pub project_filesystem: Arc<dyn ProjectFilesystemReader>,
    /// The owner-scoped outbound target catalog. The background-run notifier
    /// resolves the creator's stored notification-channel ids through it at
    /// fire time; a target that vanished since it was chosen simply drops out.
    pub delivery_targets: Arc<dyn OutboundDeliveryTargetProvider>,
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
    pub conversation: ExternalConversationRef,
    pub vendor_message_ref: String,
}

pub(crate) fn delivered_messages_from_outcome(
    outcome: &CoordinatedDeliveryOutcome,
) -> Vec<DeliveredChannelMessage> {
    match outcome {
        // `DeliveredUnconfirmed` is the ONE non-`Delivered` outcome that
        // actually sent something: the provider accepted the message and
        // returned real refs, only the durable terminal write failed. The
        // messages exist in the channel, so everything keyed off them — gate
        // reply routes, and retraction of a live auth prompt — must still be
        // bookkept, or a delivered OAuth link can never be retracted and a
        // threaded `approve` can never route.
        CoordinatedDeliveryOutcome::Delivered {
            conversation,
            vendor_message_refs,
            ..
        }
        | CoordinatedDeliveryOutcome::DeliveredUnconfirmed {
            conversation,
            vendor_message_refs,
            ..
        } => vendor_message_refs
            .iter()
            .map(|reference| DeliveredChannelMessage {
                conversation: conversation.clone(),
                vendor_message_ref: reference.clone(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Failures raised while watching a run and delivering its outputs.
#[derive(Debug, thiserror::Error)]
pub enum RunDeliveryError {
    #[error("workflow binding failed: {0}")]
    Workflow(#[from] ProductSurfaceFailure),
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
    #[error("run {run_id} did not finish before the delivery timeout")]
    RunWaitTimedOut { run_id: TurnRunId },
    /// Timeout after at least one blocked-state notification (approval/auth
    /// prompt) was already delivered. The user is not in silence, so no
    /// additional feedback message is needed.
    #[error("run {run_id} did not reach a terminal state after delivering a blocked notification")]
    RunWaitTimedOutAfterNotification { run_id: TurnRunId },
    #[error("invalid projection ref: {reason}")]
    InvalidProjectionRef { reason: String },
}

/// The last blocked state a watcher already notified about; a run returning
/// to the same (status, gate) pair is not re-announced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockedActionableMarker {
    pub(crate) status: TurnStatus,
    pub(crate) gate_ref: Option<String>,
}

/// The inbox kind a blocked run maps to. Shared by the session watcher and the
/// triggered watcher so a gate cannot be published under one kind and resolved
/// under another.
pub(crate) fn blocked_status_notification_kind(status: TurnStatus) -> Option<NotificationKind> {
    match status {
        TurnStatus::BlockedApproval => Some(NotificationKind::ApprovalRequired),
        TurnStatus::BlockedAuth => Some(NotificationKind::AuthenticationRequired),
        _ => None,
    }
}

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

pub(crate) fn jittered_poll_interval(base: Duration, run_id: &TurnRunId) -> Duration {
    if base.is_zero() {
        return base;
    }
    let mut hasher = DefaultHasher::new();
    run_id.to_string().hash(&mut hasher);
    let bucket = hasher.finish() as u32 % RUN_POLL_JITTER_BUCKETS;
    (base + base / RUN_POLL_JITTER_BUCKETS * bucket).min(MAX_RUN_POLL_INTERVAL)
}

/// Poll a run until it reaches a terminal state or a blocked state the
/// caller has not yet announced. (The live observer carries its own copy of
/// this loop to raise the working indicator between polls; keep the two in
/// sync.)
pub(crate) async fn wait_for_actionable_state(
    turn_coordinator: &dyn TurnCoordinator,
    scope: &TurnScope,
    run_id: TurnRunId,
    settings: &RunDeliverySettings,
    delivered_blocked_marker: Option<&BlockedActionableMarker>,
) -> Result<TurnRunState, RunDeliveryError> {
    let start = tokio::time::Instant::now();
    let mut poll_interval = settings.poll_interval;
    loop {
        let state = turn_coordinator
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
        if start.elapsed() >= settings.max_wait {
            return Err(RunDeliveryError::RunWaitTimedOut { run_id });
        }
        tokio::time::sleep(jittered_poll_interval(poll_interval, &run_id)).await;
        poll_interval = poll_interval.saturating_mul(2).min(MAX_RUN_POLL_INTERVAL);
    }
}

/// Cancel a run parked on an interactive-auth gate with a `Policy` reason —
/// the same `cancel_run` the auth-deny resolution uses. Idempotent per run
/// (`channel-auth-block:{run_id}`) so repeated watcher passes are safe.
/// Shared by the live observer and the triggered path so the cancellation
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
    // The flow was created under the run's user, so cancel must target the same
    // user — matching the auth-flow create/resolve sides. Owner == actor since
    // the ephemeral-per-ping remodel. Without a gate ref there is no flow to
    // resolve.
    let flow_cancel_target = match (auth_flow_cancel, gate_ref) {
        (Some(canceller), Some(gate_ref)) => Some((canceller, actor.user_id.clone(), gate_ref)),
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
            target: "ironclaw::reborn::run_delivery",
            %run_id,
            %error,
            "failed to cancel stale auth flow on channel auth auto-deny (best-effort)"
        );
    }
    Ok(())
}

pub(crate) fn thread_scope_from_binding(
    binding: &ResolvedBinding,
) -> Result<ironclaw_threads::ThreadScope, ProductSurfaceFailure> {
    let Some(agent_id) = binding.agent_id.clone() else {
        return Err(ProductSurfaceFailure::BindingResolutionFailed {
            reason: "resolved binding missing agent_id required for thread scope".to_string(),
        });
    };
    Ok(ironclaw_threads::ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id,
        project_id: binding.project_id.clone(),
        // The thread belongs to the user who invoked it (the pinger). A channel
        // ping resolves onto its own ephemeral pinger-owned thread and a DM is
        // the user's own thread, so there is one identity per run: the actor.
        owner_user_id: Some(binding.actor_user_id.clone()),
        mission_id: None,
    })
}

pub(crate) fn turn_scope_from_thread_scope(
    binding: &ResolvedBinding,
    thread_scope: &ironclaw_threads::ThreadScope,
) -> Result<TurnScope, ProductSurfaceFailure> {
    let Some(agent_id) = binding.agent_id.clone() else {
        return Err(ProductSurfaceFailure::BindingResolutionFailed {
            reason: "resolved binding missing agent_id required for turn scope".to_string(),
        });
    };
    // The run's turn scope shares the thread scope's single user (the pinger):
    // there is no separate thread owner to diverge from the actor.
    Ok(TurnScope::new_with_owner(
        binding.tenant_id.clone(),
        Some(agent_id),
        binding.project_id.clone(),
        binding.thread_id.clone(),
        thread_scope.owner_user_id.clone(),
    ))
}

impl RunDeliveryServices {
    /// Best-effort publication of bounded, metadata-only run state to the
    /// authenticated WebUI Inbox. External channel delivery remains separate.
    pub(crate) async fn publish_inbox_notification(
        &self,
        user_id: &ironclaw_host_api::ids::UserId,
        scope: &TurnScope,
        run_id: TurnRunId,
        kind: NotificationKind,
        lifecycle_ref: Option<&str>,
    ) {
        let Some(inbox) = self.notification_inbox.as_ref() else {
            return;
        };
        let notification_id = match run_notification_inbox_id(run_id, kind, lifecycle_ref) {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(%error, %run_id, "invalid durable Inbox notification id");
                return;
            }
        };
        let lifecycle_ref = match lifecycle_ref.map(LifecycleRef::new).transpose() {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error, %run_id, "invalid durable Inbox lifecycle reference");
                return;
            }
        };
        let severity = match kind {
            NotificationKind::RunCompleted => NotificationSeverity::Success,
            NotificationKind::RunFailed | NotificationKind::DeliveryFailed => {
                NotificationSeverity::Error
            }
            NotificationKind::ApprovalRequired
            | NotificationKind::AuthenticationRequired
            | NotificationKind::RunBlocked => NotificationSeverity::Warning,
        };
        let initial_state = match kind {
            NotificationKind::RunCompleted
            | NotificationKind::RunFailed
            | NotificationKind::DeliveryFailed => NotificationInitialState::Resolved,
            NotificationKind::ApprovalRequired
            | NotificationKind::AuthenticationRequired
            | NotificationKind::RunBlocked => NotificationInitialState::Open,
        };
        if let Err(error) = inbox
            .publish(PublishNotificationRequest {
                id: notification_id,
                recipient: NotificationRecipient {
                    tenant_id: scope.tenant_id.clone(),
                    user_id: user_id.clone(),
                },
                kind,
                severity,
                source: NotificationSource {
                    thread_id: scope.thread_id.clone(),
                    turn_run_id: Some(run_id),
                    lifecycle_ref,
                },
                action: NotificationAction::OpenThread {
                    thread_id: scope.thread_id.clone(),
                },
                initial_state,
                occurred_at: chrono::Utc::now(),
            })
            .await
        {
            tracing::warn!(%error, %run_id, "failed to publish durable Inbox notification");
        }
    }

    /// Best-effort resolution of the stable notification id derived from a
    /// run and lifecycle reference. A missing record is intentionally benign.
    pub(crate) async fn resolve_inbox_notification(
        &self,
        user_id: &ironclaw_host_api::ids::UserId,
        scope: &TurnScope,
        run_id: TurnRunId,
        kind: NotificationKind,
        lifecycle_ref: Option<&str>,
    ) {
        let Some(inbox) = self.notification_inbox.as_ref() else {
            return;
        };
        let notification_id = match run_notification_inbox_id(run_id, kind, lifecycle_ref) {
            Ok(id) => id,
            Err(error) => {
                // An unbuildable id means this gate's record can never be
                // retired, so it is reported rather than passed over.
                tracing::warn!(%error, %run_id, "invalid durable Inbox notification id");
                return;
            }
        };
        let result = inbox
            .resolve(NotificationMutationRequest {
                recipient: NotificationRecipient {
                    tenant_id: scope.tenant_id.clone(),
                    user_id: user_id.clone(),
                },
                notification_id,
                occurred_at: chrono::Utc::now(),
            })
            .await;
        if let Err(error) = result
            && !matches!(error, NotificationInboxError::NotificationNotFound)
        {
            tracing::warn!(%error, %run_id, "failed to resolve durable Inbox notification");
        }
    }

    /// Best-effort source-routed system notice on `conversation`. Failures
    /// are logged, never propagated — a notice must not break the flow that
    /// raised it. Delivered publicly; see [`Self::post_notice_with_visibility`]
    /// for a notice that should reach only one external actor.
    pub(crate) async fn post_notice(
        &self,
        intent: DeliveryIntent,
        scope: TurnScope,
        run_id: Option<TurnRunId>,
        conversation: &ExternalConversationRef,
        text: &str,
        notice_ref: String,
    ) -> Option<DeliveredChannelMessage> {
        self.post_notice_with_visibility(
            intent,
            scope,
            run_id,
            conversation,
            text,
            notice_ref,
            OutboundVisibility::Public,
        )
        .await
    }

    /// [`Self::post_notice`] with an explicit visibility request.
    // arch-exempt: too_many_args, needs a notice-request bundle, which would duplicate NoticeDeliveryRequest one layer up, plan #7681
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn post_notice_with_visibility(
        &self,
        intent: DeliveryIntent,
        scope: TurnScope,
        run_id: Option<TurnRunId>,
        conversation: &ExternalConversationRef,
        text: &str,
        notice_ref: String,
        visibility: OutboundVisibility,
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
                visibility,
            })
            .await
        {
            Ok(outcome) => delivered_messages_from_outcome(&outcome).into_iter().next(),
            Err(error) => {
                tracing::debug!(
                    target: "ironclaw::reborn::run_delivery",
                    %error,
                    "channel notice delivery failed (best-effort)"
                );
                None
            }
        }
    }

    /// Best-effort cleanup of an earlier delivery (`Cleanup` intent with a
    /// `Retract` part) on this component's own channel extension.
    pub(crate) async fn retract_message(
        &self,
        scope: TurnScope,
        run_id: Option<TurnRunId>,
        message: DeliveredChannelMessage,
    ) {
        self.retract_message_on_extension(&self.extension_id, scope, run_id, message)
            .await;
    }

    /// [`Self::retract_message`] for a message delivered through a DIFFERENT
    /// extension than this component's configured one. The background-run
    /// notifier fans out across every notification channel, so the extension
    /// that carried a prompt is per-message, not per-component.
    pub(crate) async fn retract_message_on_extension(
        &self,
        extension_id: &str,
        scope: TurnScope,
        run_id: Option<TurnRunId>,
        message: DeliveredChannelMessage,
    ) {
        let notice_ref = format!(
            "retract-{}",
            message
                .vendor_message_ref
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
                .collect::<String>()
        );
        if let Err(error) = self
            .coordinator
            .deliver_notice(NoticeDeliveryRequest {
                intent: DeliveryIntent::Cleanup,
                scope,
                turn_run_id: run_id,
                conversation: message.conversation.clone(),
                thread_anchor: None,
                parts: vec![OutboundPart::Retract {
                    vendor_message_ref: message.vendor_message_ref,
                }],
                extension_id,
                notice_ref,
                visibility: OutboundVisibility::Public,
            })
            .await
        {
            tracing::warn!(
                target: "ironclaw::reborn::run_delivery",
                %error,
                "failed to retract channel prompt/status message"
            );
        }
    }

    /// Best-effort run-lifecycle reaction on the message that triggered the
    /// run (its `reply_target_message_id`) — 👀 while working, ⚠️ when it needs
    /// the user, ✅ done, ❌ failed. A conversation with no reply target (nothing
    /// to react to) is a no-op, and any delivery failure is swallowed: a
    /// reaction must never fail the run. `seq` is a per-run monotonic id so each
    /// transition is a distinct, idempotent delivery — a retried loop replays the
    /// same seq and dedupes, while a genuine re-transition gets a fresh one.
    pub(crate) async fn react_to_source(
        &self,
        scope: TurnScope,
        run_id: TurnRunId,
        conversation: &ExternalConversationRef,
        reaction: RunReaction,
        action: ReactionAction,
        seq: u64,
    ) {
        let Some(source_ref) = conversation.reply_target_message_id() else {
            return;
        };
        let action_key = match action {
            ReactionAction::Add => "add",
            ReactionAction::Remove => "remove",
        };
        let reaction_key = match reaction {
            RunReaction::Working => "working",
            RunReaction::Done => "done",
            RunReaction::NeedsInput => "needs-input",
            RunReaction::Failed => "failed",
        };
        if let Err(error) = self
            .coordinator
            .deliver_notice(NoticeDeliveryRequest {
                intent: DeliveryIntent::Reaction,
                scope,
                turn_run_id: Some(run_id),
                conversation: conversation.clone(),
                thread_anchor: None,
                parts: vec![OutboundPart::React {
                    vendor_message_ref: source_ref.to_string(),
                    reaction,
                    action,
                }],
                extension_id: &self.extension_id,
                notice_ref: format!("{run_id}:{seq}:{action_key}-{reaction_key}"),
                visibility: OutboundVisibility::Public,
            })
            .await
        {
            tracing::debug!(
                target: "ironclaw::reborn::run_delivery",
                %error,
                "channel reaction delivery failed (best-effort)"
            );
        }
    }
}

/// Lifecycle reference for the actionable block a delivery timeout leaves
/// behind. Publisher and resolver must derive the same stable id from it, so
/// it is named once rather than spelled at each site.
pub(crate) const TIMEOUT_LIFECYCLE_REF: &str = "timeout";

pub(crate) fn run_notification_inbox_id(
    run_id: TurnRunId,
    kind: NotificationKind,
    lifecycle_ref: Option<&str>,
) -> Result<NotificationId, NotificationInboxError> {
    let kind = match kind {
        NotificationKind::ApprovalRequired => "approval",
        NotificationKind::AuthenticationRequired => "authentication",
        NotificationKind::RunBlocked => "blocked",
        NotificationKind::RunFailed => "failed",
        NotificationKind::RunCompleted => "completed",
        NotificationKind::DeliveryFailed => "delivery-failed",
    };
    let lifecycle_key = lifecycle_ref
        .map(|value| uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, value.as_bytes()).to_string())
        .unwrap_or_else(|| "run".to_string());
    NotificationId::new(format!("run:{run_id}:{kind}:{lifecycle_key}"))
}

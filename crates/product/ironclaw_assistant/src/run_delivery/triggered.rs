//! The background-run notifier: watches a trigger-submitted run and tells the
//! creator's **notification channels** when it needs them — an approval gate,
//! an expired credential, or a failure — through the [`DeliveryCoordinator`].
//!
//! It deliberately does NOT push results. A background run's answer lives in
//! the fire's own run thread; putting it on a channel is the model's explicit
//! `builtin.outbound_deliver` call, never an automatic push (spec §8).

use ironclaw_product_contracts::prompt_source::BlockedAuthPromptRequest;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_extension_contracts::channel_adapter::OutboundPart;
use ironclaw_host_api::ids::{AgentId, TenantId, UserId};
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    CommunicationPreferenceKey, DeliveryDefaultScope, OutboundDeliveryTargetScope, OutboundError,
    OutboundPolicyService, PrepareCommunicationDeliveryRequest, ProjectionUpdateRef,
    ReplyTargetBindingClaim, ReplyTargetBindingValidator, ReplyTargetValidationRequest,
    RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin, SystemEventReasonCode,
    TriggeredFireFailureDeliveryRequest, TriggeredRunDelivery, TriggeredRunDeliveryOutcomeKind,
    TriggeredRunDeliveryRecord, TriggeredRunDeliveryRequest, TriggeredRunDeliveryStore,
    ValidatedReplyTargetBinding,
};
use ironclaw_threads::ThreadScope;
use ironclaw_turns::{
    ReplyTargetBindingRef, TurnActor, TurnRunId, TurnRunState, TurnScope, TurnStatus,
};
use tokio::sync::Semaphore;

use super::observer::AllowNoProjectionAccess;
use super::prompts;
use super::{
    BlockedActionableMarker, DeliveredChannelMessage, RunDeliveryError, RunDeliveryServices,
    RunDeliverySettings, blocked_actionable_marker, cancel_auth_blocked_run,
    delivered_messages_from_outcome, gate_routes::record_gate_route_if_needed,
    triggered_run_delivery_settings, wait_for_actionable_state,
};
use crate::delivery_coordinator::{
    CoordinatedDeliveryError, CoordinatedDeliveryOutcome, CoordinatedDeliveryRequest,
    DeliveryIntent,
};
use crate::{ProductOutboundTargetResolver, ProductSurfaceFailure};
use ironclaw_extension_contracts::preference_target::{
    ActivePreferenceTargetCodecs, PreferenceTargetCodec,
};

// The codec contract lives in `ironclaw_extension_contracts` — the vendor
// half is implemented by the channel packages, which must never depend on
// this crate. Consumers import it from there; this module deliberately keeps
// no second import path for it (PROPOSAL §11.2.4).

// `TriggeredRunDeliveryRequest` and the `TriggeredRunDelivery` port it crosses
// live in `ironclaw_outbound` (PROPOSAL §12.11 D-A): the generic post-submit
// hook that drives this driver sits *below* product, so the contract had to be
// declared where its vocabulary already lives. Every field is either outbound's
// own triggered-delivery vocabulary or `ironclaw_host_api` turn vocabulary, so
// the move cost no type weakening.

const TRACE_TARGET: &str = "ironclaw::reborn::run_delivery";

/// One effective notification target, resolved at fire time from the
/// creator's stored notification-channel set.
#[derive(Debug, Clone)]
struct NotificationTarget {
    /// The vendor binding ref the catalog entry resolves to.
    target: ReplyTargetBindingRef,
    /// The extension whose channel carries a delivery to this target — read
    /// from the catalog entry, never guessed.
    extension_id: String,
    /// Whether the catalog entry is a personal DM. Only a personal DM may
    /// carry an OAuth authorization URL.
    direct_message: bool,
}

/// Which notification targets one message is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetAudience {
    /// Every effective notification target.
    All,
    /// Personal DMs only (OAuth authorization URLs).
    DirectMessage,
    /// Everything that is NOT a personal DM (the redacted stand-in).
    NonDirectMessage,
}

impl TargetAudience {
    fn includes(self, target: &NotificationTarget) -> bool {
        match self {
            Self::All => true,
            Self::DirectMessage => target.direct_message,
            Self::NonDirectMessage => !target.direct_message,
        }
    }
}

/// One message to fan out to some subset of the notification targets.
struct TriggeredNotification {
    event_kind: RunNotificationEventKind,
    intent: DeliveryIntent,
    text: String,
    audience: TargetAudience,
    /// AuthPrompt payloads carrying an OAuth URL must only land in a personal
    /// DM; `audience` pre-filters, and this keeps the send-time resolver check
    /// as defense in depth against a stale snapshot.
    require_direct_message_target: bool,
}

/// Everything one actionable run state produces: the messages to fan out,
/// whether the run stays parked afterwards, and the gate the delivered
/// conversations should route replies for.
struct TriggeredNotificationPlan {
    notifications: Vec<TriggeredNotification>,
    /// `Some` for approval/auth prompts: the delivered conversations become
    /// reply routes for this gate.
    gate_ref_for_routing: Option<String>,
    /// True while the run remains parked awaiting the user after this plan —
    /// the watcher keeps waiting instead of finishing.
    keeps_run_parked: bool,
}

/// Stable run and routing inputs shared by every notification attempt for one
/// triggered run.
struct TriggeredNotificationContext<'a> {
    scope: &'a TurnScope,
    thread_scope: &'a ThreadScope,
    actor: &'a TurnActor,
    run_id: TurnRunId,
    authority: &'a TriggeredReplyTargetAuthority<'a>,
}

/// Typed failure classification for a single notification delivery attempt.
enum TriggeredNotificationFailure {
    /// The resolved target is inaccessible or rejected the delivery.
    Denied,
    /// Any other delivery or transport failure.
    Other(String),
}

impl std::fmt::Display for TriggeredNotificationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied => write!(f, "delivery target access denied"),
            Self::Other(reason) => write!(f, "{reason}"),
        }
    }
}

/// Drives background-run notification: one watcher per submitted run, bounded
/// by delivery and pending-admission semaphores, recording every outcome in
/// the [`TriggeredRunDeliveryStore`].
pub struct TriggeredRunDeliveryDriver {
    services: RunDeliveryServices,
    settings: RunDeliverySettings,
    delivery_permits: Arc<Semaphore>,
    /// Bounds the total number of spawned delivery tasks (active + waiting).
    /// Overflow is recorded as `Skipped` without spawning.
    pending_permits: Arc<Semaphore>,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    /// Fallback agent used to derive the project-filesystem authority scope
    /// when a run scope carries no agent id.
    fallback_agent_id: AgentId,
    /// LIVE view of every active channel extension's preference-target codec.
    /// A notification set legitimately spans extensions — the user picks
    /// catalog entries from every connected channel — so the notifier decodes
    /// each target through whichever codec owns its grammar; first to decode
    /// wins.
    ///
    /// Deliberately a source port, not a captured `Vec`: this notifier is
    /// built once and lives for the process, so a channel activated after the
    /// first fire must still decode its own targets. It is re-read on every
    /// fire.
    target_codecs: Arc<dyn ActivePreferenceTargetCodecs>,
}

impl TriggeredRunDeliveryDriver {
    pub fn new(
        services: RunDeliveryServices,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        target_codecs: Arc<dyn ActivePreferenceTargetCodecs>,
        fallback_agent_id: AgentId,
    ) -> Self {
        Self::with_settings(
            services,
            triggered_run_delivery_settings(),
            delivery_store,
            target_codecs,
            fallback_agent_id,
        )
    }

    pub fn with_settings(
        services: RunDeliveryServices,
        settings: RunDeliverySettings,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        target_codecs: Arc<dyn ActivePreferenceTargetCodecs>,
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
            fallback_agent_id,
            target_codecs,
        }
    }

    /// Acquire a permit from the pending-delivery semaphore for testing:
    /// lets tests hold the pending slot without spawning a real delivery
    /// task, so `Skipped` outcomes are assertable.
    #[cfg(any(test, feature = "test-support"))]
    pub fn try_acquire_pending_permit(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        Arc::clone(&self.pending_permits).try_acquire_owned().ok()
    }

    /// The preference repository this notifier resolves notification channels
    /// from. Production wiring must hand the SAME store the WebUI writes, so
    /// user-set channels are visible here; tests assert pointer equality.
    pub fn communication_preferences(
        &self,
    ) -> Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository> {
        Arc::clone(&self.services.communication_preferences)
    }

    /// Watch one submitted triggered run and notify the creator's channels.
    /// Spawns a bounded background task; the call returns once admission is
    /// decided.
    pub async fn on_trigger_submitted(&self, request: TriggeredRunDeliveryRequest) {
        // Fail closed for non-personal triggers.
        if request.project_scoped {
            tracing::debug!(
                run_id = %request.run_id,
                "background run notification denied: project-scoped trigger is not personal scope"
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
                target: TRACE_TARGET,
                run_id = %request.run_id,
                "background run notification skipped: pending delivery queue full"
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
        let target_codecs = Arc::clone(&self.target_codecs);
        let fallback_agent_id = self.fallback_agent_id.clone();

        tokio::spawn(async move {
            // Hold the pending permit for the full task lifetime so it
            // counts against the cap until delivery completes.
            let _pending_permit = pending_permit;

            let Ok(_permit) = permits.clone().acquire_owned().await else {
                tracing::warn!(
                    target: TRACE_TARGET,
                    run_id = %request.run_id,
                    "background run notification skipped: delivery semaphore closed"
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
            let outcome = notify_background_run(
                &services,
                &settings,
                request,
                &*delivery_store,
                target_codecs.as_ref(),
                &fallback_agent_id,
            )
            .await;
            tracing::debug!(
                target: TRACE_TARGET,
                %run_id,
                ?outcome,
                "background run notification completed"
            );
        });
    }

    /// Notify the creator when a fire permanently fails before a run can be
    /// submitted. The stable fire ref, rather than a fabricated run id, drives
    /// coordinator idempotency and durable delivery-attempt evidence.
    pub async fn on_trigger_failed_before_submit(
        &self,
        request: TriggeredFireFailureDeliveryRequest,
    ) {
        if request.project_scoped {
            tracing::debug!(
                failure_ref = %request.failure_ref,
                "background fire failure notification denied: project-scoped trigger is not personal scope"
            );
            return;
        }
        let Ok(pending_permit) = Arc::clone(&self.pending_permits).try_acquire_owned() else {
            tracing::warn!(
                target: TRACE_TARGET,
                failure_ref = %request.failure_ref,
                "background fire failure notification skipped: pending delivery queue full"
            );
            return;
        };

        let permits = Arc::clone(&self.delivery_permits);
        let services = self.services.clone();
        let target_codecs = Arc::clone(&self.target_codecs);
        let fallback_agent_id = self.fallback_agent_id.clone();
        tokio::spawn(async move {
            let _pending_permit = pending_permit;
            let Ok(_permit) = permits.acquire_owned().await else {
                tracing::warn!(
                    target: TRACE_TARGET,
                    failure_ref = %request.failure_ref,
                    "background fire failure notification skipped: delivery semaphore closed"
                );
                return;
            };
            notify_pre_submit_failure(
                &services,
                request,
                target_codecs.as_ref(),
                &fallback_agent_id,
            )
            .await;
        });
    }
}

/// The port the generic post-submit hook drives this driver through.
///
/// The inherent method stays: product's own tests and the crate's contract
/// suite call it directly, and an inherent method wins name resolution over a
/// trait one, so no call site changes meaning by this impl existing.
#[async_trait]
impl TriggeredRunDelivery for TriggeredRunDeliveryDriver {
    async fn on_trigger_submitted(&self, request: TriggeredRunDeliveryRequest) {
        TriggeredRunDeliveryDriver::on_trigger_submitted(self, request).await;
    }

    async fn on_trigger_failed_before_submit(&self, request: TriggeredFireFailureDeliveryRequest) {
        TriggeredRunDeliveryDriver::on_trigger_failed_before_submit(self, request).await;
    }
}

async fn notify_pre_submit_failure(
    services: &RunDeliveryServices,
    request: TriggeredFireFailureDeliveryRequest,
    target_codecs: &dyn ActivePreferenceTargetCodecs,
    fallback_agent_id: &AgentId,
) {
    let TriggeredFireFailureDeliveryRequest {
        scope,
        creator_user_id,
        project_scoped: _,
        prompt,
        failure_ref,
    } = request;
    let actor = TurnActor::new(creator_user_id.clone());
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
    let codecs = target_codecs.active_preference_target_codecs();
    let targets = resolve_notification_targets(
        services,
        &codecs,
        &scope.tenant_id,
        &creator_user_id,
        failure_ref.as_str(),
    )
    .await;
    if targets.is_empty() {
        return;
    }

    let authority = TriggeredReplyTargetAuthority {
        scope: scope.clone(),
        actor: actor.clone(),
        codecs: &codecs,
    };
    let text = format!(
        "{}{}",
        prompts::BACKGROUND_RUN_FAILED_MESSAGE,
        prompts::triggered_update_footer(&prompts::triggered_label_from_prompt(&prompt))
    );
    let delivery_context = PreSubmitFailureDeliveryContext {
        services,
        scope: &scope,
        thread_scope: &thread_scope,
        actor: &actor,
        authority: &authority,
        failure_ref: &failure_ref,
        text: &text,
    };
    for target in targets {
        if let Err(error) = deliver_pre_submit_failure_to_target(&delivery_context, &target).await {
            tracing::warn!(
                target: TRACE_TARGET,
                failure_ref = %failure_ref,
                extension_id = %target.extension_id,
                reason = %error,
                "background fire failure notification failed for one channel"
            );
        }
    }
}

struct PreSubmitFailureDeliveryContext<'a> {
    services: &'a RunDeliveryServices,
    scope: &'a TurnScope,
    thread_scope: &'a ThreadScope,
    actor: &'a TurnActor,
    authority: &'a TriggeredReplyTargetAuthority<'a>,
    failure_ref: &'a ProjectionUpdateRef,
    text: &'a str,
}

/// Inner watcher coroutine for a single background run.
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
async fn notify_background_run(
    services: &RunDeliveryServices,
    settings: &RunDeliverySettings,
    request: TriggeredRunDeliveryRequest,
    delivery_store: &dyn TriggeredRunDeliveryStore,
    target_codecs: &dyn ActivePreferenceTargetCodecs,
    fallback_agent_id: &AgentId,
) -> TriggeredRunDeliveryOutcomeKind {
    let TriggeredRunDeliveryRequest {
        run_id,
        scope,
        creator_user_id,
        project_scoped: _,
        prompt,
    } = request;
    let actor = TurnActor::new(creator_user_id.clone());
    // Canonical project-filesystem authority scope for workspace-reference
    // materialization. Notices never carry attachments, but the coordinator
    // contract requires the authority scope on every request.
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

    // Read the ACTIVE codec set for THIS fire. Never hoist this out of the
    // per-fire path: a channel activated since the last fire must be able to
    // decode its own notification targets.
    let target_codecs = target_codecs.active_preference_target_codecs();

    // Resolve the creator's notification channels ONCE, at fire time.
    let run_ref = run_id.to_string();
    let targets = resolve_notification_targets(
        services,
        &target_codecs,
        &scope.tenant_id,
        &creator_user_id,
        &run_ref,
    )
    .await;

    // With no notification channels configured the notifier has nothing to do
    // for ANY arm: it must not deliver, and it must not touch the run either.
    // The web app is the whole surface — the automations hold badge and the
    // in-app gate UI (spec §7) — including for a manual-token auth gate, which
    // the user CAN complete there. Short-circuiting here also keeps a fire from
    // holding a watcher open for the full `max_wait` with nothing to say.
    if targets.is_empty() {
        tracing::debug!(
            target: TRACE_TARGET,
            %run_id,
            "background run has no notification channels; notifications stay in the web app"
        );
        let outcome = TriggeredRunDeliveryOutcomeKind::NoDefaultConfigured;
        record_triggered_run_outcome(delivery_store, run_id, outcome).await;
        return outcome;
    }

    let mut delivered_blocked_marker: Option<BlockedActionableMarker> = None;
    // (extension that carried it, message) — the notification set spans
    // extensions, so retraction is per-message.
    let mut messages_to_delete_after_final: Vec<(String, DeliveredChannelMessage)> = Vec::new();

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
                    target: TRACE_TARGET,
                    %run_id,
                    "background run parked awaiting user after notifying; recording Delivered"
                );
                let outcome = TriggeredRunDeliveryOutcomeKind::Delivered;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
            Err(err) => {
                tracing::warn!(
                    target: TRACE_TARGET,
                    %run_id,
                    error = %err,
                    "background run wait failed"
                );
                let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
        };

        let trigger_label = prompts::triggered_label_from_prompt(&prompt);
        let plan = match notification_plan_for_state(
            services,
            &scope,
            &actor,
            &state,
            run_id,
            &trigger_label,
        )
        .await
        {
            Ok(Some(plan)) => plan,
            Ok(None) => {
                // Nothing to say about this state (a completed run, a
                // cancelled run, or a blocked state with no gate ref).
                retract_stale_prompts(
                    services,
                    &scope,
                    run_id,
                    &mut messages_to_delete_after_final,
                )
                .await;
                let outcome = TriggeredRunDeliveryOutcomeKind::Skipped;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
            Err(err) => {
                tracing::warn!(
                    target: TRACE_TARGET,
                    %run_id,
                    error = %err,
                    "background run notification build failed"
                );
                let outcome = TriggeredRunDeliveryOutcomeKind::Failed;
                record_triggered_run_outcome(delivery_store, run_id, outcome).await;
                return outcome;
            }
        };

        let next_blocked_marker = blocked_actionable_marker(&state);
        let authority = TriggeredReplyTargetAuthority {
            scope: scope.clone(),
            actor: actor.clone(),
            codecs: &target_codecs,
        };
        let notification_context = TriggeredNotificationContext {
            scope: &scope,
            thread_scope: &thread_scope,
            actor: &actor,
            run_id,
            authority: &authority,
        };

        let mut delivered_for_gate_route: Vec<DeliveredChannelMessage> = Vec::new();
        let mut any_delivered = false;
        // A permanent `Denied` from ANY channel must survive later transient
        // failures: only the recorded outcome distinguishes "denied" from
        // "failed", and last-writer-wins would lose the permanent signal.
        let mut any_denied = false;
        let mut last_failure: Option<TriggeredNotificationFailure> = None;
        for notification in &plan.notifications {
            for target in targets
                .iter()
                .filter(|target| notification.audience.includes(target))
            {
                match deliver_notification_to_target(
                    services,
                    &notification_context,
                    notification,
                    target,
                )
                .await
                {
                    Ok(delivered) => {
                        any_delivered = true;
                        if notification.event_kind == RunNotificationEventKind::AuthRequired {
                            messages_to_delete_after_final.extend(
                                delivered
                                    .iter()
                                    .map(|message| (target.extension_id.clone(), message.clone())),
                            );
                        }
                        delivered_for_gate_route.extend(delivered);
                    }
                    Err(failure) => {
                        tracing::warn!(
                            target: TRACE_TARGET,
                            %run_id,
                            extension_id = %target.extension_id,
                            reason = %failure,
                            "background run notification failed for one channel"
                        );
                        if matches!(failure, TriggeredNotificationFailure::Denied) {
                            any_denied = true;
                        }
                        last_failure = Some(failure);
                    }
                }
            }
        }

        // Every conversation the prompt landed in becomes a reply route for
        // this gate, so a bare `approve` from ANY notification channel
        // resolves it.
        if let Some(gate_ref) = plan.gate_ref_for_routing.as_deref()
            && !delivered_for_gate_route.is_empty()
        {
            record_gate_route_if_needed(
                services.route_store.as_ref(),
                run_id,
                &scope.tenant_id,
                &actor.user_id,
                gate_ref,
                &scope,
                &delivered_for_gate_route,
                None,
            )
            .await;
        }

        if !any_delivered {
            let outcome = if any_denied {
                TriggeredRunDeliveryOutcomeKind::Denied
            } else {
                TriggeredRunDeliveryOutcomeKind::Failed
            };
            let _ = last_failure;
            record_triggered_run_outcome(delivery_store, run_id, outcome).await;
            return outcome;
        }

        if plan.keeps_run_parked
            && let Some(marker) = next_blocked_marker
        {
            delivered_blocked_marker = Some(marker);
            continue;
        }

        // Terminal for delivery — clean up prompts that should not persist.
        retract_stale_prompts(
            services,
            &scope,
            run_id,
            &mut messages_to_delete_after_final,
        )
        .await;
        let outcome = TriggeredRunDeliveryOutcomeKind::Delivered;
        record_triggered_run_outcome(delivery_store, run_id, outcome).await;
        return outcome;
    }
}

/// The creator's effective notification targets, resolved at fire time.
///
/// Reads the creator's `CommunicationPreferenceRecord`, applies the spec §7
/// read-time migration (a legacy single-slot record reads back as a
/// one-element set), then re-resolves every id through the OWNER-SCOPED
/// catalog: that revalidates ownership and current availability after any
/// intervening channel disconnect. A target that no longer resolves is
/// skipped with a debug log rather than failing the whole fan-out.
async fn resolve_notification_targets(
    services: &RunDeliveryServices,
    target_codecs: &[Arc<dyn PreferenceTargetCodec>],
    tenant_id: &TenantId,
    creator_user_id: &UserId,
    notification_ref: &str,
) -> Vec<NotificationTarget> {
    let key = CommunicationPreferenceKey {
        scope: DeliveryDefaultScope::personal(tenant_id.clone(), creator_user_id.clone()),
    };
    let owner_scope = OutboundDeliveryTargetScope::new(tenant_id.clone(), creator_user_id.clone());
    let resolution =
        match crate::notification_channel_resolution::resolve_effective_notification_channels_arc(
            &services.communication_preferences,
            &services.delivery_targets,
            &owner_scope,
            key,
            crate::notification_channel_resolution::LookupErrorPolicy::SkipEntry,
        )
        .await
        {
            Ok(resolution) => resolution,
            Err(error) => {
                // silent-ok: a preference/legacy-slot read failure means we cannot
                // know the notification channels; the run itself is untouched and
                // the web app surface still shows the hold.
                tracing::warn!(
                    target: TRACE_TARGET,
                    notification_ref,
                    %error,
                    "background run notification: notification-channel read failed"
                );
                return Vec::new();
            }
        };
    for (target_id, error) in &resolution.skipped {
        // silent-ok: one unreachable catalog entry must not suppress the
        // notification on every other channel.
        tracing::debug!(
            target: TRACE_TARGET,
            notification_ref,
            target_id = %target_id,
            %error,
            "background run notification: notification channel lookup failed; skipped"
        );
    }

    let mut targets = Vec::with_capacity(resolution.channels.len());
    for channel in resolution.channels {
        let entry = match channel {
            crate::notification_channel_resolution::EffectiveNotificationChannel::Resolved(
                entry,
            ) => entry,
            crate::notification_channel_resolution::EffectiveNotificationChannel::Missing {
                target_id,
            } => {
                tracing::debug!(
                    target: TRACE_TARGET,
                    notification_ref,
                    target_id = %target_id,
                    "background run notification: notification channel is no longer available to its owner; skipped"
                );
                continue;
            }
            // The WebUI read surface represents this as an Unavailable row;
            // the notifier has nothing to deliver through it — skip.
            crate::notification_channel_resolution::EffectiveNotificationChannel::LegacyUnresolvable {
                reply_ref: _,
            } => {
                tracing::debug!(
                    target: TRACE_TARGET,
                    notification_ref,
                    "background run notification: legacy notification slot no longer resolves; skipped"
                );
                continue;
            }
        };
        let reply_target_binding_ref = entry.destination;
        let direct_message = target_codecs.iter().any(|codec| {
            codec
                .conversation_for_target(&reply_target_binding_ref)
                .is_some()
                && codec.is_personal_direct_message(&reply_target_binding_ref)
        });
        targets.push(NotificationTarget {
            target: reply_target_binding_ref,
            extension_id: entry.summary.channel.as_str().to_string(),
            direct_message,
        });
    }
    targets
}

/// Build the notification plan for a background run's actionable state.
///
/// ## Background-run channel surface contract
///
/// A background run is **notification-only, plus gate-resolution input** — it
/// is NOT a conversational surface and it never pushes results. Only three
/// states produce output:
///
/// - `BlockedApproval` → gate prompt (approve/deny) on every channel
/// - `BlockedAuth`     → OAuth prompt to personal DMs + a redacted notice
///   elsewhere, or (manual token) cancel + the auth-unavailable notice
/// - `Failed`          → a failure notice on every channel
///
/// Anything else yields `None`. In particular `Completed` delivers NOTHING:
/// the answer lives in the fire's run thread, and putting it on a channel is
/// the model's explicit delivery call (spec §8).
async fn notification_plan_for_state(
    services: &RunDeliveryServices,
    scope: &TurnScope,
    actor: &TurnActor,
    state: &TurnRunState,
    run_id: TurnRunId,
    trigger_label: &str,
) -> Result<Option<TriggeredNotificationPlan>, RunDeliveryError> {
    match state.status {
        TurnStatus::BlockedApproval => {
            let Some(gate_ref) = state.gate_ref.as_ref() else {
                tracing::warn!(
                    target: TRACE_TARGET,
                    %run_id,
                    "background run blocked on approval without gate ref; skipping"
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
            Ok(Some(TriggeredNotificationPlan {
                notifications: vec![TriggeredNotification {
                    event_kind: RunNotificationEventKind::ApprovalNeeded,
                    intent: DeliveryIntent::GatePrompt,
                    // Notification channels are personal DMs or picked shared
                    // channels; the reply instruction is written for the
                    // direct case and stays readable in a shared thread.
                    text: prompts::gate_prompt_text(&view, true),
                    audience: TargetAudience::All,
                    require_direct_message_target: false,
                }],
                gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
                keeps_run_parked: true,
            }))
        }
        TurnStatus::BlockedAuth => {
            let Some(gate_ref) = state.gate_ref.as_ref() else {
                tracing::warn!(
                    target: TRACE_TARGET,
                    %run_id,
                    "background run blocked on auth without gate ref; skipping"
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
                            gate_ref,
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
                // Serviceable = completable from the channel (a provider-hosted
                // OAuth link, or a host-issued pairing code). Triggered
                // delivery always targets a DM, so bearer material is safe here.
                Some(mut view) if prompts::auth_prompt_is_serviceable(&view) => {
                    view.body = prompts::actionable_auth_prompt_body(&view);
                    view.body
                        .push_str(&prompts::triggered_gate_footer(trigger_label));
                    // The run stays parked: the user completes the re-auth in
                    // whichever surface they reach, and the routine resumes.
                    // A missing DM-capable channel is NOT a reason to cancel.
                    Ok(Some(TriggeredNotificationPlan {
                        notifications: vec![
                            TriggeredNotification {
                                event_kind: RunNotificationEventKind::AuthRequired,
                                intent: DeliveryIntent::AuthPrompt,
                                text: prompts::auth_prompt_text(&view, true),
                                audience: TargetAudience::DirectMessage,
                                // Defense in depth: the resolver re-checks the
                                // binding resolved at send time, closing the
                                // snapshot-vs-send race.
                                require_direct_message_target: true,
                            },
                            TriggeredNotification {
                                event_kind: RunNotificationEventKind::RunBlocked,
                                intent: DeliveryIntent::BackgroundRunNotice,
                                text: format!(
                                    "{}{}",
                                    prompts::BACKGROUND_RUN_REAUTH_MESSAGE,
                                    prompts::triggered_update_footer(trigger_label)
                                ),
                                audience: TargetAudience::NonDirectMessage,
                                require_direct_message_target: false,
                            },
                        ],
                        gate_ref_for_routing: Some(gate_ref.as_str().to_string()),
                        keeps_run_parked: true,
                    }))
                }
                view => {
                    // Not serviceable from a channel (manual credential entry,
                    // or an unknown challenge). Deny: cancel the parked run and
                    // notify every channel. Typing a secret into a chat is
                    // never an option.
                    let unavailable = prompts::unserviceable_auth_prompt_message(view.as_ref());
                    cancel_auth_blocked_run(
                        services.turn_coordinator.as_ref(),
                        services.auth_flow_cancel.as_deref(),
                        scope,
                        actor.clone(),
                        run_id,
                        Some(gate_ref.as_str()),
                    )
                    .await?;
                    Ok(Some(TriggeredNotificationPlan {
                        notifications: vec![TriggeredNotification {
                            event_kind: RunNotificationEventKind::RunBlocked,
                            intent: DeliveryIntent::BackgroundRunNotice,
                            text: format!(
                                "{}{}",
                                unavailable,
                                prompts::triggered_update_footer(trigger_label)
                            ),
                            audience: TargetAudience::All,
                            require_direct_message_target: false,
                        }],
                        gate_ref_for_routing: None,
                        keeps_run_parked: false,
                    }))
                }
            }
        }
        TurnStatus::Failed => Ok(Some(TriggeredNotificationPlan {
            notifications: vec![TriggeredNotification {
                event_kind: RunNotificationEventKind::RunBlocked,
                intent: DeliveryIntent::BackgroundRunNotice,
                text: format!(
                    "{}{}",
                    prompts::BACKGROUND_RUN_FAILED_MESSAGE,
                    prompts::triggered_update_footer(trigger_label)
                ),
                audience: TargetAudience::All,
                require_direct_message_target: false,
            }],
            gate_ref_for_routing: None,
            keeps_run_parked: false,
        })),
        _ => Ok(None),
    }
}

/// Deliver one system-originated fire failure to one proven notification
/// target. There is no run id; the stable fire projection ref is the
/// idempotency key persisted by the coordinator.
async fn deliver_pre_submit_failure_to_target(
    context: &PreSubmitFailureDeliveryContext<'_>,
    target: &NotificationTarget,
) -> Result<(), TriggeredNotificationFailure> {
    let projection_access_policy = AllowNoProjectionAccess;
    let outbound_policy = OutboundPolicyService::new(
        context.services.outbound_store.as_ref(),
        &projection_access_policy,
        context.authority,
    );
    let delivery = PrepareCommunicationDeliveryRequest {
        resolution_request: CommunicationDeliveryResolutionRequest {
            scope: context.scope.clone(),
            actor: context.actor.clone(),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind: RunNotificationEventKind::RunBlocked,
                origin: RunNotificationOrigin::SystemEventTarget {
                    reason: SystemEventReasonCode::Trigger,
                    target: target.target.clone(),
                },
            }),
        },
        turn_run_id: None,
        projection_ref: context.failure_ref.clone(),
        attempted_at: Utc::now(),
    };
    let outcome = context
        .services
        .coordinator
        .deliver(
            &outbound_policy,
            context.authority,
            context.services.project_filesystem.as_ref(),
            CoordinatedDeliveryRequest {
                intent: DeliveryIntent::BackgroundRunNotice,
                delivery,
                parts: vec![OutboundPart::Text(context.text.to_string())],
                attachments: Vec::new(),
                thread_anchor: None,
                require_direct_message_target: false,
                extension_id: &target.extension_id,
                thread_scope: context.thread_scope,
            },
        )
        .await
        .map_err(classify_delivery_error)?;
    match outcome {
        CoordinatedDeliveryOutcome::Failed { failure_kind, .. } => Err(
            TriggeredNotificationFailure::Other(format!("delivery failed: {failure_kind:?}")),
        ),
        _ => Ok(()),
    }
}

/// Deliver one notification to one target through the coordinator, returning
/// the delivered channel messages.
async fn deliver_notification_to_target(
    services: &RunDeliveryServices,
    context: &TriggeredNotificationContext<'_>,
    notification: &TriggeredNotification,
    target: &NotificationTarget,
) -> Result<Vec<DeliveredChannelMessage>, TriggeredNotificationFailure> {
    let projection_access_policy = AllowNoProjectionAccess;
    let outbound_policy = OutboundPolicyService::new(
        services.outbound_store.as_ref(),
        &projection_access_policy,
        context.authority,
    );
    let projection_id =
        prompts::run_notification_projection_id(context.run_id, notification.event_kind);
    let projection_ref = ProjectionUpdateRef::new(projection_id).map_err(|reason| {
        TriggeredNotificationFailure::Other(format!("invalid_projection_ref: {reason}"))
    })?;
    let delivery = PrepareCommunicationDeliveryRequest {
        resolution_request: CommunicationDeliveryResolutionRequest {
            scope: context.scope.clone(),
            actor: context.actor.clone(),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind: notification.event_kind,
                origin: RunNotificationOrigin::RunScopedTarget {
                    target: target.target.clone(),
                },
            }),
        },
        turn_run_id: Some(context.run_id),
        projection_ref,
        attempted_at: Utc::now(),
    };

    let outcome = services
        .coordinator
        .deliver(
            &outbound_policy,
            context.authority,
            services.project_filesystem.as_ref(),
            CoordinatedDeliveryRequest {
                intent: notification.intent,
                delivery,
                parts: vec![OutboundPart::Text(notification.text.clone())],
                attachments: Vec::new(),
                thread_anchor: None,
                require_direct_message_target: notification.require_direct_message_target,
                extension_id: &target.extension_id,
                thread_scope: context.thread_scope,
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

/// Retract prompts that must not outlive the run (OAuth links), each through
/// the extension that carried it.
async fn retract_stale_prompts(
    services: &RunDeliveryServices,
    scope: &TurnScope,
    run_id: TurnRunId,
    messages: &mut Vec<(String, DeliveredChannelMessage)>,
) {
    for (extension_id, message) in messages.drain(..) {
        services
            .retract_message_on_extension(&extension_id, scope.clone(), Some(run_id), message)
            .await;
    }
}

/// Classify a [`CoordinatedDeliveryError`] into the typed failure variants
/// used for outcome recording.
fn classify_delivery_error(error: CoordinatedDeliveryError) -> TriggeredNotificationFailure {
    match &error {
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
            target: TRACE_TARGET,
            %run_id,
            error = %error,
            "failed to record background run notification outcome (best-effort)"
        );
    }
}

/// Reply-target authority for background-run notifications: trusts the target
/// the notifier resolved from the creator's own notification-channel catalog
/// (scope and actor must match), decodes it through whichever registered
/// vendor codec owns its grammar, and enforces the DM requirement against the
/// send-time binding.
struct TriggeredReplyTargetAuthority<'a> {
    scope: TurnScope,
    actor: TurnActor,
    codecs: &'a [Arc<dyn PreferenceTargetCodec>],
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
    ) -> Result<crate::VerifiedProductOutboundTargetMetadata, ProductSurfaceFailure> {
        for codec in self.codecs {
            let Some(external_conversation_ref) = codec.conversation_for_target(target.target())
            else {
                continue;
            };
            // Single enforcement point for the OAuth DM rule, checked against
            // the binding resolved NOW (at send time) — race-free against a
            // stale catalog snapshot.
            if require_direct_message && !codec.is_personal_direct_message(target.target()) {
                return Err(ProductSurfaceFailure::OutboundTargetNotDirectMessage);
            }
            return Ok(crate::VerifiedProductOutboundTargetMetadata {
                external_conversation_ref,
                external_actor_ref: None,
            });
        }
        Err(ProductSurfaceFailure::BindingResolutionFailed {
            reason: format!(
                "background run notification: no registered channel codec decodes the binding ref '{}'",
                target.target().as_str()
            ),
        })
    }
}

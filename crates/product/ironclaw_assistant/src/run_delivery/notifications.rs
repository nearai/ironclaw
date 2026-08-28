//! §7a of the unified channel model — the generic notification send facade.
//!
//! The [`DeliveryCoordinator`](crate::delivery_coordinator::DeliveryCoordinator)
//! is the single send path for every channel output; this module names the
//! any-caller notification entry points over it:
//!
//! - [`notify`] delivers one out-of-band notification to **one explicit
//!   channel target** (a catalog-resolved `(target, extension_id)` pair).
//! - [`notify_user`] resolves the user's configured notification channels
//!   (the picker set, gated on the `notifications` capability) with
//!   [`resolve_user_notification_targets`] and fans the notification out.
//!
//! Callers own WHEN and WHAT; the coordinator and the channel's
//! [`ChannelDelivery::deliver`](ironclaw_extension_contracts::channel_adapter::ChannelDelivery::deliver)
//! own HOW. No caller names a channel — the extension id always comes from a
//! resolved catalog entry, never from a call-site literal. The routine
//! driver (`triggered.rs`) is one caller among any number.

use chrono::Utc;

use ironclaw_extension_contracts::channel_adapter::OutboundPart;
use ironclaw_extension_contracts::preference_target::PreferenceTargetCodec;
use ironclaw_host_api::ids::{TenantId, UserId};
use ironclaw_host_api::turn::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    CommunicationPreferenceKey, DeliveryDefaultScope, OutboundDeliveryTargetScope, OutboundError,
    OutboundPolicyService, PrepareCommunicationDeliveryRequest, ProjectionUpdateRef,
    ReplyTargetBindingValidator, RunNotificationContext, RunNotificationEventKind,
    RunNotificationOrigin,
};
use ironclaw_threads::ThreadScope;

use super::observer::AllowNoProjectionAccess;
use super::prompts;
use super::{DeliveredChannelMessage, RunDeliveryServices, delivered_messages_from_outcome};
use crate::delivery_coordinator::{
    CoordinatedDeliveryError, CoordinatedDeliveryOutcome, CoordinatedDeliveryRequest,
    DeliveryIntent,
};
use crate::outbound_delivery::ProductOutboundTargetResolver;

const TRACE_TARGET: &str = "ironclaw::reborn::run_delivery";

/// The trusted context one notification send runs under. The authority and
/// resolver are the same reply-target validation chain every outbound send
/// crosses — a notification is never a policy bypass.
pub struct ChannelNotificationContext<'a> {
    pub scope: &'a TurnScope,
    pub thread_scope: &'a ThreadScope,
    pub actor: &'a TurnActor,
    pub run_id: TurnRunId,
    pub reply_target_authority: &'a dyn ReplyTargetBindingValidator,
    pub target_resolver: &'a dyn ProductOutboundTargetResolver,
}

/// WHAT is being sent: channel-neutral text plus the policy vocabulary the
/// outbound pipeline records for it.
pub struct ChannelNotification {
    pub event_kind: RunNotificationEventKind,
    pub intent: DeliveryIntent,
    pub text: String,
    /// OAuth-URL-bearing payloads must only land in a personal DM.
    pub require_direct_message_target: bool,
    /// Distinguishes durable delivery identities within one
    /// `(run_id, event_kind)` pair (e.g. a gate ref).
    pub notice_discriminator: Option<String>,
}

/// One resolved notification target: the vendor binding ref plus the
/// extension whose channel carries the delivery — read from the catalog
/// entry, never guessed.
#[derive(Clone)]
pub struct NotificationChannelTarget {
    pub target: ReplyTargetBindingRef,
    pub extension_id: String,
    pub direct_message: bool,
}

/// Typed failure classification for a single notification delivery attempt.
pub enum NotificationDeliveryFailure {
    /// The resolved target is inaccessible or rejected the delivery.
    Denied,
    /// Any other delivery or transport failure.
    Other(String),
}

/// Typed coordinator evidence retained for callers that must distinguish a
/// real delivery from an accepted request that performed no egress.
pub(super) enum NotificationDeliveryOutcome {
    NoDelivery,
    Rejected,
    Delivered(Vec<DeliveredChannelMessage>),
    /// The provider accepted the notification, but its durable terminal
    /// delivery confirmation failed. Message refs remain valid for routing
    /// and retraction, while callers retain the weaker evidence state.
    Unconfirmed(Vec<DeliveredChannelMessage>),
}

impl std::fmt::Display for NotificationDeliveryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied => write!(f, "delivery target access denied"),
            Self::Other(reason) => write!(f, "{reason}"),
        }
    }
}

/// The outcome of resolving a user's stored notification channels.
pub struct ResolvedUserNotificationTargets {
    pub targets: Vec<NotificationChannelTarget>,
    /// True when at least one stored channel could not be resolved because
    /// the catalog lookup ERRORED (backend outage), as opposed to resolving
    /// cleanly to "not yours any more". An empty target list with this flag
    /// unset means the user configured no channels — a benign outcome.
    pub lookup_failed: bool,
}

/// `notify(target, content)`: deliver one notification to one explicit
/// channel target through the coordinator (the single send path). The
/// adapter receives it through `ChannelDelivery::deliver`.
pub async fn notify(
    services: &RunDeliveryServices,
    context: &ChannelNotificationContext<'_>,
    notification: &ChannelNotification,
    target: &NotificationChannelTarget,
) -> Result<Vec<DeliveredChannelMessage>, NotificationDeliveryFailure> {
    match notify_with_outcome(services, context, notification, target).await? {
        NotificationDeliveryOutcome::Delivered(messages)
        | NotificationDeliveryOutcome::Unconfirmed(messages) => Ok(messages),
        NotificationDeliveryOutcome::NoDelivery | NotificationDeliveryOutcome::Rejected => {
            Ok(Vec::new())
        }
    }
}

pub(super) async fn notify_with_outcome(
    services: &RunDeliveryServices,
    context: &ChannelNotificationContext<'_>,
    notification: &ChannelNotification,
    target: &NotificationChannelTarget,
) -> Result<NotificationDeliveryOutcome, NotificationDeliveryFailure> {
    let projection_access_policy = AllowNoProjectionAccess;
    let outbound_policy = OutboundPolicyService::new(
        services.outbound_store.as_ref(),
        &projection_access_policy,
        context.reply_target_authority,
    );
    let projection_id = prompts::run_notification_projection_id(
        context.run_id,
        notification.event_kind,
        notification.notice_discriminator.as_deref(),
    );
    let projection_ref = ProjectionUpdateRef::new(projection_id).map_err(|reason| {
        NotificationDeliveryFailure::Other(format!("invalid_projection_ref: {reason}"))
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
            context.target_resolver,
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
        .map_err(classify_notification_delivery_error)?;
    match &outcome {
        CoordinatedDeliveryOutcome::NoDelivery => Ok(NotificationDeliveryOutcome::NoDelivery),
        CoordinatedDeliveryOutcome::Rejected { .. } => Ok(NotificationDeliveryOutcome::Rejected),
        CoordinatedDeliveryOutcome::Failed { failure_kind, .. } => Err(
            NotificationDeliveryFailure::Other(format!("delivery failed: {failure_kind:?}")),
        ),
        CoordinatedDeliveryOutcome::Delivered { .. }
        | CoordinatedDeliveryOutcome::AlreadyDelivered { .. }
        | CoordinatedDeliveryOutcome::StreamDelivered { .. } => Ok(
            NotificationDeliveryOutcome::Delivered(delivered_messages_from_outcome(&outcome)),
        ),
        CoordinatedDeliveryOutcome::DeliveredUnconfirmed { .. }
        | CoordinatedDeliveryOutcome::StreamDeliveredUnconfirmed { .. } => Ok(
            NotificationDeliveryOutcome::Unconfirmed(delivered_messages_from_outcome(&outcome)),
        ),
    }
}

/// `notify_user(user, content)`: resolve the user's configured notification
/// channels and deliver the notification to each. Returns per-target results
/// in resolution order; resolution failure surfaces before anything sends.
pub async fn notify_user(
    services: &RunDeliveryServices,
    target_codecs: &[std::sync::Arc<dyn PreferenceTargetCodec>],
    context: &ChannelNotificationContext<'_>,
    notification: &ChannelNotification,
    tenant_id: &TenantId,
    user_id: &UserId,
    notification_ref: &str,
) -> Result<
    Vec<(
        NotificationChannelTarget,
        Result<Vec<DeliveredChannelMessage>, NotificationDeliveryFailure>,
    )>,
    OutboundError,
> {
    let resolved = resolve_user_notification_targets(
        services,
        target_codecs,
        tenant_id,
        user_id,
        notification_ref,
    )
    .await?;
    let mut outcomes = Vec::with_capacity(resolved.targets.len());
    for target in resolved.targets {
        let outcome = notify(services, context, notification, &target).await;
        outcomes.push((target, outcome));
    }
    Ok(outcomes)
}

/// Resolve one user's stored notification channels through the shared
/// effective-channel resolver (the same set the picker shows), dropping
/// entries that no longer resolve to a target the user owns.
pub async fn resolve_user_notification_targets(
    services: &RunDeliveryServices,
    target_codecs: &[std::sync::Arc<dyn PreferenceTargetCodec>],
    tenant_id: &TenantId,
    user_id: &UserId,
    notification_ref: &str,
) -> Result<ResolvedUserNotificationTargets, OutboundError> {
    let key = CommunicationPreferenceKey {
        scope: DeliveryDefaultScope::personal(tenant_id.clone(), user_id.clone()),
    };
    let owner_scope = OutboundDeliveryTargetScope::new(tenant_id.clone(), user_id.clone());
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
                // A preference read failure means we cannot know the
                // notification channels; propagate it after recording the cause.
                tracing::warn!(
                    target: TRACE_TARGET,
                    notification_ref,
                    %error,
                    "notification fan-out: notification-channel read failed"
                );
                return Err(error);
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
            "notification fan-out: notification channel lookup failed; skipped"
        );
    }
    let lookup_failed = !resolution.skipped.is_empty();

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
                    "notification fan-out: channel is no longer available to its owner; skipped"
                );
                continue;
            }
            crate::notification_channel_resolution::EffectiveNotificationChannel::LegacyUnresolvable {
                reply_ref: _,
            } => {
                tracing::debug!(
                    target: TRACE_TARGET,
                    notification_ref,
                    "notification fan-out: legacy notification slot no longer resolves; skipped"
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
        targets.push(NotificationChannelTarget {
            target: reply_target_binding_ref,
            // The catalog entry's channel name IS the extension id for every
            // registered provider — read from the entry, never guessed.
            extension_id: entry.summary.channel.as_str().to_string(),
            direct_message,
        });
    }
    Ok(ResolvedUserNotificationTargets {
        targets,
        lookup_failed,
    })
}

fn classify_notification_delivery_error(
    error: CoordinatedDeliveryError,
) -> NotificationDeliveryFailure {
    match &error {
        CoordinatedDeliveryError::Outbound(OutboundError::AccessDenied) => {
            NotificationDeliveryFailure::Denied
        }
        _ => NotificationDeliveryFailure::Other(error.to_string()),
    }
}

//! External completion presentation (2026-08-13 design §6.1, §7.9, §8.4):
//! the server-side `local_os` validation policy and the authorized Web Push
//! fallback.
//!
//! Both decisions hang off the same resolution: the owner's *effective*
//! notification channels filtered by the `run_completions` capability, which
//! structurally yields zero or one completion target (only the web-app
//! provider advertises the capability) with no extension-name conditions in
//! this file. Stream liveness is never outbound authority — the push path
//! crosses the same `OutboundPolicyService` + `DeliveryCoordinator` chain as
//! every other channel send, and the delivered payload is the typed
//! [`OutboundPart::RunCompletion`] fact, never reply content.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_extension_contracts::channel_adapter::{OutboundPart, RunCompletionNoticeView};
use ironclaw_extension_contracts::preference_target::ActivePreferenceTargetCodecs;
use ironclaw_host_api::ids::{AgentId, ProjectId, ThreadId};
use ironclaw_host_api::turn::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    CommunicationPreferenceKey, DeliveryDefaultScope, OutboundDeliveryTargetScope, OutboundError,
    PrepareCommunicationDeliveryRequest, ProjectionUpdateRef, ReplyTargetBindingClaim,
    ReplyTargetBindingValidator, ReplyTargetValidationRequest, RunNotificationContext,
    RunNotificationEventKind, RunNotificationOrigin,
};
use ironclaw_threads::ThreadScope;

use super::coordinator::{CompletionPushFallback, LocalOsIntentPolicy};
use super::records::RunCompletionNotice;
use super::store::{RunCompletionNotices, RunCompletionOwner, RunCompletionStoreError};
use crate::delivery_coordinator::{
    CoordinatedDeliveryOutcome, CoordinatedDeliveryRequest, DeliveryIntent,
};
use crate::model_channel_delivery::CodecChannelTargetResolver;
use crate::notification_channel_resolution::{
    EffectiveNotificationChannel, LookupErrorPolicy, resolve_effective_notification_channels_arc,
};
use crate::run_delivery::RunDeliveryServices;
use crate::run_delivery::AllowNoProjectionAccess;
use crate::run_delivery::prompts;

const TRACE_TARGET: &str = "ironclaw::reborn::run_completions";

/// Bounded scan when computing the grouped unread count for one thread's
/// push copy; the wire cap is two digits anyway.
const UNREAD_COUNT_SCAN_LIMIT: usize = 100;

/// One resolved run-completion delivery target: the capability-filtered
/// binding plus the extension whose channel carries it (read from the
/// catalog entry, never guessed).
struct CompletionTarget {
    target: ReplyTargetBindingRef,
    extension_id: String,
}

/// Host-owned web-app enrollment probe (§6.1 "Enrolled"). Composition
/// implements it over the web-app subscription store; this crate never
/// touches push-transport records directly.
#[async_trait::async_trait]
pub trait WebAppEnrollmentProbe: Send + Sync {
    async fn enrollment(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<WebAppEnrollmentSnapshot, String>;
}

/// The probe's answer, split so instance-correlated enrollments (new
/// records) can be matched exactly while legacy records degrade to
/// profile-level presence.
#[derive(Debug, Clone, Default)]
pub struct WebAppEnrollmentSnapshot {
    /// `browser_instance_id`s carried by correlated registrations.
    pub instance_ids: Vec<String>,
    /// Registrations that predate instance correlation.
    pub uncorrelated: usize,
}

/// The §6.1 external-presentation half: `Selected` (a live run-completion
/// target in the owner's effective notification set) and `Enrolled` (a
/// usable host-owned web-app registration) decided server-side; browser
/// permission stays browser-side and is asserted by the intent itself.
pub struct RunCompletionExternalDelivery {
    services: RunDeliveryServices,
    notices: Arc<dyn RunCompletionNotices>,
    codecs: Arc<dyn ActivePreferenceTargetCodecs>,
    enrollments: Arc<dyn WebAppEnrollmentProbe>,
}

impl RunCompletionExternalDelivery {
    pub fn new(
        services: RunDeliveryServices,
        notices: Arc<dyn RunCompletionNotices>,
        codecs: Arc<dyn ActivePreferenceTargetCodecs>,
        enrollments: Arc<dyn WebAppEnrollmentProbe>,
    ) -> Self {
        Self {
            services,
            notices,
            codecs,
            enrollments,
        }
    }

    /// Resolve the owner's effective notification channels and keep the
    /// entries whose target capability includes run completions (§7.9).
    /// Structurally zero or one entry; the first wins deterministically.
    async fn resolve_completion_target(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<Option<CompletionTarget>, OutboundError> {
        let key = CommunicationPreferenceKey {
            scope: DeliveryDefaultScope::personal(
                owner.tenant_id.clone(),
                owner.user_id.clone(),
            ),
        };
        let owner_scope =
            OutboundDeliveryTargetScope::new(owner.tenant_id.clone(), owner.user_id.clone());
        let resolution = resolve_effective_notification_channels_arc(
            &self.services.communication_preferences,
            &self.services.delivery_targets,
            &owner_scope,
            key,
            LookupErrorPolicy::SkipEntry,
        )
        .await?;
        for channel in resolution.channels {
            let EffectiveNotificationChannel::Resolved(entry) = channel else {
                continue;
            };
            if !entry.capabilities.run_completions {
                continue;
            }
            return Ok(Some(CompletionTarget {
                target: entry.destination,
                extension_id: entry.summary.channel.as_str().to_string(),
            }));
        }
        Ok(None)
    }

    /// Deliver the typed completion fact to the resolved target through the
    /// ordinary outbound chain. The durable attempt record is the outcome
    /// evidence either way; this function only reports whether egress was
    /// coordinated.
    async fn deliver_completion(
        &self,
        owner: &RunCompletionOwner,
        notice: &RunCompletionNotice,
        target: &CompletionTarget,
        run_id: TurnRunId,
        agent_id: AgentId,
        thread_id: ThreadId,
        unread_count_for_thread: u16,
    ) {
        let project_id = notice
            .project_id
            .as_deref()
            .map(ProjectId::new)
            .and_then(Result::ok);
        let scope = TurnScope::new_with_owner(
            owner.tenant_id.clone(),
            Some(agent_id.clone()),
            project_id.clone(),
            thread_id.clone(),
            Some(owner.user_id.clone()),
        );
        let thread_scope = ThreadScope {
            tenant_id: owner.tenant_id.clone(),
            agent_id,
            project_id,
            owner_user_id: Some(owner.user_id.clone()),
            mission_id: None,
        };
        let actor = TurnActor::new(owner.user_id.clone());
        let authority = CompletionReplyTargetAuthority {
            scope: scope.clone(),
            actor: actor.clone(),
        };
        let target_resolver = CodecChannelTargetResolver::with_context_label(
            self.codecs.active_preference_target_codecs(),
            "run completion notification",
        );
        let projection_access_policy = AllowNoProjectionAccess;
        let outbound_policy = ironclaw_outbound::OutboundPolicyService::new(
            self.services.outbound_store.as_ref(),
            &projection_access_policy,
            &authority,
        );
        let projection_id = prompts::run_notification_projection_id(
            run_id,
            RunNotificationEventKind::RunCompleted,
            Some(&notice.notice_id),
        );
        let projection_ref = match ProjectionUpdateRef::new(projection_id) {
            Ok(projection_ref) => projection_ref,
            Err(error) => {
                tracing::debug!(
                    target: TRACE_TARGET,
                    %error,
                    "run-completion push skipped: invalid projection ref",
                );
                return;
            }
        };
        let delivery = PrepareCommunicationDeliveryRequest {
            resolution_request: CommunicationDeliveryResolutionRequest {
                scope: scope.clone(),
                actor,
                modality: CommunicationModality::Text,
                intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                    event_kind: RunNotificationEventKind::RunCompleted,
                    origin: RunNotificationOrigin::RunScopedTarget {
                        target: target.target.clone(),
                    },
                }),
            },
            turn_run_id: Some(run_id),
            projection_ref,
            attempted_at: Utc::now(),
        };
        let view = RunCompletionNoticeView {
            notice_id: notice.notice_id.clone(),
            thread_id,
            opaque_thread_tag: notice.thread_tag.clone(),
            unread_count_for_thread,
        };
        let outcome = self
            .services
            .coordinator
            .deliver(
                &outbound_policy,
                &target_resolver,
                self.services.project_filesystem.as_ref(),
                CoordinatedDeliveryRequest {
                    intent: DeliveryIntent::RunCompletionNotice,
                    delivery,
                    parts: vec![OutboundPart::RunCompletion(Box::new(view))],
                    attachments: Vec::new(),
                    thread_anchor: None,
                    require_direct_message_target: false,
                    extension_id: &target.extension_id,
                    thread_scope: &thread_scope,
                },
            )
            .await;
        match outcome {
            Ok(CoordinatedDeliveryOutcome::Failed { failure_kind, .. }) => {
                tracing::debug!(
                    target: TRACE_TARGET,
                    failure = ?failure_kind,
                    "run-completion push attempt failed; durable attempt records the cause",
                );
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(
                    target: TRACE_TARGET,
                    %error,
                    "run-completion push delivery errored; durable attempt records the cause",
                );
            }
        }
    }
}

#[async_trait::async_trait]
impl CompletionPushFallback for RunCompletionExternalDelivery {
    async fn attempt_push(
        &self,
        owner: &RunCompletionOwner,
        notice: &RunCompletionNotice,
    ) -> Result<bool, RunCompletionStoreError> {
        let target = match self.resolve_completion_target(owner).await {
            Ok(Some(target)) => target,
            Ok(None) => return Ok(false),
            Err(error) => {
                // Unknown selection state: fail retryably rather than
                // settling `NoExternalTarget` on a backend outage.
                return Err(RunCompletionStoreError::Unavailable {
                    reason: format!("notification-channel resolution failed: {error}"),
                });
            }
        };
        // The typed pieces the outbound chain needs. A notice that cannot
        // name them cannot be authorized for egress: no push target.
        let Some(agent_id) = notice
            .agent_id
            .as_deref()
            .map(AgentId::new)
            .and_then(Result::ok)
        else {
            return Ok(false);
        };
        let Ok(thread_id) = ThreadId::new(notice.thread_id.clone()) else {
            return Ok(false);
        };
        let Ok(run_uuid) = uuid::Uuid::parse_str(&notice.run_id) else {
            return Ok(false);
        };
        let run_id = TurnRunId::from_uuid(run_uuid);

        // Push ownership is a CAS on the pending record (§5.3): exactly one
        // replica wins. A conflict means the notice was read or otherwise
        // transitioned between the coordinator's scan and this claim — the
        // fallback stands down having sent nothing.
        match self
            .notices
            .claim_push(
                owner,
                &notice.notice_id,
                &notice.terminal_projection_ref,
                Utc::now(),
            )
            .await
        {
            Ok(_) => {}
            Err(RunCompletionStoreError::Conflict { .. }) => return Ok(true),
            Err(error) => return Err(error),
        }
        // Grouped copy count: bounded scan, floor of 1 (this notice).
        let unread_count_for_thread = self
            .notices
            .unread_for_thread(owner, &notice.thread_id, UNREAD_COUNT_SCAN_LIMIT)
            .await
            .map(|unread| unread.len().min(usize::from(u16::MAX)) as u16)
            .unwrap_or(1)
            .max(1);
        // Ownership is durable; ordinary outbound attempt semantics decide
        // delivered/failed/unknown from here (§5.3). A late browser intent
        // cannot recall possible provider egress.
        self.deliver_completion(
            owner,
            notice,
            &target,
            run_id,
            agent_id,
            thread_id,
            unread_count_for_thread,
        )
        .await;
        Ok(true)
    }
}

/// The same shape as the triggered driver's reply-target authority: the
/// notification target was resolved from the owner's stored channel set, so
/// validation asserts scope/actor identity and claims the candidate.
struct CompletionReplyTargetAuthority {
    scope: TurnScope,
    actor: TurnActor,
}

#[async_trait::async_trait]
impl ReplyTargetBindingValidator for CompletionReplyTargetAuthority {
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

#[async_trait::async_trait]
impl LocalOsIntentPolicy for RunCompletionExternalDelivery {
    /// §6.1: a `local_os` intent may win only when the owner's effective
    /// notification set holds a live run-completion target (Selected) AND
    /// the claiming browser profile holds a usable host-owned registration
    /// (Enrolled). Backend uncertainty fails closed.
    async fn allows_local_os(
        &self,
        owner: &RunCompletionOwner,
        browser_instance_id: &str,
    ) -> bool {
        match self.resolve_completion_target(owner).await {
            Ok(Some(_)) => {}
            Ok(None) => return false,
            Err(error) => {
                tracing::debug!(
                    target: TRACE_TARGET,
                    %error,
                    "local_os validation: target resolution failed; denying",
                );
                return false;
            }
        }
        match self.enrollments.enrollment(owner).await {
            Ok(snapshot) => {
                let instance_correlated = snapshot
                    .instance_ids
                    .iter()
                    .any(|id| id == browser_instance_id);
                // Legacy registrations predate instance correlation; degrade
                // to profile-level presence only when NO record carries an
                // instance id (a correlated profile set never matches by
                // count alone).
                instance_correlated
                    || (snapshot.instance_ids.is_empty() && snapshot.uncorrelated > 0)
            }
            Err(error) => {
                tracing::debug!(
                    target: TRACE_TARGET,
                    error = %error,
                    "local_os validation: enrollment probe failed; denying",
                );
                false
            }
        }
    }
}

use async_trait::async_trait;

use crate::resolution_engine::OutboundResolutionEngine;
use crate::validation::validate_delivery_scope_candidate;
use crate::{
    CommunicationDeliveryResolution, CommunicationPreferenceRepository, DeliveryFailureKind,
    OutboundDeliveryAttempt, OutboundDeliveryDecision, OutboundDeliveryId, OutboundDeliveryStatus,
    OutboundError, OutboundPushCandidate, OutboundPushKind, OutboundStateStore,
    PrepareCommunicationDeliveryRequest, PrepareOutboundDeliveryRequest,
    ProjectionSubscriptionRecord, ProjectionSubscriptionRequest, ReplyTargetBindingClaim,
    ReplyTargetValidationRequest, ThreadProjectionAccessClaim, ThreadProjectionAccessGrant,
    ThreadProjectionAccessRequest, ValidatedReplyTargetBinding,
};

#[async_trait]
pub trait ThreadProjectionAccessPolicy: Send + Sync {
    /// Decide whether the request actor may subscribe to projections for the
    /// requested thread/scope. The returned [`ThreadProjectionAccessClaim`] is
    /// **untrusted** — the [`OutboundPolicyService`] mints the sealed
    /// [`ThreadProjectionAccessGrant`] only after verifying the claim's fields
    /// match the original request.
    async fn authorize_projection_access(
        &self,
        request: ThreadProjectionAccessRequest,
    ) -> Result<ThreadProjectionAccessClaim, OutboundError>;
}

#[async_trait]
pub trait ReplyTargetBindingValidator: Send + Sync {
    /// Validate that the candidate's reply target binding is still authorized
    /// for the current scope. The returned [`ReplyTargetBindingClaim`] is
    /// **untrusted** — the [`OutboundPolicyService`] mints the sealed
    /// [`ValidatedReplyTargetBinding`] only after confirming the claim's
    /// target matches the original push candidate (no target substitution).
    async fn validate_reply_target(
        &self,
        request: ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError>;
}

pub struct OutboundPolicyService<'a> {
    store: &'a dyn OutboundStateStore,
    projection_access_policy: &'a dyn ThreadProjectionAccessPolicy,
    reply_target_validator: &'a dyn ReplyTargetBindingValidator,
}

impl<'a> OutboundPolicyService<'a> {
    pub fn new(
        store: &'a dyn OutboundStateStore,
        projection_access_policy: &'a dyn ThreadProjectionAccessPolicy,
        reply_target_validator: &'a dyn ReplyTargetBindingValidator,
    ) -> Self {
        Self {
            store,
            projection_access_policy,
            reply_target_validator,
        }
    }

    pub async fn authorize_subscription(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<ProjectionSubscriptionRecord, OutboundError> {
        let claim = self
            .projection_access_policy
            .authorize_projection_access(ThreadProjectionAccessRequest {
                actor: request.actor.clone(),
                scope: request.scope.clone(),
                thread_id: request.thread_id.clone(),
            })
            .await?;
        validate_access_claim(&request, &claim)?;
        let grant = ThreadProjectionAccessGrant::from_claim(claim);

        let record = ProjectionSubscriptionRecord {
            subscription_id: request.subscription_id,
            actor: grant.actor,
            scope: grant.scope,
            thread_id: grant.thread_id,
            cursor: request.after_cursor,
        };
        self.store.upsert_subscription(record.clone()).await?;
        Ok(record)
    }

    pub async fn prepare_delivery_attempt(
        &self,
        request: PrepareOutboundDeliveryRequest,
    ) -> Result<OutboundDeliveryDecision, OutboundError> {
        if !request.candidate.requires_reply_target_revalidation {
            return Err(OutboundError::InvalidRequest {
                reason: "outbound push candidate must require reply target revalidation",
            });
        }
        validate_delivery_scope_candidate(&request.scope, &request.candidate)?;

        let validation = self
            .reply_target_validator
            .validate_reply_target(ReplyTargetValidationRequest {
                scope: request.scope.clone(),
                actor: request.actor,
                modality: request.modality,
                candidate: request.candidate.clone(),
            })
            .await;

        match validation {
            Ok(claim) => {
                claim.validate_against(&request.candidate)?;
                let target = ValidatedReplyTargetBinding::from_claim(claim);
                let attempt = OutboundDeliveryAttempt {
                    delivery_id: OutboundDeliveryId::new(),
                    scope: request.scope,
                    candidate: request.candidate,
                    status: OutboundDeliveryStatus::Pending,
                    attempted_at: request.attempted_at,
                    failure_kind: None,
                };
                self.store.record_delivery_attempt(attempt.clone()).await?;
                Ok(OutboundDeliveryDecision::Authorized { attempt, target })
            }
            Err(OutboundError::AccessDenied) => {
                let attempt = OutboundDeliveryAttempt {
                    delivery_id: OutboundDeliveryId::new(),
                    scope: request.scope,
                    candidate: request.candidate,
                    status: OutboundDeliveryStatus::Failed,
                    attempted_at: request.attempted_at,
                    failure_kind: Some(DeliveryFailureKind::AuthorizationRevoked),
                };
                self.store.record_delivery_attempt(attempt.clone()).await?;
                Ok(OutboundDeliveryDecision::Rejected { attempt })
            }
            Err(error) if is_transient_validator_error(&error) => {
                let attempt = OutboundDeliveryAttempt {
                    delivery_id: OutboundDeliveryId::new(),
                    scope: request.scope,
                    candidate: request.candidate,
                    status: OutboundDeliveryStatus::Failed,
                    attempted_at: request.attempted_at,
                    failure_kind: Some(DeliveryFailureKind::TransientValidatorError),
                };
                self.store.record_delivery_attempt(attempt.clone()).await?;
                Ok(OutboundDeliveryDecision::Rejected { attempt })
            }
            Err(error) => Err(error),
        }
    }

    pub async fn prepare_communication_delivery_attempt(
        &self,
        request: PrepareCommunicationDeliveryRequest,
        communication_preferences: &dyn CommunicationPreferenceRepository,
    ) -> Result<Option<OutboundDeliveryDecision>, OutboundError> {
        let engine = OutboundResolutionEngine::new(communication_preferences);
        let resolution = engine.resolve(&request.resolution_request).await?;
        self.prepare_communication_delivery_attempt_from_resolution(request, resolution)
            .await
    }

    /// Update the durable status for an attempt prepared by this policy service.
    pub async fn update_delivery_status(
        &self,
        request: crate::UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        self.store.update_delivery_status(request).await
    }

    async fn prepare_communication_delivery_attempt_from_resolution(
        &self,
        request: PrepareCommunicationDeliveryRequest,
        resolution: CommunicationDeliveryResolution,
    ) -> Result<Option<OutboundDeliveryDecision>, OutboundError> {
        let Some(request) = lower_communication_delivery_resolution(request, resolution) else {
            return Ok(None);
        };

        self.prepare_delivery_attempt(request).await.map(Some)
    }
}

fn lower_communication_delivery_resolution(
    request: PrepareCommunicationDeliveryRequest,
    resolution: CommunicationDeliveryResolution,
) -> Option<PrepareOutboundDeliveryRequest> {
    let PrepareCommunicationDeliveryRequest {
        resolution_request,
        turn_run_id,
        projection_ref,
        attempted_at,
    } = request;
    let CommunicationDeliveryResolution::Candidate { candidate } = resolution else {
        return None;
    };
    let kind = OutboundPushKind::from(candidate.kind);

    let scope = resolution_request.scope;
    let actor = resolution_request.actor;
    let modality = resolution_request.modality;
    let candidate = OutboundPushCandidate {
        tenant_id: scope.tenant_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        thread_id: scope.thread_id.clone(),
        turn_run_id,
        target: candidate.target,
        kind,
        projection_ref,
        // Resolution only selects a candidate; every lowered candidate must
        // pass reply-target validation before it can be rendered or sent.
        requires_reply_target_revalidation: true,
    };
    Some(PrepareOutboundDeliveryRequest {
        scope,
        actor,
        modality,
        candidate,
        attempted_at,
    })
}

fn validate_access_claim(
    request: &ProjectionSubscriptionRequest,
    claim: &ThreadProjectionAccessClaim,
) -> Result<(), OutboundError> {
    if request.actor != claim.actor
        || request.scope != claim.scope
        || request.thread_id != claim.thread_id
    {
        return Err(OutboundError::InvalidRequest {
            reason: "projection access claim does not match subscription request",
        });
    }
    Ok(())
}

/// Returns true when a non-`AccessDenied` validator error reflects a
/// transient infrastructure failure rather than a caller bug. Caller-bug
/// errors (e.g. `InvalidRequest`, `SubscriptionScopeMismatch`,
/// `DeliveryNotFound`) propagate to the caller so they are not silently
/// retried; backend/serialization failures become recorded delivery
/// attempts so the saga can retry without losing the audit trail.
fn is_transient_validator_error(error: &OutboundError) -> bool {
    match error {
        // `CasConflict` is the typed compare-and-swap failure from the
        // filesystem-backed store. It is internal to the store layer and the
        // store's bounded retry loop converts it to `Backend` before returning
        // to the service, so it should never reach this classification site —
        // but classify it as transient (retryable) for defence in depth.
        OutboundError::Backend | OutboundError::Serialization | OutboundError::CasConflict => true,
        OutboundError::InvalidRequest { .. }
        | OutboundError::SubscriptionScopeMismatch
        | OutboundError::AccessDenied
        | OutboundError::DeliveryNotFound => false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnScope};

    use super::*;
    use crate::{
        CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
        CommunicationPreferenceRecord, InMemoryOutboundStateStore, OutboundPushKind,
        RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin,
        SourceRouteContext, SystemEventReasonCode,
    };

    #[tokio::test]
    async fn prepare_communication_delivery_attempt_returns_none_for_no_delivery() {
        let store = InMemoryOutboundStateStore::default();
        let validator = TestReplyTargetBindingValidator::default();
        let access_policy = AllowAllProjectionAccessPolicy;
        let service = OutboundPolicyService::new(&store, &access_policy, &validator);
        let scope = turn_scope("thread-no-delivery");
        let request = PrepareCommunicationDeliveryRequest {
            resolution_request: CommunicationDeliveryResolutionRequest {
                scope: scope.clone(),
                actor: actor("alice"),
                modality: CommunicationModality::Text,
                intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                    event_kind: RunNotificationEventKind::DeliveryStatus,
                    origin: RunNotificationOrigin::SystemEvent {
                        reason: SystemEventReasonCode::Generic,
                    },
                }),
            },
            turn_run_id: None,
            projection_ref: projection_ref("projection:no-delivery"),
            attempted_at: now(),
        };

        let decision = service
            .prepare_communication_delivery_attempt(request, &store)
            .await
            .expect("no-delivery resolution succeeds");

        assert!(decision.is_none());
        assert!(
            store
                .list_delivery_attempts(scope)
                .await
                .expect("list delivery attempts")
                .is_empty()
        );
        assert_eq!(validator.calls(), 0);
    }

    #[tokio::test]
    async fn prepare_communication_delivery_attempt_lowers_prompt_kinds_to_gate_required() {
        let store = InMemoryOutboundStateStore::default();
        let validator = TestReplyTargetBindingValidator::default();
        let access_policy = AllowAllProjectionAccessPolicy;
        let service = OutboundPolicyService::new(&store, &access_policy, &validator);
        let scope = turn_scope("thread-approval");
        let approval_target = reply_ref("reply:approval");
        validator.allow(approval_target.clone());
        store
            .put_communication_preference(preference_record(
                &scope,
                Some("reply:final"),
                Some("reply:progress"),
                Some("reply:approval"),
                Some("reply:auth"),
            ))
            .await
            .expect("seed preferences");

        let request = PrepareCommunicationDeliveryRequest {
            resolution_request: CommunicationDeliveryResolutionRequest {
                scope: scope.clone(),
                actor: actor("alice"),
                modality: CommunicationModality::Text,
                intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                    event_kind: RunNotificationEventKind::ApprovalNeeded,
                    origin: RunNotificationOrigin::TriggeredFromSourceRoute {
                        trigger: crate::TriggerCommunicationContext {
                            trigger_origin_ref: crate::TriggerOriginRef::new("trigger:approval")
                                .expect("valid trigger id"),
                            trigger_source_kind: crate::TriggerSourceKind::Schedule,
                            fire_slot: crate::TriggerFireSlot::new("2026-06-08T09:00:00Z")
                                .expect("valid fire slot"),
                        },
                        source_route: SourceRouteContext {
                            reply_target_binding_ref: reply_ref("reply:source"),
                        },
                    },
                }),
            },
            turn_run_id: None,
            projection_ref: projection_ref("projection:approval"),
            attempted_at: now(),
        };

        let decision = service
            .prepare_communication_delivery_attempt(request, &store)
            .await
            .expect("approval prompt resolves");
        let Some(OutboundDeliveryDecision::Authorized { attempt, target }) = decision else {
            panic!("expected an authorized delivery decision");
        };

        assert_eq!(attempt.candidate.kind, OutboundPushKind::GateRequired);
        assert_eq!(target.target(), &approval_target);
        assert_eq!(validator.calls(), 1);
        assert_eq!(
            store
                .list_delivery_attempts(scope)
                .await
                .expect("list delivery attempts")
                .len(),
            1
        );
    }

    #[derive(Default)]
    struct TestReplyTargetBindingValidator {
        allowed: Mutex<HashSet<ReplyTargetBindingRef>>,
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl ReplyTargetBindingValidator for TestReplyTargetBindingValidator {
        async fn validate_reply_target(
            &self,
            request: ReplyTargetValidationRequest,
        ) -> Result<ReplyTargetBindingClaim, OutboundError> {
            *self.calls.lock().expect("lock calls") += 1;
            if self
                .allowed
                .lock()
                .expect("lock allowed")
                .contains(&request.candidate.target)
            {
                Ok(ReplyTargetBindingClaim::new(request.candidate.target))
            } else {
                Err(OutboundError::AccessDenied)
            }
        }
    }

    impl TestReplyTargetBindingValidator {
        fn allow(&self, target: ReplyTargetBindingRef) {
            self.allowed.lock().expect("lock allowed").insert(target);
        }

        fn calls(&self) -> usize {
            *self.calls.lock().expect("lock calls")
        }
    }

    struct AllowAllProjectionAccessPolicy;

    #[async_trait]
    impl ThreadProjectionAccessPolicy for AllowAllProjectionAccessPolicy {
        async fn authorize_projection_access(
            &self,
            request: ThreadProjectionAccessRequest,
        ) -> Result<ThreadProjectionAccessClaim, OutboundError> {
            Ok(ThreadProjectionAccessClaim {
                actor: request.actor,
                scope: request.scope,
                thread_id: request.thread_id,
            })
        }
    }

    fn preference_record(
        scope: &TurnScope,
        final_reply_target: Option<&str>,
        progress_target: Option<&str>,
        approval_prompt_target: Option<&str>,
        auth_prompt_target: Option<&str>,
    ) -> CommunicationPreferenceRecord {
        CommunicationPreferenceRecord {
            scope: crate::DeliveryDefaultScope::personal(scope.tenant_id.clone(), user_id("alice")),
            final_reply_target: final_reply_target.map(reply_ref),
            progress_target: progress_target.map(reply_ref),
            approval_prompt_target: approval_prompt_target.map(reply_ref),
            auth_prompt_target: auth_prompt_target.map(reply_ref),
            default_modality: Some(CommunicationModality::Text),
            updated_at: now(),
            updated_by: user_id("alice"),
        }
    }

    fn turn_scope(thread_id: &str) -> TurnScope {
        TurnScope::new_with_owner(
            TenantId::new("tenant-a").expect("valid tenant"),
            Some(AgentId::new("agent-a").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            ThreadId::new(thread_id).expect("valid thread"),
            Some(user_id("alice")),
        )
    }

    fn actor(user_id_value: &str) -> TurnActor {
        TurnActor::new(user_id(user_id_value))
    }

    fn user_id(value: &str) -> UserId {
        UserId::new(value).expect("valid user")
    }

    fn reply_ref(value: &str) -> ReplyTargetBindingRef {
        ReplyTargetBindingRef::new(value).expect("valid reply target")
    }

    fn projection_ref(value: &str) -> crate::ProjectionUpdateRef {
        crate::ProjectionUpdateRef::new(value).expect("valid projection ref")
    }

    fn now() -> chrono::DateTime<Utc> {
        Utc::now()
    }
}

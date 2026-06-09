use ironclaw_turns::ReplyTargetBindingRef;

use crate::{
    CommunicationDeliveryCandidate, CommunicationDeliveryIntent, CommunicationDeliveryKind,
    CommunicationDeliveryResolution, CommunicationDeliveryResolutionRequest,
    CommunicationPreferenceKey, CommunicationPreferenceRepository, OutboundError,
    RequestedOutboundContext, RunNotificationContext, RunNotificationOrigin,
};

/// Deterministic host-owned outbound target selection.
///
/// The engine only chooses a `CommunicationDeliveryCandidate`. It does not
/// validate the target, record attempts, or mutate any inbound / approval /
/// auth / transcript state.
#[allow(dead_code, reason = "wired into OutboundPolicyService in PR6")]
pub(crate) struct OutboundResolutionEngine<'a> {
    communication_preferences: &'a dyn CommunicationPreferenceRepository,
}

#[allow(dead_code, reason = "wired into OutboundPolicyService in PR6")]
impl<'a> OutboundResolutionEngine<'a> {
    pub(crate) fn new(
        communication_preferences: &'a dyn CommunicationPreferenceRepository,
    ) -> Self {
        Self {
            communication_preferences,
        }
    }

    pub(crate) async fn resolve(
        &self,
        request: &CommunicationDeliveryResolutionRequest,
    ) -> Result<CommunicationDeliveryResolution, OutboundError> {
        let CommunicationDeliveryResolutionRequest {
            scope,
            actor,
            modality: _,
            intent,
        } = request;
        match intent {
            CommunicationDeliveryIntent::RequestedOutbound(context) => {
                Ok(CommunicationDeliveryResolution::candidate(
                    self.candidate_from_requested_outbound(context),
                ))
            }
            CommunicationDeliveryIntent::RunNotification(context) => {
                self.resolve_run_notification_context(scope, actor, context)
                    .await
            }
        }
    }

    fn candidate_from_requested_outbound(
        &self,
        context: &RequestedOutboundContext,
    ) -> CommunicationDeliveryCandidate {
        let kind = context.delivery_kind();
        CommunicationDeliveryCandidate {
            target: context.requested_target.clone(),
            kind,
        }
    }

    async fn resolve_run_notification_context(
        &self,
        scope: &ironclaw_turns::TurnScope,
        actor: &ironclaw_turns::TurnActor,
        context: &RunNotificationContext,
    ) -> Result<CommunicationDeliveryResolution, OutboundError> {
        let kind = context.delivery_kind();
        let target = match &context.origin {
            RunNotificationOrigin::LiveSourceRoute { source_route } => {
                source_route.reply_target_binding_ref.clone()
            }
            RunNotificationOrigin::Triggered { .. } => {
                self.resolve_triggered_target(scope, actor, kind).await?
            }
            RunNotificationOrigin::TriggeredFromSourceRoute { source_route, .. } => {
                self.resolve_triggered_from_source_route_target(
                    kind,
                    &source_route.reply_target_binding_ref,
                    scope,
                    actor,
                )
                .await?
            }
            RunNotificationOrigin::SystemEvent { reason } => {
                return Ok(CommunicationDeliveryResolution::NoDelivery { reason: *reason });
            }
        };

        Ok(CommunicationDeliveryResolution::candidate(
            CommunicationDeliveryCandidate { target, kind },
        ))
    }

    async fn resolve_triggered_from_source_route_target(
        &self,
        kind: CommunicationDeliveryKind,
        source_route_target: &ReplyTargetBindingRef,
        scope: &ironclaw_turns::TurnScope,
        actor: &ironclaw_turns::TurnActor,
    ) -> Result<ReplyTargetBindingRef, OutboundError> {
        match kind {
            CommunicationDeliveryKind::ApprovalPrompt => {
                self.load_preference_target(scope, actor, PreferenceTargetKind::ApprovalPrompt)
                    .await
            }
            CommunicationDeliveryKind::AuthPrompt => {
                self.load_preference_target(scope, actor, PreferenceTargetKind::AuthPrompt)
                    .await
            }
            CommunicationDeliveryKind::FinalReply
            | CommunicationDeliveryKind::ProgressUpdate
            | CommunicationDeliveryKind::DeliveryStatus => Ok(source_route_target.clone()),
        }
    }

    async fn resolve_triggered_target(
        &self,
        scope: &ironclaw_turns::TurnScope,
        actor: &ironclaw_turns::TurnActor,
        kind: CommunicationDeliveryKind,
    ) -> Result<ReplyTargetBindingRef, OutboundError> {
        match kind {
            CommunicationDeliveryKind::FinalReply => {
                self.load_preference_target(scope, actor, PreferenceTargetKind::FinalReply)
                    .await
            }
            CommunicationDeliveryKind::ProgressUpdate
            | CommunicationDeliveryKind::DeliveryStatus => {
                self.load_preference_target(scope, actor, PreferenceTargetKind::Progress)
                    .await
            }
            CommunicationDeliveryKind::ApprovalPrompt => {
                self.load_preference_target(scope, actor, PreferenceTargetKind::ApprovalPrompt)
                    .await
            }
            CommunicationDeliveryKind::AuthPrompt => {
                self.load_preference_target(scope, actor, PreferenceTargetKind::AuthPrompt)
                    .await
            }
        }
    }

    async fn load_preference_target(
        &self,
        scope: &ironclaw_turns::TurnScope,
        actor: &ironclaw_turns::TurnActor,
        kind: PreferenceTargetKind,
    ) -> Result<ReplyTargetBindingRef, OutboundError> {
        let key = default_preference_key(scope, actor);
        let Some(record) = self
            .communication_preferences
            .load_communication_preference(key)
            .await?
        else {
            return Err(missing_preference_error(kind));
        };
        let record = record.record;

        let target = match kind {
            PreferenceTargetKind::FinalReply => record.final_reply_target,
            PreferenceTargetKind::Progress => record.progress_target,
            PreferenceTargetKind::ApprovalPrompt => record.approval_prompt_target,
            PreferenceTargetKind::AuthPrompt => record.auth_prompt_target,
        };

        target.ok_or_else(|| missing_preference_error(kind))
    }
}

fn default_preference_key(
    scope: &ironclaw_turns::TurnScope,
    actor: &ironclaw_turns::TurnActor,
) -> CommunicationPreferenceKey {
    match (
        scope.explicit_owner_user_id(),
        scope.has_explicit_thread_owner(),
        scope.agent_id.clone(),
    ) {
        (Some(owner_user_id), _, _) => {
            CommunicationPreferenceKey::personal(scope.tenant_id.clone(), owner_user_id.clone())
        }
        (None, true, Some(agent_id)) => CommunicationPreferenceKey::shared_agent(
            scope.tenant_id.clone(),
            agent_id,
            scope.project_id.clone(),
        ),
        _ => CommunicationPreferenceKey::personal(scope.tenant_id.clone(), actor.user_id.clone()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "wired into OutboundPolicyService in PR6")]
enum PreferenceTargetKind {
    FinalReply,
    Progress,
    ApprovalPrompt,
    AuthPrompt,
}

#[allow(dead_code, reason = "wired into OutboundPolicyService in PR6")]
fn missing_preference_error(kind: PreferenceTargetKind) -> OutboundError {
    let reason = match kind {
        PreferenceTargetKind::FinalReply => {
            "communication preference final reply target is missing"
        }
        PreferenceTargetKind::Progress => "communication preference progress target is missing",
        PreferenceTargetKind::ApprovalPrompt => {
            "communication preference approval prompt target is missing"
        }
        PreferenceTargetKind::AuthPrompt => {
            "communication preference auth prompt target is missing"
        }
    };
    OutboundError::InvalidRequest { reason }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnScope};

    use super::*;
    use crate::{
        CommunicationModality, CommunicationPreferenceRecord, DeliveryDefaultScope,
        InMemoryOutboundStateStore, RequestedOutboundKind, RunNotificationEventKind,
        SourceRouteContext, SystemEventReasonCode, TriggerCommunicationContext, TriggerFireSlot,
        TriggerOriginRef, TriggerSourceKind, VersionedCommunicationPreferenceRecord,
        WriteCommunicationPreferenceRequest,
    };

    struct BackendErrorPreferenceRepository;

    #[async_trait]
    impl CommunicationPreferenceRepository for BackendErrorPreferenceRepository {
        async fn load_communication_preference(
            &self,
            _key: CommunicationPreferenceKey,
        ) -> Result<Option<VersionedCommunicationPreferenceRecord>, OutboundError> {
            Err(OutboundError::Backend)
        }

        async fn write_communication_preference(
            &self,
            _request: WriteCommunicationPreferenceRequest,
        ) -> Result<VersionedCommunicationPreferenceRecord, OutboundError> {
            Err(OutboundError::Backend)
        }
    }

    #[tokio::test]
    async fn requested_outbound_prefers_the_explicit_target() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);
        let request = requested_request("reply:requested");

        store
            .put_communication_preference(preference_record(
                Some("reply:preferred"),
                None,
                None,
                None,
            ))
            .await
            .expect("seed preference");

        let candidate = engine
            .resolve(&request)
            .await
            .expect("requested outbound resolves");
        let candidate = expect_candidate(candidate);

        assert_eq!(candidate.target, reply_ref("reply:requested"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::FinalReply);
    }

    #[tokio::test]
    async fn requested_outbound_delivery_status_preserves_the_explicit_target() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);
        let request = requested_request_with_kind(
            "reply:delivery-status",
            RequestedOutboundKind::DeliveryStatus,
        );

        store
            .put_communication_preference(preference_record(
                Some("reply:preferred"),
                Some("reply:preferred-progress"),
                Some("reply:preferred-approval"),
                Some("reply:preferred-auth"),
            ))
            .await
            .expect("seed preference");

        let candidate = engine
            .resolve(&request)
            .await
            .expect("requested delivery status resolves");
        let candidate = expect_candidate(candidate);

        assert_eq!(candidate.target, reply_ref("reply:delivery-status"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::DeliveryStatus);
    }

    #[tokio::test]
    async fn triggered_preference_load_backend_error_is_propagated() {
        let engine = OutboundResolutionEngine::new(&BackendErrorPreferenceRepository);

        let error = engine
            .resolve(&run_notification_request(
                RunNotificationEventKind::FinalReplyReady,
                RunNotificationOrigin::Triggered {
                    trigger: trigger_context(),
                },
            ))
            .await
            .expect_err("backend failure must propagate");

        assert!(matches!(error, OutboundError::Backend));
    }

    #[tokio::test]
    async fn live_source_route_final_reply_prefers_the_source_route_over_preferences() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        store
            .put_communication_preference(preference_record(
                Some("reply:preferred"),
                Some("reply:preferred-progress"),
                None,
                None,
            ))
            .await
            .expect("seed preference");

        let candidate = engine
            .resolve(&run_notification_request(
                RunNotificationEventKind::FinalReplyReady,
                RunNotificationOrigin::LiveSourceRoute {
                    source_route: SourceRouteContext {
                        reply_target_binding_ref: reply_ref("reply:source-route"),
                    },
                },
            ))
            .await
            .expect("live source route final reply resolves");
        let candidate = expect_candidate(candidate);

        assert_eq!(candidate.target, reply_ref("reply:source-route"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::FinalReply);
    }

    #[tokio::test]
    async fn live_source_route_approval_needed_uses_the_source_route() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        store
            .put_communication_preference(preference_record(
                Some("reply:final"),
                Some("reply:progress"),
                Some("reply:approval"),
                Some("reply:auth"),
            ))
            .await
            .expect("seed preference");

        assert_resolves_to(
            &engine,
            RunNotificationEventKind::ApprovalNeeded,
            RunNotificationOrigin::LiveSourceRoute {
                source_route: SourceRouteContext {
                    reply_target_binding_ref: reply_ref("reply:source-route"),
                },
            },
            "reply:source-route",
            CommunicationDeliveryKind::ApprovalPrompt,
        )
        .await;
    }

    #[tokio::test]
    async fn live_source_route_auth_required_uses_the_source_route() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        store
            .put_communication_preference(preference_record(
                Some("reply:final"),
                Some("reply:progress"),
                Some("reply:approval"),
                Some("reply:auth"),
            ))
            .await
            .expect("seed preference");

        assert_resolves_to(
            &engine,
            RunNotificationEventKind::AuthRequired,
            RunNotificationOrigin::LiveSourceRoute {
                source_route: SourceRouteContext {
                    reply_target_binding_ref: reply_ref("reply:source-route"),
                },
            },
            "reply:source-route",
            CommunicationDeliveryKind::AuthPrompt,
        )
        .await;
    }

    #[tokio::test]
    async fn triggered_final_reply_uses_the_creator_users_preferred_target() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        store
            .put_communication_preference(preference_record(
                Some("reply:triggered-default"),
                Some("reply:triggered-progress"),
                None,
                None,
            ))
            .await
            .expect("seed preference");

        let candidate = engine
            .resolve(&run_notification_request(
                RunNotificationEventKind::FinalReplyReady,
                RunNotificationOrigin::Triggered {
                    trigger: trigger_context(),
                },
            ))
            .await
            .expect("triggered final reply resolves");
        let candidate = expect_candidate(candidate);

        assert_eq!(candidate.target, reply_ref("reply:triggered-default"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::FinalReply);
    }

    #[tokio::test]
    async fn triggered_final_reply_actor_fallback_uses_actor_personal_default_even_with_agent() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        store
            .put_communication_preference(CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::shared_agent(
                    TenantId::new("tenant-a").expect("valid tenant"),
                    AgentId::new("agent-a").expect("valid agent"),
                    Some(ProjectId::new("project-a").expect("valid project")),
                ),
                final_reply_target: Some(reply_ref("reply:shared-default")),
                progress_target: Some(reply_ref("reply:shared-progress")),
                approval_prompt_target: Some(reply_ref("reply:shared-approval")),
                auth_prompt_target: Some(reply_ref("reply:shared-auth")),
                default_modality: Some(CommunicationModality::Text),
                updated_at: now(),
                updated_by: UserId::new("tenant-admin").expect("valid updater"),
            })
            .await
            .expect("seed shared-agent preference");
        store
            .put_communication_preference(preference_record(
                Some("reply:personal-default"),
                Some("reply:personal-progress"),
                Some("reply:personal-approval"),
                Some("reply:personal-auth"),
            ))
            .await
            .expect("seed personal preference");

        let candidate = engine
            .resolve(&CommunicationDeliveryResolutionRequest {
                scope: actor_fallback_agent_scope(),
                actor: actor("user-a"),
                modality: CommunicationModality::Text,
                intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    origin: RunNotificationOrigin::Triggered {
                        trigger: trigger_context(),
                    },
                }),
            })
            .await
            .expect("actor-fallback triggered final reply resolves");
        let candidate = expect_candidate(candidate);

        assert_eq!(candidate.target, reply_ref("reply:personal-default"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::FinalReply);
    }

    #[tokio::test]
    async fn triggered_final_reply_actor_fallback_without_agent_uses_actor_personal_default() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        store
            .put_communication_preference(preference_record(
                Some("reply:personal-default"),
                Some("reply:personal-progress"),
                Some("reply:personal-approval"),
                Some("reply:personal-auth"),
            ))
            .await
            .expect("seed personal preference");

        let candidate = engine
            .resolve(&CommunicationDeliveryResolutionRequest {
                scope: actor_fallback_agentless_scope(),
                actor: actor("user-a"),
                modality: CommunicationModality::Text,
                intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    origin: RunNotificationOrigin::Triggered {
                        trigger: trigger_context(),
                    },
                }),
            })
            .await
            .expect("actor-fallback triggered final reply resolves");
        let candidate = expect_candidate(candidate);

        assert_eq!(candidate.target, reply_ref("reply:personal-default"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::FinalReply);
    }

    #[tokio::test]
    async fn triggered_final_reply_ownerless_agent_scope_uses_shared_agent_default() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        store
            .put_communication_preference(CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::shared_agent(
                    TenantId::new("tenant-a").expect("valid tenant"),
                    AgentId::new("agent-a").expect("valid agent"),
                    Some(ProjectId::new("project-a").expect("valid project")),
                ),
                final_reply_target: Some(reply_ref("reply:shared-default")),
                progress_target: Some(reply_ref("reply:shared-progress")),
                approval_prompt_target: Some(reply_ref("reply:shared-approval")),
                auth_prompt_target: Some(reply_ref("reply:shared-auth")),
                default_modality: Some(CommunicationModality::Text),
                updated_at: now(),
                updated_by: UserId::new("tenant-admin").expect("valid updater"),
            })
            .await
            .expect("seed shared-agent preference");
        store
            .put_communication_preference(preference_record(
                Some("reply:personal-default"),
                Some("reply:personal-progress"),
                Some("reply:personal-approval"),
                Some("reply:personal-auth"),
            ))
            .await
            .expect("seed personal preference");

        let candidate = engine
            .resolve(&CommunicationDeliveryResolutionRequest {
                scope: ownerless_agent_scope(),
                actor: actor("user-a"),
                modality: CommunicationModality::Text,
                intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    origin: RunNotificationOrigin::Triggered {
                        trigger: trigger_context(),
                    },
                }),
            })
            .await
            .expect("shared-agent triggered final reply resolves");
        let candidate = expect_candidate(candidate);

        assert_eq!(candidate.target, reply_ref("reply:shared-default"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::FinalReply);
    }

    #[tokio::test]
    async fn triggered_final_reply_ownerless_without_agent_uses_actor_personal_default() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        store
            .put_communication_preference(preference_record(
                Some("reply:personal-default"),
                Some("reply:personal-progress"),
                Some("reply:personal-approval"),
                Some("reply:personal-auth"),
            ))
            .await
            .expect("seed personal preference");

        let candidate = engine
            .resolve(&CommunicationDeliveryResolutionRequest {
                scope: ownerless_agentless_scope(),
                actor: actor("user-a"),
                modality: CommunicationModality::Text,
                intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    origin: RunNotificationOrigin::Triggered {
                        trigger: trigger_context(),
                    },
                }),
            })
            .await
            .expect("ownerless agentless triggered final reply resolves");
        let candidate = expect_candidate(candidate);

        assert_eq!(candidate.target, reply_ref("reply:personal-default"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::FinalReply);
    }

    #[tokio::test]
    async fn triggered_default_target_uses_explicit_owner_preferences_when_actor_differs() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);
        let owner = UserId::new("user-owner").expect("valid owner");

        store
            .put_communication_preference(CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(
                    TenantId::new("tenant-a").expect("valid tenant"),
                    owner.clone(),
                ),
                final_reply_target: Some(reply_ref("reply:owner-default")),
                progress_target: None,
                approval_prompt_target: None,
                auth_prompt_target: None,
                default_modality: Some(CommunicationModality::Text),
                updated_at: now(),
                updated_by: owner.clone(),
            })
            .await
            .expect("seed owner preference");
        store
            .put_communication_preference(CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(
                    TenantId::new("tenant-a").expect("valid tenant"),
                    UserId::new("user-actor").expect("valid actor"),
                ),
                final_reply_target: Some(reply_ref("reply:actor-default")),
                progress_target: None,
                approval_prompt_target: None,
                auth_prompt_target: None,
                default_modality: Some(CommunicationModality::Text),
                updated_at: now(),
                updated_by: owner,
            })
            .await
            .expect("seed actor preference");

        let resolution = engine
            .resolve(&CommunicationDeliveryResolutionRequest {
                scope: TurnScope::new_with_owner(
                    TenantId::new("tenant-a").expect("valid tenant"),
                    Some(AgentId::new("agent-a").expect("valid agent")),
                    Some(ProjectId::new("project-a").expect("valid project")),
                    thread_id("thread-owner"),
                    Some(UserId::new("user-owner").expect("valid owner")),
                ),
                actor: actor("user-actor"),
                modality: CommunicationModality::Text,
                intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    origin: RunNotificationOrigin::Triggered {
                        trigger: trigger_context(),
                    },
                }),
            })
            .await
            .expect("triggered final reply resolves");
        let candidate = expect_candidate(resolution);

        assert_eq!(candidate.target, reply_ref("reply:owner-default"));
        assert_eq!(candidate.kind, CommunicationDeliveryKind::FinalReply);
    }

    #[tokio::test]
    async fn triggered_from_source_route_approval_needed_uses_the_approval_prompt_preference() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        store
            .put_communication_preference(preference_record(
                Some("reply:final"),
                Some("reply:progress"),
                Some("reply:approval"),
                Some("reply:auth"),
            ))
            .await
            .expect("seed preference");

        assert_resolves_to(
            &engine,
            RunNotificationEventKind::ApprovalNeeded,
            RunNotificationOrigin::TriggeredFromSourceRoute {
                trigger: trigger_context(),
                source_route: SourceRouteContext {
                    reply_target_binding_ref: reply_ref("reply:source-route"),
                },
            },
            "reply:approval",
            CommunicationDeliveryKind::ApprovalPrompt,
        )
        .await;
    }

    #[tokio::test]
    async fn triggered_from_source_route_auth_required_uses_the_auth_prompt_preference() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        store
            .put_communication_preference(preference_record(
                Some("reply:final"),
                Some("reply:progress"),
                Some("reply:approval"),
                Some("reply:auth"),
            ))
            .await
            .expect("seed preference");

        assert_resolves_to(
            &engine,
            RunNotificationEventKind::AuthRequired,
            RunNotificationOrigin::TriggeredFromSourceRoute {
                trigger: trigger_context(),
                source_route: SourceRouteContext {
                    reply_target_binding_ref: reply_ref("reply:source-route"),
                },
            },
            "reply:auth",
            CommunicationDeliveryKind::AuthPrompt,
        )
        .await;
    }

    #[tokio::test]
    async fn system_event_notifications_are_metadata_only_without_candidate() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        let resolution = engine
            .resolve(&run_notification_request(
                RunNotificationEventKind::ProgressUpdate,
                RunNotificationOrigin::SystemEvent {
                    reason: SystemEventReasonCode::Operator,
                },
            ))
            .await
            .expect("system event resolves as metadata-only");

        assert_eq!(
            resolution,
            CommunicationDeliveryResolution::NoDelivery {
                reason: SystemEventReasonCode::Operator
            }
        );
    }

    #[tokio::test]
    async fn triggered_final_reply_fails_closed_without_a_preference_target() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        let missing_record = engine
            .resolve(&run_notification_request(
                RunNotificationEventKind::FinalReplyReady,
                RunNotificationOrigin::Triggered {
                    trigger: trigger_context(),
                },
            ))
            .await
            .expect_err("missing preference record must fail closed");
        assert!(matches!(
            missing_record,
            OutboundError::InvalidRequest { .. }
        ));

        store
            .put_communication_preference(preference_record(None, None, None, None))
            .await
            .expect("seed empty preference row");

        let missing_target = engine
            .resolve(&run_notification_request(
                RunNotificationEventKind::FinalReplyReady,
                RunNotificationOrigin::Triggered {
                    trigger: trigger_context(),
                },
            ))
            .await
            .expect_err("missing final reply target must fail closed");
        assert!(matches!(
            missing_target,
            OutboundError::InvalidRequest { .. }
        ));
    }

    #[tokio::test]
    async fn non_final_run_notifications_choose_the_correct_targets() {
        let store = InMemoryOutboundStateStore::default();
        let engine = OutboundResolutionEngine::new(&store);

        store
            .put_communication_preference(preference_record(
                Some("reply:final"),
                Some("reply:progress"),
                Some("reply:approval"),
                Some("reply:auth"),
            ))
            .await
            .expect("seed preference");

        assert_resolves_to(
            &engine,
            RunNotificationEventKind::ProgressUpdate,
            RunNotificationOrigin::Triggered {
                trigger: trigger_context(),
            },
            "reply:progress",
            CommunicationDeliveryKind::ProgressUpdate,
        )
        .await;
        assert_resolves_to(
            &engine,
            RunNotificationEventKind::DeliveryStatus,
            RunNotificationOrigin::Triggered {
                trigger: trigger_context(),
            },
            "reply:progress",
            CommunicationDeliveryKind::DeliveryStatus,
        )
        .await;
        assert_resolves_to(
            &engine,
            RunNotificationEventKind::ApprovalNeeded,
            RunNotificationOrigin::Triggered {
                trigger: trigger_context(),
            },
            "reply:approval",
            CommunicationDeliveryKind::ApprovalPrompt,
        )
        .await;
        assert_resolves_to(
            &engine,
            RunNotificationEventKind::AuthRequired,
            RunNotificationOrigin::Triggered {
                trigger: trigger_context(),
            },
            "reply:auth",
            CommunicationDeliveryKind::AuthPrompt,
        )
        .await;
        assert_resolves_to(
            &engine,
            RunNotificationEventKind::ProgressUpdate,
            RunNotificationOrigin::TriggeredFromSourceRoute {
                trigger: trigger_context(),
                source_route: SourceRouteContext {
                    reply_target_binding_ref: reply_ref("reply:source-route"),
                },
            },
            "reply:source-route",
            CommunicationDeliveryKind::ProgressUpdate,
        )
        .await;
        assert_resolves_to(
            &engine,
            RunNotificationEventKind::DeliveryStatus,
            RunNotificationOrigin::TriggeredFromSourceRoute {
                trigger: trigger_context(),
                source_route: SourceRouteContext {
                    reply_target_binding_ref: reply_ref("reply:source-route"),
                },
            },
            "reply:source-route",
            CommunicationDeliveryKind::DeliveryStatus,
        )
        .await;
    }

    fn requested_request(requested_target: &str) -> CommunicationDeliveryResolutionRequest {
        requested_request_with_kind(requested_target, RequestedOutboundKind::ProductMessage)
    }

    fn requested_request_with_kind(
        requested_target: &str,
        requested_kind: RequestedOutboundKind,
    ) -> CommunicationDeliveryResolutionRequest {
        CommunicationDeliveryResolutionRequest {
            scope: scope(),
            actor: actor("user-a"),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RequestedOutbound(RequestedOutboundContext {
                requested_target: reply_ref(requested_target),
                requested_kind,
            }),
        }
    }

    fn run_notification_request(
        event_kind: RunNotificationEventKind,
        origin: RunNotificationOrigin,
    ) -> CommunicationDeliveryResolutionRequest {
        CommunicationDeliveryResolutionRequest {
            scope: scope(),
            actor: actor("user-a"),
            modality: CommunicationModality::Mixed,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind,
                origin,
            }),
        }
    }

    fn preference_record(
        final_reply_target: Option<&str>,
        progress_target: Option<&str>,
        approval_prompt_target: Option<&str>,
        auth_prompt_target: Option<&str>,
    ) -> CommunicationPreferenceRecord {
        CommunicationPreferenceRecord {
            scope: DeliveryDefaultScope::personal(
                TenantId::new("tenant-a").expect("valid tenant"),
                UserId::new("user-a").expect("valid user"),
            ),
            final_reply_target: final_reply_target.map(reply_ref),
            progress_target: progress_target.map(reply_ref),
            approval_prompt_target: approval_prompt_target.map(reply_ref),
            auth_prompt_target: auth_prompt_target.map(reply_ref),
            default_modality: Some(CommunicationModality::Text),
            updated_at: now(),
            updated_by: UserId::new("user-a").expect("valid user"),
        }
    }

    fn scope() -> TurnScope {
        personal_scope()
    }

    fn personal_scope() -> TurnScope {
        TurnScope::new_with_owner(
            TenantId::new("tenant-a").expect("valid tenant"),
            Some(AgentId::new("agent-a").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            thread_id("thread-a"),
            Some(UserId::new("user-a").expect("valid user")),
        )
    }

    fn actor_fallback_agent_scope() -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant-a").expect("valid tenant"),
            Some(AgentId::new("agent-a").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            thread_id("thread-a"),
        )
    }

    fn actor_fallback_agentless_scope() -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant-a").expect("valid tenant"),
            None,
            Some(ProjectId::new("project-a").expect("valid project")),
            thread_id("thread-a"),
        )
    }

    fn ownerless_agent_scope() -> TurnScope {
        TurnScope::new_with_owner(
            TenantId::new("tenant-a").expect("valid tenant"),
            Some(AgentId::new("agent-a").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            thread_id("thread-a"),
            None,
        )
    }

    fn ownerless_agentless_scope() -> TurnScope {
        TurnScope::new_with_owner(
            TenantId::new("tenant-a").expect("valid tenant"),
            None,
            Some(ProjectId::new("project-a").expect("valid project")),
            thread_id("thread-a"),
            None,
        )
    }

    fn actor(user: &str) -> TurnActor {
        TurnActor::new(UserId::new(user).expect("valid user"))
    }

    fn trigger_context() -> TriggerCommunicationContext {
        TriggerCommunicationContext {
            trigger_origin_ref: TriggerOriginRef::new("trigger:daily")
                .expect("valid trigger origin ref"),
            trigger_source_kind: TriggerSourceKind::Schedule,
            fire_slot: TriggerFireSlot::new("2026-05-29T09:00:00Z").expect("valid fire slot"),
        }
    }

    fn thread_id(value: &str) -> ThreadId {
        ThreadId::new(value).expect("valid thread")
    }

    fn reply_ref(value: &str) -> ReplyTargetBindingRef {
        ReplyTargetBindingRef::new(value).expect("valid reply target")
    }

    fn now() -> ironclaw_host_api::Timestamp {
        chrono::Utc::now()
    }

    fn expect_candidate(
        resolution: CommunicationDeliveryResolution,
    ) -> CommunicationDeliveryCandidate {
        match resolution {
            CommunicationDeliveryResolution::Candidate { candidate } => candidate,
            CommunicationDeliveryResolution::NoDelivery { reason } => {
                panic!("expected candidate, got metadata-only resolution for {reason}")
            }
        }
    }

    async fn assert_resolves_to(
        engine: &OutboundResolutionEngine<'_>,
        event_kind: RunNotificationEventKind,
        origin: RunNotificationOrigin,
        expected_target: &str,
        expected_kind: CommunicationDeliveryKind,
    ) {
        let candidate = engine
            .resolve(&run_notification_request(event_kind, origin))
            .await
            .expect("run notification resolves");
        let candidate = expect_candidate(candidate);
        assert_eq!(candidate.target, reply_ref(expected_target));
        assert_eq!(candidate.kind, expected_kind);
    }
}

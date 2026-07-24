use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, InvocationId, ResourceScope, RunId, TenantId, ThreadId, UserId};
use ironclaw_outbound::test_support::in_memory_backed_outbound_state_store;
use ironclaw_outbound::{OutboundStateStore, RunFinalReplyDestination, RunFinalReplyTargetRecord};
use ironclaw_product::{
    CurrentDeliveryTarget, CurrentDeliveryTargetResolver, ProductWorkflowError,
    RebornOutboundDeliveryTargetId, TriggerFinalReplyTargetService,
    WEB_APP_OUTBOUND_DELIVERY_TARGET_ID,
};
use ironclaw_triggers::{TriggerDeliveryTargetId, TriggerError, TriggerRecordValidationKind};
use ironclaw_turns::{
    AcceptedMessageRef, EventCursor, GetRunStateRequest, ProductTurnContext, ReplyTargetBindingRef,
    RunOriginAdapter, RunProfileId, RunProfileVersion, SourceBindingRef, TurnActor, TurnError,
    TurnId, TurnOriginKind, TurnOwner, TurnRunId, TurnRunState, TurnScope, TurnStateStore,
    TurnStatus, TurnSurfaceType,
};

struct CurrentTargets {
    states: HashMap<TurnRunId, TurnRunState>,
    by_id: HashMap<RebornOutboundDeliveryTargetId, RunFinalReplyDestination>,
    by_binding: HashMap<ReplyTargetBindingRef, RebornOutboundDeliveryTargetId>,
}

#[async_trait]
impl TurnStateStore for CurrentTargets {
    async fn submit_turn(
        &self,
        _request: ironclaw_turns::SubmitTurnRequest,
        _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        _run_profile_resolver: &dyn ironclaw_turns::RunProfileResolver,
    ) -> Result<ironclaw_turns::SubmitTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "test source store is read-only".to_string(),
        })
    }

    async fn resume_turn(
        &self,
        _request: ironclaw_turns::ResumeTurnRequest,
    ) -> Result<ironclaw_turns::ResumeTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "test source store is read-only".to_string(),
        })
    }

    async fn retry_turn(
        &self,
        _request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "test source store is read-only".to_string(),
        })
    }

    async fn request_cancel(
        &self,
        _request: ironclaw_turns::CancelRunRequest,
    ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "test source store is read-only".to_string(),
        })
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.states
            .get(&request.run_id)
            .filter(|state| state.scope == request.scope)
            .cloned()
            .ok_or(TurnError::ScopeNotFound)
    }
}

#[async_trait]
impl CurrentDeliveryTargetResolver for CurrentTargets {
    async fn resolve_current_target(
        &self,
        _scope: &TurnScope,
        _actor: &TurnActor,
        _target: &ReplyTargetBindingRef,
    ) -> Result<Option<CurrentDeliveryTarget>, ProductWorkflowError> {
        Ok(None)
    }

    async fn resolve_current_destination(
        &self,
        _scope: &ResourceScope,
        target_id: &RebornOutboundDeliveryTargetId,
    ) -> Result<Option<RunFinalReplyDestination>, ProductWorkflowError> {
        Ok(self.by_id.get(target_id).cloned())
    }

    async fn resolve_current_target_id(
        &self,
        _scope: &ResourceScope,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<RebornOutboundDeliveryTargetId>, ProductWorkflowError> {
        Ok(self.by_binding.get(target).cloned())
    }
}

fn scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").expect("tenant"),
        user_id: UserId::new("user-a").expect("user"),
        agent_id: Some(AgentId::new("agent-a").expect("agent")),
        project_id: None,
        mission_id: None,
        thread_id: Some(ThreadId::new("thread-a").expect("thread")),
        invocation_id: InvocationId::new(),
    }
}

fn turn_scope(scope: &ResourceScope) -> TurnScope {
    TurnScope::new_with_owner(
        scope.tenant_id.clone(),
        scope.agent_id.clone(),
        scope.project_id.clone(),
        scope.thread_id.clone().expect("thread"),
        Some(scope.user_id.clone()),
    )
}

fn run_state(
    scope: TurnScope,
    run_id: TurnRunId,
    origin: TurnOriginKind,
    reply_target: &str,
) -> TurnRunState {
    let actor = TurnActor::new(scope.explicit_owner_user_id().expect("owner").clone());
    TurnRunState {
        scope,
        actor: Some(actor.clone()),
        turn_id: TurnId::new(),
        run_id,
        status: TurnStatus::Running,
        accepted_message_ref: AcceptedMessageRef::new(format!("message:{run_id}"))
            .expect("message ref"),
        source_binding_ref: SourceBindingRef::new(format!("source:{run_id}")).expect("source ref"),
        reply_target_binding_ref: ReplyTargetBindingRef::new(reply_target).expect("reply target"),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        model_usage: None,
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(1),
        product_context: Some(ProductTurnContext::new(
            origin,
            Some(TurnSurfaceType::Direct),
            (origin == TurnOriginKind::Inbound)
                .then(|| RunOriginAdapter::new("test-channel").expect("adapter")),
            TurnOwner::Personal {
                user: actor.user_id,
            },
        )),
        resume_disposition: None,
    }
}

#[tokio::test]
async fn trigger_targets_validate_current_authority_and_inherit_the_exact_source_destination() {
    let scope = scope();
    let turn_scope = turn_scope(&scope);
    let actor = TurnActor::new(scope.user_id.clone());
    let external_run = TurnRunId::new();
    let web_app_run = TurnRunId::new();
    let source_route_run = TurnRunId::new();
    let removed_target_run = TurnRunId::new();
    let missing_actor_run = TurnRunId::new();
    let states = [
        (
            external_run,
            run_state(
                turn_scope.clone(),
                external_run,
                TurnOriginKind::Inbound,
                "reply:source-route-unused",
            ),
        ),
        (
            web_app_run,
            run_state(
                turn_scope.clone(),
                web_app_run,
                TurnOriginKind::WebUi,
                "reply:web-app-host-only",
            ),
        ),
        (
            source_route_run,
            run_state(
                turn_scope.clone(),
                source_route_run,
                TurnOriginKind::Inbound,
                "reply:source-route",
            ),
        ),
        (
            removed_target_run,
            run_state(
                turn_scope.clone(),
                removed_target_run,
                TurnOriginKind::Inbound,
                "reply:source-route-unused",
            ),
        ),
        (missing_actor_run, {
            let mut state = run_state(
                turn_scope.clone(),
                missing_actor_run,
                TurnOriginKind::Inbound,
                "reply:source-route",
            );
            state.actor = None;
            state
        }),
    ]
    .into_iter()
    .collect();
    let outbound_store: Arc<dyn OutboundStateStore> =
        Arc::new(in_memory_backed_outbound_state_store());
    let external_id = RebornOutboundDeliveryTargetId::new("target:external").expect("target id");
    let external_binding = ReplyTargetBindingRef::new("reply:sealed-external").expect("binding");
    let removed_binding = ReplyTargetBindingRef::new("reply:removed-external").expect("binding");
    let source_route_id =
        RebornOutboundDeliveryTargetId::new("target:source-route").expect("target id");
    let source_route_binding = ReplyTargetBindingRef::new("reply:source-route").expect("binding");
    outbound_store
        .put_run_final_reply_target(RunFinalReplyTargetRecord {
            run_id: external_run,
            scope: turn_scope.clone(),
            actor: actor.clone(),
            destination: RunFinalReplyDestination::External {
                reply_target_binding_ref: external_binding.clone(),
            },
        })
        .await
        .expect("external destination seals");
    outbound_store
        .put_run_final_reply_target(RunFinalReplyTargetRecord {
            run_id: removed_target_run,
            scope: turn_scope.clone(),
            actor: actor.clone(),
            destination: RunFinalReplyDestination::External {
                reply_target_binding_ref: removed_binding,
            },
        })
        .await
        .expect("removed destination seals");
    outbound_store
        .put_run_final_reply_target(RunFinalReplyTargetRecord {
            run_id: web_app_run,
            scope: turn_scope,
            actor,
            destination: RunFinalReplyDestination::WebApp,
        })
        .await
        .expect("web app destination seals");
    let targets = Arc::new(CurrentTargets {
        states,
        by_id: [
            (
                external_id.clone(),
                RunFinalReplyDestination::External {
                    reply_target_binding_ref: external_binding.clone(),
                },
            ),
            (
                source_route_id.clone(),
                RunFinalReplyDestination::External {
                    reply_target_binding_ref: source_route_binding.clone(),
                },
            ),
            (
                RebornOutboundDeliveryTargetId::new(WEB_APP_OUTBOUND_DELIVERY_TARGET_ID)
                    .expect("WebApp target id"),
                RunFinalReplyDestination::WebApp,
            ),
        ]
        .into_iter()
        .collect(),
        by_binding: [
            (external_binding.clone(), external_id.clone()),
            (source_route_binding, source_route_id.clone()),
        ]
        .into_iter()
        .collect(),
    });
    let service = TriggerFinalReplyTargetService::new(
        Arc::clone(&targets) as Arc<dyn TurnStateStore>,
        outbound_store,
        targets as Arc<dyn CurrentDeliveryTargetResolver>,
    );

    service
        .validate_explicit_target(
            &scope,
            &TriggerDeliveryTargetId::new(external_id.as_str()).expect("trigger target"),
        )
        .await
        .expect("currently authorized explicit target is accepted");
    service
        .validate_explicit_target(
            &scope,
            &TriggerDeliveryTargetId::new(WEB_APP_OUTBOUND_DELIVERY_TARGET_ID)
                .expect("host WebApp target"),
        )
        .await
        .expect("host-owned WebApp target remains valid");
    let denied = service
        .validate_explicit_target(
            &scope,
            &TriggerDeliveryTargetId::new("target:removed").expect("trigger target"),
        )
        .await
        .expect_err("removed explicit target must fail closed");
    assert!(matches!(
        denied,
        TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::DeliveryTargetInvalid,
            ..
        }
    ));

    let inherited_external = service
        .resolve_source_run_target(&scope, Some(RunId::from_uuid(external_run.as_uuid())))
        .await
        .expect("sealed external destination resolves")
        .expect("external destination is inherited");
    assert_eq!(inherited_external.as_str(), external_id.as_str());

    let inherited_web_app = service
        .resolve_source_run_target(&scope, Some(RunId::from_uuid(web_app_run.as_uuid())))
        .await
        .expect("sealed WebApp destination resolves")
        .expect("WebApp is inherited as a valid host target");
    assert_eq!(
        inherited_web_app.as_str(),
        WEB_APP_OUTBOUND_DELIVERY_TARGET_ID
    );

    let inherited_source_route = service
        .resolve_source_run_target(&scope, Some(RunId::from_uuid(source_route_run.as_uuid())))
        .await
        .expect("existing source route resolves")
        .expect("source route remains an inheritance fallback");
    assert_eq!(inherited_source_route.as_str(), source_route_id.as_str());

    let removed_inherited = service
        .resolve_source_run_target(&scope, Some(RunId::from_uuid(removed_target_run.as_uuid())))
        .await
        .expect_err(
            "a sealed external destination removed from current authority must fail closed",
        );
    assert!(matches!(
        removed_inherited,
        TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::DeliveryTargetInvalid,
            ..
        }
    ));

    let missing_actor = service
        .resolve_source_run_target(&scope, Some(RunId::from_uuid(missing_actor_run.as_uuid())))
        .await
        .expect_err("a source run without a sealed actor must fail closed");
    assert!(matches!(
        missing_actor,
        TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::DeliveryTargetInvalid,
            ..
        }
    ));
}

use super::turn_events::WEBUI_TURN_EVENT_PAGE_LIMIT;
use super::*;

use async_trait::async_trait;
use ironclaw_events::{InMemoryDurableEventLog, RuntimeEvent};
use ironclaw_host_api::{
    AgentId, CapabilityId, InvocationId, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_product_adapters::{ProductOutboundEnvelope, ProductOutboundPayload};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor as TurnEventCursor,
    GateRef, GetRunStateRequest, ResumeTurnRequest, ResumeTurnResponse, RunProfileId,
    RunProfileVersion, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnError,
    TurnEventKind, TurnEventPage, TurnEventProjectionCursor, TurnLifecycleEvent, TurnRunState,
    TurnStatus,
};

#[tokio::test]
async fn webui_event_stream_drains_run_status_projection_from_event_stream_manager() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let invocation_id = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::model_started(
            ResourceScope {
                tenant_id: tenant_id.clone(),
                user_id: user_id.clone(),
                agent_id: Some(agent_id.clone()),
                project_id: None,
                mission_id: None,
                thread_id: Some(thread_id.clone()),
                invocation_id,
            },
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    );
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    let ProductOutboundPayload::ProjectionSnapshot { state } = events[0].payload() else {
        panic!("expected projection snapshot");
    };
    assert_eq!(state.items.len(), 1);
    assert!(matches!(
        state.items[0],
        ProductProjectionItem::RunStatus { ref status, .. } if status == "running"
    ));
}

#[tokio::test]
async fn webui_event_stream_resumes_after_serialized_projection_cursor() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let first_run = InvocationId::new();
    let second_run = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::model_started(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, first_run),
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();

    let event_log_dyn: Arc<dyn DurableEventLog> = event_log.clone();
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    );
    let first = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: TurnScope::new(
                tenant_id.clone(),
                Some(agent_id.clone()),
                None,
                thread_id.clone(),
            ),
            after_cursor: None,
        })
        .await
        .unwrap();

    event_log
        .append(RuntimeEvent::model_started(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, second_run),
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();
    let resumed = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
            after_cursor: Some(first[0].projection_cursor().clone()),
        })
        .await
        .unwrap();

    assert!(contains_run_status(&resumed, second_run, "running"));
    assert!(!contains_run_status(&resumed, first_run, "running"));
}

#[tokio::test]
async fn webui_event_stream_resumes_mixed_batch_without_skipping_turn_event() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let runtime_run = InvocationId::new();
    let turn_run = TurnRunId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::model_started(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, runtime_run),
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();

    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                run_id: turn_run,
                status: TurnStatus::BlockedAuth,
                kind: TurnEventKind::Blocked,
                sanitized_reason: Some("GitHub authentication required".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1)),
        }),
    );

    let first = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(first.len(), 2);
    assert!(matches!(
        first[0].payload(),
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
    assert!(matches!(
        first[1].payload(),
        ProductOutboundPayload::AuthPrompt(_)
    ));

    let resumed = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(first[0].projection_cursor().clone()),
        })
        .await
        .unwrap();

    assert_eq!(resumed.len(), 1);
    assert!(matches!(
        resumed[0].payload(),
        ProductOutboundPayload::AuthPrompt(prompt)
            if prompt.turn_run_id == turn_run
                && prompt.auth_request_ref == "gate:auth-required"
    ));
}

#[tokio::test]
async fn webui_event_stream_rejects_foreign_composite_turn_cursor() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_a = ThreadId::new("webui-events-thread-a").unwrap();
    let thread_b = ThreadId::new("webui-events-thread-b").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let scope_a = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_a.clone(),
    );
    let scope_b = TurnScope::new(tenant_id, Some(agent_id), None, thread_b);
    let cursor = product_cursor_from_webui_cursor(&WebuiProjectionCursor {
        runtime: None,
        turn: Some(TurnEventProjectionCursor::for_scope(
            scope_a,
            TurnEventCursor(10),
        )),
    })
    .unwrap();
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    );

    let error = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope: scope_b,
            after_cursor: Some(cursor),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            ..
        }
    ));
}

#[tokio::test]
async fn turn_event_bridge_maps_foreign_cursor_rebase_to_invalid_identifier() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let scope_a = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        ThreadId::new("webui-events-thread-a").unwrap(),
    );
    let scope_b = TurnScope::new(
        tenant_id,
        Some(agent_id),
        None,
        ThreadId::new("webui-events-thread-b").unwrap(),
    );
    let run_id = TurnRunId::new();
    let bridge = super::turn_events::TurnEventBridge::enabled(
        Arc::new(FakeTurnEventSource { events: Vec::new() }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope_b, &user_id, run_id, TurnEventCursor(1)),
        }),
    );

    let error = match bridge
        .drain(
            &scope_b,
            Some(TurnEventProjectionCursor::for_scope(
                scope_a,
                TurnEventCursor(10),
            )),
        )
        .await
    {
        Ok(_) => panic!("foreign turn cursor must be rejected"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            ..
        }
    ));
}

#[tokio::test]
async fn turn_event_bridge_maps_source_error_to_retryable_workflow_rejection() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let scope = TurnScope::new(
        tenant_id,
        Some(agent_id),
        None,
        ThreadId::new("webui-events-thread").unwrap(),
    );
    let run_id = TurnRunId::new();
    let bridge = super::turn_events::TurnEventBridge::enabled(
        Arc::new(FailingTurnEventSource),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, run_id, TurnEventCursor(1)),
        }),
    );

    let error = match bridge.drain(&scope, None).await {
        Ok(_) => panic!("source failure must be rejected"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unavailable,
            status_code: 503,
            retryable: true,
            ..
        }
    ));
}

#[tokio::test]
async fn webui_event_stream_emits_keepalive_when_only_turn_cursor_advances() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let run_id = TurnRunId::new();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                run_id,
                status: TurnStatus::Running,
                kind: TurnEventKind::RunnerHeartbeat,
                sanitized_reason: None,
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, run_id, TurnEventCursor(1)),
        }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].payload(),
        ProductOutboundPayload::KeepAlive
    ));
    let parsed = parse_webui_projection_cursor(events[0].projection_cursor().as_str()).unwrap();
    assert_eq!(
        parsed.turn,
        Some(TurnEventProjectionCursor::for_scope(
            scope,
            TurnEventCursor(1)
        ))
    );
}

#[tokio::test]
async fn webui_event_stream_reads_past_filtered_turn_event_pages() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let run_id = TurnRunId::new();
    let mut events = (1..=WEBUI_TURN_EVENT_PAGE_LIMIT as u64)
        .map(|cursor| TurnLifecycleEvent {
            cursor: TurnEventCursor(cursor),
            scope: scope.clone(),
            run_id,
            status: TurnStatus::Running,
            kind: TurnEventKind::RunnerHeartbeat,
            sanitized_reason: None,
        })
        .collect::<Vec<_>>();
    events.push(TurnLifecycleEvent {
        cursor: TurnEventCursor(WEBUI_TURN_EVENT_PAGE_LIMIT as u64 + 1),
        scope: scope.clone(),
        run_id,
        status: TurnStatus::BlockedAuth,
        kind: TurnEventKind::Blocked,
        sanitized_reason: Some("GitHub authentication required".to_string()),
    });
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource { events }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(
                &scope,
                &user_id,
                run_id,
                TurnEventCursor(WEBUI_TURN_EVENT_PAGE_LIMIT as u64 + 1),
            ),
        }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].payload(),
        ProductOutboundPayload::AuthPrompt(prompt)
            if prompt.turn_run_id == run_id
                && prompt.body == "GitHub authentication required"
    ));
}

#[tokio::test]
async fn webui_event_stream_does_not_prompt_for_stale_blocked_event() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let run_id = TurnRunId::new();
    let mut state = turn_run_state(&scope, &user_id, run_id, TurnEventCursor(1));
    state.event_cursor = TurnEventCursor(2);
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                run_id,
                status: TurnStatus::BlockedAuth,
                kind: TurnEventKind::Blocked,
                sanitized_reason: Some("stale auth gate".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator { state }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].payload(),
        ProductOutboundPayload::ProjectionUpdate { .. }
    ));
}

#[tokio::test]
async fn webui_event_stream_uses_request_actor_for_projection_scope() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let owner_user_id = UserId::new("webui-events-owner").unwrap();
    let other_user_id = UserId::new("webui-events-other").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::model_started(
            resource_scope(
                &tenant_id,
                &owner_user_id,
                &agent_id,
                &thread_id,
                InvocationId::new(),
            ),
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    );
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(other_user_id),
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(
        events.is_empty(),
        "projection stream must not read another user's event stream through a hidden runtime actor"
    );
}

#[tokio::test]
async fn webui_event_stream_rejects_malformed_projection_cursor() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    );

    let error = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
            after_cursor: Some(ProductProjectionCursor::new("not-json").unwrap()),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            ..
        }
    ));
}

fn resource_scope(
    tenant_id: &TenantId,
    user_id: &UserId,
    agent_id: &AgentId,
    thread_id: &ThreadId,
    invocation_id: InvocationId,
) -> ResourceScope {
    ResourceScope {
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        agent_id: Some(agent_id.clone()),
        project_id: None,
        mission_id: None,
        thread_id: Some(thread_id.clone()),
        invocation_id,
    }
}

fn contains_run_status(
    events: &[ProductOutboundEnvelope],
    invocation_id: InvocationId,
    expected_status: &str,
) -> bool {
    let expected_run_id = TurnRunId::from_uuid(invocation_id.as_uuid());
    events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionSnapshot { state }
        | ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus { run_id, status }
                    if *run_id == expected_run_id && status == expected_status
            )
        }),
        _ => false,
    })
}

struct FakeTurnEventSource {
    events: Vec<TurnLifecycleEvent>,
}

#[async_trait]
impl TurnEventProjectionSource for FakeTurnEventSource {
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        after: Option<TurnEventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let after = after.unwrap_or_default();
        let mut events = self
            .events
            .iter()
            .filter(|event| &event.scope == scope && event.cursor > after)
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by_key(|event| event.cursor);
        let truncated = events.len() > limit;
        if truncated {
            events.truncate(limit);
        }
        let next_cursor = events.last().map(|event| event.cursor).unwrap_or(after);
        Ok(TurnEventPage {
            entries: events,
            next_cursor,
            truncated,
            rebase_required: None,
        })
    }
}

struct FailingTurnEventSource;

#[async_trait]
impl TurnEventProjectionSource for FailingTurnEventSource {
    async fn read_turn_events_after(
        &self,
        _scope: &TurnScope,
        _after: Option<TurnEventCursor>,
        _limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        Err(TurnError::Unavailable {
            reason: "projection source unavailable".to_string(),
        })
    }
}

struct FakeTurnCoordinator {
    state: TurnRunState,
}

#[async_trait]
impl TurnCoordinator for FakeTurnCoordinator {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("projection tests only read run state")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unreachable!("projection tests only read run state")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        unreachable!("projection tests only read run state")
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        if request.scope == self.state.scope && request.run_id == self.state.run_id {
            Ok(self.state.clone())
        } else {
            Err(TurnError::ScopeNotFound)
        }
    }
}

fn turn_run_state(
    scope: &TurnScope,
    user_id: &UserId,
    run_id: TurnRunId,
    cursor: TurnEventCursor,
) -> TurnRunState {
    TurnRunState {
        scope: scope.clone(),
        actor: Some(TurnActor::new(user_id.clone())),
        turn_id: ironclaw_turns::TurnId::new(),
        run_id,
        status: TurnStatus::BlockedAuth,
        accepted_message_ref: AcceptedMessageRef::new("message:auth-required").unwrap(),
        source_binding_ref: SourceBindingRef::new("source:auth-required").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply:auth-required").unwrap(),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        received_at: chrono::Utc::now(),
        checkpoint_id: None,
        gate_ref: Some(GateRef::new("gate:auth-required").unwrap()),
        failure: None,
        event_cursor: cursor,
    }
}

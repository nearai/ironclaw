use super::*;

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
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedAuth,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: GateRef::new("gate:auth-required").unwrap(),
                    gate_kind: TurnBlockedGateKind::Auth,
                }),
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
async fn webui_event_stream_projects_blocked_dependent_run_status() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-dependent-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
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
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedDependentRun,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: GateRef::new("gate:await-dependent-run").unwrap(),
                    gate_kind: TurnBlockedGateKind::AwaitDependentRun,
                }),
                sanitized_reason: Some("Waiting for dependent run".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                status: TurnStatus::BlockedDependentRun,
                gate_ref: Some(GateRef::new("gate:await-dependent-run").unwrap()),
                ..turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1))
            },
        }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus { run_id, status, .. }
                    if *run_id == turn_run && status == "blocked_dependent_run"
            )
        }),
        _ => false,
    }));
}

#[tokio::test]
async fn webui_event_stream_projects_failed_run_failure_summary() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-failed-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
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
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::Failed,
                kind: TurnEventKind::Failed,
                blocked_gate: None,
                sanitized_reason: Some("lease_expired".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1)),
        }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus {
                    run_id,
                    status,
                    failure_category: Some(category),
                    failure_summary: Some(summary),
                } if *run_id == turn_run
                    && status == "failed"
                    && category == "lease_expired"
                    && summary == "The run failed because its runner lease expired."
            )
        }),
        _ => false,
    }));
}

#[tokio::test]
async fn webui_event_stream_uses_model_failure_explanation_when_available() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-model-failed-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
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
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::Failed,
                kind: TurnEventKind::Failed,
                blocked_gate: None,
                sanitized_reason: Some("driver_invalid_request".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1)),
        }),
    )
    .with_failure_explainer(Arc::new(FakeFailureExplainer {
        explanation:
            "The run asked the driver for an invalid operation, so it stopped before replying."
                .to_string(),
    }));

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus {
                    run_id,
                    status,
                    failure_category: Some(category),
                    failure_summary: Some(summary),
                } if *run_id == turn_run
                    && status == "failed"
                    && category == "driver_invalid_request"
                    && summary
                        == "The run asked the driver for an invalid operation, so it stopped before replying."
            )
        }),
        _ => false,
    }));
}

#[tokio::test]
async fn webui_event_stream_caches_model_failure_explanation_across_replay() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-cache-failed-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::Failed,
                kind: TurnEventKind::Failed,
                blocked_gate: None,
                sanitized_reason: Some("driver_invalid_request".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1)),
        }),
    )
    .with_failure_explainer(Arc::new(CountingFailureExplainer {
        explanation: "The driver rejected this request, so the run stopped.".to_string(),
        calls: Arc::clone(&calls),
    }));

    for _ in 0..2 {
        let events = services
            .webui_event_stream()
            .drain(ProjectionSubscriptionRequest {
                actor: actor.clone(),
                scope: scope.clone(),
                after_cursor: None,
            })
            .await
            .unwrap();

        assert!(events.iter().any(|event| match event.payload() {
            ProductOutboundPayload::ProjectionUpdate { state } => {
                state.items.iter().any(|item| {
                    matches!(
                        item,
                        ProductProjectionItem::RunStatus {
                            run_id,
                            failure_summary: Some(summary),
                            ..
                        } if *run_id == turn_run
                            && summary == "The driver rejected this request, so the run stopped."
                    )
                })
            }
            _ => false,
        }));
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn webui_event_stream_projects_recovery_required_failure_summary() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-recovery-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
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
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::RecoveryRequired,
                kind: TurnEventKind::RecoveryRequired,
                blocked_gate: None,
                sanitized_reason: Some("driver_failed".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                status: TurnStatus::RecoveryRequired,
                ..turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1))
            },
        }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus {
                    run_id,
                    status,
                    failure_category: Some(category),
                    failure_summary: Some(summary),
                } if *run_id == turn_run
                    && status == "recovery_required"
                    && category == "driver_failed"
                    && summary == "The run failed because the execution driver reported an error."
            )
        }),
        _ => false,
    }));
}

#[tokio::test]
async fn failure_details_returns_fallback_when_coordinator_errors() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-coordinator-fallback-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let gateway = Arc::new(RecordingFailureGateway {
        response: Mutex::new(Ok(SystemInferenceResponse {
            task_id: SystemInferenceTaskId::new(),
            output_text: "This should not be used.".to_string(),
            elapsed_ms: 1,
        })),
        requests: Mutex::new(Vec::new()),
    });
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::Failed,
                kind: TurnEventKind::Failed,
                blocked_gate: None,
                sanitized_reason: Some("driver_unavailable".to_string()),
            }],
        }),
        Arc::new(FailingTurnCoordinator),
    )
    .with_failure_explainer(Arc::new(ModelFailureExplanationProvider::new(
        gateway.clone(),
    )));

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus {
                    run_id,
                    failure_summary: Some(summary),
                    ..
                } if *run_id == turn_run
                    && summary == "The run failed because the execution driver was temporarily unavailable."
            )
        }),
        _ => false,
    }));
    assert_eq!(gateway.requests.lock().await.len(), 0);
}

#[tokio::test]
async fn failure_details_returns_fallback_when_model_gateway_times_out() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-timeout-fallback-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
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
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::Failed,
                kind: TurnEventKind::Failed,
                blocked_gate: None,
                sanitized_reason: Some("driver_panic".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                status: TurnStatus::Failed,
                ..turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1))
            },
        }),
    )
    .with_failure_explainer(Arc::new(ModelFailureExplanationProvider::new(Arc::new(
        SlowSystemInference,
    ))));

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus {
                    run_id,
                    failure_summary: Some(summary),
                    ..
                } if *run_id == turn_run
                    && summary == "The run failed because the execution driver stopped unexpectedly."
            )
        }),
        _ => false,
    }));
}

#[test]
fn bounded_failure_explanation_truncates_at_utf8_boundary() {
    let input = "é".repeat(300);
    let output = bounded_failure_explanation(&input).expect("bounded explanation");

    assert!(output.len() <= 512);
    assert!(output.is_char_boundary(output.len()));
    assert!(output.chars().all(|character| character == 'é'));
}

#[tokio::test]
async fn model_failure_explainer_returns_bounded_assistant_reply() {
    let gateway = Arc::new(RecordingFailureGateway {
        response: Mutex::new(Ok(SystemInferenceResponse {
            task_id: SystemInferenceTaskId::new(),
            output_text: "The request used an unsupported driver operation, so the run stopped."
                .to_string(),
            elapsed_ms: 1,
        })),
        requests: Mutex::new(Vec::new()),
    });
    let explainer = ModelFailureExplanationProvider::new(gateway.clone());
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-provider-thread").unwrap();
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let run_id = TurnRunId::new();
    let state = turn_run_state(&scope, &user_id, run_id, TurnEventCursor(1));

    let explanation = explainer
        .explain_failure(FailureExplanationInput {
            state,
            failure_category: "driver_invalid_request".to_string(),
            fallback_summary: "The run failed because the execution driver rejected the request."
                .to_string(),
        })
        .await;

    assert_eq!(
        explanation.as_deref(),
        Some("The request used an unsupported driver operation, so the run stopped.")
    );
    let requests = gateway.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].input_text.contains("failure_category"));
    assert_eq!(
        requests[0].identity.task_kind,
        SystemTaskKind::FailureExplanation
    );
}

#[tokio::test]
async fn model_failure_explainer_returns_none_when_gateway_fails() {
    let gateway = Arc::new(RecordingFailureGateway {
        response: Mutex::new(Err(SystemInferenceError::Failed {
            safe_summary: LoopSafeSummary::new("model unavailable").unwrap(),
        })),
        requests: Mutex::new(Vec::new()),
    });
    let explainer = ModelFailureExplanationProvider::new(gateway);
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-provider-fallback-thread").unwrap();
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let run_id = TurnRunId::new();
    let state = turn_run_state(&scope, &user_id, run_id, TurnEventCursor(1));

    let explanation = explainer
        .explain_failure(FailureExplanationInput {
            state,
            failure_category: "driver_unavailable".to_string(),
            fallback_summary: "The run failed because the execution driver was unavailable."
                .to_string(),
        })
        .await;

    assert_eq!(explanation, None);
}

#[tokio::test]
async fn webui_event_stream_tolerates_initial_turn_event_rebase() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let runtime_run = InvocationId::new();
    let turn_run = TurnRunId::new();
    let turn_cursor = TurnEventCursor(7);
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
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(RebaseTurnEventSource {
            cursor: turn_cursor,
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, turn_cursor),
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

    assert!(contains_run_status(&events, runtime_run, "running"));
    assert!(matches!(
        events.last().map(|event| event.payload()),
        Some(ProductOutboundPayload::KeepAlive)
    ));
    let parsed =
        parse_webui_projection_cursor(events.last().unwrap().projection_cursor().as_str()).unwrap();
    assert_eq!(
        parsed.turn,
        Some(TurnEventProjectionCursor::for_scope(scope, turn_cursor))
    );
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
        runtime_item: None,
        turn: Some(TurnEventProjectionCursor::for_scope(
            scope_a,
            TurnEventCursor(10),
        )),
        runtime_payloads_delivered: 0,
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
async fn webui_event_stream_rejects_foreign_composite_runtime_cursor() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_a = ThreadId::new("webui-events-thread-a").unwrap();
    let thread_b = ThreadId::new("webui-events-thread-b").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id);
    let scope_a = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_a.clone(),
    );
    let scope_b = TurnScope::new(tenant_id, Some(agent_id), None, thread_b);
    let cursor = product_cursor_from_webui_cursor(&WebuiProjectionCursor {
        runtime: Some(EventProjectionCursor::origin_for_scope(
            runtime_projection_scope(&actor, &scope_a),
        )),
        runtime_item: None,
        turn: None,
        runtime_payloads_delivered: 1,
    })
    .unwrap();
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    );

    let error = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
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
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id,
                status: TurnStatus::Running,
                kind: TurnEventKind::RunnerHeartbeat,
                blocked_gate: None,
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
            occurred_at: Some(chrono::Utc::now()),
            owner_user_id: Some(user_id.clone()),
            run_id,
            status: TurnStatus::Running,
            kind: TurnEventKind::RunnerHeartbeat,
            blocked_gate: None,
            sanitized_reason: None,
        })
        .collect::<Vec<_>>();
    events.push(TurnLifecycleEvent {
        cursor: TurnEventCursor(WEBUI_TURN_EVENT_PAGE_LIMIT as u64 + 1),
        scope: scope.clone(),
        occurred_at: Some(chrono::Utc::now()),
        owner_user_id: Some(user_id.clone()),
        run_id,
        status: TurnStatus::BlockedAuth,
        kind: TurnEventKind::Blocked,
        blocked_gate: Some(TurnBlockedGateMetadata {
            gate_ref: GateRef::new("gate:auth-required").unwrap(),
            gate_kind: TurnBlockedGateKind::Auth,
        }),
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
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id,
                status: TurnStatus::BlockedAuth,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: GateRef::new("gate:auth-required").unwrap(),
                    gate_kind: TurnBlockedGateKind::Auth,
                }),
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
async fn webui_event_stream_filters_turn_events_by_owner_user() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let owner_user_id = UserId::new("webui-events-owner").unwrap();
    let other_user_id = UserId::new("webui-events-other").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
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
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(owner_user_id.clone()),
                run_id,
                status: TurnStatus::Running,
                kind: TurnEventKind::RunnerClaimed,
                blocked_gate: None,
                sanitized_reason: None,
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &owner_user_id, run_id, TurnEventCursor(1)),
        }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(other_user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(
        events
            .iter()
            .all(|event| matches!(event.payload(), ProductOutboundPayload::KeepAlive)),
        "turn event bridge must not emit another user's lifecycle event payload"
    );
}

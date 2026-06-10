use super::*;

#[tokio::test]
async fn webui_event_stream_projects_failed_run_failure_summary() {
    assert_failed_run_status_summary(
        "webui-events-failed-thread",
        "lease_expired",
        "The run failed because its runner lease expired.",
    )
    .await;
}

#[tokio::test]
async fn webui_event_stream_projects_no_progress_failure_summary() {
    assert_failed_run_status_summary(
        "webui-events-no-progress-thread",
        "no_progress_detected",
        "The run stopped because it repeated the same step without making progress.",
    )
    .await;
}

#[tokio::test]
async fn webui_event_stream_projects_iteration_limit_failure_summary() {
    assert_failed_run_status_summary(
        "webui-events-iteration-limit-thread",
        "iteration_limit",
        "The run stopped after reaching its iteration limit before producing a reply.",
    )
    .await;
}

async fn assert_failed_run_status_summary(
    thread_id: &str,
    failure_category: &str,
    expected_summary: &str,
) {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new(thread_id).unwrap();
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
                sanitized_reason: Some(failure_category.to_string()),
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
                    && category.category() == failure_category
                    && summary == expected_summary
            )
        }),
        _ => false,
    }));
}

#[tokio::test]
async fn webui_event_stream_projects_model_credit_exhaustion_failure_summary() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-credit-failed-thread").unwrap();
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
                sanitized_reason: Some("model_credits_exhausted".to_string()),
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
                    && category.category() == "model_credits_exhausted"
                    && summary
                        == "The AI provider account is out of credits. Add credits or switch providers and try again."
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
                    && category.category() == "driver_invalid_request"
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
                    && category.category() == "driver_failed"
                    && summary == "The run failed because the execution driver reported an error."
            )
        }),
        _ => false,
    }));
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

#[test]
fn bounded_failure_explanation_returns_none_for_empty_or_whitespace_input() {
    assert_eq!(bounded_failure_explanation(""), None);
    assert_eq!(bounded_failure_explanation("   \n\t"), None);
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

    let explanation = explainer
        .explain_failure(FailureExplanationInput {
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

    let explanation = explainer
        .explain_failure(FailureExplanationInput {
            failure_category: "driver_unavailable".to_string(),
            fallback_summary: "The run failed because the execution driver was unavailable."
                .to_string(),
        })
        .await;

    assert_eq!(explanation, None);
}

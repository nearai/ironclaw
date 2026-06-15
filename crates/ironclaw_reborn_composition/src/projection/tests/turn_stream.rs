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
                    credential_requirements: Vec::new(),
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
async fn webui_event_stream_offers_always_for_typed_approval_gate() {
    let tenant_id = TenantId::new("webui-events-approval-tenant").unwrap();
    let user_id = UserId::new("webui-events-approval-user").unwrap();
    let agent_id = AgentId::new("webui-events-approval-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-approval-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let approval_request_id = ApprovalRequestId::new();
    let gate_ref = GateRef::new(format!("gate:approval-{approval_request_id}")).unwrap();
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let capability = CapabilityId::new("builtin.http").unwrap();
    approval_requests
        .save_pending(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                InvocationId::new(),
            ),
            ApprovalRequest {
                id: approval_request_id,
                correlation_id: CorrelationId::new(),
                requested_by: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                action: Box::new(Action::Dispatch {
                    capability: capability.clone(),
                    estimated_resources: ResourceEstimate {
                        network_egress_bytes: Some(4096),
                        ..ResourceEstimate::default()
                    },
                }),
                invocation_fingerprint: None,
                reason: "raw path /Users/firatsertgoz/.ssh/id_rsa and token sk-secret".to_string(),
                reusable_scope: None,
            },
        )
        .await
        .unwrap();
    let approval_requests_dyn: Arc<dyn ApprovalRequestStore> = approval_requests;
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-approval-reply").unwrap(),
    )
    .with_approval_requests(approval_requests_dyn)
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedApproval,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: gate_ref.clone(),
                    gate_kind: TurnBlockedGateKind::Approval,
                    credential_requirements: Vec::new(),
                }),
                sanitized_reason: Some("capability requires approval".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                status: TurnStatus::BlockedApproval,
                gate_ref: Some(gate_ref.clone()),
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

    let prompt = events
        .iter()
        .find_map(|event| match event.payload() {
            ProductOutboundPayload::GatePrompt(prompt) => Some(prompt),
            _ => None,
        })
        .expect("approval gate prompt");

    assert_eq!(prompt.turn_run_id, turn_run);
    assert_eq!(prompt.gate_ref, gate_ref.as_str());
    assert_eq!(prompt.headline, "Approval required");
    assert_eq!(prompt.body, "capability requires approval");
    assert!(prompt.allow_always);
    let context = prompt.approval_context.as_ref().expect("approval context");
    assert_eq!(context.tool_name, "builtin.http");
    assert_eq!(context.action.label, "Run tool");
    assert_eq!(
        context.reason.as_deref(),
        Some("raw path /Users/firatsertgoz/.ssh/id_rsa and token sk-secret")
    );
    assert_eq!(context.scope.label, "This request only");
    assert!(context.details.iter().any(|detail| {
        detail.label == "Estimated network egress" && detail.value == "4096 bytes"
    }));
}

#[tokio::test]
async fn webui_event_stream_projects_network_approval_context() {
    let tenant_id = TenantId::new("webui-events-approval-actions-tenant").unwrap();
    let user_id = UserId::new("webui-events-approval-actions-user").unwrap();
    let agent_id = AgentId::new("webui-events-approval-actions-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-approval-actions-thread").unwrap();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let network_run = TurnRunId::new();
    let network_request_id = ApprovalRequestId::new();
    let network_gate_ref = GateRef::new(format!("gate:approval-{network_request_id}")).unwrap();
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let approval_scope = resource_scope(
        &tenant_id,
        &user_id,
        &agent_id,
        &thread_id,
        InvocationId::new(),
    );

    approval_requests
        .save_pending(
            approval_scope,
            ApprovalRequest {
                id: network_request_id,
                correlation_id: CorrelationId::new(),
                requested_by: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                action: Box::new(Action::Network {
                    target: NetworkTarget {
                        scheme: NetworkScheme::Https,
                        host: "example.com".to_string(),
                        port: Some(443),
                    },
                    method: NetworkMethod::Post,
                    estimated_bytes: Some(8192),
                }),
                invocation_fingerprint: None,
                reason: "raw network reason".to_string(),
                reusable_scope: None,
            },
        )
        .await
        .unwrap();

    let approval_requests_dyn: Arc<dyn ApprovalRequestStore> = approval_requests;
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-approval-actions-reply").unwrap(),
    )
    .with_approval_requests(approval_requests_dyn)
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: network_run,
                status: TurnStatus::BlockedApproval,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: network_gate_ref.clone(),
                    gate_kind: TurnBlockedGateKind::Approval,
                    credential_requirements: Vec::new(),
                }),
                sanitized_reason: Some("network requires approval".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                status: TurnStatus::BlockedApproval,
                gate_ref: Some(network_gate_ref.clone()),
                ..turn_run_state(&scope, &user_id, network_run, TurnEventCursor(1))
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
    let prompts = events
        .iter()
        .filter_map(|event| match event.payload() {
            ProductOutboundPayload::GatePrompt(prompt) => Some(prompt),
            _ => None,
        })
        .collect::<Vec<_>>();

    let network_context = prompts
        .iter()
        .find(|prompt| prompt.gate_ref == network_gate_ref.as_str())
        .and_then(|prompt| prompt.approval_context.as_ref())
        .expect("network approval context");
    assert_eq!(network_context.tool_name, "builtin.http");
    assert_eq!(network_context.action.label, "Network request");
    assert_eq!(network_context.action.method, Some(NetworkMethod::Post));
    let destination = network_context
        .destination
        .as_ref()
        .expect("network destination");
    assert_eq!(destination.label, "POST https://example.com:443");
    assert_eq!(destination.domain.as_deref(), Some("example.com"));
    assert!(
        network_context
            .details
            .iter()
            .any(|detail| { detail.label == "Estimated transfer" && detail.value == "8192 bytes" })
    );
}

#[tokio::test]
async fn webui_event_stream_projects_spawn_approval_context() {
    let tenant_id = TenantId::new("webui-events-approval-spawn-tenant").unwrap();
    let user_id = UserId::new("webui-events-approval-spawn-user").unwrap();
    let agent_id = AgentId::new("webui-events-approval-spawn-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-approval-spawn-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let approval_request_id = ApprovalRequestId::new();
    let gate_ref = GateRef::new(format!("gate:approval-{approval_request_id}")).unwrap();
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    approval_requests
        .save_pending(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                InvocationId::new(),
            ),
            ApprovalRequest {
                id: approval_request_id,
                correlation_id: CorrelationId::new(),
                requested_by: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                action: Box::new(Action::SpawnCapability {
                    capability: CapabilityId::new("script.shell").unwrap(),
                    estimated_resources: ResourceEstimate {
                        process_count: Some(2),
                        ..ResourceEstimate::default()
                    },
                }),
                invocation_fingerprint: None,
                reason: "raw spawn reason".to_string(),
                reusable_scope: None,
            },
        )
        .await
        .unwrap();
    let approval_requests_dyn: Arc<dyn ApprovalRequestStore> = approval_requests;
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-approval-spawn-reply").unwrap(),
    )
    .with_approval_requests(approval_requests_dyn)
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedApproval,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: gate_ref.clone(),
                    gate_kind: TurnBlockedGateKind::Approval,
                    credential_requirements: Vec::new(),
                }),
                sanitized_reason: Some("spawn requires approval".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                status: TurnStatus::BlockedApproval,
                gate_ref: Some(gate_ref.clone()),
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

    let context = events
        .iter()
        .find_map(|event| match event.payload() {
            ProductOutboundPayload::GatePrompt(prompt) => prompt.approval_context.as_ref(),
            _ => None,
        })
        .expect("spawn approval context");
    assert_eq!(context.tool_name, "script.shell");
    assert_eq!(context.action.label, "Start tool");
    assert!(
        context
            .details
            .iter()
            .any(|detail| { detail.label == "Processes" && detail.value == "2" })
    );
}

#[tokio::test]
async fn webui_event_stream_keeps_approval_prompt_when_request_lookup_fails() {
    let tenant_id = TenantId::new("webui-events-approval-fallback-tenant").unwrap();
    let user_id = UserId::new("webui-events-approval-fallback-user").unwrap();
    let agent_id = AgentId::new("webui-events-approval-fallback-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-approval-fallback-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let approval_request_id = ApprovalRequestId::new();
    let gate_ref = GateRef::new(format!("gate:approval-{approval_request_id}")).unwrap();
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-approval-fallback-reply").unwrap(),
    )
    .with_approval_requests(Arc::new(FailingApprovalRequestStore))
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedApproval,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: gate_ref.clone(),
                    gate_kind: TurnBlockedGateKind::Approval,
                    credential_requirements: Vec::new(),
                }),
                sanitized_reason: Some("capability requires approval".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                status: TurnStatus::BlockedApproval,
                gate_ref: Some(gate_ref.clone()),
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

    let prompt = events
        .iter()
        .find_map(|event| match event.payload() {
            ProductOutboundPayload::GatePrompt(prompt) => Some(prompt),
            _ => None,
        })
        .expect("approval gate prompt");

    assert_eq!(prompt.gate_ref, gate_ref.as_str());
    assert!(prompt.allow_always);
    assert!(prompt.approval_context.is_none());
}

#[tokio::test]
async fn webui_event_stream_does_not_offer_always_for_generic_approval_gate() {
    let tenant_id = TenantId::new("webui-events-generic-approval-tenant").unwrap();
    let user_id = UserId::new("webui-events-generic-approval-user").unwrap();
    let agent_id = AgentId::new("webui-events-generic-approval-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-generic-approval-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let gate_ref = GateRef::new("gate:generic-approval").unwrap();
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-generic-approval-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedApproval,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: gate_ref.clone(),
                    gate_kind: TurnBlockedGateKind::Approval,
                    credential_requirements: Vec::new(),
                }),
                sanitized_reason: Some("generic approval required".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                status: TurnStatus::BlockedApproval,
                gate_ref: Some(gate_ref.clone()),
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

    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::GatePrompt(prompt)
                if prompt.turn_run_id == turn_run
                    && prompt.gate_ref == gate_ref.as_str()
                    && prompt.headline == "Approval required"
                    && prompt.body == "generic approval required"
                    && !prompt.allow_always
        )
    }));
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
                    credential_requirements: Vec::new(),
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
        live: None,
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
        live: None,
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
            credential_requirements: Vec::new(),
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
                    credential_requirements: Vec::new(),
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

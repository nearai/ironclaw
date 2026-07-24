use super::*;

fn contains_any_run_status(
    events: &[ProductOutboundEnvelope],
    invocation_id: InvocationId,
) -> bool {
    let expected_run_id = TurnRunId::from_uuid(invocation_id.as_uuid());
    events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionSnapshot { state }
        | ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus { run_id, .. } if *run_id == expected_run_id
            )
        }),
        _ => false,
    })
}

async fn append_nested_dispatch_failure(
    event_log: &InMemoryDurableEventLog,
    child_scope: ResourceScope,
    parent_invocation_id: InvocationId,
    capability: &CapabilityId,
) {
    let provider = ExtensionId::new("script").unwrap();
    for mut event in [
        RuntimeEvent::dispatch_requested(child_scope.clone(), capability.clone()),
        RuntimeEvent::runtime_selected(
            child_scope.clone(),
            capability.clone(),
            provider.clone(),
            RuntimeKind::Script,
        ),
        RuntimeEvent::dispatch_failed(
            child_scope,
            capability.clone(),
            Some(provider),
            Some(RuntimeKind::Script),
            "exit_failure",
        ),
    ] {
        event.parent_invocation_id = Some(parent_invocation_id);
        event_log.append(event).await.unwrap();
    }
}

fn contains_failed_nested_activity(
    events: &[ProductOutboundEnvelope],
    child_invocation_id: InvocationId,
    parent_invocation_id: InvocationId,
    capability: &CapabilityId,
) -> bool {
    events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == child_invocation_id
                    && activity.status == CapabilityActivityStatusView::Failed
                    && activity.turn_run_id
                        == Some(TurnRunId::from_uuid(parent_invocation_id.as_uuid()))
                    && activity.capability_id == *capability
        )
    })
}

#[tokio::test]
async fn product_event_stream_snapshot_keeps_nested_dispatch_failure_out_of_run_status() {
    let tenant_id = TenantId::new("webui-nested-dispatch-tenant").unwrap();
    let user_id = UserId::new("webui-nested-dispatch-user").unwrap();
    let agent_id = AgentId::new("webui-nested-dispatch-agent").unwrap();
    let thread_id = ThreadId::new("webui-nested-dispatch-thread").unwrap();
    let parent_invocation_id = InvocationId::new();
    let child_invocation_id = InvocationId::new();
    let parent_scope = resource_scope(
        &tenant_id,
        &user_id,
        &agent_id,
        &thread_id,
        parent_invocation_id,
    );
    let child_scope = resource_scope(
        &tenant_id,
        &user_id,
        &agent_id,
        &thread_id,
        child_invocation_id,
    );
    let capability = CapabilityId::new("script.nested").unwrap();
    let event_log = Arc::new(InMemoryDurableEventLog::new());

    event_log
        .append(RuntimeEvent::model_started(
            parent_scope.clone(),
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();
    append_nested_dispatch_failure(
        event_log.as_ref(),
        child_scope,
        parent_invocation_id,
        &capability,
    )
    .await;
    event_log
        .append(RuntimeEvent::loop_completed(
            parent_scope,
            CapabilityId::new("loop.run").unwrap(),
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-nested-dispatch-reply").unwrap(),
    );
    let events = services
        .product_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(contains_run_status(
        &events,
        parent_invocation_id,
        "completed"
    ));
    assert!(!contains_any_run_status(&events, child_invocation_id));
    assert!(contains_failed_nested_activity(
        &events,
        child_invocation_id,
        parent_invocation_id,
        &capability,
    ));
}

#[tokio::test]
async fn product_event_stream_cursor_resume_keeps_late_nested_failure_out_of_run_status() {
    let tenant_id = TenantId::new("webui-nested-resume-tenant").unwrap();
    let user_id = UserId::new("webui-nested-resume-user").unwrap();
    let agent_id = AgentId::new("webui-nested-resume-agent").unwrap();
    let thread_id = ThreadId::new("webui-nested-resume-thread").unwrap();
    let parent_invocation_id = InvocationId::new();
    let child_invocation_id = InvocationId::new();
    let parent_scope = resource_scope(
        &tenant_id,
        &user_id,
        &agent_id,
        &thread_id,
        parent_invocation_id,
    );
    let child_scope = resource_scope(
        &tenant_id,
        &user_id,
        &agent_id,
        &thread_id,
        child_invocation_id,
    );
    let capability = CapabilityId::new("script.nested").unwrap();
    let event_log = Arc::new(InMemoryDurableEventLog::new());

    event_log
        .append(RuntimeEvent::model_started(
            parent_scope.clone(),
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();

    let event_log_dyn: Arc<dyn DurableEventLog> = event_log.clone();
    let actor = TurnActor::new(user_id.clone());
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-nested-resume-reply").unwrap(),
    );
    let first = services
        .product_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();
    let cursor = first
        .last()
        .expect("initial snapshot should carry the parent run")
        .projection_cursor()
        .clone();

    append_nested_dispatch_failure(
        event_log.as_ref(),
        child_scope,
        parent_invocation_id,
        &capability,
    )
    .await;
    event_log
        .append(RuntimeEvent::loop_completed(
            parent_scope,
            CapabilityId::new("loop.run").unwrap(),
        ))
        .await
        .unwrap();

    let resumed = services
        .product_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(cursor),
        })
        .await
        .unwrap();

    assert!(
        resumed.iter().any(|event| matches!(
            event.payload(),
            ProductOutboundPayload::ProjectionUpdate { .. }
        )),
        "cursor resume should emit at least one incremental projection update"
    );
    assert!(
        resumed.iter().all(|event| !matches!(
            event.payload(),
            ProductOutboundPayload::ProjectionSnapshot { .. }
        )),
        "a valid cursor must not fall back to a projection snapshot"
    );
    assert!(contains_run_status(
        &resumed,
        parent_invocation_id,
        "completed"
    ));
    assert!(!contains_any_run_status(&resumed, child_invocation_id));
    assert!(contains_failed_nested_activity(
        &resumed,
        child_invocation_id,
        parent_invocation_id,
        &capability,
    ));
}

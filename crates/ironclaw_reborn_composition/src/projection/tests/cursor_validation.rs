use super::*;

#[test]
fn effective_runtime_payload_offset_resets_when_runtime_item_changes() {
    let delivered =
        effective_runtime_payload_offset(3, Some(EventCursor::new(7)), EventCursor::new(8));

    assert_eq!(delivered, 0);
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

#[tokio::test]
async fn webui_event_stream_rejects_runtime_delivery_offset_above_payload_limit() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id);
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let cursor = product_cursor_from_webui_cursor(&WebuiProjectionCursor {
        runtime: Some(EventProjectionCursor::origin_for_scope(
            runtime_projection_scope(&actor, &scope),
        )),
        runtime_item: None,
        turn: None,
        live_projection: None,
        runtime_payloads_delivered: WEBUI_RUNTIME_ITEM_MAX_PAYLOADS + 2,
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
            scope,
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
async fn webui_event_stream_rejects_runtime_delivery_offset_above_item_payload_count() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let invocation_id = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            CapabilityId::new("script.echo").unwrap(),
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let cursor = product_cursor_from_webui_cursor(&WebuiProjectionCursor {
        runtime: Some(EventProjectionCursor::origin_for_scope(
            runtime_projection_scope(&actor, &scope),
        )),
        runtime_item: None,
        turn: None,
        live_projection: None,
        runtime_payloads_delivered: 3,
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
            scope,
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
async fn webui_event_stream_rejects_foreign_composite_live_cursor() {
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
        turn: None,
        live_projection: Some(WebuiLiveProjectionCursor {
            scope: scope_a,
            sequence: 10,
        }),
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
async fn webui_event_stream_rejects_evicted_live_cursor() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    );
    for index in 0..=WebuiLiveProjectionStore::MAX_ENTRIES {
        services
            .live_projections
            .publish_reasoning_delta(
                scope.clone(),
                TurnRunId::new(),
                &format!("reasoning delta {index}"),
            )
            .expect("publish live reasoning");
    }
    let cursor = product_cursor_from_webui_cursor(&WebuiProjectionCursor {
        runtime: None,
        runtime_item: None,
        turn: None,
        live_projection: Some(WebuiLiveProjectionCursor {
            scope: scope.clone(),
            sequence: 1,
        }),
        runtime_payloads_delivered: 0,
    })
    .unwrap();

    let error = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: Some(cursor),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ProductAdapterError::WorkflowRejected {
            retryable: true,
            ..
        }
    ));
}

#[tokio::test]
async fn webui_event_stream_rejects_legacy_partial_snapshot_offset_above_item_payload_count() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let invocation_id = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            CapabilityId::new("script.echo").unwrap(),
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let cursor = product_cursor_from_webui_cursor(&WebuiProjectionCursor {
        runtime: None,
        runtime_item: None,
        turn: None,
        live_projection: None,
        runtime_payloads_delivered: 3,
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
            scope,
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

use super::*;

fn preview_input_ref(label: &str) -> CapabilityInputRef {
    CapabilityInputRef::new(format!("input:{label}")).unwrap()
}

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
async fn webui_event_stream_drains_capability_activity_from_projection() {
    let tenant_id = TenantId::new("webui-activity-tenant").unwrap();
    let user_id = UserId::new("webui-activity-user").unwrap();
    let agent_id = AgentId::new("webui-activity-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-thread").unwrap();
    let invocation_id = InvocationId::new();
    let capability = CapabilityId::new("script.echo").unwrap();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            capability.clone(),
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-reply").unwrap(),
    );
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id.clone()),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == invocation_id
                    && activity.thread_id.as_ref() == Some(&thread_id)
                    && activity.capability_id == capability
                    && activity.status == CapabilityActivityStatusView::Started
        )
    }));
}

#[tokio::test]
async fn webui_event_stream_enriches_activity_with_display_preview_from_store() {
    let tenant_id = TenantId::new("webui-preview-tenant").unwrap();
    let user_id = UserId::new("webui-preview-user").unwrap();
    let agent_id = AgentId::new("webui-preview-agent").unwrap();
    let thread_id = ThreadId::new("webui-preview-thread").unwrap();
    let invocation_id = InvocationId::new();
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new("builtin.read_file").unwrap();
    let input_ref = preview_input_ref("webui-preview-input");
    let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
    display_previews.record_input(
        &run_id.to_string(),
        &input_ref,
        "read_file",
        &serde_json::json!({
            "path": "src/main.rs",
            "token": "sk-secret",
            "max_bytes": 4096
        }),
    );
    display_previews.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability,
        result_ref: "result:preview-output",
        output: &serde_json::json!({"content": "fn main() {}"}),
        output_bytes: 64,
    });
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_succeeded(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            capability.clone(),
            ExtensionId::new("builtin").unwrap(),
            RuntimeKind::FirstParty,
            64,
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-preview-reply").unwrap(),
    )
    .with_display_previews(Arc::clone(&display_previews));
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id.clone()),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(
        events.iter().any(|event| {
            matches!(
                event.payload(),
                ProductOutboundPayload::CapabilityDisplayPreview(preview)
                    if preview.invocation_id == invocation_id
                        && preview.thread_id.as_ref() == Some(&thread_id)
                        && preview.capability_id == capability
                        && preview.title == "read_file"
                        && preview.subtitle.as_deref() == Some("src/main.rs")
                        && preview.input_summary.as_deref().is_some_and(|summary| summary.contains("path: src/main.rs"))
                        && preview.output_preview.as_deref() == Some("fn main() {}")
                        && preview.result_ref.as_deref() == Some("result:preview-output")
                        && preview.output_bytes == Some(64)
            )
        }),
        "events: {events:#?}"
    );
    let rendered = serde_json::to_string(&events).unwrap();
    assert!(!rendered.contains("sk-secret"));
}

#[tokio::test]
async fn capability_display_preview_store_redacts_unsafe_paths_and_secrets() {
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new("builtin.read_file").unwrap();
    let input_ref = preview_input_ref("redacted-preview-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_input(
        &run_id.to_string(),
        &input_ref,
        "read_file",
        &serde_json::json!({
            "path": "/Users/alice/secret.rs",
            "api_key": "sk-secret"
        }),
    );
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
        capability_id: &capability,
        result_ref: "result:redacted-preview",
        output: &serde_json::json!({"content": "{\"path\":\"/etc/passwd\", unc:\"\\\\host\\\\share\", token:\"sk-secret\"}"}),
        output_bytes: 42,
    });
    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(42),
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert!(preview.subtitle.is_none());
    let rendered = serde_json::to_string(&preview).unwrap();
    assert!(!rendered.contains("sk-secret"));
    assert!(!rendered.contains("/Users/alice"));
    assert!(!rendered.contains("/etc/passwd"));
    assert!(!rendered.contains("\\\\host\\\\share"));
    assert!(rendered.contains("[redacted]"));
}

#[tokio::test]
async fn capability_display_preview_store_keys_completed_results_by_invocation() {
    let run_id = TurnRunId::new();
    let first_invocation = InvocationId::new();
    let second_invocation = InvocationId::new();
    let first_capability = CapabilityId::new("script.first").unwrap();
    let second_capability = CapabilityId::new("script.second").unwrap();
    let first_input = preview_input_ref("first-preview-input");
    let second_input = preview_input_ref("second-preview-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_input(
        &run_id.to_string(),
        &first_input,
        "first",
        &serde_json::json!({"path": "src/first.rs"}),
    );
    store.record_input(
        &run_id.to_string(),
        &second_input,
        "second",
        &serde_json::json!({"path": "src/second.rs"}),
    );
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &first_input,
        invocation_id: first_invocation,
        capability_id: &first_capability,
        result_ref: "result:first",
        output: &serde_json::json!({"content": "first output"}),
        output_bytes: 12,
    });
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &second_input,
        invocation_id: second_invocation,
        capability_id: &second_capability,
        result_ref: "result:second",
        output: &serde_json::json!({"content": "second output"}),
        output_bytes: 13,
    });

    let first_preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: first_invocation,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: first_capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(12),
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();
    let second_preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: second_invocation,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: second_capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(13),
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(2),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(first_preview.result_ref.as_deref(), Some("result:first"));
    assert_eq!(
        first_preview.output_preview.as_deref(),
        Some("first output")
    );
    assert_eq!(second_preview.result_ref.as_deref(), Some("result:second"));
    assert_eq!(
        second_preview.output_preview.as_deref(),
        Some("second output")
    );
}

#[tokio::test]
async fn capability_display_preview_store_pairs_inputs_by_ref_when_results_complete_out_of_order() {
    let run_id = TurnRunId::new();
    let first_invocation = InvocationId::new();
    let second_invocation = InvocationId::new();
    let first_capability = CapabilityId::new("script.first").unwrap();
    let second_capability = CapabilityId::new("script.second").unwrap();
    let first_input = preview_input_ref("first-out-of-order-input");
    let second_input = preview_input_ref("second-out-of-order-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_input(
        &run_id.to_string(),
        &first_input,
        "first",
        &serde_json::json!({"path": "src/first.rs"}),
    );
    store.record_input(
        &run_id.to_string(),
        &second_input,
        "second",
        &serde_json::json!({"path": "src/second.rs"}),
    );
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &second_input,
        invocation_id: second_invocation,
        capability_id: &second_capability,
        result_ref: "result:second",
        output: &serde_json::json!({"content": "second output"}),
        output_bytes: 13,
    });
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &first_input,
        invocation_id: first_invocation,
        capability_id: &first_capability,
        result_ref: "result:first",
        output: &serde_json::json!({"content": "first output"}),
        output_bytes: 12,
    });

    let first_preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: first_invocation,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: first_capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(12),
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();
    let second_preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: second_invocation,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: second_capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(13),
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(2),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(first_preview.title, "first");
    assert_eq!(first_preview.subtitle.as_deref(), Some("src/first.rs"));
    assert_eq!(first_preview.result_ref.as_deref(), Some("result:first"));
    assert_eq!(
        first_preview.output_preview.as_deref(),
        Some("first output")
    );
    assert_eq!(second_preview.title, "second");
    assert_eq!(second_preview.subtitle.as_deref(), Some("src/second.rs"));
    assert_eq!(second_preview.result_ref.as_deref(), Some("result:second"));
    assert_eq!(
        second_preview.output_preview.as_deref(),
        Some("second output")
    );
}

#[test]
fn display_preview_sanitizer_does_not_redact_common_sk_substrings() {
    let sanitized = sanitize_text("mask disk risk sk-live");

    assert!(sanitized.contains("mask disk risk"));
    assert!(!sanitized.contains("sk-live"));
    assert!(sanitized.contains("[redacted]"));
}

#[test]
fn display_preview_json_sanitizer_bounds_nested_values() {
    let mut value = serde_json::json!("leaf");
    for _ in 0..(SANITIZE_JSON_MAX_DEPTH + 4) {
        value = serde_json::json!([value]);
    }

    let sanitized = sanitize_json_value(&value);
    let rendered = serde_json::to_string(&sanitized).unwrap();

    assert!(rendered.contains("[truncated]"));
    assert!(!rendered.contains("leaf"));
}

#[tokio::test]
async fn capability_display_preview_marks_json_depth_truncation() {
    let run_id = TurnRunId::new();
    let invocation_id = InvocationId::new();
    let capability = CapabilityId::new("script.deep_json").unwrap();
    let input_ref = preview_input_ref("deep-json-input");
    let store = CapabilityDisplayPreviewStore::default();
    let mut output = serde_json::json!("leaf");
    for _ in 0..(SANITIZE_JSON_MAX_DEPTH + 4) {
        output = serde_json::json!([output]);
    }
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability,
        result_ref: "result:deep-json",
        output: &output,
        output_bytes: 256,
    });

    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(256),
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert!(preview.truncated);
    assert!(
        preview
            .output_preview
            .as_deref()
            .is_some_and(|preview| preview.contains("[truncated]"))
    );
}

#[tokio::test]
async fn capability_display_preview_falls_back_for_failed_tool_without_result() {
    let capability = CapabilityId::new("script.fail").unwrap();
    let store = CapabilityDisplayPreviewStore::default();
    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: InvocationId::new(),
            run_id: None,
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Failed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: Some("operation_failed".to_string()),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(preview.title, "fail");
    assert_eq!(preview.output_kind.as_deref(), Some("text"));
    assert_eq!(preview.result_ref, None);
    assert!(
        preview
            .output_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("operation_failed"))
    );
}

#[tokio::test]
async fn capability_display_preview_store_truncates_long_output_by_lines() {
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new("script.long_output").unwrap();
    let input_ref = preview_input_ref("long-output-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
        capability_id: &capability,
        result_ref: "result:long-preview",
        output: &serde_json::Value::String(
            (0..130)
                .map(|index| format!("line-{index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        output_bytes: 2048,
    });
    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(2048),
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    let output_preview = preview.output_preview.unwrap();
    assert!(preview.truncated);
    assert!(output_preview.lines().count() <= 120);
    assert!(!output_preview.contains("line-129"));
}

#[tokio::test]
async fn webui_event_stream_preserves_sanitized_capability_activity_error_kind() {
    let tenant_id = TenantId::new("webui-activity-redacted-tenant").unwrap();
    let user_id = UserId::new("webui-activity-redacted-user").unwrap();
    let agent_id = AgentId::new("webui-activity-redacted-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-redacted-thread").unwrap();
    let invocation_id = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_failed(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            CapabilityId::new("script.echo").unwrap(),
            Some(ExtensionId::new("script").unwrap()),
            Some(RuntimeKind::Script),
            "raw failure /tmp/private-host-path SECRET_SENTINEL_sk_live",
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-redacted-reply").unwrap(),
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

    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == invocation_id
                    && activity.status == CapabilityActivityStatusView::Failed
                    && activity.error_kind.as_deref() == Some("Unclassified")
        )
    }));
}

#[tokio::test]
async fn webui_event_stream_resumes_inside_multi_payload_runtime_projection_item() {
    let tenant_id = TenantId::new("webui-activity-resume-tenant").unwrap();
    let user_id = UserId::new("webui-activity-resume-user").unwrap();
    let agent_id = AgentId::new("webui-activity-resume-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-resume-thread").unwrap();
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
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-resume-reply").unwrap(),
    );
    let initial_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(initial_events.len(), 2);
    assert!(matches!(
        initial_events[0].payload(),
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
    assert!(matches!(
        initial_events[1].payload(),
        ProductOutboundPayload::CapabilityActivity(_)
    ));
    let partial_cursor =
        parse_webui_projection_cursor(initial_events[0].projection_cursor().as_str()).unwrap();
    assert!(partial_cursor.runtime.is_none());
    assert!(partial_cursor.runtime_item.is_some());
    assert_eq!(partial_cursor.runtime_payloads_delivered, 1);

    let resumed_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(initial_events[0].projection_cursor().clone()),
        })
        .await
        .unwrap();

    assert_eq!(resumed_events.len(), 1);
    assert!(matches!(
        resumed_events[0].payload(),
        ProductOutboundPayload::CapabilityActivity(activity)
            if activity.invocation_id == invocation_id
    ));
    let resumed_cursor =
        parse_webui_projection_cursor(resumed_events[0].projection_cursor().as_str()).unwrap();
    assert!(resumed_cursor.runtime.is_some());
    assert_eq!(resumed_cursor.runtime_payloads_delivered, 0);
}

#[tokio::test]
async fn webui_event_stream_accepts_legacy_partial_origin_cursor() {
    let tenant_id = TenantId::new("webui-activity-legacy-tenant").unwrap();
    let user_id = UserId::new("webui-activity-legacy-user").unwrap();
    let agent_id = AgentId::new("webui-activity-legacy-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-legacy-thread").unwrap();
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
    let legacy_cursor = product_cursor_from_webui_cursor(&WebuiProjectionCursor {
        runtime: None,
        runtime_item: None,
        turn: None,
        runtime_payloads_delivered: 1,
    })
    .unwrap();
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-legacy-reply").unwrap(),
    );

    let resumed_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(legacy_cursor),
        })
        .await
        .unwrap();

    assert_eq!(resumed_events.len(), 1);
    assert!(matches!(
        resumed_events[0].payload(),
        ProductOutboundPayload::CapabilityActivity(activity)
            if activity.invocation_id == invocation_id
    ));
}

#[tokio::test]
async fn webui_projection_snapshot_bounds_activity_fanout_before_payload_mapping() {
    let tenant_id = TenantId::new("webui-activity-cap-tenant").unwrap();
    let user_id = UserId::new("webui-activity-cap-user").unwrap();
    let agent_id = AgentId::new("webui-activity-cap-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-cap-thread").unwrap();
    let capability = CapabilityId::new("script.echo").unwrap();
    let actor = TurnActor::new(user_id);
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id.clone());
    let projection_scope = runtime_projection_scope(&actor, &scope);
    let cursor =
        EventProjectionCursor::for_scope(projection_scope, ironclaw_events::EventCursor::new(1));
    let snapshot = ProjectionSnapshot {
        timeline: ThreadTimeline {
            entries: Vec::new(),
        },
        runs: vec![RunStatusProjection {
            invocation_id: InvocationId::new(),
            capability_id: capability.clone(),
            thread_id: Some(thread_id.clone()),
            status: RunProjectionStatus::Running,
            provider: None,
            runtime: None,
            process_id: None,
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        }],
        capability_activities: (0..(WEBUI_PROJECTION_PAGE_LIMIT + 10))
            .map(|index| CapabilityActivityProjection {
                invocation_id: InvocationId::new(),
                run_id: None,
                capability_id: capability.clone(),
                thread_id: Some(thread_id.clone()),
                status: ironclaw_event_projections::CapabilityActivityStatus::Running,
                provider: None,
                runtime: None,
                process_id: None,
                output_bytes: None,
                error_kind: None,
                last_cursor: ironclaw_events::EventCursor::new(index as u64 + 1),
                updated_at: chrono::Utc::now(),
            })
            .collect(),
        next_cursor: cursor.clone(),
        truncated: false,
    };

    let display_previews = NoopCapabilityDisplayPreviewSource;
    let item = snapshot_payloads(
        &scope,
        &display_previews,
        snapshot,
        cursor.clone(),
        None,
        0,
        WEBUI_PROJECTION_PAGE_LIMIT + 11,
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(item.total, WEBUI_RUNTIME_ITEM_MAX_PAYLOADS);
    assert_eq!(item.payloads.len(), WEBUI_RUNTIME_ITEM_MAX_PAYLOADS);
    assert!(matches!(
        &item.payloads[0],
        ProductOutboundPayload::ProjectionSnapshot { state } if state.items.len() == 1
    ));
    assert_eq!(
        item.payloads
            .iter()
            .filter(|payload| matches!(payload, ProductOutboundPayload::CapabilityActivity(_)))
            .count(),
        WEBUI_PROJECTION_PAGE_LIMIT
    );
}

#[tokio::test]
async fn webui_projection_snapshot_resumes_preview_payload() {
    let tenant_id = TenantId::new("webui-preview-resume-tenant").unwrap();
    let user_id = UserId::new("webui-preview-resume-user").unwrap();
    let agent_id = AgentId::new("webui-preview-resume-agent").unwrap();
    let thread_id = ThreadId::new("webui-preview-resume-thread").unwrap();
    let capability = CapabilityId::new("builtin.read_file").unwrap();
    let invocation_id = InvocationId::new();
    let actor = TurnActor::new(user_id.clone());
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id.clone());
    let projection_scope = runtime_projection_scope(&actor, &scope);
    let cursor =
        EventProjectionCursor::for_scope(projection_scope, ironclaw_events::EventCursor::new(1));
    let display_previews = CapabilityDisplayPreviewStore::default();
    let run_id = TurnRunId::new();
    let input_ref = preview_input_ref("preview-resume-input");
    display_previews.record_input(
        &run_id.to_string(),
        &input_ref,
        "read_file",
        &serde_json::json!({"path": "src/main.rs"}),
    );
    display_previews.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability,
        result_ref: "result:preview-resume",
        output: &serde_json::json!({"content": "fn main() {}"}),
        output_bytes: 12,
    });
    let snapshot = ProjectionSnapshot {
        timeline: ThreadTimeline {
            entries: Vec::new(),
        },
        runs: vec![RunStatusProjection {
            invocation_id,
            capability_id: capability.clone(),
            thread_id: Some(thread_id.clone()),
            status: RunProjectionStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        }],
        capability_activities: vec![CapabilityActivityProjection {
            invocation_id,
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(thread_id),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(12),
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        }],
        next_cursor: cursor.clone(),
        truncated: false,
    };

    let first = snapshot_payloads(
        &scope,
        &display_previews,
        snapshot.clone(),
        cursor.clone(),
        None,
        0,
        2,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first.total, 3);
    assert_eq!(first.payloads.len(), 2);
    assert!(matches!(
        first.payloads[0],
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
    assert!(matches!(
        first.payloads[1],
        ProductOutboundPayload::CapabilityActivity(_)
    ));

    let resumed = snapshot_payloads(
        &scope,
        &display_previews,
        snapshot,
        cursor,
        Some(first.item_cursor.runtime),
        2,
        2,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resumed.payloads.len(), 1);
    assert!(matches!(
        &resumed.payloads[0],
        ProductOutboundPayload::CapabilityDisplayPreview(preview)
            if preview.result_ref.as_deref() == Some("result:preview-resume")
    ));
}

#[tokio::test]
async fn webui_event_stream_bounds_large_activity_history_before_dto_construction() {
    let tenant_id = TenantId::new("webui-activity-overflow-tenant").unwrap();
    let user_id = UserId::new("webui-activity-overflow-user").unwrap();
    let agent_id = AgentId::new("webui-activity-overflow-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-overflow-thread").unwrap();
    let capability = CapabilityId::new("script.echo").unwrap();
    let activity_count = WEBUI_RUNTIME_ITEM_MAX_PAYLOADS + 3;
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    for _ in 0..activity_count {
        event_log
            .append(RuntimeEvent::dispatch_requested(
                resource_scope(
                    &tenant_id,
                    &user_id,
                    &agent_id,
                    &thread_id,
                    InvocationId::new(),
                ),
                capability.clone(),
            ))
            .await
            .unwrap();
    }

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-overflow-reply").unwrap(),
    );
    let initial_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(initial_events.len(), WEBUI_RUNTIME_ITEM_MAX_PAYLOADS);
    let initial_cursor = parse_webui_projection_cursor(
        initial_events
            .last()
            .expect("initial event")
            .projection_cursor()
            .as_str(),
    )
    .unwrap();
    assert!(initial_cursor.runtime.is_some());
    assert!(initial_cursor.runtime_item.is_none());
    assert_eq!(initial_cursor.runtime_payloads_delivered, 0);
    assert!(matches!(
        initial_events[0].payload(),
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));

    let resumed_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(
                initial_events
                    .last()
                    .expect("initial event")
                    .projection_cursor()
                    .clone(),
            ),
        })
        .await
        .unwrap();

    assert!(resumed_events.is_empty());
    let emitted_activity_count = initial_events
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                ProductOutboundPayload::CapabilityActivity(_)
            )
        })
        .count();
    assert_eq!(emitted_activity_count, WEBUI_PROJECTION_PAGE_LIMIT);

    let final_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(UserId::new("webui-activity-overflow-user").unwrap()),
            scope: TurnScope::new(
                TenantId::new("webui-activity-overflow-tenant").unwrap(),
                Some(AgentId::new("webui-activity-overflow-agent").unwrap()),
                None,
                ThreadId::new("webui-activity-overflow-thread").unwrap(),
            ),
            after_cursor: Some(
                initial_events
                    .last()
                    .expect("initial event")
                    .projection_cursor()
                    .clone(),
            ),
        })
        .await
        .unwrap();
    assert!(final_events.is_empty());
}

#[tokio::test]
async fn webui_event_stream_mints_resumable_cursors_for_long_valid_scope_ids() {
    let tenant_id = TenantId::new(long_test_id("tenant", 't')).unwrap();
    let user_id = UserId::new(long_test_id("user", 'u')).unwrap();
    let agent_id = AgentId::new(long_test_id("agent", 'a')).unwrap();
    let thread_id = ThreadId::new(long_test_id("thread", 'h')).unwrap();
    let capability = CapabilityId::new("script.echo").unwrap();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    for _ in 0..(WEBUI_RUNTIME_ITEM_MAX_PAYLOADS + 1) {
        event_log
            .append(RuntimeEvent::dispatch_requested(
                resource_scope(
                    &tenant_id,
                    &user_id,
                    &agent_id,
                    &thread_id,
                    InvocationId::new(),
                ),
                capability.clone(),
            ))
            .await
            .unwrap();
    }

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-long-scope-reply").unwrap(),
    );
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(events.len(), WEBUI_RUNTIME_ITEM_MAX_PAYLOADS);
    assert!(
        events
            .iter()
            .all(|event| event.projection_cursor().as_str().len() <= 1024)
    );
}

#[tokio::test]
async fn webui_event_stream_rebases_stale_partial_activity_cursor() {
    let tenant_id = TenantId::new("webui-activity-stale-tenant").unwrap();
    let user_id = UserId::new("webui-activity-stale-user").unwrap();
    let agent_id = AgentId::new("webui-activity-stale-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-stale-thread").unwrap();
    let capability = CapabilityId::new("script.echo").unwrap();
    let initial_invocation = InvocationId::new();
    let newer_invocation = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                initial_invocation,
            ),
            capability.clone(),
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
        ReplyTargetBindingRef::new("webui-activity-stale-reply").unwrap(),
    );
    let initial_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(initial_events.len(), 2);
    let stale_cursor = initial_events[0].projection_cursor().clone();

    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                newer_invocation,
            ),
            capability,
        ))
        .await
        .unwrap();

    let resumed_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(stale_cursor),
        })
        .await
        .unwrap();

    assert_eq!(resumed_events.len(), 3);
    assert!(matches!(
        resumed_events[0].payload(),
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
    assert!(resumed_events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == initial_invocation
        )
    }));
    assert!(resumed_events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == newer_invocation
        )
    }));
    let resumed_cursor = parse_webui_projection_cursor(
        resumed_events
            .last()
            .expect("resumed event")
            .projection_cursor()
            .as_str(),
    )
    .unwrap();
    assert!(resumed_cursor.runtime.is_some());
    assert!(resumed_cursor.runtime_item.is_none());
    assert_eq!(resumed_cursor.runtime_payloads_delivered, 0);
}

#[tokio::test]
async fn webui_event_stream_drains_completed_and_failed_capability_activity_metadata() {
    let tenant_id = TenantId::new("webui-activity-terminal-tenant").unwrap();
    let user_id = UserId::new("webui-activity-terminal-user").unwrap();
    let agent_id = AgentId::new("webui-activity-terminal-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-terminal-thread").unwrap();
    let completed_invocation = InvocationId::new();
    let failed_invocation = InvocationId::new();
    let capability = CapabilityId::new("script.echo").unwrap();
    let provider = ExtensionId::new("script").unwrap();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_succeeded(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                completed_invocation,
            ),
            capability.clone(),
            provider.clone(),
            RuntimeKind::Script,
            64,
        ))
        .await
        .unwrap();
    event_log
        .append(RuntimeEvent::dispatch_failed(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                failed_invocation,
            ),
            capability.clone(),
            Some(provider),
            Some(RuntimeKind::Script),
            "policy_denied",
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-terminal-reply").unwrap(),
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

    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == completed_invocation
                    && activity.status == CapabilityActivityStatusView::Completed
                    && activity.output_bytes == Some(64)
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == failed_invocation
                    && activity.status == CapabilityActivityStatusView::Failed
                    && activity.error_kind.as_deref() == Some("policy_denied")
        )
    }));
}

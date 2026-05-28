use super::*;
use ironclaw_turns::run_profile::CapabilityInputRef;

fn preview_input_ref(label: &str) -> CapabilityInputRef {
    CapabilityInputRef::new(format!("input:{label}")).unwrap()
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
async fn capability_display_preview_store_redacts_common_secret_text_shapes() {
    let run_id = TurnRunId::new();
    let invocation_id = InvocationId::new();
    let capability = CapabilityId::new("script.output").unwrap();
    let input_ref = preview_input_ref("common-secret-text-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability,
        result_ref: "result:common-secret-text",
        output: &serde_json::Value::String(
            "password: secret123 file:///etc/passwd ghp_abcdefghijklmnopqrstuvwxyz xoxb-1234567890 AKIAIOSFODNN7EXAMPLE eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.signature"
                .to_string(),
        ),
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

    let rendered = serde_json::to_string(&preview).unwrap();
    assert!(!rendered.contains("secret123"));
    assert!(!rendered.contains("file:///etc/passwd"));
    assert!(!rendered.contains("ghp_abcdefghijklmnopqrstuvwxyz"));
    assert!(!rendered.contains("xoxb-1234567890"));
    assert!(!rendered.contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(!rendered.contains("eyJhbGciOiJIUzI1NiJ9"));
    assert!(rendered.contains("[redacted]"));
}

#[tokio::test]
async fn capability_display_preview_store_redacts_camel_case_api_key_json() {
    let run_id = TurnRunId::new();
    let invocation_id = InvocationId::new();
    let capability = CapabilityId::new("script.output").unwrap();
    let input_ref = preview_input_ref("camel-case-api-key-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability,
        result_ref: "result:camel-case-api-key",
        output: &serde_json::json!({
            "apiKey": "live-api-key-secret",
            "nested": {
                "serviceCredential": "credential-secret"
            }
        }),
        output_bytes: 128,
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
            output_bytes: Some(128),
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    let rendered = serde_json::to_string(&preview).unwrap();
    assert!(!rendered.contains("live-api-key-secret"));
    assert!(!rendered.contains("credential-secret"));
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
async fn capability_display_preview_store_preserves_long_line_counts() {
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

    assert!(!preview.truncated);
    assert!(
        preview
            .output_preview
            .as_ref()
            .unwrap()
            .contains("line-129")
    );
    assert!(
        preview
            .output_preview
            .as_ref()
            .unwrap()
            .contains("line-120")
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

    let first_snapshot = snapshot.clone();
    let first = runtime_payloads_for_item(
        &scope,
        &display_previews,
        RuntimePayloadItemInput {
            runs: first_snapshot.runs,
            capability_activities: first_snapshot.capability_activities,
            cursor: cursor.clone(),
            state_kind: StatePayloadKind::Snapshot,
        },
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

    let resumed = runtime_payloads_for_item(
        &scope,
        &display_previews,
        RuntimePayloadItemInput {
            runs: snapshot.runs,
            capability_activities: snapshot.capability_activities,
            cursor,
            state_kind: StatePayloadKind::Snapshot,
        },
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

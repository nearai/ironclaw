use super::*;
use ironclaw_product_adapters::ProductOutboundPayload;
use ironclaw_turns::run_profile::CapabilityInputRef;

fn preview_input_ref(label: &str) -> CapabilityInputRef {
    CapabilityInputRef::new(format!("input:{label}")).unwrap()
}

struct PreviewProjectionFixture {
    scope: TurnScope,
    cursor: EventProjectionCursor,
    display_previews: CapabilityDisplayPreviewStore,
    run_id: TurnRunId,
    input_ref: CapabilityInputRef,
    invocation_id: InvocationId,
    capability: CapabilityId,
    snapshot: ProjectionSnapshot,
}

impl PreviewProjectionFixture {
    fn completed(
        label: &str,
        capability_name: &str,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        let tenant_id = TenantId::new(format!("webui-preview-{label}-tenant")).unwrap();
        let user_id = UserId::new(format!("webui-preview-{label}-user")).unwrap();
        let agent_id = AgentId::new(format!("webui-preview-{label}-agent")).unwrap();
        let thread_id = ThreadId::new(format!("webui-preview-{label}-thread")).unwrap();
        let capability = CapabilityId::new(format!("builtin.{capability_name}")).unwrap();
        let invocation_id = InvocationId::new();
        let run_id = TurnRunId::new();
        let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id.clone());
        let projection_scope = runtime_projection_scope(&TurnActor::new(user_id), &scope);
        let cursor = EventProjectionCursor::for_scope(
            projection_scope,
            ironclaw_events::EventCursor::new(1),
        );
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
                updated_at,
            }],
            capability_activities: vec![CapabilityActivityProjection {
                invocation_id,
                run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
                capability_id: capability.clone(),
                thread_id: Some(thread_id),
                status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
                provider: None,
                runtime: None,
                process_id: None,
                output_bytes: Some(12),
                error_kind: None,
                last_cursor: ironclaw_events::EventCursor::new(1),
                updated_at,
            }],
            next_cursor: cursor.clone(),
            truncated: false,
        };
        Self {
            scope,
            cursor,
            display_previews: CapabilityDisplayPreviewStore::default(),
            run_id,
            input_ref: preview_input_ref(&format!("preview-{label}-input")),
            invocation_id,
            capability,
            snapshot,
        }
    }

    fn record_input(&self, tool_name: &str) {
        self.display_previews.record_input(
            &self.run_id.to_string(),
            &self.input_ref,
            tool_name,
            &serde_json::json!({"path": "src/main.rs"}),
        );
    }

    fn record_result(&self, result_ref: &str, output: serde_json::Value) {
        self.display_previews
            .record_result(CapabilityDisplayPreviewResult {
                run_id: &self.run_id.to_string(),
                input_ref: &self.input_ref,
                invocation_id: self.invocation_id,
                capability_id: &self.capability,
                result_ref,
                output: &output,
                output_bytes: 12,
            });
    }

    fn item_input(&self) -> RuntimePayloadItemInput {
        RuntimePayloadItemInput {
            runs: self.snapshot.runs.clone(),
            capability_activities: self.snapshot.capability_activities.clone(),
            cursor: self.cursor.clone(),
            state_kind: StatePayloadKind::Snapshot,
        }
    }
}

#[tokio::test]
async fn webui_projection_snapshot_resumes_preview_payload() {
    let fixture = PreviewProjectionFixture::completed("resume", "read_file", chrono::Utc::now());
    fixture.record_input("read_file");
    fixture.record_result(
        "result:preview-resume",
        serde_json::json!({"content": "fn main() {}"}),
    );

    let first = runtime_payloads_for_item(
        &fixture.scope,
        &fixture.display_previews,
        fixture.item_input(),
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
        first.payloads[0].payload,
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
    assert!(matches!(
        first.payloads[1].payload,
        ProductOutboundPayload::CapabilityActivity(_)
    ));

    let resumed = runtime_payloads_for_item(
        &fixture.scope,
        &fixture.display_previews,
        fixture.item_input(),
        Some(first.item_cursor.runtime),
        2,
        2,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resumed.payloads.len(), 1);
    assert!(matches!(
        &resumed.payloads[0].payload,
        ProductOutboundPayload::CapabilityDisplayPreview(preview)
            if preview.result_ref.as_deref() == Some("result:preview-resume")
    ));
}

#[tokio::test]
async fn webui_projection_holds_cursor_when_completed_preview_is_pending() {
    let fixture = PreviewProjectionFixture::completed("pending", "write_file", chrono::Utc::now());
    fixture.record_input("write_file");

    let first = runtime_payloads_for_item(
        &fixture.scope,
        &fixture.display_previews,
        fixture.item_input(),
        None,
        0,
        8,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first.total, 3);
    assert_eq!(first.payloads.len(), 2);
    assert_eq!(first.payloads[1].delivered, 2);
    assert!(matches!(
        first.payloads[0].payload,
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
    assert!(matches!(
        first.payloads[1].payload,
        ProductOutboundPayload::CapabilityActivity(_)
    ));

    let pending_resume = runtime_payloads_for_item(
        &fixture.scope,
        &fixture.display_previews,
        fixture.item_input(),
        Some(first.item_cursor.runtime),
        first.payloads[1].delivered,
        8,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(pending_resume.total, 3);
    assert!(pending_resume.payloads.is_empty());

    let mut batch = WebuiProjectionBatch::new(WebuiProjectionCursor {
        runtime_item: Some(first.item_cursor.runtime),
        runtime_payloads_delivered: first.payloads[1].delivered,
        ..Default::default()
    });
    let advanced = batch
        .push_durable_runtime_payloads(
            pending_resume.final_cursor,
            pending_resume.item_cursor,
            pending_resume.payloads,
            pending_resume.total,
            pending_resume.already_delivered,
        )
        .unwrap();
    assert!(!advanced);
    assert_eq!(batch.cursor.runtime_item, Some(first.item_cursor.runtime));
    assert_eq!(
        batch.cursor.runtime_payloads_delivered,
        first.payloads[1].delivered
    );

    fixture.record_result(
        "result:preview-pending",
        serde_json::json!({"content": "wrote file"}),
    );

    let resumed = runtime_payloads_for_item(
        &fixture.scope,
        &fixture.display_previews,
        fixture.item_input(),
        Some(first.item_cursor.runtime),
        first.payloads[1].delivered,
        8,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resumed.total, 3);
    assert_eq!(resumed.payloads.len(), 1);
    assert_eq!(resumed.payloads[0].delivered, 3);
    assert!(matches!(
        &resumed.payloads[0].payload,
        ProductOutboundPayload::CapabilityDisplayPreview(preview)
            if preview.result_ref.as_deref() == Some("result:preview-pending")
    ));
}

#[tokio::test]
async fn webui_projection_advances_stale_completed_activity_without_preview_record() {
    let fixture = PreviewProjectionFixture::completed(
        "stale",
        "write_file",
        chrono::Utc::now() - chrono::Duration::seconds(11),
    );

    let item = runtime_payloads_for_item(
        &fixture.scope,
        &fixture.display_previews,
        fixture.item_input(),
        None,
        0,
        8,
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(item.total, 2);
    assert_eq!(item.payloads.len(), 2);
    assert!(matches!(
        item.payloads[0].payload,
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
    assert!(matches!(
        item.payloads[1].payload,
        ProductOutboundPayload::CapabilityActivity(_)
    ));

    let mut batch = WebuiProjectionBatch::new(WebuiProjectionCursor::default());
    let advanced = batch
        .push_durable_runtime_payloads(
            item.final_cursor,
            item.item_cursor,
            item.payloads,
            item.total,
            item.already_delivered,
        )
        .unwrap();
    assert!(advanced);
    assert_eq!(batch.cursor.runtime, Some(fixture.cursor));
    assert_eq!(batch.cursor.runtime_item, None);
    assert_eq!(batch.cursor.runtime_payloads_delivered, 0);
}

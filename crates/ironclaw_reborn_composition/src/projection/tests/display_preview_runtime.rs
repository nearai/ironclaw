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

struct PreviewActivityRefs {
    run_id: TurnRunId,
    input_ref: CapabilityInputRef,
    invocation_id: InvocationId,
    capability: CapabilityId,
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
                first_cursor: ironclaw_events::EventCursor::new(1),
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
        let refs = PreviewActivityRefs {
            run_id: self.run_id,
            input_ref: self.input_ref.clone(),
            invocation_id: self.invocation_id,
            capability: self.capability.clone(),
        };
        self.record_input_for(&refs, tool_name);
    }

    fn record_input_for(&self, refs: &PreviewActivityRefs, tool_name: &str) {
        self.display_previews.record_input(
            &refs.run_id.to_string(),
            &refs.input_ref,
            tool_name,
            &serde_json::json!({"path": "src/main.rs"}),
        );
    }

    fn record_result(&self, result_ref: &str, output: serde_json::Value) {
        let refs = PreviewActivityRefs {
            run_id: self.run_id,
            input_ref: self.input_ref.clone(),
            invocation_id: self.invocation_id,
            capability: self.capability.clone(),
        };
        self.record_result_for(&refs, result_ref, output);
    }

    fn record_result_for(
        &self,
        refs: &PreviewActivityRefs,
        result_ref: &str,
        output: serde_json::Value,
    ) {
        self.display_previews
            .record_result(CapabilityDisplayPreviewResult {
                run_id: &refs.run_id.to_string(),
                input_ref: &refs.input_ref,
                invocation_id: refs.invocation_id,
                capability_id: &refs.capability,
                result_ref,
                output: &output,
                output_bytes: 12,
            });
    }

    fn add_completed_activity(
        &mut self,
        label: &str,
        capability_name: &str,
        updated_at: chrono::DateTime<chrono::Utc>,
        cursor: ironclaw_events::EventCursor,
    ) -> PreviewActivityRefs {
        let capability = CapabilityId::new(format!("builtin.{capability_name}")).unwrap();
        let invocation_id = InvocationId::new();
        let run_id = TurnRunId::new();
        self.snapshot.runs.push(RunStatusProjection {
            invocation_id,
            capability_id: capability.clone(),
            thread_id: Some(self.scope.thread_id.clone()),
            status: RunProjectionStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            error_kind: None,
            last_cursor: cursor,
            updated_at,
        });
        self.snapshot
            .capability_activities
            .push(CapabilityActivityProjection {
                invocation_id,
                run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
                capability_id: capability.clone(),
                thread_id: Some(self.scope.thread_id.clone()),
                status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
                provider: None,
                runtime: None,
                process_id: None,
                output_bytes: Some(12),
                error_kind: None,
                first_cursor: cursor,
                last_cursor: cursor,
                updated_at,
            });
        PreviewActivityRefs {
            run_id,
            input_ref: preview_input_ref(&format!("preview-{label}-input")),
            invocation_id,
            capability,
        }
    }

    fn set_first_activity_updated_at(&mut self, updated_at: chrono::DateTime<chrono::Utc>) {
        self.snapshot.runs[0].updated_at = updated_at;
        self.snapshot.capability_activities[0].updated_at = updated_at;
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

#[tokio::test]
async fn webui_projection_advances_held_cursor_when_pending_preview_times_out() {
    let mut fixture =
        PreviewProjectionFixture::completed("pending-timeout", "write_file", chrono::Utc::now());
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

    fixture.set_first_activity_updated_at(chrono::Utc::now() - chrono::Duration::seconds(11));
    let timed_out = runtime_payloads_for_item(
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
    assert_eq!(timed_out.total, 2);
    assert!(timed_out.payloads.is_empty());

    let mut batch = WebuiProjectionBatch::new(WebuiProjectionCursor {
        runtime_item: Some(first.item_cursor.runtime),
        runtime_payloads_delivered: first.payloads[1].delivered,
        ..Default::default()
    });
    let advanced = batch
        .push_durable_runtime_payloads(
            timed_out.final_cursor,
            timed_out.item_cursor,
            timed_out.payloads,
            timed_out.total,
            timed_out.already_delivered,
        )
        .unwrap();
    assert!(advanced);
    assert_eq!(batch.cursor.runtime, Some(fixture.cursor));
    assert_eq!(batch.cursor.runtime_item, None);
    assert_eq!(batch.cursor.runtime_payloads_delivered, 0);
    assert_eq!(batch.payloads.len(), 1);
    assert!(matches!(
        batch.payloads[0].1,
        ProductOutboundPayload::KeepAlive
    ));
}

#[tokio::test]
async fn webui_projection_with_pending_preview_on_first_activity_streams_second_activity() {
    let now = chrono::Utc::now();
    let mut fixture = PreviewProjectionFixture::completed("multi", "read_file", now);
    let second = fixture.add_completed_activity(
        "multi-second",
        "list_files",
        now,
        ironclaw_events::EventCursor::new(2),
    );
    fixture.record_input("read_file");
    fixture.record_input_for(&second, "list_files");
    fixture.record_result_for(
        &second,
        "result:preview-multi-second",
        serde_json::json!({"content": "src/main.rs\nCargo.toml"}),
    );

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
    assert_eq!(first.total, 4);
    assert_eq!(first.payloads.len(), 3);
    assert!(matches!(
        first.payloads[0].payload,
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
    assert!(matches!(
        &first.payloads[1].payload,
        ProductOutboundPayload::CapabilityActivity(activity)
            if activity.invocation_id == fixture.invocation_id
    ));
    assert!(matches!(
        &first.payloads[2].payload,
        ProductOutboundPayload::CapabilityActivity(activity)
            if activity.invocation_id == second.invocation_id
    ));

    fixture.record_result(
        "result:preview-multi-first",
        serde_json::json!({"content": "fn main() {}"}),
    );
    let resumed = runtime_payloads_for_item(
        &fixture.scope,
        &fixture.display_previews,
        fixture.item_input(),
        Some(first.item_cursor.runtime),
        first.payloads[2].delivered,
        8,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resumed.total, 5);
    assert_eq!(resumed.payloads.len(), 2);
    assert_eq!(resumed.payloads[0].delivered, 4);
    assert_eq!(resumed.payloads[1].delivered, 5);
    assert!(matches!(
        &resumed.payloads[0].payload,
        ProductOutboundPayload::CapabilityDisplayPreview(preview)
            if preview.result_ref.as_deref() == Some("result:preview-multi-first")
    ));
    assert!(matches!(
        &resumed.payloads[1].payload,
        ProductOutboundPayload::CapabilityDisplayPreview(preview)
            if preview.result_ref.as_deref() == Some("result:preview-multi-second")
    ));
}

/// Regression: a still-running first activity must hold the runtime cursor at
/// its not-yet-existent preview slot without hiding later activity metadata.
/// The UI should show subsequent OK/ERR tool rows while preserving the preview
/// resume point so the first invocation's rich preview is delivered once ready.
#[tokio::test]
async fn webui_projection_holds_preview_cursor_while_streaming_later_activity() {
    let now = chrono::Utc::now();
    let mut fixture = PreviewProjectionFixture::completed("running", "read_file", now);
    // The first activity is in flight: no preview exists yet, but one will once
    // it completes.
    fixture.snapshot.runs[0].status = RunProjectionStatus::Running;
    fixture.snapshot.capability_activities[0].status =
        ironclaw_event_projections::CapabilityActivityStatus::Running;
    let second = fixture.add_completed_activity(
        "running-second",
        "list_files",
        now,
        ironclaw_events::EventCursor::new(2),
    );
    fixture.record_input_for(&second, "list_files");
    fixture.record_result_for(
        &second,
        "result:preview-running-second",
        serde_json::json!({"content": "src/main.rs\nCargo.toml"}),
    );

    // Drain while the first activity is still running. The drain holds at its
    // pending preview slot but still streams the second activity card so the UI
    // can show progress in call order instead of waiting for run completion.
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
    assert_eq!(first.total, 4);
    assert_eq!(first.payloads.len(), 3);
    assert!(matches!(
        first.payloads[0].payload,
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
    assert!(matches!(
        &first.payloads[1].payload,
        ProductOutboundPayload::CapabilityActivity(activity)
            if activity.invocation_id == fixture.invocation_id
    ));
    assert!(matches!(
        &first.payloads[2].payload,
        ProductOutboundPayload::CapabilityActivity(activity)
            if activity.invocation_id == second.invocation_id
    ));

    // The first invocation completes and records its preview.
    fixture.snapshot.runs[0].status = RunProjectionStatus::Completed;
    fixture.snapshot.capability_activities[0].status =
        ironclaw_event_projections::CapabilityActivityStatus::Completed;
    fixture.record_input("read_file");
    fixture.record_result(
        "result:preview-running-first",
        serde_json::json!({"content": "fn main() {}"}),
    );

    // Resume from the held cursor: both previews are delivered in order, and
    // the first activity's preview is not dropped.
    let resumed = runtime_payloads_for_item(
        &fixture.scope,
        &fixture.display_previews,
        fixture.item_input(),
        Some(first.item_cursor.runtime),
        first.payloads[2].delivered,
        8,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resumed.total, 5);
    assert_eq!(resumed.payloads.len(), 2);
    assert!(matches!(
        &resumed.payloads[0].payload,
        ProductOutboundPayload::CapabilityDisplayPreview(preview)
            if preview.result_ref.as_deref() == Some("result:preview-running-first")
    ));
    assert!(matches!(
        &resumed.payloads[1].payload,
        ProductOutboundPayload::CapabilityDisplayPreview(preview)
            if preview.result_ref.as_deref() == Some("result:preview-running-second")
    ));
}

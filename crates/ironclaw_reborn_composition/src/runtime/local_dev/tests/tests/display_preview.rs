use std::sync::Arc;

use ironclaw_host_api::{CapabilityDisplayOutputPreview, CapabilityId, InvocationId};
use ironclaw_loop_support::{
    CapabilityResultWrite, DurablePersistence, LoopCapabilityInputResolver,
    LoopCapabilityResultWriter,
};
use ironclaw_threads::{
    CapabilityDisplayPreviewEnvelope, CapabilityDisplayPreviewStatus, EnsureThreadRequest,
    InMemorySessionThreadService, MessageKind, SessionThreadService, ThreadHistoryRequest,
};

use super::{
    CapabilityDisplayPreviewStore, LocalDevCapabilityIo, UserId, local_dev_thread_scope_for_run,
    provider_tool_call, run_context,
};

#[tokio::test]
async fn capability_io_writes_display_preview_to_durable_history() {
    let run_context = run_context("durable-display-preview").await;
    let fallback_user_id = UserId::new("durable-display-preview-owner").expect("fallback user id");
    // The durable preview sink derives the thread scope from the run context;
    // register the thread under that same derived scope.
    let thread_scope = local_dev_thread_scope_for_run(&run_context, &fallback_user_id)
        .expect("run scope has an agent");
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(run_context.thread_id.clone()),
            created_by_actor_id: "actor-a".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread exists");
    let capability_io = LocalDevCapabilityIo::new_with_durable_previews(
        Arc::new(CapabilityDisplayPreviewStore::default()),
        thread_service.clone(),
        fallback_user_id.clone(),
    );
    let input_ref = capability_io
        .register_provider_tool_call_input(
            &run_context,
            &provider_tool_call(serde_json::json!({"path": "/workspace/main.rs"})),
        )
        .await
        .expect("input stages");
    let invocation_id = InvocationId::new();
    let capability_id = CapabilityId::new("builtin.write_file").expect("capability id");

    capability_io
        .write_capability_result(CapabilityResultWrite {
            run_context: &run_context,
            input_ref: &input_ref,
            invocation_id,
            capability_id: &capability_id,
            output: serde_json::json!({"success": true}),
            display_preview: Some(CapabilityDisplayOutputPreview {
                output_summary: Some("Edited 1 file: +1/-1".to_string()),
                output_preview:
                    "--- a/workspace/main.rs\n+++ b/workspace/main.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n"
                        .to_string(),
                output_kind: "unified_diff".to_string(),
                subtitle: Some("/workspace/main.rs".to_string()),
                truncated: false,
            }),
            durable_persistence: DurablePersistence::Persist,
        })
        .await
        .map(|_| ())
        .expect("result stages");

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope,
            thread_id: run_context.thread_id.clone(),
        })
        .await
        .expect("history loads");
    let preview_message = history
        .messages
        .iter()
        .find(|message| message.kind == MessageKind::CapabilityDisplayPreview)
        .expect("durable preview message");
    let envelope: CapabilityDisplayPreviewEnvelope =
        serde_json::from_str(preview_message.content.as_deref().expect("preview content"))
            .expect("preview envelope parses");

    assert_eq!(envelope.output_kind.as_deref(), Some("unified_diff"));
    assert_eq!(
        envelope.output_summary.as_deref(),
        Some("Edited 1 file: +1/-1")
    );
    assert_eq!(envelope.subtitle.as_deref(), Some("/workspace/main.rs"));
    assert!(
        envelope
            .output_preview
            .as_deref()
            .is_some_and(|preview| preview.contains("+new"))
    );
}

// Regression: a failed capability's display preview must also reach the
// durable timeline (status Failed) so the rendered detail survives
// refresh/replay, not just the live stream.
#[tokio::test]
async fn capability_io_writes_failure_display_preview_to_durable_history() {
    let run_context = run_context("durable-failure-preview").await;
    let fallback_user_id = UserId::new("durable-failure-preview-owner").expect("fallback user id");
    let thread_scope = local_dev_thread_scope_for_run(&run_context, &fallback_user_id)
        .expect("run scope has an agent");
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(run_context.thread_id.clone()),
            created_by_actor_id: "actor-a".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread exists");
    let capability_io = LocalDevCapabilityIo::new_with_durable_previews(
        Arc::new(CapabilityDisplayPreviewStore::default()),
        thread_service.clone(),
        fallback_user_id.clone(),
    );
    let input_ref = capability_io
        .register_provider_tool_call_input(
            &run_context,
            &provider_tool_call(serde_json::json!({"data": "{"})),
        )
        .await
        .expect("input stages");
    let invocation_id = InvocationId::new();
    capability_io.record_running_invocation(&run_context, invocation_id, &input_ref);
    let capability_id = CapabilityId::new("builtin.json").expect("capability id");

    capability_io
        .stage_capability_failure_preview(
            &run_context,
            invocation_id,
            &capability_id,
            "invalid JSON: expected value at line 1 column 1",
        )
        .await;

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope,
            thread_id: run_context.thread_id.clone(),
        })
        .await
        .expect("history loads");
    let preview_message = history
        .messages
        .iter()
        .find(|message| message.kind == MessageKind::CapabilityDisplayPreview)
        .expect("durable failure preview message");
    let envelope: CapabilityDisplayPreviewEnvelope =
        serde_json::from_str(preview_message.content.as_deref().expect("preview content"))
            .expect("preview envelope parses");

    assert_eq!(envelope.status, CapabilityDisplayPreviewStatus::Failed);
    assert_eq!(
        envelope.output_summary.as_deref(),
        Some("invalid JSON: expected value at line 1 column 1")
    );
}

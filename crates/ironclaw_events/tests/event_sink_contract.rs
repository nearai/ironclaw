use std::sync::Arc;

use ironclaw_events::*;
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::*;

#[tokio::test]
async fn in_memory_event_sink_records_runtime_events_in_order() {
    let sink = InMemoryEventSink::new();
    let first = RuntimeEvent::dispatch_requested(
        sample_scope(),
        CapabilityId::new("echo-wasm.say").unwrap(),
    );
    let second = RuntimeEvent::runtime_selected(
        sample_scope(),
        CapabilityId::new("echo-wasm.say").unwrap(),
        ExtensionId::new("echo-wasm").unwrap(),
        RuntimeKind::Wasm,
    );

    sink.emit(first.clone()).await.unwrap();
    sink.emit(second.clone()).await.unwrap();

    let events = sink.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, RuntimeEventKind::DispatchRequested);
    assert_eq!(
        events[0].capability_id,
        CapabilityId::new("echo-wasm.say").unwrap()
    );
    assert_eq!(events[0].runtime, None);
    assert_eq!(events[1].kind, RuntimeEventKind::RuntimeSelected);
    assert_eq!(
        events[1].provider,
        Some(ExtensionId::new("echo-wasm").unwrap())
    );
    assert_eq!(events[1].runtime, Some(RuntimeKind::Wasm));
}

#[tokio::test]
async fn jsonl_event_sink_persists_redacted_runtime_events_without_host_paths() {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(storage.clone()),
    )
    .unwrap();
    let path = VirtualPath::new("/engine/events/reborn-demo.jsonl").unwrap();
    let sink = JsonlEventSink::new(Arc::new(fs), path.clone());

    sink.emit(RuntimeEvent::dispatch_failed(
        sample_scope(),
        CapabilityId::new("echo-script.say").unwrap(),
        Some(ExtensionId::new("echo-script").unwrap()),
        Some(RuntimeKind::Script),
        "MissingRuntimeBackend",
    ))
    .await
    .unwrap();

    let bytes = sink.filesystem().read_file(&path).await.unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("dispatch_failed"));
    assert!(text.contains("MissingRuntimeBackend"));
    assert!(!text.contains(storage.to_string_lossy().as_ref()));

    let events = sink.read_events().await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, RuntimeEventKind::DispatchFailed);
    assert_eq!(
        events[0].error_kind.as_deref(),
        Some("MissingRuntimeBackend")
    );
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

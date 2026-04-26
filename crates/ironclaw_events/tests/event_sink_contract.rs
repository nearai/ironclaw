use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ironclaw_events::*;
use ironclaw_filesystem::{
    DirEntry, FileStat, FilesystemError, FilesystemOperation, LocalFilesystem, RootFilesystem,
};
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
async fn process_lifecycle_events_carry_process_identity_and_scope() {
    let scope = sample_scope();
    let process_id = ProcessId::new();

    let started = RuntimeEvent::process_started(
        scope.clone(),
        CapabilityId::new("echo.say").unwrap(),
        ExtensionId::new("echo").unwrap(),
        RuntimeKind::Wasm,
        process_id,
    );
    let failed = RuntimeEvent::process_failed(
        scope.clone(),
        CapabilityId::new("echo.say").unwrap(),
        ExtensionId::new("echo").unwrap(),
        RuntimeKind::Wasm,
        process_id,
        "RuntimeDispatch",
    );

    assert_eq!(started.kind, RuntimeEventKind::ProcessStarted);
    assert_eq!(started.process_id, Some(process_id));
    assert_eq!(started.scope.tenant_id, scope.tenant_id);
    assert_eq!(started.scope.user_id, scope.user_id);
    assert_eq!(started.provider, Some(ExtensionId::new("echo").unwrap()));
    assert_eq!(started.runtime, Some(RuntimeKind::Wasm));
    assert_eq!(failed.kind, RuntimeEventKind::ProcessFailed);
    assert_eq!(failed.process_id, Some(process_id));
    assert_eq!(failed.error_kind.as_deref(), Some("RuntimeDispatch"));

    let unsafe_error = RuntimeEvent::process_failed(
        scope,
        CapabilityId::new("echo.say").unwrap(),
        ExtensionId::new("echo").unwrap(),
        RuntimeKind::Wasm,
        process_id,
        "failed at /tmp/secret-token.txt",
    );
    assert_eq!(unsafe_error.error_kind.as_deref(), Some("Unclassified"));
}

#[tokio::test]
async fn jsonl_event_sink_does_not_overwrite_when_existing_log_read_fails() {
    let fs = Arc::new(ReadFailFilesystem::new());
    let path = scoped_runtime_event_log_path(&sample_scope(), "reborn-demo.jsonl").unwrap();
    let sink = JsonlEventSink::new(Arc::clone(&fs), path);

    let err = sink
        .emit(RuntimeEvent::dispatch_requested(
            sample_scope(),
            CapabilityId::new("echo.say").unwrap(),
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, EventError::Filesystem(_)));
    assert_eq!(fs.write_count(), 0);
}

#[tokio::test]
async fn jsonl_audit_sink_does_not_overwrite_when_existing_log_read_fails() {
    let fs = Arc::new(ReadFailFilesystem::new());
    let path = scoped_audit_log_path(&sample_scope(), "approval-audit.jsonl").unwrap();
    let sink = JsonlAuditSink::new(Arc::clone(&fs), path);

    let err = sink
        .emit_audit(sample_approval_audit_envelope())
        .await
        .unwrap_err();

    assert!(matches!(err, EventError::Filesystem(_)));
    assert_eq!(fs.write_count(), 0);
}

#[test]
fn scoped_jsonl_paths_are_tenant_user_scoped_and_reject_traversal() {
    let scope = sample_scope();

    assert_eq!(
        scoped_runtime_event_log_path(&scope, "reborn-demo.jsonl")
            .unwrap()
            .as_str(),
        "/engine/tenants/tenant1/users/user1/events/runtime/reborn-demo.jsonl"
    );
    assert_eq!(
        scoped_audit_log_path(&scope, "approval-audit.jsonl")
            .unwrap()
            .as_str(),
        "/engine/tenants/tenant1/users/user1/events/audit/approval-audit.jsonl"
    );
    assert!(matches!(
        scoped_audit_log_path(&scope, "../approval-audit.jsonl"),
        Err(EventError::InvalidPath { .. })
    ));
}

#[tokio::test]
async fn jsonl_sinks_treat_missing_log_files_as_empty() {
    let fs = Arc::new(engine_filesystem(tempfile::tempdir().unwrap().keep()));
    let scope = sample_scope();
    let event_sink = JsonlEventSink::new(
        Arc::clone(&fs),
        scoped_runtime_event_log_path(&scope, "reborn-demo.jsonl").unwrap(),
    );
    let audit_sink = JsonlAuditSink::new(
        Arc::clone(&fs),
        scoped_audit_log_path(&scope, "approval-audit.jsonl").unwrap(),
    );

    assert_eq!(event_sink.read_events().await.unwrap(), Vec::new());
    assert_eq!(audit_sink.read_records().await.unwrap(), Vec::new());
}

#[tokio::test]
async fn jsonl_event_sink_rejects_malformed_existing_log_without_appending() {
    let fs = Arc::new(engine_filesystem(tempfile::tempdir().unwrap().keep()));
    let scope = sample_scope();
    let path = scoped_runtime_event_log_path(&scope, "reborn-demo.jsonl").unwrap();
    fs.write_file(&path, b"{not-json}\n").await.unwrap();
    let sink = JsonlEventSink::new(Arc::clone(&fs), path.clone());

    assert!(matches!(
        sink.read_events().await.unwrap_err(),
        EventError::Serialize { .. }
    ));
    assert!(matches!(
        sink.emit(RuntimeEvent::dispatch_requested(
            scope,
            CapabilityId::new("echo.say").unwrap(),
        ))
        .await
        .unwrap_err(),
        EventError::Serialize { .. }
    ));
    assert_eq!(fs.read_file(&path).await.unwrap(), b"{not-json}\n");
}

#[tokio::test]
async fn jsonl_audit_sink_rejects_malformed_existing_log_without_appending() {
    let fs = Arc::new(engine_filesystem(tempfile::tempdir().unwrap().keep()));
    let scope = sample_scope();
    let path = scoped_audit_log_path(&scope, "approval-audit.jsonl").unwrap();
    fs.write_file(&path, b"{not-json}\n").await.unwrap();
    let sink = JsonlAuditSink::new(Arc::clone(&fs), path.clone());

    assert!(matches!(
        sink.read_records().await.unwrap_err(),
        EventError::Serialize { .. }
    ));
    assert!(matches!(
        sink.emit_audit(sample_approval_audit_envelope())
            .await
            .unwrap_err(),
        EventError::Serialize { .. }
    ));
    assert_eq!(fs.read_file(&path).await.unwrap(), b"{not-json}\n");
}

#[tokio::test]
async fn jsonl_event_sink_persists_redacted_runtime_events_without_host_paths() {
    let storage = tempfile::tempdir().unwrap().keep();
    let fs = engine_filesystem(storage.clone());
    let scope = sample_scope();
    let path = scoped_runtime_event_log_path(&scope, "reborn-demo.jsonl").unwrap();
    let sink = JsonlEventSink::new(Arc::new(fs), path.clone());

    sink.emit(RuntimeEvent::dispatch_failed(
        scope,
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

fn engine_filesystem(storage: std::path::PathBuf) -> LocalFilesystem {
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn sample_approval_audit_envelope() -> AuditEnvelope {
    let scope = sample_scope();
    let capability = CapabilityId::new("echo.say").unwrap();
    let request = ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: CorrelationId::new(),
        requested_by: Principal::Extension(ExtensionId::new("echo").unwrap()),
        action: Box::new(Action::Dispatch {
            capability,
            estimated_resources: ResourceEstimate::default(),
        }),
        invocation_fingerprint: None,
        reason: "redacted".to_string(),
        reusable_scope: None,
    };
    AuditEnvelope::approval_resolved(
        &scope,
        &request,
        Principal::User(scope.user_id.clone()),
        "approved",
    )
}

#[derive(Debug)]
struct ReadFailFilesystem {
    writes: AtomicUsize,
}

impl ReadFailFilesystem {
    fn new() -> Self {
        Self {
            writes: AtomicUsize::new(0),
        }
    }

    fn write_count(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl RootFilesystem for ReadFailFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::ReadFile,
            reason: "permission denied".to_string(),
        })
    }

    async fn write_file(&self, _path: &VirtualPath, _bytes: &[u8]) -> Result<(), FilesystemError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::ListDir,
            reason: "permission denied".to_string(),
        })
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Stat,
            reason: "permission denied".to_string(),
        })
    }
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

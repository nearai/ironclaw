use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_event_projections::{
    EventProjectionService, ProjectionRequest, ProjectionScope, ReplayEventProjectionService,
    RunProjectionStatus, TimelineEntryKind,
};
use ironclaw_events::{
    DurableEventLog, DurableEventSink, EventSink, EventStreamKey, InMemoryDurableEventLog,
    InMemoryEventSink, ReadScope, RuntimeEventKind,
};
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::*;
use ironclaw_processes::*;
use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornProfile, build_reborn_event_stores,
};
use serde_json::json;
use tokio::{sync::Notify, time::timeout};

#[tokio::test]
async fn process_services_complete_background_process_through_process_host_and_eventing_store() {
    let events = InMemoryEventSink::new();
    let event_sink: Arc<dyn EventSink> = Arc::new(events.clone());
    let process_store = Arc::new(EventingProcessStore::new(
        in_mem_process_store(),
        event_sink,
    ));
    let result_store = Arc::new(in_mem_process_result_store());
    let services = ProcessServices::new(Arc::clone(&process_store), Arc::clone(&result_store));
    let manager = services.background_manager(Arc::new(SuccessExecutor));
    let host = services.host().with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant-a", "user-a");

    let started = manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    assert_eq!(started.status, ProcessStatus::Running);

    let result = host.await_result(&scope, process_id).await.unwrap();

    assert_eq!(result.status, ProcessStatus::Completed);
    // Filesystem store externalizes output behind `output_ref` (§4.3); read via host.
    assert_eq!(result.output, None);
    assert_eq!(
        host.output(&scope, process_id).await.unwrap(),
        Some(json!({"ok": true}))
    );
    assert_eq!(
        host.status(&scope, process_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProcessStatus::Completed
    );
    assert_eq!(
        host.output(&scope, process_id).await.unwrap(),
        Some(json!({"ok": true}))
    );

    let recorded = events.events();
    let kinds = recorded.iter().map(|event| event.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::ProcessStarted,
            RuntimeEventKind::ProcessCompleted,
        ]
    );
    assert_eq!(recorded[0].process_id, Some(process_id));
    assert_eq!(recorded[1].process_id, Some(process_id));
    assert_eq!(
        recorded[1].provider,
        Some(ExtensionId::new("echo").unwrap())
    );
    assert_eq!(recorded[1].runtime, Some(RuntimeKind::Wasm));
}

#[tokio::test]
async fn process_services_completed_lifecycle_projects_from_jsonl_durable_log_metadata_only() {
    let temp = tempfile::tempdir().unwrap();
    let store_root = temp.path().join("reborn-event-store");
    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: store_root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .unwrap();
    let event_log = Arc::clone(&stores.events);
    let event_sink: Arc<dyn EventSink> = Arc::new(DurableEventSink::new(Arc::clone(&event_log)));
    let process_store = Arc::new(EventingProcessStore::new(
        in_mem_process_store(),
        event_sink,
    ));
    let result_store = Arc::new(in_mem_process_result_store());
    let services = ProcessServices::new(Arc::clone(&process_store), Arc::clone(&result_store));
    let manager = services.background_manager(Arc::new(SentinelSuccessExecutor));
    let host = services.host().with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant-a", "user-a");
    let start = process_start_with_input(
        process_id,
        invocation_id,
        scope.clone(),
        json!({
            "message": "PROCESS_INPUT_SENTINEL_3022 /tmp/private-process-path",
            "secret": "PROCESS_SECRET_SENTINEL_3022_sk_live_secret",
        }),
    );

    let started = manager.spawn(start).await.unwrap();
    assert_eq!(started.status, ProcessStatus::Running);
    let result = host.await_result(&scope, process_id).await.unwrap();
    assert_eq!(result.status, ProcessStatus::Completed);
    // Filesystem store externalizes output behind `output_ref` (§4.3); read via host.
    assert_eq!(result.output, None);
    assert_eq!(
        host.output(&scope, process_id).await.unwrap(),
        Some(json!({"ok": true, "raw": "PROCESS_OUTPUT_SENTINEL_3022"}))
    );

    let projection = ReplayEventProjectionService::from_runtime_log(Arc::clone(&event_log));
    let snapshot = projection
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::for_process(&scope, process_id),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(
        snapshot
            .timeline
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::ProcessStarted,
            TimelineEntryKind::ProcessCompleted,
        ]
    );
    assert!(snapshot.timeline.entries.iter().all(|entry| {
        entry.process_id == Some(process_id) && entry.invocation_id == invocation_id
    }));
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Completed);
    assert_eq!(snapshot.runs[0].process_id, Some(process_id));

    let projection_json = serde_json::to_string(&snapshot).unwrap();
    let jsonl_bytes = read_directory_text(&store_root);
    for forbidden in [
        "PROCESS_INPUT_SENTINEL_3022",
        "/tmp/private-process-path",
        "PROCESS_SECRET_SENTINEL_3022",
        "PROCESS_OUTPUT_SENTINEL_3022",
    ] {
        assert!(
            !projection_json.contains(forbidden),
            "process projection leaked {forbidden}: {projection_json}"
        );
        assert!(
            !jsonl_bytes.contains(forbidden),
            "durable process event bytes leaked {forbidden}: {jsonl_bytes}"
        );
    }
}

#[tokio::test]
async fn process_services_failed_lifecycle_projection_sanitizes_error_and_filters_process_scope() {
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let event_sink: Arc<dyn EventSink> = Arc::new(DurableEventSink::new(Arc::clone(&event_log)));
    let process_store = Arc::new(EventingProcessStore::new(
        in_mem_process_store(),
        event_sink,
    ));
    let result_store = Arc::new(in_mem_process_result_store());
    let services = ProcessServices::new(Arc::clone(&process_store), Arc::clone(&result_store));
    let manager = services.background_manager(Arc::new(SentinelFailureExecutor));
    let host = services.host().with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let sibling_process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant-a", "user-a");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    let result = host.await_result(&scope, process_id).await.unwrap();
    assert_eq!(result.status, ProcessStatus::Failed);
    assert_eq!(result.error_kind.as_deref(), Some("Unclassified"));

    let projection = ReplayEventProjectionService::from_runtime_log(Arc::clone(&event_log));
    let visible = projection
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::for_process(&scope, process_id),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(
        visible
            .timeline
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::ProcessStarted,
            TimelineEntryKind::ProcessFailed,
        ]
    );
    assert_eq!(visible.runs.len(), 1);
    assert_eq!(visible.runs[0].status, RunProjectionStatus::Failed);
    assert_eq!(visible.runs[0].error_kind.as_deref(), Some("Unclassified"));
    let serialized = serde_json::to_string(&visible).unwrap();
    for forbidden in [
        "PROCESS_ERROR_SENTINEL_3022",
        "/tmp/private-process-error",
        "sk_live_secret",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "failed process projection leaked {forbidden}: {serialized}"
        );
    }

    let sibling = projection
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::for_process(&scope, sibling_process_id),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert!(sibling.timeline.entries.is_empty());
    assert!(sibling.runs.is_empty());
    let raw_sibling_replay = event_log
        .read_after_cursor(
            &EventStreamKey::from_scope(&scope),
            &ReadScope {
                project_id: scope.project_id.clone(),
                mission_id: scope.mission_id.clone(),
                thread_id: scope.thread_id.clone(),
                process_id: Some(sibling_process_id),
            },
            None,
            10,
        )
        .await
        .unwrap();
    assert!(raw_sibling_replay.entries.is_empty());
}

#[tokio::test]
async fn process_host_kill_preserves_terminal_state_and_suppresses_late_completion_event() {
    let events = InMemoryEventSink::new();
    let event_sink: Arc<dyn EventSink> = Arc::new(events.clone());
    let executor = Arc::new(CancelThenLateSuccessExecutor::default());
    let process_store = Arc::new(PostCompletionProbeStore::new(
        EventingProcessStore::new(in_mem_process_store(), event_sink),
        Arc::clone(&executor),
    ));
    let result_store = Arc::new(in_mem_process_result_store());
    let services = ProcessServices::new(Arc::clone(&process_store), Arc::clone(&result_store));
    let manager = services.background_manager(Arc::clone(&executor));
    let host = services.host().with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant-a", "user-a");

    let started = manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    assert_eq!(started.status, ProcessStatus::Running);

    let killed = host.kill(&scope, process_id).await.unwrap();
    assert_eq!(killed.status, ProcessStatus::Killed);
    timeout(Duration::from_millis(200), executor.wait_for_cancellation())
        .await
        .unwrap();

    timeout(
        Duration::from_millis(200),
        executor.wait_for_completion_attempt(),
    )
    .await
    .unwrap();
    timeout(
        Duration::from_millis(200),
        executor.wait_for_post_completion_status_probe(),
    )
    .await
    .unwrap();

    assert_eq!(
        host.status(&scope, process_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProcessStatus::Killed
    );
    let result = host.result(&scope, process_id).await.unwrap().unwrap();
    assert_eq!(result.status, ProcessStatus::Killed);
    assert_eq!(result.output, None);
    assert_eq!(host.output(&scope, process_id).await.unwrap(), None);
    assert_eq!(executor.cancellations.load(Ordering::SeqCst), 1);

    let recorded = events.events();
    let kinds = recorded.iter().map(|event| event.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::ProcessStarted,
            RuntimeEventKind::ProcessKilled
        ]
    );
    assert!(
        !kinds.contains(&RuntimeEventKind::ProcessCompleted),
        "late executor success must not emit a misleading completion event"
    );
}

struct SentinelSuccessExecutor;

#[async_trait]
impl ProcessExecutor for SentinelSuccessExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        assert_eq!(
            request.input,
            json!({
                "message": "PROCESS_INPUT_SENTINEL_3022 /tmp/private-process-path",
                "secret": "PROCESS_SECRET_SENTINEL_3022_sk_live_secret",
            })
        );
        Ok(ProcessExecutionResult {
            output: json!({"ok": true, "raw": "PROCESS_OUTPUT_SENTINEL_3022"}),
        })
    }
}

struct SentinelFailureExecutor;

#[async_trait]
impl ProcessExecutor for SentinelFailureExecutor {
    async fn execute(
        &self,
        _request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        Err(ProcessExecutionError::new(
            "PROCESS_ERROR_SENTINEL_3022 /tmp/private-process-error sk_live_secret",
        ))
    }
}

struct SuccessExecutor;

#[async_trait]
impl ProcessExecutor for SuccessExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        assert_eq!(request.input, json!({"message": "runtime payload"}));
        Ok(ProcessExecutionResult {
            output: json!({"ok": true}),
        })
    }
}

#[derive(Default)]
struct CancelThenLateSuccessExecutor {
    cancellations: AtomicUsize,
    completion_attempts: AtomicUsize,
    post_completion_status_probes: AtomicUsize,
    cancellation_notified: Notify,
    completion_attempt_notified: Notify,
    post_completion_status_probe_notified: Notify,
}

impl CancelThenLateSuccessExecutor {
    async fn wait_for_cancellation(&self) {
        loop {
            let notified = self.cancellation_notified.notified();
            if self.cancellations.load(Ordering::SeqCst) > 0 {
                return;
            }
            notified.await;
        }
    }

    async fn wait_for_completion_attempt(&self) {
        loop {
            let notified = self.completion_attempt_notified.notified();
            if self.completion_attempts.load(Ordering::SeqCst) > 0 {
                return;
            }
            notified.await;
        }
    }

    async fn wait_for_post_completion_status_probe(&self) {
        loop {
            let notified = self.post_completion_status_probe_notified.notified();
            if self.post_completion_status_probes.load(Ordering::SeqCst) > 0 {
                return;
            }
            notified.await;
        }
    }

    fn record_post_completion_status_probe(&self) {
        if self.completion_attempts.load(Ordering::SeqCst) > 0 {
            self.post_completion_status_probes
                .fetch_add(1, Ordering::SeqCst);
            self.post_completion_status_probe_notified.notify_waiters();
        }
    }
}

#[async_trait]
impl ProcessExecutor for CancelThenLateSuccessExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        request.cancellation.cancelled().await;
        self.cancellations.fetch_add(1, Ordering::SeqCst);
        self.cancellation_notified.notify_waiters();
        tokio::time::sleep(Duration::from_millis(25)).await;
        self.completion_attempts.fetch_add(1, Ordering::SeqCst);
        self.completion_attempt_notified.notify_waiters();
        Ok(ProcessExecutionResult {
            output: json!({"should_not_publish": true}),
        })
    }
}

struct PostCompletionProbeStore<S> {
    inner: S,
    probe: Arc<CancelThenLateSuccessExecutor>,
}

impl<S> PostCompletionProbeStore<S> {
    fn new(inner: S, probe: Arc<CancelThenLateSuccessExecutor>) -> Self {
        Self { inner, probe }
    }
}

#[async_trait]
impl<S> ProcessStore for PostCompletionProbeStore<S>
where
    S: ProcessStore,
{
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        self.inner.start(start).await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.inner.complete(scope, process_id).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        self.inner.fail(scope, process_id, error_kind).await
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.inner.kill(scope, process_id).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        let record = self.inner.get(scope, process_id).await?;
        self.probe.record_post_completion_status_probe();
        Ok(record)
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        self.inner.records_for_scope(scope).await
    }
}

fn process_start(
    process_id: ProcessId,
    invocation_id: InvocationId,
    scope: ResourceScope,
) -> ProcessStart {
    process_start_with_input(
        process_id,
        invocation_id,
        scope,
        json!({"message": "runtime payload"}),
    )
}

fn process_start_with_input(
    process_id: ProcessId,
    invocation_id: InvocationId,
    scope: ResourceScope,
    input: serde_json::Value,
) -> ProcessStart {
    ProcessStart {
        process_id,
        parent_process_id: None,
        invocation_id,
        scope,
        authenticated_actor_user_id: None,
        extension_id: ExtensionId::new("echo").unwrap(),
        capability_id: CapabilityId::new("echo.say").unwrap(),
        runtime: RuntimeKind::Wasm,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        estimated_resources: ResourceEstimate::default(),
        resource_reservation_id: None,
        authorized_continuation: None,
        input,
    }
}

fn read_directory_text(root: &std::path::Path) -> String {
    let mut output = String::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let entries = std::fs::read_dir(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                output.push_str(&std::fs::read_to_string(&path).unwrap_or_else(|err| {
                    panic!("failed to read {} as utf-8 text: {err}", path.display())
                }));
            }
        }
    }
    output
}

fn sample_scope(invocation_id: InvocationId, tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id,
    }
}

fn processes_test_fs() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").expect("alias"),
        VirtualPath::new("/engine/tenants/tenant1/users/user1/processes").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

fn in_mem_process_store() -> FilesystemProcessStore<InMemoryBackend> {
    FilesystemProcessStore::new(processes_test_fs())
}

fn in_mem_process_result_store() -> FilesystemProcessResultStore<InMemoryBackend> {
    FilesystemProcessResultStore::new(processes_test_fs())
}

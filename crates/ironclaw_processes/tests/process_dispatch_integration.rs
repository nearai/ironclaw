use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_events::{EventSink, InMemoryEventSink, RuntimeEventKind};
use ironclaw_host_api::*;
use ironclaw_processes::*;
use serde_json::json;
use tokio::{sync::Notify, time::timeout};

#[tokio::test]
async fn process_services_complete_background_process_through_process_host_and_eventing_store() {
    let events = InMemoryEventSink::new();
    let event_sink: Arc<dyn EventSink> = Arc::new(events.clone());
    let process_store = Arc::new(EventingProcessStore::new(
        InMemoryProcessStore::new(),
        event_sink,
    ));
    let result_store = Arc::new(InMemoryProcessResultStore::new());
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
    assert_eq!(result.output, Some(json!({"ok": true})));
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
async fn process_host_kill_preserves_terminal_state_and_suppresses_late_completion_event() {
    let events = InMemoryEventSink::new();
    let event_sink: Arc<dyn EventSink> = Arc::new(events.clone());
    let executor = Arc::new(CancelThenLateSuccessExecutor::default());
    let process_store = Arc::new(PostCompletionProbeStore::new(
        EventingProcessStore::new(InMemoryProcessStore::new(), event_sink),
        Arc::clone(&executor),
    ));
    let result_store = Arc::new(InMemoryProcessResultStore::new());
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
    ProcessStart {
        process_id,
        parent_process_id: None,
        invocation_id,
        scope,
        extension_id: ExtensionId::new("echo").unwrap(),
        capability_id: CapabilityId::new("echo.say").unwrap(),
        runtime: RuntimeKind::Wasm,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        estimated_resources: ResourceEstimate::default(),
        resource_reservation_id: None,
        input: json!({"message": "runtime payload"}),
    }
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

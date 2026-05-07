use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_processes::*;
use serde_json::json;
use tokio::{sync::Notify, time::timeout};

#[tokio::test]
async fn process_services_wire_background_results_to_host() {
    let services = ProcessServices::in_memory();
    let manager = services.background_manager(Arc::new(SuccessExecutor));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let host = services.host().with_poll_interval(Duration::from_millis(5));
    let result = host.await_result(&scope, process_id).await.unwrap();

    assert_eq!(result.status, ProcessStatus::Completed);
    assert_eq!(result.output, Some(json!({"ok": true})));
    assert_eq!(
        host.output(&scope, process_id).await.unwrap(),
        Some(json!({"ok": true}))
    );
    assert_eq!(
        services
            .process_store()
            .get(&scope, process_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProcessStatus::Completed
    );
}

#[tokio::test]
async fn process_services_share_cancellation_registry_between_host_and_manager() {
    let services = ProcessServices::in_memory();
    let executor = Arc::new(CancellationAwareExecutor::default());
    let manager = services.background_manager(executor.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let host = services.host().with_poll_interval(Duration::from_millis(5));
    host.kill(&scope, process_id).await.unwrap();

    timeout(Duration::from_millis(200), executor.wait_for_cancellation())
        .await
        .unwrap();
    assert_eq!(executor.cancellations.load(Ordering::SeqCst), 1);
    assert_eq!(
        host.result(&scope, process_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProcessStatus::Killed
    );
}

#[tokio::test]
async fn filesystem_process_services_store_output_refs() {
    let services = ProcessServices::filesystem(Arc::new(engine_filesystem()));
    let manager = services.background_manager(Arc::new(SuccessExecutor));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let host = services.host().with_poll_interval(Duration::from_millis(5));
    let result = host.await_result(&scope, process_id).await.unwrap();

    assert_eq!(result.status, ProcessStatus::Completed);
    assert_eq!(result.output, None);
    assert!(result.output_ref.is_some());
    assert_eq!(
        host.output(&scope, process_id).await.unwrap(),
        Some(json!({"ok": true}))
    );
}

#[tokio::test]
async fn background_manager_passes_spawn_mounts_and_reservation_to_executor() {
    let services = ProcessServices::in_memory();
    let executor = Arc::new(RecordingHandoffExecutor::default());
    let manager = services.background_manager(Arc::clone(&executor));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let mounts = mount_view(
        "/workspace",
        "/projects/project1",
        MountPermissions::read_only(),
    );
    let estimate = ResourceEstimate {
        process_count: Some(1),
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };
    let reservation_id = ResourceReservationId::new();
    let mut start = process_start(process_id, invocation_id, scope.clone());
    start.mounts = mounts.clone();
    start.estimated_resources = estimate.clone();
    start.resource_reservation_id = Some(reservation_id);

    manager.spawn(start).await.unwrap();

    let request = executor.wait_for_request().await;
    assert_eq!(request.process_id, process_id);
    assert_eq!(request.scope, scope);
    assert_eq!(request.mounts, mounts);
    let reservation = request
        .resource_reservation
        .expect("prepared reservation id from spawn must reach the executor");
    assert_eq!(reservation.id, reservation_id);
    assert_eq!(reservation.scope, request.scope);
    assert_eq!(reservation.estimate, estimate);
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
struct CancellationAwareExecutor {
    cancellations: AtomicUsize,
    notified: Notify,
}

#[derive(Default)]
struct RecordingHandoffExecutor {
    request: Mutex<Option<ProcessExecutionRequest>>,
    notified: Notify,
}

impl RecordingHandoffExecutor {
    async fn wait_for_request(&self) -> ProcessExecutionRequest {
        loop {
            let notified = self.notified.notified();
            if let Some(request) = self.request.lock().unwrap().clone() {
                return request;
            }
            notified.await;
        }
    }
}

#[async_trait]
impl ProcessExecutor for RecordingHandoffExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        *self.request.lock().unwrap() = Some(request);
        self.notified.notify_waiters();
        Ok(ProcessExecutionResult {
            output: json!({"ok": true}),
        })
    }
}

impl CancellationAwareExecutor {
    async fn wait_for_cancellation(&self) {
        loop {
            let notified = self.notified.notified();
            if self.cancellations.load(Ordering::SeqCst) > 0 {
                return;
            }
            notified.await;
        }
    }
}

#[async_trait]
impl ProcessExecutor for CancellationAwareExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        request.cancellation.cancelled().await;
        self.cancellations.fetch_add(1, Ordering::SeqCst);
        self.notified.notify_waiters();
        Ok(ProcessExecutionResult {
            output: json!({"cancelled": true}),
        })
    }
}

fn mount_view(alias: &str, target: &str, permissions: MountPermissions) -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new(alias).unwrap(),
        VirtualPath::new(target).unwrap(),
        permissions,
    )])
    .unwrap()
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

fn engine_filesystem() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn sample_scope(invocation_id: InvocationId, tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    }
}

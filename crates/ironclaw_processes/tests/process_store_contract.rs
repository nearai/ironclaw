use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_processes::*;

#[tokio::test]
async fn in_memory_process_store_starts_capability_process_record() {
    let store = InMemoryProcessStore::new();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    let record = store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    assert_eq!(record.process_id, process_id);
    assert_eq!(record.invocation_id, invocation_id);
    assert_eq!(record.scope, scope);
    assert_eq!(record.extension_id, ExtensionId::new("echo").unwrap());
    assert_eq!(record.capability_id, CapabilityId::new("echo.say").unwrap());
    assert_eq!(record.runtime, RuntimeKind::Wasm);
    assert_eq!(record.status, ProcessStatus::Running);
    assert_eq!(record.parent_process_id, None);
    assert_eq!(record.grants.grants.len(), 1);
    assert_eq!(record.resource_reservation_id, None);
}

#[tokio::test]
async fn in_memory_process_store_rejects_duplicate_process_id_in_same_tenant_user() {
    let store = InMemoryProcessStore::new();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let err = store
        .start(process_start(
            process_id,
            InvocationId::new(),
            scope.clone(),
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ProcessError::ProcessAlreadyExists { process_id: id } if id == process_id
    ));
}

#[tokio::test]
async fn process_store_hides_records_from_other_tenants_and_users() {
    let store = InMemoryProcessStore::new();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let tenant_a = sample_scope(invocation_id, "tenant1", "user1");
    let tenant_b = sample_scope(invocation_id, "tenant2", "user1");
    let user_b = sample_scope(invocation_id, "tenant1", "user2");
    store
        .start(process_start(process_id, invocation_id, tenant_a.clone()))
        .await
        .unwrap();

    assert!(store.get(&tenant_b, process_id).await.unwrap().is_none());
    assert!(store.get(&user_b, process_id).await.unwrap().is_none());
    assert_eq!(
        store.records_for_scope(&tenant_b).await.unwrap(),
        Vec::new()
    );
    assert_eq!(store.records_for_scope(&user_b).await.unwrap(), Vec::new());
    assert!(matches!(
        store.kill(&tenant_b, process_id).await.unwrap_err(),
        ProcessError::UnknownProcess { .. }
    ));
}

#[tokio::test]
async fn process_store_rejects_terminal_status_overwrite() {
    let store = InMemoryProcessStore::new();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    store.kill(&scope, process_id).await.unwrap();

    let err = store.complete(&scope, process_id).await.unwrap_err();

    assert!(matches!(
        err,
        ProcessError::InvalidTransition {
            process_id: id,
            from: ProcessStatus::Killed,
            to: ProcessStatus::Completed,
        } if id == process_id
    ));
    assert_eq!(
        store.get(&scope, process_id).await.unwrap().unwrap().status,
        ProcessStatus::Killed
    );
}

#[tokio::test]
async fn background_process_manager_marks_process_completed_after_executor_success() {
    let store = Arc::new(InMemoryProcessStore::new());
    let executor = Arc::new(CountingExecutor::success());
    let manager = BackgroundProcessManager::new(store.clone(), executor.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    let started = manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    assert_eq!(started.status, ProcessStatus::Running);
    wait_for_status(store.as_ref(), &scope, process_id, ProcessStatus::Completed).await;
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn background_process_manager_marks_process_failed_after_executor_error() {
    let store = Arc::new(InMemoryProcessStore::new());
    let executor = Arc::new(CountingExecutor::failure("RuntimeDispatch"));
    let manager = BackgroundProcessManager::new(store.clone(), executor);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    wait_for_status(store.as_ref(), &scope, process_id, ProcessStatus::Failed).await;
    assert_eq!(
        store
            .get(&scope, process_id)
            .await
            .unwrap()
            .unwrap()
            .error_kind
            .as_deref(),
        Some("RuntimeDispatch")
    );
}

#[tokio::test]
async fn background_process_manager_does_not_overwrite_killed_process_on_late_success() {
    let store = Arc::new(InMemoryProcessStore::new());
    let executor = Arc::new(CountingExecutor::delayed_success(Duration::from_millis(25)));
    let manager = BackgroundProcessManager::new(store.clone(), executor);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    store.kill(&scope, process_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;

    assert_eq!(
        store.get(&scope, process_id).await.unwrap().unwrap().status,
        ProcessStatus::Killed
    );
}

#[tokio::test]
async fn background_process_manager_can_use_owned_filesystem_store() {
    let filesystem = Arc::new(engine_filesystem());
    let store = Arc::new(FilesystemProcessStore::from_arc(filesystem));
    let executor = Arc::new(CountingExecutor::success());
    let manager = BackgroundProcessManager::new(store.clone(), executor);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    wait_for_status(store.as_ref(), &scope, process_id, ProcessStatus::Completed).await;
}

#[tokio::test]
async fn filesystem_process_store_rejects_terminal_status_overwrite() {
    let fs = engine_filesystem();
    let store = FilesystemProcessStore::new(&fs);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    store.kill(&scope, process_id).await.unwrap();

    let err = store.complete(&scope, process_id).await.unwrap_err();

    assert!(matches!(
        err,
        ProcessError::InvalidTransition {
            process_id: id,
            from: ProcessStatus::Killed,
            to: ProcessStatus::Completed,
        } if id == process_id
    ));
    assert_eq!(
        store.get(&scope, process_id).await.unwrap().unwrap().status,
        ProcessStatus::Killed
    );
}

#[tokio::test]
async fn filesystem_process_store_persists_under_tenant_user_engine_processes() {
    let fs = engine_filesystem();
    let store = FilesystemProcessStore::new(&fs);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    store.complete(&scope, process_id).await.unwrap();

    let reloaded = FilesystemProcessStore::new(&fs)
        .get(&scope, process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.status, ProcessStatus::Completed);
    assert_eq!(
        FilesystemProcessStore::new(&fs)
            .records_for_scope(&scope)
            .await
            .unwrap()
            .len(),
        1
    );
}

struct CountingExecutor {
    result: Result<(), &'static str>,
    delay: Duration,
    calls: AtomicUsize,
}

impl CountingExecutor {
    fn success() -> Self {
        Self {
            result: Ok(()),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        }
    }

    fn delayed_success(delay: Duration) -> Self {
        Self {
            result: Ok(()),
            delay,
            calls: AtomicUsize::new(0),
        }
    }

    fn failure(kind: &'static str) -> Self {
        Self {
            result: Err(kind),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ProcessExecutor for CountingExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            request.capability_id,
            CapabilityId::new("echo.say").unwrap()
        );
        assert_eq!(
            request.input,
            serde_json::json!({"message": "runtime payload"})
        );
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        match self.result {
            Ok(()) => Ok(ProcessExecutionResult {
                output: serde_json::json!({"ok": true}),
            }),
            Err(kind) => Err(ProcessExecutionError::new(kind)),
        }
    }
}

async fn wait_for_status<S>(
    store: &S,
    scope: &ResourceScope,
    process_id: ProcessId,
    expected: ProcessStatus,
) where
    S: ProcessStore + ?Sized,
{
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let record = store.get(scope, process_id).await.unwrap().unwrap();
        if record.status == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "process {process_id} did not reach {expected:?}; last status was {:?}",
            record.status
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
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
        grants: CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new("echo.say").unwrap(),
                grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
                issued_by: Principal::System,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        },
        mounts: MountView::default(),
        estimated_resources: ResourceEstimate::default(),
        resource_reservation_id: None,
        input: serde_json::json!({"message": "runtime payload"}),
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
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    }
}

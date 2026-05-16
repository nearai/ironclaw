use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_host_api::*;
use ironclaw_processes::*;
use serde_json::json;
use tokio::time::timeout;

#[tokio::test]
async fn process_host_status_reads_scoped_process_record() {
    let store = InMemoryProcessStore::new();
    let host = ProcessHost::new(&store);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let other_scope = sample_scope(invocation_id, "tenant2", "user1");

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let record = host.status(&scope, process_id).await.unwrap().unwrap();
    assert_eq!(record.process_id, process_id);
    assert_eq!(record.status, ProcessStatus::Running);
    assert!(
        host.status(&other_scope, process_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn process_host_kill_transitions_running_process() {
    let store = InMemoryProcessStore::new();
    let host = ProcessHost::new(&store);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let killed = host.kill(&scope, process_id).await.unwrap();

    assert_eq!(killed.status, ProcessStatus::Killed);
    assert_eq!(
        host.status(&scope, process_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProcessStatus::Killed
    );
}

#[tokio::test]
async fn process_host_await_process_returns_terminal_exit_after_background_completion() {
    let store = Arc::new(InMemoryProcessStore::new());
    let manager = BackgroundProcessManager::new(store.clone(), Arc::new(DelayedSuccessExecutor));
    let host = ProcessHost::new(store.as_ref()).with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let exit = host.await_process(&scope, process_id).await.unwrap();

    assert_eq!(exit.process_id, process_id);
    assert_eq!(exit.status, ProcessStatus::Completed);
    assert_eq!(exit.error_kind, None);
}

#[tokio::test]
async fn process_host_kill_retries_result_side_effect_for_already_killed_process() {
    let store = InMemoryProcessStore::new();
    let result_store = Arc::new(FailOnceKillResultStore::new());
    let host = ProcessHost::new(&store).with_result_store(result_store.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let first_err = host.kill(&scope, process_id).await.unwrap_err();
    assert!(matches!(
        first_err,
        ProcessError::ProcessResultUnavailable { process_id: id } if id == process_id
    ));
    assert_eq!(
        host.status(&scope, process_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProcessStatus::Killed
    );
    assert!(
        result_store
            .get(&scope, process_id)
            .await
            .unwrap()
            .is_none()
    );

    let repaired = host.kill(&scope, process_id).await.unwrap();

    assert_eq!(repaired.status, ProcessStatus::Killed);
    assert_eq!(
        result_store
            .get(&scope, process_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProcessStatus::Killed
    );
}

#[tokio::test]
async fn process_host_await_process_returns_terminal_exit_for_already_killed_process() {
    let store = InMemoryProcessStore::new();
    let host = ProcessHost::new(&store);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    store.kill(&scope, process_id).await.unwrap();

    let exit = host.await_process(&scope, process_id).await.unwrap();

    assert_eq!(exit.status, ProcessStatus::Killed);
}

#[tokio::test]
async fn process_host_await_process_fails_closed_for_unknown_or_other_scope_process() {
    let store = InMemoryProcessStore::new();
    let host = ProcessHost::new(&store);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let other_scope = sample_scope(invocation_id, "tenant2", "user1");

    let missing = host.await_process(&scope, process_id).await.unwrap_err();
    assert!(matches!(missing, ProcessError::UnknownProcess { process_id: id } if id == process_id));

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let hidden = host
        .await_process(&other_scope, process_id)
        .await
        .unwrap_err();
    assert!(matches!(hidden, ProcessError::UnknownProcess { process_id: id } if id == process_id));
}

#[tokio::test]
async fn process_host_subscribe_emits_initial_and_terminal_records() {
    let store = InMemoryProcessStore::new();
    let host = ProcessHost::new(&store).with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let mut subscription = host.subscribe(&scope, process_id).await.unwrap();
    let initial = subscription.next().await.unwrap().unwrap();
    assert_eq!(initial.status, ProcessStatus::Running);

    store.complete(&scope, process_id).await.unwrap();

    let terminal = subscription.next().await.unwrap().unwrap();
    assert_eq!(terminal.status, ProcessStatus::Completed);
    assert_eq!(subscription.next().await.unwrap(), None);
}

#[tokio::test]
async fn process_host_subscribe_tracks_background_completion() {
    let store = Arc::new(InMemoryProcessStore::new());
    let manager = BackgroundProcessManager::new(store.clone(), Arc::new(DelayedSuccessExecutor));
    let host = ProcessHost::new(store.as_ref()).with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let mut subscription = host.subscribe(&scope, process_id).await.unwrap();
    assert_eq!(
        subscription.next().await.unwrap().unwrap().status,
        ProcessStatus::Running
    );

    let terminal = timeout(Duration::from_millis(200), subscription.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(terminal.status, ProcessStatus::Completed);
}

#[tokio::test]
async fn process_host_subscribe_closes_after_initial_terminal_record() {
    let store = InMemoryProcessStore::new();
    let host = ProcessHost::new(&store).with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    store.kill(&scope, process_id).await.unwrap();

    let mut subscription = host.subscribe(&scope, process_id).await.unwrap();

    assert_eq!(
        subscription.next().await.unwrap().unwrap().status,
        ProcessStatus::Killed
    );
    assert_eq!(subscription.next().await.unwrap(), None);
}

#[tokio::test]
async fn process_host_subscribe_fails_closed_for_unknown_or_other_scope_process() {
    let store = InMemoryProcessStore::new();
    let host = ProcessHost::new(&store);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let other_scope = sample_scope(invocation_id, "tenant2", "user1");

    let missing = host.subscribe(&scope, process_id).await.unwrap_err();
    assert!(matches!(missing, ProcessError::UnknownProcess { process_id: id } if id == process_id));

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let hidden = host.subscribe(&other_scope, process_id).await.unwrap_err();
    assert!(matches!(hidden, ProcessError::UnknownProcess { process_id: id } if id == process_id));
}

struct FailOnceKillResultStore {
    inner: InMemoryProcessResultStore,
    fail_next_kill: AtomicBool,
}

impl FailOnceKillResultStore {
    fn new() -> Self {
        Self {
            inner: InMemoryProcessResultStore::new(),
            fail_next_kill: AtomicBool::new(true),
        }
    }
}

#[async_trait]
impl ProcessResultStore for FailOnceKillResultStore {
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: serde_json::Value,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.inner.complete(scope, process_id, output).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.inner.fail(scope, process_id, error_kind).await
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError> {
        if self.fail_next_kill.swap(false, Ordering::SeqCst) {
            return Err(ProcessError::ProcessResultUnavailable { process_id });
        }
        self.inner.kill(scope, process_id).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError> {
        self.inner.get(scope, process_id).await
    }
}

struct DelayedSuccessExecutor;

#[async_trait]
impl ProcessExecutor for DelayedSuccessExecutor {
    async fn execute(
        &self,
        _request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(ProcessExecutionResult {
            output: json!({"ok": true}),
        })
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
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: Some(MissionId::new("mission1").unwrap()),
        thread_id: Some(ThreadId::new("thread1").unwrap()),
        invocation_id,
    }
}

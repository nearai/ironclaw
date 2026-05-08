use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_filesystem::{
    DirEntry, FileStat, FilesystemError, FilesystemOperation, LocalFilesystem, RootFilesystem,
};
use ironclaw_host_api::*;
use ironclaw_processes::*;
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceError, ResourceGovernor, ResourceLimits,
    ResourceTally,
};
use tokio::{sync::Notify, time::timeout};

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
async fn in_memory_process_store_rejects_duplicate_process_id_in_same_resource_scope() {
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
async fn process_store_hides_records_from_other_resource_scopes() {
    let store = InMemoryProcessStore::new();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let tenant_a = sample_scope(invocation_id, "tenant1", "user1");
    let tenant_b = sample_scope(invocation_id, "tenant2", "user1");
    let user_b = sample_scope(invocation_id, "tenant1", "user2");
    let project_b = sample_scope_with_project(invocation_id, "tenant1", "user1", "project2");
    store
        .start(process_start(process_id, invocation_id, tenant_a.clone()))
        .await
        .unwrap();

    assert!(store.get(&tenant_b, process_id).await.unwrap().is_none());
    assert!(store.get(&user_b, process_id).await.unwrap().is_none());
    assert!(store.get(&project_b, process_id).await.unwrap().is_none());
    assert_eq!(
        store.records_for_scope(&tenant_b).await.unwrap(),
        Vec::new()
    );
    assert_eq!(store.records_for_scope(&user_b).await.unwrap(), Vec::new());
    assert_eq!(
        store.records_for_scope(&project_b).await.unwrap(),
        Vec::new()
    );
    assert!(matches!(
        store.kill(&tenant_b, process_id).await.unwrap_err(),
        ProcessError::UnknownProcess { .. }
    ));
    assert!(matches!(
        store.kill(&project_b, process_id).await.unwrap_err(),
        ProcessError::UnknownProcess { .. }
    ));
}

#[tokio::test]
async fn process_store_hides_records_from_other_agents() {
    let store = InMemoryProcessStore::new();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let agent_a = sample_scope_with_agent(invocation_id, "tenant1", "user1", Some("agent-a"));
    let agent_b = sample_scope_with_agent(invocation_id, "tenant1", "user1", Some("agent-b"));
    store
        .start(process_start(process_id, invocation_id, agent_a.clone()))
        .await
        .unwrap();

    assert!(store.get(&agent_b, process_id).await.unwrap().is_none());
    assert_eq!(store.records_for_scope(&agent_b).await.unwrap(), Vec::new());
    assert!(matches!(
        store.kill(&agent_b, process_id).await.unwrap_err(),
        ProcessError::UnknownProcess { .. }
    ));
}

#[tokio::test]
async fn process_result_store_hides_records_from_other_resource_scopes() {
    let store = InMemoryProcessResultStore::new();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let agent_a = sample_scope_with_agent(invocation_id, "tenant1", "user1", Some("agent-a"));
    let agent_b = sample_scope_with_agent(invocation_id, "tenant1", "user1", Some("agent-b"));
    let project_b = sample_scope_with_agent_and_project(
        invocation_id,
        "tenant1",
        "user1",
        Some("agent-a"),
        "project2",
    );

    store
        .complete(&agent_a, process_id, serde_json::json!({"ok": true}))
        .await
        .unwrap();

    assert!(store.get(&agent_b, process_id).await.unwrap().is_none());
    assert!(store.output(&agent_b, process_id).await.unwrap().is_none());
    assert!(store.get(&project_b, process_id).await.unwrap().is_none());
    assert!(
        store
            .output(&project_b, process_id)
            .await
            .unwrap()
            .is_none()
    );
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
async fn background_process_manager_stores_success_output_result() {
    let store = Arc::new(InMemoryProcessStore::new());
    let result_store = Arc::new(InMemoryProcessResultStore::new());
    let executor = Arc::new(CountingExecutor::success());
    let manager = BackgroundProcessManager::new(store.clone(), executor)
        .with_result_store(result_store.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let host = ProcessHost::new(store.as_ref())
        .with_result_store(result_store.clone())
        .with_poll_interval(Duration::from_millis(5));

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    wait_for_status(store.as_ref(), &scope, process_id, ProcessStatus::Completed).await;
    let result = host.result(&scope, process_id).await.unwrap().unwrap();
    assert_eq!(result.process_id, process_id);
    assert_eq!(result.scope, scope);
    assert_eq!(result.status, ProcessStatus::Completed);
    assert_eq!(result.output, Some(serde_json::json!({"ok": true})));
    assert_eq!(result.output_ref, None);
    assert_eq!(result.error_kind, None);
}

#[tokio::test]
async fn background_process_manager_stores_failure_error_result() {
    let store = Arc::new(InMemoryProcessStore::new());
    let result_store = Arc::new(InMemoryProcessResultStore::new());
    let manager = BackgroundProcessManager::new(
        store.clone(),
        Arc::new(CountingExecutor::failure("runtime_dispatch")),
    )
    .with_result_store(result_store.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let host = ProcessHost::new(store.as_ref()).with_result_store(result_store.clone());

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    wait_for_status(store.as_ref(), &scope, process_id, ProcessStatus::Failed).await;
    let result = host.result(&scope, process_id).await.unwrap().unwrap();
    assert_eq!(result.status, ProcessStatus::Failed);
    assert_eq!(result.output, None);
    assert_eq!(result.output_ref, None);
    assert_eq!(result.error_kind.as_deref(), Some("runtime_dispatch"));
}

#[tokio::test]
async fn background_process_manager_reports_result_store_complete_failure_and_keeps_running_status()
{
    let store = Arc::new(InMemoryProcessStore::new());
    let result_store = Arc::new(FailingProcessResultStore::default());
    let captured = Arc::new(Mutex::new(Vec::<(BackgroundFailureStage, ProcessId)>::new()));
    let handler_captured = Arc::clone(&captured);
    let executor = Arc::new(CountingExecutor::success());
    let manager = BackgroundProcessManager::new(store.clone(), executor)
        .with_result_store(result_store.clone())
        .with_error_handler(move |failure| {
            handler_captured
                .lock()
                .unwrap()
                .push((failure.stage, failure.process_id));
        });
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    // Wait for the spawned task to attempt the result-store write.
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if !captured.lock().unwrap().is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "error handler was not invoked within deadline"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let captured_failures = captured.lock().unwrap().clone();
    assert_eq!(captured_failures.len(), 1);
    assert_eq!(
        captured_failures[0].0,
        BackgroundFailureStage::ResultStoreComplete
    );
    assert_eq!(captured_failures[0].1, process_id);
    assert_eq!(result_store.failures(), vec!["complete"]);

    // Lifecycle status must remain Running because result-first ordering
    // means status is not promoted when the result write fails.
    let record = store.get(&scope, process_id).await.unwrap().unwrap();
    assert_eq!(record.status, ProcessStatus::Running);
}

#[tokio::test]
async fn background_process_manager_marks_process_failed_after_executor_error() {
    let store = Arc::new(InMemoryProcessStore::new());
    let executor = Arc::new(CountingExecutor::failure("runtime_dispatch"));
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
        Some("runtime_dispatch")
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
async fn process_host_kill_signals_background_executor_cancellation() {
    let store = Arc::new(InMemoryProcessStore::new());
    let cancellation_registry = Arc::new(ProcessCancellationRegistry::new());
    let executor = Arc::new(CancellationAwareExecutor::default());
    let manager = BackgroundProcessManager::new(store.clone(), executor.clone())
        .with_cancellation_registry(cancellation_registry.clone());
    let host = ProcessHost::new(store.as_ref())
        .with_cancellation_registry(cancellation_registry)
        .with_poll_interval(Duration::from_millis(5));
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

    let killed = host.kill(&scope, process_id).await.unwrap();
    assert_eq!(killed.status, ProcessStatus::Killed);
    timeout(Duration::from_millis(200), executor.wait_for_cancellation())
        .await
        .unwrap();

    assert_eq!(
        subscription.next().await.unwrap().unwrap().status,
        ProcessStatus::Killed
    );
    assert_eq!(subscription.next().await.unwrap(), None);
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(
        store.get(&scope, process_id).await.unwrap().unwrap().status,
        ProcessStatus::Killed
    );
}

#[tokio::test]
async fn process_host_kill_does_not_cancel_other_tenant_process() {
    let store = Arc::new(InMemoryProcessStore::new());
    let cancellation_registry = Arc::new(ProcessCancellationRegistry::new());
    let executor = Arc::new(CancellationAwareExecutor::default());
    let manager = BackgroundProcessManager::new(store.clone(), executor.clone())
        .with_cancellation_registry(cancellation_registry.clone());
    let host = ProcessHost::new(store.as_ref())
        .with_cancellation_registry(cancellation_registry)
        .with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let owner_scope = sample_scope(invocation_id, "tenant1", "user1");
    let other_scope = sample_scope(invocation_id, "tenant2", "user1");

    manager
        .spawn(process_start(
            process_id,
            invocation_id,
            owner_scope.clone(),
        ))
        .await
        .unwrap();

    let err = host.kill(&other_scope, process_id).await.unwrap_err();
    assert!(matches!(err, ProcessError::UnknownProcess { process_id: id } if id == process_id));
    assert!(
        timeout(Duration::from_millis(30), executor.wait_for_cancellation())
            .await
            .is_err(),
        "cross-tenant kill must not signal the owner's cancellation token"
    );
    assert_eq!(
        store
            .get(&owner_scope, process_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProcessStatus::Running
    );

    host.kill(&owner_scope, process_id).await.unwrap();
    timeout(Duration::from_millis(200), executor.wait_for_cancellation())
        .await
        .unwrap();
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
async fn eventing_process_store_emits_started_and_killed_events() {
    let events = Arc::new(InMemoryEventSink::new());
    let store = EventingProcessStore::new(InMemoryProcessStore::new(), events.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    store.kill(&scope, process_id).await.unwrap();

    let emitted = events.events();
    assert_eq!(emitted.len(), 2);
    assert_eq!(emitted[0].kind, RuntimeEventKind::ProcessStarted);
    assert_eq!(emitted[0].process_id, Some(process_id));
    assert_eq!(emitted[0].scope, scope);
    assert_eq!(emitted[0].provider, Some(ExtensionId::new("echo").unwrap()));
    assert_eq!(emitted[0].runtime, Some(RuntimeKind::Wasm));
    assert_eq!(emitted[1].kind, RuntimeEventKind::ProcessKilled);
    assert_eq!(emitted[1].process_id, Some(process_id));
}

#[tokio::test]
async fn background_process_manager_emits_completed_and_failed_events() {
    let success_events = Arc::new(InMemoryEventSink::new());
    let success_store = Arc::new(EventingProcessStore::new(
        InMemoryProcessStore::new(),
        success_events.clone(),
    ));
    let success_manager =
        BackgroundProcessManager::new(success_store.clone(), Arc::new(CountingExecutor::success()));
    let success_invocation_id = InvocationId::new();
    let success_process_id = ProcessId::new();
    let success_scope = sample_scope(success_invocation_id, "tenant1", "user1");

    success_manager
        .spawn(process_start(
            success_process_id,
            success_invocation_id,
            success_scope,
        ))
        .await
        .unwrap();
    wait_for_event_count(success_events.as_ref(), 2).await;
    assert_eq!(
        success_events.events()[0].kind,
        RuntimeEventKind::ProcessStarted
    );
    assert_eq!(
        success_events.events()[1].kind,
        RuntimeEventKind::ProcessCompleted
    );
    assert_eq!(
        success_events.events()[1].process_id,
        Some(success_process_id)
    );

    let failure_events = Arc::new(InMemoryEventSink::new());
    let failure_store = Arc::new(EventingProcessStore::new(
        InMemoryProcessStore::new(),
        failure_events.clone(),
    ));
    let failure_manager = BackgroundProcessManager::new(
        failure_store,
        Arc::new(CountingExecutor::failure("runtime_dispatch")),
    );
    let failure_invocation_id = InvocationId::new();
    let failure_process_id = ProcessId::new();
    let failure_scope = sample_scope(failure_invocation_id, "tenant1", "user1");

    failure_manager
        .spawn(process_start(
            failure_process_id,
            failure_invocation_id,
            failure_scope,
        ))
        .await
        .unwrap();
    wait_for_event_count(failure_events.as_ref(), 2).await;
    assert_eq!(
        failure_events.events()[0].kind,
        RuntimeEventKind::ProcessStarted
    );
    assert_eq!(
        failure_events.events()[1].kind,
        RuntimeEventKind::ProcessFailed
    );
    assert_eq!(
        failure_events.events()[1].process_id,
        Some(failure_process_id)
    );
    assert_eq!(
        failure_events.events()[1].error_kind.as_deref(),
        Some("runtime_dispatch")
    );
}

#[tokio::test]
async fn background_process_manager_does_not_emit_completed_after_kill() {
    let events = Arc::new(InMemoryEventSink::new());
    let store = Arc::new(EventingProcessStore::new(
        InMemoryProcessStore::new(),
        events.clone(),
    ));
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

    let kinds = events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::ProcessStarted,
            RuntimeEventKind::ProcessKilled
        ]
    );
}

#[tokio::test]
async fn resource_managed_store_reserves_and_records_reservation_id() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let store = ResourceManagedProcessStore::new(InMemoryProcessStore::new(), governor.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    let record = store
        .start(process_start_with_estimate(
            process_id,
            invocation_id,
            scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap();

    assert!(record.resource_reservation_id.is_some());
    assert_eq!(
        store.get(&scope, process_id).await.unwrap().unwrap(),
        record
    );
    let tenant = ResourceAccount::tenant(scope.tenant_id.clone());
    let reserved = governor.reserved_for(&tenant);
    assert_eq!(reserved.process_count, 1);
    assert_eq!(reserved.concurrency_slots, 1);
}

#[tokio::test]
async fn resource_managed_store_denies_before_process_record_creation() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    governor.set_limit(
        ResourceAccount::tenant(scope.tenant_id.clone()),
        ResourceLimits {
            max_process_count: Some(0),
            ..ResourceLimits::default()
        },
    );
    let store = ResourceManagedProcessStore::new(InMemoryProcessStore::new(), governor.clone());

    let err = store
        .start(process_start_with_estimate(
            process_id,
            invocation_id,
            scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ProcessError::Resource(ResourceError::LimitExceeded(_))
    ));
    assert!(store.get(&scope, process_id).await.unwrap().is_none());
    assert_eq!(
        governor
            .reserved_for(&ResourceAccount::tenant(scope.tenant_id.clone()))
            .process_count,
        0
    );
}

#[tokio::test]
async fn resource_managed_store_rejects_caller_supplied_reservation_id() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let store = ResourceManagedProcessStore::new(InMemoryProcessStore::new(), governor.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let mut start = process_start(process_id, invocation_id, scope.clone());
    start.resource_reservation_id = Some(ResourceReservationId::new());

    let err = store.start(start).await.unwrap_err();

    assert!(matches!(
        err,
        ProcessError::ResourceReservationAlreadyAssigned {
            process_id: id,
            ..
        } if id == process_id
    ));
    assert!(store.get(&scope, process_id).await.unwrap().is_none());
    assert_eq!(
        governor.reserved_for(&ResourceAccount::tenant(scope.tenant_id.clone())),
        ResourceTally::default()
    );
}

#[tokio::test]
async fn resource_managed_store_releases_when_inner_store_drops_reservation_id() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let store = ResourceManagedProcessStore::new(ReservationDroppingStore, governor.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    let err = store
        .start(process_start_with_estimate(
            process_id,
            invocation_id,
            scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ProcessError::ResourceReservationMismatch {
            process_id: id,
            actual: None,
            ..
        } if id == process_id
    ));
    assert_eq!(
        governor
            .reserved_for(&ResourceAccount::tenant(scope.tenant_id.clone()))
            .process_count,
        0
    );
}

#[tokio::test]
async fn resource_managed_store_preserves_mismatch_when_reconcile_cleanup_fails() {
    let governor = Arc::new(ReconcileFailingGovernor::default());
    let store =
        ResourceManagedProcessStore::new(CompletionReservationDroppingStore::default(), governor);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(process_start_with_estimate(
            process_id,
            invocation_id,
            scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap();

    let err = store.complete(&scope, process_id).await.unwrap_err();

    assert!(matches!(
        err,
        ProcessError::ResourceCleanupFailed { original, cleanup: ResourceError::UnknownReservation { .. } }
            if matches!(*original, ProcessError::ResourceReservationMismatch { process_id: id, .. } if id == process_id)
    ));
}

#[tokio::test]
async fn resource_managed_store_preserves_original_error_when_cleanup_fails() {
    let governor = Arc::new(ReleaseFailingGovernor::default());
    let inner = InMemoryProcessStore::new();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    inner
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    let store = ResourceManagedProcessStore::new(inner, governor);

    let err = store
        .start(process_start_with_estimate(
            process_id,
            InvocationId::new(),
            scope,
            process_estimate(),
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ProcessError::ResourceCleanupFailed { original, cleanup: ResourceError::UnknownReservation { .. } }
            if matches!(*original, ProcessError::ProcessAlreadyExists { process_id: id } if id == process_id)
    ));
}

#[tokio::test]
async fn resource_managed_store_releases_reservation_when_inner_start_fails() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let inner = InMemoryProcessStore::new();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    inner
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    let store = ResourceManagedProcessStore::new(inner, governor.clone());

    let err = store
        .start(process_start_with_estimate(
            process_id,
            InvocationId::new(),
            scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, ProcessError::ProcessAlreadyExists { .. }));
    assert_eq!(
        governor
            .reserved_for(&ResourceAccount::tenant(scope.tenant_id.clone()))
            .process_count,
        0
    );
}

#[tokio::test]
async fn resource_managed_store_does_not_reconcile_unowned_process_reservation_on_complete() {
    assert_unowned_process_reservation_rejected(UnownedTransition::Complete).await;
}

#[tokio::test]
async fn resource_managed_store_does_not_release_unowned_process_reservation_on_fail() {
    assert_unowned_process_reservation_rejected(UnownedTransition::Fail).await;
}

#[tokio::test]
async fn resource_managed_store_does_not_release_unowned_process_reservation_on_kill() {
    assert_unowned_process_reservation_rejected(UnownedTransition::Kill).await;
}

#[tokio::test]
async fn resource_managed_store_reconciles_on_complete_and_releases_on_failure_or_kill() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let completion_usage = ResourceUsage {
        process_count: 1,
        output_tokens: 7,
        ..ResourceUsage::default()
    };
    let store = ResourceManagedProcessStore::new(InMemoryProcessStore::new(), governor.clone())
        .with_completion_usage(completion_usage);
    let complete_invocation_id = InvocationId::new();
    let complete_process_id = ProcessId::new();
    let complete_scope = sample_scope(complete_invocation_id, "tenant1", "user1");
    store
        .start(process_start_with_estimate(
            complete_process_id,
            complete_invocation_id,
            complete_scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap();
    store
        .complete(&complete_scope, complete_process_id)
        .await
        .unwrap();
    let tenant = ResourceAccount::tenant(complete_scope.tenant_id.clone());
    assert_eq!(governor.reserved_for(&tenant).process_count, 0);
    assert_eq!(governor.usage_for(&tenant).process_count, 1);
    assert_eq!(governor.usage_for(&tenant).output_tokens, 7);

    let fail_invocation_id = InvocationId::new();
    let fail_process_id = ProcessId::new();
    let fail_scope = sample_scope(fail_invocation_id, "tenant1", "user1");
    store
        .start(process_start_with_estimate(
            fail_process_id,
            fail_invocation_id,
            fail_scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap();
    store
        .fail(&fail_scope, fail_process_id, "RuntimeDispatch".to_string())
        .await
        .unwrap();
    assert_eq!(governor.reserved_for(&tenant).process_count, 0);
    assert_eq!(governor.usage_for(&tenant).process_count, 1);

    let kill_invocation_id = InvocationId::new();
    let kill_process_id = ProcessId::new();
    let kill_scope = sample_scope(kill_invocation_id, "tenant1", "user1");
    store
        .start(process_start_with_estimate(
            kill_process_id,
            kill_invocation_id,
            kill_scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap();
    store.kill(&kill_scope, kill_process_id).await.unwrap();
    assert_eq!(governor.reserved_for(&tenant).process_count, 0);
    assert_eq!(governor.usage_for(&tenant).process_count, 1);
}

#[tokio::test]
async fn background_process_manager_releases_process_reservation_after_executor_panic() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let store = Arc::new(ResourceManagedProcessStore::new(
        InMemoryProcessStore::new(),
        governor.clone(),
    ));
    let manager = BackgroundProcessManager::new(store.clone(), Arc::new(PanicExecutor));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let tenant = ResourceAccount::tenant(scope.tenant_id.clone());

    manager
        .spawn(process_start_with_estimate(
            process_id,
            invocation_id,
            scope.clone(),
            process_estimate(),
        ))
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
        Some("runtime_panic")
    );
    assert_eq!(governor.reserved_for(&tenant), ResourceTally::default());
    assert_eq!(governor.usage_for(&tenant), ResourceTally::default());
}

#[tokio::test]
async fn background_process_manager_cleans_up_process_resource_reservations() {
    let success_governor = Arc::new(InMemoryResourceGovernor::new());
    let success_store = Arc::new(
        ResourceManagedProcessStore::new(InMemoryProcessStore::new(), success_governor.clone())
            .with_completion_usage(ResourceUsage {
                process_count: 1,
                ..ResourceUsage::default()
            }),
    );
    let success_manager =
        BackgroundProcessManager::new(success_store.clone(), Arc::new(CountingExecutor::success()));
    let success_invocation_id = InvocationId::new();
    let success_process_id = ProcessId::new();
    let success_scope = sample_scope(success_invocation_id, "tenant1", "user1");
    success_manager
        .spawn(process_start_with_estimate(
            success_process_id,
            success_invocation_id,
            success_scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap();
    wait_for_status(
        success_store.as_ref(),
        &success_scope,
        success_process_id,
        ProcessStatus::Completed,
    )
    .await;
    let success_tenant = ResourceAccount::tenant(success_scope.tenant_id.clone());
    assert_eq!(
        success_governor.reserved_for(&success_tenant).process_count,
        0
    );
    assert_eq!(success_governor.usage_for(&success_tenant).process_count, 1);

    let failure_governor = Arc::new(InMemoryResourceGovernor::new());
    let failure_store = Arc::new(ResourceManagedProcessStore::new(
        InMemoryProcessStore::new(),
        failure_governor.clone(),
    ));
    let failure_manager = BackgroundProcessManager::new(
        failure_store.clone(),
        Arc::new(CountingExecutor::failure("runtime_dispatch")),
    );
    let failure_invocation_id = InvocationId::new();
    let failure_process_id = ProcessId::new();
    let failure_scope = sample_scope(failure_invocation_id, "tenant1", "user1");
    failure_manager
        .spawn(process_start_with_estimate(
            failure_process_id,
            failure_invocation_id,
            failure_scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap();
    wait_for_status(
        failure_store.as_ref(),
        &failure_scope,
        failure_process_id,
        ProcessStatus::Failed,
    )
    .await;
    let failure_tenant = ResourceAccount::tenant(failure_scope.tenant_id.clone());
    assert_eq!(
        failure_governor.reserved_for(&failure_tenant).process_count,
        0
    );
    assert_eq!(failure_governor.usage_for(&failure_tenant).process_count, 0);
}

#[tokio::test]
async fn background_process_manager_releases_process_reservation_after_kill_before_late_success() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let store = Arc::new(ResourceManagedProcessStore::new(
        InMemoryProcessStore::new(),
        governor.clone(),
    ));
    let executor = Arc::new(CountingExecutor::delayed_success(Duration::from_millis(25)));
    let manager = BackgroundProcessManager::new(store.clone(), executor);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start_with_estimate(
            process_id,
            invocation_id,
            scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap();
    store.kill(&scope, process_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;

    let tenant = ResourceAccount::tenant(scope.tenant_id.clone());
    assert_eq!(
        store.get(&scope, process_id).await.unwrap().unwrap().status,
        ProcessStatus::Killed
    );
    assert_eq!(governor.reserved_for(&tenant).process_count, 0);
    assert_eq!(governor.usage_for(&tenant).process_count, 0);
}

#[tokio::test]
async fn process_host_cooperative_kill_releases_process_reservation() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let store = Arc::new(ResourceManagedProcessStore::new(
        InMemoryProcessStore::new(),
        governor.clone(),
    ));
    let cancellation_registry = Arc::new(ProcessCancellationRegistry::new());
    let executor = Arc::new(CancellationAwareExecutor::default());
    let manager = BackgroundProcessManager::new(store.clone(), executor.clone())
        .with_cancellation_registry(cancellation_registry.clone());
    let host = ProcessHost::new(store.as_ref())
        .with_cancellation_registry(cancellation_registry)
        .with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let tenant = ResourceAccount::tenant(scope.tenant_id.clone());

    manager
        .spawn(process_start_with_estimate(
            process_id,
            invocation_id,
            scope.clone(),
            process_estimate(),
        ))
        .await
        .unwrap();
    assert_eq!(governor.reserved_for(&tenant).process_count, 1);

    host.kill(&scope, process_id).await.unwrap();
    timeout(Duration::from_millis(200), executor.wait_for_cancellation())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;

    assert_eq!(
        store.get(&scope, process_id).await.unwrap().unwrap().status,
        ProcessStatus::Killed
    );
    assert_eq!(governor.reserved_for(&tenant).process_count, 0);
    assert_eq!(governor.usage_for(&tenant).process_count, 0);
}

#[tokio::test]
async fn process_host_kill_records_killed_result_without_output() {
    let store = Arc::new(InMemoryProcessStore::new());
    let result_store = Arc::new(InMemoryProcessResultStore::new());
    let cancellation_registry = Arc::new(ProcessCancellationRegistry::new());
    let executor = Arc::new(CancellationAwareExecutor::default());
    let manager = BackgroundProcessManager::new(store.clone(), executor.clone())
        .with_cancellation_registry(cancellation_registry.clone())
        .with_result_store(result_store.clone());
    let host = ProcessHost::new(store.as_ref())
        .with_cancellation_registry(cancellation_registry)
        .with_result_store(result_store.clone())
        .with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    host.kill(&scope, process_id).await.unwrap();
    timeout(Duration::from_millis(200), executor.wait_for_cancellation())
        .await
        .unwrap();

    let result = host.result(&scope, process_id).await.unwrap().unwrap();
    assert_eq!(result.status, ProcessStatus::Killed);
    assert_eq!(result.output, None);
    assert_eq!(result.output_ref, None);
    assert_eq!(result.error_kind, None);
}

#[tokio::test]
async fn process_host_await_result_returns_unavailable_when_terminal_result_is_missing() {
    let store = InMemoryProcessStore::new();
    let result_store = Arc::new(DroppingProcessResultStore);
    let host = ProcessHost::new(&store)
        .with_result_store(result_store)
        .with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    store.complete(&scope, process_id).await.unwrap();

    let err = timeout(
        Duration::from_millis(100),
        host.await_result(&scope, process_id),
    )
    .await
    .unwrap()
    .unwrap_err();

    assert!(
        matches!(err, ProcessError::ProcessResultUnavailable { process_id: id } if id == process_id)
    );
}

#[tokio::test]
async fn process_host_await_result_waits_for_background_success() {
    let store = Arc::new(InMemoryProcessStore::new());
    let result_store = Arc::new(InMemoryProcessResultStore::new());
    let manager = BackgroundProcessManager::new(
        store.clone(),
        Arc::new(CountingExecutor::delayed_success(Duration::from_millis(25))),
    )
    .with_result_store(result_store.clone());
    let host = ProcessHost::new(store.as_ref())
        .with_result_store(result_store.clone())
        .with_poll_interval(Duration::from_millis(5));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let result = host.await_result(&scope, process_id).await.unwrap();
    assert_eq!(result.status, ProcessStatus::Completed);
    assert_eq!(result.output, Some(serde_json::json!({"ok": true})));
    assert_eq!(result.output_ref, None);
    assert_eq!(
        host.output(&scope, process_id).await.unwrap(),
        Some(serde_json::json!({"ok": true}))
    );
}

#[tokio::test]
async fn process_result_lookup_is_resource_scope_scoped() {
    let store = Arc::new(InMemoryProcessStore::new());
    let result_store = Arc::new(InMemoryProcessResultStore::new());
    let manager =
        BackgroundProcessManager::new(store.clone(), Arc::new(CountingExecutor::success()))
            .with_result_store(result_store.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let owner_scope = sample_scope(invocation_id, "tenant1", "user1");
    let other_tenant = sample_scope(invocation_id, "tenant2", "user1");
    let other_user = sample_scope(invocation_id, "tenant1", "user2");
    let other_project = sample_scope_with_project(invocation_id, "tenant1", "user1", "project2");
    let host = ProcessHost::new(store.as_ref()).with_result_store(result_store.clone());

    manager
        .spawn(process_start(
            process_id,
            invocation_id,
            owner_scope.clone(),
        ))
        .await
        .unwrap();
    wait_for_status(
        store.as_ref(),
        &owner_scope,
        process_id,
        ProcessStatus::Completed,
    )
    .await;

    assert!(
        host.result(&owner_scope, process_id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        host.result(&other_tenant, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        host.result(&other_user, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        host.result(&other_project, process_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn filesystem_process_result_store_persists_under_resource_scope() {
    let fs = engine_filesystem();
    let store = FilesystemProcessResultStore::new(&fs);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let other_scope = sample_scope(invocation_id, "tenant2", "user1");
    let other_project = sample_scope_with_project(invocation_id, "tenant1", "user1", "project2");

    store
        .complete(&scope, process_id, serde_json::json!({"ok": true}))
        .await
        .unwrap();

    let reloaded = FilesystemProcessResultStore::new(&fs)
        .get(&scope, process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.status, ProcessStatus::Completed);
    assert_eq!(reloaded.output, None);
    assert_eq!(
        reloaded.output_ref,
        Some(
            VirtualPath::new(format!(
                "{}/process-outputs/{}/output.json",
                stored_process_owner_root(&scope),
                process_id
            ))
            .unwrap()
        )
    );
    assert_eq!(
        FilesystemProcessResultStore::new(&fs)
            .output(&scope, process_id)
            .await
            .unwrap(),
        Some(serde_json::json!({"ok": true}))
    );
    assert!(
        FilesystemProcessResultStore::new(&fs)
            .get(&other_scope, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        FilesystemProcessResultStore::new(&fs)
            .output(&other_scope, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        FilesystemProcessResultStore::new(&fs)
            .get(&other_project, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        FilesystemProcessResultStore::new(&fs)
            .output(&other_project, process_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn background_process_manager_stores_filesystem_output_ref() {
    let fs = Arc::new(engine_filesystem());
    let store = Arc::new(InMemoryProcessStore::new());
    let result_store = Arc::new(FilesystemProcessResultStore::from_arc(fs));
    let manager =
        BackgroundProcessManager::new(store.clone(), Arc::new(CountingExecutor::success()))
            .with_result_store(result_store.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let host = ProcessHost::new(store.as_ref())
        .with_result_store(result_store.clone())
        .with_poll_interval(Duration::from_millis(5));

    manager
        .spawn(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    let result = host.await_result(&scope, process_id).await.unwrap();
    assert_eq!(result.status, ProcessStatus::Completed);
    assert_eq!(result.output, None);
    assert!(result.output_ref.is_some());
    assert_eq!(
        host.output(&scope, process_id).await.unwrap(),
        Some(serde_json::json!({"ok": true}))
    );
}

#[tokio::test]
async fn filesystem_process_store_propagates_backend_errors_that_mention_not_found() {
    let fs = BackendErrorFilesystem;
    let store = FilesystemProcessStore::new(&fs);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    let err = store.get(&scope, process_id).await.unwrap_err();

    assert!(matches!(
        err,
        ProcessError::Filesystem(reason) if reason.contains("database index not found")
    ));
}

#[tokio::test]
async fn filesystem_process_store_rejects_record_id_mismatches() {
    let fs = engine_filesystem();
    let store = FilesystemProcessStore::new(&fs);
    let invocation_id = InvocationId::new();
    let requested_process_id = ProcessId::new();
    let stored_process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let mut forged = process_record(stored_process_id, invocation_id, scope.clone());
    forged.status = ProcessStatus::Completed;

    fs.write_file(
        &stored_process_record_path(&scope, requested_process_id),
        &serde_json::to_vec_pretty(&forged).unwrap(),
    )
    .await
    .unwrap();

    let err = store.get(&scope, requested_process_id).await.unwrap_err();

    assert!(matches!(err, ProcessError::InvalidStoredRecord { .. }));
}

#[tokio::test]
async fn filesystem_process_result_store_rejects_unexpected_output_refs() {
    let fs = engine_filesystem();
    let store = FilesystemProcessResultStore::new(&fs);
    let owner_invocation_id = InvocationId::new();
    let owner_process_id = ProcessId::new();
    let owner_scope = sample_scope(owner_invocation_id, "tenant1", "user1");
    let other_invocation_id = InvocationId::new();
    let other_process_id = ProcessId::new();
    let other_scope = sample_scope(other_invocation_id, "tenant2", "user1");

    store
        .complete(
            &other_scope,
            other_process_id,
            serde_json::json!({"secret": true}),
        )
        .await
        .unwrap();
    let forged = ProcessResultRecord {
        process_id: owner_process_id,
        scope: owner_scope.clone(),
        status: ProcessStatus::Completed,
        output: None,
        output_ref: Some(stored_process_output_path(&other_scope, other_process_id)),
        error_kind: None,
    };
    fs.write_file(
        &stored_process_result_path(&owner_scope, owner_process_id),
        &serde_json::to_vec_pretty(&forged).unwrap(),
    )
    .await
    .unwrap();

    let err = store
        .output(&owner_scope, owner_process_id)
        .await
        .unwrap_err();

    assert!(matches!(err, ProcessError::InvalidStoredRecord { .. }));
}

#[tokio::test]
async fn filesystem_process_store_persists_under_resource_scope_engine_processes() {
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

enum UnownedTransition {
    Complete,
    Fail,
    Kill,
}

async fn assert_unowned_process_reservation_rejected(transition: UnownedTransition) {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_process_count: Some(2),
            ..ResourceLimits::default()
        },
    );
    let estimate = ResourceEstimate {
        process_count: Some(1),
        ..ResourceEstimate::default()
    };
    let forged_reservation = governor.reserve(scope.clone(), estimate.clone()).unwrap();
    let process_id = ProcessId::new();
    let inner = ForgedProcessStore::default();
    inner.insert(ProcessRecord {
        process_id,
        parent_process_id: None,
        invocation_id: InvocationId::new(),
        scope: scope.clone(),
        extension_id: ExtensionId::new("echo").unwrap(),
        capability_id: CapabilityId::new("echo.say").unwrap(),
        runtime: RuntimeKind::Wasm,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        estimated_resources: estimate,
        resource_reservation_id: Some(forged_reservation.id),
        status: ProcessStatus::Running,
        error_kind: None,
    });
    let store = ResourceManagedProcessStore::new(inner.clone(), governor.clone());

    let err = match transition {
        UnownedTransition::Complete => store.complete(&scope, process_id).await.unwrap_err(),
        UnownedTransition::Fail => store
            .fail(&scope, process_id, "forged".to_string())
            .await
            .unwrap_err(),
        UnownedTransition::Kill => store.kill(&scope, process_id).await.unwrap_err(),
    };

    assert!(matches!(
        err,
        ProcessError::ResourceReservationNotOwned {
            process_id: actual_process_id,
            reservation_id: Some(actual_reservation_id),
        } if actual_process_id == process_id && actual_reservation_id == forged_reservation.id
    ));
    assert_eq!(governor.reserved_for(&account).process_count, 1);
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    let record = inner.get(&scope, process_id).await.unwrap().unwrap();
    assert_eq!(record.status, ProcessStatus::Running);
}

type ForgedProcessKey = (TenantId, UserId, ProcessId);

type ForgedProcessRecords = Arc<Mutex<HashMap<ForgedProcessKey, ProcessRecord>>>;

#[derive(Clone, Default)]
struct ForgedProcessStore {
    records: ForgedProcessRecords,
}

impl ForgedProcessStore {
    fn insert(&self, record: ProcessRecord) {
        self.records.lock().unwrap().insert(
            (
                record.scope.tenant_id.clone(),
                record.scope.user_id.clone(),
                record.process_id,
            ),
            record,
        );
    }

    fn update_status(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        status: ProcessStatus,
        error_kind: Option<String>,
    ) -> Result<ProcessRecord, ProcessError> {
        let mut records = self.records.lock().unwrap();
        let record = records
            .get_mut(&(scope.tenant_id.clone(), scope.user_id.clone(), process_id))
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        record.status = status;
        record.error_kind = error_kind;
        Ok(record.clone())
    }
}

#[async_trait]
impl ProcessStore for ForgedProcessStore {
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        let record = ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            extension_id: start.extension_id,
            capability_id: start.capability_id,
            runtime: start.runtime,
            status: ProcessStatus::Running,
            grants: start.grants,
            mounts: start.mounts,
            estimated_resources: start.estimated_resources,
            resource_reservation_id: start.resource_reservation_id,
            error_kind: None,
        };
        self.insert(record.clone());
        Ok(record)
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Completed, None)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Failed, Some(error_kind))
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Killed, None)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        Ok(self
            .records
            .lock()
            .unwrap()
            .get(&(scope.tenant_id.clone(), scope.user_id.clone(), process_id))
            .cloned())
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        Ok(self
            .records
            .lock()
            .unwrap()
            .values()
            .filter(|record| {
                record.scope.tenant_id == scope.tenant_id && record.scope.user_id == scope.user_id
            })
            .cloned()
            .collect())
    }
}

#[derive(Default)]
struct CompletionReservationDroppingStore {
    inner: InMemoryProcessStore,
}

#[async_trait]
impl ProcessStore for CompletionReservationDroppingStore {
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        self.inner.start(start).await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let mut record = self.inner.complete(scope, process_id).await?;
        record.resource_reservation_id = None;
        Ok(record)
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
        self.inner.get(scope, process_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        self.inner.records_for_scope(scope).await
    }
}

#[derive(Default)]
struct ReleaseFailingGovernor {
    inner: InMemoryResourceGovernor,
}

impl ResourceGovernor for ReleaseFailingGovernor {
    fn set_limit(&self, account: ResourceAccount, limits: ResourceLimits) {
        self.inner.set_limit(account, limits);
    }

    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ironclaw_resources::ResourceReservation, ResourceError> {
        self.inner.reserve(scope, estimate)
    }

    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ironclaw_resources::ResourceReservation, ResourceError> {
        self.inner.reserve_with_id(scope, estimate, reservation_id)
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ironclaw_resources::ResourceReceipt, ResourceError> {
        self.inner.reconcile(reservation_id, actual)
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ironclaw_resources::ResourceReceipt, ResourceError> {
        Err(ResourceError::UnknownReservation { id: reservation_id })
    }
}

#[derive(Default)]
struct ReconcileFailingGovernor {
    inner: InMemoryResourceGovernor,
}

impl ResourceGovernor for ReconcileFailingGovernor {
    fn set_limit(&self, account: ResourceAccount, limits: ResourceLimits) {
        self.inner.set_limit(account, limits);
    }

    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ironclaw_resources::ResourceReservation, ResourceError> {
        self.inner.reserve(scope, estimate)
    }

    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ironclaw_resources::ResourceReservation, ResourceError> {
        self.inner.reserve_with_id(scope, estimate, reservation_id)
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        _actual: ResourceUsage,
    ) -> Result<ironclaw_resources::ResourceReceipt, ResourceError> {
        Err(ResourceError::UnknownReservation { id: reservation_id })
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ironclaw_resources::ResourceReceipt, ResourceError> {
        self.inner.release(reservation_id)
    }
}

struct BackendErrorFilesystem;

#[async_trait]
impl RootFilesystem for BackendErrorFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        Err(backend_error(path, FilesystemOperation::ReadFile))
    }

    async fn write_file(&self, path: &VirtualPath, _bytes: &[u8]) -> Result<(), FilesystemError> {
        Err(backend_error(path, FilesystemOperation::WriteFile))
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        Err(backend_error(path, FilesystemOperation::ListDir))
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        Err(backend_error(path, FilesystemOperation::Stat))
    }
}

fn backend_error(path: &VirtualPath, operation: FilesystemOperation) -> FilesystemError {
    FilesystemError::Backend {
        path: path.clone(),
        operation,
        reason: "database index not found while backend is unavailable".to_string(),
    }
}

struct ReservationDroppingStore;

#[async_trait]
impl ProcessStore for ReservationDroppingStore {
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        Ok(ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            extension_id: start.extension_id,
            capability_id: start.capability_id,
            runtime: start.runtime,
            status: ProcessStatus::Running,
            grants: start.grants,
            mounts: start.mounts,
            estimated_resources: start.estimated_resources,
            resource_reservation_id: None,
            error_kind: None,
        })
    }

    async fn complete(
        &self,
        _scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        Err(ProcessError::UnknownProcess { process_id })
    }

    async fn fail(
        &self,
        _scope: &ResourceScope,
        process_id: ProcessId,
        _error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        Err(ProcessError::UnknownProcess { process_id })
    }

    async fn kill(
        &self,
        _scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        Err(ProcessError::UnknownProcess { process_id })
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        Ok(None)
    }

    async fn records_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct CancellationAwareExecutor {
    cancellations: AtomicUsize,
    notified: Notify,
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
            output: serde_json::json!({"cancelled": true}),
        })
    }
}

struct PanicExecutor;

#[async_trait]
impl ProcessExecutor for PanicExecutor {
    async fn execute(
        &self,
        _request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        panic!("simulated runtime panic");
    }
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

struct DroppingProcessResultStore;

#[derive(Default)]
struct FailingProcessResultStore {
    failures: Mutex<Vec<&'static str>>,
}

impl FailingProcessResultStore {
    fn failures(&self) -> Vec<&'static str> {
        self.failures.lock().unwrap().clone()
    }

    fn record(&self, kind: &'static str) {
        self.failures.lock().unwrap().push(kind);
    }
}

#[async_trait]
impl ProcessResultStore for FailingProcessResultStore {
    async fn complete(
        &self,
        _scope: &ResourceScope,
        _process_id: ProcessId,
        _output: serde_json::Value,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.record("complete");
        Err(ProcessError::ProcessResultStoreUnavailable)
    }

    async fn fail(
        &self,
        _scope: &ResourceScope,
        _process_id: ProcessId,
        _error_kind: String,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.record("fail");
        Err(ProcessError::ProcessResultStoreUnavailable)
    }

    async fn kill(
        &self,
        _scope: &ResourceScope,
        _process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.record("kill");
        Err(ProcessError::ProcessResultStoreUnavailable)
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError> {
        Ok(None)
    }
}

#[async_trait]
impl ProcessResultStore for DroppingProcessResultStore {
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        _output: serde_json::Value,
    ) -> Result<ProcessResultRecord, ProcessError> {
        Ok(ProcessResultRecord {
            process_id,
            scope: scope.clone(),
            status: ProcessStatus::Completed,
            output: None,
            output_ref: None,
            error_kind: None,
        })
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessResultRecord, ProcessError> {
        Ok(ProcessResultRecord {
            process_id,
            scope: scope.clone(),
            status: ProcessStatus::Failed,
            output: None,
            output_ref: None,
            error_kind: Some(error_kind),
        })
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError> {
        Ok(ProcessResultRecord {
            process_id,
            scope: scope.clone(),
            status: ProcessStatus::Killed,
            output: None,
            output_ref: None,
            error_kind: None,
        })
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError> {
        Ok(None)
    }
}

async fn wait_for_event_count(events: &InMemoryEventSink, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let count = events.events().len();
        if count >= expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "event sink did not reach {expected} events; last count was {count}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
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

fn process_record(
    process_id: ProcessId,
    invocation_id: InvocationId,
    scope: ResourceScope,
) -> ProcessRecord {
    let start = process_start(process_id, invocation_id, scope);
    ProcessRecord {
        process_id: start.process_id,
        parent_process_id: start.parent_process_id,
        invocation_id: start.invocation_id,
        scope: start.scope,
        extension_id: start.extension_id,
        capability_id: start.capability_id,
        runtime: start.runtime,
        status: ProcessStatus::Running,
        grants: start.grants,
        mounts: start.mounts,
        estimated_resources: start.estimated_resources,
        resource_reservation_id: start.resource_reservation_id,
        error_kind: None,
    }
}

fn process_start(
    process_id: ProcessId,
    invocation_id: InvocationId,
    scope: ResourceScope,
) -> ProcessStart {
    process_start_with_estimate(
        process_id,
        invocation_id,
        scope,
        ResourceEstimate::default(),
    )
}

fn process_start_with_estimate(
    process_id: ProcessId,
    invocation_id: InvocationId,
    scope: ResourceScope,
    estimated_resources: ResourceEstimate,
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
                issued_by: Principal::HostRuntime,
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
        estimated_resources,
        resource_reservation_id: None,
        input: serde_json::json!({"message": "runtime payload"}),
    }
}

fn process_estimate() -> ResourceEstimate {
    ResourceEstimate {
        process_count: Some(1),
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    }
}

fn stored_process_record_path(scope: &ResourceScope, process_id: ProcessId) -> VirtualPath {
    VirtualPath::new(format!(
        "{}/processes/{process_id}.json",
        stored_process_owner_root(scope)
    ))
    .unwrap()
}

fn stored_process_result_path(scope: &ResourceScope, process_id: ProcessId) -> VirtualPath {
    VirtualPath::new(format!(
        "{}/process-results/{process_id}.json",
        stored_process_owner_root(scope)
    ))
    .unwrap()
}

fn stored_process_output_path(scope: &ResourceScope, process_id: ProcessId) -> VirtualPath {
    VirtualPath::new(format!(
        "{}/process-outputs/{process_id}/output.json",
        stored_process_owner_root(scope)
    ))
    .unwrap()
}

fn stored_process_owner_root(scope: &ResourceScope) -> String {
    let mut base = format!(
        "/engine/tenants/{}/users/{}",
        scope.tenant_id.as_str(),
        scope.user_id.as_str()
    );
    if let Some(agent_id) = &scope.agent_id {
        base = format!("{base}/agents/{}", agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        base = format!("{base}/projects/{}", project_id.as_str());
    }
    if let Some(mission_id) = &scope.mission_id {
        base = format!("{base}/missions/{}", mission_id.as_str());
    }
    if let Some(thread_id) = &scope.thread_id {
        base = format!("{base}/threads/{}", thread_id.as_str());
    }
    base
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

fn sample_scope_with_agent(
    invocation_id: InvocationId,
    tenant: &str,
    user: &str,
    agent: Option<&str>,
) -> ResourceScope {
    let mut scope = sample_scope(invocation_id, tenant, user);
    scope.agent_id = agent.map(|id| AgentId::new(id).unwrap());
    scope
}

fn sample_scope_with_project(
    invocation_id: InvocationId,
    tenant: &str,
    user: &str,
    project: &str,
) -> ResourceScope {
    sample_scope_with_agent_and_project(invocation_id, tenant, user, None, project)
}

fn sample_scope_with_agent_and_project(
    invocation_id: InvocationId,
    tenant: &str,
    user: &str,
    agent: Option<&str>,
    project: &str,
) -> ResourceScope {
    let mut scope = sample_scope_with_agent(invocation_id, tenant, user, agent);
    scope.project_id = Some(ProjectId::new(project).unwrap());
    scope
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

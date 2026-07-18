// arch-exempt: large_file, mechanical process-store test repoint to FilesystemProcess*Store<InMemoryBackend> helpers + output-externalization/tenant-isolation reconciliation (arch-simplification §4.3), plan #6168
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
    CasExpectation, DirEntry, Entry, FileStat, FilesystemError, FilesystemOperation,
    InMemoryBackend, LocalFilesystem, RecordVersion, RootFilesystem, ScopedFilesystem,
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
    let store = in_memory_process_store();
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
    let store = in_memory_process_store();
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

/// Sub-scope isolation that lives in the **path** (agent/project/mission/thread):
/// a record started under one project is invisible to another project on the
/// same tenant/user mount. Tenant/user isolation lives in the `MountView`
/// instead — a single fixed mount can't reproduce it (arch-simplification §4.3:
/// the store no longer hand-keys the full scope tuple; tenant/user come from the
/// mount) — and is covered by
/// `filesystem_process_store_isolates_two_tenants_with_same_user_project_ids`.
#[tokio::test]
async fn process_store_hides_records_from_other_resource_scopes() {
    let store = in_memory_process_store();
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope_a = sample_scope(invocation_id, "tenant1", "user1");
    let project_b = sample_scope_with_project(invocation_id, "tenant1", "user1", "project2");
    store
        .start(process_start(process_id, invocation_id, scope_a.clone()))
        .await
        .unwrap();

    assert!(store.get(&project_b, process_id).await.unwrap().is_none());
    assert_eq!(
        store.records_for_scope(&project_b).await.unwrap(),
        Vec::new()
    );
    assert!(matches!(
        store.kill(&project_b, process_id).await.unwrap_err(),
        ProcessError::UnknownProcess { .. }
    ));
}

#[tokio::test]
async fn process_store_hides_records_from_other_agents() {
    let store = in_memory_process_store();
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
    let store = in_memory_process_result_store();
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
    let store = in_memory_process_store();
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
    let store = Arc::new(in_memory_process_store());
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
    let store = Arc::new(in_memory_process_store());
    let result_store = Arc::new(in_memory_process_result_store());
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
    // The filesystem store externalizes output behind `output_ref` (§4.3 — the
    // InMemory store inlined it); read the bytes through the host, not the record.
    assert_eq!(result.output, None);
    assert!(result.output_ref.is_some());
    assert_eq!(
        host.output(&scope, process_id).await.unwrap(),
        Some(serde_json::json!({"ok": true}))
    );
    assert_eq!(result.error_kind, None);
}

#[tokio::test]
async fn process_stores_sanitize_failure_error_kind_before_persistence() {
    let scope = sample_scope(InvocationId::new(), "tenant1", "user1");
    let process_id = ProcessId::new();
    let lifecycle_store = in_memory_process_store();
    lifecycle_store
        .start(process_start(
            process_id,
            scope.invocation_id,
            scope.clone(),
        ))
        .await
        .unwrap();

    let failed = lifecycle_store
        .fail(
            &scope,
            process_id,
            "RAW_PROCESS_ERROR_SENTINEL_3022 /tmp/private-process sk_live".to_string(),
        )
        .await
        .unwrap();
    assert_eq!(failed.error_kind.as_deref(), Some("Unclassified"));

    let result_store = in_memory_process_result_store();
    let result = result_store
        .fail(
            &scope,
            process_id,
            "RAW_PROCESS_RESULT_SENTINEL_3022 /tmp/private-result sk_live".to_string(),
        )
        .await
        .unwrap();
    assert_eq!(result.error_kind.as_deref(), Some("Unclassified"));
}

#[tokio::test]
async fn background_process_manager_stores_failure_error_result() {
    let store = Arc::new(in_memory_process_store());
    let result_store = Arc::new(in_memory_process_result_store());
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
    let store = Arc::new(in_memory_process_store());
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
    let store = Arc::new(in_memory_process_store());
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
    let store = Arc::new(in_memory_process_store());
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
    let store = Arc::new(in_memory_process_store());
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

/// A kill issued under a *different scope* (here a different project — path-scoped
/// isolation on a single mount) must not reach the owner's process or signal its
/// cancellation token. Cross-*tenant* isolation is mount-structural and covered
/// by `filesystem_process_store_isolates_two_tenants_*` (arch-simplification §4.3).
#[tokio::test]
async fn process_host_kill_does_not_cancel_other_scope_process() {
    let store = Arc::new(in_memory_process_store());
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
    let other_scope = sample_scope_with_project(invocation_id, "tenant1", "user1", "project2");

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
        "wrong-scope kill must not signal the owner's cancellation token"
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
    let filesystem = engine_filesystem();
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
    let store = FilesystemProcessStore::new(Arc::clone(&fs));
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
    let store = EventingProcessStore::new(in_memory_process_store(), events.clone());
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
        in_memory_process_store(),
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
        in_memory_process_store(),
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
        in_memory_process_store(),
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
    let store = ResourceManagedProcessStore::new(in_memory_process_store(), governor.clone());
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
    governor
        .set_limit(
            ResourceAccount::tenant(scope.tenant_id.clone()),
            ResourceLimits::default().set_max_process_count(1),
        )
        .unwrap();
    let store = ResourceManagedProcessStore::new(in_memory_process_store(), governor.clone());

    let exceeding_estimate = ResourceEstimate::default()
        .set_process_count(2)
        .set_concurrency_slots(1);
    let err = store
        .start(process_start_with_estimate(
            process_id,
            invocation_id,
            scope.clone(),
            exceeding_estimate,
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ProcessError::Resource(ResourceError::LimitExceeded { .. })
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
    let store = ResourceManagedProcessStore::new(in_memory_process_store(), governor.clone());
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
        ResourceManagedProcessStore::new(CompletionReservationDroppingStore::new(), governor);
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
    let inner = in_memory_process_store();
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
    let inner = in_memory_process_store();
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
    let completion_usage = ResourceUsage::default()
        .set_process_count(1)
        .set_output_tokens(7);
    let store = ResourceManagedProcessStore::new(in_memory_process_store(), governor.clone())
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
        in_memory_process_store(),
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
        ResourceManagedProcessStore::new(in_memory_process_store(), success_governor.clone())
            .with_completion_usage(ResourceUsage::default().set_process_count(1)),
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
        in_memory_process_store(),
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
        in_memory_process_store(),
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
        in_memory_process_store(),
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
    let store = Arc::new(in_memory_process_store());
    let result_store = Arc::new(in_memory_process_result_store());
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
    let store = in_memory_process_store();
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
    let store = Arc::new(in_memory_process_store());
    let result_store = Arc::new(in_memory_process_result_store());
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
    // Filesystem store externalizes output behind `output_ref` (§4.3).
    assert_eq!(result.output, None);
    assert!(result.output_ref.is_some());
    assert_eq!(
        host.output(&scope, process_id).await.unwrap(),
        Some(serde_json::json!({"ok": true}))
    );
}

#[tokio::test]
async fn process_result_lookup_is_resource_scope_scoped() {
    let store = Arc::new(in_memory_process_store());
    let result_store = Arc::new(in_memory_process_result_store());
    let manager =
        BackgroundProcessManager::new(store.clone(), Arc::new(CountingExecutor::success()))
            .with_result_store(result_store.clone());
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let owner_scope = sample_scope(invocation_id, "tenant1", "user1");
    // A different project is path-isolated on a single mount; cross-tenant/user
    // isolation is mount-structural (covered by the two-tenant filesystem test).
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
        host.result(&other_project, process_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn filesystem_process_result_store_persists_under_resource_scope() {
    let fs = engine_filesystem();
    let store = FilesystemProcessResultStore::new(Arc::clone(&fs));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let other_scope = sample_scope(invocation_id, "tenant2", "user1");
    let other_project = sample_scope_with_project(invocation_id, "tenant1", "user1", "project2");

    store
        .complete(&scope, process_id, serde_json::json!({"ok": true}))
        .await
        .unwrap();

    let reloaded = FilesystemProcessResultStore::new(Arc::clone(&fs))
        .get(&scope, process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.status, ProcessStatus::Completed);
    assert_eq!(reloaded.output, None);
    assert_eq!(
        reloaded.output_ref,
        Some(stored_process_output_path(&scope, process_id)),
    );
    assert_eq!(
        FilesystemProcessResultStore::new(Arc::clone(&fs))
            .output(&scope, process_id)
            .await
            .unwrap(),
        Some(serde_json::json!({"ok": true}))
    );
    assert!(
        FilesystemProcessResultStore::new(Arc::clone(&fs))
            .get(&other_scope, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        FilesystemProcessResultStore::new(Arc::clone(&fs))
            .output(&other_scope, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        FilesystemProcessResultStore::new(Arc::clone(&fs))
            .get(&other_project, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        FilesystemProcessResultStore::new(Arc::clone(&fs))
            .output(&other_project, process_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn background_process_manager_stores_filesystem_output_ref() {
    let fs = engine_filesystem();
    let store = Arc::new(in_memory_process_store());
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
async fn filesystem_process_store_preserves_typed_backend_errors_that_mention_not_found() {
    let fs = scoped_processes_filesystem(
        Arc::new(BackendErrorFilesystem),
        &default_mount_target_string(),
    );
    let store = FilesystemProcessStore::new(fs);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    let err = store.get(&scope, process_id).await.unwrap_err();

    assert!(matches!(
        &err,
        ProcessError::Filesystem(FilesystemError::Backend { reason, .. })
            if reason.contains("database index not found")
    ));
    assert!(!err.is_filesystem_not_found());
}

#[tokio::test]
async fn filesystem_process_result_store_preserves_typed_backend_write_errors() {
    let fs = scoped_processes_filesystem(
        Arc::new(BackendErrorFilesystem),
        &default_mount_target_string(),
    );
    let store = FilesystemProcessResultStore::new(fs);
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    let err = store
        .complete(&scope, process_id, serde_json::json!({"ok": true}))
        .await
        .unwrap_err();

    assert!(matches!(
        &err,
        ProcessError::Filesystem(FilesystemError::Backend { operation, reason, .. })
            if *operation == FilesystemOperation::WriteFile
                && reason.contains("database index not found")
    ));
    assert!(!err.is_filesystem_not_found());
}

#[test]
fn process_error_filesystem_not_found_predicate_distinguishes_backend_errors() {
    let path = VirtualPath::new("/users/user1/processes/missing.json").unwrap();
    let not_found = ProcessError::from(FilesystemError::NotFound {
        path: path.clone(),
        operation: FilesystemOperation::ReadFile,
    });
    let backend = ProcessError::from(FilesystemError::Backend {
        path,
        operation: FilesystemOperation::ReadFile,
        reason: "database index not found while backend is unavailable".to_string(),
    });

    assert!(not_found.is_filesystem_not_found());
    assert!(!backend.is_filesystem_not_found());

    let wrapped_not_found = ProcessError::ResourceCleanupFailed {
        original: Box::new(not_found),
        cleanup: ResourceError::UnknownReservation {
            id: ResourceReservationId::new(),
        },
    };

    assert!(wrapped_not_found.is_filesystem_not_found());
}

#[tokio::test]
async fn filesystem_process_store_rejects_record_id_mismatches() {
    let fs = engine_filesystem();
    let store = FilesystemProcessStore::new(Arc::clone(&fs));
    let invocation_id = InvocationId::new();
    let requested_process_id = ProcessId::new();
    let stored_process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let mut forged = process_record(stored_process_id, invocation_id, scope.clone());
    forged.status = ProcessStatus::Completed;

    // Inject a forged record at the alias-relative ScopedPath for
    // `requested_process_id`. Going through the scoped filesystem (vs.
    // a raw `write_file`) keeps the test honest about the actual on-disk
    // surface the store will read from.
    fs.write_bytes(
        &scope,
        &scoped_record_path(&scope, requested_process_id),
        serde_json::to_vec_pretty(&forged).unwrap(),
    )
    .await
    .unwrap();

    let err = store.get(&scope, requested_process_id).await.unwrap_err();

    assert!(matches!(err, ProcessError::InvalidStoredRecord { .. }));
}

#[tokio::test]
async fn filesystem_process_result_store_rejects_unexpected_output_refs() {
    let fs = engine_filesystem();
    let store = FilesystemProcessResultStore::new(Arc::clone(&fs));
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
    // Forged record whose `output_ref` points at a *different* on-disk
    // location than the owner's expected output path. The store must
    // reject the read instead of dereferencing the forged ref.
    let forged = ProcessResultRecord {
        process_id: owner_process_id,
        scope: owner_scope.clone(),
        status: ProcessStatus::Completed,
        output: None,
        output_ref: Some(stored_process_output_path(&other_scope, other_process_id)),
        error_kind: None,
    };
    fs.write_bytes(
        &owner_scope,
        &scoped_result_path(&owner_scope, owner_process_id),
        serde_json::to_vec_pretty(&forged).unwrap(),
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
    let store = FilesystemProcessStore::new(Arc::clone(&fs));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();
    store.complete(&scope, process_id).await.unwrap();

    let reloaded = FilesystemProcessStore::new(Arc::clone(&fs))
        .get(&scope, process_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.status, ProcessStatus::Completed);
    assert_eq!(
        FilesystemProcessStore::new(Arc::clone(&fs))
            .records_for_scope(&scope)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn filesystem_process_store_records_for_scope_uses_index_on_record_backend() {
    // Drive `records_for_scope` against the in-memory backend (which
    // supports `query` over indexed projections) so the indexed path
    // exercised by SQL backends is covered. With the ScopedFilesystem
    // refactor, tenant/user isolation lives in the MountView (not the
    // path), so this test focuses on the *within-tenant* sub-scope
    // discrimination that does still live in the path: separate
    // `project_id` cells must not bleed into each other through the
    // indexed query path, even though they share one `/processes` mount.
    let backend = Arc::new(InMemoryBackend::new());
    let fs = scoped_processes_filesystem(Arc::clone(&backend), &default_mount_target_string());
    let store = FilesystemProcessStore::from_arc(fs);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let other_project_scope =
        sample_scope_with_project(invocation_id, "tenant1", "user1", "project2");

    let mine_a = ProcessId::new();
    let mine_b = ProcessId::new();
    let other_project = ProcessId::new();
    store
        .start(process_start(mine_a, invocation_id, scope.clone()))
        .await
        .unwrap();
    store
        .start(process_start(mine_b, invocation_id, scope.clone()))
        .await
        .unwrap();
    store
        .start(process_start(
            other_project,
            invocation_id,
            other_project_scope.clone(),
        ))
        .await
        .unwrap();

    let mine = store.records_for_scope(&scope).await.unwrap();
    let mut got: Vec<ProcessId> = mine.iter().map(|record| record.process_id).collect();
    got.sort_by_key(|id| id.as_uuid());
    let mut expected = vec![mine_a, mine_b];
    expected.sort_by_key(|id| id.as_uuid());
    assert_eq!(got, expected);

    let theirs = store.records_for_scope(&other_project_scope).await.unwrap();
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0].process_id, other_project);
}

/// Regression test for the tenant-isolation invariant: two
/// `FilesystemProcessStore`s sharing one backend but constructed with
/// different `MountView`s (i.e. different tenant/user mount targets)
/// must not see each other's records, even though their request scopes
/// share `user_id` / `project_id` / `agent_id` and the alias-relative
/// path is identical.
///
/// Before the migration to `Arc<ScopedFilesystem<F>>`, the store
/// hand-formatted `tenant_id`/`user_id` into the path string — so any
/// composition layer that forgot to do that (or did it differently in
/// one place) would silently share storage across tenants. With the
/// ScopedFilesystem refactor, the MountView resolves the leading
/// segment, and the type system makes it impossible for the store to
/// reach across mounts.
#[tokio::test]
async fn filesystem_process_store_isolates_two_tenants_with_same_user_project_ids() {
    let backend = Arc::new(InMemoryBackend::new());
    let store_a = FilesystemProcessStore::from_arc(scoped_processes_filesystem(
        Arc::clone(&backend),
        "/engine/tenants/a/users/alice/processes",
    ));
    let store_b = FilesystemProcessStore::from_arc(scoped_processes_filesystem(
        Arc::clone(&backend),
        "/engine/tenants/b/users/alice/processes",
    ));

    // Identical scope across both stores — the only thing separating them
    // is the mount-time tenant prefix on each store's MountView.
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant-a", "alice");
    store_a
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    // Tenant A sees its own record.
    assert!(
        store_a
            .get(&scope, process_id)
            .await
            .expect("store_a get succeeds")
            .is_some(),
        "tenant A must see the record it just wrote",
    );

    // Tenant B does NOT see tenant A's record, despite the identical
    // request scope and process id.
    assert!(
        store_b
            .get(&scope, process_id)
            .await
            .expect("store_b get succeeds")
            .is_none(),
        "tenant B must NOT see tenant A's record (cross-tenant leak)",
    );

    // Tenant B's records_for_scope must be empty under the shared
    // request scope.
    let b_records = store_b
        .records_for_scope(&scope)
        .await
        .expect("store_b records_for_scope succeeds");
    assert!(
        b_records.is_empty(),
        "tenant B records_for_scope must be empty under shared scope; got {} records",
        b_records.len(),
    );
}

/// Defense-in-depth regression for the tenant-isolation indexed
/// projection (see
/// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md` —
/// "What this gives us": `tenant_id` is projected alongside every
/// `Entry::record`/`Entry::bytes` write so an admin-tier query can
/// filter by tenant, and a path-rewriting bug surfaces as a
/// query-time mismatch rather than silent cross-tenant leakage).
///
/// Writes a process record under tenant A's scope, then issues a raw
/// `RootFilesystem::query` against the deliveries-equivalent records
/// root with `Filter::Eq { key: "tenant_id", value: <tenant-a> }` —
/// the record must be returned. A query for tenant B's id must
/// return zero rows over the same backend prefix.
#[tokio::test]
async fn filesystem_process_store_writes_tenant_id_indexed_projection() {
    use ironclaw_filesystem::{Filter, IndexKey, IndexValue, Page};

    let backend = Arc::new(InMemoryBackend::new());
    let fs = scoped_processes_filesystem(
        Arc::clone(&backend),
        "/engine/tenants/tenant-a/users/alice/processes",
    );
    let store = FilesystemProcessStore::from_arc(Arc::clone(&fs));
    let invocation_id = InvocationId::new();
    let process_id = ProcessId::new();
    let scope = sample_scope(invocation_id, "tenant-a", "alice");
    store
        .start(process_start(process_id, invocation_id, scope.clone()))
        .await
        .unwrap();

    // Resolve the alias-relative records prefix to the backing
    // [`VirtualPath`] through the same MountView the store uses, so the
    // raw `RootFilesystem::query` below targets exactly the bytes the
    // backend stored. The records root mirrors the in-store path
    // builder (`/processes[/projects/<id>]/records`) — the sample scope
    // carries `project_id = project1` but no agent/mission/thread.
    let records_root = ironclaw_host_api::ScopedPath::new(format!(
        "/processes/projects/{}/records",
        scope.project_id.as_ref().unwrap().as_str()
    ))
    .unwrap();
    let virtual_root = fs.resolve(&scope, &records_root).unwrap();

    let tenant_key = IndexKey::new("tenant_id").unwrap();
    let hit = backend
        .query(
            &virtual_root,
            &Filter::Eq {
                key: tenant_key.clone(),
                value: IndexValue::Text(scope.tenant_id.as_str().to_string()),
            },
            Page::new(0, Page::MAX_LIMIT),
        )
        .await
        .unwrap();
    assert_eq!(
        hit.len(),
        1,
        "tenant_id projection must surface the record via Filter::Eq",
    );

    let miss = backend
        .query(
            &virtual_root,
            &Filter::Eq {
                key: tenant_key,
                value: IndexValue::Text("tenant-b".to_string()),
            },
            Page::new(0, Page::MAX_LIMIT),
        )
        .await
        .unwrap();
    assert!(
        miss.is_empty(),
        "tenant_id projection must NOT surface tenant-a's record under tenant-b query; got {} rows",
        miss.len(),
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
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_process_count(2),
        )
        .unwrap();
    let estimate = ResourceEstimate::default().set_process_count(1);
    let forged_reservation = governor.reserve(scope.clone(), estimate.clone()).unwrap();
    let process_id = ProcessId::new();
    let inner = ForgedProcessStore::default();
    inner.insert(ProcessRecord {
        process_id,
        parent_process_id: None,
        invocation_id: InvocationId::new(),
        scope: scope.clone(),
        authenticated_actor_user_id: None,
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
            authenticated_actor_user_id: start.authenticated_actor_user_id,
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

struct CompletionReservationDroppingStore {
    inner: FilesystemProcessStore<InMemoryBackend>,
}

impl CompletionReservationDroppingStore {
    fn new() -> Self {
        Self {
            inner: in_memory_process_store(),
        }
    }
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
    fn set_limit(
        &self,
        account: ResourceAccount,
        limits: ResourceLimits,
    ) -> Result<(), ResourceError> {
        self.inner.set_limit(account, limits)
    }

    fn reserve_with_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ironclaw_resources::ReservationOutcome, ResourceError> {
        self.inner.reserve_with_outcome(scope, estimate)
    }

    fn reserve_with_id_and_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ironclaw_resources::ReservationOutcome, ResourceError> {
        self.inner
            .reserve_with_id_and_outcome(scope, estimate, reservation_id)
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

    fn account_snapshot(
        &self,
        account: &ResourceAccount,
    ) -> Result<Option<ironclaw_resources::AccountSnapshot>, ResourceError> {
        self.inner.account_snapshot(account)
    }
}

#[derive(Default)]
struct ReconcileFailingGovernor {
    inner: InMemoryResourceGovernor,
}

impl ResourceGovernor for ReconcileFailingGovernor {
    fn set_limit(
        &self,
        account: ResourceAccount,
        limits: ResourceLimits,
    ) -> Result<(), ResourceError> {
        self.inner.set_limit(account, limits)
    }

    fn reserve_with_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ironclaw_resources::ReservationOutcome, ResourceError> {
        self.inner.reserve_with_outcome(scope, estimate)
    }

    fn reserve_with_id_and_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ironclaw_resources::ReservationOutcome, ResourceError> {
        self.inner
            .reserve_with_id_and_outcome(scope, estimate, reservation_id)
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

    fn account_snapshot(
        &self,
        account: &ResourceAccount,
    ) -> Result<Option<ironclaw_resources::AccountSnapshot>, ResourceError> {
        self.inner.account_snapshot(account)
    }
}

struct BackendErrorFilesystem;

#[async_trait]
impl RootFilesystem for BackendErrorFilesystem {
    async fn put(
        &self,
        path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        Err(backend_error(path, FilesystemOperation::WriteFile))
    }

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

    // After the PR #3666 fix that breaks the put/write_file recursion, the
    // trait's default `get` is `Unsupported`. A test backend that wants to
    // fault-inject through the unified read path has to override `get`
    // explicitly — same shape that `LocalFilesystem` adopts in its native
    // impl. Mirroring the same fault here keeps the consumer test
    // exercising the "backend error mentions not_found" propagation.
    async fn get(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<ironclaw_filesystem::VersionedEntry>, FilesystemError> {
        Err(backend_error(path, FilesystemOperation::ReadFile))
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
            authenticated_actor_user_id: start.authenticated_actor_user_id,
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
        authenticated_actor_user_id: start.authenticated_actor_user_id,
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
        authenticated_actor_user_id: None,
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
    ResourceEstimate::default()
        .set_process_count(1)
        .set_concurrency_slots(1)
}

// ── Test path layout ───────────────────────────────────────────
//
// After the FilesystemProcessStore refactor onto `ScopedFilesystem`, the
// on-disk path layout is alias-relative: `/processes/...` is the alias
// and the caller's `MountView` resolves the leading segment to a
// tenant/user-scoped target. The test fixtures below construct a
// canonical MountView pointing the `/processes` alias at
// `/engine/tenants/<tenant>/users/<user>/processes` so existing tests
// that drive the store across multiple scope objects (different
// `tenant_id`/`user_id`) still exercise the post-read
// `same_scope_owner` check — even though the on-disk record lives at
// the *mount's* tenant/user, not the request scope's.

/// Canonical `/processes` mount target for the default test scope
/// (`tenant1` / `user1`). Tests that drive cross-tenant filtering via
/// the in-record `scope` field rely on this default — see
/// [`engine_filesystem`].
const DEFAULT_TEST_MOUNT_TENANT: &str = "tenant1";
const DEFAULT_TEST_MOUNT_USER: &str = "user1";

fn default_mount_target_string() -> String {
    format!("/engine/tenants/{DEFAULT_TEST_MOUNT_TENANT}/users/{DEFAULT_TEST_MOUNT_USER}/processes")
}

/// The production `FilesystemProcessStore` over a fresh in-memory backend — the
/// drop-in for the deleted `InMemoryProcessStore` (arch-simplification §4.3).
/// Single fixed `/processes` mount: isolates by agent/project/mission/thread
/// (encoded in the path) but not tenant/user (mount-scoped); cross-tenant
/// isolation is exercised by `filesystem_process_store_isolates_two_tenants_*`.
fn in_memory_process_store() -> FilesystemProcessStore<InMemoryBackend> {
    FilesystemProcessStore::new(scoped_processes_filesystem(
        Arc::new(InMemoryBackend::new()),
        &default_mount_target_string(),
    ))
}

/// The production `FilesystemProcessResultStore` over a fresh in-memory backend
/// — the drop-in for the deleted `InMemoryProcessResultStore`.
fn in_memory_process_result_store() -> FilesystemProcessResultStore<InMemoryBackend> {
    FilesystemProcessResultStore::new(scoped_processes_filesystem(
        Arc::new(InMemoryBackend::new()),
        &default_mount_target_string(),
    ))
}

fn stored_process_output_path(scope: &ResourceScope, process_id: ProcessId) -> VirtualPath {
    VirtualPath::new(format!(
        "{}/outputs/{process_id}/output.json",
        stored_process_owner_root(scope)
    ))
    .unwrap()
}

/// Resolved on-disk root for the default test mount. Tenant/user come
/// from the *mount* (always `tenant1`/`user1` in this fixture); the
/// scope's sub-axes (agent/project/mission/thread) come from the
/// request scope.
fn stored_process_owner_root(scope: &ResourceScope) -> String {
    let mut base = default_mount_target_string();
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

/// Build a `Arc<ScopedFilesystem<LocalFilesystem>>` over a fresh tempdir
/// mounted at `/engine`, with the `/processes` alias resolving to the
/// default tenant1/user1 target. Tests that need a different mount
/// target (e.g. cross-tenant isolation tests) construct a
/// `ScopedFilesystem` directly with their own `MountView`.
fn engine_filesystem() -> Arc<ScopedFilesystem<LocalFilesystem>> {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    let backend = Arc::new(fs);
    scoped_processes_filesystem(backend, &default_mount_target_string())
}

/// Wrap a raw `RootFilesystem` backend in a `ScopedFilesystem` granting
/// full read/write/list/delete on the `/processes` alias mapped to
/// `target_root`. Used both by the default fixture above and by the
/// cross-tenant isolation regression tests below that need to wire two
/// different mount targets over one shared backend.
fn scoped_processes_filesystem<F>(backend: Arc<F>, target_root: &str) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").expect("alias"),
        VirtualPath::new(target_root).expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

/// Alias-relative [`ScopedPath`] for a lifecycle record. Used by tests
/// that inject a forged record body via the scoped filesystem so the
/// production code path is still exercised on the read side.
fn scoped_record_path(scope: &ResourceScope, process_id: ProcessId) -> ScopedPath {
    ScopedPath::new(format!(
        "{}/records/{process_id}.json",
        alias_relative_owner_root(scope)
    ))
    .expect("scoped record path")
}

/// Alias-relative [`ScopedPath`] for a result record (sibling helper of
/// [`scoped_record_path`]).
fn scoped_result_path(scope: &ResourceScope, process_id: ProcessId) -> ScopedPath {
    ScopedPath::new(format!(
        "{}/results/{process_id}.json",
        alias_relative_owner_root(scope)
    ))
    .expect("scoped result path")
}

/// Build the alias-relative `/processes/...` owner prefix for a request
/// scope. Mirrors the production `scope_owner_root_string` in
/// `filesystem_store.rs` but lives in test code so a drift between
/// production and fixture path layouts shows up as a test failure.
fn alias_relative_owner_root(scope: &ResourceScope) -> String {
    let mut base = String::from("/processes");
    if let Some(agent_id) = &scope.agent_id {
        base.push_str("/agents/");
        base.push_str(agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        base.push_str("/projects/");
        base.push_str(project_id.as_str());
    }
    if let Some(mission_id) = &scope.mission_id {
        base.push_str("/missions/");
        base.push_str(mission_id.as_str());
    }
    if let Some(thread_id) = &scope.thread_id {
        base.push_str("/threads/");
        base.push_str(thread_id.as_str());
    }
    base
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

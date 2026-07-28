use std::{collections::HashMap, sync::Arc};

use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExtensionId,
    GrantConstraints, InvocationId, MountAlias, MountGrant, MountPermissions, MountView,
    NetworkPolicy, Principal, ProcessId, ProjectId, ResourceEstimate, ResourceScope, RuntimeKind,
    TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_processes::{
    ProcessError, ProcessResultStore, ProcessResultStorePort, ProcessStart, ProcessStatus,
    ProcessStore, ProcessStorePort,
};

type ProcessFs = ScopedFilesystem<InMemoryBackend>;
type ProcessLifecycleStore = ProcessStore<InMemoryBackend>;
type ResultStore = ProcessResultStore<InMemoryBackend>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedProcess {
    scope_idx: usize,
    status: ProcessStatus,
    error_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedResult {
    scope_idx: usize,
    status: ProcessStatus,
    output: Option<serde_json::Value>,
    error_kind: Option<String>,
}

#[derive(Debug, Default)]
struct ProcessReferenceModel {
    processes: HashMap<ProcessId, ExpectedProcess>,
    results: HashMap<ProcessId, ExpectedResult>,
}

impl ProcessReferenceModel {
    fn start(&mut self, process_id: ProcessId, scope_idx: usize) {
        self.processes.insert(
            process_id,
            ExpectedProcess {
                scope_idx,
                status: ProcessStatus::Running,
                error_kind: None,
            },
        );
    }

    fn complete(&mut self, process_id: ProcessId) {
        self.terminal(process_id, ProcessStatus::Completed, None);
    }

    fn fail(&mut self, process_id: ProcessId, error_kind: &str) {
        self.terminal(
            process_id,
            ProcessStatus::Failed,
            Some(error_kind.to_string()),
        );
    }

    fn kill(&mut self, process_id: ProcessId) {
        self.terminal(process_id, ProcessStatus::Killed, None);
    }

    fn result_complete(
        &mut self,
        process_id: ProcessId,
        scope_idx: usize,
        output: serde_json::Value,
    ) {
        self.results.insert(
            process_id,
            ExpectedResult {
                scope_idx,
                status: ProcessStatus::Completed,
                output: Some(output),
                error_kind: None,
            },
        );
    }

    fn result_fail(&mut self, process_id: ProcessId, scope_idx: usize, error_kind: &str) {
        self.results.insert(
            process_id,
            ExpectedResult {
                scope_idx,
                status: ProcessStatus::Failed,
                output: None,
                error_kind: Some(error_kind.to_string()),
            },
        );
    }

    fn result_kill(&mut self, process_id: ProcessId, scope_idx: usize) {
        self.results.insert(
            process_id,
            ExpectedResult {
                scope_idx,
                status: ProcessStatus::Killed,
                output: None,
                error_kind: None,
            },
        );
    }

    fn terminal(
        &mut self,
        process_id: ProcessId,
        status: ProcessStatus,
        error_kind: Option<String>,
    ) {
        let process = self
            .processes
            .get_mut(&process_id)
            .expect("terminal process must exist in model");
        assert_eq!(process.status, ProcessStatus::Running);
        process.status = status;
        process.error_kind = error_kind;
    }
}

fn process_fs() -> Arc<ProcessFs> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").unwrap(),
        VirtualPath::new("/engine/processes").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

fn stores(fs: Arc<ProcessFs>) -> (ProcessLifecycleStore, ResultStore) {
    (
        ProcessStore::new(Arc::clone(&fs)),
        ProcessResultStore::new(fs),
    )
}

fn scope(thread: &str, project: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-process-reference").unwrap(),
        user_id: UserId::new("user-process-reference").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new(project).unwrap()),
        mission_id: None,
        thread_id: Some(ThreadId::new(thread).unwrap()),
        invocation_id: InvocationId::new(),
    }
}

fn scopes() -> Vec<ResourceScope> {
    vec![
        scope("thread-a", "project-a"),
        scope("thread-b", "project-b"),
        scope("thread-c", "project-c"),
    ]
}

fn process_start(process_id: ProcessId, scope: ResourceScope) -> ProcessStart {
    ProcessStart {
        process_id,
        parent_process_id: None,
        invocation_id: scope.invocation_id,
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
        estimated_resources: ResourceEstimate::default(),
        resource_reservation_id: None,
        authorized_continuation: None,
        input: serde_json::json!({"message": "runtime payload"}),
    }
}

fn process_projection(
    records: Vec<ironclaw_processes::ProcessRecord>,
) -> HashMap<ProcessId, (ProcessStatus, Option<String>)> {
    records
        .into_iter()
        .map(|record| (record.process_id, (record.status, record.error_kind)))
        .collect()
}

async fn assert_stores_match_model(
    lifecycle_store: &ProcessLifecycleStore,
    result_store: &ResultStore,
    model: &ProcessReferenceModel,
    scopes: &[ResourceScope],
    label: &str,
) {
    for (scope_idx, scope) in scopes.iter().enumerate() {
        let expected = model
            .processes
            .iter()
            .filter_map(|(process_id, process)| {
                (process.scope_idx == scope_idx)
                    .then_some((*process_id, (process.status, process.error_kind.clone())))
            })
            .collect::<HashMap<_, _>>();
        let actual = process_projection(lifecycle_store.records_for_scope(scope).await.unwrap());
        assert_eq!(actual, expected, "{label}: records_for_scope {scope_idx}");
    }

    for (process_id, process) in &model.processes {
        let record = lifecycle_store
            .get(&scopes[process.scope_idx], *process_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.status, process.status, "{label}: status");
        assert_eq!(record.error_kind, process.error_kind, "{label}: error kind");
        let wrong_scope_idx = (process.scope_idx + 1) % scopes.len();
        assert!(
            lifecycle_store
                .get(&scopes[wrong_scope_idx], *process_id)
                .await
                .unwrap()
                .is_none(),
            "{label}: wrong-scope lifecycle lookup must be hidden"
        );
    }

    for (process_id, result) in &model.results {
        let record = result_store
            .get(&scopes[result.scope_idx], *process_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.status, result.status, "{label}: result status");
        assert_eq!(
            record.error_kind, result.error_kind,
            "{label}: result error"
        );
        assert_eq!(
            result_store
                .output(&scopes[result.scope_idx], *process_id)
                .await
                .unwrap(),
            result.output,
            "{label}: output"
        );
        let wrong_scope_idx = (result.scope_idx + 1) % scopes.len();
        assert!(
            result_store
                .get(&scopes[wrong_scope_idx], *process_id)
                .await
                .unwrap()
                .is_none(),
            "{label}: wrong-scope result lookup must be hidden"
        );
    }
}

async fn assert_process_projection_unchanged_after<Fut>(
    lifecycle_store: &ProcessLifecycleStore,
    result_store: &ResultStore,
    model: &ProcessReferenceModel,
    scopes: &[ResourceScope],
    label: &str,
    operation: impl FnOnce() -> Fut,
) where
    Fut: std::future::Future<Output = ()>,
{
    assert_stores_match_model(lifecycle_store, result_store, model, scopes, label).await;
    operation().await;
    assert_stores_match_model(lifecycle_store, result_store, model, scopes, label).await;
}

#[tokio::test]
async fn process_store_lifecycle_and_results_match_reference_model() {
    let fs = process_fs();
    let (lifecycle_store, result_store) = stores(Arc::clone(&fs));
    let scopes = scopes();
    let mut model = ProcessReferenceModel::default();

    let completed = ProcessId::new();
    lifecycle_store
        .start(process_start(completed, scopes[0].clone()))
        .await
        .unwrap();
    model.start(completed, 0);
    assert_stores_match_model(
        &lifecycle_store,
        &result_store,
        &model,
        &scopes,
        "start-completed",
    )
    .await;

    assert_process_projection_unchanged_after(
        &lifecycle_store,
        &result_store,
        &model,
        &scopes,
        "duplicate-start",
        || async {
            assert!(matches!(
                lifecycle_store
                    .start(process_start(completed, scopes[0].clone()))
                    .await,
                Err(ProcessError::ProcessAlreadyExists { process_id }) if process_id == completed
            ));
        },
    )
    .await;

    let completed_output = serde_json::json!({"ok": true});
    result_store
        .complete(&scopes[0], completed, completed_output.clone())
        .await
        .unwrap();
    model.result_complete(completed, 0, completed_output);
    lifecycle_store
        .complete(&scopes[0], completed)
        .await
        .unwrap();
    model.complete(completed);
    assert_stores_match_model(&lifecycle_store, &result_store, &model, &scopes, "complete").await;

    assert_process_projection_unchanged_after(
        &lifecycle_store,
        &result_store,
        &model,
        &scopes,
        "complete-terminal-rejects-fail",
        || async {
            assert!(matches!(
                lifecycle_store
                    .fail(&scopes[0], completed, "late_failure".to_string())
                    .await,
                Err(ProcessError::InvalidTransition { .. })
            ));
        },
    )
    .await;

    let failed = ProcessId::new();
    lifecycle_store
        .start(process_start(failed, scopes[1].clone()))
        .await
        .unwrap();
    model.start(failed, 1);
    result_store
        .fail(&scopes[1], failed, "process_reference_failure".to_string())
        .await
        .unwrap();
    model.result_fail(failed, 1, "Unclassified");
    lifecycle_store
        .fail(&scopes[1], failed, "process_reference_failure".to_string())
        .await
        .unwrap();
    model.fail(failed, "Unclassified");
    assert_stores_match_model(&lifecycle_store, &result_store, &model, &scopes, "fail").await;

    let killed = ProcessId::new();
    lifecycle_store
        .start(process_start(killed, scopes[2].clone()))
        .await
        .unwrap();
    model.start(killed, 2);
    result_store.kill(&scopes[2], killed).await.unwrap();
    model.result_kill(killed, 2);
    lifecycle_store.kill(&scopes[2], killed).await.unwrap();
    model.kill(killed);
    assert_stores_match_model(&lifecycle_store, &result_store, &model, &scopes, "kill").await;

    let (reopened_lifecycle, reopened_results) = stores(fs);
    assert_stores_match_model(
        &reopened_lifecycle,
        &reopened_results,
        &model,
        &scopes,
        "reopen",
    )
    .await;
}

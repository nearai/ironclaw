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

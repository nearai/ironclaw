use ironclaw_host_api::{
    AgentId, InvocationId, MissionId, ProjectId, ResourceScope, SecretHandle, TenantId, ThreadId,
    UserId,
};
use ironclaw_secrets::{InMemorySecretStore, SecretLeaseStatus, SecretMaterial, SecretStore};
use secrecy::ExposeSecret;

#[tokio::test]
async fn secret_store_returns_metadata_without_secret_material() {
    let store = InMemorySecretStore::new();
    let scope = sample_scope("tenant-a", "user-a");
    let handle = SecretHandle::new("github_token").unwrap();

    let metadata = store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("ghp_secret_token"),
        )
        .await
        .unwrap();

    assert_eq!(metadata.scope, scope);
    assert_eq!(metadata.handle, handle);
    assert!(!format!("{metadata:?}").contains("ghp_secret_token"));
}

#[tokio::test]
async fn secret_store_consumes_one_shot_secret_lease() {
    let store = InMemorySecretStore::new();
    let scope = sample_scope("tenant-a", "user-a");
    let handle = SecretHandle::new("api_key").unwrap();
    store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("super-secret"),
        )
        .await
        .unwrap();

    let lease = store.lease_once(&scope, &handle).await.unwrap();
    assert_eq!(lease.scope, scope);
    assert_eq!(lease.handle, handle);
    assert_eq!(lease.status, SecretLeaseStatus::Active);

    let value = store.consume(&scope, lease.id).await.unwrap();
    assert_eq!(value.expose_secret(), "super-secret");

    let second = store.consume(&scope, lease.id).await.unwrap_err();
    assert!(second.is_consumed());
}

#[tokio::test]
async fn secret_store_isolates_same_handle_between_tenants() {
    let store = InMemorySecretStore::new();
    let tenant_a = sample_scope("tenant-a", "user-a");
    let tenant_b = sample_scope("tenant-b", "user-a");
    let handle = SecretHandle::new("shared_name").unwrap();
    store
        .put(
            tenant_a.clone(),
            handle.clone(),
            SecretMaterial::from("tenant-a-secret"),
        )
        .await
        .unwrap();
    store
        .put(
            tenant_b.clone(),
            handle.clone(),
            SecretMaterial::from("tenant-b-secret"),
        )
        .await
        .unwrap();

    let tenant_a_lease = store.lease_once(&tenant_a, &handle).await.unwrap();
    let tenant_b_lease = store.lease_once(&tenant_b, &handle).await.unwrap();

    assert_eq!(
        store
            .consume(&tenant_a, tenant_a_lease.id)
            .await
            .unwrap()
            .expose_secret(),
        "tenant-a-secret"
    );
    assert_eq!(
        store
            .consume(&tenant_b, tenant_b_lease.id)
            .await
            .unwrap()
            .expose_secret(),
        "tenant-b-secret"
    );

    let cross_scope = store
        .consume(&tenant_b, tenant_a_lease.id)
        .await
        .unwrap_err();
    assert!(cross_scope.is_unknown_lease());
}

#[tokio::test]
async fn secret_store_isolates_same_handle_between_users_and_projects() {
    let store = InMemorySecretStore::new();
    let user_a = sample_scope("tenant-a", "user-a");
    let user_b = sample_scope("tenant-a", "user-b");
    let project_b = ResourceScope {
        project_id: Some(ProjectId::new("project-b").unwrap()),
        ..sample_scope("tenant-a", "user-a")
    };
    let handle = SecretHandle::new("shared_name").unwrap();
    store
        .put(
            user_a.clone(),
            handle.clone(),
            SecretMaterial::from("user-a-project-a-secret"),
        )
        .await
        .unwrap();
    store
        .put(
            user_b.clone(),
            handle.clone(),
            SecretMaterial::from("user-b-project-a-secret"),
        )
        .await
        .unwrap();
    store
        .put(
            project_b.clone(),
            handle.clone(),
            SecretMaterial::from("user-a-project-b-secret"),
        )
        .await
        .unwrap();

    let user_a_lease = store.lease_once(&user_a, &handle).await.unwrap();
    let user_b_lease = store.lease_once(&user_b, &handle).await.unwrap();
    let project_b_lease = store.lease_once(&project_b, &handle).await.unwrap();

    assert_eq!(
        store
            .consume(&user_a, user_a_lease.id)
            .await
            .unwrap()
            .expose_secret(),
        "user-a-project-a-secret"
    );
    assert_eq!(
        store
            .consume(&user_b, user_b_lease.id)
            .await
            .unwrap()
            .expose_secret(),
        "user-b-project-a-secret"
    );
    assert_eq!(
        store
            .consume(&project_b, project_b_lease.id)
            .await
            .unwrap()
            .expose_secret(),
        "user-a-project-b-secret"
    );

    let cross_user = store.consume(&user_b, user_a_lease.id).await.unwrap_err();
    assert!(cross_user.is_unknown_lease());
    let cross_project = store
        .consume(&project_b, user_a_lease.id)
        .await
        .unwrap_err();
    assert!(cross_project.is_unknown_lease());
}

#[tokio::test]
async fn secret_store_isolates_same_handle_between_agents() {
    let store = InMemorySecretStore::new();
    let mut agent_a = sample_scope("tenant-a", "user-a");
    agent_a.agent_id = Some(AgentId::new("agent-a").unwrap());
    let mut agent_b = agent_a.clone();
    agent_b.agent_id = Some(AgentId::new("agent-b").unwrap());
    let handle = SecretHandle::new("shared_name").unwrap();
    store
        .put(
            agent_a.clone(),
            handle.clone(),
            SecretMaterial::from("agent-a-secret"),
        )
        .await
        .unwrap();
    store
        .put(
            agent_b.clone(),
            handle.clone(),
            SecretMaterial::from("agent-b-secret"),
        )
        .await
        .unwrap();

    let agent_a_lease = store.lease_once(&agent_a, &handle).await.unwrap();
    let agent_b_lease = store.lease_once(&agent_b, &handle).await.unwrap();

    let cross_agent = store.consume(&agent_b, agent_a_lease.id).await.unwrap_err();
    assert!(cross_agent.is_unknown_lease());
    assert_eq!(
        store
            .consume(&agent_a, agent_a_lease.id)
            .await
            .unwrap()
            .expose_secret(),
        "agent-a-secret"
    );
    assert_eq!(
        store
            .consume(&agent_b, agent_b_lease.id)
            .await
            .unwrap()
            .expose_secret(),
        "agent-b-secret"
    );
}

#[tokio::test]
async fn revoked_secret_lease_cannot_be_consumed() {
    let store = InMemorySecretStore::new();
    let scope = sample_scope("tenant-a", "user-a");
    let handle = SecretHandle::new("api_key").unwrap();
    store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("super-secret"),
        )
        .await
        .unwrap();

    let lease = store.lease_once(&scope, &handle).await.unwrap();
    let revoked = store.revoke(&scope, lease.id).await.unwrap();
    assert_eq!(revoked.status, SecretLeaseStatus::Revoked);

    let error = store.consume(&scope, lease.id).await.unwrap_err();
    assert!(error.is_revoked());
}

#[tokio::test]
async fn missing_secret_fails_without_creating_lease() {
    let store = InMemorySecretStore::new();
    let scope = sample_scope("tenant-a", "user-a");
    let handle = SecretHandle::new("missing").unwrap();

    let error = store.lease_once(&scope, &handle).await.unwrap_err();
    assert!(error.is_unknown_secret());
    assert_eq!(store.leases_for_scope(&scope).await.unwrap(), Vec::new());
}

fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}

use std::sync::Arc;

use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::{
    AgentId, HostPath, InvocationId, MissionId, ProjectId, ResourceScope, SecretHandle, TenantId,
    ThreadId, UserId, VirtualPath,
};
use ironclaw_secrets::{
    EncryptedSecretRepository, EncryptedSecretStore, FilesystemEncryptedSecretRepository,
    SecretMaterial, SecretStore, SecretsCrypto,
};
use secrecy::ExposeSecret;
use tempfile::tempdir;

#[tokio::test]
async fn filesystem_secret_repository_persists_encrypted_records_without_plaintext() {
    let storage = tempdir().unwrap();
    let root = local_engine_root(storage.path());
    let repository = Arc::new(FilesystemEncryptedSecretRepository::new(root.clone()));
    let store = EncryptedSecretStore::new(repository.clone(), crypto());
    let scope = sample_scope("tenant-a", "user-a", Some("project-a"));
    let handle = SecretHandle::new("github_token").unwrap();

    store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("ghp_plaintext_must_not_persist"),
        )
        .await
        .unwrap();

    let record_path = repository.record_path(&scope, &handle).unwrap();
    let bytes = root.read_file(&record_path).await.unwrap();
    let raw_json = String::from_utf8(bytes).unwrap();
    assert!(!raw_json.contains("ghp_plaintext_must_not_persist"));
    assert!(raw_json.contains("encrypted_value"));

    let lease = store.lease_once(&scope, &handle).await.unwrap();
    let material = store.consume(&scope, lease.id).await.unwrap();
    assert_eq!(material.expose_secret(), "ghp_plaintext_must_not_persist");
}

#[tokio::test]
async fn filesystem_secret_repository_survives_new_store_instance_over_same_root() {
    let storage = tempdir().unwrap();
    let root = local_engine_root(storage.path());
    let scope = sample_scope("tenant-a", "user-a", Some("project-a"));
    let handle = SecretHandle::new("api_key").unwrap();

    let writer = EncryptedSecretStore::new(
        Arc::new(FilesystemEncryptedSecretRepository::new(root.clone())),
        crypto(),
    );
    writer
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("persisted-secret"),
        )
        .await
        .unwrap();

    let reader = EncryptedSecretStore::new(
        Arc::new(FilesystemEncryptedSecretRepository::new(root)),
        crypto(),
    );
    let lease = reader.lease_once(&scope, &handle).await.unwrap();
    let material = reader.consume(&scope, lease.id).await.unwrap();

    assert_eq!(material.expose_secret(), "persisted-secret");
}

#[tokio::test]
async fn filesystem_secret_repository_partitions_records_by_agent_scope() {
    let storage = tempdir().unwrap();
    let root = local_engine_root(storage.path());
    let repository = Arc::new(FilesystemEncryptedSecretRepository::new(root));
    let store = EncryptedSecretStore::new(repository.clone(), crypto());
    let agent_a = sample_scope_with_agent("tenant-a", "user-a", Some("agent-a"), Some("project-a"));
    let agent_b = sample_scope_with_agent("tenant-a", "user-a", Some("agent-b"), Some("project-a"));
    let no_agent = sample_scope_with_agent("tenant-a", "user-a", None, Some("project-a"));
    let handle = SecretHandle::new("shared_token").unwrap();

    assert_eq!(
        repository.record_path(&agent_a, &handle).unwrap().as_str(),
        "/engine/tenants/tenant-a/users/user-a/agents/agent-a/projects/project-a/secrets/shared_token.json"
    );
    assert_eq!(
        repository.record_path(&no_agent, &handle).unwrap().as_str(),
        "/engine/tenants/tenant-a/users/user-a/agents/_none/projects/project-a/secrets/shared_token.json"
    );

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
    store
        .put(
            no_agent.clone(),
            handle.clone(),
            SecretMaterial::from("no-agent-secret"),
        )
        .await
        .unwrap();

    assert_eq!(repository.list(&agent_a).await.unwrap().len(), 1);
    assert_eq!(repository.list(&agent_b).await.unwrap().len(), 1);
    assert_eq!(repository.list(&no_agent).await.unwrap().len(), 1);

    let agent_a_lease = store.lease_once(&agent_a, &handle).await.unwrap();
    let agent_b_lease = store.lease_once(&agent_b, &handle).await.unwrap();
    let no_agent_lease = store.lease_once(&no_agent, &handle).await.unwrap();

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
    assert_eq!(
        store
            .consume(&no_agent, no_agent_lease.id)
            .await
            .unwrap()
            .expose_secret(),
        "no-agent-secret"
    );
}

#[tokio::test]
async fn filesystem_secret_repository_isolates_scope_and_lists_only_visible_records() {
    let storage = tempdir().unwrap();
    let root = local_engine_root(storage.path());
    let repository = Arc::new(FilesystemEncryptedSecretRepository::new(root));
    let store = EncryptedSecretStore::new(repository.clone(), crypto());
    let tenant_a = sample_scope("tenant-a", "user-a", Some("project-a"));
    let tenant_b = sample_scope("tenant-b", "user-a", Some("project-a"));
    let project_b = sample_scope("tenant-a", "user-a", Some("project-b"));
    let handle = SecretHandle::new("shared_token").unwrap();

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
    store
        .put(
            project_b.clone(),
            handle.clone(),
            SecretMaterial::from("project-b-secret"),
        )
        .await
        .unwrap();

    assert_eq!(repository.list(&tenant_a).await.unwrap().len(), 1);
    assert_eq!(repository.list(&tenant_b).await.unwrap().len(), 1);
    assert_eq!(repository.list(&project_b).await.unwrap().len(), 1);

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
}

#[tokio::test]
async fn filesystem_secret_repository_any_exist_ignores_unrelated_engine_json() {
    let storage = tempdir().unwrap();
    let root = local_engine_root(storage.path());
    let repository = FilesystemEncryptedSecretRepository::new(root.clone());
    root.write_file(
        &VirtualPath::new("/engine/tenants/tenant-a/users/user-a/processes/process-a.json")
            .unwrap(),
        br#"{"not":"a secret record"}"#,
    )
    .await
    .unwrap();

    assert!(!repository.any_exist().await.unwrap());
}

#[tokio::test]
async fn filesystem_secret_repository_reads_legacy_no_agent_paths() {
    let storage = tempdir().unwrap();
    let root = local_engine_root(storage.path());
    let repository = Arc::new(FilesystemEncryptedSecretRepository::new(root.clone()));
    let store = EncryptedSecretStore::new(repository.clone(), crypto());
    let scope = sample_scope("tenant-a", "user-a", Some("project-a"));
    let handle = SecretHandle::new("legacy_token").unwrap();

    store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("legacy-secret"),
        )
        .await
        .unwrap();
    let canonical_path = repository.record_path(&scope, &handle).unwrap();
    let legacy_path = VirtualPath::new(
        "/engine/tenants/tenant-a/users/user-a/projects/project-a/secrets/legacy_token.json",
    )
    .unwrap();
    let bytes = root.read_file(&canonical_path).await.unwrap();
    root.write_file(&legacy_path, &bytes).await.unwrap();
    root.delete(&canonical_path).await.unwrap();

    assert!(repository.get(&scope, &handle).await.unwrap().is_some());
    assert_eq!(repository.list(&scope).await.unwrap().len(), 1);

    let reader = EncryptedSecretStore::new(repository.clone(), crypto());
    let lease = reader.lease_once(&scope, &handle).await.unwrap();
    let material = reader.consume(&scope, lease.id).await.unwrap();
    assert_eq!(material.expose_secret(), "legacy-secret");

    assert!(repository.delete(&scope, &handle).await.unwrap());
    assert!(repository.get(&scope, &handle).await.unwrap().is_none());
}

#[tokio::test]
async fn filesystem_secret_repository_records_usage_and_tombstones_delete() {
    let storage = tempdir().unwrap();
    let root = local_engine_root(storage.path());
    let repository = Arc::new(FilesystemEncryptedSecretRepository::new(root.clone()));
    let store = EncryptedSecretStore::new(repository.clone(), crypto());
    let scope = sample_scope("tenant-a", "user-a", Some("project-a"));
    let handle = SecretHandle::new("api_key").unwrap();

    store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("secret-value"),
        )
        .await
        .unwrap();
    let lease = store.lease_once(&scope, &handle).await.unwrap();
    store.consume(&scope, lease.id).await.unwrap();

    let used = repository.get(&scope, &handle).await.unwrap().unwrap();
    assert_eq!(used.metadata.usage_count, 1);
    assert!(used.metadata.last_used_at.is_some());

    assert!(repository.delete(&scope, &handle).await.unwrap());
    assert!(repository.get(&scope, &handle).await.unwrap().is_none());
    assert!(repository.list(&scope).await.unwrap().is_empty());
    assert!(!repository.any_exist().await.unwrap());

    let raw_tombstone = root
        .read_file(&repository.record_path(&scope, &handle).unwrap())
        .await
        .unwrap();
    let raw_tombstone = String::from_utf8(raw_tombstone).unwrap();
    assert!(raw_tombstone.contains("\"deleted\": true"));
    assert!(!raw_tombstone.contains("secret-value"));
}

#[cfg(feature = "filesystem-libsql")]
#[test]
fn filesystem_secret_repository_accepts_libsql_root_filesystem_backend() {
    fn assert_repository<F>()
    where
        F: RootFilesystem,
        FilesystemEncryptedSecretRepository<F>: EncryptedSecretRepository,
    {
    }

    assert_repository::<ironclaw_filesystem::LibSqlRootFilesystem>();
}

#[cfg(feature = "filesystem-postgres")]
#[test]
fn filesystem_secret_repository_accepts_postgres_root_filesystem_backend() {
    fn assert_repository<F>()
    where
        F: RootFilesystem,
        FilesystemEncryptedSecretRepository<F>: EncryptedSecretRepository,
    {
    }

    assert_repository::<ironclaw_filesystem::PostgresRootFilesystem>();
}

fn local_engine_root(path: &std::path::Path) -> Arc<LocalFilesystem> {
    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(path.to_path_buf()),
    )
    .unwrap();
    Arc::new(root)
}

fn crypto() -> SecretsCrypto {
    SecretsCrypto::new(SecretMaterial::from(
        "0123456789abcdef0123456789abcdef".to_string(),
    ))
    .unwrap()
}

fn sample_scope(tenant: &str, user: &str, project: Option<&str>) -> ResourceScope {
    sample_scope_with_agent(tenant, user, None, project)
}

fn sample_scope_with_agent(
    tenant: &str,
    user: &str,
    agent: Option<&str>,
    project: Option<&str>,
) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: agent.map(|agent| AgentId::new(agent).unwrap()),
        project_id: project.map(|project| ProjectId::new(project).unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}

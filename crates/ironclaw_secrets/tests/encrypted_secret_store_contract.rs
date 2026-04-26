use std::sync::Arc;

use ironclaw_host_api::{
    InvocationId, MissionId, ProjectId, ResourceScope, SecretHandle, TenantId, ThreadId, UserId,
};
use ironclaw_secrets::{
    EncryptedSecretRepository, EncryptedSecretStore, InMemoryEncryptedSecretRepository,
    SecretMaterial, SecretStore, SecretsCrypto,
};
use secrecy::ExposeSecret;

const MASTER_KEY: &str = "0123456789abcdef0123456789abcdef";
const OTHER_MASTER_KEY: &str = "abcdef0123456789abcdef0123456789";

#[tokio::test]
async fn encrypted_secret_store_persists_ciphertext_not_plaintext() {
    let (store, repository) = encrypted_store(MASTER_KEY);
    let scope = sample_scope("tenant-a", "user-a");
    let handle = SecretHandle::new("github_token").unwrap();

    let metadata = store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("ghp_super_secret"),
        )
        .await
        .unwrap();

    assert_eq!(metadata.scope, scope);
    assert_eq!(metadata.handle, handle);
    assert_eq!(metadata.usage_count, 0);
    assert!(metadata.last_used_at.is_none());
    assert!(!format!("{metadata:?}").contains("ghp_super_secret"));

    let encrypted = repository.get(&scope, &handle).await.unwrap().unwrap();
    assert_ne!(encrypted.encrypted_value, b"ghp_super_secret");
    assert!(
        !encrypted
            .encrypted_value
            .windows(b"ghp_super_secret".len())
            .any(|window| window == b"ghp_super_secret")
    );
    assert_eq!(encrypted.key_salt.len(), 32);
    assert!(!format!("{encrypted:?}").contains("ghp_super_secret"));

    let lease = store.lease_once(&scope, &handle).await.unwrap();
    let material = store.consume(&scope, lease.id).await.unwrap();
    assert_eq!(material.expose_secret(), "ghp_super_secret");
}

#[tokio::test]
async fn encrypted_secret_store_uses_distinct_salts_for_same_plaintext() {
    let (store, repository) = encrypted_store(MASTER_KEY);
    let scope = sample_scope("tenant-a", "user-a");
    let first = SecretHandle::new("first_token").unwrap();
    let second = SecretHandle::new("second_token").unwrap();

    store
        .put(
            scope.clone(),
            first.clone(),
            SecretMaterial::from("same-secret"),
        )
        .await
        .unwrap();
    store
        .put(
            scope.clone(),
            second.clone(),
            SecretMaterial::from("same-secret"),
        )
        .await
        .unwrap();

    let first_record = repository.get(&scope, &first).await.unwrap().unwrap();
    let second_record = repository.get(&scope, &second).await.unwrap().unwrap();

    assert_ne!(first_record.key_salt, second_record.key_salt);
    assert_ne!(first_record.encrypted_value, second_record.encrypted_value);
}

#[tokio::test]
async fn encrypted_secret_store_survives_new_store_instance_with_same_repository_and_key() {
    let repository = Arc::new(InMemoryEncryptedSecretRepository::new());
    let scope = sample_scope("tenant-a", "user-a");
    let handle = SecretHandle::new("api_key").unwrap();

    let writer = EncryptedSecretStore::new(repository.clone(), crypto(MASTER_KEY));
    writer
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("persisted-secret"),
        )
        .await
        .unwrap();

    let reader = EncryptedSecretStore::new(repository, crypto(MASTER_KEY));
    let lease = reader.lease_once(&scope, &handle).await.unwrap();
    let material = reader.consume(&scope, lease.id).await.unwrap();

    assert_eq!(material.expose_secret(), "persisted-secret");
}

#[tokio::test]
async fn encrypted_secret_store_rejects_wrong_master_key_without_consuming_lease() {
    let repository = Arc::new(InMemoryEncryptedSecretRepository::new());
    let scope = sample_scope("tenant-a", "user-a");
    let handle = SecretHandle::new("api_key").unwrap();

    let writer = EncryptedSecretStore::new(repository.clone(), crypto(MASTER_KEY));
    writer
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("persisted-secret"),
        )
        .await
        .unwrap();

    let reader = EncryptedSecretStore::new(repository, crypto(OTHER_MASTER_KEY));
    let lease = reader.lease_once(&scope, &handle).await.unwrap();
    let error = reader.consume(&scope, lease.id).await.unwrap_err();

    assert!(error.is_decryption_failed());
    let leases = reader.leases_for_scope(&scope).await.unwrap();
    assert_eq!(
        leases[0].status,
        ironclaw_secrets::SecretLeaseStatus::Active
    );
}

#[tokio::test]
async fn encrypted_secret_store_records_usage_after_successful_consume() {
    let (store, repository) = encrypted_store(MASTER_KEY);
    let scope = sample_scope("tenant-a", "user-a");
    let handle = SecretHandle::new("api_key").unwrap();

    store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("secret-value"),
        )
        .await
        .unwrap();
    let before = repository.get(&scope, &handle).await.unwrap().unwrap();
    assert_eq!(before.metadata.usage_count, 0);
    assert!(before.metadata.last_used_at.is_none());

    let lease = store.lease_once(&scope, &handle).await.unwrap();
    store.consume(&scope, lease.id).await.unwrap();

    let after = repository.get(&scope, &handle).await.unwrap().unwrap();
    assert_eq!(after.metadata.usage_count, 1);
    assert!(after.metadata.last_used_at.is_some());
}

#[tokio::test]
async fn encrypted_secret_store_isolates_same_handle_between_tenants() {
    let (store, _repository) = encrypted_store(MASTER_KEY);
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
}

fn encrypted_store(
    master_key: &str,
) -> (
    EncryptedSecretStore<InMemoryEncryptedSecretRepository>,
    Arc<InMemoryEncryptedSecretRepository>,
) {
    let repository = Arc::new(InMemoryEncryptedSecretRepository::new());
    let store = EncryptedSecretStore::new(repository.clone(), crypto(master_key));
    (store, repository)
}

fn crypto(master_key: &str) -> SecretsCrypto {
    SecretsCrypto::new(SecretMaterial::from(master_key.to_string())).unwrap()
}

fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}

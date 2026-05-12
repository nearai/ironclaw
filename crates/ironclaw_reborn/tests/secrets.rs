#![cfg(feature = "libsql-secrets")]

use std::sync::Arc;

use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle, TenantId, UserId};
use ironclaw_reborn::secrets::{
    RebornLibSqlSecretStoreConfig, RebornSecretStoreError, RebornSecretStoreHealthStatus,
    build_libsql_reborn_secret_store, check_libsql_reborn_secret_store_health,
};
use ironclaw_secrets::{SecretMaterial, SecretStoreError};
use secrecy::ExposeSecret;

#[tokio::test]
async fn reborn_secret_store_health_fails_closed_without_explicit_operator_master_key() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("reborn-secrets.db");
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());

    let health = check_libsql_reborn_secret_store_health(RebornLibSqlSecretStoreConfig {
        database,
        master_key: None,
    })
    .await;

    assert_eq!(
        health.status,
        RebornSecretStoreHealthStatus::MissingMasterKey
    );
    let debug = format!("{health:?}");
    assert!(!debug.contains("0123456789abcdef"));
    assert!(!debug.contains("sk-live"));
}

#[tokio::test]
async fn reborn_secret_store_requires_explicit_operator_master_key() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("reborn-secrets.db");
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());

    let error = match build_libsql_reborn_secret_store(RebornLibSqlSecretStoreConfig {
        database,
        master_key: None,
    })
    .await
    {
        Ok(_) => panic!("secret store must not build without an explicit master key"),
        Err(error) => error,
    };

    assert!(matches!(error, RebornSecretStoreError::MissingMasterKey));
    assert!(!format!("{error:?}").contains("0123456789abcdef"));
}

#[tokio::test]
async fn reborn_secret_store_backend_unavailable_error_does_not_format_backend_details() {
    let error = RebornSecretStoreError::BackendUnavailable;

    let display = error.to_string();
    let debug = format!("{error:?}");

    assert!(!display.contains("/tmp/operator/private/reborn-secrets.db"));
    assert!(!debug.contains("/tmp/operator/private/reborn-secrets.db"));
    assert_eq!(display, "reborn secret store backend unavailable");
}

#[tokio::test]
async fn reborn_secret_store_fails_closed_when_existing_rows_use_another_master_key() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("reborn-secrets.db");
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let store = build_libsql_reborn_secret_store(RebornLibSqlSecretStoreConfig {
        database: Arc::clone(&database),
        master_key: Some(SecretMaterial::from(
            "0123456789abcdef0123456789abcdef".to_string(),
        )),
    })
    .await
    .unwrap();
    let scope = sample_scope();
    let handle = ironclaw_host_api::SecretHandle::new("openai_key").unwrap();
    store
        .put(
            scope,
            handle,
            SecretMaterial::from("sk-live-existing-secret"),
        )
        .await
        .unwrap();
    drop(store);

    let wrong_key = Some(SecretMaterial::from(
        "abcdef0123456789abcdef0123456789".to_string(),
    ));
    let error = match build_libsql_reborn_secret_store(RebornLibSqlSecretStoreConfig {
        database: Arc::clone(&database),
        master_key: wrong_key.clone(),
    })
    .await
    {
        Ok(_) => panic!("secret store must fail closed when existing rows cannot decrypt"),
        Err(error) => error,
    };
    assert!(matches!(error, RebornSecretStoreError::InvalidMasterKey));
    assert!(!format!("{error:?}").contains("sk-live-existing-secret"));

    let health = check_libsql_reborn_secret_store_health(RebornLibSqlSecretStoreConfig {
        database,
        master_key: wrong_key,
    })
    .await;
    assert_eq!(
        health.status,
        RebornSecretStoreHealthStatus::InvalidMasterKey
    );
    assert!(!format!("{health:?}").contains("sk-live-existing-secret"));
}

#[tokio::test]
async fn reborn_secret_store_reports_malformed_master_key_as_invalid_master_key() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("reborn-secrets.db");
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let short_key = Some(SecretMaterial::from("short".to_string()));

    let error = match build_libsql_reborn_secret_store(RebornLibSqlSecretStoreConfig {
        database: Arc::clone(&database),
        master_key: short_key.clone(),
    })
    .await
    {
        Ok(_) => panic!("secret store must reject malformed operator master key"),
        Err(error) => error,
    };
    assert!(matches!(error, RebornSecretStoreError::InvalidMasterKey));

    let health = check_libsql_reborn_secret_store_health(RebornLibSqlSecretStoreConfig {
        database,
        master_key: short_key,
    })
    .await;
    assert_eq!(
        health.status,
        RebornSecretStoreHealthStatus::InvalidMasterKey
    );
    assert!(!format!("{health:?}").contains("short"));
}

#[tokio::test]
async fn reborn_secret_store_health_is_ready_for_empty_and_existing_rows_with_same_key() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("reborn-secrets.db");
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let master_key = Some(test_master_key());

    let empty_health = check_libsql_reborn_secret_store_health(RebornLibSqlSecretStoreConfig {
        database: Arc::clone(&database),
        master_key: master_key.clone(),
    })
    .await;
    assert_eq!(empty_health.status, RebornSecretStoreHealthStatus::Ready);
    assert!(empty_health.reason.is_none());

    let store = build_libsql_reborn_secret_store(RebornLibSqlSecretStoreConfig {
        database: Arc::clone(&database),
        master_key: master_key.clone(),
    })
    .await
    .unwrap();
    store
        .put(
            sample_scope(),
            SecretHandle::new("openai_key").unwrap(),
            SecretMaterial::from("sk-live-health-ready-existing"),
        )
        .await
        .unwrap();
    drop(store);

    let existing_health = check_libsql_reborn_secret_store_health(RebornLibSqlSecretStoreConfig {
        database,
        master_key,
    })
    .await;
    assert_eq!(existing_health.status, RebornSecretStoreHealthStatus::Ready);
    assert!(existing_health.reason.is_none());
    assert!(!format!("{existing_health:?}").contains("sk-live-health-ready-existing"));
}

#[tokio::test]
async fn reborn_secret_store_reopens_with_scope_isolation_and_one_shot_leases() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("reborn-secrets.db");
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let master_key = Some(test_master_key());
    let store = build_libsql_reborn_secret_store(RebornLibSqlSecretStoreConfig {
        database: Arc::clone(&database),
        master_key: master_key.clone(),
    })
    .await
    .unwrap();
    let scope_a = sample_scope_for("tenant-a", "user-a");
    let scope_b = sample_scope_for("tenant-b", "user-a");
    let missing_scope = sample_scope_for("tenant-a", "user-b");
    let handle = SecretHandle::new("openai_key").unwrap();

    store
        .put(
            scope_a.clone(),
            handle.clone(),
            SecretMaterial::from("sk-live-scope-a"),
        )
        .await
        .unwrap();
    store
        .put(
            scope_b.clone(),
            handle.clone(),
            SecretMaterial::from("sk-live-scope-b"),
        )
        .await
        .unwrap();
    drop(store);

    let reopened = build_libsql_reborn_secret_store(RebornLibSqlSecretStoreConfig {
        database,
        master_key,
    })
    .await
    .unwrap();
    assert!(
        reopened
            .metadata(&scope_a, &handle)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        reopened
            .metadata(&scope_b, &handle)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        reopened
            .metadata(&missing_scope, &handle)
            .await
            .unwrap()
            .is_none()
    );

    let lease_a = reopened.lease_once(&scope_a, &handle).await.unwrap();
    assert!(matches!(
        reopened.consume(&scope_b, lease_a.id).await,
        Err(SecretStoreError::UnknownLease { .. })
    ));
    let material_a = reopened.consume(&scope_a, lease_a.id).await.unwrap();
    assert_eq!(material_a.expose_secret(), "sk-live-scope-a");
    assert!(matches!(
        reopened.consume(&scope_a, lease_a.id).await,
        Err(SecretStoreError::LeaseConsumed { .. })
    ));

    let lease_b = reopened.lease_once(&scope_b, &handle).await.unwrap();
    let material_b = reopened.consume(&scope_b, lease_b.id).await.unwrap();
    assert_eq!(material_b.expose_secret(), "sk-live-scope-b");
}

#[tokio::test]
async fn reborn_secret_store_missing_secret_does_not_create_lease() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("reborn-secrets.db");
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let store = build_libsql_reborn_secret_store(RebornLibSqlSecretStoreConfig {
        database,
        master_key: Some(test_master_key()),
    })
    .await
    .unwrap();
    let scope = sample_scope();
    let missing = SecretHandle::new("missing_key").unwrap();

    assert!(store.metadata(&scope, &missing).await.unwrap().is_none());
    let error = store
        .lease_once(&scope, &missing)
        .await
        .expect_err("missing durable secret must not lease");
    assert!(matches!(error, SecretStoreError::UnknownSecret { .. }));
    assert!(store.leases_for_scope(&scope).await.unwrap().is_empty());
}

#[tokio::test]
async fn reborn_secret_store_active_lease_survives_secret_rotation() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("reborn-secrets.db");
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let store = build_libsql_reborn_secret_store(RebornLibSqlSecretStoreConfig {
        database,
        master_key: Some(SecretMaterial::from(
            "0123456789abcdef0123456789abcdef".to_string(),
        )),
    })
    .await
    .unwrap();
    let scope = sample_scope();
    let handle = ironclaw_host_api::SecretHandle::new("openai_key").unwrap();

    store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("sk-live-before-rotation"),
        )
        .await
        .unwrap();
    let lease = store.lease_once(&scope, &handle).await.unwrap();
    store
        .put(
            scope.clone(),
            handle,
            SecretMaterial::from("sk-live-after-rotation"),
        )
        .await
        .unwrap();

    let material = store.consume(&scope, lease.id).await.unwrap();
    assert_eq!(material.expose_secret(), "sk-live-after-rotation");
}

#[tokio::test]
async fn reborn_secret_store_persists_material_encrypted_and_exposes_only_through_secret_store() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("reborn-secrets.db");
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let store = build_libsql_reborn_secret_store(RebornLibSqlSecretStoreConfig {
        database,
        master_key: Some(SecretMaterial::from(
            "0123456789abcdef0123456789abcdef".to_string(),
        )),
    })
    .await
    .unwrap();
    let scope = sample_scope();
    let handle = ironclaw_host_api::SecretHandle::new("openai_key").unwrap();

    store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("sk-live-reborn-secret-parity"),
        )
        .await
        .unwrap();
    let raw_database = String::from_utf8_lossy(&std::fs::read(&db_path).unwrap()).to_string();
    assert!(!raw_database.contains("sk-live-reborn-secret-parity"));

    let lease = store.lease_once(&scope, &handle).await.unwrap();
    let material = store.consume(&scope, lease.id).await.unwrap();
    assert_eq!(material.expose_secret(), "sk-live-reborn-secret-parity");
}

fn test_master_key() -> SecretMaterial {
    SecretMaterial::from("0123456789abcdef0123456789abcdef".to_string())
}

fn sample_scope() -> ResourceScope {
    sample_scope_for("tenant-a", "user-a")
}

fn sample_scope_for(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

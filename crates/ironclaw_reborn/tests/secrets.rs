#![cfg(feature = "libsql-secrets")]

use std::sync::Arc;

use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, UserId};
use ironclaw_reborn::secrets::{
    RebornLibSqlSecretStoreConfig, RebornSecretStoreError, RebornSecretStoreHealthStatus,
    build_libsql_reborn_secret_store, check_libsql_reborn_secret_store_health,
};
use ironclaw_secrets::SecretMaterial;
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

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

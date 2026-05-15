#![cfg(any(feature = "libsql", feature = "postgres"))]

use std::sync::Arc;

use chrono::{Duration, Utc};

#[cfg(feature = "libsql")]
use ironclaw_secrets::LibSqlSecretsStore;
#[cfg(feature = "postgres")]
use ironclaw_secrets::PostgresSecretsStore;
use ironclaw_secrets::{
    CreateSecretParams, SecretError, SecretMaterial, SecretsCrypto, SecretsStore,
    secret_record_aad, secret_store_key_check_aad,
};

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_secret_store_persists_encrypted_secret_material_across_reopen() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("secrets.db");
    let crypto = test_crypto();
    let store = libsql_store(&db_path, Arc::clone(&crypto)).await;

    store
        .create(
            "reborn-user",
            CreateSecretParams::new("openai_key", "sk-live-reborn-secret-sentinel"),
        )
        .await
        .unwrap();
    drop(store);

    let raw_database = String::from_utf8_lossy(&std::fs::read(&db_path).unwrap()).to_string();
    assert!(
        !raw_database.contains("sk-live-reborn-secret-sentinel"),
        "raw secret material must be encrypted at rest"
    );

    let reopened = libsql_store(&db_path, crypto).await;
    let decrypted = reopened
        .get_decrypted("reborn-user", "openai_key")
        .await
        .unwrap();
    assert_eq!(decrypted.expose(), "sk-live-reborn-secret-sentinel");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_secret_store_tracks_metadata_usage_overwrite_and_empty_state_edges() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("secrets.db");
    let store = libsql_store(&db_path, test_crypto()).await;

    assert!(!store.any_exist().await.unwrap());
    assert_secret_store_tracks_metadata_usage_and_overwrite_edges(&store, "reborn-user-metadata")
        .await;
    assert!(!store.any_exist().await.unwrap());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_secret_store_preserves_mismatched_and_expired_one_shot_values() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("secrets.db");
    let store = libsql_store(&db_path, test_crypto()).await;

    assert_secret_store_preserves_mismatched_and_expired_one_shot_values(
        &store,
        "reborn-user-edge",
        "oauth_state",
        "expired_state",
    )
    .await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_secret_store_enforces_access_patterns_without_exposing_missing_names() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("secrets.db");
    let store = libsql_store(&db_path, test_crypto()).await;

    assert_secret_store_enforces_access_patterns(&store, "reborn-user-access", "OpenAI_Prod_Key")
        .await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_secret_store_verify_can_decrypt_existing_secrets_rejects_wrong_key() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("secrets.db");
    let store = libsql_store(&db_path, test_crypto()).await;

    store
        .create(
            "reborn-user",
            CreateSecretParams::new("openai_key", "sk-live-wrong-key-sentinel"),
        )
        .await
        .unwrap();
    drop(store);

    let reopened = libsql_store(&db_path, wrong_crypto()).await;
    let error = reopened
        .verify_can_decrypt_existing_secrets()
        .await
        .expect_err("wrong key must fail existing row decryptability check");
    assert!(matches!(error, SecretError::DecryptionFailed(_)));
    assert!(!format!("{error:?}").contains("sk-live-wrong-key-sentinel"));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_secret_store_key_check_bootstrap_scans_all_existing_rows() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("secrets.db");
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let store = LibSqlSecretsStore::new(Arc::clone(&database), test_crypto());
    store.run_migrations().await.unwrap();
    store
        .create(
            "a-reborn-user",
            CreateSecretParams::new("a_key", "sk-live-good-row"),
        )
        .await
        .unwrap();
    let conn = database.connect().unwrap();
    insert_libsql_secret_record(
        &conn,
        "z-reborn-user",
        "z_key",
        wrong_crypto(),
        "sk-live-wrong-row",
    )
    .await;

    let error = store
        .verify_can_decrypt_existing_secrets()
        .await
        .expect_err("pre-sentinel bootstrap must validate every existing row before installing the key check");
    assert!(matches!(error, SecretError::DecryptionFailed(_)));
    assert!(!format!("{error:?}").contains("sk-live-wrong-row"));
    let mut rows = conn
        .query("SELECT 1 FROM reborn_secret_store_key_check", ())
        .await
        .unwrap();
    assert!(rows.next().await.unwrap().is_none());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_secret_store_key_check_rejects_concurrent_conflict_winner_with_wrong_key() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("secrets.db");
    let database = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let store = LibSqlSecretsStore::new(Arc::clone(&database), test_crypto());
    store.run_migrations().await.unwrap();

    let conn = database.connect().unwrap();
    conn.execute("BEGIN IMMEDIATE", ()).await.unwrap();
    insert_libsql_secret_store_key_check(&conn, test_crypto()).await;

    let losing_store = LibSqlSecretsStore::new(Arc::clone(&database), wrong_crypto());
    let verification = tokio::task::spawn_blocking(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(losing_store.verify_can_decrypt_existing_secrets())
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    conn.execute("COMMIT", ()).await.unwrap();

    let error = verification
        .await
        .unwrap()
        .expect_err("wrong key must verify the committed key-check conflict winner");
    assert!(matches!(error, SecretError::DecryptionFailed(_)));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_secret_store_consume_if_matches_is_one_shot_and_durable() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("secrets.db");
    let crypto = test_crypto();
    let store = libsql_store(&db_path, Arc::clone(&crypto)).await;

    store
        .create(
            "reborn-user",
            CreateSecretParams::new("oauth_state", "state-secret-sentinel"),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .consume_if_matches("reborn-user", "oauth_state", "wrong")
            .await
            .unwrap(),
        ironclaw_secrets::SecretConsumeResult::Mismatched
    );
    assert_eq!(
        store
            .consume_if_matches("reborn-user", "oauth_state", "state-secret-sentinel")
            .await
            .unwrap(),
        ironclaw_secrets::SecretConsumeResult::Matched
    );
    drop(store);

    let reopened = libsql_store(&db_path, crypto).await;
    assert!(!reopened.exists("reborn-user", "oauth_state").await.unwrap());
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_secret_store_persists_encrypted_secret_material_when_database_url_is_set() {
    let Some(store) = postgres_store().await else {
        return;
    };
    let suffix = unique_suffix();
    let user_id = format!("reborn-user-{suffix}");
    let secret_name = format!("openai_key_{suffix}");

    store
        .create(
            &user_id,
            CreateSecretParams::new(&secret_name, "sk-live-postgres-reborn-secret-sentinel"),
        )
        .await
        .unwrap();
    let decrypted = store.get_decrypted(&user_id, &secret_name).await.unwrap();
    assert_eq!(
        decrypted.expose(),
        "sk-live-postgres-reborn-secret-sentinel"
    );
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_secret_store_tracks_metadata_usage_and_overwrite_edges() {
    let Some(store) = postgres_store().await else {
        return;
    };
    let suffix = unique_suffix();
    let user_id = format!("reborn-user-metadata-{suffix}");

    assert_secret_store_tracks_metadata_usage_and_overwrite_edges(&store, &user_id).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_secret_store_preserves_mismatched_and_expired_one_shot_values() {
    let Some(store) = postgres_store().await else {
        return;
    };
    let suffix = unique_suffix();
    let user_id = format!("reborn-user-edge-{suffix}");
    let mismatch_name = format!("oauth_state_{suffix}");
    let expired_name = format!("expired_state_{suffix}");

    assert_secret_store_preserves_mismatched_and_expired_one_shot_values(
        &store,
        &user_id,
        &mismatch_name,
        &expired_name,
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_secret_store_enforces_access_patterns_without_exposing_missing_names() {
    let Some(store) = postgres_store().await else {
        return;
    };
    let suffix = unique_suffix();
    let user_id = format!("reborn-user-access-{suffix}");
    let secret_name = format!("OpenAI_Prod_Key_{suffix}");

    assert_secret_store_enforces_access_patterns(&store, &user_id, &secret_name).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_secret_store_verify_can_decrypt_existing_secrets_rejects_wrong_key() {
    let Some(store) = postgres_store().await else {
        return;
    };
    let suffix = unique_suffix();
    let user_id = format!("reborn-user-wrong-key-{suffix}");
    let secret_name = format!("openai_key_{suffix}");

    store
        .create(
            &user_id,
            CreateSecretParams::new(&secret_name, "sk-live-postgres-wrong-key-sentinel"),
        )
        .await
        .unwrap();
    drop(store);

    let Some(reopened) = postgres_store_with_crypto(wrong_crypto()).await else {
        return;
    };
    let error = reopened
        .verify_can_decrypt_existing_secrets()
        .await
        .expect_err("wrong key must fail existing row decryptability check");
    assert!(matches!(error, SecretError::DecryptionFailed(_)));
    assert!(!format!("{error:?}").contains("sk-live-postgres-wrong-key-sentinel"));
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_secret_store_key_check_bootstrap_scans_all_existing_rows() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let store = PostgresSecretsStore::new(pool.clone(), test_crypto());
    store.run_migrations().await.unwrap();
    let client = pool.get().await.unwrap();
    client
        .execute(
            "DELETE FROM reborn_secret_store_key_check WHERE id = $1",
            &[&"active"],
        )
        .await
        .unwrap();
    let good_user_id = format!("a-reborn-user-bootstrap-{suffix}");
    let bad_user_id = format!("z-reborn-user-bootstrap-{suffix}");
    let good_name = format!("a_key_{suffix}");
    let bad_name = format!("z_key_{suffix}");
    store
        .create(
            &good_user_id,
            CreateSecretParams::new(&good_name, "sk-live-postgres-good-row"),
        )
        .await
        .unwrap();
    insert_postgres_secret_record(
        &client,
        &bad_user_id,
        &bad_name,
        wrong_crypto(),
        "sk-live-postgres-wrong-row",
    )
    .await;

    let error = store
        .verify_can_decrypt_existing_secrets()
        .await
        .expect_err("pre-sentinel bootstrap must validate every existing row before installing the key check");
    assert!(matches!(error, SecretError::DecryptionFailed(_)));
    assert!(!format!("{error:?}").contains("sk-live-postgres-wrong-row"));
    assert!(
        client
            .query_opt(
                "SELECT 1 FROM reborn_secret_store_key_check WHERE id = $1",
                &[&"active"],
            )
            .await
            .unwrap()
            .is_none()
    );
    client
        .execute(
            "DELETE FROM reborn_secret_records WHERE user_id = ANY($1)",
            &[&vec![good_user_id, bad_user_id]],
        )
        .await
        .unwrap();
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_secret_store_key_check_rejects_concurrent_conflict_winner_with_wrong_key() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let store = PostgresSecretsStore::new(pool.clone(), test_crypto());
    store.run_migrations().await.unwrap();

    let mut client = pool.get().await.unwrap();
    client
        .execute(
            "DELETE FROM reborn_secret_store_key_check WHERE id = $1",
            &[&"active"],
        )
        .await
        .unwrap();
    let transaction = client.transaction().await.unwrap();
    insert_postgres_secret_store_key_check(&transaction, test_crypto()).await;

    let losing_store = PostgresSecretsStore::new(pool.clone(), wrong_crypto());
    let verification =
        tokio::spawn(async move { losing_store.verify_can_decrypt_existing_secrets().await });
    for _ in 0..1000 {
        tokio::task::yield_now().await;
    }
    transaction.commit().await.unwrap();

    let error = verification
        .await
        .unwrap()
        .expect_err("wrong key must verify the committed key-check conflict winner");
    assert!(matches!(error, SecretError::DecryptionFailed(_)));

    let cleanup = pool.get().await.unwrap();
    cleanup
        .execute(
            "DELETE FROM reborn_secret_store_key_check WHERE id = $1",
            &[&"active"],
        )
        .await
        .unwrap();
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_secret_store_consume_if_matches_is_one_shot_and_durable() {
    let Some(store) = postgres_store().await else {
        return;
    };
    let suffix = unique_suffix();
    let user_id = format!("reborn-user-oauth-{suffix}");
    let secret_name = format!("oauth_state_{suffix}");

    store
        .create(
            &user_id,
            CreateSecretParams::new(&secret_name, "state-postgres-secret-sentinel"),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .consume_if_matches(&user_id, &secret_name, "wrong")
            .await
            .unwrap(),
        ironclaw_secrets::SecretConsumeResult::Mismatched
    );
    assert_eq!(
        store
            .consume_if_matches(&user_id, &secret_name, "state-postgres-secret-sentinel")
            .await
            .unwrap(),
        ironclaw_secrets::SecretConsumeResult::Matched
    );
    assert!(!store.exists(&user_id, &secret_name).await.unwrap());
}

async fn assert_secret_store_tracks_metadata_usage_and_overwrite_edges<S>(store: &S, user_id: &str)
where
    S: SecretsStore + ?Sized,
{
    let expires_at = Utc::now() + Duration::hours(1);
    let created = store
        .create(
            user_id,
            CreateSecretParams::new("Beta_Key", "sk-live-first-value")
                .with_provider("openai")
                .with_expiry(expires_at),
        )
        .await
        .unwrap();
    assert_eq!(created.name, "beta_key");
    assert_eq!(created.provider.as_deref(), Some("openai"));
    assert_eq!(created.usage_count, 0);
    assert!(created.last_used_at.is_none());

    let fetched = store.get(user_id, "BETA_KEY").await.unwrap();
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.name, "beta_key");
    assert_eq!(fetched.provider.as_deref(), Some("openai"));
    assert_eq!(fetched.expires_at, Some(expires_at));

    let refs = store.list(user_id).await.unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].name, "beta_key");
    assert_eq!(refs[0].provider.as_deref(), Some("openai"));

    store.record_usage(fetched.id).await.unwrap();
    let used = store.get(user_id, "beta_key").await.unwrap();
    assert_eq!(used.usage_count, 1);
    assert!(used.last_used_at.is_some());

    let overwritten = store
        .create(
            user_id,
            CreateSecretParams::new("BETA_KEY", "sk-live-second-value").with_provider("anthropic"),
        )
        .await
        .unwrap();
    assert_eq!(overwritten.id, created.id);
    assert_eq!(overwritten.name, "beta_key");
    assert_eq!(overwritten.created_at, created.created_at);
    assert_eq!(overwritten.usage_count, 1);
    assert!(overwritten.last_used_at.is_some());

    let after_overwrite = store.get(user_id, "beta_key").await.unwrap();
    assert_eq!(after_overwrite.id, created.id);
    assert_eq!(after_overwrite.created_at, created.created_at);
    assert_eq!(after_overwrite.usage_count, 1);
    assert!(after_overwrite.last_used_at.is_some());
    assert_eq!(after_overwrite.provider.as_deref(), Some("anthropic"));
    let decrypted = store.get_decrypted(user_id, "beta_key").await.unwrap();
    assert_eq!(decrypted.expose(), "sk-live-second-value");

    assert!(store.delete(user_id, "BETA_KEY").await.unwrap());
    assert!(!store.exists(user_id, "beta_key").await.unwrap());
    assert!(!store.delete(user_id, "beta_key").await.unwrap());
    assert!(matches!(
        store.record_usage(overwritten.id).await,
        Err(SecretError::NotFound(_))
    ));
}

async fn assert_secret_store_preserves_mismatched_and_expired_one_shot_values<S>(
    store: &S,
    user_id: &str,
    mismatch_name: &str,
    expired_name: &str,
) where
    S: SecretsStore + ?Sized,
{
    store
        .create(
            user_id,
            CreateSecretParams::new(mismatch_name, "state-secret-sentinel"),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .consume_if_matches(user_id, mismatch_name, "wrong-state")
            .await
            .unwrap(),
        ironclaw_secrets::SecretConsumeResult::Mismatched
    );
    assert!(store.exists(user_id, mismatch_name).await.unwrap());
    let still_decrypts = store.get_decrypted(user_id, mismatch_name).await.unwrap();
    assert_eq!(still_decrypts.expose(), "state-secret-sentinel");

    store
        .create(
            user_id,
            CreateSecretParams::new(expired_name, "expired-state-sentinel")
                .with_expiry(Utc::now() - Duration::seconds(1)),
        )
        .await
        .unwrap();
    assert!(matches!(
        store.get(user_id, expired_name).await,
        Err(SecretError::Expired)
    ));
    assert!(matches!(
        store.get_decrypted(user_id, expired_name).await,
        Err(SecretError::Expired)
    ));
    assert!(matches!(
        store
            .consume_if_matches(user_id, expired_name, "expired-state-sentinel")
            .await,
        Err(SecretError::Expired)
    ));
    assert!(
        store.exists(user_id, expired_name).await.unwrap(),
        "expired rows stay durable for cleanup/rotation rather than disappearing during reads"
    );
}

async fn assert_secret_store_enforces_access_patterns<S>(store: &S, user_id: &str, name: &str)
where
    S: SecretsStore + ?Sized,
{
    store
        .create(
            user_id,
            CreateSecretParams::new(name, "sk-live-access-sentinel"),
        )
        .await
        .unwrap();

    assert!(
        store
            .is_accessible(user_id, name, &[name.to_ascii_uppercase()])
            .await
            .unwrap()
    );
    assert!(
        store
            .is_accessible(user_id, name, &["openai_*".to_string()])
            .await
            .unwrap()
    );
    assert!(
        !store
            .is_accessible(user_id, name, &["github_*".to_string()])
            .await
            .unwrap()
    );
    assert!(
        !store
            .is_accessible(user_id, "missing_secret", &["missing_*".to_string()])
            .await
            .unwrap()
    );
}

#[cfg(feature = "libsql")]
async fn libsql_store(path: &std::path::Path, crypto: Arc<SecretsCrypto>) -> LibSqlSecretsStore {
    let db = Arc::new(libsql::Builder::new_local(path).build().await.unwrap());
    let store = LibSqlSecretsStore::new(db, crypto);
    store.run_migrations().await.unwrap();
    store
}

#[cfg(feature = "libsql")]
async fn insert_libsql_secret_store_key_check(
    conn: &libsql::Connection,
    crypto: Arc<SecretsCrypto>,
) {
    let (encrypted_value, key_salt) = crypto
        .encrypt(
            b"reborn-secret-store-key-check-v1",
            &secret_store_key_check_aad(),
        )
        .unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO reborn_secret_store_key_check (id, encrypted_value, key_salt, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        libsql::params!["active", encrypted_value, key_salt, now],
    )
    .await
    .unwrap();
}

#[cfg(feature = "libsql")]
async fn insert_libsql_secret_record(
    conn: &libsql::Connection,
    user_id: &str,
    name: &str,
    crypto: Arc<SecretsCrypto>,
    plaintext: &str,
) {
    let (encrypted_value, key_salt) = crypto
        .encrypt(plaintext.as_bytes(), &secret_record_aad(user_id, name))
        .unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO reborn_secret_records (user_id, name, id, encrypted_value, key_salt, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        libsql::params![user_id, name, uuid::Uuid::new_v4().to_string(), encrypted_value, key_salt, now],
    )
    .await
    .unwrap();
}

#[cfg(feature = "postgres")]
async fn postgres_store() -> Option<PostgresSecretsStore> {
    postgres_store_with_crypto(test_crypto()).await
}

#[cfg(feature = "postgres")]
async fn postgres_store_with_crypto(crypto: Arc<SecretsCrypto>) -> Option<PostgresSecretsStore> {
    let pool = postgres_pool().await?;
    let store = PostgresSecretsStore::new(pool, crypto);
    store
        .run_migrations()
        .await
        .expect("DATABASE_URL must point at a reachable Postgres test database");
    Some(store)
}

#[cfg(feature = "postgres")]
async fn postgres_pool() -> Option<deadpool_postgres::Pool> {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping Postgres secret store contract: DATABASE_URL is not set");
        return None;
    };
    let config: tokio_postgres::Config = database_url
        .parse()
        .expect("DATABASE_URL must parse as a Postgres connection string");
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    Some(
        deadpool_postgres::Pool::builder(manager)
            .max_size(4)
            .build()
            .expect("Postgres pool must build"),
    )
}

#[cfg(feature = "postgres")]
async fn insert_postgres_secret_store_key_check(
    client: &impl deadpool_postgres::GenericClient,
    crypto: Arc<SecretsCrypto>,
) {
    let (encrypted_value, key_salt) = crypto
        .encrypt(
            b"reborn-secret-store-key-check-v1",
            &secret_store_key_check_aad(),
        )
        .unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    client
        .execute(
            "INSERT INTO reborn_secret_store_key_check (id, encrypted_value, key_salt, created_at, updated_at) VALUES ($1, $2, $3, $4, $4)",
            &[&"active", &encrypted_value, &key_salt, &now],
        )
        .await
        .unwrap();
}

#[cfg(feature = "postgres")]
async fn insert_postgres_secret_record(
    client: &impl deadpool_postgres::GenericClient,
    user_id: &str,
    name: &str,
    crypto: Arc<SecretsCrypto>,
    plaintext: &str,
) {
    let (encrypted_value, key_salt) = crypto
        .encrypt(plaintext.as_bytes(), &secret_record_aad(user_id, name))
        .unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    client
        .execute(
            "INSERT INTO reborn_secret_records (user_id, name, id, encrypted_value, key_salt, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $6)",
            &[&user_id, &name, &uuid::Uuid::new_v4().to_string(), &encrypted_value, &key_salt, &now],
        )
        .await
        .unwrap();
}

fn test_crypto() -> Arc<SecretsCrypto> {
    Arc::new(
        SecretsCrypto::new(SecretMaterial::from(
            "0123456789abcdef0123456789abcdef".to_string(),
        ))
        .unwrap(),
    )
}

fn wrong_crypto() -> Arc<SecretsCrypto> {
    Arc::new(
        SecretsCrypto::new(SecretMaterial::from(
            "abcdef0123456789abcdef0123456789".to_string(),
        ))
        .unwrap(),
    )
}

#[cfg(feature = "postgres")]
fn unique_suffix() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
}

//! Reborn integration test — secret store durability over LibSql.
//!
//! Store-level durability proof: writes a secret to a `FilesystemSecretStore`
//! backed by a libSQL composite, then reopens a genuinely fresh store over the
//! same on-disk database file and reads the secret back. Proves real on-disk
//! secret durability without exercising the turn/model layer.
//!
//! Gated on the `libsql` feature because the test directly instantiates
//! `LibSqlRootFilesystem`, a type that compiles only under
//! `feature = "libsql"`.  Running without the feature produces 0 tests
//! (compile-safe); running with `--features libsql` produces 2 tests.

#![cfg(feature = "libsql")]

// The support tree is large and shared; a single-test file exercises only a
// slice of it, so suppress dead-code warnings on the includes (matches
// `reborn_integration_greeting.rs`).
#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use std::sync::Arc;

use ironclaw_filesystem::{CompositeRootFilesystem, LibSqlRootFilesystem};
use ironclaw_host_api::SecretHandle;
use ironclaw_reborn_composition::test_support::{
    LOCAL_DEV_DB_FILENAME, build_default_local_dev_database_roots_for_test,
    build_local_dev_secret_store_for_test, mount_local_dev_database_roots_for_test,
};
use ironclaw_reborn_composition::wrap_scoped;
use ironclaw_secrets::{SecretMaterial, SecretStore};
use secrecy::ExposeSecret;

use reborn_support::harness::test_product_scope;

/// Write a secret and verify it survives a libSQL connection close + reopen.
///
/// The reopen creates a genuinely fresh `libsql::Database` over the same
/// on-disk file — the original composite and all its connection state are
/// dropped before the second store is built — so this is a real durability
/// proof, not a re-instantiation-within-the-same-process-Arc test.
#[tokio::test]
async fn secret_persists_across_libsql_reopen() {
    let dir = tempfile::tempdir().expect("temp dir");

    // --- First store: write secret ---
    let mut composite = CompositeRootFilesystem::new();
    build_default_local_dev_database_roots_for_test(dir.path(), &mut composite)
        .await
        .expect("build default local-dev db roots");
    let composite = Arc::new(composite);
    let scoped = wrap_scoped(Arc::clone(&composite));
    let store = build_local_dev_secret_store_for_test(dir.path(), Arc::clone(&scoped))
        .expect("build first secret store");

    let scope = test_product_scope(
        "tenant-itest",
        "host-user",
        "agent-itest",
        Some("project-itest"),
    );
    let handle = SecretHandle::new("test-api-key").expect("valid secret handle");
    let material = SecretMaterial::from("sk-live-42".to_string());

    store
        .put(scope.clone(), handle.clone(), material, None)
        .await
        .expect("put secret to store");

    // Drop everything — the first store and its backing composite must be gone
    // before the fresh connection below can prove on-disk durability.
    drop(store);
    drop(scoped);
    drop(composite);

    // --- Reopen: fresh libsql database, fresh composite, fresh store ---
    //
    // Mirrors the `assert_reply_persists_after_reopen` pattern in
    // `tests/support/reborn/builder.rs`: `libsql::Builder::new_local` opens
    // the existing file, `run_migrations` is idempotent (schema already
    // exists), and the SAME `root` path yields the SAME cached master-key
    // file so decryption succeeds on the fresh store.
    let db_path = dir.path().join(LOCAL_DEV_DB_FILENAME);
    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("open fresh libsql for reopen"),
    );
    let fresh_fs = Arc::new(LibSqlRootFilesystem::new(db));
    // Migrations are idempotent — schema already exists from the first build.
    fresh_fs
        .run_migrations()
        .await
        .expect("run migrations on fresh libsql");
    let mut fresh_composite = CompositeRootFilesystem::new();
    mount_local_dev_database_roots_for_test(&mut fresh_composite, fresh_fs)
        .expect("mount fresh composite");
    let fresh_composite = Arc::new(fresh_composite);
    let fresh_scoped = wrap_scoped(Arc::clone(&fresh_composite));
    let fresh_store = build_local_dev_secret_store_for_test(dir.path(), fresh_scoped)
        .expect("build fresh secret store (same root → same crypto key)");

    // --- Read back: the material must survive the reopen ---
    let lease = fresh_store
        .lease_once(&scope, &handle)
        .await
        .expect("lease_once on fresh store after reopen");
    let read_material = fresh_store
        .consume(&scope, lease.id)
        .await
        .expect("consume lease on fresh store after reopen");

    assert_eq!(
        read_material.expose_secret(),
        "sk-live-42",
        "secret material must survive libsql reopen"
    );
}

/// Prove the read path is not vacuously succeeding: leasing an unknown handle
/// must return an error, not silently succeed with arbitrary data.
#[tokio::test]
async fn secret_read_back_fails_for_unknown_handle() {
    let dir = tempfile::tempdir().expect("temp dir");

    // Build the store and write one secret under a known handle.
    let mut composite = CompositeRootFilesystem::new();
    build_default_local_dev_database_roots_for_test(dir.path(), &mut composite)
        .await
        .expect("build default local-dev db roots");
    let composite = Arc::new(composite);
    let scoped = wrap_scoped(Arc::clone(&composite));
    let store =
        build_local_dev_secret_store_for_test(dir.path(), scoped).expect("build secret store");

    let scope = test_product_scope(
        "tenant-itest",
        "host-user",
        "agent-itest",
        Some("project-itest"),
    );
    let written_handle = SecretHandle::new("test-api-key").expect("valid secret handle");
    let material = SecretMaterial::from("sk-live-42".to_string());

    store
        .put(scope.clone(), written_handle, material, None)
        .await
        .expect("put secret");

    // Requesting a handle that was never written must fail.
    let unknown_handle = SecretHandle::new("nonexistent").expect("valid unknown handle");
    let result = store.lease_once(&scope, &unknown_handle).await;
    assert!(
        result.is_err(),
        "lease_once for an unknown handle must return Err, not vacuously succeed"
    );
}

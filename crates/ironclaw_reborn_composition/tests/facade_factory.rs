use std::sync::Arc;

use ironclaw_reborn_composition::{
    RebornBuildInput, RebornCompositionProfile, RebornReadinessState, build_reborn_services,
};
use ironclaw_secrets::SecretMaterial;

fn test_master_key() -> SecretMaterial {
    SecretMaterial::from("x".repeat(32))
}

#[tokio::test]
async fn disabled_returns_empty_services() {
    let services = build_reborn_services(RebornBuildInput::disabled("test-owner"))
        .await
        .unwrap();

    assert!(services.host_runtime.is_none());
    assert!(services.turn_coordinator.is_none());
    assert_eq!(services.readiness.state, RebornReadinessState::Disabled);
}

#[tokio::test]
async fn local_dev_builds_facades_without_production_claim() {
    let dir = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "test-owner",
        dir.path().to_path_buf(),
    ))
    .await
    .unwrap();

    assert!(services.host_runtime.is_some());
    assert!(services.turn_coordinator.is_some());
    assert_eq!(services.readiness.state, RebornReadinessState::DevOnly);
    assert!(services.readiness.facades.host_runtime);
    assert!(services.readiness.facades.turn_coordinator);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_rejects_memory_libsql_event_store() {
    let db = libsql::Builder::new_local(":memory:")
        .build()
        .await
        .unwrap();
    let db = Arc::new(db);

    let result = build_reborn_services(RebornBuildInput::libsql(
        RebornCompositionProfile::Production,
        "test-owner",
        db,
        ":memory:",
        None,
        test_master_key(),
    ))
    .await;

    let error = result.expect_err("production must reject in-memory event store");
    let rendered = error.to_string();
    assert!(!rendered.contains("postgres://"));
    assert!(!rendered.contains("token"));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn migration_dry_run_validates_libsql_shape() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("reborn.db");
    let db = libsql::Builder::new_local(db_path.clone())
        .build()
        .await
        .unwrap();
    let db = Arc::new(db);

    let services = build_reborn_services(RebornBuildInput::libsql(
        RebornCompositionProfile::MigrationDryRun,
        "test-owner",
        db,
        db_path.to_string_lossy(),
        None,
        test_master_key(),
    ))
    .await
    .unwrap();

    assert_eq!(
        services.readiness.state,
        RebornReadinessState::MigrationDryRunValidated
    );
    assert!(services.host_runtime.is_some());
    assert!(services.turn_coordinator.is_some());
}

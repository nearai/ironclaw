#[cfg(feature = "libsql")]
use std::sync::Arc;

#[cfg(feature = "libsql")]
use ironclaw_host_api::{EffectKind, PackageId};
#[cfg(feature = "libsql")]
use ironclaw_host_runtime::{
    SchedulerTurnRunWakeNotifier, TurnRunExecutor, TurnRunExecutorError, TurnRunScheduler,
    TurnRunSchedulerConfig, TurnRunSchedulerHandle,
};
#[cfg(feature = "libsql")]
use ironclaw_reborn_composition::{RebornBuildError, RebornCompositionProfile};
use ironclaw_reborn_composition::{RebornBuildInput, RebornReadinessState, build_reborn_services};
#[cfg(feature = "libsql")]
use ironclaw_secrets::SecretMaterial;
#[cfg(feature = "libsql")]
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
#[cfg(feature = "libsql")]
use ironclaw_turns::{
    InMemoryTurnStateStore,
    runner::{ClaimedTurnRun, TurnRunTransitionPort},
};

#[cfg(feature = "libsql")]
fn test_master_key() -> SecretMaterial {
    SecretMaterial::from("x".repeat(32))
}

#[cfg(feature = "libsql")]
struct NoopTurnRunExecutor;

#[cfg(feature = "libsql")]
#[async_trait::async_trait]
impl TurnRunExecutor for NoopTurnRunExecutor {
    async fn execute_claimed_run(
        &self,
        _claimed: ClaimedTurnRun,
        _transitions: Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), TurnRunExecutorError> {
        Ok(())
    }
}

#[cfg(feature = "libsql")]
fn production_trust_policy() -> Arc<HostTrustPolicy> {
    Arc::new(
        HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries([
            AdminEntry::for_admin(
                PackageId::new("reborn-test").unwrap(),
                HostTrustAssignment::first_party(),
                vec![EffectKind::DispatchCapability],
                None,
            ),
        ]))])
        .unwrap(),
    )
}

#[cfg(feature = "libsql")]
fn empty_trust_policy() -> Arc<HostTrustPolicy> {
    Arc::new(HostTrustPolicy::empty())
}

#[cfg(feature = "libsql")]
fn live_wake_notifier() -> (Arc<SchedulerTurnRunWakeNotifier>, TurnRunSchedulerHandle) {
    let transitions: Arc<dyn TurnRunTransitionPort> = Arc::new(InMemoryTurnStateStore::default());
    let executor: Arc<dyn TurnRunExecutor> = Arc::new(NoopTurnRunExecutor);
    let handle =
        TurnRunScheduler::new(transitions, executor, TurnRunSchedulerConfig::default()).start();
    (handle.wake_notifier(), handle)
}

#[cfg(feature = "libsql")]
async fn libsql_db_at(path: impl AsRef<std::path::Path>) -> Arc<libsql::Database> {
    Arc::new(
        libsql::Builder::new_local(path.as_ref())
            .build()
            .await
            .unwrap(),
    )
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
async fn production_requires_configured_trust_policy() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;

    let result = build_reborn_services(RebornBuildInput::libsql(
        RebornCompositionProfile::Production,
        "test-owner",
        db,
        dir.path().join("events.db").to_string_lossy(),
        None,
        test_master_key(),
    ))
    .await;

    assert!(matches!(
        result,
        Err(RebornBuildError::MissingProductionTrustPolicy)
    ));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_rejects_empty_trust_policy() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(empty_trust_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await;

    handle.shutdown().await;

    assert!(matches!(
        result,
        Err(RebornBuildError::EmptyProductionTrustPolicy)
    ));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_requires_live_turn_wake_notifier() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy()),
    )
    .await;

    assert!(matches!(
        result,
        Err(RebornBuildError::MissingTurnRunWakeNotifier)
    ));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_rejects_memory_libsql_event_store() {
    let db = Arc::new(
        libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap(),
    );
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            ":memory:",
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await;

    handle.shutdown().await;

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
    let db = libsql_db_at(&db_path).await;
    let (notifier, handle) = live_wake_notifier();

    let services = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::MigrationDryRun,
            "test-owner",
            db,
            db_path.to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await
    .unwrap();

    handle.shutdown().await;

    assert_eq!(
        services.readiness.state,
        RebornReadinessState::MigrationDryRunValidated
    );
    assert!(services.host_runtime.is_some());
    assert!(services.turn_coordinator.is_some());
}

#![cfg(any(feature = "libsql", feature = "postgres"))]

#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::sync::Arc;
#[cfg(feature = "postgres")]
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Duration, Utc};
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_host_api::VirtualPath;
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalEventId, ProductAdapterId, ProductInboundAck,
};
use ironclaw_product_workflow::{
    ActionFingerprintKey, DeleteScopedLifecycleInstallationRequest, IdempotencyDecision,
    IdempotencyLedger, LifecyclePackageKind, LifecyclePackageRef, ProductInboundAction,
    ProductWorkflowError, ScopedLifecycleActor, ScopedLifecycleInstallation,
    ScopedLifecycleInstallationId, ScopedLifecycleInstallationStore, ScopedLifecycleSubject,
    SourceBindingKey, UpsertScopedLifecycleInstallationRequest,
};
#[cfg(feature = "libsql")]
use ironclaw_product_workflow_storage::{
    RebornLibSqlIdempotencyLedger, RebornLibSqlScopedLifecycleInstallationStore,
};
#[cfg(feature = "postgres")]
use ironclaw_product_workflow_storage::{
    RebornPostgresIdempotencyLedger, RebornPostgresScopedLifecycleInstallationStore,
};

fn fingerprint(suffix: &str) -> ActionFingerprintKey {
    fingerprint_for_actor(suffix, "user1")
}

fn fingerprint_for_actor(suffix: &str, actor_id: &str) -> ActionFingerprintKey {
    ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid adapter"),
        AdapterInstallationId::new("install_alpha").expect("valid installation"),
        ExternalActorRef::new("test", actor_id, Option::<String>::None).expect("valid actor"),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new(format!("evt:{suffix}")).expect("valid event"),
    )
}

fn custom_root(suffix: &str) -> VirtualPath {
    VirtualPath::new(format!(
        "/engine/product_workflow/idempotency/test_roots/{suffix}"
    ))
    .expect("valid custom ledger root")
}

fn scoped_lifecycle_root(suffix: &str) -> VirtualPath {
    VirtualPath::new(format!(
        "/engine/product_workflow/scoped_lifecycle/test_roots/{suffix}"
    ))
    .expect("valid scoped lifecycle root")
}

#[cfg(feature = "postgres")]
fn unique_suffix(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after unix epoch")
        .as_nanos();
    format!("{name}-{nanos}")
}

#[cfg(feature = "libsql")]
async fn libsql_filesystem(path: &str) -> Arc<LibSqlRootFilesystem> {
    let db = Arc::new(
        libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build libsql db"),
    );
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem
        .run_migrations()
        .await
        .expect("run libsql filesystem migrations");
    filesystem
}

async fn assert_settled_action_survives_reopen_and_replays(
    ledger: &dyn IdempotencyLedger,
    reopened: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let decision = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin");
    let IdempotencyDecision::New(mut action) = decision else {
        panic!("expected new action");
    };
    action.settle(ProductInboundAck::NoOp);
    ledger.settle(action).await.expect("settle");

    let replay = reopened
        .begin_or_replay(fingerprint, received_at + Duration::seconds(1))
        .await
        .expect("replay");

    let IdempotencyDecision::Replay(action) = replay else {
        panic!("expected replay");
    };
    assert_eq!(action.outcome, Some(ProductInboundAck::NoOp));
}

async fn assert_in_flight_action_blocks_until_lease_expires(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    assert!(matches!(
        ledger
            .begin_or_replay(fingerprint.clone(), received_at)
            .await
            .expect("begin"),
        IdempotencyDecision::New(_)
    ));
    let blocked = ledger
        .begin_or_replay(fingerprint.clone(), received_at + Duration::seconds(5))
        .await
        .expect_err("fresh reservation should block");
    assert!(matches!(blocked, ProductWorkflowError::Transient { .. }));

    let reclaimed = ledger
        .begin_or_replay(fingerprint, received_at + Duration::seconds(11))
        .await
        .expect("expired reservation should be reclaimed");
    assert!(matches!(reclaimed, IdempotencyDecision::New(_)));
}

async fn assert_release_allows_retry_without_waiting_for_lease(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let decision = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin");
    let IdempotencyDecision::New(action) = decision else {
        panic!("expected new action");
    };
    ledger.release(action).await.expect("release");

    let retry = ledger
        .begin_or_replay(fingerprint, received_at + Duration::seconds(1))
        .await
        .expect("retry after release");
    assert!(matches!(retry, IdempotencyDecision::New(_)));
}

async fn assert_duplicate_reservation_contention_serializes(
    first: &dyn IdempotencyLedger,
    second: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let (left, right) = tokio::join!(
        first.begin_or_replay(fingerprint.clone(), received_at),
        second.begin_or_replay(fingerprint, received_at),
    );
    let results = [left, right];
    let new_count = results
        .iter()
        .filter(|result| matches!(result, Ok(IdempotencyDecision::New(_))))
        .count();
    let blocked_count = results
        .iter()
        .filter(|result| matches!(result, Err(ProductWorkflowError::Transient { .. })))
        .count();

    assert_eq!(new_count, 1);
    assert_eq!(blocked_count, 1);
}

async fn assert_superseded_reservation_cannot_settle(ledger: &dyn IdempotencyLedger, suffix: &str) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let IdempotencyDecision::New(mut stale_action) = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin")
    else {
        panic!("expected new action");
    };

    let IdempotencyDecision::New(mut replacement) = ledger
        .begin_or_replay(fingerprint, received_at + Duration::seconds(11))
        .await
        .expect("expired reservation should be reclaimed")
    else {
        panic!("expected reclaimed action");
    };

    stale_action.settle(ProductInboundAck::NoOp);
    let stale_error = ledger
        .settle(stale_action)
        .await
        .expect_err("superseded action must not settle");
    assert!(matches!(
        stale_error,
        ProductWorkflowError::Transient { .. }
    ));

    replacement.settle(ProductInboundAck::NoOp);
    ledger
        .settle(replacement)
        .await
        .expect("replacement settle");
}

async fn assert_settle_missing_reservation_returns_transient(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let mut action = ProductInboundAction::begin(fingerprint(suffix), received_at);
    action.settle(ProductInboundAck::NoOp);

    let error = ledger
        .settle(action)
        .await
        .expect_err("missing reservation must not settle");
    assert!(matches!(error, ProductWorkflowError::Transient { .. }));
}

async fn assert_custom_root_isolated_from_default_root(
    custom: &dyn IdempotencyLedger,
    default: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);
    let IdempotencyDecision::New(mut action) = custom
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin in custom root")
    else {
        panic!("expected new custom-root action");
    };
    action.settle(ProductInboundAck::NoOp);
    custom.settle(action).await.expect("settle custom root");

    let default_decision = default
        .begin_or_replay(fingerprint, received_at + Duration::seconds(1))
        .await
        .expect("begin in default root");
    assert!(matches!(default_decision, IdempotencyDecision::New(_)));
}

async fn assert_actor_identity_is_part_of_fingerprint_path(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let first_actor = fingerprint_for_actor(suffix, "user1");
    let second_actor = fingerprint_for_actor(suffix, "user2");

    assert!(matches!(
        ledger
            .begin_or_replay(first_actor, received_at)
            .await
            .expect("begin first actor"),
        IdempotencyDecision::New(_)
    ));
    assert!(matches!(
        ledger
            .begin_or_replay(second_actor, received_at)
            .await
            .expect("begin second actor"),
        IdempotencyDecision::New(_)
    ));
}

async fn assert_scoped_lifecycle_store_resolves_shared_and_private_after_reopen(
    store: &dyn ScopedLifecycleInstallationStore,
    reopened: &dyn ScopedLifecycleInstallationStore,
    suffix: &str,
) {
    let tenant =
        ironclaw_host_api::TenantId::new(format!("tenant-{suffix}")).expect("valid tenant");
    let admin = ScopedLifecycleActor::admin(
        tenant.clone(),
        ironclaw_host_api::UserId::new("admin-alpha").expect("valid admin"),
    );
    let user = ScopedLifecycleActor::user(
        tenant.clone(),
        ironclaw_host_api::UserId::new("user-alpha").expect("valid user"),
    );
    let other_user = ScopedLifecycleActor::user(
        tenant.clone(),
        ironclaw_host_api::UserId::new("user-beta").expect("valid user"),
    );
    let other_admin = ScopedLifecycleActor::admin(
        tenant.clone(),
        ironclaw_host_api::UserId::new("admin-beta").expect("valid admin"),
    );

    let now = Utc::now();
    let shared_github = ScopedLifecycleInstallation::admin_shared(
        scoped_install_id(suffix, "shared-github"),
        package_ref("github"),
        admin.clone(),
        now,
    )
    .expect("admin shared install");
    let private_github = ScopedLifecycleInstallation::user_private(
        scoped_install_id(suffix, "private-github"),
        package_ref("github"),
        user.clone(),
        now,
    );
    let private_notion = ScopedLifecycleInstallation::user_private(
        scoped_install_id(suffix, "private-notion"),
        package_ref("notion"),
        user.clone(),
        now,
    );

    store
        .upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: admin.clone(),
            installation: shared_github.clone(),
        })
        .await
        .expect("upsert shared");
    store
        .upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: user.clone(),
            installation: private_github.clone(),
        })
        .await
        .expect("upsert private override");
    store
        .upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: user.clone(),
            installation: private_notion.clone(),
        })
        .await
        .expect("upsert private notion");

    let duplicate_shared_github = ScopedLifecycleInstallation::admin_shared(
        scoped_install_id(suffix, "shared-github-duplicate"),
        package_ref("github"),
        admin.clone(),
        now,
    )
    .expect("duplicate admin shared install");
    let duplicate_shared_error = store
        .upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: admin.clone(),
            installation: duplicate_shared_github,
        })
        .await
        .expect_err("store rejects duplicate admin shared package");
    assert!(matches!(
        duplicate_shared_error,
        ProductWorkflowError::InvalidBindingRequest { .. }
    ));

    let duplicate_private_github = ScopedLifecycleInstallation::user_private(
        scoped_install_id(suffix, "private-github-duplicate"),
        package_ref("github"),
        user.clone(),
        now,
    );
    let duplicate_private_error = store
        .upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: user.clone(),
            installation: duplicate_private_github,
        })
        .await
        .expect_err("store rejects duplicate user private package");
    assert!(matches!(
        duplicate_private_error,
        ProductWorkflowError::InvalidBindingRequest { .. }
    ));

    let duplicate_installation_id = scoped_install_id(suffix, "private-duplicate-installation-id");
    let duplicate_id_left = ScopedLifecycleInstallation::user_private(
        duplicate_installation_id.clone(),
        package_ref("mail"),
        other_user.clone(),
        now,
    );
    let duplicate_id_right = ScopedLifecycleInstallation::user_private(
        duplicate_installation_id.clone(),
        package_ref("drive"),
        other_user.clone(),
        now,
    );
    let (duplicate_id_left_result, duplicate_id_right_result) = tokio::join!(
        store.upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: other_user.clone(),
            installation: duplicate_id_left,
        }),
        reopened.upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: other_user.clone(),
            installation: duplicate_id_right,
        }),
    );
    let duplicate_id_results = [&duplicate_id_left_result, &duplicate_id_right_result];
    assert_eq!(
        duplicate_id_results
            .iter()
            .filter(|result| result.is_ok())
            .count(),
        1
    );
    assert_eq!(
        duplicate_id_results
            .iter()
            .filter(|result| matches!(
                result,
                Err(ProductWorkflowError::InvalidBindingRequest { .. })
            ))
            .count(),
        1
    );
    store
        .delete_installation(DeleteScopedLifecycleInstallationRequest {
            actor: other_user.clone(),
            tenant_id: tenant.clone(),
            installation_id: duplicate_installation_id,
        })
        .await
        .expect("delete duplicate installation id winner");

    let concurrent_left_id = scoped_install_id(suffix, "private-calendar-left");
    let concurrent_right_id = scoped_install_id(suffix, "private-calendar-right");
    let concurrent_left = ScopedLifecycleInstallation::user_private(
        concurrent_left_id.clone(),
        package_ref("calendar"),
        other_user.clone(),
        now,
    );
    let concurrent_right = ScopedLifecycleInstallation::user_private(
        concurrent_right_id.clone(),
        package_ref("calendar"),
        other_user.clone(),
        now,
    );
    let (left_result, right_result) = tokio::join!(
        store.upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: other_user.clone(),
            installation: concurrent_left,
        }),
        reopened.upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: other_user.clone(),
            installation: concurrent_right,
        }),
    );
    let concurrent_results = [&left_result, &right_result];
    assert_eq!(
        concurrent_results
            .iter()
            .filter(|result| result.is_ok())
            .count(),
        1
    );
    assert_eq!(
        concurrent_results
            .iter()
            .filter(|result| matches!(
                result,
                Err(ProductWorkflowError::InvalidBindingRequest { .. })
            ))
            .count(),
        1
    );
    let concurrent_winner_id = if left_result.is_ok() {
        concurrent_left_id
    } else {
        concurrent_right_id
    };
    let deleted_calendar_id = concurrent_winner_id.clone();
    store
        .delete_installation(DeleteScopedLifecycleInstallationRequest {
            actor: other_user.clone(),
            tenant_id: tenant.clone(),
            installation_id: concurrent_winner_id,
        })
        .await
        .expect("delete concurrent package winner");
    let replacement_calendar_id = scoped_install_id(suffix, "private-calendar-replacement");
    let replacement_calendar = ScopedLifecycleInstallation::user_private(
        replacement_calendar_id.clone(),
        package_ref("calendar"),
        other_user.clone(),
        now,
    );
    store
        .upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: other_user.clone(),
            installation: replacement_calendar,
        })
        .await
        .expect("recreate package after tombstone delete");
    assert!(
        reopened
            .get_installation(&tenant, &deleted_calendar_id)
            .await
            .expect("load deleted installation id after package replacement")
            .is_none()
    );
    store
        .delete_installation(DeleteScopedLifecycleInstallationRequest {
            actor: other_user.clone(),
            tenant_id: tenant.clone(),
            installation_id: replacement_calendar_id,
        })
        .await
        .expect("delete replacement package");

    let overwrite_as_user = store
        .upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: user.clone(),
            installation: shared_github.clone(),
        })
        .await
        .expect_err("user cannot overwrite admin shared installation");
    assert_eq!(overwrite_as_user, ProductWorkflowError::BindingAccessDenied);

    let delete_as_user = store
        .delete_installation(DeleteScopedLifecycleInstallationRequest {
            actor: user.clone(),
            tenant_id: tenant.clone(),
            installation_id: shared_github.installation_id.clone(),
        })
        .await
        .expect_err("user cannot delete admin shared installation");
    assert_eq!(delete_as_user, ProductWorkflowError::BindingAccessDenied);

    let mut misattributed_update = shared_github.clone();
    misattributed_update.updated_by = other_admin;
    let audit_mismatch = store
        .upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: admin.clone(),
            installation: misattributed_update,
        })
        .await
        .expect_err("store rejects mismatched update actor");
    assert!(matches!(
        audit_mismatch,
        ProductWorkflowError::InvalidBindingRequest { .. }
    ));

    let mut changed_package = shared_github.clone();
    changed_package.package_ref = package_ref("notion");
    changed_package.updated_by = admin.clone();
    let identity_change = store
        .upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: admin,
            installation: changed_package,
        })
        .await
        .expect_err("store rejects immutable update identity change");
    assert!(matches!(
        identity_change,
        ProductWorkflowError::InvalidBindingRequest { .. }
    ));

    let reopened_effective = reopened
        .list_effective_installations(ScopedLifecycleSubject::new(
            tenant.clone(),
            user.user_id.clone(),
        ))
        .await
        .expect("effective after reopen");
    assert_eq!(
        ids(&reopened_effective.installations),
        vec![
            private_github.installation_id.as_str().to_string(),
            private_notion.installation_id.as_str().to_string(),
        ]
    );

    let other_effective = reopened
        .list_effective_installations(ScopedLifecycleSubject::new(
            tenant,
            other_user.user_id.clone(),
        ))
        .await
        .expect("other user effective");
    assert_eq!(
        ids(&other_effective.installations),
        vec![shared_github.installation_id.as_str().to_string()]
    );
}

fn scoped_install_id(suffix: &str, label: &str) -> ScopedLifecycleInstallationId {
    ScopedLifecycleInstallationId::new(format!("{label}-{suffix}")).expect("valid install id")
}

fn package_ref(id: &str) -> LifecyclePackageRef {
    LifecyclePackageRef::new(LifecyclePackageKind::Extension, id).expect("valid package")
}

fn ids(installations: &[ScopedLifecycleInstallation]) -> Vec<String> {
    installations
        .iter()
        .map(|installation| installation.installation_id.as_str().to_string())
        .collect()
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_settled_action_survives_reopen_and_replays() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let db_path = db_path.display().to_string();
    let ledger = RebornLibSqlIdempotencyLedger::new(libsql_filesystem(&db_path).await);
    let reopened = RebornLibSqlIdempotencyLedger::new(libsql_filesystem(&db_path).await);

    assert_settled_action_survives_reopen_and_replays(&ledger, &reopened, "libsql-settled-replay")
        .await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_in_flight_action_blocks_until_lease_expires() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(10),
    );
    assert_in_flight_action_blocks_until_lease_expires(&ledger, "libsql-lease").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_release_allows_retry_without_waiting_for_lease() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(60),
    );
    assert_release_allows_retry_without_waiting_for_lease(&ledger, "libsql-release").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_duplicate_reservation_contention_serializes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let db_path = db_path.display().to_string();
    let first = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path).await,
        Duration::seconds(10),
    );
    let second = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path).await,
        Duration::seconds(10),
    );

    assert_duplicate_reservation_contention_serializes(&first, &second, "libsql-contention").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_superseded_reservation_cannot_settle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(10),
    );

    assert_superseded_reservation_cannot_settle(&ledger, "libsql-superseded").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_settle_missing_reservation_returns_transient() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger =
        RebornLibSqlIdempotencyLedger::new(libsql_filesystem(&db_path.display().to_string()).await);

    assert_settle_missing_reservation_returns_transient(&ledger, "libsql-missing-settle").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_custom_root_isolated_from_default_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let filesystem = libsql_filesystem(&db_path.display().to_string()).await;
    let custom = RebornLibSqlIdempotencyLedger::with_root(
        Arc::clone(&filesystem),
        custom_root("libsql"),
        Duration::seconds(60),
    );
    let default = RebornLibSqlIdempotencyLedger::new(filesystem);

    assert_custom_root_isolated_from_default_root(&custom, &default, "libsql-custom-root").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_actor_identity_is_part_of_fingerprint_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let db_path = db_path.display().to_string();
    let ledger = RebornLibSqlIdempotencyLedger::new(libsql_filesystem(&db_path).await);

    assert_actor_identity_is_part_of_fingerprint_path(&ledger, "libsql-actor-isolation").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_scoped_lifecycle_store_resolves_shared_and_private_after_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let db_path = db_path.display().to_string();
    let root = scoped_lifecycle_root("libsql-scoped");
    let store = RebornLibSqlScopedLifecycleInstallationStore::with_root(
        libsql_filesystem(&db_path).await,
        root.clone(),
    );
    let reopened = RebornLibSqlScopedLifecycleInstallationStore::with_root(
        libsql_filesystem(&db_path).await,
        root,
    );

    assert_scoped_lifecycle_store_resolves_shared_and_private_after_reopen(
        &store,
        &reopened,
        "libsql-scoped",
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_settled_action_survives_reopen_and_replays_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger = RebornPostgresIdempotencyLedger::new(Arc::clone(&filesystem));
    let reopened = RebornPostgresIdempotencyLedger::new(filesystem);

    assert_settled_action_survives_reopen_and_replays(
        &ledger,
        &reopened,
        &unique_suffix("postgres-settled-replay"),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_in_flight_action_blocks_until_lease_expires_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger =
        RebornPostgresIdempotencyLedger::with_in_flight_lease(filesystem, Duration::seconds(10));

    assert_in_flight_action_blocks_until_lease_expires(&ledger, &unique_suffix("postgres-lease"))
        .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_release_allows_retry_without_waiting_for_lease_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger =
        RebornPostgresIdempotencyLedger::with_in_flight_lease(filesystem, Duration::seconds(60));

    assert_release_allows_retry_without_waiting_for_lease(
        &ledger,
        &unique_suffix("postgres-release"),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_duplicate_reservation_contention_serializes_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let first = RebornPostgresIdempotencyLedger::with_in_flight_lease(
        Arc::clone(&filesystem),
        Duration::seconds(10),
    );
    let second =
        RebornPostgresIdempotencyLedger::with_in_flight_lease(filesystem, Duration::seconds(10));

    assert_duplicate_reservation_contention_serializes(
        &first,
        &second,
        &unique_suffix("postgres-contention"),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_superseded_reservation_cannot_settle_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger =
        RebornPostgresIdempotencyLedger::with_in_flight_lease(filesystem, Duration::seconds(10));

    assert_superseded_reservation_cannot_settle(&ledger, &unique_suffix("postgres-superseded"))
        .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_settle_missing_reservation_returns_transient_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger = RebornPostgresIdempotencyLedger::new(filesystem);

    assert_settle_missing_reservation_returns_transient(
        &ledger,
        &unique_suffix("postgres-missing-settle"),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_custom_root_isolated_from_default_root_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let custom = RebornPostgresIdempotencyLedger::with_root(
        Arc::clone(&filesystem),
        custom_root("postgres"),
        Duration::seconds(60),
    );
    let default = RebornPostgresIdempotencyLedger::new(filesystem);

    assert_custom_root_isolated_from_default_root(
        &custom,
        &default,
        &unique_suffix("postgres-custom-root"),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_actor_identity_is_part_of_fingerprint_path_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger = RebornPostgresIdempotencyLedger::new(filesystem);

    assert_actor_identity_is_part_of_fingerprint_path(
        &ledger,
        &unique_suffix("postgres-actor-isolation"),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_scoped_lifecycle_store_resolves_shared_and_private_after_reopen_when_configured()
{
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let suffix = unique_suffix("postgres-scoped");
    let root = scoped_lifecycle_root(&suffix);
    let store = RebornPostgresScopedLifecycleInstallationStore::with_root(
        Arc::clone(&filesystem),
        root.clone(),
    );
    let reopened = RebornPostgresScopedLifecycleInstallationStore::with_root(filesystem, root);

    assert_scoped_lifecycle_store_resolves_shared_and_private_after_reopen(
        &store, &reopened, &suffix,
    )
    .await;
}

#[cfg(feature = "postgres")]
async fn postgres_filesystem() -> Option<Arc<PostgresRootFilesystem>> {
    let url = match std::env::var("IRONCLAW_PRODUCT_WORKFLOW_POSTGRES_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping postgres product workflow ledger contract: IRONCLAW_PRODUCT_WORKFLOW_POSTGRES_URL not set"
            );
            return None;
        }
    };
    let config = match url.parse::<tokio_postgres::Config>() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("skipping postgres product workflow ledger contract: invalid url ({error})");
            return None;
        }
    };
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("postgres pool builds");
    if let Err(error) = pool.get().await {
        eprintln!(
            "skipping postgres product workflow ledger contract: database unavailable ({error})"
        );
        return None;
    }
    let filesystem = Arc::new(PostgresRootFilesystem::new(pool));
    if let Err(error) = filesystem.run_migrations().await {
        eprintln!(
            "skipping postgres product workflow ledger contract: filesystem migrations failed ({error})"
        );
        return None;
    }
    Some(filesystem)
}

use std::{sync::Arc, time::Duration};

use ironclaw_filesystem::{LibSqlRootFilesystem, RootFilesystem, ScopedFilesystem, SeqNo};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, TenantId, VirtualPath,
};
use ironclaw_resources::{
    FilesystemResourceGovernor, ResourceAccount, ResourceError, ResourceGovernor, ResourceLimits,
};
use rust_decimal_macros::dec;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_libsql_contention_fails_current_write_then_same_governor_recovers() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = Arc::new(
        libsql::Builder::new_local(directory.path().join("resource-governor.db"))
            .build()
            .await
            .expect("local libSQL database"),
    );
    let filesystem = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&database)));
    filesystem
        .run_migrations()
        .await
        .expect("filesystem migrations");
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").expect("resource alias"),
        VirtualPath::new("/resources").expect("resource target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("resource mount view");
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&filesystem),
        mounts,
    ));
    let governor = Arc::new(FilesystemResourceGovernor::new(scoped));
    let account = ResourceAccount::tenant(TenantId::new("tenant1").expect("tenant id"));

    let writer = database.connect().expect("writer connection");
    writer
        .execute("BEGIN IMMEDIATE", ())
        .await
        .expect("hold SQLite writer lock");

    let blocked_governor = Arc::clone(&governor);
    let blocked_account = account.clone();
    let blocked = tokio::task::spawn_blocking(move || {
        blocked_governor.set_limit(
            blocked_account,
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
    });
    let first_result = tokio::time::timeout(Duration::from_secs(30), blocked).await;
    writer
        .execute("ROLLBACK", ())
        .await
        .expect("release SQLite writer lock");
    let first_error = first_result
        .expect("bounded contention retries must complete")
        .expect("governor task must not panic")
        .expect_err("the request holding only optimistic state must fail");
    assert!(matches!(first_error, ResourceError::Storage { .. }));

    let recovered = governor
        .account_snapshot(&account)
        .expect("same governor should reload durable state after contention");
    assert!(
        recovered.is_none(),
        "the failed optimistic limit must not survive authority reload"
    );

    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(2.00)),
                ..ResourceLimits::default()
            },
        )
        .expect("same governor should accept a durable write after lock release");
    let recovered = governor
        .account_snapshot(&account)
        .expect("recovered account snapshot")
        .expect("durable limit account");
    assert_eq!(recovered.limits.expect("limits").max_usd, Some(dec!(2.00)));

    let journal = filesystem
        .tail(
            &VirtualPath::new("/resources/deltas/log").expect("journal path"),
            SeqNo::from_backend(0),
        )
        .await
        .expect("journal tail");
    assert_eq!(
        journal.len(),
        1,
        "failed contention retries must not duplicate journal deltas"
    );
}

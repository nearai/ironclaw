#![cfg(feature = "postgres")]

use std::sync::Arc;

use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, PostgresRootFilesystem, RootFilesystem,
};
use ironclaw_host_api::VirtualPath;

async fn postgres_pool(application_name: Option<&str>) -> Option<deadpool_postgres::Pool> {
    if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
        return None;
    }
    let url = std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let mut config = url.parse::<tokio_postgres::Config>().ok()?;
    if let Some(application_name) = application_name {
        config.application_name(application_name);
    }
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .ok()
}

async fn postgres_root(application_name: &str) -> Option<(PostgresRootFilesystem, String)> {
    let pool = postgres_pool(Some(application_name)).await?;
    let fs = PostgresRootFilesystem::new(pool);
    fs.run_migrations().await.ok()?;
    let prefix = format!("/secrets/leases/pgtest_{}", uuid::Uuid::new_v4().simple());
    Some((fs, prefix))
}

async fn wait_for_delete_if_version_lock_wait(
    pool: &deadpool_postgres::Pool,
    application_name: &str,
) {
    let client = pool
        .get()
        .await
        .expect("observer connection must be available on the same reachable Postgres");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        // Match the leading CTE rather than the later `FOR UPDATE` clause:
        // PostgreSQL truncates `pg_stat_activity.query` to
        // `track_activity_query_size`, and this query can exceed that limit
        // before the lock clause.
        let waiting: bool = client
            .query_one(
                "SELECT EXISTS (
                    SELECT 1
                    FROM pg_stat_activity
                    WHERE application_name = $1
                      AND state = 'active'
                      AND wait_event_type = 'Lock'
                      AND wait_event = 'transactionid'
                      AND query LIKE '%WITH candidate AS MATERIALIZED%'
                )",
                &[&application_name],
            )
            .await
            .expect("observe delete_if_version lock wait")
            .get(0);
        if waiting {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "delete_if_version did not reach its row-lock wait before the test deadline"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Drives the real delete-and-recreate race in its own test binary so the
/// intentionally open transaction cannot deadlock behind unrelated DDL from
/// the broader filesystem contract suite.
#[tokio::test]
async fn postgres_delete_if_version_stays_notfound_under_concurrent_delete_recreate_race() {
    let application_name = format!("ironclaw_cas_delete_{}", uuid::Uuid::new_v4().simple());
    let Some((fs, prefix)) = postgres_root(&application_name).await else {
        return;
    };
    let fs = Arc::new(fs);
    let Some(racer_pool) = postgres_pool(None).await else {
        return;
    };
    let Some(observer_pool) = postgres_pool(None).await else {
        return;
    };
    let path = VirtualPath::new(format!("{prefix}/cas_delete_race")).unwrap();
    let v1 = fs
        .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
        .await
        .unwrap();

    let racer_client = racer_pool
        .get()
        .await
        .expect("racer connection must be available on the same reachable Postgres");
    let path_str = path.as_str().to_string();
    let (deleted_tx, deleted_rx) = tokio::sync::oneshot::channel::<()>();
    let (recreate_tx, recreate_rx) = tokio::sync::oneshot::channel::<()>();

    let racer = tokio::spawn(async move {
        racer_client.batch_execute("BEGIN").await.unwrap();
        racer_client
            .execute(
                "DELETE FROM root_filesystem_entries WHERE path = $1 AND is_dir = FALSE",
                &[&path_str],
            )
            .await
            .unwrap();
        // The uncommitted DELETE holds the row lock that the CAS statement
        // must reach before the replacement is allowed to commit.
        let _ = deleted_tx.send(());
        if recreate_rx.await.is_err() {
            let _ = racer_client.batch_execute("ROLLBACK").await;
            return;
        }
        racer_client
            .execute(
                // Normal `CasExpectation::Absent` recreation restarts the
                // per-incarnation version at 1. The CAS delete must compare
                // incarnation identity before deleting, or it will remove
                // this replacement when its expected version is also 1.
                "INSERT INTO root_filesystem_entries (path, version) VALUES ($1, 1)",
                &[&path_str],
            )
            .await
            .unwrap();
        racer_client.batch_execute("COMMIT").await.unwrap();
    });

    deleted_rx
        .await
        .expect("racer must signal after its DELETE runs");
    let delete_path = path.clone();
    let delete_fs = Arc::clone(&fs);
    let delete_task =
        tokio::spawn(async move { delete_fs.delete_if_version(&delete_path, v1).await });
    wait_for_delete_if_version_lock_wait(&observer_pool, &application_name).await;
    recreate_tx
        .send(())
        .expect("racer task must still be waiting to recreate");
    let result = delete_task
        .await
        .expect("delete_if_version task must not panic");

    racer.await.expect("racer task must not panic");

    assert!(
        matches!(result, Err(FilesystemError::NotFound { .. })),
        "expected NotFound after the expected row was deleted and replaced, got: {result:?}"
    );
    let replacement = fs
        .get(&path)
        .await
        .expect("replacement lookup must succeed")
        .expect("replacement incarnation must not be deleted");
    assert_eq!(replacement.version, v1);
}

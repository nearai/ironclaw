//! Postgres coverage for the frozen v1 reader — the one backend the crate's
//! `tests/migration_roundtrip.rs` acceptance suite doesn't reach. Drives the
//! same production entry points the libSQL suite does (`connect::connect`,
//! `LegacyDb::list_all_routines`) against a real Postgres testcontainer,
//! covering exactly the path review
//! flagged as most fragile: `postgres_all_routines`'s `SELECT *` + by-name
//! column reads, which panic (not a `Result`) on a schema drift that
//! `ensure_schema_current` doesn't independently catch.
//!
//! Skips (does not fail) when Docker is unavailable, matching the pattern in
//! `ironclaw_reborn_composition/tests/postgres_substrate.rs` and
//! `ironclaw_triggers/tests/repository_contract.rs`.

use secrecy::SecretString;
use testcontainers_modules::testcontainers::{ImageExt, runners::AsyncRunner};

use super::connect::connect;
use super::types::{RoutineAction, Trigger};
use crate::options::SourceDb;

/// The production `routines` schema (`migrations/V6__routines.sql` +
/// `migrations/V13__owner_scope_notify_targets.sql`'s `notify_user` nullable
/// amendment) — exactly the 24 columns [`super::connect::ensure_schema_current`]
/// checks and [`super::queries::postgres_row_to_routine`] reads by name.
const ROUTINES_SCHEMA: &str = "
    CREATE TABLE routines (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        name TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        user_id TEXT NOT NULL,
        enabled BOOLEAN NOT NULL DEFAULT true,
        trigger_type TEXT NOT NULL,
        trigger_config JSONB NOT NULL,
        action_type TEXT NOT NULL,
        action_config JSONB NOT NULL,
        cooldown_secs INTEGER NOT NULL DEFAULT 300,
        max_concurrent INTEGER NOT NULL DEFAULT 1,
        dedup_window_secs INTEGER,
        notify_channel TEXT,
        notify_user TEXT,
        notify_on_success BOOLEAN NOT NULL DEFAULT false,
        notify_on_failure BOOLEAN NOT NULL DEFAULT true,
        notify_on_attention BOOLEAN NOT NULL DEFAULT true,
        state JSONB NOT NULL DEFAULT '{}',
        last_run_at TIMESTAMPTZ,
        next_fire_at TIMESTAMPTZ,
        run_count BIGINT NOT NULL DEFAULT 0,
        consecutive_failures INTEGER NOT NULL DEFAULT 0,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )";

/// Starts a Postgres testcontainer and returns its connection URL, or `None`
/// if Docker isn't available — callers must skip (not fail) the test.
async fn start_postgres_or_skip() -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    String,
)> {
    let image = testcontainers_modules::postgres::Postgres::default()
        .with_db_name("ironclaw_migration_test")
        .with_user("postgres")
        .with_password("postgres")
        .with_tag("16-alpine");

    let container = match image.start().await {
        Ok(container) => container,
        Err(error) => {
            eprintln!(
                "skipping ironclaw_reborn_migration Postgres tests: docker/testcontainers unavailable ({error})"
            );
            return None;
        }
    };
    let host_port = container.get_host_port_ipv4(5432).await.ok()?;
    let url = format!("postgres://postgres:postgres@127.0.0.1:{host_port}/ironclaw_migration_test");
    Some((container, url))
}

#[tokio::test]
async fn postgres_connect_and_list_all_routines_round_trips_a_real_row() {
    let Some((_container, url)) = start_postgres_or_skip().await else {
        return;
    };

    // Seed the production routines schema directly (frozen v1 SQL, not the
    // reader under test), then one row exercising both a Cron trigger and a
    // Lightweight action — the two variants `Trigger`/`RoutineAction::from_db`
    // must parse back out correctly through the `SELECT *` path.
    {
        let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("must connect to seed the schema");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .batch_execute(ROUTINES_SCHEMA)
            .await
            .expect("must create the routines table");
        client
            .execute(
                "INSERT INTO routines \
                    (name, user_id, trigger_type, trigger_config, action_type, action_config) \
                 VALUES ($1, $2, 'cron', $3, 'lightweight', $4)",
                &[
                    &"nightly-digest",
                    &"user-1",
                    &serde_json::json!({"schedule": "0 0 * * *"}),
                    &serde_json::json!({"prompt": "summarize the day"}),
                ],
            )
            .await
            .expect("must insert the fixture routine");
    }

    let source = SourceDb::Postgres {
        url: SecretString::from(url),
    };
    let (db, _handles) = connect(&source)
        .await
        .expect("connect must succeed against a schema-current Postgres");

    let routines = db
        .list_all_routines()
        .await
        .expect("list_all_routines must read the seeded row back through SELECT *");

    assert_eq!(routines.len(), 1, "exactly the one seeded routine");
    let routine = &routines[0];
    assert_eq!(routine.name, "nightly-digest");
    assert_eq!(routine.user_id, "user-1");
    assert!(
        matches!(&routine.trigger, Trigger::Cron { schedule, .. } if schedule == "0 0 * * *"),
        "cron trigger must round-trip: {:?}",
        routine.trigger
    );
    assert!(
        matches!(&routine.action, RoutineAction::Lightweight { prompt, .. } if prompt == "summarize the day"),
        "lightweight action must round-trip: {:?}",
        routine.action
    );
}

#[tokio::test]
async fn postgres_connect_fails_loud_on_stale_routines_schema() {
    let Some((_container, url)) = start_postgres_or_skip().await else {
        return;
    };

    // A `routines` table missing an expected column must be rejected by
    // `connect()`'s `ensure_schema_current` check, not silently accepted and
    // then panic later inside `postgres_all_routines`. `description` is the
    // first column `ensure_schema_current` checks that this minimal table
    // lacks.
    {
        let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("must connect to seed the schema");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .batch_execute(
                "CREATE TABLE routines (
                    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    name TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    trigger_type TEXT NOT NULL,
                    trigger_config JSONB NOT NULL,
                    action_type TEXT NOT NULL,
                    action_config JSONB NOT NULL
                )",
            )
            .await
            .expect("must create the stale routines table");
    }

    let source = SourceDb::Postgres {
        url: SecretString::from(url),
    };
    let Err(error) = connect(&source).await else {
        panic!("connect must reject a routines table missing an expected column");
    };
    let message = error.to_string();
    assert!(
        message.contains("routines") && message.contains("description"),
        "error must name the stale table and the first missing expected column: {message}"
    );
}

use ironclaw_reborn_event_store::PostgresPoolTlsOptions;
use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornEventStoreError, RebornProfile, build_reborn_event_stores,
};
use secrecy::SecretString;

const POSTGRES_MIGRATION_CONNECT_MAX_WAIT_ENV: &str =
    "IRONCLAW_FILESYSTEM_POSTGRES_MIGRATION_CONNECT_MAX_WAIT_SECS";

struct ShortMigrationConnectWait {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
}

impl ShortMigrationConnectWait {
    fn install() -> Self {
        let lock = ironclaw_common::env_helpers::lock_env();
        let previous = std::env::var_os(POSTGRES_MIGRATION_CONNECT_MAX_WAIT_ENV);
        // SAFETY: env-mutating tests serialize through the canonical
        // workspace lock for the lifetime of this guard.
        unsafe { std::env::set_var(POSTGRES_MIGRATION_CONNECT_MAX_WAIT_ENV, "1") };
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for ShortMigrationConnectWait {
    fn drop(&mut self) {
        // SAFETY: the lock remains held until after this restoration, so the
        // environment cannot be changed concurrently within this process.
        unsafe {
            if let Some(previous) = &self.previous {
                std::env::set_var(POSTGRES_MIGRATION_CONNECT_MAX_WAIT_ENV, previous);
            } else {
                std::env::remove_var(POSTGRES_MIGRATION_CONNECT_MAX_WAIT_ENV);
            }
        }
    }
}

fn rejecting_remote_postgres_url(database: &str, ssl_mode: &str) -> SecretString {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("bind loopback listener for deterministic connection failure");
    let port = listener
        .local_addr()
        .expect("loopback listener address")
        .port();

    // Keep `host` remote so the production TLS policy is exercised, while
    // `hostaddr` bypasses DNS and routes the attempted connection to this
    // hermetic endpoint. Closing the accepted socket makes the backend fail
    // immediately after policy validation instead of waiting on DNS retries.
    let _rejector = std::thread::spawn(move || {
        let _ = listener.accept();
    });

    SecretString::new(
        format!(
            "host=example.invalid hostaddr=127.0.0.1 port={port} user=event_user password=RAW_PASSWORD_SENTINEL_3162 dbname={database} sslmode={ssl_mode}"
        )
        .into_boxed_str(),
    )
}

#[tokio::test]
async fn production_profile_rejects_in_memory_before_returning_service_graph() {
    let result =
        build_reborn_event_stores(RebornProfile::Production, RebornEventStoreConfig::InMemory)
            .await;

    let error = result.err().expect("production in-memory must fail");
    assert!(matches!(
        error,
        RebornEventStoreError::ProductionInMemoryDisabled
    ));
    assert!(!error.to_string().contains("memory fallback"));
}

#[tokio::test]
async fn local_and_test_profiles_allow_explicit_in_memory_stores() {
    for profile in [RebornProfile::LocalDev, RebornProfile::Test] {
        let stores = build_reborn_event_stores(profile, RebornEventStoreConfig::InMemory)
            .await
            .expect("dev/test profiles may use explicit in-memory stores");

        assert_eq!(std::sync::Arc::strong_count(&stores.events), 1);
        assert_eq!(std::sync::Arc::strong_count(&stores.audit), 1);
    }
}

#[tokio::test]
async fn production_jsonl_requires_explicit_single_node_acceptance_without_leaking_root() {
    let root =
        std::path::PathBuf::from("/tmp/HOST_PATH_SENTINEL_3162/reborn-event-store-production");

    let result = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Jsonl {
            root,
            accept_single_node_durable: false,
        },
    )
    .await;

    let error = result
        .err()
        .expect("production JSONL must require explicit acceptance");
    assert!(matches!(
        error,
        RebornEventStoreError::ProductionJsonlRequiresAcceptance
    ));
    let displayed = error.to_string();
    assert!(!displayed.contains("HOST_PATH_SENTINEL_3162"));
    assert!(!displayed.contains("/tmp/"));
}

#[tokio::test]
async fn production_jsonl_accepts_explicit_single_node_durable_config() {
    let temp = tempfile::tempdir().expect("tempdir");

    let stores = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Jsonl {
            root: temp.path().join("event-store"),
            accept_single_node_durable: true,
        },
    )
    .await
    .expect("accepted single-node JSONL config should build");

    assert_eq!(std::sync::Arc::strong_count(&stores.events), 1);
    assert_eq!(std::sync::Arc::strong_count(&stores.audit), 1);
}

#[tokio::test]
async fn production_postgres_rejects_remote_sslmode_disable_before_connecting() {
    let result = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Postgres {
            url: SecretString::new(
                "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=disable"
                    .to_string()
                    .into_boxed_str(),
            ),
            tls_options: Default::default(),
        },
    )
    .await;

    let error = result
        .err()
        .expect("remote postgres sslmode=disable must fail closed before connect");
    assert!(matches!(
        error,
        RebornEventStoreError::RemotePostgresClearTextDisabled
    ));
    let displayed = error.to_string();
    assert!(!displayed.contains("RAW_PASSWORD_SENTINEL_3162"));
    assert!(!displayed.contains("db.example.com"));
    assert!(!displayed.contains("postgres://"));
}
#[tokio::test]
#[allow(clippy::await_holding_lock, reason = "serializes process env mutation")]
async fn production_postgres_event_store_honors_explicit_remote_cleartext_opt_in() {
    let _short_wait = ShortMigrationConnectWait::install();
    let result = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Postgres {
            url: rejecting_remote_postgres_url("events", "disable"),
            tls_options: PostgresPoolTlsOptions {
                ssl_mode_override: None,
                allow_remote_cleartext: true,
            },
        },
    )
    .await;

    let error = result
        .err()
        .expect("invalid host should fail only after accepting the cleartext opt-in");
    assert!(
        !matches!(
            error,
            RebornEventStoreError::RemotePostgresClearTextDisabled
        ),
        "event-store factory must pass explicit TLS options into the Postgres backend"
    );
    let displayed = error.to_string();
    assert!(!displayed.contains("RAW_PASSWORD_SENTINEL_3162"));
    assert!(!displayed.contains("example.invalid"));
    assert!(!displayed.contains("postgres://"));
}
#[tokio::test]
#[allow(clippy::await_holding_lock, reason = "serializes process env mutation")]
async fn postgres_connection_failure_does_not_fall_back_or_leak_secret_config() {
    let _short_wait = ShortMigrationConnectWait::install();
    let result = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Postgres {
            url: rejecting_remote_postgres_url("db", "require"),
            tls_options: Default::default(),
        },
    )
    .await;

    let error = result
        .err()
        .expect("postgres adapter should try to connect and fail closed");
    assert!(
        !matches!(
            error,
            RebornEventStoreError::BackendUnavailable {
                backend: "postgres"
            }
        ),
        "postgres config must use the concrete adapter"
    );
    let displayed = error.to_string();
    assert!(!displayed.contains("RAW_PASSWORD_SENTINEL_3162"));
    assert!(!displayed.contains("example.invalid"));
    assert!(!displayed.contains("postgres://"));
    let debug = format!("{error:?}");
    assert!(!debug.contains("RAW_PASSWORD_SENTINEL_3162"));
    assert!(!debug.contains("example.invalid"));
    assert!(!debug.contains("postgres://"));
}

#[tokio::test]
async fn production_libsql_builds_local_store_without_leaking_path_or_token() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Libsql {
            path_or_url: temp
                .path()
                .join("RAW_PATH_SENTINEL_3162")
                .join("events.db")
                .display()
                .to_string(),
            auth_token: Some(SecretString::new(
                "RAW_LIBSQL_TOKEN_SENTINEL_3162"
                    .to_string()
                    .into_boxed_str(),
            )),
        },
    )
    .await
    .expect("local libsql event store should build");

    assert_eq!(std::sync::Arc::strong_count(&stores.events), 1);
    assert_eq!(std::sync::Arc::strong_count(&stores.audit), 1);
}

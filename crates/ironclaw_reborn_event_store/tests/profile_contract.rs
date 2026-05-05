use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornEventStoreError, RebornProfile, build_reborn_event_stores,
};
use secrecy::SecretString;

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

#[cfg(not(feature = "postgres"))]
#[tokio::test]
async fn unavailable_postgres_backend_error_does_not_leak_secret_config() {
    let result = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Postgres {
            url: SecretString::new(
                "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@example.invalid/db"
                    .to_string()
                    .into_boxed_str(),
            ),
        },
    )
    .await;

    let error = result
        .err()
        .expect("postgres adapter is unavailable when the feature is disabled");
    assert!(matches!(
        error,
        RebornEventStoreError::BackendUnavailable {
            backend: "postgres"
        }
    ));
    let displayed = error.to_string();
    assert!(!displayed.contains("RAW_PASSWORD_SENTINEL_3162"));
    assert!(!displayed.contains("example.invalid"));
    assert!(!displayed.contains("postgres://"));
    let debug = format!("{error:?}");
    assert!(!debug.contains("RAW_PASSWORD_SENTINEL_3162"));
    assert!(!debug.contains("example.invalid"));
    assert!(!debug.contains("postgres://"));
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_connection_failure_does_not_fall_back_or_leak_secret_config() {
    let result = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Postgres {
            url: SecretString::new(
                "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@example.invalid/db"
                    .to_string()
                    .into_boxed_str(),
            ),
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
        "postgres feature must enable the concrete adapter"
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

#[cfg(not(feature = "libsql"))]
#[tokio::test]
async fn unavailable_libsql_backend_error_does_not_leak_secret_config() {
    let result = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Libsql {
            path_or_url: "libsql://RAW_HOST_SENTINEL_3162.example.invalid".to_string(),
            auth_token: Some(SecretString::new(
                "RAW_LIBSQL_TOKEN_SENTINEL_3162"
                    .to_string()
                    .into_boxed_str(),
            )),
        },
    )
    .await;

    let error = result
        .err()
        .expect("libsql adapter is unavailable when the feature is disabled");
    assert!(matches!(
        error,
        RebornEventStoreError::BackendUnavailable { backend: "libsql" }
    ));
    let displayed = error.to_string();
    assert!(!displayed.contains("RAW_LIBSQL_TOKEN_SENTINEL_3162"));
    assert!(!displayed.contains("RAW_HOST_SENTINEL_3162"));
    assert!(!displayed.contains("libsql://"));
}

#[cfg(feature = "libsql")]
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

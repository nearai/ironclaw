// safety: test fixtures intentionally seed and inspect durable rows across separate service rebuilds; no single atomic operation.
use super::*;

#[cfg(feature = "libsql")]
async fn libsql_entry_version(db: &libsql::Database, path: &str) -> i64 {
    let conn = db.connect().expect("connect libsql db");
    let mut rows = conn
        .query(
            "SELECT version FROM root_filesystem_entries WHERE path = ?1",
            libsql::params![path],
        )
        .await
        .expect("query durable record version");
    let row = rows
        .next()
        .await
        .expect("read durable record version row")
        .expect("durable record version row exists");
    row.get(0).expect("durable record version")
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn local_dev_libsql_rebuild_migrates_slack_personal_before_publishing_services() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("local-dev");
    let scope = auth_scope("local-slack-migration");
    let first_services = build_reborn_services(
        RebornBuildInput::local_dev("test-owner", root.clone())
            .with_runtime_policy(local_only_runtime_policy()),
    )
    .await
    .unwrap();
    let product_auth = first_services.product_auth.as_ref().unwrap();
    let challenge = product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest::new(
            scope.clone(),
            ironclaw_auth::AuthProviderId::new("slack_personal").unwrap(),
            ironclaw_auth::CredentialAccountLabel::new("Local legacy Slack").unwrap(),
            ironclaw_auth::AuthContinuationRef::SetupOnly,
            Utc::now() + chrono::Duration::minutes(5),
        ))
        .await
        .unwrap();
    let submitted = product_auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope.clone(),
            challenge.interaction_id,
            SecretString::from("local-legacy-slack-token"),
        ))
        .await
        .unwrap();
    let before = product_auth
        .credential_account_service()
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope.clone(),
            submitted.account_id,
        ))
        .await
        .unwrap()
        .unwrap();
    drop(first_services);

    let rebuilt = build_reborn_services(
        RebornBuildInput::local_dev("test-owner", root)
            .with_runtime_policy(local_only_runtime_policy()),
    )
    .await
    .expect("local rebuild migrates durable auth before returning services");
    let after = rebuilt
        .product_auth
        .as_ref()
        .unwrap()
        .credential_account_service()
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope,
            submitted.account_id,
        ))
        .await
        .unwrap()
        .unwrap();
    let mut expected = before;
    expected.provider = ironclaw_auth::AuthProviderId::new("slack").unwrap();
    assert_eq!(after, expected);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_libsql_migrates_slack_personal_before_publishing_services() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let events = dir.path().join("events.db").to_string_lossy().into_owned();
    let scope = auth_scope("slack-migration");

    let (first_notifier, first_handle) = live_wake_notifier();
    let first_services = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            Arc::clone(&db),
            &events,
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(first_notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await
    .expect("first production build seeds durable product auth");
    let first_product_auth = first_services
        .product_auth
        .as_ref()
        .expect("production publishes product auth after startup");
    let retired = ironclaw_auth::AuthProviderId::new("slack_personal").unwrap();
    let challenge = first_product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest::new(
            scope.clone(),
            retired,
            ironclaw_auth::CredentialAccountLabel::new("Slack personal legacy").unwrap(),
            ironclaw_auth::AuthContinuationRef::SetupOnly,
            Utc::now() + chrono::Duration::minutes(5),
        ))
        .await
        .unwrap();
    let submitted = first_product_auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope.clone(),
            challenge.interaction_id,
            SecretString::from("legacy-slack-token"),
        ))
        .await
        .unwrap();
    let before = first_product_auth
        .credential_account_service()
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope.clone(),
            submitted.account_id,
        ))
        .await
        .unwrap()
        .expect("seeded legacy account is durable");
    first_handle.shutdown().await;
    drop(first_services);

    let (second_notifier, second_handle) = live_wake_notifier();
    let second_services = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            Arc::clone(&db),
            &events,
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(second_notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await
    .expect("second production build completes the migration before returning services");
    let after = second_services
        .product_auth
        .as_ref()
        .expect("migrated product auth is published")
        .credential_account_service()
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope.clone(),
            submitted.account_id,
        ))
        .await
        .unwrap()
        .expect("migrated account remains resolvable");
    let mut expected = before;
    expected.provider = ironclaw_auth::AuthProviderId::new("slack").unwrap();
    assert_eq!(
        after, expected,
        "production migration changes only provider"
    );
    let account_path = format!(
        "/tenants/{}/users/{}/secrets/agents/{}/projects/{}/product-auth/web/sessions/{}/accounts/{}.json",
        scope.resource.tenant_id,
        scope.resource.user_id,
        scope.resource.agent_id.as_ref().unwrap(),
        scope.resource.project_id.as_ref().unwrap(),
        scope.session_id.as_ref().unwrap(),
        submitted.account_id
    );
    let version_after_second = libsql_entry_version(&db, &account_path).await;
    second_handle.shutdown().await;
    drop(second_services);

    let (third_notifier, third_handle) = live_wake_notifier();
    let third_services = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            Arc::clone(&db),
            &events,
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(third_notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await
    .expect("third production build is an idempotent migration no-op");
    assert_eq!(
        libsql_entry_version(&db, &account_path).await,
        version_after_second,
        "third build must not rewrite the already-migrated record"
    );
    assert!(third_services.product_auth.is_some());
    third_handle.shutdown().await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_libsql_malformed_product_auth_record_fails_typed_migration() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let events = dir.path().join("events.db").to_string_lossy().into_owned();
    let scope = auth_scope("slack-malformed");
    let (first_notifier, first_handle) = live_wake_notifier();
    let first_services = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            Arc::clone(&db),
            &events,
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(first_notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await
    .unwrap();
    let product_auth = first_services.product_auth.as_ref().unwrap();
    let challenge = product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest::new(
            scope.clone(),
            ironclaw_auth::AuthProviderId::new("slack_personal").unwrap(),
            ironclaw_auth::CredentialAccountLabel::new("Malformed legacy Slack").unwrap(),
            ironclaw_auth::AuthContinuationRef::SetupOnly,
            Utc::now() + chrono::Duration::minutes(5),
        ))
        .await
        .unwrap();
    let submitted = product_auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope.clone(),
            challenge.interaction_id,
            SecretString::from("legacy-slack-token"),
        ))
        .await
        .unwrap();
    first_handle.shutdown().await;
    drop(first_services);

    let account_path = format!(
        "/tenants/{}/users/{}/secrets/agents/{}/projects/{}/product-auth/web/sessions/{}/accounts/{}.json",
        scope.resource.tenant_id,
        scope.resource.user_id,
        scope.resource.agent_id.as_ref().unwrap(),
        scope.resource.project_id.as_ref().unwrap(),
        scope.session_id.as_ref().unwrap(),
        submitted.account_id
    );
    db.connect()
        .unwrap()
        .execute(
            "UPDATE root_filesystem_entries SET contents = ?1 WHERE path = ?2",
            libsql::params![vec![0xff_u8], account_path],
        )
        .await
        .expect("corrupt seeded record for strict startup test");

    let (second_notifier, second_handle) = live_wake_notifier();
    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            Arc::clone(&db),
            &events,
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(second_notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await;
    second_handle.shutdown().await;

    assert!(matches!(
        result,
        Err(RebornBuildError::ProductAuthMigration(
            ironclaw_auth::AuthProductError::BackendUnavailable
        ))
    ));
}

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "live prerequisite: requires Docker and a reachable Postgres testcontainer"]
async fn production_postgres_migrates_slack_personal_before_publishing_services() {
    let (_container, pool, database_url) = postgres_pool_or_skip()
        .await
        .expect("live Postgres migration lane requires Docker and a reachable testcontainer");
    let scope = auth_scope("postgres-slack-migration");
    let (first_notifier, first_handle) = live_wake_notifier();
    let first_services = build_reborn_services(
        RebornBuildInput::postgres(
            RebornCompositionProfile::Production,
            "test-owner",
            pool.clone(),
            SecretMaterial::from(database_url.clone()),
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(first_notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await
    .unwrap();
    let product_auth = first_services.product_auth.as_ref().unwrap();
    let challenge = product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest::new(
            scope.clone(),
            ironclaw_auth::AuthProviderId::new("slack_personal").unwrap(),
            ironclaw_auth::CredentialAccountLabel::new("Postgres legacy Slack").unwrap(),
            ironclaw_auth::AuthContinuationRef::SetupOnly,
            Utc::now() + chrono::Duration::minutes(5),
        ))
        .await
        .unwrap();
    let submitted = product_auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope.clone(),
            challenge.interaction_id,
            SecretString::from("postgres-legacy-slack-token"),
        ))
        .await
        .unwrap();
    let before = product_auth
        .credential_account_service()
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope.clone(),
            submitted.account_id,
        ))
        .await
        .unwrap()
        .unwrap();
    first_handle.shutdown().await;
    drop(first_services);

    let (second_notifier, second_handle) = live_wake_notifier();
    let second_services = build_reborn_services(
        RebornBuildInput::postgres(
            RebornCompositionProfile::Production,
            "test-owner",
            pool.clone(),
            SecretMaterial::from(database_url.clone()),
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(second_notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await
    .expect("Postgres rebuild migrates before services are published");
    let after = second_services
        .product_auth
        .as_ref()
        .unwrap()
        .credential_account_service()
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope.clone(),
            submitted.account_id,
        ))
        .await
        .unwrap()
        .unwrap();
    let mut expected = before;
    expected.provider = ironclaw_auth::AuthProviderId::new("slack").unwrap();
    assert_eq!(after, expected);
    let account_path = format!(
        "/tenants/{}/users/{}/secrets/agents/{}/projects/{}/product-auth/web/sessions/{}/accounts/{}.json",
        scope.resource.tenant_id,
        scope.resource.user_id,
        scope.resource.agent_id.as_ref().unwrap(),
        scope.resource.project_id.as_ref().unwrap(),
        scope.session_id.as_ref().unwrap(),
        submitted.account_id
    );
    let client = pool.get().await.unwrap();
    let version_after_second: i64 = client
        .query_one(
            "SELECT version FROM root_filesystem_entries WHERE path = $1",
            &[&account_path],
        )
        .await
        .unwrap()
        .get(0);
    drop(client);
    second_handle.shutdown().await;
    drop(second_services);

    let (third_notifier, third_handle) = live_wake_notifier();
    let third_services = build_reborn_services(
        RebornBuildInput::postgres(
            RebornCompositionProfile::Production,
            "test-owner",
            pool.clone(),
            SecretMaterial::from(database_url),
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(third_notifier)
        .with_runtime_process_binding(test_sandbox_process_binding()),
    )
    .await
    .expect("third Postgres build is an idempotent migration no-op");
    let client = pool.get().await.unwrap();
    let version_after_third: i64 = client
        .query_one(
            "SELECT version FROM root_filesystem_entries WHERE path = $1",
            &[&account_path],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(version_after_third, version_after_second);
    assert!(third_services.product_auth.is_some());
    third_handle.shutdown().await;
}

#![cfg(feature = "libsql")]

use std::path::{Path, PathBuf};

use ironclaw_host_api::{AgentId, TenantId};
use ironclaw_reborn_migration::{
    ApplyAcknowledgements, Domain, MigrationOptions, MigrationSecretInputs, MigrationStatus,
    SourceDb, TargetStore, apply_migration, manifest_target_matches, plan_migration,
    verify_migration,
};

async fn seed_source(path: &Path) {
    let database = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("build source");
    let connection = database.connect().expect("connect source");
    connection
        .execute_batch(
            "CREATE TABLE settings (
                user_id TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                PRIMARY KEY (user_id, key)
             );
             INSERT INTO settings (user_id, key, value)
             VALUES ('user-1', 'model', '\"gpt-test\"');",
        )
        .await
        .expect("seed source");
}

async fn source_contents(path: &Path) -> Vec<(String, String, String)> {
    let database = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("open source");
    let connection = database.connect().expect("connect source");
    let mut rows = connection
        .query(
            "SELECT user_id, key, value FROM settings ORDER BY user_id, key",
            (),
        )
        .await
        .expect("query source");
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.expect("next row") {
        out.push((
            row.get(0).expect("user id"),
            row.get(1).expect("key"),
            row.get(2).expect("value"),
        ));
    }
    out
}

fn options(source: PathBuf, target: PathBuf) -> MigrationOptions {
    let source_home = source.parent().map(Path::to_path_buf);
    MigrationOptions {
        source: SourceDb::LibSql { path: source },
        source_home,
        target: TargetStore::LibSql { path: target },
        profile: "test-migration".to_string(),
        tenant_id: TenantId::new("migration-tenant").expect("tenant"),
        agent_id: AgentId::new("migration-agent").expect("agent"),
        secret_master_key: None,
        dry_run: true,
    }
}

#[tokio::test]
async fn apply_rejects_scope_drift_before_creating_target() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source = directory.path().join("source.db");
    let target = directory.path().join("new-target").join("target.db");
    seed_source(&source).await;
    let planned_options = options(source, target.clone());
    let manifest = plan_migration(&planned_options).await.expect("plan");

    let mut changed = planned_options;
    changed.profile = "different-profile".to_string();
    let error = apply_migration(
        changed,
        &manifest,
        MigrationSecretInputs::default(),
        ApplyAcknowledgements::offline_snapshot(),
    )
    .await
    .expect_err("scope drift must fail");

    assert!(error.to_string().contains("sealed plan"));
    assert!(!target.exists());
}

#[tokio::test]
async fn explicit_source_home_is_sealed_and_not_inferred_from_snapshot() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source_home = directory.path().join("v1-home");
    let backup_dir = directory.path().join("backups");
    std::fs::create_dir_all(&source_home).expect("source home");
    std::fs::create_dir_all(&backup_dir).expect("backup dir");
    std::fs::write(source_home.join("settings.json"), b"{}").expect("home artifact");
    let source = backup_dir.join("source.db");
    seed_source(&source).await;
    let target = directory.path().join("target.db");
    let mut migration_options = options(source, target.clone());
    migration_options.source_home = Some(source_home.clone());

    let manifest = plan_migration(&migration_options).await.expect("plan");
    assert_eq!(
        manifest
            .inventory
            .iter()
            .find(|entry| entry.source_name == "settings.json")
            .expect("settings inventory")
            .count,
        1
    );

    migration_options.source_home = Some(backup_dir);
    let error = apply_migration(
        migration_options,
        &manifest,
        MigrationSecretInputs::default(),
        ApplyAcknowledgements::offline_snapshot(),
    )
    .await
    .expect_err("source home drift must fail");
    assert!(error.to_string().contains("sealed plan"));
    assert!(!target.exists());
}

#[tokio::test]
async fn missing_source_home_blocks_apply() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source = directory.path().join("source.db");
    let target = directory.path().join("target.db");
    seed_source(&source).await;
    let mut migration_options = options(source, target.clone());
    migration_options.source_home = None;

    let manifest = plan_migration(&migration_options).await.expect("plan");
    assert!(
        manifest
            .inventory
            .iter()
            .any(|entry| entry.source_name == "v1_home" && entry.blocker.is_some())
    );
    let error = apply_migration(
        migration_options,
        &manifest,
        MigrationSecretInputs::default(),
        ApplyAcknowledgements::offline_snapshot(),
    )
    .await
    .expect_err("missing source home must block apply");
    assert!(error.to_string().contains("inventory blockers"));
    assert!(!target.exists());
}

#[tokio::test]
async fn plan_is_source_read_only_and_does_not_create_target() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source = directory.path().join("source-with-password-canary.db");
    let target = directory.path().join("new-reborn-home").join("target.db");
    seed_source(&source).await;
    let before = source_contents(&source).await;
    let bytes_before = std::fs::read(&source).expect("read source snapshot");

    let manifest = plan_migration(&options(source.clone(), target.clone()))
        .await
        .expect("plan");

    assert_eq!(manifest.status, MigrationStatus::Planned);
    assert_eq!(source_contents(&source).await, before);
    assert_eq!(
        std::fs::read(&source).expect("read source after plan"),
        bytes_before,
        "planning modified source database bytes"
    );
    assert!(!target.exists(), "planning created the target database");
    assert!(
        !target.parent().expect("target parent").exists(),
        "planning created the target home"
    );
    let json = manifest.to_json().expect("manifest json");
    assert!(!json.contains("source-with-password-canary.db"));
    assert!(!json.contains(&source.display().to_string()));
    manifest.validate_plan_hash().expect("sealed plan");
    assert!(manifest_target_matches(
        &TargetStore::LibSql {
            path: target.clone()
        },
        &manifest
    ));
    assert!(!manifest_target_matches(
        &TargetStore::LibSql {
            path: directory.path().join("different-target.db")
        },
        &manifest
    ));
    std::fs::create_dir_all(target.parent().expect("target parent")).expect("create target parent");
    std::fs::File::create(&target).expect("create target");
    assert!(
        manifest_target_matches(&TargetStore::LibSql { path: target }, &manifest),
        "target locator fingerprint must be stable across missing to created"
    );
}

#[tokio::test]
async fn plan_inventory_accounts_for_known_tables_and_home_artifacts() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source = directory.path().join("source.db");
    let target = directory.path().join("target.db");
    seed_source(&source).await;

    let manifest = plan_migration(&options(source, target))
        .await
        .expect("plan");
    for table in [
        "conversations",
        "conversation_messages",
        "users",
        "api_tokens",
        "settings",
        "memory_documents",
        "memory_document_versions",
        "routines",
        "routine_runs",
        "secrets",
        "wasm_tools",
        "wasm_channels",
        "pairing_requests",
        "claude_code_events",
        "root_filesystem_entries",
    ] {
        assert!(
            manifest
                .inventory
                .iter()
                .any(|item| item.source_name == table),
            "missing table disposition for {table}"
        );
    }
    for artifact in [
        ".env",
        "settings.json",
        "config.toml",
        "providers.json",
        "session.json",
        "mcp-servers.json",
        "acp-agents.json",
        "profiles",
        "skills",
        "installed_skills",
        "tools",
        "channels",
        "projects",
        "history",
        "logs",
    ] {
        assert!(
            manifest
                .inventory
                .iter()
                .any(|item| item.source_name == artifact),
            "missing home disposition for {artifact}"
        );
    }
    assert_eq!(
        manifest
            .inventory
            .iter()
            .find(|item| item.source_name == "settings")
            .expect("settings inventory")
            .count,
        1
    );
    assert!(manifest.domains.contains_key(&Domain::Setting));
}

#[tokio::test]
async fn apply_rejects_changed_source_before_creating_target() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source = directory.path().join("source.db");
    let target = directory.path().join("new-target").join("target.db");
    seed_source(&source).await;
    let options = options(source.clone(), target.clone());
    let manifest = plan_migration(&options).await.expect("plan");

    let database = libsql::Builder::new_local(&source)
        .build()
        .await
        .expect("open source");
    database
        .connect()
        .expect("connect source")
        .execute(
            "UPDATE settings SET value = '\"changed\"' WHERE key = 'model'",
            (),
        )
        .await
        .expect("change source");

    let error = apply_migration(
        options,
        &manifest,
        MigrationSecretInputs::default(),
        ApplyAcknowledgements::offline_snapshot(),
    )
    .await
    .expect_err("changed source must fail");
    assert!(error.to_string().contains("fingerprint changed"));
    assert!(!target.exists());
}

#[tokio::test]
async fn same_source_and_target_is_rejected_without_writes() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source = directory.path().join("source.db");
    seed_source(&source).await;
    let before = source_contents(&source).await;

    let error = plan_migration(&options(source.clone(), source.clone()))
        .await
        .expect_err("same store must fail");
    assert!(error.to_string().contains("must be different"));
    assert_eq!(source_contents(&source).await, before);
}

#[tokio::test]
async fn apply_requires_both_offline_snapshot_acknowledgements() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source = directory.path().join("source.db");
    let target = directory.path().join("new-target").join("target.db");
    seed_source(&source).await;
    let options = options(source, target.clone());
    let manifest = plan_migration(&options).await.expect("plan");

    let error = apply_migration(
        options,
        &manifest,
        MigrationSecretInputs::default(),
        ApplyAcknowledgements {
            source_is_stopped: true,
            source_is_snapshot: false,
        },
    )
    .await
    .expect_err("missing snapshot acknowledgement must fail");
    assert!(error.to_string().contains("consistent snapshot"));
    assert!(!target.exists());
}

#[tokio::test]
async fn manifest_write_is_owner_only_and_no_clobber_by_default() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source = directory.path().join("source.db");
    let target = directory.path().join("target.db");
    seed_source(&source).await;
    let manifest = plan_migration(&options(source, target))
        .await
        .expect("plan");
    let manifest_path = directory.path().join("reports").join("plan.json");

    manifest
        .write_atomic(&manifest_path, false)
        .expect("write manifest");
    assert!(manifest.write_atomic(&manifest_path, false).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&manifest_path)
            .expect("manifest metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[tokio::test]
async fn verification_requires_an_active_target_claim() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source = directory.path().join("source.db");
    let target = directory.path().join("target.db");
    seed_source(&source).await;
    let options = options(source, target);
    let planned = plan_migration(&options).await.expect("plan");
    let applying = planned
        .transition(MigrationStatus::Applying)
        .expect("applying");
    let applied = applying
        .transition(MigrationStatus::Applied)
        .expect("applied");
    let error = verify_migration(&options, &applied)
        .await
        .expect_err("an unclaimed target cannot be verified");
    assert!(
        error.to_string().contains("no active migration claim"),
        "unexpected verification error: {error}"
    );

    let verifying = applied
        .transition(MigrationStatus::Verifying)
        .expect("persist verifying before readback");
    let resumed_error = verify_migration(&options, &verifying)
        .await
        .expect_err("resumed verification still requires an active target claim");
    assert!(
        resumed_error
            .to_string()
            .contains("no active migration claim"),
        "unexpected resumed verification error: {resumed_error}"
    );

    let verified = verifying
        .transition(MigrationStatus::Verified)
        .expect("verified");
    let reverify_error = verify_migration(&options, &verified)
        .await
        .expect_err("verified manifests still require an active target claim");
    assert!(
        reverify_error
            .to_string()
            .contains("no active migration claim"),
        "unexpected re-verification error: {reverify_error}"
    );
}

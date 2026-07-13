#![cfg(feature = "libsql")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ironclaw::config::{DatabaseBackend, DatabaseConfig, SslMode};
use ironclaw::db::{Database, connect_with_handles};
use ironclaw_filesystem::ScopedFilesystem;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_projects::{FilesystemProjectRepository, ProjectRepository};
use ironclaw_reborn_migration::{
    ApplyAcknowledgements, Domain, MigrationOptions, MigrationSecretInputs, SourceDb, TargetStore,
    apply_migration, plan_migration, resume_migration,
};
use secrecy::SecretString;
use uuid::Uuid;

const TENANT: &str = "acme";
const AGENT: &str = "assistant";
const USER: &str = "alice";
const MASTER_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn libsql_config(path: &Path) -> DatabaseConfig {
    DatabaseConfig {
        backend: DatabaseBackend::LibSql,
        url: SecretString::from("unused://libsql"),
        pool_size: 2,
        ssl_mode: SslMode::default(),
        libsql_path: Some(path.to_path_buf()),
        libsql_url: None,
        libsql_auth_token: None,
    }
}

async fn seed_source(path: &Path, project_a: Uuid, project_b: Uuid) {
    let (database, _handles) = connect_with_handles(&libsql_config(path))
        .await
        .expect("open v1 fixture");
    write_project(
        database.as_ref(),
        "projects/alpha/.project.json",
        project_a,
        "Alpha",
        "First project",
        ("2024-01-02T03:04:05Z", "2024-02-03T04:05:06Z"),
        None,
    )
    .await;
    write_project(
        database.as_ref(),
        ".system/engine/projects/beta/project.json",
        project_b,
        "Beta",
        "Second project",
        ("2023-01-02T03:04:05Z", "2023-02-03T04:05:06Z"),
        Some(Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap()),
    )
    .await;
}

async fn write_project(
    database: &dyn Database,
    path: &str,
    id: Uuid,
    name: &str,
    description: &str,
    timestamps: (&str, &str),
    agent_id: Option<Uuid>,
) {
    let (created_at, updated_at) = timestamps;
    let document = database
        .get_or_create_document_by_path(USER, agent_id, path)
        .await
        .expect("create project document");
    let body = serde_json::json!({
        "id": id,
        "user_id": USER,
        "name": name,
        "description": description,
        "goals": [format!("ship {name}")],
        "metrics": [{"name": "quality", "target": "high"}],
        "metadata": {"legacy_label": name.to_ascii_lowercase()},
        "workspace_path": format!("/srv/projects/{}", name.to_ascii_lowercase()),
        "created_at": created_at,
        "updated_at": updated_at,
    });
    database
        .update_document(document.id, &body.to_string())
        .await
        .expect("write project document");
}

fn options(source: PathBuf, target: PathBuf) -> MigrationOptions {
    let source_home = source.parent().map(Path::to_path_buf);
    MigrationOptions {
        source: SourceDb::LibSql { path: source },
        source_home,
        target: TargetStore::LibSql { path: target },
        profile: "test-migration".to_string(),
        tenant_id: TenantId::new(TENANT).expect("tenant"),
        agent_id: AgentId::new(AGENT).expect("agent"),
        secret_master_key: Some(SecretString::from(MASTER_KEY)),
        dry_run: false,
    }
}

fn migration_secrets() -> MigrationSecretInputs {
    MigrationSecretInputs {
        source_master_key: Some(SecretString::from(MASTER_KEY)),
        target_master_key: None,
    }
}

async fn project_repository(
    target: &Path,
) -> FilesystemProjectRepository<ironclaw_filesystem::LibSqlRootFilesystem> {
    let database = Arc::new(
        libsql::Builder::new_local(target)
            .build()
            .await
            .expect("open Reborn target"),
    );
    let root = Arc::new(ironclaw_filesystem::LibSqlRootFilesystem::new(database));
    let scoped = Arc::new(ScopedFilesystem::new(
        root,
        ironclaw_reborn_composition::invocation_mount_view,
    ));
    FilesystemProjectRepository::new(
        scoped,
        UserId::new("project-readback").expect("readback user"),
        AgentId::new(AGENT).expect("agent"),
    )
}

#[tokio::test]
async fn migrates_both_project_layouts_and_replay_conflicts_fail_closed() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source = directory.path().join("v1.db");
    let target = directory.path().join("reborn/reborn.db");
    let project_a = Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
    let project_b = Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap();
    seed_source(&source, project_a, project_b).await;

    let migration_options = options(source.clone(), target.clone());
    let manifest = plan_migration(&migration_options).await.expect("plan");
    assert_eq!(
        manifest
            .domains
            .get(&Domain::Project)
            .expect("project checkpoint")
            .planned,
        2,
        "planning must count supported engine-v2 project documents"
    );
    let report = apply_migration(
        migration_options,
        &manifest,
        migration_secrets(),
        ApplyAcknowledgements::offline_snapshot(),
    )
    .await
    .expect("apply projects");
    assert_eq!(report.stats.projects, 2);
    assert_eq!(report.losses_in(Domain::Project), 0);

    let repository = project_repository(&target).await;
    let tenant = TenantId::new(TENANT).unwrap();
    let alpha = repository
        .get_project(&tenant, &ProjectId::new(project_a.to_string()).unwrap())
        .await
        .expect("read alpha")
        .expect("alpha exists");
    assert_eq!(alpha.owner_user_id.as_str(), USER);
    assert_eq!(alpha.name, "Alpha");
    assert_eq!(alpha.description, "First project");
    assert_eq!(alpha.created_at.to_rfc3339(), "2024-01-02T03:04:05+00:00");
    assert_eq!(alpha.updated_at.to_rfc3339(), "2024-02-03T04:05:06+00:00");
    assert_eq!(alpha.metadata["legacy_engine_v2"]["goals"][0], "ship Alpha");
    assert_eq!(
        alpha.metadata["legacy_engine_v2"]["metadata"]["legacy_label"],
        "alpha"
    );
    assert!(
        repository
            .get_project(&tenant, &ProjectId::new(project_b.to_string()).unwrap())
            .await
            .expect("read beta")
            .is_some(),
        "the .system/engine layout must also import"
    );

    let replay = resume_migration(
        options(source.clone(), target.clone()),
        report.manifest.as_ref().expect("applied manifest"),
        migration_secrets(),
        ApplyAcknowledgements::offline_snapshot(),
    )
    .await
    .expect("exact replay");
    assert_eq!(replay.stats.projects, 2);

    let mut divergent = alpha;
    divergent.description = "operator changed this project".to_string();
    repository
        .update_project(divergent)
        .await
        .expect("mutate deterministic target slot");
    let error = resume_migration(
        options(source, target),
        replay.manifest.as_ref().expect("replay manifest"),
        migration_secrets(),
        ApplyAcknowledgements::offline_snapshot(),
    )
    .await
    .expect_err("divergent project must conflict");
    assert!(
        error.to_string().contains("refusing to overwrite"),
        "unexpected project conflict: {error}"
    );
}

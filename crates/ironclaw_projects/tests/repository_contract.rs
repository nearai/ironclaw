//! Backend-agnostic contract tests for [`ProjectRepository`].
//!
//! The sole implementation is `FilesystemProjectRepository`, which persists over
//! the Reborn `ScopedFilesystem` substrate. These assertions run it against an
//! in-memory `RootFilesystem`; backend correctness (Postgres / libSQL / JSONL)
//! is `ironclaw_filesystem`'s concern, so a single in-memory run exercises all
//! repository logic with no per-backend duplication.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, UserId,
    VirtualPath,
};
use ironclaw_projects::{
    FilesystemProjectRepository, MAX_PROJECT_METADATA_BYTES, ProjectMemberRecord,
    ProjectMemberStatus, ProjectRecord, ProjectRepository, ProjectRole, ProjectState,
};

fn tenant() -> TenantId {
    TenantId::new("tenant1").unwrap()
}

fn user(name: &str) -> UserId {
    UserId::new(name).unwrap()
}

fn member(
    project_id: &ProjectId,
    user_id: &UserId,
    granted_by: &UserId,
    role: ProjectRole,
    status: ProjectMemberStatus,
) -> ProjectMemberRecord {
    let now = Utc::now();
    ProjectMemberRecord {
        tenant_id: tenant(),
        project_id: project_id.clone(),
        user_id: user_id.clone(),
        role,
        status,
        granted_by: granted_by.clone(),
        created_at: now,
        updated_at: now,
    }
}

/// Runs the full contract against any repository implementation.
async fn run_contract(repo: &dyn ProjectRepository) {
    let owner = user("alice");
    let bob = user("bob");

    let record = ProjectRecord::new(tenant(), owner.clone(), "Research", "AI research").unwrap();
    let project_id = record.project_id.clone();

    // Create + duplicate rejection.
    repo.create_project(record.clone()).await.unwrap();
    assert!(matches!(
        repo.create_project(record.clone()).await,
        Err(ironclaw_projects::ProjectError::AlreadyExists)
    ));

    // Owner resolves to Owner; stranger has no access.
    assert_eq!(
        repo.resolve_access(&tenant(), &project_id, &owner)
            .await
            .unwrap(),
        Some(ProjectRole::Owner)
    );
    assert_eq!(
        repo.resolve_access(&tenant(), &project_id, &bob)
            .await
            .unwrap(),
        None
    );

    // Grant Bob editor access; it shows up in resolve + listing + members.
    repo.upsert_member(member(
        &project_id,
        &bob,
        &owner,
        ProjectRole::Editor,
        ProjectMemberStatus::Active,
    ))
    .await
    .unwrap();
    assert_eq!(
        repo.resolve_access(&tenant(), &project_id, &bob)
            .await
            .unwrap(),
        Some(ProjectRole::Editor)
    );
    assert_eq!(
        repo.list_projects_for_user(&tenant(), &bob, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        repo.list_members(&tenant(), &project_id)
            .await
            .unwrap()
            .len(),
        1
    );

    // Regression: `created_at` is immutable across a member upsert update — a
    // role/status change must not rewrite the grant's original creation time.
    let member_created = repo.list_members(&tenant(), &project_id).await.unwrap()[0].created_at;
    let mut rebumped = member(
        &project_id,
        &bob,
        &owner,
        ProjectRole::Viewer,
        ProjectMemberStatus::Active,
    );
    rebumped.created_at = member_created + chrono::Duration::days(7);
    repo.upsert_member(rebumped).await.unwrap();
    let after_upsert = repo.list_members(&tenant(), &project_id).await.unwrap();
    assert_eq!(after_upsert.len(), 1);
    assert_eq!(
        after_upsert[0].created_at, member_created,
        "member created_at must be immutable across upsert update"
    );
    assert_eq!(
        after_upsert[0].role,
        ProjectRole::Viewer,
        "role update still persists across upsert"
    );

    // Update persists; metadata bag round-trips.
    let mut updated = repo
        .get_project(&tenant(), &project_id)
        .await
        .unwrap()
        .unwrap();
    updated.name = "Research v2".to_string();
    updated.metadata = serde_json::json!({ "github": "org/repo", "goals": ["ship"] });
    updated.updated_at = Utc::now();
    repo.update_project(updated.clone()).await.unwrap();
    let reloaded = repo
        .get_project(&tenant(), &project_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.name, "Research v2");
    assert_eq!(reloaded.metadata["github"], "org/repo");

    // Revoke Bob; access disappears immediately.
    repo.upsert_member(member(
        &project_id,
        &bob,
        &owner,
        ProjectRole::Editor,
        ProjectMemberStatus::Revoked,
    ))
    .await
    .unwrap();
    assert_eq!(
        repo.resolve_access(&tenant(), &project_id, &bob)
            .await
            .unwrap(),
        None
    );
    assert!(
        repo.list_projects_for_user(&tenant(), &bob, 10)
            .await
            .unwrap()
            .is_empty()
    );

    // Remove the member row entirely.
    assert!(
        repo.remove_member(&tenant(), &project_id, &bob)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        repo.list_members(&tenant(), &project_id)
            .await
            .unwrap()
            .is_empty()
    );

    // `created_at` is immutable across update; `state` round-trips (incl. archived).
    let before = repo
        .get_project(&tenant(), &project_id)
        .await
        .unwrap()
        .unwrap();
    let mut bumped = before.clone();
    bumped.created_at = before.created_at + chrono::Duration::days(365);
    bumped.state = ProjectState::Archived;
    bumped.updated_at = Utc::now();
    repo.update_project(bumped).await.unwrap();
    let after = repo
        .get_project(&tenant(), &project_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.created_at, before.created_at,
        "created_at must be immutable across update"
    );
    assert_eq!(
        after.state,
        ProjectState::Archived,
        "archived state must round-trip"
    );

    // Cross-tenant isolation: a project in another tenant is invisible.
    let other_tenant = TenantId::new("tenant2").unwrap();
    let other = ProjectRecord::new(other_tenant.clone(), owner.clone(), "Other", "").unwrap();
    let other_id = other.project_id.clone();
    let other_created = other.created_at;
    repo.create_project(other).await.unwrap();
    assert!(
        repo.get_project(&tenant(), &other_id)
            .await
            .unwrap()
            .is_none(),
        "cross-tenant get must miss"
    );
    assert_eq!(
        repo.resolve_access(&tenant(), &other_id, &owner)
            .await
            .unwrap(),
        None,
        "cross-tenant access must be none"
    );

    // Listing is newest-first by created_at and honors the limit cap, exercised
    // through the repository over the in-memory `RootFilesystem`.
    let mut newer = ProjectRecord::new(other_tenant.clone(), owner.clone(), "Newer", "").unwrap();
    newer.created_at = other_created + chrono::Duration::seconds(10);
    let newer_id = newer.project_id.clone();
    repo.create_project(newer).await.unwrap();
    let listed = repo
        .list_projects_for_user(&other_tenant, &owner, 10)
        .await
        .unwrap();
    assert_eq!(listed.len(), 2, "owner sees both other-tenant projects");
    assert_eq!(listed[0].project_id, newer_id, "newest project first");
    let capped = repo
        .list_projects_for_user(&other_tenant, &owner, 1)
        .await
        .unwrap();
    assert_eq!(capped.len(), 1, "limit cap is honored");
    assert_eq!(capped[0].project_id, newer_id);

    // Oversized metadata is rejected by every backend.
    let mut huge = ProjectRecord::new(other_tenant.clone(), owner.clone(), "Huge", "").unwrap();
    huge.metadata = serde_json::json!({ "blob": "x".repeat(MAX_PROJECT_METADATA_BYTES) });
    assert!(
        matches!(
            repo.create_project(huge).await,
            Err(ironclaw_projects::ProjectError::InvalidRecord { .. })
        ),
        "oversized metadata must be rejected"
    );

    // Update of a missing project is NotFound.
    let ghost = ProjectRecord::new(tenant(), owner.clone(), "Ghost", "").unwrap();
    assert!(matches!(
        repo.update_project(ghost).await,
        Err(ironclaw_projects::ProjectError::NotFound)
    ));

    // Delete removes the project and its membership.
    let removed = repo
        .delete_project(&tenant(), &project_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(removed.project_id, project_id);
    assert!(
        repo.get_project(&tenant(), &project_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repo.delete_project(&tenant(), &project_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn filesystem_satisfies_contract() {
    let repo = filesystem_repo();
    run_contract(&repo).await;
}

/// `FilesystemProjectRepository` over an in-memory `RootFilesystem`, with the
/// control-plane `/tenant-shared` mount the durable composition wiring uses.
fn filesystem_repo() -> FilesystemProjectRepository<InMemoryBackend> {
    let root = Arc::new(InMemoryBackend::default());
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/tenant-shared").unwrap(),
        VirtualPath::new("/tenants/host/shared").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(root, view));
    FilesystemProjectRepository::new(
        filesystem,
        UserId::new("user-host").unwrap(),
        AgentId::new("agent-host").unwrap(),
    )
}

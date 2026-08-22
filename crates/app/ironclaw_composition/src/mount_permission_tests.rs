use std::sync::Arc;

use ironclaw_filesystem::{FilesystemError, InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    ids::{AgentId, InvocationId, MissionId, ProjectId, TenantId, ThreadId, UserId},
    path::ScopedPath,
    resource::ResourceScope,
};

use crate::{invocation_mount_view, wrap_process_journal_scoped};

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-1").unwrap(),
        agent_id: Some(AgentId::new("agent-x").unwrap()),
        project_id: Some(ProjectId::new("project-y").unwrap()),
        mission_id: Some(MissionId::new("mission-w").unwrap()),
        thread_id: Some(ThreadId::new("thread-z").unwrap()),
        invocation_id: InvocationId::new(),
    }
}

#[test]
fn invocation_mount_view_denies_suggestion_deletion() {
    let view = invocation_mount_view(&sample_scope()).unwrap();
    let (_, grant) = view
        .resolve_with_grant(&ScopedPath::new("/suggestions/doc.json").unwrap())
        .unwrap();
    assert!(grant.permissions.read);
    assert!(grant.permissions.write);
    assert!(grant.permissions.list);
    assert!(!grant.permissions.delete);
}

#[tokio::test]
async fn invocation_mount_view_denies_notification_deletion() {
    let filesystem = ScopedFilesystem::new(Arc::new(InMemoryBackend::new()), invocation_mount_view);
    let error = filesystem
        .delete(
            &sample_scope(),
            &ScopedPath::new("/notifications/inbox.json").unwrap(),
        )
        .await
        .expect_err("notification records are retained through typed lifecycle operations");
    assert!(matches!(error, FilesystemError::PermissionDenied { .. }));
}

#[tokio::test]
async fn process_journal_migration_mount_is_system_only_and_read_only() {
    let root = Arc::new(InMemoryBackend::new());
    let scoped = wrap_process_journal_scoped(root);
    let legacy =
        ScopedPath::new("/legacy-tenants/tenant-a/users/user-a/run-state").expect("legacy path");
    assert!(
        scoped.resolve(&sample_scope(), &legacy).is_err(),
        "ordinary user scopes must not enumerate other tenant roots"
    );
    assert_eq!(
        scoped
            .resolve(&ResourceScope::system(), &legacy)
            .expect("system migration mount")
            .as_str(),
        "/tenants/tenant-a/users/user-a/run-state"
    );
    assert!(matches!(
        scoped
            .write_bytes(&ResourceScope::system(), &legacy, b"forbidden".to_vec())
            .await
            .expect_err("migration mount must not mutate legacy authorities"),
        FilesystemError::PermissionDenied { .. }
    ));
}

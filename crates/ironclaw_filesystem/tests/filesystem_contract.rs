use std::sync::Arc;

use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use tempfile::tempdir;

#[tokio::test]
async fn scoped_read_resolves_mount_view_and_reads_bytes() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(
        storage.path().join("project1/README.md"),
        b"hello filesystem",
    )
    .unwrap();

    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let scoped = ScopedFilesystem::new(
        Arc::new(root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/project1").unwrap(),
            MountPermissions::read_only(),
        )])
        .unwrap(),
    );

    let bytes = scoped
        .read_file(&ScopedPath::new("/workspace/README.md").unwrap())
        .await
        .unwrap();

    assert_eq!(bytes, b"hello filesystem");
}

#[tokio::test]
async fn scoped_write_is_denied_on_read_only_mount() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();

    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let scoped = ScopedFilesystem::new(
        Arc::new(root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/project1").unwrap(),
            MountPermissions::read_only(),
        )])
        .unwrap(),
    );

    let err = scoped
        .write_file(
            &ScopedPath::new("/workspace/generated.txt").unwrap(),
            b"nope",
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::WriteFile,
            ..
        }
    ));
    assert!(!storage.path().join("project1/generated.txt").exists());
}

#[tokio::test]
async fn scoped_append_requires_write_permission_and_appends_bytes() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/log.jsonl"), b"one\n").unwrap();

    let read_only = scoped_project_fs(storage.path(), MountPermissions::read_only());
    let err = read_only
        .append_file(
            &ScopedPath::new("/workspace/log.jsonl").unwrap(),
            b"denied\n",
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::AppendFile,
            ..
        }
    ));

    let writable = scoped_project_fs(storage.path(), MountPermissions::read_write());
    writable
        .append_file(&ScopedPath::new("/workspace/log.jsonl").unwrap(), b"two\n")
        .await
        .unwrap();

    assert_eq!(
        std::fs::read(storage.path().join("project1/log.jsonl")).unwrap(),
        b"one\ntwo\n"
    );
}

#[tokio::test]
async fn scoped_delete_requires_delete_permission_and_removes_file() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/generated.txt"), b"delete me").unwrap();

    let no_delete = scoped_project_fs(storage.path(), MountPermissions::read_write());
    let err = no_delete
        .delete(&ScopedPath::new("/workspace/generated.txt").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::Delete,
            ..
        }
    ));
    assert!(storage.path().join("project1/generated.txt").exists());

    let can_delete = scoped_project_fs(
        storage.path(),
        MountPermissions {
            read: true,
            write: true,
            delete: true,
            list: true,
            execute: false,
        },
    );
    can_delete
        .delete(&ScopedPath::new("/workspace/generated.txt").unwrap())
        .await
        .unwrap();

    assert!(!storage.path().join("project1/generated.txt").exists());

    let err = can_delete
        .delete(&ScopedPath::new("/workspace/generated.txt").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::NotFound {
            operation: FilesystemOperation::Delete,
            ..
        }
    ));
}

#[tokio::test]
async fn scoped_create_dir_all_requires_write_permission() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();

    let read_only = scoped_project_fs(storage.path(), MountPermissions::read_only());
    let err = read_only
        .create_dir_all(&ScopedPath::new("/workspace/generated/deep").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::CreateDirAll,
            ..
        }
    ));

    let writable = scoped_project_fs(storage.path(), MountPermissions::read_write());
    writable
        .create_dir_all(&ScopedPath::new("/workspace/generated/deep").unwrap())
        .await
        .unwrap();

    assert!(storage.path().join("project1/generated/deep").is_dir());
}

#[tokio::test]
async fn list_requires_list_permission_through_scoped_api() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1/src")).unwrap();

    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let scoped = ScopedFilesystem::new(
        Arc::new(root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/project1").unwrap(),
            MountPermissions {
                read: true,
                write: false,
                delete: false,
                list: false,
                execute: false,
            },
        )])
        .unwrap(),
    );

    let err = scoped
        .list_dir(&ScopedPath::new("/workspace").unwrap())
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::ListDir,
            ..
        }
    ));
}

#[tokio::test]
async fn longest_backend_virtual_mount_wins() {
    let broad = tempdir().unwrap();
    let narrow = tempdir().unwrap();
    std::fs::create_dir_all(broad.path().join("project1")).unwrap();
    std::fs::write(broad.path().join("project1/value.txt"), b"broad").unwrap();
    std::fs::write(narrow.path().join("value.txt"), b"narrow").unwrap();

    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(broad.path().to_path_buf()),
    )
    .unwrap();
    root.mount_local(
        VirtualPath::new("/projects/project1").unwrap(),
        HostPath::from_path_buf(narrow.path().to_path_buf()),
    )
    .unwrap();

    let bytes = root
        .read_file(&VirtualPath::new("/projects/project1/value.txt").unwrap())
        .await
        .unwrap();

    assert_eq!(bytes, b"narrow");
}

#[tokio::test]
async fn unknown_scoped_alias_fails_closed_through_filesystem_api() {
    let storage = tempdir().unwrap();
    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let scoped = ScopedFilesystem::new(Arc::new(root), MountView::new(Vec::new()).unwrap());
    let err = scoped
        .read_file(&ScopedPath::new("/memory/facts.md").unwrap())
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::Contract(_)));
}

#[tokio::test]
async fn artifact_write_is_confined_to_approved_virtual_mount() {
    let artifacts = tempdir().unwrap();

    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/engine/tmp/invocations/inv1/artifacts").unwrap(),
        HostPath::from_path_buf(artifacts.path().to_path_buf()),
    )
    .unwrap();

    let scoped = ScopedFilesystem::new(
        Arc::new(root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/artifacts").unwrap(),
            VirtualPath::new("/engine/tmp/invocations/inv1/artifacts").unwrap(),
            MountPermissions::read_write(),
        )])
        .unwrap(),
    );

    scoped
        .write_file(&ScopedPath::new("/artifacts/result.json").unwrap(), b"{}")
        .await
        .unwrap();

    assert_eq!(
        std::fs::read(artifacts.path().join("result.json")).unwrap(),
        b"{}"
    );
}

#[tokio::test]
async fn display_errors_do_not_leak_raw_host_paths() {
    let storage = tempdir().unwrap();
    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let err = root
        .read_file(&VirtualPath::new("/projects/missing.txt").unwrap())
        .await
        .unwrap_err();

    let display = err.to_string();
    assert!(display.contains("/projects/missing.txt"));
    assert!(!display.contains("VirtualPath("));
    assert!(!display.contains(&storage.path().display().to_string()));
}

#[cfg(unix)]
#[tokio::test]
async fn local_backend_denies_symlink_escape() {
    use std::os::unix::fs::symlink;

    let storage = tempdir().unwrap();
    let outside = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(outside.path().join("secret.txt"), b"secret").unwrap();
    symlink(
        outside.path().join("secret.txt"),
        storage.path().join("project1/escape.txt"),
    )
    .unwrap();

    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let scoped = ScopedFilesystem::new(
        Arc::new(root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/project1").unwrap(),
            MountPermissions::read_only(),
        )])
        .unwrap(),
    );

    let err = scoped
        .read_file(&ScopedPath::new("/workspace/escape.txt").unwrap())
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::SymlinkEscape { .. }));
}

#[tokio::test]
async fn read_requires_read_permission_through_scoped_api() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/secret.txt"), b"secret").unwrap();

    let scoped = scoped_project_fs(
        storage.path(),
        MountPermissions {
            read: false,
            write: true,
            delete: false,
            list: true,
            execute: false,
        },
    );

    let err = scoped
        .read_file(&ScopedPath::new("/workspace/secret.txt").unwrap())
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::ReadFile,
            ..
        }
    ));
}

#[tokio::test]
async fn stat_is_allowed_by_read_or_list_and_denied_without_both() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/file.txt"), b"abc").unwrap();

    let read_only = scoped_project_fs(
        storage.path(),
        MountPermissions {
            read: true,
            write: false,
            delete: false,
            list: false,
            execute: false,
        },
    );
    let stat = read_only
        .stat(&ScopedPath::new("/workspace/file.txt").unwrap())
        .await
        .unwrap();
    assert_eq!(stat.len, 3);

    let list_only = scoped_project_fs(
        storage.path(),
        MountPermissions {
            read: false,
            write: false,
            delete: false,
            list: true,
            execute: false,
        },
    );
    let stat = list_only
        .stat(&ScopedPath::new("/workspace/file.txt").unwrap())
        .await
        .unwrap();
    assert_eq!(stat.file_type, FileType::File);

    let no_stat = scoped_project_fs(storage.path(), MountPermissions::none());
    let err = no_stat
        .stat(&ScopedPath::new("/workspace/file.txt").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::Stat,
            ..
        }
    ));
}

#[tokio::test]
async fn list_success_returns_sorted_entries_with_virtual_paths() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/zeta.txt"), b"z").unwrap();
    std::fs::write(storage.path().join("project1/alpha.txt"), b"a").unwrap();

    let root = local_root_with_projects_mount(storage.path());
    let entries = root
        .list_dir(&VirtualPath::new("/projects/project1").unwrap())
        .await
        .unwrap();

    let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, vec!["alpha.txt", "zeta.txt"]);
    let paths: Vec<_> = entries.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "/projects/project1/alpha.txt",
            "/projects/project1/zeta.txt"
        ]
    );
}

#[tokio::test]
async fn workspace_write_creates_parent_directories() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();

    let scoped = scoped_project_fs(storage.path(), MountPermissions::read_write());
    scoped
        .write_file(
            &ScopedPath::new("/workspace/generated/deep/file.txt").unwrap(),
            b"created",
        )
        .await
        .unwrap();

    assert_eq!(
        std::fs::read(storage.path().join("project1/generated/deep/file.txt")).unwrap(),
        b"created"
    );
}

#[tokio::test]
async fn duplicate_backend_mount_is_rejected() {
    let storage = tempdir().unwrap();
    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let err = root
        .mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .unwrap_err();

    assert!(matches!(err, FilesystemError::MountConflict { .. }));
}

#[tokio::test]
async fn nonexistent_backend_mount_root_fails_without_leaking_host_path() {
    let storage = tempdir().unwrap();
    let missing = storage.path().join("missing-root");
    let mut root = LocalFilesystem::new();

    let err = root
        .mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(missing.clone()),
        )
        .unwrap_err();

    let display = err.to_string();
    assert!(display.contains("/projects"));
    assert!(!display.contains(&missing.display().to_string()));
}

#[test]
fn invalid_scoped_paths_are_rejected_before_filesystem_access() {
    for invalid in [
        "/workspace/../secret.txt",
        "file:///etc/passwd",
        "https://example.com/file",
        "/Users/alice/project/secret.txt",
        "C:\\Users\\alice\\project\\secret.txt",
        "/workspace/has\0nul",
    ] {
        assert!(
            ScopedPath::new(invalid).is_err(),
            "{invalid:?} should be rejected before filesystem access"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn local_backend_denies_write_through_symlink_escape() {
    use std::os::unix::fs::symlink;

    let storage = tempdir().unwrap();
    let outside = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(outside.path().join("secret.txt"), b"original").unwrap();
    symlink(
        outside.path().join("secret.txt"),
        storage.path().join("project1/escape.txt"),
    )
    .unwrap();

    let scoped = scoped_project_fs(storage.path(), MountPermissions::read_write());
    let err = scoped
        .write_file(
            &ScopedPath::new("/workspace/escape.txt").unwrap(),
            b"changed",
        )
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::SymlinkEscape { .. }));
    assert_eq!(
        std::fs::read(outside.path().join("secret.txt")).unwrap(),
        b"original"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn local_backend_denies_write_through_symlinked_parent_escape() {
    use std::os::unix::fs::symlink;

    let storage = tempdir().unwrap();
    let outside = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    symlink(outside.path(), storage.path().join("project1/outside-dir")).unwrap();

    let scoped = scoped_project_fs(storage.path(), MountPermissions::read_write());
    let err = scoped
        .write_file(
            &ScopedPath::new("/workspace/outside-dir/new.txt").unwrap(),
            b"escaped",
        )
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::SymlinkEscape { .. }));
    assert!(!outside.path().join("new.txt").exists());
}

fn local_root_with_projects_mount(path: &std::path::Path) -> LocalFilesystem {
    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(path.to_path_buf()),
    )
    .unwrap();
    root
}

fn scoped_project_fs(
    path: &std::path::Path,
    permissions: MountPermissions,
) -> ScopedFilesystem<LocalFilesystem> {
    ScopedFilesystem::new(
        Arc::new(local_root_with_projects_mount(path)),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/project1").unwrap(),
            permissions,
        )])
        .unwrap(),
    )
}

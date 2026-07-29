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

    let mut root = DiskFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let scoped = ScopedFilesystem::with_fixed_view(
        Arc::new(root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/project1").unwrap(),
            MountPermissions::read_only(),
        )])
        .unwrap(),
    );

    let bytes = scoped
        .read_file(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/README.md").unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(bytes, b"hello filesystem");
}

#[tokio::test]
async fn bounded_read_returns_none_without_materializing_oversized_local_file() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/schema.json"), b"abcdef").unwrap();

    let mut root = DiskFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let path = VirtualPath::new("/projects/project1/schema.json").unwrap();

    assert_eq!(
        root.read_file_bounded(&path, 6).await.unwrap(),
        Some(b"abcdef".to_vec())
    );
    assert_eq!(root.read_file_bounded(&path, 5).await.unwrap(), None);
}

#[tokio::test]
async fn local_put_absent_rejects_existing_file_without_overwrite() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();

    let mut root = DiskFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    let path = VirtualPath::new("/projects/project1/checkpoint.json").unwrap();

    root.put(
        &path,
        Entry::bytes(b"first".to_vec()),
        CasExpectation::Absent,
    )
    .await
    .unwrap();
    let err = root
        .put(
            &path,
            Entry::bytes(b"second".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::VersionMismatch { .. }));
    assert_eq!(
        std::fs::read(storage.path().join("project1/checkpoint.json")).unwrap(),
        b"first"
    );
}

#[tokio::test]
async fn scoped_write_is_denied_on_read_only_mount() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();

    let mut root = DiskFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let scoped = ScopedFilesystem::with_fixed_view(
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
            &ResourceScope::system(),
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
            &ResourceScope::system(),
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
        .append_file(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/log.jsonl").unwrap(),
            b"two\n",
        )
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
        .delete(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/generated.txt").unwrap(),
        )
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
        .delete(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/generated.txt").unwrap(),
        )
        .await
        .unwrap();

    assert!(!storage.path().join("project1/generated.txt").exists());

    let err = can_delete
        .delete(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/generated.txt").unwrap(),
        )
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
        .create_dir_all(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/generated/deep").unwrap(),
        )
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
        .create_dir_all(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/generated/deep").unwrap(),
        )
        .await
        .unwrap();

    assert!(storage.path().join("project1/generated/deep").is_dir());
}

#[tokio::test]
async fn list_requires_list_permission_through_scoped_api() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1/src")).unwrap();

    let mut root = DiskFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let scoped = ScopedFilesystem::with_fixed_view(
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
        .list_dir(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace").unwrap(),
        )
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

    let mut root = DiskFilesystem::new();
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
    let mut root = DiskFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let scoped =
        ScopedFilesystem::with_fixed_view(Arc::new(root), MountView::new(Vec::new()).unwrap());
    let err = scoped
        .read_file(
            &ResourceScope::system(),
            &ScopedPath::new("/memory/facts.md").unwrap(),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::Contract(_)));
}

#[tokio::test]
async fn artifact_write_is_confined_to_approved_virtual_mount() {
    let artifacts = tempdir().unwrap();

    let mut root = DiskFilesystem::new();
    root.mount_local(
        VirtualPath::new("/engine/tmp/invocations/inv1/artifacts").unwrap(),
        HostPath::from_path_buf(artifacts.path().to_path_buf()),
    )
    .unwrap();

    let scoped = ScopedFilesystem::with_fixed_view(
        Arc::new(root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/artifacts").unwrap(),
            VirtualPath::new("/engine/tmp/invocations/inv1/artifacts").unwrap(),
            MountPermissions::read_write(),
        )])
        .unwrap(),
    );

    scoped
        .write_file(
            &ResourceScope::system(),
            &ScopedPath::new("/artifacts/result.json").unwrap(),
            b"{}",
        )
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
    let mut root = DiskFilesystem::new();
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

    let mut root = DiskFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let scoped = ScopedFilesystem::with_fixed_view(
        Arc::new(root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/project1").unwrap(),
            MountPermissions::read_only(),
        )])
        .unwrap(),
    );

    let err = scoped
        .read_file(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/escape.txt").unwrap(),
        )
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
        .read_file(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/secret.txt").unwrap(),
        )
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
        .stat(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/file.txt").unwrap(),
        )
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
        .stat(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/file.txt").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stat.file_type, FileType::File);

    let no_stat = scoped_project_fs(storage.path(), MountPermissions::none());
    let err = no_stat
        .stat(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/file.txt").unwrap(),
        )
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
            &ResourceScope::system(),
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
    let mut root = DiskFilesystem::new();
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
    let mut root = DiskFilesystem::new();

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

#[tokio::test]
async fn local_list_dir_bounded_returns_at_most_max_entries() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(storage.path().join("project1").join(name), b"entry").unwrap();
    }
    let root = local_root_with_projects_mount(storage.path());

    let entries = root
        .list_dir_bounded(&VirtualPath::new("/projects/project1").unwrap(), 2)
        .await
        .unwrap();

    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .all(|entry| entry.path.as_str().starts_with("/projects/project1/"))
    );
}

#[tokio::test]
async fn local_list_dir_bounded_propagates_read_dir_errors() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/not-a-dir.txt"), b"entry").unwrap();
    let root = local_root_with_projects_mount(storage.path());

    let err = root
        .list_dir_bounded(
            &VirtualPath::new("/projects/project1/not-a-dir.txt").unwrap(),
            1,
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        FilesystemError::Backend {
            operation: FilesystemOperation::ListDir,
            ..
        }
    ));
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
            &ResourceScope::system(),
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
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/outside-dir/new.txt").unwrap(),
            b"escaped",
        )
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::SymlinkEscape { .. }));
    assert!(!outside.path().join("new.txt").exists());
}

/// Policy change (fd-rooted symlink-follow, #6723 follow-up): `RESOLVE_BENEATH`
/// alone is already race-free and kernel-enforced — it follows an in-bounds
/// symlink and refuses an escaping one, atomically. Rejecting *every*
/// symlink (the old behavior this test used to pin) breaks real projects
/// full of benign in-bounds symlinks (pnpm `node_modules`, `git worktree`
/// `.git` links, monorepo aliases) for no containment benefit. This test now
/// pins the inverted contract: a write through a benign, fully in-bounds
/// symlink *resolves* — the bytes land at the symlink's target, not over the
/// symlink entry itself (`rename`/`link` never follow a symlink at the
/// destination name, so `write_file_with_cas` chases the chain itself first
/// via `resolve_write_leaf` — see `local/fd_resolve.rs`). The escape case
/// (`local_backend_denies_write_through_symlink_escape`, above) is unchanged
/// and still rejected — this test's job is to prove the in-bounds case is no
/// longer collateral damage from that rejection, not to weaken it.
#[cfg(unix)]
#[tokio::test]
async fn local_backend_resolves_write_through_benign_in_bounds_symlink() {
    use std::os::unix::fs::symlink;

    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/real.txt"), b"original").unwrap();
    // In-bounds and relative — the realistic shape a project's own symlinks
    // take (pnpm, git worktree, monorepo aliases all emit relative targets).
    symlink("real.txt", storage.path().join("project1/alias.txt")).unwrap();

    let scoped = scoped_project_fs(storage.path(), MountPermissions::read_write());
    scoped
        .write_file(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/alias.txt").unwrap(),
            b"changed",
        )
        .await
        .expect("write through an in-bounds symlink must resolve");

    assert_eq!(
        std::fs::read(storage.path().join("project1/real.txt")).unwrap(),
        b"changed",
        "the write must land at the symlink's real target"
    );
    assert!(
        std::fs::symlink_metadata(storage.path().join("project1/alias.txt"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "the symlink entry itself must survive the write, not be replaced by a plain file"
    );
}

/// Read counterpart to the write-through test above: `read_file` through an
/// in-bounds symlink must resolve to the target's bytes.
#[cfg(unix)]
#[tokio::test]
async fn local_backend_resolves_read_through_benign_in_bounds_symlink() {
    use std::os::unix::fs::symlink;

    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/real.txt"), b"hello").unwrap();
    symlink("real.txt", storage.path().join("project1/alias.txt")).unwrap();

    let scoped = scoped_project_fs(storage.path(), MountPermissions::read_only());
    let bytes = scoped
        .read_file(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/alias.txt").unwrap(),
        )
        .await
        .expect("read through an in-bounds symlink must resolve");

    assert_eq!(bytes, b"hello");
}

/// A relative in-bounds symlink chain (`link -> mid -> real.txt`, each hop
/// relative) must resolve all the way through.
#[cfg(unix)]
#[tokio::test]
async fn local_backend_resolves_read_through_relative_symlink_chain() {
    use std::os::unix::fs::symlink;

    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/real.txt"), b"chained").unwrap();
    symlink("real.txt", storage.path().join("project1/mid")).unwrap();
    symlink("mid", storage.path().join("project1/link")).unwrap();

    let scoped = scoped_project_fs(storage.path(), MountPermissions::read_only());
    let bytes = scoped
        .read_file(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/link").unwrap(),
        )
        .await
        .expect("a relative in-bounds symlink chain must resolve");

    assert_eq!(bytes, b"chained");
}

/// PR #6817 follow-up finding (Task 1): a real pnpm `node_modules` layout
/// places a *parent-relative* symlink — `node_modules/@types/react ->
/// ../.pnpm/react@x.y.z/node_modules/react` — one directory level up from
/// where the symlink itself lives, then back down into a sibling subtree.
/// The target is still fully inside the mount root throughout. A resolver
/// whose containment boundary for the fast path is the symlink's *immediate
/// parent* directory (rather than the true mount root) rejects this: `..`
/// steps above that immediate parent, so a bare `RESOLVE_BENEATH`-style
/// check on the immediate parent alone reports "escape" even though the
/// fully-resolved target never leaves the mount. This test pins that a
/// parent-relative, fully in-bounds symlink chain — including one whose
/// target's directory doesn't exist yet as a distinct virtual path component
/// until the symlink is followed — resolves correctly for both read and
/// write, on every platform this crate ships on (this is the general,
/// cross-platform regression pin; `local_backend_linux_openat2_...` below is
/// the Linux-specific execution evidence for why the fast path needed to
/// change to make this true).
#[cfg(unix)]
#[tokio::test]
async fn local_backend_resolves_parent_relative_symlink_pnpm_layout() {
    use std::os::unix::fs::symlink;

    let storage = tempdir().unwrap();
    let project = storage.path().join("project1");
    // node_modules/.pnpm/react@1.0.0/node_modules/react/index.js  (the real file)
    let pnpm_real_dir = project.join("node_modules/.pnpm/react@1.0.0/node_modules/react");
    std::fs::create_dir_all(&pnpm_real_dir).unwrap();
    std::fs::write(pnpm_real_dir.join("index.js"), b"module.exports = {}").unwrap();
    // node_modules/@types  (the symlink's own directory)
    let types_dir = project.join("node_modules/@types");
    std::fs::create_dir_all(&types_dir).unwrap();
    // node_modules/@types/react -> ../.pnpm/react@1.0.0/node_modules/react
    // ("..": step out of @types back into node_modules, then descend).
    symlink(
        "../.pnpm/react@1.0.0/node_modules/react",
        types_dir.join("react"),
    )
    .unwrap();

    let scoped = scoped_project_fs(storage.path(), MountPermissions::read_write());
    let bytes = scoped
        .read_file(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/node_modules/@types/react/index.js").unwrap(),
        )
        .await
        .expect(
            "a parent-relative, fully in-bounds symlink (the real pnpm node_modules shape) \
             must resolve, not be rejected as an escape",
        );
    assert_eq!(bytes, b"module.exports = {}");

    // PR #6817 review follow-up (discussion_r3669792305 / r3670294887): the
    // write side goes through a different resolver (`descend_creating` ->
    // `resolve_write_leaf`, not `resolve_walk`) than the read assertion
    // above, so it needs its own coverage rather than inheriting the read
    // path's pass — `descend_creating` used to discard the ancestor stack it
    // built, which made this identical parent-relative symlink fail closed
    // as `SymlinkEscape` on write even though the read resolved it fine
    // (fixed in 234c1c82e: `descend_creating` now returns its ancestor stack
    // and `write_file_with_cas` threads it into `resolve_write_leaf`).
    scoped
        .write_file(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/node_modules/@types/react/index.js").unwrap(),
            b"module.exports = { updated: true }",
        )
        .await
        .expect(
            "writing through the same in-bounds parent-relative symlink must succeed, not be \
             rejected as an escape",
        );
    assert_eq!(
        std::fs::read(pnpm_real_dir.join("index.js")).unwrap(),
        b"module.exports = { updated: true }",
        "the write must land at the symlink's real target, not create a shadow file at the \
         symlink's own path"
    );
}

/// An absolute symlink target is rejected outright (`Escape`), unconditionally
/// — even when, interpreted as real bytes, it happens to land inside the
/// mount. This deliberately matches Linux's native
/// `openat2(RESOLVE_BENEATH)`, which disallows *any* absolute symlink target
/// regardless of where it points (only `RESOLVE_IN_ROOT` — a different,
/// chroot-emulating primitive this module does not use — reinterprets one).
/// See the `walk_symlink_target` doc comment in `local/fd_resolve.rs` for
/// the full reasoning, including why a "reinterpret against the mount root"
/// scheme would not even help for symlinks any real tool creates (they store
/// the real host absolute path, which essentially never coincides with the
/// mount root by construction).
#[cfg(unix)]
#[tokio::test]
async fn local_backend_denies_absolute_symlink_even_when_bytes_would_land_in_mount() {
    use std::os::unix::fs::symlink;

    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/real.txt"), b"original").unwrap();
    symlink(
        storage.path().join("project1/real.txt"),
        storage.path().join("project1/absolute-alias.txt"),
    )
    .unwrap();

    let scoped = scoped_project_fs(storage.path(), MountPermissions::read_only());
    let err = scoped
        .read_file(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/absolute-alias.txt").unwrap(),
        )
        .await
        .unwrap_err();

    assert!(
        matches!(err, FilesystemError::SymlinkEscape { .. }),
        "expected SymlinkEscape, got: {err:?}"
    );
    // PR #6817 review follow-up: this rejection must be *actionable*, not
    // just correct — a `local-dev-yolo` user hitting this on a real pyenv/
    // chezmoi/cloud-sync symlink under their mounted home directory needs
    // to know which symlink and what target caused it, and that absolute
    // targets specifically (not symlinks in general) are unsupported, so
    // they can fix it (replace with a relative link) instead of guessing.
    let message = err.to_string();
    assert!(
        message.contains("absolute-alias.txt"),
        "error message must name the offending symlink, got: {message:?}"
    );
    assert!(
        message.contains("real.txt"),
        "error message must show the symlink's absolute target, got: {message:?}"
    );
    assert!(
        message.contains("absolute") && message.contains("not supported"),
        "error message must state that absolute symlink targets are unsupported, \
         got: {message:?}"
    );
}

/// A symlink chain within the depth cap (`MAX_SYMLINK_DEPTH = 32`) still
/// resolves; one exceeding it fails cleanly instead of spinning. Builds a
/// chain of 20 relative hops (`link19 -> link18 -> … -> real.txt`), well
/// under the cap, then a cyclic pair (`cycle-a -> cycle-b -> cycle-a`) that
/// can never terminate on its own.
#[cfg(unix)]
#[tokio::test]
async fn local_backend_resolves_symlink_chain_within_cap_and_fails_cycle_cleanly() {
    use std::os::unix::fs::symlink;

    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::write(storage.path().join("project1/real.txt"), b"end of chain").unwrap();

    const CHAIN_LEN: usize = 20;
    let mut previous = "real.txt".to_string();
    for i in 0..CHAIN_LEN {
        let name = format!("link{i}");
        symlink(&previous, storage.path().join("project1").join(&name)).unwrap();
        previous = name;
    }
    symlink("cycle-b", storage.path().join("project1/cycle-a")).unwrap();
    symlink("cycle-a", storage.path().join("project1/cycle-b")).unwrap();

    let scoped = scoped_project_fs(storage.path(), MountPermissions::read_only());

    let bytes = scoped
        .read_file(
            &ResourceScope::system(),
            &ScopedPath::new(format!("/workspace/{previous}")).unwrap(),
        )
        .await
        .expect("a symlink chain within the depth cap must resolve");
    assert_eq!(bytes, b"end of chain");

    let cycle_err = scoped
        .read_file(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/cycle-a").unwrap(),
        )
        .await
        .unwrap_err();
    assert!(
        !matches!(cycle_err, FilesystemError::NotFound { .. }),
        "a genuine symlink cycle must fail cleanly (bounded), not report NotFound: {cycle_err:?}"
    );
}

/// FIX 3 regression: the symlink-hop budget is per-*resolution*, not
/// per-*component*. Builds a 4-component path where each individual
/// component resolves through its own 10-hop symlink chain (well under
/// `MAX_SYMLINK_DEPTH = 32` on its own), but the combined total across all
/// four components is 40 hops — over the cap.
///
/// Before this fix, `open_one` reset to a fresh `MAX_SYMLINK_DEPTH`-hop
/// budget for every component `resolve_walk` called it on, so a 10-hop
/// chain at each of 4 components would resolve without ever tripping the
/// cap (40 total hops, but never more than 10 in any single component's own
/// budget) — looser than the real `openat2(RESOLVE_BENEATH)` kernel fast
/// path, which applies its cap once per whole resolution. After this fix, a
/// single `SymlinkBudget` is shared across the whole `resolve_walk` walk, so
/// this same path must fail closed once cumulative hops exceed the cap,
/// matching `SYMLOOP_MAX` semantics.
#[cfg(unix)]
#[tokio::test]
async fn local_backend_rejects_path_whose_combined_per_component_symlink_chains_exceed_global_budget()
 {
    use std::os::unix::fs::symlink;

    let storage = tempdir().unwrap();
    const LEVELS: usize = 4;
    const HOPS_PER_LEVEL: usize = 10;

    // Builds a `HOPS_PER_LEVEL`-hop symlink chain inside `dir` terminating at
    // a real subdirectory of `dir`, and returns (the chain's outermost link
    // name, the real subdirectory's path) — mirroring the chain-building
    // pattern `local_backend_resolves_symlink_chain_within_cap_and_fails_cycle_cleanly`
    // uses, just nested one level deeper per call.
    fn build_chain_level(dir: &std::path::Path, level: usize) -> (String, std::path::PathBuf) {
        let real_name = format!("level{level}_real");
        let real_dir = dir.join(&real_name);
        std::fs::create_dir_all(&real_dir).unwrap();
        let mut previous = real_name;
        for hop in 0..HOPS_PER_LEVEL {
            let name = format!("level{level}_link{hop}");
            symlink(&previous, dir.join(&name)).unwrap();
            previous = name;
        }
        (previous, real_dir)
    }

    let mut current_dir = storage.path().to_path_buf();
    let mut path_components = Vec::new();
    for level in 0..LEVELS {
        let (chain_head, real_dir) = build_chain_level(&current_dir, level);
        path_components.push(chain_head);
        current_dir = real_dir;
    }
    std::fs::write(current_dir.join("file.txt"), b"deep file").unwrap();
    path_components.push("file.txt".to_string());

    let root = local_root_with_projects_mount(storage.path());
    let virtual_path =
        VirtualPath::new(format!("/projects/{}", path_components.join("/"))).unwrap();

    let error = root.read_file(&virtual_path).await.unwrap_err();
    assert!(
        matches!(error, FilesystemError::SymlinkEscape { .. }),
        "a path whose combined per-component symlink chains exceed the \
         global MAX_SYMLINK_DEPTH budget must fail closed as SymlinkEscape, \
         got: {error:?}"
    );
}

/// Deleting the bare root of an *ordinary* mount (no leaf segment, no
/// sub-path) must fail closed rather than silently deleting the mount's
/// entire on-disk tree. `DiskFilesystem::delete` (`local.rs:474-485`) has no
/// fd-relative parent for the mount root itself by design — `resolve_walk`
/// only ever hands back `parent: None` for an empty component list — so this
/// pins that the bare-root case is rejected with `PathOutsideMount`, not
/// merely "would panic on the `None` parent" or (worse) silently succeed via
/// some other code path.
#[tokio::test]
async fn local_backend_delete_of_bare_ordinary_mount_root_fails_closed() {
    let storage = tempdir().unwrap();
    std::fs::write(storage.path().join("keep-me.txt"), b"still here").unwrap();

    let root = local_root_with_projects_mount(storage.path());

    let err = root
        .delete(&VirtualPath::new("/projects").unwrap())
        .await
        .unwrap_err();

    assert!(
        matches!(err, FilesystemError::PathOutsideMount { .. }),
        "expected PathOutsideMount, got: {err:?}"
    );
    assert!(
        storage.path().join("keep-me.txt").exists(),
        "the mount root's contents must survive a rejected bare-root delete"
    );
}

/// Same fail-closed contract for a `mount_local_per_leaf` mount: deleting
/// the bare mount path (no leaf segment at all) is not just "unsafe for this
/// caller's leaf" but has no well-defined target — it would mean "every
/// caller's leaf" — so it must be rejected exactly like the bare-root
/// `read_file` case `leaf_scoped_mount_rejects_bare_mount_root_request`
/// already pins in `local.rs`, but for `delete`.
#[tokio::test]
async fn local_backend_delete_of_bare_leaf_scoped_mount_root_fails_closed() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("leaf-a")).unwrap();
    std::fs::write(storage.path().join("leaf-a/file.txt"), b"leaf-a data").unwrap();

    let mut root = DiskFilesystem::new();
    root.mount_local_per_leaf(
        VirtualPath::new("/tmp").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let err = root
        .delete(&VirtualPath::new("/tmp").unwrap())
        .await
        .unwrap_err();

    assert!(
        matches!(err, FilesystemError::PathOutsideMount { .. }),
        "expected PathOutsideMount, got: {err:?}"
    );
    assert!(
        storage.path().join("leaf-a/file.txt").exists(),
        "no caller's leaf may be wiped by a rejected bare-root delete"
    );
}

/// Builds a virtual path with `depth` synthetic single-directory-name
/// segments under `/projects/project1`. Deliberately built and walked
/// through the fd-rooted `DiskFilesystem` API end to end (`create_dir_all`,
/// then `write_file`) rather than via a single joined `std::fs` host path:
/// a several-thousand-character joined path string would hit the *host
/// OS's* `PATH_MAX` on a single syscall (unrelated to anything under test
/// here), whereas the fd-rooted resolver never constructs a joined path at
/// all — it walks one component per `openat` call — so it has no such
/// limit and is exactly the code path these tests exist to exercise.
fn deep_virtual_path(depth: usize, leaf: &str) -> VirtualPath {
    let mut raw = String::from("/projects/project1");
    for level in 0..depth {
        raw.push_str(&format!("/d{level}"));
    }
    raw.push('/');
    raw.push_str(leaf);
    VirtualPath::new(raw).unwrap()
}

fn deep_virtual_dir(depth: usize) -> VirtualPath {
    let mut raw = String::from("/projects/project1");
    for level in 0..depth {
        raw.push_str(&format!("/d{level}"));
    }
    VirtualPath::new(raw).unwrap()
}

/// `remove_dir_all_fd` is genuinely recursive Rust code on the blocking
/// pool; a tree deep enough would stack-overflow a naive implementation.
/// This pins two things at once: a tree comfortably within the depth cap
/// (`local.rs::MAX_REMOVE_DIR_DEPTH`, currently 512) still deletes
/// successfully end to end, and a tree that exceeds the cap fails cleanly
/// — a `Backend` error, not a crash — rather than being silently truncated
/// or panicking the blocking-pool thread.
#[tokio::test]
async fn local_backend_delete_of_deep_but_in_bounds_tree_succeeds() {
    let storage = tempdir().unwrap();
    let root = local_root_with_projects_mount(storage.path());

    // Comfortably within MAX_REMOVE_DIR_DEPTH (512).
    let deep_dir = deep_virtual_dir(300);
    let leaf = deep_virtual_path(300, "leaf.txt");
    root.create_dir_all(&deep_dir).await.unwrap();
    root.write_file(&leaf, b"deep file").await.unwrap();

    root.delete(&VirtualPath::new("/projects/project1").unwrap())
        .await
        .unwrap();

    let err = root
        .stat(&VirtualPath::new("/projects/project1").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, FilesystemError::NotFound { .. }));
}

#[tokio::test]
async fn local_backend_delete_of_tree_exceeding_max_depth_fails_cleanly() {
    let storage = tempdir().unwrap();
    let root = local_root_with_projects_mount(storage.path());

    // One level deeper than MAX_REMOVE_DIR_DEPTH (512): the deletion walk
    // must refuse to descend that far rather than overflowing the
    // blocking-pool thread's stack.
    let deep_dir = deep_virtual_dir(600);
    root.create_dir_all(&deep_dir).await.unwrap();

    let err = root
        .delete(&VirtualPath::new("/projects/project1").unwrap())
        .await
        .unwrap_err();

    assert!(
        matches!(err, FilesystemError::Backend { .. }),
        "expected a clean Backend error for a too-deep tree, got: {err:?}"
    );
}

/// PR #6817 review follow-up ("unbounded ancestor-fd retention"):
/// `resolve_walk`/`descend_creating` hold one open ancestor directory fd per
/// path component *simultaneously* for the duration of a single resolution,
/// and (before this fix) `VirtualPath`/`MountTarget.components` had no cap
/// on component count at all — every component is caller-supplied, so a
/// pathologically deep single virtual path could force this backend to hold
/// an attacker-chosen number of open fds for one request, a process-wide
/// (`RLIMIT_NOFILE`) exhaustion vector, not one scoped to the offending
/// request. This pins that a virtual path past
/// `mount_registry::MAX_PATH_COMPONENTS` fails closed with a dedicated,
/// diagnosable error *before* any fd work happens — never by silently
/// widening to a shorter prefix or truncating the walk (see the
/// constant's doc comment for why a fallback shape is exactly the wrong
/// move here, per the PR's own earlier LRU-fallback cross-tenant escape).
#[tokio::test]
async fn local_backend_rejects_path_components_past_the_ancestor_fd_cap() {
    let storage = tempdir().unwrap();
    let root = local_root_with_projects_mount(storage.path());

    // Comfortably past MAX_PATH_COMPONENTS (2048) — and, crucially, past
    // every legitimately-deep tree this suite itself builds elsewhere (the
    // deepest, `local_backend_delete_of_tree_exceeding_max_depth_fails_cleanly`,
    // goes to 600 components) — so this is unambiguously the pathological
    // case the cap exists for, not a false positive on a real deep layout.
    let too_deep = deep_virtual_dir(3000);

    let err = root.create_dir_all(&too_deep).await.unwrap_err();

    assert!(
        matches!(err, FilesystemError::PathTooDeep { .. }),
        "expected PathTooDeep for a path far past the component cap, got: {err:?}"
    );

    // Fails closed, not merely "differently": nothing must have been
    // created on disk by the rejected walk.
    assert!(
        !storage.path().join("project1/d0").exists(),
        "a PathTooDeep rejection must not partially create the tree"
    );
}

fn local_root_with_projects_mount(path: &std::path::Path) -> DiskFilesystem {
    let mut root = DiskFilesystem::new();
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
) -> ScopedFilesystem<DiskFilesystem> {
    ScopedFilesystem::with_fixed_view(
        Arc::new(local_root_with_projects_mount(path)),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/project1").unwrap(),
            permissions,
        )])
        .unwrap(),
    )
}

/// TOCTOU escape coverage for the `DiskFilesystem` local backend.
///
/// `local_backend_denies_symlink_escape` (above) and its write-path/parent
/// siblings plant a symlink *before* the call under test — they pin the
/// steady-state containment check but cannot exercise a race between that
/// check and the later syscall the checked path is handed to. These tests
/// plant a **legitimate** entry, let the resolver's containment check pass
/// against it, and then swap in a symlink-to-outside from a second OS thread
/// *while the async call under test is in flight* — reproducing the actual
/// pathname-check-then-separate-syscall gap described in
/// `crates/ironclaw_filesystem/src/local.rs`.
///
/// Per `crates/ironclaw_filesystem/CLAUDE.md`'s sanctioned pattern for tests
/// that need a read/write interleaving barrier, this is a tiny test-only
/// delegating harness (`Racer`), not a fault fake and not a production hook.
/// There is no seam inside `local.rs` to pause mid-resolution from outside,
/// so the harness drives a *real* OS-thread race: a background thread spins
/// tightly, alternating the on-disk entry between "real file/dir" (so the
/// resolver's check succeeds) and "symlink to outside the mount" for the
/// entire wall-clock duration of the call under test, which is real syscall
/// latency on tokio's blocking pool — exactly the window the production code
/// documents at `local.rs:172,204,230`. Iterating the race many times makes
/// a genuine TOCTOU gap observable without needing a deterministic pause
/// hook; a race-free implementation (this PR's fd-rooted fix) cannot lose
/// this race no matter how many iterations run, because containment is
/// re-verified against an already-open fd rather than a path string.
#[cfg(unix)]
mod toctou_escape {
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use ironclaw_filesystem::*;
    use ironclaw_host_api::*;
    use tempfile::tempdir;

    /// How many times to retry the check-then-swap race before concluding a
    /// vulnerable implementation would have leaked. Each iteration is a real
    /// (fast) filesystem call, so this bound keeps the test's wall-clock cost
    /// reasonable while giving the racer many shots at the narrow window.
    const RACE_ITERATIONS: usize = 400;

    /// Runs `attempt` under a background thread that repeatedly swaps
    /// `target` between a real entry (created by `reset`) and a symlink to
    /// `outside_target` for the duration of the call. Returns `true` the
    /// first time `attempt` reports the escape happened (via its own return
    /// value), or `false` if no iteration ever won the race.
    fn race<F>(target: &Path, outside_target: &Path, reset: impl Fn(), mut attempt: F) -> bool
    where
        F: FnMut() -> bool,
    {
        for _ in 0..RACE_ITERATIONS {
            reset();

            let stop = Arc::new(AtomicBool::new(false));
            let racer_stop = Arc::clone(&stop);
            let racer_target = target.to_path_buf();
            let racer_outside = outside_target.to_path_buf();
            let racer = std::thread::spawn(move || {
                while !racer_stop.load(Ordering::Relaxed) {
                    // Best-effort: the swap only needs to land during the
                    // window; failures (e.g. racing our own reset) are fine.
                    let _ = std::fs::remove_file(&racer_target)
                        .or_else(|_| std::fs::remove_dir_all(&racer_target));
                    let _ = symlink(&racer_outside, &racer_target);
                }
            });

            let escaped = attempt();
            stop.store(true, Ordering::Relaxed);
            let _ = racer.join();

            if escaped {
                return true;
            }
        }
        false
    }

    fn disk_fs(mount_root: &Path) -> DiskFilesystem {
        let mut root = DiskFilesystem::new();
        root.mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(mount_root.to_path_buf()),
        )
        .unwrap();
        root
    }

    /// (a) `resolve_existing`: the leaf is swapped from a real file to a
    /// symlink-to-outside between the containment check and the caller's
    /// `read`. A vulnerable resolver hands back a checked-then-stale path;
    /// the later `tokio::fs::read` re-resolves that path from scratch and
    /// follows the now-planted symlink, returning the outside secret's
    /// bytes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolve_existing_leaf_swap_leaks_outside_file_on_read() {
        let storage = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let project = storage.path().join("project1");
        std::fs::create_dir_all(&project).unwrap();
        let outside_secret = outside.path().join("secret.txt");
        std::fs::write(&outside_secret, b"outside-secret-contents").unwrap();

        let target = project.join("target.txt");
        let root = disk_fs(storage.path());
        let path = VirtualPath::new("/projects/project1/target.txt").unwrap();

        let escaped = race(
            &target,
            &outside_secret,
            || {
                let _ = std::fs::remove_file(&target);
                std::fs::write(&target, b"legit-contents").unwrap();
            },
            || {
                let root = &root;
                let path = &path;
                futures_block_on(async {
                    matches!(
                        root.read_file(path).await,
                        Ok(bytes) if bytes == b"outside-secret-contents"
                    )
                })
            },
        );

        assert!(
            !escaped,
            "resolve_existing must never hand read_file bytes from outside the mount, \
             even when the leaf is swapped to a symlink after the containment check"
        );
    }

    /// (b) `resolve_for_write`: the parent is checked and re-verified, but
    /// the *leaf* itself is never re-checked (`local.rs:177-211`) — a
    /// symlink planted at the leaf name after the parent check, before
    /// `append_file`'s unguarded `OpenOptions::open`, is followed straight
    /// through into the outside file. Distinct from (a): the ancestor chain
    /// here is legitimate throughout; only the leaf is swapped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolve_for_write_leaf_swap_leaks_outside_file_on_append() {
        let storage = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let project = storage.path().join("project1");
        std::fs::create_dir_all(&project).unwrap();
        let outside_secret = outside.path().join("secret.txt");
        std::fs::write(&outside_secret, b"original-outside").unwrap();

        let target = project.join("newfile.txt");
        let root = disk_fs(storage.path());
        let path = VirtualPath::new("/projects/project1/newfile.txt").unwrap();

        let escaped = race(
            &target,
            &outside_secret,
            || {
                // The legitimate steady state for a brand-new leaf is
                // "doesn't exist yet" — resolve_for_write's bootstrap path
                // is what a swap here exploits.
                let _ = std::fs::remove_file(&target);
            },
            || {
                let root = &root;
                let path = &path;
                let ok = futures_block_on(async {
                    root.append_file(path, b"attacker-controlled").await.is_ok()
                });
                ok && std::fs::read(&outside_secret)
                    .map(|bytes| bytes != b"original-outside")
                    .unwrap_or(false)
            },
        );

        assert!(
            !escaped,
            "resolve_for_write must never let append_file write through a leaf \
             symlink planted after the parent containment check"
        );
        assert_eq!(
            std::fs::read(&outside_secret).unwrap(),
            b"original-outside",
            "outside file must be untouched once the race loop completes"
        );
    }

    /// (c)/(d) `resolve_for_create_dir_all` and its shared
    /// `ensure_existing_ancestor_contained` ancestor walk: the nearest
    /// *existing* ancestor (`project1`) is legitimate, but the next,
    /// not-yet-existing path component is swapped to a symlink-to-outside
    /// between that ancestor check and `create_dir_all`'s mkdir — pinning
    /// the mkdir-before-recheck ordering documented at `local.rs:227-237`.
    ///
    /// This collapses `ensure_existing_ancestor_contained`'s own case (d)
    /// into this scenario rather than adding a fourth standalone test:
    /// `resolve_for_write`'s call to the same ancestor-walk helper is
    /// provably not independently exploitable, because `resolve_for_write`
    /// re-canonicalizes and re-checks the parent *after* its
    /// `create_dir_all(parent)` call (`local.rs:193-199`) — closing exactly
    /// the window this helper's ancestor check alone would otherwise leave
    /// open. `resolve_for_create_dir_all` has no such re-check between the
    /// ancestor walk and its own `create_dir_all`, so it is the only
    /// reachable caller where swapping the helper's verified ancestor
    /// matters, and this test exercises that call path directly.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolve_for_create_dir_all_ancestor_swap_escapes_mount() {
        let storage = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let project = storage.path().join("project1");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(outside.path()).unwrap();

        // The not-yet-existing ancestor component that gets swapped.
        let newdir = project.join("newdir");
        let root = disk_fs(storage.path());
        let path = VirtualPath::new("/projects/project1/newdir/leaf").unwrap();
        let outside_leaf_marker = outside.path().join("leaf");

        let escaped = race(
            &newdir,
            outside.path(),
            || {
                let _ = std::fs::remove_dir_all(&newdir);
                let _ = std::fs::remove_file(&newdir);
                let _ = std::fs::remove_dir_all(&outside_leaf_marker);
            },
            || {
                let root = &root;
                let path = &path;
                let _ = futures_block_on(async { root.create_dir_all(path).await });
                outside_leaf_marker.is_dir()
            },
        );

        assert!(
            !escaped,
            "resolve_for_create_dir_all must never create a directory outside the \
             mount when a not-yet-existing ancestor is swapped to a symlink after \
             the ancestor containment check"
        );
        assert!(
            !outside_leaf_marker.exists(),
            "outside directory must not gain a leftover 'leaf' entry once the race \
             loop completes"
        );
    }

    /// (d2) Recursive `delete` re-looks-up a classified child by name instead
    /// of treating the classification as the open: `DiskFilesystem::delete`
    /// (and, identically, `remove_dir_contents`'s own recursion) calls
    /// `statat(..., SYMLINK_NOFOLLOW)` to decide "is this a directory to
    /// recurse into", then separately calls `remove_dir_all_fd` ->
    /// `open_one`, which — now that this module follows in-bounds symlinks
    /// — happily opens straight through a symlink planted at that name in
    /// the gap between the two calls. Recursion then empties the symlink's
    /// *target* directory before the trailing `unlinkat(..., REMOVEDIR)`
    /// fails closed on what is now a symlink, not a directory: the data is
    /// already gone by the time that final unlink fails.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn delete_recursive_never_follows_a_classified_directory_swapped_to_a_symlink() {
        let storage = tempdir().unwrap();
        let project = storage.path().join("project1");
        std::fs::create_dir_all(&project).unwrap();

        // An in-bounds sibling directory holding data that must survive —
        // the actual thing `remove_dir_contents`'s no-follow-open fix
        // protects, distinct from `outside_target` in the other cases here
        // (which model an out-of-mount escape): this is an *in-mount*
        // symlink target, exactly the shape this module's "follow in-bounds
        // symlinks" policy makes reachable.
        let target_dir = project.join("target-dir");
        std::fs::create_dir_all(&target_dir).unwrap();
        let precious = target_dir.join("precious.txt");
        std::fs::write(&precious, b"PRECIOUS-CONTENT").unwrap();

        let victim = project.join("victim");
        let root = disk_fs(storage.path());
        let path = VirtualPath::new("/projects/project1/victim").unwrap();

        let escaped = race(
            &victim,
            &target_dir,
            || {
                let _ = std::fs::remove_dir_all(&victim);
                let _ = std::fs::remove_file(&victim);
                std::fs::create_dir_all(&victim).unwrap();
            },
            || {
                let root = &root;
                let path = &path;
                let _ = futures_block_on(async { root.delete(path).await });
                !precious.exists()
            },
        );

        assert!(
            !escaped,
            "recursive delete must never empty a sibling in-mount directory reached \
             by following a symlink planted at the classified entry's name after \
             the SYMLINK_NOFOLLOW statat that classified it as a real directory"
        );
        assert_eq!(
            std::fs::read(&precious).unwrap(),
            b"PRECIOUS-CONTENT",
            "target directory's contents must be untouched once the race loop \
             completes"
        );
    }

    /// (e) PR #6817 follow-up (Task 2): `walk_symlink_target`'s old defense
    /// against stepping above the mount root captured `fd_identity(cur)` and
    /// compared it to the root's identity *before* calling
    /// `openat(cur.as_fd(), "..", …)` — but `..` is always a live,
    /// name-based lookup, resolved by the kernel at the moment of that
    /// `openat` call against whatever `cur`'s *current* parent directory
    /// entry is, not a snapshot from whenever the identity check ran.
    /// Renaming the directory `cur` refers to (its own `(device, inode)` is
    /// unaffected by a rename — only its parent link changes) to a location
    /// outside the mount, concurrently with that `openat` call, makes `..`
    /// resolve to the new, outside parent no matter when the identity check
    /// ran relative to it.
    ///
    /// This doesn't reuse the `race()` helper above: those three cases swap
    /// a *leaf* entry for a symlink-to-outside; this one instead renames an
    /// already-open **ancestor** directory (`project1/subdir`, which holds
    /// the symlink `link -> ../secret.txt`) so its live parent link flips
    /// between "inside the mount" and "under `outside`", for the entire
    /// real-syscall-latency duration of a `read_file` call — the same
    /// many-real-fast-iterations technique, just racing a rename of the
    /// resolver's own already-open ancestor fd's parent instead of a leaf
    /// swap-to-symlink.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn walk_symlink_target_dotdot_rename_race_never_leaks_outside_file() {
        let storage = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let project = storage.path().join("project1");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("secret.txt"), b"legit-content").unwrap();
        std::fs::write(outside.path().join("secret.txt"), b"OUTSIDE SECRET").unwrap();

        let subdir_in = project.join("subdir");
        let subdir_out = outside.path().join("subdir");

        let root = disk_fs(storage.path());
        let path = VirtualPath::new("/projects/project1/subdir/link").unwrap();

        const RACE_ITERATIONS: usize = 2000;
        let mut escaped = false;
        for _ in 0..RACE_ITERATIONS {
            let _ = std::fs::remove_dir_all(&subdir_out);
            let _ = std::fs::remove_dir_all(&subdir_in);
            std::fs::create_dir_all(&subdir_in).unwrap();
            symlink("../secret.txt", subdir_in.join("link")).unwrap();

            let stop = Arc::new(AtomicBool::new(false));
            let racer_stop = Arc::clone(&stop);
            let racer_in = subdir_in.clone();
            let racer_out = subdir_out.clone();
            let racer = std::thread::spawn(move || {
                while !racer_stop.load(Ordering::Relaxed) {
                    // Best-effort, exactly like the other racers above:
                    // failures here just mean we lost this particular
                    // sub-iteration's timing, not a bug.
                    let _ = std::fs::rename(&racer_in, &racer_out);
                    let _ = std::fs::rename(&racer_out, &racer_in);
                }
            });

            let root = &root;
            let path = &path;
            let result = futures_block_on(async { root.read_file(path).await });
            stop.store(true, Ordering::Relaxed);
            let _ = racer.join();

            if matches!(result, Ok(bytes) if bytes == b"OUTSIDE SECRET") {
                escaped = true;
                break;
            }
        }
        // Restore a stable on-disk shape before the tempdirs drop, so
        // cleanup never has to contend with the racer thread's last swap.
        let _ = std::fs::remove_dir_all(&subdir_out);

        assert!(
            !escaped,
            "resolving `..` inside a followed symlink target must never leak bytes \
             from outside the mount, even when a concurrent rename moves the \
             resolver's own already-open ancestor directory to a different, \
             outside parent between the identity check and the old \
             `openat(cur, \"..\")` call"
        );
    }

    /// Blocks the current worker thread on `future`, handing the async
    /// `DiskFilesystem` call to the surrounding multi-threaded
    /// `#[tokio::test]` runtime. `block_in_place` (not a bare
    /// `Handle::block_on`) is required here: it parks this task off its
    /// worker thread so the runtime can keep servicing the racer's own
    /// blocking-pool work (`tokio::fs::*` inside the call under test)
    /// without deadlocking against the synchronous `attempt` closure shape
    /// shared with the plain `reset`/racer-thread code above.
    fn futures_block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
    }
}

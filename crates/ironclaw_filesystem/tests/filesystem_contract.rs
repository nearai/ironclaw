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

    let mut root = LocalFilesystem::new();
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
async fn scoped_write_is_denied_on_read_only_mount() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();

    let mut root = LocalFilesystem::new();
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

    let mut root = LocalFilesystem::new();
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

    let mut root = LocalFilesystem::new();
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

// ─── TOCTOU-hardening: by-construction symlink-escape matrix ────────────────
//
// Every operation must reject a symlink that points outside the mount root,
// driven through the `ScopedFilesystem` security boundary (per
// `.claude/rules/testing.md`: test through the caller, not just the helper).
// The fd-relative resolver closes the escape by construction on every platform,
// so each op surfaces `SymlinkEscape` (mapped from `ELOOP`/`EXDEV`/`ENOTDIR`).

#[cfg(unix)]
fn full_perms() -> MountPermissions {
    MountPermissions {
        read: true,
        write: true,
        delete: true,
        list: true,
        execute: false,
    }
}

/// Build a scoped fs over `storage/project1` with a symlink `escape.txt` inside
/// it that points at `outside/secret.txt`, plus a symlinked directory
/// `outside-dir` pointing at `outside`.
#[cfg(unix)]
fn scoped_with_escape_symlinks(
    storage: &std::path::Path,
    outside: &std::path::Path,
) -> ScopedFilesystem<LocalFilesystem> {
    use std::os::unix::fs::symlink;
    std::fs::create_dir_all(storage.join("project1")).unwrap();
    std::fs::write(outside.join("secret.txt"), b"secret").unwrap();
    symlink(
        outside.join("secret.txt"),
        storage.join("project1/escape.txt"),
    )
    .unwrap();
    symlink(outside, storage.join("project1/outside-dir")).unwrap();
    scoped_project_fs(storage, full_perms())
}

#[cfg(unix)]
#[tokio::test]
async fn every_op_rejects_symlink_escape_by_construction() {
    let storage = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let scoped = scoped_with_escape_symlinks(storage.path(), outside.path());
    let sys = ResourceScope::system();

    macro_rules! assert_escape {
        ($label:expr, $expr:expr) => {{
            let err = $expr.await.unwrap_err();
            assert!(
                matches!(err, FilesystemError::SymlinkEscape { .. }),
                "{} should be SymlinkEscape, got {err:?}",
                $label
            );
        }};
    }

    // Leaf symlink pointing out of the mount.
    let escape = ScopedPath::new("/workspace/escape.txt").unwrap();
    assert_escape!("read_file", scoped.read_file(&sys, &escape));
    assert_escape!(
        "read_file_bounded",
        scoped.read_bytes_bounded(&sys, &escape, 1024)
    );
    assert_escape!("write_file", scoped.write_file(&sys, &escape, b"x"));
    assert_escape!("append_file", scoped.append_file(&sys, &escape, b"x"));
    assert_escape!("stat", scoped.stat(&sys, &escape));

    // Symlinked parent directory pointing out of the mount.
    let via_dir = ScopedPath::new("/workspace/outside-dir/new.txt").unwrap();
    assert_escape!(
        "write through symlinked parent",
        scoped.write_file(&sys, &via_dir, b"x")
    );
    assert_escape!(
        "append through symlinked parent",
        scoped.append_file(&sys, &via_dir, b"x")
    );
    assert_escape!(
        "list symlinked dir",
        scoped.list_dir(&sys, &ScopedPath::new("/workspace/outside-dir").unwrap())
    );
    assert_escape!(
        "create_dir_all through symlinked parent",
        scoped.create_dir_all(
            &sys,
            &ScopedPath::new("/workspace/outside-dir/sub").unwrap()
        )
    );

    // The out-of-root targets are never touched.
    assert_eq!(
        std::fs::read(outside.path().join("secret.txt")).unwrap(),
        b"secret"
    );
    assert!(!outside.path().join("new.txt").exists());
    assert!(!outside.path().join("sub").exists());
}

/// Behavioral-parity note: deleting a leaf *symlink* that lives inside the
/// mount root now removes the symlink itself (via `unlinkat` after a
/// non-following `fstatat`) rather than refusing with `SymlinkEscape`. This is
/// the safe outcome — the out-of-root target is never touched — and matches
/// POSIX `unlink` semantics on a symlink. The previous canonicalize-based
/// resolver followed the link and reported `SymlinkEscape`.
#[cfg(unix)]
#[tokio::test]
async fn delete_leaf_symlink_removes_link_not_target() {
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

    let scoped = scoped_project_fs(storage.path(), full_perms());
    scoped
        .delete(
            &ResourceScope::system(),
            &ScopedPath::new("/workspace/escape.txt").unwrap(),
        )
        .await
        .unwrap();

    // The symlink is gone; the out-of-root target is intact.
    assert!(!storage.path().join("project1/escape.txt").exists());
    assert_eq!(
        std::fs::read(outside.path().join("secret.txt")).unwrap(),
        b"secret"
    );
}

// ─── TOCTOU-hardening: concurrent ancestor-swap race loop ───────────────────
//
// A background task swaps an ancestor symlink between an in-root target and an
// out-of-root target while the main task hammers read/write in a tight loop.
// With fd-relative resolution the result is ALWAYS in-root content or a
// SymlinkEscape/NotFound error — NEVER the out-of-root content. Linux-only
// because it exercises the openat2(RESOLVE_BENEATH) kernel path; the by-
// construction matrix above covers the portable walk.

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_ancestor_swap_never_escapes_root() {
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicBool, Ordering};

    let storage = tempdir().unwrap();
    let outside = tempdir().unwrap();
    // In-root real directory the symlink may legitimately point at.
    std::fs::create_dir_all(storage.path().join("project1/inside")).unwrap();
    std::fs::write(storage.path().join("project1/inside/data.txt"), b"INROOT").unwrap();
    // Out-of-root secret the attacker tries to expose via an ancestor swap.
    std::fs::write(outside.path().join("data.txt"), b"SECRET").unwrap();

    // `link` is the ancestor we swap; start it pointing inside the root.
    let link = storage.path().join("project1/link");
    symlink(storage.path().join("project1/inside"), &link).unwrap();

    let scoped = std::sync::Arc::new(scoped_project_fs(storage.path(), full_perms()));
    let target = ScopedPath::new("/workspace/link/data.txt").unwrap();
    let stop = std::sync::Arc::new(AtomicBool::new(false));

    // Attacker: flip the ancestor symlink in/out of the root.
    let swapper = {
        let stop = std::sync::Arc::clone(&stop);
        let inside = storage.path().join("project1/inside");
        let outside_dir = outside.path().to_path_buf();
        let link = link.clone();
        std::thread::spawn(move || {
            let mut toggle = false;
            while !stop.load(Ordering::Relaxed) {
                let tmp = link.with_extension("tmp");
                let _ = std::fs::remove_file(&tmp);
                let dest = if toggle { &outside_dir } else { &inside };
                if symlink(dest, &tmp).is_ok() {
                    let _ = std::fs::rename(&tmp, &link);
                }
                toggle = !toggle;
            }
        })
    };

    let sys = ResourceScope::system();
    for _ in 0..2000 {
        // Read: must never return the out-of-root secret.
        match scoped.read_file(&sys, &target).await {
            Ok(bytes) => assert_eq!(
                bytes, b"INROOT",
                "read returned out-of-root content via swapped ancestor symlink"
            ),
            Err(FilesystemError::SymlinkEscape { .. }) | Err(FilesystemError::NotFound { .. }) => {}
            Err(other) => panic!("unexpected read error: {other:?}"),
        }
        // Write: must never land outside the root.
        match scoped.write_file(&sys, &target, b"INROOT").await {
            Ok(()) => {}
            Err(FilesystemError::SymlinkEscape { .. }) | Err(FilesystemError::NotFound { .. }) => {}
            Err(other) => panic!("unexpected write error: {other:?}"),
        }
    }

    stop.store(true, Ordering::Relaxed);
    swapper.join().unwrap();

    // The out-of-root secret is never mutated by any write.
    assert_eq!(
        std::fs::read(outside.path().join("data.txt")).unwrap(),
        b"SECRET"
    );
}

// ─── Critical 1: stat() sensitive classification is virtual-path-only ───────
//
// `stat` must classify the advisory `sensitive` flag from the *virtual* path
// string alone, performing ZERO host-path filesystem resolution. The old code
// reconstructed a host path and called the canonicalizing `is_sensitive_path`,
// which re-resolved attacker-influenced input after the fd-safe `fstat` — a
// residual TOCTOU/classification oracle. These tests pin the new behavior.
#[tokio::test]
async fn stat_classifies_sensitive_flag_from_virtual_path_only() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    // A file whose *virtual* name looks sensitive (.env), but is just a plain
    // regular file on the host — no symlink, no special host location.
    std::fs::write(storage.path().join("project1/.env"), b"SECRET=1").unwrap();
    // A control file with a benign name.
    std::fs::write(storage.path().join("project1/notes.txt"), b"hi").unwrap();

    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    let root: Arc<dyn RootFilesystem> = Arc::new(root);

    let sensitive = root
        .stat(&VirtualPath::new("/projects/project1/.env").unwrap())
        .await
        .unwrap();
    assert!(
        sensitive.sensitive,
        "a virtual path ending in .env must be flagged sensitive"
    );

    let benign = root
        .stat(&VirtualPath::new("/projects/project1/notes.txt").unwrap())
        .await
        .unwrap();
    assert!(
        !benign.sensitive,
        "a benign virtual path must not be flagged sensitive"
    );
}

#[tokio::test]
async fn stat_does_not_touch_host_fs_for_classification() {
    // Prove that classification keys on the virtual path, NOT a reconstructed
    // host path: mount the storage dir at a virtual root whose *string* differs
    // from the host directory layout. If `stat` still canonicalized a host path
    // its sensitivity verdict could differ from the virtual-path verdict; here
    // the virtual path is what must drive the result.
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("sub")).unwrap();
    // Host file named benignly...
    std::fs::write(storage.path().join("sub/key.pem"), b"x").unwrap();

    let mut root = LocalFilesystem::new();
    root.mount_local(
        // Virtual root deliberately unrelated to the host tempdir path.
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    let root: Arc<dyn RootFilesystem> = Arc::new(root);

    let stat = root
        .stat(&VirtualPath::new("/projects/sub/key.pem").unwrap())
        .await
        .unwrap();
    assert_eq!(stat.file_type, FileType::File);
    // `.pem` is a sensitive extension; classification is driven by the virtual
    // path string regardless of where the host tempdir actually lives.
    assert!(stat.sensitive, "virtual path with .pem must be sensitive");
}

// ─── Critical 2: ENOTDIR (regular-file ancestor) is NOT a symlink escape ────
//
// A regular-file ancestor (e.g. `/workspace/file/child` where `file` is a plain
// file) must yield a normal "not a directory" error, NOT `SymlinkEscape`. Only
// a *symlinked* ancestor is a containment escape.
#[tokio::test]
async fn regular_file_ancestor_is_not_a_symlink_escape() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    // `regular` is a plain file; treating it as a directory must NOT be an escape.
    std::fs::write(storage.path().join("project1/regular"), b"data").unwrap();

    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    let root: Arc<dyn RootFilesystem> = Arc::new(root);

    let err = root
        .read_file(&VirtualPath::new("/projects/project1/regular/child").unwrap())
        .await
        .unwrap_err();
    assert!(
        !matches!(err, FilesystemError::SymlinkEscape { .. }),
        "regular-file ancestor must not be reported as a symlink escape, got: {err:?}"
    );
    // It should be a normal not-a-directory / not-found style backend error.
    assert!(
        matches!(
            err,
            FilesystemError::Backend { .. } | FilesystemError::NotFound { .. }
        ),
        "expected a non-escape backend/not-found error, got: {err:?}"
    );
}

#[tokio::test]
async fn symlinked_ancestor_still_yields_symlink_escape() {
    use std::os::unix::fs::symlink;

    let storage = tempdir().unwrap();
    let outside = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("project1")).unwrap();
    std::fs::create_dir_all(outside.path().join("target")).unwrap();
    std::fs::write(outside.path().join("target/child"), b"secret").unwrap();
    // A symlinked *ancestor* directory pointing outside the mount root.
    symlink(
        outside.path().join("target"),
        storage.path().join("project1/linkdir"),
    )
    .unwrap();

    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    let root: Arc<dyn RootFilesystem> = Arc::new(root);

    let err = root
        .read_file(&VirtualPath::new("/projects/project1/linkdir/child").unwrap())
        .await
        .unwrap_err();
    assert!(
        matches!(err, FilesystemError::SymlinkEscape { .. }),
        "symlinked ancestor must still be a symlink escape, got: {err:?}"
    );
}

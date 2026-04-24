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

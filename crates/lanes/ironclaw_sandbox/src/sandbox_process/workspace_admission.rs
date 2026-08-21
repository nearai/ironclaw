//! Descriptor-relative admission of the one host workspace leaf a Docker
//! sandbox may bind.

#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

use ironclaw_host_api::{ids::TenantUserWorkspaceKey, process::RuntimeProcessError};

#[cfg(unix)]
pub(super) async fn prepare_workspace_leaf_no_follow(
    workspace_root: PathBuf,
    key: TenantUserWorkspaceKey,
    workspace_mode: u32,
) -> Result<WorkspaceLeafAdmission, RuntimeProcessError> {
    tokio::task::spawn_blocking(move || {
        prepare_workspace_leaf_no_follow_blocking(&workspace_root, &key, workspace_mode)
    })
    .await
    .map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox workspace preparation task failed: {error}"
        ))
    })?
}

#[cfg(not(unix))]
pub(super) async fn prepare_workspace_leaf_no_follow(
    _workspace_root: PathBuf,
    _key: TenantUserWorkspaceKey,
    _workspace_mode: u32,
) -> Result<WorkspaceLeafAdmission, RuntimeProcessError> {
    Err(RuntimeProcessError::ExecutionFailed(
        "sandbox workspace preparation requires Unix no-follow directory handles".to_string(),
    ))
}

#[cfg(unix)]
pub(super) fn workspace_directory_handle_error(
    label: &str,
    error: std::io::Error,
) -> RuntimeProcessError {
    if matches!(
        error.raw_os_error(),
        Some(value) if value == libc::ELOOP || value == libc::EMLINK || value == libc::ENOTDIR
    ) {
        return RuntimeProcessError::ExecutionFailed(format!(
            "sandbox workspace {label} must be a non-symlink directory"
        ));
    }
    RuntimeProcessError::ExecutionFailed(format!(
        "sandbox workspace {label} could not be opened without following links: {error}"
    ))
}

#[cfg(unix)]
fn prepare_workspace_leaf_no_follow_blocking(
    workspace_root: &Path,
    key: &TenantUserWorkspaceKey,
    workspace_mode: u32,
) -> Result<WorkspaceLeafAdmission, RuntimeProcessError> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
            unix::ffi::OsStrExt as _,
        },
    };

    const DIRECTORY_OPEN_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW;

    fn path_c_string(path: &Path) -> Result<CString, RuntimeProcessError> {
        CString::new(path.as_os_str().as_bytes()).map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox workspace root contains an unsupported NUL byte: {error}"
            ))
        })
    }

    fn segment_c_string(segment: &str, label: &str) -> Result<CString, RuntimeProcessError> {
        CString::new(segment).map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox workspace {label} contains an unsupported NUL byte: {error}"
            ))
        })
    }

    fn take_owned_fd(fd: libc::c_int) -> std::io::Result<OwnedFd> {
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: a non-negative descriptor returned by `open`/`openat` is
        // owned by this call and has not been wrapped elsewhere.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn open_directory(path: &Path) -> std::io::Result<OwnedFd> {
        let path = path_c_string(path).map_err(|error| std::io::Error::other(error.to_string()))?;
        // SAFETY: `path` is NUL-terminated for the duration of this syscall;
        // flags request a directory and prohibit following its final symlink.
        let fd = unsafe { libc::open(path.as_ptr(), DIRECTORY_OPEN_FLAGS) };
        take_owned_fd(fd)
    }

    fn open_directory_at(parent: RawFd, name: &std::ffi::CStr) -> std::io::Result<OwnedFd> {
        // SAFETY: `parent` is an owned open directory descriptor, `name` is
        // NUL-terminated, and flags prohibit following the component symlink.
        let fd = unsafe { libc::openat(parent, name.as_ptr(), DIRECTORY_OPEN_FLAGS) };
        take_owned_fd(fd)
    }

    fn mkdir_directory_at(
        parent: RawFd,
        name: &std::ffi::CStr,
        mode: libc::mode_t,
    ) -> std::io::Result<()> {
        // SAFETY: `parent` is an owned open directory descriptor and `name`
        // is NUL-terminated for the duration of this descriptor-relative call.
        let result = unsafe { libc::mkdirat(parent, name.as_ptr(), mode) };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    fn open_or_create_directory_at(
        parent: RawFd,
        name: &std::ffi::CStr,
        label: &str,
    ) -> Result<OwnedFd, RuntimeProcessError> {
        match open_directory_at(parent, name) {
            Ok(directory) => Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Err(error) = mkdir_directory_at(parent, name, 0o700)
                    && error.kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox workspace {label} could not be created without following links: {error}"
                    )));
                }
                open_directory_at(parent, name)
                    .map_err(|error| workspace_directory_handle_error(label, error))
            }
            Err(error) => Err(workspace_directory_handle_error(label, error)),
        }
    }

    fn set_directory_mode(directory: &OwnedFd, mode: u32) -> Result<(), RuntimeProcessError> {
        // SAFETY: `directory` is an owned open directory descriptor. `fchmod`
        // changes that descriptor's inode directly, never a path re-resolution.
        let result = unsafe { libc::fchmod(directory.as_raw_fd(), mode as libc::mode_t) };
        if result == 0 {
            Ok(())
        } else {
            let error = std::io::Error::last_os_error();
            Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox workspace permissions could not be set through its directory handle: {error}"
            )))
        }
    }

    fn directory_stat(directory: &OwnedFd) -> Result<libc::stat, RuntimeProcessError> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: `directory` is an owned open descriptor and `stat` points
        // to writable storage for the kernel to initialize.
        let result = unsafe { libc::fstat(directory.as_raw_fd(), stat.as_mut_ptr()) };
        if result == 0 {
            // SAFETY: `fstat` returned success, so it initialized `stat`.
            Ok(unsafe { stat.assume_init() })
        } else {
            let error = std::io::Error::last_os_error();
            Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox trusted host workspace boundary could not inspect directory ownership: {error}"
            )))
        }
    }

    fn current_host_uid() -> libc::uid_t {
        // SAFETY: `geteuid` has no inputs and only reads the calling process's
        // effective uid.
        unsafe { libc::geteuid() }
    }

    fn validate_host_owned_directory(
        directory: &OwnedFd,
        label: &str,
        require_private_namespace: bool,
    ) -> Result<libc::stat, RuntimeProcessError> {
        let stat = directory_stat(directory)?;
        if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox trusted host workspace boundary rejects {label}: not a directory"
            )));
        }
        let current_uid = current_host_uid();
        if stat.st_uid != current_uid {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox trusted host workspace boundary rejects {label}: owner uid does not match the current host uid"
            )));
        }
        if require_private_namespace && stat.st_mode & (libc::S_IWGRP | libc::S_IWOTH) != 0 {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox trusted host workspace boundary rejects {label}: group or other write permission is not allowed"
            )));
        }
        Ok(stat)
    }

    let root = open_directory(workspace_root)
        .map_err(|error| workspace_directory_handle_error("root", error))?;
    validate_host_owned_directory(&root, "workspace root", true)?;
    let users_name = segment_c_string("users", "users root")?;
    let users = open_or_create_directory_at(root.as_raw_fd(), &users_name, "users root")?;
    validate_host_owned_directory(&users, "users namespace", true)?;
    #[cfg(test)]
    super::workspace_prepare_test_hook::run(workspace_root);
    let leaf_name = segment_c_string(key.digest_segment(), "leaf")?;
    let leaf = open_or_create_directory_at(users.as_raw_fd(), &leaf_name, "leaf")?;
    let leaf_stat = validate_host_owned_directory(&leaf, "workspace leaf", false)?;
    set_directory_mode(&leaf, workspace_mode)?;

    Ok(WorkspaceLeafAdmission {
        path: workspace_root.join("users").join(key.digest_segment()),
        identity: WorkspaceLeafIdentity {
            device: leaf_stat.st_dev as u64,
            inode: leaf_stat.st_ino as u64,
        },
    })
}

pub(super) async fn admit_workspace_leaf(
    workspace_root: PathBuf,
    key: TenantUserWorkspaceKey,
    workspace_mode: u32,
) -> Result<WorkspaceLeafAdmission, RuntimeProcessError> {
    let users_root = workspace_root.join("users");
    let workspace =
        prepare_workspace_leaf_no_follow(workspace_root.clone(), key, workspace_mode).await?;
    let canonical_workspace_root =
        tokio::fs::canonicalize(&workspace_root)
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox workspace root could not be resolved: {error}"
                ))
            })?;
    let canonical_users_root = tokio::fs::canonicalize(&users_root)
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox workspace users root could not be resolved: {error}"
            ))
        })?;
    if canonical_users_root.parent() != Some(canonical_workspace_root.as_path()) {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox workspace users root escapes the configured workspace root".to_string(),
        ));
    }
    let canonical_workspace = tokio::fs::canonicalize(&workspace.path)
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox workspace could not be resolved: {error}"
            ))
        })?;
    if canonical_workspace.parent() != Some(canonical_users_root.as_path()) {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox workspace leaf escapes the configured users root".to_string(),
        ));
    }

    Ok(WorkspaceLeafAdmission {
        path: canonical_workspace,
        identity: workspace.identity,
    })
}

pub(super) async fn revalidate_workspace_host_boundary(
    workspace_root: PathBuf,
    key: TenantUserWorkspaceKey,
    workspace_mode: u32,
    expected_workspace: &WorkspaceLeafAdmission,
) -> Result<(), RuntimeProcessError> {
    let final_workspace = admit_workspace_leaf(workspace_root, key, workspace_mode)
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox trusted host workspace boundary could not be admitted before container creation: {error}"
            ))
        })?;
    if final_workspace.path != expected_workspace.path
        || final_workspace.identity != expected_workspace.identity
    {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox trusted host workspace boundary rejected a changed caller leaf before container creation".to_string(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkspaceLeafIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(unix))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkspaceLeafIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceLeafAdmission {
    /// Docker re-resolves this path for its bind mount after revalidation.
    /// It is therefore not a retained directory handle and remains subject to
    /// filesystem races after admission.
    pub(super) path: PathBuf,
    /// POSIX identity observed during admission. It proves the revalidation
    /// observation, but cannot prevent a later path replacement.
    identity: WorkspaceLeafIdentity,
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::Path};

    use bollard::Docker;
    use ironclaw_host_api::{
        ids::{InvocationId, TenantId, TenantUserWorkspaceKey, UserId},
        process::{CommandExecutionRequest, SandboxCommandTransport},
        resource::ResourceScope,
    };
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::super::{
        RebornSandboxConfig, RebornScopedSandboxCommandTransport, container_create_test_hook,
        test_support, workspace_prepare_test_hook,
    };
    use super::*;

    fn caller_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("acme").expect("tenant"),
            user_id: UserId::new("alice").expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn workspace_transport(workspace_root: &Path) -> RebornScopedSandboxCommandTransport {
        test_support::transport(
            Docker::connect_with_http("http://127.0.0.1:2375", 1, bollard::API_DEFAULT_VERSION)
                .expect("inert docker client configuration"),
            RebornSandboxConfig::new(workspace_root),
        )
    }

    async fn workspace_transport_with_image_resolution_hook(
        workspace_root: &Path,
        on_image_resolution: Box<dyn FnOnce() + Send>,
    ) -> (
        RebornScopedSandboxCommandTransport,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fake Docker listener");
        let endpoint = format!(
            "http://{}",
            listener.local_addr().expect("listener address")
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("Docker image inspect request");
            let mut request = vec![0_u8; 4096];
            let read = stream
                .read(&mut request)
                .await
                .expect("read Docker request");
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(
                request.contains("/images/"),
                "unexpected Docker request: {request}"
            );
            on_image_resolution();

            let body = r#"{"Id":"sha256:test-worker"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write Docker response");
        });
        let docker = Docker::connect_with_http(&endpoint, 1, bollard::API_DEFAULT_VERSION)
            .expect("fake Docker client");
        (
            test_support::transport(
                docker,
                RebornSandboxConfig::new(workspace_root).with_image("worker:test"),
            ),
            server,
        )
    }

    #[tokio::test]
    async fn workspace_preparation_returns_only_the_current_caller_leaf() {
        let temp = tempfile::tempdir().expect("tempdir");
        let scope = caller_scope();
        let workspace_root = temp.path().join("workspaces");
        std::fs::create_dir(&workspace_root).expect("workspace root");
        let transport = workspace_transport(&workspace_root);

        let prepared = transport
            .prepare_workspace(&scope)
            .await
            .expect("prepare caller workspace");

        let expected = tokio::fs::canonicalize(
            workspace_root
                .join("users")
                .join(TenantUserWorkspaceKey::from_scope(&scope).digest_segment()),
        )
        .await
        .expect("canonical caller leaf");
        assert_eq!(prepared.path, expected);
        assert_eq!(prepared.path.parent(), expected.parent());
    }

    #[tokio::test]
    async fn workspace_preparation_requires_a_host_initialized_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let scope = caller_scope();
        let workspace_root = temp.path().join("workspaces");

        let error = workspace_transport(&workspace_root)
            .prepare_workspace(&scope)
            .await
            .expect_err("sandbox must not initialize the configured root through a path lookup");

        assert!(
            format!("{error}").contains("workspace root"),
            "missing root must fail closed: {error}"
        );
        assert!(!workspace_root.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_preparation_preserves_c_string_conversion_cause() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};

        let error = prepare_workspace_leaf_no_follow(
            PathBuf::from(OsString::from_vec(b"workspace\0root".to_vec())),
            TenantUserWorkspaceKey::from_scope(&caller_scope()),
            0o700,
        )
        .await
        .expect_err("NUL-containing workspace roots cannot reach openat");

        assert!(
            format!("{error}").contains("nul byte found"),
            "CString conversion cause must be retained: {error}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_preparation_rejects_symlinked_users_root_or_leaf() {
        use std::os::unix::fs::symlink;

        for symlink_kind in ["users", "leaf"] {
            let temp = tempfile::tempdir().expect("tempdir");
            let scope = caller_scope();
            let workspace_root = temp.path().join("workspaces");
            let outside = temp.path().join("outside");
            std::fs::create_dir(&workspace_root).expect("workspace root");
            std::fs::create_dir(&outside).expect("outside root");
            let key = TenantUserWorkspaceKey::from_scope(&scope);
            if symlink_kind == "users" {
                symlink(&outside, workspace_root.join("users")).expect("users symlink");
            } else {
                std::fs::create_dir(workspace_root.join("users")).expect("users root");
                symlink(
                    &outside,
                    workspace_root.join("users").join(key.digest_segment()),
                )
                .expect("leaf symlink");
            }
            let transport = workspace_transport(&workspace_root);

            let error = transport
                .prepare_workspace(&scope)
                .await
                .expect_err("symlinked caller workspace path must be rejected");

            assert!(
                format!("{error}").contains("symlink"),
                "{symlink_kind}: {error}"
            );
            assert!(
                !outside.join(key.digest_segment()).exists(),
                "{symlink_kind} must not create an outside caller leaf"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn workspace_directory_handle_error_classifies_emlink_as_a_symlink_rejection() {
        let error = workspace_directory_handle_error(
            "leaf",
            std::io::Error::from_raw_os_error(libc::EMLINK),
        );

        assert!(
            format!("{error}").contains("non-symlink directory"),
            "EMLINK must preserve the symlink-rejection error class: {error}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_preparation_rejects_a_group_writable_host_namespace() {
        use std::os::unix::fs::PermissionsExt as _;

        for writable_directory in ["workspace root", "users namespace"] {
            let temp = tempfile::tempdir().expect("tempdir");
            let scope = caller_scope();
            let workspace_root = temp.path().join("workspaces");
            let users_root = workspace_root.join("users");
            std::fs::create_dir(&workspace_root).expect("workspace root");
            std::fs::set_permissions(&workspace_root, std::fs::Permissions::from_mode(0o755))
                .expect("safe workspace root mode");
            if writable_directory == "users namespace" {
                std::fs::create_dir(&users_root).expect("users root");
                std::fs::set_permissions(&users_root, std::fs::Permissions::from_mode(0o775))
                    .expect("group-writable users mode");
            } else {
                std::fs::set_permissions(&workspace_root, std::fs::Permissions::from_mode(0o775))
                    .expect("group-writable workspace mode");
            }

            let error = workspace_transport(&workspace_root)
                .prepare_workspace(&scope)
                .await
                .expect_err("group-writable namespace must fail closed");

            assert!(
                format!("{error}").contains("trusted host workspace boundary"),
                "{writable_directory} must be rejected: {error}"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_preparation_users_swap_cannot_create_an_outside_leaf() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let scope = caller_scope();
        let workspace_root = temp.path().join("workspaces");
        let users_root = workspace_root.join("users");
        let parked_users_root = workspace_root.join("users-before-swap");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&workspace_root).expect("workspace root");
        std::fs::create_dir(&users_root).expect("users root");
        std::fs::create_dir(&outside).expect("outside root");
        let key = TenantUserWorkspaceKey::from_scope(&scope);
        let users_root_for_hook = users_root.clone();
        let outside_for_hook = outside.clone();
        let hook = workspace_prepare_test_hook::install(
            workspace_root.clone(),
            Box::new(move || {
                std::fs::rename(&users_root_for_hook, &parked_users_root).expect("park users root");
                symlink(&outside_for_hook, &users_root_for_hook).expect("swap users root");
            }),
        );

        let error = workspace_transport(&workspace_root)
            .prepare_workspace(&scope)
            .await
            .expect_err("swapped users root must fail closed");

        drop(hook);
        assert!(
            format!("{error}").contains("escapes"),
            "swapped users root must be rejected during containment validation: {error}"
        );
        assert!(
            !outside.join(key.digest_segment()).exists(),
            "workspace preparation must not create a leaf through a swapped users symlink"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_preparation_users_swap_cannot_chmod_an_outside_leaf() {
        use std::os::unix::{fs::PermissionsExt as _, fs::symlink};

        let temp = tempfile::tempdir().expect("tempdir");
        let scope = caller_scope();
        let workspace_root = temp.path().join("workspaces");
        let users_root = workspace_root.join("users");
        let parked_users_root = workspace_root.join("users-before-swap");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&workspace_root).expect("workspace root");
        std::fs::create_dir(&users_root).expect("users root");
        std::fs::create_dir(&outside).expect("outside root");
        let key = TenantUserWorkspaceKey::from_scope(&scope);
        let outside_leaf = outside.join(key.digest_segment());
        std::fs::create_dir(&outside_leaf).expect("outside leaf");
        std::fs::set_permissions(&outside_leaf, std::fs::Permissions::from_mode(0o755))
            .expect("outside leaf mode");
        let users_root_for_hook = users_root.clone();
        let outside_for_hook = outside.clone();
        let hook = workspace_prepare_test_hook::install(
            workspace_root.clone(),
            Box::new(move || {
                std::fs::rename(&users_root_for_hook, &parked_users_root).expect("park users root");
                symlink(&outside_for_hook, &users_root_for_hook).expect("swap users root");
            }),
        );

        let error = workspace_transport(&workspace_root)
            .prepare_workspace(&scope)
            .await
            .expect_err("swapped users root must fail closed");

        drop(hook);
        assert!(
            format!("{error}").contains("escapes"),
            "swapped users root must be rejected during containment validation: {error}"
        );
        assert_eq!(
            std::fs::metadata(&outside_leaf)
                .expect("outside leaf metadata")
                .permissions()
                .mode()
                & 0o777,
            0o755,
            "workspace preparation must not chmod a leaf through a swapped users symlink"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn container_create_rejects_a_users_swap_after_bind_validation() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let scope = caller_scope();
        let workspace_root = temp.path().join("workspaces");
        let users_root = workspace_root.join("users");
        let parked_users_root = workspace_root.join("users-before-swap");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&workspace_root).expect("workspace root");
        std::fs::create_dir(&outside).expect("outside root");
        let users_root_for_hook = users_root.clone();
        let outside_for_hook = outside.clone();
        let hook = container_create_test_hook::install(
            workspace_root.clone(),
            Box::new(move || {
                std::fs::rename(&users_root_for_hook, &parked_users_root).expect("park users root");
                symlink(&outside_for_hook, &users_root_for_hook).expect("swap users root");
            }),
        );

        let (transport, image_server) =
            workspace_transport_with_image_resolution_hook(&workspace_root, Box::new(|| {})).await;
        let error = transport
            .run_command(CommandExecutionRequest {
                scope,
                mounts: None,
                command: "true".to_string(),
                workdir: None,
                timeout_secs: Some(1),
                extra_env: HashMap::new(),
            })
            .await
            .expect_err("post-validation users swap must fail before Docker create");
        image_server.await.expect("fake Docker server completes");

        drop(hook);
        assert!(
            format!("{error}").contains("trusted host workspace boundary"),
            "final host admission must reject the swapped users root: {error}"
        );
        assert!(
            !format!("{error}").contains("sandbox container create failed"),
            "Docker create must not run after final host admission fails: {error}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn container_create_rejects_a_leaf_replacement_after_bind_validation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let scope = caller_scope();
        let workspace_root = temp.path().join("workspaces");
        let users_root = workspace_root.join("users");
        std::fs::create_dir(&workspace_root).expect("workspace root");
        let leaf_name = TenantUserWorkspaceKey::from_scope(&scope)
            .digest_segment()
            .to_string();
        let users_root_for_hook = users_root.clone();
        let hook = container_create_test_hook::install(
            workspace_root.clone(),
            Box::new(move || {
                let leaf = users_root_for_hook.join(&leaf_name);
                std::fs::rename(&leaf, users_root_for_hook.join("replaced-leaf"))
                    .expect("park caller leaf");
                std::fs::create_dir(&leaf).expect("replace caller leaf");
            }),
        );

        let (transport, image_server) =
            workspace_transport_with_image_resolution_hook(&workspace_root, Box::new(|| {})).await;
        let error = transport
            .run_command(CommandExecutionRequest {
                scope,
                mounts: None,
                command: "true".to_string(),
                workdir: None,
                timeout_secs: Some(1),
                extra_env: HashMap::new(),
            })
            .await
            .expect_err("replaced caller leaf must fail before Docker create");
        image_server.await.expect("fake Docker server completes");

        drop(hook);
        assert!(
            format!("{error}").contains("changed caller leaf"),
            "final host admission must reject a replacement at the same path: {error}"
        );
        assert!(
            !format!("{error}").contains("sandbox container create failed"),
            "Docker create must not run after final leaf identity changes: {error}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn container_create_revalidates_after_worker_image_resolution() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let scope = caller_scope();
        let workspace_root = temp.path().join("workspaces");
        let users_root = workspace_root.join("users");
        let parked_users_root = workspace_root.join("users-before-image-resolution");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&workspace_root).expect("workspace root");
        std::fs::create_dir(&outside).expect("outside root");

        let users_root_for_server = users_root.clone();
        let outside_for_server = outside.clone();
        let (transport, server) = workspace_transport_with_image_resolution_hook(
            &workspace_root,
            Box::new(move || {
                std::fs::rename(&users_root_for_server, &parked_users_root)
                    .expect("park users root during image resolution");
                symlink(&outside_for_server, &users_root_for_server)
                    .expect("swap users root during image resolution");
            }),
        )
        .await;

        let error = transport
            .run_command(CommandExecutionRequest {
                scope,
                mounts: None,
                command: "true".to_string(),
                workdir: None,
                timeout_secs: Some(1),
                extra_env: HashMap::new(),
            })
            .await
            .expect_err("workspace swap during image resolution must fail before Docker create");
        server.await.expect("fake Docker server completes");

        assert!(
            format!("{error}").contains("trusted host workspace boundary"),
            "final admission must follow image resolution: {error}"
        );
    }
}

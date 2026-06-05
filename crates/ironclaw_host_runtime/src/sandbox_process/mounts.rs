use std::path::{Path, PathBuf};

use ironclaw_host_api::{MountGrant, MountView, VirtualPath};

use crate::RuntimeProcessError;

use super::CONTAINER_WORKSPACE_ROOT;

#[derive(Debug, Clone, Default)]
pub(super) struct RebornSandboxMountSources {
    sources: Vec<RebornSandboxMountSource>,
}

#[derive(Debug, Clone)]
struct RebornSandboxMountSource {
    virtual_root: VirtualPath,
    host_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ContainerBind {
    source: PathBuf,
    target: String,
    mode: DockerBindMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockerBindMode {
    ReadOnly,
    ReadWrite,
}

impl RebornSandboxMountSources {
    pub(super) fn add_local_source(
        &mut self,
        virtual_root: VirtualPath,
        host_root: impl Into<PathBuf>,
    ) -> Result<(), RuntimeProcessError> {
        if self
            .sources
            .iter()
            .any(|source| source.virtual_root == virtual_root)
        {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "trusted sandbox mount source for {virtual_root} is already configured"
            )));
        }

        let host_root = std::fs::canonicalize(host_root.into()).map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "trusted sandbox mount source for {virtual_root} could not be resolved: {error}"
            ))
        })?;
        if !host_root.is_dir() {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "trusted sandbox mount source for {virtual_root} is not a directory"
            )));
        }

        self.sources.push(RebornSandboxMountSource {
            virtual_root,
            host_root,
        });
        Ok(())
    }

    pub(super) async fn prepare_container_binds(
        &self,
        workspace: &Path,
        mounts: Option<&MountView>,
    ) -> Result<Vec<ContainerBind>, RuntimeProcessError> {
        let mut binds = Vec::new();
        let has_workspace_mount = mounts.is_some_and(|mounts| {
            mounts
                .mounts
                .iter()
                .any(|mount| mount.alias.as_str() == CONTAINER_WORKSPACE_ROOT)
        });
        if !has_workspace_mount {
            binds.push(ContainerBind::new(
                workspace.to_path_buf(),
                CONTAINER_WORKSPACE_ROOT,
                DockerBindMode::ReadWrite,
            )?);
        }

        let Some(mounts) = mounts else {
            return Ok(binds);
        };

        let mut request_binds = Vec::new();
        for grant in &mounts.mounts {
            request_binds.push(self.resolve_grant(grant).await?);
        }
        request_binds.sort_by_key(|bind| bind.target.len());
        binds.extend(request_binds);

        Ok(binds)
    }

    async fn resolve_grant(
        &self,
        grant: &MountGrant,
    ) -> Result<ContainerBind, RuntimeProcessError> {
        validate_container_mount_target(grant.alias.as_str())?;
        let mode = DockerBindMode::from_grant(grant)?;
        let source = self
            .sources
            .iter()
            .filter(|source| {
                virtual_path_prefix_matches(source.virtual_root.as_str(), grant.target.as_str())
            })
            .max_by_key(|source| source.virtual_root.as_str().len())
            .ok_or_else(|| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "no trusted sandbox mount source is configured for virtual path {}",
                    grant.target
                ))
            })?;

        let mut joined = source.host_root.clone();
        let tail = grant
            .target
            .as_str()
            .strip_prefix(source.virtual_root.as_str())
            .unwrap_or_default()
            .trim_start_matches('/');
        if !tail.is_empty() {
            for segment in tail.split('/') {
                joined.push(segment);
            }
        }

        if mode == DockerBindMode::ReadWrite {
            tokio::fs::create_dir_all(&joined).await.map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox mount target {} could not be initialized: {error}",
                    grant.target
                ))
            })?;
        }
        let canonical = tokio::fs::canonicalize(&joined).await.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox mount target {} could not be resolved: {error}",
                grant.target
            ))
        })?;
        if !canonical.starts_with(&source.host_root) {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox mount target {} escapes its trusted source",
                grant.target
            )));
        }
        if !canonical.is_dir() {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox mount target {} is not a directory",
                grant.target
            )));
        }

        ContainerBind::new(canonical, grant.alias.as_str(), mode)
    }
}

impl ContainerBind {
    fn new(
        source: PathBuf,
        target: impl Into<String>,
        mode: DockerBindMode,
    ) -> Result<Self, RuntimeProcessError> {
        let target = target.into();
        reject_nul("sandbox bind source", &source.to_string_lossy())?;
        reject_nul("sandbox bind target", &target)?;
        if source.to_string_lossy().contains(':') || target.contains(':') {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox bind paths cannot contain ':'".to_string(),
            ));
        }
        Ok(Self {
            source,
            target,
            mode,
        })
    }

    pub(super) fn into_docker_bind(self) -> String {
        let mode = match self.mode {
            DockerBindMode::ReadOnly => "ro",
            DockerBindMode::ReadWrite => "rw",
        };
        format!("{}:{}:{mode}", self.source.display(), self.target)
    }
}

impl DockerBindMode {
    fn from_grant(grant: &MountGrant) -> Result<Self, RuntimeProcessError> {
        let permissions = &grant.permissions;
        let readonly = permissions.read
            && permissions.list
            && permissions.execute
            && !permissions.write
            && !permissions.delete;
        let read_write = permissions.read
            && permissions.list
            && permissions.execute
            && permissions.write
            && permissions.delete;
        match (readonly, read_write) {
            (true, false) => Ok(Self::ReadOnly),
            (false, true) => Ok(Self::ReadWrite),
            _ => Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox mount {} permissions cannot be enforced by Docker bind mounts",
                grant.alias
            ))),
        }
    }
}

fn validate_container_mount_target(target: &str) -> Result<(), RuntimeProcessError> {
    const FORBIDDEN_TARGETS: &[&str] = &[
        "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/opt", "/proc", "/root",
        "/run", "/sbin", "/sys", "/usr", "/var",
    ];
    if FORBIDDEN_TARGETS
        .iter()
        .any(|forbidden| target == *forbidden || target.starts_with(&format!("{forbidden}/")))
    {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox mount target collides with the container system filesystem".to_string(),
        ));
    }
    Ok(())
}

fn virtual_path_prefix_matches(prefix: &str, path: &str) -> bool {
    Path::new(path).starts_with(Path::new(prefix))
}

fn reject_nul(label: &str, value: &str) -> Result<(), RuntimeProcessError> {
    if value.as_bytes().contains(&0) {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "{label} contains null bytes"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{MountAlias, MountPermissions};

    use super::*;

    #[test]
    fn trusted_mount_source_validates_host_root_during_config() {
        let mut sources = RebornSandboxMountSources::default();
        let error = sources
            .add_local_source(
                VirtualPath::new("/projects").unwrap(),
                PathBuf::from("/path/that/does/not/exist"),
            )
            .unwrap_err();

        assert!(format!("{error}").contains("could not be resolved"));
    }

    #[test]
    fn trusted_mount_source_rejects_duplicate_virtual_roots() {
        let temp = tempfile::tempdir().unwrap();
        let mut sources = sources_with(VirtualPath::new("/projects").unwrap(), temp.path());

        let error = sources
            .add_local_source(VirtualPath::new("/projects").unwrap(), temp.path())
            .unwrap_err();

        assert!(format!("{error}").contains("already configured"));
    }

    #[tokio::test]
    async fn scoped_workspace_mount_replaces_default_workspace_bind() {
        let temp = tempfile::tempdir().unwrap();
        let source_root = temp.path().join("source");
        let project_root = source_root.join("app");
        tokio::fs::create_dir_all(&project_root).await.unwrap();
        let scoped_workspace = temp.path().join("scoped-workspace");
        tokio::fs::create_dir_all(&scoped_workspace).await.unwrap();
        let sources = sources_with(VirtualPath::new("/projects").unwrap(), &source_root);
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/app").unwrap(),
            process_read_only_permissions(),
        )])
        .unwrap();

        let binds = sources
            .prepare_container_binds(&scoped_workspace, Some(&mounts))
            .await
            .unwrap();

        assert_eq!(binds.len(), 1);
        assert!(
            binds[0]
                .clone()
                .into_docker_bind()
                .ends_with(":/workspace:ro")
        );
        assert!(
            !binds[0]
                .clone()
                .into_docker_bind()
                .starts_with(scoped_workspace.to_str().unwrap())
        );
    }

    #[tokio::test]
    async fn none_mounts_use_default_workspace_bind() {
        let temp = tempfile::tempdir().unwrap();
        let sources = RebornSandboxMountSources::default();

        let binds = sources
            .prepare_container_binds(temp.path(), None)
            .await
            .unwrap();

        assert_eq!(binds.len(), 1);
        assert_eq!(
            binds[0].clone().into_docker_bind(),
            format!("{}:/workspace:rw", temp.path().display())
        );
    }

    #[tokio::test]
    async fn read_write_scoped_mount_initializes_target_directory() {
        let temp = tempfile::tempdir().unwrap();
        let source_root = temp.path().join("source");
        tokio::fs::create_dir_all(&source_root).await.unwrap();
        let sources = sources_with(VirtualPath::new("/projects").unwrap(), &source_root);
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/project").unwrap(),
            VirtualPath::new("/projects/new-task").unwrap(),
            process_read_write_permissions(),
        )])
        .unwrap();

        let binds = sources
            .prepare_container_binds(temp.path(), Some(&mounts))
            .await
            .unwrap();

        assert!(source_root.join("new-task").is_dir());
        assert!(
            binds
                .into_iter()
                .any(|bind| bind.into_docker_bind().ends_with(":/project:rw"))
        );
    }

    #[tokio::test]
    async fn scoped_mount_rejects_unconfigured_virtual_target() {
        let temp = tempfile::tempdir().unwrap();
        let source_root = temp.path().join("source");
        tokio::fs::create_dir_all(&source_root).await.unwrap();
        let sources = sources_with(VirtualPath::new("/projects").unwrap(), source_root);
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/memory/app").unwrap(),
            process_read_only_permissions(),
        )])
        .unwrap();

        let error = sources
            .prepare_container_binds(temp.path(), Some(&mounts))
            .await
            .unwrap_err();

        assert!(format!("{error}").contains("no trusted sandbox mount source"));
    }

    #[tokio::test]
    async fn scoped_mount_rejects_permissions_docker_cannot_enforce() {
        let temp = tempfile::tempdir().unwrap();
        let source_root = temp.path().join("source");
        let project_root = source_root.join("app");
        tokio::fs::create_dir_all(&project_root).await.unwrap();
        let sources = sources_with(VirtualPath::new("/projects").unwrap(), source_root);
        let mut permissions = MountPermissions::read_write();
        permissions.execute = true;
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/app").unwrap(),
            permissions,
        )])
        .unwrap();

        let error = sources
            .prepare_container_binds(temp.path(), Some(&mounts))
            .await
            .unwrap_err();

        assert!(format!("{error}").contains("permissions cannot be enforced"));
    }

    #[tokio::test]
    async fn scoped_mount_rejects_container_system_targets() {
        let temp = tempfile::tempdir().unwrap();
        let source_root = temp.path().join("source");
        let project_root = source_root.join("app");
        tokio::fs::create_dir_all(&project_root).await.unwrap();
        let sources = sources_with(VirtualPath::new("/projects").unwrap(), source_root);
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/etc").unwrap(),
            VirtualPath::new("/projects/app").unwrap(),
            process_read_only_permissions(),
        )])
        .unwrap();

        let error = sources
            .prepare_container_binds(temp.path(), Some(&mounts))
            .await
            .unwrap_err();

        assert!(format!("{error}").contains("container system filesystem"));
    }

    fn sources_with(
        virtual_root: VirtualPath,
        host_root: impl Into<PathBuf>,
    ) -> RebornSandboxMountSources {
        let mut sources = RebornSandboxMountSources::default();
        sources
            .add_local_source(virtual_root, host_root.into())
            .unwrap();
        sources
    }

    fn process_read_only_permissions() -> MountPermissions {
        MountPermissions {
            execute: true,
            ..MountPermissions::read_only()
        }
    }

    fn process_read_write_permissions() -> MountPermissions {
        MountPermissions {
            execute: true,
            ..MountPermissions::read_write_list_delete()
        }
    }
}

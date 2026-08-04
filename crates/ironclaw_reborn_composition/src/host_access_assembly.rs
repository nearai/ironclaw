use std::path::{Path, PathBuf};
use std::sync::Arc;

use ironclaw_filesystem::{CompositeRootFilesystem, ScopedFilesystem};
use ironclaw_host_api::mount::MountPermissions;
use ironclaw_host_api::runtime_policy::{
    EffectiveRuntimePolicy, FilesystemBackendKind, ProcessBackendKind, SecretMode,
};
use ironclaw_host_runtime::HostProcessPort;

use crate::RebornBuildError;
use crate::runtime_mounts::{
    WorkspaceMountPolicy, db_backed_skill_context_mount_view, workspace_mount_view,
};

pub(crate) type WorkspaceFilesystems = (
    Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    WorkspaceMountPolicy,
);

pub(crate) struct HostHomeRoot {
    canonical_root: PathBuf,
    raw_alias: PathBuf,
}

impl HostHomeRoot {
    fn new(canonical_root: PathBuf, raw_alias: PathBuf) -> Self {
        Self {
            canonical_root,
            raw_alias,
        }
    }

    pub(crate) fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    pub(crate) fn aliases(&self) -> Vec<&Path> {
        vec![self.raw_alias.as_path(), self.canonical_root.as_path()]
    }
}

pub(crate) struct HostAccessAssembly {
    pub(crate) storage_root: PathBuf,
    pub(crate) workspace_root: PathBuf,
    pub(crate) host_home_root: Option<HostHomeRoot>,
    pub(crate) process_port: Option<HostProcessPort>,
}

impl HostAccessAssembly {
    /// `workspace_scoped_per_caller` is the deployment's single workspace
    /// scoping decision (see
    /// [`crate::deployment::DeploymentConfig::workspace_scoped_per_caller`]).
    /// Passing it here keeps the ambient host aliases below reachable only for
    /// the deployments that are allowed them.
    pub(crate) fn build_workspace_filesystems(
        &self,
        filesystem: Arc<CompositeRootFilesystem>,
        workspace_scoped_per_caller: bool,
    ) -> Result<WorkspaceFilesystems, RebornBuildError> {
        let read_only_workspace_mounts = workspace_mount_view(MountPermissions::read_only(), &[])
            .map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
        let host_home_aliases = self
            .host_home_root
            .as_ref()
            .map(HostHomeRoot::aliases)
            .unwrap_or_default();
        let workspace_aliases = if self.host_home_root.is_some() {
            vec![self.workspace_root.as_path()]
        } else {
            Vec::new()
        };
        let runtime_workspace_mounts = WorkspaceMountPolicy::resolve(
            workspace_scoped_per_caller,
            &workspace_aliases,
            &host_home_aliases,
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
        // Database-backed, matching the writer. A disk-backed view here is nearai/ironclaw#7168:
        // every production-shaped build writes skills to the database, so a reader on
        // `/projects/...` never sees an installed skill again.
        let skill_filesystem = Arc::new(ScopedFilesystem::new(
            Arc::clone(&filesystem),
            db_backed_skill_context_mount_view,
        ));
        let workspace_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
            filesystem,
            read_only_workspace_mounts,
        ));
        Ok((
            skill_filesystem,
            workspace_filesystem,
            runtime_workspace_mounts,
        ))
    }
}

/// Materializes host filesystem/process access from resolved policy data.
pub(crate) fn build_host_access(
    storage_root: PathBuf,
    workspace_root: Option<PathBuf>,
    host_home_root: Option<PathBuf>,
    runtime_policy: Option<EffectiveRuntimePolicy>,
) -> Result<HostAccessAssembly, RebornBuildError> {
    initialize_directory(&storage_root, "storage root")?;
    initialize_directory(
        &storage_root.join("system/extensions"),
        "system extensions root",
    )?;
    let workspace_root = workspace_root.unwrap_or_else(|| storage_root.join("workspace"));
    initialize_directory(&workspace_root, "workspace root")?;

    let storage_root = canonicalize_path(&storage_root, "storage root")?;
    let workspace_root = canonicalize_path(&workspace_root, "workspace root")?;
    validate_workspace_skill_isolation(&storage_root, &workspace_root)?;

    let include_host_home = runtime_policy.as_ref().is_some_and(|policy| {
        policy.filesystem_backend == FilesystemBackendKind::HostWorkspaceAndHome
    });
    let host_home_root = match (include_host_home, host_home_root) {
        (true, Some(path)) => Some(HostHomeRoot::new(canonicalize_host_home_root(&path)?, path)),
        (true, None) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: "host home access requires a confirmed host home root".to_string(),
            });
        }
        (false, Some(_)) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: "confirmed host home root was supplied but the resolved runtime policy \
                             does not allow host home access"
                    .to_string(),
            });
        }
        (false, None) => None,
    };
    let process_port = process_port_for_policy(
        runtime_policy.as_ref(),
        &workspace_root,
        host_home_root.as_ref(),
    );

    Ok(HostAccessAssembly {
        storage_root,
        workspace_root,
        host_home_root,
        process_port,
    })
}

fn initialize_directory(path: &Path, label: &str) -> Result<(), RebornBuildError> {
    std::fs::create_dir_all(path).map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("{label} could not be initialized: {error}"),
    })
}

fn canonicalize_path(path: &Path, label: &str) -> Result<PathBuf, RebornBuildError> {
    std::fs::canonicalize(path).map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("{label} could not be resolved: {error}"),
    })
}

fn canonicalize_existing_dir(path: &Path, label: &str) -> Result<PathBuf, RebornBuildError> {
    let path = canonicalize_path(path, label)?;
    let metadata = std::fs::metadata(&path).map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("{label} could not be inspected: {error}"),
    })?;
    if metadata.is_dir() {
        Ok(path)
    } else {
        Err(RebornBuildError::InvalidConfig {
            reason: format!("{label} must be an existing directory"),
        })
    }
}

fn canonicalize_host_home_root(path: &Path) -> Result<PathBuf, RebornBuildError> {
    let path = canonicalize_existing_dir(path, "host home root")?;
    if path.parent().is_none() {
        return Err(RebornBuildError::InvalidConfig {
            reason: "host home root must not be a filesystem root".to_string(),
        });
    }
    Ok(path)
}

pub(crate) fn validate_workspace_skill_isolation(
    storage_root: &Path,
    workspace_root: &Path,
) -> Result<(), RebornBuildError> {
    for (label, skill_root) in [
        ("/skills", storage_root.join("skills")),
        (
            "/tenant-shared/skills",
            storage_root.join("tenant-shared/skills"),
        ),
        ("/system/skills", storage_root.join("system/skills")),
        ("/system/extensions", storage_root.join("system/extensions")),
    ] {
        if paths_overlap(workspace_root, &skill_root) {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!("workspace root must not overlap default skill root {label}"),
            });
        }
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn process_port_for_policy(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    workspace_root: &Path,
    host_home_root: Option<&HostHomeRoot>,
) -> Option<HostProcessPort> {
    let runtime_policy = runtime_policy?;
    if runtime_policy.process_backend != ProcessBackendKind::LocalHost {
        return None;
    }
    let mut process_port = if runtime_policy.secret_mode == SecretMode::InheritedEnv {
        HostProcessPort::new_inherited_env()
    } else {
        HostProcessPort::new()
    }
    .with_workdir_alias("/workspace", workspace_root);
    if let Some(host_home_root) = host_home_root {
        process_port =
            process_port.with_workdir_alias("/host", host_home_root.canonical_root().to_path_buf());
        for alias in host_home_root.aliases() {
            let Some(alias_str) = alias.to_str() else {
                tracing::debug!(alias = ?alias, "skipping non-UTF-8 host home alias");
                continue;
            };
            process_port = process_port.with_workdir_alias(alias_str, alias.to_path_buf());
        }
    }
    Some(process_port)
}

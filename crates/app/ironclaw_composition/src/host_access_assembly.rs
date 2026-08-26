use std::path::{Path, PathBuf};
use std::sync::Arc;

use ironclaw_config::RebornStoragePaths;
use ironclaw_filesystem::{CompositeRootFilesystem, DiskDirectoryCapability, ScopedFilesystem};
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
    pub(crate) state_root: PathBuf,
    pub(crate) system_root: PathBuf,
    pub(crate) workspace_root: PathBuf,
    pub(crate) host_home_root: Option<HostHomeRoot>,
    pub(crate) process_port: Option<HostProcessPort>,
    pub(crate) disk_mounts: HostDiskMountCapabilities,
}

pub(crate) struct HostDiskMountCapabilities {
    pub(crate) workspace: DiskDirectoryCapability,
    pub(crate) system_extensions: DiskDirectoryCapability,
    pub(crate) system_prompts: DiskDirectoryCapability,
    pub(crate) system_skills: DiskDirectoryCapability,
}

impl HostAccessAssembly {
    /// Builds workspace views from the deployment's resolved caller-scoping decision.
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
    paths: RebornStoragePaths,
    workspace_root_for_test: Option<PathBuf>,
    host_home_root: Option<PathBuf>,
    runtime_policy: Option<EffectiveRuntimePolicy>,
    workspace_scoped_per_caller: bool,
) -> Result<HostAccessAssembly, RebornBuildError> {
    let configured_workspace_root = workspace_root_for_test
        .as_deref()
        .unwrap_or_else(|| paths.workspace_root());
    preflight_storage_namespace_paths(&paths, configured_workspace_root)?;

    let installation =
        initialize_directory_capability(paths.installation_root(), "installation root")?;
    // Validate and initialize the state namespace here; filesystem assembly
    // subsequently reopens this same canonical path for the durable backend.
    let _state = initialize_descendant_capability(
        &installation,
        paths.installation_root(),
        paths.state_root(),
        "state root",
    )?;
    let system = initialize_descendant_capability(
        &installation,
        paths.installation_root(),
        paths.system_root(),
        "system root",
    )?;
    let system_extensions = system
        .create_dir_capability(Path::new("extensions"))
        .map_err(|error| directory_initialization_error("system extensions root", error))?;
    let system_prompts = system
        .create_dir_capability(Path::new("prompts"))
        .map_err(|error| directory_initialization_error("system prompts root", error))?;
    let system_skills = system
        .create_dir_capability(Path::new("skills"))
        .map_err(|error| directory_initialization_error("system skills root", error))?;
    let workspace = initialize_descendant_capability(
        &installation,
        paths.installation_root(),
        configured_workspace_root,
        "workspace root",
    )?;

    let installation_root = canonicalize_path(paths.installation_root(), "installation root")?;
    let state_root = canonicalize_path(paths.state_root(), "state root")?;
    let system_root = canonicalize_path(paths.system_root(), "system root")?;
    let workspace_root = canonicalize_path(configured_workspace_root, "workspace root")?;
    validate_canonical_storage_paths(
        &installation_root,
        &state_root,
        &system_root,
        &workspace_root,
    )?;
    validate_workspace_skill_isolation(&system_root, &workspace_root)?;

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
        workspace_scoped_per_caller,
    );

    Ok(HostAccessAssembly {
        state_root,
        system_root,
        workspace_root,
        host_home_root,
        process_port,
        disk_mounts: HostDiskMountCapabilities {
            workspace,
            system_extensions,
            system_prompts,
            system_skills,
        },
    })
}

fn initialize_directory_capability(
    path: &Path,
    label: &str,
) -> Result<DiskDirectoryCapability, RebornBuildError> {
    // The installation root itself may be an operator-managed alias (for
    // example a mounted volume), while all namespaces beneath it must reject
    // aliases. Resolve that one configured boundary first, then perform the
    // actual creation/admission against its canonical projected path so the
    // retained descriptor never depends on reopening the alias.
    let admitted_path = canonicalize_planned_path(path, label)?;
    DiskDirectoryCapability::admit_or_create(&admitted_path)
        .map_err(|error| directory_initialization_error(label, error))
}

fn initialize_descendant_capability(
    installation: &DiskDirectoryCapability,
    configured_installation_root: &Path,
    path: &Path,
    label: &str,
) -> Result<DiskDirectoryCapability, RebornBuildError> {
    let relative = path
        .strip_prefix(configured_installation_root)
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("{label} must be beneath the selected installation root: {error}"),
        })?;
    installation
        .create_dir_capability(relative)
        .map_err(|error| directory_initialization_error(label, error))
}

fn directory_initialization_error(label: &str, error: std::io::Error) -> RebornBuildError {
    RebornBuildError::InvalidConfig {
        reason: format!("{label} could not be initialized: {error}"),
    }
}

/// Checks every resolved namespace before `create_dir_all` can follow a configured alias.
///
/// The roots can be absent on first boot, so this resolves each path through its nearest existing
/// ancestor and validates the projected canonical result. It then rejects any existing symlink in
/// the installation or namespace ancestry. Initialization then creates descendants relative to
/// retained directory capabilities and hands those same capabilities to the disk mount owner.
fn preflight_storage_namespace_paths(
    paths: &RebornStoragePaths,
    workspace_root: &Path,
) -> Result<(), RebornBuildError> {
    let installation_root =
        canonicalize_planned_path(paths.installation_root(), "installation root")?;
    let state_root = canonicalize_planned_path(paths.state_root(), "state root")?;
    let system_root = canonicalize_planned_path(paths.system_root(), "system root")?;
    let canonical_workspace_root = canonicalize_planned_path(workspace_root, "workspace root")?;
    validate_canonical_storage_paths(
        &installation_root,
        &state_root,
        &system_root,
        &canonical_workspace_root,
    )?;
    validate_workspace_skill_isolation(&system_root, &canonical_workspace_root)?;

    let system_extensions_root = paths.system_root().join("extensions");
    let system_prompts_root = paths.system_root().join("prompts");
    let system_skills_root = paths.system_root().join("skills");
    for (label, path) in [
        ("state root", paths.state_root()),
        ("system root", paths.system_root()),
        ("system extensions root", system_extensions_root.as_path()),
        ("system prompts root", system_prompts_root.as_path()),
        ("system skills root", system_skills_root.as_path()),
        ("workspace root", workspace_root),
    ] {
        reject_existing_namespace_symlinks(paths.installation_root(), path, label)?;
    }
    Ok(())
}

/// Resolves an existing path, or projects missing suffixes from the nearest existing ancestor.
/// This never creates an entry and intentionally follows existing aliases only long enough for the
/// canonical containment/disjointness validation performed by the caller.
fn canonicalize_planned_path(path: &Path, label: &str) -> Result<PathBuf, RebornBuildError> {
    let mut current = path;
    let mut missing_components = Vec::new();
    loop {
        match std::fs::symlink_metadata(current) {
            Ok(_) => {
                let canonical = std::fs::canonicalize(current).map_err(|error| {
                    RebornBuildError::InvalidConfig {
                        reason: format!("{label} could not be resolved: {error}"),
                    }
                })?;
                let canonical_metadata = std::fs::metadata(&canonical).map_err(|error| {
                    RebornBuildError::InvalidConfig {
                        reason: format!("{label} could not be inspected: {error}"),
                    }
                })?;
                if !canonical_metadata.is_dir() {
                    return Err(RebornBuildError::InvalidConfig {
                        reason: format!("{label} must be a directory"),
                    });
                }
                let mut projected = canonical;
                for component in missing_components.iter().rev() {
                    projected.push(component);
                }
                return Ok(projected);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = current.file_name() else {
                    return Err(RebornBuildError::InvalidConfig {
                        reason: format!("{label} has no existing ancestor"),
                    });
                };
                missing_components.push(name.to_os_string());
                let Some(parent) = current.parent() else {
                    return Err(RebornBuildError::InvalidConfig {
                        reason: format!("{label} has no existing ancestor"),
                    });
                };
                current = parent;
            }
            Err(error) => {
                return Err(RebornBuildError::InvalidConfig {
                    reason: format!("{label} could not be inspected: {error}"),
                });
            }
        }
    }
}

fn reject_existing_namespace_symlinks(
    installation_root: &Path,
    path: &Path,
    label: &str,
) -> Result<(), RebornBuildError> {
    let relative =
        path.strip_prefix(installation_root)
            .map_err(|_| RebornBuildError::InvalidConfig {
                reason: format!("{label} must be beneath the selected installation root"),
            })?;
    // The selected installation root may legitimately be an operator-managed
    // symlink onto a durable volume. Its descendants are still forbidden from
    // being aliases, so the walk begins at the resolved root.
    let mut current = canonicalize_planned_path(installation_root, "installation root")?;
    for component in relative.components() {
        current.push(component.as_os_str());
        if !inspect_namespace_component(&current, label)? {
            break;
        }
    }
    Ok(())
}

/// Returns false only when this and every descendant are necessarily absent.
fn inspect_namespace_component(path: &Path, label: &str) -> Result<bool, RebornBuildError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RebornBuildError::InvalidConfig {
            reason: format!("{label} must not contain a symlink: {}", path.display()),
        }),
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(RebornBuildError::InvalidConfig {
            reason: format!("{label} must be a directory: {}", path.display()),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(RebornBuildError::InvalidConfig {
            reason: format!("{label} could not be inspected: {error}"),
        }),
    }
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

fn validate_canonical_storage_paths(
    installation_root: &Path,
    state_root: &Path,
    system_root: &Path,
    workspace_root: &Path,
) -> Result<(), RebornBuildError> {
    let roots = [
        ("state root", state_root),
        ("system root", system_root),
        ("workspace root", workspace_root),
    ];
    for (label, root) in roots {
        if root == installation_root || !root.starts_with(installation_root) {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!("{label} must be beneath the selected installation root"),
            });
        }
    }
    for (index, (left_label, left)) in roots.iter().enumerate() {
        for (right_label, right) in roots.iter().skip(index + 1) {
            if paths_overlap(left, right) {
                return Err(RebornBuildError::InvalidConfig {
                    reason: format!("{left_label} must not overlap {right_label}"),
                });
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_workspace_skill_isolation(
    system_root: &Path,
    workspace_root: &Path,
) -> Result<(), RebornBuildError> {
    for (label, skill_root) in [
        ("/system/skills", system_root.join("skills")),
        ("/system/extensions", system_root.join("extensions")),
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
    workspace_scoped_per_caller: bool,
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
    .with_workdir_alias("/workspace", workspace_root)
    // Same scoping the file tools apply. Without it `/workspace` names `<root>` here and
    // `<root>/users/<tenant-user-digest>` there, so a file written by one is unreachable by the other.
    .with_workspace_scoped_per_caller(workspace_scoped_per_caller);
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

#[cfg(all(test, unix))]
#[path = "host_access_assembly_tests.rs"]
mod tests;

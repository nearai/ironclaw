use ironclaw_filesystem::{FileStat, FilesystemError, FilesystemOperation};
use ironclaw_host_api::{RuntimeDispatchErrorKind, ScopedPath, VirtualPath};
use ironclaw_safety::sensitive_paths::is_sensitive_path_str;
use serde_json::Value;

use super::{CodingCapabilityError, CodingCapabilityRequest};

use super::{
    config::{DEFAULT_EXCLUDED_DIRS, DEFAULT_SCOPED_ROOT, WORKSPACE_FILES},
    input_error,
    inputs::required_str,
    operation_error,
    types::ResolvedPath,
};

pub(super) fn resolve_required_path(
    request: &CodingCapabilityRequest<'_>,
    field: &str,
    operation: FilesystemOperation,
) -> Result<ResolvedPath, CodingCapabilityError> {
    resolve_path(request, required_str(request.input, field)?, operation)
}

pub(super) fn resolve_optional_path(
    request: &CodingCapabilityRequest<'_>,
    operation: FilesystemOperation,
) -> Result<ResolvedPath, CodingCapabilityError> {
    let path = request
        .input
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_SCOPED_ROOT);
    resolve_path(request, path, operation)
}

fn resolve_path(
    request: &CodingCapabilityRequest<'_>,
    path: &str,
    operation: FilesystemOperation,
) -> Result<ResolvedPath, CodingCapabilityError> {
    let mounts = request
        .mounts
        .ok_or_else(|| CodingCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied))?;
    let scoped_path = mounts
        .scoped_path(scoped_path_input(path))
        .map_err(|_| input_error())?;
    if is_sensitive_scoped_path(scoped_path.as_str()) {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    let (virtual_path, grant) = mounts
        .resolve_with_grant(&scoped_path)
        .map_err(|_| CodingCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied))?;
    if is_sensitive_resolved_path(&virtual_path) {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    if !operation_allowed(&grant.permissions, operation) {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    Ok(ResolvedPath {
        scoped_path,
        virtual_path,
        grant: grant.clone(),
    })
}

fn scoped_path_input(path: &str) -> String {
    if path == "." || path.is_empty() {
        DEFAULT_SCOPED_ROOT.to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{}/{}", DEFAULT_SCOPED_ROOT, path.trim_start_matches("./"))
    }
}

pub(super) fn operation_allowed(
    permissions: &ironclaw_host_api::MountPermissions,
    operation: FilesystemOperation,
) -> bool {
    match operation {
        FilesystemOperation::ReadFile => permissions.read,
        FilesystemOperation::WriteFile | FilesystemOperation::AppendFile => permissions.write,
        FilesystemOperation::ListDir => permissions.list,
        FilesystemOperation::Stat => permissions.read || permissions.list,
        FilesystemOperation::Delete => permissions.delete,
        FilesystemOperation::CreateDirAll => permissions.write,
        FilesystemOperation::MountLocal => false,
        // Coding tools never use the unified record/index/txn/event surface
        // — they are bytes-only. If a future code path routes here, treat
        // record-plane reads as `read` and writes as `write` to stay
        // fail-closed. `Append` (event-plane append) is distinct from
        // `AppendFile` (byte-plane append onto a regular file) but both
        // map to `permissions.write`.
        FilesystemOperation::Query => permissions.read && permissions.list,
        FilesystemOperation::EnsureIndex
        | FilesystemOperation::BeginTxn
        | FilesystemOperation::Append => permissions.write,
        FilesystemOperation::Tail => permissions.read,
    }
}

pub(super) async fn stat_optional(
    request: &CodingCapabilityRequest<'_>,
    path: &VirtualPath,
) -> Result<Option<FileStat>, CodingCapabilityError> {
    match request.filesystem.stat(path).await {
        Ok(stat) => Ok(Some(stat)),
        Err(FilesystemError::NotFound { .. }) => Ok(None),
        Err(error) => Err(filesystem_error(error)),
    }
}

pub(super) async fn create_parent_dir_unless_sensitive(
    request: &CodingCapabilityRequest<'_>,
    path: &VirtualPath,
) -> Result<(), CodingCapabilityError> {
    let Some(parent) = virtual_parent(path)? else {
        return Ok(());
    };
    deny_nearest_sensitive_existing_parent(request, parent.clone()).await?;
    request
        .filesystem
        .create_dir_all(&parent)
        .await
        .map_err(filesystem_denied_if_not_found)
}

pub(super) async fn deny_sensitive_existing_path(
    request: &CodingCapabilityRequest<'_>,
    path: &VirtualPath,
) -> Result<(), CodingCapabilityError> {
    let stat = request
        .filesystem
        .stat(path)
        .await
        .map_err(filesystem_error)?;
    if stat.sensitive {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    Ok(())
}

/// Walk up the directory tree, denying if any existing parent is sensitive.
///
/// Best-effort check for the local-dev threat model: assumes a trusted filesystem
/// where parent directories do not become sensitive between this walk and the
/// subsequent `create_dir_all` (TOCTOU).
async fn deny_nearest_sensitive_existing_parent(
    request: &CodingCapabilityRequest<'_>,
    mut candidate: VirtualPath,
) -> Result<(), CodingCapabilityError> {
    loop {
        match request.filesystem.stat(&candidate).await {
            Ok(stat) => {
                if stat.sensitive {
                    return Err(CodingCapabilityError::new(
                        RuntimeDispatchErrorKind::FilesystemDenied,
                    ));
                }
                return Ok(());
            }
            Err(FilesystemError::NotFound { .. }) => {
                let Some(parent) = virtual_parent(&candidate)? else {
                    return Ok(());
                };
                candidate = parent;
            }
            Err(error) => return Err(filesystem_error(error)),
        }
    }
}

fn filesystem_denied_if_not_found(error: FilesystemError) -> CodingCapabilityError {
    match error {
        FilesystemError::NotFound { .. } => {
            CodingCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied)
        }
        error => filesystem_error(error),
    }
}

fn virtual_parent(path: &VirtualPath) -> Result<Option<VirtualPath>, CodingCapabilityError> {
    let raw = path.as_str().trim_end_matches('/');
    let Some((parent, _leaf)) = raw.rsplit_once('/') else {
        return Ok(None);
    };
    if parent.is_empty() {
        return Ok(None);
    }
    VirtualPath::new(parent)
        .map(Some)
        .map_err(|_| CodingCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied))
}

pub(super) fn virtual_to_relative(
    root: &VirtualPath,
    path: &VirtualPath,
) -> Result<String, CodingCapabilityError> {
    let target = root.as_str().trim_end_matches('/');
    let raw = path.as_str();
    if raw == target {
        return Ok(String::new());
    }
    raw.strip_prefix(&format!("{target}/"))
        .map(ToString::to_string)
        .ok_or_else(|| CodingCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied))
}

pub(super) fn validate_relative_pattern(pattern: &str) -> Result<(), CodingCapabilityError> {
    if pattern.starts_with('/') || pattern.split('/').any(|segment| segment == "..") {
        return Err(input_error());
    }
    Ok(())
}

pub(super) fn is_excluded_name(name: &str) -> bool {
    DEFAULT_EXCLUDED_DIRS.contains(&name)
}

pub(super) fn is_excluded_relative_path(path: &str) -> bool {
    path.split('/').any(is_excluded_name)
}

pub(super) fn type_filter_matches(path: &str, type_filter: &str) -> bool {
    let extension = path
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or_default();
    match type_filter {
        "rust" | "rs" => extension == "rs",
        "py" | "python" => extension == "py",
        "js" | "javascript" => extension == "js" || extension == "jsx",
        "ts" | "typescript" => extension == "ts" || extension == "tsx",
        other => extension == other,
    }
}

pub(super) fn is_workspace_path(path: &str) -> bool {
    let scoped = scoped_path_input(path);
    let normalized = scoped.trim_start_matches('/');
    let relative = normalized.strip_prefix("workspace/").unwrap_or(normalized);
    // This intentionally protects only root workspace memory files. Project
    // docs such as README.md remain writable through the scoped filesystem.
    (!relative.contains('/') && WORKSPACE_FILES.contains(&relative))
        || relative.starts_with("daily/")
        || relative.starts_with("context/")
}

pub(super) fn scoped_child_path(root: &ScopedPath, relative: &str) -> String {
    if relative.is_empty() {
        root.as_str().to_string()
    } else {
        format!("{}/{}", root.as_str().trim_end_matches('/'), relative)
    }
}

pub(super) fn is_sensitive_scoped_path(path: &str) -> bool {
    is_sensitive_path_str(path)
}

fn is_sensitive_resolved_path(path: &VirtualPath) -> bool {
    is_sensitive_path_str(path.as_str())
}

pub(super) fn filesystem_error(error: FilesystemError) -> CodingCapabilityError {
    match error {
        FilesystemError::Contract(_) => input_error(),
        FilesystemError::PermissionDenied { .. }
        | FilesystemError::MountNotFound { .. }
        | FilesystemError::PathOutsideMount { .. }
        | FilesystemError::SymlinkEscape { .. }
        | FilesystemError::MountConflict { .. } => {
            CodingCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied)
        }
        FilesystemError::NotFound { .. } => operation_error(),
        FilesystemError::Backend { .. } | FilesystemError::BackendInfrastructure { .. } => {
            CodingCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        }
        // The unified record/index/CAS variants are surfaced when a backend
        // declines a typed op. Coding tools only exercise bytes, so reaching
        // here means the underlying mount is misconfigured for this caller —
        // treat as a denial rather than leaking the typed shape.
        FilesystemError::VersionMismatch { .. }
        | FilesystemError::Unsupported { .. }
        | FilesystemError::IndexConflict { .. } => {
            CodingCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied)
        }
        // FilesystemError is #[non_exhaustive]; any future variant maps to a
        // denial here until coding-tool semantics for it are designed.
        _ => CodingCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied),
    }
}

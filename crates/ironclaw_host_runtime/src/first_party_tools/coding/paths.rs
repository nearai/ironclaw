use ironclaw_filesystem::{FileStat, FilesystemError, FilesystemOperation};
use ironclaw_host_api::{RuntimeDispatchErrorKind, ScopedPath, VirtualPath};
use ironclaw_safety::sensitive_paths::is_sensitive_path_str;
use serde_json::Value;

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{
    config::{DEFAULT_EXCLUDED_DIRS, DEFAULT_SCOPED_ROOT, WORKSPACE_FILES},
    guest_error, input_error,
    inputs::required_str,
    types::ResolvedPath,
};

pub(super) fn resolve_required_path(
    request: &FirstPartyCapabilityRequest,
    field: &str,
    operation: FilesystemOperation,
) -> Result<ResolvedPath, FirstPartyCapabilityError> {
    resolve_path(request, required_str(&request.input, field)?, operation)
}

pub(super) fn resolve_optional_path(
    request: &FirstPartyCapabilityRequest,
    operation: FilesystemOperation,
) -> Result<ResolvedPath, FirstPartyCapabilityError> {
    let path = request
        .input
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_SCOPED_ROOT);
    resolve_path(request, path, operation)
}

fn resolve_path(
    request: &FirstPartyCapabilityRequest,
    path: &str,
    operation: FilesystemOperation,
) -> Result<ResolvedPath, FirstPartyCapabilityError> {
    let scoped_path = ScopedPath::new(scoped_path_input(path)).map_err(|_| input_error())?;
    if is_sensitive_scoped_path(scoped_path.as_str()) {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    let mounts = request.mounts.as_ref().ok_or_else(|| {
        FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied)
    })?;
    let (virtual_path, grant) = mounts
        .resolve_with_grant(&scoped_path)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied))?;
    if is_sensitive_resolved_path(&virtual_path) {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    if !operation_allowed(&grant.permissions, operation) {
        return Err(FirstPartyCapabilityError::new(
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
    }
}

pub(super) async fn stat_optional(
    request: &FirstPartyCapabilityRequest,
    path: &VirtualPath,
) -> Result<Option<FileStat>, FirstPartyCapabilityError> {
    match request.filesystem.stat(path).await {
        Ok(stat) => Ok(Some(stat)),
        Err(FilesystemError::NotFound { .. }) => Ok(None),
        Err(error) => Err(filesystem_error(error)),
    }
}

pub(super) async fn create_parent_dir(
    request: &FirstPartyCapabilityRequest,
    path: &VirtualPath,
) -> Result<(), FirstPartyCapabilityError> {
    let Some(parent) = virtual_parent(path)? else {
        return Ok(());
    };
    request
        .filesystem
        .create_dir_all(&parent)
        .await
        .map_err(filesystem_error)
}

fn virtual_parent(path: &VirtualPath) -> Result<Option<VirtualPath>, FirstPartyCapabilityError> {
    let raw = path.as_str().trim_end_matches('/');
    let Some((parent, _leaf)) = raw.rsplit_once('/') else {
        return Ok(None);
    };
    if parent.is_empty() {
        return Ok(None);
    }
    VirtualPath::new(parent)
        .map(Some)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied))
}

pub(super) fn virtual_to_relative(
    root: &VirtualPath,
    path: &VirtualPath,
) -> Result<String, FirstPartyCapabilityError> {
    let target = root.as_str().trim_end_matches('/');
    let raw = path.as_str();
    if raw == target {
        return Ok(String::new());
    }
    raw.strip_prefix(&format!("{target}/"))
        .map(ToString::to_string)
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied))
}

pub(super) fn validate_relative_pattern(pattern: &str) -> Result<(), FirstPartyCapabilityError> {
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

pub(super) fn filesystem_error(error: FilesystemError) -> FirstPartyCapabilityError {
    match error {
        FilesystemError::Contract(_) => input_error(),
        FilesystemError::PermissionDenied { .. }
        | FilesystemError::MountNotFound { .. }
        | FilesystemError::PathOutsideMount { .. }
        | FilesystemError::SymlinkEscape { .. }
        | FilesystemError::MountConflict { .. } => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied)
        }
        FilesystemError::NotFound { .. } => guest_error(),
        FilesystemError::Backend { .. } => guest_error(),
    }
}

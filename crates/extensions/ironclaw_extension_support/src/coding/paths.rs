use ironclaw_filesystem::{DirEntry, FileStat, FilesystemError, FilesystemOperation};
use ironclaw_host_api::{
    dispatch::RuntimeDispatchErrorKind,
    path::{ScopedPath, VirtualPath},
};
use ironclaw_safety::sensitive_paths::is_sensitive_path_str;
use serde_json::Value;
use tracing::debug;

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
    // Name the offending path and the roots that DO exist: an agent that
    // targeted an out-of-scope absolute path (e.g. one copied verbatim from a
    // task description) can only correct course when the rejection says so.
    //
    // These summaries render paths and roots delimiter-free (`/` → space):
    // the strict loop safe-summary validator rejects raw path delimiters, and
    // FilesystemDenied surfaces as a Denied loop outcome whose only
    // model-visible channel is the summary itself — a raw-path summary would
    // silently collapse to the generic category sentence. Summaries that
    // still fail validation (hostile file names) ride the model-visible
    // diagnostic detail channel instead of the summary.
    let scoped_path = mounts
        .scoped_path(scoped_path_input(path))
        .map_err(|error| {
            debug!(error = %error, "coding capability rejected scoped path input");
            CodingCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::InputEncode,
                format!(
                    "{} is not under an available scoped root (available roots: {})",
                    safe_summary_path(path),
                    available_roots(mounts)
                ),
            )
        })?;
    if is_sensitive_scoped_path(scoped_path.as_str()) {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    let (virtual_path, grant) = mounts.resolve_with_grant(&scoped_path).map_err(|error| {
        debug!(error = %error, "coding capability could not resolve scoped path");
        CodingCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::FilesystemDenied,
            format!(
                "{} does not resolve inside an available scoped root (available roots: {})",
                safe_summary_path(path),
                available_roots(mounts)
            ),
        )
    })?;
    if is_sensitive_resolved_path(&virtual_path) {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    if !operation_allowed(&grant.permissions, operation) {
        // Name the root, the writable alternatives, and -- only for a skill root -- the tool that
        // owns writes there. The bare denial said just "the tool was denied filesystem access", so an
        // agent editing its own skill had no idea what to do and fell back to remove-then-reinstall.
        //
        // `/skills` is deliberately read-only for the filesystem tools: writes go through
        // `skill_install`/`skill_update`, which validate the manifest discovery requires. Suggesting
        // those tools for a read-only WORKSPACE would be nonsense, which a test caught.
        if grant.permissions.read && !grant.permissions.write {
            let writable = writable_roots(mounts);
            let mut reason = format!("{} does not permit writes", safe_summary_path(path));
            if !writable.is_empty() {
                reason.push_str(&format!(" (writable roots: {writable})"));
            }
            if is_skill_alias(&grant) {
                reason.push_str(
                    ". Skills change through skill_install or skill_update, which validate the \
                     manifest that discovery requires",
                );
            }
            return Err(CodingCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::FilesystemDenied,
                reason,
            ));
        }
        return Err(CodingCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::FilesystemDenied,
            format!(
                "the mount for {} does not permit this operation",
                safe_summary_path(path)
            ),
        ));
    }
    Ok(ResolvedPath {
        scoped_path,
        virtual_path,
        grant: grant.clone(),
    })
}

fn available_roots(mounts: &ironclaw_host_api::mount::MountView) -> String {
    let mut roots: Vec<String> = mounts
        .mounts
        .iter()
        // Aliases are absolute ("/workspace"); render them without the
        // leading delimiter so the summary stays loop-safe.
        .map(|mount| safe_summary_path_text(mount.alias.as_str()))
        .collect();
    roots.sort_unstable();
    roots.join(", ")
}

/// The roots this caller may actually write to, for a denial that tells the agent where to go.
fn writable_roots(mounts: &ironclaw_host_api::mount::MountView) -> String {
    let mut roots: Vec<String> = mounts
        .mounts
        .iter()
        .filter(|mount| mount.permissions.write)
        .map(|mount| safe_summary_path_text(mount.alias.as_str()))
        .collect();
    roots.sort_unstable();
    roots.join(", ")
}

/// Is this grant one of the skill roots, whose writes belong to the skill capabilities?
fn is_skill_alias(grant: &ironclaw_host_api::mount::MountGrant) -> bool {
    grant.alias.as_str().ends_with("skills")
}

/// The mount aliases an agent may address, as directory entries.
///
/// `list_dir "/"` used to fail with `path  is not under an available scoped root` -- the offending
/// path rendered blank because the safe-summary encoder maps `/` to a space, so the message named
/// nothing at all. The agent was doing something reasonable: asking what the filesystem contains
/// before writing to it. The roots are exactly the answer, and they were already being computed for
/// the error text.
pub(super) fn root_alias_entries(mounts: &ironclaw_host_api::mount::MountView) -> Vec<String> {
    let mut roots: Vec<String> = mounts
        .mounts
        .iter()
        .map(|mount| format!("{}/", mount.alias.as_str()))
        .collect();
    roots.sort_unstable();
    roots.dedup();
    roots
}

/// Is this an attempt to address the filesystem root itself?
pub(super) fn is_filesystem_root_request(path: &str) -> bool {
    matches!(path.trim(), "/" | "//")
}

fn scoped_path_input(path: &str) -> String {
    if path == "." || path.is_empty() {
        DEFAULT_SCOPED_ROOT.to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else if let Some(scoped_workspace_path) = workspace_scoped_alias(path) {
        scoped_workspace_path
    } else {
        let relative = path.trim_start_matches("./");
        format!("{DEFAULT_SCOPED_ROOT}/{relative}")
    }
}

fn workspace_scoped_alias(path: &str) -> Option<String> {
    let path = strip_leading_current_dir_segments(path);
    if path == "workspace" {
        return Some(DEFAULT_SCOPED_ROOT.to_string());
    }

    path.strip_prefix("workspace/")
        .map(|relative| relative.trim_start_matches('/'))
        .map(|relative| {
            if relative.is_empty() {
                DEFAULT_SCOPED_ROOT.to_string()
            } else {
                format!("{DEFAULT_SCOPED_ROOT}/{relative}")
            }
        })
}

fn strip_leading_current_dir_segments(mut path: &str) -> &str {
    while let Some(stripped) = path.strip_prefix("./") {
        path = stripped;
    }
    path
}

pub(super) fn operation_allowed(
    permissions: &ironclaw_host_api::mount::MountPermissions,
    operation: FilesystemOperation,
) -> bool {
    match operation {
        FilesystemOperation::ReadFile => permissions.read,
        FilesystemOperation::WriteFile
        | FilesystemOperation::AppendFile
        | FilesystemOperation::CreateSubtreeAtomic => permissions.write,
        FilesystemOperation::ListDir => permissions.list,
        FilesystemOperation::Stat => permissions.read || permissions.list,
        FilesystemOperation::Delete => permissions.delete,
        FilesystemOperation::CreateDirAll => permissions.write,
        FilesystemOperation::MountLocal | FilesystemOperation::Connect => false,
        // Coding tools never use the unified record/index/txn/event surface
        // — they are bytes-only. If a future code path routes here, treat
        // record-plane reads as `read` and writes as `write` to stay
        // fail-closed. `Append` (event-plane append) is distinct from
        // `AppendFile` (byte-plane append onto a regular file) but both
        // map to `permissions.write`.
        FilesystemOperation::Query => permissions.read && permissions.list,
        FilesystemOperation::EnsureIndex
        | FilesystemOperation::BeginTxn
        | FilesystemOperation::Append
        | FilesystemOperation::ReserveSeq => permissions.write,
        FilesystemOperation::Tail | FilesystemOperation::HeadSeq => permissions.read,
    }
}

/// List a directory during a walk rooted at `root`, reading an
/// authorized-but-never-written mount ROOT as empty.
///
/// The caller's grant names the root, so "the backend has nothing under it
/// yet" means an empty directory, not an error — without this, every read
/// tool on a brand-new per-caller workspace failed before its first write.
/// Only the grant's own target gets this treatment; any deeper directory
/// keeps reporting `NotFound`.
pub(super) async fn list_dir_empty_if_missing_root(
    request: &CodingCapabilityRequest<'_>,
    root: &ResolvedPath,
    dir: &VirtualPath,
) -> Result<Vec<DirEntry>, CodingCapabilityError> {
    match request.filesystem.list_dir(dir).await {
        Err(FilesystemError::NotFound { .. }) if dir == &root.grant.target => Ok(Vec::new()),
        other => other.map_err(filesystem_error),
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

/// Walk up the directory tree, denying if any existing parent is sensitive.
///
/// Best-effort check for the standalone threat model: assumes a trusted filesystem
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

pub(super) fn filesystem_error_with_summary(
    operation: &str,
    scoped_path: &str,
    error: FilesystemError,
) -> CodingCapabilityError {
    let scoped_path = safe_summary_path(scoped_path);
    let summary = match &error {
        FilesystemError::NotFound { .. } => {
            format!("{operation} failed for {scoped_path}: file not found")
        }
        FilesystemError::PermissionDenied { .. }
        | FilesystemError::MountNotFound { .. }
        | FilesystemError::PathOutsideMount { .. }
        | FilesystemError::SymlinkEscape { .. }
        | FilesystemError::MountConflict { .. }
        | FilesystemError::VersionMismatch { .. }
        | FilesystemError::Unsupported { .. }
        | FilesystemError::IndexConflict { .. } => {
            format!("{operation} failed for {scoped_path}: permission denied or unsupported path")
        }
        FilesystemError::Backend { .. } | FilesystemError::BackendInfrastructure { .. } => {
            format!("{operation} failed for {scoped_path}: filesystem backend error")
        }
        FilesystemError::Contract(_) => {
            format!("{operation} failed for {scoped_path}: invalid path")
        }
        _ => format!("{operation} failed for {scoped_path}: filesystem error"),
    };
    let kind = filesystem_error(error).kind();
    CodingCapabilityError::with_safe_summary(kind, summary)
}

pub(super) fn safe_summary_path(scoped_path: &str) -> String {
    // The strict loop summary validator bans path delimiters. Keep all coding
    // tool path hints on this single renderer so resolution and filesystem
    // failures cannot drift into different redaction behavior.
    format!("path {}", safe_summary_path_text(scoped_path))
}

fn safe_summary_path_text(path: &str) -> String {
    path.trim_start_matches('/').replace(['/', '\\'], " ")
}

#[cfg(test)]
mod root_listing_tests {
    use super::*;
    use ironclaw_host_api::{
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };

    fn view() -> MountView {
        MountView::new(vec![
            MountGrant::new(
                MountAlias::new("/workspace").expect("alias"),
                VirtualPath::new("/projects/workspace").expect("target"),
                MountPermissions::read_write(),
            ),
            MountGrant::new(
                MountAlias::new("/skills").expect("alias"),
                VirtualPath::new("/tenants/t/users/u/skills").expect("target"),
                MountPermissions::read_only(),
            ),
        ])
        .expect("view")
    }

    /// `list_dir "/"` must answer with the roots, not an error naming nothing.
    ///
    /// The failure read `path  is not under an available scoped root` -- blank, because the
    /// safe-summary encoder maps `/` to a space. The agent was asking what the filesystem contains
    /// before writing to it, and the roots were already computed for that very error message.
    #[test]
    fn the_filesystem_root_lists_the_available_roots() {
        assert!(is_filesystem_root_request("/"));
        assert!(is_filesystem_root_request(" / "));
        assert!(!is_filesystem_root_request("/workspace"));
        assert!(!is_filesystem_root_request(""));

        assert_eq!(
            root_alias_entries(&view()),
            vec!["/skills/".to_string(), "/workspace/".to_string()],
            "sorted, one entry per grant, so an agent can see where it may work"
        );
    }
}

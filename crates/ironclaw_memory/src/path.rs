//! Memory path grammar, scope, and validation.

use std::sync::OnceLock;

use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::{HostApiError, VirtualPath};

/// Tenant/user/agent/project scope for DB-backed memory documents exposed as virtual files.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemoryDocumentScope {
    pub(crate) tenant_id: String,
    pub(crate) user_id: String,
    pub(crate) agent_id: Option<String>,
    pub(crate) project_id: Option<String>,
}

impl MemoryDocumentScope {
    pub fn new(
        tenant_id: impl Into<String>,
        user_id: impl Into<String>,
        project_id: Option<&str>,
    ) -> Result<Self, HostApiError> {
        Self::new_with_agent(tenant_id, user_id, None, project_id)
    }

    pub fn new_with_agent(
        tenant_id: impl Into<String>,
        user_id: impl Into<String>,
        agent_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<Self, HostApiError> {
        let tenant_id = validated_memory_segment("memory tenant", tenant_id.into())?;
        let user_id = validated_memory_segment("memory user", user_id.into())?;
        let agent_id = agent_id
            .map(|agent_id| validated_memory_segment("memory agent", agent_id.to_string()))
            .transpose()?;
        if agent_id.as_deref() == Some("_none") {
            return Err(HostApiError::InvalidId {
                kind: "memory agent",
                value: "_none".to_string(),
                reason: "_none is reserved for absent agent ids".to_string(),
            });
        }
        let project_id = project_id
            .map(|project_id| validated_memory_segment("memory project", project_id.to_string()))
            .transpose()?;
        if project_id.as_deref() == Some("_none") {
            return Err(HostApiError::InvalidId {
                kind: "memory project",
                value: "_none".to_string(),
                reason: "_none is reserved for absent project ids".to_string(),
            });
        }
        Ok(Self {
            tenant_id,
            user_id,
            agent_id,
            project_id,
        })
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn agent_id(&self) -> Option<&str> {
        self.agent_id.as_deref()
    }

    pub fn project_id(&self) -> Option<&str> {
        self.project_id.as_deref()
    }

    pub(crate) fn virtual_prefix(&self) -> Result<VirtualPath, HostApiError> {
        VirtualPath::new(format!(
            "/memory/tenants/{}/users/{}/agents/{}/projects/{}",
            self.tenant_id,
            self.user_id,
            self.agent_id.as_deref().unwrap_or("_none"),
            self.project_id.as_deref().unwrap_or("_none")
        ))
    }
}

/// File-shaped memory document key inside the memory document repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemoryDocumentPath {
    pub(crate) scope: MemoryDocumentScope,
    pub(crate) relative_path: String,
}

impl MemoryDocumentPath {
    pub fn new(
        tenant_id: impl Into<String>,
        user_id: impl Into<String>,
        project_id: Option<&str>,
        relative_path: impl Into<String>,
    ) -> Result<Self, HostApiError> {
        Self::new_with_agent(tenant_id, user_id, None, project_id, relative_path)
    }

    pub fn new_with_agent(
        tenant_id: impl Into<String>,
        user_id: impl Into<String>,
        agent_id: Option<&str>,
        project_id: Option<&str>,
        relative_path: impl Into<String>,
    ) -> Result<Self, HostApiError> {
        let scope = MemoryDocumentScope::new_with_agent(tenant_id, user_id, agent_id, project_id)?;
        let relative_path = validated_memory_relative_path(relative_path.into())?;
        Ok(Self {
            scope,
            relative_path,
        })
    }

    pub fn scope(&self) -> &MemoryDocumentScope {
        &self.scope
    }

    pub fn tenant_id(&self) -> &str {
        self.scope.tenant_id()
    }

    pub fn user_id(&self) -> &str {
        self.scope.user_id()
    }

    pub fn agent_id(&self) -> Option<&str> {
        self.scope.agent_id()
    }

    pub fn project_id(&self) -> Option<&str> {
        self.scope.project_id()
    }

    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub(crate) fn virtual_path(&self) -> Result<VirtualPath, HostApiError> {
        VirtualPath::new(format!(
            "{}/{}",
            self.scope.virtual_prefix()?.as_str(),
            self.relative_path
        ))
    }
}

pub(crate) struct ParsedMemoryPath {
    pub(crate) scope: MemoryDocumentScope,
    pub(crate) relative_path: Option<String>,
}

impl ParsedMemoryPath {
    pub(crate) fn from_virtual_path(
        path: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<Self, FilesystemError> {
        let segments: Vec<&str> = path.as_str().trim_matches('/').split('/').collect();
        if segments.len() < 7
            || segments.first() != Some(&"memory")
            || segments.get(1) != Some(&"tenants")
            || segments.get(3) != Some(&"users")
        {
            return Err(memory_error(
                path.clone(),
                operation,
                "expected /memory/tenants/{tenant}/users/{user}/agents/{agent}/projects/{project}/{path}",
            ));
        }

        let tenant_id = *segments.get(2).ok_or_else(|| {
            memory_error(path.clone(), operation, "memory tenant segment is missing")
        })?;
        let user_id = *segments.get(4).ok_or_else(|| {
            memory_error(path.clone(), operation, "memory user segment is missing")
        })?;

        let (agent_id, raw_project_id, relative_start) = if segments.get(5) == Some(&"agents") {
            if segments.len() < 9 || segments.get(7) != Some(&"projects") {
                return Err(memory_error(
                    path.clone(),
                    operation,
                    "expected /memory/tenants/{tenant}/users/{user}/agents/{agent}/projects/{project}/{path}",
                ));
            }
            let raw_agent_id = *segments.get(6).ok_or_else(|| {
                memory_error(path.clone(), operation, "memory agent segment is missing")
            })?;
            let agent_id = if raw_agent_id == "_none" {
                None
            } else {
                Some(raw_agent_id)
            };
            let raw_project_id = *segments.get(8).ok_or_else(|| {
                memory_error(path.clone(), operation, "memory project segment is missing")
            })?;
            (agent_id, raw_project_id, 9)
        } else if segments.get(5) == Some(&"projects") {
            let raw_project_id = *segments.get(6).ok_or_else(|| {
                memory_error(path.clone(), operation, "memory project segment is missing")
            })?;
            (None, raw_project_id, 7)
        } else {
            return Err(memory_error(
                path.clone(),
                operation,
                "expected /memory/tenants/{tenant}/users/{user}/agents/{agent}/projects/{project}/{path}",
            ));
        };

        let project_id = if raw_project_id == "_none" {
            None
        } else {
            Some(raw_project_id)
        };
        let scope = MemoryDocumentScope::new_with_agent(tenant_id, user_id, agent_id, project_id)
            .map_err(|error| {
            memory_error(
                path.clone(),
                operation,
                format!("invalid memory document scope: {error}"),
            )
        })?;
        let relative_path = if segments.len() > relative_start {
            Some(
                validated_memory_relative_path(segments[relative_start..].join("/")).map_err(
                    |error| {
                        memory_error(
                            path.clone(),
                            operation,
                            format!("invalid memory document path: {error}"),
                        )
                    },
                )?,
            )
        } else {
            None
        };

        Ok(Self {
            scope,
            relative_path,
        })
    }
}

pub(crate) fn validated_memory_segment(
    kind: &'static str,
    value: String,
) -> Result<String, HostApiError> {
    if value.trim().is_empty() {
        return Err(HostApiError::InvalidId {
            kind,
            value,
            reason: "segment must not be empty".to_string(),
        });
    }
    if value.len() > 256 {
        return Err(HostApiError::InvalidId {
            kind,
            value,
            reason: "segment must be at most 256 bytes".to_string(),
        });
    }
    if value == "." || value == ".." {
        return Err(HostApiError::InvalidId {
            kind,
            value,
            reason: "dot segments are not allowed".to_string(),
        });
    }
    if value.contains(':') {
        return Err(HostApiError::InvalidId {
            kind,
            value,
            reason: "colon is reserved for memory owner key encoding".to_string(),
        });
    }
    if value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
        || value.chars().any(char::is_control)
    {
        return Err(HostApiError::InvalidId {
            kind,
            value,
            reason: "segment must not contain path separators or control characters".to_string(),
        });
    }
    Ok(value)
}

pub(crate) fn validated_memory_relative_path(value: String) -> Result<String, HostApiError> {
    if value.trim().is_empty() {
        return Err(HostApiError::InvalidPath {
            value,
            reason: "memory document path must not be empty".to_string(),
        });
    }
    if value.starts_with('/') || value.contains('\\') || value.contains('\0') {
        return Err(HostApiError::InvalidPath {
            value,
            reason: "memory document path must be relative and use forward slashes".to_string(),
        });
    }
    if value.chars().any(char::is_control) {
        return Err(HostApiError::InvalidPath {
            value,
            reason: "memory document path must not contain control characters".to_string(),
        });
    }
    if value
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(HostApiError::InvalidPath {
            value,
            reason: "memory document path must not contain empty, '.', or '..' segments"
                .to_string(),
        });
    }
    // PR #3679 review fix (finding #5): the repository writes metadata for
    // document `foo` at `foo.meta`, chunks under `foo.chunks/<n>.json`, and
    // version archives under `foo.versions/<n>.json`. Without this check a
    // legal user document literally named `foo.meta` (or any segment
    // ending in `.chunks` / `.versions`) would share the backend path with
    // those sidecars, so writing metadata for `foo` would overwrite the
    // document `foo.meta` with JSON bytes. Reject the reserved suffixes at
    // path validation so the sidecar/document namespaces stay disjoint.
    for segment in value.split('/') {
        if segment.ends_with(".meta")
            || segment.ends_with(".chunks")
            || segment.ends_with(".versions")
        {
            return Err(HostApiError::InvalidPath {
                value,
                reason:
                    "memory document path segments must not end with `.meta`, `.chunks`, or `.versions` (reserved for sidecars)"
                        .to_string(),
            });
        }
    }
    Ok(value)
}

pub(crate) fn memory_backend_unsupported(
    scope: &MemoryDocumentScope,
    operation: FilesystemOperation,
    reason: impl Into<String>,
) -> FilesystemError {
    memory_error(
        scope
            .virtual_prefix()
            .unwrap_or_else(|_| valid_memory_path()),
        operation,
        reason,
    )
}

pub(crate) fn memory_not_found(
    path: VirtualPath,
    operation: FilesystemOperation,
) -> FilesystemError {
    memory_error(path, operation, "not found")
}

pub(crate) fn memory_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    reason: impl Into<String>,
) -> FilesystemError {
    let reason = sanitize_memory_backend_reason(reason.into());
    FilesystemError::Backend {
        path,
        operation,
        reason,
    }
}

const MEMORY_BACKEND_DETAIL_MARKERS: &[&str] = &[
    "no such table",
    "drop table",
    "sql",
    "sqlite",
    "libsql",
    "postgres error",
    "database error",
    "connection refused",
    "timeout",
    "host=",
    "port=",
    "reborn_memory_",
    "/tmp/",
    "/var/folders/",
    "/private/",
    "\\appdata\\",
];

fn sanitize_memory_backend_reason(reason: String) -> String {
    let lower = reason.to_ascii_lowercase();
    if MEMORY_BACKEND_DETAIL_MARKERS
        .iter()
        .any(|marker| lower.as_str().contains(marker))
    {
        "memory backend operation failed".to_string()
    } else {
        reason
    }
}

pub(crate) fn valid_memory_path() -> VirtualPath {
    static MEMORY_PATH: OnceLock<VirtualPath> = OnceLock::new();
    // safety: `/memory` is a registered VIRTUAL_ROOT in ironclaw_host_api::path.
    // If construction fails, host_api's VIRTUAL_ROOTS list is out of sync with
    // this crate at build time, which is a build-system invariant violation.
    MEMORY_PATH
        .get_or_init(|| VirtualPath::new("/memory").expect("/memory is a registered VIRTUAL_ROOT")) // safety: `/memory` is a registered VIRTUAL_ROOT.
        .clone()
}

#[cfg(test)]
mod path_validation_tests {
    use super::validated_memory_relative_path;

    /// PR #3679 review fix (finding #5): legal user document paths must
    /// not collide with the repository's sidecar suffix namespace.
    #[test]
    fn rejects_path_segments_ending_in_reserved_sidecar_suffixes() {
        for reserved in [
            "foo.meta",
            "subdir/foo.meta",
            "data.chunks",
            "data.chunks/inner",
            "history.versions",
            "history.versions/2",
        ] {
            let err = validated_memory_relative_path(reserved.to_string())
                .expect_err(reserved);
            let msg = format!("{err}");
            assert!(
                msg.contains(".meta") || msg.contains(".chunks") || msg.contains(".versions"),
                "expected reserved-suffix rejection in error: {msg}"
            );
        }
    }

    #[test]
    fn accepts_non_reserved_paths_with_dots_in_names() {
        for ok in [
            "foo.md",
            "subdir/foo.txt",
            "metadata-foo",
            "chunks-of-bread",
            "version-1.txt",
        ] {
            validated_memory_relative_path(ok.to_string())
                .expect("non-reserved path must be accepted");
        }
    }
}

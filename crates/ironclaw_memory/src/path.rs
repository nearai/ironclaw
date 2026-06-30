//! Memory path grammar, scope, and validation.

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

    pub fn virtual_prefix(&self) -> Result<VirtualPath, HostApiError> {
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

    /// Build a path from an already-validated [`MemoryDocumentScope`] plus a
    /// relative path that is validated here. The scope is a validated newtype,
    /// so only the relative path needs re-checking; this keeps the public
    /// constructor from ever producing a `MemoryDocumentPath` with traversal,
    /// control characters, or reserved-sidecar segments, even if a caller in
    /// another crate passes an unchecked path.
    pub fn from_scope(
        scope: MemoryDocumentScope,
        relative_path: impl Into<String>,
    ) -> Result<Self, HostApiError> {
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

    pub fn virtual_path(&self) -> Result<VirtualPath, HostApiError> {
        VirtualPath::new(format!(
            "{}/{}",
            self.scope.virtual_prefix()?.as_str(),
            self.relative_path
        ))
    }
}

pub fn validated_memory_segment(kind: &'static str, value: String) -> Result<String, HostApiError> {
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

pub fn validated_memory_relative_path(value: String) -> Result<String, HostApiError> {
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
            let err = validated_memory_relative_path(reserved.to_string()).expect_err(reserved);
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

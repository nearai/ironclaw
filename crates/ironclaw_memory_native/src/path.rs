//! Memory path grammar, scope, and validation.
//!
//! The public scope/path value types and segment/relative-path validators moved
//! to `ironclaw_memory` and are re-exported below. The repository-facing
//! virtual-path parser and the sanitized backend-error helpers stay here because
//! they depend on `ironclaw_filesystem`.

use std::sync::OnceLock;

use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::VirtualPath;

pub use ironclaw_memory::{
    MemoryDocumentPath, MemoryDocumentScope, validated_memory_relative_path,
};

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
    "/workspace/",
    "/home/",
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

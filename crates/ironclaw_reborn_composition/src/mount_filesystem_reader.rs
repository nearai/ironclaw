//! Caller-scoped, read-only multi-mount filesystem browser for the WebUI v2
//! facade.
//!
//! Implements [`FilesystemBrowseReader`] — the port the standalone "Workspace /
//! Files" viewer calls to navigate the agent's internal filesystem (persistent
//! memory + project working files, which include landed attachments). It reads
//! through a single read-only [`ScopedFilesystem`] whose mount view spans every
//! browsable alias (see [`browse_mount_view`](crate::local_dev_mounts)). A
//! [`FsMount`] selects which alias to confine to; paths in and out are
//! mount-relative so neither an alias nor a host path crosses the boundary.
//!
//! This deliberately reuses the project-filesystem reader's substrate→port
//! mapping helpers (kind/mime/error mapping, sensitive-name filtering, readable
//! guard) rather than re-deriving them, so list/stat/read stay consistent on
//! what is reachable across both surfaces.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_attachments::DEFAULT_MAX_ATTACHMENT_BYTES;
use ironclaw_filesystem::{DirEntry, FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use ironclaw_product_workflow::{
    FilesystemBrowseReader, FsMount, ProjectFsEntry, ProjectFsError, ProjectFsFile, ProjectFsStat,
};

use crate::local_dev_mounts::{BROWSE_MEMORY_ALIAS, WORKSPACE_ALIAS};
use crate::project_filesystem_reader::{
    file_name_of, guard_readable_file, map_filesystem_error, map_kind, mime_for_path,
};

/// Browses the agent's internal filesystem across mounts on behalf of an
/// already-authorized caller, over a read-only scoped filesystem.
pub(crate) struct MountScopedFilesystemReader<F: RootFilesystem> {
    filesystem: Arc<ScopedFilesystem<F>>,
    /// Upper bound on a single download — shares the inbound attachment ceiling
    /// so the viewer and the agent's tools observe the same 25 MiB limit.
    max_read_bytes: u64,
}

impl<F: RootFilesystem> MountScopedFilesystemReader<F> {
    pub(crate) fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            max_read_bytes: DEFAULT_MAX_ATTACHMENT_BYTES as u64,
        }
    }

    /// The mount alias this reader confines to, or `None` for a mount this
    /// composition does not serve (e.g. `Skills`, pending a scope-resolved
    /// skills mount). `None` is what keeps an unwired mount a clean 404 at the
    /// facade rather than a backend error.
    fn alias_for(mount: FsMount) -> Option<&'static str> {
        match mount {
            FsMount::Memory => Some(BROWSE_MEMORY_ALIAS),
            FsMount::Workspace => Some(WORKSPACE_ALIAS),
            FsMount::Skills => None,
        }
    }

    /// Compose the alias-scoped path from a mount-relative request path.
    ///
    /// The relative path is trimmed of surrounding slashes; `""`/`"/"` resolves
    /// to the mount root. `ScopedPath::new` rejects `..` traversal, raw host
    /// paths, and URLs, so the composed path can never escape the alias.
    fn scoped_path(alias: &str, relative: &str) -> Result<ScopedPath, ProjectFsError> {
        let trimmed = relative.trim_matches('/');
        let composed = if trimmed.is_empty() {
            alias.to_string()
        } else {
            format!("{alias}/{trimmed}")
        };
        ScopedPath::new(composed).map_err(|_| ProjectFsError::InvalidPath)
    }

    /// Strip the alias prefix from a scoped path, yielding the mount-relative
    /// path the browser round-trips. The mount root maps to `""`.
    fn relativize(alias: &str, scoped: &str) -> String {
        scoped
            .strip_prefix(alias)
            .map(|rest| rest.trim_start_matches('/').to_string())
            .unwrap_or_else(|| scoped.trim_start_matches('/').to_string())
    }

    fn require_alias(mount: FsMount) -> Result<&'static str, ProjectFsError> {
        Self::alias_for(mount).ok_or(ProjectFsError::NotFound)
    }
}

#[async_trait]
impl<F: RootFilesystem> FilesystemBrowseReader for MountScopedFilesystemReader<F> {
    fn available_mounts(&self) -> Vec<FsMount> {
        // Report in the product layer's display order, keeping only mounts this
        // composition actually serves.
        FsMount::ALL
            .iter()
            .copied()
            .filter(|mount| Self::alias_for(*mount).is_some())
            .collect()
    }

    async fn list_dir(
        &self,
        scope: &ResourceScope,
        mount: FsMount,
        path: &str,
    ) -> Result<Vec<ProjectFsEntry>, ProjectFsError> {
        let alias = Self::require_alias(mount)?;
        let dir = Self::scoped_path(alias, path)?;
        let is_root = path.trim_matches('/').is_empty();
        let entries = list_dir_or_empty_root(self.filesystem.list_dir(scope, &dir).await, is_root)?;
        let scoped_base = dir.as_str().trim_end_matches('/');
        Ok(entries
            .into_iter()
            // `DirEntry` carries no `sensitive` flag (only stat/read see it), so
            // listing filters sensitive names itself — otherwise a listing would
            // enumerate secret filenames (`.env`, `id_rsa`) whose bytes stay
            // denied. Keeps list/stat/read consistent on what is reachable.
            .filter_map(|entry: DirEntry| {
                let scoped_child = format!("{scoped_base}/{}", entry.name);
                if is_internal_browse_path(&scoped_child)
                    || ironclaw_safety::sensitive_paths::is_sensitive_path_str(&scoped_child)
                {
                    return None;
                }
                let path = Self::relativize(alias, &scoped_child);
                Some(ProjectFsEntry {
                    path,
                    name: entry.name,
                    kind: map_kind(entry.file_type),
                })
            })
            .collect())
    }

    async fn read_file(
        &self,
        scope: &ResourceScope,
        mount: FsMount,
        path: &str,
    ) -> Result<ProjectFsFile, ProjectFsError> {
        let alias = Self::require_alias(mount)?;
        let file = Self::scoped_path(alias, path)?;
        if is_internal_browse_path(file.as_str()) {
            return Err(ProjectFsError::Denied);
        }
        let stat = self
            .filesystem
            .stat(scope, &file)
            .await
            .map_err(map_filesystem_error)?;
        guard_readable_file(&stat, self.max_read_bytes)?;
        // Enforce the size cap at read time too: a concurrent write can grow the
        // file between the guard and the read, so the bounded read is what
        // actually prevents oversized content from being materialized.
        let bytes = self
            .filesystem
            .read_bytes_bounded(scope, &file, self.max_read_bytes as usize)
            .await
            .map_err(map_filesystem_error)?
            .ok_or(ProjectFsError::TooLarge {
                size: self.max_read_bytes.saturating_add(1),
                max: self.max_read_bytes,
            })?;
        let scoped_str = file.as_str().to_string();
        let mime_type = mime_for_path(&scoped_str);
        let filename = file_name_of(&scoped_str);
        Ok(ProjectFsFile {
            size_bytes: bytes.len() as u64,
            path: Self::relativize(alias, &scoped_str),
            filename,
            mime_type,
            bytes,
        })
    }

    async fn stat(
        &self,
        scope: &ResourceScope,
        mount: FsMount,
        path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError> {
        let alias = Self::require_alias(mount)?;
        let target = Self::scoped_path(alias, path)?;
        if is_internal_browse_path(target.as_str()) {
            return Err(ProjectFsError::Denied);
        }
        let stat = self
            .filesystem
            .stat(scope, &target)
            .await
            .map_err(map_filesystem_error)?;
        if stat.sensitive {
            return Err(ProjectFsError::Denied);
        }
        let scoped_str = target.as_str().to_string();
        Ok(ProjectFsStat {
            kind: map_kind(stat.file_type),
            size_bytes: stat.len,
            mime_type: mime_for_path(&scoped_str),
            path: Self::relativize(alias, &scoped_str),
        })
    }
}

/// Internal/hidden paths the browser must never expose: any path segment that
/// begins with "." (`.system/` engine state, `.git/`, dotfiles). The UI hides
/// these too, but this is the authoritative backend gate so a direct `/fs/*`
/// request cannot read engine internals out from under the cosmetic UI filter.
/// Memory/identity markdown (AGENTS.md, SOUL.md, USER.md, MEMORY.md, …) has no
/// leading dot and stays visible — surfacing it is the point of the memory view.
fn is_internal_browse_path(scoped: &str) -> bool {
    scoped.split('/').any(|segment| segment.starts_with('.'))
}

/// Resolve a directory listing, treating a `NotFound` on the **mount root** as
/// an empty listing rather than a 404.
///
/// A mount that exists in the browse catalog but has never been written to has
/// no directory entry yet; some backends (libsql) return `NotFound` for such a
/// root listing while others (in-memory) return an empty vec. The viewer must
/// render an empty-but-valid mount, so the root case is normalized here. A
/// missing *subdirectory* (`is_root == false`) still surfaces `NotFound` so a
/// bad path is not silently blank.
fn list_dir_or_empty_root(
    result: Result<Vec<DirEntry>, FilesystemError>,
    is_root: bool,
) -> Result<Vec<DirEntry>, ProjectFsError> {
    match result {
        Ok(entries) => Ok(entries),
        Err(error) => {
            let mapped = map_filesystem_error(error);
            if is_root && mapped == ProjectFsError::NotFound {
                // silent-ok: list_dir on a browse mount root that was never
                // written returns NotFound on some backends (libsql); the viewer
                // must render an empty mount. Subpaths still propagate NotFound.
                Ok(Vec::new())
            } else {
                Err(mapped)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        AgentId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
        TenantId, UserId, VirtualPath,
    };

    fn browse_fs() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        // Mirror the production browse view: read-only workspace + memory.
        let view = MountView::new(vec![
            MountGrant::new(
                MountAlias::new(WORKSPACE_ALIAS).unwrap(),
                VirtualPath::new("/projects/workspace").unwrap(),
                MountPermissions::read_only(),
            ),
            MountGrant::new(
                MountAlias::new(BROWSE_MEMORY_ALIAS).unwrap(),
                VirtualPath::new("/memory").unwrap(),
                MountPermissions::read_only(),
            ),
        ])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            view,
        ))
    }

    fn rw_fs() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let view = MountView::new(vec![
            MountGrant::new(
                MountAlias::new(WORKSPACE_ALIAS).unwrap(),
                VirtualPath::new("/projects/workspace").unwrap(),
                MountPermissions::read_write(),
            ),
            MountGrant::new(
                MountAlias::new(BROWSE_MEMORY_ALIAS).unwrap(),
                VirtualPath::new("/memory").unwrap(),
                MountPermissions::read_write(),
            ),
        ])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            view,
        ))
    }

    fn scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-test").unwrap(),
            user_id: UserId::new("user-test").unwrap(),
            agent_id: Some(AgentId::new("agent-test").unwrap()),
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    #[tokio::test]
    async fn available_mounts_excludes_unwired_skills() {
        let reader = MountScopedFilesystemReader::new(browse_fs());
        let mounts = reader.available_mounts();
        assert_eq!(mounts, vec![FsMount::Memory, FsMount::Workspace]);
    }

    #[tokio::test]
    async fn reads_memory_with_mount_relative_paths() {
        // Seed through a read-write view sharing the same backend root, then
        // read back through the read-only reader.
        let rw = rw_fs();
        let scope = scope();
        rw.write_bytes(
            &scope,
            &ScopedPath::new(format!("{BROWSE_MEMORY_ALIAS}/daily/today.md")).unwrap(),
            b"# notes".to_vec(),
        )
        .await
        .expect("seed memory");

        let reader = MountScopedFilesystemReader {
            filesystem: rw,
            max_read_bytes: DEFAULT_MAX_ATTACHMENT_BYTES as u64,
        };

        let entries = reader
            .list_dir(&scope, FsMount::Memory, "")
            .await
            .expect("list memory root");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "daily");
        assert_eq!(entries[0].path, "daily");

        let file = reader
            .read_file(&scope, FsMount::Memory, "daily/today.md")
            .await
            .expect("read memory file");
        assert_eq!(file.bytes, b"# notes");
        assert_eq!(file.path, "daily/today.md");
        assert_eq!(file.filename.as_deref(), Some("today.md"));
    }

    #[tokio::test]
    async fn unwired_mount_is_not_found() {
        let reader = MountScopedFilesystemReader::new(browse_fs());
        let err = reader
            .list_dir(&scope(), FsMount::Skills, "")
            .await
            .expect_err("skills mount is unwired");
        assert_eq!(err, ProjectFsError::NotFound);
    }

    #[test]
    fn empty_mount_root_lists_as_empty_not_404() {
        use ironclaw_filesystem::FilesystemOperation;
        use ironclaw_host_api::VirtualPath;

        let not_found = || FilesystemError::NotFound {
            path: VirtualPath::new("/memory").unwrap(),
            operation: FilesystemOperation::ListDir,
        };

        // Mount root: a never-written mount lists as empty (the bug was a 404).
        assert_eq!(
            list_dir_or_empty_root(Err(not_found()), true).unwrap(),
            Vec::new(),
        );

        // A missing subdirectory still surfaces NotFound so a bad path is not
        // silently blank.
        assert_eq!(
            list_dir_or_empty_root(Err(not_found()), false).unwrap_err(),
            ProjectFsError::NotFound,
        );
    }

    #[tokio::test]
    async fn traversal_is_rejected() {
        let reader = MountScopedFilesystemReader::new(browse_fs());
        let err = reader
            .stat(&scope(), FsMount::Memory, "../workspace/secret")
            .await
            .expect_err("traversal must be rejected");
        assert_eq!(err, ProjectFsError::InvalidPath);
    }

    #[tokio::test]
    async fn internal_and_sensitive_paths_are_hidden_and_denied() {
        let rw = rw_fs();
        let scope = scope();
        // A regular memory file, a credential-bearing dotfile, and an engine
        // internals path under `.system/`.
        for (path, body) in [
            ("public.md", &b"# notes"[..]),
            (".env", b"SECRET=value"),
            (".system/engine/state.json", b"{}"),
        ] {
            rw.write_bytes(
                &scope,
                &ScopedPath::new(format!("{BROWSE_MEMORY_ALIAS}/{path}")).unwrap(),
                body.to_vec(),
            )
            .await
            .expect("seed file");
        }

        let reader = MountScopedFilesystemReader {
            filesystem: rw,
            max_read_bytes: DEFAULT_MAX_ATTACHMENT_BYTES as u64,
        };

        // Listing surfaces the public file but neither the dotfile nor `.system`.
        let names: Vec<String> = reader
            .list_dir(&scope, FsMount::Memory, "")
            .await
            .expect("list memory root")
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert!(names.contains(&"public.md".to_string()));
        assert!(!names.contains(&".env".to_string()));
        assert!(!names.contains(&".system".to_string()));

        // stat/read on internal paths are denied (not just hidden from listing).
        assert_eq!(
            reader
                .stat(&scope, FsMount::Memory, ".env")
                .await
                .expect_err("sensitive stat denied"),
            ProjectFsError::Denied,
        );
        assert_eq!(
            reader
                .read_file(&scope, FsMount::Memory, ".system/engine/state.json")
                .await
                .expect_err("internal read denied"),
            ProjectFsError::Denied,
        );
    }
}

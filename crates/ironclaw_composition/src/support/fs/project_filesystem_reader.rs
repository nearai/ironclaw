//! Project-scoped read-only filesystem access for the WebUI v2 facade.
//!
//! Implements the [`ProjectFilesystemReader`] port the facade calls for
//! directory listing and file download. It reads through the same
//! project-scoped workspace [`ScopedFilesystem`] the agent's file tools resolve
//! through (a read-only mount view), so a file the agent wrote at
//! `/workspace/report.csv` is downloadable here at the same scoped path. The
//! download side backs agent-produced attachments (an `AttachmentRef`'s
//! `storage_key` is exactly such a path), but nothing here is
//! attachment-specific.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_attachments::DEFAULT_MAX_ATTACHMENT_BYTES;
use ironclaw_filesystem::{
    DirEntry, FileStat, FileType, FilesystemError, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::ScopedPath;
use ironclaw_product_workflow::{
    ProjectFilesystemReader, ProjectFsEntry, ProjectFsEntryKind, ProjectFsError, ProjectFsFile,
    ProjectFsStat,
};
use ironclaw_threads::ThreadScope;

use crate::local_dev_mounts::WORKSPACE_ALIAS;

const DEFAULT_OCTET_STREAM: &str = "application/octet-stream";

/// Reads directory listings and file bytes from a project-scoped workspace
/// filesystem on behalf of an already-authorized caller.
pub(crate) struct ProjectScopedFilesystemReader<F: RootFilesystem> {
    filesystem: Arc<ScopedFilesystem<F>>,
    workspace_alias: String,
    /// Upper bound on a single download. Shares the inbound attachment ceiling
    /// so generated files and uploads observe the same 25 MiB limit.
    max_read_bytes: u64,
}

impl<F: RootFilesystem> ProjectScopedFilesystemReader<F> {
    pub(crate) fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            workspace_alias: WORKSPACE_ALIAS.to_string(),
            max_read_bytes: DEFAULT_MAX_ATTACHMENT_BYTES as u64,
        }
    }

    #[cfg(test)]
    fn with_max_read_bytes(filesystem: Arc<ScopedFilesystem<F>>, max_read_bytes: u64) -> Self {
        Self {
            filesystem,
            workspace_alias: WORKSPACE_ALIAS.to_string(),
            max_read_bytes,
        }
    }

    /// Parse and confine a caller-supplied path. `ScopedPath::new` rejects
    /// URLs, raw host paths, and `..` traversal; we additionally require the
    /// normalized path to stay under the `/workspace` alias so the read-only
    /// workspace mount is the only reachable surface.
    fn confine(&self, path: &str) -> Result<ScopedPath, ProjectFsError> {
        let scoped = ScopedPath::new(path).map_err(|_| ProjectFsError::InvalidPath)?;
        // Component-wise containment via `Path::strip_prefix` so a sibling like
        // `/workspaceother` cannot pass a naive string-prefix check. `Ok` covers
        // both the alias root itself and any descendant under it.
        let under_workspace = Path::new(scoped.as_str())
            .strip_prefix(Path::new(&self.workspace_alias))
            .is_ok();
        if !under_workspace {
            return Err(ProjectFsError::Denied);
        }
        Ok(scoped)
    }
}

#[async_trait]
impl<F: RootFilesystem> ProjectFilesystemReader for ProjectScopedFilesystemReader<F> {
    async fn list_dir(
        &self,
        thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<Vec<ProjectFsEntry>, ProjectFsError> {
        let scope = thread_scope.to_resource_scope();
        let dir = self.confine(path)?;
        let entries = self
            .filesystem
            .list_dir(&scope, &dir)
            .await
            .map_err(map_filesystem_error)?;
        let base = dir.as_str().trim_end_matches('/');
        Ok(entries
            .into_iter()
            // `DirEntry` carries no `sensitive` flag (only `stat`/`read_file`
            // see it), so listing must filter sensitive names itself — otherwise
            // a directory listing enumerates secret filenames (`.env`, `id_rsa`)
            // even though their bytes stay denied. Cheap string check, keeping
            // list/stat/read consistent on what is reachable.
            .filter_map(|entry: DirEntry| {
                let path = format!("{base}/{}", entry.name);
                if ironclaw_safety::sensitive_paths::is_sensitive_path_str(&path) {
                    return None;
                }
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
        thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<ProjectFsFile, ProjectFsError> {
        let scope = thread_scope.to_resource_scope();
        let file = self.confine(path)?;
        let stat = self
            .filesystem
            .stat(&scope, &file)
            .await
            .map_err(map_filesystem_error)?;
        guard_readable_file(&stat, self.max_read_bytes)?;
        // Enforce the size cap *at read time*, not just from the stat above: a
        // concurrent write can grow the file between the guard and the read, so
        // a bounded read is what actually prevents oversized content from being
        // materialized. `Ok(None)` means the realized body exceeded the cap
        // (a missing file surfaces as `Err(NotFound)`).
        let bytes = self
            .filesystem
            .read_bytes_bounded(&scope, &file, self.max_read_bytes as usize)
            .await
            .map_err(map_filesystem_error)?
            .ok_or(ProjectFsError::TooLarge {
                // The bounded read refuses without reporting the grown size; the
                // stat-time guard already passed, so this only fires on a TOCTOU
                // race. Report just over the cap rather than the stale stat size.
                size: self.max_read_bytes.saturating_add(1),
                max: self.max_read_bytes,
            })?;
        let path_str = file.as_str().to_string();
        let filename = file_name_of(&path_str);
        let mime_type = mime_for_path(&path_str);
        Ok(ProjectFsFile {
            size_bytes: bytes.len() as u64,
            path: path_str,
            filename,
            mime_type,
            bytes,
        })
    }

    async fn stat(
        &self,
        thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError> {
        let scope = thread_scope.to_resource_scope();
        let target = self.confine(path)?;
        let stat = self
            .filesystem
            .stat(&scope, &target)
            .await
            .map_err(map_filesystem_error)?;
        if stat.sensitive {
            return Err(ProjectFsError::Denied);
        }
        Ok(ProjectFsStat {
            path: target.as_str().to_string(),
            kind: map_kind(stat.file_type),
            size_bytes: stat.len,
            // Same extension-derived MIME the download serves as `Content-Type`,
            // so the WebUI can pick a preview representation before fetching.
            mime_type: mime_for_path(target.as_str()),
        })
    }
}

/// Reject anything that is not a downloadable, non-sensitive, in-budget regular
/// file before its bytes are materialized.
pub(crate) fn guard_readable_file(stat: &FileStat, max_bytes: u64) -> Result<(), ProjectFsError> {
    if stat.sensitive {
        return Err(ProjectFsError::Denied);
    }
    if stat.file_type != FileType::File {
        return Err(ProjectFsError::NotAFile);
    }
    if stat.len > max_bytes {
        return Err(ProjectFsError::TooLarge {
            size: stat.len,
            max: max_bytes,
        });
    }
    Ok(())
}

pub(crate) fn map_kind(file_type: FileType) -> ProjectFsEntryKind {
    match file_type {
        FileType::File => ProjectFsEntryKind::File,
        FileType::Directory => ProjectFsEntryKind::Directory,
        FileType::Symlink => ProjectFsEntryKind::Symlink,
        FileType::Other => ProjectFsEntryKind::Other,
    }
}

fn file_name_str(path: &str) -> Option<&str> {
    path.rsplit('/').find(|segment| !segment.is_empty())
}

pub(crate) fn file_name_of(path: &str) -> Option<String> {
    file_name_str(path).map(|segment| segment.to_string())
}

pub(crate) fn mime_for_path(path: &str) -> String {
    file_name_str(path)
        .and_then(|name| {
            name.rsplit_once('.')
                .and_then(|(_, ext)| ironclaw_common::mime_for_extension(ext))
        })
        .unwrap_or(DEFAULT_OCTET_STREAM)
        .to_string()
}

/// Map a substrate filesystem error to the sanitized port error. Host paths and
/// backend reasons never cross this boundary.
pub(crate) fn map_filesystem_error(error: FilesystemError) -> ProjectFsError {
    match error {
        FilesystemError::NotFound { .. } => ProjectFsError::NotFound,
        FilesystemError::PermissionDenied { .. }
        | FilesystemError::PathOutsideMount { .. }
        | FilesystemError::SymlinkEscape { .. } => ProjectFsError::Denied,
        // Caller-shaped validation (bad path shape, etc.) → 400.
        FilesystemError::Contract(_) => ProjectFsError::InvalidPath,
        // Server-side configuration, not a caller mistake: the workspace mount
        // isn't wired, or the mount can't serve this op. Surface as
        // retryable-unavailable (503) rather than blaming the caller with a 400.
        FilesystemError::MountNotFound { .. } | FilesystemError::Unsupported { .. } => {
            ProjectFsError::Unavailable
        }
        _ => ProjectFsError::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::SystemTime;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, TenantId,
        UserId, VirtualPath,
    };
    use ironclaw_threads::ThreadScope;

    fn workspace_fs(permissions: MountPermissions) -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new(WORKSPACE_ALIAS).unwrap(),
            VirtualPath::new("/projects/workspace").unwrap(),
            permissions,
        )])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            view,
        ))
    }

    fn thread_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant-test").unwrap(),
            agent_id: AgentId::new("agent-test").unwrap(),
            project_id: None,
            owner_user_id: Some(UserId::new("user-test").unwrap()),
            mission_id: None,
        }
    }

    async fn seed(
        fs: &ScopedFilesystem<InMemoryBackend>,
        scope: &ResourceScope,
        path: &str,
        bytes: &[u8],
    ) {
        fs.write_bytes(scope, &ScopedPath::new(path).unwrap(), bytes.to_vec())
            .await
            .expect("seed write through read-write workspace mount");
    }

    #[tokio::test]
    async fn lists_dir_and_reads_file_with_scoped_paths() {
        let fs = workspace_fs(MountPermissions::read_write());
        let scope = thread_scope().to_resource_scope();
        seed(&fs, &scope, "/workspace/report.csv", b"a,b,c").await;
        let reader = ProjectScopedFilesystemReader::new(Arc::clone(&fs));

        let entries = reader
            .list_dir(&thread_scope(), "/workspace")
            .await
            .expect("listing the workspace root succeeds");
        let report = entries
            .iter()
            .find(|entry| entry.name == "report.csv")
            .expect("seeded file appears in listing");
        assert_eq!(report.path, "/workspace/report.csv");
        assert_eq!(report.kind, ProjectFsEntryKind::File);

        let file = reader
            .read_file(&thread_scope(), "/workspace/report.csv")
            .await
            .expect("reading the seeded file succeeds");
        assert_eq!(file.bytes, b"a,b,c");
        assert_eq!(file.size_bytes, 5);
        assert_eq!(file.filename.as_deref(), Some("report.csv"));
        assert_eq!(file.mime_type, "text/csv");
    }

    #[tokio::test]
    async fn rejects_path_outside_workspace() {
        let reader =
            ProjectScopedFilesystemReader::new(workspace_fs(MountPermissions::read_only()));
        let err = reader
            .read_file(&thread_scope(), "/secrets/master.key")
            .await
            .expect_err("a path outside /workspace must be denied");
        assert_eq!(err, ProjectFsError::Denied);
    }

    #[tokio::test]
    async fn missing_file_is_not_found() {
        let reader =
            ProjectScopedFilesystemReader::new(workspace_fs(MountPermissions::read_only()));
        let err = reader
            .read_file(&thread_scope(), "/workspace/nope.txt")
            .await
            .expect_err("a missing file surfaces NotFound");
        assert_eq!(err, ProjectFsError::NotFound);
    }

    fn stat_with(file_type: FileType, len: u64, sensitive: bool) -> FileStat {
        FileStat {
            path: VirtualPath::new("/projects/workspace/x").unwrap(),
            file_type,
            len,
            modified: Some(SystemTime::UNIX_EPOCH),
            sensitive,
        }
    }

    #[tokio::test]
    async fn stat_reports_extension_mime_for_preview() {
        // The stat MIME drives the WebUI preview mode, so it must match the
        // download `Content-Type` (extension-derived; octet-stream when unknown).
        let fs = workspace_fs(MountPermissions::read_write());
        let scope = thread_scope().to_resource_scope();
        seed(&fs, &scope, "/workspace/report.csv", b"a,b,c").await;
        seed(&fs, &scope, "/workspace/blob.unknownext", b"...").await;
        let reader = ProjectScopedFilesystemReader::new(Arc::clone(&fs));

        let csv = reader
            .stat(&thread_scope(), "/workspace/report.csv")
            .await
            .expect("stat succeeds");
        assert_eq!(csv.mime_type, "text/csv");
        assert_eq!(csv.size_bytes, 5);

        let unknown = reader
            .stat(&thread_scope(), "/workspace/blob.unknownext")
            .await
            .expect("stat succeeds");
        assert_eq!(unknown.mime_type, DEFAULT_OCTET_STREAM);
    }

    #[tokio::test]
    async fn list_dir_omits_sensitive_filenames() {
        // A listing must not enumerate secret filenames even though their bytes
        // stay denied — otherwise it is a recon primitive.
        let fs = workspace_fs(MountPermissions::read_write());
        let scope = thread_scope().to_resource_scope();
        seed(&fs, &scope, "/workspace/report.csv", b"a,b,c").await;
        seed(&fs, &scope, "/workspace/.env", b"SECRET=1").await;
        seed(&fs, &scope, "/workspace/id_rsa", b"-----BEGIN").await;
        let reader = ProjectScopedFilesystemReader::new(Arc::clone(&fs));

        let names: Vec<String> = reader
            .list_dir(&thread_scope(), "/workspace")
            .await
            .expect("listing succeeds")
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert!(names.contains(&"report.csv".to_string()));
        assert!(!names.contains(&".env".to_string()), "{names:?}");
        assert!(!names.contains(&"id_rsa".to_string()), "{names:?}");
    }

    #[tokio::test]
    async fn confine_denies_workspace_prefix_sibling() {
        // `/workspace-other/...` shares the textual prefix `/workspace` but is a
        // different directory; component-wise confinement must deny it.
        let reader =
            ProjectScopedFilesystemReader::new(workspace_fs(MountPermissions::read_only()));
        let err = reader
            .read_file(&thread_scope(), "/workspace-other/x.txt")
            .await
            .expect_err("a sibling of the workspace alias must be denied");
        assert_eq!(err, ProjectFsError::Denied);
    }

    #[tokio::test]
    async fn read_file_rejects_oversize() {
        // The size cap denies an oversized read before its bytes reach the
        // caller. Locks the `TooLarge` path (and its facade 413 mapping
        // upstream) end-to-end through the reader.
        let fs = workspace_fs(MountPermissions::read_write());
        let scope = thread_scope().to_resource_scope();
        seed(&fs, &scope, "/workspace/big.bin", b"0123456789").await;
        let reader = ProjectScopedFilesystemReader::with_max_read_bytes(Arc::clone(&fs), 4);

        let err = reader
            .read_file(&thread_scope(), "/workspace/big.bin")
            .await
            .expect_err("a file over the cap must be denied");
        assert!(
            matches!(err, ProjectFsError::TooLarge { .. }),
            "expected TooLarge, got {err:?}"
        );
    }

    #[test]
    fn guard_rejects_sensitive_directory_and_oversize() {
        assert_eq!(
            guard_readable_file(&stat_with(FileType::File, 1, true), 10),
            Err(ProjectFsError::Denied)
        );
        assert_eq!(
            guard_readable_file(&stat_with(FileType::Directory, 1, false), 10),
            Err(ProjectFsError::NotAFile)
        );
        assert_eq!(
            guard_readable_file(&stat_with(FileType::File, 99, false), 10),
            Err(ProjectFsError::TooLarge { size: 99, max: 10 })
        );
        assert_eq!(
            guard_readable_file(&stat_with(FileType::File, 5, false), 10),
            Ok(())
        );
    }
}

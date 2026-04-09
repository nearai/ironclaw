//! [`WorkspaceMounts`] — per-project mount table registry.
//!
//! Maps a [`ProjectId`] to a [`ProjectMounts`] (a list of `(prefix, backend)`
//! pairs). New project entries are built lazily via a [`ProjectMountFactory`]
//! supplied at construction time, so the bridge can wire in either a default
//! `FilesystemBackend` or a `ContainerizedFilesystemBackend` without
//! `WorkspaceMounts` knowing about either implementation.
//!
//! Resolution is longest-prefix-match. The agent sees one filesystem
//! (`/project/foo.txt`, `/memory/notes.md`, `/home/...`) — the registry
//! returns the backend that owns each path along with the relative path
//! inside that mount.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::types::project::ProjectId;
use crate::workspace::mount::{MountBackend, MountError};

/// One project's mount table.
#[derive(Debug, Clone, Default)]
pub struct ProjectMounts {
    /// `(prefix, backend)` pairs sorted by prefix length descending so a
    /// longest-prefix-match resolves correctly. Use [`ProjectMounts::add`]
    /// to maintain ordering.
    mounts: Vec<(String, Arc<dyn MountBackend>)>,
}

impl ProjectMounts {
    /// Empty mount table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a mount at `prefix`. The prefix must end in `/` and be absolute
    /// (start with `/`); the registry stores it normalized. Re-registering
    /// the same prefix replaces the previous backend.
    pub fn add(&mut self, prefix: impl Into<String>, backend: Arc<dyn MountBackend>) {
        let mut p = prefix.into();
        if !p.starts_with('/') {
            p = format!("/{p}");
        }
        if !p.ends_with('/') {
            p.push('/');
        }
        self.mounts.retain(|(existing, _)| existing != &p);
        self.mounts.push((p, backend));
        // Longest-prefix-first
        self.mounts.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    }

    /// Resolve a path against the table. Returns `(backend, relative_path)`
    /// where `relative_path` is the portion of `path` after the matched
    /// prefix.
    ///
    /// `path` may or may not start with `/`. Trailing slashes are preserved
    /// for the relative portion. An exact prefix match (e.g. `/project/`)
    /// returns an empty relative path.
    pub fn resolve(&self, path: &str) -> Option<(Arc<dyn MountBackend>, PathBuf)> {
        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        for (prefix, backend) in &self.mounts {
            if let Some(rest) = normalized.strip_prefix(prefix) {
                return Some((Arc::clone(backend), PathBuf::from(rest)));
            }
            // also accept exact match without trailing slash:
            // resolve("/project") with prefix "/project/" → empty rel
            if let Some(without_slash) = prefix.strip_suffix('/')
                && normalized == without_slash
            {
                return Some((Arc::clone(backend), PathBuf::new()));
            }
        }
        None
    }

    /// Number of mounts in this table (for diagnostics).
    pub fn len(&self) -> usize {
        self.mounts.len()
    }

    /// Whether this mount table is empty.
    pub fn is_empty(&self) -> bool {
        self.mounts.is_empty()
    }
}

/// Builds a fresh [`ProjectMounts`] for a project on first access.
///
/// Implemented by the bridge: in default mode, returns a
/// `FilesystemBackend(~/.ironclaw/projects/<id>/)` registered at `/project/`.
/// When a sandbox container is configured, returns a
/// `ContainerizedFilesystemBackend` instead.
#[async_trait]
pub trait ProjectMountFactory: Send + Sync + std::fmt::Debug {
    /// Build the mount table for `project_id`. Called at most once per
    /// project — the result is cached by [`WorkspaceMounts`].
    async fn build(&self, project_id: ProjectId) -> Result<ProjectMounts, MountError>;
}

/// Per-project mount table registry.
///
/// Holds a cached `HashMap<ProjectId, ProjectMounts>` and a factory that
/// builds new entries on demand. Cloneable cheaply via `Arc`.
#[derive(Debug, Clone)]
pub struct WorkspaceMounts {
    inner: Arc<WorkspaceMountsInner>,
}

#[derive(Debug)]
struct WorkspaceMountsInner {
    by_project: RwLock<HashMap<ProjectId, ProjectMounts>>,
    factory: Arc<dyn ProjectMountFactory>,
}

impl WorkspaceMounts {
    /// Build a registry that lazily creates project mount tables via `factory`.
    pub fn new(factory: Arc<dyn ProjectMountFactory>) -> Self {
        Self {
            inner: Arc::new(WorkspaceMountsInner {
                by_project: RwLock::new(HashMap::new()),
                factory,
            }),
        }
    }

    /// Resolve `path` for the given project, lazily building the project's
    /// mount table on first access. Returns `None` if no mount in the table
    /// owns the path (which is the signal to the bridge interceptor to fall
    /// through to direct host execution).
    pub async fn resolve(
        &self,
        project_id: ProjectId,
        path: &str,
    ) -> Result<Option<(Arc<dyn MountBackend>, PathBuf)>, MountError> {
        // Fast path: read lock, check cache.
        {
            let cache = self.inner.by_project.read().await;
            if let Some(mounts) = cache.get(&project_id) {
                return Ok(mounts.resolve(path));
            }
        }
        // Slow path: build, cache, resolve.
        let mounts = self.inner.factory.build(project_id).await?;
        let resolution = mounts.resolve(path);
        {
            let mut cache = self.inner.by_project.write().await;
            cache.entry(project_id).or_insert(mounts);
        }
        Ok(resolution)
    }

    /// Drop the cached mount table for a project. Call when a project is
    /// deleted or its container is reset, so the next access rebuilds.
    pub async fn invalidate(&self, project_id: ProjectId) {
        self.inner.by_project.write().await.remove(&project_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::filesystem::FilesystemBackend;
    use std::path::Path;
    use tempfile::TempDir;

    #[derive(Debug)]
    struct StaticFactory {
        roots: HashMap<ProjectId, PathBuf>,
    }

    #[async_trait]
    impl ProjectMountFactory for StaticFactory {
        async fn build(&self, project_id: ProjectId) -> Result<ProjectMounts, MountError> {
            let root = self
                .roots
                .get(&project_id)
                .ok_or_else(|| MountError::Backend {
                    reason: format!("no root for project {project_id}"),
                })?
                .clone();
            let mut mounts = ProjectMounts::new();
            mounts.add("/project/", Arc::new(FilesystemBackend::new(root)));
            Ok(mounts)
        }
    }

    fn make_registry() -> (WorkspaceMounts, ProjectId, TempDir) {
        let pid = ProjectId::new();
        let dir = tempfile::tempdir().unwrap();
        let factory = StaticFactory {
            roots: HashMap::from([(pid, dir.path().to_path_buf())]),
        };
        (WorkspaceMounts::new(Arc::new(factory)), pid, dir)
    }

    #[tokio::test]
    async fn resolves_project_path_through_factory() {
        let (mounts, pid, dir) = make_registry();
        std::fs::write(dir.path().join("foo.txt"), b"hi").unwrap();

        let (backend, rel) = mounts
            .resolve(pid, "/project/foo.txt")
            .await
            .unwrap()
            .expect("should resolve");
        assert_eq!(rel, Path::new("foo.txt"));
        let bytes = backend.read(&rel).await.unwrap();
        assert_eq!(bytes, b"hi");
    }

    #[tokio::test]
    async fn unmounted_path_returns_none() {
        let (mounts, pid, _dir) = make_registry();
        let resolution = mounts.resolve(pid, "/elsewhere/file").await.unwrap();
        assert!(resolution.is_none());
    }

    #[tokio::test]
    async fn caches_per_project_mount_table() {
        let pid = ProjectId::new();
        let dir = tempfile::tempdir().unwrap();

        // Counter wrapped in Arc so we can observe build calls
        #[derive(Debug)]
        struct CountingFactory {
            calls: Arc<std::sync::atomic::AtomicUsize>,
            root: PathBuf,
        }

        #[async_trait]
        impl ProjectMountFactory for CountingFactory {
            async fn build(&self, _: ProjectId) -> Result<ProjectMounts, MountError> {
                self.calls
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let mut mounts = ProjectMounts::new();
                mounts.add("/project/", Arc::new(FilesystemBackend::new(&self.root)));
                Ok(mounts)
            }
        }

        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let factory = CountingFactory {
            calls: Arc::clone(&calls),
            root: dir.path().to_path_buf(),
        };
        let mounts = WorkspaceMounts::new(Arc::new(factory));

        mounts.resolve(pid, "/project/a").await.unwrap();
        mounts.resolve(pid, "/project/b").await.unwrap();
        mounts.resolve(pid, "/project/c").await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);

        mounts.invalidate(pid).await;
        mounts.resolve(pid, "/project/a").await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[test]
    fn longest_prefix_wins() {
        let mut mounts = ProjectMounts::new();
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        mounts.add("/project/", Arc::new(FilesystemBackend::new(dir1.path())));
        mounts.add(
            "/project/sub/",
            Arc::new(FilesystemBackend::new(dir2.path())),
        );

        let (b1, rel1) = mounts.resolve("/project/foo.txt").unwrap();
        assert_eq!(rel1, Path::new("foo.txt"));
        // The /project/sub/ backend should NOT have caught this — only the longer prefix
        // should match its own subtree.
        assert!(format!("{:?}", b1).contains(&format!("{:?}", dir1.path())));

        let (b2, rel2) = mounts.resolve("/project/sub/bar.txt").unwrap();
        assert_eq!(rel2, Path::new("bar.txt"));
        assert!(format!("{:?}", b2).contains(&format!("{:?}", dir2.path())));
    }

    #[test]
    fn re_register_same_prefix_replaces() {
        let mut mounts = ProjectMounts::new();
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        mounts.add("/project/", Arc::new(FilesystemBackend::new(dir1.path())));
        mounts.add("/project/", Arc::new(FilesystemBackend::new(dir2.path())));
        assert_eq!(mounts.len(), 1);
    }

    #[test]
    fn prefix_normalized_to_have_trailing_slash() {
        let mut mounts = ProjectMounts::new();
        let dir = tempfile::tempdir().unwrap();
        mounts.add("/project", Arc::new(FilesystemBackend::new(dir.path())));
        let (_, rel) = mounts.resolve("/project/foo").unwrap();
        assert_eq!(rel, Path::new("foo"));
    }
}

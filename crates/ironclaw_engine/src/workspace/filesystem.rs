//! [`FilesystemBackend`] — passthrough [`MountBackend`] over a real
//! filesystem rooted at a host path.
//!
//! Used by the bridge as the default backend for the `/project/` mount when
//! no sandbox is configured. Path validation rejects absolute paths and any
//! component-level escapes (`..`); after lexical validation, the resolved
//! path is canonicalized when possible and re-checked against the root to
//! defend against symlink-based escapes.
//!
//! `read`, `write`, and `list` are fully implemented. `patch` and `shell`
//! return [`MountError::Unsupported`] in this revision; the bridge interceptor
//! falls through to the host tool when this happens, so callers don't lose
//! functionality. Both will be implemented when the containerized backend
//! lands and needs symmetric coverage.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;

use super::mount::{DirEntry, EntryKind, MountBackend, MountError, ShellOutput};

/// Passthrough mount backend rooted at a real host path.
#[derive(Debug, Clone)]
pub struct FilesystemBackend {
    root: PathBuf,
}

impl FilesystemBackend {
    /// Build a backend rooted at `root`. The root is not required to exist
    /// at construction time; individual operations create or fail as needed.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The host filesystem root for this backend.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Lexically validate `rel_path` and join it with the root.
    ///
    /// Rejects absolute paths and any path containing `..` or root
    /// components — both as a clean error to the caller and as the first
    /// layer of defense against directory-traversal attacks.
    fn safe_join(&self, rel_path: &Path) -> Result<PathBuf, MountError> {
        if rel_path.is_absolute() {
            return Err(MountError::invalid_path(
                rel_path,
                "absolute paths are not allowed; pass a path relative to the mount",
            ));
        }
        for component in rel_path.components() {
            match component {
                Component::Normal(_) | Component::CurDir => {}
                Component::ParentDir => {
                    return Err(MountError::invalid_path(
                        rel_path,
                        "`..` components are not allowed",
                    ));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(MountError::invalid_path(
                        rel_path,
                        "root or prefix components are not allowed",
                    ));
                }
            }
        }
        Ok(self.root.join(rel_path))
    }

    /// After resolving a path, defend against symlink escapes by canonicalizing
    /// any existing ancestor and verifying it stays under `self.root`.
    ///
    /// Returns the canonical form when canonicalization succeeds, otherwise
    /// the lexical join. Files that don't exist yet (write path) cannot be
    /// canonicalized — for those, we walk up to the closest existing
    /// ancestor, canonicalize *that*, then re-attach the missing tail.
    fn canonicalize_under_root(&self, joined: &Path) -> Result<PathBuf, MountError> {
        // When the root doesn't exist yet (project dir not created), skip
        // the canonicalization check entirely. Lexical safety is already
        // guaranteed by `safe_join` (no `..`, no absolute paths). Without
        // this guard, the existing-prefix walk would climb up to a real
        // ancestor (e.g. `/tmp`) and the `starts_with` check against the
        // non-existent root would always fail, blocking writes that would
        // create the directory.
        let canonical_root = match std::fs::canonicalize(&self.root) {
            Ok(r) => r,
            Err(_) => return Ok(joined.to_path_buf()),
        };

        // Find the longest existing prefix of `joined`.
        let mut existing_prefix = joined.to_path_buf();
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        loop {
            if existing_prefix.exists() {
                break;
            }
            let name = existing_prefix.file_name().map(|n| n.to_os_string());
            let parent = existing_prefix.parent().map(|p| p.to_path_buf());
            match (parent, name) {
                (Some(parent), Some(name)) => {
                    tail.push(name);
                    existing_prefix = parent;
                }
                _ => break,
            }
        }

        let canonical_prefix = match std::fs::canonicalize(&existing_prefix) {
            Ok(p) => p,
            Err(_) => existing_prefix.clone(),
        };

        if !canonical_prefix.starts_with(&canonical_root) {
            return Err(MountError::invalid_path(
                joined,
                "resolved path escapes the mount root via symlink",
            ));
        }

        let mut result = canonical_prefix;
        for component in tail.into_iter().rev() {
            result.push(component);
        }
        Ok(result)
    }

    /// Combine [`safe_join`] and [`canonicalize_under_root`] for a complete
    /// resolution that defends against both lexical and symlink escapes.
    fn resolve(&self, rel_path: &Path) -> Result<PathBuf, MountError> {
        let joined = self.safe_join(rel_path)?;
        self.canonicalize_under_root(&joined)
    }
}

#[async_trait]
impl MountBackend for FilesystemBackend {
    async fn read(&self, rel_path: &Path) -> Result<Vec<u8>, MountError> {
        let full = self.resolve(rel_path)?;
        tokio::fs::read(&full)
            .await
            .map_err(|e| MountError::io(&full, &e))
    }

    async fn write(&self, rel_path: &Path, content: &[u8]) -> Result<(), MountError> {
        let full = self.resolve(rel_path)?;
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| MountError::io(parent, &e))?;
        }
        tokio::fs::write(&full, content)
            .await
            .map_err(|e| MountError::io(&full, &e))
    }

    async fn list(&self, rel_path: &Path, depth: usize) -> Result<Vec<DirEntry>, MountError> {
        let full = self.resolve(rel_path)?;
        let mut out = Vec::new();
        list_dir_recursive(&full, &full, depth, &mut out).await?;
        Ok(out)
    }

    async fn patch(&self, _rel_path: &Path, _diff: &str) -> Result<(), MountError> {
        Err(MountError::Unsupported {
            operation: "FilesystemBackend::patch (deferred to a later phase; \
                        bridge falls through to host tool)"
                .into(),
        })
    }

    async fn shell(
        &self,
        _command: &str,
        _env: HashMap<String, String>,
        _cwd: Option<&Path>,
    ) -> Result<ShellOutput, MountError> {
        Err(MountError::Unsupported {
            operation: "FilesystemBackend::shell (deferred to a later phase; \
                        bridge falls through to host tool)"
                .into(),
        })
    }
}

/// Walk a directory and emit [`DirEntry`] values up to `depth` levels deep.
///
/// `depth = 0` lists only the immediate children of `dir`.
async fn list_dir_recursive(
    dir: &Path,
    root: &Path,
    depth: usize,
    out: &mut Vec<DirEntry>,
) -> Result<(), MountError> {
    let mut stack: Vec<(PathBuf, usize)> = vec![(dir.to_path_buf(), 0)];
    while let Some((current, current_depth)) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&current)
            .await
            .map_err(|e| MountError::io(&current, &e))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| MountError::io(&current, &e))?
        {
            let path = entry.path();
            let metadata = entry
                .metadata()
                .await
                .map_err(|e| MountError::io(&path, &e))?;

            let kind = if metadata.is_dir() {
                EntryKind::Directory
            } else if metadata.file_type().is_symlink() {
                EntryKind::Symlink
            } else {
                EntryKind::File
            };

            let rel = path
                .strip_prefix(root)
                .map(PathBuf::from)
                .unwrap_or_else(|_| path.clone());

            let size = if matches!(kind, EntryKind::File) {
                Some(metadata.len())
            } else {
                None
            };

            out.push(DirEntry {
                path: rel,
                kind,
                size,
            });

            if matches!(kind, EntryKind::Directory) && current_depth < depth {
                stack.push((path, current_depth + 1));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn backend() -> (FilesystemBackend, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = FilesystemBackend::new(dir.path());
        (backend, dir)
    }

    #[tokio::test]
    async fn write_and_read_roundtrip() {
        let (backend, _dir) = backend();
        backend
            .write(Path::new("foo.txt"), b"hello world")
            .await
            .unwrap();
        let bytes = backend.read(Path::new("foo.txt")).await.unwrap();
        assert_eq!(bytes, b"hello world");
    }

    #[tokio::test]
    async fn write_creates_nested_directories() {
        let (backend, _dir) = backend();
        backend
            .write(Path::new("a/b/c/d.txt"), b"deep")
            .await
            .unwrap();
        let bytes = backend.read(Path::new("a/b/c/d.txt")).await.unwrap();
        assert_eq!(bytes, b"deep");
    }

    #[tokio::test]
    async fn read_missing_returns_not_found() {
        let (backend, _dir) = backend();
        let err = backend.read(Path::new("missing.txt")).await.unwrap_err();
        assert!(matches!(err, MountError::NotFound { .. }));
    }

    #[tokio::test]
    async fn rejects_absolute_paths() {
        let (backend, _dir) = backend();
        let err = backend.read(Path::new("/etc/passwd")).await.unwrap_err();
        assert!(matches!(err, MountError::InvalidPath { .. }));
    }

    #[tokio::test]
    async fn rejects_parent_dir_escapes() {
        let (backend, _dir) = backend();
        let err = backend
            .read(Path::new("../../etc/passwd"))
            .await
            .unwrap_err();
        assert!(matches!(err, MountError::InvalidPath { .. }));
    }

    #[tokio::test]
    async fn rejects_symlink_escapes() {
        // Set up: backend root is a tempdir; create a symlink inside that
        // points outside. Reading through the symlink should be rejected by
        // canonicalize_under_root.
        let (backend, dir) = backend();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"oops").unwrap();
        // best effort — symlink may not work on all platforms; skip if it fails
        #[cfg(unix)]
        {
            let link = dir.path().join("escape");
            std::os::unix::fs::symlink(outside.path(), &link).unwrap();
            let err = backend
                .read(Path::new("escape/secret"))
                .await
                .expect_err("symlink escape must be rejected");
            assert!(matches!(err, MountError::InvalidPath { .. }));
        }
        #[cfg(not(unix))]
        {
            let _ = dir; // unused on non-unix
        }
    }

    #[tokio::test]
    async fn list_returns_immediate_entries() {
        let (backend, _dir) = backend();
        backend.write(Path::new("a.txt"), b"a").await.unwrap();
        backend.write(Path::new("b.txt"), b"bb").await.unwrap();
        backend.write(Path::new("sub/c.txt"), b"ccc").await.unwrap();
        let entries = backend.list(Path::new(""), 0).await.unwrap();
        let mut names: Vec<_> = entries
            .iter()
            .map(|e| e.path.display().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.txt", "b.txt", "sub"]);

        // file sizes are populated for files only
        let a = entries
            .iter()
            .find(|e| e.path == Path::new("a.txt"))
            .unwrap();
        assert_eq!(a.size, Some(1));
        let sub = entries.iter().find(|e| e.path == Path::new("sub")).unwrap();
        assert_eq!(sub.size, None);
        assert_eq!(sub.kind, EntryKind::Directory);
    }

    #[tokio::test]
    async fn list_recursive_with_depth() {
        let (backend, _dir) = backend();
        backend.write(Path::new("a.txt"), b"a").await.unwrap();
        backend.write(Path::new("d1/b.txt"), b"b").await.unwrap();
        backend.write(Path::new("d1/d2/c.txt"), b"c").await.unwrap();

        let entries = backend.list(Path::new(""), 1).await.unwrap();
        let names: std::collections::HashSet<_> = entries
            .iter()
            .map(|e| e.path.display().to_string())
            .collect();
        assert!(names.contains("a.txt"));
        assert!(names.contains("d1"));
        assert!(names.contains("d1/b.txt"));
        assert!(names.contains("d1/d2"));
        // depth=1: d1/d2 entries are listed but d1/d2/* is not
        assert!(!names.contains("d1/d2/c.txt"));
    }

    #[tokio::test]
    async fn patch_and_shell_unsupported_in_phase_1() {
        let (backend, _dir) = backend();
        let err = backend.patch(Path::new("foo"), "diff").await.unwrap_err();
        assert!(matches!(err, MountError::Unsupported { .. }));
        let err = backend.shell("ls", HashMap::new(), None).await.unwrap_err();
        assert!(matches!(err, MountError::Unsupported { .. }));
    }
}

//! [`MountBackend`] trait and associated value types.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Errors a mount backend can return.
///
/// Distinguishes "tool failed" (`Tool`) from "sandbox/transport failed"
/// (`Backend`) so the engine can surface them differently to the orchestrator.
/// `NotFound`, `PermissionDenied`, and `InvalidPath` are normal recoverable
/// outcomes the LLM should see; `Backend` is an infrastructure failure that
/// should be retried at a different layer.
#[derive(Debug, thiserror::Error)]
pub enum MountError {
    /// Path does not exist within this mount.
    #[error("not found: {path}")]
    NotFound { path: String },

    /// Path is outside the mount root, contains `..`, is absolute, or
    /// otherwise rejected by the backend's safety checks.
    #[error("invalid path: {path}: {reason}")]
    InvalidPath { path: String, reason: String },

    /// OS-level permission error.
    #[error("permission denied: {path}")]
    PermissionDenied { path: String },

    /// I/O error from the underlying storage.
    #[error("io error at {path}: {reason}")]
    Io { path: String, reason: String },

    /// Tool execution returned a non-zero status or other tool-level error.
    /// LLM should see this and can self-correct.
    #[error("tool error: {reason}")]
    Tool { reason: String },

    /// Backend transport / sandbox infrastructure failure (container down,
    /// daemon crashed, IPC broken). The orchestrator should not surface
    /// this directly to the LLM as a tool error — it's an infrastructure
    /// problem to be handled at a different layer.
    #[error("backend error: {reason}")]
    Backend { reason: String },

    /// Operation not supported by this backend in its current version.
    #[error("operation not supported: {operation}")]
    Unsupported { operation: String },
}

impl MountError {
    /// Build a [`MountError::NotFound`] from a path.
    pub fn not_found(path: impl AsRef<Path>) -> Self {
        Self::NotFound {
            path: path.as_ref().display().to_string(),
        }
    }

    /// Build a [`MountError::InvalidPath`] from a path and reason.
    pub fn invalid_path(path: impl AsRef<Path>, reason: impl Into<String>) -> Self {
        Self::InvalidPath {
            path: path.as_ref().display().to_string(),
            reason: reason.into(),
        }
    }

    /// Build a [`MountError::Io`] from an `std::io::Error` and a path.
    pub fn io(path: impl AsRef<Path>, err: &std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => Self::not_found(path),
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied {
                path: path.as_ref().display().to_string(),
            },
            _ => Self::Io {
                path: path.as_ref().display().to_string(),
                reason: err.to_string(),
            },
        }
    }
}

/// One entry returned from [`MountBackend::list`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// Path of the entry, relative to the mount root.
    pub path: PathBuf,
    /// Whether this is a file, directory, or symlink.
    pub kind: EntryKind,
    /// Size in bytes (only meaningful for files).
    pub size: Option<u64>,
}

/// Kind of a directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
}

/// Output from a shell execution via [`MountBackend::shell`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// A storage backend for one mount in the workspace mount table.
///
/// All paths passed to backend methods are **relative to the mount root**.
/// Backends MUST reject paths that escape the root via `..`, absolute paths,
/// or symlinks resolving outside the root. The trait does not specify the
/// rejection mechanism; concrete impls (such as [`crate::workspace::FilesystemBackend`])
/// implement defense-in-depth path validation.
#[async_trait]
pub trait MountBackend: Send + Sync + std::fmt::Debug {
    /// Read the contents of `rel_path` and return its bytes.
    async fn read(&self, rel_path: &Path) -> Result<Vec<u8>, MountError>;

    /// Write `content` to `rel_path`, creating parent directories as needed.
    /// Overwrites if the file already exists.
    async fn write(&self, rel_path: &Path, content: &[u8]) -> Result<(), MountError>;

    /// List entries under `rel_path` up to `depth` levels. `depth = 0` lists
    /// only the immediate entries. `rel_path` must be a directory.
    async fn list(&self, rel_path: &Path, depth: usize) -> Result<Vec<DirEntry>, MountError>;

    /// Apply a unified diff to `rel_path`.
    ///
    /// Implementations may return [`MountError::Unsupported`] if patch
    /// application is not yet wired up — the bridge interceptor will fall
    /// through to the host tool in that case.
    async fn patch(&self, rel_path: &Path, diff: &str) -> Result<(), MountError>;

    /// Execute `command` in a shell. `cwd`, when present, is relative to
    /// the mount root.
    ///
    /// Implementations may return [`MountError::Unsupported`] if shell
    /// execution is not yet wired up.
    async fn shell(
        &self,
        command: &str,
        env: HashMap<String, String>,
        cwd: Option<&Path>,
    ) -> Result<ShellOutput, MountError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_error_io_classifies_kinds() {
        let nf = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
        assert!(matches!(
            MountError::io(Path::new("/a"), &nf),
            MountError::NotFound { .. }
        ));

        let pd = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "x");
        assert!(matches!(
            MountError::io(Path::new("/a"), &pd),
            MountError::PermissionDenied { .. }
        ));

        let other = std::io::Error::other("boom");
        assert!(matches!(
            MountError::io(Path::new("/a"), &other),
            MountError::Io { .. }
        ));
    }

    #[test]
    fn dir_entry_serializes_kind_lowercase() {
        let e = DirEntry {
            path: PathBuf::from("foo.txt"),
            kind: EntryKind::File,
            size: Some(42),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"file\""));
        assert!(json.contains("\"size\":42"));
    }
}

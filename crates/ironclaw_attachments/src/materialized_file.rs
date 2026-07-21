//! Canonical in-memory representation of a materialized attachment or file.

use ironclaw_host_api::ScopedPath;

/// A trusted, in-memory file whose path has already been validated by its
/// owning filesystem boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedFile<P> {
    pub path: P,
    pub filename: Option<String>,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

impl<P> MaterializedFile<P> {
    pub fn size_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }
}

/// A file from a thread's scoped project workspace.
pub type WorkspaceFile = MaterializedFile<ScopedPath>;

/// A file from the standalone multi-mount browse surface.
pub type ProjectFsFile = MaterializedFile<String>;

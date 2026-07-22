//! Canonical in-memory representation of a materialized attachment or file.

use ironclaw_host_api::ScopedPath;
use std::fmt;

/// A trusted, in-memory file whose path has already been validated by its
/// owning filesystem boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct MaterializedFile<P> {
    pub path: P,
    pub filename: Option<String>,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

impl<P: fmt::Debug> fmt::Debug for MaterializedFile<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MaterializedFile")
            .field("path", &self.path)
            .field("filename", &self.filename)
            .field("mime_type", &self.mime_type)
            .field("size_bytes", &self.bytes.len())
            .finish()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_materialized_bytes() {
        let file = MaterializedFile {
            path: "/workspace/report.txt".to_string(),
            filename: Some("report.txt".to_string()),
            mime_type: "text/plain".to_string(),
            bytes: b"byte-sentinel-must-not-leak".to_vec(),
        };

        let rendered = format!("{file:?}");
        assert!(rendered.contains("/workspace/report.txt"));
        assert!(rendered.contains("size_bytes"));
        assert!(!rendered.contains("byte-sentinel-must-not-leak"));
        assert!(!rendered.contains("98, 121, 116, 101"));
    }
}

//! Workspace document reader.
//!
//! Used by the mission runtime to load `context_paths` files into a fired
//! mission's meta-prompt. The host (main `ironclaw` crate) implements this
//! over the existing `Workspace` API.
//!
//! Kept deliberately small: just enough surface to read a single document
//! by relative path. The engine does not write to the workspace.

use crate::types::error::EngineError;

/// Reads workspace documents by path. Implementations must be tenant-safe:
/// the workspace they wrap is the one belonging to the mission's owner.
#[async_trait::async_trait]
pub trait WorkspaceReader: Send + Sync {
    /// Read a document by relative workspace path. Returns the document body
    /// as a string. Implementations should return an error rather than panic
    /// when the file does not exist or cannot be decoded.
    async fn read_doc(&self, path: &str) -> Result<String, EngineError>;
}

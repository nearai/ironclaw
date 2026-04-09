//! `WorkspaceReader` adapter.
//!
//! Implements the engine's `WorkspaceReader` trait over the host's existing
//! `Workspace` API. The mission runtime uses this to load `context_paths`
//! files into a fired mission's meta-prompt.

use std::sync::Arc;

use ironclaw_engine::WorkspaceReader;
use ironclaw_engine::types::error::EngineError;

use crate::workspace::Workspace;

/// Adapts a host `Workspace` to the engine's `WorkspaceReader` trait.
///
/// The wrapped workspace is the one belonging to the mission's owner — the
/// caller is responsible for tenant correctness when constructing the
/// adapter (typically by passing the per-user workspace handle from
/// `agent.workspace()`).
pub struct WorkspaceReaderAdapter {
    workspace: Arc<Workspace>,
}

impl WorkspaceReaderAdapter {
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait::async_trait]
impl WorkspaceReader for WorkspaceReaderAdapter {
    async fn read_doc(&self, path: &str) -> Result<String, EngineError> {
        match self.workspace.read(path).await {
            Ok(doc) => Ok(doc.content),
            Err(error) => Err(EngineError::Store {
                reason: format!("workspace read failed for {path}: {error}"),
            }),
        }
    }
}

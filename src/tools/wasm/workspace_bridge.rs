use std::sync::{Arc, Mutex};

use crate::error::WorkspaceError;
use crate::workspace::Workspace;

use super::capabilities::{WorkspaceReader, WorkspaceWriter};

/// Adapter that exposes async workspace APIs through sync WASM host traits.
pub struct WasmWorkspaceBridge {
    workspace: Arc<Workspace>,
    runtime: Mutex<Option<tokio::runtime::Runtime>>,
}

impl WasmWorkspaceBridge {
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self {
            workspace,
            runtime: Mutex::new(None),
        }
    }

    fn with_runtime<T>(&self, f: impl FnOnce(&tokio::runtime::Runtime) -> T) -> Result<T, String> {
        let mut guard = self
            .runtime
            .lock()
            .map_err(|_| "Workspace runtime lock poisoned".to_string())?;

        if guard.is_none() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("Failed to create workspace runtime: {e}"))?;
            *guard = Some(rt);
        }

        let rt = guard
            .as_ref()
            .ok_or_else(|| "Workspace runtime initialization failed".to_string())?;

        Ok(f(rt))
    }
}

impl WorkspaceReader for WasmWorkspaceBridge {
    fn read(&self, path: &str) -> Option<String> {
        match self.with_runtime(|rt| rt.block_on(self.workspace.read(path))) {
            Ok(Ok(doc)) => Some(doc.content),
            Ok(Err(WorkspaceError::DocumentNotFound { .. })) => None,
            Ok(Err(err)) => {
                tracing::warn!(path = path, error = %err, "WASM workspace read failed");
                None
            }
            Err(err) => {
                tracing::warn!(path = path, error = %err, "WASM workspace runtime failed");
                None
            }
        }
    }
}

impl WorkspaceWriter for WasmWorkspaceBridge {
    fn write(&self, path: &str, content: &str) -> Result<(), String> {
        self.with_runtime(|rt| rt.block_on(self.workspace.write(path, content)))?
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

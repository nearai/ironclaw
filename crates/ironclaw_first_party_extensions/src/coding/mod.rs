//! First-party coding capability handlers.
//!
//! Keep v1-compatible coding families in narrow modules. Host runtime adapts
//! already-authorized capability invocations into [`CodingCapabilityRequest`];
//! this module receives scoped paths and an explicit filesystem handle only.

mod config;
mod file;
mod glob_tool;
mod grep_tool;
mod inputs;
mod paths;
mod state;
mod text;
mod types;

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{MountView, ResourceScope, RuntimeDispatchErrorKind};
use serde_json::Value;

use state::{SharedCodingEditLocks, SharedCodingReadState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingCapabilityKind {
    ReadFile,
    WriteFile,
    ListDir,
    Glob,
    Grep,
    ApplyPatch,
}

#[derive(Clone)]
pub struct CodingCapabilityRequest<'a> {
    pub(crate) kind: CodingCapabilityKind,
    pub(crate) scope: &'a ResourceScope,
    pub(crate) mounts: Option<&'a MountView>,
    pub(crate) filesystem: Arc<dyn RootFilesystem>,
    pub(crate) input: &'a Value,
}

impl<'a> CodingCapabilityRequest<'a> {
    pub fn new(
        kind: CodingCapabilityKind,
        scope: &'a ResourceScope,
        mounts: Option<&'a MountView>,
        filesystem: Arc<dyn RootFilesystem>,
        input: &'a Value,
    ) -> Self {
        Self {
            kind,
            scope,
            mounts,
            filesystem,
            input,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("coding capability dispatch failed: {kind}")]
pub struct CodingCapabilityError {
    kind: RuntimeDispatchErrorKind,
}

impl CodingCapabilityError {
    pub fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }
}

#[derive(Debug, Default)]
pub struct CodingCapabilityState {
    read_state: SharedCodingReadState,
    edit_locks: SharedCodingEditLocks,
}

impl CodingCapabilityState {
    pub async fn dispatch(
        &self,
        request: &CodingCapabilityRequest<'_>,
    ) -> Result<Value, CodingCapabilityError> {
        dispatch(request, &self.read_state, &self.edit_locks).await
    }
}

async fn dispatch(
    request: &CodingCapabilityRequest<'_>,
    read_state: &SharedCodingReadState,
    edit_locks: &SharedCodingEditLocks,
) -> Result<Value, CodingCapabilityError> {
    match request.kind {
        CodingCapabilityKind::ReadFile => file::read_file(request, read_state).await,
        CodingCapabilityKind::WriteFile => file::write_file(request, read_state, edit_locks).await,
        CodingCapabilityKind::ListDir => file::list_dir(request).await,
        CodingCapabilityKind::Glob => glob_tool::glob(request).await,
        CodingCapabilityKind::Grep => grep_tool::grep(request).await,
        CodingCapabilityKind::ApplyPatch => {
            file::apply_patch(request, read_state, edit_locks).await
        }
    }
}

fn input_error() -> CodingCapabilityError {
    CodingCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
}

fn guest_error() -> CodingCapabilityError {
    CodingCapabilityError::new(RuntimeDispatchErrorKind::Guest)
}

#[cfg(test)]
mod tests {
    #[test]
    fn coding_tools_do_not_select_runtime_backends() {
        let sources = [
            include_str!("file.rs"),
            include_str!("glob_tool.rs"),
            include_str!("grep_tool.rs"),
            include_str!("paths.rs"),
        ];
        for source in sources {
            assert!(!source.contains("ProcessBackendKind"));
            assert!(!source.contains("FilesystemBackendKind"));
        }
    }
}

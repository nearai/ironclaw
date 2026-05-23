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
use ironclaw_host_api::{CapabilityId, MountView, ResourceScope, RuntimeDispatchErrorKind};
use serde_json::Value;

use state::{SharedCodingEditLocks, SharedCodingReadState};

pub const READ_FILE_CAPABILITY_ID: &str = "builtin.read_file";
pub const WRITE_FILE_CAPABILITY_ID: &str = "builtin.write_file";
pub const LIST_DIR_CAPABILITY_ID: &str = "builtin.list_dir";
pub const GLOB_CAPABILITY_ID: &str = "builtin.glob";
pub const GREP_CAPABILITY_ID: &str = "builtin.grep";
pub const APPLY_PATCH_CAPABILITY_ID: &str = "builtin.apply_patch";

#[derive(Clone)]
pub struct CodingCapabilityRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub mounts: Option<MountView>,
    pub filesystem: Arc<dyn RootFilesystem>,
    pub input: Value,
}

impl CodingCapabilityRequest {
    pub fn new(
        capability_id: CapabilityId,
        scope: ResourceScope,
        mounts: Option<MountView>,
        filesystem: Arc<dyn RootFilesystem>,
        input: Value,
    ) -> Self {
        Self {
            capability_id,
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
        request: &CodingCapabilityRequest,
    ) -> Result<Value, CodingCapabilityError> {
        dispatch(request, &self.read_state, &self.edit_locks).await
    }
}

async fn dispatch(
    request: &CodingCapabilityRequest,
    read_state: &SharedCodingReadState,
    edit_locks: &SharedCodingEditLocks,
) -> Result<Value, CodingCapabilityError> {
    match request.capability_id.as_str() {
        READ_FILE_CAPABILITY_ID => file::read_file(request, read_state).await,
        WRITE_FILE_CAPABILITY_ID => file::write_file(request, read_state, edit_locks).await,
        LIST_DIR_CAPABILITY_ID => file::list_dir(request).await,
        GLOB_CAPABILITY_ID => glob_tool::glob(request).await,
        GREP_CAPABILITY_ID => grep_tool::grep(request).await,
        APPLY_PATCH_CAPABILITY_ID => file::apply_patch(request, read_state, edit_locks).await,
        _ => Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::UndeclaredCapability,
        )),
    }
}

fn input_error() -> CodingCapabilityError {
    CodingCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
}

fn guest_error() -> CodingCapabilityError {
    CodingCapabilityError::new(RuntimeDispatchErrorKind::Guest)
}

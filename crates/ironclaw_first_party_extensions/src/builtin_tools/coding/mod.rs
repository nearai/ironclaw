//! First-party coding capability handlers.
//!
//! Keep v1-compatible coding families in narrow modules. Shared
//! path/text/input/state helpers preserve the Reborn host contract:
//! capabilities receive scoped paths and use `RootFilesystem` only; local
//! kernel access stays inside filesystem backends.

mod config;
mod file;
mod glob_tool;
mod grep_tool;
mod inputs;
mod paths;
mod state;
mod text;
mod types;

use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode, RuntimeDispatchErrorKind};
use serde_json::Value;

use ironclaw_host_runtime::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{first_party_capability_manifest, resource_profile};

pub const READ_FILE_CAPABILITY_ID: &str = "builtin.read_file";
pub const WRITE_FILE_CAPABILITY_ID: &str = "builtin.write_file";
pub const LIST_DIR_CAPABILITY_ID: &str = "builtin.list_dir";
pub const GLOB_CAPABILITY_ID: &str = "builtin.glob";
pub const GREP_CAPABILITY_ID: &str = "builtin.grep";
pub const APPLY_PATCH_CAPABILITY_ID: &str = "builtin.apply_patch";

pub(super) use state::{SharedCodingEditLocks, SharedCodingReadState};

pub(super) fn manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    Ok(vec![
        manifest(
            READ_FILE_CAPABILITY_ID,
            "Read a file through scoped mounts with v1 read_file output shape",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
        )?,
        manifest(
            WRITE_FILE_CAPABILITY_ID,
            "Write content through scoped mounts with v1 write_file output shape",
            vec![EffectKind::WriteFilesystem],
            PermissionMode::Allow,
        )?,
        manifest(
            LIST_DIR_CAPABILITY_ID,
            "List directory contents through scoped mounts with v1 list_dir output shape",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
        )?,
        manifest(
            GLOB_CAPABILITY_ID,
            "Find files under a scoped directory with v1 glob output shape",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
        )?,
        manifest(
            GREP_CAPABILITY_ID,
            "Search scoped file contents with v1 grep output modes",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
        )?,
        manifest(
            APPLY_PATCH_CAPABILITY_ID,
            "Apply exact/fuzzy search-replace edits through scoped mounts",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            PermissionMode::Allow,
        )?,
    ])
}

fn manifest(
    id: &str,
    description: &str,
    effects: Vec<EffectKind>,
    default_permission: PermissionMode,
) -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        id,
        description,
        effects,
        default_permission,
        resource_profile(),
    )
}

pub(super) async fn dispatch(
    request: &FirstPartyCapabilityRequest,
    read_state: &SharedCodingReadState,
    edit_locks: &SharedCodingEditLocks,
) -> Result<Value, FirstPartyCapabilityError> {
    match request.capability_id.as_str() {
        READ_FILE_CAPABILITY_ID => file::read_file(request, read_state).await,
        WRITE_FILE_CAPABILITY_ID => file::write_file(request, read_state, edit_locks).await,
        LIST_DIR_CAPABILITY_ID => file::list_dir(request).await,
        GLOB_CAPABILITY_ID => glob_tool::glob(request).await,
        GREP_CAPABILITY_ID => grep_tool::grep(request).await,
        APPLY_PATCH_CAPABILITY_ID => file::apply_patch(request, read_state, edit_locks).await,
        _ => Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::UndeclaredCapability,
        )),
    }
}

fn input_error() -> FirstPartyCapabilityError {
    super::input_error()
}

fn guest_error() -> FirstPartyCapabilityError {
    super::guest_error()
}

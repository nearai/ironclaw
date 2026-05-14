//! First-party coding capability handlers.
//!
//! Keep each capability in its own module. Shared path/text/input/state helpers
//! preserve the Reborn host contract: capabilities receive scoped paths and use
//! `RootFilesystem` only; local kernel access stays inside filesystem backends.

mod apply_patch;
mod config;
mod glob;
mod grep;
mod inputs;
mod list_dir;
mod paths;
mod read_file;
mod state;
mod text;
mod types;
mod write_file;

use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{CapabilityId, EffectKind, PermissionMode, RuntimeDispatchErrorKind};
use serde_json::{Value, json};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::resource_profile;

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
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "offset": { "type": "integer" },
                    "limit": { "type": "integer" }
                },
                "required": ["path"]
            }),
        )?,
        manifest(
            WRITE_FILE_CAPABILITY_ID,
            "Write content through scoped mounts with v1 write_file output shape",
            vec![EffectKind::WriteFilesystem],
            PermissionMode::Allow,
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        )?,
        manifest(
            LIST_DIR_CAPABILITY_ID,
            "List directory contents through scoped mounts with v1 list_dir output shape",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "recursive": { "type": "boolean" },
                    "max_depth": { "type": "integer" }
                }
            }),
        )?,
        manifest(
            GLOB_CAPABILITY_ID,
            "Find files under a scoped directory with v1 glob output shape",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
            json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "max_results": { "type": "integer" }
                },
                "required": ["pattern"]
            }),
        )?,
        manifest(
            GREP_CAPABILITY_ID,
            "Search scoped file contents with v1 grep output modes",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
            json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "glob": { "type": "string" },
                    "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"] },
                    "context": { "type": "integer" },
                    "before_context": { "type": "integer" },
                    "after_context": { "type": "integer" },
                    "case_insensitive": { "type": "boolean" },
                    "head_limit": { "type": "integer" },
                    "offset": { "type": "integer" },
                    "multiline": { "type": "boolean" },
                    "type_filter": { "type": "string" }
                },
                "required": ["pattern"]
            }),
        )?,
        manifest(
            APPLY_PATCH_CAPABILITY_ID,
            "Apply exact/fuzzy search-replace edits through scoped mounts",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            PermissionMode::Allow,
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        )?,
    ])
}

fn manifest(
    id: &str,
    description: &str,
    effects: Vec<EffectKind>,
    default_permission: PermissionMode,
    parameters_schema: Value,
) -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(id)?,
        description: description.to_string(),
        effects,
        default_permission,
        parameters_schema,
        resource_profile: resource_profile(),
    })
}

pub(super) async fn dispatch(
    request: &FirstPartyCapabilityRequest,
    read_state: &SharedCodingReadState,
    edit_locks: &SharedCodingEditLocks,
) -> Result<Value, FirstPartyCapabilityError> {
    match request.capability_id.as_str() {
        READ_FILE_CAPABILITY_ID => read_file::read_file(request, read_state).await,
        WRITE_FILE_CAPABILITY_ID => write_file::write_file(request, read_state, edit_locks).await,
        LIST_DIR_CAPABILITY_ID => list_dir::list_dir(request).await,
        GLOB_CAPABILITY_ID => glob::glob(request).await,
        GREP_CAPABILITY_ID => grep::grep(request).await,
        APPLY_PATCH_CAPABILITY_ID => {
            apply_patch::apply_patch(request, read_state, edit_locks).await
        }
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

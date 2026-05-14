use ironclaw_filesystem::{FileType, FilesystemOperation};
use ironclaw_host_api::RuntimeDispatchErrorKind;
use serde_json::{Value, json};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{
    config::MAX_PATCH_SIZE,
    guest_error, input_error,
    inputs::required_str,
    paths::{
        filesystem_error, is_workspace_path, operation_allowed, resolve_required_path,
        stat_optional,
    },
    state::{SharedCodingEditLocks, SharedCodingReadState, content_hash, read_scope_key},
    text::{count_matches, decode_text, encode_text, reject_binary_probe, replace_content},
    types::MatchMethod,
};

pub(super) async fn apply_patch(
    request: &FirstPartyCapabilityRequest,
    read_state: &SharedCodingReadState,
    edit_locks: &SharedCodingEditLocks,
) -> Result<Value, FirstPartyCapabilityError> {
    let path_str = required_str(&request.input, "path")?;
    if is_workspace_path(path_str) {
        return Err(input_error());
    }
    let resolved = resolve_required_path(request, "path", FilesystemOperation::ReadFile)?;
    if !operation_allowed(&resolved.grant.permissions, FilesystemOperation::WriteFile) {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    let old_string = required_str(&request.input, "old_string")?;
    let new_string = required_str(&request.input, "new_string")?;
    if old_string == new_string {
        return Err(input_error());
    }
    let replace_all = request
        .input
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let scope = read_scope_key(request);
    let _edit_guard = edit_locks
        .lock_edit(&scope, resolved.virtual_path.as_str())
        .await;
    let stat = request
        .filesystem
        .stat(&resolved.virtual_path)
        .await
        .map_err(filesystem_error)?;
    if stat.sensitive {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    if stat.file_type != FileType::File || stat.len > MAX_PATCH_SIZE {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Resource,
        ));
    }
    let bytes = request
        .filesystem
        .read_file(&resolved.virtual_path)
        .await
        .map_err(filesystem_error)?;
    let current_hash = content_hash(&bytes);
    read_state.read().await.check_before_edit(
        &scope,
        resolved.virtual_path.as_str(),
        &current_hash,
    )?;
    reject_binary_probe(&bytes)?;
    let (content, encoding, line_ending) = decode_text(&bytes)?;
    let (match_count, match_method) = count_matches(&content, old_string);
    if match_count == 0 {
        return Err(guest_error());
    }
    if !replace_all && match_count > 1 {
        return Err(guest_error());
    }

    let (new_content, replacements) =
        replace_content(&content, old_string, new_string, replace_all, match_count)?;
    let output = encode_text(&new_content, encoding, line_ending);
    request
        .filesystem
        .write_file(&resolved.virtual_path, &output)
        .await
        .map_err(filesystem_error)?;
    if let Some(stat) = stat_optional(request, &resolved.virtual_path).await? {
        read_state.write().await.update_after_write(
            &scope,
            resolved.virtual_path.as_str(),
            stat.modified,
            content_hash(&output),
        );
    }
    let mut result = json!({
        "path": resolved.scoped_path.as_str(),
        "replacements": replacements,
        "success": true
    });
    if match_method != MatchMethod::Exact {
        result["match_method"] = json!(format!("{match_method:?}"));
    }
    Ok(result)
}

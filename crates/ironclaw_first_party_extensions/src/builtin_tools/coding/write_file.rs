use ironclaw_filesystem::FilesystemOperation;
use ironclaw_host_api::RuntimeDispatchErrorKind;
use serde_json::{Value, json};

use ironclaw_host_runtime::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{
    config::MAX_WRITE_SIZE,
    input_error,
    inputs::required_str,
    paths::{
        create_parent_dir, filesystem_error, is_workspace_path, resolve_required_path,
        stat_optional,
    },
    state::{SharedCodingEditLocks, SharedCodingReadState, content_hash, read_scope_key},
};

pub(super) async fn write_file(
    request: &FirstPartyCapabilityRequest,
    read_state: &SharedCodingReadState,
    edit_locks: &SharedCodingEditLocks,
) -> Result<Value, FirstPartyCapabilityError> {
    let path_str = required_str(&request.input, "path")?;
    if is_workspace_path(path_str) {
        return Err(input_error());
    }
    let resolved = resolve_required_path(request, "path", FilesystemOperation::WriteFile)?;
    let content = required_str(&request.input, "content")?;
    if content.len() > MAX_WRITE_SIZE {
        return Err(input_error());
    }
    let scope = read_scope_key(request);
    let _edit_guard = edit_locks
        .lock_edit(&scope, resolved.virtual_path.as_str())
        .await;
    if let Some(stat) = stat_optional(request, &resolved.virtual_path).await?
        && stat.sensitive
    {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    create_parent_dir(request, &resolved.virtual_path).await?;
    request
        .services
        .filesystem
        .write_file(&resolved.virtual_path, content.as_bytes())
        .await
        .map_err(filesystem_error)?;
    if let Some(stat) = stat_optional(request, &resolved.virtual_path).await? {
        read_state.write().await.update_after_write(
            &scope,
            resolved.virtual_path.as_str(),
            stat.modified,
            content_hash(content.as_bytes()),
        );
    }
    Ok(json!({
        "path": resolved.scoped_path.as_str(),
        "bytes_written": content.len(),
        "success": true
    }))
}

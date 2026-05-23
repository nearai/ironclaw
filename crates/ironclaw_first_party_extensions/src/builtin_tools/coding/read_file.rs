use ironclaw_filesystem::{FileType, FilesystemOperation};
use ironclaw_host_api::RuntimeDispatchErrorKind;
use serde_json::{Value, json};

use ironclaw_host_runtime::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{
    config::{DEFAULT_LINE_LIMIT, MAX_READ_SIZE},
    inputs::optional_usize,
    paths::{filesystem_error, resolve_required_path},
    state::{SharedCodingReadState, content_hash, read_scope_key},
    text::{decode_text, reject_binary_probe},
};

pub(super) async fn read_file(
    request: &FirstPartyCapabilityRequest,
    read_state: &SharedCodingReadState,
) -> Result<Value, FirstPartyCapabilityError> {
    let resolved = resolve_required_path(request, "path", FilesystemOperation::ReadFile)?;
    let offset = optional_usize(&request.input, "offset")?.unwrap_or(0);
    let limit = optional_usize(&request.input, "limit")?;
    let has_explicit_range = offset > 0 || limit.is_some();
    let stat = request
        .services
        .filesystem
        .stat(&resolved.virtual_path)
        .await
        .map_err(filesystem_error)?;
    if stat.sensitive {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    if stat.file_type != FileType::File || stat.len > MAX_READ_SIZE {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Resource,
        ));
    }

    let bytes = request
        .services
        .filesystem
        .read_file(&resolved.virtual_path)
        .await
        .map_err(filesystem_error)?;
    reject_binary_probe(&bytes)?;
    let (content, _encoding, _line_ending) = decode_text(&bytes)?;
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let start_line = offset.saturating_sub(1);
    let start_line = start_line.min(total_lines);
    let (end_line, truncated_by_default) = if let Some(limit) = limit {
        ((start_line + limit).min(total_lines), false)
    } else if !has_explicit_range && total_lines > DEFAULT_LINE_LIMIT {
        (DEFAULT_LINE_LIMIT.min(total_lines), true)
    } else {
        (total_lines, false)
    };
    let selected_lines: Vec<String> = lines[start_line..end_line]
        .iter()
        .enumerate()
        .map(|(index, line)| format!("{:>6}│ {}", start_line + index + 1, line))
        .collect();

    let partial = has_explicit_range || truncated_by_default;
    read_state.write().await.record_read(
        read_scope_key(request),
        resolved.virtual_path.as_str().to_string(),
        stat.modified,
        content_hash(&bytes),
        partial,
    );

    Ok(json!({
        "content": selected_lines.join("\n"),
        "total_lines": total_lines,
        "lines_shown": end_line - start_line,
        "truncated_by_default": truncated_by_default,
        "path": resolved.scoped_path.as_str()
    }))
}

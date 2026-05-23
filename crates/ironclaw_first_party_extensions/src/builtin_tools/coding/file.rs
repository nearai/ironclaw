//! Reborn first-party port of the v1 file coding tools.
//!
//! The v1 `Tool`/`JobContext`/local-filesystem boundary is replaced here with
//! `FirstPartyCapabilityRequest`, scoped mounts, and `RootFilesystem`.

use ironclaw_filesystem::{FileType, FilesystemOperation};
use ironclaw_host_api::RuntimeDispatchErrorKind;
use serde_json::{Value, json};

use ironclaw_host_runtime::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{
    config::{
        DEFAULT_LINE_LIMIT, MAX_DIR_ENTRIES, MAX_PATCH_SIZE, MAX_READ_SIZE, MAX_VISITED_ENTRIES,
        MAX_WRITE_SIZE,
    },
    guest_error, input_error,
    inputs::{optional_usize, required_str},
    paths::{
        create_parent_dir, filesystem_error, is_excluded_name, is_sensitive_scoped_path,
        is_workspace_path, operation_allowed, resolve_optional_path, resolve_required_path,
        scoped_child_path, stat_optional, virtual_to_relative,
    },
    state::{SharedCodingEditLocks, SharedCodingReadState, content_hash, read_scope_key},
    text::{count_matches, decode_text, encode_text, reject_binary_probe, replace_content},
    types::{ListEntry, MatchMethod, ResolvedPath},
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
    let start_line = offset.saturating_sub(1).min(total_lines);
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

pub(super) async fn list_dir(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let resolved = resolve_optional_path(request, FilesystemOperation::ListDir)?;
    let recursive = request
        .input
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_depth = optional_usize(&request.input, "max_depth")?.unwrap_or(3);
    let mut entries = collect_list_entries(request, &resolved, recursive, max_depth).await?;
    sort_list_entries(&mut entries);
    let truncated = entries.len() > MAX_DIR_ENTRIES;
    entries.truncate(MAX_DIR_ENTRIES);
    let count = entries.len();
    Ok(json!({
        "path": resolved.scoped_path.as_str(),
        "entries": entries.into_iter().map(|entry| entry.display).collect::<Vec<_>>(),
        "count": count,
        "truncated": truncated
    }))
}

async fn collect_list_entries(
    request: &FirstPartyCapabilityRequest,
    root: &ResolvedPath,
    recursive: bool,
    max_depth: usize,
) -> Result<Vec<ListEntry>, FirstPartyCapabilityError> {
    let mut output = Vec::new();
    let mut stack = vec![(root.virtual_path.clone(), 0usize)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let entries = request
            .services
            .filesystem
            .list_dir(&dir)
            .await
            .map_err(filesystem_error)?;
        for entry in entries {
            visited += 1;
            if visited > MAX_VISITED_ENTRIES {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::Resource,
                ));
            }
            let relative = virtual_to_relative(&root.virtual_path, &entry.path)?;
            let is_dir = entry.file_type == FileType::Directory;
            let scoped_path = scoped_child_path(&root.scoped_path, &relative);
            let is_sensitive = is_sensitive_scoped_path(&scoped_path);
            let display = if is_dir && recursive && is_sensitive {
                format!("{relative} [sensitive - access blocked]")
            } else if is_dir {
                format!("{relative}/")
            } else {
                // silent-ok: list_dir is best-effort for entries that disappear or fail stat.
                let Ok(stat) = request.services.filesystem.stat(&entry.path).await else {
                    tracing::debug!(
                        path = entry.path.as_str(),
                        "skipping list_dir entry after stat failed"
                    );
                    continue;
                };
                if is_sensitive || stat.sensitive {
                    continue;
                }
                format!("{} ({})", relative, format_size(stat.len))
            };
            output.push(ListEntry { display, is_dir });
            if recursive
                && is_dir
                && depth < max_depth
                && !is_sensitive
                && !is_excluded_name(entry.name.as_str())
            {
                stack.push((entry.path, depth + 1));
            }
            if output.len() > MAX_DIR_ENTRIES {
                return Ok(output);
            }
        }
    }
    Ok(output)
}

fn sort_list_entries(entries: &mut [ListEntry]) {
    entries.sort_by(|left, right| match (left.is_dir, right.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => left.display.cmp(&right.display),
    });
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes}B")
    }
}

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
    if stat.file_type != FileType::File || stat.len > MAX_PATCH_SIZE {
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
        .services
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

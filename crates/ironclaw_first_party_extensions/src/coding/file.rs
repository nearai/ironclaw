//! Reborn first-party port of the v1 file coding tools.
//!
//! The v1 `Tool`/`JobContext`/local-filesystem boundary is replaced here with
//! `CodingCapabilityRequest`, scoped mounts, and `RootFilesystem`.

use ironclaw_filesystem::{FileType, FilesystemOperation};
use ironclaw_host_api::RuntimeDispatchErrorKind;
use serde_json::{Value, json};

use super::{CodingCapabilityError, CodingCapabilityOutput, CodingCapabilityRequest};

use super::{
    config::{
        DEFAULT_LINE_LIMIT, DEFAULT_READ_MAX_BYTES, MAX_DIR_ENTRIES, MAX_PATCH_SIZE, MAX_READ_SIZE,
        MAX_VISITED_ENTRIES, MAX_WRITE_SIZE,
    },
    diff_preview::{file_diff_preview, will_use_large_diff_path},
    input_error,
    inputs::{optional_usize, required_str},
    operation_error_with_summary,
    patch::{parse_apply_patch_input, replacement_error},
    paths::{
        create_parent_dir_unless_sensitive, deny_sensitive_existing_path, filesystem_error,
        is_excluded_name, is_sensitive_scoped_path, is_workspace_path, operation_allowed,
        resolve_optional_path, resolve_required_path, scoped_child_path, stat_optional,
        virtual_to_relative,
    },
    state::{SharedCodingEditLocks, read_scope_key},
    text::{
        TextEdit, decode_text, decode_text_lossy, encode_text, previous_char_boundary,
        reject_binary_probe, reject_binary_probe_lenient, replace_content,
    },
    types::{ListEntry, MatchMethod, ResolvedPath},
};

pub(super) async fn read_file(
    request: &CodingCapabilityRequest<'_>,
) -> Result<Value, CodingCapabilityError> {
    let resolved = resolve_required_path(request, "path", FilesystemOperation::ReadFile)?;
    let offset = optional_usize(request.input, "offset")?.unwrap_or(0);
    let limit = optional_usize(request.input, "limit")?;
    let has_explicit_range = offset > 0 || limit.is_some();
    let stat = request
        .filesystem
        .stat(&resolved.virtual_path)
        .await
        .map_err(|error| {
            filesystem_error_with_summary("read_file", resolved.scoped_path.as_str(), error)
        })?;
    if stat.sensitive {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    if stat.file_type != FileType::File || stat.len > MAX_READ_SIZE {
        return Err(CodingCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::Resource,
            format!(
                "read_file failed for {}: target is not a readable file or exceeds the size limit",
                safe_summary_path(resolved.scoped_path.as_str())
            ),
        ));
    }

    let bytes = request
        .filesystem
        .read_file(&resolved.virtual_path)
        .await
        .map_err(|error| {
            filesystem_error_with_summary("read_file", resolved.scoped_path.as_str(), error)
        })?;

    let content = if should_extract_document_before_text(&bytes, resolved.scoped_path.as_str()) {
        match extract_document_text_for_read_file(&bytes, resolved.scoped_path.as_str())? {
            Some(content) => content,
            None => decode_read_file_text(&bytes)?,
        }
    } else {
        match decode_read_file_text(&bytes) {
            Ok(content) => content,
            Err(text_error) => {
                match extract_document_text_for_read_file(&bytes, resolved.scoped_path.as_str())? {
                    Some(content) => content,
                    None => return Err(text_error),
                }
            }
        }
    };

    Ok(read_file_text_output(
        &content,
        resolved.scoped_path.as_str(),
        offset,
        limit,
        has_explicit_range,
    ))
}

fn decode_read_file_text(bytes: &[u8]) -> Result<String, CodingCapabilityError> {
    // Read path is tolerant: reject only genuine (NUL-dense) binaries and decode
    // the rest lossily, so a text log with a stray NUL or non-UTF-8 byte is still
    // readable instead of hard-failing into a grep-only fallback. The patch path
    // keeps the strict probe/decode (byte fidelity for write-back).
    reject_binary_probe_lenient(bytes)?;
    let (content, _encoding, _line_ending) = decode_text_lossy(bytes);
    Ok(content)
}

fn should_extract_document_before_text(bytes: &[u8], scoped_path: &str) -> bool {
    let Some(extension) = scoped_path.rsplit('.').next().map(str::to_ascii_lowercase) else {
        return false;
    };
    match extension.as_str() {
        "pdf" => bytes.starts_with(b"%PDF-"),
        "docx" | "pptx" | "xlsx" => {
            bytes.starts_with(b"PK\x03\x04")
                || bytes.starts_with(b"PK\x05\x06")
                || bytes.starts_with(b"PK\x07\x08")
        }
        "doc" | "ppt" | "xls" => {
            bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
        }
        "rtf" => bytes.starts_with(br"{\rtf"),
        _ => false,
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReadTruncationReason {
    Bytes,
    Lines,
}

impl ReadTruncationReason {
    fn notice_label(self) -> &'static str {
        match self {
            Self::Bytes => "bytes",
            Self::Lines => "lines",
        }
    }
}

fn read_file_continuation_notice(
    start_line: usize,
    last_line_shown: usize,
    total_lines: usize,
    reason: ReadTruncationReason,
    next_offset: usize,
) -> String {
    format!(
        "[Showing lines {}-{} of {} ({} limit). Use offset={} to continue.]",
        start_line + 1,
        last_line_shown,
        total_lines,
        reason.notice_label(),
        next_offset
    )
}

fn read_file_continuation_suffix(
    start_line: usize,
    last_line_shown: usize,
    total_lines: usize,
    reason: ReadTruncationReason,
    next_offset: usize,
) -> String {
    format!(
        "\n\n{}",
        read_file_continuation_notice(
            start_line,
            last_line_shown,
            total_lines,
            reason,
            next_offset
        )
    )
}

fn read_file_text_output(
    content: &str,
    scoped_path: &str,
    offset: usize,
    limit: Option<usize>,
    has_explicit_range: bool,
) -> Value {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let start_line = offset.saturating_sub(1).min(total_lines);
    let (line_end, truncated_by_default) = if let Some(limit) = limit {
        (start_line.saturating_add(limit).min(total_lines), false)
    } else if !has_explicit_range && total_lines > DEFAULT_LINE_LIMIT {
        (DEFAULT_LINE_LIMIT.min(total_lines), true)
    } else {
        (total_lines, false)
    };

    // Render the selected line window with the line-number gutter, enforcing a
    // byte budget *on top of* the line cap so a handful of very long lines can't
    // dump hundreds of KB into the context. Truncation always lands on a complete
    // line; the model resumes past the cut with the returned `next_offset`.
    let mut rendered: Vec<String> = Vec::new();
    let mut emitted_bytes = 0usize;
    let mut truncated_by_bytes = false;
    for (index, line) in lines[start_line..line_end].iter().enumerate() {
        let formatted = format!("{:>6}│ {}", start_line + index + 1, line);
        let candidate_lines_shown = rendered.len() + 1;
        let candidate_last_line_shown = start_line + candidate_lines_shown;
        let candidate_has_more = candidate_last_line_shown < total_lines;
        let candidate_reason = if candidate_last_line_shown < line_end {
            ReadTruncationReason::Bytes
        } else {
            ReadTruncationReason::Lines
        };
        let candidate_notice_suffix = if candidate_has_more {
            read_file_continuation_suffix(
                start_line,
                candidate_last_line_shown,
                total_lines,
                candidate_reason,
                candidate_last_line_shown + 1,
            )
        } else {
            String::new()
        };
        // +1 for the newline that joins this line to the previous one.
        let cost = formatted.len() + usize::from(!rendered.is_empty());
        let candidate_total = emitted_bytes
            .saturating_add(cost)
            .saturating_add(candidate_notice_suffix.len());

        if candidate_total > DEFAULT_READ_MAX_BYTES {
            truncated_by_bytes = true;
            if rendered.is_empty() {
                // Return one clamped line instead of an empty body, while still
                // reserving room for the truncation marker and continuation note.
                let marker = " …[line truncated]";
                let clamp_budget = DEFAULT_READ_MAX_BYTES
                    .saturating_sub(marker.len())
                    .saturating_sub(candidate_notice_suffix.len());
                let clamp_to = previous_char_boundary(&formatted, clamp_budget);
                rendered.push(format!("{}{}", &formatted[..clamp_to], marker)); // safety: clamp_to is adjusted by previous_char_boundary.
            }
            break;
        }
        emitted_bytes += cost;
        rendered.push(formatted);
    }

    let lines_shown = rendered.len();
    let last_line_shown = start_line + lines_shown;
    let has_more = last_line_shown < total_lines;
    let next_offset = has_more.then_some(last_line_shown + 1);
    let truncated_by = if truncated_by_bytes {
        Some(ReadTruncationReason::Bytes)
    } else if has_more {
        Some(ReadTruncationReason::Lines)
    } else {
        None
    };

    let mut body = rendered.join("\n");
    if let (Some(reason), Some(next)) = (truncated_by, next_offset) {
        body.push_str(&read_file_continuation_suffix(
            start_line,
            last_line_shown,
            total_lines,
            reason,
            next,
        ));
    }

    json!({
        "content": body,
        "total_lines": total_lines,
        "lines_shown": lines_shown,
        "truncated_by_default": truncated_by_default,
        "truncated": truncated_by.is_some(),
        "truncated_by": truncated_by,
        "next_offset": next_offset,
        "path": scoped_path
    })
}

fn extract_document_text_for_read_file(
    bytes: &[u8],
    scoped_path: &str,
) -> Result<Option<String>, CodingCapabilityError> {
    let Some(text) =
        ironclaw_extractors::extract_document_text_by_filename(bytes, Some(scoped_path)).map_err(
            |error| {
                operation_error_with_summary(format!(
                    "read_file failed for {}: document text extraction failed: {error}",
                    safe_summary_path(scoped_path)
                ))
            },
        )?
    else {
        return Ok(None);
    };

    let text = text.trim();
    if text.is_empty() {
        return Err(operation_error_with_summary(format!(
            "read_file failed for {}: document text extraction yielded no text",
            safe_summary_path(scoped_path)
        )));
    }
    Ok(Some(text.to_string()))
}

pub(super) async fn write_file(
    request: &CodingCapabilityRequest<'_>,
    edit_locks: &SharedCodingEditLocks,
) -> Result<CodingCapabilityOutput, CodingCapabilityError> {
    let path_str = required_str(request.input, "path")?;
    if is_workspace_path(path_str) {
        return Err(input_error());
    }
    let resolved = resolve_required_path(request, "path", FilesystemOperation::WriteFile)?;
    let content = required_str(request.input, "content")?;
    if content.len() > MAX_WRITE_SIZE {
        return Err(input_error());
    }
    let scope = read_scope_key(request);
    let _edit_guard = edit_locks
        .lock_edit(&scope, resolved.virtual_path.as_str())
        .await;
    let existing_stat = stat_optional(request, &resolved.virtual_path).await?;
    if let Some(stat) = &existing_stat
        && stat.sensitive
    {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    // Skip reading the old file when the write-only permission is absent or when
    // new content alone would trigger the large-diff fast path in file_diff_preview
    // (the old file read would be wasted).
    let old_content =
        if !operation_allowed(&resolved.grant.permissions, FilesystemOperation::ReadFile)
            || will_use_large_diff_path(content)
        {
            None
        } else {
            existing_text_for_preview(request, &resolved, existing_stat.as_ref()).await
        };
    create_parent_dir_unless_sensitive(request, &resolved.virtual_path).await?;
    request
        .filesystem
        .write_file(&resolved.virtual_path, content.as_bytes())
        .await
        .map_err(filesystem_error)?;
    let output = json!({
        "path": resolved.scoped_path.as_str(),
        "bytes_written": content.len(),
        "success": true
    });
    let display_preview = old_content
        .map(|old_content| file_diff_preview(resolved.scoped_path.as_str(), &old_content, content));
    Ok(CodingCapabilityOutput::with_display_preview(
        output,
        display_preview,
    ))
}

pub(super) async fn list_dir(
    request: &CodingCapabilityRequest<'_>,
) -> Result<Value, CodingCapabilityError> {
    let resolved = resolve_optional_path(request, FilesystemOperation::ListDir)?;
    deny_sensitive_existing_path(request, &resolved.virtual_path).await?;
    let recursive = request
        .input
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_depth = optional_usize(request.input, "max_depth")?.unwrap_or(3);
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
    request: &CodingCapabilityRequest<'_>,
    root: &ResolvedPath,
    recursive: bool,
    max_depth: usize,
) -> Result<Vec<ListEntry>, CodingCapabilityError> {
    let mut output = Vec::new();
    let mut stack = vec![(root.virtual_path.clone(), 0usize)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let entries = request
            .filesystem
            .list_dir(&dir)
            .await
            .map_err(filesystem_error)?;
        for entry in entries {
            visited += 1;
            if visited > MAX_VISITED_ENTRIES {
                return Err(CodingCapabilityError::new(
                    RuntimeDispatchErrorKind::Resource,
                ));
            }
            let relative = virtual_to_relative(&root.virtual_path, &entry.path)?;
            let is_dir = entry.file_type == FileType::Directory;
            let scoped_path = scoped_child_path(&root.scoped_path, &relative);
            let is_sensitive = is_sensitive_scoped_path(&scoped_path);
            // silent-ok: list_dir is best-effort for entries that disappear or fail stat.
            let Ok(stat) = request.filesystem.stat(&entry.path).await else {
                tracing::debug!(
                    path = entry.path.as_str(),
                    "skipping list_dir entry after stat failed"
                );
                continue;
            };
            let is_sensitive = is_sensitive || stat.sensitive;
            let display = if is_dir && recursive && is_sensitive {
                format!("{relative} [sensitive - access blocked]")
            } else if is_dir && is_sensitive {
                continue;
            } else if is_dir {
                format!("{relative}/")
            } else {
                if is_sensitive {
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
    request: &CodingCapabilityRequest<'_>,
    edit_locks: &SharedCodingEditLocks,
) -> Result<CodingCapabilityOutput, CodingCapabilityError> {
    let path_str = required_str(request.input, "path")?;
    if is_workspace_path(path_str) {
        return Err(input_error());
    }
    let resolved = resolve_required_path(request, "path", FilesystemOperation::ReadFile)?;
    if !operation_allowed(&resolved.grant.permissions, FilesystemOperation::WriteFile) {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    let patch_input = parse_apply_patch_input(request.input)?;
    let scope = read_scope_key(request);
    let _edit_guard = edit_locks
        .lock_edit(&scope, resolved.virtual_path.as_str())
        .await;
    let stat = request
        .filesystem
        .stat(&resolved.virtual_path)
        .await
        .map_err(|error| {
            filesystem_error_with_summary("apply_patch", resolved.scoped_path.as_str(), error)
        })?;
    if stat.sensitive {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    if stat.file_type != FileType::File || stat.len > MAX_PATCH_SIZE {
        return Err(CodingCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::Resource,
            format!(
                "apply_patch failed for {}: target is not a file or exceeds the patch size limit",
                safe_summary_path(resolved.scoped_path.as_str())
            ),
        ));
    }
    let bytes = request
        .filesystem
        .read_file(&resolved.virtual_path)
        .await
        .map_err(|error| {
            filesystem_error_with_summary("apply_patch", resolved.scoped_path.as_str(), error)
        })?;
    reject_binary_probe(&bytes)?;
    let (content, encoding, line_ending) = decode_text(&bytes)?;
    let text_edits = patch_input
        .edits
        .iter()
        .map(|edit| TextEdit {
            old_string: edit.old_string.as_str(),
            new_string: edit.new_string.as_str(),
        })
        .collect::<Vec<_>>();
    let replacement =
        replace_content(&content, &text_edits, patch_input.replace_all).map_err(|error| {
            replacement_error(
                error,
                safe_summary_path(resolved.scoped_path.as_str()),
                patch_input.edits.len(),
            )
        })?;
    let output = encode_text(&replacement.content, encoding, line_ending);
    request
        .filesystem
        .write_file(&resolved.virtual_path, &output)
        .await
        .map_err(|error| {
            filesystem_error_with_summary("apply_patch", resolved.scoped_path.as_str(), error)
        })?;
    let mut result = json!({
        "path": resolved.scoped_path.as_str(),
        "replacements": replacement.replacements,
        "success": true
    });
    if replacement.match_method != MatchMethod::Exact {
        result["match_method"] = json!(replacement.match_method.as_wire_name());
    }
    let display_preview = file_diff_preview(
        resolved.scoped_path.as_str(),
        &content,
        &replacement.content,
    );
    Ok(CodingCapabilityOutput::with_display_preview(
        result,
        Some(display_preview),
    ))
}

fn filesystem_error_with_summary(
    operation: &str,
    scoped_path: &str,
    error: ironclaw_filesystem::FilesystemError,
) -> CodingCapabilityError {
    let scoped_path = safe_summary_path(scoped_path);
    let summary = match &error {
        ironclaw_filesystem::FilesystemError::NotFound { .. } => {
            format!("{operation} failed for {scoped_path}: file not found")
        }
        ironclaw_filesystem::FilesystemError::PermissionDenied { .. }
        | ironclaw_filesystem::FilesystemError::MountNotFound { .. }
        | ironclaw_filesystem::FilesystemError::PathOutsideMount { .. }
        | ironclaw_filesystem::FilesystemError::SymlinkEscape { .. }
        | ironclaw_filesystem::FilesystemError::MountConflict { .. }
        | ironclaw_filesystem::FilesystemError::VersionMismatch { .. }
        | ironclaw_filesystem::FilesystemError::Unsupported { .. }
        | ironclaw_filesystem::FilesystemError::IndexConflict { .. } => {
            format!("{operation} failed for {scoped_path}: permission denied or unsupported path")
        }
        ironclaw_filesystem::FilesystemError::Backend { .. }
        | ironclaw_filesystem::FilesystemError::BackendInfrastructure { .. } => {
            format!("{operation} failed for {scoped_path}: filesystem backend error")
        }
        ironclaw_filesystem::FilesystemError::Contract(_) => {
            format!("{operation} failed for {scoped_path}: invalid path")
        }
        _ => format!("{operation} failed for {scoped_path}: filesystem error"),
    };
    let kind = filesystem_error(error).kind();
    CodingCapabilityError::with_safe_summary(kind, summary)
}

fn safe_summary_path(scoped_path: &str) -> String {
    let path_hint = scoped_path
        .trim_start_matches('/')
        .replace(['/', '\\'], " ");
    format!("path {path_hint}")
}

async fn existing_text_for_preview(
    request: &CodingCapabilityRequest<'_>,
    resolved: &ResolvedPath,
    stat: Option<&ironclaw_filesystem::FileStat>,
) -> Option<String> {
    let Some(stat) = stat else {
        return Some(String::new());
    };
    if stat.file_type != FileType::File || stat.len > MAX_WRITE_SIZE as u64 {
        return None;
    }
    let bytes = request
        .filesystem
        .read_file(&resolved.virtual_path)
        .await
        // silent-ok: write_file display preview is best-effort; the write result is canonical.
        .ok()?;
    reject_binary_probe(&bytes).ok()?;
    let (content, _encoding, _line_ending) = decode_text(&bytes).ok()?;
    Some(content)
}

//! Reborn first-party port of the v1 file coding tools.
//!
//! The v1 `Tool`/`JobContext`/local-filesystem boundary is replaced here with
//! `CodingCapabilityRequest`, scoped mounts, and `RootFilesystem`.

use ironclaw_filesystem::{FileType, FilesystemOperation};
use ironclaw_host_api::dispatch::RuntimeDispatchErrorKind;
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
    operation_error, operation_error_with_summary,
    patch::{parse_apply_patch_input, replacement_error},
    paths::{
        create_parent_dir_unless_sensitive, filesystem_error, filesystem_error_with_summary,
        is_excluded_name, is_sensitive_scoped_path, is_workspace_path,
        list_dir_empty_if_missing_root, operation_allowed, resolve_optional_path,
        resolve_required_path, safe_summary_path, scoped_child_path, stat_optional,
        virtual_to_relative,
    },
    state::{
        CodingReadScopeKey, ReadRepresentation, SharedCodingEditLocks, SharedCodingReadStates,
        content_fingerprint, read_scope_key,
    },
    text::{
        TextEdit, decode_text, decode_text_lossy, encode_text, previous_char_boundary,
        reject_binary_probe, reject_binary_probe_lenient, replace_content,
    },
    types::{ListEntry, MatchMethod, ResolvedPath},
};

pub(super) async fn read_file(
    request: &CodingCapabilityRequest<'_>,
    read_states: &SharedCodingReadStates,
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

    // An OOXML document reads as its ADDRESSABLE STRUCTURE, not flattened text.
    // Flat extraction shows a redline's deleted text as though it were still in
    // the document — a model reviewing a contract that way reads the wrong
    // agreement — and gives back no ids to edit against. This is folded into
    // `read_file` rather than offered as a separate tool because a model
    // reaches for `read_file` on whatever path it is handed; a tool it must
    // know to prefer would mostly go unused.
    if let Some(format) =
        ironclaw_documents::DocumentFormat::from_path(resolved.scoped_path.as_str())
        && let Ok(view) = super::document::structured_document_view(format, &bytes)
    {
        let rendered = serde_json::to_string_pretty(&view).map_err(|error| {
            CodingCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::OperationFailed,
                format!(
                    "read_file failed for {}: {error}",
                    safe_summary_path(resolved.scoped_path.as_str())
                ),
            )
        })?;
        let output = read_file_text_output(
            &rendered,
            resolved.scoped_path.as_str(),
            offset,
            limit,
            has_explicit_range,
        );
        if !has_explicit_range && !read_output_truncated(&output) {
            read_states.record(
                &read_scope_key(request),
                resolved.virtual_path.as_str(),
                content_fingerprint(&bytes),
                ReadRepresentation::Structured,
            );
        }
        return Ok(output);
    }

    let (content, representation) =
        if should_extract_document_before_text(&bytes, resolved.scoped_path.as_str()) {
            match extract_document_text_for_read_file(&bytes, resolved.scoped_path.as_str())? {
                Some(content) => (content, ReadRepresentation::ExtractedText),
                None => (decode_read_file_text(&bytes)?, ReadRepresentation::RawText),
            }
        } else {
            match decode_read_file_text(&bytes) {
                Ok(content) => (content, ReadRepresentation::RawText),
                Err(text_error) => {
                    match extract_document_text_for_read_file(
                        &bytes,
                        resolved.scoped_path.as_str(),
                    )? {
                        Some(content) => (content, ReadRepresentation::ExtractedText),
                        None => return Err(text_error),
                    }
                }
            }
        };

    let output = read_file_text_output(
        &content,
        resolved.scoped_path.as_str(),
        offset,
        limit,
        has_explicit_range,
    );

    // Only a complete read proves the model has seen the whole file, which is
    // what unlocks write_file/apply_patch on it: no offset/limit window AND no
    // default line/byte truncation of the returned body. A truncated default
    // read of a large file must not authorize whole-file edits.
    if !has_explicit_range && !read_output_truncated(&output) {
        read_states.record(
            &read_scope_key(request),
            resolved.virtual_path.as_str(),
            content_fingerprint(&bytes),
            representation,
        );
    }

    Ok(output)
}

fn read_output_truncated(output: &Value) -> bool {
    // Fail safe: treat a missing/unreadable flag as truncated so an unproven
    // read never unlocks edits.
    output
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(true)
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
        "[Showing lines {}-{} of {} ({} limit). This file is large: to analyze, summarize, or \
         count over it, run ONE shell command or script (grep/awk/python) over the whole file and \
         read only the computed result — do NOT page through it (offset={}) and reason over it in \
         context, which is slow and costly.]",
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
    // A continuation notice only makes sense once at least one line was rendered.
    // An explicit `limit: 0` selects no lines, so there is nothing to continue
    // from and "[Showing lines 1-0 ...]" would be a nonsensical inverted range.
    // Suppressing the notice here keeps the zero-value path byte-for-byte v1.
    let has_more = lines_shown > 0 && last_line_shown < total_lines;
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
                // The safe summary is model-facing, so only `Display` may go in
                // it — `ExtractionError`'s `Display` is the classification and
                // nothing else, which is what makes this interpolation safe.
                // The parser diagnostic goes to the log via `Debug`.
                tracing::debug!(
                    path = %safe_summary_path(scoped_path),
                    ?error,
                    "read_file document text extraction failed"
                );
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
    read_states: &SharedCodingReadStates,
) -> Result<CodingCapabilityOutput, CodingCapabilityError> {
    let path_str = required_str(request.input, "path")?;
    if is_workspace_path(path_str) {
        return Err(input_error());
    }
    let resolved = resolve_required_path(request, "path", FilesystemOperation::WriteFile)?;
    if is_opaque_binary_document_path(resolved.scoped_path.as_str()) {
        return Err(binary_document_write_error(
            "write_file",
            resolved.scoped_path.as_str(),
        ));
    }
    let content = required_str(request.input, "content")?;
    if content.len() > MAX_WRITE_SIZE {
        return Err(input_error());
    }
    let scope = read_scope_key(request);
    let _edit_guard = edit_locks
        .lock_edit(scope.edit_lock_key(), resolved.virtual_path.as_str())
        .await;
    let existing_stat = stat_optional(request, &resolved.virtual_path).await?;
    if let Some(stat) = &existing_stat
        && stat.sensitive
    {
        return Err(CodingCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    // PDF is a text-authorable format (new-file creation is legitimate), but
    // an existing PDF cannot be safely overwritten via text tools once it has
    // been read as extracted text — the fingerprint bypass in issue #6898
    // applies to overwrites only. Block existing-PDF overwrites explicitly;
    // new PDF creation falls through to the normal write path.
    if let Some(stat) = &existing_stat
        && stat.file_type == FileType::File
        && is_pdf_document_path(resolved.scoped_path.as_str())
    {
        return Err(binary_document_write_error(
            "write_file",
            resolved.scoped_path.as_str(),
        ));
    }
    let can_read = operation_allowed(&resolved.grant.permissions, FilesystemOperation::ReadFile);
    // Overwriting an existing regular file requires a prior full read_file
    // whose fingerprint still matches the file's current bytes (blind
    // overwrites and mid-air collisions are the two bench-trace failure
    // modes). Write-only mounts are exempt: the model cannot read there, so
    // a blind overwrite is the only possible mode. New files need no read.
    let existing_bytes = match &existing_stat {
        Some(stat) if can_read && stat.file_type == FileType::File => Some(
            verify_read_before_edit(request, &resolved, read_states, &scope, "write_file", stat)
                .await?,
        ),
        _ => None,
    };
    // Skip decoding the old file when the read permission is absent or when
    // new content alone would trigger the large-diff fast path in
    // file_diff_preview (the decoded old content would be wasted).
    let old_content = if !can_read || will_use_large_diff_path(content) {
        None
    } else {
        match (&existing_stat, &existing_bytes) {
            (None, _) => Some(String::new()),
            (Some(stat), Some(bytes)) => existing_text_for_preview(stat, bytes),
            // Existing path that is not a regular file (e.g. a directory).
            (Some(_), None) => None,
        }
    };
    create_parent_dir_unless_sensitive(request, &resolved.virtual_path).await?;
    request
        .filesystem
        .write_file(&resolved.virtual_path, content.as_bytes())
        .await
        .map_err(filesystem_error)?;
    read_states.record(
        &scope,
        resolved.virtual_path.as_str(),
        content_fingerprint(content.as_bytes()),
        ReadRepresentation::RawText,
    );
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

/// Enforce read-before-edit on an existing file: a full `read_file` must have
/// recorded a fingerprint for this scope+path, and the file's current bytes
/// must still match it. Returns the verified bytes so callers can reuse them.
async fn verify_read_before_edit(
    request: &CodingCapabilityRequest<'_>,
    resolved: &ResolvedPath,
    read_states: &SharedCodingReadStates,
    scope: &CodingReadScopeKey,
    operation: &str,
    stat: &ironclaw_filesystem::FileStat,
) -> Result<Vec<u8>, CodingCapabilityError> {
    let Some(recorded) = read_states.recorded(scope, resolved.virtual_path.as_str()) else {
        return Err(read_before_edit_error(
            operation,
            resolved.scoped_path.as_str(),
        ));
    };
    if recorded.representation != ReadRepresentation::RawText {
        return Err(binary_document_write_error(
            operation,
            resolved.scoped_path.as_str(),
        ));
    }
    if is_opaque_binary_document_path(resolved.scoped_path.as_str())
        || is_pdf_document_path(resolved.scoped_path.as_str())
    {
        return Err(binary_document_write_error(
            operation,
            resolved.scoped_path.as_str(),
        ));
    }
    // A file grown past what read_file can return cannot match any recorded
    // read; report it as changed instead of fingerprinting unbounded bytes.
    if stat.len > MAX_READ_SIZE {
        return Err(stale_read_error(operation, resolved.scoped_path.as_str()));
    }
    let bytes = request
        .filesystem
        .read_file(&resolved.virtual_path)
        .await
        .map_err(|error| {
            filesystem_error_with_summary(operation, resolved.scoped_path.as_str(), error)
        })?;
    if content_fingerprint(&bytes) != recorded.fingerprint {
        return Err(stale_read_error(operation, resolved.scoped_path.as_str()));
    }
    // Backstop for binary targets the extension guards cannot name. Deliberately
    // the LENIENT probe: `read_file` admits text carrying a few stray NULs
    // (`reject_binary_probe_lenient` in `decode_read_file_text`), so the strict
    // probe here rejected a syslog the model had just read successfully — and
    // called it a "binary document". `apply_patch` still applies the strict
    // probe itself, where byte-fidelity for write-back demands it.
    reject_binary_probe_lenient(&bytes)
        .map_err(|_| binary_document_write_error(operation, resolved.scoped_path.as_str()))?;
    Ok(bytes)
}

fn lower_path_extension(scoped_path: &str) -> Option<String> {
    scoped_path.rsplit('.').next().map(str::to_ascii_lowercase)
}

fn is_opaque_binary_document_path(scoped_path: &str) -> bool {
    matches!(
        lower_path_extension(scoped_path).as_deref(),
        Some("doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx")
    )
}

fn is_pdf_document_path(scoped_path: &str) -> bool {
    matches!(lower_path_extension(scoped_path).as_deref(), Some("pdf"))
}

fn binary_document_write_error(operation: &str, scoped_path: &str) -> CodingCapabilityError {
    operation_error_with_summary(format!(
        "{operation} failed for {}: binary documents cannot be edited with text tools; use a document editing capability that preserves the original format",
        safe_summary_path(scoped_path)
    ))
}

pub(super) fn read_before_edit_error(operation: &str, scoped_path: &str) -> CodingCapabilityError {
    operation_error_with_summary(format!(
        "{operation} failed for {}: read it in full with read_file before editing it. Ranged reads (offset or limit) and default reads truncated at the line or byte cap do not count as having seen the whole file; a file too large to read in full cannot be edited with this tool",
        safe_summary_path(scoped_path)
    ))
}

pub(super) fn stale_read_error(operation: &str, scoped_path: &str) -> CodingCapabilityError {
    operation_error_with_summary(format!(
        "{operation} failed for {}: the file changed since it was last read; read it again with read_file before editing it",
        safe_summary_path(scoped_path)
    ))
}

pub(super) async fn list_dir(
    request: &CodingCapabilityRequest<'_>,
) -> Result<Value, CodingCapabilityError> {
    let resolved = resolve_optional_path(request, FilesystemOperation::ListDir)?;
    // A missing mount ROOT lists as empty (the grant names it; nothing has
    // been written under it yet), so the sensitive-stat guard tolerates its
    // absence. Any other missing path stays an error, as before.
    match stat_optional(request, &resolved.virtual_path).await? {
        Some(stat) if stat.sensitive => {
            return Err(CodingCapabilityError::new(
                RuntimeDispatchErrorKind::FilesystemDenied,
            ));
        }
        Some(_) => {}
        None if resolved.is_mount_root() => {}
        None => return Err(operation_error()),
    }
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
        let entries = list_dir_empty_if_missing_root(request, root, &dir).await?;
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
    read_states: &SharedCodingReadStates,
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
        .lock_edit(scope.edit_lock_key(), resolved.virtual_path.as_str())
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
    // Read-before-edit guard: patching requires a prior full read_file, and
    // the file must be unchanged since — a stale in-context view produces
    // wrong-anchor edits (mid-air collision). Shared with write_file's path so
    // both edit tools apply identical recorded-read, size-limit, re-read, and
    // fingerprint checks.
    let bytes = verify_read_before_edit(
        request,
        &resolved,
        read_states,
        &scope,
        "apply_patch",
        &stat,
    )
    .await?;
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
    read_states.record(
        &scope,
        resolved.virtual_path.as_str(),
        content_fingerprint(&output),
        ReadRepresentation::RawText,
    );
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

fn existing_text_for_preview(stat: &ironclaw_filesystem::FileStat, bytes: &[u8]) -> Option<String> {
    if stat.len > MAX_WRITE_SIZE as u64 {
        return None;
    }
    // silent-ok: write_file display preview is best-effort; the write result is canonical.
    reject_binary_probe(bytes).ok()?;
    let (content, _encoding, _line_ending) = decode_text(bytes).ok()?;
    Some(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: `read_file`'s extraction failure builds a **model-facing**
    /// safe summary, and before `ExtractionError` existed it interpolated the
    /// extractor's raw diagnostic string into it — the same string the
    /// extractor's own docs said callers must never render. This drives the
    /// call site, not `Display`: the wrapper is what composes the summary, so
    /// a unit test on the error type alone would not have caught it.
    #[test]
    fn read_file_extraction_failure_summary_carries_no_parser_detail() {
        // Bytes that look like a ZIP container but are not a valid DOCX, so
        // the private `extract_docx` produces a `zip`-crate diagnostic.
        let corrupt = b"PK\x03\x04not-a-real-docx";
        let error = extract_document_text_for_read_file(corrupt, "/notes/quarterly.docx")
            .expect_err("a corrupt DOCX must fail read_file");

        let summary = error
            .safe_summary()
            .expect("extraction failure must carry a safe summary");

        assert!(
            summary.contains("document text extraction failed"),
            "the model still needs to be told what went wrong, got {summary:?}"
        );
        assert!(
            summary.contains("path notes quarterly.docx"),
            "the redacted path hint is deliberate and must survive, got {summary:?}"
        );
        // The parser's own words must not be in it. `zip` reports invalid
        // archives as "invalid Zip archive: …"; the extractor wraps that as
        // "invalid Office XML archive: …". Neither may reach the model.
        for leaked in ["archive", "Zip", "zip", "invalid"] {
            assert!(
                !summary.contains(leaked),
                "parser diagnostic leaked into the model-facing summary \
                 ({leaked:?} in {summary:?})"
            );
        }
    }

    /// The other half of the same seam: a filename with no document extension
    /// is not a failure, it is "not my job" — `Ok(None)` so the caller falls
    /// through to its normal text read.
    #[test]
    fn read_file_extraction_returns_none_for_a_non_document_extension() {
        let result = extract_document_text_for_read_file(b"plain text", "/notes/todo.txt")
            .expect("a non-document extension is not an error");
        assert_eq!(result, None);
    }
}

use std::{
    cmp::Reverse,
    collections::BTreeSet,
    time::{SystemTime, UNIX_EPOCH},
};

use glob::Pattern;
use ironclaw_filesystem::{FileStat, FileType, FilesystemOperation};
use ironclaw_host_api::{RuntimeDispatchErrorKind, ScopedPath, VirtualPath};
use regex::RegexBuilder;
use serde_json::{Value, json};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{
    config::{DEFAULT_HEAD_LIMIT, MAX_OUTPUT_SIZE, MAX_READ_SIZE, MAX_VISITED_ENTRIES},
    input_error,
    inputs::{optional_usize, optional_usize_allow_zero, required_str},
    paths::{
        filesystem_error, is_excluded_name, is_sensitive_scoped_path, operation_allowed,
        resolve_optional_path, scoped_child_path, type_filter_matches, validate_relative_pattern,
        virtual_to_relative,
    },
    text::{decode_text, previous_char_boundary, reject_binary_probe},
    types::{GrepFileResult, GrepLine, ResolvedPath},
};

pub(super) async fn grep(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let resolved = resolve_optional_path(request, FilesystemOperation::Stat)?;
    if !operation_allowed(&resolved.grant.permissions, FilesystemOperation::ReadFile) {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    let root_stat = request
        .services
        .filesystem
        .stat(&resolved.virtual_path)
        .await
        .map_err(filesystem_error)?;
    if root_stat.sensitive {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    if root_stat.file_type == FileType::Directory
        && !operation_allowed(&resolved.grant.permissions, FilesystemOperation::ListDir)
    {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    if !matches!(root_stat.file_type, FileType::File | FileType::Directory) {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Resource,
        ));
    }
    let pattern = required_str(&request.input, "pattern")?;
    let output_mode = request
        .input
        .get("output_mode")
        .and_then(Value::as_str)
        .unwrap_or("files_with_matches");
    if !matches!(output_mode, "content" | "files_with_matches" | "count") {
        return Err(input_error());
    }
    let glob_filter = request.input.get("glob").and_then(Value::as_str);
    if let Some(filter) = glob_filter {
        validate_relative_pattern(filter)?;
    }
    let glob_filter = glob_filter
        .map(Pattern::new)
        .transpose()
        .map_err(|_| input_error())?;
    let type_filter = request.input.get("type_filter").and_then(Value::as_str);
    let case_insensitive = request
        .input
        .get("case_insensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let multiline = request
        .input
        .get("multiline")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .multi_line(true)
        .dot_matches_new_line(multiline)
        .build()
        .map_err(|_| input_error())?;
    let context = optional_usize(&request.input, "context")?;
    let before_context = if let Some(context) = context {
        context
    } else {
        optional_usize(&request.input, "before_context")?.unwrap_or(0)
    };
    let after_context = if let Some(context) = context {
        context
    } else {
        optional_usize(&request.input, "after_context")?.unwrap_or(0)
    };
    let head_limit = optional_usize_allow_zero(&request.input, "head_limit")?;
    let offset = optional_usize(&request.input, "offset")?.unwrap_or(0);
    let mut search_results = Vec::new();

    walk_files(
        request,
        &resolved,
        root_stat,
        |relative| {
            if let Some(filter) = &glob_filter
                && !filter.matches(relative)
            {
                return false;
            }
            if let Some(type_filter) = type_filter
                && !type_filter_matches(relative, type_filter)
            {
                return false;
            }
            true
        },
        |relative, bytes, modified| {
            if reject_binary_probe(bytes).is_err() {
                return Ok(true);
            }
            let Ok((content, _encoding, _line_ending)) = decode_text(bytes) else {
                return Ok(true);
            };
            if regex.is_match(&content) {
                let (line_matches, count) =
                    line_matches(&content, &regex, before_context, after_context, multiline);
                search_results.push(GrepFileResult {
                    relative: relative.to_string(),
                    modified,
                    count,
                    lines: line_matches,
                });
            }
            Ok(true)
        },
    )
    .await?;

    if output_mode == "files_with_matches" {
        search_results.sort_by(|left, right| {
            Reverse(left.modified.unwrap_or(UNIX_EPOCH))
                .cmp(&Reverse(right.modified.unwrap_or(UNIX_EPOCH)))
                .then_with(|| left.relative.cmp(&right.relative))
        });
    } else {
        search_results.sort_by(|left, right| left.relative.cmp(&right.relative));
    }
    Ok(build_grep_output(
        output_mode,
        search_results,
        offset,
        head_limit,
        before_context > 0 || after_context > 0,
    ))
}

async fn walk_files(
    request: &FirstPartyCapabilityRequest,
    root: &ResolvedPath,
    root_stat: FileStat,
    mut include: impl FnMut(&str) -> bool,
    mut visit: impl FnMut(&str, &[u8], Option<SystemTime>) -> Result<bool, FirstPartyCapabilityError>,
) -> Result<(), FirstPartyCapabilityError> {
    let mut total_bytes = 0u64;
    if root_stat.file_type == FileType::File {
        let relative = root_file_relative(&root.scoped_path);
        if include(&relative)
            && !visit_file(
                request,
                &root.virtual_path,
                &relative,
                root_stat,
                &mut total_bytes,
                &mut visit,
            )
            .await?
        {
            return Ok(());
        }
        return Ok(());
    }

    let mut stack = vec![root.virtual_path.clone()];
    let mut visited = 0usize;
    while let Some(dir) = stack.pop() {
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
            let scoped_path = scoped_child_path(&root.scoped_path, &relative);
            if is_sensitive_scoped_path(&scoped_path) {
                continue;
            }
            match entry.file_type {
                FileType::Directory => {
                    if !is_excluded_name(entry.name.as_str()) {
                        stack.push(entry.path);
                    }
                }
                FileType::File => {
                    if !include(&relative) {
                        continue;
                    }
                    let stat = request
                        .services
                        .filesystem
                        .stat(&entry.path)
                        .await
                        .map_err(filesystem_error)?;
                    if stat.sensitive {
                        continue;
                    }
                    if !visit_file(
                        request,
                        &entry.path,
                        &relative,
                        stat,
                        &mut total_bytes,
                        &mut visit,
                    )
                    .await?
                    {
                        return Ok(());
                    }
                }
                FileType::Symlink | FileType::Other => {}
            }
        }
    }
    Ok(())
}

async fn visit_file(
    request: &FirstPartyCapabilityRequest,
    path: &VirtualPath,
    relative: &str,
    stat: FileStat,
    total_bytes: &mut u64,
    visit: &mut impl FnMut(&str, &[u8], Option<SystemTime>) -> Result<bool, FirstPartyCapabilityError>,
) -> Result<bool, FirstPartyCapabilityError> {
    if stat.len > MAX_READ_SIZE {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Resource,
        ));
    }
    *total_bytes = total_bytes.saturating_add(stat.len);
    if *total_bytes > 16 * 1024 * 1024 {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Resource,
        ));
    }
    let bytes = request
        .services
        .filesystem
        .read_file(path)
        .await
        .map_err(filesystem_error)?;
    visit(relative, &bytes, stat.modified)
}

fn root_file_relative(path: &ScopedPath) -> String {
    path.as_str()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn build_grep_output(
    output_mode: &str,
    mut results: Vec<GrepFileResult>,
    offset: usize,
    head_limit: Option<usize>,
    had_context: bool,
) -> Value {
    let effective_limit = match head_limit {
        Some(0) => usize::MAX,
        Some(value) => value,
        None => DEFAULT_HEAD_LIMIT,
    };
    match output_mode {
        "files_with_matches" => {
            let total = results.len();
            let files = results
                .into_iter()
                .skip(offset)
                .take(effective_limit)
                .map(|result| result.relative)
                .collect::<Vec<_>>();
            json!({
                "files": files,
                "count": files.len(),
                "truncated": total > offset.saturating_add(effective_limit)
            })
        }
        "count" => {
            let total_count = results.len();
            let page = results
                .drain(..)
                .skip(offset)
                .take(effective_limit)
                .collect::<Vec<_>>();
            let total = page.iter().map(|result| result.count).sum::<usize>();
            json!({
                "counts": page.into_iter().map(|result| json!({
                    "file": result.relative,
                    "count": result.count
                })).collect::<Vec<_>>(),
                "total": total,
                "truncated": total_count > offset.saturating_add(effective_limit)
            })
        }
        _ => {
            let mut lines = Vec::new();
            for result in results {
                for line in result.lines {
                    let separator = if line.is_match || !had_context {
                        ':'
                    } else {
                        '-'
                    };
                    lines.push(format!(
                        "{}{}{}{}{}",
                        result.relative, separator, line.number, separator, line.text
                    ));
                }
            }
            let raw_len = lines.iter().map(|line| line.len() + 1).sum::<usize>();
            let page = lines
                .iter()
                .skip(offset)
                .take(effective_limit)
                .cloned()
                .collect::<Vec<_>>();
            let mut content = page.join("\n");
            let mut truncated =
                raw_len > MAX_OUTPUT_SIZE || lines.len() > offset.saturating_add(effective_limit);
            if content.len() > MAX_OUTPUT_SIZE {
                content.truncate(previous_char_boundary(&content, MAX_OUTPUT_SIZE));
                truncated = true;
            }
            json!({ "content": content, "truncated": truncated })
        }
    }
}

fn line_matches(
    content: &str,
    regex: &regex::Regex,
    before_context: usize,
    after_context: usize,
    multiline: bool,
) -> (Vec<GrepLine>, usize) {
    if multiline {
        return multiline_matches(content, regex, before_context, after_context);
    }

    let lines = content.lines().collect::<Vec<_>>();
    let mut include = BTreeSet::new();
    let mut matched = BTreeSet::new();
    for (index, line) in lines.iter().enumerate() {
        if regex.is_match(line) {
            matched.insert(index);
            let start = index.saturating_sub(before_context);
            let end = (index + after_context + 1).min(lines.len());
            for item in start..end {
                include.insert(item);
            }
        }
    }
    let count = matched.len();
    let lines = include
        .into_iter()
        .map(|index| GrepLine {
            number: index + 1,
            text: lines[index].to_string(),
            is_match: matched.contains(&index) || (before_context == 0 && after_context == 0),
        })
        .collect::<Vec<_>>();
    (lines, count)
}

fn multiline_matches(
    content: &str,
    regex: &regex::Regex,
    before_context: usize,
    after_context: usize,
) -> (Vec<GrepLine>, usize) {
    let indexed = indexed_lines(content);
    let mut include = BTreeSet::new();
    let mut matched = BTreeSet::new();
    let mut count = 0usize;

    for item in regex.find_iter(content) {
        count += 1;
        for (index, (_text, start, end)) in indexed.iter().enumerate() {
            if spans_overlap(item.start(), item.end(), *start, *end) {
                matched.insert(index);
                let context_start = index.saturating_sub(before_context);
                let context_end = (index + after_context + 1).min(indexed.len());
                for line_index in context_start..context_end {
                    include.insert(line_index);
                }
            }
        }
    }

    let lines = include
        .into_iter()
        .map(|index| GrepLine {
            number: index + 1,
            text: indexed[index].0.clone(),
            is_match: matched.contains(&index) || (before_context == 0 && after_context == 0),
        })
        .collect::<Vec<_>>();
    (lines, count)
}

fn indexed_lines(content: &str) -> Vec<(String, usize, usize)> {
    let mut output = Vec::new();
    let mut start = 0usize;
    for segment in content.split_inclusive('\n') {
        let end = start + segment.len();
        let text = segment.trim_end_matches('\n').to_string();
        output.push((text, start, end));
        start = end;
    }
    if output.is_empty() && content.is_empty() {
        output.push((String::new(), 0, 0));
    } else if start < content.len() {
        output.push((content[start..].to_string(), start, content.len()));
    }
    output
}

fn spans_overlap(match_start: usize, match_end: usize, line_start: usize, line_end: usize) -> bool {
    if match_start == match_end {
        return match_start >= line_start && match_start <= line_end;
    }
    match_start < line_end && match_end > line_start
}

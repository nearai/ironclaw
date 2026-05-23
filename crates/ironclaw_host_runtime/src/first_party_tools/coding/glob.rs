use glob::Pattern;
use ironclaw_filesystem::{DirEntry, FileType, FilesystemOperation};
use ironclaw_host_api::RuntimeDispatchErrorKind;
use serde_json::{Value, json};
use std::{cmp::Reverse, time::UNIX_EPOCH};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{
    config::{DEFAULT_MAX_RESULTS, GLOB_MATCH_OPTIONS, MAX_VISITED_ENTRIES},
    input_error,
    inputs::{optional_usize, required_str},
    paths::{
        filesystem_error, is_excluded_name, is_excluded_relative_path, is_sensitive_scoped_path,
        resolve_optional_path, scoped_child_path, validate_relative_pattern, virtual_to_relative,
    },
    types::ResolvedPath,
};

pub(super) async fn glob(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let start = std::time::Instant::now();
    let pattern = required_str(&request.input, "pattern")?;
    validate_relative_pattern(pattern)?;
    let resolved = resolve_optional_path(request, FilesystemOperation::ListDir)?;
    let max_results = optional_usize(&request.input, "max_results")?.unwrap_or(DEFAULT_MAX_RESULTS);
    let pattern = Pattern::new(pattern).map_err(|_| input_error())?;
    let mut files = Vec::new();
    walk_entries(request, &resolved, |entry, relative| {
        let scoped_path = scoped_child_path(&resolved.scoped_path, relative);
        if entry.file_type == FileType::File
            && !is_excluded_relative_path(relative)
            && !is_sensitive_scoped_path(&scoped_path)
            && pattern.matches_with(relative, GLOB_MATCH_OPTIONS)
        {
            files.push((relative.to_string(), entry.path.clone()));
        }
        Ok(true)
    })
    .await?;
    let mut files_with_mtime = Vec::with_capacity(files.len());
    for (relative, path) in files {
        let stat = request
            .services
            .filesystem
            .stat(&path)
            .await
            .map_err(filesystem_error)?;
        if stat.sensitive {
            continue;
        }
        let modified = stat.modified.unwrap_or(UNIX_EPOCH);
        files_with_mtime.push((relative, modified));
    }
    files_with_mtime.sort_by_key(|entry| Reverse(entry.1));
    let truncated = files_with_mtime.len() > max_results;
    files_with_mtime.truncate(max_results);
    let files = files_with_mtime
        .into_iter()
        .map(|(relative, _)| relative)
        .collect::<Vec<_>>();
    let count = files.len();
    Ok(json!({
        "files": files,
        "count": count,
        "truncated": truncated,
        "duration_ms": start.elapsed().as_millis() as u64
    }))
}

async fn walk_entries(
    request: &FirstPartyCapabilityRequest,
    root: &ResolvedPath,
    mut visit: impl FnMut(&DirEntry, &str) -> Result<bool, FirstPartyCapabilityError>,
) -> Result<(), FirstPartyCapabilityError> {
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
            let keep_going = visit(&entry, &relative)?;
            let scoped_path = scoped_child_path(&root.scoped_path, &relative);
            if entry.file_type == FileType::Directory
                && !is_excluded_name(entry.name.as_str())
                && !is_sensitive_scoped_path(&scoped_path)
            {
                stack.push(entry.path.clone());
            }
            if !keep_going {
                return Ok(());
            }
        }
    }
    Ok(())
}

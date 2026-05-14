use ironclaw_filesystem::{FileType, FilesystemOperation};
use ironclaw_host_api::RuntimeDispatchErrorKind;
use serde_json::{Value, json};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{
    config::{MAX_DIR_ENTRIES, MAX_VISITED_ENTRIES},
    inputs::optional_usize,
    paths::{
        filesystem_error, is_excluded_name, is_sensitive_scoped_path, resolve_optional_path,
        scoped_child_path, virtual_to_relative,
    },
    types::{ListEntry, ResolvedPath},
};

pub(super) async fn list_dir(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let start = std::time::Instant::now();
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
    let _duration = start.elapsed();
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
                let stat = request
                    .filesystem
                    .stat(&entry.path)
                    .await
                    .map_err(filesystem_error)?;
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

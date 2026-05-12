use ironclaw_host_api::{HostApiError, VirtualPath};

use crate::{DirEntry, FileType, FilesystemError, FilesystemOperation};

#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn directory_write_error(path: VirtualPath) -> FilesystemError {
    FilesystemError::Backend {
        path,
        operation: FilesystemOperation::WriteFile,
        reason: "cannot overwrite a directory".to_string(),
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn directory_append_error(path: VirtualPath) -> FilesystemError {
    FilesystemError::Backend {
        path,
        operation: FilesystemOperation::AppendFile,
        reason: "cannot append to a directory".to_string(),
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn virtual_path_prefixes(path: &VirtualPath) -> Result<Vec<VirtualPath>, HostApiError> {
    let mut prefixes = Vec::new();
    let mut current = String::new();
    for segment in path.as_str().trim_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        current.push('/');
        current.push_str(segment);
        prefixes.push(VirtualPath::new(current.clone())?);
    }
    Ok(prefixes)
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn direct_children(
    parent: &VirtualPath,
    rows: Vec<(VirtualPath, u64, FileType)>,
) -> Result<Vec<DirEntry>, FilesystemError> {
    let mut entries = std::collections::BTreeMap::<String, DirEntry>::new();
    let prefix = format!("{}/", parent.as_str().trim_end_matches('/'));
    for (path, _len, row_file_type) in rows {
        let Some(tail) = path.as_str().strip_prefix(&prefix) else {
            continue;
        };
        if tail.is_empty() {
            continue;
        }
        let (name, file_type) = if let Some((directory, _rest)) = tail.split_once('/') {
            (directory.to_string(), FileType::Directory)
        } else {
            (tail.to_string(), row_file_type)
        };
        let entry_path = VirtualPath::new(format!(
            "{}/{}",
            parent.as_str().trim_end_matches('/'),
            name
        ))?;
        entries.entry(name.clone()).or_insert(DirEntry {
            name,
            path: entry_path,
            file_type,
        });
    }
    if entries.is_empty() {
        return Err(not_found(parent.clone(), FilesystemOperation::ListDir));
    }
    Ok(entries.into_values().collect())
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn child_path_like_pattern(path: &VirtualPath) -> String {
    let mut pattern = String::new();
    for character in path.as_str().trim_end_matches('/').chars() {
        match character {
            '!' | '%' | '_' => {
                pattern.push('!');
                pattern.push(character);
            }
            _ => pattern.push(character),
        }
    }
    pattern.push_str("/%");
    pattern
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn not_found(path: VirtualPath, operation: FilesystemOperation) -> FilesystemError {
    FilesystemError::NotFound { path, operation }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn is_not_found<T>(result: &Result<T, FilesystemError>) -> bool {
    matches!(result, Err(FilesystemError::NotFound { .. }))
}

#[cfg(feature = "postgres")]
pub(crate) fn db_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    error: tokio_postgres::Error,
) -> FilesystemError {
    FilesystemError::Backend {
        path,
        operation,
        reason: error.to_string(),
    }
}

#[cfg(feature = "libsql")]
pub(crate) fn libsql_db_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    error: libsql::Error,
) -> FilesystemError {
    FilesystemError::Backend {
        path,
        operation,
        reason: error.to_string(),
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn valid_engine_path() -> VirtualPath {
    VirtualPath::new("/engine").unwrap_or_else(|_| unreachable!("literal virtual path is valid"))
}

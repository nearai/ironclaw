use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
pub(crate) fn system_time_from_unix_seconds(seconds: i64) -> Option<SystemTime> {
    if seconds < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(seconds as u64))
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn valid_engine_path() -> VirtualPath {
    VirtualPath::new("/engine").unwrap_or_else(|_| unreachable!("literal virtual path is valid"))
}

/// Build a deterministic SQL index identifier from a mount prefix + spec
/// name. `IndexKey`/`IndexName` are validated to `[A-Za-z_][A-Za-z0-9_]*`,
/// so the only non-identifier characters we strip are the prefix's slashes.
/// Length capped at 62 to fit Postgres' 63-char identifier cap so libsql
/// and postgres share one naming convention.
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn sql_index_name(prefix: &str, name: &str) -> String {
    let prefix_clean: String = prefix
        .trim_matches('/')
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let raw = format!("idx_rfs_{prefix_clean}_{name}");
    if raw.len() > 62 {
        let cutoff = raw
            .char_indices()
            .nth(62)
            .map(|(i, _)| i)
            .unwrap_or(raw.len());
        raw[..cutoff].to_string()
    } else {
        raw
    }
}

/// Escape a LIKE pattern that already contains a trailing `%` wildcard
/// **intentionally appended by the caller** (the path-prefix scan case in
/// `query`). The trailing `%` is preserved so it remains a wildcard;
/// every other `%`, `_`, and `!` is escaped.
///
/// Reviewer note (PR #3661): this is **NOT** suitable for user-supplied
/// LIKE input — use [`escape_like_literal`] instead, which escapes every
/// special character including a trailing `%`.
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn escape_like_with_trailing_wildcard(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '%' if chars.peek().is_some() => out.push_str("!%"),
            '_' => out.push_str("!_"),
            '!' => out.push_str("!!"),
            other => out.push(other),
        }
    }
    out
}

/// Fully-literal LIKE escape for user-supplied values. PR #3661 reviewer
/// fix: a literal prefix like `tenant:%` must not become a wildcard at
/// query time, so every `%`, `_`, and `!` is escaped.
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn escape_like_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '%' => out.push_str("!%"),
            '_' => out.push_str("!_"),
            '!' => out.push_str("!!"),
            other => out.push(other),
        }
    }
    out
}

/// Convert a raw `i64` version column into a [`RecordVersion`].
///
/// PR #3659 reviewer fix: previously sites used `version_raw.max(0) as u64`,
/// which silently masked a corrupt negative version to `0` — indistinguishable
/// from a legitimately fresh row. This helper surfaces the corruption as
/// [`FilesystemError::CorruptRecordVersion`] so the operator sees it.
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn record_version_from_i64(
    path: &VirtualPath,
    raw: i64,
) -> Result<crate::RecordVersion, FilesystemError> {
    u64::try_from(raw)
        .map(crate::RecordVersion::from_backend)
        .map_err(|_| FilesystemError::CorruptRecordVersion {
            path: path.clone(),
            raw,
        })
}

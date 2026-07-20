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

#[cfg(feature = "libsql")]
pub(crate) fn descendant_path_range(path: &VirtualPath) -> (String, String) {
    let prefix = path.as_str().trim_end_matches('/');
    // Descendants share the literal "{prefix}/" component boundary. The
    // exclusive upper bound "{prefix}0" works because '/' sorts before '0'
    // in the normalized virtual path alphabet used by these storage paths.
    (format!("{prefix}/"), format!("{prefix}0"))
}

#[cfg(feature = "libsql")]
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
    if error.code().is_some_and(is_postgres_contention) {
        return FilesystemError::BackendBusy { path, operation };
    }
    FilesystemError::Backend {
        path,
        operation,
        reason: error.to_string(),
    }
}

#[cfg(feature = "postgres")]
fn is_postgres_contention(code: &tokio_postgres::error::SqlState) -> bool {
    code == &tokio_postgres::error::SqlState::T_R_SERIALIZATION_FAILURE
        || code == &tokio_postgres::error::SqlState::T_R_DEADLOCK_DETECTED
        || code == &tokio_postgres::error::SqlState::LOCK_NOT_AVAILABLE
}

#[cfg(feature = "libsql")]
pub(crate) fn libsql_db_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    error: libsql::Error,
) -> FilesystemError {
    if is_libsql_contention(&error) {
        return FilesystemError::BackendBusy { path, operation };
    }
    FilesystemError::Backend {
        path,
        operation,
        reason: error.to_string(),
    }
}

#[cfg(feature = "libsql")]
fn is_libsql_contention(error: &libsql::Error) -> bool {
    const SQLITE_BUSY: i32 = 5;
    const SQLITE_LOCKED: i32 = 6;

    let primary_code = match error {
        libsql::Error::SqliteFailure(code, _) => Some(*code),
        libsql::Error::RemoteSqliteFailure(code, extended_code, _) => {
            Some(if *code == 0 { *extended_code } else { *code })
        }
        _ => None,
    };
    primary_code.is_some_and(|code| matches!(code & 0xff, SQLITE_BUSY | SQLITE_LOCKED))
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn system_time_from_unix_seconds(seconds: i64) -> Option<SystemTime> {
    if seconds < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(seconds as u64))
}

/// Build a [`FilesystemError::BackendInfrastructure`] for a failure that
/// happens outside any caller-supplied path scope (pool acquisition,
/// `run_migrations`, pragma setup, schema bootstrapping). The previous
/// `valid_engine_path()` helper returned a `/engine` placeholder so the
/// path-bearing [`FilesystemError::Backend`] variant could be used; that
/// placeholder masked which subsystem actually failed, so a real failure
/// reported a fictional path. Backends now use this helper instead, and
/// the variant explicitly omits `path`.
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn infrastructure_error(
    operation: FilesystemOperation,
    reason: impl Into<String>,
) -> FilesystemError {
    FilesystemError::BackendInfrastructure {
        operation,
        reason: reason.into(),
    }
}

#[cfg(feature = "postgres")]
pub(crate) fn infrastructure_pg_error(
    operation: FilesystemOperation,
    error: tokio_postgres::Error,
) -> FilesystemError {
    let reason = format!("postgres root filesystem infrastructure error: {error}");
    tracing::debug!(
        %operation,
        %reason,
        "postgres root filesystem infrastructure error"
    );
    infrastructure_error(operation, reason)
}

#[cfg(feature = "libsql")]
pub(crate) fn infrastructure_libsql_error(
    operation: FilesystemOperation,
    error: libsql::Error,
) -> FilesystemError {
    infrastructure_error(operation, error.to_string())
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
    if raw.len() <= 62 {
        return raw;
    }
    // PR #3679 review fix: distinct long inputs must map to distinct
    // identifiers. Append a stable blake3 suffix before truncating so
    // `CREATE ... IF NOT EXISTS` cannot silently reuse the wrong index.
    let hash = blake3::hash(raw.as_bytes());
    let hash_hex = hash.to_hex();
    let suffix = format!("_{}", &hash_hex.as_str()[..8]);
    let keep = 62usize.saturating_sub(suffix.len());
    let cutoff = raw
        .char_indices()
        .nth(keep)
        .map(|(i, _)| i)
        .unwrap_or(raw.len());
    let mut out = raw[..cutoff].to_string();
    out.push_str(&suffix);
    out
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

/// Convert a u64 [`RecordVersion`] value into the i64 SQL binding both
/// backends use. Audit finding F6: the prior `expected.get() as i64` cast
/// silently wraps for `RecordVersion` values ≥ 2^63 (a corrupt or
/// future-large version), producing a negative bind parameter that the
/// `WHERE version = ?` clause would never match. Surface
/// `CorruptRecordVersion` instead of letting the write silently
/// VersionMismatch.
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn record_version_to_i64(
    path: &VirtualPath,
    version: crate::RecordVersion,
) -> Result<i64, FilesystemError> {
    i64::try_from(version.get()).map_err(|_| FilesystemError::CorruptRecordVersion {
        path: path.clone(),
        raw: version.get() as i64,
    })
}

/// Convert a u64 [`Page::offset`](crate::Page::offset) into the i64 SQL
/// binding both backends use. Audit finding F6: `page.offset as i64`
/// wraps for offsets ≥ 2^63, producing a negative `OFFSET` that SQLite
/// rejects with a cryptic backend error and Postgres rejects loudly but
/// without naming what overflowed. Surface a typed `Backend` error
/// naming the operation and value instead.
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub(crate) fn page_offset_to_i64(path: &VirtualPath, offset: u64) -> Result<i64, FilesystemError> {
    i64::try_from(offset).map_err(|_| FilesystemError::Backend {
        path: path.clone(),
        operation: FilesystemOperation::Query,
        reason: format!("page offset {offset} exceeds backend i64 range"),
    })
}

#[cfg(all(test, feature = "libsql"))]
mod tests {
    use super::*;

    #[test]
    fn libsql_busy_and_locked_errors_preserve_retryable_contention() {
        let path = VirtualPath::new("/resources/deltas/log").expect("valid path"); // safety: test-only fixture

        for error in [
            libsql::Error::SqliteFailure(5, "database is busy".to_string()),
            libsql::Error::SqliteFailure(6, "database is locked".to_string()),
            libsql::Error::SqliteFailure(261, "extended busy code".to_string()),
            libsql::Error::RemoteSqliteFailure(5, 517, "remote database is busy".to_string()),
            libsql::Error::RemoteSqliteFailure(0, 262, "remote table is locked".to_string()),
        ] {
            let error = libsql_db_error(path.clone(), FilesystemOperation::Append, error);

            assert!(matches!( // safety: test-only assertion
                error,
                FilesystemError::BackendBusy {
                    path: error_path,
                    operation: FilesystemOperation::Append,
                } if error_path == path
            ));
        }
    }

    #[test]
    fn libsql_non_contention_errors_remain_terminal_backend_errors() {
        let path = VirtualPath::new("/resources/deltas/log").expect("valid path"); // safety: test-only fixture
        let error = libsql_db_error(
            path.clone(),
            FilesystemOperation::Append,
            libsql::Error::SqliteFailure(11, "database disk image is malformed".to_string()),
        );

        assert!(matches!( // safety: test-only assertion
            error,
            FilesystemError::Backend {
                path: error_path,
                operation: FilesystemOperation::Append,
                ..
            } if error_path == path
        ));
    }
}

#[cfg(all(test, feature = "postgres"))]
mod postgres_tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use tokio_postgres::error::SqlState;

    use super::*;

    #[test]
    fn postgres_transient_write_conflicts_are_retryable_contention() {
        for code in [
            SqlState::T_R_SERIALIZATION_FAILURE,
            SqlState::T_R_DEADLOCK_DETECTED,
            SqlState::LOCK_NOT_AVAILABLE,
        ] {
            assert!(is_postgres_contention(&code));
        }

        assert!(!is_postgres_contention(&SqlState::UNIQUE_VIOLATION));
    }

    #[tokio::test]
    async fn postgres_real_lock_not_available_maps_to_backend_busy_when_configured() {
        if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
            return;
        }
        let url = std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL")
            .or_else(|_| std::env::var("DATABASE_URL"));
        let Ok(url) = url else {
            return;
        };
        let (locker, locker_connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("configured Postgres locker connection");
        let (contender, contender_connection) =
            tokio_postgres::connect(&url, tokio_postgres::NoTls)
                .await
                .expect("configured Postgres contender connection");
        let locker_task = tokio::spawn(async move {
            let _ = locker_connection.await;
        });
        let contender_task = tokio::spawn(async move {
            let _ = contender_connection.await;
        });
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let table = format!("ironclaw_lock_mapping_{suffix}");
        locker
            .batch_execute(&format!(
                "CREATE TABLE {table} (id INTEGER PRIMARY KEY); \
                 INSERT INTO {table} (id) VALUES (1);"
            ))
            .await
            .expect("create lock-test row");
        locker
            .batch_execute("BEGIN")
            .await
            .expect("begin locker transaction");
        locker
            .query_one(
                &format!("SELECT id FROM {table} WHERE id = 1 FOR UPDATE"),
                &[],
            )
            .await
            .expect("locker holds row lock");

        let postgres_error = contender
            .query_one(
                &format!("SELECT id FROM {table} WHERE id = 1 FOR UPDATE NOWAIT"),
                &[],
            )
            .await
            .expect_err("NOWAIT contender must receive lock-not-available");
        assert_eq!(postgres_error.code(), Some(&SqlState::LOCK_NOT_AVAILABLE));

        let path = VirtualPath::new("/resources/deltas/log").expect("valid path");
        let mapped = db_error(path.clone(), FilesystemOperation::Append, postgres_error);
        assert!(matches!(
            mapped,
            FilesystemError::BackendBusy {
                path: error_path,
                operation: FilesystemOperation::Append,
            } if error_path == path
        ));

        locker
            .batch_execute(&format!("ROLLBACK; DROP TABLE {table};"))
            .await
            .expect("release lock and remove test table");
        drop(locker);
        drop(contender);
        locker_task.await.expect("locker connection task");
        contender_task.await.expect("contender connection task");
    }
}

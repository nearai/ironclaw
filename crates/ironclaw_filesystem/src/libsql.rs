use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::db::{
    child_path_like_pattern, direct_children, directory_append_error, directory_write_error,
    is_not_found, libsql_db_error, not_found, system_time_from_unix_seconds, valid_engine_path,
    virtual_path_prefixes,
};
use crate::{DirEntry, FileStat, FileType, FilesystemError, FilesystemOperation, RootFilesystem};

#[cfg(feature = "libsql")]
/// libSQL-backed [`RootFilesystem`] storing file contents by virtual path.
pub struct LibSqlRootFilesystem {
    db: Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlRootFilesystem {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        conn.execute_batch(LIBSQL_ROOT_FILESYSTEM_SCHEMA)
            .await
            .map_err(|error| {
                libsql_db_error(
                    valid_engine_path(),
                    FilesystemOperation::CreateDirAll,
                    error,
                )
            })?;
        ensure_libsql_root_is_dir_column(&conn).await?;
        Ok(())
    }

    async fn connect(&self) -> Result<libsql::Connection, FilesystemError> {
        let conn = self
            .db
            .connect()
            .map_err(|error| FilesystemError::Backend {
                path: valid_engine_path(),
                operation: FilesystemOperation::Stat,
                reason: error.to_string(),
            })?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|error| {
                libsql_db_error(valid_engine_path(), FilesystemOperation::Stat, error)
            })?;
        Ok(conn)
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl RootFilesystem for LibSqlRootFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT contents, is_dir FROM root_filesystem_entries WHERE path = ?1",
                libsql::params![path.as_str()],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?
        else {
            return Err(not_found(path.clone(), FilesystemOperation::ReadFile));
        };
        let is_dir: i64 = row
            .get(1)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        if is_dir != 0 {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "is a directory".to_string(),
            });
        }
        row.get(0)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        if matches!(
            self.exact_entry(path).await?,
            Some((_, FileType::Directory, _))
        ) || self.has_child_entry(path).await?
        {
            return Err(directory_write_error(path.clone()));
        }
        let conn = self.connect().await?;
        let rows = conn
            .execute(
                r#"
                INSERT INTO root_filesystem_entries (path, contents, is_dir, updated_at)
                VALUES (?1, ?2, 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                ON CONFLICT (path) DO UPDATE SET
                    contents = excluded.contents,
                    is_dir = 0,
                    updated_at = excluded.updated_at
                WHERE root_filesystem_entries.is_dir = 0
                "#,
                libsql::params![path.as_str(), libsql::Value::Blob(bytes.to_vec())],
            )
            .await
            .map_err(|error| {
                libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error)
            })?;
        if rows == 0 {
            return Err(directory_write_error(path.clone()));
        }
        Ok(())
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        if matches!(
            self.exact_entry(path).await?,
            Some((_, FileType::Directory, _))
        ) || self.has_child_entry(path).await?
        {
            return Err(directory_append_error(path.clone()));
        }
        let conn = self.connect().await?;
        // TODO(reborn): append rewrites the whole DB row. Do not use this path
        // for high-volume JSONL/event streams; route those through typed event
        // stores or append-capable artifact backends instead.
        conn.execute(
            r#"
            INSERT INTO root_filesystem_entries (path, contents, is_dir, updated_at)
            VALUES (?1, ?2, 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT (path) DO UPDATE SET
                contents = CAST(root_filesystem_entries.contents || excluded.contents AS BLOB),
                is_dir = 0,
                updated_at = excluded.updated_at
            "#,
            libsql::params![path.as_str(), libsql::Value::Blob(bytes.to_vec())],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::AppendFile, error))?;
        Ok(())
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let exact_entry = self.exact_entry(path).await?;
        if matches!(exact_entry, Some((_, FileType::File, _))) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
                reason: "not a directory".to_string(),
            });
        }
        let rows = self
            .child_entries(path, FilesystemOperation::ListDir)
            .await?;
        let children = direct_children(path, rows);
        if matches!(exact_entry, Some((_, FileType::Directory, _))) && is_not_found(&children) {
            return Ok(Vec::new());
        }
        children
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        if let Some((len, file_type, modified)) = self.exact_entry(path).await? {
            return Ok(FileStat {
                path: path.clone(),
                file_type,
                len,
                modified,
                sensitive: false,
            });
        }
        if self.has_child_entry(path).await? {
            return Ok(FileStat {
                path: path.clone(),
                file_type: FileType::Directory,
                len: 0,
                modified: None,
                sensitive: false,
            });
        }
        Err(not_found(path.clone(), FilesystemOperation::Stat))
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM root_filesystem_entries WHERE path = ?1 OR path LIKE ?2 ESCAPE '!'",
                libsql::params![path.as_str(), child_path_like_pattern(path)],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
        if deleted == 0 {
            return Err(not_found(path.clone(), FilesystemOperation::Delete));
        }
        Ok(())
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        let transaction = conn.transaction().await.map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
        })?;
        for prefix in virtual_path_prefixes(path)? {
            let mut rows = transaction
                .query(
                    "SELECT is_dir FROM root_filesystem_entries WHERE path = ?1",
                    libsql::params![prefix.as_str()],
                )
                .await
                .map_err(|error| {
                    libsql_db_error(prefix.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
            if let Some(row) = rows.next().await.map_err(|error| {
                libsql_db_error(prefix.clone(), FilesystemOperation::CreateDirAll, error)
            })? {
                let is_dir: i64 = row.get(0).map_err(|error| {
                    libsql_db_error(prefix.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
                if is_dir == 0 {
                    return Err(FilesystemError::Backend {
                        path: prefix,
                        operation: FilesystemOperation::CreateDirAll,
                        reason: "file exists where directory is required".to_string(),
                    });
                }
            }
            transaction
                .execute(
                    r#"
                    INSERT INTO root_filesystem_entries (path, contents, is_dir, updated_at)
                    VALUES (?1, X'', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                    ON CONFLICT (path) DO NOTHING
                    "#,
                    libsql::params![prefix.as_str()],
                )
                .await
                .map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
        }
        transaction.commit().await.map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
        })?;
        Ok(())
    }
}

#[cfg(feature = "libsql")]
async fn ensure_libsql_root_is_dir_column(
    conn: &libsql::Connection,
) -> Result<(), FilesystemError> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM pragma_table_info('root_filesystem_entries') WHERE name = 'is_dir'",
            (),
        )
        .await
        .map_err(|error| {
            libsql_db_error(
                valid_engine_path(),
                FilesystemOperation::CreateDirAll,
                error,
            )
        })?;
    if rows
        .next()
        .await
        .map_err(|error| {
            libsql_db_error(
                valid_engine_path(),
                FilesystemOperation::CreateDirAll,
                error,
            )
        })?
        .is_some()
    {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE root_filesystem_entries ADD COLUMN is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1))",
        (),
    )
    .await
    .map_err(|error| {
        libsql_db_error(
            valid_engine_path(),
            FilesystemOperation::CreateDirAll,
            error,
        )
    })?;
    Ok(())
}

#[cfg(feature = "libsql")]
impl LibSqlRootFilesystem {
    async fn exact_entry(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<(u64, FileType, Option<std::time::SystemTime>)>, FilesystemError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT length(contents), is_dir, CAST(strftime('%s', updated_at) AS INTEGER) AS updated_at_epoch FROM root_filesystem_entries WHERE path = ?1",
                libsql::params![path.as_str()],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let row = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let Some(row) = row else { return Ok(None) };
        let len_raw: i64 = row
            .get(0)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let is_dir_raw: i64 = row
            .get(1)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let updated_at_epoch: i64 = row
            .get(2)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let len = len_raw.max(0) as u64;
        let is_dir = is_dir_raw != 0;
        Ok(Some((
            if is_dir { 0 } else { len },
            if is_dir {
                FileType::Directory
            } else {
                FileType::File
            },
            system_time_from_unix_seconds(updated_at_epoch),
        )))
    }

    async fn child_entries(
        &self,
        parent: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<Vec<(VirtualPath, u64, FileType)>, FilesystemError> {
        let conn = self.connect().await?;
        let pattern = child_path_like_pattern(parent);
        let mut rows = conn
            .query(
                "SELECT path, length(contents), is_dir FROM root_filesystem_entries WHERE path LIKE ?1 ESCAPE '!' ORDER BY path",
                libsql::params![pattern],
            )
            .await
            .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
        let mut paths = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(parent.clone(), operation, error))?
        {
            let path: String = row
                .get(0)
                .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
            let len_raw: i64 = row
                .get(1)
                .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
            let is_dir_raw: i64 = row
                .get(2)
                .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
            let len = len_raw.max(0) as u64;
            let is_dir = is_dir_raw != 0;
            paths.push((
                VirtualPath::new(path)?,
                if is_dir { 0 } else { len },
                if is_dir {
                    FileType::Directory
                } else {
                    FileType::File
                },
            ));
        }
        Ok(paths)
    }

    async fn has_child_entry(&self, parent: &VirtualPath) -> Result<bool, FilesystemError> {
        let conn = self.connect().await?;
        let pattern = child_path_like_pattern(parent);
        let mut rows = conn
            .query(
                "SELECT 1 FROM root_filesystem_entries WHERE path LIKE ?1 ESCAPE '!' LIMIT 1",
                libsql::params![pattern],
            )
            .await
            .map_err(|error| libsql_db_error(parent.clone(), FilesystemOperation::Stat, error))?;
        Ok(rows
            .next()
            .await
            .map_err(|error| libsql_db_error(parent.clone(), FilesystemOperation::Stat, error))?
            .is_some())
    }
}

#[cfg(feature = "libsql")]
const LIBSQL_ROOT_FILESYSTEM_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS root_filesystem_entries (
    path TEXT PRIMARY KEY,
    contents BLOB NOT NULL DEFAULT X'',
    is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
-- The PRIMARY KEY on `path` already provides a unique index for equality
-- lookups, so no separate index is created.
"#;

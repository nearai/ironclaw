//! LogStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;

use super::{LibSqlBackend, fmt_ts, get_text, get_ts};
use crate::db::{LogEntryRecord, LogStore};
use crate::error::DatabaseError;

use chrono::Utc;

#[async_trait]
impl LogStore for LibSqlBackend {
    async fn insert_log_entry(
        &self,
        level: &str,
        target: &str,
        message: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "INSERT INTO log_entries (level, target, message, recorded_at) VALUES (?1, ?2, ?3, ?4)",
            params![level, target, message, now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_log_entries(&self, limit: i64) -> Result<Vec<LogEntryRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT level, target, message, recorded_at \
                 FROM log_entries ORDER BY recorded_at DESC LIMIT ?1",
                params![limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut entries = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            entries.push(LogEntryRecord {
                level: get_text(&row, 0),
                target: get_text(&row, 1),
                message: get_text(&row, 2),
                recorded_at: get_ts(&row, 3),
            });
        }
        entries.reverse();
        Ok(entries)
    }
}

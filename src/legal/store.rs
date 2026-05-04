//! Read-side legal-harness queries for the DOCX export endpoint.
//!
//! Stream C only needs to *read* a chat thread to render it. The full
//! CRUD surface (project/document/chat creation, message append) lives in
//! Streams A and B. Keeping the read store separate lets Stream C land
//! before the others without blocking on schema-trait re-shuffles.
//!
//! The store wraps a shared `Arc<libsql::Database>` and creates a fresh
//! connection per call, mirroring the pattern in
//! [`crate::secrets::store::LibSqlSecretsStore`].

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use libsql::{Database as LibSqlDatabase, params};

use crate::legal::{ChatExport, ChatMessage, ChatRole, LegalError};

/// Minimum subset of legal-harness queries needed by the DOCX export
/// handler. The wider read/write surface lives in Streams A/B.
pub struct LegalChatStore {
    db: Arc<LibSqlDatabase>,
}

impl LegalChatStore {
    /// Wrap a shared libsql handle.
    pub fn new(db: Arc<LibSqlDatabase>) -> Self {
        Self { db }
    }

    /// Load a chat with all its messages (oldest first) and the filenames
    /// of any documents each message referenced.
    ///
    /// Errors:
    /// - [`LegalError::ChatNotFound`] when the chat id is unknown.
    /// - [`LegalError::ChatEmpty`] when the chat exists but has no
    ///   messages — the export endpoint maps this to 400 because a
    ///   blank-document download is almost certainly a caller bug.
    /// - [`LegalError::Database`] for any libsql failure.
    pub async fn load_chat_for_export(&self, chat_id: &str) -> Result<ChatExport, LegalError> {
        let conn = self
            .db
            .connect()
            .map_err(|e| LegalError::Database(format!("connect failed: {e}")))?;
        // Match the rest of the codebase: every connection sets a busy
        // timeout so concurrent writers wait rather than failing instantly.
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|e| LegalError::Database(format!("set busy_timeout failed: {e}")))?;

        // 1) Header row.
        let mut rows = conn
            .query(
                "SELECT id, title, created_at FROM legal_chats WHERE id = ?1",
                params![chat_id],
            )
            .await
            .map_err(|e| LegalError::Database(format!("chat header query failed: {e}")))?;
        let header = rows
            .next()
            .await
            .map_err(|e| LegalError::Database(format!("chat header fetch failed: {e}")))?
            .ok_or_else(|| LegalError::ChatNotFound(chat_id.to_string()))?;

        let id: String = header
            .get(0)
            .map_err(|e| LegalError::Database(format!("chat id read failed: {e}")))?;
        let title: Option<String> = header
            .get(1)
            .map_err(|e| LegalError::Database(format!("chat title read failed: {e}")))?;
        let created_unix: i64 = header
            .get(2)
            .map_err(|e| LegalError::Database(format!("chat created_at read failed: {e}")))?;
        let created_at = unix_to_utc(created_unix);

        // 2) Messages, chronological order. The schema's CHECK
        //    constraint already filters role to the four legal values; we
        //    parse defensively in case a future migration loosens it.
        let mut msg_rows = conn
            .query(
                "SELECT id, role, content, document_refs, created_at \
                 FROM legal_chat_messages \
                 WHERE chat_id = ?1 \
                 ORDER BY created_at ASC, id ASC",
                params![chat_id],
            )
            .await
            .map_err(|e| LegalError::Database(format!("messages query failed: {e}")))?;

        let mut messages = Vec::new();
        while let Some(row) = msg_rows
            .next()
            .await
            .map_err(|e| LegalError::Database(format!("message row fetch failed: {e}")))?
        {
            let msg_id: String = row
                .get(0)
                .map_err(|e| LegalError::Database(format!("message id read failed: {e}")))?;
            let role_text: String = row
                .get(1)
                .map_err(|e| LegalError::Database(format!("message role read failed: {e}")))?;
            let content: String = row
                .get(2)
                .map_err(|e| LegalError::Database(format!("message content read failed: {e}")))?;
            let refs_raw: Option<String> = row.get(3).map_err(|e| {
                LegalError::Database(format!("message document_refs read failed: {e}"))
            })?;
            let created_unix: i64 = row.get(4).map_err(|e| {
                LegalError::Database(format!("message created_at read failed: {e}"))
            })?;

            let role = ChatRole::from_db(&role_text)?;
            let doc_ids = parse_document_refs(refs_raw.as_deref())?;
            let document_refs = if doc_ids.is_empty() {
                Vec::new()
            } else {
                resolve_document_filenames(&conn, &doc_ids).await?
            };

            messages.push(ChatMessage {
                id: msg_id,
                role,
                content,
                document_refs,
                created_at: unix_to_utc(created_unix),
            });
        }

        if messages.is_empty() {
            return Err(LegalError::ChatEmpty(chat_id.to_string()));
        }

        Ok(ChatExport {
            id,
            title,
            created_at,
            messages,
        })
    }
}

/// Convert a libSQL `INTEGER` unix-second timestamp into a `DateTime<Utc>`.
///
/// The canonical schema stores `created_at INTEGER NOT NULL DEFAULT
/// (unixepoch())`, so seconds-since-epoch is the contract. Out-of-range
/// values fall back to the unix epoch so the renderer always has *some*
/// timestamp to print rather than panicking on a corrupted row.
fn unix_to_utc(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap_or_default())
}

/// Parse the `document_refs` JSON column into a list of legal_document ids.
///
/// `None` and empty strings both yield an empty Vec — there's no legal
/// difference between "explicit empty array" and "no value" for this
/// field, so the renderer is identical either way.
fn parse_document_refs(raw: Option<&str>) -> Result<Vec<String>, LegalError> {
    let Some(text) = raw else {
        return Ok(Vec::new());
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str::<Vec<String>>(trimmed)
        .map_err(|e| LegalError::MalformedDocumentRefs(e.to_string()))
}

/// Look up filenames for a list of legal_document ids.
///
/// Order matches the input — i.e. mirrors the order the message author
/// referenced them. Missing ids are silently skipped; we don't want a
/// stale ref (e.g. a document deleted while a chat survived) to fail
/// the whole export. A debug log is recorded for each so an operator can
/// spot the divergence without having to read SQL.
async fn resolve_document_filenames(
    conn: &libsql::Connection,
    ids: &[String],
) -> Result<Vec<String>, LegalError> {
    let mut filenames = Vec::with_capacity(ids.len());
    for doc_id in ids {
        let mut rows = conn
            .query(
                "SELECT filename FROM legal_documents WHERE id = ?1",
                params![doc_id.as_str()],
            )
            .await
            .map_err(|e| LegalError::Database(format!("document filename query failed: {e}")))?;
        match rows
            .next()
            .await
            .map_err(|e| LegalError::Database(format!("document row fetch failed: {e}")))?
        {
            Some(row) => {
                let filename: String = row.get(0).map_err(|e| {
                    LegalError::Database(format!("document filename read failed: {e}"))
                })?;
                filenames.push(filename);
            }
            None => {
                tracing::debug!(
                    document_id = %doc_id,
                    "legal_chat_messages.document_refs references missing legal_documents row"
                );
            }
        }
    }
    Ok(filenames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_document_refs_handles_none_and_empty() {
        assert!(parse_document_refs(None).expect("none").is_empty());
        assert!(parse_document_refs(Some("")).expect("empty").is_empty());
        assert!(parse_document_refs(Some("   ")).expect("ws").is_empty());
    }

    #[test]
    fn parse_document_refs_parses_array() {
        let v = parse_document_refs(Some(r#"["a","b","c"]"#)).expect("ok");
        assert_eq!(v, vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_document_refs_rejects_garbage() {
        let err = parse_document_refs(Some("not json")).expect_err("should fail");
        assert!(matches!(err, LegalError::MalformedDocumentRefs(_)));
    }

    #[test]
    fn unix_to_utc_round_trips() {
        let dt = unix_to_utc(1_700_000_000);
        assert_eq!(dt.timestamp(), 1_700_000_000);
    }

    #[test]
    fn unix_to_utc_handles_overflow() {
        // i64::MAX is way past the supported chrono range; should
        // gracefully fall back rather than panic.
        let _ = unix_to_utc(i64::MAX);
    }
}

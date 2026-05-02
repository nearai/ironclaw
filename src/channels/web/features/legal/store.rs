//! Database access for the legal harness.
//!
//! libSQL-only today: the Postgres backend has the schema (migrations/V26)
//! but no Rust query layer wired in. Calling these helpers when the
//! database is the Postgres backend returns
//! [`crate::error::DatabaseError::Unsupported`] so handlers can map it
//! into a 501 response. Adding Postgres support is a follow-up tracked in
//! the PR body — the legal v1 deployment target is libSQL/Turso.
//!
//! No `unwrap` in the data-path code; row decode failures bubble up as
//! `DatabaseError::Query` so the gateway always returns a 500 instead of
//! crashing the worker.

use std::sync::Arc;

use crate::db::Database;
use crate::error::DatabaseError;

use super::models::{LegalDocument, LegalProject};

/// Convenience: borrow the libSQL backend from `Arc<dyn Database>` or
/// return `Unsupported`. Centralising the cast here keeps the not-supported
/// error message identical at every call site so route handlers can map
/// it onto a single 501 response.
fn libsql(db: &Arc<dyn Database>) -> Result<&crate::db::libsql::LibSqlBackend, DatabaseError> {
    crate::db::libsql_backend(db).ok_or_else(|| {
        DatabaseError::Unsupported(
            "Legal harness currently requires the libSQL backend".to_string(),
        )
    })
}

/// Insert a project. Caller supplies the ULID; the row's `created_at` is
/// populated by the column default.
pub async fn create_project(
    db: &Arc<dyn Database>,
    id: &str,
    name: &str,
    metadata: Option<&str>,
) -> Result<LegalProject, DatabaseError> {
    let backend = libsql(db)?;
    let conn = backend.connect().await?;

    conn.execute(
        "INSERT INTO legal_projects (id, name, metadata) VALUES (?1, ?2, ?3)",
        libsql::params![id, name, metadata.map(str::to_string)],
    )
    .await
    .map_err(|e| DatabaseError::Query(format!("create_project: {e}")))?;

    fetch_project_inner(&conn, id).await?.ok_or_else(|| {
        DatabaseError::Query("create_project: row missing immediately after insert".to_string())
    })
}

/// List active (not soft-deleted) projects ordered by `created_at` desc.
pub async fn list_active_projects(
    db: &Arc<dyn Database>,
) -> Result<Vec<LegalProject>, DatabaseError> {
    let backend = libsql(db)?;
    let conn = backend.connect().await?;

    let mut rows = conn
        .query(
            "SELECT id, name, deleted_at, created_at, metadata
               FROM legal_projects
              WHERE deleted_at IS NULL
              ORDER BY created_at DESC",
            libsql::params![],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("list_active_projects: {e}")))?;

    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(format!("list_active_projects iter: {e}")))?
    {
        out.push(row_to_project(&row)?);
    }
    Ok(out)
}

/// Fetch a single project by id (including soft-deleted ones — the gateway
/// decides whether to surface them).
pub async fn fetch_project(
    db: &Arc<dyn Database>,
    id: &str,
) -> Result<Option<LegalProject>, DatabaseError> {
    let backend = libsql(db)?;
    let conn = backend.connect().await?;
    fetch_project_inner(&conn, id).await
}

async fn fetch_project_inner(
    conn: &libsql::Connection,
    id: &str,
) -> Result<Option<LegalProject>, DatabaseError> {
    let mut rows = conn
        .query(
            "SELECT id, name, deleted_at, created_at, metadata
               FROM legal_projects
              WHERE id = ?1",
            libsql::params![id],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("fetch_project: {e}")))?;

    let row = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(format!("fetch_project iter: {e}")))?;
    match row {
        Some(r) => Ok(Some(row_to_project(&r)?)),
        None => Ok(None),
    }
}

/// Soft-delete a project. Returns `true` if a row was updated, `false` if
/// the project was already deleted or never existed (the handler maps that
/// into a 404).
pub async fn soft_delete_project(
    db: &Arc<dyn Database>,
    id: &str,
    now_unix: i64,
) -> Result<bool, DatabaseError> {
    let backend = libsql(db)?;
    let conn = backend.connect().await?;

    let affected = conn
        .execute(
            "UPDATE legal_projects
                SET deleted_at = ?1
              WHERE id = ?2
                AND deleted_at IS NULL",
            libsql::params![now_unix, id],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("soft_delete_project: {e}")))?;

    Ok(affected > 0)
}

/// Find a document in a project by sha256 (for upload dedupe).
pub async fn find_document_by_sha(
    db: &Arc<dyn Database>,
    project_id: &str,
    sha256: &str,
) -> Result<Option<LegalDocument>, DatabaseError> {
    let backend = libsql(db)?;
    let conn = backend.connect().await?;
    find_document_by_sha_inner(&conn, project_id, sha256).await
}

async fn find_document_by_sha_inner(
    conn: &libsql::Connection,
    project_id: &str,
    sha256: &str,
) -> Result<Option<LegalDocument>, DatabaseError> {
    let mut rows = conn
        .query(
            "SELECT id, project_id, filename, content_type, storage_path,
                    extracted_text, page_count, bytes, sha256, uploaded_at
               FROM legal_documents
              WHERE project_id = ?1 AND sha256 = ?2
              LIMIT 1",
            libsql::params![project_id, sha256],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("find_document_by_sha: {e}")))?;

    let row = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(format!("find_document_by_sha iter: {e}")))?;
    match row {
        Some(r) => Ok(Some(row_to_document(&r)?)),
        None => Ok(None),
    }
}

/// Outcome of `create_document`. Distinguishes a fresh insert from a
/// dedupe hit so the upload handler can avoid double-counting and the
/// caller knows which 200/200-existing semantics to surface.
#[derive(Debug)]
pub enum DocumentInsert {
    /// Fresh row — the caller's write succeeded.
    Inserted(LegalDocument),
    /// A row with the same `(project_id, sha256)` already exists — the
    /// returned row is the existing one (the caller's transient blob may
    /// be discarded; both copies are byte-identical by definition of sha
    /// equality).
    DuplicateExisting(LegalDocument),
}

/// Insert a new document row. Caller has already written the blob to disk
/// and computed the sha256/size/extraction.
///
/// Race semantics: the table has a UNIQUE(project_id, sha256) index. If a
/// concurrent upload of identical bytes wins the race, this returns
/// [`DocumentInsert::DuplicateExisting`] with the row that did land. The
/// caller must treat both arms as user-observable success.
#[allow(clippy::too_many_arguments)]
pub async fn create_document(
    db: &Arc<dyn Database>,
    id: &str,
    project_id: &str,
    filename: &str,
    content_type: &str,
    storage_path: &str,
    extracted_text: Option<&str>,
    page_count: Option<i64>,
    bytes: i64,
    sha256: &str,
) -> Result<DocumentInsert, DatabaseError> {
    let backend = libsql(db)?;
    let conn = backend.connect().await?;

    let insert_result = conn
        .execute(
            "INSERT INTO legal_documents
                (id, project_id, filename, content_type, storage_path,
                 extracted_text, page_count, bytes, sha256)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            libsql::params![
                id,
                project_id,
                filename,
                content_type,
                storage_path,
                extracted_text.map(str::to_string),
                page_count,
                bytes,
                sha256,
            ],
        )
        .await;

    match insert_result {
        Ok(_) => {
            let row = fetch_document_inner(&conn, id).await?.ok_or_else(|| {
                DatabaseError::Query(
                    "create_document: row missing immediately after insert".to_string(),
                )
            })?;
            Ok(DocumentInsert::Inserted(row))
        }
        Err(e) => {
            // The libsql crate flattens SQLITE_CONSTRAINT_UNIQUE into a
            // generic error; matching on the message is brittle, so on
            // any insert failure we look up the (project_id, sha256)
            // pair. If a row exists, the error was a dedupe race;
            // otherwise the caller sees the original failure.
            if let Some(existing) = find_document_by_sha_inner(&conn, project_id, sha256).await? {
                tracing::debug!(
                    project_id,
                    sha256,
                    "legal_documents insert lost dedupe race; returning existing row",
                );
                Ok(DocumentInsert::DuplicateExisting(existing))
            } else {
                Err(DatabaseError::Query(format!("create_document: {e}")))
            }
        }
    }
}

/// Fetch a single document by id. Does not filter on soft-deleted parents
/// — that's the handler's responsibility (it usually returns 404 anyway).
pub async fn fetch_document(
    db: &Arc<dyn Database>,
    id: &str,
) -> Result<Option<LegalDocument>, DatabaseError> {
    let backend = libsql(db)?;
    let conn = backend.connect().await?;
    fetch_document_inner(&conn, id).await
}

async fn fetch_document_inner(
    conn: &libsql::Connection,
    id: &str,
) -> Result<Option<LegalDocument>, DatabaseError> {
    let mut rows = conn
        .query(
            "SELECT id, project_id, filename, content_type, storage_path,
                    extracted_text, page_count, bytes, sha256, uploaded_at
               FROM legal_documents
              WHERE id = ?1",
            libsql::params![id],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("fetch_document: {e}")))?;

    let row = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(format!("fetch_document iter: {e}")))?;
    match row {
        Some(r) => Ok(Some(row_to_document(&r)?)),
        None => Ok(None),
    }
}

/// One hit from the per-project FTS search.
#[derive(Debug, Clone)]
pub struct LegalDocumentSearchHit {
    pub document_id: String,
    pub filename: String,
    /// FTS5 `snippet()` output: a short slice of `extracted_text` around
    /// the match with `<mark>...</mark>` highlights. Empty if the match
    /// hit only the filename.
    pub snippet: String,
    /// FTS5 `bm25()` rank. Lower = better match (FTS5 returns negatives
    /// to enable `ORDER BY rank`). Forwarded to the API consumer so it
    /// can surface relevance in the UI.
    pub rank: f64,
}

/// Default cap on hits per request. Keeps the response small enough
/// to render without pagination in v1.
pub const SEARCH_DEFAULT_LIMIT: i64 = 50;

/// Hard cap on hits per request to prevent a malformed `limit` from
/// pulling thousands of rows.
pub const SEARCH_MAX_LIMIT: i64 = 200;

/// Search a project's documents via FTS5. Empty queries (whitespace-only)
/// return no hits — callers should validate before reaching the store
/// but we double-check here so a stray empty `?q=` doesn't `bm25` the
/// whole table.
pub async fn search_project_documents(
    db: &Arc<dyn Database>,
    project_id: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<LegalDocumentSearchHit>, DatabaseError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, SEARCH_MAX_LIMIT);

    let backend = libsql(db)?;
    let conn = backend.connect().await?;

    // FTS5 wants the user query string in MATCH form. We pass the
    // user-supplied text verbatim so the FTS5 query parser does the
    // tokenization (it accepts plain words, AND/OR/NOT, NEAR, prefix
    // wildcards). Worst case the query parser rejects it and we surface
    // the error as a 400 in the handler.
    let mut rows = conn
        .query(
            "SELECT f.document_id, f.filename, \
                    snippet(legal_documents_fts, 3, '<mark>', '</mark>', '…', 32) AS snippet, \
                    bm25(legal_documents_fts) AS rank \
               FROM legal_documents_fts AS f \
              WHERE legal_documents_fts MATCH ?1 \
                AND f.project_id = ?2 \
              ORDER BY rank \
              LIMIT ?3",
            libsql::params![trimmed, project_id, limit],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("search_project_documents query: {e}")))?;

    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(format!("search_project_documents iter: {e}")))?
    {
        let document_id: String = row
            .get(0)
            .map_err(|e| DatabaseError::Query(format!("search hit document_id: {e}")))?;
        let filename: String = row
            .get(1)
            .map_err(|e| DatabaseError::Query(format!("search hit filename: {e}")))?;
        let snippet: String = row
            .get(2)
            .map_err(|e| DatabaseError::Query(format!("search hit snippet: {e}")))?;
        let rank: f64 = row
            .get(3)
            .map_err(|e| DatabaseError::Query(format!("search hit rank: {e}")))?;
        out.push(LegalDocumentSearchHit {
            document_id,
            filename,
            snippet,
            rank,
        });
    }
    Ok(out)
}

/// All documents in a project, newest first.
pub async fn list_documents_for_project(
    db: &Arc<dyn Database>,
    project_id: &str,
) -> Result<Vec<LegalDocument>, DatabaseError> {
    let backend = libsql(db)?;
    let conn = backend.connect().await?;

    let mut rows = conn
        .query(
            "SELECT id, project_id, filename, content_type, storage_path,
                    extracted_text, page_count, bytes, sha256, uploaded_at
               FROM legal_documents
              WHERE project_id = ?1
              ORDER BY uploaded_at DESC",
            libsql::params![project_id],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("list_documents_for_project: {e}")))?;

    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(format!("list_documents iter: {e}")))?
    {
        out.push(row_to_document(&row)?);
    }
    Ok(out)
}

// ---- Row decoders ------------------------------------------------------

fn row_to_project(row: &libsql::Row) -> Result<LegalProject, DatabaseError> {
    let map = |col: &str, e: libsql::Error| {
        DatabaseError::Query(format!("legal_projects column {col}: {e}"))
    };
    Ok(LegalProject {
        id: row.get::<String>(0).map_err(|e| map("id", e))?,
        name: row.get::<String>(1).map_err(|e| map("name", e))?,
        deleted_at: row
            .get::<Option<i64>>(2)
            .map_err(|e| map("deleted_at", e))?,
        created_at: row.get::<i64>(3).map_err(|e| map("created_at", e))?,
        metadata: row
            .get::<Option<String>>(4)
            .map_err(|e| map("metadata", e))?,
    })
}

fn row_to_document(row: &libsql::Row) -> Result<LegalDocument, DatabaseError> {
    let map = |col: &str, e: libsql::Error| {
        DatabaseError::Query(format!("legal_documents column {col}: {e}"))
    };
    Ok(LegalDocument {
        id: row.get::<String>(0).map_err(|e| map("id", e))?,
        project_id: row.get::<String>(1).map_err(|e| map("project_id", e))?,
        filename: row.get::<String>(2).map_err(|e| map("filename", e))?,
        content_type: row.get::<String>(3).map_err(|e| map("content_type", e))?,
        storage_path: row.get::<String>(4).map_err(|e| map("storage_path", e))?,
        extracted_text: row
            .get::<Option<String>>(5)
            .map_err(|e| map("extracted_text", e))?,
        page_count: row
            .get::<Option<i64>>(6)
            .map_err(|e| map("page_count", e))?,
        bytes: row.get::<i64>(7).map_err(|e| map("bytes", e))?,
        sha256: row.get::<String>(8).map_err(|e| map("sha256", e))?,
        uploaded_at: row.get::<i64>(9).map_err(|e| map("uploaded_at", e))?,
    })
}

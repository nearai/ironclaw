//! Axum handlers for the legal harness skill.
//!
//! These handlers own the project + document lifecycle. They do not yet
//! touch chats or chat messages — Streams B (chat-with-docs) and C (DOCX
//! export) layer those endpoints on top of the same migration.
//!
//! Auth: every handler is composed onto the `protected` router in
//! `src/channels/web/platform/router.rs` which already enforces the
//! gateway token via `auth_middleware`. The `AuthenticatedUser` extractor
//! is declared on each handler so accidental removal of the route
//! grouping fails the request rather than silently exposing the surface.

use std::sync::Arc;

use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::Response,
};
use serde::{Deserialize, Serialize};

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::platform::state::GatewayState;
use crate::error::DatabaseError;

use super::blobs;
use super::extract;
use super::models::{
    CreateProjectRequest, DocumentDetailResponse, DocumentResponse, LegalDocument, LegalProject,
    ProjectDetailResponse, ProjectListResponse,
};
use super::store;

/// Maximum size of an uploaded legal document, in bytes (10 MiB).
///
/// Smaller than the gateway's global 14 MiB body cap so the multipart
/// envelope (boundaries, JSON parts) has headroom.
const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;

/// Maximum length of a project name.
const MAX_PROJECT_NAME: usize = 200;

/// Maximum bytes of metadata JSON we'll persist on a project.
///
/// Schema doesn't enforce a cap; this prevents a 10 MB metadata payload
/// from filling the row.
const MAX_METADATA_BYTES: usize = 16 * 1024;

/// Content types accepted on upload. Restricting the set narrows the
/// download surface — `get_document_blob_handler` echoes the stored
/// content-type back, so anything with a HTML-like media type would let
/// an authenticated caller plant XSS that the next browser download
/// renders. PDF and OOXML are the only types the chat skill actually
/// reads anyway.
const ACCEPTED_CONTENT_TYPES: &[&str] = &[
    "application/pdf",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
];

/// Common error body returned on failures.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    error: String,
}

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorBody>) {
    (status, Json(ErrorBody { error: msg.into() }))
}

fn db_err(context: &str, e: DatabaseError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        DatabaseError::Unsupported(_) => {
            err(StatusCode::NOT_IMPLEMENTED, format!("{context}: {e}"))
        }
        DatabaseError::NotFound { .. } => err(StatusCode::NOT_FOUND, format!("{context}: {e}")),
        _ => err(StatusCode::INTERNAL_SERVER_ERROR, format!("{context}: {e}")),
    }
}

fn require_db(
    state: &Arc<GatewayState>,
) -> Result<&Arc<dyn crate::db::Database>, (StatusCode, Json<ErrorBody>)> {
    state.store.as_ref().ok_or_else(|| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database not available".to_string(),
        )
    })
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Fetch a document only if its parent project is still active.
/// Returns 404 with no information leak about whether the row exists
/// versus whether the project was deleted — both shapes look the same
/// to the caller.
async fn fetch_active_document(
    db: &Arc<dyn crate::db::Database>,
    id: &str,
) -> Result<LegalDocument, (StatusCode, Json<ErrorBody>)> {
    let doc = store::fetch_document(db, id)
        .await
        .map_err(|e| db_err("fetch_document", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("document {id} not found")))?;

    let project = store::fetch_project(db, &doc.project_id)
        .await
        .map_err(|e| db_err("fetch_project", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("document {id} not found")))?;

    if project.deleted_at.is_some() {
        // Indistinguishable from the row-missing case for the caller.
        return Err(err(
            StatusCode::NOT_FOUND,
            format!("document {id} not found"),
        ));
    }

    Ok(doc)
}

/// Decide which canonical mime to persist for a given upload.
///
/// We trust either:
/// 1. the client-declared content-type if it's exactly one of the two
///    accepted mimes, OR
/// 2. the filename extension (`.pdf` / `.docx`) when the declared mime is
///    missing or generic.
///
/// Any other declared mime fails the upload rather than getting persisted
/// and echoed back to a future download.
fn normalise_content_type(declared: Option<&str>, filename: &str) -> Option<&'static str> {
    let docx_mime = ACCEPTED_CONTENT_TYPES[1];
    let declared_lc = declared.map(|s| s.trim().to_ascii_lowercase());
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());

    if let Some(ref ct) = declared_lc {
        if ct == "application/pdf" {
            return Some(ACCEPTED_CONTENT_TYPES[0]);
        }
        if ct == docx_mime {
            return Some(ACCEPTED_CONTENT_TYPES[1]);
        }
    }
    if let Some(ref e) = ext {
        if e == "pdf" {
            return Some(ACCEPTED_CONTENT_TYPES[0]);
        }
        if e == "docx" {
            return Some(ACCEPTED_CONTENT_TYPES[1]);
        }
    }
    None
}

/// `POST /api/skills/legal/projects` — create a new project.
pub async fn create_project_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Json(req): Json<CreateProjectRequest>,
) -> Result<Json<LegalProject>, (StatusCode, Json<ErrorBody>)> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "project name is required"));
    }
    if name.len() > MAX_PROJECT_NAME {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("project name exceeds {MAX_PROJECT_NAME} chars"),
        ));
    }

    let metadata = match req.metadata {
        None => None,
        Some(serde_json::Value::Null) => None,
        Some(v) => {
            let s = serde_json::to_string(&v).map_err(|e| {
                err(
                    StatusCode::BAD_REQUEST,
                    format!("invalid metadata json: {e}"),
                )
            })?;
            if s.len() > MAX_METADATA_BYTES {
                return Err(err(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!("metadata exceeds {MAX_METADATA_BYTES} bytes"),
                ));
            }
            Some(s)
        }
    };

    let db = require_db(&state)?;
    let id = ulid::Ulid::new().to_string();
    let project = store::create_project(db, &id, name, metadata.as_deref())
        .await
        .map_err(|e| db_err("create_project", e))?;

    Ok(Json(project))
}

/// `GET /api/skills/legal/projects` — list active projects.
pub async fn list_projects_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<Json<ProjectListResponse>, (StatusCode, Json<ErrorBody>)> {
    let db = require_db(&state)?;
    let projects = store::list_active_projects(db)
        .await
        .map_err(|e| db_err("list_projects", e))?;
    let count = projects.len();
    Ok(Json(ProjectListResponse { projects, count }))
}

/// `GET /api/skills/legal/projects/:id` — project + its documents.
pub async fn get_project_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<ProjectDetailResponse>, (StatusCode, Json<ErrorBody>)> {
    let db = require_db(&state)?;
    let project = store::fetch_project(db, &id)
        .await
        .map_err(|e| db_err("fetch_project", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("project {id} not found")))?;
    if project.deleted_at.is_some() {
        return Err(err(
            StatusCode::NOT_FOUND,
            format!("project {id} has been deleted"),
        ));
    }
    let documents = store::list_documents_for_project(db, &id)
        .await
        .map_err(|e| db_err("list_documents", e))?;
    Ok(Json(ProjectDetailResponse { project, documents }))
}

#[derive(Debug, Deserialize)]
pub struct DeleteProjectQuery {
    /// `?hard=true` switches the endpoint from soft-delete (set
    /// `deleted_at`) to hard-delete (drop the row + its cascade + free
    /// any orphaned blobs). The default is soft-delete to match the
    /// pre-existing behaviour. Recognised values: `true`, `1`, `yes`
    /// (case-insensitive). Anything else is treated as `false`.
    #[serde(default)]
    pub hard: Option<String>,
}

fn parse_hard_flag(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("true") | Some("1") | Some("yes") | Some("on")
    )
}

/// `DELETE /api/skills/legal/projects/:id[?hard=true]` — soft delete by
/// default; `?hard=true` drops the row, cascades to documents/chats/
/// messages, and removes any blobs that are no longer referenced.
pub async fn delete_project_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
    Query(params): Query<DeleteProjectQuery>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let db = require_db(&state)?;
    let hard = parse_hard_flag(params.hard.as_deref());

    if !hard {
        let updated = store::soft_delete_project(db, &id, now_unix())
            .await
            .map_err(|e| db_err("soft_delete_project", e))?;
        if !updated {
            return Err(err(
                StatusCode::NOT_FOUND,
                format!("project {id} not found or already deleted"),
            ));
        }
        return Ok(StatusCode::NO_CONTENT);
    }

    // Hard delete: drop the row, cascade-delete documents/chats/messages
    // via the FK, then walk the sha256s the project's documents owned
    // and remove blobs that have no other referrers.
    let shas = store::hard_delete_project(db, &id)
        .await
        .map_err(|e| db_err("hard_delete_project", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("project {id} not found")))?;

    let data_dir = crate::bootstrap::ironclaw_base_dir();
    free_unreferenced_blobs(db, &data_dir, &shas).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/skills/legal/documents/:id` — hard-delete a document.
/// Frees the underlying blob if no other row references the same sha.
pub async fn delete_document_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let db = require_db(&state)?;

    let removed = store::delete_document(db, &id)
        .await
        .map_err(|e| db_err("delete_document", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("document {id} not found")))?;
    let (_storage_path, sha256) = removed;

    let data_dir = crate::bootstrap::ironclaw_base_dir();
    free_unreferenced_blobs(db, &data_dir, std::slice::from_ref(&sha256)).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Walk the supplied list of `sha256` values and remove the blob for
/// each one that has no remaining `legal_documents` referrer. Errors
/// from either the count-query or the filesystem are logged but never
/// surfaced to the caller — the row is already gone, so a stale blob is
/// at worst orphaned space, not a correctness issue.
async fn free_unreferenced_blobs(
    db: &Arc<dyn crate::db::Database>,
    data_dir: &std::path::Path,
    shas: &[String],
) {
    use std::collections::HashSet;

    let mut seen: HashSet<&str> = HashSet::new();
    for sha in shas {
        if !seen.insert(sha.as_str()) {
            continue;
        }
        match store::count_documents_with_sha(db, sha).await {
            Ok(0) => match blobs::delete_blob(data_dir, sha).await {
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    sha = %sha,
                    error = %e,
                    "legal: blob cleanup failed; row already deleted"
                ),
            },
            Ok(_) => {}
            Err(e) => tracing::warn!(
                sha = %sha,
                error = %e,
                "legal: count_documents_with_sha failed; skipping blob cleanup"
            ),
        }
    }
}

/// `POST /api/skills/legal/projects/:id/documents` — multipart upload,
/// extract text, persist blob + row.
///
/// The multipart envelope expects exactly one field named `file`; any
/// other field is ignored, so front-ends can layer extra metadata
/// fields without breaking this parser.
pub async fn upload_document_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(project_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<DocumentResponse>, (StatusCode, Json<ErrorBody>)> {
    let db = require_db(&state)?;

    // Verify project exists and isn't soft-deleted before reading the body
    // so a misaddressed upload fails fast.
    let project = store::fetch_project(db, &project_id)
        .await
        .map_err(|e| db_err("fetch_project", e))?
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                format!("project {project_id} not found"),
            )
        })?;
    if project.deleted_at.is_some() {
        return Err(err(
            StatusCode::CONFLICT,
            format!("project {project_id} has been deleted"),
        ));
    }

    let mut filename: Option<String> = None;
    let mut content_type: Option<String> = None;
    let mut bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("multipart read: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name != "file" {
            // Ignore extra metadata fields to keep the parser permissive.
            continue;
        }
        filename = field.file_name().map(str::to_string);
        content_type = field.content_type().map(str::to_string);
        let data = field
            .bytes()
            .await
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("read multipart body: {e}")))?
            .to_vec();
        if data.len() > MAX_UPLOAD_BYTES {
            return Err(err(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("upload exceeds {MAX_UPLOAD_BYTES} bytes"),
            ));
        }
        bytes = Some(data);
        break;
    }

    let bytes = bytes.ok_or_else(|| err(StatusCode::BAD_REQUEST, "missing file field"))?;
    let filename = filename
        .filter(|s| !s.is_empty())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "missing filename"))?;
    // Defensive: strip path components a misbehaving client might send.
    // The blob path is sha-derived so this is mostly cosmetic, but it stops
    // a `..` from leaking into the stored filename.
    let safe_filename = std::path::Path::new(&filename)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&filename)
        .to_string();
    // Normalise the stored content-type. We accept either the canonical
    // PDF/OOXML mime or a `.pdf`/`.docx` extension, but persist only the
    // canonical mime so downloads can never echo back, e.g.,
    // `text/html` from a misbehaving uploader.
    let normalised_ct = normalise_content_type(content_type.as_deref(), &safe_filename)
        .ok_or_else(|| {
            err(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Only application/pdf and OOXML (.docx) uploads are supported".to_string(),
            )
        })?;
    let content_type = normalised_ct.to_string();

    let sha = blobs::sha256_hex(&bytes);

    // Project-scoped dedupe: if this sha already exists for this project,
    // return the existing row instead of writing a duplicate.
    if let Some(existing) = store::find_document_by_sha(db, &project_id, &sha)
        .await
        .map_err(|e| db_err("dedupe lookup", e))?
    {
        return Ok(Json(existing));
    }

    // Inline extraction. If extraction fails we still want the upload to
    // succeed: text-less documents are useful for download and the chat
    // skill can degrade gracefully.
    let extracted = extract::extract(&content_type, &safe_filename, &bytes)
        .await
        .ok();
    let (text, page_count) = match extracted {
        Some(e) => (Some(e.text), e.page_count),
        None => (None, None),
    };

    let data_dir = crate::bootstrap::ironclaw_base_dir();
    let storage_rel = blobs::write_blob(&data_dir, &sha, &bytes)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("blob write: {e}"),
            )
        })?;
    let storage_path = storage_rel.to_string_lossy().to_string();

    let id = ulid::Ulid::new().to_string();
    let outcome = store::create_document(
        db,
        &id,
        &project_id,
        &safe_filename,
        &content_type,
        &storage_path,
        text.as_deref(),
        page_count,
        bytes.len() as i64,
        &sha,
    )
    .await
    .map_err(|e| db_err("create_document", e))?;

    let doc = match outcome {
        store::DocumentInsert::Inserted(d) => d,
        // Dedupe race winner: another concurrent upload of identical
        // bytes already landed. Return the existing row; the orphaned
        // sha-named blob is identical bytes so leaving it on disk is
        // harmless and content-addressed dedupe absorbs the cost.
        store::DocumentInsert::DuplicateExisting(d) => d,
    };

    Ok(Json(doc))
}

/// `GET /api/skills/legal/documents/:id` — metadata + extracted text.
///
/// Documents whose parent project is soft-deleted are treated as
/// missing (404) so the foundation upholds the same active/deleted
/// boundary that `get_project_handler` does. Without this check a
/// caller who knew a document id could still read its text through
/// `/documents/:id` after the project was deleted.
pub async fn get_document_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<DocumentDetailResponse>, (StatusCode, Json<ErrorBody>)> {
    let db = require_db(&state)?;
    let doc = fetch_active_document(db, &id).await?;
    Ok(Json(doc))
}

/// `GET /api/skills/legal/documents/:id/blob` — raw file bytes.
///
/// Soft-delete enforcement: same as `get_document_handler`, this
/// endpoint refuses to serve bytes for a document whose parent
/// project has `deleted_at IS NOT NULL`.
///
/// Sanity bounds:
/// - `Content-Type` is always one of the canonical accepted mimes
///   (verified at upload time via [`normalise_content_type`]).
/// - `Content-Disposition` is sanitised to ASCII filename only; any
///   non-ASCII character or newline is replaced with `_` to avoid
///   header injection. RFC 5987 `filename*` is intentionally not used
///   so a Unicode filename downgrades to a safe ASCII fallback rather
///   than failing the response.
pub async fn get_document_blob_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, (StatusCode, Json<ErrorBody>)> {
    let db = require_db(&state)?;
    let doc: LegalDocument = fetch_active_document(db, &id).await?;

    let data_dir = crate::bootstrap::ironclaw_base_dir();
    let bytes = blobs::read_blob(&data_dir, &doc.sha256)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("blob read: {e}")))?;

    let safe_filename = sanitise_filename_for_disposition(&doc.filename);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, doc.content_type.clone())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{safe_filename}\""),
        )
        // Belt-and-braces: the gateway already adds X-Content-Type-Options
        // globally; setting it on this response too means clones that
        // carry the body through a downstream proxy still see it.
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(bytes))
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("response build: {e}"),
            )
        })
}

/// Sanitise a filename for use inside a `Content-Disposition: attachment;
/// filename="..."` header. Replaces every non-printable-ASCII character,
/// every `"`, and every CR/LF with `_`. Never returns an empty string.
fn sanitise_filename_for_disposition(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '"' | '\\' | '\r' | '\n' => '_',
            c if c.is_ascii_graphic() || c == ' ' => c,
            _ => '_',
        })
        .collect();
    if out.is_empty() {
        out.push_str("download");
    }
    out
}

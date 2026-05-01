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
    extract::{Multipart, Path, State},
    http::{StatusCode, header},
    response::Response,
};
use serde::Serialize;

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

/// Common error body returned on failures.
#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorBody>) {
    (status, Json(ErrorBody { error: msg.into() }))
}

fn db_err(context: &str, e: DatabaseError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        DatabaseError::Unsupported(_) => err(
            StatusCode::NOT_IMPLEMENTED,
            format!("{context}: {e}"),
        ),
        DatabaseError::NotFound { .. } => err(StatusCode::NOT_FOUND, format!("{context}: {e}")),
        _ => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{context}: {e}"),
        ),
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

/// `DELETE /api/skills/legal/projects/:id` — soft delete.
pub async fn delete_project_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let db = require_db(&state)?;
    let updated = store::soft_delete_project(db, &id, now_unix())
        .await
        .map_err(|e| db_err("soft_delete_project", e))?;
    if !updated {
        return Err(err(
            StatusCode::NOT_FOUND,
            format!("project {id} not found or already deleted"),
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/skills/legal/projects/:id/documents` — multipart upload,
/// extract text, persist blob + row.
///
/// The multipart envelope expects exactly one field named `file` (matching
/// mike's wire shape — the field *name* is not copyrightable code). All
/// other field names are ignored to allow front-ends to layer extra
/// metadata fields without breaking the parser.
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
    let content_type = content_type.unwrap_or_else(|| "application/octet-stream".to_string());

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
    let extracted = extract::extract(&content_type, &safe_filename, &bytes).await.ok();
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
    let doc = store::create_document(
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

    Ok(Json(doc))
}

/// `GET /api/skills/legal/documents/:id` — metadata + extracted text.
pub async fn get_document_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<DocumentDetailResponse>, (StatusCode, Json<ErrorBody>)> {
    let db = require_db(&state)?;
    let doc = store::fetch_document(db, &id)
        .await
        .map_err(|e| db_err("fetch_document", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("document {id} not found")))?;
    Ok(Json(doc))
}

/// `GET /api/skills/legal/documents/:id/blob` — raw file bytes.
pub async fn get_document_blob_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, (StatusCode, Json<ErrorBody>)> {
    let db = require_db(&state)?;
    let doc: LegalDocument = store::fetch_document(db, &id)
        .await
        .map_err(|e| db_err("fetch_document", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("document {id} not found")))?;

    let data_dir = crate::bootstrap::ironclaw_base_dir();
    let bytes = blobs::read_blob(&data_dir, &doc.sha256).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("blob read: {e}"),
        )
    })?;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, doc.content_type.clone())
        .header(
            header::CONTENT_DISPOSITION,
            format!(
                "attachment; filename=\"{}\"",
                doc.filename.replace('"', "_")
            ),
        )
        .body(Body::from(bytes))
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("response build: {e}"),
            )
        })
}

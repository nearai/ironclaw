//! Wire types for the legal harness skill.
//!
//! These map 1:1 onto the rows defined in `migrations/V26__legal_harness.sql`
//! (and the matching libSQL `INCREMENTAL_MIGRATIONS` entry). Changes here
//! must stay in lockstep with the schema and with Streams B (chat) and
//! C (DOCX export); see `legal-harness-spec.md`.

use serde::{Deserialize, Serialize};

/// Stored project record.
///
/// `metadata` is a free-form JSON blob the caller controls. We round-trip it
/// as a string at the storage layer so callers can layer their own schema on
/// top without forcing a new migration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegalProject {
    pub id: String,
    pub name: String,
    /// Soft-delete timestamp (unix seconds). `None` means active.
    pub deleted_at: Option<i64>,
    pub created_at: i64,
    /// Optional caller-controlled metadata (raw JSON string, not parsed by the
    /// store). When `None`, the database column is `NULL`.
    pub metadata: Option<String>,
}

/// Stored document record.
///
/// `extracted_text` is `None` until extraction completes (extraction is
/// inline today, but the schema allows for an async pipeline later).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegalDocument {
    pub id: String,
    pub project_id: String,
    pub filename: String,
    pub content_type: String,
    /// Path relative to the ironclaw data dir (e.g. `legal/blobs/ab/abc..d`).
    pub storage_path: String,
    pub extracted_text: Option<String>,
    pub page_count: Option<i64>,
    pub bytes: i64,
    pub sha256: String,
    pub uploaded_at: i64,
}

/// Create-project request body.
#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    /// Optional caller metadata. Stored as a JSON string; the value here is
    /// re-serialized so callers can pass either an object or a pre-encoded
    /// string.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// List response for projects (active only by default).
#[derive(Debug, Serialize)]
pub struct ProjectListResponse {
    pub projects: Vec<LegalProject>,
    pub count: usize,
}

/// Project detail (with documents) response.
#[derive(Debug, Serialize)]
pub struct ProjectDetailResponse {
    #[serde(flatten)]
    pub project: LegalProject,
    pub documents: Vec<LegalDocument>,
}

/// Document upload response — same shape as a stored row.
pub type DocumentResponse = LegalDocument;

/// Document detail response (alias kept for endpoint clarity).
pub type DocumentDetailResponse = LegalDocument;

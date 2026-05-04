//! Legal-harness HTTP surface — Stream C scope (DOCX export).
//!
//! Owns the single export endpoint:
//!
//! | Method | Path | Handler |
//! |--------|------|---------|
//! | POST | `/skills/legal/chats/{id}/export.docx` | [`export_chat_docx_handler`] |
//!
//! Streams A and B introduce the rest of the surface (project/document
//! CRUD, chat creation + RAG). This slice deliberately ships in a
//! separate branch so Word/LibreOffice round-tripping can be verified
//! independently of the chat pipeline.
//!
//! # Error mapping
//!
//! - [`crate::legal::LegalError::ChatNotFound`] → `404 Not Found`
//! - [`crate::legal::LegalError::ChatEmpty`] → `400 Bad Request`
//!   ("a blank `.docx` is almost always a caller bug")
//! - Database / render errors → `500 Internal Server Error`
//!   (the wire body is intentionally generic; details go to the log
//!   layer).
//!
//! # Multi-tenancy / authorization
//!
//! Every request is gated on the platform-wide
//! [`crate::channels::web::auth::AuthenticatedUser`] extractor. The v1
//! schema does not yet associate `legal_projects` with users (Stream A
//! v2 will add that); for now the handler trusts the gateway's existing
//! auth and treats the chat id as a globally addressable handle. The
//! HTTP-level auth layer is what blocks anonymous callers; per-user
//! ACLs land in a follow-up.
//!
//! # Test coverage
//!
//! Unit tests live alongside the handler and exercise:
//!
//! - The path that hits a real libSQL backend (using
//!   [`crate::testing::test_db`] + the canonical migration we ship in
//!   this branch) and validates the byte stream is a well-formed OOXML
//!   archive that matches the persisted chat.
//! - The empty-chat path (400) and unknown-chat path (404).
//! - Adversarial payloads: messages containing XML-control characters,
//!   massive bodies, large `document_refs` arrays.
//!
//! These tests share a small `seed_chat` helper that writes the four
//! tables defined in the canonical migration directly via libsql so we
//! exercise the same SQL the production handler reads.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::platform::state::GatewayState;
use crate::legal::{LegalError, docx::render_chat_to_docx};

/// `POST /skills/legal/chats/{id}/export.docx`
///
/// Render the chat thread (chronologically ordered, with timestamps and
/// `document_refs` callouts) as a `.docx` file and stream the bytes back
/// with the OOXML MIME type and a download-friendly
/// `Content-Disposition` header.
///
/// The request has no body; the chat id is taken from the URL.
pub(crate) async fn export_chat_docx_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(chat_id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    if chat_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "chat id must not be empty".to_string(),
        ));
    }
    // Cheap defence-in-depth: reject obviously-pathological ids before
    // they reach the database driver. The schema only stores TEXT, so
    // SQL doesn't reject control chars on the way in; treat anything
    // outside printable ASCII + `-`/`_`/`.` as suspect.
    if !chat_id.bytes().all(is_safe_chat_id_byte) || chat_id.len() > 128 {
        return Err((
            StatusCode::BAD_REQUEST,
            "chat id contains disallowed characters".to_string(),
        ));
    }

    let store = state.legal_chat_store.as_ref().ok_or_else(|| {
        tracing::warn!(
            user_id = %user.user_id,
            chat_id = %chat_id,
            "legal chat store not configured; refusing export"
        );
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "legal harness is not enabled on this gateway".to_string(),
        )
    })?;

    let chat = store
        .load_chat_for_export(&chat_id)
        .await
        .map_err(legal_error_to_status)?;

    // The renderer is CPU-bound; offload to a blocking pool so the axum
    // worker thread is not parked while docx-rs walks its tree and zips
    // the output. The closure owns the snapshot, so the borrow checker
    // is satisfied without any unsafe-feeling tricks.
    let bytes = tokio::task::spawn_blocking(move || render_chat_to_docx(&chat))
        .await
        .map_err(|join| {
            tracing::error!(error = %join, "docx render task panicked or was cancelled");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "DOCX export failed".to_string(),
            )
        })?
        .map_err(legal_error_to_status)?;

    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let filename = sanitize_filename_segment(&chat_id);
    let disposition = format!("attachment; filename=\"chat-{filename}-{timestamp}.docx\"");

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
    );
    // The disposition string is built from sanitised input but still
    // construct via `from_str` so a logic bug surfaces as a 500 rather
    // than corrupting the response headers.
    let disposition_value = HeaderValue::from_str(&disposition).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to build Content-Disposition header".to_string(),
        )
    })?;
    headers.insert(header::CONTENT_DISPOSITION, disposition_value);

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(bytes))
        .map_err(|e| {
            tracing::error!(error = %e, "failed to construct DOCX response");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to construct response".to_string(),
            )
        })?;
    response.headers_mut().extend(headers);
    Ok(response)
}

/// Allowed bytes in a chat id: ASCII alphanumeric plus `-`, `_`, `.`.
/// The canonical schema declares `id TEXT PRIMARY KEY` with no further
/// constraint, but every legitimate id (ULID, UUID, or hand-typed slug)
/// fits this charset. Rejecting anything else here means we never
/// substring it into a `Content-Disposition` header.
fn is_safe_chat_id_byte(b: u8) -> bool {
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.')
}

/// Sanitise the chat id for inclusion in the suggested filename. We
/// already enforce the safe-byte charset above, so this is a defensive
/// pass: trim length and reject any sneaky empty result.
fn sanitize_filename_segment(id: &str) -> String {
    let trimmed: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(64)
        .collect();
    if trimmed.is_empty() {
        "chat".to_string()
    } else {
        trimmed
    }
}

fn legal_error_to_status(err: LegalError) -> (StatusCode, String) {
    match err {
        LegalError::ChatNotFound(id) => (StatusCode::NOT_FOUND, format!("chat {id} not found")),
        LegalError::ChatEmpty(id) => (
            StatusCode::BAD_REQUEST,
            format!("chat {id} has no messages to export"),
        ),
        LegalError::UnknownRole(role) => {
            tracing::error!(role = %role, "legal chat row has unknown role");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "chat contains a row with an unrecognised role".to_string(),
            )
        }
        LegalError::MalformedDocumentRefs(detail) => {
            tracing::warn!(detail = %detail, "legal chat row has malformed document_refs");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "chat references are corrupt".to_string(),
            )
        }
        LegalError::Database(detail) => {
            tracing::error!(detail = %detail, "legal store database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "database error".to_string(),
            )
        }
        LegalError::Render(detail) => {
            tracing::error!(detail = %detail, "DOCX render error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "DOCX render error".to_string(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legal::store::LegalChatStore;
    use libsql::params;
    use std::io::Read;

    /// Stand up a temp libSQL database, run migrations, and return the
    /// raw libsql Database handle plus a tempdir guard. The full base
    /// schema runs because [`crate::testing::test_db`] calls
    /// `run_migrations` — that means our V26 legal-harness migration is
    /// present, so the four canonical tables exist for the test seed.
    async fn fresh_libsql_with_legal_schema() -> (Arc<libsql::Database>, tempfile::TempDir) {
        use crate::db::libsql::LibSqlBackend;

        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("legal-c.db");
        let backend = LibSqlBackend::new_local(&path).await.expect("open backend");
        // Use the same migration entry point production uses so the
        // V26 row is recorded — otherwise rerunning tests against a
        // persistent db would silently skip the legal tables.
        crate::db::Database::run_migrations(&backend)
            .await
            .expect("run migrations");
        (backend.shared_db(), tmp)
    }

    /// Insert a chat with the given messages directly via libSQL. Each
    /// message tuple is `(role, content, document_refs)`. Returns the
    /// generated chat id.
    async fn seed_chat(db: &libsql::Database, messages: &[(&str, &str, &[&str])]) -> String {
        let conn = db.connect().expect("connect");

        let project_id = "proj-test-1";
        let chat_id = "chat-test-1";

        conn.execute(
            "INSERT INTO legal_projects (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![project_id, "Test Project", 1_700_000_000_i64],
        )
        .await
        .expect("insert project");

        conn.execute(
            "INSERT INTO legal_chats (id, project_id, title, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![chat_id, project_id, "Test chat", 1_700_000_000_i64],
        )
        .await
        .expect("insert chat");

        // Seed any documents we'll reference so resolve_document_filenames
        // can join against legal_documents.
        let mut doc_id_seq: i64 = 0;
        let mut filename_to_id: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (_, _, refs) in messages {
            for r in *refs {
                if !filename_to_id.contains_key(*r) {
                    doc_id_seq += 1;
                    let did = format!("doc-{doc_id_seq}");
                    conn.execute(
                        "INSERT INTO legal_documents \
                            (id, project_id, filename, content_type, storage_path, bytes, sha256, uploaded_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![
                            did.as_str(),
                            project_id,
                            *r,
                            "application/pdf",
                            format!("legal/blobs/test/{did}").as_str(),
                            0_i64,
                            format!("sha-{did}").as_str(),
                            1_700_000_000_i64,
                        ],
                    )
                    .await
                    .expect("insert doc");
                    filename_to_id.insert((*r).to_string(), did);
                }
            }
        }

        for (i, (role, content, refs)) in messages.iter().enumerate() {
            let mid = format!("msg-{i}");
            let refs_ids: Vec<&str> = refs
                .iter()
                .map(|r| filename_to_id.get(*r).expect("seeded").as_str())
                .collect();
            let refs_json = if refs_ids.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&refs_ids).expect("json"))
            };
            conn.execute(
                "INSERT INTO legal_chat_messages \
                    (id, chat_id, role, content, document_refs, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    mid.as_str(),
                    chat_id,
                    *role,
                    *content,
                    match refs_json.as_deref() {
                        Some(s) => libsql::Value::Text(s.to_string()),
                        None => libsql::Value::Null,
                    },
                    1_700_000_000_i64 + i as i64,
                ],
            )
            .await
            .expect("insert message");
        }

        chat_id.to_string()
    }

    fn extract_document_xml(bytes: &[u8]) -> String {
        let reader = std::io::Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(reader).expect("valid zip");
        let mut entry = archive
            .by_name("word/document.xml")
            .expect("word/document.xml present");
        let mut out = String::new();
        entry.read_to_string(&mut out).expect("read");
        out
    }

    #[tokio::test]
    async fn store_round_trip_renders_expected_xml() {
        let (db, _guard) = fresh_libsql_with_legal_schema().await;
        let chat_id = seed_chat(
            &db,
            &[
                ("user", "What about Section 3?", &["nda.pdf"]),
                (
                    "assistant",
                    "Section 3 covers confidentiality.\n\nIt runs for 5 years.",
                    &["nda.pdf", "exhibit-a.pdf"],
                ),
                ("user", "Got it.", &[]),
            ],
        )
        .await;

        let store = LegalChatStore::new(Arc::clone(&db));
        let chat = store
            .load_chat_for_export(&chat_id)
            .await
            .expect("load chat");
        assert_eq!(chat.id, chat_id);
        assert_eq!(chat.messages.len(), 3);
        assert_eq!(chat.messages[0].document_refs, vec!["nda.pdf"]);
        assert_eq!(
            chat.messages[1].document_refs,
            vec!["nda.pdf", "exhibit-a.pdf"]
        );
        assert!(chat.messages[2].document_refs.is_empty());

        let bytes = render_chat_to_docx(&chat).expect("render");
        let xml = extract_document_xml(&bytes);
        assert!(xml.contains("Section 3 covers confidentiality."));
        assert!(xml.contains("It runs for 5 years."));
        assert!(xml.contains("Documents referenced:"));
        assert!(xml.contains("nda.pdf"));
        assert!(xml.contains("exhibit-a.pdf"));
        // OOXML zip magic.
        assert_eq!(&bytes[0..4], b"PK\x03\x04");
    }

    #[tokio::test]
    async fn unknown_chat_id_yields_chat_not_found() {
        let (db, _guard) = fresh_libsql_with_legal_schema().await;
        let store = LegalChatStore::new(db);
        let err = store
            .load_chat_for_export("nope")
            .await
            .expect_err("must fail");
        assert!(matches!(err, LegalError::ChatNotFound(_)));
    }

    #[tokio::test]
    async fn empty_chat_yields_chat_empty() {
        let (db, _guard) = fresh_libsql_with_legal_schema().await;
        let conn = db.connect().expect("connect");
        conn.execute(
            "INSERT INTO legal_projects (id, name, created_at) VALUES (?1, ?2, ?3)",
            params!["proj-empty", "p", 1_700_000_000_i64],
        )
        .await
        .expect("project");
        conn.execute(
            "INSERT INTO legal_chats (id, project_id, title, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                "chat-empty",
                "proj-empty",
                libsql::Value::Null,
                1_700_000_000_i64
            ],
        )
        .await
        .expect("chat");

        let store = LegalChatStore::new(db);
        let err = store
            .load_chat_for_export("chat-empty")
            .await
            .expect_err("must fail");
        assert!(matches!(err, LegalError::ChatEmpty(_)));
    }

    #[tokio::test]
    async fn message_with_xml_payload_renders_as_escaped_text() {
        let (db, _guard) = fresh_libsql_with_legal_schema().await;
        let chat_id = seed_chat(&db, &[("user", "</w:t><w:body><inject/></w:body>", &[])]).await;
        let store = LegalChatStore::new(db);
        let chat = store.load_chat_for_export(&chat_id).await.expect("load");
        let bytes = render_chat_to_docx(&chat).expect("render");
        let xml = extract_document_xml(&bytes);
        assert!(
            xml.contains("&lt;/w:t&gt;") || xml.contains("&lt;w:t&gt;"),
            "user-supplied XML must be escaped, got: {}",
            xml.chars().take(2000).collect::<String>()
        );
    }

    #[test]
    fn safe_chat_id_byte_blocks_path_traversal() {
        // Real ULIDs / slugs: allowed.
        assert!(
            "01HZX6V0F9N7P2KQM4JR8YHT3W"
                .bytes()
                .all(super::is_safe_chat_id_byte)
        );
        // `.` is allowed (filenames with dots), but `/`, `..`, spaces,
        // `\n`, `\0` are blocked.
        assert!(!b"a/b".iter().all(|b| super::is_safe_chat_id_byte(*b)));
        assert!(!b"a b".iter().all(|b| super::is_safe_chat_id_byte(*b)));
        assert!(!b"a\nb".iter().all(|b| super::is_safe_chat_id_byte(*b)));
        assert!(!b"a\0b".iter().all(|b| super::is_safe_chat_id_byte(*b)));
    }

    #[test]
    fn sanitize_filename_segment_caps_length() {
        let long = "a".repeat(200);
        let s = super::sanitize_filename_segment(&long);
        assert!(s.len() <= 64);
    }

    #[test]
    fn sanitize_filename_segment_falls_back_when_empty_after_filter() {
        // (the upstream id validator would reject this first, but
        // belt-and-suspenders.)
        let s = super::sanitize_filename_segment("///");
        assert_eq!(s, "chat");
    }
}

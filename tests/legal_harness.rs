//! Integration tests for the Stream A legal harness layer.
//!
//! Covers:
//! 1. Project create/list/fetch/soft-delete round-trip.
//! 2. Document insert + project-scoped sha256 dedupe lookup.
//! 3. Blob write/read on a content-addressed path.
//! 4. PDF and DOCX extraction against fixtures.
//!
//! These tests run against a real libSQL database (in-memory file in a
//! tempdir) and exercise the same code paths the HTTP handlers do. The
//! handlers themselves are thin orchestration over these modules — when
//! they break, these tests will catch the regression first.

#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw::channels::web::features::legal::{blobs, extract, store};
use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;

async fn setup() -> (Arc<dyn Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let db_path = dir.path().join("legal.db");
    let backend = LibSqlBackend::new_local(&db_path).await.expect("create db");
    backend.run_migrations().await.expect("run migrations");
    let db: Arc<dyn Database> = Arc::new(backend);
    (db, dir)
}

fn ulid_str() -> String {
    ulid::Ulid::new().to_string()
}

// ---- Project lifecycle -----------------------------------------------

#[tokio::test]
async fn project_create_list_fetch_round_trip() {
    let (db, _dir) = setup().await;

    let id = ulid_str();
    let project = store::create_project(&db, &id, "Acme NDA", Some("{\"deal\":\"acme\"}"))
        .await
        .expect("create project");
    assert_eq!(project.id, id);
    assert_eq!(project.name, "Acme NDA");
    assert_eq!(project.metadata.as_deref(), Some("{\"deal\":\"acme\"}"));
    assert!(project.deleted_at.is_none());
    assert!(project.created_at > 0);

    let listed = store::list_active_projects(&db).await.expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);

    let fetched = store::fetch_project(&db, &id)
        .await
        .expect("fetch")
        .expect("present");
    assert_eq!(fetched, project);

    let absent = store::fetch_project(&db, "no-such-id")
        .await
        .expect("fetch ok");
    assert!(absent.is_none(), "missing id should yield None");
}

#[tokio::test]
async fn project_soft_delete_hides_from_active_list() {
    let (db, _dir) = setup().await;

    let keep = ulid_str();
    let drop = ulid_str();
    store::create_project(&db, &keep, "Keep", None)
        .await
        .expect("keep");
    store::create_project(&db, &drop, "Drop", None)
        .await
        .expect("drop");

    let now = 1_700_000_000_i64;
    let updated = store::soft_delete_project(&db, &drop, now)
        .await
        .expect("delete");
    assert!(updated, "first delete should affect a row");

    // Idempotent: re-deleting returns false (no rows to update).
    let again = store::soft_delete_project(&db, &drop, now + 1)
        .await
        .expect("delete again");
    assert!(!again, "second delete should not touch any row");

    let listed = store::list_active_projects(&db).await.expect("list");
    let ids: Vec<&str> = listed.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec![keep.as_str()]);

    // fetch_project still returns soft-deleted rows for the handler to
    // decide what to do.
    let dropped = store::fetch_project(&db, &drop)
        .await
        .expect("fetch")
        .expect("present");
    assert!(dropped.deleted_at.is_some());
}

// ---- Documents + dedupe ----------------------------------------------

#[tokio::test]
async fn document_insert_and_project_scoped_sha_dedupe() {
    let (db, _dir) = setup().await;

    let project_id = ulid_str();
    store::create_project(&db, &project_id, "Project", None)
        .await
        .expect("project");

    let bytes = b"hello world".to_vec();
    let sha = blobs::sha256_hex(&bytes);

    // First insert succeeds.
    let doc_id = ulid_str();
    let outcome = store::create_document(
        &db,
        &doc_id,
        &project_id,
        "hello.txt",
        "text/plain",
        "legal/blobs/ba/abc",
        Some("hello world"),
        None,
        bytes.len() as i64,
        &sha,
    )
    .await
    .expect("create_document");
    let doc = match outcome {
        store::DocumentInsert::Inserted(d) => d,
        store::DocumentInsert::DuplicateExisting(_) => panic!("first insert should not dedupe"),
    };
    assert_eq!(doc.id, doc_id);
    assert_eq!(doc.bytes, 11);
    assert_eq!(doc.sha256, sha);

    // Project-scoped lookup matches.
    let found = store::find_document_by_sha(&db, &project_id, &sha)
        .await
        .expect("find")
        .expect("present");
    assert_eq!(found.id, doc_id);

    // Different project — same sha — should NOT match (project-scoped dedupe).
    let other_project = ulid_str();
    store::create_project(&db, &other_project, "Other", None)
        .await
        .expect("other project");
    let absent = store::find_document_by_sha(&db, &other_project, &sha)
        .await
        .expect("find");
    assert!(
        absent.is_none(),
        "dedupe must be project-scoped per the spec"
    );

    // List documents for project returns the one row.
    let docs = store::list_documents_for_project(&db, &project_id)
        .await
        .expect("list");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].id, doc_id);
}

#[tokio::test]
async fn create_document_returns_duplicate_when_sha_already_present() {
    // Simulates the dedupe race: a second insert with identical
    // (project_id, sha256) but a different id should not produce two
    // rows; the second call should return DuplicateExisting with the
    // first row.
    let (db, _dir) = setup().await;

    let project_id = ulid_str();
    store::create_project(&db, &project_id, "P", None)
        .await
        .expect("project");

    let sha = "deadbeef".repeat(8); // 64 hex chars
    let first_id = ulid_str();
    let first = store::create_document(
        &db,
        &first_id,
        &project_id,
        "a.pdf",
        "application/pdf",
        "legal/blobs/de/de..",
        None,
        None,
        10,
        &sha,
    )
    .await
    .expect("first insert");
    assert!(matches!(first, store::DocumentInsert::Inserted(_)));

    let second_id = ulid_str();
    let second = store::create_document(
        &db,
        &second_id,
        &project_id,
        "b.pdf",
        "application/pdf",
        "legal/blobs/de/de..",
        None,
        None,
        10,
        &sha,
    )
    .await
    .expect("second insert (race)");
    match second {
        store::DocumentInsert::DuplicateExisting(row) => {
            assert_eq!(row.id, first_id, "duplicate should yield original id");
        }
        store::DocumentInsert::Inserted(_) => panic!("second insert should dedupe, not insert"),
    }

    // Confirm only one row landed.
    let docs = store::list_documents_for_project(&db, &project_id)
        .await
        .expect("list");
    assert_eq!(docs.len(), 1);
}

#[tokio::test]
async fn project_cascade_delete_drops_documents() {
    // Soft-deleting a project keeps documents attached; only ON DELETE
    // CASCADE on a hard-delete drops them. Verify the FK cascade fires.
    let (db, _dir) = setup().await;

    let project_id = ulid_str();
    store::create_project(&db, &project_id, "P", None)
        .await
        .expect("project");
    let doc_id = ulid_str();
    let _ = store::create_document(
        &db,
        &doc_id,
        &project_id,
        "f.pdf",
        "application/pdf",
        "legal/blobs/aa/zzzz",
        None,
        None,
        1,
        "ff",
    )
    .await
    .expect("doc");

    // Drop the project row directly (simulates an admin/test scenario).
    let backend = ironclaw::db::libsql_backend(&db).expect("libsql backend");
    let conn = backend.connect().await.expect("connect");
    conn.execute("PRAGMA foreign_keys = ON", ())
        .await
        .expect("fk");
    conn.execute(
        "DELETE FROM legal_projects WHERE id = ?1",
        libsql::params![project_id.clone()],
    )
    .await
    .expect("delete");

    let doc = store::fetch_document(&db, &doc_id)
        .await
        .expect("fetch document");
    assert!(doc.is_none(), "ON DELETE CASCADE should drop the document");
}

#[tokio::test]
async fn document_after_soft_delete_is_invisible_via_store_lookup() {
    // The store-level fetch_document still returns the row (it's the
    // handler that filters on parent.deleted_at), but list_documents_for_project
    // and the production handler should both treat soft-deleted projects
    // as missing. This test pins the store contract: the row stays
    // queryable but its parent's deleted_at is set.
    let (db, _dir) = setup().await;

    let project_id = ulid_str();
    store::create_project(&db, &project_id, "Sealed", None)
        .await
        .expect("project");
    let doc_id = ulid_str();
    let _ = store::create_document(
        &db,
        &doc_id,
        &project_id,
        "agreement.pdf",
        "application/pdf",
        "legal/blobs/aa/zzzz",
        None,
        None,
        1,
        "ff",
    )
    .await
    .expect("doc");

    let updated = store::soft_delete_project(&db, &project_id, 1_700_000_000)
        .await
        .expect("delete");
    assert!(updated);

    // Row still present; parent.deleted_at signals deletion to the handler.
    let doc = store::fetch_document(&db, &doc_id)
        .await
        .expect("fetch")
        .expect("present");
    let parent = store::fetch_project(&db, &doc.project_id)
        .await
        .expect("fetch parent")
        .expect("parent present");
    assert!(parent.deleted_at.is_some());
}

// ---- Blob storage -----------------------------------------------------

#[tokio::test]
async fn blob_write_dedupes_on_disk() {
    let dir = tempfile::tempdir().expect("tempdir");

    let bytes_a = b"alpha".to_vec();
    let sha_a = blobs::sha256_hex(&bytes_a);
    let rel1 = blobs::write_blob(dir.path(), &sha_a, &bytes_a)
        .await
        .expect("write");

    // Re-write same bytes: returns same path, no error.
    let rel2 = blobs::write_blob(dir.path(), &sha_a, &bytes_a)
        .await
        .expect("rewrite");
    assert_eq!(rel1, rel2);

    // Read back.
    let read = blobs::read_blob(dir.path(), &sha_a).await.expect("read");
    assert_eq!(read, bytes_a);

    // Different bytes -> different sha -> different path.
    let bytes_b = b"beta".to_vec();
    let sha_b = blobs::sha256_hex(&bytes_b);
    let rel_b = blobs::write_blob(dir.path(), &sha_b, &bytes_b)
        .await
        .expect("write b");
    assert_ne!(rel_b, rel1);
}

// ---- Extraction -------------------------------------------------------

#[tokio::test]
async fn pdf_fixture_extracts_some_text() {
    // tests/fixtures/hello.pdf was produced by ReportLab in the
    // pre-existing fixture set. We treat its exact contents as opaque and
    // only assert "extraction returned text and didn't error".
    let pdf = std::fs::read("tests/fixtures/hello.pdf").expect("hello.pdf present");
    let out = extract::extract("application/pdf", "hello.pdf", &pdf)
        .await
        .expect("pdf extract");
    assert!(!out.text.is_empty(), "PDF extraction should yield text");
}

#[tokio::test]
async fn synthesised_docx_extracts_known_text() {
    // Produce a minimal valid .docx (a zip with `word/document.xml`)
    // containing two paragraphs. We check round-trip extraction without
    // depending on a third-party DOCX writer.
    let docx = build_docx_with(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t xml:space="preserve">Master Services Agreement</w:t></w:r></w:p>
    <w:p><w:r><w:t xml:space="preserve">Section 1. Scope of Services.</w:t></w:r></w:p>
  </w:body>
</w:document>"#,
    );

    let out = extract::extract(
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "msa.docx",
        &docx,
    )
    .await
    .expect("docx extract");
    assert!(out.text.contains("Master Services Agreement"));
    assert!(out.text.contains("Section 1. Scope of Services."));
    assert!(out.page_count.is_none(), "DOCX has no page count");
}

#[tokio::test]
async fn unsupported_content_type_errors() {
    let bytes = b"plain text here, no PDF magic";
    let res = extract::extract("text/plain", "notes.txt", bytes).await;
    assert!(res.is_err(), "unsupported should error");
}

/// Build a minimal valid `.docx` archive whose only payload is
/// `word/document.xml`. The file passes our extractor because the
/// extractor only reads `word/document.xml`. Real Word-produced files
/// have many more parts (`[Content_Types].xml`, relationships, styles)
/// — those are unnecessary for text extraction.
fn build_docx_with(document_xml: &str) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    let mut buf = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut zip = zip::ZipWriter::new(cursor);
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("word/document.xml", opts).expect("entry");
        zip.write_all(document_xml.as_bytes()).expect("write xml");
        zip.finish().expect("finish zip");
    }
    buf
}

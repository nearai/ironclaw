//! Bridge inbound attachment bytes into durable transcript references.
//!
//! This is the unit the byte-bearing ingress layer calls: it lands each
//! attachment's bytes through the project filesystem authority (see
//! [`crate::land_attachment`]) and produces the channel-agnostic
//! [`AttachmentRef`] the transcript persists, with `storage_key` set to the
//! landed [`ScopedPath`]. Document attachments are also run through
//! [`ironclaw_extractors`] to fill `extracted_text`; audio transcription is
//! provider-backed and handled by a later pipeline stage.

use ironclaw_common::{AttachmentKind, AttachmentRef, canonical_extension, kind_for_mime};
use ironclaw_filesystem::{
    Entry, FilesystemError, RootFilesystem, ScopedAtomicSubtreeEntry, ScopedFilesystem,
};
use ironclaw_host_api::{attachment::InboundAttachment, path::ScopedPath, resource::ResourceScope};
use sha2::{Digest, Sha256};

use crate::budgets::DEFAULT_ATTACHMENT_BUDGETS;
use crate::landing::{
    AttachmentLanding, AttachmentLandingError, attachment_batch_scoped_path, attachment_scoped_path,
};

/// Canonical extension to synthesize a filename with when the MIME type is not
/// in the attachment format registry.
const UNKNOWN_EXTENSION: &str = "bin";
const BATCH_MANIFEST_FILENAME: &str = ".ironclaw-attachment-batch-v1";

/// Land each inbound attachment's bytes under the project mount and return the
/// transcript references, with `storage_key` set to the landed [`ScopedPath`]
/// and `size_bytes` set to the landed byte count.
///
/// Writes go through `filesystem`, so a read-only project mount fails closed.
/// Each attachment is bounded by `max_bytes` and an over-limit one fails the
/// batch with [`AttachmentLandingError::TooLarge`]. Validation and document
/// extraction finish before one atomic subtree creation publishes the complete
/// message batch; any failure leaves the message prefix absent.
///
/// [`ScopedPath`]: ironclaw_host_api::path::ScopedPath
pub async fn land_inbound_attachments<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    project_alias: &str,
    date: &str,
    message_id: &str,
    attachments: Vec<InboundAttachment>,
    max_bytes: usize,
) -> Result<Vec<AttachmentRef>, AttachmentLandingError>
where
    F: RootFilesystem,
{
    if attachments.is_empty() {
        return Ok(Vec::new());
    }
    if attachments.len() > DEFAULT_ATTACHMENT_BUDGETS.max_count {
        return Err(AttachmentLandingError::TooMany {
            count: attachments.len(),
            max: DEFAULT_ATTACHMENT_BUDGETS.max_count,
        });
    }
    let max_bytes = max_bytes.min(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes);
    let mut total_bytes = 0usize;
    for attachment in &attachments {
        if attachment.bytes.len() > max_bytes {
            return Err(AttachmentLandingError::TooLarge {
                size: attachment.bytes.len(),
                max: max_bytes,
            });
        }
        total_bytes = total_bytes
            .checked_add(attachment.bytes.len())
            .unwrap_or(DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes.saturating_add(1));
        if total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes {
            return Err(AttachmentLandingError::BatchTooLarge {
                size: total_bytes,
                max: DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes,
            });
        }
    }
    let batch_path = attachment_batch_scoped_path(project_alias, date, message_id)?;
    let mut refs = Vec::with_capacity(attachments.len());
    let mut entries = Vec::with_capacity(attachments.len().saturating_add(1));
    for (index, attachment) in attachments.into_iter().enumerate() {
        let InboundAttachment {
            id,
            mime_type,
            filename,
            bytes,
        } = attachment;
        let size_bytes = bytes.len() as u64;
        // Derive kind and fallback extension from the MIME type so a ref's
        // `kind` is always consistent with its `mime_type`.
        let kind = kind_for_mime(&mime_type);
        let fallback_extension = canonical_extension(&mime_type).unwrap_or(UNKNOWN_EXTENSION);
        // Extract document text before the bytes are moved into the write.
        // Images go to the vision model; audio transcription is provider-backed
        // and handled by a later pipeline stage. Video and other binary media
        // also have no document extractor, so all four leave `extracted_text`
        // unset here.
        let extracted_text = match kind {
            AttachmentKind::Document => {
                extract_document_text(&bytes, &mime_type, filename.as_deref())
            }
            AttachmentKind::Image
            | AttachmentKind::Audio
            | AttachmentKind::Video
            | AttachmentKind::Other => None,
        };
        let landing = AttachmentLanding {
            message_id,
            index,
            filename: filename.as_deref(),
            fallback_extension,
        };
        let stored = attachment_scoped_path(project_alias, date, &landing)?;
        entries.push(ScopedAtomicSubtreeEntry {
            path: stored.clone(),
            entry: Entry::bytes(bytes),
        });
        refs.push(AttachmentRef {
            id,
            kind,
            mime_type,
            filename,
            size_bytes: Some(size_bytes),
            storage_key: Some(stored.as_str().to_string()),
            extracted_text,
        });
    }
    let manifest_path = ScopedPath::new(format!(
        "{}/{}",
        batch_path.as_str(),
        BATCH_MANIFEST_FILENAME
    ))?;
    // Cover both the durable refs and every attachment body. The manifest is
    // committed in the same atomic subtree, so an exact manifest match is
    // sufficient to recognize an identical provider replay without retaining
    // a second copy of the complete byte-bearing batch in memory.
    let manifest = batch_manifest_digest(&refs, &entries);
    entries.push(ScopedAtomicSubtreeEntry {
        path: manifest_path.clone(),
        entry: Entry::bytes(manifest.clone()),
    });
    match filesystem
        .create_subtree_atomic(scope, &batch_path, entries)
        .await
    {
        Ok(_) => Ok(refs),
        Err(conflict @ FilesystemError::VersionMismatch { .. }) => {
            match filesystem.get(scope, &manifest_path).await {
                Ok(Some(committed)) if committed.entry.body == manifest => Ok(refs),
                Ok(_) | Err(FilesystemError::PermissionDenied { .. }) => {
                    Err(AttachmentLandingError::Write(conflict))
                }
                Err(error) => Err(AttachmentLandingError::Write(error)),
            }
        }
        Err(error) => Err(AttachmentLandingError::Write(error)),
    }
}

fn batch_manifest_digest(refs: &[AttachmentRef], entries: &[ScopedAtomicSubtreeEntry]) -> Vec<u8> {
    let mut digest = Sha256::new();
    digest.update(b"ironclaw.attachment-batch.v1");
    digest.update((refs.len() as u64).to_be_bytes());
    for (attachment, entry) in refs.iter().zip(entries) {
        digest_field(&mut digest, attachment.id.as_bytes());
        digest.update([match attachment.kind {
            AttachmentKind::Document => 0,
            AttachmentKind::Image => 1,
            AttachmentKind::Audio => 2,
            AttachmentKind::Video => 3,
            AttachmentKind::Other => 4,
        }]);
        digest_field(&mut digest, attachment.mime_type.as_bytes());
        digest_optional_field(&mut digest, attachment.filename.as_deref());
        match attachment.size_bytes {
            Some(size) => {
                digest.update([1]);
                digest.update(size.to_be_bytes());
            }
            None => digest.update([0]),
        }
        digest_optional_field(&mut digest, attachment.storage_key.as_deref());
        digest_optional_field(&mut digest, attachment.extracted_text.as_deref());
        digest_field(&mut digest, entry.path.as_str().as_bytes());
        digest_field(&mut digest, &entry.entry.body);
    }
    digest.finalize().to_vec()
}

fn digest_optional_field(digest: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            digest.update([1]);
            digest_field(digest, value.as_bytes());
        }
        None => digest.update([0]),
    }
}

fn digest_field(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

/// Maximum characters of extracted document *content* retained on a reference
/// (~25K tokens). Mirrors the v1 document-extraction cap. When truncation
/// occurs a short `[... truncated ...]` marker is appended, so the stored
/// `extracted_text` may exceed this by the marker's fixed length.
const MAX_EXTRACTED_TEXT_CHARS: usize = 100_000;

/// Run the type-aware text extractor over a document attachment's bytes and
/// return the extracted text, truncated to [`MAX_EXTRACTED_TEXT_CHARS`].
///
/// Returns `None` when extraction yields nothing or fails — the attachment is
/// still landed and referenced, the model just won't have its text.
fn extract_document_text(bytes: &[u8], mime: &str, filename: Option<&str>) -> Option<String> {
    match ironclaw_extractors::extract_document(bytes, mime, filename) {
        ironclaw_extractors::DocumentExtraction::Text(text) => Some(
            ironclaw_extractors::truncate_to_chars(&text, MAX_EXTRACTED_TEXT_CHARS),
        ),
        // Extraction yielded nothing usable — the attachment is still landed and
        // referenced, the model just won't have its text.
        ironclaw_extractors::DocumentExtraction::Empty => None,
        ironclaw_extractors::DocumentExtraction::Failed(error) => {
            // Extraction failure is non-fatal — the attachment is still landed
            // and referenced, the model just won't have its text. Log it so an
            // unsupported-format/corrupt-file case is observable (debug, not
            // warn: this runs in library context that may back the REPL/TUI).
            // `?error` (Debug), not `%error` (Display): the parser diagnostic
            // is deliberately absent from `Display` and a log is exactly where
            // it belongs. Nothing here reaches the model.
            tracing::debug!(mime, filename, ?error, "document text extraction failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::{DEFAULT_ATTACHMENT_BUDGETS, DEFAULT_MAX_ATTACHMENT_BYTES};
    use ironclaw_common::AttachmentKind;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        ids::{InvocationId, TenantId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, ScopedPath, VirtualPath},
        resource::ResourceScope,
    };

    // The crate no longer exports a default alias (the host composition owns
    // the canonical `/workspace` mount alias); the bridge tests pin it locally.
    const DEFAULT_PROJECT_MOUNT_ALIAS: &str = "/workspace";

    fn project_mount(
        backend: Arc<InMemoryBackend>,
        permissions: MountPermissions,
    ) -> ScopedFilesystem<InMemoryBackend> {
        ScopedFilesystem::with_fixed_view(
            backend,
            MountView::new(vec![MountGrant::new(
                MountAlias::new(DEFAULT_PROJECT_MOUNT_ALIAS).unwrap(),
                VirtualPath::new("/projects/workspace").unwrap(),
                permissions,
            )])
            .unwrap(),
        )
    }

    fn test_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-test").unwrap(),
            user_id: UserId::new("user-test").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn inbound(id: &str, mime: &str, filename: &str, bytes: &[u8]) -> InboundAttachment {
        InboundAttachment {
            id: id.to_string(),
            mime_type: mime.to_string(),
            filename: Some(filename.to_string()),
            bytes: bytes.to_vec(),
        }
    }

    #[tokio::test]
    async fn lands_bytes_and_sets_storage_key_on_each_ref() {
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(Arc::clone(&backend), MountPermissions::read_write());
        let scope = test_scope();

        let doc_bytes = b"%PDF-1.7 doc".to_vec();
        let img_bytes = vec![0x89, 0x50, 0x4E, 0x47];
        let refs = land_inbound_attachments(
            &writer,
            &scope,
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![
                inbound("att-0", "application/pdf", "report.pdf", &doc_bytes),
                inbound("att-1", "image/png", "diagram.png", &img_bytes),
            ],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("batch lands");

        assert_eq!(refs.len(), 2);

        assert_eq!(refs[0].id, "att-0");
        assert_eq!(refs[0].kind, AttachmentKind::Document);
        assert_eq!(refs[0].mime_type, "application/pdf");
        assert_eq!(refs[0].filename.as_deref(), Some("report.pdf"));
        assert_eq!(refs[0].size_bytes, Some(doc_bytes.len() as u64));
        assert_eq!(
            refs[0].storage_key.as_deref(),
            Some("/workspace/attachments/2026-06-09/msg1/1-report.pdf")
        );
        assert!(refs[0].extracted_text.is_none());

        // `kind` is derived from the MIME type, not supplied by the caller.
        assert_eq!(refs[1].kind, AttachmentKind::Image);
        assert_eq!(
            refs[1].storage_key.as_deref(),
            Some("/workspace/attachments/2026-06-09/msg1/2-diagram.png")
        );

        // The bytes are addressable at each ref's storage_key through the same
        // authority — a reader resolves the recorded ScopedPath with no extra
        // wiring.
        let reader = project_mount(backend, MountPermissions::read_only());
        for (att_ref, expected) in refs.iter().zip([doc_bytes, img_bytes]) {
            let path = ScopedPath::new(att_ref.storage_key.clone().unwrap())
                .expect("storage_key is a scoped path");
            let got = reader
                .get(&scope, &path)
                .await
                .expect("read succeeds")
                .expect("landed attachment is present");
            assert_eq!(got.entry.body, expected);
        }
    }

    #[tokio::test]
    async fn same_filename_attachments_land_at_distinct_paths() {
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(backend, MountPermissions::read_write());
        let refs = land_inbound_attachments(
            &writer,
            &test_scope(),
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![
                inbound("att-0", "text/csv", "data.csv", b"a,b\n1,2"),
                inbound("att-1", "text/csv", "data.csv", b"c,d\n3,4"),
            ],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("batch lands");

        assert_ne!(
            refs[0].storage_key, refs[1].storage_key,
            "same-filename attachments must not collide on one storage path"
        );
    }

    #[tokio::test]
    async fn fails_closed_on_read_only_project_mount() {
        let backend = Arc::new(InMemoryBackend::new());
        let read_only = project_mount(backend, MountPermissions::read_only());
        let err = land_inbound_attachments(
            &read_only,
            &test_scope(),
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![inbound("att-0", "application/pdf", "report.pdf", b"%PDF")],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect_err("a read-only project mount must reject the landing");
        assert!(matches!(err, AttachmentLandingError::Write(_)));
    }

    #[tokio::test]
    async fn lands_with_synthesized_filename_when_filename_absent() {
        // The `filename = None` path: the landed name is synthesized from the
        // index and the registry-derived extension, and the ref's `filename`
        // stays `None`. Exercises the InboundAttachment -> landing wiring the
        // named-file tests never reach.
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(backend, MountPermissions::read_write());
        let refs = land_inbound_attachments(
            &writer,
            &test_scope(),
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![InboundAttachment {
                id: "att-0".to_string(),
                mime_type: "image/png".to_string(),
                filename: None,
                bytes: vec![0x89, 0x50],
            }],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("lands");
        assert_eq!(
            refs[0].storage_key.as_deref(),
            // `png` is derived from `image/png`; `1` is the 1-based attachment
            // index; the synthesized name is `attachment.<ext>`.
            Some("/workspace/attachments/2026-06-09/msg1/1-attachment.png")
        );
        assert!(refs[0].filename.is_none());
    }

    #[tokio::test]
    async fn conflicting_message_batch_retry_preserves_the_committed_batch() {
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(Arc::clone(&backend), MountPermissions::read_write());
        let scope = test_scope();

        let committed = land_inbound_attachments(
            &writer,
            &scope,
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![inbound("att-0", "text/plain", "a.txt", b"committed")],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("initial batch lands");

        let err = land_inbound_attachments(
            &writer,
            &scope,
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![inbound(
                "att-metadata-conflict",
                "text/plain",
                "a.txt",
                b"committed",
            )],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect_err("a conflicting retry must fail closed");
        assert!(matches!(err, AttachmentLandingError::Write(_)));

        let reader = project_mount(backend, MountPermissions::read_only());
        let committed_path =
            ScopedPath::new(committed[0].storage_key.clone().expect("storage key")).unwrap();
        let stored = reader
            .get(&scope, &committed_path)
            .await
            .expect("read succeeds")
            .expect("the committed attachment remains");
        assert_eq!(stored.entry.body, b"committed");
    }

    #[tokio::test]
    async fn identical_message_batch_retry_returns_the_existing_ordered_refs() {
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(Arc::clone(&backend), MountPermissions::read_write());
        let scope = test_scope();
        let attachments = vec![
            inbound("att-0", "text/plain", "a.txt", b"first"),
            inbound("att-1", "text/plain", "b.txt", b"second"),
        ];

        let first = land_inbound_attachments(
            &writer,
            &scope,
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            attachments.clone(),
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("initial batch lands");
        let replay = land_inbound_attachments(
            &writer,
            &scope,
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            attachments,
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("an identical retry reuses the committed batch");

        assert_eq!(replay, first);
        assert_eq!(
            replay
                .iter()
                .map(|attachment| attachment.id.as_str())
                .collect::<Vec<_>>(),
            vec!["att-0", "att-1"]
        );
    }

    #[tokio::test]
    async fn write_only_message_batch_replay_returns_the_original_conflict() {
        let backend = Arc::new(InMemoryBackend::new());
        let mut write_only = MountPermissions::none();
        write_only.write = true;
        let writer = project_mount(backend, write_only);
        let scope = test_scope();
        let attachments = vec![inbound("att-0", "text/plain", "a.txt", b"first")];

        land_inbound_attachments(
            &writer,
            &scope,
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            attachments.clone(),
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("initial write-only batch lands");
        let error = land_inbound_attachments(
            &writer,
            &scope,
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            attachments,
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect_err("write-only authority cannot prove an identical replay");

        assert!(matches!(
            error,
            AttachmentLandingError::Write(FilesystemError::VersionMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn rejects_an_oversized_attachment_in_the_batch() {
        // The bridge threads its `max_bytes` bound to each landing; an item over
        // the cap fails the batch with TooLarge before its bytes are written.
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(Arc::clone(&backend), MountPermissions::read_write());
        let scope = test_scope();

        let err = land_inbound_attachments(
            &writer,
            &scope,
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![inbound("att-0", "text/plain", "big.txt", b"0123456789")],
            8,
        )
        .await
        .expect_err("an over-limit attachment must fail the batch");
        assert!(matches!(
            err,
            AttachmentLandingError::TooLarge { size: 10, max: 8 }
        ));

        // Rejected before any write — nothing landed.
        let reader = project_mount(backend, MountPermissions::read_only());
        assert!(
            reader
                .get(
                    &scope,
                    &ScopedPath::new("/workspace/attachments/2026-06-09/msg1-1-big.txt").unwrap(),
                )
                .await
                .expect("read succeeds")
                .is_none(),
            "oversized attachment must not have been written"
        );
    }

    #[tokio::test]
    async fn rejects_more_than_the_shared_attachment_count_limit() {
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(backend, MountPermissions::read_write());
        let attachments = (0..=DEFAULT_ATTACHMENT_BUDGETS.max_count)
            .map(|index| inbound(&format!("att-{index}"), "text/plain", "a.txt", b"x"))
            .collect();

        let error = land_inbound_attachments(
            &writer,
            &test_scope(),
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            attachments,
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect_err("the shared count limit must reject the complete batch");

        assert!(matches!(
            error,
            AttachmentLandingError::TooMany { count, max }
                if count == DEFAULT_ATTACHMENT_BUDGETS.max_count + 1
                    && max == DEFAULT_ATTACHMENT_BUDGETS.max_count
        ));
    }

    #[tokio::test]
    async fn caller_cannot_broaden_the_shared_per_file_limit() {
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(backend, MountPermissions::read_write());
        let bytes = vec![b'x'; DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes + 1];

        let error = land_inbound_attachments(
            &writer,
            &test_scope(),
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![inbound("att-0", "text/plain", "large.txt", &bytes)],
            usize::MAX,
        )
        .await
        .expect_err("a caller may narrow but never broaden the shared limit");

        assert!(matches!(
            error,
            AttachmentLandingError::TooLarge { size, max }
                if size == DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes + 1
                    && max == DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes
        ));
    }

    #[tokio::test]
    async fn rejects_a_batch_over_the_shared_total_byte_limit() {
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(backend, MountPermissions::read_write());
        let half_plus_one = DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes / 2 + 1;
        let first = vec![b'a'; half_plus_one];
        let second = vec![b'b'; half_plus_one];

        let error = land_inbound_attachments(
            &writer,
            &test_scope(),
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![
                inbound("att-0", "application/octet-stream", "a.bin", &first),
                inbound("att-1", "application/octet-stream", "b.bin", &second),
            ],
            DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes,
        )
        .await
        .expect_err("the shared total byte limit must reject the complete batch");

        assert!(matches!(
            error,
            AttachmentLandingError::BatchTooLarge { size, max }
                if size == half_plus_one * 2
                    && max == DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes
        ));
    }

    #[tokio::test]
    async fn document_attachment_gets_extracted_text() {
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(backend, MountPermissions::read_write());
        let refs = land_inbound_attachments(
            &writer,
            &test_scope(),
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![inbound(
                "att-0",
                "text/csv",
                "data.csv",
                b"name,score\nalice,9",
            )],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("batch lands");
        assert_eq!(
            refs[0].extracted_text.as_deref(),
            Some("name,score\nalice,9")
        );
    }

    #[tokio::test]
    async fn image_attachment_has_no_extracted_text() {
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(backend, MountPermissions::read_write());
        let refs = land_inbound_attachments(
            &writer,
            &test_scope(),
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![inbound(
                "att-0",
                "image/png",
                "x.png",
                &[0x89, 0x50, 0x4E, 0x47],
            )],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("batch lands");
        assert!(refs[0].extracted_text.is_none());
    }

    #[test]
    fn extract_document_text_truncates_long_text() {
        let long = "x".repeat(MAX_EXTRACTED_TEXT_CHARS + 50);
        let out = extract_document_text(long.as_bytes(), "text/plain", None)
            .expect("non-empty text extracts");
        assert!(out.contains("[... truncated"));
        assert!(out.chars().count() <= MAX_EXTRACTED_TEXT_CHARS + 60);
    }

    #[test]
    fn extract_document_text_is_none_on_empty() {
        assert!(extract_document_text(b"   \n  ", "text/plain", None).is_none());
    }
}

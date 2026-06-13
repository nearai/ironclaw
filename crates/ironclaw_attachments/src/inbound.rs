//! Bridge inbound attachment bytes into durable transcript references.
//!
//! This is the unit the byte-bearing ingress layer calls: it lands each
//! attachment's bytes through the project filesystem authority (see
//! [`crate::land_attachment`]) and produces the channel-agnostic
//! [`AttachmentRef`] the transcript persists, with `storage_key` set to the
//! landed [`ScopedPath`]. `extracted_text` is left `None` here — document
//! extraction and audio transcription run in a later pipeline stage.

use ironclaw_common::{AttachmentRef, canonical_extension, kind_for_mime};
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ResourceScope;

use crate::landing::{AttachmentLanding, AttachmentLandingError, land_attachment};

/// Canonical extension to synthesize a filename with when the MIME type is not
/// in the attachment format registry.
const UNKNOWN_EXTENSION: &str = "bin";

/// One inbound attachment with its raw bytes, ready to be landed and turned
/// into a transcript [`AttachmentRef`].
///
/// The attachment `kind` and the fallback filename extension are *derived from*
/// `mime_type` against the [`ironclaw_common`] attachment format registry — the
/// authoritative source — so callers cannot drift them out of sync with the
/// MIME type they pass.
#[derive(Debug, Clone)]
pub struct InboundAttachment {
    /// Stable identifier for this attachment within its message.
    pub id: String,
    /// MIME type as received at the ingress boundary. The attachment kind and
    /// fallback extension are derived from this.
    pub mime_type: String,
    /// Original filename, when the source provided one.
    pub filename: Option<String>,
    /// Raw attachment bytes to land in the project filesystem.
    pub bytes: Vec<u8>,
}

/// Land each inbound attachment's bytes under the project mount and return the
/// transcript references, with `storage_key` set to the landed [`ScopedPath`]
/// and `size_bytes` set to the landed byte count.
///
/// Writes go through `filesystem`, so a read-only project mount fails closed
/// (see [`land_attachment`]). Each attachment is bounded by `max_bytes` and an
/// over-limit one fails the batch with [`AttachmentLandingError::TooLarge`]. On
/// the first failure the whole batch returns the error; partial writes that may
/// already have landed are left in place (the filesystem authority makes them
/// addressable, and a retry re-lands at the same deterministic paths).
///
/// [`ScopedPath`]: ironclaw_host_api::ScopedPath
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
    let mut refs = Vec::with_capacity(attachments.len());
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
        let landing = AttachmentLanding {
            message_id,
            index,
            filename: filename.as_deref(),
            fallback_extension,
        };
        let stored = land_attachment(
            filesystem,
            scope,
            project_alias,
            date,
            &landing,
            bytes,
            max_bytes,
        )
        .await?;
        refs.push(AttachmentRef {
            id,
            kind,
            mime_type,
            filename,
            size_bytes: Some(size_bytes),
            storage_key: Some(stored.as_str().to_string()),
            extracted_text: None,
        });
    }
    Ok(refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::DEFAULT_MAX_ATTACHMENT_BYTES;
    use ironclaw_common::AttachmentKind;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
        ScopedPath, TenantId, UserId, VirtualPath,
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
            Some("/workspace/attachments/2026-06-09/msg1-1-report.pdf")
        );
        assert!(refs[0].extracted_text.is_none());

        // `kind` is derived from the MIME type, not supplied by the caller.
        assert_eq!(refs[1].kind, AttachmentKind::Image);
        assert_eq!(
            refs[1].storage_key.as_deref(),
            Some("/workspace/attachments/2026-06-09/msg1-2-diagram.png")
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
            Some("/workspace/attachments/2026-06-09/msg1-1-attachment.png")
        );
        assert!(refs[0].filename.is_none());
    }

    #[tokio::test]
    async fn later_item_failure_fails_the_batch_and_leaves_earlier_bytes_landed() {
        // Documents the batch boundary the rustdoc promises: the first failure
        // returns Err, and bytes already landed for earlier items are left in
        // place (dangling — no ref is returned for them).
        let backend = Arc::new(InMemoryBackend::new());
        let writer = project_mount(Arc::clone(&backend), MountPermissions::read_write());
        let scope = test_scope();

        // Force att-1's write to fail: seed a child under its computed path so
        // the backend rejects writing a file where a directory now exists.
        // att-1 is the 2nd attachment, so its 1-based index segment is `2`.
        let att1_path = "/workspace/attachments/2026-06-09/msg1-2-b.txt";
        writer
            .write_bytes(
                &scope,
                &ScopedPath::new(format!("{att1_path}/sentinel")).unwrap(),
                b"x".to_vec(),
            )
            .await
            .expect("seed a child so att-1's path is a directory");

        let err = land_inbound_attachments(
            &writer,
            &scope,
            DEFAULT_PROJECT_MOUNT_ALIAS,
            "2026-06-09",
            "msg1",
            vec![
                inbound("att-0", "text/plain", "a.txt", b"ok"),
                inbound("att-1", "text/plain", "b.txt", b"boom"),
            ],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect_err("a later landing failure fails the whole batch");
        assert!(matches!(err, AttachmentLandingError::Write(_)));

        // att-0 landed before att-1 failed: its bytes remain addressable even
        // though the batch returned no refs.
        let reader = project_mount(backend, MountPermissions::read_only());
        let landed = reader
            .get(
                &scope,
                &ScopedPath::new("/workspace/attachments/2026-06-09/msg1-1-a.txt").unwrap(),
            )
            .await
            .expect("read succeeds")
            .expect("att-0 bytes are still present");
        assert_eq!(landed.entry.body, b"ok");
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
}

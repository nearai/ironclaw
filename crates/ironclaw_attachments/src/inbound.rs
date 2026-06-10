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
/// (see [`land_attachment`]). On the first failure the whole batch returns the
/// error; partial writes that may already have landed are left in place (the
/// filesystem authority makes them addressable, and a retry re-lands at the
/// same deterministic paths).
///
/// [`ScopedPath`]: ironclaw_host_api::ScopedPath
pub async fn land_inbound_attachments<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    project_alias: &str,
    date: &str,
    message_id: &str,
    attachments: Vec<InboundAttachment>,
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
        let stored =
            land_attachment(filesystem, scope, project_alias, date, &landing, bytes).await?;
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

    use ironclaw_common::AttachmentKind;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
        ScopedPath, TenantId, UserId, VirtualPath,
    };

    use crate::landing::DEFAULT_PROJECT_MOUNT_ALIAS;

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
            Some("/workspace/attachments/2026-06-09/msg1-0-report.pdf")
        );
        assert!(refs[0].extracted_text.is_none());

        // `kind` is derived from the MIME type, not supplied by the caller.
        assert_eq!(refs[1].kind, AttachmentKind::Image);
        assert_eq!(
            refs[1].storage_key.as_deref(),
            Some("/workspace/attachments/2026-06-09/msg1-1-diagram.png")
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
        )
        .await
        .expect_err("a read-only project mount must reject the landing");
        assert!(matches!(err, AttachmentLandingError::Write(_)));
    }
}

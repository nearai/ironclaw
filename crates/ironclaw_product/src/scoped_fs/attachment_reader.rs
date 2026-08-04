//! Project-scoped inbound attachment *read-back* for product adapters.
//!
//! Named for what it holds. After the WS5 widening this module is the reader
//! and nothing else, so the path no longer says `attachment_landing` — a path
//! that would send discovery here looking for landing behavior that lives in
//! another crate.
//!
//! The write half — the [`InboundAttachmentLander`] port and its default
//! implementation — moved to `ironclaw_attachments` with that widening
//! (PROPOSAL §6.4.9). This reader could not follow it: besides
//! [`InboundAttachmentReader`] it also implements
//! [`LoopAttachmentReadPort`], and `ironclaw_loop_host` is a `loops`-layer
//! crate a `substrates` crate may not depend on — and once the struct moved,
//! that impl would have neither side in this crate either. It stays here, where
//! both traits are nameable.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_attachments::{DEFAULT_MAX_ATTACHMENT_BYTES, InboundAttachmentReader};
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{path::ScopedPath, resource::ResourceScope};
use ironclaw_loop_host::{LoopAttachmentReadError, LoopAttachmentReadPort};
use ironclaw_product_contracts::surface::{
    ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
};
use ironclaw_threads::ThreadScope;

/// a corrupt/oversized key can't materialize unbounded bytes.
pub struct ProjectScopedAttachmentReader<F: RootFilesystem> {
    filesystem: Arc<ScopedFilesystem<F>>,
    max_bytes: usize,
}

impl<F: RootFilesystem> ProjectScopedAttachmentReader<F> {
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            max_bytes: DEFAULT_MAX_ATTACHMENT_BYTES,
        }
    }

    /// Construct a reader with an explicit read ceiling. Test-only: production
    /// always uses [`DEFAULT_MAX_ATTACHMENT_BYTES`] via [`Self::new`], but the
    /// oversized branch is only reachable in a test with a tiny ceiling.
    #[cfg(test)]
    fn with_max_bytes(filesystem: Arc<ScopedFilesystem<F>>, max_bytes: usize) -> Self {
        Self {
            filesystem,
            max_bytes,
        }
    }
}

#[async_trait]
impl<F: RootFilesystem> LoopAttachmentReadPort for ProjectScopedAttachmentReader<F> {
    async fn read_attachment_bytes(
        &self,
        scope: &ResourceScope,
        storage_key: &str,
    ) -> Result<Vec<u8>, LoopAttachmentReadError> {
        let path = ScopedPath::new(storage_key.to_string())
            .map_err(|error| LoopAttachmentReadError::Backend(error.to_string()))?;
        match self
            .filesystem
            .read_bytes_bounded(scope, &path, self.max_bytes)
            .await
        {
            Ok(Some(bytes)) => Ok(bytes),
            // `read_bytes_bounded` returns `Ok(None)` only when the file is
            // larger than `max_bytes` — an oversized attachment we refuse to
            // materialize, not a missing one.
            Ok(None) => Err(LoopAttachmentReadError::Backend(format!(
                "attachment exceeds the {}-byte read limit",
                self.max_bytes
            ))),
            Err(FilesystemError::NotFound { .. }) => Err(LoopAttachmentReadError::NotFound),
            Err(FilesystemError::PermissionDenied { .. }) => {
                Err(LoopAttachmentReadError::Forbidden)
            }
            Err(error) => Err(LoopAttachmentReadError::Backend(error.to_string())),
        }
    }
}

/// Read counterpart wired into the product surface so the bytes endpoint can serve
/// image thumbnails. It reuses the loop read port — the same bounded,
/// `MountView`-re-scoped read — and translates the scope and error taxonomy to
/// the product API surface. A missing/oversized/forbidden read becomes a sanitized
/// product error rather than leaking a host path or backend string.
#[async_trait]
impl<F: RootFilesystem> InboundAttachmentReader for ProjectScopedAttachmentReader<F> {
    async fn read(
        &self,
        thread_scope: &ThreadScope,
        storage_key: &str,
    ) -> Result<Vec<u8>, ProductSurfaceError> {
        let scope = thread_scope.to_resource_scope();
        self.read_attachment_bytes(&scope, storage_key)
            .await
            .map_err(|error| match error {
                LoopAttachmentReadError::NotFound => ProductSurfaceError {
                    code: ProductSurfaceErrorCode::NotFound,
                    kind: ProductSurfaceErrorKind::NotFound,
                    status_code: 404,
                    retryable: false,
                    field: None,
                    validation_code: None,
                },
                LoopAttachmentReadError::Forbidden => ProductSurfaceError {
                    code: ProductSurfaceErrorCode::Forbidden,
                    kind: ProductSurfaceErrorKind::ParticipantDenied,
                    status_code: 403,
                    retryable: false,
                    field: None,
                    validation_code: None,
                },
                // Carry the cause to the log (sanitized 500 on the wire) rather
                // than dropping it — see error-handling rule.
                LoopAttachmentReadError::Backend(reason) => {
                    ProductSurfaceError::internal_from(reason)
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_attachments::InboundAttachmentLander;
    use ironclaw_attachments::{ProjectScopedAttachmentLander, WORKSPACE_ALIAS};
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        attachment::InboundAttachment,
        ids::{AgentId, TenantId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };

    fn workspace_fs(permissions: MountPermissions) -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new(WORKSPACE_ALIAS).unwrap(),
            VirtualPath::new("/projects/workspace").unwrap(),
            permissions,
        )])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            view,
        ))
    }

    fn thread_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant-test").unwrap(),
            agent_id: AgentId::new("agent-test").unwrap(),
            project_id: None,
            owner_user_id: Some(UserId::new("user-test").unwrap()),
            mission_id: None,
        }
    }

    #[tokio::test]
    async fn reader_reads_back_landed_attachment_bytes() {
        // The reader is the producer side of the image-vision path: it must read
        // back exactly what the lander wrote under the same workspace mount.
        let fs = workspace_fs(MountPermissions::read_write());
        let lander = ProjectScopedAttachmentLander::new(Arc::clone(&fs));
        let refs = lander
            .land(
                &thread_scope(),
                "msg1",
                vec![InboundAttachment {
                    id: "att-0".to_string(),
                    mime_type: "image/png".to_string(),
                    filename: Some("diagram.png".to_string()),
                    bytes: vec![1, 2, 3, 4],
                }],
            )
            .await
            .expect("landing succeeds through a read-write workspace mount");
        let storage_key = refs[0].storage_key.as_deref().expect("storage_key set");

        let reader = ProjectScopedAttachmentReader::new(Arc::clone(&fs));
        let bytes = reader
            .read_attachment_bytes(&thread_scope().to_resource_scope(), storage_key)
            .await
            .expect("reading back the landed attachment succeeds");
        assert_eq!(bytes, vec![1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn reader_missing_attachment_maps_to_not_found() {
        let reader =
            ProjectScopedAttachmentReader::new(workspace_fs(MountPermissions::read_write()));
        let err = reader
            .read_attachment_bytes(
                &thread_scope().to_resource_scope(),
                "/workspace/attachments/2026-06-14/m1-0-missing.png",
            )
            .await
            .expect_err("an absent attachment is a not-found, not bytes");
        assert!(matches!(err, LoopAttachmentReadError::NotFound));
    }

    #[tokio::test]
    async fn reader_oversized_attachment_is_a_backend_refusal_not_not_found() {
        let fs = workspace_fs(MountPermissions::read_write());
        let lander = ProjectScopedAttachmentLander::new(Arc::clone(&fs));
        let refs = lander
            .land(
                &thread_scope(),
                "msg1",
                vec![InboundAttachment {
                    id: "att-0".to_string(),
                    mime_type: "image/png".to_string(),
                    filename: Some("diagram.png".to_string()),
                    bytes: vec![1, 2, 3, 4],
                }],
            )
            .await
            .expect("landing succeeds through a read-write workspace mount");
        let storage_key = refs[0].storage_key.as_deref().expect("storage_key set");

        // A 2-byte ceiling rejects the 4-byte attachment. The reader must not
        // mislabel an oversized file as `NotFound`.
        let reader = ProjectScopedAttachmentReader::with_max_bytes(Arc::clone(&fs), 2);
        let err = reader
            .read_attachment_bytes(&thread_scope().to_resource_scope(), storage_key)
            .await
            .expect_err("an oversized attachment is refused");
        match err {
            LoopAttachmentReadError::Backend(reason) => assert!(reason.contains("exceeds")),
            other => panic!("expected a backend refusal, got {other}"),
        }
    }

    // The `InboundAttachmentReader` half — the product-surface translation —
    // had no coverage at all: every test above drives `read_attachment_bytes`
    // (the loop port), so the whole `map_err` taxonomy below it was dead to the
    // suite. These four pin it, because the taxonomy IS the contract for the
    // bytes endpoint: a caller distinguishes "gone" from "not yours" from
    // "broken" only by the status this closure picks.
    //
    // Crate tier rather than `tests/integration/`: the int tier reaches the
    // success path already (`tests/integration/attach.rs` drives the production
    // wiring), but the three failure arms need a mount that denies reads and a
    // storage key that fails `ScopedPath` validation — both are properties of
    // this adapter's construction, not of a wired runtime.

    #[tokio::test]
    async fn product_reader_serves_landed_bytes_through_the_thread_scope() {
        let fs = workspace_fs(MountPermissions::read_write());
        let lander = ProjectScopedAttachmentLander::new(Arc::clone(&fs));
        let refs = lander
            .land(
                &thread_scope(),
                "msg1",
                vec![InboundAttachment {
                    id: "att-0".to_string(),
                    mime_type: "image/png".to_string(),
                    filename: Some("diagram.png".to_string()),
                    bytes: vec![9, 8, 7],
                }],
            )
            .await
            .expect("landing succeeds");
        let storage_key = refs[0].storage_key.as_deref().expect("storage_key set");

        let reader = ProjectScopedAttachmentReader::new(Arc::clone(&fs));
        let bytes = InboundAttachmentReader::read(&reader, &thread_scope(), storage_key)
            .await
            .expect("the product reader serves the landed bytes");
        assert_eq!(bytes, vec![9, 8, 7]);
    }

    #[tokio::test]
    async fn product_reader_maps_a_missing_attachment_to_404_not_found() {
        let reader =
            ProjectScopedAttachmentReader::new(workspace_fs(MountPermissions::read_write()));
        let error = InboundAttachmentReader::read(
            &reader,
            &thread_scope(),
            "/workspace/attachments/2026-06-14/m1-0-missing.png",
        )
        .await
        .expect_err("an absent attachment is an error, not empty bytes");
        assert_eq!(error.status_code, 404);
        assert_eq!(error.code, ProductSurfaceErrorCode::NotFound);
        assert_eq!(error.kind, ProductSurfaceErrorKind::NotFound);
        assert!(
            !error.retryable,
            "a missing attachment never becomes present"
        );
    }

    #[tokio::test]
    async fn product_reader_maps_a_denied_mount_to_403_rather_than_404() {
        // The distinction matters: answering 404 for a readable-but-forbidden
        // attachment would tell a caller it does not exist.
        let reader = ProjectScopedAttachmentReader::new(workspace_fs(MountPermissions::none()));
        let error = InboundAttachmentReader::read(
            &reader,
            &thread_scope(),
            "/workspace/attachments/2026-06-14/m1-0-diagram.png",
        )
        .await
        .expect_err("a mount that grants no read must not serve bytes");
        assert_eq!(error.status_code, 403);
        assert_eq!(error.code, ProductSurfaceErrorCode::Forbidden);
        assert_eq!(error.kind, ProductSurfaceErrorKind::ParticipantDenied);
    }

    #[tokio::test]
    async fn product_reader_maps_a_malformed_storage_key_to_a_sanitized_500() {
        // `storage_key` reaches here from a stored attachment ref, so a
        // corrupted one must fail closed at `ScopedPath::new` rather than be
        // handed to the filesystem — and it must classify as Internal, not as
        // NotFound/Forbidden, which would tell the caller something false about
        // an attachment that may well exist.
        //
        // Note on what is NOT asserted: that the rejected value is absent from
        // the error. `ProductSurfaceError` has no reason-bearing field and
        // `internal_from` logs its source and discards it, so any such check
        // could never fail — the non-leak is structural, not behavioral, and an
        // assertion for it would be vacuous. Equality against `internal()`
        // pins the whole sanitized shape instead, so a future variant that
        // *did* add a detail field would fail here.
        let reader =
            ProjectScopedAttachmentReader::new(workspace_fs(MountPermissions::read_write()));
        let error = InboundAttachmentReader::read(
            &reader,
            &thread_scope(),
            "https://evil.example.com/attachment.png",
        )
        .await
        .expect_err("a URL is not a scoped path");
        assert_eq!(error, ProductSurfaceError::internal());
        assert_eq!(error.status_code, 500);
    }
}

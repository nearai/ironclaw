//! Project-scoped inbound attachment landing for the WebUI v2 facade.
//!
//! Implements the [`InboundAttachmentLander`] port the facade calls before
//! accepting a user message: it writes attachment bytes through the
//! project-scoped workspace [`ScopedFilesystem`] — the same filesystem
//! authority the agent's file tools resolve through — and returns the
//! transcript references to persist. Going through that one authority is what
//! makes a landed attachment readable by `file_read`/`list_dir` at the recorded
//! `storage_key` in this and later turns.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_attachments::{
    DEFAULT_MAX_ATTACHMENT_BYTES, InboundAttachment, land_inbound_attachments,
};
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind, ResourceScope,
    ScopedPath,
};
use ironclaw_loop_host::{LoopAttachmentReadError, LoopAttachmentReadPort};
use ironclaw_product::{InboundAttachmentLander, InboundAttachmentReader};
use ironclaw_threads::{AttachmentRef, ThreadScope};

use crate::local_dev_mounts::WORKSPACE_ALIAS;

/// Lands inbound attachments through a project-scoped workspace filesystem.
pub(crate) struct ProjectScopedAttachmentLander<F: RootFilesystem> {
    filesystem: Arc<ScopedFilesystem<F>>,
    project_alias: String,
    /// Per-attachment size ceiling passed to the landing routine. The
    /// `send_message` route's 14 MiB body cap is the primary gate; this is
    /// defense in depth so a single attachment can never land unbounded bytes.
    max_attachment_bytes: usize,
}

impl<F: RootFilesystem> ProjectScopedAttachmentLander<F> {
    pub(crate) fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            project_alias: WORKSPACE_ALIAS.to_string(),
            max_attachment_bytes: DEFAULT_MAX_ATTACHMENT_BYTES,
        }
    }
}

/// Reads landed attachment bytes back through the same project-scoped workspace
/// filesystem, so the loop model port can build multimodal image parts for a
/// vision-capable model. The read re-scopes `storage_key` through the
/// `MountView` authority (it is never treated as a host path), and is bounded so
/// a corrupt/oversized key can't materialize unbounded bytes.
pub(crate) struct ProjectScopedAttachmentReader<F: RootFilesystem> {
    filesystem: Arc<ScopedFilesystem<F>>,
    max_bytes: usize,
}

impl<F: RootFilesystem> ProjectScopedAttachmentReader<F> {
    pub(crate) fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
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

#[async_trait]
impl<F: RootFilesystem> InboundAttachmentLander for ProjectScopedAttachmentLander<F> {
    async fn land(
        &self,
        thread_scope: &ThreadScope,
        message_id: &str,
        attachments: Vec<InboundAttachment>,
    ) -> Result<Vec<AttachmentRef>, ProductSurfaceError> {
        let scope = thread_scope.to_resource_scope();
        // Partition by UTC date so a project's attachments directory stays
        // browsable; the rest of the path (message id + index + filename) makes
        // each attachment uniquely addressable.
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        land_inbound_attachments(
            self.filesystem.as_ref(),
            &scope,
            &self.project_alias,
            &date,
            message_id,
            attachments,
            self.max_attachment_bytes,
        )
        .await
        // The user-facing error stays a sanitized 500; `internal_from` logs the
        // underlying landing failure (invalid mount path vs. write/permission
        // denied) so an operator can tell a misconfigured mount from a full disk.
        .map_err(|error| {
            ProductSurfaceError::internal_from(format!(
                "land inbound attachments for message {message_id}: {error}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::ProductSurfaceErrorCode;
    use ironclaw_host_api::{
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, TenantId, UserId, VirtualPath,
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
    async fn lands_attachment_and_returns_ref_with_storage_key() {
        let lander =
            ProjectScopedAttachmentLander::new(workspace_fs(MountPermissions::read_write()));
        let refs = lander
            .land(
                &thread_scope(),
                "msg1",
                vec![InboundAttachment {
                    id: "att-0".to_string(),
                    mime_type: "application/pdf".to_string(),
                    filename: Some("report.pdf".to_string()),
                    bytes: b"%PDF-1.7".to_vec(),
                }],
            )
            .await
            .expect("landing succeeds through a read-write workspace mount");
        assert_eq!(refs.len(), 1);
        let storage_key = refs[0].storage_key.as_deref().expect("storage_key set");
        assert!(
            storage_key.starts_with("/workspace/attachments/")
                && storage_key.ends_with("-report.pdf"),
            "unexpected storage key: {storage_key}"
        );
    }

    #[tokio::test]
    async fn read_only_workspace_mount_maps_to_internal_error() {
        let lander =
            ProjectScopedAttachmentLander::new(workspace_fs(MountPermissions::read_only()));
        let err = lander
            .land(
                &thread_scope(),
                "msg1",
                vec![InboundAttachment {
                    id: "att-0".to_string(),
                    mime_type: "application/pdf".to_string(),
                    filename: Some("report.pdf".to_string()),
                    bytes: b"%PDF".to_vec(),
                }],
            )
            .await
            .expect_err("read-only workspace mount must fail closed");
        assert_eq!(err.code, ProductSurfaceErrorCode::Internal);
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
}

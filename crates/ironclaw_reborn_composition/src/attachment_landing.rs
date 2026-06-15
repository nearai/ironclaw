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
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_product_workflow::{InboundAttachmentLander, RebornServicesError};
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

#[async_trait]
impl<F: RootFilesystem> InboundAttachmentLander for ProjectScopedAttachmentLander<F> {
    async fn land(
        &self,
        thread_scope: &ThreadScope,
        message_id: &str,
        attachments: Vec<InboundAttachment>,
    ) -> Result<Vec<AttachmentRef>, RebornServicesError> {
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
            RebornServicesError::internal_from(format!(
                "land inbound attachments for message {message_id}: {error}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, TenantId, UserId, VirtualPath,
    };
    use ironclaw_product_workflow::RebornServicesErrorCode;

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
        assert_eq!(err.code, RebornServicesErrorCode::Internal);
    }
}

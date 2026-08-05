//! The default [`InboundAttachmentLander`] over a project-scoped workspace.
//!
//! It writes attachment bytes through the project-scoped [`ScopedFilesystem`] —
//! the same filesystem authority the agent's file tools resolve through — and
//! returns the transcript references to persist. Going through that one
//! authority is what makes a landed attachment readable by `file_read` /
//! `list_dir` at the recorded `storage_key` in this and later turns.
//!
//! The read counterpart (`ProjectScopedAttachmentReader`) stays in
//! `ironclaw_assistant`: it also implements `ironclaw_loop_host`'s
//! `LoopAttachmentReadPort`, and `ironclaw_loop_host` is a `loops`-layer crate
//! this substrate may not name. See PROPOSAL §6.4.9 and the WS5 `attachments`
//! CHECKLIST row.
//!
//! [`ScopedFilesystem`]: ironclaw_filesystem::ScopedFilesystem

use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use ironclaw_common::AttachmentRef;
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{attachment::InboundAttachment, path::ScopedPath};
use ironclaw_product_contracts::surface::ProductSurfaceError;
use ironclaw_threads::ThreadScope;

use crate::{
    AttachmentCleanupReport, DEFAULT_MAX_ATTACHMENT_BYTES, InboundAttachmentLander,
    WORKSPACE_ALIAS, land_inbound_attachments,
};

const STALE_RECONCILIATION_DAYS: i64 = 1;
const STALE_CLEANUP_MAX_DATE_DIRS: usize = 32;
const STALE_CLEANUP_MAX_BATCHES_PER_DATE: usize = 64;
const STALE_CLEANUP_MAX_DELETES: usize = 64;

/// Lands inbound attachments through a project-scoped workspace filesystem.
pub struct ProjectScopedAttachmentLander<F: RootFilesystem> {
    filesystem: Arc<ScopedFilesystem<F>>,
    project_alias: String,
    /// Per-attachment size ceiling passed to the landing routine. The
    /// `send_message` route's 14 MiB body cap is the primary gate; this is
    /// defense in depth so a single attachment can never land unbounded bytes.
    max_attachment_bytes: usize,
}

impl<F: RootFilesystem> ProjectScopedAttachmentLander<F> {
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
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

    async fn rollback(
        &self,
        thread_scope: &ThreadScope,
        attachments: &[AttachmentRef],
    ) -> Result<(), ProductSurfaceError> {
        let Some(first_storage_key) = attachments
            .first()
            .and_then(|attachment| attachment.storage_key.as_deref())
        else {
            return Ok(());
        };
        let batch_path = attachment_batch_parent(first_storage_key)?;
        for attachment in attachments.iter().skip(1) {
            let storage_key = attachment.storage_key.as_deref().ok_or_else(|| {
                ProductSurfaceError::internal_from(
                    "landed attachment rollback reference has no storage key",
                )
            })?;
            if attachment_batch_parent(storage_key)? != batch_path {
                return Err(ProductSurfaceError::internal_from(
                    "landed attachment rollback references span multiple batches",
                ));
            }
        }

        let scope = thread_scope.to_resource_scope();
        match self.filesystem.delete(&scope, &batch_path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(ProductSurfaceError::internal_from(format!(
                "roll back inbound attachment batch at {}: {error}",
                batch_path.as_str()
            ))),
        }
    }

    async fn cleanup_stale(
        &self,
        thread_scope: &ThreadScope,
        referenced_storage_keys: &[String],
    ) -> Result<AttachmentCleanupReport, ProductSurfaceError> {
        // The caller supplies a complete authoritative snapshot. An empty
        // snapshot cannot distinguish "nothing is referenced" from a failed
        // or partial scan, so cleanup fails closed and deletes nothing.
        if referenced_storage_keys.is_empty() {
            return Ok(AttachmentCleanupReport::default());
        }
        let attachment_root = workspace_attachment_root()?;
        let attachment_prefix = format!("{}/", attachment_root.as_str());
        let mut referenced_batches = HashSet::new();
        for storage_key in referenced_storage_keys {
            // Thread history also contains structured outbound attachments,
            // whose storage keys legitimately point at agent-created workspace
            // files rather than the inbound landing area. They are outside
            // this reconciler's ownership and must not make an otherwise
            // exhaustive inbound snapshot fail.
            let Some(relative) = storage_key.strip_prefix(&attachment_prefix) else {
                continue;
            };
            // Legacy flat keys are `<date>/<file>` and cannot name a message
            // directory. New keys are exactly `<date>/<message>/<file>`.
            if relative.split('/').count() != 3 {
                return Err(ProductSurfaceError::internal_from(
                    "stale attachment cleanup received an invalid reference shape",
                ));
            }
            referenced_batches.insert(attachment_batch_parent(storage_key)?.as_str().to_string());
        }
        // Preserve the same fail-closed behavior as an empty snapshot when the
        // scan contains only attachment domains this reconciler does not own.
        if referenced_batches.is_empty() {
            return Ok(AttachmentCleanupReport::default());
        }

        let scope = thread_scope.to_resource_scope();
        let date_dirs = match self
            .filesystem
            .list_dir_bounded(&scope, &attachment_root, STALE_CLEANUP_MAX_DATE_DIRS)
            .await
        {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => {
                return Ok(AttachmentCleanupReport::default());
            }
            Err(error) => {
                return Err(ProductSurfaceError::internal_from(format!(
                    "list inbound attachment dates for stale cleanup: {error}"
                )));
            }
        };
        let cutoff =
            chrono::Utc::now().date_naive() - chrono::Duration::days(STALE_RECONCILIATION_DAYS);
        let mut report = AttachmentCleanupReport::default();

        for date_entry in date_dirs {
            if date_entry.file_type != FileType::Directory {
                continue;
            }
            let Ok(date) = chrono::NaiveDate::parse_from_str(&date_entry.name, "%Y-%m-%d") else {
                continue;
            };
            if date >= cutoff {
                continue;
            }
            let date_path =
                ScopedPath::new(format!("{}/{}", attachment_root.as_str(), date_entry.name))
                    .map_err(ProductSurfaceError::internal_from)?;
            let batches = match self
                .filesystem
                .list_dir_bounded(&scope, &date_path, STALE_CLEANUP_MAX_BATCHES_PER_DATE)
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. }) => continue,
                Err(error) => {
                    return Err(ProductSurfaceError::internal_from(format!(
                        "list inbound attachment batches for stale cleanup: {error}"
                    )));
                }
            };
            for batch in batches {
                if batch.file_type != FileType::Directory
                    || report.deleted_batches >= STALE_CLEANUP_MAX_DELETES
                {
                    continue;
                }
                report.scanned_batches += 1;
                let batch_path = ScopedPath::new(format!(
                    "{}/{}/{}",
                    attachment_root.as_str(),
                    date_entry.name,
                    batch.name
                ))
                .map_err(ProductSurfaceError::internal_from)?;
                if referenced_batches.contains(batch_path.as_str()) {
                    continue;
                }
                match self.filesystem.delete(&scope, &batch_path).await {
                    Ok(()) => report.deleted_batches += 1,
                    Err(FilesystemError::NotFound { .. }) => {}
                    Err(error) => {
                        return Err(ProductSurfaceError::internal_from(format!(
                            "delete stale inbound attachment batch at {}: {error}",
                            batch_path.as_str()
                        )));
                    }
                }
            }
        }
        Ok(report)
    }
}

fn attachment_batch_parent(storage_key: &str) -> Result<ScopedPath, ProductSurfaceError> {
    let (parent, filename) = storage_key.rsplit_once('/').ok_or_else(|| {
        ProductSurfaceError::internal_from("landed attachment storage key has no batch parent")
    })?;
    let attachment_prefix = format!("{}/", workspace_attachment_root()?.as_str());
    let relative = parent.strip_prefix(&attachment_prefix).ok_or_else(|| {
        ProductSurfaceError::internal_from(
            "landed attachment rollback path is outside the attachment root",
        )
    })?;
    if filename.is_empty()
        || relative.split('/').count() != 2
        || relative.split('/').any(str::is_empty)
    {
        return Err(ProductSurfaceError::internal_from(
            "landed attachment rollback path has an invalid batch shape",
        ));
    }
    ScopedPath::new(parent.to_string()).map_err(ProductSurfaceError::internal_from)
}

fn workspace_attachment_root() -> Result<ScopedPath, ProductSurfaceError> {
    ScopedPath::new(format!("/{WORKSPACE_ALIAS}/attachments"))
        .map_err(ProductSurfaceError::internal_from)
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_common::AttachmentKind;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        ids::{AgentId, TenantId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };
    use ironclaw_product_contracts::surface::ProductSurfaceErrorCode;

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
    async fn rollback_removes_the_complete_landed_batch() {
        let fs = workspace_fs(MountPermissions::read_write_list_delete());
        let lander = ProjectScopedAttachmentLander::new(Arc::clone(&fs));
        let scope = thread_scope();
        let refs = lander
            .land(
                &scope,
                "rollback-message",
                vec![
                    InboundAttachment {
                        id: "first".to_string(),
                        mime_type: "text/plain".to_string(),
                        filename: Some("first.txt".to_string()),
                        bytes: b"first".to_vec(),
                    },
                    InboundAttachment {
                        id: "second".to_string(),
                        mime_type: "text/plain".to_string(),
                        filename: Some("second.txt".to_string()),
                        bytes: b"second".to_vec(),
                    },
                ],
            )
            .await
            .expect("batch lands");
        let first_path =
            ScopedPath::new(refs[0].storage_key.clone().expect("storage key")).unwrap();

        lander
            .rollback(&scope, &refs)
            .await
            .expect("rollback deletes the batch");

        assert!(matches!(
            fs.stat(&scope.to_resource_scope(), &first_path).await,
            Err(FilesystemError::NotFound { .. })
        ));
    }

    /// Rollback deletes a whole batch directory, so it must refuse any
    /// reference it cannot prove names one — otherwise a malformed
    /// `storage_key` picks the delete target. Each row is a distinct
    /// rejection: the first five are the guards in `attachment_batch_parent`
    /// (no parent separator, outside the attachment root, then the three
    /// batch-shape conditions), the last is the loop in `rollback` itself
    /// meeting a later reference that was never landed.
    ///
    /// Driven through `rollback` rather than the helper directly, because
    /// `rollback` is what decides whether a delete happens. `internal_from`
    /// deliberately collapses every case to a sanitized `Internal` code, so
    /// the input is what distinguishes them — and the assertion that matters
    /// is that none of them reaches the delete.
    #[tokio::test]
    async fn rollback_refuses_malformed_batch_references() {
        let fs = workspace_fs(MountPermissions::read_write_list_delete());
        let lander = ProjectScopedAttachmentLander::new(Arc::clone(&fs));
        let scope = thread_scope();

        // Land a real batch first: if a malformed reference ever fell through
        // to the delete, this is what it would destroy.
        let landed = lander
            .land(
                &scope,
                "keep-me",
                vec![InboundAttachment {
                    id: "keep".to_string(),
                    mime_type: "text/plain".to_string(),
                    filename: Some("keep.txt".to_string()),
                    bytes: b"keep".to_vec(),
                }],
            )
            .await
            .expect("batch lands");
        let landed_path =
            ScopedPath::new(landed[0].storage_key.clone().expect("storage key")).unwrap();

        let root = workspace_attachment_root()
            .expect("attachment root")
            .as_str()
            .to_string();
        let cases: Vec<(&str, Vec<Option<String>>)> = vec![
            (
                "storage key has no batch parent",
                vec![Some("no-separator".to_string())],
            ),
            (
                "batch parent is outside the attachment root",
                vec![Some("/elsewhere/2026-01-01/msg/f.txt".to_string())],
            ),
            (
                "batch shape: empty filename",
                vec![Some(format!("{root}/2026-01-01/msg/"))],
            ),
            (
                "batch shape: wrong segment count",
                vec![Some(format!("{root}/only-one/f.txt"))],
            ),
            (
                "batch shape: empty segment",
                vec![Some(format!("{root}/2026-01-01//f.txt"))],
            ),
            (
                "a later reference carries no storage key",
                vec![Some(format!("{root}/2026-01-01/msg/first.txt")), None],
            ),
        ];

        for (case, keys) in cases {
            let refs: Vec<AttachmentRef> = keys
                .into_iter()
                .enumerate()
                .map(|(index, storage_key)| AttachmentRef {
                    id: format!("att-{index}"),
                    kind: AttachmentKind::Document,
                    mime_type: "text/plain".to_string(),
                    filename: Some("f.txt".to_string()),
                    size_bytes: None,
                    storage_key,
                    extracted_text: None,
                })
                .collect();
            let err = lander.rollback(&scope, &refs).await.expect_err(case);
            assert_eq!(err.code, ProductSurfaceErrorCode::Internal, "{case}");
        }

        // Nothing was deleted along the way.
        assert!(
            fs.stat(&scope.to_resource_scope(), &landed_path)
                .await
                .is_ok(),
            "a refused rollback must not delete an unrelated landed batch"
        );
    }

    #[tokio::test]
    async fn stale_cleanup_deletes_only_old_unreferenced_message_batches() {
        let fs = workspace_fs(MountPermissions::read_write_list_delete());
        let scope = thread_scope();
        let resource_scope = scope.to_resource_scope();
        let old_date = (chrono::Utc::now() - chrono::Duration::days(3))
            .format("%Y-%m-%d")
            .to_string();
        let orphan = land_inbound_attachments(
            fs.as_ref(),
            &resource_scope,
            WORKSPACE_ALIAS,
            &old_date,
            "orphan-message",
            vec![InboundAttachment {
                id: "orphan".to_string(),
                mime_type: "text/plain".to_string(),
                filename: Some("orphan.txt".to_string()),
                bytes: b"orphan".to_vec(),
            }],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("seed orphan batch");
        let protected = land_inbound_attachments(
            fs.as_ref(),
            &resource_scope,
            WORKSPACE_ALIAS,
            &old_date,
            "referenced-message",
            vec![InboundAttachment {
                id: "protected".to_string(),
                mime_type: "text/plain".to_string(),
                filename: Some("protected.txt".to_string()),
                bytes: b"protected".to_vec(),
            }],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("seed referenced batch");
        let protected_keys = protected
            .iter()
            .filter_map(|attachment| attachment.storage_key.clone())
            .collect::<Vec<_>>();
        let lander = ProjectScopedAttachmentLander::new(Arc::clone(&fs));

        let report = lander
            .cleanup_stale(&scope, &protected_keys)
            .await
            .expect("stale cleanup succeeds");

        assert_eq!(report.deleted_batches, 1);
        let orphan_key = orphan[0].storage_key.as_deref().expect("orphan key");
        assert!(matches!(
            fs.stat(
                &resource_scope,
                &ScopedPath::new(orphan_key.to_string()).unwrap()
            )
            .await,
            Err(FilesystemError::NotFound { .. })
        ));
        let protected_key = protected[0].storage_key.as_deref().expect("protected key");
        assert!(
            fs.stat(
                &resource_scope,
                &ScopedPath::new(protected_key.to_string()).unwrap()
            )
            .await
            .is_ok()
        );
    }

    /// All three of `cleanup_stale`'s pre-scan exits, in one fixture, because
    /// they share one guarantee: a snapshot this pass cannot trust deletes
    /// nothing. The three differ in how loud they are, and that difference is
    /// deliberate — an empty or wholly-unowned snapshot is a legitimate state
    /// and returns an empty report, whereas an in-root reference of the wrong
    /// depth is a malformed input this reconciler owns and aborts the pass
    /// loudly rather than reclaiming against a key it cannot parse.
    #[tokio::test]
    async fn stale_cleanup_with_empty_or_unowned_snapshot_deletes_nothing() {
        let fs = workspace_fs(MountPermissions::read_write_list_delete());
        let scope = thread_scope();
        let old_date = (chrono::Utc::now() - chrono::Duration::days(3))
            .format("%Y-%m-%d")
            .to_string();
        let refs = land_inbound_attachments(
            fs.as_ref(),
            &scope.to_resource_scope(),
            WORKSPACE_ALIAS,
            &old_date,
            "preserved-message",
            vec![InboundAttachment {
                id: "preserved".to_string(),
                mime_type: "text/plain".to_string(),
                filename: Some("preserved.txt".to_string()),
                bytes: b"preserved".to_vec(),
            }],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("seed batch");
        let path = ScopedPath::new(refs[0].storage_key.clone().expect("storage key")).unwrap();
        let lander = ProjectScopedAttachmentLander::new(Arc::clone(&fs));

        assert_eq!(
            lander
                .cleanup_stale(&scope, &[])
                .await
                .expect("empty snapshot fails closed"),
            AttachmentCleanupReport::default()
        );
        assert_eq!(
            lander
                .cleanup_stale(&scope, &["/workspace/not-attachments/file.txt".to_string()])
                .await
                .expect("unowned attachment references are ignored"),
            AttachmentCleanupReport::default()
        );
        assert!(
            fs.stat(&scope.to_resource_scope(), &path).await.is_ok(),
            "incomplete cleanup snapshots must not delete the batch"
        );

        // An in-root reference whose relative depth is not `<date>/<message>/<file>`
        // aborts the whole pass. One malformed key means the snapshot cannot be
        // read as authoritative, and reclaiming from a partially-understood
        // snapshot is how a live batch gets deleted.
        let error = lander
            .cleanup_stale(
                &scope,
                &["/workspace/attachments/2026-01-01/flat.txt".to_string()],
            )
            .await
            .expect_err("an in-root reference of the wrong depth must fail loudly");
        assert_eq!(error.code, ProductSurfaceErrorCode::Internal);
        assert!(
            fs.stat(&scope.to_resource_scope(), &path).await.is_ok(),
            "an aborted pass must not delete anything"
        );
    }

    #[tokio::test]
    async fn stale_cleanup_ignores_outbound_workspace_references_in_a_complete_snapshot() {
        let fs = workspace_fs(MountPermissions::read_write_list_delete());
        let scope = thread_scope();
        let resource_scope = scope.to_resource_scope();
        let old_date = (chrono::Utc::now() - chrono::Duration::days(3))
            .format("%Y-%m-%d")
            .to_string();
        let orphan = land_inbound_attachments(
            fs.as_ref(),
            &resource_scope,
            WORKSPACE_ALIAS,
            &old_date,
            "orphan-message",
            vec![InboundAttachment {
                id: "orphan".to_string(),
                mime_type: "text/plain".to_string(),
                filename: Some("orphan.txt".to_string()),
                bytes: b"orphan".to_vec(),
            }],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("seed orphan batch");
        let protected = land_inbound_attachments(
            fs.as_ref(),
            &resource_scope,
            WORKSPACE_ALIAS,
            &old_date,
            "referenced-message",
            vec![InboundAttachment {
                id: "protected".to_string(),
                mime_type: "text/plain".to_string(),
                filename: Some("protected.txt".to_string()),
                bytes: b"protected".to_vec(),
            }],
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("seed referenced batch");
        let protected_key = protected[0].storage_key.clone().expect("storage key");
        let lander = ProjectScopedAttachmentLander::new(Arc::clone(&fs));

        let report = lander
            .cleanup_stale(
                &scope,
                &[
                    "/workspace/agent-created-reply.txt".to_string(),
                    protected_key.clone(),
                ],
            )
            .await
            .expect("outbound references do not poison inbound reconciliation");

        assert_eq!(report.deleted_batches, 1);
        assert!(matches!(
            fs.stat(
                &resource_scope,
                &ScopedPath::new(orphan[0].storage_key.clone().expect("orphan storage key"))
                    .unwrap()
            )
            .await,
            Err(FilesystemError::NotFound { .. })
        ));
        assert!(
            fs.stat(&resource_scope, &ScopedPath::new(protected_key).unwrap())
                .await
                .is_ok()
        );
    }
}

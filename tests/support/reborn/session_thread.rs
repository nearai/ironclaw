use std::sync::Arc;

use ironclaw_filesystem::{LocalFilesystem, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ThreadId, VirtualPath,
};
use ironclaw_threads::{
    FilesystemSessionThreadService, SessionThreadService, ThreadHistoryRequest,
    ThreadMessageRecord, ThreadScope,
};
use thiserror::Error;

use super::filesystem::local_filesystem;

#[derive(Debug, Error)]
pub enum RebornThreadHarnessError {
    #[error("failed to create thread harness tempdir: {0}")]
    Tempdir(#[from] std::io::Error),
    #[error("failed to configure local filesystem: {0}")]
    Filesystem(#[from] ironclaw_filesystem::FilesystemError),
    #[error("invalid mount view: {0}")]
    MountView(#[from] ironclaw_host_api::HostApiError),
    #[error("thread service failed: {0}")]
    Thread(#[from] ironclaw_threads::SessionThreadError),
    #[error("thread history does not contain final assistant reply containing {0:?}")]
    MissingFinalReply(String),
}

pub struct RebornThreadHarness {
    pub scope: ThreadScope,
    pub service: Arc<FilesystemSessionThreadService<LocalFilesystem>>,
    backend: Arc<LocalFilesystem>,
    root: Arc<tempfile::TempDir>,
}

impl RebornThreadHarness {
    pub fn filesystem_temp(scope: ThreadScope) -> Result<Self, RebornThreadHarnessError> {
        let root = Arc::new(tempfile::tempdir()?);
        let backend = Arc::new(local_filesystem(root.path())?);
        Self::filesystem_shared_backend(scope, backend, root)
    }

    pub fn filesystem_shared_backend(
        scope: ThreadScope,
        backend: Arc<LocalFilesystem>,
        root: Arc<tempfile::TempDir>,
    ) -> Result<Self, RebornThreadHarnessError> {
        let scoped = scoped_threads_fs_at(Arc::clone(&backend), &scope)?;
        let service = Arc::new(FilesystemSessionThreadService::new(scoped));
        Ok(Self {
            scope,
            service,
            backend,
            root,
        })
    }

    pub fn reopened(&self) -> Result<Self, RebornThreadHarnessError> {
        Self::filesystem_shared_backend(
            self.scope.clone(),
            Arc::clone(&self.backend),
            Arc::clone(&self.root),
        )
    }

    pub fn service_instance(
        &self,
    ) -> Result<FilesystemSessionThreadService<LocalFilesystem>, RebornThreadHarnessError> {
        let scoped = scoped_threads_fs_at(Arc::clone(&self.backend), &self.scope)?;
        Ok(FilesystemSessionThreadService::new(scoped))
    }

    pub async fn history(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ThreadMessageRecord>, RebornThreadHarnessError> {
        Ok(self
            .service
            .list_thread_history(ThreadHistoryRequest {
                scope: self.scope.clone(),
                thread_id,
            })
            .await?
            .messages)
    }

    pub async fn assert_final_reply(
        &self,
        thread_id: ThreadId,
        text: &str,
    ) -> Result<(), RebornThreadHarnessError> {
        let history = self.history(thread_id).await?;
        let found = history
            .iter()
            .rev()
            .find(|message| {
                message.kind == ironclaw_threads::MessageKind::Assistant
                    && message.status == ironclaw_threads::MessageStatus::Finalized
            })
            .is_some_and(|message| {
                message
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains(text))
            });
        if found {
            Ok(())
        } else {
            Err(RebornThreadHarnessError::MissingFinalReply(
                text.to_string(),
            ))
        }
    }
}

fn scoped_threads_fs_at<F>(
    backend: Arc<F>,
    scope: &ThreadScope,
) -> Result<Arc<ScopedFilesystem<F>>, ironclaw_host_api::HostApiError>
where
    F: RootFilesystem,
{
    let user_id = scope
        .owner_user_id
        .as_ref()
        .map_or("_system", |user_id| user_id.as_str());
    let target = format!(
        "/engine/tenants/{}/users/{}/threads",
        scope.tenant_id, user_id
    );
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads").expect("valid threads alias"),
        VirtualPath::new(target).expect("valid threads target"),
        MountPermissions::read_write_list_delete(),
    )])?;
    Ok(Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts)))
}

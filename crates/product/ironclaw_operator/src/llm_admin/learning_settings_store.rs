//! Deployment-wide filesystem persistence for learning settings.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{path::ScopedPath, resource::ResourceScope};
use ironclaw_product_contracts::operator_llm::{
    LearningSettings, LearningSettingsStore, LearningSettingsStoreError,
};

const SETTINGS_PATH: &str = "/tenant-shared/llm-learning.json";
const SETTINGS_MAX_BYTES: usize = 4 * 1024;

/// Filesystem-backed deployment setting. The scope is fixed at construction so
/// every authenticated caller in the deployment reads the same record rather
/// than deriving storage from the request caller.
pub struct FilesystemLearningSettingsStore<F: RootFilesystem + ?Sized> {
    filesystem: Arc<ScopedFilesystem<F>>,
    scope: ResourceScope,
}

impl<F: RootFilesystem + ?Sized> FilesystemLearningSettingsStore<F> {
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>, scope: ResourceScope) -> Self {
        Self { filesystem, scope }
    }

    fn path() -> Result<ScopedPath, LearningSettingsStoreError> {
        ScopedPath::new(SETTINGS_PATH).map_err(|_| LearningSettingsStoreError::InvalidData)
    }
}

#[async_trait]
impl<F: RootFilesystem + ?Sized> LearningSettingsStore for FilesystemLearningSettingsStore<F> {
    async fn read(&self) -> Result<Option<LearningSettings>, LearningSettingsStoreError> {
        let path = Self::path()?;
        let bytes = match self
            .filesystem
            .read_bytes_bounded(&self.scope, &path, SETTINGS_MAX_BYTES)
            .await
        {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Err(LearningSettingsStoreError::InvalidData),
            Err(FilesystemError::NotFound { .. }) => return Ok(None),
            Err(error) => {
                tracing::error!(error = %error, "learning settings read failed");
                return Err(LearningSettingsStoreError::Unavailable);
            }
        };
        serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            tracing::error!(error = %error, "learning settings record is invalid");
            LearningSettingsStoreError::InvalidData
        })
    }

    async fn write(&self, settings: &LearningSettings) -> Result<(), LearningSettingsStoreError> {
        let bytes = serde_json::to_vec(settings).map_err(|error| {
            tracing::error!(error = %error, "learning settings serialization failed");
            LearningSettingsStoreError::InvalidData
        })?;
        if bytes.len() > SETTINGS_MAX_BYTES {
            return Err(LearningSettingsStoreError::InvalidData);
        }
        let path = Self::path()?;
        self.filesystem
            .write_bytes(&self.scope, &path, bytes)
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "learning settings write failed");
                LearningSettingsStoreError::Unavailable
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        ids::{AgentId, InvocationId, ProjectId, TenantId, ThreadId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };

    fn scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("settings-tenant").expect("tenant"),
            user_id: UserId::new("settings-user").expect("user"),
            agent_id: Some(AgentId::new("settings-agent").expect("agent")),
            project_id: Some(ProjectId::new("settings-project").expect("project")),
            mission_id: None,
            thread_id: Some(ThreadId::new("settings-thread").expect("thread")),
            invocation_id: InvocationId::new(),
        }
        .tenant_shared_managed_scope()
    }

    fn store(backend: Arc<InMemoryBackend>) -> FilesystemLearningSettingsStore<InMemoryBackend> {
        let filesystem = ScopedFilesystem::new(backend, |scope| {
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/tenant-shared")?,
                VirtualPath::new(format!("/tenants/{}/shared", scope.tenant_id.as_str()))?,
                MountPermissions::read_write(),
            )])
        });
        FilesystemLearningSettingsStore::new(Arc::new(filesystem), scope())
    }

    #[tokio::test]
    async fn missing_record_defaults_to_disabled() {
        let store = store(Arc::new(InMemoryBackend::new()));
        assert_eq!(store.read().await.expect("read"), None);
    }

    #[tokio::test]
    async fn settings_survive_store_reconstruction() {
        let backend = Arc::new(InMemoryBackend::new());
        let settings = LearningSettings {
            enabled: true,
            model: Some("model-a".to_string()),
            memory_write_policy:
                ironclaw_product_contracts::operator_llm::MemoryWritePolicy::Automatic,
        };
        store(Arc::clone(&backend))
            .write(&settings)
            .await
            .expect("write");
        assert_eq!(store(backend).read().await.expect("read"), Some(settings));
    }

    #[tokio::test]
    async fn old_record_shape_uses_defaults() {
        let backend = Arc::new(InMemoryBackend::new());
        let store = store(Arc::clone(&backend));
        let path =
            FilesystemLearningSettingsStore::<InMemoryBackend>::path().expect("settings path");
        store
            .filesystem
            .write_bytes(&scope(), &path, br#"{"enabled":true}"#.to_vec())
            .await
            .expect("write old record");
        assert_eq!(
            store.read().await.expect("read"),
            Some(LearningSettings {
                enabled: true,
                model: None,
                memory_write_policy:
                    ironclaw_product_contracts::operator_llm::MemoryWritePolicy::Staged,
            })
        );
    }
}

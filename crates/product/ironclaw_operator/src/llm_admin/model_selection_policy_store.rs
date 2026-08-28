//! Tenant-scoped filesystem persistence for the operator-owned user model policy.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{ids::InvocationId, path::ScopedPath, resource::ResourceScope};
use ironclaw_product_contracts::{
    operator_llm::{
        ModelSelectionPolicy, ModelSelectionPolicyStore, ModelSelectionPolicyStoreError,
    },
    surface::ProductSurfaceCaller,
};

const POLICY_PATH: &str = "/tenant-shared/llm-user-model-policy.json";
const POLICY_MAX_BYTES: usize = 64 * 1024;

pub struct FilesystemModelSelectionPolicyStore<F: RootFilesystem + ?Sized> {
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F: RootFilesystem + ?Sized> FilesystemModelSelectionPolicyStore<F> {
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn path() -> Result<ScopedPath, ModelSelectionPolicyStoreError> {
        ScopedPath::new(POLICY_PATH).map_err(|_| ModelSelectionPolicyStoreError::InvalidData)
    }

    fn scope(caller: &ProductSurfaceCaller) -> ResourceScope {
        ResourceScope {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
        .tenant_shared_managed_scope()
    }
}

#[async_trait]
impl<F: RootFilesystem + ?Sized> ModelSelectionPolicyStore
    for FilesystemModelSelectionPolicyStore<F>
{
    async fn read(
        &self,
        caller: &ProductSurfaceCaller,
    ) -> Result<Option<ModelSelectionPolicy>, ModelSelectionPolicyStoreError> {
        let path = Self::path()?;
        let scope = Self::scope(caller);
        let bytes = match self
            .filesystem
            .read_bytes_bounded(&scope, &path, POLICY_MAX_BYTES)
            .await
        {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Err(ModelSelectionPolicyStoreError::InvalidData),
            Err(FilesystemError::NotFound { .. }) => return Ok(None),
            Err(error) => {
                tracing::error!(error = %error, "user model policy read failed");
                return Err(ModelSelectionPolicyStoreError::Unavailable);
            }
        };
        serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            tracing::error!(error = %error, "user model policy record is invalid");
            ModelSelectionPolicyStoreError::InvalidData
        })
    }

    async fn write(
        &self,
        caller: &ProductSurfaceCaller,
        policy: &ModelSelectionPolicy,
    ) -> Result<(), ModelSelectionPolicyStoreError> {
        let bytes =
            serde_json::to_vec(policy).map_err(|_| ModelSelectionPolicyStoreError::InvalidData)?;
        if bytes.len() > POLICY_MAX_BYTES {
            return Err(ModelSelectionPolicyStoreError::InvalidData);
        }
        let path = Self::path()?;
        let scope = Self::scope(caller);
        self.filesystem
            .write_bytes(&scope, &path, bytes)
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "user model policy write failed");
                ModelSelectionPolicyStoreError::Unavailable
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        ids::{TenantId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };
    use ironclaw_product_contracts::operator_llm::{LlmModelCatalogEntry, LlmModelModality};

    fn caller(tenant: &str, user: &str) -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            TenantId::new(tenant).expect("tenant"),
            UserId::new(user).expect("user"),
            None,
            None,
        )
    }

    fn filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        Arc::new(ScopedFilesystem::new(
            Arc::new(InMemoryBackend::new()),
            |scope| {
                MountView::new(vec![MountGrant::new(
                    MountAlias::new("/tenant-shared")?,
                    VirtualPath::new(format!("/tenants/{}/shared", scope.tenant_id.as_str()))?,
                    MountPermissions::read_write(),
                )])
            },
        ))
    }

    fn store() -> FilesystemModelSelectionPolicyStore<InMemoryBackend> {
        FilesystemModelSelectionPolicyStore::new(filesystem())
    }

    fn policy(provider: &str) -> ModelSelectionPolicy {
        ModelSelectionPolicy {
            provider_id: provider.to_string(),
            workspace_default: "model-a".to_string(),
            allowed_models: vec!["model-a".to_string(), "model-b".to_string()],
            model_entries: vec![LlmModelCatalogEntry {
                id: "model-a".to_string(),
                input_modalities: vec![LlmModelModality::Text, LlmModelModality::Image],
                output_modalities: vec![LlmModelModality::Text],
            }],
        }
    }

    #[tokio::test]
    async fn policy_is_shared_by_users_inside_one_tenant_and_isolated_across_tenants() {
        let store = store();
        store
            .write(&caller("tenant-a", "admin"), &policy("provider-a"))
            .await
            .expect("write policy");

        assert_eq!(
            store
                .read(&caller("tenant-a", "member"))
                .await
                .expect("read"),
            Some(policy("provider-a")),
        );
        assert_eq!(
            store
                .read(&caller("tenant-b", "admin"))
                .await
                .expect("read"),
            None,
        );
    }

    #[tokio::test]
    async fn legacy_policy_record_loads_without_capability_metadata() {
        let filesystem = filesystem();
        let store = FilesystemModelSelectionPolicyStore::new(Arc::clone(&filesystem));
        let policy_caller = caller("tenant-a", "admin");
        filesystem
            .write_bytes(
                &FilesystemModelSelectionPolicyStore::<InMemoryBackend>::scope(&policy_caller),
                &FilesystemModelSelectionPolicyStore::<InMemoryBackend>::path()
                    .expect("policy path"),
                br#"{"provider_id":"nearai","workspace_default":"model-a","allowed_models":["model-a"]}"#
                    .to_vec(),
            )
            .await
            .expect("write legacy policy");

        let loaded = store
            .read(&policy_caller)
            .await
            .expect("read legacy policy")
            .expect("policy");

        assert!(loaded.model_entries.is_empty());
    }
}

//! Tenant-scoped filesystem persistence for the operator-owned user model policy.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, FilesystemError, RootFilesystem,
    ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{ids::InvocationId, path::ScopedPath, resource::ResourceScope};
use ironclaw_product_contracts::{
    operator_llm::{
        MODEL_SELECTION_POLICY_MAX_BYTES, ModelSelectionPolicy, ModelSelectionPolicyStore,
        ModelSelectionPolicyStoreError, ModelSelectionPolicyUpdateMode,
    },
    surface::ProductSurfaceCaller,
};

const POLICY_PATH: &str = "/tenant-shared/llm-user-model-policy.json";
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

    fn decode_policy(bytes: &[u8]) -> Result<ModelSelectionPolicy, ModelSelectionPolicyStoreError> {
        if bytes.len() > MODEL_SELECTION_POLICY_MAX_BYTES {
            return Err(ModelSelectionPolicyStoreError::InvalidData);
        }
        serde_json::from_slice(bytes).map_err(|error| {
            tracing::error!(error = %error, "user model policy record is invalid");
            ModelSelectionPolicyStoreError::InvalidData
        })
    }

    fn encode_policy(
        policy: &ModelSelectionPolicy,
    ) -> Result<Entry, ModelSelectionPolicyStoreError> {
        let bytes = serde_json::to_vec(policy).map_err(|error| {
            tracing::error!(error = %error, "user model policy serialization failed");
            ModelSelectionPolicyStoreError::InvalidData
        })?;
        if bytes.len() > MODEL_SELECTION_POLICY_MAX_BYTES {
            return Err(ModelSelectionPolicyStoreError::InvalidData);
        }
        Ok(Entry::bytes(bytes).with_content_type(ContentType::json()))
    }
}

fn map_cas_update_error(
    error: CasUpdateError<ModelSelectionPolicyStoreError>,
) -> ModelSelectionPolicyStoreError {
    match error {
        CasUpdateError::Apply(error) => error,
        error => {
            tracing::error!(error = ?error, "atomic user model policy update failed");
            ModelSelectionPolicyStoreError::Unavailable
        }
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
            .read_bytes_bounded(&scope, &path, MODEL_SELECTION_POLICY_MAX_BYTES)
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
        Self::decode_policy(&bytes).map(Some)
    }

    async fn update(
        &self,
        caller: &ProductSurfaceCaller,
        policy: &ModelSelectionPolicy,
        mode: ModelSelectionPolicyUpdateMode,
    ) -> Result<ModelSelectionPolicy, ModelSelectionPolicyStoreError> {
        let path = Self::path()?;
        let scope = Self::scope(caller);
        let requested = policy.clone();
        cas_update(
            &self.filesystem,
            &scope,
            &path,
            Self::decode_policy,
            Self::encode_policy,
            move |current| {
                let mut next = requested.clone();
                async move {
                    if mode == ModelSelectionPolicyUpdateMode::PreserveExistingModelEntries {
                        next.model_entries = current
                            .filter(|stored| stored.provider_id == next.provider_id)
                            .map(|stored| {
                                stored
                                    .model_entries
                                    .into_iter()
                                    .filter(|entry| next.allowed_models.contains(&entry.id))
                                    .collect()
                            })
                            .unwrap_or_default();
                    }
                    Ok(CasApply::new(next.clone(), next))
                }
            },
        )
        .await
        .map_err(map_cas_update_error)
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
            .update(
                &caller("tenant-a", "admin"),
                &policy("provider-a"),
                ModelSelectionPolicyUpdateMode::Replace,
            )
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

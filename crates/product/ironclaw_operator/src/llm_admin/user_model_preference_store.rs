//! Caller-scoped filesystem persistence for user model preferences.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{ids::InvocationId, path::ScopedPath, resource::ResourceScope};
use ironclaw_product_contracts::{
    operator_llm::{UserModelPreference, UserModelPreferenceStore, UserModelPreferenceStoreError},
    surface::ProductSurfaceCaller,
};

const PREFERENCE_PATH: &str = "/llm-preferences/model.json";
const PREFERENCE_MAX_BYTES: usize = 4 * 1024;

/// Filesystem-backed preference store isolated by authenticated tenant/user.
pub struct FilesystemUserModelPreferenceStore<F: RootFilesystem + ?Sized> {
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F: RootFilesystem + ?Sized> FilesystemUserModelPreferenceStore<F> {
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn path() -> Result<ScopedPath, UserModelPreferenceStoreError> {
        ScopedPath::new(PREFERENCE_PATH).map_err(|_| UserModelPreferenceStoreError::InvalidData)
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
    }
}

#[async_trait]
impl<F: RootFilesystem + ?Sized> UserModelPreferenceStore
    for FilesystemUserModelPreferenceStore<F>
{
    async fn read(
        &self,
        caller: &ProductSurfaceCaller,
    ) -> Result<Option<UserModelPreference>, UserModelPreferenceStoreError> {
        let path = Self::path()?;
        let scope = Self::scope(caller);
        let bytes = match self
            .filesystem
            .read_bytes_bounded(&scope, &path, PREFERENCE_MAX_BYTES)
            .await
        {
            Ok(Some(bytes)) => bytes,
            // The bounded-read contract uses `None` for an existing oversized file.
            Ok(None) => return Err(UserModelPreferenceStoreError::InvalidData),
            Err(FilesystemError::NotFound { .. }) => return Ok(None),
            Err(error) => {
                tracing::error!(
                    target: crate::operator_logs::SERVER_DIAGNOSTIC_TARGET,
                    error = %error,
                    "user model preference read failed"
                );
                tracing::error!(
                    error_category = "filesystem_unavailable",
                    "user model preference read failed"
                );
                return Err(UserModelPreferenceStoreError::Unavailable);
            }
        };
        serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            tracing::error!(
                target: crate::operator_logs::SERVER_DIAGNOSTIC_TARGET,
                error = %error,
                "user model preference record is invalid"
            );
            tracing::error!(
                error_category = "invalid_record",
                "user model preference record is invalid"
            );
            UserModelPreferenceStoreError::InvalidData
        })
    }

    async fn write(
        &self,
        caller: &ProductSurfaceCaller,
        preference: &UserModelPreference,
    ) -> Result<(), UserModelPreferenceStoreError> {
        let bytes = serde_json::to_vec(preference)
            .map_err(|_| UserModelPreferenceStoreError::InvalidData)?;
        if bytes.len() > PREFERENCE_MAX_BYTES {
            return Err(UserModelPreferenceStoreError::InvalidData);
        }
        let path = Self::path()?;
        let scope = Self::scope(caller);
        self.filesystem
            .write_bytes(&scope, &path, bytes)
            .await
            .map_err(|error| {
                tracing::error!(
                    target: crate::operator_logs::SERVER_DIAGNOSTIC_TARGET,
                    error = %error,
                    "user model preference write failed"
                );
                tracing::error!(
                    error_category = "filesystem_unavailable",
                    "user model preference write failed"
                );
                UserModelPreferenceStoreError::Unavailable
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::{
        DiskFilesystem, Fault, FaultInjecting, FilesystemOperation, InMemoryBackend,
    };
    use ironclaw_host_api::{
        ids::{TenantId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{HostPath, MountAlias, VirtualPath},
    };

    fn caller(tenant: &str, user: &str) -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            TenantId::new(tenant).expect("tenant"),
            UserId::new(user).expect("user"),
            None,
            None,
        )
    }

    fn store() -> FilesystemUserModelPreferenceStore<InMemoryBackend> {
        let filesystem = ScopedFilesystem::new(Arc::new(InMemoryBackend::new()), |scope| {
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/llm-preferences")?,
                VirtualPath::new(format!(
                    "/tenants/{}/users/{}/llm-preferences",
                    scope.tenant_id.as_str(),
                    scope.user_id.as_str()
                ))?,
                MountPermissions::read_write(),
            )])
        });
        FilesystemUserModelPreferenceStore::new(Arc::new(filesystem))
    }

    fn disk_store(
        host_root: &std::path::Path,
    ) -> FilesystemUserModelPreferenceStore<DiskFilesystem> {
        let mut backend = DiskFilesystem::new();
        backend
            .mount_local(
                VirtualPath::new("/tenants").expect("virtual root"),
                HostPath::from_path_buf(host_root.to_path_buf()),
            )
            .expect("mount disk backend");
        let filesystem = ScopedFilesystem::new(Arc::new(backend), |scope| {
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/llm-preferences")?,
                VirtualPath::new(format!(
                    "/tenants/{}/users/{}/llm-preferences",
                    scope.tenant_id.as_str(),
                    scope.user_id.as_str()
                ))?,
                MountPermissions::read_write(),
            )])
        });
        FilesystemUserModelPreferenceStore::new(Arc::new(filesystem))
    }

    fn fault_injecting_store() -> (
        FilesystemUserModelPreferenceStore<FaultInjecting<InMemoryBackend>>,
        Arc<FaultInjecting<InMemoryBackend>>,
    ) {
        let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
        let filesystem = ScopedFilesystem::new(Arc::clone(&backend), |scope| {
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/llm-preferences")?,
                VirtualPath::new(format!(
                    "/tenants/{}/users/{}/llm-preferences",
                    scope.tenant_id.as_str(),
                    scope.user_id.as_str()
                ))?,
                MountPermissions::read_write(),
            )])
        });
        (
            FilesystemUserModelPreferenceStore::new(Arc::new(filesystem)),
            backend,
        )
    }

    #[tokio::test]
    async fn preference_is_isolated_by_tenant_and_user() {
        let store = store();
        let preference = UserModelPreference {
            model: Some("model-b".to_string()),
        };
        store
            .write(&caller("tenant-a", "alice"), &preference)
            .await
            .expect("write preference");

        assert_eq!(
            store
                .read(&caller("tenant-a", "alice"))
                .await
                .expect("read preference"),
            Some(preference),
        );
        assert_eq!(
            store
                .read(&caller("tenant-a", "bob"))
                .await
                .expect("read other user"),
            None,
        );
        assert_eq!(
            store
                .read(&caller("tenant-b", "alice"))
                .await
                .expect("read other tenant"),
            None,
        );
    }

    #[tokio::test]
    async fn preference_survives_store_reconstruction_on_disk() {
        let storage = tempfile::tempdir().expect("temporary storage");
        let preference = UserModelPreference {
            model: Some("model-b".to_string()),
        };
        disk_store(storage.path())
            .write(&caller("tenant-a", "alice"), &preference)
            .await
            .expect("write preference");

        let reopened = disk_store(storage.path());
        assert_eq!(
            reopened
                .read(&caller("tenant-a", "alice"))
                .await
                .expect("read preference after reconstruction"),
            Some(preference),
        );
        assert_eq!(
            reopened
                .read(&caller("tenant-a", "bob"))
                .await
                .expect("read other user after reconstruction"),
            None,
        );
    }

    #[tokio::test]
    async fn oversized_preference_fails_closed_as_invalid_data() {
        let backend = Arc::new(InMemoryBackend::new());
        let filesystem = Arc::new(ScopedFilesystem::new(backend, |scope| {
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/llm-preferences")?,
                VirtualPath::new(format!(
                    "/tenants/{}/users/{}/llm-preferences",
                    scope.tenant_id.as_str(),
                    scope.user_id.as_str()
                ))?,
                MountPermissions::read_write(),
            )])
        }));
        let test_caller = caller("tenant-a", "alice");
        filesystem
            .write_bytes(
                &FilesystemUserModelPreferenceStore::<InMemoryBackend>::scope(&test_caller),
                &FilesystemUserModelPreferenceStore::<InMemoryBackend>::path()
                    .expect("preference path"),
                vec![b'x'; PREFERENCE_MAX_BYTES + 1],
            )
            .await
            .expect("write oversized record directly");
        let store = FilesystemUserModelPreferenceStore::new(filesystem);

        assert_eq!(
            store.read(&test_caller).await,
            Err(UserModelPreferenceStoreError::InvalidData),
        );
    }

    #[tokio::test]
    async fn backend_failures_are_classified_as_unavailable() {
        let (store, backend) = fault_injecting_store();
        let test_caller = caller("tenant-a", "alice");
        let preference = UserModelPreference {
            model: Some("model-b".to_string()),
        };
        store
            .write(&test_caller, &preference)
            .await
            .expect("seed preference before injecting failures");
        backend.add_fault(
            Fault::on(FilesystemOperation::ReadFile)
                .path("/llm-preferences/")
                .backend("preference read unavailable"),
        );
        backend.add_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .path("/llm-preferences/")
                .backend("preference write unavailable"),
        );

        assert_eq!(
            store.read(&test_caller).await,
            Err(UserModelPreferenceStoreError::Unavailable),
        );
        assert_eq!(
            store.write(&test_caller, &preference).await,
            Err(UserModelPreferenceStoreError::Unavailable),
        );
    }
}

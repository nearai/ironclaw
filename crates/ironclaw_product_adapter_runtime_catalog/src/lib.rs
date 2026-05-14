//! Runtime-facing ProductAdapter activation catalog for IronClaw Reborn.
//!
//! This crate reads [`ProductAdapterRegistryStore`] and shapes enabled adapter
//! installations for runtime composition. It does not own persistence, load
//! WASM components, route webhooks, perform HTTP egress, or resolve secret
//! material.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use ironclaw_product_adapter_registry::{
    ProductAdapterInstallation, ProductAdapterManifest, ProductAdapterRegistryStore, RegistryError,
};
use ironclaw_product_adapters::{AdapterInstallationId, ProductAdapterId};
use thiserror::Error;

/// Runtime-facing catalog over a ProductAdapter registry store.
#[derive(Debug, Clone)]
pub struct ProductAdapterRuntimeCatalog<S> {
    store: S,
}

impl<S> ProductAdapterRuntimeCatalog<S>
where
    S: ProductAdapterRegistryStore,
{
    /// Creates a catalog backed by the provided registry store.
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Lists runtime-visible ProductAdapter entries.
    ///
    /// Only explicitly enabled installations are returned. For each enabled
    /// installation, the registered manifest is fetched and paired with the
    /// installation snapshot so runtime consumers do not re-query the registry.
    pub async fn list_enabled_entries(
        &self,
    ) -> Result<Vec<ProductAdapterRuntimeEntry>, ProductAdapterRuntimeCatalogError> {
        let installations = self.store.list_enabled_installations().await?;
        let manifests: HashMap<_, _> = self
            .store
            .list_manifests()
            .await?
            .into_iter()
            .map(|manifest| (manifest.adapter_id().clone(), manifest))
            .collect();
        let mut entries = Vec::with_capacity(installations.len());
        for installation in installations {
            let manifest = manifests
                .get(installation.adapter_id())
                .cloned()
                .ok_or_else(|| ProductAdapterRuntimeCatalogError::MissingManifest {
                    installation_id: installation.installation_id().clone(),
                    adapter_id: installation.adapter_id().clone(),
                })?;
            if manifest.manifest_hash() != installation.manifest_ref().manifest_hash() {
                return Err(ProductAdapterRuntimeCatalogError::ManifestHashMismatch {
                    installation_id: installation.installation_id().clone(),
                    adapter_id: installation.adapter_id().clone(),
                });
            }
            entries.push(ProductAdapterRuntimeEntry::new(installation, manifest));
        }
        entries.sort_by(|a, b| a.installation_id().cmp(b.installation_id()));
        Ok(entries)
    }
}

/// Enabled ProductAdapter installation paired with its registered manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductAdapterRuntimeEntry {
    installation: ProductAdapterInstallation,
    manifest: ProductAdapterManifest,
}

impl ProductAdapterRuntimeEntry {
    fn new(installation: ProductAdapterInstallation, manifest: ProductAdapterManifest) -> Self {
        Self {
            installation,
            manifest,
        }
    }

    /// Returns the enabled installation snapshot.
    pub fn installation(&self) -> &ProductAdapterInstallation {
        &self.installation
    }

    /// Returns the manifest registered for this installation's adapter id.
    pub fn manifest(&self) -> &ProductAdapterManifest {
        &self.manifest
    }

    /// Returns the runtime installation id.
    pub fn installation_id(&self) -> &AdapterInstallationId {
        self.installation.installation_id()
    }

    /// Returns the adapter id.
    pub fn adapter_id(&self) -> &ProductAdapterId {
        self.installation.adapter_id()
    }
}

/// Errors raised while building runtime catalog entries.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProductAdapterRuntimeCatalogError {
    /// Registry operation failed.
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// Store surfaced an enabled installation whose manifest is absent.
    #[error("enabled installation {installation_id} references missing manifest {adapter_id}")]
    MissingManifest {
        installation_id: AdapterInstallationId,
        adapter_id: ProductAdapterId,
    },
    /// Store surfaced an enabled installation whose manifest pin no longer matches.
    #[error("enabled installation {installation_id} has stale manifest hash for {adapter_id}")]
    ManifestHashMismatch {
        installation_id: AdapterInstallationId,
        adapter_id: ProductAdapterId,
    },
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_host_api::SecretHandle;
    use ironclaw_product_adapter_registry::{
        InMemoryProductAdapterRegistryStore, ManifestHash, ProductAdapterActivationState,
        ProductAdapterComponentRef, ProductAdapterCredentialBinding, ProductAdapterHealthSnapshot,
        ProductAdapterManifestRef,
    };
    use ironclaw_product_adapters::{
        AuthRequirement, DeclaredEgressHost, DeclaredEgressTarget, EgressCredentialHandle,
        ProductAdapterCapabilities, ProductSurfaceKind,
    };

    use super::*;

    fn adapter_id(value: &str) -> ProductAdapterId {
        ProductAdapterId::new(value).unwrap()
    }

    fn installation_id(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).unwrap()
    }

    fn credential(value: &str) -> EgressCredentialHandle {
        EgressCredentialHandle::new(value).unwrap()
    }

    fn host(value: &str) -> DeclaredEgressHost {
        DeclaredEgressHost::new(value).unwrap()
    }

    fn secret_handle(value: &str) -> SecretHandle {
        SecretHandle::new(format!("secret_{value}")).unwrap()
    }

    fn manifest(adapter: &str) -> ProductAdapterManifest {
        manifest_with_hash(adapter, "sha256:abc123")
    }

    fn manifest_with_hash(adapter: &str, manifest_hash: &str) -> ProductAdapterManifest {
        let token = credential("telegram_bot_token");
        ProductAdapterManifest::new(
            adapter_id(adapter),
            semver::Version::new(0, 1, 0),
            ProductSurfaceKind::ExternalChannel,
            ProductAdapterComponentRef::new(format!("file://adapters/{adapter}.wasm")).unwrap(),
            ProductAdapterCapabilities::external_channel_default(),
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Telegram-Bot-Api-Secret-Token".to_string(),
            },
            vec![DeclaredEgressTarget::new(
                host("api.telegram.org"),
                Some(token.clone()),
            )],
            vec![token],
            Some(ManifestHash::new(manifest_hash).unwrap()),
        )
        .unwrap()
    }

    fn installation(
        id: &str,
        adapter: &str,
        state: ProductAdapterActivationState,
    ) -> ProductAdapterInstallation {
        ProductAdapterInstallation::new(
            installation_id(id),
            adapter_id(adapter),
            state,
            ProductAdapterManifestRef::new(
                adapter_id(adapter),
                Some(ManifestHash::new("sha256:abc123").unwrap()),
            ),
            vec![ProductAdapterCredentialBinding::new(
                credential("telegram_bot_token"),
                secret_handle(id),
            )],
            Utc::now(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn empty_registry_yields_empty_catalog() {
        let catalog =
            ProductAdapterRuntimeCatalog::new(InMemoryProductAdapterRegistryStore::default());

        let entries = catalog.list_enabled_entries().await.unwrap();

        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn installed_and_disabled_installations_are_excluded() {
        let store = InMemoryProductAdapterRegistryStore::default();
        store
            .upsert_manifest(manifest("telegram-v2"))
            .await
            .unwrap();
        store
            .upsert_installation(installation(
                "acme-telegram-installed",
                "telegram-v2",
                ProductAdapterActivationState::Installed,
            ))
            .await
            .unwrap();
        store
            .upsert_installation(installation(
                "acme-telegram-disabled",
                "telegram-v2",
                ProductAdapterActivationState::Disabled,
            ))
            .await
            .unwrap();
        let catalog = ProductAdapterRuntimeCatalog::new(store);

        let entries = catalog.list_enabled_entries().await.unwrap();

        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn enabled_installation_returns_manifest_and_installation_data() {
        let store = InMemoryProductAdapterRegistryStore::default();
        store
            .upsert_manifest(manifest("telegram-v2"))
            .await
            .unwrap();
        store
            .upsert_installation(installation(
                "acme-telegram-prod",
                "telegram-v2",
                ProductAdapterActivationState::Enabled,
            ))
            .await
            .unwrap();
        let catalog = ProductAdapterRuntimeCatalog::new(store);

        let entries = catalog.list_enabled_entries().await.unwrap();

        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.installation_id().as_str(), "acme-telegram-prod");
        assert_eq!(entry.adapter_id().as_str(), "telegram-v2");
        assert_eq!(
            entry.manifest().component_ref().as_str(),
            "file://adapters/telegram-v2.wasm"
        );
        assert_eq!(entry.installation().credential_bindings().len(), 1);
    }

    #[tokio::test]
    async fn entries_are_sorted_by_installation_id() {
        let store = InMemoryProductAdapterRegistryStore::default();
        store
            .upsert_manifest(manifest("telegram-v2"))
            .await
            .unwrap();
        for id in ["zeta-telegram", "alpha-telegram"] {
            store
                .upsert_installation(installation(
                    id,
                    "telegram-v2",
                    ProductAdapterActivationState::Enabled,
                ))
                .await
                .unwrap();
        }
        let catalog = ProductAdapterRuntimeCatalog::new(store);

        let ids: Vec<_> = catalog
            .list_enabled_entries()
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.installation_id().as_str().to_string())
            .collect();

        assert_eq!(ids, vec!["alpha-telegram", "zeta-telegram"]);
    }

    #[tokio::test]
    async fn registry_validation_blocks_bad_binding_before_catalog_reads_it() {
        let store = InMemoryProductAdapterRegistryStore::default();
        store
            .upsert_manifest(manifest("telegram-v2"))
            .await
            .unwrap();
        let invalid = ProductAdapterInstallation::new(
            installation_id("acme-telegram-prod"),
            adapter_id("telegram-v2"),
            ProductAdapterActivationState::Enabled,
            ProductAdapterManifestRef::new(
                adapter_id("telegram-v2"),
                Some(ManifestHash::new("sha256:abc123").unwrap()),
            ),
            vec![ProductAdapterCredentialBinding::new(
                credential("slack_bot_token"),
                secret_handle("acme_telegram_prod"),
            )],
            Utc::now(),
        )
        .unwrap();

        let err = store.upsert_installation(invalid).await.unwrap_err();

        assert!(matches!(
            err,
            RegistryError::UndeclaredCredentialHandle { .. }
        ));
    }

    #[tokio::test]
    async fn missing_manifest_for_enabled_installation_is_error() {
        let store = Arc::new(InconsistentStore::with_enabled_installation(installation(
            "orphan-telegram",
            "telegram-v2",
            ProductAdapterActivationState::Enabled,
        )));
        let catalog = ProductAdapterRuntimeCatalog::new(store);

        let err = catalog.list_enabled_entries().await.unwrap_err();

        assert!(matches!(
            err,
            ProductAdapterRuntimeCatalogError::MissingManifest { .. }
        ));
    }

    #[tokio::test]
    async fn stale_manifest_hash_for_enabled_installation_is_error() {
        let installation = installation(
            "stale-telegram",
            "telegram-v2",
            ProductAdapterActivationState::Enabled,
        );
        let store = Arc::new(InconsistentStore::with_installation_and_manifest(
            installation,
            manifest_with_hash("telegram-v2", "sha256:different"),
        ));
        let catalog = ProductAdapterRuntimeCatalog::new(store);

        let err = catalog.list_enabled_entries().await.unwrap_err();

        assert!(matches!(
            err,
            ProductAdapterRuntimeCatalogError::ManifestHashMismatch { .. }
        ));
    }

    #[derive(Debug, Default)]
    struct InconsistentStore {
        installations: Mutex<Vec<ProductAdapterInstallation>>,
        manifests: Mutex<HashMap<ProductAdapterId, ProductAdapterManifest>>,
    }

    impl InconsistentStore {
        fn with_enabled_installation(installation: ProductAdapterInstallation) -> Self {
            Self {
                installations: Mutex::new(vec![installation]),
                manifests: Mutex::new(HashMap::new()),
            }
        }

        fn with_installation_and_manifest(
            installation: ProductAdapterInstallation,
            manifest: ProductAdapterManifest,
        ) -> Self {
            let mut manifests = HashMap::new();
            manifests.insert(manifest.adapter_id().clone(), manifest);
            Self {
                installations: Mutex::new(vec![installation]),
                manifests: Mutex::new(manifests),
            }
        }
    }

    #[async_trait]
    impl ProductAdapterRegistryStore for InconsistentStore {
        async fn list_manifests(&self) -> Result<Vec<ProductAdapterManifest>, RegistryError> {
            Ok(self.manifests.lock().unwrap().values().cloned().collect())
        }

        async fn get_manifest(
            &self,
            adapter_id: &ProductAdapterId,
        ) -> Result<Option<ProductAdapterManifest>, RegistryError> {
            Ok(self.manifests.lock().unwrap().get(adapter_id).cloned())
        }

        async fn upsert_manifest(
            &self,
            manifest: ProductAdapterManifest,
        ) -> Result<(), RegistryError> {
            self.manifests
                .lock()
                .unwrap()
                .insert(manifest.adapter_id().clone(), manifest);
            Ok(())
        }

        async fn list_installations(
            &self,
        ) -> Result<Vec<ProductAdapterInstallation>, RegistryError> {
            Ok(self.installations.lock().unwrap().clone())
        }

        async fn list_enabled_installations(
            &self,
        ) -> Result<Vec<ProductAdapterInstallation>, RegistryError> {
            Ok(self
                .installations
                .lock()
                .unwrap()
                .iter()
                .filter(|installation| {
                    installation.activation_state() == ProductAdapterActivationState::Enabled
                })
                .cloned()
                .collect())
        }

        async fn get_installation(
            &self,
            installation_id: &AdapterInstallationId,
        ) -> Result<Option<ProductAdapterInstallation>, RegistryError> {
            Ok(self
                .installations
                .lock()
                .unwrap()
                .iter()
                .find(|installation| installation.installation_id() == installation_id)
                .cloned())
        }

        async fn upsert_installation(
            &self,
            installation: ProductAdapterInstallation,
        ) -> Result<(), RegistryError> {
            let mut installations = self.installations.lock().unwrap();
            if let Some(existing) = installations
                .iter_mut()
                .find(|existing| existing.installation_id() == installation.installation_id())
            {
                *existing = installation;
            } else {
                installations.push(installation);
            }
            Ok(())
        }

        async fn set_activation_state(
            &self,
            installation_id: &AdapterInstallationId,
            state: ProductAdapterActivationState,
        ) -> Result<(), RegistryError> {
            if let Some(installation) = self
                .installations
                .lock()
                .unwrap()
                .iter_mut()
                .find(|installation| installation.installation_id() == installation_id)
            {
                let updated = ProductAdapterInstallation::new(
                    installation.installation_id().clone(),
                    installation.adapter_id().clone(),
                    state,
                    installation.manifest_ref().clone(),
                    installation.credential_bindings().to_vec(),
                    Utc::now(),
                )?;
                *installation = updated;
                Ok(())
            } else {
                Err(RegistryError::InstallationNotFound {
                    installation_id: installation_id.clone(),
                })
            }
        }

        async fn update_health(
            &self,
            installation_id: &AdapterInstallationId,
            _health: ProductAdapterHealthSnapshot,
        ) -> Result<(), RegistryError> {
            if self
                .installations
                .lock()
                .unwrap()
                .iter()
                .any(|installation| installation.installation_id() == installation_id)
            {
                Ok(())
            } else {
                Err(RegistryError::InstallationNotFound {
                    installation_id: installation_id.clone(),
                })
            }
        }
    }
}

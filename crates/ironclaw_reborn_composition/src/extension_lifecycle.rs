use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionActivationState, ExtensionError, ExtensionInstallation, ExtensionInstallationError,
    ExtensionInstallationId, ExtensionInstallationStore, ExtensionLifecycleService,
    ExtensionManifestRecord, ExtensionManifestRef, ExtensionPackage, ManifestHash, ManifestSource,
    SharedExtensionRegistry,
};
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::{ExtensionId, VirtualPath, sha256_digest_token};
use ironclaw_product_workflow::{
    LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase, LifecycleProductResponse,
    ProductWorkflowError,
};
use serde_json::json;
use tokio::sync::Mutex;

use crate::available_extensions::{
    AvailableExtensionCatalog, AvailableExtensionPackage, materialize_available_extension,
    visible_capability_ids,
};
use crate::lifecycle::response_with_payload;

pub(crate) struct RebornLocalExtensionManagementPort {
    filesystem: Arc<LocalFilesystem>,
    catalog: AvailableExtensionCatalog,
    installation_store: Arc<dyn ExtensionInstallationStore>,
    lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
    active_registry: Arc<SharedExtensionRegistry>,
}

impl RebornLocalExtensionManagementPort {
    pub(crate) fn new(
        filesystem: Arc<LocalFilesystem>,
        catalog: AvailableExtensionCatalog,
        installation_store: Arc<dyn ExtensionInstallationStore>,
        lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
        active_registry: Arc<SharedExtensionRegistry>,
    ) -> Self {
        Self {
            filesystem,
            catalog,
            installation_store,
            lifecycle_service,
            active_registry,
        }
    }

    pub(crate) async fn search(
        &self,
        query: &str,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let extensions = self.catalog.search(query);
        let summaries = extensions
            .into_iter()
            .map(|extension| extension.summary_json())
            .collect::<Vec<_>>();
        Ok(response_with_payload(
            None,
            LifecyclePhase::Discovered,
            json!({
                "extensions": summaries,
                "count": summaries.len(),
            }),
        ))
    }

    pub(crate) async fn install(
        &self,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let available = self.catalog.resolve(&package_ref)?;
        let plan = prepare_install(available)?;
        let rollback = self.register_lifecycle_package(&available.package).await?;

        if let Err(error) =
            materialize_available_extension(self.filesystem.as_ref(), available).await
        {
            self.rollback_lifecycle_install(&available.package.id, rollback)
                .await;
            return Err(error);
        }
        if let Err(error) = self.persist_install_plan(plan).await {
            let _ = self
                .delete_materialized_extension_files(&available.package.id)
                .await;
            self.rollback_lifecycle_install(&available.package.id, rollback)
                .await;
            return Err(error);
        }

        Ok(response_with_payload(
            Some(package_ref),
            LifecyclePhase::Installed,
            json!({
                "installed": true,
                "visible_capability_ids": visible_capability_ids(available).map(|id| id.as_str()).collect::<Vec<_>>(),
            }),
        ))
    }

    pub(crate) async fn activate(
        &self,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(&package_ref)?;
        let package = self.lifecycle_package(&extension_id).await?;
        replace_active_extension(&self.active_registry, package)?;
        if let Err(error) = self.enable_lifecycle_package(&extension_id).await {
            remove_active_extension(&self.active_registry, &extension_id);
            return Err(error);
        }
        if let Err(error) = self
            .installation_store
            .set_activation_state(&installation_id, ExtensionActivationState::Enabled)
            .await
        {
            self.disable_lifecycle_package(&extension_id).await;
            remove_active_extension(&self.active_registry, &extension_id);
            return Err(map_extension_installation_error(error));
        }

        Ok(response_with_payload(
            Some(package_ref),
            LifecyclePhase::Active,
            json!({ "activated": true }),
        ))
    }

    pub(crate) async fn remove(
        &self,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(&package_ref)?;
        self.delete_materialized_extension_files(&extension_id)
            .await?;
        self.installation_store
            .set_activation_state(&installation_id, ExtensionActivationState::Disabled)
            .await
            .map_err(map_extension_installation_error)?;
        self.installation_store
            .delete_installation(&installation_id)
            .await
            .map_err(map_extension_installation_error)?;
        self.installation_store
            .delete_manifest(&extension_id)
            .await
            .map_err(map_extension_installation_error)?;
        self.remove_lifecycle_package(&extension_id).await?;
        remove_active_extension(&self.active_registry, &extension_id);

        Ok(response_with_payload(
            Some(package_ref),
            LifecyclePhase::Removed,
            json!({ "removed": true }),
        ))
    }

    async fn register_lifecycle_package(
        &self,
        package: &ExtensionPackage,
    ) -> Result<LifecycleRollback, ProductWorkflowError> {
        let mut lifecycle = self.lifecycle_service.lock().await;
        let previous_package = lifecycle.registry().get_extension(&package.id).cloned();
        if previous_package.is_some() {
            lifecycle
                .update(package.clone())
                .await
                .map_err(map_extension_error)?;
        } else {
            lifecycle
                .install(package.clone())
                .await
                .map_err(map_extension_error)?;
        }
        Ok(LifecycleRollback { previous_package })
    }

    async fn lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<ExtensionPackage, ProductWorkflowError> {
        let lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .registry()
            .get_extension(extension_id)
            .cloned()
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: format!("extension {} is not installed", extension_id.as_str()),
            })
    }

    async fn enable_lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError> {
        self.lifecycle_service
            .lock()
            .await
            .enable(extension_id)
            .await
            .map_err(map_extension_error)
    }

    async fn disable_lifecycle_package(&self, extension_id: &ExtensionId) {
        let _ = self
            .lifecycle_service
            .lock()
            .await
            .disable(extension_id)
            .await;
    }

    async fn remove_lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError> {
        self.lifecycle_service
            .lock()
            .await
            .remove(extension_id)
            .await
            .map_err(map_extension_error)
    }

    async fn rollback_lifecycle_install(
        &self,
        extension_id: &ExtensionId,
        rollback: LifecycleRollback,
    ) {
        let mut lifecycle = self.lifecycle_service.lock().await;
        if let Some(package) = rollback.previous_package {
            let _ = lifecycle.update(package).await;
        } else {
            let _ = lifecycle.remove(extension_id).await;
        }
    }

    async fn persist_install_plan(
        &self,
        plan: ExtensionInstallPlan,
    ) -> Result<(), ProductWorkflowError> {
        let extension_id = plan.installation.extension_id().clone();
        if let Err(error) = self
            .installation_store
            .upsert_manifest(plan.manifest_record)
            .await
        {
            return Err(map_extension_installation_error(error));
        }
        if let Err(error) = self
            .installation_store
            .upsert_installation(plan.installation)
            .await
        {
            let _ = self.installation_store.delete_manifest(&extension_id).await;
            return Err(map_extension_installation_error(error));
        }
        Ok(())
    }

    async fn delete_materialized_extension_files(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError> {
        let Ok(extension_root) =
            VirtualPath::new(format!("/system/extensions/{}", extension_id.as_str()))
        else {
            return Ok(());
        };
        self.filesystem
            .delete(&extension_root)
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("failed to remove extension files: {error}"),
            })
    }
}

struct ExtensionInstallPlan {
    manifest_record: ExtensionManifestRecord,
    installation: ExtensionInstallation,
}

struct LifecycleRollback {
    previous_package: Option<ExtensionPackage>,
}

fn prepare_install(
    available: &AvailableExtensionPackage,
) -> Result<ExtensionInstallPlan, ProductWorkflowError> {
    let manifest_hash = ManifestHash::new(sha256_digest_token(available.manifest_toml.as_bytes()))
        .map_err(map_extension_installation_error)?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected extension install: {error}"),
        }
    })?;
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host API contract registry rejected extension install: {error}"),
            }
        })?;
    let manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
        &available.manifest_toml,
        ManifestSource::HostBundled,
        &host_ports,
        Some(manifest_hash.clone()),
        &contracts,
    )
    .map_err(map_extension_installation_error)?;
    let installation_id = ExtensionInstallationId::new(available.package.id.as_str().to_string())
        .map_err(map_extension_installation_error)?;
    let installation = ExtensionInstallation::new(
        installation_id,
        available.package.id.clone(),
        ExtensionActivationState::Installed,
        ExtensionManifestRef::new(available.package.id.clone(), Some(manifest_hash)),
        Vec::new(),
        chrono::Utc::now(),
    )
    .map_err(map_extension_installation_error)?;
    Ok(ExtensionInstallPlan {
        manifest_record,
        installation,
    })
}

fn extension_ids_from_package_ref(
    package_ref: &LifecyclePackageRef,
) -> Result<(ExtensionId, ExtensionInstallationId), ProductWorkflowError> {
    package_ref.require_kind(LifecyclePackageKind::Extension)?;
    let extension_id = ExtensionId::new(package_ref.id.as_str().to_string()).map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: error.to_string(),
        }
    })?;
    let installation_id = ExtensionInstallationId::new(extension_id.as_str().to_string())
        .map_err(map_extension_installation_error)?;
    Ok((extension_id, installation_id))
}

fn replace_active_extension(
    active_registry: &SharedExtensionRegistry,
    package: ExtensionPackage,
) -> Result<(), ProductWorkflowError> {
    active_registry.upsert(package).map_err(map_extension_error)
}

fn remove_active_extension(active_registry: &SharedExtensionRegistry, extension_id: &ExtensionId) {
    active_registry.remove(extension_id);
}

fn map_extension_error(error: ExtensionError) -> ProductWorkflowError {
    match error {
        ExtensionError::Filesystem(_) | ExtensionError::LifecycleEventSink { .. } => {
            ProductWorkflowError::Transient {
                reason: error.to_string(),
            }
        }
        _ => ProductWorkflowError::InvalidBindingRequest {
            reason: error.to_string(),
        },
    }
}

fn map_extension_installation_error(error: ExtensionInstallationError) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::available_extensions::{AvailableExtensionAsset, AvailableExtensionPackage};
    use ironclaw_extensions::{
        ExtensionLifecycleService, ExtensionManifest, ExtensionRegistry,
        InMemoryExtensionInstallationStore,
    };
    use ironclaw_host_api::{
        HostPath, HostPortCatalog, MountAlias, MountGrant, MountPermissions, MountView, TenantId,
        UserId,
    };
    use ironclaw_product_workflow::{
        LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
        LifecycleProductSurfaceContext, LifecycleReadinessBlocker,
    };

    #[tokio::test]
    async fn extension_lifecycle_installs_activates_and_removes_catalog_package() {
        let (_dir, storage_root, facade, active_registry) = extension_lifecycle_fixture();

        // safety: test-only lifecycle facade calls; no database transaction is involved.
        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "fixture".to_string(),
                },
            )
            .await
            .expect("search extensions");
        assert_eq!(search.phase, LifecyclePhase::Discovered);
        let extensions = search
            .payload
            .as_ref()
            .and_then(|payload| payload.get("extensions"))
            .and_then(serde_json::Value::as_array)
            .expect("extension summaries");
        assert_eq!(extensions.len(), 1);
        let visible_ids = extensions[0]
            .get("visible_read_only_capability_ids")
            .and_then(serde_json::Value::as_array)
            .expect("visible read-only ids");
        assert_eq!(
            visible_ids
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>(),
            vec!["fixture.search"]
        );

        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        let install = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install extension");
        assert_eq!(install.phase, LifecyclePhase::Installed);
        assert!(
            storage_root
                .join("system/extensions/fixture/manifest.toml")
                .exists()
        );
        assert!(
            storage_root
                .join("system/extensions/fixture/wasm/fixture.wasm")
                .exists()
        );
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_none()
        );

        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("activate extension");
        assert_eq!(activate.phase, LifecyclePhase::Active);
        let active = active_registry.snapshot();
        assert!(
            active
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_some()
        );
        assert!(
            active
                .get_capability(&ironclaw_host_api::CapabilityId::new("fixture.search").unwrap())
                .is_some()
        );
        assert!(
            active
                .get_capability(&ironclaw_host_api::CapabilityId::new("fixture.write").unwrap())
                .is_some()
        );

        let remove = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect("remove extension");
        assert_eq!(remove.phase, LifecyclePhase::Removed);
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_none()
        );
        assert!(
            !storage_root
                .join("system/extensions/fixture/manifest.toml")
                .exists()
        );
        assert!(
            !storage_root
                .join("system/extensions/fixture/wasm/fixture.wasm")
                .exists()
        );
    }

    #[tokio::test]
    async fn extension_install_rejects_skill_package_ref() {
        let (_dir, _storage_root, facade, _active_registry) = extension_lifecycle_fixture();

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Skill, "fixture")
                        .expect("valid skill ref"),
                },
            )
            .await
            .expect_err("extension install rejects non-extension refs");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    #[tokio::test]
    async fn extension_auth_and_configure_return_unsupported() {
        let (_dir, _storage_root, facade, _active_registry) = extension_lifecycle_fixture();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").unwrap();

        for action in [
            LifecycleProductAction::ExtensionAuth {
                package_ref: package_ref.clone(),
            },
            LifecycleProductAction::ExtensionConfigure {
                package_ref: package_ref.clone(),
                payload: None,
            },
        ] {
            let response = facade
                .execute(lifecycle_surface_context(), action)
                .await
                .expect("unsupported response");
            assert_unsupported_extension_response(response);
        }
    }

    #[tokio::test]
    async fn project_package_returns_unsupported() {
        let (_dir, _storage_root, facade, _active_registry) = extension_lifecycle_fixture();
        let response = facade
            .project_package(
                lifecycle_surface_context(),
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").unwrap(),
            )
            .await
            .expect("unsupported projection");

        assert_unsupported_extension_response(response);
    }

    fn extension_lifecycle_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

        let mut filesystem = LocalFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let filesystem = Arc::new(filesystem);
        let skill_management = Arc::new(crate::lifecycle::RebornLocalSkillManagementPort::new(
            UserId::new("lifecycle-owner").expect("valid user"),
            Arc::clone(&filesystem),
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/skills").expect("valid alias"),
                VirtualPath::new("/projects/skills").expect("valid path"),
                MountPermissions::read_write_list_delete(),
            )])
            .expect("valid mount view"),
        ));
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let extension_management = Arc::new(RebornLocalExtensionManagementPort::new(
            Arc::clone(&filesystem),
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            Arc::new(InMemoryExtensionInstallationStore::default()),
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                ExtensionRegistry::new(),
            ))),
            Arc::clone(&active_registry),
        ));
        let facade = crate::lifecycle::RebornLocalLifecycleFacade::new(skill_management)
            .with_extension_management(extension_management);
        (dir, storage_root, facade, active_registry)
    }

    fn lifecycle_surface_context() -> LifecycleProductContext {
        LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
            tenant_id: TenantId::new("lifecycle-tenant").expect("valid tenant"),
            user_id: UserId::new("lifecycle-owner").expect("valid user"),
            agent_id: None,
            project_id: None,
        })
    }

    fn fixture_extension_package() -> AvailableExtensionPackage {
        static MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "fixture"
name = "Fixture Extension"
version = "0.1.0"
description = "Lifecycle fixture extension"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/fixture.wasm"

[[capabilities]]
id = "fixture.search"
description = "Search fixture data"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capabilities]]
id = "fixture.write"
description = "Write fixture data"
effects = ["network", "external_write"]
default_permission = "ask"
visibility = "host_internal"
input_schema_ref = "schemas/write.input.json"
output_schema_ref = "schemas/write.output.json"
"#;
        let manifest = ExtensionManifest::parse(
            MANIFEST,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
        )
        .expect("fixture manifest");
        let root = VirtualPath::new("/system/extensions/fixture").expect("extension root");
        let package = ExtensionPackage::from_manifest(manifest, root).expect("fixture package");
        AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
                .expect("fixture package ref"),
            manifest_toml: MANIFEST.to_string(),
            package,
            assets: vec![
                AvailableExtensionAsset {
                    path: "manifest.toml".to_string(),
                    bytes: MANIFEST.as_bytes().to_vec(),
                },
                AvailableExtensionAsset {
                    path: "wasm/fixture.wasm".to_string(),
                    bytes: b"\0asm\x01\0\0\0".to_vec(),
                },
            ],
        }
    }

    fn assert_unsupported_extension_response(response: LifecycleProductResponse) {
        assert_eq!(response.phase, LifecyclePhase::UnsupportedOrLegacy);
        assert!(response.blockers.iter().any(|blocker| matches!(
            blocker,
            LifecycleReadinessBlocker::Runtime { ref_id: Some(ref_id) }
                if ref_id.as_str() == "extension_auth_and_configure_not_yet_wired"
        )));
    }
}

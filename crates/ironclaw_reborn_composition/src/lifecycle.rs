use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
    ExtensionInstallationStore, ExtensionLifecycleService, ExtensionManifestRecord,
    ExtensionManifestRef, ManifestHash, ManifestSource, SharedExtensionRegistry,
};
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::{
    ExtensionId, InvocationId, MountView, ResourceScope, UserId, VirtualPath, sha256_digest_token,
};
use ironclaw_product_workflow::{
    LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase,
    LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductResponse, LifecycleReadinessBlocker, ProductWorkflowError,
};
use ironclaw_skills::{
    SkillInstallRequest, SkillManagementContext, SkillManagementError, SkillManagementErrorKind,
    SkillRemoveRequest, SkillSearchRequest, install_skill, remove_skill, search_skills,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::available_extensions::{
    AvailableExtensionCatalog, materialize_available_extension, visible_capability_ids,
};

const SKILL_SEARCH_RESULT_LIMIT: usize = 50;

#[derive(Clone)]
pub(crate) struct RebornLocalSkillManagementPort {
    owner_user_id: UserId,
    filesystem: Arc<LocalFilesystem>,
    skill_management_mounts: MountView,
}

impl RebornLocalSkillManagementPort {
    pub(crate) fn new(
        owner_user_id: UserId,
        filesystem: Arc<LocalFilesystem>,
        skill_management_mounts: MountView,
    ) -> Self {
        Self {
            owner_user_id,
            filesystem,
            skill_management_mounts,
        }
    }

    fn skill_context(&self) -> Result<SkillManagementContext, ProductWorkflowError> {
        let scope = ResourceScope::local_default(self.owner_user_id.clone(), InvocationId::new())
            .map_err(invalid_skill_context)?;
        Ok(SkillManagementContext::new(
            self.filesystem.clone(),
            self.skill_management_mounts.clone(),
            scope,
        ))
    }

    async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<ironclaw_skills::SkillSearchResult, ProductWorkflowError> {
        let context = self.skill_context()?;
        search_skills(&context, SkillSearchRequest { query, limit })
            .await
            .map_err(map_skill_error)
    }

    async fn install(
        &self,
        name: Option<&str>,
        content: &str,
    ) -> Result<ironclaw_skills::SkillInstallResult, ProductWorkflowError> {
        let context = self.skill_context()?;
        install_skill(&context, SkillInstallRequest { name, content })
            .await
            .map_err(map_skill_error)
    }

    async fn remove(
        &self,
        name: &str,
    ) -> Result<ironclaw_skills::SkillRemoveResult, ProductWorkflowError> {
        let context = self.skill_context()?;
        remove_skill(&context, SkillRemoveRequest { name })
            .await
            .map_err(map_skill_error)
    }
}

fn invalid_skill_context(error: impl std::fmt::Display) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    }
}

#[derive(Clone)]
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

    async fn search(&self, query: &str) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let extensions = self.catalog.search(query)?;
        let summaries = extensions
            .iter()
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

    async fn install(
        &self,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let available = self.catalog.resolve(&package_ref)?;
        let manifest_hash =
            ManifestHash::new(sha256_digest_token(available.manifest_toml.as_bytes()))
                .map_err(map_extension_installation_error)?;
        let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host port catalog rejected extension install: {error}"),
            }
        })?;
        let contracts =
            ironclaw_host_runtime::default_host_api_contract_registry().map_err(|error| {
                ProductWorkflowError::InvalidBindingRequest {
                    reason: format!(
                        "host API contract registry rejected extension install: {error}"
                    ),
                }
            })?;
        let manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
            available.manifest_toml,
            ManifestSource::HostBundled,
            &host_ports,
            Some(manifest_hash.clone()),
            &contracts,
        )
        .map_err(map_extension_installation_error)?;
        let installation_id =
            ExtensionInstallationId::new(available.package.id.as_str().to_string())
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
        let previous_package = {
            let mut lifecycle = self.lifecycle_service.lock().await;
            let previous_package = lifecycle
                .registry()
                .get_extension(&available.package.id)
                .cloned();
            if previous_package.is_some() {
                lifecycle
                    .update(available.package.clone())
                    .await
                    .map_err(map_extension_error)?;
            } else {
                lifecycle
                    .install(available.package.clone())
                    .await
                    .map_err(map_extension_error)?;
            }
            previous_package
        };
        if let Err(error) =
            materialize_available_extension(self.filesystem.as_ref(), &available).await
        {
            self.rollback_lifecycle_install(&available.package.id, previous_package)
                .await;
            return Err(error);
        }
        if let Err(error) = self
            .installation_store
            .upsert_manifest(manifest_record)
            .await
        {
            let _ = self
                .delete_materialized_extension_files(&available.package.id)
                .await;
            self.rollback_lifecycle_install(&available.package.id, previous_package)
                .await;
            return Err(map_extension_installation_error(error));
        }
        if let Err(error) = self
            .installation_store
            .upsert_installation(installation)
            .await
        {
            let _ = self
                .installation_store
                .delete_manifest(&available.package.id)
                .await;
            let _ = self
                .delete_materialized_extension_files(&available.package.id)
                .await;
            self.rollback_lifecycle_install(&available.package.id, previous_package)
                .await;
            return Err(map_extension_installation_error(error));
        }
        Ok(response_with_payload(
            Some(package_ref),
            LifecyclePhase::Installed,
            json!({
                "installed": true,
                "visible_capability_ids": visible_capability_ids(&available).iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            }),
        ))
    }

    async fn activate(
        &self,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(&package_ref)?;
        self.installation_store
            .set_activation_state(&installation_id, ExtensionActivationState::Enabled)
            .await
            .map_err(map_extension_installation_error)?;
        let mut lifecycle = self.lifecycle_service.lock().await;
        let package = lifecycle
            .registry()
            .get_extension(&extension_id)
            .cloned()
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: format!("extension {} is not installed", extension_id.as_str()),
            })?;
        replace_active_extension(&self.active_registry, package)?;
        lifecycle.enable(&extension_id).await.map_err(|error| {
            remove_active_extension(&self.active_registry, &extension_id);
            map_extension_error(error)
        })?;
        Ok(response_with_payload(
            Some(package_ref),
            LifecyclePhase::Active,
            json!({ "activated": true }),
        ))
    }

    async fn remove(
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
        let mut lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .remove(&extension_id)
            .await
            .map_err(map_extension_error)?;
        remove_active_extension(&self.active_registry, &extension_id);
        Ok(response_with_payload(
            Some(package_ref),
            LifecyclePhase::Removed,
            json!({ "removed": true }),
        ))
    }

    async fn rollback_lifecycle_install(
        &self,
        extension_id: &ExtensionId,
        previous_package: Option<ironclaw_extensions::ExtensionPackage>,
    ) {
        let mut lifecycle = self.lifecycle_service.lock().await;
        if let Some(package) = previous_package {
            let _ = lifecycle.update(package).await;
        } else {
            let _ = lifecycle.remove(extension_id).await;
        }
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

#[derive(Clone)]
pub(crate) struct RebornLocalLifecycleFacade {
    skill_management: Arc<RebornLocalSkillManagementPort>,
    extension_management: Option<Arc<RebornLocalExtensionManagementPort>>,
}

impl RebornLocalLifecycleFacade {
    pub(crate) fn new(skill_management: Arc<RebornLocalSkillManagementPort>) -> Self {
        Self {
            skill_management,
            extension_management: None,
        }
    }

    pub(crate) fn with_extension_management(
        mut self,
        extension_management: Arc<RebornLocalExtensionManagementPort>,
    ) -> Self {
        self.extension_management = Some(extension_management);
        self
    }

    async fn execute_action(
        &self,
        action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        match action {
            LifecycleProductAction::SkillSearch { query } => {
                let result = self
                    .skill_management
                    .search(&query, SKILL_SEARCH_RESULT_LIMIT)
                    .await?;
                let matched_skills: Vec<Value> =
                    result.skills.into_iter().map(skill_json).collect();
                let count = matched_skills.len();
                Ok(response_with_payload(
                    None,
                    LifecyclePhase::Installed,
                    json!({
                        "skills": matched_skills,
                        "count": count,
                        "limit": SKILL_SEARCH_RESULT_LIMIT,
                        "truncated": result.truncated,
                    }),
                ))
            }
            LifecycleProductAction::SkillInstall { name, content } => {
                let installed = self
                    .skill_management
                    .install(name.as_ref().map(LifecyclePackageId::as_str), &content)
                    .await?;
                Ok(response_with_payload(
                    Some(skill_package_ref(&installed.name)?),
                    LifecyclePhase::Installed,
                    json!({
                        "installed": true,
                        "name": installed.name,
                    }),
                ))
            }
            LifecycleProductAction::SkillRemove { package_ref } => {
                package_ref.require_kind(LifecyclePackageKind::Skill)?;
                let removed = self
                    .skill_management
                    .remove(package_ref.id.as_str())
                    .await?;
                Ok(response_with_payload(
                    Some(skill_package_ref(&removed.name)?),
                    LifecyclePhase::Removed,
                    json!({
                        "removed": true,
                        "name": removed.name,
                    }),
                ))
            }
            LifecycleProductAction::ExtensionSearch { query } => {
                let Some(extension_management) = &self.extension_management else {
                    return unsupported_projection(None);
                };
                extension_management.search(&query).await
            }
            LifecycleProductAction::ExtensionInstall { package_ref } => {
                let Some(extension_management) = &self.extension_management else {
                    return unsupported_projection(Some(package_ref));
                };
                extension_management.install(package_ref).await
            }
            LifecycleProductAction::ExtensionActivate { package_ref } => {
                let Some(extension_management) = &self.extension_management else {
                    return unsupported_projection(Some(package_ref));
                };
                extension_management.activate(package_ref).await
            }
            LifecycleProductAction::ExtensionRemove { package_ref } => {
                let Some(extension_management) = &self.extension_management else {
                    return unsupported_projection(Some(package_ref));
                };
                extension_management.remove(package_ref).await
            }
            LifecycleProductAction::ExtensionAuth { package_ref }
            | LifecycleProductAction::ExtensionConfigure { package_ref, .. } => {
                unsupported_projection(Some(package_ref.clone()))
            }
        }
    }
}

#[async_trait]
impl LifecycleProductFacade for RebornLocalLifecycleFacade {
    async fn execute(
        &self,
        _context: LifecycleProductContext,
        action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        self.execute_action(action).await
    }

    async fn project_package(
        &self,
        _context: LifecycleProductContext,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        unsupported_projection(Some(package_ref))
    }
}

fn skill_package_ref(name: &str) -> Result<LifecyclePackageRef, ProductWorkflowError> {
    LifecyclePackageRef::new(LifecyclePackageKind::Skill, name)
}

fn response_with_payload(
    package_ref: Option<LifecyclePackageRef>,
    phase: LifecyclePhase,
    payload: Value,
) -> LifecycleProductResponse {
    LifecycleProductResponse {
        package_ref,
        phase,
        blockers: Vec::new(),
        message: None,
        payload: Some(payload),
    }
}

fn skill_json(skill: ironclaw_skills::SkillSummary) -> Value {
    json!({
        "name": skill.name,
        "version": skill.version,
        "description": skill.description,
        "source": skill.source.as_str(),
        "keywords": skill.keywords,
        "tags": skill.tags,
        "requires_skills": skill.requires_skills,
    })
}

fn unsupported_projection(
    package_ref: Option<LifecyclePackageRef>,
) -> Result<LifecycleProductResponse, ProductWorkflowError> {
    Ok(LifecycleProductResponse::projection(
        package_ref,
        LifecyclePhase::UnsupportedOrLegacy,
        vec![LifecycleReadinessBlocker::runtime(Some(
            "extension_auth_and_configure_not_yet_wired".to_string(),
        ))?],
    ))
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
    package: ironclaw_extensions::ExtensionPackage,
) -> Result<(), ProductWorkflowError> {
    active_registry.upsert(package).map_err(map_extension_error)
}

fn remove_active_extension(active_registry: &SharedExtensionRegistry, extension_id: &ExtensionId) {
    active_registry.remove(extension_id);
}

fn map_extension_error(error: ironclaw_extensions::ExtensionError) -> ProductWorkflowError {
    match error {
        ironclaw_extensions::ExtensionError::Filesystem(_)
        | ironclaw_extensions::ExtensionError::LifecycleEventSink { .. } => {
            ProductWorkflowError::Transient {
                reason: error.to_string(),
            }
        }
        _ => ProductWorkflowError::InvalidBindingRequest {
            reason: error.to_string(),
        },
    }
}

fn map_extension_installation_error(
    error: ironclaw_extensions::ExtensionInstallationError,
) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    }
}

fn map_skill_error(error: SkillManagementError) -> ProductWorkflowError {
    match error.kind() {
        SkillManagementErrorKind::InvalidInput
        | SkillManagementErrorKind::NotFound
        | SkillManagementErrorKind::Conflict
        | SkillManagementErrorKind::InvalidSkill => ProductWorkflowError::InvalidBindingRequest {
            reason: "skill management request rejected".to_string(),
        },
        SkillManagementErrorKind::FilesystemDenied => ProductWorkflowError::BindingAccessDenied,
        SkillManagementErrorKind::Resource => ProductWorkflowError::Transient {
            reason: "skill management resource unavailable".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extensions::{ExtensionManifest, ExtensionPackage};
    use ironclaw_host_api::{
        HostPath, HostPortCatalog, MountAlias, MountGrant, MountPermissions, TenantId, VirtualPath,
    };

    use crate::available_extensions::{AvailableExtensionAsset, AvailableExtensionPackage};
    use ironclaw_product_workflow::LifecycleProductSurfaceContext;

    #[tokio::test]
    async fn skill_lifecycle_facade_installs_lists_and_removes_via_skill_management() {
        let (_dir, storage_root, facade) = lifecycle_fixture();

        let install = facade
            .execute_action(LifecycleProductAction::SkillInstall {
                name: None,
                content:
                    "---\nname: lifecycle-skill\ndescription: lifecycle test\n---\nUse lifecycle.\n"
                        .to_string(),
            })
            .await
            .expect("install skill");
        assert_eq!(install.phase, LifecyclePhase::Installed);
        assert_eq!(
            install.package_ref,
            Some(
                LifecyclePackageRef::new(LifecyclePackageKind::Skill, "lifecycle-skill")
                    .expect("valid skill ref")
            )
        );
        assert!(
            storage_root
                .join("skills/lifecycle-skill/SKILL.md")
                .exists()
        );

        let list = facade
            .execute_action(LifecycleProductAction::SkillSearch {
                query: "lifecycle".to_string(),
            })
            .await
            .expect("list skills");
        assert_eq!(list.phase, LifecyclePhase::Installed);
        assert_eq!(
            list.payload
                .as_ref()
                .and_then(|payload| payload.get("count"))
                .and_then(Value::as_u64),
            Some(1)
        );

        for index in 0..55 {
            facade
                .execute_action(LifecycleProductAction::SkillInstall {
                    name: Some(
                        LifecyclePackageId::new(format!("bulk-skill-{index:02}"))
                            .expect("valid skill id"),
                    ),
                    content: format!(
                        "---\nname: bulk-skill-{index:02}\ndescription: bulk test\n---\nUse bulk.\n"
                    ),
                })
                .await
                .expect("install bulk skill");
        }

        let all_skills = facade
            .execute_action(LifecycleProductAction::SkillSearch {
                query: String::new(),
            })
            .await
            .expect("list all skills");
        let payload = all_skills.payload.as_ref().expect("skill search payload");
        assert_eq!(payload.get("count").and_then(Value::as_u64), Some(50));
        assert_eq!(payload.get("limit").and_then(Value::as_u64), Some(50));
        assert_eq!(
            payload.get("truncated").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            payload
                .get("skills")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(50)
        );

        let wrong_kind = facade
            .execute_action(LifecycleProductAction::SkillRemove {
                package_ref: LifecyclePackageRef::new(
                    LifecyclePackageKind::Extension,
                    "lifecycle-skill",
                )
                .expect("valid extension ref"),
            })
            .await
            .expect_err("skill remove must reject non-skill package refs");
        assert!(matches!(
            wrong_kind,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert!(
            storage_root
                .join("skills/lifecycle-skill/SKILL.md")
                .exists()
        );

        let remove = facade
            .execute_action(LifecycleProductAction::SkillRemove {
                package_ref: LifecyclePackageRef::new(
                    LifecyclePackageKind::Skill,
                    "lifecycle-skill",
                )
                .expect("valid skill ref"),
            })
            .await
            .expect("remove skill");
        assert_eq!(remove.phase, LifecyclePhase::Removed);
        assert!(
            !storage_root
                .join("skills/lifecycle-skill/SKILL.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn skill_lifecycle_facade_serializes_concurrent_install_and_remove() {
        let (_dir, storage_root, facade) = lifecycle_fixture();

        let facade_a = facade.clone();
        let facade_b = facade.clone();
        let install_a = facade_a.execute_action(LifecycleProductAction::SkillInstall {
            name: Some(LifecyclePackageId::new("concurrent-a").expect("valid skill id")),
            content: skill_content("concurrent-a"),
        });
        let install_b = facade_b.execute_action(LifecycleProductAction::SkillInstall {
            name: Some(LifecyclePackageId::new("concurrent-b").expect("valid skill id")),
            content: skill_content("concurrent-b"),
        });
        let (installed_a, installed_b) = tokio::join!(install_a, install_b);
        installed_a.expect("install concurrent-a");
        installed_b.expect("install concurrent-b");

        let facade_a = facade.clone();
        let remove_a = facade_a.execute_action(LifecycleProductAction::SkillRemove {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Skill, "concurrent-a")
                .expect("valid skill ref"),
        });
        let remove_b = facade.execute_action(LifecycleProductAction::SkillRemove {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Skill, "concurrent-b")
                .expect("valid skill ref"),
        });
        let (removed_a, removed_b) = tokio::join!(remove_a, remove_b);
        removed_a.expect("remove concurrent-a");
        removed_b.expect("remove concurrent-b");

        assert!(!storage_root.join("skills/concurrent-a/SKILL.md").exists());
        assert!(!storage_root.join("skills/concurrent-b/SKILL.md").exists());
    }

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
            .and_then(Value::as_array)
            .expect("extension summaries");
        assert_eq!(extensions.len(), 1);
        let visible_ids = extensions[0]
            .get("visible_read_only_capability_ids")
            .and_then(Value::as_array)
            .expect("visible read-only ids");
        assert_eq!(
            visible_ids
                .iter()
                .filter_map(Value::as_str)
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
    async fn skill_lifecycle_facade_maps_skill_management_errors() {
        let (_dir, _storage_root, facade) = lifecycle_fixture();

        let invalid_install = facade
            .execute_action(LifecycleProductAction::SkillInstall {
                name: Some(LifecyclePackageId::new("broken-skill").expect("valid skill id")),
                content: "not a skill manifest".to_string(),
            })
            .await
            .expect_err("invalid skill content should fail");
        assert!(matches!(
            invalid_install,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));

        let missing_remove = facade
            .execute_action(LifecycleProductAction::SkillRemove {
                package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Skill, "missing-skill")
                    .expect("valid skill ref"),
            })
            .await
            .expect_err("missing skill remove should fail");
        assert!(matches!(
            missing_remove,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
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

    #[tokio::test]
    async fn skill_only_facade_rejects_extension_search() {
        let (_dir, _storage_root, facade) = lifecycle_fixture();

        let response = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "fixture".to_string(),
                },
            )
            .await
            .expect("unsupported extension search");

        assert_unsupported_extension_response(response);
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
            manifest_toml: MANIFEST,
            package,
            assets: vec![
                AvailableExtensionAsset {
                    path: "manifest.toml",
                    bytes: MANIFEST.as_bytes(),
                },
                AvailableExtensionAsset {
                    path: "wasm/fixture.wasm",
                    bytes: b"\0asm\x01\0\0\0",
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

    fn extension_lifecycle_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        RebornLocalLifecycleFacade,
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
        let skill_management = Arc::new(RebornLocalSkillManagementPort::new(
            UserId::new("lifecycle-owner").expect("valid user"),
            Arc::clone(&filesystem),
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/skills").expect("valid alias"),
                VirtualPath::new("/projects/skills").expect("valid path"),
                MountPermissions::read_write_list_delete(),
            )])
            .expect("valid mount view"),
        ));
        let active_registry = Arc::new(SharedExtensionRegistry::new(
            ironclaw_extensions::ExtensionRegistry::new(),
        ));
        let extension_management = Arc::new(RebornLocalExtensionManagementPort::new(
            Arc::clone(&filesystem),
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            Arc::new(ironclaw_extensions::InMemoryExtensionInstallationStore::default()),
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                ironclaw_extensions::ExtensionRegistry::new(),
            ))),
            Arc::clone(&active_registry),
        ));
        let facade = RebornLocalLifecycleFacade::new(skill_management)
            .with_extension_management(extension_management);
        (dir, storage_root, facade, active_registry)
    }

    fn lifecycle_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        RebornLocalLifecycleFacade,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(&storage_root).expect("storage root");

        let mut filesystem = LocalFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        let skill_management = Arc::new(RebornLocalSkillManagementPort::new(
            UserId::new("lifecycle-owner").expect("valid user"),
            Arc::new(filesystem),
            MountView::new(vec![
                MountGrant::new(
                    MountAlias::new("/skills").expect("valid alias"),
                    VirtualPath::new("/projects/skills").expect("valid path"),
                    MountPermissions::read_write_list_delete(),
                ),
                MountGrant::new(
                    MountAlias::new("/system/skills").expect("valid alias"),
                    VirtualPath::new("/projects/system/skills").expect("valid path"),
                    MountPermissions::read_only(),
                ),
            ])
            .expect("valid mount view"),
        ));
        let facade = RebornLocalLifecycleFacade::new(skill_management);
        (dir, storage_root, facade)
    }

    fn skill_content(name: &str) -> String {
        format!("---\nname: {name}\ndescription: lifecycle test\n---\nUse lifecycle.\n")
    }
}

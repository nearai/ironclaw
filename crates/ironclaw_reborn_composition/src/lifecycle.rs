use std::{path::PathBuf, sync::Arc};

use crate::local_dev_mounts::skill_management_mount_view;
use async_trait::async_trait;
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::{HostPath, InvocationId, MountView, ResourceScope, UserId, VirtualPath};
use ironclaw_product_workflow::{
    LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase,
    LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleReadinessBlocker,
    LifecycleSkillSource, LifecycleSkillSummary, ProductWorkflowError,
};
use ironclaw_skills::{
    SkillInstallRequest, SkillInstallSource, SkillManagementContext, SkillManagementError,
    SkillManagementErrorKind, SkillRemoveRequest, SkillSearchRequest, install_skill, list_skills,
    remove_skill, search_skills,
};

use crate::extension_lifecycle::RebornExtensionManagementPort;

const SKILL_SEARCH_RESULT_LIMIT: usize = 50;

#[derive(Clone)]
pub(crate) struct RebornLocalSkillManagementPort {
    owner_user_id: UserId,
    filesystem: Arc<dyn RootFilesystem>,
    skill_management_mounts: MountView,
}

impl RebornLocalSkillManagementPort {
    pub(crate) fn new(
        owner_user_id: UserId,
        filesystem: Arc<dyn RootFilesystem>,
        skill_management_mounts: MountView,
    ) -> Self {
        Self {
            owner_user_id,
            filesystem,
            skill_management_mounts,
        }
    }

    fn skill_context(&self) -> Result<SkillManagementContext, RebornLocalSkillManagementError> {
        let scope = ResourceScope::local_default(self.owner_user_id.clone(), InvocationId::new())
            .map_err(invalid_skill_context)?;
        Ok(SkillManagementContext::new(
            self.filesystem.clone(),
            self.skill_management_mounts.clone(),
            scope,
        ))
    }

    pub(crate) async fn list(
        &self,
    ) -> Result<Vec<ironclaw_skills::SkillSummary>, RebornLocalSkillManagementError> {
        let context = self.skill_context()?;
        Ok(list_skills(&context).await?)
    }

    pub(crate) async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<ironclaw_skills::SkillSearchResult, RebornLocalSkillManagementError> {
        let context = self.skill_context()?;
        Ok(search_skills(&context, SkillSearchRequest { query, limit }).await?)
    }

    async fn install(
        &self,
        name: Option<&str>,
        content: &str,
    ) -> Result<ironclaw_skills::SkillInstallResult, RebornLocalSkillManagementError> {
        let context = self.skill_context()?;
        Ok(install_skill(
            &context,
            SkillInstallRequest {
                name,
                content,
                files: &[],
                source: SkillInstallSource::User,
                source_url: None,
            },
        )
        .await?)
    }

    async fn remove(
        &self,
        name: &str,
    ) -> Result<ironclaw_skills::SkillRemoveResult, RebornLocalSkillManagementError> {
        let context = self.skill_context()?;
        Ok(remove_skill(&context, SkillRemoveRequest { name }).await?)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RebornLocalSkillManagementError {
    #[error("invalid skill management context: {reason}")]
    InvalidContext { reason: String },
    #[error("skill management failed: {0:?}")]
    Skill(SkillManagementError),
}

impl From<SkillManagementError> for RebornLocalSkillManagementError {
    fn from(error: SkillManagementError) -> Self {
        Self::Skill(error)
    }
}

pub(crate) fn build_local_skill_management_port<F>(
    owner_user_id: UserId,
    filesystem: Arc<F>,
) -> Result<Arc<RebornLocalSkillManagementPort>, crate::RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    let skill_management_mounts =
        skill_management_mount_view().map_err(|error| crate::RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let filesystem: Arc<dyn RootFilesystem> = filesystem;
    Ok(Arc::new(RebornLocalSkillManagementPort::new(
        owner_user_id,
        filesystem,
        skill_management_mounts,
    )))
}

pub(crate) fn build_existing_local_dev_skill_management_port(
    owner_id: impl Into<String>,
    local_dev_storage_root: impl Into<PathBuf>,
) -> Result<Option<Arc<RebornLocalSkillManagementPort>>, crate::RebornBuildError> {
    let owner_id = owner_id.into();
    let local_dev_storage_root = local_dev_storage_root.into();
    if !local_dev_storage_root.try_exists().map_err(|error| {
        crate::RebornBuildError::InvalidConfig {
            reason: format!("local-dev skill storage root could not be inspected: {error}"),
        }
    })? {
        return Ok(None);
    }
    if !local_dev_storage_root.is_dir() {
        return Err(crate::RebornBuildError::InvalidConfig {
            reason: "local-dev skill storage root is not a directory".to_string(),
        });
    }

    let mut filesystem = LocalFilesystem::new();
    filesystem.mount_local(
        VirtualPath::new("/projects")?,
        HostPath::from_path_buf(local_dev_storage_root),
    )?;
    let owner_user_id =
        UserId::new(owner_id).map_err(|error| crate::RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    build_local_skill_management_port(owner_user_id, Arc::new(filesystem)).map(Some)
}

fn invalid_skill_context(error: impl std::fmt::Display) -> RebornLocalSkillManagementError {
    RebornLocalSkillManagementError::InvalidContext {
        reason: error.to_string(),
    }
}

#[derive(Clone)]
pub(crate) struct RebornLocalLifecycleFacade {
    skill_management: Arc<RebornLocalSkillManagementPort>,
    extension_management: Option<Arc<RebornExtensionManagementPort>>,
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
        extension_management: Arc<RebornExtensionManagementPort>,
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
                    .await
                    .map_err(map_local_skill_management_error)?;
                let matched_skills = result
                    .skills
                    .into_iter()
                    .map(skill_summary)
                    .collect::<Result<Vec<_>, _>>()?;
                let count = matched_skills.len();
                Ok(response_with_payload(
                    None,
                    LifecyclePhase::Discovered,
                    LifecycleProductPayload::SkillSearch {
                        skills: matched_skills,
                        count,
                        limit: SKILL_SEARCH_RESULT_LIMIT,
                        truncated: result.truncated,
                    },
                ))
            }
            LifecycleProductAction::SkillInstall { name, content } => {
                let installed = self
                    .skill_management
                    .install(name.as_ref().map(LifecyclePackageId::as_str), &content)
                    .await
                    .map_err(map_local_skill_management_error)?;
                Ok(response_with_payload(
                    Some(skill_package_ref(&installed.name)?),
                    LifecyclePhase::Installed,
                    LifecycleProductPayload::SkillInstall {
                        installed: true,
                        name: LifecyclePackageId::new(installed.name)?,
                    },
                ))
            }
            LifecycleProductAction::SkillRemove { package_ref } => {
                package_ref.require_kind(LifecyclePackageKind::Skill)?;
                let removed = self
                    .skill_management
                    .remove(package_ref.id.as_str())
                    .await
                    .map_err(map_local_skill_management_error)?;
                Ok(response_with_payload(
                    Some(skill_package_ref(&removed.name)?),
                    LifecyclePhase::Removed,
                    LifecycleProductPayload::SkillRemove {
                        removed: true,
                        name: LifecyclePackageId::new(removed.name)?,
                    },
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
                unsupported_extension_auth_configure_projection(Some(package_ref))
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

pub(crate) fn response_with_payload(
    package_ref: Option<LifecyclePackageRef>,
    phase: LifecyclePhase,
    payload: LifecycleProductPayload,
) -> LifecycleProductResponse {
    LifecycleProductResponse {
        package_ref,
        phase,
        blockers: Vec::new(),
        message: None,
        payload: Some(payload),
    }
}

fn skill_summary(
    skill: ironclaw_skills::SkillSummary,
) -> Result<LifecycleSkillSummary, ProductWorkflowError> {
    Ok(LifecycleSkillSummary {
        name: LifecyclePackageId::new(skill.name)?,
        version: skill.version,
        description: skill.description,
        source: match skill.source {
            ironclaw_skills::ManagedSkillSource::System => LifecycleSkillSource::System,
            ironclaw_skills::ManagedSkillSource::User
            | ironclaw_skills::ManagedSkillSource::Installed => LifecycleSkillSource::User,
        },
        keywords: skill.keywords,
        tags: skill.tags,
        requires_skills: skill.requires_skills,
    })
}

fn unsupported_projection(
    package_ref: Option<LifecyclePackageRef>,
) -> Result<LifecycleProductResponse, ProductWorkflowError> {
    Ok(LifecycleProductResponse::projection(
        package_ref,
        LifecyclePhase::UnsupportedOrLegacy,
        vec![LifecycleReadinessBlocker::runtime(Some(
            "extension_lifecycle_local_runtime_unwired".to_string(),
        ))?],
    ))
}

fn unsupported_extension_auth_configure_projection(
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

fn map_skill_error(error: SkillManagementError) -> ProductWorkflowError {
    match error.kind() {
        SkillManagementErrorKind::InvalidInput
        | SkillManagementErrorKind::NotFound
        | SkillManagementErrorKind::Conflict
        | SkillManagementErrorKind::InvalidSkill => ProductWorkflowError::InvalidBindingRequest {
            reason: error
                .reason()
                .unwrap_or("skill management request rejected")
                .to_string(),
        },
        SkillManagementErrorKind::FilesystemDenied => ProductWorkflowError::BindingAccessDenied,
        SkillManagementErrorKind::Resource => ProductWorkflowError::Transient {
            reason: "skill management resource unavailable".to_string(),
        },
    }
}

fn map_local_skill_management_error(
    error: RebornLocalSkillManagementError,
) -> ProductWorkflowError {
    match error {
        RebornLocalSkillManagementError::InvalidContext { reason } => {
            ProductWorkflowError::InvalidBindingRequest { reason }
        }
        RebornLocalSkillManagementError::Skill(error) => map_skill_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::LocalFilesystem;
    use ironclaw_host_api::{HostPath, MountAlias, MountGrant, MountPermissions, VirtualPath};

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
        assert_eq!(list.phase, LifecyclePhase::Discovered);
        let Some(LifecycleProductPayload::SkillSearch { count, .. }) = list.payload.as_ref() else {
            panic!("expected skill search payload");
        };
        assert_eq!(*count, 1);

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
        let Some(LifecycleProductPayload::SkillSearch {
            skills,
            count,
            limit,
            truncated,
        }) = all_skills.payload.as_ref()
        else {
            panic!("expected skill search payload");
        };
        assert_eq!(*count, 50);
        assert_eq!(*limit, 50);
        assert!(*truncated);
        assert_eq!(skills.len(), 50);

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
    async fn skill_lifecycle_facade_maps_skill_management_errors() {
        let (_dir, _storage_root, facade) = lifecycle_fixture();

        let invalid_install = facade
            .execute_action(LifecycleProductAction::SkillInstall {
                name: Some(LifecyclePackageId::new("broken-skill").expect("valid skill id")),
                content: "---\nname: broken-skill\n\nmissing closing delimiter".to_string(),
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

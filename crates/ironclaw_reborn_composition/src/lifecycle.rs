use std::{path::PathBuf, sync::Arc};

use crate::local_dev_mounts::scoped_skill_management_mount_view;
use async_trait::async_trait;
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::{
    CredentialStageError, HostApiError, HostPath, InvocationId, MountView, ResourceScope,
    RuntimeHttpEgress, UserId, VirtualPath,
};
use ironclaw_product_workflow::{
    LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase,
    LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleReadinessBlocker,
    LifecycleSkillSource, LifecycleSkillSummary, ProductWorkflowError,
};
use ironclaw_skills::{
    LearnedSkillProvenance, SkillInstallRequest, SkillInstallSource, SkillManagementContext,
    SkillManagementError, SkillManagementErrorKind, SkillRemoveRequest, SkillSearchRequest,
    SkillUpdateRequest, install_skill, list_skills, read_learned_provenance, read_skill_content,
    remove_skill, search_skills, skill_is_bundle, update_skill, write_learned_provenance,
};

use crate::extension_activation_credentials::RuntimeExtensionActivationCredentialGate;
use crate::extension_lifecycle::RebornLocalExtensionManagementPort;
use crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionService;

const SKILL_SEARCH_RESULT_LIMIT: usize = 50;

type SkillManagementMountResolver =
    dyn Fn(&ResourceScope) -> Result<MountView, HostApiError> + Send + Sync;

#[derive(Clone)]
pub(crate) struct RebornLocalSkillManagementPort {
    owner_user_id: UserId,
    filesystem: Arc<dyn RootFilesystem>,
    skill_management_mount_resolver: Arc<SkillManagementMountResolver>,
}

impl RebornLocalSkillManagementPort {
    #[cfg(test)]
    pub(crate) fn new(
        owner_user_id: UserId,
        filesystem: Arc<dyn RootFilesystem>,
        skill_management_mounts: MountView,
    ) -> Self {
        let resolver = Arc::new(move |_scope: &ResourceScope| Ok(skill_management_mounts.clone()));
        Self::new_with_mount_resolver(owner_user_id, filesystem, resolver)
    }

    pub(crate) fn new_with_mount_resolver(
        owner_user_id: UserId,
        filesystem: Arc<dyn RootFilesystem>,
        skill_management_mount_resolver: Arc<SkillManagementMountResolver>,
    ) -> Self {
        Self {
            owner_user_id,
            filesystem,
            skill_management_mount_resolver,
        }
    }

    pub(crate) fn owner_scope(&self) -> Result<ResourceScope, RebornLocalSkillManagementError> {
        ResourceScope::local_default(self.owner_user_id.clone(), InvocationId::new())
            .map_err(invalid_skill_context)
    }

    fn skill_context_for_scope(
        &self,
        scope: ResourceScope,
    ) -> Result<SkillManagementContext, RebornLocalSkillManagementError> {
        let mounts =
            (self.skill_management_mount_resolver)(&scope).map_err(invalid_skill_context)?;
        Ok(SkillManagementContext::new(
            self.filesystem.clone(),
            mounts,
            scope,
        ))
    }

    pub(crate) async fn list_for_scope(
        &self,
        scope: ResourceScope,
    ) -> Result<Vec<ironclaw_skills::SkillSummary>, RebornLocalSkillManagementError> {
        let context = self.skill_context_for_scope(scope)?;
        Ok(list_skills(&context).await?)
    }

    pub(crate) async fn search_for_scope(
        &self,
        scope: ResourceScope,
        query: &str,
        limit: usize,
    ) -> Result<ironclaw_skills::SkillSearchResult, RebornLocalSkillManagementError> {
        let context = self.skill_context_for_scope(scope)?;
        Ok(search_skills(&context, SkillSearchRequest { query, limit }).await?)
    }

    pub(crate) async fn read_content_for_scope(
        &self,
        scope: ResourceScope,
        name: &str,
    ) -> Result<ironclaw_skills::SkillContentResult, RebornLocalSkillManagementError> {
        let context = self.skill_context_for_scope(scope)?;
        Ok(read_skill_content(&context, ironclaw_skills::SkillContentRequest { name }).await?)
    }

    pub(crate) async fn update_for_scope(
        &self,
        scope: ResourceScope,
        name: &str,
        content: &str,
    ) -> Result<ironclaw_skills::SkillUpdateResult, RebornLocalSkillManagementError> {
        let context = self.skill_context_for_scope(scope)?;
        Ok(update_skill(&context, SkillUpdateRequest { name, content }).await?)
    }

    pub(crate) async fn install_for_scope(
        &self,
        scope: ResourceScope,
        name: Option<&str>,
        content: &str,
    ) -> Result<ironclaw_skills::SkillInstallResult, RebornLocalSkillManagementError> {
        let context = self.skill_context_for_scope(scope)?;
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

    pub(crate) async fn remove_for_scope(
        &self,
        scope: ResourceScope,
        name: &str,
    ) -> Result<ironclaw_skills::SkillRemoveResult, RebornLocalSkillManagementError> {
        let context = self.skill_context_for_scope(scope)?;
        Ok(remove_skill(&context, SkillRemoveRequest { name }).await?)
    }

    /// Read the learning-sink provenance sidecar for a skill, if any. `None`
    /// means it is not a machine-learned skill (human-built, installed, or never
    /// learned) — which the gate treats as "not auto-evolvable".
    pub(crate) async fn read_provenance_for_scope(
        &self,
        scope: ResourceScope,
        name: &str,
    ) -> Result<Option<LearnedSkillProvenance>, RebornLocalSkillManagementError> {
        let context = self.skill_context_for_scope(scope)?;
        Ok(read_learned_provenance(&context, name).await?)
    }

    /// Write the machine-baseline provenance sidecar (the body + manifest hash the
    /// overwrite gate compares against). Intended for the machine learning path
    /// (`PortSkillWriter`) and the approve path, which both legitimately
    /// (re-)record the baseline. This is `pub(crate)` with no typed caller guard,
    /// so it is an intent contract, not an enforced one: a plain human EDIT must
    /// never route here, or it would refresh the baseline and hide itself from
    /// the gate.
    pub(crate) async fn write_provenance_for_scope(
        &self,
        scope: ResourceScope,
        name: &str,
        provenance: &LearnedSkillProvenance,
    ) -> Result<(), RebornLocalSkillManagementError> {
        let context = self.skill_context_for_scope(scope)?;
        write_learned_provenance(&context, name, provenance).await?;
        Ok(())
    }

    /// True iff the skill is a multi-file bundle (sibling files beyond `SKILL.md`
    /// + dotfiles) — the gate must not auto-overwrite its `SKILL.md`.
    pub(crate) async fn is_bundle_for_scope(
        &self,
        scope: ResourceScope,
        name: &str,
    ) -> Result<bool, RebornLocalSkillManagementError> {
        let context = self.skill_context_for_scope(scope)?;
        Ok(skill_is_bundle(&context, name).await?)
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
    let mount_resolver: Arc<SkillManagementMountResolver> =
        Arc::new(scoped_skill_management_mount_view);
    let filesystem: Arc<dyn RootFilesystem> = filesystem;
    Ok(Arc::new(
        RebornLocalSkillManagementPort::new_with_mount_resolver(
            owner_user_id,
            filesystem,
            mount_resolver,
        ),
    ))
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
    extension_management: Option<Arc<RebornLocalExtensionManagementPort>>,
    runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    credential_accounts: Option<Arc<dyn RuntimeCredentialAccountSelectionService>>,
}

impl RebornLocalLifecycleFacade {
    pub(crate) fn new(skill_management: Arc<RebornLocalSkillManagementPort>) -> Self {
        Self {
            skill_management,
            extension_management: None,
            runtime_http_egress: None,
            credential_accounts: None,
        }
    }

    pub(crate) fn with_extension_management(
        mut self,
        extension_management: Arc<RebornLocalExtensionManagementPort>,
    ) -> Self {
        self.extension_management = Some(extension_management);
        self
    }

    pub(crate) fn with_runtime_http_egress(
        mut self,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    ) -> Self {
        self.runtime_http_egress = Some(runtime_http_egress);
        self
    }

    pub(crate) fn with_runtime_credential_accounts(
        mut self,
        credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
    ) -> Self {
        self.credential_accounts = Some(credential_accounts);
        self
    }

    async fn execute_action(
        &self,
        context: LifecycleProductContext,
        action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        match action {
            LifecycleProductAction::SkillSearch { query } => {
                let scope = self
                    .skill_management
                    .owner_scope()
                    .map_err(map_local_skill_management_error)?;
                let result = self
                    .skill_management
                    .search_for_scope(scope, &query, SKILL_SEARCH_RESULT_LIMIT)
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
                let scope = self
                    .skill_management
                    .owner_scope()
                    .map_err(map_local_skill_management_error)?;
                let installed = self
                    .skill_management
                    .install_for_scope(
                        scope,
                        name.as_ref().map(LifecyclePackageId::as_str),
                        &content,
                    )
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
                let scope = self
                    .skill_management
                    .owner_scope()
                    .map_err(map_local_skill_management_error)?;
                let removed = self
                    .skill_management
                    .remove_for_scope(scope, package_ref.id.as_str())
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
                let credential_gate = if matches!(&context, LifecycleProductContext::Surface(_)) {
                    if let Some(credential_accounts) = &self.credential_accounts {
                        Some(RuntimeExtensionActivationCredentialGate::new(
                            lifecycle_resource_scope(&context)?,
                            credential_accounts.clone(),
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                extension_management
                    .search(&query, credential_gate.as_ref())
                    .await
            }
            LifecycleProductAction::ExtensionList => {
                let Some(extension_management) = &self.extension_management else {
                    return unsupported_projection(None);
                };
                extension_management.list_installed().await
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
                let credential_gate = self
                    .extension_activation_credential_gate(
                        &context,
                        extension_management,
                        &package_ref,
                    )
                    .await?;
                if extension_management
                    .package_requires_hosted_mcp_discovery(&package_ref)
                    .await?
                {
                    let Some(runtime_http_egress) = self.runtime_http_egress.clone() else {
                        return Err(ProductWorkflowError::InvalidBindingRequest {
                            reason: format!(
                                "extension {} requires hosted MCP schema discovery and cannot be activated through the static lifecycle facade",
                                package_ref.id
                            ),
                        });
                    };
                    let scope = lifecycle_resource_scope(&context)?;
                    let mode =
                        crate::extension_lifecycle::ExtensionActivationMode::HostedMcpDiscovery {
                            scope,
                            runtime_http_egress,
                        };
                    return match credential_gate {
                        Some(credential_gate) => {
                            extension_management
                                .activate_with_credential_gate(package_ref, mode, credential_gate)
                                .await
                        }
                        None => extension_management.activate(package_ref, mode).await,
                    };
                }
                let mode = crate::extension_lifecycle::ExtensionActivationMode::Static;
                match credential_gate {
                    Some(credential_gate) => {
                        extension_management
                            .activate_with_credential_gate(package_ref, mode, credential_gate)
                            .await
                    }
                    None => extension_management.activate(package_ref, mode).await,
                }
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

    async fn extension_activation_credential_gate(
        &self,
        context: &LifecycleProductContext,
        extension_management: &RebornLocalExtensionManagementPort,
        package_ref: &LifecyclePackageRef,
    ) -> Result<Option<RuntimeExtensionActivationCredentialGate>, ProductWorkflowError> {
        let requirements = extension_management
            .activation_credential_requirements(package_ref)
            .await?;
        if requirements.is_empty() {
            return Ok(None);
        }
        let Some(credential_accounts) = &self.credential_accounts else {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} requires product auth credentials before activation",
                    package_ref.id
                ),
            });
        };
        let scope = lifecycle_resource_scope(context)?;
        let credential_gate =
            RuntimeExtensionActivationCredentialGate::new(scope, Arc::clone(credential_accounts));
        let missing_requirements = credential_gate
            .missing_requirements(requirements)
            .await
            .map_err(map_lifecycle_credential_stage_error)?;
        if missing_requirements.is_empty() {
            return Ok(Some(credential_gate));
        }
        Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!(
                "extension {} requires product auth credentials before activation",
                package_ref.id
            ),
        })
    }
}

#[async_trait]
impl LifecycleProductFacade for RebornLocalLifecycleFacade {
    async fn execute(
        &self,
        context: LifecycleProductContext,
        action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        self.execute_action(context, action).await
    }

    async fn project_package(
        &self,
        _context: LifecycleProductContext,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        if package_ref.kind == LifecyclePackageKind::Extension {
            let Some(extension_management) = &self.extension_management else {
                return unsupported_projection(Some(package_ref));
            };
            return extension_management.project(package_ref).await;
        }
        unsupported_projection(Some(package_ref))
    }
}

fn skill_package_ref(name: &str) -> Result<LifecyclePackageRef, ProductWorkflowError> {
    LifecyclePackageRef::new(LifecyclePackageKind::Skill, name)
}

fn lifecycle_resource_scope(
    context: &LifecycleProductContext,
) -> Result<ResourceScope, ProductWorkflowError> {
    let LifecycleProductContext::Surface(context) = context else {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: "extension lifecycle activation requires a surface caller".to_string(),
        });
    };
    Ok(ResourceScope {
        tenant_id: context.tenant_id.clone(),
        user_id: context.user_id.clone(),
        agent_id: context.agent_id.clone(),
        project_id: context.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    })
}

fn map_lifecycle_credential_stage_error(error: CredentialStageError) -> ProductWorkflowError {
    match error {
        CredentialStageError::AuthRequired => ProductWorkflowError::InvalidBindingRequest {
            reason: "extension requires product auth credentials before activation".to_string(),
        },
        CredentialStageError::Backend => ProductWorkflowError::InvalidBindingRequest {
            reason: "extension product auth credential state is invalid".to_string(),
        },
    }
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
    use ironclaw_host_api::{
        AgentId, HostPath, MountAlias, MountGrant, MountPermissions, ProjectId, TenantId,
        VirtualPath,
    };
    use ironclaw_product_workflow::LifecycleProductSurfaceContext;

    #[tokio::test]
    async fn skill_lifecycle_facade_installs_lists_and_removes_via_skill_management() {
        let (_dir, storage_root, facade) = lifecycle_fixture();

        let install = facade
            .execute_action(lifecycle_test_context(), LifecycleProductAction::SkillInstall {
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
            .execute_action(
                lifecycle_test_context(),
                LifecycleProductAction::SkillSearch {
                    query: "lifecycle".to_string(),
                },
            )
            .await
            .expect("list skills");
        assert_eq!(list.phase, LifecyclePhase::Discovered);
        let Some(LifecycleProductPayload::SkillSearch { count, .. }) = list.payload.as_ref() else {
            panic!("expected skill search payload");
        };
        assert_eq!(*count, 1);

        for index in 0..55 {
            facade
                .execute_action(lifecycle_test_context(), LifecycleProductAction::SkillInstall {
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
            .execute_action(
                lifecycle_test_context(),
                LifecycleProductAction::SkillSearch {
                    query: String::new(),
                },
            )
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
            .execute_action(
                lifecycle_test_context(),
                LifecycleProductAction::SkillRemove {
                    package_ref: LifecyclePackageRef::new(
                        LifecyclePackageKind::Extension,
                        "lifecycle-skill",
                    )
                    .expect("valid extension ref"),
                },
            )
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
            .execute_action(
                lifecycle_test_context(),
                LifecycleProductAction::SkillRemove {
                    package_ref: LifecyclePackageRef::new(
                        LifecyclePackageKind::Skill,
                        "lifecycle-skill",
                    )
                    .expect("valid skill ref"),
                },
            )
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
    async fn default_skill_management_port_isolates_user_skill_roots_by_scope() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/skills/system-helper"))
            .expect("system skill dir");
        std::fs::write(
            storage_root.join("system/skills/system-helper/SKILL.md"),
            skill_content("system-helper"),
        )
        .expect("system skill");

        let mut filesystem = LocalFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        let skill_management = build_local_skill_management_port(
            UserId::new("runtime-owner").expect("valid user"),
            Arc::new(filesystem),
        )
        .expect("skill management port");
        let alice_scope = skill_management_test_scope("tenant-alpha", "alice");
        let bob_scope = skill_management_test_scope("tenant-alpha", "bob");

        skill_management
            .install_for_scope(
                alice_scope.clone(),
                Some("shared-name"),
                &skill_content("shared-name"),
            )
            .await
            .expect("alice installs skill");

        let alice_skills = skill_management
            .list_for_scope(alice_scope)
            .await
            .expect("alice lists skills");
        assert!(alice_skills.iter().any(|skill| skill.name == "shared-name"));
        assert!(
            alice_skills
                .iter()
                .any(|skill| skill.name == "system-helper")
        );

        let bob_skills = skill_management
            .list_for_scope(bob_scope)
            .await
            .expect("bob lists skills");
        assert!(!bob_skills.iter().any(|skill| skill.name == "shared-name"));
        assert!(bob_skills.iter().any(|skill| skill.name == "system-helper"));
        assert!(
            storage_root
                .join("tenants/tenant-alpha/users/alice/skills/shared-name/SKILL.md")
                .exists()
        );
        assert!(
            !storage_root
                .join("tenants/tenant-alpha/users/bob/skills/shared-name/SKILL.md")
                .exists()
        );
    }

    #[test]
    fn lifecycle_resource_scope_uses_surface_caller_identity() {
        let context = LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new("user-alpha").expect("user"),
            agent_id: Some(AgentId::new("agent-alpha").expect("agent")),
            project_id: Some(ProjectId::new("project-alpha").expect("project")),
        });

        let scope = lifecycle_resource_scope(&context).expect("surface scope");

        assert_eq!(scope.tenant_id.as_str(), "tenant-alpha");
        assert_eq!(scope.user_id.as_str(), "user-alpha");
        assert_eq!(
            scope.agent_id.as_ref().map(|id| id.as_str()),
            Some("agent-alpha")
        );
        assert_eq!(
            scope.project_id.as_ref().map(|id| id.as_str()),
            Some("project-alpha")
        );
        assert!(scope.thread_id.is_none());
    }

    #[tokio::test]
    async fn skill_lifecycle_facade_serializes_concurrent_install_and_remove() {
        let (_dir, storage_root, facade) = lifecycle_fixture();

        let facade_a = facade.clone();
        let facade_b = facade.clone();
        let install_a = facade_a.execute_action(
            lifecycle_test_context(),
            LifecycleProductAction::SkillInstall {
                name: Some(LifecyclePackageId::new("concurrent-a").expect("valid skill id")),
                content: skill_content("concurrent-a"),
            },
        );
        let install_b = facade_b.execute_action(
            lifecycle_test_context(),
            LifecycleProductAction::SkillInstall {
                name: Some(LifecyclePackageId::new("concurrent-b").expect("valid skill id")),
                content: skill_content("concurrent-b"),
            },
        );
        let (installed_a, installed_b) = tokio::join!(install_a, install_b);
        installed_a.expect("install concurrent-a");
        installed_b.expect("install concurrent-b");

        let facade_a = facade.clone();
        let remove_a = facade_a.execute_action(
            lifecycle_test_context(),
            LifecycleProductAction::SkillRemove {
                package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Skill, "concurrent-a")
                    .expect("valid skill ref"),
            },
        );
        let remove_b = facade.execute_action(
            lifecycle_test_context(),
            LifecycleProductAction::SkillRemove {
                package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Skill, "concurrent-b")
                    .expect("valid skill ref"),
            },
        );
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
            .execute_action(
                lifecycle_test_context(),
                LifecycleProductAction::SkillInstall {
                    name: Some(LifecyclePackageId::new("broken-skill").expect("valid skill id")),
                    content: "---\nname: broken-skill\n\nmissing closing delimiter".to_string(),
                },
            )
            .await
            .expect_err("invalid skill content should fail");
        assert!(matches!(
            invalid_install,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));

        let missing_remove = facade
            .execute_action(
                lifecycle_test_context(),
                LifecycleProductAction::SkillRemove {
                    package_ref: LifecyclePackageRef::new(
                        LifecyclePackageKind::Skill,
                        "missing-skill",
                    )
                    .expect("valid skill ref"),
                },
            )
            .await
            .expect_err("missing skill remove should fail");
        assert!(matches!(
            missing_remove,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    #[tokio::test]
    async fn learned_provenance_port_methods_round_trip_and_detect_bundles() {
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
        let port = RebornLocalSkillManagementPort::new(
            UserId::new("prov-owner").expect("valid user"),
            Arc::new(filesystem),
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/skills").expect("valid alias"),
                VirtualPath::new("/projects/skills").expect("valid path"),
                MountPermissions::read_write_list_delete(),
            )])
            .expect("valid mount view"),
        );
        let scope = port.owner_scope().expect("owner scope");

        let content = "---\nname: learned-one\ndescription: a learned skill\nactivation:\n  keywords: [deploy]\n---\nDo the deploy.\n";
        port.install_for_scope(scope.clone(), None, content)
            .await
            .expect("install");

        // No provenance until the sink writes one; a lone SKILL.md is not a bundle.
        assert!(
            port.read_provenance_for_scope(scope.clone(), "learned-one")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            !port
                .is_bundle_for_scope(scope.clone(), "learned-one")
                .await
                .unwrap()
        );

        // Write the baseline through the machine-only port, read it back.
        let provenance = LearnedSkillProvenance::for_machine_content(content).expect("baseline");
        port.write_provenance_for_scope(scope.clone(), "learned-one", &provenance)
            .await
            .unwrap();
        let read_back = port
            .read_provenance_for_scope(scope.clone(), "learned-one")
            .await
            .unwrap()
            .expect("provenance present after write");
        assert_eq!(read_back, provenance);
        assert!(read_back.matches_live_content(content));

        // A human drops a sibling file in → now a bundle.
        std::fs::write(
            storage_root.join("skills/learned-one/helper.py"),
            b"print('hi')",
        )
        .expect("write sibling file");
        assert!(
            port.is_bundle_for_scope(scope.clone(), "learned-one")
                .await
                .unwrap()
        );
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

    fn lifecycle_test_context() -> LifecycleProductContext {
        LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
            tenant_id: TenantId::new("lifecycle-tenant").expect("tenant"),
            user_id: UserId::new("lifecycle-owner").expect("user"),
            agent_id: None,
            project_id: None,
        })
    }

    fn skill_management_test_scope(tenant_id: &str, user_id: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant_id).expect("tenant"),
            user_id: UserId::new(user_id).expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }
}

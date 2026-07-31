use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    http::RuntimeHttpEgress,
    ids::{ExtensionId, InvocationId, UserId},
    product_surface::ProductSurfaceError,
    resource::ResourceScope,
    state::InstallationState,
};
use ironclaw_product::{
    LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction,
    LifecycleProductContext, LifecycleProductPayload, LifecycleProductResponse,
    LifecycleProductService, LifecycleReadinessBlocker, LifecycleSkillSource,
    LifecycleSkillSummary, ProductSurfaceFailure, lifecycle_product_surface_error,
};
#[cfg(test)]
use ironclaw_skills::build_scoped_skill_management_port;
use ironclaw_skills::{
    ScopedSkillManagementError, ScopedSkillManagementPort, SkillManagementError,
    SkillManagementErrorKind,
};

use crate::extension_activation_credentials::RuntimeExtensionActivationCredentialGate;
use crate::extension_lifecycle::RebornLocalExtensionManagementPort;
use ironclaw_auth::RuntimeCredentialAccountSelectionService;

const SKILL_SEARCH_RESULT_LIMIT: usize = 50;

#[derive(Clone)]
pub struct ExtensionHostLifecycleProductService {
    skill_management: Arc<ScopedSkillManagementPort>,
    extension_management: Option<Arc<RebornLocalExtensionManagementPort>>,
    channel_config: Option<Arc<ironclaw_extension_host::ChannelConfigService>>,
    runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    credential_accounts: Option<Arc<dyn RuntimeCredentialAccountSelectionService>>,
}

impl ExtensionHostLifecycleProductService {
    pub fn new(skill_management: Arc<ScopedSkillManagementPort>) -> Self {
        Self {
            skill_management,
            extension_management: None,
            channel_config: None,
            runtime_http_egress: None,
            credential_accounts: None,
        }
    }

    pub fn with_extension_management(
        mut self,
        extension_management: Arc<RebornLocalExtensionManagementPort>,
    ) -> Self {
        self.extension_management = Some(extension_management);
        self
    }

    pub fn with_channel_config(
        mut self,
        channel_config: Arc<ironclaw_extension_host::ChannelConfigService>,
    ) -> Self {
        self.channel_config = Some(channel_config);
        self
    }

    pub fn with_runtime_http_egress(
        mut self,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    ) -> Self {
        self.runtime_http_egress = Some(runtime_http_egress);
        self
    }

    pub fn with_runtime_credential_accounts(
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
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
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
                    InstallationState::Installed,
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
                    InstallationState::Installed,
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
                    InstallationState::Removed,
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
                let caller = lifecycle_caller(&context)?;
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
                let credential_gate = credential_gate.as_ref().map(|gate| {
                    gate as &dyn ironclaw_extension_host::ExtensionActivationCredentialGate
                });
                extension_management
                    .search(&query, credential_gate, &caller)
                    .await
            }
            LifecycleProductAction::ExtensionList => {
                let Some(extension_management) = &self.extension_management else {
                    return unsupported_projection(None);
                };
                let caller = lifecycle_caller(&context)?;
                extension_management.list_installed(&caller).await
            }
            LifecycleProductAction::ExtensionInstall { package_ref } => {
                let Some(extension_management) = &self.extension_management else {
                    return unsupported_projection(Some(package_ref));
                };
                let caller = lifecycle_caller(&context)?;
                self.execute_extension_install_with_activation(
                    context,
                    extension_management,
                    package_ref,
                    &caller,
                )
                .await
            }
            LifecycleProductAction::ExtensionActivate { package_ref } => {
                let Some(extension_management) = &self.extension_management else {
                    return unsupported_projection(Some(package_ref));
                };
                let caller = lifecycle_caller(&context)?;
                let credential_gate = self
                    .extension_activation_credential_gate(
                        &context,
                        extension_management,
                        &package_ref,
                        &caller,
                    )
                    .await?;
                if extension_management
                    .package_requires_hosted_mcp_discovery(&package_ref)
                    .await?
                {
                    let Some(runtime_http_egress) = self.runtime_http_egress.clone() else {
                        return Err(ProductSurfaceFailure::InvalidBindingRequest {
                            reason: format!(
                                "extension {} requires hosted MCP schema discovery and cannot be activated through the static lifecycle service",
                                package_ref.id
                            ),
                        });
                    };
                    let scope = lifecycle_resource_scope(&context)?;
                    let mode =
                        ironclaw_extension_host::ExtensionActivationMode::HostedMcpDiscovery {
                            scope,
                            runtime_http_egress,
                        };
                    return match credential_gate {
                        Some(credential_gate) => {
                            extension_management
                                .activate_with_credential_gate(
                                    package_ref,
                                    mode,
                                    credential_gate,
                                    &caller,
                                )
                                .await
                        }
                        None => {
                            extension_management
                                .activate(package_ref, mode, &caller)
                                .await
                        }
                    };
                }
                let mode = ironclaw_extension_host::ExtensionActivationMode::Static;
                match credential_gate {
                    Some(credential_gate) => {
                        extension_management
                            .activate_with_credential_gate(
                                package_ref,
                                mode,
                                credential_gate,
                                &caller,
                            )
                            .await
                    }
                    None => {
                        extension_management
                            .activate(package_ref, mode, &caller)
                            .await
                    }
                }
            }
            LifecycleProductAction::ExtensionRemove { package_ref } => {
                let Some(extension_management) = &self.extension_management else {
                    return unsupported_projection(Some(package_ref));
                };
                // Thread the caller scope so the port can revoke the removed
                // extension's exclusive credential (the convergence point shared
                // with the agent capability path).
                let scope = lifecycle_resource_scope(&context)?;
                extension_management
                    .remove(package_ref, &scope, Some(&scope.user_id))
                    .await
            }
            LifecycleProductAction::ExtensionAuth { package_ref } => {
                unsupported_extension_auth_configure_projection(Some(package_ref))
            }
            LifecycleProductAction::ExtensionConfigure {
                package_ref,
                payload,
            } => {
                // The configure half of the setup surface: validate + persist
                // manifest-declared channel-config values (extension-runtime
                // §6.4; a save against an active extension re-runs activation
                // per §6.5). Auth keeps the unsupported projection above.
                let (Some(extension_management), Some(channel_config)) =
                    (&self.extension_management, &self.channel_config)
                else {
                    return unsupported_extension_auth_configure_projection(Some(package_ref));
                };
                let extension_id = ExtensionId::new(package_ref.id.as_str()).map_err(|error| {
                    ProductSurfaceFailure::InvalidBindingRequest {
                        reason: format!("invalid extension id: {error}"),
                    }
                })?;
                let values = parse_channel_config_payload(payload.as_ref())?;
                channel_config
                    .save(&extension_id, values)
                    .await
                    .map_err(map_channel_config_error)?;
                let caller = lifecycle_caller(&context)?;
                let mut response = extension_management.project(package_ref, &caller).await?;
                response.message = Some("channel configuration saved".to_string());
                Ok(response)
            }
        }
    }

    async fn extension_activation_credential_gate(
        &self,
        context: &LifecycleProductContext,
        extension_management: &RebornLocalExtensionManagementPort,
        package_ref: &LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<Option<RuntimeExtensionActivationCredentialGate>, ProductSurfaceFailure> {
        // The requirements preflight checks ownership first, so a non-owner
        // exits here with the masked "is not installed" denial before any
        // credential or hosted-MCP probing can leak the install's existence.
        let requirements = extension_management
            .activation_credential_requirements(package_ref, caller)
            .await?;
        if requirements.is_empty() {
            return Ok(None);
        }
        // Credential readiness is evaluated exactly once inside activation,
        // where missing requirements become typed lifecycle blockers. When
        // product auth is not composed, the normal `activate` path uses the
        // unavailable gate and reports every declared requirement as missing.
        let Some(credential_accounts) = self.credential_accounts.as_ref() else {
            return Ok(None);
        };
        Ok(Some(RuntimeExtensionActivationCredentialGate::new(
            lifecycle_resource_scope(context)?,
            Arc::clone(credential_accounts),
        )))
    }

    async fn execute_extension_install_with_activation(
        &self,
        context: LifecycleProductContext,
        extension_management: &RebornLocalExtensionManagementPort,
        package_ref: LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        let install_response = extension_management
            .install(package_ref.clone(), caller)
            .await?;
        let activation_response = async {
            let credential_gate = self
                .extension_activation_credential_gate(
                    &context,
                    extension_management,
                    &package_ref,
                    caller,
                )
                .await?;
            if extension_management
                .package_requires_hosted_mcp_discovery(&package_ref)
                .await?
            {
                let Some(runtime_http_egress) = self.runtime_http_egress.clone() else {
                    return Err(ProductSurfaceFailure::InvalidBindingRequest {
                        reason: format!(
                            "extension {} requires hosted MCP schema discovery and cannot be activated through the static lifecycle service",
                            package_ref.id
                        ),
                    });
                };
                let scope = lifecycle_resource_scope(&context)?;
                let mode = ironclaw_extension_host::ExtensionActivationMode::HostedMcpDiscovery {
                    scope,
                    runtime_http_egress,
                };
                return match credential_gate {
                    Some(credential_gate) => {
                        extension_management
                            .activate_with_credential_gate(
                                package_ref,
                                mode,
                                credential_gate,
                                caller,
                            )
                            .await
                    }
                    None => extension_management.activate(package_ref, mode, caller).await,
                };
            }
            let mode = ironclaw_extension_host::ExtensionActivationMode::Static;
            match credential_gate {
                Some(credential_gate) => {
                    extension_management
                        .activate_with_credential_gate(package_ref, mode, credential_gate, caller)
                        .await
                }
                None => extension_management.activate(package_ref, mode, caller).await,
            }
        }
        .await;
        match activation_response {
            Ok(activation_response) if activation_response.phase == InstallationState::Active => {
                Ok(install_response_with_activation(
                    install_response,
                    activation_response,
                ))
            }
            Ok(activation_response)
                if activation_response_has_credential_blocker(&activation_response) =>
            {
                Ok(install_response_with_activation(
                    install_response,
                    activation_response,
                ))
            }
            Ok(_) => Ok(install_response),
            Err(error) => install_activation_error(error, install_response),
        }
    }
}

fn install_response_with_activation(
    mut install_response: LifecycleProductResponse,
    activation_response: LifecycleProductResponse,
) -> LifecycleProductResponse {
    install_response.phase = activation_response.phase;
    install_response.blockers = activation_response.blockers;
    install_response.message = activation_response.message;

    let activation_visible_capability_ids = match activation_response.payload {
        Some(LifecycleProductPayload::ExtensionActivate {
            visible_capability_ids,
            ..
        }) => Some(visible_capability_ids),
        _ => None,
    };
    if let Some(LifecycleProductPayload::ExtensionInstall {
        visible_capability_ids,
        next_step,
        ..
    }) = install_response.payload.as_mut()
    {
        if let Some(activation_visible_capability_ids) = activation_visible_capability_ids {
            *visible_capability_ids = activation_visible_capability_ids;
        }
        *next_step = if install_response.phase == InstallationState::Active {
            "Activation completed; model-visible extension tools are ready.".to_string()
        } else {
            "Activation did not complete; inspect the lifecycle phase and blockers.".to_string()
        };
    }
    install_response
}

fn activation_response_has_credential_blocker(response: &LifecycleProductResponse) -> bool {
    matches!(
        response.payload.as_ref(),
        Some(LifecycleProductPayload::ExtensionActivate {
            activated: false,
            ..
        })
    )
}

fn install_activation_error(
    error: ProductSurfaceFailure,
    install_response: LifecycleProductResponse,
) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
    match error {
        ProductSurfaceFailure::ProviderInstanceNotConfigured { .. } => Err(error),
        ProductSurfaceFailure::Transient { reason } => {
            tracing::debug!(
                target: "ironclaw::reborn::extension_lifecycle",
                %reason,
                "post-install activation reconciliation failed; returning installed lifecycle state"
            );
            Ok(install_response)
        }
        ProductSurfaceFailure::InvalidBindingRequest { reason }
            if reason.starts_with("hosted MCP discovery failed:")
                || reason
                    == "generic extension host rejected the activation: hosted MCP discovery published no callable tools" =>
        {
            tracing::debug!(
                target: "ironclaw::reborn::extension_lifecycle",
                %reason,
                "post-install hosted MCP discovery failed; returning installed lifecycle state"
            );
            Ok(install_response)
        }
        error => Err(error),
    }
}

#[async_trait]
impl LifecycleProductService for ExtensionHostLifecycleProductService {
    async fn execute(
        &self,
        context: LifecycleProductContext,
        action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
        self.execute_action(context, action)
            .await
            .map_err(lifecycle_product_surface_error)
    }

    async fn project_package(
        &self,
        context: LifecycleProductContext,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
        let result = async {
            if package_ref.kind == LifecyclePackageKind::Extension {
                let Some(extension_management) = &self.extension_management else {
                    return unsupported_projection(Some(package_ref));
                };
                let caller = lifecycle_caller(&context)?;
                return extension_management.project(package_ref, &caller).await;
            }
            unsupported_projection(Some(package_ref))
        }
        .await;
        result.map_err(lifecycle_product_surface_error)
    }

    async fn import_extension_bundle(
        &self,
        _context: LifecycleProductContext,
        bundle: Vec<u8>,
    ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
        let result = async {
            let Some(extension_management) = &self.extension_management else {
                return Err(ProductSurfaceFailure::InvalidBindingRequest {
                    reason: "extension management is not available in this runtime".to_string(),
                });
            };
            extension_management.import_bundle(bundle).await
        }
        .await;
        result.map_err(lifecycle_product_surface_error)
    }

    /// Project the durable installation records' redacted `last_error` so the
    /// extensions wire's `activation_error` reports *why* a `Failed` extension
    /// failed. Reads the generic host's working records through the extension
    /// management port (the same source the installation-state projection uses
    /// to surface `Failed`). Empty when extension management is not composed.
    async fn installed_activation_errors(
        &self,
        _context: LifecycleProductContext,
    ) -> Result<std::collections::HashMap<String, String>, ProductSurfaceError> {
        let result = match &self.extension_management {
            Some(extension_management) => {
                extension_management.installation_activation_errors().await
            }
            None => Ok(std::collections::HashMap::new()),
        };
        result.map_err(lifecycle_product_surface_error)
    }
}

fn skill_package_ref(name: &str) -> Result<LifecyclePackageRef, ProductSurfaceFailure> {
    Ok(LifecyclePackageRef::new(LifecyclePackageKind::Skill, name)?)
}

fn lifecycle_resource_scope(
    context: &LifecycleProductContext,
) -> Result<ResourceScope, ProductSurfaceFailure> {
    match context {
        LifecycleProductContext::Surface(context) => Ok(ResourceScope {
            tenant_id: context.tenant_id.clone(),
            user_id: context.user_id.clone(),
            agent_id: context.agent_id.clone(),
            project_id: context.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }),
        LifecycleProductContext::Command(command_context) => {
            // Commands have no surface context of their own. Their verified
            // auth claim is the authority-bearing source for the caller, and
            // a host-minted tenant claim is the corresponding tenant scope.
            // Claims without a tenant remain valid for standalone commands,
            // which use the local default scope just like the local command
            // service.
            let caller = lifecycle_caller(context)?;
            let mut scope =
                ResourceScope::local_default(caller, InvocationId::new()).map_err(|error| {
                    ProductSurfaceFailure::InvalidBindingRequest {
                        reason: format!("command lifecycle scope is invalid: {error}"),
                    }
                })?;
            if let Some(tenant_id) = command_context.auth_claim.tenant_id() {
                scope.tenant_id = tenant_id.clone();
            }
            Ok(scope)
        }
    }
}

/// Owner-attributing caller identity for extension lifecycle actions.
///
/// Surface callers carry a typed [`UserId`]; command callers derive it from
/// the verified auth claim minted by host authentication — commands must stay
/// owner-attributed, not fall back to an ownerless path.
fn lifecycle_caller(context: &LifecycleProductContext) -> Result<UserId, ProductSurfaceFailure> {
    match context {
        LifecycleProductContext::Surface(context) => Ok(context.user_id.clone()),
        LifecycleProductContext::Command(context) => UserId::new(context.auth_claim.subject())
            .map_err(|error| ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "command auth subject is not a valid lifecycle caller identity: {error}"
                ),
            }),
    }
}

pub fn response_with_payload(
    package_ref: Option<LifecyclePackageRef>,
    phase: InstallationState,
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
) -> Result<LifecycleSkillSummary, ProductSurfaceFailure> {
    Ok(LifecycleSkillSummary {
        name: LifecyclePackageId::new(skill.name)?,
        version: skill.version,
        description: skill.description,
        source: match skill.source {
            ironclaw_skills::ManagedSkillSource::System => LifecycleSkillSource::System,
            ironclaw_skills::ManagedSkillSource::User => LifecycleSkillSource::User,
            ironclaw_skills::ManagedSkillSource::Installed => LifecycleSkillSource::Installed,
        },
        keywords: skill.keywords,
        tags: skill.tags,
        requires_skills: skill.requires_skills,
    })
}

fn unsupported_projection(
    package_ref: Option<LifecyclePackageRef>,
) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
    Ok(LifecycleProductResponse::projection(
        package_ref,
        InstallationState::Unsupported,
        vec![LifecycleReadinessBlocker::runtime(Some(
            "extension_lifecycle_local_runtime_unwired".to_string(),
        ))?],
    ))
}

fn unsupported_extension_auth_configure_projection(
    package_ref: Option<LifecyclePackageRef>,
) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
    Ok(LifecycleProductResponse::projection(
        package_ref,
        InstallationState::Unsupported,
        vec![LifecycleReadinessBlocker::runtime(Some(
            "extension_auth_and_configure_not_yet_wired".to_string(),
        ))?],
    ))
}

/// Decode a configure payload: optional `fields` and `secrets` string maps,
/// unioned into `(handle, value)` pairs (the service classifies each handle
/// by its manifest descriptor, so which map a value rode in is advisory).
fn parse_channel_config_payload(
    payload: Option<&serde_json::Value>,
) -> Result<Vec<(String, String)>, ProductSurfaceFailure> {
    #[derive(Default, serde::Deserialize)]
    struct ConfigurePayload {
        #[serde(default)]
        fields: std::collections::BTreeMap<String, String>,
        #[serde(default)]
        secrets: std::collections::BTreeMap<String, String>,
    }
    let decoded = match payload {
        Some(payload) => {
            serde_json::from_value::<ConfigurePayload>(payload.clone()).map_err(|error| {
                ProductSurfaceFailure::InvalidBindingRequest {
                    reason: format!("invalid extension configure payload: {error}"),
                }
            })?
        }
        None => ConfigurePayload::default(),
    };
    Ok(decoded.fields.into_iter().chain(decoded.secrets).collect())
}

fn map_channel_config_error(
    error: ironclaw_extension_host::ChannelConfigError,
) -> ProductSurfaceFailure {
    use ironclaw_extension_host::ChannelConfigError;
    match error {
        ChannelConfigError::Storage { reason } => ProductSurfaceFailure::Transient { reason },
        ChannelConfigError::NotInstalled { .. }
        | ChannelConfigError::UnknownField { .. }
        | ChannelConfigError::Reactivation { .. } => ProductSurfaceFailure::InvalidBindingRequest {
            reason: error.to_string(),
        },
    }
}

fn map_skill_error(error: SkillManagementError) -> ProductSurfaceFailure {
    match error.kind() {
        SkillManagementErrorKind::InvalidInput
        | SkillManagementErrorKind::NotFound
        | SkillManagementErrorKind::Conflict
        | SkillManagementErrorKind::InvalidSkill => ProductSurfaceFailure::InvalidBindingRequest {
            reason: error
                .reason()
                .unwrap_or("skill management request rejected")
                .to_string(),
        },
        SkillManagementErrorKind::FilesystemDenied => ProductSurfaceFailure::BindingAccessDenied,
        SkillManagementErrorKind::Resource => ProductSurfaceFailure::Transient {
            reason: "skill management resource unavailable".to_string(),
        },
    }
}

fn map_local_skill_management_error(error: ScopedSkillManagementError) -> ProductSurfaceFailure {
    match error {
        ScopedSkillManagementError::InvalidContext { reason } => {
            ProductSurfaceFailure::InvalidBindingRequest { reason }
        }
        ScopedSkillManagementError::Skill(error) => map_skill_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::DiskFilesystem;
    use ironclaw_host_api::{
        ids::{AgentId, ProjectId, TenantId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{HostPath, MountAlias, VirtualPath},
    };
    use ironclaw_product::LifecycleProductSurfaceContext;

    #[tokio::test]
    async fn skill_lifecycle_service_installs_lists_and_removes_via_skill_management() {
        let (_dir, storage_root, service) = lifecycle_fixture();

        let install = service
            .execute_action(lifecycle_test_context(), LifecycleProductAction::SkillInstall {
                name: None,
                content:
                    "---\nname: lifecycle-skill\ndescription: lifecycle test\n---\nUse lifecycle.\n"
                        .to_string(),
            })
            .await
            .expect("install skill");
        assert_eq!(install.phase, InstallationState::Installed);
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

        let list = service
            .execute_action(
                lifecycle_test_context(),
                LifecycleProductAction::SkillSearch {
                    query: "lifecycle".to_string(),
                },
            )
            .await
            .expect("list skills");
        assert_eq!(list.phase, InstallationState::Installed);
        let Some(LifecycleProductPayload::SkillSearch { count, .. }) = list.payload.as_ref() else {
            panic!("expected skill search payload");
        };
        assert_eq!(*count, 1);

        for index in 0..55 {
            service
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

        let all_skills = service
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

        let wrong_kind = service
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
            ProductSurfaceFailure::InvalidBindingRequest { .. }
        ));
        assert!(
            storage_root
                .join("skills/lifecycle-skill/SKILL.md")
                .exists()
        );

        let remove = service
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
        assert_eq!(remove.phase, InstallationState::Removed);
        assert!(
            !storage_root
                .join("skills/lifecycle-skill/SKILL.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn default_skill_management_port_isolates_user_skill_roots_by_scope() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("standalone");
        std::fs::create_dir_all(storage_root.join("system/skills/system-helper"))
            .expect("system skill dir");
        std::fs::write(
            storage_root.join("system/skills/system-helper/SKILL.md"),
            skill_content("system-helper"),
        )
        .expect("system skill");

        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        let skill_management = build_scoped_skill_management_port(
            UserId::new("runtime-owner").expect("valid user"),
            Arc::new(filesystem),
        );
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
    async fn skill_lifecycle_service_serializes_concurrent_install_and_remove() {
        let (_dir, storage_root, service) = lifecycle_fixture();

        let service_a = service.clone();
        let service_b = service.clone();
        let install_a = service_a.execute_action(
            lifecycle_test_context(),
            LifecycleProductAction::SkillInstall {
                name: Some(LifecyclePackageId::new("concurrent-a").expect("valid skill id")),
                content: skill_content("concurrent-a"),
            },
        );
        let install_b = service_b.execute_action(
            lifecycle_test_context(),
            LifecycleProductAction::SkillInstall {
                name: Some(LifecyclePackageId::new("concurrent-b").expect("valid skill id")),
                content: skill_content("concurrent-b"),
            },
        );
        let (installed_a, installed_b) = tokio::join!(install_a, install_b);
        installed_a.expect("install concurrent-a");
        installed_b.expect("install concurrent-b");

        let service_a = service.clone();
        let remove_a = service_a.execute_action(
            lifecycle_test_context(),
            LifecycleProductAction::SkillRemove {
                package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Skill, "concurrent-a")
                    .expect("valid skill ref"),
            },
        );
        let remove_b = service.execute_action(
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
    async fn skill_lifecycle_service_maps_skill_management_errors() {
        let (_dir, _storage_root, service) = lifecycle_fixture();

        let invalid_install = service
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
            ProductSurfaceFailure::InvalidBindingRequest { .. }
        ));

        let missing_remove = service
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
            ProductSurfaceFailure::InvalidBindingRequest { .. }
        ));
    }

    fn lifecycle_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        ExtensionHostLifecycleProductService,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("standalone");
        std::fs::create_dir_all(&storage_root).expect("storage root");

        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        let skill_management = Arc::new(ScopedSkillManagementPort::new(
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
        let service = ExtensionHostLifecycleProductService::new(skill_management);
        (dir, storage_root, service)
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

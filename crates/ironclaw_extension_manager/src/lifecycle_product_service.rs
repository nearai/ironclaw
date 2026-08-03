use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::lifecycle_id::LifecyclePackageId;
use ironclaw_extension_contracts::state::InstallationState;
use ironclaw_host_api::{
    ids::{ExtensionId, InvocationId, UserId},
    resource::ResourceScope,
};
use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_product_contracts::lifecycle_service::{
    LifecycleProductContext, LifecycleProductService,
};
use ironclaw_product_contracts::package_lifecycle::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction, LifecycleProductPayload,
    LifecycleProductResponse, LifecycleReadinessBlocker, LifecycleSkillSource,
    LifecycleSkillSummary,
};
use ironclaw_product_contracts::surface::ProductSurfaceError;
#[cfg(test)]
use ironclaw_skills::build_scoped_skill_management_port;
use ironclaw_skills::{
    ScopedSkillManagementError, ScopedSkillManagementPort, SkillManagementError,
    SkillManagementErrorKind,
};

use ironclaw_auth::RuntimeCredentialAccountSelectionService;
use ironclaw_extension_host::extension_activation_credentials::RuntimeExtensionActivationCredentialGate;
use ironclaw_extension_host::extension_lifecycle::RebornLocalExtensionManagementPort;

const SKILL_SEARCH_RESULT_LIMIT: usize = 50;

#[derive(Clone)]
pub struct ExtensionHostLifecycleProductService {
    skill_management: Arc<ScopedSkillManagementPort>,
    extension_management: Option<Arc<RebornLocalExtensionManagementPort>>,
    channel_config: Option<Arc<ironclaw_extension_host::ChannelConfigService>>,
    credential_accounts: Option<Arc<dyn RuntimeCredentialAccountSelectionService>>,
}

impl ExtensionHostLifecycleProductService {
    pub fn new(skill_management: Arc<ScopedSkillManagementPort>) -> Self {
        Self {
            skill_management,
            extension_management: None,
            channel_config: None,
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
    ) -> Result<LifecycleProductResponse, ProductOperationFailure> {
        match action {
            LifecycleProductAction::ExtensionRegisterHostedMcp { request } => {
                let Some(extension_management) = &self.extension_management else {
                    return unsupported_projection(None);
                };
                extension_management.register_hosted_mcp(request).await
            }
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
                self.execute_extension_activation(
                    &context,
                    extension_management,
                    package_ref,
                    &caller,
                )
                .await
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
                    ProductOperationFailure::InvalidBindingRequest {
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
    ) -> Result<Option<RuntimeExtensionActivationCredentialGate>, ProductOperationFailure> {
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
    ) -> Result<LifecycleProductResponse, ProductOperationFailure> {
        let install_response = extension_management
            .install(package_ref.clone(), caller)
            .await?;
        let activation_response = self
            .execute_extension_activation(&context, extension_management, package_ref, caller)
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

    /// The one post-install and explicit-activation path. Every package uses
    /// the same credential preflight, optional preparation boundary, and
    /// ordinary static activation sequence.
    async fn execute_extension_activation(
        &self,
        context: &LifecycleProductContext,
        extension_management: &RebornLocalExtensionManagementPort,
        package_ref: LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductOperationFailure> {
        let credential_gate = self
            .extension_activation_credential_gate(
                context,
                extension_management,
                &package_ref,
                caller,
            )
            .await?;
        let unavailable_gate =
            ironclaw_extension_host::UnavailableExtensionActivationCredentialGate;
        let credential_gate: &dyn ironclaw_extension_host::ExtensionActivationCredentialGate =
            credential_gate
                .as_ref()
                .map(|gate| gate as &dyn ironclaw_extension_host::ExtensionActivationCredentialGate)
                .unwrap_or(&unavailable_gate);
        extension_management
            .activate_with_credential_gate(
                package_ref,
                lifecycle_resource_scope(context)?,
                credential_gate,
                caller,
            )
            .await
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

/// Project this crate's own lifecycle failure onto the sanitized surface
/// error, logging the transient cause first.
///
/// The status table itself lives with the contract
/// (`ProductOperationFailure` -> `ProductSurfaceError`), so this path and
/// `ironclaw_product`'s `lifecycle_product_surface_error` cannot answer
/// differently for the same failure. Only the logging is local: contracts may
/// not log, and the 503 body is sanitized, so without this line the cause is
/// dropped entirely and the failure is undiagnosable.
fn lifecycle_surface_error(error: ProductOperationFailure) -> ProductSurfaceError {
    if let ProductOperationFailure::Transient { reason } = &error {
        tracing::warn!(reason = %reason, "lifecycle action failed with a transient error");
    }
    error.into()
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
    error: ProductOperationFailure,
    install_response: LifecycleProductResponse,
) -> Result<LifecycleProductResponse, ProductOperationFailure> {
    match error {
        ProductOperationFailure::ProviderInstanceNotConfigured { .. } => Err(error),
        ProductOperationFailure::Transient { reason } => {
            tracing::debug!(
                target: "ironclaw::reborn::extension_lifecycle",
                %reason,
                "post-install activation reconciliation failed; returning installed lifecycle state"
            );
            Ok(install_response)
        }
        ProductOperationFailure::InvalidBindingRequest { reason }
            if ironclaw_extension_host::hosted_mcp_discovery_left_the_install_usable(&reason) =>
        {
            tracing::debug!(
                target: "ironclaw::reborn::extension_lifecycle",
                %reason,
                "post-install hosted MCP discovery failed; returning installed lifecycle state"
            );
            Ok(install_response)
        }
        error => {
            tracing::debug!(
                target: "ironclaw::reborn::extension_lifecycle",
                reason = %error,
                "post-install activation failed"
            );
            Err(error)
        }
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
            .map_err(lifecycle_surface_error)
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
        result.map_err(lifecycle_surface_error)
    }

    async fn import_extension_bundle(
        &self,
        _context: LifecycleProductContext,
        bundle: Vec<u8>,
    ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
        let result = async {
            let Some(extension_management) = &self.extension_management else {
                return Err(ProductOperationFailure::InvalidBindingRequest {
                    reason: "extension management is not available in this runtime".to_string(),
                });
            };
            extension_management.import_bundle(bundle).await
        }
        .await;
        result.map_err(lifecycle_surface_error)
    }

    /// Project the durable installation records' redacted `last_error` so the
    /// extensions wire's `activation_error` reports *why* a `Failed` extension
    /// failed. Reads the generic host's working records through the extension
    /// management port (the same source the installation-state projection uses
    /// to surface `Failed`). Empty when extension management is not composed.
    async fn installed_activation_errors(
        &self,
        _context: LifecycleProductContext,
    ) -> Result<std::collections::HashMap<ExtensionId, String>, ProductSurfaceError> {
        let result = match &self.extension_management {
            Some(extension_management) => {
                extension_management.installation_activation_errors().await
            }
            None => Ok(std::collections::HashMap::new()),
        };
        result.map_err(lifecycle_surface_error)
    }
}

fn skill_package_ref(name: &str) -> Result<LifecyclePackageRef, ProductOperationFailure> {
    Ok(LifecyclePackageRef::new(LifecyclePackageKind::Skill, name)?)
}

fn lifecycle_resource_scope(
    context: &LifecycleProductContext,
) -> Result<ResourceScope, ProductOperationFailure> {
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
                    ProductOperationFailure::InvalidBindingRequest {
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
fn lifecycle_caller(context: &LifecycleProductContext) -> Result<UserId, ProductOperationFailure> {
    match context {
        LifecycleProductContext::Surface(context) => Ok(context.user_id.clone()),
        LifecycleProductContext::Command(context) => UserId::new(context.auth_claim.subject())
            .map_err(|error| ProductOperationFailure::InvalidBindingRequest {
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
) -> Result<LifecycleSkillSummary, ProductOperationFailure> {
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
) -> Result<LifecycleProductResponse, ProductOperationFailure> {
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
) -> Result<LifecycleProductResponse, ProductOperationFailure> {
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
) -> Result<Vec<(String, String)>, ProductOperationFailure> {
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
                ProductOperationFailure::InvalidBindingRequest {
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
) -> ProductOperationFailure {
    use ironclaw_extension_host::ChannelConfigError;
    match error {
        ChannelConfigError::Storage { reason } => ProductOperationFailure::Transient { reason },
        ChannelConfigError::NotInstalled { .. }
        | ChannelConfigError::UnknownField { .. }
        | ChannelConfigError::Reactivation { .. } => {
            ProductOperationFailure::InvalidBindingRequest {
                reason: error.to_string(),
            }
        }
    }
}

fn map_skill_error(error: SkillManagementError) -> ProductOperationFailure {
    match error.kind() {
        SkillManagementErrorKind::InvalidInput
        | SkillManagementErrorKind::NotFound
        | SkillManagementErrorKind::Conflict
        | SkillManagementErrorKind::InvalidSkill => {
            ProductOperationFailure::InvalidBindingRequest {
                reason: error
                    .reason()
                    .unwrap_or("skill management request rejected")
                    .to_string(),
            }
        }
        SkillManagementErrorKind::FilesystemDenied => ProductOperationFailure::BindingAccessDenied,
        SkillManagementErrorKind::Resource => ProductOperationFailure::Transient {
            reason: "skill management resource unavailable".to_string(),
        },
    }
}

fn map_local_skill_management_error(error: ScopedSkillManagementError) -> ProductOperationFailure {
    match error {
        ScopedSkillManagementError::InvalidContext { reason } => {
            ProductOperationFailure::InvalidBindingRequest { reason }
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
    use ironclaw_product_contracts::lifecycle_service::LifecycleProductSurfaceContext;

    /// This crate's projection must answer exactly what the contract's own
    /// projection answers — the status table lives there and this wrapper adds
    /// only the transient log. Drift here means the WebUI reports one status
    /// through product's lifecycle service and a different one through the
    /// extension host's, for the identical failure.
    ///
    /// The `Transient` case also drives the logging branch, which is the only
    /// line in this function that is not a delegation.
    #[test]
    fn lifecycle_surface_error_matches_the_contract_projection_for_every_variant() {
        for failure in [
            ProductOperationFailure::BindingResolutionFailed {
                reason: "no tenant".into(),
            },
            ProductOperationFailure::BindingAccessDenied,
            ProductOperationFailure::InvalidBindingRequest {
                reason: "bad ref".into(),
            },
            ProductOperationFailure::ProviderInstanceNotConfigured {
                reason: "ironclaw config set google.client_id <id>".into(),
            },
            ProductOperationFailure::UnsupportedActionKind {
                kind: "teleport".into(),
            },
            ProductOperationFailure::Transient {
                reason: "db timeout".into(),
            },
        ] {
            let projected = lifecycle_surface_error(failure.clone());
            let expected: ProductSurfaceError = failure.clone().into();
            assert_eq!(projected, expected, "projection drifted for {failure:?}");
        }
    }

    fn installed_response() -> LifecycleProductResponse {
        LifecycleProductResponse::projection(
            Some(
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, "gmail")
                    .expect("package ref"),
            ),
            InstallationState::Installed,
            Vec::new(),
        )
    }

    /// Install runs activation as a follow-on step, so this classifier decides
    /// which activation failures are *swallowed* — reported to the caller as a
    /// successful install — and which abort the whole operation. Swallowing the
    /// wrong one hides a real failure behind a green install, so every arm is
    /// pinned, and the discriminating input is the failure itself: the same
    /// `installed_response()` goes in every time and only the error varies.
    #[test]
    fn post_install_activation_failures_are_swallowed_only_when_the_install_still_stands() {
        // Provider misconfiguration must reach the caller: the extension is
        // installed but unusable, and only an operator can fix it.
        assert!(
            install_activation_error(
                ProductOperationFailure::ProviderInstanceNotConfigured {
                    reason: "ironclaw config set google.client_id <id>".to_string(),
                },
                installed_response(),
            )
            .is_err(),
            "an unconfigured provider must not be hidden behind a successful install"
        );

        // A transient reconciliation blip leaves the install itself intact.
        assert_eq!(
            install_activation_error(
                ProductOperationFailure::Transient {
                    reason: "db timeout".to_string(),
                },
                installed_response(),
            )
            .expect("a transient activation blip keeps the install"),
            installed_response(),
            "the caller must still see the state the install actually reached"
        );

        // Hosted-MCP discovery is best-effort at install time — but only for
        // the two exact reasons this guard names.
        assert_eq!(
            install_activation_error(
                ProductOperationFailure::InvalidBindingRequest {
                    reason: "hosted MCP catalog preparation failed: upstream 500".to_string(),
                },
                installed_response(),
            )
            .expect("hosted MCP preparation is best-effort at install time"),
            installed_response(),
            "a hosted-MCP preparation failure still leaves a usable install"
        );
        assert!(
            install_activation_error(
                ProductOperationFailure::InvalidBindingRequest {
                    reason: "some other rejection".to_string(),
                },
                installed_response(),
            )
            .is_err(),
            "the guard is reason-specific: any other rejection must still surface"
        );
    }

    /// Both unsupported projections must answer `Unsupported` with a runtime
    /// blocker rather than an error — the WebUI renders the blocker, and an
    /// error would render as a failed request instead.
    #[test]
    fn unwired_lifecycle_surfaces_project_unsupported_with_a_runtime_blocker() {
        for response in [
            unsupported_projection(None).expect("projection is infallible for a well-formed ref"),
            unsupported_extension_auth_configure_projection(None)
                .expect("projection is infallible for a well-formed ref"),
        ] {
            assert_eq!(response.phase, InstallationState::Unsupported);
            assert!(
                matches!(
                    response.blockers.as_slice(),
                    [LifecycleReadinessBlocker::Runtime { ref_id: Some(_) }]
                ),
                "an unwired surface must name why it is unavailable: {:?}",
                response.blockers
            );
        }
    }

    /// The configure payload unions `fields` and `secrets`; a payload that is
    /// not that shape is the caller's mistake, not an outage.
    #[test]
    fn a_configure_payload_decodes_both_maps_and_rejects_anything_else() {
        assert_eq!(
            parse_channel_config_payload(None).expect("an absent payload is empty, not invalid"),
            Vec::<(String, String)>::new(),
        );
        assert_eq!(
            parse_channel_config_payload(Some(&serde_json::json!({
                "fields": {"channel": "#general"},
                "secrets": {"token": "xoxb-1"},
            })))
            .expect("a well-formed payload decodes"),
            vec![
                ("channel".to_string(), "#general".to_string()),
                ("token".to_string(), "xoxb-1".to_string()),
            ],
            "both maps must reach the service, not just `fields`"
        );

        let failure = parse_channel_config_payload(Some(&serde_json::json!({"fields": 7})))
            .expect_err("`fields` must be a string map");
        assert!(
            matches!(
                failure,
                ProductOperationFailure::InvalidBindingRequest { .. }
            ),
            "a malformed payload is a rejected request, not an outage: {failure:?}"
        );
    }

    /// Storage trouble is retryable; every other configure-surface failure is
    /// the caller's to fix. Collapsing the two would either make the WebUI
    /// retry a permanent rejection or give up on a blip.
    #[test]
    fn configure_surface_failures_split_storage_from_caller_error() {
        assert_eq!(
            map_channel_config_error(ironclaw_extension_host::ChannelConfigError::Storage {
                reason: "backend unreachable".to_string(),
            }),
            ProductOperationFailure::Transient {
                reason: "backend unreachable".to_string(),
            },
            "storage trouble is retryable and keeps its reason"
        );
        for caller_error in [
            ironclaw_extension_host::ChannelConfigError::NotInstalled {
                extension_id: "slack".to_string(),
            },
            ironclaw_extension_host::ChannelConfigError::UnknownField {
                handle: "nope".to_string(),
            },
            ironclaw_extension_host::ChannelConfigError::Reactivation {
                reason: "restart refused".to_string(),
            },
        ] {
            assert_eq!(
                map_channel_config_error(caller_error.clone()),
                ProductOperationFailure::InvalidBindingRequest {
                    reason: caller_error.to_string(),
                },
                "{caller_error:?} is the caller's to fix, not ours to retry"
            );
        }
    }

    /// `FilesystemDenied` is an authorization outcome and must project as
    /// `BindingAccessDenied` — projecting it as a generic invalid request would
    /// tell the caller to retry a denial, and projecting it as `Transient`
    /// would hide the denial entirely.
    #[test]
    fn skill_management_failures_keep_denial_separate_from_outage_and_bad_input() {
        assert_eq!(
            map_skill_error(SkillManagementError::new(
                SkillManagementErrorKind::FilesystemDenied
            )),
            ProductOperationFailure::BindingAccessDenied,
            "a filesystem denial is an authorization outcome"
        );
        assert_eq!(
            map_skill_error(SkillManagementError::new(
                SkillManagementErrorKind::Resource
            )),
            ProductOperationFailure::Transient {
                reason: "skill management resource unavailable".to_string(),
            },
            "a resource shortage is retryable"
        );
        assert_eq!(
            map_skill_error(SkillManagementError::with_reason(
                SkillManagementErrorKind::NotFound,
                "no such skill",
            )),
            ProductOperationFailure::InvalidBindingRequest {
                reason: "no such skill".to_string(),
            },
            "a caller-fixable failure forwards its reason"
        );
        assert_eq!(
            map_skill_error(SkillManagementError::new(
                SkillManagementErrorKind::NotFound
            )),
            ProductOperationFailure::InvalidBindingRequest {
                reason: "skill management request rejected".to_string(),
            },
            "a reasonless rejection still gets usable text"
        );
    }

    /// The scoped wrapper must not flatten its two cases: a bad scope is the
    /// caller's, and an inner skill failure keeps the inner classification
    /// (including the `BindingAccessDenied` denial above).
    #[test]
    fn scoped_skill_failures_preserve_the_inner_classification() {
        assert_eq!(
            map_local_skill_management_error(ScopedSkillManagementError::InvalidContext {
                reason: "no skills mount".to_string(),
            }),
            ProductOperationFailure::InvalidBindingRequest {
                reason: "no skills mount".to_string(),
            },
        );
        assert_eq!(
            map_local_skill_management_error(ScopedSkillManagementError::Skill(
                SkillManagementError::new(SkillManagementErrorKind::FilesystemDenied)
            )),
            ProductOperationFailure::BindingAccessDenied,
            "wrapping a denial must not downgrade it to a generic rejection"
        );
    }

    #[derive(Clone, Default)]
    struct SharedLogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    struct SharedLogWriterGuard(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedLogWriterGuard {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLogWriter {
        type Writer = SharedLogWriterGuard;

        fn make_writer(&'a self) -> Self::Writer {
            SharedLogWriterGuard(std::sync::Arc::clone(&self.0))
        }
    }

    impl SharedLogWriter {
        fn contents(&self) -> String {
            String::from_utf8(
                self.0
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone(),
            )
            .expect("tracing output is UTF-8")
        }
    }

    /// The transient warning is the *whole* reason this crate kept a local
    /// projection wrapper instead of calling the contract's `From` directly,
    /// so both halves of its contract are asserted together:
    ///
    /// - the caller's 503 is **sanitized** — the store's cause never crosses
    ///   the membrane, because the wire contract has no free-text field; and
    /// - the cause is **not discarded** — it reaches the warning where an
    ///   operator can diagnose it.
    ///
    /// A subscriber has to be installed for this to mean anything: `tracing`
    /// short-circuits on the null dispatcher, so without one the macro body
    /// never runs and the test could not tell "logged it" from "dropped it" —
    /// which is exactly the assertion that matters here. Scoped with
    /// `with_default` rather than set globally, so parallel tests are
    /// unaffected.
    #[test]
    fn a_transient_failure_is_sanitized_for_the_caller_and_still_diagnosable_in_the_log() {
        let cause = "channel config backend refused the read";
        let logs = SharedLogWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_target(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(logs.clone())
            .finish();

        let projected = tracing::subscriber::with_default(subscriber, || {
            lifecycle_surface_error(ProductOperationFailure::Transient {
                reason: cause.to_string(),
            })
        });

        // Half one: nothing about the cause reaches the caller.
        assert_eq!(projected.status_code, 503);
        assert!(projected.retryable);
        assert_eq!(projected.field, None);
        assert_eq!(projected.validation_code, None);
        let on_the_wire = serde_json::to_string(&projected).expect("surface error serializes");
        assert!(
            !on_the_wire.contains(cause),
            "the transient cause must not cross the membrane: {on_the_wire}"
        );

        // Half two: the cause is not merely dropped — it is diagnosable.
        let logged = logs.contents();
        assert!(
            logged.contains(cause),
            "the transient cause must survive in the log, got {logged:?}"
        );
        assert!(
            logged.contains("lifecycle action failed with a transient error"),
            "the warning keeps its stable message, got {logged:?}"
        );
    }

    /// The counterpart to the test above: a *rejection* carries no operational
    /// cause, so it must not spend a warning. Without this, "log everything"
    /// would pass the test above while turning every invalid request into
    /// operator noise.
    #[test]
    fn a_rejected_request_does_not_emit_the_transient_warning() {
        let logs = SharedLogWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_target(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(logs.clone())
            .finish();

        let projected = tracing::subscriber::with_default(subscriber, || {
            lifecycle_surface_error(ProductOperationFailure::InvalidBindingRequest {
                reason: "bad package ref".to_string(),
            })
        });

        assert_eq!(projected.status_code, 400);
        assert!(!projected.retryable);
        assert!(
            logs.contents().is_empty(),
            "a rejection must not emit the transient warning, got {:?}",
            logs.contents()
        );
    }

    /// The two statuses a caller acts on differently: a transient failure is
    /// retryable and a rejected request is not. Pinned separately from the
    /// table above so a change that made everything retryable would fail here
    /// with an obvious message rather than as an equality mismatch.
    #[test]
    fn transient_lifecycle_failures_are_retryable_and_invalid_requests_are_not() {
        let transient = lifecycle_surface_error(ProductOperationFailure::Transient {
            reason: "db timeout".into(),
        });
        assert_eq!(transient.status_code, 503);
        assert!(transient.retryable);

        let invalid = lifecycle_surface_error(ProductOperationFailure::InvalidBindingRequest {
            reason: "bad ref".into(),
        });
        assert_eq!(invalid.status_code, 400);
        assert!(!invalid.retryable);
    }

    #[test]
    fn installed_skill_source_remains_installed_in_lifecycle_projection() {
        let projected = skill_summary(ironclaw_skills::SkillSummary {
            name: "registry-skill".to_string(),
            version: "1.0.0".to_string(),
            description: "installed from a verified catalog".to_string(),
            source: ironclaw_skills::ManagedSkillSource::Installed,
            keywords: Vec::new(),
            tags: Vec::new(),
            requires_skills: Vec::new(),
            auto_activate: false,
        })
        .expect("valid skill summary");

        assert_eq!(projected.source, LifecycleSkillSource::Installed);
    }

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
                .execute_action(
                    lifecycle_test_context(),
                    LifecycleProductAction::SkillInstall {
                        name: Some(
                            LifecyclePackageId::new(format!("bulk-skill-{index:02}"))
                                .expect("valid skill id"),
                        ),
                        content: format!(
                        "---\nname: bulk-skill-{index:02}\ndescription: bulk test\n---\nUse bulk.\n"
                    ),
                    },
                )
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
            ProductOperationFailure::InvalidBindingRequest { .. }
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

    /// A `LifecycleProductContext::Command` whose verified auth subject is
    /// `subject`.
    ///
    /// ✎ An earlier revision of this branch waived `lifecycle_caller`'s Command
    /// arm as untestable, on the reasoning that `VerifiedAuthClaim`'s
    /// constructors are `pub(crate)` to `ironclaw_host_api`. That is true of
    /// the constructors and **false of the barrier**: `ProtocolAuthEvidence`
    /// exposes `test_verified` under `#[cfg(any(test, feature =
    /// "test-support"))]`, and this crate's `[dev-dependencies]` already enable
    /// `ironclaw_host_api/test-support`. `channel_command_roles.rs` in this
    /// same crate has been building command contexts through that seam all
    /// along. Raised by @serrrfirat on #7000; the waiver is deleted and the
    /// path is tested instead.
    fn command_context(subject: &str) -> LifecycleProductContext {
        use ironclaw_extension_contracts::channel_adapter::ProductTriggerReason;
        use ironclaw_extension_contracts::external::{
            ExternalActorRef, ExternalConversationRef, ExternalEventId,
        };
        use ironclaw_host_api::product_adapter::{
            AdapterInstallationId, AuthRequirement, ProductAdapterId, ProtocolAuthEvidence,
        };
        use ironclaw_product_contracts::action::{
            ActionFingerprintKey, ProductActionId, SourceBindingKey,
        };
        use ironclaw_product_contracts::command::ProductCommandContext;
        use ironclaw_product_contracts::inbound::ProductInboundPayload;
        use ironclaw_product_contracts::inbound::{
            InboundCommandPayload, ParsedProductInbound, ProductInboundEnvelope,
            TrustedInboundContext,
        };

        let adapter_id = ProductAdapterId::new("test_adapter").expect("valid adapter");
        let installation_id =
            AdapterInstallationId::new("install_alpha").expect("valid installation");
        let evidence = ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Secret".into(),
            },
            subject,
        );
        let trusted = TrustedInboundContext::from_verified_evidence(
            adapter_id,
            installation_id,
            chrono::Utc::now(),
            &evidence,
        )
        .expect("verified evidence yields a trusted context");
        let parsed = ParsedProductInbound::new(
            ExternalEventId::new("evt:lifecycle-caller").expect("valid event"),
            ExternalActorRef::new("test", "actor-1", Option::<String>::None).expect("valid actor"),
            ExternalConversationRef::new(None, "conv1", None, None).expect("valid conversation"),
            ProductInboundPayload::Command(
                InboundCommandPayload::new("extensions", "", ProductTriggerReason::DirectChat)
                    .expect("valid command"),
            ),
        )
        .expect("parsed inbound");
        let envelope = ProductInboundEnvelope::from_trusted_parse(trusted, parsed)
            .expect("trusted parse yields an envelope");
        let source_binding_key =
            SourceBindingKey::new(envelope.source_binding_key()).expect("valid binding key");
        let fingerprint = ActionFingerprintKey::new(
            envelope.adapter_id().clone(),
            envelope.installation_id().clone(),
            envelope.external_actor_ref().clone(),
            source_binding_key,
            envelope.external_event_id().clone(),
        );
        let context =
            ProductCommandContext::from_envelope(&envelope, ProductActionId::new(), fingerprint)
                .expect("command context");
        LifecycleProductContext::Command(Box::new(context))
    }

    /// Commands must stay **owner-attributed**: the caller identity comes from
    /// the verified auth claim minted by host authentication, and a subject that
    /// is not a valid `UserId` is refused rather than silently falling back to
    /// an ownerless scope.
    ///
    /// The subject is the discriminating input — the same context shape goes in
    /// both times and only the claim's subject varies — so a `lifecycle_caller`
    /// that ignored the claim, or that swallowed the validation error, fails
    /// here rather than passing against a constant.
    #[test]
    fn a_command_caller_comes_from_the_auth_claim_and_an_invalid_subject_is_refused() {
        let caller = lifecycle_caller(&command_context("user-alpha"))
            .expect("a valid auth subject is a valid lifecycle caller");
        assert_eq!(
            caller.as_str(),
            "user-alpha",
            "the command caller must be the verified auth subject, not a default"
        );

        let failure = lifecycle_caller(&command_context("user/alpha"))
            .expect_err("a subject with a path separator is not a valid UserId");
        let ProductOperationFailure::InvalidBindingRequest { reason } = failure else {
            panic!("an unusable auth subject is the caller's to fix, got {failure:?}");
        };
        assert!(
            reason.contains("command auth subject is not a valid lifecycle caller identity"),
            "the rejection must say which identity failed, got {reason:?}"
        );
    }

    /// The Command arm's scope derives from that same claim: the caller becomes
    /// the scope's user, and a claim carrying **no tenant** keeps the local
    /// default scope rather than inventing a tenant — the behavior the function's
    /// own comment promises ("just like the local command service"). Pinned
    /// beside the Surface arm above so the two context sources cannot drift into
    /// different scoping rules.
    #[test]
    fn lifecycle_resource_scope_uses_the_command_auth_claim_identity() {
        use ironclaw_host_api::resource::{
            LOCAL_DEFAULT_AGENT_ID, LOCAL_DEFAULT_PROJECT_ID, LOCAL_DEFAULT_TENANT_ID,
        };

        let scope = lifecycle_resource_scope(&command_context("user-beta")).expect("command scope");

        assert_eq!(
            scope.user_id.as_str(),
            "user-beta",
            "the scope's user must be the verified auth subject"
        );
        assert_eq!(
            scope.tenant_id.as_str(),
            LOCAL_DEFAULT_TENANT_ID,
            "a claim with no tenant must keep the local default, not invent one"
        );
        assert_eq!(
            scope.agent_id.as_ref().map(|id| id.as_str()),
            Some(LOCAL_DEFAULT_AGENT_ID)
        );
        assert_eq!(
            scope.project_id.as_ref().map(|id| id.as_str()),
            Some(LOCAL_DEFAULT_PROJECT_ID)
        );
        assert!(scope.thread_id.is_none());

        let refused = lifecycle_resource_scope(&command_context("user/beta"))
            .expect_err("an unusable auth subject cannot produce a scope");
        assert!(
            matches!(
                refused,
                ProductOperationFailure::InvalidBindingRequest { .. }
            ),
            "an unusable auth subject is a rejection, not a retryable failure: {refused:?}"
        );
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
            ProductOperationFailure::InvalidBindingRequest { .. }
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
            ProductOperationFailure::InvalidBindingRequest { .. }
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

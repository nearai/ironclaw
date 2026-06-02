use std::collections::HashSet;

use crate::{
    LifecycleExtensionSummary, LifecycleInstalledExtensionSummary, LifecyclePackageRef,
    LifecyclePhase, LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleProductSurfaceContext,
    RebornExtensionActionResponse, RebornExtensionInfo, RebornExtensionListResponse,
    RebornExtensionRegistryEntry, RebornExtensionRegistryResponse, RebornServicesError,
    WebUiAuthenticatedCaller,
};

use super::extension_onboarding;
use super::lifecycle_setup::map_lifecycle_error;

pub(super) async fn list_extensions(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
) -> Result<RebornExtensionListResponse, RebornServicesError> {
    let lifecycle = execute_lifecycle(
        facade,
        lifecycle_surface_context(caller),
        LifecycleProductAction::ExtensionList,
    )
    .await?;
    Ok(RebornExtensionListResponse {
        extensions: lifecycle_extension_infos(&lifecycle),
    })
}

pub(super) async fn list_extension_registry(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
) -> Result<RebornExtensionRegistryResponse, RebornServicesError> {
    let context = lifecycle_surface_context(caller);
    let (installed_result, registry_result) = tokio::join!(
        execute_lifecycle(
            facade,
            context.clone(),
            LifecycleProductAction::ExtensionList
        ),
        execute_lifecycle(
            facade,
            context,
            LifecycleProductAction::ExtensionSearch {
                query: String::new(),
            },
        ),
    );
    let (installed, registry) = (installed_result?, registry_result?);
    let installed_ids = match &installed.payload {
        Some(LifecycleProductPayload::ExtensionList { extensions, .. }) => extensions.as_slice(),
        _ => &[],
    }
    .iter()
    .map(|extension| extension.summary.package_ref.id.as_str().to_string())
    .collect::<HashSet<_>>();
    let registry_entries = match &registry.payload {
        Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) => extensions.as_slice(),
        _ => &[],
    };
    Ok(RebornExtensionRegistryResponse {
        entries: registry_entries
            .iter()
            .cloned()
            .map(|summary| registry_entry(summary, &installed_ids))
            .collect(),
    })
}

pub(super) async fn install_extension(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
) -> Result<RebornExtensionActionResponse, RebornServicesError> {
    let context = lifecycle_surface_context(caller);
    let lifecycle = execute_lifecycle(
        facade,
        context.clone(),
        LifecycleProductAction::ExtensionInstall { package_ref },
    )
    .await?;
    let projection = project_action_package_best_effort(facade, context, &lifecycle).await;
    Ok(action_response(&lifecycle, None, projection.as_ref()))
}

pub(super) async fn activate_extension(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
) -> Result<RebornExtensionActionResponse, RebornServicesError> {
    let context = lifecycle_surface_context(caller);
    let lifecycle = execute_lifecycle(
        facade,
        context.clone(),
        LifecycleProductAction::ExtensionActivate { package_ref },
    )
    .await?;
    let projection = project_action_package_best_effort(facade, context, &lifecycle).await;
    Ok(action_response(
        &lifecycle,
        Some(lifecycle.phase == LifecyclePhase::Active),
        projection.as_ref(),
    ))
}

pub(super) async fn remove_extension(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
) -> Result<RebornExtensionActionResponse, RebornServicesError> {
    let lifecycle = execute_lifecycle(
        facade,
        lifecycle_surface_context(caller),
        LifecycleProductAction::ExtensionRemove { package_ref },
    )
    .await?;
    Ok(action_response(&lifecycle, None, None))
}

async fn execute_lifecycle(
    facade: &dyn LifecycleProductFacade,
    context: LifecycleProductContext,
    action: LifecycleProductAction,
) -> Result<LifecycleProductResponse, RebornServicesError> {
    facade
        .execute(context, action)
        .await
        .map_err(map_lifecycle_error)
}

async fn project_action_package(
    facade: &dyn LifecycleProductFacade,
    context: LifecycleProductContext,
    lifecycle: &LifecycleProductResponse,
) -> Result<Option<LifecycleProductResponse>, RebornServicesError> {
    let Some(package_ref) = lifecycle.package_ref.clone() else {
        return Ok(None);
    };
    facade
        .project_package(context, package_ref)
        .await
        .map(Some)
        .map_err(map_lifecycle_error)
}

async fn project_action_package_best_effort(
    facade: &dyn LifecycleProductFacade,
    context: LifecycleProductContext,
    lifecycle: &LifecycleProductResponse,
) -> Option<LifecycleProductResponse> {
    // Install/activate already mutated lifecycle state. Projection only enriches
    // the response with onboarding copy, so failure must not turn a completed
    // action into a browser-visible mutation error.
    project_action_package(facade, context, lifecycle)
        .await
        .ok()
        .flatten()
}

fn lifecycle_surface_context(caller: WebUiAuthenticatedCaller) -> LifecycleProductContext {
    LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
        tenant_id: caller.tenant_id,
        user_id: caller.user_id,
        agent_id: caller.agent_id,
        project_id: caller.project_id,
    })
}

fn lifecycle_installed_extensions(
    lifecycle: &LifecycleProductResponse,
) -> Vec<LifecycleInstalledExtensionSummary> {
    match &lifecycle.payload {
        Some(LifecycleProductPayload::ExtensionList { extensions, .. }) => extensions.clone(),
        _ => Vec::new(),
    }
}

fn lifecycle_extension_infos(lifecycle: &LifecycleProductResponse) -> Vec<RebornExtensionInfo> {
    lifecycle_installed_extensions(lifecycle)
        .into_iter()
        .map(extension_info)
        .collect()
}

fn registry_entry(
    summary: LifecycleExtensionSummary,
    installed_ids: &HashSet<String>,
) -> RebornExtensionRegistryEntry {
    let kind = summary.runtime_kind.wire_kind().to_string();
    let installed = installed_ids.contains(summary.package_ref.id.as_str());
    RebornExtensionRegistryEntry {
        package_ref: summary.package_ref,
        display_name: summary.name,
        kind,
        description: summary.description,
        installed,
        keywords: Vec::new(),
        version: Some(summary.version),
    }
}

fn extension_info(installed: LifecycleInstalledExtensionSummary) -> RebornExtensionInfo {
    let phase = installed.phase;
    let onboarding = extension_onboarding::for_installed(&installed);
    let summary = installed.summary;
    RebornExtensionInfo {
        package_ref: summary.package_ref,
        display_name: summary.name,
        kind: summary.runtime_kind.wire_kind().to_string(),
        description: summary.description,
        authenticated: matches!(
            phase,
            LifecyclePhase::Active | LifecyclePhase::Activating | LifecyclePhase::Configured
        ),
        active: phase == LifecyclePhase::Active,
        tools: summary.visible_read_only_capability_ids,
        needs_setup: matches!(
            phase,
            LifecyclePhase::Installed | LifecyclePhase::Configured | LifecyclePhase::Failed
        ),
        has_auth: !summary.credential_requirements.is_empty(),
        activation_status: Some(phase_status(phase).to_string()),
        activation_error: None,
        version: Some(summary.version),
        onboarding_state: onboarding.state,
        onboarding: onboarding.onboarding,
    }
}

fn phase_status(phase: LifecyclePhase) -> &'static str {
    match phase {
        LifecyclePhase::Active => "active",
        LifecyclePhase::Disabled => "disabled",
        LifecyclePhase::Removed => "removed",
        LifecyclePhase::Failed => "failed",
        LifecyclePhase::UnsupportedOrLegacy => "unsupported",
        LifecyclePhase::Discovered => "available",
        LifecyclePhase::Installing => "installing",
        LifecyclePhase::Installed => "installed",
        LifecyclePhase::Configured => "configured",
        LifecyclePhase::Activating => "activating",
        LifecyclePhase::UpgradeRequired => "upgrade_required",
        LifecyclePhase::Removing => "removing",
    }
}

fn action_response(
    lifecycle: &LifecycleProductResponse,
    activated: Option<bool>,
    projection: Option<&LifecycleProductResponse>,
) -> RebornExtensionActionResponse {
    let success = !matches!(
        lifecycle.phase,
        LifecyclePhase::Failed | LifecyclePhase::UnsupportedOrLegacy
    );
    let onboarding = projection
        .map(extension_onboarding::from_lifecycle)
        .unwrap_or_else(extension_onboarding::ExtensionOnboarding::empty);
    RebornExtensionActionResponse {
        success,
        message: onboarding
            .instructions
            .clone()
            .or_else(|| lifecycle.message.clone())
            .unwrap_or_else(|| "Extension lifecycle action completed".to_string()),
        activated,
        auth_url: None,
        awaiting_token: onboarding.awaiting_token,
        instructions: onboarding.instructions,
        onboarding_state: onboarding.state,
        onboarding: onboarding.onboarding,
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};

    use super::*;
    use crate::{
        LifecycleExtensionCredentialRequirement, LifecycleExtensionCredentialSetup,
        LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind, LifecycleExtensionSource,
        LifecycleInstalledExtensionSummary, LifecyclePackageKind, ProductWorkflowError,
        RebornExtensionOnboardingState,
    };

    #[tokio::test]
    async fn install_action_projects_lifecycle_onboarding_when_available() {
        let facade = ActionProjectionFacade {
            projection_error: false,
        };

        let response = install_extension(&facade, caller(), package_ref())
            .await
            .expect("install response");

        assert!(response.success);
        assert_eq!(
            response.message,
            "Fixture needs a token before its tools can run."
        );
        assert_eq!(
            response.onboarding_state,
            Some(RebornExtensionOnboardingState::SetupRequired)
        );
        assert_eq!(response.awaiting_token, Some(true));
        assert_eq!(
            response
                .onboarding
                .as_ref()
                .and_then(|payload| payload.credential_instructions.as_deref()),
            Some("Paste the fixture token.")
        );
    }

    #[tokio::test]
    async fn install_action_keeps_success_when_message_projection_fails() {
        let facade = ActionProjectionFacade {
            projection_error: true,
        };

        let response = install_extension(&facade, caller(), package_ref())
            .await
            .expect("install response");

        assert!(response.success);
        assert_eq!(response.message, "Fixture installed.");
        assert!(response.onboarding_state.is_none());
        assert!(response.onboarding.is_none());
    }

    struct ActionProjectionFacade {
        projection_error: bool,
    }

    #[async_trait]
    impl LifecycleProductFacade for ActionProjectionFacade {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            assert!(matches!(
                action,
                LifecycleProductAction::ExtensionInstall { .. }
            ));
            Ok(LifecycleProductResponse {
                package_ref: Some(package_ref()),
                phase: LifecyclePhase::Installed,
                blockers: Vec::new(),
                message: Some("Fixture installed.".to_string()),
                payload: Some(LifecycleProductPayload::ExtensionInstall {
                    installed: true,
                    visible_capability_ids: Vec::new(),
                }),
            })
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            if self.projection_error {
                return Err(ProductWorkflowError::Transient {
                    reason: "projection unavailable".to_string(),
                });
            }
            Ok(LifecycleProductResponse {
                package_ref: Some(package_ref()),
                phase: LifecyclePhase::Installed,
                blockers: Vec::new(),
                message: None,
                payload: Some(LifecycleProductPayload::ExtensionList {
                    extensions: vec![LifecycleInstalledExtensionSummary {
                        summary: summary_with_onboarding(),
                        phase: LifecyclePhase::Installed,
                    }],
                    count: 1,
                }),
            })
        }
    }

    fn caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new("tenant-alpha").expect("valid tenant"),
            UserId::new("user-alpha").expect("valid user"),
            Some(AgentId::new("agent-alpha").expect("valid agent")),
            Some(ProjectId::new("project-alpha").expect("valid project")),
        )
    }

    fn package_ref() -> LifecyclePackageRef {
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("valid ref")
    }

    fn summary_with_onboarding() -> LifecycleExtensionSummary {
        LifecycleExtensionSummary {
            package_ref: package_ref(),
            name: "Fixture".to_string(),
            version: "1.0.0".to_string(),
            description: "test extension".to_string(),
            source: LifecycleExtensionSource::HostBundled,
            runtime_kind: LifecycleExtensionRuntimeKind::WasmTool,
            visible_read_only_capability_ids: Vec::new(),
            credential_requirements: vec![LifecycleExtensionCredentialRequirement {
                name: "fixture_token".to_string(),
                provider: "fixture".to_string(),
                required: true,
                setup: LifecycleExtensionCredentialSetup::ManualToken,
            }],
            onboarding: Some(LifecycleExtensionOnboarding {
                instructions: "Fixture needs a token before its tools can run.".to_string(),
                credential_instructions: Some("Paste the fixture token.".to_string()),
                setup_url: None,
                credential_next_step: Some(
                    "After saving the token, activate Fixture to publish its tools.".to_string(),
                ),
            }),
        }
    }
}

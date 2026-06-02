use std::collections::HashSet;

use crate::{
    LifecycleExtensionSummary, LifecycleInstalledExtensionSummary, LifecyclePackageRef,
    LifecyclePhase, LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleProductSurfaceContext,
    RebornExtensionActionResponse, RebornExtensionInfo, RebornExtensionListResponse,
    RebornExtensionRegistryEntry, RebornExtensionRegistryResponse, RebornServicesError,
    WebUiAuthenticatedCaller,
};

use super::lifecycle_setup::map_lifecycle_error;

pub(super) async fn list_extensions(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
) -> Result<RebornExtensionListResponse, RebornServicesError> {
    let lifecycle =
        execute_lifecycle(facade, caller, LifecycleProductAction::ExtensionList).await?;
    Ok(RebornExtensionListResponse {
        extensions: lifecycle_extension_infos(&lifecycle),
    })
}

pub(super) async fn list_extension_registry(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
) -> Result<RebornExtensionRegistryResponse, RebornServicesError> {
    let (installed_result, registry_result) = tokio::join!(
        execute_lifecycle(
            facade,
            caller.clone(),
            LifecycleProductAction::ExtensionList
        ),
        execute_lifecycle(
            facade,
            caller,
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
    let lifecycle = execute_lifecycle(
        facade,
        caller,
        LifecycleProductAction::ExtensionInstall { package_ref },
    )
    .await?;
    Ok(action_response(&lifecycle, None))
}

pub(super) async fn activate_extension(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
) -> Result<RebornExtensionActionResponse, RebornServicesError> {
    let lifecycle = execute_lifecycle(
        facade,
        caller,
        LifecycleProductAction::ExtensionActivate { package_ref },
    )
    .await?;
    Ok(action_response(
        &lifecycle,
        Some(lifecycle.phase == LifecyclePhase::Active),
    ))
}

pub(super) async fn remove_extension(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
) -> Result<RebornExtensionActionResponse, RebornServicesError> {
    let lifecycle = execute_lifecycle(
        facade,
        caller,
        LifecycleProductAction::ExtensionRemove { package_ref },
    )
    .await?;
    Ok(action_response(&lifecycle, None))
}

async fn execute_lifecycle(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
    action: LifecycleProductAction,
) -> Result<LifecycleProductResponse, RebornServicesError> {
    facade
        .execute(lifecycle_surface_context(caller), action)
        .await
        .map_err(map_lifecycle_error)
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
        has_auth: false,
        activation_status: Some(phase_status(phase).to_string()),
        activation_error: None,
        version: Some(summary.version),
        onboarding_state: None,
        onboarding: None,
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
) -> RebornExtensionActionResponse {
    let success = !matches!(
        lifecycle.phase,
        LifecyclePhase::Failed | LifecyclePhase::UnsupportedOrLegacy
    );
    RebornExtensionActionResponse {
        success,
        message: lifecycle
            .message
            .clone()
            .unwrap_or_else(|| "Extension lifecycle action completed".to_string()),
        activated,
        auth_url: None,
        awaiting_token: None,
        instructions: None,
        onboarding_state: None,
        onboarding: None,
    }
}

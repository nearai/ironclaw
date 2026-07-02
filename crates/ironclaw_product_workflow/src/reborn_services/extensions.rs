use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use futures::{StreamExt, TryStreamExt, stream};
use ironclaw_host_api::ExtensionId;

use crate::{
    ChannelConnectionFacade, LifecycleExtensionSummary, LifecycleExtensionSurfaceKind,
    LifecycleInstalledExtensionSummary, LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase,
    LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleProductSurfaceContext,
    RebornExtensionActionResponse, RebornExtensionInfo, RebornExtensionListResponse,
    RebornExtensionOnboardingState, RebornExtensionRegistryEntry, RebornExtensionRegistryResponse,
    RebornServicesError, WebUiAuthenticatedCaller,
};

use super::{
    ExtensionCredentialSetupService,
    extension_credentials::{
        ExtensionCredentialReadiness, credential_scope, readiness_for_requirements,
    },
    extension_onboarding,
    lifecycle_setup::map_lifecycle_error,
};

const EXTENSION_READINESS_CONCURRENCY: usize = 8;

pub(super) async fn list_extensions(
    facade: Arc<dyn LifecycleProductFacade>,
    extension_credentials: Option<Arc<dyn ExtensionCredentialSetupService>>,
    channel_connection_facade: Arc<dyn ChannelConnectionFacade>,
    caller: WebUiAuthenticatedCaller,
) -> Result<RebornExtensionListResponse, RebornServicesError> {
    let context = lifecycle_surface_context(caller.clone());
    let lifecycle = execute_lifecycle(
        facade.as_ref(),
        context,
        LifecycleProductAction::ExtensionList,
    )
    .await?;
    let installed = lifecycle_installed_extensions(&lifecycle);
    let connections = channel_connection_facade
        .caller_channel_connections(caller.clone())
        .await?;
    Ok(RebornExtensionListResponse {
        extensions: lifecycle_extension_infos(
            installed,
            extension_credentials,
            caller,
            connections,
        )
        .await?,
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
            .map(|extension| registry_entry(extension.summary, &installed_ids))
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
    channel_connection_facade: Arc<dyn ChannelConnectionFacade>,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
) -> Result<RebornExtensionActionResponse, RebornServicesError> {
    let context = lifecycle_surface_context(caller.clone());
    let channel = removable_channel_id(facade, context.clone(), &package_ref).await?;
    if let Some(channel) = channel.as_deref() {
        channel_connection_facade
            .disconnect_channel_for_caller(caller, channel)
            .await?;
    }
    let lifecycle = execute_lifecycle(
        facade,
        context,
        LifecycleProductAction::ExtensionRemove { package_ref },
    )
    .await?;
    Ok(action_response(&lifecycle, None, None))
}

async fn removable_channel_id(
    facade: &dyn LifecycleProductFacade,
    context: LifecycleProductContext,
    package_ref: &LifecyclePackageRef,
) -> Result<Option<String>, RebornServicesError> {
    if package_ref.kind != LifecyclePackageKind::Extension {
        return Ok(None);
    }
    let lifecycle =
        execute_lifecycle(facade, context, LifecycleProductAction::ExtensionList).await?;
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = lifecycle.payload else {
        return Ok(None);
    };
    Ok(extensions.into_iter().find_map(|installed| {
        (installed.summary.package_ref == *package_ref
            && has_external_channel_surface(&installed.summary))
        .then(|| installed.summary.package_ref.id.as_str().to_string())
    }))
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

async fn lifecycle_extension_infos(
    installed: Vec<LifecycleInstalledExtensionSummary>,
    extension_credentials: Option<Arc<dyn ExtensionCredentialSetupService>>,
    caller: WebUiAuthenticatedCaller,
    connections: HashMap<String, bool>,
) -> Result<Vec<RebornExtensionInfo>, RebornServicesError> {
    let resolved = stream::iter(installed)
        .map(|installed| {
            let caller = caller.clone();
            let extension_credentials = extension_credentials.clone();
            async move {
                let readiness = credential_readiness_for_extension(
                    extension_credentials.as_deref(),
                    &caller,
                    &installed,
                )
                .await?;
                Ok::<_, RebornServicesError>((installed, readiness))
            }
        })
        .buffered(EXTENSION_READINESS_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?;
    Ok(resolved
        .into_iter()
        .map(|(installed, readiness)| extension_info(installed, readiness, &connections))
        .collect())
}

fn registry_entry(
    summary: LifecycleExtensionSummary,
    installed_ids: &HashSet<String>,
) -> RebornExtensionRegistryEntry {
    let kind = extension_kind(&summary).to_string();
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

async fn credential_readiness_for_extension(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    caller: &WebUiAuthenticatedCaller,
    installed: &LifecycleInstalledExtensionSummary,
) -> Result<ExtensionCredentialReadiness, RebornServicesError> {
    let extension_id = ExtensionId::new(installed.summary.package_ref.id.as_str())
        .map_err(|_| RebornServicesError::internal_invariant())?;
    let scope = credential_scope(caller, &installed.summary.package_ref);
    readiness_for_requirements(
        extension_credentials,
        scope,
        &extension_id,
        &installed.summary.credential_requirements,
    )
    .await
}

fn extension_info(
    installed: LifecycleInstalledExtensionSummary,
    readiness: ExtensionCredentialReadiness,
    connections: &HashMap<String, bool>,
) -> RebornExtensionInfo {
    let phase = installed.phase;
    let has_auth = !installed.summary.credential_requirements.is_empty();
    let lifecycle_authenticated = matches!(
        phase,
        LifecyclePhase::Active | LifecyclePhase::Activating | LifecyclePhase::Configured
    );
    let authenticated = match readiness {
        ExtensionCredentialReadiness::NotRequired => lifecycle_authenticated,
        ExtensionCredentialReadiness::Configured => true,
        ExtensionCredentialReadiness::MissingRequired => false,
        ExtensionCredentialReadiness::Unknown => lifecycle_authenticated,
    };
    let onboarding =
        extension_onboarding::for_installed_with_credential_status(&installed, readiness);
    let summary = installed.summary;
    let has_external_channel_surface = has_external_channel_surface(&summary);
    let kind = extension_kind(&summary).to_string();
    let channel_unconnected = has_external_channel_surface
        && connections.get(summary.package_ref.id.as_str()) == Some(&false);
    // A channel extension the calling user has not personally connected (e.g.
    // Slack pairing) surfaces as `setup_required` so the WebUI shows the same
    // Configure affordance as a credential-gated extension. The per-user
    // connections map only contains channels with that concept; a connected
    // channel (value `true`) keeps its normal onboarding state, and this is
    // intentionally distinct from the admin Channels tab's `pairing_required`.
    let onboarding_state = if channel_unconnected {
        Some(RebornExtensionOnboardingState::SetupRequired)
    } else {
        onboarding.state
    };
    RebornExtensionInfo {
        package_ref: summary.package_ref,
        display_name: summary.name,
        kind,
        description: summary.description,
        authenticated: authenticated && !channel_unconnected,
        active: phase == LifecyclePhase::Active,
        tools: summary.visible_capability_ids,
        needs_setup: channel_unconnected
            || readiness == ExtensionCredentialReadiness::MissingRequired
            || matches!(
                phase,
                LifecyclePhase::Installed | LifecyclePhase::Configured | LifecyclePhase::Failed
            ),
        has_auth,
        activation_status: Some(phase_status(phase).to_string()),
        activation_error: None,
        version: Some(summary.version),
        onboarding_state,
        onboarding: onboarding.onboarding,
    }
}

fn extension_kind(summary: &LifecycleExtensionSummary) -> &'static str {
    if has_external_channel_surface(summary) {
        "channel"
    } else {
        summary.runtime_kind.wire_kind()
    }
}

fn has_external_channel_surface(summary: &LifecycleExtensionSummary) -> bool {
    summary
        .surface_kinds
        .contains(&LifecycleExtensionSurfaceKind::ExternalChannel)
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
    use std::{
        collections::HashSet,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use ironclaw_auth::{CredentialAccountId, CredentialAccountProjection};
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};

    use super::*;
    use crate::reborn_services::StaticChannelConnectionFacade;
    use crate::{
        ChannelConnectionFacade, ExtensionCredentialStatusRequest,
        ExtensionCredentialSubmitRequest, LifecycleExtensionCredentialRequirement,
        LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding,
        LifecycleExtensionRuntimeKind, LifecycleExtensionSource, LifecycleExtensionSurfaceKind,
        LifecycleInstalledExtensionSummary, LifecyclePackageKind, LifecycleSearchExtensionSummary,
        ProductWorkflowError, RebornExtensionOnboardingState, RebornServicesError,
        RebornServicesErrorCode, RebornServicesErrorKind, WebUiAuthenticatedCaller,
    };

    #[derive(Default)]
    struct TestConnections {
        connections: std::collections::HashMap<String, bool>,
        disconnects: Mutex<Vec<(UserId, String)>>,
        disconnect_failures: Mutex<usize>,
    }

    impl TestConnections {
        fn with_connections(entries: &[(&str, bool)]) -> Self {
            Self {
                connections: entries
                    .iter()
                    .map(|(key, value)| ((*key).to_string(), *value))
                    .collect(),
                disconnects: Mutex::new(Vec::new()),
                disconnect_failures: Mutex::new(0),
            }
        }

        fn disconnects(&self) -> Vec<(UserId, String)> {
            self.disconnects.lock().expect("lock").clone()
        }

        fn fail_next_disconnects(&self, count: usize) {
            *self.disconnect_failures.lock().expect("lock") = count;
        }
    }

    #[async_trait]
    impl ChannelConnectionFacade for TestConnections {
        async fn caller_channel_connections(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<std::collections::HashMap<String, bool>, RebornServicesError> {
            Ok(self.connections.clone())
        }

        async fn disconnect_channel_for_caller(
            &self,
            caller: WebUiAuthenticatedCaller,
            channel: &str,
        ) -> Result<(), RebornServicesError> {
            self.disconnects
                .lock()
                .expect("lock")
                .push((caller.user_id, channel.to_string()));
            let mut failures = self.disconnect_failures.lock().expect("lock");
            if *failures > 0 {
                *failures -= 1;
                return Err(RebornServicesError::service_unavailable(true));
            }
            Ok(())
        }
    }

    fn no_channel_connections() -> Arc<dyn ChannelConnectionFacade> {
        Arc::new(TestConnections::default())
    }

    fn channel_connections(entries: &[(&str, bool)]) -> Arc<dyn ChannelConnectionFacade> {
        Arc::new(TestConnections::with_connections(entries))
    }

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

    #[tokio::test]
    async fn remove_action_disconnects_callers_channel_when_removing_channel_extension() {
        let facade = RemoveFacade::default();
        let caller = caller();
        let connections = Arc::new(TestConnections::default());
        let channel_connections: Arc<dyn ChannelConnectionFacade> = connections.clone();

        let response =
            remove_extension(&facade, channel_connections, caller.clone(), package_ref())
                .await
                .expect("remove response");

        assert!(response.success);
        assert_eq!(
            connections.disconnects(),
            vec![(caller.user_id.clone(), "fixture".to_string())],
            "the WebUI remove path must clear the caller's per-channel personal connection"
        );
        let calls = facade.calls.lock().expect("lock");
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0].1, LifecycleProductAction::ExtensionList));
        let (context, action) = &calls[1];
        assert_eq!(
            *action,
            LifecycleProductAction::ExtensionRemove {
                package_ref: package_ref(),
            }
        );
        match context {
            LifecycleProductContext::Surface(surface) => {
                assert_eq!(surface.tenant_id, caller.tenant_id);
                assert_eq!(surface.user_id, caller.user_id);
                assert_eq!(surface.agent_id, caller.agent_id);
                assert_eq!(surface.project_id, caller.project_id);
            }
            other => panic!("unexpected lifecycle context: {other:?}"),
        }
    }

    #[tokio::test]
    async fn remove_action_does_not_remove_channel_extension_when_disconnect_fails() {
        let facade = RemoveFacade::default();
        let caller = caller();
        let connections = Arc::new(TestConnections::default());
        connections.fail_next_disconnects(1);
        let channel_connections: Arc<dyn ChannelConnectionFacade> = connections.clone();

        let error = remove_extension(&facade, channel_connections, caller.clone(), package_ref())
            .await
            .expect_err("disconnect failure must stop removal");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(
            connections.disconnects(),
            vec![(caller.user_id.clone(), "fixture".to_string())],
            "the caller's channel cleanup should be attempted"
        );
        let calls = facade.calls.lock().expect("lock");
        assert_eq!(calls.len(), 1);
        assert!(
            matches!(calls[0].1, LifecycleProductAction::ExtensionList),
            "extension removal must remain retryable when channel cleanup fails"
        );
    }

    #[tokio::test]
    async fn remove_action_stays_retryable_when_removal_fails_after_disconnect() {
        // Disconnect deliberately runs before `ExtensionRemove` and is
        // destructive: when removal then fails, the caller's pairing is
        // already gone and they re-pair afterwards (accepted cost). What must
        // hold is that the failure surfaces to the caller and a retry
        // converges — the pre-remove lookup still finds the installed package
        // and the disconnect is idempotent for an already-unpaired caller, so
        // removal is re-attempted instead of being skipped.
        let facade = RemoveFacade::default();
        facade.fail_next_removes(1);
        let caller = caller();
        let connections = Arc::new(TestConnections::default());
        let channel_connections: Arc<dyn ChannelConnectionFacade> = connections.clone();

        let error = remove_extension(
            &facade,
            channel_connections.clone(),
            caller.clone(),
            package_ref(),
        )
        .await
        .expect_err("removal failure must surface after disconnect succeeded");
        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert!(error.retryable, "transient removal failures stay retryable");

        remove_extension(&facade, channel_connections, caller.clone(), package_ref())
            .await
            .expect("retry converges once removal succeeds");

        assert_eq!(
            connections.disconnects(),
            vec![
                (caller.user_id.clone(), "fixture".to_string()),
                (caller.user_id.clone(), "fixture".to_string()),
            ],
            "each attempt re-runs the idempotent caller disconnect"
        );
        let calls = facade.calls.lock().expect("lock");
        let actions: Vec<_> = calls.iter().map(|(_, action)| action.clone()).collect();
        assert_eq!(actions.len(), 4, "list+remove per attempt");
        assert!(matches!(actions[0], LifecycleProductAction::ExtensionList));
        assert!(matches!(
            actions[1],
            LifecycleProductAction::ExtensionRemove { .. }
        ));
        assert!(matches!(actions[2], LifecycleProductAction::ExtensionList));
        assert!(matches!(
            actions[3],
            LifecycleProductAction::ExtensionRemove { .. }
        ));
    }

    #[tokio::test]
    async fn static_channel_connection_facade_fails_disconnect_closed() {
        let error = StaticChannelConnectionFacade
            .disconnect_channel_for_caller(caller(), "slack")
            .await
            .expect_err("unwired disconnect must not report success");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn remove_action_does_not_disconnect_non_channel_extensions() {
        let facade = RemoveFacade::non_channel();
        let caller = caller();
        let connections = Arc::new(TestConnections::default());
        let channel_connections: Arc<dyn ChannelConnectionFacade> = connections.clone();

        let response = remove_extension(&facade, channel_connections, caller, package_ref())
            .await
            .expect("remove response");

        assert!(response.success);
        assert!(
            connections.disconnects().is_empty(),
            "non-channel extension removal must not clear channel personal connections"
        );
        let calls = facade.calls.lock().expect("lock");
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0].1, LifecycleProductAction::ExtensionList));
        assert!(matches!(
            calls[1].1,
            LifecycleProductAction::ExtensionRemove { .. }
        ));
    }

    #[tokio::test]
    async fn list_marks_active_credentialed_extension_unauthenticated_without_caller_account() {
        let facade = ListingFacade {
            extension: LifecycleInstalledExtensionSummary {
                summary: summary_with_onboarding(),
                phase: LifecyclePhase::Active,
            },
        };
        let credentials = Arc::new(RecordingCredentials::default());
        let caller = caller();

        let credentials_service: Arc<dyn ExtensionCredentialSetupService> = credentials.clone();
        let response = list_extensions(
            Arc::new(facade),
            Some(credentials_service),
            no_channel_connections(),
            caller.clone(),
        )
        .await
        .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");

        assert!(extension.active, "lifecycle activation remains visible");
        assert!(
            !extension.authenticated,
            "credential readiness must be evaluated for the current caller"
        );
        assert!(extension.needs_setup);
        assert_eq!(
            extension.onboarding_state,
            Some(RebornExtensionOnboardingState::SetupRequired)
        );

        let requests = credentials.status_requests.lock().expect("lock");
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.scope.resource.tenant_id, caller.tenant_id);
        assert_eq!(request.scope.resource.user_id, caller.user_id);
        assert_eq!(request.scope.resource.agent_id, caller.agent_id);
        assert_eq!(request.scope.resource.project_id, caller.project_id);
        assert_eq!(request.provider.as_str(), "fixture");
        assert_eq!(request.requester_extension.as_str(), "fixture");
    }

    #[tokio::test]
    async fn list_preserves_lifecycle_state_when_credential_status_is_retryably_unavailable() {
        let facade = ListingFacade {
            extension: LifecycleInstalledExtensionSummary {
                summary: summary_with_onboarding(),
                phase: LifecyclePhase::Active,
            },
        };
        let credentials = UnavailableCredentials;

        let response = list_extensions(
            Arc::new(facade),
            Some(Arc::new(credentials)),
            no_channel_connections(),
            caller(),
        )
        .await
        .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");

        assert!(extension.active);
        assert!(
            extension.authenticated,
            "retryable status outages should not be projected as missing credentials"
        );
        assert!(!extension.needs_setup);
        assert!(extension.onboarding_state.is_none());
    }

    #[tokio::test]
    async fn list_marks_active_host_managed_credential_extension_ready_without_setup_prompt() {
        let facade = ListingFacade {
            extension: LifecycleInstalledExtensionSummary {
                summary: summary_without_browser_setup_credentials(),
                phase: LifecyclePhase::Active,
            },
        };
        let credentials = Arc::new(RecordingCredentials::default());
        let credentials_service: Arc<dyn ExtensionCredentialSetupService> = credentials.clone();

        let response = list_extensions(
            Arc::new(facade),
            Some(credentials_service),
            no_channel_connections(),
            caller(),
        )
        .await
        .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");

        assert!(extension.active);
        assert!(extension.authenticated);
        assert!(!extension.needs_setup);
        assert!(!extension.has_auth);
        assert!(extension.onboarding_state.is_none());
        assert!(extension.onboarding.is_none());
        assert!(
            credentials.status_requests.lock().expect("lock").is_empty(),
            "host-managed credentials must not trigger browser credential status checks"
        );
    }

    #[tokio::test]
    async fn list_checks_extension_readiness_with_bounded_concurrency() {
        let facade = MultiListingFacade {
            extensions: (0..EXTENSION_READINESS_CONCURRENCY + 3)
                .map(|index| LifecycleInstalledExtensionSummary {
                    summary: summary_with_onboarding_for(&format!("fixture-{index}")),
                    phase: LifecyclePhase::Active,
                })
                .collect(),
        };
        let credentials = Arc::new(ConcurrentCredentials::default());
        let credentials_service: Arc<dyn ExtensionCredentialSetupService> = credentials.clone();

        let response = list_extensions(
            Arc::new(facade),
            Some(credentials_service),
            no_channel_connections(),
            caller(),
        )
        .await
        .expect("list extensions");

        assert_eq!(
            response.extensions.len(),
            EXTENSION_READINESS_CONCURRENCY + 3
        );
        assert!(
            credentials.max_active.load(Ordering::SeqCst) > 1,
            "readiness checks should not run as a serialized page-load path"
        );
        assert!(
            credentials.max_active.load(Ordering::SeqCst) <= EXTENSION_READINESS_CONCURRENCY,
            "readiness checks must stay bounded"
        );
    }

    #[test]
    fn product_adapter_surface_projects_channel_kind() {
        let mut summary = summary_with_onboarding();
        summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
        summary.surface_kinds = vec![LifecycleExtensionSurfaceKind::ExternalChannel];

        let entry = registry_entry(summary, &HashSet::new());

        assert_eq!(entry.kind, "channel");
    }

    #[test]
    fn non_channel_extension_keeps_runtime_wire_kind() {
        // wasm_tool runtime with no channel surface → "wasm_tool"
        let mut wasm_summary = summary_with_onboarding();
        wasm_summary.runtime_kind = LifecycleExtensionRuntimeKind::WasmTool;
        wasm_summary.surface_kinds = Vec::new();
        assert_eq!(
            extension_kind(&wasm_summary),
            "wasm_tool",
            "WasmTool with empty surface_kinds must wire as wasm_tool"
        );

        // mcp_server runtime with no channel surface → "mcp_server"
        let mut mcp_summary = summary_with_onboarding();
        mcp_summary.runtime_kind = LifecycleExtensionRuntimeKind::McpServer;
        mcp_summary.surface_kinds = Vec::new();
        assert_eq!(
            extension_kind(&mcp_summary),
            "mcp_server",
            "McpServer with empty surface_kinds must wire as mcp_server"
        );

        // channel surface overrides runtime kind → "channel"
        let mut channel_summary = summary_with_onboarding();
        channel_summary.runtime_kind = LifecycleExtensionRuntimeKind::WasmTool;
        channel_summary.surface_kinds = vec![LifecycleExtensionSurfaceKind::ExternalChannel];
        assert_eq!(
            extension_kind(&channel_summary),
            "channel",
            "ExternalChannel surface must override runtime kind to channel"
        );
    }

    #[tokio::test]
    async fn list_projects_external_channel_surface_kind_through_extension_info() {
        let mut summary = summary_with_onboarding();
        summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
        summary.surface_kinds = vec![LifecycleExtensionSurfaceKind::ExternalChannel];
        summary.credential_requirements = Vec::new();
        let facade = ListingFacade {
            extension: LifecycleInstalledExtensionSummary {
                summary,
                phase: LifecyclePhase::Active,
            },
        };

        let response = list_extensions(Arc::new(facade), None, no_channel_connections(), caller())
            .await
            .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");

        assert_eq!(extension.kind, "channel");

        let mut unconnected_summary = summary_with_onboarding();
        unconnected_summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
        unconnected_summary.surface_kinds = vec![LifecycleExtensionSurfaceKind::ExternalChannel];
        unconnected_summary.credential_requirements = Vec::new();
        let unconnected = list_extensions(
            Arc::new(ListingFacade {
                extension: LifecycleInstalledExtensionSummary {
                    summary: unconnected_summary,
                    phase: LifecyclePhase::Active,
                },
            }),
            None,
            channel_connections(&[("fixture", false)]),
            caller(),
        )
        .await
        .expect("list extensions");
        assert_eq!(
            unconnected
                .extensions
                .first()
                .expect("one")
                .onboarding_state,
            Some(RebornExtensionOnboardingState::SetupRequired),
            "an unconnected channel must surface as setup_required for the Configure flow",
        );
        let unconnected_extension = unconnected.extensions.first().expect("one");
        assert!(
            !unconnected_extension.authenticated,
            "an unconnected channel must not look authenticated for the caller"
        );
        assert!(
            unconnected_extension.needs_setup,
            "an unconnected channel must keep the setup affordance visible"
        );
    }

    #[tokio::test]
    async fn list_extension_registry_projects_external_channel_kind_and_installed_status_from_webui_caller()
     {
        let caller = caller();
        let installed_summary = {
            let mut summary = summary_with_onboarding_for("installed-fixture");
            summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
            summary.surface_kinds = vec![LifecycleExtensionSurfaceKind::ExternalChannel];
            summary
        };
        let registry_installed_summary = installed_summary.clone();
        let registry_uninstalled_summary = {
            let mut summary = summary_with_onboarding_for("uninstalled-fixture");
            summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
            summary.surface_kinds = vec![LifecycleExtensionSurfaceKind::ExternalChannel];
            summary
        };
        let facade = RegistryListingFacade {
            installed: LifecycleInstalledExtensionSummary {
                summary: installed_summary,
                phase: LifecyclePhase::Active,
            },
            registry: vec![
                search_extension_summary(registry_installed_summary),
                search_extension_summary(registry_uninstalled_summary),
            ],
            calls: Mutex::new(Vec::new()),
        };

        let response = list_extension_registry(&facade, caller.clone())
            .await
            .expect("registry response");

        assert_eq!(response.entries.len(), 2);

        let installed_entry = response
            .entries
            .iter()
            .find(|entry| entry.package_ref.id.as_str() == "installed-fixture")
            .expect("installed entry");
        assert_eq!(installed_entry.kind, "channel");
        assert!(installed_entry.installed);

        let uninstalled_entry = response
            .entries
            .iter()
            .find(|entry| entry.package_ref.id.as_str() == "uninstalled-fixture")
            .expect("uninstalled entry");
        assert_eq!(uninstalled_entry.kind, "channel");
        assert!(!uninstalled_entry.installed);

        let calls = facade.calls.lock().expect("lock");
        assert_eq!(calls.len(), 2);
        for (context, action) in calls.iter() {
            match action {
                LifecycleProductAction::ExtensionList => {}
                LifecycleProductAction::ExtensionSearch { query } => {
                    assert!(query.is_empty(), "registry search uses the empty query");
                }
                other => panic!("unexpected lifecycle action: {other:?}"),
            }
            match context {
                LifecycleProductContext::Surface(surface) => {
                    assert_eq!(surface.tenant_id, caller.tenant_id);
                    assert_eq!(surface.user_id, caller.user_id);
                    assert_eq!(surface.agent_id, caller.agent_id);
                    assert_eq!(surface.project_id, caller.project_id);
                }
                other => panic!("unexpected lifecycle context: {other:?}"),
            }
        }
    }

    #[derive(Default)]
    struct RecordingCredentials {
        status_requests: Mutex<Vec<ExtensionCredentialStatusRequest>>,
    }

    #[async_trait]
    impl ExtensionCredentialSetupService for RecordingCredentials {
        async fn credential_status(
            &self,
            request: ExtensionCredentialStatusRequest,
        ) -> Result<Option<CredentialAccountProjection>, RebornServicesError> {
            self.status_requests.lock().expect("lock").push(request);
            Ok(None)
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, RebornServicesError> {
            Ok(CredentialAccountId::new())
        }
    }

    struct UnavailableCredentials;

    #[async_trait]
    impl ExtensionCredentialSetupService for UnavailableCredentials {
        async fn credential_status(
            &self,
            _request: ExtensionCredentialStatusRequest,
        ) -> Result<Option<CredentialAccountProjection>, RebornServicesError> {
            Err(RebornServicesError {
                code: RebornServicesErrorCode::Unavailable,
                kind: RebornServicesErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            })
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, RebornServicesError> {
            Ok(CredentialAccountId::new())
        }
    }

    #[derive(Default)]
    struct ConcurrentCredentials {
        active: AtomicUsize,
        max_active: AtomicUsize,
    }

    #[async_trait]
    impl ExtensionCredentialSetupService for ConcurrentCredentials {
        async fn credential_status(
            &self,
            _request: ExtensionCredentialStatusRequest,
        ) -> Result<Option<CredentialAccountProjection>, RebornServicesError> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(None)
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, RebornServicesError> {
            Ok(CredentialAccountId::new())
        }
    }

    struct ListingFacade {
        extension: LifecycleInstalledExtensionSummary,
    }

    #[async_trait]
    impl LifecycleProductFacade for ListingFacade {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            assert!(matches!(action, LifecycleProductAction::ExtensionList));
            Ok(LifecycleProductResponse {
                package_ref: None,
                phase: self.extension.phase,
                blockers: Vec::new(),
                message: None,
                payload: Some(LifecycleProductPayload::ExtensionList {
                    extensions: vec![self.extension.clone()],
                    count: 1,
                }),
            })
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            panic!("list_extensions should execute the list action, not project one package")
        }
    }

    struct MultiListingFacade {
        extensions: Vec<LifecycleInstalledExtensionSummary>,
    }

    #[async_trait]
    impl LifecycleProductFacade for MultiListingFacade {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            assert!(matches!(action, LifecycleProductAction::ExtensionList));
            Ok(LifecycleProductResponse {
                package_ref: None,
                phase: LifecyclePhase::Active,
                blockers: Vec::new(),
                message: None,
                payload: Some(LifecycleProductPayload::ExtensionList {
                    extensions: self.extensions.clone(),
                    count: self.extensions.len(),
                }),
            })
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            panic!("list_extensions should execute the list action, not project one package")
        }
    }

    struct RegistryListingFacade {
        installed: LifecycleInstalledExtensionSummary,
        registry: Vec<LifecycleSearchExtensionSummary>,
        calls: Mutex<Vec<(LifecycleProductContext, LifecycleProductAction)>>,
    }

    #[async_trait]
    impl LifecycleProductFacade for RegistryListingFacade {
        async fn execute(
            &self,
            context: LifecycleProductContext,
            action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            self.calls
                .lock()
                .expect("lock")
                .push((context.clone(), action.clone()));
            match action {
                LifecycleProductAction::ExtensionList => Ok(LifecycleProductResponse {
                    package_ref: None,
                    phase: self.installed.phase,
                    blockers: Vec::new(),
                    message: None,
                    payload: Some(LifecycleProductPayload::ExtensionList {
                        extensions: vec![self.installed.clone()],
                        count: 1,
                    }),
                }),
                LifecycleProductAction::ExtensionSearch { query } => {
                    assert!(query.is_empty(), "registry search uses the empty query");
                    Ok(LifecycleProductResponse {
                        package_ref: None,
                        phase: LifecyclePhase::Active,
                        blockers: Vec::new(),
                        message: None,
                        payload: Some(LifecycleProductPayload::ExtensionSearch {
                            extensions: self.registry.clone(),
                            count: self.registry.len(),
                        }),
                    })
                }
                other => panic!("unexpected lifecycle action: {other:?}"),
            }
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            panic!("list_extension_registry should not project one package")
        }
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
                    next_step: "Call builtin.extension_activate next.".to_string(),
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

    struct RemoveFacade {
        calls: Mutex<Vec<(LifecycleProductContext, LifecycleProductAction)>>,
        channel: bool,
        remove_failures: Mutex<usize>,
    }

    impl Default for RemoveFacade {
        fn default() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                channel: true,
                remove_failures: Mutex::new(0),
            }
        }
    }

    impl RemoveFacade {
        fn non_channel() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                channel: false,
                remove_failures: Mutex::new(0),
            }
        }

        fn fail_next_removes(&self, count: usize) {
            *self.remove_failures.lock().expect("lock") = count;
        }
    }

    #[async_trait]
    impl LifecycleProductFacade for RemoveFacade {
        async fn execute(
            &self,
            context: LifecycleProductContext,
            action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            self.calls
                .lock()
                .expect("lock")
                .push((context, action.clone()));
            match action {
                LifecycleProductAction::ExtensionList => {
                    let mut summary = summary_with_onboarding();
                    if self.channel {
                        summary.surface_kinds =
                            vec![LifecycleExtensionSurfaceKind::ExternalChannel];
                    }
                    Ok(LifecycleProductResponse {
                        package_ref: None,
                        phase: LifecyclePhase::Installed,
                        blockers: Vec::new(),
                        message: None,
                        payload: Some(LifecycleProductPayload::ExtensionList {
                            extensions: vec![LifecycleInstalledExtensionSummary {
                                summary,
                                phase: LifecyclePhase::Installed,
                            }],
                            count: 1,
                        }),
                    })
                }
                LifecycleProductAction::ExtensionRemove { package_ref } => {
                    {
                        let mut failures = self.remove_failures.lock().expect("lock");
                        if *failures > 0 {
                            *failures -= 1;
                            return Err(ProductWorkflowError::Transient {
                                reason: "extension removal unavailable".to_string(),
                            });
                        }
                    }
                    Ok(LifecycleProductResponse {
                        package_ref: Some(package_ref),
                        phase: LifecyclePhase::Removed,
                        blockers: Vec::new(),
                        message: Some("Fixture removed.".to_string()),
                        payload: Some(LifecycleProductPayload::ExtensionRemove { removed: true }),
                    })
                }
                other => panic!("unexpected lifecycle action: {other:?}"),
            }
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            _package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
            panic!("remove_extension should not project one package")
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
        summary_with_onboarding_for("fixture")
    }

    fn summary_with_onboarding_for(package_id: &str) -> LifecycleExtensionSummary {
        LifecycleExtensionSummary {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id)
                .expect("valid package ref"),
            name: "Fixture".to_string(),
            version: "1.0.0".to_string(),
            description: "test extension".to_string(),
            source: LifecycleExtensionSource::HostBundled,
            runtime_kind: LifecycleExtensionRuntimeKind::WasmTool,
            surface_kinds: Vec::new(),
            visible_capability_ids: Vec::new(),
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

    fn summary_without_browser_setup_credentials() -> LifecycleExtensionSummary {
        LifecycleExtensionSummary {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai")
                .expect("valid package ref"),
            name: "NEAR AI".to_string(),
            version: "1.0.0".to_string(),
            description: "host-managed MCP extension".to_string(),
            source: LifecycleExtensionSource::HostBundled,
            runtime_kind: LifecycleExtensionRuntimeKind::McpServer,
            surface_kinds: Vec::new(),
            visible_capability_ids: vec!["nearai.web_search".to_string()],
            visible_read_only_capability_ids: vec!["nearai.web_search".to_string()],
            credential_requirements: Vec::new(),
            onboarding: Some(LifecycleExtensionOnboarding {
                instructions: "NEAR AI MCP uses the NEAR AI credentials configured for the assistant. If NEAR AI is not configured yet, add a NEAR AI API key in assistant inference settings before activating this extension."
                    .to_string(),
                credential_instructions: Some(
                    "Configure NEAR AI for the assistant with an API key; MCP reuses that credential."
                        .to_string(),
                ),
                setup_url: None,
                credential_next_step: Some(
                    "After NEAR AI is configured for the assistant, activate NEAR AI MCP to publish its tools."
                        .to_string(),
                ),
            }),
        }
    }

    fn search_extension_summary(
        summary: LifecycleExtensionSummary,
    ) -> LifecycleSearchExtensionSummary {
        LifecycleSearchExtensionSummary {
            summary,
            installation_phase: None,
        }
    }
}

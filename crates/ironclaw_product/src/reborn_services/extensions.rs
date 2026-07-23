use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::{StreamExt, TryStreamExt, stream};
use ironclaw_auth::{CredentialAccountStatus, project_auth_account_state};
use ironclaw_host_api::{
    CapabilitySurfaceKind, ExtensionId, InstallationState, ProductSurfaceCaller,
    ProductSurfaceError, ProductSurfaceValidationCode,
};

use crate::{
    ChannelAuthAccountState, ChannelConnectionFacade, LifecycleExtensionSummary,
    LifecycleInstalledExtensionSummary, LifecycleProductAction, LifecycleProductContext,
    LifecycleProductFacade, LifecycleProductPayload, LifecycleProductResponse,
    LifecycleProductSurfaceContext, ProductView, RebornAccountBindingSource, RebornAuthAccount,
    RebornExtensionInfo, RebornExtensionListResponse, RebornExtensionOnboardingState,
    RebornExtensionRegistryEntry, RebornExtensionRegistryResponse, RebornExtensionSurface,
    RebornVendorAuthAccounts,
};

use super::{
    ExtensionCredentialSetupService,
    extension_credentials::{
        ExtensionCredentialReadiness, credential_scope, readiness_for_requirements,
    },
    extension_onboarding,
    lifecycle_setup::{map_lifecycle_error, validation_error},
};

const EXTENSION_READINESS_CONCURRENCY: usize = 8;

pub const EXTENSIONS_VIEW: ProductView<serde_json::Value, RebornExtensionListResponse> =
    ProductView::unpaginated("extensions");

pub const EXTENSION_REGISTRY_VIEW: ProductView<serde_json::Value, RebornExtensionRegistryResponse> =
    ProductView::unpaginated("extension_registry");

pub(super) async fn list_extensions(
    facade: Arc<dyn LifecycleProductFacade>,
    extension_credentials: Option<Arc<dyn ExtensionCredentialSetupService>>,
    channel_connection_facade: Arc<dyn ChannelConnectionFacade>,
    caller: ProductSurfaceCaller,
) -> Result<RebornExtensionListResponse, ProductSurfaceError> {
    let context = lifecycle_surface_context(caller.clone());
    let lifecycle = execute_lifecycle(
        facade.as_ref(),
        context.clone(),
        LifecycleProductAction::ExtensionList,
    )
    .await?;
    let installed = lifecycle_installed_extensions(&lifecycle);
    let connections = channel_connection_facade
        .caller_channel_connections(caller.clone())
        .await?;
    // Per-caller auth-account status per channel vendor: lets each account
    // project its real §6.3 state (expired / refresh-failed) instead of the
    // connected/disconnected collapse the connection bool alone permits.
    let account_states = channel_connection_facade
        .caller_channel_account_states(caller.clone())
        .await?;
    // Redacted per-extension activation errors from the durable installation
    // records, projected onto `RebornExtensionInfo::activation_error`.
    let activation_errors = facade
        .installed_activation_errors(context)
        .await
        .map_err(map_lifecycle_error)?;
    Ok(RebornExtensionListResponse {
        extensions: lifecycle_extension_infos(
            installed,
            extension_credentials,
            caller,
            connections,
            account_states,
            activation_errors,
        )
        .await?,
    })
}

pub(super) async fn list_extension_registry(
    facade: &dyn LifecycleProductFacade,
    caller: ProductSurfaceCaller,
) -> Result<RebornExtensionRegistryResponse, ProductSurfaceError> {
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

pub(super) async fn import_extension_capability(
    facade: &dyn LifecycleProductFacade,
    caller: ProductSurfaceCaller,
    input: serde_json::Value,
) -> Result<(), ProductSurfaceError> {
    let bundle_base64 = match input {
        serde_json::Value::Object(mut object) => object
            .remove("bundle_base64")
            .and_then(|value| value.as_str().map(ToString::to_string))
            .ok_or_else(|| {
                validation_error("bundle_base64", ProductSurfaceValidationCode::MissingField)
            })?,
        _ => {
            return Err(validation_error(
                "input",
                ProductSurfaceValidationCode::InvalidValue,
            ));
        }
    };
    let bundle = STANDARD.decode(bundle_base64).map_err(|_| {
        validation_error("bundle_base64", ProductSurfaceValidationCode::InvalidValue)
    })?;
    let context = lifecycle_surface_context(caller);
    facade
        .import_extension_bundle(context, bundle)
        .await
        .map_err(map_lifecycle_error)?;
    Ok(())
}

async fn execute_lifecycle(
    facade: &dyn LifecycleProductFacade,
    context: LifecycleProductContext,
    action: LifecycleProductAction,
) -> Result<LifecycleProductResponse, ProductSurfaceError> {
    facade
        .execute(context, action)
        .await
        .map_err(map_lifecycle_error)
}

fn lifecycle_surface_context(caller: ProductSurfaceCaller) -> LifecycleProductContext {
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
    caller: ProductSurfaceCaller,
    connections: HashMap<String, bool>,
    account_states: HashMap<String, ChannelAuthAccountState>,
    activation_errors: HashMap<String, String>,
) -> Result<Vec<RebornExtensionInfo>, ProductSurfaceError> {
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
                Ok::<_, ProductSurfaceError>((installed, readiness))
            }
        })
        .buffered(EXTENSION_READINESS_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?;
    Ok(resolved
        .into_iter()
        .map(|(installed, readiness)| {
            extension_info(
                installed,
                readiness,
                &connections,
                &account_states,
                &activation_errors,
            )
        })
        .collect())
}

fn registry_entry(
    summary: LifecycleExtensionSummary,
    installed_ids: &HashSet<String>,
) -> RebornExtensionRegistryEntry {
    let runtime = summary.runtime_kind.runtime_wire_name().to_string();
    let surfaces = wire_surfaces(&summary, None);
    let installed = installed_ids.contains(summary.package_ref.id.as_str());
    RebornExtensionRegistryEntry {
        package_ref: summary.package_ref,
        display_name: summary.name,
        runtime,
        description: summary.description,
        installed,
        keywords: Vec::new(),
        version: Some(summary.version),
        surfaces,
    }
}

async fn credential_readiness_for_extension(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    caller: &ProductSurfaceCaller,
    installed: &LifecycleInstalledExtensionSummary,
) -> Result<ExtensionCredentialReadiness, ProductSurfaceError> {
    let extension_id = ExtensionId::new(installed.summary.package_ref.id.as_str())
        .map_err(|_| ProductSurfaceError::internal_invariant())?;
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
    account_states: &HashMap<String, ChannelAuthAccountState>,
    activation_errors: &HashMap<String, String>,
) -> RebornExtensionInfo {
    let phase = installed.phase;
    let has_auth = !installed.summary.credential_requirements.is_empty();
    let lifecycle_authenticated = matches!(
        phase,
        InstallationState::Active | InstallationState::Configured
    );
    let authenticated = match readiness {
        ExtensionCredentialReadiness::NotRequired => lifecycle_authenticated,
        ExtensionCredentialReadiness::Configured => true,
        ExtensionCredentialReadiness::MissingRequired => false,
        ExtensionCredentialReadiness::Unknown => lifecycle_authenticated,
    };
    let onboarding =
        extension_onboarding::for_installed_with_credential_status(&installed, readiness);
    let install_scope = installed.install_scope;
    let summary = installed.summary;
    let has_external_channel_surface = has_external_channel_surface(&summary);
    let runtime = summary.runtime_kind.runtime_wire_name().to_string();
    let channel_unconnected = has_external_channel_surface
        && connections.get(summary.package_ref.id.as_str()) == Some(&false);
    // A channel extension the calling user has not personally connected (via
    // the vendor's OAuth) surfaces as `setup_required` so the WebUI shows the same
    // Configure affordance as a credential-gated extension. The per-user
    // connections map only contains channels with that concept; a connected
    // channel (value `true`) keeps its normal onboarding state, and this is
    // intentionally distinct from the admin Channels tab's `pairing_required`.
    let onboarding_state = if channel_unconnected {
        Some(RebornExtensionOnboardingState::SetupRequired)
    } else {
        onboarding.state
    };
    let connected = if has_external_channel_surface {
        connections.get(summary.package_ref.id.as_str()).copied()
    } else {
        None
    };
    let account_state = account_states.get(summary.package_ref.id.as_str());
    // Redacted activation error for this extension (host installation record's
    // typed `last_error`), threaded onto the card slot the frontend already
    // renders. `None` when the facade surfaces no failure for this extension.
    let activation_error = activation_errors
        .get(summary.package_ref.id.as_str())
        .cloned();
    let auth_accounts = vendor_auth_accounts(&summary, connected, account_state);
    let resolved_account_id = auth_accounts
        .first()
        .and_then(|vendor| vendor.accounts.first())
        .map(|account| account.account_id.clone());
    let surfaces = wire_surfaces(&summary, resolved_account_id);
    RebornExtensionInfo {
        package_ref: summary.package_ref,
        display_name: summary.name,
        runtime,
        description: summary.description,
        authenticated: authenticated && !channel_unconnected,
        active: phase == InstallationState::Active,
        tools: summary.visible_capability_ids,
        needs_setup: channel_unconnected
            || readiness == ExtensionCredentialReadiness::MissingRequired
            || matches!(
                phase,
                InstallationState::Installed
                    | InstallationState::Configured
                    | InstallationState::Failed
            ),
        has_auth,
        installation_state: wire_installation_state(phase, readiness),
        activation_error,
        version: Some(summary.version),
        onboarding_state,
        onboarding: onboarding.onboarding,
        auth_accounts,
        surfaces,
        install_scope,
    }
}

/// Wire surfaces for a lifecycle summary: tool/auth pass through; the
/// channel surface carries typed direction, the caller's connection state
/// (when a connections map applies), and the connect affordance.
fn wire_surfaces(
    summary: &LifecycleExtensionSummary,
    resolved_account_id: Option<String>,
) -> Vec<RebornExtensionSurface> {
    summary
        .surface_kinds
        .iter()
        .filter_map(|kind| match kind {
            CapabilitySurfaceKind::Tool => Some(RebornExtensionSurface::Tool),
            CapabilitySurfaceKind::Auth => Some(RebornExtensionSurface::Auth),
            CapabilitySurfaceKind::Channel => Some(RebornExtensionSurface::Channel {
                inbound: summary
                    .channel_directions
                    .map(|directions| directions.inbound)
                    .unwrap_or(false),
                outbound: summary
                    .channel_directions
                    .map(|directions| directions.outbound)
                    .unwrap_or(false),
                // Length ≤ 1 today: the surface resolves to its vendor's single
                // account through the default binding (ADR 0001, shape only).
                binding_source: resolved_account_id
                    .as_ref()
                    .map(|_| RebornAccountBindingSource::Default),
                resolved_account_id: resolved_account_id.clone(),
                connection: summary.channel_connection.clone(),
            }),
            // Reserved kinds have no manifest section yet, so no wire form.
            CapabilitySurfaceKind::Trigger | CapabilitySurfaceKind::File => None,
        })
        .collect()
}

fn has_external_channel_surface(summary: &LifecycleExtensionSummary) -> bool {
    summary
        .surface_kinds
        .contains(&CapabilitySurfaceKind::Channel)
}

/// The wire installation state (§6.1): the composition-projected state,
/// refined to `Configured` when the caller's required credentials are present
/// but the extension is only `Installed` (not yet active). The composition
/// projection already yields `Active` / `Disabled` / `Failed`; this adds the
/// derived `Configured` distinction the product layer can prove from
/// credential readiness.
fn wire_installation_state(
    projected: InstallationState,
    readiness: ExtensionCredentialReadiness,
) -> InstallationState {
    if projected == InstallationState::Installed
        && readiness == ExtensionCredentialReadiness::Configured
    {
        InstallationState::Configured
    } else {
        projected
    }
}

/// The credential-authority vendor a channel/auth surface binds. Prefers the
/// declared auth recipe vendor; falls back to the package id (today the two
/// real channel package ids equal their vendor ids).
fn channel_auth_vendor(summary: &LifecycleExtensionSummary) -> String {
    summary
        .credential_requirements
        .first()
        .map(|requirement| requirement.provider.clone())
        .unwrap_or_else(|| summary.package_ref.id.as_str().to_string())
}

/// Per-vendor accounts list for the extensions wire (overview §6.4, ADR 0001).
/// One vendor, at most one account today; the list shape is frozen so the
/// post-P7 multi-account follow-up lands without a wire break. `None`
/// connection signal (no per-caller connection concept) yields no vendor entry.
///
/// The account's state is the shared §6.3 machine, projected by
/// [`project_auth_account_state`] from the caller's durable auth-account signal
/// (`account_state`): a real credential-account status surfaces `expired` /
/// `refresh-failed` (with a typed `last_error`) and a live auth flow surfaces
/// `authenticating`. When the facade carries no richer status the connection
/// bool is the MIG-1 backfill — a live grant reads as a `configured` account
/// and projects `connected`.
fn vendor_auth_accounts(
    summary: &LifecycleExtensionSummary,
    connected: Option<bool>,
    account_state: Option<&ChannelAuthAccountState>,
) -> Vec<RebornVendorAuthAccounts> {
    let Some(is_connected) = connected else {
        return Vec::new();
    };
    let vendor = channel_auth_vendor(summary);
    // Prefer the facade's durable credential-account status; fall back to the
    // connection bool (a live grant backfills to `configured` → `connected`).
    let account_status = account_state
        .and_then(|state| state.account_status)
        .or(is_connected.then_some(CredentialAccountStatus::Configured));
    let active_flow_status = account_state.and_then(|state| state.active_flow_status);
    let (state, last_error) = project_auth_account_state(account_status, active_flow_status);
    vec![RebornVendorAuthAccounts {
        vendor: vendor.clone(),
        // One account per vendor today; its id is the vendor id until the
        // multi-account follow-up wires real per-account identity.
        accounts: vec![RebornAuthAccount {
            account_id: vendor,
            label: summary.name.clone(),
            state,
            last_error,
            is_default: true,
        }],
    }]
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
    use ironclaw_host_api::{AgentId, CapabilitySurfaceKind, ProjectId, TenantId, UserId};

    use super::*;
    use crate::reborn_services::StaticChannelConnectionFacade;
    use crate::{
        ChannelConnectionFacade, ExtensionCredentialStatusRequest,
        ExtensionCredentialSubmitRequest, LifecycleExtensionCredentialRequirement,
        LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding,
        LifecycleExtensionRuntimeKind, LifecycleExtensionSource,
        LifecycleInstalledExtensionSummary, LifecyclePackageKind, LifecyclePackageRef,
        LifecycleSearchExtensionSummary, ProductWorkflowError, RebornExtensionOnboardingState,
    };
    use ironclaw_host_api::{
        ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
    };

    #[derive(Default)]
    struct TestConnections {
        connections: std::collections::HashMap<String, bool>,
    }

    impl TestConnections {
        fn with_connections(entries: &[(&str, bool)]) -> Self {
            Self {
                connections: entries
                    .iter()
                    .map(|(key, value)| ((*key).to_string(), *value))
                    .collect(),
            }
        }
    }

    #[async_trait]
    impl ChannelConnectionFacade for TestConnections {
        async fn caller_channel_connections(
            &self,
            _caller: ProductSurfaceCaller,
        ) -> Result<std::collections::HashMap<String, bool>, ProductSurfaceError> {
            Ok(self.connections.clone())
        }
    }

    fn no_channel_connections() -> Arc<dyn ChannelConnectionFacade> {
        Arc::new(TestConnections::default())
    }

    fn channel_connections(entries: &[(&str, bool)]) -> Arc<dyn ChannelConnectionFacade> {
        Arc::new(TestConnections::with_connections(entries))
    }

    #[tokio::test]
    async fn static_channel_connection_facade_fails_disconnect_closed() {
        let error = StaticChannelConnectionFacade
            .disconnect_channel_for_caller(caller(), "slack")
            .await
            .expect_err("unwired disconnect must not report success");

        assert_eq!(error.code, ProductSurfaceErrorCode::Unavailable);
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn list_marks_active_credentialed_extension_unauthenticated_without_caller_account() {
        let facade = ListingFacade {
            extension: LifecycleInstalledExtensionSummary {
                summary: summary_with_onboarding(),
                phase: InstallationState::Active,
                install_scope: None,
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
                phase: InstallationState::Active,
                install_scope: None,
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
                phase: InstallationState::Active,
                install_scope: None,
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
                    phase: InstallationState::Active,
                    install_scope: None,
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
        summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];

        let entry = registry_entry(summary, &HashSet::new());

        assert!(
            entry
                .surfaces
                .iter()
                .any(|surface| matches!(surface, RebornExtensionSurface::Channel { .. })),
            "{:?}",
            entry.surfaces
        );
    }

    #[test]
    fn runtime_wire_names_are_implementation_labels_not_taxonomy() {
        // The wire carries the honest runtime name; product taxonomy travels
        // in `surfaces`, so runtime never masquerades as a product kind.
        assert_eq!(
            LifecycleExtensionRuntimeKind::WasmTool.runtime_wire_name(),
            "wasm"
        );
        assert_eq!(
            LifecycleExtensionRuntimeKind::McpServer.runtime_wire_name(),
            "mcp"
        );
        assert_eq!(
            LifecycleExtensionRuntimeKind::Script.runtime_wire_name(),
            "script"
        );

        // A channel-surface extension keeps its runtime label AND projects
        // the channel surface — two separate axes.
        let mut channel_summary = summary_with_onboarding();
        channel_summary.runtime_kind = LifecycleExtensionRuntimeKind::WasmTool;
        channel_summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
        assert_eq!(
            channel_summary.runtime_kind.runtime_wire_name(),
            "wasm",
            "runtime label is unchanged by the channel surface"
        );
        assert!(
            wire_surfaces(&channel_summary, None)
                .iter()
                .any(|surface| matches!(surface, RebornExtensionSurface::Channel { .. })),
            "the channel surface projects alongside the runtime label"
        );
    }

    #[tokio::test]
    async fn list_projects_external_channel_surface_kind_through_extension_info() {
        let mut summary = summary_with_onboarding();
        summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
        summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
        summary.credential_requirements = Vec::new();
        let facade = ListingFacade {
            extension: LifecycleInstalledExtensionSummary {
                summary,
                phase: InstallationState::Active,
                install_scope: None,
            },
        };

        let response = list_extensions(Arc::new(facade), None, no_channel_connections(), caller())
            .await
            .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");

        assert!(
            extension
                .surfaces
                .iter()
                .any(|surface| matches!(surface, RebornExtensionSurface::Channel { .. })),
            "{:?}",
            extension.surfaces
        );

        let mut unconnected_summary = summary_with_onboarding();
        unconnected_summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
        unconnected_summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
        unconnected_summary.credential_requirements = Vec::new();
        let unconnected = list_extensions(
            Arc::new(ListingFacade {
                extension: LifecycleInstalledExtensionSummary {
                    summary: unconnected_summary,
                    phase: InstallationState::Active,
                    install_scope: None,
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
            summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
            summary
        };
        let registry_installed_summary = installed_summary.clone();
        let registry_uninstalled_summary = {
            let mut summary = summary_with_onboarding_for("uninstalled-fixture");
            summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
            summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
            summary
        };
        let facade = RegistryListingFacade {
            installed: LifecycleInstalledExtensionSummary {
                summary: installed_summary,
                phase: InstallationState::Active,
                install_scope: None,
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
        assert!(
            installed_entry
                .surfaces
                .iter()
                .any(|surface| matches!(surface, RebornExtensionSurface::Channel { .. }))
        );
        assert!(installed_entry.installed);

        let uninstalled_entry = response
            .entries
            .iter()
            .find(|entry| entry.package_ref.id.as_str() == "uninstalled-fixture")
            .expect("uninstalled entry");
        assert!(
            uninstalled_entry
                .surfaces
                .iter()
                .any(|surface| matches!(surface, RebornExtensionSurface::Channel { .. }))
        );
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
        ) -> Result<Option<CredentialAccountProjection>, ProductSurfaceError> {
            self.status_requests.lock().expect("lock").push(request);
            Ok(None)
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, ProductSurfaceError> {
            Ok(CredentialAccountId::new())
        }
    }

    struct UnavailableCredentials;

    #[async_trait]
    impl ExtensionCredentialSetupService for UnavailableCredentials {
        async fn credential_status(
            &self,
            _request: ExtensionCredentialStatusRequest,
        ) -> Result<Option<CredentialAccountProjection>, ProductSurfaceError> {
            Err(ProductSurfaceError {
                code: ProductSurfaceErrorCode::Unavailable,
                kind: ProductSurfaceErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            })
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, ProductSurfaceError> {
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
        ) -> Result<Option<CredentialAccountProjection>, ProductSurfaceError> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(None)
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, ProductSurfaceError> {
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
                phase: InstallationState::Active,
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
                        phase: InstallationState::Active,
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

    fn caller() -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            TenantId::new("tenant-alpha").expect("valid tenant"),
            UserId::new("user-alpha").expect("valid user"),
            Some(AgentId::new("agent-alpha").expect("valid agent")),
            Some(ProjectId::new("project-alpha").expect("valid project")),
        )
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
            channel_directions: None,
            channel_connection: None,
            channel_presentation: None,
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
            channel_directions: None,
            channel_connection: None,
            channel_presentation: None,
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

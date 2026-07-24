use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use ironclaw_auth::{
    CredentialAccountId, CredentialAccountLabel, CredentialAccountProjection, CredentialOwnership,
};
use ironclaw_host_api::{AgentId, CapabilitySurfaceKind, ProjectId, TenantId, UserId};

use super::*;
use crate::reborn_services::StaticChannelConnectionFacade;
use crate::{
    ChannelConnectionFacade, ChannelConnectionRequirement, ExtensionCredentialStatusRequest,
    ExtensionCredentialSubmitRequest, LifecycleExtensionCredentialRequirement,
    LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind,
    LifecycleExtensionSource, LifecycleInstalledExtensionSummary, LifecyclePackageKind,
    LifecyclePackageRef, LifecycleSearchExtensionSummary, ProductWorkflowError,
    RebornChannelConnectStrategy,
};
use ironclaw_host_api::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
};

#[derive(Default)]
struct TestConnections {
    connections: std::collections::HashMap<String, bool>,
    account_states: std::collections::HashMap<String, ChannelAuthAccountState>,
}

impl TestConnections {
    fn with_connections(entries: &[(&str, bool)]) -> Self {
        Self {
            connections: entries
                .iter()
                .map(|(key, value)| ((*key).to_string(), *value))
                .collect(),
            account_states: std::collections::HashMap::new(),
        }
    }

    fn with_account_state(
        extension_id: &str,
        connected: bool,
        account_status: CredentialAccountStatus,
    ) -> Self {
        Self {
            connections: std::collections::HashMap::from([(extension_id.to_string(), connected)]),
            account_states: std::collections::HashMap::from([(
                extension_id.to_string(),
                ChannelAuthAccountState {
                    account_status: Some(account_status),
                    active_flow_status: None,
                },
            )]),
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

    async fn caller_channel_account_states(
        &self,
        _caller: ProductSurfaceCaller,
    ) -> Result<std::collections::HashMap<String, ChannelAuthAccountState>, ProductSurfaceError>
    {
        Ok(self.account_states.clone())
    }
}

fn no_channel_connections() -> Arc<dyn ChannelConnectionFacade> {
    Arc::new(TestConnections::default())
}

fn channel_connections(entries: &[(&str, bool)]) -> Arc<dyn ChannelConnectionFacade> {
    Arc::new(TestConnections::with_connections(entries))
}

fn test_channel_connection(strategy: RebornChannelConnectStrategy) -> ChannelConnectionRequirement {
    ChannelConnectionRequirement {
        channel: "fixture".to_string(),
        display_name: "Fixture Messenger".to_string(),
        strategy,
        instructions: "Connect Fixture Messenger.".to_string(),
        input_placeholder: String::new(),
        submit_label: "Connect".to_string(),
        error_message: "Connection failed.".to_string(),
    }
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
async fn list_projects_setup_needed_when_caller_lacks_required_account() {
    let facade = ListingFacade {
        extension: LifecycleInstalledExtensionSummary {
            summary: summary_with_onboarding(),
            phase: LifecyclePublicState::Active,
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

    assert_eq!(
        extension.installation_state,
        LifecyclePublicState::SetupNeeded
    );
    let wire = serde_json::to_value(extension).expect("serialize extension");
    for retired in [
        "authenticated",
        "active",
        "needs_setup",
        "has_auth",
        "onboarding_state",
    ] {
        assert!(
            wire.get(retired).is_none(),
            "extension projection must not serialize retired field {retired}"
        );
    }

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
async fn nonconfigured_durable_credential_status_overrides_active_runtime_checkpoint() {
    for account_status in [
        CredentialAccountStatus::Revoked,
        CredentialAccountStatus::Expired,
        CredentialAccountStatus::RefreshFailed,
        CredentialAccountStatus::Missing,
    ] {
        let facade = ListingFacade {
            extension: LifecycleInstalledExtensionSummary {
                summary: summary_with_onboarding(),
                phase: LifecyclePublicState::Active,
                install_scope: None,
            },
        };
        let credentials = Arc::new(RecordingCredentials::with_account_status(account_status));
        let credentials_service: Arc<dyn ExtensionCredentialSetupService> = credentials;

        let response = list_extensions(
            Arc::new(facade),
            Some(credentials_service),
            no_channel_connections(),
            caller(),
        )
        .await
        .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");

        assert_eq!(
            extension.installation_state,
            LifecyclePublicState::SetupNeeded,
            "{account_status:?} personal auth must prevent active projection"
        );
    }
}

#[tokio::test]
async fn list_preserves_lifecycle_state_when_credential_status_is_retryably_unavailable() {
    let facade = ListingFacade {
        extension: LifecycleInstalledExtensionSummary {
            summary: summary_with_onboarding(),
            phase: LifecyclePublicState::Active,
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

    assert_eq!(
        extension.installation_state,
        LifecyclePublicState::Active,
        "retryable status outages should not be projected as missing credentials",
    );
}

#[tokio::test]
async fn list_marks_active_host_managed_credential_extension_ready_without_setup_prompt() {
    let facade = ListingFacade {
        extension: LifecycleInstalledExtensionSummary {
            summary: summary_without_browser_setup_credentials(),
            phase: LifecyclePublicState::Active,
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

    assert_eq!(extension.installation_state, LifecyclePublicState::Active);
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
                phase: LifecyclePublicState::Active,
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
            phase: LifecyclePublicState::Active,
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
    unconnected_summary.channel_connection =
        Some(test_channel_connection(RebornChannelConnectStrategy::OAuth));
    unconnected_summary.credential_requirements = Vec::new();
    let unconnected = list_extensions(
        Arc::new(ListingFacade {
            extension: LifecycleInstalledExtensionSummary {
                summary: unconnected_summary,
                phase: LifecyclePublicState::Active,
                install_scope: None,
            },
        }),
        None,
        channel_connections(&[("fixture", false)]),
        caller(),
    )
    .await
    .expect("list extensions");
    let unconnected_extension = unconnected.extensions.first().expect("one");
    assert_eq!(
        unconnected_extension.installation_state,
        LifecyclePublicState::SetupNeeded,
        "an unconnected channel must keep the setup affordance visible"
    );
}

#[tokio::test]
async fn durable_personal_auth_failure_overrides_active_runtime_checkpoint() {
    for (account_status, expected_state, expected_error) in [
        (
            CredentialAccountStatus::Revoked,
            ironclaw_auth::AuthAccountState::Disconnected,
            Some(ironclaw_auth::AuthAccountLastError::GrantRevoked),
        ),
        (
            CredentialAccountStatus::Expired,
            ironclaw_auth::AuthAccountState::Expired,
            Some(ironclaw_auth::AuthAccountLastError::RefreshFailed),
        ),
        (
            CredentialAccountStatus::RefreshFailed,
            ironclaw_auth::AuthAccountState::Expired,
            Some(ironclaw_auth::AuthAccountLastError::RefreshFailed),
        ),
        (
            CredentialAccountStatus::Missing,
            ironclaw_auth::AuthAccountState::Disconnected,
            Some(ironclaw_auth::AuthAccountLastError::CredentialMissing),
        ),
    ] {
        let mut summary = summary_with_onboarding();
        summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
        summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
        summary.credential_requirements = Vec::new();
        summary.channel_connection =
            Some(test_channel_connection(RebornChannelConnectStrategy::OAuth));
        let facade = ListingFacade {
            extension: LifecycleInstalledExtensionSummary {
                summary,
                phase: LifecyclePublicState::Active,
                install_scope: None,
            },
        };
        let connections: Arc<dyn ChannelConnectionFacade> = Arc::new(
            TestConnections::with_account_state("fixture", true, account_status),
        );

        let response = list_extensions(Arc::new(facade), None, connections, caller())
            .await
            .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");
        assert_eq!(
            extension.installation_state,
            LifecyclePublicState::SetupNeeded,
            "{account_status:?} personal auth must prevent active projection"
        );
        let account = extension
            .auth_accounts
            .first()
            .and_then(|vendor| vendor.accounts.first())
            .expect("channel account");
        assert_eq!(account.state, expected_state);
        assert_eq!(account.last_error, expected_error);
    }
}

#[tokio::test]
async fn configured_channel_account_without_identity_binding_remains_setup_needed() {
    let mut summary = summary_with_onboarding();
    summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
    summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
    summary.credential_requirements = Vec::new();
    summary.channel_connection = Some(test_channel_connection(RebornChannelConnectStrategy::OAuth));
    let facade = ListingFacade {
        extension: LifecycleInstalledExtensionSummary {
            summary,
            phase: LifecyclePublicState::Active,
            install_scope: None,
        },
    };
    let connections: Arc<dyn ChannelConnectionFacade> = Arc::new(
        TestConnections::with_account_state("fixture", false, CredentialAccountStatus::Configured),
    );

    let response = list_extensions(Arc::new(facade), None, connections, caller())
        .await
        .expect("list extensions");
    let extension = response.extensions.first().expect("one extension");

    assert_eq!(
        extension.installation_state,
        LifecyclePublicState::SetupNeeded
    );
}

#[tokio::test]
async fn split_channel_snapshot_mismatch_fails_closed() {
    let mut summary = summary_with_onboarding();
    summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
    summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
    summary.credential_requirements = Vec::new();
    summary.channel_connection = Some(test_channel_connection(RebornChannelConnectStrategy::OAuth));
    let facade = ListingFacade {
        extension: LifecycleInstalledExtensionSummary {
            summary,
            phase: LifecyclePublicState::Active,
            install_scope: None,
        },
    };
    let connections: Arc<dyn ChannelConnectionFacade> = Arc::new(TestConnections {
        connections: std::collections::HashMap::from([("fixture".to_string(), true)]),
        account_states: std::collections::HashMap::from([(
            "fixture".to_string(),
            ChannelAuthAccountState::default(),
        )]),
    });

    let response = list_extensions(Arc::new(facade), None, connections, caller())
        .await
        .expect("list extensions");
    let extension = response.extensions.first().expect("one extension");

    assert_eq!(
        extension.installation_state,
        LifecyclePublicState::SetupNeeded,
        "a binding read from before an account removal must fail closed"
    );
}

#[tokio::test]
async fn pairing_only_channel_uses_binding_without_requiring_product_auth_account() {
    for (connected, expected_active) in [(true, true), (false, false)] {
        let mut summary = summary_with_onboarding();
        summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
        summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
        summary.credential_requirements = Vec::new();
        summary.channel_connection = Some(test_channel_connection(
            RebornChannelConnectStrategy::WebGeneratedCode,
        ));
        let facade = ListingFacade {
            extension: LifecycleInstalledExtensionSummary {
                summary,
                phase: LifecyclePublicState::Active,
                install_scope: None,
            },
        };
        let connections: Arc<dyn ChannelConnectionFacade> = Arc::new(TestConnections {
            connections: std::collections::HashMap::from([("fixture".to_string(), connected)]),
            // The generic account reader explicitly observed no product-auth
            // row. That is expected for a host-generated-code recipe.
            account_states: std::collections::HashMap::from([(
                "fixture".to_string(),
                ChannelAuthAccountState::default(),
            )]),
        });

        let response = list_extensions(Arc::new(facade), None, connections, caller())
            .await
            .expect("list extensions");
        let extension = response.extensions.first().expect("one extension");

        assert_eq!(
            extension.installation_state,
            if expected_active {
                LifecyclePublicState::Active
            } else {
                LifecyclePublicState::SetupNeeded
            }
        );
    }
}

#[tokio::test]
async fn admin_managed_channel_requires_neither_personal_binding_nor_account() {
    let mut summary = summary_with_onboarding();
    summary.runtime_kind = LifecycleExtensionRuntimeKind::FirstParty;
    summary.surface_kinds = vec![CapabilitySurfaceKind::Channel];
    summary.credential_requirements = Vec::new();
    summary.channel_connection = Some(test_channel_connection(
        RebornChannelConnectStrategy::AdminManagedChannels,
    ));
    let facade = ListingFacade {
        extension: LifecycleInstalledExtensionSummary {
            summary,
            phase: LifecyclePublicState::Active,
            install_scope: None,
        },
    };
    let connections: Arc<dyn ChannelConnectionFacade> = Arc::new(TestConnections {
        // The generic adapter may still carry a false entry for a channel
        // whose manifest declares no caller-owned connection.
        connections: std::collections::HashMap::from([("fixture".to_string(), false)]),
        account_states: std::collections::HashMap::from([(
            "fixture".to_string(),
            ChannelAuthAccountState::default(),
        )]),
    });

    let response = list_extensions(Arc::new(facade), None, connections, caller())
        .await
        .expect("list extensions");
    let extension = response.extensions.first().expect("one extension");

    assert_eq!(extension.installation_state, LifecyclePublicState::Active);
    assert!(extension.auth_accounts.is_empty());
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
            phase: LifecyclePublicState::Active,
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
    account_status: Option<CredentialAccountStatus>,
}

impl RecordingCredentials {
    fn with_account_status(account_status: CredentialAccountStatus) -> Self {
        Self {
            account_status: Some(account_status),
            ..Self::default()
        }
    }
}

#[async_trait]
impl ExtensionCredentialSetupService for RecordingCredentials {
    async fn credential_status(
        &self,
        request: ExtensionCredentialStatusRequest,
    ) -> Result<Option<CredentialAccountProjection>, ProductSurfaceError> {
        let account = self
            .account_status
            .map(|status| CredentialAccountProjection {
                id: CredentialAccountId::new(),
                provider: request.provider.clone(),
                label: CredentialAccountLabel::new("fixture account").expect("valid account label"),
                status,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                secret_handle_count: 1,
            });
        self.status_requests.lock().expect("lock").push(request);
        Ok(account)
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
            phase: LifecyclePublicState::Active,
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
                    phase: LifecyclePublicState::Active,
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
                "After saving the token, IronClaw finishes Fixture installation automatically and publishes its tools.".to_string(),
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
                "After NEAR AI is configured for the assistant, IronClaw finishes installation automatically and publishes its tools."
                    .to_string(),
            ),
        }),
    }
}

fn search_extension_summary(summary: LifecycleExtensionSummary) -> LifecycleSearchExtensionSummary {
    LifecycleSearchExtensionSummary {
        summary,
        installation_phase: None,
    }
}

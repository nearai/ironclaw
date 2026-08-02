//! Hosted-MCP registration protocol journeys.
//!
//! This is deliberately a new suite: existing integration files cover bundled
//! MCP discovery, while these scenarios need mutable authentication and OAuth
//! metadata fixtures for user-registered endpoints.

#[allow(dead_code)]
#[path = "../support/hosted_mcp_registration_server.rs"]
mod hosted_mcp_registration_server;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use hosted_mcp_registration_server::{
    HostedMcpAuthPolicy, HostedMcpRegistrationNetworkEgress, HostedMcpRegistrationServer,
    HostedMcpTool, ScriptedMetadataResponse,
};
use ironclaw_auth::{
    AuthContinuationRef, AuthProductError, AuthProductScope, AuthProviderClient, AuthProviderId,
    AuthSurface, AuthorizationCodeHash, CredentialAccountLabel, OAuthAuthorizationCode,
    OAuthAuthorizationUrl, OAuthProviderCallbackRequest, OAuthProviderExchange,
    OAuthProviderExchangeContext, OAuthProviderRefresh, OAuthProviderRefreshRequest,
    OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret, ProviderScope,
    RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest, RebornOAuthCallbackOutcome,
    RebornOAuthCallbackRequest, RebornOAuthStartFlowRequest,
};
use ironclaw_extension_contracts::hosted_mcp::{
    HostedMcpAuthSelection, HostedMcpEndpoint, RegisterHostedMcpRequest,
};
use ironclaw_extension_contracts::lifecycle_id::LifecyclePackageId;
use ironclaw_extension_host::lifecycle_test_support::{
    build_lifecycle_test_services, build_lifecycle_test_services_with_auth_provider,
    invoke_with_standalone_approval, lifecycle_product_context,
    rebuild_lifecycle_test_services_with_auth_provider, webui_gate_resource_scope_for_owner,
};
use ironclaw_extensions::ExtensionInstallationStorePort;
use ironclaw_host_api::{
    action::{NetworkPolicy, NetworkScheme, NetworkTargetPattern},
    capability::{CapabilityGrant, CapabilitySet, EffectKind, GrantConstraints},
    ids::{CapabilityGrantId, CapabilityId, ExtensionId, SecretHandle},
    mount::MountView,
    runtime::{RuntimeKind, TrustClass},
    scope::Principal,
};
use ironclaw_product::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction, LifecycleProductPayload,
};
use ironclaw_product_contracts::lifecycle_service::LifecycleProductService;
use ironclaw_product_contracts::surface::{ProductSurfaceErrorCode, ProductSurfaceErrorKind};
use ironclaw_secrets::SecretStorePort;
use secrecy::SecretString;
use serde_json::json;
use std::sync::{Arc, OnceLock};

fn live_network_egress() -> Arc<dyn ironclaw_network::NetworkHttpEgress> {
    Arc::new(ironclaw_network::PolicyNetworkHttpEgress::new(
        ironclaw_network::ReqwestNetworkTransport::default(),
    ))
}

fn runtime_context(
    scope: ironclaw_host_api::resource::ResourceScope,
    capability: &str,
) -> ironclaw_host_api::scope::ExecutionContext {
    let grantee = ExtensionId::new("hosted-mcp-registration-test").expect("test extension id");
    let mut context = ironclaw_host_api::scope::ExecutionContext::local_default(
        scope.user_id.clone(),
        grantee.clone(),
        RuntimeKind::Mcp,
        TrustClass::Sandbox,
        CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new(capability).expect("capability id"),
                grantee: Principal::Extension(grantee),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![
                        EffectKind::DispatchCapability,
                        EffectKind::Network,
                        EffectKind::UseSecret,
                    ],
                    mounts: MountView::default(),
                    network: NetworkPolicy {
                        allowed_targets: vec![NetworkTargetPattern {
                            scheme: Some(NetworkScheme::Https),
                            host_pattern: "mcp.example.test".to_string(),
                            port: None,
                        }],
                        deny_private_ip_ranges: true,
                        max_egress_bytes: Some(1_000_000),
                    },
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        },
        MountView::default(),
    )
    .expect("test execution context");
    let mut scope = scope;
    context.tenant_id = scope.tenant_id.clone();
    context.user_id = scope.user_id.clone();
    context.agent_id = scope.agent_id.clone();
    context.project_id = scope.project_id.clone();
    context.mission_id = scope.mission_id.clone();
    context.thread_id = scope.thread_id.clone();
    scope.invocation_id = context.invocation_id;
    context.resource_scope = scope;
    context.authenticated_actor_user_id = Some(context.user_id.clone());
    context.run_id = Some(ironclaw_host_api::ids::RunId::new());
    context
        .validate()
        .expect("test invocation context preserves scope invariants");
    context
}

fn registration_request(auth_selection: HostedMcpAuthSelection) -> RegisterHostedMcpRequest {
    RegisterHostedMcpRequest {
        desired_id: LifecyclePackageId::new("fixture").expect("package id"),
        desired_name: "Fixture MCP".to_string(),
        endpoint: HostedMcpEndpoint::new("https://mcp.example.test/mcp")
            .expect("public fixture endpoint"),
        auth_selection: Some(auth_selection),
    }
}

fn automatic_request() -> RegisterHostedMcpRequest {
    registration_request(HostedMcpAuthSelection::Auto)
}

fn fixture_package_ref() -> LifecyclePackageRef {
    LifecyclePackageRef::new(LifecyclePackageKind::Extension, "mcp-fixture")
        .expect("fixture package ref")
}

async fn install_fixture(
    services: &ironclaw_extension_host::lifecycle_test_support::ExtensionLifecycleTestServices,
    scope: ironclaw_host_api::resource::ResourceScope,
) -> ironclaw_product_contracts::package_lifecycle::LifecycleProductResponse {
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect("ordinary lifecycle install completes")
}

fn credential_provider_from_response(
    response: &ironclaw_product_contracts::package_lifecycle::LifecycleProductResponse,
) -> AuthProviderId {
    response
        .blockers
        .iter()
        .find_map(|blocker| {
            match blocker {
            ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential {
                ref_id: Some(provider),
            } => AuthProviderId::new(provider.as_str()).ok(),
            _ => None,
        }
        })
        .expect("credential blocker identifies the provider for the existing auth UI")
}

async fn submit_fixture_bearer(
    services: &ironclaw_extension_host::lifecycle_test_support::ExtensionLifecycleTestServices,
    scope: ironclaw_host_api::resource::ResourceScope,
    provider: AuthProviderId,
    token: &str,
) -> Result<ironclaw_auth::RebornManualTokenSubmitResponse, ironclaw_auth::RebornManualTokenError> {
    let auth_scope = AuthProductScope::credential_owner(&scope, AuthSurface::Api);
    let challenge = services
        .product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest::new(
            auth_scope.clone(),
            provider,
            CredentialAccountLabel::new("Fixture MCP").expect("credential label"),
            AuthContinuationRef::LifecycleActivation {
                package_ref: ironclaw_auth::LifecyclePackageRef::new("mcp-fixture")
                    .expect("auth package ref"),
            },
            Utc::now() + ChronoDuration::minutes(5),
        ))
        .await
        .expect("manual-token setup challenge");
    services
        .product_auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            auth_scope,
            challenge.interaction_id,
            SecretString::from(token.to_string()),
        ))
        .await
}

/// Vendor-SDK seam only: product-auth still owns durable account creation,
/// callback completion, secret-handle consumption, and lifecycle continuation.
struct FixtureOAuthProvider {
    secret_store: Arc<OnceLock<Arc<dyn SecretStorePort>>>,
    access_token: String,
}

#[async_trait]
impl AuthProviderClient for FixtureOAuthProvider {
    async fn exchange_callback(
        &self,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        let access_secret = SecretHandle::new(format!("hosted-oauth-access-{}", context.flow_id))
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        self.secret_store
            .get()
            .ok_or(AuthProductError::BackendUnavailable)?
            .put(
                context.scope.resource.clone(),
                access_secret.clone(),
                SecretString::from(self.access_token.clone()),
                None,
            )
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        Ok(OAuthProviderExchange {
            provider: request.provider,
            account_label: request.account_label,
            authorization_code_hash: request.authorization_code_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            access_secret,
            refresh_secret: None,
            scopes: request.scopes,
            account_id: None,
            provider_identity: None,
        })
    }

    async fn refresh_token(
        &self,
        _request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        Err(AuthProductError::RefreshFailed)
    }
}

fn fixture_digest(value: &str) -> String {
    format!(
        "{:064x}",
        value.bytes().fold(0_u64, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u64::from(byte))
        })
    )
}

async fn complete_fixture_oauth_callback(
    services: &ironclaw_extension_host::lifecycle_test_support::ExtensionLifecycleTestServices,
    scope: &ironclaw_host_api::resource::ResourceScope,
    provider: ironclaw_auth::AuthProviderId,
) -> Result<ironclaw_auth::RebornOAuthCallbackResponse, ironclaw_auth::RebornOAuthCallbackError> {
    let auth_scope = AuthProductScope::credential_owner(scope, AuthSurface::Api);
    let state_hash =
        OpaqueStateHash::new(fixture_digest("hosted-oauth-state")).expect("state digest");
    let pkce_hash =
        PkceVerifierHash::new(fixture_digest("hosted-oauth-pkce")).expect("PKCE digest");
    let flow = services
        .product_auth
        .start_setup_oauth_flow(RebornOAuthStartFlowRequest {
            flow_id: None,
            scope: auth_scope.clone(),
            provider: provider.clone(),
            requester_extension: Some(ExtensionId::new("mcp-fixture").expect("extension id")),
            authorization_url: OAuthAuthorizationUrl::new("https://auth.example.test/authorize")
                .expect("fixture authorization URL"),
            opaque_state_hash: state_hash.clone(),
            pkce_verifier_hash: pkce_hash.clone(),
            pkce_verifier: SecretString::from("hosted-oauth-pkce".to_string()),
            update_binding: None,
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: ironclaw_auth::LifecyclePackageRef::new("mcp-fixture")
                    .expect("auth package ref"),
            },
            expires_at: Utc::now() + ChronoDuration::minutes(5),
        })
        .await
        .expect("existing product-auth OAuth flow starts");
    services
        .product_auth
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: auth_scope,
            flow_id: flow.id,
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider,
                    account_label: CredentialAccountLabel::new("Fixture MCP OAuth")
                        .expect("credential label"),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "hosted-oauth-code".to_string(),
                    ))
                    .expect("authorization code"),
                    authorization_code_hash: AuthorizationCodeHash::new(fixture_digest(
                        "hosted-oauth-code",
                    ))
                    .expect("authorization code digest"),
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "hosted-oauth-pkce".to_string(),
                    ))
                    .expect("PKCE verifier"),
                    pkce_verifier_hash: pkce_hash,
                    scopes: Vec::<ProviderScope>::new(),
                },
            },
        })
        .await
}

fn mrc_trace_tools() -> Vec<HostedMcpTool> {
    let trace = serde_json::from_str::<serde_json::Value>(include_str!(
        "../fixtures/hosted_mcp/microsoft_mrc_streamable_http.json"
    ))
    .expect("committed MRC trace is valid JSON");
    trace["tools_list"]["names"]
        .as_array()
        .expect("trace records tool names")
        .iter()
        .map(|name| {
            HostedMcpTool::read_only(
                name.as_str().expect("trace tool name is a string"),
                json!({"source":"sanitized MRC trace"}),
            )
        })
        .collect()
}

/// Notion's authenticated catalog legitimately documents individual tools in
/// several kilobytes of prose.  Keep this fixture below the structural 16 KiB
/// field limit but above the former 2 KiB compatibility limit, so the OAuth
/// callback journey proves catalog preparation can publish it end-to-end.
fn notion_style_long_description() -> String {
    let paragraph = "Search and retrieve pages, databases, blocks, comments, and users that the connected Notion account can access. Use structured filters when available, preserve the caller's workspace boundaries, and return enough context for a follow-up read without changing any page content. ";
    let description = paragraph.repeat(24);
    assert!(description.len() > 2_048);
    assert!(description.len() <= 16 * 1024);
    description
}

#[tokio::test]
async fn no_auth_registration_story_replays_streamable_http_mrc_trace_through_real_lifecycle() {
    let server =
        HostedMcpRegistrationServer::start(HostedMcpAuthPolicy::NoAuth, mrc_trace_tools()).await;
    let services = build_lifecycle_test_services(
        "hosted-mcp-user",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
    )
    .await;
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-user");
    let registration_result = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await;
    let registration = match registration_result {
        Ok(response) => response,
        Err(error) => panic!(
            "public lifecycle action persists the tenant definition: {error:?}; fixture requests: {:?}",
            server.requests(),
        ),
    };
    assert_eq!(
        registration
            .package_ref
            .as_ref()
            .expect("package ref")
            .id
            .as_str(),
        "mcp-fixture"
    );
    assert_eq!(
        registration.phase,
        ironclaw_extension_contracts::state::InstallationState::Installed,
        "registration persists a catalog definition without implicit installation: {registration:#?}",
    );
    assert!(
        server.requests().is_empty(),
        "registration must not contact MCP"
    );
    assert!(
        services
            .extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capability projection")
            .is_empty(),
        "registration must not activate the definition"
    );
    let exact_retry = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("an exact tenant definition retry is idempotent");
    assert_eq!(exact_retry.package_ref, registration.package_ref);
    assert_eq!(
        exact_retry.phase,
        ironclaw_extension_contracts::state::InstallationState::Installed
    );
    let mut conflicting = automatic_request();
    conflicting.desired_name = "Different fixture MCP".to_string();
    assert!(
        services
            .lifecycle_service
            .execute(
                lifecycle_product_context(scope.clone()),
                LifecycleProductAction::ExtensionRegisterHostedMcp {
                    request: conflicting,
                },
            )
            .await
            .is_err(),
        "a different immutable definition with the same tenant package id conflicts"
    );
    let installed = install_fixture(&services, scope.clone()).await;
    assert_eq!(
        installed.phase,
        ironclaw_extension_contracts::state::InstallationState::Active
    );
    let active_capabilities = services
        .extension_management
        .active_model_visible_capabilities()
        .await
        .expect("registered package publishes its discovered capability");
    let capability_id = active_capabilities
        .iter()
        .find(|capability| {
            capability
                .id
                .as_str()
                .ends_with(".get_recent_azure_updates")
        })
        .map(|capability| capability.id.as_str().to_string())
        .unwrap_or_else(|| {
            panic!(
                "MRC trace publishes its documented read-only Azure list capability; actual ids: {:?}",
                active_capabilities
                    .iter()
                    .map(|capability| capability.id.as_str())
                    .collect::<Vec<_>>()
            )
        });
    let outcome = invoke_with_standalone_approval(
        &services,
        &capability_id,
        runtime_context(scope.clone(), &capability_id),
        json!({"skip": 0}),
    )
    .await;
    assert!(
        matches!(
            outcome,
            ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(_)
        ),
        "published MCP tool is callable through the real runtime: {outcome:?}; fixture requests: {:?}",
        server.requests(),
    );
    let requests = server.requests();
    let methods = requests
        .iter()
        .filter_map(|request| request.rpc_method.as_deref())
        .collect::<Vec<_>>();
    assert!(methods.contains(&"initialize"));
    assert!(methods.contains(&"tools/list"));
    assert!(methods.contains(&"tools/call"));
}

#[tokio::test]
async fn registered_but_never_installed_definition_survives_restart_and_installs_without_reregistration()
 {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::NoAuth,
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let services = build_lifecycle_test_services(
        "hosted-mcp-registered-only",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
    )
    .await;
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-registered-only");
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("registration persists the tenant definition without installing it");

    // Deliberately never install. Rebuild services over the same durable
    // backing (simulated restart) and confirm the registered definition is
    // still discoverable, even though restore only ever walked installation
    // rows — the durable `registered-definitions/{id}.json` row has no
    // installation row backing it here.
    let restored = rebuild_lifecycle_test_services_with_auth_provider(
        &services,
        "hosted-mcp-registered-only",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
        Arc::new(ironclaw_auth::UnavailableAuthProviderClient),
    )
    .await;

    let search = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionSearch {
                query: "Fixture MCP".to_string(),
            },
        )
        .await
        .expect("registered definition remains searchable after restart");
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = search.payload else {
        panic!("search returns extension summaries")
    };
    assert!(
        extensions
            .iter()
            .any(|extension| extension.summary.package_ref == fixture_package_ref()),
        "a registered-but-never-installed hosted MCP definition must survive restart: {extensions:#?}",
    );

    assert!(
        restored
            .extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capability projection")
            .is_empty(),
        "a registered-but-uninstalled definition must not be active or publish tools after restart",
    );

    let installed = install_fixture(&restored, scope).await;
    assert_eq!(
        installed.phase,
        ironclaw_extension_contracts::state::InstallationState::Active,
        "the restored definition installs without re-registration: {installed:#?}",
    );
}

#[tokio::test]
async fn reserved_hosted_mcp_id_is_rejected_without_registration_or_installation_side_effects() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::NoAuth,
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let services = build_lifecycle_test_services(
        "hosted-mcp-reserved-id",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
    )
    .await;
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-reserved-id");
    let mut request = automatic_request();
    request.desired_id = LifecyclePackageId::new("mcp-fixture").expect("package id is well-formed");

    let error = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp { request },
        )
        .await
        .expect_err("the reserved mcp- prefix must be rejected before admission");
    assert_eq!(error.code, ProductSurfaceErrorCode::InvalidRequest);
    assert_eq!(error.kind, ProductSurfaceErrorKind::Validation);
    assert_eq!(error.status_code, 400);
    assert!(!error.retryable);
    assert!(
        server.requests().is_empty(),
        "invalid registration must not contact the MCP"
    );

    let rejected_extension = ExtensionId::new("mcp-mcp-fixture").expect("derived extension id");
    let installation_store = services.extension_management.installation_store_for_test();
    assert!(
        installation_store
            .get_registered_package_definition(&rejected_extension)
            .await
            .expect("registered-definition lookup")
            .is_none(),
        "invalid registration must not create a tenant catalog definition"
    );
    let installations = installation_store
        .list_installations()
        .await
        .expect("installation-store readback");
    assert!(
        installations
            .iter()
            .all(|installation| installation.extension_id() != &rejected_extension),
        "invalid registration must not create an installation"
    );
}

#[tokio::test]
async fn tenant_definition_is_discoverable_but_installation_and_removal_stay_per_user() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::NoAuth,
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let services = build_lifecycle_test_services(
        "tenant-admin",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
    )
    .await;
    let admin_scope = webui_gate_resource_scope_for_owner("tenant-admin");
    let member_scope = webui_gate_resource_scope_for_owner("tenant-member");
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(admin_scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("tenant admin registers the definition");

    let admin_before = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(admin_scope.clone()),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("admin installation list");
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = admin_before.payload
    else {
        panic!("list returns installed extension summaries")
    };
    assert!(
        extensions
            .iter()
            .all(|extension| extension.summary.package_ref != fixture_package_ref()),
        "registration must not implicitly install for its creator"
    );

    let member_search = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(member_scope.clone()),
            LifecycleProductAction::ExtensionSearch {
                query: "Fixture MCP".to_string(),
            },
        )
        .await
        .expect("tenant member can discover the tenant definition");
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = member_search.payload
    else {
        panic!("search returns extension summaries")
    };
    assert!(
        extensions
            .iter()
            .any(|extension| extension.summary.package_ref == fixture_package_ref()),
        "tenant definition is shared for discovery"
    );
    let member_before = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(member_scope.clone()),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("member installation list");
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = member_before.payload
    else {
        panic!("list returns installed extension summaries")
    };
    assert!(
        extensions
            .iter()
            .all(|extension| extension.summary.package_ref != fixture_package_ref()),
        "shared definition is not installed for another user implicitly"
    );

    let member_install = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(member_scope.clone()),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect("member joins through the ordinary install lifecycle");
    assert_eq!(
        member_install.phase,
        ironclaw_extension_contracts::state::InstallationState::Active
    );

    let admin_install = install_fixture(&services, admin_scope.clone()).await;
    assert_eq!(
        admin_install.phase,
        ironclaw_extension_contracts::state::InstallationState::Active
    );

    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(admin_scope.clone()),
            LifecycleProductAction::ExtensionRemove {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect("admin removes only their installation membership");
    let admin_user_id = ironclaw_host_api::ids::UserId::new("tenant-admin").expect("admin user id");
    let member_user_id =
        ironclaw_host_api::ids::UserId::new("tenant-member").expect("member user id");
    let capabilities_after_admin_removal = services
        .extension_management
        .active_model_visible_capabilities()
        .await
        .expect("active capability projection after admin removal");
    let fixture_capability_after_removal = capabilities_after_admin_removal
        .iter()
        .find(|capability| capability.id.as_str().starts_with("mcp-fixture."))
        .expect(
            "member's installation keeps the fixture MCP capability published \
             after the admin's removal",
        );
    let owner_members = fixture_capability_after_removal
        .owner
        .members()
        .expect("hosted-MCP installations are member-scoped, not tenant-wide");
    assert!(
        !owner_members.contains(&admin_user_id),
        "removal must revoke the admin's publication membership on the shared \
         installation's capability, not just their own list view: {fixture_capability_after_removal:#?}"
    );
    assert!(
        owner_members.contains(&member_user_id),
        "the member's own membership must survive the admin's removal (per-user \
         scoping): {fixture_capability_after_removal:#?}"
    );
    let member_after = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(member_scope.clone()),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("member installation survives another user's removal");
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = member_after.payload
    else {
        panic!("list returns installed extension summaries")
    };
    assert!(
        extensions
            .iter()
            .any(|extension| extension.summary.package_ref == fixture_package_ref()),
        "member keeps their installation"
    );
    let admin_search = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(admin_scope),
            LifecycleProductAction::ExtensionSearch {
                query: "Fixture MCP".to_string(),
            },
        )
        .await
        .expect("removed definition remains in the tenant catalog");
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = admin_search.payload
    else {
        panic!("search returns extension summaries")
    };
    assert!(
        extensions
            .iter()
            .any(|extension| extension.summary.package_ref == fixture_package_ref()),
        "removal retains the tenant definition for later installs"
    );
}

#[tokio::test]
async fn bearer_registration_stays_setup_needed_until_the_existing_auth_continuation_succeeds() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::ExactBearerWithoutChallenge {
            token: "correct-token".to_string(),
        },
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let services = build_lifecycle_test_services(
        "hosted-mcp-bearer-user",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
    )
    .await;
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-bearer-user");
    let registration = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("bearer MCP definition is admitted before credentials exist");
    assert_eq!(
        registration.phase,
        ironclaw_extension_contracts::state::InstallationState::Installed
    );
    assert!(
        registration.blockers.is_empty(),
        "registration does not probe auth"
    );
    assert!(
        server.requests().is_empty(),
        "registration must not contact MCP"
    );
    let unfinished_retry = install_fixture(&services, scope.clone()).await;
    assert!(
        unfinished_retry.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
        )),
        "ordinary installation exposes credential readiness"
    );
    assert!(
        services
            .extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capability projection")
            .is_empty(),
        "unfinished auth publishes no MCP tools"
    );

    assert_eq!(
        unfinished_retry.phase,
        ironclaw_extension_contracts::state::InstallationState::Installed
    );

    let provider = credential_provider_from_response(&unfinished_retry);
    let wrong = submit_fixture_bearer(&services, scope.clone(), provider.clone(), "wrong-token")
        .await
        .expect("the secure form accepts an opaque token before the MCP validates it");
    assert_eq!(
        wrong.continuation,
        AuthContinuationRef::LifecycleActivation {
            package_ref: ironclaw_auth::LifecyclePackageRef::new("mcp-fixture")
                .expect("auth package ref")
        }
    );
    assert!(
        services
            .extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capability projection")
            .is_empty(),
        "failed auth continuation publishes no MCP tools"
    );
    let rejected_retry = install_fixture(&services, scope.clone()).await;
    assert_eq!(
        rejected_retry.phase,
        ironclaw_host_api::state::InstallationState::Installed,
        "a rejected bearer token keeps the install in setup-needed state"
    );
    assert_eq!(
        rejected_retry.message.as_deref(),
        Some("Hosted MCP rejected the bearer credentials; update them and retry activation."),
        "a rejected bearer token must surface the hosted-MCP setup reason: {rejected_retry:#?}"
    );
    assert!(
        rejected_retry.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_host_api::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
        )),
        "a rejected bearer token must retain the credential blocker: {rejected_retry:#?}"
    );

    let submitted = submit_fixture_bearer(&services, scope.clone(), provider, "correct-token")
        .await
        .expect("correct bearer completes the shared lifecycle continuation");
    assert_eq!(
        submitted.continuation,
        AuthContinuationRef::LifecycleActivation {
            package_ref: ironclaw_auth::LifecyclePackageRef::new("mcp-fixture")
                .expect("auth package ref")
        }
    );
    let completed_retry = install_fixture(&services, scope.clone()).await;
    let capabilities = services
        .extension_management
        .active_model_visible_capabilities()
        .await
        .expect("activated capability projection");
    assert!(
        capabilities
            .iter()
            .any(|capability| capability.id.as_str().ends_with(".search")),
        "successful auth continuation activates the existing installation; retry: {completed_retry:#?}; capabilities: {capabilities:#?}; fixture requests: {:#?}",
        server.requests(),
    );
    assert!(
        server.requests().iter().any(|request| {
            request.rpc_method.as_deref() == Some("tools/list") && request.authorization_matches
        }),
        "activation reaches the MCP with the host-injected bearer"
    );
    let capability = capabilities
        .into_iter()
        .find(|capability| capability.id.as_str().ends_with(".search"))
        .expect("bearer MCP search capability");
    let outcome = invoke_with_standalone_approval(
        &services,
        capability.id.as_str(),
        runtime_context(scope, capability.id.as_str()),
        json!({}),
    )
    .await;
    assert!(
        matches!(
            outcome,
            ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(_)
        ),
        "bearer MCP invocation must complete after setup: {outcome:#?}"
    );
    assert!(server.requests().iter().any(|request| {
        request.rpc_method.as_deref() == Some("tools/call") && request.authorization_matches
    }));
}

#[tokio::test]
async fn pending_oauth_registration_survives_fresh_restore_and_resumes_existing_setup() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuth {
            access_token: "oauth-token".to_string(),
        },
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let initial_secret_store = Arc::new(OnceLock::new());
    let services = build_lifecycle_test_services_with_auth_provider(
        "hosted-mcp-oauth-restore",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
        Arc::new(FixtureOAuthProvider {
            secret_store: Arc::clone(&initial_secret_store),
            access_token: "oauth-token".to_string(),
        }),
    )
    .await;
    assert!(initial_secret_store.set(services.secret_store()).is_ok());
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-oauth-restore");
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("registration admits the definition before setup");
    let pending = install_fixture(&services, scope.clone()).await;
    let pending_provider = credential_provider_from_response(&pending);
    assert!(pending.blockers.iter().any(|blocker| matches!(
        blocker,
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
    )));

    let restored_secret_store = Arc::new(OnceLock::new());
    let restored = rebuild_lifecycle_test_services_with_auth_provider(
        &services,
        "hosted-mcp-oauth-restore",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
        Arc::new(FixtureOAuthProvider {
            secret_store: Arc::clone(&restored_secret_store),
            access_token: "oauth-token".to_string(),
        }),
    )
    .await;
    assert!(restored_secret_store.set(restored.secret_store()).is_ok());

    // Drive the real activation caller directly, WITHOUT re-issuing
    // `ExtensionInstall` (which would side-effect-repair the lifecycle
    // registry). This proves the restore path itself registers a pending
    // hosted-MCP install so activation/OAuth-continuation resumes see the
    // credential blocker, not a masked "extension ... is not installed".
    let activation_only = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect("a restored pending hosted MCP install resumes via activation alone");
    assert!(
        activation_only.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
        )),
        "activation without a prior re-install must still surface the credential setup \
         blocker rather than \"is not installed\": {activation_only:#?}"
    );
    assert!(
        restored
            .extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capability projection")
            .is_empty(),
        "activation alone must not publish tools for the still-pending restored extension",
    );

    let resumed = install_fixture(&restored, scope).await;
    assert_eq!(
        credential_provider_from_response(&resumed),
        pending_provider,
        "a fresh host rebuild resumes the same generic OAuth setup requirement",
    );
    assert!(resumed.blockers.iter().any(|blocker| matches!(
        blocker,
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
    )));
    assert!(
        restored
            .extension_management
            .active_model_visible_capabilities()
            .await
            .expect("restored active capability projection")
            .is_empty(),
        "a pending OAuth install stays non-callable until the user completes setup",
    );
}

#[tokio::test]
async fn oauth_registration_discovers_standard_metadata_then_hands_off_to_generic_auth_setup() {
    let mut notion_search = HostedMcpTool::read_only("search", json!("ok"));
    notion_search.description = notion_style_long_description();
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuth {
            access_token: "oauth-token".to_string(),
        },
        vec![notion_search],
    )
    .await;
    let fixture_secret_store = Arc::new(OnceLock::new());
    let services = build_lifecycle_test_services_with_auth_provider(
        "hosted-mcp-oauth-user",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
        Arc::new(FixtureOAuthProvider {
            secret_store: Arc::clone(&fixture_secret_store),
            access_token: "oauth-token".to_string(),
        }),
    )
    .await;
    assert!(
        fixture_secret_store.set(services.secret_store()).is_ok(),
        "fixture OAuth provider binds the product-auth secret store once"
    );
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-oauth-user");

    let registration = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("OAuth MCP registration persists the tenant definition");

    assert_eq!(
        registration.phase,
        ironclaw_extension_contracts::state::InstallationState::Installed
    );
    assert!(
        registration.blockers.is_empty(),
        "registration does not probe auth"
    );
    assert!(
        server.requests().is_empty(),
        "registration must not contact MCP"
    );
    let install = install_fixture(&services, scope.clone()).await;
    let provider = credential_provider_from_response(&install);
    assert!(
        install.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
        )),
        "ordinary install discovers OAuth and returns the credential setup blocker: {install:#?}"
    );
    assert!(
        services
            .extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capability projection")
            .is_empty(),
        "the protected MCP publishes no tools before the user finishes the existing OAuth setup"
    );

    let requests = server.requests();
    assert!(
        requests.iter().any(|request| {
            request.method == "GET" && request.path == "/.well-known/oauth-protected-resource"
        }),
        "the 401 challenge must lead to protected-resource metadata discovery: {requests:#?}"
    );
    assert!(
        requests.iter().any(|request| {
            request.method == "GET" && request.path == "/.well-known/oauth-authorization-server"
        }),
        "the admitted resource metadata must lead to authorization-server metadata discovery: {requests:#?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.rpc_method.as_deref() == Some("initialize")),
        "the generic MCP discovery attempt must have received the OAuth challenge"
    );
    assert!(
        provider.as_str().starts_with("mcp-"),
        "the metadata-derived credential requirement remains a generic provider identity"
    );

    let completed = complete_fixture_oauth_callback(&services, &scope, provider.clone())
        .await
        .expect("OAuth callback uses the production continuation dispatcher");
    assert_eq!(
        completed.continuation,
        AuthContinuationRef::LifecycleActivation {
            package_ref: ironclaw_auth::LifecyclePackageRef::new("mcp-fixture")
                .expect("auth package ref"),
        }
    );
    let requester = ExtensionId::new("mcp-fixture").expect("extension id");
    let provider_vendor = ironclaw_host_api::ids::VendorId::new(provider.as_str())
        .expect("metadata-derived provider remains a vendor id");
    let account_request = ironclaw_auth::runtime_credential_account_selection_request(
        &scope,
        &provider_vendor,
        ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
        &[],
        &requester,
    )
    .expect("runtime credential request");
    let account = services
        .product_auth
        .runtime_credential_account_selection_service()
        .select_unique_configured_runtime_account(account_request)
        .await
        .expect("completed OAuth account is selectable by the MCP extension");
    let access_secret = account
        .access_secret
        .as_ref()
        .expect("completed OAuth account retains an access secret");
    assert!(
        services
            .secret_store()
            .metadata(&account.scope.resource, access_secret)
            .await
            .expect("OAuth access-secret metadata read")
            .is_some(),
        "completed OAuth access-secret material remains in the shared secret store"
    );
    // Callback success is not sufficient: the ordinary lifecycle continuation
    // must replace the provisional hosted-MCP manifest. This is the durable
    // boundary behind the WebUI's "Finish setup" versus ready presentation.
    let installation_store = services.extension_management.installation_store_for_test();
    let installation = installation_store
        .list_installations()
        .await
        .expect("installation-store readback")
        .into_iter()
        .find(|installation| installation.extension_id().as_str() == "mcp-fixture")
        .expect("OAuth installation remains durable after callback");
    let manifest = installation_store
        .get_manifest(installation.extension_id())
        .await
        .expect("installed manifest readback")
        .expect("OAuth installation keeps its manifest");
    assert!(
        manifest.resolved().has_model_visible_capabilities(),
        "a callback that publishes tools must durably record them: {manifest:#?}"
    );
    let listed = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("installation list after OAuth callback");
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = listed.payload else {
        panic!("list returns installed extension summaries")
    };
    assert!(
        extensions.iter().any(|extension| {
            extension.summary.package_ref == fixture_package_ref()
                && extension.phase == ironclaw_extension_contracts::state::InstallationState::Active
        }),
        "a callback that publishes tools must project as active: {extensions:#?}"
    );
    let capability = services
        .extension_management
        .active_model_visible_capabilities()
        .await
        .expect("callback activation publishes the OAuth MCP tool")
        .into_iter()
        .find(|capability| capability.id.as_str().ends_with(".search"))
        .expect("OAuth MCP search capability");
    assert!(
        capability.effects.contains(&EffectKind::UseSecret),
        "credentialed MCP capability must declare the use-secret effect: {capability:#?}"
    );
    assert_eq!(
        capability.runtime_credentials.len(),
        1,
        "credentialed MCP capability must retain its product-auth obligation: {capability:#?}"
    );
    let outcome = invoke_with_standalone_approval(
        &services,
        capability.id.as_str(),
        runtime_context(scope, capability.id.as_str()),
        json!({}),
    )
    .await;
    assert!(
        matches!(
            outcome,
            ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(_)
        ),
        "OAuth MCP invocation must complete after callback activation: {outcome:#?}; requests: {:#?}",
        server.requests()
    );
    assert!(server.requests().iter().any(|request| {
        request.rpc_method.as_deref() == Some("tools/call") && request.authorization_matches
    }));
}

/// Drives all four `fetch_oauth_metadata` failure branches (transport error,
/// non-200 status, oversized body, malformed JSON) through the ordinary
/// install path. Each sub-case gets its own fixture server/services (the
/// scripted overrides are one-shot), but shares one assertion: the failure
/// must surface (as an error, or as a still-installed non-progressing
/// response) and the definition must remain unprepared, with no tools
/// published.
#[tokio::test]
async fn oauth_metadata_fetch_failures_leave_the_definition_unprepared() {
    /// Discriminates the two distinct outcomes `fetch_oauth_metadata`
    /// failures must produce, per sub-case (see the comment above the
    /// `match install_result` below for which sub-case gets which).
    enum ExpectedInstallOutcome {
        /// Swallowed into a still-installed, non-progressing response.
        SwallowedIntoInstalled,
        /// Propagates as a real `ProductSurfaceErrorKind::Validation` error.
        PropagatedValidationError,
    }

    async fn assert_metadata_failure_blocks_preparation(
        user_id: &str,
        egress: Arc<dyn ironclaw_network::NetworkHttpEgress>,
        expected: ExpectedInstallOutcome,
    ) {
        let fixture_secret_store = Arc::new(OnceLock::new());
        let services = build_lifecycle_test_services_with_auth_provider(
            user_id,
            Some(egress),
            false,
            Arc::new(FixtureOAuthProvider {
                secret_store: Arc::clone(&fixture_secret_store),
                access_token: "oauth-token".to_string(),
            }),
        )
        .await;
        assert!(fixture_secret_store.set(services.secret_store()).is_ok());
        let scope = webui_gate_resource_scope_for_owner(user_id);
        services
            .lifecycle_service
            .execute(
                lifecycle_product_context(scope.clone()),
                LifecycleProductAction::ExtensionRegisterHostedMcp {
                    request: automatic_request(),
                },
            )
            .await
            .expect("OAuth registration persists the tenant definition");

        let install_result = services
            .lifecycle_service
            .execute(
                lifecycle_product_context(scope),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: fixture_package_ref(),
                },
            )
            .await;
        // The transport-error branch is swallowed into a still-installed
        // response (mirrors `install_activation_error`'s `Transient` arm);
        // the non-200 and malformed-JSON branches propagate as a real
        // error. Either way, no blockers get fabricated and no OAuth setup
        // requirement is discovered.
        match expected {
            ExpectedInstallOutcome::SwallowedIntoInstalled => {
                let response = install_result.unwrap_or_else(|error| {
                    panic!(
                        "a transport-error metadata fetch must swallow into a \
                         still-installed response, not propagate an error: {error:#?}"
                    )
                });
                assert!(
                    response.blockers.is_empty(),
                    "a metadata-fetch failure surfaces as a still-installed response, not a \
                     fabricated credential blocker: {response:#?}"
                );
            }
            ExpectedInstallOutcome::PropagatedValidationError => {
                let error = install_result.err().unwrap_or_else(|| {
                    panic!(
                        "a non-200/oversized/malformed metadata fetch must propagate as a \
                         real error, not a still-installed response"
                    )
                });
                assert_eq!(
                    error.kind,
                    ProductSurfaceErrorKind::Validation,
                    "a non-transient metadata-fetch failure surfaces as an invalid binding \
                     request: {error:#?}"
                );
            }
        }
        assert!(
            services
                .extension_management
                .active_model_visible_capabilities()
                .await
                .expect("active capability projection")
                .is_empty(),
            "a metadata-fetch failure must not publish any tools"
        );
        let installation_store = services.extension_management.installation_store_for_test();
        let manifest = installation_store
            .get_manifest(&ExtensionId::new("mcp-fixture").expect("extension id"))
            .await
            .expect("installed manifest readback")
            .expect("fixture manifest remains durable after a metadata-fetch failure");
        assert!(
            !manifest.resolved().has_model_visible_capabilities(),
            "a metadata-fetch failure must leave the definition publishing nothing: {manifest:#?}"
        );
    }

    let oauth_policy = || HostedMcpAuthPolicy::OAuth {
        access_token: "oauth-token".to_string(),
    };
    let search_tool = || vec![HostedMcpTool::read_only("search", json!("ok"))];

    // Sub-case 1: transport error on the protected-resource fetch.
    let transport_server = HostedMcpRegistrationServer::start(oauth_policy(), search_tool()).await;
    let transport_egress = Arc::new(
        HostedMcpRegistrationNetworkEgress::for_server_with_transport_failure_on(
            &transport_server,
            "/.well-known/oauth-protected-resource",
        ),
    );
    assert_metadata_failure_blocks_preparation(
        "hosted-mcp-oauth-metadata-transport",
        transport_egress,
        ExpectedInstallOutcome::SwallowedIntoInstalled,
    )
    .await;

    // Sub-case 2: non-200 status on the protected-resource fetch.
    let non200_server = HostedMcpRegistrationServer::start(oauth_policy(), search_tool()).await;
    non200_server.script_protected_resource_response(ScriptedMetadataResponse::new(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        b"boom".to_vec(),
    ));
    let non200_egress = Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
        &non200_server,
    ));
    assert_metadata_failure_blocks_preparation(
        "hosted-mcp-oauth-metadata-non200",
        non200_egress,
        ExpectedInstallOutcome::PropagatedValidationError,
    )
    .await;

    // Sub-case 3: an oversized (>64 KiB) protected-resource body.
    let oversized_server = HostedMcpRegistrationServer::start(oauth_policy(), search_tool()).await;
    oversized_server.script_protected_resource_response(ScriptedMetadataResponse::new(
        axum::http::StatusCode::OK,
        vec![b'a'; 64 * 1024 + 1],
    ));
    let oversized_egress = Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
        &oversized_server,
    ));
    assert_metadata_failure_blocks_preparation(
        "hosted-mcp-oauth-metadata-oversized",
        oversized_egress,
        ExpectedInstallOutcome::PropagatedValidationError,
    )
    .await;

    // Sub-case 4: malformed JSON in the protected-resource body.
    let malformed_server = HostedMcpRegistrationServer::start(oauth_policy(), search_tool()).await;
    malformed_server.script_protected_resource_response(ScriptedMetadataResponse::new(
        axum::http::StatusCode::OK,
        b"not-json".to_vec(),
    ));
    let malformed_egress = Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
        &malformed_server,
    ));
    assert_metadata_failure_blocks_preparation(
        "hosted-mcp-oauth-metadata-malformed",
        malformed_egress,
        ExpectedInstallOutcome::PropagatedValidationError,
    )
    .await;
}

#[tokio::test]
async fn oauth_callback_with_a_rejected_token_stays_setup_needed_and_publishes_no_tools() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuth {
            access_token: "accepted-token".to_string(),
        },
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let fixture_secret_store = Arc::new(OnceLock::new());
    let services = build_lifecycle_test_services_with_auth_provider(
        "hosted-mcp-oauth-rejected-token",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
        Arc::new(FixtureOAuthProvider {
            secret_store: Arc::clone(&fixture_secret_store),
            access_token: "rejected-token".to_string(),
        }),
    )
    .await;
    assert!(fixture_secret_store.set(services.secret_store()).is_ok());
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-oauth-rejected-token");
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("OAuth registration persists the tenant definition");
    let pending = install_fixture(&services, scope.clone()).await;
    let provider = credential_provider_from_response(&pending);

    let callback = complete_fixture_oauth_callback(&services, &scope, provider)
        .await
        .expect("the OAuth provider callback itself completed");
    assert_eq!(callback.status, ironclaw_auth::AuthFlowStatus::Completed);
    assert!(
        services
            .extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capability projection")
            .is_empty(),
        "a token the MCP rejects must not publish callable tools",
    );
    assert!(server.requests().iter().any(|request| {
        request.rpc_method.as_deref() == Some("initialize")
            && request.authorization_present
            && !request.authorization_matches
    }));

    let installation_store = services.extension_management.installation_store_for_test();
    let installation = installation_store
        .list_installations()
        .await
        .expect("installation-store readback")
        .into_iter()
        .find(|installation| installation.extension_id().as_str() == "mcp-fixture")
        .expect("fixture installation remains durable after rejected token");
    let manifest = installation_store
        .get_manifest(installation.extension_id())
        .await
        .expect("installed manifest readback")
        .expect("fixture manifest remains durable after rejected token");
    assert!(
        !manifest.resolved().has_model_visible_capabilities(),
        "the user can retry setup after an MCP-side credential rejection",
    );
    let listed = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("installation list");
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = listed.payload else {
        panic!("list returns installed extension summaries")
    };
    assert!(extensions.iter().any(|extension| {
        extension.summary.package_ref == fixture_package_ref()
            && extension.phase == ironclaw_extension_contracts::state::InstallationState::Installed
    }));
}

#[tokio::test]
async fn oauth_empty_catalog_after_callback_retains_account_and_stays_installed() {
    // A provider can accept OAuth yet temporarily return no usable catalog.
    // OAuth remains complete and its credential remains available, while the
    // extension stays installed for a later catalog retry.
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuth {
            access_token: "oauth-token".to_string(),
        },
        Vec::new(),
    )
    .await;
    let fixture_secret_store = Arc::new(OnceLock::new());
    let services = build_lifecycle_test_services_with_auth_provider(
        "hosted-mcp-oauth-empty-catalog",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
        Arc::new(FixtureOAuthProvider {
            secret_store: Arc::clone(&fixture_secret_store),
            access_token: "oauth-token".to_string(),
        }),
    )
    .await;
    assert!(fixture_secret_store.set(services.secret_store()).is_ok());
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-oauth-empty-catalog");
    let registration = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("OAuth registration persists the tenant definition");
    assert!(
        registration.blockers.is_empty(),
        "registration does not probe auth"
    );
    let install = install_fixture(&services, scope.clone()).await;
    let provider = credential_provider_from_response(&install);

    let callback = complete_fixture_oauth_callback(&services, &scope, provider.clone())
        .await
        .expect("OAuth completion is not relabeled as a catalog authorization failure");
    assert_eq!(callback.status, ironclaw_auth::AuthFlowStatus::Completed);

    let requester = ExtensionId::new("mcp-fixture").expect("extension id");
    let provider_vendor = ironclaw_host_api::ids::VendorId::new(provider.as_str())
        .expect("metadata-derived provider remains a vendor id");
    let account_request = ironclaw_auth::runtime_credential_account_selection_request(
        &scope,
        &provider_vendor,
        ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
        &[],
        &requester,
    )
    .expect("runtime credential request");
    let account = services
        .product_auth
        .runtime_credential_account_selection_service()
        .select_unique_configured_runtime_account(account_request)
        .await
        .expect("the completed OAuth account stays configured for retry");
    let access_secret = account
        .access_secret
        .as_ref()
        .expect("configured account retains the access-secret handle");
    assert!(
        services
            .secret_store()
            .metadata(&account.scope.resource, access_secret)
            .await
            .expect("OAuth access-secret metadata read")
            .is_some(),
        "a discovery failure must not purge a successfully exchanged credential"
    );
    assert!(server.requests().iter().any(|request| {
        request.rpc_method.as_deref() == Some("tools/list") && request.authorization_matches
    }));
    assert!(
        services
            .extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capability projection")
            .is_empty(),
        "an empty post-auth catalog leaves no callable tools"
    );
    let installation_store = services.extension_management.installation_store_for_test();
    let raw_installations = installation_store
        .list_installations()
        .await
        .expect("raw installation-store readback");
    let raw_installation = raw_installations
        .iter()
        .find(|installation| installation.extension_id().as_str() == "mcp-fixture")
        .expect("fixture installation remains durable after OAuth completion");
    let installed_manifest = installation_store
        .get_manifest(raw_installation.extension_id())
        .await
        .expect("installed manifest readback")
        .expect("fixture manifest remains durable after OAuth completion");
    assert!(
        !installed_manifest
            .resolved()
            .has_model_visible_capabilities(),
        "an empty catalog must leave the package publishing nothing: {installed_manifest:#?}"
    );
    let listed = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("installation list");
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = listed.payload else {
        panic!("list returns installed extension summaries")
    };
    assert!(
        extensions.iter().any(|extension| {
            extension.summary.package_ref == fixture_package_ref()
                && extension.phase
                    == ironclaw_extension_contracts::state::InstallationState::Installed
        }),
        "empty catalog remains installed for a later lifecycle retry: {extensions:#?}"
    );
}

/// Manual smoke for the official unauthenticated Microsoft Release
/// Communications endpoint. It stays opt-in because the vendor-owned catalog
/// and public-network availability are intentionally outside the hermetic PR
/// contract. It does, however, cross the same policy-mediated egress and
/// lifecycle paths as the deterministic fixture journeys.
#[tokio::test]
#[ignore = "live public MCP smoke; set IRONCLAW_LIVE_MCP_TESTS=1 and run this exact test"]
async fn live_notion_oauth_registration_reaches_generic_setup() {
    assert_eq!(
        std::env::var("IRONCLAW_LIVE_MCP_TESTS").ok().as_deref(),
        Some("1"),
        "refusing public-network smoke: set IRONCLAW_LIVE_MCP_TESTS=1"
    );
    let services =
        build_lifecycle_test_services("hosted-mcp-live-notion", Some(live_network_egress()), false)
            .await;
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-live-notion");
    let registration = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: RegisterHostedMcpRequest {
                    desired_id: LifecyclePackageId::new("notion").expect("static package id"),
                    desired_name: "Notion".to_string(),
                    endpoint: HostedMcpEndpoint::new("https://mcp.notion.com/mcp")
                        .expect("official public endpoint"),
                    auth_selection: Some(HostedMcpAuthSelection::Auto),
                },
            },
        )
        .await
        .expect("Notion OAuth registration persists its definition");
    assert_eq!(
        registration.phase,
        ironclaw_extension_contracts::state::InstallationState::Installed
    );
    assert!(registration.blockers.is_empty());
    let install = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionInstall {
                package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion")
                    .expect("static package ref"),
            },
        )
        .await
        .expect("Notion install should reach generic credential setup");
    assert!(
        install.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
        )),
        "Notion install must hand off to generic setup: {install:#?}"
    );
}

#[tokio::test]
#[ignore = "live public MCP smoke; set IRONCLAW_LIVE_MCP_TESTS=1 and run this exact test"]
async fn live_microsoft_mrc_registers_discovers_and_invokes_a_read_only_tool() {
    assert_eq!(
        std::env::var("IRONCLAW_LIVE_MCP_TESTS").ok().as_deref(),
        Some("1"),
        "refusing public-network smoke: set IRONCLAW_LIVE_MCP_TESTS=1"
    );
    let services = build_lifecycle_test_services(
        "hosted-mcp-live-microsoft",
        Some(live_network_egress()),
        false,
    )
    .await;
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-live-microsoft");
    let registration = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: RegisterHostedMcpRequest {
                    desired_id: LifecyclePackageId::new("microsoft-mrc")
                        .expect("static package id"),
                    desired_name: "Microsoft Release Communications".to_string(),
                    endpoint: HostedMcpEndpoint::new(
                        "https://www.microsoft.com/releasecommunications/mcp",
                    )
                    .expect("official public endpoint"),
                    auth_selection: Some(HostedMcpAuthSelection::NoAuth),
                },
            },
        )
        .await
        .expect("public no-auth server registers through the standard lifecycle");
    assert_eq!(
        registration.phase,
        ironclaw_extension_contracts::state::InstallationState::Installed
    );
    let install = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionInstall {
                package_ref: LifecyclePackageRef::new(
                    LifecyclePackageKind::Extension,
                    "microsoft-mrc",
                )
                .expect("static package ref"),
            },
        )
        .await
        .expect("public no-auth server installs through the standard lifecycle");
    assert_eq!(
        install.phase,
        ironclaw_extension_contracts::state::InstallationState::Active
    );

    let capabilities = services
        .extension_management
        .active_model_visible_capabilities()
        .await
        .expect("active MRC catalog");
    let names = capabilities
        .iter()
        .filter_map(|capability| capability.id.as_str().rsplit('.').next())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        names,
        std::collections::BTreeSet::from([
            "get_recent_azure_updates",
            "get_azure_update_by_id",
            "get_recent_m365_roadmaps",
            "get_m365_roadmap_by_id",
        ]),
        "vendor catalog changed; update this explicit live contract deliberately"
    );
    let capability = capabilities
        .iter()
        .find(|capability| {
            capability
                .id
                .as_str()
                .ends_with(".get_recent_azure_updates")
        })
        .expect("documented read-only Azure list tool");
    let outcome = invoke_with_standalone_approval(
        &services,
        capability.id.as_str(),
        runtime_context(scope, capability.id.as_str()),
        json!({"skip": 0}),
    )
    .await;
    assert!(
        matches!(
            outcome,
            ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(_)
        ),
        "documented read-only Azure list tool completes: {outcome:?}"
    );
}

// The `HostedMcpPreparationService::register` / `ExtensionLifecycleManager::
// import_bundle` lock-order regression test used to live here, driving both
// production entry points concurrently through `lifecycle_service` with
// `tokio::join!`. That version could not reliably force the two tasks to
// interleave on the shared catalog/operation locks (it passed in ~0.07s even
// with the lock-order fix reverted), so it was a non-discriminating
// regression test. The deterministic replacement — which pauses `register`
// mid-flight via a controllable installation-store double to force the
// actual AB-BA contention — lives at the crate tier:
// `crates/ironclaw_extension_host/src/product_lifecycle.rs`'s
// `concurrent_register_and_import_bundle_do_not_deadlock` test, which can
// construct `ExtensionLifecycleManager` directly with that double.

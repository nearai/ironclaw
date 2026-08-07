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
use ironclaw_assistant::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction, LifecycleProductPayload,
};
use ironclaw_auth::{
    AdmissionClientProfile, AuthContinuationRef, AuthProductError, AuthProductScope,
    AuthProviderClient, AuthProviderId, AuthSurface, AuthorizationCodeHash, CredentialAccountLabel,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthClientProfileRegistry,
    OAuthProviderCallbackRequest, OAuthProviderExchange, OAuthProviderExchangeContext,
    OAuthProviderRefresh, OAuthProviderRefreshRequest, OpaqueStateHash, PkceVerifierHash,
    PkceVerifierSecret, ProviderScope, RebornManualTokenSetupRequest,
    RebornManualTokenSubmitRequest, RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest,
    RebornOAuthStartFlowRequest,
};
use ironclaw_extension_contracts::hosted_mcp::{
    HostedMcpAuthSelection, HostedMcpEndpoint, RegisterHostedMcpRequest,
};
use ironclaw_extension_contracts::lifecycle_id::LifecyclePackageId;
use ironclaw_extension_contracts::recipe::{
    HttpsEndpoint, RecipeClientCredentials, VendorAuthRecipe,
};
use ironclaw_extension_manager::lifecycle_test_support::{
    build_lifecycle_test_services, build_lifecycle_test_services_with_auth_provider,
    build_lifecycle_test_services_with_oauth_client_profiles, invoke_with_standalone_approval,
    lifecycle_product_context, rebuild_lifecycle_test_services_with_auth_provider,
    webui_gate_resource_scope_for_owner,
};
use ironclaw_extension_registry::{
    ExtensionInstallationStorePort, ExtensionManifestRecord, ManifestSource, PackageRootBinding,
    RegisteredPackageDefinition,
};
use ironclaw_host_api::{
    action::{NetworkPolicy, NetworkScheme, NetworkTargetPattern},
    capability::{CapabilityGrant, CapabilitySet, EffectKind, GrantConstraints},
    ids::{CapabilityGrantId, CapabilityId, ExtensionId, SecretHandle},
    mount::MountView,
    runtime::{RuntimeKind, TrustClass},
    scope::Principal,
};
use ironclaw_product_contracts::lifecycle_service::LifecycleProductService;
use ironclaw_product_contracts::surface::{ProductSurfaceErrorCode, ProductSurfaceErrorKind};
use ironclaw_secrets::SecretStorePort;
use secrecy::SecretString;
use serde_json::json;
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

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
    registration_request_with_id("fixture", auth_selection)
}

fn registration_request_with_id(
    desired_id: &str,
    auth_selection: HostedMcpAuthSelection,
) -> RegisterHostedMcpRequest {
    RegisterHostedMcpRequest {
        desired_id: LifecyclePackageId::new(desired_id).expect("package id"),
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

#[derive(Debug)]
struct FixtureOAuthClientProfileRegistry(AdmissionClientProfile);

#[async_trait]
impl OAuthClientProfileRegistry for FixtureOAuthClientProfileRegistry {
    async fn resolve(&self, profile_id: &str) -> Option<AdmissionClientProfile> {
        (self.0.id == profile_id).then(|| self.0.clone())
    }
}

fn fixture_oauth_client_profile() -> AdmissionClientProfile {
    AdmissionClientProfile {
        id: "fixture-profile".to_string(),
        resource: HttpsEndpoint::new("https://mcp.example.test/mcp".to_string())
            .expect("fixture resource endpoint"),
        issuer: HttpsEndpoint::new("https://auth.example.test".to_string())
            .expect("fixture authorization-server issuer"),
        credentials: RecipeClientCredentials {
            client_id_handle: SecretHandle::new("fixture-oauth-client-id")
                .expect("fixture client-id handle"),
            client_secret_handle: Some(
                SecretHandle::new("fixture-oauth-client-secret")
                    .expect("fixture client-secret handle"),
            ),
        },
    }
}

async fn install_fixture(
    services: &ironclaw_extension_manager::lifecycle_test_support::ExtensionLifecycleTestServices,
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

/// Install a normal no-auth empty-catalog package, then replace only its
/// persisted manifest to emulate an installation written before registration
/// resolved hosted-MCP authentication.
async fn install_legacy_hosted_mcp_manifest(
    services: &ironclaw_extension_manager::lifecycle_test_support::ExtensionLifecycleTestServices,
    scope: ironclaw_host_api::resource::ResourceScope,
    registration_auth: HostedMcpAuthSelection,
) {
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("production registration admits the no-auth legacy fixture");
    install_fixture(services, scope).await;

    let extension_id = ExtensionId::new("mcp-fixture").expect("fixture extension id");
    let installation_store = services.extension_management.installation_store_for_test();
    let installation = installation_store
        .list_installations()
        .await
        .expect("fixture installation list")
        .into_iter()
        .find(|installation| installation.extension_id() == &extension_id)
        .expect("fixture installation persists");
    let manifest = installation_store
        .get_manifest(&extension_id)
        .await
        .expect("fixture manifest readback")
        .expect("fixture installation has a manifest");
    let mut resolved = manifest.resolved().clone();
    resolved
        .mcp
        .as_mut()
        .expect("hosted fixture has an MCP declaration")
        .registration_auth = registration_auth;
    let legacy_manifest = ExtensionManifestRecord::from_resolved(
        manifest.raw_toml(),
        manifest.manifest().source,
        resolved,
        manifest.manifest_hash().cloned(),
    )
    .expect("legacy hosted-MCP manifest remains valid")
    .with_definition_retention(manifest.definition_retention());
    installation_store
        .upsert_manifest_only(
            installation.installation_id(),
            installation.incarnation_id(),
            installation.manifest_ref(),
            installation.updated_at(),
            legacy_manifest,
        )
        .await
        .expect("public store port replaces only the persisted legacy manifest");
}

async fn restored_legacy_hosted_mcp(
    user_id: &str,
    registration_auth: HostedMcpAuthSelection,
    recovery_egress: Arc<dyn ironclaw_network::NetworkHttpEgress>,
) -> (
    ironclaw_extension_manager::lifecycle_test_support::ExtensionLifecycleTestServices,
    ironclaw_host_api::resource::ResourceScope,
) {
    let bootstrap =
        HostedMcpRegistrationServer::start(HostedMcpAuthPolicy::NoAuth, Vec::new()).await;
    let services = build_lifecycle_test_services(
        user_id,
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &bootstrap,
        ))),
        false,
    )
    .await;
    let scope = webui_gate_resource_scope_for_owner(user_id);
    install_legacy_hosted_mcp_manifest(&services, scope.clone(), registration_auth).await;
    let restored = rebuild_lifecycle_test_services_with_auth_provider(
        &services,
        user_id,
        Some(recovery_egress),
        false,
        Arc::new(ironclaw_auth::UnavailableAuthProviderClient),
    )
    .await;
    (restored, scope)
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
    services: &ironclaw_extension_manager::lifecycle_test_support::ExtensionLifecycleTestServices,
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
    services: &ironclaw_extension_manager::lifecycle_test_support::ExtensionLifecycleTestServices,
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
            requested_scopes: Vec::new(),
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

/// A registration preflight performs real network I/O before the lifecycle
/// mutation lock. At capacity, the next caller must receive the existing
/// retryable product failure without joining an unbounded queue or reaching
/// the MCP server; releasing the first batch must make a later registration
/// succeed.
#[tokio::test]
async fn hosted_mcp_registration_preflight_fails_fast_at_capacity_and_releases_permits() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::NoAuth,
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let gate = server.block_mcp_preflight_requests();
    let services = build_lifecycle_test_services(
        "hosted-mcp-registration-admission",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
    )
    .await;
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-registration-admission");
    let mut registrations = Vec::new();
    for index in 0..8 {
        let lifecycle_service = Arc::clone(&services.lifecycle_service);
        let context = lifecycle_product_context(scope.clone());
        let request = registration_request_with_id(
            &format!("preflight-{index}"),
            HostedMcpAuthSelection::Auto,
        );
        registrations.push(tokio::spawn(async move {
            lifecycle_service
                .execute(
                    context,
                    LifecycleProductAction::ExtensionRegisterHostedMcp { request },
                )
                .await
        }));
    }
    tokio::time::timeout(Duration::from_secs(5), gate.wait_for_entries(8))
        .await
        .expect("eight preflights reach the real MCP request before saturation");

    let saturated = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: registration_request_with_id(
                    "preflight-saturated",
                    HostedMcpAuthSelection::Auto,
                ),
            },
        )
        .await
        .expect_err("a ninth registration must fail fast instead of waiting for a preflight slot");
    assert_eq!(saturated.code, ProductSurfaceErrorCode::Unavailable);
    assert_eq!(saturated.kind, ProductSurfaceErrorKind::ServiceUnavailable);
    assert_eq!(saturated.status_code, 503);
    assert!(saturated.retryable, "saturation must remain retryable");
    assert_eq!(
        server.requests().len(),
        8,
        "the saturated registration must not start its own network preflight"
    );

    gate.release();
    for registration in registrations {
        registration
            .await
            .expect("preflight task does not panic")
            .expect("a released preflight completes registration");
    }
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: registration_request_with_id(
                    "preflight-after-release",
                    HostedMcpAuthSelection::Auto,
                ),
            },
        )
        .await
        .expect("a released preflight permit admits the next registration");
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
        "the shared lifecycle response vocabulary uses a neutral installed phase for the definition-only registration response: {registration:#?}",
    );
    assert!(
        server
            .requests()
            .iter()
            .any(|request| request.rpc_method.as_deref() == Some("initialize")),
        "automatic registration proves that the server accepts unauthenticated MCP requests"
    );
    assert!(
        !server
            .requests()
            .iter()
            .any(|request| request.rpc_method.as_deref() == Some("tools/list")),
        "registration probes auth only; catalog discovery remains an installation step"
    );
    let registered_definition = services
        .extension_management
        .installation_store_for_test()
        .get_registered_package_definition(&ExtensionId::new("mcp-fixture").expect("extension id"))
        .await
        .expect("registered definition readback")
        .expect("registration persists the resolved definition with its membership");
    assert!(matches!(
        registered_definition
            .definition()
            .resolved()
            .mcp
            .as_ref()
            .map(|mcp| &mcp.registration_auth),
        Some(HostedMcpAuthSelection::NoAuth)
    ));
    let caller =
        ironclaw_host_api::ids::UserId::new("hosted-mcp-user").expect("registering user id");
    assert_eq!(
        registered_definition.audience().manager_user_ids(),
        Some(&std::collections::BTreeSet::from([caller.clone()])),
    );
    assert_eq!(
        registered_definition.audience().member_user_ids(),
        Some(&std::collections::BTreeSet::from([caller])),
    );
    assert!(
        services
            .extension_management
            .installation_store_for_test()
            .list_installations()
            .await
            .expect("installation readback")
            .is_empty(),
        "registration must not create an installation",
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
    let requests_before_retry = server.requests().len();
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
    assert_eq!(
        server.requests().len(),
        requests_before_retry,
        "an exact durable retry must not depend on the remote server still being available"
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
async fn automatic_no_auth_empty_catalog_registers_before_catalog_discovery() {
    let server = HostedMcpRegistrationServer::start(HostedMcpAuthPolicy::NoAuth, Vec::new()).await;
    let services = build_lifecycle_test_services(
        "hosted-mcp-no-auth-empty-catalog",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
    )
    .await;
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-no-auth-empty-catalog");
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("initialize success is enough to register a no-auth MCP");

    let installed = install_fixture(&services, scope.clone()).await;
    assert_eq!(
        installed.phase,
        ironclaw_extension_contracts::state::InstallationState::Installed,
        "an empty catalog has no activatable surface"
    );
    let projected = services
        .lifecycle_service
        .project_package(lifecycle_product_context(scope), fixture_package_ref())
        .await
        .expect("the finalized no-auth package reprojects");
    assert!(
        !projected.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Setup {
                ref_id: Some(ref_id),
            } if ref_id.as_str()
                == ironclaw_product_contracts::package_lifecycle::HOSTED_MCP_AUTH_SELECTION_BLOCKER_REF
        )),
        "a successful no-auth discovery must not re-open auth selection: {projected:#?}"
    );

    let requests_before_explicit_no_auth = server.requests().len();
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(webui_gate_resource_scope_for_owner(
                "hosted-mcp-no-auth-empty-catalog",
            )),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: registration_request_with_id(
                    "explicit-no-auth",
                    HostedMcpAuthSelection::NoAuth,
                ),
            },
        )
        .await
        .expect("an explicit no-auth choice registers without auth preflight");
    assert_eq!(
        server.requests().len(),
        requests_before_explicit_no_auth,
        "explicit no-auth registration must not probe the MCP before persistence"
    );

    let oauth_error = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(webui_gate_resource_scope_for_owner(
                "hosted-mcp-no-auth-empty-catalog",
            )),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: registration_request_with_id(
                    "oauth-against-no-auth",
                    HostedMcpAuthSelection::OAuth {
                        client_profile_id: None,
                    },
                ),
            },
        )
        .await
        .expect_err("explicit OAuth must reject a server that accepts unauthenticated access");
    assert_eq!(oauth_error.code, ProductSurfaceErrorCode::InvalidRequest);
    assert_eq!(oauth_error.kind, ProductSurfaceErrorKind::Validation);
    assert_eq!(oauth_error.status_code, 400);
}

#[tokio::test]
async fn legacy_auto_manifest_selects_bearer_through_lifecycle_and_checkpoints_setup() {
    let bearer = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::ExactBearerWithoutChallenge {
            token: "fixture-token".to_string(),
        },
        Vec::new(),
    )
    .await;
    let (restored, scope) = restored_legacy_hosted_mcp(
        "hosted-mcp-legacy-select-bearer",
        HostedMcpAuthSelection::Auto,
        Arc::new(HostedMcpRegistrationNetworkEgress::for_server(&bearer)),
    )
    .await;

    let automatic_error = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionSelectHostedMcpAuth {
                package_ref: fixture_package_ref(),
                auth_selection: HostedMcpAuthSelection::Auto,
            },
        )
        .await
        .expect_err("auth recovery requires an explicit user selection");
    assert_eq!(
        automatic_error.code,
        ProductSurfaceErrorCode::InvalidRequest
    );
    assert_eq!(automatic_error.kind, ProductSurfaceErrorKind::Validation);

    let selected = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionSelectHostedMcpAuth {
                package_ref: fixture_package_ref(),
                auth_selection: HostedMcpAuthSelection::Bearer,
            },
        )
        .await
        .expect("lifecycle auth selection checkpoints bearer setup");
    assert!(selected.blockers.iter().any(|blocker| matches!(
        blocker,
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
    )));
    let manifest = restored
        .extension_management
        .installation_store_for_test()
        .get_manifest(&ExtensionId::new("mcp-fixture").expect("fixture extension id"))
        .await
        .expect("checkpointed manifest readback")
        .expect("fixture manifest persists");
    assert!(matches!(
        manifest
            .resolved()
            .mcp
            .as_ref()
            .map(|mcp| &mcp.registration_auth),
        Some(HostedMcpAuthSelection::Bearer)
    ));

    let reselection_error = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionSelectHostedMcpAuth {
                package_ref: fixture_package_ref(),
                auth_selection: HostedMcpAuthSelection::OAuth {
                    client_profile_id: None,
                },
            },
        )
        .await
        .expect_err("checkpointed bearer setup cannot be overwritten by another auth selection");
    assert_eq!(
        reselection_error.code,
        ProductSurfaceErrorCode::InvalidRequest
    );
    assert_eq!(reselection_error.kind, ProductSurfaceErrorKind::Validation);
}

#[tokio::test]
async fn legacy_auto_manifest_challenged_by_oauth_checkpoints_oauth_requirements() {
    let oauth = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuth {
            access_token: "oauth-token".to_string(),
        },
        Vec::new(),
    )
    .await;
    let (restored, scope) = restored_legacy_hosted_mcp(
        "hosted-mcp-legacy-auto-oauth",
        HostedMcpAuthSelection::Auto,
        Arc::new(HostedMcpRegistrationNetworkEgress::for_server(&oauth)),
    )
    .await;

    let activated = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect("legacy automatic auth prepares OAuth through lifecycle activation");
    assert!(activated.blockers.iter().any(|blocker| matches!(
        blocker,
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
    )));
    let manifest = restored
        .extension_management
        .installation_store_for_test()
        .get_manifest(&ExtensionId::new("mcp-fixture").expect("fixture extension id"))
        .await
        .expect("checkpointed manifest readback")
        .expect("fixture manifest persists");
    assert!(matches!(
        manifest
            .resolved()
            .mcp
            .as_ref()
            .map(|mcp| &mcp.registration_auth),
        Some(HostedMcpAuthSelection::Auto)
    ));
    assert!(manifest.resolved().auth.iter().any(|auth| matches!(
        &auth.setup,
        ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth { .. }
    )));
}

#[tokio::test]
async fn legacy_auto_manifest_without_oauth_client_path_requires_explicit_selection() {
    let oauth = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuthWithoutChallenge {
            access_token: "oauth-token".to_string(),
        },
        Vec::new(),
    )
    .await;
    oauth.script_authorization_server_response(ScriptedMetadataResponse::new(
        axum::http::StatusCode::OK,
        serde_json::to_vec(&json!({
            "issuer": "https://auth.example.test",
            "authorization_endpoint": "https://auth.example.test/authorize",
            "token_endpoint": "https://auth.example.test/token",
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"]
        }))
        .expect("authorization metadata without DCR serializes"),
    ));
    let (restored, scope) = restored_legacy_hosted_mcp(
        "hosted-mcp-legacy-auto-without-client-path",
        HostedMcpAuthSelection::Auto,
        Arc::new(HostedMcpRegistrationNetworkEgress::for_server(&oauth)),
    )
    .await;

    let activated = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect("missing OAuth client setup returns an explicit auth choice");
    assert!(activated.blockers.iter().any(|blocker| matches!(
        blocker,
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Setup {
            ref_id: Some(ref_id),
        } if ref_id.as_str()
            == ironclaw_product_contracts::package_lifecycle::HOSTED_MCP_AUTH_SELECTION_BLOCKER_REF
    )));
    assert!(
        !activated.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential {
                ..
            }
        )),
        "unresolved automatic auth must request a choice before credential setup"
    );

    let projected = restored
        .lifecycle_service
        .project_package(lifecycle_product_context(scope), fixture_package_ref())
        .await
        .expect("the unresolved auth choice reprojects");
    assert!(projected.blockers.iter().any(|blocker| matches!(
        blocker,
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Setup {
            ref_id: Some(ref_id),
        } if ref_id.as_str()
            == ironclaw_product_contracts::package_lifecycle::HOSTED_MCP_AUTH_SELECTION_BLOCKER_REF
    )));
}

#[tokio::test]
async fn legacy_explicit_oauth_with_unusable_metadata_resets_to_auth_selection() {
    let oauth = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuthWithoutChallenge {
            access_token: "oauth-token".to_string(),
        },
        Vec::new(),
    )
    .await;
    oauth.script_protected_resource_response(ScriptedMetadataResponse::new(
        axum::http::StatusCode::NOT_FOUND,
        Vec::new(),
    ));
    let (restored, scope) = restored_legacy_hosted_mcp(
        "hosted-mcp-legacy-oauth-unusable",
        HostedMcpAuthSelection::OAuth {
            client_profile_id: None,
        },
        Arc::new(HostedMcpRegistrationNetworkEgress::for_server(&oauth)),
    )
    .await;

    let activated = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect("unusable OAuth metadata returns the setup checkpoint");
    assert!(activated.blockers.iter().any(|blocker| matches!(
        blocker,
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Setup {
            ref_id: Some(ref_id),
        } if ref_id.as_str()
            == ironclaw_product_contracts::package_lifecycle::HOSTED_MCP_AUTH_SELECTION_BLOCKER_REF
    )));
    let manifest = restored
        .extension_management
        .installation_store_for_test()
        .get_manifest(&ExtensionId::new("mcp-fixture").expect("fixture extension id"))
        .await
        .expect("reset manifest readback")
        .expect("fixture manifest persists");
    assert!(matches!(
        manifest
            .resolved()
            .mcp
            .as_ref()
            .map(|mcp| &mcp.registration_auth),
        Some(HostedMcpAuthSelection::Auto)
    ));
    assert!(manifest.resolved().auth.is_empty());
}

#[tokio::test]
async fn legacy_no_auth_manifest_rejected_by_server_resets_to_auth_selection() {
    let bearer = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::ExactBearerWithoutChallenge {
            token: "fixture-token".to_string(),
        },
        Vec::new(),
    )
    .await;
    let (restored, scope) = restored_legacy_hosted_mcp(
        "hosted-mcp-legacy-no-auth-rejected",
        HostedMcpAuthSelection::NoAuth,
        Arc::new(HostedMcpRegistrationNetworkEgress::for_server(&bearer)),
    )
    .await;

    let activated = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect("rejected no-auth manifest returns the setup checkpoint");
    assert!(activated.blockers.iter().any(|blocker| matches!(
        blocker,
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Setup {
            ref_id: Some(ref_id),
        } if ref_id.as_str()
            == ironclaw_product_contracts::package_lifecycle::HOSTED_MCP_AUTH_SELECTION_BLOCKER_REF
    )));
    let manifest = restored
        .extension_management
        .installation_store_for_test()
        .get_manifest(&ExtensionId::new("mcp-fixture").expect("fixture extension id"))
        .await
        .expect("reset manifest readback")
        .expect("fixture manifest persists");
    assert!(matches!(
        manifest
            .resolved()
            .mcp
            .as_ref()
            .map(|mcp| &mcp.registration_auth),
        Some(HostedMcpAuthSelection::Auto)
    ));
    assert!(manifest.resolved().auth.is_empty());
}

#[tokio::test]
async fn privately_registered_definition_survives_restart_and_installs_without_reregistration() {
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
    let creator_scope = webui_gate_resource_scope_for_owner("hosted-mcp-creator");
    let operator_scope = webui_gate_resource_scope_for_owner("hosted-mcp-registered-only");
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(creator_scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("registration persists a private definition");

    assert!(
        services
            .extension_management
            .installation_store_for_test()
            .list_installations()
            .await
            .expect("installation-store readback")
            .is_empty(),
        "registration must remain definition-only",
    );

    // Deliberately never install. Rebuild services over the same durable
    // backing and confirm the private definition restores without publishing
    // tools or synthesizing an installation.
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
            lifecycle_product_context(creator_scope.clone()),
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
        "a privately registered hosted MCP must survive restart for its creator: {extensions:#?}",
    );

    let operator_search = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(operator_scope.clone()),
            LifecycleProductAction::ExtensionSearch {
                query: "Fixture MCP".to_string(),
            },
        )
        .await
        .expect("operator searches after another user's registration is restored");
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = operator_search.payload
    else {
        panic!("search returns extension summaries")
    };
    assert!(
        extensions
            .iter()
            .all(|extension| extension.summary.package_ref != fixture_package_ref()),
        "restart must not widen a restored custom MCP to the tenant operator: {extensions:#?}",
    );

    let operator_list = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(operator_scope.clone()),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("operator lists extensions after another user's registration is restored");
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = operator_list.payload
    else {
        panic!("list returns installed extension summaries")
    };
    assert!(
        extensions
            .iter()
            .all(|extension| extension.summary.package_ref != fixture_package_ref()),
        "restart must preserve installed-list isolation: {extensions:#?}",
    );

    let denial = restored
        .lifecycle_service
        .execute(
            lifecycle_product_context(operator_scope),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect_err("operator cannot join a restored custom MCP by guessing its package id");
    assert_eq!(denial.code, ProductSurfaceErrorCode::InvalidRequest);

    assert!(
        restored
            .extension_management
            .installation_store_for_test()
            .list_installations()
            .await
            .expect("restored installation-store readback")
            .is_empty(),
        "restore must not synthesize an installation for a registered definition",
    );

    let registered = restored
        .extension_management
        .installation_store_for_test()
        .get_registered_package_definition(
            &ExtensionId::new("mcp-fixture").expect("fixture extension id"),
        )
        .await
        .expect("restored definition readback")
        .expect("restored registered definition");
    let creator =
        ironclaw_host_api::ids::UserId::new("hosted-mcp-creator").expect("creator user id");
    assert_eq!(
        registered.audience().manager_user_ids(),
        Some(&std::collections::BTreeSet::from([creator.clone()])),
        "restart must preserve exactly the registering manager",
    );
    assert_eq!(
        registered.audience().member_user_ids(),
        Some(&std::collections::BTreeSet::from([creator])),
        "restart must preserve exactly the registering visible member",
    );

    assert!(
        restored
            .extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capability projection")
            .is_empty(),
        "a registered definition must not publish tools after restart",
    );

    let installed = install_fixture(&restored, creator_scope).await;
    assert_eq!(
        installed.phase,
        ironclaw_extension_contracts::state::InstallationState::Active,
        "the restored private definition installs without re-registration: {installed:#?}",
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
async fn registration_cannot_replace_a_non_user_registered_definition() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::NoAuth,
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let services = build_lifecycle_test_services(
        "hosted-mcp-definition-collision",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
    )
    .await;
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-definition-collision");
    let raw = r#"schema_version = "reborn.extension_manifest.v3"
id = "mcp-fixture"
name = "Bundled collision fixture"
version = "0.1.0"
description = "non-user-registered definition collision"
trust = "third_party"

[mcp]
server = "https://bundled.example.test/mcp"
namespace = "mcp-fixture"
max_tools = 32
default_permission = "ask"
effects = ["network"]
"#;
    let parsed = ExtensionManifestRecord::from_toml_with_root_binding(
        raw,
        ManifestSource::UserRegistered,
        &ironclaw_host_api::host_port::default_host_port_catalog().expect("host port catalog"),
        None,
        &ironclaw_extension_host::product_extension_host_api_contract_registry()
            .expect("host API contracts"),
        PackageRootBinding::Virtual,
    )
    .expect("collision fixture parses through the reserved user-registration boundary");
    // Simulate a legacy/corrupt durable definition whose source no longer
    // agrees with the reserved ID. The registration guard must mask and retain
    // it even though current manifest admission would reject creating it.
    let existing = ExtensionManifestRecord::from_resolved(
        parsed.raw_toml(),
        ManifestSource::HostBundled,
        parsed.resolved().clone(),
        parsed.manifest_hash().cloned(),
    )
    .expect("source-mismatched durable fixture remains structurally reconstructable");
    let installation_store = services.extension_management.installation_store_for_test();
    installation_store
        .admit_package_definition(RegisteredPackageDefinition::managed_by(
            existing,
            scope.user_id.clone(),
        ))
        .await
        .expect("fixture definition admission succeeds");

    let error = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect_err("registration must not replace a non-user-registered definition");
    assert_eq!(error.code, ProductSurfaceErrorCode::InvalidRequest);
    assert!(
        server.requests().is_empty(),
        "a masked definition collision must fail before registration egress"
    );

    let extension_id = ExtensionId::new("mcp-fixture").expect("fixture extension id");
    let retained = installation_store
        .get_registered_package_definition(&extension_id)
        .await
        .expect("registered-definition lookup")
        .expect("existing definition remains admitted");
    assert_eq!(
        retained.definition().manifest().source,
        ManifestSource::HostBundled
    );
    assert!(
        installation_store
            .list_installations()
            .await
            .expect("installation-store readback")
            .is_empty(),
        "a definition collision must not create an installation"
    );
}

#[tokio::test]
async fn user_registered_hosted_mcp_is_discoverable_only_by_the_registering_user() {
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
    let operator_scope = webui_gate_resource_scope_for_owner("tenant-admin");
    let creator_scope = webui_gate_resource_scope_for_owner("tenant-member");
    let registration = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(creator_scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("user A registers the definition");
    assert!(matches!(
        registration.payload,
        Some(LifecycleProductPayload::ExtensionInstall {
            installed: false,
            ..
        })
    ));

    let requests_before_foreign_retry = server.requests().len();
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(operator_scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect_err("tenant operator cannot claim another user's custom MCP registration");
    assert_eq!(
        server.requests().len(),
        requests_before_foreign_retry,
        "foreign re-registration is rejected from membership before endpoint egress"
    );

    let creator_search = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(creator_scope.clone()),
            LifecycleProductAction::ExtensionSearch {
                query: "Fixture MCP".to_string(),
            },
        )
        .await
        .expect("creator searches their registered definition");
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = creator_search.payload
    else {
        panic!("search returns extension summaries")
    };
    assert!(
        extensions
            .iter()
            .any(|extension| extension.summary.package_ref == fixture_package_ref()),
        "the registering user must retain visibility of their custom hosted MCP"
    );

    let operator_search = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(operator_scope.clone()),
            LifecycleProductAction::ExtensionSearch {
                query: "Fixture MCP".to_string(),
            },
        )
        .await
        .expect("tenant operator searches the shared tenant catalog");
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = operator_search.payload
    else {
        panic!("search returns extension summaries")
    };
    assert!(
        extensions
            .iter()
            .all(|extension| extension.summary.package_ref != fixture_package_ref()),
        "a custom hosted MCP registration must not grant visibility or membership to another user"
    );

    let guessed_projection = services
        .lifecycle_service
        .project_package(
            lifecycle_product_context(operator_scope.clone()),
            fixture_package_ref(),
        )
        .await
        .expect_err(
            "a user who guesses the package ref must not project another user's custom MCP",
        );
    assert_eq!(
        guessed_projection.code,
        ProductSurfaceErrorCode::InvalidRequest,
    );

    let creator_list = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(creator_scope.clone()),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("creator lists their installed extensions");
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = creator_list.payload
    else {
        panic!("list returns installed extension summaries")
    };
    assert!(
        extensions
            .iter()
            .all(|extension| extension.summary.package_ref != fixture_package_ref()),
        "registration alone must not place the custom MCP in the creator's installed list"
    );

    let operator_list = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(operator_scope.clone()),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("tenant operator lists installed extensions");
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = operator_list.payload
    else {
        panic!("list returns installed extension summaries")
    };
    assert!(
        extensions
            .iter()
            .all(|extension| extension.summary.package_ref != fixture_package_ref()),
        "tenant operators must not enumerate another user's custom MCP"
    );

    let guessed_install = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(operator_scope.clone()),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect_err("a user who guesses the package ref must not join another user's custom MCP");
    assert_eq!(
        guessed_install.code,
        ProductSurfaceErrorCode::InvalidRequest,
    );

    let installation_store = services.extension_management.installation_store_for_test();
    assert!(
        installation_store
            .list_installations()
            .await
            .expect("installation-store readback")
            .is_empty(),
        "registration must not create installation membership",
    );
    let registered = installation_store
        .get_registered_package_definition(
            &ExtensionId::new("mcp-fixture").expect("fixture extension id"),
        )
        .await
        .expect("registered-definition readback")
        .expect("registered definition");
    let creator =
        ironclaw_host_api::ids::UserId::new("tenant-member").expect("registering user id");
    assert_eq!(
        registered.audience().manager_user_ids(),
        Some(&std::collections::BTreeSet::from([creator.clone()])),
        "registration initializes exactly the caller as explicit manager",
    );
    assert_eq!(
        registered.audience().member_user_ids(),
        Some(&std::collections::BTreeSet::from([creator.clone()])),
        "registration initializes exactly the caller as visible member",
    );

    install_fixture(&services, creator_scope).await;
    let installation = installation_store
        .list_installations()
        .await
        .expect("installation-store readback")
        .into_iter()
        .find(|installation| installation.extension_id().as_str() == "mcp-fixture")
        .expect("explicit install creates the custom MCP installation membership");
    let members = installation
        .owner()
        .members()
        .expect("custom hosted MCP registration is user-scoped");
    assert_eq!(
        members,
        &std::collections::BTreeSet::from([
            ironclaw_host_api::ids::UserId::new("tenant-member").expect("registering user id")
        ]),
        "only the registering user belongs to the custom MCP installation"
    );

    let operator_search_after_install = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(operator_scope.clone()),
            LifecycleProductAction::ExtensionSearch {
                query: "Fixture MCP".to_string(),
            },
        )
        .await
        .expect("tenant operator searches after the creator installs the custom MCP");
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
        operator_search_after_install.payload
    else {
        panic!("search returns extension summaries")
    };
    assert!(
        extensions
            .iter()
            .all(|extension| extension.summary.package_ref != fixture_package_ref()),
        "installing a custom MCP must not widen its catalog visibility to another user"
    );

    let guessed_projection_after_creator_install = services
        .lifecycle_service
        .project_package(
            lifecycle_product_context(operator_scope.clone()),
            fixture_package_ref(),
        )
        .await
        .expect_err(
            "a user who guesses the package ref must not project the creator's installed custom MCP",
        );
    assert_eq!(
        guessed_projection_after_creator_install.code,
        ProductSurfaceErrorCode::InvalidRequest,
    );

    let guessed_install_after_creator_install = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(operator_scope),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect_err(
            "a user who guesses the package ref must not join after the creator installs it",
        );
    assert_eq!(
        guessed_install_after_creator_install.code,
        ProductSurfaceErrorCode::InvalidRequest,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_custom_mcp_registration_leaves_exactly_one_managed_definition() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::NoAuth,
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let services = build_lifecycle_test_services(
        "tenant-operator",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
    )
    .await;
    let alice_scope = webui_gate_resource_scope_for_owner("alice");
    let bob_scope = webui_gate_resource_scope_for_owner("bob");

    let alice_registration = services.lifecycle_service.execute(
        lifecycle_product_context(alice_scope.clone()),
        LifecycleProductAction::ExtensionRegisterHostedMcp {
            request: automatic_request(),
        },
    );
    let bob_registration = services.lifecycle_service.execute(
        lifecycle_product_context(bob_scope.clone()),
        LifecycleProductAction::ExtensionRegisterHostedMcp {
            request: automatic_request(),
        },
    );
    let (alice_result, bob_result) = tokio::join!(alice_registration, bob_registration);

    assert_ne!(
        alice_result.is_ok(),
        bob_result.is_ok(),
        "exactly one concurrent caller must win registration: alice={alice_result:#?}, bob={bob_result:#?}",
    );
    let (winner, loser_scope, loser_error) = if alice_result.is_ok() {
        (
            "alice",
            bob_scope,
            bob_result.expect_err("bob must lose the conflicting registration"),
        )
    } else {
        (
            "bob",
            alice_scope,
            alice_result.expect_err("alice must lose the conflicting registration"),
        )
    };
    assert_eq!(loser_error.code, ProductSurfaceErrorCode::InvalidRequest);

    let installation_store = services.extension_management.installation_store_for_test();
    assert!(
        installation_store
            .list_installations()
            .await
            .expect("installation-store readback")
            .is_empty(),
        "neither concurrent registration may create an installation",
    );
    let registered = installation_store
        .get_registered_package_definition(
            &ExtensionId::new("mcp-fixture").expect("fixture extension id"),
        )
        .await
        .expect("registered-definition readback")
        .expect("winning definition");
    let winner = ironclaw_host_api::ids::UserId::new(winner).expect("winner user id");
    assert_eq!(
        registered.audience().manager_user_ids(),
        Some(&std::collections::BTreeSet::from([winner.clone()])),
        "the losing registration must not join the winner's manager set",
    );
    assert_eq!(
        registered.audience().member_user_ids(),
        Some(&std::collections::BTreeSet::from([winner])),
        "the losing registration must not join the winner's visibility membership",
    );

    let loser_search = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(loser_scope.clone()),
            LifecycleProductAction::ExtensionSearch {
                query: "Fixture MCP".to_string(),
            },
        )
        .await
        .expect("loser searches the shared tenant catalog");
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = loser_search.payload
    else {
        panic!("search returns extension summaries")
    };
    assert!(
        extensions
            .iter()
            .all(|extension| extension.summary.package_ref != fixture_package_ref()),
        "the losing caller must not discover the winner's custom MCP",
    );
    let guessed_install = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(loser_scope),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect_err("the losing caller cannot join by guessing the package id");
    assert_eq!(
        guessed_install.code,
        ProductSurfaceErrorCode::InvalidRequest,
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
        .expect("automatic registration returns an actionable auth choice");
    assert!(
        registration.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Setup {
                ref_id: Some(ref_id),
            } if ref_id.as_str()
                == ironclaw_product_contracts::package_lifecycle::HOSTED_MCP_AUTH_SELECTION_BLOCKER_REF
        )),
        "a bare 401 stops registration and requests an explicit auth type: {registration:#?}"
    );
    assert!(
        registration
            .message
            .as_deref()
            .is_some_and(|message| message.contains("choose OAuth or Bearer token")),
        "registration offers only auth methods still possible after a 401: {registration:#?}"
    );
    assert!(
        services
            .extension_management
            .installation_store_for_test()
            .get_registered_package_definition(
                &ExtensionId::new("mcp-fixture").expect("extension id")
            )
            .await
            .expect("definition readback")
            .is_none(),
        "an unresolved automatic registration must not persist a useless package"
    );
    assert!(
        server
            .requests()
            .iter()
            .any(|request| request.rpc_method.as_deref() == Some("initialize")),
        "automatic registration must probe the MCP before deciding"
    );

    let oauth_error = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: registration_request(HostedMcpAuthSelection::OAuth {
                    client_profile_id: None,
                }),
            },
        )
        .await
        .expect_err("an explicit OAuth retry still has to prove OAuth metadata");
    assert_eq!(oauth_error.kind, ProductSurfaceErrorKind::Validation);
    assert!(
        services
            .extension_management
            .installation_store_for_test()
            .get_registered_package_definition(
                &ExtensionId::new("mcp-fixture").expect("extension id")
            )
            .await
            .expect("definition readback")
            .is_none(),
        "a wrong OAuth choice must remain inside registration without persistence"
    );

    let explicit_registration = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: registration_request(HostedMcpAuthSelection::Bearer),
            },
        )
        .await
        .expect("explicit bearer retry persists the resolved definition");
    assert_eq!(
        explicit_registration.phase,
        ironclaw_extension_contracts::state::InstallationState::Installed
    );
    assert!(explicit_registration.blockers.is_empty());

    let setup_needed = install_fixture(&services, scope.clone()).await;
    assert!(setup_needed.blockers.iter().any(|blocker| matches!(
        blocker,
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
    )));
    assert!(!setup_needed.blockers.iter().any(|blocker| matches!(
        blocker,
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Setup {
            ref_id: Some(ref_id),
        } if ref_id.as_str()
            == ironclaw_product_contracts::package_lifecycle::HOSTED_MCP_AUTH_SELECTION_BLOCKER_REF
    )), "installation must not ask for the auth type after registration resolved it");

    let provider = credential_provider_from_response(&setup_needed);
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
        ironclaw_extension_contracts::state::InstallationState::Installed,
        "a rejected bearer token keeps the install in setup-needed state"
    );
    assert_eq!(
        rejected_retry.message.as_deref(),
        Some(
            "Hosted MCP rejected the saved credentials; update or reconnect them and retry activation."
        ),
        "a rejected bearer token must surface the hosted-MCP setup reason: {rejected_retry:#?}"
    );
    assert!(
        rejected_retry.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
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
        HostedMcpAuthPolicy::OAuthWithoutChallenge {
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
        "valid OAuth metadata resolves automatic registration"
    );
    assert!(
        server
            .requests()
            .iter()
            .any(|request| request.rpc_method.as_deref() == Some("initialize")),
        "automatic registration probes the MCP"
    );
    assert!(
        server
            .requests()
            .iter()
            .any(|request| request.method == "GET"),
        "registration proves OAuth from metadata before persisting"
    );
    let install = install_fixture(&services, scope.clone()).await;
    let provider = credential_provider_from_response(&install);
    assert!(
        install.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Credential { .. }
        )),
        "ordinary install uses the registration-time OAuth decision and returns the credential setup blocker: {install:#?}"
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
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "GET")
            .map(|request| request.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/.well-known/oauth-protected-resource/mcp",
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-authorization-server",
        ],
        "metadata-less OAuth must try path-specific RFC 9728 discovery, then root, then the admitted authorization server: {requests:#?}"
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

#[tokio::test]
async fn oauth_registration_accepts_path_metadata_without_root_fallback() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuthWithoutChallengePathMetadata {
            access_token: "oauth-token".to_string(),
        },
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let secret_store = Arc::new(OnceLock::new());
    let services = build_lifecycle_test_services_with_auth_provider(
        "hosted-mcp-oauth-path-metadata",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
        Arc::new(FixtureOAuthProvider {
            secret_store: Arc::clone(&secret_store),
            access_token: "oauth-token".to_string(),
        }),
    )
    .await;
    assert!(secret_store.set(services.secret_store()).is_ok());
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-oauth-path-metadata");
    services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: registration_request(HostedMcpAuthSelection::OAuth {
                    client_profile_id: None,
                }),
            },
        )
        .await
        .expect("explicit OAuth registration proves path metadata");
    install_fixture(&services, scope).await;

    let metadata_paths = server
        .requests()
        .into_iter()
        .filter(|request| request.method == "GET")
        .map(|request| request.path)
        .collect::<Vec<_>>();
    assert_eq!(
        metadata_paths,
        vec![
            "/.well-known/oauth-protected-resource/mcp",
            "/.well-known/oauth-authorization-server",
        ],
        "successful path-specific metadata must skip the origin-root fallback"
    );
}

#[tokio::test]
async fn explicit_oauth_registration_uses_the_selected_client_profile() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuth {
            access_token: "oauth-token".to_string(),
        },
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let services = build_lifecycle_test_services_with_oauth_client_profiles(
        "hosted-mcp-oauth-selected-profile",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
        Arc::new(FixtureOAuthClientProfileRegistry(
            fixture_oauth_client_profile(),
        )),
    )
    .await;
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-oauth-selected-profile");

    let registration = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: registration_request(HostedMcpAuthSelection::OAuth {
                    client_profile_id: Some("fixture-profile".to_string()),
                }),
            },
        )
        .await
        .expect("explicit OAuth registration admits the selected client profile");
    assert_eq!(
        registration.phase,
        ironclaw_extension_contracts::state::InstallationState::Installed
    );
    assert!(registration.blockers.is_empty());

    let definition = services
        .extension_management
        .installation_store_for_test()
        .get_registered_package_definition(&ExtensionId::new("mcp-fixture").expect("extension id"))
        .await
        .expect("registered-definition lookup")
        .expect("selected OAuth profile persists on the registered definition");
    assert!(
        services
            .extension_management
            .installation_store_for_test()
            .list_installations()
            .await
            .expect("installation listing succeeds")
            .is_empty(),
        "selecting an OAuth profile during registration must not install the extension"
    );
    let definition = definition.definition();
    assert!(matches!(
        definition.resolved().mcp.as_ref().map(|mcp| &mcp.registration_auth),
        Some(HostedMcpAuthSelection::OAuth {
            client_profile_id: Some(profile_id),
        }) if profile_id == "fixture-profile"
    ));
    let Some(VendorAuthRecipe::Oauth2Code(recipe)) = definition
        .resolved()
        .auth
        .first()
        .and_then(|auth| auth.recipe.as_ref())
    else {
        panic!("selected profile must persist an OAuth recipe")
    };
    let credentials = recipe
        .client_credentials
        .as_ref()
        .expect("selected OAuth profile must persist credential handles");
    assert_eq!(
        credentials.client_id_handle.as_str(),
        "fixture-oauth-client-id"
    );
    assert_eq!(
        credentials
            .client_secret_handle
            .as_ref()
            .map(SecretHandle::as_str),
        Some("fixture-oauth-client-secret")
    );
}

#[tokio::test]
async fn explicit_oauth_registration_rejects_an_unknown_client_profile_without_persistence() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuth {
            access_token: "oauth-token".to_string(),
        },
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    let services = build_lifecycle_test_services(
        "hosted-mcp-oauth-unknown-profile",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
    )
    .await;
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-oauth-unknown-profile");

    let error = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: registration_request(HostedMcpAuthSelection::OAuth {
                    client_profile_id: Some("unknown-profile".to_string()),
                }),
            },
        )
        .await
        .expect_err("an unknown OAuth client profile must be rejected during registration");
    assert_eq!(error.kind, ProductSurfaceErrorKind::Validation);
    assert!(
        services
            .extension_management
            .installation_store_for_test()
            .get_registered_package_definition(
                &ExtensionId::new("mcp-fixture").expect("extension id")
            )
            .await
            .expect("registered-definition lookup")
            .is_none(),
        "an unknown client profile must not persist a definition"
    );
}

#[tokio::test]
async fn automatic_registration_requires_an_explicit_choice_when_oauth_has_no_dcr_endpoint() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuth {
            access_token: "oauth-token".to_string(),
        },
        vec![HostedMcpTool::read_only("search", json!("ok"))],
    )
    .await;
    server.script_authorization_server_response(ScriptedMetadataResponse::new(
        axum::http::StatusCode::OK,
        serde_json::to_vec(&json!({
            "issuer": "https://auth.example.test",
            "authorization_endpoint": "https://auth.example.test/authorize",
            "token_endpoint": "https://auth.example.test/token",
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"]
        }))
        .expect("GitHub-shaped authorization metadata serializes"),
    ));
    let services = build_lifecycle_test_services(
        "hosted-mcp-oauth-without-dcr",
        Some(Arc::new(HostedMcpRegistrationNetworkEgress::for_server(
            &server,
        ))),
        false,
    )
    .await;
    let registration = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(webui_gate_resource_scope_for_owner(
                "hosted-mcp-oauth-without-dcr",
            )),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: automatic_request(),
            },
        )
        .await
        .expect("automatic registration returns an actionable auth choice");

    assert!(registration.blockers.iter().any(|blocker| matches!(
        blocker,
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Setup {
            ref_id: Some(ref_id),
        } if ref_id.as_str()
            == ironclaw_product_contracts::package_lifecycle::HOSTED_MCP_AUTH_SELECTION_BLOCKER_REF
    )));
    assert!(
        services
            .extension_management
            .installation_store_for_test()
            .get_registered_package_definition(
                &ExtensionId::new("mcp-fixture").expect("extension id")
            )
            .await
            .expect("definition readback")
            .is_none(),
        "OAuth metadata without a usable client path must not persist an unusable package"
    );
}

/// Drives all four `fetch_oauth_metadata` failure branches (transport error,
/// non-200 status, oversized body, malformed JSON) through registration
/// preflight. Each sub-case gets its own fixture server/services (the scripted
/// overrides are one-shot), but shares one assertion: the failure surfaces at
/// registration and no package definition is persisted.
#[tokio::test]
async fn oauth_metadata_fetch_failures_stop_registration_before_persistence() {
    /// Discriminates the two outcomes `fetch_oauth_metadata` failures produce.
    enum ExpectedRegistrationOutcome {
        /// A transport failure remains retryable.
        Retryable,
        /// Invalid metadata is a non-retryable validation failure.
        PropagatedValidationError,
    }

    async fn assert_metadata_failure_blocks_preparation(
        user_id: &str,
        egress: Arc<dyn ironclaw_network::NetworkHttpEgress>,
        expected: ExpectedRegistrationOutcome,
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
        let registration_result = services
            .lifecycle_service
            .execute(
                lifecycle_product_context(scope),
                LifecycleProductAction::ExtensionRegisterHostedMcp {
                    request: automatic_request(),
                },
            )
            .await;
        let error = registration_result
            .err()
            .unwrap_or_else(|| panic!("invalid OAuth metadata must stop registration"));
        match expected {
            ExpectedRegistrationOutcome::Retryable => {
                assert!(
                    error.retryable,
                    "transport failure must remain retryable: {error:#?}"
                );
            }
            ExpectedRegistrationOutcome::PropagatedValidationError => {
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
        let definition = installation_store
            .get_registered_package_definition(
                &ExtensionId::new("mcp-fixture").expect("extension id"),
            )
            .await
            .expect("registered definition readback");
        assert!(
            definition.is_none(),
            "a metadata-fetch failure must not persist an unusable definition"
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
        ExpectedRegistrationOutcome::Retryable,
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
        ExpectedRegistrationOutcome::PropagatedValidationError,
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
        ExpectedRegistrationOutcome::PropagatedValidationError,
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
        ExpectedRegistrationOutcome::PropagatedValidationError,
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
        "registration proves OAuth before persisting"
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

// The `CustomMcpRegistrationService::register` / `ExtensionLifecycleManager::
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

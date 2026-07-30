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
    HostedMcpTool,
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
use ironclaw_extension_host::lifecycle_test_support::{
    build_lifecycle_test_services, build_lifecycle_test_services_with_auth_provider,
    invoke_with_local_dev_approval, lifecycle_product_context, webui_gate_resource_scope_for_owner,
};
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExtensionId,
    GrantConstraints, HostedMcpAuthSelection, HostedMcpEndpoint, LifecyclePackageId, MountView,
    NetworkPolicy, NetworkScheme, NetworkTargetPattern, Principal, RegisterHostedMcpRequest,
    RuntimeKind, SecretHandle, TrustClass,
};
use ironclaw_product::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction, LifecycleProductPayload,
    LifecycleProductService,
};
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
    scope: ironclaw_host_api::ResourceScope,
    capability: &str,
) -> ironclaw_host_api::ExecutionContext {
    let grantee = ExtensionId::new("hosted-mcp-registration-test").expect("test extension id");
    let mut context = ironclaw_host_api::ExecutionContext::local_default(
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
                    allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
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
    context.run_id = Some(ironclaw_host_api::RunId::new());
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

fn no_auth_request() -> RegisterHostedMcpRequest {
    registration_request(HostedMcpAuthSelection::NoAuth)
}

fn oauth_request() -> RegisterHostedMcpRequest {
    registration_request(HostedMcpAuthSelection::OAuth {
        client_profile_id: None,
    })
}

fn fixture_package_ref() -> LifecyclePackageRef {
    LifecyclePackageRef::new(LifecyclePackageKind::Extension, "mcp-fixture")
        .expect("fixture package ref")
}

fn credential_provider_from_response(
    response: &ironclaw_host_api::LifecycleProductResponse,
) -> AuthProviderId {
    response
        .blockers
        .iter()
        .find_map(|blocker| match blocker {
            ironclaw_host_api::LifecycleReadinessBlocker::Credential {
                ref_id: Some(provider),
            } => AuthProviderId::new(provider.as_str()).ok(),
            _ => None,
        })
        .expect("credential blocker identifies the provider for the existing auth UI")
}

async fn submit_fixture_bearer(
    services: &ironclaw_extension_host::lifecycle_test_support::ExtensionLifecycleTestServices,
    scope: ironclaw_host_api::ResourceScope,
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
                request: no_auth_request(),
            },
        )
        .await;
    let registration = match registration_result {
        Ok(response) => response,
        Err(error) => panic!(
            "public lifecycle action registers, installs, and activates: {error:?}; fixture requests: {:?}",
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
        ironclaw_host_api::InstallationState::Active,
        "no-auth registration should finish preparation and activation; response: {registration:#?}; fixture requests: {:#?}",
        server.requests(),
    );
    let exact_retry = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: no_auth_request(),
            },
        )
        .await
        .expect("an exact tenant definition retry is idempotent");
    assert_eq!(exact_retry.package_ref, registration.package_ref);
    assert_eq!(
        exact_retry.phase,
        ironclaw_host_api::InstallationState::Active
    );
    let mut conflicting = no_auth_request();
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
    let outcome = invoke_with_local_dev_approval(
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
                request: no_auth_request(),
            },
        )
        .await
        .expect("tenant admin registers and installs the definition");

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
        ironclaw_host_api::InstallationState::Active
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
        HostedMcpAuthPolicy::ExactBearer {
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
                request: registration_request(HostedMcpAuthSelection::Bearer),
            },
        )
        .await
        .expect("bearer MCP definition is admitted before credentials exist");
    assert_eq!(
        registration.phase,
        ironclaw_host_api::InstallationState::Installed
    );
    assert!(
        registration.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_host_api::LifecycleReadinessBlocker::Credential { .. }
        )),
        "registration exposes ordinary credential readiness"
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

    let unfinished_retry = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect("agent install retry remains actionable");
    assert_eq!(
        unfinished_retry.phase,
        ironclaw_host_api::InstallationState::Installed
    );

    let provider = credential_provider_from_response(&registration);
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
    let completed_retry = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_package_ref(),
            },
        )
        .await
        .expect("agent install converges after auth setup");
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
        server
            .requests()
            .iter()
            .any(|request| request.authorization_matches),
        "activation reaches the MCP with the host-injected bearer"
    );
}

#[tokio::test]
async fn oauth_registration_discovers_standard_metadata_then_hands_off_to_generic_auth_setup() {
    let server = HostedMcpRegistrationServer::start(
        HostedMcpAuthPolicy::OAuth {
            access_token: "oauth-token".to_string(),
        },
        vec![HostedMcpTool::read_only("search", json!("ok"))],
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
    fixture_secret_store
        .set(services.secret_store())
        .expect("fixture OAuth provider binds the product-auth secret store once");
    let scope = webui_gate_resource_scope_for_owner("hosted-mcp-oauth-user");

    let registration = services
        .lifecycle_service
        .execute(
            lifecycle_product_context(scope.clone()),
            LifecycleProductAction::ExtensionRegisterHostedMcp {
                request: oauth_request(),
            },
        )
        .await
        .expect("OAuth MCP registration should discover its standards metadata before auth");

    assert_eq!(
        registration.phase,
        ironclaw_host_api::InstallationState::Installed
    );
    let provider = credential_provider_from_response(&registration);
    assert!(
        registration.blockers.iter().any(|blocker| matches!(
            blocker,
            ironclaw_host_api::LifecycleReadinessBlocker::Credential { .. }
        )),
        "OAuth discovery must return the ordinary credential setup blocker: {registration:#?}"
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

    let auth_scope = AuthProductScope::credential_owner(&scope, AuthSurface::Api);
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
    let completed = services
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
        .expect("OAuth callback uses the production continuation dispatcher");
    assert_eq!(
        completed.continuation,
        AuthContinuationRef::LifecycleActivation {
            package_ref: ironclaw_auth::LifecyclePackageRef::new("mcp-fixture")
                .expect("auth package ref"),
        }
    );
    let capability = services
        .extension_management
        .active_model_visible_capabilities()
        .await
        .expect("callback activation publishes the OAuth MCP tool")
        .into_iter()
        .find(|capability| capability.id.as_str().ends_with(".search"))
        .expect("OAuth MCP search capability");
    let outcome = invoke_with_local_dev_approval(
        &services,
        capability.id.as_str(),
        runtime_context(scope, capability.id.as_str()),
        json!({}),
    )
    .await;
    assert!(matches!(
        outcome,
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(_)
    ));
    assert!(server.requests().iter().any(|request| {
        request.rpc_method.as_deref() == Some("tools/call") && request.authorization_matches
    }));
}

/// Manual smoke for the official unauthenticated Microsoft Release
/// Communications endpoint. It stays opt-in because the vendor-owned catalog
/// and public-network availability are intentionally outside the hermetic PR
/// contract. It does, however, cross the same policy-mediated egress and
/// lifecycle paths as the deterministic fixture journeys.
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
        ironclaw_host_api::InstallationState::Active
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
    let outcome = invoke_with_local_dev_approval(
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

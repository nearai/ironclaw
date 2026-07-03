#[cfg(test)]
mod tests {
    use super::super::*;

    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, header},
    };
    use ironclaw_auth::{
        AuthFlowManager, AuthProviderId, AuthSurface, CredentialAccountLabel,
        CredentialAccountService, CredentialAccountStatus, CredentialOwnership,
        NewCredentialAccount, ProviderScope,
    };
    use ironclaw_capabilities::{CapabilityObligationHandler, CapabilityObligationRequest};
    use ironclaw_host_api::{
        MissionId, NetworkMethod, ResourceScope, RuntimeHttpEgress, RuntimeHttpEgressRequest,
        RuntimeHttpEgressResponse, SecretHandle, TenantId, ThreadId, UserId,
    };
    use ironclaw_product_workflow::WebUiAuthenticatedCaller;
    use ironclaw_secrets::InMemorySecretStore;
    use serde_json::json;
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::RebornAuthContinuationDispatcher;
    use crate::notion_oauth::notion_provider_spec;
    use crate::oauth_dcr::{OAuthDcrProvider, OAuthDcrProviderConfig, OAuthDcrProviderRegistry};

    #[derive(Debug)]
    struct NoopDispatcher;

    #[async_trait]
    impl RebornAuthContinuationDispatcher for NoopDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            _event: ironclaw_auth::AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            Ok(())
        }
    }

    fn test_resource_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new("user-alpha").expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn test_caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new("user-alpha").expect("user"),
            None,
            None,
        )
    }

    #[derive(Debug)]
    struct RouteDcrSetupEgress;

    #[async_trait]
    impl RuntimeHttpEgress for RouteDcrSetupEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
            let body = match request.url.as_str() {
            "https://mcp.notion.com/mcp/.well-known/oauth-protected-resource" => {
                br#"{"authorization_servers":["https://oauth.notion.com"]}"#.to_vec()
            }
            "https://oauth.notion.com/.well-known/oauth-authorization-server" => {
                br#"{"authorization_endpoint":"https://oauth.notion.com/authorize","token_endpoint":"https://oauth.notion.com/token","registration_endpoint":"https://oauth.notion.com/register"}"#.to_vec()
            }
            "https://oauth.notion.com/register" => br#"{"client_id":"dcr-client","registration_client_uri":"https://oauth.notion.com/register/dcr-client","registration_access_token":"registration-token"}"#.to_vec(),
            "https://oauth.notion.com/register/dcr-client"
                if request.method == NetworkMethod::Delete =>
            {
                br#"{}"#.to_vec()
            }
            other => panic!("unexpected DCR route egress URL: {other}"),
        };
            Ok(RuntimeHttpEgressResponse {
                status: 200,
                headers: Vec::new(),
                request_bytes: request.body.len() as u64,
                response_bytes: body.len() as u64,
                body,
                saved_body: None,
                redaction_applied: false,
            })
        }
    }

    #[derive(Debug)]
    struct NoopObligationHandler;

    #[async_trait]
    impl CapabilityObligationHandler for NoopObligationHandler {
        async fn satisfy(
            &self,
            _request: CapabilityObligationRequest<'_>,
        ) -> Result<(), ironclaw_capabilities::CapabilityObligationError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn extension_oauth_start_handler_binds_reconnect_to_existing_owner_account() {
        // Regression (#4935 defect A — account fork): a 2nd OAuth flow for the
        // same owner must BIND to the owner's existing account, not fork a
        // duplicate. The existing account differs from the flow scope only by
        // `invocation_id` (fresh per flow) — the old `scope_matches`
        // full-equality compared `invocation_id` and so never matched, forking a
        // new account on every reconnect. The bind is now owner-granularity.
        let secret_store = Arc::new(InMemorySecretStore::new());
        let dcr_provider = Arc::new(
            OAuthDcrProvider::new(
                OAuthDcrProviderConfig {
                    spec: notion_provider_spec(),
                    callback_origin: "http://127.0.0.1:3000".to_string(),
                    client_name: "Ironclaw".to_string(),
                    account_label: CredentialAccountLabel::new("notion").expect("label"),
                    scopes: Vec::new(),
                },
                Arc::new(RouteDcrSetupEgress),
                secret_store,
                Arc::new(NoopObligationHandler),
            )
            .expect("DCR provider"),
        );
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let product_auth =
            RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopDispatcher))
                .with_dcr_oauth_registry(Arc::new(OAuthDcrProviderRegistry::new(vec![
                    dcr_provider,
                ])));
        let state = ProductAuthRouteState::new(
            Arc::new(product_auth),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));
        // Owner fields equal to the flow scope; only `invocation_id` differs.
        let existing_scope = AuthProductScope::new(test_resource_scope(), AuthSurface::Callback);
        let existing = shared
            .create_account(NewCredentialAccount {
                scope: existing_scope,
                provider: AuthProviderId::new("notion").expect("provider"),
                label: CredentialAccountLabel::new("work notion").expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("existing-notion-access").expect("secret")),
                refresh_secret: Some(SecretHandle::new("existing-notion-refresh").expect("secret")),
                scopes: Vec::new(),
            })
            .await
            .expect("seed existing account");
        let flow_invocation_id = InvocationId::new();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/notion/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "provider": "notion",
                            "account_label": "work notion",
                            "scopes": [],
                            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
                            "invocation_id": flow_invocation_id.to_string(),
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("start json");
        let flow_id = AuthFlowId::from_uuid(
            Uuid::parse_str(json["flow_id"].as_str().expect("flow id")).expect("flow uuid"),
        );
        let mut flow_resource = test_resource_scope();
        flow_resource.invocation_id = flow_invocation_id;
        let flow_scope = AuthProductScope::new(flow_resource, AuthSurface::Callback);
        let flow = shared
            .get_flow(&flow_scope, flow_id)
            .await
            .expect("flow lookup")
            .expect("flow");
        assert_eq!(
            flow.update_binding
                .expect("reconnect must bind to the owner's existing account")
                .account_id,
            existing.id,
        );
    }

    #[tokio::test]
    async fn extension_oauth_start_handler_does_not_bind_across_owner_boundary() {
        // Owner isolation guard for defect A: an account owned by a DIFFERENT
        // agent must never be bound by this owner's reconnect. tenant/user/
        // agent/project stay hard-`==` in the owner match, so a cross-owner
        // account is invisible and the flow starts with no update binding.
        let secret_store = Arc::new(InMemorySecretStore::new());
        let dcr_provider = Arc::new(
            OAuthDcrProvider::new(
                OAuthDcrProviderConfig {
                    spec: notion_provider_spec(),
                    callback_origin: "http://127.0.0.1:3000".to_string(),
                    client_name: "Ironclaw".to_string(),
                    account_label: CredentialAccountLabel::new("notion").expect("label"),
                    scopes: Vec::new(),
                },
                Arc::new(RouteDcrSetupEgress),
                secret_store,
                Arc::new(NoopObligationHandler),
            )
            .expect("DCR provider"),
        );
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let product_auth =
            RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopDispatcher))
                .with_dcr_oauth_registry(Arc::new(OAuthDcrProviderRegistry::new(vec![
                    dcr_provider,
                ])));
        let state = ProductAuthRouteState::new(
            Arc::new(product_auth),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));
        // Same tenant/user as the caller, but a different agent_id -> different
        // owner. Must NOT be bound.
        let mut other_owner = test_resource_scope();
        other_owner.agent_id = Some(AgentId::new("agent-other").expect("agent"));
        shared
            .create_account(NewCredentialAccount {
                scope: AuthProductScope::new(other_owner, AuthSurface::Callback),
                provider: AuthProviderId::new("notion").expect("provider"),
                label: CredentialAccountLabel::new("other-agent notion").expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("other-owner-access").expect("secret")),
                refresh_secret: Some(SecretHandle::new("other-owner-refresh").expect("secret")),
                scopes: Vec::new(),
            })
            .await
            .expect("seed cross-owner account");
        let flow_invocation_id = InvocationId::new();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/notion/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "provider": "notion",
                            "account_label": "work notion",
                            "scopes": [],
                            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
                            "invocation_id": flow_invocation_id.to_string(),
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("start json");
        let flow_id = AuthFlowId::from_uuid(
            Uuid::parse_str(json["flow_id"].as_str().expect("flow id")).expect("flow uuid"),
        );
        let mut flow_resource = test_resource_scope();
        flow_resource.invocation_id = flow_invocation_id;
        let flow_scope = AuthProductScope::new(flow_resource, AuthSurface::Callback);
        let flow = shared
            .get_flow(&flow_scope, flow_id)
            .await
            .expect("flow lookup")
            .expect("flow");
        assert!(flow.update_binding.is_none());
    }

    #[tokio::test]
    async fn extension_google_oauth_start_rebinds_account_authorized_in_a_different_thread() {
        // #4935 user-visible regression: a Google account a user authorized in
        // one chat thread/mission must be rebound — not forked — when the OAuth
        // reconnect starts from a different context. The bind is owner-
        // granularity (tenant/user/agent/project), so the account's
        // `thread_id`/`mission_id` are stripped from the match; the old
        // `scope_matches` full-equality would have missed this account and
        // forked a duplicate on every reconnect.
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let product_auth = Arc::new(RebornProductAuthServices::from_shared(
            shared.clone(),
            Arc::new(NoopDispatcher),
        ));
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_google_oauth(
            GoogleOAuthRouteConfig::new(
                "google-client.apps.googleusercontent.com",
                "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
            )
            .expect("google oauth route config"),
        );
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));

        // The existing account was authorized in thread-auth-1 / mission-auth-1.
        let mut existing_resource = test_resource_scope();
        existing_resource.thread_id = Some(ThreadId::new("thread-auth-1").expect("thread"));
        existing_resource.mission_id = Some(MissionId::new("mission-auth-1").expect("mission"));
        let existing_scope = AuthProductScope::new(existing_resource, AuthSurface::Callback);
        let account = shared
            .create_account(NewCredentialAccount {
                scope: existing_scope,
                provider: AuthProviderId::new(GOOGLE_PROVIDER_ID).expect("provider"),
                label: CredentialAccountLabel::new("google-drive google").expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("google-drive-access").expect("secret")),
                refresh_secret: Some(SecretHandle::new("google-drive-refresh").expect("secret")),
                scopes: vec![
                    ProviderScope::new("https://www.googleapis.com/auth/drive")
                        .expect("provider scope"),
                ],
            })
            .await
            .expect("seed configured google account");

        // The reconnect starts from a different context: the Google start route
        // carries no thread/mission, so the flow scope has neither — i.e. a
        // different thread/mission than where the account was authorized.
        let flow_invocation_id = InvocationId::new();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/google-drive/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "provider": GOOGLE_PROVIDER_ID,
                            "account_label": "google-drive google",
                            "scopes": ["https://www.googleapis.com/auth/drive"],
                            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
                            "invocation_id": flow_invocation_id.to_string(),
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("start json");
        let flow_id = AuthFlowId::from_uuid(
            Uuid::parse_str(json["flow_id"].as_str().expect("flow id")).expect("flow uuid"),
        );
        // The flow is stored under the reconnect scope (no thread/mission).
        let mut flow_resource = test_resource_scope();
        flow_resource.invocation_id = flow_invocation_id;
        let flow_scope = AuthProductScope::new(flow_resource, AuthSurface::Callback);
        let flow = shared
            .get_flow(&flow_scope, flow_id)
            .await
            .expect("flow lookup")
            .expect("flow");
        let update_binding = flow
            .update_binding
            .expect("cross-thread reconnect must rebind the owner's existing account");
        assert_eq!(update_binding.account_id, account.id);
    }
}

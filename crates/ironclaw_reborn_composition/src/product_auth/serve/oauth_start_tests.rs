#[cfg(test)]
mod tests {
    use super::super::*;

    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, header},
    };
    use ironclaw_auth::{
        AuthEngine, AuthEngineDeps, AuthFlowManager, AuthProviderId, AuthSurface,
        CredentialAccountLabel, CredentialAccountService, CredentialAccountStatus,
        CredentialOwnership, EngineCallbackBase, NewCredentialAccount, ProviderScope,
        ResolvedVendorAuthRecipe, StaticAuthRecipeResolver,
    };
    use ironclaw_host_api::ProductSurfaceCaller;
    use ironclaw_host_api::{
        MissionId, ResourceScope, RuntimeHttpEgress, RuntimeHttpEgressRequest,
        RuntimeHttpEgressResponse, SecretHandle, TenantId, ThreadId, UserId, VendorAuthRecipe,
    };
    use ironclaw_secrets::FilesystemSecretStore;
    use serde_json::json;
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::RebornAuthContinuationDispatcher;

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
        async fn dispatch_canceled_auth_continuation(
            &self,
            _event: ironclaw_auth::AuthContinuationEvent,
        ) -> Result<(), ironclaw_auth::AuthProductError> {
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

    fn test_caller() -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new("user-alpha").expect("user"),
            None,
            None,
        )
    }

    #[derive(Debug)]
    struct PanicEgress;

    #[async_trait]
    impl RuntimeHttpEgress for PanicEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
            panic!(
                "start-route flow preparation must not reach a vendor: {}",
                request.url
            );
        }
    }

    #[derive(Debug)]
    struct StaticCredentials;

    #[async_trait]
    impl ironclaw_auth::EngineClientCredentialsSource for StaticCredentials {
        async fn resolve(
            &self,
            vendor: &str,
            _credentials: &ironclaw_host_api::RecipeClientCredentials,
        ) -> Result<ironclaw_auth::EngineOAuthClientMaterial, AuthProductError> {
            Ok(ironclaw_auth::EngineOAuthClientMaterial {
                client_id: ironclaw_auth::OAuthClientId::new(format!("{vendor}-client-id"))?,
                client_secret: None,
            })
        }
    }

    fn vendor_recipe(vendor: &str, scopes: &[&str]) -> ResolvedVendorAuthRecipe {
        let recipe: VendorAuthRecipe = serde_json::from_value(json!({
            "method": "oauth2_code",
            "display_name": format!("{vendor} account"),
            "authorization_endpoint": format!("https://auth.{vendor}.example/authorize"),
            "token_endpoint": format!("https://auth.{vendor}.example/token"),
            "scopes": scopes,
            "client_credentials": { "client_id_handle": format!("{vendor}_oauth_client_id") },
            "token_response": { "access_token": "/access_token" },
        }))
        .expect("recipe parses");
        ResolvedVendorAuthRecipe {
            vendor: vendor.to_string(),
            recipe,
            token_exchange_resource: None,
        }
    }

    fn test_engine(recipes: Vec<ResolvedVendorAuthRecipe>) -> Arc<AuthEngine> {
        Arc::new(AuthEngine::new(AuthEngineDeps {
            recipes: Arc::new(StaticAuthRecipeResolver::new(recipes)),
            client_credentials: Arc::new(StaticCredentials),
            egress: Arc::new(PanicEgress),
            secret_store: Arc::new(FilesystemSecretStore::ephemeral()),
            callback_base: EngineCallbackBase::new(
                "http://127.0.0.1:3000/api/reborn/product-auth/oauth",
            )
            .expect("callback base"),
            dcr_client_name: "Ironclaw".to_string(),
        }))
    }

    fn engine_backed_route_state(
        shared: Arc<ironclaw_auth::InMemoryAuthProductServices>,
        recipes: Vec<ResolvedVendorAuthRecipe>,
        extension_id: &str,
        requirement_name: &str,
        provider: &str,
        scopes: &[&str],
    ) -> ProductAuthRouteState {
        let product_auth = RebornProductAuthServices::from_shared(shared, Arc::new(NoopDispatcher))
            .with_auth_engine(test_engine(recipes));
        let mut state = ProductAuthRouteState::new(
            Arc::new(product_auth),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        state.installed_extension_lookup = Some(Arc::new(InstalledExtensionLookup::Scripted {
            extension_id: ExtensionId::new(extension_id).expect("test extension id"),
            requirement_name: requirement_name.to_string(),
            requirement: InstalledExtensionOAuthRequirement {
                provider: provider.to_string(),
                account_label: format!("{extension_id} {provider}"),
                scopes: scopes.iter().map(|scope| (*scope).to_string()).collect(),
            },
        }));
        state
    }

    #[tokio::test]
    async fn extension_oauth_start_records_server_owned_lifecycle_activation() {
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let state = engine_backed_route_state(
            shared.clone(),
            vec![vendor_recipe("vendor-a", &[])],
            "tool-a",
            "primary_oauth",
            "vendor-a",
            &[],
        );
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));
        let invocation_id = InvocationId::new();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/tool-a/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "requirement": "primary_oauth",
                            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
                            "invocation_id": invocation_id.to_string(),
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
        assert_eq!(json["continuation"]["type"], "lifecycle_activation");
        assert_eq!(json["continuation"]["package_ref"], "tool-a");

        let flow_id = AuthFlowId::from_uuid(
            Uuid::parse_str(json["flow_id"].as_str().expect("flow id")).expect("flow uuid"),
        );
        let mut resource = test_resource_scope();
        resource.invocation_id = invocation_id;
        let flow = shared
            .get_flow(
                &AuthProductScope::new(resource, AuthSurface::Callback),
                flow_id,
            )
            .await
            .expect("flow lookup")
            .expect("flow");
        assert_eq!(
            flow.continuation,
            ironclaw_auth::AuthContinuationRef::LifecycleActivation {
                package_ref: ironclaw_auth::LifecyclePackageRef::new("tool-a")
                    .expect("package ref"),
            }
        );
    }

    /// Regression: an installed extension must not mint credentials for a
    /// different extension's globally configured OAuth vendor/scopes. The
    /// route must resolve the selected requirement from the installed
    /// extension's manifest instead of treating the browser payload plus the
    /// global recipe catalog as authority.
    #[tokio::test]
    async fn extension_oauth_start_rejects_cross_extension_oauth_requirement() {
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let state = engine_backed_route_state(
            shared.clone(),
            vec![
                vendor_recipe("vendor-a", &["items:read"]),
                vendor_recipe("vendor-b", &["admin:write"]),
            ],
            "tool-a",
            "primary_oauth",
            "vendor-a",
            &["items:read"],
        );
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));
        let invocation_id = InvocationId::new();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/tool-a/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "requirement": "primary_oauth",
                            "provider": "vendor-b",
                            "account_label": "cross-extension account",
                            "scopes": ["admin:write"],
                            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
                            "invocation_id": invocation_id.to_string(),
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn extension_oauth_start_handler_binds_reconnect_to_existing_owner_account() {
        // Regression (#4935 defect A — account fork): a 2nd OAuth flow for the
        // same owner must BIND to the owner's existing account, not fork a
        // duplicate. The existing account differs from the flow scope only by
        // `invocation_id` (fresh per flow) — the old `scope_matches`
        // full-equality compared `invocation_id` and so never matched, forking a
        // new account on every reconnect. The bind is now owner-granularity.
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let state = engine_backed_route_state(
            shared.clone(),
            vec![vendor_recipe("notion", &[])],
            "notion",
            "notion_oauth",
            "notion",
            &[],
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
                            "requirement": "notion_oauth",
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
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let state = engine_backed_route_state(
            shared.clone(),
            vec![vendor_recipe("notion", &[])],
            "notion",
            "notion_oauth",
            "notion",
            &[],
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
                            "requirement": "notion_oauth",
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
    async fn extension_oauth_start_rebinds_account_authorized_in_a_different_thread() {
        // #4935 user-visible regression: an account a user authorized in one
        // chat thread/mission must be rebound — not forked — when the OAuth
        // reconnect starts from a different context. The bind is owner-
        // granularity (tenant/user/agent/project), so the account's
        // `thread_id`/`mission_id` are stripped from the match; the old
        // `scope_matches` full-equality would have missed this account and
        // forked a duplicate on every reconnect.
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let state = engine_backed_route_state(
            shared.clone(),
            vec![vendor_recipe("driveco", &["files:read"])],
            "drive-ext",
            "drive_oauth",
            "driveco",
            &["files:read"],
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
                provider: AuthProviderId::new("driveco").expect("provider"),
                label: CredentialAccountLabel::new("drive account").expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("drive-access").expect("secret")),
                refresh_secret: Some(SecretHandle::new("drive-refresh").expect("secret")),
                scopes: vec![ProviderScope::new("files:read").expect("provider scope")],
            })
            .await
            .expect("seed configured account");

        // The reconnect starts from a different context: the start route
        // carries no thread/mission, so the flow scope has neither — i.e. a
        // different thread/mission than where the account was authorized.
        let flow_invocation_id = InvocationId::new();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/drive-ext/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "requirement": "drive_oauth",
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

    /// The route rejects requirement keys outside the installed manifest and
    /// forwards only that manifest requirement's scopes into the auth engine.
    #[tokio::test]
    async fn extension_oauth_start_uses_manifest_owned_requirement_scopes() {
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let state = engine_backed_route_state(
            shared.clone(),
            vec![vendor_recipe("acmevendor", &["msg:read", "msg:write"])],
            "acme-messenger",
            "acme_oauth",
            "acmevendor",
            &["msg:read", "msg:write"],
        );
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));

        let start = |requirement: &str| {
            let app = app.clone();
            let requirement = requirement.to_string();
            async move {
                app.oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/api/webchat/v2/extensions/acme-messenger/setup/oauth/start")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(
                            json!({
                                "requirement": requirement,
                                "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
                                "invocation_id": InvocationId::new().to_string(),
                            })
                            .to_string(),
                        ))
                        .expect("request"),
                )
                .await
                .expect("route response")
            }
        };

        // A requirement key not owned by this extension is rejected before
        // the global recipe catalog can authorize it.
        let response = start("admin_oauth").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // The selected manifest requirement supplies the provider scopes.
        let response = start("acme_oauth").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("start json");
        let url = url::Url::parse(json["authorization_url"].as_str().expect("url")).expect("parse");
        let scope = url
            .query_pairs()
            .find(|(key, _)| key == "scope")
            .map(|(_, value)| value.into_owned())
            .expect("scope param");
        assert_eq!(scope, "msg:read msg:write");
    }
}

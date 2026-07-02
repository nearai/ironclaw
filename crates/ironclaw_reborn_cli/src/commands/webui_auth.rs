//! WebChat v2 auth-surface assembly for `ironclaw-reborn serve`.
//!
//! Owns the one place that turns host config into the pair the listener
//! needs: the `WebuiAuthenticator` the protected v2 routes use, plus the
//! optional public login-route mount. `serve.rs` only wires host config
//! and calls [`build_webui_auth_surface`]; it does not itself open the
//! identity resolver, the local trigger-access store, run the
//! signed-session builder, or know the `Option`/provider invariants —
//! those live here, next to the admission adapter
//! ([`crate::commands::user_directory`]) and the startup config
//! ([`crate::commands::serve_sso`]).

use std::sync::Arc;

use anyhow::anyhow;
use ironclaw_reborn_composition::host_api::{AgentId, ProjectId, TenantId};
use ironclaw_reborn_composition::{
    LocalTriggerAccessStore, PublicRouteMount, RebornIdentityResolver, WebuiAuthenticator,
};
use ironclaw_reborn_webui_ingress::{
    SessionUserAccessValidator, SignedSessionLoginConfig, build_signed_session_login,
};
use secrecy::SecretString;

use crate::commands::serve_sso::SsoStartupConfig;
use crate::commands::user_directory::{
    LocalTriggerAccessBootstrap, LocalTriggerSessionAccessValidator, WebuiUserDirectory,
};

/// The composed WebChat v2 auth surface: the authenticator the protected
/// routes verify bearers with, plus the optional public login-route mount
/// (present only when SSO providers are configured).
pub(crate) struct WebuiAuthSurface {
    pub(crate) authenticator: Arc<dyn WebuiAuthenticator>,
    pub(crate) public_mount: Option<PublicRouteMount>,
}

/// How to seed local-dev trigger-fire access for SSO users on login.
///
/// Carries the already-open local trigger-access store plus the scope an
/// admitted user's access row is seeded under. `serve.rs` opens the store once
/// through the active runtime profile so SSO and trigger-poller wiring share
/// the same backend.
#[derive(Clone)]
pub(crate) struct LocalTriggerAccessBootstrapConfig {
    pub(crate) store: Arc<dyn LocalTriggerAccessStore>,
    pub(crate) tenant_id: TenantId,
    pub(crate) agent_id: AgentId,
    pub(crate) project_id: Option<ProjectId>,
}

/// Build the auth surface from resolved startup config.
///
/// With no SSO provider configured (`sso_startup` is `None`), the listener
/// keeps its plain env-bearer authenticator and mounts no public routes.
/// With providers configured, this layers the fail-closed email-domain
/// admission adapter on top of the runtime-owned canonical Reborn identity
/// resolver and hands the result to the ingress signed-session builder.
///
/// `identity_resolver` is the resolver the runtime opened on its own
/// substrate handle. It is `None` only when the runtime carries no
/// local-runtime substrate; with SSO configured that is unrecoverable, so
/// this fails closed rather than minting users against a missing store.
///
/// When `local_trigger_access` is present and SSO is configured, admitted
/// users get a local trigger-access row seeded on each login (via the
/// admission adapter). There is no startup reconciliation in this path: the
/// bootstrap only seeds, it does not enumerate or revoke.
pub(crate) async fn build_webui_auth_surface(
    sso_startup: Option<SsoStartupConfig>,
    identity_resolver: Option<Arc<dyn RebornIdentityResolver>>,
    tenant_id: TenantId,
    session_signing_secret: SecretString,
    env_authenticator: Arc<dyn WebuiAuthenticator>,
    local_trigger_access: Option<LocalTriggerAccessBootstrapConfig>,
) -> anyhow::Result<WebuiAuthSurface> {
    let Some(sso) = sso_startup else {
        // No SSO providers: keep the env-bearer authenticator and mount no
        // public routes. There are no SSO logins to seed local trigger
        // access for, so any bootstrap config is unused on this path.
        return Ok(WebuiAuthSurface {
            authenticator: env_authenticator,
            public_mount: None,
        });
    };

    // The host `WebuiUserDirectory` adapter layers the fail-closed
    // email-domain admission allowlist on top of the runtime-owned resolver
    // before any user is created. No resolver means no durable user source —
    // fail closed instead of admitting SSO logins against nothing.
    let identity_resolver = identity_resolver.ok_or_else(|| {
        anyhow!(
            "WebChat v2 SSO is configured but the runtime exposes no identity \
             resolver (no local-runtime substrate); refusing to start"
        )
    })?;
    let local_trigger_access = local_trigger_access.ok_or_else(|| {
        anyhow!(
            "WebChat v2 SSO is configured but the runtime exposes no local access \
             store; refusing to mint signed sessions without an access validator"
        )
    })?;
    let session_epoch = sso.session_epoch;

    let mut user_directory = WebuiUserDirectory::new(
        identity_resolver,
        tenant_id.clone(),
        sso.allowed_email_domains,
    );
    user_directory = user_directory
        .with_local_trigger_access(local_trigger_access_bootstrap(local_trigger_access.clone()));

    let wiring = build_signed_session_login(SignedSessionLoginConfig {
        tenant_id,
        user_directory: Arc::new(user_directory),
        operator_secret: session_signing_secret,
        session_epoch,
        session_user_access_validator: Some(local_trigger_session_access_validator(
            local_trigger_access,
        )),
        base_url: sso.base_url,
        providers: sso.providers,
        env_authenticator,
    })
    .expect("non-empty providers always produce login wiring"); // safety: sso_startup_config_from_env returns None when providers is empty, so this Some(sso) arm always has a non-empty provider list

    eprintln!(
        "ironclaw-reborn: WebChat v2 SSO login mounted — \
         see GET /auth/providers for the enabled set"
    );
    Ok(WebuiAuthSurface {
        authenticator: wiring.authenticator,
        public_mount: Some(wiring.mount),
    })
}

fn local_trigger_access_bootstrap(
    config: LocalTriggerAccessBootstrapConfig,
) -> LocalTriggerAccessBootstrap {
    let LocalTriggerAccessBootstrapConfig {
        store,
        tenant_id,
        agent_id,
        project_id,
    } = config;
    LocalTriggerAccessBootstrap::new(store, tenant_id, agent_id, project_id)
}

fn local_trigger_session_access_validator(
    config: LocalTriggerAccessBootstrapConfig,
) -> Arc<dyn SessionUserAccessValidator> {
    let LocalTriggerAccessBootstrapConfig {
        store,
        tenant_id,
        agent_id,
        project_id,
    } = config;
    Arc::new(LocalTriggerSessionAccessValidator::new(
        store, tenant_id, agent_id, project_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Method, Request, StatusCode, header};
    use http_body_util::BodyExt;
    use ironclaw_reborn_composition::WebuiAuthentication;
    use ironclaw_reborn_webui_ingress::{
        OAuthError, OAuthProvider, OAuthProviderName, OAuthUserProfile,
    };
    use serde::Deserialize;
    use std::net::SocketAddr;
    use tower::ServiceExt;

    /// Bearer verifier that accepts nothing — stands in for the env-bearer
    /// authenticator without pulling in its construction requirements.
    struct RejectingAuth;

    #[async_trait]
    impl WebuiAuthenticator for RejectingAuth {
        async fn authenticate(&self, _token: &str) -> Option<WebuiAuthentication> {
            None
        }
    }

    struct StubProvider(OAuthProviderName);

    #[async_trait]
    impl OAuthProvider for StubProvider {
        fn name(&self) -> &OAuthProviderName {
            &self.0
        }

        fn authorization_url(
            &self,
            callback_url: &str,
            state: &str,
            _code_challenge: &str,
        ) -> String {
            format!("https://provider.example/authorize?redirect_uri={callback_url}&state={state}")
        }

        async fn exchange_code(
            &self,
            _code: &str,
            _callback_url: &str,
            _code_verifier: &str,
        ) -> Result<OAuthUserProfile, OAuthError> {
            Ok(OAuthUserProfile {
                provider_user_id: "google-subject-1".to_string(),
                email: Some("alice@example.com".to_string()),
                email_verified: true,
                verified_emails: vec!["alice@example.com".to_string()],
                display_name: None,
            })
        }
    }

    fn with_peer(mut request: Request<Body>) -> Request<Body> {
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));
        request
    }

    fn state_from_location(location: &str) -> String {
        let query = location.split_once('?').expect("query").1;
        for pair in query.split('&') {
            if let Some(value) = pair.strip_prefix("state=") {
                return value.to_string();
            }
        }
        panic!("no state in {location}");
    }

    fn ticket_from_landing(landing: &str) -> String {
        let query = landing.split_once('?').expect("query").1;
        let query = query.split_once('#').map(|(q, _)| q).unwrap_or(query);
        for pair in query.split('&') {
            if let Some(value) = pair.strip_prefix("login_ticket=") {
                return value.to_string();
            }
        }
        panic!("no login_ticket in {landing}");
    }

    #[derive(Deserialize)]
    struct SessionExchangeResponse {
        token: String,
    }

    async fn mint_session_token(public_mount: &PublicRouteMount) -> String {
        let router = public_mount.router.clone();
        let login = router
            .clone()
            .oneshot(with_peer(
                Request::builder()
                    .method(Method::GET)
                    .uri("/auth/login/google?redirect_after=%2Fv2")
                    .body(Body::empty())
                    .expect("request"),
            ))
            .await
            .expect("login request");
        assert_eq!(login.status(), StatusCode::TEMPORARY_REDIRECT);
        let auth_url = login
            .headers()
            .get(header::LOCATION)
            .expect("login Location")
            .to_str()
            .expect("utf-8")
            .to_string();
        let state = state_from_location(&auth_url);

        let callback = router
            .clone()
            .oneshot(with_peer(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/auth/callback/google?code=auth-code&state={state}"
                    ))
                    .body(Body::empty())
                    .expect("request"),
            ))
            .await
            .expect("callback request");
        assert_eq!(callback.status(), StatusCode::SEE_OTHER);
        let landing = callback
            .headers()
            .get(header::LOCATION)
            .expect("callback Location")
            .to_str()
            .expect("utf-8")
            .to_string();
        let ticket = ticket_from_landing(&landing);

        let response = router
            .oneshot(with_peer(
                Request::builder()
                    .method(Method::POST)
                    .uri("/auth/session/exchange")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "ticket": ticket }).to_string(),
                    ))
                    .expect("request"),
            ))
            .await
            .expect("session exchange request");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("exchange body")
            .to_bytes();
        let payload: SessionExchangeResponse = serde_json::from_slice(&bytes).expect("json");
        payload.token
    }

    #[tokio::test]
    async fn sso_without_identity_resolver_fails_closed() {
        // SSO providers configured but the runtime exposes no identity
        // resolver (no local-runtime substrate). Admitting logins against a
        // missing user source would silently mint users into nothing, so the
        // surface must refuse to start rather than fall back or panic.
        let sso = SsoStartupConfig {
            providers: Vec::new(),
            base_url: "https://app.example.com".to_string(),
            allowed_email_domains: vec!["example.com".to_string()],
            session_epoch: None,
        };

        let result = build_webui_auth_surface(
            Some(sso),
            None, // no resolver — the fail-closed branch under test
            TenantId::new("tenant-host").expect("tenant"),
            SecretString::from("session-signing-secret".to_string()),
            Arc::new(RejectingAuth),
            None,
        )
        .await;

        let error = match result {
            Ok(_) => panic!("configured SSO with no identity resolver must fail closed"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("no identity"),
            "startup error must name the missing identity resolver, got: {error}"
        );
    }

    #[tokio::test]
    async fn no_sso_keeps_env_authenticator_and_mounts_no_public_routes() {
        // With no SSO configured the surface is the plain env-bearer
        // authenticator and no public login routes — the absent-resolver
        // check must not fire on this path, and a bootstrap config is unused.
        let result = build_webui_auth_surface(
            None,
            None,
            TenantId::new("tenant-host").expect("tenant"),
            SecretString::from("session-signing-secret".to_string()),
            Arc::new(RejectingAuth),
            None,
        )
        .await;

        match result {
            Ok(surface) => assert!(
                surface.public_mount.is_none(),
                "no SSO must mount no public login routes"
            ),
            Err(error) => panic!("no SSO is a valid configuration, got error: {error}"),
        }
    }

    #[tokio::test]
    async fn sso_without_local_access_store_fails_closed() {
        let sso = SsoStartupConfig {
            providers: vec![Arc::new(StubProvider(
                OAuthProviderName::new("google").expect("provider name"),
            ))],
            base_url: "https://app.example".to_string(),
            allowed_email_domains: vec!["example.com".to_string()],
            session_epoch: None,
        };

        let result = build_webui_auth_surface(
            Some(sso),
            Some(ironclaw_reborn_composition::open_reborn_identity_resolver(
                &TenantId::new("sso-missing-access-tenant").expect("tenant"),
            )),
            TenantId::new("sso-missing-access-tenant").expect("tenant"),
            SecretString::from("operator-session-secret".to_string()),
            Arc::new(RejectingAuth),
            None,
        )
        .await;

        let error = match result {
            Ok(_) => panic!("configured SSO with no access validator must fail closed"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("access validator"),
            "startup error must name the missing access validator, got: {error}"
        );
    }

    #[tokio::test]
    async fn sso_with_local_trigger_access_bootstrap_builds_surface() {
        // SSO configured with a local-trigger-access bootstrap: the surface
        // must attach the per-login seeder to the admission adapter and mount
        // the public login routes — proving the bootstrap config is wired
        // through, not silently dropped.
        let tmp = tempfile::tempdir().expect("tempdir");
        let access_store_path = tmp.path().join("reborn-local-dev.db");
        let access_store =
            ironclaw_reborn_composition::open_local_trigger_access_store(&access_store_path)
                .await
                .expect("open local trigger access store");
        let sso = SsoStartupConfig {
            providers: vec![Arc::new(StubProvider(
                OAuthProviderName::new("google").expect("provider name"),
            ))],
            base_url: "https://app.example".to_string(),
            allowed_email_domains: vec!["example.com".to_string()],
            session_epoch: None,
        };

        let surface = build_webui_auth_surface(
            Some(sso),
            Some(ironclaw_reborn_composition::open_reborn_identity_resolver(
                &TenantId::new("sso-bootstrap-tenant").expect("tenant"),
            )),
            TenantId::new("sso-bootstrap-tenant").expect("tenant"),
            SecretString::from("operator-session-secret".to_string()),
            Arc::new(RejectingAuth),
            Some(LocalTriggerAccessBootstrapConfig {
                store: access_store,
                tenant_id: TenantId::new("sso-bootstrap-tenant").expect("tenant"),
                agent_id: AgentId::new("sso-bootstrap-agent").expect("agent"),
                project_id: Some(ProjectId::new("sso-bootstrap-project").expect("project")),
            }),
        )
        .await
        .expect("SSO surface with a bootstrap config must build");

        assert!(
            surface.public_mount.is_some(),
            "configured SSO must mount the public login routes"
        );
    }

    #[tokio::test]
    async fn sso_surface_rejects_signed_session_after_local_access_revoked() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let access_store_path = tmp.path().join("reborn-local-dev.db");
        let access_store =
            ironclaw_reborn_composition::open_local_trigger_access_store(&access_store_path)
                .await
                .expect("open local trigger access store");
        let tenant_id = TenantId::new("sso-validator-tenant").expect("tenant");
        let agent_id = AgentId::new("sso-validator-agent").expect("agent");
        let project_id = ProjectId::new("sso-validator-project").expect("project");
        let sso = SsoStartupConfig {
            providers: vec![Arc::new(StubProvider(
                OAuthProviderName::new("google").expect("provider name"),
            ))],
            base_url: "https://app.example".to_string(),
            allowed_email_domains: vec!["example.com".to_string()],
            session_epoch: None,
        };

        let surface = build_webui_auth_surface(
            Some(sso),
            Some(ironclaw_reborn_composition::open_reborn_identity_resolver(
                &tenant_id,
            )),
            tenant_id.clone(),
            SecretString::from("operator-session-secret".to_string()),
            Arc::new(RejectingAuth),
            Some(LocalTriggerAccessBootstrapConfig {
                store: access_store.clone(),
                tenant_id: tenant_id.clone(),
                agent_id: agent_id.clone(),
                project_id: Some(project_id.clone()),
            }),
        )
        .await
        .expect("SSO surface with a bootstrap config must build");

        let token = mint_session_token(surface.public_mount.as_ref().expect("public mount")).await;
        let initial_auth = surface
            .authenticator
            .authenticate(&token)
            .await
            .expect("freshly minted signed session should authenticate");
        let signed_session_user = initial_auth.user_id;
        assert!(
            !signed_session_user.as_str().is_empty(),
            "test profile should resolve to an admitted SSO user",
        );

        access_store
            .reconcile_local_access(
                ironclaw_reborn_composition::LocalTriggerAccessReconciliation {
                    tenant_id: &tenant_id,
                    user_ids: &[],
                    agent_id: Some(&agent_id),
                    project_id: Some(&project_id),
                    role: ironclaw_reborn_composition::LocalTriggerAccessRole::Owner,
                    source:
                        ironclaw_reborn_composition::LocalTriggerAccessSource::LocalDevSsoBootstrap,
                },
            )
            .await
            .expect("revoke local access");

        assert!(
            surface.authenticator.authenticate(&token).await.is_none(),
            "the same signed bearer must be rejected after local access is reconciled away",
        );
    }
}

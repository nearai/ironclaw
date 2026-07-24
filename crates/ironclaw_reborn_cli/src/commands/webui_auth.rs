//! WebChat v2 auth-surface assembly for `ironclaw serve`.
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
use ironclaw_reborn_composition::RebornIdentityResolver;
use ironclaw_reborn_composition::host_api::TenantId;
use ironclaw_webui::{
    CompositeAuthenticator, PublicRouteMount, SessionAuthenticator, SignedSessionLoginConfig,
    WebuiAuthenticator, build_signed_session_login, empty_webui_v2_auth_providers_mount,
    signed_session_store,
};
use secrecy::SecretString;

use crate::commands::serve_sso::SsoStartupConfig;
use crate::commands::user_directory::WebuiUserDirectory;

/// The composed WebChat v2 auth surface: the authenticator the protected
/// routes verify bearers with, plus the optional public login-route mount
/// (present only when SSO providers are configured).
pub(crate) struct WebuiAuthSurface {
    pub(crate) authenticator: Arc<dyn WebuiAuthenticator>,
    pub(crate) public_mount: Option<PublicRouteMount>,
}

/// Build the auth surface from resolved startup config.
///
/// With no SSO provider configured (`sso_startup` is `None`), the listener
/// keeps its env-bearer authenticator, also validates signed session tokens
/// minted by the admin API, and mounts only the inert auth surface so
/// `/auth/providers` can return an empty provider list.
/// With providers configured, this layers the fail-closed email-domain
/// admission adapter on top of the runtime-owned canonical Reborn identity
/// resolver and hands the result to the ingress signed-session builder.
///
/// `identity_resolver` is the resolver the runtime opened on its own
/// substrate handle. It is `None` only when the runtime carries no
/// local-runtime substrate; with SSO configured that is unrecoverable, so
/// this fails closed rather than minting users against a missing store.
///
/// An admitted SSO user's trigger-fire access is no longer seeded here: the
/// canonical `StoredUser` the resolver persists on login IS the membership the
/// runtime's fire-time checker reads (arch-simplification §4.4).
pub(crate) async fn build_webui_auth_surface(
    sso_startup: Option<SsoStartupConfig>,
    identity_resolver: Option<Arc<dyn RebornIdentityResolver>>,
    tenant_id: TenantId,
    session_signing_secret: SecretString,
    env_authenticator: Arc<dyn WebuiAuthenticator>,
) -> anyhow::Result<WebuiAuthSurface> {
    let Some(sso) = sso_startup else {
        // No SSO providers: no public login routes. But the
        // serve layer *always* wires the admin-API token minter, which mints
        // signed **session** tokens (the user-create bearer). Those validate
        // only through a `SessionAuthenticator` over the same signed store —
        // absent it, an admin-created user's API token would 401 on every
        // request (regression caught by `tests/e2e/scenarios/test_admin_api.py`).
        // Compose the env-bearer (operator) authenticator with a session
        // authenticator over that store so minted tokens work without SSO;
        // operator capabilities still follow the env token only, so the session
        // bearer stays non-operator.
        let session_authenticator: Arc<dyn WebuiAuthenticator> = Arc::new(
            SessionAuthenticator::new(signed_session_store(&session_signing_secret, &tenant_id)),
        );
        let authenticator: Arc<dyn WebuiAuthenticator> = Arc::new(CompositeAuthenticator::new(
            session_authenticator,
            env_authenticator,
        ));
        let public_mount = empty_webui_v2_auth_providers_mount();
        return Ok(WebuiAuthSurface {
            authenticator,
            public_mount: Some(public_mount),
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

    let user_directory = WebuiUserDirectory::new(
        identity_resolver,
        tenant_id.clone(),
        sso.allowed_email_domains,
    );

    let wiring = build_signed_session_login(SignedSessionLoginConfig {
        tenant_id,
        user_directory: Arc::new(user_directory),
        operator_secret: session_signing_secret,
        base_url: sso.base_url,
        providers: sso.providers,
        env_authenticator,
    })
    .expect("non-empty providers always produce login wiring"); // safety: sso_startup_config_from_env returns None when providers is empty, so this Some(sso) arm always has a non-empty provider list

    eprintln!(
        "ironclaw: WebChat v2 SSO login mounted — \
         see GET /auth/providers for the enabled set"
    );
    Ok(WebuiAuthSurface {
        authenticator: wiring.authenticator,
        public_mount: Some(wiring.mount),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use ironclaw_webui::WebuiAuthentication;
    use ironclaw_webui::{OAuthError, OAuthProvider, OAuthProviderName, OAuthUserProfile};

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
            _callback_url: &str,
            _state: &str,
            _code_challenge: &str,
        ) -> String {
            "https://provider.example/authorize".to_string()
        }

        async fn exchange_code(
            &self,
            _code: &str,
            _callback_url: &str,
            _code_verifier: &str,
        ) -> Result<OAuthUserProfile, OAuthError> {
            unreachable!("provider exchange is not exercised by auth-surface wiring tests")
        }
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
        };

        let result = build_webui_auth_surface(
            Some(sso),
            None, // no resolver — the fail-closed branch under test
            TenantId::new("tenant-host").expect("tenant"),
            SecretString::from("session-signing-secret".to_string()),
            Arc::new(RejectingAuth),
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
    async fn no_sso_composes_env_and_session_auth_and_mounts_empty_provider_route() {
        // With no SSO configured the surface still needs env-bearer access
        // plus signed-session bearer access for admin-created users. It also
        // mounts an inert public auth surface for provider discovery. The
        // absent-resolver check must not fire on this path.
        let result = build_webui_auth_surface(
            None,
            None,
            TenantId::new("tenant-host").expect("tenant"),
            SecretString::from("session-signing-secret".to_string()),
            Arc::new(RejectingAuth),
        )
        .await;

        match result {
            Ok(surface) => assert!(
                surface.public_mount.as_ref().is_some_and(|mount| {
                    mount.descriptors.len() == 1
                        && mount.descriptors[0].route_pattern().as_str() == "/auth/providers"
                }),
                "no SSO must mount only /auth/providers with an empty list"
            ),
            Err(error) => panic!("no SSO is a valid configuration, got error: {error}"),
        }
    }

    #[tokio::test]
    async fn sso_configured_builds_surface_with_public_login_mount() {
        // SSO configured: the surface layers the admission adapter over the
        // runtime identity resolver and mounts the public login routes. Trigger
        // access is no longer seeded here (arch-simplification §4.4) — the
        // resolver's own `StoredUser` is the membership the fire-time checker
        // reads.
        let sso = SsoStartupConfig {
            providers: vec![Arc::new(StubProvider(
                OAuthProviderName::new("google").expect("provider name"),
            ))],
            base_url: "https://app.example".to_string(),
            allowed_email_domains: vec!["example.com".to_string()],
        };

        let surface = build_webui_auth_surface(
            Some(sso),
            Some(ironclaw_reborn_composition::open_reborn_identity_resolver(
                &TenantId::new("sso-bootstrap-tenant").expect("tenant"),
            )),
            TenantId::new("sso-bootstrap-tenant").expect("tenant"),
            SecretString::from("operator-session-secret".to_string()),
            Arc::new(RejectingAuth),
        )
        .await
        .expect("SSO surface must build");

        assert!(
            surface.public_mount.is_some(),
            "configured SSO must mount the public login routes"
        );
    }
}

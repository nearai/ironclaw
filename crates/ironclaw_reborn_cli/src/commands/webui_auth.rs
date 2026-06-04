//! WebChat v2 auth-surface assembly for `ironclaw-reborn serve`.
//!
//! Owns the one place that turns host config into the pair the listener
//! needs: the `WebuiAuthenticator` the protected v2 routes use, plus the
//! optional public login-route mount. `serve.rs` only wires host config
//! and calls [`build_webui_auth_surface`]; it does not itself open the
//! user store, run the signed-session builder, or know the `Option`/
//! provider invariants — those live here, next to the admission adapter
//! ([`crate::commands::user_directory`]) and the startup config
//! ([`crate::commands::serve_sso`]).

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use ironclaw_reborn_composition::host_api::TenantId;
use ironclaw_reborn_composition::{
    PublicRouteMount, WebuiAuthenticator, open_reborn_identity_resolver,
};
use ironclaw_reborn_webui_ingress::{SignedSessionLoginConfig, build_signed_session_login};
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
/// With no SSO provider configured (`sso_startup` is `None`), the
/// listener keeps its plain env-bearer authenticator and mounts no public
/// routes. With providers configured, this opens the canonical Reborn
/// identity resolver on the substrate DB, layers the fail-closed
/// email-domain admission adapter on top, and hands the result to the
/// ingress signed-session builder.
pub(crate) async fn build_webui_auth_surface(
    sso_startup: Option<SsoStartupConfig>,
    user_store_path: &Path,
    tenant_id: TenantId,
    session_signing_secret: SecretString,
    env_authenticator: Arc<dyn WebuiAuthenticator>,
) -> anyhow::Result<WebuiAuthSurface> {
    let Some(sso) = sso_startup else {
        return Ok(WebuiAuthSurface {
            authenticator: env_authenticator,
            public_mount: None,
        });
    };

    // Open the canonical Reborn identity resolver through the composition
    // facade (which keeps the libSQL substrate handle private). The host
    // `WebuiUserDirectory` adapter layers the fail-closed email-domain
    // admission allowlist on top before any user is created.
    let identity_resolver = open_reborn_identity_resolver(user_store_path)
        .await
        .context("failed to initialize the Reborn identity resolver")?;

    let wiring = build_signed_session_login(SignedSessionLoginConfig {
        tenant_id: tenant_id.clone(),
        user_directory: Arc::new(WebuiUserDirectory::new(
            identity_resolver,
            tenant_id,
            sso.allowed_email_domains,
        )),
        operator_secret: session_signing_secret,
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

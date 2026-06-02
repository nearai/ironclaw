//! WebChat v2 SSO startup config for `ironclaw-reborn serve`.
//!
//! Compiled under the `webui-v2-beta` feature (the same feature that
//! compiles the `serve` command). This module owns the host config side
//! of SSO login startup policy: reading the operator's env vars into a
//! provider list, resolving the public base URL, and refusing cleartext
//! OAuth on a public interface. The auth/session model itself — the
//! signed-token session store, the composite authenticator, the
//! single-operator user directory, and the route wiring — lives in
//! `ironclaw_reborn_webui_ingress` (see
//! [`ironclaw_reborn_webui_ingress::build_signed_session_login`]), which
//! is where this crate's guardrails place `WebuiAuthenticator` /
//! `SessionStore` implementations. `serve.rs` calls
//! [`sso_startup_config_from_env`] and, when it returns `Some`, hands
//! the result plus the operator identity/secret to that builder.

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use ironclaw_reborn_webui_ingress::{
    GitHubOAuthConfig, GitHubProvider, GoogleOAuthConfig, GoogleProvider, OAuthProvider,
};
use secrecy::SecretString;

/// Resolved SSO startup config: the providers to mount plus the public
/// base URL their callback URLs are built from. Constructed by
/// [`sso_startup_config_from_env`]; consumed by `serve.rs`, which adds
/// the operator identity/secret and calls the ingress builder.
pub(crate) struct SsoStartupConfig {
    pub(crate) providers: Vec<Arc<dyn OAuthProvider>>,
    pub(crate) base_url: String,
}

/// Resolve the SSO startup config from environment, applying all
/// startup-time SSO policy in one place: provider discovery, base-URL
/// resolution, and the cleartext-OAuth guard.
///
/// Returns `Ok(None)` when no provider is configured (the listener then
/// keeps its plain env-bearer auth and mounts no public login routes),
/// `Err` on misconfiguration (a provider opted in without its secret, or
/// an http:// base URL on a non-loopback bind).
pub(crate) fn sso_startup_config_from_env(
    listen_addr: SocketAddr,
) -> anyhow::Result<Option<SsoStartupConfig>> {
    let providers = oauth_providers_from_env()?;
    if providers.is_empty() {
        return Ok(None);
    }

    // Reborn-scoped base URL only — deliberately NOT falling back to the
    // v1 gateway's `OAUTH_BASE_URL`, so a legacy v1 setting can never
    // silently rewrite Reborn WebChat callback URLs. Absent the Reborn
    // var, use the bound listener address.
    let base_url = env::var("IRONCLAW_REBORN_WEBUI_BASE_URL")
        .ok()
        .filter(|raw| !raw.trim().is_empty())
        .unwrap_or_else(|| format!("http://{listen_addr}"));

    // Refuse cleartext OAuth on a public interface: http:// redirect URIs
    // leak authorization codes in transit (and Google/GitHub reject them
    // for production apps). Loopback http:// stays allowed for local dev.
    if base_url.starts_with("http://") && !listen_addr.ip().is_loopback() {
        anyhow::bail!(
            "WebChat v2 SSO base URL `{base_url}` uses http:// on a non-loopback interface, \
             which would transmit OAuth authorization codes in cleartext. Set \
             IRONCLAW_REBORN_WEBUI_BASE_URL to an https:// URL."
        );
    }

    Ok(Some(SsoStartupConfig {
        providers,
        base_url,
    }))
}

/// Collect the configured OAuth providers from env. A provider is
/// opted in by setting its `IRONCLAW_REBORN_WEBUI_*_CLIENT_ID`. The
/// matching `*_CLIENT_SECRET` is then REQUIRED: opting a provider in
/// without its secret is a misconfiguration, not an optional feature —
/// registering it would surface a login button whose code exchange
/// always fails at the provider — so it fails startup loudly rather than
/// silently skipping with a buried log line.
///
/// These WebChat-login vars live in the `IRONCLAW_REBORN_WEBUI_*`
/// namespace — the same one as `IRONCLAW_REBORN_WEBUI_TOKEN` /
/// `IRONCLAW_REBORN_WEBUI_USER_ID` — deliberately separate from the
/// bare `GOOGLE_CLIENT_ID` / `IRONCLAW_REBORN_GOOGLE_*` vars that the
/// product-auth credential-connection flow reads. Sharing the bare
/// names would couple two unrelated surfaces: setting `GOOGLE_CLIENT_ID`
/// just to enable login would also activate the product-auth Google
/// resolver, which hard-errors when its required redirect URI is absent
/// and would take down every `ironclaw-reborn` command. The distinct
/// namespace keeps SSO login and product-auth independently
/// configurable. (The browser-user *login* client and the product
/// *credential* client are usually distinct OAuth clients anyway —
/// different registered redirect URIs.)
fn oauth_providers_from_env() -> anyhow::Result<Vec<Arc<dyn OAuthProvider>>> {
    let mut providers: Vec<Arc<dyn OAuthProvider>> = Vec::new();

    if let Some(client_id) = non_empty_env("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID") {
        let client_secret = non_empty_env("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET")
            .ok_or_else(|| {
                anyhow!(
                    "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID is set but \
                     IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET is missing"
                )
            })?;
        let allowed_hd = non_empty_env("IRONCLAW_REBORN_WEBUI_GOOGLE_ALLOWED_HD");
        // Every successful login is mapped to the single operator (see
        // `FixedUserDirectory` in the ingress crate), so with no
        // hosted-domain restriction ANY Google account that completes the
        // flow becomes the operator. Warn loudly when it is unset.
        if allowed_hd.is_none() {
            tracing::warn!(
                target = "ironclaw::reborn::cli::serve",
                "IRONCLAW_REBORN_WEBUI_GOOGLE_ALLOWED_HD is unset — any Google account that \
                 completes login is mapped to the operator identity; set it to restrict \
                 Google login to one Workspace domain",
            );
        }
        let provider = GoogleProvider::new(GoogleOAuthConfig {
            client_id,
            client_secret: SecretString::from(client_secret),
            allowed_hd,
        })
        .context("failed to build Google OAuth provider")?;
        providers.push(Arc::new(provider));
    }

    if let Some(client_id) = non_empty_env("IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID") {
        let client_secret = non_empty_env("IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_SECRET")
            .ok_or_else(|| {
                anyhow!(
                    "IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID is set but \
                     IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_SECRET is missing"
                )
            })?;
        let provider = GitHubProvider::new(GitHubOAuthConfig {
            client_id,
            client_secret: SecretString::from(client_secret),
        })
        .context("failed to build GitHub OAuth provider")?;
        providers.push(Arc::new(provider));
    }

    Ok(providers)
}

/// Read an env var, returning `None` when it is unset or blank.
fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|raw| !raw.trim().is_empty())
}

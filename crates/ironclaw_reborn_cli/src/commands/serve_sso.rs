//! WebChat v2 SSO provider discovery for `ironclaw-reborn serve`.
//!
//! Compiled under the `webui-v2-beta` feature (the same feature that
//! compiles the `serve` command). This module owns only the host
//! config side of SSO login: reading the operator's env vars into a
//! list of [`OAuthProvider`]s. The auth/session model itself — the
//! signed-token session store, the composite authenticator, the
//! single-operator user directory, and the route wiring — lives in
//! `ironclaw_reborn_webui_ingress` (see
//! [`ironclaw_reborn_webui_ingress::build_signed_session_login`]), which
//! is where this crate's guardrails place `WebuiAuthenticator` /
//! `SessionStore` implementations. `serve.rs` reads the provider list
//! from here plus the operator identity / secret / base URL, and hands
//! them to that builder.

use std::env;
use std::sync::Arc;

use anyhow::Context;
use ironclaw_reborn_webui_ingress::{
    GitHubOAuthConfig, GitHubProvider, GoogleOAuthConfig, GoogleProvider, OAuthProvider,
};
use secrecy::SecretString;

/// Collect the configured OAuth providers from env. A provider is
/// opted in by setting its `IRONCLAW_REBORN_WEBUI_*_CLIENT_ID`. The
/// matching `*_CLIENT_SECRET` is required: if the client id is set but
/// the secret is missing, the provider is skipped with a `warn!` rather
/// than registered with an empty secret (which would surface a login
/// button whose code exchange always fails at the provider).
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
pub(crate) fn oauth_providers_from_env() -> anyhow::Result<Vec<Arc<dyn OAuthProvider>>> {
    let mut providers: Vec<Arc<dyn OAuthProvider>> = Vec::new();

    if let Some(client_id) = non_empty_env("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID") {
        match non_empty_env("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET") {
            Some(client_secret) => {
                let allowed_hd = non_empty_env("IRONCLAW_REBORN_WEBUI_GOOGLE_ALLOWED_HD");
                // Every successful login is mapped to the single operator
                // (see `FixedUserDirectory` in the ingress crate), so with
                // no hosted-domain restriction ANY Google account that
                // completes the flow becomes the operator. Warn loudly
                // when it is unset.
                if allowed_hd.is_none() {
                    tracing::warn!(
                        target = "ironclaw::reborn::cli::serve",
                        "IRONCLAW_REBORN_WEBUI_GOOGLE_ALLOWED_HD is unset — any Google account \
                         that completes login is mapped to the operator identity; set it to \
                         restrict Google login to one Workspace domain",
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
            None => tracing::warn!(
                target = "ironclaw::reborn::cli::serve",
                "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID is set but \
                 IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET is missing; skipping the Google \
                 login provider (code exchange would always fail without the secret)",
            ),
        }
    }

    if let Some(client_id) = non_empty_env("IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID") {
        match non_empty_env("IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_SECRET") {
            Some(client_secret) => {
                let provider = GitHubProvider::new(GitHubOAuthConfig {
                    client_id,
                    client_secret: SecretString::from(client_secret),
                })
                .context("failed to build GitHub OAuth provider")?;
                providers.push(Arc::new(provider));
            }
            None => tracing::warn!(
                target = "ironclaw::reborn::cli::serve",
                "IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID is set but \
                 IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_SECRET is missing; skipping the GitHub \
                 login provider (code exchange would always fail without the secret)",
            ),
        }
    }

    Ok(providers)
}

/// Read an env var, returning `None` when it is unset or blank.
fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|raw| !raw.trim().is_empty())
}

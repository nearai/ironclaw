//! Built-in OAuth provider metadata and override behavior.

/// Built-in OAuth credentials shipped with the binary for select providers.
pub struct OAuthCredentials {
    pub client_id: &'static str,
    pub client_secret: &'static str,
}

/// Google OAuth "Desktop App" credentials, shared across all Google tools.
///
/// **Why these are embedded in source.** Google explicitly classifies the
/// `client_secret` for the *Installed App / Desktop App* OAuth flow as
/// non-confidential — see
/// <https://developers.google.com/identity/protocols/oauth2/native-app>:
/// "The process of embedding a client secret in a desktop application
/// isn't straightforward... the client_secret is not actually treated as
/// a secret." The security model for the Desktop App flow relies on PKCE
/// and the user's consent screen, not on the secrecy of `client_secret`.
///
/// CI / hosted builds that want to use a different OAuth client (e.g. a
/// platform client owned by a vendor that DOES have a confidential secret
/// behind a proxy) can override both values at *build time* via the
/// `IRONCLAW_GOOGLE_CLIENT_ID` and `IRONCLAW_GOOGLE_CLIENT_SECRET` env
/// vars consumed by `option_env!`. When that override is present and the
/// hosted-OAuth proxy is configured, `hosted_proxy_client_secret` below
/// strips the baked-in secret so the proxy injects the real one.
///
/// Tracking "move defaults to runtime-only injection without source
/// fallback" as a separate hardening item — out of scope for this PR.
const GOOGLE_CLIENT_ID: &str = match option_env!("IRONCLAW_GOOGLE_CLIENT_ID") {
    Some(v) => v,
    None => "564604149681-efo25d43rs85v0tibdepsmdv5dsrhhr0.apps.googleusercontent.com",
};
const GOOGLE_CLIENT_SECRET: &str = match option_env!("IRONCLAW_GOOGLE_CLIENT_SECRET") {
    Some(v) => v,
    None => "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",
};

/// Returns built-in OAuth credentials for a provider, keyed by secret_name.
///
/// The secret_name comes from the tool's capabilities.json `auth.secret_name` field.
/// Returns `None` if no built-in credentials are configured for that provider.
pub fn builtin_credentials(secret_name: &str) -> Option<OAuthCredentials> {
    match secret_name {
        "google_oauth_token" => Some(OAuthCredentials {
            client_id: GOOGLE_CLIENT_ID,
            client_secret: GOOGLE_CLIENT_SECRET,
        }),
        _ => None,
    }
}

/// Returns the compile-time override env var name, if this provider supports one.
pub fn builtin_client_id_override_env(secret_name: &str) -> Option<&'static str> {
    match secret_name {
        "google_oauth_token" => Some("IRONCLAW_GOOGLE_CLIENT_ID"),
        _ => None,
    }
}

/// Suppress the baked-in desktop OAuth client secret when a hosted proxy is configured.
///
/// In hosted deployments, IronClaw may resolve the platform Google client ID from
/// environment variables while still falling back to the baked-in desktop secret.
/// That client_id/client_secret mismatch breaks Google token exchange and refresh.
///
/// When the proxy is configured, the platform will inject the correct server-side
/// secret for matching platform credentials, so the baked-in secret must be omitted.
pub fn hosted_proxy_client_secret(
    client_secret: &Option<String>,
    builtin: Option<&OAuthCredentials>,
    exchange_proxy_configured: bool,
) -> Option<String> {
    if !exchange_proxy_configured {
        return client_secret.clone();
    }

    let builtin_secret = builtin.map(|credentials| credentials.client_secret);
    match (client_secret, builtin_secret) {
        (Some(resolved), Some(baked_in)) if resolved == baked_in => None,
        _ => client_secret.clone(),
    }
}

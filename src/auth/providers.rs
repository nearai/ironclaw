//! Built-in OAuth provider metadata and override behavior.

/// Built-in OAuth credentials shipped with the binary for select providers.
pub struct OAuthCredentials {
    pub client_id: &'static str,
    pub client_secret: &'static str,
}

/// Google OAuth "Desktop App" credentials, shared across all Google tools.
/// Compile-time env vars override the hardcoded defaults below.
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

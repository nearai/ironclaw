//! Built-in OAuth credentials for common providers.
//!
//! Many CLI tools (gcloud, rclone, gdrive) ship with default OAuth credentials
//! so users don't need to register their own OAuth app. Google explicitly
//! documents that client_secret for "Desktop App" / "Installed App" types
//! is NOT actually secret.
//!
//! # Setting credentials
//!
//! Option A (compile-time): Set env vars before building:
//!   IRONCLAW_GOOGLE_CLIENT_ID=xxx.apps.googleusercontent.com
//!   IRONCLAW_GOOGLE_CLIENT_SECRET=xxx
//!   cargo build --release
//!
//! Option B (hardcode): Replace the `option_env!` calls below with `Some("value")`.
//!
//! Option C (runtime override): Users can always override with runtime env vars
//! (GOOGLE_OAUTH_CLIENT_ID / GOOGLE_OAUTH_CLIENT_SECRET), which take priority
//! over built-in defaults.

pub struct OAuthCredentials {
    pub client_id: &'static str,
    pub client_secret: &'static str,
}

/// Google OAuth "Desktop App" credentials, shared across all Google tools.
const GOOGLE_CLIENT_ID: Option<&str> = option_env!("IRONCLAW_GOOGLE_CLIENT_ID");
const GOOGLE_CLIENT_SECRET: Option<&str> = option_env!("IRONCLAW_GOOGLE_CLIENT_SECRET");

/// Returns built-in OAuth credentials for a provider, keyed by secret_name.
///
/// The secret_name comes from the tool's capabilities.json `auth.secret_name` field.
/// Returns `None` if no built-in credentials are configured for that provider.
pub fn builtin_credentials(secret_name: &str) -> Option<OAuthCredentials> {
    match secret_name {
        "google_oauth_token" => Some(OAuthCredentials {
            client_id: GOOGLE_CLIENT_ID?,
            client_secret: GOOGLE_CLIENT_SECRET?,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::oauth_defaults::builtin_credentials;

    #[test]
    fn test_unknown_provider_returns_none() {
        assert!(builtin_credentials("unknown_token").is_none());
    }

    #[test]
    fn test_google_returns_based_on_compile_env() {
        // This test's result depends on whether IRONCLAW_GOOGLE_CLIENT_ID
        // was set at compile time. We just verify it doesn't panic.
        let _ = builtin_credentials("google_oauth_token");
    }
}

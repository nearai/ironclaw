pub mod calendar;
pub mod client;
pub mod credential;
pub mod gmail;
pub mod network;
pub mod oauth_provider;

use std::sync::Arc;

use ironclaw_secrets::SecretStore;

use crate::{EnvConfig, NativeExtensionError, RegistrationOutput};

pub mod scopes {
    pub const CALENDAR_READONLY: &str = "https://www.googleapis.com/auth/calendar.readonly";
    pub const CALENDAR_EVENTS: &str = "https://www.googleapis.com/auth/calendar.events";
    pub const GMAIL_READONLY: &str = "https://www.googleapis.com/auth/gmail.readonly";
    pub const GMAIL_SEND: &str = "https://www.googleapis.com/auth/gmail.send";
    pub const GMAIL_MODIFY: &str = "https://www.googleapis.com/auth/gmail.modify";
}

pub fn register(
    env: &EnvConfig,
    secrets: Arc<dyn SecretStore>,
    output: &mut RegistrationOutput,
) -> Result<(), NativeExtensionError> {
    let resolver = Arc::new(credential::GoogleCredentialResolver::new(secrets));
    if let Some(provider) = oauth_provider::GoogleProvider::from_config(env)? {
        output
            .network_policies
            .push(network::google_api_network_policy());
        // Calendar and Gmail handlers issue HTTP through the per-invocation
        // host `runtime_http_egress` (HostHttpEgressService), not a transport
        // built here, so no standalone HTTP client is constructed at
        // registration. Both packages share the one credential resolver and
        // Google `OAuthProvider` — they resolve the same `google_oauth_token`.
        let oauth_provider: Arc<dyn ironclaw_oauth::OAuthProvider> = provider.clone();
        calendar::register_calendar(resolver.clone(), oauth_provider.clone(), output)?;
        gmail::register_gmail(resolver, oauth_provider, output)?;
        output.oauth_providers.push(provider);
    }
    Ok(())
}

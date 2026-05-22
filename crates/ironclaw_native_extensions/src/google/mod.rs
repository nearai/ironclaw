pub mod client;
pub mod credential;
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
    let _credential_resolver = credential::GoogleCredentialResolver::new(secrets);
    if let Some(provider) = oauth_provider::GoogleProvider::from_config(env)? {
        output.oauth_providers.push(provider);
        output
            .network_policies
            .push(network::google_api_network_policy());
    }
    Ok(())
}

//! Host-bundled native extension registration for Reborn.
//!
//! This crate owns provider-specific native extension scaffolding. It does not
//! compose registries into a running host; composition consumes
//! [`RegistrationOutput`] and decides where each item is installed.
#![warn(unreachable_pub)]

use std::sync::Arc;

use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::FirstPartyCapabilityHandler;
use ironclaw_oauth::OAuthProvider;
use ironclaw_secrets::SecretStore;
use thiserror::Error;

#[cfg(feature = "google")]
pub mod google;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvConfig {
    pub oauth_broker_active: bool,
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    pub google_allowed_hd: Option<String>,
}

impl EnvConfig {
    pub fn from_env() -> Self {
        Self {
            oauth_broker_active: std::env::var("IRONCLAW_OAUTH_EXCHANGE_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .is_some(),
            google_client_id: env_optional("GOOGLE_CLIENT_ID"),
            google_client_secret: env_optional("GOOGLE_CLIENT_SECRET"),
            google_allowed_hd: env_optional("GOOGLE_ALLOWED_HD"),
        }
    }
}

#[derive(Default)]
pub struct RegistrationOutput {
    pub packages: Vec<ExtensionPackage>,
    pub handlers: Vec<(CapabilityId, Arc<dyn FirstPartyCapabilityHandler>)>,
    pub oauth_providers: Vec<Arc<dyn OAuthProvider>>,
    pub network_policies: Vec<ironclaw_host_api::NetworkPolicy>,
}

#[derive(Debug, Error)]
pub enum NativeExtensionError {
    #[error(transparent)]
    OAuth(#[from] ironclaw_oauth::OAuthError),
    #[error(transparent)]
    HostApi(#[from] ironclaw_host_api::HostApiError),
    #[error(transparent)]
    Secret(#[from] ironclaw_secrets::SecretStoreError),
    #[error(transparent)]
    Network(#[from] ironclaw_network::NetworkHttpError),
}

pub fn register_all(
    env: &EnvConfig,
    secrets: Arc<dyn SecretStore>,
) -> Result<RegistrationOutput, NativeExtensionError> {
    #[cfg(feature = "google")]
    {
        let mut output = RegistrationOutput::default();
        google::register(env, secrets, &mut output)?;
        Ok(output)
    }
    #[cfg(not(feature = "google"))]
    {
        let _ = (env, secrets);
        Ok(RegistrationOutput::default())
    }
}

fn env_optional(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

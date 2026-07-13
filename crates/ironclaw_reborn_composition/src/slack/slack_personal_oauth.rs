//! Slack personal (user-token) OAuth client-credential wiring over the
//! generic auth engine.
//!
//! The per-vendor provider spec, gate provider, and post-exchange identity
//! hook are gone: Slack's OAuth flow executes the manifest's `[auth.slack]`
//! recipe through the host auth engine like every other vendor, and the
//! post-exchange identity binding rides the generic channel-identity hook
//! (`extension_host::channel_identity`). What remains here is Slack-owned
//! wiring registered as **data**: the deployment client-credential lookups
//! (the operator saves the Slack OAuth client id/secret through the setup
//! service after startup).

use std::fmt;
use std::sync::Arc;

use ironclaw_auth::AuthProductError;
use secrecy::SecretString;

use crate::product_auth::credentials::product_auth_providers::{
    CompositionClientCredentials, DynamicClientCredentialLookup,
};
use crate::slack::slack_setup::SlackPersonalSetupServiceSlot;

/// The unified Slack vendor id (matches the manifest's `[auth.slack]`).
pub(crate) const SLACK_VENDOR_ID: &str = "slack";

/// Register the Slack OAuth client-credential lookups: the recipe's
/// `client_credentials` handles resolve through the operator setup service at
/// request time (the operator may save them after startup).
pub(crate) fn register_slack_personal_client_credentials(
    credentials: &mut CompositionClientCredentials,
    recipes: &dyn ironclaw_auth::AuthRecipeResolver,
    slot: SlackPersonalSetupServiceSlot,
) {
    let Some(resolved) = recipes.recipe_for_vendor(SLACK_VENDOR_ID) else {
        tracing::warn!("no [auth.slack] recipe resolved; Slack OAuth client lookups not wired");
        return;
    };
    let ironclaw_host_api::VendorAuthRecipe::Oauth2Code(recipe) = &resolved.recipe else {
        return;
    };
    let Some(client_credentials) = &recipe.client_credentials else {
        return;
    };
    credentials.register_dynamic(
        client_credentials.client_id_handle.as_str(),
        Arc::new(SlotClientCredentialLookup {
            slot: slot.clone(),
            secret: false,
        }),
    );
    if let Some(secret_handle) = &client_credentials.client_secret_handle {
        credentials.register_dynamic(
            secret_handle.as_str(),
            Arc::new(SlotClientCredentialLookup { slot, secret: true }),
        );
    }
}

struct SlotClientCredentialLookup {
    slot: SlackPersonalSetupServiceSlot,
    secret: bool,
}

impl fmt::Debug for SlotClientCredentialLookup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SlotClientCredentialLookup")
            .field("secret", &self.secret)
            .finish()
    }
}

#[async_trait::async_trait]
impl DynamicClientCredentialLookup for SlotClientCredentialLookup {
    async fn resolve(&self) -> Result<SecretString, AuthProductError> {
        let service = self.slot.get().ok_or_else(|| {
            tracing::warn!("Slack personal OAuth slot not yet filled (startup race)");
            AuthProductError::BackendUnavailable
        })?;
        let (client_id, client_secret) = service.oauth_credentials().await.map_err(|error| {
            tracing::warn!(error = %error, "Slack personal OAuth credentials not configured");
            AuthProductError::MalformedConfig
        })?;
        if self.secret {
            Ok(client_secret)
        } else {
            Ok(SecretString::from(client_id.as_str().to_string()))
        }
    }
}

//! Slack personal (user-token) OAuth wiring over the generic auth engine.
//!
//! The former per-vendor provider spec and gate provider are gone: Slack's
//! OAuth flow executes the manifest's `[auth.slack]` recipe through the host
//! auth engine like every other vendor. What remains here is Slack-owned
//! wiring registered as **data**:
//!
//! - the deployment client-credential lookups (the operator saves the Slack
//!   OAuth client id/secret through the setup service after startup), and
//! - the post-exchange identity hook that binds the exchanged `authed_user`
//!   identity to the authenticated Reborn user (identity-binding machinery,
//!   deleted with `composition/src/slack/**` in P6).

use std::fmt;
use std::sync::Arc;

use ironclaw_auth::{AuthProductError, AuthProductScope, OAuthProviderIdentity};
use secrecy::SecretString;

use crate::product_auth::api::auth::{
    OAuthProviderIdentityBindingRollback, OAuthProviderIdentityCheck,
    OAuthProviderIdentityCheckFuture,
};
use crate::product_auth::credentials::product_auth_providers::{
    CompositionClientCredentials, DynamicClientCredentialLookup,
};
use crate::provider_identity::{
    RebornUserIdentityBindingDeleteStore, RebornUserIdentityBindingError,
};
use crate::slack::slack_host_beta::SlackPersonalConnectionScopeResolver;
use crate::slack::slack_personal_binding::{
    SlackPersonalBindingPrincipal, SlackPersonalUserBinder, SlackPersonalUserBindingError,
    SlackPersonalUserBindingRequest,
};
use crate::slack::slack_serve::{SlackApiAppId, SlackEnterpriseId, SlackTeamId, SlackUserId};
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

/// The pieces the Slack post-exchange identity binding needs.
#[derive(Clone)]
pub struct SlackPersonalOAuthBindingConfig {
    pub(crate) binding_service: Arc<dyn SlackPersonalUserBinder>,
    pub(crate) connection_scope_resolver: Arc<dyn SlackPersonalConnectionScopeResolver>,
    /// Undoes an identity binding written by the callback identity hook when
    /// `complete_oauth_callback` fails afterwards; the binding is the
    /// user-visible "connected" signal, so it must not survive a completion
    /// failure that already deleted the token material.
    pub(crate) binding_rollback_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
}

impl SlackPersonalOAuthBindingConfig {
    pub(crate) fn new(
        binding_service: Arc<dyn SlackPersonalUserBinder>,
        connection_scope_resolver: Arc<dyn SlackPersonalConnectionScopeResolver>,
        binding_rollback_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    ) -> Self {
        Self {
            binding_service,
            connection_scope_resolver,
            binding_rollback_store,
        }
    }
}

impl fmt::Debug for SlackPersonalOAuthBindingConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SlackPersonalOAuthBindingConfig")
            .field("binding_service", &self.binding_service)
            .field(
                "connection_scope_resolver",
                &"Arc<dyn SlackPersonalConnectionScopeResolver>",
            )
            .field(
                "binding_rollback_store",
                &"Arc<dyn RebornUserIdentityBindingDeleteStore>",
            )
            .finish()
    }
}

/// Build the Slack vendor-identity hook factory the product-auth routes
/// register under the `slack` vendor id: it binds the exchanged `authed_user`
/// identity to the authenticated Reborn user and hands back a rollback.
pub(crate) fn slack_personal_identity_hook_factory(
    config: SlackPersonalOAuthBindingConfig,
) -> Arc<crate::product_auth::serve::VendorIdentityHookFactory> {
    Arc::new(move |callback_scope: &AuthProductScope| {
        let config = config.clone();
        let callback_scope = callback_scope.clone();
        Some(
            Box::new(move |provider_identity: Option<OAuthProviderIdentity>| {
                let config = config.clone();
                let callback_scope = callback_scope.clone();
                Box::pin(async move {
                    bind_slack_personal_oauth_identity_for_callback(
                        &config,
                        &callback_scope,
                        provider_identity.as_ref(),
                    )
                    .await
                    .map(Some)
                }) as OAuthProviderIdentityCheckFuture
            }) as OAuthProviderIdentityCheck,
        )
    })
}

async fn bind_slack_personal_oauth_identity_for_callback(
    config: &SlackPersonalOAuthBindingConfig,
    callback_scope: &AuthProductScope,
    provider_identity: Option<&OAuthProviderIdentity>,
) -> Result<OAuthProviderIdentityBindingRollback, AuthProductError> {
    let identity = provider_identity.ok_or(AuthProductError::MalformedCallback)?;
    let connection_scope = config
        .connection_scope_resolver
        .resolve_personal_connection_scope()
        .await
        .map_err(|error| {
            tracing::warn!(
                %error,
                "Slack personal OAuth binding connection scope resolver failed"
            );
            AuthProductError::BackendUnavailable
        })?
        .ok_or(AuthProductError::BackendUnavailable)?;
    let team_id = identity
        .team_id
        .as_ref()
        .ok_or(AuthProductError::MalformedCallback)?;
    if team_id.as_str() != connection_scope.team_id.as_str() {
        return Err(AuthProductError::MalformedCallback);
    }
    let api_app_id = identity
        .app_id
        .as_ref()
        .ok_or(AuthProductError::MalformedCallback)?;
    let enterprise_id = identity
        .enterprise_id
        .as_ref()
        .map(|value| SlackEnterpriseId::new(value.clone()));

    // Computed before the request takes ownership of the installation id so
    // the rollback can target exactly the binding this callback writes.
    let bound_provider_user_id = crate::provider_identity::installation_scoped_provider_user_id(
        &connection_scope.installation_id,
        identity.subject.as_str(),
    );
    config
        .binding_service
        .bind_personal_user(
            SlackPersonalBindingPrincipal {
                tenant_id: callback_scope.resource.tenant_id.clone(),
                user_id: callback_scope.resource.user_id.clone(),
            },
            SlackPersonalUserBindingRequest {
                installation_id: connection_scope.installation_id,
                slack_user_id: SlackUserId::new(identity.subject.as_str()),
                team_id: SlackTeamId::new(team_id.clone()),
                enterprise_id,
                api_app_id: SlackApiAppId::new(api_app_id.clone()),
            },
        )
        .await
        .map_err(slack_personal_user_binding_auth_error)?;

    let rollback_store = Arc::clone(&config.binding_rollback_store);
    let rollback_user_id = callback_scope.resource.user_id.clone();
    Ok(Box::pin(async move {
        // Passing the full provider_user_id as the prefix confines the delete
        // to the binding this exact callback wrote. Best-effort by contract:
        // a rollback failure only errs toward "shows connected without a
        // credential", which Disconnect already repairs.
        if let Err(error) = rollback_store
            .delete_user_identity_bindings_for_user(
                crate::slack::slack_channel_connection::SLACK_IDENTITY_PROVIDER,
                &rollback_user_id,
                Some(bound_provider_user_id.as_str()),
            )
            .await
        {
            tracing::warn!(
                %error,
                "failed to roll back Slack identity binding after OAuth completion failure"
            );
        }
    }))
}

fn slack_personal_user_binding_auth_error(
    error: SlackPersonalUserBindingError,
) -> AuthProductError {
    match error {
        SlackPersonalUserBindingError::UnknownInstallation { .. }
        | SlackPersonalUserBindingError::InstallationNotTenantScoped { .. }
        | SlackPersonalUserBindingError::SlackInstallationContextMismatch { .. }
        | SlackPersonalUserBindingError::InvalidSlackId { .. } => {
            AuthProductError::MalformedCallback
        }
        SlackPersonalUserBindingError::BindingStore(
            RebornUserIdentityBindingError::ProviderIdentityAlreadyBound,
        ) => AuthProductError::ProviderIdentityAlreadyConnected,
        SlackPersonalUserBindingError::BindingStore(_) => AuthProductError::BackendUnavailable,
    }
}

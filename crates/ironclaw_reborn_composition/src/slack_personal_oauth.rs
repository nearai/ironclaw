//! Slack personal (user-token) OAuth: provider spec + blocked-gate provider.
//!
//! This is one of the two production [`OAuthGateProvider`] implementors. The
//! shared challenge/turn-gate-reuse/PKCE-store/cleanup/expiry logic lives in
//! [`crate::product_auth::oauth::oauth_gate::OAuthGateFlowDriver`]; only the flow preparation differs
//! from Google — Slack resolves client credentials from its setup slot at
//! request time and emits `user_scope=` (its `scope=` is reserved for bot
//! tokens) via the generic authorization-URL builder.

use std::fmt;

use ironclaw_auth::{
    AuthFlowId, AuthProductError, AuthProductScope, CredentialAccountLabel,
    OAuthAuthorizationEndpoint, OAuthAuthorizeUrlRequest, OAuthCallbackState,
    OAuthCallbackStateKind, OAuthScopeParam, PkceVerifierSecret, ProviderScope,
    SLACK_PERSONAL_AUTHORIZATION_ENDPOINT, SLACK_PERSONAL_PROVIDER_ID,
    build_authorization_url_with_scope_param, opaque_state_hash, pkce_s256_challenge,
    pkce_verifier_hash,
};
use secrecy::SecretString;

use crate::product_auth::oauth::oauth_gate::{OAuthGateProvider, PreparedOAuthGateFlow};
use crate::product_auth::oauth::oauth_provider_client::{
    ExchangeScopePolicy, HostOAuthProviderSpec, TokenResponseShape,
};
use crate::slack_setup::SlackPersonalSetupServiceSlot;

/// Host OAuth provider spec for the Slack personal (user-token) provider.
///
/// `SlackAuthedUser` response shape so the exchanger extracts the user token
/// from `authed_user.access_token`. Slack does not return granted scopes in a
/// standard `scope` field, so the exchange falls back to the requested scopes.
pub(crate) fn slack_personal_provider_spec() -> HostOAuthProviderSpec {
    HostOAuthProviderSpec {
        provider_id: SLACK_PERSONAL_PROVIDER_ID,
        capability_id: "ironclaw_auth.slack_personal_oauth",
        token_endpoint: ironclaw_auth::SLACK_PERSONAL_TOKEN_ENDPOINT,
        secret_handle_prefix: "slack_personal",
        resource: None,
        exchange_scope_policy: ExchangeScopePolicy::FallbackToRequested,
        token_response_shape: TokenResponseShape::SlackAuthedUser,
    }
}

/// Slack personal (user-token) blocked-turn OAuth gate provider.
///
/// Holds the Slack setup slot; the shared [`crate::product_auth::oauth::oauth_gate::OAuthGateFlowDriver`]
/// owns everything else.
#[derive(Clone)]
pub(crate) struct SlackPersonalOAuthGateProvider {
    slot: SlackPersonalSetupServiceSlot,
}

impl SlackPersonalOAuthGateProvider {
    pub(crate) fn new(slot: SlackPersonalSetupServiceSlot) -> Self {
        Self { slot }
    }
}

#[async_trait::async_trait]
impl OAuthGateProvider for SlackPersonalOAuthGateProvider {
    fn provider_id(&self) -> &'static str {
        SLACK_PERSONAL_PROVIDER_ID
    }

    fn pkce_secret_handle_label(&self) -> &'static str {
        "slack-personal-oauth-gate-flow-pkce"
    }

    async fn prepare_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        scopes: Vec<ProviderScope>,
    ) -> Result<PreparedOAuthGateFlow, AuthProductError> {
        let service = self
            .slot
            .get()
            .ok_or(AuthProductError::BackendUnavailable)?;
        let (client_id, _client_secret) = service.oauth_credentials().await.map_err(|e| {
            tracing::warn!(error = %e, "Slack personal OAuth credentials not configured");
            AuthProductError::BackendUnavailable
        })?;
        let account_label = CredentialAccountLabel::new("slack_personal")?;
        let state = OAuthCallbackState::new(
            OAuthCallbackStateKind::SLACK_PERSONAL,
            flow_id,
            scope.clone(),
            account_label,
            scopes.clone(),
        )?
        .encode()?;
        let opaque_state_hash = opaque_state_hash(state.as_str())?;
        let pkce_verifier = SecretString::from(ironclaw_common::pkce::generate_code_verifier());
        let pkce_secret = PkceVerifierSecret::new(pkce_verifier.clone())?;
        let pkce_verifier_hash = pkce_verifier_hash(&pkce_secret)?;
        let pkce_challenge = pkce_s256_challenge(&pkce_secret);
        let authorization_endpoint =
            OAuthAuthorizationEndpoint::new(SLACK_PERSONAL_AUTHORIZATION_ENDPOINT)?;
        let authorization_url = build_authorization_url_with_scope_param(
            OAuthAuthorizeUrlRequest {
                authorization_endpoint: &authorization_endpoint,
                client_id: &client_id,
                redirect_uri: self.slot.redirect_uri(),
                state: &state,
                code_challenge: &pkce_challenge,
                scopes: &scopes,
                extra_params: &[],
            },
            OAuthScopeParam::UserScope,
        )?;
        Ok(PreparedOAuthGateFlow {
            authorization_url,
            opaque_state_hash,
            pkce_verifier_hash,
            pkce_verifier,
        })
    }
}

impl fmt::Debug for SlackPersonalOAuthGateProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SlackPersonalOAuthGateProvider")
            .field("slot", &self.slot)
            .finish()
    }
}

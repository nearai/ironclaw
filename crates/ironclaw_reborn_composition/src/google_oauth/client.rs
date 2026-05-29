use std::{fmt, sync::Arc};

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProviderClient, GOOGLE_PROVIDER_ID, GOOGLE_TOKEN_ENDPOINT, OAuthClientId,
    OAuthProviderCallbackRequest, OAuthProviderExchange, OAuthProviderExchangeContext,
    OAuthProviderRefresh, OAuthProviderRefreshRequest, OAuthRedirectUri,
    validate_provider_callback_request,
};
use ironclaw_host_api::{
    CapabilityId, NetworkMethod, RuntimeCredentialInjection, RuntimeHttpEgress,
    RuntimeHttpEgressRequest, RuntimeKind,
};
use secrecy::{ExposeSecret, SecretString};

use crate::google_oauth::policy_authorizer::{
    GoogleProviderEgressPolicyAuthorizer, google_token_network_policy,
};
use crate::google_oauth::secret_sink::{
    GoogleProviderRefreshTokenStorageRequest, GoogleProviderTokenSet, GoogleProviderTokenSink,
    GoogleProviderTokenStorageRequest,
};
use crate::google_oauth::token_request::{
    serialize_authorization_code_token_request, serialize_refresh_token_request,
};
use crate::google_oauth::token_response::{
    parse_token_response, scopes_for_exchange, scopes_for_refresh,
};

const GOOGLE_OAUTH_CAPABILITY: &str = "ironclaw_auth.google_oauth";
const DEFAULT_TIMEOUT_MS: u32 = 30_000;
const DEFAULT_RESPONSE_BODY_LIMIT: u64 = 16 * 1024;

/// Concrete Google OAuth token-exchange client for Reborn composition.
///
/// The auth crate owns the provider-exchange contracts; this adapter owns the
/// host-runtime HTTP egress, network-policy handoff, and secret-store bridge.
#[derive(Clone)]
pub(crate) struct GoogleProviderClient {
    egress: Arc<dyn RuntimeHttpEgress>,
    token_sink: Arc<dyn GoogleProviderTokenSink>,
    egress_policy_authorizer: Arc<dyn GoogleProviderEgressPolicyAuthorizer>,
    client_id: OAuthClientId,
    client_secret: Option<SecretString>,
    redirect_uri: OAuthRedirectUri,
    runtime: RuntimeKind,
    capability_id: CapabilityId,
    timeout_ms: u32,
    response_body_limit: u64,
}

impl GoogleProviderClient {
    pub(crate) fn new(
        egress: Arc<dyn RuntimeHttpEgress>,
        token_sink: Arc<dyn GoogleProviderTokenSink>,
        egress_policy_authorizer: Arc<dyn GoogleProviderEgressPolicyAuthorizer>,
        client_id: OAuthClientId,
        redirect_uri: OAuthRedirectUri,
    ) -> Result<Self, AuthProductError> {
        Ok(Self {
            egress,
            token_sink,
            egress_policy_authorizer,
            client_id,
            client_secret: None,
            redirect_uri,
            runtime: RuntimeKind::System,
            capability_id: CapabilityId::new(GOOGLE_OAUTH_CAPABILITY)
                .map_err(|_| AuthProductError::BackendUnavailable)?,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            response_body_limit: DEFAULT_RESPONSE_BODY_LIMIT,
        })
    }

    #[cfg(test)]
    pub(super) fn with_runtime(mut self, runtime: RuntimeKind) -> Self {
        self.runtime = runtime;
        self
    }

    #[cfg(test)]
    pub(super) fn with_timeout_ms(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    #[cfg(test)]
    pub(super) fn with_response_body_limit(mut self, response_body_limit: u64) -> Self {
        self.response_body_limit = response_body_limit;
        self
    }

    pub(crate) fn with_client_secret(mut self, client_secret: SecretString) -> Self {
        self.client_secret = Some(client_secret);
        self
    }
}

impl fmt::Debug for GoogleProviderClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleProviderClient")
            .field("client_id", &self.client_id)
            .field("redirect_uri", &self.redirect_uri)
            .field("runtime", &self.runtime)
            .field("capability_id", &self.capability_id)
            .field("timeout_ms", &self.timeout_ms)
            .field("response_body_limit", &self.response_body_limit)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("egress", &"Arc<dyn RuntimeHttpEgress>")
            .field("token_sink", &"Arc<dyn GoogleProviderTokenSink>")
            .field(
                "egress_policy_authorizer",
                &"Arc<dyn GoogleProviderEgressPolicyAuthorizer>",
            )
            .finish()
    }
}

#[async_trait]
impl AuthProviderClient for GoogleProviderClient {
    async fn exchange_callback(
        &self,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        if request.provider.as_str() != GOOGLE_PROVIDER_ID {
            return Err(AuthProductError::TokenExchangeFailed);
        }
        validate_provider_callback_request(&request)?;
        let callback_scope = context.scope.resource.clone();
        if callback_scope.is_system() {
            return Err(AuthProductError::CrossScopeDenied);
        }

        let body = serialize_authorization_code_token_request(
            self.client_id.as_str(),
            self.redirect_uri.as_str(),
            self.client_secret.as_ref(),
            request.authorization_code.expose_secret(),
            request.pkce_verifier.expose_secret(),
        );
        let network_policy = google_token_network_policy(self.response_body_limit);
        self.egress_policy_authorizer
            .authorize_google_token_exchange(&callback_scope, &self.capability_id, &network_policy)
            .await?;

        let egress_request = RuntimeHttpEgressRequest {
            runtime: self.runtime,
            scope: callback_scope.clone(),
            capability_id: self.capability_id.clone(),
            method: NetworkMethod::Post,
            url: GOOGLE_TOKEN_ENDPOINT.to_string(),
            headers: vec![
                (
                    "content-type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                ),
                ("accept".to_string(), "application/json".to_string()),
            ],
            body,
            network_policy,
            credential_injections: Vec::<RuntimeCredentialInjection>::new(),
            response_body_limit: Some(self.response_body_limit),
            save_body_to: None,
            timeout_ms: Some(self.timeout_ms),
        };
        // Production host egress requires the policy staged above for this
        // scope/capability. The request-carried policy is only a legacy/test
        // fallback and must not be treated as authority on the production path.
        let response = self
            .egress
            .execute(egress_request)
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)?;

        if !(200..300).contains(&response.status) {
            return Err(AuthProductError::TokenExchangeFailed);
        }

        let token_response = parse_token_response(&response.body)?;
        let scopes = scopes_for_exchange(&token_response)?;
        let token_sink = Arc::clone(&self.token_sink);
        let stored_tokens = token_sink
            .store_tokens(GoogleProviderTokenStorageRequest {
                scope: callback_scope,
                flow_id: context.flow_id,
                tokens: GoogleProviderTokenSet {
                    access_token: token_response.response.access_token,
                    refresh_token: token_response.response.refresh_token,
                },
            })
            .await?;

        Ok(OAuthProviderExchange {
            provider: request.provider,
            account_label: request.account_label,
            authorization_code_hash: request.authorization_code_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            access_secret: stored_tokens.access_secret,
            refresh_secret: stored_tokens.refresh_secret,
            scopes,
            account_id: None,
        })
    }

    async fn refresh_token(
        &self,
        request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        if request.provider.as_str() != GOOGLE_PROVIDER_ID {
            return Err(AuthProductError::RefreshFailed);
        }
        let refresh_scope = request.scope.resource.clone();
        if refresh_scope.is_system() {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let refresh_token = self
            .token_sink
            .load_refresh_token(&refresh_scope, &request.refresh_secret)
            .await?;
        let body = serialize_refresh_token_request(
            self.client_id.as_str(),
            self.client_secret.as_ref(),
            refresh_token.expose_secret(),
        );
        let network_policy = google_token_network_policy(self.response_body_limit);
        self.egress_policy_authorizer
            .authorize_google_token_exchange(&refresh_scope, &self.capability_id, &network_policy)
            .await?;

        let egress_request = RuntimeHttpEgressRequest {
            runtime: self.runtime,
            scope: refresh_scope.clone(),
            capability_id: self.capability_id.clone(),
            method: NetworkMethod::Post,
            url: GOOGLE_TOKEN_ENDPOINT.to_string(),
            headers: vec![
                (
                    "content-type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                ),
                ("accept".to_string(), "application/json".to_string()),
            ],
            body,
            network_policy,
            credential_injections: Vec::<RuntimeCredentialInjection>::new(),
            response_body_limit: Some(self.response_body_limit),
            save_body_to: None,
            timeout_ms: Some(self.timeout_ms),
        };
        // Production host egress requires the policy staged above for this
        // scope/capability. The request-carried policy is only a legacy/test
        // fallback and must not be treated as authority on the production path.
        let response = self
            .egress
            .execute(egress_request)
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)?;

        if !(200..300).contains(&response.status) {
            return Err(map_refresh_error(response.status));
        }

        let token_response =
            parse_token_response(&response.body).map_err(|_| AuthProductError::RefreshFailed)?;
        let scopes = scopes_for_refresh(&token_response, &request.scopes);
        let stored_tokens = self
            .token_sink
            .store_refreshed_tokens(GoogleProviderRefreshTokenStorageRequest {
                scope: refresh_scope,
                account_id: request.account_id,
                tokens: GoogleProviderTokenSet {
                    access_token: token_response.response.access_token,
                    refresh_token: token_response.response.refresh_token,
                },
            })
            .await?;

        Ok(OAuthProviderRefresh {
            provider: request.provider,
            access_secret: stored_tokens.access_secret,
            refresh_secret: stored_tokens.refresh_secret,
            scopes,
        })
    }

    async fn cleanup_exchange(
        &self,
        context: OAuthProviderExchangeContext,
        exchange: &OAuthProviderExchange,
    ) -> Result<(), AuthProductError> {
        if exchange.provider.as_str() != GOOGLE_PROVIDER_ID {
            return Ok(());
        }
        let mut handles = vec![exchange.access_secret.clone()];
        handles.extend(exchange.refresh_secret.clone());
        self.token_sink
            .delete_tokens(&context.scope.resource, &handles)
            .await
    }
}

fn map_refresh_error(status: u16) -> AuthProductError {
    if (500..600).contains(&status) {
        AuthProductError::BackendUnavailable
    } else {
        AuthProductError::RefreshFailed
    }
}

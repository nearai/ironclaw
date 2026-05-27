use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ResourceScope,
    RuntimeCredentialInjection, RuntimeHttpEgress, RuntimeHttpEgressRequest, RuntimeKind,
    SecretHandle,
};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::form_urlencoded::Serializer;

use crate::{
    AuthProductError, AuthProviderClient, GOOGLE_PROVIDER_ID, GOOGLE_TOKEN_ENDPOINT, OAuthClientId,
    OAuthProviderCallbackRequest, OAuthProviderExchange, OAuthRedirectUri, OAuthTokenResponse,
};

const GOOGLE_OAUTH_CAPABILITY: &str = "ironclaw_auth.google_oauth";
const DEFAULT_TIMEOUT_MS: u32 = 30_000;
const DEFAULT_RESPONSE_BODY_LIMIT: u64 = 16 * 1024;

/// Boundary for turning provider token material into durable secret handles.
///
/// `ironclaw_auth` intentionally does not own durable secret storage; the
/// caller injects the storage boundary via this trait.
pub trait GoogleProviderTokenSink: Send + Sync {
    fn store_tokens(
        &self,
        tokens: GoogleProviderTokenSet,
    ) -> Result<GoogleProviderStoredTokens, AuthProductError>;
}

/// Boundary for staging/authorizing the Google token-exchange network policy.
///
/// Production Reborn egress uses staged policy handoffs instead of trusting the
/// policy embedded in `RuntimeHttpEgressRequest`; callers must inject this
/// authority boundary before token exchange is attempted.
pub trait GoogleProviderEgressPolicyAuthorizer: Send + Sync {
    fn authorize_google_token_exchange(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        policy: &NetworkPolicy,
    ) -> Result<(), AuthProductError>;
}

/// Raw Google token material passed exactly once to the injected storage
/// boundary. This type intentionally does not implement serde.
pub struct GoogleProviderTokenSet {
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
}

impl fmt::Debug for GoogleProviderTokenSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleProviderTokenSet")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleProviderStoredTokens {
    pub access_secret: SecretHandle,
    pub refresh_secret: Option<SecretHandle>,
}

#[derive(Clone)]
pub struct GoogleProviderClient {
    egress: Arc<dyn RuntimeHttpEgress>,
    token_sink: Arc<dyn GoogleProviderTokenSink>,
    egress_policy_authorizer: Arc<dyn GoogleProviderEgressPolicyAuthorizer>,
    client_id: OAuthClientId,
    client_secret: Option<SecretString>,
    redirect_uri: OAuthRedirectUri,
    runtime: RuntimeKind,
    scope: ResourceScope,
    capability_id: CapabilityId,
    timeout_ms: u32,
    response_body_limit: u64,
}

impl GoogleProviderClient {
    pub fn new(
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
            scope: ResourceScope::system(),
            capability_id: CapabilityId::new(GOOGLE_OAUTH_CAPABILITY).map_err(|_| {
                AuthProductError::invalid_request("google provider capability id is invalid")
            })?,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            response_body_limit: DEFAULT_RESPONSE_BODY_LIMIT,
        })
    }

    pub fn with_runtime(mut self, runtime: RuntimeKind) -> Self {
        self.runtime = runtime;
        self
    }

    pub fn with_scope(mut self, scope: ResourceScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn with_response_body_limit(mut self, response_body_limit: u64) -> Self {
        self.response_body_limit = response_body_limit;
        self
    }

    pub fn with_client_secret(mut self, client_secret: SecretString) -> Self {
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
            .field("scope", &self.scope)
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
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        if request.provider.as_str() != GOOGLE_PROVIDER_ID {
            return Err(AuthProductError::TokenExchangeFailed);
        }
        crate::provider::validate_provider_callback_request(&request)?;

        let body = serialize_token_request(
            self.client_id.as_str(),
            self.redirect_uri.as_str(),
            self.client_secret.as_ref(),
            request.authorization_code.expose_secret(),
            request.pkce_verifier.expose_secret(),
        );
        let network_policy = google_token_network_policy(self.response_body_limit);
        self.egress_policy_authorizer
            .authorize_google_token_exchange(&self.scope, &self.capability_id, &network_policy)?;

        let egress = Arc::clone(&self.egress);
        let egress_request = RuntimeHttpEgressRequest {
            runtime: self.runtime,
            scope: self.scope.clone(),
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
        let response = tokio::task::spawn_blocking(move || egress.execute(egress_request))
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        let response = response.map_err(|_| AuthProductError::BackendUnavailable)?;

        if !(200..300).contains(&response.status) {
            return Err(AuthProductError::TokenExchangeFailed);
        }

        let token_response = parse_token_response(&response.body)?;
        let scopes = token_response.scopes;
        let token_sink = Arc::clone(&self.token_sink);
        let stored_tokens = tokio::task::spawn_blocking(move || {
            token_sink.store_tokens(GoogleProviderTokenSet {
                access_token: token_response.access_token,
                refresh_token: token_response.refresh_token,
            })
        })
        .await
        .map_err(|_| AuthProductError::BackendUnavailable)??;

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
}

fn google_token_network_policy(response_body_limit: u64) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "oauth2.googleapis.com".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(response_body_limit),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct GoogleTokenResponseBody {
    access_token: SecretString,
    #[serde(default)]
    refresh_token: Option<SecretString>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    token_type: Option<String>,
}

fn parse_token_response(body: &[u8]) -> Result<OAuthTokenResponse, AuthProductError> {
    let parsed: GoogleTokenResponseBody =
        serde_json::from_slice(body).map_err(|_| AuthProductError::TokenExchangeFailed)?;
    let response = OAuthTokenResponse::new(
        parsed.access_token,
        parsed.refresh_token,
        parsed.scope.as_deref(),
        parsed.expires_in,
    )
    .map_err(|_| AuthProductError::TokenExchangeFailed)?;

    let _ = parsed.token_type;
    Ok(response)
}

fn serialize_token_request(
    client_id: &str,
    redirect_uri: &str,
    client_secret: Option<&SecretString>,
    authorization_code: &str,
    pkce_verifier: &str,
) -> Vec<u8> {
    let mut serializer = Serializer::new(String::new());
    serializer
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", authorization_code)
        .append_pair("code_verifier", pkce_verifier)
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri);
    if let Some(client_secret) = client_secret {
        serializer.append_pair("client_secret", client_secret.expose_secret());
    }
    serializer.finish().into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_body_parses_to_token_response() {
        let response = parse_token_response(
            br#"{"access_token":"access","refresh_token":"refresh","scope":"repo gmail.readonly","expires_in":3600,"token_type":"Bearer"}"#,
        )
        .expect("response");
        assert_eq!(response.scopes.len(), 2);
        assert_eq!(response.expires_in_seconds, Some(3600));
    }

    #[test]
    fn token_response_rejects_empty_or_missing_access_token() {
        assert_eq!(
            parse_token_response(b"").expect_err("empty response"),
            AuthProductError::TokenExchangeFailed
        );
        assert_eq!(
            parse_token_response(br#"{"refresh_token":"refresh"}"#)
                .expect_err("missing access token"),
            AuthProductError::TokenExchangeFailed
        );
    }
}

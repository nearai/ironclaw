use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthFlowId, AuthProductError, AuthProviderClient, CredentialAccountId, OAuthClientId,
    OAuthProviderCallbackRequest, OAuthProviderExchange, OAuthProviderExchangeContext,
    OAuthProviderRefresh, OAuthProviderRefreshRequest, OAuthRedirectUri, OAuthTokenResponse,
    ProviderScope, validate_provider_callback_request,
};
use ironclaw_capabilities::{
    CapabilityObligationHandler, CapabilityObligationPhase, CapabilityObligationRequest,
};
use ironclaw_host_api::{
    CapabilityId, CapabilitySet, CorrelationId, ExtensionId, MountView, NetworkMethod,
    NetworkPolicy, NetworkScheme, NetworkTargetPattern, Obligation, ResourceEstimate,
    ResourceScope, RuntimeCredentialInjection, RuntimeHttpEgress, RuntimeHttpEgressRequest,
    RuntimeKind, SecretHandle, TrustClass,
};
use ironclaw_secrets::SecretStore;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

const DEFAULT_TIMEOUT_MS: u32 = 30_000;
const DEFAULT_RESPONSE_BODY_LIMIT: u64 = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExchangeScopePolicy {
    RequireProviderScope,
    FallbackToRequested,
}

#[derive(Debug, Clone)]
pub(crate) struct HostOAuthProviderSpec {
    pub(crate) provider_id: &'static str,
    pub(crate) capability_id: &'static str,
    pub(crate) token_endpoint: &'static str,
    pub(crate) secret_handle_prefix: &'static str,
    pub(crate) resource: Option<&'static str>,
    pub(crate) exchange_scope_policy: ExchangeScopePolicy,
}

#[derive(Clone, Debug)]
pub(crate) struct OAuthClientMaterial {
    pub(crate) client_id: OAuthClientId,
    pub(crate) client_secret: Option<SecretString>,
    pub(crate) redirect_uri: OAuthRedirectUri,
    pub(crate) token_endpoint: String,
}

#[async_trait]
pub(crate) trait OAuthClientMaterialSource: Send + Sync + fmt::Debug {
    async fn exchange_material(
        &self,
        scope: &ResourceScope,
        flow_id: AuthFlowId,
    ) -> Result<OAuthClientMaterial, AuthProductError>;

    async fn refresh_material(
        &self,
        scope: &ResourceScope,
        refresh_secret: &SecretHandle,
    ) -> Result<OAuthClientMaterial, AuthProductError>;

    async fn bind_refresh_material(
        &self,
        scope: &ResourceScope,
        flow_id: AuthFlowId,
        refresh_secret: &SecretHandle,
    ) -> Result<(), AuthProductError>;

    async fn cleanup_exchange_material(
        &self,
        scope: &ResourceScope,
        flow_id: AuthFlowId,
    ) -> Result<(), AuthProductError>;
}

#[derive(Clone)]
struct StaticOAuthClientMaterialSource {
    material: OAuthClientMaterial,
}

impl fmt::Debug for StaticOAuthClientMaterialSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StaticOAuthClientMaterialSource")
            .field("client_id", &self.material.client_id)
            .field("redirect_uri", &self.material.redirect_uri)
            .field("token_endpoint", &self.material.token_endpoint)
            .field(
                "client_secret",
                &self.material.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[async_trait]
impl OAuthClientMaterialSource for StaticOAuthClientMaterialSource {
    async fn exchange_material(
        &self,
        _scope: &ResourceScope,
        _flow_id: AuthFlowId,
    ) -> Result<OAuthClientMaterial, AuthProductError> {
        Ok(self.material.clone())
    }

    async fn refresh_material(
        &self,
        _scope: &ResourceScope,
        _refresh_secret: &SecretHandle,
    ) -> Result<OAuthClientMaterial, AuthProductError> {
        Ok(self.material.clone())
    }

    async fn bind_refresh_material(
        &self,
        _scope: &ResourceScope,
        _flow_id: AuthFlowId,
        _refresh_secret: &SecretHandle,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }

    async fn cleanup_exchange_material(
        &self,
        _scope: &ResourceScope,
        _flow_id: AuthFlowId,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct HostOAuthProviderClient {
    spec: HostOAuthProviderSpec,
    egress: Arc<dyn RuntimeHttpEgress>,
    secret_store: Arc<dyn SecretStore>,
    obligation_handler: Arc<dyn CapabilityObligationHandler>,
    client_material: Arc<dyn OAuthClientMaterialSource>,
    static_client_material: Option<OAuthClientMaterial>,
    runtime: RuntimeKind,
    capability_id: CapabilityId,
    timeout_ms: u32,
    response_body_limit: u64,
}

impl HostOAuthProviderClient {
    pub(crate) fn new(
        spec: HostOAuthProviderSpec,
        egress: Arc<dyn RuntimeHttpEgress>,
        secret_store: Arc<dyn SecretStore>,
        obligation_handler: Arc<dyn CapabilityObligationHandler>,
        client_id: OAuthClientId,
        redirect_uri: OAuthRedirectUri,
    ) -> Result<Self, AuthProductError> {
        let capability_id = CapabilityId::new(spec.capability_id)
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        oauth_endpoint_host(spec.token_endpoint)?;
        let material = OAuthClientMaterial {
            client_id,
            client_secret: None,
            redirect_uri,
            token_endpoint: spec.token_endpoint.to_string(),
        };
        Ok(Self {
            spec,
            egress,
            secret_store,
            obligation_handler,
            client_material: Arc::new(StaticOAuthClientMaterialSource {
                material: material.clone(),
            }),
            static_client_material: Some(material),
            runtime: RuntimeKind::System,
            capability_id,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            response_body_limit: DEFAULT_RESPONSE_BODY_LIMIT,
        })
    }

    pub(crate) fn new_with_client_material(
        spec: HostOAuthProviderSpec,
        egress: Arc<dyn RuntimeHttpEgress>,
        secret_store: Arc<dyn SecretStore>,
        obligation_handler: Arc<dyn CapabilityObligationHandler>,
        client_material: Arc<dyn OAuthClientMaterialSource>,
    ) -> Result<Self, AuthProductError> {
        let capability_id = CapabilityId::new(spec.capability_id)
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        oauth_endpoint_host(spec.token_endpoint)?;
        Ok(Self {
            spec,
            egress,
            secret_store,
            obligation_handler,
            client_material,
            static_client_material: None,
            runtime: RuntimeKind::System,
            capability_id,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            response_body_limit: DEFAULT_RESPONSE_BODY_LIMIT,
        })
    }

    pub(crate) fn with_client_secret(mut self, client_secret: SecretString) -> Self {
        let Some(mut material) = self.static_client_material.clone() else {
            return self;
        };
        material.client_secret = Some(client_secret);
        self.client_material = Arc::new(StaticOAuthClientMaterialSource {
            material: material.clone(),
        });
        self.static_client_material = Some(material);
        self
    }

    async fn execute_token_request(
        &self,
        scope: ResourceScope,
        token_endpoint: &str,
        body: Vec<u8>,
        refresh_request: bool,
    ) -> Result<OAuthTokenResponse, AuthProductError> {
        let token_host = oauth_endpoint_host(token_endpoint)?;
        let policy = oauth_network_policy(&token_host, self.response_body_limit);
        authorize_oauth_egress(
            Arc::clone(&self.obligation_handler),
            &scope,
            &self.capability_id,
            &policy,
        )
        .await?;
        let response = self
            .egress
            .execute(RuntimeHttpEgressRequest {
                runtime: self.runtime,
                scope,
                capability_id: self.capability_id.clone(),
                method: NetworkMethod::Post,
                url: token_endpoint.to_string(),
                headers: vec![
                    (
                        "content-type".to_string(),
                        "application/x-www-form-urlencoded".to_string(),
                    ),
                    ("accept".to_string(), "application/json".to_string()),
                ],
                body,
                network_policy: policy,
                credential_injections: Vec::<RuntimeCredentialInjection>::new(),
                response_body_limit: Some(self.response_body_limit),
                save_body_to: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        if !(200..300).contains(&response.status) {
            return Err(if (500..600).contains(&response.status) {
                AuthProductError::BackendUnavailable
            } else if refresh_request {
                AuthProductError::RefreshFailed
            } else {
                AuthProductError::TokenExchangeFailed
            });
        }
        parse_token_response(&response.body).map_err(|error| {
            if refresh_request {
                match error {
                    AuthProductError::BackendUnavailable => AuthProductError::BackendUnavailable,
                    _ => AuthProductError::RefreshFailed,
                }
            } else {
                error
            }
        })
    }

    async fn store_tokens(
        &self,
        scope: ResourceScope,
        flow_id: AuthFlowId,
        tokens: OAuthTokenResponse,
    ) -> Result<StoredOAuthTokens, AuthProductError> {
        let access_secret =
            exchange_token_handle(&self.spec, flow_id, scope.invocation_id, "access")?;
        let refresh_secret = tokens
            .refresh_token
            .as_ref()
            .map(|_| exchange_token_handle(&self.spec, flow_id, scope.invocation_id, "refresh"))
            .transpose()?;
        self.store_token_pair(scope, access_secret, refresh_secret, tokens)
            .await
    }

    async fn load_refresh_token(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretString, AuthProductError> {
        let lease = self
            .secret_store
            .lease_once(scope, handle)
            .await
            .map_err(map_refresh_secret_error)?;
        self.secret_store
            .consume(scope, lease.id)
            .await
            .map_err(map_refresh_secret_error)
    }

    async fn store_refreshed_tokens(
        &self,
        scope: ResourceScope,
        account_id: CredentialAccountId,
        tokens: OAuthTokenResponse,
    ) -> Result<StoredOAuthTokens, AuthProductError> {
        let access_secret = refresh_token_handle(&self.spec, account_id, "access")?;
        let refresh_secret = tokens
            .refresh_token
            .as_ref()
            .map(|_| refresh_token_handle(&self.spec, account_id, "refresh"))
            .transpose()?;
        self.store_token_pair(scope, access_secret, refresh_secret, tokens)
            .await
    }

    async fn store_token_pair(
        &self,
        scope: ResourceScope,
        access_secret: SecretHandle,
        refresh_secret: Option<SecretHandle>,
        tokens: OAuthTokenResponse,
    ) -> Result<StoredOAuthTokens, AuthProductError> {
        let refresh_token = tokens.refresh_token;
        self.secret_store
            .put(scope.clone(), access_secret.clone(), tokens.access_token)
            .await
            .map_err(map_secret_store_error)?;
        let refresh_secret = match (refresh_secret, refresh_token) {
            (Some(handle), Some(refresh_token)) => {
                if let Err(error) = self
                    .secret_store
                    .put(scope.clone(), handle.clone(), refresh_token)
                    .await
                {
                    cleanup_written_access(
                        &self.secret_store,
                        &scope,
                        &access_secret,
                        self.spec.provider_id,
                    )
                    .await;
                    return Err(map_secret_store_error(error));
                }
                Some(handle)
            }
            (None, None) => None,
            _ => return Err(AuthProductError::BackendUnavailable),
        };
        Ok(StoredOAuthTokens {
            access_secret,
            refresh_secret,
        })
    }

    async fn delete_tokens(
        &self,
        scope: &ResourceScope,
        handles: &[SecretHandle],
    ) -> Result<(), AuthProductError> {
        let mut first_error = None;
        for handle in handles {
            if let Err(error) = self.secret_store.delete(scope, handle).await
                && first_error.is_none()
            {
                first_error = Some(map_secret_store_error(error));
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl fmt::Debug for HostOAuthProviderClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostOAuthProviderClient")
            .field("provider_id", &self.spec.provider_id)
            .field("client_material", &self.client_material)
            .field("runtime", &self.runtime)
            .field("capability_id", &self.capability_id)
            .field("timeout_ms", &self.timeout_ms)
            .field("response_body_limit", &self.response_body_limit)
            .finish()
    }
}

#[async_trait]
impl AuthProviderClient for HostOAuthProviderClient {
    async fn exchange_callback(
        &self,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        if request.provider.as_str() != self.spec.provider_id {
            return Err(AuthProductError::TokenExchangeFailed);
        }
        validate_provider_callback_request(&request)?;
        let callback_scope = context.scope.resource.clone();
        if callback_scope.is_system() {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let material = self
            .client_material
            .exchange_material(&callback_scope, context.flow_id)
            .await?;
        let body = authorization_code_body(
            &self.spec,
            material.client_id.as_str(),
            material.redirect_uri.as_str(),
            material.client_secret.as_ref(),
            request.authorization_code.expose_secret(),
            request.pkce_verifier.expose_secret(),
        );
        let token_response = self
            .execute_token_request(
                callback_scope.clone(),
                &material.token_endpoint,
                body,
                false,
            )
            .await?;
        let scopes = scopes_for_exchange(&self.spec, &token_response, &request.scopes)?;
        let stored_tokens = self
            .store_tokens(callback_scope, context.flow_id, token_response)
            .await?;
        if let Some(refresh_secret) = &stored_tokens.refresh_secret {
            self.client_material
                .bind_refresh_material(&context.scope.resource, context.flow_id, refresh_secret)
                .await?;
        }

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
        if request.provider.as_str() != self.spec.provider_id {
            return Err(AuthProductError::RefreshFailed);
        }
        let refresh_scope = request.scope.resource.clone();
        if refresh_scope.is_system() {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let refresh_token = self
            .load_refresh_token(&refresh_scope, &request.refresh_secret)
            .await?;
        let material = self
            .client_material
            .refresh_material(&refresh_scope, &request.refresh_secret)
            .await?;
        let body = refresh_body(
            &self.spec,
            material.client_id.as_str(),
            material.client_secret.as_ref(),
            refresh_token.expose_secret(),
        );
        let token_response = self
            .execute_token_request(refresh_scope.clone(), &material.token_endpoint, body, true)
            .await?;
        let scopes = scopes_for_refresh(&token_response, &request.scopes);
        let stored_tokens = self
            .store_refreshed_tokens(refresh_scope, request.account_id, token_response)
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
        if exchange.provider.as_str() != self.spec.provider_id {
            return Ok(());
        }
        let mut handles = vec![exchange.access_secret.clone()];
        handles.extend(exchange.refresh_secret.clone());
        self.delete_tokens(&context.scope.resource, &handles)
            .await?;
        self.client_material
            .cleanup_exchange_material(&context.scope.resource, context.flow_id)
            .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredOAuthTokens {
    access_secret: SecretHandle,
    refresh_secret: Option<SecretHandle>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct TokenResponseBody {
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
    let parsed: TokenResponseBody =
        serde_json::from_slice(body).map_err(|_| AuthProductError::TokenExchangeFailed)?;
    let response_scope = parsed
        .scope
        .as_deref()
        .filter(|scope| !scope.trim().is_empty());
    let _ = parsed.token_type;
    OAuthTokenResponse::new(
        parsed.access_token,
        parsed.refresh_token,
        response_scope,
        parsed.expires_in,
    )
    .map_err(|_| AuthProductError::TokenExchangeFailed)
}

fn scopes_for_exchange(
    spec: &HostOAuthProviderSpec,
    token_response: &OAuthTokenResponse,
    requested_scopes: &[ProviderScope],
) -> Result<Vec<ProviderScope>, AuthProductError> {
    if !token_response.scopes.is_empty() {
        return Ok(token_response.scopes.clone());
    }
    match spec.exchange_scope_policy {
        ExchangeScopePolicy::RequireProviderScope => Err(AuthProductError::TokenExchangeFailed),
        ExchangeScopePolicy::FallbackToRequested => Ok(requested_scopes.to_vec()),
    }
}

fn scopes_for_refresh(
    token_response: &OAuthTokenResponse,
    existing_scopes: &[ProviderScope],
) -> Vec<ProviderScope> {
    if token_response.scopes.is_empty() {
        existing_scopes.to_vec()
    } else {
        token_response.scopes.clone()
    }
}

fn authorization_code_body(
    spec: &HostOAuthProviderSpec,
    client_id: &str,
    redirect_uri: &str,
    client_secret: Option<&SecretString>,
    code: &str,
    pkce_verifier: &str,
) -> Vec<u8> {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", code)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("client_id", client_id)
        .append_pair("code_verifier", pkce_verifier);
    append_resource(spec, &mut serializer);
    if let Some(client_secret) = client_secret {
        serializer.append_pair("client_secret", client_secret.expose_secret());
    }
    serializer.finish().into_bytes()
}

fn refresh_body(
    spec: &HostOAuthProviderSpec,
    client_id: &str,
    client_secret: Option<&SecretString>,
    refresh_token: &str,
) -> Vec<u8> {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer
        .append_pair("grant_type", "refresh_token")
        .append_pair("refresh_token", refresh_token)
        .append_pair("client_id", client_id);
    append_resource(spec, &mut serializer);
    if let Some(client_secret) = client_secret {
        serializer.append_pair("client_secret", client_secret.expose_secret());
    }
    serializer.finish().into_bytes()
}

fn append_resource(
    spec: &HostOAuthProviderSpec,
    serializer: &mut url::form_urlencoded::Serializer<String>,
) {
    if let Some(resource) = spec.resource {
        serializer.append_pair("resource", resource);
    }
}

fn exchange_token_handle(
    spec: &HostOAuthProviderSpec,
    flow_id: AuthFlowId,
    invocation_id: ironclaw_host_api::InvocationId,
    token_kind: &'static str,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!(
        "{}-oauth-{token_kind}-{flow_id}-{invocation_id}",
        spec.secret_handle_prefix
    ))
    .map_err(|_| AuthProductError::BackendUnavailable)
}

fn refresh_token_handle(
    spec: &HostOAuthProviderSpec,
    account_id: CredentialAccountId,
    token_kind: &'static str,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!(
        "{}-oauth-refresh-{token_kind}-{account_id}",
        spec.secret_handle_prefix
    ))
    .map_err(|_| AuthProductError::BackendUnavailable)
}

pub(crate) fn oauth_endpoint_host(endpoint: &str) -> Result<String, AuthProductError> {
    let url = url::Url::parse(endpoint).map_err(|_| AuthProductError::BackendUnavailable)?;
    if url.scheme() != "https" {
        return Err(AuthProductError::BackendUnavailable);
    }
    url.host_str()
        .filter(|host| !host.trim().is_empty())
        .map(str::to_string)
        .ok_or(AuthProductError::BackendUnavailable)
}

pub(crate) fn oauth_network_policy(token_host: &str, response_body_limit: u64) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: token_host.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(response_body_limit),
    }
}

pub(crate) async fn authorize_oauth_egress(
    handler: Arc<dyn CapabilityObligationHandler>,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    policy: &NetworkPolicy,
) -> Result<(), AuthProductError> {
    let context = oauth_execution_context(scope.clone())?;
    let estimate = ResourceEstimate {
        network_egress_bytes: policy.max_egress_bytes,
        ..ResourceEstimate::default()
    };
    handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id,
            estimate: &estimate,
            obligations: &[Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            }],
        })
        .await
        .map_err(|_| AuthProductError::BackendUnavailable)
}

fn oauth_execution_context(
    resource_scope: ResourceScope,
) -> Result<ironclaw_host_api::ExecutionContext, AuthProductError> {
    let context = ironclaw_host_api::ExecutionContext {
        invocation_id: resource_scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("ironclaw_auth")
            .map_err(|_| AuthProductError::BackendUnavailable)?,
        runtime: RuntimeKind::System,
        trust: TrustClass::System,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        resource_scope,
    };
    context
        .validate()
        .map_err(|_| AuthProductError::BackendUnavailable)?;
    Ok(context)
}

async fn cleanup_written_access(
    store: &Arc<dyn SecretStore>,
    scope: &ResourceScope,
    access_secret: &SecretHandle,
    provider_id: &str,
) {
    if let Err(delete_error) = store.delete(scope, access_secret).await {
        tracing::warn!(
            provider_id,
            secret_store_reason = delete_error.stable_reason(),
            "oauth callback cleanup failed after refresh token write failure"
        );
    }
}

fn map_secret_store_error(error: ironclaw_secrets::SecretStoreError) -> AuthProductError {
    tracing::debug!(
        secret_store_reason = error.stable_reason(),
        "oauth provider secret store operation failed"
    );
    AuthProductError::BackendUnavailable
}

fn map_refresh_secret_error(error: ironclaw_secrets::SecretStoreError) -> AuthProductError {
    if error.is_unknown_secret()
        || error.is_unknown_lease()
        || error.is_consumed()
        || error.is_revoked()
        || error.is_expired()
    {
        AuthProductError::RefreshFailed
    } else {
        tracing::debug!(
            secret_store_reason = error.stable_reason(),
            "oauth provider refresh secret load failed"
        );
        AuthProductError::BackendUnavailable
    }
}

#[cfg(test)]
mod tests;

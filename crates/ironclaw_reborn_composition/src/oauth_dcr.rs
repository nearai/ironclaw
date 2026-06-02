use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowId, AuthFlowKind, AuthFlowManager,
    AuthFlowOwnerScope, AuthFlowRecordSource, AuthGateRef, AuthProductError, AuthProductScope,
    AuthProviderId, CredentialAccountLabel, NewAuthFlow, OAuthAuthorizationEndpoint,
    OAuthAuthorizeUrlRequest, OAuthClientId, OAuthExtraParam, OAuthRedirectUri, OAuthState,
    PkceVerifierSecret, ProviderScope, TurnGateAuthFlowQuery, TurnRunRef, build_authorization_url,
    opaque_state_hash, pkce_s256_challenge, pkce_verifier_hash,
};
use ironclaw_capabilities::CapabilityObligationHandler;
use ironclaw_host_api::{
    CapabilityId, InvocationId, NetworkMethod, ResourceScope, RuntimeCredentialAuthRequirement,
    RuntimeHttpEgress, RuntimeHttpEgressRequest, RuntimeKind, SecretHandle,
};
use ironclaw_product_adapters::AuthPromptChallengeKind;
use ironclaw_secrets::{SecretMaterial, SecretStore};
use ironclaw_turns::{TurnRunId, TurnScope};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::oauth_dcr_protocol::{
    AuthorizationServerMetadata, DcrRegistrationRequest, DcrRegistrationResponse,
    ProtectedResourceMetadata, StoredDcrClientMaterial, authorization_server_metadata_url,
    authorization_server_metadata_url_from_issuer, callback_base_url, flow_secret_handle,
    protected_resource_metadata_url, refresh_secret_handle, scope_text, validate_callback_origin,
};
use crate::oauth_provider_client::{
    HostOAuthProviderSpec, OAuthClientMaterial, OAuthClientMaterialSource, authorize_oauth_egress,
    oauth_endpoint_host, oauth_network_policy,
};
use crate::projection::AuthChallengeView;

const DCR_RESPONSE_BODY_LIMIT: u64 = 32 * 1024;
const DCR_TIMEOUT_MS: u32 = 30_000;
const DCR_FLOW_TTL_SECONDS: i64 = 600;

#[derive(Debug, Clone)]
pub(crate) struct OAuthDcrProviderConfig {
    pub(crate) spec: HostOAuthProviderSpec,
    pub(crate) callback_origin: String,
    pub(crate) client_name: String,
    pub(crate) account_label: CredentialAccountLabel,
    pub(crate) scopes: Vec<ProviderScope>,
}

#[derive(Clone)]
pub(crate) struct OAuthDcrProvider {
    spec: HostOAuthProviderSpec,
    callback_origin: String,
    client_name: String,
    account_label: CredentialAccountLabel,
    scopes: Vec<ProviderScope>,
    egress: Arc<dyn RuntimeHttpEgress>,
    secret_store: Arc<dyn SecretStore>,
    obligation_handler: Arc<dyn CapabilityObligationHandler>,
    capability_id: CapabilityId,
}

impl OAuthDcrProvider {
    pub(crate) fn new(
        config: OAuthDcrProviderConfig,
        egress: Arc<dyn RuntimeHttpEgress>,
        secret_store: Arc<dyn SecretStore>,
        obligation_handler: Arc<dyn CapabilityObligationHandler>,
    ) -> Result<Self, AuthProductError> {
        validate_callback_origin(&config.callback_origin)?;
        let capability_id = CapabilityId::new(config.spec.capability_id)
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        Ok(Self {
            spec: config.spec,
            callback_origin: config.callback_origin,
            client_name: config.client_name,
            account_label: config.account_label,
            scopes: config.scopes,
            egress,
            secret_store,
            obligation_handler,
            capability_id,
        })
    }

    pub(crate) fn spec(&self) -> &HostOAuthProviderSpec {
        &self.spec
    }

    pub(crate) async fn challenge_for_blocked_gate(
        &self,
        flow_manager: &Arc<dyn AuthFlowManager>,
        flow_source: &Arc<dyn AuthFlowRecordSource>,
        scope: &TurnScope,
        owner_user_id: &ironclaw_host_api::UserId,
        run_id: TurnRunId,
        gate_ref: &AuthGateRef,
    ) -> Result<AuthChallengeView, AuthProductError> {
        let auth_scope = auth_scope_for_blocked_turn(scope, owner_user_id);
        let turn_run_ref = TurnRunRef::new(run_id.to_string())?;
        let query = TurnGateAuthFlowQuery {
            owner: AuthFlowOwnerScope {
                tenant_id: auth_scope.resource.tenant_id.clone(),
                user_id: auth_scope.resource.user_id.clone(),
                agent_id: auth_scope.resource.agent_id.clone(),
                project_id: auth_scope.resource.project_id.clone(),
                thread_id: scope.thread_id.clone(),
            },
            turn_run_ref: turn_run_ref.clone(),
            gate_ref: gate_ref.clone(),
            include_terminal: false,
        };
        if let Some(existing) = flow_source.flow_for_turn_gate(query.clone()).await? {
            return challenge_view_from_flow(&existing);
        }

        let flow_id = AuthFlowId::new();
        let material = self
            .prepare_flow_material(&auth_scope, flow_id, &turn_run_ref, gate_ref)
            .await?;
        let expires_at = Utc::now() + ChronoDuration::seconds(DCR_FLOW_TTL_SECONDS);
        let request = NewAuthFlow {
            id: Some(flow_id),
            scope: auth_scope,
            kind: AuthFlowKind::IntegrationCredential,
            provider: AuthProviderId::new(self.spec.provider_id)?,
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: material.authorization_url,
                expires_at,
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref,
                gate_ref: gate_ref.clone(),
            },
            update_binding: None,
            opaque_state_hash: Some(material.opaque_state_hash),
            pkce_verifier_hash: Some(material.pkce_verifier_hash),
            expires_at,
        };
        let flow = match flow_manager.create_flow(request).await {
            Ok(flow) => flow,
            Err(AuthProductError::BackendConflict) => flow_source
                .flow_for_turn_gate(query)
                .await?
                .ok_or(AuthProductError::BackendConflict)?,
            Err(error) => return Err(error),
        };
        if flow.id == flow_id {
            if let Err(error) = self
                .store_flow_material(
                    &flow.scope,
                    flow_id,
                    material.pkce_verifier,
                    &material.client_material,
                )
                .await
            {
                self.cleanup_flow_material(&flow.scope.resource, flow_id)
                    .await?;
                let _ = flow_manager.cancel_flow(&flow.scope, flow_id).await;
                return Err(error);
            }
        }
        challenge_view_from_flow(&flow)
    }

    #[allow(
        dead_code,
        reason = "used by the webui-v2-beta OAuth callback route through RebornProductAuthServices"
    )]
    pub(crate) async fn pkce_verifier_for_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<SecretString>, AuthProductError> {
        let handle = flow_secret_handle(&self.spec, flow_id, "pkce")?;
        match self.load_secret(&scope.resource, &handle).await {
            Ok(value) => Ok(Some(value)),
            Err(AuthProductError::UnknownOrExpiredFlow) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn prepare_flow_material(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        turn_run_ref: &TurnRunRef,
        gate_ref: &AuthGateRef,
    ) -> Result<PreparedDcrFlow, AuthProductError> {
        let metadata = self.discover_authorization_server(&scope.resource).await?;
        let pkce_verifier = SecretString::from(ironclaw_common::pkce::generate_code_verifier());
        let state = ironclaw_common::pkce::generate_code_verifier();
        let redirect_uri = self.callback_redirect_uri(scope, flow_id)?;
        let registration = self
            .register_client(
                &scope.resource,
                &metadata.registration_endpoint,
                &redirect_uri,
            )
            .await?;
        let client_id = OAuthClientId::new(registration.client_id)?;
        let authorization_endpoint =
            OAuthAuthorizationEndpoint::new(metadata.authorization_endpoint.clone())?;
        let state_value = OAuthState::new(state.clone())?;
        let pkce_secret = PkceVerifierSecret::new(SecretString::from(
            pkce_verifier.expose_secret().to_string(),
        ))?;
        let code_challenge = pkce_s256_challenge(&pkce_secret);
        let extra_params = self.authorization_extra_params()?;
        let authorization_url = build_authorization_url(OAuthAuthorizeUrlRequest {
            authorization_endpoint: &authorization_endpoint,
            client_id: &client_id,
            redirect_uri: &redirect_uri,
            state: &state_value,
            code_challenge: &code_challenge,
            scopes: &self.scopes,
            extra_params: &extra_params,
        })?;
        let client_material = StoredDcrClientMaterial {
            client_id: client_id.as_str().to_string(),
            client_secret: registration
                .client_secret
                .map(|secret| secret.expose_secret().to_string()),
            redirect_uri: redirect_uri.as_str().to_string(),
            token_endpoint: metadata.token_endpoint,
        };
        tracing::debug!(
            provider = self.spec.provider_id,
            flow_id = %flow_id,
            turn_run_ref = %turn_run_ref,
            gate_ref = %gate_ref,
            "prepared DCR OAuth material for blocked auth gate"
        );
        Ok(PreparedDcrFlow {
            authorization_url,
            opaque_state_hash: opaque_state_hash(&state)?,
            pkce_verifier_hash: pkce_verifier_hash(&pkce_secret)?,
            pkce_verifier,
            client_material,
        })
    }

    async fn discover_authorization_server(
        &self,
        scope: &ResourceScope,
    ) -> Result<AuthorizationServerMetadata, AuthProductError> {
        let Some(resource) = self.spec.resource else {
            return Err(AuthProductError::BackendUnavailable);
        };
        let resource_metadata_url = protected_resource_metadata_url(resource)?;
        let resource_metadata = self.get_json::<ProtectedResourceMetadata>(
            scope.clone(),
            &resource_metadata_url,
            DCR_RESPONSE_BODY_LIMIT,
        );
        let authorization_server_metadata = match resource_metadata.await {
            Ok(metadata) => metadata
                .authorization_servers
                .into_iter()
                .next()
                .map(|issuer| authorization_server_metadata_url_from_issuer(&issuer))
                .transpose()?
                .ok_or(AuthProductError::BackendUnavailable)?,
            Err(_) => authorization_server_metadata_url(resource)?,
        };
        let metadata = self
            .get_json::<AuthorizationServerMetadata>(
                scope.clone(),
                &authorization_server_metadata,
                DCR_RESPONSE_BODY_LIMIT,
            )
            .await?;
        if metadata.registration_endpoint.trim().is_empty() {
            return Err(AuthProductError::BackendUnavailable);
        }
        Ok(metadata)
    }

    async fn register_client(
        &self,
        scope: &ResourceScope,
        registration_endpoint: &str,
        redirect_uri: &OAuthRedirectUri,
    ) -> Result<DcrRegistrationResponse, AuthProductError> {
        let request = DcrRegistrationRequest {
            client_name: &self.client_name,
            redirect_uris: vec![redirect_uri.as_str()],
            grant_types: vec!["authorization_code", "refresh_token"],
            response_types: vec!["code"],
            token_endpoint_auth_method: "none",
        };
        self.post_json(
            scope.clone(),
            registration_endpoint,
            &request,
            DCR_RESPONSE_BODY_LIMIT,
        )
        .await
    }

    async fn store_flow_material(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        pkce_verifier: SecretString,
        material: &StoredDcrClientMaterial,
    ) -> Result<(), AuthProductError> {
        self.put_secret(
            &scope.resource,
            flow_secret_handle(&self.spec, flow_id, "pkce")?,
            pkce_verifier,
        )
        .await?;
        self.put_material(
            &scope.resource,
            flow_secret_handle(&self.spec, flow_id, "client")?,
            material,
        )
        .await
    }

    async fn load_flow_client_material(
        &self,
        scope: &ResourceScope,
        flow_id: AuthFlowId,
    ) -> Result<OAuthClientMaterial, AuthProductError> {
        let material = self
            .load_material(scope, &flow_secret_handle(&self.spec, flow_id, "client")?)
            .await?;
        material.into_client_material()
    }

    async fn bind_refresh_material(
        &self,
        scope: &ResourceScope,
        flow_id: AuthFlowId,
        refresh_secret: &SecretHandle,
    ) -> Result<(), AuthProductError> {
        let material = self
            .load_material(scope, &flow_secret_handle(&self.spec, flow_id, "client")?)
            .await?;
        self.put_material(
            scope,
            refresh_secret_handle(&self.spec, refresh_secret)?,
            &material,
        )
        .await
    }

    async fn load_refresh_client_material(
        &self,
        scope: &ResourceScope,
        refresh_secret: &SecretHandle,
    ) -> Result<OAuthClientMaterial, AuthProductError> {
        let material = self
            .load_material(scope, &refresh_secret_handle(&self.spec, refresh_secret)?)
            .await?;
        material.into_client_material()
    }

    async fn cleanup_flow_material(
        &self,
        scope: &ResourceScope,
        flow_id: AuthFlowId,
    ) -> Result<(), AuthProductError> {
        let handles = [
            flow_secret_handle(&self.spec, flow_id, "pkce")?,
            flow_secret_handle(&self.spec, flow_id, "client")?,
        ];
        for handle in handles {
            let _ = self.secret_store.delete(scope, &handle).await;
        }
        Ok(())
    }

    fn callback_redirect_uri(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<OAuthRedirectUri, AuthProductError> {
        let mut url = callback_base_url(&self.callback_origin, flow_id)?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("user_id", scope.resource.user_id.as_str());
            query.append_pair("invocation_id", &scope.resource.invocation_id.to_string());
            query.append_pair("provider", self.spec.provider_id);
            query.append_pair("account_label", self.account_label.as_str());
            query.append_pair("scope", &scope_text(&self.scopes));
            if let Some(agent_id) = &scope.resource.agent_id {
                query.append_pair("agent_id", agent_id.as_str());
            }
            if let Some(project_id) = &scope.resource.project_id {
                query.append_pair("project_id", project_id.as_str());
            }
            if let Some(thread_id) = &scope.resource.thread_id {
                query.append_pair("thread_id", thread_id.as_str());
            }
            if let Some(session_id) = &scope.session_id {
                query.append_pair("session_id", session_id.as_str());
            }
        }
        OAuthRedirectUri::new(url.to_string())
    }

    fn authorization_extra_params(&self) -> Result<Vec<OAuthExtraParam>, AuthProductError> {
        self.spec
            .resource
            .map(|resource| OAuthExtraParam::new("resource", resource))
            .transpose()
            .map(|param| param.into_iter().collect())
    }

    async fn get_json<T>(
        &self,
        scope: ResourceScope,
        url: &str,
        response_body_limit: u64,
    ) -> Result<T, AuthProductError>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.execute_json_request(
            scope,
            NetworkMethod::Get,
            url,
            Vec::new(),
            response_body_limit,
        )
        .await
    }

    async fn post_json<T, B>(
        &self,
        scope: ResourceScope,
        url: &str,
        body: &B,
        response_body_limit: u64,
    ) -> Result<T, AuthProductError>
    where
        T: for<'de> Deserialize<'de>,
        B: Serialize,
    {
        let body = serde_json::to_vec(body).map_err(|_| AuthProductError::BackendUnavailable)?;
        self.execute_json_request(scope, NetworkMethod::Post, url, body, response_body_limit)
            .await
    }

    async fn execute_json_request<T>(
        &self,
        scope: ResourceScope,
        method: NetworkMethod,
        url: &str,
        body: Vec<u8>,
        response_body_limit: u64,
    ) -> Result<T, AuthProductError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let host = oauth_endpoint_host(url)?;
        let policy = oauth_network_policy(&host, response_body_limit);
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
                runtime: RuntimeKind::System,
                scope,
                capability_id: self.capability_id.clone(),
                method,
                url: url.to_string(),
                headers: vec![
                    ("accept".to_string(), "application/json".to_string()),
                    ("content-type".to_string(), "application/json".to_string()),
                ],
                body,
                network_policy: policy,
                credential_injections: Vec::new(),
                response_body_limit: Some(response_body_limit),
                save_body_to: None,
                timeout_ms: Some(DCR_TIMEOUT_MS),
            })
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        if !(200..300).contains(&response.status) {
            return Err(AuthProductError::BackendUnavailable);
        }
        serde_json::from_slice(&response.body).map_err(|_| AuthProductError::BackendUnavailable)
    }

    async fn put_material(
        &self,
        scope: &ResourceScope,
        handle: SecretHandle,
        material: &StoredDcrClientMaterial,
    ) -> Result<(), AuthProductError> {
        let encoded =
            serde_json::to_string(material).map_err(|_| AuthProductError::BackendUnavailable)?;
        self.put_secret(scope, handle, SecretString::from(encoded))
            .await
    }

    async fn load_material(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<StoredDcrClientMaterial, AuthProductError> {
        let material = self.load_secret(scope, handle).await?;
        serde_json::from_str(material.expose_secret())
            .map_err(|_| AuthProductError::BackendUnavailable)
    }

    async fn put_secret(
        &self,
        scope: &ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<(), AuthProductError> {
        self.secret_store
            .put(scope.clone(), handle, material)
            .await
            .map(|_| ())
            .map_err(|_| AuthProductError::BackendUnavailable)
    }

    async fn load_secret(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretString, AuthProductError> {
        let lease = self
            .secret_store
            .lease_once(scope, handle)
            .await
            .map_err(|error| {
                if error.is_unknown_secret() {
                    AuthProductError::UnknownOrExpiredFlow
                } else {
                    AuthProductError::BackendUnavailable
                }
            })?;
        self.secret_store
            .consume(scope, lease.id)
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)
    }
}

impl fmt::Debug for OAuthDcrProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthDcrProvider")
            .field("provider_id", &self.spec.provider_id)
            .field("callback_origin", &self.callback_origin)
            .field("client_name", &self.client_name)
            .field("scopes", &self.scopes)
            .finish()
    }
}

#[async_trait]
impl OAuthClientMaterialSource for OAuthDcrProvider {
    async fn exchange_material(
        &self,
        scope: &ResourceScope,
        flow_id: AuthFlowId,
    ) -> Result<OAuthClientMaterial, AuthProductError> {
        self.load_flow_client_material(scope, flow_id).await
    }

    async fn refresh_material(
        &self,
        scope: &ResourceScope,
        refresh_secret: &SecretHandle,
    ) -> Result<OAuthClientMaterial, AuthProductError> {
        self.load_refresh_client_material(scope, refresh_secret)
            .await
    }

    async fn bind_refresh_material(
        &self,
        scope: &ResourceScope,
        flow_id: AuthFlowId,
        refresh_secret: &SecretHandle,
    ) -> Result<(), AuthProductError> {
        OAuthDcrProvider::bind_refresh_material(self, scope, flow_id, refresh_secret).await
    }

    async fn cleanup_exchange_material(
        &self,
        scope: &ResourceScope,
        flow_id: AuthFlowId,
    ) -> Result<(), AuthProductError> {
        self.cleanup_flow_material(scope, flow_id).await
    }
}

#[derive(Clone, Default)]
pub(crate) struct OAuthDcrProviderRegistry {
    providers: BTreeMap<String, Arc<OAuthDcrProvider>>,
}

impl OAuthDcrProviderRegistry {
    pub(crate) fn new(providers: Vec<Arc<OAuthDcrProvider>>) -> Self {
        Self {
            providers: providers
                .into_iter()
                .map(|provider| (provider.spec.provider_id.to_string(), provider))
                .collect(),
        }
    }

    pub(crate) async fn challenge_for_blocked_gate(
        &self,
        flow_manager: &Arc<dyn AuthFlowManager>,
        flow_source: &Arc<dyn AuthFlowRecordSource>,
        requirements: &[RuntimeCredentialAuthRequirement],
        scope: &TurnScope,
        owner_user_id: &ironclaw_host_api::UserId,
        run_id: TurnRunId,
        gate_ref: &AuthGateRef,
    ) -> Result<Option<AuthChallengeView>, AuthProductError> {
        let [requirement] = requirements else {
            return Ok(None);
        };
        let provider = requirement.provider.as_str();
        let Some(dcr_provider) = self.providers.get(provider) else {
            return Ok(None);
        };
        dcr_provider
            .challenge_for_blocked_gate(
                flow_manager,
                flow_source,
                scope,
                owner_user_id,
                run_id,
                gate_ref,
            )
            .await
            .map(Some)
    }

    #[allow(
        dead_code,
        reason = "used by the webui-v2-beta OAuth callback route through RebornProductAuthServices"
    )]
    pub(crate) async fn pkce_verifier_for_flow(
        &self,
        scope: &AuthProductScope,
        provider: &AuthProviderId,
        flow_id: AuthFlowId,
    ) -> Result<Option<SecretString>, AuthProductError> {
        let Some(dcr_provider) = self.providers.get(provider.as_str()) else {
            return Ok(None);
        };
        dcr_provider.pkce_verifier_for_flow(scope, flow_id).await
    }
}

impl fmt::Debug for OAuthDcrProviderRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthDcrProviderRegistry")
            .field("providers", &self.providers.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Debug)]
struct PreparedDcrFlow {
    authorization_url: ironclaw_auth::OAuthAuthorizationUrl,
    opaque_state_hash: ironclaw_auth::OpaqueStateHash,
    pkce_verifier_hash: ironclaw_auth::PkceVerifierHash,
    pkce_verifier: SecretString,
    client_material: StoredDcrClientMaterial,
}

fn auth_scope_for_blocked_turn(
    scope: &TurnScope,
    owner_user_id: &ironclaw_host_api::UserId,
) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope {
            tenant_id: scope.tenant_id.clone(),
            user_id: owner_user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(scope.thread_id.clone()),
            invocation_id: InvocationId::new(),
        },
        ironclaw_auth::AuthSurface::Callback,
    )
}

fn challenge_view_from_flow(
    flow: &ironclaw_auth::AuthFlowRecord,
) -> Result<AuthChallengeView, AuthProductError> {
    let Some(AuthChallenge::OAuthUrl {
        authorization_url,
        expires_at,
    }) = &flow.challenge
    else {
        return Err(AuthProductError::BackendUnavailable);
    };
    Ok(AuthChallengeView {
        kind: AuthPromptChallengeKind::OAuthUrl,
        provider: flow.provider.clone(),
        account_label: None,
        authorization_url: Some(authorization_url.clone()),
        expires_at: Some(*expires_at),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dcr_provider_creates_blocked_gate_flow_and_stores_pkce_material() {
        let provider = OAuthDcrProvider::new(
            OAuthDcrProviderConfig {
                spec: HostOAuthProviderSpec {
                    provider_id: "notion",
                    capability_id: "ironclaw_auth.notion_oauth",
                    token_endpoint: "https://mcp.notion.com/token",
                    secret_handle_prefix: "notion",
                    resource: Some("https://mcp.notion.com/mcp"),
                    exchange_scope_policy:
                        crate::oauth_provider_client::ExchangeScopePolicy::FallbackToRequested,
                },
                callback_origin: "http://127.0.0.1:3000".to_string(),
                client_name: "Ironclaw".to_string(),
                account_label: CredentialAccountLabel::new("notion").unwrap(),
                scopes: Vec::new(),
            },
            Arc::new(DcrSetupEgress),
            Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
            Arc::new(TestObligationHandler),
        )
        .unwrap();
        let auth = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let flow_manager: Arc<dyn AuthFlowManager> = auth.clone();
        let flow_source: Arc<dyn AuthFlowRecordSource> = auth.clone();
        let scope = TurnScope::new(
            ironclaw_host_api::TenantId::new("tenant").unwrap(),
            Some(ironclaw_host_api::AgentId::new("agent").unwrap()),
            Some(ironclaw_host_api::ProjectId::new("project").unwrap()),
            ironclaw_host_api::ThreadId::new("thread").unwrap(),
        );
        let owner = ironclaw_host_api::UserId::new("user").unwrap();
        let run_id = TurnRunId::new();
        let gate_ref =
            AuthGateRef::new("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string()).unwrap();

        let view = provider
            .challenge_for_blocked_gate(
                &flow_manager,
                &flow_source,
                &scope,
                &owner,
                run_id,
                &gate_ref,
            )
            .await
            .unwrap();

        assert!(matches!(view.kind, AuthPromptChallengeKind::OAuthUrl));
        assert_eq!(view.provider.as_str(), "notion");
        let authorization_url = view.authorization_url.unwrap();
        assert!(authorization_url.as_str().contains("client_id=dcr-client"));
        assert!(
            authorization_url
                .as_str()
                .contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A3000")
        );

        let flow = flow_source
            .flow_for_turn_gate(TurnGateAuthFlowQuery {
                owner: AuthFlowOwnerScope {
                    tenant_id: scope.tenant_id.clone(),
                    user_id: owner.clone(),
                    agent_id: scope.agent_id.clone(),
                    project_id: scope.project_id.clone(),
                    thread_id: scope.thread_id.clone(),
                },
                turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
                gate_ref,
                include_terminal: false,
            })
            .await
            .unwrap()
            .expect("flow");

        let pkce = provider
            .pkce_verifier_for_flow(&flow.scope, flow.id)
            .await
            .unwrap();
        assert!(pkce.is_some());
    }

    #[test]
    fn callback_redirect_uri_carries_existing_callback_query_fields() {
        let provider = OAuthDcrProvider::new(
            OAuthDcrProviderConfig {
                spec: HostOAuthProviderSpec {
                    provider_id: "notion",
                    capability_id: "ironclaw_auth.notion_oauth",
                    token_endpoint: "https://mcp.notion.com/token",
                    secret_handle_prefix: "notion",
                    resource: Some("https://mcp.notion.com/mcp"),
                    exchange_scope_policy:
                        crate::oauth_provider_client::ExchangeScopePolicy::FallbackToRequested,
                },
                callback_origin: "http://127.0.0.1:3000".to_string(),
                client_name: "Ironclaw".to_string(),
                account_label: CredentialAccountLabel::new("notion").unwrap(),
                scopes: vec![ProviderScope::new("read").unwrap()],
            },
            Arc::new(TestEgress),
            Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
            Arc::new(TestObligationHandler),
        )
        .unwrap();
        let scope = AuthProductScope::new(
            ResourceScope {
                tenant_id: ironclaw_host_api::TenantId::new("tenant").unwrap(),
                user_id: ironclaw_host_api::UserId::new("user").unwrap(),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: Some(ironclaw_host_api::ThreadId::new("thread").unwrap()),
                invocation_id: InvocationId::new(),
            },
            ironclaw_auth::AuthSurface::Callback,
        );

        let redirect = provider
            .callback_redirect_uri(&scope, AuthFlowId::from_uuid(uuid::Uuid::nil()))
            .unwrap();

        assert!(
            redirect
                .as_str()
                .contains("/api/reborn/product-auth/oauth/callback/")
        );
        assert!(redirect.as_str().contains("provider=notion"));
        assert!(redirect.as_str().contains("account_label=notion"));
        assert!(redirect.as_str().contains("scope=read"));
    }

    #[derive(Debug)]
    struct DcrSetupEgress;

    #[async_trait]
    impl RuntimeHttpEgress for DcrSetupEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<
            ironclaw_host_api::RuntimeHttpEgressResponse,
            ironclaw_host_api::RuntimeHttpEgressError,
        > {
            let body = match request.url.as_str() {
                "https://mcp.notion.com/mcp/.well-known/oauth-protected-resource" => {
                    br#"{"authorization_servers":["https://issuer.example"]}"#.to_vec()
                }
                "https://issuer.example/.well-known/oauth-authorization-server" => {
                    br#"{"authorization_endpoint":"https://issuer.example/authorize","token_endpoint":"https://issuer.example/token","registration_endpoint":"https://issuer.example/register"}"#.to_vec()
                }
                "https://issuer.example/register" => br#"{"client_id":"dcr-client"}"#.to_vec(),
                other => panic!("unexpected DCR egress URL: {other}"),
            };
            Ok(ironclaw_host_api::RuntimeHttpEgressResponse {
                status: 200,
                headers: Vec::new(),
                request_bytes: request.body.len() as u64,
                response_bytes: body.len() as u64,
                body,
                saved_body: None,
                redaction_applied: false,
            })
        }
    }

    #[derive(Debug)]
    struct TestEgress;

    #[async_trait]
    impl RuntimeHttpEgress for TestEgress {
        async fn execute(
            &self,
            _request: RuntimeHttpEgressRequest,
        ) -> Result<
            ironclaw_host_api::RuntimeHttpEgressResponse,
            ironclaw_host_api::RuntimeHttpEgressError,
        > {
            panic!("test egress should not execute")
        }
    }

    #[derive(Debug)]
    struct TestObligationHandler;

    #[async_trait]
    impl CapabilityObligationHandler for TestObligationHandler {
        async fn satisfy(
            &self,
            _request: ironclaw_capabilities::CapabilityObligationRequest<'_>,
        ) -> Result<(), ironclaw_capabilities::CapabilityObligationError> {
            Ok(())
        }
    }
}

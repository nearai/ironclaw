use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowId, AuthFlowKind, AuthFlowManager,
    AuthFlowOwnerScope, AuthFlowRecordSource, AuthGateRef, AuthProductError, AuthProductScope,
    AuthProviderId, CredentialAccountLabel, CredentialAccountUpdateBinding, NewAuthFlow,
    OAuthAuthorizationEndpoint, OAuthAuthorizeUrlRequest, OAuthClientId, OAuthExtraParam,
    OAuthRedirectUri, OAuthState, PkceVerifierSecret, ProviderScope, TurnGateAuthFlowQuery,
    TurnRunRef, build_authorization_url, opaque_state_hash, pkce_s256_challenge,
    pkce_verifier_hash,
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
use tokio::sync::{Mutex as AsyncMutex, RwLock};

use crate::AuthChallengeView;
use crate::oauth_dcr_protocol::{
    AuthorizationServerMetadata, DcrRegistrationRequest, DcrRegistrationResponse,
    ProtectedResourceMetadata, StoredDcrClientMaterial, authorization_server_metadata_url,
    authorization_server_metadata_url_from_issuer, callback_base_url, flow_secret_handle,
    protected_resource_metadata_url, refresh_secret_handle, scope_text, validate_callback_origin,
    validate_endpoint_origin, validate_issuer_related_to_resource,
};
use crate::oauth_provider_client::{
    HostOAuthProviderSpec, OAuthClientMaterial, OAuthClientMaterialSource, authorize_oauth_egress,
    oauth_endpoint_host, oauth_network_policy,
};

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
    metadata_cache: Arc<RwLock<Option<CachedAuthorizationServerMetadata>>>,
    setup_lock: Arc<AsyncMutex<()>>,
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
            metadata_cache: Arc::new(RwLock::new(None)),
            setup_lock: Arc::new(AsyncMutex::new(())),
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

        let _setup_guard = self.setup_lock.lock().await;
        if let Some(existing) = flow_source.flow_for_turn_gate(query.clone()).await? {
            return challenge_view_from_flow(&existing);
        }

        let flow_id = AuthFlowId::new();
        let material = self
            .prepare_flow_material(
                &auth_scope,
                flow_id,
                DcrFlowContext::BlockedGate {
                    turn_run_ref: &turn_run_ref,
                    gate_ref,
                },
            )
            .await?;
        let expires_at = Utc::now() + ChronoDuration::seconds(DCR_FLOW_TTL_SECONDS);
        let request = NewAuthFlow {
            id: Some(flow_id),
            scope: auth_scope.clone(),
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
            Err(AuthProductError::BackendConflict) => {
                self.cleanup_registered_client(&auth_scope.resource, &material.registration)
                    .await;
                flow_source
                    .flow_for_turn_gate(query)
                    .await?
                    .ok_or(AuthProductError::BackendConflict)?
            }
            Err(error) => {
                self.cleanup_registered_client(&auth_scope.resource, &material.registration)
                    .await;
                return Err(error);
            }
        };
        if flow.id == flow_id
            && let Err(error) = self
                .store_flow_material(
                    &flow.scope,
                    flow_id,
                    material.pkce_verifier,
                    &material.client_material,
                )
                .await
        {
            self.cleanup_registered_client(&flow.scope.resource, &material.registration)
                .await;
            if self
                .cleanup_flow_material(&flow.scope.resource, flow_id)
                .await
                .is_err()
            {
                tracing::warn!(
                    provider = self.spec.provider_id,
                    flow_id = %flow_id,
                    cleanup_kind = "flow_material",
                    "failed to clean up DCR flow material after storage failure"
                );
            }
            if flow_manager
                .cancel_flow(&flow.scope, flow_id)
                .await
                .is_err()
            {
                tracing::warn!(
                    provider = self.spec.provider_id,
                    flow_id = %flow_id,
                    cleanup_kind = "cancel_flow",
                    "failed to cancel DCR flow after storage failure"
                );
            }
            return Err(error);
        }
        challenge_view_from_flow(&flow)
    }

    #[allow(
        dead_code,
        reason = "used by the webui-v2-beta extension OAuth route through RebornProductAuthServices"
    )]
    pub(crate) async fn start_setup_flow(
        &self,
        flow_manager: &Arc<dyn AuthFlowManager>,
        scope: AuthProductScope,
        account_label: CredentialAccountLabel,
        provider_scopes: &[ProviderScope],
        update_binding: Option<CredentialAccountUpdateBinding>,
        expires_at: ironclaw_auth::Timestamp,
    ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
        if provider_scopes != self.scopes.as_slice() {
            return Err(AuthProductError::BackendUnavailable);
        }
        let flow_id = AuthFlowId::new();
        let material = self
            .prepare_flow_material(
                &scope,
                flow_id,
                DcrFlowContext::Setup {
                    account_label: &account_label,
                },
            )
            .await?;
        let request = NewAuthFlow {
            id: Some(flow_id),
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: AuthProviderId::new(self.spec.provider_id)?,
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: material.authorization_url,
                expires_at,
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding,
            opaque_state_hash: Some(material.opaque_state_hash),
            pkce_verifier_hash: Some(material.pkce_verifier_hash),
            expires_at,
        };
        let flow = match flow_manager.create_flow(request).await {
            Ok(flow) => flow,
            Err(error) => {
                self.cleanup_registered_client(&scope.resource, &material.registration)
                    .await;
                return Err(error);
            }
        };
        if let Err(error) = self
            .store_flow_material(
                &flow.scope,
                flow_id,
                material.pkce_verifier,
                &material.client_material,
            )
            .await
        {
            self.cleanup_registered_client(&flow.scope.resource, &material.registration)
                .await;
            if self
                .cleanup_flow_material(&flow.scope.resource, flow_id)
                .await
                .is_err()
            {
                tracing::warn!(
                    provider = self.spec.provider_id,
                    flow_id = %flow_id,
                    cleanup_kind = "flow_material",
                    "failed to clean up DCR setup flow material after storage failure"
                );
            }
            if flow_manager
                .cancel_flow(&flow.scope, flow_id)
                .await
                .is_err()
            {
                tracing::warn!(
                    provider = self.spec.provider_id,
                    flow_id = %flow_id,
                    cleanup_kind = "cancel_flow",
                    "failed to cancel DCR setup flow after storage failure"
                );
            }
            return Err(error);
        }
        Ok(flow)
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
        context: DcrFlowContext<'_>,
    ) -> Result<PreparedDcrFlow, AuthProductError> {
        let metadata = self.discover_authorization_server(&scope.resource).await?;
        let pkce_verifier = SecretString::from(ironclaw_common::pkce::generate_code_verifier());
        let account_label = match context {
            DcrFlowContext::BlockedGate { .. } => &self.account_label,
            DcrFlowContext::Setup { account_label } => account_label,
        };
        let provider = AuthProviderId::new(self.spec.provider_id)?;
        let state_value = DcrOAuthCallbackState::new(
            flow_id,
            scope.clone(),
            provider,
            account_label.clone(),
            self.scopes.clone(),
        )
        .encode()?;
        let redirect_uri = self.callback_redirect_uri(scope, flow_id, account_label)?;
        let registration = self
            .register_client(
                &scope.resource,
                &metadata.registration_endpoint,
                &redirect_uri,
            )
            .await?;
        let client_id = OAuthClientId::new(registration.client_id.clone())?;
        let authorization_endpoint =
            OAuthAuthorizationEndpoint::new(metadata.authorization_endpoint.clone())?;
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
            client_secret: None,
            redirect_uri: redirect_uri.as_str().to_string(),
            token_endpoint: metadata.token_endpoint,
        };
        match context {
            DcrFlowContext::BlockedGate {
                turn_run_ref,
                gate_ref,
            } => {
                tracing::debug!(
                    provider = self.spec.provider_id,
                    flow_id = %flow_id,
                    turn_run_ref = %turn_run_ref,
                    gate_ref = %gate_ref,
                    "prepared DCR OAuth material for blocked auth gate"
                );
            }
            DcrFlowContext::Setup { .. } => {
                tracing::debug!(
                    provider = self.spec.provider_id,
                    flow_id = %flow_id,
                    "prepared DCR OAuth material for setup flow"
                );
            }
        }
        Ok(PreparedDcrFlow {
            authorization_url,
            opaque_state_hash: opaque_state_hash(state_value.as_str())?,
            pkce_verifier_hash: pkce_verifier_hash(&pkce_secret)?,
            pkce_verifier,
            client_material,
            registration,
        })
    }

    async fn discover_authorization_server(
        &self,
        scope: &ResourceScope,
    ) -> Result<AuthorizationServerMetadata, AuthProductError> {
        if let Some(cached) = self.metadata_cache.read().await.as_ref()
            && cached.expires_at > Instant::now()
        {
            return Ok(cached.metadata.clone());
        }
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
            Ok(metadata) => {
                let issuer = metadata
                    .authorization_servers
                    .into_iter()
                    .next()
                    .ok_or(AuthProductError::BackendUnavailable)?;
                validate_issuer_related_to_resource(resource, &issuer)?;
                authorization_server_metadata_url_from_issuer(&issuer)?
            }
            Err(error) => {
                tracing::debug!(
                    provider = self.spec.provider_id,
                    reason = ?error.code(),
                    "DCR protected-resource metadata unavailable; falling back to issuer well-known URL"
                );
                authorization_server_metadata_url(resource)?
            }
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
        validate_endpoint_origin(
            &metadata.authorization_endpoint,
            &authorization_server_metadata,
        )?;
        validate_endpoint_origin(&metadata.token_endpoint, &authorization_server_metadata)?;
        validate_endpoint_origin(
            &metadata.registration_endpoint,
            &authorization_server_metadata,
        )?;
        *self.metadata_cache.write().await = Some(CachedAuthorizationServerMetadata {
            metadata: metadata.clone(),
            expires_at: Instant::now() + Duration::from_secs(DCR_FLOW_TTL_SECONDS as u64),
        });
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
        let mut first_error = None;
        for handle in handles {
            if let Err(error) = self.secret_store.delete(scope, &handle).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), |_| Err(AuthProductError::BackendUnavailable))
    }

    async fn cleanup_registered_client(
        &self,
        scope: &ResourceScope,
        registration: &DcrRegistrationResponse,
    ) {
        let Some(registration_client_uri) = registration.registration_client_uri.as_deref() else {
            return;
        };
        let Some(registration_access_token) = registration.registration_access_token.as_ref()
        else {
            return;
        };
        if self
            .execute_json_request::<serde_json::Value>(
                scope.clone(),
                NetworkMethod::Delete,
                registration_client_uri,
                Vec::new(),
                DCR_RESPONSE_BODY_LIMIT,
                Some(registration_access_token),
            )
            .await
            .is_err()
        {
            tracing::warn!(
                provider = self.spec.provider_id,
                cleanup_kind = "dcr_registration",
                "failed to deregister orphaned DCR client after flow failure"
            );
        }
    }

    fn callback_redirect_uri(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        account_label: &CredentialAccountLabel,
    ) -> Result<OAuthRedirectUri, AuthProductError> {
        let mut url = callback_base_url(&self.callback_origin, flow_id)?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("user_id", scope.resource.user_id.as_str());
            query.append_pair("invocation_id", &scope.resource.invocation_id.to_string());
            query.append_pair("provider", self.spec.provider_id);
            query.append_pair("account_label", account_label.as_str());
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
            None,
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
        self.execute_json_request(
            scope,
            NetworkMethod::Post,
            url,
            body,
            response_body_limit,
            None,
        )
        .await
    }

    async fn execute_json_request<T>(
        &self,
        scope: ResourceScope,
        method: NetworkMethod,
        url: &str,
        body: Vec<u8>,
        response_body_limit: u64,
        bearer_token: Option<&SecretString>,
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
        let mut headers = vec![
            ("accept".to_string(), "application/json".to_string()),
            ("content-type".to_string(), "application/json".to_string()),
        ];
        if let Some(token) = bearer_token {
            headers.push((
                "authorization".to_string(),
                format!("Bearer {}", token.expose_secret()),
            ));
        }
        let response = self
            .egress
            .execute(RuntimeHttpEgressRequest {
                runtime: RuntimeKind::System,
                scope,
                capability_id: self.capability_id.clone(),
                method,
                url: url.to_string(),
                headers,
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
        request: DcrGateChallengeRequest<'_>,
    ) -> Result<Option<AuthChallengeView>, AuthProductError> {
        let DcrGateChallengeRequest {
            flow_manager,
            flow_source,
            requirements,
            scope,
            owner_user_id,
            run_id,
            gate_ref,
        } = request;
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

    #[allow(
        dead_code,
        reason = "used by the webui-v2-beta extension OAuth route through RebornProductAuthServices"
    )]
    pub(crate) async fn start_setup_flow(
        &self,
        flow_manager: &Arc<dyn AuthFlowManager>,
        request: DcrSetupFlowRequest,
    ) -> Result<Option<ironclaw_auth::AuthFlowRecord>, AuthProductError> {
        let DcrSetupFlowRequest {
            scope,
            provider,
            account_label,
            provider_scopes,
            update_binding,
            expires_at,
        } = request;
        let Some(dcr_provider) = self.providers.get(provider.as_str()) else {
            return Ok(None);
        };
        dcr_provider
            .start_setup_flow(
                flow_manager,
                scope,
                account_label,
                &provider_scopes,
                update_binding,
                expires_at,
            )
            .await
            .map(Some)
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

enum DcrFlowContext<'a> {
    BlockedGate {
        turn_run_ref: &'a TurnRunRef,
        gate_ref: &'a AuthGateRef,
    },
    #[allow(
        dead_code,
        reason = "used by the webui-v2-beta extension OAuth route through RebornProductAuthServices"
    )]
    Setup {
        account_label: &'a CredentialAccountLabel,
    },
}

#[derive(Debug)]
struct PreparedDcrFlow {
    authorization_url: ironclaw_auth::OAuthAuthorizationUrl,
    opaque_state_hash: ironclaw_auth::OpaqueStateHash,
    pkce_verifier_hash: ironclaw_auth::PkceVerifierHash,
    pkce_verifier: SecretString,
    client_material: StoredDcrClientMaterial,
    registration: DcrRegistrationResponse,
}

#[derive(Debug, Clone)]
struct CachedAuthorizationServerMetadata {
    metadata: AuthorizationServerMetadata,
    expires_at: Instant,
}

pub(crate) struct DcrGateChallengeRequest<'a> {
    pub(crate) flow_manager: &'a Arc<dyn AuthFlowManager>,
    pub(crate) flow_source: &'a Arc<dyn AuthFlowRecordSource>,
    pub(crate) requirements: &'a [RuntimeCredentialAuthRequirement],
    pub(crate) scope: &'a TurnScope,
    pub(crate) owner_user_id: &'a ironclaw_host_api::UserId,
    pub(crate) run_id: TurnRunId,
    pub(crate) gate_ref: &'a AuthGateRef,
}

pub(crate) struct DcrSetupFlowRequest {
    pub(crate) scope: AuthProductScope,
    pub(crate) provider: AuthProviderId,
    pub(crate) account_label: CredentialAccountLabel,
    pub(crate) provider_scopes: Vec<ProviderScope>,
    pub(crate) update_binding: Option<CredentialAccountUpdateBinding>,
    pub(crate) expires_at: ironclaw_auth::Timestamp,
}

#[derive(Debug, Clone)]
pub(crate) struct DcrOAuthCallbackState {
    flow_id: AuthFlowId,
    scope: AuthProductScope,
    provider: AuthProviderId,
    account_label: CredentialAccountLabel,
    requested_scopes: Vec<ProviderScope>,
    nonce: String,
}

#[derive(Serialize, Deserialize)]
struct DcrOAuthCallbackStateWire {
    flow_id: AuthFlowId,
    resource: ResourceScope,
    session_id: Option<ironclaw_auth::AuthSessionId>,
    provider: AuthProviderId,
    account_label: CredentialAccountLabel,
    requested_scopes: Vec<ProviderScope>,
    nonce: String,
}

#[allow(dead_code)] // Used by the optional product-auth route surface; all-features test targets can compile this module without that route.
impl DcrOAuthCallbackState {
    const PREFIX: &'static str = "icd1.";

    pub(crate) fn new(
        flow_id: AuthFlowId,
        scope: AuthProductScope,
        provider: AuthProviderId,
        account_label: CredentialAccountLabel,
        requested_scopes: Vec<ProviderScope>,
    ) -> Self {
        Self {
            flow_id,
            scope,
            provider,
            account_label,
            requested_scopes,
            nonce: ironclaw_common::pkce::generate_code_verifier(),
        }
    }

    pub(crate) fn encode(&self) -> Result<OAuthState, AuthProductError> {
        let wire = DcrOAuthCallbackStateWire {
            flow_id: self.flow_id,
            resource: self.scope.resource.clone(),
            session_id: self.scope.session_id.clone(),
            provider: self.provider.clone(),
            account_label: self.account_label.clone(),
            requested_scopes: self.requested_scopes.clone(),
            nonce: self.nonce.clone(),
        };
        let payload =
            serde_json::to_vec(&wire).map_err(|_| AuthProductError::BackendUnavailable)?;
        OAuthState::new(format!(
            "{}{}",
            Self::PREFIX,
            URL_SAFE_NO_PAD.encode(payload)
        ))
    }
}

#[cfg(any(test, feature = "webui-v2-beta"))]
impl DcrOAuthCallbackState {
    pub(crate) fn has_prefix(raw: &str) -> bool {
        raw.starts_with(Self::PREFIX)
    }

    pub(crate) fn flow_id(&self) -> AuthFlowId {
        self.flow_id
    }

    pub(crate) fn scope(&self) -> &AuthProductScope {
        &self.scope
    }

    pub(crate) fn provider(&self) -> &AuthProviderId {
        &self.provider
    }

    pub(crate) fn account_label(&self) -> &CredentialAccountLabel {
        &self.account_label
    }

    pub(crate) fn requested_scopes(&self) -> &[ProviderScope] {
        &self.requested_scopes
    }

    pub(crate) fn decode(raw: &str) -> Result<Self, AuthProductError> {
        let encoded = raw
            .strip_prefix(Self::PREFIX)
            .ok_or(AuthProductError::MalformedCallback)?;
        let payload = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| AuthProductError::MalformedCallback)?;
        let wire: DcrOAuthCallbackStateWire =
            serde_json::from_slice(&payload).map_err(|_| AuthProductError::MalformedCallback)?;
        OAuthState::new(wire.nonce.clone()).map_err(|_| AuthProductError::MalformedCallback)?;
        let mut scope = AuthProductScope::new(wire.resource, ironclaw_auth::AuthSurface::Callback);
        if let Some(session_id) = wire.session_id {
            scope = scope.with_session_id(session_id);
        }
        Ok(Self {
            flow_id: wire.flow_id,
            scope,
            provider: wire.provider,
            account_label: wire.account_label,
            requested_scopes: wire.requested_scopes,
            nonce: wire.nonce,
        })
    }
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
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    #[tokio::test]
    async fn dcr_provider_creates_setup_only_flow_and_stores_pkce_material() {
        let provider = test_provider(Arc::new(DcrSetupEgress));
        let auth = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let flow_manager: Arc<dyn AuthFlowManager> = auth.clone();

        let flow = provider
            .start_setup_flow(
                &flow_manager,
                sample_auth_scope(),
                CredentialAccountLabel::new("work notion").unwrap(),
                &[],
                None,
                Utc::now() + ChronoDuration::seconds(DCR_FLOW_TTL_SECONDS),
            )
            .await
            .unwrap();

        assert_eq!(flow.provider.as_str(), "notion");
        assert!(matches!(flow.continuation, AuthContinuationRef::SetupOnly));
        let Some(AuthChallenge::OAuthUrl {
            authorization_url, ..
        }) = &flow.challenge
        else {
            panic!("setup flow should render an OAuth URL challenge");
        };
        assert!(authorization_url.as_str().contains("client_id=dcr-client"));
        let parsed = url::Url::parse(authorization_url.as_str()).unwrap();
        let redirect_uri = parsed
            .query_pairs()
            .find_map(|(name, value)| (name == "redirect_uri").then(|| value.into_owned()))
            .expect("redirect uri");
        let redirect = url::Url::parse(&redirect_uri).unwrap();
        assert_eq!(
            redirect
                .query_pairs()
                .find_map(|(name, value)| (name == "account_label").then(|| value.into_owned())),
            Some("work notion".to_string())
        );
        assert!(
            authorization_url
                .as_str()
                .contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A3000")
        );
        let pkce = provider
            .pkce_verifier_for_flow(&flow.scope, flow.id)
            .await
            .unwrap();
        assert!(pkce.is_some());
    }

    #[tokio::test]
    async fn dcr_provider_setup_flow_rejects_scope_mismatch() {
        let provider = test_provider(Arc::new(DcrSetupEgress));
        let auth = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let flow_manager: Arc<dyn AuthFlowManager> = auth.clone();

        let error = provider
            .start_setup_flow(
                &flow_manager,
                sample_auth_scope(),
                CredentialAccountLabel::new("work notion").unwrap(),
                &[ProviderScope::new("read").unwrap()],
                None,
                Utc::now() + ChronoDuration::seconds(DCR_FLOW_TTL_SECONDS),
            )
            .await
            .expect_err("scope mismatch should be rejected before registration");

        assert_eq!(
            error.code(),
            ironclaw_auth::AuthErrorCode::BackendUnavailable
        );
        assert!(
            auth.flows_for_owner(sample_flow_owner())
                .await
                .unwrap()
                .is_empty(),
            "scope mismatch must not create a setup flow"
        );
    }

    #[test]
    fn dcr_oauth_callback_state_round_trips_callback_fields() {
        let scope = sample_auth_scope();
        let provider = AuthProviderId::new("notion").unwrap();
        let account_label = CredentialAccountLabel::new("work notion").unwrap();
        let requested_scopes = vec![ProviderScope::new("read").unwrap()];
        let state = DcrOAuthCallbackState::new(
            AuthFlowId::new(),
            scope.clone(),
            provider.clone(),
            account_label.clone(),
            requested_scopes.clone(),
        );

        let encoded = state.encode().expect("encoded DCR callback state");
        let decoded = DcrOAuthCallbackState::decode(encoded.as_str())
            .expect("encoded DCR callback state should decode");

        assert!(DcrOAuthCallbackState::has_prefix(encoded.as_str()));
        assert_eq!(decoded.flow_id(), state.flow_id());
        assert_eq!(decoded.scope(), &scope);
        assert_eq!(decoded.provider(), &provider);
        assert_eq!(decoded.account_label(), &account_label);
        assert_eq!(decoded.requested_scopes(), requested_scopes.as_slice());
    }

    #[test]
    fn dcr_oauth_callback_state_rejects_missing_prefix_or_corrupt_payload() {
        let missing_prefix = DcrOAuthCallbackState::decode("not-dcr-state")
            .expect_err("missing DCR prefix should fail");
        let corrupt_payload =
            DcrOAuthCallbackState::decode("icd1.not-base64").expect_err("corrupt payload fails");

        assert_eq!(
            missing_prefix.code(),
            ironclaw_auth::AuthErrorCode::MalformedCallback
        );
        assert_eq!(
            corrupt_payload.code(),
            ironclaw_auth::AuthErrorCode::MalformedCallback
        );
    }

    #[tokio::test]
    async fn discover_authorization_server_empty_authorization_servers_returns_backend_unavailable()
    {
        let provider = test_provider(Arc::new(DcrDiscoveryEgress::empty_authorization_servers()));
        let error = provider
            .discover_authorization_server(&sample_resource_scope())
            .await
            .expect_err("empty authorization_servers must fail");

        assert_eq!(
            error.code(),
            ironclaw_auth::AuthErrorCode::BackendUnavailable
        );
    }

    #[tokio::test]
    async fn discover_authorization_server_falls_back_to_issuer_url_when_resource_metadata_fails() {
        let provider = test_provider(Arc::new(DcrDiscoveryEgress::resource_metadata_fails()));

        let metadata = provider
            .discover_authorization_server(&sample_resource_scope())
            .await
            .expect("issuer fallback metadata");

        assert_eq!(
            metadata.registration_endpoint,
            "https://mcp.notion.com/register"
        );
    }

    #[tokio::test]
    async fn discover_authorization_server_empty_registration_endpoint_returns_backend_unavailable()
    {
        let provider = test_provider(Arc::new(DcrDiscoveryEgress::empty_registration_endpoint()));

        let error = provider
            .discover_authorization_server(&sample_resource_scope())
            .await
            .expect_err("empty registration endpoint must fail");

        assert_eq!(
            error.code(),
            ironclaw_auth::AuthErrorCode::BackendUnavailable
        );
    }

    #[tokio::test]
    async fn store_flow_material_cleanup_runs_when_client_material_write_fails() {
        let secret_store = Arc::new(SecondPutFailingSecretStore::new());
        let provider = OAuthDcrProvider::new(
            test_config(),
            Arc::new(DcrSetupEgress),
            secret_store.clone(),
            Arc::new(TestObligationHandler),
        )
        .unwrap();
        let auth = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let flow_manager: Arc<dyn AuthFlowManager> = auth.clone();
        let flow_source: Arc<dyn AuthFlowRecordSource> = auth.clone();
        let scope = sample_turn_scope();
        let owner = ironclaw_host_api::UserId::new("user").unwrap();
        let run_id = TurnRunId::new();
        let gate_ref =
            AuthGateRef::new("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string()).unwrap();

        let error = provider
            .challenge_for_blocked_gate(
                &flow_manager,
                &flow_source,
                &scope,
                &owner,
                run_id,
                &gate_ref,
            )
            .await
            .expect_err("second secret write fails");

        assert_eq!(
            error.code(),
            ironclaw_auth::AuthErrorCode::BackendUnavailable
        );
        let put_handles = secret_store.put_handles();
        assert_eq!(
            put_handles.len(),
            2,
            "PKCE and client material put attempted"
        );
        let deleted_handles = secret_store.deleted_handles();
        assert_eq!(
            deleted_handles, put_handles,
            "rollback must delete both flow-scoped handles"
        );
        assert!(
            secret_store
                .metadata(&sample_auth_scope().resource, &put_handles[0])
                .await
                .unwrap()
                .is_none(),
            "PKCE material written before client failure must be removed"
        );
        assert!(
            flow_source
                .flow_for_turn_gate(TurnGateAuthFlowQuery {
                    owner: AuthFlowOwnerScope {
                        tenant_id: scope.tenant_id.clone(),
                        user_id: owner,
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
                .is_none(),
            "failed DCR storage must cancel the newly created flow"
        );
    }

    #[tokio::test]
    async fn dcr_provider_setup_flow_cleans_up_material_and_cancels_flow_when_client_material_write_fails()
     {
        let secret_store = Arc::new(SecondPutFailingSecretStore::new());
        let provider = OAuthDcrProvider::new(
            test_config(),
            Arc::new(DcrSetupEgress),
            secret_store.clone(),
            Arc::new(TestObligationHandler),
        )
        .unwrap();
        let auth = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let flow_manager: Arc<dyn AuthFlowManager> = auth.clone();
        let scope = sample_auth_scope();

        let error = provider
            .start_setup_flow(
                &flow_manager,
                scope.clone(),
                CredentialAccountLabel::new("work notion").unwrap(),
                &[],
                None,
                Utc::now() + ChronoDuration::seconds(DCR_FLOW_TTL_SECONDS),
            )
            .await
            .expect_err("second secret write fails");

        assert_eq!(
            error.code(),
            ironclaw_auth::AuthErrorCode::BackendUnavailable
        );
        let put_handles = secret_store.put_handles();
        assert_eq!(
            put_handles.len(),
            2,
            "PKCE and client material put attempted"
        );
        let deleted_handles = secret_store.deleted_handles();
        assert_eq!(
            deleted_handles, put_handles,
            "rollback must delete both setup flow handles"
        );
        let flows = auth.flows_for_owner(sample_flow_owner()).await.unwrap();
        assert_eq!(flows.len(), 1, "rollback should leave one terminal flow");
        assert_eq!(
            flows[0].status,
            ironclaw_auth::AuthFlowStatus::Canceled,
            "failed DCR setup storage must cancel the newly created flow"
        );
        assert!(
            secret_store
                .metadata(&scope.resource, &put_handles[0])
                .await
                .unwrap()
                .is_none(),
            "PKCE material written before client failure must be removed"
        );
    }

    #[tokio::test]
    async fn dcr_provider_cleans_up_registered_client_when_blocked_gate_create_flow_fails() {
        let egress = Arc::new(RecordingDcrSetupEgress::new());
        let provider = OAuthDcrProvider::new(
            test_config(),
            egress.clone(),
            Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
            Arc::new(TestObligationHandler),
        )
        .unwrap();
        let flow_manager: Arc<dyn AuthFlowManager> = Arc::new(BackendUnavailableFlowManager);
        let auth = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let flow_source: Arc<dyn AuthFlowRecordSource> = auth;
        let scope = sample_turn_scope();
        let owner = ironclaw_host_api::UserId::new("user").unwrap();
        let run_id = TurnRunId::new();
        let gate_ref =
            AuthGateRef::new("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string()).unwrap();

        let error = provider
            .challenge_for_blocked_gate(
                &flow_manager,
                &flow_source,
                &scope,
                &owner,
                run_id,
                &gate_ref,
            )
            .await
            .expect_err("create_flow should fail");

        assert_eq!(
            error.code(),
            ironclaw_auth::AuthErrorCode::BackendUnavailable
        );
        assert!(
            egress
                .requests()
                .iter()
                .any(|(method, url)| *method == NetworkMethod::Delete
                    && url == "https://oauth.notion.com/register/dcr-client"),
            "create_flow failures must deregister the DCR client"
        );
    }

    #[tokio::test]
    async fn pkce_verifier_for_flow_returns_none_when_secret_not_found() {
        let provider = test_provider(Arc::new(TestEgress));

        let pkce = provider
            .pkce_verifier_for_flow(&sample_auth_scope(), AuthFlowId::new())
            .await
            .unwrap();

        assert!(pkce.is_none());
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
            .callback_redirect_uri(
                &scope,
                AuthFlowId::from_uuid(uuid::Uuid::nil()),
                &CredentialAccountLabel::new("notion").unwrap(),
            )
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

    fn test_provider(egress: Arc<dyn RuntimeHttpEgress>) -> OAuthDcrProvider {
        OAuthDcrProvider::new(
            test_config(),
            egress,
            Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
            Arc::new(TestObligationHandler),
        )
        .unwrap()
    }

    fn test_config() -> OAuthDcrProviderConfig {
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
        }
    }

    fn sample_resource_scope() -> ResourceScope {
        sample_auth_scope().resource
    }

    fn sample_auth_scope() -> AuthProductScope {
        AuthProductScope::new(
            ResourceScope {
                tenant_id: ironclaw_host_api::TenantId::new("tenant").unwrap(),
                user_id: ironclaw_host_api::UserId::new("user").unwrap(),
                agent_id: Some(ironclaw_host_api::AgentId::new("agent").unwrap()),
                project_id: Some(ironclaw_host_api::ProjectId::new("project").unwrap()),
                mission_id: None,
                thread_id: Some(ironclaw_host_api::ThreadId::new("thread").unwrap()),
                invocation_id: InvocationId::new(),
            },
            ironclaw_auth::AuthSurface::Callback,
        )
    }

    fn sample_turn_scope() -> TurnScope {
        TurnScope::new(
            ironclaw_host_api::TenantId::new("tenant").unwrap(),
            Some(ironclaw_host_api::AgentId::new("agent").unwrap()),
            Some(ironclaw_host_api::ProjectId::new("project").unwrap()),
            ironclaw_host_api::ThreadId::new("thread").unwrap(),
        )
    }

    fn sample_flow_owner() -> AuthFlowOwnerScope {
        let scope = sample_auth_scope();
        AuthFlowOwnerScope {
            tenant_id: scope.resource.tenant_id,
            user_id: scope.resource.user_id,
            agent_id: scope.resource.agent_id,
            project_id: scope.resource.project_id,
            thread_id: scope.resource.thread_id.unwrap(),
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum DcrDiscoveryCase {
        EmptyAuthorizationServers,
        ResourceMetadataFails,
        EmptyRegistrationEndpoint,
    }

    #[derive(Debug)]
    struct DcrDiscoveryEgress {
        case: DcrDiscoveryCase,
    }

    impl DcrDiscoveryEgress {
        fn empty_authorization_servers() -> Self {
            Self {
                case: DcrDiscoveryCase::EmptyAuthorizationServers,
            }
        }

        fn resource_metadata_fails() -> Self {
            Self {
                case: DcrDiscoveryCase::ResourceMetadataFails,
            }
        }

        fn empty_registration_endpoint() -> Self {
            Self {
                case: DcrDiscoveryCase::EmptyRegistrationEndpoint,
            }
        }
    }

    #[async_trait]
    impl RuntimeHttpEgress for DcrDiscoveryEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<
            ironclaw_host_api::RuntimeHttpEgressResponse,
            ironclaw_host_api::RuntimeHttpEgressError,
        > {
            let (status, body) = match (self.case, request.url.as_str()) {
                (
                    DcrDiscoveryCase::EmptyAuthorizationServers,
                    "https://mcp.notion.com/mcp/.well-known/oauth-protected-resource",
                ) => (200, br#"{"authorization_servers":[]}"#.to_vec()),
                (
                    DcrDiscoveryCase::ResourceMetadataFails,
                    "https://mcp.notion.com/mcp/.well-known/oauth-protected-resource",
                ) => (500, Vec::new()),
                (
                    DcrDiscoveryCase::ResourceMetadataFails,
                    "https://mcp.notion.com/.well-known/oauth-authorization-server",
                ) => (
                    200,
                    br#"{"authorization_endpoint":"https://mcp.notion.com/authorize","token_endpoint":"https://mcp.notion.com/token","registration_endpoint":"https://mcp.notion.com/register"}"#.to_vec(),
                ),
                (
                    DcrDiscoveryCase::EmptyRegistrationEndpoint,
                    "https://mcp.notion.com/mcp/.well-known/oauth-protected-resource",
                ) => (
                    200,
                    br#"{"authorization_servers":["https://oauth.notion.com"]}"#.to_vec(),
                ),
                (
                    DcrDiscoveryCase::EmptyRegistrationEndpoint,
                    "https://oauth.notion.com/.well-known/oauth-authorization-server",
                ) => (
                    200,
                    br#"{"authorization_endpoint":"https://oauth.notion.com/authorize","token_endpoint":"https://oauth.notion.com/token","registration_endpoint":""}"#.to_vec(),
                ),
                other => panic!("unexpected DCR discovery egress URL: {other:?}"),
            };
            Ok(ironclaw_host_api::RuntimeHttpEgressResponse {
                status,
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
    struct SecondPutFailingSecretStore {
        inner: ironclaw_secrets::InMemorySecretStore,
        put_count: AtomicUsize,
        put_handles: Mutex<Vec<SecretHandle>>,
        deleted_handles: Mutex<Vec<SecretHandle>>,
    }

    impl SecondPutFailingSecretStore {
        fn new() -> Self {
            Self {
                inner: ironclaw_secrets::InMemorySecretStore::new(),
                put_count: AtomicUsize::new(0),
                put_handles: Mutex::new(Vec::new()),
                deleted_handles: Mutex::new(Vec::new()),
            }
        }

        fn put_handles(&self) -> Vec<SecretHandle> {
            self.put_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }

        fn deleted_handles(&self) -> Vec<SecretHandle> {
            self.deleted_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl SecretStore for SecondPutFailingSecretStore {
        async fn put(
            &self,
            scope: ResourceScope,
            handle: SecretHandle,
            material: SecretMaterial,
        ) -> Result<ironclaw_secrets::SecretMetadata, ironclaw_secrets::SecretStoreError> {
            self.put_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(handle.clone());
            if self.put_count.fetch_add(1, Ordering::SeqCst) == 1 {
                return Err(ironclaw_secrets::SecretStoreError::StoreUnavailable {
                    reason: "injected second put failure".to_string(),
                });
            }
            self.inner.put(scope, handle, material).await
        }

        async fn metadata(
            &self,
            scope: &ResourceScope,
            handle: &SecretHandle,
        ) -> Result<Option<ironclaw_secrets::SecretMetadata>, ironclaw_secrets::SecretStoreError>
        {
            self.inner.metadata(scope, handle).await
        }

        async fn delete(
            &self,
            scope: &ResourceScope,
            handle: &SecretHandle,
        ) -> Result<bool, ironclaw_secrets::SecretStoreError> {
            self.deleted_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(handle.clone());
            self.inner.delete(scope, handle).await
        }

        async fn lease_once(
            &self,
            scope: &ResourceScope,
            handle: &SecretHandle,
        ) -> Result<ironclaw_secrets::SecretLease, ironclaw_secrets::SecretStoreError> {
            self.inner.lease_once(scope, handle).await
        }

        async fn consume(
            &self,
            scope: &ResourceScope,
            lease_id: ironclaw_secrets::SecretLeaseId,
        ) -> Result<SecretMaterial, ironclaw_secrets::SecretStoreError> {
            self.inner.consume(scope, lease_id).await
        }

        async fn revoke(
            &self,
            scope: &ResourceScope,
            lease_id: ironclaw_secrets::SecretLeaseId,
        ) -> Result<ironclaw_secrets::SecretLease, ironclaw_secrets::SecretStoreError> {
            self.inner.revoke(scope, lease_id).await
        }

        async fn leases_for_scope(
            &self,
            scope: &ResourceScope,
        ) -> Result<Vec<ironclaw_secrets::SecretLease>, ironclaw_secrets::SecretStoreError>
        {
            self.inner.leases_for_scope(scope).await
        }
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
                    br#"{"authorization_servers":["https://oauth.notion.com"]}"#.to_vec()
                }
                "https://oauth.notion.com/.well-known/oauth-authorization-server" => {
                    br#"{"authorization_endpoint":"https://oauth.notion.com/authorize","token_endpoint":"https://oauth.notion.com/token","registration_endpoint":"https://oauth.notion.com/register"}"#.to_vec()
                }
                "https://oauth.notion.com/register" => br#"{"client_id":"dcr-client","registration_client_uri":"https://oauth.notion.com/register/dcr-client","registration_access_token":"registration-token"}"#.to_vec(),
                "https://oauth.notion.com/register/dcr-client"
                    if request.method == NetworkMethod::Delete =>
                {
                    br#"{}"#.to_vec()
                }
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
    struct RecordingDcrSetupEgress {
        requests: Mutex<Vec<(NetworkMethod, String)>>,
    }

    impl RecordingDcrSetupEgress {
        fn new() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<(NetworkMethod, String)> {
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl RuntimeHttpEgress for RecordingDcrSetupEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<
            ironclaw_host_api::RuntimeHttpEgressResponse,
            ironclaw_host_api::RuntimeHttpEgressError,
        > {
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((request.method, request.url.clone()));
            DcrSetupEgress.execute(request).await
        }
    }

    #[derive(Debug)]
    struct BackendUnavailableFlowManager;

    #[async_trait]
    impl AuthFlowManager for BackendUnavailableFlowManager {
        async fn create_flow(
            &self,
            _request: NewAuthFlow,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            Err(AuthProductError::BackendUnavailable)
        }

        async fn get_flow(
            &self,
            _scope: &AuthProductScope,
            _flow_id: AuthFlowId,
        ) -> Result<Option<ironclaw_auth::AuthFlowRecord>, AuthProductError> {
            unreachable!("create-flow failure test does not read flows")
        }

        async fn claim_oauth_callback(
            &self,
            _scope: &AuthProductScope,
            _request: ironclaw_auth::OAuthCallbackClaimRequest,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            unreachable!("create-flow failure test does not claim callbacks")
        }

        async fn complete_oauth_callback(
            &self,
            _scope: &AuthProductScope,
            _input: ironclaw_auth::OAuthCallbackInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            unreachable!("create-flow failure test does not complete callbacks")
        }

        async fn complete_credential_selection(
            &self,
            _scope: &AuthProductScope,
            _input: ironclaw_auth::CredentialSelectionInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            unreachable!("create-flow failure test does not complete selection")
        }

        async fn complete_manual_token(
            &self,
            _scope: &AuthProductScope,
            _input: ironclaw_auth::ManualTokenCompletionInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            unreachable!("create-flow failure test does not complete manual tokens")
        }

        async fn cancel_manual_token(
            &self,
            _scope: &AuthProductScope,
            _interaction_id: ironclaw_auth::AuthInteractionId,
        ) -> Result<Option<ironclaw_auth::AuthFlowRecord>, AuthProductError> {
            unreachable!("create-flow failure test does not cancel manual tokens")
        }

        async fn fail_oauth_callback(
            &self,
            _scope: &AuthProductScope,
            _input: ironclaw_auth::OAuthCallbackFailureInput,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            unreachable!("create-flow failure test does not fail callbacks")
        }

        async fn mark_continuation_dispatched(
            &self,
            _scope: &AuthProductScope,
            _flow_id: AuthFlowId,
            _emitted_at: ironclaw_auth::Timestamp,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            unreachable!("create-flow failure test does not mark continuations")
        }

        async fn cancel_flow(
            &self,
            _scope: &AuthProductScope,
            _flow_id: AuthFlowId,
        ) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
            unreachable!("create-flow failure test does not cancel flows")
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

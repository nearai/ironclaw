use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowId, AuthFlowKind, AuthFlowManager,
    AuthFlowOwnerScope, AuthFlowRecord, AuthFlowRecordSource, AuthGateRef, AuthProductError,
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
    GoogleOAuthCallbackState, NewAuthFlow, PkceVerifierSecret, ProviderScope,
    TurnGateAuthFlowQuery, TurnRunRef, build_google_authorization_url, opaque_state_hash,
    pkce_s256_challenge, pkce_verifier_hash,
};
use ironclaw_host_api::{
    InvocationId, ResourceScope, RuntimeCredentialAuthRequirement, SecretHandle,
};
use ironclaw_product_adapters::AuthPromptChallengeKind;
use ironclaw_secrets::{SecretMaterial, SecretStore};
use ironclaw_turns::{TurnRunId, TurnScope};
use secrecy::SecretString;
use tokio::sync::Mutex as AsyncMutex;

use crate::input::OAuthClientConfig;
use crate::projection::AuthChallengeView;

const GATE_FLOW_TTL_SECONDS: i64 = 600;

#[derive(Clone)]
pub(crate) struct OAuthGateProviderRegistry {
    providers: BTreeMap<String, Arc<GoogleOAuthGateProvider>>,
}

impl OAuthGateProviderRegistry {
    pub(crate) fn new(providers: Vec<Arc<GoogleOAuthGateProvider>>) -> Self {
        Self {
            providers: providers
                .into_iter()
                .map(|provider| (provider.provider_id().to_string(), provider))
                .collect(),
        }
    }

    pub(crate) async fn challenge_for_blocked_gate(
        &self,
        request: OAuthGateChallengeRequest<'_>,
    ) -> Result<Option<AuthChallengeView>, AuthProductError> {
        let [requirement] = request.requirements else {
            return Ok(None);
        };
        let Some(provider) = self.providers.get(requirement.provider.as_str()) else {
            return Ok(None);
        };
        provider
            .challenge_for_blocked_gate(request, requirement)
            .await
            .map(Some)
    }

    pub(crate) async fn pkce_verifier_for_flow(
        &self,
        scope: &AuthProductScope,
        provider: &AuthProviderId,
        flow_id: AuthFlowId,
    ) -> Result<Option<SecretString>, AuthProductError> {
        let Some(provider) = self.providers.get(provider.as_str()) else {
            return Ok(None);
        };
        provider.pkce_verifier_for_flow(scope, flow_id).await
    }
}

impl fmt::Debug for OAuthGateProviderRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthGateProviderRegistry")
            .field("providers", &self.providers.keys().collect::<Vec<_>>())
            .finish()
    }
}

pub(crate) struct OAuthGateChallengeRequest<'a> {
    pub(crate) flow_manager: &'a Arc<dyn AuthFlowManager>,
    pub(crate) flow_source: &'a Arc<dyn AuthFlowRecordSource>,
    pub(crate) requirements: &'a [RuntimeCredentialAuthRequirement],
    pub(crate) scope: &'a TurnScope,
    pub(crate) owner_user_id: &'a ironclaw_host_api::UserId,
    pub(crate) run_id: TurnRunId,
    pub(crate) gate_ref: &'a AuthGateRef,
}

#[derive(Clone)]
pub(crate) struct GoogleOAuthGateProvider {
    client: OAuthClientConfig,
    secret_store: Arc<dyn SecretStore>,
    setup_lock: Arc<AsyncMutex<()>>,
}

impl GoogleOAuthGateProvider {
    pub(crate) fn new(client: OAuthClientConfig, secret_store: Arc<dyn SecretStore>) -> Self {
        Self {
            client,
            secret_store,
            setup_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    fn provider_id(&self) -> &'static str {
        ironclaw_auth::GOOGLE_PROVIDER_ID
    }

    async fn challenge_for_blocked_gate(
        &self,
        request: OAuthGateChallengeRequest<'_>,
        requirement: &RuntimeCredentialAuthRequirement,
    ) -> Result<AuthChallengeView, AuthProductError> {
        let auth_scope = auth_scope_for_blocked_turn(request.scope, request.owner_user_id);
        let turn_run_ref = TurnRunRef::new(request.run_id.to_string())?;
        let query = turn_gate_query(&auth_scope, request.scope, &turn_run_ref, request.gate_ref);
        if let Some(existing) = request
            .flow_source
            .flow_for_turn_gate(query.clone())
            .await?
        {
            return challenge_view_from_flow(&existing);
        }

        let _setup_guard = self.setup_lock.lock().await;
        if let Some(existing) = request
            .flow_source
            .flow_for_turn_gate(query.clone())
            .await?
        {
            return challenge_view_from_flow(&existing);
        }

        let flow_id = AuthFlowId::new();
        let scopes = provider_scopes(&requirement.provider_scopes)?;
        let prepared = self.prepare_flow(&auth_scope, flow_id, scopes).await?;
        let expires_at = Utc::now() + ChronoDuration::seconds(GATE_FLOW_TTL_SECONDS);
        let flow = match request
            .flow_manager
            .create_flow(NewAuthFlow {
                id: Some(flow_id),
                scope: auth_scope.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: AuthProviderId::new(self.provider_id())?,
                challenge: AuthChallenge::OAuthUrl {
                    authorization_url: prepared.authorization_url,
                    expires_at,
                },
                continuation: AuthContinuationRef::TurnGateResume {
                    turn_run_ref,
                    gate_ref: request.gate_ref.clone(),
                },
                update_binding: None,
                opaque_state_hash: Some(prepared.opaque_state_hash),
                pkce_verifier_hash: Some(prepared.pkce_verifier_hash),
                expires_at,
            })
            .await
        {
            Ok(flow) => flow,
            Err(AuthProductError::BackendConflict) => request
                .flow_source
                .flow_for_turn_gate(query)
                .await?
                .ok_or(AuthProductError::BackendConflict)?,
            Err(error) => return Err(error),
        };

        if flow.id == flow_id
            && let Err(error) = self
                .store_pkce_verifier(&flow.scope.resource, flow_id, prepared.pkce_verifier)
                .await
        {
            if request
                .flow_manager
                .cancel_flow(&flow.scope, flow_id)
                .await
                .is_err()
            {
                tracing::warn!(
                    provider = self.provider_id(),
                    flow_id = %flow_id,
                    "failed to cancel OAuth gate flow after PKCE storage failure"
                );
            }
            return Err(error);
        }

        challenge_view_from_flow(&flow)
    }

    async fn prepare_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        scopes: Vec<ProviderScope>,
    ) -> Result<PreparedOAuthGateFlow, AuthProductError> {
        let account_label = CredentialAccountLabel::new("google")?;
        let state =
            GoogleOAuthCallbackState::new(flow_id, scope.clone(), account_label, scopes.clone())?
                .encode()?;
        let opaque_state_hash = opaque_state_hash(state.as_str())?;
        let pkce_verifier = SecretString::from(ironclaw_common::pkce::generate_code_verifier());
        let pkce_secret = PkceVerifierSecret::new(pkce_verifier.clone())?;
        let pkce_verifier_hash = pkce_verifier_hash(&pkce_secret)?;
        let pkce_challenge = pkce_s256_challenge(&pkce_secret);
        let authorization_url = build_google_authorization_url(
            self.client.client_id.as_str(),
            self.client.redirect_uri.as_str(),
            state.as_str(),
            &pkce_challenge,
            &scopes,
            None,
        )?;
        Ok(PreparedOAuthGateFlow {
            authorization_url,
            opaque_state_hash,
            pkce_verifier_hash,
            pkce_verifier,
        })
    }

    async fn store_pkce_verifier(
        &self,
        scope: &ResourceScope,
        flow_id: AuthFlowId,
        material: SecretMaterial,
    ) -> Result<(), AuthProductError> {
        self.secret_store
            .put(scope.clone(), pkce_secret_handle(flow_id)?, material)
            .await
            .map(|_| ())
            .map_err(|_| AuthProductError::BackendUnavailable)
    }

    async fn pkce_verifier_for_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<SecretString>, AuthProductError> {
        let handle = pkce_secret_handle(flow_id)?;
        let lease = match self.secret_store.lease_once(&scope.resource, &handle).await {
            Ok(lease) => lease,
            Err(error) if error.is_unknown_secret() => return Ok(None),
            Err(_) => return Err(AuthProductError::BackendUnavailable),
        };
        self.secret_store
            .consume(&scope.resource, lease.id)
            .await
            .map(Some)
            .map_err(|_| AuthProductError::BackendUnavailable)
    }
}

impl fmt::Debug for GoogleOAuthGateProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleOAuthGateProvider")
            .field("client_id", &self.client.client_id.as_str())
            .field("redirect_uri", &self.client.redirect_uri)
            .finish()
    }
}

#[derive(Debug)]
struct PreparedOAuthGateFlow {
    authorization_url: ironclaw_auth::OAuthAuthorizationUrl,
    opaque_state_hash: ironclaw_auth::OpaqueStateHash,
    pkce_verifier_hash: ironclaw_auth::PkceVerifierHash,
    pkce_verifier: SecretString,
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
        AuthSurface::Callback,
    )
}

fn turn_gate_query(
    auth_scope: &AuthProductScope,
    turn_scope: &TurnScope,
    turn_run_ref: &TurnRunRef,
    gate_ref: &AuthGateRef,
) -> TurnGateAuthFlowQuery {
    TurnGateAuthFlowQuery {
        owner: AuthFlowOwnerScope {
            tenant_id: auth_scope.resource.tenant_id.clone(),
            user_id: auth_scope.resource.user_id.clone(),
            agent_id: auth_scope.resource.agent_id.clone(),
            project_id: auth_scope.resource.project_id.clone(),
            thread_id: turn_scope.thread_id.clone(),
        },
        turn_run_ref: turn_run_ref.clone(),
        gate_ref: gate_ref.clone(),
        include_terminal: false,
    }
}

fn provider_scopes(raw_scopes: &[String]) -> Result<Vec<ProviderScope>, AuthProductError> {
    raw_scopes
        .iter()
        .map(|scope| ProviderScope::new(scope.clone()))
        .collect()
}

fn challenge_view_from_flow(flow: &AuthFlowRecord) -> Result<AuthChallengeView, AuthProductError> {
    match flow.challenge.as_ref() {
        Some(AuthChallenge::OAuthUrl {
            authorization_url,
            expires_at,
        }) => Ok(AuthChallengeView {
            kind: AuthPromptChallengeKind::OAuthUrl,
            provider: flow.provider.clone(),
            account_label: None,
            authorization_url: Some(authorization_url.clone()),
            expires_at: Some(*expires_at),
        }),
        Some(_) | None => Err(AuthProductError::BackendUnavailable),
    }
}

fn pkce_secret_handle(flow_id: AuthFlowId) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!("google-oauth-gate-flow-pkce-{flow_id}"))
        .map_err(|_| AuthProductError::BackendUnavailable)
}

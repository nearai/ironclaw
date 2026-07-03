//! Slack personal (user-token) OAuth: provider spec + blocked-gate provider.
//!
//! Approach (B) duplicate: this mirrors the Google product-auth OAuth gate in
//! [`crate::oauth_gate`] but builds a Slack authorization URL (`user_scope`) and
//! encodes a [`SlackPersonalOAuthCallbackState`]. The provider-agnostic helpers
//! (`auth_scope_for_blocked_turn`, `turn_gate_query`, `challenge_view_from_flow`,
//! `provider_scopes`, `OAuthGateChallengeRequest`) are shared from
//! `crate::oauth_gate`; only the flow preparation and PKCE handle differ. The
//! Google path is left functionally unchanged.

use std::fmt;
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowId, AuthFlowKind, AuthFlowManager, AuthFlowRecord,
    AuthFlowRecordSource, AuthProductError, AuthProductScope, AuthProviderId,
    CredentialAccountLabel, NewAuthFlow, PkceVerifierSecret, ProviderScope,
    SLACK_PERSONAL_PROVIDER_ID, SlackPersonalOAuthCallbackState, TurnGateAuthFlowQuery, TurnRunRef,
    build_slack_personal_authorization_url, opaque_state_hash, pkce_s256_challenge,
    pkce_verifier_hash,
};
use ironclaw_host_api::{ResourceScope, SecretHandle};
use ironclaw_secrets::{SecretMaterial, SecretStore};
use secrecy::SecretString;
use tokio::sync::Mutex as AsyncMutex;

use crate::AuthChallengeView;
use crate::oauth_gate::{
    OAuthGateChallengeRequest, auth_scope_for_blocked_turn, challenge_view_from_flow,
    provider_scopes, turn_gate_query,
};
use crate::oauth_provider_client::{
    ExchangeScopePolicy, HostOAuthProviderSpec, TokenResponseShape,
};
use crate::slack_setup::SlackPersonalSetupServiceSlot;

const GATE_FLOW_TTL_SECONDS: i64 = 600;

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

#[derive(Clone)]
pub(crate) struct SlackPersonalOAuthGateProviderRegistry {
    providers: std::collections::BTreeMap<String, Arc<SlackPersonalOAuthGateProvider>>,
}

impl SlackPersonalOAuthGateProviderRegistry {
    pub(crate) fn new(providers: Vec<Arc<SlackPersonalOAuthGateProvider>>) -> Self {
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
        for requirement in request.requirements {
            let Some(provider) = self.providers.get(requirement.provider.as_str()) else {
                continue;
            };
            return provider
                .challenge_for_blocked_gate(request, requirement)
                .await
                .map(Some);
        }
        Ok(None)
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

impl fmt::Debug for SlackPersonalOAuthGateProviderRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SlackPersonalOAuthGateProviderRegistry")
            .field("providers", &self.providers.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Clone)]
pub(crate) struct SlackPersonalOAuthGateProvider {
    slot: SlackPersonalSetupServiceSlot,
    secret_store: Arc<dyn SecretStore>,
    setup_lock: Arc<AsyncMutex<()>>,
}

impl SlackPersonalOAuthGateProvider {
    pub(crate) fn new(
        slot: SlackPersonalSetupServiceSlot,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            slot,
            secret_store,
            setup_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    fn provider_id(&self) -> &'static str {
        SLACK_PERSONAL_PROVIDER_ID
    }

    async fn challenge_for_blocked_gate(
        &self,
        request: OAuthGateChallengeRequest<'_>,
        requirement: &ironclaw_host_api::RuntimeCredentialAuthRequirement,
    ) -> Result<AuthChallengeView, AuthProductError> {
        let auth_scope = auth_scope_for_blocked_turn(request.scope, request.owner_user_id);
        let turn_run_ref = TurnRunRef::new(request.run_id.to_string())?;
        let query = turn_gate_query(&auth_scope, request.scope, &turn_run_ref, request.gate_ref);

        let _setup_guard = self.setup_lock.lock().await;
        if let Some(existing) = self
            .reusable_flow_for_query(request.flow_manager, request.flow_source, query.clone())
            .await?
        {
            return challenge_view_from_flow(&existing);
        }

        let flow_id = AuthFlowId::new();
        let scopes = provider_scopes(&requirement.provider_scopes)?;
        let prepared = self.prepare_flow(&auth_scope, flow_id, scopes).await?;
        let expires_at = Utc::now() + ChronoDuration::seconds(GATE_FLOW_TTL_SECONDS);
        self.store_pkce_verifier(
            &auth_scope.resource,
            flow_id,
            prepared.pkce_verifier.clone(),
        )
        .await?;
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
            Err(AuthProductError::BackendConflict) => {
                self.cleanup_pkce_verifier(&auth_scope.resource, flow_id)
                    .await;
                self.reusable_flow_for_query(request.flow_manager, request.flow_source, query)
                    .await?
                    .ok_or(AuthProductError::BackendConflict)?
            }
            Err(error) => {
                self.cleanup_pkce_verifier(&auth_scope.resource, flow_id)
                    .await;
                return Err(error);
            }
        };

        challenge_view_from_flow(&flow)
    }

    async fn reusable_flow_for_query(
        &self,
        flow_manager: &Arc<dyn AuthFlowManager>,
        flow_source: &Arc<dyn AuthFlowRecordSource>,
        query: TurnGateAuthFlowQuery,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let Some(existing) = flow_source.flow_for_turn_gate(query).await? else {
            return Ok(None);
        };
        if existing.expires_at > Utc::now() {
            return Ok(Some(existing));
        }
        self.cleanup_pkce_verifier(&existing.scope.resource, existing.id)
            .await;
        flow_manager
            .cancel_flow(&existing.scope, existing.id)
            .await
            .map(|_| None)
    }

    async fn prepare_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        scopes: Vec<ProviderScope>,
    ) -> Result<PreparedSlackOAuthGateFlow, AuthProductError> {
        let service = self
            .slot
            .get()
            .ok_or(AuthProductError::BackendUnavailable)?;
        let (client_id, _client_secret) = service.oauth_credentials().await.map_err(|e| {
            tracing::warn!(error = %e, "Slack personal OAuth credentials not configured");
            AuthProductError::BackendUnavailable
        })?;
        let account_label = CredentialAccountLabel::new("slack_personal")?;
        let state = SlackPersonalOAuthCallbackState::new(
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
        let authorization_url = build_slack_personal_authorization_url(
            client_id.as_str(),
            self.slot.redirect_uri().as_str(),
            state.as_str(),
            &pkce_challenge,
            &scopes,
        )?;
        Ok(PreparedSlackOAuthGateFlow {
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
            .put(scope.clone(), pkce_secret_handle(flow_id)?, material, None)
            .await
            .map(|_| ())
            .map_err(|e| {
                tracing::warn!(
                    provider = self.provider_id(),
                    flow_id = %flow_id,
                    error = %e,
                    "failed to store Slack OAuth PKCE verifier"
                );
                AuthProductError::BackendUnavailable
            })
    }

    async fn cleanup_pkce_verifier(&self, scope: &ResourceScope, flow_id: AuthFlowId) {
        let Ok(handle) = pkce_secret_handle(flow_id) else {
            return;
        };
        if self.secret_store.delete(scope, &handle).await.is_err() {
            tracing::warn!(
                provider = self.provider_id(),
                flow_id = %flow_id,
                "failed to clean up Slack OAuth gate PKCE verifier after flow creation failure"
            );
        }
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
            Err(e) => {
                tracing::warn!(
                    provider = self.provider_id(),
                    flow_id = %flow_id,
                    error = %e,
                    "failed to lease Slack OAuth PKCE verifier"
                );
                return Err(AuthProductError::BackendUnavailable);
            }
        };
        self.secret_store
            .consume(&scope.resource, lease.id)
            .await
            .map(Some)
            .map_err(|e| {
                tracing::warn!(
                    provider = self.provider_id(),
                    flow_id = %flow_id,
                    error = %e,
                    "failed to consume Slack OAuth PKCE verifier"
                );
                AuthProductError::BackendUnavailable
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

#[derive(Debug)]
struct PreparedSlackOAuthGateFlow {
    authorization_url: ironclaw_auth::OAuthAuthorizationUrl,
    opaque_state_hash: ironclaw_auth::OpaqueStateHash,
    pkce_verifier_hash: ironclaw_auth::PkceVerifierHash,
    pkce_verifier: SecretString,
}

fn pkce_secret_handle(flow_id: AuthFlowId) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!("slack-personal-oauth-gate-flow-pkce-{flow_id}")).map_err(|e| {
        tracing::warn!(flow_id = %flow_id, error = %e, "failed to build Slack OAuth PKCE secret handle");
        AuthProductError::BackendUnavailable
    })
}

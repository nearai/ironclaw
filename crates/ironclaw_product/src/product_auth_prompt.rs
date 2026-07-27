use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthChallenge, AuthFlowOwnerScope, AuthGateRef, AuthProductError, RebornProductAuthServices,
    TurnGateAuthFlowQuery, TurnRunRef,
};
use ironclaw_host_api::{RuntimeCredentialAuthRequirement, UserId};
use ironclaw_turns::{TurnRunId, TurnScope};

use crate::{
    AuthChallengeProvider, AuthChallengeView, AuthPromptChallengeKind, BlockedAuthFlowCanceller,
};

pub fn product_auth_challenge_provider(
    product_auth: &Arc<RebornProductAuthServices>,
) -> Option<Arc<dyn AuthChallengeProvider>> {
    product_auth
        .flow_record_source()
        .map(|_| Arc::clone(product_auth) as Arc<dyn AuthChallengeProvider>)
}

pub fn blocked_auth_flow_canceller(
    product_auth: &Arc<RebornProductAuthServices>,
) -> Option<Arc<dyn BlockedAuthFlowCanceller>> {
    product_auth
        .flow_record_source()
        .map(|_| Arc::clone(product_auth) as Arc<dyn BlockedAuthFlowCanceller>)
}

#[async_trait]
impl AuthChallengeProvider for RebornProductAuthServices {
    async fn challenge_for_gate(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
        credential_requirements: &[RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, AuthProductError> {
        let gate_ref = AuthGateRef::new(gate_ref.to_string()).map_err(|error| {
            tracing::debug!(%error, "invalid gate_ref in auth challenge lookup");
            AuthProductError::BackendUnavailable
        })?;
        let Some(source) = self.flow_record_source() else {
            return Ok(None);
        };
        let flow_manager = self.flow_manager();
        if let Some(driver) = self.oauth_gate_driver()
            && let Some(flow) = driver
                .challenge_for_blocked_gate(ironclaw_auth::OAuthGateChallengeRequest {
                    flow_manager: &flow_manager,
                    flow_source: &source,
                    requirements: credential_requirements,
                    scope,
                    owner_user_id,
                    run_id,
                    gate_ref: &gate_ref,
                })
                .await?
        {
            let Some(challenge) = flow.challenge.as_ref() else {
                return Ok(None);
            };
            return Ok(Some(auth_challenge_to_view(challenge, &flow.provider)));
        }
        let flow = source
            .flow_for_turn_gate(TurnGateAuthFlowQuery {
                owner: AuthFlowOwnerScope {
                    tenant_id: scope.tenant_id.clone(),
                    user_id: owner_user_id.clone(),
                    agent_id: scope.agent_id.clone(),
                    project_id: scope.project_id.clone(),
                    thread_id: scope.thread_id.clone(),
                },
                turn_run_ref: TurnRunRef::new(run_id.to_string()).map_err(|error| {
                    tracing::debug!(%error, "invalid run_id in auth challenge lookup");
                    AuthProductError::BackendUnavailable
                })?,
                gate_ref,
                include_terminal: false,
            })
            .await?;
        let Some(flow) = flow else {
            return Ok(None);
        };
        let Some(challenge) = flow.challenge.as_ref() else {
            return Ok(None);
        };
        Ok(Some(auth_challenge_to_view(challenge, &flow.provider)))
    }
}

#[async_trait]
impl BlockedAuthFlowCanceller for RebornProductAuthServices {
    async fn cancel_blocked_auth_flow(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
    ) -> Result<(), AuthProductError> {
        self.cancel_blocked_auth_flow(scope, owner_user_id, run_id, gate_ref)
            .await
    }
}

fn auth_challenge_to_view(
    challenge: &AuthChallenge,
    provider: &ironclaw_auth::AuthProviderId,
) -> AuthChallengeView {
    match challenge {
        AuthChallenge::OAuthUrl {
            authorization_url,
            expires_at,
        } => AuthChallengeView {
            kind: AuthPromptChallengeKind::OAuthUrl,
            provider: provider.clone(),
            account_label: None,
            authorization_url: Some(authorization_url.clone()),
            expires_at: Some(*expires_at),
        },
        AuthChallenge::ManualTokenRequired {
            provider,
            label,
            expires_at,
            ..
        } => AuthChallengeView {
            kind: AuthPromptChallengeKind::ManualToken,
            provider: provider.clone(),
            account_label: Some(label.clone()),
            authorization_url: None,
            expires_at: Some(*expires_at),
        },
        AuthChallenge::AccountSelectionRequired { .. }
        | AuthChallenge::ReauthorizeRequired { .. }
        | AuthChallenge::SetupRequired { .. } => AuthChallengeView {
            kind: AuthPromptChallengeKind::Other,
            provider: provider.clone(),
            account_label: None,
            authorization_url: None,
            expires_at: None,
        },
    }
}

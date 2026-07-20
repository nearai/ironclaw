//! Product-neutral rendering support for blocked-auth prompts.

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProviderId, CredentialAccountLabel, OAuthAuthorizationUrl,
};
use ironclaw_host_api::{RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement, UserId};
use ironclaw_product_adapters::{
    AuthPromptChallengeKind, AuthPromptView, ProductAdapterError, RedactedString,
};
use ironclaw_turns::{TurnRunId, TurnScope};

/// Redacted challenge metadata safe to project through product adapters.
#[derive(Debug, Clone)]
pub struct AuthChallengeView {
    pub kind: AuthPromptChallengeKind,
    pub provider: AuthProviderId,
    pub account_label: Option<CredentialAccountLabel>,
    pub authorization_url: Option<OAuthAuthorizationUrl>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl AuthChallengeView {
    fn enrich(self, mut view: AuthPromptView) -> AuthPromptView {
        view.challenge_kind = Some(self.kind);
        view.provider = Some(self.provider.as_str().to_string());
        view.account_label = self.account_label.map(|label| label.as_str().to_string());
        view.authorization_url = self.authorization_url.map(|url| url.as_str().to_string());
        view.expires_at = self.expires_at;
        view.connection = None;
        view
    }
}

/// Scoped read port for enriching an auth prompt with its durable challenge.
#[async_trait]
pub trait AuthChallengeProvider: Send + Sync {
    async fn challenge_for_gate(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
        credential_requirements: &[RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, AuthProductError>;
}

/// Cancels the durable auth flow behind a directly-cancelled blocked run.
#[async_trait]
pub trait BlockedAuthFlowCanceller: Send + Sync {
    async fn cancel_blocked_auth_flow(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
    ) -> Result<(), AuthProductError>;
}

/// Enrich an already-owned wire view without introducing a crossing request DTO.
pub async fn enrich_auth_prompt_view(
    view: AuthPromptView,
    fallback_owner_user_id: &UserId,
    scope: &TurnScope,
    credential_requirements: &[RuntimeCredentialAuthRequirement],
    auth_challenges: Option<&dyn AuthChallengeProvider>,
) -> Result<AuthPromptView, ProductAdapterError> {
    let owner_user_id = scope
        .explicit_owner_user_id()
        .unwrap_or(fallback_owner_user_id);
    let challenge = match auth_challenges {
        Some(provider) => provider
            .challenge_for_gate(
                scope,
                owner_user_id,
                view.turn_run_id,
                &view.auth_request_ref,
                credential_requirements,
            )
            .await
            .map_err(|error| {
                tracing::debug!(
                    %error,
                    run_id = %view.turn_run_id,
                    "auth challenge lookup failed during auth prompt rendering"
                );
                ProductAdapterError::WorkflowTransient {
                    reason: RedactedString::new("auth challenge lookup failed"),
                }
            })?,
        None => None,
    };
    Ok(match challenge {
        Some(challenge) => challenge.enrich(view),
        None => auth_prompt_from_credential_requirement(view, credential_requirements),
    })
}

fn auth_prompt_from_credential_requirement(
    mut view: AuthPromptView,
    credential_requirements: &[RuntimeCredentialAuthRequirement],
) -> AuthPromptView {
    let [requirement] = credential_requirements else {
        return view;
    };
    let provider = requirement.provider.as_str().to_string();
    match &requirement.setup {
        RuntimeCredentialAccountSetup::ManualToken => {
            view.challenge_kind = Some(AuthPromptChallengeKind::ManualToken);
            view.account_label = Some(provider.clone());
        }
        RuntimeCredentialAccountSetup::OAuth { .. } => {
            view.challenge_kind = Some(AuthPromptChallengeKind::OAuthUrl);
        }
        RuntimeCredentialAccountSetup::Pairing => {
            view.challenge_kind = Some(AuthPromptChallengeKind::Pairing);
            view.account_label = Some(provider.clone());
        }
        RuntimeCredentialAccountSetup::Retired => {}
    }
    view.provider = Some(provider);
    view
}

//! Composition implementations of the generic run-delivery ports
//! (`ironclaw_product::run_delivery`): approval-gate context from
//! the projection layer, blocked-auth prompt views from the product-auth
//! engine, and the auth-flow cancel bridge. All delivery *semantics* live in
//! the generic components; these adapters only surface composition-owned
//! read models.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{AuthProductError, AuthProviderId};
use ironclaw_host_api::{RuntimeCredentialAccountSetup, UserId};
use ironclaw_product::{
    ApprovalPromptContextSource, AuthChallengeProvider, AuthChallengeView, BlockedAuthPromptSource,
    ChannelPairingRegistry, PairingAuthChallengeView,
};
use ironclaw_product::{ApprovalPromptContextView, AuthPromptView, ProductAdapterError};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_turns::{GateRef, TurnScope};

use ironclaw_product::auth_prompt_view_for_blocked_auth;

/// One recipe-driven challenge materializer for every product surface.
/// Product-auth owns OAuth/manual challenges; the canonical channel-pairing
/// registry owns host-issued pairing codes. Callers see one typed provider.
pub(crate) struct RecipeAuthChallengeProvider {
    product_auth: Option<Arc<dyn AuthChallengeProvider>>,
    pairing: Option<Arc<ChannelPairingRegistry>>,
}

impl RecipeAuthChallengeProvider {
    pub(crate) fn compose(
        product_auth: Option<Arc<dyn AuthChallengeProvider>>,
        pairing: Option<Arc<ChannelPairingRegistry>>,
    ) -> Option<Arc<dyn AuthChallengeProvider>> {
        if product_auth.is_none() && pairing.is_none() {
            return None;
        }
        Some(Arc::new(Self {
            product_auth,
            pairing,
        }))
    }
}

#[async_trait]
impl AuthChallengeProvider for RecipeAuthChallengeProvider {
    async fn challenge_for_gate(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: ironclaw_turns::TurnRunId,
        gate_ref: &str,
        credential_requirements: &[ironclaw_host_api::RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, AuthProductError> {
        if let [requirement] = credential_requirements
            && requirement.setup == RuntimeCredentialAccountSetup::Pairing
        {
            let Some(service) = self
                .pairing
                .as_ref()
                .and_then(|registry| registry.get(requirement.requester_extension.as_str()))
            else {
                return Ok(None);
            };
            let issue = service
                .pending_or_issue(owner_user_id)
                .await
                .map_err(|error| {
                    tracing::debug!(
                        target = "ironclaw::reborn::channel_pairing",
                        %error,
                        "pairing challenge materialization failed"
                    );
                    AuthProductError::BackendUnavailable
                })?;
            let Some(issue) = issue else {
                return Ok(None);
            };
            return Ok(Some(AuthChallengeView {
                kind: ironclaw_product::AuthPromptChallengeKind::Pairing,
                provider: AuthProviderId::new(requirement.provider.as_str().to_string())
                    .map_err(|_| AuthProductError::MalformedConfig)?,
                account_label: None,
                authorization_url: None,
                expires_at: Some(issue.expires_at),
                pairing: Some(PairingAuthChallengeView {
                    issue,
                    connection: service.connection_requirement().clone(),
                }),
            }));
        }

        match &self.product_auth {
            Some(provider) => {
                provider
                    .challenge_for_gate(
                        scope,
                        owner_user_id,
                        run_id,
                        gate_ref,
                        credential_requirements,
                    )
                    .await
            }
            None => Ok(None),
        }
    }
}

/// Approval-gate context over the shared projection read model — the same
/// source the WebUI gate projection renders from.
pub(crate) struct ProjectionApprovalPromptContextSource {
    approval_requests: Arc<dyn ApprovalRequestStore>,
}

impl ProjectionApprovalPromptContextSource {
    pub(crate) fn new(approval_requests: Arc<dyn ApprovalRequestStore>) -> Self {
        Self { approval_requests }
    }
}

#[async_trait]
impl ApprovalPromptContextSource for ProjectionApprovalPromptContextSource {
    async fn approval_prompt_context(
        &self,
        gate_ref: &GateRef,
        owner_user_id: &UserId,
        scope: &TurnScope,
    ) -> Option<ApprovalPromptContextView> {
        crate::projection::approval_prompt_context_view(
            Some(self.approval_requests.as_ref()),
            gate_ref,
            owner_user_id,
            scope,
        )
        .await
    }
}

/// Blocked-auth prompt views over the product-auth challenge engine.
pub(crate) struct ProductAuthBlockedAuthPromptSource {
    auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
}

impl ProductAuthBlockedAuthPromptSource {
    pub(crate) fn new(auth_challenges: Option<Arc<dyn AuthChallengeProvider>>) -> Self {
        Self { auth_challenges }
    }
}

#[async_trait]
impl BlockedAuthPromptSource for ProductAuthBlockedAuthPromptSource {
    async fn auth_prompt_for_blocked_run(
        &self,
        request: ironclaw_product::BlockedAuthPromptRequest<'_>,
    ) -> Result<AuthPromptView, ProductAdapterError> {
        auth_prompt_view_for_blocked_auth(request, self.auth_challenges.as_deref()).await
    }
}

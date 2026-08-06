//! Host implementations of the generic run-delivery ports
//! (`ironclaw_product_contracts::prompt_source`): approval-gate context from
//! the approval request store, blocked-auth prompt views from the product-auth
//! engine, and the auth-flow cancel bridge. All delivery *semantics* live in
//! the generic components; these adapters only surface host-owned read models.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::product_prompt::{
    AuthChallengeProvider, AuthChallengeView, PairingAuthChallengeView,
    auth_prompt_view_for_blocked_auth,
};
use ironclaw_auth::{AuthProductError, AuthProviderId};
use ironclaw_extension_contracts::auth_prompt::AuthPromptView;
use ironclaw_host_api::product_adapter_error::ProductAdapterError;
use ironclaw_host_api::turn::{TurnGateRef, TurnScope};
use ironclaw_host_api::{capability::RuntimeCredentialAccountSetup, ids::UserId};
use ironclaw_product_contracts::approval_prompt::{
    approval_prompt_context_for_request, approval_prompt_lookup_scope,
    approval_request_id_from_gate_ref,
};
use ironclaw_product_contracts::outbound::ApprovalPromptContextView;
use ironclaw_product_contracts::prompt_source::{
    ApprovalPromptContextSource, BlockedAuthPromptRequest, BlockedAuthPromptSource,
};

use crate::channel_pairing::ChannelPairingRegistry;

/// One recipe-driven challenge materializer for every product surface.
/// Product auth owns OAuth/manual challenges; the canonical channel-pairing
/// registry owns host-issued pairing codes. Callers see one typed provider.
pub struct RecipeAuthChallengeProvider {
    product_auth: Option<Arc<dyn AuthChallengeProvider>>,
    pairing: Option<Arc<ChannelPairingRegistry>>,
}

impl RecipeAuthChallengeProvider {
    pub fn compose(
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
        scope: &ironclaw_host_api::turn::TurnScope,
        owner_user_id: &UserId,
        run_id: ironclaw_host_api::turn::TurnRunId,
        gate_ref: &str,
        credential_requirements: &[ironclaw_host_api::decision::RuntimeCredentialAuthRequirement],
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
                kind: ironclaw_extension_contracts::auth_prompt::AuthPromptChallengeKind::Pairing,
                provider: AuthProviderId::new(requirement.provider.as_str().to_string()).map_err(
                    |error| {
                        // `MalformedConfig` is a unit variant, so the cause has
                        // nowhere to ride to the caller -- log it here rather
                        // than dropping it (`.claude/rules/error-handling.md`).
                        tracing::warn!(
                            provider = %requirement.provider,
                            %error,
                            "pairing challenge has an unusable provider id"
                        );
                        AuthProductError::MalformedConfig
                    },
                )?,
                account_label: None,
                authorization_url: None,
                expires_at: Some(issue.expires_at),
                pairing: Some(PairingAuthChallengeView {
                    code: issue.code.as_str().to_string(),
                    deep_link: issue.deep_link,
                    expires_at: issue.expires_at,
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
///
/// The store read is here because the store is here
/// (`ironclaw_approvals::ApprovalRequestStorePort`); the gate-ref parse, the
/// lookup scope, and the request→view projection are the *shared* half and live
/// in `ironclaw_product_contracts::approval_prompt`, so this and product's
/// `projection::approval_prompt_context_view` render from one definition
/// instead of this crate reaching up into product for it.
pub struct ProjectionApprovalPromptContextSource {
    approval_requests: Arc<dyn ironclaw_approvals::ApprovalRequestStorePort>,
}

impl ProjectionApprovalPromptContextSource {
    pub fn new(approval_requests: Arc<dyn ironclaw_approvals::ApprovalRequestStorePort>) -> Self {
        Self { approval_requests }
    }
}

#[async_trait]
impl ApprovalPromptContextSource for ProjectionApprovalPromptContextSource {
    async fn approval_prompt_context(
        &self,
        gate_ref: &TurnGateRef,
        owner_user_id: &UserId,
        scope: &TurnScope,
    ) -> Option<ApprovalPromptContextView> {
        let request_id = approval_request_id_from_gate_ref(gate_ref)?;
        let resource_scope = approval_prompt_lookup_scope(scope, owner_user_id);
        match self
            .approval_requests
            .get(&resource_scope, request_id)
            .await
        {
            Ok(Some(record)) => approval_prompt_context_for_request(&record.request),
            // silent-ok: the same documented best-effort degradation product's
            // delivery-prompt path applies — a missing or unreadable request
            // renders the generic prompt rather than failing the delivery.
            Ok(None) | Err(_) => None,
        }
    }
}

/// Blocked-auth prompt views over the product-auth challenge engine.
pub struct ProductAuthBlockedAuthPromptSource {
    auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
}

impl ProductAuthBlockedAuthPromptSource {
    pub fn new(auth_challenges: Option<Arc<dyn AuthChallengeProvider>>) -> Self {
        Self { auth_challenges }
    }
}

#[async_trait]
impl BlockedAuthPromptSource for ProductAuthBlockedAuthPromptSource {
    async fn auth_prompt_for_blocked_run(
        &self,
        request: BlockedAuthPromptRequest<'_>,
    ) -> Result<AuthPromptView, ProductAdapterError> {
        auth_prompt_view_for_blocked_auth(request, self.auth_challenges.as_deref()).await
    }
}

//! Composition implementations of the generic run-delivery ports
//! (`ironclaw_product::run_delivery`): approval-gate context from
//! the projection layer, blocked-auth prompt views from the product-auth
//! engine, and the auth-flow cancel bridge. All delivery *semantics* live in
//! the generic components; these adapters only surface composition-owned
//! read models.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::UserId;
use ironclaw_product::{
    ApprovalPromptContextSource, AuthChallengeProvider, BlockedAuthPromptSource,
};
use ironclaw_product::{ApprovalPromptContextView, AuthPromptView, ProductAdapterError};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_turns::{GateRef, TurnScope};

use ironclaw_product::auth_prompt_view_for_blocked_auth;

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

//! Composition implementations of the generic run-delivery ports
//! (`ironclaw_product_workflow::run_delivery`): approval-gate context from
//! the projection layer, blocked-auth prompt views from the product-auth
//! engine, and the auth-flow cancel bridge. All delivery *semantics* live in
//! the generic components; these adapters only surface composition-owned
//! read models.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::UserId;
use ironclaw_product_adapters::{ApprovalPromptContextView, AuthPromptView, ProductAdapterError};
use ironclaw_product_workflow::{
    ApprovalPromptContextSource, BlockedAuthFlowCancel, BlockedAuthPromptSource,
};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_turns::{GateRef, TurnRunId, TurnScope};

use crate::product_auth::api::auth_prompt::{
    BlockedAuthPromptRequest as ProductAuthBlockedAuthPromptRequest,
    auth_prompt_view_for_blocked_auth,
};
use crate::{AuthChallengeProvider, BlockedAuthFlowCanceller};

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
        request: ironclaw_product_workflow::BlockedAuthPromptRequest<'_>,
    ) -> Result<AuthPromptView, ProductAdapterError> {
        auth_prompt_view_for_blocked_auth(ProductAuthBlockedAuthPromptRequest {
            fallback_owner_user_id: request.fallback_owner_user_id,
            scope: request.scope,
            run_id: request.run_id,
            gate_ref: request.gate_ref,
            invocation_id: None,
            body: request.body,
            credential_requirements: request.credential_requirements,
            auth_challenges: self.auth_challenges.as_deref(),
        })
        .await
    }
}

/// Bridges the generic flow-cancel port onto the product-auth canceller.
pub(crate) struct ProductAuthBlockedAuthFlowCancel {
    canceller: Arc<dyn BlockedAuthFlowCanceller>,
}

impl ProductAuthBlockedAuthFlowCancel {
    pub(crate) fn new(canceller: Arc<dyn BlockedAuthFlowCanceller>) -> Self {
        Self { canceller }
    }
}

#[async_trait]
impl BlockedAuthFlowCancel for ProductAuthBlockedAuthFlowCancel {
    async fn cancel_blocked_auth_flow(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
    ) -> Result<(), String> {
        self.canceller
            .cancel_blocked_auth_flow(scope, owner_user_id, run_id, gate_ref)
            .await
            .map_err(|error| error.to_string())
    }
}

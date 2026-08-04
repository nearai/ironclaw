//! The gate-prompt enrichment ports (PROPOSAL §6.1.3).
//!
//! When a run parks on an approval or auth gate, the delivery path and the
//! projection layer both render a prompt. *What* is being approved, and *which
//! challenge* an auth gate is waiting on, are read models owned outside
//! product — the approval-request store and the pairing/auth engines — so both
//! arrive through ports. `ironclaw_extension_host` implements both over its
//! pairing registry and the approvals store.
//!
//! Never here: prompt *rendering* (product owns the view constructor), the
//! challenge engine, or any implementation of these ports.

use async_trait::async_trait;
use ironclaw_extension_contracts::auth_prompt::AuthPromptView;
use ironclaw_host_api::decision::RuntimeCredentialAuthRequirement;
use ironclaw_host_api::ids::{InvocationId, UserId};
use ironclaw_host_api::product_adapter_error::ProductAdapterError;
use ironclaw_host_api::turn::{TurnGateRef, TurnRunId, TurnScope};

use crate::outbound::ApprovalPromptContextView;

/// Inputs for resolving a blocked-auth run's prompt view. One request shape
/// for every renderer (delivery path, projection layer); the challenge
/// provider is a separate argument, not request data.
pub struct BlockedAuthPromptRequest<'a> {
    pub fallback_owner_user_id: &'a UserId,
    pub scope: &'a TurnScope,
    pub run_id: TurnRunId,
    /// The gate the run is parked on. Typed for the same reason the sibling
    /// [`ApprovalPromptContextSource::approval_prompt_context`] is: every
    /// producer already holds a `TurnGateRef` (the run state's `gate_ref`), and
    /// a bare `&str` here let any other string in the request — the body, an
    /// id — take its place without a compile error.
    pub gate_ref: &'a TurnGateRef,
    /// Invocation the blocked capability ran under, when the renderer has it
    /// (the projection layer does; the delivery path renders without one).
    pub invocation_id: Option<InvocationId>,
    pub body: String,
    pub credential_requirements: &'a [RuntimeCredentialAuthRequirement],
}

/// Approval-gate context enrichment: resolves WHAT is being approved
/// (tool/action/reason) for a gate ref — the same source the WebUI gate
/// projection reads. Implemented over the approval request store; `None`
/// results degrade prompts to generic wording.
#[async_trait]
pub trait ApprovalPromptContextSource: Send + Sync {
    async fn approval_prompt_context(
        &self,
        gate_ref: &TurnGateRef,
        owner_user_id: &UserId,
        scope: &TurnScope,
    ) -> Option<ApprovalPromptContextView>;
}

/// Auth-prompt enrichment: resolves the challenge (OAuth authorization URL
/// vs manual credential entry) for a run blocked on auth. Implemented over the
/// auth engine and the host-issued pairing registry.
#[async_trait]
pub trait BlockedAuthPromptSource: Send + Sync {
    async fn auth_prompt_for_blocked_run(
        &self,
        request: BlockedAuthPromptRequest<'_>,
    ) -> Result<AuthPromptView, ProductAdapterError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_host_api::ids::{TenantId, ThreadId};

    static_assertions::assert_obj_safe!(ApprovalPromptContextSource, BlockedAuthPromptSource);

    struct NoEnrichment;

    #[async_trait]
    impl ApprovalPromptContextSource for NoEnrichment {
        async fn approval_prompt_context(
            &self,
            _gate_ref: &TurnGateRef,
            _owner_user_id: &UserId,
            _scope: &TurnScope,
        ) -> Option<ApprovalPromptContextView> {
            None
        }
    }

    fn scope() -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant-1").expect("valid tenant"),
            None,
            None,
            ThreadId::new("thread-1").expect("valid thread"),
        )
    }

    #[tokio::test]
    async fn absent_approval_context_degrades_to_none_instead_of_erroring() {
        // The port returns `Option`, not `Result`, on purpose: a gate whose
        // context cannot be resolved still has to render a prompt, just a
        // generic one. An error type here would let an enrichment miss take
        // down the whole delivery.
        let source: std::sync::Arc<dyn ApprovalPromptContextSource> =
            std::sync::Arc::new(NoEnrichment);
        let gate_ref = TurnGateRef::new("gate-1").expect("valid gate ref");
        let owner = UserId::new("user-1").expect("valid user");
        assert!(
            source
                .approval_prompt_context(&gate_ref, &owner, &scope())
                .await
                .is_none()
        );
    }

    #[test]
    fn a_blocked_auth_request_borrows_its_scope_and_requirements() {
        // The request is a borrow-only view by design — it is built per render
        // and must not force the caller to clone the scope or the requirement
        // slice onto the heap for a prompt that may never be sent.
        let owner = UserId::new("user-1").expect("valid user");
        let scope = scope();
        // The gate ref is typed, matching the sibling port above: both prompt
        // sources now take the gate the same way, so a renderer cannot hand
        // one of them the run's body or an id where the gate belongs.
        let gate_ref = TurnGateRef::new("gate-1").expect("valid gate ref");
        let request = BlockedAuthPromptRequest {
            fallback_owner_user_id: &owner,
            scope: &scope,
            run_id: TurnRunId::new(),
            gate_ref: &gate_ref,
            invocation_id: None,
            body: "needs auth".to_string(),
            credential_requirements: &[],
        };
        assert_eq!(request.fallback_owner_user_id, &owner);
        assert_eq!(request.gate_ref, &gate_ref);
        assert_eq!(request.gate_ref.as_str(), "gate-1");
        assert!(request.credential_requirements.is_empty());
        assert!(request.invocation_id.is_none());
    }
}

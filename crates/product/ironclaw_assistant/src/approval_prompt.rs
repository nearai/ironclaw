//! Shared approval prompt lookup and redacted context projection.

use crate::ApprovalPromptContextView;
use ironclaw_approvals::ApprovalRequestStorePort;
use ironclaw_approvals::ApprovalStoreError;
use ironclaw_host_api::ids::{InvocationId, UserId};
use ironclaw_host_api::turn::{TurnGateRef, TurnScope};
use ironclaw_product_contracts::approval_prompt::{
    approval_prompt_context_for_request, approval_prompt_lookup_scope,
    approval_request_id_from_gate_ref,
};
use thiserror::Error;

#[derive(Debug, Default)]
pub struct ApprovalPromptLookup {
    pub context: Option<ApprovalPromptContextView>,
    pub invocation_id: Option<InvocationId>,
}

#[derive(Debug, Error)]
#[error("approval prompt context is temporarily unavailable")]
pub struct ApprovalPromptLookupError {
    #[source]
    source: ApprovalStoreError,
}

pub async fn approval_prompt_lookup(
    approval_requests: Option<&dyn ApprovalRequestStorePort>,
    gate_ref: &TurnGateRef,
    owner_user_id: &UserId,
    turn_scope: &TurnScope,
) -> Result<ApprovalPromptLookup, ApprovalPromptLookupError> {
    let (store, request_id) =
        match approval_requests.zip(approval_request_id_from_gate_ref(gate_ref)) {
            Some(value) => value,
            None => return Ok(ApprovalPromptLookup::default()),
        };
    let scope = approval_prompt_lookup_scope(turn_scope, owner_user_id);
    match store.get(&scope, request_id).await {
        Ok(Some(record)) => Ok(ApprovalPromptLookup {
            context: approval_prompt_context_for_request(&record.request),
            invocation_id: Some(record.scope.invocation_id),
        }),
        Ok(None) => Ok(ApprovalPromptLookup::default()),
        Err(source) => Err(ApprovalPromptLookupError { source }),
    }
}

pub async fn approval_prompt_context_view(
    approval_requests: Option<&dyn ApprovalRequestStorePort>,
    gate_ref: &TurnGateRef,
    owner_user_id: &UserId,
    turn_scope: &TurnScope,
) -> Result<Option<ApprovalPromptContextView>, ApprovalPromptLookupError> {
    approval_prompt_lookup(approval_requests, gate_ref, owner_user_id, turn_scope)
        .await
        .map(|lookup| lookup.context)
}

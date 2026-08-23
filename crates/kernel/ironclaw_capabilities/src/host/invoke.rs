//! Workflow 1 — `invoke_json`: the fresh, inline capability invocation.
//!
//! Owns the caller-facing entry point only: it hands the decision to
//! [`super::authorize`], then dispatches, completes obligations, and maps the
//! fold back to a [`CapabilityDispatchResult`]. It never decides policy.

use ironclaw_host_api::{
    authorized::AuthorizeResult,
    dispatch::{CapabilityDispatchResult, CapabilityDispatcher},
    ids::CapabilityId,
    resource::ResourceEstimate,
    scope::ExecutionContext,
};
use tracing::debug;

use super::error_mapping::{
    enrich_dispatch_error_credential_requirements, obligation_invocation_error_kind,
};
use super::{
    AuthorizeFold, AuthorizedFold, CapabilityHost, InvocationInput, authorized_dispatch_witness,
};
use crate::helpers::{
    apply_invocation_state_transition_if_configured, complete_invocation_after_side_effect,
    fail_invocation_if_configured,
};
use crate::{CapabilityInvocationError, CapabilityObligationOutcome, CapabilityObligationPhase};

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    #[tracing::instrument(
        level = "debug",
        skip(self, input),
        fields(
            invocation_id = %context.invocation_id,
            capability_id = %capability_id,
            scope = ?context.resource_scope,
        )
    )]
    pub async fn invoke_json(
        &self,
        context: ExecutionContext,
        capability_id: CapabilityId,
        estimate: ResourceEstimate,
        input: serde_json::Value,
    ) -> Result<CapabilityDispatchResult, CapabilityInvocationError> {
        let request = InvocationInput {
            context,
            capability_id,
            estimate,
            input,
        };
        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        let scope = request.context.resource_scope.clone();

        // The whole pre-dispatch authority fold — context validation,
        // fingerprint, process-invocation start, capability lookup, trust-aware
        // authorization, obligation preparation, and (Slice C) minting the
        // sealed `Authorized` witness — is one method. `invoke_json` maps its
        // `AuthorizeResult` back to today's exact dispatch and error behavior.
        let (obligations, obligation_outcome, authorized) = match self.authorize(&request).await? {
            AuthorizeFold::Authorized(fold) => {
                let AuthorizedFold {
                    result,
                    frozen_deadline: _,
                    obligations,
                    obligation_outcome,
                } = *fold;
                debug!(
                    authorize_result = ?result.as_ref().map(AuthorizeResult::kind),
                    obligation_count = obligations.len(),
                    "capability authorization allowed dispatch"
                );
                let authorized = match authorized_dispatch_witness(result, &capability_id) {
                    Ok(authorized) => authorized,
                    Err(error) => {
                        self.abort_obligations(
                            CapabilityObligationPhase::Invoke,
                            &request.context,
                            &request.capability_id,
                            &request.estimate,
                            obligations.as_slice(),
                            &obligation_outcome,
                        )
                        .await;
                        apply_invocation_state_transition_if_configured(
                            self.invocation_state,
                            &scope,
                            invocation_id,
                            &error,
                        )
                        .await;
                        return Err(error);
                    }
                };
                (obligations, obligation_outcome, authorized)
            }
            AuthorizeFold::Denied { result, reason } => {
                debug!(
                    authorize_result = %result.kind(),
                    reason = ?reason,
                    "capability authorization denied dispatch"
                );
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                    detail: None,
                });
            }
            AuthorizeFold::Blocked { result } => {
                debug!(
                    authorize_result = %result.kind(),
                    "capability authorization requires approval"
                );
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: request.capability_id,
                });
            }
        };

        debug!("capability dispatch starting");
        let dispatch = match self.dispatcher.dispatch_json(*authorized).await {
            Ok(dispatch) => {
                debug!(
                    provider = %dispatch.provider,
                    runtime = ?dispatch.runtime,
                    "capability dispatch completed"
                );
                dispatch
            }
            Err(error) => {
                debug!(
                    dispatch_failure_kind = %error.failure_kind(),
                    "capability dispatch failed"
                );
                self.abort_obligations(
                    CapabilityObligationPhase::Invoke,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                let error =
                    enrich_dispatch_error_credential_requirements(error, obligations.as_slice());
                let invocation_error = CapabilityInvocationError::from(error);
                apply_invocation_state_transition_if_configured(
                    self.invocation_state,
                    &scope,
                    invocation_id,
                    &invocation_error,
                )
                .await;
                return Err(invocation_error);
            }
        };

        let dispatch = match self
            .complete_dispatch_obligations(
                CapabilityObligationPhase::Invoke,
                &request.context,
                &request.capability_id,
                &request.estimate,
                obligations.as_slice(),
                &dispatch,
            )
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                debug!(
                    error_kind = obligation_invocation_error_kind(&error),
                    "capability invoke obligation completion failed"
                );
                let cleanup_outcome = CapabilityObligationOutcome::default();
                self.abort_obligations(
                    CapabilityObligationPhase::Invoke,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &cleanup_outcome,
                )
                .await;
                fail_invocation_if_configured(
                    self.invocation_state,
                    &scope,
                    invocation_id,
                    obligation_invocation_error_kind(&error),
                )
                .await;
                return Err(error);
            }
        };

        if let Some(invocation_state) = self.invocation_state {
            complete_invocation_after_side_effect(
                invocation_state,
                &scope,
                invocation_id,
                &capability_id,
                "dispatch",
            )
            .await;
            debug!("capability invocation state completed");
        }

        debug!("capability invocation completed");
        Ok(dispatch)
    }
}

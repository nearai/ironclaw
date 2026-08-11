//! The three obligation seams the workflows call around dispatch.
//!
//! `prepare` before the side effect, `complete` after it succeeds, `abort`
//! after it fails. This module is only the seam — the obligation *handler*
//! lives behind [`CapabilityObligationHandler`] in `crate::obligations`; the
//! rule is that no workflow calls a handler directly.

use ironclaw_host_api::{
    decision::Obligation,
    dispatch::{CapabilityDispatchResult, CapabilityDispatcher},
    resource::ResourceEstimate,
    scope::ExecutionContext,
};
use tracing::warn;

use super::CapabilityHost;
use super::error_mapping::{
    completion_obligation_error_to_invocation, prepare_obligation_error_to_invocation,
};
use crate::obligations::post_dispatch_obligations;
use crate::{
    CapabilityInvocationError, CapabilityObligationAbortRequest,
    CapabilityObligationCompletionRequest, CapabilityObligationOutcome, CapabilityObligationPhase,
    CapabilityObligationRequest,
};

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    pub(super) async fn prepare_obligations(
        &self,
        phase: CapabilityObligationPhase,
        context: &ExecutionContext,
        capability_id: &ironclaw_host_api::ids::CapabilityId,
        estimate: &ResourceEstimate,
        obligations: Vec<Obligation>,
    ) -> Result<CapabilityObligationOutcome, CapabilityInvocationError> {
        if obligations.is_empty() {
            return Ok(CapabilityObligationOutcome::default());
        }
        if matches!(phase, CapabilityObligationPhase::Spawn) {
            let unsupported = post_dispatch_obligations(&obligations);
            if !unsupported.is_empty() {
                return Err(CapabilityInvocationError::UnsupportedObligations {
                    capability: capability_id.clone(),
                    obligations: unsupported,
                });
            }
        }
        let Some(handler) = self.obligation_handler else {
            return Err(CapabilityInvocationError::UnsupportedObligations {
                capability: capability_id.clone(),
                obligations,
            });
        };
        handler
            .prepare(CapabilityObligationRequest {
                phase,
                context,
                capability_id,
                estimate,
                obligations: obligations.as_slice(),
            })
            .await
            .map_err(|error| prepare_obligation_error_to_invocation(capability_id, error))
    }

    pub(super) async fn complete_dispatch_obligations(
        &self,
        phase: CapabilityObligationPhase,
        context: &ExecutionContext,
        capability_id: &ironclaw_host_api::ids::CapabilityId,
        estimate: &ResourceEstimate,
        obligations: &[Obligation],
        dispatch: &CapabilityDispatchResult,
    ) -> Result<CapabilityDispatchResult, CapabilityInvocationError> {
        if obligations.is_empty() {
            return Ok(dispatch.clone());
        }
        let Some(handler) = self.obligation_handler else {
            let unsupported = post_dispatch_obligations(obligations);
            if unsupported.is_empty() {
                return Ok(dispatch.clone());
            }
            return Err(CapabilityInvocationError::UnsupportedObligations {
                capability: capability_id.clone(),
                obligations: unsupported,
            });
        };
        handler
            .complete_dispatch(CapabilityObligationCompletionRequest {
                phase,
                context,
                capability_id,
                estimate,
                obligations,
                dispatch,
            })
            .await
            .map_err(|error| completion_obligation_error_to_invocation(capability_id, error))
    }

    pub(super) async fn abort_obligations(
        &self,
        phase: CapabilityObligationPhase,
        context: &ExecutionContext,
        capability_id: &ironclaw_host_api::ids::CapabilityId,
        estimate: &ResourceEstimate,
        obligations: &[Obligation],
        outcome: &CapabilityObligationOutcome,
    ) {
        if obligations.is_empty() {
            return;
        }
        let Some(handler) = self.obligation_handler else {
            return;
        };
        if let Err(error) = handler
            .abort(CapabilityObligationAbortRequest {
                phase,
                context,
                capability_id,
                estimate,
                obligations,
                outcome,
            })
            .await
        {
            warn!(
                capability_id = %capability_id,
                error = %error,
                "obligation abort failed after downstream side-effect failure",
            );
        }
    }
}

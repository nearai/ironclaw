//! Canonical mapping from inline capability-host responses to runtime outcomes.
//!
//! Fresh invocation, approval resume, and auth resume all cross this seam. The
//! processor owns response interpretation only; authorization, dispatch,
//! obligation settlement, and durable/model projection remain with their
//! existing owners.

use ironclaw_capabilities::CapabilityInvocationError;
use ironclaw_extension_registry::ExtensionRegistry;
use ironclaw_host_api::{
    dispatch::CapabilityDispatchResult,
    ids::{CapabilityId, InvocationId},
    resource::ResourceScope,
    result_meta::FailureKind,
};

use crate::production::{
    DefaultHostRuntime, auth_required_outcome, capability_is_standard_write, failure_from,
};
use crate::{
    HostRuntimeError, RuntimeApprovalGate, RuntimeBlockedReason, RuntimeCapabilityCompleted,
    RuntimeCapabilityFailure, RuntimeCapabilityOutcome,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InlineInvocationMode {
    Fresh,
    ApprovalResume,
    AuthResume,
}

pub(super) struct CapabilityResponseContext<'a> {
    pub registry: &'a ExtensionRegistry,
    pub capability_id: CapabilityId,
    pub scope: &'a ResourceScope,
    pub invocation_id: InvocationId,
    pub mode: InlineInvocationMode,
}

pub(super) async fn process_capability_response(
    runtime: &DefaultHostRuntime,
    context: CapabilityResponseContext<'_>,
    response: Result<CapabilityDispatchResult, CapabilityInvocationError>,
) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
    match response {
        Ok(dispatch) => Ok(completed_or_output_violation_outcome(
            dispatch,
            context.capability_id,
            context.registry,
        )),
        Err(CapabilityInvocationError::AuthorizationRequiresAuth {
            capability,
            required_secrets,
            credential_requirements,
        }) => Ok(auth_required_outcome(
            capability,
            required_secrets,
            credential_requirements,
        )),
        Err(CapabilityInvocationError::AuthorizationRequiresApproval { capability }) => {
            match context.mode {
                InlineInvocationMode::Fresh => match runtime
                    .lookup_approval_request_id(context.scope, context.invocation_id)
                    .await
                {
                    Ok(Some(approval_request_id)) => Ok(
                        RuntimeCapabilityOutcome::ApprovalRequired(RuntimeApprovalGate {
                            approval_request_id,
                            capability_id: capability,
                            reason: RuntimeBlockedReason::ApprovalRequired,
                        }),
                    ),
                    Ok(None) => Ok(RuntimeCapabilityOutcome::Failed(
                        RuntimeCapabilityFailure::new(
                            capability,
                            FailureKind::Authorization,
                            Some(
                                "approval required but no approval request was persisted"
                                    .to_string(),
                            ),
                        ),
                    )),
                    Err(host_error) => {
                        tracing::warn!(
                            capability_id = %capability,
                            error = %host_error,
                            "approval request lookup failed; surfacing as host runtime unavailability"
                        );
                        Err(host_error)
                    }
                },
                // A resume must never start a second approval loop. Surface a
                // failed resume if the capability kernel asks for approval again.
                InlineInvocationMode::ApprovalResume | InlineInvocationMode::AuthResume => {
                    Ok(RuntimeCapabilityOutcome::Failed(failed_response(
                        CapabilityInvocationError::AuthorizationRequiresApproval { capability },
                        context.registry,
                        context.capability_id,
                    )))
                }
            }
        }
        Err(error) => {
            let should_fail_dispatch_run = context.mode == InlineInvocationMode::Fresh
                && matches!(error, CapabilityInvocationError::Dispatch { .. });
            let failure = failed_response(error, context.registry, context.capability_id);
            if should_fail_dispatch_run {
                runtime
                    .fail_dispatch_run(&failure, context.scope, context.invocation_id)
                    .await;
            }
            Ok(RuntimeCapabilityOutcome::Failed(failure))
        }
    }
}

fn failed_response(
    error: CapabilityInvocationError,
    registry: &ExtensionRegistry,
    capability_id: CapabilityId,
) -> RuntimeCapabilityFailure {
    let is_standard_write = capability_is_standard_write(registry, &capability_id);
    failure_from(error, capability_id).with_is_standard_write(is_standard_write)
}

fn completed_or_output_violation_outcome(
    dispatch: CapabilityDispatchResult,
    capability_id: CapabilityId,
    registry: &ExtensionRegistry,
) -> RuntimeCapabilityOutcome {
    let standard_op = registry
        .get_capability(&capability_id)
        .and_then(|descriptor| descriptor.standard_op);
    let completed = RuntimeCapabilityCompleted {
        capability_id: capability_id.clone(),
        output: dispatch.output,
        display_preview: dispatch.display_preview,
        usage: dispatch.usage,
    };

    if let Some(op) = standard_op
        && let Some(issues) =
            crate::standard_op_output::standard_op_output_violations(op, &completed.output)
    {
        return RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure::new(
            capability_id,
            FailureKind::InvalidResult,
            Some(format!(
                "standard op output failed validation: {}",
                issues.join("; ")
            )),
        ));
    }

    RuntimeCapabilityOutcome::Completed(Box::new(completed))
}

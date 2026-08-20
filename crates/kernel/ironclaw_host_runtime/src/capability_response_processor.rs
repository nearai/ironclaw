//! Canonical mapping from inline capability-host responses to runtime outcomes.
//!
//! Fresh invocation, approval resume, and auth resume all cross this seam; the
//! spawn path also reuses its error mapping while keeping successful spawn
//! outcomes in `spawn_capability`. The processor owns response interpretation
//! only; authorization, dispatch, obligation settlement, and durable/model
//! projection remain with their existing owners.

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
            requirement,
        }) => {
            let ironclaw_host_api::dispatch::DispatchAuthRequirement {
                required_secrets,
                credential_requirements,
                model_visible_cause,
            } = *requirement;
            Ok(auth_required_outcome(
                capability,
                required_secrets,
                credential_requirements,
                model_visible_cause.map(Box::new),
            ))
        }
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
                        // Surface persistence outages as Unavailable rather than
                        // pretending the approval was never persisted; otherwise a
                        // transient run-state failure looks indistinguishable from
                        // the (separately bug-prone) cap-host-skipped-persist path.
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
            // Dispatch failures are model-visible. Fresh invocations use a
            // best-effort durable transition because the corresponding record
            // may already be absent; replacing the actionable provider failure
            // with HostRuntimeError::Unavailable would hide the real cause.
            // Resumed invocations own an existing blocked record, so a failed
            // terminal transition must propagate as host unavailability rather
            // than leaving that record eligible for a later resume.
            let is_dispatch_error = matches!(error, CapabilityInvocationError::Dispatch { .. });
            let failure = failed_response(error, context.registry, context.capability_id);
            if is_dispatch_error {
                let transition = runtime
                    .fail_dispatch_run(&failure, context.scope, context.invocation_id)
                    .await;
                if matches!(
                    context.mode,
                    InlineInvocationMode::ApprovalResume | InlineInvocationMode::AuthResume
                ) {
                    transition?;
                }
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

/// Single choke point for every path that turns a successful capability
/// dispatch into a `Completed` outcome. [`process_capability_response`] above
/// has exactly three callers — `invoke_capability` (Fresh), `resume_capability`
/// (ApprovalResume), and `auth_resume_capability` (AuthResume), the only
/// resume paths that can complete a capability rather than suspend or fail it
/// — and all three route their successful-dispatch case through this function
/// instead of constructing `Completed` themselves, so the standard-op output
/// check cannot be skipped on one entry path while covered on another (see
/// `.claude/rules/review-discipline.md`).
///
/// A capability bound to a standard messaging op (`descriptor.standard_op`)
/// has its dispatch output checked against that op's canonical output schema
/// before `Completed` is allowed to stick. A violation becomes a
/// model-visible `Failed` outcome instead, using the same
/// [`FailureKind::InvalidResult`] kind wasm `InvalidResult` dispatch
/// errors already produce, so the model can retry or report rather than the
/// run completing with a shape no downstream consumer validated. Bespoke
/// capabilities (`standard_op: None`) and an unknown capability id
/// (descriptor lookup miss — already errors elsewhere) are returned untouched.
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

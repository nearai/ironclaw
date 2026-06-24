use ironclaw_approvals::ApprovalResolver;
use ironclaw_host_api::{Action, CapabilityId, ExecutionContext, ResourceEstimate};
use ironclaw_host_runtime::{
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest,
    RuntimeFailureKind,
};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_trust::TrustDecision;

use crate::{
    RebornServices,
    local_dev_capability_policy::{
        LocalDevApprovalPolicyAction, LocalDevCapabilityPolicyError,
        local_dev_one_shot_lease_approval,
    },
};

pub(crate) async fn invoke_json_with_local_dev_approval(
    services: &RebornServices,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
    trust_decision: TrustDecision,
) -> Result<serde_json::Value, RuntimeFailureKind> {
    match invoke_with_local_dev_approval(services, capability_id, context, input, trust_decision)
        .await
    {
        RuntimeCapabilityOutcome::Completed(completed) => Ok(completed.output),
        RuntimeCapabilityOutcome::Failed(failure) => Err(failure.kind),
        other => panic!("unexpected runtime outcome: {other:?}"),
    }
}

pub(crate) async fn invoke_with_local_dev_approval(
    services: &RebornServices,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
    trust_decision: TrustDecision,
) -> RuntimeCapabilityOutcome {
    let runtime = services
        .host_runtime
        .as_ref()
        .expect("host runtime composed"); // safety: test-only helper in #[cfg(test)] module.
    let local_runtime = services
        .local_runtime
        .as_ref()
        .expect("local-dev runtime substrate"); // safety: test-only helper in #[cfg(test)] module.
    let capability = CapabilityId::new(capability_id).expect("valid capability id"); // safety: test-only helper in #[cfg(test)] module.
    let estimate = ResourceEstimate::default();
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability.clone(),
            estimate.clone(),
            input.clone(),
            trust_decision.clone(),
        ))
        .await
        .expect("runtime invocation completes"); // safety: test-only helper in #[cfg(test)] module.
    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => {
            let approval_record = local_runtime
                .approval_requests
                .get(&context.resource_scope, gate.approval_request_id)
                .await
                .expect("local-dev approval record read") // safety: test-only helper in #[cfg(test)] module.
                .expect("local-dev approval request persisted"); // safety: test-only helper in #[cfg(test)] module.
            let policy_action = LocalDevApprovalPolicyAction::from_host_action(
                approval_record.request.action.as_ref(),
            )
            .expect("dispatch or spawn action in local-dev approval"); // safety: test-only approval helper compiled only under #[cfg(test)].
            // For local-dev builtin capabilities, derive lease terms through the
            // capability policy (single source of truth, can't drift from production).
            // For extension capabilities not registered in the builtin policy (e.g.
            // third-party skills like gsuite), fall back to the execution context grants.
            let approval = match local_runtime.capability_policy.lease_approval_for(
                policy_action,
                &local_runtime.workspace_mounts,
                &local_runtime.skill_mounts,
                &local_runtime.memory_mounts,
                &local_runtime.system_extensions_lifecycle_mounts,
            ) {
                Ok(approval) => approval,
                Err(LocalDevCapabilityPolicyError::MissingGrant { .. }) => {
                    lease_approval_from_context(&context, &capability)
                }
                Err(error) => {
                    panic!("capability policy lease approval failed for {capability}: {error}")
                }
            };
            let resolver = ApprovalResolver::new(
                local_runtime.approval_requests.as_ref(),
                local_runtime.capability_leases.as_ref(),
            );
            match approval_record.request.action.as_ref() {
                Action::Dispatch { .. } => resolver
                    .approve_dispatch(&context.resource_scope, gate.approval_request_id, approval)
                    .await
                    .expect("local-dev approval issues dispatch resume lease"), // safety: test-only helper in #[cfg(test)] module.
                Action::SpawnCapability { .. } => resolver
                    .approve_spawn(&context.resource_scope, gate.approval_request_id, approval)
                    .await
                    .expect("local-dev approval issues spawn resume lease"), // safety: test-only helper in #[cfg(test)] module.
                other => panic!("unexpected local-dev approval action: {other:?}"),
            };

            runtime
                .resume_capability(RuntimeCapabilityResumeRequest::new(
                    context,
                    gate.approval_request_id,
                    capability,
                    estimate,
                    input,
                    trust_decision,
                ))
                .await
                .expect("approved runtime invocation resumes") // safety: test-only helper in #[cfg(test)] module.
        }
        other => other,
    }
}

/// Fallback: build a `LeaseApproval` from an extension capability's grant in
/// the execution context. Used only when the capability is not registered in the
/// local-dev builtin policy (e.g. third-party extension skills).
fn lease_approval_from_context(
    context: &ExecutionContext,
    capability: &CapabilityId,
) -> ironclaw_approvals::LeaseApproval {
    let constraints = context
        .grants
        .grants
        .iter()
        .find(|grant| &grant.capability == capability)
        .expect("matching test capability grant") // safety: test-only helper in #[cfg(test)] module.
        .constraints
        .clone();
    local_dev_one_shot_lease_approval(constraints)
}

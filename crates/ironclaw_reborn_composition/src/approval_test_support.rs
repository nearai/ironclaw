use ironclaw_approvals::{ApprovalResolver, AutoApproveSettingInput, AutoApproveSettingStore};
use ironclaw_host_api::MountView;
use ironclaw_host_api::{Action, CapabilityId, ExecutionContext, Principal, ResourceEstimate};
use ironclaw_host_runtime::{
    HostRuntime, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeCapabilityResumeRequest, RuntimeFailureKind,
};
use ironclaw_run_state::ApprovalRequestStore;
use std::sync::Arc;

use crate::RebornRuntime;
use crate::builtin_capability_policy::{
    BuiltinApprovalPolicyAction, BuiltinCapabilityPolicyError, builtin_one_shot_lease_approval,
};
use crate::factory::{
    ComposedApprovalRequestStore, ComposedCapabilityLeaseStore, RebornRuntimeSubstrate,
    RebornRuntimeSurfaces,
};

pub(crate) trait LocalDevApprovalHarness {
    fn host_runtime(&self) -> Option<&Arc<dyn HostRuntime>>;
    fn approval_requests(&self) -> Option<&Arc<ComposedApprovalRequestStore>>;
    fn capability_leases(&self) -> Option<&Arc<ComposedCapabilityLeaseStore>>;
    fn capability_policy(
        &self,
    ) -> Option<&Arc<crate::builtin_capability_policy::BuiltinCapabilityPolicy>>;
    fn workspace_mounts(&self) -> Option<&MountView>;
    fn skill_mounts(&self) -> Option<&MountView>;
    fn memory_mounts(&self) -> Option<&MountView>;
    fn system_extensions_lifecycle_mounts(&self) -> Option<&MountView>;
}

impl LocalDevApprovalHarness for RebornRuntime {
    fn host_runtime(&self) -> Option<&Arc<dyn HostRuntime>> {
        self.host_runtime.as_ref()
    }

    fn approval_requests(&self) -> Option<&Arc<ComposedApprovalRequestStore>> {
        self.approval_requests.as_ref()
    }

    fn capability_leases(&self) -> Option<&Arc<ComposedCapabilityLeaseStore>> {
        self.capability_leases.as_ref()
    }

    fn capability_policy(
        &self,
    ) -> Option<&Arc<crate::builtin_capability_policy::BuiltinCapabilityPolicy>> {
        self.capability_policy.as_ref()
    }

    fn workspace_mounts(&self) -> Option<&MountView> {
        self.workspace_mounts.as_ref()
    }

    fn skill_mounts(&self) -> Option<&MountView> {
        self.skill_mounts.as_ref()
    }

    fn memory_mounts(&self) -> Option<&MountView> {
        self.memory_mounts.as_ref()
    }

    fn system_extensions_lifecycle_mounts(&self) -> Option<&MountView> {
        self.system_extensions_lifecycle_mounts.as_ref()
    }
}

impl LocalDevApprovalHarness for RebornRuntimeSubstrate {
    fn host_runtime(&self) -> Option<&Arc<dyn HostRuntime>> {
        Some(&self.host_runtime)
    }

    fn approval_requests(&self) -> Option<&Arc<ComposedApprovalRequestStore>> {
        Some(&self.runtime_surfaces.as_ref()?.approval_requests)
    }

    fn capability_leases(&self) -> Option<&Arc<ComposedCapabilityLeaseStore>> {
        Some(&self.runtime_surfaces.as_ref()?.capability_leases)
    }

    fn capability_policy(
        &self,
    ) -> Option<&Arc<crate::builtin_capability_policy::BuiltinCapabilityPolicy>> {
        Some(&self.runtime_surfaces.as_ref()?.capability_policy)
    }

    fn workspace_mounts(&self) -> Option<&MountView> {
        Some(&self.runtime_surfaces.as_ref()?.workspace_mounts)
    }

    fn skill_mounts(&self) -> Option<&MountView> {
        Some(&self.runtime_surfaces.as_ref()?.skill_mounts)
    }

    fn memory_mounts(&self) -> Option<&MountView> {
        Some(&self.runtime_surfaces.as_ref()?.memory_mounts)
    }

    fn system_extensions_lifecycle_mounts(&self) -> Option<&MountView> {
        Some(
            &self
                .runtime_surfaces
                .as_ref()?
                .system_extensions_lifecycle_mounts,
        )
    }
}

/// Turn the global auto-approve switch off for `context`'s actor scope.
/// Global auto-approve defaults ON, so any test exercising the per-tool approval
/// gate must flip it off first. Shared by every `src` `#[cfg(test)]` site;
/// integration-test and root-crate binaries keep their own copies (they cannot
/// see this crate-internal helper).
pub(crate) async fn disable_global_auto_approve(
    runtime_surfaces: &RebornRuntimeSurfaces,
    context: &ExecutionContext,
) {
    runtime_surfaces
        .auto_approve_settings
        .set(AutoApproveSettingInput {
            scope: context.resource_scope.clone(),
            enabled: false,
            updated_by: Principal::User(context.resource_scope.user_id.clone()),
        })
        .await
        .expect("disable global auto-approve"); // safety: test-only gating precondition
}

pub(crate) async fn invoke_json_with_local_dev_approval(
    runtime: &impl LocalDevApprovalHarness,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
) -> Result<serde_json::Value, RuntimeFailureKind> {
    match invoke_with_local_dev_approval(runtime, capability_id, context, input).await {
        RuntimeCapabilityOutcome::Completed(completed) => Ok(completed.output),
        RuntimeCapabilityOutcome::Failed(failure) => Err(failure.kind),
        other => panic!("unexpected runtime outcome: {other:?}"),
    }
}

pub(crate) async fn invoke_with_local_dev_approval(
    runtime: &impl LocalDevApprovalHarness,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
) -> RuntimeCapabilityOutcome {
    let host_runtime = runtime.host_runtime().expect("host runtime composed"); // safety: test-only helper in #[cfg(test)] module.
    let approval_requests = runtime
        .approval_requests()
        .expect("local-dev runtime approval store");
    let capability_leases = runtime
        .capability_leases()
        .expect("local-dev runtime capability lease store");
    let capability_policy = runtime
        .capability_policy()
        .expect("local-dev runtime capability policy");
    let workspace_mounts = runtime
        .workspace_mounts()
        .expect("local-dev runtime workspace mounts");
    let skill_mounts = runtime
        .skill_mounts()
        .expect("local-dev runtime skill mounts");
    let memory_mounts = runtime
        .memory_mounts()
        .expect("local-dev runtime memory mounts");
    let system_extensions_lifecycle_mounts = runtime
        .system_extensions_lifecycle_mounts()
        .expect("local-dev runtime system extension lifecycle mounts");
    let capability = CapabilityId::new(capability_id).expect("valid capability id"); // safety: test-only helper in #[cfg(test)] module.
    let estimate = ResourceEstimate::default();
    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context.clone(),
            capability.clone(),
            estimate.clone(),
            input.clone(),
        ))
        .await
        .expect("runtime invocation completes"); // safety: test-only helper in #[cfg(test)] module.
    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => {
            let approval_record = approval_requests
                .get(&context.resource_scope, gate.approval_request_id)
                .await
                .expect("local-dev approval record read") // safety: test-only helper in #[cfg(test)] module.
                .expect("local-dev approval request persisted"); // safety: test-only helper in #[cfg(test)] module.
            let policy_action = BuiltinApprovalPolicyAction::from_host_action(
                approval_record.request.action.as_ref(),
            )
            .expect("dispatch or spawn action in local-dev approval"); // safety: test-only approval helper compiled only under #[cfg(test)].
            // For local-dev builtin capabilities, derive lease terms through the
            // capability policy (single source of truth, can't drift from production).
            // For extension capabilities not registered in the builtin policy (e.g.
            // third-party skills like gsuite), fall back to the execution context grants.
            let approval = match capability_policy.lease_approval_for(
                policy_action,
                workspace_mounts,
                skill_mounts,
                memory_mounts,
                system_extensions_lifecycle_mounts,
            ) {
                Ok(approval) => approval,
                Err(BuiltinCapabilityPolicyError::MissingGrant { .. }) => {
                    lease_approval_from_context(&context, &capability)
                }
                Err(error) => {
                    panic!("capability policy lease approval failed for {capability}: {error}")
                }
            };
            let resolver =
                ApprovalResolver::new(approval_requests.as_ref(), capability_leases.as_ref());
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

            host_runtime
                .resume_capability(RuntimeCapabilityResumeRequest::new(
                    context,
                    gate.approval_request_id,
                    capability,
                    estimate,
                    input,
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
    builtin_one_shot_lease_approval(constraints)
}

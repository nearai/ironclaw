use ironclaw_approvals::ApprovalRequestStorePort as _;
use ironclaw_approvals::{
    ApprovalResolver, AutoApproveSettingInput, AutoApproveSettingStorePort as _,
};
use ironclaw_host_api::mount::MountView;
use ironclaw_host_api::{
    action::Action,
    ids::CapabilityId,
    resource::ResourceEstimate,
    result_meta::FailureKind,
    scope::{ExecutionContext, Principal},
};
use ironclaw_host_runtime::{HostRuntime, RuntimeCapabilityOutcome};
use std::sync::Arc;

use crate::builtin_capability_policy::{
    BuiltinApprovalPolicyAction, BuiltinCapabilityPolicyError, builtin_one_shot_lease_approval,
};
use crate::factory::{
    ComposedApprovalRequestStore, ComposedAutoApproveSettingStore, ComposedCapabilityLeaseStore,
    RebornRuntimeStores,
};

pub(crate) trait ApprovalHarness {
    fn host_runtime(&self) -> Option<&Arc<dyn HostRuntime>>;
    fn approval_requests(&self) -> Option<&Arc<ComposedApprovalRequestStore>>;
    fn capability_leases(&self) -> Option<&Arc<ComposedCapabilityLeaseStore>>;
    fn capability_policy(
        &self,
    ) -> Option<&Arc<crate::builtin_capability_policy::BuiltinCapabilityPolicy>>;
    /// `None` under a per-caller workspace policy: this harness mints lease
    /// terms from a fixed view, which only a shared-workspace deployment has.
    fn workspace_mounts(&self) -> Option<&MountView>;
    fn memory_mounts(&self) -> Option<&MountView>;
    fn system_extensions_lifecycle_mounts(&self) -> Option<&MountView>;
    fn auto_approve_settings(&self) -> Option<&Arc<ComposedAutoApproveSettingStore>>;
}

impl ApprovalHarness for RebornRuntimeStores {
    fn host_runtime(&self) -> Option<&Arc<dyn HostRuntime>> {
        Some(&self.host_runtime)
    }

    fn approval_requests(&self) -> Option<&Arc<ComposedApprovalRequestStore>> {
        Some(&self.approval_requests)
    }

    fn capability_leases(&self) -> Option<&Arc<ComposedCapabilityLeaseStore>> {
        Some(&self.capability_leases)
    }

    fn capability_policy(
        &self,
    ) -> Option<&Arc<crate::builtin_capability_policy::BuiltinCapabilityPolicy>> {
        Some(&self.capability_policy)
    }

    fn workspace_mounts(&self) -> Option<&MountView> {
        crate::factory::test_support::shared_workspace_view(&self.workspace_mounts)
    }

    fn memory_mounts(&self) -> Option<&MountView> {
        Some(&self.memory_mounts)
    }

    fn system_extensions_lifecycle_mounts(&self) -> Option<&MountView> {
        Some(&self.system_extensions_lifecycle_mounts)
    }

    fn auto_approve_settings(&self) -> Option<&Arc<ComposedAutoApproveSettingStore>> {
        Some(&self.auto_approve_settings)
    }
}

/// Turn the global auto-approve switch off for `context`'s actor scope.
/// Global auto-approve defaults ON, so any test exercising the per-tool approval
/// gate must flip it off first. Shared by every crate-internal `#[cfg(test)]`
/// site; integration-test and root-crate binaries keep their own copies (they
/// cannot see this helper).
pub(crate) async fn disable_global_auto_approve(
    runtime: &impl ApprovalHarness,
    context: &ExecutionContext,
) {
    runtime
        .auto_approve_settings()
        .expect("standalone auto-approve store") // safety: test-only helper in #[cfg(test)] module.
        .set(AutoApproveSettingInput {
            scope: context.resource_scope.clone(),
            enabled: false,
            updated_by: Principal::User(context.resource_scope.user_id.clone()),
        })
        .await
        .expect("disable global auto-approve"); // safety: test-only gating precondition
}

pub(crate) async fn invoke_json_with_standalone_approval(
    runtime: &impl ApprovalHarness,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
) -> Result<serde_json::Value, FailureKind> {
    match invoke_with_standalone_approval(runtime, capability_id, context, input).await {
        RuntimeCapabilityOutcome::Completed(completed) => Ok(completed.output),
        RuntimeCapabilityOutcome::Failed(failure) => Err(failure.kind),
        other => panic!("unexpected runtime outcome: {other:?}"),
    }
}

pub(crate) async fn invoke_with_standalone_approval(
    runtime: &impl ApprovalHarness,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
) -> RuntimeCapabilityOutcome {
    let host_runtime = runtime.host_runtime().expect("host runtime composed"); // safety: test-only helper in #[cfg(test)] module.
    let approval_requests = runtime
        .approval_requests()
        .expect("standalone runtime approval store"); // safety: test-only helper in #[cfg(test)] module.
    let capability_leases = runtime
        .capability_leases()
        .expect("standalone runtime capability lease store"); // safety: test-only helper in #[cfg(test)] module.
    let capability_policy = runtime
        .capability_policy()
        .expect("standalone runtime capability policy"); // safety: test-only helper in #[cfg(test)] module.
    let workspace_mounts = runtime
        .workspace_mounts()
        .expect("standalone runtime workspace mounts"); // safety: test-only helper in #[cfg(test)] module.
    // Derived from the invocation's own scope, exactly as `PolicyApprovalLeaseTermsProvider::
    // skill_mounts_for` does in production. Reading a pre-built, scope-free view off the runtime is
    // what let this harness assert lease terms over `/projects/skills` while every skill capability
    // wrote to `/tenants/<t>/users/<u>/skills`.
    let skill_mounts =
        crate::runtime_mounts::db_backed_skill_management_mount_view(&context.resource_scope)
            .expect("standalone skill mounts scope"); // safety: test-only helper in #[cfg(test)] module.
    let memory_mounts = runtime
        .memory_mounts()
        .expect("standalone runtime memory mounts"); // safety: test-only helper in #[cfg(test)] module.
    let system_extensions_lifecycle_mounts = runtime
        .system_extensions_lifecycle_mounts()
        .expect("standalone runtime system extension lifecycle mounts"); // safety: test-only helper in #[cfg(test)] module.
    let capability = CapabilityId::new(capability_id).expect("valid capability id"); // safety: test-only helper in #[cfg(test)] module.
    let estimate = ResourceEstimate::default();
    let outcome = host_runtime
        .invoke_capability((
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
                .expect("standalone approval record read") // safety: test-only helper in #[cfg(test)] module.
                .expect("standalone approval request persisted"); // safety: test-only helper in #[cfg(test)] module.
            let policy_action = BuiltinApprovalPolicyAction::from_host_action(
                approval_record.request.action.as_ref(),
            )
            .expect("dispatch or spawn action in standalone approval"); // safety: test-only approval helper compiled only under #[cfg(test)].
            // For standalone builtin capabilities, derive lease terms through the
            // capability policy (single source of truth, can't drift from production).
            // For extension capabilities not registered in the builtin policy (e.g.
            // third-party skills like gsuite), fall back to the execution context grants.
            let approval = match capability_policy.lease_approval_for(
                policy_action,
                workspace_mounts,
                &skill_mounts,
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
                    .expect("standalone approval issues dispatch resume lease"), // safety: test-only helper in #[cfg(test)] module.
                Action::SpawnCapability { .. } => resolver
                    .approve_spawn(&context.resource_scope, gate.approval_request_id, approval)
                    .await
                    .expect("standalone approval issues spawn resume lease"), // safety: test-only helper in #[cfg(test)] module.
                other => panic!("unexpected standalone approval action: {other:?}"), // safety: test-only helper validates dispatch/spawn actions above.
            };

            host_runtime
                .resume_capability((
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
/// standalone builtin policy (e.g. third-party extension skills).
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

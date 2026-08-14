//! Deployment `[admin_configuration]` writes through the production capability.
//!
//! Mirrors (rather than imports — a separate test binary) the same helper in
//! `extension_delivery.rs`: the values land through
//! `builtin.admin_configuration_replace` on the real host runtime, so secrets
//! reach the scoped secret store and non-secret fields reach the canonical
//! configuration projection with zero test-only config injection.

use ironclaw_host_api::{
    action::NetworkPolicy,
    capability::{CapabilityGrant, CapabilitySet, EffectKind, GrantConstraints},
    ids::{CapabilityGrantId, CapabilityId, CorrelationId, ExtensionId, InvocationId, ProductKind},
    invocation::InvocationOrigin,
    mount::MountView,
    resource::{ResourceEstimate, ResourceScope},
    runtime::{RuntimeKind, TrustClass},
    scope::{ExecutionContext, Principal},
};
use ironclaw_host_runtime::RuntimeCapabilityOutcome;
use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};

/// Replace one administrator-configuration group's values.
///
/// `operator_user_id` must be the composition owner the profile named
/// (`ToolsProfile::service_label`'s user), not the capability executor: admin
/// configuration is deployment-scoped and deliberately writes as the operator.
pub async fn replace_admin_configuration(
    group: &RebornIntegrationGroup,
    operator_user_id: &str,
    group_id: &str,
    expected_revision: u64,
    values: serde_json::Value,
) -> HarnessResult<()> {
    let services = group
        .capability_harness()
        .ok_or("host-runtime capability harness")?
        .reborn_services_for_test()
        .ok_or("composed reborn services")?;
    let operator_user_id = ironclaw_host_api::ids::UserId::new(operator_user_id)?;
    let capability_id = CapabilityId::new("builtin.admin_configuration_replace")?;
    let product_ingress = ExtensionId::new("ironclaw_webui")?;
    let invocation_id = InvocationId::new();
    let runtime_scope = &group.shared.product_harness.scope;
    let runtime_agent_id = runtime_scope
        .agent_id
        .clone()
        .ok_or("device-link profile runtime scope has an agent id")?;
    let scope = ResourceScope {
        tenant_id: runtime_scope.tenant_id.clone(),
        user_id: operator_user_id.clone(),
        agent_id: Some(runtime_agent_id),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    let context = ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: operator_user_id.clone(),
        authenticated_actor_user_id: Some(operator_user_id),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: None,
        thread_id: None,
        run_id: None,
        origin: Some(InvocationOrigin::Product(ProductKind::new("webui")?)),
        extension_id: product_ingress.clone(),
        runtime: RuntimeKind::FirstParty,
        trust: TrustClass::Sandbox,
        grants: CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: capability_id.clone(),
                grantee: Principal::Extension(product_ingress),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![
                        EffectKind::ReadFilesystem,
                        EffectKind::WriteFilesystem,
                        EffectKind::DeleteFilesystem,
                        EffectKind::UseSecret,
                    ],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: Some(1),
                },
            }],
        },
        mounts: MountView::default(),
        resource_scope: scope,
    };
    context
        .validate()
        .map_err(|error| format!("admin capability context does not validate: {error:?}"))?;
    let outcome = services
        .host_runtime_for_test()
        .ok_or("composed host runtime")?
        .invoke_capability((
            context,
            capability_id,
            ResourceEstimate::default(),
            json!({
                "group_id": group_id,
                "expected_revision": expected_revision,
                "values": values,
            }),
        ))
        .await
        .map_err(|error| format!("admin configuration dispatch failed: {error:?}"))?;
    match outcome {
        RuntimeCapabilityOutcome::Completed(_) => Ok(()),
        other => Err(format!(
            "administrator configuration must complete through the authorized runtime, got {other:?}"
        )
        .into()),
    }
}

//! Authorized first-party mutations for operator configuration.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_approvals::{AutoApproveSettingInput, AutoApproveSettingStore};
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, HostApiError, PermissionMode, Principal,
    ResourceEstimate, ResourceProfile, ResourceUsage, RuntimeDispatchErrorKind, UserId,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product_workflow::OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID;
use serde::Deserialize;

pub(crate) fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package.manifest.capabilities.push(manifest()?);
    ExtensionPackage::from_manifest(package.manifest, package.root)
}

pub(crate) fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    auto_approve: Arc<dyn AutoApproveSettingStore>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID)?,
        Arc::new(SetAutoApproveHandler { auto_approve }),
    );
    Ok(())
}

fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID)?,
        implements: Vec::new(),
        description: "Set the authenticated operator's global auto-approve-tools setting."
            .to_string(),
        effects: vec![EffectKind::ModifyApproval],
        default_permission: PermissionMode::Allow,
        visibility: CapabilityVisibility::Api,
        input_schema_ref: CapabilityProfileSchemaRef::new(
            "schemas/builtin/operator_config_set_auto_approve.input.v1.json",
        )?,
        output_schema_ref: Some(CapabilityProfileSchemaRef::new(
            "schemas/builtin/operator_config_set_auto_approve.output.v1.json",
        )?),
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        resource_profile: Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(500)
                .set_output_bytes(1024),
            hard_ceiling: None,
        }),
    })
}

struct SetAutoApproveHandler {
    auto_approve: Arc<dyn AutoApproveSettingStore>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetAutoApproveInput {
    enabled: bool,
}

#[async_trait]
impl FirstPartyCapabilityHandler for SetAutoApproveHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        if request.capability_id.as_str() != OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::UndeclaredCapability,
            )
            .with_usage(resource_usage(started)));
        }
        let Some(actor) = authenticated_operator(&request) else {
            return Err(
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::PolicyDenied)
                    .with_usage(resource_usage(started)),
            );
        };
        let actor = actor.clone();
        let input: SetAutoApproveInput = serde_json::from_value(request.input).map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
                .with_usage(resource_usage(started))
        })?;
        let scope = request.scope.tenant_user_settings_scope();
        let record = self
            .auto_approve
            .set(AutoApproveSettingInput {
                scope,
                enabled: input.enabled,
                updated_by: Principal::User(actor),
            })
            .await
            .map_err(|error| {
                tracing::warn!(%error, "operator auto-approve setting mutation failed");
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
                    .with_usage(resource_usage(started))
            })?;
        Ok(FirstPartyCapabilityResult::new(
            serde_json::json!({
                "key": "agent.auto_approve_tools",
                "enabled": record.enabled,
                "tenant_id": record.key.tenant_id.as_str(),
                "user_id": record.key.user_id.as_str(),
            }),
            resource_usage(started),
        ))
    }
}

fn authenticated_operator(request: &FirstPartyCapabilityRequest) -> Option<&UserId> {
    let actor = request.authenticated_actor_user_id.as_ref()?;
    if actor == &request.scope.user_id {
        Some(actor)
    } else {
        None
    }
}

fn resource_usage(started: Instant) -> ResourceUsage {
    ResourceUsage::default()
        .set_wall_clock_ms(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, InvocationId, ResourceScope, TenantId};

    use super::*;

    #[test]
    fn capability_is_api_only_modify_approval() {
        let manifest = manifest().expect("manifest");
        assert_eq!(manifest.visibility, CapabilityVisibility::Api);
        assert_eq!(manifest.effects, vec![EffectKind::ModifyApproval]);
        assert_eq!(manifest.default_permission, PermissionMode::Allow);
    }

    #[test]
    fn authenticated_operator_must_match_resource_user() {
        let operator = UserId::new("operator").expect("operator");
        let member = UserId::new("member").expect("member");
        let scope = ResourceScope {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            user_id: operator.clone(),
            agent_id: Some(AgentId::new("agent").expect("agent")),
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let mut request = FirstPartyCapabilityRequest::request_for_test(
            CapabilityId::new(OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID)
                .expect("capability id"),
            scope,
            serde_json::json!({ "enabled": true }),
            None,
        );
        request.authenticated_actor_user_id = Some(member);
        assert!(authenticated_operator(&request).is_none());
        request.authenticated_actor_user_id = Some(operator.clone());
        assert_eq!(authenticated_operator(&request), Some(&operator));
    }
}

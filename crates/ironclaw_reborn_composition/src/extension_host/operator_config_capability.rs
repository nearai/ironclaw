//! Authorized first-party mutations for operator configuration.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_approvals::{AutoApproveSettingInput, AutoApproveSettingStore};
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, HostApiError, OriginGateMatrix,
    PermissionMode, Principal, ResourceEstimate, ResourceProfile, ResourceUsage,
    RuntimeDispatchErrorKind, UserId,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product_workflow::OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID;

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
        origin_gate_matrix: Some(OriginGateMatrix::product_consent_only()),
    })
}

struct SetAutoApproveHandler {
    auto_approve: Arc<dyn AutoApproveSettingStore>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for SetAutoApproveHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        ensure_declared(&request, started)?;
        let actor = authenticated_actor(&request, started)?;
        let enabled = parse_enabled(request.input, started)?;
        let scope = request.scope.tenant_user_settings_scope();
        let record = self
            .auto_approve
            .set(AutoApproveSettingInput {
                scope,
                enabled,
                updated_by: Principal::User(actor),
            })
            .await
            .map_err(|error| {
                tracing::warn!(%error, "operator auto-approve setting mutation failed");
                dispatch_error(RuntimeDispatchErrorKind::Backend, started)
            })?;
        Ok(dispatch_result(
            serde_json::json!({
                "key": "agent.auto_approve_tools",
                "enabled": record.enabled,
                "tenant_id": record.key.tenant_id.as_str(),
                "user_id": record.key.user_id.as_str(),
            }),
            started,
        ))
    }
}

fn ensure_declared(
    request: &FirstPartyCapabilityRequest,
    started: Instant,
) -> Result<(), FirstPartyCapabilityError> {
    if request.capability_id.as_str() == OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID {
        Ok(())
    } else {
        Err(dispatch_error(
            RuntimeDispatchErrorKind::UndeclaredCapability,
            started,
        ))
    }
}

fn authenticated_actor(
    request: &FirstPartyCapabilityRequest,
    started: Instant,
) -> Result<UserId, FirstPartyCapabilityError> {
    match request.authenticated_actor_user_id.as_ref() {
        Some(actor) if actor == &request.scope.user_id => Ok(actor.clone()),
        _ => Err(dispatch_error(
            RuntimeDispatchErrorKind::PolicyDenied,
            started,
        )),
    }
}

fn parse_enabled(
    input: serde_json::Value,
    started: Instant,
) -> Result<bool, FirstPartyCapabilityError> {
    let object = input
        .as_object()
        .ok_or_else(|| dispatch_error(RuntimeDispatchErrorKind::InputEncode, started))?;
    let enabled = object
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| dispatch_error(RuntimeDispatchErrorKind::InputEncode, started))?;
    if object.len() == 1 {
        Ok(enabled)
    } else {
        Err(dispatch_error(
            RuntimeDispatchErrorKind::InputEncode,
            started,
        ))
    }
}

fn dispatch_error(kind: RuntimeDispatchErrorKind, started: Instant) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::new(kind).with_usage(resource_usage(started))
}

fn dispatch_result(output: serde_json::Value, started: Instant) -> FirstPartyCapabilityResult {
    FirstPartyCapabilityResult::new(output, resource_usage(started))
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
    fn authenticated_actor_must_match_resource_user() {
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
        assert!(authenticated_actor(&request, Instant::now()).is_err());
        request.authenticated_actor_user_id = Some(operator.clone());
        assert_eq!(
            authenticated_actor(&request, Instant::now()).expect("actor"),
            operator
        );
    }
}

//! Authorized first-party mutations for operator configuration.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_approvals::{
    AutoApproveSettingInput, AutoApproveSettingStorePort, PersistentApprovalAction,
    PersistentApprovalPolicyError, PersistentApprovalPolicyInput, PersistentApprovalPolicyKey,
    PersistentApprovalPolicyStorePort, ToolPermissionOverride, ToolPermissionOverrideInput,
    ToolPermissionOverrideKey, ToolPermissionOverrideStorePort, ToolPermissionState,
};
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, GrantConstraints, HostApiError,
    OriginGateMatrix, PermissionMode, Principal, ResourceEstimate, ResourceProfile, ResourceScope,
    ResourceUsage, RuntimeDispatchErrorKind, UserId,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product::{
    OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID,
    OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID, RebornOperatorToolCatalog,
    RebornOperatorToolInfo,
};

pub(crate) fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package.manifest.capabilities.push(manifest()?);
    package
        .manifest
        .capabilities
        .push(tool_permission_manifest()?);
    ExtensionPackage::from_manifest(package.manifest, package.root)
}

pub(crate) fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    auto_approve: Arc<dyn AutoApproveSettingStorePort>,
    overrides: Arc<dyn ToolPermissionOverrideStorePort>,
    persistent_policies: Arc<dyn PersistentApprovalPolicyStorePort>,
    tool_catalog: Arc<dyn RebornOperatorToolCatalog>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID)?,
        Arc::new(SetAutoApproveHandler { auto_approve }),
    );
    registry.insert_handler(
        CapabilityId::new(OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID)?,
        Arc::new(SetToolPermissionHandler {
            overrides,
            persistent_policies,
            tool_catalog,
        }),
    );
    Ok(())
}

fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID)?,
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
        max_egress_bytes: None,
        resource_profile: Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(500)
                .set_output_bytes(1024),
            hard_ceiling: None,
        }),
        origin_gate_matrix: Some(OriginGateMatrix::product_consent_only()),
    })
}

fn tool_permission_manifest() -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID)?,
        description: "Set the authenticated operator's permission for one tool.".to_string(),
        effects: vec![EffectKind::ModifyApproval],
        default_permission: PermissionMode::Allow,
        visibility: CapabilityVisibility::Api,
        input_schema_ref: CapabilityProfileSchemaRef::new(
            "schemas/builtin/operator_config_set_tool_permission.input.v1.json",
        )?,
        output_schema_ref: Some(CapabilityProfileSchemaRef::new(
            "schemas/builtin/operator_config_set_tool_permission.output.v1.json",
        )?),
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        max_egress_bytes: None,
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
    auto_approve: Arc<dyn AutoApproveSettingStorePort>,
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

struct SetToolPermissionHandler {
    overrides: Arc<dyn ToolPermissionOverrideStorePort>,
    persistent_policies: Arc<dyn PersistentApprovalPolicyStorePort>,
    tool_catalog: Arc<dyn RebornOperatorToolCatalog>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for SetToolPermissionHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        ensure_tool_permission_declared(&request, started)?;
        let actor = authenticated_actor(&request, started)?;
        let input = parse_tool_permission_input(request.input, started)?;
        let tool = find_operator_tool(
            self.tool_catalog.as_ref(),
            &input.capability_id,
            &request.scope.user_id,
            started,
        )
        .await?;
        if tool_permission_locked(&tool) {
            return Err(dispatch_error(
                RuntimeDispatchErrorKind::PolicyDenied,
                started,
            ));
        }
        apply_tool_permission_state(
            self.overrides.as_ref(),
            self.persistent_policies.as_ref(),
            &request.scope,
            &actor,
            &tool,
            input.state,
            started,
        )
        .await?;
        Ok(dispatch_result(
            serde_json::json!({
                "key": format!("tool.{}", input.capability_id.as_str()),
                "capability_id": input.capability_id.as_str(),
                "state": tool_permission_state_wire(input.state),
                "tenant_id": request.scope.tenant_id.as_str(),
                "user_id": request.scope.user_id.as_str(),
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

fn ensure_tool_permission_declared(
    request: &FirstPartyCapabilityRequest,
    started: Instant,
) -> Result<(), FirstPartyCapabilityError> {
    if request.capability_id.as_str() == OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID {
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

struct ToolPermissionInput {
    capability_id: CapabilityId,
    state: ToolPermissionUpdate,
}

#[derive(Clone, Copy)]
enum ToolPermissionUpdate {
    Default,
    State(ToolPermissionState),
}

fn parse_tool_permission_input(
    input: serde_json::Value,
    started: Instant,
) -> Result<ToolPermissionInput, FirstPartyCapabilityError> {
    let object = input
        .as_object()
        .ok_or_else(|| dispatch_error(RuntimeDispatchErrorKind::InputEncode, started))?;
    if object.len() != 2 {
        return Err(dispatch_error(
            RuntimeDispatchErrorKind::InputEncode,
            started,
        ));
    }
    let capability_id = object
        .get("capability_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| dispatch_error(RuntimeDispatchErrorKind::InputEncode, started))
        .and_then(|value| {
            CapabilityId::new(value)
                .map_err(|_| dispatch_error(RuntimeDispatchErrorKind::InputEncode, started))
        })?;
    let state = object
        .get("state")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| dispatch_error(RuntimeDispatchErrorKind::InputEncode, started))
        .and_then(|value| match value {
            "default" => Ok(ToolPermissionUpdate::Default),
            "always_allow" => Ok(ToolPermissionUpdate::State(
                ToolPermissionState::AlwaysAllow,
            )),
            "ask_each_time" | "ask" => Ok(ToolPermissionUpdate::State(
                ToolPermissionState::AskEachTime,
            )),
            "disabled" => Ok(ToolPermissionUpdate::State(ToolPermissionState::Disabled)),
            _ => Err(dispatch_error(
                RuntimeDispatchErrorKind::InputEncode,
                started,
            )),
        })?;
    Ok(ToolPermissionInput {
        capability_id,
        state,
    })
}

async fn find_operator_tool(
    catalog: &dyn RebornOperatorToolCatalog,
    capability_id: &CapabilityId,
    caller: &UserId,
    started: Instant,
) -> Result<RebornOperatorToolInfo, FirstPartyCapabilityError> {
    catalog
        .list_operator_tools(caller)
        .await
        .into_iter()
        .find(|tool| tool.capability_id == *capability_id)
        .ok_or_else(|| dispatch_error(RuntimeDispatchErrorKind::PolicyDenied, started))
}

async fn apply_tool_permission_state(
    overrides: &dyn ToolPermissionOverrideStorePort,
    persistent_policies: &dyn PersistentApprovalPolicyStorePort,
    scope: &ResourceScope,
    actor: &UserId,
    tool: &RebornOperatorToolInfo,
    update: ToolPermissionUpdate,
    started: Instant,
) -> Result<(), FirstPartyCapabilityError> {
    let operator_scope = operator_tool_permission_scope(scope);
    match update {
        ToolPermissionUpdate::Default => {
            revoke_persistent_policy(persistent_policies, &operator_scope, tool, started).await?;
            overrides
                .clear(&ToolPermissionOverrideKey::new(
                    &operator_scope,
                    tool.capability_id.clone(),
                ))
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "operator tool permission override clear failed");
                    dispatch_error(RuntimeDispatchErrorKind::Backend, started)
                })?;
        }
        ToolPermissionUpdate::State(ToolPermissionState::AlwaysAllow) => {
            persistent_policies
                .allow(PersistentApprovalPolicyInput {
                    scope: operator_scope.clone(),
                    action: PersistentApprovalAction::Dispatch,
                    capability_id: tool.capability_id.clone(),
                    grantee: Principal::Extension(tool.provider.clone()),
                    approved_by: Principal::User(actor.clone()),
                    constraints: GrantConstraints {
                        allowed_effects: tool.effects.as_ref().to_vec(),
                        mounts: Default::default(),
                        network: Default::default(),
                        secrets: Vec::new(),
                        resource_ceiling: None,
                        expires_at: None,
                        max_invocations: None,
                    },
                    source_approval_request_id: None,
                })
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "operator persistent approval policy write failed");
                    dispatch_error(RuntimeDispatchErrorKind::Backend, started)
                })?;
            overrides
                .clear(&ToolPermissionOverrideKey::new(
                    &operator_scope,
                    tool.capability_id.clone(),
                ))
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "operator tool permission override clear failed");
                    dispatch_error(RuntimeDispatchErrorKind::Backend, started)
                })?;
        }
        ToolPermissionUpdate::State(state @ ToolPermissionState::AskEachTime)
        | ToolPermissionUpdate::State(state @ ToolPermissionState::Disabled) => {
            revoke_persistent_policy(persistent_policies, &operator_scope, tool, started).await?;
            let override_state = match state {
                ToolPermissionState::AskEachTime => ToolPermissionOverride::AskEachTime,
                ToolPermissionState::Disabled => ToolPermissionOverride::Disabled,
                ToolPermissionState::AlwaysAllow => {
                    return Err(dispatch_error(
                        RuntimeDispatchErrorKind::InputEncode,
                        started,
                    ));
                }
            };
            overrides
                .set(ToolPermissionOverrideInput {
                    scope: operator_scope,
                    capability_id: tool.capability_id.clone(),
                    state: override_state,
                    updated_by: Principal::User(actor.clone()),
                })
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "operator tool permission override write failed");
                    dispatch_error(RuntimeDispatchErrorKind::Backend, started)
                })?;
        }
    }
    Ok(())
}

async fn revoke_persistent_policy(
    persistent_policies: &dyn PersistentApprovalPolicyStorePort,
    operator_scope: &ResourceScope,
    tool: &RebornOperatorToolInfo,
    started: Instant,
) -> Result<(), FirstPartyCapabilityError> {
    match persistent_policies
        .revoke(&persistent_user_policy_key(operator_scope, tool))
        .await
    {
        Ok(_) | Err(PersistentApprovalPolicyError::UnknownPolicy) => Ok(()),
        Err(error) => {
            tracing::warn!(%error, "operator persistent approval policy revoke failed");
            Err(dispatch_error(RuntimeDispatchErrorKind::Backend, started))
        }
    }
}

fn persistent_user_policy_key(
    scope: &ResourceScope,
    tool: &RebornOperatorToolInfo,
) -> PersistentApprovalPolicyKey {
    PersistentApprovalPolicyKey::new(
        scope,
        PersistentApprovalAction::Dispatch,
        tool.capability_id.clone(),
        Principal::Extension(tool.provider.clone()),
    )
}

fn operator_tool_permission_scope(scope: &ResourceScope) -> ResourceScope {
    scope.tenant_user_settings_scope()
}

fn tool_permission_locked(tool: &RebornOperatorToolInfo) -> bool {
    tool.default_permission == PermissionMode::Deny || hard_floor_tool(tool)
}

fn hard_floor_tool(tool: &RebornOperatorToolInfo) -> bool {
    tool.effects.iter().any(|effect| {
        matches!(
            effect,
            EffectKind::Financial | EffectKind::ModifyApproval | EffectKind::ModifyBudget
        )
    })
}

fn tool_permission_state_wire(update: ToolPermissionUpdate) -> &'static str {
    match update {
        ToolPermissionUpdate::Default => "default",
        ToolPermissionUpdate::State(ToolPermissionState::AlwaysAllow) => "always_allow",
        ToolPermissionUpdate::State(ToolPermissionState::AskEachTime) => "ask_each_time",
        ToolPermissionUpdate::State(ToolPermissionState::Disabled) => "disabled",
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
    use ironclaw_approvals::{
        AutoApproveSettingStore, CapabilityPermissionOverrideStorePort,
        PersistentApprovalPolicyStore, ToolPermissionOverrideStore,
    };
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{AgentId, ExtensionId, InvocationId, ResourceScope, TenantId};

    use super::*;

    #[test]
    fn capabilities_are_api_only_modify_approval() {
        for manifest in [
            manifest().expect("auto-approve manifest"),
            tool_permission_manifest().expect("tool-permission manifest"),
        ] {
            assert_eq!(manifest.visibility, CapabilityVisibility::Api);
            assert_eq!(manifest.effects, vec![EffectKind::ModifyApproval]);
            assert_eq!(manifest.default_permission, PermissionMode::Allow);
        }
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

    #[tokio::test]
    async fn tool_permission_handler_writes_persistent_policy_and_override() {
        let scoped = crate::wrap_scoped(Arc::new(InMemoryBackend::new()));
        let overrides = Arc::new(ToolPermissionOverrideStore::new(Arc::clone(&scoped)));
        let persistent_policies = Arc::new(PersistentApprovalPolicyStore::new(Arc::clone(&scoped)));
        let auto_approve = Arc::new(AutoApproveSettingStore::new(scoped));
        let capability_id = CapabilityId::new("ext.search").expect("capability id");
        let provider = ExtensionId::new("ext").expect("provider id");
        let tool_catalog: Arc<dyn RebornOperatorToolCatalog> =
            Arc::new(StaticToolCatalog(vec![RebornOperatorToolInfo {
                capability_id: capability_id.clone(),
                provider: provider.clone(),
                description: Arc::from("Search"),
                default_permission: PermissionMode::Ask,
                effects: Arc::<[EffectKind]>::from(vec![EffectKind::Network]),
            }]));
        let mut registry = FirstPartyCapabilityRegistry::new();
        insert_handler(
            &mut registry,
            auto_approve,
            overrides.clone(),
            persistent_policies.clone(),
            tool_catalog,
        )
        .expect("insert handlers");
        let handler = registry
            .get(&CapabilityId::new(OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID).expect("id"))
            .expect("tool permission handler");
        let user = UserId::new("operator").expect("user id");
        let scope = ResourceScope::local_default(user.clone(), InvocationId::new())
            .expect("resource scope");

        let mut request = FirstPartyCapabilityRequest::request_for_test(
            CapabilityId::new(OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID)
                .expect("capability id"),
            scope.clone(),
            serde_json::json!({
                "capability_id": capability_id.as_str(),
                "state": "always_allow",
            }),
            None,
        );
        request.authenticated_actor_user_id = Some(user.clone());
        let result = handler.dispatch(request).await.expect("dispatch");
        assert_eq!(result.output["state"], "always_allow");
        let operator_scope = scope.tenant_user_settings_scope();
        let policy_key = PersistentApprovalPolicyKey::new(
            &operator_scope,
            PersistentApprovalAction::Dispatch,
            capability_id.clone(),
            Principal::Extension(provider),
        );
        assert!(
            persistent_policies
                .lookup(&policy_key)
                .await
                .expect("policy lookup")
                .and_then(|policy| policy.active_grant())
                .is_some()
        );
        assert!(
            overrides
                .get(&ToolPermissionOverrideKey::new(
                    &operator_scope,
                    capability_id.clone()
                ))
                .await
                .expect("override lookup")
                .is_none()
        );

        let mut request = FirstPartyCapabilityRequest::request_for_test(
            CapabilityId::new(OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID)
                .expect("capability id"),
            scope.clone(),
            serde_json::json!({
                "capability_id": capability_id.as_str(),
                "state": "disabled",
            }),
            None,
        );
        request.authenticated_actor_user_id = Some(user);
        let result = handler.dispatch(request).await.expect("dispatch");
        assert_eq!(result.output["state"], "disabled");
        assert!(
            persistent_policies
                .lookup(&policy_key)
                .await
                .expect("policy lookup")
                .and_then(|policy| policy.active_grant())
                .is_none()
        );
        assert_eq!(
            overrides
                .get(&ToolPermissionOverrideKey::new(
                    &operator_scope,
                    capability_id
                ))
                .await
                .expect("override lookup")
                .map(|record| record.state),
            Some(ToolPermissionOverride::Disabled)
        );
    }

    struct StaticToolCatalog(Vec<RebornOperatorToolInfo>);

    #[async_trait]
    impl RebornOperatorToolCatalog for StaticToolCatalog {
        async fn list_operator_tools(&self, _caller: &UserId) -> Vec<RebornOperatorToolInfo> {
            self.0.clone()
        }
    }
}

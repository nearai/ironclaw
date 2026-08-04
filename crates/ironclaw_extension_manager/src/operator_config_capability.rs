//! Authorized first-party mutations for operator configuration.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_approvals::{
    AutoApproveSettingInput, PersistentApprovalAction, PersistentApprovalPolicyError,
    PersistentApprovalPolicyInput, PersistentApprovalPolicyKey, ToolPermissionOverride,
    ToolPermissionOverrideInput, ToolPermissionOverrideKey, ToolPermissionState,
};
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    capability::{EffectKind, GrantConstraints, OriginGateMatrix, PermissionMode},
    capability_profile::CapabilityProfileSchemaRef,
    dispatch::RuntimeDispatchErrorKind,
    error::HostApiError,
    ids::{CapabilityId, UserId},
    resource::{ResourceEstimate, ResourceProfile, ResourceScope, ResourceUsage},
    scope::Principal,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product::{
    OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID,
    OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID,
};
use ironclaw_product_contracts::operator_tools::{
    RebornOperatorToolCatalog, RebornOperatorToolInfo,
};

pub fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package.manifest.capabilities.push(manifest()?);
    package
        .manifest
        .capabilities
        .push(tool_permission_manifest()?);
    let root = package
        .materialized_root()
        .map_err(|error| ExtensionError::InvalidManifest {
            reason: format!("built-in package requires a materialized root: {error}"),
        })?
        .clone();
    ExtensionPackage::from_manifest(package.manifest, root)
}

pub fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    auto_approve: Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort>,
    overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort>,
    persistent_policies: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort>,
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
    auto_approve: Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort>,
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
                tracing::debug!(%error, "operator auto-approve setting mutation failed");
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
    overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort>,
    persistent_policies: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort>,
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
    overrides: &dyn ironclaw_approvals::ToolPermissionOverrideStorePort,
    persistent_policies: &dyn ironclaw_approvals::PersistentApprovalPolicyStorePort,
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
                    tracing::debug!(%error, "operator tool permission override clear failed");
                    dispatch_error(RuntimeDispatchErrorKind::Backend, started)
                })?;
        }
        ToolPermissionUpdate::State(ToolPermissionState::AlwaysAllow) => {
            // Clear the contradicting override BEFORE minting the grant. These are
            // two stores and the pair is not atomic, so the order decides what a
            // partial failure leaves behind. Granting first and failing to clear
            // would persist a live `Dispatch` grant underneath a stale
            // `ToolPermissionOverride::Disabled`: the gate honours the explicit
            // override, so the tool reads as disabled while carrying auto-approval
            // authority that takes effect the moment anything else clears the
            // override. Clearing first can only ever leave *less* authority than
            // the operator asked for (the tool falls back to its default), which is
            // the fail-closed direction.
            overrides
                .clear(&ToolPermissionOverrideKey::new(
                    &operator_scope,
                    tool.capability_id.clone(),
                ))
                .await
                .map_err(|error| {
                    tracing::debug!(%error, "operator tool permission override clear failed");
                    dispatch_error(RuntimeDispatchErrorKind::Backend, started)
                })?;
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
                    tracing::debug!(%error, "operator persistent approval policy write failed");
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
                    tracing::debug!(%error, "operator tool permission override write failed");
                    dispatch_error(RuntimeDispatchErrorKind::Backend, started)
                })?;
        }
    }
    Ok(())
}

async fn revoke_persistent_policy(
    persistent_policies: &dyn ironclaw_approvals::PersistentApprovalPolicyStorePort,
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
            tracing::debug!(%error, "operator persistent approval policy revoke failed");
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
        AutoApproveSettingStore, PersistentApprovalPolicyStore, ToolPermissionOverrideStore,
    };
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        ids::{AgentId, ExtensionId, InvocationId, TenantId},
        resource::ResourceScope,
    };

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

    /// The approvals stores plus the tool-permission handler, wired over one
    /// in-memory filesystem so a test can assert on what the handler actually
    /// persisted.
    #[allow(clippy::type_complexity)]
    fn tool_permission_fixture(
        tools: Vec<RebornOperatorToolInfo>,
    ) -> (
        Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort>,
        Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort>,
        Arc<dyn FirstPartyCapabilityHandler>,
    ) {
        let scoped = Arc::new(ironclaw_filesystem::ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            ironclaw_host_api::mount::MountView::new(vec![
                ironclaw_host_api::mount::MountGrant::new(
                    ironclaw_host_api::path::MountAlias::new("/approvals")
                        .expect("test approvals mount alias"),
                    ironclaw_host_api::path::VirtualPath::new("/projects/approvals")
                        .expect("test approvals mount target"),
                    ironclaw_host_api::mount::MountPermissions::read_write_list_delete(),
                ),
            ])
            .expect("test mount view"),
        ));
        let overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort> =
            Arc::new(ToolPermissionOverrideStore::new(Arc::clone(&scoped)));
        let persistent_policies: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort> =
            Arc::new(PersistentApprovalPolicyStore::new(Arc::clone(&scoped)));
        let auto_approve = Arc::new(AutoApproveSettingStore::new(scoped));
        let tool_catalog: Arc<dyn RebornOperatorToolCatalog> = Arc::new(StaticToolCatalog(tools));
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
        (overrides, persistent_policies, handler)
    }

    #[tokio::test]
    async fn tool_permission_handler_writes_persistent_policy_and_override() {
        let capability_id = CapabilityId::new("ext.search").expect("capability id");
        let provider = ExtensionId::new("ext").expect("provider id");
        let (overrides, persistent_policies, handler) =
            tool_permission_fixture(vec![RebornOperatorToolInfo {
                capability_id: capability_id.clone(),
                provider: provider.clone(),
                description: Arc::from("Search"),
                default_permission: PermissionMode::Ask,
                effects: Arc::<[EffectKind]>::from(vec![EffectKind::Network]),
            }]);
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

    /// The hard floor, driven through `handler.dispatch` rather than through
    /// `hard_floor_tool`/`tool_permission_locked` directly.
    ///
    /// Those predicates gate a persistent authority write, and a wrapper plus a
    /// catalog lookup sit between them and that write — so a unit test on the
    /// predicate alone would not catch a wrong `matches!` arm or an inverted
    /// `==` in the caller (`.claude/rules/testing.md`, "Test through the
    /// caller"). Every locked shape must be refused *and* leave both stores
    /// untouched: a refusal that still wrote would be the dangerous outcome.
    #[tokio::test]
    async fn locked_tools_are_refused_and_write_nothing() {
        let user = UserId::new("operator").expect("user id");
        let provider = ExtensionId::new("ext").expect("provider id");

        // One case per reason a tool is locked: the three hard-floor effects,
        // and a `Deny` default with an otherwise innocuous effect.
        let cases: Vec<(&str, PermissionMode, EffectKind)> = vec![
            ("ext.pay", PermissionMode::Ask, EffectKind::Financial),
            (
                "ext.approve",
                PermissionMode::Ask,
                EffectKind::ModifyApproval,
            ),
            ("ext.budget", PermissionMode::Ask, EffectKind::ModifyBudget),
            ("ext.denied", PermissionMode::Deny, EffectKind::Network),
        ];

        for (tool_id, default_permission, effect) in cases {
            let capability_id = CapabilityId::new(tool_id).expect("capability id");
            let (overrides, persistent_policies, handler) =
                tool_permission_fixture(vec![RebornOperatorToolInfo {
                    capability_id: capability_id.clone(),
                    provider: provider.clone(),
                    description: Arc::from("Locked"),
                    default_permission,
                    effects: Arc::<[EffectKind]>::from(vec![effect]),
                }]);
            let scope = ResourceScope::local_default(user.clone(), InvocationId::new())
                .expect("resource scope");

            for state in ["always_allow", "ask_each_time", "disabled", "default"] {
                let mut request = FirstPartyCapabilityRequest::request_for_test(
                    CapabilityId::new(OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID)
                        .expect("capability id"),
                    scope.clone(),
                    serde_json::json!({
                        "capability_id": capability_id.as_str(),
                        "state": state,
                    }),
                    None,
                );
                request.authenticated_actor_user_id = Some(user.clone());
                let error = handler
                    .dispatch(request)
                    .await
                    .expect_err(&format!("{tool_id} -> {state} must be refused"));
                assert_eq!(
                    error.kind(),
                    Some(RuntimeDispatchErrorKind::PolicyDenied),
                    "{tool_id} -> {state} must be refused as a policy denial, not another kind"
                );
            }

            let operator_scope = ResourceScope::local_default(user.clone(), InvocationId::new())
                .expect("resource scope")
                .tenant_user_settings_scope();
            let policy_key = PersistentApprovalPolicyKey::new(
                &operator_scope,
                PersistentApprovalAction::Dispatch,
                capability_id.clone(),
                Principal::Extension(provider.clone()),
            );
            assert!(
                persistent_policies
                    .lookup(&policy_key)
                    .await
                    .expect("policy lookup")
                    .and_then(|policy| policy.active_grant())
                    .is_none(),
                "{tool_id}: a refused request must not mint a persistent grant"
            );
            assert!(
                overrides
                    .get(&ToolPermissionOverrideKey::new(
                        &operator_scope,
                        capability_id
                    ))
                    .await
                    .expect("override lookup")
                    .is_none(),
                "{tool_id}: a refused request must not write an override"
            );
        }
    }

    /// An override store whose `clear` always fails, so a test can observe what
    /// a partial failure of the two-store `always_allow` write leaves behind.
    struct ClearFailsOverrideStore {
        inner: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort>,
    }

    #[async_trait]
    impl ironclaw_approvals::CapabilityPermissionOverrideStorePort for ClearFailsOverrideStore {
        async fn set(
            &self,
            input: ironclaw_approvals::CapabilityPermissionOverrideInput,
        ) -> Result<
            ironclaw_approvals::CapabilityPermissionOverrideRecord,
            ironclaw_approvals::CapabilityPermissionStoreError,
        > {
            self.inner.set(input).await
        }

        async fn get(
            &self,
            key: &ironclaw_approvals::CapabilityPermissionOverrideKey,
        ) -> Result<
            Option<ironclaw_approvals::CapabilityPermissionOverrideRecord>,
            ironclaw_approvals::CapabilityPermissionStoreError,
        > {
            self.inner.get(key).await
        }

        async fn clear(
            &self,
            _key: &ironclaw_approvals::CapabilityPermissionOverrideKey,
        ) -> Result<(), ironclaw_approvals::CapabilityPermissionStoreError> {
            Err(
                ironclaw_approvals::CapabilityPermissionStoreError::Filesystem(
                    "injected clear failure".to_string(),
                ),
            )
        }
    }

    /// `always_allow` writes two stores and the pair is not atomic, so the
    /// order decides what a partial failure leaves behind.
    ///
    /// Granting first and then failing to clear would persist a live `Dispatch`
    /// grant underneath the operator's earlier `Disabled` override — auto-approval
    /// authority the operator never sees, waiting for anything else to clear the
    /// override. Clearing first can only ever leave *less* authority than was
    /// asked for. This pins the fail-closed direction: after a failed
    /// `always_allow`, there must be no persistent grant.
    #[tokio::test]
    async fn a_failed_always_allow_never_leaves_a_grant_behind() {
        let capability_id = CapabilityId::new("ext.search").expect("capability id");
        let provider = ExtensionId::new("ext").expect("provider id");
        let tool = RebornOperatorToolInfo {
            capability_id: capability_id.clone(),
            provider: provider.clone(),
            description: Arc::from("Search"),
            default_permission: PermissionMode::Ask,
            effects: Arc::<[EffectKind]>::from(vec![EffectKind::Network]),
        };
        let (overrides, persistent_policies, _) = tool_permission_fixture(vec![tool.clone()]);
        let failing_overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort> =
            Arc::new(ClearFailsOverrideStore {
                inner: Arc::clone(&overrides),
            });
        let user = UserId::new("operator").expect("user id");
        let scope = ResourceScope::local_default(user.clone(), InvocationId::new())
            .expect("resource scope");
        let operator_scope = scope.tenant_user_settings_scope();

        // The operator had previously disabled the tool.
        overrides
            .set(ToolPermissionOverrideInput {
                scope: operator_scope.clone(),
                capability_id: capability_id.clone(),
                state: ToolPermissionOverride::Disabled,
                updated_by: Principal::User(user.clone()),
            })
            .await
            .expect("seed disabled override");

        let error = apply_tool_permission_state(
            failing_overrides.as_ref(),
            persistent_policies.as_ref(),
            &scope,
            &user,
            &tool,
            ToolPermissionUpdate::State(ToolPermissionState::AlwaysAllow),
            Instant::now(),
        )
        .await
        .expect_err("the failing override clear must surface");
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::Backend));

        let policy_key = PersistentApprovalPolicyKey::new(
            &operator_scope,
            PersistentApprovalAction::Dispatch,
            capability_id,
            Principal::Extension(provider),
        );
        assert!(
            persistent_policies
                .lookup(&policy_key)
                .await
                .expect("policy lookup")
                .and_then(|policy| policy.active_grant())
                .is_none(),
            "a failed always_allow must not leave a live Dispatch grant under the stale \
             Disabled override"
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

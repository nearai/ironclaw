use futures_util::{StreamExt, stream};
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_extensions::{CapabilityVisibility, ExtensionPackage, ExtensionRegistry};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityGrant, Decision, EffectKind, ResourceEstimate, RuntimeKind,
    canonical_json_v1, runtime_policy::EffectiveRuntimePolicy, sha256_digest_token,
};
use ironclaw_trust::TrustDecision;
use serde_json::{Value, json};

use crate::{
    CapabilitySurfaceVersion, HostRuntimeError, VisibleCapabilityRequest, VisibleCapabilitySurface,
    capability_catalog::read_json_ref,
    first_party_tools::{BUILTIN_FIRST_PARTY_PROVIDER, resolve_builtin_input_schema_ref},
    plan_capability,
};

const ALL_RUNTIME_KINDS: &[RuntimeKind] = &[
    RuntimeKind::Wasm,
    RuntimeKind::Mcp,
    RuntimeKind::Script,
    RuntimeKind::FirstParty,
    RuntimeKind::System,
];

const ALL_EFFECT_KINDS: &[EffectKind] = &[
    EffectKind::ReadFilesystem,
    EffectKind::WriteFilesystem,
    EffectKind::DeleteFilesystem,
    EffectKind::Network,
    EffectKind::UseSecret,
    EffectKind::ExecuteCode,
    EffectKind::SpawnProcess,
    EffectKind::DispatchCapability,
    EffectKind::ModifyExtension,
    EffectKind::ModifyApproval,
    EffectKind::ModifyBudget,
    EffectKind::ExternalWrite,
    EffectKind::Financial,
];
const VISIBLE_CAPABILITY_AUTHORIZATION_CONCURRENCY: usize = 16;

/// Visibility-only policy applied before authorization estimates are rendered.
///
/// This is a narrowing surface policy, not an authority source. A runtime/effect
/// listed here can still be omitted by missing grants, missing provider trust,
/// denied trust ceilings, or an authorizer denial. A runtime/effect absent here
/// is omitted before the authorizer is consulted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySurfacePolicy {
    /// Runtime kinds that may appear on this projection.
    ///
    /// Empty means allow none. Order and duplicates do not affect filtering or
    /// surface-version fingerprinting.
    pub allowed_runtimes: Vec<RuntimeKind>,
    /// Effect ceiling for visible descriptors.
    ///
    /// This is strict subset semantics: every effect declared by a capability
    /// must appear in this list or the capability is omitted. Empty means allow
    /// none. Order and duplicates do not affect filtering or surface-version
    /// fingerprinting.
    pub allowed_effects: Vec<EffectKind>,
    /// Whether capabilities that require approval may be rendered as askable.
    ///
    /// This is informational only. It does not issue approval leases or widen
    /// direct invocation authority.
    pub include_requires_approval: bool,
    /// Maximum visible capabilities returned after filtering, in registry
    /// order. `Some(0)` returns an empty surface without authorizer calls.
    pub max_capabilities: Option<usize>,
}

impl CapabilitySurfacePolicy {
    pub fn allow_all() -> Self {
        Self {
            allowed_runtimes: ALL_RUNTIME_KINDS.to_vec(),
            allowed_effects: ALL_EFFECT_KINDS.to_vec(),
            include_requires_approval: true,
            max_capabilities: None,
        }
    }

    fn allows_runtime(&self, runtime: RuntimeKind) -> bool {
        self.allowed_runtimes.contains(&runtime)
    }

    fn allows_effects(&self, effects: &[EffectKind]) -> bool {
        effects
            .iter()
            .all(|effect| self.allowed_effects.contains(effect))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisibleCapabilityAccess {
    /// Caller can invoke directly if the same context remains authorized at
    /// invocation time.
    Available,
    /// Capability may be shown as askable, but actual use must still block on
    /// the approval/lease path.
    RequiresApproval,
}

/// Capability metadata safe to render on a model/tool surface.
///
/// This is a visibility affordance, not authority. Direct invocation still
/// re-runs host-owned trust, grants, approvals, obligations, and runtime
/// dispatch checks.
#[derive(Debug, Clone, PartialEq)]
pub struct VisibleCapability {
    /// Redacted declarative capability descriptor from the extension registry.
    pub descriptor: CapabilityDescriptor,
    /// Current visibility status for this context and policy.
    pub access: VisibleCapabilityAccess,
    /// Host-selected estimate used for the visibility authorization check.
    pub estimated_resources: ResourceEstimate,
}

pub(crate) struct CapabilityCatalog<'a> {
    registry: &'a ExtensionRegistry,
    authorizer: &'a dyn TrustAwareCapabilityDispatchAuthorizer,
    base_version: &'a CapabilitySurfaceVersion,
    runtime_policy: &'a EffectiveRuntimePolicy,
    filesystem: Option<&'a dyn RootFilesystem>,
}

impl<'a> CapabilityCatalog<'a> {
    pub(crate) fn new(
        registry: &'a ExtensionRegistry,
        authorizer: &'a dyn TrustAwareCapabilityDispatchAuthorizer,
        base_version: &'a CapabilitySurfaceVersion,
        runtime_policy: &'a EffectiveRuntimePolicy,
    ) -> Self {
        Self {
            registry,
            authorizer,
            base_version,
            runtime_policy,
            filesystem: None,
        }
    }

    pub(crate) fn with_filesystem(mut self, filesystem: &'a dyn RootFilesystem) -> Self {
        self.filesystem = Some(filesystem);
        self
    }

    pub(crate) async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
        request.context.validate().map_err(|error| {
            HostRuntimeError::invalid_request(format!("invalid execution context: {error}"))
        })?;

        let Some(max_capabilities) = request.policy.max_capabilities else {
            return self.visible_capabilities_unbounded(request).await;
        };
        let mut capabilities = Vec::new();
        for descriptor in self.registry.capabilities() {
            if capabilities.len() >= max_capabilities {
                break;
            }
            if !self.is_model_visible(descriptor)
                || !request.policy.allows_runtime(descriptor.runtime)
                || !request.policy.allows_effects(&descriptor.effects)
            {
                continue;
            }
            if plan_capability(descriptor, self.runtime_policy).is_err() {
                continue;
            }
            let Some(trust_decision) = request.provider_trust.get(&descriptor.provider) else {
                continue;
            };
            if let Some(capability) = self
                .authorize_visible_capability(&request, descriptor, trust_decision)
                .await?
            {
                capabilities.push(capability);
            }
        }

        let version = surface_version(
            self.base_version,
            &request,
            self.runtime_policy,
            &capabilities,
        )?;
        Ok(VisibleCapabilitySurface {
            version,
            capabilities,
        })
    }

    async fn visible_capabilities_unbounded(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
        let candidates = self
            .registry
            .capabilities()
            .filter(|descriptor| {
                self.is_model_visible(descriptor)
                    && request.policy.allows_runtime(descriptor.runtime)
                    && request.policy.allows_effects(&descriptor.effects)
                    && plan_capability(descriptor, self.runtime_policy).is_ok()
                    && request.provider_trust.contains_key(&descriptor.provider)
            })
            .cloned()
            .collect::<Vec<_>>();

        let results = stream::iter(candidates.into_iter().map(|descriptor| {
            let request = &request;
            async move {
                let Some(trust_decision) = request.provider_trust.get(&descriptor.provider) else {
                    return Ok(None);
                };
                self.authorize_visible_capability(request, &descriptor, trust_decision)
                    .await
            }
        }))
        .buffered(VISIBLE_CAPABILITY_AUTHORIZATION_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

        let mut capabilities = Vec::new();
        for result in results {
            if let Some(capability) = result? {
                capabilities.push(capability);
            }
        }
        let version = surface_version(
            self.base_version,
            &request,
            self.runtime_policy,
            &capabilities,
        )?;
        Ok(VisibleCapabilitySurface {
            version,
            capabilities,
        })
    }

    async fn authorize_visible_capability(
        &self,
        request: &VisibleCapabilityRequest,
        descriptor: &CapabilityDescriptor,
        trust_decision: &TrustDecision,
    ) -> Result<Option<VisibleCapability>, HostRuntimeError> {
        let estimate = descriptor
            .resource_profile
            .as_ref()
            .map(|profile| profile.default_estimate.clone())
            .unwrap_or_default();
        let mut context = request.context.clone();
        context.trust = trust_decision.effective_trust.class();

        let access = match self
            .authorizer
            .authorize_dispatch_with_trust(&context, descriptor, &estimate, trust_decision)
            .await
        {
            Decision::Allow { .. } => VisibleCapabilityAccess::Available,
            Decision::RequireApproval { .. } if request.policy.include_requires_approval => {
                VisibleCapabilityAccess::RequiresApproval
            }
            Decision::RequireApproval { .. } | Decision::Deny { .. } => return Ok(None),
        };

        Ok(Some(VisibleCapability {
            descriptor: self.surface_descriptor(descriptor).await?,
            access,
            estimated_resources: estimate,
        }))
    }

    fn is_model_visible(&self, descriptor: &CapabilityDescriptor) -> bool {
        self.registry
            .capability_visibility(&descriptor.id)
            .unwrap_or(CapabilityVisibility::Model)
            == CapabilityVisibility::Model
    }

    async fn surface_descriptor(
        &self,
        descriptor: &CapabilityDescriptor,
    ) -> Result<CapabilityDescriptor, HostRuntimeError> {
        let mut descriptor = descriptor.clone();
        let reference = descriptor
            .parameters_schema
            .get("$ref")
            .and_then(Value::as_str)
            .map(str::to_string);

        if descriptor.provider.as_str() == BUILTIN_FIRST_PARTY_PROVIDER {
            let Some(reference) = reference else {
                return Err(HostRuntimeError::invalid_request(format!(
                    "built-in capability {} must publish from an input schema ref",
                    descriptor.id
                )));
            };
            descriptor.parameters_schema = resolve_builtin_input_schema_ref(&reference)
                .ok_or_else(|| {
                    HostRuntimeError::invalid_request(format!(
                        "built-in capability {} references unknown input schema {}",
                        descriptor.id, reference
                    ))
                })?;
            return Ok(descriptor);
        }

        let Some(reference) = reference else {
            return Ok(descriptor);
        };
        let Some(filesystem) = self.filesystem else {
            return Ok(descriptor);
        };
        let Some(package) = self.registry.get_extension(&descriptor.provider) else {
            return Ok(descriptor);
        };
        descriptor.parameters_schema =
            resolve_package_input_schema_ref(filesystem, package, &descriptor.id, &reference)
                .await?;
        Ok(descriptor)
    }
}

async fn resolve_package_input_schema_ref(
    filesystem: &dyn RootFilesystem,
    package: &ExtensionPackage,
    capability_id: &ironclaw_host_api::CapabilityId,
    reference: &str,
) -> Result<Value, HostRuntimeError> {
    let Some(declaration) = package
        .manifest
        .capabilities
        .iter()
        .find(|capability| &capability.id == capability_id)
    else {
        return Err(HostRuntimeError::invalid_request(format!(
            "capability {capability_id} is missing manifest declaration"
        )));
    };
    if declaration.input_schema_ref.as_str() != reference {
        return Err(HostRuntimeError::invalid_request(format!(
            "capability {capability_id} descriptor schema ref {reference} does not match manifest input schema ref {}",
            declaration.input_schema_ref.as_str()
        )));
    }
    read_json_ref(
        filesystem,
        &package.root,
        &declaration.input_schema_ref,
        "input_schema_ref",
    )
    .await
}

fn surface_version(
    base_version: &CapabilitySurfaceVersion,
    request: &VisibleCapabilityRequest,
    runtime_policy: &EffectiveRuntimePolicy,
    capabilities: &[VisibleCapability],
) -> Result<CapabilitySurfaceVersion, HostRuntimeError> {
    let context_payload = context_version_payload(request)?;
    let mut capability_payload = capabilities
        .iter()
        .map(|capability| {
            let descriptor = canonical_descriptor_for_version(&capability.descriptor);
            let trust = request
                .provider_trust
                .get(&capability.descriptor.provider)
                .map(trust_decision_version_payload);
            (
                capability_version_key(capability),
                json!({
                    "descriptor": descriptor,
                    "estimated_resources": &capability.estimated_resources,
                    "access": access_token(capability.access),
                    "provider_trust": trust,
                }),
            )
        })
        .collect::<Vec<_>>();
    capability_payload.sort_by(|(left, _), (right, _)| left.cmp(right));
    let capability_payload = capability_payload
        .into_iter()
        .map(|(_, payload)| payload)
        .collect::<Vec<_>>();
    let payload = json!({
        "version": 1,
        "kind": "visible_capability_surface",
        "base_version": base_version.as_str(),
        "surface_kind": request.surface_kind.as_str(),
        "context": context_payload,
        "policy": {
            "allowed_runtimes": canonical_runtime_kinds(&request.policy.allowed_runtimes),
            "allowed_effects": canonical_effect_kinds(&request.policy.allowed_effects),
            "include_requires_approval": request.policy.include_requires_approval,
            "max_capabilities": request.policy.max_capabilities,
        },
        "runtime_policy": runtime_policy,
        "capabilities": capability_payload,
    });
    let canonical = canonical_json_v1(&payload).map_err(host_api_error)?;
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| HostRuntimeError::invalid_request(error.to_string()))?;
    CapabilitySurfaceVersion::new(sha256_digest_token(&bytes))
}

fn context_version_payload(request: &VisibleCapabilityRequest) -> Result<Value, HostRuntimeError> {
    let context = &request.context;
    Ok(json!({
        "tenant_id": &context.tenant_id,
        "user_id": &context.user_id,
        "agent_id": &context.agent_id,
        "project_id": &context.project_id,
        "mission_id": &context.mission_id,
        "thread_id": &context.thread_id,
        "extension_id": &context.extension_id,
        "runtime": context.runtime,
        "grants": canonical_grants(&context.grants.grants)?,
    }))
}

fn canonical_grants(grants: &[CapabilityGrant]) -> Result<Vec<Value>, HostRuntimeError> {
    let mut payload = grants
        .iter()
        .map(|grant| {
            let value = json!({
                "capability": &grant.capability,
                "grantee": &grant.grantee,
                "allowed_effects": canonical_effect_kinds(&grant.constraints.allowed_effects),
                "resource_ceiling": &grant.constraints.resource_ceiling,
                "expires_at": &grant.constraints.expires_at,
                "max_invocations": grant.constraints.max_invocations,
                "secret_count": grant.constraints.secrets.len(),
            });
            let canonical = canonical_json_v1(&value).map_err(host_api_error)?;
            let key = stable_json_string(&canonical)?;
            Ok((key, canonical))
        })
        .collect::<Result<Vec<_>, HostRuntimeError>>()?;
    payload.sort_by(|(left, _), (right, _)| left.cmp(right));
    Ok(payload.into_iter().map(|(_, value)| value).collect())
}

fn trust_decision_version_payload(trust_decision: &TrustDecision) -> Value {
    json!({
        "effective_trust": &trust_decision.effective_trust,
        "authority_ceiling": {
            "allowed_effects": canonical_effect_kinds(&trust_decision.authority_ceiling.allowed_effects),
            "max_resource_ceiling": &trust_decision.authority_ceiling.max_resource_ceiling,
        },
    })
}

fn canonical_descriptor_for_version(descriptor: &CapabilityDescriptor) -> CapabilityDescriptor {
    let mut descriptor = descriptor.clone();
    descriptor
        .effects
        .sort_by_key(|effect| effect_kind_token(*effect));
    descriptor.effects.dedup();
    descriptor
}

fn capability_version_key(
    capability: &VisibleCapability,
) -> (String, String, &'static str, &'static str) {
    (
        capability.descriptor.id.as_str().to_string(),
        capability.descriptor.provider.as_str().to_string(),
        runtime_kind_token(capability.descriptor.runtime),
        access_token(capability.access),
    )
}

fn canonical_runtime_kinds(runtimes: &[RuntimeKind]) -> Vec<&'static str> {
    let mut values = runtimes
        .iter()
        .map(|runtime| runtime_kind_token(*runtime))
        .collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    values
}

fn canonical_effect_kinds(effects: &[EffectKind]) -> Vec<&'static str> {
    let mut values = effects
        .iter()
        .map(|effect| effect_kind_token(*effect))
        .collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    values
}

fn runtime_kind_token(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Wasm => "wasm",
        RuntimeKind::Mcp => "mcp",
        RuntimeKind::Script => "script",
        RuntimeKind::FirstParty => "first_party",
        RuntimeKind::System => "system",
    }
}

fn effect_kind_token(effect: EffectKind) -> &'static str {
    match effect {
        EffectKind::ReadFilesystem => "read_filesystem",
        EffectKind::WriteFilesystem => "write_filesystem",
        EffectKind::DeleteFilesystem => "delete_filesystem",
        EffectKind::Network => "network",
        EffectKind::UseSecret => "use_secret",
        EffectKind::ExecuteCode => "execute_code",
        EffectKind::SpawnProcess => "spawn_process",
        EffectKind::DispatchCapability => "dispatch_capability",
        EffectKind::ModifyExtension => "modify_extension",
        EffectKind::ModifyApproval => "modify_approval",
        EffectKind::ModifyBudget => "modify_budget",
        EffectKind::ExternalWrite => "external_write",
        EffectKind::Financial => "financial",
    }
}

fn access_token(access: VisibleCapabilityAccess) -> &'static str {
    match access {
        VisibleCapabilityAccess::Available => "available",
        VisibleCapabilityAccess::RequiresApproval => "requires_approval",
    }
}

fn stable_json_string(value: &Value) -> Result<String, HostRuntimeError> {
    serde_json::to_string(value)
        .map_err(|error| HostRuntimeError::invalid_request(error.to_string()))
}

fn host_api_error(error: ironclaw_host_api::HostApiError) -> HostRuntimeError {
    HostRuntimeError::invalid_request(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_authorization::GrantAuthorizer;
    use ironclaw_host_api::{
        CapabilityId, ExtensionId, PermissionMode, TrustClass,
        runtime_policy::{
            ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy,
            FilesystemBackendKind, NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
        },
    };

    fn test_runtime_policy() -> EffectiveRuntimePolicy {
        EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::SecureDefault,
            resolved_profile: RuntimeProfile::SecureDefault,
            filesystem_backend: FilesystemBackendKind::ScopedVirtual,
            process_backend: ProcessBackendKind::None,
            network_mode: NetworkMode::Deny,
            secret_mode: SecretMode::Deny,
            approval_policy: ApprovalPolicy::AskAlways,
            audit_mode: AuditMode::LocalMinimal,
        }
    }

    #[tokio::test]
    async fn builtin_surface_descriptor_requires_input_schema_ref() {
        let descriptor = CapabilityDescriptor {
            id: CapabilityId::new("builtin.bad").unwrap(),
            provider: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).unwrap(),
            runtime: RuntimeKind::FirstParty,
            trust_ceiling: TrustClass::UserTrusted,
            description: "bad built-in descriptor".to_string(),
            parameters_schema: json!({"type": "object"}),
            effects: vec![EffectKind::DispatchCapability],
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            resource_profile: None,
        };
        let registry = ExtensionRegistry::new();
        let runtime_policy = test_runtime_policy();
        let surface_version = CapabilitySurfaceVersion::new("surface-v1").unwrap();
        let authorizer = GrantAuthorizer;
        let catalog =
            CapabilityCatalog::new(&registry, &authorizer, &surface_version, &runtime_policy);

        let error = catalog
            .surface_descriptor(&descriptor)
            .await
            .expect_err("built-in schema refs are required");

        assert!(
            matches!(error, HostRuntimeError::InvalidRequest { ref reason }
                if reason.contains("must publish from an input schema ref")),
            "unexpected error: {error:?}"
        );
    }
}

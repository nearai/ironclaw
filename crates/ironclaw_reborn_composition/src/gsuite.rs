use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::CredentialAccountService;
use ironclaw_capabilities::{
    CapabilityObligationError, CapabilityObligationHandler, CapabilityObligationPhase,
    CapabilityObligationRequest,
};
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionManifest, ExtensionPackage,
    ExtensionRuntime, MANIFEST_SCHEMA_VERSION, ManifestSource,
};
use ironclaw_first_party_extensions::{
    GsuiteCapabilitySpec, GsuiteCredentialStageError, GsuiteCredentialStageRequest,
    GsuiteCredentialStager, GsuiteDispatchError, GsuiteDispatchRequest, GsuiteExecutor,
    GsuitePackageSpec, NoopGsuiteCredentialStager, gsuite_package_specs, gsuite_resource_profile,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, CapabilitySet, CorrelationId, ExecutionContext,
    ExtensionId, HostApiError, MountView, Obligation, RequestedTrustClass, ResourceEstimate,
    ResourceScope, RuntimeKind, TrustClass, VirtualPath,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

/// Host-bundled GSuite packages available to an install/activation surface.
///
/// These packages are deliberately not inserted into the default built-in
/// first-party registry. A product or composition install surface must
/// explicitly register their package descriptors and first-party handlers before
/// their capabilities become model-visible.
pub fn bundled_gsuite_extension_packages() -> Result<Vec<ExtensionPackage>, ExtensionError> {
    gsuite_package_specs()
        .iter()
        .map(package_from_spec)
        .collect()
}

/// Build GSuite handlers for a surface that can install and activate GSuite packages.
///
/// Handler registration is allowed before lifecycle activation because runtime
/// dispatch still requires active package descriptors.
pub fn bundled_gsuite_first_party_handlers(
    accounts: Arc<dyn CredentialAccountService>,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    bundled_gsuite_first_party_handlers_with_credential_stager(
        accounts,
        Arc::new(NoopGsuiteCredentialStager),
    )
}

pub fn bundled_gsuite_first_party_handlers_with_credential_stager(
    accounts: Arc<dyn CredentialAccountService>,
    credential_stager: Arc<dyn GsuiteCredentialStager>,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let mut registry = FirstPartyCapabilityRegistry::new();
    register_bundled_gsuite_first_party_handlers(&mut registry, accounts, credential_stager)?;
    Ok(registry)
}

pub(crate) fn register_bundled_gsuite_first_party_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
    accounts: Arc<dyn CredentialAccountService>,
    credential_stager: Arc<dyn GsuiteCredentialStager>,
) -> Result<(), HostApiError> {
    let handler = Arc::new(GsuiteFirstPartyHandler {
        executor: GsuiteExecutor::new(accounts).with_credential_stager(credential_stager),
    });
    for package in gsuite_package_specs() {
        for capability in package.capabilities {
            registry.insert_handler(CapabilityId::new(capability.id)?, Arc::clone(&handler));
        }
    }
    Ok(())
}

struct GsuiteFirstPartyHandler {
    executor: GsuiteExecutor,
}

#[async_trait]
impl FirstPartyCapabilityHandler for GsuiteFirstPartyHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let egress = request
            .services
            .runtime_http_egress
            .as_ref()
            .ok_or_else(|| {
                FirstPartyCapabilityError::new(
                    ironclaw_host_api::RuntimeDispatchErrorKind::NetworkDenied,
                )
            })?
            .clone();
        let result = self
            .executor
            .dispatch(GsuiteDispatchRequest {
                capability_id: &request.capability_id,
                scope: &request.scope,
                input: &request.input,
                runtime_http_egress: egress,
            })
            .await
            .map_err(gsuite_error)?;
        Ok(FirstPartyCapabilityResult::new(result.output, result.usage))
    }
}

fn package_from_spec(spec: &GsuitePackageSpec) -> Result<ExtensionPackage, ExtensionError> {
    let capabilities = spec
        .capabilities
        .iter()
        .map(|capability| capability_manifest(capability, spec.schema_prefix))
        .collect::<Result<Vec<_>, _>>()?;
    ExtensionPackage::from_manifest(
        ExtensionManifest {
            schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            id: ExtensionId::new(spec.extension_id)?,
            name: spec.name.to_string(),
            version: "0.1.0".to_string(),
            description: spec.description.to_string(),
            source: ManifestSource::HostBundled,
            requested_trust: RequestedTrustClass::FirstPartyRequested,
            descriptor_trust_default: TrustClass::Sandbox,
            runtime: ExtensionRuntime::FirstParty {
                service: spec.service.to_string(),
            },
            host_apis: Vec::new(),
            capabilities,
        },
        VirtualPath::new(format!("/system/extensions/{}", spec.extension_id))?,
    )
}

fn capability_manifest(
    capability: &GsuiteCapabilitySpec,
    schema_prefix: &str,
) -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(capability.id)?,
        implements: Vec::new(),
        description: capability.description.to_string(),
        effects: capability.effects.to_vec(),
        default_permission: capability.default_permission,
        visibility: CapabilityVisibility::Model,
        input_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/{}/{}.input.v1.json",
            schema_prefix, capability.short_name
        ))?,
        output_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/{}/{}.output.v1.json",
            schema_prefix, capability.short_name
        ))?,
        prompt_doc_ref: Some(CapabilityProfileSchemaRef::new(format!(
            "prompts/{}/{}.md",
            schema_prefix, capability.short_name
        ))?),
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        resource_profile: Some(gsuite_resource_profile()),
    })
}

fn gsuite_error(error: GsuiteDispatchError) -> FirstPartyCapabilityError {
    let mapped = if error.is_auth_required() {
        FirstPartyCapabilityError::auth_required()
    } else {
        FirstPartyCapabilityError::new(error.kind())
    };
    if let Some(usage) = error.usage().cloned() {
        mapped.with_usage(usage)
    } else {
        mapped
    }
}

pub(crate) struct ObligationGsuiteCredentialStager {
    obligation_handler: Arc<dyn CapabilityObligationHandler>,
}

impl ObligationGsuiteCredentialStager {
    pub(crate) fn new(obligation_handler: Arc<dyn CapabilityObligationHandler>) -> Self {
        Self { obligation_handler }
    }
}

#[async_trait]
impl GsuiteCredentialStager for ObligationGsuiteCredentialStager {
    async fn stage(
        &self,
        request: GsuiteCredentialStageRequest<'_>,
    ) -> Result<(), GsuiteCredentialStageError> {
        let context = gsuite_staging_context(request.scope, request.extension_id.clone())?;
        let obligation = Obligation::InjectSecretOnce {
            handle: request.access_secret.clone(),
        };
        let obligations = [obligation];
        let estimate = ResourceEstimate::default();
        self.obligation_handler
            .satisfy(CapabilityObligationRequest {
                phase: CapabilityObligationPhase::Invoke,
                context: &context,
                capability_id: request.capability_id,
                estimate: &estimate,
                obligations: &obligations,
            })
            .await
            .map_err(stage_obligation_error)
    }
}

fn gsuite_staging_context(
    scope: &ResourceScope,
    extension_id: ExtensionId,
) -> Result<ExecutionContext, GsuiteCredentialStageError> {
    let mounts = MountView::new(Vec::new()).map_err(|_| GsuiteCredentialStageError::Backend)?;
    let context = ExecutionContext {
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        extension_id,
        runtime: RuntimeKind::FirstParty,
        trust: TrustClass::FirstParty,
        grants: CapabilitySet::default(),
        mounts,
        resource_scope: scope.clone(),
    };
    context
        .validate()
        .map_err(|_| GsuiteCredentialStageError::Backend)?;
    Ok(context)
}

fn stage_obligation_error(error: CapabilityObligationError) -> GsuiteCredentialStageError {
    match error {
        CapabilityObligationError::AuthRequired => GsuiteCredentialStageError::AuthRequired,
        CapabilityObligationError::Unsupported { .. }
        | CapabilityObligationError::Failed { .. } => GsuiteCredentialStageError::Backend,
    }
}

//! API-visible first-party mutation for outbound delivery preferences.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, HostApiError, OriginGateMatrix,
    PermissionMode, ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode,
    ProductSurfaceErrorKind, ResourceEstimate, ResourceProfile, ResourceUsage,
    RuntimeDispatchErrorKind,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product::{
    OUTBOUND_PREFERENCES_SET_CAPABILITY_ID, OutboundPreferencesProductFacade,
    RebornSetOutboundPreferencesRequest,
};

pub(crate) fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package.manifest.capabilities.push(manifest()?);
    ExtensionPackage::from_manifest(package.manifest, package.root)
}

pub(crate) fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    facade: Arc<dyn OutboundPreferencesProductFacade>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(OUTBOUND_PREFERENCES_SET_CAPABILITY_ID)?,
        Arc::new(SetOutboundPreferencesHandler { facade }),
    );
    Ok(())
}

fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(OUTBOUND_PREFERENCES_SET_CAPABILITY_ID)?,
        description: "Set or clear the authenticated user's final-reply outbound delivery target."
            .to_string(),
        effects: vec![EffectKind::ExternalWrite],
        default_permission: PermissionMode::Allow,
        visibility: CapabilityVisibility::Api,
        input_schema_ref: CapabilityProfileSchemaRef::new(
            "schemas/builtin/outbound_preferences_set.input.v1.json",
        )?,
        output_schema_ref: Some(CapabilityProfileSchemaRef::new(
            "schemas/builtin/outbound_preferences_set.output.v1.json",
        )?),
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        max_egress_bytes: None,
        resource_profile: Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(500)
                .set_output_bytes(64 * 1024),
            hard_ceiling: None,
        }),
        origin_gate_matrix: Some(OriginGateMatrix::product_consent_only()),
    })
}

struct SetOutboundPreferencesHandler {
    facade: Arc<dyn OutboundPreferencesProductFacade>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for SetOutboundPreferencesHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        ensure_declared(&request, started)?;
        let caller = authenticated_caller(&request, started)?;
        let input: RebornSetOutboundPreferencesRequest = serde_json::from_value(request.input)
            .map_err(|_| dispatch_error(RuntimeDispatchErrorKind::InputEncode, started))?;
        let response = self
            .facade
            .set_outbound_preferences(caller, input)
            .await
            .map_err(|error| dispatch_error(map_services_error(error), started))?;
        let output = serde_json::to_value(response)
            .map_err(|_| dispatch_error(RuntimeDispatchErrorKind::InvalidResult, started))?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            resource_usage(started),
        ))
    }
}

fn authenticated_caller(
    request: &FirstPartyCapabilityRequest,
    started: Instant,
) -> Result<ProductSurfaceCaller, FirstPartyCapabilityError> {
    if request.authenticated_actor_user_id.as_ref() != Some(&request.scope.user_id) {
        return Err(dispatch_error(
            RuntimeDispatchErrorKind::PolicyDenied,
            started,
        ));
    }
    Ok(ProductSurfaceCaller::new(
        request.scope.tenant_id.clone(),
        request.scope.user_id.clone(),
        request.scope.agent_id.clone(),
        request.scope.project_id.clone(),
    ))
}

fn ensure_declared(
    request: &FirstPartyCapabilityRequest,
    started: Instant,
) -> Result<(), FirstPartyCapabilityError> {
    if request.capability_id.as_str() == OUTBOUND_PREFERENCES_SET_CAPABILITY_ID {
        Ok(())
    } else {
        Err(dispatch_error(
            RuntimeDispatchErrorKind::UndeclaredCapability,
            started,
        ))
    }
}

fn map_services_error(error: ProductSurfaceError) -> RuntimeDispatchErrorKind {
    match (error.code, error.kind) {
        (ProductSurfaceErrorCode::InvalidRequest, _) | (ProductSurfaceErrorCode::NotFound, _) => {
            RuntimeDispatchErrorKind::InputEncode
        }
        (ProductSurfaceErrorCode::Forbidden, _) => RuntimeDispatchErrorKind::PolicyDenied,
        (ProductSurfaceErrorCode::Unavailable, ProductSurfaceErrorKind::ServiceUnavailable) => {
            RuntimeDispatchErrorKind::Backend
        }
        (ProductSurfaceErrorCode::Conflict, _) => RuntimeDispatchErrorKind::OperationFailed,
        _ => RuntimeDispatchErrorKind::Backend,
    }
}

fn dispatch_error(kind: RuntimeDispatchErrorKind, started: Instant) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::new(kind).with_usage(resource_usage(started))
}

fn resource_usage(started: Instant) -> ResourceUsage {
    ResourceUsage::default()
        .set_wall_clock_ms(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_is_api_only_external_write() {
        let manifest = manifest().expect("manifest");
        assert_eq!(manifest.visibility, CapabilityVisibility::Api);
        assert_eq!(manifest.default_permission, PermissionMode::Allow);
        assert_eq!(manifest.effects, vec![EffectKind::ExternalWrite]);
    }
}
